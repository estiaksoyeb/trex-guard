#pragma once

#include "rules.h"

#include <string>
#include <unordered_set>
#include <vector>

namespace trex {

struct CustomRule {
    std::string name;
    Risk risk = Risk::Review;
    std::string pattern;
    std::string explanation;
};

struct Config {
    bool auto_approve_safe = false;
    std::vector<std::string> allowlist;             // strings or regexes to treat as safe
    std::unordered_set<std::string> disabled_rules; // rule names to ignore
    std::vector<CustomRule> custom_rules;           // user-defined rules
};

// Gets default configuration path: $XDG_CONFIG_HOME/trex-guard/config.ini or ~/.config/trex-guard/config.ini
std::string get_default_config_path();

// Loads configuration from a given file path (or default path if empty).
// If file does not exist, returns default Config.
Config load_config(const std::string& path = "");

// Parse configuration from an INI string directly (useful for testing).
Config parse_config_string(const std::string& content);

}  // namespace trex
