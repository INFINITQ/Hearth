//! Incremental parser for server-sent events (the `text/event-stream` format).
//!
//! Bytes arrive in arbitrary chunks: a line, a UTF-8 character or a `\r\n` pair can be split
//! across two of them. The parser buffers bytes and only decodes complete lines.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    has_data: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk and returns the events it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        let mut start = 0;
        while let Some(offset) = self.buffer[start..].iter().position(|&b| b == b'\n') {
            let end = start + offset;
            let line = decode_line(&self.buffer[start..end]);
            start = end + 1;
            if let Some(event) = self.line(&line) {
                events.push(event);
            }
        }
        self.buffer.drain(..start);
        events
    }

    /// The stream ended: completes an event that wasn't followed by a blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            let line = decode_line(&rest);
            if let Some(event) = self.line(&line) {
                return Some(event);
            }
        }
        self.dispatch()
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None; // a comment (servers send these as keep-alives)
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => {
                self.data.push(value.to_owned());
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_owned()),
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        if !self.has_data {
            self.event = None;
            return None;
        }
        let event = SseEvent {
            event: self.event.take(),
            data: self.data.join("\n"),
        };
        self.data.clear();
        self.has_data = false;
        Some(event)
    }
}

fn decode_line(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(events: &[SseEvent]) -> Vec<&str> {
        events.iter().map(|e| e.data.as_str()).collect()
    }

    #[test]
    fn parses_events_separated_by_blank_lines() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: {\"a\":1}\n\ndata: [DONE]\n\n");
        assert_eq!(data(&events), ["{\"a\":1}", "[DONE]"]);
    }

    #[test]
    fn handles_lines_and_utf8_characters_split_across_chunks() {
        let mut parser = SseParser::new();
        let text = "data: héllo ✓\n\n".as_bytes();
        let mut events = Vec::new();
        for byte in text {
            events.extend(parser.push(std::slice::from_ref(byte)));
        }
        assert_eq!(data(&events), ["héllo ✓"]);
    }

    #[test]
    fn handles_crlf_line_endings() {
        let mut parser = SseParser::new();
        let mut events = parser.push(b"data: one\r");
        events.extend(parser.push(b"\n\r\ndata: two\r\n\r\n"));
        assert_eq!(data(&events), ["one", "two"]);
    }

    #[test]
    fn skips_comments_and_joins_multi_line_data() {
        let mut parser = SseParser::new();
        let events = parser.push(b": keep-alive\nevent: update\ndata: a\ndata: b\n\n");
        assert_eq!(
            events,
            [SseEvent {
                event: Some("update".into()),
                data: "a\nb".into()
            }]
        );
    }

    #[test]
    fn finish_completes_an_event_without_a_trailing_blank_line() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: last").is_empty());
        assert_eq!(parser.finish().map(|e| e.data), Some("last".into()));
        assert_eq!(parser.finish(), None);
    }
}
