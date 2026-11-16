//! End-to-end tests for the `tcx` binary.
//!
//! These spawn the real executable so the CLI surface (output shape and exit
//! codes) is covered, not just the library.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("tcx-cli-{}-{}-{}", std::process::id(), label, n));
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
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Runs `tcx` with an isolated `TCX_HOME`.
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

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn id_prints_opaque_id_first() {
    let tmp = TempDir::new("first-field");
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "i"]);

    let out = tcx(&home, &["id", repo.to_str().unwrap()]);
    assert!(out.status.success());
    let line = stdout(&out);
    let id = line.split(' ').next().unwrap();
    assert_eq!(id.len(), 40, "should be a full object hash: {line}");
    assert!(line.contains("git\t"), "kind is the second field: {line}");
}

#[test]
fn human_output_has_stable_labels() {
    let tmp = TempDir::new("human");
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "i"]);

    let out = tcx(&home, &["id", "--human", repo.to_str().unwrap()]);
    let text = stdout(&out);
    for label in ["id:", "kind:", "root:", "common-dir:", "roots:", "store:", "provisional:"] {
        assert!(text.contains(label), "missing {label} in:\n{text}");
    }
    assert!(text.contains("provisional: false"));
}

#[test]
fn empty_repo_reports_provisional_then_adopts() {
    let tmp = TempDir::new("adopt");
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);

    let before = tcx(&home, &["id", repo.to_str().unwrap()]);
    let provisional = stdout(&before).split(' ').next().unwrap().to_string();
    assert!(provisional.starts_with("p-"), "got {provisional}");

    // Create the provisional store with a note in it.
    let dir = home.join("data/projects").join(&provisional);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("notes.md"), "keep me\n").unwrap();

    git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);

    let after = tcx(&home, &["id", "--human", repo.to_str().unwrap()]);
    let text = stdout(&after);
    assert!(text.contains("adopted:     true"), "got:\n{text}");
    assert!(
        stderr(&after).contains("adopted provisional store"),
        "stderr should explain the move: {}",
        stderr(&after)
    );

    // The note survived and the provisional directory is gone.
    let projects = home.join("data/projects");
    let names: Vec<String> = fs::read_dir(&projects)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "only the real store should remain: {names:?}");
    assert!(!names[0].starts_with("p-"));
    let note = fs::read_to_string(projects.join(&names[0]).join("notes.md")).unwrap();
    assert_eq!(note, "keep me\n");
}

#[test]
fn exit_codes_are_stable() {
    let tmp = TempDir::new("codes");
    let home = tmp.path().join("home");

    assert_eq!(tcx(&home, &[]).status.code(), Some(2), "bare tcx is a usage error");
    assert_eq!(tcx(&home, &["--help"]).status.code(), Some(0));
    assert_eq!(tcx(&home, &["--version"]).status.code(), Some(0));
    assert_eq!(tcx(&home, &["bogus"]).status.code(), Some(2));
    assert_eq!(tcx(&home, &["id", "--nope"]).status.code(), Some(2));
    assert_eq!(
        tcx(&home, &["id", "/definitely/not/here"]).status.code(),
        Some(1),
        "missing path is an error, not a usage error"
    );
}

#[test]
fn version_matches_package_version() {
    let tmp = TempDir::new("version");
    let out = tcx(tmp.path(), &["--version"]);
    let text = stdout(&out);
    assert!(text.starts_with("tcx "), "got {text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn no_home_is_a_clear_error() {
    let tmp = TempDir::new("nohome");
    let out = Command::new(env!("CARGO_BIN_EXE_tcx"))
        .args(["id"])
        .env_remove("TCX_HOME")
        .env_remove("HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .current_dir(tmp.path())
        .output()
        .expect("run tcx");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("TCX_HOME"), "got {}", stderr(&out));
}
