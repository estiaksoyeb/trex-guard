#!/bin/sh
# Tests for scripts/install.sh and scripts/tg.
#
# The installer's job is to install what CI built and refuse everything else.
# These tests run it against a fake release served from the local filesystem,
# so they need no network and no real GitHub release.
#
# Run: ./tests/install_test.sh

set -eu

cd "$(dirname "$0")/.."
REPO_ROOT="$(pwd)"
WORK="$(mktemp -d 2>/dev/null || mktemp -d -t trex-install-test)"
trap 'rm -rf "$WORK"' EXIT INT TERM

pass=0
fail=0

ok() { pass=$((pass + 1)); printf 'ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf 'FAIL %s\n     %s\n' "$1" "${2:-}"; }

# --- a fake release -------------------------------------------------------
#
# The tarball holds stub executables: the installer's real checks are that it
# finds the right asset, verifies it, installs every tool, and refuses a bad
# checksum. What the binaries do is covered by the Rust tests.
make_release() { # make_release <dir> <arch-tag>
    dir="$1"
    arch="$2"
    stage="$dir/trex-guard-${arch}"
    mkdir -p "$stage"
    for tool in tgr tgw tcx; do
        cat > "$stage/$tool" <<'STUB'
#!/bin/sh
case "${1:-}" in
    --version) echo "$(basename "$0") stub 0.0.0" ;;
    *) exit 0 ;;
esac
STUB
        chmod 755 "$stage/$tool"
    done
    cp "$REPO_ROOT/scripts/tg" "$stage/tg"
    cp "$REPO_ROOT/examples/config.ini" "$stage/config.ini"
    tar -czf "$dir/trex-guard-${arch}.tar.gz" -C "$dir" "trex-guard-${arch}"
    ( cd "$dir" && sha256sum "trex-guard-${arch}.tar.gz" > SHA256SUMS )
}

# The installer fetches from GitHub URLs. To exercise it offline the fetch is
# pointed at a directory by shadowing curl/wget on PATH with a shim that maps
# the release URL onto the local fake.
make_shims() { # make_shims <bin-dir> <assets-dir>
    mkdir -p "$1"
    cat > "$1/curl" <<SHIM
#!/bin/sh
# -fsSL -o <dest> <url>  ->  copy <assets>/<basename url>
out=""
url=""
while [ \$# -gt 0 ]; do
    case "\$1" in
        -o) out="\$2"; shift 2 ;;
        -*) shift ;;
        *) url="\$1"; shift ;;
    esac
done
name="\$(basename "\$url")"
if [ -f "$2/\$name" ]; then
    cp "$2/\$name" "\$out"
    exit 0
fi
echo "404 \$url" >&2
exit 22
SHIM
    chmod 755 "$1/curl"
}

ARCH_TAG="linux-$(uname -m | sed 's/x86_64/x86_64/; s/aarch64/aarch64/; s/arm64/aarch64/')"

# --- tests ---------------------------------------------------------------

test_installs_all_four() {
    assets="$WORK/assets-ok"
    mkdir -p "$assets"
    make_release "$assets" "$ARCH_TAG"
    make_shims "$WORK/shim-ok" "$assets"

    prefix="$WORK/prefix-ok"
    # HOME *and* XDG_CONFIG_HOME are set: the installer prefers XDG, which is
    # set on a CI runner, so setting only HOME made this pass locally and fail
    # in CI. Both point into the sandbox so nothing touches the real user.
    out=$(PATH="$WORK/shim-ok:$PATH" HOME="$WORK/home-ok" \
        XDG_CONFIG_HOME="$WORK/home-ok/.config" \
        sh "$REPO_ROOT/scripts/install.sh" \
        --prefix "$prefix" 2>&1) || {
        bad "installs all four tools" "installer failed:
$out"
        return
    }
    missing=""
    for tool in tgr tgw tcx tg; do
        [ -x "$prefix/$tool" ] || missing="$missing $tool"
    done
    if [ -z "$missing" ]; then
        ok "installs all four tools"
    else
        bad "installs all four tools" "missing:$missing"
    fi

    if [ -f "$WORK/home-ok/.config/trex-guard/config.ini" ]; then
        ok "writes a default config when none exists"
    else
        bad "writes a default config when none exists" "no config.ini under HOME"
    fi
}

test_rejects_bad_checksum() {
    assets="$WORK/assets-bad"
    mkdir -p "$assets"
    make_release "$assets" "$ARCH_TAG"
    # Corrupt the archive but leave SHA256SUMS describing the original.
    printf 'x' >> "$assets/trex-guard-${ARCH_TAG}.tar.gz"
    make_shims "$WORK/shim-bad" "$assets"

    prefix="$WORK/prefix-bad"
    if PATH="$WORK/shim-bad:$PATH" sh "$REPO_ROOT/scripts/install.sh" \
        --prefix "$prefix" >"$WORK/bad.log" 2>&1; then
        bad "rejects a bad checksum" "installer succeeded on a corrupted archive"
    elif grep -q "checksum mismatch" "$WORK/bad.log"; then
        ok "rejects a bad checksum"
    else
        bad "rejects a bad checksum" "failed for the wrong reason:
$(cat "$WORK/bad.log")"
    fi
    if [ -e "$prefix/tgr" ]; then
        bad "bad checksum installs nothing" "tgr exists after a failed verify"
    else
        ok "bad checksum installs nothing"
    fi
}

test_missing_asset_fails() {
    assets="$WORK/assets-empty"
    mkdir -p "$assets"
    make_shims "$WORK/shim-empty" "$assets"

    prefix="$WORK/prefix-empty"
    if PATH="$WORK/shim-empty:$PATH" sh "$REPO_ROOT/scripts/install.sh" \
        --prefix "$prefix" >"$WORK/empty.log" 2>&1; then
        bad "missing release asset fails" "installer succeeded with no asset"
    else
        ok "missing release asset fails"
    fi
}

test_rejects_unsupported_flag() {
    if sh "$REPO_ROOT/scripts/install.sh" --nonsense >/dev/null 2>&1; then
        bad "rejects an unknown option" "accepted --nonsense"
    else
        ok "rejects an unknown option"
    fi
}

test_does_not_overwrite_config() {
    assets="$WORK/assets-cfg"
    mkdir -p "$assets"
    make_release "$assets" "$ARCH_TAG"
    make_shims "$WORK/shim-cfg" "$assets"

    home="$WORK/home-cfg"
    mkdir -p "$home/.config/trex-guard"
    echo "mode = auto" > "$home/.config/trex-guard/config.ini"

    if ! PATH="$WORK/shim-cfg:$PATH" HOME="$home" XDG_CONFIG_HOME="$home/.config" \
        sh "$REPO_ROOT/scripts/install.sh" \
        --prefix "$WORK/prefix-cfg" >"$WORK/cfg.log" 2>&1; then
        bad "keeps an existing config" "installer failed:
$(cat "$WORK/cfg.log")"
        return
    fi
    if [ "$(cat "$home/.config/trex-guard/config.ini")" = "mode = auto" ]; then
        ok "keeps an existing config"
    else
        bad "keeps an existing config" "user config was overwritten"
    fi
}

# `tg` must work with no C++ guard present, which is what a release installs.
test_tg_runs_tgr() {
    dir="$WORK/tg-only"
    mkdir -p "$dir"
    cp "$REPO_ROOT/scripts/tg" "$dir/tg"
    cat > "$dir/tgr" <<'STUB'
#!/bin/sh
echo "TGR_CALLED $*"
STUB
    chmod 755 "$dir/tgr" "$dir/tg"

    # `--version` is answered by the launcher itself, so it proves nothing
    # about which guard is reached. A bare launch is what must land on tgr.
    out=$(PATH="$dir:/usr/bin:/bin" TREX_GUARD_ACTIVE=0 TGR_ACTIVE=0 "$dir/tg" 2>&1)
    case "$out" in
        "TGR_CALLED"*)
            ok "tg runs tgr"
            ;;
        *)
            bad "tg runs tgr" "unexpected output: $out"
            ;;
    esac
}

# The C++ guard is gone. An old binary left behind would be confusing even
# though `tg` no longer looks for it, so the installer removes it by name.
test_removes_old_cpp_guard() {
    assets="$WORK/assets-oldcpp"
    mkdir -p "$assets"
    make_release "$assets" "$ARCH_TAG"
    make_shims "$WORK/shim-oldcpp" "$assets"

    prefix="$WORK/prefix-oldcpp"
    mkdir -p "$prefix"
    printf '#!/bin/sh\necho old cpp guard\n' > "$prefix/trex-guard"
    chmod 755 "$prefix/trex-guard"

    if ! PATH="$WORK/shim-oldcpp:$PATH" HOME="$WORK/home-oldcpp" \
        XDG_CONFIG_HOME="$WORK/home-oldcpp/.config" \
        sh "$REPO_ROOT/scripts/install.sh" --prefix "$prefix" >"$WORK/oldcpp.log" 2>&1; then
        bad "removes an old C++ guard" "installer failed:
$(cat "$WORK/oldcpp.log")"
        return
    fi
    if [ -e "$prefix/trex-guard" ]; then
        bad "removes an old C++ guard" "trex-guard still present after install"
    else
        ok "removes an old C++ guard"
    fi
}

test_tg_refuses_nested_session() {
    dir="$WORK/tg-nest"
    mkdir -p "$dir"
    cp "$REPO_ROOT/scripts/tg" "$dir/tg"
    cat > "$dir/tgr" <<'STUB'
#!/bin/sh
echo "TGR_CALLED"
STUB
    chmod 755 "$dir/tgr" "$dir/tg"

    out=$(PATH="$dir:/usr/bin:/bin" TGR_ACTIVE=1 "$dir/tg" 2>&1)
    case "$out" in
        *"already active"*)
            ok "tg refuses to nest inside tgr"
            ;;
        *)
            bad "tg refuses to nest inside tgr" "unexpected output: $out"
            ;;
    esac
}

test_installs_all_four
test_rejects_bad_checksum
test_missing_asset_fails
test_rejects_unsupported_flag
test_does_not_overwrite_config
test_tg_runs_tgr
test_removes_old_cpp_guard
test_tg_refuses_nested_session

echo ""
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
