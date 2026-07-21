use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    sync::Arc,
};

#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, anyhow};
use fs2::FileExt;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{Notify, RwLock, mpsc},
};

#[cfg(windows)]
use tokio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

use crate::{model::Request, paths::Paths, supervisor::Supervisor};

pub async fn run(paths: Paths) -> Result<()> {
    fs::create_dir_all(&paths.home)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock_file)?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }

    #[cfg(unix)]
    let listener = {
        if UnixStream::connect(&paths.socket).await.is_ok() {
            return Ok(());
        }
        let _ = fs::remove_file(&paths.socket);
        UnixListener::bind(&paths.socket)
            .with_context(|| format!("无法监听 {}", paths.socket.display()))?
    };
    #[cfg(windows)]
    let listener = {
        if connect_windows(&paths).await.is_ok() {
            return Ok(());
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        fs::write(&paths.endpoint_file, port.to_string())?;
        listener
    };
    fs::write(&paths.pid_file, std::process::id().to_string())?;
    #[cfg(unix)]
    eprintln!(
        "[asvc-daemon] listening on {} (pid {})",
        paths.socket.display(),
        std::process::id()
    );
    #[cfg(windows)]
    eprintln!(
        "[asvc-daemon] listening on 127.0.0.1:{} (pid {})",
        listener.local_addr()?.port(),
        std::process::id()
    );

    let supervisor = Supervisor::new(paths.clone())?;
    let shutdown = Arc::new(Notify::new());
    let signal_shutdown = Arc::clone(&shutdown);
    tokio::spawn(async move {
        wait_for_signal().await;
        signal_shutdown.notify_waiters();
    });

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(handle_connection(
                    stream,
                    Arc::clone(&supervisor),
                    Arc::clone(&shutdown),
                ));
            }
            _ = shutdown.notified() => break,
        }
    }

    supervisor.shutdown().await;
    #[cfg(unix)]
    let _ = fs::remove_file(&paths.socket);
    #[cfg(windows)]
    let _ = fs::remove_file(&paths.endpoint_file);
    let _ = fs::remove_file(&paths.pid_file);
    drop(lock);
    Ok(())
}

async fn handle_connection<S>(stream: S, supervisor: Arc<Supervisor>, shutdown: Arc<Notify>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let attachments = Arc::new(RwLock::new(HashSet::<String>::new()));

    let writer_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if writer.write_all(frame.as_bytes()).await.is_err()
                || writer.write_all(b"\n").await.is_err()
            {
                break;
            }
        }
    });

    let mut events = supervisor.subscribe();
    let event_tx = tx.clone();
    let event_attachments = Arc::clone(&attachments);
    let event_task = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event_attachments.read().await.contains(event.name()) {
                        let _ = event_tx.send(serde_json::to_string(&event).unwrap());
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    while let Ok(Some(line)) = lines.next_line().await {
        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(_) => continue,
        };
        let id = request.id();
        let response =
            handle_request(request, &supervisor, &attachments, Arc::clone(&shutdown)).await;
        let frame = match response {
            Ok(value) => json!({ "id": id, "ok": true, "result": value }),
            Err(error) => json!({ "id": id, "ok": false, "error": error.to_string() }),
        };
        if tx.send(frame.to_string()).is_err() {
            break;
        }
    }

    event_task.abort();
    drop(tx);
    let _ = writer_task.await;
}

#[cfg(windows)]
async fn connect_windows(paths: &Paths) -> std::io::Result<TcpStream> {
    let port = fs::read_to_string(&paths.endpoint_file)?
        .trim()
        .parse::<u16>()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    TcpStream::connect(("127.0.0.1", port)).await
}

async fn handle_request(
    request: Request,
    supervisor: &Arc<Supervisor>,
    attachments: &Arc<RwLock<HashSet<String>>>,
    shutdown: Arc<Notify>,
) -> Result<Value> {
    match request {
        Request::Ping { .. } => value(json!({ "pong": true })),
        Request::List { .. } => value(supervisor.list().await),
        Request::Register { spec, start, .. } => value(supervisor.register(spec, start).await?),
        Request::Start { name, .. } => value(supervisor.start(&name).await?),
        Request::StartAll { .. } => value(supervisor.start_all().await),
        Request::Stop { name, .. } => value(supervisor.stop(&name).await?),
        Request::StopAll { .. } => value(supervisor.stop_all().await),
        Request::Restart { name, .. } => value(supervisor.restart(&name).await?),
        Request::Remove { name, .. } => {
            supervisor.remove(&name).await?;
            value(json!({ "removed": name }))
        }
        Request::RemoveAll { .. } => value(supervisor.remove_all().await),
        Request::Logs { name, lines, .. } => {
            value(supervisor.logs(&name, lines.unwrap_or(200)).await?)
        }
        Request::Attach { name, backlog, .. } => {
            let info = supervisor
                .get(&name)
                .await
                .ok_or_else(|| anyhow!("未知服务: {name}"))?;
            attachments.write().await.insert(name.clone());
            let backlog = supervisor.logs(&name, backlog.unwrap_or(200)).await?;
            value(json!({ "info": info, "backlog": backlog }))
        }
        Request::Detach { name, .. } => {
            attachments.write().await.remove(&name);
            value(json!({ "detached": name }))
        }
        Request::Shutdown { .. } => {
            // Let the response reach the client before the accept loop begins cleanup.
            let notify = Arc::clone(&shutdown);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                notify.notify_waiters();
            });
            value(json!({ "shuttingDown": true }))
        }
    }
}

fn value(value: impl Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
