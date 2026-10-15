use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::config::{
    is_config_file, parse_config_file, parse_config_str, resolve_app_to_pm2_env, AppConfig,
    DeployEnvConfig, EcosystemConfig, ModuleConfiguration, StartOptions,
};
use crate::constants::ONLINE_STATUS;
use crate::daemon::run_daemon;
use crate::paths::PmrPaths;
use crate::process::ProcessDescription;
use crate::protocol::{BusEvent, RpcRequest, RpcResponse};

#[derive(Debug, Clone)]
pub struct PmrClient {
    pub paths: PmrPaths,
}

impl Default for PmrClient {
    fn default() -> Self {
        Self::new(None)
    }
}

impl PmrClient {
    pub fn new(custom_home: Option<&Path>) -> Self {
        let paths = PmrPaths::new(custom_home);
        let _ = paths.init_file_structure();
        Self { paths }
    }

    pub async fn ping_daemon(&self) -> bool {
        if !self.paths.daemon_rpc_port.exists() {
            return false;
        }
        matches!(
            tokio::time::timeout(
                Duration::from_secs(2),
                self.request_with_timeout(&RpcRequest::Ping, Duration::from_secs(2))
            )
            .await,
            Ok(Ok(r)) if r.ok
        )
    }

    pub async fn ensure_daemon(&self, no_daemon: bool) -> Result<bool> {
        self.paths.init_file_structure()?;
        if self.ping_daemon().await {
            return Ok(false);
        }

        if no_daemon {
            let paths_clone = self.paths.clone();
            tokio::spawn(async move {
                let _ = run_daemon(paths_clone).await;
            });
        } else {
            // Coordinate concurrent daemon startups with an exclusive lock file
            let start_lock_path = self.paths.pm2_home.join("daemon-start.lock");
            let lock_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&start_lock_path)?;

            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                unsafe {
                    libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX);
                }
            }

            if self.ping_daemon().await {
                return Ok(false);
            }

            let exe = PmrPaths::resolve_pmr_binary();
            let log_out = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.paths.pm2_log_file_path)?;
            let log_err = log_out.try_clone()?;

            let mut cmd = Command::new(exe);
            cmd.arg("__daemon")
                .env("PMR_HOME", &self.paths.pm2_home)
                .env("PM2_HOME", &self.paths.pm2_home)
                .stdin(Stdio::null())
                .stdout(Stdio::from(log_out))
                .stderr(Stdio::from(log_err));

            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                unsafe {
                    cmd.pre_exec(|| {
                        libc::setsid();
                        Ok(())
                    });
                }
            }

            cmd.spawn().context("Failed to spawn PMR background daemon")?;

            for _ in 0..100 {
                if self.ping_daemon().await {
                    return Ok(true);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            drop(lock_file);
            return Err(anyhow!(
                "Timed out waiting for PMR daemon at {}",
                self.paths.daemon_rpc_port.display()
            ));
        }

        for _ in 0..100 {
            if self.ping_daemon().await {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        Err(anyhow!(
            "Timed out waiting for PMR daemon at {}",
            self.paths.daemon_rpc_port.display()
        ))
    }

    pub async fn request(&self, req: &RpcRequest) -> Result<RpcResponse> {
        self.request_with_timeout(req, Duration::from_secs(60)).await
    }

    pub async fn request_with_timeout(
        &self,
        req: &RpcRequest,
        timeout_dur: Duration,
    ) -> Result<RpcResponse> {
        tokio::time::timeout(timeout_dur, async {
            let stream = UnixStream::connect(&self.paths.daemon_rpc_port)
                .await
                .with_context(|| {
                    format!(
                        "Failed to connect to PMR RPC socket {}",
                        self.paths.daemon_rpc_port.display()
                    )
                })?;
            let (reader, mut writer) = stream.into_split();
            let payload = serde_json::to_string(req)? + "\n";
            writer.write_all(payload.as_bytes()).await?;
            writer.flush().await?;

            let mut lines = BufReader::new(reader).lines();
            let line = lines
                .next_line()
                .await?
                .ok_or_else(|| anyhow!("Daemon closed RPC connection without response"))?;
            let resp: RpcResponse = serde_json::from_str(&line)?;
            Ok(resp)
        })
        .await
        .map_err(|_| anyhow!("Timed out waiting for RPC response from PMR daemon"))?
    }

    pub async fn subscribe_bus(&self) -> Result<mpsc::UnboundedReceiver<BusEvent>> {
        let stream = UnixStream::connect(&self.paths.daemon_pub_port)
            .await
            .with_context(|| {
                format!(
                    "Failed to connect to PMR Pub socket {}",
                    self.paths.daemon_pub_port.display()
                )
            })?;
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(ev) = serde_json::from_str::<BusEvent>(&line) {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }

    pub async fn list(&self) -> Result<Vec<ProcessDescription>> {
        self.request(&RpcRequest::GetMonitorData)
            .await?
            .into_processes()
    }

    pub async fn resolve_target_ids(&self, target: &str) -> Result<Vec<u32>> {
        self.resolve_target_ids_filtered(target, true).await
    }

    /// Resolve process IDs matching PM2's selector precedence:
    /// 1. `"all"` -> all processes (excluding `pmx_module` when `include_modules_in_all` is false)
    /// 2. `"/regex/"` -> regex match on `name`
    /// 3. Exact `name` match
    /// 4. Exact `namespace` match
    /// 5. Numeric `pm_id` match
    /// 6. Exact `pm_exec_path` match
    pub async fn resolve_target_ids_filtered(
        &self,
        target: &str,
        include_modules_in_all: bool,
    ) -> Result<Vec<u32>> {
        let procs = self.list().await?;
        if target == "all" {
            return Ok(procs
                .iter()
                .filter(|p| include_modules_in_all || !p.pm2_env.pmx_module)
                .map(|p| p.pm_id)
                .collect());
        }
        if target.starts_with('/') && target.ends_with('/') && target.len() > 2 {
            let pattern = &target[1..target.len() - 1];
            let re = Regex::new(pattern)
                .map_err(|e| anyhow!("Invalid regex selector '{}': {}", target, e))?;
            let ids: Vec<u32> = procs
                .iter()
                .filter(|p| re.is_match(&p.name))
                .map(|p| p.pm_id)
                .collect();
            return Ok(ids);
        }

        // 1. Check exact application name first (even if numeric-looking)
        let by_name: Vec<u32> = procs
            .iter()
            .filter(|p| p.name == target)
            .map(|p| p.pm_id)
            .collect();
        if !by_name.is_empty() {
            return Ok(by_name);
        }

        // 2. Check exact namespace
        let by_ns: Vec<u32> = procs
            .iter()
            .filter(|p| p.pm2_env.namespace == target)
            .map(|p| p.pm_id)
            .collect();
        if !by_ns.is_empty() {
            return Ok(by_ns);
        }

        // 3. Check numeric pm_id
        if let Ok(id) = target.parse::<u32>() {
            if procs.iter().any(|p| p.pm_id == id) {
                return Ok(vec![id]);
            }
        }

        // 4. Check script path
        let by_path: Vec<u32> = procs
            .iter()
            .filter(|p| p.pm2_env.pm_exec_path == target)
            .map(|p| p.pm_id)
            .collect();
        Ok(by_path)
    }

    fn load_ecosystem_target(target: &str) -> Result<Option<EcosystemConfig>> {
        if target == "-" {
            use std::io::Read;
            let mut raw = String::new();
            std::io::stdin().read_to_string(&mut raw)?;
            let eco = parse_config_str(&raw, None)?;
            return Ok(Some(eco));
        }
        let target_path = Path::new(target);
        if is_config_file(target).is_some() && target_path.exists() {
            let eco = parse_config_file(target_path)?;
            return Ok(Some(eco));
        }
        Ok(None)
    }

    fn filter_ecosystem_apps(apps: Vec<AppConfig>, opts: &StartOptions) -> Vec<AppConfig> {
        let Some(ref only_str) = opts.only else {
            return apps;
        };
        let allowed: Vec<String> = only_str
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .map(|p| p.to_string())
            .collect();
        apps.into_iter()
            .filter(|app| {
                let candidate_name = app.name.clone().unwrap_or_else(|| {
                    Path::new(&app.script)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                });
                allowed.contains(&candidate_name)
            })
            .collect()
    }

    pub fn build_update_env_map(&self, opts: &StartOptions) -> BTreeMap<String, String> {
        let mut env_map = BTreeMap::new();
        if opts.update_env {
            for (k, v) in env::vars() {
                if !opts.filter_env.is_empty()
                    && opts.filter_env.iter().any(|pat| !pat.is_empty() && k.contains(pat))
                {
                    continue;
                }
                env_map.insert(k, v);
            }
            if let Some(ref env_name) = opts.env_name {
                env_map.insert("NODE_ENV".to_string(), env_name.clone());
            }
        }
        for (k, v) in &opts.extra_env {
            env_map.insert(k.clone(), v.clone());
        }
        env_map
    }

    pub fn build_conf_patch(&self, opts: &StartOptions, app_name: Option<&str>) -> Option<Value> {
        let mut map = serde_json::Map::new();

        if let Some(ref name) = opts.name {
            map.insert("name".to_string(), json!(name));
        }
        if let Some(ref ns) = opts.namespace {
            map.insert("namespace".to_string(), json!(ns));
        }
        if let Some(ref cwd) = opts.cwd {
            map.insert("pm_cwd".to_string(), json!(cwd));
        }
        if !opts.args.is_empty() {
            map.insert("args".to_string(), json!(opts.args));
        }
        if let Some(ref interp) = opts.interpreter {
            map.insert("exec_interpreter".to_string(), json!(interp));
        }
        if !opts.node_args.is_empty() {
            map.insert("node_args".to_string(), json!(opts.node_args));
        }
        if let Some(ref out) = opts.output {
            map.insert("pm_out_log_path".to_string(), json!(out));
        }
        if let Some(ref err) = opts.error {
            map.insert("pm_err_log_path".to_string(), json!(err));
        }
        if let Some(ref log) = opts.log {
            map.insert("pm_log_path".to_string(), json!(log));
        }
        if let Some(ref pid) = opts.pid {
            map.insert("pm_pid_path".to_string(), json!(pid));
        }
        if opts.merge_logs {
            map.insert("merge_logs".to_string(), json!(true));
        }
        if opts.disable_logs {
            map.insert("disable_logs".to_string(), json!(true));
        }
        if let Some(ref lt) = opts.log_type {
            map.insert("log_type".to_string(), json!(lt));
        }
        if opts.time {
            map.insert(
                "log_date_format".to_string(),
                json!(crate::constants::DEFAULT_LOG_DATE_FORMAT),
            );
        } else if let Some(ref ldf) = opts.log_date_format {
            map.insert("log_date_format".to_string(), json!(ldf));
        }
        if let Some(mmr) = opts.max_memory_restart {
            map.insert("max_memory_restart".to_string(), json!(mmr));
        }
        if let Some(rd) = opts.restart_delay {
            map.insert("restart_delay".to_string(), json!(rd));
        }
        if let Some(ebrd) = opts.exp_backoff_restart_delay {
            map.insert("exp_backoff_restart_delay".to_string(), json!(ebrd));
            map.insert("prev_restart_delay".to_string(), json!(ebrd));
        }
        if let Some(mr) = opts.max_restarts {
            map.insert("max_restarts".to_string(), json!(mr));
        }
        if let Some(mu) = opts.min_uptime {
            map.insert("min_uptime".to_string(), json!(mu));
        }
        if let Some(ref cr) = opts.cron_restart {
            map.insert("cron_restart".to_string(), json!(cr));
        }
        if opts.watch {
            map.insert("watch".to_string(), json!(true));
        }
        if !opts.ignore_watch.is_empty() {
            map.insert("ignore_watch".to_string(), json!(opts.ignore_watch));
        }
        if let Some(wd) = opts.watch_delay {
            map.insert("watch_delay".to_string(), json!(wd));
        }
        if let Some(kt) = opts.kill_timeout {
            map.insert("kill_timeout".to_string(), json!(kt));
        }
        if let Some(lt) = opts.listen_timeout {
            map.insert("listen_timeout".to_string(), json!(lt));
        }
        if opts.wait_ready {
            map.insert("wait_ready".to_string(), json!(true));
        }
        if opts.shutdown_with_message {
            map.insert("shutdown_with_message".to_string(), json!(true));
        }
        if opts.no_treekill {
            map.insert("treekill".to_string(), json!(false));
        }
        if opts.no_autorestart {
            map.insert("autorestart".to_string(), json!(false));
        }
        if opts.no_autostart {
            map.insert("autostart".to_string(), json!(false));
        }
        if opts.no_vizion {
            map.insert("vizion".to_string(), json!(false));
        }
        if opts.no_pmx {
            map.insert("pmx".to_string(), json!(false));
            map.insert("automation".to_string(), json!(false));
        }
        if !opts.stop_exit_codes.is_empty() {
            map.insert("stop_exit_codes".to_string(), json!(opts.stop_exit_codes));
        }
        if let Some(uid) = opts.uid {
            map.insert("uid".to_string(), json!(uid));
        }
        if let Some(gid) = opts.gid {
            map.insert("gid".to_string(), json!(gid));
        }

        // Inject updated module_conf if an app_name is known
        if let Some(name) = app_name {
            let all_conf = ModuleConfiguration::get_all_sync(&self.paths);
            if let Some(mod_val) = all_conf.get(name) {
                map.insert(name.to_string(), mod_val.clone());
                if let Some(mod_obj) = mod_val.as_object() {
                    let mut env_patch = serde_json::Map::new();
                    for (k, v) in mod_obj {
                        let s = v
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| v.to_string());
                        env_patch.insert(k.clone(), Value::String(s));
                    }
                    map.insert("env".to_string(), Value::Object(env_patch));
                }
            }
        }

        if map.is_empty() {
            None
        } else {
            Some(Value::Object(map))
        }
    }

    /// Start or restart/reload from an ecosystem file, stdin JSON, existing process name/id, or script path.
    /// If `reload_mode` is `Some(false)`, acts like `startOrRestart` (restarts online apps);
    /// if `Some(true)`, acts like `startOrReload` (reloads online apps).
    pub async fn start_target(
        &self,
        target: &str,
        opts: &StartOptions,
        reload_mode: Option<bool>,
    ) -> Result<Vec<ProcessDescription>> {
        // 1. Check if target is "-" (stdin) or a config file
        if let Some(eco) = Self::load_ecosystem_target(target)? {
            // In PM2, `pm2 start ecosystem.config.js` invokes `_startJson` with `restartProcessId`,
            // restarting existing apps from the ecosystem config!
            let effective_mode = reload_mode.or(Some(false));
            return self.start_ecosystem(eco, opts, effective_mode).await;
        }

        // 2. Check if target refers to an already registered process ID or name (when file does not exist)
        let target_path = Path::new(target);
        if !target_path.exists() {
            let existing_ids = self
                .resolve_target_ids_filtered(target, true)
                .await
                .unwrap_or_default();
            if !existing_ids.is_empty() {
                let procs = self.list().await?;
                let env_map = self.build_update_env_map(opts);
                for id in existing_ids {
                    let proc_name = procs
                        .iter()
                        .find(|p| p.pm_id == id)
                        .map(|p| p.name.as_str());
                    let conf = self.build_conf_patch(opts, proc_name);
                    if opts.watch {
                        let _ = self
                            .request(&RpcRequest::ToggleWatch {
                                target: id.to_string(),
                            })
                            .await;
                    }
                    let resp = self
                        .request(&RpcRequest::RestartProcessId {
                            id,
                            env: env_map.clone(),
                            conf,
                        })
                        .await?;
                    if !resp.ok {
                        anyhow::bail!(
                            "{}",
                            resp.error.unwrap_or_else(|| "Restart failed".to_string())
                        );
                    }
                }
                return self.list().await;
            }
        }

        // 3. Otherwise treat `target` as a script/binary/command
        let app = AppConfig {
            script: target.to_string(),
            ..Default::default()
        };
        self.start_app_config(app, opts, &BTreeMap::new(), reload_mode)
            .await
    }

    pub async fn start_ecosystem(
        &self,
        eco: EcosystemConfig,
        opts: &StartOptions,
        reload_mode: Option<bool>,
    ) -> Result<Vec<ProcessDescription>> {
        let filtered_apps = Self::filter_ecosystem_apps(eco.apps, opts);
        let mut errors = Vec::new();
        for app in filtered_apps {
            if let Err(e) = self
                .start_app_config_inner(app, opts, &eco.deploy, reload_mode, true)
                .await
            {
                errors.push(e.to_string());
            }
        }
        if !errors.is_empty() {
            anyhow::bail!("{}", errors.join("; "));
        }
        self.list().await
    }

    pub async fn start_app_config(
        &self,
        app: AppConfig,
        opts: &StartOptions,
        deploy_conf: &BTreeMap<String, DeployEnvConfig>,
        reload_mode: Option<bool>,
    ) -> Result<Vec<ProcessDescription>> {
        let is_internal_or_eco =
            app.script == "__pmr_serve__" || app.script == "__pmr_web__" || reload_mode.is_some();
        self.start_app_config_inner(app, opts, deploy_conf, reload_mode, is_internal_or_eco)
            .await
    }

    async fn start_app_config_inner(
        &self,
        app: AppConfig,
        opts: &StartOptions,
        deploy_conf: &BTreeMap<String, DeployEnvConfig>,
        reload_mode: Option<bool>,
        from_ecosystem: bool,
    ) -> Result<Vec<ProcessDescription>> {
        let force = opts.force || app.force.unwrap_or(false);
        let (pm2_env, instances) = resolve_app_to_pm2_env(app, opts, deploy_conf, &self.paths)?;

        if opts.write {
            let file_name = format!("{}-pm2.json", pm2_env.name);
            let out_path = env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(file_name);
            let _ = fs::write(&out_path, serde_json::to_string_pretty(&pm2_env)?);
        }

        let existing = self.list().await?;
        let same_app_ids: Vec<(u32, String)> = existing
            .iter()
            .filter(|p| {
                if from_ecosystem {
                    p.name == pm2_env.name
                } else {
                    p.pm2_env.pm_exec_path == pm2_env.pm_exec_path && p.name == pm2_env.name
                }
            })
            .map(|p| (p.pm_id, p.pm2_env.status.clone()))
            .collect();

        if !same_app_ids.is_empty() && !force {
            let conf_val = serde_json::to_value(&pm2_env).ok();
            let mut env_update = pm2_env.env.clone();
            for (k, v) in self.build_update_env_map(opts) {
                env_update.insert(k, v);
            }

            for (id, status) in same_app_ids {
                match reload_mode {
                    Some(true) => {
                        let req = if status == ONLINE_STATUS {
                            RpcRequest::ReloadProcessId {
                                id,
                                env: env_update.clone(),
                                conf: conf_val.clone(),
                                soft: false,
                            }
                        } else {
                            RpcRequest::RestartProcessId {
                                id,
                                env: env_update.clone(),
                                conf: conf_val.clone(),
                            }
                        };
                        let resp = self.request(&req).await?;
                        if !resp.ok {
                            anyhow::bail!("{}", resp.error.unwrap_or_default());
                        }
                    }
                    Some(false) => {
                        let resp = self
                            .request(&RpcRequest::RestartProcessId {
                                id,
                                env: env_update.clone(),
                                conf: conf_val.clone(),
                            })
                            .await?;
                        if !resp.ok {
                            anyhow::bail!("{}", resp.error.unwrap_or_default());
                        }
                    }
                    None => {
                        if status != ONLINE_STATUS || opts.update_env || from_ecosystem {
                            let resp = self
                                .request(&RpcRequest::RestartProcessId {
                                    id,
                                    env: env_update.clone(),
                                    conf: conf_val.clone(),
                                })
                                .await?;
                            if !resp.ok {
                                anyhow::bail!("{}", resp.error.unwrap_or_default());
                            }
                        } else {
                            anyhow::bail!(
                                "Script already launched, add -f option to force re-execution"
                            );
                        }
                    }
                }
            }
            return self.list().await;
        }

        let resp = self
            .request(&RpcRequest::Prepare {
                env: Box::new(pm2_env),
                instances,
            })
            .await?;
        if !resp.ok {
            anyhow::bail!(
                "{}",
                resp.error
                    .unwrap_or_else(|| "Failed to start app".to_string())
            );
        }
        self.list().await
    }

    pub async fn stop(&self, target: &str, opts: &StartOptions) -> Result<Vec<ProcessDescription>> {
        if let Some(eco) = Self::load_ecosystem_target(target)? {
            let apps = Self::filter_ecosystem_apps(eco.apps, opts);
            for app in apps {
                let name = app.name.unwrap_or_else(|| {
                    Path::new(&app.script)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                });
                let ids = self.resolve_target_ids_filtered(&name, false).await?;
                for id in ids {
                    let _ = self
                        .request(&RpcRequest::StopWatch {
                            target: id.to_string(),
                        })
                        .await;
                    let _ = self.request(&RpcRequest::StopProcessId { id }).await?;
                }
            }
            return self.list().await;
        }

        let ids = self.resolve_target_ids_filtered(target, false).await?;
        if ids.is_empty() && target != "all" {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        for id in ids {
            let _ = self
                .request(&RpcRequest::StopWatch {
                    target: id.to_string(),
                })
                .await;
            let resp = self.request(&RpcRequest::StopProcessId { id }).await?;
            if !resp.ok {
                anyhow::bail!("{}", resp.error.unwrap_or_default());
            }
        }
        self.list().await
    }

    pub async fn restart(
        &self,
        target: &str,
        opts: &StartOptions,
    ) -> Result<Vec<ProcessDescription>> {
        if let Some(eco) = Self::load_ecosystem_target(target)? {
            return self.start_ecosystem(eco, opts, Some(false)).await;
        }
        let ids = self.resolve_target_ids_filtered(target, false).await?;
        if ids.is_empty() && target != "all" {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        let procs = self.list().await?;
        let env_map = self.build_update_env_map(opts);
        for id in ids {
            let proc_name = procs
                .iter()
                .find(|p| p.pm_id == id)
                .map(|p| p.name.as_str());
            let conf = self.build_conf_patch(opts, proc_name);
            if opts.watch {
                let _ = self
                    .request(&RpcRequest::ToggleWatch {
                        target: id.to_string(),
                    })
                    .await;
            }
            let resp = self
                .request(&RpcRequest::RestartProcessId {
                    id,
                    env: env_map.clone(),
                    conf,
                })
                .await?;
            if !resp.ok {
                anyhow::bail!("{}", resp.error.unwrap_or_default());
            }
        }
        self.list().await
    }

    pub async fn reload(
        &self,
        target: &str,
        opts: &StartOptions,
        soft: bool,
    ) -> Result<Vec<ProcessDescription>> {
        if let Some(eco) = Self::load_ecosystem_target(target)? {
            return self.start_ecosystem(eco, opts, Some(true)).await;
        }
        let ids = self.resolve_target_ids_filtered(target, false).await?;
        if ids.is_empty() && target != "all" {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        let procs = self.list().await?;
        let env_map = self.build_update_env_map(opts);
        for id in ids {
            let proc_name = procs
                .iter()
                .find(|p| p.pm_id == id)
                .map(|p| p.name.as_str());
            let conf = self.build_conf_patch(opts, proc_name);
            let resp = self
                .request(&RpcRequest::ReloadProcessId {
                    id,
                    env: env_map.clone(),
                    conf,
                    soft,
                })
                .await?;
            if !resp.ok {
                anyhow::bail!("{}", resp.error.unwrap_or_default());
            }
        }
        self.list().await
    }

    pub async fn delete(
        &self,
        target: &str,
        opts: &StartOptions,
    ) -> Result<Vec<ProcessDescription>> {
        if let Some(eco) = Self::load_ecosystem_target(target)? {
            let apps = Self::filter_ecosystem_apps(eco.apps, opts);
            for app in apps {
                let name = app.name.unwrap_or_else(|| {
                    Path::new(&app.script)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                });
                let ids = self.resolve_target_ids_filtered(&name, false).await?;
                for id in ids {
                    let _ = self
                        .request(&RpcRequest::StopWatch {
                            target: id.to_string(),
                        })
                        .await;
                    let _ = self.request(&RpcRequest::DeleteProcessId { id }).await?;
                }
            }
            return self.list().await;
        }

        let ids = self.resolve_target_ids_filtered(target, false).await?;
        if ids.is_empty() && target != "all" {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        for id in ids {
            let _ = self
                .request(&RpcRequest::StopWatch {
                    target: id.to_string(),
                })
                .await;
            let resp = self.request(&RpcRequest::DeleteProcessId { id }).await?;
            if !resp.ok {
                anyhow::bail!("{}", resp.error.unwrap_or_default());
            }
        }
        self.list().await
    }

    pub async fn scale(&self, name: &str, number: &str) -> Result<Vec<ProcessDescription>> {
        self.request(&RpcRequest::Scale {
            name: name.to_string(),
            number: number.to_string(),
        })
        .await?
        .into_processes()
    }

    pub async fn reset(&self, target: &str) -> Result<Vec<ProcessDescription>> {
        let ids = self.resolve_target_ids_filtered(target, true).await?;
        if ids.is_empty() && target != "all" {
            anyhow::bail!("Process {} not found", target);
        }
        for id in ids {
            let _ = self.request(&RpcRequest::ResetMetaProcessId { id }).await?;
        }
        self.list().await
    }

    pub async fn kill_daemon(&self) -> Result<()> {
        if !self.ping_daemon().await {
            return Ok(());
        }
        let _ = self
            .request(&RpcRequest::KillMe {
                pid: Some(std::process::id()),
            })
            .await;
        for _ in 0..40 {
            if !self.ping_daemon().await && !self.paths.daemon_rpc_port.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
}
