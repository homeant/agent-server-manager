#[cfg(target_os = "macos")]
use std::{
    collections::HashSet,
    env, fs, io,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread::sleep,
    time::Instant,
};
use std::time::Duration;

use asvc::{
    client::Client,
    config::Config,
    i18n::{self, Locale},
    model::{BatchResult, LogLine, ServiceInfo, ServiceSpec},
    paths::Paths,
};
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tauri::State;

const BUNDLED_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub enum CliInstallState {
    Missing,
    Current,
    Outdated,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum CliInstallSource {
    None,
    Desktop,
    Homebrew,
    Npm,
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliInstallStatus {
    pub supported: bool,
    pub state: CliInstallState,
    pub path: Option<String>,
    pub installed_version: Option<String>,
    pub bundled_version: String,
    pub source: CliInstallSource,
    pub candidates: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonRuntimeStatus {
    pub connected: bool,
    pub version: Option<String>,
    pub bundled_version: String,
    pub current: bool,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopCliManifest {
    path: String,
    version: String,
}

#[cfg(target_os = "macos")]
struct CapturedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

pub struct AppState {
    pub paths: Paths,
}

impl AppState {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }
}

async fn request<T>(paths: Paths, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let mut client = Client::connect(&paths, true)
        .await
        .map_err(|error| format!("{error:#}"))?;
    client
        .request(payload)
        .await
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub async fn daemon_status(state: State<'_, AppState>) -> Result<DaemonRuntimeStatus, String> {
    daemon_runtime_status(&state.paths).await
}

async fn daemon_runtime_status(paths: &Paths) -> Result<DaemonRuntimeStatus, String> {
    let Ok(mut client) = Client::connect(paths, false).await else {
        return Ok(DaemonRuntimeStatus {
            connected: false,
            version: None,
            bundled_version: BUNDLED_CLI_VERSION.to_owned(),
            current: true,
        });
    };
    let ping: Value = client
        .request(json!({ "type": "ping" }))
        .await
        .map_err(|error| format!("{error:#}"))?;
    let version = ping
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(DaemonRuntimeStatus {
        connected: true,
        current: version.as_deref() == Some(BUNDLED_CLI_VERSION),
        version,
        bundled_version: BUNDLED_CLI_VERSION.to_owned(),
    })
}

#[tauri::command]
pub async fn migrate_daemon(
    state: State<'_, AppState>,
) -> Result<DaemonRuntimeStatus, String> {
    let mut running = Vec::new();
    if let Ok(mut client) = Client::connect(&state.paths, false).await {
        let services: Vec<ServiceInfo> = client
            .request(json!({ "type": "list" }))
            .await
            .map_err(|error| format!("Could not read services before daemon migration: {error:#}"))?;
        running.extend(
            services
                .into_iter()
                .filter(|service| {
                    matches!(
                        service.status,
                        asvc::model::ServiceStatus::Running
                            | asvc::model::ServiceStatus::Starting
                    )
                })
                .map(|service| service.spec.name),
        );
        let _: Value = client
            .request(json!({ "type": "shutdown" }))
            .await
            .map_err(|error| format!("Could not stop the previous daemon: {error:#}"))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while Client::connect(&state.paths, false).await.is_ok() {
            if tokio::time::Instant::now() >= deadline {
                return Err("Timed out waiting for the previous daemon to stop.".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let mut client = Client::connect(&state.paths, true)
        .await
        .map_err(|error| format!("Could not start the Desktop daemon: {error:#}"))?;
    let mut restore_errors = Vec::new();
    for name in running {
        if let Err(error) = client
            .request::<ServiceInfo>(json!({ "type": "start", "name": name }))
            .await
        {
            restore_errors.push(format!("{name}: {error:#}"));
        }
    }
    if !restore_errors.is_empty() {
        return Err(format!(
            "The Desktop daemon started, but some services could not be restored: {}",
            restore_errors.join("; ")
        ));
    }
    daemon_runtime_status(&state.paths).await
}

#[tauri::command]
pub fn set_locale(state: State<'_, AppState>, locale: String) -> Result<(), String> {
    let locale = match locale.as_str() {
        "en" => Locale::English,
        "zh" | "zh-CN" => Locale::Chinese,
        _ => return Err(format!("Unsupported locale: {locale}")),
    };

    Config { locale }
        .save(&state.paths)
        .map_err(|error| format!("{error:#}"))?;
    i18n::set_locale(locale);
    Ok(())
}

#[tauri::command]
pub async fn cli_install_status(state: State<'_, AppState>) -> Result<CliInstallStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let paths = state.paths.clone();
        tokio::task::spawn_blocking(move || inspect_cli_installation(&paths))
            .await
            .map_err(|error| format!("CLI inspection task failed: {error}"))?
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Ok(CliInstallStatus {
            supported: false,
            state: CliInstallState::Current,
            path: None,
            installed_version: Some(BUNDLED_CLI_VERSION.to_owned()),
            bundled_version: BUNDLED_CLI_VERSION.to_owned(),
            source: CliInstallSource::Desktop,
            candidates: Vec::new(),
        })
    }
}

#[tauri::command]
pub async fn install_cli(state: State<'_, AppState>) -> Result<CliInstallStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let paths = state.paths.clone();
        tokio::task::spawn_blocking(move || {
            let status = inspect_cli_installation(&paths)?;
            if matches!(status.state, CliInstallState::Current) {
                return Ok(status);
            }
            if matches!(status.state, CliInstallState::Conflict) {
                return Err(format!(
                    "Multiple asvc commands are present in PATH: {}. Remove the extra installations before Desktop can take ownership.",
                    status.candidates.join(", ")
                ));
            }

            let destination = status
                .path
                .as_deref()
                .map(PathBuf::from)
                .ok_or_else(|| "No safe CLI installation directory was found in PATH.".to_owned())?;
            let login_path = user_login_path();
            remove_package_manager_installation(status.source, &destination, &login_path)?;
            let source = bundled_cli_path()?;
            install_cli_macos(&source, &destination)?;
            write_desktop_cli_manifest(&paths, &destination)?;

            let installed = inspect_cli_installation(&paths)?;
            if !matches!(installed.state, CliInstallState::Current) {
                return Err(format!(
                    "The CLI was installed at {}, but the user's PATH still does not resolve to the Desktop version.",
                    destination.display()
                ));
            }
            Ok(installed)
        })
        .await
        .map_err(|error| format!("CLI installation task failed: {error}"))?
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Err(
            "Installing the bundled CLI from the desktop app is currently supported on macOS only."
                .to_owned(),
        )
    }
}

#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg(target_os = "macos")]
fn inspect_cli_installation(paths: &Paths) -> Result<CliInstallStatus, String> {
    let login_path = user_login_path();
    let candidates = find_cli_candidates(&login_path);
    let managed_path = read_desktop_cli_manifest(paths).map(|manifest| manifest.path);
    let resolved = candidates.first().cloned();
    let destination = resolved
        .clone()
        .or_else(|| preferred_install_path(&login_path));
    let installed_version = resolved
        .as_deref()
        .and_then(|path| cli_version(path, &login_path));
    let source = resolved
        .as_deref()
        .map(|path| detect_cli_source(path, managed_path.as_deref()))
        .unwrap_or(CliInstallSource::None);
    let state = if candidates.len() > 1 {
        CliInstallState::Conflict
    } else if resolved.is_none() {
        CliInstallState::Missing
    } else if source == CliInstallSource::Desktop
        && installed_version.as_deref() == Some(BUNDLED_CLI_VERSION)
    {
        CliInstallState::Current
    } else {
        CliInstallState::Outdated
    };

    Ok(CliInstallStatus {
        supported: true,
        state,
        path: destination.map(|path| path.to_string_lossy().into_owned()),
        installed_version,
        bundled_version: BUNDLED_CLI_VERSION.to_owned(),
        source,
        candidates: candidates
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    })
}

#[cfg(target_os = "macos")]
fn user_login_path() -> std::ffi::OsString {
    let inherited = env::var_os("PATH").unwrap_or_default();
    let shell = env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
    let mut command = Command::new(shell);
    // Finder does not inherit the PATH assembled by the user's terminal startup files. Use a
    // bounded interactive login probe so .zprofile/.zshrc (or their shell equivalents) contribute
    // the same PATH that `command -v asvc` sees in a normal terminal.
    command
        .args(["-l", "-i", "-c", "exec /usr/bin/env"])
        .stdin(Stdio::null());
    let Ok(output) = capture_with_timeout(&mut command, Duration::from_secs(3)) else {
        return inherited;
    };
    if !output.status.success() {
        return inherited;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("PATH="))
        .map(std::ffi::OsString::from)
        .unwrap_or(inherited)
}

#[cfg(target_os = "macos")]
fn find_cli_candidates(path: &std::ffi::OsStr) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    env::split_paths(path)
        .map(|directory| directory.join("asvc"))
        .filter(|candidate| is_executable_file(candidate))
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

#[cfg(target_os = "macos")]
fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn preferred_install_path(path: &std::ffi::OsStr) -> Option<PathBuf> {
    let home = asvc::paths::user_home();
    let preferred = [
        home.join(".local/bin"),
        home.join("bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ];
    let directories: Vec<PathBuf> = env::split_paths(path).collect();
    preferred
        .into_iter()
        .filter(|candidate| directories.contains(candidate))
        .min_by_key(|candidate| {
            directories
                .iter()
                .position(|directory| directory == candidate)
                .unwrap_or(usize::MAX)
        })
        .map(|directory| directory.join("asvc"))
}

#[cfg(target_os = "macos")]
fn cli_version(path: &Path, login_path: &std::ffi::OsStr) -> Option<String> {
    let mut command = Command::new(path);
    command
        .arg("--version")
        .env("PATH", login_path)
        .stdin(Stdio::null());
    let output = capture_with_timeout(&mut command, Duration::from_secs(3)).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|version| !version.is_empty())
}

#[cfg(target_os = "macos")]
fn detect_cli_source(path: &Path, managed_path: Option<&str>) -> CliInstallSource {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = canonical.to_string_lossy();
    if value.contains("/Cellar/asvc/") || value.contains("/homebrew/Cellar/asvc/") {
        CliInstallSource::Homebrew
    } else if value.contains("/node_modules/@homeant/asvc/") {
        CliInstallSource::Npm
    } else if managed_path == Some(path.to_string_lossy().as_ref()) {
        CliInstallSource::Desktop
    } else {
        CliInstallSource::Unknown
    }
}

#[cfg(target_os = "macos")]
fn remove_package_manager_installation(
    source: CliInstallSource,
    destination: &Path,
    login_path: &std::ffi::OsStr,
) -> Result<(), String> {
    let (manager, arguments) = match source {
        CliInstallSource::Homebrew => (
            find_command(login_path, "brew")
                .ok_or_else(|| "The Homebrew asvc installation was found, but brew is not in PATH.".to_owned())?,
            vec!["uninstall", "--formula", "asvc"],
        ),
        CliInstallSource::Npm => (
            find_command(login_path, "npm")
                .ok_or_else(|| "The npm asvc installation was found, but npm is not in PATH.".to_owned())?,
            vec!["uninstall", "--global", "@homeant/asvc"],
        ),
        _ => return Ok(()),
    };
    let mut command = Command::new(&manager);
    command
        .args(arguments)
        .env("PATH", login_path)
        .stdin(Stdio::null());
    let output = capture_with_timeout(&mut command, Duration::from_secs(120))
        .map_err(|error| format!("Could not run {}: {error}", manager.display()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("{} could not remove the existing asvc installation.", manager.display())
        } else {
            detail
        });
    }
    if destination.exists() {
        return Err(format!(
            "The package manager completed, but {} still exists.",
            destination.display()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn find_command(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(target_os = "macos")]
fn capture_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<CapturedOutput> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "command timed out"));
        }
        sleep(Duration::from_millis(20));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_end(&mut stdout)?;
    }
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_end(&mut stderr)?;
    }
    Ok(CapturedOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(target_os = "macos")]
fn desktop_cli_manifest_path(paths: &Paths) -> PathBuf {
    paths.home.join("desktop-cli.json")
}

#[cfg(target_os = "macos")]
fn read_desktop_cli_manifest(paths: &Paths) -> Option<DesktopCliManifest> {
    serde_json::from_slice(&fs::read(desktop_cli_manifest_path(paths)).ok()?).ok()
}

#[cfg(target_os = "macos")]
fn write_desktop_cli_manifest(paths: &Paths, path: &Path) -> Result<(), String> {
    fs::create_dir_all(&paths.home).map_err(|error| format!("Cannot create asvc home: {error}"))?;
    let manifest = DesktopCliManifest {
        path: path.to_string_lossy().into_owned(),
        version: BUNDLED_CLI_VERSION.to_owned(),
    };
    fs::write(
        desktop_cli_manifest_path(paths),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot record the Desktop CLI installation: {error}"))
}

#[cfg(target_os = "macos")]
fn bundled_cli_path() -> Result<PathBuf, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("Cannot locate Asvc: {error}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| format!("Cannot locate Asvc beside {}", executable.display()))?;
    let candidate = directory.join("asvc");
    if candidate.is_file() {
        return Ok(candidate);
    }
    Err(format!(
        "The desktop bundle does not contain its asvc CLI sidecar: {}",
        candidate.display()
    ))
}

#[cfg(target_os = "macos")]
fn install_cli_macos(source: &Path, destination: &Path) -> Result<(), String> {
    match install_cli_direct(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            install_cli_with_authorization(source, destination)
        }
        Err(error) => Err(format!(
            "Could not install asvc at {}: {error}",
            destination.display()
        )),
    }
}

#[cfg(target_os = "macos")]
fn install_cli_direct(source: &Path, destination: &Path) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "CLI destination has no parent")
    })?;
    fs::create_dir_all(parent)?;

    let temporary = parent.join(format!(".asvc-install-{}", std::process::id()));
    let result = (|| {
        let _ = fs::remove_file(&temporary);
        fs::copy(source, &temporary)?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))?;
        fs::rename(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(target_os = "macos")]
fn install_cli_with_authorization(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "CLI destination has no parent".to_owned())?;
    let shell_command = format!(
        "/bin/mkdir -p {} && /usr/bin/install -m 755 {} {}",
        shell_quote(parent),
        shell_quote(source),
        shell_quote(destination)
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_quote(&shell_command)
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", script.as_str()])
        .output()
        .map_err(|error| format!("Could not request macOS authorization: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if details.is_empty() {
        Err("CLI installation was cancelled or failed.".to_owned())
    } else {
        Err(format!("CLI installation failed: {details}"))
    }
}

#[cfg(target_os = "macos")]
fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn applescript_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[tauri::command]
pub async fn get_services(state: State<'_, AppState>) -> Result<Vec<ServiceInfo>, String> {
    request(state.paths.clone(), json!({ "type": "list" })).await
}

#[tauri::command]
pub async fn get_logs(
    state: State<'_, AppState>,
    name: String,
    lines: usize,
) -> Result<Vec<LogLine>, String> {
    request(
        state.paths.clone(),
        json!({ "type": "logs", "name": name, "lines": lines }),
    )
    .await
}

#[tauri::command]
pub async fn start_service(
    state: State<'_, AppState>,
    name: String,
) -> Result<ServiceInfo, String> {
    request(
        state.paths.clone(),
        json!({ "type": "start", "name": name }),
    )
    .await
}

#[tauri::command]
pub async fn stop_service(state: State<'_, AppState>, name: String) -> Result<ServiceInfo, String> {
    request(state.paths.clone(), json!({ "type": "stop", "name": name })).await
}

#[tauri::command]
pub async fn restart_service(
    state: State<'_, AppState>,
    name: String,
) -> Result<ServiceInfo, String> {
    request(
        state.paths.clone(),
        json!({ "type": "restart", "name": name }),
    )
    .await
}

#[tauri::command]
pub async fn start_all(state: State<'_, AppState>) -> Result<BatchResult, String> {
    request(state.paths.clone(), json!({ "type": "startAll" })).await
}

#[tauri::command]
pub async fn stop_all(state: State<'_, AppState>) -> Result<BatchResult, String> {
    request(state.paths.clone(), json!({ "type": "stopAll" })).await
}

#[tauri::command]
pub async fn remove_service(state: State<'_, AppState>, name: String) -> Result<Value, String> {
    request(
        state.paths.clone(),
        json!({ "type": "remove", "name": name }),
    )
    .await
}

#[tauri::command]
pub async fn register_service(
    state: State<'_, AppState>,
    spec: ServiceSpec,
) -> Result<ServiceInfo, String> {
    request(
        state.paths.clone(),
        json!({ "type": "register", "spec": spec, "start": true }),
    )
    .await
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ID: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = env::temp_dir().join(format!(
                "asvc-desktop-cli-test-{}-{}",
                std::process::id(),
                TEST_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn executable(&self, relative: &str, version: &str) -> PathBuf {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn cli_candidates_follow_path_order_and_are_deduplicated() {
        let fixture = Fixture::new();
        let first = fixture.executable("first/asvc", "0.4.4");
        let second = fixture.executable("second/asvc", "0.4.5");
        let first_dir = first.parent().unwrap();
        let second_dir = second.parent().unwrap();
        let path = env::join_paths([first_dir, second_dir, first_dir]).unwrap();

        assert_eq!(find_cli_candidates(&path), vec![first.clone(), second]);
        assert_eq!(cli_version(&first, &path).as_deref(), Some("0.4.4"));
    }

    #[test]
    fn detects_homebrew_and_npm_managed_commands() {
        let fixture = Fixture::new();
        let brew_binary = fixture.executable("Cellar/asvc/0.4.4/bin/asvc", "0.4.4");
        let brew_link = fixture.root.join("brew-bin/asvc");
        fs::create_dir_all(brew_link.parent().unwrap()).unwrap();
        symlink(&brew_binary, &brew_link).unwrap();
        assert_eq!(
            detect_cli_source(&brew_link, None),
            CliInstallSource::Homebrew
        );

        let npm_binary = fixture.executable(
            "lib/node_modules/@homeant/asvc/npm/bin/asvc.js",
            "0.4.4",
        );
        let npm_link = fixture.root.join("npm-bin/asvc");
        fs::create_dir_all(npm_link.parent().unwrap()).unwrap();
        symlink(&npm_binary, &npm_link).unwrap();
        assert_eq!(detect_cli_source(&npm_link, None), CliInstallSource::Npm);
    }

    #[test]
    fn preferred_install_location_must_already_be_in_path() {
        let fixture = Fixture::new();
        let local = asvc::paths::user_home().join(".local/bin");
        let other = fixture.root.join("bin");
        let path = env::join_paths([&other, &local, Path::new("/usr/local/bin")]).unwrap();
        assert_eq!(preferred_install_path(&path), Some(local.join("asvc")));

        let no_safe_directory = env::join_paths([other]).unwrap();
        assert_eq!(preferred_install_path(&no_safe_directory), None);
    }
}
