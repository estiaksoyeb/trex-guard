//! Risk levels and a match result.
//!
//! Ported from `src/rules.h`. The ordering matters: `Danger > Review > Safe`,
//! and the guard renders the *highest* risk found across every rule. Anything
//! that changes this ordering changes what the user is shown.

use std::fmt;

/// How dangerous a matched fragment is.
///
/// `PartialOrd` is derived from the declaration order, which gives
/// `Safe < Review < Danger` — the same ordering the C++ `enum class Risk`
/// uses and compares against with `static_cast<int>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Risk {
    /// Nothing matched.
    #[default]
    Safe,
    /// Executes, but is destructive enough to deserve a look first.
    Review,
    /// Refused unless explicitly approved.
    Danger,
}

impl fmt::Display for Risk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Risk::Safe => "safe",
            Risk::Review => "review",
            Risk::Danger => "danger",
        })
    }
}

/// One flagged fragment of a paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Worst risk among `matches`, or `Safe` when nothing was flagged.
    pub risk: Risk,
    /// Name of the rule that fired (empty when `Safe`).
    pub rule: String,
    /// The exact text that matched.
    pub snippet: String,
    /// Why it is risky, shown under the snippet.
    pub explanation: String,
    /// Every individual match, sorted Danger first (empty when `Safe`).
    pub matches: Vec<Match>,
}

impl Match {
    /// A paste with nothing flagged.
    pub fn safe() -> Match {
        Match {
            risk: Risk::Safe,
            rule: String::new(),
            snippet: String::new(),
            explanation: String::new(),
            matches: Vec::new(),
        }
    }

    /// Whether a human needs to see this before it runs.
    pub fn needs_review(&self) -> bool {
        self.risk != Risk::Safe
    }
}

impl Default for Match {
    fn default() -> Self {
        Match::safe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_ordering_is_safe_review_danger() {
        // The C++ guard compares these as integers; the same order here means
        // "take the max" is a plain comparison.
        assert!(Risk::Safe < Risk::Review);
        assert!(Risk::Review < Risk::Danger);
    }

    #[test]
    fn a_safe_match_needs_no_review() {
        assert!(!Match::safe().needs_review());
    }
}
