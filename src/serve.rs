use anyhow::Result;
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
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
    host: Option<String>,
    spa: bool,
    basic_user: Option<String>,
    basic_pass: Option<String>,
    monitor: Option<String>,
    custom_name: Option<String>,
    opts: &StartOptions,
) -> Result<Vec<ProcessDescription>> {
    let serve_port = port
        .or_else(|| env::var("PM2_SERVE_PORT").ok().and_then(|p| p.parse().ok()))
        .unwrap_or(DEFAULT_SERVE_PORT);
    let serve_host = host
        .or_else(|| env::var("PM2_SERVE_HOST").ok())
        .unwrap_or_else(|| "0.0.0.0".to_string());
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
    env.insert("PM2_SERVE_HOST".to_string(), serve_host);
    env.insert(
        "PM2_SERVE_PATH".to_string(),
        serve_path.to_string_lossy().to_string(),
    );
    env.insert("PM2_SERVE_SPA".to_string(), spa.to_string());
    if let Some(m) = monitor {
        env.insert("PM2_SERVE_MONITOR".to_string(), m);
    }
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

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            out.push(((hi << 4) | lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn resolve_safe_relative_path(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
        }
    }
    let joined = root.join(&normalized);
    if joined.exists() {
        if let Ok(canon) = fs::canonicalize(&joined) {
            if !canon.starts_with(root) {
                return None;
            }
            return Some(canon);
        }
    }
    Some(joined)
}

fn check_basic_auth(header_val: Option<&str>, expected_user: &str, expected_pass: &str) -> bool {
    let Some(raw) = header_val else {
        return false;
    };
    let mut parts = raw.splitn(2, char::is_whitespace);
    let Some(scheme) = parts.next() else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Some(token) = parts.next().map(|s| s.trim().trim_end_matches('=')) else {
        return false;
    };
    let Ok(decoded_bytes) = STANDARD_NO_PAD.decode(token) else {
        return false;
    };
    let Ok(decoded_str) = String::from_utf8(decoded_bytes) else {
        return false;
    };
    let Some((u, p)) = decoded_str.split_once(':') else {
        return false;
    };
    u == expected_user && p == expected_pass
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
    let monitor_bucket = env::var("PM2_SERVE_MONITOR").ok().filter(|s| !s.is_empty());

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    println!("Exposing {} on {}", root_dir.display(), addr);

    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let root = root_dir.clone();
        let homepage_c = homepage.clone();
        let b_user = basic_user.clone();
        let b_pass = basic_pass.clone();
        let mon_bucket = monitor_bucket.clone();

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

            let parts: Vec<&str> = request_line.split_whitespace().collect();
            let method = parts.first().copied().unwrap_or("GET");
            let is_head = method.eq_ignore_ascii_case("HEAD");

            if basic_auth && !check_basic_auth(auth_header.as_deref(), &b_user, &b_pass) {
                let header = "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"Authentication service\"\r\nContent-Type: text/plain\r\nContent-Length: 12\r\nConnection: close\r\n\r\n";
                let _ = writer.write_all(header.as_bytes()).await;
                if !is_head {
                    let _ = writer.write_all(b"Unauthorized").await;
                }
                return;
            }

            if !method.eq_ignore_ascii_case("GET") && !is_head {
                let body = b"405 Method Not Allowed";
                let header = format!(
                    "HTTP/1.1 405 Method Not Allowed\r\nAllow: GET, HEAD\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = writer.write_all(header.as_bytes()).await;
                if !is_head {
                    let _ = writer.write_all(body).await;
                }
                return;
            }

            let raw_uri = parts.get(1).copied().unwrap_or("/");
            let clean_uri = raw_uri.split('?').next().unwrap_or("/");
            let Some(decoded_uri) = percent_decode(clean_uri) else {
                let body = b"400 Bad Request";
                let header = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = writer.write_all(header.as_bytes()).await;
                if !is_head {
                    let _ = writer.write_all(body).await;
                }
                return;
            };

            let rel_path = if decoded_uri == "/" {
                homepage_c.trim_start_matches('/')
            } else {
                decoded_uri.trim_start_matches('/')
            };

            let is_within_root = |p: &Path| -> bool {
                fs::canonicalize(p)
                    .map(|c| c.starts_with(&root))
                    .unwrap_or(false)
            };

            let Some(mut target_file) = resolve_safe_relative_path(&root, rel_path) else {
                let body = b"403 Forbidden";
                let header = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = writer.write_all(header.as_bytes()).await;
                if !is_head {
                    let _ = writer.write_all(body).await;
                }
                return;
            };

            if target_file.is_dir() {
                target_file = target_file.join(homepage_c.trim_start_matches('/'));
            }

            let (status_line, mime, body) = if target_file.exists() {
                if !is_within_root(&target_file) {
                    (
                        "HTTP/1.1 403 Forbidden",
                        "text/plain",
                        b"403 Forbidden".to_vec(),
                    )
                } else if target_file.is_file() {
                    match fs::read(&target_file) {
                        Ok(bytes) => ("HTTP/1.1 200 OK", content_type_for_path(&target_file), bytes),
                        Err(_) => (
                            "HTTP/1.1 500 Internal Server Error",
                            "text/plain",
                            b"500 Internal Server Error".to_vec(),
                        ),
                    }
                } else {
                    (
                        "HTTP/1.1 404 Not Found",
                        "text/plain",
                        b"404 Not Found".to_vec(),
                    )
                }
            } else if spa {
                // First try basename in root dir for nested SPA asset paths (e.g. /users/app.js -> /app.js)
                let basename_candidate = Path::new(rel_path)
                    .file_name()
                    .map(|f| root.join(f))
                    .filter(|p| p.exists());
                if let Some(asset_path) = basename_candidate {
                    if !is_within_root(&asset_path) {
                        (
                            "HTTP/1.1 403 Forbidden",
                            "text/plain",
                            b"403 Forbidden".to_vec(),
                        )
                    } else if asset_path.is_file() {
                        if let Ok(bytes) = fs::read(&asset_path) {
                            ("HTTP/1.1 200 OK", content_type_for_path(&asset_path), bytes)
                        } else {
                            ("HTTP/1.1 404 Not Found", "text/plain", b"404 Not Found".to_vec())
                        }
                    } else {
                        ("HTTP/1.1 404 Not Found", "text/plain", b"404 Not Found".to_vec())
                    }
                } else {
                    let fallback = root.join(homepage_c.trim_start_matches('/'));
                    if fallback.exists() && !is_within_root(&fallback) {
                        (
                            "HTTP/1.1 403 Forbidden",
                            "text/plain",
                            b"403 Forbidden".to_vec(),
                        )
                    } else if let Ok(bytes) = fs::read(&fallback) {
                        ("HTTP/1.1 200 OK", content_type_for_path(&fallback), bytes)
                    } else {
                        (
                            "HTTP/1.1 404 Not Found",
                            "text/plain",
                            b"404 Not Found".to_vec(),
                        )
                    }
                }
            } else {
                let custom_404 = root.join("404.html");
                if custom_404.exists() && is_within_root(&custom_404) {
                    if let Ok(bytes) = fs::read(&custom_404) {
                        ("HTTP/1.1 404 Not Found", "text/html", bytes)
                    } else {
                        (
                            "HTTP/1.1 404 Not Found",
                            "text/plain",
                            b"404 Not Found".to_vec(),
                        )
                    }
                } else {
                    (
                        "HTTP/1.1 404 Not Found",
                        "text/plain",
                        b"404 Not Found".to_vec(),
                    )
                }
            };

            let mut body = body;
            if status_line == "HTTP/1.1 200 OK" && mime == "text/html" {
                if let Some(ref bucket) = mon_bucket {
                    if let Ok(html_str) = String::from_utf8(body.clone()) {
                        let snippet = format!(
                            "<script src=\"https://pm2.io/grid-agent.js\" data-bucket=\"{}\"></script>",
                            bucket
                        );
                        let injected = if let Some(idx) = html_str.find("</head>") {
                            format!("{}{}{}", &html_str[..idx], snippet, &html_str[idx..])
                        } else {
                            format!("{}{}", html_str, snippet)
                        };
                        body = injected.into_bytes();
                    }
                }
            }

            let header = format!(
                "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, HEAD\r\nConnection: close\r\n\r\n",
                status_line,
                mime,
                body.len()
            );
            let _ = writer.write_all(header.as_bytes()).await;
            if !is_head {
                let _ = writer.write_all(&body).await;
            }
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
        "webmanifest" => "application/manifest+json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "wasm" => "application/wasm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "sfnt" => "application/font-sfnt",
        "pdf" => "application/pdf",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "md" => "text/markdown",
        "txt" => "text/plain",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        _ => "text/plain",
    }
}
