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
//! | `tgr --check FILE` | nothing; the file is classified and `tgr` exits |
//! | any of the above | `--force` is stripped wherever it appears |
//!
//! # `--check`
//!
//! A non-PTY entry point for another program (the `tgw` watchdog) to ask the
//! guard what it thinks of a paste without starting a shell:
//!
//! ```text
//! tgr --check FILE     # or "-" to read the text from stdin
//! tgr --check -        # reads to EOF, unlike the paste path
//! ```
//!
//! It calls [`tgr::rules::classify_with`] — the same function the PTY path
//! calls — with the same config, so a verdict here and a verdict on a paste
//! cannot disagree. The only difference is I/O: `--check -` reads to EOF
//! instead of waiting for bracketed-paste markers, because there is no human
//! to wait for.
//!
//! Exit codes are the contract `tgw` codes against:
//!
//! | Code | Meaning |
//! | --- | --- |
//! | 0 | safe: nothing matched |
//! | 1 | danger: at least one danger rule matched |
//! | 3 | review: worst match is review |
//! | 2 | usage, read, size or encoding error: the caller must fail closed |
//!
//! Input is capped at [`CHECK_MAX_BYTES`]. Over the cap is exit 2, never a
//! truncated classification: half a paste judged safe is worse than refusing
//! it.
//!
//! stderr gets one `rule=<name> risk=<safe\|review\|danger>` line per distinct
//! matched rule, so a caller can map rule names to its own policy without
//! ever reading the paste itself.
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

use std::io::Read;
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
  tgr --check FILE  Classify FILE and exit (no shell, no guard)
  tgr --check -     Classify text read from stdin
  tgr --list-rules  Print every watch-rule name, one per line
  tgr --help        Show this help message
  tgr --version     Show version ({VERSION})

Options:
  --force           Start even when a guard is already active
                    (scripts pass this; it is stripped from the command)

--check exit codes (for programs that gate on them):
  0  safe      1  danger      3  review      2  usage or read error
  stderr: one line per matched rule, \"rule=<name> risk=<safe|review|danger>\"
  Any code other than 0, 1 or 3 must be treated as fail closed.

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

/// Parses `--check PATH` / `--check -` out of argv.
///
/// Only recognised as the **first** argument, so it cannot shadow a command
/// named `--check`, and only in its own two-word form: `tgr --check` with no
/// argument is a usage error rather than a request to classify an empty name.
fn check_arg(argv: &[String]) -> Option<Result<&str, &'static str>> {
    if argv.first().map(String::as_str) != Some("--check") {
        return None;
    }
    match argv.get(1) {
        Some(path) if argv.len() == 2 => Some(Ok(path.as_str())),
        Some(_) => Some(Err("--check takes exactly one path")),
        None => Some(Err("--check needs a file, or - for stdin")),
    }
}

/// Maps a classified result to the `--check` exit code.
///
/// `Safe` is 0, `Review` is 3, `Danger` is 1. Anything the caller does not
/// recognise is told to fail closed, so the codes must stay distinct.
fn check_exit(matched: &tgr::risk::Match) -> i32 {
    match matched.risk {
        tgr::risk::Risk::Safe => 0,
        tgr::risk::Risk::Danger => 1,
        tgr::risk::Risk::Review => 3,
    }
}

/// Prints one machine-stable line per distinct matched rule.
///
/// A rule that fires on several snippets is reported once: the caller cares
/// which rules matched, not how many times.
fn report_rules(matched: &tgr::risk::Match) -> String {
    fn walk(m: &tgr::risk::Match, seen: &mut Vec<(String, tgr::risk::Risk)>) {
        if m.rule.is_empty() {
            return;
        }
        for inner in &m.matches {
            walk(inner, seen);
        }
        if !seen.iter().any(|(r, _)| *r == m.rule) {
            seen.push((m.rule.clone(), m.risk));
        }
    }

    let mut seen: Vec<(String, tgr::risk::Risk)> = Vec::new();
    walk(matched, &mut seen);
    seen.iter()
        .map(|(rule, risk)| format!("rule={rule} risk={risk}\n"))
        .collect()
}

/// Largest input `--check` will classify.
///
/// The paste path has a terminal-sized feed, so it never needed a cap. A
/// caller piping a file can hand over gigabytes, and classifying that is a
/// memory exhaustion, not a security decision. Over the cap is exit 2 (fail
/// closed), never a truncated classification: half a paste classified as safe
/// is worse than refusing it.
const CHECK_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Reads `--check` input, refusing anything larger than [`CHECK_MAX_BYTES`].
///
/// `-` reads stdin to EOF; the paste path waits for bracketed-paste markers
/// instead, because it has a human to wait for and `--check` does not.
fn read_check_input(path: &str) -> Result<String, String> {
    fn take<R: std::io::Read>(r: R) -> Result<String, String> {
        let mut buf = Vec::new();
        // `take` bounds the read, so a pipe that never ends cannot exhaust
        // memory; the extra byte is what tells us it was over the cap.
        r.take(CHECK_MAX_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        if buf.len() as u64 > CHECK_MAX_BYTES {
            return Err(format!("input exceeds {CHECK_MAX_BYTES} bytes"));
        }
        String::from_utf8(buf).map_err(|e| format!("input is not valid UTF-8: {e}"))
    }

    if path == "-" {
        take(std::io::stdin())
    } else {
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        take(f)
    }
}

/// Runs `--check` and returns the process exit code.
fn run_check(arg: Result<&str, &'static str>) -> i32 {
    let path = match arg {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("tgr: {msg}");
            return 2;
        }
    };

    let text = match read_check_input(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tgr: --check {path}: {e}");
            return 2;
        }
    };

    let config = tgr::config::load(None);
    let verdict = tgr::rules::classify_with(&text, Some(&config));
    for problem in &verdict.problems {
        let msg = match problem {
            tgr::rules::RegexProblem::Invalid { rule, message } => {
                format!("rule `{rule}` does not compile ({message}); the paste is held")
            }
            tgr::rules::RegexProblem::BacktrackLimit { rule } => {
                format!("rule `{rule}` exceeded the backtrack limit; the paste is held")
            }
        };
        eprintln!("tgr: rule problem: {msg}");
    }
    eprint!("{}", report_rules(&verdict.matched));

    // Watch rules report a name and nothing else. They cannot change the
    // exit code: see `crate::watch`.
    for name in tgr::watch::matches(&config, &text) {
        eprintln!("watch={name}");
    }
    let (_, broken) = tgr::watch::compile(&config);
    for b in &broken {
        eprintln!("tgr: watch rule `{}` does not compile ({}); it cannot match", b.name, b.message);
    }

    // Mirror the PTY path: a rule that could not run holds the paste, so the
    // caller must not treat it as safe merely because nothing matched.
    if verdict.has_problems() {
        return 3;
    }
    check_exit(&verdict.matched)
}

/// Runs `--list-rules`: every watch-rule name in effect, one per line.
///
/// This is the source of truth `tgw` validates `hold_rules` against, so it
/// prints names only — a caller parses lines, not prose. A rule that does not
/// compile is marked, because it will never match and a hold that depends on
/// it would silently never happen.
fn run_list_rules() -> i32 {
    let config = tgr::config::load(None);
    let (ok, broken) = tgr::watch::compile(&config);
    let broken_names: Vec<String> = broken.iter().map(|b| b.name.clone()).collect();
    for (name, _) in &ok {
        println!("{name}");
    }
    for name in &broken_names {
        println!("{name}\tBROKEN");
    }
    for b in &broken {
        eprintln!("tgr: watch rule `{}` does not compile: {}", b.name, b.message);
    }
    if ok.is_empty() { 1 } else { 0 }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    if argv.first().map(String::as_str) == Some("--list-rules") && argv.len() == 1 {
        std::process::exit(run_list_rules());
    }

    if let Some(arg) = check_arg(&argv) {
        std::process::exit(run_check(arg));
    }

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
        for needle in [
            "tgr <command>",
            "--help",
            "--version",
            "--force",
            "TGR_ACTIVE",
            "--check FILE",
            "--check -",
            "--list-rules",
        ] {
            assert!(text.contains(needle), "help does not mention {needle:?}");
        }
    }

    #[test]
    fn help_documents_the_check_exit_codes() {
        // The exit codes are a contract another program codes against, so
        // every one of them must be stated, and the fail-closed rule with it.
        let text = help();
        for needle in ["0  safe", "1  danger", "3  review", "2  usage"] {
            assert!(text.contains(needle), "help omits exit code {needle:?}");
        }
        assert!(text.contains("fail closed"), "help omits the fail-closed rule");
    }

    #[test]
    fn check_is_only_recognised_as_the_first_argument() {
        assert_eq!(check_arg(&[]), None);
        assert_eq!(check_arg(&v(&["ls", "--check", "f"])), None);
        assert_eq!(check_arg(&v(&["--check", "f"])), Some(Ok("f")));
        assert_eq!(check_arg(&v(&["--check", "-"])), Some(Ok("-")));
    }

    #[test]
    fn check_without_a_path_is_a_usage_error_not_an_empty_name() {
        assert!(check_arg(&v(&["--check"])).unwrap().is_err());
        assert!(check_arg(&v(&["--check", "a", "b"])).unwrap().is_err());
    }

    #[test]
    fn exit_codes_distinguish_all_three_risks() {
        // If two risks shared a code, a caller could not tell "danger" from
        // "review", and the contract in the docs would be a lie.
        let mk = |risk| tgr::risk::Match {
            risk,
            rule: "r".into(),
            snippet: "s".into(),
            explanation: String::new(),
            matches: Vec::new(),
        };
        assert_eq!(check_exit(&mk(tgr::risk::Risk::Safe)), 0);
        assert_eq!(check_exit(&mk(tgr::risk::Risk::Danger)), 1);
        assert_eq!(check_exit(&mk(tgr::risk::Risk::Review)), 3);
    }

    #[test]
    fn report_rules_names_each_rule_once() {
        let inner = |rule: &str, risk| tgr::risk::Match {
            risk,
            rule: rule.into(),
            snippet: "s".into(),
            explanation: String::new(),
            matches: Vec::new(),
        };
        let matched = tgr::risk::Match {
            risk: tgr::risk::Risk::Danger,
            rule: "rm -rf".into(),
            snippet: "s".into(),
            explanation: String::new(),
            matches: vec![
                inner("rm -rf", tgr::risk::Risk::Danger),
                inner("rm -rf", tgr::risk::Risk::Danger),
                inner("git clean", tgr::risk::Risk::Review),
            ],
        };
        let out = report_rules(&matched);
        assert_eq!(out, "rule=rm -rf risk=danger\nrule=git clean risk=review\n");
    }

    #[test]
    fn report_rules_is_empty_for_a_safe_paste() {
        assert_eq!(report_rules(&tgr::risk::Match::safe()), "");
    }

    #[test]
    fn check_refuses_input_over_the_cap() {
        // Over the cap is a refusal, not a truncated classification: half a
        // paste judged safe would be worse than not answering.
        let path = std::env::temp_dir().join(format!("tgr-cap-{}", std::process::id()));
        std::fs::write(&path, vec![b'a'; CHECK_MAX_BYTES as usize + 1]).unwrap();
        let err = read_check_input(path.to_str().unwrap()).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(err.contains("exceeds"), "unexpected error: {err}");
    }

    #[test]
    fn check_accepts_input_at_the_cap() {
        let path = std::env::temp_dir().join(format!("tgr-capok-{}", std::process::id()));
        std::fs::write(&path, vec![b'a'; CHECK_MAX_BYTES as usize]).unwrap();
        let got = read_check_input(path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        assert_eq!(got.unwrap().len(), CHECK_MAX_BYTES as usize);
    }

    #[test]
    fn check_refuses_non_utf8() {
        let path = std::env::temp_dir().join(format!("tgr-utf8-{}", std::process::id()));
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let err = read_check_input(path.to_str().unwrap()).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(err.contains("UTF-8"), "unexpected error: {err}");
    }

    #[test]
    fn check_reports_a_missing_file() {
        let err = read_check_input("/nonexistent/tgr-no-such-file").unwrap_err();
        assert!(!err.is_empty());
    }
}
