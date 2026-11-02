//! Hearth's Tauri shell: creates the window and wires hearth-core to the UI.

// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("Hearth targets Windows 11 only.");

mod caption;
mod chat_commands;
mod code_commands;
mod commands;
mod error;
mod guard;
mod links;
mod mcp_commands;
mod runtime_commands;
mod terminal_commands;
mod theme;
mod web_commands;
mod webview2;
mod window;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hearth_core::agent::Agent;
use hearth_core::chat::{Chat, StoreResolver};
use hearth_core::mcp::Mcp;
use hearth_core::runs::RunManager;
use hearth_core::runtime::Runtime;
use hearth_core::settings::SettingsStore;
use hearth_core::store::{ConnectionInput, Store, StoreError};
use hearth_core::terminal::Terminals;
use hearth_core::web::Web;
use tauri::{Emitter, Manager};

pub use error::{Error, Result};

/// Hearth's data folder (settings, conversations, logs, WebView2 profile).
pub struct DataDir(pub PathBuf);

fn main() {
    guard::scrub_webview2_environment();
    let context = tauri::generate_context!();
    guard::require_release_security(&context);

    let mut builder = tauri::Builder::default();
    // One copy of Hearth at a time: a second launch brings the first one forward. A test run
    // has its own data folder and runs as its own instance, so it skips this.
    if guard::test_data_dir().is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::focus_main(app);
        }));
    }

    let result = builder
        .invoke_handler(tauri::generate_handler![
            commands::app_ready,
            commands::system_info,
            commands::settings_get,
            commands::settings_set,
            commands::window_set_dark,
            commands::caption_maximize_rect,
            chat_commands::sessions_list,
            chat_commands::session_create,
            chat_commands::session_get,
            chat_commands::attachment_data,
            chat_commands::session_update,
            chat_commands::session_delete,
            chat_commands::branch_select,
            chat_commands::search,
            chat_commands::session_export,
            chat_commands::connections_list,
            chat_commands::connection_save,
            chat_commands::connection_delete,
            chat_commands::connection_models,
            chat_commands::connection_test,
            chat_commands::run_start,
            chat_commands::run_attach,
            chat_commands::run_cancel,
            chat_commands::runs_active,
            chat_commands::open_link,
            code_commands::project_pick,
            code_commands::project_open,
            code_commands::projects_recent,
            code_commands::project_trust,
            code_commands::project_sessions,
            code_commands::code_session_create,
            code_commands::run_permission,
            code_commands::run_answer,
            code_commands::run_steer,
            code_commands::session_changes,
            code_commands::session_revert,
            code_commands::session_undo,
            code_commands::shells_info,
            runtime_commands::runtime_overview,
            runtime_commands::models_rescan,
            runtime_commands::model_load,
            runtime_commands::model_unload,
            runtime_commands::model_logs,
            runtime_commands::runtime_telemetry,
            runtime_commands::pick_llama_server,
            runtime_commands::pick_model_folder,
            terminal_commands::terminal_open,
            terminal_commands::terminal_attach,
            terminal_commands::terminal_write,
            terminal_commands::terminal_resize,
            terminal_commands::terminal_close,
            terminal_commands::terminals_list,
            web_commands::web_keys,
            web_commands::web_key_set,
            web_commands::web_test,
            mcp_commands::mcp_servers,
            mcp_commands::mcp_save,
            mcp_commands::mcp_remove,
            mcp_commands::mcp_set_enabled,
            mcp_commands::mcp_import,
            mcp_commands::mcp_restart,
        ])
        .setup(|app| {
            let data = data_dir(app.handle())?;
            std::fs::create_dir_all(&data)?;
            let settings = Arc::new(SettingsStore::open(data.join("settings.json")));
            apply_test_settings(&settings);
            let store = Arc::new(open_store(&data.join("hearth.db"))?);
            seed_test_connection(&store, &settings);

            let runs = Arc::new(RunManager::new());
            let handle = app.handle().clone();
            runs.on_change(move |info| {
                let _ = handle.emit_to(window::MAIN, chat_commands::RUN_STATE_EVENT, info);
            });
            let runtime = Runtime::new(settings.clone(), data.join("logs"));
            let handle = app.handle().clone();
            let watched = runtime.clone();
            runtime.on_change(move || {
                let _ = handle.emit_to(
                    window::MAIN,
                    runtime_commands::RUNTIME_EVENT,
                    watched.status(),
                );
            });
            let resolver = Arc::new(StoreResolver::with_runtime(store.clone(), runtime.clone()));
            app.manage(runtime);
            let chat = Chat::new(
                store.clone(),
                runs.clone(),
                resolver.clone(),
                settings.clone(),
            );
            // MCP servers start in the background; their tools join tasks once ready.
            let mcp = Mcp::new(store.clone());
            let handle = app.handle().clone();
            let watched = mcp.clone();
            mcp.on_change(move || {
                let _ = handle.emit_to(window::MAIN, mcp_commands::MCP_EVENT, watched.servers());
            });
            let starting = mcp.clone();
            tauri::async_runtime::spawn(async move { starting.start_all() });
            let agent = Agent::with_mcp(
                store.clone(),
                runs.clone(),
                resolver,
                settings.clone(),
                mcp.clone(),
            );
            let web = Arc::new(Web::new(store.clone(), settings.clone()));

            app.manage(settings);
            app.manage(store);
            app.manage(runs);
            app.manage(chat);
            app.manage(agent);
            app.manage(Arc::new(Terminals::new()));
            app.manage(web);
            app.manage(mcp);
            app.manage(DataDir(data.clone()));
            window::create_main(app.handle(), &data)?;
            Ok(())
        })
        .run(context);

    if let Err(err) = result {
        guard::fatal(&format!("Hearth could not start.\n\n{err}"));
    }
}

/// Where Hearth keeps its data: %LOCALAPPDATA%\app.hearth, or a test run's own folder (see
/// `guard::test_data_dir`).
fn data_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<PathBuf> {
    match guard::test_data_dir() {
        Some(dir) => Ok(dir),
        None => app.path().app_local_data_dir(),
    }
}

/// Opens the conversation database. A damaged one is kept aside (never deleted) and a fresh one
/// started, so Hearth always opens.
fn open_store(path: &Path) -> std::result::Result<Store, String> {
    match Store::open(path) {
        Ok(store) => Ok(store),
        Err(StoreError::Sqlite(err)) => {
            let aside = path.with_extension(format!("db.damaged-{}", hearth_core::now_ms()));
            let _ = std::fs::rename(path, &aside);
            Store::open(path).map_err(|fresh| {
                format!("The conversation database is damaged ({err}), and a new one couldn't be created ({fresh}).")
            })
        }
        Err(other) => Err(other.to_string()),
    }
}

/// Test runs started with `HEARTH_TEST_SETTINGS` (a JSON merge patch) get those settings.
fn apply_test_settings(settings: &SettingsStore) {
    let Some(patch) = guard::test_value("HEARTH_TEST_SETTINGS") else {
        return;
    };
    match serde_json::from_str::<serde_json::Value>(&patch) {
        Ok(patch) => {
            if let Err(err) = settings.update(&patch) {
                eprintln!("HEARTH_TEST_SETTINGS was rejected: {err}");
            }
        }
        Err(err) => eprintln!("HEARTH_TEST_SETTINGS isn't JSON: {err}"),
    }
}

/// Test runs started with `HEARTH_TEST_LLM_URL` get a "Mock" connection as their default model.
fn seed_test_connection(store: &Store, settings: &SettingsStore) {
    let Some(url) = guard::test_llm_url() else {
        return;
    };
    let exists = store
        .connections()
        .map(|all| all.iter().any(|c| c.name == "Mock"))
        .unwrap_or(false);
    if exists {
        return;
    }
    let input = ConnectionInput {
        id: None,
        name: "Mock".into(),
        base_url: url,
        api_key: None,
        llama_extensions: true,
    };
    let Ok(connection) = store.save_connection(&input) else {
        return;
    };
    let model = serde_json::json!({ "kind": "connection", "connectionId": connection.id, "model": "mock-model" });
    let _ =
        settings.update(&serde_json::json!({ "models": { "chat": model.clone(), "code": model } }));
}
