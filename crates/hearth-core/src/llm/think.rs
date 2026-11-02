//! Splits `<think>…</think>` out of streamed content, for servers that don't separate reasoning
//! themselves (llama-server with `--reasoning-format none`, some LM Studio or Ollama setups).
//!
//! Tags may arrive split across deltas, so text that could be the start of a tag is held back
//! until the next delta decides it.

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing visible yet: the reply may start with `<think>`.
    Start,
    Thinking,
    Answer,
}

/// Reasoning and answer text produced by one delta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    pub reasoning: String,
    pub content: String,
}

#[derive(Debug)]
pub struct ThinkSplitter {
    state: State,
    pending: String,
    /// Strip the blank lines that follow `</think>`.
    trim_answer_start: bool,
}

impl ThinkSplitter {
    /// `starts_open` is for chat templates that open the `<think>` block in the prompt, so the
    /// reply begins inside it.
    pub fn new(starts_open: bool) -> Self {
        Self {
            state: if starts_open {
                State::Thinking
            } else {
                State::Start
            },
            pending: String::new(),
            trim_answer_start: false,
        }
    }

    pub fn push(&mut self, delta: &str) -> Split {
        self.pending.push_str(delta);
        let mut out = Split::default();
        loop {
            match self.state {
                State::Start => {
                    let trimmed = self.pending.trim_start();
                    if let Some(rest) = trimmed.strip_prefix(OPEN) {
                        self.pending = rest.to_owned();
                        self.state = State::Thinking;
                    } else if OPEN.starts_with(trimmed) {
                        return out; // could still become `<think>`: wait for more
                    } else {
                        self.state = State::Answer;
                    }
                }
                State::Thinking => match self.pending.find(CLOSE) {
                    Some(at) => {
                        out.reasoning.push_str(&self.pending[..at]);
                        self.pending = self.pending[at + CLOSE.len()..].to_owned();
                        self.state = State::Answer;
                        self.trim_answer_start = true;
                    }
                    None => {
                        let keep = partial_suffix(&self.pending, CLOSE);
                        let emit = self.pending.len() - keep;
                        out.reasoning.push_str(&self.pending[..emit]);
                        self.pending.drain(..emit);
                        return out;
                    }
                },
                State::Answer => {
                    if self.trim_answer_start {
                        let trimmed = self.pending.trim_start_matches(['\n', '\r']);
                        if trimmed.is_empty() {
                            self.pending.clear();
                            return out;
                        }
                        self.pending = trimmed.to_owned();
                        self.trim_answer_start = false;
                    }
                    out.content.push_str(&self.pending);
                    self.pending.clear();
                    return out;
                }
            }
        }
    }

    /// The stream ended: whatever was held back is released.
    pub fn finish(&mut self) -> Split {
        let rest = std::mem::take(&mut self.pending);
        match self.state {
            State::Thinking => Split {
                reasoning: rest,
                content: String::new(),
            },
            State::Start | State::Answer => Split {
                reasoning: String::new(),
                content: rest,
            },
        }
    }
}

/// Length of the longest suffix of `text` that is a proper prefix of `tag`.
fn partial_suffix(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&n| {
            text.len() >= n
                && text.is_char_boundary(text.len() - n)
                && tag.starts_with(&text[text.len() - n..])
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(splitter: &mut ThinkSplitter, deltas: &[&str]) -> Split {
        let mut total = Split::default();
        for delta in deltas {
            let split = splitter.push(delta);
            total.reasoning.push_str(&split.reasoning);
            total.content.push_str(&split.content);
        }
        let rest = splitter.finish();
        total.reasoning.push_str(&rest.reasoning);
        total.content.push_str(&rest.content);
        total
    }

    #[test]
    fn separates_a_think_block_split_across_deltas() {
        let mut splitter = ThinkSplitter::new(false);
        let split = run(
            &mut splitter,
            &[
                "<th",
                "ink>Let me ",
                "see.</thi",
                "nk>\n\nThe answer",
                " is 4.",
            ],
        );
        assert_eq!(split.reasoning, "Let me see.");
        assert_eq!(split.content, "The answer is 4.");
    }

    #[test]
    fn passes_plain_answers_through() {
        let mut splitter = ThinkSplitter::new(false);
        let split = run(&mut splitter, &["Hello", " <b>world</b>"]);
        assert_eq!(
            split,
            Split {
                reasoning: String::new(),
                content: "Hello <b>world</b>".into()
            }
        );
    }

    #[test]
    fn handles_templates_that_open_the_block_in_the_prompt() {
        let mut splitter = ThinkSplitter::new(true);
        let split = run(&mut splitter, &["hmm…", "</think>", "Done."]);
        assert_eq!(split.reasoning, "hmm…");
        assert_eq!(split.content, "Done.");
    }

    #[test]
    fn an_unfinished_think_block_stays_reasoning() {
        let mut splitter = ThinkSplitter::new(false);
        let split = run(&mut splitter, &["<think>still going </"]);
        assert_eq!(split.reasoning, "still going </");
        assert_eq!(split.content, "");
    }
}
