#!/bin/sh
# trex-guard installer
#
# One-liner:
#   curl -fsSL https://raw.githubusercontent.com/estiaksoyeb/trex-guard/master/scripts/install.sh | sh
#
# Installs the three Rust tools — `tgr`, `tgw`, `tcx` — plus the `tg` launcher,
# from a published GitHub release. There is no build fallback: if a release is
# missing or does not verify, this stops rather than compiling something the
# CI never saw.
#
# Options (pass after `sh -s --`):
#   --version TAG    install a specific tag (default: latest release)
#   --prefix DIR     install directory (default: detected, see below)
#   --no-verify      skip checksum verification (NOT recommended)

set -eu

REPO="estiaksoyeb/trex-guard"
GITHUB_URL="https://github.com/${REPO}"

VERSION=""
PREFIX_DIR=""
VERIFY=1

BOLD="\033[1m"
GREEN="\033[1;32m"
YELLOW="\033[1;33m"
RED="\033[1;31m"
RESET="\033[0m"

log_info()  { printf "${GREEN}[+]${RESET} %s\n" "$1"; }
log_warn()  { printf "${YELLOW}[!]${RESET} %s\n" "$1"; }
log_error() { printf "${RED}[x]${RESET} %s\n" "$1" >&2; }

die() {
    log_error "$1"
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || die "--version needs a tag"
            VERSION="$2"
            shift 2
            ;;
        --prefix)
            [ $# -ge 2 ] || die "--prefix needs a directory"
            PREFIX_DIR="$2"
            shift 2
            ;;
        --no-verify)
            VERIFY=0
            shift
            ;;
        -h|--help)
            sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

# 1. Platform. Only Linux is released; saying so is better than downloading a
#    binary that cannot run.
OS="$(uname -s)"
[ "$OS" = "Linux" ] || die "unsupported OS: $OS (releases are Linux only)"

case "$(uname -m)" in
    x86_64|amd64)  ARCH_TAG="linux-x86_64" ;;
    aarch64|arm64) ARCH_TAG="linux-aarch64" ;;
    *) die "unsupported architecture: $(uname -m) (released: x86_64, aarch64)" ;;
esac

# 2. Install directory. Termux's $PREFIX wins, then a writable /usr/local, then
#    a user-local directory. Never require root to install a user tool.
if [ -n "$PREFIX_DIR" ]; then
    BIN_DIR="$PREFIX_DIR"
elif [ -n "${PREFIX:-}" ] && [ -d "${PREFIX}/bin" ]; then
    BIN_DIR="${PREFIX}/bin"
elif [ -w /usr/local/bin ] || [ "$(id -u)" = "0" ]; then
    BIN_DIR="/usr/local/bin"
else
    BIN_DIR="${HOME}/.local/bin"
fi
mkdir -p "$BIN_DIR"

# 3. Download. curl or wget; no fallback to a source build.
FETCH=""
command -v curl >/dev/null 2>&1 && FETCH="curl"
[ -z "$FETCH" ] && command -v wget >/dev/null 2>&1 && FETCH="wget"
[ -n "$FETCH" ] || die "need curl or wget"

fetch() { # fetch <url> <dest>
    case "$FETCH" in
        curl) curl -fsSL -o "$2" "$1" ;;
        wget) wget -q -O "$2" "$1" ;;
    esac
}

if [ -n "$VERSION" ]; then
    BASE="${GITHUB_URL}/releases/download/${VERSION}"
else
    BASE="${GITHUB_URL}/releases/latest/download"
fi

TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t trex-install)"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT INT TERM

TARBALL="trex-guard-${ARCH_TAG}.tar.gz"
log_info "Installing trex-guard (${ARCH_TAG}${VERSION:+ $VERSION})"

fetch "${BASE}/${TARBALL}" "${TMP_DIR}/${TARBALL}" \
    || die "could not download ${BASE}/${TARBALL}"

# 4. Integrity. A truncated download that untars partially is how half an
#    install happens, so the checksum is checked unless explicitly waived.
if [ "$VERIFY" -eq 1 ]; then
    if fetch "${BASE}/SHA256SUMS" "${TMP_DIR}/SHA256SUMS" 2>/dev/null; then
        expected="$(grep " ${TARBALL}\$" "${TMP_DIR}/SHA256SUMS" | awk '{print $1}')"
        if [ -z "$expected" ]; then
            die "SHA256SUMS has no entry for ${TARBALL}"
        fi
        actual="$(sha256sum "${TMP_DIR}/${TARBALL}" | awk '{print $1}')"
        [ "$expected" = "$actual" ] \
            || die "checksum mismatch for ${TARBALL}: expected ${expected}, got ${actual}"
        log_info "Checksum verified"
    else
        die "could not download SHA256SUMS; refusing to install unverified (use --no-verify to override)"
    fi
fi

tar -xzf "${TMP_DIR}/${TARBALL}" -C "$TMP_DIR" \
    || die "could not extract ${TARBALL}"
STAGE="${TMP_DIR}/trex-guard-${ARCH_TAG}"
[ -d "$STAGE" ] || die "archive did not contain ${STAGE##*/}"

# 5. Install. Every tool is required: a partial install where `tg` exists but
#    `tgr` does not is worse than a failed install.
for tool in tgr tgw tcx tg; do
    [ -f "${STAGE}/${tool}" ] || die "release is missing ${tool}"
done

for tool in tgr tgw tcx tg; do
    install -m 755 "${STAGE}/${tool}" "${BIN_DIR}/${tool}"
    log_info "installed ${BIN_DIR}/${tool}"
done

# The C++ guard was removed. An old copy left on PATH would be harmless to
# `tg` now (it looks only for `tgr`) but would still sit there confusing anyone
# who runs it, so it is removed by name. Only the exact file is removed.
if [ -e "${BIN_DIR}/trex-guard" ]; then
    rm -f "${BIN_DIR}/trex-guard"
    log_info "removed the old C++ guard at ${BIN_DIR}/trex-guard"
fi

# The guard config is a default only: never overwrite one the user has edited.
GUARD_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/trex-guard"
mkdir -p "$GUARD_CONFIG_DIR"
if [ ! -f "${GUARD_CONFIG_DIR}/config.ini" ] && [ -f "${STAGE}/config.ini" ]; then
    install -m 644 "${STAGE}/config.ini" "${GUARD_CONFIG_DIR}/config.ini"
    log_info "created ${GUARD_CONFIG_DIR}/config.ini (default rules)"
else
    log_info "kept existing ${GUARD_CONFIG_DIR}/config.ini"
fi

# The workflow prompt is what `tg fresh` prints. Without it `fresh` fails with
# "no prompt template found", so a fresh install would ship a tool whose main
# command does not work. Same rule as the guard config: installed only when
# absent, never overwritten, because the rules are yours to edit.
#
# Location follows tcx's own lookup: $TCX_HOME/config when TCX_HOME is set,
# else $XDG_CONFIG_HOME/tcx (then ~/.config/tcx). It is not tied to BIN_DIR.
if [ -n "${TCX_HOME:-}" ]; then
    TCX_CONFIG_DIR="${TCX_HOME}/config"
else
    TCX_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/tcx"
fi
mkdir -p "$TCX_CONFIG_DIR"
if [ ! -f "${TCX_CONFIG_DIR}/prompt.md" ] && [ -f "${STAGE}/prompt.md" ]; then
    install -m 644 "${STAGE}/prompt.md" "${TCX_CONFIG_DIR}/prompt.md"
    log_info "created ${TCX_CONFIG_DIR}/prompt.md (workflow rules)"
elif [ -f "${TCX_CONFIG_DIR}/prompt.md" ]; then
    log_info "kept existing ${TCX_CONFIG_DIR}/prompt.md"
else
    log_warn "release has no prompt.md; run `tg fresh` will not work until you write ${TCX_CONFIG_DIR}/prompt.md"
fi

# 6. Verify what was installed actually runs. A binary that cannot execute on
#    this host (wrong libc, wrong arch) should fail here, not at first paste.
for tool in tgr tgw tcx; do
    "${BIN_DIR}/${tool}" --version >/dev/null 2>&1 \
        || die "${tool} was installed but does not run on this system"
done
log_info "all tools run"

case ":$PATH:" in
    *":${BIN_DIR}:"*) ;;
    *)
        log_warn "${BIN_DIR} is not in \$PATH"
        echo "  Add this to ~/.bashrc or ~/.profile:"
        echo "    export PATH=\"${BIN_DIR}:\$PATH\""
        ;;
esac

echo ""
printf "${GREEN}${BOLD}trex-guard installed${RESET}\n"
echo "  Tools:      tgr, tgw, tcx"
echo "  Launcher:   tg"
echo "  Directory:  ${BIN_DIR}"
echo "  Rules:      ${GUARD_CONFIG_DIR}/config.ini"
echo "  Prompt:     ${TCX_CONFIG_DIR}/prompt.md"
echo ""
echo "Quick start:"
echo "  tg          launch a guarded shell"
echo "  tg fresh | clip   start a chat with the workflow prompt"
echo "  tgr --help  the guard on its own"
echo "  tcx --help  the context store"
echo "  tgw --help  the clipboard watchdog"
