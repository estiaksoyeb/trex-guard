# TREX Guard

TREX Guard is a fast terminal safety gate for AI-generated shell commands.

The goal is to intercept pasted command blocks before they reach the shell, analyze them for potentially dangerous operations, and require explicit human approval when appropriate.

## Current status

Phase 1 — PTY foundation: **built, not yet behavior-validated**

The current prototype creates a child shell inside a pseudo-terminal and transparently forwards input/output between the user's terminal and that shell.

No safety scanning exists yet.

## Development principles

- Human remains the execution authority.
- Pasted commands must be inspectable before execution.
- Normal interactive shell behavior should remain fast and unobtrusive.
- Existing Bash and Ble.sh setups should remain usable.
- Safety checks must fail toward review rather than falsely claiming safety.
- Configuration should eventually be user-controlled.
- The first implementation should avoid modifying the user's existing shell configuration.
