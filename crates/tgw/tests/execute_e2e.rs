//! Execution and `hold_rules`, through the binary.
//!
//! The point of these tests is the safety property, not the feature: a gate
//! answer must map to a run **only** when the mode allows it and no
//! `hold_rules` name matched. Each test below is a way to try to get a run
//! without a pass, and each must fail to.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tgw() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgw"))
}

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("tgw-exec-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        Sandbox { dir }
    }

    fn script(&self, name: &str, body: &str) -> PathBuf {
        let p = self.dir.join("bin").join(name);
        std::fs::write(&p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    /// The path a block will touch so the test can tell it ran.
    fn witness(&self) -> PathBuf {
        self.dir.join("witness")
    }

    fn config(&self, extra: &str) {
        let clip = self.script("clip", "#!/bin/sh\ncat \"$TGW_CLIP\"\n");
        let gate = self.script(
            "gate",
            "#!/bin/sh\nprintf '%s\\n' \"$TGW_GATE_ERR\" >&2\nexit \"${TGW_GATE_EXIT:-0}\"\n",
        );
        let ini = format!(
            "clip_get_cmd = {clip}\ngate_cmd = {gate} {{file}}\n{extra}\n",
            clip = clip.display(),
            gate = gate.display()
        );
        std::fs::write(self.dir.join("config").join("watch.ini"), ini).unwrap();
    }

    fn run(&self, args: &[&str], text: &str, gate_exit: &str, gate_err: &str) -> Output {
        let clip = self.dir.join("clip.txt");
        std::fs::write(&clip, text).unwrap();
        Command::new(tgw())
            .args(args)
            .env("TCX_HOME", &self.dir)
            .env("TGW_CLIP", clip)
            .env("TGW_GATE_EXIT", gate_exit)
            .env("TGW_GATE_ERR", gate_err)
            // Deliberately hostile: if env filtering is broken the block will
            // see these. A block must only see env_allow.
            .env("LEAKY_TOKEN", "should-not-be-visible")
            .output()
            .expect("run tgw")
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("data/watch/log/events.log")).unwrap_or_default()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A block that records that it ran.
fn touch(w: &Path) -> String {
    format!("set +H\ntouch {}\n", w.display())
}

#[test]
fn auto_mode_runs_a_passing_block() {
    let sb = Sandbox::new("autorun");
    sb.config("mode = auto");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(w.exists(), "the block did not run in auto:\n{stdout}");
    assert!(stdout.contains("run"), "{stdout}");
    assert!(sb.log().contains("ran successfully"), "{}", sb.log());
}

#[test]
fn dry_mode_never_runs_even_on_a_pass() {
    let sb = Sandbox::new("dry");
    sb.config("mode = dry");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "");
    assert!(!w.exists(), "dry executed a block:\n{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn confirm_mode_never_runs_without_approval() {
    let sb = Sandbox::new("confirm");
    sb.config("mode = confirm");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "");
    assert!(!w.exists(), "{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn a_hold_never_runs_in_auto_mode() {
    let sb = Sandbox::new("autohold");
    sb.config("mode = auto");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "3", "rule=rm risk=review");
    assert!(!w.exists(), "a held block ran in auto:\n{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn a_block_never_runs_in_auto_mode() {
    let sb = Sandbox::new("autoblock");
    sb.config("mode = auto");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "1", "rule=rm -rf risk=danger");
    assert!(!w.exists(), "a blocked block ran in auto:\n{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn a_gate_error_never_runs_in_auto_mode() {
    let sb = Sandbox::new("autoerr");
    sb.config("mode = auto");
    let w = sb.witness();
    for (code, err) in [("2", ""), ("7", ""), ("", "gate crashed")] {
        let out = sb.run(&["once"], &touch(&w), code, err);
        let _ = out;
        assert!(!w.exists(), "exit {code:?} ran a block in auto");
    }
}

#[test]
fn hold_rules_turns_a_pass_into_a_hold_in_auto_mode() {
    // The gate says safe. The human said "rm always needs a tap." The human
    // wins, and the block does not run.
    let sb = Sandbox::new("holdrule");
    sb.config("mode = auto\nhold_rules = rm");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "watch=rm");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!w.exists(), "a hold_rule was ignored:\n{stdout}");
    assert!(stdout.contains("hold"), "{stdout}");
    assert!(stdout.contains("hold_rules matched: rm"), "{stdout}");
}

#[test]
fn a_rule_name_also_matches_hold_rules() {
    // Names come from both `rule=` and `watch=`; the human should not have to
    // know which table the gate took the name from.
    let sb = Sandbox::new("holdrule2");
    sb.config("mode = auto\nhold_rules = git-clean");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "watch=git-clean");
    assert!(!w.exists(), "{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn an_unrelated_name_does_not_hold() {
    let sb = Sandbox::new("nohold");
    sb.config("mode = auto\nhold_rules = rm, mv");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "watch=sed-i");
    assert!(w.exists(), "an unrelated watch name held the block:\n{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn hold_rules_cannot_escalate_or_de_escalate_a_block() {
    // Naming a rule in hold_rules must not turn a danger into a hold.
    let sb = Sandbox::new("nodescalate");
    sb.config("mode = auto\nhold_rules = rm");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "1", "rule=rm risk=danger\nwatch=rm");
    assert!(!w.exists(), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(String::from_utf8_lossy(&out.stdout).contains("block"));
}

#[test]
fn a_block_only_sees_env_allow() {
    let sb = Sandbox::new("env");
    sb.config("mode = auto");
    let w = sb.witness();
    let text = format!(
        "set +H\necho \"${{LEAKY_TOKEN:-unset}}\" > {}\n",
        w.display()
    );
    sb.run(&["once"], &text, "0", "");
    let got = std::fs::read_to_string(&w).unwrap_or_default();
    assert_eq!(
        got.trim(),
        "unset",
        "a variable outside env_allow reached the block"
    );
}

#[test]
fn a_block_sees_the_in_block_marker() {
    let sb = Sandbox::new("marker");
    sb.config("mode = auto");
    let w = sb.witness();
    let text = format!(
        "set +H\necho \"${{TGW_IN_BLOCK:-unset}}\" > {}\n",
        w.display()
    );
    sb.run(&["once"], &text, "0", "");
    assert_eq!(std::fs::read_to_string(&w).unwrap().trim(), "1");
}

#[test]
fn a_failing_run_is_recorded_as_a_hold_not_a_success() {
    let sb = Sandbox::new("failrun");
    sb.config("mode = auto");
    let out = sb.run(&["once"], "set +H\nexit 9\n", "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hold"), "a failed run was reported as success:\n{stdout}");
    assert!(stdout.contains("exited 9"), "{stdout}");
}

#[test]
fn a_timed_out_run_is_reported_as_a_hold() {
    let sb = Sandbox::new("timeout");
    sb.config("mode = auto\ntimeout_s = 1");
    let out = sb.run(&["once"], "set +H\nsleep 30\n", "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("timed out"), "{stdout}");
}

#[test]
fn a_block_cannot_move_later_blocks() {
    let sb = Sandbox::new("cdguard");
    let cwdfile = sb.dir.join("cwd");
    std::fs::write(&cwdfile, "/tmp\n").unwrap();
    sb.config(&format!(
        "mode = auto\ncwd_mode = file\ncwd_file = {}",
        cwdfile.display()
    ));
    let w = sb.witness();
    let out = sb.run(
        &["once"],
        // The binary is named explicitly: a block sees only `env_allow`, and
        // the test process's PATH is not a promise about what the block gets.
        &format!(
            "set +H\n{} cd / >/dev/null 2>&1; echo \"$?\" > {}\n",
            tgw().display(),
            w.display()
        ),
        "0",
        "",
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let code = std::fs::read_to_string(&w).unwrap_or_default();
    assert_eq!(code.trim(), "2", "tgw cd succeeded inside a block:\n{stdout}");
    assert_eq!(std::fs::read_to_string(&cwdfile).unwrap().trim(), "/tmp");
}

#[test]
fn output_over_the_cap_is_marked_in_the_log() {
    let sb = Sandbox::new("cap");
    sb.config("mode = auto\nmax_output_bytes = 64");
    sb.run(
        &["once"],
        "set +H\ni=0; while [ $i -lt 500 ]; do echo 0123456789; i=$((i+1)); done\n",
        "0",
        "",
    );
    assert!(sb.log().contains("truncated"), "{}", sb.log());
}

#[test]
fn cwd_file_rereads_between_blocks() {
    let sb = Sandbox::new("cwdreread");
    let a = sb.dir.join("a");
    let b = sb.dir.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let cwdfile = sb.dir.join("cwd");
    std::fs::write(&cwdfile, format!("{}\n", a.display())).unwrap();
    sb.config(&format!(
        "mode = auto\ncwd_mode = file\ncwd_file = {}",
        cwdfile.display()
    ));

    let w1 = sb.dir.join("w1");
    sb.run(&["once"], &format!("set +H\npwd > {}\n", w1.display()), "0", "");
    assert!(std::fs::read_to_string(&w1).unwrap().contains("/a"));

    // A second block, after the file changed.
    std::fs::write(&cwdfile, format!("{}\n", b.display())).unwrap();
    let w2 = sb.dir.join("w2");
    sb.run(&["once"], &format!("set +H\npwd > {}\n", w2.display()), "0", "");
    assert!(
        std::fs::read_to_string(&w2).unwrap().contains("/b"),
        "cwd_file was not re-read"
    );
}

#[test]
fn tgw_cd_writes_an_absolute_path() {
    let sb = Sandbox::new("cd");
    let cwdfile = sb.dir.join("cwd");
    std::fs::write(&cwdfile, "/tmp\n").unwrap();
    sb.config(&format!(
        "cwd_mode = file\ncwd_file = {}",
        cwdfile.display()
    ));
    let out = Command::new(tgw())
        .args(["cd", "/usr"])
        .env("TCX_HOME", &sb.dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_to_string(&cwdfile).unwrap().trim(), "/usr");
}

#[test]
fn tgw_cd_refuses_relative_and_missing_paths() {
    let sb = Sandbox::new("cdbad");
    let cwdfile = sb.dir.join("cwd");
    std::fs::write(&cwdfile, "/tmp\n").unwrap();
    sb.config(&format!("cwd_mode = file\ncwd_file = {}", cwdfile.display()));
    for bad in ["relative/dir", "/nonexistent/tgw-dir"] {
        let out = Command::new(tgw())
            .args(["cd", bad])
            .env("TCX_HOME", &sb.dir)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{bad} was accepted");
        assert_eq!(std::fs::read_to_string(&cwdfile).unwrap().trim(), "/tmp");
    }
}
