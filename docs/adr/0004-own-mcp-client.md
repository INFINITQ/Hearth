# 0004. Hearth's own small MCP client, for tools

Status: accepted (2026-11-03)

## Context

The plan was to use rmcp, the official Rust SDK for the Model Context Protocol. By November 2026
it had moved through major versions 3.3 to 3.8 in about six weeks, each changing its API, and it
brings a sizeable dependency tree (macros, schema generation). Hearth uses a small part of MCP:
starting a server, `initialize`, `tools/list` and `tools/call`, over stdio or Streamable HTTP.

## Decision

Hearth has its own client in `hearth-core/src/mcp/client.rs` (about 450 lines) for exactly that
part: JSON-RPC 2.0 over a server's stdin/stdout (one JSON object per line) or over HTTP POST with
JSON or SSE answers and the session and protocol-version headers. Requests a server sends are
refused, except `ping`. Servers run through `proc::spawn`, so each is in its own Job Object.

## Consequences

- Tools work with any server that follows the spec; resources, prompts, sampling and
  elicitation aren't available. Adding them means growing this client or moving to rmcp.
- The older SSE transport (separate GET stream and POST endpoint) isn't supported; such servers
  fail with a clear error.
- The testkit has a matching fake server (`fake-mcp-server.exe` and `POST /mcp` on the mock), so
  the client is tested end to end without network or Node.
