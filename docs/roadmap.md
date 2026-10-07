# Roadmap

## Phase 1 — Transparent PTY

- [x] CMake project
- [x] C++17 executable
- [x] `forkpty()` shell creation
- [x] PTY input/output forwarding
- [ ] Interactive behavior validation
- [ ] Resize propagation
- [ ] Signal/job-control validation

## Phase 2 — Paste interception

- [x] Determine reliable paste-boundary behavior through the PTY
- [x] Capture a complete paste without executing it
- [x] Gate forwarding: buffer the paste and release only on approval
- [x] Release captured input after approval (single-key `y`/other)
- [ ] Preserve ordinary typing (verify non-paste input unaffected)
- [ ] Preserve Ble.sh compatibility (known repaint friction after cancel)

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
- [x] `find ... -delete`
- [ ] shell execution/evaluation
- [x] download-and-pipe-to-shell patterns
- [ ] embedded scripts/heredocs

The scanner should eventually normalize commands instead of relying only on raw substring matching.

## Phase 4 — Human approval UI

- [ ] Safe result
- [ ] Review result
- [ ] High-risk result
- [ ] Exact suspicious command display
- [ ] Explanation of risk
- [ ] Enter = approve
- [ ] Ctrl-C = cancel

## Phase 5 — Configuration

Potential configuration location:

`~/.config/trex-guard/`

Potential configuration:

- enabled/disabled checks
- severity thresholds
- custom rules
- allowlists
- project-specific rules
- UI preferences
- logging preferences

Exact configuration format will be decided after the core architecture is validated.

## Phase 6 — Installation/integration

- [ ] install command
- [ ] optional shell integration
- [ ] optional guarded-shell launcher
- [ ] documentation
- [ ] uninstall procedure
