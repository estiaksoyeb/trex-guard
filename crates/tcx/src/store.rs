//! Store location for `tcx`.
//!
//! Storage is split by the *kind* of data, not lumped into one dot-folder:
//!
//! | Data   | Location                        | Rebuildable |
//! |--------|---------------------------------|-------------|
//! | notes  | `$XDG_DATA_HOME/tcx`            | no          |
//! | index  | `$XDG_CACHE_HOME/tcx`           | yes         |
//! | config | `$XDG_CONFIG_HOME/tcx`          | no          |
//!
//! `TCX_HOME` overrides all three and points at a single root containing
//! `data/`, `cache/` and `config/`. This matters on Termux and in proot/chroot
//! setups, where `HOME` differs between environments and would otherwise
//! produce two silently separate stores.
//!
//! These paths must never collide with `~/.config/trex-guard`, which belongs to
//! the frozen `tg` guard.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The three store roots for the current environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub config: PathBuf,
}

impl Store {
    /// Resolves store paths from the environment.
    ///
    /// Precedence: `TCX_HOME` (single root), else the XDG variables, else
    /// `~/.local/share`, `~/.cache`, `~/.config`.
    pub fn from_env() -> Result<Self, Error> {
        if let Some(home) = env_nonempty("TCX_HOME") {
            let home = PathBuf::from(home);
            return Ok(Store {
                data: home.join("data"),
                cache: home.join("cache"),
                config: home.join("config"),
            });
        }

        let xdg_data = env_nonempty("XDG_DATA_HOME");
        let xdg_cache = env_nonempty("XDG_CACHE_HOME");
        let xdg_config = env_nonempty("XDG_CONFIG_HOME");

        // On Termux and similar the XDG variables are commonly unset, so the
        // HOME fallback is the normal path, not an edge case.
        let needs_home = xdg_data.is_none() || xdg_cache.is_none() || xdg_config.is_none();
        let home = if needs_home {
            Some(env_nonempty("HOME").ok_or(Error::NoHome)?)
        } else {
            None
        };

        let data = match xdg_data {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(home.as_deref().expect("checked above")).join(".local/share"),
        };
        let cache = match xdg_cache {
            Some(c) => PathBuf::from(c),
            None => PathBuf::from(home.as_deref().expect("checked above")).join(".cache"),
        };
        let config = match xdg_config {
            Some(c) => PathBuf::from(c),
            None => PathBuf::from(home.as_deref().expect("checked above")).join(".config"),
        };

        Ok(Store {
            data: data.join("tcx"),
            cache: cache.join("tcx"),
            config: config.join("tcx"),
        })
    }

    /// Directory holding a project's durable notes.
    pub fn project_data(&self, id: &str) -> PathBuf {
        self.data.join("projects").join(id)
    }

    /// Directory holding a project's rebuildable index.
    pub fn project_cache(&self, id: &str) -> PathBuf {
        self.cache.join("projects").join(id)
    }

    /// Whether a store directory exists for `id`.
    pub fn has_project(&self, id: &str) -> bool {
        self.project_data(id).is_dir()
    }

    /// Moves a provisional store into its real id.
    ///
    /// `rename` is atomic on one filesystem. Both directories live under the
    /// same data root, so this is a same-filesystem move unless the user has
    /// mounted `projects/` oddly.
    pub fn rename_project(&self, from: &str, to: &str) -> io::Result<()> {
        let src = self.project_data(from);
        let dst = self.project_data(to);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(src, dst)
    }

    /// Creates the store directory for `id` if missing.
    pub fn ensure_project(&self, id: &str) -> io::Result<PathBuf> {
        let dir = self.project_data(id);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Finds which of `candidates` already has a store, in order.
    ///
    /// Used when a repository has several roots: if a store already exists for
    /// any of them, that one wins so identity does not flip when the checked
    /// out branch changes.
    pub fn existing_project<'a>(&self, candidates: &'a [String]) -> Option<&'a String> {
        candidates.iter().find(|id| self.has_project(id))
    }

    /// Whether `path` lives under this store's data root.
    pub fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.data)
    }
}

/// Errors from store resolution.
#[derive(Debug)]
pub enum Error {
    /// Neither `TCX_HOME` nor `HOME` is set.
    NoHome,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoHome => write!(
                f,
                "cannot locate a store: set TCX_HOME or HOME \
                 (XDG variables alone are not enough if any are unset)"
            ),
        }
    }
}

impl std::error::Error for Error {}

fn env_nonempty(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}
