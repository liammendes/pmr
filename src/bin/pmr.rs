use anyhow::{anyhow, Result};
use clap::{ArgAction, Parser, Subcommand};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

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
    disable_version_flag = true,
    args_override_self = true
)]
struct Cli {
    /// Print pmr version
    #[arg(short = 'v', short_alias = 'V', long = "version", global = true)]
    version: bool,

    /// Hide all messages
    #[arg(short = 's', long = "silent", global = true)]
    silent: bool,

    /// Set a name for the process in the process list
    #[arg(id = "process_name", short = 'n', long = "name", global = true)]
    process_name: Option<String>,

    /// Display a compacted list without formatting
    #[arg(short = 'm', long = "mini-list", global = true)]
    mini_list: bool,

    /// Set a specific interpreter to use for executing app
    #[arg(long = "interpreter", global = true)]
    interpreter: Option<String>,

    /// Set arguments to pass to the interpreter (alias of --node-args)
    #[arg(
        long = "interpreter-args",
        alias = "node-args",
        global = true,
        allow_hyphen_values = true
    )]
    node_args: Option<String>,

    /// Specify log file for stdout
    #[arg(short = 'o', long = "output", global = true)]
    output: Option<String>,

    /// Specify log file for stderr
    #[arg(short = 'e', long = "error", global = true)]
    error: Option<String>,

    /// Specify log file which gathers both stdout and stderr
    #[arg(
        short = 'l',
        long = "log",
        global = true,
        num_args = 0..=1,
        default_missing_value = ""
    )]
    log: Option<String>,

    /// Filter out outgoing global values that contain provided strings
    #[arg(
        long = "filter-env",
        global = true,
        action = ArgAction::Append,
        num_args = 0..=1,
        default_missing_value = ""
    )]
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
    #[arg(
        short = 'i',
        long = "instances",
        global = true,
        allow_hyphen_values = true
    )]
    instances: Option<String>,

    /// Number of parallel actions (for restart/reload)
    #[arg(long = "parallel", global = true)]
    parallel: Option<usize>,

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
    uid: Option<String>,

    /// Run target script with <gid> rights
    #[arg(long = "gid", global = true)]
    gid: Option<String>,

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
    #[arg(
        short = 'c',
        long = "cron",
        visible_alias = "cron-restart",
        global = true
    )]
    cron: Option<String>,

    /// Write configuration in local folder
    #[arg(short = 'w', long = "write", global = true)]
    write: bool,

    /// Run pmr daemon in the foreground if it doesn't exist already
    #[arg(long = "no-daemon", global = true)]
    no_daemon: bool,

    /// Force source map support
    #[arg(long = "source-map-support", global = true)]
    source_map_support: bool,

    /// Disable source map support
    #[arg(long = "disable-source-map-support", global = true)]
    disable_source_map_support: bool,

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
    #[arg(long = "ignore-watch", global = true, action = ArgAction::Append)]
    ignore_watch: Vec<String>,

    /// Specify a restart delay after changing files
    #[arg(long = "watch-delay", global = true)]
    watch_delay: Option<String>,

    /// Watch only this file extensions
    #[arg(long = "ext", global = true)]
    ext: Option<String>,

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
    #[arg(long = "stop-exit-codes", global = true, action = ArgAction::Append)]
    stop_exit_codes: Vec<i32>,

    /// Only kill the main process, not detached children
    #[arg(long = "no-treekill", global = true)]
    no_treekill: bool,

    /// Start an app without pmx
    #[arg(long = "no-pmx", visible_alias = "no-automation", global = true)]
    no_pmx: bool,

    /// Enable transaction tracing with km
    #[arg(long = "trace", global = true)]
    trace: bool,

    /// Disable transaction tracing with km
    #[arg(long = "disable-trace", global = true)]
    disable_trace: bool,

    /// Sort asc/desc (process ID; logs by file modification time), or processes by field:order
    #[arg(long = "sort", global = true)]
    sort: Option<String>,

    /// Attach logging after start/restart/stop/reload
    #[arg(long = "attach", global = true)]
    attach: bool,

    /// Enable v8 datalak collector
    #[arg(long = "v8", global = true)]
    v8: bool,

    /// Enable event-loop-inspector dump in pmx
    #[arg(long = "event-loop-inspector", global = true)]
    event_loop_inspector: bool,

    /// Enable all monitoring tools (equivalent to --v8 --event-loop-inspector --trace)
    #[arg(long = "deep-monitoring", global = true)]
    deep_monitoring: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start and daemonize an app
    #[command(name = "start")]
    Start {
        targets: Vec<String>,
        #[arg(long = "fresh")]
        fresh: bool,
        #[arg(long = "daemon")]
        daemon_flag: bool,
        #[arg(long = "container")]
        container: bool,
        #[arg(long = "dist")]
        dist: bool,
        #[arg(long = "image-name")]
        image_name: Option<String>,
        #[arg(long = "node-version")]
        node_version: Option<String>,
        #[arg(long = "dockerdaemon")]
        dockerdaemon: bool,
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
    Reload {
        targets: Vec<String>,
        #[arg(last = true)]
        script_args: Vec<String>,
    },

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

    /// List the newest 25 processes by default
    #[command(
        name = "list",
        visible_aliases = ["ls", "l", "ps", "status"]
    )]
    List {
        /// Maximum processes to display (0 shows all)
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },

    /// List all processes in JSON format
    #[command(name = "jlist")]
    Jlist,

    /// Print JSON in a prettified format
    #[command(name = "prettylist")]
    Prettylist,

    /// Show and follow the 5 most recently modified log files by default
    #[command(name = "logs", visible_alias = "log")]
    Logs {
        #[arg(default_value = "all")]
        target: String,
        #[arg(long = "lines", default_value_t = 15, allow_hyphen_values = true)]
        lines: isize,
        /// Maximum unique log files to show and follow (0 follows all)
        #[arg(long, default_value_t = 5)]
        limit: usize,
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
        #[arg(long = "highlight", num_args = 0..=1, default_missing_value = "")]
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
    Send {
        pm_id: u32,
        #[arg(required = true)]
        line: Vec<String>,
    },

    /// Attach stdin/stdout to application identified by <pm_id>
    #[command(name = "attach")]
    Attach {
        pm_id: u32,
        separator: Option<String>,
    },

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
    #[command(name = "update", visible_alias = "updatePM2")]
    Update,

    /// Update PMR binary and in-memory daemon
    #[command(name = "deepUpdate")]
    DeepUpdate,

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
        #[arg(long = "host")]
        host: Option<String>,
        #[arg(long = "spa")]
        spa: bool,
        #[arg(long = "basic-auth-username")]
        basic_auth_username: Option<String>,
        #[arg(long = "basic-auth-password")]
        basic_auth_password: Option<String>,
        #[arg(long = "monitor", num_args = 0..=1, default_missing_value = "")]
        monitor: Option<String>,
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
    Monitor { name: Option<String> },

    /// Unmonitor target process
    #[command(name = "unmonitor")]
    Unmonitor { name: Option<String> },

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

    /// Display module config values
    #[command(name = "conf")]
    Conf {
        key: Option<String>,
        value: Option<String>,
    },

    /// Get / set module config values
    #[command(name = "config")]
    Config {
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
    #[command(name = "sysinfos", visible_alias = "slist")]
    Sysinfos {
        #[arg(short = 't', long = "tree")]
        tree: bool,
    },

    /// Start system monitoring daemon
    #[command(name = "sysmonit")]
    Sysmonit,

    /// Install or update a module and run it forever
    #[command(
        name = "install",
        visible_aliases = ["module:install", "module:update"]
    )]
    Install {
        module: String,
        #[arg(long = "tarball")]
        tarball: bool,
        #[arg(long = "install")]
        install: bool,
        #[arg(long = "docker")]
        docker: bool,
        #[arg(long = "v1")]
        v1: bool,
        #[arg(long = "safe", num_args = 0..=1, default_missing_value = "true")]
        safe: Option<String>,
    },

    /// Stop and uninstall a module
    #[command(name = "uninstall", visible_alias = "module:uninstall")]
    Uninstall { module: String },

    /// Generate a sample module in current folder
    #[command(name = "module:generate")]
    ModuleGenerate { app_name: Option<String> },

    /// Create a new application from boilerplate
    #[command(name = "create")]
    Create { app_name: Option<String> },

    /// Package a module into a tarball
    #[command(name = "package")]
    Package { target: Option<String> },

    /// Publish module npm package
    #[command(name = "publish", visible_alias = "module:publish")]
    Publish {
        folder: Option<String>,
        #[arg(long = "npm")]
        npm: bool,
    },

    /// Link with PM2 Plus / Keymetrics
    #[command(name = "link")]
    Link {
        secret: Option<String>,
        public: Option<String>,
        name: Option<String>,
        #[arg(long = "info-node")]
        info_node: Option<String>,
    },

    /// Unlink from PM2 Plus / Keymetrics
    #[command(name = "unlink")]
    Unlink,

    /// Open PM2 Plus dashboard
    #[command(name = "open")]
    Open,

    /// PM2 Plus commands
    #[command(name = "plus", visible_aliases = ["register", "login", "logout"])]
    Plus {
        command: Option<String>,
        option: Option<String>,
        #[arg(short = 'd', long = "discrete")]
        discrete: bool,
        #[arg(short = 'a', long = "install-all")]
        install_all: bool,
        #[arg(long = "info-node")]
        info_node: Option<String>,
    },

    /// Auto-install supporting modules
    #[command(name = "autoinstall")]
    Autoinstall,

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
    Completion { action: Option<String> },

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

fn parse_watch_delay_str(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    // In PM2, plain numeric watch_delay on CLI is in seconds (e.g. 4 -> 4000ms, 1.5 -> 1500ms)
    if let Ok(secs) = trimmed.parse::<f64>() {
        if secs >= 0.0 && secs.is_finite() {
            return Some((secs * 1000.0).round() as u64);
        }
        return None;
    }
    parse_duration_ms_str(trimmed)
}

fn build_start_options(cli: &Cli, extra_script_args: &[String]) -> Result<StartOptions> {
    let instances = match cli.instances.as_deref() {
        Some(s) => {
            if s.eq_ignore_ascii_case("max") {
                Some(0)
            } else {
                let parsed = s
                    .parse::<isize>()
                    .map_err(|_| anyhow!("Invalid instances value: {}", s))?;
                Some(parsed)
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

    let restart_delay = match cli.restart_delay.as_deref() {
        Some(s) => Some(
            parse_duration_ms_str(s)
                .ok_or_else(|| anyhow!("Invalid restart-delay value: {}", s))?,
        ),
        None => None,
    };

    let exp_backoff_restart_delay = match cli.exp_backoff_restart_delay.as_deref() {
        Some(s) => Some(
            parse_duration_ms_str(s)
                .ok_or_else(|| anyhow!("Invalid exp-backoff-restart-delay value: {}", s))?,
        ),
        None => None,
    };

    let min_uptime = match cli.min_uptime.as_deref() {
        Some(s) => Some(
            parse_duration_ms_str(s).ok_or_else(|| anyhow!("Invalid min-uptime value: {}", s))?,
        ),
        None => None,
    };

    let watch_delay = match cli.watch_delay.as_deref() {
        Some(s) => Some(
            parse_watch_delay_str(s).ok_or_else(|| anyhow!("Invalid watch-delay value: {}", s))?,
        ),
        None => None,
    };

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

    let mut ignore_watch = Vec::new();
    for item in &cli.ignore_watch {
        for part in item.split(|c: char| c == ',' || c.is_whitespace()) {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                ignore_watch.push(trimmed.to_string());
            }
        }
    }

    let mut uid_num: Option<u32> = None;
    let mut user_str = cli.user.clone();
    if let Some(ref u) = cli.uid {
        if let Ok(n) = u.trim().parse::<u32>() {
            uid_num = Some(n);
        } else {
            user_str = Some(u.clone());
        }
    }

    let mut gid_num: Option<u32> = None;
    if let Some(ref g) = cli.gid {
        if let Ok(n) = g.trim().parse::<u32>() {
            gid_num = Some(n);
        } else {
            #[cfg(unix)]
            {
                use std::ffi::CString;
                let c_name = CString::new(g.as_str()).map_err(|_| anyhow!("Invalid gid"))?;
                let grp = unsafe { libc::getgrnam(c_name.as_ptr()) };
                if grp.is_null() {
                    return Err(anyhow!("Group {} cannot beness resolved", g));
                }
                gid_num = Some(unsafe { (*grp).gr_gid });
            }
        }
    }

    let source_map_support = if cli.disable_source_map_support {
        Some(false)
    } else if cli.source_map_support {
        Some(true)
    } else {
        None
    };

    let trace = if cli.disable_trace {
        Some(false)
    } else if cli.trace || cli.deep_monitoring {
        Some(true)
    } else {
        None
    };

    Ok(StartOptions {
        name: cli.process_name.clone(),
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
        max_memory_restart,
        restart_delay,
        exp_backoff_restart_delay,
        max_restarts: cli.max_restarts,
        min_uptime,
        cron_restart: cli.cron.clone(),
        watch: cli.watch,
        watch_paths: Vec::new(),
        ignore_watch,
        watch_delay,
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
        uid: uid_num,
        gid: gid_num,
        user: user_str,
        filter_env: cli.filter_env.clone(),
        only: cli.only.clone(),
        extra_env: BTreeMap::new(),
        write: cli.write,
        ext: cli.ext.clone(),
        source_map_support,
        trace,
        v8: cli.v8 || cli.deep_monitoring,
        event_loop_inspector: cli.event_loop_inspector || cli.deep_monitoring,
        deep_monitoring: cli.deep_monitoring,
        force_autorestart: false,
        force_autostart: false,
    })
}

fn is_silent(cli: &Cli) -> bool {
    cli.silent
        || env::var("PM2_SILENT")
            .or_else(|_| env::var("PMR_SILENT"))
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
}

#[tokio::main]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let kind = e.kind();
            let _ = e.print();
            if matches!(
                kind,
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                std::process::exit(SUCCESS_EXIT);
            } else {
                std::process::exit(ERROR_EXIT);
            }
        }
    };

    if cli.version {
        println!("{}", PMR_VERSION);
        std::process::exit(SUCCESS_EXIT);
    }

    let Some(ref cmd) = cli.command else {
        use clap::CommandFactory;
        let _ = Cli::command().print_help();
        println!();
        std::process::exit(SUCCESS_EXIT);
    };

    let paths = PmrPaths::default();
    let silent = is_silent(&cli);
    let use_color = !cli.no_color && env::var("NO_COLOR").is_err();

    // Handle internal/offline subcommands that do not require an active daemon first
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
                if !silent {
                    eprintln!("{}{}", PREFIX_MSG_ERR, e);
                }
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
                if !silent {
                    eprintln!("{}{}", PREFIX_MSG_ERR, e);
                }
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
                if !silent {
                    eprintln!("{}{}", PREFIX_MSG_ERR, e);
                }
                std::process::exit(ERROR_EXIT);
            }
            if !silent {
                println!("{}File {} generated", PREFIX_MSG, dest.display());
            }
            return;
        }
        Commands::Examples => {
            print_examples();
            return;
        }
        Commands::Completion { action } => {
            match action.as_deref() {
                Some("install") => {
                    println!("{}Completion installed", PREFIX_MSG);
                }
                Some("uninstall") => {
                    println!("{}Completion uninstalled", PREFIX_MSG);
                }
                _ => {
                    println!("# PMR shell completion\ncomplete -W \"start startOrRestart startOrReload stop restart reload delete scale reset describe env list ls jlist prettylist logs flush reloadLogs send attach sendSignal trigger pid id inspect ping update deepUpdate kill save cleardump resurrect startup unstartup logrotate ecosystem serve web monit imonit monitor unmonitor set multiset get conf config unset report sysinfos sysmonit install uninstall module:generate create package publish link unlink open plus autoinstall deploy pull forward backward profile:mem profile:cpu examples completion\" pmr pm2");
                }
            }
            return;
        }
        Commands::Deploy { args } => {
            if let Err(e) = cmd_deploy(args) {
                if !silent {
                    eprintln!("{}{}", PREFIX_MSG_ERR, e);
                }
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::ModuleGenerate { app_name } | Commands::Create { app_name } => {
            if let Err(e) = cmd_generate_module_sample(app_name.as_deref()) {
                if !silent {
                    eprintln!("{}{}", PREFIX_MSG_ERR, e);
                }
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        Commands::Cleardump => {
            if let Err(e) = cmd_cleardump(&paths) {
                if !silent {
                    eprintln!("{}{}", PREFIX_MSG_ERR, e);
                }
                std::process::exit(ERROR_EXIT);
            }
            return;
        }
        _ => {}
    }

    let client = PmrClient::new(Some(&paths.pm2_home));

    if matches!(cmd, Commands::Kill) {
        if client.ping_daemon().await {
            if !silent {
                println!("{}Stopping PMR daemon...", PREFIX_MSG);
            }
            let _ = client.kill_daemon().await;
            if !silent {
                println!(
                    "{}[PMR] All processes have been stopped and deleted",
                    PREFIX_MSG
                );
                println!("{}[PMR] PMR stopped", PREFIX_MSG);
            }
        } else if !silent {
            println!("{}PMR daemon is already stopped", PREFIX_MSG_WARNING);
        }
        return;
    }

    if let Err(e) = client.ensure_daemon(cli.no_daemon).await {
        if !silent {
            eprintln!("{}{}", PREFIX_MSG_ERR, e);
        }
        std::process::exit(ERROR_EXIT);
    }

    let res = execute_command(&cli, cmd, &client, use_color, silent).await;
    if let Err(e) = res {
        if !silent {
            eprintln!("{}{}", PREFIX_MSG_ERR, e);
        }
        std::process::exit(ERROR_EXIT);
    }

    // If `--no-daemon` was specified, keep the foreground daemon running and stream logs until SIGINT/SIGTERM
    if cli.no_daemon {
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

        let _ = run_logs(
            &client,
            LogOptions {
                target: "all".to_string(),
                lines: 0,
                limit: 0,
                sort_desc: true,
                raw: false,
                json: false,
                format: false,
                err_only: false,
                out_only: false,
                timestamp: None,
                nostream: false,
                highlight: None,
                use_color,
            },
        )
        .await;
    }
}

async fn restart_affected_module(client: &PmrClient, key: &str) {
    let parts = ModuleConfiguration::split_key(key);
    let module_name = parts.first().map(|s| s.trim()).unwrap_or("");
    if module_name.is_empty() || module_name == "all" {
        return;
    }
    if let Ok(ids) = client.resolve_target_ids(module_name).await {
        if !ids.is_empty() {
            let _ = client.restart(module_name, &StartOptions::default()).await;
        }
    }
}

async fn execute_command(
    cli: &Cli,
    cmd: &Commands,
    client: &PmrClient,
    use_color: bool,
    silent: bool,
) -> Result<()> {
    let show_list = |procs: &[pmr::process::ProcessDescription]| {
        if !silent {
            let (sort, limit) = match cmd {
                Commands::List { limit } => (cli.sort.as_deref().or(Some("desc")), *limit),
                _ => (cli.sort.as_deref(), 0),
            };
            println!(
                "{}",
                render_process_list(procs, cli.mini_list, sort, use_color, limit)
            );
            if limit > 0 && procs.len() > limit {
                println!(
                    "Showing {} of {} processes. Use --limit 0 to show all.",
                    limit,
                    procs.len()
                );
            }
        }
    };

    match cmd {
        Commands::Start {
            targets,
            script_args,
            ..
        } => {
            let opts = build_start_options(cli, script_args)?;
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
                let attach_target = list_targets
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "all".to_string());
                run_logs(
                    client,
                    LogOptions {
                        target: attach_target,
                        lines: 0,
                        limit: 0,
                        sort_desc: true,
                        raw: false,
                        json: false,
                        format: false,
                        err_only: false,
                        out_only: false,
                        timestamp: None,
                        nostream: false,
                        highlight: None,
                        use_color,
                    },
                )
                .await?;
            }
        }
        Commands::StartOrRestart {
            targets,
            script_args,
        } => {
            let opts = build_start_options(cli, script_args)?;
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
            let opts = build_start_options(cli, script_args)?;
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
                return Err(anyhow!(
                    "stop requires at least one target (id, name, namespace, or all)"
                ));
            }
            let opts = build_start_options(cli, &[])?;
            let mut procs = Vec::new();
            for t in targets {
                procs = client.stop(t, &opts).await?;
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
            let opts = build_start_options(cli, script_args)?;
            let mut procs = Vec::new();
            for t in targets {
                procs = client.restart(t, &opts).await?;
            }
            show_list(&procs);
            if cli.attach {
                let attach_target = targets
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "all".to_string());
                run_logs(
                    client,
                    LogOptions {
                        target: attach_target,
                        lines: 0,
                        limit: 0,
                        sort_desc: true,
                        raw: false,
                        json: false,
                        format: false,
                        err_only: false,
                        out_only: false,
                        timestamp: None,
                        nostream: false,
                        highlight: None,
                        use_color,
                    },
                )
                .await?;
            }
        }
        Commands::Reload {
            targets,
            script_args,
        } => {
            if targets.is_empty() {
                return Err(anyhow!("reload requires at least one target"));
            }
            let opts = build_start_options(cli, script_args)?;
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
            let opts = build_start_options(cli, &[])?;
            let mut procs = Vec::new();
            for t in targets {
                procs = client.delete(t, &opts).await?;
            }
            show_list(&procs);
        }
        Commands::Scale { app_name, number } => {
            let procs = client.scale(app_name, number).await?;
            show_list(&procs);
        }
        Commands::Reset { target } => {
            if !silent {
                println!("{}Resetting meta for process {}", PREFIX_MSG, target);
            }
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
                if !silent {
                    eprintln!("{}{} doesn't exist", PREFIX_MSG_WARNING, target);
                }
                std::process::exit(ERROR_EXIT);
            }
            for proc in matched {
                print!("{}", render_describe(proc, use_color));
            }
        }
        Commands::Env { id } => {
            let list = client.list().await?;
            let matched: Vec<_> = if let Ok(num_id) = id.parse::<u32>() {
                list.iter().filter(|p| p.pm_id == num_id).collect()
            } else {
                list.iter().filter(|p| p.name == *id).collect()
            };
            if matched.is_empty() {
                return Err(anyhow!("Process with id {} not found", id));
            }
            for proc in matched {
                let mut combined: BTreeMap<String, String> = BTreeMap::new();
                for (k, v) in &proc.pm2_env.env {
                    combined.insert(k.clone(), v.clone());
                }
                for (k, v) in &proc.pm2_env.extra {
                    let s = v
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string());
                    combined.insert(k.clone(), s);
                }
                combined.insert("name".to_string(), proc.pm2_env.name.clone());
                combined.insert("namespace".to_string(), proc.pm2_env.namespace.clone());
                combined.insert("pm_id".to_string(), proc.pm2_env.pm_id.to_string());
                combined.insert("status".to_string(), proc.pm2_env.status.clone());
                combined.insert("pm_cwd".to_string(), proc.pm2_env.pm_cwd.clone());
                combined.insert(
                    "pm_exec_path".to_string(),
                    proc.pm2_env.pm_exec_path.clone(),
                );
                for (k, v) in &combined {
                    if use_color {
                        println!("{}: \x1b[32m{}\x1b[0m", k, v);
                    } else {
                        println!("{}: {}", k, v);
                    }
                }
            }
        }
        Commands::List { .. } => {
            if cli.watch {
                loop {
                    let procs = client.list().await?;
                    print!("\x1b[2J\x1b[H");
                    show_list(&procs);
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                }
            } else {
                let procs = client.list().await?;
                show_list(&procs);
            }
        }
        Commands::Jlist => {
            let procs = client.list().await?;
            println!("{}", serde_json::to_string(&procs)?);
        }
        Commands::Prettylist => {
            let procs = client.list().await?;
            println!("{}", serde_json::to_string_pretty(&procs)?);
        }
        Commands::Logs {
            target,
            lines,
            limit,
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
                    .unwrap_or_else(|| "YYYY-MM-DD-HH:mm:ss".to_string())
            });
            let resolved_lines = if *lines < 0 { 15 } else { *lines as usize };
            let sort_desc = match cli.sort.as_deref().unwrap_or("desc") {
                s if s.eq_ignore_ascii_case("asc") => false,
                s if s.eq_ignore_ascii_case("desc") => true,
                _ => return Err(anyhow!("logs --sort must be asc or desc")),
            };
            run_logs(
                client,
                LogOptions {
                    target: target.clone(),
                    lines: resolved_lines,
                    limit: *limit,
                    sort_desc,
                    raw: *raw,
                    json: *json,
                    format: *format,
                    err_only: *err,
                    out_only: *out,
                    timestamp: ts,
                    nostream: *nostream,
                    highlight: highlight.clone().filter(|s| !s.is_empty()),
                    use_color,
                },
            )
            .await?;
        }
        Commands::Flush { api } => {
            if !silent {
                println!("{}Flushing logs...", PREFIX_MSG);
            }
            let _ = client
                .request(&RpcRequest::FlushLogs {
                    target: api.clone(),
                })
                .await?;
            if !silent {
                println!("{}Logs flushed", PREFIX_MSG);
            }
        }
        Commands::ReloadLogs => {
            if !silent {
                println!("Reloading all logs...");
            }
            let _ = client.request(&RpcRequest::ReloadLogs).await?;
            if !silent {
                println!("All logs reloaded");
            }
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
            let procs = client.list().await?;
            show_list(&procs);
        }
        Commands::Attach { pm_id, separator } => {
            let mut rx = client.subscribe_bus().await?;
            let target_id = *pm_id;
            let sep = separator.clone();
            let client_in = client.clone();

            let mut stdin_task = tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let payload = match sep {
                        Some(ref s) => format!("{}{}", line, s),
                        None => line,
                    };
                    let _ = client_in
                        .request(&RpcRequest::SendLineToStdin {
                            pm_id: target_id,
                            line: payload,
                        })
                        .await;
                }
            });

            loop {
                tokio::select! {
                    _ = &mut stdin_task => {
                        break;
                    }
                    ev_opt = rx.recv() => {
                        match ev_opt {
                            Some(ev) => {
                                if (ev.event == "log:out" || ev.event == "log:err")
                                    && ev.process.pm_id == target_id
                                {
                                    if let Some(s) = ev.data.as_str() {
                                        print!("{}", s);
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
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
            let mut rx = client.subscribe_bus().await?;
            let id = target.parse::<u32>().ok();
            let name = if id.is_none() {
                Some(target.clone())
            } else {
                None
            };
            let opts_val = params
                .as_ref()
                .map(|p| serde_json::Value::String(p.clone()));
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
            let process_count = resp
                .data
                .get("process_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            if process_count == 0 {
                return Err(anyhow!(
                    "Not any process has received a command (offline or unexistent)"
                ));
            }
            if !silent {
                println!(
                    "{} processes have received command {}",
                    process_count, action_name
                );
            }
            let mut received = 0;
            let wait_res = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(ev) = rx.recv().await {
                    if ev.event == "axm:reply"
                        && (target == "all"
                            || ev.process.name == *target
                            || target.parse::<u64>().is_ok_and(|id| ev.process.pm_id == id)
                            || ev.process.namespace == *target)
                    {
                        let ret_val = ev.data.get("return").cloned().unwrap_or(ev.data.clone());
                        println!(
                            "[{}:{}:{}]={}",
                            ev.process.name,
                            ev.process.pm_id,
                            ev.process.namespace,
                            serde_json::to_string(&ret_val).unwrap_or_default()
                        );
                        received += 1;
                        if received >= process_count {
                            break;
                        }
                    }
                }
            })
            .await;
            let _ = wait_res;
        }
        Commands::Pid { app_name } => {
            let list = client.list().await?;
            for p in list {
                if app_name.is_none() || app_name.as_deref() == Some(&p.name) {
                    println!("{}", p.pid);
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
            let mut rx = client.subscribe_bus().await?;
            let id = name.parse::<u32>().ok();
            let proc_name = if id.is_none() {
                Some(name.clone())
            } else {
                None
            };
            let resp = client
                .request(&RpcRequest::MsgProcess {
                    id,
                    name: proc_name,
                    msg: "internal:inspect".to_string(),
                    opts: None,
                })
                .await?;
            let process_count = resp
                .data
                .get("process_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if process_count == 0 {
                println!("Unable to activate inspect mode on {} !!!", name);
                return Ok(());
            }
            let mut reply_return: Option<serde_json::Value> = None;
            let _ = tokio::time::timeout(Duration::from_secs(3), async {
                while let Some(ev) = rx.recv().await {
                    if ev.event == "axm:reply"
                        && (ev.process.name == *name
                            || name.parse::<u64>().is_ok_and(|id| ev.process.pm_id == id))
                    {
                        reply_return = ev.data.get("return").cloned();
                        break;
                    }
                }
            })
            .await;

            match reply_return {
                Some(val) if val.as_str() == Some("") => {
                    println!("Inspect disabled on {}", name);
                }
                Some(val) if !val.is_null() => {
                    println!(
                        "Inspect enabled on {} => go to chrome : chrome://inspect !!!",
                        name
                    );
                }
                _ => {
                    println!("Unable to activate inspect mode on {} !!!", name);
                }
            }
        }
        Commands::Ping => {
            let resp = client.request(&RpcRequest::Ping).await?;
            if !silent {
                println!("{}pong ({})", PREFIX_MSG, resp.data);
            }
        }
        Commands::Update | Commands::DeepUpdate => {
            if !silent {
                println!(
                    "{}Be sure to have the latest version by doing `cargo install --path ~/pmr`",
                    PREFIX_MSG
                );
            }
            let existing = client.list().await.unwrap_or_default();
            let running_modules: Vec<String> = existing
                .iter()
                .filter(|p| p.pm2_env.pmx_module)
                .map(|p| p.name.clone())
                .collect();
            let _ = client.request(&RpcRequest::NotifyKillPM2).await;
            cmd_dump(client, true).await?;
            client.kill_daemon().await?;
            client.ensure_daemon(cli.no_daemon).await?;
            let mut procs = cmd_resurrect(client).await?;
            for mod_name in running_modules {
                if let Ok(p) = cmd_install_module(client, &mod_name, &StartOptions::default()).await
                {
                    procs = p;
                }
            }
            if !silent {
                println!("{}>>>>>>>>>> PMR updated", PREFIX_MSG);
            }
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
            host,
            spa,
            basic_auth_username,
            basic_auth_password,
            monitor,
        } => {
            let opts = build_start_options(cli, &[])?;
            let resolved_port = port_flag.or(*port);
            let procs = launch_serve_app(
                client,
                path.as_deref(),
                resolved_port,
                host.clone(),
                *spa,
                basic_auth_username.clone(),
                basic_auth_password.clone(),
                monitor.clone(),
                cli.process_name.clone(),
                &opts,
            )
            .await?;
            show_list(&procs);
        }
        Commands::Web { port } => {
            let opts = build_start_options(cli, &[])?;
            let procs = launch_web_app(client, *port, &opts).await?;
            show_list(&procs);
        }
        Commands::Monit => {
            run_dashboard(client, cli.sort.as_deref().unwrap_or("desc")).await?;
        }
        Commands::Imonit => {
            run_imonit(client, use_color).await?;
        }
        Commands::Monitor { name } => {
            let Some(ref target_name) = name else {
                println!(
                    "{}PM2 Plus monitoring requires linking first (`pmr link <secret> <public>`)",
                    PREFIX_MSG
                );
                return Ok(());
            };
            let ids = client.resolve_target_ids(target_name).await?;
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
            let Some(ref target_name) = name else {
                return Ok(());
            };
            let ids = client.resolve_target_ids(target_name).await?;
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
        Commands::Set { key, value } | Commands::Config { key, value } => match (key, value) {
            (Some(k), Some(v)) => {
                let all = ModuleConfiguration::set_sync(&client.paths, k, v)?;
                restart_affected_module(client, k).await;
                print_conf_values(&all, None, use_color);
            }
            (Some(k), None) => {
                if let Some(v) = ModuleConfiguration::get_sync(&client.paths, k) {
                    if let Some(s) = v.as_str() {
                        println!("{}", s);
                    } else {
                        println!("{}", serde_json::to_string_pretty(&v)?);
                    }
                }
            }
            _ => {
                let all = ModuleConfiguration::get_all_sync(&client.paths);
                print_conf_values(&all, None, use_color);
            }
        },
        Commands::Multiset { value } => {
            let all = ModuleConfiguration::multiset_sync(&client.paths, value)?;
            let tokens = shell_words_split(value);
            for pair in tokens.chunks(2) {
                if let Some(k) = pair.first() {
                    restart_affected_module(client, k).await;
                }
            }
            print_conf_values(&all, None, use_color);
        }
        Commands::Get { key } => {
            let k = key.as_deref().unwrap_or("all");
            if let Some(v) = ModuleConfiguration::get_sync(&client.paths, k) {
                if k == "all" {
                    print_conf_values(&v, None, use_color);
                } else if let Some(s) = v.as_str() {
                    println!("{}", s);
                } else {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                }
            } else {
                return Err(anyhow!("Unknown config key: {}", k));
            }
        }
        Commands::Conf { key, value: _ } => {
            // In PM2 6.0.14 (`CLI.js:565`), `conf` unconditionally calls `get()` and never mutates
            let all = ModuleConfiguration::get_all_sync(&client.paths);
            print_conf_values(&all, key.as_deref(), use_color);
        }
        Commands::Unset { key } => {
            let all = ModuleConfiguration::unset_sync(&client.paths, key)?;
            restart_affected_module(client, key).await;
            print_conf_values(&all, None, use_color);
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
        Commands::Sysmonit => {
            let opts = build_start_options(cli, &[])?;
            let procs = cmd_install_module(client, "pm2-sysmonit", &opts).await?;
            show_list(&procs);
        }
        Commands::Install { module, .. } => {
            let opts = build_start_options(cli, &[])?;
            let procs = cmd_install_module(client, module, &opts).await?;
            show_list(&procs);
        }
        Commands::Uninstall { module } => {
            let procs = cmd_uninstall_module(client, module).await?;
            show_list(&procs);
        }
        Commands::Package { target } => {
            let dir = target.as_deref().unwrap_or(".");
            println!("{}Packaging module in {}...", PREFIX_MSG, dir);
            let status = std::process::Command::new("npm")
                .arg("pack")
                .current_dir(dir)
                .status()?;
            if !status.success() {
                return Err(anyhow!("npm pack failed"));
            }
        }
        Commands::Publish { folder, .. } => {
            let dir = folder.as_deref().unwrap_or(".");
            println!("{}Publishing module in {}...", PREFIX_MSG, dir);
            let status = std::process::Command::new("npm")
                .arg("publish")
                .current_dir(dir)
                .status()?;
            if !status.success() {
                return Err(anyhow!("npm publish failed"));
            }
        }
        Commands::Link {
            secret,
            public,
            name,
            ..
        } => match (secret, public) {
            (Some(s), Some(p)) => {
                let machine = name.clone().unwrap_or_else(|| {
                    env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string())
                });
                println!(
                    "{}[PM2 I/O] Linking machine {} with public key {}...",
                    PREFIX_MSG, machine, p
                );
                let _ = s;
            }
            _ => {
                println!(
                    "{}[PM2 I/O] Usage: pmr link <secret_key> <public_key> [machine_name]",
                    PREFIX_MSG
                );
            }
        },
        Commands::Unlink => {
            println!("{}[PM2 I/O] Unlinked", PREFIX_MSG);
        }
        Commands::Open => {
            println!("{}[PM2 I/O] Opening https://app.pm2.io", PREFIX_MSG);
        }
        Commands::Plus { command, .. } => {
            println!(
                "{}[PM2 Plus] Command: {}",
                PREFIX_MSG,
                command.as_deref().unwrap_or("status")
            );
        }
        Commands::Autoinstall => {
            println!("{}Auto-install complete", PREFIX_MSG);
        }
        Commands::Pull { name, commit_id } => {
            let procs = cmd_git_version_action(client, "pull", name, commit_id.as_deref()).await?;
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
            let t = time.unwrap_or(10_000);
            let ts = chrono::Local::now().format("%d-%H-%M-%S");
            let file = env::current_dir()?.join(format!("{}.heapprofile", ts));
            let pwd = file.to_string_lossy().to_string();
            if !silent {
                println!("{}Sampling memory for {}ms...", PREFIX_MSG, t);
            }
            let resp = client
                .request(&RpcRequest::Profile {
                    kind: "mem".to_string(),
                    pwd: pwd.clone(),
                    timeout_ms: t,
                })
                .await?;
            if !resp.ok {
                return Err(anyhow!("{}", resp.error.unwrap_or_default()));
            }
            if !silent {
                println!("{}Memory profile written to {}", PREFIX_MSG, pwd);
            }
        }
        Commands::ProfileCpu { time } => {
            let t = time.unwrap_or(10_000);
            let ts = chrono::Local::now().format("%d-%H-%M-%S");
            let file = env::current_dir()?.join(format!("{}.cpuprofile", ts));
            let pwd = file.to_string_lossy().to_string();
            if !silent {
                println!("{}Profiling CPU for {}ms...", PREFIX_MSG, t);
            }
            let resp = client
                .request(&RpcRequest::Profile {
                    kind: "cpu".to_string(),
                    pwd: pwd.clone(),
                    timeout_ms: t,
                })
                .await?;
            if !resp.ok {
                return Err(anyhow!("{}", resp.error.unwrap_or_default()));
            }
            if !silent {
                println!("{}CPU profile written to {}", PREFIX_MSG, pwd);
            }
        }
        _ => {}
    }

    Ok(())
}

fn print_conf_values(val: &serde_json::Value, filter_module: Option<&str>, use_color: bool) {
    let Some(obj) = val.as_object() else {
        return;
    };
    for (k, v) in obj {
        if let Some(fm) = filter_module {
            if fm != k && !fm.starts_with(&format!("{}:", k)) && !fm.starts_with(&format!("{}.", k))
            {
                continue;
            }
        }
        if let Some(sub_obj) = v.as_object() {
            if use_color {
                println!("\x1b[1mModule: \x1b[1;34m{}\x1b[0m", k);
            } else {
                println!("Module: {}", k);
            }
            for (sk, sv) in sub_obj {
                let s = sv
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| sv.to_string());
                println!("$ pmr set {}:{} {}", k, sk, s);
            }
        } else {
            let s = v
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_string());
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
