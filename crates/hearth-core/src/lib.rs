//! Hearth's app logic.
//!
//! Everything here is plain Rust with no Tauri dependency, so it is tested directly
//! (`cargo test -p hearth-core`). The Tauri shell in `apps/hearth/src-tauri` only wires these
//! pieces to the window and the UI.

pub mod agent;
pub mod chat;
pub mod fsutil;
pub mod llm;
pub mod proc;
pub mod runs;
pub mod runtime;
pub mod settings;
pub mod store;

/// Version of the core, for diagnostics.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Milliseconds since the Unix epoch (UTC), the timestamp format used everywhere in Hearth.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// A new time-sortable unique id (UUIDv7).
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
