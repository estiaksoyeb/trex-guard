#include "shell_parser.h"

#include <cctype>
#include <sstream>
#include <unordered_set>

namespace trex {

namespace {

bool is_ident_start(char c) {
    return std::isalpha(static_cast<unsigned char>(c)) || c == '_';
}

bool is_ident_char(char c) {
    return std::isalnum(static_cast<unsigned char>(c)) || c == '_';
}

bool is_env_assignment(const std::string& token) {
    size_t eq = token.find('=');
    if (eq == std::string::npos || eq == 0) return false;
    if (!is_ident_start(token[0])) return false;
    for (size_t i = 1; i < eq; ++i) {
        if (!is_ident_char(token[i])) return false;
    }
    return true;
}

std::string get_basename(const std::string& path) {
    size_t pos = path.find_last_of("/\\");
    if (pos == std::string::npos) {
        return path;
    }
    return path.substr(pos + 1);
}

const std::unordered_set<std::string> kKnownWrappers = {
    "sudo", "doas", "env", "nice", "nohup", "command", "builtin", "exec", "time", "xargs"
};

}  // namespace

bool ParsedCommand::has_flag(const std::string& flag) const {
    for (const auto& a : args) {
        if (a == flag) return true;
    }
    return false;
}

bool ParsedCommand::has_short_flag_char(char c) const {
    for (const auto& a : args) {
        if (a.size() >= 2 && a[0] == '-' && a[1] != '-') {
            if (a.find(c, 1) != std::string::npos) {
                return true;
            }
        }
    }
    return false;
}

bool ParsedCommand::has_long_flag(const std::string& name) const {
    std::string prefix = "--" + name;
    for (const auto& a : args) {
        if (a == prefix || a.rfind(prefix + "=", 0) == 0) {
            return true;
        }
    }
    return false;
}

bool ParsedCommand::has_arg(const std::string& arg) const {
    for (const auto& a : args) {
        if (a == arg) return true;
    }
    return false;
}

std::vector<std::string> ParsedCommand::positional_args() const {
    std::vector<std::string> pos;
    for (const auto& a : args) {
        if (!a.empty() && a[0] != '-') {
            pos.push_back(a);
        }
    }
    return pos;
}

static ParsedCommand build_command(const std::vector<std::string>& tokens, const std::string& raw_snippet) {
    ParsedCommand cmd;
    cmd.raw_snippet = raw_snippet;
    if (tokens.empty()) return cmd;

    size_t idx = 0;
    // 1. Collect leading environment variables
    while (idx < tokens.size() && is_env_assignment(tokens[idx])) {
        cmd.env_vars.push_back(tokens[idx]);
        idx++;
    }

    // 2. Unwrap wrappers (sudo, doas, env, nice, nohup, etc.)
    while (idx < tokens.size()) {
        std::string raw_tok = tokens[idx];
        std::string base = get_basename(raw_tok);
        if (kKnownWrappers.find(base) == kKnownWrappers.end()) {
            break;
        }

        cmd.wrappers.push_back(base);
        idx++;

        if (base == "sudo" || base == "doas") {
            while (idx < tokens.size() && !tokens[idx].empty() && tokens[idx][0] == '-') {
                if (tokens[idx] == "--") {
                    idx++;
                    break;
                }
                std::string flag = tokens[idx];
                idx++;
                // Skip argument for flags taking value (e.g. -u user, -g group, -p prompt)
                if ((flag == "-u" || flag == "-g" || flag == "-p" || flag == "-C") && idx < tokens.size()) {
                    idx++;
                }
            }
        } else if (base == "env") {
            while (idx < tokens.size() && !tokens[idx].empty()) {
                if (tokens[idx] == "-i" || tokens[idx] == "--ignore-environment") {
                    idx++;
                } else if ((tokens[idx] == "-u" || tokens[idx] == "--unset") && idx + 1 < tokens.size()) {
                    idx += 2;
                } else if (is_env_assignment(tokens[idx])) {
                    cmd.env_vars.push_back(tokens[idx]);
                    idx++;
                } else {
                    break;
                }
            }
        } else if (base == "nice") {
            while (idx < tokens.size() && !tokens[idx].empty() && tokens[idx][0] == '-') {
                std::string flag = tokens[idx];
                idx++;
                if (flag == "-n" && idx < tokens.size()) {
                    idx++;
                }
            }
        } else if (base == "command" || base == "builtin" || base == "exec" || base == "time" || base == "nohup") {
            while (idx < tokens.size() && !tokens[idx].empty() && tokens[idx][0] == '-') {
                if (tokens[idx] == "--") {
                    idx++;
                    break;
                }
                idx++;
            }
        }
    }

    if (idx < tokens.size()) {
        cmd.raw_executable = tokens[idx];
        cmd.base_name = get_basename(tokens[idx]);
        idx++;
    }

    while (idx < tokens.size()) {
        cmd.args.push_back(tokens[idx]);
        idx++;
    }

    return cmd;
}

std::vector<Pipeline> parse_shell_commands(const std::string& paste) {
    std::vector<Pipeline> pipelines;
    Pipeline current_pipeline;
    std::vector<std::string> current_tokens;
    std::string current_token;
    std::string current_stage_snippet;
    std::string current_pipeline_snippet;

    bool in_single_quote = false;
    bool in_double_quote = false;
    bool in_comment = false;

    auto finish_token = [&]() {
        if (!current_token.empty()) {
            current_tokens.push_back(current_token);
            current_token.clear();
        }
    };

    auto finish_stage = [&]() {
        finish_token();
        if (!current_tokens.empty()) {
            ParsedCommand cmd = build_command(current_tokens, current_stage_snippet);
            current_pipeline.stages.push_back(std::move(cmd));
            current_tokens.clear();
        }
        current_stage_snippet.clear();
    };

    auto finish_pipeline = [&]() {
        finish_stage();
        if (!current_pipeline.stages.empty()) {
            current_pipeline.raw_snippet = current_pipeline_snippet;
            pipelines.push_back(std::move(current_pipeline));
            current_pipeline = Pipeline{};
        }
        current_pipeline_snippet.clear();
    };

    size_t n = paste.size();
    for (size_t i = 0; i < n; ++i) {
        char c = paste[i];

        if (in_comment) {
            current_pipeline_snippet += c;
            if (c == '\n' || c == '\r') {
                in_comment = false;
                finish_pipeline();
            }
            continue;
        }

        if (in_single_quote) {
            current_stage_snippet += c;
            current_pipeline_snippet += c;
            if (c == '\'') {
                in_single_quote = false;
            } else {
                current_token += c;
            }
            continue;
        }

        if (in_double_quote) {
            current_stage_snippet += c;
            current_pipeline_snippet += c;
            if (c == '\\' && i + 1 < n) {
                char next = paste[i + 1];
                if (next == '\"' || next == '\\' || next == '$' || next == '`') {
                    current_token += next;
                    current_stage_snippet += next;
                    current_pipeline_snippet += next;
                    i++;
                    continue;
                }
            }
            if (c == '\"') {
                in_double_quote = false;
            } else {
                current_token += c;
            }
            continue;
        }

        // Outside quotes
        if (c == '\\') {
            if (i + 1 < n && paste[i + 1] == '\n') {
                // Line continuation
                current_stage_snippet += "\\\n";
                current_pipeline_snippet += "\\\n";
                i++;
                continue;
            }
            if (i + 1 < n) {
                current_stage_snippet += c;
                current_stage_snippet += paste[i + 1];
                current_pipeline_snippet += c;
                current_pipeline_snippet += paste[i + 1];
                current_token += paste[i + 1];
                i++;
                continue;
            }
        }

        if (c == '\'') {
            in_single_quote = true;
            current_stage_snippet += c;
            current_pipeline_snippet += c;
            continue;
        }

        if (c == '\"') {
            in_double_quote = true;
            current_stage_snippet += c;
            current_pipeline_snippet += c;
            continue;
        }

        if (c == '#' && (current_token.empty() && (current_stage_snippet.empty() || std::isspace(static_cast<unsigned char>(current_stage_snippet.back()))))) {
            in_comment = true;
            current_pipeline_snippet += c;
            continue;
        }

        // Check for pipeline (| or |&) vs logical OR (||)
        if (c == '|') {
            if (i + 1 < n && paste[i + 1] == '|') {
                // '||' statement separator
                current_pipeline_snippet += "||";
                finish_pipeline();
                i++;
                continue;
            }
            // Single '|' or '|&' is a pipe stage separator
            current_pipeline_snippet += c;
            if (i + 1 < n && paste[i + 1] == '&') {
                current_pipeline_snippet += '&';
                i++;
            }
            finish_stage();
            continue;
        }

        // Check for statement separators: ;, &&, &, \n
        if (c == ';') {
            current_pipeline_snippet += c;
            finish_pipeline();
            continue;
        }

        if (c == '&') {
            if (i + 1 < n && paste[i + 1] == '&') {
                // '&&'
                current_pipeline_snippet += "&&";
                finish_pipeline();
                i++;
                continue;
            }
            // Background '&'
            current_pipeline_snippet += c;
            finish_pipeline();
            continue;
        }

        if (c == '\n' || c == '\r') {
            current_pipeline_snippet += c;
            finish_pipeline();
            continue;
        }

        if (std::isspace(static_cast<unsigned char>(c))) {
            current_stage_snippet += c;
            current_pipeline_snippet += c;
            finish_token();
            continue;
        }

        // Regular character
        current_token += c;
        current_stage_snippet += c;
        current_pipeline_snippet += c;
    }

    finish_pipeline();
    return pipelines;
}

}  // namespace trex
