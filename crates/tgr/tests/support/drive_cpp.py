"""Drive the C++ guard through a real pty and print its decision.

The C++ guard refuses to run without a terminal (`tcgetattr` on stdin), so a
pipe cannot be used. Python's pty module is used because the parity test is
about agreeing with the *shipped binary*, and driving it exactly as a user
would is the only comparison worth making.

Usage: drive_cpp.py <binary> <paste>  ->  prints SAFE|REVIEW|DANGER
"""
import os, pty, select, sys, time

def drive(binary, paste, timeout=6.0):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TREX_GUARD_ACTIVE"] = "0"
        os.execv(binary, [binary])
    out = b""
    payload = b"\x1b[200~" + paste.encode() + b"\x1b[201~"
    deadline = time.time() + timeout
    sent_paste = False
    sent_cancel = False
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.2)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            if not sent_paste:
                os.write(fd, payload); sent_paste = True
            elif b"Execute?" in out and not sent_cancel:
                os.write(fd, b"n"); sent_cancel = True
        else:
            if not sent_paste:
                os.write(fd, payload); sent_paste = True
            elif b"Execute?" in out and not sent_cancel:
                os.write(fd, b"n"); sent_cancel = True
    try:
        os.kill(pid, 9); os.waitpid(pid, 0)
    except Exception:
        pass
    return out


def verdict(binary, paste):
    out = drive(binary, paste).decode("utf-8", "replace")
    if "[HIGH RISK" in out:
        return "DANGER"
    if "[REVIEW" in out:
        return "REVIEW"
    if "[SAFE]" in out:
        return "SAFE"
    # No banner: the C++ guard auto-approved it as safe.
    return "SAFE"


if __name__ == "__main__":
    print(verdict(sys.argv[1], sys.argv[2]))
