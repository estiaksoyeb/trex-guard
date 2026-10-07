# TREX Guard

TREX Guard is a fast terminal safety gate for AI-generated shell commands.

The goal is to intercept pasted command blocks before they reach the shell, analyze them for potentially dangerous operations, and require explicit human approval when appropriate.

## Current status

Phase 1 — PTY foundation: **behavior-validated for transparent forwarding**

Phase 2 — Paste interception: **done** (hold → approve/cancel → forward)

Phase 3 — Safety scanner: **in progress** (literal-string v0)

The current prototype creates a child shell inside a pseudo-terminal and
transparently forwards input/output between the user's terminal and that
shell. Bracketed pastes are held for review: a header shows the text,
flags known-dangerous patterns (`[DANGER: ...]`), and asks for `y` to
run or any other key to cancel.

The scanner is literal-string only; obfuscated shell forms evade it.

## Development principles

- Human remains the execution authority.
- Pasted commands must be inspectable before execution.
- Normal interactive shell behavior should remain fast and unobtrusive.
- Existing Bash and Ble.sh setups should remain usable.
- Safety checks must fail toward review rather than falsely claiming safety.
- Configuration should eventually be user-controlled.
- The first implementation should avoid modifying the user's existing shell configuration.
