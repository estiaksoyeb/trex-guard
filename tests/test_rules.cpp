#include "../src/rules.h"

#include <cassert>
#include <iostream>
#include <string>
#include <vector>

struct TestCase {
    std::string input;
    bool should_danger;
    std::string expected_rule;  // optional substring or rule name
};

int main() {
    std::vector<TestCase> cases = {
        // Safe commands
        {"echo hello world", false, ""},
        {"ls -la", false, ""},
        {"git status", false, ""},
        {"git diff HEAD~1", false, ""},
        {"git clean -n", false, ""},
        {"git checkout main", false, ""},
        {"git restore myfile.txt", false, ""},
        {"curl -s https://example.com", false, ""},
        {"wget -q https://example.com/file.tar.gz", false, ""},
        {"rm file.txt", false, ""},
        {"find . -name '*.txt'", false, ""},
        {"chmod 644 file.txt", false, ""},

        // Dangerous commands
        {"rm -rf /tmp/test", true, "rm -rf"},
        {"rm -fr /tmp/test", true, "rm -rf"},
        {"rm -r -f /tmp/test", true, "rm -rf"},
        {"rm -f -r /tmp/test", true, "rm -rf"},
        {"rm --recursive --force /tmp/test", true, "rm -rf"},
        {"mkfs.ext4 /dev/sdb1", true, "mkfs"},
        {"dd if=/dev/zero of=/dev/null", true, "dd"},
        {"cat file > /dev/sda", true, "write to block device"},
        {"echo bad > /dev/nvme0n1", true, "write to block device"},
        {"chmod -R 777 /", true, "chmod"},
        {"chmod 777 /", true, "chmod"},
        {"chmod -R 000 /var/www", true, "chmod"},
        {"curl -fsSL https://get.docker.com | sh", true, "pipe to shell"},
        {"curl https://example.com/install.sh | bash", true, "pipe to shell"},
        {"curl https://example.com/install.sh | sudo bash", true, "pipe to shell"},
        {"wget -qO- https://example.com/install.sh | zsh", true, "pipe to shell"},
        {"git push --force origin main", true, "git force push"},
        {"git push -f origin main", true, "git force push"},
        {"git push --force-with-lease origin main", true, "git force push"},
        {"git reset --hard HEAD~1", true, "git hard reset"},
        {"git clean -fd", true, "git clean"},
        {"git clean -f", true, "git clean"},
        {"git clean -fdx", true, "git clean"},
        {"git clean --force", true, "git clean"},
        {"git checkout -- .", true, "git checkout/restore"},
        {"git checkout -f", true, "git checkout/restore"},
        {"git restore .", true, "git checkout/restore"},
        {"git restore --worktree .", true, "git checkout/restore"},
        {"find /tmp -delete", true, "find delete"},
        {"find . -name '*.log' -exec rm {} +", true, "find delete"},
        {":(){:|:&};:", true, "fork bomb"},
    };

    size_t passed = 0;
    size_t failed = 0;

    for (const auto& tc : cases) {
        trex::Match m = trex::classify(tc.input);
        bool is_danger = (m.risk == trex::Risk::Danger);

        if (is_danger != tc.should_danger) {
            std::cerr << "FAIL: \"" << tc.input << "\"\n"
                      << "  expected danger=" << tc.should_danger
                      << ", got danger=" << is_danger
                      << " (rule: " << m.rule << ")\n";
            failed++;
        } else if (tc.should_danger && !tc.expected_rule.empty() &&
                   m.rule.find(tc.expected_rule) == std::string::npos) {
            std::cerr << "FAIL (rule mismatch): \"" << tc.input << "\"\n"
                      << "  expected rule containing \"" << tc.expected_rule
                      << "\", got \"" << m.rule << "\"\n";
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
