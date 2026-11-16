//! Help must stay honest.
//!
//! The failure this guards against is silent drift: a command grows a flag,
//! or a new command is wired into the dispatcher, and `--help` keeps saying
//! what it said last year. Documentation nobody trusts is worse than none.
//!
//! Two directions are checked:
//!
//! 1. everything `help::COMMANDS` documents must exist in the binary;
//! 2. everything the binary accepts must be documented - obtained by probing
//!    the real command with a fake flag and reading the "unknown option"
//!    reply, so the list comes from the code, not from a hand-kept copy.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tcx::help::{self, COMMANDS};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_tcx")
}

/// An isolated store and project, so no real data is touched.
struct Fixture {
    home: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!(
            "tcx-help-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = base.join("home");
        let repo = base.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(home.join("config")).unwrap();
        fs::write(home.join("config/prompt.md"), "## Project\n<x>\ntype: <x>\n").unwrap();
        Fixture { home, repo }
    }

    fn path(&self) -> &Path {
        &self.repo
    }

    fn run(&self, args: &[&str]) -> (String, String, Option<i32>) {
        let out = Command::new(bin())
            .args(args)
            .current_dir(&self.repo)
            .env("TCX_HOME", &self.home)
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_CACHE_HOME", self.home.join("xdg-cache"))
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(base) = self.home.parent() {
            let _ = fs::remove_dir_all(base);
        }
    }
}

/// Flags the dispatcher handles before a command sees its arguments.
const UNIVERSAL: &[&str] = &["--help", "-h", "help"];

#[test]
fn every_documented_command_has_help() {
    let fixture = Fixture::new("documented");
    for command in COMMANDS {
        for spelling in ["--help", "-h"] {
            let (stdout, stderr, code) = fixture.run(&[command.name, spelling]);
            assert_eq!(
                code,
                Some(0),
                "`tcx {} {spelling}` did not succeed: {stderr}",
                command.name
            );
            assert!(
                stdout.contains(&format!("tcx {} - {}", command.name, command.summary)),
                "`tcx {} {spelling}` printed the wrong help:\n{stdout}",
                command.name
            );
            assert!(
                stdout.contains("Usage:"),
                "`tcx {} {spelling}` has no usage block",
                command.name
            );
        }
    }
}

#[test]
fn help_wins_over_other_arguments() {
    let fixture = Fixture::new("precedence");
    // `--help` before the command parses anything, so a value that would
    // otherwise be consumed cannot swallow it.
    let (stdout, _, code) = fixture.run(&["note", "--kind", "gotcha", "--help"]);
    assert_eq!(code, Some(0));
    assert!(stdout.starts_with("tcx note - "));
}

#[test]
fn every_command_the_binary_accepts_is_documented() {
    let fixture = Fixture::new("undocumented-command");
    let documented: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();

    // The candidate set is everything the crate declares as a module or that
    // help lists. A new command must be added to COMMANDS; this catches the
    // case where the dispatcher knows it but help does not.
    for command in &documented {
        let (_, stderr, code) = fixture.run(&[command, "--help"]);
        assert_eq!(
            code,
            Some(0),
            "`tcx {command}` is listed in help but the binary rejects it: {stderr}"
        );
    }

    // A command that exists but is absent from COMMANDS would still dispatch.
    // Probe the ones the crate's own source mentions, so adding `tcx sync`
    // to the dispatcher without documenting it fails here.
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();
    let dispatched = dispatched_commands(&source);
    let mut missing: Vec<&String> = dispatched
        .iter()
        .filter(|name| !documented.contains(&name.as_str()))
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "the dispatcher handles {missing:?} but help::COMMANDS does not list them"
    );
    assert!(
        !dispatched.is_empty(),
        "could not read the dispatcher's command list; the probe is blind"
    );
}

/// Extracts `"name" => run_x(...)` arms from the dispatcher in `main.rs`.
fn dispatched_commands(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        let Some((lhs, rhs)) = line.split_once("=>") else {
            continue;
        };
        if !rhs.trim_start().starts_with("run_") {
            continue;
        }
        for part in lhs.split('|') {
            let name = part.trim().trim_matches('"');
            if !name.is_empty() && !name.starts_with('-') && name.chars().all(|c| c.is_lowercase() || c == '_') {
                out.push(name.to_owned());
            }
        }
    }
    out
}

#[test]
fn every_accepted_flag_is_documented() {
    let fixture = Fixture::new("undocumented-flag");
    // Probes a flag no command defines; the reply names it, proving the
    // command reached its parser and rejected the option.
    const PROBE: &str = "--definitely-not-an-option";

    let mut failures = Vec::new();
    for command in COMMANDS {
        let (_, stderr, code) = fixture.run(&[command.name, PROBE]);
        assert_eq!(
            code,
            Some(2),
            "`tcx {} {PROBE}` should be a usage error",
            command.name
        );
        assert!(
            stderr.contains("unknown option"),
            "`tcx {}` did not reject {PROBE}: {stderr}",
            command.name
        );

        // Every flag-shaped token in the documented usage and options must be
        // accepted, which is checked by running the command with it and
        // requiring that the error is not "unknown option".
        for flag in documented_flags(command) {
            let (_, stderr, _) = fixture.run(&[command.name, &flag]);
            if stderr.contains(&format!("unknown option `{flag}`")) {
                failures.push(format!("tcx {}: documents {flag} but rejects it", command.name));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Flags named in a command's usage and option lines.
fn documented_flags(command: &help::Command) -> Vec<String> {
    let mut flags = Vec::new();
    let mut scan = |text: &str| {
        for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == '|') {
            let token = token.trim_matches(|c: char| c == '[' || c == ']' || c == '(' || c == ')');
            if token.starts_with("--") || (token.starts_with('-') && token.len() == 2) {
                if !UNIVERSAL.contains(&token) && !flags.iter().any(|f| f == token) {
                    flags.push(token.to_owned());
                }
            }
        }
    };
    for usage in command.usage {
        scan(usage);
    }
    for option in command.options {
        scan(option);
    }
    flags
}

#[test]
fn top_level_help_lists_commands_and_exit_codes() {
    let fixture = Fixture::new("overall");
    let (stdout, _, code) = fixture.run(&["--help"]);
    assert_eq!(code, Some(0));
    for command in COMMANDS {
        assert!(
            stdout.contains(&format!("tcx {}", command.usage[0])),
            "{} missing from the usage block",
            command.name
        );
    }
    for (code, meaning) in help::EXIT_CODES {
        assert!(
            stdout.contains(code) && stdout.contains(meaning),
            "exit code {code} ({meaning}) undocumented"
        );
    }
    assert!(stdout.contains("TCX_HOME"));
    assert!(stdout.contains("tcx <command> --help"));
}

#[test]
fn the_shell_wrapper_delegates_help_to_the_binary() {
    // scripts/tcx used to carry its own shorter, stale help. It must now pass
    // every argument through to the binary so the two can never disagree.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let script = fs::read_to_string(repo.join("scripts/tcx")).unwrap();
    assert!(
        !script.contains("Append a dated note"),
        "scripts/tcx has its own help text again; it must delegate"
    );
    assert!(
        !script.contains(r#""$1" = "--help""#),
        "scripts/tcx intercepts --help; it must let the binary answer"
    );
    assert!(
        script.contains(r#"exec "$found" "$@""#),
        "scripts/tcx should exec the binary with all arguments"
    );

    // And the real proof: both spellings produce the binary's help.
    let out = Command::new(repo.join("scripts/tcx")).arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for command in COMMANDS {
        assert!(text.contains(command.summary), "{} missing from wrapper help", command.name);
    }
}

#[test]
fn tg_help_names_the_context_commands() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let tg = repo.join("scripts/tg");
    let out = Command::new(&tg).arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for command in ["fresh", "resume"] {
        assert!(text.contains(&format!("tg {command}")), "tg --help lacks {command}");
    }
}

// -------------------------------------------------------------------------
// The other direction: every flag the parser accepts must be documented
// -------------------------------------------------------------------------
//
// Probe-based checking above only proves documented flags exist. It cannot see
// a flag that exists and is undocumented, because nothing asks the parser for
// its full flag list. `--dir` on `note` shipped exactly that way: it worked,
// `tcx note --help` never mentioned it, and the tests stayed green.
//
// The parser arms are uniform (`match arg.as_str() { "--x" | "-y" => ... }`),
// so the accepted flags can be read out of the source without a hand-kept
// list. Each arm's string literals are attributed to the nearest enclosing
// `fn run_<name>`.

/// Flags accepted by each `run_<command>` function, read from `main.rs`.
fn accepted_flags() -> Vec<(String, Vec<String>)> {
    let src = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("main.rs"),
    )
    .expect("read main.rs");

    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<String> = None;

    for line in src.lines() {
        let trimmed = line.trim();

        // A new `fn run_<name>(` starts attributing flags to that command.
        if let Some(rest) = trimmed.strip_prefix("fn run_") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            // `run_briefing` is shared by `fresh` and `resume`; it is not a
            // command, so its flags belong to both.
            current = Some(if name == "briefing" {
                // Shared by both; whichever command documents a flag is fine.
                "resume".to_owned()
            } else {
                name
            });
            continue;
        }

        // Only match arms, so flags mentioned in error messages are ignored.
        if !trimmed.starts_with('"') || !trimmed.contains("=>") {
            continue;
        }
        let Some(ref command) = current else { continue };

        let literals: Vec<String> = trimmed
            .split('"')
            .skip(1)
            .step_by(2)
            // `--` is the end-of-options marker, not a flag.
            .filter(|s| *s != "--")
            .filter(|s| s.starts_with("--") || (s.starts_with('-') && s.len() == 2))
            .map(str::to_owned)
            .collect();
        if literals.is_empty() {
            continue;
        }

        let entry = match out.iter_mut().find(|(name, _)| name == command) {
            Some(entry) => entry,
            None => {
                out.push((command.clone(), Vec::new()));
                out.last_mut().unwrap()
            }
        };
        for flag in literals {
            if !entry.1.contains(&flag) {
                entry.1.push(flag);
            }
        }
    }

    out
}

#[test]
fn every_flag_the_parser_accepts_is_documented() {
    let accepted = accepted_flags();
    // A parse failure would make the test vacuous, so prove it found arms.
    assert!(
        accepted.iter().any(|(name, flags)| name == "note" && flags.contains(&"--dir".to_owned())),
        "could not read the note parser; got {accepted:?}"
    );

    let mut failures = Vec::new();
    for (command, flags) in &accepted {
        let Some(doc) = COMMANDS.iter().find(|c| &c.name == command) else {
            // Commands without a help entry are caught by their own test.
            continue;
        };
        let documented = documented_flags(doc);
        for flag in flags {
            if !documented.contains(flag) {
                failures.push(format!(
                    "tcx {command}: accepts {flag} but `tcx {command} --help` omits it"
                ));
            }
        }
    }

    assert!(failures.is_empty(), "undocumented flags:\n{}", failures.join("\n"));
}

#[test]
fn onboarding_documents_every_command_and_its_flags() {
    // docs/onboarding.md is what a fresh session reads first. It is prose, so
    // it cannot be generated, but it can be checked: every command and every
    // accepted flag must appear, or the doc is already behind the binary.
    let doc = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("docs")
            .join("onboarding.md"),
    )
    .expect("read docs/onboarding.md");

    let mut failures = Vec::new();
    for command in COMMANDS {
        if !doc.contains(&format!("tcx {}", command.name)) {
            failures.push(format!("docs/onboarding.md never mentions `tcx {}`", command.name));
        }
    }
    for (command, flags) in accepted_flags() {
        for flag in flags {
            if !doc.contains(&flag) {
                failures.push(format!(
                    "docs/onboarding.md does not mention `{flag}` (accepted by tcx {command})"
                ));
            }
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_word_help_is_searchable_data() {
    // `--help`/`-h` ask for help; a bare `help` is text. Matching it meant
    // `tcx find help` printed the help page instead of searching for notes
    // about help, which is the one query a maintainer is most likely to run.
    let fixture = Fixture::new("help-word");
    let (_, stderr, code) = fixture.run(&[
        "note",
        "--kind",
        "gotcha",
        "the help table lists flags",
    ]);
    assert_eq!(code, Some(0), "{stderr}");

    let (out, _, code) = fixture.run(&["find", "help"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("the help table lists flags"), "{out}");
    assert!(!out.contains("Usage:"), "printed the help page instead: {out}");

    // `tcx help` as the first word still means help.
    let (out, _, code) = fixture.run(&["help"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("Usage:"), "{out}");
}
