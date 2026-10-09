# Configuration & Custom Rules Guide

TREX Guard (`tg`) is configured through a simple INI configuration file.

---

## 1. Config File Location

TREX Guard automatically searches for configuration in:
- `~/.config/trex-guard/config.ini`
- Or `$XDG_CONFIG_HOME/trex-guard/config.ini` (if `$XDG_CONFIG_HOME` is set)

To initialize your user configuration file:

```bash
mkdir -p ~/.config/trex-guard
cp examples/config.ini ~/.config/trex-guard/config.ini
```

If no configuration file is present, TREX Guard uses safe defaults (`auto_approve_safe = true` with all built-in rules enabled).

---

## 2. Configuration Sections

### `[general]`

```ini
[general]
# auto_approve_safe: Controls gating behavior for harmless pastes.
# - true (default): Harmless commands pass through to the shell with zero friction.
# - false: EVERY paste requires you to press Enter to approve.
auto_approve_safe = true
```

---

### `[allowlist]`

Lines listed in `[allowlist]` are treated as trusted substrings. If any line in a paste matches an allowlist entry, the paste is immediately treated as `[SAFE]` and bypasses all warnings.

```ini
[allowlist]
# Bypass checks for your company or trusted scripts
curl -fsSL https://internal.corp.domain/setup.sh | bash
git push --force origin scratch-branch
```

---

### `[disabled_rules]`

You can disable specific built-in rules by name without disabling the entire firewall.

Available built-in rule names:
- `fork bomb`
- `rm -rf`
- `file deletion`
- `mkfs`
- `dd`
- `write to block device`
- `chmod`
- `pipe to shell`
- `find delete`
- `git force push`
- `git hard reset`
- `git clean`
- `git checkout/restore`
- `shell eval`
- `encoded shell pipe`
- `heredoc to shell`
- `inline script execution`
- `subshell execution`

Example:
```ini
[disabled_rules]
git force push
file deletion
```

---

### `[custom_rules]`

You can define custom detection rules to intercept any command you want.

#### Syntax:
```text
<Rule Name> | <risk: review OR danger> | <regex_pattern> | <Explanation>
```

- **Rule Name**: Name displayed on the security review card.
- **Risk**:
  - `review`: Triggers a yellow `[REVIEW]` card requiring confirmation.
  - `danger`: Triggers a red `[HIGH RISK]` card requiring confirmation.
- **Regex Pattern**: Standard ECMAScript regex pattern.
- **Explanation**: Human-readable warning explaining why the command was flagged.

#### Examples:

1. **Flag Gradle Tasks (`gradlew` / `./gradlew`)**:
   ```ini
   [custom_rules]
   gradlew build | review | \b(?:\./)?gradlew\b | Triggers Gradle compilation tasks
   ```

2. **Flag `ls` command**:
   ```ini
   [custom_rules]
   ls command | review | \bls\b | Directory listing command
   ```
   *(Note: `\bls\b` ensures word boundaries so words like `pulse` or `also` are not falsely matched).*

3. **Flag Python script executions**:
   ```ini
   [custom_rules]
   python script | review | \bpython[0-9.]*\s+\S+ | Executes a Python script file
   ```

4. **Flag Database Drops as High Risk (`danger`)**:
   ```ini
   [custom_rules]
   database drop | danger | \bdrop\s+database\b | Destructive SQL drop database statement
   ```

5. **Flag NPM / Yarn / PNPM publishing**:
   ```ini
   [custom_rules]
   package publish | review | \b(npm|yarn|pnpm)\s+publish\b | Publishes package to registry
   ```

---

## 3. Capabilities & Boundaries

### What TREX Guard CAN Do:
1. **Pre-Execution Paste Interception**: Intercepts clipboard pastes in memory inside the PTY proxy before the shell line editor ever executes them.
2. **Multi-Threat Bottom Review**: Gathers all risky operations in a paste and displays them in a numbered bottom review panel directly above `Execute? [Enter/^C]`.
3. **Zero-Friction Transparency**: Automatically executes benign commands when `auto_approve_safe = true`.
4. **Mobile & Termux Resilient**: Safely handles on-screen keyboard unfocusing, window resizes (`SIGWINCH`), and terminal focus reporting (`\x1b[O`) without accidental cancellations.
5. **Fast `ble.sh` Batch Integration**: Preserves bracketed paste markers so `ble.sh` runs fast in-memory batch insertion with zero input lag (`██▌ processing input...` eliminated).

### What TREX Guard CANNOT Do (By Design):
1. **Manually Typed Keystrokes**: TREX Guard is a **paste firewall**, not a keylogger. It only gates commands pasted from clipboard. Manually typed keystrokes pass directly to your shell.
2. **Runtime Shell Variable Resolution**: It scans the literal text stream for suspicious patterns and syntax. It does not execute a full Bash interpreter to resolve runtime dynamic variable expansions (e.g., `A=rm; B=-rf; $A $B /`).
3. **Unguarded Shells**: Commands executed outside an active `tg` session are not guarded. Run `tg` or configure your shell to auto-launch it on login.

---

## 4. Checking If You Are Inside `tg`

To verify whether your current terminal session is guarded:

### 1. Environment Variable (Quickest One-Liner)
`tg` exports `TREX_GUARD_ACTIVE=1` into every child shell it launches:
```bash
echo $TREX_GUARD_ACTIVE
```
- Outputs `1`: **Inside** `tg` (guarded).
- Outputs blank / empty: **Outside** (unguarded).

### 2. Built-In Self-Detection Notice
If you type `tg` while already inside a guarded session:
```text
Notice: A guarded shell is already active in this terminal session.
To launch anyway, run: tg --force
```

### 3. Parent Process Check
```bash
ps -p $PPID -o comm=
```
- Outputs `trex-guard` when running inside the safety proxy.

### 4. Visual Prompt Badge (`PS1`)
To display a persistent badge on your command prompt whenever `tg` is active, add this one-liner to your `~/.bashrc` (or `~/.zshrc`):

```bash
[ "$TREX_GUARD_ACTIVE" = "1" ] && PS1="\[\033[1;32m\][GUARD]\[\033[0m\] $PS1"
```

When active, your prompt will automatically show:
```text
[GUARD] user@host ~/path $
```
When you exit `tg`, the badge automatically disappears.

