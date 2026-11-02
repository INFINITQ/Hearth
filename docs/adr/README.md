# Decision records

Short records of decisions that shape Hearth, so the reasons stay with the code. Each one says what
was decided, why, and what it costs. A new record supersedes an old one rather than editing it.

| #                                            | Decision                                                    |
| -------------------------------------------- | ----------------------------------------------------------- |
| [0001](0001-tauri-rust-llamacpp.md)          | Tauri 2 + Rust backend + React UI, models through llama.cpp |
| [0002](0002-one-llama-server-per-model.md)   | One app-managed llama-server process per loaded model       |
| [0003](0003-ui-never-touches-the-network.md) | The UI never touches the network; Rust does                 |
| [0004](0004-own-mcp-client.md)               | Hearth's own small MCP client, for tools                    |
| [0005](0005-conpty-without-portable-pty.md)  | Terminals use ConPTY directly, not portable-pty             |

Template:

```markdown
# NNNN. Title

Status: accepted (date)

## Context

## Decision

## Consequences
```
