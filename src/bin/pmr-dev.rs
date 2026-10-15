use anyhow::Result;
use clap::Parser;
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use pmr::client::PmrClient;
use pmr::config::{shell_words_split, StartOptions};
use pmr::constants::{LAUNCHING_STATUS, ONLINE_STATUS, PMR_VERSION, WAITING_RESTART};
use pmr::ux::{run_logs, LogOptions};

#[derive(Parser, Debug)]
#[command(
    name = "pmr-dev",
    version = PMR_VERSION,
    about = "PMR Development Mode with auto-watch and inline logs (drop-in replacement for pm2-dev)"
)]
struct DevCli {
    #[arg(long = "raw")]
    raw: bool,

    #[arg(long = "timestamp")]
    timestamp: bool,

    #[arg(long = "node-args")]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = DevCli::parse();
    let target = if cli.args.first().map(|s| s.as_str()) == Some("start") && cli.args.len() > 1 {
        cli.args[1].clone()
    } else {
        cli.args[0].clone()
    };

    let dev_home = env::var("HOME")
        .map(|h| PathBuf::from(h).join(".pmr-dev"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/.pmr-dev"));

    let client = PmrClient::new(Some(&dev_home));
    client.ensure_daemon(false).await?;

    let mut ignore_watch = vec!["node_modules".to_string()];
    if let Some(ref ig) = cli.ignore {
        for part in ig.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                ignore_watch.push(trimmed.to_string());
            }
        }
    }

    let opts = StartOptions {
        interpreter: cli.interpreter.clone(),
        node_args: cli
            .node_args
            .as_deref()
            .map(shell_words_split)
            .unwrap_or_default(),
        env_name: cli.env_name.clone(),
        watch: true,
        ignore_watch: ignore_watch.clone(),
        restart_delay: Some(1000),
        ..Default::default()
    };

    let procs = client.start_target(&target, &opts, Some(false)).await?;
    println!("===============================================================================");
    println!("--- PMR development mode ------------------------------------------------------");
    println!("Apps started         : {}", procs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
    println!("Processes started    : {}", procs.len());
    println!("Watch and Restart    : \x1b[32menabled\x1b[0m");
    println!("Ignored folder       : {}", ignore_watch.join(", "));
    if let Some(ref pe) = cli.post_exec {
        println!("Post restart cmd     : {}", pe);
    }
    println!("===============================================================================");

    if cli.test_mode {
        return Ok(());
    }

    if let Some(post_cmd) = cli.post_exec.clone() {
        let silent_exec = cli.silent_exec;
        if let Ok(mut rx) = client.subscribe_bus().await {
            tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    if ev.event == "process:event"
                        && ev.data.get("event").and_then(|v| v.as_str()) == Some("online")
                    {
                        let mut cmd = Command::new("sh");
                        cmd.arg("-c").arg(&post_cmd);
                        if silent_exec {
                            let _ = cmd.output();
                        } else {
                            let _ = cmd.status();
                        }
                    }
                }
            });
        }
    }

    if cli.auto_exit {
        let client_ae = client.clone();
        tokio::spawn(async move {
            let mut fail_count = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if let Ok(list) = client_ae.list().await {
                    let active = list.iter().any(|p| {
                        p.pm2_env.status == ONLINE_STATUS
                            || p.pm2_env.status == LAUNCHING_STATUS
                            || p.pm2_env.status == WAITING_RESTART
                    });
                    if !active {
                        fail_count += 1;
                        if fail_count >= 2 {
                            let _ = client_ae.kill_daemon().await;
                            std::process::exit(1);
                        }
                    } else {
                        fail_count = 0;
                    }
                }
            }
        });
    }

    let client_sig = client.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = client_sig.kill_daemon().await;
        std::process::exit(0);
    });

    run_logs(
        &client,
        LogOptions {
            target: "all".to_string(),
            lines: 15,
            raw: cli.raw,
            json: false,
            format: false,
            err_only: false,
            out_only: false,
            timestamp: if cli.timestamp {
                Some("%Y-%m-%d-%H:%M:%S".to_string())
            } else {
                None
            },
            nostream: false,
            highlight: None,
        },
    )
    .await?;

    Ok(())
}
