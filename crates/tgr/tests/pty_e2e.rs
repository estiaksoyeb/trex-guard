//! End-to-end tests of `tgr` through a real terminal.
//!
//! The classifier is covered by unit tests and by `cpp_suite.rs`; the parity
//! test covers agreement with the C++ binary. What neither can check is the
//! *terminal* half: that a paste is intercepted, that a safe paste is
//! forwarded, that a dangerous one is held until a key is pressed, and that
//! the prompt's answer actually decides whether the command runs.
//!
//! Those need a pty, so this drives the built binary through `drive_pty.py`,
//! exactly as a person at a keyboard would.
//!
//! # Why the shell is scripted with `--norc`
//!
//! The tests run the user's real guard against a real bash. `--norc` keeps the
//! user's own `~/.bashrc` (tmux, starship, ble.sh) out of the test, so a test
//! failure means `tgr` is wrong rather than that the developer's prompt
//! plugin changed. The guard itself is unmodified.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tgr_binary() -> PathBuf {
    // `CARGO_BIN_EXE_tgr` is set by cargo for integration tests of a binary
    // target, so this always tests the binary just built.
    PathBuf::from(env!("CARGO_BIN_EXE_tgr"))
}

fn helper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("support")
        .join("drive_pty.py")
}

/// Runs `tgr --norc` with `paste` and returns (screen text, wait status).
///
/// `reply` is what the user types at the prompt; `None` means "do not reply"
/// (used for pastes that should be auto-approved).
/// A single-quoted Python string literal.
fn py_repr(s: &str) -> String {
    format!("{s:?}")
}

fn drive_with(paste: &str, reply: Option<&str>) -> (String, i32) {
    // Values are passed as Python repr literals so quoting is Python's job:
    // a paste may contain quotes, backslashes or newlines.
    let script = format!(
        r#"
import sys
sys.path.insert(0, {support})
from drive_pty import drive, plain
out, status = drive({binary}, {paste}, {reply}, extra_args=("--norc",))
sys.stdout.write(plain(out))
sys.exit(0 if status == 9 else status)  # 9 = SIGKILL from the driver, expected
"#,
        support = py_repr(helper().parent().unwrap().to_str().unwrap()),
        binary = py_repr(tgr_binary().to_str().unwrap()),
        paste = py_repr(paste),
        reply = py_repr(reply.unwrap_or("")),
    );
    let out = Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 must be available for the pty tests");

    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn help_and_version_exit_zero_without_a_terminal() {
    // These must work in a pipeline: they are what a user, a script, or a
    // package manager asks before anything else.
    for flag in ["--help", "-h"] {
        let out = Command::new(tgr_binary()).arg(flag).output().unwrap();
        assert!(out.status.success(), "{flag} did not exit 0");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("Usage:"), "{flag} printed no usage");
        assert!(text.contains("--force"), "{flag} omits --force");
    }
    let out = Command::new(tgr_binary()).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("tgr version"));
}

#[test]
fn a_dangerous_paste_is_held_and_cancelling_runs_nothing() {
    let marker = PathBuf::from(format!("/tmp/tgr-e2e-cancel-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);

    // `shred` is a `Review` rule ("file deletion") and really deletes the
    // file, so "cancelled" can be proven by the marker surviving.
    std::fs::write(&marker, b"must survive").unwrap();
    let paste = format!("shred -u {}", marker.display());
    let (screen, _) = drive_with(&paste, Some("n"));

    assert!(
        screen.contains("[REVIEW"),
        "no review banner was shown for a Review paste:\n{screen}"
    );
    assert!(
        screen.contains("[cancelled]"),
        "cancelling did not report itself:\n{screen}"
    );
    assert!(
        marker.exists(),
        "the cancelled command deleted its target; the guard forwarded it:\n{screen}"
    );
    assert_eq!(
        std::fs::read(&marker).unwrap(),
        b"must survive",
        "the cancelled command ran anyway"
    );
    let _ = std::fs::remove_file(&marker);
}

#[test]
fn a_dangerous_paste_runs_only_when_approved() {
    let marker = PathBuf::from(format!("/tmp/tgr-e2e-approve-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);

    std::fs::write(&marker, b"should be shredded").unwrap();
    let paste = format!("shred -u {}", marker.display());
    let (screen, _) = drive_with(&paste, Some("y"));

    assert!(
        screen.contains("[REVIEW"),
        "no review banner was shown for a Review paste:\n{screen}"
    );
    assert!(
        screen.contains("[approved]"),
        "approving did not report itself:\n{screen}"
    );
    assert!(
        !marker.exists(),
        "approved command did not run; the guard swallowed the payload:\n{screen}"
    );
    let _ = std::fs::remove_file(&marker);
}

#[test]
fn a_safe_paste_needs_no_prompt() {
    let (screen, _) = drive_with("echo nothing-risky-here", None);
    assert!(
        !screen.contains("Execute?"),
        "a safe paste was sent to a prompt:\n{screen}"
    );
    assert!(
        !screen.contains("[HIGH RISK") && !screen.contains("[REVIEW"),
        "a safe paste was flagged:\n{screen}"
    );
}

#[test]
fn a_review_paste_is_prompted_but_not_treated_as_danger() {
    let (screen, _) = drive_with("git reset --hard HEAD", Some("n"));
    assert!(
        screen.contains("[REVIEW"),
        "a Review paste got no banner:\n{screen}"
    );
    assert!(
        !screen.contains("[HIGH RISK"),
        "a Review paste was escalated to Danger:\n{screen}"
    );
}

#[test]
fn tg_still_starts_inside_a_tgr_session() {
    // The whole reason for a separate marker. `scripts/tg` exits with a notice
    // when TREX_GUARD_ACTIVE=1; if tgr set that variable, a tgr session could
    // never run the C++ guard. This runs the real installed `tg` inside a real
    // `tgr` and requires it to answer for itself.
    if which("tg").is_none() {
        eprintln!("SKIP: no `tg` on PATH");
        return;
    }
    let (screen, _) = drive_with("tg --version\n", None);
    assert!(
        screen.contains("trex-guard wrapper"),
        "tg refused to start inside tgr, so the session markers are confused:\n{screen}"
    );
    assert!(
        !screen.contains("already active"),
        "tg thought a tg session was already active inside tgr:\n{screen}"
    );
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

#[test]
fn the_child_sees_the_tgr_marker_not_the_tg_one() {
    // Two guards must not mistake each other for themselves. If tgr exported
    // TREX_GUARD_ACTIVE, `scripts/tg` would refuse to start a nested tg and
    // users would lose the C++ guard inside a tgr session.
    let probe = PathBuf::from(format!("/tmp/tgr-e2e-env-{}", std::process::id()));
    let _ = std::fs::remove_file(&probe);

    // A paste carrying its own newline is executed by the line editor on
    // insert, so this runs without needing an approval key. The shell decides
    // the answer, so the quoting stays readable: "set" or "unset".
    let paste = format!(
        "if [ \"$TGR_ACTIVE\" = 1 ]; then echo 1 > {}; else echo not1 > {}; fi; \
         if [ \"$TREX_GUARD_ACTIVE\" = 1 ]; then echo 1 >> {}; else echo not1 >> {}; fi\n",
        probe.display(),
        probe.display(),
        probe.display(),
        probe.display()
    );

    let (screen, _) = drive_with(&paste, None);
    // The shell runs the pasted line a moment after the driver stops
    // reading, so give it a chance to finish before inspecting the result.
    let mut seen = String::new();
    for _ in 0..50 {
        if let Ok(text) = std::fs::read_to_string(&probe) {
            seen = text;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = std::fs::remove_file(&probe);

    let lines: Vec<&str> = seen.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("1"),
        "the guarded shell must get TGR_ACTIVE=1 (read: {seen:?}):\n{screen}"
    );
    // The driver, like `scripts/tg`, clears this before starting tgr, so an
    // unset result proves tgr did not re-create the C++ guard's marker.
    assert_eq!(
        lines.get(1).copied(),
        Some("not1"),
        "tgr must not set TREX_GUARD_ACTIVE, or `scripts/tg` would think a tg \
         session is active (read: {seen:?}):\n{screen}"
    );
}

/// Drives `tgr`, types `keys`, and waits for the guard **process** to exit.
///
/// Returns `Some(status)` if it exited within `deadline`, `None` if it had to
/// be killed. Separate from `drive_with` because that driver always SIGKILLs
/// its child, so it cannot tell "exited" from "hung" — which is exactly the
/// bug this checks for.
fn drive_until_exit(keys: &str, deadline: std::time::Duration) -> Option<i32> {
    let script = format!(
        r#"
import sys, os, pty, time, select, signal
sys.path.insert(0, {support})
from drive_pty import plain

pid, fd = pty.fork()
if pid == 0:
    os.environ.pop("TGR_ACTIVE", None)
    os.environ["PS1"] = "TGRPROMPT> "
    os.execv({binary}, [{binary}, "--norc"])

out = b""
def pump(seconds):
    global out
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return False
            if not chunk:
                return False
            out += chunk
    return True

# Wait for the prompt so the keystrokes reach a live shell.
start = time.time()
while time.time() - start < 5.0:
    if not pump(0.05):
        break
    if b"TGRPROMPT>" in out:
        break

# Keys arrive as a list of byte values, so a control character or a CR
# cannot be mangled by string escaping on either side.
for key in {keys}:
    os.write(fd, bytes([key]))
    pump(0.15)

# Now the real question: does the guard itself go away?
#
# EOF on the pty (EIO) only means the *shell* is gone; the guard still has to
# reap it and exit. So keep waiting for the guard after EOF rather than
# stopping at it -- breaking here would report a hang that is not there.
eof = False
end = time.time() + {deadline}
while time.time() < end:
    if not eof and not pump(0.05):
        eof = True
    w, status = os.waitpid(pid, os.WNOHANG)
    if w:
        try:
            os.close(fd)
        except OSError:
            pass
        sys.stdout.write("EXITED " + str(os.waitstatus_to_exitcode(status)))
        sys.exit(0)

os.kill(pid, signal.SIGKILL)
os.waitpid(pid, 0)
sys.stdout.write("HUNG")
sys.exit(0)
"#,
        support = py_repr(helper().parent().unwrap().to_str().unwrap()),
        binary = py_repr(tgr_binary().to_str().unwrap()),
        keys = {
            // A Python list literal of byte values. `{:?}` on a Rust string
            // would emit `\u{4}`, which is a Python syntax error, so the
            // script would die and the test would report a hang that is not
            // there.
            let items: Vec<String> = keys.bytes().map(|b| b.to_string()).collect();
            format!("[{}]", items.join(", "))
        },
        deadline = 3.0,
    );
    let out = Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 must be available for the pty tests");
    let text = String::from_utf8_lossy(&out.stdout);
    match text.split_whitespace().next() {
        Some("EXITED") => text
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok()),
        _ => None,
    }
}

#[test]
fn the_guard_exits_when_the_shell_does() {
    // The reported bug: typing `exit` printed `exit` and then nothing. The
    // shell had exited and was left as a zombie while `tgr` spun at 100% CPU.
    //
    // Cause: when the pty slave closes, `poll` reports the master as always
    // ready with POLLHUP. The relay only asked for and checked POLLIN, so it
    // never read the EOF, never returned to `waitpid`, and looped as fast as
    // the CPU allowed.
    assert_eq!(
        drive_until_exit("exit\r", std::time::Duration::from_secs(3)),
        Some(0),
        "tgr did not exit after the shell was told to"
    );
}

#[test]
fn ctrl_d_exits_the_guard_too() {
    // Ctrl-D is EOF for the shell: same path, same hang before the fix.
    let status = drive_until_exit("\u{4}\u{4}", std::time::Duration::from_secs(3));
    assert!(
        status.is_some(),
        "tgr did not exit after Ctrl-D (the shell treats it as EOF)"
    );
}

/// Starts `tgr` in a pty, sends `signal`, and waits for the guard to exit.
///
/// Returns `Some(elapsed_secs, orphaned_children)` if it exited within the
/// deadline, `None` if it had to be killed. `orphaned_children` is read from
/// `/proc` after the guard is gone, so a shell left running behind the guard
/// is reported rather than missed.
fn drive_until_signalled(signal: &str, deadline: std::time::Duration) -> Option<(f64, usize)> {
    let script = format!(
        r#"
import sys, os, pty, time, select, signal

pid, fd = pty.fork()
if pid == 0:
    os.environ.pop("TGR_ACTIVE", None)
    os.environ["PS1"] = "TGRPROMPT> "
    os.execv({binary}, [{binary}, "--norc"])

out = b""
start = time.time()
while time.time() - start < 5.0:
    r, _, _ = select.select([fd], [], [], 0.05)
    if r:
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
        if b"TGRPROMPT>" in out:
            break

# Record the children before signalling, so what is left afterwards is
# exactly what the guard failed to clean up.
# `ps` rather than /proc/<pid>/task/<tid>/children, which is not readable
# from here.
listing = os.popen("ps -o pid= --ppid " + str(pid)).read().split()
before = set(int(c) for c in listing if c.isdigit())

os.kill(pid, getattr(signal, {signal}))
t0 = time.time()
while time.time() - t0 < {deadline}:
    w, status = os.waitpid(pid, os.WNOHANG)
    if w:
        break
    time.sleep(0.05)
else:
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    sys.stdout.write("HUNG")
    sys.exit(0)

# Give the kernel a moment to reparent, then count survivors.
time.sleep(0.2)
alive = 0
for c in before:
    try:
        os.kill(c, 0)
        alive += 1
        os.kill(c, signal.SIGKILL)
    except OSError:
        pass
sys.stdout.write("EXITED %.2f %d" % (time.time() - t0, alive))
"#,
        binary = py_repr(tgr_binary().to_str().unwrap()),
        signal = py_repr(signal),
        deadline = deadline.as_secs_f64(),
    );
    let out = Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 must be available for the pty tests");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace();
    if parts.next() != Some("EXITED") {
        return None;
    }
    let secs = parts.next().and_then(|s| s.parse().ok())?;
    let orphans = parts.next().and_then(|s| s.parse().ok())?;
    Some((secs, orphans))
}

#[test]
fn sigterm_ends_the_guard_and_the_shell() {
    // The second reported bug: `kill tgr` did nothing. The handler set a flag
    // that stopped the relay loop, but the signal was never passed to the
    // shell, so `waitpid` blocked on a shell that was still running.
    //
    // Interactive bash ignores SIGTERM and SIGHUP by design, so forwarding
    // alone is not enough: the guard waits briefly, then kills the shell the
    // way closing the terminal would.
    let (secs, orphans) = drive_until_signalled("SIGTERM", std::time::Duration::from_secs(3))
        .expect("tgr did not exit after SIGTERM");
    assert!(
        secs < 3.0,
        "tgr took {secs:.2}s to exit after SIGTERM, so it hung first"
    );
    assert_eq!(orphans, 0, "the guarded shell survived tgr");
}
