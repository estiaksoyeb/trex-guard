# Installing trex-guard

There is one way to install: from a published GitHub release.

```sh
curl -fsSL https://raw.githubusercontent.com/estiaksoyeb/trex-guard/master/scripts/install.sh | sh
```

That installs `tgr`, `tgw`, `tcx` and the `tg` launcher, verifying a SHA-256
checksum before anything is installed. Linux `x86_64` and `aarch64`; the
binaries are statically linked, so no particular glibc is needed.

## What gets installed

| file | what it is |
|------|------------|
| `tgr` | the guard |
| `tg`  | launcher: starts `tgr`, or passes `fresh`/`resume` to `tcx` |
| `tcx` | context store |
| `tgw` | clipboard watchdog |

The install directory is chosen in this order: `--prefix`, Termux's `$PREFIX`,
a writable `/usr/local/bin`, then `~/.local/bin`. If the directory is not on
`PATH`, the installer says so rather than editing your shell files.

## Options

```sh
sh scripts/install.sh --version v0.2.0     # a specific release
sh scripts/install.sh --prefix ~/.local/bin
sh scripts/install.sh --no-verify          # skip the checksum (not recommended)
```

## What it will not do

It does not build from source. If the release is missing, or the checksum does
not match, it stops. Nothing reaches your machine that CI did not build and
test. That means a working release must exist — which is why CI runs before any
tag is pushed.

It does not overwrite `~/.config/trex-guard/config.ini`. An existing config is
kept, because it is where your rules live.

If an old C++ `trex-guard` binary is found in the install directory, it is
removed. The C++ guard is no longer part of the project.

## Verifying an install

```sh
tgr --check <(echo 'rm -rf /tmp/x')   # should print a danger verdict, exit 1
tgr --check <(echo 'echo hello')      # should print nothing, exit 0
tg                                    # a guarded shell
```

## Uninstalling

```sh
rm -f /usr/local/bin/{tgr,tgw,tcx,tg}
```

The guard's config is in `~/.config/trex-guard/config.ini` and is left in place.

## Building from a checkout

For development only; releases are built by CI.

```sh
cargo build --release --workspace
cargo test --workspace
```
