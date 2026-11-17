"""Drive a guard binary through a real pty.

The guard refuses to run without a terminal (`tcgetattr` on stdin), so neither
a pipe nor `Command::output` can test it honestly. This helper forks a pty,
runs the binary as a user would, and returns what the terminal showed plus the
exit status.

Usage:
  drive_pty.py <binary> <paste>   -> repr of output + exit status
"""
import os
import pty
import re
import select
import signal
import time


def drive(
    binary, paste, reply=None, timeout=6.0, extra_args=(), env=None, settle=0.6, quiet=0.25
):
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
    # The budget starts when the paste is sent, not when the process starts.
    # Timing it from here meant that under load the shell's own startup could
    # eat the whole `timeout`, leaving a truncated screen: a danger paste with
    # no `HIGH RISK` line reads as "safe", which looks exactly like the two
    # paths disagreeing when the only problem was a slow machine.
    deadline = None
    paste_sent_at = None
    replied = False
    # Length of the screen when the paste went out, so "the guard has said
    # something since" can be told apart from "nothing has happened yet".
    out_at_paste = 0
    # When the screen last moved. Exit is decided by the screen going quiet,
    # not by the wall clock, so the cost of a paste is how long the guard takes
    # to say its piece rather than how long the budget allows it to.
    last_output = time.time()

    def send(data):
        if isinstance(data, str):
            data = data.encode()
        try:
            os.write(fd, data)
            return True
        except OSError:
            return False

    # Wait for the guarded shell to print a prompt before pasting. A paste
    # sent before bash has finished starting is buffered by the terminal and
    # arrives with no bracketed-paste handling, so it is never classified —
    # the guard looks broken when only the timing was wrong.
    # `PS1` is set above to `TGRPROMPT> `, so this is a reliable readiness
    # signal rather than a guess at how long bash takes to start.
    startup = time.time() + min(timeout, 5.0)
    while time.time() < startup:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            if b"TGRPROMPT>" in out:
                break

    while deadline is None or time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            last_output = time.time()

        if paste_sent_at is None:
            if not send(payload):
                break
            paste_sent_at = time.time()
            deadline = paste_sent_at + timeout
            out_at_paste = len(out)
            continue

        if b"Execute?" in out:
            if reply is None:
                break  # leave the guard at its prompt; the caller only reads
            if not replied:
                send(reply)
                replied = True
            # Do not stop here. The review banner can share the burst with the
            # prompt, but it can also arrive just after it, and a screen read
            # before `HIGH RISK` lands reads as "safe". Keep reading until the
            # screen goes quiet, which is a fact about the guard rather than a
            # guess at how fast the machine is.
            quiet_for = time.time() - last_output
            if quiet_for > quiet:
                break
        elif reply is None and paste_sent_at is not None and time.time() - paste_sent_at > settle:
            break  # auto-approved: nothing will be asked
        elif (
            paste_sent_at is not None
            and not replied
            and len(out) > out_at_paste
            and time.time() - last_output > settle
        ):
            # A safe paste is auto-approved, so `Execute?` never appears; the
            # screen simply goes back to a prompt. Two prompts on screen and no
            # output for `settle` means it ran and is waiting for the next
            # paste. Without this the driver sat out the whole `timeout` on
            # every safe paste, which was most of the suite's runtime.
            break

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
