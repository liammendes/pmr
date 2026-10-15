use anyhow::{anyhow, Context, Result};
use regex::Regex;
use std::collections::BTreeMap;
use std::env;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::config::{
    is_config_file, parse_config_file, parse_config_str, resolve_app_to_pm2_env, AppConfig,
    DeployEnvConfig, StartOptions,
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
        matches!(self.request(&RpcRequest::Ping).await, Ok(r) if r.ok)
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
            let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("pmr"));
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
        let procs = self.list().await?;
        if target == "all" {
            return Ok(procs.iter().map(|p| p.pm_id).collect());
        }
        if let Ok(id) = target.parse::<u32>() {
            if procs.iter().any(|p| p.pm_id == id) {
                return Ok(vec![id]);
            }
        }
        if target.starts_with('/') && target.ends_with('/') && target.len() > 2 {
            let pattern = &target[1..target.len() - 1];
            if let Ok(re) = Regex::new(pattern) {
                let ids: Vec<u32> = procs
                    .iter()
                    .filter(|p| re.is_match(&p.name))
                    .map(|p| p.pm_id)
                    .collect();
                if !ids.is_empty() {
                    return Ok(ids);
                }
            }
        }

        let ids: Vec<u32> = procs
            .iter()
            .filter(|p| {
                p.name == target
                    || p.pm2_env.namespace == target
                    || p.pm2_env.pm_exec_path == target
            })
            .map(|p| p.pm_id)
            .collect();
        Ok(ids)
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
        // 1. Check if target is "-" (stdin)
        if target == "-" {
            use std::io::Read;
            let mut raw = String::new();
            std::io::stdin().read_to_string(&mut raw)?;
            let eco = parse_config_str(&raw, None)?;
            return self.start_ecosystem(eco, opts, reload_mode).await;
        }

        // 2. Check if target is a known config file
        let target_path = Path::new(target);
        if is_config_file(target).is_some() && target_path.exists() {
            let eco = parse_config_file(target_path)?;
            return self.start_ecosystem(eco, opts, reload_mode).await;
        }

        // 3. Check if target refers to an already registered process ID or name (when file does not exist)
        if !target_path.exists() {
            let existing_ids = self.resolve_target_ids(target).await.unwrap_or_default();
            if !existing_ids.is_empty() {
                for id in existing_ids {
                    let resp = self
                        .request(&RpcRequest::RestartProcessId {
                            id,
                            env: opts.extra_env.clone(),
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

        // 4. Otherwise treat `target` as a script/binary/command
        let app = AppConfig {
            script: target.to_string(),
            ..Default::default()
        };
        self.start_app_config(app, opts, &BTreeMap::new(), reload_mode)
            .await
    }

    pub async fn start_ecosystem(
        &self,
        eco: crate::config::EcosystemConfig,
        opts: &StartOptions,
        reload_mode: Option<bool>,
    ) -> Result<Vec<ProcessDescription>> {
        let only_filter: Option<Vec<String>> = opts.only.as_ref().map(|s| {
            s.split(|c: char| c == ',' || c.is_whitespace())
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string())
                .collect()
        });

        for app in eco.apps {
            if let Some(ref allowed) = only_filter {
                let candidate_name = app.name.clone().unwrap_or_else(|| {
                    Path::new(&app.script)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                });
                if !allowed.contains(&candidate_name) {
                    continue;
                }
            }
            self.start_app_config(app, opts, &eco.deploy, reload_mode)
                .await?;
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
        let force = opts.force || app.force.unwrap_or(false);
        let (pm2_env, instances) = resolve_app_to_pm2_env(app, opts, deploy_conf, &self.paths)?;

        // Check if app with same name already exists
        let existing = self.list().await?;
        let same_name_ids: Vec<(u32, String)> = existing
            .iter()
            .filter(|p| p.name == pm2_env.name)
            .map(|p| (p.pm_id, p.pm2_env.status.clone()))
            .collect();

        if !same_name_ids.is_empty() && !force {
            for (id, status) in same_name_ids {
                match reload_mode {
                    Some(true) => {
                        let resp = self
                            .request(&RpcRequest::ReloadProcessId {
                                id,
                                env: pm2_env.env.clone(),
                                soft: false,
                            })
                            .await?;
                        if !resp.ok {
                            anyhow::bail!("{}", resp.error.unwrap_or_default());
                        }
                    }
                    Some(false) => {
                        let resp = self
                            .request(&RpcRequest::RestartProcessId {
                                id,
                                env: pm2_env.env.clone(),
                            })
                            .await?;
                        if !resp.ok {
                            anyhow::bail!("{}", resp.error.unwrap_or_default());
                        }
                    }
                    None => {
                        if status != ONLINE_STATUS {
                            let resp = self
                                .request(&RpcRequest::RestartProcessId {
                                    id,
                                    env: pm2_env.env.clone(),
                                })
                                .await?;
                            if !resp.ok {
                                anyhow::bail!("{}", resp.error.unwrap_or_default());
                            }
                        } else if opts.update_env {
                            let resp = self
                                .request(&RpcRequest::RestartProcessId {
                                    id,
                                    env: pm2_env.env.clone(),
                                })
                                .await?;
                            if !resp.ok {
                                anyhow::bail!("{}", resp.error.unwrap_or_default());
                            }
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
            anyhow::bail!("{}", resp.error.unwrap_or_else(|| "Failed to start app".to_string()));
        }
        self.list().await
    }

    pub async fn stop(&self, target: &str) -> Result<Vec<ProcessDescription>> {
        if is_config_file(target).is_some() && Path::new(target).exists() {
            let eco = parse_config_file(Path::new(target))?;
            for app in eco.apps {
                let name = app.name.unwrap_or_else(|| {
                    Path::new(&app.script)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                });
                let ids = self.resolve_target_ids(&name).await?;
                for id in ids {
                    let _ = self.request(&RpcRequest::StopProcessId { id }).await?;
                }
            }
            return self.list().await;
        }

        let ids = self.resolve_target_ids(target).await?;
        if ids.is_empty() {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        for id in ids {
            let resp = self.request(&RpcRequest::StopProcessId { id }).await?;
            if !resp.ok {
                anyhow::bail!("{}", resp.error.unwrap_or_default());
            }
        }
        self.list().await
    }

    pub async fn restart(&self, target: &str, opts: &StartOptions) -> Result<Vec<ProcessDescription>> {
        if is_config_file(target).is_some() && Path::new(target).exists() {
            let eco = parse_config_file(Path::new(target))?;
            return self.start_ecosystem(eco, opts, Some(false)).await;
        }
        let ids = self.resolve_target_ids(target).await?;
        if ids.is_empty() {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        for id in ids {
            let resp = self
                .request(&RpcRequest::RestartProcessId {
                    id,
                    env: opts.extra_env.clone(),
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
        if is_config_file(target).is_some() && Path::new(target).exists() {
            let eco = parse_config_file(Path::new(target))?;
            return self.start_ecosystem(eco, opts, Some(true)).await;
        }
        let ids = self.resolve_target_ids(target).await?;
        if ids.is_empty() {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        for id in ids {
            let resp = self
                .request(&RpcRequest::ReloadProcessId {
                    id,
                    env: opts.extra_env.clone(),
                    soft,
                })
                .await?;
            if !resp.ok {
                anyhow::bail!("{}", resp.error.unwrap_or_default());
            }
        }
        self.list().await
    }

    pub async fn delete(&self, target: &str) -> Result<Vec<ProcessDescription>> {
        if is_config_file(target).is_some() && Path::new(target).exists() {
            let eco = parse_config_file(Path::new(target))?;
            for app in eco.apps {
                let name = app.name.unwrap_or_else(|| {
                    Path::new(&app.script)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                });
                let ids = self.resolve_target_ids(&name).await?;
                for id in ids {
                    let _ = self.request(&RpcRequest::DeleteProcessId { id }).await?;
                }
            }
            return self.list().await;
        }

        let ids = self.resolve_target_ids(target).await?;
        if ids.is_empty() && target != "all" {
            anyhow::bail!("Process or Namespace {} not found", target);
        }
        for id in ids {
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
        let ids = self.resolve_target_ids(target).await?;
        if ids.is_empty() {
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
        Ok(())
    }
}
