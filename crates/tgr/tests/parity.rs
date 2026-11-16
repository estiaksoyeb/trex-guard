//! Differential test: `tgr` against the real C++ guard.
//!
//! The ported table in `cpp_suite.rs` proves `tgr` matches *my reading* of the
//! C++ suite. That is circular. This test removes the middleman: it feeds the
//! same pastes to the compiled `tg` binary and to `tgr`'s classifier and
//! requires the risk to agree.
//!
//! # How `tg` is asked
//!
//! `tg` has no "classify this string" mode, so this drives its real review
//! prompt: start `trex-guard`, write a paste, and read the `[HIGH RISK]` /
//! `[REVIEW]` / auto-approve behaviour. That is the behaviour that matters,
//! not the internal enum.
//!
//! If the C++ binary is not built, the test **skips loudly** rather than
//! failing a fresh checkout, but a CI run with the binary present must pass.

use std::path::{Path, PathBuf};
use std::process::Command;

use tgr::{classify, Risk};

fn guard_binary() -> Option<PathBuf> {
    let candidates = [
        Path::new("../../build/trex-guard"),
        Path::new("../../build-release/trex-guard"),
        Path::new("../../trex-guard"),
    ];
    for c in candidates {
        if c.exists() {
            return Some(c.to_path_buf());
        }
    }
    if let Ok(out) = Command::new("sh").arg("-c").arg("command -v trex-guard").output() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

#[path = "support/corpus.rs"]
mod corpus;


/// Runs the real C++ guard through a pty and reads its decision.
///
/// The guard needs a terminal, so a pipe cannot drive it; the helper script
/// forks a pty, sends the paste as a bracketed paste, and reports the banner.
fn cpp_verdict(binary: &Path, paste: &str) -> Option<Risk> {
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("support")
        .join("drive_cpp.py");
    let out = Command::new("python3")
        .arg(helper)
        .arg(binary)
        .arg(paste)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(match String::from_utf8_lossy(&out.stdout).trim() {
        "DANGER" => Risk::Danger,
        "REVIEW" => Risk::Review,
        "SAFE" => Risk::Safe,
        other => {
            eprintln!("unexpected verdict from helper: {other:?}");
            return None;
        }
    })
}

#[test]
fn tgr_agrees_with_the_cpp_guard_on_the_corpus() {
    let Some(binary) = guard_binary() else {
        eprintln!(
            "SKIP: no trex-guard binary found (build with `cmake -B build && cmake --build build`). \
             Parity is unverified in this run."
        );
        return;
    };

    let mut compared = 0;
    let mut divergences = Vec::new();

    for paste in corpus::PASTES.iter().copied() {
        let Some(cpp) = cpp_verdict(&binary, paste) else {
            divergences.push(format!("{paste:?}: could not drive the C++ guard"));
            continue;
        };
        compared += 1;
        let ours = classify(paste).risk;
        if ours != cpp {
            divergences.push(format!(
                "{paste:?}\n  cpp: {cpp:?}\n  tgr: {ours:?} (rule {:?})",
                classify(paste).rule
            ));
        }
    }

    assert!(
        compared > 0,
        "no paste produced a review banner; the harness never reached the guard"
    );
    assert!(
        divergences.is_empty(),
        "{} divergences from the C++ guard:\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}
