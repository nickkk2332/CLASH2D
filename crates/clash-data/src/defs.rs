//! Definition types, registries and the finished set of definitions
//! (04 §9).

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use clash_core::{DenseId, Hash128, LookupMap};
use serde::de::DeserializeOwned;

use crate::de::{DeCx, FeatureUse, LinkShared, Resolved, read_def, resolve_id};
use crate::diag::{Code, DefLabel, Diagnostic, Diagnostics, Severity, code};
use crate::expr::{CompileCx, CompileEnv, ExprDefs, ExprRegistry};
use crate::fields::{Located, Tag};
use crate::json::Node;
use crate::mods::{ModManifest, Profile};
use crate::source::{SourceMap, Span, to_u32};

/// A definition type: a Rust struct that mods fill in from JSON.
///
/// The type lives in the crate that gives it meaning (01 §3). This crate
/// drives the loading without knowing what it is.
///
/// # Registering a definition type
///
/// ```
/// use clash_core::{define_id, Real};
/// use clash_data::{Def, DefTypes, Energy, Loader, Ref, TagSet, ValidateCx};
///
/// define_id! {
///     /// A material.
///     pub struct MaterialId(u16);
///     /// A block.
///     pub struct BlockId(u16);
/// }
///
/// /// What things are made of.
/// #[derive(Debug, serde::Deserialize)]
/// pub struct MaterialDef {
///     pub density: Real,
///     #[serde(default)]
///     pub burns_at: Option<Energy>,
///     #[serde(default)]
///     pub tags: TagSet,
/// }
///
/// impl Def for MaterialDef {
///     type Id = MaterialId;
///     const TYPE: &'static str = "material";
///     const DOC: &'static str = "What things are made of.";
///
///     fn validate(&self, cx: &mut ValidateCx<'_>) {
///         if self.density <= Real::ZERO {
///             cx.error("`density` must be above zero");
///         }
///     }
/// }
///
/// /// What fills a cell.
/// #[derive(Debug, serde::Deserialize)]
/// pub struct BlockDef {
///     pub material: Ref<MaterialDef>,
/// }
///
/// impl Def for BlockDef {
///     type Id = BlockId;
///     const TYPE: &'static str = "block";
/// }
///
/// let mut types = DefTypes::new();
/// types.register::<MaterialDef>();
/// types.register::<BlockDef>();
///
/// let outcome = Loader::new(&types)
///     .memory_mod(
///         "core",
///         r#"{ "id": "core" }"#,
///         &[(
///             "data/world.json",
///             r#"[
///               { "type": "material", "id": "granite", "density": 2600, "tags": ["mineral"] },
///               { "type": "block", "id": "solid", "material": "granite" }
///             ]"#,
///         )],
///     )
///     .load_all();
/// assert_eq!(outcome.diagnostics.render(), "");
/// let defs = outcome.defs.unwrap();
///
/// let blocks = defs.registry::<BlockDef>();
/// let solid = blocks.lookup("core:solid").unwrap();
/// let granite = blocks.get(solid).material.id();
/// assert_eq!(defs.registry::<MaterialDef>().name(granite), "core:granite");
/// assert_eq!(defs.get::<MaterialDef>(granite).density, Real::of(2600.0));
/// ```
pub trait Def: DeserializeOwned + Send + Sync + 'static {
    /// The dense id of a definition of this type, made with
    /// `clash_core::define_id!`. Its width is the most definitions the
    /// type can hold.
    type Id: DenseId + Send + Sync + 'static;

    /// The JSON name: the value of `"type"` in a data file.
    const TYPE: &'static str;

    /// What the type is, for the generated reference.
    const DOC: &'static str = "";

    /// Default values of fields, as the text of a JSON object, for
    /// `scale` and `add` on a field a definition leaves out (04 §6.1).
    /// Only fields a mod might scale need be listed. Empty for none.
    const DEFAULTS: &'static str = "";

    /// The experimental feature the whole type is part of, if any
    /// (04 §8.4). A mod that defines one must opt in to the feature.
    const EXPERIMENTAL: Option<&'static str> = None;

    /// Called once on each definition after it is read, to resolve what
    /// plain reading cannot: a reference written in a syntax of the
    /// type's own, kept until now as a [`Located`] string.
    fn finish(&mut self, _cx: &mut FinishCx<'_>) {}

    /// Called once on each definition after every definition has been
    /// read, to compile its expression fields (04 §7.3): each
    /// [`Expr`](crate::Expr), [`Cond`](crate::Cond) and
    /// [`Text`](crate::Text). The type says what is in scope in each
    /// field with a [`Scope`](crate::Scope) and hands the field to
    /// [`CompileCx::compile`]:
    ///
    /// ```ignore
    /// fn compile(&mut self, cx: &mut CompileCx<'_>) {
    ///     let scope = Scope::new().subject("self", "entity").subject("here", "loc");
    ///     cx.compile(&mut self.when, &scope);
    ///     cx.compile(&mut self.energy, &scope);
    ///     cx.compile(&mut self.log, &scope);
    /// }
    /// ```
    ///
    /// A field that is not handed over is never compiled: it evaluates
    /// to null, and a text renders as it was written.
    fn compile(&mut self, _cx: &mut CompileCx<'_>) {}

    /// The type's own checks: ranges, fields that exclude each other,
    /// limits (04 §8.1, the semantic stage). Runs after every definition
    /// of every type has been read, so it may look at the others.
    fn validate(&self, _cx: &mut ValidateCx<'_>) {}

    /// An inert stand-in for a definition that could not be loaded, used
    /// by lenient loading (04 §8.3). `None` means the type has none, and
    /// lenient loading stops at a broken definition of it.
    fn placeholder() -> Option<Self> {
        None
    }

    /// Definitions the engine supplies itself, as names in the reserved
    /// `engine` namespace. They take the first dense ids, in the order
    /// given: an element type returns `[("none", ...)]` so that dense id
    /// 0 is `engine:none` (04 §4.4).
    fn builtins() -> Vec<(&'static str, Self)> {
        Vec::new()
    }
}

/// The definitions of one type, addressed by dense id (04 §9).
pub struct Registry<T: Def> {
    items: Vec<T>,
    names: Vec<String>,
    index: LookupMap<String, u32>,
    placeholders: Vec<bool>,
    obsolete: Vec<bool>,
}

impl<T: Def> Registry<T> {
    /// The definition with a dense id.
    ///
    /// # Panics
    ///
    /// If `id` is not an id of this registry. Ids come from this
    /// registry's own `lookup` and `iter` and from `Ref`s loaded with it;
    /// one from another `Defs` is an engine bug (04 M5).
    pub fn get(&self, id: T::Id) -> &T {
        &self.items[id.index()]
    }

    /// The definition with a dense id, or `None` if the id is not one of
    /// this registry's.
    pub fn try_get(&self, id: T::Id) -> Option<&T> {
        self.items.get(id.index())
    }

    /// The dense id of a qualified id, `namespace:name`.
    pub fn lookup(&self, qualified: &str) -> Option<T::Id> {
        self.index
            .get(qualified)
            .and_then(|&index| T::Id::from_index(index as usize))
    }

    /// The qualified id of a dense id, or `""` if the id is not one of
    /// this registry's.
    pub fn name(&self, id: T::Id) -> &str {
        self.names.get(id.index()).map_or("", String::as_str)
    }

    /// Every definition, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (T::Id, &T)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| T::Id::from_index(index).map(|id| (id, item)))
    }

    /// Every dense id, in order.
    pub fn ids(&self) -> impl Iterator<Item = T::Id> + '_ {
        (0..self.items.len()).filter_map(T::Id::from_index)
    }

    /// The qualified ids, in id order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// How many definitions there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether a definition is a placeholder for one that could not be
    /// loaded (04 §8.3).
    pub fn is_placeholder(&self, id: T::Id) -> bool {
        self.placeholders.get(id.index()).copied().unwrap_or(false)
    }

    /// Whether a definition is marked `"obsolete": true` (04 §12.2).
    pub fn is_obsolete(&self, id: T::Id) -> bool {
        self.obsolete.get(id.index()).copied().unwrap_or(false)
    }
}

/// The tags of one [`Defs`], numbered in sorted order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagTable {
    names: Vec<String>,
}

impl TagTable {
    pub(crate) fn new(names: Vec<String>) -> TagTable {
        TagTable { names }
    }

    /// The tag for a string, if any definition carries it.
    pub fn lookup(&self, name: &str) -> Option<Tag> {
        self.names
            .binary_search_by(|known| known.as_str().cmp(name))
            .ok()
            .map(|index| Tag::from_index(to_u32(index)))
    }

    /// The string of a tag, or `""` for a tag from another table.
    pub fn name(&self, tag: Tag) -> &str {
        self.names.get(tag.index()).map_or("", String::as_str)
    }

    /// The strings, in tag order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// How many tags there are.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// What the generated reference says about a registered definition type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeInfo {
    /// The JSON name.
    pub name: &'static str,
    /// What the type is.
    pub doc: &'static str,
    /// The experimental feature it is part of, if any.
    pub experimental: Option<&'static str>,
}

/// One row of what the loader stages for a type, in dense id order.
pub(crate) struct StageEntry<'a> {
    pub(crate) id: &'a str,
    /// The finished tree; `None` for a builtin or a definition that
    /// failed to merge.
    pub(crate) tree: Option<&'a Node>,
    pub(crate) id_span: Option<Span>,
    pub(crate) obsolete: bool,
    pub(crate) builtin: bool,
}

/// What staging a type needs from the load.
pub(crate) struct StageEnv<'a> {
    pub(crate) sources: &'a SourceMap,
    pub(crate) link: &'a LinkShared<'a>,
    pub(crate) lenient: bool,
}

/// A definition type with its Rust type erased.
pub(crate) trait AnyType: Send + Sync {
    fn info(&self) -> TypeInfo;
    fn defaults(&self) -> &'static str;
    fn builtin_names(&self) -> Vec<&'static str>;
    /// Reads a tree only to find the inline definitions and tags in it.
    fn discover(&self, node: &Node, cx: &DeCx<'_>);
    /// Reads every definition of the type.
    fn stage(
        &self,
        entries: &[StageEntry<'_>],
        env: &StageEnv<'_>,
        diagnostics: &mut Diagnostics,
    ) -> Box<dyn AnyStaged>;
}

/// The definitions of one type while loading: some may have failed.
pub(crate) trait AnyStaged {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Compiles the expression fields of every definition that loaded
    /// (04 §8.1, the compile stage).
    fn compile(&mut self, env: &CompileEnv<'_>, lenient: bool, diagnostics: &mut Diagnostics);
    fn validate(&self, view: &DefsView<'_>, sources: &SourceMap, diagnostics: &mut Diagnostics);
    /// Turns into a registry, if every definition is there.
    fn into_registry(self: Box<Self>) -> Option<(TypeId, Box<dyn AnyRegistry>)>;
}

/// A finished registry with its Rust type erased.
pub(crate) trait AnyRegistry: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn names(&self) -> &[String];
}

impl<T: Def> AnyRegistry for Registry<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn names(&self) -> &[String] {
        &self.names
    }
}

struct TypeEntry<T: Def>(PhantomData<fn() -> T>);

struct Staged<T: Def> {
    items: Vec<Option<T>>,
    names: Vec<String>,
    index: BTreeMap<String, usize>,
    spans: Vec<Option<Span>>,
    placeholders: Vec<bool>,
    obsolete: Vec<bool>,
    /// The type has more definitions than its id can count.
    overflow: bool,
}

/// The definitions of one type while loading, for a stage that knows
/// the type: each definition if it loaded, its qualified id, where its
/// id was written (`None` for one of the engine's own) and whether it
/// is a placeholder.
pub(crate) struct StagedDefs<'a, T: Def> {
    pub(crate) items: &'a mut [Option<T>],
    pub(crate) names: &'a [String],
    pub(crate) spans: &'a [Option<Span>],
    pub(crate) placeholders: &'a mut [bool],
}

/// The staged definitions of type `T`, if the type is registered.
pub(crate) fn staged_defs<'a, T: Def>(
    staged: &'a mut BTreeMap<&'static str, Box<dyn AnyStaged>>,
) -> Option<StagedDefs<'a, T>> {
    let staged = staged
        .get_mut(T::TYPE)?
        .as_any_mut()
        .downcast_mut::<Staged<T>>()?;
    Some(StagedDefs {
        items: &mut staged.items,
        names: &staged.names,
        spans: &staged.spans,
        placeholders: &mut staged.placeholders,
    })
}

/// What a lenient load puts in place of a broken definition (04 §8.3):
/// the type's placeholder, with a note, or nothing, with an error that
/// says the type has none.
pub(crate) fn stand_in_for<T: Def>(
    label: &DefLabel,
    id_span: Option<Span>,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
) -> Option<T> {
    let stand_in = T::placeholder();
    let mut diagnostic = match &stand_in {
        Some(_) => Diagnostic::new(
            code::PLACEHOLDER,
            format!("{label} is broken and was replaced by a placeholder"),
        ),
        None => Diagnostic::new(
            code::NO_PLACEHOLDER,
            format!(
                "{label} is broken, and the type `{}` has no placeholder to stand in for it",
                T::TYPE
            ),
        ),
    }
    .in_def(Some(label));
    if let Some(span) = id_span {
        diagnostic = diagnostic.at(sources, span);
    }
    diagnostics.push(diagnostic);
    stand_in
}

impl<T: Def> AnyType for TypeEntry<T> {
    fn info(&self) -> TypeInfo {
        TypeInfo {
            name: T::TYPE,
            doc: T::DOC,
            experimental: T::EXPERIMENTAL,
        }
    }

    fn defaults(&self) -> &'static str {
        T::DEFAULTS
    }

    fn builtin_names(&self) -> Vec<&'static str> {
        T::builtins().into_iter().map(|(name, _)| name).collect()
    }

    fn discover(&self, node: &Node, cx: &DeCx<'_>) {
        let _ = read_def::<T>(node, cx);
    }

    fn stage(
        &self,
        entries: &[StageEntry<'_>],
        env: &StageEnv<'_>,
        diagnostics: &mut Diagnostics,
    ) -> Box<dyn AnyStaged> {
        let mut staged = Staged::<T> {
            items: Vec::with_capacity(entries.len()),
            names: Vec::with_capacity(entries.len()),
            index: entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.id.to_owned(), index))
                .collect(),
            spans: Vec::with_capacity(entries.len()),
            placeholders: Vec::with_capacity(entries.len()),
            obsolete: Vec::with_capacity(entries.len()),
            overflow: false,
        };
        if let Some(last) = entries.len().checked_sub(1)
            && T::Id::from_index(last).is_none()
        {
            staged.overflow = true;
            let mut diagnostic = Diagnostic::new(
                code::TOO_MANY_DEFINITIONS,
                format!(
                    "there are {} definitions of type `{}`, more than its id can count",
                    entries.len(),
                    T::TYPE
                ),
            );
            if let Some(span) = entries.iter().rev().find_map(|entry| entry.id_span) {
                diagnostic = diagnostic.at(env.sources, span);
            }
            diagnostics.push(diagnostic);
        }
        let mut builtins = T::builtins().into_iter();
        for entry in entries {
            let label = DefLabel::new(T::TYPE, entry.id);
            let mut value = if entry.builtin {
                builtins.next().map(|(_, value)| value)
            } else {
                match entry.tree {
                    Some(tree) => {
                        let cx = DeCx::new(env.sources, Some(label.clone()), Some(env.link));
                        let outcome = read_def::<T>(tree, &cx);
                        diagnostics.extend(outcome.diagnostics);
                        outcome.value
                    }
                    None => None,
                }
            };
            if !entry.builtin
                && let Some(read) = &mut value
            {
                let mut cx = FinishCx {
                    sources: env.sources,
                    link: env.link,
                    label: &label,
                    id_span: entry.id_span,
                    diagnostics: Vec::new(),
                    failed: false,
                };
                read.finish(&mut cx);
                let failed = cx.failed;
                diagnostics.extend(cx.diagnostics);
                if failed {
                    value = None;
                }
            }
            if !entry.builtin
                && let (Some(feature), Some(span)) = (T::EXPERIMENTAL, entry.id_span)
            {
                env.link.feature_uses.borrow_mut().insert(FeatureUse {
                    feature: feature.to_owned(),
                    span,
                    label: Some(label.clone()),
                    path: String::new(),
                });
            }
            let mut placeholder = false;
            if value.is_none() && env.lenient && !entry.builtin {
                value = stand_in_for::<T>(&label, entry.id_span, env.sources, diagnostics);
                placeholder = value.is_some();
            }
            staged.items.push(value);
            staged.names.push(entry.id.to_owned());
            staged.spans.push(entry.id_span);
            staged.placeholders.push(placeholder);
            staged.obsolete.push(entry.obsolete);
        }
        Box::new(staged)
    }
}

impl<T: Def> AnyStaged for Staged<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn compile(&mut self, env: &CompileEnv<'_>, lenient: bool, diagnostics: &mut Diagnostics) {
        for (index, item) in self.items.iter_mut().enumerate() {
            let Some(value) = item else {
                continue;
            };
            // A placeholder is inert; a builtin is the engine's own.
            if self.placeholders[index] || self.spans[index].is_none() {
                continue;
            }
            let label = DefLabel::new(T::TYPE, self.names[index].clone());
            let mut cx = CompileCx::new(env, &label, self.spans[index]);
            value.compile(&mut cx);
            let failed = cx.failed();
            diagnostics.extend(cx.into_diagnostics());
            if failed {
                *item = None;
                if lenient {
                    *item = stand_in_for::<T>(&label, self.spans[index], env.sources, diagnostics);
                    self.placeholders[index] = item.is_some();
                }
            }
        }
    }

    fn validate(&self, view: &DefsView<'_>, sources: &SourceMap, diagnostics: &mut Diagnostics) {
        for (index, item) in self.items.iter().enumerate() {
            let Some(item) = item else {
                continue;
            };
            // A placeholder is inert; a builtin is the engine's own.
            if self.placeholders[index] || self.spans[index].is_none() {
                continue;
            }
            let label = DefLabel::new(T::TYPE, self.names[index].clone());
            let mut cx = ValidateCx {
                view,
                sources,
                label: &label,
                id_span: self.spans[index],
                diagnostics,
            };
            item.validate(&mut cx);
        }
    }

    fn into_registry(self: Box<Self>) -> Option<(TypeId, Box<dyn AnyRegistry>)> {
        if self.overflow {
            return None;
        }
        let items: Vec<T> = self.items.into_iter().collect::<Option<Vec<T>>>()?;
        let index: LookupMap<String, u32> = self
            .names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.clone(), to_u32(index)))
            .collect();
        let registry = Registry::<T> {
            items,
            names: self.names,
            index,
            placeholders: self.placeholders,
            obsolete: self.obsolete,
        };
        Some((TypeId::of::<T>(), Box::new(registry)))
    }
}

/// A whole-set check (04 §8.1, the cross stage): it sees every
/// definition of every type and reports what is wrong between them.
pub type CrossCheck = fn(&mut CrossCx<'_>);

/// A step that adds to the expression registry what a crate's own
/// definitions declare, such as the events mods define. It runs after
/// every definition has been read and before any expression is
/// compiled.
pub type RegistryStep = fn(&mut RegistryCx<'_>);

/// What a [`RegistryStep`] works with.
pub struct RegistryCx<'a> {
    pub(crate) view: &'a DefsView<'a>,
    pub(crate) registry: &'a mut ExprRegistry,
    pub(crate) sources: &'a SourceMap,
    pub(crate) diagnostics: &'a mut Diagnostics,
}

impl<'a> RegistryCx<'a> {
    /// Every definition that loaded.
    pub fn defs(&self) -> &'a DefsView<'a> {
        self.view
    }

    /// The registry expressions will be compiled against: the host's,
    /// with the definition ids of the mods being loaded. Add to it.
    pub fn registry(&mut self) -> &mut ExprRegistry {
        self.registry
    }

    /// The files of this load, for positions.
    pub fn sources(&self) -> &'a SourceMap {
        self.sources
    }

    /// Reports a diagnostic.
    pub fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }
}

/// The definition types the engine knows.
///
/// Each crate that owns definition types offers one function that
/// registers them all; the application and the tools call each crate's
/// in turn and hand the result to a [`Loader`](crate::Loader).
#[derive(Default)]
pub struct DefTypes {
    types: BTreeMap<&'static str, Box<dyn AnyType>>,
    cross_checks: Vec<CrossCheck>,
    registry_steps: Vec<RegistryStep>,
    reserved: Vec<&'static str>,
}

impl DefTypes {
    /// No types.
    pub fn new() -> DefTypes {
        DefTypes::default()
    }

    /// Registers a definition type under its JSON name.
    ///
    /// # Panics
    ///
    /// If another type is already registered under the same JSON name:
    /// that is a mistake in the engine, not in a mod.
    pub fn register<T: Def>(&mut self) -> &mut DefTypes {
        let previous = self
            .types
            .insert(T::TYPE, Box::new(TypeEntry::<T>(PhantomData)));
        assert!(
            previous.is_none(),
            "definition type `{}` is registered twice",
            T::TYPE
        );
        self
    }

    /// Adds a whole-set check, run after every definition has passed its
    /// own `validate`.
    pub fn add_cross_check(&mut self, check: CrossCheck) -> &mut DefTypes {
        self.cross_checks.push(check);
        self
    }

    /// Adds a step that fills the expression registry from definitions,
    /// run before any expression is compiled (04 §7.3). The loader adds
    /// definition ids, `function` definitions and `variable` declarations
    /// itself; a crate whose definitions declare something else that
    /// expressions name, such as events, adds it with a step.
    pub fn add_registry_step(&mut self, step: RegistryStep) -> &mut DefTypes {
        self.registry_steps.push(step);
        self
    }

    /// Reserves a namespace for the engine besides `engine` itself: the
    /// engine's settings live in namespaces named after their subsystem
    /// (04 §10). No mod may have a reserved namespace as its id.
    pub fn reserve_namespace(&mut self, namespace: &'static str) -> &mut DefTypes {
        if !self.reserved.contains(&namespace) {
            self.reserved.push(namespace);
        }
        self
    }

    /// The namespaces reserved with
    /// [`reserve_namespace`](DefTypes::reserve_namespace).
    pub fn reserved_namespaces(&self) -> &[&'static str] {
        &self.reserved
    }

    /// Whether a JSON name is registered.
    pub fn contains(&self, type_name: &str) -> bool {
        self.types.contains_key(type_name)
    }

    /// The registered types, in order of JSON name.
    pub fn infos(&self) -> Vec<TypeInfo> {
        self.types.values().map(|entry| entry.info()).collect()
    }

    /// The registered JSON names, sorted.
    pub fn names(&self) -> Vec<&'static str> {
        self.types.keys().copied().collect()
    }

    pub(crate) fn get(&self, type_name: &str) -> Option<&dyn AnyType> {
        self.types.get(type_name).map(|entry| &**entry)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'static str, &dyn AnyType)> {
        self.types.iter().map(|(name, entry)| (*name, &**entry))
    }

    pub(crate) fn cross_checks(&self) -> &[CrossCheck] {
        &self.cross_checks
    }

    pub(crate) fn registry_steps(&self) -> &[RegistryStep] {
        &self.registry_steps
    }
}

/// Every definition read so far, for checks that look across them.
///
/// Unlike [`Defs`], lookups here can fail: a definition that did not
/// survive an earlier stage is absent (04 §8.1).
pub struct DefsView<'a> {
    pub(crate) staged: &'a BTreeMap<&'static str, Box<dyn AnyStaged>>,
    pub(crate) tags: &'a TagTable,
    /// The registry expressions were compiled against, once they have
    /// been: in the semantic and cross stages, not in a registry step.
    pub(crate) exprs: Option<&'a crate::expr::ExprRegistry>,
}

impl<'a> DefsView<'a> {
    fn staged<T: Def>(&self) -> Option<&'a Staged<T>> {
        self.staged
            .get(T::TYPE)
            .and_then(|staged| staged.as_any().downcast_ref::<Staged<T>>())
    }

    /// The registry the definitions' expressions were compiled against,
    /// for a cross check that compiles something of its own. `None`
    /// while that registry is still being built: in a registry step.
    pub fn expr_registry(&self) -> Option<&'a crate::expr::ExprRegistry> {
        self.exprs
    }

    /// The definition with a dense id, if it loaded.
    pub fn get<T: Def>(&self, id: T::Id) -> Option<&'a T> {
        self.staged::<T>()?.items.get(id.index())?.as_ref()
    }

    /// The dense id of a qualified id.
    pub fn lookup<T: Def>(&self, qualified: &str) -> Option<T::Id> {
        let staged = self.staged::<T>()?;
        T::Id::from_index(*staged.index.get(qualified)?)
    }

    /// The qualified id of a dense id.
    pub fn name<T: Def>(&self, id: T::Id) -> Option<&'a str> {
        self.staged::<T>()?
            .names
            .get(id.index())
            .map(String::as_str)
    }

    /// Every definition of a type that loaded, in id order.
    pub fn iter<T: Def>(&self) -> impl Iterator<Item = (T::Id, &'a T)> + 'a {
        self.staged::<T>()
            .into_iter()
            .flat_map(|staged| staged.items.iter().enumerate())
            .filter_map(|(index, item)| Some((T::Id::from_index(index)?, item.as_ref()?)))
    }

    /// The tags.
    pub fn tags(&self) -> &'a TagTable {
        self.tags
    }
}

/// What [`Def::finish`] works with.
pub struct FinishCx<'a> {
    sources: &'a SourceMap,
    link: &'a LinkShared<'a>,
    label: &'a DefLabel,
    id_span: Option<Span>,
    diagnostics: Vec<Diagnostic>,
    failed: bool,
}

impl FinishCx<'_> {
    /// The files of this load, for positions.
    pub fn sources(&self) -> &SourceMap {
        self.sources
    }

    /// The qualified id of the definition being finished.
    pub fn id(&self) -> &str {
        &self.label.id
    }

    /// Resolves an id written at `span` to a definition of type `T`, the
    /// way a [`Ref`](crate::Ref) does: in the namespace of the mod that
    /// wrote it, then its imports (04 §4.3). A failure is reported at
    /// `span` and breaks the definition.
    pub fn resolve<T: Def>(&mut self, id: &str, span: Span) -> Option<T::Id> {
        let mut warnings = Vec::new();
        let resolved = resolve_id(self.link, self.sources, T::TYPE, id, span, &mut warnings);
        for warning in warnings {
            self.report(warning);
        }
        match resolved {
            Resolved::Found(index) => T::Id::from_index(index as usize),
            Resolved::Failed(diagnostic) => {
                self.report(*diagnostic);
                None
            }
        }
    }

    /// Resolves a located string as [`resolve`](FinishCx::resolve) does.
    pub fn resolve_located<T: Def>(&mut self, id: &Located<String>) -> Option<T::Id> {
        self.resolve::<T>(&id.value, id.span)
    }

    /// The tag for a string, if any definition carries it.
    pub fn tag(&self, name: &str) -> Option<Tag> {
        self.link
            .tags
            .binary_search_by(|known| known.as_str().cmp(name))
            .ok()
            .map(|index| Tag::from_index(to_u32(index)))
    }

    /// The namespace of the mod that wrote the value at `span`.
    pub fn namespace(&self, span: Span) -> Option<&str> {
        self.link
            .mods
            .manifest_of(span.file)
            .map(|manifest| manifest.id.as_str())
    }

    /// Reports an error at `span`. The definition is broken.
    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        let diagnostic = Diagnostic::new(code::INVALID_VALUE, message).at(self.sources, span);
        self.report(diagnostic);
    }

    /// Reports a diagnostic of any code, attributed to this definition.
    /// An error breaks the definition.
    pub fn report(&mut self, diagnostic: Diagnostic) {
        if diagnostic.severity == Severity::Error {
            self.failed = true;
        }
        let mut diagnostic = diagnostic.in_def(Some(self.label));
        if diagnostic.location.is_none()
            && let Some(span) = self.id_span
        {
            diagnostic = diagnostic.at(self.sources, span);
        }
        self.diagnostics.push(diagnostic);
    }

    /// Notes that the value at `span` uses an experimental feature
    /// (04 §8.4). The loader refuses it unless the mod that wrote the
    /// value has opted in.
    pub fn use_experimental(&mut self, feature: &str, span: Span) {
        self.link.feature_uses.borrow_mut().insert(FeatureUse {
            feature: feature.to_owned(),
            span,
            label: Some(self.label.clone()),
            path: String::new(),
        });
    }
}

/// What [`Def::validate`] works with.
pub struct ValidateCx<'a> {
    view: &'a DefsView<'a>,
    sources: &'a SourceMap,
    label: &'a DefLabel,
    id_span: Option<Span>,
    diagnostics: &'a mut Diagnostics,
}

impl<'a> ValidateCx<'a> {
    /// Every definition that loaded.
    pub fn defs(&self) -> &'a DefsView<'a> {
        self.view
    }

    /// The files of this load, for positions.
    pub fn sources(&self) -> &'a SourceMap {
        self.sources
    }

    /// The qualified id of the definition being checked.
    pub fn id(&self) -> &str {
        &self.label.id
    }

    /// Reports an error against the definition as a whole.
    pub fn error(&mut self, message: impl Into<String>) {
        self.report(Diagnostic::new(code::VALIDATION, message));
    }

    /// Reports an error at a position inside the definition.
    pub fn error_at(&mut self, span: Span, message: impl Into<String>) {
        let diagnostic = Diagnostic::new(code::VALIDATION, message).at(self.sources, span);
        self.report(diagnostic);
    }

    /// Reports a warning against the definition as a whole.
    pub fn warn(&mut self, message: impl Into<String>) {
        self.report(Diagnostic::new(code::VALIDATION_WARNING, message));
    }

    /// Reports a warning at a position inside the definition.
    pub fn warn_at(&mut self, span: Span, message: impl Into<String>) {
        let diagnostic = Diagnostic::new(code::VALIDATION_WARNING, message).at(self.sources, span);
        self.report(diagnostic);
    }

    /// Reports a diagnostic of any code, attributed to this definition.
    /// One with no position is placed at the definition's id.
    pub fn report(&mut self, diagnostic: Diagnostic) {
        let mut diagnostic = diagnostic.in_def(Some(self.label));
        if diagnostic.location.is_none()
            && let Some(span) = self.id_span
        {
            diagnostic = diagnostic.at(self.sources, span);
        }
        self.diagnostics.push(diagnostic);
    }
}

/// What a [`CrossCheck`] works with.
pub struct CrossCx<'a> {
    pub(crate) view: &'a DefsView<'a>,
    pub(crate) sources: &'a SourceMap,
    pub(crate) diagnostics: &'a mut Diagnostics,
}

impl<'a> CrossCx<'a> {
    /// Every definition that loaded.
    pub fn defs(&self) -> &'a DefsView<'a> {
        self.view
    }

    /// The files of this load, for positions.
    pub fn sources(&self) -> &'a SourceMap {
        self.sources
    }

    /// Reports a diagnostic.
    pub fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Reports a diagnostic with a code and a message at a position.
    pub fn report_at(&mut self, code: Code, span: Span, message: impl Into<String>) {
        let diagnostic = Diagnostic::new(code, message).at(self.sources, span);
        self.diagnostics.push(diagnostic);
    }
}

/// A finished definition as a tree, for tools: what `show` prints.
#[derive(Clone, Debug, PartialEq)]
pub struct FinishedTree {
    /// The definition type's JSON name.
    pub type_name: String,
    /// The qualified id.
    pub id: String,
    /// The fields after every patch and all inheritance. Each value's
    /// span is where it was last written.
    pub tree: Node,
    /// Whether the definition is abstract: finished, and not loaded.
    pub is_abstract: bool,
    /// Whether it was written in place inside another definition
    /// (04 §5).
    pub inline: bool,
}

/// A mod of a finished load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedModInfo {
    /// The manifest.
    pub manifest: ModManifest,
    /// The mod's directory as it was given to the loader.
    pub root: String,
}

/// The finished, immutable set of all definitions (04 §9).
///
/// `Defs` is built once by a [`Loader`](crate::Loader) and never changes
/// (04 M4). It is `Send + Sync`; share it behind the `Arc` the loader
/// returns. Swapping in a new one goes through save and load (04 M5).
pub struct Defs {
    registries: BTreeMap<TypeId, Box<dyn AnyRegistry>>,
    by_name: BTreeMap<&'static str, TypeId>,
    tags: TagTable,
    fingerprint: Hash128,
    sources: Arc<SourceMap>,
    mods: Vec<LoadedModInfo>,
    profile: Option<Profile>,
    trees: Vec<FinishedTree>,
    experimental: Vec<String>,
    exprs: ExprDefs,
}

/// The pieces a [`Defs`] is assembled from.
pub(crate) struct DefsParts {
    pub(crate) registries: Vec<(&'static str, TypeId, Box<dyn AnyRegistry>)>,
    pub(crate) tags: TagTable,
    pub(crate) fingerprint: Hash128,
    pub(crate) sources: Arc<SourceMap>,
    pub(crate) mods: Vec<LoadedModInfo>,
    pub(crate) profile: Option<Profile>,
    pub(crate) trees: Vec<FinishedTree>,
    pub(crate) experimental: Vec<String>,
    pub(crate) exprs: ExprDefs,
}

impl Defs {
    pub(crate) fn assemble(parts: DefsParts) -> Defs {
        let mut registries = BTreeMap::new();
        let mut by_name = BTreeMap::new();
        for (name, type_id, registry) in parts.registries {
            by_name.insert(name, type_id);
            registries.insert(type_id, registry);
        }
        Defs {
            registries,
            by_name,
            tags: parts.tags,
            fingerprint: parts.fingerprint,
            sources: parts.sources,
            mods: parts.mods,
            profile: parts.profile,
            trees: parts.trees,
            experimental: parts.experimental,
            exprs: parts.exprs,
        }
    }

    /// The expression side of the load: the registry every expression
    /// was compiled against, the functions and variables mods define,
    /// and where each compiled expression came from.
    pub fn exprs(&self) -> &ExprDefs {
        &self.exprs
    }

    /// The registry every expression was compiled against: what the
    /// host gave the loader, with the definition ids, functions and
    /// variables of the loaded mods added. Shared, so the simulation can
    /// keep it.
    pub fn expr_registry(&self) -> &Arc<ExprRegistry> {
        self.exprs.registry()
    }

    /// The compiled functions mods define (05 §12).
    pub fn functions(&self) -> &crate::expr::Functions {
        self.exprs.functions()
    }

    /// The variables mods declare (05 §6).
    pub fn variables(&self) -> &crate::expr::Variables {
        self.exprs.variables()
    }

    /// The registry of a definition type.
    ///
    /// # Panics
    ///
    /// If `T` was not registered with the [`DefTypes`] this was loaded
    /// with: that is a mistake in the engine, not in a mod. Use
    /// [`try_registry`](Defs::try_registry) where a type may be absent.
    pub fn registry<T: Def>(&self) -> &Registry<T> {
        match self.try_registry::<T>() {
            Some(registry) => registry,
            None => panic!(
                "definition type `{}` was not registered with the loader",
                T::TYPE
            ),
        }
    }

    /// The registry of a definition type, if the type was registered.
    pub fn try_registry<T: Def>(&self) -> Option<&Registry<T>> {
        self.registries
            .get(&TypeId::of::<T>())
            .and_then(|registry| registry.as_any().downcast_ref::<Registry<T>>())
    }

    /// The definition of type `T` with a dense id. Shorthand for
    /// `registry::<T>().get(id)`, and panics as those do.
    pub fn get<T: Def>(&self, id: T::Id) -> &T {
        self.registry::<T>().get(id)
    }

    /// The dense id of a qualified id of type `T`.
    pub fn lookup<T: Def>(&self, qualified: &str) -> Option<T::Id> {
        self.try_registry::<T>()?.lookup(qualified)
    }

    /// The registered definition types' JSON names, sorted.
    pub fn type_names(&self) -> Vec<&'static str> {
        self.by_name.keys().copied().collect()
    }

    /// The qualified ids of a definition type by its JSON name, in id
    /// order. For tools that do not know the Rust type.
    pub fn ids_of(&self, type_name: &str) -> Option<&[String]> {
        let type_id = self.by_name.get(type_name)?;
        self.registries
            .get(type_id)
            .map(|registry| registry.names())
    }

    /// The tags.
    pub fn tags(&self) -> &TagTable {
        &self.tags
    }

    /// The content fingerprint: a hash of every finished tree in
    /// canonical form (04 §9). The same profile and files always give the
    /// same fingerprint, whatever order the file system lists them in
    /// (M1), and a change to any finished tree changes it. Where a value
    /// sits in a file does not.
    pub fn fingerprint(&self) -> Hash128 {
        self.fingerprint
    }

    /// The files this was loaded from, for positions in later
    /// diagnostics.
    pub fn sources(&self) -> &Arc<SourceMap> {
        &self.sources
    }

    /// The mods, in load order.
    pub fn mods(&self) -> &[LoadedModInfo] {
        &self.mods
    }

    /// The profile this was loaded from, if it was loaded from one.
    pub fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    /// The experimental features the loaded mods opted in to, sorted
    /// (04 §8.4). A save records them.
    pub fn experimental_features(&self) -> &[String] {
        &self.experimental
    }

    /// Every finished tree, sorted by type and id, abstract definitions
    /// included. For `show` and other tools.
    pub fn trees(&self) -> &[FinishedTree] {
        &self.trees
    }

    /// The finished tree of one definition.
    pub fn tree(&self, type_name: &str, id: &str) -> Option<&FinishedTree> {
        self.trees
            .binary_search_by(|tree| {
                (tree.type_name.as_str(), tree.id.as_str()).cmp(&(type_name, id))
            })
            .ok()
            .map(|index| &self.trees[index])
    }
}

impl std::fmt::Debug for Defs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Defs")
            .field("types", &self.by_name.keys().collect::<Vec<_>>())
            .field("fingerprint", &self.fingerprint)
            .field("mods", &self.mods.len())
            .finish_non_exhaustive()
    }
}
