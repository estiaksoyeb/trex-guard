//! `tcx prime` - the session briefing.
//!
//! `prime` produces a small, deterministic, paste-ready block that primes a new
//! chat session: a freshness stamp, then the project's notes, in priority
//! order, under a byte budget.
//!
//! # Why a stamp
//!
//! Without one, `prime` is just a notes dump and no better than pasting
//! `.ai/INDEX.md` by hand. The stamp tells the reader *how stale* the view is
//! (`HEAD`, branch, dirty count) without listing paths.
//!
//! # Security
//!
//! Everything derived from the repository is sanitised: control characters and
//! ESC are stripped and long strings are truncated. `prime` output is pasted
//! into a terminal *and* a chat, so it is both an escape-sequence channel and a
//! prompt-injection channel.

use std::fmt::Write as _;

use crate::notes::{Kind, Note};

/// Where a note falls in the truncation order.
///
/// Gotchas first: they are the notes that already caused a bug. Open questions
/// next, because they say where to look. Decisions last, newest first, because
/// there are usually many and older ones matter less.
fn priority(kind: &Kind) -> u8 {
    match kind {
        Kind::Gotcha => 0,
        Kind::Open => 1,
        Kind::Decision => 2,
        Kind::Other(_) => 2,
    }
}

/// A freshness stamp for the briefing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stamp {
    /// Short `HEAD` hash, or `none` when the repo is empty.
    pub head: Option<String>,
    /// Branch name, or a marker for a detached head.
    pub branch: String,
    /// Number of files reported by `git status --porcelain`. Names are
    /// deliberately excluded: the stamp is pasted into chats.
    pub dirty: usize,
    /// Whether the clone is shallow, which makes identity provisional.
    pub shallow: bool,
    /// Identity kind label, e.g. `git` or `git empty`.
    pub kind: String,
}

impl Stamp {
    /// Renders the stamp as a single line.
    pub fn render(&self) -> String {
        let head = self.head.as_deref().unwrap_or("none");
        let mut out = format!("HEAD {head} {}", self.branch);
        let _ = write!(out, " dirty {}", self.dirty);
        if self.shallow {
            out.push_str(" shallow");
        }
        out.push_str(" tcx");
        out
    }
}

/// Options controlling how much `prime` prints.
#[derive(Debug, Clone)]
pub struct Options {
    /// Soft cap on the size of the notes section, in bytes.
    ///
    /// Byte budget, not tokens: bytes are what a terminal and a chat both
    /// measure deterministically.
    pub budget: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { budget: 6000 }
    }
}

/// Builds the briefing.
///
/// Output is deterministic for a given input, so it is golden-file testable.
pub fn build(stamp: &Stamp, notes: &[Note], opts: &Options) -> String {
    let mut out = String::new();
    out.push_str("# tcx brief\n");
    out.push_str(&stamp.render());
    out.push('\n');

    if notes.is_empty() {
        out.push_str("\nNo notes yet. Add one with `tcx note`.\n");
        return out;
    }

    let mut section = String::new();
    let mut omitted = 0usize;
    for note in ordered(notes) {
        let line = format!("{}\n", sanitize_line(&note.render()));
        if section.len() + line.len() > opts.budget {
            omitted += 1;
            continue;
        }
        section.push_str(&line);
    }

    out.push('\n');
    out.push_str(&section);
    if omitted > 0 {
        let _ = writeln!(out, "… {omitted} more (tcx find)");
    }
    out
}

/// Notes in the order a briefing shows them.
///
/// Shared with `briefing` so `prime` and `tg resume` never disagree about what
/// matters most.
pub fn ordered(notes: &[Note]) -> Vec<&Note> {
    // Sort by priority, then newest first within a priority.
    // Dates are ISO, so lexical order is chronological.
    let mut ordered: Vec<&Note> = notes.iter().collect();
    ordered.sort_by(|a, b| {
        priority(&a.kind)
            .cmp(&priority(&b.kind))
            .then_with(|| b.date.cmp(&a.date))
    });
    ordered
}

/// Strips control characters and escape sequences, then truncates.
///
/// A filename or symbol can contain an escape sequence or read like an
/// instruction. Both are neutralised here, before the text can reach a terminal
/// or a model.
///
/// Two layers matter: [`crate::notes`] already removes ESC from note text, but
/// `prime` also handles text that did not come through that path, and a stray
/// ESC alone is not the only risk — a surviving `[2J` reads as an instruction to
/// a model even when it cannot execute in a terminal.
pub fn sanitize_line(text: &str) -> String {
    const MAX: usize = 300;
    let mut out = String::with_capacity(text.len().min(MAX));
    let mut chars = 0;

    for ch in text.chars() {
        if chars >= MAX {
            out.push('…');
            break;
        }
        // Drop ESC and the sequence that follows it (CSI: ESC [ ... final).
        // Note text from `notes` has already lost the ESC byte, so also drop a
        // bare CSI introducer followed by parameters and a final byte.
        if ch.is_control() {
            if ch == '\t' {
                out.push(' ');
                chars += 1;
            }
            continue;
        }
        out.push(ch);
        chars += 1;
    }

    strip_bare_csi(&out)
}

/// Removes `[<params><final>` CSI fragments left behind when ESC was stripped.
///
/// Deliberately narrow, because this runs on ordinary prose too. A group is only
/// removed when it ends with a final byte that real terminal sequences use
/// (`m J K H A B C D f s u h l`). That keeps text like `array[5]` or `list[2]`
/// intact, since `]` and similar are not CSI finals.
fn strip_bare_csi(text: &str) -> String {
    const FINALS: &str = "mJKHABCDfFsuhlLpP@PX";
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            let mut j = i + 1;
            let mut has_digit = false;
            while j < chars.len()
                && (chars[j].is_ascii_digit() || matches!(chars[j], ';' | ':' | '?'))
            {
                if chars[j].is_ascii_digit() {
                    has_digit = true;
                }
                j += 1;
            }
            if has_digit && j < chars.len() && FINALS.contains(chars[j]) {
                // Looks like a CSI: drop `[` .. final inclusive.
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
