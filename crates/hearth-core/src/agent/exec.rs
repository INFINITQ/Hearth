//! Running the agent's shell commands: in the foreground (output streamed as it comes, with a
//! timeout and Stop), or in the background (servers and watchers, checked later). Each command
//! runs in its own Job Object, so stopping it ends everything it started.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::shell::{Shell, clean, decode};
use crate::proc;

/// The start of a command's output is kept whole; after that only the newest part.
const HEAD_BYTES: usize = 64 * 1024;
const TAIL_BYTES: usize = 1024 * 1024;
/// How long pipes may stay open after the command ended (a process it started in the
/// background still holding them) before that process is stopped too.
const PIPE_GRACE: Duration = Duration::from_secs(2);
/// How often live output is passed on.
const LIVE_EVERY: Duration = Duration::from_millis(150);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A command's output: its beginning and its most recent part, whatever its length.
#[derive(Default)]
pub struct OutputBuffer {
    head: Vec<u8>,
    tail: Vec<u8>,
    dropped: usize,
}

impl OutputBuffer {
    pub fn push(&mut self, bytes: &[u8]) {
        let room = HEAD_BYTES.saturating_sub(self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..room]);
        self.tail.extend_from_slice(&bytes[room..]);
        if self.tail.len() > TAIL_BYTES {
            let mut cut = self.tail.len() - TAIL_BYTES;
            // Don't start in the middle of a UTF-8 character.
            while cut < self.tail.len() && (self.tail[cut] & 0xC0) == 0x80 {
                cut += 1;
            }
            self.tail.drain(..cut);
            self.dropped += cut;
        }
    }

    /// The output, cleaned for reading.
    pub fn text(&self) -> String {
        let head = clean(&decode(&self.head));
        if self.tail.is_empty() {
            return head;
        }
        let tail = clean(&decode(&self.tail));
        if self.dropped == 0 {
            // Split at an arbitrary byte: decode it as one piece.
            return clean(&decode(
                &[self.head.as_slice(), self.tail.as_slice()].concat(),
            ));
        }
        format!(
            "{head}\n… ({} KB of output left out) …\n{tail}",
            self.dropped / 1024
        )
    }
}

/// How a foreground command ended.
pub struct Ran {
    pub output: String,
    /// `None` when it was stopped (timeout or Stop).
    pub exit: Option<i32>,
    pub elapsed: Duration,
    pub timed_out: bool,
    pub cancelled: bool,
    /// Processes it left running in the background were stopped.
    pub stopped_leftovers: bool,
}

fn read_into(
    mut pipe: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    tx: mpsc::UnboundedSender<Vec<u8>>,
) {
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            match pipe.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buffer[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

fn start(
    shell: &Shell,
    command: &str,
    cwd: &Path,
) -> io::Result<(
    proc::Process,
    Option<super::shell::Script>,
    mpsc::UnboundedReceiver<Vec<u8>>,
)> {
    let (mut cmd, script) = shell.command(command, cwd)?;
    let mut process = proc::spawn(&mut cmd)?;
    let (tx, rx) = mpsc::unbounded_channel();
    if let Some(stdout) = process.child.stdout.take() {
        read_into(stdout, tx.clone());
    }
    if let Some(stderr) = process.child.stderr.take() {
        read_into(stderr, tx);
    }
    Ok((process, script, rx))
}

/// Runs a command to the end (or the timeout, or Stop), passing output on as it arrives.
pub async fn run(
    shell: &Shell,
    command: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
    mut live: impl FnMut(String),
) -> io::Result<Ran> {
    let started = Instant::now();
    let (mut process, _script, mut rx) = start(shell, command, cwd)?;
    let mut buffer = OutputBuffer::default();
    let mut pending: Vec<u8> = Vec::new();
    let mut exit: Option<Option<i32>> = None;
    let mut pipes_open = true;
    let (mut timed_out, mut cancelled, mut stopped_leftovers) = (false, false, false);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(LIVE_EVERY);
    let mut grace: Option<Instant> = None;

    while exit.is_none() || pipes_open {
        tokio::select! {
            chunk = rx.recv(), if pipes_open => match chunk {
                Some(chunk) => {
                    buffer.push(&chunk);
                    pending.extend_from_slice(&chunk);
                }
                None => pipes_open = false,
            },
            status = process.child.wait(), if exit.is_none() => {
                exit = Some(status.ok().and_then(|s| s.code()));
                grace = Some(Instant::now() + PIPE_GRACE);
            }
            () = &mut deadline, if exit.is_none() => {
                timed_out = true;
                process.job.kill();
            }
            () = cancel.cancelled(), if exit.is_none() => {
                cancelled = true;
                process.job.kill();
            }
            _ = tick.tick() => {
                if !pending.is_empty() {
                    live(clean(&String::from_utf8_lossy(&std::mem::take(&mut pending))));
                }
                if grace.is_some_and(|until| Instant::now() >= until) && pipes_open {
                    // The command ended but something it started still holds its output open.
                    stopped_leftovers = true;
                    process.job.kill();
                    grace = None;
                }
            }
        }
    }
    if !pending.is_empty() {
        live(clean(&String::from_utf8_lossy(&pending)));
    }
    let stopped = timed_out || cancelled;
    Ok(Ran {
        output: buffer.text(),
        exit: if stopped { None } else { exit.flatten() },
        elapsed: started.elapsed(),
        timed_out,
        cancelled,
        stopped_leftovers,
    })
}

/// A command running in the background.
pub struct Background {
    pub id: u32,
    pub command: String,
    job: proc::Job,
    output: Arc<Mutex<OutputBuffer>>,
    /// `None` while it runs; then its exit code (`None` inside if it was killed).
    exit: watch::Receiver<Option<Option<i32>>>,
    started: Instant,
}

impl Background {
    pub fn kill(&self) {
        self.job.kill();
    }

    pub fn running(&self) -> bool {
        self.exit.borrow().is_none()
    }

    /// Waits up to `wait` for it to end, then describes it with the last `tail` lines.
    pub async fn report(&self, wait: Duration, tail: usize) -> String {
        if !wait.is_zero() {
            let mut exit = self.exit.clone();
            let _ = tokio::time::timeout(wait, exit.wait_for(Option::is_some)).await;
        }
        let output = lock(&self.output).text();
        let lines: Vec<&str> = output.lines().collect();
        let shown = lines[lines.len().saturating_sub(tail)..].join("\n");
        let state = match *self.exit.borrow() {
            None => format!(
                "is still running ({}s so far)",
                self.started.elapsed().as_secs()
            ),
            Some(Some(code)) => format!("exited with code {code}"),
            Some(None) => "was stopped".to_owned(),
        };
        let body = if shown.is_empty() {
            "(no output yet)".to_owned()
        } else {
            shown
        };
        let more = if lines.len() > tail {
            format!(" (last {tail} of {} lines)", lines.len())
        } else {
            String::new()
        };
        format!(
            "Process {} (`{}`) {state}. Output{more}:\n{body}",
            self.id, self.command
        )
    }
}

/// A session's background commands.
#[derive(Default)]
pub struct Processes {
    next: AtomicU32,
    list: Mutex<HashMap<u32, Arc<Background>>>,
}

impl Processes {
    pub fn start(&self, shell: &Shell, command: &str, cwd: &Path) -> io::Result<Arc<Background>> {
        let (process, script, mut rx) = start(shell, command, cwd)?;
        let proc::Process { mut child, job } = process;
        let output = Arc::new(Mutex::new(OutputBuffer::default()));
        let (tx, exit) = watch::channel(None);
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let sink = output.clone();
        tokio::spawn(async move {
            let _script = script;
            while let Some(chunk) = rx.recv().await {
                lock(&sink).push(&chunk);
            }
        });
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(|s| s.code());
            let _ = tx.send(Some(code));
        });
        let background = Arc::new(Background {
            id,
            command: command.to_owned(),
            job,
            output,
            exit,
            started: Instant::now(),
        });
        lock(&self.list).insert(id, background.clone());
        Ok(background)
    }

    pub fn get(&self, id: u32) -> Option<Arc<Background>> {
        lock(&self.list).get(&id).cloned()
    }

    pub fn kill_all(&self) {
        for process in lock(&self.list).values() {
            process.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::shell::ShellKind;

    fn bash() -> Option<Shell> {
        let shell = Shell::find(ShellKind::GitBash);
        if shell.is_none() {
            eprintln!("Git Bash isn't installed here; skipped");
        }
        shell
    }

    #[test]
    fn long_output_keeps_its_start_and_end() {
        let mut buffer = OutputBuffer::default();
        buffer.push(b"first line\n");
        buffer.push(&vec![b'x'; HEAD_BYTES + TAIL_BYTES + 10]);
        buffer.push(b"\nlast line");
        let text = buffer.text();
        assert!(text.starts_with("first line\n"));
        assert!(text.contains("KB of output left out"));
        assert!(text.ends_with("last line"));
    }

    #[tokio::test]
    async fn a_command_streams_output_and_reports_its_exit_code() {
        let Some(shell) = bash() else { return };
        let dir = tempfile::tempdir().unwrap();
        let mut live = String::new();
        let ran = run(
            &shell,
            "echo one; sleep 0.3; echo two; exit 3",
            dir.path(),
            Duration::from_secs(20),
            &CancellationToken::new(),
            |text| {
                live.push_str(&text);
                live.push('\n');
            },
        )
        .await
        .unwrap();
        assert_eq!(ran.exit, Some(3));
        assert_eq!(ran.output, "one\ntwo");
        assert!(live.contains("one") && live.contains("two"));
    }

    #[tokio::test]
    async fn timeouts_and_stop_end_the_command() {
        let Some(shell) = bash() else { return };
        let dir = tempfile::tempdir().unwrap();
        let ran = run(
            &shell,
            "echo started; sleep 30",
            dir.path(),
            Duration::from_secs(2),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        assert!(ran.timed_out && ran.exit.is_none());
        assert!(ran.elapsed < Duration::from_secs(10), "{:?}", ran.elapsed);
        assert_eq!(ran.output, "started");

        let cancel = CancellationToken::new();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            stopper.cancel();
        });
        let ran = run(
            &shell,
            "sleep 30",
            dir.path(),
            Duration::from_secs(60),
            &cancel,
            |_| {},
        )
        .await
        .unwrap();
        assert!(ran.cancelled && ran.elapsed < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn processes_left_running_in_the_background_are_stopped() {
        let Some(shell) = bash() else { return };
        let dir = tempfile::tempdir().unwrap();
        let ran = run(
            &shell,
            "sleep 30 & echo done",
            dir.path(),
            Duration::from_secs(60),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(ran.output, "done");
        assert!(
            ran.stopped_leftovers && ran.elapsed < Duration::from_secs(15),
            "{:?}",
            ran.elapsed
        );
    }

    #[tokio::test]
    async fn background_commands_can_be_checked_and_stopped() {
        let Some(shell) = bash() else { return };
        let dir = tempfile::tempdir().unwrap();
        let processes = Processes::default();
        let server = processes
            .start(&shell, "echo listening; sleep 30", dir.path())
            .unwrap();
        let report = server.report(Duration::from_secs(2), 10).await;
        assert!(
            report.contains("is still running") && report.contains("listening"),
            "{report}"
        );
        processes.get(server.id).unwrap().kill();
        let report = server.report(Duration::from_secs(10), 10).await;
        assert!(
            report.contains("exited with code") || report.contains("was stopped"),
            "{report}"
        );
        assert!(!server.running());

        let quick = processes
            .start(&shell, "echo hi; exit 4", dir.path())
            .unwrap();
        assert!(
            quick
                .report(Duration::from_secs(10), 10)
                .await
                .contains("exited with code 4")
        );
        assert_eq!(quick.id, server.id + 1);
    }
}
