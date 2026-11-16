//! Project detection - fills the `## Project` section of the prompt.
//!
//! `tg fresh` prints the user's workflow prompt with the `## Project` block
//! filled in from the repository the user is standing in. Everything here only
//! *reads* the tree: no build system is invoked, because the output is copied
//! into a chat and must be produced in milliseconds.
//!
//! # Why files, not commands
//!
//! Deriving the block by running Gradle or Cargo would be accurate and slow,
//! and would execute repository code on behalf of a tool whose user only asked
//! for a text block. Reading manifests is enough for the fields the prompt
//! needs, and it is deterministic, so it is golden-file testable.
//!
//! # Multi-module repositories
//!
//! A Kotlin repo lists `:app` and `:core` in `settings.gradle`, but only one
//! module is usually the one the user works in. The rule used here: pick the
//! module that owns the package the user most recently touched, preferring
//! source directories that a shell is most likely to be inside. When that is
//! ambiguous the largest module wins, and the others are listed so the model
//! can follow them.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The filled `## Project` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Display name, e.g. `mpvRex`.
    pub name: String,
    /// Human description assembled from the manifest, e.g.
    /// `Android app, Kotlin + Jetpack Compose`.
    pub description: String,
    /// Application id or crate name, when one is declared.
    pub package: Option<String>,
    /// Source directory relative to the repository root, with a trailing `/`.
    pub source: Option<String>,
    /// Build system, e.g. `Gradle (Kotlin DSL)`.
    pub build: Option<String>,
    /// Other modules in a multi-module build, relative to the root.
    pub modules: Vec<String>,
    /// `code`, `docs` or `config`.
    pub kind: Kind,
}

/// What the rules in the prompt apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Code,
    Docs,
    Config,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Code => "code",
            Kind::Docs => "docs",
            Kind::Config => "config",
        }
    }
}

/// Renders the section for the `## Project` heading.
pub fn render(project: &Project) -> String {
    let mut out = String::new();
    out.push_str(&sanitize(&project.name));
    out.push_str(": ");
    out.push_str(&sanitize(&project.description));
    out.push('\n');
    if let Some(package) = &project.package {
        out.push_str("Package: ");
        out.push_str(&sanitize(package));
        out.push('\n');
    }
    if let Some(source) = &project.source {
        out.push_str("Source: ");
        out.push_str(&sanitize(source));
        out.push('\n');
    }
    if let Some(build) = &project.build {
        out.push_str("Build: ");
        out.push_str(&sanitize(build));
        out.push('\n');
    }
    if !project.modules.is_empty() {
        out.push_str("Modules: ");
        let modules: Vec<String> = project.modules.iter().map(|m| sanitize(m)).collect();
        out.push_str(&modules.join(", "));
        out.push('\n');
    }
    out.push_str("type: ");
    out.push_str(project.kind.as_str());
    out
}

/// Builds the whole prompt by substituting the `## Project` section.
///
/// The template holds a `## Project` heading and a `type:` line under it;
/// everything from the heading to the next `## ` heading (or the end of file)
/// is replaced. A template without the heading gets the section appended, so a
/// trimmed prompt still works.
pub fn fill(template: &str, section: &str) -> String {
    const HEADING: &str = "## Project";
    let block = format!("{HEADING}\n{section}\n");

    let Some(start) = find_heading(template) else {
        let mut out = template.to_owned();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&block);
        return out;
    };

    // Replace the heading line and everything under it up to the next heading,
    // so a section already filled by a previous run cannot leave a duplicate
    // behind. Only a real heading counts: a `## Project` mentioned in prose is
    // not one.
    let after_heading = template[start..]
        .find('\n')
        .map(|nl| start + nl + 1)
        .unwrap_or(template.len());
    let end = template[after_heading..]
        .find("\n## ")
        .map(|offset| after_heading + offset + 1)
        .unwrap_or(template.len());

    let mut out = String::with_capacity(template.len() + section.len());
    out.push_str(&template[..start]);
    out.push_str(&block);
    if end < template.len() {
        out.push('\n');
        out.push_str(&template[end..]);
    }
    out
}

/// Byte offset of the `## Project` heading line, if the template has one.
///
/// A heading is a line that, once trimmed, is exactly `## Project`, so
/// `## Projects` or a sentence containing the words do not match.
fn find_heading(template: &str) -> Option<usize> {
    const HEADING: &str = "## Project";
    let mut offset = 0usize;
    for line in template.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if content.trim() == HEADING {
            return Some(offset + (content.len() - content.trim_start().len()));
        }
        offset += line.len();
    }
    None
}

/// Detects the project rooted at `root`.
pub fn detect(root: &Path) -> Project {
    let name = detect_name(root);
    let kind = classify(root);

    let mut project = if has(root, "Cargo.toml") {
        detect_cargo(root, name, kind)
    } else if has(root, "build.gradle.kts") || has(root, "build.gradle") {
        detect_gradle(root, name, kind)
    } else if has(root, "settings.gradle.kts") || has(root, "settings.gradle") {
        detect_gradle(root, name, kind)
    } else if has(root, "CMakeLists.txt") {
        detect_cmake(root, name, kind)
    } else if has(root, "package.json") {
        detect_node(root, name, kind)
    } else if has(root, "pyproject.toml") || has(root, "setup.py") {
        detect_simple(root, name, kind, "Python", "py")
    } else if has(root, "go.mod") {
        detect_simple(root, name, kind, "Go", "go")
    } else {
        detect_simple(root, name, kind, &language_for(root, "code"), "")
    };

    if project.description.is_empty() {
        project.description = kind_fallback(kind);
    }
    project
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

fn detect_cargo(root: &Path, name: String, kind: Kind) -> Project {
    let manifest = read(root.join("Cargo.toml"));
    let (package, description, _edition, mut crates) = parse_workspace(&manifest);
    if crates.is_empty() {
        crates = parse_members_multiline(&manifest);
    }
    crates.retain(|c| root.join(c).is_dir());
    let is_workspace = !crates.is_empty();
    let description = match (&package, is_workspace) {
        (_, true) if crates.len() > 1 => format!("Rust workspace ({} crates)", crates.len()),
        (Some(p), _) => match description.as_deref() {
            Some(d) => format!("Rust — {d} ({p})"),
            None => format!("Rust — {p}"),
        },
        (None, _) => "Rust".to_owned(),
    };

    // A workspace root has no `src/`; the crate the shell is in, else the
    // first listed, is the one to name.
    let crate_dir = if is_workspace {
        pick_module(root, &crates)
    } else {
        None
    };
    let source = match &crate_dir {
        Some(dir) if root.join(dir).join("src").is_dir() => Some(format!("{dir}/src/")),
        _ if root.join("src").is_dir() => Some("src/".to_owned()),
        _ => None,
    };

    Project {
        name,
        description,
        package: package.filter(|_| !is_workspace),
        source,
        build: Some("Cargo".to_owned()),
        modules: if is_workspace { crates } else { Vec::new() },
        kind,
    }
}

fn detect_gradle(root: &Path, name: String, kind: Kind) -> Project {
    let kts = has(root, "build.gradle.kts") || has(root, "settings.gradle.kts");
    let build = if kts {
        "Gradle (Kotlin DSL)"
    } else {
        "Gradle (Groovy)"
    };

    let mut modules = settings_modules(root);
    if modules.is_empty() {
        modules = ["main"].into_iter().map(str::to_owned).collect();
    }
    let module = pick_module(root, &modules);
    let module_dir = module
        .as_ref()
        .map(|m| root.join(m.trim_start_matches(':').replace(':', "/")))
        .unwrap_or_else(|| root.to_path_buf());

    let app = read(module_dir.join("build.gradle.kts"));
    let app = if app.is_empty() {
        read(module_dir.join("build.gradle"))
    } else {
        app
    };
    let versions = read_any(root, &["gradle/libs.versions.toml"]);

    // `namespace` declares the code identity; `applicationId` may carry build
    // type suffixes, so it is only the fallback.
    let package = first_of(&app, &["namespace", "applicationId"])
        .or_else(|| detect_manifest_package(root, &module_dir));
    let android = has(root, "app/src/main/AndroidManifest.xml")
        || (module.is_some() && has(&module_dir, "src/main/AndroidManifest.xml"));
    let compose = app.contains("kotlin.compose")
        || app.contains("kotlin(\"plugin.compose\")")
        || app.contains("compose = true")
        || versions.contains("compose");

    let description = match (android, compose) {
        (true, true) => "Android app, Kotlin + Jetpack Compose".to_owned(),
        (true, false) => "Android app, Kotlin".to_owned(),
        (false, true) => "Kotlin, Jetpack Compose".to_owned(),
        (false, false) => "Kotlin / Gradle".to_owned(),
    };

    // Lead with the module the source path sits in, so the Project block names
    // the directory the user actually works in.
    if let Some(chosen) = &module {
        modules.sort_by_key(|m| m != chosen);
    }
    if modules.as_slice() == ["main"] {
        modules.clear();
    }

    let source = source_path(root, &module_dir, package.as_deref()).map(|s| relative(root, &s));
    Project {
        name,
        description,
        package,
        source,
        build: Some(build.to_owned()),
        modules,
        kind,
    }
}

fn detect_cmake(root: &Path, name: String, kind: Kind) -> Project {
    let cmake = read(root.join("CMakeLists.txt"));
    let cpp = language_for(root, "code");
    Project {
        name,
        description: if cpp.is_empty() {
            "C / C++ via CMake".to_owned()
        } else {
            format!("{cpp} via CMake")
        },
        package: None,
        source: dirs_with_ext(root, &["cpp", "cc", "cxx", "c"])
            .first()
            .map(|d| format!("{d}/")),
        build: Some("CMake".to_owned()),
        modules: if cmake.contains("add_subdirectory") {
            vec!["(subdirectories in CMakeLists.txt)".to_owned()]
        } else {
            Vec::new()
        },
        kind,
    }
}

fn detect_node(root: &Path, name: String, kind: Kind) -> Project {
    let manifest = read(root.join("package.json"));
    let description = if manifest.contains("\"typescript\"") {
        "TypeScript".to_owned()
    } else {
        "JavaScript".to_owned()
    };
    Project {
        name,
        description,
        package: first_of(&manifest, &["\"name\""]).map(|s| s.trim_matches('"').to_owned()),
        source: ["src", "app", "lib"]
            .iter()
            .find(|d| root.join(d).is_dir())
            .map(|d| format!("{d}/")),
        build: Some("npm".to_owned()),
        modules: Vec::new(),
        kind,
    }
}

fn detect_simple(root: &Path, name: String, kind: Kind, language: &str, ext: &str) -> Project {
    Project {
        name,
        description: if language.is_empty() {
            kind_fallback(kind)
        } else {
            language.to_owned()
        },
        package: None,
        source: if ext.is_empty() {
            None
        } else {
            dirs_with_ext(root, &[ext]).first().map(|d| format!("{d}/"))
        },
        build: None,
        modules: Vec::new(),
        kind,
    }
}

/// Chooses the module the user is most likely working in.
///
/// Preference order: the module the current directory is inside, then the
/// module with the most source files (usually the app), then the first listed.
fn pick_module(root: &Path, modules: &[String]) -> Option<String> {
    if modules.len() <= 1 {
        return modules.first().cloned();
    }

    // Where is the shell? Only meaningful when the repo root is the cwd, which
    // is how `tg fresh` is normally run.
    let cwd = std::env::current_dir().ok();
    if let Some(cwd) = &cwd {
        for module in modules {
            let dir = root.join(module.trim_start_matches(':').replace(':', "/"));
            if cwd.starts_with(&dir) && dir != root {
                return Some(module.clone());
            }
        }
    }

    let mut best: Option<(&String, usize)> = None;
    for module in modules {
        let dir = root.join(module.trim_start_matches(':').replace(':', "/"));
        let count = count_code_files(&dir);
        if best.map(|(_, c)| count > c).unwrap_or(true) {
            best = Some((module, count));
        }
    }
    best.map(|(m, _)| m.clone())
}

/// Reads `include(":app")` style module declarations from settings files.
fn settings_modules(root: &Path) -> Vec<String> {
    let text = read_any(root, &["settings.gradle.kts", "settings.gradle"]);
    let mut modules = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("include") {
            continue;
        }
        for part in line.split('"').skip(1).step_by(2) {
            if part.starts_with(':') && !modules.iter().any(|m: &String| m == part) {
                modules.push(part.to_owned());
            }
        }
    }
    modules
}

fn detect_manifest_package(root: &Path, module_dir: &Path) -> Option<String> {
    for dir in [module_dir, root] {
        for path in [
            dir.join("src/main/AndroidManifest.xml"),
            dir.join("app/src/main/AndroidManifest.xml"),
        ] {
            let text = read(path.clone());
            // The `package` attribute is only valid on the `<manifest>` tag;
            // other elements may carry unrelated attributes of that name.
            if let Some(line) = text.lines().find(|l| l.contains("<manifest")) {
                if let Some(p) = first_of(line, &["package"]) {
                    if p.contains('.') {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

/// Finds the source root that corresponds to `package`, relative to `root`.
///
/// For `xyz.mpv.rex` this looks for a directory ending in `xyz/mpv/rex`; the
/// conventional layout `src/main/kotlin/` is returned as the directory to
/// search from, which is what a `rg` needs.
fn source_path(root: &Path, module_dir: &Path, package: Option<&str>) -> Option<PathBuf> {
    let package_path = package.map(|p| p.replace('.', "/"));
    if let Some(package_path) = package_path {
        for base in [
            "src/main/kotlin",
            "src/main/java",
            "src/main",
            "src",
            "app/src/main/kotlin",
            "app/src/main/java",
        ] {
            let candidate = module_dir.join(base).join(&package_path);
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
        // The package directory exists but not under a conventional base; find
        // the shallowest directory whose path ends with the package path.
        if let Some(found) = find_deepest(module_dir, &package_path, 6) {
            return Some(found);
        }
    }

    for base in ["src/main/kotlin", "src/main/java", "src"] {
        let candidate = module_dir.join(base);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let _ = root;
    None
}

fn find_deepest(root: &Path, suffix: &str, max_depth: usize) -> Option<PathBuf> {
    if max_depth == 0 {
        return None;
    }
    // Check this directory first so the answer is the shallowest match.
    if root.to_string_lossy().ends_with(suffix) {
        return Some(root.to_path_buf());
    }
    let Ok(entries) = fs::read_dir(root) else {
        return None;
    };
    let mut children: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !matches!(
                    p.file_name().and_then(|n| n.to_str()),
                    Some("build" | ".git" | "node_modules" | ".gradle")
                )
        })
        .collect();
    // Deterministic order regardless of filesystem.
    children.sort();
    children
        .into_iter()
        .find_map(|child| find_deepest(&child, suffix, max_depth - 1))
}

/// Renders a path relative to `base`, with a trailing slash.
fn relative(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    format!("{}/", rel.display())
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn has(root: &Path, rel: &str) -> bool {
    root.join(rel).exists()
}

fn read(path: PathBuf) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn read_any(root: &Path, names: &[&str]) -> String {
    for name in names {
        let text = read(root.join(name));
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

/// Returns the value of the first `key = "value"` or `key "value"` match.
fn first_of(text: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        for line in text.lines() {
            let line = line.trim();
            if !line.starts_with(key) {
                continue;
            }
            let rest = line[key.len()..].trim_start();
            if let Some(value) = quoted(rest) {
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// Extracts the first quoted or bare token from `rest`.
fn quoted(rest: &str) -> Option<String> {
    let rest = rest.trim_start_matches(['=', ' ', '(', ' ', ':', ';']);
    let rest = rest.trim();
    if let Some(stripped) = rest.strip_prefix('"') {
        return stripped.split('"').next().map(str::to_owned);
    }
    let token: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// Parses a Cargo manifest for `name`, `description`, `edition` and workspace
/// members' crate directories.
fn parse_workspace(text: &str) -> (Option<String>, Option<String>, Option<String>, Vec<String>) {
    let mut section = String::new();
    let mut package_name = None;
    let mut description = None;
    let mut edition = None;
    let mut members: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.trim_matches(['[', ']']).to_owned();
            continue;
        }
        let (key, value) = match trimmed.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        let value = quoted(value).unwrap_or_default();

        match section.as_str() {
            "package" => match key {
                "name" => package_name = Some(value),
                "description" => description = Some(value),
                "edition" => edition = Some(value),
                _ => {}
            },
            "workspace" if key == "members" => {
                for member in value.split(',') {
                    let member = member.trim();
                    if !member.is_empty() {
                        members.push(member.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    (package_name, description, edition, members)
}

/// Reads `members = [...]` across multiple lines.
fn parse_members_multiline(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("members") && trimmed.contains('=') {
            inside = true;
        }
        if inside {
            for part in trimmed.split('"').skip(1).step_by(2) {
                if !part.is_empty() {
                    out.push(part.trim_end_matches('/').to_owned());
                }
            }
            if trimmed.contains(']') {
                break;
            }
        }
    }
    out
}

/// Human language label from the dominant source extension.
fn language_for(root: &Path, _fallback: &str) -> String {
    let counts: BTreeMap<&str, usize> = [
        ("kt", "Kotlin"),
        ("rs", "Rust"),
        ("py", "Python"),
        ("go", "Go"),
        ("ts", "TypeScript"),
        ("java", "Java"),
        ("cpp", "C++"),
        ("cc", "C++"),
        ("c", "C"),
        ("js", "JavaScript"),
    ]
    .into_iter()
    .filter_map(|(ext, label)| {
        let n = count_ext(root, ext, 6);
        (n > 0).then_some((label, n))
    })
    .collect();

    // C and C++ both count; on a tie prefer C++ so a mixed tree reads as C++.
    let mut best: Option<(&str, usize)> = None;
    for (label, &n) in &counts {
        let better = match best {
            None => true,
            Some((best_label, best_n)) => {
                n > best_n || (n == best_n && *label != "C" && best_label == "C")
            }
        };
        if better {
            best = Some((label, n));
        }
    }
    best.map(|(label, _)| label.to_owned()).unwrap_or_default()
}

fn classify(root: &Path) -> Kind {
    if count_code_files(root) > 0 {
        Kind::Code
    } else if count_ext(root, "md", 2) > 0 {
        Kind::Docs
    } else {
        Kind::Config
    }
}

fn kind_fallback(kind: Kind) -> String {
    match kind {
        Kind::Code => "source repository".to_owned(),
        Kind::Docs => "documentation".to_owned(),
        Kind::Config => "configuration".to_owned(),
    }
}

fn dirs_with_ext(root: &Path, exts: &[&str]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 4 {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut found = false;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    ".git" | "build" | "target" | "node_modules" | ".gradle" | "dist" | ".venv"
                ) {
                    continue;
                }
                stack.push((path, depth + 1));
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if exts.contains(&ext) {
                    found = true;
                }
            }
        }
        if found {
            let rel = dir.strip_prefix(root).unwrap_or(&dir);
            let rel = if rel.as_os_str().is_empty() {
                ".".to_owned()
            } else {
                rel.display().to_string()
            };
            dirs.push(rel);
        }
    }
    // Shallowest first: the topmost directory holding source is the tree to grep.
    dirs.sort_by_key(|d| (d.matches('/').count(), d.clone()));
    dirs
}

fn count_code_files(root: &Path) -> usize {
    const EXTS: &[&str] = &[
        "kt", "kts", "java", "rs", "py", "go", "ts", "tsx", "js", "cpp", "cc", "cxx", "c", "h",
        "hpp", "swift", "rb", "cs", "scala",
    ];
    const SKIP: &[&str] = &[
        ".git",
        "build",
        "target",
        "node_modules",
        ".gradle",
        "dist",
        ".venv",
        "vendor",
    ];
    let mut total = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 8 {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    stack.push((path, depth + 1));
                }
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if EXTS.contains(&ext) {
                    total += 1;
                }
            }
        }
    }
    total
}

fn count_ext(root: &Path, ext: &str, max_depth: usize) -> usize {
    let mut total = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !matches!(
                    name.as_str(),
                    ".git" | "build" | "target" | "node_modules" | ".gradle"
                ) {
                    stack.push((path, depth + 1));
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
                total += 1;
            }
        }
    }
    total
}

fn detect_name(root: &Path) -> String {
    // Git's own name for the work tree is the one the user sees.
    let dir_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_owned());
    dir_name
}

/// Strips control characters so a hostile manifest cannot inject escape
/// sequences into the terminal or instructions into the chat.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(400)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_the_project_block() {
        let template = "intro\n\n## Project\n<placeholder>\ntype: <x>\n\n## After\nkeep\n";
        let out = fill(template, "demo: Rust\nBuild: Cargo\ntype: code");
        assert!(out.contains("## Project\ndemo: Rust\nBuild: Cargo\ntype: code\n"));
        assert!(out.contains("\n## After\nkeep\n"));
        assert!(!out.contains("<placeholder>"));
    }

    #[test]
    fn fill_is_idempotent() {
        let template = "intro\n\n## Project\n<placeholder>\n";
        let once = fill(template, "demo: Rust\ntype: code");
        let twice = fill(&once, "demo: Rust\ntype: code");
        assert_eq!(once, twice);
        assert_eq!(once.matches("## Project").count(), 1);
    }

    #[test]
    fn fill_uses_the_heading_not_a_prose_mention() {
        let template = "intro\n\nSee the ## Project block below\n\n## Project\nold\n\n## After\nkeep\n";
        let out = fill(template, "demo: Rust\ntype: code");
        // The prose mention is left alone; the real heading is replaced.
        assert!(out.contains("See the ## Project block below\n\n## Project\ndemo: Rust\ntype: code\n"));
        assert!(out.contains("\n## After\nkeep\n"));
        assert!(!out.contains("old"));
    }

    #[test]
    fn fill_appends_when_heading_missing() {
        let out = fill("only text", "demo: Rust\ntype: code");
        assert!(out.starts_with("only text\n\n## Project\n"));
    }

    #[test]
    fn fill_replaces_at_end_of_file() {
        let out = fill("body\n\n## Project\nold\n", "new\ntype: code");
        assert_eq!(out, "body\n\n## Project\nnew\ntype: code\n");
    }
}
