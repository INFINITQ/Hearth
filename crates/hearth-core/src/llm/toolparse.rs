//! Tool calls that arrive as plain text, and arguments that aren't quite JSON.
//!
//! When a server's chat template doesn't turn a model's tool call into structured `tool_calls`,
//! the call ends up in the reply text in one of a few well-known shapes. These parsers find them
//! (only for tools that exist, so prose about tools isn't mistaken for a call) and remove the
//! markup from the visible text. `parse_arguments` repairs the usual JSON slips.

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct TextToolCall {
    pub name: String,
    pub arguments: Value,
}

/// Finds tool calls written into `text`. Returns the calls and the text without their markup.
pub fn extract(text: &str, known: &dyn Fn(&str) -> bool) -> (Vec<TextToolCall>, String) {
    let mut calls = Vec::new();
    let mut rest = text.to_owned();
    for parser in [hermes as Parser, qwen_xml, mistral, python_tag, fenced_json] {
        let (found, remaining) = parser(&rest, known);
        if !found.is_empty() {
            calls.extend(found);
            rest = remaining;
        }
    }
    (calls, rest.trim().to_owned())
}

type Parser = fn(&str, &dyn Fn(&str) -> bool) -> (Vec<TextToolCall>, String);

/// `{"name": "...", "arguments"|"parameters": {...}}` from a JSON value.
fn call_from_json(value: &Value, known: &dyn Fn(&str) -> bool) -> Option<TextToolCall> {
    let object = value.as_object()?;
    let (name, arguments) = match object.get("function").and_then(Value::as_object) {
        Some(function) => (
            function.get("name")?,
            function
                .get("arguments")
                .or_else(|| function.get("parameters")),
        ),
        None => (
            object.get("name")?,
            object.get("arguments").or_else(|| object.get("parameters")),
        ),
    };
    let name = name.as_str()?;
    if !known(name) {
        return None;
    }
    let arguments = match arguments {
        Some(Value::String(text)) => parse_arguments(text).unwrap_or(json!({})),
        Some(value) => value.clone(),
        None => json!({}),
    };
    Some(TextToolCall {
        name: name.to_owned(),
        arguments,
    })
}

/// Text between `open` and `close` markers (a missing close runs to the end).
fn blocks<'a>(text: &'a str, open: &str, close: &str) -> Vec<(usize, usize, &'a str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find(open).map(|i| from + i) {
        let body_start = start + open.len();
        let (body_end, end) = match text[body_start..].find(close) {
            Some(i) => (body_start + i, body_start + i + close.len()),
            None => (text.len(), text.len()),
        };
        out.push((start, end, &text[body_start..body_end]));
        from = end;
    }
    out
}

fn remove(text: &str, spans: &[(usize, usize)]) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    for &(start, end) in spans {
        out.push_str(&text[cursor..start]);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// `<tool_call>{"name": ..., "arguments": ...}</tool_call>` (Hermes, Qwen 2.5/3).
fn hermes(text: &str, known: &dyn Fn(&str) -> bool) -> (Vec<TextToolCall>, String) {
    let mut calls = Vec::new();
    let mut spans = Vec::new();
    for (start, end, body) in blocks(text, "<tool_call>", "</tool_call>") {
        let body = body.trim();
        if body.starts_with('{')
            && let Ok(value) = parse_arguments(body)
            && let Some(call) = call_from_json(&value, known)
        {
            calls.push(call);
            spans.push((start, end));
        }
    }
    (calls, remove(text, &spans))
}

/// Qwen3-Coder: `<function=name><parameter=key>value</parameter></function>`, optionally inside
/// `<tool_call>…</tool_call>`.
fn qwen_xml(text: &str, known: &dyn Fn(&str) -> bool) -> (Vec<TextToolCall>, String) {
    let mut calls = Vec::new();
    let mut spans = Vec::new();
    for (start, end, body) in blocks(text, "<function=", "</function>") {
        let Some((name, params)) = body.split_once('>') else {
            continue;
        };
        let name = name.trim();
        if !known(name) {
            continue;
        }
        let mut arguments = Map::new();
        for (_, _, param) in blocks(params, "<parameter=", "</parameter>") {
            if let Some((key, value)) = param.split_once('>') {
                let value = value.strip_prefix('\n').unwrap_or(value);
                let value = value.strip_suffix('\n').unwrap_or(value);
                let parsed = match serde_json::from_str::<Value>(value.trim()) {
                    Ok(
                        v @ (Value::Number(_)
                        | Value::Bool(_)
                        | Value::Array(_)
                        | Value::Object(_)
                        | Value::Null),
                    ) => v,
                    _ => Value::String(value.to_owned()),
                };
                arguments.insert(key.trim().to_owned(), parsed);
            }
        }
        calls.push(TextToolCall {
            name: name.to_owned(),
            arguments: Value::Object(arguments),
        });
        // Take the surrounding <tool_call> wrapper too, if there is one.
        let wrapped_start = text[..start]
            .trim_end()
            .strip_suffix("<tool_call>")
            .map(str::len)
            .unwrap_or(start);
        let after = &text[end..];
        let wrapped_end = after
            .trim_start()
            .strip_prefix("</tool_call>")
            .map(|rest| text.len() - rest.len())
            .unwrap_or(end);
        spans.push((wrapped_start, wrapped_end));
    }
    (calls, remove(text, &spans))
}

/// Mistral: `[TOOL_CALLS][{"name": ..., "arguments": ...}]`.
fn mistral(text: &str, known: &dyn Fn(&str) -> bool) -> (Vec<TextToolCall>, String) {
    let Some(start) = text.find("[TOOL_CALLS]") else {
        return (Vec::new(), text.to_owned());
    };
    let body = text[start + "[TOOL_CALLS]".len()..].trim();
    let Ok(Value::Array(items)) = parse_arguments(body) else {
        return (Vec::new(), text.to_owned());
    };
    let calls: Vec<TextToolCall> = items
        .iter()
        .filter_map(|item| call_from_json(item, known))
        .collect();
    let rest = if calls.is_empty() {
        text.to_owned()
    } else {
        text[..start].to_owned()
    };
    (calls, rest)
}

/// Llama 3: `<|python_tag|>{"name": ..., "parameters": ...}`.
fn python_tag(text: &str, known: &dyn Fn(&str) -> bool) -> (Vec<TextToolCall>, String) {
    let Some(start) = text.find("<|python_tag|>") else {
        return (Vec::new(), text.to_owned());
    };
    let body = text[start + "<|python_tag|>".len()..]
        .trim()
        .trim_end_matches("<|eom_id|>")
        .trim();
    match parse_arguments(body)
        .ok()
        .and_then(|v| call_from_json(&v, known))
    {
        Some(call) => (vec![call], text[..start].to_owned()),
        None => (Vec::new(), text.to_owned()),
    }
}

/// A fenced JSON block whose content is a tool call: ```json {"name": "read_file", …} ```.
fn fenced_json(text: &str, known: &dyn Fn(&str) -> bool) -> (Vec<TextToolCall>, String) {
    let mut calls = Vec::new();
    let mut spans = Vec::new();
    for (start, end, body) in blocks(text, "```", "```") {
        let body = body.strip_prefix("json").unwrap_or(body).trim();
        if !body.starts_with('{') {
            continue;
        }
        if let Some(call) = parse_arguments(body)
            .ok()
            .and_then(|v| call_from_json(&v, known))
        {
            calls.push(call);
            spans.push((start, end));
        }
    }
    (calls, remove(text, &spans))
}

/// Parses tool arguments, repairing what models commonly get wrong: code fences, trailing
/// commas, single quotes, Python's `True`/`False`/`None`, raw newlines inside strings, a
/// truncated end, and JSON encoded twice.
pub fn parse_arguments(raw: &str) -> Result<Value, String> {
    let trimmed = strip_fences(raw.trim());
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(unwrap_double(value));
    }
    let repaired = repair(trimmed);
    serde_json::from_str::<Value>(&repaired)
        .map(unwrap_double)
        .map_err(|e| e.to_string())
}

fn strip_fences(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

/// `"{\"path\": \"x\"}"` (a JSON string holding JSON) → the inner value.
fn unwrap_double(value: Value) -> Value {
    if let Value::String(text) = &value
        && let Ok(inner @ Value::Object(_)) = serde_json::from_str::<Value>(text)
    {
        return inner;
    }
    value
}

fn is_word_at(chars: &[char], at: usize, word: &str) -> bool {
    let len = word.chars().count();
    let matches = chars.len() >= at + len && chars[at..at + len].iter().copied().eq(word.chars());
    let before_ok = at == 0 || !chars[at - 1].is_alphanumeric();
    let after_ok = chars.get(at + len).is_none_or(|c| !c.is_alphanumeric());
    matches && before_ok && after_ok
}

fn trim_trailing_comma(out: &mut String) {
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    if out.ends_with(',') {
        out.pop();
    }
}

fn repair(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let mut closers: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if escaped {
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                out.push(c);
                escaped = true;
            } else if c == q {
                out.push('"');
                quote = None;
            } else {
                match c {
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(c),
                }
            }
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                out.push('"');
            }
            '{' => {
                closers.push('}');
                out.push(c);
            }
            '[' => {
                closers.push(']');
                out.push(c);
            }
            '}' | ']' => {
                trim_trailing_comma(&mut out);
                closers.pop();
                out.push(c);
            }
            _ => {
                let literal = [("True", "true"), ("False", "false"), ("None", "null")]
                    .into_iter()
                    .find(|(word, _)| is_word_at(&chars, i, word));
                if let Some((word, replacement)) = literal {
                    out.push_str(replacement);
                    i += word.len();
                    continue;
                }
                out.push(c);
            }
        }
        i += 1;
    }
    if quote.is_some() {
        out.push('"');
    }
    trim_trailing_comma(&mut out);
    while let Some(closer) = closers.pop() {
        out.push(closer);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(name: &str) -> bool {
        matches!(name, "read_file" | "edit_file" | "run_command")
    }

    #[test]
    fn hermes_tool_call_tags() {
        let text = "Let me look.\n<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}}\n</tool_call>";
        let (calls, rest) = extract(text, &known);
        assert_eq!(
            calls,
            [TextToolCall {
                name: "read_file".into(),
                arguments: json!({ "path": "a.rs" })
            }]
        );
        assert_eq!(rest, "Let me look.");
    }

    #[test]
    fn qwen3_coder_xml() {
        let text = "<tool_call>\n<function=edit_file>\n<parameter=path>\nsrc/a.rs\n</parameter>\n<parameter=old_string>\nlet x = 1;\n</parameter>\n<parameter=replace_all>\nfalse\n</parameter>\n</function>\n</tool_call>";
        let (calls, rest) = extract(text, &known);
        assert_eq!(calls[0].name, "edit_file");
        assert_eq!(
            calls[0].arguments,
            json!({ "path": "src/a.rs", "old_string": "let x = 1;", "replace_all": false })
        );
        assert_eq!(rest, "");
    }

    #[test]
    fn mistral_python_tag_and_fenced_json() {
        let (calls, _) = extract(
            "[TOOL_CALLS][{\"name\": \"run_command\", \"arguments\": {\"command\": \"ls\"}}]",
            &known,
        );
        assert_eq!(calls[0].arguments, json!({ "command": "ls" }));
        let (calls, _) = extract(
            "<|python_tag|>{\"name\": \"read_file\", \"parameters\": {\"path\": \"x\"}}<|eom_id|>",
            &known,
        );
        assert_eq!(calls[0].name, "read_file");
        let (calls, rest) = extract(
            "Here:\n```json\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"y\"}}\n```",
            &known,
        );
        assert_eq!(calls[0].arguments, json!({ "path": "y" }));
        assert_eq!(rest, "Here:");
    }

    #[test]
    fn unknown_tools_and_ordinary_code_stay_text() {
        let text = "Use this config:\n```json\n{\"name\": \"my-app\", \"version\": \"1.0\"}\n```";
        let (calls, rest) = extract(text, &known);
        assert!(calls.is_empty());
        assert_eq!(rest, text);
    }

    #[test]
    fn broken_arguments_are_repaired() {
        assert_eq!(parse_arguments("{\"a\": 1,}").unwrap(), json!({ "a": 1 }));
        assert_eq!(
            parse_arguments("{'path': 'x.rs', 'all': True}").unwrap(),
            json!({ "path": "x.rs", "all": true })
        );
        assert_eq!(
            parse_arguments("{\"text\": \"line1\nline2\"}").unwrap(),
            json!({ "text": "line1\nline2" })
        );
        assert_eq!(
            parse_arguments("{\"path\": \"x.rs\", \"content\": \"fn main() {").unwrap(),
            json!({ "path": "x.rs", "content": "fn main() {" })
        );
        assert_eq!(
            parse_arguments("\"{\\\"path\\\": \\\"z\\\"}\"").unwrap(),
            json!({ "path": "z" })
        );
        assert_eq!(
            parse_arguments("```json\n{\"a\": None}\n```").unwrap(),
            json!({ "a": null })
        );
        assert_eq!(parse_arguments("  ").unwrap(), json!({}));
        assert!(parse_arguments("not json at all").is_err());
    }
}
