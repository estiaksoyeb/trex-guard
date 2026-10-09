#include "rules.h"
#include "config.h"
#include "shell_parser.h"

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

// Literal rules for exact substring matches (e.g. classic fork bombs)
const LiteralRule kLiteralRules[] = {
    {"fork bomb",
     Risk::Danger,
     "Self-replicating shell function that exhausts system process table",
     ":(){:|:&};:"},
};

const std::vector<RegexRule>& get_regex_rules() {
    static const std::vector<RegexRule> rules = {
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

        // Danger: Heredoc piped or fed directly into shell interpreter
        {"heredoc to shell",
         Risk::Danger,
         "Directly executes inline heredoc script into a shell interpreter",
         R"(\b(bash|sh|zsh)\s*<<\s*['"]?[A-Za-z0-9_]+['"]?|<<\s*['"]?[A-Za-z0-9_]+['"]?\s*\|\s*(sudo\s+)?(ba|z|a)?sh\b)"},

        // Danger: Dynamic shell eval
        {"shell eval",
         Risk::Danger,
         "Dynamic shell evaluation can execute arbitrary uninspected code",
         R"(\beval\s+[\$\"'])"},

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

std::string trim_snippet(const std::string& snip) {
    size_t start = snip.find_first_not_of("\r\n\t ;&|`$");
    if (start == std::string::npos) return "";
    size_t end = snip.find_last_not_of("\r\n\t ;&|`$");
    return snip.substr(start, end - start + 1);
}

bool is_shell_interpreter(const std::string& name) {
    return name == "sh" || name == "bash" || name == "zsh" ||
           name == "ash" || name == "dash" || name == "fish";
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
        if (config && config->disabled_rules.count(rule) > 0) {
            return;
        }
        std::string clean = trim_snippet(snippet);
        if (clean.empty()) return;

        for (auto& existing : all_matches) {
            if (existing.snippet == clean ||
                existing.snippet.find(clean) != std::string::npos ||
                clean.find(existing.snippet) != std::string::npos) {
                if (static_cast<int>(risk) > static_cast<int>(existing.risk) ||
                    (risk == existing.risk && clean.size() > existing.snippet.size())) {
                    existing.risk = risk;
                    existing.rule = rule;
                    existing.snippet = clean;
                    existing.explanation = exp;
                }
                return;
            }
        }
        Match m;
        m.risk = risk;
        m.rule = rule;
        m.snippet = clean;
        m.explanation = exp;
        all_matches.push_back(m);
    };

    // 2. Structured Semantic Pattern Recognition
    std::vector<Pipeline> pipelines = parse_shell_commands(paste);

    for (const auto& pipeline : pipelines) {
        // Pipeline Stage Correlation
        if (pipeline.stages.size() >= 2) {
            for (size_t i = 0; i < pipeline.stages.size() - 1; ++i) {
                const auto& upstream = pipeline.stages[i];
                for (size_t j = i + 1; j < pipeline.stages.size(); ++j) {
                    const auto& downstream = pipeline.stages[j];

                    // pipe to shell: curl/wget piped directly to shell
                    if ((upstream.base_name == "curl" || upstream.base_name == "wget") &&
                        is_shell_interpreter(downstream.base_name)) {
                        add_match(Risk::Danger, "pipe to shell", pipeline.raw_snippet,
                                  "Executes remote untrusted script directly in shell without prior inspection");
                    }

                    // encoded shell pipe: base64 -d piped to shell
                    if (upstream.base_name == "base64" &&
                        (upstream.has_flag("-d") || upstream.has_long_flag("decode") || upstream.has_short_flag_char('d')) &&
                        is_shell_interpreter(downstream.base_name)) {
                        add_match(Risk::Danger, "encoded shell pipe", pipeline.raw_snippet,
                                  "Decodes and directly executes obfuscated shell commands");
                    }
                }
            }
        }

        // Semantic analysis for individual command stages
        for (const auto& cmd : pipeline.stages) {
            const std::string& bin = cmd.base_name;
            std::string snip = cmd.raw_snippet;

            // Semantic: Git commands
            if (bin == "git") {
                // Skip global git flags to identify the subcommand
                size_t sub_idx = 0;
                while (sub_idx < cmd.args.size()) {
                    const std::string& a = cmd.args[sub_idx];
                    if (a == "-C" || a == "-c" || a == "--git-dir" || a == "--work-tree" ||
                        a == "--namespace" || a == "--exec-path" || a == "--config-env") {
                        sub_idx += 2;
                    } else if (a.rfind("--git-dir=", 0) == 0 || a.rfind("--work-tree=", 0) == 0 ||
                               a.rfind("-c", 0) == 0 || a.rfind("-C", 0) == 0) {
                        sub_idx += 1;
                    } else if (a == "--no-pager" || a == "-p" || a == "--paginate" ||
                               a == "--bare" || a == "--no-replace-objects" || a == "--literal-pathspecs" ||
                               a == "-v" || a == "--version" || a == "-h" || a == "--help") {
                        sub_idx += 1;
                    } else if (!a.empty() && a[0] == '-') {
                        sub_idx += 1;
                    } else {
                        break;
                    }
                }

                if (sub_idx < cmd.args.size()) {
                    std::string subcommand = cmd.args[sub_idx];
                    std::vector<std::string> sub_args(cmd.args.begin() + sub_idx + 1, cmd.args.end());

                    if (subcommand == "reset") {
                        bool has_hard = false;
                        for (const auto& sa : sub_args) {
                            if (sa == "--hard" || sa.rfind("--hard=", 0) == 0) {
                                has_hard = true;
                                break;
                            }
                        }
                        if (has_hard) {
                            add_match(Risk::Review, "git hard reset", snip,
                                      "Discards all uncommitted changes and resets branch head");
                        }
                    } else if (subcommand == "push") {
                        bool has_force = false;
                        for (const auto& sa : sub_args) {
                            if (sa == "--force" || sa == "-f" || sa == "--force-with-lease" ||
                                (sa.size() >= 2 && sa[0] == '-' && sa[1] != '-' && sa.find('f') != std::string::npos)) {
                                has_force = true;
                                break;
                            }
                        }
                        if (has_force) {
                            add_match(Risk::Review, "git force push", snip,
                                      "Overwrites remote repository history, potentially destroying remote commits");
                        }
                    } else if (subcommand == "clean") {
                        bool has_force = false;
                        for (const auto& sa : sub_args) {
                            if (sa == "--force" || sa == "-f" ||
                                (sa.size() >= 2 && sa[0] == '-' && sa[1] != '-' && sa.find('f') != std::string::npos)) {
                                has_force = true;
                                break;
                            }
                        }
                        if (has_force) {
                            add_match(Risk::Review, "git clean", snip,
                                      "Permanently deletes untracked files and directories from the repository");
                        }
                    } else if (subcommand == "checkout") {
                        bool is_destructive = false;
                        for (size_t k = 0; k < sub_args.size(); ++k) {
                            const auto& sa = sub_args[k];
                            if (sa == "-f" || sa == "--force" || sa == ".") {
                                is_destructive = true;
                                break;
                            }
                            if (sa == "--" && k + 1 < sub_args.size() && sub_args[k + 1] == ".") {
                                is_destructive = true;
                                break;
                            }
                        }
                        if (is_destructive) {
                            add_match(Risk::Review, "git checkout/restore", snip,
                                      "Discards modified working tree state across working tree files");
                        }
                    } else if (subcommand == "restore") {
                        bool is_destructive = false;
                        for (const auto& sa : sub_args) {
                            if (sa == "." || sa == "--worktree") {
                                is_destructive = true;
                                break;
                            }
                        }
                        if (is_destructive) {
                            add_match(Risk::Review, "git checkout/restore", snip,
                                      "Discards modified working tree state across working tree files");
                        }
                    }
                }
            }

            // Semantic: rm command
            if (bin == "rm") {
                bool has_recursive = cmd.has_short_flag_char('r') || cmd.has_short_flag_char('R') || cmd.has_long_flag("recursive");
                bool has_force = cmd.has_short_flag_char('f') || cmd.has_long_flag("force");

                if (has_recursive && has_force) {
                    add_match(Risk::Danger, "rm -rf", snip,
                              "Recursive force deletion permanently removes directories without prompt");
                } else if (!cmd.args.empty()) {
                    bool only_help = (cmd.has_long_flag("help") || cmd.has_long_flag("version")) && cmd.args.size() == 1;
                    if (!only_help) {
                        add_match(Risk::Review, "file deletion", snip,
                                  "Permanently deletes files or directories from the filesystem");
                    }
                }
            }

            // Semantic: other file deletion tools (rmdir, unlink, shred)
            if (bin == "rmdir" || bin == "unlink" || bin == "shred") {
                add_match(Risk::Review, "file deletion", snip,
                          "Permanently deletes files or directories from the filesystem");
            }

            // Semantic: chmod dangerous/recursive
            if (bin == "chmod") {
                bool has_recursive = cmd.has_short_flag_char('R') || cmd.has_long_flag("recursive");
                bool has_danger_mode = false;
                bool has_root_target = false;
                for (const auto& a : cmd.args) {
                    if (a == "777" || a == "000" || a == "a+rwx" || a == "u=rwx,go=rwx" ||
                        a.find("777") != std::string::npos || a.find("000") != std::string::npos) {
                        has_danger_mode = true;
                    }
                    if (a == "/" || a == "/*" || a == "/root") {
                        has_root_target = true;
                    }
                }
                if ((has_recursive && has_danger_mode) || (has_root_target && (has_danger_mode || has_recursive))) {
                    add_match(Risk::Danger, "chmod", snip,
                              "Broad or recursive permission changes compromise system security and file integrity");
                }
            }

            // Semantic: find -delete / -exec rm
            if (bin == "find") {
                bool has_del = false;
                for (size_t k = 0; k < cmd.args.size(); ++k) {
                    if (cmd.args[k] == "-delete") {
                        has_del = true;
                        break;
                    }
                    if (cmd.args[k] == "-exec" && k + 1 < cmd.args.size() && cmd.args[k + 1] == "rm") {
                        has_del = true;
                        break;
                    }
                }
                if (has_del) {
                    add_match(Risk::Danger, "find delete", snip,
                              "Unprompted bulk deletion of matched filesystem objects");
                }
            }

            // Semantic: subshell bash/sh/zsh -c
            if (is_shell_interpreter(bin) && cmd.has_flag("-c")) {
                add_match(Risk::Review, "subshell execution", snip,
                          "Executes inline command string within an explicit subshell or elevated context");
            }
        }
    }

    // 3. Literal substring rules (e.g. fork bombs)
    for (const auto& r : kLiteralRules) {
        size_t pos = 0;
        while ((pos = paste.find(r.needle, pos)) != std::string::npos) {
            add_match(r.risk, r.name, r.needle, r.explanation);
            pos += std::strlen(r.needle);
        }
    }

    // 4. Regex rules (for block device redirects, mkfs, dd, inline eval, heredocs, etc.)
    const auto& regex_rules = get_regex_rules();
    for (const auto& r : regex_rules) {
        for (auto it = std::sregex_iterator(paste.begin(), paste.end(), r.pattern);
             it != std::sregex_iterator(); ++it) {
            std::string snip = it->str();
            add_match(r.risk, r.name, snip, r.explanation);
        }
    }

    // 5. User custom rules from configuration
    if (config) {
        for (const auto& cr : config->custom_rules) {
            try {
                std::regex re(cr.pattern);
                for (auto it = std::sregex_iterator(paste.begin(), paste.end(), re);
                     it != std::sregex_iterator(); ++it) {
                    std::string snip = it->str();
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
