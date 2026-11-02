//! Windows' light/dark app mode, read before the window exists so the very first frame
//! already has the right background.

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

/// Window backgrounds, the `--bg` tokens in apps/hearth/src/styles/app.css.
pub const LIGHT_BG: (u8, u8, u8) = (0xFA, 0xF9, 0xF7);
pub const DARK_BG: (u8, u8, u8) = (0x1A, 0x19, 0x17);

/// True when Windows is set to dark mode for apps.
pub fn system_prefers_dark() -> bool {
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: valid key and value names and an output buffer of the declared size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && value == 0
}

/// The window background for a theme.
pub fn background(dark: bool) -> tauri::window::Color {
    let (r, g, b) = if dark { DARK_BG } else { LIGHT_BG };
    tauri::window::Color(r, g, b, 255)
}
