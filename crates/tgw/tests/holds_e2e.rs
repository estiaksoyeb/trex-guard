//! Holds, approvals and denials, through the binary.
//!
//! The safety property under test: a hold runs **only** if the bytes are the
//! bytes that were held and the gate still passes them. Each test below is a
//! way to get something else to run, and each must be refused.

use std::path::PathBuf;
use std::process::{Command, Output};

fn tgw() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgw"))
}

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("tgw-hold-{tag}-{}", std::process::id()));
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

    fn config(&self, extra: &str) {
        let clip = self.script("clip", "#!/bin/sh\ncat \"$TGW_CLIP\"\n");
        // The gate's answer comes from a file, so a test can change it between
        // the hold and the approval.
        let answer = self.dir.join("gate-answer");
        std::fs::write(&answer, "0").unwrap();
        let err = self.dir.join("gate-err");
        std::fs::write(&err, "").unwrap();
        let gate = self.script(
            "gate",
            &format!(
                "#!/bin/sh\ncat {err} >&2\nexit \"$(cat {ans})\"\n",
                err = err.display(),
                ans = answer.display()
            ),
        );
        let ini = format!(
            "clip_get_cmd = {clip}\ngate_cmd = {gate} {{file}}\n{extra}\n",
            clip = clip.display(),
            gate = gate.display()
        );
        std::fs::write(self.dir.join("config").join("watch.ini"), ini).unwrap();
    }

    fn set_gate(&self, code: &str, stderr: &str) {
        std::fs::write(self.dir.join("gate-answer"), code).unwrap();
        std::fs::write(self.dir.join("gate-err"), stderr).unwrap();
    }

    fn run(&self, args: &[&str], text: &str) -> Output {
        let clip = self.dir.join("clip.txt");
        std::fs::write(&clip, text).unwrap();
        Command::new(tgw())
            .args(args)
            .env("TCX_HOME", &self.dir)
            .env("TGW_CLIP", clip)
            .output()
            .expect("run tgw")
    }

    fn plain(&self, args: &[&str]) -> Output {
        Command::new(tgw())
            .args(args)
            .env("TCX_HOME", &self.dir)
            .output()
            .expect("run tgw")
    }

    fn state(&self) -> PathBuf {
        self.dir.join("data/watch")
    }

    /// Stages one block and returns its hold id.
    fn hold(&self, text: &str) -> String {
        let out = self.run(&["once"], text);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout
            .lines()
            .find(|l| l.starts_with("id: "))
            .unwrap_or_else(|| panic!("no id in output:\n{stdout}"));
        line["id: ".len()..]
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.state().join("log/events.log")).unwrap_or_default()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn touch(w: &std::path::Path) -> String {
    format!("set +H\ntouch {}\n", w.display())
}

#[test]
fn a_confirm_mode_pass_creates_a_pending_hold() {
    let sb = Sandbox::new("pending");
    sb.config("mode = confirm");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));
    assert!(!w.exists(), "confirm ran the block");
    let status = sb.plain(&["status"]);
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains(&id), "the hold is not listed:\n{stdout}");
    assert!(stdout.contains("pending holds: 1"), "{stdout}");
}

#[test]
fn approve_runs_the_block_and_clears_the_hold() {
    let sb = Sandbox::new("approve");
    sb.config("mode = confirm");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));
    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(w.exists(), "approve did not run the block");
    assert!(!sb.state().join("holds").join(format!("{id}.hold")).exists());
    let status = sb.plain(&["status"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("pending holds: 0"));
}

#[test]
fn deny_removes_the_hold_and_never_runs_it() {
    let sb = Sandbox::new("deny");
    sb.config("mode = confirm");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));
    let out = sb.plain(&["deny", &id]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!w.exists(), "deny ran the block");
    assert!(!sb.state().join("holds").join(format!("{id}.hold")).exists());
    assert!(sb.log().contains("denied by human"), "{}", sb.log());
}

#[test]
fn editing_a_held_block_after_the_hold_refuses_it() {
    // The attack this whole module exists for: hold something harmless, then
    // swap the bytes before the human taps.
    let sb = Sandbox::new("edit");
    sb.config("mode = confirm");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));

    let stage = sb.state().join("stage");
    let file = std::fs::read_dir(&stage)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().map(|e| e == "sh").unwrap_or(false))
        .expect("a staged file");
    std::fs::write(&file, format!("set +H\ntouch {}\n", sb.dir.join("evil").display())).unwrap();
    assert_eq!(file.metadata().unwrap().permissions().readonly(), false);

    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(1), "an edited block was approved");
    assert!(String::from_utf8_lossy(&out.stderr).contains("changed since the hold"));
    assert!(!sb.dir.join("evil").exists());
    assert!(!w.exists());
}

#[test]
fn a_hold_that_now_blocks_cannot_be_approved() {
    let sb = Sandbox::new("nowblock");
    sb.config("mode = confirm");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));
    // The gate learns a new danger rule between the hold and the tap.
    sb.set_gate("1", "rule=new-danger risk=danger");
    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(1), "a freshly-blocked block was approved");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no longer passes"), "{err}");
    assert!(!w.exists());
}

#[test]
fn a_gate_error_at_approval_time_refuses() {
    let sb = Sandbox::new("gateerr");
    sb.config("mode = confirm");
    let id = sb.hold(&touch(&sb.dir.join("w")));
    sb.set_gate("2", "");
    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(1), "a gate error was treated as a pass");
}

#[test]
fn an_expired_hold_cannot_be_approved() {
    let sb = Sandbox::new("expired");
    sb.config("mode = confirm\nhold_ttl_s = 1");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(1), "an expired hold was approved");
    assert!(String::from_utf8_lossy(&out.stderr).contains("expired"));
    assert!(!w.exists());
}

#[test]
fn switching_to_dry_refuses_a_pending_approval() {
    let sb = Sandbox::new("modedry");
    sb.config("mode = confirm");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));
    // The human moves to dry: nothing may run, whatever an old hold says.
    let ini = std::fs::read_to_string(sb.dir.join("config/watch.ini")).unwrap();
    std::fs::write(
        sb.dir.join("config/watch.ini"),
        ini.replace("mode = confirm", "mode = dry"),
    )
    .unwrap();
    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(1), "dry mode approved a hold");
    assert!(!w.exists());
}

#[test]
fn a_dry_mode_hold_is_recorded_but_not_approvable() {
    let sb = Sandbox::new("dryhold");
    sb.config("mode = dry");
    let w = sb.dir.join("w");
    let id = sb.hold(&touch(&w));
    let status = sb.plain(&["status"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains(&id));
    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!w.exists(), "dry mode ran an approved hold");
}

#[test]
fn an_unknown_id_is_refused() {
    let sb = Sandbox::new("noid");
    sb.config("mode = confirm");
    for cmd in ["approve", "deny"] {
        let out = sb.plain(&[cmd, "deadbeef"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("no hold"));
    }
}

#[test]
fn a_path_as_an_id_is_refused() {
    let sb = Sandbox::new("pathid");
    sb.config("mode = confirm");
    let out = sb.plain(&["approve", "../../etc/passwd"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no hold"));
}

#[test]
fn auto_stays_locked_until_enough_approvals_ran() {
    let sb = Sandbox::new("autolock");
    sb.config("mode = auto\nauto_unlock_approvals = 2");
    // Locked: the loop refuses to start.
    let out = sb.plain(&["run", "--once"]);
    assert_eq!(out.status.code(), Some(2), "auto ran while locked");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("locked"), "{err}");

    // Do the approvals the lock asks for.
    sb.config("mode = confirm\nauto_unlock_approvals = 2");
    for i in 0..2 {
        let w = sb.dir.join(format!("w{i}"));
        let id = sb.hold(&touch(&w));
        let out = sb.plain(&["approve", &id]);
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    }

    // Now auto is unlocked and a pass runs.
    sb.config("mode = auto\nauto_unlock_approvals = 2");
    let w = sb.dir.join("last");
    let out = sb.run(&["once"], &touch(&w));
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(w.exists(), "auto did not run once unlocked");
}

#[test]
fn a_failed_approval_run_keeps_the_hold_and_is_not_counted() {
    let sb = Sandbox::new("failrun");
    sb.config("mode = confirm");
    let id = sb.hold("set +H\nexit 7\n");
    let out = sb.plain(&["approve", &id]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        sb.state().join("holds").join(format!("{id}.hold")).exists(),
        "a failed run deleted the hold"
    );
    let status = sb.plain(&["status"]);
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("0/20"), "a failed approval was counted:\n{stdout}");
}

#[test]
fn approval_output_is_printed_and_logged() {
    let sb = Sandbox::new("output");
    sb.config("mode = confirm");
    let id = sb.hold("set +H\necho from-the-block\n");
    let out = sb.plain(&["approve", &id]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("from-the-block"));
    assert!(sb.log().contains("approved and exited 0"), "{}", sb.log());
}

#[test]
fn status_reports_the_lock_state_rather_than_implying_it() {
    let sb = Sandbox::new("statuslock");
    sb.config("mode = auto\nauto_unlock_approvals = 5");
    let out = sb.plain(&["status"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("locked"), "{stdout}");
    assert!(stdout.contains("5 more approval"), "{stdout}");
}
