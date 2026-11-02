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
