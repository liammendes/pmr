use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use sysinfo::System;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::client::PmrClient;
use crate::config::{AppConfig, StartOptions};
use crate::constants::DEFAULT_WEB_PORT;
use crate::paths::PmrPaths;
use crate::process::ProcessDescription;

pub async fn launch_web_app(
    client: &PmrClient,
    port: Option<u16>,
    opts: &StartOptions,
) -> Result<Vec<ProcessDescription>> {
    let web_port = port
        .or_else(|| env::var("PM2_WEB_PORT").ok().and_then(|p| p.parse().ok()))
        .or_else(|| env::var("PM2_API_PORT").ok().and_then(|p| p.parse().ok()))
        .unwrap_or(DEFAULT_WEB_PORT);

    let mut env = BTreeMap::new();
    env.insert("PM2_WEB_PORT".to_string(), web_port.to_string());

    let app = AppConfig {
        script: "__pmr_web__".to_string(),
        name: Some("pmr-http-interface".to_string()),
        env,
        ..Default::default()
    };

    println!(
        "{}Launching web interface on port {}...",
        crate::constants::PREFIX_MSG,
        web_port
    );
    client
        .start_app_config(app, opts, &BTreeMap::new(), None)
        .await
}

pub async fn run_web_server(paths: PmrPaths) -> Result<()> {
    let host = env::var("PM2_API_IPADDR").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PM2_WEB_PORT")
        .or_else(|_| env::var("PM2_API_PORT"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_WEB_PORT);
    let strip_env = env::var("PM2_WEB_STRIP_ENV_VARS")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    let client = PmrClient::new(Some(&paths.pm2_home));

    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let client_c = client.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut buf_reader = BufReader::new(reader);
            let mut request_line = String::new();
            if buf_reader.read_line(&mut request_line).await.is_err() || request_line.is_empty() {
                return;
            }
            let parts: Vec<&str> = request_line.split_whitespace().collect();
            let path = parts.get(1).copied().unwrap_or("/");

            if path != "/" {
                let body = b"{\"err\":\"404\"}";
                let header = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = writer.write_all(header.as_bytes()).await;
                let _ = writer.write_all(body).await;
                return;
            }

            let mut procs = client_c.list().await.unwrap_or_default();
            if strip_env {
                for p in &mut procs {
                    p.pm2_env.env.clear();
                }
            }

            let mut sys = System::new_all();
            sys.refresh_all();
            let load = System::load_average();
            let cpus: Vec<_> = sys
                .cpus()
                .iter()
                .map(|c| {
                    json!({
                        "model": c.brand(),
                        "speed": c.frequency(),
                    })
                })
                .collect();

            let payload = json!({
                "system_info": {
                    "hostname": System::host_name().unwrap_or_default(),
                    "uptime": System::uptime(),
                },
                "monit": {
                    "loadavg": [load.one, load.five, load.fifteen],
                    "total_mem": sys.total_memory(),
                    "free_mem": sys.available_memory(),
                    "cpu": cpus,
                },
                "processes": procs,
            });

            let body = serde_json::to_vec(&payload).unwrap_or_default();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Cache-Control, Pragma, Origin, Authorization, Content-Type, X-Requested-With\r\nAccess-Control-Allow-Methods: GET\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = writer.write_all(header.as_bytes()).await;
            let _ = writer.write_all(&body).await;
        });
    }
}
