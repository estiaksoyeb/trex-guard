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
fn load_config() -> Option<config::Config> {
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
    if cfg.mode == config::Mode::Auto {
        let (unlocked, need) = holds::auto_lock(&cfg.state_dir(), &cfg);
        if !unlocked {
            eprintln!(
                "tgw: mode=auto is locked: {} more approval(s) needed ({} of {}).",
                need,
                holds::approvals(&cfg.state_dir()),
                cfg.auto_unlock_approvals
            );
            eprintln!("Approve that many holds, or set mode=confirm.");
            return 2;
        }
    }

    // The whole loop runs through the same `process_once` as `tgw once`, so
    // there is exactly one pipeline in the program.
    let mut seen = Seen::new(16);
    let mut ctx = Context {
        cfg: &cfg,
        log: &Log::new(cfg.log_dir(), 200),
        seen: &mut seen,
    };
    if let Err(e) = tgw::clip::ensure_state_dir(&cfg.state_dir()) {
        eprintln!("tgw: state dir {}: {e}", cfg.state_dir().display());
        return 1;
    }

    if once {
        return report_once(&mut ctx);
    }

    eprintln!(
        "tgw: watching (mode={}, poll={} ms). Ctrl-C to stop.",
        cfg.mode.as_str(),
        cfg.poll_ms
    );
    loop {
        let out = pipeline::process_once(&mut ctx);
        announce(&cfg, &out);
        std::thread::sleep(std::time::Duration::from_millis(cfg.poll_ms.max(50)));
    }
}

fn cmd_once(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("once", "once takes no arguments");
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    let mut seen = Seen::new(16);
    let mut ctx = Context {
        cfg: &cfg,
        log: &Log::new(cfg.log_dir(), 200),
        seen: &mut seen,
    };
    if let Err(e) = tgw::clip::ensure_state_dir(&cfg.state_dir()) {
        eprintln!("tgw: state dir {}: {e}", cfg.state_dir().display());
        return 1;
    }
    report_once(&mut ctx)
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

fn cmd_doctor(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("doctor", "doctor takes no arguments");
    }
    eprintln!("tgw: doctor is not built yet (slice 6)");
    1
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
