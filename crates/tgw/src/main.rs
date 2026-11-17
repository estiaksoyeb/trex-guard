//! The `tgw` command.

use std::process::exit;

use tgw::clip::Seen;
use tgw::holds;
use tgw::log::Verdict;
use tgw::log::Log;
use tgw::pipeline::{self, Context};
use tgw::{config, help};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // Universal flags win before any command parses anything, so a value that
    // would otherwise be consumed cannot swallow `--help`.
    match argv.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            if let Some(command) = argv.get(1).and_then(|n| help::find(n)) {
                print!("{}", help::render_command(command));
            } else {
                print!("{}", help::render_all());
            }
            return;
        }
        Some("--version" | "-v") => {
            println!("tgw version {VERSION}");
            return;
        }
        None => {
            // No command: print help to stderr and fail like a usage error,
            // because running a bare watchdog with no instruction is a
            // mistake, not a request.
            eprint!("{}", help::render_all());
            exit(2);
        }
        _ => {}
    }

    let command = argv[0].clone();
    let rest = &argv[1..];

    // `tgw <command> --help` prints that command's help and succeeds.
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        if let Some(c) = help::find(&command) {
            print!("{}", help::render_command(c));
            return;
        }
    }

    let code = match command.as_str() {
        "run" => cmd_run(rest),
        "once" => cmd_once(rest),
        "approve" => cmd_approve(rest),
        "deny" => cmd_deny(rest),
        "status" => cmd_status(rest),
        "log" => cmd_log(rest),
        "doctor" => cmd_doctor(rest),
        "check-config" => cmd_check_config(rest),
        "cd" => cmd_cd(rest),
        other => {
            eprintln!("tgw: unknown command `{other}`");
            eprintln!("Try `tgw --help`.");
            exit(2);
        }
    };
    exit(code);
}

/// Prints usage for a command and returns the usage-error code.
fn usage(command: &str, message: &str) -> i32 {
    eprintln!("tgw: {message}");
    if let Some(c) = help::find(command) {
        eprint!("{}", help::render_command(c));
    }
    2
}

/// Loads and validates config, reporting every problem. `None` means stop.
///
/// A first run has no config file at all. Rather than making that a refusal
/// the user has to diagnose, the default is written once and then loaded. The
/// default is `confirm` with an explicit gate, so this creates nothing that
/// can run a block unattended. An existing file is **never** overwritten.
fn load_config() -> Option<config::Config> {
    if let Some(written) = config::create_default() {
        eprintln!("tgw: created {}", written.display());
    }
    let (cfg, path) = config::load();
    let mut problems = cfg.validate();
    if let Some(p) = &path {
        if !p.exists() {
            problems.insert(0, format!("no config file at {}", p.display()));
        }
    }
    if problems.is_empty() {
        return Some(cfg);
    }
    eprintln!("tgw: configuration is not usable:");
    for p in &problems {
        eprintln!("  - {p}");
    }
    eprintln!("Run `tgw check-config` to see this without starting anything.");
    None
}

fn cmd_check_config(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("check-config", "check-config takes no arguments");
    }
    // First run: create the default rather than reporting its absence. This is
    // the command the error message points at, so it has to be the command
    // that fixes it. An existing config is never touched.
    if let Some(written) = config::create_default() {
        println!("created: {}", written.display());
    }
    let path = config::config_path();
    match &path {
        Some(p) => println!("config: {}", p.display()),
        None => println!("config: (no TCX_HOME, XDG_CONFIG_HOME or HOME set)"),
    }
    let (cfg, text_problems) = match &path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(text) => {
                let mut c = config::Config::default();
                let problems = config::parse_into(&mut c, &text);
                (c, problems)
            }
            Err(e) => (config::Config::default(), vec![format!("cannot read: {e}")]),
        },
        None => (
            config::Config::default(),
            vec!["no config directory could be resolved".to_string()],
        ),
    };
    let mut problems = text_problems;
    problems.extend(cfg.validate());
    if problems.is_empty() {
        println!("ok: mode={} cwd_mode={}", cfg.mode.as_str(), cfg.cwd_mode.as_str());
        return 0;
    }
    for p in &problems {
        println!("problem: {p}");
    }
    1
}

fn cmd_run(rest: &[String]) -> i32 {
    let mut once = false;
    let mut interval = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--once" => once = true,
            "--interval" => {
                i += 1;
                match rest.get(i).and_then(|v| v.parse::<u64>().ok()) {
                    Some(ms) if ms > 0 => interval = Some(ms),
                    _ => return usage("run", "--interval needs milliseconds"),
                }
            }
            other => return usage("run", &format!("unknown option `{other}`")),
        }
        i += 1;
    }
    let Some(mut cfg) = load_config() else {
        return 2;
    };
    if let Some(ms) = interval {
        cfg.poll_ms = ms;
    }
    if let Err(code) = auto_lock_gate(&cfg) {
        return code;
    }

    // The whole loop runs through the same `process_once` as `tgw once`, so
    // there is exactly one pipeline in the program.
    let mut seen = Seen::new(16);
    let log = Log::new(cfg.log_dir(), 200);
    let mut ctx = Context::new(&cfg, &log, &mut seen);
    let problems = tgw::run::check_state_dir(&cfg);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("tgw: {p}");
        }
        return 2;
    }

    if once {
        return report_once(&mut ctx);
    }

    // One instance. Two watchdogs would each see every block, and in `auto`
    // would each try to run it.
    let _lock = match tgw::run::InstanceLock::acquire(&cfg.state_dir()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("tgw: {e}");
            return 1;
        }
    };
    tgw::run::install_signal_handler();
    let swept = tgw::run::sweep_expired(&cfg);
    if swept > 0 {
        eprintln!("tgw: cleared {swept} expired hold(s)");
    }

    eprintln!(
        "tgw: watching (mode={}, poll={} ms). Ctrl-C to stop.",
        cfg.mode.as_str(),
        cfg.poll_ms
    );
    while !tgw::run::stop_requested() {
        let out = pipeline::process_once(&mut ctx);
        announce(&cfg, &out);
        // Sleep in slices so Ctrl-C is noticed promptly even with a long poll.
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(cfg.poll_ms.max(50));
        while std::time::Instant::now() < deadline && !tgw::run::stop_requested() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    eprintln!("tgw: stopped.");
    0
}

fn cmd_once(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("once", "once takes no arguments");
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    // `once` is the same pipeline as `run`, so it must honour the same lock.
    // Otherwise it is a quiet way to run in `auto` before it is unlocked.
    if let Err(code) = auto_lock_gate(&cfg) {
        return code;
    }
    let mut seen = Seen::new(16);
    let log = Log::new(cfg.log_dir(), 200);
    let mut ctx = Context::new(&cfg, &log, &mut seen);
    if let Err(e) = tgw::clip::ensure_state_dir(&cfg.state_dir()) {
        eprintln!("tgw: state dir {}: {e}", cfg.state_dir().display());
        return 1;
    }
    report_once(&mut ctx)
}

/// Refuses to run with `mode = auto` while the approval lock is closed.
///
/// One function for every entry point that can run a block, so a new command
/// cannot forget it.
fn auto_lock_gate(cfg: &config::Config) -> Result<(), i32> {
    if cfg.mode != config::Mode::Auto {
        return Ok(());
    }
    let (unlocked, need) = holds::auto_lock(&cfg.state_dir(), cfg);
    if unlocked {
        return Ok(());
    }
    eprintln!(
        "tgw: mode=auto is locked: {} more approval(s) needed ({} of {}).",
        need,
        holds::approvals(&cfg.state_dir()),
        cfg.auto_unlock_approvals
    );
    eprintln!("Approve that many holds, or set mode=confirm.");
    Err(2)
}

/// Prints what happened, for `once`.
fn report_once(ctx: &mut Context<'_>) -> i32 {
    let out = pipeline::process_once(ctx);
    match out.verdict {
        tgw::log::Verdict::Ignore => {
            println!("ignore: {}", out.reason);
            0
        }
        v => {
            println!("{}: {}", v.as_str(), out.reason);
            println!("id: {}  ({} ms)", out.id, out.duration_ms);
            if let Some(s) = &out.staged {
                println!("staged: {}", s.path.display());
            }
            0
        }
    }
}

/// Prints a one-line notice for a non-ignore verdict.
fn announce(cfg: &config::Config, out: &pipeline::Outcome) {
    if out.verdict == tgw::log::Verdict::Ignore {
        return;
    }
    eprintln!("tgw: {}", tgw::log::summary(out.verdict, &out.reason, &out.id));
    notify(cfg, out.verdict.as_str(), &out.id);
}

/// `tgw approve ID`: re-check a hold, then run it.
///
/// Every check that made the hold a hold is repeated. The human is approving
/// *the block they saw*, and the only way to know this is the one they saw is
/// to hash it again and ask the gate again.
fn cmd_approve(rest: &[String]) -> i32 {
    if rest.len() != 1 {
        return usage("approve", "approve needs exactly one ID");
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    let state = cfg.state_dir();
    let log = tgw::log::Log::new(cfg.log_dir(), 200);

    // `hold_rules` and the `auto` lock are both reasons a pass may not simply
    // run; approval is the human saying yes, so neither applies here.
    let hold = match holds::load(&state, &rest[0]) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("tgw: {}", e.reason());
            return 1;
        }
    };

    let verified = holds::verify(&cfg, &state, &hold, |path| tgw::gate::check(&cfg, path));
    let path = match verified {
        Ok(p) => p,
        Err(refusal) => {
            let mut entry = tgw::log::Entry::new(Verdict::Hold, format!("approval refused: {}", refusal.reason()));
            entry.id = hold.id.clone();
            entry.hash = hold.hash;
            let _ = log.append(&entry);
            eprintln!("tgw: refused: {}", refusal.reason());
            return 1;
        }
    };

    let run = match tgw::execute::run_block(&cfg, &path) {
        Ok(r) => r,
        Err(e) => {
            let mut entry = tgw::log::Entry::new(Verdict::Block, e.reason());
            entry.id = hold.id.clone();
            entry.hash = hold.hash;
            let _ = log.append(&entry);
            eprintln!("tgw: {}", e.reason());
            return 1;
        }
    };

    let mut entry = tgw::log::Entry::new(
        if run.success() { Verdict::Run } else { Verdict::Hold },
        format!("approved and {}", if run.timed_out { "timed out".to_string() } else { format!("exited {}", run.exit.map(|c| c.to_string()).unwrap_or_else(|| "on a signal".into())) }),
    );
    entry.id = hold.id.clone();
    entry.hash = hold.hash;
    entry.exit = run.exit;
    entry.duration_ms = run.duration_ms;
    let _ = log.append(&entry);

    let mut out = String::new();
    out.push_str(&String::from_utf8_lossy(&run.stdout));
    if !run.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&run.stderr));
    }
    print!("{out}");
    if run.truncated {
        eprintln!("tgw: output truncated at {} bytes", cfg.max_output_bytes);
    }

    if run.success() {
        holds::remove(&state, &hold);
        let _ = holds::record_approval(&state);
        notify(&cfg, "tgw: ran", &hold.id);
        return 0;
    }

    // A failed run keeps the hold: the human may want to see what went wrong,
    // and deleting it silently would lose the only copy they have.
    eprintln!(
        "tgw: the block failed; the hold is kept. `tgw deny {}` to discard it.",
        hold.id
    );
    1
}

/// `tgw deny ID`: discard a hold. Nothing runs, ever.
fn cmd_deny(rest: &[String]) -> i32 {
    if rest.len() != 1 {
        return usage("deny", "deny needs exactly one ID");
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    let state = cfg.state_dir();
    let hold = match holds::load(&state, &rest[0]) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("tgw: {}", e.reason());
            return 1;
        }
    };
    let mut entry = tgw::log::Entry::new(Verdict::Block, format!("denied by human: {}", hold.reason));
    entry.id = hold.id.clone();
    entry.hash = hold.hash;
    let _ = tgw::log::Log::new(cfg.log_dir(), 200).append(&entry);
    holds::remove(&state, &hold);
    println!("denied: {}", hold.id);
    0
}

/// Runs `notify_cmd`, substituting `{id}` and `{status}`. Failure is ignored:
/// a notification is a courtesy, not a gate.
fn notify(cfg: &config::Config, status: &str, id: &str) {
    if cfg.notify_cmd.trim().is_empty() {
        return;
    }
    let Ok(argv) = config::split_argv(&cfg.notify_cmd) else {
        return;
    };
    let argv = config::substitute(&argv, &[("id", id), ("status", status)]);
    if argv.is_empty() {
        return;
    }
    let _ = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn cmd_status(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("status", "status takes no arguments");
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    let state = cfg.state_dir();
    let (unlocked, still_needed) = holds::auto_lock(&state, &cfg);
    println!("mode: {}", cfg.mode.as_str());
    match (cfg.mode, unlocked) {
        (tgw::config::Mode::Auto, false) => println!(
            "note: mode=auto is configured but locked; {} more approval(s) unlock it",
            still_needed
        ),
        (tgw::config::Mode::Auto, true) => {
            println!("auto unlock: unlocked ({} approvals)", holds::approvals(&state))
        }
        _ => println!(
            "auto unlock: {}/{} approvals",
            holds::approvals(&state),
            cfg.auto_unlock_approvals
        ),
    }
    println!("state: {}", state.display());
    println!("log: {}", cfg.log_dir().display());
    println!("hold_rules: {}", if cfg.hold_rules.is_empty() { "(none)".to_string() } else { cfg.hold_rules.join(", ") });

    let pending = holds::pending(&state);
    println!("pending holds: {}", pending.len());
    let now = tgw::log::now_unix();
    for h in &pending {
        println!(
            "  {}  {}s left  mode={}  {}",
            h.id,
            h.expires.saturating_sub(now),
            h.mode.as_str(),
            h.reason
        );
    }
    0
}

fn cmd_log(rest: &[String]) -> i32 {
    let mut tail: usize = 20;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--tail" => {
                i += 1;
                match rest.get(i).and_then(|v| v.parse::<usize>().ok()) {
                    Some(n) => tail = n,
                    None => return usage("log", "--tail needs a number"),
                }
            }
            other => return usage("log", &format!("unknown option `{other}`")),
        }
        i += 1;
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    let file = cfg.log_dir().join("events.log");
    let Ok(text) = std::fs::read_to_string(&file) else {
        println!("(no log at {})", file.display());
        return 0;
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(tail);
    for line in &lines[start..] {
        println!("{line}");
    }
    0
}

/// `tgw doctor`: prove the pieces work before trusting them with a block.
///
/// Every check is something that would otherwise fail silently in daily use:
/// a clipboard command that prints nothing, a gate that passes everything, a
/// `hold_rules` name that no longer exists after a `tgr` upgrade, a notify
/// command that cannot run, a state dir that cannot be written.
fn cmd_doctor(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("doctor", "doctor takes no arguments");
    }
    let (cfg, path) = config::load();
    let mut failed = 0;
    let mut warn = 0;
    let ok = |m: &str| println!("ok:    {m}");
    let bad = |failed: &mut usize, m: String| {
        println!("FAIL:  {m}");
        *failed += 1;
    };

    match &path {
        Some(p) if p.exists() => ok(&format!("config {}", p.display())),
        Some(p) => {
            bad(&mut failed, format!("no config at {}", p.display()));
        }
        None => bad(
            &mut failed,
            "no config directory (set TCX_HOME or HOME)".into(),
        ),
    }
    for p in cfg.validate() {
        bad(&mut failed, p);
    }
    if failed > 0 {
        // The remaining checks would test a config that cannot run.
        println!();
        println!("{failed} problem(s); fix them and run `tgw doctor` again.");
        return 1;
    }

    // State directories.
    let before = tgw::run::check_state_dir(&cfg);
    if before.is_empty() {
        ok(&format!("state dir {}", cfg.state_dir().display()));
        ok(&format!("log dir {}", cfg.log_dir().display()));
    } else {
        for p in before {
            bad(&mut failed, p);
        }
    }

    // Clipboard: it must run *and* succeed. An empty answer is normal (the
    // clipboard may be empty), but a non-zero exit or a timeout is not.
    let read = tgw::clip::read_clipboard(&cfg, std::time::Duration::from_secs(5));
    match &read {
        Ok(c) => ok(&format!(
            "clipboard read returned {} byte(s) in {} ms",
            c.bytes.len(),
            c.elapsed.as_millis()
        )),
        Err(e) => bad(
            &mut failed,
            format!("clip_get_cmd did not produce a clipboard: {}", e.reason()),
        ),
    }

    // Gate: it must tell a clean sample from a known-bad one. A gate that
    // passes both is not a gate.
    let stage = cfg.state_dir().join("doctor");
    let _ = std::fs::create_dir_all(&stage);
    let clean = stage.join("clean.sh");
    let dangerous = stage.join("dangerous.sh");
    let clean_text = "set +H\necho hello\n";
    let danger_text = "set +H\nrm -rf /tmp/tgw-doctor-should-not-exist\n";
    let clean_res = std::fs::write(&clean, clean_text)
        .map_err(|e| e.to_string())
        .and_then(|_| Ok(tgw::gate::check(&cfg, &clean)));
    let danger_res = std::fs::write(&dangerous, danger_text)
        .map_err(|e| e.to_string())
        .and_then(|_| Ok(tgw::gate::check(&cfg, &dangerous)));
    match (&clean_res, &danger_res) {
        (Ok(g), _) if !matches!(g, tgw::pipeline::GateResult::Pass { .. }) => bad(
            &mut failed,
            format!(
                "the gate did not pass a clean sample (`echo hello`): {g:?} — is gate_cmd looking at {{file}}?"
            ),
        ),
        (Err(e), _) => bad(&mut failed, format!("cannot write a doctor sample: {e}")),
        (_, Ok(tgw::pipeline::GateResult::Block { reason })) => {
            ok(&format!("gate blocks a known-bad sample ({reason})"))
        }
        (_, Ok(other)) => bad(
            &mut failed,
            format!("the gate did not block `rm -rf /tmp/...`: {other:?} — it may not be the gate you think"),
        ),
        (_, Err(e)) => bad(&mut failed, format!("cannot write a doctor sample: {e}")),
    }
    // `hold_rules` must name rules that exist. A typo means a hold that
    // silently never happens.
    match gate_rule_names(&cfg) {
        Some(names) if !names.is_empty() => {
            let unknown: Vec<&str> = cfg
                .hold_rules
                .iter()
                .filter(|r| !names.contains(r))
                .map(|r| r.as_str())
                .collect();
            if unknown.is_empty() {
                ok(&format!(
                    "hold_rules ({} name(s)) all exist in the gate",
                    cfg.hold_rules.len()
                ));
            } else {
                bad(
                    &mut failed,
                    format!(
                        "hold_rules names not reported by `--list-rules`: {}",
                        unknown.join(", ")
                    ),
                );
            }
        }
        Some(names) => bad(
            &mut failed,
            format!(
                "the gate answered `--list-rules` with no rules ({} line(s)); it may not be the gate you think",
                names.len()
            ),
        ),
        None => bad(
            &mut failed,
            "the gate cannot be asked for its rules, so hold_rules cannot be checked".into(),
        ),
    }
    let _ = std::fs::remove_file(&clean);
    let _ = std::fs::remove_file(&dangerous);
    let _ = std::fs::remove_dir(&stage);

    // Notify: optional, but if configured it must run.
    if cfg.notify_cmd.trim().is_empty() {
        println!("skip:  notify_cmd is not set (verdicts go to the terminal only)");
    } else {
        let before = std::time::Instant::now();
        let mut child = config::split_argv(&cfg.notify_cmd).ok().and_then(|a| {
            if a.is_empty() {
                return None;
            }
            let a = config::substitute(&a, &[("id", "doctor"), ("status", "doctor")]);
            std::process::Command::new(&a[0])
                .args(&a[1..])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()
        });
        match child.as_mut().map(|c| c.wait()) {
            Some(Ok(st)) if st.success() => ok(&format!(
                "notify_cmd ran in {} ms",
                before.elapsed().as_millis()
            )),
            Some(Ok(st)) => {
                println!("warn:  notify_cmd exited {} (a notification is a courtesy)", st);
                warn += 1;
            }
            _ => {
                println!("warn:  notify_cmd could not run (a notification is a courtesy)");
                warn += 1;
            }
        }
    }

    // Wakelock: optional. Presence is reported, never started, because taking
    // one here would change the machine the human is testing on.
    if cfg.wakelock_cmd.trim().is_empty() {
        println!("skip:  wakelock_cmd is not set");
    } else {
        let head = config::split_argv(&cfg.wakelock_cmd)
            .ok()
            .and_then(|a| a.into_iter().next());
        match head {
            Some(prog) if which(&prog) => ok(&format!("wakelock program {prog} exists")),
            Some(prog) => {
                println!("warn:  wakelock program {prog} not on PATH");
                warn += 1;
            }
            None => {
                println!("warn:  wakelock_cmd does not parse");
                warn += 1;
            }
        }
    }

    // Mode.
    match cfg.mode {
        tgw::config::Mode::Auto => {
            let (unlocked, need) = holds::auto_lock(&cfg.state_dir(), &cfg);
            if unlocked {
                ok("mode=auto is unlocked");
            } else {
                println!(
                    "warn:  mode=auto is locked ({} more approval(s)); `run` will refuse to start",
                    need
                );
                warn += 1;
            }
        }
        m => ok(&format!("mode={}", m.as_str())),
    }

    println!();
    if failed > 0 {
        println!("{failed} failure(s), {warn} warning(s).");
        1
    } else if warn > 0 {
        println!("all required checks passed, {warn} warning(s).");
        0
    } else {
        println!("all checks passed.");
        0
    }
}

/// Asks the gate for its rule names, so `hold_rules` can be checked.
///
/// `gate_cmd` is a *checking* command (`tgr --check {file}`), and `tgr`
/// rejects `--check` together with `--list-rules`. So the checking flags and
/// the `{file}` token are dropped and `--list-rules` is asked on its own,
/// against the same program. A gate that cannot be asked that way returns
/// `None`, which `doctor` reports rather than passing silently.
fn gate_rule_names(cfg: &config::Config) -> Option<Vec<String>> {
    let argv = config::split_argv(&cfg.gate_cmd).ok()?;
    if argv.is_empty() {
        return None;
    }
    let mut ask: Vec<String> = vec![argv[0].clone()];
    let mut skip_next = false;
    for a in &argv[1..] {
        if skip_next {
            skip_next = false;
            continue;
        }
        if a == "--check" || a == "-c" {
            // `--check PATH`: drop the flag and its value.
            skip_next = true;
            continue;
        }
        if a.contains("{file}") {
            continue;
        }
        ask.push(a.clone());
    }
    if ask.iter().any(|a| a == "--list-rules") {
        return None;
    }
    let out = std::process::Command::new(&ask[0])
        .args(&ask[1..])
        .arg("--list-rules")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
    )
}

fn which(prog: &str) -> bool {
    if prog.contains('/') {
        return std::path::Path::new(prog).exists();
    }
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
        .unwrap_or(false)
}


/// `tgw cd PATH`: writes the directory `cwd_mode = file` will read.
///
/// Refuses when `TGW_IN_BLOCK` is set. That is an **accident guard, not a
/// boundary**: a block can still write the file directly. It exists so a block
/// that happens to say `tgw cd /` does not silently redirect every later block,
/// and so the human finds out immediately instead of after the next run.
fn cmd_cd(rest: &[String]) -> i32 {
    if rest.len() != 1 {
        return usage("cd", "cd needs exactly one PATH");
    }
    if std::env::var_os(tgw::execute::IN_BLOCK).is_some() {
        eprintln!("tgw: cd refuses to run inside a block; a block cannot move later blocks");
        return 2;
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    let path = std::path::Path::new(&rest[0]);
    if !path.is_absolute() {
        eprintln!(
            "tgw: cd needs an absolute path, got `{}` (tgw has no shell to resolve it against)",
            rest[0]
        );
        return 2;
    }
    if !path.is_dir() {
        eprintln!("tgw: not a directory: {}", path.display());
        return 2;
    }
    if cfg.cwd_mode != tgw::config::CwdMode::File {
        eprintln!(
            "tgw: cwd_mode is `{}`, so cwd_file is not read; set `cwd_mode = file` first",
            cfg.cwd_mode.as_str()
        );
        return 2;
    }
    // Write into the state dir then rename: a reader never sees half a path.
    let target = std::path::Path::new(&cfg.cwd_file);
    let tmp = target.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, format!("{}\n", path.display())) {
        eprintln!("tgw: cannot write {}: {e}", tmp.display());
        return 1;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("tgw: cannot write {}: {e}", target.display());
        return 1;
    }
    println!("cd: {}", path.display());
    0
}
