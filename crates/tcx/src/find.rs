//! `tcx find` - search this project's notes.
//!
//! v1 searches notes only. Searching files and symbols is deliberately deferred
//! until the notes path has proved useful, because a half-built symbol index is
//! worse than none.
//!
//! Matching is a plain case-insensitive substring test over the rendered note.
//! Deliberately dumb: a regex engine that can hang on a hostile note is a worse
//! trade than a predictable substring scan.

use crate::notes::Note;

/// What a query matched against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit<'a> {
    pub note: &'a Note,
}

/// Searches `notes` for `query`, returning matches in file order.
///
/// An empty query matches everything; the caller decides whether that is what
/// it wants.
pub fn find<'a>(notes: &'a [Note], query: &str) -> Vec<Hit<'a>> {
    let needle = query.trim().to_lowercase();
    notes
        .iter()
        .filter(|n| needle.is_empty() || n.render().to_lowercase().contains(&needle))
        .map(|note| Hit { note })
        .collect()
}

/// Lists all distinct keywords across `notes`, most frequent first.
///
/// Useful when the caller has no keyword in mind and needs a way in.
pub fn keywords(notes: &[Note]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for note in notes {
        for kw in &note.keywords {
            *counts.entry(kw.to_lowercase()).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    // Most frequent first, then alphabetical for a stable order.
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}
