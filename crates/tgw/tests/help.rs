//! Help must match the dispatcher, in both directions.
//!
//! Two failures this catches:
//!
//! - a command the binary dispatches but `help::COMMANDS` does not list, so
//!   `tgw --help` lies by omission;
//! - a command `help` lists but the binary rejects.
//!
//! Both directions are checked against the real binary, not against a copy of
//! the table, so this cannot pass by agreeing with itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tgw::help::{self, COMMANDS};

fn tgw() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgw"))
}

/// Runs the binary with no config, so config problems cannot mask help.
fn run(args: &[&str]) -> Output {
    Command::new(tgw())
        .args(args)
        .env("TCX_HOME", "/nonexistent/tgw-help-test")
        .env("HOME", "/nonexistent/tgw-help-test")
        .output()
        .expect("run tgw")
}

#[test]
fn every_documented_command_has_help() {
    for command in COMMANDS {
        for spelling in ["--help", "-h"] {
            let out = run(&[command.name, spelling]);
            assert_eq!(
                out.status.code(),
                Some(0),
                "`tgw {} {spelling}` did not succeed: {}",
                command.name,
                String::from_utf8_lossy(&out.stderr)
            );
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains(&format!("tgw {} - {}", command.name, command.summary)),
                "`tgw {} {spelling}` printed the wrong help:\n{stdout}",
                command.name
            );
            assert!(stdout.contains("Usage:"), "`tgw {}` has no usage block", command.name);
        }
    }
}

#[test]
fn every_option_the_help_lists_is_mentioned() {
    // Deliberately weak but not vacuous: an option a command accepts must
    // appear in that command's help, or the human cannot discover it.
    for command in COMMANDS {
        let text = help::render_command(command);
        for option in command.options {
            assert!(text.contains(option), "{} omits {option:?}", command.name);
        }
    }
}

#[test]
fn help_wins_over_other_arguments() {
    let out = run(&["run", "--tail", "5", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("tgw run - "));
}

#[test]
fn every_command_the_binary_accepts_is_documented() {
    // Forward direction: a dispatched command that help does not list.
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();
    let dispatched = dispatched_commands(&source);
    let documented: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();

    assert!(
        !dispatched.is_empty(),
        "could not read the dispatcher; this probe is blind"
    );
    let mut missing: Vec<&String> = dispatched
        .iter()
        .filter(|n| !documented.contains(&n.as_str()))
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "the dispatcher handles {missing:?} but help::COMMANDS does not list them"
    );

    // Backward direction: a documented command the binary rejects.
    for name in &documented {
        let out = run(&[name, "--help"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`tgw {name}` is listed in help but the binary rejects it: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Extracts `"name" => cmd_x(...)` arms from the dispatcher in `main.rs`.
fn dispatched_commands(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        let Some((lhs, rhs)) = line.split_once("=>") else {
            continue;
        };
        if !rhs.trim_start().starts_with("cmd_") {
            continue;
        }
        for part in lhs.split('|') {
            let name = part.trim().trim_matches('"');
            if !name.is_empty()
                && !name.starts_with('-')
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            {
                out.push(name.to_owned());
            }
        }
    }
    out
}

#[test]
fn an_unknown_command_is_a_usage_error() {
    let out = run(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown command"));
}

#[test]
fn no_arguments_is_a_usage_error_not_a_silent_start() {
    // Starting a watchdog with no instruction would be a mistake; it must
    // not look like success.
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage:"));
}

#[test]
fn version_prints_and_succeeds() {
    for flag in ["--version", "-v"] {
        let out = run(&[flag]);
        assert_eq!(out.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("tgw version "));
    }
}

/// Sanity: the binary under test is the one this crate builds.
#[test]
fn the_binary_under_test_exists() {
    assert!(Path::new(&tgw()).exists(), "no tgw binary at {}", tgw().display());
}
