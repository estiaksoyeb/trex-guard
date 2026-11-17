//! Execution and verdict mapping, through the binary.
//!
//! The point of these tests is the safety property, not the feature: a gate
//! answer must map to a run **only** when it was a pass and the mode allows
//! it. Each test below is a way to try to get a run without a pass, and each
//! must fail to.

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
    sb.config("mode = auto\nauto_unlock_approvals = 0");
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
    sb.config("mode = auto\nauto_unlock_approvals = 0");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "3", "rule=rm risk=review");
    assert!(!w.exists(), "a held block ran in auto:\n{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn a_block_never_runs_in_auto_mode() {
    let sb = Sandbox::new("autoblock");
    sb.config("mode = auto\nauto_unlock_approvals = 0");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "1", "rule=rm -rf risk=danger");
    assert!(!w.exists(), "a blocked block ran in auto:\n{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn a_gate_error_never_runs_in_auto_mode() {
    let sb = Sandbox::new("autoerr");
    sb.config("mode = auto\nauto_unlock_approvals = 0");
    let w = sb.witness();
    for (code, err) in [("2", ""), ("7", ""), ("", "gate crashed")] {
        let out = sb.run(&["once"], &touch(&w), code, err);
        let _ = out;
        assert!(!w.exists(), "exit {code:?} ran a block in auto");
    }
}

#[test]
fn a_pass_runs_in_auto_with_no_second_list() {
    // `hold_rules` is gone: the gate's exit code is the whole verdict. A pass
    // runs in auto, and there is no second list that can hold it.
    let sb = Sandbox::new("holdrule");
    sb.config("mode = auto\nauto_unlock_approvals = 0");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "watch=rm");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(w.exists(), "a plain pass did not run in auto:\n{stdout}");
}

#[test]
fn an_old_hold_rules_key_is_refused() {
    // An existing watch.ini may still carry the removed key. tgw must refuse
    // to start rather than ignore it, and the block must not run.
    let sb = Sandbox::new("oldkey");
    sb.config("mode = auto\nauto_unlock_approvals = 0\nhold_rules = rm");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "watch=rm");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!w.exists(), "a block ran with an unusable config:\n{stdout}{stderr}");
    assert!(stderr.contains("hold_rules"), "{stderr}");
}

#[test]
fn a_danger_is_still_a_block() {
    // The one invariant worth re-asserting now that `hold_rules` is gone:
    // nothing in watch.ini can turn a block into a hold.
    let sb = Sandbox::new("nodescalate");
    sb.config("mode = auto\nauto_unlock_approvals = 0");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "1", "rule=rm risk=danger\nwatch=rm");
    assert!(!w.exists(), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(String::from_utf8_lossy(&out.stdout).contains("block"));
}

#[test]
fn a_block_only_sees_env_allow() {
    let sb = Sandbox::new("env");
    sb.config("mode = auto\nauto_unlock_approvals = 0");
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
    sb.config("mode = auto\nauto_unlock_approvals = 0");
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
    sb.config("mode = auto\nauto_unlock_approvals = 0");
    let out = sb.run(&["once"], "set +H\nexit 9\n", "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hold"), "a failed run was reported as success:\n{stdout}");
    assert!(stdout.contains("exited 9"), "{stdout}");
}

#[test]
fn a_timed_out_run_is_reported_as_a_hold() {
    let sb = Sandbox::new("timeout");
    sb.config("mode = auto\nauto_unlock_approvals = 0\ntimeout_s = 1");
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
        "mode = auto\nauto_unlock_approvals = 0\ncwd_mode = file\ncwd_file = {}",
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
    sb.config("mode = auto\nauto_unlock_approvals = 0\nmax_output_bytes = 64");
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
        "mode = auto\nauto_unlock_approvals = 0\ncwd_mode = file\ncwd_file = {}",
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

#[test]
fn a_config_that_fails_to_parse_never_runs_a_block() {
    // Found while removing `hold_rules`: `config::load` discarded every parse
    // problem, so a typo'd key or a bad `mode` value left the block running on
    // a config that had not been understood. The gate is only as trustworthy
    // as the config describing it.
    for broken in ["bogus_key = 1", "mode = yolo"] {
        let sb = Sandbox::new("parsefail");
        sb.config(&format!("mode = auto\nauto_unlock_approvals = 0\n{broken}"));
        let w = sb.witness();
        let out = sb.run(&["once"], &touch(&w), "0", "");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !w.exists(),
            "a block ran with an unusable config ({broken}):\n{stderr}"
        );
        assert!(
            stderr.contains("not usable"),
            "{broken} was not reported as a problem:\n{stderr}"
        );
    }
}

#[test]
fn a_held_block_can_be_approved() {
    // The reported failure: "it showed hold but never ran". A hold is not a
    // run, so `approve` has to be able to find the record and execute it.
    let sb = Sandbox::new("approveflow");
    sb.config("mode = confirm\nhold_ttl_s = 300");
    let w = sb.witness();
    let out = sb.run(&["once"], &touch(&w), "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!w.exists(), "a confirm-mode block ran without approval:\n{stdout}");

    let id = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("id: "))
        // The line is `id: <hex>  (<ms> ms)`; take the id only.
        .and_then(|s| s.split_whitespace().next())
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no hold id printed:\n{stdout}"));

    let approved = sb.run(&["approve", &id], &touch(&w), "0", "");
    let aout = String::from_utf8_lossy(&approved.stdout);
    assert_eq!(
        approved.status.code(),
        Some(0),
        "approve failed for {id}:\n{aout}{}",
        String::from_utf8_lossy(&approved.stderr)
    );
    assert!(w.exists(), "the approved block did not run:\n{aout}");
}

#[test]
fn a_block_is_offered_again_once_its_hold_is_gone() {
    // Every staged hash used to be marked seen permanently, so a block that
    // was held and expired could never be offered again: copying it a second
    // time was silently ignored. That is why `tgw` looked like it stopped
    // working after the first attempt.
    let sb = Sandbox::new("reoffer");
    sb.config("mode = confirm\nhold_ttl_s = 300");
    let w = sb.witness();
    let block = touch(&w);

    let first = sb.run(&["once"], &block, "0", "");
    let fout = String::from_utf8_lossy(&first.stdout);
    assert!(fout.contains("hold"), "first pass was not held:\n{fout}");

    // The clipboard still holds the same block on the next poll: dedupe must
    // stop it being offered twice in a row.
    let again = sb.run(&["once"], &block, "0", "");
    let aout = String::from_utf8_lossy(&again.stdout);
    assert!(
        aout.contains("duplicate") || aout.contains("ignore"),
        "the same block was offered twice in a row:\n{aout}"
    );

    // Drop the hold as expiry would, then erase the seen marker's age so the
    // block is past its window without sleeping for it.
    let stage = sb.dir.join("data/watch/stage");
    for e in std::fs::read_dir(&stage).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "seen") {
            std::fs::write(e.path(), "0").unwrap();
        }
    }
    let third = sb.run(&["once"], &block, "0", "");
    let tout = String::from_utf8_lossy(&third.stdout);
    assert!(
        tout.contains("hold"),
        "a block was never offered again after its seen marker aged out:\n{tout}"
    );
}

#[test]
fn an_expired_hold_is_reported_not_deleted_silently() {
    // A hold that is announced and then vanishes is indistinguishable from
    // `tgw` never having seen the block.
    let sb = Sandbox::new("expired");
    sb.config("mode = confirm\nhold_ttl_s = 300");
    let w = sb.witness();
    sb.run(&["once"], &touch(&w), "0", "");

    // Age the hold past its expiry rather than waiting 300 s.
    let holds = sb.dir.join("data/watch/holds");
    let mut aged = 0;
    for e in std::fs::read_dir(&holds).unwrap().flatten() {
        let text = std::fs::read_to_string(e.path()).unwrap();
        let out: String = text
            .lines()
            .map(|l| {
                if let Some(v) = l.strip_prefix("expires=") {
                    let old: u64 = v.parse().unwrap();
                    format!("expires={}", old.saturating_sub(300))
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(e.path(), out).unwrap();
        aged += 1;
    }
    assert!(aged > 0, "no hold was recorded to expire");

    // `run` sweeps at startup and then loops forever, so it is started and
    // read until either the message appears or a deadline passes. Waiting for
    // the process to exit would hang the suite.
    let clip = sb.dir.join("clip.txt");
    std::fs::write(&clip, "").unwrap();
    let mut child = Command::new(tgw())
        .args(["run", "--interval", "50"])
        .env("TCX_HOME", &sb.dir)
        .env("TGW_CLIP", clip)
        .env("TGW_GATE_EXIT", "0")
        .env("TGW_GATE_ERR", "")
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("start tgw run");

    let mut stderr = String::new();
    {
        use std::io::Read;
        let pipe = child.stderr.as_mut().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut buf = [0u8; 512];
        while std::time::Instant::now() < deadline && !stderr.contains("expired") {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => stderr.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => break,
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        stderr.contains("expired"),
        "an expired hold was dropped without a word:\n{stderr}"
    );
}

#[test]
fn a_hold_that_cannot_be_saved_is_an_error_not_an_id() {
    // `holds::save` used to be `let _ = ...`. The hold was announced with an
    // id, `tgw approve` could not find it, and the only visible symptom was
    // that nothing ever ran.
    let sb = Sandbox::new("savefail");
    sb.config("mode = confirm\nhold_ttl_s = 300");
    let w = sb.witness();

    // Make the holds directory unwritable by replacing it with a file, so
    // `create_dir_all` cannot succeed.
    let holds = sb.dir.join("data/watch/holds");
    std::fs::create_dir_all(sb.dir.join("data/watch")).unwrap();
    std::fs::write(&holds, "not a directory").unwrap();

    let out = sb.run(&["once"], &touch(&w), "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let logged = sb.log();
    assert!(!w.exists(), "a block ran while its hold could not be saved:\n{stdout}");
    assert!(
        logged.contains("cannot save hold"),
        "a failed hold save was swallowed (log: {logged})"
    );
}
