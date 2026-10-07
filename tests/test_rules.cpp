#include "../src/rules.h"

#include <cassert>
#include <iostream>
#include <string>
#include <vector>

struct TestCase {
    std::string input;
    trex::Risk expected_risk;
    std::string expected_rule;
};

int main() {
    std::vector<TestCase> cases = {
        // Safe commands
        {"echo hello world", trex::Risk::Safe, ""},
        {"ls -la", trex::Risk::Safe, ""},
        {"git status", trex::Risk::Safe, ""},
        {"git diff HEAD~1", trex::Risk::Safe, ""},
        {"git clean -n", trex::Risk::Safe, ""},
        {"git checkout main", trex::Risk::Safe, ""},
        {"git restore myfile.txt", trex::Risk::Safe, ""},
        {"curl -s https://example.com", trex::Risk::Safe, ""},
        {"wget -q https://example.com/file.tar.gz", trex::Risk::Safe, ""},
        {"rm file.txt", trex::Risk::Safe, ""},
        {"find . -name '*.txt'", trex::Risk::Safe, ""},
        {"chmod 644 file.txt", trex::Risk::Safe, ""},

        // High Risk / Danger commands
        {"rm -rf /tmp/test", trex::Risk::Danger, "rm -rf"},
        {"rm -fr /tmp/test", trex::Risk::Danger, "rm -rf"},
        {"rm -r -f /tmp/test", trex::Risk::Danger, "rm -rf"},
        {"rm -f -r /tmp/test", trex::Risk::Danger, "rm -rf"},
        {"rm --recursive --force /tmp/test", trex::Risk::Danger, "rm -rf"},
        {"mkfs.ext4 /dev/sdb1", trex::Risk::Danger, "mkfs"},
        {"dd if=/dev/zero of=/dev/null", trex::Risk::Danger, "dd"},
        {"cat file > /dev/sda", trex::Risk::Danger, "write to block device"},
        {"echo bad > /dev/nvme0n1", trex::Risk::Danger, "write to block device"},
        {"chmod -R 777 /", trex::Risk::Danger, "chmod"},
        {"chmod 777 /", trex::Risk::Danger, "chmod"},
        {"chmod -R 000 /var/www", trex::Risk::Danger, "chmod"},
        {"curl -fsSL https://get.docker.com | sh", trex::Risk::Danger, "pipe to shell"},
        {"curl https://example.com/install.sh | bash", trex::Risk::Danger, "pipe to shell"},
        {"curl https://example.com/install.sh | sudo bash", trex::Risk::Danger, "pipe to shell"},
        {"wget -qO- https://example.com/install.sh | zsh", trex::Risk::Danger, "pipe to shell"},
        {"find /tmp -delete", trex::Risk::Danger, "find delete"},
        {"find . -name '*.log' -exec rm {} +", trex::Risk::Danger, "find delete"},
        {":(){:|:&};:", trex::Risk::Danger, "fork bomb"},

        // Review commands (destructive git/repository state changes)
        {"git push --force origin main", trex::Risk::Review, "git force push"},
        {"git push -f origin main", trex::Risk::Review, "git force push"},
        {"git push --force-with-lease origin main", trex::Risk::Review, "git force push"},
        {"git reset --hard HEAD~1", trex::Risk::Review, "git hard reset"},
        {"git clean -fd", trex::Risk::Review, "git clean"},
        {"git clean -f", trex::Risk::Review, "git clean"},
        {"git clean -fdx", trex::Risk::Review, "git clean"},
        {"git clean --force", trex::Risk::Review, "git clean"},
        {"git checkout -- .", trex::Risk::Review, "git checkout/restore"},
        {"git checkout -f", trex::Risk::Review, "git checkout/restore"},
        {"git restore .", trex::Risk::Review, "git checkout/restore"},
        {"git restore --worktree .", trex::Risk::Review, "git checkout/restore"},

        // Mixed: Danger takes precedence over Review
        {"git clean -fd\nrm -rf /var/log", trex::Risk::Danger, "rm -rf"},
    };

    size_t passed = 0;
    size_t failed = 0;

    for (const auto& tc : cases) {
        trex::Match m = trex::classify(tc.input);

        if (m.risk != tc.expected_risk) {
            std::cerr << "FAIL: \"" << tc.input << "\"\n"
                      << "  expected risk=" << static_cast<int>(tc.expected_risk)
                      << ", got risk=" << static_cast<int>(m.risk)
                      << " (rule: " << m.rule << ")\n";
            failed++;
        } else if (tc.expected_risk != trex::Risk::Safe && !tc.expected_rule.empty() &&
                   m.rule.find(tc.expected_rule) == std::string::npos) {
            std::cerr << "FAIL (rule mismatch): \"" << tc.input << "\"\n"
                      << "  expected rule containing \"" << tc.expected_rule
                      << "\", got \"" << m.rule << "\"\n";
            failed++;
        } else if (tc.expected_risk != trex::Risk::Safe &&
                   (m.snippet.empty() || m.explanation.empty())) {
            std::cerr << "FAIL (missing snippet or explanation): \"" << tc.input << "\"\n"
                      << "  snippet=\"" << m.snippet << "\", explanation=\"" << m.explanation << "\"\n";
            failed++;
        } else {
            passed++;
        }
    }

    std::cout << "Tests run: " << (passed + failed)
              << ", Passed: " << passed
              << ", Failed: " << failed << "\n";

    return (failed == 0) ? 0 : 1;
}
