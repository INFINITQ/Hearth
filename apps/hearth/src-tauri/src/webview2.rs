//! Hearth creates the WebView2 environment itself instead of letting Tauri do it, so it can keep
//! crash dumps on this PC and choose every browser flag (ported from Folio, see its ADR 0002).
//! With our own environment, Tauri's browser-flag and proxy settings are ignored, so all of
//! them live here.

use std::path::Path;
use std::sync::mpsc;

use tauri::{Runtime, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_SCROLLBAR_STYLE_FLUENT_OVERLAY,
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2_19, ICoreWebView2Controller,
    ICoreWebView2Environment, ICoreWebView2Environment11, ICoreWebView2EnvironmentOptions,
    ICoreWebView2Settings3, ICoreWebView2Settings4, ICoreWebView2Settings5, ICoreWebView2Settings6,
    ICoreWebView2Settings8,
};
use webview2_com::{
    CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler,
    PermissionRequestedEventHandler,
};
use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

use crate::Result;

/// Chromium flags for Hearth's webview. The UI itself never needs the network: llama-server,
/// web search and MCP are all reached from Rust (docs/adr/0003-ui-never-touches-the-network.md).
pub const BROWSER_ARGS: &str = concat!(
    // Tauri's defaults: no mini menu, no PDF mini menu, no SmartScreen URL checks.
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
    " --block-new-web-contents=true",
    // No background services (component updates, domain reliability reports, link pings).
    " --disable-background-networking",
    " --disable-component-update",
    " --disable-domain-reliability",
    " --no-pings",
    // Last backstop: anything that slips past the content policy goes to a closed local port
    // instead of the internet.
    " --proxy-server=http://127.0.0.1:9",
);

/// Creates the WebView2 environment with Hearth's privacy settings. Blocks (pumping messages)
/// until WebView2 is ready.
pub fn create_environment(data_dir: &Path) -> Result<ICoreWebView2Environment> {
    // WebView2 needs COM on this (main) thread as a single-threaded apartment. Tauri does this
    // when it builds a webview, but we create the environment before that.
    // SAFETY: plain Win32 call; repeated calls on the same thread are harmless.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: property setters on a freshly created, not yet shared options object.
    unsafe {
        options.set_additional_browser_arguments(BROWSER_ARGS.to_string());
        options.set_are_browser_extensions_enabled(false);
        options.set_allow_single_sign_on_using_os_primary_account(false);
        options.set_exclusive_user_data_folder_access(true);
        // Crash dumps stay on this PC (WebView2 uploads them to Microsoft otherwise, and a dump
        // can contain conversation text). `clear_crash_reports` deletes old ones.
        options.set_is_custom_crash_reporting_enabled(true);
        options.set_enable_tracking_prevention(true);
        options.set_scroll_bar_style(COREWEBVIEW2_SCROLLBAR_STYLE_FLUENT_OVERLAY);
    }

    let (tx, rx) = mpsc::channel::<windows::core::Result<ICoreWebView2Environment>>();
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |status, environment| {
            let result = status
                .and_then(|()| environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            tx.send(result)
                .map_err(|_| windows::core::Error::from(E_UNEXPECTED))
        },
    ));

    // SAFETY: valid arguments (a null browser folder means the WebView2 that comes with
    // Windows); the handler is kept alive by WebView2 until it runs.
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &HSTRING::from(data_dir.as_os_str()),
            &ICoreWebView2EnvironmentOptions::from(options),
            &handler,
        )?;
    }
    let environment = webview2_com::wait_with_pump(rx)??;
    Ok(environment)
}

/// Deletes crash dumps left by earlier runs. They can contain conversation text.
pub fn clear_crash_reports(environment: &ICoreWebView2Environment) {
    let Ok(environment) = environment.cast::<ICoreWebView2Environment11>() else {
        return;
    };
    let mut raw = PWSTR::null();
    // SAFETY: WebView2 writes a CoTaskMem string that `take_pwstr` frees.
    if unsafe { environment.FailureReportFolderPath(&mut raw) }.is_err() {
        return;
    }
    let folder = webview2_com::take_pwstr(raw);
    if folder.is_empty() {
        return;
    }
    if let Err(err) = remove_contents(Path::new(&folder)) {
        eprintln!("hearth: could not clear old crash reports in {folder}: {err}");
    }
}

fn remove_contents(folder: &Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Switches off browser features Hearth never needs, once the webview exists: browser
/// shortcuts (F5 would reload away a running reply), the browser context menu, zoom, swipe
/// navigation, autofill, password saving, reputation checks, and every permission prompt
/// (camera, microphone, location, notifications...).
pub fn harden<R: Runtime>(window: &WebviewWindow<R>) -> Result<()> {
    window.with_webview(|webview| {
        // SAFETY: runs on the main thread with the live controller.
        if let Err(err) = unsafe { harden_controller(&webview.controller()) } {
            eprintln!("hearth: could not harden WebView2 settings: {err}");
        }
    })?;
    Ok(())
}

/// Asks WebView2 to give back what memory it can (while the window is minimized), or to work
/// normally again. WebView2 decides what to release: caches first, then more.
pub fn set_low_memory<R: Runtime>(window: &WebviewWindow<R>, low: bool) {
    let level = match low {
        true => COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        false => COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    let _ = window.with_webview(move |webview| {
        // SAFETY: runs on the main thread with the live controller; older WebView2 runtimes
        // without the setting are left alone.
        unsafe {
            if let Ok(core) = webview.controller().CoreWebView2()
                && let Ok(core) = core.cast::<ICoreWebView2_19>()
            {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        }
    });
}

unsafe fn harden_controller(controller: &ICoreWebView2Controller) -> windows::core::Result<()> {
    // SAFETY: the caller guarantees a live controller on the main thread.
    unsafe {
        let core = controller.CoreWebView2()?;
        let settings = core.Settings()?;
        settings.SetAreDefaultContextMenusEnabled(false)?;
        settings.SetIsZoomControlEnabled(false)?;
        settings.SetIsStatusBarEnabled(false)?;
        settings.SetAreHostObjectsAllowed(false)?;
        if let Ok(s) = settings.cast::<ICoreWebView2Settings3>() {
            s.SetAreBrowserAcceleratorKeysEnabled(false)?;
        }
        if let Ok(s) = settings.cast::<ICoreWebView2Settings4>() {
            s.SetIsGeneralAutofillEnabled(false)?;
            s.SetIsPasswordAutosaveEnabled(false)?;
        }
        if let Ok(s) = settings.cast::<ICoreWebView2Settings5>() {
            s.SetIsPinchZoomEnabled(false)?;
        }
        if let Ok(s) = settings.cast::<ICoreWebView2Settings6>() {
            s.SetIsSwipeNavigationEnabled(false)?;
        }
        if let Ok(s) = settings.cast::<ICoreWebView2Settings8>() {
            s.SetIsReputationCheckingRequired(false)?;
        }

        let mut token = 0i64;
        core.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_sender, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}
