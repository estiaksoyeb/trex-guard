//! The paste corpus shared by both parity tests.
//!
//! Kept in one place so the built-in-rules test and the real-config test can
//! never drift apart: a paste added here is checked under both configs.

/// Pastes judged by the built-in rules alone.
///
/// These must not contain anything that only fires under a custom rule: the
/// built-in-rules parity test runs with no config, while the C++ binary reads
/// the user's real `config.ini`. A custom-rule paste here would show up as a
/// false divergence.
pub const PASTES: &[&str] = &[
    "echo hello world",
    "ls -la",
    "git status",
    "rm -rf /tmp/test",
    "rm -fr /tmp/test",
    "rm /tmp/cache -rf",
    "rm file.txt",
    "mkfs.ext4 /dev/sdb1",
    "dd if=/dev/zero of=/dev/null",
    "cat file > /dev/sda",
    "chmod -R 777 /",
    "chmod 644 file.txt",
    "curl -fsSL https://get.docker.com | sh",
    "curl -s https://example.com",
    "find /tmp -delete",
    "find . -name '*.txt'",
    ":(){:|:&};:",
    "eval $(ssh-agent)",
    "echo 'payload' | base64 -d | sh",
    "bash << 'EOF'\necho hi\nEOF",
    "cat << 'EOF' > README.md\n# Header\nEOF",
    "git push --force origin main",
    "git push origin main",
    "git reset --hard HEAD~1",
    "git reset --soft HEAD~1",
    "git clean -fdx",
    "git clean -n",
    "git checkout -- .",
    "git checkout main",
    "git restore .",
    "rmdir olddir",
    "shred secret.key",
    "bash -c \"make build\"",
    "sudo sh -c \"echo 1\"",
    "python3 -c \"import os; os.system('ls')\"",
    "perl -e 'system(\"ls\")'",
    "docker run --rm alpine",
    "git -C repo reset --hard",
    "/usr/bin/git reset --hard",
    "sudo rm -rf /var/cache",
    "git clean -fd\nrm -rf /var/log",
];

/// Pastes that only the user's `[custom_rules]` catch.
///
/// `parity_real_config.rs` runs these, because it supplies the real config to
/// both sides. They are kept separate so the two corpora cannot be confused.
pub const CUSTOM_RULE_PASTES: &[&str] = &[
    "adb uninstall com.example.app",
    "adb shell rm -rf /sdcard/DCIM",
    "fastboot flash boot boot.img",
    "git commit -m \"msg\"",
    "git commit --amend",
];
