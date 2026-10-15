use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::constants::{
    DEFAULT_MAX_RESTARTS, DEFAULT_MIN_UPTIME_MS, FORK_MODE_ID, GRACEFUL_LISTEN_TIMEOUT_MS,
    KILL_RETRY_TIME_MS, KILL_TIMEOUT_MS, ONLINE_STATUS, PMR_VERSION,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Monit {
    pub memory: u64,
    pub cpu: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersioningInfo {
    #[serde(rename = "type")]
    pub vcs_type: String,
    pub url: String,
    pub revision: String,
    pub comment: String,
    pub branch: String,
    pub repo_path: String,
    pub update_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxmAction {
    pub action_name: String,
    #[serde(default)]
    pub action_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxmMetric {
    pub value: serde_json::Value,
    #[serde(default)]
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pm2Env {
    #[serde(default)]
    pub pm_id: u32,
    pub name: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub unique_id: String,
    #[serde(default = "default_status")]
    pub status: String,

    #[serde(default)]
    pub pm_uptime: u64,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub restart_time: u32,
    #[serde(default)]
    pub unstable_restarts: u32,
    #[serde(default)]
    pub prev_restart_delay: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,

    #[serde(default = "default_min_uptime")]
    pub min_uptime: u64,
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
    #[serde(default)]
    pub restart_delay: u64,
    #[serde(default)]
    pub exp_backoff_restart_delay: u64,
    #[serde(default = "default_true")]
    pub autorestart: bool,
    #[serde(default = "default_true")]
    pub autostart: bool,
    #[serde(default)]
    pub stop_exit_codes: Vec<i32>,

    pub pm_exec_path: String,
    pub pm_cwd: String,
    #[serde(default = "default_interpreter")]
    pub exec_interpreter: String,
    #[serde(default = "default_exec_mode")]
    pub exec_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instances: Option<usize>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub node_args: Vec<String>,
    #[serde(default = "default_instance_var")]
    pub instance_var: String,
    #[serde(rename = "NODE_APP_INSTANCE", skip_serializing_if = "Option::is_none")]
    pub node_app_instance: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub increment_var: Option<String>,

    pub pm_out_log_path: String,
    pub pm_err_log_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pm_log_path: Option<String>,
    pub pm_pid_path: String,
    #[serde(default)]
    pub merge_logs: bool,
    #[serde(default)]
    pub disable_logs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_date_format: Option<String>,

    #[serde(default = "default_kill_timeout")]
    pub kill_timeout: u64,
    #[serde(default = "default_kill_retry_time")]
    pub kill_retry_time: u64,
    #[serde(default = "default_listen_timeout")]
    pub listen_timeout: u64,
    #[serde(default)]
    pub wait_ready: bool,
    #[serde(default)]
    pub shutdown_with_message: bool,
    #[serde(default = "default_true")]
    pub treekill: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_memory_restart: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_restart: Option<String>,

    #[serde(default)]
    pub watch: bool,
    #[serde(default)]
    pub watch_paths: Vec<String>,
    #[serde(default)]
    pub ignore_watch: Vec<String>,
    #[serde(default)]
    pub watch_delay: u64,

    #[serde(default = "default_version")]
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_version: Option<String>,
    #[serde(default = "default_pmr_version")]
    pub _pm2_version: String,

    #[serde(default)]
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,

    #[serde(default = "default_true")]
    pub vizion: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versioning: Option<VersioningInfo>,
    #[serde(default = "default_true")]
    pub pmx: bool,
    #[serde(default)]
    pub pmx_module: bool,
    #[serde(default)]
    pub _km_monitored: bool,

    #[serde(default)]
    pub axm_actions: Vec<AxmAction>,
    #[serde(default)]
    pub axm_monitor: BTreeMap<String, AxmMetric>,
    #[serde(default = "default_empty_obj")]
    pub axm_options: serde_json::Value,
    #[serde(default = "default_empty_obj")]
    pub axm_dynamic: serde_json::Value,

    #[serde(default)]
    pub env: BTreeMap<String, String>,

    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Pm2Env {
    /// Normalize a raw PM2 or PMR JSON dump entry into a valid `Pm2Env`.
    pub fn from_pm2_value(mut v: Value) -> anyhow::Result<Self> {
        let obj = v
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("dump entry is not a JSON object"))?;

        obj.entry("pm_id").or_insert_with(|| json!(0));
        if !obj.contains_key("name") {
            let fallback = obj
                .get("pm_exec_path")
                .and_then(|s| s.as_str())
                .and_then(|p| std::path::Path::new(p).file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("app")
                .to_string();
            obj.insert("name".to_string(), json!(fallback));
        }

        if let Some(Value::Object(env_map)) = obj.get_mut("env") {
            env_map.retain(|_, val| !val.is_null());
            for val in env_map.values_mut() {
                if !val.is_string() {
                    let s = match val {
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        ref other => other.to_string(),
                    };
                    *val = Value::String(s);
                }
            }
        } else {
            obj.insert("env".to_string(), json!({}));
        }

        // Handle PM2 watch: ["src"] or watch: "src"
        match obj.get("watch").cloned() {
            Some(Value::Array(arr)) => {
                let paths: Vec<String> = arr
                    .into_iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
                obj.insert("watch_paths".to_string(), json!(paths));
                obj.insert("watch".to_string(), json!(true));
            }
            Some(Value::String(s)) => {
                obj.insert("watch_paths".to_string(), json!([s]));
                obj.insert("watch".to_string(), json!(true));
            }
            Some(Value::Bool(_)) => {}
            _ => {
                obj.insert("watch".to_string(), json!(false));
            }
        }

        if let Some(n) = obj.get("stop_exit_codes").and_then(|x| x.as_i64()) {
            obj.insert("stop_exit_codes".to_string(), json!([n]));
        } else if !matches!(obj.get("stop_exit_codes"), Some(Value::Array(_))) {
            obj.remove("stop_exit_codes");
        }

        for list_key in ["args", "node_args", "ignore_watch", "watch_paths"] {
            match obj.get(list_key).cloned() {
                Some(Value::String(s)) => {
                    let parts: Vec<String> = s.split_whitespace().map(|x| x.to_string()).collect();
                    obj.insert(list_key.to_string(), json!(parts));
                }
                Some(Value::Array(arr)) => {
                    let parts: Vec<String> = arr
                        .into_iter()
                        .map(|x| match x {
                            Value::String(s) => s,
                            ref other => other.to_string(),
                        })
                        .collect();
                    obj.insert(list_key.to_string(), json!(parts));
                }
                _ => {
                    obj.remove(list_key);
                }
            }
        }

        for k in ["uid", "gid", "instances"] {
            if let Some(val) = obj.get(k).cloned() {
                if let Some(s) = val.as_str() {
                    if let Ok(n) = s.parse::<u64>() {
                        obj.insert(k.to_string(), json!(n));
                    } else {
                        obj.remove(k);
                    }
                } else if !val.is_u64() {
                    obj.remove(k);
                }
            }
        }

        if obj
            .get("versioning")
            .map(|v| serde_json::from_value::<Option<VersioningInfo>>(v.clone()).is_err())
            .unwrap_or(false)
        {
            obj.remove("versioning");
        }
        if obj
            .get("axm_monitor")
            .map(|v| serde_json::from_value::<BTreeMap<String, AxmMetric>>(v.clone()).is_err())
            .unwrap_or(false)
        {
            obj.remove("axm_monitor");
        }
        if obj
            .get("axm_actions")
            .map(|v| serde_json::from_value::<Vec<AxmAction>>(v.clone()).is_err())
            .unwrap_or(false)
        {
            obj.remove("axm_actions");
        }
        for k in ["axm_options", "axm_dynamic"] {
            if matches!(obj.get(k), Some(Value::Null) | Some(Value::Bool(_))) {
                obj.remove(k);
            }
        }

        let mut env: Pm2Env = serde_json::from_value(v)?;
        env.sync_flattened_env();
        Ok(env)
    }

    /// Populate `extra` with top-level copies of `env` variables and `instance_var` for PM2 `jlist` / `env` compatibility.
    pub fn is_reserved_field(k: &str) -> bool {
        matches!(
            k,
            "pm_id"
                | "name"
                | "namespace"
                | "unique_id"
                | "status"
                | "pm_uptime"
                | "created_at"
                | "restart_time"
                | "unstable_restarts"
                | "prev_restart_delay"
                | "exit_code"
                | "min_uptime"
                | "max_restarts"
                | "restart_delay"
                | "exp_backoff_restart_delay"
                | "autorestart"
                | "autostart"
                | "stop_exit_codes"
                | "pm_exec_path"
                | "pm_cwd"
                | "exec_interpreter"
                | "exec_mode"
                | "instances"
                | "args"
                | "node_args"
                | "instance_var"
                | "NODE_APP_INSTANCE"
                | "increment_var"
                | "pm_out_log_path"
                | "pm_err_log_path"
                | "pm_log_path"
                | "pm_pid_path"
                | "merge_logs"
                | "disable_logs"
                | "log_type"
                | "log_date_format"
                | "kill_timeout"
                | "kill_retry_time"
                | "listen_timeout"
                | "wait_ready"
                | "shutdown_with_message"
                | "treekill"
                | "max_memory_restart"
                | "cron_restart"
                | "watch"
                | "watch_paths"
                | "ignore_watch"
                | "watch_delay"
                | "version"
                | "node_version"
                | "_pm2_version"
                | "username"
                | "uid"
                | "gid"
                | "vizion"
                | "versioning"
                | "pmx"
                | "pmx_module"
                | "_km_monitored"
                | "axm_actions"
                | "axm_monitor"
                | "axm_options"
                | "axm_dynamic"
                | "env"
        )
    }

    pub fn sync_flattened_env(&mut self) {
        self.extra.retain(|k, _| !Self::is_reserved_field(k));
        for (k, v) in &self.env {
            if !Self::is_reserved_field(k) {
                if k == &self.name && v.starts_with('{') && v.ends_with('}') {
                    if let Ok(parsed) = serde_json::from_str::<Value>(v) {
                        self.extra.insert(k.clone(), parsed);
                        continue;
                    }
                }
                self.extra.insert(k.clone(), Value::String(v.clone()));
            }
        }
        if let Some(inst) = self.node_app_instance {
            if !self.instance_var.is_empty() && !Self::is_reserved_field(&self.instance_var) {
                self.extra.insert(self.instance_var.clone(), json!(inst));
            }
        }
    }

    /// Apply a dynamic configuration patch (`conf` from `restart`/`reload`).
    pub fn apply_conf_patch(&mut self, patch: &Value) {
        let Some(obj) = patch.as_object() else {
            return;
        };
        if let Some(arr) = obj.get("args").and_then(|v| v.as_array()) {
            self.args = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(arr) = obj.get("node_args").and_then(|v| v.as_array()) {
            self.node_args = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(s) = obj.get("exec_interpreter").and_then(|v| v.as_str()) {
            self.exec_interpreter = s.to_string();
        }
        if let Some(s) = obj.get("namespace").and_then(|v| v.as_str()) {
            self.namespace = s.to_string();
        }
        if let Some(b) = obj.get("pmx").and_then(|v| v.as_bool()) {
            self.pmx = b;
        }
        if let Some(b) = obj.get("vizion").and_then(|v| v.as_bool()) {
            self.vizion = b;
        }
        if let Some(v) = obj.get("max_memory_restart") {
            if v.is_null() || v.as_str() == Some("null") {
                self.max_memory_restart = None;
            } else if let Some(n) = v.as_u64() {
                self.max_memory_restart = Some(n);
            }
        }
        if let Some(v) = obj.get("cron_restart") {
            if v.is_null() || v.as_str() == Some("null") {
                self.cron_restart = None;
            } else if let Some(s) = v.as_str() {
                self.cron_restart = Some(s.to_string());
            }
        }
        if let Some(b) = obj.get("autorestart").and_then(|v| v.as_bool()) {
            self.autorestart = b;
        }
        if let Some(n) = obj.get("max_restarts").and_then(|v| v.as_u64()) {
            self.max_restarts = n as u32;
        }
        if let Some(n) = obj.get("min_uptime").and_then(|v| v.as_u64()) {
            self.min_uptime = n;
        }
        if let Some(n) = obj.get("restart_delay").and_then(|v| v.as_u64()) {
            self.restart_delay = n;
        }
        if let Some(n) = obj
            .get("exp_backoff_restart_delay")
            .and_then(|v| v.as_u64())
        {
            self.exp_backoff_restart_delay = n;
        }
        if let Some(arr) = obj.get("stop_exit_codes").and_then(|v| v.as_array()) {
            self.stop_exit_codes = arr
                .iter()
                .filter_map(|x| x.as_i64().map(|n| n as i32))
                .collect();
        }
        if let Some(b) = obj.get("watch").and_then(|v| v.as_bool()) {
            self.watch = b;
        }
        if let Some(arr) = obj.get("watch_paths").and_then(|v| v.as_array()) {
            self.watch_paths = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(arr) = obj.get("ignore_watch").and_then(|v| v.as_array()) {
            self.ignore_watch = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(n) = obj.get("watch_delay").and_then(|v| v.as_u64()) {
            self.watch_delay = n;
        }
        if let Some(n) = obj.get("kill_timeout").and_then(|v| v.as_u64()) {
            self.kill_timeout = n;
        }
        if let Some(n) = obj.get("listen_timeout").and_then(|v| v.as_u64()) {
            self.listen_timeout = n;
        }
        if let Some(b) = obj.get("wait_ready").and_then(|v| v.as_bool()) {
            self.wait_ready = b;
        }
        if let Some(b) = obj.get("shutdown_with_message").and_then(|v| v.as_bool()) {
            self.shutdown_with_message = b;
        }
        if let Some(b) = obj.get("treekill").and_then(|v| v.as_bool()) {
            self.treekill = b;
        }
        if let Some(v) = obj.get("log_date_format") {
            if v.is_null() || v.as_str() == Some("null") {
                self.log_date_format = None;
            } else if let Some(s) = v.as_str() {
                self.log_date_format = Some(s.to_string());
            }
        }
        if let Some(v) = obj.get("log_type") {
            if v.is_null() || v.as_str() == Some("null") {
                self.log_type = None;
            } else if let Some(s) = v.as_str() {
                self.log_type = Some(s.to_string());
            }
        }
        if let Some(s) = obj.get("instance_var").and_then(|v| v.as_str()) {
            self.instance_var = s.to_string();
        }
        if let Some(s) = obj.get("increment_var").and_then(|v| v.as_str()) {
            self.increment_var = Some(s.to_string());
        }
        if let Some(env_obj) = obj.get("env").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                let s = v
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| v.to_string());
                self.env.insert(k.clone(), s);
            }
        }
        if let Some(mod_val) = obj.get(&self.name) {
            if let Some(mod_obj) = mod_val.as_object() {
                for (k, v) in mod_obj {
                    let s = v
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string());
                    self.env.insert(k.clone(), s);
                }
                if let Ok(json_str) = serde_json::to_string(mod_val) {
                    self.env.insert(self.name.clone(), json_str);
                }
            } else if mod_val.is_null() {
                self.env.remove(&self.name);
                self.extra.remove(&self.name);
            }
        }
        self.sync_flattened_env();
    }
}

fn default_namespace() -> String {
    "default".to_string()
}
fn default_status() -> String {
    ONLINE_STATUS.to_string()
}
fn default_min_uptime() -> u64 {
    DEFAULT_MIN_UPTIME_MS
}
fn default_max_restarts() -> u32 {
    DEFAULT_MAX_RESTARTS
}
fn default_true() -> bool {
    true
}
fn default_interpreter() -> String {
    "none".to_string()
}
fn default_exec_mode() -> String {
    FORK_MODE_ID.to_string()
}
fn default_instance_var() -> String {
    "NODE_APP_INSTANCE".to_string()
}
fn default_kill_timeout() -> u64 {
    KILL_TIMEOUT_MS
}
fn default_kill_retry_time() -> u64 {
    KILL_RETRY_TIME_MS
}
fn default_listen_timeout() -> u64 {
    GRACEFUL_LISTEN_TIMEOUT_MS
}
fn default_version() -> String {
    "N/A".to_string()
}
fn default_pmr_version() -> String {
    PMR_VERSION.to_string()
}
fn default_empty_obj() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessDescription {
    pub pid: u32,
    pub name: String,
    pub pm2_env: Pm2Env,
    pub pm_id: u32,
    pub monit: Monit,
}
