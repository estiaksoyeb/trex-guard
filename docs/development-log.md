# Development Log

## 2026-11-17 — `tgr`: the guard, ported to Rust, beside the original

The C++ guard works and stays installed. `tgr` was built as a **parallel**
binary so it can be compared against the real thing before anyone depends on
it, and dropped by not typing it if it does not earn daily use.

The port was made by reading all 1858 lines of `src/`, not from memory. The
rule was to match `tg` exactly, quirks included, and to record every deliberate
difference in `docs/tgr.md` — five so far, all in the cautious direction except
the terminal-size one.

### The acceptance criteria are the C++ binary, not my reading of it

88 cases from `tests/test_rules.cpp` were ported verbatim
(`tests/cpp_suite.rs`). That passes, but it only proves `tgr` matches *my
reading* of the suite. So two parity tests drive the **real** `./build/trex-guard`
through a pseudo-terminal and require the same verdict: one with the built-in
rules, one with the user's real `config.ini` (21 custom rules).

`tg` cannot be driven through a pipe — it calls `tcgetattr` on stdin and exits
with `Inappropriate ioctl for device` — so both use
`tests/support/drive_cpp.py`, which forks a pty and reads the review banner.
Each paste starts a real shell, so a parity run takes about three minutes.

That test paid for itself immediately: it flagged five pastes where the C++
guard said Danger/Review and `tgr` said Safe. The cause was the test, not the
code — the custom-rule pastes had been added to the built-in-rules corpus,
which runs with no config while the C++ side reads the real one. The corpora are
now separate (`PASTES` and `CUSTOM_RULE_PASTES`), which is also the reason the
second test exists.

### What the end-to-end tests check that nothing else can

`tests/pty_e2e.rs` drives `tgr` itself and proves the things a classifier test
cannot: an approved `Review` paste really shreds its target, a cancelled one
leaves it intact, a safe paste is not prompted, and the guarded shell sees
`TGR_ACTIVE=1` while `TREX_GUARD_ACTIVE` is untouched. That last one is what
stops `scripts/tg` from believing a `tg` session is active inside a `tgr` one.

Three harness bugs were found and fixed while writing those tests, each of
which had made a test look like a product failure: Rust syntax (`Some(…)`) in a
generated Python script; a driver that re-sent the paste after the shell
returned to its prompt; and a marker check that read the file before the shell
had written it. The fourth was a genuine misunderstanding: `touch` is `Safe` in
both guards — neither has a rule for it — so a test that expected a prompt for
`touch` was testing a rule that does not exist.

### The pty driver was racing the shell

Running the installed binary by hand showed every paste echoed raw, with the
bracketed-paste markers intact — the guard looked as if it had stopped
classifying. It had not: the driver wrote the paste before bash had finished
starting, so the terminal buffered it and bash read it as ordinary typing.
The driver now waits for the shell's prompt before sending anything. The
end-to-end tests had been passing on that race, which is the useful lesson:
they were green, but not for the reason claimed.

With the race fixed, the suite still passes — and one test was added for the
property the separate marker exists to protect: the real installed `tg` starts
inside a real `tgr` session, rather than printing its "already active" notice.

### Interactive use is the remaining test

Tests prove the classifier and the prompt. They cannot prove it feels right in
a real terminal with the user's own `~/.bashrc`, which here execs into tmux.
Daily use is the test for that.

## 2026-11-16 — `docs/onboarding.md`: one place to understand the project

The knowledge needed to work on this repository was spread across
`docs/workflow.md` (the loop and the mistakes), `docs/tcx.md` (the design
record) and the code. A fresh session had to read all three and reconcile them,
and the design record already had stale items left behind by earlier decisions.

`docs/onboarding.md` now answers, in order: what the two binaries are, what the
human's job is, the two per-chat commands, where context lives, what belongs in
a note, what `map` generated versus stored, why each architectural split exists,
and every mistake already made with the rule it produced. It ends with a table
of where to read next.

`docs/workflow.md` is now a short pointer rather than a second copy. Two copies
of the same rules is the drift failure catalogued in `docs/onboarding.md` §9.10,
and this changelog records a previous instance of it.

Stale items removed while consolidating:

- `docs/tcx.md` §15 still listed the rejected 5-session trial as "the next
  thing to do"; it is now marked settled, with a pointer to why.
- §13's cut list still contained `map`, which shipped.
- §16 said "92 tests"; the count is no longer restated there.
- a live store note told every new session the trial was an open question. A
  superseding note now records that it was rejected (the original is kept:
  notes are append-only).

The `resume` output is now the practical check that the documentation is good
enough: if a new session still needs a file listing to get oriented, the
project section or the notes are the problem, not the session.

### The doc was then tested against a fresh reader

A subagent was given only `docs/onboarding.md` and asked what it understood.
That found real defects, not style issues:

- the template fallback was documented as `~/.config/tcx/prompt.md`; the code
  actually tries `<config>/prompt.md` then `<config>/tg/prompt.md`, where
  `<config>` already ends in `tcx`. `tcx fresh --help` had the same wrong text.
- `note --dir` worked but was absent from `tcx note --help`, so the flag `map`
  depends on was invisible. The same was true of `--path` on `prime` and
  `import`.
- the help tests claimed to check both directions; they only checked
  documented → accepted. `every_flag_the_parser_accepts_is_documented` now
  reads the parser arms out of `main.rs` and requires each flag in the table.
  Verified by deleting `--dir` from the table and watching it fail.
- the doc said both per-chat commands run once per chat, and never said
  whether the AI may edit its own prompt (it may not; the rules are the
  human's), or when to ask for `prime` over `resume`.

Two flags remain deliberately overloaded — `-k` on `note`/`find` means
`--kind`, and `-d` means `--dir` on `note` but `--depth` on `map`. Both are now
documented in `--help` and in the onboarding doc, and both are spelled long in
examples.

### A second cold read, and one more code bug

The corrected doc was given to a second fresh reader, alongside the code. It
confirmed the substantive claims and found one more defect in the binary:

- `tcx map --under <missing>` printed `… is not a directory` **to stdout and
  exited 0**, so a typo was indistinguishable from a successful empty map. It
  is now stderr + exit 1, and `map::render` returns `Result` to make that
  impossible to reintroduce by accident.
- the store layout under `TCX_HOME` has **no** `tcx` suffix; only the XDG
  layout does. The doc had the rule backwards, contradicting its own example,
  and `briefing.rs`'s module doc listed the wrong template fallbacks.

Doc corrections from the same read: `fresh --help` documents the fallback
*order*, not the resolved path (the resolved path goes to stderr at runtime);
`TCX_HOME` is only set in a shell that sourced the integration file; the
budget skips whole notes rather than truncating them; `import`'s positional
argument is the source directory, not the project; and the trailing-path guard
on `note` only catches paths containing `/`, not bare directory names.

## 2026-11-16 — `tcx map`

Answers "where does everything live" without a file dump, split so neither half
can invalidate the other:

- the tree is **walked live from disk** at render time, so a moved file breaks
  nothing;
- the judgment is **notes with a `dir:` field**, written by the AI as it learns,
  and dated.

`--depth N` (default 2) and `--under PATH` control the output. Data-only
directories (schema dumps, migrations) collapse to `(N data files)`, generated
and vendored trees are skipped, and judgment on a directory that no longer
exists is printed under `## no longer in the tree` rather than dropped.

`--under` initially failed for deep paths, because depth was measured from the
root and the subtree was looked for inside an already-shallow tree. Depth now
counts from the node being mapped.

While building it, `tcx note "text" .` was found to store the `.` as note text:
`note` is the only command with no positional PATH. A trailing `.`, `..` or
existing path is now rejected with exit 2.

## 2026-11-16 — dropped `tcx-trial`

The tally helper for the rejected 5-session trial is gone: `scripts/tcx-trial`,
its CMake install line, and the "Running the trial" section of `docs/tcx.md`.
It installed a third command for a purpose the user had already refused, and
left a dead entry point on every machine.

`docs/workflow.md` keeps the lesson and now says explicitly not to rebuild it.

Entry points are now two: `tg` (guard) and `tcx` (context).

## 2026-11-16 — per-command help, and a test that catches help drift

`tcx <command> --help` did not exist: every command answered `--help` with
"unknown option" and exit 2, and the only help was one hand-written block.
`--path` was undocumented on several commands and the defaults (6000 bytes,
`-k decision`) were never stated.

Help is now rendered from a single table in `src/help.rs`, used by both the
summary and each command. `--help` is answered before a command parses, so it
cannot be swallowed as an option value. `scripts/tcx`, which carried its own
shorter and stale copy, now delegates.

`tests/help.rs` probes the real binary so the documentation cannot drift: it
reads the dispatcher's command list out of `main.rs`, rejects flags the parser
does not accept, and fails if a dispatcher arm is missing from the table. All
three were verified by deliberately breaking the code.

## 2026-11-16 — `tcx find -k`

Rule 18 of the workflow prompt instructs the AI to read notes with
`tcx find -k <keyword>`, but `find` had no `-k` flag: every scoped lookup would
have exited 2. The flag now filters by keyword (repeatable, case-insensitive)
before the free-text query. This is the failure mode to watch for in general:
the prompt is configuration, and configuration that names a command must be
checked against the binary.

## 2026-11-16 — config-only prompt, no clipboard coupling

Two corrections to the `fresh`/`resume` slice.

- **The prompt is read only from the config directory** (`$TCX_HOME/config/prompt.md`
  or `~/.config/tcx/prompt.md`), never from the repository. The earlier order let
  a checkout's `.tg/prompt.md` override the user's rules, which is the wrong
  direction of trust. The shipped `.tg/prompt.md` was removed.
- **Nothing is hardcoded to `clip`.** `tg fresh` and `tg resume` print to
  stdout; `tg fresh | clip` is the user's choice at the shell. Confirmed by
  grepping the crate: the only occurrence was a comment.

## 2026-11-16 — `tg fresh` and `tg resume`

The two commands that start and resume a chat now exist, as `tcx fresh` and
`tcx resume`, reached through `tg`. The guard never starts a shell for them;
`scripts/tg` forwards them before the recursive-wrap check.

- Prompt text is a file, not a binary constant. (Initially looked up in the
  repo first; corrected the same day to config-only - see the entry above.)
  Editing rules never needs a rebuild.
- `## Project` is filled by **reading** manifests (Cargo, Gradle, CMake, npm,
  pyproject, go.mod) - no build system is executed. Verified against `mpvRex`,
  which yields `xyz.mpv.rex`, `app/src/main/kotlin/xyz/mpv/rex/`,
  `Gradle (Kotlin DSL)`, `Android app, Kotlin + Jetpack Compose`.
- `resume` appends `## What previous sessions learned` using `prime`'s shared
  ordering, sanitiser and byte budget, so the two never disagree.
- 113 tests. Two bugs came only from running against the real tree, not a
  fixture: `root.join(":app")` resolves to `/app` (leading colon must be
  stripped), and the heading search started after the heading so the section
  body was deleted.

## 2026-11-16 — TREX ConteXt (`tcx`): what was built, and what was misunderstood

### Built

A separate Rust crate (`crates/tcx`) that stores a project's judgment notes
outside the repository. `tg` is untouched. Commands: `id`, `note`, `prime`,
`find`, `import`. 92 tests.

### The misunderstanding that cost the most

`tcx note` was designed and explained as something the **human** types after a
session. That is wrong. In this workflow the AI writes the command inside the
bash block the human is already pasting, so the human never composes it.

The human's only job is: run the block, paste the output back. Anything that
assumes otherwise is a design error.

See `docs/workflow.md`, which now states this up front.

### Also wrong

- Proposed a 5-session counting trial to prove `prime` helps. Busywork: daily
  use answers it in a week for free. Don't invent experiments.
- Answered "how do I test this" with procedure instead of building the next
  command.
- Asked questions the pasted workflow prompt already answered.

### What follows

The product is two commands run once per chat: `tg fresh` (prompt with a filled
`## Project` section) and `tg resume` (same, plus what the last session learned).
Prompt text belongs in a file, not in the binary.

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

### UI Polish: Streamlined Prompt & Header UX (2026-10-08)

Redesigned the human approval prompt and risk banners for compact readability:

- **Prompt**: Replaced the verbose `--- press Enter (or y) to approve, Ctrl-C (or Esc/n) to cancel: ` with a clean, standard CLI prompt: `Execute? [Enter/^C] `.
- **Header**: Compacted banner labels and byte/line indicators (e.g. `[HIGH RISK: rm -rf] (35B, 2L)`).
- **Feedback**: Simplified cancellation feedback to `[cancelled]`.

### Shell Argument Forwarding & Non-Login Child Spawning (2026-10-08)

Resolved ble.sh auto-launch issue when running under `exec bash --norc`:

- **Root Cause**: Previously, `trex-guard` hardcoded `execl(shell, shell, "--login", nullptr)`. A login shell always forces execution of `/etc/profile` and `~/.bashrc`, which in turn initialized `ble.sh` even if the user ran `exec bash --norc` in their outer terminal.
- **Removed Forced `--login`**: Default child shell is now launched as an interactive shell without forced login file reloading.
- **Argument Forwarding**: `tg` and `trex-guard` now accept arbitrary shell arguments (e.g. `tg --norc`, `tg bash --norc`, `tg zsh`).
- **Parent Environment Inheritance**: Implemented `ancestor_has_norc()` in `src/main.cpp` traversing `/proc/<pid>/stat` parent links. Inside the child fork, `getppid()` points to the guard proxy itself; traversing up the ancestor process chain reaches the outer interactive shell. If any ancestor shell was launched with `--norc`, the child shell automatically inherits `--norc`, keeping `ble.sh` disabled.
- **Verification**: Verified in automated PTY integration tests that launching `tg` from an `exec bash --norc` session runs a clean Bash shell without ble.sh (`CLEAN_BASH_NO_BLE`).

### Mobile UX: Keyboard Unfocus & Terminal Focus Reporting (2026-10-08)

Fixed accidental paste cancellation when unfocusing the keyboard on mobile/Termux:

- **Root Causes**:
  1. Unfocusing or dismissing the virtual keyboard in Termux/Android changes the terminal window height, triggering `SIGWINCH`. In `src/main.cpp`, `read(STDIN_FILENO, &c, 1)` failed with `-1` and `errno == EINTR`, which the naive approval loop treated as a cancellation (`approved = false`).
  2. Terminal emulators with focus tracking emit FocusOut sequences (`\x1b[O`) when the soft keyboard is hidden or when switching apps. Naive single-byte reading ingested `\x1b` and treated it as non-Enter / cancel.
- **Solution (`read_approval_decision`)**:
  - Automatically handles `EINTR`, updates the child PTY window geometry via `TIOCSWINSZ` without exiting, and loops back to wait for user input.
  - Ignores focus event sequences (`\x1b[I` FocusIn, `\x1b[O` FocusOut).
  - Ignores escape prefixes from touch gestures or arrow keys, while preserving standalone `Esc` or `Ctrl-C` (`\x03`) as explicit cancellations.
  - Only executes upon explicit confirmation (<kbd>Enter</kbd>, `y`, `Y`).

### Safety Scanner: File Deletion Detection & Git Checkout Coverage (2026-10-08)

Resolved silent auto-approval flaw for unprompted file deletions (`rm`, `rmdir`, `unlink`, `shred`):

- **Root Cause**: Previously, the scanner only classified *recursive force* deletions (`rm -rf`) as dangerous, while ordinary file removals (`rm README.md`, `rm file.txt`) were explicitly marked `Safe`. Under `auto_approve_safe = true`, a multi-line paste containing benign build commands mixed with `rm <file>` executed without confirmation, permanently deleting files.
- **Rule Fix**:
  - Added `file deletion` rule under `Risk::Review` matching `rm`, `rmdir`, `unlink`, and `shred` commands with arguments.
  - Excluded flags like `docker run --rm` or words containing `rm` (`confirm`, `perform`, `format`).
  - Added support for `git checkout .` (without `--`) to catch working-tree discards.
  - Trimmed leading punctuation/whitespace from matched regex snippets.
- **Verification**: Added 7 test cases covering `rm`, `unlink`, `shred`, `git checkout .`, and safe `--rm` flags. 68/68 automated tests pass.

### PTY & Terminal UX: Bracketed Paste Preservation (`ble.sh` Zero-Lag) (2026-10-08)

Resolved severe input processing lag (`██▌ processing input...`) in `ble.sh` and enhanced cross-shell paste execution:

- **Root Cause**: Modern terminals wrap pasted text in `\x1b[200~` and `\x1b[201~`. `trex-guard` stripped these markers to scan the command, and previously forwarded raw text to the child PTY. `ble.sh` consequently saw hundreds or thousands of unbracketed keystrokes and was forced to parse, tokenize, and evaluate each character individually in pure Bash script.
- **Solution (`forward_paste_payload`)**:
  - Re-wraps forwarded payload in bracketed paste markers (`\x1b[200~` + data + `\x1b[201~`), triggering `ble.sh`'s native batch-insert mode for instantaneous rendering without character-by-character decode lag.
  - Automatically appends POSIX line acceptance (`\n`) for approved prompts and pastes with trailing newlines, ensuring immediate command execution across both `ble.sh` and GNU Readline (`bash --norc`).
- **Verification**: End-to-end PTY integration tests verified that both single-line and multi-line pastes execute immediately in `ble.sh` and `bash --norc` with `processing input` eliminated.

### UI & Scanner: Bottom-Positioned Review Section & Multi-Finding Reporting (2026-10-09)

Redesigned the approval UI layout and upgraded the scanner to report all findings:

- **Problem (User Handwritten Feedback)**:
  1. *Scroll-off on large pastes*: The review banner was displayed *above* the paste block. When pasting long multi-line scripts (30–50+ lines), the flags scrolled far off the top of the terminal, forcing the user to scroll up to understand why execution was blocked.
  2. *Single-match blindness*: The scanner stopped at the first match. If a paste contained several dangerous commands (e.g. `rm -rf`, `rm file`, `curl | bash`), only one was shown in the banner.
- **Solution**:
  1. *Layout Reordering*: Reordered the terminal display so the `[Pasted Content]` preview prints first, followed by `--- [Security Review] ---` directly above the `Execute? [Enter/^C]` prompt. Security flags are always immediately visible on screen right where the decision is made without scrolling.
  2. *Multi-Finding Scanner*: `classify()` now iterates across all literal, regex, and custom rules using `std::sregex_iterator` to collect all matched threats into `m.matches`, ranked by severity (`Danger` before `Review`) and deduplicated.
  3. *Comprehensive Findings List*: When multiple threats exist, the review section enumerates every finding with its rule name, matched command snippet, and risk explanation.
- **Verification**: Added unit tests for multi-match detection, severity ranking, and deduplication (70/70 tests pass). Verified in end-to-end PTY sessions.

### UI Polish: Configurable Compact Review Mode (`compact_review`) (2026-10-09)

Added support for suppressing verbose multi-line risk explanations:

- **Motivation**: Users frequently want the prompt gating protection and the rule name tag, but find the explanatory text (`Risk: Permanently deletes files...`) redundant on everyday operations.
- **Implementation**:
  - Added `show_risk_explanation = true` (default) to `Config`.
  - Parsed `compact_review = true` (or `show_risk_explanation = false`) in `[general]` section of `config.ini`.
  - In `src/main.cpp`, each finding conditionally renders on a clean single line when `compact_review` is enabled.
- **Verification**: Added unit test assertions in `tests/test_rules.cpp` (72/72 tests pass). Verified in simulated PTY that `compact_review = true` outputs a single line per finding without risk explanations.

### Scanner Evolution: Semantic Command Pattern Recognition (2026-10-09)

Replaced brittle, monolithic regular expressions with a structured shell AST parser and semantic pattern recognition engine:

- **Problem**:
  1. *Combinatorial Regex Explosion*: Commands like `git reset --hard` were missed when arguments were reordered (e.g. `git reset HEAD~1 --hard`), when global flags were used (`git -C repo reset --hard`, `git --no-pager reset --hard`), or when commands were wrapped (`sudo git reset --hard`).
  2. *Split Flags & Option Positioning*: Commands like `rm /path -r -f` or `rm -v -f -r` required complex, brittle regex permutations that still leaked unhandled patterns.
- **Solution**:
  1. *Zero-Dependency Shell Parser (`src/shell_parser.h`, `src/shell_parser.cpp`)*:
     - Tokenizes commands with proper quote handling (`'...'`, `"..."`), escape sequences, statement separators (`;`, `&&`, `||`, `&`, newlines), and pipeline tracking (`|`).
     - Normalizes executables across path prefixes (`/usr/bin/git` -> `git`) and unwraps wrappers (`sudo`, `doas`, `env`, `nice`, `nohup`).
  2. *Semantic Command Analyzers (`src/rules.cpp`)*:
     - **Git Engine**: Skips global flags, identifies Git subcommands (`reset`, `push`, `clean`, `checkout`, `restore`), and detects destructive arguments regardless of flag order or target commit position.
     - **File Deletion Engine**: Inspects argument flags on `rm` for recursive and force combinations across any argument position, correctly identifying `rm -rf` vs single-file deletion.
     - **Pipeline Correlation**: Correlates upstream download utilities (`curl`, `wget`) directly with downstream shell interpreters (`bash`, `sh`, `zsh`).
     - **Chmod Engine**: Semantically detects recursive flags combined with destructive permissions masks or root paths.
- **Verification**: Added extensive unit tests covering Git reset permutations, global flags, argument reordering, split `rm` flags, and wrapper prefixes (91/91 tests pass).

### Terminal Hardening: Persistent Bracketed Paste & Unbracketed Burst Detection (2026-10-09)

Fixed paste leakage occurring when pasting outside an idle Readline prompt or in terminals sending unbracketed clipboard payloads:

- **Root Cause**:
  1. *Child Shell Mode Disabling*: Whenever a command or child script executes, Readline turns off bracketed paste mode (`\x1b[?2004l`). When pasted at that transition, the terminal sends raw keystrokes without `\x1b[200~` markers.
  2. *PasteTap Blindness*: `PasteTap` previously assumed all pastes arrive with `\x1b[200~`. Raw unbracketed input was forwarded directly to the shell as manual typing.
- **Solution**:
  1. *Persistent Outer Bracketed Mode*: `trex-guard` now explicitly emits `\x1b[?2004h` to `STDOUT_FILENO` on raw mode startup (and restores with `\x1b[?2004l` on exit), and suppresses child attempts to disable it (`\x1b[?2004l`) in master output.
  2. *Unbracketed Multi-Line & Burst Detection*: Stdin reader detects unbracketed input containing newlines (`\n` or `\r` with size > 1) or large bursts (`size >= 8`), drains the burst using a 15ms `select` timeout, and routes the accumulated payload directly into `classify()` and the security review prompt.
- **Verification**: Verified via automated PTY tests that unbracketed multi-line pastes containing `git reset HEAD~1 --hard` trigger the Security Review prompt, while normal single-character typing continues without delay.

### Parser Hardening: Terminal Paste Carriage Return (`\r`) in Comment Lines (2026-10-09)

Fixed scanner bypass where scripts starting with `#` comments caused subsequent commands to be ignored:

- **Root Cause**: Terminal pastes in raw PTY mode deliver line breaks as carriage returns (`\r`) or CRLF (`\r\n`). In `src/shell_parser.cpp`, the `in_comment` state transition checked only for `\n`. When a comment ended in `\r`, `in_comment` remained true for all subsequent lines in the paste buffer, consuming destructive commands (`rm -rf`, `curl | bash`, `git push --force`) as comment text. The paste was classified as `Safe` and auto-approved without warning.
- **Solution**: Updated comment line termination in `src/shell_parser.cpp` to reset `in_comment` and end the pipeline on either `\n` or `\r`.
- **Verification**: Added unit tests in `tests/test_rules.cpp` (93/93 tests pass) and automated PTY tests verifying that scripts starting with comments and separated by `\r` trigger the Security Review prompt.




















### A third bug, found by using the tool

Writing the notes for this entry turned up a defect that no test would have
looked for: `tcx find help` printed the help page instead of searching, because
`wants_help` accepted a bare `help` alongside `--help` and `-h`. The help
*page* is not a search result; a query is data. Only the flags count now, and
`tests/help.rs` asserts the word is searchable. The unit test that had pinned
the old behaviour was updated — it had encoded the bug.

## tgw: the watchdog

`tgw` came after `tgr`, because it needs `tgr --check` to exist before it can
gate without a terminal. It was built in slices, each committed and tested
before the next, and each slice's safety property is tested by breaking it on
purpose and watching the test fail.

### Slices, and what each one cost

| Slice | What | Tests |
| --- | --- | --- |
| 1 | `watch.ini`, the help table, drift tests | 21 lib, 9 config, 8 help |
| 2-3 | clipboard, staging, the gate mapping | 52 lib, 12 pipeline |
| 4 | execution, `hold_rules`, `tgw cd` | 19 execute |
| 5 | holds, approve, deny, the auto lock | 15 hold |
| 6 | doctor, single instance, signals | 10 doctor |
| 7 | `docs/tgw.md` | (docs) |

### Bugs the slices themselves produced

A slice is not done when it compiles. Each of these was found by a test or by
using the thing:

- **The clipboard read could hang forever.** The deadline was checked after a
  blocking `read`, which never returns if the tool never writes. `std` cannot
  set a read timeout on a pipe, so the read moved to a thread and the deadline
  is enforced from outside it.
- **Killing the gate did not kill its children.** `sh -c 'something; sleep 30'`
  survived `child.kill()`, kept the stderr pipe open, and the timeout looked
  like a 30-second hang. The gate now runs in its own session and a timeout
  kills the group. This is the same reason `run_block` does it.
- **A restart re-staged every block.** Ids come from the content hash, so a
  watchdog restarted with the clipboard unchanged hit `File exists`. A `.seen`
  marker makes the second sight a duplicate instead of a failure.
- **A passing block threw away its watch names.** `GateResult::Pass` had no
  reason, so `hold_rules` could never match — the one case the name mapping
  exists for. `Pass` carries the names now.
- **`doctor` called a warning a failure, twice.** The `auto` lock was reported
  as a state-dir problem and again as a warning. The lock is a warning; `run`
  refuses separately.
- **`doctor` never checked `hold_rules`.** It appended `--list-rules` to
  `gate_cmd`, but `tgr` rejects `--check` with `--list-rules`, so the check
  could never pass. The checking flags are now dropped before asking.
- **`wakelock_cmd` was documented but never started.** The doc claimed it was
  "started and released around a run". It is now, with a test for the timeout
  path, because a doc that promises an unimplemented behaviour is worse than
  no doc.
- **`tgw once` ran in locked `auto`.** Found by pointing `once` at a locked
  config and watching it run a block. `once` is documented as the same pipeline
  as `run`, so it has to honour the same lock; otherwise it is a quiet bypass
  of the one thing standing between a fresh install and unattended execution.
  Both entry points now share one lock check.

### A bug in tgr, found while validating tgw

The full workspace run failed in `check_parity`: `adb uninstall` was danger by
paste and safe by `--check`. Driving the paste path by hand showed `HIGH RISK:
adb uninstall`, so `--check` was not wrong.

The cause was in the test: both comparison tests run concurrently and shared
one temp path, so one read the other's block. Fixed with a unique file per
call and run three times to tell a race fix from a flake.

That the test was racy matters beyond tidiness. It is the proof that `--check`
reads the same config as the paste path, and `tgw` trusts `--check` for every
block it runs.

### What was deliberately not built

- **`mode = auto` for a fresh install.** It is locked behind 20 approvals.
  Auto is what makes `tgw` convenient, which is exactly why it should be
  earned rather than assumed.
- **Any flag that skips the gate.** There is no such flag and there will not be
  one. `dry` exists to run nothing, not to run ungated.
- **A clipboard write from `tgw`.** The block writes its own output. A
  watchdog writing to the clipboard would be a second source of truth about
  what the human is about to paste.
