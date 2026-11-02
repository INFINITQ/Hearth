# Build status

Where the build stands (plan: README "Status"; engineering details: build-notes.md).

## Done

- **M0**: scaffold, Folio window and title bar, design tokens, models guide, E2E harness.
- **M1**: SQLite store (message trees, FTS5), streaming LLM client, testkit mock server.
- **M2**: chat end to end.
  - **Backend:** RunManager (runs live in Rust, attach with snapshot), chat runs (send, regenerate,
    edit, stop), ~33 ms batching, 1 s autosave, auto titles, connections and default model roles.
  - **UI:** streaming transcript (Streamdown with Shiki and KaTeX, `$…$` and `\(…\)` math),
    thinking blocks, versions, edit and regenerate, stop, model picker, per-chat settings, sidebar
    history with date groups, rename/pin/delete/export, full-text search, connection settings.
- **M3**: the coding agent with file tools and permissions.
  - **Backend** (`hearth-core/src/agent/`):
    - `paths.rs`: Windows, Git Bash and WSL paths, containment, reserved names.
    - `text.rs`: encodings, BOM and CRLF kept on write.
    - `matcher.rs`: exact, then whitespace and line-number tolerant edits, with the closest match
      on a miss.
    - `tools.rs`: read_file, list_dir, glob, grep, write_file, edit_file, todo_write, ask_user;
      two-phase prepare/execute; read-before-write stamps.
    - `permissions.rs`: Plan / Ask / Auto-edit / Full auto, plus session grants.
    - `prompt.rs`: the rules, a frozen environment snapshot, and AGENTS.md (or CLAUDE.md).
    - `runner.rs`: the loop, with permission and question waits, every call answered, unique call
      ids, tool calls parsed from text, repeat and step guards.
    - `llm/toolparse.rs`: fallback tool-call parsing and JSON repair.
    - Store migration 2: projects (`workspaces`), and the session's frozen environment.
    - Run events: tool started/finished, permission request/resolved, question/answered, to-dos,
      notices, and a `waiting` run state for sidebar dots.
  - **App:** project commands (pick with a trust prompt, recent, open), coding sessions,
    `run_permission`, `run_answer`; `run_start` routes coding sessions to the agent.
  - **UI:** project switcher, new-session view, transcript with tool rows (diffs open after an
    edit), permission card (diff, Allow, Allow for the session, Deny, deny with a note), question
    card, to-do panel, mode switch (Shift+Tab), "needs your answer" dots.
  - **Tests (all green):**
    - Rust: 75 core unit, 9 agent integration, 8 chat, 11 client, 4 app.
    - Vitest: 19.
    - Playwright E2E: 20, of which 5 are coding sessions (reload while a prompt waits, deny with
      a note, questions, screenshots).

- **M4**: commands, checkpoints, the Changes panel, stop and steering.
  - **Backend:**
    - `proc.rs`: every child process in its own Job Object, started suspended and resumed after
      joining, with no console window; killing the job ends the whole tree (tested with a
      grandchild).
    - `agent/shell.rs`: Git Bash (registry, PATH, usual folders) or PowerShell; scripts via a temp
      `.sh` file or `-EncodedCommand`; output decoded (UTF-8 or the OEM code page), ANSI codes
      and progress redraws removed, first and last 150 lines for the model.
    - `agent/command.rs`: read-only commands (Plan runs them; Auto-edit doesn't ask), dangerous
      ones (always ask, even in Full auto), rule prefixes (`npm test`).
    - `agent/exec.rs`: foreground runs with live output, timeout and Stop; leftover background
      processes stopped; background commands with `command_output` and `kill_process`.
    - `agent/changes.rs`: checkpoints before each file's first change in a task; the session's
      changes with diffs; revert a file; undo from a task on.
    - Runner: run_command / command_output / kill_process, "Always allow `prefix` here" rules
      (migration 3: `checkpoints`, `permission_rules`), steering (messages sent mid-task join
      before the next step), a note to the model after an undo.
    - Settings: `code.shell` (Git Bash or PowerShell).
  - **UI:** command rows with live output, command permission cards (with a warning for
    dangerous ones), the Changes panel (diffs, revert), "Undo file changes from here" on each
    task, sending while the agent works (queued until the next step), the shell in Settings.
  - **Tests (all green):**
    - Rust: 93 core unit, 15 agent integration, 8 chat, 11 client, 4 app.
    - Vitest: 20.
    - Playwright E2E: 26.

- **M5**: the managed llama-server runtime and the Models page.
  - **Backend** (`hearth-core/src/runtime/`):
    - `gguf.rs`: GGUF header reader (architecture, name, size label, quantization, context,
      layers, experts, chat template → tools/thinking, mmproj, splits), vocabulary skipped.
    - `scan.rs`: model folders (plus LM Studio's), split files grouped, mmproj paired (F16
      first), quantization from the file name, stable ids.
    - `llama.rs`: find llama-server (setting, PATH, usual folders); probe `--version`,
      `--list-devices`, `--help`; a missing CUDA DLL and CPU-only builds explained.
    - `manager.rs`: one llama-server per loaded model (free port, random API key, Job Object,
      only known flags, presets, `--cache-ram 2048` for big models); a watcher polls `/health`
      and notices crashes; least recently used unloaded first; plain-language errors.
    - `telemetry.rs`: VRAM (nvidia-smi) and RAM.
    - `ModelRef::Local`: a chat or task with a local model loads it first (`model_loading`).
  - **Testkit:** `fake-llama-server.exe` (probes, flags, 503 while loading, out of memory,
    crashes, argv file).
  - **UI:** the Models page (llama.cpp card or setup guide, folders, models with roles,
    Load/Unload, status, per-model settings, errors with the log, VRAM/RAM meters), local models
    in the model picker and in Settings, "Loading <model>…" in transcripts.
  - **Fixed:** settings updates now merge deeply (a shallow optimistic merge dropped nested
    settings).
  - **Tests (all green):**
    - Rust: 102 core unit, 15 agent, 8 chat, 11 client, 1 runtime integration, 4 app.
    - Vitest: 22.
    - Playwright E2E: 31.

- **M6**: context management.
  - **Backend** (`hearth-core/src/context.rs`, `chat.rs`, `agent/runner.rs`):
    - The window from the local model's settings or llama.cpp's `/props`; saved with each
      reply's usage (`contextWindow`) and sent as a `context` event.
    - Above 80%, the older part is summarized by the same model; recent messages keep 35% of the
      window; cuts fall before a user or assistant message, never inside a tool exchange.
    - Summaries are `summary` messages (migration 4 adds `covers_until`); requests carry the
      newest summary in the system prompt plus the messages after it. Nothing is deleted.
    - "Exceeds the available context size" from llama-server: its `n_ctx` wins, the earlier part
      is summarized, and the request is tried once more. Works for chats and coding tasks.
  - **UI:** the context meter in the composer (ring and percentage, with a tooltip), a divider
    where earlier messages were summarized (opens to show the summary), and "Summarizing the
    earlier conversation to make room…" while it happens. The chat transcript now also shows
    "Loading <model>…" before the reply starts.
  - **Tests (all green):**
    - Rust: 107 core unit, 17 agent, 9 chat, 11 client, 1 runtime, 4 app.
    - Vitest: 23.
    - Playwright E2E: 32.

- **M7**: the terminal panel.
  - **Backend** (`hearth-core/src/terminal/`): ConPTY pseudo consoles made with the `windows`
    crate (no extra dependency); shells start suspended and run in their own Job Object; a
    UTF-8 stream decoder; 256 KB of scrollback replayed when the UI attaches; exit codes.
    Tauri commands `terminal_open/attach/write/resize/close`, `terminals_list`.
  - **UI:** the panel under coding sessions (Terminal button, Ctrl+`), xterm.js 6 tabs per
project (Git Bash or PowerShell from "+"), drag or arrow keys to resize, Ctrl+C copies a
selection and Ctrl+V pastes, themes for light and dark. The Agent tab lists the agent's
commands with their live output. A Vite plugin fixes xterm.js under `freezePrototype`.
  - **Fixed:** the UI's tool-name aliases now match hearth-core's (`Bash`, `shell`…).
  - **Tests (all green):**
    - Rust: 113 core unit (real ConPTY sessions with cmd.exe), 17 agent, 9 chat, 11 client,
      1 runtime, 4 app.
    - Vitest: 24.
    - Playwright E2E: 33 (a real Git Bash in the panel).

- **M8**: web search and reading pages.
  - **Backend** (`hearth-core/src/web/`): DuckDuckGo (HTML, no key), Brave, Tavily and SearXNG,
    all giving the same hits; pages read with Readability as Markdown; local and private
    addresses refused unless allowed (checked on every redirect, connections pinned to the
    checked address); keys in the `kv` table. Checked once by hand against the real DuckDuckGo
    and rust-lang.org (`live_web`, an ignored test).
  - **Chat:** the Web button turns web search on for a chat; replies search and read for up to
    five steps, saved in the tree; the last step has to answer (`tool_choice: none`, same tools,
    so the server reuses its prompt cache).
  - **Agent:** `web_search` and `web_fetch` (Network class: ask first except in Full auto), with
    the usual aliases (`browser.search`, `fetch_url`…).
  - **UI:** search and read lines (searches open to their results), source chips under the
    answer, Settings → Web (provider, key, SearXNG address, local pages, a test search).
  - **Tests (all green):**
    - Rust: 126 core unit, 18 agent, 11 chat, 11 client, 1 runtime, 4 app.
    - Vitest: 25.
    - Playwright E2E: 34 (Settings → Web, then a reply that searches, reads and cites).

- **M9**: MCP servers.
  - **Backend** (`hearth-core/src/mcp/`): a small own client instead of rmcp (it ships a new
    major API every few weeks; Hearth needs only `initialize`, `tools/list` and `tools/call`):
    stdio (JSON lines, `ping` answered, other server requests refused) and Streamable HTTP
    (JSON or SSE answers, `Mcp-Session-Id`, `MCP-Protocol-Version`). Commands found like a
    shell would (`npx.cmd`), run in a Job Object, stderr kept for errors. Settings in the `kv`
    table; Claude Desktop style import; servers restart when changed; a crash marks the server
    failed.
  - **Agent:** `mcp__<server>__<tool>` tools after the built-in ones; External class with
    "Always allow this tool here" saved per project (`permission_rules`, class `mcp`).
  - **Testkit:** `fake-mcp-server.exe` (tools `echo`, `add`, `fail`; failure and crash
    switches) and `POST /mcp` on the mock server.
  - **UI:** Settings → MCP servers: add or edit (command or URL), import JSON, on/off, restart,
    remove, state with tools or the error and what the server printed.
  - **Tests (all green):**
    - Rust: 134 core unit, 19 agent, 11 chat, 11 client, 5 MCP, 1 runtime, 4 app.
    - Vitest: 25.
    - Playwright E2E: 35 (a server added in Settings, then a task that uses its tool).

- **M10**: attachments and images.
  - **Backend:** the `attachments` table (migration 5); attachments checked and saved with the
    user message (chats and coding tasks), loaded with message paths, never sent to the UI in
    bulk (`attachment_data` for one); an edited message keeps its files. Requests get attached
    text as `<attachment>` blocks and images as `image_url` parts when the model can see them
    (`Runtime::vision`, llama.cpp `/props` modalities); otherwise a note in the message and a
    notice in the transcript. Context estimates count attachments.
  - **UI:** the paperclip, drag and drop (Tauri's drop handler off), and paste; chips while
    files are read (PDF text with pdf.js 6.4, images resized); attachments on messages, images
    loaded when shown and opened larger on click; notices now show in chats too.
  - **Fixed:** a store deadlock (re-locking the database while holding it) found by the new
    tests before it shipped.
  - **Tests (all green):**
    - Rust: 134 core unit, 19 agent, 14 chat, 11 client, 5 MCP, 1 runtime, 4 app.
    - Vitest: 28.
    - Playwright E2E: 37 (a Markdown file, a PDF and an image to a vision model; an image to a
      model without vision).

- **M11**: polish, installer and docs.
  - **Guide** in the sidebar: docs/models.md in the app (bundled, reads offline).
  - **Get started** card on an empty chat or session while no model is chosen: llama.cpp found,
    model files found, a model chosen, each with where to do it.
  - **Ctrl+K** command palette: actions (new chat or session, open a project, web search, the
    terminal, screens, sidebar, theme) and recent conversations, filtered as you type.
  - The taskbar button flashes when a reply finishes or the agent asks something while Hearth
    is in the background.
  - The NSIS installer (per-user) builds with `pnpm build`.
  - README rewritten; decision records 0004 (own MCP client) and 0005 (ConPTY directly).
  - **Fixed after a code review of M7–M10:**
    - MCP servers set up as `npx` (the usual way) failed: the lookup found Node's extensionless
      shell script before `npx.cmd`. Names without an executable extension now only match
      `.com`, `.exe`, `.bat` or `.cmd`.
    - HTTP MCP notifications could hang a stopped task (now 10 s at most); a stray non-UTF-8
      line no longer ends a server's connection; SSE answers no longer break characters split
      between network chunks; a request sent as a server ends no longer waits out its timeout.
    - A server that quits on its own shows as stopped at once (not at the next call), a stale
      connection can't overwrite a newer one's state, and restarting or removing a server ends
      a start still in progress.
    - Page reading's address checks held only for direct connections: it now bypasses system
      proxies when local pages aren't allowed, also refuses IPv4-compatible, NAT64, 6to4,
      site-local, Teredo and documentation IPv6 forms, parses HTML off the async threads and
      gives a page a minute at most.
    - A user message and its files are saved in one transaction; transcripts load attachment
      sizes, not their contents (requests still get them).
    - Terminals: closing the console no longer holds a lock that resizing waits on; a failed
      thread start no longer leaks the console.
  - **Tests (all green):**
    - Rust: 135 core unit, 19 agent, 14 chat, 11 client, 6 MCP, 1 runtime, 4 app (the review
      fixes added tests for the `npx` lookup, split characters in SSE, IPv6 forms and a server
      that quits on its own).
    - Vitest: 28.
    - Playwright E2E: 38 (setup card, guide and palette added to the smoke tests).

- **Accessibility pass** (after M11): axe-core (WCAG 2.2 A/AA and best practices) on every
  screen and state, in light and dark, as a new E2E spec (`e2e/a11y.spec.ts`). Fixed:
  - Text contrast: the lightest text read at 2.7:1 (light) and 3.3:1 (dark). The two secondary
    text tokens moved so every text color reaches 4.5:1 on all resting backgrounds; filled
    buttons use a deeper amber in light mode (white text 5:1), the danger button gets dark text
    in dark mode, and light-mode error text is a deeper red.
  - The model picker's spoken name now contains its visible text ("Model: mock-model"); the
    Changes button's name includes its count.
  - An open conversation's title is the page's heading (h1); the chat settings popover has a
    name.
  - Terminal tabs follow the ARIA tabs pattern: only tabs in the tab list, arrow keys and
    Home/End move between them (the focus stays on the tabs), Delete closes a shell; a click
    still moves into the shell.
  - Not changed: wide code blocks aren't given a tab stop, because WebView2 (Chromium 130+)
    already puts scrolling regions in the Tab order.
  - **Tests:** Playwright E2E 41 (3 accessibility tests; the terminal test now covers the arrow
    keys and Delete).

- **Long conversations** (after M11): every streamed token used to render the whole
  conversation again. Measured in the browser preview (development build) with 300 messages,
  each reply with a table, code and math:

  |                          | Before | After  |
  | ------------------------ | ------ | ------ |
  | Each token, median       | 40 ms  | 2 ms   |
  | Each token, slowest 10 % | 58 ms  | 3 ms   |
  | No history (for scale)   | 2.9 ms | 2.1 ms |
  - Chat messages, web steps, agent steps and tasks are memoized; finished agent steps no
    longer receive the live run, and per-step answer maps and web turns compare by contents
    (they're rebuilt on every change).
  - Messages away from the screen skip layout and painting (`content-visibility: auto`, with
    a clip margin so focus rings and shadows aren't cut off); the newest six always render,
    so opening a conversation lands exactly at its end.
  - **Tests:** Vitest 31 (a render-count test for both transcripts that fails on the old code;
    comparison helpers).

- **Fixed: a step could vanish from the transcript during a run.** After starting a run, the
  app re-read the conversation from the database and put that copy on screen. When the run's
  events arrived during the read (likely under load, or with quick tools), a step that had just
  finished with its tool calls disappeared until the run ended. This was the E2E test that
  failed once under load ("Ask mode shows the diff first"): the permission card showed, the
  edit's row didn't. While a run goes on, a re-read now keeps what its events showed
  (`withNewer` in `lib/runReducer.ts`). A store test reproduces the race with a slow read and
  fails on the old code; Vitest 36.

## What's left

- **A run with a real model.** Everything is tested against the mock model server; llama.cpp
  and the models are for you to install (Guide). The first real run may show model-specific
  quirks (tool-call formats, thinking tags) that the parsers don't cover yet.
- **Coding sessions** can't edit or regenerate earlier messages (chats can).
- **MCP:** tools only (no resources, prompts or sampling); servers on the old SSE transport
  aren't supported (ADR 0004).
- **Web:** DuckDuckGo limits automated searches now and then (Hearth says so; Brave, Tavily or
  SearXNG avoid it). PDFs on the web aren't read.
- **Attachments:** images need a model with vision; scanned PDFs have no text to read (no OCR).
- **Opening a very long conversation** renders every message's Markdown at once: about 0.6 s
  for 300 messages with code and math in a development build (less in the installed app).
  Streaming and scrolling stay fast; rendering only the messages on screen would be the next
  step if that ever matters.
- **The installer isn't signed,** so Windows SmartScreen warns the first time.
