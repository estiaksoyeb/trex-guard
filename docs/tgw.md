# tgw — the watchdog

`tgw` sits between the clipboard and the guard. You copy a block in a chat;
`tgw` asks `tgr --check` what it thinks of it; then it runs it, holds it for a
tap, or refuses. The block's own `| clip` puts the output back on the
clipboard, so the copy-paste round trip is one copy in chat and one paste in
the terminal — the block does not need to be pasted at all.

```text
chat block --> clipboard --> tgw --> tgr --check
                                   |--> run      (gate passed, mode allows)
                                   |--> hold     (gate review, or dry/confirm)
                                   |--> block    (gate danger, or anything unclear)
```

## First run needs no setup

There is nothing to configure before starting. On first use `tgw` writes
`watch.ini` itself — `confirm` mode, `clipg` as the clipboard reader, and
`tgr --check {file}` as the gate — and `tgw check-config` (the command the
error messages point at) is the thing that creates it. An existing file is
**never** overwritten.

The file is looked for in every conventional place, in order:

1. `$TCX_HOME/config/watch.ini`
2. `$XDG_CONFIG_HOME/tcx/watch.ini`
3. `~/.config/tcx/watch.ini`
4. `~/.local/share/tcx-home/config/watch.ini`

The first two matter more than they look. Setting `TCX_HOME` for `tcx` used to
hide a `watch.ini` sitting in `~/.config/tcx/` from `tgw`, which read as "the
file is right there and the tool cannot see it". If `TCX_HOME` is set
explicitly, that is treated as a statement of where config lives and the search
stops there rather than reading a config the user did not point it at.

## What it is for

The block-based workflow asks the human to paste a block into a terminal. That
paste is where the guard earns its place: it decides whether the block may run.
`tgw` keeps that decision and moves the *execution* off the terminal, so a long
block does not need to be watched and a dangerous one does not need to be
read by eye at speed.

It is deliberately not a shell, not an agent, and not a sandbox. A block that
`tgw` runs has the same privileges the human's shell has.

## What it must never do

These are the properties the code exists to protect. Each is enforced by a
test that fails if the property is broken.

- **Never classify risk.** `tgw` does not parse a block. It runs
  `gate_cmd` (normally `tgr --check {file}`), reads an exit code, and maps it.
  The rule names on stderr are used for display and for `hold_rules`, never as
  a judgement of their own.
- **Never bypass the gate.** There is no flag, no config key and no mode that
  skips `gate_cmd`. `dry` runs nothing; `confirm` waits for a human; `auto`
  runs a pass. There is exactly one call site of the executor, and it is
  reachable only from a pass.
- **Never make a verdict looser than the gate's.** `hold_rules` can turn a pass
  into a hold. Nothing can turn a block or a gate error into a run.
- **Never read the guard's config.** `~/.config/trex-guard` belongs to the
  guards. `tgw` only ever *executes* the gate binary; it does not know what a
  rule is.
- **Never write to the clipboard.** The block writes its own output. A
  watchdog that wrote to the clipboard would be a second source of truth about
  what the human is about to paste.

## Modes

| Mode | Passing block | Hold | Block |
| --- | --- | --- | --- |
| `dry` (default) | recorded as a hold, **not run** | recorded, not run | recorded, not run |
| `confirm` | recorded as a hold, waits for a tap | waits for a tap | recorded, not run |
| `auto` | runs | waits for a tap | recorded, not run |

`dry` is the default so a fresh install cannot run anything by accident. It
still records holds, so you can see exactly what *would* have run — approving
one is refused, because `dry` means dry.

### The `auto` lock

`mode = auto` is refused until the human has approved `auto_unlock_approvals`
blocks (default 20) in `confirm`. The counter counts approvals that actually
ran. A failed run keeps its hold and does not count. Denies are logged and do
not reset the counter: resetting would turn a discard into a punishment.

`tgw status` prints the remaining count; `tgw run` and `tgw once` both refuse
to start with `mode = auto` while it is locked. `auto_unlock_approvals = 0`
unlocks it from the first run, which is the escape hatch for someone who has
already used `confirm` for long enough to trust it.

## Holds

A hold is a record in the state directory plus the staged file. Nothing about
it is trusted later. `tgw approve ID` re-reads the file, re-hashes it, checks
its permissions, checks the expiry, compares the mode, and asks the gate
again. All of these must agree with the record or the approval is refused:

| Check | What it catches |
| --- | --- |
| hash matches | the staged file was edited after the hold |
| file is `0600` | something else can read or own the block now |
| not expired | a hold is a moment, not a standing permission |
| mode still allows running | you moved to `dry` after the hold |
| gate still passes | the gate learned a rule since |

A **block can never be approved**. The hold record carries `verdict=hold`
and `tgw` refuses any record that does not, so dropping a file into the holds
directory does not create an approval path.

`tgw deny ID` deletes the record and the staged file and logs the reason the
block was held, so a discarded block is still explainable afterwards.

## The gate contract

`gate_cmd` must contain `{file}`. The exit code decides everything:

| Exit | Meaning | `tgw` |
| --- | --- | --- |
| 0 | safe | pass |
| 1 | danger | block, never approvable |
| 3 | review | hold |
| 2 | usage or read error | block |
| anything else | unspecified | block |
| killed by a signal | no answer | block |
| cannot start / timed out | no answer | block |

The last four are fail-closed: a gate that has not said "safe" has not said
"safe". The gate runs in its own session and a timeout kills the whole process
group, so a gate that spawned something cannot outlive it.

## `hold_rules`: the human's veto

The gate names the rules it matched (`rule=…` and `watch=…` on stderr). You
can say that some of those names always need a tap, even on a pass:

```ini
mode = auto
hold_rules = rm, mv, git-clean, sed-i
```

A pass whose names include one of these becomes a hold. Only **names** live
here: the patterns are `tgr`'s, in its `[watch-rules]` section, so the two
programs cannot drift apart. `tgw` never pattern-matches.

The names come from `tgr --list-rules`; `tgw doctor` fails if a name in
`hold_rules` is not there, because a typo means a hold that silently never
happens.

## Configuration

`$TCX_HOME/config/watch.ini`, else `$XDG_CONFIG_HOME/tcx/watch.ini`, else
`~/.config/tcx/watch.ini`. It is **never** read from the current directory or
the repository: a file in the tree can change under your feet, and a watchdog
whose policy follows the repository is not a boundary.

| Key | Default | Notes |
| --- | --- | --- |
| `clip_get_cmd` | *(required)* | prints the clipboard to stdout |
| `gate_cmd` | *(required)* | must contain `{file}` |
| `notify_cmd` | *(none)* | `{id}` `{status}` substituted; courtesy only |
| `wakelock_cmd` / `wakelock_release_cmd` | *(none)* | started and released around a run |
| `shell` | `bash` | argv string; runs the staged file |
| `prelude` | *(none)* | sourced before the block, for aliases |
| `env_allow` | `PATH HOME TCX_HOME TERM LANG` | exactly what the block sees |
| `cwd_mode` | `inherit` | `inherit`, `fixed` or `file` |
| `cwd` | *(none)* | required for `fixed`; absolute, must exist |
| `cwd_file` | *(none)* | required for `file`; re-read for every block |
| `trigger_first_line` | `set +H` | a block's first non-empty line |
| `poll_ms` | `500` | clipboard poll interval |
| `max_block_bytes` | `65536` | a larger clipboard is ignored, not truncated |
| `timeout_s` | `120` | a block that runs longer is killed |
| `gate_timeout_s` | `10` | a gate that runs longer is killed and blocks |
| `max_output_bytes` | `65536` | captured per stream; the rest is drained |
| `hold_ttl_s` | `300` | how long a hold can be approved |
| `mode` | `dry` | `dry`, `confirm` or `auto` |
| `auto_unlock_approvals` | `20` | approvals needed to unlock `auto` |
| `state_dir` | `$XDG_DATA_HOME/tcx/watch` | 0700; holds, staging, the lock |
| `log_dir` | `state_dir/log` | `events.log` and kept block copies |
| `hold_rules` | *(none)* | rule **names** that always hold |

Commands are **argv strings**, split without a shell: no `$VAR` expansion, and
`|`, `;` and `>` are ordinary characters. Values are substituted as whole
tokens, so a path with a space cannot become two arguments. `{file}` is
replaced with the staged path and is never split.

Unknown keys are **problems**, not ignored: `gate_cmd = tgr --chek {file}`
would otherwise mean no gate. `tgw check-config` prints every problem at once.

## Running a block

- stdin is `/dev/null`; a block that reads the terminal cannot hang.
- The environment is exactly `env_allow`, plus `TGW_IN_BLOCK=1`. A token in
  your shell is not visible to a block unless you listed it.
- The block runs in its own process group, so its timeout kills its children
  too.
- Output is captured up to `max_output_bytes` per stream and printed; the rest
  is still drained so the block cannot deadlock on a full pipe.
- `cwd_mode = file` re-reads `cwd_file` for **every** block, so a stale read
  cannot outlive the file. A relative or missing path is an error, not a
  fallback.

`TGW_IN_BLOCK` makes `tgw cd` refuse inside a block. This is an **accident
guard, not a boundary**: a block can still write `cwd_file` directly. It exists
so a block that happens to say `tgw cd /` does not redirect every later block.

## Commands

| Command | Does |
| --- | --- |
| `tgw run [--once] [--interval MS]` | watch until Ctrl-C |
| `tgw once` | one iteration, for testing |
| `tgw approve ID` | re-check and run a hold |
| `tgw deny ID` | discard a hold |
| `tgw status` | mode, lock state, pending holds |
| `tgw log [--tail N]` | recent events |
| `tgw doctor` | prove the pieces work |
| `tgw check-config` | validate `watch.ini` |
| `tgw cd PATH` | write `cwd_file` (absolute paths only) |

Exit codes: `0` success, `1` runtime error or refused, `2` usage error.

## `doctor`: what it checks, and why each one

`doctor` is deliberately noisy about the failure that would otherwise be
silent:

- **The config**, every problem at once. It stops here if the config cannot
  run, because the other checks would be testing the wrong thing.
- **State and log directories** exist and are writable; the state dir is
  `0700`. It reports if the directory *was* loose, because that is worth
  knowing once.
- **The clipboard** runs and exits 0. An empty answer is normal.
- **The gate** passes `echo hello` and blocks `rm -rf /tmp/...`. A gate that
  passes both is not a gate; one that blocks both cannot see `{file}`.
- **`hold_rules`** against the gate's own `--list-rules`. `gate_cmd` is a
  checking command and `tgr` rejects `--check` with `--list-rules`, so the
  checking flags are dropped and the program is asked on its own. A gate that
  cannot answer, or answers with no rules, fails rather than passing silently.
- **`notify_cmd`** runs, if set. It is a warning, not a failure.
- **The wakelock program** exists, if set. It is never started.
- **`mode = auto`** is unlocked. A locked `auto` is a warning; `run` refuses
  separately.

## The log

One line per event in `log_dir/events.log`:

```text
ts | id | hash | verdict | reason | exit | duration_ms
```

Seven fields, always. A `|` or newline in a reason is replaced so a script can
split on `|`. Every non-ignored verdict keeps a copy of the block in
`log_dir/blocks/` (last 200), because after the fact the useful artefact is the
exact bytes.

## State directory

```text
state_dir/
  stage/       0600 blocks, one per content hash, plus a .seen marker
  holds/       0700; one key=value record per pending hold
  approvals    how many approvals have actually run
  tgw.lock     the single-instance pid
```

Staging a block under its content hash means the same block cannot be staged
twice. A `.seen` marker survives a restart, so a watchdog restarted with the
same clipboard does not re-run the last block.

## One instance

Two watchdogs would each see every block, and in `auto` would each try to run
it. The second refuses to start, naming the pid. A lock left by a crashed
process is cleared automatically, because requiring a human to delete a file
after every crash is how a watchdog gets switched off for good.

## Signals

SIGINT and SIGTERM stop the loop after the current iteration. A block already
running keeps its own timeout: cutting it in half would leave whatever it was
doing half-done, which is worse than waiting.

## Building and testing

```bash
cargo test -p tgw
```

`tgw` is `std`-only: the two system calls it needs (`setsid`, `kill`) are
declared as `extern "C"` rather than pulled from a crate. The tests run the
real binary against fake clipboard and gate scripts in a temporary `TCX_HOME`,
so they exercise the process plumbing, not just the functions.

The safety tests are written to be broken on purpose: making `dry` run, making
`hold_rules` run, or disabling the approval hash check each makes a test fail.
