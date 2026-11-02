# 0003. The UI never touches the network; Rust does

Status: accepted (2026-11-02)

## Context

The webview shows model output, which can contain text from web pages, MCP servers and project
files: untrusted input. A script running in the webview could answer permission prompts or type
into terminals, and the page could leak conversation text through images or requests.

## Decision

- **Content security policy** (Folio's, in `tauri.conf.json`): only the app itself and Tauri's
  IPC. No remote scripts, images, fonts or connections; `freezePrototype` on. Release builds
  refuse to start without it.
- **WebView2 backstops:**
  - a dead-end proxy (`--proxy-server=http://127.0.0.1:9`);
  - InPrivate profile;
  - no background networking;
  - crash dumps kept local;
  - every permission request denied;
  - WebRTC constructors removed before page scripts run.
- **ESLint** bans `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`, `sendBeacon` and
  `window.open` in UI code.
- **Everything that needs the network goes through Rust:** llama-server on loopback, web search,
  page fetches and MCP over HTTP. There the rules apply: per-domain approval, private-address
  filtering, size and time limits.
- **Links** open in the default browser through a Rust command (http, https and mailto only). Remote
  images in model output render as link chips, not images.

## Consequences

- UI code can't take shortcuts with `fetch`; every new network feature needs a Rust command.
- The end-to-end smoke test checks that `fetch`, images, WebSockets and pop-ups can't leave the app.
