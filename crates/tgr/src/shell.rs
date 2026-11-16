//! Paste -> pipelines -> commands.
//!
//! Ported from `src/shell_parser.cpp`. This is a *scanner*, not a shell: it
//! understands quotes, escapes, comments, separators and pipes well enough to
//! find the command names and flags the rules care about. It deliberately does
//! not expand variables or resolve aliases.
//!
//! # Why the quirks are kept
//!
//! Every branch here decides whether a command is *seen*. A "cleaner" rewrite
//! that handles one more quoting case can also stop seeing a command the C++
//! version caught, or start seeing text inside a comment. Since the guard's
//! value is "this exact paste was judged this way", the port follows the
//! original control flow, including:
//!
//! - `||` and `&&` end a pipeline, but a bare `|` continues it;
//! - an unterminated quote simply runs to the end (no error);
//! - `#` starts a comment only at a token boundary;
//! - `\` + newline is a continuation.

/// One command inside a pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedCommand {
    /// Original text of this stage.
    pub raw_snippet: String,
    /// Normalised executable name after path and wrapper stripping.
    pub base_name: String,
    /// Executable as written, before path stripping.
    pub raw_executable: String,
    /// Wrappers unwrapped to reach the executable (`sudo`, `env`, ...).
    pub wrappers: Vec<String>,
    /// Leading `NAME=value` assignments.
    pub env_vars: Vec<String>,
    /// Arguments, flags and positionals together.
    pub args: Vec<String>,
}

impl ParsedCommand {
    /// Whether this exact argument is present.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.args.iter().any(|a| a == flag)
    }

    /// Whether a short-flag cluster contains `c` (`-rf` contains `r` and `f`).
    pub fn has_short_flag_char(&self, c: char) -> bool {
        self.args.iter().any(|a| {
            a.len() >= 2 && a.starts_with('-') && !a.starts_with("--") && a[1..].contains(c)
        })
    }

    /// Whether `--name` or `--name=value` is present.
    pub fn has_long_flag(&self, name: &str) -> bool {
        let prefix = format!("--{name}");
        self.args
            .iter()
            .any(|a| a == &prefix || a.starts_with(&format!("{prefix}=")))
    }

    /// Whether this exact argument is present.
    pub fn has_arg(&self, arg: &str) -> bool {
        self.args.iter().any(|a| a == arg)
    }

    /// Arguments that do not start with `-`.
    pub fn positional_args(&self) -> Vec<&str> {
        self.args
            .iter()
            .filter(|a| !a.is_empty() && !a.starts_with('-'))
            .map(String::as_str)
            .collect()
    }
}

/// Commands joined by pipes, so `curl ... | bash` can be judged as a unit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pipeline {
    pub stages: Vec<ParsedCommand>,
    pub raw_snippet: String,
}

/// Wrappers that obscure the real command.
const KNOWN_WRAPPERS: &[&str] = &[
    "sudo", "doas", "env", "nice", "nohup", "command", "builtin", "exec", "time", "xargs",
];

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_env_assignment(token: &str) -> bool {
    let Some(eq) = token.find('=') else {
        return false;
    };
    if eq == 0 {
        return false;
    }
    let bytes = token.as_bytes();
    if !is_ident_start(bytes[0]) {
        return false;
    }
    bytes[1..eq].iter().all(|&b| is_ident_char(b))
}

fn basename(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(pos) => &path[pos + 1..],
        None => path,
    }
}

fn build_command(tokens: &[String], raw_snippet: &str) -> ParsedCommand {
    let mut cmd = ParsedCommand {
        raw_snippet: raw_snippet.to_string(),
        ..Default::default()
    };
    if tokens.is_empty() {
        return cmd;
    }

    let mut idx = 0;

    // 1. Leading environment assignments.
    while idx < tokens.len() && is_env_assignment(&tokens[idx]) {
        cmd.env_vars.push(tokens[idx].clone());
        idx += 1;
    }

    // 2. Strip wrappers, consuming their own flags.
    while idx < tokens.len() {
        let base = basename(&tokens[idx]).to_string();
        if !KNOWN_WRAPPERS.contains(&base.as_str()) {
            break;
        }
        cmd.wrappers.push(base.clone());
        idx += 1;

        match base.as_str() {
            "sudo" | "doas" => {
                while idx < tokens.len() && tokens[idx].starts_with('-') {
                    if tokens[idx] == "--" {
                        idx += 1;
                        break;
                    }
                    let flag = &tokens[idx];
                    idx += 1;
                    // Flags that take a separate value.
                    if matches!(flag.as_str(), "-u" | "-g" | "-p" | "-C") && idx < tokens.len() {
                        idx += 1;
                    }
                }
            }
            "env" => {
                while idx < tokens.len() && !tokens[idx].is_empty() {
                    let tok = &tokens[idx];
                    if tok == "-i" || tok == "--ignore-environment" {
                        idx += 1;
                    } else if (tok == "-u" || tok == "--unset") && idx + 1 < tokens.len() {
                        idx += 2;
                    } else if is_env_assignment(tok) {
                        cmd.env_vars.push(tok.clone());
                        idx += 1;
                    } else {
                        break;
                    }
                }
            }
            "nice" => {
                while idx < tokens.len() && tokens[idx].starts_with('-') {
                    let flag = &tokens[idx];
                    idx += 1;
                    if flag == "-n" && idx < tokens.len() {
                        idx += 1;
                    }
                }
            }
            _ => {
                // command, builtin, exec, time, nohup, xargs
                while idx < tokens.len() && tokens[idx].starts_with('-') {
                    if tokens[idx] == "--" {
                        idx += 1;
                        break;
                    }
                    idx += 1;
                }
            }
        }
    }

    if idx < tokens.len() {
        cmd.raw_executable = tokens[idx].clone();
        cmd.base_name = basename(&tokens[idx]).to_string();
        idx += 1;
    }

    cmd.args.extend(tokens[idx..].iter().cloned());
    cmd
}

/// Splits a paste into pipelines of commands.
pub fn parse(paste: &str) -> Vec<Pipeline> {
    let mut pipelines: Vec<Pipeline> = Vec::new();
    let mut stages: Vec<ParsedCommand> = Vec::new();
    let mut stage_snippet = String::new();
    let mut pipeline_snippet = String::new();
    let mut tokens: Vec<String> = Vec::new();
    let mut token = String::new();

    let mut in_single = false;
    let mut in_double = false;
    let mut in_comment = false;

    macro_rules! finish_token {
        () => {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        };
    }
    macro_rules! finish_stage {
        () => {
            finish_token!();
            if !tokens.is_empty() {
                stages.push(build_command(&tokens, &stage_snippet));
                tokens.clear();
            }
            stage_snippet.clear();
        };
    }
    macro_rules! finish_pipeline {
        () => {
            finish_stage!();
            if !stages.is_empty() {
                pipelines.push(Pipeline {
                    stages: std::mem::take(&mut stages),
                    raw_snippet: std::mem::take(&mut pipeline_snippet),
                });
            }
            pipeline_snippet.clear();
        };
    }

    let chars: Vec<char> = paste.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let c = chars[i];

        if in_comment {
            pipeline_snippet.push(c);
            if c == '\n' || c == '\r' {
                in_comment = false;
                finish_pipeline!();
            }
            i += 1;
            continue;
        }

        if in_single {
            stage_snippet.push(c);
            pipeline_snippet.push(c);
            if c == '\'' {
                in_single = false;
            } else {
                token.push(c);
            }
            i += 1;
            continue;
        }

        if in_double {
            stage_snippet.push(c);
            pipeline_snippet.push(c);
            if c == '\\' && i + 1 < n {
                let next = chars[i + 1];
                if matches!(next, '"' | '\\' | '$' | '`') {
                    token.push(next);
                    stage_snippet.push(next);
                    pipeline_snippet.push(next);
                    i += 2;
                    continue;
                }
            }
            if c == '"' {
                in_double = false;
            } else {
                token.push(c);
            }
            i += 1;
            continue;
        }

        if c == '\\' {
            if i + 1 < n && chars[i + 1] == '\n' {
                stage_snippet.push_str("\\\n");
                pipeline_snippet.push_str("\\\n");
                i += 2;
                continue;
            }
            if i + 1 < n {
                stage_snippet.push(c);
                stage_snippet.push(chars[i + 1]);
                pipeline_snippet.push(c);
                pipeline_snippet.push(chars[i + 1]);
                token.push(chars[i + 1]);
                i += 2;
                continue;
            }
        }

        if c == '\'' {
            in_single = true;
            stage_snippet.push(c);
            pipeline_snippet.push(c);
            i += 1;
            continue;
        }

        if c == '"' {
            in_double = true;
            stage_snippet.push(c);
            pipeline_snippet.push(c);
            i += 1;
            continue;
        }

        // A `#` starts a comment only at a token boundary.
        if c == '#' && token.is_empty()
            && (stage_snippet.is_empty()
                || stage_snippet.chars().last().is_some_and(char::is_whitespace))
        {
            in_comment = true;
            pipeline_snippet.push(c);
            i += 1;
            continue;
        }

        if c == '|' {
            if i + 1 < n && chars[i + 1] == '|' {
                pipeline_snippet.push_str("||");
                finish_pipeline!();
                i += 2;
                continue;
            }
            pipeline_snippet.push(c);
            if i + 1 < n && chars[i + 1] == '&' {
                pipeline_snippet.push('&');
                i += 1;
            }
            finish_stage!();
            i += 1;
            continue;
        }

        if c == ';' {
            pipeline_snippet.push(c);
            finish_pipeline!();
            i += 1;
            continue;
        }

        if c == '&' {
            if i + 1 < n && chars[i + 1] == '&' {
                pipeline_snippet.push_str("&&");
                finish_pipeline!();
                i += 2;
                continue;
            }
            pipeline_snippet.push(c);
            finish_pipeline!();
            i += 1;
            continue;
        }

        if c == '\n' || c == '\r' {
            pipeline_snippet.push(c);
            finish_pipeline!();
            i += 1;
            continue;
        }

        if c.is_whitespace() {
            stage_snippet.push(c);
            pipeline_snippet.push(c);
            finish_token!();
            i += 1;
            continue;
        }

        token.push(c);
        stage_snippet.push(c);
        pipeline_snippet.push(c);
        i += 1;
    }

    finish_pipeline!();
    pipelines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bases(paste: &str) -> Vec<String> {
        parse(paste)
            .into_iter()
            .flat_map(|p| p.stages)
            .map(|s| s.base_name)
            .collect()
    }

    #[test]
    fn splits_statements_and_pipelines() {
        assert_eq!(
            bases("echo a && rm x; git status | grep foo"),
            vec!["echo", "rm", "git", "grep"]
        );
    }

    #[test]
    fn a_double_pipe_is_a_separator_not_a_pipe() {
        let p = parse("a || b");
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].stages.len(), 1);
    }

    #[test]
    fn strips_paths_and_wrappers() {
        let p = parse("sudo env FOO=1 /bin/rm -rf /tmp");
        let cmd = &p[0].stages[0];
        assert_eq!(cmd.base_name, "rm");
        assert_eq!(cmd.args, vec!["-rf", "/tmp"]);
        assert!(cmd.wrappers.contains(&"sudo".to_string()));
        assert!(cmd.wrappers.contains(&"env".to_string()));
    }

    #[test]
    fn sudo_flags_that_take_values_are_skipped() {
        let p = parse("sudo -u root rm -rf /");
        assert_eq!(p[0].stages[0].base_name, "rm");
    }

    #[test]
    fn quoted_text_stays_one_token() {
        let p = parse("echo 'a b' \"c d\"");
        assert_eq!(p[0].stages[0].args, vec!["a b", "c d"]);
    }

    #[test]
    fn a_comment_does_not_hide_the_next_line() {
        assert_eq!(bases("# note\nrm -rf /"), vec!["rm"]);
    }

    #[test]
    fn short_flag_clusters_are_seen() {
        let p = parse("rm -fr /tmp");
        let cmd = &p[0].stages[0];
        assert!(cmd.has_short_flag_char('r'));
        assert!(cmd.has_short_flag_char('f'));
    }

    #[test]
    fn long_flags_accept_equals_values() {
        let p = parse("git reset --hard=HEAD");
        assert!(p[0].stages[0].has_long_flag("hard"));
    }

    #[test]
    fn carriage_returns_separate_statements() {
        assert_eq!(bases("# c\rrm -rf /x\r"), vec!["rm"]);
    }
}
