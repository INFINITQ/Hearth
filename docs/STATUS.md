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

## Next: M4, commands, checkpoints, diff panel, stop and steer

- `hearth_core::proc`: Job Objects, CREATE_NO_WINDOW, Git Bash / PowerShell resolution.
- Tools:
  - `run_command`: timeout, background mode, head and tail of the output, safe-list analysis;
  - `command_output` and `kill_process`.
- Checkpoints (file snapshots per turn), "Undo turn", a Changes panel with per-file revert.
- Steering: a message typed mid-run goes in before the next model call.
- Persistent "always allow" rules for command prefixes (the `permission_rules` table).
