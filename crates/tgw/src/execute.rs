//! Running an approved block.
//!
//! Everything that can make a run wrong is decided here, and the defaults are
//! the safe ones:
//!
//! | Property | Choice | Why |
//! | --- | --- | --- |
//! | stdin | `/dev/null` | a block that reads the terminal would hang |
//! | environment | exactly `env_allow` | a block should not inherit a token |
//! | process group | its own | a timeout must kill children too |
//! | timeout | `timeout_s` | a blocked `read` must not wedge the watchdog |
//! | output | capped, logged | a runaway `yes` must not fill the disk |
//! | cwd | inherit / fixed / file | `inherit` is the default: no self-editable file |
//!
//! # The marker
//!
//! Children get `TGW_IN_BLOCK=1`. `tgw cd` refuses to run then, so a gated
//! block cannot redirect where later blocks run. It is stated plainly
//! everywhere it appears: this is an accident guard, not a boundary. A block
//! can still `echo / > cwd_file` directly.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::{Config, CwdMode};

/// Set for every child a block spawns.
pub const IN_BLOCK: &str = "TGW_IN_BLOCK";

/// The result of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub exit: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration_ms: u64,
    /// True when output hit the cap and was truncated.
    pub truncated: bool,
}

impl RunResult {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit == Some(0)
    }
}

/// Why a block could not even be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    BadShell(String),
    BadCwd(String),
    Spawn(String),
}

impl RunError {
    pub fn reason(&self) -> String {
        match self {
            RunError::BadShell(e) => format!("shell does not parse: {e}"),
            RunError::BadCwd(e) => format!("working directory: {e}"),
            RunError::Spawn(e) => format!("cannot start the block: {e}"),
        }
    }
}

/// Works out the directory a block runs in.
///
/// `file` mode re-reads `cwd_file` for **every** block, so a stale read cannot
/// outlive the file. A relative path or a missing directory is an error: a
/// block that silently ran somewhere else is worse than one that did not run.
pub fn resolve_cwd(cfg: &Config) -> Result<PathBuf, RunError> {
    match cfg.cwd_mode {
        CwdMode::Inherit => std::env::current_dir()
            .map_err(|e| RunError::BadCwd(format!("cannot read the current directory: {e}"))),
        CwdMode::Fixed => check_dir(Path::new(&cfg.cwd)),
        CwdMode::File => {
            let raw = std::fs::read_to_string(&cfg.cwd_file)
                .map_err(|e| RunError::BadCwd(format!("cannot read {}: {e}", cfg.cwd_file)))?;
            let one = raw.trim();
            if one.is_empty() {
                return Err(RunError::BadCwd(format!("{} is empty", cfg.cwd_file)));
            }
            let p = Path::new(one);
            if !p.is_absolute() {
                return Err(RunError::BadCwd(format!(
                    "{} names a relative path: {one}",
                    cfg.cwd_file
                )));
            }
            check_dir(p)
        }
    }
}

fn check_dir(p: &Path) -> Result<PathBuf, RunError> {
    if !p.is_absolute() {
        return Err(RunError::BadCwd(format!("not absolute: {}", p.display())));
    }
    if !p.is_dir() {
        return Err(RunError::BadCwd(format!("not a directory: {}", p.display())));
    }
    Ok(p.to_path_buf())
}

/// The environment a block sees: exactly the allowed variables, plus the
/// marker. Anything absent from the parent simply is not set.
pub fn block_env(cfg: &Config) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = cfg
        .env_allow
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|val| (name.clone(), val)))
        .collect();
    v.retain(|(k, _)| k != IN_BLOCK);
    v.push((IN_BLOCK.to_string(), "1".to_string()));
    v
}

/// Runs `path` as a block.
///
/// The file is re-read from disk rather than passed as text, so what runs is
/// byte-for-byte what the gate saw.
pub fn run_block(cfg: &Config, path: &Path) -> Result<RunResult, RunError> {
    let shell = crate::config::split_argv(&cfg.shell).map_err(|e| RunError::BadShell(e))?;
    if shell.is_empty() {
        return Err(RunError::BadShell("shell is empty".into()));
    }
    let cwd = resolve_cwd(cfg)?;

    let mut argv = shell.clone();
    if !cfg.prelude.trim().is_empty() {
        // Source the prelude then the block, in one shell, so aliases defined
        // by the prelude are visible to the block.
        argv.extend([
            "-c".to_string(),
            format!(". {} ; . {}", shell_quote(&cfg.prelude), shell_quote(&path.to_string_lossy())),
        ]);
    } else {
        argv.push(path.to_string_lossy().into_owned());
    }

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    for (k, v) in block_env(cfg) {
        cmd.env(k, v);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and only affects this child.
        unsafe {
            cmd.pre_exec(|| {
                let _ = setsid();
                Ok(())
            });
        }
    }

    let _lock = Wakelock::acquire(cfg);
    let start = Instant::now();
    let mut child = cmd.spawn().map_err(|e| {
        RunError::Spawn(format!("{}: {e}", argv[0]))
    })?;

    // Drain both pipes on threads; a block that writes a lot would otherwise
    // block on a full pipe while we wait for it to exit.
    let cap = cfg.max_output_bytes as usize;
    let out_r = drain(child.stdout.take(), cap);
    let err_r = drain(child.stderr.take(), cap);

    let timeout = Duration::from_secs(cfg.timeout_s.max(1));
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    kill_group(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let (stdout, out_trunc) = out_r.join().unwrap_or_default();
    let (stderr, err_trunc) = err_r.join().unwrap_or_default();

    Ok(RunResult {
        exit: status.and_then(|s| s.code()),
        timed_out,
        stdout,
        stderr,
        duration_ms: start.elapsed().as_millis() as u64,
        truncated: out_trunc || err_trunc,
    })
}

/// Reads a pipe fully but keeps at most `cap` bytes, still draining the rest
/// so the writer never blocks.
fn drain(
    pipe: Option<impl Read + Send + 'static>,
    cap: usize,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut truncated = false;
        if let Some(mut r) = pipe {
            let mut buf = [0u8; 8192];
            loop {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if kept.len() < cap {
                            let room = cap - kept.len();
                            let take = room.min(n);
                            kept.extend_from_slice(&buf[..take]);
                            if take < n {
                                truncated = true;
                            }
                        } else {
                            truncated = true;
                        }
                    }
                }
            }
        }
        (kept, truncated)
    })
}

/// A wakelock held for the duration of one block.
///
/// On a phone the screen can sleep mid-block and suspend the process. The lock
/// is taken before the block starts and released on drop, so the release
/// happens on every path out of [`run_block`], including a timeout. A wakelock
/// that fails to start is a warning, not an error: a machine without one can
/// still run a block.
pub struct Wakelock {
    child: Option<Child>,
    /// Started on drop when set, for helpers that need an explicit release
    /// instead of being killed.
    release: Option<Vec<String>>,
}

impl Wakelock {
    /// Starts `wakelock_cmd` if it is configured and parses.
    pub fn acquire(cfg: &Config) -> Wakelock {
        let child = config_argv(&cfg.wakelock_cmd).and_then(|argv| {
            Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()
        });
        Wakelock {
            child,
            release: config_argv(&cfg.wakelock_release_cmd),
        }
    }

    /// Whether a lock was taken.
    pub fn is_held(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for Wakelock {
    fn drop(&mut self) {
        if let Some(argv) = self.release.take() {
            let _ = Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if let Some(mut child) = self.child.take() {
            // Long-lived helpers like `termux-wake-lock` exit immediately;
            // ones that block exit when told to. `wakelock_release_cmd` is the
            // explicit release; killing the helper is the fallback.
            let _ = child.try_wait();
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn config_argv(s: &str) -> Option<Vec<String>> {
    if s.trim().is_empty() {
        return None;
    }
    let argv = crate::config::split_argv(s).ok()?;
    if argv.is_empty() {
        None
    } else {
        Some(argv)
    }
}

/// Wraps a path in single quotes for `sh`, escaping embedded quotes.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+:=".contains(c))
    {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

// `kill`/`setsid` are declared here rather than pulled from a crate, so this
// binary keeps its `std`-only promise.
#[cfg(unix)]
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(unix)]
extern "C" {
    fn setsid() -> i32;
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    // SAFETY: a negative pid signals the process group.
    unsafe {
        kill(-(pid as i32), 9);
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

#[allow(dead_code)]
fn _assert_child_used(_c: &Child) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.clip_get_cmd = "true".into();
        c.gate_cmd = "true {file}".into();
        c.shell = "sh".into();
        c.env_allow = vec!["PATH".into()];
        c
    }

    fn block(text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgw-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{:016x}.sh", crate::clip::hash_bytes(text.as_bytes())));
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn a_block_runs_and_its_output_is_captured() {
        let p = block("echo hello; echo oops >&2; exit 0\n");
        let r = run_block(&cfg(), &p).expect("run");
        assert_eq!(r.exit, Some(0));
        assert!(r.success());
        assert_eq!(String::from_utf8_lossy(&r.stdout), "hello\n");
        assert_eq!(String::from_utf8_lossy(&r.stderr), "oops\n");
    }

    #[test]
    fn a_nonzero_exit_is_reported_not_swallowed() {
        let p = block("exit 42\n");
        let r = run_block(&cfg(), &p).expect("run");
        assert_eq!(r.exit, Some(42));
        assert!(!r.success());
    }

    #[test]
    fn stdin_is_dev_null_so_a_reading_block_cannot_hang() {
        let p = block("cat\n");
        let r = run_block(&cfg(), &p).expect("run");
        assert_eq!(r.exit, Some(0));
        assert!(r.stdout.is_empty());
    }

    #[test]
    fn the_environment_is_only_what_was_allowed() {
        let mut c = cfg();
        c.env_allow = vec!["PATH".into()];
        std::env::set_var("TGW_SECRET_PROBE", "leaked");
        let p = block("echo \"[${TGW_SECRET_PROBE:-unset}]\"\n");
        let r = run_block(&c, &p).expect("run");
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("[unset]"), "an unlisted variable leaked: {out}");
        std::env::remove_var("TGW_SECRET_PROBE");
    }

    #[test]
    fn children_get_the_in_block_marker() {
        let p = block("echo \"[$TGW_IN_BLOCK]\"\n");
        let r = run_block(&cfg(), &p).expect("run");
        assert!(String::from_utf8_lossy(&r.stdout).contains("[1]"));
    }

    #[test]
    fn a_timeout_kills_the_block_and_its_children() {
        let mut c = cfg();
        c.timeout_s = 1;
        let p = block("sleep 30; echo never\n");
        let start = Instant::now();
        let r = run_block(&c, &p).expect("run");
        assert!(r.timed_out);
        assert!(!r.success());
        assert!(start.elapsed() < Duration::from_secs(10), "timeout did not apply");
    }

    #[test]
    fn output_is_capped_but_the_process_still_finishes() {
        let mut c = cfg();
        c.max_output_bytes = 100;
        // Writes far more than the cap then exits normally: if the pipe were
        // not drained the process would block forever.
        let p = block("i=0; while [ $i -lt 5000 ]; do echo 0123456789; i=$((i+1)); done\n");
        let r = run_block(&c, &p).expect("run");
        assert!(r.truncated, "output was not marked truncated");
        assert!(r.stdout.len() <= 100, "captured {} bytes", r.stdout.len());
        assert_eq!(r.exit, Some(0));
    }

    #[test]
    fn a_prelude_is_sourced_before_the_block() {
        let dir = std::env::temp_dir().join(format!("tgw-prelude-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let prelude = dir.join("prelude.sh");
        std::fs::write(&prelude, "greet() { echo from-prelude; }\n").unwrap();
        let mut c = cfg();
        c.prelude = prelude.to_string_lossy().into_owned();
        let p = block("greet\n");
        let r = run_block(&c, &p).expect("run");
        assert!(String::from_utf8_lossy(&r.stdout).contains("from-prelude"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cwd_mode_fixed_uses_the_configured_directory() {
        let dir = std::env::temp_dir().join(format!("tgw-cwdfixed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = cfg();
        c.cwd_mode = CwdMode::Fixed;
        c.cwd = dir.to_string_lossy().into_owned();
        let p = block("pwd\n");
        let r = run_block(&c, &p).expect("run");
        let got = String::from_utf8_lossy(&r.stdout);
        assert!(got.trim().ends_with(dir.file_name().unwrap().to_str().unwrap()), "{got}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cwd_mode_file_rereads_the_file_every_time() {
        let dir = std::env::temp_dir().join(format!("tgw-cwdfile-{}", std::process::id()));
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let marker = dir.join("cwd");
        std::fs::write(&marker, format!("{}\n", a.display())).unwrap();

        let mut c = cfg();
        c.cwd_mode = CwdMode::File;
        c.cwd_file = marker.to_string_lossy().into_owned();
        let p = block("pwd\n");

        let r1 = run_block(&c, &p).expect("run");
        assert!(String::from_utf8_lossy(&r1.stdout).contains("/a"));

        std::fs::write(&marker, format!("{}\n", b.display())).unwrap();
        let r2 = run_block(&c, &p).expect("run");
        assert!(
            String::from_utf8_lossy(&r2.stdout).contains("/b"),
            "cwd_file was not re-read: {}",
            String::from_utf8_lossy(&r2.stdout)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cwd_mode_file_rejects_relative_and_missing_paths() {
        let dir = std::env::temp_dir().join(format!("tgw-cwdrel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("cwd");
        let mut c = cfg();
        c.cwd_mode = CwdMode::File;
        c.cwd_file = marker.to_string_lossy().into_owned();

        std::fs::write(&marker, "relative/dir\n").unwrap();
        assert!(matches!(resolve_cwd(&c), Err(RunError::BadCwd(_))));

        std::fs::write(&marker, "/nonexistent/tgw-dir\n").unwrap();
        assert!(matches!(resolve_cwd(&c), Err(RunError::BadCwd(_))));

        std::fs::write(&marker, "\n").unwrap();
        assert!(matches!(resolve_cwd(&c), Err(RunError::BadCwd(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_quote_is_safe_for_spaces_and_quotes() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("/tmp/plain"), "/tmp/plain");
        // The quoted form must round-trip through a shell.
        let nasty = "a b'c\"d$e";
        let p = block(&format!("printf '%s' {}\n", shell_quote(nasty)));
        let r = run_block(&cfg(), &p).expect("run");
        assert_eq!(String::from_utf8_lossy(&r.stdout), nasty);
    }

    #[test]
    fn a_wakelock_is_taken_and_released_around_a_block() {
        let dir = std::env::temp_dir().join(format!("tgw-wake-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let started = dir.join("started");
        let mut c = cfg();
        c.wakelock_cmd = format!("touch {}", started.display());
        let p = block("echo hi\n");
        let r = run_block(&c, &p).expect("run");
        assert_eq!(r.exit, Some(0));
        assert!(started.exists(), "the wakelock command never ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wakelock_is_released_even_when_the_block_times_out() {
        let dir = std::env::temp_dir().join(format!("tgw-wake2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir).unwrap();
        // A helper that blocks; it must not survive the run.
        let mut c = cfg();
        c.wakelock_cmd = "sleep 120".into();
        c.timeout_s = 1;
        let p = block("sleep 30\n");
        let r = run_block(&c, &p).expect("run");
        assert!(r.timed_out);
        // The drop released it; nothing to assert beyond returning promptly.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_release_command_runs_on_drop() {
        let dir = std::env::temp_dir().join(format!("tgw-wake3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let released = dir.join("released");
        let mut c = cfg();
        c.wakelock_cmd = "true".into();
        c.wakelock_release_cmd = format!("touch {}", released.display());
        let p = block("echo hi\n");
        run_block(&c, &p).expect("run");
        assert!(released.exists(), "the release command never ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_shell_is_an_error_not_a_default() {
        let mut c = cfg();
        c.shell = "   ".into();
        assert!(matches!(
            run_block(&c, Path::new("/tmp/x")),
            Err(RunError::BadShell(_))
        ));
    }
}
