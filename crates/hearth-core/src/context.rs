//! Keeping conversations inside the model's context window.
//!
//! Each reply reports how many tokens the conversation took. Past 80% of the window, the older
//! part is summarized by the same model, and a summary message stands in for it in requests.
//! The messages themselves stay (in the transcript and the database). Recent messages are kept
//! word for word, and a cut never separates a tool call from its result.

use crate::llm::{ChatMessage, ChatRequest, LlmClient, LlmError};
use crate::store::{Message, MessageRole};

/// Compact when the conversation uses more than this share of the window.
pub const COMPACT_AT: f64 = 0.8;
/// How much of the window recent messages may keep after compacting.
const KEEP_SHARE: f64 = 0.35;
const SUMMARY_TOKENS: u32 = 1200;
/// Long messages are shortened to this many characters in what's summarized.
const MESSAGE_CHARS: usize = 2400;

const SUMMARY_PROMPT: &str = "You write summaries of conversations so an assistant can continue them without the \
full history. Summarize the conversation you're given: the user's goals and requests, decisions made, facts learned, \
files read or changed (with their paths), commands run and what they showed, and what's still open or next. Keep \
names, paths, numbers and error messages exact. Use short bullet points under a few headings. Don't add anything \
that isn't in the conversation.";

/// The conversation as a request sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// The newest summary, standing in for everything up to what it covers.
    pub summary: Option<String>,
    pub messages: Vec<Message>,
}

/// The newest summary, and the messages after what it covers (summaries left out).
pub fn view(path: &[Message]) -> View {
    let Some(last) = path.iter().rposition(|m| m.role == MessageRole::Summary) else {
        return View {
            summary: None,
            messages: path.to_vec(),
        };
    };
    let summary = &path[last];
    let start = summary
        .covers_until
        .as_deref()
        .and_then(|id| path.iter().position(|m| m.id == id))
        .map_or(last + 1, |covered| covered + 1);
    let messages = path[start..]
        .iter()
        .filter(|m| m.role != MessageRole::Summary)
        .cloned()
        .collect();
    View {
        summary: Some(summary.content.clone()),
        messages,
    }
}

/// Roughly how many tokens a text takes (4 characters each; good enough to plan with).
pub fn estimate_text(text: &str) -> u64 {
    (text.chars().count() as u64).div_ceil(4)
}

pub fn estimate(message: &Message) -> u64 {
    let calls = message
        .tool_calls
        .as_ref()
        .map_or(0, |calls| calls.to_string().len() as u64 / 4);
    estimate_text(&message.content)
        + message.reasoning.as_deref().map_or(0, estimate_text)
        + calls
        + 8
}

/// Tokens a reply reported for its whole conversation (prompt and reply).
pub fn usage_total(message: &Message) -> Option<u64> {
    let usage = message.usage.as_ref()?;
    let prompt = usage.get("promptTokens")?.as_u64()?;
    let completion = usage
        .get("completionTokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    (prompt > 0).then_some(prompt + completion)
}

/// How much of the window the next request will use: the newest reply's own count plus
/// estimates for what came after it, or an estimate of everything since the newest summary.
pub fn used(path: &[Message], system: &str) -> u64 {
    let last_summary = path.iter().rposition(|m| m.role == MessageRole::Summary);
    let last_usage = path
        .iter()
        .rposition(|m| m.role == MessageRole::Assistant && usage_total(m).is_some());
    match last_usage {
        Some(at) if last_summary.is_none_or(|summary| summary < at) => {
            usage_total(&path[at]).unwrap_or(0) + path[at + 1..].iter().map(estimate).sum::<u64>()
        }
        _ => {
            let view = view(path);
            estimate_text(system)
                + view.summary.as_deref().map_or(0, estimate_text)
                + view.messages.iter().map(estimate).sum::<u64>()
        }
    }
}

pub fn should_compact(used: u64, window: u64) -> bool {
    window > 0 && used as f64 > window as f64 * COMPACT_AT
}

/// Less than this before the cut isn't worth a summary.
const MIN_SUMMARIZED: u64 = 256;

/// Where to cut after the server said the request was too long (so estimates can't be trusted):
/// before the newest user message, or the newest step if that's all there is.
pub fn cut_forced(messages: &[Message]) -> Option<usize> {
    let worth = |i: usize| messages[..i].iter().map(estimate).sum::<u64>() >= MIN_SUMMARIZED / 4;
    let last_user = (1..messages.len())
        .rev()
        .find(|&i| messages[i].role == MessageRole::User && worth(i));
    last_user.or_else(|| {
        (1..messages.len())
            .rev()
            .find(|&i| messages[i].role == MessageRole::Assistant && worth(i))
    })
}

/// Where to cut so the messages from there on fit in `keep` tokens: before a user message or an
/// assistant message (never between a tool call and its result). `None` when there isn't enough
/// before the cut to be worth summarizing.
pub fn cut(messages: &[Message], window: u64) -> Option<usize> {
    let keep = (window as f64 * KEEP_SHARE) as u64;
    let mut tail = 0;
    let mut start = messages.len();
    for (index, message) in messages.iter().enumerate().rev() {
        tail += estimate(message);
        if tail > keep {
            break;
        }
        start = index;
    }
    // Keep at least the newest message, whatever its size.
    let start = start.min(messages.len().saturating_sub(1));
    (start..messages.len())
        .find(|&i| matches!(messages[i].role, MessageRole::User | MessageRole::Assistant))
        .filter(|&i| messages[..i].iter().map(estimate).sum::<u64>() >= MIN_SUMMARIZED)
}

fn shorten(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_owned();
    }
    let head: String = chars[..max * 3 / 4].iter().collect();
    let tail: String = chars[chars.len() - max / 4..].iter().collect();
    format!(
        "{head}\n… ({} characters left out) …\n{tail}",
        chars.len() - max
    )
}

/// The messages as a plain transcript for the summarizer, within `max_chars`.
pub fn render(messages: &[Message], max_chars: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for message in messages {
        let text = match message.role {
            MessageRole::User => format!("User: {}", shorten(&message.content, MESSAGE_CHARS)),
            MessageRole::Assistant => {
                let mut text = format!("Assistant: {}", shorten(&message.content, MESSAGE_CHARS));
                if let Some(calls) = message.tool_calls.as_ref().and_then(|c| c.as_array()) {
                    for call in calls {
                        let name = call["function"]["name"].as_str().unwrap_or("tool");
                        let args = call["function"]["arguments"].as_str().unwrap_or("");
                        text.push_str(&format!("\n  (calls {name} {})", shorten(args, 300)));
                    }
                }
                text
            }
            MessageRole::Tool => format!(
                "Result of {}: {}",
                message.tool_name.as_deref().unwrap_or("a tool"),
                shorten(&message.content, MESSAGE_CHARS)
            ),
            MessageRole::Summary => format!("(Earlier summary)\n{}", message.content),
        };
        parts.push(text);
    }
    let all = parts.join("\n\n");
    if all.chars().count() <= max_chars {
        return all;
    }
    // Too long even shortened: keep the start (the goals) and the most recent part.
    shorten(&all, max_chars)
}

/// A summary of `messages` (after `previous`, an earlier summary) written by the model.
pub async fn summarize(
    client: &LlmClient,
    model: &str,
    llama_extensions: bool,
    previous: Option<&str>,
    messages: &[Message],
    window: u64,
) -> Result<String, LlmError> {
    // The summarizing request has to fit in the window too.
    let budget = (window as usize).saturating_mul(4) * 55 / 100;
    let mut text = String::new();
    if let Some(previous) = previous {
        text.push_str(&format!(
            "A summary of what came before:\n{previous}\n\nWhat came after it:\n\n"
        ));
    }
    text.push_str(&render(messages, budget.max(4000)));
    let mut request = ChatRequest {
        model: model.to_owned(),
        messages: vec![ChatMessage::system(SUMMARY_PROMPT), ChatMessage::user(text)],
        max_tokens: Some(SUMMARY_TOKENS),
        ..Default::default()
    };
    request.sampling.temperature = Some(0.3);
    if llama_extensions {
        let mut kwargs = serde_json::Map::new();
        kwargs.insert("enable_thinking".into(), serde_json::json!(false));
        request.chat_template_kwargs = Some(kwargs);
    }
    let reply = client.complete(&request).await?;
    let summary = reply.content.trim();
    if summary.is_empty() {
        return Err(LlmError::Protocol(
            "the model wrote an empty summary".into(),
        ));
    }
    Ok(summary.to_owned())
}

/// The system prompt with the summary of the earlier conversation added.
pub fn with_summary(system: &str, summary: Option<&str>) -> String {
    match summary {
        Some(summary) => format!(
            "{system}\n\n<summary_of_earlier_conversation>\n{summary}\n</summary_of_earlier_conversation>\n\
             The messages before these were summarized above to save room."
        ),
        None => system.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::MessageStatus;

    fn message(id: &str, role: MessageRole, content: &str) -> Message {
        Message {
            id: id.into(),
            session_id: "s".into(),
            parent_id: None,
            role,
            turn: 1,
            step: 0,
            content: content.into(),
            reasoning: None,
            reasoning_ms: None,
            tool_calls: None,
            tool_call_id: None,
            tool_name: None,
            tool_status: None,
            tool_display: None,
            model_label: None,
            finish_reason: None,
            status: MessageStatus::Complete,
            error: None,
            usage: None,
            created_at: 0,
            covers_until: None,
        }
    }

    #[test]
    fn a_summary_stands_in_for_what_it_covers() {
        let mut summary = message("s1", MessageRole::Summary, "They fixed add().");
        summary.covers_until = Some("a1".into());
        let path = vec![
            message("u1", MessageRole::User, "Fix add()"),
            message("a1", MessageRole::Assistant, "Done."),
            message("u2", MessageRole::User, "Now sub()"),
            summary,
            message("a2", MessageRole::Assistant, "Looking."),
        ];
        let view = view(&path);
        assert_eq!(view.summary.as_deref(), Some("They fixed add()."));
        assert_eq!(
            view.messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["u2", "a2"]
        );
        assert!(
            with_summary("Rules.", view.summary.as_deref())
                .contains("<summary_of_earlier_conversation>\nThey fixed add().")
        );
    }

    #[test]
    fn usage_is_trusted_until_a_summary_replaces_part_of_it() {
        let mut reply = message("a1", MessageRole::Assistant, "Done.");
        reply.usage = Some(json!({ "promptTokens": 9000, "completionTokens": 500 }));
        let mut path = vec![
            message("u1", MessageRole::User, "Hi"),
            reply,
            message("t1", MessageRole::Tool, &"x".repeat(400)),
        ];
        assert_eq!(used(&path, "system"), 9500 + 100 + 8);
        let mut summary = message("s1", MessageRole::Summary, "Short.");
        summary.covers_until = Some("a1".into());
        path.push(summary);
        assert!(used(&path, "system") < 200, "{}", used(&path, "system"));
        assert!(
            should_compact(8100, 10_000) && !should_compact(7900, 10_000) && !should_compact(5, 0)
        );
    }

    #[test]
    fn cuts_keep_recent_messages_and_tool_results_with_their_calls() {
        let big = "y".repeat(4000);
        let path = vec![
            message("u1", MessageRole::User, &big),
            message("a1", MessageRole::Assistant, &big),
            message("t1", MessageRole::Tool, &big),
            message("a2", MessageRole::Assistant, &big),
            message("t2", MessageRole::Tool, "small"),
            message("u2", MessageRole::User, "next"),
        ];
        // A 2000-token window keeps 700 tokens: t2 and u2 fit, a2 doesn't. The cut moves on to
        // u2, since a cut before t2 would separate it from a2's call.
        assert_eq!(cut(&path, 2000), Some(5));
        // 4000 keeps 1400: a2 with its result and u2.
        assert_eq!(cut(&path, 4000), Some(3));
        // 9000 keeps all but the first message, which is big enough to be worth summarizing…
        assert_eq!(cut(&path, 9000), Some(1));
        // …unlike a small one.
        let small = [
            message("u1", MessageRole::User, "hi"),
            message("a1", MessageRole::Assistant, &big),
        ];
        assert_eq!(cut(&small, 100), None, "too little to summarize");
        // After an overflow: before the newest user message, whatever the estimates say.
        assert_eq!(cut_forced(&path), Some(5));
        assert_eq!(
            cut_forced(&path[..5]),
            Some(3),
            "no later user message: the newest step"
        );
    }

    #[test]
    fn nothing_to_cut_in_a_single_message() {
        assert_eq!(cut_forced(&[message("u1", MessageRole::User, "x")]), None);
    }

    #[test]
    fn transcripts_for_the_summarizer_are_shortened() {
        let mut call = message("a1", MessageRole::Assistant, "Reading.");
        call.tool_calls = Some(
            json!([{ "id": "c", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.py\"}" } }]),
        );
        let mut result = message("t1", MessageRole::Tool, &"z".repeat(5000));
        result.tool_name = Some("read_file".into());
        let text = render(
            &[message("u1", MessageRole::User, "Fix it"), call, result],
            100_000,
        );
        assert!(text.starts_with("User: Fix it\n\nAssistant: Reading.\n  (calls read_file {\"path\":\"a.py\"})\n\nResult of read_file: zzz"));
        assert!(text.contains("characters left out"));
        assert!(
            render(
                &[message("u1", MessageRole::User, &"q".repeat(10_000))],
                1000
            )
            .chars()
            .count()
                < 1200
        );
    }
}
