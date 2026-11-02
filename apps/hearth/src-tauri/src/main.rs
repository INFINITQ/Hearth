//! Hearth's Tauri shell: creates the window and wires hearth-core to the UI.

// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("Hearth targets Windows 11 only.");

mod caption;
mod commands;
mod error;
mod guard;
mod theme;
mod webview2;
mod window;

use std::path::PathBuf;

use hearth_core::settings::SettingsStore;
use tauri::Manager;

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
        ])
        .setup(|app| {
            let data = data_dir(app.handle())?;
            std::fs::create_dir_all(&data)?;
            app.manage(SettingsStore::open(data.join("settings.json")));
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
