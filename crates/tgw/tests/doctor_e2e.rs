//! `tgw doctor`, the single-instance lock, and clean shutdown.
//!
//! `doctor` exists to catch the failures that are silent in daily use, so each
//! test sets up one of those failures and requires it to be reported. A doctor
//! that only ever says "ok" is worse than none, so every branch here has a
//! failing case as well as a passing one.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn tgw() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgw"))
}

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("tgw-doc-{tag}-{}", std::process::id()));
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

    /// A gate that behaves like a real one: a clean sample passes, `rm -rf`
    /// blocks, and `--list-rules` answers.
    fn honest_gate(&self) -> PathBuf {
        self.script(
            "gate",
            "#!/bin/sh\n\
             if [ \"$1\" = \"--list-rules\" ]; then printf 'rm\\nmv\\n'; exit 0; fi\n\
             if grep -q 'rm -rf' \"$2\"; then echo 'rule=rm risk=danger' >&2; exit 1; fi\n\
             exit 0\n",
        )
    }

    fn config(&self, gate: &Path, extra: &str) {
        let clip = self.script("clip", "#!/bin/sh\nprintf 'set +H\\necho hi\\n'\n");
        let ini = format!(
            "clip_get_cmd = {clip}\ngate_cmd = {gate} --check {{file}}\n{extra}\n",
            clip = clip.display(),
            gate = gate.display()
        );
        std::fs::write(self.dir.join("config").join("watch.ini"), ini).unwrap();
    }

    fn doctor(&self) -> Output {
        Command::new(tgw())
            .args(["doctor"])
            .env("TCX_HOME", &self.dir)
            .output()
            .expect("run tgw doctor")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn an_honest_setup_passes_every_check() {
    let sb = Sandbox::new("good");
    let gate = sb.honest_gate();
    sb.config(&gate, "notify_cmd = true\nmode = confirm");
    let out = sb.doctor();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(0), "{t}");
    assert!(t.contains("all checks passed"), "{t}");
    assert!(t.contains("gate blocks a known-bad sample"), "{t}");
}

#[test]
fn a_gate_that_passes_everything_is_caught() {
    // The most dangerous misconfiguration: a gate that cannot see the block
    // approves everything, and every other check would look fine.
    let sb = Sandbox::new("permissive");
    let gate = sb.script(
        "gate",
        "#!/bin/sh\nif [ \"$1\" = \"--list-rules\" ]; then printf 'rm\\n'; exit 0; fi\nexit 0\n",
    );
    sb.config(&gate, "");
    let out = sb.doctor();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(t.contains("did not block"), "{t}");
}

#[test]
fn a_gate_that_blocks_clean_code_is_caught() {
    let sb = Sandbox::new("strict");
    let gate = sb.script(
        "gate",
        "#!/bin/sh\nif [ \"$1\" = \"--list-rules\" ]; then printf 'rm\\n'; exit 0; fi\nexit 1\n",
    );
    sb.config(&gate, "");
    let out = sb.doctor();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(t.contains("did not pass a clean sample"), "{t}");
}

#[test]
fn a_missing_clipboard_command_is_caught() {
    let sb = Sandbox::new("noclip");
    let gate = sb.honest_gate();
    sb.config(&gate, "");
    let ini = std::fs::read_to_string(sb.dir.join("config/watch.ini")).unwrap();
    let ini = ini
        .lines()
        .filter(|l| !l.starts_with("clip_get_cmd"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\nclip_get_cmd = /nonexistent/tgw-no-clip\n";
    std::fs::write(sb.dir.join("config/watch.ini"), ini).unwrap();
    let out = sb.doctor();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(t.contains("clipboard"), "{t}");
}

#[test]
fn a_broken_config_stops_before_the_other_checks() {
    let sb = Sandbox::new("badcfg");
    let gate = sb.honest_gate();
    // No {file}: the gate cannot see the block.
    let clip = sb.script("clip", "#!/bin/sh\ntrue\n");
    std::fs::write(
        sb.dir.join("config/watch.ini"),
        format!(
            "clip_get_cmd = {}\ngate_cmd = {} --check\n",
            clip.display(),
            gate.display()
        ),
    )
    .unwrap();
    let out = sb.doctor();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(t.contains("{file}"), "{t}");
    assert!(
        !t.contains("gate blocks"),
        "doctor went on to test a gate its config cannot run:\n{t}"
    );
}

#[test]
fn a_locked_auto_mode_is_a_warning_not_a_failure() {
    let sb = Sandbox::new("autowarn");
    let gate = sb.honest_gate();
    sb.config(&gate, "mode = auto\nauto_unlock_approvals = 5");
    let out = sb.doctor();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(0), "a lock is a warning, not a failure:\n{t}");
    assert!(t.contains("locked"), "{t}");
    assert!(t.contains("5 more approval"), "{t}");
}

#[test]
fn a_second_watch_loop_is_refused_while_the_first_runs() {
    let sb = Sandbox::new("lock");
    let gate = sb.honest_gate();
    // A clipboard that always answers nothing, so the loop just idles.
    let clip = sb.script("clip", "#!/bin/sh\nexit 0\n");
    sb.config(&gate, "");
    let ini = std::fs::read_to_string(sb.dir.join("config/watch.ini")).unwrap();
    let ini = ini
        .lines()
        .filter(|l| !l.starts_with("clip_get_cmd"))
        .collect::<Vec<_>>()
        .join("\n")
        + &format!("\nclip_get_cmd = {}\n", clip.display());
    std::fs::write(sb.dir.join("config/watch.ini"), ini).unwrap();

    let mut first = Command::new(tgw())
        .arg("run")
        .env("TCX_HOME", &sb.dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the first loop");

    // Wait for the lock to appear, so the second start races nothing.
    let lock = sb.dir.join("data/watch/tgw.lock");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !lock.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(lock.exists(), "the first loop never took the lock");

    let second = Command::new(tgw())
        .arg("run")
        .env("TCX_HOME", &sb.dir)
        .output()
        .expect("start the second loop");
    let t = text(&second);
    assert_eq!(second.status.code(), Some(1), "a second instance started:\n{t}");
    assert!(t.contains("already running"), "{t}");

    // SIGTERM must stop the first cleanly and release the lock.
    let _ = Command::new("kill")
        .args(["-TERM", &first.id().to_string()])
        .status();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if first.try_wait().ok().flatten().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let status = first.try_wait().ok().flatten();
    assert!(status.is_some(), "SIGTERM did not stop the loop");
    assert_eq!(status.unwrap().code(), Some(0), "the loop did not exit cleanly");
    assert!(!lock.exists(), "the lock outlived the loop");
}
