//! End-to-end: the binary against fake clipboard and gate scripts.
//!
//! No real clipboard, no real guard. Each test builds a `TCX_HOME` with a
//! `watch.ini` pointing at scripts it writes, then runs `tgw once` and reads
//! the log. That keeps the tests about `tgw`'s own decisions.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tgw() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgw"))
}

/// A sandbox: config, scripts, and a clipboard file to point at.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("tgw-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        Sandbox { dir }
    }

    /// Writes `body` as an executable script in `bin/`.
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

    fn clipboard(&self, name: &str, text: &str) -> PathBuf {
        let p = self.dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn config(&self, ini: &str) {
        std::fs::write(self.dir.join("config").join("watch.ini"), ini).unwrap();
    }

    fn run(&self, args: &[&str], clip: &Path) -> Output {
        Command::new(tgw())
            .args(args)
            .env("TCX_HOME", &self.dir)
            .env("TGW_CLIP", clip)
            .output()
            .expect("run tgw")
    }

    fn log(&self) -> String {
        let p = self.dir.join("data").join("watch").join("log").join("events.log");
        std::fs::read_to_string(p).unwrap_or_default()
    }

    fn state(&self) -> PathBuf {
        self.dir.join("data").join("watch")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A clipboard command that prints the file named by `$TGW_CLIP`.
const CLIP_SCRIPT: &str = "#!/bin/sh\ncat \"$TGW_CLIP\"\n";

/// A gate that always answers with the code in `$TGW_GATE_EXIT` and the
/// stderr in `$TGW_GATE_ERR`.
const GATE_SCRIPT: &str = "#!/bin/sh\n\
printf '%s\\n' \"$TGW_GATE_ERR\" >&2\n\
exit \"${TGW_GATE_EXIT:-0}\"\n";

fn setup(tag: &str) -> (Sandbox, PathBuf, String) {
    let sb = Sandbox::new(tag);
    let clip = sb.script("clip", CLIP_SCRIPT);
    let gate = sb.script("gate", GATE_SCRIPT);
    let ini = format!(
        "clip_get_cmd = {clip}\ngate_cmd = {gate} {{file}}\nmode = dry\n",
        clip = clip.display(),
        gate = gate.display()
    );
    sb.config(&ini);
    let block = sb.clipboard("clip.txt", "set +H\necho hi\n");
    (sb, block, ini)
}

fn run_with(sb: &Sandbox, clip: &Path, gate_exit: &str, gate_err: &str) -> Output {
    Command::new(tgw())
        .args(["once"])
        .env("TCX_HOME", &sb.dir)
        .env("TGW_CLIP", clip)
        .env("TGW_GATE_EXIT", gate_exit)
        .env("TGW_GATE_ERR", gate_err)
        .output()
        .expect("run tgw once")
}

#[test]
fn a_trigger_block_is_staged_and_gated() {
    let (sb, block, _) = setup("staged");
    let out = run_with(&sb, &block, "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hold"), "expected a hold in dry mode:\n{stdout}");
    assert!(stdout.contains("staged:"), "{stdout}");
    let log = sb.log();
    assert!(log.contains("hold"), "{log}");
    // The block was copied into the keep directory.
    let kept = sb.state().join("log").join("blocks");
    assert!(std::fs::read_dir(&kept).map(|d| d.count()).unwrap_or(0) >= 1);
}

#[test]
fn text_without_the_trigger_is_ignored_and_never_gated() {
    let (sb, _, _) = setup("notrigger");
    let clip = sb.clipboard("clip.txt", "hello world\n");
    // A gate that would fail loudly if it were called.
    let out = run_with(&sb, &clip, "99", "should not run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ignore"), "{stdout}");
    assert!(sb.log().contains("ignore"), "{}", sb.log());
}

#[test]
fn a_danger_verdict_blocks_and_is_never_run() {
    let (sb, block, _) = setup("danger");
    let out = run_with(&sb, &block, "1", "rule=rm -rf risk=danger");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("block"), "{stdout}");
    assert!(stdout.contains("rm -rf"), "the reason was lost:\n{stdout}");
    assert!(sb.log().contains("block"), "{}", sb.log());
}

#[test]
fn a_review_verdict_holds() {
    let (sb, block, _) = setup("review");
    let out = run_with(&sb, &block, "3", "rule=file deletion risk=review");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hold"), "{stdout}");
    assert!(stdout.contains("file deletion"), "{stdout}");
}

#[test]
fn a_gate_error_blocks_rather_than_erroring_out() {
    // Exit 2 is the gate saying "I could not decide". That must stop the
    // block, not skip it.
    let (sb, block, _) = setup("gateerr");
    let out = run_with(&sb, &block, "2", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("block"), "{stdout}");
}

#[test]
fn an_unexpected_gate_exit_blocks() {
    let (sb, block, _) = setup("weird");
    let out = run_with(&sb, &block, "7", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("block"), "exit 7 was not fail-closed:\n{stdout}");
}

#[test]
fn the_same_block_is_not_processed_twice_after_a_restart() {
    let (sb, block, ini) = setup("restart");
    let first = run_with(&sb, &block, "0", "");
    assert!(String::from_utf8_lossy(&first.stdout).contains("hold"));

    // A second process: the in-memory ring is empty, so only the on-disk
    // marker can dedupe it.
    let second = run_with(&sb, &block, "0", "");
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("duplicate"),
        "the same block was processed again after a restart:\n{stdout}"
    );
    assert_eq!(std::fs::read_to_string(sb.dir.join("config/watch.ini")).unwrap(), ini);
}

#[test]
fn a_clipboard_read_failure_is_an_ignore_not_a_crash() {
    let (sb, _, _) = setup("readfail");
    // Point at a script that fails.
    let bad = sb.script("badclip", "#!/bin/sh\nexit 5\n");
    sb.config(&format!(
        "clip_get_cmd = {}\ngate_cmd = {} {{file}}\n",
        bad.display(),
        bad.display()
    ));
    let out = sb.run(&["once"], &sb.dir.join("clip.txt"));
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ignore"));
}

#[test]
fn an_oversize_clipboard_is_ignored_before_staging() {
    let (sb, _, _) = setup("oversize");
    let big = sb.clipboard("big.txt", &format!("set +H\n{}", "x".repeat(200_000)));
    let out = run_with(&sb, &big, "0", "");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ignore"), "{stdout}");
    assert!(stdout.contains("exceeds cap"), "{stdout}");
}

#[test]
fn a_clipboard_with_a_nul_byte_is_ignored() {
    let (sb, _, _) = setup("nul");
    let p = sb.dir.join("nul.txt");
    std::fs::write(&p, b"set +H\necho\0hi\n").unwrap();
    let out = run_with(&sb, &p, "0", "");
    assert!(String::from_utf8_lossy(&out.stdout).contains("NUL"));
}

#[test]
fn a_block_containing_a_watch_match_months_ago_still_reports_it() {
    let (sb, _, _) = setup("watchname");
    let clip = sb.clipboard("clip.txt", "set +H\nsed -i 's/a/b/' f\n");
    let out = run_with(&sb, &clip, "0", "watch=sed-i\n");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("watch:sed-i"), "{stdout}");
}

#[test]
fn staging_creates_a_private_directory() {
    let (sb, block, _) = setup("private");
    run_with(&sb, &block, "0", "");
    let stage = sb.state().join("stage");
    let meta = std::fs::metadata(&stage).expect("stage dir exists");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    }
}
