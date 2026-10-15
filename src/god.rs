use anyhow::{anyhow, Result};
use chrono::{Local, Utc};
use cron::Schedule;
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{broadcast, mpsc, Mutex, Notify};
use uuid::Uuid;

use crate::config::{find_git_versioning, find_package_version};
use crate::constants::{
    CLUSTER_MODE_ID, ERRORED_STATUS, EXP_BACKOFF_RESET_TIMER_MS, LAUNCHING_STATUS, ONLINE_STATUS,
    STOPPED_STATUS, STOPPING_STATUS, WAITING_RESTART,
};
use crate::paths::PmrPaths;
use crate::process::{AxmAction, AxmMetric, Monit, Pm2Env, ProcessDescription};
use crate::protocol::{BusEvent, BusProcessRef};

pub struct ManagedProcess {
    pub pm2_env: Pm2Env,
    pub pid: u32,
    pub monit: Monit,
    pub stdin_tx: Option<mpsc::UnboundedSender<String>>,
    pub ipc_tx: Option<mpsc::UnboundedSender<String>>,
    pub stop_notify: Arc<Notify>,
    pub ready_notify: Arc<Notify>,
    pub log_reload_notify: Arc<Notify>,
    pub watcher: Option<RecommendedWatcher>,
    pub generation: u64,
}

pub struct God {
    pub clusters_db: BTreeMap<u32, ManagedProcess>,
    pub cluster_balancers: BTreeMap<u16, Arc<std::sync::Mutex<BTreeMap<u32, u16>>>>,
    pub next_id: u32,
    pub pm2_being_killed: bool,
    pub started_at: u64,
    pub paths: PmrPaths,
    pub bus_tx: broadcast::Sender<BusEvent>,
    pub sys: System,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl God {
    pub fn new(paths: PmrPaths, bus_tx: broadcast::Sender<BusEvent>) -> Arc<Mutex<Self>> {
        let mut sys = System::new_all();
        sys.refresh_all();
        Arc::new(Mutex::new(Self {
            clusters_db: BTreeMap::new(),
            cluster_balancers: BTreeMap::new(),
            next_id: 0,
            pm2_being_killed: false,
            started_at: now_ms(),
            paths,
            bus_tx,
            sys,
        }))
    }

    pub fn emit_bus_event(&self, event: &str, pm_id: Value, name: &str, namespace: &str, data: Value) {
        let _ = self.bus_tx.send(BusEvent {
            event: event.to_string(),
            at: now_ms(),
            process: BusProcessRef {
                pm_id,
                name: name.to_string(),
                namespace: namespace.to_string(),
                rev: None,
            },
            data,
        });
    }

    pub fn log_pm2(&self, msg: &str) {
        let ts = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let line = format!("{}: PM2 log: {}\n", ts, msg);
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.paths.pm2_log_file_path)
        {
            let _ = f.write_all(line.as_bytes());
        }
        self.emit_bus_event(
            "log:PM2",
            json!("PM2"),
            "PM2",
            "default",
            json!({ "data": line }),
        );
    }

    pub fn allocate_id(&mut self) -> u32 {
        if self.clusters_db.is_empty() {
            self.next_id = 0;
        }
        while self.clusters_db.contains_key(&self.next_id) {
            self.next_id += 1;
        }
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn allocate_instance_index(&self, app_name: &str, exclude_id: Option<u32>) -> u32 {
        let used: HashSet<u32> = self
            .clusters_db
            .values()
            .filter(|p| p.pm2_env.name == app_name && Some(p.pm2_env.pm_id) != exclude_id)
            .filter_map(|p| p.pm2_env.node_app_instance)
            .collect();
        let mut idx = 0;
        while used.contains(&idx) {
            idx += 1;
        }
        idx
    }

    pub fn refresh_monitor_data(&mut self) {
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        for proc in self.clusters_db.values_mut() {
            let target_pid = proc
                .pm2_env
                .axm_options
                .get("pid")
                .and_then(|v| v.as_u64())
                .map(|p| p as u32)
                .unwrap_or(proc.pid);

            if proc.pm2_env.status == ONLINE_STATUS && target_pid > 0 {
                if let Some(p) = self.sys.process(Pid::from_u32(target_pid)) {
                    proc.monit.memory = p.memory();
                    proc.monit.cpu = (p.cpu_usage() * 10.0).round() / 10.0;
                } else {
                    proc.monit.memory = 0;
                    proc.monit.cpu = 0.0;
                }
            } else {
                proc.monit.memory = 0;
                proc.monit.cpu = 0.0;
            }
        }
    }

    pub fn get_formatted_processes(&mut self) -> Vec<ProcessDescription> {
        self.refresh_monitor_data();
        self.clusters_db
            .values()
            .map(|p| ProcessDescription {
                pid: p.pid,
                name: p.pm2_env.name.clone(),
                pm2_env: p.pm2_env.clone(),
                pm_id: p.pm2_env.pm_id,
                monit: p.monit.clone(),
            })
            .collect()
    }

    pub fn get_formatted_process(&mut self, id: u32) -> Option<ProcessDescription> {
        self.refresh_monitor_data();
        self.clusters_db.get(&id).map(|p| ProcessDescription {
            pid: p.pid,
            name: p.pm2_env.name.clone(),
            pm2_env: p.pm2_env.clone(),
            pm_id: p.pm2_env.pm_id,
            monit: p.monit.clone(),
        })
    }
}

pub async fn prepare_and_launch(
    god: Arc<Mutex<God>>,
    template_env: Pm2Env,
    instances: usize,
) -> Result<Vec<ProcessDescription>> {
    let count = instances.max(1);
    let mut results = Vec::with_capacity(count);

    for _ in 0..count {
        let mut env = template_env.clone();
        env.unique_id = Uuid::new_v4().to_string();
        let (pm_id, autostart) = {
            let mut g = god.lock().await;
            let id = g.allocate_id();
            let inst_idx = g.allocate_instance_index(&env.name, Some(id));
            env.pm_id = id;
            env.node_app_instance = Some(inst_idx);
            env.env
                .insert(env.instance_var.clone(), inst_idx.to_string());
            env.env
                .insert("NODE_APP_INSTANCE".to_string(), inst_idx.to_string());

            if let Some(ref inc_var) = env.increment_var {
                if let Some(base_val) = template_env.env.get(inc_var) {
                    if let Ok(base_num) = base_val.parse::<i64>() {
                        env.env.insert(
                            inc_var.clone(),
                            (base_num + inst_idx as i64).to_string(),
                        );
                    }
                }
            }

            suffix_paths_for_instance(&mut env);
            let autostart = env.autostart && env.status != STOPPED_STATUS;
            if !autostart {
                env.status = STOPPED_STATUS.to_string();
            }

            let mp = ManagedProcess {
                pm2_env: env.clone(),
                pid: 0,
                monit: Monit::default(),
                stdin_tx: None,
                ipc_tx: None,
                stop_notify: Arc::new(Notify::new()),
                ready_notify: Arc::new(Notify::new()),
                log_reload_notify: Arc::new(Notify::new()),
                watcher: None,
                generation: 0,
            };
            g.clusters_db.insert(id, mp);
            if env.watch {
                setup_watcher_for_process(Arc::clone(&god), &mut g, id);
            }
            (id, autostart)
        };

        if autostart {
            start_process_id(Arc::clone(&god), pm_id).await?;
        }

        let g_desc = {
            let mut g = god.lock().await;
            g.get_formatted_process(pm_id)
                .ok_or_else(|| anyhow!("Process {} disappeared", pm_id))?
        };
        results.push(g_desc);
    }

    Ok(results)
}

fn suffix_paths_for_instance(env: &mut Pm2Env) {
    let id_suffix = format!("-{}.log", env.pm_id);
    if !env.merge_logs {
        if env.pm_out_log_path != "/dev/null" && !env.pm_out_log_path.ends_with(&id_suffix) {
            if let Some(stem) = env.pm_out_log_path.strip_suffix(".log") {
                env.pm_out_log_path = format!("{}-{}.log", stem, env.pm_id);
            }
        }
        if env.pm_err_log_path != "/dev/null" && !env.pm_err_log_path.ends_with(&id_suffix) {
            if let Some(stem) = env.pm_err_log_path.strip_suffix(".log") {
                env.pm_err_log_path = format!("{}-{}.log", stem, env.pm_id);
            } else if let Some(stem) = env.pm_err_log_path.strip_suffix(".err") {
                env.pm_err_log_path = format!("{}-{}.err", stem, env.pm_id);
            }
        }
        if let Some(ref mut log_p) = env.pm_log_path {
            if log_p != "/dev/null" && !log_p.ends_with(&id_suffix) {
                if let Some(stem) = log_p.strip_suffix(".log") {
                    *log_p = format!("{}-{}.log", stem, env.pm_id);
                }
            }
        }
    }
    let pid_suffix = format!("-{}.pid", env.pm_id);
    if !env.pm_pid_path.ends_with(&pid_suffix) {
        if let Some(stem) = env.pm_pid_path.strip_suffix(".pid") {
            env.pm_pid_path = format!("{}-{}.pid", stem, env.pm_id);
        }
    }
}

pub fn start_process_id(god: Arc<Mutex<God>>, id: u32) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ProcessDescription>> + Send>> {
    Box::pin(async move {
    let (
        mut env,
        paths,
        stop_notify,
        ready_notify,
        log_reload_notify,
        generation,
    ) = {
        let mut g = god.lock().await;
        let paths = g.paths.clone();
        let proc = g
            .clusters_db
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Process {} not found", id))?;

        if proc.pm2_env.status == ONLINE_STATUS && proc.pid > 0 {
            return Ok(ProcessDescription {
                pid: proc.pid,
                name: proc.pm2_env.name.clone(),
                pm2_env: proc.pm2_env.clone(),
                pm_id: proc.pm2_env.pm_id,
                monit: proc.monit.clone(),
            });
        }

        proc.generation += 1;
        let gen = proc.generation;
        proc.stop_notify = Arc::new(Notify::new());
        proc.ready_notify = Arc::new(Notify::new());
        proc.pm2_env.pm_uptime = now_ms();
        if proc.pm2_env.created_at.is_none() {
            proc.pm2_env.created_at = Some(proc.pm2_env.pm_uptime);
        }
        proc.pm2_env.status = if proc.pm2_env.wait_ready {
            LAUNCHING_STATUS.to_string()
        } else {
            ONLINE_STATUS.to_string()
        };

        // Refresh version & git metadata
        proc.pm2_env.version = find_package_version(
            Path::new(&proc.pm2_env.pm_exec_path),
            Path::new(&proc.pm2_env.pm_cwd),
        );
        if proc.pm2_env.vizion {
            proc.pm2_env.versioning = find_git_versioning(Path::new(&proc.pm2_env.pm_cwd));
        }

        (
            proc.pm2_env.clone(),
            paths,
            Arc::clone(&proc.stop_notify),
            Arc::clone(&proc.ready_notify),
            Arc::clone(&proc.log_reload_notify),
            gen,
        )
    };

    // Build command & arguments
    let (program, cmd_args) = if env.exec_interpreter == "none" || env.exec_interpreter.is_empty() {
        (env.pm_exec_path.clone(), env.args.clone())
    } else {
        let mut a = Vec::new();
        a.extend(env.node_args.clone());
        a.push(env.pm_exec_path.clone());
        a.extend(env.args.clone());
        (env.exec_interpreter.clone(), a)
    };

    let mut cmd = TokioCommand::new(&program);
    cmd.args(&cmd_args)
        .current_dir(&env.pm_cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Put child in its own process group on Unix so terminal Ctrl-C or treekill works cleanly
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    // Populate child environment
    for (k, v) in &env.env {
        cmd.env(k, v);
    }
    cmd.env("pm_id", env.pm_id.to_string());
    cmd.env("name", &env.name);
    cmd.env("namespace", &env.namespace);
    cmd.env("exec_mode", &env.exec_mode);
    cmd.env("pm_exec_path", &env.pm_exec_path);
    cmd.env("pm_cwd", &env.pm_cwd);
    cmd.env("pm_out_log_path", &env.pm_out_log_path);
    cmd.env("pm_err_log_path", &env.pm_err_log_path);
    cmd.env("pm_pid_path", &env.pm_pid_path);
    cmd.env("PM2_HOME", paths.pm2_home.to_string_lossy().to_string());
    cmd.env("PMR_HOME", paths.pm2_home.to_string_lossy().to_string());
    cmd.env("PM2_USAGE", "CLI");
    cmd.env("PM2_PROGRAMMATIC", "true");
    cmd.env(
        "PMR_NODE_IPC_SOCK",
        paths.node_ipc_sock.to_string_lossy().to_string(),
    );

    if program.contains("node") && paths.node_preload_shim_path.exists() {
        let shim_flag = format!("--require {}", paths.node_preload_shim_path.display());
        let existing = env
            .env
            .get("NODE_OPTIONS")
            .cloned()
            .unwrap_or_default();
        let combined = if existing.is_empty() {
            shim_flag
        } else if !existing.contains("pmr-preload.cjs") {
            format!("{} {}", existing, shim_flag)
        } else {
            existing
        };
        cmd.env("NODE_OPTIONS", combined);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn '{}' ({}): {}", env.name, program, e))?;

    let pid = child.id().unwrap_or(0);
    let _ = fs::write(&env.pm_pid_path, pid.to_string());

    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
    if let Some(mut child_stdin) = child.stdin.take() {
        tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                let mut payload = line;
                if !payload.ends_with('\n') {
                    payload.push('\n');
                }
                if child_stdin.write_all(payload.as_bytes()).await.is_err() {
                    break;
                }
                let _ = child_stdin.flush().await;
            }
        });
    }

    // Spawn stdout & stderr log streaming tasks
    if let Some(child_stdout) = child.stdout.take() {
        spawn_log_reader(
            Arc::clone(&god),
            id,
            env.name.clone(),
            env.namespace.clone(),
            "out",
            env.pm_out_log_path.clone(),
            env.pm_log_path.clone(),
            env.disable_logs,
            env.log_type.clone(),
            env.log_date_format.clone(),
            Arc::clone(&log_reload_notify),
            child_stdout,
        );
    }
    if let Some(child_stderr) = child.stderr.take() {
        spawn_log_reader(
            Arc::clone(&god),
            id,
            env.name.clone(),
            env.namespace.clone(),
            "err",
            env.pm_err_log_path.clone(),
            env.pm_log_path.clone(),
            env.disable_logs,
            env.log_type.clone(),
            env.log_date_format.clone(),
            Arc::clone(&log_reload_notify),
            child_stderr,
        );
    }

    let desc = {
        let mut g = god.lock().await;
        if let Some(proc) = g.clusters_db.get_mut(&id) {
            proc.pid = pid;
            proc.stdin_tx = Some(stdin_tx);
            env = proc.pm2_env.clone();
        }
        g.log_pm2(&format!(
            "App [{}:{}] starting in -{}-",
            env.name, id, env.exec_mode
        ));
        g.emit_bus_event(
            "process:event",
            json!(id),
            &env.name,
            &env.namespace,
            json!({
                "event": "start",
                "manually": true,
                "process": { "pm_id": id, "name": env.name, "status": env.status }
            }),
        );
        if !env.wait_ready {
            g.log_pm2(&format!("App [{}:{}] online", env.name, id));
            g.emit_bus_event(
                "process:event",
                json!(id),
                &env.name,
                &env.namespace,
                json!({
                    "event": "online",
                    "manually": false,
                    "process": { "pm_id": id, "name": env.name, "status": ONLINE_STATUS }
                }),
            );
        }
        g.get_formatted_process(id)
            .ok_or_else(|| anyhow!("Process {} missing", id))?
    };

    // If wait_ready is true, wait up to listen_timeout for ready_notify before marking online
    if env.wait_ready {
        let god_ready = Arc::clone(&god);
        let ready_n = Arc::clone(&ready_notify);
        let listen_timeout = env.listen_timeout;
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_millis(listen_timeout), ready_n.notified())
                .await;
            let mut g = god_ready.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&id) {
                if proc.generation == generation && proc.pm2_env.status == LAUNCHING_STATUS {
                    proc.pm2_env.status = ONLINE_STATUS.to_string();
                    let name = proc.pm2_env.name.clone();
                    let ns = proc.pm2_env.namespace.clone();
                    g.log_pm2(&format!("App [{}:{}] online", name, id));
                    g.emit_bus_event(
                        "process:event",
                        json!(id),
                        &name,
                        &ns,
                        json!({
                            "event": "online",
                            "manually": false,
                            "process": { "pm_id": id, "name": name, "status": ONLINE_STATUS }
                        }),
                    );
                }
            }
        });
    }

    // Spawn exit watcher task
    let god_exit = Arc::clone(&god);
    tokio::spawn(async move {
        let status_res = child.wait().await;
        let exit_code = status_res.ok().and_then(|s| s.code()).unwrap_or(-1);
        handle_process_exit(god_exit, id, generation, exit_code, stop_notify).await;
    });

    Ok(desc)
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_log_reader<R>(
    god: Arc<Mutex<God>>,
    pm_id: u32,
    app_name: String,
    namespace: String,
    stream_type: &'static str,
    primary_path: String,
    combined_path: Option<String>,
    disable_logs: bool,
    log_type: Option<String>,
    log_date_format: Option<String>,
    reload_notify: Arc<Notify>,
    reader: R,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        let open_file = |p: &str| -> Option<std::fs::File> {
            if disable_logs || p.is_empty() || p.eq_ignore_ascii_case("/dev/null") {
                return None;
            }
            if let Some(parent) = Path::new(p).parent() {
                let _ = fs::create_dir_all(parent);
            }
            OpenOptions::new().create(true).append(true).open(p).ok()
        };

        let mut primary_file = open_file(&primary_path);
        let mut combined_file = combined_path.as_deref().and_then(open_file);
        let bus_event_name = if stream_type == "err" {
            "log:err"
        } else {
            "log:out"
        };

        loop {
            tokio::select! {
                _ = reload_notify.notified() => {
                    primary_file = open_file(&primary_path);
                    combined_file = combined_path.as_deref().and_then(open_file);
                }
                line_res = lines.next_line() => {
                    let Ok(Some(line)) = line_res else {
                        break;
                    };
                    let formatted = if log_type.as_deref() == Some("json") {
                        let obj = json!({
                            "message": line,
                            "timestamp": Utc::now().to_rfc3339(),
                            "type": stream_type,
                            "process_id": pm_id,
                            "app_name": app_name,
                        });
                        format!("{}\n", obj)
                    } else if let Some(ref fmt_str) = log_date_format {
                        let chrono_fmt = convert_dayjs_to_chrono(fmt_str);
                        let ts = Local::now().format(&chrono_fmt).to_string();
                        format!("{}: {}\n", ts, line)
                    } else {
                        format!("{}\n", line)
                    };

                    if let Some(ref mut f) = primary_file {
                        let _ = f.write_all(formatted.as_bytes());
                    }
                    if let Some(ref mut cf) = combined_file {
                        let _ = cf.write_all(formatted.as_bytes());
                    }

                    let g = god.lock().await;
                    g.emit_bus_event(
                        bus_event_name,
                        json!(pm_id),
                        &app_name,
                        &namespace,
                        json!({ "data": formatted }),
                    );
                }
            }
        }
    });
}

fn convert_dayjs_to_chrono(fmt: &str) -> String {
    if fmt.contains('%') {
        return fmt.to_string();
    }
    fmt.replace("YYYY", "%Y")
        .replace("MM", "%m")
        .replace("DD", "%d")
        .replace("HH", "%H")
        .replace("mm", "%M")
        .replace("ss", "%S")
        .replace("ZZ", "%z")
        .replace('Z', "%:z")
}

async fn handle_process_exit(
    god: Arc<Mutex<God>>,
    id: u32,
    generation: u64,
    exit_code: i32,
    stop_notify: Arc<Notify>,
) {
    let restart_delay_opt = {
        let mut g = god.lock().await;
        if g.pm2_being_killed {
            return;
        }
        let Some(proc) = g.clusters_db.get_mut(&id) else {
            return;
        };
        if proc.generation != generation {
            return;
        }

        let _ = fs::remove_file(&proc.pm2_env.pm_pid_path);
        proc.pid = 0;
        proc.stdin_tx = None;
        proc.ipc_tx = None;
        proc.monit = Monit::default();
        proc.pm2_env.exit_code = Some(exit_code);

        let name = proc.pm2_env.name.clone();
        let ns = proc.pm2_env.namespace.clone();
        let current_status = proc.pm2_env.status.clone();

        let intentional_stop = current_status == STOPPING_STATUS
            || current_status == STOPPED_STATUS
            || current_status == ERRORED_STATUS
            || !proc.pm2_env.autorestart
            || proc.pm2_env.stop_exit_codes.contains(&exit_code);

        if intentional_stop {
            if proc.pm2_env.status != ERRORED_STATUS {
                proc.pm2_env.status = STOPPED_STATUS.to_string();
            }
            let st = proc.pm2_env.status.clone();
            g.log_pm2(&format!(
                "App [{}:{}] exited with code [{}]",
                name, id, exit_code
            ));
            g.emit_bus_event(
                "process:event",
                json!(id),
                &name,
                &ns,
                json!({
                    "event": "exit",
                    "manually": true,
                    "process": { "pm_id": id, "name": name, "status": st, "exit_code": exit_code }
                }),
            );
            return;
        }

        let now = now_ms();
        let uptime = now.saturating_sub(proc.pm2_env.pm_uptime);
        let since_created = now.saturating_sub(proc.pm2_env.created_at.unwrap_or(now));
        let min_uptime = proc.pm2_env.min_uptime;
        let max_restarts = proc.pm2_env.max_restarts;

        if since_created < (min_uptime.saturating_mul(max_restarts as u64)) && uptime < min_uptime {
            proc.pm2_env.unstable_restarts += 1;
        }

        if proc.pm2_env.unstable_restarts >= max_restarts {
            proc.pm2_env.status = ERRORED_STATUS.to_string();
            proc.pm2_env.unstable_restarts = 0;
            proc.pm2_env.created_at = None;
            let exec_p = proc.pm2_env.pm_exec_path.clone();
            g.log_pm2(&format!(
                "Script {} had too many unstable restarts ({}). Stopped. \"errored\"",
                exec_p, max_restarts
            ));
            g.emit_bus_event(
                "process:event",
                json!(id),
                &name,
                &ns,
                json!({
                    "event": "restart overlimit",
                    "manually": false,
                    "process": { "pm_id": id, "name": name, "status": ERRORED_STATUS }
                }),
            );
            return;
        }

        let delay = if proc.pm2_env.exp_backoff_restart_delay > 0 {
            let next_delay = if proc.pm2_env.prev_restart_delay == 0 {
                proc.pm2_env.exp_backoff_restart_delay
            } else {
                ((proc.pm2_env.prev_restart_delay as f64) * 1.5)
                    .floor()
                    .min(15_000.0) as u64
            };
            proc.pm2_env.prev_restart_delay = next_delay;
            next_delay
        } else {
            proc.pm2_env.restart_delay
        };

        if delay > 0 {
            proc.pm2_env.status = WAITING_RESTART.to_string();
        }
        let st = proc.pm2_env.status.clone();
        g.log_pm2(&format!(
            "App [{}:{}] exited with code [{}] via signal, restarting in {}ms",
            name, id, exit_code, delay
        ));
        g.emit_bus_event(
            "process:event",
            json!(id),
            &name,
            &ns,
            json!({
                "event": "exit",
                "manually": false,
                "process": { "pm_id": id, "name": name, "status": st, "exit_code": exit_code }
            }),
        );
        Some(delay)
    };

    if let Some(delay) = restart_delay_opt {
        if delay > 0 {
            tokio::select! {
                _ = stop_notify.notified() => return,
                _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
            }
        }
        let should_start = {
            let mut g = god.lock().await;
            if g.pm2_being_killed {
                false
            } else if let Some(proc) = g.clusters_db.get_mut(&id) {
                if proc.generation == generation
                    && proc.pm2_env.status != STOPPED_STATUS
                    && proc.pm2_env.status != STOPPING_STATUS
                    && proc.pm2_env.status != ERRORED_STATUS
                {
                    proc.pm2_env.restart_time += 1;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if should_start {
            let _ = start_process_id(god, id).await;
        }
    }
}

pub async fn stop_process_id(god: Arc<Mutex<God>>, id: u32) -> Result<ProcessDescription> {
    let (pid, env, ipc_tx, stop_notify) = {
        let mut g = god.lock().await;
        let proc = g
            .clusters_db
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Process {} not found", id))?;

        proc.stop_notify.notify_waiters();
        if proc.pm2_env.status == STOPPED_STATUS || proc.pid == 0 {
            proc.pm2_env.status = STOPPED_STATUS.to_string();
            proc.pid = 0;
            let _ = fs::remove_file(&proc.pm2_env.pm_pid_path);
            return Ok(ProcessDescription {
                pid: 0,
                name: proc.pm2_env.name.clone(),
                pm2_env: proc.pm2_env.clone(),
                pm_id: proc.pm2_env.pm_id,
                monit: Monit::default(),
            });
        }

        proc.pm2_env.status = STOPPING_STATUS.to_string();
        let pid = proc.pid;
        let env = proc.pm2_env.clone();
        let ipc_tx = proc.ipc_tx.clone();
        let stop_n = Arc::clone(&proc.stop_notify);
        g.log_pm2(&format!("Stopping app:{}:{}", env.name, id));
        (pid, env, ipc_tx, stop_n)
    };

    stop_notify.notify_waiters();
    kill_process_gracefully(pid, &env, ipc_tx).await;

    let mut g = god.lock().await;
    if let Some(proc) = g.clusters_db.get_mut(&id) {
        proc.pid = 0;
        proc.stdin_tx = None;
        proc.ipc_tx = None;
        proc.monit = Monit::default();
        proc.pm2_env.axm_actions.clear();
        proc.pm2_env.axm_monitor.clear();
        proc.pm2_env.status = STOPPED_STATUS.to_string();
        let _ = fs::remove_file(&proc.pm2_env.pm_pid_path);
        let name = proc.pm2_env.name.clone();
        let ns = proc.pm2_env.namespace.clone();
        g.emit_bus_event(
            "process:event",
            json!(id),
            &name,
            &ns,
            json!({
                "event": "stop",
                "manually": true,
                "process": { "pm_id": id, "name": name, "status": STOPPED_STATUS }
            }),
        );
    }

    g.get_formatted_process(id)
        .ok_or_else(|| anyhow!("Process {} not found after stop", id))
}

pub async fn kill_process_gracefully(
    pid: u32,
    env: &Pm2Env,
    ipc_tx: Option<mpsc::UnboundedSender<String>>,
) {
    if pid == 0 {
        return;
    }

    let descendants = if env.treekill {
        collect_descendant_pids(pid)
    } else {
        Vec::new()
    };

    // Step 1: shutdown_with_message OR SIGINT
    if env.shutdown_with_message {
        if let Some(tx) = ipc_tx {
            let _ = tx.send(json!({ "__pmr_raw": "shutdown" }).to_string());
        } else {
            send_signal_to_tree(pid, &descendants, libc::SIGINT);
        }
    } else {
        let sig = std::env::var("PM2_KILL_SIGNAL")
            .ok()
            .and_then(|s| parse_unix_signal(&s))
            .unwrap_or(libc::SIGINT);
        send_signal_to_tree(pid, &descendants, sig);
    }

    let retry_ms = env.kill_retry_time.max(25);
    let timeout_ms = env.kill_timeout.max(100);
    let mut elapsed = 0;
    while elapsed < timeout_ms {
        if !process_is_alive(pid) {
            // Ensure descendants are also cleaned up
            for &d in &descendants {
                if process_is_alive(d) {
                    unsafe {
                        libc::kill(d as i32, libc::SIGKILL);
                    }
                }
            }
            return;
        }
        tokio::time::sleep(Duration::from_millis(retry_ms)).await;
        elapsed += retry_ms;
    }

    // Step 2: Escalate to SIGKILL
    let latest_descendants = if env.treekill {
        collect_descendant_pids(pid)
    } else {
        Vec::new()
    };
    send_signal_to_tree(pid, &latest_descendants, libc::SIGKILL);
    for &d in &descendants {
        if process_is_alive(d) {
            unsafe {
                libc::kill(d as i32, libc::SIGKILL);
            }
        }
    }

    let mut elapsed2 = 0;
    while elapsed2 < timeout_ms {
        if !process_is_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(retry_ms)).await;
        elapsed2 += retry_ms;
    }
}

pub fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        if libc::kill(pid as i32, 0) != 0 {
            return false;
        }
    }
    true
}

fn collect_descendant_pids(root_pid: u32) -> Vec<u32> {
    let mut sys = System::new_all();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut children_map: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (pid, proc) in sys.processes() {
        if let Some(parent) = proc.parent() {
            children_map
                .entry(parent.as_u32())
                .or_default()
                .push(pid.as_u32());
        }
    }

    let mut result = Vec::new();
    let mut stack = vec![root_pid];
    while let Some(cur) = stack.pop() {
        if let Some(kids) = children_map.get(&cur) {
            for &k in kids {
                if !result.contains(&k) {
                    result.push(k);
                    stack.push(k);
                }
            }
        }
    }
    result
}

fn send_signal_to_tree(root_pid: u32, descendants: &[u32], sig: i32) {
    for &d in descendants.iter().rev() {
        unsafe {
            libc::kill(d as i32, sig);
        }
    }
    unsafe {
        libc::kill(root_pid as i32, sig);
    }
}

pub fn parse_unix_signal(sig_str: &str) -> Option<i32> {
    if let Ok(num) = sig_str.parse::<i32>() {
        return Some(num);
    }
    match sig_str.trim().to_ascii_uppercase().as_str() {
        "SIGHUP" | "HUP" => Some(libc::SIGHUP),
        "SIGINT" | "INT" => Some(libc::SIGINT),
        "SIGQUIT" | "QUIT" => Some(libc::SIGQUIT),
        "SIGILL" | "ILL" => Some(libc::SIGILL),
        "SIGTRAP" | "TRAP" => Some(libc::SIGTRAP),
        "SIGABRT" | "ABRT" => Some(libc::SIGABRT),
        "SIGKILL" | "KILL" => Some(libc::SIGKILL),
        "SIGUSR1" | "USR1" => Some(libc::SIGUSR1),
        "SIGUSR2" | "USR2" => Some(libc::SIGUSR2),
        "SIGPIPE" | "PIPE" => Some(libc::SIGPIPE),
        "SIGALRM" | "ALRM" => Some(libc::SIGALRM),
        "SIGTERM" | "TERM" => Some(libc::SIGTERM),
        "SIGCONT" | "CONT" => Some(libc::SIGCONT),
        "SIGSTOP" | "STOP" => Some(libc::SIGSTOP),
        "SIGTSTP" | "TSTP" => Some(libc::SIGTSTP),
        _ => None,
    }
}

pub async fn restart_process_id(
    god: Arc<Mutex<God>>,
    id: u32,
    extra_env: BTreeMap<String, String>,
) -> Result<ProcessDescription> {
    let was_running = {
        let mut g = god.lock().await;
        let proc = g
            .clusters_db
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Process {} not found", id))?;
        proc.pm2_env.created_at = Some(now_ms());
        proc.pm2_env.unstable_restarts = 0;
        proc.pm2_env.prev_restart_delay = 0;
        for (k, v) in extra_env {
            proc.pm2_env.env.insert(k, v);
        }
        proc.pm2_env.status == ONLINE_STATUS || proc.pm2_env.status == LAUNCHING_STATUS || proc.pid > 0
    };

    if was_running {
        stop_process_id(Arc::clone(&god), id).await?;
        {
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&id) {
                proc.pm2_env.restart_time += 1;
            }
        }
    }

    start_process_id(god, id).await
}

/// Zero-downtime reload (`pmr reload` / `pmr gracefulReload`).
/// In `cluster_mode` when online, spawns the new worker first and waits for `ready_notify`
/// (which fires on `listening` or `process.send('ready')`) or `listen_timeout` before stopping the old worker!
pub async fn reload_process_id(
    god: Arc<Mutex<God>>,
    id: u32,
    extra_env: BTreeMap<String, String>,
    soft: bool,
) -> Result<ProcessDescription> {
    let (is_cluster_online, old_pid, mut old_env, old_ipc_tx, listen_timeout) = {
        let mut g = god.lock().await;
        let proc = g
            .clusters_db
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Process {} not found", id))?;

        for (k, v) in &extra_env {
            proc.pm2_env.env.insert(k.clone(), v.clone());
        }

        let is_cluster_online = proc.pm2_env.status == ONLINE_STATUS
            && proc.pm2_env.exec_mode == CLUSTER_MODE_ID
            && proc.pid > 0;
        if !is_cluster_online {
            (
                false,
                proc.pid,
                proc.pm2_env.clone(),
                proc.ipc_tx.clone(),
                proc.pm2_env.listen_timeout,
            )
        } else {
            // Detach old PID from current slot so start_process_id launches replacement in slot `id`
            let old_pid = proc.pid;
            let old_env = proc.pm2_env.clone();
            let old_ipc = proc.ipc_tx.take();
            let lt = proc.pm2_env.listen_timeout;
            proc.pid = 0;
            proc.pm2_env.restart_time += 1;
            proc.pm2_env.created_at = Some(now_ms());
            proc.pm2_env.unstable_restarts = 0;
            proc.pm2_env.prev_restart_delay = 0;
            (true, old_pid, old_env, old_ipc, lt)
        }
    };

    if !is_cluster_online {
        return restart_process_id(god, id, extra_env).await;
    }

    let ready_notify = {
        let g = god.lock().await;
        g.clusters_db
            .get(&id)
            .map(|p| Arc::clone(&p.ready_notify))
            .unwrap_or_else(|| Arc::new(Notify::new()))
    };

    // Launch new worker while old worker is still serving traffic!
    let desc = start_process_id(Arc::clone(&god), id).await?;

    // Wait for new worker to signal `listening` or `ready` (or timeout after `listen_timeout`)
    let _ = tokio::time::timeout(
        Duration::from_millis(listen_timeout.max(500)),
        ready_notify.notified(),
    )
    .await;

    // Now gracefully terminate the old worker
    if soft {
        old_env.shutdown_with_message = true;
    }
    kill_process_gracefully(old_pid, &old_env, old_ipc_tx).await;

    Ok(desc)
}

pub async fn delete_process_id(god: Arc<Mutex<God>>, id: u32) -> Result<ProcessDescription> {
    let desc = stop_process_id(Arc::clone(&god), id).await?;
    let mut g = god.lock().await;
    if let Some(mut removed) = g.clusters_db.remove(&id) {
        removed.watcher = None;
        let _ = fs::remove_file(&removed.pm2_env.pm_pid_path);
        g.emit_bus_event(
            "process:event",
            json!(id),
            &removed.pm2_env.name,
            &removed.pm2_env.namespace,
            json!({
                "event": "delete",
                "manually": true,
                "process": { "pm_id": id, "name": removed.pm2_env.name, "status": STOPPED_STATUS }
            }),
        );
    }
    if g.clusters_db.is_empty() {
        g.next_id = 0;
    }
    Ok(desc)
}

pub async fn duplicate_process_id(god: Arc<Mutex<God>>, id: u32) -> Result<ProcessDescription> {
    let mut template_env = {
        let g = god.lock().await;
        let proc = g
            .clusters_db
            .get(&id)
            .ok_or_else(|| anyhow!("Process {} not found", id))?;
        proc.pm2_env.clone()
    };
    template_env.restart_time = 0;
    template_env.unstable_restarts = 0;
    template_env.created_at = Some(now_ms());
    template_env.status = ONLINE_STATUS.to_string();
    // Strip old instance suffix from log/pid paths
    let old_log_suffix = format!("-{}.log", id);
    if let Some(stem) = template_env.pm_out_log_path.strip_suffix(&old_log_suffix) {
        template_env.pm_out_log_path = format!("{}.log", stem);
    }
    if let Some(stem) = template_env.pm_err_log_path.strip_suffix(&old_log_suffix) {
        template_env.pm_err_log_path = format!("{}.log", stem);
    }
    let old_pid_suffix = format!("-{}.pid", id);
    if let Some(stem) = template_env.pm_pid_path.strip_suffix(&old_pid_suffix) {
        template_env.pm_pid_path = format!("{}.pid", stem);
    }

    let mut launched = prepare_and_launch(god, template_env, 1).await?;
    launched
        .pop()
        .ok_or_else(|| anyhow!("Failed to duplicate process {}", id))
}

pub async fn scale_application(
    god: Arc<Mutex<God>>,
    name: &str,
    number_str: &str,
) -> Result<Vec<ProcessDescription>> {
    let matching_ids: Vec<u32> = {
        let g = god.lock().await;
        g.clusters_db
            .values()
            .filter(|p| p.pm2_env.name == name)
            .map(|p| p.pm2_env.pm_id)
            .collect()
    };

    if matching_ids.is_empty() {
        return Err(anyhow!("Application {} not found", name));
    }

    let current_count = matching_ids.len() as isize;
    let target_count = if let Some(add) = number_str.strip_prefix('+') {
        current_count + add.parse::<isize>()?
    } else if let Some(sub) = number_str.strip_prefix('-') {
        (current_count - sub.parse::<isize>()?).max(1)
    } else {
        number_str.parse::<isize>()?.max(1)
    };

    if target_count > current_count {
        let to_add = (target_count - current_count) as usize;
        let first_id = matching_ids[0];
        for _ in 0..to_add {
            duplicate_process_id(Arc::clone(&god), first_id).await?;
        }
    } else if target_count < current_count {
        let to_remove = (current_count - target_count) as usize;
        for &id in matching_ids.iter().rev().take(to_remove) {
            delete_process_id(Arc::clone(&god), id).await?;
        }
    }

    let mut g = god.lock().await;
    Ok(g.get_formatted_processes())
}

pub fn setup_watcher_for_process(god: Arc<Mutex<God>>, g: &mut God, id: u32) {
    let Some(proc) = g.clusters_db.get_mut(&id) else {
        return;
    };
    proc.watcher = None;
    if !proc.pm2_env.watch {
        return;
    }

    let watch_delay = proc.pm2_env.watch_delay.max(250);
    let ignore_patterns = proc.pm2_env.ignore_watch.clone();
    let cwd = PathBuf::from(&proc.pm2_env.pm_cwd);
    let watch_paths: Vec<PathBuf> = if proc.pm2_env.watch_paths.is_empty() {
        vec![cwd.clone()]
    } else {
        proc.pm2_env
            .watch_paths
            .iter()
            .map(|p| {
                let pb = PathBuf::from(p);
                if pb.is_absolute() {
                    pb
                } else {
                    cwd.join(pb)
                }
            })
            .collect()
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let watcher_res = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(ev) = res {
                let _ = tx.send(ev);
            }
        },
        NotifyConfig::default(),
    );

    if let Ok(mut watcher) = watcher_res {
        for wp in &watch_paths {
            if wp.exists() {
                let _ = watcher.watch(wp, RecursiveMode::Recursive);
            }
        }
        proc.watcher = Some(watcher);

        let god_watch = Arc::clone(&god);
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if !ev.kind.is_modify() && !ev.kind.is_create() && !ev.kind.is_remove() {
                    continue;
                }
                let should_ignore = ev.paths.iter().any(|p| {
                    let s = p.to_string_lossy();
                    if s.contains("/.git/")
                        || s.contains("/node_modules/")
                        || s.contains("/target/")
                        || s.ends_with(".log")
                        || s.ends_with(".pid")
                    {
                        return true;
                    }
                    ignore_patterns.iter().any(|pat| s.contains(pat))
                });
                if should_ignore {
                    continue;
                }

                tokio::time::sleep(Duration::from_millis(watch_delay)).await;
                // Drain queued events during debounce window
                while rx.try_recv().is_ok() {}

                let still_watching = {
                    let g = god_watch.lock().await;
                    g.clusters_db
                        .get(&id)
                        .map(|p| p.pm2_env.watch)
                        .unwrap_or(false)
                };
                if still_watching {
                    let _ = restart_process_id(Arc::clone(&god_watch), id, BTreeMap::new()).await;
                } else {
                    break;
                }
            }
        });
    }
}

/// Background Worker & Cron loop (`lib/Worker.js` + `God.registerCron`).
pub fn spawn_worker_and_cron_loops(god: Arc<Mutex<God>>) {
    // 1. Worker loop: refreshes memory/CPU, resets exp_backoff_restart_delay after 30s, checks max_memory_restart
    let god_worker = Arc::clone(&god);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(2_000));
        loop {
            interval.tick().await;
            let to_reload = {
                let mut g = god_worker.lock().await;
                if g.pm2_being_killed {
                    break;
                }
                g.refresh_monitor_data();
                let now = now_ms();
                let mut over_mem = Vec::new();
                for (&id, proc) in g.clusters_db.iter_mut() {
                    if proc.pm2_env.status == ONLINE_STATUS {
                        if proc.pm2_env.exp_backoff_restart_delay > 0
                            && proc.pm2_env.prev_restart_delay > 0
                            && now.saturating_sub(proc.pm2_env.pm_uptime)
                                > EXP_BACKOFF_RESET_TIMER_MS
                        {
                            proc.pm2_env.prev_restart_delay = 0;
                        }

                        if let Some(max_mem) = proc.pm2_env.max_memory_restart {
                            if proc.monit.memory > max_mem {
                                over_mem.push((id, proc.pm2_env.name.clone(), proc.monit.memory, max_mem));
                            }
                        }
                    }
                }
                for (id, name, cur_m, max_m) in &over_mem {
                    g.log_pm2(&format!(
                        "[PM2][WORKER] Process {} ({}) exceeded max_memory_restart ({} > {}), reloading...",
                        id, name, cur_m, max_m
                    ));
                }
                over_mem
            };

            for (id, _, _, _) in to_reload {
                let _ = reload_process_id(Arc::clone(&god_worker), id, BTreeMap::new(), false).await;
            }
        }
    });

    // 2. Cron loop: checks cron_restart schedules every second
    let god_cron = Arc::clone(&god);
    tokio::spawn(async move {
        let mut last_sec = i64::MIN;
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let now = Utc::now();
            let cur_sec = now.timestamp();
            if cur_sec == last_sec {
                continue;
            }
            last_sec = cur_sec;

            let ids_to_restart = {
                let g = god_cron.lock().await;
                if g.pm2_being_killed {
                    break;
                }
                let mut due = Vec::new();
                for (&id, proc) in &g.clusters_db {
                    if proc.pm2_env.status != ONLINE_STATUS {
                        continue;
                    }
                    let Some(ref cron_expr) = proc.pm2_env.cron_restart else {
                        continue;
                    };
                    if cron_expr.is_empty() || cron_expr == "0" {
                        continue;
                    }
                    // Normalize 5-field cron to 6-field cron (prepend "0 " for seconds)
                    let normalized = if cron_expr.split_whitespace().count() == 5 {
                        format!("0 {}", cron_expr)
                    } else {
                        cron_expr.clone()
                    };
                    if let Ok(schedule) = Schedule::from_str(&normalized) {
                        let prev = now - chrono::Duration::seconds(1);
                        if let Some(next_time) = schedule.after(&prev).next() {
                            if next_time.timestamp() == cur_sec {
                                due.push(id);
                            }
                        }
                    }
                }
                due
            };

            for id in ids_to_restart {
                let _ = restart_process_id(Arc::clone(&god_cron), id, BTreeMap::new()).await;
            }
        }
    });
}

/// Handle messages arriving from the Node preload IPC socket (`node_ipc.sock`).
pub async fn handle_node_ipc_message(
    god: Arc<Mutex<God>>,
    msg: Value,
    ipc_tx: mpsc::UnboundedSender<String>,
) {
    let Some(pm_id) = msg.get("pm_id").and_then(|v| v.as_u64()).map(|n| n as u32) else {
        return;
    };
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    let mut g = god.lock().await;
    let Some(proc) = g.clusters_db.get_mut(&pm_id) else {
        return;
    };
    proc.ipc_tx = Some(ipc_tx.clone());

    match msg_type {
        "internal:hello" => {
            if let Some(nv) = msg.get("node_version").and_then(|v| v.as_str()) {
                proc.pm2_env.node_version = Some(nv.to_string());
            }
        }
        "listening" => {
            let req_port = msg.get("requested_port").and_then(|v| v.as_u64()).map(|n| n as u16);
            let wrk_port = msg.get("worker_port").and_then(|v| v.as_u64()).map(|n| n as u16);
            proc.ready_notify.notify_waiters();
            if proc.pm2_env.status == LAUNCHING_STATUS && !proc.pm2_env.wait_ready {
                proc.pm2_env.status = ONLINE_STATUS.to_string();
            }
            if let (Some(rp), Some(wp)) = (req_port, wrk_port) {
                if let Some(wmap) = g.cluster_balancers.get(&rp) {
                    wmap.lock().unwrap().insert(pm_id, wp);
                    let _ = ipc_tx.send(json!({ "__pmr_cluster_ack": true }).to_string());
                } else {
                    let wmap = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
                    wmap.lock().unwrap().insert(pm_id, wp);
                    g.cluster_balancers.insert(rp, Arc::clone(&wmap));
                    let ipc_tx_clone = ipc_tx.clone();
                    tokio::spawn(async move {
                        let listener_res = match tokio::net::TcpListener::bind(("127.0.0.1", rp)).await {
                            Ok(l) => Ok(l),
                            Err(_) => tokio::net::TcpListener::bind(("0.0.0.0", rp)).await,
                        };
                        let _ = ipc_tx_clone.send(json!({ "__pmr_cluster_ack": true }).to_string());
                        if let Ok(listener) = listener_res {
                            let mut rr_idx: usize = 0;
                            while let Ok((mut inbound, _)) = listener.accept().await {
                                let target_wp = {
                                    let guard = wmap.lock().unwrap();
                                    let ports: Vec<u16> = guard.values().copied().collect();
                                    if ports.is_empty() {
                                        None
                                    } else {
                                        let p = ports[rr_idx % ports.len()];
                                        rr_idx = rr_idx.wrapping_add(1);
                                        Some(p)
                                    }
                                };
                                if let Some(twp) = target_wp {
                                    tokio::spawn(async move {
                                        if let Ok(mut outbound) = tokio::net::TcpStream::connect(("127.0.0.1", twp)).await {
                                            let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                                        }
                                    });
                                }
                            }
                        }
                    });
                }
            }
        }
        "process:msg" => {
            let raw = msg.get("raw").cloned().unwrap_or(Value::Null);
            if raw.as_str() == Some("ready") {
                proc.pm2_env.status = ONLINE_STATUS.to_string();
                proc.ready_notify.notify_waiters();
            }
            if let Some(obj) = raw.as_object() {
                if let Some(t) = obj.get("type").and_then(|v| v.as_str()) {
                    match t {
                        "axm:monitor" => {
                            if let Some(data) = obj.get("data").and_then(|d| d.as_object()) {
                                for (k, v) in data {
                                    let metric = if let Some(mobj) = v.as_object() {
                                        AxmMetric {
                                            value: mobj.get("value").cloned().unwrap_or(Value::Null),
                                            unit: mobj
                                                .get("unit")
                                                .and_then(|u| u.as_str())
                                                .unwrap_or("")
                                                .to_string(),
                                        }
                                    } else {
                                        AxmMetric {
                                            value: v.clone(),
                                            unit: String::new(),
                                        }
                                    };
                                    proc.pm2_env.axm_monitor.insert(k.clone(), metric);
                                }
                            }
                        }
                        "axm:action" => {
                            if let Some(data) = obj.get("data").and_then(|d| d.as_object()) {
                                if let Some(aname) = data.get("action_name").and_then(|a| a.as_str()) {
                                    if !proc.pm2_env.axm_actions.iter().any(|a| a.action_name == aname) {
                                        proc.pm2_env.axm_actions.push(AxmAction {
                                            action_name: aname.to_string(),
                                            action_type: data
                                                .get("action_type")
                                                .and_then(|a| a.as_str())
                                                .unwrap_or("custom")
                                                .to_string(),
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            let name = proc.pm2_env.name.clone();
            let ns = proc.pm2_env.namespace.clone();
            g.emit_bus_event(
                "process:msg",
                json!(pm_id),
                &name,
                &ns,
                json!({ "raw": raw }),
            );
        }
        _ => {}
    }
}
