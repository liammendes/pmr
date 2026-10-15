use anyhow::Result;
use clap::Parser;
use std::time::Duration;

use pmr::client::PmrClient;
use pmr::config::{parse_byte_size_str, shell_words_split, StartOptions};
use pmr::constants::{
    DEFAULT_WEB_PORT, LAUNCHING_STATUS, ONLINE_STATUS, PMR_VERSION, WAITING_RESTART,
};
use pmr::paths::PmrPaths;
use pmr::ux::{run_logs, LogOptions};
use pmr::web::launch_web_app;

#[derive(Parser, Debug)]
#[command(
    name = "pmr-runtime",
    version = PMR_VERSION,
    about = "PMR Container & Foreground Runtime (drop-in replacement for pm2-runtime)"
)]
struct RuntimeCli {
    #[arg(short = 'i', long = "instances")]
    instances: Option<String>,

    #[arg(long = "no-autostart")]
    no_autostart: bool,

    #[arg(long = "no-autorestart")]
    no_autorestart: bool,

    #[arg(long = "stop-exit-codes")]
    stop_exit_codes: Vec<i32>,

    #[arg(long = "node-args")]
    node_args: Option<String>,

    #[arg(short = 'n', long = "name")]
    name: Option<String>,

    #[arg(long = "max-memory-restart")]
    max_memory_restart: Option<String>,

    #[arg(short = 'c', long = "cron")]
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

    #[arg(long = "delay", default_value_t = 0)]
    delay: u64,

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

    /// Either `start <target>` or `<target>`
    #[arg(required = true)]
    args: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = RuntimeCli::parse();
    let target = if cli.args.first().map(|s| s.as_str()) == Some("start") && cli.args.len() > 1 {
        cli.args[1].clone()
    } else {
        cli.args[0].clone()
    };

    let paths = PmrPaths::default();
    let client = PmrClient::new(Some(&paths.pm2_home));
    client.ensure_daemon(true).await?;

    if cli.delay > 0 {
        tokio::time::sleep(Duration::from_secs(cli.delay)).await;
    }

    let instances = cli.instances.as_deref().map(|s| {
        if s.eq_ignore_ascii_case("max") {
            0
        } else {
            s.parse::<isize>().unwrap_or(1)
        }
    });

    let opts = StartOptions {
        name: cli.name.clone(),
        interpreter: cli.interpreter.clone(),
        node_args: cli
            .node_args
            .as_deref()
            .map(shell_words_split)
            .unwrap_or_default(),
        instances,
        output: cli.output.clone(),
        error: cli.error.clone(),
        env_name: cli.env_name.clone(),
        max_memory_restart: cli
            .max_memory_restart
            .as_deref()
            .and_then(parse_byte_size_str),
        cron_restart: cli.cron.clone(),
        watch: cli.watch,
        no_autorestart: cli.no_autorestart,
        no_autostart: cli.no_autostart,
        stop_exit_codes: cli.stop_exit_codes.clone(),
        only: cli.only.clone(),
        ..Default::default()
    };

    client.start_target(&target, &opts, None).await?;

    if let Some(port_opt) = cli.web {
        let port = port_opt.unwrap_or(DEFAULT_WEB_PORT);
        let _ = launch_web_app(&client, Some(port), &StartOptions::default()).await;
    }

    // Auto-exit monitor task unless --no-auto-exit
    if !cli.no_auto_exit {
        let client_mon = client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut fail_count = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                if let Ok(procs) = client_mon.list().await {
                    let active = procs.iter().any(|p| {
                        !p.pm2_env.pmx_module
                            && (p.pm2_env.status == ONLINE_STATUS
                                || p.pm2_env.status == LAUNCHING_STATUS
                                || p.pm2_env.status == WAITING_RESTART)
                    });
                    if !active {
                        fail_count += 1;
                        if fail_count >= 2 {
                            let _ = client_mon.kill_daemon().await;
                            std::process::exit(0);
                        }
                    } else {
                        fail_count = 0;
                    }
                }
            }
        });
    }

    // Clean shutdown on Ctrl-C
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
            lines: 0,
            raw: !cli.formatted && !cli.json && !cli.format,
            json: cli.json,
            format: cli.format,
            err_only: false,
            out_only: false,
            timestamp: None,
            nostream: false,
            highlight: None,
        },
    )
    .await?;

    Ok(())
}
