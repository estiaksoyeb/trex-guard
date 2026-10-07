# Architecture

## Goal

Provide a terminal-level safety gate between the user's terminal and an interactive shell.

The intended architecture is:

Terminal
   |
   v
TREX Guard
   |
   +-- paste detection
   +-- command analysis
   +-- risk classification
   +-- human approval
   |
   v
PTY
   |
   v
Bash / Readline / Ble.sh

## Why a PTY

A normal `.bashrc` hook does not provide a reliable, general-purpose interception point for arbitrary pasted input before shell processing.

TREX Guard therefore uses a pseudo-terminal so that the child shell can remain a normal interactive shell.

The prototype uses:

- `forkpty()`
- a PTY master
- a child shell
- terminal window-size propagation
- bidirectional I/O forwarding

## Current prototype

`src/main.cpp`

The prototype currently:

1. Reads the terminal's current termios configuration.
2. Reads the terminal window size.
3. Creates a PTY using `forkpty()`.
4. Starts `$SHELL --login` in the child.
5. Forwards terminal input to the PTY master.
6. Forwards child output to the terminal.
7. Waits for the child shell to exit.

The prototype does **not** currently:

- detect pastes
- scan commands
- classify risk
- modify shell configuration
- install itself
- persist configuration
- intercept dangerous commands

## Target architecture

PTY layer
   |
   v
Input stream
   |
   v
Paste detector
   |
   v
Shell lexer/parser
   |
   v
Risk engine
   |
   +---- safe --------> shell
   |
   +---- review ------> human approval
   |
   +---- high risk ---> explicit confirmation

The scanner should not rely exclusively on literal string matching. Equivalent shell forms should eventually be normalized before rules are evaluated.

## Design constraint

TREX Guard should remain transparent for ordinary interactive use. The safety machinery should become visible primarily when pasted or otherwise suspicious input is encountered.
