# TREX ConteXt (`tcx`) — Design & Implementation Record

> Status: **v1 in progress**. This document is the authoritative record of the
> decisions made while building `tcx`. It exists so a fresh session can pick up
> without re-deriving anything.

## 1. What `tcx` is

`tcx` is the **out-of-repo context store** for AI-assisted development on large
repositories. It answers one question cheaply:

> Given this project, what should a new chat session know before it starts?

It stores two fundamentally different kinds of data, and never confuses them:

| Kind | Rebuildable? | Lives in | Written by |
| --- | --- | --- | --- |
| **Notes** (judgment) | No | data dir | humans / AI, append-only |
| **Index** (derived) | Yes | cache dir | `tcx` itself |
| **Config** | No | config dir | humans |

## 2. Relationship to `tg` (the guard)

`tcx` is **not** part of `tg`, and `tg` is **not** part of `tcx`.

- `tg` (TREX Guard) is the frozen C++17 PTY paste-safety firewall. It stays
  exactly as it is. No behaviour change, no rewrite folded into this work.
- `tcx` is a separate Rust binary in the same repository (Cargo workspace).
- **Dependency direction is one-way**: nothing in `tg` may ever import `tcx`.
  The guard must not grow an indexer's attack surface.

### Why the split is mandatory

1. **Trust dilution.** A security tool that also indexes files has a larger
   attack surface and a weaker security story. Separation keeps `tg` auditable.
2. **The proven-core claim.** "`tg` is tested and unchanged" is only true if
   `tg` is untouched.
3. **Two changes at once destroys diagnosis.** A Rust port of `tg` and a new
   context tool landing together means a regression has two possible causes.

### Why the guard's config must not live in the repo

The strongest argument for out-of-repo state is **security**, not tidiness:

> If `tg` read its allowlist from the repository, then a cloned repo could ship
> an `[allowlist]` entry that whitelists `rm -rf /`. Config must be outside the
> repo so a repo cannot weaken its own guard.

## 3. Naming

| Name | Meaning | Binary |
| --- | --- | --- |
| **TREX** | Umbrella | — |
| **TREX Guard** | The paste-safety guard | `tg` |
| **TREX ConteXt** | The context store | `tcx` |

Rejected names and why:

- `trex` — collides with Cisco TRex (traffic generator) and T-Rex (miner);
  implies "the whole thing".
- `primer` — reads oddly as a command.
- `tmap` — undersells `note`/`prime`, which are the actual product.
- `tc` — collides with Linux traffic control.

`tcx` was checked locally: `command -v tcx` and `apt-cache search --names-only
'^tcx$'` are both empty. **Still to check:** crates.io, PyPI, Homebrew. Fallback
if a collision appears: `tctx`.

## 4. Storage layout

Split by **data kind**, never one dot-folder:

```
$XDG_DATA_HOME/tcx/projects/<id>/notes.md     # judgment, irreplaceable
$XDG_CACHE_HOME/tcx/projects/<id>/...         # index, rebuildable
$XDG_CONFIG_HOME/tcx/config.ini               # config
```

`TCX_HOME` overrides all three and points at a single root containing `data/`,
`cache/` and `config/`.

### Why `TCX_HOME` matters (verified)

All three XDG variables are **unset** on the development machine
(`XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_CONFIG_HOME` all empty;
`HOME=/root`). So the `~/.local/share`, `~/.cache`, `~/.config` fallback is the
**normal path**, not an edge case.

Termux and a proot/chroot have **different `HOME`s**, which means two silently
separate stores. `TCX_HOME` is the fix: set it once in shell integration so
there is exactly one canonical store regardless of environment.

### Never share with `~/.config/trex-guard`

That directory belongs to `tg` and holds the live guard config (22 active custom
rules at time of writing). `tcx` must never write there.

## 5. Project identity

Identity answers: **which store does this project own?**

It is derived from the repository, **never** from the working-tree path, so
worktrees, clones and moves resolve to the same store.

### The rule

1. Resolve the git root (`git rev-parse --show-toplevel`). No git → the
   canonical starting directory is the identity (`dir-<fnv1a64(path)>`).
2. Compute the **set** of root commits reachable from `HEAD`
   (`git rev-list --max-parents=0 HEAD`), **sorted**.
3. Identity = the chosen root commit. Roots are kept as a set, not a single
   hash, because a repository can have more than one.

### Provisional identities

Two cases cannot produce a real root and are marked **provisional**:

| Case | Why | Detection |
| --- | --- | --- |
| **Empty repo** | No commits; `rev-parse HEAD` fails. | `git rev-parse --verify --quiet HEAD` fails |
| **Shallow clone** | The visible "root" is only the boundary commit. | `git rev-parse --is-shallow-repository` |

Provisional stores are named `p-<fnv1a64(canonical git-common-dir)>`. The
common dir is used because it is stable across worktrees and subdirectories.

#### Why shallow must be provisional (verified)

`rev-list --max-parents=0 HEAD` in a shallow clone returns the **boundary
commit**, which looks parentless. Measured on the development machine:

```
depth 1 root: 2e52997d...
depth 3 root: 4bb627ec...
src    root: 33cdaf86...
after --deepen 1: d6704ae2...
```

Every depth change yields a different "root". Using it as identity would fork
the store on every `--depth`, `--deepen` and `--unshallow`. So it is never
treated as a real root.

### Orphan branches / unrelated histories (verified)

`rev-list --max-parents=0 HEAD` depends on the checked-out branch. A `gh-pages`
orphan branch has its own root:

```
main branch roots:   04e39d32...
orphan branch roots: 1aba09a1...
```

So identity **flips with the branch** unless handled. Rules:

- Use the **set** of roots, sorted.
- If **any** root already has a store, that root wins.
- If several do, warn and use the first sorted.
- Otherwise create under the first sorted root.

The identity string is **opaque**: nothing parses it, so SHA-1 vs SHA-256 does
not matter. (The dev repo is SHA-1.)

### Provisional → real migration

A provisional store must not orphan its notes when the repo gains its first
commit (or is unshallowed):

- If `p-<hash>` exists and `<real-id>` does **not**: `rename()` it into place
  (atomic on one filesystem; both live under the same data root).
- If **both** exist: do **not** auto-merge. Warn, keep both, defer `--adopt`.
  (Two clones of the same repo can legitimately produce both.)

## 6. Notes format

Append-only, one line per note, matching the `.ai/` convention so
`tcx import .ai/` is a copy rather than a conversion:

```
<date> | <kind> | <text> | files: <names> | [<keywords>]
```

- `date` (`YYYY-MM-DD`), `kind` and `text` are required; the rest is omitted
  when empty.
- `kind` is `decision | gotcha | open`. Unknown values are **preserved
  verbatim** so a newer `tcx` is not mistaken for corruption.
- Control characters and newlines in `text` are collapsed to spaces, so a note
  can never break the one-line format. This matters because notes are pasted
  into conversations.
- Each note is a **single `write`** to a file opened with `O_APPEND`. Appends
  below `PIPE_BUF` are atomic on Linux, so concurrent writers cannot interleave.
  Notes are never truncated or rewritten.
- `prime` prioritises **gotchas**, then open, then decisions newest-first.

## 7. Security boundary (the part that is easy to miss)

`tcx`'s output is pasted into a **terminal** and a **chat**. Anything derived
from the repository is therefore two channels at once:

1. **Escape-sequence injection** — a filename containing `\x1b[2J` could clear
   the screen.
2. **Prompt injection** — a filename or symbol that reads like an instruction
   could steer the AI.

**Rule:** strip ESC and all control characters, and cap string lengths, on
anything repo-derived before it is printed. This applies to `prime`, `find`,
and any future surface.

## 8. Regex safety (applies to `tg`'s future port, not to `tcx` v1)

An earlier claim — "`std::regex` ports directly to the `regex` crate" — was
**wrong** and is retracted.

- The `regex` crate is RE2-derived and supports **neither backreferences nor
  lookaround**.
- Built-in `tg` rules use only `\b`, `\s`, alternation, groups and char classes
  → they compile under `regex`.
- **User `[custom_rules]`** may legitimately contain `(\w+)\s+\1` or a
  lookahead → `regex` rejects them. Use `fancy-regex` for user rules.

### Verified backtracking hazard

`fancy-regex 0.13.0`, pattern `(a|aa)+\1$`, release build:

```
n=14  unbounded    541.847µs
n=18  unbounded      3.672ms   bounded 1.260ms  BacktrackLimitExceeded
n=22  unbounded     24.630ms   bounded 1.264ms  BacktrackLimitExceeded
n=26  unbounded     62.617ms   bounded 1.247ms  BacktrackLimitExceeded
```

**Gotcha:** the textbook blowup patterns (`(a+)+$`, `^(a*)*$`,
`^(a|aa|aaa)*$`) did **not** trigger this engine (sub-millisecond to n=34).
Only the **backreference** path did. A regression test must therefore use a
backreference-bearing pattern, or it will pass vacuously.

**Mitigation for the guard port:** set `backtrack_limit`; on
`BacktrackLimitExceeded` treat the paste as `REVIEW` (never `SAFE`) and warn.
A compile failure on a user rule must also fail loudly, never silently drop the
rule.

The live config was checked: 22 active custom rules, **zero** use backreferences
or lookaround. So the hazard is user-rules-only, not built-ins.

## 9. Command surface

| Command | Status | Purpose |
| --- | --- | --- |
| `tcx id` | **done** | Resolve identity + store path |
| `tcx note` | **done** | Append / list dated notes |
| `tcx prime` | **done** | Budgeted, paste-ready session briefing |
| `tcx find` | **done** | Search notes; `-k` scopes by keyword, `--tags` lists them |
| `tcx import` | **done** | Import an existing `.ai/` directory |
| `tcx fresh` | **done** | Prompt with `## Project` filled from this repo |
| `tcx resume` | **done** | `fresh` plus what previous sessions learned |
| `tcx map` | deferred | Project map |
| `tcx mcp` | deferred | MCP server, behind a cargo feature |

`ctx` was renamed to `find` because `tcx ctx` stutters.

### Usage

```text
tcx id [--human] [PATH]
tcx note [-k KIND] [-f FILE]... [-t TAG]... TEXT
tcx note --list [PATH]
tcx prime [--budget BYTES] [PATH]
tcx find [-k KEYWORD]... [--tags] [QUERY...] [PATH]
tcx import [--dir DIR] [PATH]
tcx fresh [--template FILE] [PATH]
tcx resume [--budget BYTES] [PATH]
```

`tg fresh` and `tg resume` are the user-facing spellings; `tg` passes them
straight to `tcx`. They are the two commands run once per chat (see
`docs/workflow.md`). `fresh` prints the prompt with the `## Project` section
filled from the repository; `resume` appends the store's notes under
`## What previous sessions learned`, in `prime`'s order and budget.

`import` defaults to `<project root>/.ai` and **never modifies the source
directory**. It deduplicates on re-run, so importing twice is safe.

### Exit codes (stable interface)

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | runtime error (missing path, no store) |
| 2 | usage error (unknown command/option, no arguments) |

Bare `tcx` is a **usage error (2)**, not success.

### Help

`tcx --help` lists every command, its usage, the exit codes and the environment.
`tcx <command> --help` (or `-h`) prints that command's usage, options, defaults
and warnings. `--help` is answered before the command parses anything, so
`tcx note --kind gotcha --help` shows the note help rather than consuming
`--help` as a value.

Both are rendered from one table in `src/help.rs`. A second copy of the command
list is a copy that goes stale, so `scripts/tcx` no longer has its own help
text; it passes `--help` to the binary.

`tests/help.rs` keeps the table honest in both directions:

| Test | Catches |
| --- | --- |
| every documented command has help | a command in the table that the binary does not answer |
| every command the binary accepts is documented | a dispatcher arm added without a table entry |
| every accepted flag is documented | a flag in the docs that the parser rejects |
| help wins over other arguments | `--help` being swallowed as an option value |
| shell wrapper delegates | help text duplicated in `scripts/tcx` |
| top-level lists commands and exit codes | a command or exit code missing from the summary |

Each was verified by deliberately breaking the code and watching the test fail:
a dispatcher arm without a table entry, a documented flag that does not exist,
and `--help` handling disabled.

### `tcx id` output

Machine-readable (default), opaque id first so it can be captured:

```
<id> <kind>\t<root>
```

Human (`--human`) uses stable labels: `id:`, `kind:`, `root:`, `common-dir:`,
`roots:`, `store:`, `provisional:`, and `adopted:` when a migration happened.

## 10. `prime` design (implemented)

- v0 **must differ** from a plain notes dump, or it is no better than
  `.ai/INDEX.md`. It prints:
  1. a **stamp** (`HEAD`, branch/detached, dirty count, `shallow` when
     applicable),
  2. the **notes**, budgeted.
- A shallow tree may be added **only** after `prime` is measured, computed on
  the fly (<100 ms, no cache), which makes freshness moot until there is a
  measured reason to cache.
- Budget in **bytes** (default 6000), not tokens.
- Truncation order: gotchas → open → decisions newest-first → tree.
- Ends with an explicit `… N more (tcx find)` line.
- Filenames/symbols are **not** printed in `prime`; `find` annotates
  individual files `[dirty]` instead.

### Implemented output shape

```text
# tcx brief
HEAD 2afdcfb master dirty 5 tcx

2026-11-16 | gotcha | shallow clones report a boundary commit ... | [identity]
2026-11-16 | open | does prime earn its keep over .ai/INDEX.md? ... | [prime]
2026-11-16 | decision | notes in XDG data, index in XDG cache ... | [storage]
```

The stamp deliberately carries **no paths**: it is pasted into a chat.

### CSI sanitisation (implemented, and why it is two-layered)

`notes` already strips ESC when a note is written, so `prime` normally sees clean
text. `prime` strips escape sequences again because it may be handed text that
did not go through that path. The second pass is deliberately narrow: it only
removes a bracketed group ending in a real CSI final byte
(`m J K H A B C D f s u h l ...`), so legitimate prose such as `array[5]`,
`list[2]` and `map[key]` survives. Both behaviours are tested.

### Measured state

`prime` runs in well under 100 ms on this repository with no cache, because it
only reads a small notes file and three `git` invocations.

### Metric, defined before the trial

> **Orientation turns** from the first message to the first edit-producing step,
> compared against the last few sessions.

Timeboxed to 5 sessions. If `note` + `prime` alone do not win, the indexer was
never going to win either.

### Running the trial

Setup is one time; `scripts/tcx-integration.bash` is sourced from `~/.bashrc`
and sets `TCX_HOME` so Termux and a chroot cannot split the store.

```bash
tcx-trial log control 4      # what you do now: prompt + .ai/INDEX.md
tcx-trial log treatment 2    # prompt + `tcx prime` output
tcx-trial show               # averages + verdict
tcx-trial reset
```

Recording rules:

- **Log immediately** after the session. Memory drifts within a day.
- **Count honestly.** An orientation turn is a round trip that exists only to
  locate things: "where does X live", "show me the files", "what does this
  function do". It is *not* a turn spent designing or discussing.
- **Interleave the arms** (control, treatment, control, treatment) rather than
  doing all of one then all of the other. Learning effects are the main
  confounder: by session 4 you know the repo better regardless of the tool.
- **Use at least one repo you do not already know well.** A repo you can
  navigate from memory will show no difference.

Verdict bands: treatment ahead by >= 0.5 turn wins; behind by >= 0.5 loses; a
gap under half a turn is "no meaningful difference", which means keep `prime`
and **skip** the indexer.

## 11. Freshness (deferred until the index exists)

- **Do not** rely on mtime/size: checkouts, `cp` and proot filesystems lie.
- Use **git blob IDs** from the index for tracked files; rehash only files in
  `git status --porcelain`.
- Invalidate **per file**, not per index.
- Use a **schema/generator version string**, not a binary hash.
- Never store line numbers. Store `file + symbol`; resolve lines live.

Known failure modes to design for: untracked generated files, submodules,
sparse checkouts, index rewrites during rebase.

## 11a. `fresh` / `resume`: the prompt builder

These are the product commands. Everything else in `tcx` supports them.

### Template lookup

The prompt text lives in a file, never in the binary, because the rules change
far more often than the tool is rebuilt. First found wins:

1. `--template FILE`
2. `$TCX_HOME/config/prompt.md`
3. `<config>/tcx/prompt.md` (i.e. `~/.config/tcx/prompt.md`)

**The repository is never consulted.** A checkout must not be able to change
the rules an AI is given, which is also why the template is not installed into
the repo and no `.tg/prompt.md` is shipped. On this machine the file is
`~/.local/share/tcx-home/config/prompt.md` because `TCX_HOME` is set.

The path actually used is printed to **stderr**, and the prompt to stdout: the
program never invokes a clipboard tool. Use `tg fresh | clip` (or any other
sink) at the shell, because only the human knows where the text should go.

### Filling `## Project`

The template's `## Project` heading is a real heading line, not a mention in
prose; everything from it to the next `## ` heading is replaced with the
detected block. Filling is idempotent: running it on an already-filled prompt
does not produce two `## Project` sections.

Detection **reads** manifests and never runs a build system, so it takes
milliseconds and executes no repository code:

| Field | Source |
| --- | --- |
| name | repository directory name |
| description | `Cargo.toml` description; Gradle: Android manifest + Compose usage |
| Package | `namespace` (not `applicationId`, which carries build-type suffixes); Cargo `name` |
| Source | the directory matching `namespace` under the module, e.g. `app/src/main/kotlin/xyz/mpv/rex/` |
| Build | manifest present |
| Modules | `include(":app")` lines, or Cargo workspace members |
| type | `code` when source files exist, else `docs`, else `config` |

Multi-module repositories pick the module the shell is inside, else the one
with the most source files (the app), and list the rest.

### `resume`

`resume` is `fresh` plus one appended section, `## What previous sessions
learned`, using `prime`'s shared ordering (`prime::ordered`) and sanitiser
(`prime::sanitize_line`) and the same 6000-byte budget. That prevents the two
commands disagreeing about what matters most.

### Hardening

- Detected fields are sanitised: control characters dropped, each field capped
  at 400 characters. The output is pasted into a terminal *and* a chat.
- Gradle `:app` must have its leading colon stripped before joining the path;
  `Path::join(":app")` silently produces an absolute path that ignores the
  repository root. This was a real bug caught by testing against `mpvRex`.

## 12. Rust architecture

Single Cargo workspace at the repo root (`Cargo.toml`), members under
`crates/`. The existing CMake build is untouched and independent.

Planned crate split (only `tcx` exists today):

| Crate | Contents | unsafe |
| --- | --- | --- |
| `tg-pty` | forkpty, termios, ioctl, SIGWINCH | yes (only here) |
| `tg-classify` | parser + rules | `forbid(unsafe_code)` |
| `tg-config` | ini format, kept for compatibility | no |
| `tcx` | identity, notes, store (today) | no |
| `tg-store`, `tg-map`, `tg-query` | later | no |

Dependencies chosen so far: **none** outside `std`. `fancy-regex` is only
needed by the future guard port; `ignore` replaces `rg` for the future walk.

**Decision:** no external `rg` at runtime. Two matching paths mean two
behaviours (binary detection, multiline, `-P` lookaround). Use in-process
matching and **fail loudly at config load** on unsupported seam patterns.

## 13. Cut list (do not build in v1)

MCP/server, tree-sitter, LSP, embeddings, auto-written notes, multi-machine or
team sync, "other guards on the PTY layer", renaming `tg`, seam patterns,
`ctags`, `map`.

## 14. Test status

**127 tests passing** across eight test files plus unit tests:

| File | Tests | Covers |
| --- | --- | --- |
| `src/project.rs` (unit) | 5 | `## Project` heading detection and filling, idempotence |
| `src/briefing.rs` (unit) | 3 | fresh vs resume content |
| `tests/fresh.rs` | 13 | `fresh`/`resume` end to end: Cargo, Gradle, template precedence, budget |
| `tests/id.rs` | 19 | every identity case in the matrix below |
| `tests/notes.rs` | 19 | render, parse, sanitise, append, concurrency |
| `tests/prime.rs` | 16 | stamp, golden output, ordering, budget, CSI |
| `tests/cli.rs` | 7 | `id` output shape, exit codes, adoption, `find -k` |
| `tests/help.rs` | 7 | help coverage, drift detection, wrapper delegation |
| `tests/note_prime.rs` | 12 | `note`/`prime` end to end, worktrees, determinism |
| `tests/find_import.rs` | 18 | `find`, keyword scoping, `import`, dedupe, source immutability |
| `tests/wrapper.rs` | 3 | launcher resolution, no self-recursion |

Not covered by `cargo test`: `scripts/tcx-trial`. It is a shell tally
helper, verified by hand (log, show, reset, and both validation errors).

### Test matrix for `tcx id` (all passing)

| Case | Assertion |
| --- | --- |
| empty repo | provisional, `p-` prefix, no roots |
| empty repo in subdir | same id as at root |
| first commit | upgrades to real root |
| linked worktree | same id as main (verified manually and in tests) |
| orphan branch | different root per branch |
| worktree on orphan | does not claim the main root |
| unrelated-history merge | two sorted roots |
| shallow clone | provisional, never a boundary hash |
| shallow depth 1 vs 3 | neither identity is a real hash |
| unshallowed | becomes real, id equals source root |
| non-git directory | `dir-` identity, no common dir |
| non-git subdir | **different** id (documented behaviour) |
| non-git via `..` | same canonical id |
| missing path | `Error::NotFound` |
| provisional→real rename | `Migration::Rename` |
| both stores exist | `Migration::Conflict` |
| provisional absent | `Migration::None` |
| still provisional | `Migration::None` |

Plus CLI tests: opaque-id-first output, stable human labels, adoption preserves
notes, exit codes, version, no-`HOME` error.

Current status: **127 tests passing** (5 project + 3 briefing + 13 fresh + 19
identity + 19 notes + 16 prime + 6 CLI + 12 note/prime CLI + 17 find/import +
3 wrapper).

### Notable bugs found by tests (and fixed)

1. **Non-git subdirectories shared a store.** The first version hashed the
   starting path but a test asserted two subdirectories matched. They should
   not: without git there is no root to climb to. Behaviour is now explicitly
   "identity is the directory you are in", with a test pinning it.
2. **Fenced code content was imported as notes.** Skipping only the fence
   markers imported the shell command *inside* the fence. Fixed to skip fenced
   blocks entirely.
3. **Dated lines without a kind were flattened.** `.ai/` commonly writes
   `<date> | <text>`, which is only two fields and failed strict parsing; the
   fallback then imported the whole line as text, losing the date. Now
   recognised explicitly.
4. **CSI fragments survived ESC stripping.** The ESC byte was removed but
   `[2J` remained as visible text. A second, narrow pass removes CSI-shaped
   groups without eating `array[5]`.
5. **`tg fresh` produced no `## Project` body.** The heading was found but the
   *line-end* search ran from the heading's last character, so the first match
   was the newline ending the heading itself; the section body was replaced by
   nothing. Fixed to search from the start of the heading.
6. **A mention of the heading in prose hijacked the replacement.** Any
   occurrence of `## Project` was treated as the section. Now only a line that
   is exactly the heading counts.
7. **Gradle modules resolved to `/app`.** `root.join(":app")` treats `:app` as
   absolute and discards the repository root, so the manifest was never read.
   Caught only by testing against the real `mpvRex` tree; a hand-made fixture
   would have hidden it.
8. **The `scripts/tcx` wrapper recursed forever.** It located the binary with
   `command -v tcx`, which finds the wrapper *itself* when `scripts/` is on
   `PATH`, then `exec`'d it again. `tg` never had this bug because it looks for
   `trex-guard`, a differently named binary. Fixed by comparing resolved paths
   and skipping a self-hit; `tests/wrapper.rs` pins it with a 10 s bound so a
   regression fails instead of hanging.

## 15. Open questions

1. `tcx import .ai/` — should importing also rewrite the `.ai/` directory, or
   leave it untouched? **Decided: leave untouched**, and a test enforces it.
2. When a repo has several roots and several stores, `--adopt` semantics are
   undefined. Deferred deliberately.
3. Does `prime` earning its keep actually hold? Untested. See §10 metric.
   This is the **next thing to do**: run the 5-session trial.
4. crates.io / PyPI / Homebrew name check for `tcx`. Not yet done.
5. `find` is a plain substring match, so `root` also matches `proot`. Acceptable
   for v1 (predictable beats clever); revisit only if it causes real misses.
6. No install path yet: `cargo install --path crates/tcx` works, but there is no
   `tcx` wrapper script alongside `scripts/tg`, and no shell integration that
   sets `TCX_HOME`.

   **Resolved.** `scripts/tcx` is the wrapper and `scripts/tcx-integration.bash`
   sets `TCX_HOME` and defines `tcx-clip`. Both are installed by CMake.

## 16. Build & test

```bash
cargo build                 # builds crates/tcx -> target/debug/tcx
cargo test                  # 92 tests
cargo build --release
```

The CMake build for `tg` is untouched and independent; `Cargo.toml` at the repo
root is a separate workspace that happens to share the directory. CMake installs
`scripts/tcx` and `scripts/tcx-integration.bash` alongside the guard's files.

## 17. Shell integration

`scripts/tcx` is a launcher, `scripts/tcx-integration.bash` is the setup:

```bash
. /usr/share/trex-guard/tcx-integration.bash   # or scripts/ in a checkout
```

What it does:

- Sets `TCX_HOME` to `$HOME/.local/share/tcx-home` **only if unset**. This is
  the fix for the Termux/chroot split: one canonical store instead of two.
- Defines `tcx-clip`, which pipes `tcx prime` into `clip`, when `clip` exists.

`scripts/tcx-trial` records the gate's measurements. Its log defaults to
`$TCX_HOME/trial.log`, outside the repo, so trial data is never committed.

Verified behaviours (`tests/wrapper.rs` plus smoke tests):

| Scenario | Result |
| --- | --- |
| `TCX_HOME` unset | integration sets it |
| `TCX_HOME` explicitly set | left alone, never overridden |
| no `HOME` at all | left unset, no crash |
| sourced twice | stable, idempotent |
| `tcx-clip` without `clip` | not defined |
| wrapper on `PATH` before itself | does not recurse |
| real binary earlier on `PATH` | binary wins over wrapper |
