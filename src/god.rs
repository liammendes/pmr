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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
use tokio::sync::{broadcast, mpsc, watch, Mutex, Notify};
use uuid::Uuid;

use crate::config::{find_git_versioning, find_package_version};
use crate::constants::{
    CLUSTER_MODE_ID, ERRORED_STATUS, EXP_BACKOFF_RESET_TIMER_MS, GRACEFUL_TIMEOUT_MS,
    LAUNCHING_STATUS, ONLINE_STATUS, STOPPED_STATUS, STOPPING_STATUS, WAITING_RESTART,
};
use crate::paths::PmrPaths;
use crate::process::{AxmAction, AxmMetric, Monit, Pm2Env, ProcessDescription};
use crate::protocol::{BusEvent, BusProcessRef, ProcessList};

pub struct ClusterBalancer {
    pub workers: Arc<std::sync::Mutex<BTreeMap<u32, u16>>>,
    pub shutdown: Arc<Notify>,
}

pub struct ManagedProcess {
    pub pm2_env: Pm2Env,
    pub pid: u32,
    pub monit: Monit,
    pub stdin_tx: Option<mpsc::UnboundedSender<String>>,
    pub ipc_tx: Option<mpsc::UnboundedSender<String>>,
    pub stop_notify: Arc<Notify>,
    pub ready_tx: watch::Sender<bool>,
    pub exit_tx: watch::Sender<bool>,
    pub log_reload_tx: watch::Sender<u64>,
    pub watcher: Option<RecommendedWatcher>,
    pub generation: u64,
}

pub struct God {
    pub runner: Arc<crate::runner::Runner>,
    pub clusters_db: BTreeMap<u32, ManagedProcess>,
    pub cluster_balancers: BTreeMap<u16, ClusterBalancer>,
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
    pub fn new(
        paths: PmrPaths,
        bus_tx: broadcast::Sender<BusEvent>,
        runner: Arc<crate::runner::Runner>,
    ) -> Arc<Mutex<Self>> {
        let mut sys = System::new_all();
        sys.refresh_all();
        Arc::new(Mutex::new(Self {
            runner,
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

    pub fn unregister_worker(&mut self, pm_id: u32) {
        let mut empty_ports = Vec::new();
        for (&port, balancer) in &self.cluster_balancers {
            let mut guard = balancer.workers.lock().unwrap();
            guard.remove(&pm_id);
            if guard.is_empty() {
                empty_ports.push(port);
            }
        }
        for port in empty_ports {
            if let Some(balancer) = self.cluster_balancers.remove(&port) {
                balancer.shutdown.notify_one();
            }
        }
    }

    pub fn emit_bus_event(
        &self,
        event: &str,
        pm_id: Value,
        name: &str,
        namespace: &str,
        data: Value,
    ) {
        let status = data
            .get("event")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let manually = data.get("manually").and_then(|v| v.as_bool());
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
            status,
            manually,
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
        let ids: Vec<_> = self.clusters_db.keys().copied().collect();
        self.refresh_monitor_data_for(&ids);
    }

    fn refresh_monitor_data_for(&mut self, ids: &[u32]) {
        let pids: Vec<_> = ids
            .iter()
            .filter_map(|id| self.clusters_db.get(id))
            .filter(|p| matches!(p.pm2_env.status.as_str(), ONLINE_STATUS | LAUNCHING_STATUS))
            .map(|p| {
                Pid::from_u32(
                    p.pm2_env
                        .axm_options
                        .get("pid")
                        .and_then(|v| v.as_u64())
                        .map_or(p.pid, |p| p as u32),
                )
            })
            .filter(|pid| pid.as_u32() > 0)
            .collect();
        if !pids.is_empty() {
            self.sys
                .refresh_processes(ProcessesToUpdate::Some(&pids), true);
        }
        for id in ids {
            let Some(proc) = self.clusters_db.get_mut(id) else {
                continue;
            };
            let target_pid = proc
                .pm2_env
                .axm_options
                .get("pid")
                .and_then(|v| v.as_u64())
                .map(|p| p as u32)
                .unwrap_or(proc.pid);

            if (proc.pm2_env.status == ONLINE_STATUS || proc.pm2_env.status == LAUNCHING_STATUS)
                && target_pid > 0
            {
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
            proc.pm2_env.sync_flattened_env();
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
        self.refresh_monitor_data_for(&[id]);
        self.clusters_db.get(&id).map(|p| ProcessDescription {
            pid: p.pid,
            name: p.pm2_env.name.clone(),
            pm2_env: p.pm2_env.clone(),
            pm_id: p.pm2_env.pm_id,
            monit: p.monit.clone(),
        })
    }

    /// Select IDs before refreshing metrics or cloning process environments.
    pub fn get_process_list(&mut self, limit: usize, descending: bool) -> ProcessList {
        let total = self.clusters_db.len();
        let limit = if limit == 0 { total } else { limit };
        let ids: Vec<_> = if descending {
            self.clusters_db.keys().rev().take(limit).copied().collect()
        } else {
            self.clusters_db.keys().take(limit).copied().collect()
        };
        self.refresh_monitor_data_for(&ids);
        let processes = ids
            .into_iter()
            .filter_map(|id| self.clusters_db.get(&id))
            .map(|p| ProcessDescription {
                pid: p.pid,
                name: p.pm2_env.name.clone(),
                pm2_env: p.pm2_env.clone(),
                pm_id: p.pm2_env.pm_id,
                monit: p.monit.clone(),
            })
            .collect();
        ProcessList { total, processes }
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
            let inst_var_name = if env.instance_var.is_empty() {
                "NODE_APP_INSTANCE".to_string()
            } else {
                env.instance_var.clone()
            };
            if inst_var_name != "NODE_APP_INSTANCE" {
                env.env.remove("NODE_APP_INSTANCE");
            }
            env.env.insert(inst_var_name, inst_idx.to_string());

            if let Some(ref inc_var) = env.increment_var {
                let base_num = template_env
                    .env
                    .get(inc_var)
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0);
                env.env
                    .insert(inc_var.clone(), (base_num + inst_idx as i64).to_string());
            }

            suffix_paths_for_instance(&mut env);
            let autostart = env.autostart && env.status != STOPPED_STATUS;
            if !autostart {
                env.status = STOPPED_STATUS.to_string();
            }
            env.sync_flattened_env();

            let (ready_tx, _) = watch::channel(false);
            let (exit_tx, _) = watch::channel(true);
            let (log_reload_tx, _) = watch::channel(0u64);

            let mp = ManagedProcess {
                pm2_env: env.clone(),
                pid: 0,
                monit: Monit::default(),
                stdin_tx: None,
                ipc_tx: None,
                stop_notify: Arc::new(Notify::new()),
                ready_tx,
                exit_tx,
                log_reload_tx,
                watcher: None,
                generation: 0,
            };
            g.clusters_db.insert(id, mp);
            if env.watch && autostart {
                setup_watcher_for_process(Arc::clone(&god), &mut g, id);
            }
            (id, autostart)
        };

        if autostart {
            if let Err(e) = start_process_id(Arc::clone(&god), pm_id).await {
                let mut g = god.lock().await;
                if let Some(mut removed) = g.clusters_db.remove(&pm_id) {
                    removed.watcher = None;
                }
                if g.clusters_db.is_empty() {
                    g.next_id = 0;
                }
                return Err(e);
            }
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

fn strip_numeric_suffix_before_ext<'a>(path: &'a str, ext: &str) -> Option<&'a str> {
    let stem = path.strip_suffix(ext)?;
    if let Some((prefix, digits)) = stem.rsplit_once('-') {
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            return Some(prefix);
        }
    }
    Some(stem)
}

pub fn suffix_paths_for_instance(env: &mut Pm2Env) {
    if !env.merge_logs {
        if env.pm_out_log_path != "/dev/null" {
            if let Some(base) = strip_numeric_suffix_before_ext(&env.pm_out_log_path, ".log") {
                env.pm_out_log_path = format!("{}-{}.log", base, env.pm_id);
            }
        }
        if env.pm_err_log_path != "/dev/null" {
            if let Some(base) = strip_numeric_suffix_before_ext(&env.pm_err_log_path, ".log") {
                env.pm_err_log_path = format!("{}-{}.log", base, env.pm_id);
            } else if let Some(base) = strip_numeric_suffix_before_ext(&env.pm_err_log_path, ".err")
            {
                env.pm_err_log_path = format!("{}-{}.err", base, env.pm_id);
            }
        }
        if let Some(ref mut log_p) = env.pm_log_path {
            if log_p != "/dev/null" {
                if let Some(base) = strip_numeric_suffix_before_ext(log_p, ".log") {
                    *log_p = format!("{}-{}.log", base, env.pm_id);
                }
            }
        }
    }
    if env.pm_pid_path != "/dev/null" {
        if let Some(base) = strip_numeric_suffix_before_ext(&env.pm_pid_path, ".pid") {
            env.pm_pid_path = format!("{}-{}.pid", base, env.pm_id);
        }
    }
}

pub fn start_process_id(
    god: Arc<Mutex<God>>,
    id: u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ProcessDescription>> + Send>> {
    Box::pin(async move {
        let (
            mut env,
            paths,
            bus_tx,
            stop_notify,
            ready_tx,
            mut ready_rx,
            exit_tx,
            log_reload_rx,
            generation,
            permit,
            history,
        ) = {
            let mut g = god.lock().await;
            if g.pm2_being_killed {
                return Err(anyhow!("daemon is shutting down"));
            }
            let paths = g.paths.clone();
            let bus_tx = g.bus_tx.clone();
            if let Some(proc) = g.clusters_db.get(&id) {
                if proc.generation > 0
                    && proc.pm2_env.status == LAUNCHING_STATUS
                    && !*proc.exit_tx.borrow()
                {
                    return Err(anyhow!("Process {} is already launching", id));
                }
                if proc.pm2_env.status == ONLINE_STATUS && proc.pid > 0 {
                    return Ok(ProcessDescription {
                        pid: proc.pid,
                        name: proc.pm2_env.name.clone(),
                        pm2_env: proc.pm2_env.clone(),
                        pm_id: id,
                        monit: proc.monit.clone(),
                    });
                }
            }
            let permit = g.runner.capacity.acquire()
                .ok_or_else(|| anyhow!("running process limit reached; use pmr run to queue commands or pmr limits to adjust"))?;
            let history = Arc::clone(&g.runner.store);
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
            let (r_tx, r_rx) = watch::channel(false);
            let (e_tx, _) = watch::channel(false);
            proc.ready_tx = r_tx.clone();
            proc.exit_tx = e_tx.clone();
            let log_reload_rx = proc.log_reload_tx.subscribe();

            proc.pm2_env.pm_uptime = now_ms();
            if proc.pm2_env.created_at.is_none() {
                proc.pm2_env.created_at = Some(proc.pm2_env.pm_uptime);
            }
            proc.pm2_env.status = LAUNCHING_STATUS.to_string();

            // Refresh version & git metadata
            proc.pm2_env.version = find_package_version(
                Path::new(&proc.pm2_env.pm_exec_path),
                Path::new(&proc.pm2_env.pm_cwd),
            );
            if proc.pm2_env.vizion {
                proc.pm2_env.versioning = find_git_versioning(Path::new(&proc.pm2_env.pm_cwd));
            }
            proc.pm2_env.sync_flattened_env();

            if proc.pm2_env.watch && proc.watcher.is_none() {
                setup_watcher_for_process(Arc::clone(&god), &mut g, id);
            }

            let proc = g.clusters_db.get(&id).unwrap();
            (
                proc.pm2_env.clone(),
                paths,
                bus_tx,
                Arc::clone(&proc.stop_notify),
                r_tx,
                r_rx,
                e_tx,
                log_reload_rx,
                gen,
                permit,
                history,
            )
        };

        // Build command & arguments
        let (program, cmd_args) =
            if env.exec_interpreter == "none" || env.exec_interpreter.is_empty() {
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

        #[cfg(unix)]
        {
            if let Some(uid) = env.uid {
                cmd.uid(uid);
            }
            if let Some(gid) = env.gid {
                cmd.gid(gid);
            }
            unsafe {
                cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }
        }

        // Populate child environment
        for (k, v) in &env.env {
            cmd.env(k, v);
        }
        cmd.env("pm_id", env.pm_id.to_string());
        cmd.env("name", &env.name);
        cmd.env("namespace", &env.namespace);
        cmd.env("exec_mode", &env.exec_mode);
        cmd.env("exec_interpreter", &env.exec_interpreter);
        cmd.env("pm_exec_path", &env.pm_exec_path);
        cmd.env("pm_cwd", &env.pm_cwd);
        cmd.env("pm_out_log_path", &env.pm_out_log_path);
        cmd.env("pm_err_log_path", &env.pm_err_log_path);
        cmd.env("pm_pid_path", &env.pm_pid_path);
        cmd.env("pm_uptime", env.pm_uptime.to_string());
        cmd.env("status", &env.status);
        cmd.env("username", &env.username);
        cmd.env("vizion", env.vizion.to_string());
        cmd.env("automation", env.pmx.to_string());
        cmd.env("pmx", env.pmx.to_string());
        cmd.env("autostart", env.autostart.to_string());
        cmd.env("autorestart", env.autorestart.to_string());
        cmd.env("treekill", env.treekill.to_string());
        cmd.env("instance_var", &env.instance_var);
        cmd.env("restart_time", env.restart_time.to_string());
        cmd.env("unstable_restarts", env.unstable_restarts.to_string());
        cmd.env(
            "created_at",
            env.created_at.unwrap_or(env.pm_uptime).to_string(),
        );
        if let Some(inst) = env.node_app_instance {
            let iv = if env.instance_var.is_empty() {
                "NODE_APP_INSTANCE"
            } else {
                &env.instance_var
            };
            cmd.env(iv, inst.to_string());
        }
        cmd.env("PM2_HOME", paths.pm2_home.to_string_lossy().to_string());
        cmd.env("PMR_HOME", paths.pm2_home.to_string_lossy().to_string());
        cmd.env("PM2_USAGE", "CLI");
        cmd.env("PMR_PARENT_PID", std::process::id().to_string());
        cmd.env(
            "PMR_NODE_IPC_SOCK",
            paths.node_ipc_sock.to_string_lossy().to_string(),
        );

        if (program.contains("node") || program.contains("bun"))
            && paths.node_preload_shim_path.exists()
        {
            let shim_flag = format!("--require {}", paths.node_preload_shim_path.display());
            let existing = env.env.get("NODE_OPTIONS").cloned().unwrap_or_default();
            let combined = if existing.is_empty() {
                shim_flag
            } else if !existing.contains("pmr-preload.cjs") {
                format!("{} {}", existing, shim_flag)
            } else {
                existing
            };
            cmd.env("NODE_OPTIONS", combined);
        }

        let history_spec = crate::history::CommandSpec {
            id: Uuid::new_v4().to_string(),
            program: program.clone(),
            args: cmd_args.clone(),
            cwd: env.pm_cwd.clone(),
            name: env.name.clone(),
            namespace: env.namespace.clone(),
            env: BTreeMap::new(),
        };
        let history_seq = match history
            .call(move |s| s.submit(history_spec, "managed"))
            .await
        {
            Ok(record) => record.seq,
            Err(error) => {
                let mut g = god.lock().await;
                if let Some(proc) = g.clusters_db.get_mut(&id) {
                    proc.pm2_env.status = ERRORED_STATUS.into();
                }
                let _ = exit_tx.send(true);
                return Err(error);
            }
        };
        let mut spawn_guard = god.lock().await;
        let valid = !spawn_guard.pm2_being_killed
            && spawn_guard.clusters_db.get(&id).is_some_and(|p| {
                p.generation == generation && p.pm2_env.status == LAUNCHING_STATUS
            });
        if !valid {
            drop(spawn_guard);
            let _ = history
                .call(move |s| s.finish(history_seq, "cancelled", None))
                .await;
            let _ = exit_tx.send(true);
            return Err(anyhow!("Process launch cancelled"));
        }
        let spawned = cmd.spawn();
        if let Ok(child) = &spawned {
            if let Some(proc) = spawn_guard.clusters_db.get_mut(&id) {
                proc.pid = child.id().unwrap_or(0);
                if !proc.pm2_env.wait_ready {
                    proc.pm2_env.status = ONLINE_STATUS.into();
                }
            }
        }
        drop(spawn_guard);
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                let _ = history
                    .call(move |s| s.finish(history_seq, "failed", None))
                    .await;
                let mut g = god.lock().await;
                let _ = exit_tx.send(true);
                if let Some(proc) = g.clusters_db.get_mut(&id) {
                    proc.pid = 0;
                    proc.pm2_env.status = ERRORED_STATUS.to_string();
                    proc.pm2_env.sync_flattened_env();
                }
                g.log_pm2(&format!(
                    "Failed to spawn App [{}:{}] ({}): {}",
                    env.name, id, program, e
                ));
                g.emit_bus_event(
                    "process:event",
                    json!(id),
                    &env.name,
                    &env.namespace,
                    json!({
                        "event": "exit",
                        "manually": false,
                        "process": { "pm_id": id, "name": env.name, "status": ERRORED_STATUS }
                    }),
                );
                return Err(anyhow!(
                    "Failed to spawn '{}' ({}): {}",
                    env.name,
                    program,
                    e
                ));
            }
        };

        let pid = child.id().unwrap_or(0);
        if let Err(error) = history.call(move |s| s.set_pid(history_seq, pid)).await {
            eprintln!("history pid update: {error}");
        }
        if env.pm_pid_path != "/dev/null" {
            if let Some(parent) = Path::new(&env.pm_pid_path).parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&env.pm_pid_path, pid.to_string());
        }

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
                bus_tx.clone(),
                id,
                env.name.clone(),
                env.namespace.clone(),
                "out",
                env.pm_out_log_path.clone(),
                env.pm_log_path.clone(),
                env.disable_logs,
                env.log_type.clone(),
                env.log_date_format.clone(),
                log_reload_rx.clone(),
                child_stdout,
            );
        }
        if let Some(child_stderr) = child.stderr.take() {
            spawn_log_reader(
                bus_tx,
                id,
                env.name.clone(),
                env.namespace.clone(),
                "err",
                env.pm_err_log_path.clone(),
                env.pm_log_path.clone(),
                env.disable_logs,
                env.log_type.clone(),
                env.log_date_format.clone(),
                log_reload_rx,
                child_stderr,
            );
        }

        {
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&id) {
                proc.pid = pid;
                proc.stdin_tx = Some(stdin_tx);
                proc.pm2_env.sync_flattened_env();
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
                if env.exec_mode != CLUSTER_MODE_ID {
                    let _ = ready_tx.send(true);
                }
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
        }

        // Spawn exit watcher task before waiting for ready so early crashes are caught immediately
        let god_exit = Arc::clone(&god);
        let mut exit_rx_for_ready = exit_tx.subscribe();
        tokio::spawn(async move {
            let status_res = child.wait().await;
            drop(permit);
            let _ = exit_tx.send(true);
            let exit_code = status_res.ok().and_then(|s| s.code()).unwrap_or(-1);
            if let Err(error) = history
                .call(move |s| {
                    s.finish(
                        history_seq,
                        if exit_code == 0 {
                            "succeeded"
                        } else {
                            "failed"
                        },
                        Some(exit_code),
                    )
                })
                .await
            {
                eprintln!("history completion: {error}");
            }
            handle_process_exit(god_exit, id, generation, exit_code, stop_notify).await;
        });

        // If wait_ready is true, wait up to listen_timeout for ready_tx (or early child exit)
        if env.wait_ready {
            let listen_timeout = env.listen_timeout.max(100);
            if !*ready_rx.borrow() {
                let _ = tokio::time::timeout(Duration::from_millis(listen_timeout), async {
                    tokio::select! {
                        _ = ready_rx.changed() => {}
                        _ = exit_rx_for_ready.changed() => {}
                    }
                })
                .await;
            }
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&id) {
                if !*exit_rx_for_ready.borrow()
                    && proc.generation == generation
                    && proc.pm2_env.status == LAUNCHING_STATUS
                {
                    proc.pm2_env.status = ONLINE_STATUS.to_string();
                    proc.pm2_env.sync_flattened_env();
                    let _ = proc.ready_tx.send(true);
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
        }

        let mut g = god.lock().await;
        g.get_formatted_process(id)
            .ok_or_else(|| anyhow!("Process {} missing", id))
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_log_reader<R>(
    bus_tx: broadcast::Sender<BusEvent>,
    pm_id: u32,
    app_name: String,
    namespace: String,
    stream_type: &'static str,
    primary_path: String,
    combined_path: Option<String>,
    disable_logs: bool,
    log_type: Option<String>,
    log_date_format: Option<String>,
    mut reload_rx: watch::Receiver<u64>,
    mut reader: R,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        if disable_logs {
            let _ = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await;
            return;
        }

        let open_file = |p: &str| -> Option<std::fs::File> {
            if p.is_empty() || p.eq_ignore_ascii_case("/dev/null") || p == "NULL" {
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

        let emit_line = |raw_line: &str,
                         primary_file: &mut Option<std::fs::File>,
                         combined_file: &mut Option<std::fs::File>| {
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
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
                if fmt_str.is_empty() {
                    format!("{}\n", line)
                } else {
                    let chrono_fmt = convert_dayjs_to_chrono(fmt_str);
                    let ts = Local::now().format(&chrono_fmt).to_string();
                    format!("{}: {}\n", ts, line)
                }
            } else {
                format!("{}\n", line)
            };

            if let Some(ref mut f) = primary_file {
                let _ = f.write_all(formatted.as_bytes());
            }
            if let Some(ref mut cf) = combined_file {
                let _ = cf.write_all(formatted.as_bytes());
            }

            let _ = bus_tx.send(BusEvent {
                event: bus_event_name.to_string(),
                at: now_ms(),
                process: BusProcessRef {
                    pm_id: json!(pm_id),
                    name: app_name.clone(),
                    namespace: namespace.clone(),
                    rev: None,
                },
                data: json!({ "data": formatted }),
                status: None,
                manually: None,
            });
        };

        let mut pending: Vec<u8> = Vec::with_capacity(4096);
        let mut buf = [0u8; 4096];
        let mut reload_open = true;

        loop {
            tokio::select! {
                changed = reload_rx.changed(), if reload_open => {
                    if changed.is_ok() {
                        primary_file = open_file(&primary_path);
                        combined_file = combined_path.as_deref().and_then(open_file);
                    } else {
                        reload_open = false;
                    }
                }
                read_res = reader.read(&mut buf) => {
                    match read_res {
                        Ok(0) | Err(_) => {
                            if !pending.is_empty() {
                                let line = String::from_utf8_lossy(&pending);
                                emit_line(&line, &mut primary_file, &mut combined_file);
                                pending.clear();
                            }
                            break;
                        }
                        Ok(n) => {
                            pending.extend_from_slice(&buf[..n]);
                            while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                                let line_bytes: Vec<u8> = pending.drain(..=pos).collect();
                                let without_nl = &line_bytes[..line_bytes.len() - 1];
                                let line = String::from_utf8_lossy(without_nl);
                                emit_line(&line, &mut primary_file, &mut combined_file);
                            }
                        }
                    }
                }
            }
        }
    });
}

pub fn convert_dayjs_to_chrono(fmt: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + 8);
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(end) = fmt[i + 1..].find(']') {
                let literal = &fmt[i + 1..i + 1 + end];
                for ch in literal.chars() {
                    if ch == '%' {
                        out.push_str("%%");
                    } else {
                        out.push(ch);
                    }
                }
                i += end + 2;
                continue;
            }
        }
        let rem = &fmt[i..];
        if rem.starts_with("YYYY") {
            out.push_str("%Y");
            i += 4;
        } else if rem.starts_with("YY") {
            out.push_str("%y");
            i += 2;
        } else if rem.starts_with("MMMM") {
            out.push_str("%B");
            i += 4;
        } else if rem.starts_with("MMM") {
            out.push_str("%b");
            i += 3;
        } else if rem.starts_with("MM") {
            out.push_str("%m");
            i += 2;
        } else if rem.starts_with('M') {
            out.push_str("%-m");
            i += 1;
        } else if rem.starts_with("dddd") {
            out.push_str("%A");
            i += 4;
        } else if rem.starts_with("ddd") {
            out.push_str("%a");
            i += 3;
        } else if rem.starts_with("DD") {
            out.push_str("%d");
            i += 2;
        } else if rem.starts_with('D') {
            out.push_str("%-d");
            i += 1;
        } else if rem.starts_with("HH") {
            out.push_str("%H");
            i += 2;
        } else if rem.starts_with('H') {
            out.push_str("%-H");
            i += 1;
        } else if rem.starts_with("hh") {
            out.push_str("%I");
            i += 2;
        } else if rem.starts_with('h') {
            out.push_str("%-I");
            i += 1;
        } else if rem.starts_with("mm") {
            out.push_str("%M");
            i += 2;
        } else if rem.starts_with('m') {
            out.push_str("%-M");
            i += 1;
        } else if rem.starts_with("ss") {
            out.push_str("%S");
            i += 2;
        } else if rem.starts_with('s') {
            out.push_str("%-S");
            i += 1;
        } else if rem.starts_with("SSS") {
            out.push_str("%3f");
            i += 3;
        } else if rem.starts_with("ZZ") {
            out.push_str("%z");
            i += 2;
        } else if rem.starts_with('Z') {
            out.push_str("%:z");
            i += 1;
        } else if rem.starts_with('A') {
            out.push_str("%p");
            i += 1;
        } else if rem.starts_with('a') {
            out.push_str("%P");
            i += 1;
        } else {
            let ch = rem.chars().next().unwrap();
            if ch == '%' {
                out.push_str("%%");
            } else {
                out.push(ch);
            }
            i += ch.len_utf8();
        }
    }
    out
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
        let is_current_gen = g
            .clusters_db
            .get(&id)
            .map(|p| p.generation == generation)
            .unwrap_or(false);
        if !is_current_gen {
            return;
        }
        g.unregister_worker(id);

        let Some(proc) = g.clusters_db.get_mut(&id) else {
            return;
        };

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
            proc.pm2_env.sync_flattened_env();
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
        let min_uptime = proc.pm2_env.min_uptime;
        let max_restarts = proc.pm2_env.max_restarts;

        if uptime < min_uptime {
            proc.pm2_env.unstable_restarts += 1;
        }

        if proc.pm2_env.unstable_restarts >= max_restarts {
            proc.pm2_env.status = ERRORED_STATUS.to_string();
            proc.pm2_env.unstable_restarts = 0;
            proc.pm2_env.created_at = None;
            proc.pm2_env.sync_flattened_env();
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
        proc.pm2_env.sync_flattened_env();
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
                    proc.pm2_env.sync_flattened_env();
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
    let (pid, env, ipc_tx, stop_notify, exit_rx) = {
        let mut g = god.lock().await;
        g.unregister_worker(id);
        let proc = g
            .clusters_db
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Process {} not found", id))?;

        proc.stop_notify.notify_waiters();
        proc.watcher = None;
        if proc.pm2_env.status == STOPPED_STATUS || proc.pid == 0 {
            proc.pm2_env.status = STOPPED_STATUS.to_string();
            proc.pid = 0;
            proc.pm2_env.sync_flattened_env();
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
        proc.pm2_env.sync_flattened_env();
        let pid = proc.pid;
        let env = proc.pm2_env.clone();
        let ipc_tx = proc.ipc_tx.clone();
        let stop_n = Arc::clone(&proc.stop_notify);
        let exit_rx = proc.exit_tx.subscribe();
        g.log_pm2(&format!("Stopping app:{}:{}", env.name, id));
        (pid, env, ipc_tx, stop_n, exit_rx)
    };

    stop_notify.notify_waiters();
    kill_process_gracefully(pid, &env, ipc_tx, Some(exit_rx)).await;

    let mut g = god.lock().await;
    g.unregister_worker(id);
    if let Some(proc) = g.clusters_db.get_mut(&id) {
        proc.pid = 0;
        proc.stdin_tx = None;
        proc.ipc_tx = None;
        proc.monit = Monit::default();
        proc.pm2_env.axm_actions.clear();
        proc.pm2_env.axm_monitor.clear();
        proc.pm2_env.status = STOPPED_STATUS.to_string();
        proc.pm2_env.sync_flattened_env();
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
    mut exit_rx: Option<watch::Receiver<bool>>,
) {
    if pid == 0 {
        return;
    }

    let descendants = if env.treekill {
        collect_descendant_pids(pid)
    } else {
        Vec::new()
    };

    // Step 1: shutdown_with_message OR SIGINT (or PM2_KILL_SIGNAL)
    let default_sig = std::env::var("PM2_KILL_SIGNAL")
        .ok()
        .and_then(|s| parse_unix_signal(&s))
        .unwrap_or(libc::SIGINT);

    if env.shutdown_with_message {
        if let Some(ref tx) = ipc_tx {
            let _ = tx.send(json!({ "__pmr_raw": "shutdown" }).to_string());
        } else {
            send_signal_to_tree(pid, &descendants, default_sig);
        }
    } else {
        send_signal_to_tree(pid, &descendants, default_sig);
    }

    let retry_ms = env.kill_retry_time.max(25);
    let timeout_ms = env.kill_timeout.max(100);
    let start_instant = tokio::time::Instant::now();
    let deadline = start_instant + Duration::from_millis(timeout_ms);

    loop {
        let root_exited = match exit_rx {
            Some(ref rx) => *rx.borrow() || !process_is_alive(pid),
            None => !process_is_alive(pid),
        };
        if root_exited {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if let Some(ref mut rx) = exit_rx {
            let _ = tokio::time::timeout(Duration::from_millis(retry_ms), rx.changed()).await;
        } else {
            tokio::time::sleep(Duration::from_millis(retry_ms)).await;
        }
    }

    // If shutdown_with_message was used and process is still alive after timeout, send SIGINT first if soft reload
    let root_still_alive = match exit_rx {
        Some(ref rx) => !*rx.borrow() && process_is_alive(pid),
        None => process_is_alive(pid),
    };

    if !root_still_alive && env.treekill && !descendants.is_empty() {
        // Give descendants up to the remaining kill_timeout to exit gracefully before SIGKILL
        while tokio::time::Instant::now() < deadline {
            if descendants.iter().all(|&d| !process_is_alive(d)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(retry_ms)).await;
        }
    }

    // Step 2: Escalate to SIGKILL on any surviving processes
    let latest_descendants = if env.treekill {
        collect_descendant_pids(pid)
    } else {
        Vec::new()
    };
    if root_still_alive {
        send_signal_to_tree(pid, &latest_descendants, libc::SIGKILL);
    }
    for &d in &descendants {
        if process_is_alive(d) {
            unsafe {
                libc::kill(d as i32, libc::SIGKILL);
            }
        }
    }

    let mut elapsed2 = 0;
    while elapsed2 < timeout_ms.min(1000) {
        let exited = match exit_rx {
            Some(ref rx) => *rx.borrow() || !process_is_alive(pid),
            None => !process_is_alive(pid),
        };
        if exited {
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
    conf: Option<Value>,
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
        if let Some(ref patch) = conf {
            proc.pm2_env.apply_conf_patch(patch);
        }
        for (k, v) in extra_env {
            proc.pm2_env.env.insert(k, v);
        }
        suffix_paths_for_instance(&mut proc.pm2_env);
        proc.pm2_env.sync_flattened_env();
        proc.pm2_env.status == ONLINE_STATUS
            || proc.pm2_env.status == LAUNCHING_STATUS
            || proc.pid > 0
    };

    if was_running {
        stop_process_id(Arc::clone(&god), id).await?;
        {
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&id) {
                proc.pm2_env.restart_time += 1;
                proc.pm2_env.sync_flattened_env();
            }
        }
    }

    start_process_id(god, id).await
}

pub async fn reload_process_id(
    god: Arc<Mutex<God>>,
    id: u32,
    extra_env: BTreeMap<String, String>,
    conf: Option<Value>,
    soft: bool,
) -> Result<ProcessDescription> {
    let (
        is_cluster_online,
        old_pid,
        mut old_env,
        old_ipc_tx,
        old_exit_rx,
        listen_timeout,
        previous_lifecycle,
    ) = {
        let mut g = god.lock().await;
        let proc = g
            .clusters_db
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Process {} not found", id))?;

        let previous_lifecycle = (
            proc.generation,
            proc.exit_tx.clone(),
            proc.ready_tx.clone(),
            Arc::clone(&proc.stop_notify),
        );
        if let Some(ref patch) = conf {
            proc.pm2_env.apply_conf_patch(patch);
        }
        for (k, v) in &extra_env {
            proc.pm2_env.env.insert(k.clone(), v.clone());
        }
        suffix_paths_for_instance(&mut proc.pm2_env);
        proc.pm2_env.sync_flattened_env();

        let is_cluster_online = proc.pm2_env.status == ONLINE_STATUS
            && proc.pm2_env.exec_mode == CLUSTER_MODE_ID
            && proc.pid > 0;
        if !is_cluster_online {
            if soft && proc.pid > 0 {
                proc.pm2_env.status = STOPPING_STATUS.to_string();
                proc.pm2_env.sync_flattened_env();
            }
            (
                false,
                proc.pid,
                proc.pm2_env.clone(),
                proc.ipc_tx.clone(),
                proc.exit_tx.subscribe(),
                proc.pm2_env.listen_timeout,
                previous_lifecycle,
            )
        } else {
            let old_pid = proc.pid;
            let old_env = proc.pm2_env.clone();
            let old_ipc = proc.ipc_tx.take();
            let old_exit_rx = proc.exit_tx.subscribe();
            let lt = proc.pm2_env.listen_timeout;
            proc.pid = 0;
            proc.pm2_env.restart_time += 1;
            proc.pm2_env.created_at = Some(now_ms());
            proc.pm2_env.unstable_restarts = 0;
            proc.pm2_env.prev_restart_delay = 0;
            proc.pm2_env.sync_flattened_env();
            (
                true,
                old_pid,
                old_env,
                old_ipc,
                old_exit_rx,
                lt,
                previous_lifecycle,
            )
        }
    };

    if !is_cluster_online {
        if soft {
            // For soft/gracefulReload in fork mode, send shutdown message / wait before restart
            let has_running = old_pid > 0;
            if has_running {
                old_env.shutdown_with_message = true;
                old_env.kill_timeout = old_env.kill_timeout.max(GRACEFUL_TIMEOUT_MS);
                kill_process_gracefully(old_pid, &old_env, old_ipc_tx, Some(old_exit_rx)).await;
                let mut g = god.lock().await;
                if let Some(proc) = g.clusters_db.get_mut(&id) {
                    proc.pid = 0;
                    proc.pm2_env.status = STOPPED_STATUS.to_string();
                    proc.pm2_env.restart_time += 1;
                    proc.pm2_env.sync_flattened_env();
                }
                drop(g);
                return start_process_id(god, id).await;
            }
        }
        return restart_process_id(god, id, extra_env, None).await;
    }

    // Launch new worker while old worker is still serving traffic
    let desc = match start_process_id(Arc::clone(&god), id).await {
        Ok(desc) => desc,
        Err(error) => {
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&id) {
                proc.pid = old_pid;
                proc.pm2_env = old_env;
                proc.ipc_tx = old_ipc_tx;
                (
                    proc.generation,
                    proc.exit_tx,
                    proc.ready_tx,
                    proc.stop_notify,
                ) = previous_lifecycle;
            }
            return Err(error);
        }
    };

    let mut ready_rx = {
        let g = god.lock().await;
        g.clusters_db
            .get(&id)
            .map(|p| p.ready_tx.subscribe())
            .unwrap_or_else(|| watch::channel(true).1)
    };

    // Wait for new worker to signal `listening` or `ready` (or timeout after `listen_timeout`)
    if !*ready_rx.borrow() {
        let _ = tokio::time::timeout(
            Duration::from_millis(listen_timeout.max(500)),
            ready_rx.changed(),
        )
        .await;
    }

    // Gracefully terminate the old worker
    if soft {
        old_env.shutdown_with_message = true;
        old_env.kill_timeout = old_env.kill_timeout.max(GRACEFUL_TIMEOUT_MS);
    }
    kill_process_gracefully(old_pid, &old_env, old_ipc_tx, Some(old_exit_rx)).await;

    let mut g = god.lock().await;
    Ok(g.get_formatted_process(id).unwrap_or(desc))
}

pub async fn delete_process_id(god: Arc<Mutex<God>>, id: u32) -> Result<ProcessDescription> {
    let desc = stop_process_id(Arc::clone(&god), id).await?;
    let mut g = god.lock().await;
    g.unregister_worker(id);
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
                    ignore_patterns
                        .iter()
                        .any(|pat| !pat.is_empty() && s.contains(pat))
                });
                if should_ignore {
                    continue;
                }

                tokio::time::sleep(Duration::from_millis(watch_delay)).await;
                while rx.try_recv().is_ok() {}

                let still_watching = {
                    let g = god_watch.lock().await;
                    if g.pm2_being_killed {
                        false
                    } else if let Some(p) = g.clusters_db.get(&id) {
                        p.pm2_env.watch
                    } else {
                        false
                    }
                };
                if still_watching {
                    let _ =
                        restart_process_id(Arc::clone(&god_watch), id, BTreeMap::new(), None).await;
                } else {
                    break;
                }
            }
        });
    }
}

fn normalize_cron_dow_field(dow: &str) -> String {
    let map_num = |token: &str| -> String {
        match token {
            "0" | "7" => "SUN".to_string(),
            "1" => "MON".to_string(),
            "2" => "TUE".to_string(),
            "3" => "WED".to_string(),
            "4" => "THU".to_string(),
            "5" => "FRI".to_string(),
            "6" => "SAT".to_string(),
            other => other.to_string(),
        }
    };
    dow.split(',')
        .map(|part| {
            let (range_part, step_part) = match part.split_once('/') {
                Some((r, s)) => (r, Some(s)),
                None => (part, None),
            };
            let mapped_range = if let Some((a, b)) = range_part.split_once('-') {
                format!("{}-{}", map_num(a), map_num(b))
            } else {
                map_num(range_part)
            };
            match step_part {
                Some(s) => format!("{}/{}", mapped_range, s),
                None => mapped_range,
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub fn normalize_cron_expr(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() == 5 {
        let dow = normalize_cron_dow_field(parts[4]);
        format!(
            "0 {} {} {} {} {}",
            parts[0], parts[1], parts[2], parts[3], dow
        )
    } else if parts.len() == 6 {
        let dow = normalize_cron_dow_field(parts[5]);
        format!(
            "{} {} {} {} {} {}",
            parts[0], parts[1], parts[2], parts[3], parts[4], dow
        )
    } else {
        expr.to_string()
    }
}

/// Background Worker & Cron loop (`lib/Worker.js` + `God.registerCron`).
pub fn spawn_worker_and_cron_loops(god: Arc<Mutex<God>>) {
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
                                over_mem.push((
                                    id,
                                    proc.pm2_env.name.clone(),
                                    proc.monit.memory,
                                    max_mem,
                                ));
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
                let _ =
                    reload_process_id(Arc::clone(&god_worker), id, BTreeMap::new(), None, false)
                        .await;
            }
        }
    });

    // Cron loop: checks cron_restart schedules every second using local time
    let god_cron = Arc::clone(&god);
    tokio::spawn(async move {
        let mut last_sec = i64::MIN;
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let now = Local::now();
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
                    if proc.pm2_env.status == STOPPING_STATUS
                        || proc.pm2_env.status == LAUNCHING_STATUS
                    {
                        continue;
                    }
                    let Some(ref cron_expr) = proc.pm2_env.cron_restart else {
                        continue;
                    };
                    if cron_expr.is_empty() || cron_expr == "0" {
                        continue;
                    }
                    let normalized = normalize_cron_expr(cron_expr);
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
                let _ = restart_process_id(Arc::clone(&god_cron), id, BTreeMap::new(), None).await;
            }
        }
    });
}

async fn bind_cluster_listener(
    requested_host: Option<&str>,
    port: u16,
) -> std::io::Result<tokio::net::TcpListener> {
    if let Some(host) = requested_host {
        let h = host.trim();
        if !h.is_empty() && h != "0.0.0.0" && h != "::" && h != "[::]" {
            if let Ok(l) = tokio::net::TcpListener::bind((h, port)).await {
                return Ok(l);
            }
        }
    }
    if let Ok(l) = tokio::net::TcpListener::bind(("::", port)).await {
        return Ok(l);
    }
    if let Ok(l) = tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        return Ok(l);
    }
    tokio::net::TcpListener::bind(("127.0.0.1", port)).await
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
    let msg_pid = msg
        .get("pid")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    let mut g = god.lock().await;
    let Some(proc) = g.clusters_db.get_mut(&pm_id) else {
        return;
    };
    if proc.pid > 0 && msg_pid > 0 && proc.pid != msg_pid {
        return;
    }
    proc.ipc_tx = Some(ipc_tx.clone());

    match msg_type {
        "internal:hello" => {
            if let Some(nv) = msg.get("node_version").and_then(|v| v.as_str()) {
                proc.pm2_env.node_version = Some(nv.to_string());
                proc.pm2_env.sync_flattened_env();
            }
        }
        "listening" => {
            let req_port = msg
                .get("requested_port")
                .and_then(|v| v.as_u64())
                .map(|n| n as u16);
            let wrk_port = msg
                .get("worker_port")
                .and_then(|v| v.as_u64())
                .map(|n| n as u16);
            let req_host = msg
                .get("requested_host")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if !proc.pm2_env.wait_ready {
                let _ = proc.ready_tx.send(true);
                if proc.pm2_env.status == LAUNCHING_STATUS {
                    proc.pm2_env.status = ONLINE_STATUS.to_string();
                    proc.pm2_env.sync_flattened_env();
                }
            }
            if let (Some(rp), Some(wp)) = (req_port, wrk_port) {
                if let Some(balancer) = g.cluster_balancers.get(&rp) {
                    balancer.workers.lock().unwrap().insert(pm_id, wp);
                    let _ = ipc_tx.send(json!({ "__pmr_cluster_ack": true }).to_string());
                } else {
                    let wmap = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
                    wmap.lock().unwrap().insert(pm_id, wp);
                    let shutdown = Arc::new(Notify::new());
                    g.cluster_balancers.insert(
                        rp,
                        ClusterBalancer {
                            workers: Arc::clone(&wmap),
                            shutdown: Arc::clone(&shutdown),
                        },
                    );
                    let ipc_tx_clone = ipc_tx.clone();
                    let god_bal = Arc::clone(&god);
                    tokio::spawn(async move {
                        match bind_cluster_listener(req_host.as_deref(), rp).await {
                            Ok(listener) => {
                                let _ = ipc_tx_clone
                                    .send(json!({ "__pmr_cluster_ack": true }).to_string());
                                let mut rr_idx: usize = 0;
                                loop {
                                    tokio::select! {
                                        _ = shutdown.notified() => {
                                            break;
                                        }
                                        accept_res = listener.accept() => {
                                            let Ok((mut inbound, _)) = accept_res else {
                                                tokio::time::sleep(Duration::from_millis(10)).await;
                                                continue;
                                            };
                                            let ports: Vec<u16> = {
                                                let guard = wmap.lock().unwrap();
                                                guard.values().copied().collect()
                                            };
                                            if ports.is_empty() {
                                                continue;
                                            }
                                            let start_idx = rr_idx;
                                            rr_idx = rr_idx.wrapping_add(1);
                                            tokio::spawn(async move {
                                                for offset in 0..ports.len() {
                                                    let twp = ports[(start_idx + offset) % ports.len()];
                                                    if let Ok(mut outbound) =
                                                        tokio::net::TcpStream::connect(("127.0.0.1", twp)).await
                                                    {
                                                        let _ = tokio::io::copy_bidirectional(
                                                            &mut inbound,
                                                            &mut outbound,
                                                        )
                                                        .await;
                                                        break;
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                {
                                    let mut g = god_bal.lock().await;
                                    g.cluster_balancers.remove(&rp);
                                }
                                let _ = ipc_tx_clone.send(
                                    json!({
                                        "__pmr_cluster_ack": false,
                                        "__pmr_cluster_err": e.to_string()
                                    })
                                    .to_string(),
                                );
                            }
                        }
                    });
                }
            }
        }
        "process:msg" => {
            let raw = msg.get("raw").cloned().unwrap_or(Value::Null);
            let mut marked_online = false;
            if raw.as_str() == Some("ready")
                || raw.get("data").and_then(|v| v.as_str()) == Some("ready")
            {
                let was_launching = proc.pm2_env.status == LAUNCHING_STATUS;
                proc.pm2_env.status = ONLINE_STATUS.to_string();
                proc.pm2_env.sync_flattened_env();
                let _ = proc.ready_tx.send(true);
                marked_online = was_launching;
            }
            let mut extra_bus_event: Option<(String, Value)> = None;
            if let Some(obj) = raw.as_object() {
                if let Some(t) = obj.get("type").and_then(|v| v.as_str()) {
                    match t {
                        "axm:monitor" => {
                            if let Some(data) = obj.get("data").and_then(|d| d.as_object()) {
                                for (k, v) in data {
                                    let metric = if let Some(mobj) = v.as_object() {
                                        AxmMetric {
                                            value: mobj
                                                .get("value")
                                                .cloned()
                                                .unwrap_or(Value::Null),
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
                                if let Some(aname) =
                                    data.get("action_name").and_then(|a| a.as_str())
                                {
                                    if !proc
                                        .pm2_env
                                        .axm_actions
                                        .iter()
                                        .any(|a| a.action_name == aname)
                                    {
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
                        "axm:option:configuration" => {
                            if let Some(data) = obj.get("data").and_then(|d| d.as_object()) {
                                if !proc.pm2_env.axm_options.is_object() {
                                    proc.pm2_env.axm_options = json!({});
                                }
                                if let Some(opts_obj) = proc.pm2_env.axm_options.as_object_mut() {
                                    for (k, v) in data {
                                        opts_obj.insert(k.clone(), v.clone());
                                    }
                                }
                            }
                        }
                        "axm:dynamic" => {
                            if let Some(data) = obj.get("data") {
                                proc.pm2_env.axm_dynamic = data.clone();
                            }
                        }
                        "axm:reply" | "process:exception" | "human:event" => {
                            extra_bus_event = Some((
                                t.to_string(),
                                obj.get("data").cloned().unwrap_or(raw.clone()),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            let name = proc.pm2_env.name.clone();
            let ns = proc.pm2_env.namespace.clone();
            if marked_online {
                g.log_pm2(&format!("App [{}:{}] online", name, pm_id));
                g.emit_bus_event(
                    "process:event",
                    json!(pm_id),
                    &name,
                    &ns,
                    json!({
                        "event": "online",
                        "manually": false,
                        "process": { "pm_id": pm_id, "name": name, "status": ONLINE_STATUS }
                    }),
                );
            }
            if let Some((ev_name, ev_data)) = extra_bus_event {
                g.emit_bus_event(&ev_name, json!(pm_id), &name, &ns, ev_data);
            }
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
