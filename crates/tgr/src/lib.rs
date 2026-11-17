//! TREX Guard, in Rust.
//!
//! This is a *behavioural clone* of the C++ `tg`, built to run beside it, not
//! to replace it. The rule while porting is therefore:
//!
//! > Match the C++ guard exactly, including its quirks. Correct only what is
//! > provably wrong or dangerous, and record every deliberate difference in
//! > `docs/tgr.md`.
//!
//! A port that quietly improves things is a port nobody can compare, and the
//! C++ suite is the only specification that exists.
//!
//! # Layout
//!
//! | Module | Ported from | Contains |
//! | --- | --- | --- |
//! | [`risk`] | `rules.h` | the `Safe`/`Review`/`Danger` ordering |
//! | [`config`] | `config.cpp` | `config.ini` parsing, same file as `tg` |
//! | [`shell`] | `shell_parser.cpp` | pastes split into pipelines and commands |
//! | [`rules`] | `rules.cpp` | built-in, literal, regex and user rules |
//!
//! # Deliberate differences from the C++ guard
//!
//! Each is listed in `docs/tgr.md` with its reason. In short:
//!
//! 1. **A user regex that does not compile is an error, not silence.** The C++
//!    guard catches `std::regex_error` and drops the rule, so a typo in
//!    `[custom_rules]` silently disables a protection the user believes is on.
//! 2. **A regex that exceeds the backtrack limit fails closed.** The paste is
//!    at least `Review`, never `Safe` (see `docs/tcx.md` §8).
//!
//! Both make `tgr` *more* cautious than `tg`; neither makes it less safe.

pub mod config;
pub mod pty;

/// Set by the `SIGTERM`/`SIGHUP`/`SIGINT`/`SIGQUIT` handler so the PTY loop can
/// stop, restore the terminal and forward the signal. Kept in the library
/// because both the handler (installed by `main`) and the loop (in [`pty`])
/// must see the same flag, and a handler may do nothing else.
pub static TERMINATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Which terminating signal arrived, so it can be forwarded to the guarded
/// shell. 0 means none. A handler may only store to an atomic, so the number
/// is recorded and turned into a `Signal` by the main loop.
pub static TERMINATE_SIGNAL: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Whether a terminating signal has been received.
pub fn terminated() -> bool {
    TERMINATE.load(std::sync::atomic::Ordering::SeqCst)
}

/// The terminating signal that arrived, if any.
pub fn terminate_signal() -> Option<i32> {
    match TERMINATE_SIGNAL.load(std::sync::atomic::Ordering::SeqCst) {
        0 => None,
        sig => Some(sig),
    }
}
pub mod risk;
pub mod rules;
pub mod shell;
pub mod watch;

pub use config::Config;
pub use risk::{Match, Risk};
pub use rules::classify;
