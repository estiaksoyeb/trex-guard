# TREX Guard

TREX Guard is a fast terminal safety gate for AI-generated shell commands.

The goal is to intercept pasted command blocks before they reach the shell, analyze them for potentially dangerous operations, and require explicit human approval when appropriate.

## Current status

Phase 1 — PTY foundation: **done** (transparent forwarding, resize propagation, signal-safe terminal cleanup)

Phase 2 — Paste interception: **done** (hold → approve/cancel → forward)

Phase 3 — Safety scanner: **in progress** (pattern scanner with test suite)

Phase 4 — Human approval UI: **done** (tiered risk, snippet display, explanations, Enter/Ctrl-C keys)

Phase 5 — Configuration: **done** (custom rules, allowlists, disabled rules, auto_approve_safe)

Phase 6 — Installation/integration: **done** (CMake install, tg launcher, shell integration)

The current prototype creates a child shell inside a pseudo-terminal and
transparently forwards input/output between the user's terminal and that
shell. Bracketed pastes are analyzed for safety:
- **`[SAFE]` pastes**: Pass through immediately and execute transparently without prompting.
- **`[REVIEW REQUIRED]` & `[HIGH RISK]` pastes**: Intercepted and held for operator review.
  A structured banner highlights the suspicious command snippet and risk rationale,
  and prompts the user for approval (<kbd>Enter</kbd>/`y`) or cancellation (<kbd>Ctrl-C</kbd>/`n`).

The scanner detects dangerous command patterns (destructive git commands,
recursive deletions, pipe-to-shell downloads, block device writes, mkfs, fork bombs)
and is verified by an automated test suite. Obfuscated shell forms can still evade it
prior to a full AST parser.

## Development principles

- Human remains the execution authority.
- Pasted commands must be inspectable before execution.
- Normal interactive shell behavior should remain fast and unobtrusive.
- Existing Bash and Ble.sh setups should remain usable.
- Safety checks must fail toward review rather than falsely claiming safety.
- Configuration should eventually be user-controlled.
- The first implementation should avoid modifying the user's existing shell configuration.
