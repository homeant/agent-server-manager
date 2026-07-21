use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    thread::sleep,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    asvc_home: PathBuf,
    binary: &'static str,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = fixture_base().join(format!(
            "asvc-rust-e2e-{}-{nonce}-{}",
            std::process::id(),
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed),
        ));
        let home = root.join("home");
        let asvc_home = home.join(".asvc");
        fs::create_dir_all(&home).unwrap();
        Self {
            root,
            home,
            asvc_home,
            binary: env!("CARGO_BIN_EXE_asvc"),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_path(args, &base_path())
    }

    fn run_with_path(&self, args: &[&str], path: &str) -> Output {
        let mut command = Command::new(self.binary);
        configure_command(&mut command, &self.home, &self.asvc_home, path);
        let output = command.args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{} {}\nstdout:\n{}\nstderr:\n{}",
            self.binary,
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8(self.run(args).stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new(self.binary)
            .args(["daemon", "stop"])
            .envs(test_env(&self.home, &self.asvc_home, &base_path()))
            .output();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn standalone_cli_and_daemon_work_without_node_or_version_manager() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.stdout(&["--version"]).trim(),
        env!("CARGO_PKG_VERSION")
    );

    assert!(
        fixture
            .stdout(&["daemon", "status"])
            .contains("daemon 未运行")
    );
    assert!(!fixture.asvc_home.join("daemon.pid").exists());

    assert!(fixture.stdout(&["list"]).contains("暂无服务"));
    assert!(
        fixture
            .stdout(&["daemon", "status"])
            .contains("daemon 运行中")
    );
    let daemon_pid = fs::read_to_string(fixture.asvc_home.join("daemon.pid")).unwrap();
    assert_daemon_process(daemon_pid.trim(), fixture.binary);

    let cwd = fixture.root.to_string_lossy();
    assert!(
        fixture
            .stdout(&["start", "smoke", "-c", long_command(), "--cwd", &cwd, "-d",])
            .contains("running")
    );
    assert!(fixture.stdout(&["list"]).contains("smoke"));
    assert!(fixture.stdout(&["stop", "smoke"]).contains("stopped"));
    assert!(
        fixture
            .stdout(&["daemon", "stop"])
            .contains("daemon 已停止")
    );

    wait_until_missing(&fixture.asvc_home.join("daemon.pid"));
}

#[test]
fn reads_the_existing_typescript_registry_format() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.asvc_home).unwrap();
    let legacy = serde_json::json!([{
        "name": "legacy",
        "command": long_command(),
        "cwd": fixture.root,
        "env": { "EXAMPLE": "yes" },
        "port": 3456,
        "autorestart": false
    }]);
    fs::write(
        fixture.asvc_home.join("registry.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    let list = fixture.stdout(&["list"]);
    assert!(list.contains("legacy"));
    assert!(list.contains("3456"));
}

#[test]
fn registration_captures_the_calling_users_path_for_later_restarts() {
    let fixture = Fixture::new();
    let bin_dir = fixture.root.join("user-bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let probe = bin_dir.join(probe_filename());
    fs::write(&probe, probe_script()).unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&probe).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&probe, permissions).unwrap();
    }

    let caller_path = path_with(&bin_dir);
    let cwd = fixture.root.to_string_lossy();
    assert!(
        String::from_utf8(
            fixture
                .run_with_path(
                    &[
                        "start",
                        "path-probe",
                        "-c",
                        probe_command(),
                        "--cwd",
                        &cwd,
                        "-d",
                    ],
                    &caller_path,
                )
                .stdout,
        )
        .unwrap()
        .contains("running")
    );
    assert!(
        fixture
            .stdout(&["logs", "path-probe"])
            .contains("inherited-user-path")
    );
    fixture.run(&["stop", "path-probe"]);

    // restart comes from a minimal PATH, but the registered service keeps the
    // caller PATH captured when its definition was created.
    assert!(
        fixture
            .stdout(&["restart", "path-probe"])
            .contains("running")
    );
    fixture.run(&["stop", "path-probe"]);

    let registry: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.asvc_home.join("registry.json")).unwrap())
            .unwrap();
    assert_eq!(
        registry[0]["env"]["PATH"].as_str(),
        Some(caller_path.as_str())
    );
}

fn configure_command(command: &mut Command, home: &Path, asvc_home: &Path, path: &str) {
    command.env_clear().envs(test_env(home, asvc_home, path));
}

#[cfg(unix)]
fn test_env(home: &Path, asvc_home: &Path, path: &str) -> Vec<(String, String)> {
    vec![
        ("HOME".into(), home.to_string_lossy().into_owned()),
        ("ASVC_HOME".into(), asvc_home.to_string_lossy().into_owned()),
        ("PATH".into(), path.into()),
    ]
}

#[cfg(windows)]
fn test_env(home: &Path, asvc_home: &Path, path: &str) -> Vec<(String, String)> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    vec![
        ("HOME".into(), home.to_string_lossy().into_owned()),
        ("USERPROFILE".into(), home.to_string_lossy().into_owned()),
        ("ASVC_HOME".into(), asvc_home.to_string_lossy().into_owned()),
        ("PATH".into(), path.into()),
        ("SystemRoot".into(), system_root),
        ("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()),
    ]
}

#[cfg(unix)]
fn fixture_base() -> PathBuf {
    // Keep Unix-domain socket paths below the macOS SUN_LEN limit.
    PathBuf::from("/tmp")
}

#[cfg(windows)]
fn fixture_base() -> PathBuf {
    std::env::temp_dir()
}

#[cfg(unix)]
fn base_path() -> String {
    std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")])
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[cfg(windows)]
fn base_path() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    std::env::join_paths([
        PathBuf::from(format!(r"{root}\System32")),
        PathBuf::from(root),
    ])
    .unwrap()
    .to_string_lossy()
    .into_owned()
}

fn path_with(entry: &Path) -> String {
    let mut paths = vec![entry.to_path_buf()];
    #[cfg(unix)]
    paths.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        paths.extend([
            PathBuf::from(format!(r"{root}\System32")),
            PathBuf::from(root),
        ]);
    }
    std::env::join_paths(paths)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[cfg(unix)]
fn long_command() -> &'static str {
    "/bin/sleep 30"
}

#[cfg(windows)]
fn long_command() -> &'static str {
    "ping.exe -n 31 127.0.0.1 >NUL"
}

#[cfg(unix)]
fn assert_daemon_process(pid: &str, binary: &str) {
    let output = Command::new("/bin/ps")
        .args(["-p", pid, "-o", "command="])
        .output()
        .unwrap();
    let command = String::from_utf8_lossy(&output.stdout);
    assert!(command.contains(binary));
    assert!(command.contains("__daemon"));
}

#[cfg(windows)]
fn assert_daemon_process(pid: &str, _binary: &str) {
    let output = Command::new("tasklist.exe")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("asvc.exe"));
}

#[cfg(unix)]
fn probe_filename() -> &'static str {
    "asvc-path-probe"
}

#[cfg(windows)]
fn probe_filename() -> &'static str {
    "asvc-path-probe.cmd"
}

#[cfg(unix)]
fn probe_script() -> &'static str {
    "#!/bin/sh\necho inherited-user-path\nsleep 30\n"
}

#[cfg(windows)]
fn probe_script() -> &'static str {
    "@echo off\r\necho inherited-user-path\r\nping.exe -n 31 127.0.0.1 >NUL\r\n"
}

#[cfg(unix)]
fn probe_command() -> &'static str {
    "asvc-path-probe"
}

#[cfg(windows)]
fn probe_command() -> &'static str {
    "asvc-path-probe.cmd"
}

fn wait_until_missing(path: &Path) {
    for _ in 0..50 {
        if !path.exists() {
            return;
        }
        sleep(Duration::from_millis(20));
    }
    panic!("{} was not removed", path.display());
}
