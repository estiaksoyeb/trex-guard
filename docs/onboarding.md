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

`trex-guard` is two binaries in one repository:

- **`tg`** — TREX Guard, a frozen C++17 PTY paste-safety firewall. It
  intercepts pasted shell text *before* it reaches the shell and halts
  high-risk pastes for review. Sources in `src/`, built by CMake.
- **`tcx`** — TREX ConteXt, a Rust binary that keeps AI-assisted context
  *outside* the repository: decisions, gotchas, open questions, and a shallow
  annotated map of the tree. Sources in `crates/tcx/`, built by Cargo.

They share a repository and nothing else. That is deliberate (see §8.1).

`tg` and `tcx` are not two halves of one feature. `tg` protects the terminal
from a bad paste. `tcx` answers "what should a new chat know before it starts".
The only place they touch is that `tg fresh` / `tg resume` are forwarded to
`tcx`, in one direction only.

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
human:  tg                        # fresh terminal -> guard active
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
| Never share `~/.config/trex-guard` | That belongs to `tg` and holds the live guard config. Two tools editing one config file corrupts both. |
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

### 8.1 Why `tg` stays frozen

`tg` is a security tool. A security tool that also indexes files has a larger
attack surface and a weaker story. Splitting keeps it auditable, keeps "`tg` is
tested and unchanged" a true statement, and means a regression has one possible
cause, not two. The dependency is one-way: `tg` may call `tcx`; `tcx` never
imports the guard.

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

### 8.5 Why no dependency beyond `std`

`tcx` builds with no external crates. It cross-compiles and installs trivially,
and it has no supply chain for a security-adjacent tool. Revisit only when a
real need appears.

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
| dated history of each slice and what it fixed | `docs/development-log.md` |
| the guard's PTY model and security boundary | `docs/architecture.md` |
| guard config syntax | `docs/configuration.md` |
| install, shell integration | `docs/install.md` |
| the live rules an AI follows | your prompt template (§3) |

### Building and testing

```bash
cargo test                  # all tcx tests
cargo build --release       # target/release/tcx
cmake -B build && cmake --build build   # the guard, independent
```

The two builds are independent. `Cargo.toml` at the repo root is a separate
workspace that happens to share the directory with the CMake project.

### Repository layout

```text
src/                  tg, C++17 guard (headers sit beside their .cpp)
tests/                guard tests (C++)
crates/tcx/src/       tcx: id, store, notes, prime, find, import, project,
                      briefing, map, help
crates/tcx/tests/     integration tests, including help drift detection
scripts/tg            launcher; forwards fresh/resume to tcx
scripts/tcx           tcx launcher (delegates --help to the binary)
scripts/tcx-integration.bash   sets TCX_HOME, defines tcx-clip
docs/                 this file and the records above
```
