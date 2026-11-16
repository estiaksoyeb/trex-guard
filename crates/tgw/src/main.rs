//! The `tgw` command.

use std::process::exit;

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
    for a in rest {
        match a.as_str() {
            // Accepted and documented; the loop runs once then exits.
            "--once" => {}
            other => return usage("run", &format!("unknown option `{other}`")),
        }
    }
    if load_config().is_none() {
        return 2;
    }
    eprintln!("tgw: the watch loop is not built yet (slice 2)");
    1
}

fn cmd_once(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("once", "once takes no arguments");
    }
    if load_config().is_none() {
        return 2;
    }
    eprintln!("tgw: processing is not built yet (slice 2)");
    1
}

fn cmd_approve(rest: &[String]) -> i32 {
    if rest.len() != 1 {
        return usage("approve", "approve needs exactly one ID");
    }
    if load_config().is_none() {
        return 2;
    }
    eprintln!("tgw: holds are not built yet (slice 5)");
    1
}

fn cmd_deny(rest: &[String]) -> i32 {
    if rest.len() != 1 {
        return usage("deny", "deny needs exactly one ID");
    }
    if load_config().is_none() {
        return 2;
    }
    eprintln!("tgw: holds are not built yet (slice 5)");
    1
}

fn cmd_status(rest: &[String]) -> i32 {
    if !rest.is_empty() {
        return usage("status", "status takes no arguments");
    }
    let Some(cfg) = load_config() else {
        return 2;
    };
    println!("mode: {}", cfg.mode.as_str());
    println!("state: {}", cfg.state_dir().display());
    println!("log: {}", cfg.log_dir().display());
    println!("pending holds: 0");
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

fn cmd_cd(rest: &[String]) -> i32 {
    if rest.len() != 1 {
        return usage("cd", "cd needs exactly one PATH");
    }
    if load_config().is_none() {
        return 2;
    }
    eprintln!("tgw: cd is not built yet (slice 4)");
    1
}
