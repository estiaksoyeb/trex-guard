//! `tcx import` - bring an existing `.ai/` directory into the store.
//!
//! The `.ai/` convention already writes one dated line per note, so importing is
//! mostly a copy. The two things it adds:
//!
//! * a **kind**, taken from the source file (`decisions.md` -> `decision`,
//!   `gotchas.md` -> `gotcha`) or from the line itself when it already has one;
//! * **deduplication**, so importing twice does not double the store.
//!
//! The source directory is never modified. `.ai/` may be tracked in a private
//! repo, and rewriting it would break the "code wins" contract the workflow
//! relies on.

use std::fs;
use std::io;
use std::path::Path;

use crate::notes::{Kind, Note, Notes};

/// Where notes came from, and how they map to kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Decisions,
    Gotchas,
    /// `INDEX.md`, which holds open questions and a project card. Treated as
    /// `open`, since that is the section it is allowed to carry forward.
    Index,
}

impl Source {
    /// The file name this source is read from.
    pub fn file_name(self) -> &'static str {
        match self {
            Source::Decisions => "decisions.md",
            Source::Gotchas => "gotchas.md",
            Source::Index => "INDEX.md",
        }
    }

    /// The default kind for lines that do not carry their own.
    pub fn default_kind(self) -> Kind {
        match self {
            Source::Decisions => Kind::Decision,
            Source::Gotchas => Kind::Gotcha,
            Source::Index => Kind::Open,
        }
    }
}

/// Result of an import.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Notes appended to the store.
    pub added: usize,
    /// Notes skipped because an identical line was already present.
    pub duplicates: usize,
    /// Lines that looked like prose but not like a note.
    pub skipped: usize,
    /// Sources that were not found, by file name.
    pub missing: Vec<&'static str>,
}

/// Imports every known `.ai/` file found in `dir`.
///
/// Returns an error only for I/O failures; a missing file is reported in
/// [`Report::missing`] rather than failing, since partial `.ai/` directories are
/// normal.
pub fn import_dir(dir: &Path, notes: &Notes) -> io::Result<Report> {
    let mut report = Report::default();

    for source in [Source::Decisions, Source::Gotchas, Source::Index] {
        let path = dir.join(source.file_name());
        if !path.is_file() {
            report.missing.push(source.file_name());
            continue;
        }
        let parsed = parse_file(&path, source)?;
        let existing = notes.read_all()?;
        let mut seen: Vec<String> = existing.iter().map(Note::render).collect();

        for note in parsed {
            let rendered = note.render();
            if seen.iter().any(|s| s == &rendered) {
                report.duplicates += 1;
                continue;
            }
            notes.append(&note)?;
            seen.push(rendered);
            report.added += 1;
        }
    }

    Ok(report)
}

/// Parses one `.ai/` file into notes.
///
/// Handles two shapes:
/// * already-formatted notes (`<date> | <kind> | ...`), kept as they are;
/// * bare judgment lines, which get today's date and the source's kind.
///
/// Markdown headings and code fences are skipped rather than imported: they are
/// structure, not judgment. Fenced *content* is skipped too, because a shell
/// command inside a fence is not a note.
pub fn parse_file(path: &Path, source: Source) -> io::Result<Vec<Note>> {
    let text = fs::read_to_string(path)?;
    let mut out = Vec::new();
    let mut in_fence = false;

    for raw in text.lines() {
        let line = raw.trim();

        if line.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('-') && line.len() > 1 {
            // List bullet: strip the marker and treat the rest as text.
            let body = line[1..].trim();
            if body.is_empty() {
                continue;
            }
            out.push(Note::new(source.default_kind(), body));
            continue;
        }
        out.push(parse_line(line, source));
    }

    Ok(out)
}

/// Parses one non-structural line.
///
/// `.ai/` lines are often `<date> | <text>` with no kind, which does not satisfy
/// [`Note::parse`]. Those are recognised explicitly so the date and text survive
/// the import instead of being flattened into a single string.
fn parse_line(line: &str, source: Source) -> Note {
    if let Some(note) = Note::parse(line) {
        return note;
    }

    // `<date> | <text>`: a dated line without a kind.
    let mut parts = line.splitn(2, " | ");
    if let (Some(date), Some(rest)) = (parts.next(), parts.next()) {
        if looks_like_date(date.trim()) {
            return Note {
                date: date.trim().to_owned(),
                kind: source.default_kind(),
                text: rest.trim().to_owned(),
                files: Vec::new(),
                keywords: Vec::new(),
            };
        }
    }

    Note::new(source.default_kind(), line)
}

/// True for `YYYY-MM-DD`.
fn looks_like_date(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s[..4].bytes().all(|b| b.is_ascii_digit())
        && s[5..7].bytes().all(|b| b.is_ascii_digit())
        && s[8..].bytes().all(|b| b.is_ascii_digit())
}
