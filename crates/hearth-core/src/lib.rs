//! Hearth's app logic.
//!
//! Everything here is plain Rust with no Tauri dependency, so it is tested directly
//! (`cargo test -p hearth-core`). The Tauri shell in `apps/hearth/src-tauri` only wires these
//! pieces to the window and the UI.

pub mod fsutil;
pub mod settings;

/// Version of the core, for diagnostics.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
