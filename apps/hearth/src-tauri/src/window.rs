//! The main window: frameless with Hearth's own title bar, a solid themed background, and
//! hidden until the UI has painted (so it never flashes white).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hearth_core::settings::{SettingsStore, Theme};
use tauri::webview::NewWindowResponse;
use tauri::{
    AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent, Wry,
};

use crate::{Result, theme, webview2};

pub const MAIN: &str = "main";

/// Runs before any page script: removes WebRTC and switches off DNS prefetching.
const INIT_SCRIPT: &str = include_str!("init.js");

static READY: AtomicBool = AtomicBool::new(false);

pub fn create_main(app: &AppHandle<Wry>, data_dir: &Path) -> Result<()> {
    // WebView2 adds its own `EBWebView` folder inside this one.
    let environment = webview2::create_environment(data_dir)?;
    webview2::clear_crash_reports(&environment);

    // Settings and the Windows theme are handed over before the page loads, so the first
    // paint already has the right theme.
    let settings = app.state::<SettingsStore>().get();
    let system_dark = theme::system_prefers_dark();
    let dark = match settings.theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => system_dark,
    };
    let boot = serde_json::json!({ "settings": settings, "systemDark": system_dark });

    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Hearth")
        .inner_size(1280.0, 820.0)
        .min_inner_size(720.0, 480.0)
        .decorations(false)
        .background_color(theme::background(dark))
        .visible(false)
        .incognito(true)
        .general_autofill_enabled(false)
        .zoom_hotkeys_enabled(false)
        .initialization_script(INIT_SCRIPT)
        .initialization_script(format!("window.__HEARTH_BOOT__ = Object.freeze({boot});"))
        .on_navigation(crate::guard::allow_navigation)
        .on_new_window(|_url, _features| NewWindowResponse::Deny)
        .on_download(|_webview, _event| false)
        .with_environment(environment)
        .build()?;

    webview2::harden(&window)?;
    crate::caption::install(&window)?;

    // Minimized, WebView2 is asked to use less memory; restored, it works normally again.
    let watched = window.clone();
    let minimized = AtomicBool::new(false);
    window.on_window_event(move |event| {
        if let WindowEvent::Resized(_) = event {
            let now = watched.is_minimized().unwrap_or(false);
            if minimized.swap(now, Ordering::SeqCst) != now {
                webview2::set_low_memory(&watched, now);
            }
        }
    });

    show_if_ui_hangs(window);
    Ok(())
}

/// Brings the main window forward (a second launch of Hearth does this).
pub fn focus_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Called by the UI once it has painted.
pub fn mark_ready() {
    READY.store(true, Ordering::SeqCst);
}

/// If the UI never says it's ready (a crash or a bug), show the window anyway so the app
/// doesn't keep running invisibly.
fn show_if_ui_hangs(window: WebviewWindow<Wry>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        if !READY.load(Ordering::SeqCst) && !crate::guard::background_mode() {
            let _ = window.show();
        }
    });
}
