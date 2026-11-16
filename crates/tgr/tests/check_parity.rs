//! `tgr --check` must agree with the paste path.
//!
//! `--check` exists so another program (the `tgw` watchdog) can ask the guard
//! what it thinks of a block without starting a shell. That only means
//! something if the answer is the one a human pasting the same bytes would
//! get. These tests drive both paths over the shared corpus and require the
//! same verdict.
//!
//! The paste path is driven through a real pty, because the guard refuses to
//! run without a terminal (`tcgetattr` on stdin). `--check` is driven as an
//! ordinary subprocess, because that is how its caller will run it.
//!
//! If this fails, do not "fix" it by relaxing the comparison: either `--check`
//! drifted from `classify_with`, or the paste path did. Both are bugs.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/corpus.rs"]
mod corpus;

fn tgr_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgr"))
}

fn helper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("support")
        .join("drive_pty.py")
}

/// A single-quoted Python string literal.
fn py_repr(s: &str) -> String {
    format!("{s:?}")
}

/// Drives the paste path through a pty and returns the screen text.
fn paste_screen(paste: &str) -> String {
    let script = format!(
        r#"
import sys
sys.path.insert(0, {support})
from drive_pty import drive, plain
out, status = drive({binary}, {paste}, "n", extra_args=("--norc",))
sys.stdout.write(plain(out))
"#,
        support = py_repr(helper().parent().unwrap().to_str().unwrap()),
        binary = py_repr(tgr_binary().to_str().unwrap()),
        paste = py_repr(paste),
    );
    let out = Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 must be available for the pty tests");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn paste_verdict(paste: &str) -> &'static str {
    let text = paste_screen(paste);
    if text.contains("HIGH RISK") {
        "danger"
    } else if text.contains("REVIEW") {
        "review"
    } else {
        "safe"
    }
}

/// Runs `tgr --check <file>` and maps the exit code to a verdict word.
fn check_verdict(paste: &str) -> String {
    // A unique file per call. The two comparison tests run concurrently in
    // one process, so a shared path let one test read the other's block --
    // which showed up as `--check` "disagreeing" on a paste it never saw.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("tgr-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(format!("block-{n}"));
    std::fs::write(&path, paste).expect("write block");

    let out = Command::new(tgr_binary())
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run tgr --check");
    let _ = std::fs::remove_file(&path);

    match out.status.code() {
        Some(0) => "safe".into(),
        Some(1) => "danger".into(),
        Some(3) => "review".into(),
        other => panic!(
            "tgr --check {paste:?} exited {other:?}, which is not a verdict\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
    }
}

fn compare(label: &str, pastes: &[&str]) {
    let mut bad = Vec::new();
    for &paste in pastes {
        let a = paste_verdict(paste);
        let b = check_verdict(paste);
        if a != b {
            bad.push(format!("  {paste:?}: paste path said {a}, --check said {b}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{label}: --check disagrees with the paste path on {} paste(s):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn check_agrees_with_the_paste_path_on_built_in_rules() {
    compare("built-in rules", corpus::PASTES);
}

#[test]
fn check_agrees_with_the_paste_path_on_the_real_config() {
    // This corpus contains pastes that only fire under a custom rule, so it is
    // what proves `--check` reads the same config as the guard.
    compare("real config", corpus::CUSTOM_RULE_PASTES);
}
