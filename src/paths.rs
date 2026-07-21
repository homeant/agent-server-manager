use std::{env, path::PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    #[cfg(unix)]
    pub socket: PathBuf,
    #[cfg(windows)]
    pub endpoint_file: PathBuf,
    pub pid_file: PathBuf,
    pub lock_file: PathBuf,
    pub registry: PathBuf,
    pub log_dir: PathBuf,
    pub daemon_log: PathBuf,
}

impl Paths {
    pub fn discover() -> Self {
        let home = env::var_os("ASVC_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home().join(".asvc"));
        Self {
            #[cfg(unix)]
            socket: env::var_os("ASVC_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("daemon.sock")),
            #[cfg(windows)]
            endpoint_file: home.join("daemon.port"),
            pid_file: home.join("daemon.pid"),
            lock_file: home.join("daemon.lock"),
            registry: home.join("registry.json"),
            log_dir: home.join("logs"),
            daemon_log: home.join("daemon.log"),
            home,
        }
    }
}

pub fn user_home() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
