//! A Model Context Protocol client for tools: JSON-RPC 2.0 over a server's stdin/stdout, or
//! over Streamable HTTP. Hearth uses `initialize`, `tools/list` and `tools/call`; requests a
//! server sends (other than `ping`) are politely refused.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::proc;

/// The protocol version Hearth asks for (servers may answer with another they support).
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Lines of a server's stderr kept for its error messages and the Settings page.
const LOG_LINES: usize = 50;
/// How long an HTTP notification may take.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// A connection to one server.
pub struct Client {
    kind: Kind,
    next_id: AtomicI64,
    log: Arc<Mutex<VecDeque<String>>>,
    closed: Arc<AtomicBool>,
    /// Notified when a stdio server's output ends.
    ended: Arc<Notify>,
}

/// One line of a server's output, decoded leniently (a stray byte that isn't UTF-8 doesn't end
/// the connection). `None` at the end.
async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Option<String> {
    let mut bytes = Vec::new();
    match reader.read_until(b'\n', &mut bytes).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(
            String::from_utf8_lossy(&bytes)
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
        ),
    }
}

enum Kind {
    Stdio {
        input: mpsc::UnboundedSender<String>,
        pending: Pending,
        /// The process and its job: dropping it ends the server and anything it started.
        _process: Mutex<Option<proc::Process>>,
    },
    Http {
        client: reqwest::Client,
        url: String,
        headers: HeaderMap,
        session: Mutex<Option<String>>,
        protocol: Mutex<Option<String>>,
    },
}

/// What Windows can start directly, in the order `cmd.exe` tries them (PATHEXT).
const EXECUTABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

/// A program by name, as the shell would find it: on PATH, with `.exe`, `.cmd` or `.bat`.
/// MCP configs say `npx`, which is `npx.cmd` on Windows; the extensionless `npx` next to it is
/// a shell script Windows can't run, so a name without an executable extension only ever
/// matches with one.
pub fn find_program(program: &str, path: Option<&str>) -> Option<PathBuf> {
    let given = Path::new(program);
    let named = given
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXECUTABLE.contains(&format!(".{}", ext.to_ascii_lowercase()).as_str()));
    let with_extension = |base: &Path| {
        if named {
            return base.is_file().then(|| base.to_path_buf());
        }
        EXECUTABLE.iter().find_map(|ext| {
            let candidate = PathBuf::from(format!("{}{ext}", base.display()));
            candidate.is_file().then_some(candidate)
        })
    };
    if given.is_absolute() || program.contains(['/', '\\']) {
        return with_extension(given);
    }
    let path = path
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    std::env::split_paths(&path).find_map(|dir| with_extension(&dir.join(program)))
}

fn rpc_error(error: &Value) -> String {
    let message = error["message"].as_str().unwrap_or("an unknown error");
    match error["code"].as_i64() {
        Some(-32601) => format!("The server doesn't know this request ({message})."),
        Some(code) => format!("{message} (error {code})"),
        None => message.to_owned(),
    }
}

impl Client {
    /// Starts a server program and talks to it over its stdin and stdout.
    pub fn stdio(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: Option<&Path>,
    ) -> Result<Client, String> {
        let program = find_program(command, env.get("PATH").map(String::as_str))
            .ok_or_else(|| format!("`{command}` wasn't found. Is it installed, and on PATH?"))?;
        let mut builder = proc::command(&program);
        builder
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            builder.current_dir(cwd);
        }
        let mut process = proc::spawn(&mut builder)
            .map_err(|err| format!("`{command}` couldn't start: {err}"))?;
        let stdin = process.child.stdin.take().ok_or("no stdin")?;
        let stdout = process.child.stdout.take().ok_or("no stdout")?;
        let stderr = process.child.stderr.take().ok_or("no stderr")?;

        let log = Arc::new(Mutex::new(VecDeque::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(Notify::new());
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (input, mut lines_out) = mpsc::unbounded_channel::<String>();

        // Writes requests and answers, one JSON object per line.
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = lines_out.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                    || stdin.flush().await.is_err()
                {
                    break;
                }
            }
        });
        // Keeps the last lines the server wrote to stderr (in whatever encoding).
        let errors = log.clone();
        tokio::spawn(async move {
            let mut stderr = BufReader::new(stderr);
            while let Some(line) = read_line(&mut stderr).await {
                let mut log = lock(&errors);
                log.push_back(line);
                while log.len() > LOG_LINES {
                    log.pop_front();
                }
            }
        });
        // Reads answers (and the server's own requests and notifications).
        let (answers, reply) = (pending.clone(), input.clone());
        let (done, signal) = (closed.clone(), ended.clone());
        tokio::spawn(async move {
            let mut stdout = BufReader::new(stdout);
            while let Some(line) = read_line(&mut stdout).await {
                // Some servers print other things to stdout; only JSON-RPC counts.
                let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                if let Some(method) = message["method"].as_str() {
                    if let Some(id) = message.get("id") {
                        let answer = if method == "ping" {
                            json!({ "jsonrpc": "2.0", "id": id, "result": {} })
                        } else {
                            json!({ "jsonrpc": "2.0", "id": id, "error": {
                                "code": -32601, "message": "Hearth doesn't support this request."
                            } })
                        };
                        let _ = reply.send(answer.to_string());
                    }
                    continue;
                }
                if let Some(id) = message["id"].as_i64()
                    && let Some(waiting) = lock(&answers).remove(&id)
                {
                    let result = match message.get("error") {
                        Some(error) => Err(rpc_error(error)),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = waiting.send(result);
                }
            }
            done.store(true, Ordering::SeqCst);
            for (_, waiting) in lock(&answers).drain() {
                let _ = waiting.send(Err("The server stopped.".into()));
            }
            signal.notify_waiters();
        });

        Ok(Client {
            kind: Kind::Stdio {
                input,
                pending,
                _process: Mutex::new(Some(process)),
            },
            next_id: AtomicI64::new(1),
            log,
            closed,
            ended,
        })
    }

    /// Waits until the server's process ends (never, for HTTP servers).
    pub async fn wait_closed(&self) {
        loop {
            // Registered before the check, so an end in between isn't missed.
            let ended = self.ended.notified();
            if self.is_closed() {
                return;
            }
            ended.await;
        }
    }

    /// Talks to a server over Streamable HTTP.
    pub fn http(url: &str, headers: &BTreeMap<String, String>) -> Result<Client, String> {
        let parsed =
            reqwest::Url::parse(url).map_err(|_| format!("“{url}” isn't a web address."))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("MCP servers are reached over http or https.".into());
        }
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.trim().as_bytes())
                .map_err(|_| format!("“{name}” isn't a valid header name."))?;
            let value = HeaderValue::from_str(value.trim())
                .map_err(|_| format!("The value of {name} isn't a valid header value."))?;
            map.insert(name, value);
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Client {
            kind: Kind::Http {
                client,
                url: url.to_owned(),
                headers: map,
                session: Mutex::new(None),
                protocol: Mutex::new(None),
            },
            next_id: AtomicI64::new(1),
            log: Arc::new(Mutex::new(VecDeque::new())),
            closed: Arc::new(AtomicBool::new(false)),
            ended: Arc::new(Notify::new()),
        })
    }

    /// The server's last stderr lines.
    pub fn log(&self) -> Vec<String> {
        lock(&self.log).iter().cloned().collect()
    }

    /// The process ended (stdio servers).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// The handshake. Returns the server's name and version.
    pub async fn initialize(&self) -> Result<Value, String> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "Hearth", "version": env!("CARGO_PKG_VERSION") },
                }),
                // `npx -y <server>` may download the server first.
                Duration::from_secs(90),
                None,
            )
            .await?;
        if let Kind::Http { protocol, .. } = &self.kind {
            *lock(protocol) = result["protocolVersion"].as_str().map(str::to_owned);
        }
        self.notify("notifications/initialized", json!({})).await;
        Ok(result["serverInfo"].clone())
    }

    /// Every tool the server offers (following `nextCursor`).
    pub async fn list_tools(&self) -> Result<Vec<Value>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let page = self
                .request("tools/list", params, Duration::from_secs(30), None)
                .await?;
            tools.extend(page["tools"].as_array().cloned().unwrap_or_default());
            cursor = page["nextCursor"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, String> {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
            Duration::from_secs(600),
            Some(cancel),
        )
        .await
    }

    async fn notify(&self, method: &str, params: Value) {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &self.kind {
            Kind::Stdio { input, .. } => {
                let _ = input.send(message.to_string());
            }
            Kind::Http { .. } => {
                // Nothing waits on a notification's answer, so a server that never sends one
                // mustn't hold anything up.
                let _ = tokio::time::timeout(NOTIFY_TIMEOUT, self.post(&message, None)).await;
            }
        }
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, String> {
        if self.is_closed() {
            return Err("The server stopped.".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let stopped = async {
            match cancel {
                Some(cancel) => cancel.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let answer = async {
            match &self.kind {
                Kind::Stdio { input, pending, .. } => {
                    let (tx, rx) = oneshot::channel();
                    lock(pending).insert(id, tx);
                    // The server may have ended (and its last answers been sent) in between.
                    if self.is_closed() || input.send(message.to_string()).is_err() {
                        lock(pending).remove(&id);
                        return Err("The server stopped.".into());
                    }
                    rx.await
                        .unwrap_or_else(|_| Err("The server stopped.".into()))
                }
                Kind::Http { .. } => self.post(&message, Some(id)).await,
            }
        };
        let outcome = tokio::select! {
            result = answer => result,
            () = tokio::time::sleep(timeout) => Err(format!(
                "The server didn't answer {method} within {} seconds.",
                timeout.as_secs()
            )),
            () = stopped => Err("Stopped.".into()),
        };
        if outcome.is_err() {
            if let Kind::Stdio { pending, .. } = &self.kind {
                lock(pending).remove(&id);
            }
            if method == "tools/call" && !self.is_closed() {
                self.notify(
                    "notifications/cancelled",
                    json!({ "requestId": id, "reason": "Stopped or timed out." }),
                )
                .await;
            }
        }
        outcome
    }

    /// Posts a message over HTTP; for requests, the matching answer (a JSON body or an event
    /// in an SSE stream).
    async fn post(&self, message: &Value, id: Option<i64>) -> Result<Value, String> {
        let Kind::Http {
            client,
            url,
            headers,
            session,
            protocol,
        } = &self.kind
        else {
            return Err("not an HTTP server".into());
        };
        let mut request = client
            .post(url)
            .headers(headers.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .json(message);
        if let Some(session) = lock(session).clone() {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(protocol) = lock(protocol).clone() {
            request = request.header("MCP-Protocol-Version", protocol);
        }
        let response = request
            .send()
            .await
            .map_err(|err| format!("Couldn't reach the server: {err}"))?;
        if let Some(new) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *lock(session) = Some(new.to_owned());
        }
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(
                "The server wants authorization: add the header it needs (e.g. Authorization) in its settings."
                    .into(),
            );
        }
        if !status.is_success() {
            return Err(format!("The server answered with an error ({status})."));
        }
        let Some(id) = id else {
            return Ok(Value::Null);
        };
        let streamed = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let pick = |message: &Value| -> Option<Result<Value, String>> {
            let messages = match message {
                Value::Array(batch) => batch.clone(),
                single => vec![single.clone()],
            };
            messages
                .into_iter()
                .find(|m| m["id"].as_i64() == Some(id) && m.get("method").is_none())
                .map(|m| match m.get("error") {
                    Some(error) => Err(rpc_error(error)),
                    None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                })
        };
        if !streamed {
            let body: Value = response
                .json()
                .await
                .map_err(|err| format!("The server's answer isn't JSON: {err}"))?;
            return pick(&body).unwrap_or_else(|| Err("The server's answer was missing.".into()));
        }
        let mut stream = response.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| format!("The server's stream broke off: {err}"))?;
            buffer.extend_from_slice(&chunk);
            for event in take_events(&mut buffer) {
                if let Ok(message) = serde_json::from_str::<Value>(&event)
                    && let Some(answer) = pick(&message)
                {
                    return answer;
                }
            }
        }
        Err("The server's stream ended without an answer.".into())
    }
}

/// Complete SSE events' data, taken from the front of `buffer`. Bytes stay bytes until an event
/// is whole, so a character split between two network chunks isn't broken.
fn take_events(buffer: &mut Vec<u8>) -> Vec<String> {
    let blank = |bytes: &[u8]| {
        (0..bytes.len().saturating_sub(1)).rev().find_map(|i| {
            if bytes[i..].starts_with(b"\n\n") {
                Some((i, 2))
            } else if bytes[i..].starts_with(b"\r\n\r\n") {
                Some((i, 4))
            } else {
                None
            }
        })
    };
    let Some((end, gap)) = blank(buffer) else {
        return Vec::new();
    };
    let complete = String::from_utf8_lossy(&buffer[..end]).replace("\r\n", "\n");
    buffer.drain(..end + gap);
    complete
        .split("\n\n")
        .filter_map(|event| {
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect();
            (!data.is_empty()).then(|| data.join("\n"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_events_are_taken_whole() {
        let mut buffer = b"event: message\r\ndata: {\"a\":1}\r\n\r\ndata: {\"b\":".to_vec();
        assert_eq!(take_events(&mut buffer), vec![r#"{"a":1}"#.to_owned()]);
        assert_eq!(buffer, b"data: {\"b\":");
        buffer.extend_from_slice(b"2}\n\n: a comment\n\n");
        assert_eq!(take_events(&mut buffer), vec![r#"{"b":2}"#.to_owned()]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_character_split_between_chunks_stays_whole() {
        let event = "data: {\"t\":\"café ✓\"}\n\n".as_bytes();
        let cut = event.iter().position(|&b| b == 0xC3).unwrap() + 1;
        let mut buffer = event[..cut].to_vec();
        assert!(take_events(&mut buffer).is_empty());
        buffer.extend_from_slice(&event[cut..]);
        assert_eq!(
            take_events(&mut buffer),
            vec!["{\"t\":\"café ✓\"}".to_owned()]
        );
    }

    #[test]
    fn programs_are_found_with_windows_extensions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("npx.cmd"), "@echo off").unwrap();
        // Node also puts a shell script called just `npx` there, which Windows can't run.
        std::fs::write(dir.path().join("npx"), "#!/bin/sh").unwrap();
        let path = dir.path().display().to_string();
        assert_eq!(
            find_program("npx", Some(&path)),
            Some(dir.path().join("npx.cmd"))
        );
        assert_eq!(find_program("missing-tool", Some(&path)), None);
        assert_eq!(
            find_program("npx.cmd", Some(&path)),
            Some(dir.path().join("npx.cmd"))
        );
        let full = dir.path().join("npx").display().to_string();
        assert_eq!(find_program(&full, None), Some(dir.path().join("npx.cmd")));
    }

    #[test]
    fn errors_read_plainly() {
        assert_eq!(
            rpc_error(&json!({ "code": -32602, "message": "bad params" })),
            "bad params (error -32602)"
        );
        assert!(rpc_error(&json!({ "code": -32601, "message": "x" })).contains("doesn't know"));
    }
}
