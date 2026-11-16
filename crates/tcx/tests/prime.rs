//! Unit tests for the `prime` briefing.

use tcx::notes::{Kind, Note};
use tcx::prime::{self, Options, Stamp};

fn stamp() -> Stamp {
    Stamp {
        head: Some("abc1234".into()),
        branch: "main".into(),
        dirty: 2,
        shallow: false,
        kind: "git".into(),
    }
}

fn note(date: &str, kind: Kind, text: &str) -> Note {
    Note {
        date: date.into(),
        kind,
        text: text.into(),
        files: Vec::new(),
        keywords: Vec::new(),
    }
}

// -------------------------------------------------------------------------
// stamp
// -------------------------------------------------------------------------

#[test]
fn stamp_renders_compact_line() {
    assert_eq!(stamp().render(), "HEAD abc1234 main dirty 2 tcx");
}

#[test]
fn stamp_marks_shallow() {
    let s = Stamp {
        shallow: true,
        ..stamp()
    };
    assert_eq!(s.render(), "HEAD abc1234 main dirty 2 shallow tcx");
}

#[test]
fn stamp_handles_empty_repo() {
    let s = Stamp {
        head: None,
        branch: "no-branch".into(),
        dirty: 0,
        shallow: false,
        kind: "git empty".into(),
    };
    assert_eq!(s.render(), "HEAD none no-branch dirty 0 tcx");
}

// -------------------------------------------------------------------------
// golden output
// -------------------------------------------------------------------------

#[test]
fn golden_briefing() {
    let notes = vec![
        note("2026-11-16", Kind::Gotcha, "mtime lies under proot"),
        note("2026-11-15", Kind::Decision, "root commit is the identity"),
    ];
    let text = prime::build(&stamp(), &notes, &Options::default());
    let expected = "\
# tcx brief
HEAD abc1234 main dirty 2 tcx

2026-11-16 | gotcha | mtime lies under proot
2026-11-15 | decision | root commit is the identity
";
    assert_eq!(text, expected);
}

#[test]
fn empty_notes_says_so() {
    let text = prime::build(&stamp(), &[], &Options::default());
    assert!(text.contains("# tcx brief"));
    assert!(text.contains("No notes yet"));
}

// -------------------------------------------------------------------------
// ordering
// -------------------------------------------------------------------------

#[test]
fn gotchas_come_first_then_open_then_decisions() {
    let notes = vec![
        note("2026-11-16", Kind::Decision, "a decision"),
        note("2026-11-16", Kind::Gotcha, "a gotcha"),
        note("2026-11-16", Kind::Open, "an open question"),
    ];
    let text = prime::build(&stamp(), &notes, &Options::default());
    let gotcha = text.find("a gotcha").unwrap();
    let open = text.find("an open question").unwrap();
    let decision = text.find("a decision").unwrap();
    assert!(gotcha < open, "gotchas before open");
    assert!(open < decision, "open before decisions");
}

#[test]
fn decisions_are_newest_first() {
    let notes = vec![
        note("2026-01-01", Kind::Decision, "older"),
        note("2026-11-16", Kind::Decision, "newer"),
        note("2026-06-01", Kind::Decision, "middle"),
    ];
    let text = prime::build(&stamp(), &notes, &Options::default());
    let newer = text.find("newer").unwrap();
    let middle = text.find("middle").unwrap();
    let older = text.find("older").unwrap();
    assert!(newer < middle && middle < older, "newest first:\n{text}");
}

// -------------------------------------------------------------------------
// budget
// -------------------------------------------------------------------------

#[test]
fn budget_omits_notes_and_reports_count() {
    let mut notes = Vec::new();
    for i in 0..50 {
        notes.push(note("2026-11-16", Kind::Decision, &format!("decision number {i}")));
    }
    let text = prime::build(&stamp(), &notes, &Options { budget: 200 });
    assert!(text.contains("more (tcx find)"), "should report omissions:\n{text}");
    // Something must have been kept: the first note always fits.
    assert!(text.contains("decision number"));
}

#[test]
fn budget_keeps_everything_when_generous() {
    let notes = vec![
        note("2026-11-16", Kind::Decision, "one"),
        note("2026-11-16", Kind::Decision, "two"),
    ];
    let text = prime::build(&stamp(), &notes, &Options { budget: 100_000 });
    assert!(!text.contains("more (tcx find)"));
    assert!(text.contains("one") && text.contains("two"));
}

#[test]
fn truncation_keeps_priority_order() {
    // A tight budget plus a mix: gotchas must survive over decisions.
    let mut notes = vec![note("2026-11-16", Kind::Gotcha, "important gotcha")];
    for i in 0..30 {
        notes.push(note("2026-11-16", Kind::Decision, &format!("filler decision {i}")));
    }
    let text = prime::build(&stamp(), &notes, &Options { budget: 120 });
    assert!(text.contains("important gotcha"), "gotcha must survive:\n{text}");
}

// -------------------------------------------------------------------------
// sanitisation
// -------------------------------------------------------------------------

#[test]
fn control_characters_never_reach_output() {
    let notes = vec![note("2026-11-16", Kind::Decision, "evil \x1b[2J clear")];
    let text = prime::build(&stamp(), &notes, &Options::default());
    assert!(!text.contains('\x1b'), "ESC survived: {text:?}");
    assert!(!text.contains("[2J"), "sequence survived:\n{text}");
    assert!(text.contains("evil"), "text around the sequence is kept");
}

#[test]
fn ordinary_brackets_survive_sanitisation() {
    // The CSI stripper must not eat legitimate prose.
    let notes = vec![note(
        "2026-11-16",
        Kind::Decision,
        "index an array[5] and read list[2] plus map[key]",
    )];
    let text = prime::build(&stamp(), &notes, &Options::default());
    assert!(text.contains("array[5]"), "got:\n{text}");
    assert!(text.contains("list[2]"), "got:\n{text}");
    assert!(text.contains("map[key]"), "got:\n{text}");
}

#[test]
fn sgr_sequences_are_stripped() {
    let notes = vec![note("2026-11-16", Kind::Decision, "bold \x1b[1;32mtext\x1b[0m here")];
    let text = prime::build(&stamp(), &notes, &Options::default());
    assert!(!text.contains("[1;32m"), "SGR survived:\n{text}");
    assert!(text.contains("bold") && text.contains("here"));
}

#[test]
fn long_lines_are_truncated() {
    let long = "x".repeat(1000);
    let notes = vec![note("2026-11-16", Kind::Decision, &long)];
    let text = prime::build(&stamp(), &notes, &Options { budget: 100_000 });
    assert!(text.contains('…'), "long note should be truncated");
    assert!(!text.contains(&"x".repeat(400)), "note kept too much");
}

// -------------------------------------------------------------------------
// determinism
// -------------------------------------------------------------------------

#[test]
fn output_is_deterministic() {
    let notes = vec![
        note("2026-11-16", Kind::Decision, "a"),
        note("2026-11-16", Kind::Gotcha, "b"),
        note("2026-11-16", Kind::Open, "c"),
    ];
    let first = prime::build(&stamp(), &notes, &Options::default());
    for _ in 0..10 {
        assert_eq!(first, prime::build(&stamp(), &notes, &Options::default()));
    }
}

#[test]
fn output_ends_with_newline() {
    let text = prime::build(&stamp(), &[], &Options::default());
    assert!(text.ends_with('\n'));
}
