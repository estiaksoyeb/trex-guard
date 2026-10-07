#pragma once

#include <string>

namespace trex {

enum class Risk {
    Safe,
    Danger,
};

struct Match {
    Risk risk = Risk::Safe;
    std::string rule;   // name of the matched rule, empty when Safe
};

// Literal-string scanner. Intentionally simple: it does NOT parse shell,
// so obfuscated or quoted forms can evade it. Replace with a real parser
// before relying on it for safety decisions.
Match classify(const std::string& paste);

}  // namespace trex
