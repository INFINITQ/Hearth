//! Terminals for the terminal panel: shells (Git Bash or PowerShell) in pseudo consoles.
//!
//! A terminal lives in Rust, so it keeps running while the UI reloads or shows something else.
//! The UI attaches to it and first gets what it showed so far (its scrollback).

mod pty;

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;

use crate::agent::shell::{Shell, ShellKind};
use crate::{new_id, now_ms};
pub use pty::{Pty, Spawn};

/// How much output a terminal keeps for the UI to replay.
const SCROLLBACK: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub shell: ShellKind,
    pub cwd: String,
    /// The project it was opened in (the UI shows a project's own terminals).
    pub project_id: Option<String>,
    pub running: bool,
    pub exit_code: Option<i64>,
    pub created_at: i64,
}

/// What a terminal sends the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalEvent {
    Output {
        data: String,
    },
    /// The shell ended (after its last output).
    Exit {
        code: Option<i64>,
    },
}

/// Where a terminal's events go (the UI's channel).
pub trait TerminalSink: Send + Sync {
    fn send(&self, event: TerminalEvent);
}

/// What runs in a terminal.
#[derive(Debug, Clone)]
pub struct Launch {
    pub kind: ShellKind,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Launch {
    /// An interactive shell.
    pub fn shell(shell: &Shell) -> Launch {
        let mut env = vec![
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ];
        let args = match shell.kind {
            ShellKind::GitBash => {
                // A login shell would otherwise start in the home folder.
                env.push(("CHERE_INVOKING".to_owned(), "1".to_owned()));
                vec!["--login".to_owned(), "-i".to_owned()]
            }
            ShellKind::PowerShell => vec!["-NoLogo".to_owned()],
        };
        Launch {
            kind: shell.kind,
            program: shell.path.clone(),
            args,
            env,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Turns output bytes into text, keeping a character split between two reads whole.
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(err) => {
                    let (valid, after) = rest.split_at(err.valid_up_to());
                    text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match err.error_len() {
                        Some(bad) => {
                            text.push('\u{FFFD}');
                            rest = &after[bad..];
                        }
                        // The start of a character whose other bytes come with the next read.
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        text
    }
}

/// Drops the oldest output once there's too much, starting the rest at a line.
fn trim(text: &mut String) {
    if text.len() <= SCROLLBACK {
        return;
    }
    let mut cut = text.len() - SCROLLBACK * 3 / 4;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    if let Some(newline) = text[cut..].find('\n') {
        cut += newline + 1;
    }
    text.drain(..cut);
}

struct State {
    info: TerminalInfo,
    scrollback: String,
    sink: Option<Arc<dyn TerminalSink>>,
}

struct Terminal {
    id: String,
    pty: Pty,
    state: Mutex<State>,
}

impl Terminal {
    /// Passes output on until the console closes, then says the shell ended.
    fn read(&self, mut output: File) {
        let mut decoder = Utf8Stream::default();
        let mut buffer = vec![0u8; 32 * 1024];
        loop {
            let read = match output.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let data = decoder.push(&buffer[..read]);
            if data.is_empty() {
                continue;
            }
            let mut state = lock(&self.state);
            state.scrollback.push_str(&data);
            trim(&mut state.scrollback);
            if let Some(sink) = &state.sink {
                sink.send(TerminalEvent::Output { data });
            }
        }
        let mut state = lock(&self.state);
        state.info.running = false;
        let code = state.info.exit_code;
        if let Some(sink) = &state.sink {
            sink.send(TerminalEvent::Exit { code });
        }
    }

    /// When the shell ends, closes the console so its output ends too.
    fn wait(&self) {
        let code = self.pty.wait();
        lock(&self.state).info.exit_code = code;
        // A moment for the console to pass on the shell's last words.
        std::thread::sleep(Duration::from_millis(150));
        self.pty.close_console();
    }
}

/// The open terminals, oldest first.
#[derive(Default)]
pub struct Terminals {
    all: Mutex<Vec<Arc<Terminal>>>,
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "This terminal has closed.")
}

impl Terminals {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a shell in `cwd`, sized `cols` × `rows`.
    pub fn open(
        &self,
        launch: &Launch,
        cwd: &Path,
        project_id: Option<String>,
        cols: u16,
        rows: u16,
    ) -> io::Result<TerminalInfo> {
        let args: Vec<&str> = launch.args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = launch
            .env
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let (pty, output) = Pty::spawn(&Spawn {
            program: &launch.program,
            args: &args,
            cwd,
            env: &env,
            cols,
            rows,
        })?;
        let info = TerminalInfo {
            id: new_id(),
            shell: launch.kind,
            cwd: cwd.display().to_string(),
            project_id,
            running: true,
            exit_code: None,
            created_at: now_ms(),
        };
        let terminal = Arc::new(Terminal {
            id: info.id.clone(),
            pty,
            state: Mutex::new(State {
                info: info.clone(),
                scrollback: String::new(),
                sink: None,
            }),
        });
        lock(&self.all).push(terminal.clone());
        let reader = terminal.clone();
        std::thread::Builder::new()
            .name("terminal-output".into())
            .spawn(move || reader.read(output))?;
        let waiter = terminal;
        std::thread::Builder::new()
            .name("terminal-wait".into())
            .spawn(move || waiter.wait())?;
        Ok(info)
    }

    fn find(&self, id: &str) -> io::Result<Arc<Terminal>> {
        lock(&self.all)
            .iter()
            .find(|terminal| terminal.id == id)
            .cloned()
            .ok_or_else(closed)
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        lock(&self.all)
            .iter()
            .map(|terminal| lock(&terminal.state).info.clone())
            .collect()
    }

    /// Sends the terminal's events to `sink` (replacing an earlier one), starting with what it
    /// showed so far.
    pub fn attach(&self, id: &str, sink: Arc<dyn TerminalSink>) -> io::Result<()> {
        let terminal = self.find(id)?;
        let mut state = lock(&terminal.state);
        if !state.scrollback.is_empty() {
            sink.send(TerminalEvent::Output {
                data: state.scrollback.clone(),
            });
        }
        if !state.info.running {
            sink.send(TerminalEvent::Exit {
                code: state.info.exit_code,
            });
        }
        state.sink = Some(sink);
        Ok(())
    }

    /// Types `data` into the terminal.
    pub fn write(&self, id: &str, data: &str) -> io::Result<()> {
        self.find(id)?.pty.write(data.as_bytes())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> io::Result<()> {
        self.find(id)?.pty.resize(cols, rows)
    }

    /// Ends the shell and everything started in it.
    pub fn close(&self, id: &str) {
        let removed = {
            let mut all = lock(&self.all);
            all.iter()
                .position(|terminal| terminal.id == id)
                .map(|index| all.remove(index))
        };
        if let Some(terminal) = removed {
            // Closing the console waits for it to finish; nobody needs to wait for that.
            let _ = std::thread::Builder::new()
                .name("terminal-close".into())
                .spawn(move || terminal.pty.kill());
        }
    }

    pub fn close_all(&self) {
        let all = std::mem::take(&mut *lock(&self.all));
        for terminal in all {
            terminal.pty.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[derive(Default)]
    struct Collect(Mutex<Vec<TerminalEvent>>);

    impl TerminalSink for Collect {
        fn send(&self, event: TerminalEvent) {
            lock(&self.0).push(event);
        }
    }

    impl Collect {
        fn text(&self) -> String {
            let mut text = String::new();
            for event in lock(&self.0).iter() {
                if let TerminalEvent::Output { data } = event {
                    text.push_str(data);
                }
            }
            text
        }

        fn exited(&self) -> Option<Option<i64>> {
            lock(&self.0).iter().find_map(|event| match event {
                TerminalEvent::Exit { code } => Some(*code),
                TerminalEvent::Output { .. } => None,
            })
        }
    }

    fn until(what: &str, check: impl Fn() -> bool) {
        let start = Instant::now();
        while !check() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// cmd.exe starts fastest and is on every Windows.
    fn cmd() -> Launch {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        Launch {
            kind: ShellKind::PowerShell,
            program: PathBuf::from(root).join("System32\\cmd.exe"),
            args: vec!["/d".into(), "/q".into()],
            env: Vec::new(),
        }
    }

    #[test]
    fn a_character_split_between_reads_stays_whole() {
        let mut stream = Utf8Stream::default();
        let bytes = "é✓".as_bytes();
        assert_eq!(stream.push(&bytes[..1]), "");
        assert_eq!(stream.push(&bytes[1..3]), "é");
        assert_eq!(stream.push(&bytes[3..]), "✓");
        assert_eq!(stream.push(&[0xff, b'a']), "\u{FFFD}a");
    }

    #[test]
    fn the_scrollback_keeps_the_newest_lines() {
        let mut text: String = (0..40_000).map(|n| format!("line {n}\n")).collect();
        trim(&mut text);
        assert!(text.len() <= SCROLLBACK);
        assert!(text.starts_with("line "));
        assert!(text.ends_with("line 39999\n"));
    }

    #[test]
    fn a_terminal_runs_what_is_typed_and_says_how_it_ended() {
        let terminals = Terminals::new();
        let info = terminals
            .open(&cmd(), &std::env::temp_dir(), Some("p1".into()), 80, 24)
            .unwrap();
        assert!(info.running);
        let sink = Arc::new(Collect::default());
        terminals.attach(&info.id, sink.clone()).unwrap();
        terminals.write(&info.id, "set /a 123*1000+456\r").unwrap();
        until("the sum", || sink.text().contains("123456"));
        terminals.resize(&info.id, 100, 30).unwrap();
        terminals.write(&info.id, "exit 3\r").unwrap();
        until("the exit", || sink.exited().is_some());
        assert_eq!(sink.exited(), Some(Some(3)));
        let listed = terminals.list();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].running);
        assert_eq!(listed[0].project_id.as_deref(), Some("p1"));

        // A UI that attaches later (after a reload) gets what it showed, and that it ended.
        let late = Arc::new(Collect::default());
        terminals.attach(&info.id, late.clone()).unwrap();
        assert!(late.text().contains("123456"));
        assert_eq!(late.exited(), Some(Some(3)));

        terminals.close(&info.id);
        assert!(terminals.list().is_empty());
        assert!(terminals.write(&info.id, "x").is_err());
    }

    #[test]
    fn closing_a_terminal_ends_what_runs_in_it() {
        let terminals = Terminals::new();
        let info = terminals
            .open(&cmd(), &std::env::temp_dir(), None, 80, 24)
            .unwrap();
        let sink = Arc::new(Collect::default());
        terminals.attach(&info.id, sink.clone()).unwrap();
        terminals.write(&info.id, "ping -n 60 127.0.0.1\r").unwrap();
        until("ping", || sink.text().contains("127.0.0.1"));
        let started = Instant::now();
        terminals.close(&info.id);
        until("the end", || sink.exited().is_some());
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
