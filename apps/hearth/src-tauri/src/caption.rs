//! Snap Layouts for Hearth's custom title bar (ported from Folio, see its ADR 0004).
//!
//! Windows 11 shows its Snap Layouts flyout only when the window under the cursor answers
//! `WM_NCHITTEST` with `HTMAXBUTTON`. Over our HTML maximize button that window belongs to
//! WebView2, which we can't change. So a tiny native child window sits exactly over the button:
//! it never paints (the HTML button shows through), answers `HTMAXBUTTON`, and tells the page
//! about hover and presses so the HTML button can still look right.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use tauri::{Emitter, WebviewWindow, WindowEvent, Wry};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetParent, HTMAXBUTTON, HWND_TOP, IDC_ARROW, IsZoomed,
    LoadCursorW, RegisterClassW, SW_MAXIMIZE, SW_RESTORE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WM_ERASEBKGND, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
    WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_PAINT, WNDCLASSW,
    WS_CHILD, WS_CLIPSIBLINGS,
};
use windows::core::w;

use crate::Result;

/// Event the page receives: `"enter" | "leave" | "press" | "release"`.
pub const EVENT: &str = "hearth://caption";

/// The HTML maximize button's position in CSS pixels, measured by the UI: distance of its right
/// edge from the window's right edge, its top, and its size.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ButtonRect {
    pub right: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

struct Overlay {
    /// The overlay's HWND, stored as an integer because `HWND` isn't `Send`.
    hwnd: isize,
    window: WebviewWindow<Wry>,
    rect: Mutex<Option<ButtonRect>>,
}

static OVERLAY: OnceLock<Overlay> = OnceLock::new();
static TRACKING: AtomicBool = AtomicBool::new(false);

/// Creates the (hidden) overlay over the main window. It appears once the UI reports where its
/// maximize button is.
pub fn install(window: &WebviewWindow<Wry>) -> Result<()> {
    let parent = window.hwnd()?;
    // SAFETY: plain Win32 calls on the main thread with valid arguments.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("HearthSnapButton"),
            ..Default::default()
        };
        // Returns 0 if the class already exists, which is fine.
        RegisterClassW(&class);
        // The window title doubles as its accessible name for Narrator.
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("HearthSnapButton"),
            w!("Maximize"),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )?
    };

    let _ = OVERLAY.set(Overlay {
        hwnd: hwnd.0 as isize,
        window: window.clone(),
        rect: Mutex::new(None),
    });
    window.on_window_event(|event| {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            reposition();
        }
    });
    Ok(())
}

/// The UI reports its maximize button's position (on load and whenever its layout changes).
pub fn set_rect(rect: ButtonRect) {
    if let Some(overlay) = OVERLAY.get() {
        if let Ok(mut current) = overlay.rect.lock() {
            *current = Some(rect);
        }
        reposition();
    }
}

fn reposition() {
    let Some(overlay) = OVERLAY.get() else { return };
    let Some(rect) = overlay.rect.lock().ok().and_then(|r| *r) else {
        return;
    };
    let (Ok(scale), Ok(size)) = (overlay.window.scale_factor(), overlay.window.inner_size()) else {
        return;
    };
    let px = |css: f64| (css * scale).round() as i32;
    let (width, height) = (px(rect.width), px(rect.height));
    let x = size.width as i32 - px(rect.right) - width;
    let y = px(rect.top);
    // SAFETY: our own child window; the call fails harmlessly if it no longer exists.
    unsafe {
        let _ = SetWindowPos(
            HWND(overlay.hwnd as *mut _),
            Some(HWND_TOP),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

fn emit(state: &str) {
    if let Some(overlay) = OVERLAY.get() {
        let _ = overlay.window.emit_to(overlay.window.label(), EVENT, state);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // The whole point: "this is a maximize button", so Windows offers Snap Layouts.
        WM_NCHITTEST => LRESULT(HTMAXBUTTON as isize),
        WM_NCMOUSEMOVE => {
            if !TRACKING.swap(true, Ordering::SeqCst) {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: valid, fully initialised struct for our own window.
                let _ = unsafe { TrackMouseEvent(&mut track) };
                emit("enter");
            }
            LRESULT(0)
        }
        WM_NCMOUSELEAVE => {
            TRACKING.store(false, Ordering::SeqCst);
            emit("leave");
            LRESULT(0)
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
            emit("press");
            LRESULT(0)
        }
        WM_NCLBUTTONUP => {
            emit("release");
            // SAFETY: the parent is the live main window.
            unsafe {
                if let Ok(parent) = GetParent(hwnd) {
                    let command = if IsZoomed(parent).as_bool() {
                        SW_RESTORE
                    } else {
                        SW_MAXIMIZE
                    };
                    let _ = ShowWindow(parent, command);
                }
            }
            LRESULT(0)
        }
        // Never paint, so the HTML button underneath stays visible.
        WM_PAINT => {
            // SAFETY: marks our own window's update region as painted.
            let _ = unsafe { ValidateRect(Some(hwnd), None) };
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
