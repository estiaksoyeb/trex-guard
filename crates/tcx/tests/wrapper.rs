//! Tests for the `scripts/tcx` wrapper.
//!
//! The wrapper is a tiny shell script whose only job is to find the real
//! binary. It has already caused one infinite loop (exec'ing itself when the
//! scripts directory was on `PATH`), so that behaviour is pinned here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

/// Absolute path to the repository's `scripts/tcx`.
fn wrapper() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/tcx; the repo root is two levels up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/tcx")
}

fn run(env_path: &str, args: &[&str]) -> Output {
    Command::new("sh")
        .arg(wrapper())
        .args(args)
        .env("PATH", env_path)
        .env("TCX_HOME", std::env::temp_dir().join("tcx-wrapper-test"))
        .output()
        .expect("run wrapper")
}

#[test]
fn wrapper_finds_the_binary_from_the_repo() {
    // PATH deliberately excludes the scripts dir, so resolution goes through
    // the repo build directories.
    let out = run("/usr/bin:/bin", &["--version"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("tcx"), "got: {text}");
}

#[test]
fn wrapper_does_not_recurse_when_scripts_is_on_path() {
    // The original bug: `command -v tcx` found the wrapper itself and exec'd
    // it forever. A timeout makes a regression a failure instead of a hang.
    let scripts = wrapper().parent().unwrap().to_path_buf();
    let path = format!("{}:/usr/bin:/bin", scripts.display());

    let mut child = Command::new("sh")
        .arg(wrapper())
        .args(["--version"])
        .env("PATH", &path)
        .env("TCX_HOME", std::env::temp_dir().join("tcx-wrapper-test"))
        .spawn()
        .expect("spawn wrapper");

    // Wait with a bound so a recursion bug fails fast rather than hanging CI.
    let start = std::time::Instant::now();
    loop {
        match child.try_wait().expect("wait") {
            Some(status) => {
                assert!(
                    status.success(),
                    "wrapper failed: {status:?}"
                );
                return;
            }
            None => {
                if start.elapsed() > Duration::from_secs(10) {
                    let _ = child.kill();
                    panic!("wrapper recursed or hung with scripts/ on PATH");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn wrapper_help_works() {
    let out = run("/usr/bin:/bin", &["--help"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("TREX ConteXt"), "got: {text}");
}
