use anyhow::Result;
use clap::Parser;
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use pmr::client::PmrClient;
use pmr::config::{shell_words_split, StartOptions};
use pmr::constants::{ERROR_EXIT, LAUNCHING_STATUS, ONLINE_STATUS, PMR_VERSION};
use pmr::ux::{run_logs, LogOptions};

#[derive(Parser, Debug)]
#[command(
    name = "pmr-dev",
    version = PMR_VERSION,
    about = "PMR Development Mode with auto-watch and inline logs (drop-in replacement for pm2-dev)",
    args_override_self = true
)]
struct DevCli {
    #[arg(long = "raw")]
    raw: bool,

    #[arg(long = "timestamp")]
    timestamp: bool,

    #[arg(
        long = "node-args",
        alias = "interpreter-args",
        allow_hyphen_values = true
    )]
    node_args: Option<String>,

    #[arg(long = "ignore")]
    ignore: Option<String>,

    #[arg(long = "post-exec")]
    post_exec: Option<String>,

    #[arg(long = "silent-exec")]
    silent_exec: bool,

    #[arg(long = "test-mode")]
    test_mode: bool,

    #[arg(long = "interpreter")]
    interpreter: Option<String>,

    #[arg(long = "env")]
    env_name: Option<String>,

    #[arg(long = "auto-exit")]
    auto_exit: bool,

    #[arg(required = true)]
    args: Vec<String>,

    #[arg(last = true)]
    script_args: Vec<String>,
}

#[tokio::main]
async fn main() {
    let cli = match DevCli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let kind = e.kind();
            let _ = e.print();
            if matches!(
                kind,
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                std::process::exit(0);
            } else {
                std::process::exit(ERROR_EXIT);
            }
        }
    };

    if let Err(e) = run_dev(cli).await {
        eprintln!("[PMR-Dev] Error: {}", e);
        std::process::exit(ERROR_EXIT);
    }
}

async fn run_dev(cli: DevCli) -> Result<()> {
    let (target, mut extra_args) =
        if cli.args.first().map(|s| s.as_str()) == Some("start") && cli.args.len() > 1 {
            (cli.args[1].clone(), cli.args[2..].to_vec())
        } else {
            (cli.args[0].clone(), cli.args[1..].to_vec())
        };
    extra_args.extend(cli.script_args.clone());

    let dev_home = env::var("PMR_HOME")
        .or_else(|_| env::var("PM2_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::var("HOME")
                .map(|h| PathBuf::from(h).join(".pm2-dev"))
                .unwrap_or_else(|_| PathBuf::from("/tmp/.pm2-dev"))
        });

    let client = PmrClient::new(Some(&dev_home));
    client.ensure_daemon(false).await?;

    let client_sig = client.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigint = signal(SignalKind::interrupt()).ok();
            let mut sigterm = signal(SignalKind::terminate()).ok();
            tokio::select! {
                _ = async { if let Some(ref mut s) = sigint { s.recv().await } else { std::future::pending().await } } => {},
                _ = async { if let Some(ref mut s) = sigterm { s.recv().await } else { std::future::pending().await } } => {},
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = client_sig.kill_daemon().await;
        std::process::exit(0);
    });

    let mut ignore_watch = vec!["node_modules".to_string()];
    if let Some(ref ig) = cli.ignore {
        for part in ig.split(|c: char| c == ',' || c.is_whitespace()) {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                ignore_watch.push(trimmed.to_string());
            }
        }
    }

    let opts = StartOptions {
        args: extra_args,
        interpreter: cli.interpreter.clone(),
        node_args: cli
            .node_args
            .as_deref()
            .map(shell_words_split)
            .unwrap_or_default(),
        env_name: cli.env_name.clone(),
        watch: true,
        ignore_watch: ignore_watch.clone(),
        force_autorestart: true,
        force_autostart: true,
        ..Default::default()
    };

    let post_cmd = cli.post_exec.clone();
    let silent_exec = cli.silent_exec;
    if let Ok(mut rx) = client.subscribe_bus().await {
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if ev.event == "process:event" {
                    let ev_type = ev
                        .data
                        .get("event")
                        .and_then(|v| v.as_str())
                        .or(ev.status.as_deref());
                    if ev_type == Some("restart") {
                        println!(
                            ">> Process {} restarted",
                            ev.process.name
                        );
                    }
                    if ev_type == Some("online") {
                        if let Some(ref cmd_str) = post_cmd {
                            let mut cmd = Command::new("sh");
                            cmd.arg("-c").arg(cmd_str);
                            if silent_exec {
                                let _ = cmd.output();
                            } else {
                                let _ = cmd.status();
                            }
                        }
                    }
                }
            }
        });
    }

    let use_color = env::var("NO_COLOR").is_err();
    let client_logs = client.clone();
    let log_opts = LogOptions {
        target: "all".to_string(),
        lines: 0,
        raw: cli.raw,
        json: false,
        format: false,
        err_only: false,
        out_only: false,
        timestamp: if cli.timestamp {
            Some("YYYY-MM-DD-HH:mm:ss".to_string())
        } else {
            None
        },
        nostream: false,
        highlight: None,
        use_color,
    };
    let logs_handle = tokio::spawn(async move {
        let _ = run_logs(&client_logs, log_opts).await;
    });

    tokio::time::sleep(Duration::from_millis(25)).await;

    let procs = client.start_target(&target, &opts, Some(false)).await?;
    println!("===============================================================================");
    println!("--- PMR development mode ------------------------------------------------------");
    println!(
        "Apps started         : {}",
        procs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("Processes started    : {}", procs.len());
    println!("Watch and Restart    : \x1b[32menabled\x1b[0m");
    println!("Ignored folder       : {}", ignore_watch.join(", "));
    if let Some(ref pe) = cli.post_exec {
        println!("Post restart cmd     : {}", pe);
    }
    println!("===============================================================================");

    if cli.test_mode {
        let _ = client.kill_daemon().await;
        return Ok(());
    }

    if cli.auto_exit {
        let client_ae = client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let mut fail_count = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if let Ok(list) = client_ae.list().await {
                    let active = list.iter().any(|p| {
                        p.pm2_env.status == ONLINE_STATUS
                            || p.pm2_env.status == LAUNCHING_STATUS
                    });
                    if !active {
                        fail_count += 1;
                        if fail_count >= 2 {
                            let _ = client_ae.kill_daemon().await;
                            std::process::exit(0);
                        }
                    } else {
                        fail_count = 0;
                    }
                }
            }
        });
    }

    let _ = logs_handle.await;
    Ok(())
}
