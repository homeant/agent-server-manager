use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs::{self, OpenOptions},
    future::Future,
    io::Write,
    path::Path,
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use anyhow::{Context, Result, anyhow, bail};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    sync::{Mutex, Semaphore, broadcast},
    time::{sleep, timeout},
};

use crate::{
    model::{
        BatchItemResult, BatchResult, Event, LogLine, LogStream, ServiceInfo, ServiceSpec,
        ServiceStatus,
    },
    paths::Paths,
};

const RING_SIZE: usize = 2_000;
const STOP_GRACE: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_secs(1);
const BATCH_CONCURRENCY: usize = 4;

#[derive(Clone, Copy)]
enum KillMode {
    Graceful,
    Force,
}

struct ManagedService {
    spec: ServiceSpec,
    status: ServiceStatus,
    pid: Option<u32>,
    started_at: Option<i64>,
    last_exit_code: Option<i32>,
    last_exit_signal: Option<String>,
    restarts: u32,
    restarting: bool,
    intentional_stop: bool,
    generation: u64,
    ring: VecDeque<LogLine>,
}

impl ManagedService {
    fn new(spec: ServiceSpec) -> Self {
        Self {
            spec,
            status: ServiceStatus::Stopped,
            pid: None,
            started_at: None,
            last_exit_code: None,
            last_exit_signal: None,
            restarts: 0,
            restarting: false,
            intentional_stop: false,
            generation: 0,
            ring: VecDeque::with_capacity(RING_SIZE),
        }
    }

    fn info(&self) -> ServiceInfo {
        ServiceInfo {
            spec: self.spec.clone(),
            status: self.status,
            pid: self.pid,
            cpu_percent: None,
            memory_bytes: None,
            started_at: self.started_at,
            last_exit_code: self.last_exit_code,
            last_exit_signal: self.last_exit_signal.clone(),
            restarts: self.restarts,
            restarting: self.restarting,
        }
    }
}

#[derive(Default)]
struct Registry {
    order: Vec<String>,
    services: HashMap<String, ManagedService>,
}

pub struct Supervisor {
    paths: Paths,
    registry: Mutex<Registry>,
    mutation: Mutex<()>,
    events: broadcast::Sender<Event>,
}

impl Supervisor {
    pub fn new(paths: Paths) -> Result<Arc<Self>> {
        fs::create_dir_all(&paths.log_dir)
            .with_context(|| format!("无法创建日志目录 {}", paths.log_dir.display()))?;
        let (events, _) = broadcast::channel(4_096);
        let supervisor = Arc::new(Self {
            registry: Mutex::new(load_registry(&paths.registry)),
            mutation: Mutex::new(()),
            events,
            paths,
        });
        Ok(supervisor)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub async fn has(&self, name: &str) -> bool {
        self.registry.lock().await.services.contains_key(name)
    }

    pub async fn get(&self, name: &str) -> Option<ServiceInfo> {
        self.registry
            .lock()
            .await
            .services
            .get(name)
            .map(ManagedService::info)
    }

    pub async fn info(&self, name: &str) -> Option<ServiceInfo> {
        let usage = read_process_usage();
        self.get(name).await.map(|mut info| {
            add_process_usage(&mut info, &usage);
            info
        })
    }

    pub async fn list(&self) -> Vec<ServiceInfo> {
        let usage = read_process_usage();
        let registry = self.registry.lock().await;
        registry
            .order
            .iter()
            .filter_map(|name| registry.services.get(name))
            .map(|service| {
                let mut info = service.info();
                add_process_usage(&mut info, &usage);
                info
            })
            .collect()
    }

    pub async fn logs(&self, name: &str, lines: usize) -> Result<Vec<LogLine>> {
        let registry = self.registry.lock().await;
        let service = registry
            .services
            .get(name)
            .ok_or_else(|| anyhow!("未知服务: {name}"))?;
        let skip = service.ring.len().saturating_sub(lines);
        Ok(service.ring.iter().skip(skip).cloned().collect())
    }

    pub async fn register(self: &Arc<Self>, spec: ServiceSpec, start: bool) -> Result<ServiceInfo> {
        let _mutation = self.mutation.lock().await;
        let info = {
            let mut registry = self.registry.lock().await;
            if let Some(service) = registry.services.get_mut(&spec.name) {
                service.spec = spec.clone();
                service.info()
            } else {
                registry.order.push(spec.name.clone());
                registry
                    .services
                    .insert(spec.name.clone(), ManagedService::new(spec.clone()));
                registry.services.get(&spec.name).unwrap().info()
            }
        };
        self.persist_registry().await;
        self.system(
            &spec.name,
            format!("registered: {}  (cwd: {})", spec.command, spec.cwd),
        )
        .await;
        self.emit_status(&spec.name).await;
        if start {
            self.start_locked(&spec.name).await
        } else {
            Ok(info)
        }
    }

    pub async fn start(self: &Arc<Self>, name: &str) -> Result<ServiceInfo> {
        let _mutation = self.mutation.lock().await;
        self.start_locked(name).await
    }

    async fn start_locked(self: &Arc<Self>, name: &str) -> Result<ServiceInfo> {
        if let Some(info) = self.get(name).await {
            if matches!(
                info.status,
                ServiceStatus::Running | ServiceStatus::Starting
            ) {
                return Ok(info);
            }
        } else {
            bail!("未知服务: {name}");
        }
        self.spawn_process(name).await?;
        sleep(SETTLE).await;
        self.get(name)
            .await
            .ok_or_else(|| anyhow!("未知服务: {name}"))
    }

    pub async fn stop(self: &Arc<Self>, name: &str) -> Result<ServiceInfo> {
        let _mutation = self.mutation.lock().await;
        self.stop_locked(name).await
    }

    async fn stop_locked(&self, name: &str) -> Result<ServiceInfo> {
        let pid = {
            let mut registry = self.registry.lock().await;
            let service = registry
                .services
                .get_mut(name)
                .ok_or_else(|| anyhow!("未知服务: {name}"))?;
            let Some(pid) = service.pid else {
                service.status = ServiceStatus::Stopped;
                let info = service.info();
                drop(registry);
                let _ = self.events.send(Event::Status { info: info.clone() });
                return Ok(info);
            };
            service.intentional_stop = true;
            service.status = ServiceStatus::Stopping;
            let info = service.info();
            let _ = self.events.send(Event::Status { info });
            pid
        };

        kill_group(pid, KillMode::Graceful);
        if timeout(STOP_GRACE, self.wait_for_exit(name, pid))
            .await
            .is_err()
        {
            self.system(name, "SIGTERM 超时，强制 SIGKILL".to_string())
                .await;
            kill_group(pid, KillMode::Force);
            let _ = timeout(Duration::from_secs(2), self.wait_for_exit(name, pid)).await;
        }
        let info = self
            .get(name)
            .await
            .ok_or_else(|| anyhow!("未知服务: {name}"))?;
        if info.pid == Some(pid) {
            bail!("服务进程组 {pid} 在 SIGKILL 后仍未退出");
        }
        Ok(info)
    }

    async fn wait_for_exit(&self, name: &str, pid: u32) {
        loop {
            let exited = {
                let registry = self.registry.lock().await;
                registry
                    .services
                    .get(name)
                    .map(|service| service.pid != Some(pid))
                    .unwrap_or(true)
            };
            if exited {
                return;
            }
            sleep(Duration::from_millis(40)).await;
        }
    }

    pub async fn restart(self: &Arc<Self>, name: &str) -> Result<ServiceInfo> {
        let _mutation = self.mutation.lock().await;
        if !self.has(name).await {
            bail!("未知服务: {name}");
        }
        self.system(name, "restarting...".to_string()).await;
        {
            let mut registry = self.registry.lock().await;
            registry.services.get_mut(name).unwrap().restarting = true;
        }
        self.emit_status(name).await;
        let result = async {
            self.stop_locked(name).await?;
            {
                let mut registry = self.registry.lock().await;
                registry.services.get_mut(name).unwrap().restarts = 0;
            }
            self.spawn_process(name).await?;
            sleep(SETTLE).await;
            self.get(name)
                .await
                .ok_or_else(|| anyhow!("未知服务: {name}"))
        }
        .await;
        {
            let mut registry = self.registry.lock().await;
            if let Some(service) = registry.services.get_mut(name) {
                service.restarting = false;
            }
        }
        self.emit_status(name).await;
        result
    }

    pub async fn remove(self: &Arc<Self>, name: &str) -> Result<()> {
        let _mutation = self.mutation.lock().await;
        self.remove_locked(name, true).await
    }

    async fn remove_locked(&self, name: &str, persist: bool) -> Result<()> {
        self.stop_locked(name).await?;
        let mut registry = self.registry.lock().await;
        registry.services.remove(name);
        registry.order.retain(|item| item != name);
        drop(registry);
        if persist {
            self.persist_registry().await;
        }
        Ok(())
    }

    pub async fn start_all(self: &Arc<Self>) -> BatchResult {
        let _mutation = self.mutation.lock().await;
        let names = self.names().await;
        let semaphore = Arc::new(Semaphore::new(BATCH_CONCURRENCY));
        let mut tasks = Vec::with_capacity(names.len());
        for (index, name) in names.iter().cloned().enumerate() {
            let supervisor = Arc::clone(self);
            let semaphore = Arc::clone(&semaphore);
            let task_name = name.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire_owned().await.unwrap();
                let before = supervisor.get(&task_name).await.unwrap();
                if matches!(
                    before.status,
                    ServiceStatus::Running | ServiceStatus::Starting
                ) {
                    return BatchItemResult {
                        name: task_name,
                        outcome: "skipped".into(),
                        info: Some(before.clone()),
                        reason: Some(if before.status == ServiceStatus::Running {
                            "already-running".into()
                        } else {
                            "already-starting".into()
                        }),
                        error: None,
                    };
                }
                match supervisor.start_locked(&task_name).await {
                    Ok(info)
                        if matches!(
                            info.status,
                            ServiceStatus::Running | ServiceStatus::Starting
                        ) =>
                    {
                        batch_item(&task_name, "started", Some(info), None)
                    }
                    Ok(info) => batch_item(
                        &task_name,
                        "failed",
                        Some(info.clone()),
                        Some(format!("启动后状态为 {}", info.status.as_str())),
                    ),
                    Err(error) => batch_item(&task_name, "failed", None, Some(error.to_string())),
                }
            });
            tasks.push((index, name, task));
        }
        let mut ordered = vec![None; names.len()];
        for (index, name, task) in tasks {
            ordered[index] = Some(match task.await {
                Ok(item) => item,
                Err(error) => batch_item(&name, "failed", None, Some(error.to_string())),
            });
        }
        BatchResult {
            action: "start".into(),
            items: ordered.into_iter().flatten().collect(),
        }
    }

    pub async fn stop_all(self: &Arc<Self>) -> BatchResult {
        let _mutation = self.mutation.lock().await;
        let names = self.names().await;
        let semaphore = Arc::new(Semaphore::new(BATCH_CONCURRENCY));
        let mut tasks = Vec::with_capacity(names.len());
        for (index, name) in names.iter().cloned().enumerate() {
            let supervisor = Arc::clone(self);
            let semaphore = Arc::clone(&semaphore);
            let task_name = name.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire_owned().await.unwrap();
                let before = supervisor.get(&task_name).await.unwrap();
                if before.pid.is_none() {
                    return BatchItemResult {
                        name: task_name,
                        outcome: "skipped".into(),
                        info: Some(before),
                        reason: Some("not-running".into()),
                        error: None,
                    };
                }
                match supervisor.stop_locked(&task_name).await {
                    Ok(info) => batch_item(&task_name, "stopped", Some(info), None),
                    Err(error) => batch_item(
                        &task_name,
                        "failed",
                        supervisor.get(&task_name).await,
                        Some(error.to_string()),
                    ),
                }
            });
            tasks.push((index, name, task));
        }
        let mut ordered = vec![None; names.len()];
        for (index, name, task) in tasks {
            ordered[index] = Some(match task.await {
                Ok(item) => item,
                Err(error) => batch_item(&name, "failed", None, Some(error.to_string())),
            });
        }
        BatchResult {
            action: "stop".into(),
            items: ordered.into_iter().flatten().collect(),
        }
    }

    pub async fn remove_all(self: &Arc<Self>) -> BatchResult {
        let _mutation = self.mutation.lock().await;
        let names = self.names().await;
        let semaphore = Arc::new(Semaphore::new(BATCH_CONCURRENCY));
        let mut tasks = Vec::with_capacity(names.len());
        for (index, name) in names.iter().cloned().enumerate() {
            let supervisor = Arc::clone(self);
            let semaphore = Arc::clone(&semaphore);
            let task_name = name.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire_owned().await.unwrap();
                match supervisor.remove_locked(&task_name, false).await {
                    Ok(()) => batch_item(&task_name, "removed", None, None),
                    Err(error) => batch_item(
                        &task_name,
                        "failed",
                        supervisor.get(&task_name).await,
                        Some(error.to_string()),
                    ),
                }
            });
            tasks.push((index, name, task));
        }
        let mut ordered = vec![None; names.len()];
        for (index, name, task) in tasks {
            ordered[index] = Some(match task.await {
                Ok(item) => item,
                Err(error) => batch_item(&name, "failed", None, Some(error.to_string())),
            });
        }
        self.persist_registry().await;
        BatchResult {
            action: "remove".into(),
            items: ordered.into_iter().flatten().collect(),
        }
    }

    pub async fn shutdown(self: &Arc<Self>) {
        let _ = self.stop_all().await;
    }

    async fn names(&self) -> Vec<String> {
        self.registry.lock().await.order.clone()
    }

    fn spawn_process<'a>(
        self: &'a Arc<Self>,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let (spec, generation) = {
                let mut registry = self.registry.lock().await;
                let service = registry
                    .services
                    .get_mut(name)
                    .ok_or_else(|| anyhow!("未知服务: {name}"))?;
                service.status = ServiceStatus::Starting;
                service.intentional_stop = false;
                service.generation += 1;
                (service.spec.clone(), service.generation)
            };
            self.emit_status(name).await;

            #[cfg(unix)]
            let mut command = {
                let mut command = Command::new("/bin/sh");
                command.arg("-c");
                command
            };
            #[cfg(windows)]
            let mut command = {
                let mut command = Command::new("cmd.exe");
                command.args(["/D", "/S", "/C"]);
                command
            };
            command
                .arg(&spec.command)
                .current_dir(&spec.cwd)
                .env_clear()
                .envs(build_service_env(spec.env.as_ref()))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            command.process_group(0);
            #[cfg(windows)]
            command.creation_flags(0x0000_0200 | 0x0800_0000);

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    {
                        let mut registry = self.registry.lock().await;
                        if let Some(service) = registry.services.get_mut(name) {
                            service.status = ServiceStatus::Errored;
                        }
                    }
                    self.system(name, format!("spawn error: {error}")).await;
                    self.emit_status(name).await;
                    return Ok(());
                }
            };

            let pid = child.id().ok_or_else(|| anyhow!("启动后未获得 pid"))?;
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let mut registry = self.registry.lock().await;
                let service = registry.services.get_mut(name).unwrap();
                service.pid = Some(pid);
                service.started_at = Some(now_ms());
                service.status = ServiceStatus::Running;
            }
            self.system(name, format!("started (pid {pid})")).await;
            self.emit_status(name).await;

            if let Some(stdout) = stdout {
                tokio::spawn(read_output(
                    Arc::clone(self),
                    name.to_string(),
                    generation,
                    LogStream::Stdout,
                    stdout,
                ));
            }
            if let Some(stderr) = stderr {
                tokio::spawn(read_output(
                    Arc::clone(self),
                    name.to_string(),
                    generation,
                    LogStream::Stderr,
                    stderr,
                ));
            }

            let supervisor = Arc::clone(self);
            let service_name = name.to_string();
            tokio::spawn(async move {
                let status = child.wait().await;
                supervisor
                    .handle_exit(&service_name, generation, status)
                    .await;
            });
            Ok(())
        })
    }

    async fn handle_exit(
        self: Arc<Self>,
        name: &str,
        generation: u64,
        result: std::io::Result<std::process::ExitStatus>,
    ) {
        let (autorestart, intentional, code, signal) = {
            let mut registry = self.registry.lock().await;
            let Some(service) = registry.services.get_mut(name) else {
                return;
            };
            if service.generation != generation {
                return;
            }
            let (code, signal) = match result {
                Ok(status) => exit_details(status),
                Err(_) => (None, None),
            };
            let intentional = service.intentional_stop;
            service.pid = None;
            service.last_exit_code = code;
            service.last_exit_signal = signal.clone();
            service.status = if intentional {
                ServiceStatus::Stopped
            } else {
                ServiceStatus::Exited
            };
            (service.spec.autorestart, intentional, code, signal)
        };
        let suffix = signal
            .as_ref()
            .map(|value| format!(", signal {value}"))
            .unwrap_or_default();
        self.system(
            name,
            format!(
                "exited (code {}{suffix})",
                code.map(|value| value.to_string())
                    .unwrap_or_else(|| "null".into())
            ),
        )
        .await;
        self.emit_status(name).await;

        if autorestart && !intentional {
            self.system(name, "autorestart...".to_string()).await;
            {
                let mut registry = self.registry.lock().await;
                if let Some(service) = registry.services.get_mut(name) {
                    service.restarts += 1;
                }
            }
            sleep(Duration::from_millis(500)).await;
            let should_restart = {
                let registry = self.registry.lock().await;
                registry
                    .services
                    .get(name)
                    .map(|service| {
                        service.generation == generation && service.status == ServiceStatus::Exited
                    })
                    .unwrap_or(false)
            };
            if should_restart {
                let _ = self.spawn_process(name).await;
            }
        }
    }

    async fn push_line(&self, name: &str, stream: LogStream, line: String) {
        let entry = LogLine {
            name: name.to_string(),
            stream,
            line,
            ts: now_ms(),
        };
        {
            let mut registry = self.registry.lock().await;
            let Some(service) = registry.services.get_mut(name) else {
                return;
            };
            service.ring.push_back(entry.clone());
            if service.ring.len() > RING_SIZE {
                service.ring.pop_front();
            }
        }
        append_log(&self.paths.log_dir, &entry);
        let _ = self.events.send(entry.into());
    }

    async fn system(&self, name: &str, line: String) {
        self.push_line(name, LogStream::System, line).await;
    }

    async fn emit_status(&self, name: &str) {
        if let Some(info) = self.get(name).await {
            let _ = self.events.send(Event::Status { info });
        }
    }

    async fn persist_registry(&self) {
        let specs: Vec<ServiceSpec> = {
            let registry = self.registry.lock().await;
            registry
                .order
                .iter()
                .filter_map(|name| registry.services.get(name))
                .map(|service| service.spec.clone())
                .collect()
        };
        let Ok(json) = serde_json::to_vec_pretty(&specs) else {
            return;
        };
        let temporary = self.paths.registry.with_extension("json.tmp");
        if fs::write(&temporary, json).is_ok() {
            let _ = fs::rename(temporary, &self.paths.registry);
        }
    }
}

async fn read_output<R>(
    supervisor: Arc<Supervisor>,
    name: String,
    generation: u64,
    stream: LogStream,
    reader: R,
) where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer).await {
            Ok(0) => break,
            Ok(_) => {
                while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                    buffer.pop();
                }
                if buffer.is_empty() {
                    continue;
                }
                let is_current = {
                    let registry = supervisor.registry.lock().await;
                    registry
                        .services
                        .get(&name)
                        .map(|service| service.generation == generation)
                        .unwrap_or(false)
                };
                if !is_current {
                    break;
                }
                supervisor
                    .push_line(
                        &name,
                        stream.clone(),
                        String::from_utf8_lossy(&buffer).into_owned(),
                    )
                    .await;
            }
            Err(_) => break,
        }
    }
}

fn load_registry(path: &Path) -> Registry {
    let specs: Vec<ServiceSpec> = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut registry = Registry::default();
    for spec in specs {
        if spec.name.is_empty() || spec.command.is_empty() {
            continue;
        }
        registry.order.retain(|name| name != &spec.name);
        registry.order.push(spec.name.clone());
        registry
            .services
            .insert(spec.name.clone(), ManagedService::new(spec));
    }
    registry
}

fn build_service_env(overrides: Option<&BTreeMap<String, String>>) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    if let Some(overrides) = overrides {
        env.extend(overrides.clone());
    }
    env
}

fn append_log(log_dir: &Path, entry: &LogLine) {
    let path = log_dir.join(format!("{}.log", entry.name));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", entry.line);
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(unix)]
fn kill_group(pid: u32, mode: KillMode) {
    let signal = match mode {
        KillMode::Graceful => libc::SIGTERM,
        KillMode::Force => libc::SIGKILL,
    };
    unsafe {
        if libc::kill(-(pid as i32), signal) != 0 {
            let _ = libc::kill(pid as i32, signal);
        }
    }
}

#[cfg(windows)]
fn kill_group(pid: u32, mode: KillMode) {
    let mut command = std::process::Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if matches!(mode, KillMode::Force) {
        command.arg("/F");
    }
    let _ = command.output();
}

#[cfg(unix)]
fn signal_name(signal: i32) -> String {
    match signal {
        libc::SIGTERM => "SIGTERM".into(),
        libc::SIGKILL => "SIGKILL".into(),
        libc::SIGINT => "SIGINT".into(),
        libc::SIGHUP => "SIGHUP".into(),
        other => format!("SIG{other}"),
    }
}

#[cfg(unix)]
fn exit_details(status: std::process::ExitStatus) -> (Option<i32>, Option<String>) {
    (status.code(), status.signal().map(signal_name))
}

#[cfg(windows)]
fn exit_details(status: std::process::ExitStatus) -> (Option<i32>, Option<String>) {
    (status.code(), None)
}

#[cfg(unix)]
fn read_process_usage() -> HashMap<u32, (f64, u64)> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pgid=,pcpu=,rss="])
        .env("LC_ALL", "C")
        .output()
    else {
        return HashMap::new();
    };
    let mut usage = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3 {
            continue;
        }
        let (Ok(pgid), Ok(cpu), Ok(rss)) = (
            fields[0].parse::<u32>(),
            fields[1].parse::<f64>(),
            fields[2].parse::<u64>(),
        ) else {
            continue;
        };
        let item = usage.entry(pgid).or_insert((0.0, 0));
        item.0 += cpu;
        item.1 += rss * 1_024;
    }
    usage
}

#[cfg(windows)]
fn read_process_usage() -> HashMap<u32, (f64, u64)> {
    HashMap::new()
}

fn add_process_usage(info: &mut ServiceInfo, usage: &HashMap<u32, (f64, u64)>) {
    if let Some(pid) = info.pid
        && let Some((cpu, memory)) = usage.get(&pid)
    {
        info.cpu_percent = Some(*cpu);
        info.memory_bytes = Some(*memory);
    }
}

fn batch_item(
    name: &str,
    outcome: &str,
    info: Option<ServiceInfo>,
    error: Option<String>,
) -> BatchItemResult {
    BatchItemResult {
        name: name.into(),
        outcome: outcome.into(),
        info,
        reason: None,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::build_service_env;
    use std::collections::BTreeMap;

    #[test]
    fn explicit_service_path_wins() {
        let mut overrides = BTreeMap::new();
        overrides.insert("PATH".into(), "/service/bin:/usr/bin".into());
        assert_eq!(
            build_service_env(Some(&overrides)).get("PATH").unwrap(),
            "/service/bin:/usr/bin"
        );
    }
}
