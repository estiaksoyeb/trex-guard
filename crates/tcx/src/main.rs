//! `tcx` - TREX ConteXt.
//!
//! Command-line entry point. All logic lives in the library so it can be tested
//! without spawning a process.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tcx::notes::{Kind, Note, Notes};
use tcx::{briefing, help, id, project, store};

/// Exit codes are part of the interface; scripts branch on them.
const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print!("{}", help::overall());
        return ExitCode::from(EXIT_USAGE);
    };

    // `tcx <command> --help` answers before the command parses anything, so a
    // flag that is valid elsewhere cannot be mistaken for an error.
    if help::wants_help(&args[1..]) && help::for_command(command).is_some() {
        print!("{}", help::for_command(command).unwrap());
        return ExitCode::from(EXIT_OK);
    }

    match command {
        // `help` is accepted as the first word only; elsewhere it is data.
        "--help" | "-h" | "help" => {
            print!("{}", help::overall());
            ExitCode::from(EXIT_OK)
        }
        "--version" | "-V" | "version" => {
            println!("tcx {}", env!("CARGO_PKG_VERSION"));
            ExitCode::from(EXIT_OK)
        }
        "id" => run_id(&args[1..]),
        "note" => run_note(&args[1..]),
        "prime" => run_prime(&args[1..]),
        "find" => run_find(&args[1..]),
        "import" => run_import(&args[1..]),
        "fresh" => run_fresh(&args[1..]),
        "resume" => run_resume(&args[1..]),
        "map" => run_map(&args[1..]),
        other => {
            eprintln!("tcx: unknown command `{other}`");
            eprintln!("run `tcx --help` for usage");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Everything a command needs to know about the current project.
struct Project {
    identity: id::Identity,
    store: store::Store,
    /// The store id actually in use, which may be an adopted or pre-existing
    /// root rather than `identity.id`.
    store_id: String,
    adopted: Option<(String, String)>,
    warnings: Vec<String>,
}

impl Project {
    /// Resolves the project, adopting a provisional store if needed.
    fn resolve(path: &str) -> Result<Project, String> {
        let identity = id::resolve(path).map_err(|e| e.to_string())?;
        let store = store::Store::from_env().map_err(|e| e.to_string())?;

        let mut adopted = None;
        let mut warnings = Vec::new();

        // Adopt a provisional store if this repo has outgrown it (first commit,
        // or an unshallowed clone). This keeps notes from orphaning.
        if !identity.is_provisional() {
            if let Some(provisional) = id::provisional_id_for(&identity) {
                match id::plan_migration(&identity, Some(&provisional), |name| {
                    store.has_project(name)
                }) {
                    id::Migration::Rename { from, to } => match store.rename_project(&from, &to) {
                        Ok(()) => adopted = Some((from, to)),
                        Err(e) => warnings.push(format!("could not adopt store {from}: {e}")),
                    },
                    id::Migration::Conflict { from, to } => warnings.push(format!(
                        "both {from} and {to} exist; keeping both (merge is manual)"
                    )),
                    id::Migration::None => {}
                }
            }
        }

        // For a repo with several roots, prefer whichever already owns a store
        // so identity does not flip with the checked out branch.
        let store_id = match identity.kind {
            id::Kind::Git if identity.roots.len() > 1 => {
                match id::choose_root(&identity, |r| store.has_project(r)) {
                    (id::Choice::Existing(id), extra) => {
                        if !extra.is_empty() {
                            warnings.push(format!(
                                "{} roots already have stores; using {}",
                                extra.len(),
                                id
                            ));
                        }
                        id.clone()
                    }
                    (id::Choice::New(id), _) => id.clone(),
                }
            }
            _ => identity.id.clone(),
        };

        Ok(Project {
            identity,
            store,
            store_id,
            adopted,
            warnings,
        })
    }

    fn notes(&self) -> Notes {
        Notes::at(self.store.project_data(&self.store_id).join("notes.md"))
    }

    fn print_adoption(&self) {
        if let Some((from, to)) = &self.adopted {
            eprintln!("tcx: adopted provisional store {from} -> {to}");
        }
        for warning in &self.warnings {
            eprintln!("tcx: warning: {warning}");
        }
    }
}

fn run_id(args: &[String]) -> ExitCode {
    let mut path = ".".to_string();
    let mut human = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--human" => human = true,
            "--" => {
                if let Some(p) = iter.next() {
                    path = p.clone();
                }
            }
            other if other.starts_with('-') => {
                eprintln!("tcx id: unknown option `{other}`");
                return ExitCode::from(EXIT_USAGE);
            }
            other => path = other.to_string(),
        }
    }

    let project = match Project::resolve(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tcx id: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    project.print_adoption();

    let identity = &project.identity;
    let store_id = &project.store_id;

    if human {
        println!("id:          {store_id}");
        println!("kind:        {}", identity.kind_label());
        println!("root:        {}", identity.root.display());
        if let Some(common) = &identity.common_dir {
            println!("common-dir:  {}", common.display());
        }
        println!("roots:       {}", identity.roots.len());
        for root in &identity.roots {
            println!("  {root}");
        }
        println!("store:       {}", project.store.project_data(store_id).display());
        println!("provisional: {}", identity.is_provisional());
        if project.adopted.is_some() {
            println!("adopted:     true");
        }
    } else {
        // Machine-readable single line: opaque id first so it can be captured
        // with `tcx id | cut -d' ' -f1`.
        println!(
            "{} {}\t{}",
            store_id,
            identity.kind_label(),
            identity.root.display()
        );
    }

    ExitCode::from(EXIT_OK)
}

fn run_note(args: &[String]) -> ExitCode {
    let mut path = ".".to_string();
    let mut kind = Kind::Decision;
    let mut files: Vec<String> = Vec::new();
    let mut keywords: Vec<String> = Vec::new();
    let mut dir: Option<String> = None;
    let mut text_parts: Vec<String> = Vec::new();
    let mut only_notes = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--kind" | "-k" => match iter.next() {
                Some(k) => kind = Kind::parse(k.trim()),
                None => {
                    eprintln!("tcx note: --kind needs a value (decision|gotcha|open)");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--file" | "-f" => match iter.next() {
                Some(f) => files.push(f.clone()),
                None => {
                    eprintln!("tcx note: --file needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--tag" | "-t" => match iter.next() {
                Some(t) => keywords.push(t.clone()),
                None => {
                    eprintln!("tcx note: --tag needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--dir" | "-d" => match iter.next() {
                Some(d) => dir = Some(d.clone()),
                None => {
                    eprintln!("tcx note: --dir needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--path" | "-p" => match iter.next() {
                Some(p) => path = p.clone(),
                None => {
                    eprintln!("tcx note: --path needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--list" => only_notes = true,
            "--" => {
                text_parts.extend(iter.by_ref().cloned());
            }
            other if other.starts_with('-') => {
                eprintln!("tcx note: unknown option `{other}`");
                return ExitCode::from(EXIT_USAGE);
            }
            other => text_parts.push(other.to_string()),
        }
    }

    // A trailing `.` or a path is almost always a slipped positional argument
    // from the habits the other commands teach. `note` has no positional PATH,
    // so it would quietly become part of the note text. Catch it instead.
    if let Some(last) = text_parts.last() {
        // Only flag what actually resolves: a note may legitimately mention
        // `foo/bar`. `.` and `..` are flagged regardless because they are
        // never prose.
        let looks_like_path = matches!(last.as_str(), "." | "..")
            || (last.contains('/') && Path::new(last).exists());
        if looks_like_path {
            eprintln!("tcx note: `{last}` looks like a path, but note takes no positional path");
            eprintln!("tcx note: use --path {last} to choose the project");
            return ExitCode::from(EXIT_USAGE);
        }
    }

    let project = match Project::resolve(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tcx note: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    project.print_adoption();

    let notes = project.notes();

    if only_notes {
        return match notes.read_all() {
            Ok(all) => {
                for note in all {
                    println!("{}", note.render());
                }
                ExitCode::from(EXIT_OK)
            }
            Err(e) => {
                eprintln!("tcx note: {e}");
                ExitCode::from(EXIT_ERROR)
            }
        };
    }

    let text = text_parts.join(" ");
    if text.trim().is_empty() {
        eprintln!("tcx note: nothing to record");
        eprintln!("usage: tcx note [-k decision|gotcha|open] [-f FILE]... [-t TAG]... TEXT");
        return ExitCode::from(EXIT_USAGE);
    }

    let mut note = Note::new(kind, text).with_files(files).with_keywords(keywords);
    if let Some(dir) = dir {
        note = note.with_dir(dir);
    }
    match notes.append(&note) {
        Ok(()) => {
            println!("{}", note.render());
            ExitCode::from(EXIT_OK)
        }
        Err(e) => {
            eprintln!("tcx note: {e}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run_prime(args: &[String]) -> ExitCode {
    let mut path = ".".to_string();
    let mut budget = tcx::prime::Options::default().budget;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--budget" | "-b" => match iter.next() {
                Some(b) => match b.parse::<usize>() {
                    Ok(n) if n > 0 => budget = n,
                    _ => {
                        eprintln!("tcx prime: --budget must be a positive number of bytes");
                        return ExitCode::from(EXIT_USAGE);
                    }
                },
                None => {
                    eprintln!("tcx prime: --budget needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--path" | "-p" => match iter.next() {
                Some(p) => path = p.clone(),
                None => {
                    eprintln!("tcx prime: --path needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("tcx prime: unknown option `{other}`");
                return ExitCode::from(EXIT_USAGE);
            }
            other => path = other.to_string(),
        }
    }

    let project = match Project::resolve(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tcx prime: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    project.print_adoption();

    let notes = match project.notes().read_all() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("tcx prime: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };

    let state = id::head_state(&project.identity);
    let stamp = tcx::prime::Stamp {
        head: state.as_ref().and_then(|s| s.head.clone()),
        branch: state
            .as_ref()
            .map(|s| s.branch.clone())
            .unwrap_or_else(|| "no-git".to_owned()),
        dirty: state.as_ref().map(|s| s.dirty).unwrap_or(0),
        shallow: project.identity.kind == id::Kind::GitShallow,
        kind: project.identity.kind_label().to_owned(),
    };

    let text = tcx::prime::build(&stamp, &notes, &tcx::prime::Options { budget });
    print!("{text}");
    ExitCode::from(EXIT_OK)
}

fn run_find(args: &[String]) -> ExitCode {
    let mut path = ".".to_string();
    let mut list_tags = false;
    let mut keywords: Vec<String> = Vec::new();
    let mut query_parts: Vec<String> = Vec::new();

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--tags" => list_tags = true,
            // Repeatable, so `tcx find -k identity -k storage` narrows to
            // either keyword.
            "--kind" | "-k" => match iter.next() {
                Some(k) => keywords.push(k.clone()),
                None => {
                    eprintln!("tcx find: --kind needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--path" | "-p" => match iter.next() {
                Some(p) => path = p.clone(),
                None => {
                    eprintln!("tcx find: --path needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--" => query_parts.extend(iter.by_ref().cloned()),
            other if other.starts_with('-') => {
                eprintln!("tcx find: unknown option `{other}`");
                return ExitCode::from(EXIT_USAGE);
            }
            other => query_parts.push(other.to_string()),
        }
    }

    let project = match Project::resolve(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tcx find: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    project.print_adoption();

    let notes = match project.notes().read_all() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("tcx find: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };

    if list_tags {
        for (tag, count) in tcx::find::keywords(&notes) {
            println!("{count:>4} {tag}");
        }
        return ExitCode::from(EXIT_OK);
    }

    // Keyword filter first, then the free-text query on what survived.
    let scoped = tcx::find::matching_keywords(&notes, &keywords);
    let query = query_parts.join(" ");
    for note in scoped {
        if query.is_empty() || tcx::find::find(std::slice::from_ref(note), &query).len() == 1 {
            println!("{}", note.render());
        }
    }
    ExitCode::from(EXIT_OK)
}

fn run_import(args: &[String]) -> ExitCode {
    let mut path = ".".to_string();
    let mut dir: Option<String> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dir" | "-d" => match iter.next() {
                Some(d) => dir = Some(d.clone()),
                None => {
                    eprintln!("tcx import: --dir needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--path" | "-p" => match iter.next() {
                Some(p) => path = p.clone(),
                None => {
                    eprintln!("tcx import: --path needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("tcx import: unknown option `{other}`");
                return ExitCode::from(EXIT_USAGE);
            }
            other => dir = Some(other.to_string()),
        }
    }

    let project = match Project::resolve(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tcx import: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    project.print_adoption();

    // Default to `<project root>/.ai`.
    let source = match dir {
        Some(d) => PathBuf::from(d),
        None => project.identity.root.join(".ai"),
    };

    if !source.is_dir() {
        eprintln!("tcx import: {} is not a directory", source.display());
        return ExitCode::from(EXIT_ERROR);
    }

    match tcx::import::import_dir(&source, &project.notes()) {
        Ok(report) => {
            println!(
                "imported {} ({} duplicates skipped, {} lines ignored)",
                report.added, report.duplicates, report.skipped
            );
            for missing in &report.missing {
                eprintln!("tcx import: {missing} not found");
            }
            ExitCode::from(EXIT_OK)
        }
        Err(e) => {
            eprintln!("tcx import: {e}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run_map(args: &[String]) -> ExitCode {
    let mut path = ".".to_string();
    let mut opts = tcx::map::Options::default();

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--depth" | "-d" => match iter.next().and_then(|d| d.parse::<usize>().ok()) {
                Some(n) if n > 0 => opts.depth = n,
                _ => {
                    eprintln!("tcx map: --depth must be a positive number");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--under" | "-u" => match iter.next() {
                Some(u) => opts.under = Some(u.clone()),
                None => {
                    eprintln!("tcx map: --under needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--path" | "-p" => match iter.next() {
                Some(p) => path = p.clone(),
                None => {
                    eprintln!("tcx map: --path needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("tcx map: unknown option `{other}`");
                return ExitCode::from(EXIT_USAGE);
            }
            other => path = other.to_string(),
        }
    }

    let project = match Project::resolve(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tcx map: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    project.print_adoption();

    let notes = match project.notes().read_all() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("tcx map: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };

    match tcx::map::render(&project.identity.root, &notes, &opts) {
        Ok(text) => {
            print!("{text}");
            ExitCode::from(EXIT_OK)
        }
        Err(e) => {
            // stderr and exit 1: a missing subtree must not look like a
            // successful, empty map to anything reading the output.
            eprintln!("tcx map: {e}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run_fresh(args: &[String]) -> ExitCode {
    run_briefing(briefing::Mode::Fresh, args)
}

fn run_resume(args: &[String]) -> ExitCode {
    run_briefing(briefing::Mode::Resume, args)
}

/// Shared implementation of `fresh` and `resume`.
///
/// Both are run once per chat and the output is pasted. They differ only in
/// whether the previous session's notes are appended.
fn run_briefing(mode: briefing::Mode, args: &[String]) -> ExitCode {
    let command = match mode {
        briefing::Mode::Fresh => "fresh",
        briefing::Mode::Resume => "resume",
    };

    let mut path = ".".to_string();
    let mut budget = briefing::Options::default().budget;
    let mut template: Option<PathBuf> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--budget" | "-b" => match iter.next().and_then(|b| b.parse::<usize>().ok()) {
                // `fresh` prints no notes, so a budget would silently do
                // nothing. Accepting a flag that cannot act is worse than
                // rejecting it: it looks like it worked.
                Some(_) if mode == briefing::Mode::Fresh => {
                    eprintln!("tcx fresh: --budget applies to the notes, which fresh does not print; use resume");
                    return ExitCode::from(EXIT_USAGE);
                }
                Some(n) if n > 0 => budget = n,
                _ => {
                    eprintln!("tcx {command}: --budget must be a positive number of bytes");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--template" | "-T" => match iter.next() {
                Some(t) => template = Some(PathBuf::from(t)),
                None => {
                    eprintln!("tcx {command}: --template needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            "--path" | "-p" => match iter.next() {
                Some(p) => path = p.clone(),
                None => {
                    eprintln!("tcx {command}: --path needs a value");
                    return ExitCode::from(EXIT_USAGE);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("tcx {command}: unknown option `{other}`");
                return ExitCode::from(EXIT_USAGE);
            }
            other => path = other.to_string(),
        }
    }

    let project = match Project::resolve(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tcx {command}: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    project.print_adoption();

    let (template_text, template_path) =
        match briefing::template(&project.store, template.as_deref()) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("tcx {command}: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };

    let detected = project::detect(&project.identity.root);

    let notes = if mode == briefing::Mode::Resume {
        match project.notes().read_all() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("tcx {command}: {e}");
                return ExitCode::from(EXIT_ERROR);
            }
        }
    } else {
        Vec::new()
    };

    let opts = briefing::Options { budget, template: None };
    let text = briefing::build(mode, &template_text, &detected, &notes, &opts);

    // stdout is the payload so it can be piped to `clip`; where the rules came
    // from goes to stderr, where it cannot pollute the paste.
    eprintln!("tcx {command}: template {}", template_path.display());
    print!("{text}");
    ExitCode::from(EXIT_OK)
}

// Keeps `PathBuf` in scope for future commands that need it.
#[allow(dead_code)]
fn _unused(_: PathBuf) {}
