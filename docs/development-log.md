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

