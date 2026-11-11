//! The pipeline: from a profile or a directory of mods to [`Defs`]
//! (04 §2).
//!
//! ```text
//!  profile ─► discover mods ─► order ─► parse files ─► collect objects by (type, id)
//!                                                           │
//!      Defs ◄─ validate ◄─ typed load ◄─ link ids ◄─ resolve inheritance ◄─ fold patches
//! ```
//!
//! Errors from every stage are collected and reported together. A stage
//! runs for every definition that survived the one before; loading never
//! stops at the first error (04 §8.1, M2).
//!
//! Nothing here depends on the order the file system lists files in:
//! directories are read sorted, mods are ordered by the rules of 04 §3.4,
//! and dense ids are assigned in sorted order of qualified id (M1).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::{Hash128, StateHasher};

use crate::de::{DeCx, IdTable, LinkShared};
use crate::defs::{
    AnyStaged, CrossCx, DefTypes, Defs, DefsParts, DefsView, FinishedTree, LoadedModInfo,
    StageEntry, StageEnv, TagTable,
};
use crate::diag::{DefLabel, Diagnostic, Diagnostics, closest, code, did_you_mean};
use crate::json::{Limits, Node, NodeKind, Num, parse_json};
use crate::merge::{Finished, Key, LoadedMod, MergeCx, ModTable, ParsedFile, collect, merge};
use crate::mods::{
    DATA_DIR, ENGINE_NAMESPACE, FoundMod, MANIFEST_FILE, ModBacking, PROFILES_DIR, Profile,
    discover_dir, join_shown, json_files_under, load_manifest, order_mods, read_mod_dir,
    read_profile, read_text, shown_path,
};
use crate::source::{SourceMap, Span};

/// What a load produced.
#[derive(Debug)]
pub struct LoadOutcome {
    /// The definitions, if the load succeeded: no errors, or a lenient
    /// load in which every broken definition has a placeholder.
    pub defs: Option<Arc<Defs>>,
    /// Everything there was to say, in reading order.
    pub diagnostics: Diagnostics,
    /// Every file that was read.
    pub sources: Arc<SourceMap>,
}

impl LoadOutcome {
    /// Whether any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.has_errors()
    }

    /// The definitions, or the diagnostics if there are none.
    pub fn into_result(self) -> Result<Arc<Defs>, Diagnostics> {
        match self.defs {
            Some(defs) => Ok(defs),
            None => Err(self.diagnostics),
        }
    }
}

/// A mod given to the loader as text, for tests and tools.
#[derive(Clone, Debug)]
struct MemoryMod {
    dir: String,
    manifest: String,
    files: Vec<(String, String)>,
}

/// Loads mods into [`Defs`].
///
/// ```
/// use clash_data::{DefTypes, Loader};
///
/// // The engine runs with zero mods loaded (04 M7).
/// let types = DefTypes::new();
/// let outcome = Loader::new(&types).load_mods(&[]);
/// assert!(!outcome.has_errors());
/// let defs = outcome.defs.unwrap();
/// assert!(defs.mods().is_empty());
/// ```
///
/// From disk, name the directories mods are searched for in and what to
/// load:
///
/// ```no_run
/// use clash_data::{DefTypes, Loader};
///
/// let types = DefTypes::new();
/// let outcome = Loader::new(&types)
///     .search_path("mods")
///     .load_profile("default");
/// print!("{}", outcome.diagnostics.render());
/// ```
#[derive(Clone)]
pub struct Loader<'t> {
    types: &'t DefTypes,
    search: Vec<PathBuf>,
    memory: Vec<MemoryMod>,
    memory_profiles: Vec<(String, String)>,
    lenient: bool,
    limits: Limits,
}

/// Which mods to load.
enum Selection {
    /// Every mod found, ordered by id.
    All,
    /// These, in this order, as a profile lists them.
    Listed(Vec<String>, Option<Vec<Span>>),
    /// One mod and everything it requires.
    WithRequirements(String),
}

impl<'t> Loader<'t> {
    /// A loader for these definition types, with nowhere to look yet.
    pub fn new(types: &'t DefTypes) -> Loader<'t> {
        Loader {
            types,
            search: Vec::new(),
            memory: Vec::new(),
            memory_profiles: Vec::new(),
            lenient: false,
            limits: Limits::default(),
        }
    }

    /// Adds a directory that mods are searched for in: each of its
    /// subdirectories that holds a `mod.json` is a mod (04 §3.1).
    /// Directories are searched in the order added, and the first to
    /// hold a given mod id supplies it.
    pub fn search_path(mut self, dir: impl Into<PathBuf>) -> Loader<'t> {
        self.search.push(dir.into());
        self
    }

    /// Adds a mod given as text. `dir` is the directory name it is shown
    /// under, `manifest` the text of its `mod.json`, and `files` its
    /// other files as paths relative to the mod (`data/materials.json`)
    /// and their text. Mods in memory are searched before directories.
    pub fn memory_mod(mut self, dir: &str, manifest: &str, files: &[(&str, &str)]) -> Loader<'t> {
        self.memory.push(MemoryMod {
            dir: dir.to_owned(),
            manifest: manifest.to_owned(),
            files: files
                .iter()
                .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
                .collect(),
        });
        self
    }

    /// Adds a profile given as text, which
    /// [`load_profile`](Loader::load_profile) finds under `name`.
    pub fn memory_profile(mut self, name: &str, text: &str) -> Loader<'t> {
        self.memory_profiles
            .push((name.to_owned(), text.to_owned()));
        self
    }

    /// Whether to replace each broken definition with a placeholder and
    /// carry on (04 §8.3). Off unless asked for.
    pub fn lenient(mut self, lenient: bool) -> Loader<'t> {
        self.lenient = lenient;
        self
    }

    /// Sets the safety limits (04 §15).
    pub fn limits(mut self, limits: Limits) -> Loader<'t> {
        self.limits = limits;
        self
    }

    /// Loads every mod found, as if a profile listed them in order of
    /// id.
    pub fn load_all(self) -> LoadOutcome {
        self.run(Selection::All, None, SourceMap::new(), Diagnostics::new())
    }

    /// Loads exactly these mods, as a profile that lists them in this
    /// order would. An empty list gives empty, valid `Defs` (04 M7).
    pub fn load_mods(self, ids: &[&str]) -> LoadOutcome {
        let ids = ids.iter().map(|id| (*id).to_owned()).collect();
        self.run(
            Selection::Listed(ids, None),
            None,
            SourceMap::new(),
            Diagnostics::new(),
        )
    }

    /// Loads one mod and every mod it requires, directly or through
    /// another.
    pub fn load_mod_with_requirements(self, id: &str) -> LoadOutcome {
        self.run(
            Selection::WithRequirements(id.to_owned()),
            None,
            SourceMap::new(),
            Diagnostics::new(),
        )
    }

    /// Loads the mod in a directory, and every mod it requires from the
    /// directory beside it. This is what linting one mod does.
    pub fn load_mod_dir(mut self, dir: &Path) -> LoadOutcome {
        let mut sources = SourceMap::new();
        let mut diagnostics = Diagnostics::new();
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Read the manifest once here only to learn the id; discovery
        // reads it again as part of the search.
        let mut scratch = SourceMap::new();
        let mut scratch_diagnostics = Diagnostics::new();
        let found = read_mod_dir(
            dir,
            &name,
            &mut scratch,
            &self.limits,
            &mut scratch_diagnostics,
        );
        let parent = match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        if !self.search.contains(&parent) {
            self.search.insert(0, parent);
        }
        match found {
            Some(found) => self.run(
                Selection::WithRequirements(found.manifest.id),
                None,
                sources,
                diagnostics,
            ),
            None => {
                // The manifest is unusable: report why, against the real
                // source map.
                let _ = read_mod_dir(dir, &name, &mut sources, &self.limits, &mut diagnostics);
                if diagnostics.is_empty() {
                    diagnostics.extend(scratch_diagnostics);
                }
                LoadOutcome {
                    defs: None,
                    diagnostics,
                    sources: Arc::new(sources),
                }
            }
        }
    }

    /// Loads the profile in a file (04 §3.3).
    pub fn load_profile_file(self, path: &Path) -> LoadOutcome {
        let mut sources = SourceMap::new();
        let mut diagnostics = Diagnostics::new();
        let shown = shown_path(path);
        match read_text(path, &self.limits) {
            Ok(text) => self.load_profile_text_into(&shown, text, sources, diagnostics),
            Err(error) => {
                diagnostics.push(Diagnostic::new(
                    code::BAD_PROFILE,
                    format!("cannot read the profile `{shown}`: {error}"),
                ));
                sources.add(shown, "");
                LoadOutcome {
                    defs: None,
                    diagnostics,
                    sources: Arc::new(sources),
                }
            }
        }
    }

    /// Loads a profile given as text. `shown` is the path it is reported
    /// under.
    pub fn load_profile_text(self, shown: &str, text: &str) -> LoadOutcome {
        self.load_profile_text_into(shown, text.to_owned(), SourceMap::new(), Diagnostics::new())
    }

    /// Loads a profile by name: `profiles/<name>.json` beside each search
    /// directory, then in the `profiles/` directory of each mod found,
    /// then among the profiles given in memory (04 §3.3). The default
    /// profile is the one named `default`.
    pub fn load_profile(self, name: &str) -> LoadOutcome {
        let file_name = format!("{name}.json");
        if let Some((_, text)) = self
            .memory_profiles
            .iter()
            .find(|(profile_name, _)| profile_name == name)
        {
            let text = text.clone();
            let shown = join_shown(PROFILES_DIR, &file_name);
            return self.load_profile_text(&shown, &text);
        }
        let mut looked: Vec<String> = Vec::new();
        let mut at: Option<PathBuf> = None;
        'search: for dir in &self.search {
            let beside = match dir.parent() {
                Some(parent) => parent.join(PROFILES_DIR).join(&file_name),
                None => Path::new(PROFILES_DIR).join(&file_name),
            };
            if beside.is_file() {
                at = Some(beside);
                break;
            }
            looked.push(shown_path(&beside));
            if let Ok(entries) = crate::mods::sorted_entries(dir) {
                for (entry, is_dir) in entries {
                    let inside = dir.join(&entry).join(PROFILES_DIR).join(&file_name);
                    if is_dir && inside.is_file() {
                        at = Some(inside);
                        break 'search;
                    }
                }
            }
            looked.push(format!("{}/*/{PROFILES_DIR}/{file_name}", shown_path(dir)));
        }
        if let Some(path) = at {
            return self.load_profile_file(&path);
        }
        let mut diagnostics = Diagnostics::new();
        let mut diagnostic = Diagnostic::new(
            code::BAD_PROFILE,
            format!("no profile named \"{name}\" was found"),
        );
        for place in looked {
            diagnostic = diagnostic.note(format!("looked for `{place}`"));
        }
        diagnostics.push(diagnostic);
        LoadOutcome {
            defs: None,
            diagnostics,
            sources: Arc::new(SourceMap::new()),
        }
    }

    fn load_profile_text_into(
        self,
        shown: &str,
        text: String,
        mut sources: SourceMap,
        mut diagnostics: Diagnostics,
    ) -> LoadOutcome {
        let file = sources.add(shown, text);
        let (tree, problems) = parse_json(file, sources.text(file), &self.limits);
        for problem in problems {
            diagnostics.push(
                Diagnostic::new(problem.code, problem.message)
                    .at(&sources, problem.span)
                    .suggest_opt(problem.suggestion),
            );
        }
        let profile = tree.and_then(|tree| read_profile(&tree, file, &sources, &mut diagnostics));
        match profile {
            Some(profile) => {
                let selection =
                    Selection::Listed(profile.mods.clone(), Some(profile.mod_spans.clone()));
                self.run(selection, Some(profile), sources, diagnostics)
            }
            None => LoadOutcome {
                defs: None,
                diagnostics,
                sources: Arc::new(sources),
            },
        }
    }

    /// Finds every mod: those in memory first, then each search
    /// directory in turn.
    fn discover(&self, sources: &mut SourceMap, diagnostics: &mut Diagnostics) -> Vec<FoundMod> {
        let mut found: Vec<FoundMod> = Vec::new();
        let mut add =
            |candidates: Vec<FoundMod>, sources: &SourceMap, diagnostics: &mut Diagnostics| {
                for candidate in candidates {
                    match found
                        .iter()
                        .find(|other| other.manifest.id == candidate.manifest.id)
                    {
                        None => found.push(candidate),
                        Some(first) => diagnostics.push(
                            Diagnostic::new(
                                code::MOD_SHADOWED,
                                format!(
                                    "mod \"{}\" in `{}` is hidden by the one in `{}`",
                                    candidate.manifest.id, candidate.root, first.root
                                ),
                            )
                            .at(sources, candidate.manifest.id_span),
                        ),
                    }
                }
            };
        let mut memory: Vec<FoundMod> = Vec::new();
        for memory_mod in &self.memory {
            let root = join_shown("mods", &memory_mod.dir);
            let manifest = load_manifest(
                &join_shown(&root, MANIFEST_FILE),
                memory_mod.manifest.clone(),
                sources,
                &self.limits,
                diagnostics,
            );
            let Some(manifest) = manifest else {
                continue;
            };
            if let Some(first) = memory.iter().find(|other| other.manifest.id == manifest.id) {
                diagnostics.push(
                    Diagnostic::new(
                        code::DUPLICATE_MOD,
                        format!(
                            "mod \"{}\" is declared by two directories: `{}` and `{root}`",
                            manifest.id, first.root
                        ),
                    )
                    .at(sources, manifest.id_span)
                    .note_at(
                        sources,
                        first.manifest.id_span,
                        "first declared here",
                    ),
                );
                continue;
            }
            memory.push(FoundMod {
                root,
                manifest,
                backing: ModBacking::Memory(memory_mod.files.clone()),
            });
        }
        add(memory, sources, diagnostics);
        for dir in &self.search {
            let candidates = discover_dir(dir, sources, &self.limits, diagnostics);
            add(candidates, sources, diagnostics);
        }
        found
    }

    fn run(
        self,
        selection: Selection,
        profile: Option<Profile>,
        mut sources: SourceMap,
        mut diagnostics: Diagnostics,
    ) -> LoadOutcome {
        let found = self.discover(&mut sources, &mut diagnostics);
        let (chosen, spans) = choose(
            &selection,
            &found,
            profile.as_ref(),
            &sources,
            &mut diagnostics,
        );
        let order = order_mods(
            &chosen,
            &found,
            spans.as_deref(),
            &sources,
            &mut diagnostics,
        );
        for &index in &order {
            let manifest = &found[index].manifest;
            if self
                .types
                .reserved_namespaces()
                .contains(&manifest.id.as_str())
            {
                diagnostics.push(
                    Diagnostic::new(
                        code::BAD_MOD_ID,
                        format!(
                            "the namespace \"{}\" is reserved for the engine",
                            manifest.id
                        ),
                    )
                    .at(&sources, manifest.id_span),
                );
            }
        }

        // Read and parse the data files of the chosen mods, in load order.
        let mut table = ModTable::default();
        let mut files: Vec<ParsedFile> = Vec::new();
        for &index in &order {
            let found_mod = &found[index];
            let mod_index = table.push(LoadedMod {
                manifest: found_mod.manifest.clone(),
                root: found_mod.root.clone(),
            });
            for (relative, text) in self.data_files(found_mod, &mut diagnostics) {
                let file = sources.add(join_shown(&found_mod.root, &relative), text);
                table.add_file(file, mod_index);
                let (tree, problems) = parse_json(file, sources.text(file), &self.limits);
                for problem in problems {
                    diagnostics.push(
                        Diagnostic::new(problem.code, problem.message)
                            .at(&sources, problem.span)
                            .suggest_opt(problem.suggestion),
                    );
                }
                if let Some(tree) = tree {
                    files.push(ParsedFile { mod_index, tree });
                }
            }
        }

        // Each type's declared defaults, as a file of the engine's own.
        let type_names = self.types.names();
        let mut defaults: BTreeMap<&'static str, Node> = BTreeMap::new();
        for (name, entry) in self.types.iter() {
            let text = entry.defaults();
            if text.trim().is_empty() {
                continue;
            }
            let file = sources.add(format!("<{ENGINE_NAMESPACE}>/defaults/{name}.json"), text);
            let (tree, problems) = parse_json(file, sources.text(file), &self.limits);
            for problem in problems {
                diagnostics.push(
                    Diagnostic::new(problem.code, problem.message)
                        .at(&sources, problem.span)
                        .note(format!("in the defaults the engine declares for `{name}`")),
                );
            }
            if let Some(tree) = tree {
                defaults.insert(name, tree);
            }
        }

        // Collect, fold and finish.
        let mut merge_cx = MergeCx {
            sources: &sources,
            mods: &table,
            type_names: &type_names,
            defaults: &defaults,
            diagnostics: &mut diagnostics,
        };
        let raws = collect(&files, &mut merge_cx);
        drop(files);
        let merged = merge(raws, &mut merge_cx);
        let mut finished = merged.finished;
        for (key, definition) in &finished {
            if definition.is_abstract && !merged.used_parents.contains(key) {
                diagnostics.push(
                    Diagnostic::new(
                        code::UNUSED_ABSTRACT,
                        format!(
                            "abstract {} \"{}\" is never copied from",
                            definition.type_name, definition.id
                        ),
                    )
                    .at(&sources, definition.id_span)
                    .in_def(Some(&DefLabel::new(
                        definition.type_name.clone(),
                        definition.id.clone(),
                    ))),
                );
            }
        }

        // First pass: find inline definitions and tags. Nothing is
        // resolved and nothing is reported.
        let no_ids: BTreeMap<&'static str, IdTable> = BTreeMap::new();
        let discover = LinkShared::new(&table, &no_ids, &[], true);
        let mut too_many = finished.len() > self.limits.max_definitions;
        let mut inline: BTreeMap<Key, Finished> = BTreeMap::new();
        let mut pending: Vec<Key> = Vec::new();
        if !too_many {
            for (key, definition) in &finished {
                if definition.is_abstract {
                    continue;
                }
                if let (Some(tree), Some(entry)) = (&definition.tree, self.types.get(&key.0)) {
                    let label = DefLabel::new(key.0.clone(), key.1.clone());
                    entry.discover(tree, &DeCx::new(&sources, Some(label), Some(&discover)));
                }
            }
        }
        loop {
            let found_inline = std::mem::take(&mut *discover.found_inline.borrow_mut());
            for found_one in found_inline {
                let key = (found_one.type_name.to_owned(), found_one.id.clone());
                if !self.types.contains(found_one.type_name) {
                    continue;
                }
                let id_span = found_one
                    .node
                    .get("id")
                    .map_or(first_char(found_one.node.span), |node| node.span);
                let first = finished
                    .get(&key)
                    .or_else(|| inline.get(&key))
                    .map(|first| first.id_span);
                if let Some(first) = first {
                    diagnostics.push(
                        Diagnostic::new(
                            code::DUPLICATE_ID,
                            format!("{} \"{}\" is already defined", key.0, key.1),
                        )
                        .at(&sources, id_span)
                        .in_def(Some(&DefLabel::new(key.0.clone(), key.1.clone())))
                        .note_at(&sources, first, "first defined here"),
                    );
                    continue;
                }
                inline.insert(
                    key.clone(),
                    Finished {
                        type_name: key.0.clone(),
                        id: key.1.clone(),
                        id_span,
                        tree: Some(found_one.node),
                        is_abstract: false,
                        obsolete: false,
                        inline: true,
                    },
                );
                pending.push(key);
            }
            if finished.len() + inline.len() > self.limits.max_definitions {
                too_many = true;
                break;
            }
            let Some(key) = pending.pop() else {
                break;
            };
            if let (Some(definition), Some(entry)) = (inline.get(&key), self.types.get(&key.0))
                && let Some(tree) = &definition.tree
            {
                let label = DefLabel::new(key.0.clone(), key.1.clone());
                entry.discover(tree, &DeCx::new(&sources, Some(label), Some(&discover)));
            }
        }
        if too_many {
            diagnostics.push(Diagnostic::new(
                code::TOO_MANY_DEFINITIONS,
                format!(
                    "the mods hold more than {} definitions, which is the limit",
                    self.limits.max_definitions
                ),
            ));
        }
        let tag_names: Vec<String> = discover.found_tags.borrow().iter().cloned().collect();
        drop(discover);
        finished.append(&mut inline);

        // Link: every id of every type gets its dense number before any
        // definition is read (04 §7.2). Builtins first, then sorted.
        let mut ids: BTreeMap<&'static str, IdTable> = BTreeMap::new();
        for (name, entry) in self.types.iter() {
            let mut id_table = IdTable::default();
            for builtin in entry.builtin_names() {
                id_table.names.push(format!("{ENGINE_NAMESPACE}:{builtin}"));
            }
            ids.insert(name, id_table);
        }
        for (key, definition) in &finished {
            let Some(id_table) = ids.get_mut(key.0.as_str()) else {
                continue;
            };
            if definition.is_abstract {
                id_table.abstracts.insert(key.1.clone());
            } else {
                id_table.names.push(key.1.clone());
            }
        }
        for id_table in ids.values_mut() {
            id_table.index = id_table
                .names
                .iter()
                .enumerate()
                .map(|(index, name)| (name.clone(), crate::source::to_u32(index)))
                .collect();
        }

        // Typed load, type by type in order of name.
        let tags = TagTable::new(tag_names);
        let link = LinkShared::new(&table, &ids, tags.names(), false);
        let env = StageEnv {
            sources: &sources,
            link: &link,
            lenient: self.lenient,
        };
        let mut staged: BTreeMap<&'static str, Box<dyn AnyStaged>> = BTreeMap::new();
        if !too_many {
            for (name, entry) in self.types.iter() {
                let builtins = entry.builtin_names().len();
                let id_table = &ids[name];
                let mut entries: Vec<StageEntry<'_>> = Vec::with_capacity(id_table.names.len());
                for (index, id) in id_table.names.iter().enumerate() {
                    if index < builtins {
                        entries.push(StageEntry {
                            id,
                            tree: None,
                            id_span: None,
                            obsolete: false,
                            builtin: true,
                        });
                        continue;
                    }
                    let definition = &finished[&(name.to_owned(), id.clone())];
                    entries.push(StageEntry {
                        id,
                        tree: definition.tree.as_ref(),
                        id_span: Some(definition.id_span),
                        obsolete: definition.obsolete,
                        builtin: false,
                    });
                }
                staged.insert(name, entry.stage(&entries, &env, &mut diagnostics));
            }
        }

        // Experimental features (04 §8.4).
        let mut reported: BTreeSet<(String, Span)> = BTreeSet::new();
        for usage in link.feature_uses.borrow().iter() {
            let Some(manifest) = table.manifest_of(usage.span.file) else {
                continue;
            };
            if manifest.experimental.contains(&usage.feature)
                || !reported.insert((usage.feature.clone(), usage.span))
            {
                continue;
            }
            diagnostics.push(
                Diagnostic::new(
                    code::EXPERIMENTAL_NOT_ENABLED,
                    format!(
                        "this uses the experimental feature \"{}\", which mod \"{}\" has not opted in to",
                        usage.feature, manifest.id
                    ),
                )
                .at(&sources, usage.span)
                .in_def(usage.label.as_ref())
                .with_path(usage.path.clone())
                .suggest(format!(
                    "add \"{}\" to `experimental` in the mod.json of \"{}\"",
                    usage.feature, manifest.id
                )),
            );
        }
        let mut experimental: BTreeSet<String> = BTreeSet::new();
        for loaded in &table.mods {
            for feature in &loaded.manifest.experimental {
                experimental.insert(feature.clone());
                diagnostics.push(
                    Diagnostic::new(
                        code::EXPERIMENTAL_IN_USE,
                        format!(
                            "mod \"{}\" relies on the experimental feature \"{feature}\", which may change or be removed",
                            loaded.manifest.id
                        ),
                    )
                    .at(&sources, loaded.manifest.id_span),
                );
            }
        }
        drop(link);

        // Semantic and cross stages, for what survived (04 §8.1).
        {
            let view = DefsView {
                staged: &staged,
                tags: &tags,
            };
            for one in staged.values() {
                one.validate(&view, &sources, &mut diagnostics);
            }
            for check in self.types.cross_checks() {
                let mut cx = CrossCx {
                    view: &view,
                    sources: &sources,
                    diagnostics: &mut diagnostics,
                };
                check(&mut cx);
            }
        }

        diagnostics.sort();
        let fingerprint = fingerprint(&finished, &table);
        let usable = !too_many && (self.lenient || !diagnostics.has_errors());
        let sources = Arc::new(sources);
        let mut defs = None;
        if usable {
            let mut registries = Vec::with_capacity(staged.len());
            let mut complete = true;
            for (name, one) in staged {
                match one.into_registry() {
                    Some((type_id, registry)) => registries.push((name, type_id, registry)),
                    None => complete = false,
                }
            }
            if complete {
                let trees = finished
                    .into_values()
                    .filter_map(|definition| {
                        Some(FinishedTree {
                            type_name: definition.type_name,
                            id: definition.id,
                            tree: definition.tree?,
                            is_abstract: definition.is_abstract,
                            inline: definition.inline,
                        })
                    })
                    .collect();
                defs = Some(Arc::new(Defs::assemble(DefsParts {
                    registries,
                    tags,
                    fingerprint,
                    sources: Arc::clone(&sources),
                    mods: table
                        .mods
                        .iter()
                        .map(|loaded| LoadedModInfo {
                            manifest: loaded.manifest.clone(),
                            root: loaded.root.clone(),
                        })
                        .collect(),
                    profile,
                    trees,
                    experimental: experimental.into_iter().collect(),
                })));
            }
        }
        LoadOutcome {
            defs,
            diagnostics,
            sources,
        }
    }

    /// The data files of a mod: every `*.json` under its `data/`, as
    /// paths relative to the mod, sorted, with their text.
    fn data_files(&self, found: &FoundMod, diagnostics: &mut Diagnostics) -> Vec<(String, String)> {
        match &found.backing {
            ModBacking::Memory(files) => {
                let prefix = format!("{DATA_DIR}/");
                let mut files: Vec<(String, String)> = files
                    .iter()
                    .filter(|(path, _)| path.starts_with(&prefix) && path.ends_with(".json"))
                    .cloned()
                    .collect();
                files.sort();
                files
            }
            ModBacking::Disk(root) => {
                let data = root.join(DATA_DIR);
                if !data.is_dir() {
                    return Vec::new();
                }
                let relative = match json_files_under(&data, &self.limits) {
                    Ok(relative) => relative,
                    Err((path, error)) => {
                        diagnostics.push(Diagnostic::new(
                            code::UNREADABLE_FILE,
                            format!("cannot read `{path}`: {error}"),
                        ));
                        return Vec::new();
                    }
                };
                let mut files = Vec::with_capacity(relative.len());
                for relative in relative {
                    let path = data.join(&relative);
                    let shown = format!("{DATA_DIR}/{relative}");
                    match read_text(&path, &self.limits) {
                        Ok(text) => files.push((shown, text)),
                        Err(error) => {
                            let code = if error.contains("the limit is") {
                                code::FILE_TOO_LARGE
                            } else {
                                code::UNREADABLE_FILE
                            };
                            diagnostics.push(Diagnostic::new(
                                code,
                                format!(
                                    "cannot read `{}`: {error}",
                                    join_shown(&found.root, &shown)
                                ),
                            ));
                        }
                    }
                }
                files
            }
        }
    }
}

fn first_char(span: Span) -> Span {
    Span::new(span.file, span.start, span.start + 1)
}

/// Works out which of the found mods to load, as indices into `found` in
/// profile order.
fn choose(
    selection: &Selection,
    found: &[FoundMod],
    profile: Option<&Profile>,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
) -> (Vec<usize>, Option<Vec<Span>>) {
    let index_of = |id: &str| found.iter().position(|one| one.manifest.id == id);
    let known = || found.iter().map(|one| one.manifest.id.as_str());
    match selection {
        Selection::All => {
            let mut all: Vec<usize> = (0..found.len()).collect();
            all.sort_by(|&a, &b| found[a].manifest.id.cmp(&found[b].manifest.id));
            (all, None)
        }
        Selection::Listed(ids, spans) => {
            let mut chosen = Vec::new();
            let mut chosen_spans = Vec::new();
            for (position, id) in ids.iter().enumerate() {
                let span = spans
                    .as_ref()
                    .and_then(|spans| spans.get(position))
                    .copied();
                match index_of(id) {
                    Some(index) => {
                        if !chosen.contains(&index) {
                            chosen.push(index);
                            if let Some(span) = span {
                                chosen_spans.push(span);
                            }
                        }
                    }
                    None => {
                        let what = match profile {
                            Some(profile) => format!("profile \"{}\" lists", profile.id),
                            None => "the load asks for".to_owned(),
                        };
                        let mut diagnostic = Diagnostic::new(
                            code::MISSING_MOD,
                            format!("{what} mod \"{id}\", which was not found"),
                        )
                        .suggest_opt(did_you_mean(closest(id, known())));
                        if let Some(span) = span {
                            diagnostic = diagnostic.at(sources, span);
                        }
                        diagnostics.push(diagnostic);
                    }
                }
            }
            let spans =
                (spans.is_some() && chosen_spans.len() == chosen.len()).then_some(chosen_spans);
            (chosen, spans)
        }
        Selection::WithRequirements(id) => {
            let Some(start) = index_of(id) else {
                diagnostics.push(
                    Diagnostic::new(code::MISSING_MOD, format!("mod \"{id}\" was not found"))
                        .suggest_opt(did_you_mean(closest(id, known()))),
                );
                return (Vec::new(), None);
            };
            // The mod, then what it requires, breadth first. A missing
            // requirement is reported when the mods are ordered.
            let mut chosen = vec![start];
            let mut at = 0;
            while at < chosen.len() {
                let manifest = &found[chosen[at]].manifest;
                for requirement in &manifest.requires {
                    if let Some(index) = index_of(&requirement.id)
                        && !chosen.contains(&index)
                    {
                        chosen.push(index);
                    }
                }
                at += 1;
            }
            (chosen, None)
        }
    }
}

/// The content fingerprint: a hash of every finished tree in canonical
/// form (04 §9).
///
/// Canonical form is the tree's values in order, without positions, so
/// that moving a definition within a file or between files of one mod
/// changes nothing. Key order is kept because it can carry meaning
/// (04 §5). A string also carries the namespace of the mod that wrote it,
/// because that decides what a bare reference in it means (04 §6.1).
fn fingerprint(finished: &BTreeMap<Key, Finished>, mods: &ModTable) -> Hash128 {
    let mut hasher = StateHasher::new();
    let live: Vec<&Finished> = finished
        .values()
        .filter(|definition| !definition.is_abstract)
        .collect();
    hasher.write_len(live.len());
    for definition in live {
        hasher.write_str(&definition.type_name);
        hasher.write_str(&definition.id);
        hasher.write_bool(definition.obsolete);
        match &definition.tree {
            Some(tree) => {
                hasher.write_bool(true);
                hash_tree(tree, mods, &mut hasher);
            }
            None => hasher.write_bool(false),
        }
    }
    hasher.finish()
}

fn hash_tree(node: &Node, mods: &ModTable, hasher: &mut StateHasher) {
    match &node.kind {
        NodeKind::Null => hasher.write_u8(0),
        NodeKind::Bool(value) => {
            hasher.write_u8(1);
            hasher.write_bool(*value);
        }
        NodeKind::Number(Num::Int(value)) => {
            hasher.write_u8(2);
            hasher.write_i64(*value);
        }
        NodeKind::Number(Num::UInt(value)) => {
            hasher.write_u8(3);
            hasher.write_u64(*value);
        }
        NodeKind::Number(Num::Float(value)) => {
            hasher.write_u8(4);
            hasher.write_u64(value.to_bits());
        }
        NodeKind::Quantity { value, dim } => {
            hasher.write_u8(5);
            hasher.write_u64(value.to_bits());
            hasher.write_str(dim.si_unit());
        }
        NodeKind::String(text) => {
            hasher.write_u8(6);
            hasher.write_str(text);
            let namespace = mods
                .manifest_of(node.span.file)
                .map_or("", |manifest| manifest.id.as_str());
            hasher.write_str(namespace);
        }
        NodeKind::Array(items) => {
            hasher.write_u8(7);
            hasher.write_len(items.len());
            for item in items {
                hash_tree(item, mods, hasher);
            }
        }
        NodeKind::Object(members) => {
            hasher.write_u8(8);
            hasher.write_len(members.len());
            for member in members {
                hasher.write_str(&member.key);
                hash_tree(&member.value, mods, hasher);
            }
        }
    }
}
