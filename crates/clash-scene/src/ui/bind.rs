//! Bindings (12 §10.4): the expressions and text templates a screen's
//! properties and effects are written as, and how they are compiled when
//! mods load.
//!
//! A [`Binding`] holds what was written and where. The definition type
//! that owns it (`screen`, `ui_part`) compiles it in `Def::compile`,
//! through a [`BindCx`] that knows what the place offers:
//!
//! | Name | Is | Slot |
//! |------|----|------|
//! | `view` | The active controlled character | subject 0 |
//! | `party` | All directly controlled characters | subject 1 |
//! | `app` | Things outside the simulation | subject 2 |
//! | `screen` | The screen's variables, read as `screen.name` | subject 3 |
//! | `it` | The current row, where there is one | subject 4 |
//! | `$name` | The screen's parameters, in order of name | locals |
//!
//! Every binding is pure. In a screen whose context is not `debug` it is
//! also a mind context (05 §7.3): `view`, `party`, a parameter of type
//! `party_member` and whatever is reached from them as their own may be
//! read in full, and everything else only through what is registered as
//! safe for belief.

use core::fmt;
use std::collections::BTreeMap;

use clash_data::{CompileCx, Diagnostic, FromSite, Kind, KindValue, Scope, Site, code};
use clash_expr::ast::{Expr as Ast, ExprKind, TemplatePart};
use clash_expr::{Compiled, CompiledText, Context, Type};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

use super::def::Literal;

/// The slot of `view` among a binding's subjects.
pub const SLOT_VIEW: u16 = 0;
/// The slot of `party`.
pub const SLOT_PARTY: u16 = 1;
/// The slot of `app`.
pub const SLOT_APP: u16 = 2;
/// The slot of `screen`.
pub const SLOT_SCREEN: u16 = 3;
/// The slot of `it`.
pub const SLOT_IT: u16 = 4;

/// The kind of subject `view` and the members of `party` are, when the
/// simulation has registered it. With no such kind, as in a tool that
/// loads without a simulation, they are `any`.
pub const KIND_ENTITY: &str = "entity";
/// The kind of subject `app` is. The scene registers it.
pub const KIND_APP: &str = "app";
/// The kind of subject an entry of one of `app`'s lists is.
pub const KIND_APP_ENTRY: &str = "app_entry";
/// The parameter type that means "one of the party": an entity that may
/// be read in full in an ordinary screen (12 §10.4).
pub const TYPE_PARTY_MEMBER: &str = "party_member";

/// What a binding was compiled to.
#[derive(Clone, Debug, Default)]
enum Code {
    /// Not compiled: read outside the loader, or broken.
    #[default]
    None,
    /// An expression.
    Expr(Compiled),
    /// A text template.
    Text(CompiledText),
}

/// The source of a bound property: an expression or a text template,
/// where it was written, and, once its definition has been compiled, its
/// compiled form.
///
/// A binding read by anything but the mod loader has no place and is
/// never compiled: a screen made from it shows a placeholder.
#[derive(Clone, Debug, Default)]
pub struct Binding {
    source: String,
    site: Site,
    code: Code,
    own: bool,
    ty: Option<Type>,
    /// A query written as an object (05 §11), kept as compact JSON.
    query: bool,
}

/// Two bindings are equal when they were written the same.
impl PartialEq for Binding {
    fn eq(&self, other: &Binding) -> bool {
        self.source == other.source
    }
}

impl From<&str> for Binding {
    fn from(source: &str) -> Binding {
        Binding::unsited(source)
    }
}

impl From<String> for Binding {
    fn from(source: String) -> Binding {
        Binding::unsited(source)
    }
}

impl Binding {
    /// A binding written nowhere: never compiled.
    pub fn unsited(source: impl Into<String>) -> Binding {
        Binding {
            source: source.into(),
            ..Binding::default()
        }
    }

    /// A binding and where it was written.
    pub fn sited(source: impl Into<String>, site: Site) -> Binding {
        Binding {
            source: source.into(),
            site,
            ..Binding::default()
        }
    }

    /// A query written as an object, kept until queries are built
    /// (05 §11): it compiles to nothing and gives no rows.
    pub fn query(json: impl Into<String>, site: Site) -> Binding {
        Binding {
            source: json.into(),
            site,
            query: true,
            ..Binding::default()
        }
    }

    /// The source as written.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Where the source was written and at what JSON path.
    pub fn site(&self) -> &Site {
        &self.site
    }

    /// Whether the binding has been compiled.
    pub fn is_compiled(&self) -> bool {
        !matches!(self.code, Code::None)
    }

    /// The compiled expression, if the binding is one and compiled.
    pub fn expr(&self) -> Option<&Compiled> {
        match &self.code {
            Code::Expr(code) => Some(code),
            _ => None,
        }
    }

    /// The compiled template, if the binding is one and compiled.
    pub fn template(&self) -> Option<&CompiledText> {
        match &self.code {
            Code::Text(code) => Some(code),
            _ => None,
        }
    }

    /// Whether the value is the party's own (05 §7.3), so that a row of
    /// it may be read in full.
    pub fn is_own(&self) -> bool {
        self.own
    }

    /// The type of the binding's value, once compiled.
    pub fn ty(&self) -> Option<&Type> {
        self.ty.as_ref()
    }

    /// Compiles the binding as an expression that must give `expected`.
    pub(crate) fn compile_expr(&mut self, bx: &mut BindCx<'_, '_>, expected: &Type) {
        if self.query {
            return;
        }
        let context = bx.context(&self.site);
        let outcome = clash_expr::check(&self.source, bx.cx.registry(), &context, expected);
        let mut diagnostics = outcome.diagnostics;
        if let Some(checked) = outcome.value {
            self.own = checked.is_own();
            self.ty = Some(checked.ty().clone());
            let compiled = checked.compile();
            diagnostics.extend(compiled.diagnostics);
            if let Some(code) = compiled.value {
                self.code = Code::Expr(code);
            }
        }
        if let Ok(tree) = clash_expr::parse_expr(&self.source) {
            bx.undeclared_variables(&tree, &mut diagnostics);
        }
        let failed = diagnostics.iter().any(clash_expr::Diagnostic::is_error);
        if failed {
            self.code = Code::None;
        }
        bx.cx.report_compiler(diagnostics, &self.site);
    }

    /// Compiles the binding as a text template.
    pub(crate) fn compile_text(&mut self, bx: &mut BindCx<'_, '_>) {
        let context = bx.context(&self.site);
        let outcome = clash_expr::compile_template(&self.source, bx.cx.registry(), &context);
        let mut diagnostics = outcome.diagnostics;
        if let Some(code) = outcome.value {
            self.code = Code::Text(code);
        }
        if let Ok(template) = clash_expr::parse_template(&self.source) {
            for part in &template.parts {
                if let TemplatePart::Expr { expr, .. } = part {
                    bx.undeclared_variables(expr, &mut diagnostics);
                }
            }
        }
        if diagnostics.iter().any(clash_expr::Diagnostic::is_error) {
            self.code = Code::None;
        }
        bx.cx.report_compiler(diagnostics, &self.site);
    }
}

/// `{ "expr": "…" }`, the long way to write an expression (04 §7.1).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExprObject {
    pub(super) expr: Binding,
}

/// Reads an expression written as a string outside the loader.
struct PlainBinding;

impl<'de> Visitor<'de> for PlainBinding {
    type Value = Binding;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an expression as a string")
    }

    fn visit_str<E: de::Error>(self, source: &str) -> Result<Binding, E> {
        Ok(Binding::unsited(source))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Binding, A::Error> {
        ExprObject::deserialize(de::value::MapAccessDeserializer::new(map)).map(|e| e.expr)
    }
}

impl<'de> FromSite<'de> for Binding {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Binding, A::Error> {
        match kind {
            Kind::Object => value.read::<ExprObject>().map(|e| e.expr),
            // Anything else is read as a string, so that a value of the
            // wrong kind is reported as any wrong value is.
            _ => Ok(Binding::sited(value.read::<String>()?, site)),
        }
    }
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Binding, D::Error> {
        clash_data::by_site_or_plain(deserializer, PlainBinding)
    }
}

/// A parameter of a screen as bindings see it.
#[derive(Clone, Debug)]
pub(crate) struct ParamType {
    pub(crate) name: String,
    pub(crate) ty: Type,
    pub(crate) own: bool,
}

/// What `it` is where a binding is compiled.
#[derive(Clone, Debug)]
pub(crate) struct ItType {
    pub(crate) ty: Type,
    pub(crate) own: bool,
}

/// What a `screen` or a `ui_part` compiles its bindings with: the
/// loader's context, and what the place offers by name.
pub(crate) struct BindCx<'a, 'c> {
    pub(crate) cx: &'a mut CompileCx<'c>,
    /// Whether the screen's context is `debug`: truth may be read.
    pub(crate) debug: bool,
    /// The screen's parameters, in slot order.
    pub(crate) params: Vec<ParamType>,
    /// The screen's variables. `None` where they are not known, as in a
    /// part, which may be placed in any screen.
    pub(crate) vars: Option<BTreeMap<String, Literal>>,
    /// What `it` is, where there is a current row.
    pub(crate) it: Option<ItType>,
}

impl BindCx<'_, '_> {
    /// The type `view` has: the simulation's `entity`, or `any` where no
    /// such kind is registered.
    pub(crate) fn entity_type(&self) -> Type {
        self.cx
            .registry()
            .kind_id(KIND_ENTITY)
            .map_or(Type::Any, Type::Subject)
    }

    fn context(&mut self, site: &Site) -> Context {
        let registry = self.cx.registry();
        let entity = self.entity_type();
        let app = registry
            .kind_id(KIND_APP)
            .map_or(Type::Any, Type::Subject);
        let party = Type::list(entity.clone());
        let mut scope = Scope::new().pure();
        scope = if self.debug {
            scope.subject("view", entity).subject("party", party)
        } else {
            scope.own("view", entity).own("party", party).belief()
        };
        scope = scope
            .subject("app", app)
            .subject("screen", Type::map(Type::Any));
        if let Some(it) = &self.it {
            scope = if it.own && !self.debug {
                scope.own("it", it.ty.clone())
            } else {
                scope.subject("it", it.ty.clone())
            };
        }
        let mut context = self.cx.context(&scope, site);
        for param in &self.params {
            if param.own && !self.debug {
                context.add_own_local(param.name.clone(), param.ty.clone());
            } else {
                context.add_local(param.name.clone(), param.ty.clone());
            }
        }
        context
    }

    /// Reports each `screen.name` of an expression that names a variable
    /// the screen does not declare.
    fn undeclared_variables(&self, tree: &Ast, out: &mut Vec<clash_expr::Diagnostic>) {
        let Some(vars) = &self.vars else { return };
        walk(tree, &mut |node| {
            if let ExprKind::Member { object, name } = &node.kind
                && let ExprKind::Name(root) = &object.kind
                && !root.is_qualified()
                && root.name == "screen"
                && !vars.contains_key(&name.name)
            {
                let known = vars.keys().map(String::as_str);
                let hint = clash_expr::closest_name(&name.name, known)
                    .map(|close| format!("did you mean `screen.{close}`?"));
                out.push(
                    clash_expr::Diagnostic::error(
                        clash_expr::codes::UNKNOWN_NAME,
                        node.span,
                        format!(
                            "the screen has no variable `{}`: declare it in `vars`",
                            name.name
                        ),
                    )
                    .with_suggestion_opt(hint),
                );
            }
        });
    }

    /// Runs `f` with `it` bound as given, and puts back what it was.
    pub(crate) fn with_it<R>(
        &mut self,
        it: Option<ItType>,
        f: impl FnOnce(&mut BindCx<'_, '_>) -> R,
    ) -> R {
        let before = core::mem::replace(&mut self.it, it);
        let result = f(self);
        self.it = before;
        result
    }

    /// Reports a mistake that is not an expression's, at a JSON path of
    /// the definition being compiled.
    pub(crate) fn mistake(&mut self, site: &Site, message: String) {
        let mut diagnostic = Diagnostic::new(code::VALIDATION, message).with_path(site.path.clone());
        if let Some(span) = site.span {
            diagnostic = diagnostic.at(self.cx.sources(), span);
        }
        self.cx.report(diagnostic);
    }
}

/// Calls `f` on every node of a syntax tree.
fn walk(tree: &Ast, f: &mut impl FnMut(&Ast)) {
    f(tree);
    match &tree.kind {
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Number { .. }
        | ExprKind::Str(_)
        | ExprKind::Name(_)
        | ExprKind::Local(_) => {}
        ExprKind::List(items) => items.iter().for_each(|item| walk(item, f)),
        ExprKind::Map(entries) => {
            for (key, value) in entries {
                walk(key, f);
                walk(value, f);
            }
        }
        ExprKind::Member { object, .. } => walk(object, f),
        ExprKind::Call { callee, args } => {
            walk(callee, f);
            args.iter().for_each(|arg| walk(arg, f));
        }
        ExprKind::Index { object, index } => {
            walk(object, f);
            walk(index, f);
        }
        ExprKind::Unary { operand, .. } => walk(operand, f),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk(lhs, f);
            walk(rhs, f);
        }
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            walk(cond, f);
            walk(then, f);
            walk(otherwise, f);
        }
    }
}

/// The type of an element of a list of this type: what `it` is for rows
/// that come from it.
pub(crate) fn element_type(list: Option<&Type>) -> Type {
    list.and_then(Type::element)
        .map_or(Type::Any, |element| element.non_null().clone())
}
