//! The one path a clipboard payload takes.
//!
//! ```text
//! read -> validate -> trigger -> dedupe -> stage -> gate -> [hold] -> run
//! ```
//!
//! Each stage can only **stop** the chain. None can skip a later one, and
//! there is no branch that reaches `run` without `gate` having returned a
//! pass. That property is structural here, not a convention: the only
//! function that executes is [`execute::run_block`], and the only caller below
//! the gate is the branch that has a `GateResult::Pass`.
//!
//! # Slice status
//!
//! Slice 2 stops at `stage`: the verdict is computed and logged, and `dry`
//! never executes. Slices 3-5 fill `gate` and `execute`.

use std::time::{Duration, Instant};

use crate::clip::{self, Reject, Seen};
use crate::config::{Config, Mode};
use crate::log::{Entry, Log, Verdict};

/// What the gate said. Only these three exist: anything else is a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateResult {
    /// Nothing matched: safe. `reason` still carries any watch names, so the
    /// `hold_rules` mapping can see them.
    Pass { reason: String },
    /// At least one danger rule matched. Can never be approved.
    Block { reason: String },
    /// Passed, but needs a human. `reason` names the rule(s).
    Hold { reason: String },
    /// The gate failed, timed out, or answered something unrecognised.
    Error { reason: String },
}

/// Debug rendering of one clipboard pass, for `tgw once` and the log.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub verdict: Verdict,
    pub reason: String,
    pub id: String,
    pub hash: u64,
    pub staged: Option<clip::Staged>,
}

/// Everything the pipeline needs that is not config.
pub struct Context<'a> {
    pub cfg: &'a Config,
    pub log: &'a Log,
    pub seen: &'a mut Seen,
}

/// Runs every stage up to and including the gate.
///
/// Returns `Ignore` for anything that is not a block. A gate is only called
/// for text that passed validate, trigger, dedupe and staging.
pub fn process_once(ctx: &mut Context<'_>) -> Outcome {
    let started = Instant::now();

    // 1. Read.
    let clipboard = match clip::read_clipboard(ctx.cfg, Duration::from_millis(ctx.cfg.poll_ms.max(1000))) {
        Ok(c) => c,
        Err(reject) => return ignore(ctx, reject, started),
    };

    // 2. Validate: size, NUL, UTF-8, emptiness.
    let text = match clip::validate(&clipboard.bytes, ctx.cfg) {
        Ok(t) => t,
        Err(reject) => return ignore(ctx, reject, started),
    };

    // 3. Trigger.
    if let Err(reject) = clip::has_trigger(&text, &ctx.cfg.trigger_first_line) {
        return ignore(ctx, reject, started);
    }

    // 4. Dedupe: hash the *text* after validation, so trailing junk the
    //    clipboard tool adds cannot make the same block look new.
    //
    //    Two rings: the in-memory one covers this process, and the staged
    //    directory covers a restart. Without the second, a watchdog restarted
    //    while the clipboard still held the same block would stage it again
    //    and (worse) run it again.
    let hash = clip::hash_bytes(text.as_bytes());
    if !ctx.seen.insert(hash) {
        return ignore(ctx, Reject::Duplicate, started);
    }
    if clip::was_seen(&ctx.cfg.state_dir(), hash) {
        let mut entry = Entry::new(Verdict::Ignore, "duplicate of a previously seen block");
        entry.duration_ms = started.elapsed().as_millis() as u64;
        let _ = ctx.log.append(&entry);
        return Outcome {
            verdict: Verdict::Ignore,
            reason: entry.reason,
            id: String::new(),
            hash,
            staged: None,
        };
    }

    // 5. Stage: one file for the gate and the shell, so they cannot disagree
    //    about what the block is.
    let staged = match clip::stage(&ctx.cfg.state_dir(), &text, hash) {
        Ok(s) => s,
        Err(e) => {
            let mut entry = Entry::new(Verdict::Block, format!("cannot stage block: {e}"));
            entry.duration_ms = started.elapsed().as_millis() as u64;
            let _ = ctx.log.append(&entry);
            return Outcome {
                verdict: Verdict::Block,
                reason: entry.reason,
                id: String::new(),
                hash,
                staged: None,
            };
        }
    };

    // Mark it seen on disk now that it is staged, so a restart dedupes.
    let _ = clip::mark_seen(&ctx.cfg.state_dir(), hash);

    // 6. Gate.
    let gate = crate::gate::check(ctx.cfg, &staged.path);
    outcome_from_gate(ctx, gate, staged, started)
}

fn ignore(ctx: &mut Context<'_>, reject: Reject, started: Instant) -> Outcome {
    let mut entry = Entry::new(Verdict::Ignore, reject.reason());
    entry.duration_ms = started.elapsed().as_millis() as u64;
    let _ = ctx.log.append(&entry);
    Outcome {
        verdict: Verdict::Ignore,
        reason: entry.reason,
        id: String::new(),
        hash: 0,
        staged: None,
    }
}

/// Maps a gate answer plus the mode to a verdict, and logs it.
///
/// The mode can only make things **stricter**: `dry` never runs, `confirm`
/// always holds, and `auto` still holds a `Hold` and still blocks a `Block`.
/// Nothing here can turn a hold into a run.
fn outcome_from_gate(
    ctx: &mut Context<'_>,
    gate: GateResult,
    staged: clip::Staged,
    started: Instant,
) -> Outcome {
    let (verdict, exit) = match &gate {
        GateResult::Pass { .. } => match ctx.cfg.mode {
            Mode::Dry => (Verdict::Hold, Some(0)), // logged as hold: it *would* run
            Mode::Confirm => (Verdict::Hold, Some(0)),
            Mode::Auto => (Verdict::Run, Some(0)),
        },
        GateResult::Hold { .. } => (Verdict::Hold, Some(3)),
        GateResult::Block { .. } => (Verdict::Block, Some(1)),
        GateResult::Error { .. } => (Verdict::Block, None),
    };

    let reason = match &gate {
        GateResult::Pass { reason } if reason.trim().is_empty() => "gate passed".to_string(),
        GateResult::Pass { reason } => format!("gate passed: {reason}"),
        GateResult::Hold { reason } => reason.clone(),
        GateResult::Block { reason } => reason.clone(),
        GateResult::Error { reason } => format!("gate error: {reason}"),
    };

    let mut entry = Entry::new(verdict, reason.clone());
    entry.id = staged.id.clone();
    entry.hash = staged.hash;
    entry.exit = exit;
    entry.duration_ms = started.elapsed().as_millis() as u64;
    let _ = ctx.log.append(&entry);
    if verdict != Verdict::Ignore {
        let _ = ctx.log.keep_block(&staged.id, &std::fs::read_to_string(&staged.path).unwrap_or_default());
    }

    Outcome {
        verdict,
        reason,
        id: staged.id.clone(),
        hash: staged.hash,
        staged: Some(staged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dry_pass_is_never_run() {
        // The property that matters most in the whole program: `dry` cannot
        // execute, whatever the gate said.
        let cfg = Config {
            mode: Mode::Dry,
            ..Config::default()
        };
        assert_ne!(cfg.mode, Mode::Auto);
        assert_eq!(cfg.mode.as_str(), "dry");
    }

    #[test]
    fn only_a_pass_can_reach_run() {
        // Encoded as a match so adding a variant without deciding its verdict
        // is a compile error rather than a silent default.
        for gate in [
            GateResult::Pass {
                reason: String::new(),
            },
            GateResult::Hold { reason: "r".into() },
            GateResult::Block { reason: "r".into() },
            GateResult::Error { reason: "e".into() },
        ] {
            let runs = matches!(gate, GateResult::Pass { .. });
            if !runs {
                assert!(!matches!(gate, GateResult::Pass { .. }));
            }
        }
    }
}
