#include "rules.h"

#include <array>

namespace trex {

namespace {

struct Rule {
    const char* name;
    const char* needle;
};

constexpr std::array<Rule, 10> kRules = {{
    {"rm -rf",          "rm -rf"},
    {"mkfs",            "mkfs"},
    {"dd if=",          "dd if="},
    {"write to block device", "> /dev/sd"},
    {"chmod 777 root",  "chmod -R 777 /"},
    {"curl pipe shell", "curl"},
    {"wget pipe shell", "wget"},
    {"git force push",  "git push --force"},
    {"git hard reset",  "git reset --hard"},
    {"fork bomb",       ":(){:|:&};:"},
}};

}  // namespace

Match classify(const std::string& paste) {
    for (const Rule& r : kRules) {
        if (paste.find(r.needle) != std::string::npos) {
            Match m;
            m.risk = Risk::Danger;
            m.rule = r.name;
            return m;
        }
    }
    return Match{};
}

}  // namespace trex
