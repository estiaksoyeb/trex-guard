#include "rules.h"
#include "config.h"

#include <regex>
#include <string>
#include <vector>

namespace trex {

namespace {

struct RegexRule {
    const char* name;
    Risk risk;
    const char* explanation;
    std::regex pattern;

    RegexRule(const char* n, Risk r, const char* exp, const char* pat,
              std::regex_constants::syntax_option_type flags = std::regex_constants::ECMAScript)
        : name(n), risk(r), explanation(exp), pattern(pat, flags) {}
};

struct LiteralRule {
    const char* name;
    Risk risk;
    const char* explanation;
    const char* needle;
};

// Literal rules for exact substring matches
const LiteralRule kLiteralRules[] = {
    {"fork bomb",
     Risk::Danger,
     "Self-replicating shell function that exhausts system process table",
     ":(){:|:&};:"},
    {"rm -rf",
     Risk::Danger,
     "Recursive force deletion permanently removes directories without prompt",
     "rm -rf"},
    {"rm -rf",
     Risk::Danger,
     "Recursive force deletion permanently removes directories without prompt",
     "rm -fr"},
    {"rm -rf",
     Risk::Danger,
     "Recursive force deletion permanently removes directories without prompt",
     "rm -r -f"},
    {"rm -rf",
     Risk::Danger,
     "Recursive force deletion permanently removes directories without prompt",
     "rm -f -r"},
};

const std::vector<RegexRule>& get_regex_rules() {
    static const std::vector<RegexRule> rules = {
        // Danger: Recursive / force rm variations
        {"rm -rf",
         Risk::Danger,
         "Recursive force deletion permanently removes directories without prompt",
         R"(\brm\s+([^\r\n]*\s)?-(?:[a-zA-Z]*r[a-zA-Z]*f|[a-zA-Z]*f[a-zA-Z]*r)\b)"},
        {"rm -rf",
         Risk::Danger,
         "Recursive force deletion permanently removes directories without prompt",
         R"(\brm\s+.*--recursive\s+.*--force|\brm\s+.*--force\s+.*--recursive)"},

        // Danger: Filesystem creation / formatting
        {"mkfs",
         Risk::Danger,
         "Creates a filesystem and overwrites existing partition data",
         R"(\bmkfs(\.[a-zA-Z0-9]+)?\b)"},

        // Danger: dd operations reading raw inputs
        {"dd",
         Risk::Danger,
         "Low-level block read/write operation can destroy partition tables and filesystems",
         R"(\bdd\s+.*if=)"},

        // Danger: Overwriting raw block devices
        {"write to block device",
         Risk::Danger,
         "Direct write to a raw block device bypasses filesystems and causes data loss",
         R"(>\s*/dev/(sd[a-z]|nvme[0-9]|vd[a-z]|hd[a-z]|mmcblk))"},

        // Danger: Dangerous or recursive permissions changes
        {"chmod",
         Risk::Danger,
         "Broad or recursive permission changes compromise system security and file integrity",
         R"(\bchmod\s+.*(-R\s+)?(777|000)\s+/|\bchmod\s+.*-R\s+(777|000))"},

        // Danger: Download piped directly into a shell interpreter
        {"pipe to shell",
         Risk::Danger,
         "Executes remote untrusted script directly in shell without prior inspection",
         R"(\b(curl|wget)\s+[^|\r\n]+\|\s*(sudo\s+)?(ba|z|a)?sh\b)"},

        // Danger: Find with delete / exec rm
        {"find delete",
         Risk::Danger,
         "Unprompted bulk deletion of matched filesystem objects",
         R"(\bfind\s+.*(-delete\b|-exec\s+rm\b))"},

        // Review: Git force push
        {"git force push",
         Risk::Review,
         "Overwrites remote repository history, potentially destroying remote commits",
         R"(\bgit\s+push\s+.*(--force\b|-f\b|--force-with-lease))"},

        // Review: Git destructive reset
        {"git hard reset",
         Risk::Review,
         "Discards all uncommitted changes and resets branch head",
         R"(\bgit\s+reset\s+--hard\b)"},

        // Review: Git clean destructive actions
        {"git clean",
         Risk::Review,
         "Permanently deletes untracked files and directories from the repository",
         R"(\bgit\s+clean\s+.*(-[a-zA-Z]*f[a-zA-Z]*\b|--force\b))"},

        // Review: Git checkout/restore entire working tree
        {"git checkout/restore",
         Risk::Review,
         "Discards modified working tree state across working tree files",
         R"(\bgit\s+checkout\b.*(--\s+\.|-f\b)|\bgit\s+restore\b.*(?:\s\.(?:\s|$)|--worktree))"},
    };
    return rules;
}

}  // namespace

Match classify(const std::string& paste, const Config* config) {
    // 1. Check allowlist from configuration
    if (config) {
        for (const auto& item : config->allowlist) {
            if (!item.empty() && paste.find(item) != std::string::npos) {
                return Match{}; // Allowlisted pastes are considered Safe
            }
        }
    }

    Match review_match;

    // 2. Check literal substring rules
    for (const auto& r : kLiteralRules) {
        if (config && config->disabled_rules.count(r.name) > 0) {
            continue;
        }
        if (paste.find(r.needle) != std::string::npos) {
            Match m;
            m.risk = r.risk;
            m.rule = r.name;
            m.snippet = r.needle;
            m.explanation = r.explanation;
            if (m.risk == Risk::Danger)
                return m;
            if (review_match.risk == Risk::Safe)
                review_match = m;
        }
    }

    // 3. Check regex pattern rules
    const auto& regex_rules = get_regex_rules();
    for (const auto& r : regex_rules) {
        if (config && config->disabled_rules.count(r.name) > 0) {
            continue;
        }
        std::smatch sm;
        if (std::regex_search(paste, sm, r.pattern)) {
            Match m;
            m.risk = r.risk;
            m.rule = r.name;
            m.snippet = sm.str();
            m.explanation = r.explanation;
            if (m.risk == Risk::Danger)
                return m;
            if (review_match.risk == Risk::Safe)
                review_match = m;
        }
    }

    // 4. Check user custom rules from configuration
    if (config) {
        for (const auto& cr : config->custom_rules) {
            if (config->disabled_rules.count(cr.name) > 0) {
                continue;
            }
            try {
                std::regex re(cr.pattern);
                std::smatch sm;
                if (std::regex_search(paste, sm, re)) {
                    Match m;
                    m.risk = cr.risk;
                    m.rule = cr.name;
                    m.snippet = sm.str();
                    m.explanation = cr.explanation;
                    if (m.risk == Risk::Danger)
                        return m;
                    if (review_match.risk == Risk::Safe)
                        review_match = m;
                }
            } catch (const std::regex_error&) {
                // Ignore invalid user regexes gracefully
            }
        }
    }

    return review_match;
}

}  // namespace trex
