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

Projects (folders) are stored in `hearth.db`. Opening a folder for the first time asks whether
its contents are trusted; untrusted projects don't get their AGENTS.md read.

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
