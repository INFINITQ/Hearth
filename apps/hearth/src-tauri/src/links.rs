//! Opening links in the user's browser. Only web and mail links; the webview itself never
//! navigates away from the app.

use tauri::Url;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use crate::{Error, Result};

/// Opens `url` in the default browser or mail app.
pub fn open(url: &str) -> Result<()> {
    let parsed =
        Url::parse(url.trim()).map_err(|_| Error::Invalid("That isn't a valid link.".into()))?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err(Error::Invalid(
            "Only web and email links can be opened.".into(),
        ));
    }
    if let Some(log) = crate::guard::test_links_file() {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)?;
        writeln!(file, "{parsed}")?;
        return Ok(());
    }
    // SAFETY: plain Win32 call with valid, NUL-terminated strings.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(parsed.as_str()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success with a value greater than 32.
    if result.0 as isize <= 32 {
        return Err(Error::Invalid("Windows couldn't open that link.".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_web_and_mail_links_are_allowed() {
        for bad in [
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "ms-settings:",
            "not a url",
        ] {
            assert!(super::open(bad).is_err(), "{bad} should be refused");
        }
    }
}
