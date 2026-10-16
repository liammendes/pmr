use serde_json::{json, Value};
use std::fs::{self, File, FileTimes};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, UNIX_EPOCH};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    home: PathBuf,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("pmr");
        let config = temp.path().join("ecosystem.json");
        let apps: Vec<_> = (0..count)
            .map(|i| {
                json!({
                    "name": format!("worker-{:02}", count - i - 1),
                    "script": "/bin/sh",
                    "interpreter": "none",
                    "autostart": false,
                    "autorestart": false,
                    "vizion": false,
                    "namespace": if i % 2 == 0 { "even" } else { "odd" },
                })
            })
            .collect();
        fs::write(&config, json!({"apps": apps}).to_string()).unwrap();
        let fixture = Self { _temp: temp, home };
        if count > 0 {
            fixture.run(&["start", config.to_str().unwrap()]);
        } else {
            fixture.run(&["ping"]);
        }
        fixture
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pmr"));
        for (key, _) in std::env::vars() {
            if key.starts_with("PM2_") || key.starts_with("PMR_") {
                cmd.env_remove(key);
            }
        }
        cmd.args(args)
            .env("PMR_HOME", &self.home)
            .env("PM2_HOME", &self.home)
            .env("NO_COLOR", "1");
        cmd
    }

    fn run(&self, args: &[&str]) -> String {
        let out = self.command(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn processes(&self) -> Vec<Value> {
        serde_json::from_str(&self.run(&["jlist"])).unwrap()
    }

    fn write_logs(&self) {
        for proc in self.processes() {
            let id = proc["pm_id"].as_u64().unwrap();
            for (offset, field, stream) in
                [(0, "pm_out_log_path", "out"), (1, "pm_err_log_path", "err")]
            {
                let path = PathBuf::from(proc["pm2_env"][field].as_str().unwrap());
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, format!("old-{id}-{stream}\nlog-{id}-{stream}\n")).unwrap();
                File::options()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_times(
                        FileTimes::new()
                            .set_modified(UNIX_EPOCH + Duration::from_secs(100 + id * 2 + offset)),
                    )
                    .unwrap();
            }
        }
        File::options()
            .write(true)
            .open(self.home.join("pm2.log"))
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)))
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.command(&["kill"]).output();
    }
}

fn mini_ids(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("pm2 id : "))
        .map(|id| id.parse().unwrap())
        .collect()
}

#[test]
fn list_limits_sort_before_truncating_and_preserve_json_inventory() {
    let fixture = Fixture::new(30);
    for alias in ["list", "ls", "l", "ps", "status"] {
        let output = fixture.run(&[alias, "-m"]);
        assert_eq!(mini_ids(&output), (5..30).rev().collect::<Vec<_>>());
        assert!(output.contains("Showing 25 of 30 processes"));
    }
    let table = fixture.run(&["ls"]);
    let table_ids: Vec<u32> = table
        .lines()
        .filter_map(|line| line.split('│').nth(1)?.trim().parse().ok())
        .collect();
    assert_eq!(table_ids, (5..30).rev().collect::<Vec<_>>());
    assert_eq!(
        mini_ids(&fixture.run(&["ls", "-m", "--limit", "3", "--sort", "asc"])),
        vec![0, 1, 2]
    );
    assert_eq!(
        mini_ids(&fixture.run(&["ls", "-m", "--limit", "3", "--sort", "desc"])),
        vec![29, 28, 27]
    );
    assert_eq!(
        mini_ids(&fixture.run(&["ls", "-m", "--limit", "2", "--sort", "name:asc"])),
        vec![29, 28]
    );
    let unlimited = fixture.run(&["ls", "-m", "--limit", "0", "--sort", "asc"]);
    assert_eq!(mini_ids(&unlimited), (0..30).collect::<Vec<_>>());
    assert!(!unlimited.contains("Showing"));
    assert_eq!(
        mini_ids(&fixture.run(&["ls", "-m", "--limit", "99"])).len(),
        30
    );
    assert_eq!(fixture.processes().len(), 30);
    let pretty: Vec<Value> = serde_json::from_str(&fixture.run(&["prettylist"])).unwrap();
    assert_eq!(pretty.len(), 30);
}

#[test]
fn logs_limit_unique_files_by_modification_time_after_filtering() {
    let fixture = Fixture::new(4);
    fixture.write_logs();
    let raw = fixture.run(&["logs", "--raw", "--nostream", "--lines", "1"]);
    assert_eq!(
        raw.lines().collect::<Vec<_>>(),
        vec![
            "log-3-err",
            "log-3-out",
            "log-2-err",
            "log-2-out",
            "log-1-err"
        ]
    );
    let plain = fixture.run(&["logs", "--nostream", "--lines", "1"]);
    assert_eq!(
        plain
            .lines()
            .filter(|line| line.ends_with("last 1 lines:"))
            .count(),
        5
    );
    assert_eq!(
        fixture.run(&[
            "logs",
            "--raw",
            "--nostream",
            "--lines",
            "1",
            "--sort",
            "asc",
            "--limit",
            "2"
        ]),
        "log-0-out\nlog-0-err\n"
    );
    assert_eq!(
        fixture.run(&[
            "logs",
            "--raw",
            "--nostream",
            "--lines",
            "1",
            "--sort",
            "desc",
            "--limit",
            "1"
        ]),
        "log-3-err\n"
    );
    assert_eq!(
        fixture.run(&[
            "logs",
            "--raw",
            "--nostream",
            "--lines",
            "1",
            "--out",
            "--limit",
            "2"
        ]),
        "log-3-out\nlog-2-out\n"
    );
    assert_eq!(
        fixture.run(&[
            "logs",
            "--raw",
            "--nostream",
            "--lines",
            "1",
            "--err",
            "--limit",
            "2"
        ]),
        "log-3-err\nlog-2-err\n"
    );
    for target in ["even", "/worker-0[13]/"] {
        assert_eq!(
            fixture.run(&[
                "logs",
                target,
                "--raw",
                "--nostream",
                "--lines",
                "1",
                "--limit",
                "2"
            ]),
            "log-2-err\nlog-2-out\n"
        );
    }
    assert_eq!(
        fixture.run(&["logs", "0", "--raw", "--nostream", "--lines", "1"]),
        "log-0-err\nlog-0-out\n"
    );
    assert_eq!(
        fixture.run(&["logs", "worker-03", "--raw", "--nostream", "--lines", "1"]),
        "log-0-err\nlog-0-out\n"
    );
    assert_eq!(
        fixture
            .run(&[
                "logs",
                "--raw",
                "--nostream",
                "--lines",
                "1",
                "--limit",
                "0"
            ])
            .lines()
            .count(),
        8
    );
    assert!(fixture
        .run(&["logs", "missing", "--raw", "--nostream"])
        .is_empty());
}

#[test]
fn invalid_limits_and_log_sort_fail_clearly() {
    let fixture = Fixture::new(0);
    for command in ["ls", "logs"] {
        for value in ["-1", "many"] {
            let Output { status, .. } = fixture
                .command(&[command, "--limit", value])
                .output()
                .unwrap();
            assert!(!status.success());
        }
    }
    let out = fixture
        .command(&["logs", "--sort", "sideways", "--nostream"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("logs --sort must be asc or desc"));
}

#[test]
fn shared_log_paths_count_as_one_file() {
    let fixture = Fixture::new(0);
    let shared = fixture._temp.path().join("shared.log");
    let other = fixture._temp.path().join("other.log");
    for (name, path) in [("first", &shared), ("second", &shared), ("third", &other)] {
        fixture.run(&[
            "start",
            "/bin/sh",
            "--name",
            name,
            "--no-autostart",
            "--no-vizion",
            "--output",
            path.to_str().unwrap(),
            "--error",
            path.to_str().unwrap(),
        ]);
    }
    fs::write(&shared, "shared\n").unwrap();
    fs::write(&other, "other\n").unwrap();
    for path in [&shared, &other] {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(100)))
            .unwrap();
    }
    // Equal timestamps use path order so repeated runs select the same files.
    assert_eq!(
        fixture.run(&[
            "logs",
            "--raw",
            "--nostream",
            "--limit",
            "2",
            "--sort",
            "asc"
        ]),
        "other\nshared\n"
    );
    assert_eq!(
        fixture.run(&[
            "logs",
            "--raw",
            "--nostream",
            "--limit",
            "2",
            "--sort",
            "desc"
        ]),
        "shared\nother\n"
    );
}

struct StreamChild(std::process::Child);

impl Drop for StreamChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn live_logs_follow_only_the_selected_files_even_with_no_history() {
    let fixture = Fixture::new(0);
    for name in ["first", "second"] {
        fixture.run(&["start", "/bin/sh", "--name", name, "--interpreter", "none", "--no-autorestart", "--no-vizion", "--", "-c",
            "while IFS= read -r line; do printf 'out-%s\n' \"$line\"; printf 'err-%s\n' \"$line\" >&2; done"]);
    }
    fixture.write_logs();
    let capture = fixture._temp.path().join("capture.txt");
    let mut cmd = fixture.command(&["logs", "--raw", "--lines", "0", "--out", "--limit", "1"]);
    let _stream = StreamChild(cmd.stdout(File::create(&capture).unwrap()).spawn().unwrap());
    // Repeat until the subscriber is connected, bounded by the deadline.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        fixture.run(&["send", "1", "included"]);
        std::thread::sleep(Duration::from_millis(100));
        let text = fs::read_to_string(&capture).unwrap();
        assert!(!text.contains("excluded"), "{text}");
        assert!(!text.contains("err-"), "{text}");
        if text.contains("out-included") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "No selected live log received: {text}"
        );
    }
    fixture.run(&["send", "0", "excluded"]);
    let excluded_log = PathBuf::from(
        fixture.processes()[0]["pm2_env"]["pm_out_log_path"]
            .as_str()
            .unwrap(),
    );
    while !fs::read_to_string(&excluded_log)
        .unwrap()
        .contains("out-excluded")
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    fixture.run(&["send", "1", "finished"]);
    loop {
        let text = fs::read_to_string(&capture).unwrap();
        assert!(!text.contains("excluded"), "{text}");
        assert!(!text.contains("err-"), "{text}");
        if text.contains("out-finished") {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn latest_log_lookup_returns_one_small_record() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let fixture = Fixture::new(30);
    let paths = pmr::PmrPaths::new(Some(&fixture.home));
    for (target, expected) in [
        ("all", 29),
        ("even", 28),
        ("worker-29", 0),
        ("/worker-0[12]/", 28),
    ] {
        let mut stream = UnixStream::connect(&paths.daemon_rpc_port).unwrap();
        writeln!(
            stream,
            "{}",
            json!({"method":"getLatestLogProcess","params":{"target":target}})
        )
        .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["data"]["pm_id"], expected);
        assert!(response["data"].get("pm2_env").is_none());
        assert!(
            line.len() < 2048,
            "The latest record must not contain the full inventory or environment"
        );
    }
}

#[test]
fn daemon_limits_process_rows_before_returning_them() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let fixture = Fixture::new(30);
    let paths = pmr::PmrPaths::new(Some(&fixture.home));
    for (limit, descending, expected) in [
        (3, true, vec![29, 28, 27]),
        (3, false, vec![0, 1, 2]),
        (0, false, (0..30).collect()),
        (99, true, (0..30).rev().collect()),
    ] {
        let mut stream = UnixStream::connect(&paths.daemon_rpc_port).unwrap();
        writeln!(
            stream,
            "{}",
            json!({
                "method": "getProcessList", "params": {"limit": limit, "descending": descending}
            })
        )
        .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["data"]["total"], 30);
        let ids: Vec<_> = response["data"]["processes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["pm_id"].as_u64().unwrap())
            .collect();
        assert_eq!(ids, expected);
    }
}

#[test]
fn live_mode_rejects_incompatible_output_flags() {
    let fixture = Fixture::new(0);
    for option in ["--raw", "--json", "--format", "--nostream"] {
        let out = fixture
            .command(&["logs", "--live", option])
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be used with"));
    }
    let out = fixture.command(&["logs", "--live"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("interactive terminal"));
}
