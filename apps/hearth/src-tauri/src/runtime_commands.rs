//! Commands for the Models page: llama-server, model folders, the models found, loading and
//! unloading them, and how much memory is in use.

use std::path::PathBuf;

use hearth_core::runtime::llama::ServerInfo;
use hearth_core::runtime::telemetry::{self, Telemetry};
use hearth_core::runtime::{LocalModel, ModelStatus, Runtime};
use serde::Serialize;
use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// Sent to the UI whenever a model starts loading, becomes ready, fails or is unloaded.
pub const RUNTIME_EVENT: &str = "hearth://runtime";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    path: String,
    exists: bool,
    /// Searched automatically (LM Studio's folder), not one the user added.
    automatic: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    server: Option<ServerInfo>,
    /// Why llama-server can't be used, when it can't.
    server_error: Option<String>,
    folders: Vec<Folder>,
    models: Vec<LocalModel>,
    status: Vec<ModelStatus>,
}

/// Everything the Models page shows. `refresh` probes llama-server and scans folders again.
#[tauri::command]
pub async fn runtime_overview(
    runtime: State<'_, Runtime>,
    settings: State<'_, std::sync::Arc<hearth_core::settings::SettingsStore>>,
    refresh: bool,
) -> Result<Overview> {
    let (server, server_error) = match runtime.server_info(refresh).await {
        Ok(info) => (Some(info), None),
        Err(err) => (None, Some(err)),
    };
    let configured: Vec<String> = settings.get().runtime.model_folders;
    let mut folders: Vec<Folder> = configured
        .iter()
        .map(|path| Folder {
            exists: PathBuf::from(path).is_dir(),
            path: path.clone(),
            automatic: false,
        })
        .collect();
    folders.extend(
        hearth_core::runtime::scan::lm_studio_folders()
            .into_iter()
            .map(|path| Folder {
                path: path.display().to_string(),
                exists: true,
                automatic: true,
            }),
    );
    let models = if refresh {
        runtime.rescan()
    } else {
        runtime.models()
    };
    Ok(Overview {
        server,
        server_error,
        folders,
        models,
        status: runtime.status(),
    })
}

#[tauri::command]
pub async fn models_rescan(runtime: State<'_, Runtime>) -> Result<Vec<LocalModel>> {
    Ok(runtime.rescan())
}

/// Loads a model now (instead of on first use). Resolves once it's ready.
#[tauri::command]
pub async fn model_load(runtime: State<'_, Runtime>, model_id: String) -> Result<()> {
    runtime
        .ensure(&model_id, &CancellationToken::new())
        .await
        .map(|_| ())
        .map_err(Error::Invalid)
}

#[tauri::command]
pub async fn model_unload(runtime: State<'_, Runtime>, model_id: String) -> Result<()> {
    runtime.unload(&model_id);
    Ok(())
}

/// The last lines llama-server printed for a model.
#[tauri::command]
pub async fn model_logs(runtime: State<'_, Runtime>, model_id: String) -> Result<Vec<String>> {
    Ok(runtime.logs(&model_id))
}

#[tauri::command]
pub async fn runtime_telemetry() -> Result<Telemetry> {
    Ok(telemetry::read().await)
}

/// Lets the user choose llama-server.exe. `None` if cancelled.
#[tauri::command]
pub async fn pick_llama_server() -> Result<Option<String>> {
    let picked = match crate::guard::test_value("HEARTH_TEST_PICK_FILE") {
        Some(path) => Some(PathBuf::from(path)),
        None => rfd::AsyncFileDialog::new()
            .set_title("Choose llama-server.exe")
            .add_filter("llama-server", &["exe"])
            .pick_file()
            .await
            .map(|handle| handle.path().to_path_buf()),
    };
    Ok(picked.map(|path| path.display().to_string()))
}

/// Lets the user choose a folder with models. `None` if cancelled.
#[tauri::command]
pub async fn pick_model_folder() -> Result<Option<String>> {
    let picked = match crate::guard::test_pick_folder() {
        Some(path) => Some(path),
        None => rfd::AsyncFileDialog::new()
            .set_title("Choose a folder with GGUF models")
            .pick_folder()
            .await
            .map(|handle| handle.path().to_path_buf()),
    };
    Ok(picked.map(|path| path.display().to_string()))
}
