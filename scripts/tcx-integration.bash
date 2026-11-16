# TREX ConteXt (tcx) shell integration
#
# Source this from ~/.bashrc or ~/.zshrc:
#     . /usr/share/trex-guard/tcx-integration.bash
#
# Why this exists:
#
#   * The three XDG variables are commonly unset (they are unset on the
#     development machine). Without help, tcx falls back to ~/.local/share and
#     friends, which is fine — but Termux and a proot/chroot have DIFFERENT
#     HOMEs, producing two silently separate stores.
#   * Setting TCX_HOME once, to a single absolute path, makes the store unique
#     regardless of which environment you are in.

# Only set TCX_HOME if the user has not. Never override an explicit choice.
if [ -z "${TCX_HOME:-}" ]; then
    if [ -n "${HOME:-}" ]; then
        TCX_HOME="$HOME/.local/share/tcx-home"
        export TCX_HOME
    fi
fi

# Convenience: `prime | clip` in one step, since the briefing is meant for
# pasting into a chat. Only defined when `clip` exists.
if command -v clip >/dev/null 2>&1; then
    tcx-clip() {
        tcx prime "$@" | clip
    }
fi
