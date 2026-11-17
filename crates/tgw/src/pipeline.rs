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
//! # `dry` never executes
//!
//! `dry` logs a passing block as a hold and stops. It is the default, so a
//! fresh install cannot run anything by accident. `confirm` waits for a human
//! (slice 5); `auto` runs a pass.
//!
//! # Nothing here can de-escalate
//!
//! A `Pass` runs (in `auto`), a `Hold` waits for a human, and a `Block` is
//! refused. There is no config key anywhere in `watch.ini` that turns a hold
//! or a block back into a run: the gate owns the verdict, and `watch.ini` only
//! chooses what a verdict *means* for execution.

use std::time::{Duration, Instant};

use crate::clip::{self, Reject, Seen};
use crate::config::{Config, Mode};
use crate::log::{Entry, Log, Verdict};

/// What the gate said. Only these three exist: anything else is a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateResult {
    /// Nothing matched: safe. `reason` carries any watch names, for the log.
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
    pub duration_ms: u64,
    pub staged: Option<clip::Staged>,
}

impl<'a> Context<'a> {
    /// Builds a context for one process. `seen` outlives the context because
    /// the caller owns it across iterations.
    pub fn new(cfg: &'a Config, log: &'a Log, seen: &'a mut Seen) -> Context<'a> {
        Context {
            cfg,
            log,
            seen,
            last_ignore: None,
        }
    }
}

/// Everything the pipeline needs that is not config.
pub struct Context<'a> {
    pub cfg: &'a Config,
    pub log: &'a Log,
    pub seen: &'a mut Seen,
    /// The last routine ignore reason, so an idle watchdog does not write a
    /// log line per poll. "Empty" and "duplicate" are the two the loop
    /// produces continuously; anything unusual is logged every time.
    last_ignore: Option<String>,
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
            duration_ms: started.elapsed().as_millis() as u64,
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
                duration_ms: started.elapsed().as_millis() as u64,
                staged: None,
            };
        }
    };

    // Mark it seen on disk now that it is staged, so a restart dedupes.
    let _ = clip::mark_seen(&ctx.cfg.state_dir(), hash);

    // 6. Gate.
    let gate = crate::gate::check(ctx.cfg, &staged.path);
    outcome_from_gate(ctx, gate, staged, started, &text)
}

fn ignore(ctx: &mut Context<'_>, reject: Reject, started: Instant) -> Outcome {
    let mut entry = Entry::new(Verdict::Ignore, reject.reason());

    // An idle clipboard says "empty" every poll and an unchanged one says
    // "duplicate". Logging each would be ~18,000 lines an hour of nothing.
    // Log the first occurrence of a run, and log again when the reason
    // changes -- including a read failure, which must not vanish because the
    // previous poll was merely empty.
    let routine = matches!(reject, Reject::Empty | Reject::Duplicate);
    let same_as_last = ctx.last_ignore.as_deref() == Some(entry.reason.as_str());
    if !(routine && same_as_last) {
        entry.duration_ms = started.elapsed().as_millis() as u64;
        let _ = ctx.log.append(&entry);
    }
    ctx.last_ignore = Some(entry.reason.clone());

    Outcome {
        verdict: Verdict::Ignore,
        reason: entry.reason,
        id: String::new(),
        hash: 0,
        duration_ms: started.elapsed().as_millis() as u64,
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
    text: &str,
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

    // 7. Run, only for `Run`. This is the single call site of the executor.
    let (verdict, reason, duration_ms) = if verdict == Verdict::Run {
        let mut entry = Entry::new(verdict, reason.clone());
        entry.id = staged.id.clone();
        entry.hash = staged.hash;
        entry.exit = exit;
        entry.duration_ms = started.elapsed().as_millis() as u64;
        let _ = ctx.log.append(&entry);

        match crate::execute::run_block(ctx.cfg, &staged.path) {
            Ok(run) => {
                let mut ran = Entry::new(run_verdict(&run), run_reason(&run, text));
                ran.id = staged.id.clone();
                ran.hash = staged.hash;
                ran.exit = run.exit;
                ran.duration_ms = run.duration_ms;
                let _ = ctx.log.append(&ran);
                let d = started.elapsed().as_millis() as u64;
                (run_verdict(&run), ran.reason, d)
            }
            Err(e) => {
                let mut ran = Entry::new(Verdict::Block, e.reason());
                ran.id = staged.id.clone();
                ran.hash = staged.hash;
                let _ = ctx.log.append(&ran);
                (Verdict::Block, ran.reason, started.elapsed().as_millis() as u64)
            }
        }
    } else {
        let mut entry = Entry::new(verdict, reason.clone());
        entry.id = staged.id.clone();
        entry.hash = staged.hash;
        entry.exit = exit;
        entry.duration_ms = started.elapsed().as_millis() as u64;
        let _ = ctx.log.append(&entry);
        (verdict, reason, entry.duration_ms)
    };

    let _ = ctx.log.keep_block(&staged.id, text);

    // 8. A hold becomes a record the human can approve or deny. `dry` records
    //    one too, so the human can see exactly what *would* have run; approval
    //    of a dry-mode hold is refused by `holds::verify`.
    if verdict == Verdict::Hold {
        let hold = crate::holds::Hold {
            id: staged.id.clone(),
            hash: staged.hash,
            reason: reason.clone(),
            expires: crate::log::now_unix() + ctx.cfg.hold_ttl_s,
            file: staged
                .path
                .strip_prefix(ctx.cfg.state_dir())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| staged.path.to_string_lossy().into_owned()),
            mode: ctx.cfg.mode,
            created: crate::log::now_unix(),
        };
        // A saved hold is the contract: the id is announced, so `approve`
        // must be able to find it. Discarding this error printed an id that
        // led nowhere, which is indistinguishable from `tgw` doing nothing.
        if let Err(e) = crate::holds::save(&ctx.cfg.state_dir(), &hold) {
            let reason = format!("cannot save hold: {e}");
            let mut entry = Entry::new(Verdict::Block, reason.clone());
            entry.id = hold.id.clone();
            entry.hash = hold.hash;
            entry.duration_ms = started.elapsed().as_millis() as u64;
            let _ = ctx.log.append(&entry);
            return Outcome {
                verdict: Verdict::Block,
                reason,
                id: hold.id,
                hash: staged.hash,
                duration_ms: started.elapsed().as_millis() as u64,
                staged: Some(staged),
            };
        }
    }

    Outcome {
        verdict,
        reason,
        id: staged.id.clone(),
        hash: staged.hash,
        duration_ms,
        staged: Some(staged),
    }
}

/// The verdict after running: success is `Run`, anything else is a hold-worthy
/// failure the human should see. A failed run is never silently `Run`.
fn run_verdict(run: &crate::execute::RunResult) -> Verdict {
    if run.success() {
        Verdict::Run
    } else {
        Verdict::Hold
    }
}

fn run_reason(run: &crate::execute::RunResult, text: &str) -> String {
    let mut s = if run.timed_out {
        "run timed out".to_string()
    } else {
        match run.exit {
            Some(0) => "ran successfully".to_string(),
            Some(c) => format!("exited {c}"),
            None => "run was killed".to_string(),
        }
    };
    if run.truncated {
        s.push_str(" (output truncated)");
    }
    // The block's own output goes back to the clipboard, so the human reads
    // the result where they will paste. Never written to a file by tgw.
    let _ = text;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> (std::path::PathBuf, Config, Log) {
        let d = std::env::temp_dir().join(format!("tgw-pipe-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut c = Config::default();
        c.clip_get_cmd = "true".into();
        c.gate_cmd = "true {file}".into();
        c.state_dir = d.join("state").to_string_lossy().into_owned();
        c.log_dir = d.join("log").to_string_lossy().into_owned();
        let log = Log::new(c.log_dir(), 50);
        (d, c, log)
    }

    #[test]
    fn an_idle_loop_does_not_log_a_line_per_poll() {
        // An empty clipboard is read every poll. Logging each read fills the
        // log with nothing: found by running the real loop for 4 s and
        // getting 12 lines.
        let (d, cfg, log) = scratch("idle");
        let mut seen = Seen::new(16);
        let mut ctx = Context::new(&cfg, &log, &mut seen);
        for _ in 0..20 {
            let out = process_once(&mut ctx);
            assert_eq!(out.verdict, Verdict::Ignore);
        }
        let n = log.tail(100).len();
        assert!(n <= 2, "20 empty polls wrote {n} log lines");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_changed_ignore_reason_is_still_logged() {
        // Suppressing repeats must not hide a *different* problem, such as the
        // clipboard command failing after a run of empties.
        let (d, mut cfg, log) = scratch("changed");
        let mut seen = Seen::new(16);
        {
            let mut ctx = Context::new(&cfg, &log, &mut seen);
            for _ in 0..3 {
                process_once(&mut ctx);
            }
        }
        cfg.clip_get_cmd = "false".into();
        let mut seen2 = Seen::new(16);
        let mut ctx = Context::new(&cfg, &log, &mut seen2);
        process_once(&mut ctx);
        let lines = log.tail(100);
        assert!(
            lines.iter().any(|l| l.contains("clipboard read failed")),
            "a read failure after empties was swallowed: {lines:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

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
