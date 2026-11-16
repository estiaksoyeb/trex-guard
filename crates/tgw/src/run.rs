//! Running the watchdog loop: one instance, clean shutdown, a state dir that
//! is safe to write.
//!
//! # One instance
//!
//! Two watchdogs polling one clipboard would each see every block, and the
//! one-shot `.seen` markers only dedupe *content* — so two instances would
//! race to stage the same block and one would log "cannot stage". Worse, in
//! `auto` they would both try to run it. A lock file makes the second instance
//! refuse at startup instead.
//!
//! The lock is a `create_new` file holding the pid. Stale locks (the process
//! is gone) are cleared, so a crash does not require manual cleanup. The pid
//! is checked with `kill -0` semantics through `/proc`, which is the cheapest
//! check that works without a dependency.
//!
//! # Signals
//!
//! SIGINT and SIGTERM remove the lock and stage nothing new. They are
//! installed with `signal()` rather than `sigaction` because the default
//! disposition is the only thing being changed, and the handler only sets a
//! flag. A handler that touches the filesystem is a handler that can deadlock.
//!
//! Shutdown is deliberately **not** mid-block: the loop polls between
//! iterations, so a block already running keeps its own timeout. Killing a
//! running block on Ctrl-C would leave whatever it was doing half-done, which
//! is worse than waiting for `timeout_s`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{Config, Mode};
use crate::holds;

/// Set by the signal handler.
static STOP: AtomicBool = AtomicBool::new(false);

/// Whether a shutdown was requested.
pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

/// Clears the stop flag, so a test can restart the loop.
pub fn reset_stop() {
    STOP.store(false, Ordering::SeqCst);
}

extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
}

const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

extern "C" fn on_signal(_sig: i32) {
    // Only an atomic store. Anything else risks a deadlock in the handler.
    STOP.store(true, Ordering::SeqCst);
}

/// Installs the SIGINT/SIGTERM handler that sets [`STOP`].
pub fn install_signal_handler() {
    let handler = on_signal as extern "C" fn(i32) as usize;
    // SAFETY: `signal` with a plain function pointer as the handler.
    unsafe {
        signal(SIGINT, handler);
        signal(SIGTERM, handler);
    }
}

/// A held single-instance lock, released on drop.
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
}

impl InstanceLock {
    /// The lock file path inside a state dir.
    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join("tgw.lock")
    }

    /// Takes the lock, or reports which process holds it.
    pub fn acquire(state_dir: &Path) -> Result<InstanceLock, String> {
        let path = Self::path(state_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(f, "{}", std::process::id());
                Ok(InstanceLock { path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let other = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                match other {
                    Some(pid) if pid_alive(pid) => Err(format!(
                        "another tgw is already running (pid {pid}); one is enough"
                    )),
                    // A crash left the file. Clear it rather than refuse to
                    // start: requiring a human to delete a file after every
                    // crash is how a watchdog gets turned off for good.
                    Some(pid) => {
                        let _ = std::fs::remove_file(&path);
                        let _ = pid;
                        Self::acquire(state_dir)
                    }
                    None => {
                        let _ = std::fs::remove_file(&path);
                        Self::acquire(state_dir)
                    }
                }
            }
            Err(e) => Err(format!("cannot create {}: {e}", path.display())),
        }
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether `pid` is a live process.
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Makes sure the state directory exists and is 0700, and that the log
/// directory is writable. Returns every problem.
pub fn check_state_dir(cfg: &Config) -> Vec<String> {
    let mut problems = Vec::new();
    let state = cfg.state_dir();
    // Measure before fixing: `ensure_state_dir` sets 0700, and reporting a
    // problem we have already corrected would hide that the directory was
    // loose at some point, which is worth knowing once.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A state dir another user can write is a state dir that can be
        // pre-populated with a staged block and a hold record.
        if let Ok(meta) = std::fs::metadata(&state) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                problems.push(format!(
                    "state dir {} was {mode:o}; it must not be readable by others (700) — corrected",
                    state.display()
                ));
            }
        }
    }
    if let Err(e) = crate::clip::ensure_state_dir(&state) {
        problems.push(format!("state dir {}: {e}", state.display()));
    }
    let log_dir = cfg.log_dir();
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        problems.push(format!("log dir {}: {e}", log_dir.display()));
    } else {
        let probe = log_dir.join(".tgw-write-probe");
        match std::fs::write(&probe, b"x") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
            }
            Err(e) => problems.push(format!("log dir {} is not writable: {e}", log_dir.display())),
        }
    }
    // The `auto` lock is deliberately *not* a problem here. `doctor` and
    // `status` report it as a warning, and `run` refuses separately; listing
    // it as a state-dir problem made `doctor` print it twice and call a
    // warning a failure.
    problems
}

/// Drops expired holds and their staged files, returning how many went.
///
/// Called once at startup and then rarely: expired holds are harmless, and
/// deleting them while the human is looking at `status` would be confusing.
pub fn sweep_expired(cfg: &Config) -> usize {
    let state = cfg.state_dir();
    let now = crate::log::now_unix();
    let Ok(entries) = std::fs::read_dir(holds::holds_dir(&state)) else {
        return 0;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let Some(stem) = e.path().file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if let Ok(h) = holds::load(&state, &stem) {
            if h.expires <= now {
                holds::remove(&state, &h);
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tgw-run-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_second_instance_is_refused_and_the_first_keeps_its_lock() {
        let d = tmp("lock");
        let first = InstanceLock::acquire(&d).expect("first");
        let err = InstanceLock::acquire(&d).expect_err("a second lock was granted");
        assert!(err.contains("already running"), "{err}");
        drop(first);
        // Released on drop, so a new one is granted.
        let _second = InstanceLock::acquire(&d).expect("after release");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stale_lock_from_a_dead_pid_is_cleared() {
        let d = tmp("stale");
        // 2^22 is above the default pid_max on most kernels and certainly not
        // this test's pid, so /proc does not have it.
        std::fs::write(InstanceLock::path(&d), "4194304\n").unwrap();
        let lock = InstanceLock::acquire(&d).expect("a stale lock was not cleared");
        drop(lock);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_garbage_lock_file_is_cleared_rather_than_wedging() {
        let d = tmp("garbage");
        std::fs::write(InstanceLock::path(&d), "not a pid\n").unwrap();
        let lock = InstanceLock::acquire(&d).expect("garbage lock wedged startup");
        drop(lock);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_world_readable_state_dir_is_a_problem() {
        let d = tmp("perms");
        let mut c = Config::default();
        c.clip_get_cmd = "true".into();
        c.gate_cmd = "true {file}".into();
        c.state_dir = d.join("state").to_string_lossy().into_owned();
        c.log_dir = d.join("log").to_string_lossy().into_owned();
        assert_eq!(check_state_dir(&c), Vec::<String>::new(), "a fresh state dir reported problems");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let state = Path::new(&c.state_dir);
            std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o755)).unwrap();
            let problems = check_state_dir(&c);
            assert!(
                problems.iter().any(|p| p.contains("700")),
                "a 0755 state dir was accepted: {problems:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_auto_lock_is_not_a_state_dir_problem() {
        // It is a warning with its own line in `doctor` and `status`, and a
        // separate refusal in `run`. Reporting it here too would call a
        // warning a failure and print it twice.
        let d = tmp("autolock");
        let mut c = Config::default();
        c.clip_get_cmd = "true".into();
        c.gate_cmd = "true {file}".into();
        c.state_dir = d.join("state").to_string_lossy().into_owned();
        c.log_dir = d.join("log").to_string_lossy().into_owned();
        c.mode = Mode::Auto;
        c.auto_unlock_approvals = 3;
        assert_eq!(check_state_dir(&c), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_stop_flag_starts_clear_and_can_be_set() {
        reset_stop();
        assert!(!stop_requested());
        STOP.store(true, Ordering::SeqCst);
        assert!(stop_requested());
        reset_stop();
        assert!(!stop_requested());
    }
}
