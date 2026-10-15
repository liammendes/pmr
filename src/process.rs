use serde::{Deserialize, Serialize};
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
    #[serde(default)]
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
