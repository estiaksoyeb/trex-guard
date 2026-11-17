# Onboarding: What This Is, How It Is Used, And Why

> Read this first, then `docs/tcx.md` if you need the design record.
>
> This document exists because a fresh session — one with no memory of the one
> that built this — must be able to answer three questions without guessing:
>
> 1. What is this project?
> 2. How does the human actually work with me?
> 3. Why is it built this way, and what has already gone wrong?

Every claim below is either implemented code, a decision recorded in
`docs/tcx.md`, or a mistake that was actually made in a previous session.

---

## 1. The project in one paragraph

`trex-guard` is four binaries in one repository:

- **`tgr`** — the guard: a paste-safety firewall that intercepts pasted shell
  text *before* it reaches the shell and halts high-risk pastes for review.
  It replaced a C++ guard, keeping that guard's rules and config file. Sources
  in `crates/tgr/`, built by Cargo. See `docs/tgr.md`.
- **`tg`** — the launcher, a small shell script that starts `tgr` (and forwards
  `fresh`/`resume` to `tcx`).
- **`tcx`** — TREX ConteXt, a Rust binary that keeps AI-assisted context
  *outside* the repository: decisions, gotchas, open questions, and a shallow
  annotated map of the tree. Sources in `crates/tcx/`, built by Cargo.
- **`tgw`** — TREX Guard Watchdog, a `std`-only Rust binary that watches the
  clipboard and gates each block through `tgr --check` before running it,
  holding it, or refusing it. It never classifies risk itself and never writes
  to the clipboard. Sources in `crates/tgw/`, built by Cargo. See
  `docs/tgw.md`.

They share a repository and almost nothing else. That is deliberate (see §8.1).
`tgw` depends on `tgr` at runtime only: it executes whatever `gate_cmd` names.

The guards and `tcx` are not two halves of one feature. A guard protects the
terminal from a bad paste. `tcx` answers "what should a new chat know before it
starts". The only place they touch is that `tg fresh` / `tg resume` are
forwarded to `tcx` by the `tg` launcher, in one direction only. `tgr` is a bare
guard binary with no launcher and no forwarding.

### Which guard to use

There is one guard, and `tg` runs it. `tg` is a shell script that execs `tgr`,
found beside the launcher first and then on `PATH`; it is not a second guard and
there is no C++ binary to fall back to (it was removed on 2026-11-17).

```bash
tg              # a guarded shell
tg fresh | clip # per-chat commands, forwarded to tcx
tgr             # the same guard, without the tcx forwarding
```

The division is exact:

- **Starting a shell**: `tg` or `tgr`. Both are the same guard.
- **`fresh` / `resume`**: `tg` only. `tgr` is a guard and nothing else, so
  `tgr fresh` starts a guarded shell and tries to run a command called `fresh`.
  It does not forward to `tcx`.

`tgr` shows a Review prompt where a user rule in `config.ini` cannot be used —
a rule that does not compile, or one that exceeds the backtrack limit. The
paste is held at `Review` rather than the rule being silently dropped. This is
deliberate and is one of the recorded differences; if the human sees a Review
prompt, that is a rule that could not be applied, not a new risk in the paste.

The guard reads `~/.config/trex-guard/config.ini` and never writes it.

---

## 2. How the human works, and what your job is

### The one fact that governs everything

**The human's entire job is: run one block, paste the output back.**

The human never composes commands, never types `tcx note ...`, never decides
what to grep. You write every command. You emit paste-ready bash blocks; the
human pastes them into a terminal and pastes the output back into the chat.

If a design assumes the human will "just type a command when they need it",
the design is wrong. This single assumption explains almost every decision in
this repository.

### The loop

```text
human:  tg    (or tgr)            # fresh terminal -> guard active
human:  tg fresh | clip           # prompt with the Project section filled in
human:  paste into a new chat

ai:     emits ONE bash block      # { rg ...; } | clip
human:  paste the block, paste output back

        ... this repeats while working ...

ai:     as it learns, includes note-writing inside its blocks
human:  paste -> notes saved (no extra effort)

human:  tg resume | clip          # new chat: prompt + Project + what was learned
human:  paste -> the new AI is caught up
```

The human is **the execution layer**. That is the whole design of the workflow
prompt. Nothing in the tooling may add steps to that loop.

### How you write a note

A note rides inside a block you were already sending, as one extra line:

```bash
tcx note --kind gotcha --dir crates/tcx/src --tag help \
  'the help table is the source of truth; a flag parsed but not listed ships invisible'
```

- **Single-quote the text.** It then survives `"`, `$`, `|` and brackets
  untouched, including `|`, which would otherwise split the note line.
- Use the long flags (`--kind`, `--dir`, `--tag`), not `-k`/`-d`.
- End the note with the fact and its reason, not a narration of the session.
- On an edit, append the note *inside the same block, after the change has
  been verified*, so a failed edit cannot leave a note claiming success.

### What your blocks look like

Real example, taken from an actual session:

```bash
set +H
{
  echo "## Layout Files"
  rg -n --max-columns 160 -m 5 'Open full player|Now Playing' .

  echo "## Theme/Color Files"
  rg -n --max-columns 160 'MaterialTheme|dynamicColor' .

  echo "## Current Play/Pause Logic"
  rg -n --max-columns 160 -m 5 'play|pause|isPlaying' . | head -n 20
} | clip
echo exit=$?
```

The rules that shape this behaviour live in the prompt template, not in this
document (see §3). Do not restate them here; read the template.

---

## 3. The two commands that justify the tool's existence

One of them is run **per chat**, then pasted: `fresh` for the first chat on a
project, `resume` for every chat after that. Never both.

They print to **stdout only**. The program never invokes a clipboard tool:
`tg fresh | clip` is a choice the human makes at the shell, because only the
human knows where the text should go. The template path is printed on
**stderr**, so it never lands in a paste.

### `tg fresh` — start a session

Prints the workflow prompt, with the `## Project` section filled in from the
repository the human is standing in. The shape, using a different project as an
example (this repository reports Rust/Cargo):

```text
## Project
mpvRex: Android video player, Kotlin + Jetpack Compose
Package: xyz.mpv.rex
Source: app/src/main/kotlin/xyz/mpv/rex/
Build: Gradle
type: code
```

Detection **reads** manifests and never runs a build system, so it takes
milliseconds and executes no repository code. Sources: `Cargo.toml`, Gradle
(`settings.gradle.kts`, Android manifest, Compose usage), CMake, npm
`package.json`, `pyproject.toml`, `go.mod`.

### `tg resume` — continue in a new chat

`fresh`, plus the store's notes under `## What previous sessions learned`. That
is what lets a brand-new AI — with no filesystem access and no memory —
continue without re-exploring the repository.

### Where the prompt text lives

In a file the human edits, never in the binary. First found wins:

1. `--template FILE`
2. `<config>/prompt.md`
3. `<config>/tg/prompt.md` (legacy name, still honoured)

`<config>` is `$TCX_HOME/config` when `TCX_HOME` is set, otherwise
`~/.config/tcx`. On this machine the shell integration sets `TCX_HOME`, so the
live file is:

```text
~/.local/share/tcx-home/config/prompt.md
```

That is true **only in a shell that sourced the integration file**
(`scripts/tcx-integration.bash`, sourced from `~/.bashrc`). In a shell without
it, `TCX_HOME` is unset, the lookup falls back to `~/.config/tcx/prompt.md`, and
if that file does not exist `fresh` exits 1 with "no prompt template found".
If that happens, source the integration file rather than moving the template.
`tcx fresh --help` documents the fallback *order*; the template path actually
used is printed to stderr on every run.

**The repository is never consulted.** A cloned repo must not be able to change
the rules an AI is given.

**The rules belong to the human.** Edit that file only when the human asks you
to change how you work. It is outside the repo, so your blocks *can* write to
it; do not treat that as permission. If you believe a rule is wrong, say so and
propose the edit — let the human decide, because you are the thing the rules
govern.

---

## 4. The store: where context lives

Context lives **outside the repository**, split by kind, never one dot-folder.
Two layouts, and the difference matters because only one adds a `tcx` suffix:

```text
# XDG layout (TCX_HOME unset)
$XDG_DATA_HOME/tcx/projects/<id>/notes.md    # judgment, irreplaceable
$XDG_CACHE_HOME/tcx/projects/<id>/           # index; none exists in v1
$XDG_CONFIG_HOME/tcx/prompt.md               # the prompt template

# TCX_HOME layout (single root, no extra `tcx` segment)
$TCX_HOME/data/projects/<id>/notes.md
$TCX_HOME/cache/projects/<id>/
$TCX_HOME/config/prompt.md
```

`tcx id --human` prints the resolved `projects/<id>` store directory; `tcx
fresh` prints, on stderr, the exact template file it used. Trust those over
this table.

In practice v1 writes only the notes and the prompt template. The cache
directory exists for the index that is deliberately deferred, because nothing
yet needs a cache to be fast. There is no `config.ini`: the only setting that
matters is `TCX_HOME` itself. The shell integration sets it to
`$HOME/.local/share/tcx-home` only if unset.

Three rules follow from this, and each one has a reason:

| Rule | Why |
| --- | --- |
| The repo must not be polluted | The human works in worktrees and clones; a `.ai/` folder in the repo goes stale per checkout and pollutes diffs. Existing `.ai/` dirs are an *import source*, never a destination. |
| Never *write* `~/.config/trex-guard` | That is the live guard config, shared read-only by `tg` and `tgr`. A third tool editing it can corrupt the guards. `tgw` never reads it either. |
| `TCX_HOME` exists | Termux and a proot/chroot have different `HOME`s, which would silently create two stores. One absolute path fixes that. On this machine all three XDG vars are unset, so the fallback is the normal path, not an edge case. |

### Project identity

Identity answers "which store does this project own". It is derived from the
repository, **never** from the working-tree path, so worktrees, clones and moved
directories resolve to the same store.

- Identity = the **sorted set** of root commits reachable from `HEAD`.
- Empty repo or shallow clone → **provisional** `p-<fnv1a64>`, because a
  shallow "root" is just the boundary commit and changes on every `--deepen`.
  A provisional store is renamed into place when a real root appears; if both
  exist, both are kept and nothing is auto-merged.
- A set, not one root, because an orphan branch (`gh-pages`) has its own root
  and would otherwise flip the identity when you switch branches.
- **No git at all** → identity is the directory itself (each subdirectory gets
  its own store). `tcx id --human` shows which case you are in.

---

## 5. Notes: the format and the discipline

One note is one line:

```text
<date> | <kind> | <text> | files: <names> | dir: <path> | [keywords]
```

- Only `date`, `kind`, `text` are required. Empty fields are omitted.
- `kind` is `decision | gotcha | open`. Unknown values are preserved verbatim.
- `dir:` attaches judgment to a directory so it shows inside `tcx map`. Write
  it with `tcx note -d PATH`, or `--dir PATH` (spell it long: `-d` means
  `--depth` on `map`).
- Control characters and newlines are collapsed to spaces: a note is pasted
  into a terminal *and* a chat, so a note must never be able to break either.
- Each note is a single `O_APPEND` write, so concurrent writers cannot
  interleave. Notes are never rewritten; a correction is a new note.
- The date comes from the system clock. **Never invent a date.**

### What belongs in the store, and what must never go in

| Belongs (judgment `rg` cannot rebuild) | Never belongs (rebuildable, or rots) |
| --- | --- |
| a decision and its reason | SEEN/INFERRED ledgers |
| a gotcha that surprised you | line ranges, symbol outlines |
| an open question | file listings, directory trees |
| one-line judgment per directory (`-d`) | commit hashes, "current state" snapshots |

The test is: *would `rg` give this back in a second?* If yes, do not store it.

---

## 6. `tcx map`: generated structure, stored judgment

`tcx map` answers "where does everything live" without dumping files.

| Part | Source | Can go stale |
| --- | --- | --- |
| directories and files | walked live from disk at render time | no |
| one-line judgment per directory | notes with a `dir:` field | yes — but dated |

Nothing about the tree is stored. That is what makes the split safe: a moved
file breaks nothing, because there is no stored tree to break.

```bash
# Paths are from another project; only the commands matter here.
tcx note -k gotcha -d app/src/.../miniplayer "NOT the full player; Open full player hands off to PlayerActivity"
tcx map --depth 2
tcx map --under app/src/.../ui/browser --depth 1
```

Output shape:

```text
# map
depth 1 tcx

  miniplayer/  - NOT the full player; Open full player hands off to PlayerActivity
      2026-11-16: morphing sheet; state in MiniPlayerStateManager
  networkstreaming/
```

- `--depth` counts directory levels **from the node being mapped** (default 2).
  `--under PATH` maps one subtree and counts from there, so a deeply nested
  path is reachable at depth 1.
- Generated and vendored trees are skipped: `target`, `node_modules`, `build`,
  `dist`, `out`, `vendor`, `tmp`, `RCS`, `CVS`, `__pycache__`, `.venv`, `venv`,
  `.tox`, `.mypy_cache`, `.pytest_cache`, `.next`, `.nuxt`, `coverage`, `.cxx`,
  `.gradle`, `.idea`, `.vscode`. Hidden entries are skipped too, except
  `.github`, `.gitlab-ci.yml` and `.tg`.
- A directory of only data files with more than 8 entries collapses to
  `(N data files)`; forty numbered JSON migrations teach nothing about where
  code lives.
- Judgment on a directory that no longer exists prints under
  `## no longer in the tree`. It is the one thing here that cannot be
  regenerated, so it is never silently dropped. Only `dir:` notes appear in the
  map at all; ordinary notes are for `find` and `resume`.
- Output is deterministic for a given tree and note set.
- `--under` naming something that is not a directory is an **error** (stderr,
  exit 1), not an empty map. It used to print the message to stdout and exit 0,
  which made a typo indistinguishable from a real answer.

---

## 7. Command surface, and what each command is for

```text
tcx id       resolve identity + store path
tcx note     append / list dated notes
tcx find     search notes; -k scopes by keyword, --tags lists them
tcx prime    budgeted, paste-ready briefing
tcx import   import an existing .ai/ directory (never modifies it)
tcx fresh    prompt with ## Project filled from this repo
tcx resume   fresh plus what previous sessions learned
tcx map      shallow tree annotated with directory notes

tgw run      watch the clipboard until stopped
tgw once     one iteration, for testing
tgw approve  re-check and run a held block
tgw deny     discard a held block
tgw status   mode, auto-lock state, pending holds
tgw log      recent verdicts
tgw doctor   prove clipboard, gate and notify work
tgw check-config  validate watch.ini
tgw cd       write the cwd used by cwd_mode = file
```

Flags not shown above, all discoverable with `tcx <command> --help`, all checked
against the parser by the test suite so this list cannot silently rot:

| Flag | On | Meaning |
| --- | --- | --- |
| `-f, --file FILE` | `note` | file the note is about; repeatable |
| `-t, --tag TAG` | `note` | keyword for scoped search; repeatable |
| `--list` | `note` | print every note instead of appending |
| `-b, --budget BYTES` | `prime`, `resume` | byte cap on notes (default 6000); `fresh` recognises it and refuses it, because `fresh` prints no notes |
| `-T, --template FILE` | `fresh`, `resume` | use this prompt instead of the config copy |
| `-p, --path PATH` | `note`, `find`, `prime`, `import`, `fresh`, `resume`, `map` | resolve this project instead of the current directory |
| `--tags` | `find` | list keywords with counts |
| `-k, --kind KEYWORD` | `find` | only notes carrying this keyword; repeatable |
| `-d, --dir DIR` | `import` | source `.ai/` directory |

`prime` is notes-only (a freshness stamp plus the notes, budgeted); `resume` is
the prompt *including* those notes. For starting a chat, ask for `tg resume`.
`prime` exists for when the notes are wanted without the prompt.

The budget skips whole notes rather than truncating them: what does not fit is
replaced by `… N more (tcx find)`. So a few long notes can crowd out many short
ones — write notes tight, and one note should carry one fact.

Exit codes are a stable interface: `0` success, `1` runtime error,
`2` usage error. Bare `tcx` is a usage error, not success.

`--help` exists for the top level and for every command, all rendered from one
table in `crates/tcx/src/help.rs`. (`--version` is the one exception: it is a
version line, not part of the table.) `crates/tcx/tests/help.rs` enforces both
directions against the real binary:

- documented → accepted: every flag in the table is run, and must not be
  rejected as unknown;
- **accepted → documented: the parser arms in `main.rs` are read out of the
  source, and every flag they accept must appear in the table.**

**If you add a flag, add it to that table or the tests fail.** That is intended,
and it is not theoretical: `note --dir` shipped parsed, working, and absent from
`--help`, because only the first direction was checked. `prime` and `import`
were missing their `--path` the same way.

Most commands take an optional trailing PATH and resolve the project from the
current directory when it is absent. Run from the repo root and identity
resolves. Two exceptions, both positional traps:

- **`note` has no positional PATH** — it takes `--path`. A trailing `.`, `..`,
  or a token containing `/` that exists is rejected with exit 2 rather than
  stored as note text. A bare directory *name* (`tcx note "text" src`) is
  **not** caught and still becomes note text: never omit the closing quote of
  the note and leave a path after it.
- **`import`'s positional argument is the source directory**, not the project
  (`tcx import foo` imports `foo/`, it does not select project `foo`). The
  project comes from `--path`.

Two flags are overloaded and this is a real trap:

- `-k` is `--kind` on both `note` and `find`, but means different things:
  a note *kind* (`decision|gotcha|open`) on `note`, and a *keyword* on `find`.
- `-d` is `--dir` on `note` and `import`, but `--depth` on `map`.

Spell them long (`--kind`, `--dir`, `--depth`) and the ambiguity disappears.

---

## 8. Why the pieces are separate

### 8.1 Why the guard is separate from `tcx`

`tgr` is a security tool. A security tool that also indexes files has a larger
attack surface and a weaker story. Splitting keeps it auditable, keeps "the
guard is tested and unchanged" a true statement, and means a regression has one
possible cause, not two. The dependency is one-way: `tg` may call `tcx`; `tcx`
never imports a guard.

**History, because it explains the layout.** `tgr` was written as a Rust port
of a C++ guard while that guard sat beside it, so the original could keep
protecting the terminal while every paste was compared against both. The rule
throughout the port was "match the C++ guard exactly, including its quirks;
change only what is provably wrong, and write down every difference." Those
five differences are in `docs/tgr.md`, and they are not all the same kind of
thing:

- **stricter about pastes** — 2 and 3 hold pastes at `Review` that the C++ guard
  would have let through, because a rule could not run;
- **an isolation choice** — 1 keeps the two guards' sessions separate;
- **no behaviour change** — 4 and 5 trade a fatal error and a signal-handler
  re-raise for a default and a normal unwind.

Do not read "deliberate difference" as "more cautious". Differences 2 and 3 are
the only ones that change what the human sees.

**The C++ guard was removed on 2026-11-17**, and `tgr` is now the only guard;
`tg` is a launcher that runs it. So there is no second implementation to compare
against, and the parity tests that did so are gone. What survives is the case
table those comparisons were built on: `crates/tgr/tests/cpp_suite.rs`, kept as
data. **It is now the specification rather than a copy of one** — which is
exactly why it was kept as data when the source was deleted. Do not "fix" a
failure there by changing the expectation.

**The marker is still `TGR_ACTIVE`, and `TREX_GUARD_ACTIVE` is still checked.**
The guarded child gets `TGR_ACTIVE=1` from `tgr`; `scripts/tg` refuses to start
when `TREX_GUARD_ACTIVE=1`, and `tgr` never sets that variable. That separation
was what let `tg` run inside a `tgr` session while both existed, and it is
still tested, because a launcher that thinks it is nested will refuse to start.

**Config is read, never written, and by one program now.** The human's custom
rules apply to the running guard, so `tgr` must read `config.ini`. It only ever
reads it. The §4 rule that `tcx` must never touch `~/.config/trex-guard` still
stands and is about *writing*.

### 8.2 Why config is never read from the repository

If `tg` read its allowlist from the repo, a cloned repo could ship an
`[allowlist]` entry whitelisting `rm -rf /`. The same logic applies to the
prompt: a checkout must not be able to change the rules an AI is given. This is
why the template lookup never consults the repo and why no `.tg/prompt.md` is
shipped.

Note the near-collision in names: `tcx` writes its own `tcx/config/` directory,
and one *legacy* template name inside it is `tg/prompt.md`. Neither is the
guard's `~/.config/trex-guard/config.ini`, which `tcx` never reads or writes.
Same prefix, different files, kept apart on purpose.

### 8.3 Why the prompt is a file, not a binary

The rules change far more often than the tool is rebuilt. A rule edit must cost
one file save, not a compile.

### 8.4 Why the tree in `map` is generated

Stored structure rots on the first `git mv`; generated structure cannot. Stored
judgment does not rot on a move, it merely becomes dated. Each half stores only
what the other cannot produce.

### 8.5 Why `tcx` has no dependency beyond `std`

`tcx` builds with no external crates. It cross-compiles and installs trivially,
and it has no supply chain for a security-adjacent tool. Revisit only when a
real need appears.

### 8.6 Why `tgr` does have dependencies

`tgr` is not `tcx`, and the difference is worth stating because it looks like an
inconsistency. A guard needs two things `std` does not provide well: a regex
engine with the same behaviour as C++ `std::regex`, and safe POSIX PTY calls
(`forkpty`, `termios`, `ioctl`, signals).

- `fancy-regex` — the closest equivalent to `std::regex`. **One engine for
  everything**, built-in and user rules alike, so there is one behaviour to
  reason about rather than two. It backtracks, which is why every user regex is
  given a backtrack limit and a limit hit fails closed (`Review`, never `Safe`).
- `nix` — wraps the PTY syscalls. Hand-rolling `forkpty` and `termios` in a
  security binary is more unsafe code than the wrapper saves.

That is the whole dependency list, and it is pinned in `crates/tgr/Cargo.toml`.
Adding a third crate to a guard needs a reason as good as these.


---

## 9. Mistakes already made — do not repeat them

These are not hypothetical. Each one happened in a session that built `tcx`.

### 9.1 Designed for the wrong user

`tcx note` was discussed as if the human would type it after a session. Wrong.
You write it; the human only runs blocks that already exist.

### 9.2 Invented a measurement ritual

A "5-session trial" was proposed: count orientation turns, log them in a tally
script, compare averages. The human's answer, correctly: they would know within
a week of ordinary use, and the ritual is busywork. The script was deleted.

**Rule: do not invent experiments to validate something daily use answers for
free.** If a tool is used every day, ship it and let use be the test.

### 9.3 Explained instead of building

Asked how to test, the answer was five steps of procedure. What was wanted was
the next working command. When the human says "continue", "do it" or "start",
they mean write the code, not write the instructions.

### 9.4 Assumed commands the human types

Any interface where the human composes arguments at a prompt is suspect. The
real interface is one command, no arguments, output on the clipboard.

### 9.5 Over-asked before building

Repeatedly stopped to ask questions whose answers were already in the workflow
prompt the human pasted. Read the pasted prompt first; ask only about what it
genuinely does not say.

### 9.6 Documented a flag that did not exist

The prompt told the AI to use `tcx find -k <keyword>`, but `find` had no `-k`:
every scoped lookup would have exited 2. The general failure mode: **the prompt
is configuration, and configuration that names a command must be tested against
the binary.**

The flag was added, but it was spelled `--kind` (consistent with `note`) while
it takes a *keyword*. That collision survives; see §7.

### 9.7 Checked one direction and called it "cannot drift"

The help tests proved every *documented* flag exists, and the comment called it
"both directions". The missing direction — every *accepted* flag is documented —
let `note --dir`, `prime --path` and `import --path` ship working and invisible.
Found by having a fresh reader simulate a new session against the doc.

**Rule: when a test claims to cover both directions, break each direction on
purpose and watch it fail.** A test that passes is evidence only of what it was
run against.

### 9.8 Trusted a design without running it on a real tree

These were found only by running against a real project, never by tests on
synthetic fixtures:

- `root.join(":app")` resolves to `/app` — the leading colon from Gradle's
  `include(":app")` must be stripped.
- The `## Project` heading search started *after* the heading, so filling it
  deleted the section body.
- `tcx map --under <deep path>` was unusable because depth was measured from
  the root; the subtree was looked up inside an already-shallow tree.

**Rule: run new output on a real repository before declaring it done.**

### 9.9 Let a stray argument silently corrupt data

`tcx note "text" .` stored the `.` as part of the note text: `note` is the only
command with no positional PATH, so the habit the other commands teach corrupts
the note. A trailing `.`, `..`, or existing path is now rejected with exit 2 and
a `--path` hint.

### 9.10 Let documentation drift

`docs/tcx.md` still called the abandoned 5-session trial "the next thing to
do" after it was rejected, and `scripts/tcx` carried its own stale copy of the
help. Both were removed. **If a decision changes, grep the docs for the old
wording in the same session.**

### 9.11 Ported from memory instead of from the source

*From when the C++ guard existed; the source is now in the repository history.*

The C++ guard was 1858 lines and every branch of it decided whether a command
is *seen*. A "cleaner" Rust rewrite might handle one more quoting case and stop
catching a paste the original caught. The port was therefore made by reading
all of `src/` first, branch by branch, including the quirks — `||` ends a
pipeline but a bare `|` continues it, an unterminated quote runs to the end,
`git reset --harder` must stay Safe.

**Rule: for a behaviour-preserving port, the original source is the spec for
*what to write*. Read it before writing, and keep the quirks, because the quirks
are behaviour.** It is not the spec for *whether you succeeded* — see §9.12.

### 9.12 A passing test proved only that it matched my reading

`cpp_suite.rs` holds the cases from the C++ guard's `tests/test_rules.cpp`
verbatim. They passed, and that proved nothing about whether the port matched
the *binary* — it proved the port matched my transcription of the test file. The
real check was running the shipped `trex-guard` and `tgr` on the same pastes
through a pty and diffing verdicts (`parity.rs`, `parity_real_config.rs`, both
since removed with the C++ guard).

So the two rules stacked, they did not compete: **read the source to write it
(§9.11), then compare against the running binary to believe it (§9.12).** Source
reading says what to build; only the binary can say whether you built it. Same
failure as §9.7, from another direction.

With the second implementation gone, this comparison can no longer be made, and
that is the cost of deleting it. What remains is the case table — which is why
it was kept as data. Any future claim that "the guard still behaves the same"
rests on that table and on `check_parity` (that `--check` agrees with the paste
path), not on a second opinion.

### 9.13 Four times the harness lied, not the product

Every one of these looked like a `tgr` defect. None was:

- a generated Python script contained Rust syntax (`Some(...)`), so the helper
  died before printing anything;
- the pty driver **re-sent the paste** once the shell was back at its prompt,
  which looked like a hang;
- the pty driver **wrote the paste before bash had finished starting**; the
  terminal buffered it and bash read it as ordinary typing, so it was never
  captured as a bracketed paste and nothing was classified. The end-to-end
  tests had been **passing on that race** the whole time;
- a test expected a review prompt for `touch`, which neither guard flags. The
  test asserted a rule that does not exist.

The third is the one to remember. It was found by running the freshly
installed binary by hand and seeing every paste echoed raw — which looks
exactly like a guard that stopped working. The suite was green; the suite was
green for the wrong reason.

**Rule: when a test fails, establish whether the product or the harness is
lying before changing either. And a green suite is not evidence until you have
seen the thing it claims to observe actually happen.** The harness is code
too, and it must wait for what it reports on.

---

## 10. What "clear" looks like

Before proposing anything, be able to state:

1. **Who runs it** — always the human, always a block you wrote.
2. **When it runs** — once per chat (`fresh`, `resume`), or inside an existing
   block (`note`, `map`, `find`).
3. **What the human does with the result** — pastes it into chat, nothing else.
4. **Whether it removes a step or adds one.** If it adds one, it is probably
   wrong.

---

## 11. Where to go next

| You need | Read |
| --- | --- |
| the full design record, every decision and its measurement | `docs/tcx.md` |
| why `tgr` exists, its deliberate differences, and how it is verified | `docs/tgr.md` |
| dated history of each slice and what it fixed | `docs/development-log.md` |
| the guard's PTY model and security boundary | `docs/architecture.md` |
| guard config syntax | `docs/configuration.md` |
| install, shell integration | `docs/install.md` (§11, "Releasing") |
| the live rules an AI follows | your prompt template (§3) |

### Building and testing

```bash
cargo test                              # tcx, tgr and tgw
cargo test -p tcx                       # tcx alone (147 tests, fast)
cargo test -p tgr --lib --test cpp_suite --test pty_e2e   # tgr, skipping the slow corpus tests
cargo build --release -p tgr            # target/release/tgr
cargo build --release -p tgw            # target/release/tgw
./tests/install_test.sh                 # the installer, against a fake release
```

The pty suites (`check_parity`, `watch_rules`, `pty_e2e`) drive the real guard
through a terminal: every paste starts a shell. They are the only tests that
would have caught the exit and SIGTERM hangs, so run the full `cargo test`
before committing anything that touches `crates/tgr/src/rules.rs`, `shell.rs`
or `config.rs`.

The C++ guard was removed on 2026-11-17. Its acceptance cases survive as data
in `crates/tgr/tests/cpp_suite.rs`, which is now the specification, and the
`parity` tests that drove the C++ binary went with it. The old implementation
is in the repository history.

### Releasing

Releases ship **only** the Rust tools, as `musl` static binaries for
`aarch64` and `x86_64`. There is no source build and no C++ any more.

1. Bump `version` in the root `Cargo.toml`. All three crates inherit it via
   `version.workspace = true`, so this one line is the whole version. A release
   whose binaries report the wrong number cannot be debugged from a bug report;
   this was wrong once (binaries said 0.1.0 while the tag said 0.2.1).
2. Push to `master` and let `.github/workflows/ci.yml` go green on **both**
   architectures. Do not tag a red commit.
3. Push a `v*` tag on that green commit. `.github/workflows/release.yml` builds
   both musl binaries, checks they are genuinely static with `readelf` (a
   `INTERP` segment means dynamic), and publishes with a `SHA256SUMS` asset.
4. Install it the way a user does, from GitHub, and check the versions:

```bash
curl -fsSL https://raw.githubusercontent.com/estiaksoyeb/trex-guard/master/scripts/install.sh | sh
tg --version && tgr --version && tgw --version && tcx --version
```

`scripts/install.sh` downloads only from GitHub Releases, verifies the checksum
before installing anything, never builds from source, and never overwrites an
existing `~/.config/trex-guard/config.ini`. `tests/install_test.sh` checks all of
that against a fake release and needs no network.

`tg --version` and `scripts/tcx --version` exec the binary rather than printing
a literal. Both printed a hardcoded `0.1.0` once; do not reintroduce that.

### Repository layout

```text
tests/                installer test (the C++ guard's own tests were removed)
crates/tcx/src/       tcx: id, store, notes, prime, find, import, project,
                      briefing, map, help
crates/tcx/tests/     integration tests, including help drift detection
crates/tgr/src/       tgr: risk, config, shell, rules, pty, main
crates/tgr/tests/     cpp_suite (the specification), check_parity, pty_e2e,
                      watch_rules, support/*.py drivers
scripts/tg            launcher; runs tgr, forwards fresh/resume to tcx
scripts/install.sh    installs a GitHub release, verifies its checksum
scripts/tcx           tcx launcher (delegates --help to the binary)
                      (there is no scripts/tgr: tgr is the binary itself)
scripts/tcx-integration.bash   sets TCX_HOME, defines tcx-clip
docs/                 this file and the records above
```

