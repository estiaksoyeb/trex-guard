//! `tg fresh` - the prompt, with the project section filled in.
//!
//! This is the command that starts a chat. The user runs it once, pastes the
//! result, and does nothing else. Everything here exists to make that paste
//! correct without the user composing anything.
//!
//! # Inputs
//!
//! 1. A prompt template file, because the rules are edited far more often than
//!    the tool is rebuilt.
//! 2. The repository the user is standing in, read (never executed) to fill the
//!    `## Project` section.
//!
//! # Location of the template
//!
//! The template lives in *your* config directory, never in the repository, so
//! a checkout cannot change the rules an AI is given:
//!
//! 1. the `template` argument, when the caller passes one
//! 2. `<config>/prompt.md`
//! 3. `<config>/tg/prompt.md` (legacy name)
//!
//! `<config>` is `$TCX_HOME/config` when `TCX_HOME` is set, otherwise
//! `~/.config/tcx`. Nothing is read from the repository itself.
//!
//! # Security
//!
//! The filled section is derived from the repository, and the result is pasted
//! into a chat. Manifest text is therefore sanitised before it is rendered:
//! control characters and escape sequences are stripped and each field is
//! length-capped.

use std::fs;
use std::path::{Path, PathBuf};

use crate::notes::Note;
use crate::project::{self, Project};
use crate::store::Store;

/// Errors from building a briefing.
#[derive(Debug)]
pub enum Error {
    /// No prompt template was found at any of the candidate paths.
    NoTemplate(Vec<PathBuf>),
    /// A template was found but could not be read.
    Io(PathBuf, std::io::Error),
    /// The project root could not be determined.
    NoRoot,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoTemplate(paths) => {
                write!(f, "no prompt template found. Looked in:")?;
                for path in paths {
                    write!(f, "\n  {}", path.display())?;
                }
                Ok(())
            }
            Error::Io(path, e) => write!(f, "cannot read {}: {e}", path.display()),
            Error::NoRoot => write!(f, "cannot find a project root above the current directory"),
        }
    }
}

impl std::error::Error for Error {}

/// What to include in the briefing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Prompt only. Starts a chat.
    Fresh,
    /// Prompt plus what the previous session learned. Continues in a new chat.
    Resume,
}

/// Options for building a briefing.
#[derive(Debug, Clone)]
pub struct Options {
    /// Byte cap on the notes section of a resume briefing.
    pub budget: usize,
    /// Explicit template path; skips discovery.
    pub template: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            budget: 6000,
            template: None,
        }
    }
}

/// Finds the template to use.
///
/// Returns the template text and the path it came from, so the caller can
/// report where the rules being applied live.
///
/// The search never looks inside the repository. An explicit path is used as
/// given; otherwise only the config directory is consulted.
pub fn template(store: &Store, explicit: Option<&Path>) -> Result<(String, PathBuf), Error> {
    let candidates: Vec<PathBuf> = match explicit {
        Some(path) => vec![path.to_path_buf()],
        None => vec![
            store.config.join("prompt.md"),
            store.config.join("tg").join("prompt.md"),
        ],
    };

    for path in &candidates {
        match fs::read_to_string(path) {
            Ok(text) => return Ok((text, path.clone())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::Io(path.clone(), e)),
        }
    }
    Err(Error::NoTemplate(candidates))
}

/// Builds the briefing text.
pub fn build(
    mode: Mode,
    template_text: &str,
    project: &Project,
    notes: &[Note],
    opts: &Options,
) -> String {
    let section = project::render(project);
    let mut out = project::fill(template_text, &section);

    if mode == Mode::Resume {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&resume_section(notes, opts.budget));
    }
    out
}

/// Renders the "what the last session learned" block.
///
/// Same ordering and sanitising as `prime` - gotchas first, then open
/// questions, then decisions newest first, under the same byte budget - but
/// without the freshness stamp: in a resume prompt there is no repository
/// state to report, only notes.
fn resume_section(notes: &[Note], budget: usize) -> String {
    let mut out = String::from("## What previous sessions learned\n");

    if notes.is_empty() {
        out.push_str("\nNo notes yet. Notes are recorded with `tcx note`.\n");
        return out;
    }

    out.push('\n');
    let ordered = crate::prime::ordered(notes);
    let mut used = 0usize;
    let mut omitted = 0usize;
    for note in ordered {
        let line = format!("{}\n", crate::prime::sanitize_line(&note.render()));
        if used + line.len() > budget {
            omitted += 1;
            continue;
        }
        used += line.len();
        out.push_str(&line);
    }
    if omitted > 0 {
        out.push_str(&format!("… {omitted} more (tcx find)\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::Kind;

    fn sample_project() -> Project {
        Project {
            name: "demo".to_owned(),
            description: "Rust".to_owned(),
            package: Some("demo".to_owned()),
            source: Some("src/".to_owned()),
            build: Some("Cargo".to_owned()),
            modules: Vec::new(),
            kind: project::Kind::Code,
        }
    }

    #[test]
    fn fresh_has_project_and_no_notes() {
        let out = build(
            Mode::Fresh,
            "intro\n\n## Project\nold\n",
            &sample_project(),
            &[],
            &Options::default(),
        );
        assert!(out.contains("## Project\ndemo: Rust\nPackage: demo\nSource: src/"));
        assert!(out.contains("type: code"));
        assert!(!out.contains("previous sessions"));
    }

    #[test]
    fn resume_appends_notes() {
        let note = Note::new(Kind::Gotcha, "x must run before y");
        let out = build(
            Mode::Resume,
            "## Project\n",
            &sample_project(),
            &[note],
            &Options::default(),
        );
        assert!(out.contains("## What previous sessions learned"));
        assert!(out.contains("x must run before y"));
    }

    #[test]
    fn resume_with_no_notes_says_so() {
        let out = build(
            Mode::Resume,
            "## Project\n",
            &sample_project(),
            &[],
            &Options::default(),
        );
        assert!(out.contains("No notes yet"));
    }
}
