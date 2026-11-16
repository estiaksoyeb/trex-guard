//! `tcx map` - a shallow tree of the repository, annotated with judgment.
//!
//! # Why the structure is generated, not written
//!
//! The workflow prompt's rule 18 forbids persisting things `rg` can rebuild,
//! because a stored tree rots the moment a file moves. So the split is:
//!
//! | Part | Source | Can go stale |
//! | --- | --- | --- |
//! | directories and files | walked live from disk | no |
//! | one-line judgment per directory | notes with a `dir:` field | yes, but it is dated |
//!
//! A note attached to a directory that no longer exists is still printed, under
//! a "no longer in the tree" heading, rather than silently dropped: it is the
//! one thing here that cannot be regenerated.
//!
//! # Depth
//!
//! `--depth` counts directory levels. Depth 1 is the root's children. The
//! default is deliberately shallow, because the purpose is orientation, not a
//! file listing; the AI asks for more depth only once it knows where to look.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::notes::Note;

/// Directories that never belong in a map: they are generated, vendored or
/// enormous, and listing them teaches nothing about the code.
const SKIP: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "build",
    "dist",
    "out",
    ".gradle",
    ".idea",
    ".vscode",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".next",
    ".nuxt",
    "coverage",
    ".cxx",
    "tmp",
    // RCS/CVS backup directories: version-control internals, never code.
    "RCS",
    "CVS",
];

/// Hidden directories are skipped, with these exceptions: they hold real
/// project configuration an AI needs to know exists.
const KEEP_HIDDEN: &[&str] = &[".github", ".gitlab-ci.yml", ".tg"];

/// A directory whose entries are all data files (schemas, fixtures, generated
/// migrations) is summarised by file count instead of listed. Beyond a few
/// levels, a migration dump teaches nothing about where code lives.
const DATA_EXTS: &[&str] = &["json", "jsonl", "xml", "csv", "tsv", "yaml", "yml", "proto"];

/// Above this many entries, a directory of pure data files is collapsed.
const COLLAPSE_ABOVE: usize = 8;

/// Options for building a map.
#[derive(Debug, Clone)]
pub struct Options {
    /// Directory levels to show, counting from the root's children (1).
    pub depth: usize,
    /// Byte cap for the judgment lines.
    pub budget: usize,
    /// Only show this subtree, relative to the root.
    pub under: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            depth: 2,
            budget: 4000,
            under: None,
        }
    }
}

/// One node in the generated tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Directory or file name.
    pub name: String,
    /// Whether this is a directory.
    pub is_dir: bool,
    /// Path relative to the root, which is what notes match against.
    pub path: String,
    /// Children, sorted: directories first, then files, both alphabetical.
    pub children: Vec<Node>,
}

/// Walks `root` to `depth` levels, returning the tree.
pub fn tree(root: &Path, depth: usize) -> Node {
    walk(root, root, depth, 0)
}

fn walk(root: &Path, dir: &Path, max_depth: usize, level: usize) -> Node {
    let rel = dir
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let name = if rel.is_empty() {
        root.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_owned())
    } else {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    let mut children = Vec::new();
    if level < max_depth {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let child_name = entry.file_name().to_string_lossy().to_string();
                let is_dir = path.is_dir();
                if SKIP.contains(&child_name.as_str()) {
                    continue;
                }
                // Hidden entries stay out, except the few that are real
                // project configuration.
                if child_name.starts_with('.') && !KEEP_HIDDEN.contains(&child_name.as_str()) {
                    continue;
                }
                let child_rel = if rel.is_empty() {
                    child_name.clone()
                } else {
                    format!("{rel}/{child_name}")
                };
                if is_dir {
                    children.push(walk(root, &path, max_depth, level + 1));
                } else {
                    children.push(Node {
                        name: child_name,
                        is_dir: false,
                        path: child_rel,
                        children: Vec::new(),
                    });
                }
            }
        }
    }

    collapse_data_dirs(&mut children);

    children.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Node {
        name,
        is_dir: true,
        path: rel,
        children,
    }
}

/// Replaces a directory of pure data files with a single count line.
///
/// `schemas/` holding 40 numbered JSON migrations is real, but listing all 40
/// at every depth buries the directories that carry the code.
fn collapse_data_dirs(children: &mut Vec<Node>) {
    for child in children.iter_mut() {
        if !child.is_dir || child.children.is_empty() {
            continue;
        }
        let all_data = child.children.iter().all(|c| {
            !c.is_dir
                && Path::new(&c.name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| DATA_EXTS.contains(&e))
                    .unwrap_or(false)
        });
        if all_data && child.children.len() > COLLAPSE_ABOVE {
            let count = child.children.len();
            child.children = vec![Node {
                name: format!("({count} data files)"),
                is_dir: false,
                path: format!("{}/", child.path),
                children: Vec::new(),
            }];
        }
    }
}

/// Notes that describe a directory, keyed by path.
fn judgement(notes: &[Note]) -> BTreeMap<String, Vec<&Note>> {
    let mut out: BTreeMap<String, Vec<&Note>> = BTreeMap::new();
    for note in notes {
        if let Some(dir) = &note.dir {
            let dir = normalise_dir(dir);
            out.entry(dir).or_default().push(note);
        }
    }
    out
}

/// Normalises `dir:` values so `./src`, `src/` and `src` all match `src`.
fn normalise_dir(dir: &str) -> String {
    let dir = dir.trim().trim_start_matches("./");
    let dir = dir.trim_end_matches('/');
    if dir == "." {
        String::new()
    } else {
        dir.to_owned()
    }
}

/// Renders the map.
///
/// Deterministic for a given tree and note set, so it is golden-file testable.
pub fn render(root: &Path, notes: &[Note], opts: &Options) -> String {
    let annotated = judgement(notes);
    let target = opts
        .under
        .as_deref()
        .map(normalise_dir)
        .unwrap_or_default();

    // `--under` names a subtree because the caller already knows where to
    // look, so depth is measured from that node instead of being capped by the
    // distance to it. A deep path must not be unreachable just because the
    // walk started at the root.
    let (walk_root, prefix) = if target.is_empty() {
        (root.to_path_buf(), String::new())
    } else {
        let dir = root.join(&target);
        if !dir.is_dir() {
            return format!("tcx map: `{target}` is not a directory in this project\n");
        }
        (dir, target.clone())
    };

    let subtree = tree(&walk_root, opts.depth.max(1));
    let shown = rebase(&subtree, &prefix);
    // The orphan check needs the whole tree, not just the subtree.
    let full = if target.is_empty() {
        tree(&walk_root, opts.depth.max(1))
    } else {
        tree(root, 64)
    };

    let mut out = String::new();
    out.push_str("# map\n");
    out.push_str(&format!("depth {} tcx\n\n", opts.depth));
    render_node(&shown, 0, &annotated, opts, &mut out, true);

    // Notes attached to directories that are no longer in the tree. They are
    // the only thing here that cannot be regenerated, so they are surfaced
    // rather than dropped.
    let present: Vec<&str> = subtree_paths(&full);
    let mut orphans: Vec<(&String, &Vec<&Note>)> = annotated
        .iter()
        .filter(|(path, _)| !path.is_empty() && !present.contains(&path.as_str()))
        .collect();
    orphans.sort_by(|a, b| a.0.cmp(b.0));
    if !orphans.is_empty() {
        out.push_str("\n## no longer in the tree\n");
        for (path, list) in orphans {
            for note in list {
                out.push_str(&format!("{}: {}\n", path, truncate(&note.text, 200)));
            }
        }
    }

    out
}

fn render_node(
    node: &Node,
    level: usize,
    annotated: &BTreeMap<String, Vec<&Note>>,
    opts: &Options,
    out: &mut String,
    topmost: bool,
) {
    let indent = "  ".repeat(level);
    if !topmost || node.path.is_empty() {
        if !node.path.is_empty() {
            let marker = if node.is_dir { "/" } else { "" };
            out.push_str(&format!("{indent}{}{marker}", node.name));
            if let Some(list) = annotated.get(&node.path) {
                // First judgment line rides on the directory line; the rest
                // follow indented, so a directory never breaks the tree shape.
                if let Some(first) = list.first() {
                    out.push_str("  - ");
                    out.push_str(&truncate(&first.text, 160));
                }
            }
            out.push('\n');
        }
    }

    if node.is_dir {
        let extra: Vec<&&Note> = annotated
            .get(&node.path)
            .map(|l| l.iter().skip(1).collect())
            .unwrap_or_default();
        for note in extra {
            out.push_str(&format!(
                "{indent}    {}: {}\n",
                note.date,
                truncate(&note.text, 160)
            ));
        }
    }

    for child in &node.children {
        render_node(child, level + 1, annotated, opts, out, false);
    }

    // Files are not annotated individually; only directories carry judgment.
    let _ = opts;
}

/// Rewrites a subtree's paths to include the prefix it was walked from.
fn rebase(node: &Node, prefix: &str) -> Node {
    let path = if prefix.is_empty() {
        node.path.clone()
    } else if node.path.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}/{}", node.path)
    };
    Node {
        name: node.name.clone(),
        is_dir: node.is_dir,
        path,
        children: node.children.iter().map(|c| rebase(c, prefix)).collect(),
    }
}

fn subtree_paths(node: &Node) -> Vec<&str> {
    let mut out = vec![node.path.as_str()];
    for child in &node.children {
        out.extend(subtree_paths(child));
    }
    out
}

/// Caps a line and strips control characters: the map is pasted into a chat.
fn truncate(text: &str, max: usize) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim();
    if clean.chars().count() <= max {
        return clean.to_owned();
    }
    let cut: String = clean.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_dir_accepts_the_obvious_spellings() {
        assert_eq!(normalise_dir("./src/"), "src");
        assert_eq!(normalise_dir("src"), "src");
        assert_eq!(normalise_dir("."), "");
        assert_eq!(normalise_dir("a/b/"), "a/b");
    }

    #[test]
    fn truncate_strips_control_characters() {
        assert_eq!(truncate("a\u{1b}[2Jb", 100), "a [2Jb");
    }

    #[test]
    fn truncate_caps_length() {
        let long = "x".repeat(300);
        let cut = truncate(&long, 10);
        assert_eq!(cut.chars().count(), 11);
        assert!(cut.ends_with('…'));
    }
}
