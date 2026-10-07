# Shell integration for TREX Guard (Bash / Zsh)
# Source this file in your ~/.bashrc or ~/.zshrc:
#   source /path/to/shell-integration.bash

# Convenient alias
alias tg='trex-guard'

# Optional prompt indicator: returns " [GUARD]" when active
trex_guard_indicator() {
    if [ "$TREX_GUARD_ACTIVE" = "1" ]; then
        printf " \033[1;32m[GUARD]\033[0m"
    fi
}

# Optional auto-launch helper: launches trex-guard once on login if not already guarded
# To enable, uncomment the line below in your profile:
# [ -z "$TREX_GUARD_ACTIVE" ] && [ -t 0 ] && exec trex-guard
