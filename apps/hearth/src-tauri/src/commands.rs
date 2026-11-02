//! Commands the UI can call. Each one is a thin wrapper over hearth-core.

use std::sync::Arc;

use hearth_core::settings::{Settings, SettingsStore};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State, WebviewWindow, Wry};

use crate::{DataDir, Result, theme, window};

/// Event carrying the new settings after any change, so every part of the UI stays in sync.
pub const SETTINGS_EVENT: &str = "hearth://settings";

/// Called by the UI once it has painted, so the window never flashes.
#[tauri::command]
pub async fn app_ready(window: WebviewWindow<Wry>) -> Result<()> {
    window::mark_ready();
    if crate::guard::background_mode() {
        return Ok(());
    }
    window.show()?;
    window.set_focus()?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    app_version: &'static str,
    core_version: &'static str,
    webview_version: String,
    data_dir: String,
}

#[tauri::command]
pub async fn system_info(data_dir: State<'_, DataDir>) -> Result<SystemInfo> {
    Ok(SystemInfo {
        app_version: env!("CARGO_PKG_VERSION"),
        core_version: hearth_core::version(),
        webview_version: tauri::webview_version().unwrap_or_default(),
        data_dir: data_dir.0.display().to_string(),
    })
}

#[tauri::command]
pub async fn settings_get(settings: State<'_, Arc<SettingsStore>>) -> Result<Settings> {
    Ok(settings.get())
}

/// Applies a JSON merge patch (`null` resets a value) and tells every window.
#[tauri::command]
pub async fn settings_set(
    app: AppHandle<Wry>,
    settings: State<'_, Arc<SettingsStore>>,
    patch: Value,
) -> Result<Settings> {
    let next = settings.update(&patch)?;
    let _ = app.emit_to(window::MAIN, SETTINGS_EVENT, &next);
    Ok(next)
}

/// Keeps the native window background in step with the UI theme (it shows while resizing).
#[tauri::command]
pub async fn window_set_dark(window: WebviewWindow<Wry>, dark: bool) -> Result<()> {
    window.set_background_color(Some(theme::background(dark)))?;
    Ok(())
}

#[tauri::command]
pub async fn caption_maximize_rect(rect: crate::caption::ButtonRect) -> Result<()> {
    crate::caption::set_rect(rect);
    Ok(())
}
