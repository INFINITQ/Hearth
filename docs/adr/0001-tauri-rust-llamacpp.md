# 0001. Tauri 2 + Rust backend + React UI, models through llama.cpp

Status: accepted (2026-11-02)

## Context

Hearth is a Windows 11 desktop app for chatting with local models and for a coding agent that
edits files and runs commands. The PC has 12 GB of VRAM and 32 GB of RAM, so every megabyte the app
itself uses is a megabyte the model can't have. The same author already ships Tauri apps (Folio)
with a proven window, WebView2 hardening and end-to-end test harness.

Runtimes considered: llama.cpp's `llama-server`, LM Studio, Ollama. The best models for this PC are
mixture-of-experts models whose expert weights must live in system RAM. llama.cpp places those
best (`--fit`, `--n-cpu-moe`), runs any GGUF from Hugging Face with its own chat template
(including community fine-tunes), and is open source.

## Decision

- **Shell:** Tauri 2 (pinned 2.12.5), WebView2, the Folio window code (frameless, Snap Layouts,
  hardened WebView2).
- **Backend:** Rust. All logic lives in `crates/hearth-core` with no Tauri dependency, so it is
  tested directly. The Tauri crate only wires commands and events.
- **UI:** React 19 + TypeScript + Tailwind v4, "Calm minimal" design tokens.
- **Models:** llama.cpp `llama-server`, started and managed by Hearth. Any OpenAI-compatible server
  (LM Studio, Ollama, a remote box) can be added as a plain connection.

## Consequences

- A small installer and roughly 200 MB of RAM for the app, leaving the rest for the model.
- Agent tools (files, search, processes, terminals) are written in Rust, using ripgrep's crates,
  ConPTY and Windows Job Objects directly.
- llama.cpp's command line changes often. Hearth probes `--help` and only passes flags a build
  supports (see build notes).
