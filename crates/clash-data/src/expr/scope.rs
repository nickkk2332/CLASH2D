//! [`Scope`]: what a definition type says is in scope in a field, and
//! [`CompileCx`]: what [`Def::compile`](crate::Def::compile) works with
//! (04 §7.3; 05 §7.3).

use std::cell::RefCell;

use clash_expr::{Context, Type, Undeclared};

use super::codes;
use super::defs::Functions;
use super::{Compile, ExprRegistry};
use crate::diag::{DefLabel, Diagnostic, Severity, closest, did_you_mean};
use crate::fields::Site;
use crate::merge::ModTable;
use crate::mods::ENGINE_NAMESPACE;
use crate::source::{SourceMap, Span};

/// A type in a [`Scope`]: by name, as the language writes it, or as a
/// type the caller already holds.
///
/// A definition type lives in a crate that may not know the registry's
/// ids, so it names kinds and types as a mod would: `"entity"`,
/// `"real"`, `"id<material>"`, `"entity?"`. The name is looked up when a
/// field is compiled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TypeSpec {
    /// A type by name.
    Named(String),
    /// A type.
    Exact(Type),
}

impl From<&str> for TypeSpec {
    fn from(name: &str) -> TypeSpec {
        TypeSpec::Named(name.to_owned())
    }
}

impl From<String> for TypeSpec {
    fn from(name: String) -> TypeSpec {
        TypeSpec::Named(name)
    }
}

impl From<Type> for TypeSpec {
    fn from(ty: Type) -> TypeSpec {
        TypeSpec::Exact(ty)
    }
}

/// What is in scope where an expression field appears (05 §7.3): the
/// subjects bound there and their kinds, the locals, the event whose
/// fields can be read, whether the value must be pure, and whether it
/// reads truth or only what a character believes.
///
/// A definition type builds one in [`Def::compile`](crate::Def::compile)
/// and compiles its fields against it:
///
/// ```
/// use clash_data::Scope;
///
/// // A rule about an entity somewhere: random draws allowed, truth read.
/// let rule = Scope::new().subject("self", "entity").subject("here", "loc");
/// // What a screen shows: the same subjects, and the value must be pure.
/// let shown = rule.clone().pure();
/// // What a character decides by: its own state and what it perceived.
/// let goal = Scope::new().mind("self", "entity").local("urgency", "real");
/// # let _ = (shown, goal);
/// ```
///
/// Subjects and locals take slots in the order they are added, which is
/// the order the host must supply their values in when the field is
/// evaluated (see [`clash_expr::Context`]).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Scope {
    subjects: Vec<(String, TypeSpec, bool)>,
    locals: Vec<(String, TypeSpec)>,
    event: Option<String>,
    pure: bool,
    belief: bool,
    expect: Option<TypeSpec>,
}

impl Scope {
    /// A scope with nothing bound, in which random functions are allowed
    /// and truth is read.
    pub fn new() -> Scope {
        Scope::default()
    }

    /// Binds a subject, or any named value, and gives it the next slot.
    #[must_use]
    pub fn subject(mut self, name: impl Into<String>, ty: impl Into<TypeSpec>) -> Scope {
        self.subjects.push((name.into(), ty.into(), false));
        self
    }

    /// Binds the character whose state the field may read freely in a
    /// mind context (05 §7.3), without making the scope a mind context.
    #[must_use]
    pub fn own(mut self, name: impl Into<String>, ty: impl Into<TypeSpec>) -> Scope {
        self.subjects.push((name.into(), ty.into(), true));
        self
    }

    /// Makes this the mind context of a character bound as `name`: only
    /// belief may be read, and the character's own state is belief.
    /// Goals, methods, tactics, an action's defaults and ordinary screens
    /// compile in such a scope (01 R11).
    #[must_use]
    pub fn mind(self, name: impl Into<String>, ty: impl Into<TypeSpec>) -> Scope {
        self.own(name, ty).belief()
    }

    /// Declares a local, written `$name`, and gives it the next slot.
    #[must_use]
    pub fn local(mut self, name: impl Into<String>, ty: impl Into<TypeSpec>) -> Scope {
        self.locals.push((name.into(), ty.into()));
        self
    }

    /// Makes the fields of an event readable as `event.field`, by the
    /// event's registered name.
    #[must_use]
    pub fn event(mut self, name: impl Into<String>) -> Scope {
        self.event = Some(name.into());
        self
    }

    /// Requires purity: random functions are refused.
    #[must_use]
    pub fn pure(mut self) -> Scope {
        self.pure = true;
        self
    }

    /// Makes this a mind context: only belief may be read.
    #[must_use]
    pub fn belief(mut self) -> Scope {
        self.belief = true;
        self
    }

    /// Sets the type the field's value must have, in place of the one
    /// its Rust type gives. For an [`Expr`](crate::Expr) whose type is
    /// only known from data, as a function's `returns` is.
    #[must_use]
    pub fn expecting(mut self, ty: impl Into<TypeSpec>) -> Scope {
        self.expect = Some(ty.into());
        self
    }

    /// Whether random functions are refused.
    pub fn is_pure(&self) -> bool {
        self.pure
    }

    /// Whether this is a mind context.
    pub fn is_belief(&self) -> bool {
        self.belief
    }

    /// How many subjects are bound: the slot the next one takes.
    pub(crate) fn subject_count(&self) -> usize {
        self.subjects.len()
    }

    /// The slot of the first subject bound as own, the character of a
    /// mind context.
    pub(crate) fn own_slot(&self) -> Option<usize> {
        self.subjects.iter().position(|(_, _, own)| *own)
    }

    /// The same scope with no type set for the field's value, for a
    /// part of a field that has a type of its own.
    #[must_use]
    pub(crate) fn without_expectation(mut self) -> Scope {
        self.expect = None;
        self
    }
}

/// Where a compiled expression came from: what a run-time warning's
/// origin number stands for (05 §4.5).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Origin {
    /// The definition the expression is in.
    pub definition: DefLabel,
    /// The JSON path of the field inside it.
    pub path: String,
    /// Where the string that holds the expression was written.
    pub span: Option<Span>,
}

/// How positions in what the compiler was given relate to the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SpanMap {
    /// The source is the decoded content of the JSON string at the site.
    InString,
    /// The source was made from the value at the site: point at it all.
    Whole,
}

/// What compiling needs from the load, shared by every definition.
pub(crate) struct CompileEnv<'a> {
    pub(crate) sources: &'a SourceMap,
    pub(crate) registry: &'a ExprRegistry,
    pub(crate) mods: &'a ModTable,
    pub(crate) functions: &'a Functions,
    pub(crate) origins: &'a RefCell<Vec<Origin>>,
}

/// What [`Def::compile`](crate::Def::compile) works with: the registry
/// to compile against, and where diagnostics go.
///
/// ```ignore
/// fn compile(&mut self, cx: &mut CompileCx<'_>) {
///     let scope = Scope::new().subject("self", "entity").subject("here", "loc");
///     cx.compile(&mut self.when, &scope);            // a Cond
///     cx.compile(&mut self.energy, &scope);          // an Expr<Number>
///     cx.compile(&mut self.log, &scope.clone().pure()); // a Text
/// }
/// ```
pub struct CompileCx<'a> {
    env: &'a CompileEnv<'a>,
    label: &'a DefLabel,
    id_span: Option<Span>,
    diagnostics: Vec<Diagnostic>,
    failed: bool,
}

impl<'a> CompileCx<'a> {
    pub(crate) fn new(
        env: &'a CompileEnv<'a>,
        label: &'a DefLabel,
        id_span: Option<Span>,
    ) -> CompileCx<'a> {
        CompileCx {
            env,
            label,
            id_span,
            diagnostics: Vec::new(),
            failed: false,
        }
    }

    /// Whether anything reported so far was an error.
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    /// How many errors have been reported so far: a part of a field
    /// compiled soundly if the count did not grow.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub(crate) fn functions(&self) -> &'a Functions {
        self.env.functions
    }

    /// Compiles a field, or every field in an `Option`, a `Vec`, a `Box`
    /// or a map, against a scope. What is wrong is reported with file,
    /// line, column, definition and JSON path, and breaks the
    /// definition.
    pub fn compile<F: Compile + ?Sized>(&mut self, field: &mut F, scope: &Scope) {
        field.compile_in(self, scope);
    }

    /// The registry fields are compiled against: the host's, with the
    /// definition ids, functions and variables of the mods being loaded.
    pub fn registry(&self) -> &'a ExprRegistry {
        self.env.registry
    }

    /// The files of this load, for positions.
    pub fn sources(&self) -> &'a SourceMap {
        self.env.sources
    }

    /// The qualified id of the definition being compiled.
    pub fn id(&self) -> &str {
        &self.label.id
    }

    /// A scope that binds the subjects of a registered event, in the
    /// order it delivers them, and makes its fields readable. If no
    /// event of that name is registered, that is reported and the scope
    /// is empty.
    pub fn event_scope(&mut self, event: &str) -> Scope {
        let registry = self.env.registry;
        let Some(found) = registry.event_id(event).and_then(|id| registry.event(id)) else {
            let names = registry.events().map(|event| event.spec.name.as_str());
            let diagnostic = Diagnostic::new(
                codes::BAD_SCOPE,
                format!("this definition is about the event `{event}`, which is not registered"),
            )
            .suggest_opt(did_you_mean(closest(event, names)));
            self.report(diagnostic);
            return Scope::new();
        };
        let mut scope = Scope::new().event(event);
        for (name, kind) in &found.spec.subjects {
            scope = scope.subject(name.clone(), Type::Subject(*kind));
        }
        scope
    }

    /// Reads a type written in data, as the language writes types
    /// (05 §3): `real`, `entity`, `id<material>`, `list<int>`, `real?`.
    /// One that is not a type is reported at `span` and breaks the
    /// definition.
    pub fn parse_type(&mut self, name: &str, span: Option<Span>, path: &str) -> Option<Type> {
        let registry = self.env.registry;
        if let Some(ty) = registry.parse_type(name) {
            return Some(ty);
        }
        let kinds = registry.kinds().map(|kind| kind.spec.name.as_str());
        let known = BUILT_IN_TYPES.iter().copied().chain(kinds);
        let mut diagnostic =
            Diagnostic::new(codes::BAD_TYPE_NAME, format!("\"{name}\" is not a type"))
                .with_path(path)
                .suggest_opt(did_you_mean(closest(name, known)));
        if let Some(span) = span {
            diagnostic = diagnostic.at(self.env.sources, span);
        }
        self.report(diagnostic);
        None
    }

    /// The [`Context`] an expression at `site` is compiled in: what the
    /// scope binds, the namespace and imports of the mod that wrote the
    /// value, that mod's policy on undeclared variables, and an origin
    /// number for run-time warnings.
    ///
    /// [`compile`](CompileCx::compile) does this for the field types of
    /// this crate. It is offered for a field type of another crate, such
    /// as a list of effects, that drives the expression compiler itself.
    pub fn context(&mut self, scope: &Scope, site: &Site) -> Context {
        let mut context = Context::new();
        for (name, ty, own) in &scope.subjects {
            let ty = self.resolve_type(ty, "subject", name);
            context.add_subject(name.clone(), ty, *own);
        }
        for (name, ty) in &scope.locals {
            let ty = self.resolve_type(ty, "local", name);
            context.add_local(name.clone(), ty);
        }
        if let Some(event) = &scope.event {
            match self.env.registry.event_id(event) {
                Some(id) => context = context.event(id),
                None => {
                    let diagnostic = Diagnostic::new(
                        codes::BAD_SCOPE,
                        format!(
                            "this field's scope names the event `{event}`, which is not registered"
                        ),
                    )
                    .with_path(site.path.clone());
                    self.report(diagnostic);
                }
            }
        }
        if scope.pure {
            context = context.pure();
        }
        if scope.belief {
            context = context.belief();
        }
        let strict = site
            .span
            .and_then(|span| self.env.mods.manifest_of(span.file))
            .is_some_and(|manifest| manifest.strict_vars);
        if strict {
            context = context.undeclared(Undeclared::Deny);
        }
        apply_mod_scope(context, self.env.mods, site.span).origin(self.origin(site))
    }

    /// The type a field wants: what its scope says, or else what its
    /// Rust type says.
    pub(crate) fn expected(&mut self, scope: &Scope, otherwise: Type) -> Type {
        match &scope.expect {
            Some(ty) => self.resolve_type(ty, "result", "type"),
            None => otherwise,
        }
    }

    /// A new origin number for an expression at `site`.
    pub(crate) fn origin(&mut self, site: &Site) -> u64 {
        let mut origins = self.env.origins.borrow_mut();
        origins.push(Origin {
            definition: self.label.clone(),
            path: site.path.clone(),
            span: site.span,
        });
        origins.len() as u64
    }

    fn resolve_type(&mut self, ty: &TypeSpec, what: &str, name: &str) -> Type {
        match ty {
            TypeSpec::Exact(ty) => ty.clone(),
            TypeSpec::Named(written) => match self.env.registry.parse_type(written) {
                Some(ty) => ty,
                None => {
                    let diagnostic = Diagnostic::new(
                        codes::BAD_SCOPE,
                        format!(
                            "this field's scope gives the {what} `{name}` the type `{written}`, which is not registered"
                        ),
                    );
                    self.report(diagnostic);
                    Type::Any
                }
            },
        }
    }

    /// Reports what the expression compiler said about the source at
    /// `site`.
    pub(crate) fn report_expr(
        &mut self,
        diagnostics: impl IntoIterator<Item = clash_expr::Diagnostic>,
        site: &Site,
        map: SpanMap,
    ) {
        for diagnostic in diagnostics {
            let converted = self.convert(&diagnostic, site, map);
            self.report(converted);
        }
    }

    /// Reports what the expression compiler said about a source that is
    /// the content of the JSON string at `site`: each diagnostic gets a
    /// code of group `06`, the file position inside the string with its
    /// escapes accounted for, the definition and the JSON path.
    ///
    /// For a field type of another crate that drives the expression
    /// compiler itself with a [`context`](CompileCx::context).
    pub fn report_compiler(
        &mut self,
        diagnostics: impl IntoIterator<Item = clash_expr::Diagnostic>,
        site: &Site,
    ) {
        self.report_expr(diagnostics, site, SpanMap::InString);
    }

    /// A compiler diagnostic as a loader diagnostic.
    pub(crate) fn convert(
        &self,
        diagnostic: &clash_expr::Diagnostic,
        site: &Site,
        map: SpanMap,
    ) -> Diagnostic {
        let code = codes::for_expr_code(diagnostic.code).unwrap_or(codes::BAD_SOURCE);
        let severity = match diagnostic.severity {
            clash_expr::Severity::Error => Severity::Error,
            clash_expr::Severity::Warning => Severity::Warning,
            clash_expr::Severity::Note => Severity::Note,
        };
        let mut converted = Diagnostic::new(code, diagnostic.message.clone())
            .with_severity(severity)
            .with_path(site.path.clone())
            .suggest_opt(diagnostic.suggestion.clone());
        if let Some(span) = site.span {
            let span = match map {
                SpanMap::InString => self.env.sources.span_in_string(
                    span,
                    diagnostic.span.start as usize,
                    diagnostic.span.end as usize,
                ),
                SpanMap::Whole => span,
            };
            converted = converted.at(self.env.sources, span);
        }
        converted
    }

    /// Reports an error of the compile stage at a site. The definition
    /// is broken.
    pub(crate) fn error_at(
        &mut self,
        code: crate::diag::Code,
        site: &Site,
        message: impl Into<String>,
    ) {
        let mut diagnostic = Diagnostic::new(code, message).with_path(site.path.clone());
        if let Some(span) = site.span {
            diagnostic = diagnostic.at(self.env.sources, span);
        }
        self.report(diagnostic);
    }

    /// Reports a diagnostic of any code, attributed to this definition.
    /// An error breaks the definition. One with no position is placed at
    /// the definition's id.
    pub fn report(&mut self, diagnostic: Diagnostic) {
        if diagnostic.severity == Severity::Error {
            self.failed = true;
        }
        let mut diagnostic = diagnostic.in_def(Some(self.label));
        if diagnostic.location.is_none()
            && let Some(span) = self.id_span
        {
            diagnostic = diagnostic.at(self.env.sources, span);
        }
        self.diagnostics.push(diagnostic);
    }
}

/// Gives a context the namespace bare names are looked up in: that of
/// the mod that wrote the value at `span`, then the namespaces it
/// imports, then the engine's (04 §4.3, §6.1).
pub(crate) fn apply_mod_scope(context: Context, mods: &ModTable, span: Option<Span>) -> Context {
    match span.and_then(|span| mods.manifest_of(span.file)) {
        Some(manifest) => {
            let imports = manifest
                .imports
                .iter()
                .map(String::as_str)
                .filter(|import| *import != manifest.id && *import != ENGINE_NAMESPACE)
                .chain([ENGINE_NAMESPACE]);
            context.namespace(manifest.id.clone()).imports(imports)
        }
        None => context.imports([ENGINE_NAMESPACE]),
    }
}

/// The names of the types that are not kinds of subject, for
/// suggestions.
const BUILT_IN_TYPES: &[&str] = &[
    "any", "bool", "int", "real", "number", "string", "list", "map", "null",
];
