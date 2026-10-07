#pragma once

#include <string>

namespace trex {

enum class Risk {
    Safe,
    Review,
    Danger,
};

struct Match {
    Risk risk = Risk::Safe;
    std::string rule;         // name of the matched rule, empty when Safe
    std::string snippet;      // exact matched command snippet
    std::string explanation;  // explanation of the risk
};

// Safety scanner that checks for known risky command patterns.
Match classify(const std::string& paste);

}  // namespace trex
