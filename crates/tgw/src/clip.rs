//! Reading the clipboard, validating what came back, and staging it.
//!
//! # Why a staged file
//!
//! The gate and the shell must see **the same bytes**. If each read the
//! clipboard, a process that changed it between the two reads would be
//! gated on one block and run another — a time-of-check/time-of-use hole. So
//! the clipboard is read once, written to a private file, and both stages get
//! that file.
//!
//! # Rejects, before staging
//!
//! | Reject | Why |
//! | --- | --- |
//! | not UTF-8 | a block is text; bytes the human did not type are not a block |
//! | contains NUL | NUL truncates C strings and cannot round-trip a shell |
//! | over `max_block_bytes` | an accident (a log copied instead of a block) |
//! | empty | nothing to run |
//!
//! Each is a recorded `ignore`, not an error: the clipboard changes constantly
//! and most of it is not a block.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::Config;

/// Why a clipboard read did not produce a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    /// The clipboard command failed or timed out.
    ReadFailed(String),
    NotUtf8,
    Nul,
    TooLarge { bytes: usize, cap: u64 },
    Empty,
    /// First non-empty line is not the trigger.
    NoTrigger { saw: String },
    /// Seen before: the same content is already in the ring.
    Duplicate,
}

impl Reject {
    /// One line for the log.
    pub fn reason(&self) -> String {
        match self {
            Reject::ReadFailed(e) => format!("clipboard read failed: {e}"),
            Reject::NotUtf8 => "not valid UTF-8".into(),
            Reject::Nul => "contains a NUL byte".into(),
            Reject::TooLarge { bytes, cap } => format!("{bytes} bytes exceeds cap {cap}"),
            Reject::Empty => "empty".into(),
            Reject::NoTrigger { saw } => format!("first line is not the trigger: {saw:?}"),
            Reject::Duplicate => "duplicate of a recent block".into(),
        }
    }
}

/// Raw clipboard bytes, with how long the read took.
#[derive(Debug, Clone)]
pub struct Clipboard {
    pub bytes: Vec<u8>,
    pub elapsed: Duration,
}

/// Runs `clip_get_cmd` and returns its stdout.
///
/// Never panics and never returns `Err` for a timeout: a clipboard read is
/// expected to fail sometimes (a bridge restarts, the phone sleeps), so the
/// error is data the caller logs and backs off on.
pub fn read_clipboard(cfg: &Config, timeout: Duration) -> Result<Clipboard, Reject> {
    let argv = crate::config::split_argv(&cfg.clip_get_cmd)
        .map_err(|e| Reject::ReadFailed(format!("clip_get_cmd: {e}")))?;
    if argv.is_empty() {
        return Err(Reject::ReadFailed("clip_get_cmd is empty".into()));
    }

    let start = Instant::now();
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Reject::ReadFailed(format!("{}: {e}", argv[0])))?;

    // Read on a thread so the deadline is enforced even when the tool hangs
    // without writing. `std` cannot set a read timeout on a pipe, so the
    // timeout has to come from outside the blocking read.
    let stdout = child.stdout.take().expect("stdout was piped");
    let reader = std::thread::spawn(move || {
        let mut r = stdout;
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    });

    let deadline = start + timeout;
    loop {
        if reader.is_finished() {
            break;
        }
        if Instant::now() >= deadline {
            kill(&mut child);
            return Err(Reject::ReadFailed(format!(
                "timed out after {} ms",
                timeout.as_millis()
            )));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let out = reader.join().unwrap_or_default();

    let status = child.wait().map_err(|e| Reject::ReadFailed(format!("wait: {e}")))?;
    if !status.success() {
        return Err(Reject::ReadFailed(format!(
            "{} exited {}",
            argv[0],
            status.code().map(|c| c.to_string()).unwrap_or_else(|| "on a signal".into())
        )));
    }
    Ok(Clipboard {
        bytes: out,
        elapsed: start.elapsed(),
    })
}

fn kill(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Checks a clipboard payload and returns the text if it is a block.
///
/// Size is checked before UTF-8 so that a huge binary blob is reported as
/// "too large" rather than spending effort decoding it.
pub fn validate(bytes: &[u8], cfg: &Config) -> Result<String, Reject> {
    if bytes.len() as u64 > cfg.max_block_bytes {
        return Err(Reject::TooLarge {
            bytes: bytes.len(),
            cap: cfg.max_block_bytes,
        });
    }
    if bytes.contains(&0) {
        return Err(Reject::Nul);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Reject::NotUtf8)?;
    if text.trim().is_empty() {
        return Err(Reject::Empty);
    }
    Ok(text.to_string())
}

/// Whether `text` starts with the configured trigger line.
///
/// Matching is on the **first non-empty line**, compared to the trigger with
/// trailing whitespace ignored, so `set +H` and `set +H   ` are the same block
/// but `echo set +H` is not. This is an accident guard, not a security
/// control: the gate is the security control.
pub fn has_trigger(text: &str, trigger: &str) -> Result<(), Reject> {
    let first = text.lines().map(str::trim_end).find(|l| !l.trim().is_empty());
    match first {
        Some(line) if line.trim() == trigger.trim() => Ok(()),
        Some(line) => Err(Reject::NoTrigger {
            saw: line.to_string(),
        }),
        None => Err(Reject::Empty),
    }
}

/// A rolling set of recently seen content hashes.
///
/// The clipboard still holds the block after it runs, and the block's own
/// output replaces it. Both are re-read every poll, so the same content is
/// seen many times; without this the watchdog would re-run everything forever,
/// and would treat its own output as a new block.
#[derive(Debug, Default)]
pub struct Seen {
    ring: std::collections::VecDeque<u64>,
    set: std::collections::HashSet<u64>,
    cap: usize,
}

impl Seen {
    pub fn new(cap: usize) -> Seen {
        Seen {
            ring: std::collections::VecDeque::with_capacity(cap.max(1)),
            set: std::collections::HashSet::new(),
            cap: cap.max(1),
        }
    }

    /// Records `hash`; returns false if it was already present.
    pub fn insert(&mut self, hash: u64) -> bool {
        if !self.set.insert(hash) {
            return false;
        }
        self.ring.push_back(hash);
        while self.ring.len() > self.cap {
            if let Some(old) = self.ring.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }

    pub fn contains(&self, hash: u64) -> bool {
        self.set.contains(&hash)
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

/// A content hash. FNV-1a: not cryptographic, and deliberately so — this
/// detects "the same clipboard text again", not an adversary.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// A block written to disk, ready for the gate and the shell.
#[derive(Debug, Clone)]
pub struct Staged {
    /// Short hex id, used in notifications and logs.
    pub id: String,
    pub path: PathBuf,
    pub hash: u64,
    pub bytes: usize,
}

/// Writes `text` to a private file under `state_dir/stage/`.
///
/// The directory is 0700 and the file 0600: a block can contain a secret, and
/// another user on the machine has no business reading it. An existing file
/// with the same name is never overwritten — ids are random-ish and unique.
pub fn stage(state_dir: &Path, text: &str, hash: u64) -> std::io::Result<Staged> {
    let dir = state_dir.join("stage");
    std::fs::create_dir_all(&dir)?;
    restrict_dir(&dir)?;

    let id = format!("{:08x}{:04x}", hash as u32, (hash >> 32) as u16);
    let path = dir.join(format!("{id}.sh"));
    // `create_new` so a predictable id cannot be used to overwrite a file
    // another process staged.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    restrict_file(&path)?;
    {
        use std::io::Write;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    Ok(Staged {
        id,
        path,
        hash,
        bytes: text.len(),
    })
}

/// Records that `hash` has been staged, so a restart dedupes it.
///
/// A plain marker file, kept separate from the `.sh` because the block copy
/// may be pruned by log retention while the "already seen" fact must outlive
/// it. Write failure is not fatal: the worst case is one duplicate.
pub fn mark_seen(state_dir: &Path, hash: u64) -> std::io::Result<()> {
    let dir = state_dir.join("stage");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{hash:016x}.seen"));
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(drop)
}

/// Whether `hash` was staged before.
pub fn was_seen(state_dir: &Path, hash: u64) -> bool {
    state_dir
        .join("stage")
        .join(format!("{hash:016x}.seen"))
        .exists()
}

/// Creates the state directory tree with 0700.
pub fn ensure_state_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    restrict_dir(dir)
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.clip_get_cmd = "true".into();
        c.gate_cmd = "true {file}".into();
        c
    }

    #[test]
    fn validate_rejects_each_bad_shape() {
        let c = cfg();
        assert_eq!(validate(b"\xff\xfe", &c), Err(Reject::NotUtf8));
        assert_eq!(validate(b"rm\0x", &c), Err(Reject::Nul));
        assert_eq!(validate(b"   \n\t\n", &c), Err(Reject::Empty));
        assert_eq!(validate(b"", &c), Err(Reject::Empty));
        let mut big = c.clone();
        big.max_block_bytes = 4;
        assert!(matches!(
            validate(b"12345", &big),
            Err(Reject::TooLarge { .. })
        ));
        assert_eq!(validate(b"text", &c).unwrap(), "text");
    }

    #[test]
    fn size_is_checked_before_utf8() {
        // A huge binary blob should be reported as too large, not spend effort
        // decoding it.
        let mut c = cfg();
        c.max_block_bytes = 4;
        match validate(&[0xff; 100], &c) {
            Err(Reject::TooLarge { .. }) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn trigger_is_the_first_non_empty_line() {
        assert!(has_trigger("set +H\nrm -rf x", "set +H").is_ok());
        assert!(has_trigger("\n\n  set +H\nls", "set +H").is_ok());
        assert!(has_trigger("set +H   \nls", "set +H").is_ok());
        assert!(has_trigger("echo set +H", "set +H").is_err());
        assert!(has_trigger("set +HH", "set +H").is_err());
    }

    #[test]
    fn trigger_absence_is_reported_with_what_was_seen() {
        match has_trigger("hello\nset +H", "set +H") {
            Err(Reject::NoTrigger { saw }) => assert_eq!(saw, "hello"),
            other => panic!("expected NoTrigger, got {other:?}"),
        }
    }

    #[test]
    fn a_seen_ring_forgets_the_oldest() {
        let mut s = Seen::new(2);
        assert!(s.insert(1));
        assert!(s.insert(2));
        assert!(!s.insert(1), "a repeat must be refused");
        assert!(s.insert(3));
        assert_eq!(s.len(), 2);
        assert!(!s.contains(1), "the oldest entry should have been evicted");
        assert!(s.contains(3));
    }

    #[test]
    fn hash_distinguishes_similar_text() {
        assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
        assert_ne!(hash_bytes(b"ab"), hash_bytes(b"ba"));
        assert_eq!(hash_bytes(b"same"), hash_bytes(b"same"));
    }

    #[test]
    fn staging_writes_private_files_that_are_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("tgw-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let text = "set +H\necho hi\n";
        let h = hash_bytes(text.as_bytes());
        let s1 = stage(&dir, text, h).expect("stage");
        assert_eq!(std::fs::read_to_string(&s1.path).unwrap(), text);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&s1.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "staged file is not 0600");
            let dmode = std::fs::metadata(dir.join("stage")).unwrap().permissions().mode();
            assert_eq!(dmode & 0o777, 0o700, "stage dir is not 0700");
        }

        // The same content cannot be staged twice under one id.
        assert!(stage(&dir, text, h).is_err(), "a staged file was overwritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_clipboard_command_is_a_reject_not_a_panic() {
        let mut c = cfg();
        c.clip_get_cmd = "false".into();
        assert!(matches!(
            read_clipboard(&c, Duration::from_secs(2)),
            Err(Reject::ReadFailed(_))
        ));
    }

    #[test]
    fn a_missing_clipboard_command_is_a_reject() {
        let mut c = cfg();
        c.clip_get_cmd = "/nonexistent/tgw-no-such-clip".into();
        assert!(matches!(
            read_clipboard(&c, Duration::from_secs(2)),
            Err(Reject::ReadFailed(_))
        ));
    }

    #[test]
    fn a_clipboard_command_that_reads_stdin_gets_a_closed_stdin() {
        // `cat` would hang forever if stdin were inherited from the terminal.
        let mut c = cfg();
        c.clip_get_cmd = "cat".into();
        let got = read_clipboard(&c, Duration::from_secs(3)).expect("cat with null stdin");
        assert!(got.bytes.is_empty());
    }

    #[test]
    fn a_hanging_clipboard_command_times_out() {
        let mut c = cfg();
        c.clip_get_cmd = "sleep 30".into();
        let start = Instant::now();
        let r = read_clipboard(&c, Duration::from_millis(300));
        assert!(matches!(r, Err(Reject::ReadFailed(_))));
        assert!(start.elapsed() < Duration::from_secs(5), "timeout did not apply");
    }

    #[test]
    fn a_successful_read_returns_stdout() {
        let mut c = cfg();
        c.clip_get_cmd = "printf set\\ +H".into();
        let got = read_clipboard(&c, Duration::from_secs(2)).unwrap();
        assert_eq!(got.bytes, b"set +H");
    }
}
