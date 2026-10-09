# TREX Guard (`tg`)

TREX Guard is a transparent, high-performance pseudo-terminal (PTY) safety firewall for clipboard-pasted shell commands.

It intercepts multi-line pasted commands before they reach the shell, analyzes them for destructive or dangerous operations, and requires explicit operator confirmation when appropriate.

---

## Features

- **Transparent Zero-Friction Protection**: Harmless commands (`git status`, harmless loops, builds) execute immediately without prompts when `auto_approve_safe = true`.
- **Bottom-Positioned Review Panel**: The paste preview is displayed first, followed immediately by security warnings right above the prompt. On 50+ line scripts, you never need to scroll up to inspect risks.
- **Multi-Threat Reporting**: Detects and enumerates *all* risky commands in a paste, ranked by severity (`High Risk / Danger` before `Review`).
- **Full Shell & `ble.sh` Compatibility**: Re-wraps approved pastes in bracketed-paste markers, eliminating pure-Bash character decode lag (`██▌ processing input...`).
- **Mobile & Termux Resilient**: Safely handles virtual keyboard unfocus events and terminal resizes without dropping or cancelling the prompt.
- **Configurable**: Define custom regex rules, allowlists, and toggle built-in rules via `~/.config/trex-guard/config.ini`.

---

## Current Status

- **Phase 1 — PTY Foundation**: **done** (transparent forwarding, dynamic SIGWINCH forwarding, signal-safe terminal restoration).
- **Phase 2 — Paste Interception**: **done** (bracketed paste gating, hold/forward architecture).
- **Phase 3 — Safety Scanner**: **done** (tiered scanner covering `rm -rf`, file deletions, `mkfs`, `dd`, `curl | sh`, destructive git actions, `eval`, `base64 | sh`, fork bombs; 70 automated tests).
- **Phase 4 — Human Approval UI**: **done** (bottom review layout, multi-finding enumeration, compact `Execute? [Enter/^C]` prompt).
- **Phase 5 — Configuration**: **done** (XDG `config.ini`, custom regex rules, allowlist, rule disabling, auto-approval toggle).
- **Phase 6 — Installation & Integration**: **done** (CMake install, `tg` launcher wrapper, shell auto-launch).

---

## Quick Start

### 1. Build & Test
```bash
cmake -B build
cmake --build build
./build/test_rules
```

### 2. Install Globally
```bash
# Installs tg and trex-guard into /usr/local/bin
sudo cmake --install build
# or user-local (~/.local/bin):
cmake --install build --prefix ~/.local
```

### 3. Launch
From any directory, simply type:
```bash
tg
```

To auto-launch `tg` on every terminal session, add to your `~/.bashrc`:
```bash
[ -z "$TREX_GUARD_ACTIVE" ] && [ -t 0 ] && exec tg
```

---

## Documentation

- [Configuration & Custom Rules Guide](docs/configuration.md)
- [Installation and Shell Integration](docs/install.md)
- [Architecture & Threat Model](docs/architecture.md)
- [Development Log & Changelog](docs/development-log.md)
- [Project Roadmap](docs/roadmap.md)

---

## Development Principles

- Human remains the execution authority.
- Pasted commands must be inspectable before execution.
- Normal interactive shell behavior should remain fast and unobtrusive.
- Existing Bash and Ble.sh setups must operate seamlessly without lag.
- Safety checks must fail toward review rather than falsely claiming safety.

