//! Turns a stream of deltas into the finished reply.

use super::types::{FinishReason, StreamEvent, ToolCall, Usage};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssistantReply {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish: Option<FinishReason>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Default)]
struct PartialCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

/// Collects stream events. Tool-call pieces are matched by their index; a call whose id never
/// arrived gets a generated one.
#[derive(Debug, Default)]
pub struct Assembler {
    content: String,
    reasoning: String,
    calls: Vec<PartialCall>,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::Reasoning(text) => self.reasoning.push_str(text),
            StreamEvent::Content(text) => self.content.push_str(text),
            StreamEvent::ToolCall {
                index,
                id,
                name,
                arguments,
            } => {
                while self.calls.len() <= *index {
                    self.calls.push(PartialCall::default());
                }
                let call = &mut self.calls[*index];
                if let Some(id) = id.as_ref().filter(|id| !id.is_empty()) {
                    call.id = Some(id.clone());
                }
                if let Some(name) = name.as_ref().filter(|name| !name.is_empty()) {
                    call.name.clone_from(name);
                }
                call.arguments.push_str(arguments);
            }
            StreamEvent::Usage(usage) => self.usage = Some(usage.clone()),
            StreamEvent::Finish(reason) => self.finish = Some(reason.clone()),
            StreamEvent::PromptProgress { .. } => {}
        }
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn reasoning(&self) -> &str {
        &self.reasoning
    }

    pub fn finish(self) -> AssistantReply {
        let tool_calls = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, call)| !call.name.is_empty())
            .map(|(index, call)| {
                let arguments = if call.arguments.trim().is_empty() {
                    "{}".to_owned()
                } else {
                    call.arguments
                };
                ToolCall::new(
                    call.id.unwrap_or_else(|| format!("call_{index}")),
                    call.name,
                    arguments,
                )
            })
            .collect();
        AssistantReply {
            content: self.content,
            reasoning: self.reasoning,
            tool_calls,
            finish: self.finish,
            usage: self.usage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(index: usize, id: Option<&str>, name: Option<&str>, arguments: &str) -> StreamEvent {
        StreamEvent::ToolCall {
            index,
            id: id.map(String::from),
            name: name.map(String::from),
            arguments: arguments.into(),
        }
    }

    #[test]
    fn assembles_interleaved_tool_calls_by_index() {
        let mut assembler = Assembler::new();
        for event in [
            StreamEvent::Content("Reading both.".into()),
            call(0, Some("a"), Some("read_file"), ""),
            call(1, Some("b"), Some("glob"), "{\"pat"),
            call(0, None, None, "{\"path\":\"x.rs\"}"),
            call(1, None, None, "tern\":\"*.md\"}"),
            StreamEvent::Finish(FinishReason::ToolCalls),
        ] {
            assembler.push(&event);
        }
        let reply = assembler.finish();
        assert_eq!(reply.content, "Reading both.");
        assert_eq!(
            reply.tool_calls,
            vec![
                ToolCall::new("a", "read_file", "{\"path\":\"x.rs\"}"),
                ToolCall::new("b", "glob", "{\"pattern\":\"*.md\"}")
            ]
        );
        assert_eq!(reply.finish, Some(FinishReason::ToolCalls));
    }

    #[test]
    fn missing_ids_and_empty_arguments_get_defaults() {
        let mut assembler = Assembler::new();
        assembler.push(&call(0, None, Some("todo_write"), ""));
        let reply = assembler.finish();
        assert_eq!(
            reply.tool_calls,
            vec![ToolCall::new("call_0", "todo_write", "{}")]
        );
    }
}
