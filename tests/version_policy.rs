use semver::Version;
use std::cmp::Ordering;
use std::process::Command;

fn advances(base: &str, head: &str) -> bool {
    Version::parse(head)
        .unwrap()
        .cmp_precedence(&Version::parse(base).unwrap())
        == Ordering::Greater
}

#[test]
fn semantic_versions_require_a_real_increase() {
    assert!(advances("1.0.0", "1.0.1"));
    assert!(advances("1.0.0", "1.1.0"));
    assert!(advances("1.1.0-rc.1", "1.1.0"));
    assert!(!advances("1.0.0", "1.0.0+build.2"));
    assert!(!advances("1.1.0", "1.1.0-rc.1"));
    assert!(!advances("1.1.0", "1.0.1"));
}

#[test]
fn pull_requests_increase_the_package_version() {
    // Reuse the existing cargo-test CI job. Local/offline test runs do not fetch.
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request")
    {
        return;
    }
    let base = std::env::var("GITHUB_BASE_REF").expect("GitHub must supply the PR base branch");
    let root = env!("CARGO_MANIFEST_DIR");
    let result = Command::new("git")
        .current_dir(root)
        .args([
            "fetch",
            "--no-tags",
            "--depth=1",
            "origin",
            &format!("refs/heads/{base}"),
        ])
        .output()
        .expect("fetch the current PR base");
    assert!(
        result.status.success(),
        "Cannot fetch PR base: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new("git")
        .current_dir(root)
        .args(["show", "FETCH_HEAD:Cargo.toml"])
        .output()
        .expect("read base manifest");
    assert!(result.status.success(), "Cannot read PR base manifest");
    let manifest: toml::Value =
        toml::from_str(std::str::from_utf8(&result.stdout).unwrap()).unwrap();
    let base_version = manifest["package"]["version"].as_str().unwrap();
    let version = env!("CARGO_PKG_VERSION");
    assert!(advances(base_version, version),
        "Every PR needs a SemVer bump in Cargo.toml and Cargo.lock: base={base_version}, head={version}");
}
