use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn pmr_bin() -> String {
    env!("CARGO_BIN_EXE_pmr").to_string()
}

fn run_pmr(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(pmr_bin())
        .args(args)
        .env("PMR_HOME", home)
        .env("PM2_HOME", home)
        .env("NO_COLOR", "1")
        .output()
        .expect("Failed to execute pmr")
}

#[test]
fn test_lifecycle_start_list_describe_stop_restart_delete() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".pmr");
    let script = tmp.path().join("worker.sh");
    fs::write(
        &script,
        "#!/bin/sh\necho \"hello stdout\"\necho \"hello stderr\" >&2\nwhile true; do sleep 1; done\n",
    )
    .unwrap();

    // Start process
    let out = run_pmr(
        &home,
        &[
            "start",
            script.to_str().unwrap(),
            "--name",
            "alpha-worker",
            "--namespace",
            "prod",
        ],
    );
    assert!(
        out.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Check jlist
    let jout = run_pmr(&home, &["jlist"]);
    assert!(jout.status.success());
    let procs: Vec<Value> = serde_json::from_slice(&jout.stdout).unwrap();
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0]["name"], "alpha-worker");
    assert_eq!(procs[0]["pm2_env"]["namespace"], "prod");
    assert_eq!(procs[0]["pm2_env"]["status"], "online");
    let pid1 = procs[0]["pid"].as_u64().unwrap();
    assert!(pid1 > 0);

    // Wait briefly for stdout/stderr logs to flush
    thread::sleep(Duration::from_millis(300));

    // Check logs --nostream
    let lout = run_pmr(&home, &["logs", "alpha-worker", "--nostream", "--raw"]);
    let log_text = String::from_utf8_lossy(&lout.stdout);
    assert!(log_text.contains("hello stdout"), "logs output: {}", log_text);
    assert!(log_text.contains("hello stderr"), "logs output: {}", log_text);

    // Check describe
    let dout = run_pmr(&home, &["describe", "alpha-worker"]);
    assert!(dout.status.success());
    let desc_str = String::from_utf8_lossy(&dout.stdout);
    assert!(desc_str.contains("Describing process with id 0 - name alpha-worker"));
    assert!(desc_str.contains("prod"));

    // Restart process
    let rout = run_pmr(&home, &["restart", "alpha-worker"]);
    assert!(rout.status.success());
    let jout2 = run_pmr(&home, &["jlist"]);
    let procs2: Vec<Value> = serde_json::from_slice(&jout2.stdout).unwrap();
    assert_eq!(procs2[0]["pm2_env"]["restart_time"], 1);

    // Stop process
    let sout = run_pmr(&home, &["stop", "alpha-worker"]);
    assert!(sout.status.success());
    let jout3 = run_pmr(&home, &["jlist"]);
    let procs3: Vec<Value> = serde_json::from_slice(&jout3.stdout).unwrap();
    assert_eq!(procs3[0]["pm2_env"]["status"], "stopped");
    assert_eq!(procs3[0]["pid"], 0);

    // Delete process
    let del_out = run_pmr(&home, &["delete", "alpha-worker"]);
    assert!(del_out.status.success());
    let jout4 = run_pmr(&home, &["jlist"]);
    let procs4: Vec<Value> = serde_json::from_slice(&jout4.stdout).unwrap();
    assert!(procs4.is_empty());

    // Kill daemon
    let kout = run_pmr(&home, &["kill"]);
    assert!(kout.status.success());
}

#[test]
fn test_ecosystem_env_scale_save_and_resurrect() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".pmr");
    let script = tmp.path().join("app.sh");
    fs::write(&script, "#!/bin/sh\nwhile true; do sleep 1; done\n").unwrap();

    let eco_path = tmp.path().join("ecosystem.config.js");
    let eco_content = format!(
        r#"module.exports = {{
  apps: [{{
    name: "web-cluster",
    script: "{}",
    instances: 2,
    exec_mode: "cluster",
    env: {{
      NODE_ENV: "development",
      API_KEY: "dev-key"
    }},
    env_production: {{
      NODE_ENV: "production",
      API_KEY: "prod-key"
    }}
  }}]
}};"#,
        script.display()
    );
    fs::write(&eco_path, eco_content).unwrap();

    let out = run_pmr(
        &home,
        &[
            "start",
            eco_path.to_str().unwrap(),
            "--env",
            "production",
        ],
    );
    assert!(out.status.success());

    let jout = run_pmr(&home, &["jlist"]);
    let procs: Vec<Value> = serde_json::from_slice(&jout.stdout).unwrap();
    assert_eq!(procs.len(), 2);
    assert_eq!(procs[0]["pm2_env"]["env"]["NODE_ENV"], "production");
    assert_eq!(procs[0]["pm2_env"]["env"]["API_KEY"], "prod-key");
    assert_eq!(procs[0]["pm2_env"]["exec_mode"], "cluster_mode");

    // Scale up by +1 -> 3 instances
    let sout = run_pmr(&home, &["scale", "web-cluster", "+1"]);
    assert!(sout.status.success());
    let jout2 = run_pmr(&home, &["jlist"]);
    let procs2: Vec<Value> = serde_json::from_slice(&jout2.stdout).unwrap();
    assert_eq!(procs2.len(), 3);

    // Save / dump
    let save_out = run_pmr(&home, &["save"]);
    assert!(save_out.status.success());
    assert!(home.join("dump.pm2").exists());

    // Delete all
    let del_out = run_pmr(&home, &["delete", "all"]);
    assert!(del_out.status.success());
    let jout3 = run_pmr(&home, &["jlist"]);
    let procs3: Vec<Value> = serde_json::from_slice(&jout3.stdout).unwrap();
    assert_eq!(procs3.len(), 0);

    // Resurrect
    let res_out = run_pmr(&home, &["resurrect"]);
    assert!(res_out.status.success());
    let jout4 = run_pmr(&home, &["jlist"]);
    let procs4: Vec<Value> = serde_json::from_slice(&jout4.stdout).unwrap();
    assert_eq!(procs4.len(), 3);

    let _ = run_pmr(&home, &["kill"]);
}

#[test]
fn test_module_configuration_set_get_unset() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".pmr");

    let out1 = run_pmr(&home, &["set", "pmr-logrotate:max_size", "10M"]);
    assert!(out1.status.success());

    let out2 = run_pmr(&home, &["get", "pmr-logrotate:max_size"]);
    assert!(out2.status.success());
    assert_eq!(String::from_utf8_lossy(&out2.stdout).trim(), "10M");

    let out3 = run_pmr(&home, &["unset", "pmr-logrotate:max_size"]);
    assert!(out3.status.success());

    let _ = run_pmr(&home, &["kill"]);
}

#[test]
fn test_crash_autorestart_and_max_restarts_overlimit() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".pmr");
    let crash_script = tmp.path().join("crasher.sh");
    fs::write(&crash_script, "#!/bin/sh\necho \"crashing\"\nexit 1\n").unwrap();

    // Start with max_restarts = 2 and min_uptime = 5000ms
    let out = run_pmr(
        &home,
        &[
            "start",
            crash_script.to_str().unwrap(),
            "--name",
            "crasher",
            "--max-restarts",
            "2",
            "--min-uptime",
            "5000",
        ],
    );
    assert!(out.status.success());

    // Wait for crash loop to hit max_restarts (2) and transition to "errored"
    let mut final_status = String::new();
    for _ in 0..30 {
        thread::sleep(Duration::from_millis(100));
        let jout = run_pmr(&home, &["jlist"]);
        if let Ok(procs) = serde_json::from_slice::<Vec<Value>>(&jout.stdout) {
            if let Some(st) = procs.first().and_then(|p| p["pm2_env"]["status"].as_str()) {
                final_status = st.to_string();
                if final_status == "errored" {
                    break;
                }
            }
        }
    }
    assert_eq!(final_status, "errored");
    let _ = run_pmr(&home, &["kill"]);
}

#[test]
fn test_stop_exit_codes_skips_autorestart() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".pmr");
    let script = tmp.path().join("graceful_exit.sh");
    fs::write(&script, "#!/bin/sh\nexit 42\n").unwrap();

    let out = run_pmr(
        &home,
        &[
            "start",
            script.to_str().unwrap(),
            "--name",
            "exit42",
            "--stop-exit-codes",
            "42",
        ],
    );
    assert!(out.status.success());

    thread::sleep(Duration::from_millis(300));
    let jout = run_pmr(&home, &["jlist"]);
    let procs: Vec<Value> = serde_json::from_slice(&jout.stdout).unwrap();
    assert_eq!(procs[0]["pm2_env"]["status"], "stopped");
    assert_eq!(procs[0]["pm2_env"]["restart_time"], 0);
    let _ = run_pmr(&home, &["kill"]);
}

#[test]
fn test_node_cluster_mode_port_sharing_ready_and_zero_downtime_reload() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".pmr");
    let server_js = tmp.path().join("server.js");

    // Pick an available local port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let js_code = format!(
        r#"const http = require('http');
const server = http.createServer((req, res) => {{
  res.writeHead(200, {{ 'Content-Type': 'application/json' }});
  res.end(JSON.stringify({{ pid: process.pid, instance: process.env.NODE_APP_INSTANCE }}));
}});
server.listen({}, '127.0.0.1', () => {{
  if (process.send) {{
    process.send('ready');
    process.send({{
      type: 'axm:monitor',
      data: {{ 'Active Requests': {{ value: 7, unit: 'req' }} }}
    }});
  }}
}});
"#,
        port
    );
    fs::write(&server_js, js_code).unwrap();

    let out = run_pmr(
        &home,
        &[
            "start",
            server_js.to_str().unwrap(),
            "-i",
            "2",
            "--name",
            "node-cluster",
            "--wait-ready",
        ],
    );
    assert!(
        out.status.success(),
        "start node cluster failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    thread::sleep(Duration::from_millis(400));

    let jout = run_pmr(&home, &["jlist"]);
    let procs: Vec<Value> = serde_json::from_slice(&jout.stdout).unwrap();
    assert_eq!(procs.len(), 2);
    assert_eq!(procs[0]["pm2_env"]["status"], "online");
    assert_eq!(procs[1]["pm2_env"]["status"], "online");
    assert!(procs[0]["pm2_env"]["node_version"].is_string());

    // Zero-downtime reload
    let rout = run_pmr(&home, &["reload", "node-cluster"]);
    assert!(rout.status.success());

    let jout2 = run_pmr(&home, &["jlist"]);
    let procs2: Vec<Value> = serde_json::from_slice(&jout2.stdout).unwrap();
    assert_eq!(procs2.len(), 2);
    assert_eq!(procs2[0]["pm2_env"]["status"], "online");
    assert_eq!(procs2[0]["pm2_env"]["restart_time"], 1);

    let _ = run_pmr(&home, &["kill"]);
}

#[test]
fn test_serve_and_web_api() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".pmr");
    let pub_dir = tmp.path().join("public");
    fs::create_dir_all(&pub_dir).unwrap();
    fs::write(pub_dir.join("index.html"), "<h1>Hello from PMR Serve</h1>").unwrap();

    let l1 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let serve_port = l1.local_addr().unwrap().port();
    drop(l1);

    let l2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let web_port = l2.local_addr().unwrap().port();
    drop(l2);

    let out1 = run_pmr(
        &home,
        &[
            "serve",
            pub_dir.to_str().unwrap(),
            &serve_port.to_string(),
            "--spa",
        ],
    );
    assert!(out1.status.success());

    let out2 = run_pmr(&home, &["web", &web_port.to_string()]);
    assert!(out2.status.success());

    thread::sleep(Duration::from_millis(500));

    // Fetch from static server (SPA route fallback)
    let resp_serve = Command::new("curl")
        .args(["-s", &format!("http://127.0.0.1:{}/some/spa/route", serve_port)])
        .output()
        .unwrap();
    let body_serve = String::from_utf8_lossy(&resp_serve.stdout);
    assert!(
        body_serve.contains("Hello from PMR Serve"),
        "serve body: {}",
        body_serve
    );

    // Fetch from web monitoring API
    let resp_web = Command::new("curl")
        .args(["-s", &format!("http://127.0.0.1:{}/", web_port)])
        .output()
        .unwrap();
    let web_json: Value = serde_json::from_slice(&resp_web.stdout).unwrap();
    assert!(web_json["system_info"]["hostname"].is_string());
    assert!(web_json["processes"].is_array());
    assert_eq!(web_json["processes"].as_array().unwrap().len(), 2);

    let _ = run_pmr(&home, &["kill"]);
}
