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
  - **Tests:** 62 Rust, 13 Vitest, 15 Playwright E2E (mock server).

## Next: M3, agent file tools and permissions
See build-notes "Agent design" and "Tools". Code mode reuses RunManager and the transcript and
adds:
- a workspace picker;
- tools (read, list, glob, grep, write, edit) with prepare/permission/execute;
- the permission modes;
- tool cards.
