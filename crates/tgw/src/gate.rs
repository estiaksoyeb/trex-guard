//! Asking `tgr --check` about a staged block.
//!
//! `tgw` never looks at the paste. It runs the gate command, reads an exit
//! code, and maps that code to a verdict. Every mapping lives here, in one
//! function, because a second place that decides "this is safe" is a second
//! place that can be wrong.
//!
//! # The contract
//!
//! | Exit | Meaning | `tgw` |
//! | --- | --- | --- |
//! | 0 | safe | pass |
//! | 1 | danger | block, never approvable |
//! | 3 | review | hold |
//! | 2 | usage or read error | block |
//! | anything else | unspecified | block |
//! | cannot spawn | gate missing | block |
//! | timeout | gate hung | block |
//!
//! The last four are the fail-closed cases. A gate that times out has not said
//! "safe", and an absent gate has said nothing at all, so neither may pass.
//!
//! # Why stderr is parsed for names
//!
//! `tgr --check` prints `rule=<name> risk=<...>` and `watch=<name>`. The names
//! are used for two things: telling the human *why* a block was held, and the
//! `hold_rules` mapping. Nothing in this module interprets a rule name as a
//! risk; that is the gate's answer, in the exit code.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::pipeline::GateResult;

/// Names the gate reported on stderr.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// `rule=<name> risk=<...>` lines.
    pub rules: Vec<(String, String)>,
    /// `watch=<name>` lines.
    pub watch: Vec<String>,
}

impl Report {
    /// Parses the gate's stderr. Unknown lines are ignored, so the gate can
    /// get chattier without breaking this.
    pub fn parse(stderr: &str) -> Report {
        let mut r = Report::default();
        for line in stderr.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("rule=") {
                let mut parts = rest.splitn(2, " risk=");
                let name = parts.next().unwrap_or("").trim().to_string();
                let risk = parts.next().unwrap_or("").trim().to_string();
                if !name.is_empty() {
                    r.rules.push((name, risk));
                }
            } else if let Some(name) = line.strip_prefix("watch=") {
                let name = name.trim();
                if !name.is_empty() {
                    r.watch.push(name.to_string());
                }
            }
        }
        r
    }

    /// A human-readable reason: every rule name, then every watch match.
    pub fn reason(&self) -> String {
        let mut parts: Vec<String> = self
            .rules
            .iter()
            .map(|(n, risk)| format!("{n} ({risk})"))
            .collect();
        parts.extend(self.watch.iter().map(|w| format!("watch:{w}")));
        parts.join(", ")
    }

    /// Every name, rule and watch, for `hold_rules` matching.
    pub fn all_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.rules.iter().map(|(n, _)| n.clone()).collect();
        v.extend(self.watch.iter().cloned());
        v
    }
}

/// Maps a gate exit code to a verdict. The single place that decides.
pub fn verdict_for(code: Option<i32>, stderr: &str) -> GateResult {
    let report = Report::parse(stderr);
    match code {
        Some(0) => GateResult::Pass {
            reason: report.reason(),
        },
        Some(1) => GateResult::Block {
            reason: non_empty(report.reason(), "gate reported danger"),
        },
        Some(3) => GateResult::Hold {
            reason: non_empty(report.reason(), "gate reported review"),
        },
        Some(2) => GateResult::Error {
            reason: "gate error (exit 2)".into(),
        },
        Some(other) => GateResult::Error {
            reason: format!("gate exited {other}, which is not a verdict"),
        },
        None => GateResult::Error {
            reason: "gate was killed by a signal".into(),
        },
    }
}

#[cfg(unix)]
extern "C" {
    /// Declared directly so this crate keeps its `std`-only promise. Calling
    /// it always succeeds for a child that is not already a group leader.
    fn setsid() -> i32;
}

/// Kills the whole process group of `pid`.
///
/// A gate that spawned `sleep` outlives a plain `child.kill()`, and the
/// surviving process keeps the stderr pipe open. Killing the group is what
/// makes a timeout actually return.
#[cfg(unix)]
fn kill_group(pid: u32) {
    let pgid = pid as i32;
    // SAFETY: `kill` is async-signal-safe; a negative pid signals a group.
    unsafe {
        kill(-pgid, 9);
    }
}

#[cfg(unix)]
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

fn non_empty(s: String, fallback: &str) -> String {
    if s.trim().is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

/// Runs the gate on `path` and returns what to do.
///
/// Never panics. A missing binary, a timeout, a spawn error and a non-zero
/// exit all become `GateResult::Error` or `Block`, which the pipeline treats
/// as "do not run".
pub fn check(cfg: &Config, path: &Path) -> GateResult {
    let argv = match crate::config::split_argv(&cfg.gate_cmd) {
        Ok(v) => v,
        Err(e) => {
            return GateResult::Error {
                reason: format!("gate_cmd does not parse: {e}"),
            }
        }
    };
    let argv = crate::config::substitute(&argv, &[("file", &path.to_string_lossy())]);
    if argv.is_empty() {
        return GateResult::Error {
            reason: "gate_cmd is empty".into(),
        };
    }

    let timeout = Duration::from_secs(cfg.gate_timeout_s.max(1));
    // Run the gate in its own session so a timeout can kill everything it
    // spawned. Without this, `gate_cmd = sh -c 'something; sleep 30'` survives
    // the kill of `sh` and keeps the stderr pipe open, which would make the
    // timeout above look like a hang.
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and touches nothing but this
        // process's own session. It fails harmlessly if we are already a
        // group leader, which the executor ignores.
        unsafe {
            cmd.pre_exec(|| {
                let _ = setsid();
                Ok(())
            });
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return GateResult::Error {
                reason: format!("cannot run gate `{}`: {e}", argv[0]),
            }
        }
    };

    // Drain stderr on a thread so a chatty gate cannot fill the pipe and
    // deadlock while we wait for it to exit.
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr_pipe.read_to_string(&mut s);
        s
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    kill_group(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let stderr = reader.join().unwrap_or_default();
    match status {
        None => GateResult::Error {
            reason: format!("gate timed out after {} s", timeout.as_secs()),
        },
        Some(st) => verdict_for(st.code(), &stderr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(cmd: &str) -> Config {
        let mut c = Config::default();
        c.clip_get_cmd = "true".into();
        c.gate_cmd = cmd.into();
        c.gate_timeout_s = 2;
        c
    }

    #[test]
    fn the_four_contract_exit_codes_map_exactly() {
        assert!(matches!(verdict_for(Some(0), ""), GateResult::Pass { .. }));
        assert!(matches!(verdict_for(Some(1), ""), GateResult::Block { .. }));
        assert!(matches!(verdict_for(Some(3), ""), GateResult::Hold { .. }));
        assert!(matches!(verdict_for(Some(2), ""), GateResult::Error { .. }));
    }

    #[test]
    fn every_other_exit_code_blocks() {
        // 5 is not a verdict. Treating it as "not safe, not held" would be a
        // bypass, so it becomes a block.
        for code in [4, 5, 7, 64, 127, 255, -1] {
            match verdict_for(Some(code), "") {
                GateResult::Error { .. } | GateResult::Block { .. } => {}
                other => panic!("exit {code} mapped to {other:?}, which is not fail-closed"),
            }
        }
    }

    #[test]
    fn a_signalled_gate_blocks() {
        assert!(matches!(verdict_for(None, ""), GateResult::Error { .. }));
    }

    #[test]
    fn rule_and_watch_names_are_parsed() {
        let r = Report::parse(
            "rule=rm -rf risk=danger\nrule=git clean risk=review\nwatch=sed-i\nnonsense\n",
        );
        assert_eq!(r.rules.len(), 2);
        assert_eq!(r.rules[0].0, "rm -rf");
        assert_eq!(r.rules[1].1, "review");
        assert_eq!(r.watch, vec!["sed-i"]);
        assert!(r.reason().contains("rm -rf"));
        assert!(r.reason().contains("watch:sed-i"));
        assert_eq!(r.all_names(), vec!["rm -rf", "git clean", "sed-i"]);
    }

    #[test]
    fn a_real_gate_command_passes_a_clean_block() {
        let dir = std::env::temp_dir().join(format!("tgw-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.sh");
        std::fs::write(&f, "set +H\necho hi\n").unwrap();
        // `true` ignores the argument and exits 0.
        let g = check(&cfg_with("true {file}"), &f);
        assert!(matches!(g, GateResult::Pass { .. }), "{g:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_gate_binary_blocks() {
        let g = check(&cfg_with("/nonexistent/tgw-gate {file}"), Path::new("/tmp/x"));
        assert!(matches!(g, GateResult::Error { .. }), "{g:?}");
    }

    #[test]
    fn a_hanging_gate_times_out_and_blocks() {
        // `sh -c` so the extra path argument does not become `sleep`'s
        // operand (which would make sleep exit 1 immediately, not hang).
        let mut c = cfg_with("sh -c 'sleep 30' {file}");
        c.gate_timeout_s = 1;
        let start = Instant::now();
        let g = check(&c, Path::new("/tmp/x"));
        assert!(matches!(g, GateResult::Error { .. }), "{g:?}");
        assert!(start.elapsed() < Duration::from_secs(10), "timeout did not apply");
    }

    #[test]
    fn exit_codes_from_a_real_process_are_mapped() {
        // Uses `sh -c` with an explicit exit so the mapping is exercised end
        // to end, not just in `verdict_for`.
        let cases = [(0, true), (1, false), (3, false)];
        for (code, is_pass) in cases {
            let cmd = format!("sh -c 'exit {}' {{file}}", code);
            let g = check(&cfg_with(&cmd), Path::new("/tmp/x"));
            assert_eq!(
                matches!(g, GateResult::Pass { .. }),
                is_pass,
                "exit {code}: {g:?}"
            );
        }
    }

    #[test]
    fn stderr_from_a_real_gate_is_read() {
        let cmd = "sh -c 'echo rule=rm\\ risk=danger >&2; exit 1' {file}";
        let g = check(&cfg_with(cmd), Path::new("/tmp/x"));
        match g {
            GateResult::Block { reason } => assert!(reason.contains("rm"), "{reason}"),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn a_chatty_gate_does_not_deadlock() {
        // 1 MB of stderr: if the pipe were not drained concurrently the gate
        // would block writing and the timeout would fire.
        let cmd = "sh -c 'i=0; while [ $i -lt 20000 ]; do echo noisy; i=$((i+1)); done; exit 0' {file}";
        let mut c = cfg_with(cmd);
        c.gate_timeout_s = 5;
        let g = check(&c, Path::new("/tmp/x"));
        assert!(matches!(g, GateResult::Pass { .. }), "{g:?}");
    }
}
