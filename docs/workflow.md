# How This Project Is Actually Used

> Read this first. Every misunderstanding in the session that built `tcx` is
> written down here so it does not happen again.

## The one fact that matters

**The human's entire job is: run one block, paste the output back.**

The AI writes every command. The human never composes commands, never types
`tcx note ...`, never decides what to grep. The AI emits paste-ready bash blocks;
the human pastes them into a terminal and pastes the output back into the chat.

If a design assumes the human will "just type a command when they need it", the
design is wrong.

## The loop

```text
human:  tg                        # fresh terminal -> guard active
human:  tg fresh | clip           # prompt with the Project section filled in
human:  paste into a new chat

ai:     emits a bash block        # { rg ...; } | clip
human:  paste the block, paste output back

        ... this repeats while working ...

ai:     as it learns, includes note-writing inside its blocks
human:  paste -> notes saved (no extra effort)

human:  tg resume | clip          # new chat: prompt + Project + what was learned
human:  paste -> the new AI is caught up
```

The human is **the execution layer**. That is the whole design of the workflow
prompt. Nothing in the tooling may add steps to that loop.

## What the AI's blocks look like

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

Observations that follow from this shape:

- Blocks start with `set +H` and end with `echo exit=$?`.
- Queries are batched into one block and piped to `clip`.
- Output is capped (`--max-columns 160`, `-m 5`, `head -n 20`).
- Notes ride along inside these same blocks once the AI is told to write them.

## Where notes come from

Notes are **written by the AI**, inside its paste blocks, as the conversation
continues. They are not typed by the human afterwards.

So this is correct as-is:

```bash
tcx note -k gotcha "something here that surprised me"
tcx note -k decision "why this project does X this way"
tcx note -k open "something I still don't know"
```

The AI generates these lines. The human runs them because they arrive inside a
block that was already going to be pasted anyway.

## Two commands that justify the tool's existence

Both are run **once per chat**, then pasted. That is the whole interaction.

### `tg fresh` - start a session

Implemented: `tcx fresh`, reached through `tg fresh` (the guard never starts a
shell for it). The prompt text lives in `~/.local/share/tcx-home/config/prompt.md`
(or `<repo>/.tg/prompt.md` to pin one repository); edit the file, no rebuild.
The path used is printed on stderr, so `tg fresh | clip` copies only the prompt.

It prints the workflow prompt with the `## Project` section already filled in
from the repository the user is standing in:

```text
## Project
mpvRex: Android video player, Kotlin + Jetpack Compose
Package: xyz.mpv.rex
Source: app/src/main/kotlin/xyz/mpv/rex/
Build: Gradle
type: code
```

The user copies it and pastes it at the top of a new chat. Nothing else.

### `tg resume` - continue in a new chat

Implemented as `tcx resume`. Same prompt and Project section, **plus** what the
previous session learned (decisions, gotchas, open questions, where work
stopped), so a fresh AI catches up without re-exploring the repository. Notes
come from the store the AI already writes to while working.

## Mistakes that were actually made

Recorded so they are not repeated.

### 1. Designed for the wrong user

`tcx note` was discussed as if the human would type it after a session. Wrong.
The AI writes it; the human only runs blocks that already exist.

### 2. Invented a measurement ritual

A "5-session trial" was proposed: count orientation turns, log with
`tcx-trial log control 4`, compare averages. The user's answer, correctly: they
would know within a week of ordinary use, and the ritual is busywork.

Rule: **do not invent experiments to validate something daily use answers for
free.** If a tool is used every day, ship it and let use be the test.

### 3. Explained instead of building

Asked how to test, the answer given was five steps of procedure. What was wanted
was the next working command. When the user says "continue" or "do it", they
mean write the code, not write the instructions.

### 4. Assumed commands the human types

Any interface where the human composes arguments at a prompt is suspect. The
real interface is: one command, no arguments, output on the clipboard.

### 5. Over-asked before building

Repeatedly stopped to ask questions whose answers were already visible in the
workflow prompt the user pasted. Read the pasted prompt first; ask only about
what it genuinely does not say.

## What "clear" looks like

Before proposing anything, be able to state:

1. Who runs the command - always the human, always a block the AI wrote.
2. When it runs - once per chat (`fresh`, `resume`), or inside an existing block
   (`tcx note`).
3. What the human does with the result - pastes it into chat, nothing else.
4. Whether it removes a step or adds one. If it adds one, it is probably wrong.

## Consequences for the build

- `tg fresh` and `tg resume` are the product. They are the two commands that
  remove a manual step.
- `tcx note` stays as it is, because the AI already emits it correctly.
- The prompt text should live in a **file**, not the binary, so the user can
  refine their rules without a rebuild.
- Existing `.ai/` directories are an input to import, not a thing to compete
  with. `tcx` stores the same notes outside the repo so the repo stays clean and
  worktrees share them.
