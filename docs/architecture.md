# Architecture

Hearth is a Tauri 2 app: a Rust process owns everything that touches the machine (files,
processes, models, network, storage), and a React UI in WebView2 shows it.

```
┌──────────────────────────── hearth.exe ────────────────────────────┐
│  WebView2 (React UI)            Rust                               │
│  ─────────────────              ────                               │
│  screens, stores    ── invoke ──▶ src-tauri: commands, window,     │
│  (Zustand)          ◀─ Channel ──    title bar, events              │
│                     ◀─ events ───        │                          │
│                                          ▼                          │
│                              crates/hearth-core                     │
│                     settings · store (SQLite) · llm client ·        │
│                     runtime (llama-server) · chat · agent · tools · │
│                     search · mcp · terminal · proc (Job Objects)    │
└──────────────────────────────────┬─────────────────────────────────┘
                                   │ 127.0.0.1 + API key
                         llama-server.exe (one per loaded model)
```

## Principles

- **hearth-core has no Tauri dependency.** It is unit- and integration-tested with plain `cargo
test`, against a mock model server from `crates/hearth-testkit`.
- **Rust owns runs.** A chat reply or an agent task keeps running if the UI reloads. The UI
  re-attaches to it and gets a snapshot first.
- **The UI never touches the network** ([ADR 0003](adr/0003-ui-never-touches-the-network.md)).
- **Every child process lives in a Job Object.** That covers llama-server, agent commands, MCP
  servers and terminals. Stop, timeouts and quitting Hearth end whole process trees.
- **Typed contracts.** Commands return `{ kind, message }` errors. Settings are validated in Rust
  before they're saved.

## Data

Hearth keeps its data in `%LOCALAPPDATA%\app.hearth\`. Debug test runs use their own folder
(`HEARTH_TEST_DATA_DIR`).

- `settings.json`: theme, layout, and later runtime, model and agent settings. Written atomically.
  JSON merge patches from the UI are validated before they're saved.
- `hearth.db` (from M1): conversations as a message tree, attachments, checkpoints, presets,
  connections, MCP servers.
- `EBWebView\`: the WebView2 profile.

## The coding agent (`hearth-core/src/agent/`)

A task is a run like a chat reply, but it loops: the model streams a step, its tool calls run one
at a time, their answers are saved as tool messages, and the model continues until it replies
without a tool call (or a guard stops it).

- **Tools** (`tools.rs`) work in two phases. `prepare` checks the arguments, resolves the path and
  works out the diff without touching anything; `execute` makes the change. Names and arguments
  from other agents (`Read`, `str_replace`, `file_path`…) are accepted.
- **Permissions** (`permissions.rs`) decide between allow, ask and deny from the session's mode
  (Plan, Ask, Auto-edit, Full auto) and what the user allowed for the session. A prompt waits in
  Rust for the user's answer; the UI gets it again after a reload.
- **The prompt** (`prompt.rs`) is fixed rules plus an environment snapshot and the project's
  AGENTS.md, frozen when the session's first task starts, so llama-server can reuse its cache.
- **The loop** (`runner.rs`) answers every tool call (with a note when the user stopped the run),
  gives every call its own id, parses tool calls a model wrote as text, warns after three
  identical calls and pauses after five, and pauses after 50 steps.
- **Edits are safe to retry:** a file must have been read (and not changed since) before it can be
  changed; line endings, BOM and encoding are kept; edits that don't match exactly fall back to
  whitespace-tolerant matching and otherwise show the closest text.
- **Commands** (`shell.rs`, `exec.rs`, `command.rs`) run in Git Bash or PowerShell, each in its
  own Job Object (`proc.rs`), so Stop and timeouts end the whole process tree. Output streams to
  the UI; the model gets the first and last 150 lines. Read-only commands and project "Always
  allow" rules skip the prompt in Auto-edit; dangerous commands always ask.
- **Undo** (`changes.rs`): before the agent first changes a file in a task, the file is saved as
  a checkpoint. The Changes panel compares files with their first checkpoint; undoing a task
  restores every file changed from that task on. Commands' own changes aren't tracked.
- **Steering:** a message sent while the agent works waits in Rust and joins the conversation
  before the next step.

Projects (folders) are stored in `hearth.db`. Opening a folder for the first time asks whether
its contents are trusted; untrusted projects don't get their AGENTS.md read.

## The model runtime (`hearth-core/src/runtime/`)

Hearth runs local models itself with llama.cpp's `llama-server`, one process per loaded model
([ADR 0002](adr/0002-one-llama-server-per-model.md)).

- **Finding things:** `llama.rs` finds llama-server (the chosen path, PATH, or the usual folders)
  and probes `--version`, `--list-devices` and `--help`. `scan.rs` reads GGUF headers
  (`gguf.rs`) in the model folders and LM Studio's, groups split files, and pairs vision
  projectors.
- **Running:** `manager.rs` starts a server on a free 127.0.0.1 port with a random API key, in
  its own Job Object, passing only flags the build knows. A watcher polls `/health` until the
  model is ready and notices if the server stops. When the limit (one by default) is reached,
  the least recently used model is unloaded first. Failures are explained from the log (out of
  memory, a missing DLL, an unknown argument, an old llama.cpp).
- **Using a model:** a `local` model reference resolves to `Endpoint::Local`; the first request
  loads the server (the run shows "Loading <model>…") and later ones reuse it.

## Terminals (`hearth-core/src/terminal/`)

The terminal panel under a coding session runs real shells (Git Bash by default, or
PowerShell) in Windows pseudo consoles (ConPTY).

- **Starting:** `pty.rs` creates the pseudo console and starts the shell suspended, puts it in
  its own Job Object and only then lets it run, like every other child process. Closing a tab
  (or Hearth) ends the shell and everything started in it. Git Bash starts as a login shell in
  the project folder (`CHERE_INVOKING`), with `TERM=xterm-256color`.
- **Living in Rust:** `Terminals` keeps each shell with the last 256 KB of its output. The UI
  attaches over a channel and first gets that scrollback, so a terminal keeps running (and
  showing what it showed) when the panel is hidden or the page reloads. Keystrokes go through a
  writer thread, so a busy shell never blocks a command.
- **The UI** draws terminals with xterm.js in tabs, one panel per project, next to the Agent tab,
  which lists the commands the agent ran in the session with their output as it arrives.
  Ctrl+`shows and hides the panel. xterm.js assigns`toString`on an object, which Tauri's
frozen`Object.prototype`forbids, so`vite.config.ts` rewrites that one line as a property
  definition.

## Context management (`hearth-core/src/context.rs`)

Chats and coding tasks keep their requests inside the model's context window.

- **The window** comes from the local model's settings, or from `/props` on a llama.cpp
  connection. Other servers have no known window and are never compacted.
- **How full it is:** the newest reply's own token count (prompt plus reply) plus estimates
  (4 characters a token) for what came after it. Every finished reply saves the window with its
  usage, and the run sends a `context` event, so the composer's meter shows the share in use.
- **Compacting:** before a request that would use more than 80% of the window, the older part is
  summarized by the same model (not streamed, thinking off, at most 1,200 tokens). Recent
  messages keep up to 35% of the window word for word, and a cut never separates a tool call from
  its result. The summary is saved as a `summary` message after the newest message, with
  `covers_until` naming the last message it stands in for. Requests then carry the summary in the
  system prompt and only the messages after `covers_until`. Nothing is deleted: the transcript
  shows every message, with a divider where the summary was made (it opens to show it).
- **When the server says no:** if llama-server still answers "exceeds the available context
  size", its `n_ctx` replaces the known window, everything before the newest user message is
  summarized, and the request is tried once more.

## Window

The window is frameless and has a custom title bar. Windows 11 Snap Layouts work through a native
overlay window answering `HTMAXBUTTON` (from Folio). The window is created hidden, with the theme
injected before the page loads, and shown once the UI has painted, so it never flashes. When
minimized, WebView2 is asked to release memory.

## Testing

- **Rust:** `cargo test` for core logic; integration tests use the testkit's mock LLM and fake
  llama-server.
- **UI:** Vitest with jsdom and an in-memory fake backend (`lib/backend.ts`).
- **End to end:** Playwright attaches over WebView2's debugging port to the real debug build with
  the UI embedded (frozen prototypes only apply there). The window is hidden and the data folder
  is fresh for each run.

See [build-notes.md](build-notes.md) for the detailed contracts and the Windows traps.
