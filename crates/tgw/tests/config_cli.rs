//! `tgw check-config` and the startup refusal, driven through the binary.
//!
//! The unit tests cover parsing. These cover the *contract*: what a human sees
//! when their config is wrong, and the guarantee that `tgw` will not start
//! with a config it cannot trust.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};

fn tgw() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tgw"))
}

/// A throwaway `TCX_HOME` with the given `watch.ini`.
struct Home {
    dir: PathBuf,
}

impl Home {
    fn new(tag: &str, ini: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("tgw-cfg-{tag}-{}", std::process::id()));
        let cfg = dir.join("config");
        std::fs::create_dir_all(&cfg).expect("create config dir");
        if !ini.is_empty() {
            let mut f = std::fs::File::create(cfg.join("watch.ini")).expect("create watch.ini");
            f.write_all(ini.as_bytes()).expect("write watch.ini");
        }
        Home { dir }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(tgw())
            .args(args)
            .env("TCX_HOME", &self.dir)
            .output()
            .expect("run tgw")
    }

    fn ini_path(&self) -> PathBuf {
        self.dir.join("config").join("watch.ini")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const GOOD: &str = "\
clip_get_cmd = clipg
gate_cmd = tgr --check {file}
notify_cmd = notify-send approved
mode = dry
";

#[test]
fn a_good_config_reports_ok_and_its_path() {
    let home = Home::new("good", GOOD);
    let out = home.run(&["check-config"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("ok:"), "{stdout}");
    assert!(stdout.contains("mode=dry"), "{stdout}");
    assert!(
        stdout.contains(&home.ini_path().display().to_string()),
        "check-config did not print the path it read:\n{stdout}"
    );
}

#[test]
fn a_missing_config_is_reported_with_its_path() {
    let home = Home::new("missing", "");
    let out = home.run(&["check-config"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.contains("cannot read"), "{stdout}");
    assert!(stdout.contains("gate_cmd"), "{stdout}");
}

#[test]
fn every_problem_is_listed_not_just_the_first() {
    let home = Home::new("many", "gate_cmd = tgr --check {file}\nmode = nope\npoll_ms = often\n");
    let out = home.run(&["check-config"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.contains("clip_get_cmd"), "missing clipboard not reported:\n{stdout}");
    assert!(stdout.contains("unknown mode"), "{stdout}");
    assert!(stdout.contains("whole number"), "{stdout}");
}

#[test]
fn a_gate_without_the_file_placeholder_is_refused() {
    // The most dangerous config mistake: a gate that cannot see the block
    // would approve everything.
    let home = Home::new("nogatefile", "clip_get_cmd = clipg\ngate_cmd = true\n");
    let out = home.run(&["check-config"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.contains("{file}"), "{stdout}");
}

#[test]
fn tgw_refuses_to_start_with_a_bad_config() {
    let home = Home::new("refuse", "mode = auto\n");
    for args in [["run"], ["once"]] {
        let out = home.run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`tgw {}` started with no gate configured",
            args[0]
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("not usable"), "{stderr}");
        assert!(stderr.contains("check-config"), "stderr does not point at check-config: {stderr}");
    }
}

#[test]
fn an_unknown_key_is_a_problem_not_a_silent_ignore() {
    let home = Home::new("typo", &format!("{GOOD}gat_cmd = oops\n"));
    let out = home.run(&["check-config"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.contains("unknown key `gat_cmd`"), "{stdout}");
}

#[test]
fn the_config_is_never_read_from_the_current_directory() {
    // A `watch.ini` in cwd must be ignored: a repository can change under the
    // human's feet, and a watchdog that gates differently because of a file
    // in the tree is not a boundary.
    let home = Home::new("cwd", "");
    let cwd = std::env::temp_dir().join(format!("tgw-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("watch.ini"), GOOD).unwrap();

    let out = Command::new(tgw())
        .args(["check-config"])
        .env("TCX_HOME", &home.dir)
        .current_dir(&cwd)
        .output()
        .expect("run tgw");
    let _ = std::fs::remove_dir_all(&cwd);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "a cwd watch.ini was used:\n{stdout}");
    assert!(!stdout.contains("ok:"), "{stdout}");
}

#[test]
fn hold_rules_parse_as_a_name_list_only() {
    // tgw never sees a pattern, so anything here is a name. A name that does
    // not exist in `tgr --list-rules` is a `doctor` problem, not a parse
    // problem, because validation needs the gate binary.
    let home = Home::new("holdrules", &format!("{GOOD}hold_rules = rm, mv\n"));
    let out = home.run(&["check-config"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn mode_defaults_to_dry_when_not_given() {
    let home = Home::new("defaultdry", "clip_get_cmd = clipg\ngate_cmd = tgr --check {file}\n");
    let out = home.run(&["check-config"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(String::from_utf8_lossy(&out.stdout).contains("mode=dry"));
}
