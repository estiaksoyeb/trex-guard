//! The classifier: paste -> [`Match`].
//!
//! Ported from `src/rules.cpp`. Five passes, in the C++ order, because the
//! order decides which rule *names* a match and how duplicates collapse:
//!
//! 1. allowlist (a substring hit makes the whole paste Safe);
//! 2. pipeline stage correlation (`curl ... | bash`);
//! 3. per-command semantics (`rm -rf`, `git push --force`, ...);
//! 4. literal needles (fork bomb);
//! 5. built-in regexes, then user `[custom_rules]`.
//!
//! # The one place this deliberately differs
//!
//! A user regex that fails to compile is an **error** ([`RegexProblem::Invalid`])
//! rather than being silently skipped, and a regex that exceeds the backtrack
//! limit makes the paste at least `Review` ([`RegexProblem::BacktrackLimit`]).
//! `docs/tcx.md` §8 measured that limit; the C++ guard has no equivalent and
//! would simply have run the query.
//!
//! Transient regex problems are collected in [`Match::matches`] as a synthetic
//! `regex safety` rule so the user is told *why* their paste was held, instead
//! of a bare `[REVIEW]` with no rule name.

use std::sync::OnceLock;

use fancy_regex::{Regex, RegexBuilder};

use crate::config::Config;
use crate::risk::{Match, Risk};
use crate::shell::{self, ParsedCommand, Pipeline};

/// CAPTCHAs and pathological patterns must not stall a keypress.
const BACKTRACK_LIMIT: usize = 1_000_000;

/// A problem compiling or running a user regex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegexProblem {
    /// The pattern does not compile (typo, unsupported feature).
    Invalid { rule: String, message: String },
    /// The pattern ran too long: treat as suspicious, never as Safe.
    BacktrackLimit { rule: String },
}

impl RegexProblem {
    fn describe(&self) -> (String, String) {
        match self {
            RegexProblem::Invalid { rule, message } => (
                format!("{rule} (invalid regex)"),
                format!("This rule is configured but does not compile, so it cannot run: {message}"),
            ),
            RegexProblem::BacktrackLimit { rule } => (
                format!("{rule} (regex timeout)"),
                "This rule took too long to evaluate; the paste is held rather than assumed safe"
                    .to_string(),
            ),
        }
    }
}

/// Result of a full classification, including regex problems that did not
/// produce a normal match.
#[derive(Debug, Clone)]
pub struct Verdict {
    /// The same value the C++ `classify()` returns.
    pub matched: Match,
    /// User-regex problems, in configuration order. Non-empty means at least
    /// one rule could not run; callers should surface it.
    pub problems: Vec<RegexProblem>,
}

impl Verdict {
    /// At least one configured rule could not be applied.
    pub fn has_problems(&self) -> bool {
        !self.problems.is_empty()
    }
}

struct RegexRule {
    name: &'static str,
    risk: Risk,
    explanation: &'static str,
    pattern: &'static str,
}

struct LiteralRule {
    name: &'static str,
    risk: Risk,
    explanation: &'static str,
    needle: &'static str,
}

const LITERAL_RULES: &[LiteralRule] = &[LiteralRule {
    name: "fork bomb",
    risk: Risk::Danger,
    explanation: "Self-replicating shell function that exhausts system process table",
    needle: ":(){:|:&};:",
}];

const REGEX_RULES: &[RegexRule] = &[
    RegexRule {
        name: "mkfs",
        risk: Risk::Danger,
        explanation: "Creates a filesystem and overwrites existing partition data",
        pattern: r"\bmkfs(\.[a-zA-Z0-9]+)?\b",
    },
    RegexRule {
        name: "dd",
        risk: Risk::Danger,
        explanation: "Low-level block read/write operation can destroy partition tables and filesystems",
        pattern: r"\bdd\s+.*if=",
    },
    RegexRule {
        name: "write to block device",
        risk: Risk::Danger,
        explanation: "Direct write to a raw block device bypasses filesystems and causes data loss",
        pattern: r">\s*/dev/(sd[a-z]|nvme[0-9]|vd[a-z]|hd[a-z]|mmcblk)",
    },
    RegexRule {
        name: "heredoc to shell",
        risk: Risk::Danger,
        explanation: "Directly executes inline heredoc script into a shell interpreter",
        pattern: r#"\b(bash|sh|zsh)\s*<<\s*['"]?[A-Za-z0-9_]+['"]?|<<\s*['"]?[A-Za-z0-9_]+['"]?\s*\|\s*(sudo\s+)?(ba|z|a)?sh\b"#,
    },
    RegexRule {
        name: "shell eval",
        risk: Risk::Danger,
        explanation: "Dynamic shell evaluation can execute arbitrary uninspected code",
        pattern: r#"\beval\s+[\$"']"#,
    },
    RegexRule {
        name: "inline script execution",
        risk: Risk::Danger,
        explanation: "Executes system process or dynamic code through scripting interpreter one-liner",
        pattern: r"\bpython[0-9.]*\s+-c\s+.*(os\.system|subprocess\.|exec\(|eval\()|\bperl\s+-e\s+.*(system|exec)|\bruby\s+-e\s+.*(system|exec)",
    },
    RegexRule {
        name: "subshell execution",
        risk: Risk::Review,
        explanation: "Executes inline command string within an explicit subshell or elevated context",
        pattern: r"\b(sudo\s+)?(bash|sh|zsh)\s+-c\s+",
    },
];

/// Compiled built-ins, built once.
///
/// A built-in that does not compile is a bug in this crate, so it panics
/// loudly at first use rather than silently leaving a hole in the rules.
fn builtin_regexes() -> &'static Vec<(&'static RegexRule, Regex)> {
    static CELL: OnceLock<Vec<(&'static RegexRule, Regex)>> = OnceLock::new();
    CELL.get_or_init(|| {
        REGEX_RULES
            .iter()
            .map(|rule| {
                let re = Regex::new(rule.pattern).unwrap_or_else(|e| {
                    panic!("built-in rule `{}` does not compile: {e}", rule.name)
                });
                (rule, re)
            })
            .collect()
    })
}

fn is_shell_interpreter(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "zsh" | "ash" | "dash" | "fish")
}

/// Trims surrounding separators from a matched snippet.
fn trim_snippet(snip: &str) -> &str {
    snip.trim_matches(|c: char| matches!(c, '\r' | '\n' | '\t' | ' ' | ';' | '&' | '|' | '`' | '$'))
}

/// Accumulates matches, collapsing overlapping snippets the way C++ does.
struct Collector<'a> {
    config: Option<&'a Config>,
    all: Vec<Match>,
}

impl<'a> Collector<'a> {
    fn new(config: Option<&'a Config>) -> Self {
        Collector {
            config,
            all: Vec::new(),
        }
    }

    fn add(&mut self, risk: Risk, rule: &str, snippet: &str, explanation: &str) {
        // A disabled rule never fires, whatever else matched.
        if let Some(cfg) = self.config {
            if cfg.disabled_rules.contains(rule) {
                return;
            }
        }

        let clean = trim_snippet(snippet).to_string();
        if clean.is_empty() {
            return;
        }

        // Collapse snippets that contain each other, keeping the worse risk
        // (or the longer snippet at equal risk).
        for existing in &mut self.all {
            let overlaps = existing.snippet == clean
                || existing.snippet.contains(&clean)
                || clean.contains(&existing.snippet);
            if overlaps {
                if risk > existing.risk || (risk == existing.risk && clean.len() > existing.snippet.len())
                {
                    existing.risk = risk;
                    existing.rule = rule.to_string();
                    existing.snippet = clean;
                    existing.explanation = explanation.to_string();
                }
                return;
            }
        }

        self.all.push(Match {
            risk,
            rule: rule.to_string(),
            snippet: clean,
            explanation: explanation.to_string(),
            matches: Vec::new(),
        });
    }

    /// Danger first, then Review, preserving insertion order within a rank.
    fn finish(mut self) -> Match {
        if self.all.is_empty() {
            return Match::safe();
        }
        self.all.sort_by(|a, b| b.risk.cmp(&a.risk));
        let mut primary = self.all[0].clone();
        primary.matches = self.all;
        primary
    }
}

/// Applies the pipeline-level rules (`curl | bash`, `base64 -d | sh`).
fn classify_pipeline(collector: &mut Collector<'_>, pipeline: &Pipeline) {
    let stages = &pipeline.stages;
    if stages.len() >= 2 {
        for i in 0..stages.len() - 1 {
            let upstream = &stages[i];
            for downstream in &stages[i + 1..] {
                if matches!(upstream.base_name.as_str(), "curl" | "wget")
                    && is_shell_interpreter(&downstream.base_name)
                {
                    collector.add(
                        Risk::Danger,
                        "pipe to shell",
                        &pipeline.raw_snippet,
                        "Executes remote untrusted script directly in shell without prior inspection",
                    );
                }

                let decodes = upstream.base_name == "base64"
                    && (upstream.has_flag("-d")
                        || upstream.has_long_flag("decode")
                        || upstream.has_short_flag_char('d'));
                if decodes && is_shell_interpreter(&downstream.base_name) {
                    collector.add(
                        Risk::Danger,
                        "encoded shell pipe",
                        &pipeline.raw_snippet,
                        "Decodes and directly executes obfuscated shell commands",
                    );
                }
            }
        }
    }
}

fn git_subcommand<'c>(cmd: &'c ParsedCommand) -> Option<(&'c str, &'c [String])> {
    let args = &cmd.args;
    let mut idx = 0;
    while idx < args.len() {
        let a = args[idx].as_str();
        let takes_value = matches!(
            a,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" | "--config-env"
        );
        if takes_value {
            idx += 2;
        } else if a.starts_with("--git-dir=")
            || a.starts_with("--work-tree=")
            || a.starts_with("-c")
            || a.starts_with("-C")
        {
            idx += 1;
        } else if a.starts_with('-') {
            // Includes --no-pager, -p, --paginate, --bare, --version, -h, ...
            idx += 1;
        } else {
            break;
        }
    }
    if idx < args.len() {
        Some((args[idx].as_str(), &args[idx + 1..]))
    } else {
        None
    }
}

fn has_force_flag(sub_args: &[String]) -> bool {
    sub_args.iter().any(|sa| {
        sa == "--force"
            || sa == "-f"
            || sa == "--force-with-lease"
            || (sa.len() >= 2 && sa.starts_with('-') && !sa.starts_with("--") && sa.contains('f'))
    })
}

fn classify_git(collector: &mut Collector<'_>, cmd: &ParsedCommand) {
    let Some((subcommand, sub_args)) = git_subcommand(cmd) else {
        return;
    };
    let snip = &cmd.raw_snippet;

    match subcommand {
        "reset" => {
            if sub_args.iter().any(|a| a == "--hard" || a.starts_with("--hard=")) {
                collector.add(
                    Risk::Review,
                    "git hard reset",
                    snip,
                    "Discards all uncommitted changes and resets branch head",
                );
            }
        }
        "push" => {
            if has_force_flag(sub_args) {
                collector.add(
                    Risk::Review,
                    "git force push",
                    snip,
                    "Overwrites remote repository history, potentially destroying remote commits",
                );
            }
        }
        "clean" => {
            if has_force_flag(sub_args) {
                collector.add(
                    Risk::Review,
                    "git clean",
                    snip,
                    "Permanently deletes untracked files and directories from the repository",
                );
            }
        }
        "checkout" => {
            let mut destructive = false;
            for (k, sa) in sub_args.iter().enumerate() {
                if sa == "-f" || sa == "--force" || sa == "." {
                    destructive = true;
                    break;
                }
                if sa == "--" && sub_args.get(k + 1).is_some_and(|n| n == ".") {
                    destructive = true;
                    break;
                }
            }
            if destructive {
                collector.add(
                    Risk::Review,
                    "git checkout/restore",
                    snip,
                    "Discards modified working tree state across working tree files",
                );
            }
        }
        "restore" => {
            if sub_args.iter().any(|sa| sa == "." || sa == "--worktree") {
                collector.add(
                    Risk::Review,
                    "git checkout/restore",
                    snip,
                    "Discards modified working tree state across working tree files",
                );
            }
        }
        _ => {}
    }
}

fn classify_command(collector: &mut Collector<'_>, cmd: &ParsedCommand) {
    let bin = cmd.base_name.as_str();
    let snip = &cmd.raw_snippet;

    if bin == "git" {
        classify_git(collector, cmd);
    }

    if bin == "rm" {
        let recursive =
            cmd.has_short_flag_char('r') || cmd.has_short_flag_char('R') || cmd.has_long_flag("recursive");
        let force = cmd.has_short_flag_char('f') || cmd.has_long_flag("force");

        if recursive && force {
            collector.add(
                Risk::Danger,
                "rm -rf",
                snip,
                "Recursive force deletion permanently removes directories without prompt",
            );
        } else if !cmd.args.is_empty() {
            let only_help = (cmd.has_long_flag("help") || cmd.has_long_flag("version")) && cmd.args.len() == 1;
            if !only_help {
                collector.add(
                    Risk::Review,
                    "file deletion",
                    snip,
                    "Permanently deletes files or directories from the filesystem",
                );
            }
        }
    }

    if matches!(bin, "rmdir" | "unlink" | "shred") {
        collector.add(
            Risk::Review,
            "file deletion",
            snip,
            "Permanently deletes files or directories from the filesystem",
        );
    }

    if bin == "chmod" {
        let recursive = cmd.has_short_flag_char('R') || cmd.has_long_flag("recursive");
        let mut danger_mode = false;
        let mut root_target = false;
        for a in &cmd.args {
            if a == "777"
                || a == "000"
                || a == "a+rwx"
                || a == "u=rwx,go=rwx"
                || a.contains("777")
                || a.contains("000")
            {
                danger_mode = true;
            }
            if a == "/" || a == "/*" || a == "/root" {
                root_target = true;
            }
        }
        if (recursive && danger_mode) || (root_target && (danger_mode || recursive)) {
            collector.add(
                Risk::Danger,
                "chmod",
                snip,
                "Broad or recursive permission changes compromise system security and file integrity",
            );
        }
    }

    if bin == "find" {
        let mut deletes = false;
        for (k, a) in cmd.args.iter().enumerate() {
            if a == "-delete" {
                deletes = true;
                break;
            }
            if a == "-exec" && cmd.args.get(k + 1).is_some_and(|n| n == "rm") {
                deletes = true;
                break;
            }
        }
        if deletes {
            collector.add(
                Risk::Danger,
                "find delete",
                snip,
                "Unprompted bulk deletion of matched filesystem objects",
            );
        }
    }

    if is_shell_interpreter(bin) && cmd.has_flag("-c") {
        collector.add(
            Risk::Review,
            "subshell execution",
            snip,
            "Executes inline command string within an explicit subshell or elevated context",
        );
    }
}

fn compile_user(pattern: &str) -> Result<Regex, fancy_regex::Error> {
    RegexBuilder::new(pattern)
        .backtrack_limit(BACKTRACK_LIMIT)
        .build()
}

/// Classifies a paste with the built-in rules only.
pub fn classify(paste: &str) -> Match {
    classify_with(paste, None).matched
}

/// Classifies a paste, reporting any user-rule problems separately.
pub fn classify_with(paste: &str, config: Option<&Config>) -> Verdict {
    let mut collector = Collector::new(config);
    let mut problems = Vec::new();

    // 1. Allowlist: a substring hit makes the whole paste Safe.
    if let Some(cfg) = config {
        for item in &cfg.allowlist {
            if !item.is_empty() && paste.contains(item.as_str()) {
                return Verdict {
                    matched: Match::safe(),
                    problems,
                };
            }
        }
    }

    // 2. Pipelines and per-command semantics.
    let pipelines = shell::parse(paste);
    for pipeline in &pipelines {
        classify_pipeline(&mut collector, pipeline);
        for cmd in &pipeline.stages {
            classify_command(&mut collector, cmd);
        }
    }

    // 3. Literal needles.
    for rule in LITERAL_RULES {
        let mut from = 0;
        while let Some(pos) = paste[from..].find(rule.needle) {
            collector.add(rule.risk, rule.name, rule.needle, rule.explanation);
            from += pos + rule.needle.len();
        }
    }

    // 4. Built-in regexes.
    for (rule, re) in builtin_regexes() {
        for m in re.find_iter(paste).flatten() {
            collector.add(rule.risk, rule.name, m.as_str(), rule.explanation);
        }
    }

    // 5. User rules. This is where the port deliberately differs.
    if let Some(cfg) = config {
        for rule in &cfg.custom_rules {
            match compile_user(&rule.pattern) {
                Ok(re) => {
                    let mut hit_backtrack = false;
                    for m in re.find_iter(paste) {
                        match m {
                            Ok(m) => collector.add(rule.risk, &rule.name, m.as_str(), &rule.explanation),
                            Err(fancy_regex::Error::RuntimeError(
                                fancy_regex::RuntimeError::BacktrackLimitExceeded,
                            )) => {
                                hit_backtrack = true;
                                break;
                            }
                            // Stack exhaustion is also a limit, not a typo.
                            Err(fancy_regex::Error::RuntimeError(
                                fancy_regex::RuntimeError::StackOverflow,
                            )) => {
                                hit_backtrack = true;
                                break;
                            }
                            // Any other runtime error: treat as a failed rule.
                            Err(e) => {
                                problems.push(RegexProblem::Invalid {
                                    rule: rule.name.clone(),
                                    message: e.to_string(),
                                });
                                break;
                            }
                        }
                    }
                    if hit_backtrack {
                        problems.push(RegexProblem::BacktrackLimit {
                            rule: rule.name.clone(),
                        });
                    }
                }
                Err(e) => problems.push(RegexProblem::Invalid {
                    rule: rule.name.clone(),
                    message: e.to_string(),
                }),
            }
        }
    }

    // A rule that could not run is itself worth flagging: the user believes
    // that protection is active. Fold it in as a synthetic Review match so the
    // pasted text cannot be auto-approved on the strength of rules that never
    // executed.
    for problem in &problems {
        let (rule, explanation) = problem.describe();
        collector.add(Risk::Review, &rule, paste, &explanation);
    }

    Verdict {
        matched: collector.finish(),
        problems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_compiles() {
        // A panicking built-in would otherwise surface at first paste.
        assert_eq!(builtin_regexes().len(), REGEX_RULES.len());
    }

    #[test]
    fn an_invalid_user_regex_is_reported_not_ignored() {
        let cfg = crate::config::parse("[custom_rules]\nbroken | danger | (unclosed | oops\n");
        let v = classify_with("echo hi", Some(&cfg));
        assert!(v.has_problems());
        assert!(matches!(v.problems[0], RegexProblem::Invalid { .. }));
        // And it must not read as Safe.
        assert_eq!(v.matched.risk, Risk::Review);
    }

    #[test]
    fn a_backtracking_user_regex_fails_closed() {
        // The pattern docs/tcx.md section 8 measured; the textbook blowups do
        // not trigger this engine, a backreference does.
        // Note: `|` separates the rule's four fields, so a custom pattern
        // cannot contain alternation. That limitation is inherited from the
        // C++ format and applies identically to `tg`; see docs/tgr.md.
        // Only a *non-matching* subject forces exponential backtracking: with
        // a match, find() stops early and the limit is never approached.
        let cfg =
            crate::config::parse("[custom_rules]\nblowup | danger | (a{1,2})+\\1$ | pathological\n");
        let paste = "a".repeat(30) + "b";
        let v = classify_with(&paste, Some(&cfg));
        assert!(
            v.problems
                .iter()
                .any(|p| matches!(p, RegexProblem::BacktrackLimit { .. })),
            "expected a backtrack limit, got {:?}",
            v.problems
        );
        assert_ne!(v.matched.risk, Risk::Safe);
    }

    #[test]
    fn a_disabled_rule_never_fires() {
        let cfg = crate::config::parse("[disabled_rules]\nrm -rf\n");
        assert_eq!(classify_with("rm -rf /x", Some(&cfg)).matched.risk, Risk::Safe);
    }

    #[test]
    fn an_allowlisted_paste_is_safe_whatever_it_contains() {
        let cfg = crate::config::parse("[allowlist]\nrm -rf /tmp/allowed\n");
        assert_eq!(
            classify_with("rm -rf /tmp/allowed", Some(&cfg)).matched.risk,
            Risk::Safe
        );
    }
}
