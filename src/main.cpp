#include <pty.h>

#include "rules.h"
#include "config.h"
#include <signal.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>
#include <iostream>

namespace {

volatile sig_atomic_t window_changed = 0;
volatile sig_atomic_t g_terminal_modified = 0;
struct termios g_original{};
pid_t g_child = -1;

bool ancestor_has_norc() {
    pid_t cur = getppid();
    for (int depth = 0; depth < 6 && cur > 1; ++depth) {
        std::string cmd_path = "/proc/" + std::to_string(cur) + "/cmdline";
        std::ifstream f(cmd_path, std::ios::binary);
        if (f.is_open()) {
            std::string arg;
            while (std::getline(f, arg, '\0')) {
                if (arg == "--norc")
                    return true;
            }
        }
        std::string stat_path = "/proc/" + std::to_string(cur) + "/stat";
        std::ifstream sf(stat_path);
        if (!sf.is_open())
            break;
        std::string stat_line;
        if (!std::getline(sf, stat_line))
            break;
        auto rparen = stat_line.rfind(')');
        if (rparen == std::string::npos || rparen + 4 >= stat_line.size())
            break;
        std::istringstream ss(stat_line.substr(rparen + 2));
        char state;
        pid_t ppid = 0;
        if (ss >> state >> ppid && ppid > 1) {
            cur = ppid;
        } else {
            break;
        }
    }
    return false;
}

void handle_sigwinch(int) {
    window_changed = 1;
}

void restore_terminal() {
    if (g_terminal_modified) {
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &g_original);
        g_terminal_modified = 0;
    }
}

void handle_cleanup_signal(int sig) {
    restore_terminal();
    if (g_child > 0) {
        kill(g_child, sig);
    }
    struct sigaction sa{};
    sa.sa_handler = SIG_DFL;
    sigemptyset(&sa.sa_mask);
    sigaction(sig, &sa, nullptr);
    raise(sig);
}

enum class Decision {
    Approve,
    Cancel,
};

Decision read_approval_decision(int master, struct winsize* window) {
    char buf[64];
    while (true) {
        if (window_changed) {
            window_changed = 0;
            if (ioctl(STDIN_FILENO, TIOCGWINSZ, window) == 0)
                ioctl(master, TIOCSWINSZ, window);
        }

        ssize_t n = read(STDIN_FILENO, buf, sizeof(buf));
        if (n < 0) {
            if (errno == EINTR) {
                // Interrupted by window resize (SIGWINCH) or signal: continue waiting
                continue;
            }
            return Decision::Cancel;
        }
        if (n == 0) {
            return Decision::Cancel;
        }

        // Terminal focus reporting: FocusIn (\x1b[I) and FocusOut (\x1b[O).
        // Ignore focus events so unfocusing/focusing the on-screen keyboard doesn't cancel.
        if (n >= 3 && buf[0] == '\x1b' && buf[1] == '[' && (buf[2] == 'I' || buf[2] == 'O')) {
            continue;
        }

        // Standalone Escape cancels; escape sequences (arrows, touch gestures) are ignored.
        if (buf[0] == '\x1b') {
            if (n == 1) {
                return Decision::Cancel;
            }
            continue;
        }

        for (ssize_t i = 0; i < n; ++i) {
            char c = buf[i];
            if (c == '\r' || c == '\n' || c == 'y' || c == 'Y') {
                return Decision::Approve;
            }
            if (c == '\x03' || c == 'n' || c == 'N') {
                return Decision::Cancel;
            }
        }
    }
}

// Result of feeding one chunk of stdin to the paste detector.
struct PasteFeedResult {
    std::string forward;   // bytes to send straight through to the PTY
    std::string hold;      // bytes captured as part of a paste
    bool paste_complete = false;  // a full paste is now in `hold`
};

struct PasteTap {
    bool in_paste = false;
    size_t match = 0;
    std::string captured;

    // Classifies one chunk of stdin. Returns the bytes to forward immediately
    // (typed input) and, when a paste ends, the captured paste text.
    PasteFeedResult feed(const char* data, size_t size) {
        PasteFeedResult r;
        for (size_t i = 0; i < size; ++i) {
            const char c = data[i];
            const char* marker = in_paste ? "\x1b[201~" : "\x1b[200~";
            if (c == marker[match]) {
                if (++match < 6)
                    continue;
                match = 0;
                if (in_paste) {
                    in_paste = false;
                    r.hold = captured;
                    r.paste_complete = true;
                    captured.clear();
                } else {
                    in_paste = true;
                    captured.clear();
                }
                continue;
            }
            if (match > 0) {
                if (in_paste)
                    captured.append(marker, match);
                else
                    r.forward.append(marker, match);
                match = 0;
                if (c == marker[0]) {
                    match = 1;
                    continue;
                }
            }
            if (in_paste)
                captured.push_back(c);
            else
                r.forward.push_back(c);
        }
        return r;
    }
};

bool write_all(int fd, const char* data, size_t size) {
    size_t sent = 0;

    while (sent < size) {
        ssize_t written = write(fd, data + sent, size - sent);

        if (written > 0) {
            sent += static_cast<size_t>(written);
            continue;
        }

        if (written == -1 && errno == EINTR)
            continue;

        return false;
    }

    return true;
}

// Forwards approved paste payload wrapped in bracketed-paste markers so line
// editors (ble.sh, readline) can use fast batch insertion without character decode lag.
bool forward_paste_payload(int master, const std::string& data, bool submit) {
    const char bp_begin[] = "\x1b[200~";
    const char bp_end[] = "\x1b[201~";
    if (!write_all(master, bp_begin, sizeof(bp_begin) - 1))
        return false;
    if (!write_all(master, data.data(), data.size()))
        return false;
    if (!write_all(master, bp_end, sizeof(bp_end) - 1))
        return false;
    if (submit) {
        const char newline = '\n';
        if (!write_all(master, &newline, 1))
            return false;
    }
    return true;
}

}  // namespace

int main(int argc, char* argv[]) {
    if (argc > 1) {
        std::string arg = argv[1];
        if (arg == "--help" || arg == "-h") {
            std::cout << "TREX Guard - Terminal paste safety gate\n\n"
                      << "Usage:\n"
                      << "  trex-guard            Launch guarded shell\n"
                      << "  trex-guard --help     Show this help message\n"
                      << "  trex-guard --version  Show version information\n";
            return 0;
        }
        if (arg == "--version" || arg == "-v") {
            std::cout << "trex-guard version 0.1.0\n";
            return 0;
        }
    }

    struct termios original{};
    if (tcgetattr(STDIN_FILENO, &original) == -1) {
        std::cerr << "trex-guard: tcgetattr: "
                  << std::strerror(errno) << '\n';
        return 1;
    }
    g_original = original;
    std::atexit(restore_terminal);

    struct winsize window{};
    if (ioctl(STDIN_FILENO, TIOCGWINSZ, &window) == -1) {
        std::cerr << "trex-guard: TIOCGWINSZ: "
                  << std::strerror(errno) << '\n';
        return 1;
    }

    int master = -1;
    pid_t child = forkpty(&master, nullptr, &original, &window);

    if (child == -1) {
        std::cerr << "trex-guard: forkpty: "
                  << std::strerror(errno) << '\n';
        return 1;
    }
    g_child = child;

    if (child == 0) {
        setenv("TREX_GUARD_ACTIVE", "1", 1);

        const char* default_shell = std::getenv("SHELL");
        if (!default_shell || !*default_shell)
            default_shell = "/bin/bash";

        std::vector<const char*> args;
        if (argc > 1) {
            if (argv[1][0] == '-') {
                args.push_back(default_shell);
                for (int i = 1; i < argc; ++i) {
                    if (std::strcmp(argv[i], "--force") != 0) {
                        args.push_back(argv[i]);
                    }
                }
            } else {
                for (int i = 1; i < argc; ++i) {
                    if (std::strcmp(argv[i], "--force") != 0) {
                        args.push_back(argv[i]);
                    }
                }
            }
        } else {
            args.push_back(default_shell);
            if (ancestor_has_norc()) {
                args.push_back("--norc");
            }
        }
        args.push_back(nullptr);

        execvp(args[0], const_cast<char* const*>(args.data()));

        std::cerr << "trex-guard: execvp: "
                  << std::strerror(errno) << '\n';
        _exit(127);
    }

    struct termios raw = original;
    cfmakeraw(&raw);

    if (tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw) == -1) {
        std::cerr << "trex-guard: tcsetattr: "
                  << std::strerror(errno) << '\n';
        kill(child, SIGHUP);
        close(master);
        waitpid(child, nullptr, 0);
        return 1;
    }
    g_terminal_modified = 1;

    struct sigaction sa_clean{};
    sa_clean.sa_handler = handle_cleanup_signal;
    sigemptyset(&sa_clean.sa_mask);
    sa_clean.sa_flags = 0;
    sigaction(SIGTERM, &sa_clean, nullptr);
    sigaction(SIGHUP, &sa_clean, nullptr);
    sigaction(SIGINT, &sa_clean, nullptr);
    sigaction(SIGQUIT, &sa_clean, nullptr);

    struct sigaction old_sigwinch{};
    struct sigaction sigwinch{};
    sigwinch.sa_handler = handle_sigwinch;
    sigemptyset(&sigwinch.sa_mask);
    sigwinch.sa_flags = 0;

    if (sigaction(SIGWINCH, &sigwinch, &old_sigwinch) == -1) {
        std::cerr << "trex-guard: sigaction: "
                  << std::strerror(errno) << '\n';
        restore_terminal();
        kill(child, SIGHUP);
        close(master);
        waitpid(child, nullptr, 0);
        return 1;
    }

    char buffer[8192];
    int exit_code = 1;
    PasteTap paste_tap;
    trex::Config config = trex::load_config();

    while (true) {
        if (window_changed) {
            window_changed = 0;

            if (ioctl(STDIN_FILENO, TIOCGWINSZ, &window) == 0)
                ioctl(master, TIOCSWINSZ, &window);
        }

        fd_set fds;
        FD_ZERO(&fds);
        FD_SET(STDIN_FILENO, &fds);
        FD_SET(master, &fds);

        const int maxfd =
            (STDIN_FILENO > master ? STDIN_FILENO : master) + 1;

        int ready = select(maxfd, &fds, nullptr, nullptr, nullptr);

        if (ready == -1) {
            if (errno == EINTR)
                continue;

            std::cerr << "trex-guard: select: "
                      << std::strerror(errno) << '\n';
            break;
        }

        if (FD_ISSET(STDIN_FILENO, &fds)) {
            ssize_t n = read(STDIN_FILENO, buffer, sizeof(buffer));

            if (n == 0)
                break;

            if (n < 0) {
                if (errno == EINTR)
                    continue;

                break;
            }

            PasteFeedResult r = paste_tap.feed(buffer, static_cast<size_t>(n));

            if (!r.forward.empty() &&
                !write_all(master, r.forward.data(), r.forward.size()))
                break;

            if (r.paste_complete) {
                std::string shown;
                size_t lines = 0;
                for (char ch : r.hold) {
                    if (ch == '\r') {
                        shown += "\r\n";
                        ++lines;
                    } else if (ch == '\n') {
                        shown += "\r\n";
                        ++lines;
                    } else {
                        shown += ch;
                    }
                }
                if (!shown.empty() &&
                    shown[shown.size() - 1] != '\n' &&
                    shown[shown.size() - 1] != '\r')
                    ++lines;

                trex::Match m = trex::classify(r.hold, &config);

                if (m.risk == trex::Risk::Safe && config.auto_approve_safe) {
                    bool has_trailing_newline =
                        !r.hold.empty() && (r.hold.back() == '\n' || r.hold.back() == '\r');
                    if (!forward_paste_payload(master, r.hold, has_trailing_newline))
                        break;
                    continue;
                }

                std::string review_section;
                if (m.risk == trex::Risk::Danger) {
                    if (m.matches.size() > 1) {
                        review_section = "\x1b[1;31m[HIGH RISK]\x1b[0m (" +
                                         std::to_string(m.matches.size()) +
                                         " flagged commands found):\r\n";
                        for (size_t i = 0; i < m.matches.size(); ++i) {
                            const auto& item = m.matches[i];
                            const char* tag_color = (item.risk == trex::Risk::Danger) ? "\x1b[1;31m" : "\x1b[1;33m";
                            review_section += "  " + std::to_string(i + 1) + ". " +
                                              tag_color + "[" + item.rule + "]\x1b[0m " + item.snippet + "\r\n";
                            if (config.show_risk_explanation && !item.explanation.empty()) {
                                review_section += "     \x1b[2mRisk:\x1b[0m " + item.explanation + "\r\n";
                            }
                        }
                    } else {
                        review_section = "\x1b[1;31m[HIGH RISK: " + m.rule + "]\x1b[0m\r\n" +
                                         "  \x1b[31mCommand:\x1b[0m " + m.snippet + "\r\n";
                        if (config.show_risk_explanation && !m.explanation.empty()) {
                            review_section += "  \x1b[31mRisk:\x1b[0m    " + m.explanation + "\r\n";
                        }
                    }
                } else if (m.risk == trex::Risk::Review) {
                    if (m.matches.size() > 1) {
                        review_section = "\x1b[1;33m[REVIEW]\x1b[0m (" +
                                         std::to_string(m.matches.size()) +
                                         " flagged commands found):\r\n";
                        for (size_t i = 0; i < m.matches.size(); ++i) {
                            const auto& item = m.matches[i];
                            review_section += "  " + std::to_string(i + 1) + ". \x1b[1;33m[" +
                                              item.rule + "]\x1b[0m " + item.snippet + "\r\n";
                            if (config.show_risk_explanation && !item.explanation.empty()) {
                                review_section += "     \x1b[2mRisk:\x1b[0m " + item.explanation + "\r\n";
                            }
                        }
                    } else {
                        review_section = "\x1b[1;33m[REVIEW: " + m.rule + "]\x1b[0m\r\n" +
                                         "  \x1b[33mCommand:\x1b[0m " + m.snippet + "\r\n";
                        if (config.show_risk_explanation && !m.explanation.empty()) {
                            review_section += "  \x1b[33mRisk:\x1b[0m    " + m.explanation + "\r\n";
                        }
                    }
                } else {
                    review_section = "\x1b[1;32m[SAFE]\x1b[0m\r\n";
                }

                std::string header =
                    "\r\n\x1b[2m--- [Pasted Content] (" +
                    std::to_string(r.hold.size()) + "B, " +
                    std::to_string(lines) + "L) ---\x1b[0m\r\n" +
                    shown +
                    "\r\n\x1b[2m--- [Security Review] ---\x1b[0m\r\n" +
                    review_section +
                    "\x1b[1mExecute? [Enter/^C] \x1b[0m";

                if (!write_all(STDOUT_FILENO, header.data(), header.size()))
                    break;

                Decision decision = read_approval_decision(master, &window);

                if (decision == Decision::Approve) {
                    if (!forward_paste_payload(master, r.hold, true))
                        break;
                    const char* ok = "\r\n\x1b[1;32m[approved]\x1b[0m\r\n";
                    write_all(STDOUT_FILENO, ok, std::strlen(ok));
                } else {
                    const char* no = "\r\n\x1b[1;31m[cancelled]\x1b[0m\r\n";
                    write_all(STDOUT_FILENO, no, std::strlen(no));
                    // Send interrupt (Ctrl-C) to child master PTY so line editors
                    // (ble.sh, readline) reset input buffer and immediately repaint prompt
                    const char interrupt = '\x03';
                    write_all(master, &interrupt, 1);
                }
            }
        }

        if (FD_ISSET(master, &fds)) {
            ssize_t n = read(master, buffer, sizeof(buffer));

            if (n == 0)
                break;

            if (n < 0) {
                if (errno == EINTR)
                    continue;

                if (errno == EIO)
                    break;

                break;
            }

            if (!write_all(
                    STDOUT_FILENO,
                    buffer,
                    static_cast<size_t>(n)))
                break;
        }
    }

    sigaction(SIGWINCH, &old_sigwinch, nullptr);
    restore_terminal();
    close(master);

    int status = 0;
    if (waitpid(child, &status, 0) == -1)
        return 1;

    if (WIFEXITED(status))
        exit_code = WEXITSTATUS(status);
    else if (WIFSIGNALED(status))
        exit_code = 128 + WTERMSIG(status);

    return exit_code;
}
