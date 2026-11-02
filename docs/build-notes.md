# Build notes

The engineering decisions behind Hearth that aren't obvious from the code: llama-server facts,
Windows traps, how the agent compensates for local models, contracts, and test hooks. They came
from the design review on 2026-11-02. Keep this file current as milestones land.

## llama-server (llama.cpp v0.8 / b11960 era)

- **Defaults that matter:**
  - `--jinja` is on by default (`--no-jinja` turns it off).
  - `--fit on` is the default: unset `-ngl` / MoE placement / context are fitted to free memory with a
    1024 MiB margin per device (`--fit-target`).
  - `-c 0` means "the model's trained context" (a 256K KV cache): **always pass `-c`**.
  - `-np` defaults to auto (4 slots sharing one KV cache): **always pass `-np 1`**.
  - `--cache-ram` defaults to an 8192 MiB host-RAM prompt cache. Size it from free RAM: use 2048
    when the model is over 16 GB on a 32 GB machine.
  - `--mlock` / `--no-mmap` became `-lm/--load-mode`.
- **Flags change often:** probe `--version`, `--help` (to learn the flag set) and `--list-devices`
  (it also catches missing cudart DLLs, which mean a silent CPU fallback). Cache the probe by path,
  size and mtime. Emit only flags the build lists, and flag unknown user extra args before launch.
- **Launch:**

  ```
  -m <gguf> [--mmproj <f>] --host 127.0.0.1 --port <free> --api-key <random> --alias <presetId>
  -c <ctx> -np 1 --jinja --reasoning-format deepseek --no-webui -fa auto -ctk/-ctv <q8_0|f16>
  --cache-ram <MiB> (--fit on | -ngl <n> --n-cpu-moe <k>) [-b/-ub] [-t] <extra args>
  ```

- **Endpoints:**
  - `/health` returns 503 "Loading model" until the model is ready, then 200.
  - `/props.default_generation_settings.n_ctx`; `/props` also has `modalities` and `chat_template_caps`.
  - `/v1/chat/completions` streams `reasoning_content`, `tool_calls` deltas, `usage` and `timings`.
- **Per-request fields:** `chat_template_kwargs` (Qwen: `enable_thinking`), `reasoning_effort`
  (gpt-oss; `none` turns thinking off), `return_progress:true` (prompt processing progress),
  `cache_prompt:true`.
- **Thinking control by family:**
  - **Qwen3.6:** thinks by default; disable with `chat_template_kwargs.enable_thinking=false`.
  - **Gemma 4:** thinking is on when `<|think|>` starts the system prompt; never send old thoughts back.
  - **gpt-oss:** `reasoning_effort`; send its reasoning back only within the current turn.
- **Log lines worth parsing:**
  - memory: `load_tensors: CUDA0 model buffer size = X MiB`, `… KV buffer size`;
  - ready: `server is listening on http://127.0.0.1:PORT`;
  - failures: out-of-memory and "couldn't bind".

## Child processes (`hearth_core::proc`; clippy bans raw `Command::new`)

- **Spawning:**
  - every spawn gets `CREATE_NO_WINDOW` (a GUI app otherwise flashes a console window per child);
  - spawn `CREATE_SUSPENDED`, then `AssignProcessToJobObject`, then resume the threads (found via
    Toolhelp32);
  - the job has `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; `TerminateJobObject` kills the whole tree.
- **One job each** for llama-server, every agent command, every MCP server and every PTY child
  (assigned by PID after spawn).
- **Always drain stdout and stderr** into a log ring and file. A full 4–64 KB pipe blocks the child
  ("hangs while loading").
- **Ports:** bind `127.0.0.1:0` to find a free port; retry if the server can't bind (Hyper-V
  reserves port ranges). The local HTTP client uses `.no_proxy()`.
- **`.cmd` shims** (npx, pnpm): resolve them with the `which` crate (honours PATHEXT). Rust ≥1.77
  escapes `.cmd` arguments safely or returns an error.

## Shells for agent commands

- **Git Bash:**
  - Find it via `HKLM\SOFTWARE\GitForWindows\InstallPath`, or from `git.exe` go to `..\bin\bash.exe`.
    Never use `C:\Windows\System32\bash.exe`: that's WSL.
  - Write the command to a temp `.sh` file (UTF-8, LF line endings, first line `exec 2>&1`) and run
    `bash.exe -l <script>` with `CHERE_INVOKING=1`, so the login profile doesn't `cd $HOME`.
  - Never build `-c "<cmd>"` strings: MSVC and Cygwin quoting differ.
- **PowerShell:**
  - Run `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand <base64
UTF-16LE>`, prefixed with `[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);
$ProgressPreference='SilentlyContinue'`.
  - Commands over ~8k characters go into a temp `.ps1` run with `-File`.
- **Environment:**
  - stdin = NUL;
  - `PAGER=cat GIT_PAGER=cat GIT_TERMINAL_PROMPT=0 GCM_INTERACTIVE=never GIT_EDITOR=true
GIT_SEQUENCE_EDITOR=true TERM=dumb npm_config_yes=true PYTHONUTF8=1 PYTHONIOENCODING=utf-8`;
  - don't set `CI` (it changes how builds behave).
- **Output pipeline for the model:**
  - decode as UTF-8 if valid, else the OEM code page (`MultiByteToWideChar(CP_OEMCP)`);
  - strip ANSI, collapse `\r` overwrites, CRLF→LF;
  - keep the first and last 150 lines, with the full output saved as a blob.

  The terminal tab gets the raw bytes.

- **Microsoft Store python stub:** `python3` exits with 9009. List the real `python`/`py` paths in
  the environment block, and add a hint to the tool result when the stub's output appears.

## Paths, encodings, writes

- **Accepted path forms:** `C:\…`, `C:/…`, `/c/…`, `/mnt/c/…`, `~` and relative paths. Normalize
  with `dunce` / `std::path::absolute`.
- **Containment check:** normalize, compare lowercased, then canonicalize the nearest existing
  parent before any write (junctions can point outside the workspace).
- **Reject:** `CON PRN AUX NUL COMn LPTn`, trailing dots or spaces, and `:` alternate data streams.
  Report paths relative to the workspace with `/`.
- **Encodings:** detect BOM, UTF-16 or UTF-8, else guess with `chardetng`.
  - Edits match on an LF-normalized copy with an offset map, splice into the original bytes, and
    convert `new_string` to the file's line endings. BOM, encoding and trailing newline are kept.
  - New files: UTF-8 without BOM, using the line endings most sibling files use.
- **Writes:** temp file + `MoveFileExW(REPLACE_EXISTING)`, falling back to an in-place write. Retry
  3× at 100 ms (antivirus and indexer sharing violations). A read-only file gets a clear error.

## How the agent compensates for local models

| Failure                                                                                                                                                                         | Compensation                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tool call written as text (`<tool_call>{…}`, Qwen `<function=…><parameter=…>`, `[TOOL_CALLS]`, `<\|python_tag\|>`, leaked harmony markup, fenced JSON), or inside the reasoning | Only when no native `tool_calls` came back: run fallback parsers over the content, then the reasoning. Accept a call only if it names a registered tool. Strip the markup from visible text, and show "Preparing tool call…" while streaming.                                                                                                                                                                                 |
| Broken JSON arguments                                                                                                                                                           | Lenient repair (raw newlines, trailing commas, single quotes, `True`/`None`, truncated end, JSON encoded twice), then `serde_path_to_error`. If that fails, the tool result explains the exact error and gives a minimal example.                                                                                                                                                                                             |
| Names from other agents                                                                                                                                                         | Tool aliases: `Bash\|bash\|shell\|run_shell_command`→`run_command`, `Read\|cat`→`read_file`, `Edit\|str_replace\|replace`→`edit_file`, `Write\|create_file`→`write_file`, `LS\|list_directory`→`list_dir`, `Grep\|search_file_content`→`grep`. Argument aliases: `file_path\|filename`→`path`, `old_str`→`old_string`, `cmd`→`command`. Convert number and boolean strings, unwrap `{arguments:{…}}`, treat `null` as absent. |
| Edits that don't match                                                                                                                                                          | Try exact → line-ending-normalized → trailing-whitespace-insensitive → indentation-insensitive (re-indent the new text) → `cat -n` line prefixes stripped. Check uniqueness. The error shows the closest match ±3 lines with line numbers.                                                                                                                                                                                    |
| Loops                                                                                                                                                                           | Same tool + arguments + result 3× → nudge in the result; 5× → pause. A 50-step limit raises a "Continue?" event.                                                                                                                                                                                                                                                                                                              |
| Context blowups                                                                                                                                                                 | Caps: `read_file` 2000 lines, `grep` 100 matches (file-list mode by default), `list_dir` 500 entries, commands first and last 150 lines, `web_fetch` 20k characters. All can be paged; full output is kept as blobs.                                                                                                                                                                                                          |
| Forgetting the plan                                                                                                                                                             | `todo_write` echoes the list back; every 10 steps a short to-do reminder rides on the newest tool result.                                                                                                                                                                                                                                                                                                                     |
| OS confusion                                                                                                                                                                    | The environment block states Windows 11, the shell and its version, both path styles, the tools found, and "no sudo/apt".                                                                                                                                                                                                                                                                                                     |
| Hanging servers / watch modes                                                                                                                                                   | `background:true` + `command_output` / `kill_process`. The timeout message tells the model to use background mode.                                                                                                                                                                                                                                                                                                            |
| Output cut off mid tool call                                                                                                                                                    | `finish_reason=length` → tell the model to make a smaller edit.                                                                                                                                                                                                                                                                                                                                                               |
| Special tokens leaking                                                                                                                                                          | Per-profile strip list (`<\|im_end\|>`, `<\|endoftext\|>`, `<end_of_turn>`).                                                                                                                                                                                                                                                                                                                                                  |
| Prompt injection (web, MCP, AGENTS.md)                                                                                                                                          | Untrusted output is wrapped in clear delimiters. Network and external tools are never auto-approved for a new domain or tool outside Full auto. Workspace trust prompt.                                                                                                                                                                                                                                                       |

## Agent design

- **Two-phase tools:** `prepare()` validates, resolves paths and computes the diff or command
  analysis with no side effects. Then the permission check runs, then `execute()`. Prompts show
  the real diff.
- **Read before write:** keep each file's hash and mtime as of the model's last read in the session.
  An edit to a file that changed (or was never read) fails and tells the model to re-read it.
- **Every tool call gets an answer:** each assistant `tool_call` is followed by a tool message;
  denied, cancelled, step-limit and crash cases get synthesized ones. A validator checks this
  before every request (chat templates break otherwise).
- **Stable prompt prefix** (llama-server's prompt cache):
  - the system prompt and tool list stay byte-identical within a session;
  - the environment block is frozen at session creation (`sessions.env_snapshot`);
  - tools are sorted, with no timestamps;
  - dynamic reminders go only on the newest message.
- **Runs live in Rust (`RunManager`):** `run_attach(runId, channel)` sends a `snapshot` first, so
  React StrictMode, reloads and renderer crashes are all safe. The app event `hearth://run-state`
  drives sidebar spinners and "needs approval" badges.
- **Code-mode branching:** editing an earlier prompt doesn't rewind files. Offer "Undo files to
  before this turn, then edit".
- **Workspace trust prompt** on first open; AGENTS.md (or CLAUDE.md as a fallback) and repository
  content are untrusted.

### Tools

| Tool                    | Parameters                                                                                                              | Class                                             |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| `read_file`             | `path`; `offset?` ≥1; `limit?` ≤2000 (2000)                                                                             | read (outside ws: ask)                            |
| `list_dir`              | `path?`="."; `depth?` 1–5 (2); `include_hidden?`                                                                        | read                                              |
| `glob`                  | `pattern`; `path?`                                                                                                      | read                                              |
| `grep`                  | `pattern`; `path?`; `glob?`; `case_insensitive?`; `output?` files\|content\|count; `context?` 0–5; `max_results?` (100) | read                                              |
| `write_file`            | `path`; `content`                                                                                                       | edit                                              |
| `edit_file`             | `path`; `old_string`; `new_string`; `replace_all?`                                                                      | edit                                              |
| `run_command`           | `command`; `cwd?`; `timeout_s?` 1–600 (120); `background?`; `description?`                                              | execute                                           |
| `command_output`        | `process_id`; `wait_s?` 0–30; `tail_lines?`                                                                             | read                                              |
| `kill_process`          | `process_id`                                                                                                            | execute (automatic for the agent's own processes) |
| `todo_write`            | `todos: [{content, status: pending\|in_progress\|completed}]`                                                           | internal                                          |
| `ask_user`              | `question`; `options?` (2–5)                                                                                            | internal (pauses the run)                         |
| `web_search`            | `query`; `max_results?` 1–10 (5)                                                                                        | network                                           |
| `web_fetch`             | `url`; `max_chars?` (20000); `start_char?`                                                                              | network (per domain)                              |
| `mcp__<server>__<tool>` | the server's schema, sanitized                                                                                          | external                                          |

**Permission matrix:**

| Mode                | read in ws | read outside ws | edit in ws | edit outside ws | execute                                       | network                        | external                     |
| ------------------- | ---------- | --------------- | ---------- | --------------- | --------------------------------------------- | ------------------------------ | ---------------------------- |
| Plan                | allow      | ask             | deny       | deny            | safe-list only                                | search allow, fetch ask        | deny unless read-only        |
| Ask                 | allow      | ask             | ask        | ask             | ask                                           | ask                            | ask                          |
| Auto-edit (default) | allow      | ask             | allow      | ask             | ask, unless a rule or the safe list allows it | search allow, fetch per domain | ask, unless a rule allows it |
| Full auto           | allow      | allow           | allow      | allow           | allow (catastrophic patterns ask)             | allow                          | allow                        |

- **Safe read-only list:** `ls`, `pwd`, `cat`, `head`, `tail`, `wc`, `which`, `git status`,
  `git diff`, `git log`, `git show`, `rg`, `grep`, `find` (without `-exec`/`-delete`) and `--version`.
- Any compound command, substitution or redirect is never auto-allowed.

## Run events (Channel; each event has `seq` and `runId`)

- **Run lifecycle:**
  - `snapshot{messages, pending?, todos, state}` · `started{sessionId, userMessageId?, model}`
  - `waiting{reason: model_loading|model_swap|queue}` · `prompt_progress{processed, total, cached}`
- **Streaming:**
  - `message_started{messageId, parentId, step}` · `reasoning_delta` · `content_delta`
  - `tool_call_delta{messageId, index, callId?, name?, argsDelta}`
  - `message_finished{messageId, finishReason, toolCalls, usage?}`
- **Tools:**
  - `tool_started{callId, name, display}` · `tool_output{callId, chunk}`
  - `permission_request{requestId, callId, class, title, detail, choices}` · `permission_resolved`
  - `question{questionId, callId, question, options?}` · `question_answered`
  - `tool_finished{callId, toolMessageId, status, summary, display, durationMs, filesChanged?}`
- **State and end:**
  - `todos{items}` · `context{used, nCtx, breakdown}` · `compaction{phase, …}`
  - `steer{phase: queued|injected|dropped, …}` · `limit_reached{steps}`
  - `notice{level, code, text}` · `error{code, message, retryable}` ·
    `finished{status: completed|cancelled|failed|paused}`

Rust batches deltas (flushing every ~33 ms or 4 KB). Message content is written to SQLite every
~1 s and when the message ends; the FTS index updates only when a message completes.

**App events:**

- `hearth://runtime`
- `hearth://telemetry` (every 2 s while the window is visible)
- `hearth://run-state`
- `hearth://sessions`
- `hearth://mcp`
- `hearth://settings`
- `hearth://caption`

## SQLite (WAL mode, `foreign_keys=ON`, UTC milliseconds, UUIDv7 IDs)

- **Tables:**
  - `sessions(mode chat|code, title, workspace_id, model_ref_json, params_json, permission_mode,
search_mode, active_leaf_id, env_snapshot, ctx_summary_id, ctx_elide_before, pinned, archived, …)`
  - `messages(session_id, parent_id, role user|assistant|tool|summary, turn, step, content, reasoning,
reasoning_ms, tool_calls_json, tool_call_id, tool_name, tool_status, tool_display_json,
full_blob, model_label, finish_reason, status streaming|complete|interrupted|error, usage_json, …)`
  - `messages_fts` and `titles_fts` (FTS5 with their own copies: rowid-based FTS breaks after VACUUM
    on TEXT primary keys)
  - `attachments`, `sources`, `todos`
  - `checkpoints(session_id, turn, path NOCASE, before_blob, after_blob, …, UNIQUE(session, turn, path))`
  - `blobs(sha256, bytes, refcount)`: content is stored under `<data>/blobs/ab/<sha256>`
  - `permission_rules`, `workspaces`, `model_files`, `presets`, `connections`, `mcp_servers`, `kv`
- **Startup and resilience:**
  - On start, messages still marked `streaming` become `interrupted`.
  - Daily `VACUUM INTO backups/`, keeping 7.
  - A corrupt database is set aside and a fresh one started.

## Security

- **Secrets** (search keys, connection keys, MCP env vars and headers) are encrypted with DPAPI
  (Folio's `secure.rs`) and never returned to the UI.
- **`web_fetch`** uses a DNS-filtering resolver. At connect time it blocks loopback, private,
  link-local, carrier-grade NAT, unique-local and cloud-metadata addresses, which also defeats DNS
  rebinding. Tests opt in to loopback.
- **Markdown** renders with no raw HTML. Links go through Rust `open_link` (http, https and mailto
  only); remote images become link chips.
- **No IPC command executes arbitrary commands.** Commands run only inside agent runs or terminal
  tabs.

## Testing

- **Mock LLM script** (Rust builder or JSON; steps are consumed in order, one per request):

  ```json
  {
    "model": "mock-coder",
    "n_ctx": 32768,
    "steps": [
      {
        "expect": [{ "system_contains": "AGENTS.md" }, { "has_tool": "edit_file" }],
        "reply": {
          "reasoning": "…",
          "tool_calls": [{ "name": "read_file", "arguments": { "path": "src/calc.py" } }]
        }
      },
      {
        "reply": {
          "content": "Fixed.",
          "stream": { "chunk_chars": 3, "delay_ms": 10, "hold": "final" },
          "usage": { "prompt_tokens": 1800, "completion_tokens": 12 },
          "timings": { "predicted_per_second": 42.0 }
        }
      }
    ]
  }
  ```

  - **Expectations:** a failure returns HTTP 500 `mock expectation failed: …`.
  - **Faults:**
    - HTTP-level: `http{status, body}`, `disconnect_after_chunks`, `stall_ms`, `loading_503{times}`;
    - stream shape: `raw_sse[]`, `split_utf8`, `crlf_sse`, `no_done`, `raw_arguments`;
    - context: `context_exceeded{n_ctx, n_prompt}`, `prompt_progress[]`.
  - **Hold points:** the stream pauses until the test releases it, so stop, steer and mid-stream
    screenshots are deterministic.
  - **Control endpoints:** `POST /__mock/script`, `POST /__mock/release/<hold>`,
    `GET /__mock/requests`, `POST /__mock/reset`.

- **Fake llama-server** (`fake-llama-server.exe`):
  - **Arguments:** accepts llama-server's flags and writes its argv to `FAKE_LLAMA_ARGV_FILE`.
  - **Probes:** `--version`, `--help` and `--list-devices` print realistic output; `FAKE_LLAMA_HELP=old`
    and `FAKE_LLAMA_DEVICES=none` give variants.
  - **Loading:** checks the GGUF magic bytes; `/health` returns 503 for `FAKE_LLAMA_LOAD_MS`
    (default 800), then 200.
  - **Auth:** enforces `--api-key`.
  - **Faults (`FAKE_LLAMA_FAIL`):** `oom-on-load`, `crash-after-ms:N`, `hang-on-load`,
    `spawn-grandchild`, `bad-arg:<flag>`.
- **Mock MCP server** (rmcp server, stdio or HTTP): tools `echo`, `add`, `slow{ms}`, `fail`,
  `big_output`; `--crash-after N`.
- **E2E hooks** (debug builds only):

  | Variable                                                                    | Effect                                                                                                                           |
  | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
  | `HEARTH_BACKGROUND=1`                                                       | window never shown or focused                                                                                                    |
  | `HEARTH_TEST_DATA_DIR`                                                      | data in that folder; its own instance; sealed: no discovery of the user's llama-server or models, outbound HTTP to loopback only |
  | `HEARTH_TEST_SETTINGS=<json>`                                               | a settings merge patch applied at start (e.g. `runtime.llamaServerPath`, `runtime.modelFolders`)                                 |
  | `HEARTH_TEST_LLM_URL`                                                       | creates a "Mock" connection and sets it as both default models                                                                   |
  | `HEARTH_TEST_PICK_FOLDER`, `HEARTH_TEST_PICK_FILE`, `HEARTH_TEST_SAVE_PATH` | answers the native dialogs (folder, llama-server.exe, save)                                                                      |
  | `HEARTH_TEST_LINKS=<file>`                                                  | links and reveal requests are written to the file instead of opened                                                              |
  | `HEARTH_TEST_NOW`, `HEARTH_TEST_FAKE_SYSTEM`, `HEARTH_TEST_REDUCED_MOTION`  | deterministic screenshots                                                                                                        |

- **`pnpm verify` order:**
  1. `cargo fmt --check`
  2. clippy with `-D warnings`
  3. `cargo test`
  4. `pnpm typecheck`, `pnpm lint`, `pnpm format:check`, `pnpm test`
  5. `pnpm build:e2e`: testkit binaries, then `tauri build --debug --no-bundle`. The embedded
     frontend matters, because frozen prototypes only apply there.
  6. `pnpm e2e`
- **Live smoke tests** (`live_*`, `e2e/live.spec.ts`) run only when `HEARTH_LIVE_LLAMA_SERVER` and
  `HEARTH_LIVE_MODEL` point at existing files. Nothing is ever downloaded.

## Dependency choices

- **Rust:**
  - Runtime: tokio, tokio-util, futures-util.
  - HTTP: reqwest 0.13 with `default-features=false` + `native-tls` (SChannel: no aws-lc/CMake);
    eventsource-stream or our own SSE parser.
  - Arguments: schemars 1 + serde_path_to_error; jsonschema (no default features) for MCP arguments.
  - Storage and IDs: rusqlite (bundled, FTS5) + rusqlite_migration; uuid v7, jiff.
  - Search, diffs, text: ignore/globset/grep-searcher/grep-regex; similar, strsim; encoding_rs,
    chardetng; dunce; strip-ansi-escapes.
  - Windows: windows 0.62 (Job Objects, Toolhelp32, DPAPI); which; portable-pty 0.9.
  - MCP: rmcp `=3.8.0`, behind one module, with stderr piped.
  - Web: dom_smoothie, htmd, dom_query; image (png, jpeg, webp, gif).
  - System: nvml-wrapper, sysinfo; tracing.
  - Rust→TS types: ts-rs.
  - Tests: axum, insta, tempfile, pretty_assertions.
- **npm:**
  - Styling: tailwindcss 4, clsx, tailwind-merge.
  - UI: radix-ui, lucide-react, react-resizable-panels, cmdk.
  - Markdown: streamdown + @streamdown/code + @streamdown/math. Fallback: react-markdown + remark-gfm
    - remark-math + rehype-katex + shiki with the JS engine.
  - Terminal: @xterm/xterm + addon-fit.
  - Lists, PDF, fonts: @tanstack/react-virtual; pdfjs-dist 6.4.299 (worker, `isEvalSupported:false`);
    @fontsource-variable/inter and /jetbrains-mono.
  - Tests: @testing-library/react + user-event.
- **Supply chain:** keep Folio's pnpm policy (exact versions, 3-day minimum release age, trust
  no-downgrade, strict builds).
