//! Holds: blocks waiting for a human tap.
//!
//! A hold is a small record in the state directory plus the staged block. It
//! exists so that "run this" is a separate, later, human act — the watchdog
//! decides *whether a human may be asked*, never that the answer is yes.
//!
//! # What an approval must re-check
//!
//! Approving does **not** trust the record. It re-reads the staged file,
//! re-hashes it, and re-runs the gate, then compares all three against the
//! record:
//!
//! | Check | Why | On failure |
//! | --- | --- | --- |
//! | hash matches | the file may have been edited after staging | refuse |
//! | file still 0600 | something else may own it now | refuse |
//! | not expired | a hold is a moment, not a standing permission | refuse |
//! | mode still allows running | a mode change since the hold is a decision | refuse |
//! | gate still passes | the gate may have learned a rule since | refuse |
//!
//! # What can never be approved
//!
//! A `Block`. [`Hold`] has no field that could hold one: the record stores the
//! original verdict and [`load`] rejects any record that is not a `hold`. There
//! is no code path that constructs an approvable danger.
//!
//! # The `auto` lock
//!
//! `mode = auto` is refused until the human has approved
//! `auto_unlock_approvals` blocks (default 20). The counter only counts
//! approvals that actually ran. Denies are logged and do **not** reset it —
//! they are already recorded, and resetting on a deny would make the lock feel
//! like a punishment rather than a warm-up.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::config::{Config, Mode};
use crate::log::now_unix;

/// A block waiting for a human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub id: String,
    pub hash: u64,
    /// Why it was held: the gate's rule names, or `hold_rules matched: ...`.
    pub reason: String,
    /// Unix time after which the hold is dead.
    pub expires: u64,
    /// The staged file this hold refers to, relative to the state dir.
    pub file: String,
    /// The mode in force when the hold was made.
    pub mode: Mode,
    /// Unix time it was created.
    pub created: u64,
}

/// Why an approval was refused. Each is shown to the human, so each says what
/// to do next where that is possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Unknown(String),
    Expired { at: u64, now: u64 },
    HashMismatch { recorded: u64, found: u64 },
    MissingFile(String),
    /// The file's permissions changed, so this is not the block that was held.
    Permissions(String),
    /// The mode is now `dry`, so nothing may run whatever the human says.
    ModeChanged { was: Mode, now: Mode },
    /// The gate, asked again, no longer passes.
    GateNow(String),
    /// The gate could not be asked again.
    GateError(String),
}

impl Refusal {
    pub fn reason(&self) -> String {
        match self {
            Refusal::Unknown(id) => format!("no hold with id `{id}`"),
            Refusal::Expired { at, now } => format!(
                "hold expired at {at} ({} s ago)",
                now.saturating_sub(*at)
            ),
            Refusal::HashMismatch { recorded, found } => format!(
                "the staged file changed since the hold was made (recorded {recorded:016x}, found {found:016x})"
            ),
            Refusal::MissingFile(p) => format!("the staged block is gone: {p}"),
            Refusal::Permissions(p) => format!("the staged block is not private any more: {p}"),
            Refusal::ModeChanged { was, now } => format!(
                "mode changed from {} to {} since the hold; approve a fresh block instead",
                was.as_str(),
                now.as_str()
            ),
            Refusal::GateNow(e) => format!("the gate no longer passes it: {e}"),
            Refusal::GateError(e) => format!("the gate could not be asked again: {e}"),
        }
    }
}

/// The file a hold record lives in.
pub fn holds_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("holds")
}

fn record_path(state_dir: &Path, id: &str) -> PathBuf {
    holds_dir(state_dir).join(format!("{id}.hold"))
}

/// Writes a hold record. The record is plain `key=value`, one per line, so a
/// human can read it with `cat` when something is stuck.
pub fn save(state_dir: &Path, hold: &Hold) -> std::io::Result<()> {
    let dir = holds_dir(state_dir);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut text = String::new();
    let _ = writeln!(text, "id={}", hold.id);
    let _ = writeln!(text, "hash={:016x}", hold.hash);
    let _ = writeln!(text, "reason={}", hold.reason.replace('\n', " "));
    let _ = writeln!(text, "expires={}", hold.expires);
    let _ = writeln!(text, "file={}", hold.file);
    let _ = writeln!(text, "mode={}", hold.mode.as_str());
    let _ = writeln!(text, "created={}", hold.created);
    let _ = writeln!(text, "verdict=hold");

    let path = record_path(state_dir, &hold.id);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

/// Reads one hold record. A record whose `verdict` is not `hold` is refused:
/// a block must not become approvable by being written to the holds directory.
pub fn load(state_dir: &Path, id: &str) -> Result<Hold, Refusal> {
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        // Ids are hex; anything else is not a hold and could be a path trick.
        return Err(Refusal::Unknown(id.to_string()));
    }
    let path = record_path(state_dir, id);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Err(Refusal::Unknown(id.to_string()));
    };
    let mut hold = Hold {
        id: id.to_string(),
        hash: 0,
        reason: String::new(),
        expires: 0,
        file: String::new(),
        mode: Mode::Dry,
        created: 0,
    };
    let mut verdict = String::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "hash" => hold.hash = u64::from_str_radix(v.trim(), 16).unwrap_or(0),
            "reason" => hold.reason = v.to_string(),
            "expires" => hold.expires = v.trim().parse().unwrap_or(0),
            "file" => hold.file = v.to_string(),
            "created" => hold.created = v.trim().parse().unwrap_or(0),
            "mode" => {
                if let Ok(m) = Mode::parse(v) {
                    hold.mode = m;
                }
            }
            "verdict" => verdict = v.trim().to_string(),
            _ => {}
        }
    }
    if verdict != "hold" {
        // Fail closed: a record without `verdict=hold` is not a hold.
        return Err(Refusal::Unknown(id.to_string()));
    }
    Ok(hold)
}

/// Every pending hold that has not expired, youngest first.
pub fn pending(state_dir: &Path) -> Vec<Hold> {
    let Ok(entries) = std::fs::read_dir(holds_dir(state_dir)) else {
        return Vec::new();
    };
    let now = now_unix();
    let mut out: Vec<Hold> = entries
        .flatten()
        .filter_map(|e| {
            let stem = e.path().file_stem()?.to_string_lossy().into_owned();
            load(state_dir, &stem).ok()
        })
        .filter(|h| h.expires > now)
        .collect();
    out.sort_by_key(|h| std::cmp::Reverse(h.created));
    out
}

/// Deletes a hold record and its staged file.
pub fn remove(state_dir: &Path, hold: &Hold) {
    let _ = std::fs::remove_file(record_path(state_dir, &hold.id));
    let _ = std::fs::remove_file(state_dir.join(&hold.file));
}

/// Checks a hold against the file on disk and the config, and returns the
/// verified path. Does **not** approve anything: the caller still runs it.
pub fn verify(
    cfg: &Config,
    state_dir: &Path,
    hold: &Hold,
    gate: impl FnOnce(&Path) -> crate::pipeline::GateResult,
) -> Result<PathBuf, Refusal> {
    let now = now_unix();
    if hold.expires <= now {
        return Err(Refusal::Expired {
            at: hold.expires,
            now,
        });
    }
    if cfg.mode == Mode::Dry {
        return Err(Refusal::ModeChanged {
            was: hold.mode,
            now: cfg.mode,
        });
    }
    let path = state_dir.join(&hold.file);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Refusal::MissingFile(path.display().to_string()))
        }
        Err(e) => {
            return Err(Refusal::MissingFile(format!("{}: {e}", path.display())));
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                return Err(Refusal::Permissions(format!(
                    "{} is {mode:o}, expected 600",
                    path.display()
                )));
            }
        }
    }
    let found = crate::clip::hash_bytes(&bytes);
    if found != hold.hash {
        return Err(Refusal::HashMismatch {
            recorded: hold.hash,
            found,
        });
    }
    match gate(&path) {
        crate::pipeline::GateResult::Pass { reason, .. } => {
            // A `hold_rules` match at approval time is still a pass: the human
            // is the approval. But a rule that now blocks must refuse.
            let _ = reason;
            Ok(path)
        }
        crate::pipeline::GateResult::Block { reason } => Err(Refusal::GateNow(format!(
            "danger: {reason}"
        ))),
        crate::pipeline::GateResult::Hold { reason } => {
            // Still review, not danger. A human approving a review is the
            // whole point, so this proceeds.
            let _ = reason;
            Ok(path)
        }
        crate::pipeline::GateResult::Error { reason } => Err(Refusal::GateError(reason)),
    }
}

// ---------------------------------------------------------------------------
// Approval counter, for the `auto` lock
// ---------------------------------------------------------------------------

fn counter_path(state_dir: &Path) -> PathBuf {
    state_dir.join("approvals")
}

/// How many approvals have actually run.
pub fn approvals(state_dir: &Path) -> u64 {
    std::fs::read_to_string(counter_path(state_dir))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Records one approval that ran. Call only after a successful run: counting
/// a tap that did nothing would unlock `auto` before the human has seen the
/// blocks work.
pub fn record_approval(state_dir: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(state_dir)?;
    let n = approvals(state_dir) + 1;
    let path = counter_path(state_dir);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, format!("{n}\n"))?;
    std::fs::rename(&tmp, &path)?;
    Ok(n)
}

/// Whether `auto` is unlocked, and how many approvals are still needed.
pub fn auto_lock(state_dir: &Path, cfg: &Config) -> (bool, u64) {
    let have = approvals(state_dir);
    let need = cfg.auto_unlock_approvals;
    (have >= need, need.saturating_sub(have))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tgw-holds-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn hold(id: &str) -> Hold {
        Hold {
            id: id.into(),
            hash: 0xabc,
            reason: "rule=rm".into(),
            expires: now_unix() + 300,
            file: format!("stage/{id}.sh"),
            mode: Mode::Confirm,
            created: now_unix(),
        }
    }

    fn clean_cfg() -> Config {
        let mut c = Config::default();
        c.clip_get_cmd = "true".into();
        c.gate_cmd = "true {file}".into();
        c.mode = Mode::Confirm;
        c
    }

    fn stage(dir: &Path, id: &str, text: &str) -> u64 {
        let staged = crate::clip::stage(dir, text, crate::clip::hash_bytes(text.as_bytes())).unwrap();
        // Stage names by hash, not id; make the hold point at the real file.
        let name = staged
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut h = hold(id);
        h.hash = staged.hash;
        h.file = format!("stage/{name}");
        save(dir, &h).unwrap();
        staged.hash
    }

    #[test]
    fn a_hold_round_trips() {
        let d = tmp("roundtrip");
        let h = hold("abc123");
        save(&d, &h).unwrap();
        assert_eq!(load(&d, "abc123").unwrap(), h);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unknown_id_is_refused_not_invented() {
        let d = tmp("unknown");
        assert!(matches!(load(&d, "nope"), Err(Refusal::Unknown(_))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_record_without_verdict_hold_is_not_a_hold() {
        // Someone dropping a record into the holds dir must not get an
        // approval path.
        let d = tmp("nogate");
        std::fs::create_dir_all(holds_dir(&d)).unwrap();
        std::fs::write(record_path(&d, "evil"), "id=evil\nhash=0\nfile=stage/x.sh\n").unwrap();
        assert!(matches!(load(&d, "evil"), Err(Refusal::Unknown(_))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_record_claiming_a_block_is_refused() {
        let d = tmp("blockrec");
        std::fs::create_dir_all(holds_dir(&d)).unwrap();
        std::fs::write(
            record_path(&d, "evil"),
            "id=evil\nverdict=block\nexpires=99999999999\n",
        )
        .unwrap();
        assert!(load(&d, "evil").is_err(), "a block record became approvable");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_id_cannot_be_a_path() {
        let d = tmp("pathid");
        assert!(load(&d, "../../etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn expired_holds_are_not_pending() {
        let d = tmp("expired");
        let mut h = hold("old");
        h.expires = now_unix() - 1;
        save(&d, &h).unwrap();
        let fresh = hold("fresh");
        save(&d, &fresh).unwrap();
        let p = pending(&d);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, "fresh");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_expired_hold_is_refused_even_if_the_file_is_perfect() {
        let d = tmp("expverify");
        let mut h = hold("exp");
        h.expires = now_unix() - 1;
        stage(&d, "exp", "set +H\necho hi\n");
        // Rewrite the record with an already-past expiry: staging saved a
        // fresh one.
        let mut h2 = load(&d, "exp").unwrap();
        h2.expires = h.expires;
        save(&d, &h2).unwrap();
        let h = load(&d, "exp").unwrap();
        let r = verify(&clean_cfg(), &d, &h, |_| crate::pipeline::GateResult::Pass {
            reason: String::new(),
            names: vec![],
        });
        assert!(matches!(r, Err(Refusal::Expired { .. })), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_edited_file_is_refused_by_hash() {
        let d = tmp("edit");
        stage(&d, "ed", "set +H\necho hi\n");
        let h = load(&d, "ed").unwrap();
        let path = d.join(&h.file);
        std::fs::write(&path, "set +H\nrm -rf /\n").unwrap();
        let r = verify(&clean_cfg(), &d, &h, |_| crate::pipeline::GateResult::Pass {
            reason: String::new(),
            names: vec![],
        });
        assert!(matches!(r, Err(Refusal::HashMismatch { .. })), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_verified_hold_passes_through() {
        let d = tmp("verifyok");
        stage(&d, "ok", "set +H\necho hi\n");
        let h = load(&d, "ok").unwrap();
        let r = verify(&clean_cfg(), &d, &h, |_| crate::pipeline::GateResult::Pass {
            reason: String::new(),
            names: vec![],
        });
        assert!(r.is_ok(), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hold_that_now_blocks_is_refused() {
        let d = tmp("nowblock");
        stage(&d, "nb", "set +H\necho hi\n");
        let h = load(&d, "nb").unwrap();
        let r = verify(&clean_cfg(), &d, &h, |_| crate::pipeline::GateResult::Block {
            reason: "rm -rf".into(),
        });
        assert!(matches!(r, Err(Refusal::GateNow(_))), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_gate_error_at_approval_refuses() {
        let d = tmp("gateerr");
        stage(&d, "ge", "set +H\necho hi\n");
        let h = load(&d, "ge").unwrap();
        let r = verify(&clean_cfg(), &d, &h, |_| crate::pipeline::GateResult::Error {
            reason: "timed out".into(),
        });
        assert!(matches!(r, Err(Refusal::GateError(_))), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dry_mode_refuses_any_approval() {
        let d = tmp("dryapprove");
        stage(&d, "dr", "set +H\necho hi\n");
        let h = load(&d, "dr").unwrap();
        let mut c = clean_cfg();
        c.mode = Mode::Dry;
        let r = verify(&c, &d, &h, |_| crate::pipeline::GateResult::Pass {
            reason: String::new(),
            names: vec![],
        });
        assert!(matches!(r, Err(Refusal::ModeChanged { .. })), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_file_is_refused() {
        let d = tmp("missing");
        let h = hold("gone");
        save(&d, &h).unwrap();
        let r = verify(&clean_cfg(), &d, &h, |_| crate::pipeline::GateResult::Pass {
            reason: String::new(),
            names: vec![],
        });
        assert!(matches!(r, Err(Refusal::MissingFile(_))), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_public_staged_file_is_refused() {
        let d = tmp("perms");
        stage(&d, "pm", "set +H\necho hi\n");
        let h = load(&d, "pm").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(d.join(&h.file), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let r = verify(&clean_cfg(), &d, &h, |_| crate::pipeline::GateResult::Pass {
            reason: String::new(),
            names: vec![],
        });
        assert!(matches!(r, Err(Refusal::Permissions(_))), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_approval_counter_counts_and_unlocks() {
        let d = tmp("counter");
        let mut c = clean_cfg();
        c.auto_unlock_approvals = 3;
        assert_eq!(approvals(&d), 0);
        assert_eq!(auto_lock(&d, &c), (false, 3));
        for _ in 0..3 {
            record_approval(&d).unwrap();
        }
        assert_eq!(approvals(&d), 3);
        assert_eq!(auto_lock(&d, &c), (true, 0));
        let _ = std::fs::remove_dir_all(&d);
    }
}
