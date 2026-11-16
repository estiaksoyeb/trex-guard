//! The `tgw` command: a watchdog that gates clipboard blocks through `tgr`
//! and (only when told to) runs them.
//!
//! # What it is
//!
//! The human writes a block in a chat. They copy it to the clipboard. `tgw`
//! sees it, asks `tgr --check` what it thinks, and either runs it, holds it for
//! a tap, or refuses. The block's own `| clip` puts the output back on the
//! clipboard, so the human copies once and pastes once instead of switching to
//! a terminal.
//!
//! # What it must never do
//!
//! - **Never classify risk.** That is `tgr`'s job. `tgw` sees an exit code and
//!   a list of rule names and nothing else.
//! - **Never bypass the gate.** There is no flag and no config key that skips
//!   it.
//! - **Never read the guard config.** `~/.config/trex-guard` belongs to the
//!   guards; `tgw` only invokes the gate binary.
//! - **Never write to the clipboard.**
//!
//! # Layout
//!
//! | Module | Contains |
//! | --- | --- |
//! | [`config`] | `watch.ini` parsing, argv-string splitting |
//! | [`help`] | the one command table, rendered for `--help` |
//! | [`clip`] | reading the clipboard, validating, hashing, staging |
//! | [`execute`] | running an approved block: env, cwd, timeout, output cap |
//! | [`gate`] | running `tgr --check` and mapping its exit code |
//! | [`holds`] | hold records, the re-check on approval, the `auto` lock |
//! | [`log`] | the event log and kept block copies |
//! | [`pipeline`] | the one ordered path from clipboard to verdict |
//! | [`run`] | single instance, signal handling, state dir checks, sweeps |
//!
//! The rest (clipboard polling, staging, gating, execution, holds) arrives in
//! later slices; see `docs/tgw.md`.

pub mod clip;
pub mod config;
pub mod execute;
pub mod gate;
pub mod holds;
pub mod help;
pub mod log;
pub mod pipeline;
pub mod run;
