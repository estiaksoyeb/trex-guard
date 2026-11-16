//! Unit tests for note rendering, parsing and appending.

use std::fs;

use tcx::notes::{Kind, Note, Notes};

fn temp_path(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "tcx-notes-{}-{}-{}",
        std::process::id(),
        label,
        n
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.join("notes.md")
}

fn note(text: &str) -> Note {
    Note {
        date: "2026-11-16".into(),
        kind: Kind::Decision,
        text: text.into(),
        files: Vec::new(),
        keywords: Vec::new(),
    }
}

// -------------------------------------------------------------------------
// rendering
// -------------------------------------------------------------------------

#[test]
fn renders_minimal_note() {
    assert_eq!(
        note("use root commit for identity").render(),
        "2026-11-16 | decision | use root commit for identity"
    );
}

#[test]
fn renders_with_files_and_keywords() {
    let n = note("adopt provisional store")
        .with_files(vec!["src/id.rs".into(), "src/store.rs".into()])
        .with_keywords(vec!["identity".into(), "migration".into()]);
    assert_eq!(
        n.render(),
        "2026-11-16 | decision | adopt provisional store \
         | files: src/id.rs, src/store.rs | [identity, migration]"
    );
}

#[test]
fn all_kinds_render() {
    for (kind, label) in [
        (Kind::Decision, "decision"),
        (Kind::Gotcha, "gotcha"),
        (Kind::Open, "open"),
    ] {
        let n = Note {
            kind,
            ..note("x")
        };
        assert_eq!(n.render(), format!("2026-11-16 | {label} | x"));
    }
}

#[test]
fn unknown_kind_is_preserved() {
    let k = Kind::parse("hypothesis");
    assert_eq!(k, Kind::Other("hypothesis".into()));
    assert_eq!(k.as_str(), "hypothesis");
    let n = Note {
        kind: k,
        ..note("a guess")
    };
    assert_eq!(n.render(), "2026-11-16 | hypothesis | a guess");
}

// -------------------------------------------------------------------------
// sanitisation: a note must never break the one-line format
// -------------------------------------------------------------------------

#[test]
fn newlines_in_text_are_collapsed() {
    let n = note("first line\nsecond line\r\nthird");
    let rendered = n.render();
    assert!(!rendered.contains('\n'));
    assert!(!rendered.contains('\r'));
    assert_eq!(rendered, "2026-11-16 | decision | first line second line third");
}

#[test]
fn escape_and_control_chars_are_stripped() {
    let n = note("evil \x1b[2J clear \x07 bell \x00 nul");
    let rendered = n.render();
    for bad in ['\x1b', '\x07', '\x00'] {
        assert!(!rendered.contains(bad), "control {bad:?} survived: {rendered:?}");
    }
}

#[test]
fn repeated_spaces_are_squashed() {
    let n = note("a    b\t\tc");
    assert_eq!(n.render(), "2026-11-16 | decision | a b c");
}

// -------------------------------------------------------------------------
// parsing
// -------------------------------------------------------------------------

#[test]
fn round_trips_through_parse() {
    let original = note("identity uses root commits")
        .with_files(vec!["src/id.rs".into()])
        .with_keywords(vec!["identity".into()]);
    let rendered = original.render();
    let parsed = Note::parse(&rendered).expect("parse");
    assert_eq!(parsed, original);
}

#[test]
fn parses_note_without_files_or_keywords() {
    let parsed = Note::parse("2026-11-16 | open | does prime earn its keep?").unwrap();
    assert_eq!(parsed.date, "2026-11-16");
    assert_eq!(parsed.kind, Kind::Open);
    assert_eq!(parsed.text, "does prime earn its keep?");
    assert!(parsed.files.is_empty());
    assert!(parsed.keywords.is_empty());
}

#[test]
fn ignores_blank_and_comment_lines() {
    assert!(Note::parse("").is_none());
    assert!(Note::parse("   ").is_none());
    assert!(Note::parse("# a comment").is_none());
}

#[test]
fn rejects_lines_without_enough_fields() {
    assert!(Note::parse("2026-11-16 | decision").is_none());
    assert!(Note::parse("just some text").is_none());
}

#[test]
fn files_with_commas_parse_as_a_list() {
    let parsed =
        Note::parse("2026-11-16 | gotcha | x | files: a.rs, b.rs, c.rs | [t1, t2]").unwrap();
    assert_eq!(parsed.files, vec!["a.rs", "b.rs", "c.rs"]);
    assert_eq!(parsed.keywords, vec!["t1", "t2"]);
}

// -------------------------------------------------------------------------
// appending
// -------------------------------------------------------------------------

#[test]
fn append_creates_file_and_parent() {
    let path = temp_path("create");
    // Ask for a deeper path so parent creation is exercised.
    let nested = path.parent().unwrap().join("deep/nested/notes.md");
    let notes = Notes::at(&nested);
    notes.append(&note("first")).unwrap();
    let text = fs::read_to_string(&nested).unwrap();
    assert_eq!(text, "2026-11-16 | decision | first\n");
}

#[test]
fn append_preserves_existing_lines() {
    let path = temp_path("preserve");
    let notes = Notes::at(&path);
    notes.append(&note("one")).unwrap();
    notes.append(&note("two")).unwrap();
    notes.append(&note("three")).unwrap();

    let text = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].ends_with("| one"));
    assert!(lines[1].ends_with("| two"));
    assert!(lines[2].ends_with("| three"));
    assert!(text.ends_with('\n'));
}

#[test]
fn read_all_round_trips_appended_notes() {
    let path = temp_path("readall");
    let notes = Notes::at(&path);
    let a = note("one").with_files(vec!["a.rs".into()]);
    let b = Note {
        kind: Kind::Gotcha,
        ..note("two")
    };
    notes.append(&a).unwrap();
    notes.append(&b).unwrap();

    let all = notes.read_all().unwrap();
    assert_eq!(all, vec![a, b]);
}

#[test]
fn read_all_on_missing_file_is_empty() {
    let path = temp_path("missing");
    let notes = Notes::at(&path);
    assert!(notes.read_all().unwrap().is_empty());
}

#[test]
fn append_all_writes_each_note() {
    let path = temp_path("appendall");
    let notes = Notes::at(&path);
    let batch = vec![note("a"), note("b"), note("c")];
    assert_eq!(notes.append_all(&batch).unwrap(), 3);
    assert_eq!(notes.read_all().unwrap(), batch);
}

#[test]
fn concurrent_writers_do_not_interleave() {
    use std::sync::Arc;
    use std::thread;

    let path = temp_path("concurrent");
    let notes = Arc::new(Notes::at(&path));
    let mut handles = Vec::new();
    for t in 0..8 {
        let notes = Arc::clone(&notes);
        handles.push(thread::spawn(move || {
            for i in 0..25 {
                notes
                    .append(&note(&format!("thread{t}-note{i}")))
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let all = notes.read_all().unwrap();
    assert_eq!(all.len(), 8 * 25, "no note may be lost");
    // Every line must be a complete, parseable note: no interleaved halves.
    for n in &all {
        assert!(n.text.starts_with("thread"), "corrupt line: {:?}", n.text);
        assert!(n.text.contains("-note"), "corrupt line: {:?}", n.text);
    }
}

// -------------------------------------------------------------------------
// dates
// -------------------------------------------------------------------------

#[test]
fn new_note_has_iso_date() {
    let n = Note::new(Kind::Decision, "x");
    // YYYY-MM-DD, plausible year.
    assert_eq!(n.date.len(), 10);
    assert_eq!(n.date.as_bytes()[4], b'-');
    assert_eq!(n.date.as_bytes()[7], b'-');
    let year: i32 = n.date[..4].parse().expect("numeric year");
    assert!((2020..=2100).contains(&year), "unexpected year in {}", n.date);
}
