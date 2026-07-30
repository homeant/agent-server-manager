use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSpec {
    pub name: String,
    pub command: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default)]
    pub autorestart: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Exited,
    Errored,
}

impl ServiceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Exited => "exited",
            Self::Errored => "errored",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    #[serde(flatten)]
    pub spec: ServiceSpec,
    pub status: ServiceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    pub last_exit_code: Option<i32>,
    pub last_exit_signal: Option<String>,
    pub restarts: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub restarting: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub name: String,
    pub stream: LogStream,
    pub line: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchItemResult {
    pub name: String,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<ServiceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub action: String,
    pub items: Vec<BatchItemResult>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    #[serde(rename = "ping")]
    Ping { id: u64 },
    #[serde(rename = "list")]
    List { id: u64 },
    #[serde(rename = "info")]
    Info { id: u64, name: String },
    #[serde(rename = "register")]
    Register {
        id: u64,
        spec: ServiceSpec,
        #[serde(default)]
        start: bool,
    },
    #[serde(rename = "start")]
    Start { id: u64, name: String },
    #[serde(rename = "startAll")]
    StartAll { id: u64 },
    #[serde(rename = "stop")]
    Stop { id: u64, name: String },
    #[serde(rename = "stopAll")]
    StopAll { id: u64 },
    #[serde(rename = "restart")]
    Restart { id: u64, name: String },
    #[serde(rename = "remove")]
    Remove { id: u64, name: String },
    #[serde(rename = "removeAll")]
    RemoveAll { id: u64 },
    #[serde(rename = "logs")]
    Logs {
        id: u64,
        name: String,
        lines: Option<usize>,
    },
    #[serde(rename = "attach")]
    Attach {
        id: u64,
        name: String,
        backlog: Option<usize>,
    },
    #[serde(rename = "detach")]
    Detach { id: u64, name: String },
    #[serde(rename = "shutdown")]
    Shutdown { id: u64 },
}

impl Request {
    pub fn id(&self) -> u64 {
        match self {
            Self::Ping { id }
            | Self::List { id }
            | Self::Info { id, .. }
            | Self::Register { id, .. }
            | Self::Start { id, .. }
            | Self::StartAll { id }
            | Self::Stop { id, .. }
            | Self::StopAll { id }
            | Self::Restart { id, .. }
            | Self::Remove { id, .. }
            | Self::RemoveAll { id }
            | Self::Logs { id, .. }
            | Self::Attach { id, .. }
            | Self::Detach { id, .. }
            | Self::Shutdown { id } => *id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    Log {
        name: String,
        stream: LogStream,
        line: String,
        ts: i64,
    },
    Status {
        info: ServiceInfo,
    },
}

impl Event {
    pub fn name(&self) -> &str {
        match self {
            Self::Log { name, .. } => name,
            Self::Status { info } => &info.spec.name,
        }
    }
}

impl From<LogLine> for Event {
    fn from(line: LogLine) -> Self {
        Self::Log {
            name: line.name,
            stream: line.stream,
            line: line.line,
            ts: line.ts,
        }
    }
}
