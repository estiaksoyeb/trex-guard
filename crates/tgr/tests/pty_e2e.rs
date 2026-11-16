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
