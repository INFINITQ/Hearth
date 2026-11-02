# Hearth

A private desktop app for Windows 11 that runs open models (Qwen, Gemma, GPT-OSS…) on your own PC,
for two jobs:

- **Chat:** a calm, minimal ChatGPT/Claude-style conversation app with streaming markdown, code and
  math, thinking, branching edits, files, images and web search.
- **Code:** a Claude-Code-style agent that opens a project folder, reads and edits files, runs
  commands in a terminal, debugs, and asks before anything risky. Every change can be undone.

You choose which model does which job. Models run through llama.cpp's `llama-server`, which Hearth
starts, stops and swaps for you. Nothing leaves your machine unless you turn on web search.

**Setting up models:** see [docs/models.md](docs/models.md). It covers installing llama.cpp, the
best models for an RTX 5070 with 32 GB of RAM (official and uncensored), and troubleshooting.

## Status

In development. Milestones:

|     | Milestone                                                   | State |
| --- | ----------------------------------------------------------- | ----- |
| M0  | Scaffold, window and title bar, design tokens, models guide | done  |
| M1  | Core: SQLite store, LLM streaming client, mock model server | done  |
| M2  | Chat end to end                                             | done  |
| M3  | Coding agent: file tools and permissions                    | done  |
| M4  | Coding agent: commands, undo, diff review, stop and steer   | done  |
| M5  | llama-server runtime manager and Models page                | done  |
| M6  | Context management (compaction)                             | done  |
| M7  | Terminal panel                                              | next  |
| M8  | Web search and fetch                                        |       |
| M9  | MCP servers                                                 |       |
| M10 | Attachments and images                                      |       |
| M11 | Polish, installer, docs                                     |       |

## Development

Requirements: Windows 11, Node 24, pnpm 11, Rust 1.99 (MSVC) with the VS Build Tools, WebView2.

```powershell
pnpm install
pnpm dev            # the app with hot reload (Vite on :5180 + Tauri)
pnpm --filter @hearth/app dev:web   # the UI alone in a browser, with a fake backend
```

Checks:

```powershell
pnpm typecheck; pnpm lint; pnpm test     # TypeScript, ESLint, Vitest
pnpm clippy; pnpm test:rust              # Rust lints and tests
pnpm build:e2e; pnpm e2e                 # debug build with the UI embedded, then Playwright
pnpm build                               # NSIS installer (per-user)
```

End-to-end tests drive the real app over WebView2's debugging port with the window hidden and a
throwaway data folder. They never touch your settings or conversations. All tests use a mock
model server, so no model is needed.

## Layout

```
apps/hearth/            React UI (src/) and the Tauri shell (src-tauri/)
crates/hearth-core/     app logic with no Tauri dependency (settings, runtime, chat, agent, tools, storage)
e2e/                    Playwright tests over WebView2's debugging port
docs/                   models guide, architecture, decision records, build notes
```

## Docs

- [docs/models.md](docs/models.md): install llama.cpp, pick and download models
- [docs/architecture.md](docs/architecture.md): how the pieces fit
- [docs/adr/](docs/adr/README.md): decision records
- [docs/build-notes.md](docs/build-notes.md): engineering notes (Windows traps, agent design, test hooks)
