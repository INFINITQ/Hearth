//! Start-up guards, test hooks and the navigation policy.

use std::ffi::OsString;
use std::path::PathBuf;

use tauri::Url;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::{HSTRING, w};

/// Release builds drop every `WEBVIEW2_*` environment variable before WebView2 starts.
/// WebView2 reads them as extra browser flags, which could re-enable remote debugging or undo
/// the hardening. Debug builds keep them for the end-to-end tests.
pub fn scrub_webview2_environment() {
    if cfg!(debug_assertions) {
        return;
    }
    let keys: Vec<OsString> = std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| {
            key.to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("WEBVIEW2_")
        })
        .collect();
    for key in keys {
        // SAFETY: runs first thing in `main`, before Tauri or WebView2 start any threads.
        unsafe { std::env::remove_var(&key) };
    }
}

/// Release builds refuse to start without the content security policy and frozen prototypes,
/// so a misconfigured build can never ship an unprotected UI.
pub fn require_release_security(context: &tauri::Context<tauri::Wry>) {
    if cfg!(debug_assertions) {
        return;
    }
    let security = &context.config().app.security;
    if security.csp.is_none() || !security.freeze_prototype {
        fatal("This build of Hearth has no content security policy, so it refuses to start.");
    }
}

/// The webview may only show the app itself: `http://tauri.localhost` in release builds, plus
/// the Vite dev server on `localhost` in debug builds. Everything else is blocked (links open
/// in your browser instead).
pub fn allow_navigation(url: &Url) -> bool {
    let app_page = matches!(
        (url.scheme(), url.host_str()),
        ("http" | "https", Some("tauri.localhost"))
    );
    let dev_server = matches!((url.scheme(), url.host_str()), ("http", Some("localhost")));
    app_page || (dev_server && cfg!(debug_assertions))
}

/// Debug builds started with `HEARTH_BACKGROUND=1` keep their window hidden and never take
/// focus, so automated tests don't disturb whoever is using the PC. Always off in release.
pub fn background_mode() -> bool {
    cfg!(debug_assertions) && std::env::var_os("HEARTH_BACKGROUND").is_some_and(|v| v == "1")
}

/// Debug builds started with `HEARTH_TEST_DATA_DIR` keep everything in that folder (settings,
/// conversations, WebView2's profile) and run as their own instance, so tests never touch the
/// installed app's data. Always off in release.
pub fn test_data_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("HEARTH_TEST_DATA_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// A debug-build test value from the environment (always `None` in release builds).
pub fn test_value(name: &str) -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// `HEARTH_TEST_LLM_URL`: a mock model server; Hearth adds it as the "Mock" connection and
/// makes it the default chat and coding model.
pub fn test_llm_url() -> Option<String> {
    test_value("HEARTH_TEST_LLM_URL")
}

/// `HEARTH_TEST_SAVE_PATH`: where Save dialogs "save" in tests.
pub fn test_save_path() -> Option<PathBuf> {
    test_value("HEARTH_TEST_SAVE_PATH").map(PathBuf::from)
}

/// `HEARTH_TEST_PICK_FOLDER`: the folder "Open folder…" picks in tests.
pub fn test_pick_folder() -> Option<PathBuf> {
    test_value("HEARTH_TEST_PICK_FOLDER").map(PathBuf::from)
}

/// `HEARTH_TEST_LINKS`: links the app would open are appended to this file instead.
pub fn test_links_file() -> Option<PathBuf> {
    test_value("HEARTH_TEST_LINKS").map(PathBuf::from)
}

/// Shows an error box and exits. Used before (or instead of) a working UI.
pub fn fatal(message: &str) -> ! {
    let text = HSTRING::from(message);
    // SAFETY: plain Win32 call with valid, NUL-terminated strings.
    unsafe { MessageBoxW(None, &text, w!("Hearth"), MB_OK | MB_ICONERROR) };
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::allow_navigation;
    use tauri::Url;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn app_pages_are_allowed() {
        assert!(allow_navigation(&url("http://tauri.localhost/index.html")));
        assert!(allow_navigation(&url("https://tauri.localhost/")));
    }

    #[test]
    fn everything_else_is_blocked() {
        for bad in [
            "https://example.com/",
            "http://tauri.localhost.evil.com/",
            "file:///C:/Windows/win.ini",
            "edge://settings",
            "javascript:alert(1)",
            "data:text/html,hi",
            "http://ipc.localhost/",
            "http://127.0.0.1:8080/",
        ] {
            assert!(!allow_navigation(&url(bad)), "{bad} should be blocked");
        }
    }
}
