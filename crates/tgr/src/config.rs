//! `config.ini` parsing.
//!
//! Ported from `src/config.cpp`, line for line in behaviour.
//!
//! # Same file as `tg`, on purpose
//!
//! `tgr` reads `$XDG_CONFIG_HOME/trex-guard/config.ini` — the *same* file the
//! C++ guard reads. That is deliberate: the user's 22 custom rules, allowlist
//! and disabled rules must apply identically to both binaries, or `tgr` is
//! guarding a different system than the one being configured.
//!
//! Sharing the file is safe because the file is read-only to both. The reason
//! `tcx` must never write here (`docs/tcx.md` §4) still holds, and still is
//! not true of `tgr`: it only ever reads.
//!
//! # Parsed by precedence
//!
//! Headerless lines are ignored, as in C++. A section header is
//! `[name]`; its name is lowercased. Comments start with `#` or `;` only when
//! they begin the line — a trailing `#` is part of the value, matching the C++.

use std::path::PathBuf;

use crate::risk::Risk;

/// A rule from `[custom_rules]`: `name | risk | pattern | explanation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomRule {
    pub name: String,
    pub risk: Risk,
    /// A regex pattern, compiled lazily by [`crate::rules`].
    pub pattern: String,
    pub explanation: String,
}

/// The parsed configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub auto_approve_safe: bool,
    /// When false, the `Risk: ...` explanation line is hidden.
    pub show_risk_explanation: bool,
    /// Substrings that make a whole paste `Safe`. Matched by substring, not
    /// regex, despite the C++ comment calling them "strings or regexes".
    pub allowlist: Vec<String>,
    pub disabled_rules: std::collections::HashSet<String>,
    pub custom_rules: Vec<CustomRule>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            auto_approve_safe: true,
            show_risk_explanation: true,
            allowlist: Vec::new(),
            disabled_rules: std::collections::HashSet::new(),
            custom_rules: Vec::new(),
        }
    }
}

/// `$XDG_CONFIG_HOME/trex-guard/config.ini`, else `~/.config/...`.
///
/// Identical to [`src/config.cpp::get_default_config_path`]. An empty result
/// (no `HOME`, no XDG) means "no config", which is a valid state: defaults.
pub fn default_path() -> PathBuf {
    if let Some(xdg) = nonempty_env("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("trex-guard").join("config.ini");
    }
    if let Some(home) = nonempty_env("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("trex-guard")
            .join("config.ini");
    }
    PathBuf::new()
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Reads the config file, falling back to defaults when it is missing.
///
/// A missing or unreadable file is **not** an error: the C++ guard guards with
/// defaults rather than refusing to start, and a guard that will not start
/// protects nothing.
pub fn load(path: Option<&std::path::Path>) -> Config {
    let target = match path {
        Some(p) => p.to_path_buf(),
        None => default_path(),
    };
    if target.as_os_str().is_empty() {
        return Config::default();
    }
    match std::fs::read_to_string(&target) {
        Ok(text) => parse(&text),
        Err(_) => Config::default(),
    }
}

/// Parses INI text.
pub fn parse(content: &str) -> Config {
    let mut cfg = Config::default();
    let mut section = String::new();

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_lowercase();
            continue;
        }

        match section.as_str() {
            "general" => {
                if let Some(pos) = line.find('=') {
                    let key = line[..pos].trim().to_lowercase();
                    let val = line[pos + 1..].trim().to_lowercase();
                    let truthy = val == "true" || val == "1" || val == "yes";
                    match key.as_str() {
                        "auto_approve_safe" => cfg.auto_approve_safe = truthy,
                        "show_risk_explanation" => cfg.show_risk_explanation = truthy,
                        // `compact_review = true` *hides* the explanation.
                        "compact_review" | "compact" => cfg.show_risk_explanation = !truthy,
                        _ => {}
                    }
                }
            }
            "allowlist" => cfg.allowlist.push(line.to_string()),
            "disabled_rules" => {
                cfg.disabled_rules.insert(line.to_string());
            }
            "custom_rules" => {
                let parts: Vec<&str> = line.split('|').map(str::trim).collect();
                if parts.len() >= 3 {
                    let risk = match parts[1].to_lowercase().as_str() {
                        "danger" | "high" | "high_risk" => Risk::Danger,
                        _ => Risk::Review,
                    };
                    cfg.custom_rules.push(CustomRule {
                        name: parts[0].to_string(),
                        risk,
                        pattern: parts[2].to_string(),
                        explanation: parts.get(3).map(|s| s.to_string()).unwrap_or_default(),
                    });
                }
            }
            _ => {}
        }
    }

    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_cpp_struct() {
        let cfg = Config::default();
        assert!(cfg.auto_approve_safe);
        assert!(cfg.show_risk_explanation);
        assert!(cfg.allowlist.is_empty());
        assert!(cfg.custom_rules.is_empty());
    }

    #[test]
    fn parses_the_sections_the_guard_uses() {
        let cfg = parse(
            "[general]\nauto_approve_safe = false\n\n[allowlist]\nrm -rf /tmp/ok\n\n\
             [disabled_rules]\nfork bomb\n\n[custom_rules]\ndrop db | danger | \\bdrop\\b | drops\n",
        );
        assert!(!cfg.auto_approve_safe);
        assert_eq!(cfg.allowlist, vec!["rm -rf /tmp/ok"]);
        assert!(cfg.disabled_rules.contains("fork bomb"));
        assert_eq!(cfg.custom_rules.len(), 1);
        assert_eq!(cfg.custom_rules[0].risk, Risk::Danger);
        assert_eq!(cfg.custom_rules[0].name, "drop db");
    }

    #[test]
    fn compact_review_hides_the_explanation() {
        assert!(!parse("[general]\ncompact_review = true\n").show_risk_explanation);
    }

    #[test]
    fn a_custom_rule_needs_three_fields() {
        assert!(parse("[custom_rules]\nonly | two\n").custom_rules.is_empty());
    }

    #[test]
    fn an_unknown_risk_word_is_review_not_danger() {
        // Fails safe in the *quiet* direction, matching C++: an unrecognised
        // risk must not silently escalate everything to Danger either.
        let cfg = parse("[custom_rules]\nn | nonsense | x | y\n");
        assert_eq!(cfg.custom_rules[0].risk, Risk::Review);
    }

    #[test]
    fn a_missing_file_is_defaults_not_a_failure() {
        let cfg = load(Some(std::path::Path::new("/nonexistent/tgr/config.ini")));
        assert_eq!(cfg, Config::default());
    }
}
