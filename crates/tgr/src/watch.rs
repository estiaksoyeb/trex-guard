//! Named rules that are reported by `tgr --check` but never affect a paste.
//!
//! These exist for one caller: the `tgw` watchdog. It wants to treat some
//! ordinary-looking commands as needing a human (`sed -i`, `git reset`, `rm`),
//! but `tgr` must not start holding those pastes — that would change daily
//! terminal behaviour and make `tgr` diverge from `tg`, which is the one thing
//! the port forbids.
//!
//! So a watch rule matches text and reports **nothing but its name**. It does
//! not contribute risk, cannot raise a verdict, and is never consulted by the
//! paste path at all. `classify_with` does not call this module; only
//! `main::run_check` does.
//!
//! # Why this is safe
//!
//! | Where | Sees watch rules | Effect |
//! | --- | --- | --- |
//! | `tgr` (paste path) | no | none — the code is not called |
//! | `tg` | no such concept | none |
//! | `tgr --check` | yes | one extra `watch=<name>` line on stderr |
//!
//! `tests/watch_rules.rs` proves the paste path is byte-identical with the
//! rules present and with them removed.
//!
//! # Naming
//!
//! **The names are an interface**: renaming one changes what shows up in
//! `rule=<name>` on stderr and in `tgw`'s explanation of a hold.
//! `tgr --list-rules` prints them.
//!
//! # Configuration
//!
//! Defaults are compiled in so a fresh install works with no file edited.
//! `[watch-rules]` in `config.ini` can add a rule (`name | pattern`) or
//! disable a default by naming it. `tg` ignores the section entirely, because
//! it ignores every section it does not know.

use std::collections::HashSet;

use crate::config::Config;

/// A named, risk-free rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRule {
    pub name: String,
    /// A regex over the raw paste, compiled by [`compile`].
    pub pattern: String,
}

/// The rules compiled into the binary.
///
/// Each one is a command shape that is destructive but too ordinary to halt a
/// paste for. The patterns are anchored on the start of a command word so
/// `echo sed -i` does not match, and they deliberately do not try to be a
/// shell parser: a false positive costs a hold prompt, and a false negative is
/// covered by the ordinary risk rules.
const DEFAULT_RULES: &[(&str, &str)] = &[
    // In-place editing rewrites a file the human may not have a copy of.
    ("sed-i", r"(?m)(^|[|;&(]\s*)sed\s+\S*-[A-Za-z]*i"),
    ("git-reset", r"(?m)(^|[|;&(]\s*)git\s+[^|;&\n]*\breset\b"),
    ("git-checkout", r"(?m)(^|[|;&(]\s*)git\s+[^|;&\n]*\bcheckout\b"),
    ("git-clean", r"(?m)(^|[|;&(]\s*)git\s+[^|;&\n]*\bclean\b"),
    ("git-restore", r"(?m)(^|[|;&(]\s*)git\s+[^|;&\n]*\brestore\b"),
    // Plain deletion (the recursive-force form is already Danger).
    ("rm", r"(?m)(^|[|;&(]\s*)(sudo\s+)?rm\s"),
    ("mv", r"(?m)(^|[|;&(]\s*)(sudo\s+)?mv\s"),
    ("chmod", r"(?m)(^|[|;&(]\s*)(sudo\s+)?chmod\s"),
    ("chown", r"(?m)(^|[|;&(]\s*)(sudo\s+)?chown\s"),
    ("dd", r"(?m)(^|[|;&(]\s*)(sudo\s+)?dd\s"),
    // Writing outside /tmp, which is where this project's scratch space lives.
    (
        "redirect-outside-tmp",
        r">\s*/?(?!tmp/|dev/null|dev/stdout|dev/stderr)[a-zA-Z]",
    ),
];

/// Every rule in effect: defaults, minus disabled, plus additions.
///
/// Order is defaults-first then additions, so the reported order is stable for
/// a given config.
pub fn effective(config: &Config) -> Vec<WatchRule> {
    let disabled: HashSet<&str> = config.disabled_watch_rules.iter().map(String::as_str).collect();
    let mut out: Vec<WatchRule> = DEFAULT_RULES
        .iter()
        .filter(|(name, _)| !disabled.contains(name))
        .map(|(name, pattern)| WatchRule {
            name: (*name).to_string(),
            pattern: (*pattern).to_string(),
        })
        .collect();
    for rule in &config.watch_rules {
        if disabled.contains(rule.name.as_str()) {
            continue;
        }
        // A configured rule replaces a default of the same name, so a human
        // can retune `sed-i` without it appearing twice.
        out.retain(|r| r.name != rule.name);
        out.push(rule.clone());
    }
    out
}

/// A rule whose pattern did not compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenRule {
    pub name: String,
    pub message: String,
}

/// Compiles the effective rules, returning the broken ones separately.
///
/// A broken rule is **reported, not silently dropped**: dropping it would let
/// a hold silently stop happening, which is the failure this whole module is
/// meant to make visible. The caller decides what to do; `--check` prints it
/// and `--list-rules` marks it.
pub fn compile(config: &Config) -> (Vec<(String, fancy_regex::Regex)>, Vec<BrokenRule>) {
    let mut ok = Vec::new();
    let mut broken = Vec::new();
    for rule in effective(config) {
        match fancy_regex::Regex::new(&rule.pattern) {
            Ok(re) => ok.push((rule.name, re)),
            Err(e) => broken.push(BrokenRule {
                name: rule.name,
                message: e.to_string(),
            }),
        }
    }
    (ok, broken)
}

/// Returns the names of every rule that matched `paste`, in rule order.
///
/// Names only: there is no risk here by construction, and the return type
/// says so.
pub fn matches(config: &Config, paste: &str) -> Vec<String> {
    let (rules, _) = compile(config);
    rules
        .into_iter()
        .filter(|(_, re)| re.is_match(paste).unwrap_or(false))
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(paste: &str) -> Vec<String> {
        matches(&Config::default(), paste)
    }

    #[test]
    fn every_default_pattern_compiles() {
        let (ok, broken) = compile(&Config::default());
        assert!(broken.is_empty(), "broken defaults: {broken:?}");
        assert_eq!(ok.len(), DEFAULT_RULES.len());
    }

    #[test]
    fn default_names_are_unique() {
        let mut seen = HashSet::new();
        for (name, _) in DEFAULT_RULES {
            assert!(seen.insert(*name), "duplicate watch rule name: {name}");
        }
    }

    #[test]
    fn matches_ordinary_destructive_commands() {
        assert_eq!(names("sed -i 's/a/b/' f"), ["sed-i"]);
        assert_eq!(names("git reset --hard"), ["git-reset"]);
        assert_eq!(names("rm file.txt"), ["rm"]);
        assert_eq!(names("mv a b"), ["mv"]);
        assert_eq!(names("chmod 644 f"), ["chmod"]);
        assert_eq!(names("chown u f"), ["chown"]);
        assert_eq!(names("dd if=a of=b"), ["dd"]);
    }

    #[test]
    fn matches_after_a_pipe_or_semicolon() {
        assert_eq!(names("echo x; rm y"), ["rm"]);
        assert_eq!(names("true && mv a b"), ["mv"]);
        assert_eq!(names("cat f | sed -i 's/a/b/'"), ["sed-i"]);
    }

    #[test]
    fn matches_through_sudo() {
        assert_eq!(names("sudo rm /etc/x"), ["rm"]);
        assert_eq!(names("sudo chmod 777 /x"), ["chmod"]);
    }

    #[test]
    fn does_not_match_the_same_words_as_arguments() {
        // The point of anchoring: these are not deletions.
        assert!(names("echo rm -rf /").is_empty(), "{:?}", names("echo rm -rf /"));
        assert!(names("grep 'sed -i' file").is_empty());
        assert!(names("man rm").is_empty());
    }

    #[test]
    fn does_not_match_harmless_rm_usage_in_a_path() {
        // `rm` as part of a word, not a command.
        assert!(names("ls /var/rm").is_empty());
        assert!(names("cat myrm.txt").is_empty());
    }

    #[test]
    fn redirect_to_tmp_and_null_is_ignored() {
        assert!(names("echo x > /tmp/f").is_empty(), "{:?}", names("echo x > /tmp/f"));
        assert!(names("cmd > /dev/null").is_empty());
        assert_eq!(names("echo x > /etc/passwd"), ["redirect-outside-tmp"]);
    }

    #[test]
    fn a_disabled_default_disappears() {
        let mut cfg = Config::default();
        cfg.disabled_watch_rules.insert("rm".into());
        assert!(matches(&cfg, "rm f").is_empty());
        let all = effective(&cfg);
        assert!(!all.iter().any(|r| r.name == "rm"));
    }

    #[test]
    fn a_configured_rule_is_added_and_can_replace_a_default() {
        let mut cfg = Config::default();
        cfg.watch_rules.push(WatchRule {
            name: "sed-i".into(),
            pattern: r"\bsed\b".into(),
        });
        cfg.watch_rules.push(WatchRule {
            name: "kubectl-delete".into(),
            pattern: r"\bkubectl\s+delete\b".into(),
        });
        let all = effective(&cfg);
        assert_eq!(all.iter().filter(|r| r.name == "sed-i").count(), 1);
        assert!(all.iter().any(|r| r.name == "kubectl-delete"));
        assert_eq!(matches(&cfg, "kubectl delete pod"), ["kubectl-delete"]);
    }

    #[test]
    fn a_broken_configured_rule_is_reported_not_dropped() {
        let mut cfg = Config::default();
        cfg.watch_rules.push(WatchRule {
            name: "bad".into(),
            pattern: "(".into(),
        });
        let (_, broken) = compile(&cfg);
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].name, "bad");
    }

    #[test]
    fn an_allowlisted_paste_still_reports_watch_matches() {
        // The allowlist makes a paste Safe for the guard; it cannot make a
        // destructive command stop being destructive for the watchdog, but
        // this module does not know about the allowlist at all. Documenting
        // the boundary: watch matching is independent of risk config.
        assert_eq!(names("rm -rf /tmp/ok"), ["rm"]);
    }
}
