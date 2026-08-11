use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    thread::sleep,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

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
        let output = self.run_unchecked(args);
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

    fn run_unchecked(&self, args: &[&str]) -> Output {
        let mut command = Command::new(self.binary);
        configure_command(&mut command, &self.home, &self.asvc_home, &base_path());
        command.args(args).output().unwrap()
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
    let missing_status_name = fixture.run_unchecked(&["status"]);
    assert!(!missing_status_name.status.success());
    let missing_status_name_stderr = String::from_utf8_lossy(&missing_status_name.stderr);
    assert!(missing_status_name_stderr.contains("<NAME>"));
    assert!(missing_status_name_stderr.contains("status <NAME>"));
    assert!(
        fixture
            .stdout(&["completion", "bash"])
            .contains("restart|logs|status|info")
    );

    assert!(
        fixture
            .stdout(&["daemon", "status"])
            .contains("daemon is not running")
    );
    assert!(!fixture.asvc_home.join("daemon.pid").exists());

    assert!(fixture.stdout(&["list"]).contains("no services"));
    let daemon_status = fixture.stdout(&["daemon", "status"]);
    assert!(daemon_status.contains("daemon is running"));
    assert!(daemon_status.contains(env!("CARGO_PKG_VERSION")));
    let daemon_pid = fs::read_to_string(fixture.asvc_home.join("daemon.pid")).unwrap();
    assert_daemon_process(daemon_pid.trim(), fixture.binary);

    let cwd = fixture.root.to_string_lossy();
    assert!(
        fixture
            .stdout(&["start", "smoke", "-c", long_command(), "--cwd", &cwd, "-d",])
            .contains("running")
    );
    assert!(fixture.stdout(&["list"]).contains("smoke"));
    let status = fixture.stdout(&["status", "smoke"]);
    assert!(status.contains("smoke → running"));
    assert!(status.contains("pid "));
    assert!(status.contains("uptime "));
    let info = fixture.stdout(&["info", "smoke"]);
    assert!(info.contains("Name: smoke"));
    assert!(info.contains("Status: running"));
    assert!(info.contains(&format!("Working directory: {cwd}")));
    assert!(info.contains(&format!("Command: {}", long_command())));
    assert!(info.contains("Auto restart: no"));
    assert!(
        fixture
            .stdout(&["restart", "smoke"])
            .contains("startup verified for 1s")
    );
    let unknown = fixture.run_unchecked(&["info", "missing"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown service: missing"));
    assert!(fixture.stdout(&["stop", "smoke"]).contains("stopped"));
    let stopped_status = fixture.run_unchecked(&["status", "smoke"]);
    assert!(!stopped_status.status.success());
    assert!(String::from_utf8_lossy(&stopped_status.stdout).contains("smoke → stopped"));
    assert!(
        fixture
            .stdout(&["daemon", "stop"])
            .contains("daemon stopped")
    );

    wait_until_missing(&fixture.asvc_home.join("daemon.pid"));
}

#[test]
fn switches_locale_and_updates_a_running_daemon() {
    let fixture = Fixture::new();

    assert_eq!(fixture.stdout(&["config", "get", "locale"]).trim(), "en");
    assert!(
        fixture
            .stdout(&["daemon", "status"])
            .contains("daemon is not running")
    );
    assert!(fixture.stdout(&["list"]).contains("no services"));

    assert!(
        fixture
            .stdout(&["config", "set", "locale", "zh-CN"])
            .contains("语言已更新: zh-CN")
    );
    assert_eq!(fixture.stdout(&["config", "get", "locale"]).trim(), "zh-CN");
    assert!(
        fixture
            .stdout(&["daemon", "status"])
            .contains("daemon 运行中")
    );
    let unknown = fixture.run_unchecked(&["info", "missing"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("未知服务: missing"));

    assert!(
        fixture
            .stdout(&["config", "set", "locale", "en"])
            .contains("Locale updated: en")
    );
    let unknown = fixture.run_unchecked(&["info", "missing"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown service: missing"));

    let invalid = fixture.run_unchecked(&["config", "set", "local", "zh-CN"]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid value 'local'"));

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.asvc_home.join("config.json")).unwrap())
            .unwrap();
    assert_eq!(config["locale"], "en");
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

#[test]
fn installs_syncs_and_uninstalls_managed_agent_skills() {
    let fixture = Fixture::new();
    let skill_name = "local-dev-server-manager";
    let bundled = include_str!("../skill/asvc-service-manager/SKILL.md").replacen(
        "name: asvc-service-manager",
        &format!("name: {skill_name}"),
        1,
    );
    let codex_dir = fixture.home.join(".agents/skills").join(skill_name);
    let claude_dir = fixture.home.join(".claude/skills").join(skill_name);
    let managed_dir = fixture.asvc_home.join("skills/asvc");
    let manifest_path = managed_dir.join("install.json");

    assert!(
        fixture
            .stdout(&["skill", "status"])
            .contains("Managed status: not installed")
    );
    let installed = fixture.stdout(&[
        "skill", "install", "--name", skill_name, "--target", "codex", "--target", "claude",
    ]);
    assert!(installed.contains("Codex skill installed"));
    assert!(installed.contains("Claude Code skill installed"));
    assert_eq!(
        fs::read_to_string(codex_dir.join("SKILL.md")).unwrap(),
        bundled.as_str()
    );
    assert_eq!(
        fs::read_to_string(claude_dir.join("SKILL.md")).unwrap(),
        bundled.as_str()
    );
    assert_skill_install_type(&codex_dir, &managed_dir);
    assert_skill_install_type(&claude_dir, &managed_dir);

    let status = fixture.stdout(&["skill", "status"]);
    assert!(status.contains(&format!(
        "Bundled skill version: {}",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(status.contains(&format!("Skill name: {skill_name}")));
    assert!(status.contains("Codex: current"));
    assert!(status.contains("Claude Code: current"));

    let rename_refused =
        fixture.run_unchecked(&["skill", "install", "--name", "another-skill-name"]);
    assert!(!rename_refused.status.success());
    assert!(
        String::from_utf8_lossy(&rename_refused.stderr)
            .contains("uninstall it before changing its name")
    );

    let confirmation_required = fixture.run_unchecked(&["skill", "uninstall", "--target", "codex"]);
    assert!(!confirmation_required.status.success());
    assert!(String::from_utf8_lossy(&confirmation_required.stderr).contains("--yes"));
    assert!(path_lexists(&codex_dir));

    // Simulate a previously managed skill version. A normal finite command
    // should update it from the skill embedded in the new CLI.
    let old_skill = format!("---\nname: {skill_name}\ndescription: old managed copy\n---\nold\n");
    fs::write(managed_dir.join("SKILL.md"), &old_skill).unwrap();
    #[cfg(windows)]
    {
        fs::write(codex_dir.join("SKILL.md"), &old_skill).unwrap();
        fs::write(claude_dir.join("SKILL.md"), &old_skill).unwrap();
    }
    let old_sha = format!("{:x}", Sha256::digest(old_skill.as_bytes()));
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["bundledSha256"] = old_sha.clone().into();
    for target in manifest["targets"].as_array_mut().unwrap() {
        target["installedSha256"] = old_sha.clone().into();
    }
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let synced = fixture.run(&["daemon", "status"]);
    assert!(
        String::from_utf8_lossy(&synced.stderr)
            .contains("Skill automatically synchronized with asvc")
    );
    assert_eq!(
        fs::read_to_string(codex_dir.join("SKILL.md")).unwrap(),
        bundled.as_str()
    );
    assert_eq!(
        fs::read_to_string(claude_dir.join("SKILL.md")).unwrap(),
        bundled.as_str()
    );

    // User edits are never overwritten or removed.
    fs::write(codex_dir.join("SKILL.md"), "user-owned edit\n").unwrap();
    let skipped = fixture.run(&["daemon", "status"]);
    assert!(
        String::from_utf8_lossy(&skipped.stderr)
            .contains("skipped automatic skill synchronization")
    );
    assert_eq!(
        fs::read_to_string(codex_dir.join("SKILL.md")).unwrap(),
        "user-owned edit\n"
    );
    let refused = fixture.run_unchecked(&["skill", "install", "--target", "codex"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--yes"));
    assert_eq!(
        fs::read_to_string(codex_dir.join("SKILL.md")).unwrap(),
        "user-owned edit\n"
    );
    assert!(
        fixture
            .stdout(&["skill", "install", "--target", "codex", "--yes",])
            .contains("Codex skill installed")
    );
    assert_eq!(
        fs::read_to_string(codex_dir.join("SKILL.md")).unwrap(),
        bundled.as_str()
    );

    let removed = fixture.stdout(&[
        "skill",
        "uninstall",
        "--target",
        "codex",
        "--target",
        "claude",
        "--yes",
    ]);
    assert!(removed.contains("Codex skill uninstalled"));
    assert!(removed.contains("Claude Code skill uninstalled"));
    assert!(!path_lexists(&codex_dir));
    assert!(!path_lexists(&claude_dir));
    assert!(!manifest_path.exists());
}

#[test]
fn refuses_to_replace_an_unmanaged_skill() {
    let fixture = Fixture::new();
    let skill_file = fixture
        .home
        .join(".agents/skills/asvc-service-manager/SKILL.md");
    fs::create_dir_all(skill_file.parent().unwrap()).unwrap();
    fs::write(&skill_file, "third-party skill\n").unwrap();

    let output = fixture.run_unchecked(&["skill", "install"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not managed by asvc"));
    assert_eq!(
        fs::read_to_string(&skill_file).unwrap(),
        "third-party skill\n"
    );
    assert!(!fixture.asvc_home.join("skills/asvc/install.json").exists());
}

#[test]
fn preserves_the_legacy_asvc_skill_name() {
    let fixture = Fixture::new();
    let invalid = fixture.run_unchecked(&["skill", "install", "--name", "Not Valid"]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("may contain only"));

    fixture.stdout(&["skill", "install", "--name", "asvc"]);
    let legacy_dir = fixture.home.join(".agents/skills/asvc");
    assert!(
        fs::read_to_string(legacy_dir.join("SKILL.md"))
            .unwrap()
            .starts_with("---\nname: asvc\n")
    );
    assert!(
        fixture
            .stdout(&["skill", "status"])
            .contains("Skill name: asvc")
    );
    fixture.stdout(&["skill", "uninstall", "--yes"]);
    assert!(!path_lexists(&legacy_dir));
}

fn configure_command(command: &mut Command, home: &Path, asvc_home: &Path, path: &str) {
    command.env_clear().envs(test_env(home, asvc_home, path));
}

#[cfg(unix)]
fn assert_skill_install_type(path: &Path, managed_dir: &Path) {
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(path).unwrap(), managed_dir);
}

#[cfg(windows)]
fn assert_skill_install_type(path: &Path, _managed_dir: &Path) {
    assert!(fs::symlink_metadata(path).unwrap().is_dir());
    assert!(!fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}

fn path_lexists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
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
