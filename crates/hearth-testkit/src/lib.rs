//! Test doubles for Hearth. Tests never use real models: they script what the "model" says.

pub mod mcp;
pub mod mock;

pub use mock::{Expect, Fault, MockServer, Reply, Script, Step, ToolCallSpec};
