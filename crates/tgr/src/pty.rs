//! The guarded terminal: a PTY between the user and a shell.
//!
//! Ported from the second half of `src/main.cpp`. The structure is kept
//! because each piece exists for a reason that a rewrite would have to
//! rediscover:
//!
//! | Piece | Why |
//! | --- | --- |
//! | bracketed paste on stdin | the only reliable paste boundary |
//! | burst detection fallback | terminals that do not bracket pastes |
//! | 15 ms drain window | one read rarely holds a whole paste |
//! | markers re-added on release | ble.sh/readline batch-insert instead of decoding |
//! | `\x1b[?2004l` suppression | the shell must not switch paste mode off under us |
//! | Ctrl-C on cancel | makes the line editor repaint its prompt immediately |
//!
//! # Deliberately different from C++
//!
//! - The child gets `TGR_ACTIVE=1`, **not** `TREX_GUARD_ACTIVE=1`. Setting the
//!   C++ guard's marker would make `scripts/tg` think a `tg` session is active
//!   and refuse to start, and would make nested `tgr` sessions silently no-op.
//!   Two guards must not be able to confuse each other.
//! - A paste whose classification could not be fully evaluated (a broken user
//!   regex) is never auto-approved, and says why.

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::pty::{forkpty, ForkptyResult, Winsize};
use nix::sys::signal::{kill, Signal};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{read, write, Pid};

use crate::config::Config;
use crate::risk::{Match, Risk};
use crate::rules;

/// The marker exported to the guarded shell.
pub const ACTIVE_ENV: &str = "TGR_ACTIVE";

const BP_ON: &[u8] = b"\x1b[?2004h";
const BP_OFF: &[u8] = b"\x1b[?2004l";
const BP_BEGIN: &[u8] = b"\x1b[200~";
const BP_END: &[u8] = b"\x1b[201~";

/// One chunk of stdin, split into bytes to forward and bytes to hold.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Feed {
    pub forward: Vec<u8>,
    pub hold: Vec<u8>,
    pub paste_complete: bool,
}

/// Watches stdin for bracketed paste sequences.
#[derive(Debug, Default)]
pub struct PasteTap {
    pub in_paste: bool,
    match_len: usize,
    captured: Vec<u8>,
}

impl PasteTap {
    /// Classifies one read from stdin.
    ///
    /// Bytes that are not part of a paste are forwarded immediately; bytes
    /// between the markers are held until the closing marker arrives. Partial
    /// markers at a buffer boundary are handled by carrying `match_len`
    /// across calls, which is why this is a struct and not a function.
    pub fn feed(&mut self, data: &[u8]) -> Feed {
        let mut out = Feed::default();
        for &c in data {
            let marker: &[u8] = if self.in_paste { BP_END } else { BP_BEGIN };
            let m = marker.len();

            if c == marker[self.match_len] {
                self.match_len += 1;
                if self.match_len < m {
                    continue;
                }
                self.match_len = 0;
                if self.in_paste {
                    self.in_paste = false;
                    out.hold = std::mem::take(&mut self.captured);
                    out.paste_complete = true;
                } else {
                    self.in_paste = true;
                    self.captured.clear();
                }
                continue;
            }

            if self.match_len > 0 {
                if self.in_paste {
                    self.captured.extend_from_slice(&marker[..self.match_len]);
                } else {
                    out.forward.extend_from_slice(&marker[..self.match_len]);
                }
                self.match_len = 0;
                if c == marker[0] {
                    self.match_len = 1;
                    continue;
                }
            }

            if self.in_paste {
                self.captured.push(c);
            } else {
                out.forward.push(c);
            }
        }
        out
    }
}

/// Whether an unbracketed read looks like a paste rather than typing.
///
/// Ported exactly: multi-line input, or >= 8 bytes not starting with ESC.
/// Both thresholds are guesses tuned against real terminals; changing them
/// changes what users get prompted for, so they are named constants.
pub fn is_unbracketed_burst(bytes: &[u8]) -> bool {
    let has_newline = bytes.iter().any(|&b| b == b'\n' || b == b'\r');
    (has_newline && bytes.len() > 1) || (bytes.len() >= 8 && bytes.first() != Some(&0x1b))
}

/// What the user chose at the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Cancel,
}

fn write_all(fd: BorrowedFd<'_>, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        match write(fd, data) {
            Ok(0) => return Err(io::Error::other("write returned 0")),
            Ok(n) => data = &data[n..],
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::other(e)),
        }
    }
    Ok(())
}

fn slave_fd() -> BorrowedFd<'static> {
    // SAFETY: the slave side is the process's own stdin; it is not closed here.
    unsafe { BorrowedFd::borrow_raw(0) }
}

fn stdout_fd() -> BorrowedFd<'static> {
    // SAFETY: same reasoning for stdout.
    unsafe { BorrowedFd::borrow_raw(1) }
}

/// Re-adds bracketed-paste markers around the approved payload so line editors
/// take the fast batch-insert path.
fn forward_payload(master: BorrowedFd<'_>, data: &[u8], submit: bool) -> io::Result<()> {
    write_all(master, BP_BEGIN)?;
    write_all(master, data)?;
    write_all(master, BP_END)?;
    if submit {
        write_all(master, b"\n")?;
    }
    Ok(())
}

/// Reads one key and decides whether to run the paste.
///
/// Focus events (`\x1b[I`, `\x1b[O`) are ignored so touching the on-screen
/// keyboard does not cancel; a lone Escape cancels; other sequences are
/// ignored so arrow keys and gestures do not decide anything.
fn read_decision(master: BorrowedFd<'_>, stdin: BorrowedFd<'_>) -> io::Result<Decision> {
    let mut buf = [0u8; 64];
    loop {
        let n = match read(stdin, &mut buf) {
            Ok(0) => return Ok(Decision::Cancel),
            Ok(n) => n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Ok(Decision::Cancel),
        };
        let chunk = &buf[..n];

        if n >= 3 && chunk[0] == 0x1b && chunk[1] == b'[' && (chunk[2] == b'I' || chunk[2] == b'O') {
            continue;
        }
        if chunk[0] == 0x1b {
            if n == 1 {
                return Ok(Decision::Cancel);
            }
            continue;
        }
        for &c in chunk {
            if c == b'\r' || c == b'\n' || c == b'y' || c == b'Y' {
                return Ok(Decision::Approve);
            }
            if c == 0x03 || c == b'n' || c == b'N' {
                return Ok(Decision::Cancel);
            }
        }
        let _ = master;
    }
}

/// Renders the review block for a decision.
pub fn render_review(paste: &[u8], matched: &Match, config: &Config, problems: &[String]) -> String {
    let mut shown = String::new();
    let mut lines = 0usize;
    for &b in paste {
        match b {
            b'\r' | b'\n' => {
                shown.push_str("\r\n");
                lines += 1;
            }
            _ => shown.push(b as char),
        }
    }
    if !shown.is_empty() && !shown.ends_with('\n') {
        lines += 1;
    }

    let mut out = String::new();
    out.push_str(&format!(
        "\r\n\x1b[2m--- [Pasted Content] ({}B, {}L) ---\x1b[0m\r\n{shown}\r\n",
        paste.len(),
        lines
    ));
    out.push_str("\x1b[2m--- [Security Review] ---\x1b[0m\r\n");

    for problem in problems {
        out.push_str(&format!("\x1b[1;33m[CONFIG]\x1b[0m {problem}\r\n"));
    }

    let items: &[Match] = if matched.matches.is_empty() {
        std::slice::from_ref(matched)
    } else {
        &matched.matches
    };

    match matched.risk {
        Risk::Danger | Risk::Review => {
            let danger = matched.risk == Risk::Danger;
            let (head, tag) = if danger {
                ("\x1b[1;31m", "\x1b[1;31m")
            } else {
                ("\x1b[1;33m", "\x1b[1;33m")
            };
            let label = if danger { "HIGH RISK" } else { "REVIEW" };

            if items.len() > 1 {
                out.push_str(&format!(
                    "{head}[{label}]\x1b[0m ({} flagged commands found):\r\n",
                    items.len()
                ));
                for (i, item) in items.iter().enumerate() {
                    let color = if item.risk == Risk::Danger {
                        "\x1b[1;31m"
                    } else {
                        "\x1b[1;33m"
                    };
                    out.push_str(&format!(
                        "  {}. {color}[{}]\x1b[0m {}\r\n",
                        i + 1,
                        item.rule,
                        item.snippet
                    ));
                    if config.show_risk_explanation && !item.explanation.is_empty() {
                        out.push_str(&format!("     \x1b[2mRisk:\x1b[0m {}\r\n", item.explanation));
                    }
                }
            } else {
                out.push_str(&format!("{head}[{label}: {}]\x1b[0m\r\n", matched.rule));
                out.push_str(&format!("  {tag}Command:\x1b[0m {}\r\n", matched.snippet));
                if config.show_risk_explanation && !matched.explanation.is_empty() {
                    out.push_str(&format!("  {tag}Risk:\x1b[0m    {}\r\n", matched.explanation));
                }
            }
        }
        Risk::Safe => out.push_str("\x1b[1;32m[SAFE]\x1b[0m\r\n"),
    }

    out.push_str("\x1b[1mExecute? [Enter/^C] \x1b[0m");
    out
}

/// Set by `main`'s `SIGWINCH` handler. The loop consumes it and forwards the
/// new size to the pty, so a resize reaches the guarded shell instead of
/// leaving it drawing at the old dimensions.
pub static WINDOW_CHANGED: AtomicBool = AtomicBool::new(false);

/// Runs the guarded shell until it exits, returning the shell's exit code.
pub fn run(config: Config, shell_args: Vec<String>) -> i32 {
    let stdin = slave_fd();
    let stdout = stdout_fd();
    WINDOW_CHANGED.store(false, Ordering::SeqCst);

    let original = match tcgetattr(stdin) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tgr: tcgetattr: {e}");
            return 1;
        }
    };

    // The C++ guard treats a failed TIOCGWINSZ as fatal; here it is only used
    // to seed the pty, and a zero size is a working default, so a terminal
    // that cannot report its size still gets a guarded shell.
    let window = read_winsize(stdin).unwrap_or(Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    });

    // SAFETY: forkpty is called before any threads exist in this process.
    let forked = match unsafe { forkpty(Some(&window), Some(&original)) } {
        Ok(f) => f,
        Err(e) => {
            eprintln!("tgr: forkpty: {e}");
            return 1;
        }
    };

    let (child, master) = match forked {
        ForkptyResult::Child => {
            std::env::set_var(ACTIVE_ENV, "1");
            // SAFETY: in the forked child, exec immediately; no Rust state is
            // touched between fork and exec except setenv above.
            let err = exec_child(&shell_args);
            eprintln!("tgr: exec: {err}");
            std::process::exit(127);
        }
        ForkptyResult::Parent { child, master } => (child, master),
    };

    let mut raw = original.clone();
    cfmakeraw(&mut raw);
    if let Err(e) = tcsetattr(stdin, SetArg::TCSAFLUSH, &raw) {
        eprintln!("tgr: tcsetattr: {e}");
        let _ = kill(child, Signal::SIGHUP);
        let _ = waitpid(child, None);
        return 1;
    }
    let _ = write_all(stdout, BP_ON);

    let result = loop_guard(child, &master, &original, config);

    // Always restore the terminal, whatever happened above. Doing this in the
    // parent rather than in a signal handler is what lets the handler stay
    // async-signal-safe.
    let _ = write_all(stdout, BP_OFF);
    let _ = tcsetattr(stdin, SetArg::TCSAFLUSH, &original);

    let result = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("tgr: {e}");
            let _ = kill(child, Signal::SIGHUP);
            let _ = waitpid(child, None);
            return 1;
        }
    };

    if let Some(sig) = result.forward_signal {
        // The guard was told to stop: pass it on so the guarded shell shuts
        // down the way it would have without a guard in the way.
        let _ = kill(child, sig);
    }

    match waitpid(child, None) {
        Ok(WaitStatus::Exited(_, code)) => code,
        Ok(WaitStatus::Signaled(_, sig, _)) => 128 + sig as i32,
        _ => 1,
    }
}

/// How the loop ended.
#[derive(Debug, Default)]
struct LoopEnd {
    /// A terminating signal arrived and must be forwarded to the child.
    forward_signal: Option<Signal>,
}

fn exec_child(args: &[String]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let (program, rest) = args.split_first().expect("non-empty args");
    let err = std::process::Command::new(program).args(rest).exec();
    err
}

fn read_winsize(fd: BorrowedFd<'_>) -> Option<Winsize> {
    let mut ws = Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ writes a libc::winsize into our own buffer.
    let rc = unsafe { nix::libc::ioctl(fd.as_raw_fd(), nix::libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 {
        Some(ws)
    } else {
        None
    }
}

fn set_winsize(fd: BorrowedFd<'_>, ws: &Winsize) -> bool {
    // SAFETY: TIOCSWINSZ only reads the winsize we pass in.
    unsafe { nix::libc::ioctl(fd.as_raw_fd(), nix::libc::TIOCSWINSZ, ws) == 0 }
}

fn loop_guard(
    _child: Pid,
    master: &OwnedFd,
    original: &nix::sys::termios::Termios,
    config: Config,
) -> io::Result<LoopEnd> {
    let stdin = slave_fd();
    let stdout = stdout_fd();
    let master_fd = master.as_fd();
    let _ = original;

    // Resizes and signals arrive asynchronously. `poll` is woken by them
    // (EINTR) and the flags are then checked before any further work, so the
    // loop never blocks indefinitely without noticing one.
    let end = LoopEnd::default();
    let mut tap = PasteTap::default();
    let mut buf = [0u8; 8192];

    loop {
        if crate::terminated() {
            break;
        }
        if WINDOW_CHANGED.swap(false, Ordering::SeqCst) {
            if let Some(ws) = read_winsize(stdin) {
                set_winsize(master_fd, &ws);
            }
        }

        let mut fds = [
            PollFd::new(stdin, PollFlags::POLLIN),
            PollFd::new(master_fd, PollFlags::POLLIN),
        ];
        match poll(&mut fds, PollTimeout::from(250u16)) {
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::other(e)),
        }

        if fds[0].revents().unwrap_or(PollFlags::empty()).contains(PollFlags::POLLIN) {
            let n = match read(stdin, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            };

            let mut feed = tap.feed(&buf[..n]);

            // Fallback for terminals that send pastes unbracketed.
            if !tap.in_paste && !feed.paste_complete && !feed.forward.is_empty()
                && is_unbracketed_burst(&feed.forward)
            {
                let mut held = std::mem::take(&mut feed.forward);
                // Pull the rest of the burst: one read rarely holds it all.
                loop {
                    let mut drain = [PollFd::new(stdin, PollFlags::POLLIN)];
                    match poll(&mut drain, PollTimeout::from(15u16)) {
                        Ok(0) => break,
                        Ok(_) => {
                            let mut more = [0u8; 4096];
                            match read(stdin, &mut more) {
                                Ok(0) => break,
                                Ok(n) => held.extend_from_slice(&more[..n]),
                                Err(nix::errno::Errno::EINTR) => continue,
                                Err(_) => break,
                            }
                        }
                        Err(nix::errno::Errno::EINTR) => continue,
                        Err(_) => break,
                    }
                }
                feed.hold = held;
                feed.paste_complete = true;
            }

            if !feed.forward.is_empty() {
                write_all(master_fd, &feed.forward)?;
            }

            if !feed.paste_complete {
                continue;
            }

            let verdict = rules::classify_with(&String::from_utf8_lossy(&feed.hold), Some(&config));
            let problems: Vec<String> = verdict
                .problems
                .iter()
                .map(|p| match p {
                    rules::RegexProblem::Invalid { rule, message } => {
                        format!("rule `{rule}` does not compile ({message}); the paste is held")
                    }
                    rules::RegexProblem::BacktrackLimit { rule } => {
                        format!("rule `{rule}` exceeded the backtrack limit; the paste is held")
                    }
                })
                .collect();

            let trailing_newline = feed
                .hold
                .last()
                .is_some_and(|&b| b == b'\n' || b == b'\r');

            if verdict.matched.risk == Risk::Safe && config.auto_approve_safe && problems.is_empty() {
                forward_payload(master_fd, &feed.hold, trailing_newline)?;
                continue;
            }

            let review = render_review(&feed.hold, &verdict.matched, &config, &problems);
            write_all(stdout, review.as_bytes())?;

            match read_decision(master_fd, stdin)? {
                Decision::Approve => {
                    forward_payload(master_fd, &feed.hold, true)?;
                    write_all(stdout, b"\r\n\x1b[1;32m[approved]\x1b[0m\r\n")?;
                }
                Decision::Cancel => {
                    write_all(stdout, b"\r\n\x1b[1;31m[cancelled]\x1b[0m\r\n")?;
                    // Ctrl-C so the line editor resets and repaints.
                    write_all(master_fd, &[0x03])?;
                }
            }
        }

        if fds[1].revents().unwrap_or(PollFlags::empty()).contains(PollFlags::POLLIN) {
            let n = match read(master_fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            };
            // The guarded shell must not turn paste mode off under us.
            let out = suppress_paste_off(&buf[..n]);
            if !out.is_empty() {
                write_all(stdout, &out)?;
            }
        }
    }

    Ok(end)
}

/// Removes `\x1b[?2004l` sequences from child output.
pub fn suppress_paste_off(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i..].starts_with(BP_OFF) {
            i += BP_OFF.len();
            continue;
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_a_bracketed_paste_in_one_read() {
        let mut tap = PasteTap::default();
        let feed = tap.feed(b"\x1b[200~rm -rf /\x1b[201~");
        assert!(feed.paste_complete);
        assert_eq!(feed.hold, b"rm -rf /");
        assert!(feed.forward.is_empty());
    }

    #[test]
    fn a_marker_split_across_reads_is_still_found() {
        let mut tap = PasteTap::default();
        let a = tap.feed(b"\x1b[20");
        assert!(!a.paste_complete);
        let b = tap.feed(b"0~hello\x1b[201~");
        assert!(b.paste_complete);
        assert_eq!(b.hold, b"hello");
    }

    #[test]
    fn ordinary_typing_is_forwarded_immediately() {
        let mut tap = PasteTap::default();
        let feed = tap.feed(b"ls\r");
        assert_eq!(feed.forward, b"ls\r");
        assert!(!feed.paste_complete);
    }

    #[test]
    fn burst_detection_matches_the_cpp_thresholds() {
        assert!(is_unbracketed_burst(b"a\nb"));
        assert!(is_unbracketed_burst(b"12345678"));
        assert!(!is_unbracketed_burst(b"1234567"));
        assert!(!is_unbracketed_burst(b"\x1b[A"));
    }

    #[test]
    fn paste_mode_off_is_suppressed_from_child_output() {
        let out = suppress_paste_off(b"ok\x1b[?2004ldone");
        assert_eq!(out, b"okdone");
    }

    #[test]
    fn review_names_the_rule_and_the_command() {
        let cfg = Config::default();
        let m = crate::classify("rm -rf /tmp/x");
        let text = render_review(b"rm -rf /tmp/x", &m, &cfg, &[]);
        assert!(text.contains("HIGH RISK: rm -rf"));
        assert!(text.contains("rm -rf /tmp/x"));
        assert!(text.contains("Execute?"));
    }

    #[test]
    fn a_config_problem_is_shown_in_the_review_block() {
        let cfg = Config::default();
        let m = Match::safe();
        let text = render_review(b"echo hi", &m, &cfg, &["rule `x` does not compile".into()]);
        assert!(text.contains("[CONFIG]"));
    }
}
