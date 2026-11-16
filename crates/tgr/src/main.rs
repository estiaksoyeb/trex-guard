//! The `tgr` command.
//!
//! Ported from `main()` in `src/main.cpp`. The argument rules are copied
//! exactly, because they decide what the user's shell command line becomes:
//!
//! | Invocation | What runs |
//! | --- | --- |
//! | `tgr` | `$SHELL`, plus `--norc` if an ancestor was started with it |
//! | `tgr -x ...` | `$SHELL -x ...` (flags are passed *to the shell*) |
//! | `tgr cmd ...` | `cmd ...` (first non-flag argument is the program) |
//! | any of the above | `--force` is stripped wherever it appears |
//!
//! # Why `--force` exists
//!
//! `scripts/tg` refuses to start a guard inside a guard. When the user wants
//! it anyway - to test, or to nest deliberately - `--force` is the explicit
//! override. `tgr` accepts the same flag so scripts that pass it keep working.
//!
//! # Signals
//!
//! A POSIX signal handler may only touch an atomic and call async-signal-safe
//! functions, so the handlers here do the minimum: `SIGWINCH` sets a flag the
//! main loop turns into a `TIOCSWINSZ`, and the terminating signals set a flag
//! that makes the loop unwind normally, restoring the terminal and forwarding
//! the signal to the child.

use std::sync::atomic::Ordering;

use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};

use tgr::pty;

const VERSION: &str = env!("CARGO_PKG_VERSION");

extern "C" fn on_sigwinch(_: i32) {
    pty::WINDOW_CHANGED.store(true, Ordering::SeqCst);
}

extern "C" fn on_terminate(_: i32) {
    tgr::TERMINATE.store(true, Ordering::SeqCst);
}

fn help() -> String {
    format!(
        "tgr - TREX Guard (Rust port) - Terminal paste safety gate

Usage:
  tgr               Launch a guarded shell
  tgr <command>     Run a command under the guard
  tgr --help        Show this help message
  tgr --version     Show version ({VERSION})

Options:
  --force           Start even when a guard is already active
                    (scripts pass this; it is stripped from the command)

The guard reads the same config as tg:
  $XDG_CONFIG_HOME/trex-guard/config.ini
  (default: ~/.config/trex-guard/config.ini)

While a guarded shell runs it exports TGR_ACTIVE=1 to the child, so tools can
tell a tgr session apart from a tg one.
"
    )
}

/// Mirrors `ancestor_has_norc()` in the C++ guard.
///
/// Walks up to six ancestors looking for `--norc` in a command line, so that a
/// shell started by `tgr` behaves like the one outside it: if the user's outer
/// shell was started with `--norc`, the guarded one should be too. `/proc` is
/// Linux-specific; elsewhere this returns false, which is what the C++ does
/// when it cannot open those files.
fn ancestor_has_norc() -> bool {
    let mut cur = nix::unistd::getppid().as_raw();
    for _ in 0..6 {
        if cur <= 1 {
            break;
        }
        if let Ok(raw) = std::fs::read(format!("/proc/{cur}/cmdline")) {
            if raw.split(|&b| b == 0).any(|arg| arg == b"--norc") {
                return true;
            }
        }
        // `stat` is `pid (comm) state ppid ...`. `comm` may itself contain
        // spaces and parentheses, so the fields start after the *last* `)`.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{cur}/stat")) else {
            break;
        };
        let Some(rparen) = stat.rfind(')') else {
            break;
        };
        let mut fields = stat[rparen + 1..].split_whitespace();
        let _state = fields.next();
        let ppid: i32 = match fields.next().and_then(|s| s.parse().ok()) {
            Some(p) => p,
            None => break,
        };
        if ppid <= 1 {
            break;
        }
        cur = ppid;
    }
    false
}

/// Works out the argv of the guarded process.
///
/// Separated from `main` so it can be tested without a terminal: it is the
/// only place that decides what the user actually runs.
pub fn child_argv(argv: &[String], default_shell: &str, norc: bool) -> Vec<String> {
    let strip = |f: &dyn Fn(&String) -> bool| -> Vec<String> {
        argv.iter().filter(|a| !f(a)).cloned().collect()
    };
    if argv.is_empty() {
        let mut v = vec![default_shell.to_string()];
        if norc {
            v.push("--norc".to_string());
        }
        return v;
    }
    if argv[0].starts_with('-') {
        // Flags belong to the shell: `tgr -x` means `$SHELL -x`.
        let mut v = vec![default_shell.to_string()];
        v.extend(strip(&|a| *a == "--force"));
        return v;
    }
    strip(&|a| *a == "--force")
}

fn install(sig: Signal, handler: extern "C" fn(i32)) -> nix::Result<()> {
    let action = SigAction::new(SigHandler::Handler(handler), SaFlags::empty(), SigSet::empty());
    // SAFETY: both handlers only store to an atomic, which is
    // async-signal-safe. They never allocate, lock, or touch the terminal.
    unsafe { sigaction(sig, &action) }.map(drop)
}

fn install_handlers() -> nix::Result<()> {
    for sig in [
        Signal::SIGTERM,
        Signal::SIGHUP,
        Signal::SIGINT,
        Signal::SIGQUIT,
    ] {
        install(sig, on_terminate)?;
    }
    install(Signal::SIGWINCH, on_sigwinch)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    match argv.first().map(String::as_str) {
        Some("--help" | "-h") => {
            print!("{}", help());
            return;
        }
        Some("--version" | "-v") => {
            println!("tgr version {VERSION}");
            return;
        }
        _ => {}
    }

    let default_shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string());

    let child = child_argv(&argv, &default_shell, ancestor_has_norc());

    if let Err(e) = install_handlers() {
        eprintln!("tgr: sigaction: {e}");
        return;
    }

    let config = tgr::config::load(None);
    let code = pty::run(config, child);
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_runs_the_default_shell() {
        assert_eq!(child_argv(&[], "/bin/bash", false), v(&["/bin/bash"]));
    }

    #[test]
    fn norc_is_inherited_by_the_guarded_shell() {
        assert_eq!(child_argv(&[], "/bin/zsh", true), v(&["/bin/zsh", "--norc"]));
    }

    #[test]
    fn leading_flags_belong_to_the_shell() {
        assert_eq!(
            child_argv(&v(&["-x", "-i"]), "/bin/bash", false),
            v(&["/bin/bash", "-x", "-i"])
        );
    }

    #[test]
    fn a_command_is_run_directly() {
        assert_eq!(
            child_argv(&v(&["ls", "-la"]), "/bin/bash", false),
            v(&["ls", "-la"])
        );
    }

    #[test]
    fn force_is_stripped_wherever_it_appears() {
        // `tgr --force bash`: the first word is a flag, so this is "bash as a
        // shell flag", i.e. `$SHELL bash`. That looks odd but it is exactly
        // what the C++ guard does (verified against the built binary).
        assert_eq!(
            child_argv(&v(&["--force", "bash"]), "/bin/bash", false),
            v(&["/bin/bash", "bash"])
        );
        // The program form strips it and runs the program.
        assert_eq!(
            child_argv(&v(&["bash", "--force"]), "/bin/bash", false),
            v(&["bash"])
        );
        assert_eq!(
            child_argv(&v(&["ls", "--force", "-l"]), "/bin/bash", false),
            v(&["ls", "-l"])
        );
        assert_eq!(
            child_argv(&v(&["--force", "-i"]), "/bin/bash", false),
            v(&["/bin/bash", "-i"])
        );
    }

    #[test]
    fn help_documents_every_mode_and_the_force_flag() {
        // Guards against the help drifting from the argument parser: each
        // accepted form must be described.
        let text = help();
        for needle in ["tgr <command>", "--help", "--version", "--force", "TGR_ACTIVE"] {
            assert!(text.contains(needle), "help does not mention {needle:?}");
        }
    }
}
