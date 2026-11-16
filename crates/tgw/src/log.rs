//! The event log: one line per decision, and a copy of every block.
//!
//! # Format
//!
//! ```text
//! ts | id | hash | verdict | reason | exit | duration_ms
//! ```
//!
//! Fixed field count, `|`-separated, so `tgw log` and any script can parse it.
//! Fields never contain `|` (the reason is sanitised), and never contain a
//! newline (the same sanitising collapses whitespace).
//!
//! # Why every non-ignore verdict keeps its block
//!
//! A hold that expires, a block that would have run, a run that produced an
//! error — after the fact, the only useful artefact is the exact bytes. The
//! log line says what happened; the saved block says to what.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// What the pipeline decided about a clipboard payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not a block, already seen, or unreadable. Nothing happens.
    Ignore,
    /// The gate refused it. It can never run.
    Block,
    /// The gate passed it, but a human must approve first.
    Hold,
    /// The gate passed it and the mode allows running it.
    Run,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Ignore => "ignore",
            Verdict::Block => "block",
            Verdict::Hold => "hold",
            Verdict::Run => "run",
        }
    }
}

/// One log record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub ts: u64,
    pub id: String,
    pub hash: u64,
    pub verdict: Verdict,
    pub reason: String,
    /// Exit code of the gate or the run, when there was one.
    pub exit: Option<i32>,
    pub duration_ms: u64,
}

impl Entry {
    pub fn new(verdict: Verdict, reason: impl Into<String>) -> Entry {
        Entry {
            ts: now_unix(),
            id: String::new(),
            hash: 0,
            verdict,
            reason: reason.into(),
            exit: None,
            duration_ms: 0,
        }
    }

    /// Renders one line. `|` and newlines in the reason are replaced so the
    /// field count is always seven.
    pub fn render(&self) -> String {
        let reason: String = self
            .reason
            .chars()
            .map(|c| if c == '|' || c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        let exit = self.exit.map(|e| e.to_string()).unwrap_or_else(|| "-".into());
        format!(
            "{} | {} | {:016x} | {} | {} | {} | {}",
            self.ts,
            if self.id.is_empty() { "-" } else { &self.id },
            self.hash,
            self.verdict.as_str(),
            reason.trim(),
            exit,
            self.duration_ms
        )
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Appends to `log_dir/events.log` and keeps block copies in `log_dir/blocks`.
#[derive(Debug, Clone)]
pub struct Log {
    dir: PathBuf,
    /// How many block copies to keep.
    retention: usize,
}

impl Log {
    pub fn new(dir: impl Into<PathBuf>, retention: usize) -> Log {
        Log {
            dir: dir.into(),
            retention: retention.max(1),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn events_path(&self) -> PathBuf {
        self.dir.join("events.log")
    }

    /// Appends one line. A log write failure is reported but must not stop
    /// the pipeline: losing a log line is bad, refusing to run a block
    /// because the disk is full is worse.
    pub fn append(&self, entry: &Entry) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;
        writeln!(f, "{}", entry.render())
    }

    /// Keeps a copy of a block that was held or run, pruning old ones.
    pub fn keep_block(&self, id: &str, text: &str) -> std::io::Result<PathBuf> {
        let dir = self.dir.join("blocks");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{id}.sh"));
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;
        f.write_all(text.as_bytes())?;
        self.prune_blocks(&dir);
        Ok(path)
    }

    /// Drops the oldest block copies beyond `retention`.
    fn prune_blocks(&self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let m = e.metadata().ok()?;
                if !m.is_file() {
                    return None;
                }
                Some((m.modified().unwrap_or(UNIX_EPOCH), p))
            })
            .collect();
        if files.len() <= self.retention {
            return;
        }
        files.sort_by_key(|(t, _)| *t);
        let drop_n = files.len() - self.retention;
        for (_, p) in files.into_iter().take(drop_n) {
            let _ = std::fs::remove_file(p);
        }
    }

    /// The last `n` entries.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(self.events_path()) else {
            return Vec::new();
        };
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        lines[start..].iter().map(|s| s.to_string()).collect()
    }

    /// Counts entries with a verdict, for `status`.
    pub fn count(&self, verdict: Verdict) -> usize {
        let Ok(text) = std::fs::read_to_string(self.events_path()) else {
            return 0;
        };
        let want = format!(" | {} | ", verdict.as_str());
        text.lines().filter(|l| l.contains(&want)).count()
    }
}

/// A one-line summary for a notification.
pub fn summary(verdict: Verdict, reason: &str, id: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, "{}", verdict.as_str());
    if !reason.is_empty() {
        let _ = write!(s, ": {reason}");
    }
    if !id.is_empty() {
        let _ = write!(s, " [{id}]");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tgw-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn an_entry_renders_seven_fields() {
        let mut e = Entry::new(Verdict::Hold, "rule=rm risk=review");
        e.id = "abc123".into();
        e.hash = 0xdead_beef;
        e.exit = Some(3);
        e.duration_ms = 12;
        let line = e.render();
        assert_eq!(line.split(" | ").count(), 7, "{line}");
        assert!(line.contains("hold"));
    }

    #[test]
    fn a_pipe_or_newline_in_the_reason_cannot_add_fields() {
        let e = Entry::new(Verdict::Block, "bad | reason\nnext line");
        let line = e.render();
        assert_eq!(line.split(" | ").count(), 7, "{line}");
        assert!(!line.contains('\n'), "{line}");
    }

    #[test]
    fn append_and_tail_round_trip() {
        let d = tmp("tail");
        let log = Log::new(&d, 5);
        for i in 0..3 {
            log.append(&Entry::new(Verdict::Ignore, format!("n{i}"))).unwrap();
        }
        let got = log.tail(2);
        assert_eq!(got.len(), 2);
        assert!(got[1].contains("n2"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tail_on_a_missing_log_is_empty_not_an_error() {
        let d = tmp("nolof");
        let log = Log::new(&d, 5);
        assert!(log.tail(10).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn counts_are_per_verdict() {
        let d = tmp("count");
        let log = Log::new(&d, 5);
        log.append(&Entry::new(Verdict::Block, "a")).unwrap();
        log.append(&Entry::new(Verdict::Block, "b")).unwrap();
        log.append(&Entry::new(Verdict::Run, "c")).unwrap();
        assert_eq!(log.count(Verdict::Block), 2);
        assert_eq!(log.count(Verdict::Run), 1);
        assert_eq!(log.count(Verdict::Hold), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn block_copies_are_pruned_to_retention() {
        let d = tmp("prune");
        let log = Log::new(&d, 3);
        for i in 0..6 {
            log.keep_block(&format!("id{i}"), "set +H").unwrap();
            // Filesystem timestamps can be coarse; make each distinct.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let n = std::fs::read_dir(d.join("blocks")).unwrap().count();
        assert_eq!(n, 3, "retention was not applied");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn summary_mentions_verdict_reason_and_id() {
        let s = summary(Verdict::Hold, "rule=rm", "abc");
        assert!(s.contains("hold"));
        assert!(s.contains("rule=rm"));
        assert!(s.contains("abc"));
    }
}
