use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::client::PmrClient;
use crate::config::{AppConfig, StartOptions};
use crate::constants::DEFAULT_SERVE_PORT;
use crate::process::ProcessDescription;

#[allow(clippy::too_many_arguments)]
pub async fn launch_serve_app(
    client: &PmrClient,
    target_path: Option<&str>,
    port: Option<u16>,
    spa: bool,
    basic_user: Option<String>,
    basic_pass: Option<String>,
    custom_name: Option<String>,
    opts: &StartOptions,
) -> Result<Vec<ProcessDescription>> {
    let serve_port = port
        .or_else(|| env::var("PM2_SERVE_PORT").ok().and_then(|p| p.parse().ok()))
        .unwrap_or(DEFAULT_SERVE_PORT);
    let raw_path = target_path
        .map(PathBuf::from)
        .or_else(|| env::var("PM2_SERVE_PATH").ok().map(PathBuf::from))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let serve_path = fs::canonicalize(&raw_path).unwrap_or(raw_path);

    let app_name = custom_name
        .or_else(|| opts.name.clone())
        .unwrap_or_else(|| format!("static-page-server-{}", serve_port));

    let mut env = BTreeMap::new();
    env.insert("PM2_SERVE_PORT".to_string(), serve_port.to_string());
    env.insert(
        "PM2_SERVE_PATH".to_string(),
        serve_path.to_string_lossy().to_string(),
    );
    env.insert("PM2_SERVE_SPA".to_string(), spa.to_string());
    if let (Some(u), Some(p)) = (basic_user, basic_pass) {
        env.insert("PM2_SERVE_BASIC_AUTH".to_string(), "true".to_string());
        env.insert("PM2_SERVE_BASIC_AUTH_USERNAME".to_string(), u);
        env.insert("PM2_SERVE_BASIC_AUTH_PASSWORD".to_string(), p);
    }

    let app = AppConfig {
        script: "__pmr_serve__".to_string(),
        name: Some(app_name),
        cwd: Some(serve_path.to_string_lossy().to_string()),
        env,
        ..Default::default()
    };

    client
        .start_app_config(app, opts, &BTreeMap::new(), None)
        .await
}

pub async fn run_static_server() -> Result<()> {
    let host = env::var("PM2_SERVE_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PM2_SERVE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SERVE_PORT);
    let root_str = env::var("PM2_SERVE_PATH").unwrap_or_else(|_| ".".to_string());
    let root_dir = fs::canonicalize(&root_str).unwrap_or_else(|_| PathBuf::from(&root_str));
    let spa = env::var("PM2_SERVE_SPA")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let homepage = env::var("PM2_SERVE_HOMEPAGE").unwrap_or_else(|_| "/index.html".to_string());
    let basic_auth = env::var("PM2_SERVE_BASIC_AUTH")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let basic_user = env::var("PM2_SERVE_BASIC_AUTH_USERNAME").unwrap_or_default();
    let basic_pass = env::var("PM2_SERVE_BASIC_AUTH_PASSWORD").unwrap_or_default();
    let expected_auth = if basic_auth {
        let token = BASE64.encode(format!("{}:{}", basic_user, basic_pass));
        Some(format!("Basic {}", token))
    } else {
        None
    };

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    println!("Exposing {} on {}", root_dir.display(), addr);

    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let root = root_dir.clone();
        let homepage_c = homepage.clone();
        let expected_auth_c = expected_auth.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut buf_reader = BufReader::new(reader);
            let mut request_line = String::new();
            if buf_reader.read_line(&mut request_line).await.is_err() || request_line.is_empty() {
                return;
            }

            let mut auth_header = None;
            loop {
                let mut hline = String::new();
                if buf_reader.read_line(&mut hline).await.is_err() {
                    break;
                }
                if hline == "\r\n" || hline == "\n" || hline.is_empty() {
                    break;
                }
                if let Some((k, v)) = hline.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("authorization") {
                        auth_header = Some(v.trim().to_string());
                    }
                }
            }

            if let Some(ref exp) = expected_auth_c {
                if auth_header.as_ref() != Some(exp) {
                    let resp = "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"Authentication service\"\r\nContent-Type: text/plain\r\nContent-Length: 12\r\nConnection: close\r\n\r\nUnauthorized";
                    let _ = writer.write_all(resp.as_bytes()).await;
                    return;
                }
            }

            let parts: Vec<&str> = request_line.split_whitespace().collect();
            let raw_uri = parts.get(1).copied().unwrap_or("/");
            let clean_uri = raw_uri.split('?').next().unwrap_or("/");
            let rel_path = if clean_uri == "/" {
                homepage_c.trim_start_matches('/')
            } else {
                clean_uri.trim_start_matches('/')
            };

            // Reject path traversal
            if rel_path.contains("..") {
                let body = b"403 Forbidden";
                let header = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = writer.write_all(header.as_bytes()).await;
                let _ = writer.write_all(body).await;
                return;
            }

            let mut target_file = root.join(rel_path);
            if target_file.is_dir() {
                target_file = target_file.join("index.html");
            }

            let (status_line, mime, body) = if target_file.is_file() {
                match fs::read(&target_file) {
                    Ok(bytes) => ("HTTP/1.1 200 OK", content_type_for_path(&target_file), bytes),
                    Err(_) => (
                        "HTTP/1.1 500 Internal Server Error",
                        "text/plain",
                        b"500 Internal Server Error".to_vec(),
                    ),
                }
            } else if spa {
                let fallback = root.join(homepage_c.trim_start_matches('/'));
                if let Ok(bytes) = fs::read(&fallback) {
                    ("HTTP/1.1 200 OK", "text/html", bytes)
                } else {
                    ("HTTP/1.1 404 Not Found", "text/plain", b"404 Not Found".to_vec())
                }
            } else {
                let custom_404 = root.join("404.html");
                if let Ok(bytes) = fs::read(&custom_404) {
                    ("HTTP/1.1 404 Not Found", "text/html", bytes)
                } else {
                    ("HTTP/1.1 404 Not Found", "text/plain", b"404 Not Found".to_vec())
                }
            };

            let header = format!(
                "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET\r\nConnection: close\r\n\r\n",
                status_line,
                mime,
                body.len()
            );
            let _ = writer.write_all(header.as_bytes()).await;
            let _ = writer.write_all(&body).await;
        });
    }
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html",
        "js" | "mjs" | "cjs" => "text/javascript",
        "css" => "text/css",
        "json" | "map" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "pdf" => "application/pdf",
        "xml" => "application/xml",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "webp" => "image/webp",
        _ => "text/plain",
    }
}
