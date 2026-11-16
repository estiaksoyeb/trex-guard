# tgr — the guard, in Rust

`tgr` is the TREX Guard rewritten in Rust. It runs **beside** the C++ `tg`, not
instead of it. Both are installed; both protect a shell; neither knows about the
other's session.

## Why a parallel port at all

The C++ guard works. Rewriting a security tool is usually a bad idea, because
the only specification it has is its own behaviour. So the rule for this port
was:

> Match `tg` exactly, including its quirks. Change only what is provably wrong
> or dangerous, and write down every deliberate difference.

The port therefore ships as a second binary with its own tests, and can be
dropped back out at any time by not typing `tgr`. If it earns daily use, the
C++ guard can be retired — but that is a decision made from experience, not
from a rewrite.

## What is shared with `tg`

- **The config file**: `$XDG_CONFIG_HOME/trex-guard/config.ini` (default
  `~/.config/trex-guard/config.ini`), read-only. Your `[custom_rules]`,
  `[allowlist]` and `[disabled_rules]` apply identically to both guards. A
  guard that ignored your custom rules would be protecting a different system
  than the one you configured.
- **The rules**: every built-in rule, the pipeline rules (`curl … | bash`), the
  git sub-rules, and the snippet de-duplication behave as in `src/rules.cpp`.
- **The terminal model**: bracketed paste capture, the 15 ms burst drain for
  terminals that do not bracket pastes, the `\x1b[?2004l` suppression, and
  re-wrapping approved payloads in paste markers.

## What is deliberately different

### 1. Sessions cannot be confused with each other

The guarded child gets `TGR_ACTIVE=1`, **not** `TREX_GUARD_ACTIVE=1`.

`scripts/tg` refuses to start when `TREX_GUARD_ACTIVE` is set. If `tgr` set that
variable, you could not run `tg` inside a `tgr` session. And if `tgr` set
`TGR_ACTIVE` and also honoured it for nesting, a nested `tgr` would silently
do nothing. Two guards, two markers.

### 2. A user regex that does not compile is an error, not silence

The C++ guard wraps `std::regex` construction in `catch (std::regex_error)` and
drops the rule. A typo in `[custom_rules]` therefore silently disables a
protection you believe is on — the worst failure mode a guard has.

`tgr` reports it: the review panel shows

```
[CONFIG] rule `drop db` does not compile (…); the paste is held
```

and the paste is held at `Review`. It is never auto-approved on the strength of
a rule that could not run.

### 3. A regex that exceeds the backtrack limit fails closed

`fancy-regex` can backtrack exponentially. Every user regex is given a
backtrack limit (`BACKTRACK_LIMIT` in `src/rules.rs`; the hazard is measured in
`docs/tcx.md` §8). If the limit is hit, the paste is held at `Review` with
`rule `…` exceeded the backtrack limit; the paste is held`, instead of being
reported Safe because the match "did not finish".

Only a *non-matching* subject reaches the limit: when a match is found,
`find` stops early. This is why the test uses `(a{1,2})+\1$` against a string
that almost matches.

### 4. A failed terminal-size query is not fatal

The C++ guard exits if `TIOCGWINSZ` fails. `tgr` seeds the pty with size zero
and continues. The size is a convenience; refusing to guard because a terminal
cannot report its dimensions is the wrong trade. The size is refreshed on
`SIGWINCH` in both.

### 5. Terminating signals unwind instead of re-raising

The C++ handler restores the terminal, forwards the signal to the child and
re-raises it from inside the handler. `tgr` sets a flag; the main loop leaves
its poll, restores the terminal, forwards the signal to the child, and reaps it.
The observable result (terminal restored, child terminated) is the same, but no
cleanup work happens in a signal handler.

## What is *not* different

Everything the C++ suite asserts. That suite is the only specification `tg`
has, so it is the acceptance criteria:

- `crates/tgr/tests/cpp_suite.rs` — all 88 cases from `tests/test_rules.cpp`,
  ported verbatim.
- `crates/tgr/tests/parity.rs` — runs the **real** `./build/trex-guard` and
  `tgr` on a shared corpus and requires the same verdict.
- `crates/tgr/tests/parity_real_config.rs` — the same, under your real
  `config.ini`, including the custom rules.

The parity tests drive the C++ binary through a **pseudo-terminal**
(`tests/support/drive_cpp.py`), because `tg` calls `tcgetattr` on stdin and
refuses to run under a pipe. Testing the shipped binary as a user would run it
is the only comparison worth making.

`crates/tgr/tests/pty_e2e.rs` drives `tgr` the same way and checks the things a
classifier test cannot: that a `Review` paste runs only when approved, that a
cancelled one has no effect, that a safe paste is not prompted, and that the
child sees `TGR_ACTIVE=1`.

## Differences that are inherited, not introduced

`[custom_rules]` splits on `|`, so a custom pattern **cannot contain
alternation**. This is a limitation of the C++ config format and applies
identically to `tg`; changing it would silently reinterpret existing configs.

## Usage

```
tgr                 # guarded shell
tgr <command> ...   # run one command under the guard
tgr --help
tgr --version
```

`--force` is accepted and stripped, for scripts that pass it.

## Building and testing

```
cargo build -p tgr
cargo test -p tgr
```

The parity tests skip loudly if `./build/trex-guard` is missing (build it with
`cmake -B build && cmake --build build`). They are slow — each paste starts a
real shell — around three minutes each.
