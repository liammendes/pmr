use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::constants::{
    interpreter_for_extension, CLUSTER_MODE_ID, DEFAULT_LOG_DATE_FORMAT, DEFAULT_MAX_RESTARTS,
    DEFAULT_MIN_UPTIME_MS, FORK_MODE_ID, GRACEFUL_LISTEN_TIMEOUT_MS, KILL_RETRY_TIME_MS,
    KILL_TIMEOUT_MS, ONLINE_STATUS, PMR_VERSION, STOPPED_STATUS,
};
use crate::paths::PmrPaths;
use crate::process::{Pm2Env, VersioningInfo};

fn deserialize_string_map<'de, D>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<Value>::deserialize(deserializer)?;
    Ok(val.as_ref().map(value_to_string_map).unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployEnvConfig {
    pub user: Option<String>,
    pub host: Option<Value>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub repo: Option<String>,
    pub path: Option<String>,
    pub key: Option<String>,
    pub port: Option<Value>,
    pub ssh_options: Option<Value>,
    #[serde(rename = "pre-setup")]
    pub pre_setup: Option<String>,
    #[serde(rename = "post-setup")]
    pub post_setup: Option<String>,
    #[serde(rename = "pre-deploy-local")]
    pub pre_deploy_local: Option<String>,
    #[serde(rename = "pre-deploy")]
    pub pre_deploy: Option<String>,
    #[serde(rename = "post-deploy")]
    pub post_deploy: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_map")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct EcosystemConfig {
    pub apps: Vec<AppConfig>,
    pub deploy: BTreeMap<String, DeployEnvConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub script: String,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub exec_interpreter: Option<String>,
    #[serde(default)]
    pub node_args: Vec<String>,
    pub exec_mode: Option<String>,
    pub instances: Option<isize>,
    pub out_file: Option<String>,
    pub error_file: Option<String>,
    pub log_file: Option<String>,
    #[serde(default)]
    pub log_file_default: bool,
    pub pid_file: Option<String>,
    pub merge_logs: Option<bool>,
    pub disable_logs: Option<bool>,
    pub log_type: Option<String>,
    pub log_date_format: Option<String>,
    pub time: Option<bool>,
    pub max_memory_restart: Option<u64>,
    pub min_uptime: Option<u64>,
    pub max_restarts: Option<u32>,
    pub restart_delay: Option<u64>,
    pub exp_backoff_restart_delay: Option<u64>,
    pub autorestart: Option<bool>,
    pub autostart: Option<bool>,
    #[serde(default)]
    pub stop_exit_codes: Vec<i32>,
    pub cron_restart: Option<String>,
    pub watch: Option<bool>,
    #[serde(default)]
    pub watch_paths: Vec<String>,
    #[serde(default)]
    pub ignore_watch: Vec<String>,
    pub watch_delay: Option<u64>,
    pub kill_timeout: Option<u64>,
    pub listen_timeout: Option<u64>,
    pub wait_ready: Option<bool>,
    pub shutdown_with_message: Option<bool>,
    pub treekill: Option<bool>,
    pub port: Option<u16>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub gid_name: Option<String>,
    pub username: Option<String>,
    pub instance_var: Option<String>,
    pub increment_var: Option<String>,
    pub append_env_to_name: Option<bool>,
    #[serde(default)]
    pub filter_env: Vec<String>,
    pub filter_env_exact: Option<String>,
    pub filter_env_all: bool,
    pub force: Option<bool>,
    pub vizion: Option<bool>,
    pub source_map_support: Option<bool>,
    pub pmx: Option<bool>,
    pub pmx_module: Option<bool>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub env_variants: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub from_ecosystem: bool,
    #[serde(default)]
    pub multi_app_ecosystem: bool,
}

/// Parse human byte strings like `"150M"`, `"2G"`, `"512K"`, `"1024"`.
pub fn parse_byte_size(val: &Value) -> Option<u64> {
    match val {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => parse_byte_size_str(s),
        _ => None,
    }
}

pub fn parse_byte_size_str(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let upper = s.to_ascii_uppercase();
    let (num_part, mult) =
        if let Some(n) = upper.strip_suffix("GB").or_else(|| upper.strip_suffix('G')) {
            (n.trim(), 1024 * 1024 * 1024u64)
        } else if let Some(n) = upper.strip_suffix("MB").or_else(|| upper.strip_suffix('M')) {
            (n.trim(), 1024 * 1024u64)
        } else if let Some(n) = upper.strip_suffix("KB").or_else(|| upper.strip_suffix('K')) {
            (n.trim(), 1024u64)
        } else if let Some(n) = upper.strip_suffix('B') {
            (n.trim(), 1u64)
        } else {
            (upper.as_str(), 1u64)
        };
    num_part
        .parse::<f64>()
        .ok()
        .map(|v| (v * mult as f64) as u64)
}

/// Parse human duration strings like `"5s"`, `"2m"`, `"1h"`, `"4000ms"`, `"1000"`.
pub fn parse_duration_ms(val: &Value) -> Option<u64> {
    match val {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => parse_duration_ms_str(s),
        _ => None,
    }
}

pub fn parse_duration_ms_str(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(n) = s.strip_suffix("ms") {
        return n.trim().parse::<u64>().ok();
    }
    if let Some(n) = s.strip_suffix('h') {
        return n.trim().parse::<u64>().ok().map(|v| v * 3_600_000);
    }
    if let Some(n) = s.strip_suffix('m') {
        return n.trim().parse::<u64>().ok().map(|v| v * 60_000);
    }
    if let Some(n) = s.strip_suffix('s') {
        return n.trim().parse::<u64>().ok().map(|v| v * 1_000);
    }
    s.parse::<u64>().ok()
}

fn value_to_string_vec(val: &Value) -> Vec<String> {
    match val {
        Value::String(s) => shell_words_split(s),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn shell_words_split(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn value_to_string_map(val: &Value) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Value::Object(obj) = val {
        for (k, v) in obj {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            map.insert(k.clone(), s);
        }
    }
    map
}

fn value_as_u64(val: &Value) -> Option<u64> {
    match val {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok())),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

impl AppConfig {
    pub fn from_json_value(val: &Value) -> Result<Self> {
        let obj = val
            .as_object()
            .ok_or_else(|| anyhow!("Application configuration must be an object"))?;

        let mut cfg = AppConfig::default();

        for (raw_k, v) in obj {
            // Normalize camelCase to snake_case while preserving env_<Name>
            let k = camel_to_snake(raw_k);
            match k.as_str() {
                "script" | "exec" | "cmd" | "command" | "pm_exec_path" => {
                    if let Some(s) = v.as_str() {
                        cfg.script = s.to_string();
                    }
                }
                "name" => cfg.name = v.as_str().map(|s| s.to_string()),
                "namespace" => cfg.namespace = v.as_str().map(|s| s.to_string()),
                "cwd" | "pm_cwd" => cfg.cwd = v.as_str().map(|s| s.to_string()),
                "args" => cfg.args = value_to_string_vec(v),
                "exec_interpreter" | "interpreter" => {
                    cfg.exec_interpreter = v.as_str().map(|s| s.to_string())
                }
                "node_args" | "interpreter_args" => cfg.node_args = value_to_string_vec(v),
                "exec_mode" => cfg.exec_mode = v.as_str().map(|s| s.to_string()),
                "execute_command" => {
                    if v.as_bool() == Some(true) && cfg.exec_mode.is_none() {
                        cfg.exec_mode = Some("fork_mode".to_string());
                    }
                }
                "instances" => match v {
                    Value::Number(n) => cfg.instances = n.as_i64().map(|i| i as isize),
                    Value::String(s) if s == "max" => cfg.instances = Some(0),
                    Value::String(s) => cfg.instances = s.trim().parse::<isize>().ok(),
                    _ => {}
                },
                "out_file" | "out" | "output" | "out_log" | "pm_out_log_path" => {
                    cfg.out_file = v.as_str().map(|s| s.to_string())
                }
                "error_file" | "error" | "err" | "err_file" | "err_log" | "pm_err_log_path" => {
                    cfg.error_file = v.as_str().map(|s| s.to_string())
                }
                "log_file" | "log" | "pm_log_path" => match v {
                    Value::String(s) => {
                        cfg.log_file = Some(s.clone());
                        cfg.log_file_default = false;
                    }
                    Value::Bool(true) => {
                        cfg.log_file_default = true;
                    }
                    Value::Bool(false) => {
                        cfg.log_file = None;
                        cfg.log_file_default = false;
                    }
                    _ => {}
                },
                "pid_file" | "pid" | "pm_pid_path" => {
                    cfg.pid_file = v.as_str().map(|s| s.to_string())
                }
                "merge_logs" | "combine_logs" => cfg.merge_logs = v.as_bool(),
                "disable_logs" => cfg.disable_logs = v.as_bool(),
                "log_type" => cfg.log_type = v.as_str().map(|s| s.to_string()),
                "log_date_format" => match v {
                    Value::String(s) => cfg.log_date_format = Some(s.clone()),
                    Value::Bool(false) => cfg.log_date_format = None,
                    _ => {}
                },
                "time" => cfg.time = v.as_bool(),
                "max_memory_restart" => cfg.max_memory_restart = parse_byte_size(v),
                "min_uptime" => cfg.min_uptime = parse_duration_ms(v),
                "max_restarts" => {
                    cfg.max_restarts = value_as_u64(v).map(|n| n as u32);
                }
                "restart_delay" => cfg.restart_delay = parse_duration_ms(v),
                "exp_backoff_restart_delay" => cfg.exp_backoff_restart_delay = parse_duration_ms(v),
                "autorestart" => cfg.autorestart = v.as_bool(),
                "autostart" => cfg.autostart = v.as_bool(),
                "stop_exit_codes" => match v {
                    Value::Number(n) => {
                        if let Some(code) = n.as_i64() {
                            cfg.stop_exit_codes.push(code as i32);
                        }
                    }
                    Value::String(s) => {
                        if let Ok(code) = s.trim().parse::<i32>() {
                            cfg.stop_exit_codes.push(code);
                        }
                    }
                    Value::Array(arr) => {
                        for item in arr {
                            if let Some(code) = item.as_i64() {
                                cfg.stop_exit_codes.push(code as i32);
                            } else if let Some(s) = item.as_str() {
                                if let Ok(code) = s.trim().parse::<i32>() {
                                    cfg.stop_exit_codes.push(code);
                                }
                            }
                        }
                    }
                    _ => {}
                },
                "cron_restart" | "cron" => match v {
                    Value::String(s) if s == "0" => cfg.cron_restart = None,
                    Value::String(s) => cfg.cron_restart = Some(s.clone()),
                    Value::Number(n) if n.as_i64() == Some(0) => cfg.cron_restart = None,
                    _ => {}
                },
                "watch" => match v {
                    Value::Bool(b) => cfg.watch = Some(*b),
                    Value::String(s) => {
                        cfg.watch = Some(true);
                        cfg.watch_paths.push(s.clone());
                    }
                    Value::Array(arr) => {
                        cfg.watch = Some(true);
                        cfg.watch_paths = value_to_string_vec(&Value::Array(arr.clone()));
                    }
                    _ => {}
                },
                "ignore_watch" => cfg.ignore_watch = value_to_string_vec(v),
                "watch_delay" => cfg.watch_delay = parse_duration_ms(v),
                "kill_timeout" => cfg.kill_timeout = parse_duration_ms(v),
                "listen_timeout" => cfg.listen_timeout = parse_duration_ms(v),
                "wait_ready" => cfg.wait_ready = v.as_bool(),
                "shutdown_with_message" => cfg.shutdown_with_message = v.as_bool(),
                "treekill" => cfg.treekill = v.as_bool(),
                "port" => cfg.port = value_as_u64(v).map(|p| p as u16),
                "uid" | "user" => match v {
                    Value::Number(n) => cfg.uid = n.as_u64().map(|u| u as u32),
                    Value::String(s) => {
                        if let Ok(u) = s.trim().parse::<u32>() {
                            cfg.uid = Some(u);
                        } else {
                            cfg.username = Some(s.clone());
                        }
                    }
                    _ => {}
                },
                "gid" => match v {
                    Value::Number(n) => cfg.gid = n.as_u64().map(|g| g as u32),
                    Value::String(s) => {
                        if let Ok(g) = s.trim().parse::<u32>() {
                            cfg.gid = Some(g);
                        } else {
                            cfg.gid_name = Some(s.clone());
                        }
                    }
                    _ => {}
                },
                "username" => cfg.username = v.as_str().map(|s| s.to_string()),
                "instance_var" => cfg.instance_var = v.as_str().map(|s| s.to_string()),
                "increment_var" => cfg.increment_var = v.as_str().map(|s| s.to_string()),
                "append_env_to_name" => cfg.append_env_to_name = v.as_bool(),
                "filter_env" => match v {
                    Value::Bool(true) => cfg.filter_env_all = true,
                    Value::Bool(false) => {
                        cfg.filter_env_all = false;
                        cfg.filter_env.clear();
                    }
                    Value::String(s) => cfg.filter_env_exact = Some(s.clone()),
                    Value::Array(_) => cfg.filter_env = value_to_string_vec(v),
                    _ => {}
                },
                "force" => cfg.force = v.as_bool(),
                "vizion" => cfg.vizion = v.as_bool(),
                "pmx" | "automation" => cfg.pmx = v.as_bool(),
                "pmx_module" => cfg.pmx_module = v.as_bool(),
                "env" => cfg.env = value_to_string_map(v),
                other if other.starts_with("env_") => {
                    let env_name = &other[4..];
                    if !env_name.is_empty() {
                        cfg.env_variants
                            .insert(env_name.to_string(), value_to_string_map(v));
                    }
                }
                _ => {}
            }
        }

        if cfg.script.is_empty() {
            return Err(anyhow!(
                "Application configuration missing required 'script' field"
            ));
        }

        Ok(cfg)
    }
}

fn camel_to_snake(s: &str) -> String {
    if s.starts_with("env_") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Check if a filename is a known PM2/PMR ecosystem config file.
pub fn is_config_file(filename: &str) -> Option<&'static str> {
    if filename.ends_with(".config.js")
        || filename.ends_with(".config.cjs")
        || filename == "ecosystem.js"
        || filename.ends_with("/ecosystem.js")
    {
        Some("js")
    } else if filename.ends_with(".config.mjs") {
        Some("mjs")
    } else if filename.ends_with(".json")
        || filename.ends_with(".json5")
        || filename.ends_with(".pm2")
        || filename.ends_with(".pmr")
    {
        Some("json")
    } else if filename.ends_with(".yml") || filename.ends_with(".yaml") {
        Some("yaml")
    } else if filename.ends_with(".toml") {
        Some("toml")
    } else {
        None
    }
}

/// Load and parse an ecosystem configuration from disk or raw string.
pub fn parse_config_file(path: &Path) -> Result<EcosystemConfig> {
    let filename = path.to_string_lossy();
    let kind = is_config_file(&filename).unwrap_or("json");
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read configuration file {}", path.display()))?;

    let val: Value = match kind {
        "yaml" => serde_yaml::from_str(&raw)
            .with_context(|| format!("Failed to parse YAML config {}", path.display()))?,
        "toml" => {
            let toml_val: toml::Value = toml::from_str(&raw)
                .with_context(|| format!("Failed to parse TOML config {}", path.display()))?;
            serde_json::to_value(toml_val)?
        }
        "js" | "mjs" => eval_js_config(path, &raw)?,
        _ => {
            if let Ok(v) = parse_json_or_json5(&raw) {
                v
            } else {
                eval_js_config(path, &raw).with_context(|| {
                    format!("Failed to parse JSON/JSON5/JS config {}", path.display())
                })?
            }
        }
    };

    ecosystem_from_value(&val)
}

pub fn parse_config_str(raw: &str, hint: Option<&str>) -> Result<EcosystemConfig> {
    let val: Value = match hint {
        Some("yaml") => serde_yaml::from_str(raw)?,
        Some("toml") => {
            let tv: toml::Value = toml::from_str(raw)?;
            serde_json::to_value(tv)?
        }
        _ => parse_json_or_json5(raw)?,
    };
    ecosystem_from_value(&val)
}

fn parse_json_or_json5(raw: &str) -> Result<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return Ok(v);
    }
    let v: Value = json5::from_str(raw)?;
    Ok(v)
}

fn eval_js_config(path: &Path, raw: &str) -> Result<Value> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix("module.exports")
        .and_then(|s| s.trim_start().strip_prefix('='))
        .or_else(|| trimmed.strip_prefix("export default"))
        .unwrap_or(trimmed)
        .trim()
        .trim_end_matches(';')
        .trim();

    if let Ok(v) = json5::from_str::<Value>(stripped) {
        return Ok(v);
    }

    // Fallback to evaluating with `node` using a delimiter so user console.log doesn't break JSON
    let abs_path = fs::canonicalize(path).unwrap_or_else(|_| {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    });
    let marker = "__PMR_JSON_CONFIG_START__";
    let script = format!(
        "import('url').then(async ({{pathToFileURL}}) => {{ try {{ const m = await import(pathToFileURL({}).href); process.stdout.write({} + JSON.stringify(m.default || m)); }} catch (e) {{ const m = require({}); process.stdout.write({} + JSON.stringify(m)); }} }}).catch(err => {{ console.error(err); process.exit(1); }})",
        serde_json::to_string(&abs_path.to_string_lossy())?,
        serde_json::to_string(marker)?,
        serde_json::to_string(&abs_path.to_string_lossy())?,
        serde_json::to_string(marker)?
    );
    let output = Command::new("node").arg("-e").arg(&script).output()?;
    if output.status.success() {
        let out_str = String::from_utf8_lossy(&output.stdout);
        let json_part = if let Some(idx) = out_str.rfind(marker) {
            &out_str[idx + marker.len()..]
        } else {
            out_str.as_ref()
        };
        if let Ok(v) = serde_json::from_str::<Value>(json_part.trim()) {
            return Ok(v);
        }
    }

    Err(anyhow!(
        "Could not parse JS ecosystem config file {}",
        path.display()
    ))
}

pub fn ecosystem_from_value(val: &Value) -> Result<EcosystemConfig> {
    let mut eco = EcosystemConfig::default();

    if let Some(arr) = val.as_array() {
        let multi = arr.len() > 1;
        for item in arr {
            let mut app = AppConfig::from_json_value(item)?;
            app.from_ecosystem = true;
            app.multi_app_ecosystem = multi;
            eco.apps.push(app);
        }
        return Ok(eco);
    }

    if let Some(obj) = val.as_object() {
        if let Some(deploy_val) = obj.get("deploy").and_then(|v| v.as_object()) {
            for (k, v) in deploy_val {
                if let Ok(d) = serde_json::from_value::<DeployEnvConfig>(v.clone()) {
                    eco.deploy.insert(k.clone(), d);
                }
            }
        }

        if let Some(static_arr) = obj.get("static").and_then(|v| v.as_array()) {
            for item in static_arr {
                if let Some(sobj) = item.as_object() {
                    let serve_path = sobj
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or(".")
                        .to_string();
                    let serve_port = sobj.get("port").and_then(value_as_u64).unwrap_or(8080);
                    let serve_host = sobj
                        .get("host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.0.0.0")
                        .to_string();
                    let name = sobj
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("static-page-server-{}", serve_port));
                    let mut env = BTreeMap::new();
                    env.insert("PM2_SERVE_PATH".to_string(), serve_path);
                    env.insert("PM2_SERVE_PORT".to_string(), serve_port.to_string());
                    env.insert("PM2_SERVE_HOST".to_string(), serve_host);
                    if let Some(spa) = sobj.get("spa").and_then(|v| v.as_bool()) {
                        env.insert("PM2_SERVE_SPA".to_string(), spa.to_string());
                    }
                    if let Some(homepage) = sobj.get("homepage").and_then(|v| v.as_str()) {
                        env.insert("PM2_SERVE_HOMEPAGE".to_string(), homepage.to_string());
                    }
                    if let Some(monitor) = sobj.get("monitor").and_then(|v| v.as_str()) {
                        env.insert("PM2_SERVE_MONITOR".to_string(), monitor.to_string());
                    }
                    if let Some(auth) = sobj.get("basic_auth").and_then(|v| v.as_object()) {
                        if let (Some(u), Some(p)) = (
                            auth.get("username").and_then(|v| v.as_str()),
                            auth.get("password").and_then(|v| v.as_str()),
                        ) {
                            env.insert("PM2_SERVE_BASIC_AUTH".to_string(), "true".to_string());
                            env.insert("PM2_SERVE_BASIC_AUTH_USERNAME".to_string(), u.to_string());
                            env.insert("PM2_SERVE_BASIC_AUTH_PASSWORD".to_string(), p.to_string());
                        }
                    }
                    eco.apps.push(AppConfig {
                        script: "__pmr_serve__".to_string(),
                        name: Some(name),
                        env,
                        from_ecosystem: true,
                        ..Default::default()
                    });
                }
            }
        }

        let apps_val = obj.get("apps").or_else(|| obj.get("pm2")).unwrap_or(val);

        if let Some(arr) = apps_val.as_array() {
            let multi = arr.len() + eco.apps.len() > 1;
            for item in arr {
                let mut app = AppConfig::from_json_value(item)?;
                app.from_ecosystem = true;
                app.multi_app_ecosystem = multi;
                eco.apps.push(app);
            }
        } else if apps_val.is_object()
            && (apps_val.get("script").is_some()
                || apps_val.get("exec").is_some()
                || apps_val.get("cmd").is_some()
                || apps_val.get("command").is_some()
                || apps_val.get("pm_exec_path").is_some())
        {
            let mut app = AppConfig::from_json_value(apps_val)?;
            app.from_ecosystem = true;
            eco.apps.push(app);
        }
    }

    if eco.apps.len() > 1 {
        for app in &mut eco.apps {
            app.multi_app_ecosystem = true;
        }
    }

    Ok(eco)
}

#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub cwd: Option<String>,
    pub args: Vec<String>,
    pub interpreter: Option<String>,
    pub node_args: Vec<String>,
    pub instances: Option<isize>,
    pub exec_mode: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub log: Option<String>,
    pub pid: Option<String>,
    pub merge_logs: bool,
    pub disable_logs: bool,
    pub log_type: Option<String>,
    pub log_date_format: Option<String>,
    pub time: bool,
    pub env_name: Option<String>,
    pub update_env: bool,
    pub force: bool,
    pub max_memory_restart: Option<u64>,
    pub restart_delay: Option<u64>,
    pub exp_backoff_restart_delay: Option<u64>,
    pub max_restarts: Option<u32>,
    pub min_uptime: Option<u64>,
    pub cron_restart: Option<String>,
    pub watch: bool,
    pub watch_paths: Vec<String>,
    pub ignore_watch: Vec<String>,
    pub watch_delay: Option<u64>,
    pub kill_timeout: Option<u64>,
    pub listen_timeout: Option<u64>,
    pub wait_ready: bool,
    pub shutdown_with_message: bool,
    pub no_treekill: bool,
    pub no_autorestart: bool,
    pub no_autostart: bool,
    pub no_vizion: bool,
    pub no_pmx: bool,
    pub stop_exit_codes: Vec<i32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub user: Option<String>,
    pub filter_env: Vec<String>,
    pub only: Option<String>,
    pub extra_env: BTreeMap<String, String>,
    pub write: bool,
    pub ext: Option<String>,
    pub source_map_support: Option<bool>,
    pub trace: Option<bool>,
    pub v8: bool,
    pub event_loop_inspector: bool,
    pub deep_monitoring: bool,
    pub force_autorestart: bool,
    pub force_autostart: bool,
}

fn resolve_log_or_pid_file(
    custom: Option<&str>,
    default_dir: &Path,
    default_filename: &str,
    pm_cwd: &Path,
) -> Result<String> {
    match custom {
        Some(raw) if !raw.is_empty() => {
            if raw == "NULL" || raw == "/dev/null" {
                return Ok("/dev/null".to_string());
            }
            let expanded = PmrPaths::expand_tilde(raw);
            let resolved = if expanded.is_absolute() {
                expanded
            } else {
                pm_cwd.join(expanded)
            };
            if let Some(parent) = resolved.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("Could not create folder: {}", parent.display())
                    })?;
                }
            }
            Ok(resolved.to_string_lossy().to_string())
        }
        _ => Ok(default_dir
            .join(default_filename)
            .to_string_lossy()
            .to_string()),
    }
}

#[cfg(unix)]
fn lookup_user_by_name(name: &str) -> Result<(u32, u32, Option<String>)> {
    let c_name = CString::new(name).map_err(|_| anyhow!("Invalid user name"))?;
    unsafe {
        let pw = libc::getpwnam(c_name.as_ptr());
        if pw.is_null() {
            return Err(anyhow!("[PM2][ERROR] User {} cannot be found", name));
        }
        let uid = (*pw).pw_uid;
        let gid = (*pw).pw_gid;
        let home = if !(*pw).pw_dir.is_null() {
            Some(CStr::from_ptr((*pw).pw_dir).to_string_lossy().to_string())
        } else {
            None
        };
        Ok((uid, gid, home))
    }
}

#[cfg(unix)]
fn lookup_user_by_uid(uid: u32) -> Result<(u32, u32, Option<String>, Option<String>)> {
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return Err(anyhow!("[PM2][ERROR] User {} cannot be found", uid));
        }
        let gid = (*pw).pw_gid;
        let home = if !(*pw).pw_dir.is_null() {
            Some(CStr::from_ptr((*pw).pw_dir).to_string_lossy().to_string())
        } else {
            None
        };
        let name = if !(*pw).pw_name.is_null() {
            Some(CStr::from_ptr((*pw).pw_name).to_string_lossy().to_string())
        } else {
            None
        };
        Ok((uid, gid, home, name))
    }
}

#[cfg(unix)]
fn lookup_group_by_name(name: &str) -> Result<u32> {
    let c_name = CString::new(name).map_err(|_| anyhow!("Invalid group name"))?;
    unsafe {
        let gr = libc::getgrnam(c_name.as_ptr());
        if gr.is_null() {
            return Err(anyhow!("[PM2][ERROR] Group {} cannot be found", name));
        }
        Ok((*gr).gr_gid)
    }
}

/// Merge CLI `StartOptions` into an `AppConfig` and resolve into a ready-to-run `Pm2Env`.
pub fn resolve_app_to_pm2_env(
    mut app: AppConfig,
    opts: &StartOptions,
    deploy_conf: &BTreeMap<String, DeployEnvConfig>,
    paths: &PmrPaths,
) -> Result<(Pm2Env, usize)> {
    if let Some(ref name) = opts.name {
        if !app.from_ecosystem {
            app.name = Some(name.clone());
        }
    }
    if let Some(ref ns) = opts.namespace {
        if !app.from_ecosystem || app.namespace.is_none() {
            app.namespace = Some(ns.clone());
        }
    }
    if let Some(ref cwd) = opts.cwd {
        if !app.from_ecosystem || app.cwd.is_none() {
            app.cwd = Some(cwd.clone());
        }
    }
    if !opts.args.is_empty() {
        app.args.extend(opts.args.clone());
    }
    if let Some(ref interp) = opts.interpreter {
        app.exec_interpreter = Some(interp.clone());
    }
    if !opts.node_args.is_empty() {
        app.node_args.extend(opts.node_args.clone());
    }
    if let Some(inst) = opts.instances {
        if !app.from_ecosystem {
            app.instances = Some(inst);
        }
    }
    if let Some(ref mode) = opts.exec_mode {
        if !app.from_ecosystem || app.exec_mode.is_none() {
            app.exec_mode = Some(mode.clone());
        }
    }
    if let Some(ref out) = opts.output {
        app.out_file = Some(out.clone());
    }
    if let Some(ref err) = opts.error {
        app.error_file = Some(err.clone());
    }
    if let Some(ref log) = opts.log {
        app.log_file = Some(log.clone());
    }
    if let Some(ref pid) = opts.pid {
        app.pid_file = Some(pid.clone());
    }
    if opts.merge_logs {
        app.merge_logs = Some(true);
    }
    if opts.disable_logs {
        app.disable_logs = Some(true);
    }
    if let Some(ref lt) = opts.log_type {
        app.log_type = Some(lt.clone());
    }
    if let Some(ref ldf) = opts.log_date_format {
        app.log_date_format = Some(ldf.clone());
    }
    if (opts.time || app.time.unwrap_or(false) || env::var("ASZ_MODE").is_ok())
        && app.log_date_format.is_none()
    {
        app.log_date_format = Some(DEFAULT_LOG_DATE_FORMAT.to_string());
    }
    if let Some(mmr) = opts.max_memory_restart {
        app.max_memory_restart = Some(mmr);
    }
    if let Some(rd) = opts.restart_delay {
        app.restart_delay = Some(rd);
    }
    if let Some(ebrd) = opts.exp_backoff_restart_delay {
        app.exp_backoff_restart_delay = Some(ebrd);
    }
    if let Some(mr) = opts.max_restarts {
        app.max_restarts = Some(mr);
    }
    if let Some(mu) = opts.min_uptime {
        app.min_uptime = Some(mu);
    }
    if let Some(ref cron) = opts.cron_restart {
        if cron == "0" {
            app.cron_restart = None;
        } else {
            app.cron_restart = Some(cron.clone());
        }
    }
    if opts.watch && (!app.from_ecosystem || app.watch.is_none()) {
        app.watch = Some(true);
    }
    if !opts.watch_paths.is_empty() {
        app.watch_paths.extend(opts.watch_paths.clone());
    }
    if !opts.ignore_watch.is_empty() {
        app.ignore_watch.extend(opts.ignore_watch.clone());
    }
    if let Some(wd) = opts.watch_delay {
        app.watch_delay = Some(wd);
    }
    if let Some(kt) = opts.kill_timeout {
        app.kill_timeout = Some(kt);
    }
    if let Some(lt) = opts.listen_timeout {
        app.listen_timeout = Some(lt);
    }
    if opts.wait_ready {
        app.wait_ready = Some(true);
    }
    if opts.shutdown_with_message {
        app.shutdown_with_message = Some(true);
    }
    if opts.no_treekill {
        app.treekill = Some(false);
    }
    if opts.no_autorestart {
        app.autorestart = Some(false);
    } else if opts.force_autorestart {
        app.autorestart = Some(true);
    }
    if opts.no_autostart {
        app.autostart = Some(false);
    } else if opts.force_autostart {
        app.autostart = Some(true);
    }
    if let Some(sms) = opts.source_map_support {
        app.source_map_support = Some(sms);
    }
    if opts.no_vizion {
        app.vizion = Some(false);
    }
    if opts.no_pmx {
        app.pmx = Some(false);
    }
    if !opts.stop_exit_codes.is_empty() {
        app.stop_exit_codes.extend(opts.stop_exit_codes.clone());
    }
    if let Some(uid) = opts.uid {
        app.uid = Some(uid);
    }
    if let Some(gid) = opts.gid {
        app.gid = Some(gid);
    }
    if let Some(ref user) = opts.user {
        if let Ok(u) = user.trim().parse::<u32>() {
            app.uid = Some(u);
        } else {
            app.username = Some(user.clone());
        }
    }
    if !opts.filter_env.is_empty() {
        app.filter_env.extend(opts.filter_env.clone());
    }

    let caller_cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let base_cwd = match app.cwd {
        Some(ref c) => {
            let expanded = PmrPaths::expand_tilde(c);
            if expanded.is_absolute() {
                expanded
            } else {
                caller_cwd.join(expanded)
            }
        }
        None => caller_cwd,
    };
    let pm_cwd = fs::canonicalize(&base_cwd).unwrap_or(base_cwd);

    // Handle inline shell commands with spaces (like `pm2 start "python3 app.py --foo"`)
    let mut script_str = app.script.clone();
    if script_str == "serve" {
        script_str = "__pmr_serve__".to_string();
    }

    let (pm_exec_path, resolved_interpreter) =
        if script_str == "__pmr_serve__" || script_str == "__pmr_web__" {
            let pmr_exe = PmrPaths::resolve_pmr_binary();
            (pmr_exe.to_string_lossy().to_string(), "none".to_string())
        } else if script_str.contains(' ')
            && !Path::new(&script_str).exists()
            && !pm_cwd.join(&script_str).exists()
        {
            let orig_cmd = script_str.clone();
            if app.name.is_none() {
                app.name = Some(orig_cmd.clone());
            }
            let shell_bin = which_in_path("bash")
                .or_else(|| which_in_path("sh"))
                .unwrap_or_else(|| PathBuf::from("/bin/sh"));
            let mut new_args = vec!["-c".to_string(), orig_cmd];
            new_args.extend(app.args.clone());
            app.args = new_args;
            if app.exec_mode.is_none() {
                app.exec_mode = Some(FORK_MODE_ID.to_string());
            }
            (shell_bin.to_string_lossy().to_string(), "none".to_string())
        } else {
            let expanded_script = PmrPaths::expand_tilde(&script_str);
            let candidate = if expanded_script.is_absolute() {
                expanded_script
            } else {
                pm_cwd.join(&expanded_script)
            };

            let resolved_path = if candidate.exists() {
                if candidate.is_dir() {
                    resolve_directory_script(&candidate)?
                } else {
                    fs::canonicalize(&candidate).unwrap_or(candidate)
                }
            } else if let Some(in_path) = which_in_path(&script_str) {
                in_path
            } else {
                return Err(anyhow!("Script not found: {}", candidate.display()));
            };

            let ext = resolved_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let mut interp = match app.exec_interpreter.clone() {
                Some(i) => i,
                None => match interpreter_for_extension(ext) {
                    Some(i) => i.to_string(),
                    None => {
                        if is_binary_file(&resolved_path) {
                            "none".to_string()
                        } else if has_shebang(&resolved_path) {
                            extract_shebang(&resolved_path).unwrap_or_else(|| "bash".to_string())
                        } else {
                            "node".to_string()
                        }
                    }
                },
            };
            if interp == "python"
                && which_in_path("python").is_none()
                && which_in_path("python3").is_some()
            {
                interp = "python3".to_string();
            }
            (resolved_path.to_string_lossy().to_string(), interp)
        };

    if script_str == "__pmr_serve__" && !app.args.iter().any(|a| a == "__serve") {
        app.args.insert(0, "__serve".to_string());
    } else if script_str == "__pmr_web__" && !app.args.iter().any(|a| a == "__web") {
        app.args.insert(0, "__web".to_string());
    }

    // Default app name = script filename without extension
    let mut app_name = app.name.clone().unwrap_or_else(|| {
        Path::new(&app.script)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("app")
            .to_string()
    });

    if app.append_env_to_name.unwrap_or(false) {
        if let Some(ref env_name) = opts.env_name {
            if !env_name.is_empty() && !app_name.ends_with(&format!("-{}", env_name)) {
                app_name = format!("{}-{}", app_name, env_name);
            }
        }
    }

    // Determine exec_mode & instances
    let instances_specified = app.instances.is_some();
    let raw_instances = app.instances.unwrap_or(1);
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get() as isize)
        .unwrap_or(1);
    let resolved_instances = if raw_instances == 0 {
        num_cpus.max(1) as usize
    } else if raw_instances < 0 {
        (num_cpus + raw_instances).max(1) as usize
    } else {
        raw_instances as usize
    };

    let exec_mode = match app.exec_mode.as_deref() {
        Some("cluster") | Some("cluster_mode") => CLUSTER_MODE_ID.to_string(),
        Some("fork") | Some("fork_mode") => FORK_MODE_ID.to_string(),
        _ => {
            if instances_specified
                && (resolved_interpreter.contains("node") || resolved_interpreter.contains("bun"))
            {
                CLUSTER_MODE_ID.to_string()
            } else {
                FORK_MODE_ID.to_string()
            }
        }
    };

    // Resolve optional uid/gid/user
    let mut resolved_uid: Option<u32> = app.uid;
    let mut resolved_gid: Option<u32> = app.gid;
    let mut resolved_user_home: Option<String> = None;

    #[cfg(unix)]
    {
        if let Some(ref uname) = app.username {
            let (u, g, home) = lookup_user_by_name(uname)?;
            if resolved_uid.is_none() {
                resolved_uid = Some(u);
            }
            if resolved_gid.is_none() {
                resolved_gid = Some(g);
            }
            resolved_user_home = home;
        } else if let Some(u) = resolved_uid {
            if let Ok((_u, g, home, uname)) = lookup_user_by_uid(u) {
                if resolved_gid.is_none() {
                    resolved_gid = Some(g);
                }
                resolved_user_home = home;
                if app.username.is_none() {
                    app.username = uname;
                }
            }
        }
        if let Some(ref gname) = app.gid_name {
            resolved_gid = Some(lookup_group_by_name(gname)?);
        }
    }

    // Environment merging matching PM2:
    // 1. process.env (filtered by filter_env)
    // 2. deploy.<env>.env (when --env is passed)
    // 3. app.env
    // 4. PORT, PYTHONUNBUFFERED, HOME (if uid/user resolved), PM2_HOME, PMR_HOME
    // 5. Module additional conf (from module_conf.json[app_name])
    // 6. app.env_<env> (when --env is passed)
    // 7. opts.extra_env
    let mut merged_env = BTreeMap::new();
    if !app.filter_env_all {
        for (k, v) in env::vars() {
            if let Some(ref exact) = app.filter_env_exact {
                if &k == exact {
                    continue;
                }
            }
            if app
                .filter_env
                .iter()
                .any(|f| !f.is_empty() && k.contains(f))
            {
                continue;
            }
            merged_env.insert(k, v);
        }
    }

    for (k, v) in &app.env {
        merged_env.insert(k.clone(), v.clone());
    }
    if let Some(ref home) = resolved_user_home {
        merged_env.insert("HOME".to_string(), home.clone());
    }
    if let Some(port) = app.port {
        merged_env.insert("PORT".to_string(), port.to_string());
    }
    if resolved_interpreter.contains("python") {
        merged_env.insert("PYTHONUNBUFFERED".to_string(), "1".to_string());
    }
    merged_env.insert(
        "PM2_HOME".to_string(),
        paths.pm2_home.to_string_lossy().to_string(),
    );
    merged_env.insert(
        "PMR_HOME".to_string(),
        paths.pm2_home.to_string_lossy().to_string(),
    );

    // Module additional conf: direct lookup on get_all_sync()[app_name] (no splitting on '.' or ':')
    let all_mod_conf = ModuleConfiguration::get_all_sync(paths);
    if let Some(mod_conf) = all_mod_conf.get(&app_name) {
        if let Some(obj) = mod_conf.as_object() {
            for (k, v) in obj {
                let s = v
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| v.to_string());
                merged_env.insert(k.clone(), s);
            }
            if let Ok(json_str) = serde_json::to_string(mod_conf) {
                merged_env.insert(app_name.clone(), json_str);
            }
        }
    }

    if let Some(ref env_name) = opts.env_name {
        if let Some(d) = deploy_conf.get(env_name) {
            for (k, v) in &d.env {
                merged_env.insert(k.clone(), v.clone());
            }
        }
        if let Some(variant) = app.env_variants.get(env_name) {
            for (k, v) in variant {
                merged_env.insert(k.clone(), v.clone());
            }
        } else {
            println!(
                "{}Environment [{}] is not defined in process file",
                crate::constants::PREFIX_MSG_WARNING,
                env_name
            );
        }
    }
    for (k, v) in &opts.extra_env {
        merged_env.insert(k.clone(), v.clone());
    }

    let formatted_name = PmrPaths::format_app_name(&app_name);
    let out_log = resolve_log_or_pid_file(
        app.out_file.as_deref(),
        &paths.default_log_path,
        &format!("{}-out.log", formatted_name),
        &pm_cwd,
    )?;
    let err_log = resolve_log_or_pid_file(
        app.error_file.as_deref(),
        &paths.default_log_path,
        &format!("{}-error.log", formatted_name),
        &pm_cwd,
    )?;
    let pid_file = resolve_log_or_pid_file(
        app.pid_file.as_deref(),
        &paths.default_pid_path,
        &format!("{}.pid", formatted_name),
        &pm_cwd,
    )?;
    let combined_log = if let Some(ref lf) = app.log_file {
        Some(resolve_log_or_pid_file(
            Some(lf.as_str()),
            &paths.default_log_path,
            &format!("{}.log", formatted_name),
            &pm_cwd,
        )?)
    } else if app.log_file_default {
        Some(
            paths
                .default_log_path
                .join(format!("{}.log", formatted_name))
                .to_string_lossy()
                .to_string(),
        )
    } else {
        None
    };

    let merge_logs = app
        .merge_logs
        .unwrap_or(!instances_specified && exec_mode == FORK_MODE_ID);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let version = find_package_version(Path::new(&pm_exec_path), &pm_cwd);
    let vizion = app.vizion.unwrap_or(true);
    let versioning = if vizion {
        find_git_versioning(&pm_cwd)
    } else {
        None
    };

    let username = app
        .username
        .or_else(|| env::var("USER").ok())
        .or_else(|| env::var("USERNAME").ok())
        .unwrap_or_else(|| "root".to_string());

    let status = if app.autostart.unwrap_or(true) {
        ONLINE_STATUS.to_string()
    } else {
        STOPPED_STATUS.to_string()
    };

    let mut pm2_env = Pm2Env {
        pm_id: 0,
        name: app_name,
        namespace: app.namespace.unwrap_or_else(|| "default".to_string()),
        unique_id: Uuid::new_v4().to_string(),
        status,
        pm_uptime: now_ms,
        created_at: Some(now_ms),
        restart_time: 0,
        unstable_restarts: 0,
        prev_restart_delay: 0,
        exit_code: None,
        min_uptime: app.min_uptime.unwrap_or(DEFAULT_MIN_UPTIME_MS),
        max_restarts: app.max_restarts.unwrap_or(DEFAULT_MAX_RESTARTS),
        restart_delay: app.restart_delay.unwrap_or(0),
        exp_backoff_restart_delay: app.exp_backoff_restart_delay.unwrap_or(0),
        autorestart: app.autorestart.unwrap_or(true),
        autostart: app.autostart.unwrap_or(true),
        stop_exit_codes: app.stop_exit_codes,
        pm_exec_path,
        pm_cwd: pm_cwd.to_string_lossy().to_string(),
        exec_interpreter: resolved_interpreter,
        exec_mode,
        instances: Some(resolved_instances),
        args: app.args,
        node_args: app.node_args,
        instance_var: app
            .instance_var
            .unwrap_or_else(|| "NODE_APP_INSTANCE".to_string()),
        node_app_instance: Some(0),
        increment_var: app.increment_var,
        pm_out_log_path: out_log,
        pm_err_log_path: err_log,
        pm_log_path: combined_log,
        pm_pid_path: pid_file,
        merge_logs,
        disable_logs: app.disable_logs.unwrap_or(false),
        log_type: app.log_type,
        log_date_format: app.log_date_format,
        kill_timeout: app.kill_timeout.unwrap_or(KILL_TIMEOUT_MS),
        kill_retry_time: KILL_RETRY_TIME_MS,
        listen_timeout: app.listen_timeout.unwrap_or(GRACEFUL_LISTEN_TIMEOUT_MS),
        wait_ready: app.wait_ready.unwrap_or(false),
        shutdown_with_message: app.shutdown_with_message.unwrap_or(false),
        treekill: app.treekill.unwrap_or(true),
        max_memory_restart: app.max_memory_restart,
        cron_restart: app.cron_restart,
        watch: app.watch.unwrap_or(false),
        watch_paths: app.watch_paths,
        ignore_watch: app.ignore_watch,
        watch_delay: app.watch_delay.unwrap_or(0),
        version,
        node_version: None,
        _pm2_version: PMR_VERSION.to_string(),
        username,
        uid: resolved_uid,
        gid: resolved_gid,
        vizion,
        versioning,
        pmx: app.pmx.unwrap_or(true),
        pmx_module: app.pmx_module.unwrap_or(false),
        _km_monitored: false,
        axm_actions: Vec::new(),
        axm_monitor: BTreeMap::new(),
        axm_options: serde_json::json!({}),
        axm_dynamic: serde_json::json!({}),
        env: merged_env,
        extra: BTreeMap::new(),
    };
    pm2_env.sync_flattened_env();

    Ok((pm2_env, resolved_instances))
}

fn resolve_directory_script(dir: &Path) -> Result<PathBuf> {
    let pkg_json = dir.join("package.json");
    if pkg_json.exists() {
        if let Ok(raw) = fs::read_to_string(&pkg_json) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                if let Some(main) = val.get("main").and_then(|m| m.as_str()) {
                    let main_path = dir.join(main);
                    if main_path.exists() {
                        return Ok(fs::canonicalize(&main_path).unwrap_or(main_path));
                    }
                }
            }
        }
    }
    let index_js = dir.join("index.js");
    if index_js.exists() {
        return Ok(fs::canonicalize(&index_js).unwrap_or(index_js));
    }
    Err(anyhow!("Script not found in directory {}", dir.display()))
}

pub fn which_in_path(bin_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let full = dir.join(bin_name);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

pub fn is_binary_file(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let sample = &bytes[..bytes.len().min(512)];
    if sample.is_empty() {
        return false;
    }
    if sample.starts_with(b"\x7fELF")
        || sample.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || sample.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || sample.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
    {
        return true;
    }
    sample.contains(&0)
}

fn has_shebang(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    bytes.starts_with(b"#!")
}

fn extract_shebang(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let first_line = content.lines().next()?;
    let shebang = first_line.strip_prefix("#!")?.trim();
    let parts: Vec<&str> = shebang.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    if parts[0].ends_with("/env") && parts.len() > 1 {
        Some(parts[1].to_string())
    } else {
        Some(parts[0].to_string())
    }
}

pub fn find_package_version(exec_path: &Path, cwd: &Path) -> String {
    let start = exec_path.parent().unwrap_or(cwd);
    let mut cur = Some(start);
    while let Some(dir) = cur {
        let pkg = dir.join("package.json");
        if pkg.is_file() {
            if let Ok(raw) = fs::read_to_string(&pkg) {
                if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                    if let Some(v) = val.get("version").and_then(|v| v.as_str()) {
                        return v.to_string();
                    }
                }
            }
        }
        let cargo = dir.join("Cargo.toml");
        if cargo.is_file() {
            if let Ok(raw) = fs::read_to_string(&cargo) {
                if let Ok(val) = toml::from_str::<toml::Value>(&raw) {
                    if let Some(v) = val
                        .get("package")
                        .and_then(|p| p.get("version"))
                        .and_then(|v| v.as_str())
                    {
                        return v.to_string();
                    }
                }
            }
        }
        cur = dir.parent();
    }
    "N/A".to_string()
}

pub fn find_git_versioning(cwd: &Path) -> Option<VersioningInfo> {
    let mut cur = Some(cwd);
    let mut repo_root = None;
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            repo_root = Some(dir.to_path_buf());
            break;
        }
        cur = dir.parent();
    }
    let repo_path = repo_root?;

    let run_git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&repo_path)
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let revision = run_git(&["rev-parse", "HEAD"]).unwrap_or_default();
    if revision.is_empty() {
        return None;
    }
    let branch =
        run_git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "main".to_string());
    let comment = run_git(&["log", "-1", "--pretty=%s"]).unwrap_or_default();
    let url = run_git(&["config", "--get", "remote.origin.url"]).unwrap_or_default();
    let update_time = Some(chrono::Utc::now().to_rfc3339());

    Some(VersioningInfo {
        vcs_type: "git".to_string(),
        url,
        revision,
        comment,
        branch,
        repo_path: repo_path.to_string_lossy().to_string(),
        update_time,
    })
}

/// Persistent key-value configuration manager (`~/.pmr/module_conf.json`), matching `lib/Configuration.js`.
pub struct ModuleConfiguration;

impl ModuleConfiguration {
    pub fn split_key(key: &str) -> Vec<String> {
        let sep = if key.contains('.') {
            Some('.')
        } else if key.contains(':') {
            Some(':')
        } else {
            None
        };
        let Some(delim) = sep else {
            return vec![key.to_string()];
        };
        let mut out = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        for ch in key.chars() {
            if ch == '"' {
                in_quotes = !in_quotes;
            } else if ch == delim && !in_quotes {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
        out
    }

    fn read_checked(paths: &PmrPaths) -> Result<Value> {
        if !paths.pm2_module_conf_file.exists() {
            return Ok(serde_json::json!({}));
        }
        let raw = fs::read_to_string(&paths.pm2_module_conf_file)?;
        if raw.trim().is_empty() {
            return Ok(serde_json::json!({}));
        }
        let val: Value = serde_json::from_str(&raw)
            .with_context(|| format!("Invalid JSON in {}", paths.pm2_module_conf_file.display()))?;
        Ok(val)
    }

    fn write_atomic(paths: &PmrPaths, val: &Value) -> Result<()> {
        let serialized = serde_json::to_string_pretty(val)?;
        let tmp = paths
            .pm2_module_conf_file
            .with_extension(format!("json.tmp.{}", std::process::id()));
        fs::write(&tmp, serialized)?;
        fs::rename(&tmp, &paths.pm2_module_conf_file)?;
        Ok(())
    }

    pub fn get_all_sync(paths: &PmrPaths) -> Value {
        Self::read_checked(paths).unwrap_or_else(|_| serde_json::json!({}))
    }

    pub fn get_sync(paths: &PmrPaths, key: &str) -> Option<Value> {
        let all = Self::get_all_sync(paths);
        if key.is_empty() || key == "all" {
            return Some(all);
        }
        let parts = Self::split_key(key);
        let mut cur = &all;
        for part in parts {
            cur = cur.get(&part)?;
        }
        Some(cur.clone())
    }

    pub fn set_sync(paths: &PmrPaths, key: &str, value: &str) -> Result<Value> {
        Self::set_value_sync(paths, key, Value::String(value.to_string()))
    }

    pub fn set_value_sync(paths: &PmrPaths, key: &str, value: Value) -> Result<Value> {
        let mut all = Self::read_checked(paths)?;
        if !all.is_object() {
            all = serde_json::json!({});
        }
        let parts = Self::split_key(key);
        if parts.is_empty() {
            return Ok(all);
        }
        let mut cur = &mut all;
        for (i, part) in parts.iter().enumerate() {
            if i + 1 == parts.len() {
                if let Some(obj) = cur.as_object_mut() {
                    obj.insert(part.clone(), value);
                }
                break;
            } else {
                if !cur.get(part).map(|v| v.is_object()).unwrap_or(false) {
                    if let Some(obj) = cur.as_object_mut() {
                        obj.insert(part.clone(), serde_json::json!({}));
                    }
                }
                cur = cur.get_mut(part).unwrap();
            }
        }
        Self::write_atomic(paths, &all)?;
        Ok(all)
    }

    pub fn unset_sync(paths: &PmrPaths, key: &str) -> Result<Value> {
        let mut all = Self::read_checked(paths)?;
        if key == "all" {
            all = serde_json::json!({});
        } else {
            let parts = Self::split_key(key);
            if !parts.is_empty() {
                let mut cur = &mut all;
                for (i, part) in parts.iter().enumerate() {
                    if i + 1 == parts.len() {
                        if let Some(obj) = cur.as_object_mut() {
                            obj.remove(part);
                        }
                        break;
                    } else if let Some(next) = cur.get_mut(part) {
                        if !next.is_object() {
                            break;
                        }
                        cur = next;
                    } else {
                        break;
                    }
                }
            }
        }
        Self::write_atomic(paths, &all)?;
        Ok(all)
    }

    pub fn multiset_sync(paths: &PmrPaths, serial: &str) -> Result<Value> {
        let tokens = shell_words_split(serial);
        for pair in tokens.chunks(2) {
            if pair.len() == 2 {
                Self::set_sync(paths, &pair[0], &pair[1])?;
            }
        }
        Self::read_checked(paths)
    }
}
