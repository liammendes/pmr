use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("pmr");
        let fixture = Self { _temp: temp, home };
        fixture.run(&["ping"]);
        fixture
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pmr"));
        for (key, _) in std::env::vars() {
            if key.starts_with("PMR_") || key.starts_with("PM2_") {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .env("PMR_HOME", &self.home)
            .env("PM2_HOME", &self.home)
            .env("NO_COLOR", "1");
        command
    }
    fn output(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn run(&self, args: &[&str]) -> String {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.run(args)).unwrap()
    }
    fn submit(&self, name: &str, script: &str) -> String {
        self.json(&["run", "/bin/sh", "--name", name, "--", "-c", script])["id"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn wait(&self, id: &str, status: &str) {
        let start = Instant::now();
        loop {
            let row = self.json(&["history", "show", id]);
            if row["status"] == status {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "expected {status}: {row}"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.output(&["kill"]);
    }
}

#[test]
fn cluster_reload_at_capacity_preserves_the_original_worker() {
    let f = Fixture::new();
    let script = f._temp.path().join("worker.cjs");
    std::fs::write(&script, "setInterval(() => {}, 1000);").unwrap();
    f.run(&["limits", "--running", "1"]);
    f.run(&[
        "start",
        script.to_str().unwrap(),
        "--name",
        "worker",
        "--interpreter",
        "node",
        "-i",
        "1",
        "--no-vizion",
    ]);
    let before = f.json(&["jlist"]);
    let pid = before[0]["pid"].as_u64().unwrap();
    assert!(pid > 0);
    let reload = f.output(&["reload", "worker"]);
    assert!(!reload.status.success());
    let after = f.json(&["jlist"]);
    assert_eq!(after[0]["pid"], pid);
    assert_eq!(after[0]["pm2_env"]["status"], "online");
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
}

#[test]
fn real_output_is_capped_and_retains_the_tail() {
    let f = Fixture::new();
    f.run(&["retention", "--output-bytes", "4096"]);
    let id = f.submit(
        "output",
        "i=0; while [ \"$i\" -lt 2000 ]; do printf 'line-%04d\\n' \"$i\"; i=$((i+1)); done",
    );
    f.wait(&id, "succeeded");
    let row = f.json(&["history", "show", &id]);
    assert!(row["output_bytes"].as_u64().unwrap() <= 4096);
    assert!(row["dropped_bytes"].as_u64().unwrap() > 0);
    let tail = f.run(&["history", "logs", &id, "--lines", "2"]);
    assert!(tail.contains("line-1999"), "{tail}");
    assert!(!tail.contains("line-0000"));
}

#[test]
fn shared_capacity_queue_backpressure_and_cancellation() {
    let f = Fixture::new();
    f.run(&["limits", "--running", "2", "--queued", "2"]);
    f.run(&[
        "start",
        "/bin/sleep",
        "--name",
        "service",
        "--interpreter",
        "none",
        "--no-autorestart",
        "--",
        "30",
    ]);
    let first = f.submit("first", "sleep 30");
    f.wait(&first, "running");
    let second = f.submit("second", "echo second");
    let third = f.submit("third", "echo third");
    f.wait(&second, "queued");
    f.wait(&third, "queued");
    let full = f.output(&["run", "/bin/echo", "--", "overflow"]);
    assert!(!full.status.success());
    assert!(String::from_utf8_lossy(&full.stderr).contains("queue is full"));
    assert_eq!(f.json(&["limits"])["running"], 2);
    f.run(&["history", "cancel", &first]);
    f.wait(&first, "cancelled");
    f.wait(&second, "succeeded");
    f.wait(&third, "succeeded");
    assert_eq!(f.run(&["history", "logs", &third]).trim(), "third");
    assert_eq!(f.json(&["jlist"]).as_array().unwrap().len(), 1);
}

#[test]
fn wait_exit_code_idempotency_and_indexed_filters() {
    let f = Fixture::new();
    let id = uuid::Uuid::new_v4().to_string();
    for _ in 0..2 {
        f.run(&[
            "run",
            "/bin/echo",
            "--id",
            &id,
            "--name",
            "needle",
            "--namespace",
            "tests",
            "--",
            "once",
        ]);
    }
    f.wait(&id, "succeeded");
    let filtered = f.json(&[
        "history", "list", "--search", "needle", "--sort", "asc", "--limit", "1",
    ]);
    assert_eq!(filtered["records"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["records"][0]["id"], id);
    assert_eq!(f.json(&["history"])["records"].as_array().unwrap().len(), 1);
    let failed = f.output(&[
        "run",
        "/bin/sh",
        "--wait",
        "--",
        "-c",
        "echo failed-output; exit 7",
    ]);
    assert_eq!(failed.status.code(), Some(7));
    assert!(String::from_utf8_lossy(&failed.stdout).contains("failed-output"));
    assert_eq!(f.json(&["retention"])["settings"]["history_days"], 0);
    assert_eq!(f.json(&["history", "prune"])["records_removed"], 0);
}

#[test]
fn graceful_restart_keeps_queue_and_does_not_replay_started_work() {
    let f = Fixture::new();
    f.run(&["limits", "--running", "1"]);
    let running = f.submit("started", "echo before; sleep 30");
    f.wait(&running, "running");
    let pending = f.submit("pending", "echo after-restart");
    f.wait(&pending, "queued");
    f.run(&["kill"]);
    let start = Instant::now();
    while f.home.join("rpc.sock").exists() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(30));
    }
    f.run(&["ping"]);
    f.wait(&running, "interrupted");
    f.wait(&pending, "succeeded");
    assert_eq!(
        f.run(&["history", "logs", &pending]).trim(),
        "after-restart"
    );
}

#[test]
fn concurrent_submissions_complete_with_bounded_active_state() {
    let f = Fixture::new();
    f.run(&["limits", "--running", "4", "--queued", "100"]);
    let mut submissions = Vec::new();
    for n in 0..24 {
        let mut command = f.command(&[
            "run",
            "/bin/sh",
            "--name",
            &format!("job-{n}"),
            "--",
            "-c",
            "sleep 0.1; echo done",
        ]);
        submissions.push(std::thread::spawn(move || command.output().unwrap()));
    }
    let ids: Vec<String> = submissions
        .into_iter()
        .map(|thread| {
            let output = thread.join().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    let start = Instant::now();
    loop {
        let limits = f.json(&["limits"]);
        assert!(limits["running"].as_u64().unwrap() <= 4);
        let history = f.json(&["history", "list", "--limit", "100"]);
        if history["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "succeeded")
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(ids.len(), 24);
    assert!(f.json(&["jlist"]).as_array().unwrap().is_empty());
    assert_eq!(f.json(&["limits"])["active_commands"], 0);
}
