# Development Log

## 2026-10-08 — Initial investigation

### Environment

The project is being developed inside:

/root/Projects/trex-guard

Environment observed:

- Architecture: AArch64
- Compiler: GCC 13.3.0
- CMake: 3.28.3
- Bash: 5.2.21
- tmux: 3.4
- Terminal `$TERM`: `tmux-256color`
- Shell: `/bin/bash`
- Current shell terminal: `/dev/pts/11`

Bash reports:

- enable-bracketed-paste = on
- editing-mode = emacs
- keymap = emacs

The tmux pane reports the same PTY used by the shell.

### Initial hypothesis

The first idea was to implement the safety gate through `.bashrc`/Readline.

Testing showed that Bash exposes the `bracketed-paste-begin` Readline function, but a normal `bind` replacement did not provide the required programmable interception behavior.

A standalone raw terminal probe was also tested. That probe did not reproduce Readline's paste framing, so it was not sufficient evidence for a terminal-level paste implementation.

### Architecture decision

The project will use a C++ PTY-based architecture rather than depending on `.bashrc` hooks.

Reasons:

- independent of the user's shell configuration
- can preserve an interactive child shell
- suitable for a fast native implementation
- allows future configuration and rule-engine features
- does not require replacing Ble.sh
- allows the user to continue using Bash normally

### Phase 1 implementation

Created:

CMakeLists.txt
src/main.cpp

The first prototype uses `forkpty()` and `select()` to create a transparent interactive shell.

Build result:

[100%] Built target trex-guard
exit=0

The prototype has not yet been behavior-tested.

### Next step

Run the transparent PTY prototype and verify:

- prompt behavior
- normal commands
- arrow keys
- Ctrl-C
- Ctrl-D
- terminal colors
- multiline input
- interactive commands
- shell exit status

Only after the transparent PTY behavior is established should paste interception be implemented.

### Paste detection test results (2026-10-08)

Added a temporary observe-only PasteTap state machine in src/main.cpp.
It reads every byte from stdin, tracks the bracketed-paste markers
across chunk boundaries, and logs to /tmp/trex-paste.log.

Test 1 — three-line paste:

    CHUNK 40
    PASTE_START
    PASTE_END bytes=28 newlines=2 text=echo ONE\recho TWO\recho THREE

One read; one START/END pair. newlines=2 because the payload has two
carriage-return separators and no trailing terminator. Correct.

Test 2 — 3000-line paste (seq 1 3000 | sed 's/^/: /' | clip):

    CHUNK 4095   (x5)
    CHUNK 3525
    PASTE_START
    PASTE_END bytes=19893 newlines=3000

Six reads, still exactly one START/END pair; bytes and newlines match
the payload. Confirms cross-chunk state tracking works.

Known constraint: when a command is running, the child shell sends the
paste-mode-off sequence, so pastes at that moment arrive without
bracketed markers. A marker-based detector cannot intercept those.

Next implementation step: gate forwarding. On PASTE_START, stop writing
paste bytes to the master and accumulate them; on PASTE_END, present
the buffer for approval and forward or drop. Remove the temporary
logging once the gate is in place.


### Phase 2 + Phase 3 v0 (2026-10-08)

Implemented paste gating and a literal-string safety scanner.

- PasteTap.feed now returns PasteFeedResult{forward, hold, paste_complete}
  instead of logging. Typed bytes go to forward; paste bytes accumulate
  in hold.
- main() writes forward to the PTY, and on paste_complete prints the held
  text with a line count and an approval prompt. Single-key response:
  y approves, any other key cancels. Approved bytes are written to the
  master; cancelled bytes are dropped.
- Removed log_line, escape_bytes, and all /tmp/trex-paste.log writes.
- Added src/rules.cpp / src/rules.h with trex::classify(): ten literal
  rules, returns Risk::Danger plus a rule name. main() prints
  [DANGER: <rule>] in the header.

Verified:

- Three-line paste held, approved (commands ran), cancelled (nothing ran).
- Benign paste (echo hi) shows no DANGER marker.
- Dangerous paste (rm -rf /tmp/scratch) shows [DANGER: rm -rf].

Known issues / deferrals:

- Scanner is literal-string; no shell parsing. curl/wget rules are broad.
- Approved paste is echoed twice on screen (guard + Bash line discipline).
- After cancel, ble.sh in the outer terminal needs an extra Enter to
  repaint; this is outside the guard.

### Phase 3 scanner refinements & test suite (2026-10-08)

Refined safety scanner rules and added comprehensive test coverage:

- Narrowed `curl` and `wget` rules to actual pipe-to-shell patterns (e.g. `curl ... | sh`, `wget ... | bash`, `curl ... | sudo bash`). Plain curl/wget downloads are no longer falsely flagged as dangerous.
- Added regex rules for destructive `git clean` (`-f`, `-fd`, `-fdx`, `--force`).
- Added regex rules for destructive `git checkout` / `git restore` (`git checkout -- .`, `git checkout -f`, `git restore .`, `--worktree`).
- Added regex rules for `git push --force` (`--force`, `-f`, `--force-with-lease`).
- Added rules for `chmod` broad/root recursive permissions changes (`chmod -R 777 /`, `chmod -R 000`, etc.).
- Added rules for `find ... -delete` and `find ... -exec rm`.
- Added rules for block device overwrites (`> /dev/sd*`, `> /dev/nvme*`, etc.).
- Created `tests/test_rules.cpp` with 43 test cases covering both safe and dangerous command variations.
- Integrated `test_rules` target and CTest in `CMakeLists.txt`. All 43 test assertions pass cleanly.

### Phase 4 Human approval UI (2026-10-08)

Implemented the full interactive human approval UI:

- **Tiered Risk Classification**: Added `Risk::Safe`, `Risk::Review`, and `Risk::Danger`. High-risk commands (e.g. `rm -rf`, raw disk writes, mkfs, pipe-to-shell, fork bomb) take precedence, while moderate repository-destructive actions (`git clean`, `git reset --hard`, `git checkout -f`, `git push --force`) trigger Review.
- **Exact Snippet and Explanation**: `trex::classify()` now populates `m.snippet` with the exact matched command substring (extracted via `std::smatch`) and `m.explanation` with a concise risk explanation.
- **Terminal UI**: Banners format risk tier in distinct colors/headers (`[HIGH RISK]`, `[REVIEW REQUIRED]`, `[SAFE]`), display the matched snippet and risk rationale, format paste contents, and prompt the operator.
- **Approval Keybindings**: Operator can approve with <kbd>Enter</kbd> (`\r`/`\n`) or `y`/`Y`, or cancel with <kbd>Ctrl-C</kbd> (`\x03`), <kbd>Esc</kbd> (`\x1b`), or `n`/`N`. Cancelled pastes are dropped immediately without execution.
- **Tests**: Expanded unit tests to 44 assertions testing tier classification, snippet presence, explanation presence, and danger-over-review precedence.

### Phase 5 Configuration Subsystem (2026-10-08)

Implemented the configuration subsystem (`src/config.h`, `src/config.cpp`):

- **Location**: Default path loaded from `$XDG_CONFIG_HOME/trex-guard/config.ini` or `~/.config/trex-guard/config.ini`. If absent, defaults are safely used without errors.
- **Sections**:
  - `[general]`: `auto_approve_safe = true/false` allows automated forwarding of safe pastes without prompting if desired by the operator.
  - `[allowlist]`: Substrings / commands to explicitly treat as Safe, bypassing rule matches (e.g. internal scripts or known force pushes).
  - `[disabled_rules]`: Disables specified built-in rules by name.
  - `[custom_rules]`: Allows user-defined rules in `name | risk | pattern | explanation` format.
- **Reference Example**: Added `examples/config.ini` for user documentation.
- **Verification**: Added configuration unit tests in `tests/test_rules.cpp` validating parsing, allowlists, disabled rules, and custom rules across tiers. Tests pass 49/49.

### Phase 6 Installation & Integration (2026-10-08)

Implemented installation targets, launcher wrapper, and shell integration:

- **CLI flags**: Updated `src/main.cpp` to support `--help` (`-h`) and `--version` (`-v`), avoiding shell launches when querying information.
- **Environment marker**: Child processes now export `TREX_GUARD_ACTIVE=1` so child shells and tools can detect the active guard session.
- **Guarded-shell launcher**: Added `scripts/tg`, a shell wrapper that detects existing guard sessions, supports `--help` and `--version`, and executes `trex-guard`.
- **Shell integration**: Added `scripts/shell-integration.bash` providing `alias tg='trex-guard'` and `trex_guard_indicator()` for shell prompts.
- **CMake install & uninstall**: Added `install()` targets for binary, launcher script, config template, and shell integration script using `GNUInstallDirs`. Added custom `uninstall` target via `cmake/cmake_uninstall.cmake.in`.
- **Documentation**: Created `docs/install.md` detailing build, install, shell integration, configuration, and uninstallation steps.

### Phase 1 Hardening: Signal Handling & Terminal Restoration (2026-10-08)

Hardened terminal restoration and signal safety in `src/main.cpp`:

- **Atexit Cleanup**: Registered `std::atexit(restore_terminal)` to guarantee that whenever the program exits normally, the original `termios` configuration is restored to the outer terminal.
- **Signal Handlers**: Installed signal handlers for `SIGTERM`, `SIGHUP`, `SIGINT`, and `SIGQUIT` that invoke `restore_terminal()`, forward the signal to the child PTY, reset the signal handler to `SIG_DFL`, and re-raise. This prevents the parent terminal from remaining stuck in raw mode (`echo` disabled, staircase newlines) if the guard process is unexpectedly killed or terminated.
- **State Guard**: Added atomic flag `g_terminal_modified` ensuring `tcsetattr` is only called if raw mode was actually engaged.

### Phase 3 Advanced Rules: Shell Eval, Heredocs & Script Execution (2026-10-08)

Added detection rules for indirect command execution and script injection:

- **Shell Eval (`Risk::Danger`)**: Detects `eval $(...)`, `eval "$..."`, and dynamic string evaluations that evade direct static token analysis.
- **Encoded Shell Pipes (`Risk::Danger`)**: Detects `base64 -d | sh` and variants that pipe decoded binary/text payloads directly into a shell interpreter.
- **Heredocs to Shell (`Risk::Danger`)**: Detects inline heredoc payloads executed by piping or redirecting into a shell interpreter (`bash << 'EOF'`, `<< 'EOF' | sh`).
- **Inline Script Execution (`Risk::Danger`)**: Detects Python (`python -c ... os.system(...)`), Perl, and Ruby one-liners that invoke subprocesses or execute system commands. Innocent scripts (e.g. `python3 -c "print('hi')"`) remain `[SAFE]`.
- **Subshell Execution (`Risk::Review`)**: Flags `bash -c "..."`, `sh -c "..."`, and `sudo sh -c "..."` for operator review.
- **Verification**: Added 10 new test assertions in `tests/test_rules.cpp`. Total test suite passes at 59/59 (100%).

### Phase 2 ble.sh Repaint Polish (2026-10-08)

Resolved prompt repaint friction on paste cancellation:

- **Root Cause**: When a paste was cancelled, the guard consumed the cancellation key and discarded the paste without sending anything to the child master PTY. As a result, line editors (such as `ble.sh` or Readline) remained waiting at their previous input offset and required an extra <kbd>Enter</kbd> from the user to trigger a prompt redraw.
- **Resolution**: Upon paste cancellation in `src/main.cpp`, the guard sends an interrupt character (`\x03` / Ctrl-C) to the child master PTY. This instructs the child's line discipline and line editor to reset the input line buffer and cleanly reprint a fresh prompt on the terminal immediately.

### Auto-Approval of Safe Pastes by Default (2026-10-08)

Refined firewall gating behavior to eliminate friction for safe commands:

- **Rationale**: Requiring human confirmation on harmless commands (e.g. `ls`, `git status`, `python script.py`) creates cognitive fatigue. A transparent firewall should pass safe commands through immediately, reserving confirmation only for commands classified as `Review` or `High Risk / Danger`.
- **Change**: Updated default `auto_approve_safe = true` in `Config`. When a paste contains no risky patterns, it passes straight through to the shell.
- **Configurability**: Users desiring strict manual verification of all pastes can set `auto_approve_safe = false` in `~/.config/trex-guard/config.ini`.
- **Verification**: Updated test suite asserting default `auto_approve_safe = true` and configurable override. 61/61 tests pass.








