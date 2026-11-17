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

The release tarball also carries `examples/prompt.md`, the default workflow
prompt; the installer copies it into your config (see below).

It also writes two config files, each only when you do not already have one:

| file | what it is |
|------|------------|
| `~/.config/trex-guard/config.ini` | the guard's rules |
| `~/.config/tcx/prompt.md` | the workflow prompt `tg fresh` prints |

The prompt lands in `$TCX_HOME/config/prompt.md` when `TCX_HOME` is set, since
that is where `tcx` looks. Both files are yours: an existing one is never
overwritten, so edit freely.

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

It does not overwrite `~/.config/trex-guard/config.ini` or
`~/.config/tcx/prompt.md`. Existing files are kept, because they are where your
rules live. A consequence: a later release's revised default prompt does not
reach you automatically — compare `examples/prompt.md` and copy what you want.

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

The guard's config is in `~/.config/trex-guard/config.ini` and the workflow
prompt in `~/.config/tcx/prompt.md`; both are left in place.

## Building from a checkout

For development only; releases are built by CI.

```sh
cargo build --release --workspace
cargo test --workspace
```
