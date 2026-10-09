#include "rules.h"
#include "config.h"

#include <algorithm>
#include <cstring>
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
};

const std::vector<RegexRule>& get_regex_rules() {
    static const std::vector<RegexRule> rules = {
        // Danger: Recursive / force rm variations
        {"rm -rf",
         Risk::Danger,
         "Recursive force deletion permanently removes directories without prompt",
         R"(\brm\s+([^\r\n]*\s)?-(?:[a-zA-Z]*r[a-zA-Z]*f|[a-zA-Z]*f[a-zA-Z]*r)(\s+[^\r\n]*\S)?)"},
        {"rm -rf",
         Risk::Danger,
         "Recursive force deletion permanently removes directories without prompt",
         R"(\brm\s+.*--recursive\s+.*--force(\s+[^\r\n]*\S)?|\brm\s+.*--force\s+.*--recursive(\s+[^\r\n]*\S)?)"},
        {"rm -rf",
         Risk::Danger,
         "Recursive force deletion permanently removes directories without prompt",
         R"(\brm\s+.*-[a-zA-Z]*r[a-zA-Z]*\s+.*-[a-zA-Z]*f[a-zA-Z]*(\s+[^\r\n]*\S)?|\brm\s+.*-[a-zA-Z]*f[a-zA-Z]*\s+.*-[a-zA-Z]*r[a-zA-Z]*(\s+[^\r\n]*\S)?)"},

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
         R"(\bgit\s+checkout\b.*(?:--\s+\.|\s\.(?:\s|$)|-f\b)|\bgit\s+restore\b.*(?:\s\.(?:\s|$)|--worktree))"},

        // Review: File deletion (single file or non-recursive rm, rmdir, unlink, shred)
        {"file deletion",
         Risk::Review,
         "Permanently deletes files or directories from the filesystem",
         R"((?:^|[^a-zA-Z0-9_.-])(rm|rmdir|unlink|shred)\s+([^\r\n]*\S))"},

        // Danger: Dynamic shell eval
        {"shell eval",
         Risk::Danger,
         "Dynamic shell evaluation can execute arbitrary uninspected code",
         R"(\beval\s+[\$\"'])"},

        // Danger: Encoded payload execution via shell pipe
        {"encoded shell pipe",
         Risk::Danger,
         "Decodes and directly executes obfuscated shell commands",
         R"(\bbase64\s+(-d|--decode)\b.*\|\s*(sudo\s+)?(ba|z|a)?sh\b)"},

        // Danger: Heredoc piped or fed directly into shell interpreter
        {"heredoc to shell",
         Risk::Danger,
         "Directly executes inline heredoc script into a shell interpreter",
         R"(\b(bash|sh|zsh)\s*<<\s*['"]?[A-Za-z0-9_]+['"]?|<<\s*['"]?[A-Za-z0-9_]+['"]?\s*\|\s*(sudo\s+)?(ba|z|a)?sh\b)"},

        // Danger: Scripting interpreter executing system process one-liners
        {"inline script execution",
         Risk::Danger,
         "Executes system process or dynamic code through scripting interpreter one-liner",
         R"(\bpython[0-9.]*\s+-c\s+.*(os\.system|subprocess\.|exec\(|eval\()|\bperl\s+-e\s+.*(system|exec)|\bruby\s+-e\s+.*(system|exec))"},

        // Review: Subshell execution
        {"subshell execution",
         Risk::Review,
         "Executes inline command string within an explicit subshell or elevated context",
         R"(\b(sudo\s+)?(bash|sh|zsh)\s+-c\s+)"},
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

    std::vector<Match> all_matches;
    auto add_match = [&](Risk risk, const std::string& rule, const std::string& snippet, const std::string& exp) {
        for (auto& existing : all_matches) {
            if (existing.snippet == snippet ||
                existing.snippet.find(snippet) != std::string::npos ||
                snippet.find(existing.snippet) != std::string::npos) {
                if (static_cast<int>(risk) > static_cast<int>(existing.risk) ||
                    (risk == existing.risk && snippet.size() > existing.snippet.size())) {
                    existing.risk = risk;
                    existing.rule = rule;
                    existing.snippet = snippet;
                    existing.explanation = exp;
                }
                return;
            }
        }
        Match m;
        m.risk = risk;
        m.rule = rule;
        m.snippet = snippet;
        m.explanation = exp;
        all_matches.push_back(m);
    };

    // 2. Check literal substring rules
    for (const auto& r : kLiteralRules) {
        if (config && config->disabled_rules.count(r.name) > 0) {
            continue;
        }
        size_t pos = 0;
        while ((pos = paste.find(r.needle, pos)) != std::string::npos) {
            add_match(r.risk, r.name, r.needle, r.explanation);
            pos += std::strlen(r.needle);
        }
    }

    // 3. Check regex pattern rules
    const auto& regex_rules = get_regex_rules();
    for (const auto& r : regex_rules) {
        if (config && config->disabled_rules.count(r.name) > 0) {
            continue;
        }
        for (auto it = std::sregex_iterator(paste.begin(), paste.end(), r.pattern);
             it != std::sregex_iterator(); ++it) {
            std::string snip = it->str();
            size_t start = snip.find_first_not_of("\r\n\t ;&|`$");
            if (start != std::string::npos && start > 0) {
                snip = snip.substr(start);
            }
            add_match(r.risk, r.name, snip, r.explanation);
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
                for (auto it = std::sregex_iterator(paste.begin(), paste.end(), re);
                     it != std::sregex_iterator(); ++it) {
                    std::string snip = it->str();
                    size_t start = snip.find_first_not_of("\r\n\t ;&|`$");
                    if (start != std::string::npos && start > 0) {
                        snip = snip.substr(start);
                    }
                    add_match(cr.risk, cr.name, snip, cr.explanation);
                }
            } catch (const std::regex_error&) {
                // Ignore invalid user regexes gracefully
            }
        }
    }

    if (all_matches.empty()) {
        return Match{};
    }

    // Sort matches: Danger first, then Review
    std::stable_sort(all_matches.begin(), all_matches.end(), [](const Match& a, const Match& b) {
        return static_cast<int>(a.risk) > static_cast<int>(b.risk);
    });

    Match primary = all_matches.front();
    primary.matches = std::move(all_matches);
    return primary;
}

}  // namespace trex
