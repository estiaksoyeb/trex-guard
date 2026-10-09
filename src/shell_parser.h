#pragma once

#include <string>
#include <vector>

namespace trex {

struct ParsedCommand {
    std::string raw_snippet;             // Original text of this command/stage
    std::string base_name;               // Normalized executable basename (e.g. "git", "rm")
    std::string raw_executable;          // Original executable string before path stripping
    std::vector<std::string> wrappers;   // Unwrapped wrappers (e.g. "sudo", "env", "nice")
    std::vector<std::string> env_vars;   // Leading environment assignments (e.g. "FOO=1")
    std::vector<std::string> args;       // Arguments passed to executable (both flags & positionals)

    // Helper inspect methods
    bool has_flag(const std::string& flag) const;
    bool has_short_flag_char(char c) const;
    bool has_long_flag(const std::string& name) const;
    bool has_arg(const std::string& arg) const;
    std::vector<std::string> positional_args() const;
};

struct Pipeline {
    std::vector<ParsedCommand> stages;
    std::string raw_snippet;
};

// Parses a raw paste string into shell pipelines and commands.
// Handles quotes (', "), escapes, comments (#), statement separators (;, &&, ||, &, newlines),
// and pipeline connections (|).
std::vector<Pipeline> parse_shell_commands(const std::string& paste);

}  // namespace trex
