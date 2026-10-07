#include <pty.h>

#include "rules.h"
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
#include <string>
#include <iostream>

namespace {

volatile sig_atomic_t window_changed = 0;

void handle_sigwinch(int) {
    window_changed = 1;
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

}  // namespace

int main() {
    struct termios original{};
    if (tcgetattr(STDIN_FILENO, &original) == -1) {
        std::cerr << "trex-guard: tcgetattr: "
                  << std::strerror(errno) << '\n';
        return 1;
    }

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

    if (child == 0) {
        const char* shell = std::getenv("SHELL");
        if (!shell || !*shell)
            shell = "/bin/bash";

        execl(shell, shell, "--login", static_cast<char*>(nullptr));

        std::cerr << "trex-guard: exec: "
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

    struct sigaction old_sigwinch{};
    struct sigaction sigwinch{};
    sigwinch.sa_handler = handle_sigwinch;
    sigemptyset(&sigwinch.sa_mask);
    sigwinch.sa_flags = 0;

    if (sigaction(SIGWINCH, &sigwinch, &old_sigwinch) == -1) {
        std::cerr << "trex-guard: sigaction: "
                  << std::strerror(errno) << '\n';
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &original);
        kill(child, SIGHUP);
        close(master);
        waitpid(child, nullptr, 0);
        return 1;
    }

    char buffer[8192];
    int exit_code = 1;
    PasteTap paste_tap;

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

                trex::Match m = trex::classify(r.hold);
                std::string danger;
                if (m.risk == trex::Risk::Danger)
                    danger = "  [DANGER: " + m.rule + "]";

                std::string header =
                    "\r\n--- pasted " + std::to_string(r.hold.size()) +
                    " bytes, " + std::to_string(lines) +
                    " lines ---" + danger + "\r\n" + shown +
                    "\r\n--- press y to approve, any other key to cancel: ";

                if (!write_all(STDOUT_FILENO, header.data(), header.size()))
                    break;

                char c = 0;
                ssize_t got = read(STDIN_FILENO, &c, 1);
                const bool approved =
                    (got == 1 && (c == 'y' || c == 'Y'));

                if (approved) {
                    if (!write_all(master, r.hold.data(), r.hold.size()))
                        break;
                    const char* ok = "\r\n[approved]\r\n";
                    write_all(STDOUT_FILENO, ok, std::strlen(ok));
                } else {
                    const char* no = "\r\n[paste cancelled]\r\n";
                    write_all(STDOUT_FILENO, no, std::strlen(no));
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
    tcsetattr(STDIN_FILENO, TCSAFLUSH, &original);
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
