use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::path::PathBuf;

use pmr::client::PmrClient;
use pmr::config::{
    parse_byte_size_str, parse_duration_ms_str, shell_words_split, ModuleConfiguration,
    StartOptions,
};
use pmr::constants::{
    APP_CONF_DEFAULT_FILE, ECOSYSTEM_SIMPLE_TPL, ECOSYSTEM_TPL, ERROR_EXIT, PMR_VERSION,
    PREFIX_MSG, PREFIX_MSG_ERR, PREFIX_MSG_WARNING, SUCCESS_EXIT,
};
use pmr::daemon::run_daemon;
use pmr::deploy::{cmd_deploy, cmd_git_version_action};
use pmr::modules::{cmd_generate_module_sample, cmd_install_module, cmd_uninstall_module};
use pmr::paths::PmrPaths;
use pmr::protocol::RpcRequest;
use pmr::serve::{launch_serve_app, run_static_server};
use pmr::startup::{
    cmd_cleardump, cmd_dump, cmd_logrotate, cmd_resurrect, cmd_startup, cmd_unstartup,
};
use pmr::tui::{run_dashboard, run_imonit};
use pmr::ux::{
    print_report, render_describe, render_process_list, render_tree, run_logs, LogOptions,
};
use pmr::web::{launch_web_app, run_web_server};

#[derive(Parser, Debug)]
#[command(
    name = "pmr",
    version = PMR_VERSION,
    about = "PMR - Production Process Manager in Rust (PM2 compatible)",
    disable_version_flag = true
)]
struct Cli {
    /// Print pmr version
    #[arg(short = 'v', short_alias = 'V', long = "version", global = true)]
    version: bool,

    /// Hide all messages
    #[arg(short = 's', long = "silent", global = true)]
    silent: bool,

    /// Set a name for the process in the process list
    #[arg(short = 'n', long = "name", global = true)]
    name: Option<String>,

    /// Display a compacted list without formatting
    #[arg(short = 'm', long = "mini-list", global = true)]
    mini_list: bool,

    /// Set a specific interpreter to use for executing app
    #[arg(long = "interpreter", global = true)]
    interpreter: Option<String>,

    /// Set arguments to pass to the interpreter (alias of --node-args)
    #[arg(long = "interpreter-args", alias = "node-args", global = true)]
    node_args: Option<String>,

    /// Specify log file for stdout
    #[arg(short = 'o', long = "output", global = true)]
    output: Option<String>,

    /// Specify log file for stderr
    #[arg(short = 'e', long = "error", global = true)]
    error: Option<String>,

    /// Specify log file which gathers both stdout and stderr
    #[arg(short = 'l', long = "log", global = true)]
    log: Option<String>,

    /// Filter out outgoing global values that contain provided strings
    #[arg(long = "filter-env", global = true)]
    filter_env: Vec<String>,

    /// Specify log output style (raw by default, json optional)
    #[arg(long = "log-type", global = true)]
    log_type: Option<String>,

    /// Add custom prefix timestamp to logs
    #[arg(long = "log-date-format", global = true)]
    log_date_format: Option<String>,

    /// Enable time logging
    #[arg(long = "time", global = true)]
    time: bool,

    /// Disable all logs storage
    #[arg(long = "disable-logs", global = true)]
    disable_logs: bool,

    /// Specify which set of environment variables from ecosystem file must be injected
    #[arg(long = "env", global = true)]
    env_name: Option<String>,

    /// Force an update of the environment with restart/reload
    #[arg(short = 'a', long = "update-env", global = true)]
    update_env: bool,

    /// Force actions
    #[arg(short = 'f', long = "force", global = true)]
    force: bool,

    /// Launch [number] instances (for networked app)(load balanced), 0 or 'max' for CPU count
    #[arg(short = 'i', long = "instances", global = true)]
    instances: Option<String>,

    /// Shutdown an application with process.send('shutdown') instead of SIGINT
    #[arg(long = "shutdown-with-message", global = true)]
    shutdown_with_message: bool,

    /// Specify pid file
    #[arg(short = 'p', long = "pid", global = true)]
    pid: Option<String>,

    /// Delay before sending final SIGKILL signal to process (in ms)
    #[arg(short = 'k', long = "kill-timeout", global = true)]
    kill_timeout: Option<u64>,

    /// Listen timeout on application reload (in ms)
    #[arg(long = "listen-timeout", global = true)]
    listen_timeout: Option<u64>,

    /// Restart the app if an amount of memory is exceeded (e.g. 150M, 1G)
    #[arg(long = "max-memory-restart", global = true)]
    max_memory_restart: Option<String>,

    /// Specify a delay between restarts (in milliseconds)
    #[arg(long = "restart-delay", global = true)]
    restart_delay: Option<String>,

    /// Specify an exponential backoff delay between restarts (in milliseconds)
    #[arg(long = "exp-backoff-restart-delay", global = true)]
    exp_backoff_restart_delay: Option<String>,

    /// Execute a program using fork system
    #[arg(short = 'x', long = "execute-command", global = true)]
    execute_command: bool,

    /// Only restart the script COUNT times
    #[arg(long = "max-restarts", global = true)]
    max_restarts: Option<u32>,

    /// Minimum uptime in ms (or e.g. 5s, 1m) to be considered started
    #[arg(long = "min-uptime", global = true)]
    min_uptime: Option<String>,

    /// Define user when generating startup script or running process
    #[arg(short = 'u', long = "user", global = true)]
    user: Option<String>,

    /// Run target script with <uid> rights
    #[arg(long = "uid", global = true)]
    uid: Option<u32>,

    /// Run target script with <gid> rights
    #[arg(long = "gid", global = true)]
    gid: Option<u32>,

    /// Start application within specified namespace
    #[arg(long = "namespace", global = true)]
    namespace: Option<String>,

    /// Run target script from path <cwd>
    #[arg(long = "cwd", global = true)]
    cwd: Option<String>,

    /// Define home path when generating startup script
    #[arg(long = "hp", global = true)]
    hp: Option<String>,

    /// Override systemd script to wait for full internet connectivity
    #[arg(long = "wait-ip", global = true)]
    wait_ip: bool,

    /// Define service name when generating startup script
    #[arg(long = "service-name", global = true)]
    service_name: Option<String>,

    /// Restart a running process based on a cron pattern
    #[arg(short = 'c', long = "cron", visible_alias = "cron-restart", global = true)]
    cron: Option<String>,

    /// Run pmr daemon in the foreground if it doesn't exist already
    #[arg(long = "no-daemon", global = true)]
    no_daemon: bool,

    /// With json declaration, allow to only act on one application
    #[arg(long = "only", global = true)]
    only: Option<String>,

    /// Ask pmr to wait for ready event from your app
    #[arg(long = "wait-ready", global = true)]
    wait_ready: bool,

    /// Merge logs from different instances
    #[arg(long = "merge-logs", global = true)]
    merge_logs: bool,

    /// Watch application folder for changes
    #[arg(long = "watch", global = true)]
    watch: bool,

    /// List of paths to ignore when watching
    #[arg(long = "ignore-watch", global = true)]
    ignore_watch: Vec<String>,

    /// Specify a restart delay after changing files
    #[arg(long = "watch-delay", global = true)]
    watch_delay: Option<String>,

    /// Skip colors
    #[arg(long = "no-color", global = true)]
    no_color: bool,

    /// Start an app without vizion feature (versioning control)
    #[arg(long = "no-vizion", global = true)]
    no_vizion: bool,

    /// Add an app without automatic start
    #[arg(long = "no-autostart", global = true)]
    no_autostart: bool,

    /// Start an app without automatic restart
    #[arg(long = "no-autorestart", global = true)]
    no_autorestart: bool,

    /// Specify exit codes that should skip automatic restart
    #[arg(long = "stop-exit-codes", global = true)]
    stop_exit_codes: Vec<i32>,

    /// Only kill the main process, not detached children
    #[arg(long = "no-treekill", global = true)]
    no_treekill: bool,

    /// Start an app without pmx
    #[arg(long = "no-pmx", visible_alias = "no-automation", global = true)]
    no_pmx: bool,

    /// Sort processes according to field's name (e.g. name:asc, cpu:desc, memory:desc)
    #[arg(long = "sort", global = true)]
    sort: Option<String>,

    /// Attach logging after start/restart/stop/reload
    #[arg(long = "attach", global = true)]
    attach: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start and daemonize an app
    #[command(name = "start")]
    Start {
        targets: Vec<String>,
        #[arg(last = true)]
        script_args: Vec<String>,
    },

    /// Start or restart JSON/ecosystem file
    #[command(name = "startOrRestart")]
    StartOrRestart {
        targets: Vec<String>,
        #[arg(last = true)]
        script_args: Vec<String>,
    },

    /// Start or gracefully reload JSON/ecosystem file
    #[command(name = "startOrReload", visible_alias = "startOrGracefulReload")]
    StartOrReload {
        targets: Vec<String>,
        #[arg(last = true)]
        script_args: Vec<String>,
    },

    /// Stop a process
    #[command(name = "stop")]
    Stop { targets: Vec<String> },

    /// Restart a process
    #[command(name = "restart")]
    Restart {
        targets: Vec<String>,
        #[arg(last = true)]
        script_args: Vec<String>,
    },

    /// Reload processes (0-downtime for cluster mode)
    #[command(name = "reload", visible_alias = "gracefulReload")]
    Reload { targets: Vec<String> },

    /// Stop and delete a process from pmr process list
    #[command(name = "delete", visible_alias = "del")]
    Delete { targets: Vec<String> },

    /// Scale up/down a process in cluster mode
    #[command(name = "scale")]
    Scale { app_name: String, number: String },

    /// Reset counters for process
    #[command(name = "reset")]
    Reset { target: String },

    /// Describe all parameters of a process
    #[command(
        name = "describe",
        visible_aliases = ["desc", "info", "show"]
    )]
    Describe { target: String },

    /// List all environment variables of a process id
    #[command(name = "env")]
    Env { id: String },

    /// List all processes
    #[command(
        name = "list",
        visible_aliases = ["ls", "l", "ps", "status"]
    )]
    List,

    /// List all processes in JSON format
    #[command(name = "jlist")]
    Jlist,

    /// Print JSON in a prettified format
    #[command(name = "prettylist")]
    Prettylist,

    /// Stream logs file. Default stream all logs
    #[command(name = "logs", visible_alias = "log")]
    Logs {
        #[arg(default_value = "all")]
        target: String,
        #[arg(long = "lines", default_value_t = 15)]
        lines: usize,
        #[arg(long = "raw")]
        raw: bool,
        #[arg(long = "json")]
        json: bool,
        #[arg(long = "format")]
        format: bool,
        #[arg(long = "err")]
        err: bool,
        #[arg(long = "out")]
        out: bool,
        #[arg(long = "timestamp")]
        timestamp: Option<Option<String>>,
        #[arg(long = "nostream")]
        nostream: bool,
        #[arg(long = "highlight")]
        highlight: Option<String>,
    },

    /// Flush logs
    #[command(name = "flush")]
    Flush { api: Option<String> },

    /// Reload all logs
    #[command(name = "reloadLogs")]
    ReloadLogs,

    /// Send stdin to <pm_id>
    #[command(name = "send")]
    Send { pm_id: u32, line: Vec<String> },

    /// Attach stdin/stdout to application identified by <pm_id>
    #[command(name = "attach")]
    Attach { pm_id: u32 },

    /// Send a system signal to the target process
    #[command(name = "sendSignal")]
    SendSignal { signal: String, target: String },

    /// Trigger process action
    #[command(name = "trigger")]
    Trigger {
        target: String,
        action_name: String,
        params: Option<String>,
    },

    /// Return pid of [app_name] or all
    #[command(name = "pid")]
    Pid { app_name: Option<String> },

    /// Get process id by name
    #[command(name = "id")]
    Id { name: String },

    /// Inspect a process
    #[command(name = "inspect")]
    Inspect { name: String },

    /// Ping pmr daemon - if not up it will launch it
    #[command(name = "ping")]
    Ping,

    /// Update in-memory PMR with local PMR
    #[command(name = "update", visible_aliases = ["updatePM2", "deepUpdate"])]
    Update,

    /// Kill daemon
    #[command(name = "kill")]
    Kill,

    /// Dump all processes for resurrecting them later
    #[command(name = "save", visible_alias = "dump")]
    Save,

    /// Create empty dump file
    #[command(name = "cleardump")]
    Cleardump,

    /// Resurrect previously dumped processes
    #[command(name = "resurrect")]
    Resurrect,

    /// Enable the pmr startup hook
    #[command(name = "startup")]
    Startup { platform: Option<String> },

    /// Disable the pmr startup hook
    #[command(name = "unstartup")]
    Unstartup { platform: Option<String> },

    /// Copy default logrotate configuration
    #[command(name = "logrotate")]
    Logrotate,

    /// Generate a process conf file (mode = null or simple)
    #[command(name = "ecosystem", visible_alias = "init")]
    Ecosystem { mode: Option<String> },

    /// Serve a directory over http via port
    #[command(name = "serve", visible_alias = "expose")]
    Serve {
        path: Option<String>,
        port: Option<u16>,
        #[arg(long = "port")]
        port_flag: Option<u16>,
        #[arg(long = "spa")]
        spa: bool,
        #[arg(long = "basic-auth-username")]
        basic_auth_username: Option<String>,
        #[arg(long = "basic-auth-password")]
        basic_auth_password: Option<String>,
    },

    /// Launch health & status web API on :9615
    #[command(name = "web")]
    Web { port: Option<u16> },

    /// Launch terminal dashboard with monitoring and logs
    #[command(name = "monit", visible_aliases = ["dashboard", "dash"])]
    Monit,

    /// Launch legacy termcaps bar monitoring
    #[command(name = "imonit")]
    Imonit,

    /// Monitor target process
    #[command(name = "monitor")]
    Monitor { name: String },

    /// Unmonitor target process
    #[command(name = "unmonitor")]
    Unmonitor { name: String },

    /// Sets the specified config <key> <value>
    #[command(name = "set")]
    Set {
        key: Option<String>,
        value: Option<String>,
    },

    /// Multiset e.g. "key1 val1 key2 val2"
    #[command(name = "multiset")]
    Multiset { value: String },

    /// Get value for <key>
    #[command(name = "get")]
    Get { key: Option<String> },

    /// Get / set module config values
    #[command(name = "conf", visible_alias = "config")]
    Conf {
        key: Option<String>,
        value: Option<String>,
    },

    /// Clears the specified config <key>
    #[command(name = "unset")]
    Unset { key: String },

    /// Give a full pmr report
    #[command(name = "report")]
    Report,

    /// List system infos in JSON or tree
    #[command(name = "sysinfos", visible_aliases = ["slist", "sysmonit"])]
    Sysinfos {
        #[arg(short = 't', long = "tree")]
        tree: bool,
    },

    /// Install or update a module and run it forever
    #[command(
        name = "install",
        visible_aliases = ["module:install", "module:update"]
    )]
    Install { module: String },

    /// Stop and uninstall a module
    #[command(name = "uninstall", visible_alias = "module:uninstall")]
    Uninstall { module: String },

    /// Generate a sample module in current folder
    #[command(name = "module:generate", visible_alias = "create")]
    ModuleGenerate { app_name: Option<String> },

    /// Deploy your ecosystem configuration via SSH
    #[command(name = "deploy")]
    Deploy { args: Vec<String> },

    /// Updates repository for a given app
    #[command(name = "pull")]
    Pull {
        name: String,
        commit_id: Option<String>,
    },

    /// Updates repository to the next commit for a given app
    #[command(name = "forward")]
    Forward { name: String },

    /// Downgrades repository to the previous commit for a given app
    #[command(name = "backward")]
    Backward { name: String },

    /// Sample PMR heap memory
    #[command(name = "profile:mem")]
    ProfileMem { time: Option<u64> },

    /// Profile PMR cpu
    #[command(name = "profile:cpu")]
    ProfileCpu { time: Option<u64> },

    /// Display pmr usage examples
    #[command(name = "examples")]
    Examples,

    /// Output shell completion script
    #[command(name = "completion")]
    Completion,

    /// Internal daemon entrypoint
    #[command(name = "__daemon", hide = true)]
    InternalDaemon,

    /// Internal static server entrypoint
    #[command(name = "__serve", hide = true)]
    InternalServe,

    /// Internal web API entrypoint
    #[command(name = "__web", hide = true)]
    InternalWeb,
}

fn build_start_options(cli: &Cli, extra_script_args: &[String]) -> StartOptions {
    let instances = cli.instances.as_deref().map(|s| {
        if s.eq_ignore_ascii_case("max") {
            0
        } else {
            s.parse::<isize>().unwrap_or(1)
        }
    });

    let node_args = cli
        .node_args
        .as_deref()
        .map(shell_words_split)
        .unwrap_or_default();

    let exec_mode = if cli.execute_command {
        Some("fork_mode".to_string())
    } else {
        None
    };

    StartOptions {
        name: cli.name.clone(),
        namespace: cli.namespace.clone(),
        cwd: cli.cwd.clone(),
        args: extra_script_args.to_vec(),
        interpreter: cli.interpreter.clone(),
        node_args,
        instances,
        exec_mode,
        output: cli.output.clone(),
        error: cli.error.clone(),
        log: cli.log.clone(),
        pid: cli.pid.clone(),
        merge_logs: cli.merge_logs,
        disable_logs: cli.disable_logs,
        log_type: cli.log_type.clone(),
        log_date_format: cli.log_date_format.clone(),
        time: cli.time,
        env_name: cli.env_name.clone(),
        update_env: cli.update_env,
        force: cli.force,
        max_memory_restart: cli
            .max_memory_restart
            .as_deref()
            .and_then(parse_byte_size_str),
        restart_delay: cli
            .restart_delay
            .as_deref()
            .and_then(parse_duration_ms_str),
        exp_backoff_restart_delay: cli
            .exp_backoff_restart_delay
            .as_deref()
            .and_then(parse_duration_ms_str),
        max_restarts: cli.max_restarts,
        min_uptime: cli.min_uptime.as_deref().and_then(parse_duration_ms_str),
        cron_restart: cli.cron.clone(),
        watch: cli.watch,
        watch_paths: Vec::new(),
        ignore_watch: cli.ignore_watch.clone(),
        watch_delay: cli
            .watch_delay
            .as_deref()
            .and_then(parse_duration_ms_str),
        kill_timeout: cli.kill_timeout,
        listen_timeout: cli.listen_timeout,
        wait_ready: cli.wait_ready,
        shutdown_with_message: cli.shutdown_with_message,
        no_treekill: cli.no_treekill,
        no_autorestart: cli.no_autorestart,
        no_autostart: cli.no_autostart,
        no_vizion: cli.no_vizion,
        no_pmx: cli.no_pmx,
        stop_exit_codes: cli.stop_exit_codes.clone(),
        uid: cli.uid,
        gid: cli.gid,
        user: cli.user.clone(),
        filter_env: cli.filter_env.clone(),
        only: cli.only.clone(),
        extra_env: Default::default(),
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.version {
        println!("{}", PMR_VERSION);
        std::process::exit(SUCCESS_EXIT);
    }

    let Some(ref cmd) = cli.command else {
        let _ = <Cli as clap::CommandFactory>::command().print_help();
        println!();
        std::process::exit(ERROR_EXIT);
    };

    let paths = PmrPaths::default();

    // Handle internal & daemon-free commands first
    match cmd {
        Commands::InternalDaemon => {
            if let Err(e) = run_daemon(paths).await {
                eprintln!("Daemon error: {}", e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::InternalServe => {
            if let Err(e) = run_static_server().await {
                eprintln!("Serve error: {}", e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::InternalWeb => {
            if let Err(e) = run_web_server(paths).await {
                eprintln!("Web error: {}", e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::Startup { platform } => {
            if let Err(e) = cmd_startup(
                &paths,
                platform.as_deref(),
                cli.user.as_deref(),
                cli.hp.as_deref(),
                cli.wait_ip,
                cli.service_name.as_deref(),
            ) {
                eprintln!("{}{}", PREFIX_MSG_ERR, e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::Unstartup { platform } => {
            if let Err(e) = cmd_unstartup(
                &paths,
                platform.as_deref(),
                cli.user.as_deref(),
                cli.hp.as_deref(),
                cli.service_name.as_deref(),
            ) {
                eprintln!("{}{}", PREFIX_MSG_ERR, e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::Logrotate => {
            if let Err(e) = cmd_logrotate(&paths, cli.user.as_deref()) {
                let _ = e;
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::Ecosystem { mode } => {
            let content = if mode.as_deref() == Some("simple") {
                ECOSYSTEM_SIMPLE_TPL
            } else {
                ECOSYSTEM_TPL
            };
            let dest = PathBuf::from(APP_CONF_DEFAULT_FILE);
            if let Err(e) = fs::write(&dest, content) {
                eprintln!("{}{}", PREFIX_MSG_ERR, e);
                std::process::exit(ERROR_EXIT);
            }
            println!("{}File {} generated", PREFIX_MSG, dest.display());
            return;
        }
        Commands::Examples => {
            print_examples();
            return;
        }
        Commands::Completion => {
            println!("# PMR shell completion\ncomplete -W \"start stop restart reload delete ls jlist describe logs monit save resurrect kill startup serve web\" pmr");
            return;
        }
        Commands::Deploy { args } => {
            if let Err(e) = cmd_deploy(args) {
                eprintln!("{}{}", PREFIX_MSG_ERR, e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::ModuleGenerate { app_name } => {
            if let Err(e) = cmd_generate_module_sample(app_name.as_deref()) {
                eprintln!("{}{}", PREFIX_MSG_ERR, e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::Cleardump => {
            if let Err(e) = cmd_cleardump(&paths) {
                eprintln!("{}{}", PREFIX_MSG_ERR, e);
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        _ => {}
    }

    let client = PmrClient::new(Some(&paths.pm2_home));
    let use_color = !cli.no_color && env::var("NO_COLOR").is_err();

    if matches!(cmd, Commands::Kill) {
        if client.ping_daemon().await {
            println!("{}Stopping PMR daemon...", PREFIX_MSG);
            let _ = client.kill_daemon().await;
            println!("{}[PMR] All processes have been stopped and deleted", PREFIX_MSG);
            println!("{}[PMR] PMR stopped", PREFIX_MSG);
        } else {
            println!("{}PMR daemon is already stopped", PREFIX_MSG_WARNING);
        }
        return;
    }

    if let Err(e) = client.ensure_daemon(cli.no_daemon).await {
        eprintln!("{}{}", PREFIX_MSG_ERR, e);
        std::process::exit(ERROR_EXIT);
    }

    let res = execute_command(&cli, cmd, &client, use_color).await;
    if let Err(e) = res {
        if !cli.silent {
            eprintln!("{}{}", PREFIX_MSG_ERR, e);
        }
        std::process::exit(ERROR_EXIT);
    }
}

async fn execute_command(
    cli: &Cli,
    cmd: &Commands,
    client: &PmrClient,
    use_color: bool,
) -> Result<()> {
    let show_list = |procs: &[pmr::process::ProcessDescription]| {
        if !cli.silent {
            println!(
                "{}",
                render_process_list(procs, cli.mini_list, cli.sort.as_deref(), use_color)
            );
        }
    };

    match cmd {
        Commands::Start {
            targets,
            script_args,
        } => {
            let opts = build_start_options(cli, script_args);
            let list_targets = if targets.is_empty() {
                vec![APP_CONF_DEFAULT_FILE.to_string()]
            } else {
                targets.clone()
            };
            let mut procs = Vec::new();
            for t in &list_targets {
                procs = client.start_target(t, &opts, None).await?;
            }
            show_list(&procs);
            if cli.attach {
                run_logs(
                    client,
                    LogOptions {
                        target: "all".to_string(),
                        lines: 15,
                        raw: false,
                        json: false,
                        format: false,
                        err_only: false,
                        out_only: false,
                        timestamp: None,
                        nostream: false,
                        highlight: None,
                    },
                )
                .await?;
            }
        }
        Commands::StartOrRestart {
            targets,
            script_args,
        } => {
            let opts = build_start_options(cli, script_args);
            let list_targets = if targets.is_empty() {
                vec![APP_CONF_DEFAULT_FILE.to_string()]
            } else {
                targets.clone()
            };
            let mut procs = Vec::new();
            for t in &list_targets {
                procs = client.start_target(t, &opts, Some(false)).await?;
            }
            show_list(&procs);
        }
        Commands::StartOrReload {
            targets,
            script_args,
        } => {
            let opts = build_start_options(cli, script_args);
            let list_targets = if targets.is_empty() {
                vec![APP_CONF_DEFAULT_FILE.to_string()]
            } else {
                targets.clone()
            };
            let mut procs = Vec::new();
            for t in &list_targets {
                procs = client.start_target(t, &opts, Some(true)).await?;
            }
            show_list(&procs);
        }
        Commands::Stop { targets } => {
            if targets.is_empty() {
                return Err(anyhow!("stop requires at least one target (id, name, namespace, or all)"));
            }
            let mut procs = Vec::new();
            for t in targets {
                if cli.watch {
                    let _ = client
                        .request(&RpcRequest::StopWatch { target: t.clone() })
                        .await;
                }
                procs = client.stop(t).await?;
            }
            show_list(&procs);
        }
        Commands::Restart {
            targets,
            script_args,
        } => {
            if targets.is_empty() {
                return Err(anyhow!("restart requires at least one target"));
            }
            let opts = build_start_options(cli, script_args);
            let mut procs = Vec::new();
            for t in targets {
                if cli.watch {
                    let _ = client
                        .request(&RpcRequest::ToggleWatch { target: t.clone() })
                        .await;
                }
                procs = client.restart(t, &opts).await?;
            }
            show_list(&procs);
        }
        Commands::Reload { targets } => {
            if targets.is_empty() {
                return Err(anyhow!("reload requires at least one target"));
            }
            let opts = build_start_options(cli, &[]);
            let mut procs = Vec::new();
            for t in targets {
                procs = client.reload(t, &opts, false).await?;
            }
            show_list(&procs);
        }
        Commands::Delete { targets } => {
            if targets.is_empty() {
                return Err(anyhow!("delete requires at least one target"));
            }
            let mut procs = Vec::new();
            for t in targets {
                procs = client.delete(t).await?;
            }
            show_list(&procs);
        }
        Commands::Scale { app_name, number } => {
            let procs = client.scale(app_name, number).await?;
            show_list(&procs);
        }
        Commands::Reset { target } => {
            println!("{}Resetting meta for process {}", PREFIX_MSG, target);
            let procs = client.reset(target).await?;
            show_list(&procs);
        }
        Commands::Describe { target } => {
            let list = client.list().await?;
            let matched: Vec<_> = list
                .iter()
                .filter(|p| p.pm_id.to_string() == *target || p.name == *target)
                .collect();
            if matched.is_empty() {
                eprintln!("{}{} doesn't exist", PREFIX_MSG_WARNING, target);
                std::process::exit(ERROR_EXIT);
            }
            for proc in matched {
                print!("{}", render_describe(proc, use_color));
            }
        }
        Commands::Env { id } => {
            let list = client.list().await?;
            let matched: Vec<_> = list
                .iter()
                .filter(|p| p.pm_id.to_string() == *id || p.name == *id)
                .collect();
            if matched.is_empty() {
                return Err(anyhow!("Process with id {} not found", id));
            }
            for proc in matched {
                for (k, v) in &proc.pm2_env.env {
                    if use_color {
                        println!("{}: \x1b[32m{}\x1b[0m", k, v);
                    } else {
                        println!("{}: {}", k, v);
                    }
                }
            }
        }
        Commands::List => {
            let procs = client.list().await?;
            show_list(&procs);
        }
        Commands::Jlist => {
            let procs = client.list().await?;
            print!("{}", serde_json::to_string(&procs)?);
        }
        Commands::Prettylist => {
            let procs = client.list().await?;
            println!("{}", serde_json::to_string_pretty(&procs)?);
        }
        Commands::Logs {
            target,
            lines,
            raw,
            json,
            format,
            err,
            out,
            timestamp,
            nostream,
            highlight,
        } => {
            let ts = timestamp.as_ref().map(|opt| {
                opt.clone()
                    .unwrap_or_else(|| "%Y-%m-%d-%H:%M:%S".to_string())
            });
            run_logs(
                client,
                LogOptions {
                    target: target.clone(),
                    lines: *lines,
                    raw: *raw,
                    json: *json,
                    format: *format,
                    err_only: *err,
                    out_only: *out,
                    timestamp: ts,
                    nostream: *nostream,
                    highlight: highlight.clone(),
                },
            )
            .await?;
        }
        Commands::Flush { api } => {
            println!("{}Flushing logs...", PREFIX_MSG);
            let _ = client
                .request(&RpcRequest::FlushLogs {
                    target: api.clone(),
                })
                .await?;
            println!("{}Logs flushed", PREFIX_MSG);
        }
        Commands::ReloadLogs => {
            println!("Reloading all logs...");
            let _ = client.request(&RpcRequest::ReloadLogs).await?;
            println!("All logs reloaded");
        }
        Commands::Send { pm_id, line } => {
            let text = line.join(" ");
            let resp = client
                .request(&RpcRequest::SendLineToStdin {
                    pm_id: *pm_id,
                    line: text,
                })
                .await?;
            if !resp.ok {
                return Err(anyhow!("{}", resp.error.unwrap_or_default()));
            }
            println!("{}Line sent to stdin of process {}", PREFIX_MSG, pm_id);
        }
        Commands::Attach { pm_id } => {
            println!(
                "{}Attaching to process {} (Ctrl-C to exit)...",
                PREFIX_MSG, pm_id
            );
            let client_in = client.clone();
            let target_id = *pm_id;
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = client_in
                        .request(&RpcRequest::SendLineToStdin {
                            pm_id: target_id,
                            line,
                        })
                        .await;
                }
            });
            run_logs(
                client,
                LogOptions {
                    target: pm_id.to_string(),
                    lines: 0,
                    raw: false,
                    json: false,
                    format: false,
                    err_only: false,
                    out_only: false,
                    timestamp: None,
                    nostream: false,
                    highlight: None,
                },
            )
            .await?;
        }
        Commands::SendSignal { signal, target } => {
            let req = if let Ok(process_id) = target.parse::<u32>() {
                RpcRequest::SendSignalToProcessId {
                    process_id,
                    signal: signal.clone(),
                }
            } else {
                RpcRequest::SendSignalToProcessName {
                    process_name: target.clone(),
                    signal: signal.clone(),
                }
            };
            let procs = client.request(&req).await?.into_processes()?;
            show_list(&procs);
        }
        Commands::Trigger {
            target,
            action_name,
            params,
        } => {
            let id = target.parse::<u32>().ok();
            let name = if id.is_none() {
                Some(target.clone())
            } else {
                None
            };
            let opts_val = params.as_ref().map(|p| {
                serde_json::from_str(p).unwrap_or_else(|_| serde_json::Value::String(p.clone()))
            });
            let resp = client
                .request(&RpcRequest::MsgProcess {
                    id,
                    name,
                    msg: action_name.clone(),
                    opts: opts_val,
                })
                .await?;
            if !resp.ok {
                return Err(anyhow!("{}", resp.error.unwrap_or_default()));
            }
            println!(
                "{}Triggered action '{}' on {}",
                PREFIX_MSG, action_name, target
            );
        }
        Commands::Pid { app_name } => {
            let list = client.list().await?;
            for p in list {
                if app_name.is_none()
                    || app_name.as_deref() == Some(&p.name)
                    || app_name.as_deref() == Some(&p.pm_id.to_string())
                {
                    if p.pid > 0 {
                        println!("{}", p.pid);
                    }
                }
            }
        }
        Commands::Id { name } => {
            let list = client.list().await?;
            let ids: Vec<u32> = list
                .iter()
                .filter(|p| p.name == *name)
                .map(|p| p.pm_id)
                .collect();
            println!("{:?}", ids);
        }
        Commands::Inspect { name } => {
            let _ = client
                .request(&RpcRequest::MsgProcess {
                    id: name.parse::<u32>().ok(),
                    name: Some(name.clone()),
                    msg: "internal:inspect".to_string(),
                    opts: None,
                })
                .await?;
            println!("{}Triggered inspect on {}", PREFIX_MSG, name);
        }
        Commands::Ping => {
            let resp = client.request(&RpcRequest::Ping).await?;
            println!("{}pong ({})", PREFIX_MSG, resp.data);
        }
        Commands::Update => {
            println!("{}Be sure to have the latest version by doing `cargo install --path ~/pmr`", PREFIX_MSG);
            let _ = client.request(&RpcRequest::NotifyKillPM2).await;
            cmd_dump(client, true).await?;
            client.kill_daemon().await?;
            client.ensure_daemon(cli.no_daemon).await?;
            let procs = cmd_resurrect(client).await?;
            println!("{}>>>>>>>>>> PMR updated", PREFIX_MSG);
            show_list(&procs);
        }
        Commands::Save => {
            cmd_dump(client, cli.force).await?;
        }
        Commands::Resurrect => {
            let procs = cmd_resurrect(client).await?;
            show_list(&procs);
        }
        Commands::Serve {
            path,
            port,
            port_flag,
            spa,
            basic_auth_username,
            basic_auth_password,
        } => {
            let opts = build_start_options(cli, &[]);
            let resolved_port = port_flag.or(*port);
            let procs = launch_serve_app(
                client,
                path.as_deref(),
                resolved_port,
                *spa,
                basic_auth_username.clone(),
                basic_auth_password.clone(),
                cli.name.clone(),
                &opts,
            )
            .await?;
            show_list(&procs);
        }
        Commands::Web { port } => {
            let opts = build_start_options(cli, &[]);
            let procs = launch_web_app(client, *port, &opts).await?;
            show_list(&procs);
        }
        Commands::Monit => {
            run_dashboard(client).await?;
        }
        Commands::Imonit => {
            run_imonit(client).await?;
        }
        Commands::Monitor { name } => {
            let ids = client.resolve_target_ids(name).await?;
            for id in ids {
                let _ = client
                    .request(&RpcRequest::Monitor {
                        pm_id: id,
                        state: true,
                    })
                    .await?;
            }
            let procs = client.list().await?;
            show_list(&procs);
        }
        Commands::Unmonitor { name } => {
            let ids = client.resolve_target_ids(name).await?;
            for id in ids {
                let _ = client
                    .request(&RpcRequest::Monitor {
                        pm_id: id,
                        state: false,
                    })
                    .await?;
            }
            let procs = client.list().await?;
            show_list(&procs);
        }
        Commands::Set { key, value } => match (key, value) {
            (Some(k), Some(v)) => {
                let all = ModuleConfiguration::set_sync(&client.paths, k, v)?;
                print_conf_values(&all, None);
            }
            (Some(k), None) => {
                if let Some(v) = ModuleConfiguration::get_sync(&client.paths, k) {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                }
            }
            _ => {
                let all = ModuleConfiguration::get_all_sync(&client.paths);
                print_conf_values(&all, None);
            }
        },
        Commands::Multiset { value } => {
            let all = ModuleConfiguration::multiset_sync(&client.paths, value)?;
            print_conf_values(&all, None);
        }
        Commands::Get { key } => {
            let k = key.as_deref().unwrap_or("all");
            if let Some(v) = ModuleConfiguration::get_sync(&client.paths, k) {
                if k == "all" {
                    print_conf_values(&v, None);
                } else if let Some(s) = v.as_str() {
                    println!("{}", s);
                } else {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                }
            } else {
                return Err(anyhow!("Unknown config key: {}", k));
            }
        }
        Commands::Conf { key, value } => match (key, value) {
            (Some(k), Some(v)) => {
                let all = ModuleConfiguration::set_sync(&client.paths, k, v)?;
                print_conf_values(&all, None);
            }
            (Some(k), None) => {
                let all = ModuleConfiguration::get_all_sync(&client.paths);
                print_conf_values(&all, Some(k));
            }
            _ => {
                let all = ModuleConfiguration::get_all_sync(&client.paths);
                print_conf_values(&all, None);
            }
        },
        Commands::Unset { key } => {
            let all = ModuleConfiguration::unset_sync(&client.paths, key)?;
            print_conf_values(&all, None);
        }
        Commands::Report => {
            print_report(client).await?;
        }
        Commands::Sysinfos { tree } => {
            let resp = client.request(&RpcRequest::GetSystemData).await?;
            if *tree {
                print!("{}", render_tree(&resp.data, "", true));
            } else {
                println!("{}", serde_json::to_string_pretty(&resp.data)?);
            }
        }
        Commands::Install { module } => {
            let opts = build_start_options(cli, &[]);
            let procs = cmd_install_module(client, module, &opts).await?;
            show_list(&procs);
        }
        Commands::Uninstall { module } => {
            let procs = cmd_uninstall_module(client, module).await?;
            show_list(&procs);
        }
        Commands::Pull { name, commit_id } => {
            let procs =
                cmd_git_version_action(client, "pull", name, commit_id.as_deref()).await?;
            show_list(&procs);
        }
        Commands::Forward { name } => {
            let procs = cmd_git_version_action(client, "forward", name, None).await?;
            show_list(&procs);
        }
        Commands::Backward { name } => {
            let procs = cmd_git_version_action(client, "backward", name, None).await?;
            show_list(&procs);
        }
        Commands::ProfileMem { time } => {
            let t = time.unwrap_or(5_000);
            let ts = chrono::Local::now().format("%d-%H-%M-%S");
            let file = env::current_dir()?.join(format!("{}.heapprofile", ts));
            let pwd = file.to_string_lossy().to_string();
            println!("{}Sampling memory for {}ms...", PREFIX_MSG, t);
            let _ = client
                .request(&RpcRequest::Profile {
                    kind: "mem".to_string(),
                    pwd: pwd.clone(),
                    timeout_ms: t,
                })
                .await?;
            println!("{}Memory profile written to {}", PREFIX_MSG, pwd);
        }
        Commands::ProfileCpu { time } => {
            let t = time.unwrap_or(5_000);
            let ts = chrono::Local::now().format("%d-%H-%M-%S");
            let file = env::current_dir()?.join(format!("{}.cpuprofile", ts));
            let pwd = file.to_string_lossy().to_string();
            println!("{}Profiling CPU for {}ms...", PREFIX_MSG, t);
            let _ = client
                .request(&RpcRequest::Profile {
                    kind: "cpu".to_string(),
                    pwd: pwd.clone(),
                    timeout_ms: t,
                })
                .await?;
            println!("{}CPU profile written to {}", PREFIX_MSG, pwd);
        }
        _ => {}
    }

    Ok(())
}

fn print_conf_values(val: &serde_json::Value, filter_module: Option<&str>) {
    let Some(obj) = val.as_object() else {
        return;
    };
    for (k, v) in obj {
        if let Some(fm) = filter_module {
            if fm != k {
                continue;
            }
        }
        if let Some(sub_obj) = v.as_object() {
            println!("\x1b[1mModule: \x1b[1;34m{}\x1b[0m", k);
            for (sk, sv) in sub_obj {
                let s = sv.as_str().map(|s| s.to_string()).unwrap_or_else(|| sv.to_string());
                println!("$ pmr set {}:{} {}", k, sk, s);
            }
        } else {
            let s = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
            println!("$ pmr set {} {}", k, s);
        }
    }
}

fn print_examples() {
    println!(
        r#"
  Start and add a process to the pmr process list:
    $ pmr start app.js --name app

  Start a Rust binary or shell script:
    $ pmr start ./target/release/server --name api -- --port 8080

  Stop a process:
    $ pmr stop app

  Restart a process:
    $ pmr restart app

  Clusterize an app across all CPU cores:
    $ pmr start app.js -i max

  Zero-downtime reload:
    $ pmr reload all

  Save and resurrect processes across reboots:
    $ pmr save
    $ pmr resurrect
    $ pmr startup
"#
    );
}
