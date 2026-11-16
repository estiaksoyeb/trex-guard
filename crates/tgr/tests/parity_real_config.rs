//! Parity against the real C++ guard using the user's *actual* config.
//!
//! `parity.rs` uses the built-in rules only. This test uses the same corpus
//! plus the real `~/.config/trex-guard/config.ini`, so a divergence caused by
//! the 22 custom rules shows up here. It is the configuration the user's daily
//! shell is actually protected by.

#[path = "support/corpus.rs"]
mod corpus;

use std::path::{Path, PathBuf};
use std::process::Command;

use tgr::{config, rules, Risk};

fn guard_binary() -> Option<PathBuf> {
    for c in [
        Path::new("../../build/trex-guard"),
        Path::new("../../build-release/trex-guard"),
    ] {
        if c.exists() {
            return Some(c.to_path_buf());
        }
    }
    None
}

fn real_config() -> Option<config::Config> {
    let path = config::default_path();
    if path.as_os_str().is_empty() || !path.exists() {
        return None;
    }
    Some(config::load(Some(&path)))
}

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
            eprintln!("unexpected verdict: {other:?}");
            return None;
        }
    })
}

#[test]
fn tgr_agrees_with_the_cpp_guard_under_the_real_config() {
    let (Some(binary), Some(cfg)) = (guard_binary(), real_config()) else {
        eprintln!("SKIP: need both the C++ binary and a real config.ini");
        return;
    };
    assert!(
        !cfg.custom_rules.is_empty(),
        "the real config has no custom rules; this test would prove nothing"
    );

    let mut divergences = Vec::new();
    for paste in corpus::PASTES.iter().chain(corpus::CUSTOM_RULE_PASTES) {
        let Some(cpp) = cpp_verdict(&binary, paste) else {
            divergences.push(format!("{paste:?}: could not drive the C++ guard"));
            continue;
        };
        let ours = rules::classify_with(paste, Some(&cfg)).matched.risk;
        if ours != cpp {
            divergences.push(format!("{paste:?}\n  cpp: {cpp:?}\n  tgr: {ours:?}"));
        }
    }
    assert!(
        divergences.is_empty(),
        "{} divergences with the real config:\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}
