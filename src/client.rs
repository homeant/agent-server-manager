use std::{collections::VecDeque, fs, time::Duration};

#[cfg(unix)]
use std::{
    fs::OpenOptions,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf},
    time::{sleep, timeout},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(windows)]
type PlatformStream = TcpStream;
#[cfg(unix)]
type PlatformStream = UnixStream;

use crate::{model::Event, paths::Paths};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const BATCH_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub struct Client {
    lines: Lines<BufReader<ReadHalf<PlatformStream>>>,
    writer: WriteHalf<PlatformStream>,
    next_id: u64,
    pending_events: VecDeque<Event>,
}

impl Client {
    pub async fn connect(paths: &Paths, auto_spawn: bool) -> Result<Self> {
        match connect_once(paths).await {
            Ok(stream) => Ok(Self::from_stream(stream)),
            Err(error) if !auto_spawn => Err(error).context("daemon 未运行"),
            Err(_) => {
                spawn_daemon(paths)?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    match connect_once(paths).await {
                        Ok(stream) => return Ok(Self::from_stream(stream)),
                        Err(error) if tokio::time::Instant::now() >= deadline => {
                            return Err(error).context("等待 daemon 启动超时");
                        }
                        Err(_) => sleep(Duration::from_millis(100)).await,
                    }
                }
            }
        }
    }

    fn from_stream(stream: PlatformStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            lines: BufReader::new(reader).lines(),
            writer,
            next_id: 1,
            pending_events: VecDeque::new(),
        }
    }

    pub async fn request<T: DeserializeOwned>(&mut self, mut request: Value) -> Result<T> {
        let id = self.next_id;
        self.next_id += 1;
        request
            .as_object_mut()
            .ok_or_else(|| anyhow!("请求必须是 JSON object"))?
            .insert("id".into(), json!(id));
        let request_type = request
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        self.writer
            .write_all(format!("{}\n", serde_json::to_string(&request)?).as_bytes())
            .await?;
        self.writer.flush().await?;

        let limit = if matches!(request_type.as_str(), "startAll" | "stopAll" | "removeAll") {
            BATCH_TIMEOUT
        } else {
            REQUEST_TIMEOUT
        };
        timeout(limit, async {
            loop {
                let line = self
                    .lines
                    .next_line()
                    .await?
                    .ok_or_else(|| anyhow!("连接已关闭"))?;
                let value: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if value.get("event").is_some() {
                    if let Ok(event) = serde_json::from_value(value) {
                        self.pending_events.push_back(event);
                    }
                    continue;
                }
                if value.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if value.get("ok").and_then(Value::as_bool) == Some(true) {
                    return serde_json::from_value(
                        value.get("result").cloned().unwrap_or(Value::Null),
                    )
                    .map_err(Into::into);
                }
                return Err(anyhow!(
                    "{}",
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("daemon 请求失败")
                ));
            }
        })
        .await
        .map_err(|_| anyhow!("请求超时（{request_type}）：daemon 无响应"))?
    }

    pub async fn next_event(&mut self) -> Result<Event> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(event);
        }
        loop {
            let line = self
                .lines
                .next_line()
                .await?
                .ok_or_else(|| anyhow!("连接已关闭"))?;
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if value.get("event").is_some() {
                return Ok(serde_json::from_value(value)?);
            }
        }
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = Event> + '_ {
        self.pending_events.drain(..)
    }
}

#[cfg(unix)]
fn spawn_daemon(paths: &Paths) -> Result<()> {
    fs::create_dir_all(&paths.home)?;
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.daemon_log)?;
    let stderr = stdout.try_clone()?;
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().context("无法启动 daemon")?;
    // daemon owns its own session; dropping Child does not terminate it.
    drop(child);
    Ok(())
}

#[cfg(windows)]
fn spawn_daemon(paths: &Paths) -> Result<()> {
    use std::{mem::size_of, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CreateProcessW, DETACHED_PROCESS,
            PROCESS_INFORMATION, STARTUPINFOW,
        },
    };

    fs::create_dir_all(&paths.home)?;
    let executable = std::env::current_exe()?;
    let application: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut command_line = vec![b'"' as u16];
    command_line.extend(executable.as_os_str().encode_wide());
    command_line.push(b'"' as u16);
    command_line.extend(" __daemon".encode_utf16());
    command_line.push(0);
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // Command::spawn can leave the calling process's captured stdout/stderr handles open on
    // Windows. A daemon holding those handles prevents agents and test runners from observing
    // EOF after the CLI exits. CreateProcessW with bInheritHandles=FALSE gives the detached
    // daemon no caller-owned pipes while still inheriting the caller's environment.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error()).context("无法启动 daemon");
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

#[cfg(unix)]
async fn connect_once(paths: &Paths) -> std::io::Result<PlatformStream> {
    UnixStream::connect(&paths.socket).await
}

#[cfg(windows)]
async fn connect_once(paths: &Paths) -> std::io::Result<PlatformStream> {
    let port = fs::read_to_string(&paths.endpoint_file)?
        .trim()
        .parse::<u16>()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    TcpStream::connect(("127.0.0.1", port)).await
}
