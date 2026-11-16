# How This Project Is Actually Used

> This page moved to [`docs/onboarding.md`](onboarding.md), which covers the same
> material — the loop, the two commands, the store, `map`, and every mistake
> already made — in one place, alongside what the project is and why it is
> split the way it is.

Two rules survive here because they are the ones most easily forgotten:

**The human's entire job is: run one block, paste the output back.** The AI
writes every command. The human never composes commands, never types
`tcx note ...`, never decides what to grep.

**Do not invent experiments to validate something daily use answers for free.**
The 5-session trial was rejected for this reason, and its tally script has been
deleted. Do not rebuild it.

The live workflow rules an AI follows are not in any document in this
repository: they are in the prompt template (`$TCX_HOME/config/prompt.md`, or
`~/.config/tcx/prompt.md`), which `tg fresh` and `tg resume` print.
