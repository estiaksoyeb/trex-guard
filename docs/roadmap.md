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

- [ ] Determine reliable paste-boundary behavior through the PTY
- [ ] Capture a complete paste without executing it
- [ ] Preserve ordinary typing
- [ ] Preserve Ble.sh compatibility
- [ ] Release captured input after approval

## Phase 3 — Safety scanner

Initial high-risk categories:

- [ ] `rm` recursive/force deletion
- [ ] `git reset --hard`
- [ ] destructive `git clean`
- [ ] destructive checkout/restore
- [ ] filesystem formatting tools
- [ ] `dd`
- [ ] recursive permission changes
- [ ] `find ... -delete`
- [ ] shell execution/evaluation
- [ ] download-and-pipe-to-shell patterns
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
