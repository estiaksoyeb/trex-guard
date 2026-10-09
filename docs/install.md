# Installation and Integration Guide

TREX Guard can be installed via a quick one-line script (no compiler needed) or built from source with CMake.

---

## 1. Quick Install (No C/C++ Environment Required)

If you don't have a C++ compiler or CMake installed, run the automated installer:

```bash
curl -fsSL https://raw.githubusercontent.com/estiaksoyeb/trex-guard/master/scripts/install.sh | bash
```

This automatically:
- Detects your system architecture (`x86_64`, `aarch64`, etc.).
- Downloads the matching precompiled static release.
- Installs `trex-guard` and the `tg` launcher into your `$PATH` (`/usr/local/bin`, `$PREFIX/bin`, or `~/.local/bin`).
- Sets up default configuration at `~/.config/trex-guard/config.ini`.

---

## 2. Building and Running From Source

To build without installing system-wide:

```bash
cmake -B build
cmake --build build
```

Run test suite:
```bash
./build/test_rules
# or
ctest --test-dir build --output-on-failure
```

Launch the guarded shell locally:
```bash
./scripts/tg
# or
./build/trex-guard
```

---

## 2. Installing

### System-wide (Default `/usr/local`):
```bash
sudo cmake --install build
```

### User-local (`~/.local`):
```bash
cmake --install build --prefix ~/.local
```
*(Ensure `~/.local/bin` is in your `$PATH`)*

This installs:
- Binary: `trex-guard` (and `tg` launcher script) in `bin/`
- Data / templates: `config.ini` and `shell-integration.bash` in `share/trex-guard/`

---

## 3. Shell Integration

### Optional Alias & Prompt Indicator
To display a `[GUARD]` badge in your prompt whenever `tg` is active, add this one-liner to your `~/.bashrc`:

```bash
[ "$TREX_GUARD_ACTIVE" = "1" ] && PS1="\[\033[1;32m\][GUARD]\[\033[0m\] $PS1"
```

Alternatively, source the bundled helper script:
```bash
# If installed system-wide:
source /usr/local/share/trex-guard/shell-integration.bash

# Or if installed user-local:
source ~/.local/share/trex-guard/shell-integration.bash
```

This provides:
- The `tg` shortcut to launch the guard.
- The `trex_guard_indicator` function to display `[GUARD]` in your custom prompt.
- Automatically prevents nested guards.

### Auto-launch on Terminal Open
To automatically guard interactive terminal sessions, add to `~/.bashrc` or `~/.zshrc`:

```bash
[ -z "$TREX_GUARD_ACTIVE" ] && [ -t 0 ] && exec tg
```

---

## 4. Configuration

Copy the example configuration to your user config directory:

```bash
mkdir -p ~/.config/trex-guard
cp examples/config.ini ~/.config/trex-guard/config.ini
```

Edit `~/.config/trex-guard/config.ini` to customize:
- `auto_approve_safe = true/false`
- `[allowlist]`
- `[disabled_rules]`
- `[custom_rules]`

---

## 5. Uninstalling

From your build directory:

```bash
sudo cmake --build build --target uninstall
# or without sudo if installed user-locally:
cmake --build build --target uninstall
```
