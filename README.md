# trex-guard

A terminal paste gate: blocks pasted into a shell are classified before they
run, and anything dangerous or worth a second look is stopped or held.

Three tools, all Rust:

| tool  | what it does                                                        |
|-------|---------------------------------------------------------------------|
| `tgr` | the guard. Launches a shell and classifies each paste              |
| `tg`  | launcher: starts a guarded shell, or passes `fresh`/`resume` to `tcx` |
| `tcx` | context store: decisions, gotchas, and a budgeted session briefing  |
| `tgw` | clipboard watchdog: holds a copied block until you approve it       |

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/estiaksoyeb/trex-guard/master/scripts/install.sh | sh
```

Installs `tgr`, `tg`, `tcx` and `tgw` from the latest GitHub release, verifying
a published SHA-256 checksum before anything is installed. Linux `x86_64` and
`aarch64`; the binaries are statically linked, so no specific glibc is required.

Options, if you are not piping to `sh`:

```sh
sh scripts/install.sh --version v0.2.0     # a specific release
sh scripts/install.sh --prefix ~/.local/bin
```

The installer does **not** build from source. If a release is missing or its
checksum does not match, it stops. Nothing reaches your machine that CI did not
build and test.

## Use

```sh
tg                      # guarded shell
tgr --check block.sh    # ask the guard about a file without running it
tgr --check -           # ...or stdin
tcx --help              # context store
tgw --help              # watchdog
```

`tgr --check` exits `0` safe, `1` danger, `3` review, `2` usage or read error.

Rules live in `~/.config/trex-guard/config.ini`, shared by `tgr` and the
original C++ guard.

## Development

```sh
cargo test --workspace   # includes the pty and parity suites
./tests/install_test.sh  # installer, against a fake release
```

The pty and parity suites drive real guards through a terminal. They take a
few minutes; that is the price of testing the thing that actually matters.

Releases are built by GitHub Actions on a `v*` tag: see
`.github/workflows/release.yml`. The C++ guard is no longer released — `tg`
runs `tgr` — but its source stays in the tree and its rule tests still run in
CI as the reference `tgr` is checked against.
