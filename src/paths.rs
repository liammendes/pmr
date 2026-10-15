use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::constants::NODE_PRELOAD_SHIM_JS;

#[derive(Debug, Clone)]
pub struct PmrPaths {
    pub pm2_home: PathBuf,
    pub pm2_conf_file: PathBuf,
    pub pm2_module_conf_file: PathBuf,
    pub pm2_log_file_path: PathBuf,
    pub pm2_pid_file_path: PathBuf,
    pub pm2_reload_lockfile: PathBuf,
    pub default_pid_path: PathBuf,
    pub default_log_path: PathBuf,
    pub default_module_path: PathBuf,
    pub pm2_io_access_token: PathBuf,
    pub dump_file_path: PathBuf,
    pub dump_backup_file_path: PathBuf,
    pub daemon_rpc_port: PathBuf,
    pub daemon_pub_port: PathBuf,
    pub interactor_rpc_port: PathBuf,
    pub interactor_log_file_path: PathBuf,
    pub interactor_pid_path: PathBuf,
    pub interaction_conf: PathBuf,
    pub node_ipc_sock: PathBuf,
    pub node_preload_shim_path: PathBuf,
}

impl Default for PmrPaths {
    fn default() -> Self {
        Self::new(None)
    }
}

impl PmrPaths {
    pub fn default_home() -> PathBuf {
        if let Ok(val) = env::var("PMR_HOME") {
            if !val.trim().is_empty() {
                return PathBuf::from(val);
            }
        }
        if let Ok(val) = env::var("PM2_HOME") {
            if !val.trim().is_empty() {
                return PathBuf::from(val);
            }
        }
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(".pmr");
        }
        if let Ok(homepath) = env::var("HOMEPATH") {
            let drive = env::var("HOMEDRIVE").unwrap_or_default();
            return PathBuf::from(format!("{}{}", drive, homepath)).join(".pmr");
        }
        PathBuf::from("/etc/.pmr")
    }

    pub fn new(custom_home: Option<&Path>) -> Self {
        let pm2_home = match custom_home {
            Some(p) => p.to_path_buf(),
            None => Self::default_home(),
        };

        let resolve_env_or = |env_key: &str, default_name: &str| -> PathBuf {
            if let Ok(v) = env::var(env_key) {
                if !v.trim().is_empty() {
                    return PathBuf::from(v);
                }
            }
            pm2_home.join(default_name)
        };

        Self {
            pm2_conf_file: resolve_env_or("PM2_CONF_FILE", "conf.js"),
            pm2_module_conf_file: resolve_env_or("PM2_MODULE_CONF_FILE", "module_conf.json"),
            pm2_log_file_path: resolve_env_or("PM2_LOG_FILE_PATH", "pm2.log"),
            pm2_pid_file_path: resolve_env_or("PM2_PID_FILE_PATH", "pm2.pid"),
            pm2_reload_lockfile: resolve_env_or("PM2_RELOAD_LOCKFILE", "reload.lock"),
            default_pid_path: resolve_env_or("PM2_DEFAULT_PID_PATH", "pids"),
            default_log_path: resolve_env_or("PM2_DEFAULT_LOG_PATH", "logs"),
            default_module_path: resolve_env_or("PM2_DEFAULT_MODULE_PATH", "modules"),
            pm2_io_access_token: resolve_env_or("PM2_IO_ACCESS_TOKEN", "pm2-io-token"),
            dump_file_path: resolve_env_or("PM2_DUMP_FILE_PATH", "dump.pm2"),
            dump_backup_file_path: resolve_env_or("PM2_DUMP_BACKUP_FILE_PATH", "dump.pm2.bak"),
            daemon_rpc_port: resolve_env_or("PM2_DAEMON_RPC_PORT", "rpc.sock"),
            daemon_pub_port: resolve_env_or("PM2_DAEMON_PUB_PORT", "pub.sock"),
            interactor_rpc_port: resolve_env_or("PM2_INTERACTOR_RPC_PORT", "interactor.sock"),
            interactor_log_file_path: resolve_env_or("PM2_INTERACTOR_LOG_FILE_PATH", "agent.log"),
            interactor_pid_path: resolve_env_or("PM2_INTERACTOR_PID_PATH", "agent.pid"),
            interaction_conf: resolve_env_or("PM2_INTERACTION_CONF", "agent.json5"),
            node_ipc_sock: pm2_home.join("node_ipc.sock"),
            node_preload_shim_path: pm2_home.join("pmr-preload.cjs"),
            pm2_home,
        }
    }

    pub fn init_file_structure(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.pm2_home)?;
        fs::create_dir_all(&self.default_log_path)?;
        fs::create_dir_all(&self.default_pid_path)?;
        fs::create_dir_all(&self.default_module_path)?;

        if !self.pm2_module_conf_file.exists() {
            let _ = fs::write(&self.pm2_module_conf_file, "{}\n");
        }

        let touch_path = self.pm2_home.join("touch");
        if !touch_path.exists() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = fs::write(touch_path, now.to_string());
        }

        let _ = fs::write(&self.node_preload_shim_path, NODE_PRELOAD_SHIM_JS);
        Ok(())
    }

    pub fn format_app_name(name: &str) -> String {
        name.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    }
}
