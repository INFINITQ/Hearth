//! Talking to models: an OpenAI-compatible streaming client (llama-server, LM Studio, Ollama's
//! `/v1`), the server-sent-events parser under it, and helpers that turn a stream of deltas into
//! a finished reply.

pub mod assemble;
pub mod client;
pub mod sse;
pub mod think;
pub mod types;

pub use assemble::{Assembler, AssistantReply};
pub use client::{ChatStream, Health, LlmClient};
pub use types::*;
