"""Drive a guard binary through a real pty.

Both guards refuse to run without a terminal (`tcgetattr` on stdin), so neither
a pipe nor `Command::output` can test them honestly. This helper forks a pty,
runs the binary as a user would, and returns what the terminal showed plus the
exit status.

Usage:
  drive_cpp.py <binary> <paste>   -> SAFE | REVIEW | DANGER
  drive_pty.py <binary> <paste>   -> repr of output + exit status
"""
import os
import pty
import re
import select
import signal
import time


def drive(binary, paste, reply=None, timeout=6.0, extra_args=(), env=None, settle=0.6):
    """Runs `binary`, sends `paste` once as a bracketed paste, then `reply`.

    `reply` is sent only once the review prompt appears (or, for a safe paste
    that was auto-approved, `settle` seconds after the paste). Nothing is ever
    sent twice: re-sending the paste would type it into a shell that already
    came back to its prompt, which looks like a hang and is not.

    Returns (output_bytes, wait_status).
    """
    child_env = dict(os.environ)
    child_env.setdefault("SHELL", "/bin/bash")
    child_env["TGR_ACTIVE"] = "0"
    child_env["TREX_GUARD_ACTIVE"] = "0"
    child_env["PS1"] = "TGRPROMPT> "
    child_env["PROMPT_COMMAND"] = ""
    child_env["TERM"] = "xterm-256color"
    if env:
        child_env.update(env)

    pid, fd = pty.fork()
    if pid == 0:
        os.environ.clear()
        os.environ.update(child_env)
        os.execv(binary, [binary, *extra_args])

    out = b""
    payload = b"\x1b[200~" + paste.encode() + b"\x1b[201~"
    deadline = time.time() + timeout
    paste_sent_at = None
    replied = False

    def send(data):
        if isinstance(data, str):
            data = data.encode()
        try:
            os.write(fd, data)
            return True
        except OSError:
            return False

    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk

        if paste_sent_at is None:
            if not send(payload):
                break
            paste_sent_at = time.time()
            continue

        if replied:
            continue

        if b"Execute?" in out:
            if reply is None:
                break  # leave the guard at its prompt; the caller only reads
            send(reply)
            replied = True
        elif paste_sent_at is not None and time.time() - paste_sent_at > settle and reply is None:
            break  # auto-approved: nothing will be asked

    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass
    _, status = os.waitpid(pid, 0)
    try:
        os.close(fd)
    except OSError:
        pass
    return out, status


def plain(data):
    """Strips ANSI escapes so assertions can look for words, not bytes."""
    text = data.decode("utf-8", "replace")
    text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", text)
    text = re.sub(r"\x1b[()][A-Z0-9]|\x1b[78>=c]", "", text)
    return text
