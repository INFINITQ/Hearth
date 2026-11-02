# 0005. Terminals use ConPTY directly, not portable-pty

Status: accepted (2026-11-03)

## Context

The terminal panel runs real shells in pseudo consoles. portable-pty (from WezTerm) wraps ConPTY,
but starts the shell running: Hearth could only put it in a Job Object afterwards, leaving a
moment in which anything the shell starts escapes the job. Every other child process in Hearth
starts suspended, joins its job, then runs (`proc.rs`), so Stop and closing Hearth end the whole
tree.

## Decision

`hearth-core/src/terminal/pty.rs` creates the pseudo console with the `windows` crate (already a
dependency), starts the shell with `CREATE_SUSPENDED` and the pseudo-console attribute, assigns
it to a new Job Object, then resumes it. Output is read on a thread and decoded as a UTF-8
stream; keystrokes go through a writer thread so a busy shell never blocks a command.

## Consequences

- Closing a terminal tab (or Hearth) ends the shell and everything started in it, with no gap.
- About 300 lines of `unsafe` Windows code to maintain, covered by tests that run real
  `cmd.exe` sessions (typing, resizing, exit codes, closing a running program).
- No extra dependency.
