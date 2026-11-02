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

## Next: M7, the terminal panel

- ConPTY shells (Git Bash by default, PowerShell) in xterm.js tabs below the coding session.
- Each terminal in its own Job Object; closing a tab or the app ends its processes.
- An "Agent" tab that follows the output of the agent's commands.
