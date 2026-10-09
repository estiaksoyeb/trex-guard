# TREX Guard (`tg`)

> **Transparent, zero-latency pseudo-terminal (PTY) safety firewall for clipboard-pasted shell commands.**

[![Release](https://img.shields.io/github/v/release/estiaksoyeb/trex-guard?color=blue&logo=github)](https://github.com/estiaksoyeb/trex-guard/releases)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20Android%20(Termux)-orange.svg)]()
[![Tests](https://img.shields.io/badge/tests-93%2F93%20passed-brightgreen.svg)]()

**TREX Guard** protects your terminal against accidental clipboard disasters. It intercepts pastes at the operating system pseudo-terminal layer *before* characters reach the shell, evaluates commands semantically against destructive threat models, and halts high-risk operations for explicit operator review.

---

## ⚡ Why TREX Guard?

Accidentally pasting multi-line scripts or commands containing `rm -rf`, `git reset --hard`, or remote `curl | bash` pipelines can execute instantly and irreversibly before you even have a chance to read them.

TREX Guard solves this with a **zero-overhead, transparent terminal firewall**:

* **Zero Typing Latency**: Keystrokes, tab-completion, cursor navigation, and interactive CLI programs pass through at native speed with 0ms latency.
* **Frictionless Safe Pastes**: Safe commands (`git status`, directory navigation, code snippets) execute immediately without prompts when `auto_approve_safe = true`.
* **Semantic Threat Recognition**: Detects destructive operations regardless of flag permutations, argument order, path placements, or wrappers (`sudo`, `env`).
* **Ergonomic Review Layout**: The paste preview is printed first, followed by clear severity tags (`HIGH RISK` vs `REVIEW`) right at the decision prompt—no scrolling required on large scripts.
* **Mobile & Terminal Resilient**: Handles terminal window resizes and mobile touch/keyboard focus transitions without cancelling review prompts.

---

## 🛡️ Built-in Threat Detection

TREX Guard inspects commands using an integrated shell command parser and semantic analyzer:

| Threat Category | Severity | Examples Intercepted |
| :--- | :---: | :--- |
| **Recursive / Force Deletion** | `HIGH RISK` | `rm -rf /`, `rm /path -v -r -f`, `sudo rm -fr /var/cache` |
| **Untrusted Remote Execution** | `HIGH RISK` | `curl -sSL ... \| bash`, `wget -qO- ... \| sudo sh` |
| **Destructive Git History Changes** | `REVIEW` | `git reset --hard HEAD~1`, `git push origin main -f` |
| **Unchecked Repository Cleaning** | `REVIEW` | `git clean -fd`, `git clean -fdx` |
| **Raw Block Device Overwrites** | `HIGH RISK` | `cat image.iso > /dev/sda`, `dd if=/dev/zero ...` |
| **Broad Permission Overhauls** | `HIGH RISK` | `chmod -R 777 /`, `chmod 000 /var/www` |
| **Unprompted Bulk File Deletions** | `HIGH RISK` | `find / -delete`, `find . -exec rm {} +` |
| **Obfuscated Dynamic Execution** | `HIGH RISK` | `base64 -d \| sh`, `eval "$UNTRUSTED"` |
| **Resource Exhaustion** | `HIGH RISK` | `:(){:|:&};:` (Fork bombs) |

---

## 🖥️ Terminal Interface Preview

When a dangerous paste is detected, execution is halted immediately:

```text
--- [Pasted Content] (118B, 3L) ---
echo "Starting cleanup..."
rm -rf /var/cache/app
git reset HEAD~1 --hard

--- [Security Review] ---
[HIGH RISK] (2 flagged commands found):
  1. [rm -rf] rm -rf /var/cache/app
     Risk: Recursive force deletion permanently removes directories without prompt
  2. [git hard reset] git reset HEAD~1 --hard
     Risk: Discards all uncommitted changes and resets branch head

Execute? [Enter/^C] 
```

* **Approve**: Press **Enter** or **y** to execute the paste.
* **Cancel**: Press **Ctrl-C**, **Esc**, or **n** to discard the paste safely.

---

## 🚀 Quick Start

### 1-Line Install (Recommended — No Compiler Required)

Install prebuilt static binaries for your system (`x86_64` or `aarch64` / ARM64):

```bash
curl -fsSL https://raw.githubusercontent.com/estiaksoyeb/trex-guard/master/scripts/install.sh | bash
```

### Build & Install From Source

```bash
git clone https://github.com/estiaksoyeb/trex-guard.git
cd trex-guard
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
sudo cmake --install build
```

---

## 💡 Usage

Launch a guarded shell session:

```bash
tg
```

### Auto-Launch With Every Shell (Recommended)

To protect every terminal session automatically, add this to your `~/.bashrc` or `~/.zshrc`:

```bash
[ -z "$TREX_GUARD_ACTIVE" ] && [ -t 0 ] && exec tg
```

### Prompt Guard Badge (Optional)

Inside an active session, `$TREX_GUARD_ACTIVE=1` is exported. Add a visual status indicator to your `PS1`:

```bash
[ "$TREX_GUARD_ACTIVE" = "1" ] && PS1="\[\033[1;32m\][GUARD]\[\033[0m\] $PS1"
```

---

## ⚙️ Configuration

Custom settings are managed via `~/.config/trex-guard/config.ini`:

```ini
[general]
# Automatically run safe pastes without prompting (default: true)
auto_approve_safe = true

# Compact review mode: hide multi-line descriptions for single-line findings (default: false)
compact_review = false

[allowlist]
# Pastes matching these substrings or patterns are always treated as Safe
git push --force origin scratch-branch
curl -fsSL https://internal-corp.domain/setup.sh | bash

[disabled_rules]
# Disable specific built-in safety rules if desired
git force push

[custom_rules]
# Define custom regex rules: <name> | <risk> | <pattern> | <explanation>
drop database | danger | \bdrop\s+database\b | Destructive SQL drop database statement
npm publish   | review | \bnpm\s+publish\b   | Publishes package to public registry
```

---

## 📚 Documentation

* [Configuration Guide](docs/configuration.md) — Custom rules, allowlists, and configuration syntax
* [Installation & Shell Integration](docs/install.md) — Shell hooks, packaging, and setup options
* [Architecture & Design](docs/architecture.md) — PTY interception model and security boundaries
* [Changelog & Development Log](docs/development-log.md) — Detailed feature progression and version history

---

## 📄 License

Distributed under the [MIT License](LICENSE).
