//! Mods and profiles: `mod.json`, profile files, versions, discovery on
//! disk and load order (04 §3).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::diag::{Diagnostic, Diagnostics, closest, code, did_you_mean};
use crate::json::{Limits, Node, NodeKind, parse_json};
use crate::source::{FileId, SourceMap, Span};

/// The namespace reserved for the engine (04 §4.4). No mod may have this
/// id, and no mod may define or change anything in it.
pub const ENGINE_NAMESPACE: &str = "engine";

/// The engine's version, which a mod's `engine` requirement is checked
/// against.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The name of a mod's manifest file.
pub const MANIFEST_FILE: &str = "mod.json";

/// The directory of a mod that holds its definitions.
pub const DATA_DIR: &str = "data";

/// The directory, beside the mods directory or inside a mod, that holds
/// profiles.
pub const PROFILES_DIR: &str = "profiles";

/// Whether `text` is one part of an id: `[a-z][a-z0-9_]*` (04 §4.1).
pub fn is_id_part(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '_'))
}

/// A version: numbers separated by dots. A missing number counts as zero,
/// so `1.2` equals `1.2.0`.
#[derive(Clone, Debug, Default)]
pub struct Version {
    parts: Vec<u64>,
}

impl Version {
    /// Reads a version such as `0.3.0`.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        for part in text.split('.') {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            parts.push(part.parse().ok()?);
        }
        Some(Version { parts })
    }

    fn part(&self, index: usize) -> u64 {
        self.parts.get(index).copied().unwrap_or(0)
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Version) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        let len = self.parts.len().max(other.parts.len());
        (0..len)
            .map(|index| self.part(index).cmp(&other.part(index)))
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.parts.is_empty() {
            return f.write_str("0");
        }
        let parts: Vec<String> = self.parts.iter().map(u64::to_string).collect();
        f.write_str(&parts.join("."))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Compare {
    AtLeast,
    Above,
    AtMost,
    Below,
    Exactly,
}

/// A requirement on a version: `>=0.1`, `>=1.0, <2.0`, `=0.3.0`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VersionReq {
    text: String,
    parts: Vec<(Compare, Version)>,
}

impl VersionReq {
    /// Reads a requirement: comparisons separated by commas. A bare
    /// version means exactly that version.
    pub fn parse(text: &str) -> Option<VersionReq> {
        let mut parts = Vec::new();
        for part in text.split(',') {
            let part = part.trim();
            let (compare, rest) = if let Some(rest) = part.strip_prefix(">=") {
                (Compare::AtLeast, rest)
            } else if let Some(rest) = part.strip_prefix("<=") {
                (Compare::AtMost, rest)
            } else if let Some(rest) = part.strip_prefix('>') {
                (Compare::Above, rest)
            } else if let Some(rest) = part.strip_prefix('<') {
                (Compare::Below, rest)
            } else if let Some(rest) = part.strip_prefix('=') {
                (Compare::Exactly, rest)
            } else {
                (Compare::Exactly, part)
            };
            parts.push((compare, Version::parse(rest)?));
        }
        Some(VersionReq {
            text: text.trim().to_owned(),
            parts,
        })
    }

    /// Whether `version` meets every comparison.
    pub fn matches(&self, version: &Version) -> bool {
        self.parts.iter().all(|(compare, wanted)| {
            let ordering = version.cmp(wanted);
            match compare {
                Compare::AtLeast => ordering != Ordering::Less,
                Compare::Above => ordering == Ordering::Greater,
                Compare::AtMost => ordering != Ordering::Greater,
                Compare::Below => ordering == Ordering::Less,
                Compare::Exactly => ordering == Ordering::Equal,
            }
        })
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// One entry of a mod's `requires`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Requirement {
    /// The required mod's id.
    pub id: String,
    /// The version it must have, if any was asked for.
    pub version: Option<VersionReq>,
    /// Where the entry was written.
    pub span: Span,
}

/// A mod's `mod.json` (04 §3.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModManifest {
    /// The mod's id, which is its namespace.
    pub id: String,
    /// The name shown to players. Defaults to the id.
    pub name: String,
    /// The mod's version. Defaults to `0`.
    pub version: Version,
    /// The engine version the mod asks for.
    pub engine: Option<VersionReq>,
    /// What the mod is.
    pub description: String,
    /// Who made it.
    pub authors: Vec<String>,
    /// Mods that must be present. Loaded before this one.
    pub requires: Vec<Requirement>,
    /// Mods loaded before this one if present.
    pub after: Vec<String>,
    /// Mods that may not be loaded alongside.
    pub conflicts: Vec<String>,
    /// Namespaces searched for bare references, in order (04 §4.3).
    /// Defaults to the ids in `requires`.
    pub imports: Vec<String>,
    /// Experimental features this mod opts in to (04 §8.4).
    pub experimental: Vec<String>,
    /// The manifest file.
    pub file: FileId,
    /// Where the id was written.
    pub id_span: Span,
}

impl ModManifest {
    /// Whether this mod lists `other` in its `requires` or `after`: the
    /// sign that it knows of `other` and means to win a disputed field
    /// (04 §6.3).
    pub fn knows_of(&self, other: &str) -> bool {
        self.requires
            .iter()
            .any(|requirement| requirement.id == other)
            || self.after.iter().any(|id| id == other)
    }
}

/// A profile: a named set of mods and how the game starts with them
/// (04 §3.3).
///
/// This crate reads the mod list. What the rest means belongs to the
/// crates that own screens and settings, so those fields are kept as
/// written, with their positions.
#[derive(Clone, PartialEq, Debug)]
pub struct Profile {
    /// The profile's id.
    pub id: String,
    /// The name shown to players. Defaults to the id.
    pub name: String,
    /// The mods, in the order listed. Position breaks ties in load order
    /// (04 §3.4).
    pub mods: Vec<String>,
    /// Where each entry of `mods` was written.
    pub mod_spans: Vec<Span>,
    /// The screen shown first, as written, and where.
    pub boot_screen: Option<(String, Span)>,
    /// The profile's `settings`, as written.
    pub settings: Option<Node>,
    /// The profile's `screens`, as written.
    pub screens: Option<Node>,
    /// The profile's `preferences`, as written.
    pub preferences: Option<Node>,
    /// The profile's file.
    pub file: FileId,
}

/// Reads fields out of one JSON object, reporting what is wrong with it.
struct ObjectReader<'a> {
    node: &'a Node,
    what: &'static str,
    known: &'static [&'static str],
    sources: &'a SourceMap,
    diagnostics: &'a mut Diagnostics,
}

impl<'a> ObjectReader<'a> {
    fn error(&mut self, span: Span, message: String) {
        self.diagnostics
            .push(Diagnostic::new(code::BAD_MANIFEST, message).at(self.sources, span));
    }

    fn check_unknown(&mut self) {
        let Some(members) = self.node.as_object() else {
            return;
        };
        for member in members {
            if !self.known.contains(&member.key.as_str()) {
                let suggestion = did_you_mean(closest(&member.key, self.known.iter().copied()));
                self.diagnostics.push(
                    Diagnostic::new(
                        code::UNKNOWN_KEY,
                        format!("unknown key \"{}\" in {}", member.key, self.what),
                    )
                    .at(self.sources, member.key_span)
                    .suggest_opt(suggestion),
                );
            }
        }
    }

    fn string(&mut self, key: &str) -> Option<(String, Span)> {
        let node = self.node.get(key)?;
        match node.as_str() {
            Some(text) => Some((text.to_owned(), node.span)),
            None => {
                let message = format!(
                    "`{key}` in {} must be a string, but this is {}",
                    self.what,
                    node.kind_name()
                );
                self.error(node.span, message);
                None
            }
        }
    }

    fn string_list(&mut self, key: &str) -> Vec<(String, Span)> {
        let Some(node) = self.node.get(key) else {
            return Vec::new();
        };
        let Some(items) = node.as_array() else {
            let message = format!(
                "`{key}` in {} must be a list of strings, but this is {}",
                self.what,
                node.kind_name()
            );
            self.error(node.span, message);
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in items {
            match item.as_str() {
                Some(text) => out.push((text.to_owned(), item.span)),
                None => {
                    let message = format!(
                        "`{key}` in {} must be a list of strings, but this entry is {}",
                        self.what,
                        item.kind_name()
                    );
                    self.error(item.span, message);
                }
            }
        }
        out
    }
}

const MANIFEST_KEYS: &[&str] = &[
    "id",
    "name",
    "version",
    "engine",
    "description",
    "authors",
    "requires",
    "after",
    "conflicts",
    "imports",
    "experimental",
];

const PROFILE_KEYS: &[&str] = &[
    "id",
    "name",
    "mods",
    "boot_screen",
    "settings",
    "screens",
    "preferences",
];

/// Reads a `mod.json` that has already been parsed. Returns `None`, with
/// the reason reported, if it cannot be used at all.
pub(crate) fn read_manifest(
    node: &Node,
    file: FileId,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
) -> Option<ModManifest> {
    if node.as_object().is_none() {
        diagnostics.push(
            Diagnostic::new(
                code::BAD_MANIFEST,
                format!(
                    "`{MANIFEST_FILE}` must be one object, but this is {}",
                    node.kind_name()
                ),
            )
            .at(sources, node.span),
        );
        return None;
    }
    let mut reader = ObjectReader {
        node,
        what: "`mod.json`",
        known: MANIFEST_KEYS,
        sources,
        diagnostics,
    };
    reader.check_unknown();
    let id = reader.string("id");
    let name = reader.string("name");
    let version = reader.string("version");
    let engine = reader.string("engine");
    let description = reader.string("description");
    let authors = reader.string_list("authors");
    let after = reader.string_list("after");
    let conflicts = reader.string_list("conflicts");
    let imports = node.get("imports").map(|_| reader.string_list("imports"));
    let experimental = reader.string_list("experimental");

    let Some((id, id_span)) = id else {
        if node.get("id").is_none() {
            diagnostics.push(
                Diagnostic::new(code::BAD_MANIFEST, "`mod.json` has no `id`")
                    .at(
                        sources,
                        Span::new(file, node.span.start, node.span.start + 1),
                    )
                    .suggest("add \"id\": the mod's namespace"),
            );
        }
        return None;
    };
    if !is_id_part(&id) {
        diagnostics.push(
            Diagnostic::new(code::BAD_MOD_ID, format!("\"{id}\" is not a valid mod id"))
                .at(sources, id_span)
                .suggest("use lowercase letters, digits and underscores, starting with a letter"),
        );
        return None;
    }
    if id == ENGINE_NAMESPACE {
        diagnostics.push(
            Diagnostic::new(
                code::BAD_MOD_ID,
                format!("the namespace \"{ENGINE_NAMESPACE}\" is reserved for the engine"),
            )
            .at(sources, id_span),
        );
        return None;
    }

    let version = match version {
        Some((text, span)) => Version::parse(&text).unwrap_or_else(|| {
            diagnostics.push(
                Diagnostic::new(code::BAD_VERSION, format!("\"{text}\" is not a version"))
                    .at(sources, span)
                    .suggest("write numbers separated by dots: \"0.3.0\""),
            );
            Version::default()
        }),
        None => Version::default(),
    };
    let engine =
        engine.and_then(|(text, span)| read_requirement(&text, span, sources, diagnostics));

    let mut requires = Vec::new();
    if let Some(list) = node.get("requires") {
        match list.as_array() {
            Some(items) => {
                for item in items {
                    if let Some(requirement) = read_requires_entry(item, sources, diagnostics) {
                        requires.push(requirement);
                    }
                }
            }
            None => diagnostics.push(
                Diagnostic::new(
                    code::BAD_MANIFEST,
                    format!(
                        "`requires` in `mod.json` must be a list, but this is {}",
                        list.kind_name()
                    ),
                )
                .at(sources, list.span),
            ),
        }
    }
    let imports = match imports {
        Some(list) => list.into_iter().map(|(text, _)| text).collect(),
        None => requires
            .iter()
            .map(|requirement| requirement.id.clone())
            .collect(),
    };
    Some(ModManifest {
        name: name.map_or_else(|| id.clone(), |(text, _)| text),
        id,
        version,
        engine,
        description: description.map(|(text, _)| text).unwrap_or_default(),
        authors: authors.into_iter().map(|(text, _)| text).collect(),
        requires,
        after: after.into_iter().map(|(text, _)| text).collect(),
        conflicts: conflicts.into_iter().map(|(text, _)| text).collect(),
        imports,
        experimental: experimental.into_iter().map(|(text, _)| text).collect(),
        file,
        id_span,
    })
}

fn read_requirement(
    text: &str,
    span: Span,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
) -> Option<VersionReq> {
    let requirement = VersionReq::parse(text);
    if requirement.is_none() {
        diagnostics.push(
            Diagnostic::new(
                code::BAD_VERSION,
                format!("\"{text}\" is not a version requirement"),
            )
            .at(sources, span)
            .suggest("write comparisons such as \">=0.1\" or \">=1.0, <2.0\""),
        );
    }
    requirement
}

fn read_requires_entry(
    item: &Node,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
) -> Option<Requirement> {
    match &item.kind {
        NodeKind::String(id) => Some(Requirement {
            id: id.clone(),
            version: None,
            span: item.span,
        }),
        NodeKind::Object(_) => {
            let mut reader = ObjectReader {
                node: item,
                what: "an entry of `requires`",
                known: &["id", "version"],
                sources,
                diagnostics,
            };
            reader.check_unknown();
            let id = reader.string("id");
            let version = reader.string("version");
            let Some((id, _)) = id else {
                if item.get("id").is_none() {
                    diagnostics.push(
                        Diagnostic::new(code::BAD_MANIFEST, "an entry of `requires` has no `id`")
                            .at(sources, item.span),
                    );
                }
                return None;
            };
            let version = version
                .and_then(|(text, span)| read_requirement(&text, span, sources, diagnostics));
            Some(Requirement {
                id,
                version,
                span: item.span,
            })
        }
        _ => {
            diagnostics.push(
                Diagnostic::new(
                    code::BAD_MANIFEST,
                    format!(
                        "an entry of `requires` is a mod id or an object with `id` and `version`, but this is {}",
                        item.kind_name()
                    ),
                )
                .at(sources, item.span),
            );
            None
        }
    }
}

/// Reads a profile file that has already been parsed.
pub(crate) fn read_profile(
    node: &Node,
    file: FileId,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
) -> Option<Profile> {
    if node.as_object().is_none() {
        diagnostics.push(
            Diagnostic::new(
                code::BAD_MANIFEST,
                format!(
                    "a profile must be one object, but this is {}",
                    node.kind_name()
                ),
            )
            .at(sources, node.span),
        );
        return None;
    }
    let mut reader = ObjectReader {
        node,
        what: "a profile",
        known: PROFILE_KEYS,
        sources,
        diagnostics,
    };
    reader.check_unknown();
    let id = reader.string("id");
    let name = reader.string("name");
    let boot_screen = reader.string("boot_screen");
    let mods = reader.string_list("mods");
    let Some((id, _)) = id else {
        if node.get("id").is_none() {
            diagnostics.push(
                Diagnostic::new(code::BAD_MANIFEST, "a profile has no `id`").at(
                    sources,
                    Span::new(file, node.span.start, node.span.start + 1),
                ),
            );
        }
        return None;
    };
    if node.get("mods").is_none() {
        diagnostics.push(
            Diagnostic::new(code::BAD_MANIFEST, "a profile has no `mods`")
                .at(
                    sources,
                    Span::new(file, node.span.start, node.span.start + 1),
                )
                .suggest("add \"mods\": the list of mod ids, which may be empty"),
        );
        return None;
    }
    let mut seen: Vec<&str> = Vec::new();
    for (mod_id, span) in &mods {
        if seen.contains(&mod_id.as_str()) {
            diagnostics.push(
                Diagnostic::new(
                    code::BAD_MANIFEST,
                    format!("mod \"{mod_id}\" is listed twice in the profile"),
                )
                .at(sources, *span),
            );
        }
        seen.push(mod_id);
    }
    let mut unique_mods = Vec::new();
    let mut mod_spans = Vec::new();
    for (mod_id, span) in mods {
        if !unique_mods.contains(&mod_id) {
            unique_mods.push(mod_id);
            mod_spans.push(span);
        }
    }
    Some(Profile {
        name: name.map_or_else(|| id.clone(), |(text, _)| text),
        id,
        mods: unique_mods,
        mod_spans,
        boot_screen,
        settings: node.get("settings").cloned(),
        screens: node.get("screens").cloned(),
        preferences: node.get("preferences").cloned(),
        file,
    })
}

/// Where a mod's files come from.
#[derive(Clone, Debug)]
pub(crate) enum ModBacking {
    /// A directory on disk.
    Disk(PathBuf),
    /// Files given in memory: relative path and text.
    Memory(Vec<(String, String)>),
}

/// A mod that was found, before its data is read.
#[derive(Clone, Debug)]
pub(crate) struct FoundMod {
    /// The mod's directory as shown in diagnostics, with `/` separators.
    pub(crate) root: String,
    /// The manifest.
    pub(crate) manifest: ModManifest,
    /// Where its files are.
    pub(crate) backing: ModBacking,
}

/// Joins a shown directory and a relative path with `/`.
pub(crate) fn join_shown(root: &str, relative: &str) -> String {
    if root.is_empty() {
        relative.to_owned()
    } else if root.ends_with('/') {
        format!("{root}{relative}")
    } else {
        format!("{root}/{relative}")
    }
}

/// A path as shown in diagnostics: with `/` separators on every platform.
pub(crate) fn shown_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The names in a directory, sorted, so that nothing depends on the order
/// the file system lists them in (M1). Names that are not Unicode and
/// names starting with a dot are skipped.
pub(crate) fn sorted_entries(dir: &Path) -> std::io::Result<Vec<(String, bool)>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        // Following links: a mod may be a link to a working copy.
        let is_dir = std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
        entries.push((name, is_dir));
    }
    entries.sort();
    Ok(entries)
}

/// Reads a file as UTF-8 text, refusing one over the size limit before
/// reading it.
pub(crate) fn read_text(path: &Path, limits: &Limits) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if meta.len() > limits.max_file_bytes as u64 {
        return Err(format!(
            "file is {} bytes; the limit is {}",
            meta.len(),
            limits.max_file_bytes
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    String::from_utf8(bytes).map_err(|_| "file is not UTF-8 text".to_owned())
}

/// Lists the `*.json` files under a directory, any depth, as paths
/// relative to it with `/` separators, sorted.
pub(crate) fn json_files_under(
    dir: &Path,
    limits: &Limits,
) -> Result<Vec<String>, (String, String)> {
    let mut out = Vec::new();
    let mut pending: Vec<(PathBuf, String, usize)> = vec![(dir.to_path_buf(), String::new(), 0)];
    while let Some((path, relative, depth)) = pending.pop() {
        if depth > limits.max_depth {
            return Err((
                shown_path(&path),
                format!("directories are nested more than {} deep", limits.max_depth),
            ));
        }
        let entries =
            sorted_entries(&path).map_err(|error| (shown_path(&path), error.to_string()))?;
        for (name, is_dir) in entries {
            let child_relative = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            if is_dir {
                pending.push((path.join(&name), child_relative, depth + 1));
            } else if name.ends_with(".json") {
                out.push(child_relative);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Resolves a file reference written inside a mod against the mod's
/// directory, refusing one that leaves it (04 §15).
///
/// `relative` is what the mod wrote, with `/` separators. It is refused,
/// with a reason, if it is absolute, uses a drive or `\`, or steps out
/// through `..`. The check is on the path as written; it does not touch
/// the file system.
///
/// ```
/// use std::path::Path;
/// use clash_data::resolve_in_mod;
///
/// let root = Path::new("mods/ashlands");
/// assert_eq!(
///     resolve_in_mod(root, "assets/font.png").unwrap(),
///     root.join("assets").join("font.png")
/// );
/// assert!(resolve_in_mod(root, "../core/mod.json").is_err());
/// assert!(resolve_in_mod(root, "/etc/passwd").is_err());
/// ```
pub fn resolve_in_mod(mod_root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() {
        return Err("a file reference is empty".to_owned());
    }
    if relative.contains('\\') {
        return Err(format!(
            "file reference \"{relative}\" uses `\\`; write paths with `/`"
        ));
    }
    let mut out = mod_root.to_path_buf();
    let mut depth = 0_usize;
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => {
                // `C:` and the like parse as a normal component elsewhere.
                if part.to_string_lossy().contains(':') {
                    return Err(format!(
                        "file reference \"{relative}\" names a drive; a mod's files are inside its directory"
                    ));
                }
                out.push(part);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return Err(format!(
                        "file reference \"{relative}\" leaves the mod's directory"
                    ));
                }
                out.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "file reference \"{relative}\" is absolute; a mod's files are inside its directory"
                ));
            }
        }
    }
    if depth == 0 {
        return Err(format!(
            "file reference \"{relative}\" names the mod's directory itself"
        ));
    }
    Ok(out)
}

/// Parses a manifest's text and reads it.
pub(crate) fn load_manifest(
    shown: &str,
    text: String,
    sources: &mut SourceMap,
    limits: &Limits,
    diagnostics: &mut Diagnostics,
) -> Option<ModManifest> {
    let file = sources.add(shown, text);
    let (tree, problems) = parse_json(file, sources.text(file), limits);
    for problem in problems {
        diagnostics.push(
            Diagnostic::new(problem.code, problem.message)
                .at(sources, problem.span)
                .suggest_opt(problem.suggestion),
        );
    }
    read_manifest(&tree?, file, sources, diagnostics)
}

/// Finds the mods in one search directory: every immediate subdirectory
/// that holds a `mod.json`. `dir` may also be a mod directory itself.
pub(crate) fn discover_dir(
    dir: &Path,
    sources: &mut SourceMap,
    limits: &Limits,
    diagnostics: &mut Diagnostics,
) -> Vec<FoundMod> {
    let mut found: Vec<FoundMod> = Vec::new();
    let shown_dir = shown_path(dir);
    let entries = match sorted_entries(dir) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(Diagnostic::new(
                code::UNREADABLE_FILE,
                format!("cannot read the mods directory `{shown_dir}`: {error}"),
            ));
            return found;
        }
    };
    for (name, is_dir) in entries {
        if !is_dir {
            continue;
        }
        let mod_dir = dir.join(&name);
        if !mod_dir.join(MANIFEST_FILE).is_file() {
            continue;
        }
        if let Some(found_mod) = read_mod_dir(&mod_dir, &name, sources, limits, diagnostics) {
            if let Some(first) = found
                .iter()
                .find(|other| other.manifest.id == found_mod.manifest.id)
            {
                diagnostics.push(
                    Diagnostic::new(
                        code::DUPLICATE_MOD,
                        format!(
                            "mod \"{}\" is declared by two directories: `{}` and `{}`",
                            found_mod.manifest.id, first.root, found_mod.root
                        ),
                    )
                    .at(sources, found_mod.manifest.id_span)
                    .note_at(
                        sources,
                        first.manifest.id_span,
                        "first declared here",
                    ),
                );
                continue;
            }
            found.push(found_mod);
        }
    }
    found
}

/// Reads the manifest of the mod in `mod_dir`.
pub(crate) fn read_mod_dir(
    mod_dir: &Path,
    dir_name: &str,
    sources: &mut SourceMap,
    limits: &Limits,
    diagnostics: &mut Diagnostics,
) -> Option<FoundMod> {
    let root = shown_path(mod_dir);
    let manifest_shown = join_shown(&root, MANIFEST_FILE);
    let text = match read_text(&mod_dir.join(MANIFEST_FILE), limits) {
        Ok(text) => text,
        Err(error) => {
            diagnostics.push(Diagnostic::new(
                code::UNREADABLE_FILE,
                format!("cannot read `{manifest_shown}`: {error}"),
            ));
            return None;
        }
    };
    let manifest = load_manifest(&manifest_shown, text, sources, limits, diagnostics)?;
    if manifest.id != dir_name {
        diagnostics.push(
            Diagnostic::new(
                code::MOD_DIR_NAME,
                format!(
                    "mod \"{}\" is in a directory named `{dir_name}`",
                    manifest.id
                ),
            )
            .at(sources, manifest.id_span)
            .suggest(format!("rename the directory to `{}`", manifest.id)),
        );
    }
    Some(FoundMod {
        root,
        manifest,
        backing: ModBacking::Disk(mod_dir.to_path_buf()),
    })
}

/// Puts the chosen mods in load order (04 §3.4): everything in a mod's
/// `requires`, and every chosen mod in its `after`, comes before it;
/// remaining ties are broken by position in `chosen`, then by id.
///
/// `chosen` holds indices into `mods`, in the profile's order. Missing
/// requirements, unmet versions, conflicts and cycles are reported. The
/// order returned always holds every chosen mod, so that loading can go
/// on and report what else is wrong.
pub(crate) fn order_mods(
    chosen: &[usize],
    mods: &[FoundMod],
    profile_spans: Option<&[Span]>,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
) -> Vec<usize> {
    let position: BTreeMap<&str, usize> = chosen
        .iter()
        .enumerate()
        .map(|(position, &index)| (mods[index].manifest.id.as_str(), position))
        .collect();
    let engine = Version::parse(ENGINE_VERSION).unwrap_or_default();

    // Checks that do not affect the order.
    for &index in chosen {
        let manifest = &mods[index].manifest;
        if let Some(requirement) = &manifest.engine
            && !requirement.matches(&engine)
        {
            diagnostics.push(
                Diagnostic::new(
                    code::VERSION_MISMATCH,
                    format!(
                        "mod \"{}\" asks for engine {requirement}, and this is engine {engine}",
                        manifest.id
                    ),
                )
                .at(sources, manifest.id_span),
            );
        }
        for requirement in &manifest.requires {
            match position.get(requirement.id.as_str()) {
                None => {
                    let known = mods.iter().any(|other| other.manifest.id == requirement.id);
                    let mut diagnostic = Diagnostic::new(
                        code::MISSING_MOD,
                        format!(
                            "mod \"{}\" requires \"{}\", which is not {}",
                            manifest.id,
                            requirement.id,
                            if known { "in the profile" } else { "installed" }
                        ),
                    )
                    .at(sources, requirement.span);
                    if known {
                        diagnostic = diagnostic.suggest(format!(
                            "add \"{}\" to the profile's `mods`",
                            requirement.id
                        ));
                    } else {
                        let ids = mods.iter().map(|other| other.manifest.id.as_str());
                        diagnostic =
                            diagnostic.suggest_opt(did_you_mean(closest(&requirement.id, ids)));
                    }
                    diagnostics.push(diagnostic);
                }
                Some(&at) => {
                    let other = &mods[chosen[at]].manifest;
                    if let Some(wanted) = &requirement.version
                        && !wanted.matches(&other.version)
                    {
                        diagnostics.push(
                            Diagnostic::new(
                                code::VERSION_MISMATCH,
                                format!(
                                    "mod \"{}\" requires \"{}\" {wanted}, and the one present is {}",
                                    manifest.id, other.id, other.version
                                ),
                            )
                            .at(sources, requirement.span),
                        );
                    }
                }
            }
        }
        for conflict in &manifest.conflicts {
            if position.contains_key(conflict.as_str()) {
                let mut diagnostic = Diagnostic::new(
                    code::MOD_CONFLICT,
                    format!(
                        "mod \"{}\" may not be loaded alongside \"{conflict}\"",
                        manifest.id
                    ),
                );
                diagnostic = match profile_spans
                    .and_then(|spans| spans.get(position[manifest.id.as_str()]))
                {
                    Some(&span) => diagnostic.at(sources, span),
                    None => diagnostic.at(sources, manifest.id_span),
                };
                diagnostics.push(diagnostic.suggest("remove one of the two from the profile"));
            }
        }
    }

    // What each chosen mod waits for, as positions in `chosen`.
    let waits: Vec<Vec<usize>> = chosen
        .iter()
        .map(|&index| {
            let manifest = &mods[index].manifest;
            let mut before: Vec<usize> = manifest
                .requires
                .iter()
                .map(|requirement| requirement.id.as_str())
                .chain(manifest.after.iter().map(String::as_str))
                .filter(|id| *id != manifest.id)
                .filter_map(|id| position.get(id).copied())
                .collect();
            before.sort_unstable();
            before.dedup();
            before
        })
        .collect();

    let key = |at: usize| (at, mods[chosen[at]].manifest.id.as_str());
    let mut placed = vec![false; chosen.len()];
    let mut order: Vec<usize> = Vec::with_capacity(chosen.len());
    loop {
        let next = (0..chosen.len())
            .filter(|&at| !placed[at] && waits[at].iter().all(|&before| placed[before]))
            .min_by_key(|&at| key(at));
        match next {
            Some(at) => {
                placed[at] = true;
                order.push(chosen[at]);
            }
            None => break,
        }
    }
    if order.len() < chosen.len() {
        // Every mod left waits for another that is left. Walk from the
        // first until one repeats: that is a circle.
        let start = (0..chosen.len())
            .filter(|&at| !placed[at])
            .min_by_key(|&at| key(at))
            .unwrap_or(0);
        let mut walk = vec![start];
        let circle = loop {
            let at = walk[walk.len() - 1];
            let Some(&next) = waits[at].iter().find(|&&before| !placed[before]) else {
                break walk.clone();
            };
            if let Some(first) = walk.iter().position(|&seen| seen == next) {
                let mut circle = walk[first..].to_vec();
                circle.push(next);
                break circle;
            }
            walk.push(next);
        };
        let names: Vec<&str> = circle
            .iter()
            .map(|&at| mods[chosen[at]].manifest.id.as_str())
            .collect();
        let first = &mods[chosen[circle[0]]].manifest;
        diagnostics.push(
            Diagnostic::new(
                code::MOD_CYCLE,
                format!(
                    "mods depend on each other in a circle: {}",
                    names.join(" -> ")
                ),
            )
            .at(sources, first.id_span)
            .note("each mod loads after the mods in its `requires` and `after`"),
        );
        // Carry on with the rest in profile order so that other mistakes
        // are still found.
        for (at, &index) in chosen.iter().enumerate() {
            if !placed[at] {
                order.push(index);
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_of(text: &str) -> (Option<ModManifest>, Diagnostics) {
        let mut sources = SourceMap::new();
        let mut diagnostics = Diagnostics::new();
        let manifest = load_manifest(
            "mods/x/mod.json",
            text.to_owned(),
            &mut sources,
            &Limits::default(),
            &mut diagnostics,
        );
        (manifest, diagnostics)
    }

    #[test]
    fn versions_compare_number_by_number() {
        let v = |text: &str| Version::parse(text).unwrap();
        assert!(v("0.3.0") > v("0.2.9"));
        assert!(v("0.10") > v("0.9"));
        assert_eq!(v("1.2"), v("1.2.0"));
        assert!(v("2") > v("1.99.99"));
        assert_eq!(v("0.3.0").to_string(), "0.3.0");
        assert!(Version::parse("").is_none());
        assert!(Version::parse("1..2").is_none());
        assert!(Version::parse("1.x").is_none());
        assert!(Version::parse("-1").is_none());
    }

    #[test]
    fn requirements_combine_comparisons() {
        let req = |text: &str| VersionReq::parse(text).unwrap();
        let v = |text: &str| Version::parse(text).unwrap();
        assert!(req(">=0.1").matches(&v("0.1.0")));
        assert!(req(">=0.1").matches(&v("0.3")));
        assert!(!req(">=0.1").matches(&v("0.0.9")));
        assert!(req(">=1.0, <2.0").matches(&v("1.5")));
        assert!(!req(">=1.0, <2.0").matches(&v("2.0")));
        assert!(req("=0.3.0").matches(&v("0.3")));
        assert!(req("0.3").matches(&v("0.3.0")));
        assert!(req(">1").matches(&v("1.0.1")));
        assert!(req("<=1").matches(&v("1")));
        assert!(VersionReq::parse(">=").is_none());
        assert!(VersionReq::parse("~1.0").is_none());
        assert_eq!(req(" >=0.1 ").to_string(), ">=0.1");
    }

    #[test]
    fn the_example_manifest_of_the_specification_reads() {
        let (manifest, diagnostics) = manifest_of(
            r#"{
              "id": "ashlands",
              "name": "The Ashlands",
              "version": "0.3.0",
              "engine": ">=0.1",
              "description": "A dead world and the things that live on it.",
              "authors": ["someone"],
              "requires": [ { "id": "core", "version": ">=0.1" }, "ui" ],
              "after": ["weather_plus"],
              "conflicts": ["old_ashlands"],
              "experimental": []
            }"#,
        );
        assert_eq!(diagnostics.render(), "");
        let manifest = manifest.unwrap();
        assert_eq!(manifest.id, "ashlands");
        assert_eq!(manifest.name, "The Ashlands");
        assert_eq!(manifest.version, Version::parse("0.3").unwrap());
        assert_eq!(manifest.requires.len(), 2);
        assert_eq!(manifest.requires[0].id, "core");
        assert!(manifest.requires[0].version.is_some());
        assert_eq!(manifest.requires[1].id, "ui");
        // `imports` defaults to the ids in `requires`.
        assert_eq!(manifest.imports, ["core", "ui"]);
        assert!(manifest.knows_of("core"));
        assert!(manifest.knows_of("weather_plus"));
        assert!(!manifest.knows_of("old_ashlands"));
    }

    #[test]
    fn manifest_mistakes_are_located() {
        let (manifest, diagnostics) = manifest_of(r#"{ "name": "x" }"#);
        assert!(manifest.is_none());
        assert_eq!(diagnostics.as_slice()[0].message, "`mod.json` has no `id`");

        let (manifest, diagnostics) = manifest_of(r#"{ "id": "Bad-Id" }"#);
        assert!(manifest.is_none());
        let diagnostic = &diagnostics.as_slice()[0];
        assert_eq!(diagnostic.code, "E0002");
        assert_eq!(diagnostic.location.as_ref().unwrap().column, 9);

        let (manifest, diagnostics) = manifest_of(r#"{ "id": "engine" }"#);
        assert!(manifest.is_none());
        assert!(diagnostics.as_slice()[0].message.contains("reserved"));

        let (manifest, diagnostics) = manifest_of(
            r#"{ "id": "x", "verison": "1", "requires": [3], "after": "core", "version": "one" }"#,
        );
        assert!(manifest.is_some());
        let codes: Vec<&str> = diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["E0203", "E0001", "E0009", "E0001"]);
        assert_eq!(
            diagnostics.as_slice()[0].suggestion.as_deref(),
            Some("did you mean \"version\"?")
        );

        let (manifest, diagnostics) = manifest_of("[]");
        assert!(manifest.is_none());
        assert!(
            diagnostics.as_slice()[0]
                .message
                .contains("must be one object")
        );
    }

    #[test]
    fn file_references_stay_inside_the_mod() {
        let root = Path::new("mods/m");
        assert_eq!(
            resolve_in_mod(root, "a/./b/../c.png").unwrap(),
            root.join("a").join("c.png")
        );
        for bad in [
            "",
            "..",
            "../x",
            "a/../../x",
            "/abs",
            "a\\b",
            "C:/x",
            ".",
            "a/..",
        ] {
            assert!(
                resolve_in_mod(root, bad).is_err(),
                "{bad:?} must be refused"
            );
        }
        assert!(
            resolve_in_mod(root, "../x")
                .unwrap_err()
                .contains("leaves the mod's directory")
        );
    }

    #[test]
    fn id_parts_follow_the_form() {
        for good in ["a", "ash", "ash_2", "a_b_c9"] {
            assert!(is_id_part(good), "{good}");
        }
        for bad in ["", "A", "9a", "_a", "a-b", "a:b", "a b", "é"] {
            assert!(!is_id_part(bad), "{bad}");
        }
    }
}
