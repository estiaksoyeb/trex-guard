//! `tcx map` - generated tree, AI-written judgment on it.
//!
//! The point of the split is that a moved file cannot invalidate anything the
//! AI wrote, and the AI's judgment cannot smuggle a stale copy of the tree into
//! the store. These tests pin both halves.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_tcx")
}

struct Fixture {
    base: PathBuf,
    home: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!(
            "tcx-map-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = base.join("home");
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&home).unwrap();
        Fixture { base, home, repo }
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.repo.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
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
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn sample(tag: &str) -> Fixture {
    let f = Fixture::new(tag);
    f.write("src/main.rs", "fn main() {}");
    f.write("src/parser.rs", "");
    f.write("docs/readme.md", "");
    f.write("target/debug/junk.rs", "");
    f.write("node_modules/pkg/index.js", "");
    f.write("README.md", "");
    f
}

#[test]
fn map_shows_the_tree_and_skips_generated_dirs() {
    let f = sample("basic");
    let (out, stderr, code) = f.run(&["map"]);

    assert_eq!(code, Some(0), "{stderr}");
    assert!(out.starts_with("# map\n"));
    assert!(out.contains("src/\n"));
    assert!(out.contains("    main.rs\n"), "{out}");
    assert!(out.contains("docs/\n"));
    assert!(out.contains("README.md\n"));
    // Generated and vendored trees are noise, not structure.
    assert!(!out.contains("target"), "{out}");
    assert!(!out.contains("node_modules"), "{out}");
}

#[test]
fn depth_controls_how_much_is_shown() {
    let f = Sample_deep();
    let (shallow, _, _) = f.run(&["map", "--depth", "1"]);
    assert!(shallow.contains("a/\n"));
    assert!(!shallow.contains("b/"), "depth 1 showed grandchildren:\n{shallow}");

    let (deep, _, _) = f.run(&["map", "--depth", "3"]);
    assert!(deep.contains("b/\n"), "{deep}");
    assert!(deep.contains("c/\n"), "{deep}");
}

fn Sample_deep() -> Fixture {
    let f = Fixture::new("deep");
    f.write("a/b/c/leaf.rs", "");
    f
}

#[test]
fn directory_notes_appear_on_the_directory_line() {
    let f = sample("annotated");
    let (_, stderr, code) = f.run(&[
        "note",
        "-k",
        "gotcha",
        "-d",
        "src",
        "the parser is hand-written; do not reach for a crate",
    ]);
    assert_eq!(code, Some(0), "{stderr}");

    let (out, _, code) = f.run(&["map"]);
    assert_eq!(code, Some(0));
    assert!(
        out.contains("src/  - the parser is hand-written"),
        "judgment missing from the tree:\n{out}"
    );
}

#[test]
fn under_limits_the_map_to_one_subtree() {
    let f = sample("under");
    let (out, _, code) = f.run(&["map", "--under", "src"]);

    assert_eq!(code, Some(0));
    assert!(out.contains("main.rs"));
    assert!(!out.contains("README.md"), "showed files outside the subtree");
}

#[test]
fn under_is_not_capped_by_the_distance_from_the_root() {
    // A caller naming a deep subtree knows where it is; depth counts from
    // there, not from the root.
    let f = Fixture::new("deepunder");
    f.write("a/b/c/d/e/deep.rs", "");
    let (out, _, code) = f.run(&["map", "--under", "a/b/c/d/e", "--depth", "1"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("deep.rs"), "{out}");
}

#[test]
fn under_an_absent_directory_says_so() {
    let f = sample("absent");
    let (out, _, code) = f.run(&["map", "--under", "nope"]);
    // Not an error exit: asking for a subtree that is gone is a question with
    // a clear answer, not a failure.
    assert_eq!(code, Some(0));
    assert!(out.contains("`nope` is not a directory in this project"), "{out}");
}

#[test]
fn a_note_on_a_removed_directory_is_kept() {
    // This is the one thing map cannot regenerate, so it must not vanish.
    let f = sample("orphan");
    let (_, _, code) = f.run(&[
        "note", "-k", "decision", "-d", "src/removed", "this directory is gone",
    ]);
    assert_eq!(code, Some(0));

    let (out, _, _) = f.run(&["map"]);
    assert!(out.contains("no longer in the tree"), "{out}");
    assert!(out.contains("src/removed"), "{out}");
    assert!(out.contains("this directory is gone"), "{out}");
}

#[test]
fn a_data_only_directory_is_collapsed() {
    let f = Fixture::new("schemas");
    for i in 0..20 {
        f.write(&format!("schemas/{i}.json"), "{}");
    }
    f.write("src/main.rs", "");

    let (out, _, _) = f.run(&["map", "--depth", "2"]);
    assert!(out.contains("(20 data files)"), "{out}");
    assert!(!out.contains("0.json"), "listed every schema:\n{out}");
}

#[test]
fn map_is_deterministic() {
    let f = sample("deterministic");
    let (first, _, _) = f.run(&["map"]);
    let (second, _, _) = f.run(&["map"]);
    assert_eq!(first, second);
}

#[test]
fn note_refuses_a_stray_positional_path() {
    // Every other command takes a positional PATH, so `.` slips in by habit.
    // Storing it as note text is silent data corruption.
    let f = sample("straypath");
    let (stdout, stderr, code) = f.run(&["note", "-k", "gotcha", "text here", "."]);
    assert_eq!(code, Some(2));
    assert!(stdout.is_empty());
    assert!(stderr.contains("looks like a path"));

    // And nothing was written.
    let (notes, _, _) = f.run(&["note", "--list"]);
    assert!(notes.is_empty(), "a rejected note was stored: {notes}");
}

#[test]
fn note_still_accepts_prose_mentioning_a_path() {
    let f = sample("prosepath");
    let (_, stderr, code) = f.run(&["note", "-k", "gotcha", "see src/main.rs for the entry point"]);
    assert_eq!(code, Some(0), "{stderr}");
}

#[test]
fn map_rejects_a_bad_depth() {
    let f = sample("baddepth");
    let (_, stderr, code) = f.run(&["map", "--depth", "0"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("positive"));
}

#[test]
fn map_help_is_available() {
    let f = sample("help");
    let (out, _, code) = f.run(&["map", "--help"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("--depth"));
    assert!(out.contains("--under"));
}
