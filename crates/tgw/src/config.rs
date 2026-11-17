//! `watch.ini`: the watchdog's own configuration.
//!
//! Read from `<config>/watch.ini` using the same lookup as `tcx`
//! ([`crate::config::config_dir`]): `$TCX_HOME/config/`, else
//! `$XDG_CONFIG_HOME/tcx/`, else `~/.config/tcx/`. It is **never** read from
//! the current directory or the repository, because a repository can change
//! under the human's feet.
//!
//! Format is deliberately dull: `key = value`, `#`/`;` comments, `[section]`
//! headers ignored. No parser dependency.
//!
//! # Arguments are split, never shelled
//!
//! Commands are **argv strings**, split by [`split_argv`] and executed with no
//! shell involved. That means a value cannot inject a pipeline by accident,
//! and it also means quoting is the config author's job:
//!
//! ```ini
//! notify_cmd = notify-send "Block approved"
//! ```
//!
//! splits into `notify-send` and `Block approved` — two args, not three.
//!
//! # Fail closed
//!
//! [`Config::validate`] returns every problem at once rather than the first.
//! A watchdog that starts with half a config is a watchdog that gates with
//! half a policy, so `tgw` refuses to run until the problems are fixed.
//! `tgw check-config` exists to show them without starting anything.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What a passing block does.
///
/// `dry` is the default, and deliberately the safest: it logs and notifies but
/// never executes. A fresh install cannot run anything by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Log and notify only. Never executes.
    #[default]
    Dry,
    /// Every block that passes the gate waits for approval.
    Confirm,
    /// A gate exit 0 runs without approval. Holds still wait.
    Auto,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Dry => "dry",
            Mode::Confirm => "confirm",
            Mode::Auto => "auto",
        }
    }

    /// Parses a mode name. Unknown names are an error, never a fallback: a
    /// typo in `mode = confrim` must not silently become `dry`, because the
    /// human would then believe blocks are running when they are not.
    pub fn parse(s: &str) -> Result<Mode, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dry" => Ok(Mode::Dry),
            "confirm" => Ok(Mode::Confirm),
            "auto" => Ok(Mode::Auto),
            other => Err(format!("unknown mode `{other}` (use dry, confirm or auto)")),
        }
    }
}

/// Where a block runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CwdMode {
    /// The directory `tgw` was started in.
    #[default]
    Inherit,
    /// A fixed directory from `cwd`.
    Fixed,
    /// The directory named in `cwd_file`, re-read per block.
    File,
}

impl CwdMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CwdMode::Inherit => "inherit",
            CwdMode::Fixed => "fixed",
            CwdMode::File => "file",
        }
    }

    pub fn parse(s: &str) -> Result<CwdMode, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "inherit" => Ok(CwdMode::Inherit),
            "fixed" => Ok(CwdMode::Fixed),
            "file" => Ok(CwdMode::File),
            other => Err(format!(
                "unknown cwd_mode `{other}` (use inherit, fixed or file)"
            )),
        }
    }
}

/// Environment variables passed to a block when `env_allow` is not given.
const DEFAULT_ENV_ALLOW: &[&str] = &["PATH", "HOME", "TCX_HOME", "TERM", "LANG"];

/// A parsed `watch.ini`, with every key's default filled in.
///
/// `Default` is the whole-configuration default, so tests and `--help` can
/// talk about "the default config" without inventing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Command that prints the clipboard to stdout. Required.
    pub clip_get_cmd: String,
    /// Command that gates a staged file. Required. `{file}` is substituted.
    pub gate_cmd: String,
    /// Command that signals a verdict. `{id}` `{status}` `{summary}`.
    pub notify_cmd: String,
    /// Command that takes a wakelock, if the platform has one.
    pub wakelock_cmd: String,
    /// Command that releases it.
    pub wakelock_release_cmd: String,
    /// Shell used to run a block, as an argv string.
    pub shell: String,
    /// File sourced before the block, for aliases.
    pub prelude: String,
    /// Exactly which environment variables a block sees.
    pub env_allow: Vec<String>,
    pub cwd_mode: CwdMode,
    /// Directory used when `cwd_mode = fixed`.
    pub cwd: String,
    /// File containing the directory used when `cwd_mode = file`.
    pub cwd_file: String,
    /// The first non-empty line a block must have to be considered at all.
    pub trigger_first_line: String,
    pub poll_ms: u64,
    pub max_block_bytes: u64,
    pub timeout_s: u64,
    pub gate_timeout_s: u64,
    pub max_output_bytes: u64,
    pub hold_ttl_s: u64,
    pub mode: Mode,
    /// How many human approvals unlock `mode = auto`.
    pub auto_unlock_approvals: u64,
    pub state_dir: String,
    pub log_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            clip_get_cmd: String::new(),
            gate_cmd: String::new(),
            notify_cmd: String::new(),
            wakelock_cmd: String::new(),
            wakelock_release_cmd: String::new(),
            shell: "bash".into(),
            prelude: String::new(),
            env_allow: DEFAULT_ENV_ALLOW.iter().map(|s| s.to_string()).collect(),
            cwd_mode: CwdMode::Inherit,
            cwd: String::new(),
            cwd_file: String::new(),
            // The live prompt template mandates this as every block's first
            // line, so it is observed from the workflow rather than invented.
            trigger_first_line: "set +H".into(),
            poll_ms: 500,
            max_block_bytes: 65536,
            timeout_s: 120,
            gate_timeout_s: 10,
            max_output_bytes: 65536,
            hold_ttl_s: 300,
            mode: Mode::Dry,
            auto_unlock_approvals: 20,
            state_dir: String::new(),
            log_dir: String::new(),
        }
    }
}

/// Where `watch.ini` and the state/log directories live.
///
/// Deliberately the same rules as `tcx` so a human who set `TCX_HOME` for one
/// tool gets the same root for the other.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(home) = env_nonempty("TCX_HOME") {
        return Some(PathBuf::from(home).join("config"));
    }
    if let Some(xdg) = env_nonempty("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("tcx"));
    }
    env_nonempty("HOME").map(|h| PathBuf::from(h).join(".config").join("tcx"))
}

/// The default state directory: `$TCX_HOME/data/watch/`, else XDG data.
pub fn default_state_dir() -> Option<PathBuf> {
    if let Some(home) = env_nonempty("TCX_HOME") {
        return Some(PathBuf::from(home).join("data").join("watch"));
    }
    if let Some(xdg) = env_nonempty("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg).join("tcx").join("watch"));
    }
    env_nonempty("HOME").map(|h| {
        PathBuf::from(h)
            .join(".local")
            .join("share")
            .join("tcx")
            .join("watch")
    })
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Every place `watch.ini` may live, most specific first.
///
/// The lookup above picks **one** root from the environment, so setting
/// `TCX_HOME` silently hides a `watch.ini` that already exists under
/// `~/.config/tcx/`. From the user's side that looks like "the file is right
/// there and the tool cannot see it". So the environment-preferred path is
/// tried first and the conventional ones after it, rather than betting on one.
pub fn config_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(dir) = config_dir() {
        push(dir.join("watch.ini"));
    }
    // An explicit `TCX_HOME` is a statement about where config lives. Falling
    // through to another root's file would make `tgw` read a config the user
    // did not point it at, so that case stops here.
    if env_nonempty("TCX_HOME").is_some() {
        return out;
    }
    if let Some(xdg) = env_nonempty("XDG_CONFIG_HOME") {
        push(PathBuf::from(xdg).join("tcx").join("watch.ini"));
    }
    if let Some(home) = env_nonempty("HOME") {
        push(PathBuf::from(&home).join(".config").join("tcx").join("watch.ini"));
        push(PathBuf::from(home).join(".local/share/tcx-home/config/watch.ini"));
    }
    out
}

/// The path `tgw` uses: the first candidate that exists.
///
/// Falls back to the environment-preferred path so a caller that is about to
/// *create* a config writes it where the current setup expects it.
pub fn config_path() -> Option<PathBuf> {
    let candidates = config_candidates();
    match candidates.iter().find(|p| p.is_file()) {
        Some(found) => Some(found.clone()),
        None => candidates.into_iter().next(),
    }
}

/// Writes a default `watch.ini` if no candidate exists yet.
///
/// Returns the path written, or `None` when a config already exists anywhere
/// (nothing is ever overwritten) or no directory could be created.
pub fn create_default() -> Option<PathBuf> {
    let candidates = config_candidates();
    if candidates.iter().any(|p| p.is_file()) {
        return None;
    }
    let target = candidates.into_iter().next()?;
    let dir = target.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&target, DEFAULT_WATCH_INI).ok()?;
    Some(target)
}

/// The starting configuration, written on first run.
///
/// `confirm` mode and an explicit gate: nothing runs until approved, and there
/// is no variant of this file that runs a block ungated.
pub const DEFAULT_WATCH_INI: &str = "\
# tgw - the watchdog. See `tgw --help` and docs/tgw.md.
#
# Start in confirm: nothing runs until you tap approve. Move to auto only
# after `tgw status` says the approval lock is open.

clip_get_cmd = clipg
gate_cmd = tgr --check {file}

mode = confirm
poll_ms = 500
timeout_s = 120
hold_ttl_s = 300
";

/// Reads `watch.ini`, returning the path it read (or would have) with it.
///
/// A missing file is **not** an error here: it yields defaults, which then
/// fail [`Config::validate`] because `clip_get_cmd` and `gate_cmd` are
/// required. That way "no config" and "empty config" report the same problem
/// instead of the former looking like a successful start.
pub fn load() -> (Config, Option<PathBuf>, Vec<String>) {
    let path = config_path();
    let mut cfg = Config::default();
    let mut problems = Vec::new();
    if let Some(p) = &path {
        match std::fs::read_to_string(p) {
            Ok(text) => problems = parse_into(&mut cfg, &text),
            Err(e) => problems.push(format!("{}: cannot read: {e}", p.display())),
        }
    }
    (cfg, path, problems)
}

/// Parses `watch.ini` text over the defaults.
///
/// Unknown keys are **collected as problems**, not ignored: a typo in
/// `gate_cmd` would otherwise mean no gate at all. See [`parse_detailed`].
pub fn parse(text: &str) -> (Config, Vec<String>) {
    let mut cfg = Config::default();
    let problems = parse_into(&mut cfg, text);
    (cfg, problems)
}

/// Applies `text` to `cfg`, returning every problem found.
///
/// Separate from [`parse`] so tests can start from a non-default config.
pub fn parse_into(cfg: &mut Config, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();

    for (lineno, raw) in text.lines().enumerate() {
        let n = lineno + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            // Sections are accepted and ignored: a hand-edited file often
            // grows them, and refusing a harmless header helps nobody. A key
            // that is unknown inside a section is still a problem.
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            problems.push(format!("line {n}: expected `key = value`, found `{line}`"));
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        match key.as_str() {
            "clip_get_cmd" => cfg.clip_get_cmd = value.into(),
            "gate_cmd" => cfg.gate_cmd = value.into(),
            "notify_cmd" => cfg.notify_cmd = value.into(),
            "wakelock_cmd" => cfg.wakelock_cmd = value.into(),
            "wakelock_release_cmd" => cfg.wakelock_release_cmd = value.into(),
            "shell" => cfg.shell = value.into(),
            "prelude" => cfg.prelude = value.into(),
            "cwd" => cfg.cwd = value.into(),
            "cwd_file" => cfg.cwd_file = value.into(),
            "trigger_first_line" => cfg.trigger_first_line = value.into(),
            "state_dir" => cfg.state_dir = value.into(),
            "log_dir" => cfg.log_dir = value.into(),
            "env_allow" => {
                cfg.env_allow = split_list(value);
            }
            "cwd_mode" => match CwdMode::parse(value) {
                Ok(m) => cfg.cwd_mode = m,
                Err(e) => problems.push(format!("line {n}: {e}")),
            },
            "mode" => match Mode::parse(value) {
                Ok(m) => cfg.mode = m,
                Err(e) => problems.push(format!("line {n}: {e}")),
            },
            "poll_ms" => set_u64(&mut cfg.poll_ms, value, n, "poll_ms", &mut problems),
            "max_block_bytes" => {
                set_u64(&mut cfg.max_block_bytes, value, n, "max_block_bytes", &mut problems)
            }
            "timeout_s" => set_u64(&mut cfg.timeout_s, value, n, "timeout_s", &mut problems),
            "gate_timeout_s" => {
                set_u64(&mut cfg.gate_timeout_s, value, n, "gate_timeout_s", &mut problems)
            }
            "max_output_bytes" => set_u64(
                &mut cfg.max_output_bytes,
                value,
                n,
                "max_output_bytes",
                &mut problems,
            ),
            "hold_ttl_s" => set_u64(&mut cfg.hold_ttl_s, value, n, "hold_ttl_s", &mut problems),
            "auto_unlock_approvals" => set_u64(
                &mut cfg.auto_unlock_approvals,
                value,
                n,
                "auto_unlock_approvals",
                &mut problems,
            ),
            // Removed key, kept as a named error so an old file says what to
            // do instead of reporting a bare "unknown key".
            "hold_rules" => problems.push(format!(
                "line {n}: `hold_rules` was removed: rules, including which ones \
                 hold a block, are defined in trex-guard's config.ini as `review` \
                 rules. Delete this line."
            )),
            other => problems.push(format!("line {n}: unknown key `{other}`")),
        }
        if let Some(k) = known_key(&key) {
            seen.insert(k);
        }
    }

    let _ = seen;
    problems
}

/// The canonical name of a key, for "was it set" questions later.
fn known_key(key: &str) -> Option<&'static str> {
    const KEYS: &[&str] = &[
        "clip_get_cmd",
        "gate_cmd",
        "notify_cmd",
        "wakelock_cmd",
        "wakelock_release_cmd",
        "shell",
        "prelude",
        "env_allow",
        "cwd_mode",
        "cwd",
        "cwd_file",
        "trigger_first_line",
        "poll_ms",
        "max_block_bytes",
        "timeout_s",
        "gate_timeout_s",
        "max_output_bytes",
        "hold_ttl_s",
        "mode",
        "auto_unlock_approvals",
        "state_dir",
        "log_dir",
    ];
    KEYS.iter().find(|k| **k == key).copied()
}

fn set_u64(slot: &mut u64, value: &str, lineno: usize, name: &str, problems: &mut Vec<String>) {
    match value.parse::<u64>() {
        Ok(v) => *slot = v,
        Err(_) => problems.push(format!(
            "line {lineno}: {name} must be a whole number, found `{value}`"
        )),
    }
}

/// Splits a comma-separated list, trimming and dropping empties.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Splits an argument string the way a shell would, except:
///
/// - **no expansion** at all: `$HOME` stays the literal `$HOME`;
/// - **no operators**: `|`, `;`, `>` are ordinary characters, so a command
///   field can never grow a pipeline;
/// - unterminated quotes are an **error**, not a silent best guess.
///
/// Supports `'single'`, `"double"` (with `\"` and `\\` escapes only) and
/// backslash escapes outside quotes, which is what makes
/// `powershell -c Get-Clipboard` and `notify-send "a b"` both work.
pub fn split_argv(s: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_cur = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if has_cur {
                    out.push(std::mem::take(&mut cur));
                    has_cur = false;
                }
            }
            '\'' => {
                has_cur = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err("unterminated single quote".into()),
                    }
                }
            }
            '"' => {
                has_cur = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('"') => cur.push('"'),
                            Some('\\') => cur.push('\\'),
                            Some(other) => {
                                cur.push('\\');
                                cur.push(other);
                            }
                            None => return Err("unterminated double quote".into()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err("unterminated double quote".into()),
                    }
                }
            }
            '\\' => {
                has_cur = true;
                match chars.next() {
                    Some(ch) => cur.push(ch),
                    None => return Err("trailing backslash".into()),
                }
            }
            other => {
                has_cur = true;
                cur.push(other);
            }
        }
    }
    if has_cur {
        out.push(cur);
    }
    Ok(out)
}

/// Substitutes `{name}` placeholders in an argv list.
///
/// Only whole-token placeholders are substituted (`{file}`), never inside a
/// larger word, so a path containing `{` cannot confuse it and a substituted
/// value is always exactly one argument. A value that legitimately starts with
/// `{` is written `\{`.
pub fn substitute(argv: &[String], pairs: &[(&str, &str)]) -> Vec<String> {
    argv.iter()
        .map(|arg| {
            if arg.len() >= 2 && arg.starts_with('{') && arg.ends_with('}') {
                let key = &arg[1..arg.len() - 1];
                if let Some((_, v)) = pairs.iter().find(|(k, _)| *k == key) {
                    return (*v).to_string();
                }
            }
            arg.clone()
        })
        .collect()
}

impl Config {
    /// Every problem that stops `tgw` from running.
    ///
    /// Empty means the config is usable. Collected rather than returned one at
    /// a time: fixing config by restart-and-retry is how people give up.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.clip_get_cmd.trim().is_empty() {
            problems.push("clip_get_cmd is required (it prints the clipboard to stdout)".into());
        }
        if self.gate_cmd.trim().is_empty() {
            problems.push(
                "gate_cmd is required (e.g. `tgr --check {file}`); there is no ungated mode".into(),
            );
        } else {
            match split_argv(&self.gate_cmd) {
                Err(e) => problems.push(format!("gate_cmd does not parse: {e}")),
                Ok(argv) => {
                    if !argv.iter().any(|a| a.contains("{file}")) {
                        problems.push(
                            "gate_cmd must contain {file}, or the gate cannot see the block".into(),
                        );
                    }
                }
            }
        }
        for (name, value) in [
            ("clip_get_cmd", &self.clip_get_cmd),
            ("notify_cmd", &self.notify_cmd),
            ("wakelock_cmd", &self.wakelock_cmd),
            ("wakelock_release_cmd", &self.wakelock_release_cmd),
            ("shell", &self.shell),
        ] {
            if value.trim().is_empty() {
                continue;
            }
            if let Err(e) = split_argv(value) {
                problems.push(format!("{name} does not parse: {e}"));
            }
        }
        if self.trigger_first_line.trim().is_empty() {
            problems.push(
                "trigger_first_line is required: without it every clipboard change is a block"
                    .into(),
            );
        }
        if self.timeout_s == 0 {
            problems.push("timeout_s must be at least 1 second".into());
        }
        if self.gate_timeout_s == 0 {
            problems.push("gate_timeout_s must be at least 1 second".into());
        }
        if self.poll_ms == 0 {
            problems.push("poll_ms must be at least 1".into());
        }
        if self.max_block_bytes == 0 {
            problems.push("max_block_bytes must be at least 1".into());
        }
        match self.cwd_mode {
            CwdMode::Fixed if self.cwd.trim().is_empty() => {
                problems.push("cwd_mode = fixed requires cwd".into());
            }
            CwdMode::File if self.cwd_file.trim().is_empty() => {
                problems.push("cwd_mode = file requires cwd_file".into());
            }
            CwdMode::Fixed => {
                let p = Path::new(&self.cwd);
                if !p.is_absolute() {
                    problems.push(format!("cwd must be an absolute path, got `{}`", self.cwd));
                } else if !p.is_dir() {
                    problems.push(format!("cwd does not exist: `{}`", self.cwd));
                }
            }
            _ => {}
        }
        if self.cwd_mode == CwdMode::File {
            let p = Path::new(&self.cwd_file);
            if !p.is_absolute() {
                problems.push(format!(
                    "cwd_file must be an absolute path, got `{}`",
                    self.cwd_file
                ));
            }
        }
        if self.env_allow.iter().any(|v| v.trim().is_empty()) {
            problems.push("env_allow contains an empty name".into());
        }
        problems
    }

    /// The state directory, with the XDG default applied.
    pub fn state_dir(&self) -> PathBuf {
        if self.state_dir.trim().is_empty() {
            default_state_dir().unwrap_or_else(|| PathBuf::from("/tmp/tgw-state"))
        } else {
            PathBuf::from(&self.state_dir)
        }
    }

    /// The log directory, defaulting to `state_dir/log`.
    pub fn log_dir(&self) -> PathBuf {
        if self.log_dir.trim().is_empty() {
            self.state_dir().join("log")
        } else {
            PathBuf::from(&self.log_dir)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_dry_install() {
        let c = Config::default();
        assert_eq!(c.mode, Mode::Dry);
        assert_eq!(c.cwd_mode, CwdMode::Inherit);
        assert_eq!(c.trigger_first_line, "set +H");
        assert!(c.clip_get_cmd.is_empty());
        assert!(c.gate_cmd.is_empty());
    }

    #[test]
    fn a_missing_gate_and_clipboard_are_both_reported() {
        let problems = Config::default().validate();
        assert!(problems.iter().any(|p| p.contains("clip_get_cmd")));
        assert!(problems.iter().any(|p| p.contains("gate_cmd")));
    }

    #[test]
    fn gate_cmd_must_reference_the_file() {
        let mut c = Config::default();
        c.clip_get_cmd = "clipg".into();
        c.gate_cmd = "tgr --check".into();
        let problems = c.validate();
        assert!(
            problems.iter().any(|p| p.contains("{file}")),
            "a gate that cannot see the block was accepted: {problems:?}"
        );
    }

    #[test]
    fn a_valid_minimal_config_has_no_problems() {
        let mut c = Config::default();
        c.clip_get_cmd = "clipg".into();
        c.gate_cmd = "tgr --check {file}".into();
        assert_eq!(c.validate(), Vec::<String>::new());
    }

    #[test]
    fn unknown_keys_are_problems_not_silent() {
        let (_, problems) = parse("gate_cmd = tgr --check {file}\ngat_cmd = oops\n");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("unknown key `gat_cmd`"), "{problems:?}");
    }

    #[test]
    fn a_typo_in_mode_does_not_fall_back_to_dry() {
        let (c, problems) = parse("mode = confrim\n");
        assert_eq!(c.mode, Mode::Dry, "the default is still dry");
        assert!(problems.iter().any(|p| p.contains("unknown mode")), "{problems:?}");
    }

    #[test]
    fn lists_split_on_commas_and_trim() {
        let (c, problems) = parse("env_allow = PATH, HOME\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.env_allow, vec!["PATH", "HOME"]);
    }

    #[test]
    fn hold_rules_is_no_longer_a_key() {
        // It was removed: it could only turn a *safe* pass into a hold, i.e.
        // it was a second place to express "review this", competing with the
        // rules in `config.ini`. Naming it now is an error rather than being
        // ignored, so an old file does not look configured when it is not.
        let (_, problems) = parse("hold_rules = rm, mv\n");
        let msg = problems.join("\n");
        assert!(msg.contains("`hold_rules` was removed"), "{msg}");
        // The message has to say where the rules live now, or the user is
        // left to guess.
        assert!(msg.contains("config.ini"), "{msg}");
        assert!(msg.contains("review"), "{msg}");
    }

    #[test]
    fn numbers_must_be_numbers() {
        let (_, problems) = parse("poll_ms = often\n");
        assert!(problems[0].contains("whole number"), "{problems:?}");
    }

    #[test]
    fn sections_and_comments_are_accepted() {
        let (c, problems) = parse(
            "# a comment\n; another\n[general]\nmode = confirm\n\n[commands]\ngate_cmd = tgr --check {file}\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.mode, Mode::Confirm);
    }

    #[test]
    fn split_argv_handles_quotes_and_escapes() {
        assert_eq!(split_argv("tgr --check {file}").unwrap(), ["tgr", "--check", "{file}"]);
        assert_eq!(
            split_argv(r#"notify-send "a b" c"#).unwrap(),
            ["notify-send", "a b", "c"]
        );
        assert_eq!(split_argv("powershell -c Get-Clipboard").unwrap().len(), 3);
        assert_eq!(split_argv(r#"echo 'x y'"#).unwrap(), ["echo", "x y"]);
        assert_eq!(split_argv(r#"echo a\ b"#).unwrap(), ["echo", "a b"]);
    }

    #[test]
    fn split_argv_never_expands_or_interprets() {
        // No shell: `$HOME`, `|` and `>` are literal characters.
        assert_eq!(split_argv("echo $HOME").unwrap(), ["echo", "$HOME"]);
        assert_eq!(split_argv("echo a | sh").unwrap(), ["echo", "a", "|", "sh"]);
        assert_eq!(split_argv("echo a > f").unwrap(), ["echo", "a", ">", "f"]);
    }

    #[test]
    fn split_argv_rejects_unterminated_quotes() {
        assert!(split_argv(r#"echo "abc"#).is_err());
        assert!(split_argv("echo 'abc").is_err());
        assert!(split_argv("echo abc\\").is_err());
    }

    #[test]
    fn substitution_is_whole_token_only() {
        let argv = vec!["x".to_string(), "{file}".to_string(), "pre-{file}".to_string()];
        let got = substitute(&argv, &[("file", "/tmp/a b")]);
        assert_eq!(got, ["x", "/tmp/a b", "pre-{file}"]);
    }

    #[test]
    fn a_substituted_value_is_always_one_argument() {
        // The whole reason for whole-token substitution: a path with a space
        // cannot become two arguments.
        let argv = vec!["tgw".to_string(), "{file}".to_string()];
        let got = substitute(&argv, &[("file", "/tmp/a b c")]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1], "/tmp/a b c");
    }

    #[test]
    fn fixed_cwd_must_be_absolute_and_exist() {
        let mut c = Config::default();
        c.clip_get_cmd = "clipg".into();
        c.gate_cmd = "tgr --check {file}".into();
        c.cwd_mode = CwdMode::Fixed;
        c.cwd = "relative/path".into();
        assert!(c.validate().iter().any(|p| p.contains("absolute")));
        c.cwd = "/nonexistent/tgw-dir".into();
        assert!(c.validate().iter().any(|p| p.contains("does not exist")));
    }

    #[test]
    fn file_cwd_requires_the_file_key() {
        let mut c = Config::default();
        c.clip_get_cmd = "clipg".into();
        c.gate_cmd = "tgr --check {file}".into();
        c.cwd_mode = CwdMode::File;
        assert!(c.validate().iter().any(|p| p.contains("cwd_file")));
    }

    #[test]
    fn an_empty_trigger_is_refused() {
        let mut c = Config::default();
        c.clip_get_cmd = "clipg".into();
        c.gate_cmd = "tgr --check {file}".into();
        c.trigger_first_line = "  ".into();
        assert!(c.validate().iter().any(|p| p.contains("trigger_first_line")));
    }
}
