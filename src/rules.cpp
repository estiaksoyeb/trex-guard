#include "rules.h"

#include <regex>
#include <string>
#include <vector>

namespace trex {

namespace {

struct RegexRule {
    const char* name;
    std::regex pattern;

    RegexRule(const char* n, const char* pat, std::regex_constants::syntax_option_type flags = std::regex_constants::ECMAScript)
        : name(n), pattern(pat, flags) {}
};

struct LiteralRule {
    const char* name;
    const char* needle;
};

// Literal rules for exact substring matches
const LiteralRule kLiteralRules[] = {
    {"fork bomb", ":(){:|:&};:"},
    {"rm -rf", "rm -rf"},
    {"rm -rf", "rm -fr"},
    {"rm -rf", "rm -r -f"},
    {"rm -rf", "rm -f -r"},
};

const std::vector<RegexRule>& get_regex_rules() {
    static const std::vector<RegexRule> rules = {
        // Recursive / force rm variations
        {"rm -rf",
         R"(\brm\s+([^\r\n]*\s)?-(?:[a-zA-Z]*r[a-zA-Z]*f|[a-zA-Z]*f[a-zA-Z]*r)\b)"},
        {"rm -rf",
         R"(\brm\s+.*--recursive\s+.*--force|\brm\s+.*--force\s+.*--recursive)"},

        // Filesystem creation / formatting
        {"mkfs",
         R"(\bmkfs(\.[a-zA-Z0-9]+)?\b)"},

        // dd operations reading raw inputs
        {"dd",
         R"(\bdd\s+.*if=)"},

        // Overwriting raw block devices
        {"write to block device",
         R"(>\s*/dev/(sd[a-z]|nvme[0-9]|vd[a-z]|hd[a-z]|mmcblk))"},

        // Dangerous or recursive permissions changes
        {"chmod",
         R"(\bchmod\s+.*(-R\s+)?(777|000)\s+/|\bchmod\s+.*-R\s+(777|000))"},

        // Download piped directly into a shell interpreter
        {"pipe to shell",
         R"(\b(curl|wget)\s+[^|\r\n]+\|\s*(sudo\s+)?(ba|z|a)?sh\b)"},

        // Git force push
        {"git force push",
         R"(\bgit\s+push\s+.*(--force\b|-f\b|--force-with-lease))"},

        // Git destructive reset
        {"git hard reset",
         R"(\bgit\s+reset\s+--hard\b)"},

        // Git clean destructive actions
        {"git clean",
         R"(\bgit\s+clean\s+.*(-[a-zA-Z]*f[a-zA-Z]*\b|--force\b))"},

        // Git checkout/restore entire working tree
        {"git checkout/restore",
         R"(\bgit\s+checkout\b.*(--\s+\.|-f\b)|\bgit\s+restore\b.*(?:\s\.(?:\s|$)|--worktree))"},

        // Find with delete / exec rm
        {"find delete",
         R"(\bfind\s+.*(-delete\b|-exec\s+rm\b))"},
    };
    return rules;
}

}  // namespace

Match classify(const std::string& paste) {
    // Check literal substring rules first
    for (const auto& r : kLiteralRules) {
        if (paste.find(r.needle) != std::string::npos) {
            Match m;
            m.risk = Risk::Danger;
            m.rule = r.name;
            return m;
        }
    }

    // Check regex pattern rules
    const auto& regex_rules = get_regex_rules();
    for (const auto& r : regex_rules) {
        if (std::regex_search(paste, r.pattern)) {
            Match m;
            m.risk = Risk::Danger;
            m.rule = r.name;
            return m;
        }
    }

    return Match{};
}

}  // namespace trex
