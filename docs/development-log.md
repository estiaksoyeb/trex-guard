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
