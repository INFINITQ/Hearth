use serde::{Serialize, Serializer, ser::SerializeStruct};

/// Errors returned to the UI as `{ kind, message }`, so it can react to the kind.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("WebView2: {0}")]
    WebView2(String),
    #[error("Windows: {0}")]
    Windows(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Stable, machine-readable kind for the UI.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Invalid(_) => "invalid",
            Error::Tauri(_) => "tauri",
            Error::Io(_) => "io",
            Error::WebView2(_) => "webview2",
            Error::Windows(_) => "windows",
        }
    }
}

impl From<windows::core::Error> for Error {
    fn from(err: windows::core::Error) -> Self {
        Error::Windows(err.to_string())
    }
}

impl From<webview2_com::Error> for Error {
    fn from(err: webview2_com::Error) -> Self {
        Error::WebView2(format!("{err:?}"))
    }
}

impl From<hearth_core::settings::SettingsError> for Error {
    fn from(err: hearth_core::settings::SettingsError) -> Self {
        match err {
            hearth_core::settings::SettingsError::Io(io) => Error::Io(io),
            other => Error::Invalid(other.to_string()),
        }
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Error", 2)?;
        state.serialize_field("kind", self.kind())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}
