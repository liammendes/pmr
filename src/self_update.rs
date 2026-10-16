//! Release installation deliberately leaves the running daemon alone.
use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path};
use std::time::Duration;

const REPOSITORY: &str = "https://api.github.com/repos/liammendes/pmr";
const BINARIES: [&str; 4] = ["pmr", "pmr-runtime", "pmr-docker", "pmr-dev"];
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn target() -> Result<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => Ok("aarch64-apple-darwin"),
        ("x86_64", "macos") => Ok("x86_64-apple-darwin"),
        ("aarch64", "linux") if cfg!(target_env = "gnu") => Ok("aarch64-unknown-linux-gnu"),
        ("x86_64", "linux") if cfg!(target_env = "gnu") => Ok("x86_64-unknown-linux-gnu"),
        _ => bail!("No published binary for this platform; build from source."),
    }
}

fn download(client: &Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = client.get(url).send()?.error_for_status()?;
    if response.content_length().is_some_and(|size| size > limit) {
        bail!("Release download exceeds size limit");
    }
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("Release download exceeds size limit");
    }
    Ok(bytes)
}

fn asset_url(release: &Release, name: &str) -> Result<String> {
    let expected = format!(
        "https://github.com/liammendes/pmr/releases/download/{}/{}",
        release.tag_name, name
    );
    let matching: Vec<_> = release
        .assets
        .iter()
        .filter(|asset| asset.name == name)
        .collect();
    if matching.len() != 1 || matching[0].browser_download_url != expected {
        bail!("Missing or unexpected release asset: {name}");
    }
    Ok(expected)
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn verify_checksum(archive: &[u8], checksum: &[u8], name: &str) -> Result<()> {
    let text = std::str::from_utf8(checksum)?.trim();
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() != 2
        || fields[1].trim_start_matches('*') != name
        || fields[0] != digest_hex(archive)
    {
        bail!("Release checksum does not match {name}");
    }
    Ok(())
}

fn unpack(archive: &[u8], destination: &Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(archive);
    // Bound decompressed input too, including files we do not install.
    let mut tar = tar::Archive::new(decoder.take(512 * 1024 * 1024));
    let mut found = BTreeMap::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            bail!("Unsafe path in release archive");
        }
        let Some(name) = path.to_str().filter(|name| BINARIES.contains(name)) else {
            continue;
        };
        if !entry.header().entry_type().is_file() || entry.size() > 80 * 1024 * 1024 {
            bail!("Invalid binary in release archive: {name}");
        }
        if found.insert(name.to_string(), ()).is_some() {
            bail!("Duplicate release binary: {name}");
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination.join(name))?;
        std::io::copy(&mut entry, &mut output)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output.set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        output.flush()?;
        output.sync_all()?;
    }
    if found.len() != BINARIES.len() {
        bail!("Release archive must contain all four PMR binaries");
    }
    Ok(())
}

fn install(archive: &[u8], directory: &Path, expected_version: &str) -> Result<()> {
    // Serialize installs across PMR processes. The lock is intentionally persistent.
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".pmr-update.lock"))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("Another PMR update is in progress");
        }
    }
    let stage = tempfile::Builder::new()
        .prefix(".pmr-update-")
        .tempdir_in(directory)?;
    let incoming = stage.path().join("new");
    let backup = stage.path().join("backup");
    fs::create_dir(&incoming)?;
    fs::create_dir(&backup)?;
    unpack(archive, &incoming)?;
    let probe = std::process::Command::new(incoming.join("pmr"))
        .arg("--version")
        .env_clear()
        .output()
        .context("Downloaded PMR cannot run on this machine")?;
    if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).trim() != expected_version
    {
        bail!("Downloaded PMR failed the platform/version check; installed binaries are unchanged");
    }
    // Complete validation and backups before replacing any installed file.
    for name in BINARIES {
        let path = directory.join(name);
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            bail!("Expected a regular installed binary: {}", path.display());
        }
        fs::copy(&path, backup.join(name))?;
        File::open(backup.join(name))?.sync_all()?;
    }
    for (replaced, name) in BINARIES.iter().enumerate() {
        if let Err(error) = fs::rename(incoming.join(name), directory.join(name)) {
            for previous in BINARIES.iter().take(replaced) {
                if let Err(rollback) = fs::rename(backup.join(previous), directory.join(previous)) {
                    let recovery = stage.keep();
                    bail!(
                        "Update failed: {error}; rollback failed: {rollback}. Backups: {}",
                        recovery.display()
                    );
                }
            }
            return Err(error.into());
        }
    }
    File::open(directory)?.sync_all()?;
    drop(lock);
    Ok(())
}

pub fn update(check: bool, requested: Option<&str>) -> Result<()> {
    let current = Version::parse(crate::constants::PMR_VERSION)?;
    let endpoint = match requested {
        Some(version) => {
            let version = Version::parse(version.trim_start_matches('v'))?;
            format!("{REPOSITORY}/releases/tags/v{version}")
        }
        None => format!("{REPOSITORY}/releases/latest"),
    };
    let client = Client::builder()
        .user_agent(format!("pmr/{current}"))
        .https_only(true)
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(15))
        .build()?;
    let release: Release = serde_json::from_slice(&download(&client, &endpoint, 2 * 1024 * 1024)?)?;
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .ok_or_else(|| anyhow!("Invalid release tag"))?,
    )?;
    if release.draft || (requested.is_none() && (release.prerelease || !version.pre.is_empty())) {
        bail!("Expected a published stable release");
    }
    if let Some(requested) = requested {
        if version != Version::parse(requested.trim_start_matches('v'))? {
            bail!("Release version mismatch");
        }
    }
    if version <= current {
        println!("PMR {current} is up to date (selected release: {version}).");
        return Ok(());
    }
    let name = format!("pmr-v{version}-{}.tar.gz", target()?);
    let archive_url = asset_url(&release, &name)?;
    let checksum_url = asset_url(&release, &format!("{name}.sha256"))?;
    if check {
        println!(
            "PMR {version} is available (installed: {current}). Run pmr self-update to install."
        );
        return Ok(());
    }
    let archive = download(&client, &archive_url, MAX_DOWNLOAD)?;
    let checksum = download(&client, &checksum_url, 1024)?;
    verify_checksum(&archive, &checksum, &name)?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let directory = executable
        .parent()
        .ok_or_else(|| anyhow!("Cannot locate install directory"))?;
    install(&archive, directory, &version.to_string())
        .with_context(|| format!("Cannot update binaries in {}", directory.display()))?;
    println!("Installed PMR {version}. The running daemon is unchanged. Run pmr update when it is safe to restart your processes.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle(names: &[&str]) -> Vec<u8> {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(gzip);
        for name in names {
            let mut header = tar::Header::new_gnu();
            header.set_size(b"#!/bin/sh\nprintf '1.1.0\\n'\n".len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, name, &b"#!/bin/sh\nprintf '1.1.0\\n'\n"[..])
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }
    #[test]
    fn validates_checksums_and_exact_asset_origin() {
        let bytes = b"release";
        let checksum = format!("{}  pmr.tar.gz\n", digest_hex(bytes));
        verify_checksum(bytes, checksum.as_bytes(), "pmr.tar.gz").unwrap();
        assert!(verify_checksum(b"changed", checksum.as_bytes(), "pmr.tar.gz").is_err());
        assert!(verify_checksum(bytes, checksum.as_bytes(), "other").is_err());
        let release = Release {
            tag_name: "v1.1.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: "pmr.tar.gz".into(),
                browser_download_url: "https://example.com/pmr.tar.gz".into(),
            }],
        };
        assert!(asset_url(&release, "pmr.tar.gz").is_err());
    }
    #[test]
    fn incomplete_archive_leaves_install_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        for name in BINARIES {
            fs::write(dir.path().join(name), "old").unwrap();
        }
        assert!(install(&bundle(&["pmr"]), dir.path(), "1.1.0").is_err());
        for name in BINARIES {
            assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), "old");
        }
        assert!(install(&bundle(&BINARIES), dir.path(), "9.9.9").is_err());
        for name in BINARIES {
            assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), "old");
        }
        install(&bundle(&BINARIES), dir.path(), "1.1.0").unwrap();
        for name in BINARIES {
            assert_eq!(
                fs::read_to_string(dir.path().join(name)).unwrap(),
                "#!/bin/sh\nprintf '1.1.0\\n'\n"
            );
        }
    }
    #[test]
    fn duplicate_or_symlink_binaries_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(unpack(&bundle(&["pmr", "pmr"]), dir.path()).is_err());
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("other"), "old").unwrap();
            std::os::unix::fs::symlink(dir.path().join("other"), dir.path().join("pmr")).unwrap();
            assert!(install(&bundle(&BINARIES), dir.path(), "1.1.0").is_err());
            assert_eq!(fs::read_to_string(dir.path().join("other")).unwrap(), "old");
        }
    }
}
