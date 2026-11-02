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

## The web (`hearth-core/src/web/`)

Chats with web search on and the agent's `web_search` and `web_fetch` tools share one service.

- **Searching** (`search.rs`): DuckDuckGo's HTML page by default (no key; ads and redirect links
  are left out, and its "are you a person" page is explained), or Brave Search, Tavily or a
  SearXNG server. Every provider gives the same hits: title, address, snippet. API keys are kept
  in the database's `kv` table; the UI only learns whether one is set.
- **Reading** (`fetch.rs`): the page's main text by Readability (dom_smoothie), as Markdown, cut
  to 16,000 characters by default. Only http(s) pages; PDFs and files are refused with a reason.
  Unless Settings → Web allows it, addresses on this PC or the local network are refused: each
  hop of a redirect is looked up and checked, and the connection goes to the checked address.
  For that check to hold, page reading then connects directly instead of through a system proxy
  (a proxy would look the name up again itself). Only global IPv6 counts as the internet, and
  6to4 addresses are judged by the IPv4 address inside them. A page gets a minute at most.
- **In chats**, the composer's Web button turns web search on for the chat (its `search_mode`).
  A reply then gets the two web tools and may search and read for up to five steps before it has
  to answer; the steps and their answers are saved in the message tree like the agent's. The
  transcript shows each search and page as a line (searches open to their results) and the
  pages used as source chips under the answer.
- **In coding sessions** the two tools are in the Network class: they ask first in Ask and
  Auto-edit mode, and run freely in Full auto.
- Results and pages are information, not instructions, and both prompts say so.

## MCP servers (`hearth-core/src/mcp/`)

MCP servers give the coding agent more tools. Hearth has its own small client
(`client.rs`) for the part it uses: `initialize`, `tools/list` (with pages) and `tools/call`,
over a server's stdin/stdout or Streamable HTTP (JSON or SSE answers, the session header).

- **Settings** live in the database's `kv` table: a name, on or off, and either a command (with
  arguments, environment and folder) or a URL (with headers). Claude Desktop, Cursor and VS Code
  configs (`"mcpServers"`) can be pasted in to import them.
- **Running:** servers start when Hearth starts and again when their settings change. A
  command is looked up the way a shell would (`npx` is `npx.cmd`) and runs in its own Job
  Object. Its stderr is kept (the last 50 lines), so a server that can't start says why. A
  server that dies is marked as failed. The Settings page follows every change.
- **Tools:** a ready server's tools join each new coding task as `mcp__<server>__<tool>`
  (at most 64 characters), after the built-in tools. They're in the External class: Plan mode
  refuses them, Full auto runs them, and otherwise the agent asks, offering "Always allow this
  tool here" (saved per project, like command rules). Results are read as text (images and
  other content are described), cut at 40,000 characters.

## Attachments

Chat messages and coding tasks can carry up to 10 files: text and code files, PDFs and images.

- **Reading** happens in the UI (`lib/attachments.ts`): text files as text, a PDF's text with
  pdf.js (loaded only when a PDF is attached; its worker comes from the app itself), and images
  scaled down to 1,568 pixels on the long side (PNGs stay PNG, the rest become JPEG). Files can
  be picked, dropped on the composer (Tauri's own drop handler is off for this) or pasted.
- **Saving:** Rust keeps them in the `attachments` table (migration 5), checked (images must be
  `data:image/` URIs, text is cut at 400,000 characters). Messages carry only names, kinds and
  sizes; the transcript loads an image when it shows it (`attachment_data`). An edited message
  keeps its files.
- **Sending:** attached text goes into the user's message as `<attachment name="…">` blocks.
  Images go as `image_url` parts when the model can see them: a local model with its vision
  projector (mmproj) loaded, or what a llama.cpp server's `/props` reports. Otherwise the
  model is told an image was attached, and the chat shows why it wasn't sent. Context
  estimates count attached text, and about 800 tokens per image.

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

## The UI (`apps/hearth/src`)

- **Screens:** Chat, Code (with the Changes and terminal panels), Models, Settings, and the
  Guide (docs/models.md bundled into the app, so it reads offline).
- **State** lives in Zustand stores: `app` (screen, settings, the command palette), `chat`
  (sessions, the open conversation, its live run, projects), `models`, `terminal`. A run's
  events go through one pure reducer (`lib/runReducer.ts`), tested on its own.
- **First run:** while no model is chosen for chats (or coding), an empty conversation shows a
  "Get started" card: llama.cpp found, model files found, a model chosen.
- **Ctrl+K** opens a command palette with every action and recent conversations. App shortcuts
  don't fire while a terminal has the focus (except Ctrl+`), so the shell gets Ctrl+B and the rest.
- **Background runs:** runs live in Rust, so switching conversations or reloading the page never
  stops one; when a reply finishes or the agent asks something while Hearth is in the background,
  its taskbar button flashes.
- **Accessibility:** text colors read at 4.5:1 or more on every resting background in both themes
  (the token block in `styles/app.css` says which token is for what); an open conversation's title
  is the page's heading; the terminal tabs follow the ARIA tabs pattern (arrow keys, Delete
  closes a shell). `e2e/a11y.spec.ts` runs axe-core on every screen and state, light and dark.

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
  is fresh for each run. The hidden window doesn't run CSS transitions, so tests that read colors
  switch transitions off first (see `e2e/a11y.spec.ts`).

See [build-notes.md](build-notes.md) for the detailed contracts and the Windows traps.
