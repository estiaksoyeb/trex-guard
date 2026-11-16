//! Help text, generated from one table.
//!
//! `tcx --help` and `tcx <command> --help` must never disagree, so both are
//! rendered from [`COMMANDS`]. Adding a command to [`COMMANDS`] is what makes
//! `tests/help.rs` check it; adding a command to the dispatcher without adding
//! it here is caught by the same test.
//!
//! This module is the single source of truth for the command surface. If you
//! teach the dispatcher a new command, add it here or the test suite fails.

/// One command's documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// The subcommand name.
    pub name: &'static str,
    /// One-line summary used in the top-level list.
    pub summary: &'static str,
    /// Usage lines, without the `tcx ` prefix.
    pub usage: &'static [&'static str],
    /// Options, each starting with its flags.
    pub options: &'static [&'static str],
    /// Notes worth stating: defaults, unusual behaviour, safety rules.
    pub notes: &'static [&'static str],
}

/// Every subcommand, in the order `--help` lists them.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "id",
        summary: "Resolve the project identity and its store path",
        usage: &["id [--human] [PATH]"],
        options: &["--human       Human-readable labels instead of one machine line"],
        notes: &[
            "Identity is derived from git root commits, never from the path, so",
            "worktrees, clones and moves share one store.",
            "Outside git, identity is the directory you are in (subdirectories",
            "get separate stores).",
        ],
    },
    Command {
        name: "note",
        summary: "Append a dated note (decision|gotcha|open)",
        usage: &[
            "note [-k KIND] [-f FILE]... [-t TAG]... [--path PATH] TEXT",
            "note --list [--path PATH]",
        ],
        options: &[
            "-k, --kind KIND   decision (default) | gotcha | open",
            "-f, --file FILE   File the note is about; repeatable",
            "-t, --tag TAG     Keyword for scoped search; repeatable",
            "--list            Print every note instead of appending one",
            "--path, -p PATH   Project to resolve (default: .)",
        ],
        notes: &[
            "Notes are one line each, appended and never rewritten. The date is",
            "taken from the system clock, so callers never supply one.",
            "The store lives outside the repository.",
        ],
    },
    Command {
        name: "prime",
        summary: "Print a budgeted session briefing for pasting into a chat",
        usage: &["prime [--budget BYTES] [PATH]"],
        options: &["-b, --budget BYTES   Byte cap on the notes (default: 6000)"],
        notes: &[
            "Prints a freshness stamp, then notes in order: gotchas, then open",
            "questions, then decisions newest-first.",
            "`resume` prints the same notes as part of the prompt; `prime` is the",
            "briefing on its own.",
        ],
    },
    Command {
        name: "find",
        summary: "Search notes; -k scopes by keyword, --tags lists keywords",
        usage: &["find [-k KEYWORD]... [--tags] [QUERY...] [PATH]"],
        options: &[
            "-k, --kind KEYWORD   Only notes carrying this keyword; repeatable",
            "--tags               List the project's keywords with counts",
            "--path, -p PATH      Project to resolve (default: .)",
        ],
        notes: &[
            "Keywords are matched before the query, so -k narrows what QUERY",
            "then searches. Matching is case-insensitive substring, not regex.",
        ],
    },
    Command {
        name: "import",
        summary: "Import an existing .ai/ directory (never modifies it)",
        usage: &["import [--dir DIR] [PATH]"],
        options: &["-d, --dir DIR        Source directory (default: <project>/.ai)"],
        notes: &[
            "The source directory is only read. Importing twice is safe:",
            "duplicate notes are skipped.",
        ],
    },
    Command {
        name: "fresh",
        summary: "Print the prompt with ## Project filled in from this repo",
        usage: &["fresh [--template FILE] [PATH]"],
        options: &[
            "-T, --template FILE   Use this template instead of the config copy",
            "--path, -p PATH       Project to resolve (default: .)",
        ],
        notes: &[
            "The prompt goes to stdout; the template path used goes to stderr,",
            "so `tg fresh | clip` copies only the prompt.",
            "The template is read only from your config directory, never from the",
            "repository: --template, then $TCX_HOME/config/prompt.md, then",
            "<config>/tcx/prompt.md.",
            "Prints project details and nothing else; `resume` adds the notes.",
        ],
    },
    Command {
        name: "resume",
        summary: "As fresh, plus what previous sessions learned",
        usage: &["resume [--budget BYTES] [--template FILE] [PATH]"],
        options: &[
            "-b, --budget BYTES    Byte cap on the notes (default: 6000)",
            "-T, --template FILE   Use this template instead of the config copy",
            "--path, -p PATH       Project to resolve (default: .)",
        ],
        notes: &[
            "Same prompt as `fresh`, then `## What previous sessions learned`",
            "built from the store in the same order and budget as `prime`.",
        ],
    },
];

/// Exit codes are part of the interface; scripts branch on them.
pub const EXIT_CODES: &[(&str, &str)] = &[
    ("0", "success"),
    ("1", "runtime error (missing template, unreadable store)"),
    ("2", "usage error (unknown command or option, nothing to do)"),
];

/// Renders the top-level help.
pub fn overall() -> String {
    let mut out = String::from("tcx - TREX ConteXt\n\nUsage:\n");
    for command in COMMANDS {
        for usage in command.usage {
            out.push_str("  tcx ");
            out.push_str(usage);
            out.push('\n');
        }
    }
    out.push_str("  tcx --help | --version\n\nCommands:\n");
    for command in COMMANDS {
        out.push_str(&format!("  {:<7} {}\n", command.name, command.summary));
    }

    out.push_str("\nExit codes:\n");
    for (code, meaning) in EXIT_CODES {
        out.push_str(&format!("  {code}  {meaning}\n"));
    }

    out.push_str("\nEnvironment:\n");
    out.push_str("  TCX_HOME   Single root containing data/, cache/ and config/\n");
    out.push_str("  XDG_DATA_HOME, XDG_CACHE_HOME, XDG_CONFIG_HOME\n");
    out.push_str("             Used when set; otherwise ~/.local/share, ~/.cache, ~/.config\n");
    out.push_str("\nRun `tcx <command> --help` for one command's options.\n");
    out
}

/// Renders the help for one subcommand.
pub fn for_command(name: &str) -> Option<String> {
    let command = COMMANDS.iter().find(|c| c.name == name)?;

    let mut out = format!("tcx {} - {}\n\nUsage:\n", command.name, command.summary);
    for usage in command.usage {
        out.push_str("  tcx ");
        out.push_str(usage);
        out.push('\n');
    }

    if !command.options.is_empty() {
        out.push_str("\nOptions:\n");
        for option in command.options {
            out.push_str(&format!("  {option}\n"));
        }
    }

    if !command.notes.is_empty() {
        out.push_str("\nNotes:\n");
        for note in command.notes {
            out.push_str(&format!("  {note}\n"));
        }
    }
    Some(out)
}

/// Whether `args` asks for help (`--help`, `-h`, `help`).
pub fn wants_help(args: &[String]) -> bool {
    args.iter()
        .any(|a| matches!(a.as_str(), "--help" | "-h" | "help"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_documented() {
        for command in COMMANDS {
            assert!(!command.summary.is_empty(), "{} has no summary", command.name);
            assert!(!command.usage.is_empty(), "{} has no usage", command.name);
            let help = for_command(command.name).unwrap();
            assert!(help.contains("Usage:"));
            assert!(help.contains("tcx "));
        }
    }

    #[test]
    fn overall_lists_every_command() {
        let help = overall();
        for command in COMMANDS {
            assert!(
                help.contains(&format!("tcx {}", command.usage[0])),
                "{} missing from usage",
                command.name
            );
            assert!(
                help.contains(command.summary),
                "{} summary missing",
                command.name
            );
        }
    }

    #[test]
    fn unknown_command_has_no_help() {
        assert!(for_command("nope").is_none());
    }

    #[test]
    fn help_detection_accepts_the_usual_spellings() {
        for spelling in ["--help", "-h", "help"] {
            assert!(wants_help(&[spelling.to_owned()]));
        }
        assert!(!wants_help(&["--template".to_owned()]));
        assert!(!wants_help(&[]));
    }
}
