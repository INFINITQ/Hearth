//! Request and response shapes of the OpenAI-compatible chat API, plus llama.cpp's extensions.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

/// Message content: plain text, or parts (text and images) for vision models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl Content {
    /// The text of the content (images skipped).
    pub fn text(&self) -> String {
        match self {
            Content::Text(text) => text.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } => Some(text.as_str()),
                    ContentPart::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON text, exactly as the model produced it (it may be invalid).
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionCall,
}

fn function_kind() -> String {
    "function".into()
}

impl ToolCall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: function_kind(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    /// The model's thinking. Sent back only when the model family wants it (see profiles).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    fn new(role: Role, content: Option<Content>) -> Self {
        Self {
            role,
            content,
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
    pub fn system(text: impl Into<String>) -> Self {
        Self::new(Role::System, Some(Content::Text(text.into())))
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, Some(Content::Text(text.into())))
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::new(Role::Assistant, Some(Content::Text(text.into())))
    }
    pub fn tool(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        let mut message = Self::new(Role::Tool, Some(Content::Text(text.into())));
        message.tool_call_id = Some(call_id.into());
        message
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionDef,
}

impl ToolDef {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            kind: function_kind(),
            function: FunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// Sampling settings. Unset values are left to the server (and the model's defaults).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sampling {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<Value>,
    #[serde(flatten)]
    pub sampling: Sampling,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    // llama.cpp extensions (sent only to servers known to understand them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_progress: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_prompt: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    Other(String),
}

impl FinishReason {
    pub fn parse(value: &str) -> Self {
        match value {
            "stop" | "eos" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "tool_calls" | "function_call" => FinishReason::ToolCalls,
            other => FinishReason::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            FinishReason::Stop => "stop",
            FinishReason::Length => "length",
            FinishReason::ToolCalls => "tool_calls",
            FinishReason::Other(other) => other,
        }
    }
}

/// Token counts and speed for one reply. llama.cpp adds the speeds and the cached prompt length.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_per_second: Option<f64>,
}

/// One piece of a streamed reply.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Reasoning(String),
    Content(String),
    /// A piece of a tool call. The first piece of each call carries its id and name; the
    /// arguments arrive as JSON text in pieces.
    ToolCall {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
    /// llama.cpp reading the prompt (with `return_progress`).
    PromptProgress {
        processed: u64,
        total: u64,
        cached: u64,
    },
    Usage(Usage),
    Finish(FinishReason),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LlmError {
    #[error("Couldn't reach the model server at {url} ({detail}).")]
    Unreachable { url: String, detail: String },
    #[error("The model server needs an API key, or the key is wrong.")]
    Unauthorized,
    #[error("The model is still loading.")]
    Loading,
    #[error(
        "The conversation is too long for this model: {n_prompt} tokens, and the limit is {n_ctx}."
    )]
    ContextExceeded { n_prompt: u64, n_ctx: u64 },
    #[error("The model server returned an error ({status}): {message}")]
    Http { status: u16, message: String },
    #[error("The connection to the model server was lost.")]
    ConnectionLost,
    #[error("The model server sent something unexpected: {0}")]
    Protocol(String),
    /// Hearth couldn't start the model's llama-server.
    #[error("{0}")]
    Runtime(String),
    #[error("Stopped.")]
    Cancelled,
}

impl LlmError {
    /// Stable code for the UI and logs.
    pub fn code(&self) -> &'static str {
        match self {
            LlmError::Unreachable { .. } => "unreachable",
            LlmError::Unauthorized => "unauthorized",
            LlmError::Loading => "loading",
            LlmError::ContextExceeded { .. } => "context_exceeded",
            LlmError::Http { .. } => "http",
            LlmError::ConnectionLost => "connection_lost",
            LlmError::Protocol(_) => "protocol",
            LlmError::Runtime(_) => "runtime",
            LlmError::Cancelled => "cancelled",
        }
    }

    /// Whether trying again (unchanged) might work.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            LlmError::Unreachable { .. } | LlmError::Loading | LlmError::ConnectionLost
        ) || matches!(self, LlmError::Http { status, .. } if *status >= 500)
    }
}
