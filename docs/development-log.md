# Development Log

## 2026-11-17 — v0.2.2: the workflow prompt now ships with the release

Rule 2 of the prompt template was changed to ban `;`-joined batches (`set +H`,
`{`, labelled commands one per line, `} | clip`). Editing the template exposed
a gap: nothing shipped it. `briefing::template` reads only
`<config>/prompt.md`, `install.sh` installed no file, and `examples/` had no
copy — so a fresh install had a `tg fresh` that exited 1 with "no prompt
template found", while the human who wrote the rules had one only because it
was hand-placed here.

The file stays out of the binary. `examples/prompt.md` is copied into the
release tarball next to `config.ini`, and `install.sh` seeds it to
`$TCX_HOME/config/prompt.md` when `TCX_HOME` is set, else
`$XDG_CONFIG_HOME/tcx/prompt.md` — the two places `tcx` actually looks —
creating directories as needed and **never overwriting** an existing file (the
same rule as `config.ini`). Embedding it with `include_str!` was rejected: it
would let a shipped binary hand an AI rules the user never chose, and force a
rebuild whenever a rule changes.

Two installer tests cover it: the prompt appears on a clean install, and a
user's own `prompt.md` survives an upgrade; a third proves the destination
follows `TCX_HOME` rather than XDG. `docs/tcx.md` also said the legacy
fallback was `<config>/tcx/prompt.md`; the code uses `<config>/tg/prompt.md`,
and the doc now matches the code.

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

### Two ways `tgr` could trap the terminal

Reported from daily use, not from a test: type `tgr`, then `exit`, and the
prompt comes back but never returns. `tg` escapes; `tgr` does not. Killing
`tgr` did not work either.

**The exit hang.** A probe (`ps`, `/proc/<pid>/status`, `wchan`) showed the
guarded bash as a *zombie* and `tgr` in state `R` — running, not waiting. So
the child was already dead and `tgr` was spinning, which ruled out `waitpid`
being the blocker. The relay polled with `POLLIN` and only checked `POLLIN`.
A minimal reproduction with no guard at all — open a pty master, close the
slave, poll it — hit 187,061 iterations in half a second: `poll` (and
`select`) report a hung-up descriptor as ready *immediately and forever*, so
the 250 ms timeout never applies, the loop never reads, never sees EOF, and
never returns to `waitpid`.

This was a **port regression**, not an inherited flaw. The C++ loop uses
`select` and acts on `FD_ISSET`, so a hang-up is read like any other
readiness, returns `0` or `EIO`, and ends the loop. The port narrowed that to
`POLLIN` in both the wait mask and the check. Fixing it restored the original
semantics rather than adding a new mechanism.

**The SIGTERM hang.** With the loop fixed, `kill tgr` still did nothing. The
handler set a flag that stopped the loop, but nothing told the shell, so
`waitpid(child, None)` blocked forever — `/proc/<pid>/wchan` said `do_wait`.
`LoopEnd::forward_signal` existed to carry the signal and **was never
assigned**; the `if let Some(sig)` that would have forwarded it could not run.
The signal number is now recorded in an atomic from the handler and forwarded
after the loop leaves.

That still was not enough. **Interactive bash ignores `SIGTERM` and
`SIGHUP` by design**, so forwarding a polite signal to a shell that is built
to ignore it left `tgr` waiting again. `tgr` now waits 500 ms, then `SIGKILL`s
the shell — what closing the terminal window does. `SIGINT` and `SIGHUP` were
checked and both end cleanly with no orphaned shell.

**Why the tests did not catch either.** The pty driver always `SIGKILL`ed its
child, so "the guard exited" and "the guard hung until we killed it" were
indistinguishable. A test that kills its subject cannot observe a hang. The
new driver waits for the guard process itself to exit against a deadline, and
a further probe checked that no child bash survives.

Three tests now cover this: `the_guard_exits_when_the_shell_does`,
`ctrl_d_exits_the_guard_too` and `sigterm_ends_the_guard_and_the_shell`. Each
was run against the reverted code and failed there; the SIGTERM test was
checked against *both* halves removed individually, since either alone
reproduces the hang.

One bug was in the tests themselves while writing them: `py_repr` uses Rust's
`{:?}`, which renders Ctrl-D as `"\u{4}"` — a Python syntax error. The script
died, stdout was empty, and the test reported a hang that was not there. Keys
are now passed as a list of byte values. And the first version stopped
pumping output at pty EOF, which happens when the *shell* leaves while the
*guard* is still reaping it; the driver keeps waiting for the guard past EOF.

### `tgr` let a user nest without saying so

Reported as "tgr doesnt alert me when im runing inside tgr". `tg` prints
`Notice: A guarded shell is already active` and points at `--force`; `tgr`
started a second guard silently.

The flag was the tell. `tgr`'s own module doc said `--force` is accepted "so
scripts that pass it keep working" — which was honest: the guard check it
overrides had never been ported, so `--force` did nothing and `child_argv`
stripped it before anything could look. `tgr` exported `TGR_ACTIVE=1` to its
child and never read it back, so the marker existed purely for other tools.

Fixed by checking `TGR_ACTIVE=1` at startup and honouring `--force` anywhere
in argv, matching `tg`'s positions and exit code. Each guard reads only its
own marker, so `tgr` inside `tg` still starts — the separation the port was
built around.

Two test bugs of my own along the way, both worth recording because both
produced a convincing but false failure: the pty helper built the child argv
without `argv[0]`, so `--force` sat in the program-name slot and was never
seen as a flag; and the "did it start?" check read `waitpid` once, catching a
refusing guard mid-exit and calling it started. The generated script is now
printed under `TGR_TEST_DEBUG=1` rather than reasoned about.

### Installing did not leave a working setup

Reported as: "installing the software should automatically fix everything."
It did not. `tgw run` failed on a machine where a correct `watch.ini` already
existed, and the fix asked of the user was to copy it somewhere else by hand.

Two separate faults, and the second is the one that mattered:

**The installer never installed the Rust tools.** `scripts/install.sh` knew
only about the C++ `trex-guard` and `tg`. It never placed `tgr`, `tgw` or
`tcx`, and never wrote `watch.ini`. The installer now ships them from the
tarball when present, builds them from source when `cargo` is available, and
runs `check-config` at the end so the watchdog is verified rather than assumed.

**`tgw` bet on one config path.** It resolved a single root from the
environment, so setting `TCX_HOME` for `tcx` hid a `watch.ini` that already
existed under `~/.config/tcx/`. From the user's side the file was plainly
there and the tool refused to see it — and the error message named the path it
wanted rather than the one that existed.

It now searches every conventional location in order, and creates the default
when none exists. `check-config` creates it too, because that is the command
the error message points at, so "just run `tgw check-config`" has to be true.
An existing file is never overwritten.

The one deliberate limit: an **explicit** `TCX_HOME` stops the search. Falling
through to another root's file there would mean `tgw` reading a config the
user did not point it at. Written down because it is the sort of rule a later
reader would otherwise "simplify" away.

Two existing tests asserted that a missing config was an error. They were not
deleted: one was rewritten to assert the default is created *and* is usable
(`confirm` plus an explicit gate), the other to assert an existing config is
never overwritten. A third covers the default install — no environment
variables at all, config under `~/.config/tcx` — which is the case the
installer produces.

A test-harness fault caught along the way: the config tests set `TCX_HOME` but
not `HOME`, so the new search reached the real `~/.config/tcx/watch.ini` and
passed or failed depending on what was installed on the machine. `Home::run`
now isolates `HOME` and `XDG_CONFIG_HOME` too.

### `hold_rules` removed, and the fail-open it was hiding

`hold_rules` was a list of gate rule names in `watch.ini` that always held,
even when the gate passed the block. Removed.

The reason is the one the user gave: there should be one source of truth, and
the guard's `config.ini` already is it. Two files expressing "I want to see
this before it runs" drift apart — you edit one and not the other, and the half
you forgot silently stops applying.

It was also barely doing anything. A hold only changes a **pass**: a danger is
already blocked and a review is already held, so `hold_rules` was consulted for
neither. It could only ever matter in `auto` mode, for a name `tgr` emits from
a *safe* block. In the shipped list of six names, three (`mv`, `sed-i`,
`git-checkout`) are not rule names `tgr` produces at all, and two had their own
review/danger verdict already. Only `rm` could fire.

Worse, `doctor` accepted those names because it validated against
`tgr --list-rules` — which includes *watch* rules — so a list of six mostly
inert names passed inspection. A check that passes on a broken config is worse
than no check.

Anything that should be reviewed now goes in the guard's `config.ini` as a
`review` rule: the gate returns review, `tgw` holds it in every mode, and one
file owns the decision. An old `watch.ini` still carrying `hold_rules` is
refused with a message naming `config.ini`, not silently ignored.

**The bug this uncovered is more serious than the removal.** Writing a test
that an old `hold_rules` key is refused showed the block *running*. The cause
had nothing to do with `hold_rules`: `config::load()` called `parse_into` and
discarded its return value, so **every** parse problem — an unknown key, a
misspelled `mode`, a malformed number — was thrown away on the real run path.
Only `check-config` ever displayed them. A `watch.ini` that failed to parse
left `tgw` running with defaults, which meant a gate line that never took
effect silently became no gate at all.

`load()` now returns its problems, and both `load_config()` and `doctor` treat
them as fatal. A config that was not understood is not used. Covered by
`a_config_that_fails_to_parse_never_runs_a_block`, confirmed to fail with the
old behaviour restored.

Tests removed rather than rewritten where the behaviour is gone: three
`doctor` tests for the deleted rule-name check, and the hold-rule escalation
cases. Kept and re-pointed at what still holds: a pass runs in auto with no
second list, a danger stays a block, and an old key is refused.

## "It showed hold but never ran" — three bugs behind one symptom

The report was that a held block announced an id and then nothing happened, and
that after an edit even the announcement stopped appearing. That second half is
what made it look like `tgw` had gone dead, and it was a separate bug from the
first.

What the state directory showed: `.seen` markers for both ids still on disk,
`holds/` empty. So the block *had* been seen and held both times. The log did
contain both hold lines; `tgw log` filters and limits, and grepping
`events.log` directly is what showed them. Do not trust `tgw log` alone.

Three faults, in the order they bit:

1. **The hold expired and was deleted in silence.** `sweep_expired` removed it
   and printed nothing. A hold that was announced and then vanished is
   indistinguishable from a block that was never seen. It now returns the ids
   it dropped and the caller says so, with what to do about it.
2. **The seen marker was permanent.** Dedupe existed to cover the clipboard
   still holding the block and the block's output replacing it — a window of
   seconds. It was implemented as a file that never expired, so a block that
   was held and never approved could never be offered again. Copying it a
   second time was silently ignored as a duplicate. The marker now records when
   it was written and expires after 300 s; an unreadable marker counts as not
   seen, because offering a block twice is recoverable and ignoring it forever
   is not.
3. **A failed `holds::save` was discarded** with `let _ =`. The id was announced
   anyway, so `approve` could not find it. It is now an error that logs
   `cannot save hold: …` and blocks, rather than printing a lead that goes
   nowhere.

Fixing (2) exposed a fourth: `stage()` used `create_new`, so re-offering
identical content failed with `File exists`. The id is derived from the content
hash, so a block whose earlier file is still there could never be re-staged.
`AlreadyExists` is now accepted only when the bytes are identical; different
content under the same id is still refused, so a colliding or forged id cannot
make the gate read one block while the shell runs another.

`announce()` also now says, after a hold, that nothing has run and which
command runs it. A hold is not a run, and saying only "hold" reads as "done".

The user was right three times and I was wrong three times. I blamed the
clipboard bridge; it works. I blamed the trigger line; it was fine. Then I
explained "a hold is not a run" as though that were the whole answer, when the
real faults were silent expiry and permanent dedupe. "Look at the code enough"
was the correct instruction.

## Test suite time was mostly a fixed wait, not work

`check_parity` took 264 s and failed intermittently in the full suite while
passing alone. Both facts had one cause: `drive_pty.py` read to its deadline on
every paste instead of stopping when the guard had answered, and the early-exit
branch was gated on `reply is None`, so it never fired for the parity tests.
Under load the screen could still be truncated when the deadline passed, and a
truncated screen has no `HIGH RISK` in it — which reads as "safe", which looks
exactly like the two paths disagreeing. The flake was never in `tgr`.

The driver now stops when the screen has gone quiet *and* a verdict or reply is
present. `check_parity` 264 s → 40 s, `watch_rules` 134 s → 14 s, same answers,
flake gone.

The C++ parity suites (`parity`, `parity_real_config`) still take ~250 s each
and were left alone. They are slower for a different reason, and investigating
it turned up something worth recording rather than fixing: the C++ guard does
not answer ble.sh's cursor-position query, so with the real `.bashrc` the shell
sits at `[ble: press RET to continue]` and never runs the pasted block. Those
suites are therefore comparing text that is not a verdict from a shell that
actually ran the block. `tg` is frozen, so this is written down, not changed.

## Releases come from GitHub now, and CI is the gate

The C++ guard is no longer built or shipped. `tgr` is a standalone port that
reads the same `config.ini`, so a release needs no CMake and no g++. The C++
source stays in the tree and `test_rules` still runs in CI, because `parity`
compares the port against it; without a `trex-guard` binary those suites print
SKIP and pass, which is a green that proves nothing. The Rust job therefore
builds the reference itself rather than relying on a separate job that cannot
share a filesystem.

Releases are musl-static for aarch64 and x86_64 — verified against the actual
artifact, not assumed: the GitHub-built `tgr` reports `statically linked` and
runs here, and still exits 1 on `rm -rf` and 0 on `echo`.

`tg` falls back to `tgr` when no C++ guard is installed. It also now checks
both session markers: it used to check only `TREX_GUARD_ACTIVE`, so a `tg`
inside a `tgr` session recurred instead of refusing. `--help` and `--version`
are answered before that check, which previously blocked them inside a session.

The installer installs only from a published release and verifies
`SHA256SUMS`. It has no build-from-source fallback: an unverifiable download
stops rather than compiling something CI never saw. That is a deliberate
trade — it needs a working release — and it is why CI had to be green before
the tag could be pushed.

**Three mistakes CI caught that local runs did not**, which is the argument for
having the gate at all:

1. A wakelock test asserted a file written by the helper command. The helper
   is spawned and killed on release, so whether it reached the write depends on
   scheduling: green here, red on `ubuntu-latest`. It now asserts `is_held()`,
   which is the actual contract, and the two cases that must *not* hold a lock.
2. The installer test set `HOME` but the installer prefers `XDG_CONFIG_HOME`,
   which CI sets. The config landed outside the sandbox. Re-run locally with a
   hostile `XDG_CONFIG_HOME` to confirm it now matches CI and leaks nothing.
3. The static-linkage check tested `ldd` output for "not a dynamic". A static
   musl binary makes `ldd` print "statically linked", so a correctly built
   x86_64 release failed verification, while aarch64 passed only because `ldd`
   was absent there. `file` reports linkage exactly.

Each of those three would have blocked every release, and none was a product
bug. All were tests or checks asserting something that happened to be true on
one machine.

Clippy is reported but not enforced. There are ~30 pre-existing style
warnings; failing a release on a collapsible `if` is noise, but the output is
kept so a lint that could hide a defect can be promoted and fixed.
