use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::process::{Pm2Env, ProcessDescription};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum RpcRequest {
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "getVersion")]
    GetVersion,
    #[serde(rename = "getReport")]
    GetReport,
    #[serde(rename = "getSystemData")]
    GetSystemData,
    #[serde(rename = "getMonitorData")]
    GetMonitorData,
    #[serde(rename = "prepare")]
    Prepare {
        env: Box<Pm2Env>,
        instances: usize,
    },
    #[serde(rename = "startProcessId")]
    StartProcessId { id: u32 },
    #[serde(rename = "stopProcessId")]
    StopProcessId { id: u32 },
    #[serde(rename = "restartProcessId")]
    RestartProcessId {
        id: u32,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        conf: Option<Value>,
    },
    #[serde(rename = "reloadProcessId")]
    ReloadProcessId {
        id: u32,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        soft: bool,
        #[serde(default)]
        conf: Option<Value>,
    },
    #[serde(rename = "deleteProcessId")]
    DeleteProcessId { id: u32 },
    #[serde(rename = "duplicateProcessId")]
    DuplicateProcessId { id: u32 },
    #[serde(rename = "scale")]
    Scale { name: String, number: String },
    #[serde(rename = "resetMetaProcessId")]
    ResetMetaProcessId { id: u32 },
    #[serde(rename = "sendLineToStdin")]
    SendLineToStdin { pm_id: u32, line: String },
    #[serde(rename = "sendDataToProcessId")]
    SendDataToProcessId { id: u32, packet: Value },
    #[serde(rename = "msgProcess")]
    MsgProcess {
        id: Option<u32>,
        name: Option<String>,
        msg: String,
        opts: Option<Value>,
    },
    #[serde(rename = "sendSignalToProcessId")]
    SendSignalToProcessId { process_id: u32, signal: String },
    #[serde(rename = "sendSignalToProcessName")]
    SendSignalToProcessName {
        process_name: String,
        signal: String,
    },
    #[serde(rename = "startWatch")]
    StartWatch { target: String },
    #[serde(rename = "stopWatch")]
    StopWatch { target: String },
    #[serde(rename = "toggleWatch")]
    ToggleWatch { target: String },
    #[serde(rename = "monitor")]
    Monitor { pm_id: u32, state: bool },
    #[serde(rename = "reloadLogs")]
    ReloadLogs,
    #[serde(rename = "flushLogs")]
    FlushLogs { target: Option<String> },
    #[serde(rename = "notifyByProcessId")]
    NotifyByProcessId {
        id: u32,
        action_name: String,
        manually: bool,
    },
    #[serde(rename = "notifyKillPM2")]
    NotifyKillPM2,
    #[serde(rename = "killMe")]
    KillMe { pid: Option<u32> },
    #[serde(rename = "profile")]
    Profile {
        kind: String,
        pwd: String,
        timeout_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub data: Value,
}

impl RpcResponse {
    pub fn success<T: Serialize>(val: T) -> Self {
        Self {
            ok: true,
            error: None,
            data: serde_json::to_value(val).unwrap_or(Value::Null),
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(msg.into()),
            data: Value::Null,
        }
    }

    pub fn into_processes(self) -> anyhow::Result<Vec<ProcessDescription>> {
        if !self.ok {
            anyhow::bail!("{}", self.error.unwrap_or_else(|| "RPC error".to_string()));
        }
        let procs: Vec<ProcessDescription> = serde_json::from_value(self.data)?;
        Ok(procs)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusProcessRef {
    pub pm_id: Value,
    pub name: String,
    #[serde(default = "default_ns")]
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
}

fn default_ns() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusEvent {
    pub event: String,
    pub at: u64,
    pub process: BusProcessRef,
    #[serde(default)]
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manually: Option<bool>,
}
