# Hearth

A private desktop app for Windows 11 that runs open models (Qwen, Gemma, GPT-OSS…) on your own PC,
for two jobs:

- **Chat:** a calm, minimal ChatGPT/Claude-style conversation app with streaming markdown, code and
  math, thinking, branching edits, files, PDFs, images and web search.
- **Code:** a Claude-Code-style agent that opens a project folder, reads and edits files, runs
  commands, searches the web, uses MCP servers, and asks before anything risky. Every file
  change can be undone.

You choose which model does which job (the same one or two different ones). Models run through
llama.cpp's `llama-server`, which Hearth starts, stops and swaps for you; any OpenAI-compatible
server (LM Studio, Ollama, a remote machine) works too. Nothing leaves your machine unless you turn
on web search or add a remote server.

## Getting started

1. **Install Hearth:** build the installer (below) and run
   `target\release\bundle\nsis\Hearth_0.1.0_x64-setup.exe`. It installs for your user only.
2. **Install llama.cpp and download a model:** the guide is in the app (**Guide** in the sidebar)
   and in [docs/models.md](docs/models.md). It covers the CUDA build for RTX 50-series cards and
   the best models for an RTX 5070 with 32 GB of RAM, official and uncensored.
3. **Point Hearth at it:** on the **Models** page, add your model folder (and llama-server, if it
   isn't on PATH), then choose the model for chats and for coding. The **Get started** card on an
   empty chat walks through the same steps.

## What's in it

- **Chats:** versions of replies and edits as branches, thinking shown apart, a context meter;
  long conversations are summarized when they near the model's limit (nothing is deleted).
- **Web search:** the **Web** button lets a reply search (DuckDuckGo, or Brave, Tavily or your
  own SearXNG) and read pages, with the sources under the answer. Pages on your PC or network
  aren't read unless you allow it, and until then pages are read without your proxy (allow it
  if your network only works through one).
- **Attachments:** text and code files, PDFs (their text) and images (for models with vision),
  picked, dropped or pasted.
- **Coding sessions:** file tools with diffs, commands in Git Bash or PowerShell (live output, a
  Job Object per command), permission modes (Plan, Ask, Auto-edit, Full auto), "Always allow" rules
  per project, undo per task, the Changes panel, sending a message while the agent works, and a
  to-do list.
- **Terminal panel:** real shells under the session (ConPTY), plus an Agent tab with the output
  of the agent's commands.
- **MCP servers:** add or import them (Claude Desktop format) in Settings; their tools join the
  agent's, asking first.
- **Models page:** found models with their size, quantization and context, load and unload, per-model
  settings, VRAM and RAM use, plain-language errors with the server's log.

Keyboard: **Ctrl+K** commands and conversations · **Ctrl+N** new chat or session · **Ctrl+B**
sidebar · **Ctrl+,** settings · **Ctrl+`** terminal · **Shift+Tab** permission mode · **Esc** stop.
On the terminal tabs, **←/→** switch tabs and **Delete** closes a shell.

## Status

All planned milestones are built and tested:

|     | Milestone                                                   | State |
| --- | ----------------------------------------------------------- | ----- |
| M0  | Scaffold, window and title bar, design tokens, models guide | done  |
| M1  | Core: SQLite store, LLM streaming client, mock model server | done  |
| M2  | Chat end to end                                             | done  |
| M3  | Coding agent: file tools and permissions                    | done  |
| M4  | Coding agent: commands, undo, diff review, stop and steer   | done  |
| M5  | llama-server runtime manager and Models page                | done  |
| M6  | Context management (compaction)                             | done  |
| M7  | Terminal panel                                              | done  |
| M8  | Web search and fetch                                        | done  |
| M9  | MCP servers                                                 | done  |
| M10 | Attachments and images                                      | done  |
| M11 | Polish, installer, docs                                     | done  |

It hasn't run against a real model yet: llama.cpp and the models are yours to install (see the
guide). Every test uses Hearth's mock model server. [docs/STATUS.md](docs/STATUS.md) has the
details and what's left.

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
cargo build -p hearth-testkit --bins     # the mock servers the Rust and E2E tests use
pnpm clippy; pnpm test:rust              # Rust lints and tests
pnpm build:e2e; pnpm e2e                 # debug build with the UI embedded, then Playwright
pnpm build                               # release build and NSIS installer (per-user)
```

End-to-end tests drive the real app over WebView2's debugging port with the window hidden and a
throwaway data folder. They never touch your settings or conversations.

## Layout

```
apps/hearth/            React UI (src/) and the Tauri shell (src-tauri/)
crates/hearth-core/     app logic with no Tauri dependency (settings, runtime, chat, agent, tools,
                        web, MCP, terminals, storage)
crates/hearth-testkit/  mock model server, fake llama-server, fake MCP server
e2e/                    Playwright tests over WebView2's debugging port
docs/                   models guide, architecture, decision records, build notes, status
```

## Docs

- [docs/models.md](docs/models.md): install llama.cpp, pick and download models
- [docs/architecture.md](docs/architecture.md): how the pieces fit
- [docs/adr/](docs/adr/README.md): decision records
- [docs/build-notes.md](docs/build-notes.md): engineering notes (Windows traps, agent design, test hooks)
- [docs/STATUS.md](docs/STATUS.md): what each milestone built, test counts, what's next
