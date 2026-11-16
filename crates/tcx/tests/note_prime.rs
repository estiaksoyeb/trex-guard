//! End-to-end tests for `tcx note` and `tcx prime`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tcx-np-{}-{}-{}",
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
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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

/// A repo with one commit, ready to use as a project.
fn project(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    (home, repo)
}

// -------------------------------------------------------------------------
// note
// -------------------------------------------------------------------------

#[test]
fn note_appends_and_lists() {
    let tmp = TempDir::new("note-list");
    let (home, repo) = project(&tmp);
    let repo = repo.to_str().unwrap();

    let out = tcx(&home, &["note", "--path", repo, "first decision"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("| decision | first decision"));

    let out = tcx(
        &home,
        &[
            "note",
            "--path", repo,
            "-k", "gotcha",
            "-t", "procmtime",
            "-f", "src/a.rs",
            "mtime lies",
        ],
    );
    assert!(out.status.success());

    let listed = stdout(&tcx(&home, &["note", "--list", "--path", repo]));
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 2, "got:\n{listed}");
    assert!(lines[0].contains("decision"));
    assert!(lines[1].contains("gotcha"));
    assert!(lines[1].contains("files: src/a.rs"));
    assert!(lines[1].contains("[procmtime]"));
}

#[test]
fn note_rejects_empty_text() {
    let tmp = TempDir::new("note-empty");
    let (home, repo) = project(&tmp);
    let out = tcx(&home, &["note", "--path", repo.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn note_list_on_fresh_project_is_empty() {
    let tmp = TempDir::new("note-fresh");
    let (home, repo) = project(&tmp);
    let out = tcx(&home, &["note", "--list", "--path", repo.to_str().unwrap()]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "");
}

#[test]
fn note_unknown_kind_is_preserved() {
    let tmp = TempDir::new("note-kind");
    let (home, repo) = project(&tmp);
    let repo = repo.to_str().unwrap();
    tcx(&home, &["note", "--path", repo, "-k", "hypothesis", "maybe"]);
    let listed = stdout(&tcx(&home, &["note", "--list", "--path", repo]));
    assert!(listed.contains("| hypothesis |"), "got:\n{listed}");
}

// -------------------------------------------------------------------------
// prime
// -------------------------------------------------------------------------

#[test]
fn prime_on_fresh_project_has_stamp_and_no_notes() {
    let tmp = TempDir::new("prime-fresh");
    let (home, repo) = project(&tmp);

    let out = tcx(&home, &["prime", repo.to_str().unwrap()]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.starts_with("# tcx brief\n"), "got:\n{text}");
    assert!(text.contains("HEAD "), "stamp head missing:\n{text}");
    assert!(text.contains("tcx"), "stamp marker missing:\n{text}");
    assert!(text.contains("No notes yet"), "got:\n{text}");
}

#[test]
fn prime_orders_gotchas_first() {
    let tmp = TempDir::new("prime-order");
    let (home, repo) = project(&tmp);
    let repo = repo.to_str().unwrap();

    tcx(&home, &["note", "--path", repo, "-k", "decision", "a decision"]);
    tcx(&home, &["note", "--path", repo, "-k", "gotcha", "a gotcha"]);

    let text = stdout(&tcx(&home, &["prime", repo]));
    let g = text.find("a gotcha").unwrap();
    let d = text.find("a decision").unwrap();
    assert!(g < d, "gotcha should come first:\n{text}");
}

#[test]
fn prime_respects_budget() {
    let tmp = TempDir::new("prime-budget");
    let (home, repo) = project(&tmp);
    let repo = repo.to_str().unwrap();
    for i in 0..40 {
        tcx(&home, &["note", "--path", repo, &format!("decision number {i}")]);
    }

    let text = stdout(&tcx(&home, &["prime", "--budget", "150", repo]));
    assert!(text.contains("more (tcx find)"), "got:\n{text}");
}

#[test]
fn prime_rejects_bad_budget() {
    let tmp = TempDir::new("prime-badbudget");
    let (home, repo) = project(&tmp);
    let repo = repo.to_str().unwrap();
    assert_eq!(tcx(&home, &["prime", "--budget", "zero", repo]).status.code(), Some(2));
    assert_eq!(tcx(&home, &["prime", "--budget", "0", repo]).status.code(), Some(2));
    assert_eq!(tcx(&home, &["prime", "--budget"]).status.code(), Some(2));
}

#[test]
fn prime_reports_dirty_count() {
    let tmp = TempDir::new("prime-dirty");
    let (home, repo) = project(&tmp);
    fs::write(repo.join("new-file.txt"), "x").unwrap();

    let text = stdout(&tcx(&home, &["prime", repo.to_str().unwrap()]));
    assert!(text.contains("dirty 1"), "expected one dirty file:\n{text}");
    // Names must not leak into the stamp.
    assert!(!text.contains("new-file.txt"), "paths must not appear:\n{text}");
}

#[test]
fn prime_on_empty_repo_stamps_none() {
    let tmp = TempDir::new("prime-empty");
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);

    let text = stdout(&tcx(&home, &["prime", repo.to_str().unwrap()]));
    assert!(text.contains("HEAD none"), "got:\n{text}");
}

#[test]
fn prime_notes_are_shared_across_worktrees() {
    let tmp = TempDir::new("prime-wt");
    let (home, repo) = project(&tmp);
    let wt = tmp.path().join("wt");
    git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);

    tcx(&home, &["note", "--path", repo.to_str().unwrap(), "shared note"]);
    let text = stdout(&tcx(&home, &["prime", wt.to_str().unwrap()]));
    assert!(text.contains("shared note"), "worktree must see the note:\n{text}");
}

#[test]
fn prime_is_deterministic_across_runs() {
    let tmp = TempDir::new("prime-det");
    let (home, repo) = project(&tmp);
    let repo = repo.to_str().unwrap();
    tcx(&home, &["note", "--path", repo, "one"]);
    tcx(&home, &["note", "--path", repo, "-k", "gotcha", "two"]);

    let first = stdout(&tcx(&home, &["prime", repo]));
    for _ in 0..5 {
        assert_eq!(first, stdout(&tcx(&home, &["prime", repo])));
    }
}
