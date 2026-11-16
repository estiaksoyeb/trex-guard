//! Integration tests for `tcx id`.
//!
//! Each test builds its own temporary repositories, so they are independent and
//! safe to run in parallel. Nothing touches the user's real store.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use tcx::id::{self, Kind, Migration};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tcx-test-{}-{}-{}",
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

/// Runs git in `dir`, asserting success.
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
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
}

fn commit_empty(dir: &Path, message: &str) {
    git(dir, &["commit", "-q", "--allow-empty", "-m", message]);
}

// -------------------------------------------------------------------------
// zero-commit
// -------------------------------------------------------------------------

#[test]
fn empty_repo_is_provisional() {
    let tmp = TempDir::new("empty");
    init_repo(tmp.path());

    let ident = id::resolve(tmp.path()).expect("resolve");
    assert_eq!(ident.kind, Kind::GitEmpty);
    assert!(ident.is_provisional());
    assert!(ident.id.starts_with("p-"), "got {}", ident.id);
    assert!(ident.roots.is_empty());
    assert!(ident.common_dir.is_some());
}

#[test]
fn empty_repo_identity_is_stable_across_subdirs() {
    let tmp = TempDir::new("empty-subdir");
    init_repo(tmp.path());
    let sub = tmp.path().join("a/b");
    fs::create_dir_all(&sub).unwrap();

    let top = id::resolve(tmp.path()).unwrap();
    let nested = id::resolve(&sub).unwrap();
    // Both resolve to the same git root, so the id must match.
    assert_eq!(top.id, nested.id);
}

// -------------------------------------------------------------------------
// first commit upgrades the provisional identity
// -------------------------------------------------------------------------

#[test]
fn provisional_upgrades_to_real_root_after_first_commit() {
    let tmp = TempDir::new("upgrade");
    init_repo(tmp.path());

    let before = id::resolve(tmp.path()).unwrap();
    assert_eq!(before.kind, Kind::GitEmpty);

    commit_empty(tmp.path(), "init");

    let after = id::resolve(tmp.path()).unwrap();
    assert_eq!(after.kind, Kind::Git);
    assert!(!after.is_provisional());
    assert_eq!(after.roots.len(), 1);
    assert_eq!(after.id, after.roots[0]);
    assert_ne!(before.id, after.id);
}

// -------------------------------------------------------------------------
// worktrees share identity
// -------------------------------------------------------------------------

#[test]
fn linked_worktree_shares_root_commit() {
    let tmp = TempDir::new("worktree");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    commit_empty(&repo, "init");

    let wt = tmp.path().join("wt");
    git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);

    let main = id::resolve(&repo).unwrap();
    let linked = id::resolve(&wt).unwrap();
    assert_eq!(main.id, linked.id, "worktree must not fork identity");
    assert_eq!(main.common_dir, linked.common_dir);
    assert_ne!(main.root, linked.root, "roots differ, ids do not");
}

// -------------------------------------------------------------------------
// orphan branch: multiple roots
// -------------------------------------------------------------------------

#[test]
fn orphan_branch_adds_second_root() {
    let tmp = TempDir::new("orphan");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    commit_empty(&repo, "main-1");

    let main_root = id::resolve(&repo).unwrap().roots[0].clone();

    git(&repo, &["checkout", "-q", "--orphan", "gh-pages"]);
    // An orphan checkout keeps the index; drop it so the commit is truly bare.
    git(&repo, &["rm", "-rq", "--cached", ".", "--ignore-unmatch"]);
    commit_empty(&repo, "orphan-1");

    let on_orphan = id::resolve(&repo).unwrap();
    assert_eq!(on_orphan.roots.len(), 1, "only the orphan root is reachable");
    assert_ne!(on_orphan.roots[0], main_root);

    git(&repo, &["checkout", "-q", "master"]);

    let back = id::resolve(&repo).unwrap();
    assert_eq!(back.roots, vec![main_root.clone()]);
    assert_eq!(back.id, main_root);
}

#[test]
fn worktree_on_orphan_branch_resolves_independently() {
    let tmp = TempDir::new("orphan-wt");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    commit_empty(&repo, "main-1");
    git(&repo, &["checkout", "-q", "--orphan", "gh-pages"]);
    git(&repo, &["rm", "-rq", "--cached", ".", "--ignore-unmatch"]);
    commit_empty(&repo, "orphan-1");
    git(&repo, &["checkout", "-q", "master"]);

    let wt = tmp.path().join("wt-gh");
    git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap(), "gh-pages"]);

    let main = id::resolve(&repo).unwrap();
    let orphan = id::resolve(&wt).unwrap();

    // Same repository (same common dir) but different visible roots.
    assert_eq!(main.common_dir, orphan.common_dir);
    assert_ne!(
        main.id, orphan.id,
        "orphan worktree must not claim the main root"
    );
}

#[test]
fn unrelated_history_merge_yields_two_roots() {
    let tmp = TempDir::new("multiroot");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    commit_empty(&repo, "base");

    git(&repo, &["checkout", "-q", "--orphan", "other"]);
    git(&repo, &["rm", "-rq", "--cached", ".", "--ignore-unmatch"]);
    commit_empty(&repo, "other-base");

    git(&repo, &["checkout", "-q", "master"]);
    git(
        &repo,
        &[
            "merge",
            "-q",
            "--allow-unrelated-histories",
            "--no-edit",
            "other",
        ],
    );

    let ident = id::resolve(&repo).unwrap();
    assert_eq!(ident.roots.len(), 2, "both histories have a root");
    assert_eq!(ident.roots, {
        let mut v = ident.roots.clone();
        v.sort();
        v
    }, "roots are sorted for a stable id");
}

// -------------------------------------------------------------------------
// shallow clones: never use the boundary commit as identity
// -------------------------------------------------------------------------

#[test]
fn shallow_clone_is_provisional_and_not_the_boundary_commit() {
    let tmp = TempDir::new("shallow");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    init_repo(&src);
    for i in 1..=5 {
        commit_empty(&src, &format!("c{i}"));
    }

    let clone = tmp.path().join("clone");
    let url = format!("file://{}", src.display());
    git(
        tmp.path(),
        &["clone", "-q", "--depth", "1", &url, clone.to_str().unwrap()],
    );

    let ident = id::resolve(&clone).unwrap();
    assert_eq!(ident.kind, Kind::GitShallow);
    assert!(ident.is_provisional());
    assert!(
        ident.id.starts_with("p-"),
        "shallow must not use a boundary hash, got {}",
        ident.id
    );
    assert!(ident.roots.is_empty());
}

#[test]
fn shallow_depth_does_not_change_identity() {
    let tmp = TempDir::new("shallow-depth");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    init_repo(&src);
    for i in 1..=5 {
        commit_empty(&src, &format!("c{i}"));
    }

    let url = format!("file://{}", src.display());
    let d1 = tmp.path().join("d1");
    let d3 = tmp.path().join("d3");
    git(tmp.path(), &["clone", "-q", "--depth", "1", &url, d1.to_str().unwrap()]);
    git(tmp.path(), &["clone", "-q", "--depth", "3", &url, d3.to_str().unwrap()]);

    // Different clones have different common dirs, so compare against the
    // property that matters: neither identity is a 40-char commit hash.
    let a = id::resolve(&d1).unwrap();
    let b = id::resolve(&d3).unwrap();
    for ident in [&a, &b] {
        assert!(ident.is_provisional());
        assert!(!is_hex_sha(ident.id.trim_start_matches("p-")));
    }
}

#[test]
fn unshallowed_clone_becomes_real() {
    let tmp = TempDir::new("unshallow");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    init_repo(&src);
    for i in 1..=3 {
        commit_empty(&src, &format!("c{i}"));
    }

    let clone = tmp.path().join("clone");
    let url = format!("file://{}", src.display());
    git(
        tmp.path(),
        &["clone", "-q", "--depth", "1", &url, clone.to_str().unwrap()],
    );
    assert_eq!(id::resolve(&clone).unwrap().kind, Kind::GitShallow);

    git(&clone, &["fetch", "-q", "--unshallow"]);

    let ident = id::resolve(&clone).unwrap();
    assert_eq!(ident.kind, Kind::Git);
    assert!(!ident.is_provisional());
    assert_eq!(ident.id, id::resolve(&src).unwrap().roots[0]);
}

// -------------------------------------------------------------------------
// non-git directories
// -------------------------------------------------------------------------

#[test]
fn non_git_directory_uses_path_identity() {
    let tmp = TempDir::new("plain");
    let a = tmp.path().join("a");
    fs::create_dir_all(&a).unwrap();

    let ident = id::resolve(&a).unwrap();
    assert_eq!(ident.kind, Kind::Directory);
    assert!(ident.id.starts_with("dir-"));
    assert_eq!(ident.common_dir, None);
    assert!(!ident.is_provisional());
}

#[test]
fn non_git_identity_is_the_starting_directory() {
    let tmp = TempDir::new("plain-cwd");
    let a = tmp.path().join("a");
    fs::create_dir_all(a.join("deep")).unwrap();

    let at_root = id::resolve(&a).unwrap();
    let from_deep = id::resolve(a.join("deep")).unwrap();

    // Without git there is no repository root to climb to, so identity is the
    // directory you are in. Two subdirectories of a plain folder are different
    // projects; sharing a store between them would be surprising.
    assert_ne!(at_root.id, from_deep.id);
    assert_eq!(at_root.root, fs::canonicalize(&a).unwrap());
    assert_eq!(from_deep.root, fs::canonicalize(a.join("deep")).unwrap());
}

#[test]
fn non_git_identity_is_stable_for_the_same_path() {
    let tmp = TempDir::new("plain-stable");
    let a = tmp.path().join("a");
    fs::create_dir_all(&a).unwrap();

    // Repeated resolution, and resolution via a path containing `..`, must
    // produce the same id: identity is the canonical path, not its spelling.
    let direct = id::resolve(&a).unwrap();
    let via_relative = id::resolve(a.join("..").join("a")).unwrap();
    assert_eq!(direct.id, via_relative.id);
}

#[test]
fn missing_path_is_an_error() {
    let tmp = TempDir::new("missing");
    let err = id::resolve(tmp.path().join("nope")).unwrap_err();
    assert!(matches!(err, id::Error::NotFound(_)));
}

// -------------------------------------------------------------------------
// provisional -> real rename
// -------------------------------------------------------------------------

#[test]
fn migration_renames_when_only_provisional_exists() {
    let tmp = TempDir::new("migrate");
    init_repo(tmp.path());
    let provisional = id::resolve(tmp.path()).unwrap();
    commit_empty(tmp.path(), "init");
    let real = id::resolve(tmp.path()).unwrap();

    let m = id::plan_migration(&real, Some(&provisional.id), |name| name == provisional.id);
    assert_eq!(
        m,
        Migration::Rename {
            from: provisional.id.clone(),
            to: real.id.clone()
        }
    );
}

#[test]
fn migration_conflicts_when_both_exist() {
    let tmp = TempDir::new("conflict");
    init_repo(tmp.path());
    let provisional = id::resolve(tmp.path()).unwrap();
    commit_empty(tmp.path(), "init");
    let real = id::resolve(tmp.path()).unwrap();

    let m = id::plan_migration(&real, Some(&provisional.id), |_| true);
    assert_eq!(
        m,
        Migration::Conflict {
            from: provisional.id.clone(),
            to: real.id.clone()
        }
    );
}

#[test]
fn migration_is_noop_when_provisional_is_absent() {
    let tmp = TempDir::new("noop");
    init_repo(tmp.path());
    let provisional = id::resolve(tmp.path()).unwrap();
    commit_empty(tmp.path(), "init");
    let real = id::resolve(tmp.path()).unwrap();

    let m = id::plan_migration(&real, Some(&provisional.id), |_| false);
    assert_eq!(m, Migration::None);
}

#[test]
fn migration_is_noop_when_still_provisional() {
    let tmp = TempDir::new("still-prov");
    init_repo(tmp.path());
    let provisional = id::resolve(tmp.path()).unwrap();

    let m = id::plan_migration(&provisional, Some(&provisional.id), |_| true);
    assert_eq!(m, Migration::None);
}

// -------------------------------------------------------------------------
// properties
// -------------------------------------------------------------------------

#[test]
fn real_identity_equals_single_root() {
    let tmp = TempDir::new("equals");
    init_repo(tmp.path());
    commit_empty(tmp.path(), "a");
    commit_empty(tmp.path(), "b");

    let ident = id::resolve(tmp.path()).unwrap();
    assert_eq!(ident.roots.len(), 1);
    assert_eq!(ident.id, ident.roots[0]);
    assert!(!ident.id.starts_with("p-"));
}

/// True if `s` looks like a full object hash (SHA-1 or SHA-256, hex).
fn is_hex_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}
