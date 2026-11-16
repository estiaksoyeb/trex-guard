//! Watch rules must be invisible to the paste path.
//!
//! `tgr` gained named, risk-free rules that `--check` reports for the `tgw`
//! watchdog (`crate::watch`). The whole justification for them being safe is
//! that the human pasting a command sees **exactly** what they would see
//! without them. If that is false, `tgr` has silently stopped being a clone of
//! `tg`, which is the one thing the port forbids.
//!
//! These tests drive the real binary through a pty with the default watch
//! rules present and with `[watch-rules]` disabling every one of them, and
//! require byte-identical output. They also require the paste path to hold the
//! same commands either way, so "identical" cannot mean "both did nothing".

use std::path::{Path, PathBuf};
use std::process::Command;

fn tgr_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgr"))
}

fn helper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("support")
        .join("drive_pty.py")
}

fn py_repr(s: &str) -> String {
    format!("{s:?}")
}

/// Drives the paste path with a given config file and returns the raw screen.
///
/// `drive_pty.py` passes `extra_args` to the guard, so the config path goes in
/// through the environment the guard already honours (`XDG_CONFIG_HOME`).
fn paste_with_config(paste: &str, config_dir: &Path) -> String {
    let script = format!(
        r#"
import os, sys
sys.path.insert(0, {support})
from drive_pty import drive, plain
os.environ["XDG_CONFIG_HOME"] = {config_dir}
out, status = drive({binary}, {paste}, "n", extra_args=("--norc",))
sys.stdout.write(plain(out))
"#,
        support = py_repr(helper().parent().unwrap().to_str().unwrap()),
        binary = py_repr(tgr_binary().to_str().unwrap()),
        paste = py_repr(paste),
        config_dir = py_repr(config_dir.to_str().unwrap()),
    );
    let out = Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 must be available for the pty tests");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Makes a config dir containing `config.ini` with the given body.
fn config_dir(tag: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tgr-watch-{tag}-{}", std::process::id()));
    let cfg = dir.join("trex-guard");
    std::fs::create_dir_all(&cfg).expect("create config dir");
    std::fs::write(cfg.join("config.ini"), body).expect("write config");
    dir
}

/// Every default watch rule disabled, so the paste path cannot see any.
const ALL_WATCH_RULES_DISABLED: &str = "\
[watch-rules]
disable = sed-i
disable = git-reset
disable = git-checkout
disable = git-clean
disable = git-restore
disable = rm
disable = mv
disable = chmod
disable = chown
disable = dd
disable = redirect-outside-tmp
";

/// Pastes that the watch rules match. If the rules leaked into the paste
/// path, at least one of these would change its output.
const WATCH_MATCHING_PASTES: &[&str] = &[
    "sed -i 's/a/b/' /tmp/watch-f",
    "git reset --hard HEAD",
    "git checkout -- .",
    "git clean -fd",
    "git restore .",
    "rm /tmp/watch-f",
    "mv /tmp/watch-a /tmp/watch-b",
    "chmod 644 /tmp/watch-f",
    "chown 0 /tmp/watch-f",
    "dd if=/dev/zero of=/tmp/watch-f count=1",
    "echo x > /tmp/watch-f",
];

#[test]
fn watch_rules_do_not_change_any_paste() {
    let on = config_dir("on", "");
    let off = config_dir("off", ALL_WATCH_RULES_DISABLED);

    let mut diffs = Vec::new();
    for &paste in WATCH_MATCHING_PASTES {
        let a = paste_with_config(paste, &on);
        let b = paste_with_config(paste, &off);
        if a != b {
            diffs.push(format!("  {paste:?}\n   default: {a:?}\n   disabled: {b:?}"));
        }
    }
    assert!(
        diffs.is_empty(),
        "watch rules changed the paste path on {} paste(s):\n{}",
        diffs.len(),
        diffs.join("\n")
    );
    let _ = std::fs::remove_dir_all(&on);
    let _ = std::fs::remove_dir_all(&off);
}

#[test]
fn a_dangerous_paste_is_still_held_with_watch_rules_present() {
    // "identical output" must not be satisfied by both paths being inert.
    // `rm -rf` is Danger, so the guard must still stop it.
    let on = config_dir("held", "");
    let screen = paste_with_config("rm -rf /tmp/watch-should-not-exist", &on);
    let _ = std::fs::remove_dir_all(&on);
    assert!(
        screen.contains("HIGH RISK"),
        "a dangerous paste was not held with watch rules present:\n{screen}"
    );
}

#[test]
fn a_watch_matched_paste_is_not_held() {
    // The point of the whole design: `sed -i` is destructive but ordinary.
    // It must NOT raise a review prompt, or the terminal behaves differently
    // from `tg`.
    let on = config_dir("nothold", "");
    let screen = paste_with_config("sed -i 's/a/b/' /tmp/watch-f", &on);
    let _ = std::fs::remove_dir_all(&on);
    assert!(
        !screen.contains("[REVIEW"),
        "a watch rule raised a review prompt on the paste path:\n{screen}"
    );
    assert!(
        !screen.contains("[CONFIG]"),
        "a watch rule produced a config warning on the paste path:\n{screen}"
    );
}
