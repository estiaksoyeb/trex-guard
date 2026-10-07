# Roadmap

## Phase 1 — Transparent PTY

- [x] CMake project
- [x] C++17 executable
- [x] `forkpty()` shell creation
- [x] PTY input/output forwarding
- [x] Interactive behavior validation
- [x] Resize propagation
- [x] Signal/job-control validation (SIGTERM/SIGHUP/SIGINT/SIGQUIT and atexit cleanup)

## Phase 2 — Paste interception

- [x] Determine reliable paste-boundary behavior through the PTY
- [x] Capture a complete paste without executing it
- [x] Gate forwarding: buffer the paste and release only on approval
- [x] Release captured input after approval (single-key `y`/other)
- [x] Preserve ordinary typing (verify non-paste input unaffected)
- [x] Preserve Ble.sh compatibility (fixed repaint friction after cancel via child interrupt)

## Phase 3 — Safety scanner

v0 (literal-string scanner, `src/rules.cpp`) is in place: ten rules
(`rm -rf`, `mkfs`, `dd if=`, write to block device, `chmod -R 777 /`,
`curl`, `wget`, `git push --force`, `git reset --hard`, fork bomb).
It flags matches with `[DANGER: <rule>]` in the approval header.
It does not parse shell, so obfuscated/quoted forms evade it.

- [x] `rm` recursive/force deletion
- [x] `git reset --hard`
- [x] filesystem formatting tools (`mkfs`)
- [x] fork bomb
- [x] Narrow `curl`/`wget` rules to actual pipe-to-shell forms
- [ ] Replace literal scanner with a real shell parser
- [x] destructive `git clean`
- [x] destructive checkout/restore
- [x] `dd`
- [x] recursive permission changes
- [x] shell execution/evaluation (eval, subshell -c, inline scripts)
- [x] download-and-pipe-to-shell patterns
- [x] embedded scripts/heredocs (heredoc piped to shell, encoded pipes)

The scanner should eventually normalize commands instead of relying only on raw substring matching.

## Phase 4 — Human approval UI

- [x] Safe result
- [x] Review result
- [x] High-risk result
- [x] Exact suspicious command display
- [x] Explanation of risk
- [x] Enter = approve
- [x] Ctrl-C = cancel

## Phase 5 — Configuration

Configuration file location: `~/.config/trex-guard/config.ini` (or `$XDG_CONFIG_HOME/trex-guard/config.ini`)
Example provided in `examples/config.ini`.

- [x] enabled/disabled checks (`[disabled_rules]`)
- [x] severity thresholds / tiers (`[custom_rules]` support `review` and `danger`)
- [x] custom rules (`[custom_rules]` with pattern and explanation)
- [x] allowlists (`[allowlist]` to bypass checks for known safe patterns)
- [x] UI preferences (`auto_approve_safe = true/false`)

## Phase 6 — Installation/integration

- [x] install command (`cmake --install` supports system or `--prefix ~/.local`)
- [x] optional shell integration (`scripts/shell-integration.bash` with aliases and prompt indicator)
- [x] optional guarded-shell launcher (`scripts/tg` wrapper with recursion prevention)
- [x] documentation (`docs/install.md` and updated `README.md`)
- [x] uninstall procedure (`cmake --build build --target uninstall`)
