//! Help text, generated from one table.
//!
//! `tgw --help` and `tgw <command> --help` must never disagree, so both are
//! rendered from [`COMMANDS`]. Adding a command to [`COMMANDS`] is what makes
//! `tests/help.rs` check it; adding a command to the dispatcher without adding
//! it here is caught by the same test, in both directions.

/// One command's documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    pub summary: &'static str,
    /// Usage lines, without the `tgw ` prefix.
    pub usage: &'static [&'static str],
    /// Options, each starting with its flags.
    pub options: &'static [&'static str],
    /// Notes worth stating: defaults, unusual behaviour, safety rules.
    pub notes: &'static [&'static str],
}

/// Every subcommand, in the order `--help` lists them.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "run",
        summary: "Watch the clipboard until stopped",
        usage: &["run [--once] [--interval MS]"],
        options: &[
            "--once            Process the current clipboard once, then exit",
            "--interval MS     Override poll_ms for this run",
        ],
        notes: &[
            "The loop never exits on a clipboard error; it backs off and retries.",
            "In `dry` mode nothing is ever executed, whatever the gate says.",
        ],
    },
    Command {
        name: "once",
        summary: "Process the current clipboard once",
        usage: &["once"],
        options: &[],
        notes: &["Same pipeline as `run`, one iteration. Useful for testing."],
    },
    Command {
        name: "approve",
        summary: "Run a block that was held",
        usage: &["approve ID"],
        options: &[],
        notes: &[
            "The staged file is re-hashed and re-gated first; if either changed",
            "since the hold, the approval is refused.",
            "A `block` verdict can never be approved, only a `hold`.",
        ],
    },
    Command {
        name: "deny",
        summary: "Discard a held block",
        usage: &["deny ID"],
        options: &[],
        notes: &["The staged file is deleted. Nothing runs."],
    },
    Command {
        name: "status",
        summary: "Show mode, pending holds and the last verdicts",
        usage: &["status"],
        options: &[],
        notes: &[
            "Shows the approval count that unlocks `mode = auto`, and any",
            "denies, so the lock is visible rather than implied.",
        ],
    },
    Command {
        name: "log",
        summary: "Print recent log entries",
        usage: &["log [--tail N]"],
        options: &["--tail N   Show the last N entries (default: 20)"],
        notes: &["One line per event: ts | id | hash | verdict | reason | exit | duration_ms"],
    },
    Command {
        name: "doctor",
        summary: "Check that the clipboard, gate and notify commands work",
        usage: &["doctor"],
        options: &[],
        notes: &[
            "Fails if the gate cannot tell a clean sample from a known-bad one.",
            "Also checks that the gate reports its rule names, so it is the",
            "gate you think it is.",
        ],
    },
    Command {
        name: "check-config",
        summary: "Validate watch.ini and print every problem",
        usage: &["check-config"],
        options: &[],
        notes: &["Prints the path it read, then each problem on its own line."],
    },
    Command {
        name: "cd",
        summary: "Write the directory used by cwd_mode = file",
        usage: &["cd PATH"],
        options: &[],
        notes: &[
            "Refuses to run inside a block (TGW_IN_BLOCK is set), so a gated",
            "block cannot redirect where later blocks run.",
        ],
    },
];

/// The top-level help.
pub fn render_all() -> String {
    let mut out = String::new();
    out.push_str("tgw - TREX Guard Watchdog - gate clipboard blocks and run them\n\n");
    out.push_str("Usage:\n");
    out.push_str("  tgw <command> [options]\n");
    out.push_str("  tgw --help\n");
    out.push_str("  tgw --version\n\n");
    out.push_str("Commands:\n");
    let width = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0);
    for c in COMMANDS {
        out.push_str(&format!("  {:<width$}  {}\n", c.name, c.summary, width = width));
    }
    out.push_str("\nExit codes:\n");
    out.push_str("  0  success\n");
    out.push_str("  1  runtime error\n");
    out.push_str("  2  usage error\n");
    out.push_str("\nConfig: $TCX_HOME/config/watch.ini, else ~/.config/tcx/watch.ini\n");
    out.push_str("It is never read from the current directory or the repository.\n");
    out.push_str("`tgw <command> --help` documents each command.\n");
    out
}

/// One command's help, rendered from its [`Command`] entry.
pub fn render_command(c: &Command) -> String {
    let mut out = format!("tgw {} - {}\n\nUsage:\n", c.name, c.summary);
    for u in c.usage {
        out.push_str(&format!("  tgw {u}\n"));
    }
    if !c.options.is_empty() {
        out.push_str("\nOptions:\n");
        for o in c.options {
            out.push_str(&format!("  {o}\n"));
        }
    }
    if !c.notes.is_empty() {
        out.push_str("\nNotes:\n");
        for n in c.notes {
            out.push_str(&format!("  {n}\n"));
        }
    }
    out
}

/// Looks a command up by name.
pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_usage_and_a_summary() {
        for c in COMMANDS {
            assert!(!c.summary.is_empty(), "{} has no summary", c.name);
            assert!(!c.usage.is_empty(), "{} has no usage", c.name);
            for u in c.usage {
                let ok = *u == c.name || u.starts_with(&format!("{} ", c.name));
                assert!(ok, "{} usage {u:?} names another command", c.name);
            }
        }
    }

    #[test]
    fn names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate command: {}", c.name);
        }
    }

    #[test]
    fn the_top_level_help_lists_every_command() {
        let text = render_all();
        for c in COMMANDS {
            assert!(text.contains(c.name), "top-level help omits {}", c.name);
        }
        assert!(text.contains("0  success"));
        assert!(text.contains("2  usage error"));
    }

    #[test]
    fn command_help_matches_the_table() {
        for c in COMMANDS {
            let text = render_command(c);
            assert!(text.starts_with(&format!("tgw {} - {}", c.name, c.summary)));
            assert!(text.contains("Usage:"));
            for o in c.options {
                assert!(text.contains(o), "{} help omits option {o:?}", c.name);
            }
        }
    }
}
