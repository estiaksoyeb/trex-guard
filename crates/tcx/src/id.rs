//! Project identity resolution for `tcx`.
//!
//! Identity answers one question: **which store does this project own?**
//!
//! The key is derived from the repository, never from the working-tree path, so
//! that worktrees, clones and moves all resolve to the same store.
//!
//! # Resolution strategy
//!
//! For a git repository we compute the set of *root commits* reachable from
//! `HEAD` (`git rev-list --max-parents=0 HEAD`). A repository may have several
//! roots, e.g. an orphan branch such as `gh-pages`, or an unrelated-history
//! merge. Identity therefore uses **the set**, sorted, not a single hash.
//!
//! Two situations yield a *provisional* identity instead of a real one:
//!
//! * **No commits yet.** There is no root commit to name the store after.
//!   `rev-parse HEAD` fails on a fresh `git init`.
//! * **Shallow clone.** `rev-list --max-parents=0 HEAD` returns the *boundary*
//!   commit, which looks parentless. That hash changes with `--depth`, on
//!   `--deepen` and on `--unshallow`, so using it as identity would fork the
//!   store every time the clone's depth changed. It must never be treated as a
//!   real root.
//!
//! Provisional stores are named `p-<hash>` where `<hash>` is taken from the
//! canonical `git-common-dir` path, which is stable across worktrees and
//! subdirectories. When a real root later becomes available, the provisional
//! store is renamed into place (see [`plan_migration`]).
//!
//! The identity string is treated as **opaque** everywhere. Nothing parses it,
//! so the repository's object format (SHA-1 or SHA-256) does not matter.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How a project's identity was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A git repository with one or more root commits reachable from `HEAD`.
    Git,
    /// A git repository that is shallow: the visible "root" is only a boundary.
    GitShallow,
    /// A git repository with no commits yet.
    GitEmpty,
    /// Not a git repository: identity is the canonical root path.
    Directory,
}

impl Kind {
    /// Whether the identity can change later without the project moving.
    ///
    /// Provisional identities (shallow and empty repos) are expected to be
    /// replaced by a real one once history becomes available.
    pub fn is_provisional(self) -> bool {
        matches!(self, Kind::GitShallow | Kind::GitEmpty)
    }
}

/// A resolved project identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Opaque store name. See the module docs: never parse this.
    pub id: String,
    pub kind: Kind,
    /// Canonical path of the worktree root (the top level for git, else the dir).
    pub root: PathBuf,
    /// Canonical `git-common-dir`, shared by every worktree. `None` if not git.
    pub common_dir: Option<PathBuf>,
    /// Sorted root commits, one per independent history. Empty when provisional.
    pub roots: Vec<String>,
}

impl Identity {
    /// Whether this identity is expected to be replaced later.
    pub fn is_provisional(&self) -> bool {
        self.kind.is_provisional()
    }

    /// One-line description used by the `prime` stamp.
    ///
    /// Deliberately excludes filesystem paths: the stamp is pasted into chats.
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            Kind::Git => "git",
            Kind::GitShallow => "git shallow",
            Kind::GitEmpty => "git empty",
            Kind::Directory => "directory",
        }
    }
}

/// Errors from identity resolution.
#[derive(Debug)]
pub enum Error {
    /// The starting path does not exist.
    NotFound(PathBuf),
    /// A `git` invocation failed in a way we did not expect.
    Git {
        args: String,
        stderr: String,
    },
    /// Filesystem access failed while canonicalising.
    Io(PathBuf, std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(p) => write!(f, "path does not exist: {}", p.display()),
            Error::Git { args, stderr } => {
                write!(f, "git {args} failed: {}", stderr.trim())
            }
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

/// Resolves the identity of the project containing `start`.
pub fn resolve(start: impl AsRef<Path>) -> Result<Identity, Error> {
    let start = start.as_ref();
    let canonical = fs::canonicalize(start).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound(start.to_path_buf())
        } else {
            Error::Io(start.to_path_buf(), e)
        }
    })?;

    match git_toplevel(&canonical)? {
        Some(root) => resolve_git(root, &canonical),
        None => Ok(resolve_directory(canonical)),
    }
}

/// Identity for a non-git directory: the canonical root path itself.
fn resolve_directory(root: PathBuf) -> Identity {
    let id = format!("dir-{}", fnv1a64(root.as_os_str()));
    Identity {
        id,
        kind: Kind::Directory,
        root,
        common_dir: None,
        roots: Vec::new(),
    }
}

fn resolve_git(root: PathBuf, _start: &Path) -> Result<Identity, Error> {
    // `--git-common-dir` is relative in the main worktree but absolute in a
    // linked one, so always ask for it and then resolve against the root.
    let common_raw = git_in(&root, &["rev-parse", "--git-common-dir"])?;
    let common_rel = PathBuf::from(common_raw.trim());
    let common_dir = fs::canonicalize(root.join(&common_rel))
        .map_err(|e| Error::Io(root.join(&common_rel), e))?;

    if git_in(&root, &["rev-parse", "--is-shallow-repository"])?.trim() == "true" {
        return Ok(provisional(root, common_dir, Kind::GitShallow));
    }

    // No commits yet: `rev-parse --verify HEAD` fails rather than printing.
    if git_status(&root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() {
        return Ok(provisional(root, common_dir, Kind::GitEmpty));
    }

    let out = git_in(&root, &["rev-list", "--max-parents=0", "HEAD"])?;
    let roots: BTreeSet<String> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();

    if roots.is_empty() {
        // HEAD exists but no parentless commit is reachable: treat as empty
        // rather than inventing an identity we cannot defend.
        return Ok(provisional(root, common_dir, Kind::GitEmpty));
    }

    let roots: Vec<String> = roots.into_iter().collect();
    Ok(Identity {
        // Sorted first root keeps the id stable; ties are resolved at
        // store-creation time (see `plan_migration` / `existing_store`).
        id: roots[0].clone(),
        kind: Kind::Git,
        root,
        common_dir: Some(common_dir),
        roots,
    })
}

/// Provisional identity, named after the canonical common dir.
fn provisional(root: PathBuf, common_dir: PathBuf, kind: Kind) -> Identity {
    Identity {
        id: format!("p-{}", fnv1a64(common_dir.as_os_str())),
        kind,
        root,
        common_dir: Some(common_dir),
        roots: Vec::new(),
    }
}

/// Picks the store id for an identity that may have several roots.
///
/// A repository can have more than one root commit (an orphan branch, an
/// unrelated-history merge). Which root `HEAD` exposes depends on the checked
/// out branch, so using `roots[0]` blindly would make the store change when the
/// user switches branches.
///
/// Instead: if any root already has a store, that root wins. If several do, the
/// earliest in sorted order wins and the caller should warn. Otherwise the first
/// sorted root is used, which keeps new stores stable.
pub fn choose_root<'a>(identity: &'a Identity, exists: impl Fn(&str) -> bool) -> (Choice<'a>, Vec<&'a String>) {
    let matching: Vec<&String> = identity.roots.iter().filter(|r| exists(r)).collect();
    match matching.len() {
        0 => (Choice::New(&identity.roots[0]), Vec::new()),
        1 => (Choice::Existing(matching[0]), Vec::new()),
        _ => (Choice::Existing(matching[0]), matching),
    }
}

/// Result of [`choose_root`].
#[derive(Debug, PartialEq, Eq)]
pub enum Choice<'a> {
    /// No store exists yet; create one under this root.
    New(&'a String),
    /// A store already exists for this root; use it.
    Existing(&'a String),
}

/// What the `prime` stamp should say about a project's git state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadState {
    /// Abbreviated commit hash, or `None` for an empty repository.
    pub head: Option<String>,
    /// Branch name, or `detached` when HEAD is not on a branch.
    pub branch: String,
    /// Count of entries in `git status --porcelain`.
    pub dirty: usize,
}

/// Reads the current `HEAD` state for the `prime` stamp.
///
/// Returns `None` for non-git projects. Counts dirty entries rather than
/// listing them: the stamp is pasted into chats.
pub fn head_state(identity: &Identity) -> Option<HeadState> {
    if identity.common_dir.is_none() {
        return None;
    }
    let root = &identity.root;

    let head = git_status(root, &["rev-parse", "--short", "--verify", "--quiet", "HEAD"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let branch = match git_status(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Ok(s) if !s.trim().is_empty() => s.trim().to_owned(),
        // An empty repo and a detached head both fail here; tell them apart.
        _ => {
            if head.is_some() {
                "detached".to_owned()
            } else {
                "no-branch".to_owned()
            }
        }
    };

    let dirty = git_status(root, &["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);

    Some(HeadState {
        head,
        branch,
        dirty,
    })
}

/// Computes what the provisional id *would be* for this identity's repository.
pub fn provisional_id_for(identity: &Identity) -> Option<String> {
    identity
        .common_dir
        .as_ref()
        .map(|dir| format!("p-{}", fnv1a64(dir.as_os_str())))
}

/// What to do about an existing provisional store when a real root is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// No provisional store exists; nothing to do.
    None,
    /// Rename the provisional store to the real id.
    Rename { from: String, to: String },
    /// Both stores exist: refuse to merge, keep both, tell the user.
    Conflict { from: String, to: String },
}

/// Decides whether a provisional store should be adopted into the real id.
///
/// `exists` is asked about store names. The rename is only proposed when the
/// real store does not exist yet; otherwise the two are kept separate because
/// deciding which notes win is a human call (`--adopt`, deferred).
pub fn plan_migration(identity: &Identity, provisional_id: Option<&str>, exists: impl Fn(&str) -> bool) -> Migration {
    let Some(from) = provisional_id else {
        return Migration::None;
    };
    if identity.kind.is_provisional() || from == identity.id {
        return Migration::None;
    }
    if !exists(from) {
        return Migration::None;
    }
    if exists(&identity.id) {
        return Migration::Conflict {
            from: from.to_owned(),
            to: identity.id.clone(),
        };
    }
    Migration::Rename {
        from: from.to_owned(),
        to: identity.id.clone(),
    }
}

// ---------------------------------------------------------------------------
// git helpers
// ---------------------------------------------------------------------------

fn git_toplevel(start: &Path) -> Result<Option<PathBuf>, Error> {
    match git_status(start, &["rev-parse", "--show-toplevel"]) {
        Ok(out) => {
            let top = out.trim_end_matches(['\n', '\r']);
            if top.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(top)))
            }
        }
        Err(_) => Ok(None),
    }
}

fn git_in(dir: &Path, args: &[&str]) -> Result<String, Error> {
    run_git(dir, args).map_err(|stderr| Error::Git {
        args: args.join(" "),
        stderr,
    })
}

fn git_status(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_git(dir, args)
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// FNV-1a over the platform bytes of a path.
///
/// A plain hasher keeps the id short and dependency-free. Collisions here only
/// merge two *provisional* stores, and the migration step refuses to merge when
/// a real id already exists, so the blast radius is small.
fn fnv1a64(s: &OsStr) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}
