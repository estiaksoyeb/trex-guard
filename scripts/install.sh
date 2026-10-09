#!/bin/sh
# trex-guard installer script
# One-liner: curl -fsSL https://raw.githubusercontent.com/estiaksoyeb/trex-guard/master/scripts/install.sh | bash

set -e

REPO="estiaksoyeb/trex-guard"
GITHUB_URL="https://github.com/${REPO}"

# ANSI color codes
BOLD="\033[1m"
GREEN="\033[1;32m"
YELLOW="\033[1;33m"
RED="\033[1;31m"
RESET="\033[0m"

log_info() {
    printf "${GREEN}[+]${RESET} %s\n" "$1"
}

log_warn() {
    printf "${YELLOW}[!]${RESET} %s\n" "$1"
}

log_error() {
    printf "${RED}[x]${RESET} %s\n" "$1" >&2
}

# 1. Detect Architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$ARCH" in
    x86_64|amd64)
        ARCH_TAG="x86_64"
        ;;
    aarch64|arm64)
        ARCH_TAG="aarch64"
        ;;
    armv7*|armhf)
        ARCH_TAG="armv7"
        ;;
    *)
        log_error "Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

log_info "Detected system: ${OS} (${ARCH_TAG})"

# 2. Determine Installation Directory
if [ -n "$PREFIX" ] && [ -d "$PREFIX/bin" ]; then
    # Termux / Android environment
    BIN_DIR="$PREFIX/bin"
    DATA_DIR="$PREFIX/share/trex-guard"
elif [ "$(id -u)" = "0" ] || [ -w "/usr/local/bin" ]; then
    # Root or writable /usr/local/bin
    BIN_DIR="/usr/local/bin"
    DATA_DIR="/usr/local/share/trex-guard"
else
    # User-local directory
    BIN_DIR="$HOME/.local/bin"
    DATA_DIR="$HOME/.local/share/trex-guard"
fi

CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/trex-guard"
mkdir -p "$BIN_DIR" "$DATA_DIR" "$CONFIG_DIR"

# Helper to check download tool
if command -v curl >/dev/null 2>&1; then
    FETCH_CMD="curl -fsSL"
elif command -v wget >/dev/null 2>&1; then
    FETCH_CMD="wget -qO-"
else
    log_error "Neither curl nor wget found. Please install curl or wget."
    exit 1
fi

TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'trex-install')"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

log_info "Fetching latest trex-guard for ${ARCH_TAG}..."
TARBALL_URL="${GITHUB_URL}/releases/latest/download/trex-guard-${ARCH_TAG}.tar.gz"

DOWNLOAD_SUCCESS=0
if command -v curl >/dev/null 2>&1; then
    if curl -fsSL -o "$TMP_DIR/trex-guard.tar.gz" "$TARBALL_URL" 2>/dev/null; then
        DOWNLOAD_SUCCESS=1
    fi
elif command -v wget >/dev/null 2>&1; then
    if wget -q -O "$TMP_DIR/trex-guard.tar.gz" "$TARBALL_URL" 2>/dev/null; then
        DOWNLOAD_SUCCESS=1
    fi
fi

if [ "$DOWNLOAD_SUCCESS" -eq 1 ]; then
    log_info "Extracting precompiled release package..."
    tar -xzf "$TMP_DIR/trex-guard.tar.gz" -C "$TMP_DIR"
    
    install -m 755 "$TMP_DIR/trex-guard" "$BIN_DIR/trex-guard"
    install -m 755 "$TMP_DIR/tg" "$BIN_DIR/tg"
    
    if [ -f "$TMP_DIR/config.ini" ]; then
        install -m 644 "$TMP_DIR/config.ini" "$DATA_DIR/config.ini"
    fi
    if [ -f "$TMP_DIR/shell-integration.bash" ]; then
        install -m 644 "$TMP_DIR/shell-integration.bash" "$DATA_DIR/shell-integration.bash"
    fi
else
    log_warn "Precompiled release asset not found for ${ARCH_TAG} at ${TARBALL_URL}."
    log_info "Attempting fallback build from source repository..."

    if ! command -v cmake >/dev/null 2>&1; then
        log_error "CMake is required to build from source when precompiled binaries are unavailable."
        log_error "Please install cmake and g++ / clang, or download a release from ${GITHUB_URL}/releases"
        exit 1
    fi

    if ! command -v git >/dev/null 2>&1; then
        log_error "Git is required to clone the source repository."
        exit 1
    fi

    git clone --depth 1 "${GITHUB_URL}.git" "$TMP_DIR/source"
    cmake -B "$TMP_DIR/source/build" "$TMP_DIR/source" -DCMAKE_BUILD_TYPE=Release
    cmake --build "$TMP_DIR/source/build"

    install -m 755 "$TMP_DIR/source/build/trex-guard" "$BIN_DIR/trex-guard"
    install -m 755 "$TMP_DIR/source/scripts/tg" "$BIN_DIR/tg"
    install -m 644 "$TMP_DIR/source/examples/config.ini" "$DATA_DIR/config.ini"
    install -m 644 "$TMP_DIR/source/scripts/shell-integration.bash" "$DATA_DIR/shell-integration.bash"
fi

# Initialize user config if not present
if [ ! -f "$CONFIG_DIR/config.ini" ] && [ -f "$DATA_DIR/config.ini" ]; then
    cp "$DATA_DIR/config.ini" "$CONFIG_DIR/config.ini"
    log_info "Created default configuration: $CONFIG_DIR/config.ini"
fi

# Check if BIN_DIR is in PATH
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        log_warn "$BIN_DIR is not currently in your \$PATH."
        echo "Add it by adding this to your ~/.bashrc or ~/.profile:"
        echo "  export PATH=\"$BIN_DIR:\$PATH\""
        ;;
esac

echo ""
printf "${GREEN}${BOLD}✓ TREX Guard installed successfully!${RESET}\n"
echo "  Binaries installed to: $BIN_DIR"
echo "  Configuration file:    $CONFIG_DIR/config.ini"
echo ""
echo "Quick Start:"
echo "  Run 'tg' to launch your guarded terminal session."
echo ""
