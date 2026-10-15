use anyhow::{anyhow, Result};
use clap::{ArgAction, Parser};
use std::env;
use std::time::Duration;

use pmr::client::PmrClient;
use pmr::config::{parse_byte_size_str, shell_words_split, StartOptions};
use pmr::constants::{
    DEFAULT_WEB_PORT, ERROR_EXIT, LAUNCHING_STATUS, ONLINE_STATUS, PMR_VERSION,
};
use pmr::paths::PmrPaths;
use pmr::ux::{run_logs, LogOptions};
use pmr::web::launch_web_app;

#[derive(Parser, Debug)]
#[command(
    name = "pmr-runtime",
    version = PMR_VERSION,
    about = "PMR Container & Foreground Runtime (drop-in replacement for pm2-runtime)",
    args_override_self = true
)]
struct RuntimeCli {
    #[arg(short = 'i', long = "instances", allow_hyphen_values = true)]
    instances: Option<String>,

    #[arg(long = "no-autostart")]
    no_autostart: bool,

    #[arg(long = "no-autorestart")]
    no_autorestart: bool,

    #[arg(long = "stop-exit-codes", action = ArgAction::Append)]
    stop_exit_codes: Vec<i32>,

    #[arg(
        long = "node-args",
        alias = "interpreter-args",
        allow_hyphen_values = true
    )]
    node_args: Option<String>,

    #[arg(short = 'n', long = "name")]
    name: Option<String>,

    #[arg(long = "max-memory-restart")]
    max_memory_restart: Option<String>,

    #[arg(short = 'c', long = "cron", visible_alias = "cron-restart")]
    cron: Option<String>,

    #[arg(long = "interpreter")]
    interpreter: Option<String>,

    #[arg(long = "format")]
    format: bool,

    #[arg(long = "raw")]
    raw: bool,

    #[arg(long = "formatted")]
    formatted: bool,

    #[arg(long = "json")]
    json: bool,

    #[arg(long = "delay", default_value_t = 0.0)]
    delay: f64,

    #[arg(long = "web")]
    web: Option<Option<u16>>,

    #[arg(long = "only")]
    only: Option<String>,

    #[arg(long = "no-auto-exit")]
    no_auto_exit: bool,

    #[arg(long = "env")]
    env_name: Option<String>,

    #[arg(long = "watch")]
    watch: bool,

    #[arg(long = "error")]
    error: Option<String>,

    #[arg(long = "output")]
    output: Option<String>,

    #[arg(long = "secret")]
    secret: Option<String>,

    #[arg(long = "public")]
    public: Option<String>,

    #[arg(long = "machine-name")]
    machine_name: Option<String>,

    #[arg(long = "trace")]
    trace: bool,

    #[arg(long = "v8")]
    v8: bool,

    #[arg(long = "deep-monitoring")]
    deep_monitoring: bool,

    /// Either `start <target>` or `<target>`
    #[arg(required = true)]
    args: Vec<String>,

    /// Trailing arguments passed to script after `--`
    #[arg(last = true)]
    script_args: Vec<String>,
}

#[tokio::main]
async fn main() {
    let cli = match RuntimeCli::try_parse() {
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

    if let Err(e) = run_runtime(cli).await {
        eprintln!("[PMR-Runtime] Error: {}", e);
        std::process::exit(ERROR_EXIT);
    }
}

async fn run_runtime(cli: RuntimeCli) -> Result<()> {
    let (target, mut extra_args) =
        if cli.args.first().map(|s| s.as_str()) == Some("start") && cli.args.len() > 1 {
            (cli.args[1].clone(), cli.args[2..].to_vec())
        } else {
            (cli.args[0].clone(), cli.args[1..].to_vec())
        };
    extra_args.extend(cli.script_args.clone());

    let paths = PmrPaths::default();
    let client = PmrClient::new(Some(&paths.pm2_home));
    client.ensure_daemon(true).await?;

    // Clean shutdown on SIGINT or SIGTERM
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

    if cli.delay > 0.0 && cli.delay.is_finite() {
        tokio::time::sleep(Duration::from_secs_f64(cli.delay)).await;
    }

    let instances = match cli.instances.as_deref() {
        Some(s) => {
            if s.eq_ignore_ascii_case("max") {
                Some(0)
            } else {
                Some(
                    s.parse::<isize>()
                        .map_err(|_| anyhow!("Invalid instances value: {}", s))?,
                )
            }
        }
        None => None,
    };

    let max_memory_restart = match cli.max_memory_restart.as_deref() {
        Some(s) => Some(
            parse_byte_size_str(s)
                .ok_or_else(|| anyhow!("Invalid max-memory-restart value: {}", s))?,
        ),
        None => None,
    };

    let opts = StartOptions {
        name: cli.name.clone(),
        args: extra_args,
        interpreter: cli.interpreter.clone(),
        node_args: cli
            .node_args
            .as_deref()
            .map(shell_words_split)
            .unwrap_or_default(),
        instances,
        output: Some(cli.output.clone().unwrap_or_else(|| "/dev/null".to_string())),
        error: Some(cli.error.clone().unwrap_or_else(|| "/dev/null".to_string())),
        env_name: cli.env_name.clone(),
        max_memory_restart,
        cron_restart: cli.cron.clone(),
        watch: cli.watch,
        no_autorestart: cli.no_autorestart,
        no_autostart: cli.no_autostart,
        stop_exit_codes: cli.stop_exit_codes.clone(),
        only: cli.only.clone(),
        trace: if cli.trace || cli.deep_monitoring {
            Some(true)
        } else {
            None
        },
        v8: cli.v8 || cli.deep_monitoring,
        deep_monitoring: cli.deep_monitoring,
        ..Default::default()
    };

    let _ = (&cli.secret, &cli.public, &cli.machine_name);
    let use_color = env::var("NO_COLOR").is_err();

    // Start log streaming task BEFORE launching target so startup output is captured immediately
    let client_logs = client.clone();
    let log_opts = LogOptions {
        target: "all".to_string(),
        lines: 0,
        raw: !cli.formatted && !cli.json && !cli.format,
        json: cli.json,
        format: cli.format,
        err_only: false,
        out_only: false,
        timestamp: None,
        nostream: false,
        highlight: None,
        use_color,
    };
    let logs_handle = tokio::spawn(async move {
        let _ = run_logs(&client_logs, log_opts).await;
    });

    // Give bus subscriber a brief moment to connect before launching fast scripts
    tokio::time::sleep(Duration::from_millis(25)).await;

    let procs = client.start_target(&target, &opts, None).await?;
    if procs.is_empty() {
        let _ = client.kill_daemon().await;
        return Err(anyhow!("No application started"));
    }

    if let Some(port_opt) = cli.web {
        let port = port_opt.unwrap_or(DEFAULT_WEB_PORT);
        launch_web_app(&client, Some(port), &StartOptions::default()).await?;
    }

    // Auto-exit monitor task matching Runtime4Docker.js (5s initial delay, 3s interval, 3 retries -> exit(2))
    if !cli.no_auto_exit {
        let client_mon = client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5000)).await;
            let mut fail_count = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(3000)).await;
                if let Ok(procs) = client_mon.list().await {
                    let active = procs.iter().any(|p| {
                        !p.pm2_env.pmx_module
                            && (p.pm2_env.status == ONLINE_STATUS
                                || p.pm2_env.status == LAUNCHING_STATUS)
                    });
                    if !active {
                        fail_count += 1;
                        if fail_count >= 3 {
                            let _ = client_mon.kill_daemon().await;
                            std::process::exit(2);
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
