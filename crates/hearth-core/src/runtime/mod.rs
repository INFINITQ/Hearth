//! Hearth's own model runtime: finding GGUF models and llama-server, and running one
//! llama-server per loaded model (see ADR 0002).

pub mod gguf;
pub mod llama;
pub mod manager;
pub mod scan;
pub mod telemetry;

pub use manager::{ModelStatus, Runtime, ServerState};
pub use scan::LocalModel;
