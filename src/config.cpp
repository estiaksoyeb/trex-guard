#include "config.h"

#include <cstdlib>
#include <fstream>
#include <sstream>
#include <algorithm>
#include <cctype>

namespace trex {

namespace {

std::string trim(const std::string& str) {
    size_t start = 0;
    while (start < str.size() && std::isspace(static_cast<unsigned char>(str[start]))) {
        start++;
    }
    size_t end = str.size();
    while (end > start && std::isspace(static_cast<unsigned char>(str[end - 1]))) {
        end--;
    }
    return str.substr(start, end - start);
}

std::vector<std::string> split(const std::string& str, char delim) {
    std::vector<std::string> tokens;
    std::string token;
    std::istringstream tokenStream(str);
    while (std::getline(tokenStream, token, delim)) {
        tokens.push_back(trim(token));
    }
    return tokens;
}

std::string to_lower(std::string s) {
    std::transform(s.begin(), s.end(), s.begin(), [](unsigned char c) {
        return static_cast<char>(std::tolower(c));
    });
    return s;
}

}  // namespace

std::string get_default_config_path() {
    const char* xdg = std::getenv("XDG_CONFIG_HOME");
    if (xdg && *xdg) {
        return std::string(xdg) + "/trex-guard/config.ini";
    }
    const char* home = std::getenv("HOME");
    if (home && *home) {
        return std::string(home) + "/.config/trex-guard/config.ini";
    }
    return "";
}

Config parse_config_string(const std::string& content) {
    Config cfg;
    std::istringstream stream(content);
    std::string line;
    std::string current_section;

    while (std::getline(stream, line)) {
        line = trim(line);
        if (line.empty() || line[0] == '#' || line[0] == ';') {
            continue;
        }

        if (line.front() == '[' && line.back() == ']') {
            current_section = to_lower(trim(line.substr(1, line.size() - 2)));
            continue;
        }

        if (current_section == "general") {
            auto pos = line.find('=');
            if (pos != std::string::npos) {
                std::string key = to_lower(trim(line.substr(0, pos)));
                std::string val = to_lower(trim(line.substr(pos + 1)));
                if (key == "auto_approve_safe") {
                    cfg.auto_approve_safe = (val == "true" || val == "1" || val == "yes");
                }
            }
        } else if (current_section == "allowlist") {
            cfg.allowlist.push_back(line);
        } else if (current_section == "disabled_rules") {
            cfg.disabled_rules.insert(line);
        } else if (current_section == "custom_rules") {
            auto parts = split(line, '|');
            if (parts.size() >= 3) {
                CustomRule cr;
                cr.name = parts[0];
                std::string r_str = to_lower(parts[1]);
                if (r_str == "danger" || r_str == "high" || r_str == "high_risk") {
                    cr.risk = Risk::Danger;
                } else {
                    cr.risk = Risk::Review;
                }
                cr.pattern = parts[2];
                if (parts.size() >= 4) {
                    cr.explanation = parts[3];
                }
                cfg.custom_rules.push_back(cr);
            }
        }
    }

    return cfg;
}

Config load_config(const std::string& path) {
    std::string target = path;
    if (target.empty()) {
        target = get_default_config_path();
    }
    if (target.empty()) {
        return Config{};
    }

    std::ifstream file(target);
    if (!file.is_open()) {
        return Config{};
    }

    std::stringstream buffer;
    buffer << file.rdbuf();
    return parse_config_string(buffer.str());
}

}  // namespace trex
