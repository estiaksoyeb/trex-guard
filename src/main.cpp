#include <pty.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

#include <cerrno>
#include <cstring>
#include <iostream>

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

    char buffer[8192];

    while (true) {
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

            ssize_t sent = 0;
            while (sent < n) {
                ssize_t written = write(
                    master,
                    buffer + sent,
                    static_cast<size_t>(n - sent)
                );

                if (written < 0) {
                    if (errno == EINTR)
                        continue;
                    goto done;
                }

                sent += written;
            }
        }

        if (FD_ISSET(master, &fds)) {
            ssize_t n = read(master, buffer, sizeof(buffer));

            if (n == 0)
                break;

            if (n < 0) {
                if (errno == EINTR)
                    continue;

                break;
            }

            ssize_t sent = 0;
            while (sent < n) {
                ssize_t written = write(
                    STDOUT_FILENO,
                    buffer + sent,
                    static_cast<size_t>(n - sent)
                );

                if (written < 0) {
                    if (errno == EINTR)
                        continue;
                    goto done;
                }

                sent += written;
            }
        }
    }

done:
    close(master);

    int status = 0;
    waitpid(child, &status, 0);

    if (WIFEXITED(status))
        return WEXITSTATUS(status);

    if (WIFSIGNALED(status))
        return 128 + WTERMSIG(status);

    return 1;
}
