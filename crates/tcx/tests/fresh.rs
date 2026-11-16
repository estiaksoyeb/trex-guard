//! `tcx fresh` / `tcx resume` over the CLI.
//!
//! These are the two commands the workflow actually centres on, so they are
//! tested end to end: build a throwaway repository, run the binary against it
//! with an isolated `TCX_HOME`, and inspect stdout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_tcx")
}

/// A temporary directory that is removed on drop.
struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        let base = std::env::temp_dir().join(format!(
            "tcx-fresh-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        Temp(base)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

fn init_repo(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "init"]);
}

/// Runs `tcx <args>` with an isolated store and the given cwd.
fn run(home: &Path, cwd: &Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env("TCX_HOME", home)
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

const TEMPLATE: &str = "# Workflow\n\n## Rules\nbe careful\n\n## Project\n<placeholder>\ntype: <x>\n";

fn cargo_repo(tag: &str) -> Temp {
    let temp = Temp::new(tag);
    write(
        temp.path(),
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\ndescription = \"a demo\"\n",
    );
    write(temp.path(), "src/main.rs", "fn main() {}\n");
    write(temp.path(), "prompt.md", TEMPLATE);
    init_repo(temp.path());
    temp
}

#[test]
fn fresh_fills_the_project_section_from_cargo() {
    let repo = cargo_repo("cargo");
    let home = Temp::new("home");

    let (stdout, stderr, code) = run(home.path(), repo.path(), &["fresh"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.starts_with("# Workflow\n"));
    // The name is the repository directory, which is a randomised temp dir here.
    assert!(stdout.contains(": Rust — a demo (demo)\nPackage: demo\n"), "{stdout}");
    assert!(stdout.contains("Source: src/\n"));
    assert!(stdout.contains("Build: Cargo\n"));
    assert!(stdout.contains("type: code"));
    assert!(!stdout.contains("<placeholder>"), "placeholder survived");
    // Rules below the project block are preserved.
    assert!(stdout.contains("## Rules\nbe careful\n"));
    // The template path goes to stderr so it cannot pollute the paste.
    assert!(stderr.contains("prompt.md"));
}

#[test]
fn fresh_fills_an_android_gradle_project() {
    let repo = Temp::new("gradle");
    write(
        repo.path(),
        "settings.gradle.kts",
        "pluginManagement {\n}\ninclude(\":app\")\ninclude(\":core\")\n",
    );
    write(repo.path(), "build.gradle.kts", "plugins { }\n");
    write(
        repo.path(),
        "app/build.gradle.kts",
        "plugins {\n  alias(libs.plugins.kotlin.compose.compiler)\n}\nandroid {\n  namespace = \"com.example.app\"\n\n  defaultConfig {\n    applicationId = \"com.example.app\"\n  }\n}\n",
    );
    write(repo.path(), "app/src/main/AndroidManifest.xml", "<manifest>\n");
    // Enough files in :app that the largest-module rule picks it.
    write(
        repo.path(),
        "app/src/main/kotlin/com/example/app/Main.kt",
        "package com.example.app\n",
    );
    write(
        repo.path(),
        "core/src/main/kotlin/com/example/core/Core.kt",
        "package com.example.core\n",
    );
    write(repo.path(), "prompt.md", TEMPLATE);
    init_repo(repo.path());
    let home = Temp::new("home");

    let (stdout, stderr, code) = run(home.path(), repo.path(), &["fresh"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("Android app, Kotlin + Jetpack Compose"),
        "{stdout}"
    );
    assert!(stdout.contains("Package: com.example.app\n"));
    assert!(stdout.contains("Source: app/src/main/kotlin/com/example/app/\n"));
    assert!(stdout.contains("Build: Gradle (Kotlin DSL)\n"));
    assert!(stdout.contains("Modules: :app, :core\n"));
    assert!(stdout.contains("type: code"));
}

#[test]
fn fresh_reports_every_template_candidate_when_missing() {
    let repo = cargo_repo("notemplate");
    fs::remove_file(repo.path().join("prompt.md")).unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-qm", "drop template"]);
    let home = Temp::new("home");

    let (stdout, stderr, code) = run(home.path(), repo.path(), &["fresh"]);

    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(stderr.contains("no prompt template found"));
    assert!(stderr.contains(".tg/prompt.md"));
}

#[test]
fn template_lookup_prefers_the_repo_over_the_global_copy() {
    let repo = cargo_repo("precedence");
    let home = Temp::new("home");
    write(home.path(), "config/prompt.md", "GLOBAL\n## Project\nold\n");

    let (stdout, _, code) = run(home.path(), repo.path(), &["fresh"]);

    assert_eq!(code, 0);
    assert!(stdout.starts_with("# Workflow"), "used global, not repo");
}

#[test]
fn template_lookup_falls_back_to_the_global_copy() {
    let repo = cargo_repo("globalfallback");
    fs::remove_file(repo.path().join("prompt.md")).unwrap();
    let home = Temp::new("home");
    write(home.path(), "config/prompt.md", "GLOBAL\n## Project\nold\n");

    let (stdout, _, code) = run(home.path(), repo.path(), &["fresh"]);

    assert_eq!(code, 0);
    assert!(stdout.starts_with("GLOBAL"));
}

#[test]
fn explicit_template_wins() {
    let repo = cargo_repo("explicittemplate");
    let home = Temp::new("home");
    write(home.path(), "mine.md", "MINE\n## Project\nold\n");

    let (stdout, _, code) = run(
        home.path(),
        repo.path(),
        &["fresh", "--template", home.path().join("mine.md").to_str().unwrap()],
    );

    assert_eq!(code, 0);
    assert!(stdout.starts_with("MINE"));
}

#[test]
fn fresh_never_mentions_notes() {
    let repo = cargo_repo("freshnonotes");
    let home = Temp::new("home");
    let (_, _, code) = run(
        home.path(),
        repo.path(),
        &["note", "-k", "gotcha", "watch out for the thing"],
    );
    assert_eq!(code, 0);

    let (stdout, _, code) = run(home.path(), repo.path(), &["fresh"]);

    assert_eq!(code, 0);
    assert!(!stdout.contains("watch out for the thing"));
    assert!(!stdout.contains("previous sessions"));
}

#[test]
fn resume_appends_notes_that_fresh_does_not() {
    let repo = cargo_repo("resume");
    let home = Temp::new("home");
    for (kind, text) in [
        ("gotcha", "the init order matters"),
        ("decision", "store lives outside the repo"),
        ("open", "should modules be split"),
    ] {
        let (_, _, code) = run(home.path(), repo.path(), &["note", "-k", kind, text]);
        assert_eq!(code, 0);
    }

    let (stdout, stderr, code) = run(home.path(), repo.path(), &["resume"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    // The name is the repository directory, which is a randomised temp dir here.
    assert!(stdout.contains(": Rust — a demo (demo)\nPackage: demo\n"), "{stdout}");
    assert!(stdout.contains("## What previous sessions learned"));
    assert!(stdout.contains("the init order matters"));
    assert!(stdout.contains("store lives outside the repo"));
    assert!(stdout.contains("should modules be split"));
    // Gotchas come first: they are what already caused a bug.
    let gotcha = stdout.find("the init order matters").unwrap();
    let open = stdout.find("should modules be split").unwrap();
    assert!(gotcha < open);
}

#[test]
fn resume_says_when_there_are_no_notes() {
    let repo = cargo_repo("resumeempty");
    let home = Temp::new("home");

    let (stdout, _, code) = run(home.path(), repo.path(), &["resume"]);

    assert_eq!(code, 0);
    assert!(stdout.contains("## What previous sessions learned"));
    assert!(stdout.contains("No notes yet"));
}

#[test]
fn resume_respects_the_budget() {
    let repo = cargo_repo("budget");
    let home = Temp::new("home");
    for i in 0..40 {
        let (_, _, code) = run(
            home.path(),
            repo.path(),
            &["note", "-k", "decision", &format!("decision number {i} with padding")],
        );
        assert_eq!(code, 0);
    }

    let (stdout, _, code) = run(home.path(), repo.path(), &["resume", "--budget", "200"]);

    assert_eq!(code, 0);
    assert!(stdout.contains("more (tcx find)"), "{stdout}");
}

#[test]
fn fresh_rejects_unknown_options() {
    let repo = cargo_repo("badopt");
    let home = Temp::new("home");

    let (_, stderr, code) = run(home.path(), repo.path(), &["fresh", "--nope"]);

    assert_eq!(code, 2);
    assert!(stderr.contains("unknown option"));
}

#[test]
fn help_lists_both_commands() {
    let home = Temp::new("home");
    let (stdout, _, code) = run(home.path(), Path::new("."), &["--help"]);

    assert_eq!(code, 0);
    assert!(stdout.contains("tcx fresh"));
    assert!(stdout.contains("tcx resume"));
}
