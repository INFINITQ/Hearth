# 0002. One app-managed llama-server process per loaded model

Status: accepted (2026-11-02)

## Context

llama-server can serve one model per process, or run as a "router" (`--models-dir`,
`--models-preset`, `--models-max`) that loads models on demand. With 32 GB of RAM only one of the
recommended 13–22 GB models fits at a time anyway.

## Decision

Hearth starts one `llama-server` process per loaded model, on `127.0.0.1` with a free port and a
random `--api-key`, with flags built from the model's preset (`-c` always explicit, `-np 1`,
`--fit` placement unless overridden). By default at most one model is loaded; asking for another
waits for any run on the current model, then swaps. Each process lives in a Windows Job Object with
kill-on-close, so it never outlives Hearth.

## Consequences

- Behaviour is predictable: Hearth knows exactly which flags each model got, and a crash or
  out-of-memory only affects that model, with its log tail shown.
- Switching models restarts a process (10–30 s for a large model), which the UI shows.
- Changing a preset restarts that model; there is no hot reload.
- Router mode stays an option for later, if multi-model setups on bigger machines need it.
