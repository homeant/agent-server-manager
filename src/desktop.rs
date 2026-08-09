use std::{env, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};

use asvc::paths::Paths;

/// Launch the native Tauri console that is built alongside the CLI.
///
/// Keeping this tiny launcher in the CLI preserves the existing `asvc` entry point while the
/// actual desktop process can own its WebView, tray integrations, and frontend bundle.
pub fn run(_paths: Paths) -> Result<()> {
    let executable = desktop_binary().ok_or_else(|| {
        anyhow::anyhow!(
            "Tauri desktop binary was not found; run `npx --yes @tauri-apps/cli@2.11.4 dev` first"
        )
    })?;
    let status = Command::new(&executable)
        .status()
        .with_context(|| format!("failed to launch {}", executable.display()))?;
    if !status.success() {
        bail!("desktop process exited with {status}");
    }
    Ok(())
}

fn desktop_binary() -> Option<PathBuf> {
    if let Some(path) = env::var_os("ASVC_DESKTOP_BIN").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }

    if let Ok(current_exe) = env::current_exe()
        && let Some(candidate) = current_exe
            .parent()
            .map(|parent| parent.join("asvc-desktop"))
        && candidate.is_file()
    {
        return Some(candidate);
    }

    let tauri_target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src-tauri")
        .join("target");
    ["debug", "release"].into_iter().find_map(|profile| {
        let candidate = tauri_target.join(profile).join("asvc-desktop");
        candidate.is_file().then_some(candidate)
    })
}
