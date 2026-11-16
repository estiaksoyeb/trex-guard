//! Tests for `tcx find` and `tcx import`.
//!
//! Both unit-level (library) and end-to-end (binary) coverage.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use tcx::find;
use tcx::import::{self, Source};
use tcx::notes::{Kind, Note, Notes};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tcx-fi-{}-{}-{}",
            std::process::id(),
            label,
            n
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn note(date: &str, kind: Kind, text: &str, tags: &[&str]) -> Note {
    Note {
        date: date.into(),
        kind,
        text: text.into(),
        files: Vec::new(),
        keywords: tags.iter().map(|s| s.to_string()).collect(),
    }
}

// -------------------------------------------------------------------------
// find (library)
// -------------------------------------------------------------------------

#[test]
fn find_matches_substring_case_insensitively() {
    let notes = vec![
        note("2026-11-16", Kind::Decision, "Identity uses root commits", &[]),
        note("2026-11-16", Kind::Gotcha, "mtime lies under proot", &[]),
    ];
    // Case-insensitive: ROOT matches "root". Note that "proot" also contains
    // "root" as a substring, so a bare substring search matches both. That is
    // the documented v1 behaviour: dumb, predictable, no regex.
    let hits = find::find(&notes, "ROOT");
    assert_eq!(hits.len(), 2);
    assert!(hits[0].note.text.contains("root commits"));

    // A term that only appears in one note is filtered correctly.
    let one = find::find(&notes, "identity");
    assert_eq!(one.len(), 1);
}

#[test]
fn find_empty_query_matches_everything() {
    let notes = vec![
        note("2026-11-16", Kind::Decision, "a", &[]),
        note("2026-11-16", Kind::Decision, "b", &[]),
    ];
    assert_eq!(find::find(&notes, "").len(), 2);
    assert_eq!(find::find(&notes, "   ").len(), 2);
}

#[test]
fn find_returns_nothing_for_no_match() {
    let notes = vec![note("2026-11-16", Kind::Decision, "a", &[])];
    assert!(find::find(&notes, "zzz").is_empty());
}

#[test]
fn find_searches_keywords_and_files_too() {
    let mut n = note("2026-11-16", Kind::Gotcha, "unrelated text", &["procmtime"]);
    n.files = vec!["src/id.rs".into()];
    let notes = vec![n];

    assert_eq!(find::find(&notes, "procmtime").len(), 1);
    assert_eq!(find::find(&notes, "id.rs").len(), 1);
    assert_eq!(find::find(&notes, "unrelated").len(), 1);
}

#[test]
fn find_preserves_file_order() {
    let notes = vec![
        note("2026-11-01", Kind::Decision, "first match", &[]),
        note("2026-11-16", Kind::Decision, "second match", &[]),
    ];
    let hits = find::find(&notes, "match");
    assert_eq!(hits[0].note.date, "2026-11-01");
    assert_eq!(hits[1].note.date, "2026-11-16");
}

#[test]
fn keywords_are_counted_and_sorted() {
    let notes = vec![
        note("2026-11-16", Kind::Decision, "a", &["identity", "git"]),
        note("2026-11-16", Kind::Decision, "b", &["identity"]),
        note("2026-11-16", Kind::Gotcha, "c", &["Proot"]),
    ];
    let tags = find::keywords(&notes);
    assert_eq!(tags[0], ("identity".to_string(), 2), "most frequent first");
    // Remaining are alphabetical, case-folded.
    assert_eq!(tags[1], ("git".to_string(), 1));
    assert_eq!(tags[2], ("proot".to_string(), 1));
}

#[test]
fn keywords_on_empty_is_empty() {
    assert!(find::keywords(&[]).is_empty());
}

// -------------------------------------------------------------------------
// import (library)
// -------------------------------------------------------------------------

#[test]
fn import_maps_files_to_kinds() {
    let tmp = TempDir::new("import-kinds");
    let ai = tmp.path().join(".ai");
    fs::create_dir_all(&ai).unwrap();
    fs::write(
        ai.join("decisions.md"),
        "# Decisions\n\n2026-11-01 | tap dispatch lives in PlayerView\n- a bullet decision\n",
    )
    .unwrap();
    fs::write(ai.join("gotchas.md"), "mtime lies under proot\n").unwrap();

    let decisions = import::parse_file(&ai.join("decisions.md"), Source::Decisions).unwrap();
    // Heading skipped, dated line parsed, bullet stripped.
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0].kind, Kind::Decision);
    assert_eq!(decisions[0].text, "tap dispatch lives in PlayerView");
    assert_eq!(decisions[1].text, "a bullet decision");

    let gotchas = import::parse_file(&ai.join("gotchas.md"), Source::Gotchas).unwrap();
    assert_eq!(gotchas.len(), 1);
    assert_eq!(gotchas[0].kind, Kind::Gotcha);
}

#[test]
fn import_preserves_existing_kind_in_line() {
    let tmp = TempDir::new("import-kind");
    let f = tmp.path().join("decisions.md");
    fs::write(&f, "2026-11-01 | gotcha | actually a gotcha\n").unwrap();
    let parsed = import::parse_file(&f, Source::Decisions).unwrap();
    assert_eq!(parsed[0].kind, Kind::Gotcha, "the line's own kind wins");
}

#[test]
fn import_skips_code_fences() {
    let tmp = TempDir::new("import-fence");
    let f = tmp.path().join("gotchas.md");
    fs::write(&f, "```bash\nrm -rf x\n```\nreal gotcha\n").unwrap();
    let parsed = import::parse_file(&f, Source::Gotchas).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].text, "real gotcha");
}

#[test]
fn import_dir_dedupes_on_second_run() {
    let tmp = TempDir::new("import-dedupe");
    let ai = tmp.path().join(".ai");
    fs::create_dir_all(&ai).unwrap();
    fs::write(ai.join("decisions.md"), "a decision\nanother decision\n").unwrap();

    let notes = Notes::at(tmp.path().join("store/notes.md"));
    let first = import::import_dir(&ai, &notes).unwrap();
    assert_eq!(first.added, 2);
    assert_eq!(notes.read_all().unwrap().len(), 2);

    let second = import::import_dir(&ai, &notes).unwrap();
    assert_eq!(second.added, 0);
    assert_eq!(second.duplicates, 2);
    assert_eq!(notes.read_all().unwrap().len(), 2, "no doubling");
}

#[test]
fn import_dir_reports_missing_files() {
    let tmp = TempDir::new("import-missing");
    let ai = tmp.path().join(".ai");
    fs::create_dir_all(&ai).unwrap();
    fs::write(ai.join("gotchas.md"), "only a gotcha\n").unwrap();

    let notes = Notes::at(tmp.path().join("store/notes.md"));
    let report = import::import_dir(&ai, &notes).unwrap();
    assert_eq!(report.added, 1);
    assert!(report.missing.contains(&"decisions.md"));
    assert!(report.missing.contains(&"INDEX.md"));
    assert!(!report.missing.contains(&"gotchas.md"));
}

#[test]
fn import_never_modifies_the_source() {
    let tmp = TempDir::new("import-readonly");
    let ai = tmp.path().join(".ai");
    fs::create_dir_all(&ai).unwrap();
    let f = ai.join("decisions.md");
    let original = "# Decisions\n2026-11-01 | keep me\n";
    fs::write(&f, original).unwrap();

    let notes = Notes::at(tmp.path().join("store/notes.md"));
    import::import_dir(&ai, &notes).unwrap();

    assert_eq!(fs::read_to_string(&f).unwrap(), original, ".ai/ must not change");
}

// -------------------------------------------------------------------------
// end to end
// -------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?}");
}

fn tcx(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tcx"))
        .args(args)
        .env("TCX_HOME", home)
        .output()
        .expect("run tcx")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn project(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    (home, repo)
}

#[test]
fn cli_find_filters_notes() {
    let tmp = TempDir::new("cli-find");
    let (home, repo) = project(&tmp);
    let repo_s = repo.to_str().unwrap();

    tcx(&home, &["note", "--path", repo_s, "identity uses root commits"]);
    tcx(&home, &["note", "--path", repo_s, "-k", "gotcha", "mtime lies"]);

    let hits = stdout(&tcx(&home, &["find", "--path", repo_s, "identity"]));
    assert_eq!(hits.lines().count(), 1, "got:\n{hits}");
    assert!(hits.contains("root commits"));

    let none = stdout(&tcx(&home, &["find", "--path", repo_s, "zzzz"]));
    assert_eq!(none, "");
}

#[test]
fn cli_find_tags_lists_keywords() {
    let tmp = TempDir::new("cli-tags");
    let (home, repo) = project(&tmp);
    let repo_s = repo.to_str().unwrap();
    tcx(&home, &["note", "--path", repo_s, "-t", "identity", "a"]);
    tcx(&home, &["note", "--path", repo_s, "-t", "identity", "-t", "git", "b"]);

    let text = stdout(&tcx(&home, &["find", "--tags", "--path", repo_s]));
    let lines: Vec<&str> = text.lines().collect();
    // Counts are right-aligned in a 4-wide column.
    assert!(lines[0].trim_start().starts_with("2 "), "got:\n{text}");
    assert!(lines[0].trim_end().ends_with("identity"), "got:\n{text}");
    assert!(lines[1].trim_start().starts_with("1 "), "got:\n{text}");
}

#[test]
fn cli_import_reads_dot_ai_by_default() {
    let tmp = TempDir::new("cli-import");
    let (home, repo) = project(&tmp);
    let ai = repo.join(".ai");
    fs::create_dir_all(&ai).unwrap();
    fs::write(ai.join("decisions.md"), "imported decision\n").unwrap();
    fs::write(ai.join("gotchas.md"), "imported gotcha\n").unwrap();

    let repo_s = repo.to_str().unwrap();
    let out = tcx(&home, &["import", "--path", repo_s]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("imported 2"), "got:\n{}", stdout(&out));

    let primed = stdout(&tcx(&home, &["prime", repo_s]));
    assert!(primed.contains("imported gotcha"));
    assert!(primed.contains("imported decision"));
    // The gotcha must sort ahead of the decision.
    assert!(primed.find("imported gotcha") < primed.find("imported decision"));
}

#[test]
fn cli_import_errors_when_dir_absent() {
    let tmp = TempDir::new("cli-import-missing");
    let (home, repo) = project(&tmp);
    let out = tcx(&home, &["import", "--path", repo.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn find_scopes_by_keyword() {
    use tcx::find::matching_keywords;
    use tcx::notes::{Kind, Note};

    let notes = vec![
        Note::new(Kind::Gotcha, "identity is provisional when shallow")
            .with_keywords(vec!["identity".to_owned()]),
        Note::new(Kind::Decision, "notes live in XDG data")
            .with_keywords(vec!["storage".to_owned()]),
        Note::new(Kind::Open, "untagged"),
    ];

    let identity = vec!["identity".to_owned()];
    let hits = matching_keywords(&notes, &identity);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].text.contains("provisional"));

    // Either keyword matches, case-insensitively.
    let either = vec!["IDENTITY".to_owned(), "storage".to_owned()];
    assert_eq!(matching_keywords(&notes, &either).len(), 2);

    // No filter matches everything; a filter that matches nothing is empty.
    assert_eq!(matching_keywords(&notes, &[]).len(), 3);
    assert_eq!(matching_keywords(&notes, &["absent".to_owned()]).len(), 0);
}
