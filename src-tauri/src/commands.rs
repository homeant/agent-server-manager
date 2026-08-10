use std::path::Path;

#[cfg(target_os = "macos")]
use std::{fs, io, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

use asvc::{
    client::Client,
    config::Config,
    i18n::{self, Locale},
    model::{BatchResult, LogLine, ServiceInfo, ServiceSpec},
    paths::Paths,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tauri::State;

#[cfg(target_os = "macos")]
const CLI_INSTALL_PATH: &str = "/usr/local/bin/asvc";

#[derive(Debug, Serialize)]
pub struct CliInstallStatus {
    pub supported: bool,
    pub installed: bool,
    pub path: String,
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
pub async fn daemon_status(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(Client::connect(&state.paths, false).await.is_ok())
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
pub fn cli_install_status() -> Result<CliInstallStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let path = Path::new(CLI_INSTALL_PATH);
        Ok(CliInstallStatus {
            supported: true,
            installed: path.is_file(),
            path: CLI_INSTALL_PATH.to_owned(),
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(CliInstallStatus {
            supported: false,
            installed: false,
            path: String::new(),
        })
    }
}

#[tauri::command]
pub fn install_cli() -> Result<CliInstallStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let source = bundled_cli_path()?;
        install_cli_macos(&source, Path::new(CLI_INSTALL_PATH))?;
        cli_install_status()
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(
            "Installing the bundled CLI from the desktop app is currently supported on macOS only."
                .to_owned(),
        )
    }
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
