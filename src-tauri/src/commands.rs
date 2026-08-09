use asvc::{
    client::Client,
    config::Config,
    i18n::{self, Locale},
    model::{BatchResult, LogLine, ServiceInfo, ServiceSpec},
    paths::Paths,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tauri::State;

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
    request(
        state.paths.clone(),
        json!({ "type": "stop", "name": name }),
    )
    .await
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
pub async fn remove_service(
    state: State<'_, AppState>,
    name: String,
) -> Result<Value, String> {
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
