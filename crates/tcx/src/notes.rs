//! Durable notes for a project.
//!
//! Notes are the *irreplaceable* half of the store: judgment that cannot be
//! rebuilt from the repository. Everything here is written once and never
//! rewritten.
//!
//! # Format
//!
//! One line per note, matching the `.ai/` convention so `tcx import .ai/` is a
//! copy rather than a conversion:
//!
//! ```text
//! <date> | <kind> | <text> | files: <names> | dir: <path> | [keywords]
//! ```
//!
//! Only `date`, `kind` and `text` are required. `files`, `dir` and `[keywords]`
//! are omitted when empty, which keeps short notes short.
//!
//! `dir` is what makes `tcx map` possible: a note attached to a directory is
//! rendered inside the generated tree rather than as a flat line. `kind` is one of
//! `decision`, `gotcha` or `open`; unknown values are preserved verbatim so a
//! newer `tcx` writing a new kind is not mistaken for corruption.
//!
//! # Durability
//!
//! Each note is a single `write` to a file opened with `O_APPEND`. Appends
//! below `PIPE_BUF` are atomic on Linux, so concurrent processes cannot
//! interleave halves of a line. Notes are never truncated or rewritten.

use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The kind of judgment a note records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A choice that was made, and why.
    Decision,
    /// A non-obvious constraint that already caused a bug.
    Gotcha,
    /// Something unresolved.
    Open,
    /// A kind written by a newer version; preserved verbatim.
    Other(String),
}

impl Kind {
    /// Parses a kind, preserving unknown values.
    pub fn parse(s: &str) -> Kind {
        match s {
            "decision" => Kind::Decision,
            "gotcha" => Kind::Gotcha,
            "open" => Kind::Open,
            other => Kind::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Kind::Decision => "decision",
            Kind::Gotcha => "gotcha",
            Kind::Open => "open",
            Kind::Other(s) => s,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// ISO date, `YYYY-MM-DD`.
    pub date: String,
    pub kind: Kind,
    pub text: String,
    /// Files this note is about, if any.
    pub files: Vec<String>,
    /// Free-form tags used by scoped search.
    pub keywords: Vec<String>,
    /// Directory this note describes, relative to the project root.
    ///
    /// Used by `tcx map` to attach judgment to a point in the tree. A note
    /// without `dir` is an ordinary note.
    pub dir: Option<String>,
}

impl Note {
    /// Builds a note, taking the date from the system clock.
    pub fn new(kind: Kind, text: impl Into<String>) -> Note {
        Note {
            date: today(),
            kind,
            text: text.into(),
            files: Vec::new(),
            keywords: Vec::new(),
            dir: None,
        }
    }

    /// Attaches this note to a directory, for `tcx map`.
    pub fn with_dir(mut self, dir: impl Into<String>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files;
        self
    }

    pub fn with_keywords(mut self, keywords: Vec<String>) -> Self {
        self.keywords = keywords;
        self
    }

    /// Renders the note as one line, without a trailing newline.
    ///
    /// Field separators and newlines inside the text are neutralised so a note
    /// can never break the one-line format. This is deliberate: notes are
    /// pasted into conversations, and a stray newline would look like two
    /// entries.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(self.text.len() + 64);
        out.push_str(&self.date);
        out.push_str(" | ");
        out.push_str(self.kind.as_str());
        out.push_str(" | ");
        out.push_str(&sanitize(&self.text));
        if !self.files.is_empty() {
            out.push_str(" | files: ");
            out.push_str(&self.files.join(", "));
        }
        if let Some(dir) = &self.dir {
            let dir = sanitize(dir);
            if !dir.is_empty() {
                out.push_str(" | dir: ");
                out.push_str(&dir);
            }
        }
        if !self.keywords.is_empty() {
            out.push_str(" | [");
            out.push_str(&self.keywords.join(", "));
            out.push(']');
        }
        out
    }

    /// Parses a line produced by [`Note::render`].
    ///
    /// Returns `None` for blank lines and comments, which lets a whole note
    /// file be parsed in one pass.
    pub fn parse(line: &str) -> Option<Note> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }

        let mut parts = line.split(" | ");
        let date = parts.next()?.trim();
        let kind = Kind::parse(parts.next()?.trim());
        let text = parts.next()?.trim();

        let mut files = Vec::new();
        let mut keywords = Vec::new();
        let mut dir = None;
        for extra in parts {
            let extra = extra.trim();
            if let Some(list) = extra.strip_prefix("files:") {
                files = split_list(list);
            } else if let Some(path) = extra.strip_prefix("dir:") {
                let path = path.trim();
                if !path.is_empty() {
                    dir = Some(path.to_owned());
                }
            } else if extra.starts_with('[') && extra.ends_with(']') {
                keywords = split_list(&extra[1..extra.len() - 1]);
            }
        }

        Some(Note {
            date: date.to_owned(),
            kind,
            text: text.to_owned(),
            files,
            keywords,
            dir,
        })
    }
}

/// Appends notes to one project's note file.
#[derive(Debug)]
pub struct Notes {
    path: PathBuf,
}

impl Notes {
    /// Opens (without creating) the note file at `path`.
    pub fn at(path: impl Into<PathBuf>) -> Notes {
        Notes { path: path.into() }
    }

    /// The note file location.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one note as a single atomic write.
    ///
    /// Creates the parent directory and the file if missing. The file is
    /// opened with `O_APPEND` so concurrent writers cannot overwrite each
    /// other's lines.
    pub fn append(&self, note: &Note) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut line = note.render();
        line.push('\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        // One write call: atomic for appends under PIPE_BUF on Linux.
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Appends several notes, one atomic write each.
    pub fn append_all(&self, notes: &[Note]) -> io::Result<usize> {
        for note in notes {
            self.append(note)?;
        }
        Ok(notes.len())
    }

    /// Reads and parses every note.
    ///
    /// A missing file yields an empty list rather than an error: a project
    /// with no notes is normal.
    pub fn read_all(&self) -> io::Result<Vec<Note>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(text.lines().filter_map(Note::parse).collect()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }
}

/// Collapses newlines and trims so a note stays on one line.
fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = false;
    for ch in text.chars() {
        // Control characters, including newlines and carriage returns, become
        // spaces. This keeps the one-line invariant even for pasted text.
        let ch = if ch.is_control() { ' ' } else { ch };
        if ch == ' ' {
            if last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        out.push(ch);
    }
    out.trim().to_owned()
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Today's date as `YYYY-MM-DD`, from the system clock.
///
/// Deliberately not a dependency: the date is only used for ordering and for
/// telling old notes from fresh ones.
fn today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Converts days since 1970-01-01 to a civil date.
///
/// Howard Hinnant's algorithm; valid for the whole range `tcx` will ever see.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
