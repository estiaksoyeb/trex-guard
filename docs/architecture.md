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

`src/main.cpp`, `src/rules.cpp` (`trex::classify`)

The prototype currently:

1. Reads the terminal's current termios configuration.
2. Reads the terminal window size.
3. Creates a PTY using `forkpty()`.
4. Starts `$SHELL --login` in the child.
5. Forwards terminal input to the PTY master.
6. Forwards child output to the terminal.
7. Waits for the child shell to exit.

The prototype now intercepts pastes. `struct PasteTap` in src/main.cpp
classifies each stdin chunk, emitting typed bytes as "forward" and paste
bytes as "hold". When a paste completes, the guard prints the held text
and asks for approval; only on `y` are the bytes written to the PTY
master. A literal-string scanner (`src/rules.cpp`, `trex::classify`)
tags known-dangerous patterns for display. It does not parse shell, so
obfuscated forms evade it.

Confirmed behavior (SEEN):

- Bracketed-paste markers arrive intact on the proxy's stdin.
- A 3000-line paste (19893 bytes) was captured as a single
  PASTE_START/PASTE_END pair despite being split across six reads of
  about 4095 bytes each, with the correct newline count.

Known constraint (SEEN in an earlier tap, not yet re-tested here):

- While a command is running, the child shell disables paste mode, so
  pastes made at that moment arrive without bracketed markers. This
  bounds what a marker-based detector can intercept.

The prototype does not currently:

- parse shell syntax (scanner is literal-string only)
- enforce different behavior for DANGER vs REVIEW (both just prompt)
- modify shell configuration
- install itself
- persist configuration

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
