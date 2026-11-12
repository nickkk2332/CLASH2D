//! The parameters, variables and locals of an action (03 §6.3, §6.4).

use std::fmt;

use clash_data::expr::{Any, Type};
use clash_data::{
    CompileCx, Diagnostic, Expr, FromKind, Kind, KindValue, Located, Scope, Span, by_kind, code,
    is_id_part,
};
use clash_expr::is_keyword;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

/// The type name that makes a parameter a list of what `of` says.
const LIST_TYPE: &str = "list";

/// The name 03 §6.3 gives a location where a parameter is declared, and
/// the kind of subject it is (05 §6: `cell` is the kind `loc`).
const CELL_TYPE: &str = "cell";
const CELL_KIND: &str = "loc";

/// The name a reaction's window is bound under: `$window` (03 §7.3).
pub const WINDOW_LOCAL: &str = "window";

/// The kind of subject every participant of an action is.
pub(crate) const ENTITY_KIND: &str = "entity";

/// How many values a list parameter takes (03 §6.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ListBounds {
    /// The least number of values.
    pub min: u32,
    /// The greatest number of values, if the parameter states one. With
    /// none, a list is as long as any value may be
    /// (`rules.max_value_size`).
    pub max: Option<u32>,
}

impl ListBounds {
    /// Whether a list of `len` values is within the bounds.
    pub fn allows(self, len: usize) -> bool {
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        len >= self.min && self.max.is_none_or(|max| len <= max)
    }
}

/// One parameter of an action (03 §6.3).
///
/// ```json
/// "params": {
///   "target": { "type": "entity" },
///   "part":   { "type": "part", "default": "auto_part($target)" },
///   "route":  { "type": "list", "of": "cell", "max": 24,
///               "check": "route_fault(self, $route)" }
/// }
/// ```
///
/// Read through [`ActionDef::params`](super::ActionDef::params), in the
/// order they were written.
#[derive(Debug)]
pub struct Param {
    /// The name, read in the action's expressions as `$name`.
    pub name: Located<String>,
    /// The type, as written.
    pub written_type: Located<String>,
    /// For a list: the type of its elements, as written.
    pub of: Option<Located<String>>,
    /// The value the parameter takes when the command leaves it out: an
    /// expression in the mind context of the actor, which may read the
    /// parameters declared before this one. A parameter with none must
    /// be given.
    pub default: Option<Expr<Any>>,
    /// Advice on a value being chosen: an expression in the mind context
    /// of the actor, pure, that gives null when the value is sound and
    /// otherwise the reason it is not. It may read this parameter and
    /// those before it. It never refuses anything by itself.
    pub check: Option<Expr<Any>>,
    min: Option<Located<u32>>,
    max: Option<Located<u32>>,
    ty: Type,
    list: Option<ListBounds>,
    slot: u16,
}

impl Param {
    /// The type a value for the parameter must have.
    pub fn ty(&self) -> &Type {
        &self.ty
    }

    /// For a list parameter, how many values it takes.
    pub fn list(&self) -> Option<ListBounds> {
        self.list
    }

    /// Whether the command must give the parameter: it has no default.
    pub fn is_required(&self) -> bool {
        self.default.is_none()
    }

    /// The slot of the parameter among the action's locals.
    pub fn slot(&self) -> u16 {
        self.slot
    }
}

/// The object form of a parameter.
#[derive(Deserialize)]
struct ParamObject {
    #[serde(rename = "type")]
    ty: Located<String>,
    #[serde(default)]
    of: Option<Located<String>>,
    #[serde(default)]
    min: Option<Located<u32>>,
    #[serde(default)]
    max: Option<Located<u32>>,
    #[serde(default)]
    default: Option<Expr<Any>>,
    #[serde(default)]
    check: Option<Expr<Any>>,
}

/// A parameter's value: a type, or an object with the type in it.
struct ParamValue(ParamObject);

impl<'de> FromKind<'de> for ParamValue {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<ParamValue, A::Error> {
        match kind {
            Kind::Object => value.read::<ParamObject>().map(ParamValue),
            // Anything else is read as a string, so that a value of the
            // wrong kind is reported as any wrong value is.
            _ => Ok(ParamValue(ParamObject {
                ty: value.read::<Located<String>>()?,
                of: None,
                min: None,
                max: None,
                default: None,
                check: None,
            })),
        }
    }
}

impl<'de> Deserialize<'de> for ParamValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ParamValue, D::Error> {
        by_kind(deserializer)
    }
}

/// The parameters of an action, in the order written, which is the order
/// defaults are worked out in (04 §5).
#[derive(Debug, Default)]
pub(crate) struct Params(pub(crate) Vec<Param>);

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Params, D::Error> {
        struct ParamsVisitor;

        impl<'de> Visitor<'de> for ParamsVisitor {
            type Value = Params;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of parameters, each a name and its type")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Params, A::Error> {
                let mut params = Vec::new();
                while let Some(name) = map.next_key::<Located<String>>()? {
                    let ParamValue(object) = map.next_value::<ParamValue>()?;
                    params.push(Param {
                        name,
                        written_type: object.ty,
                        of: object.of,
                        default: object.default,
                        check: object.check,
                        min: object.min,
                        max: object.max,
                        ty: Type::Any,
                        list: None,
                        slot: 0,
                    });
                }
                Ok(Params(params))
            }
        }

        deserializer.deserialize_map(ParamsVisitor)
    }
}

/// One variable of an action (03 §6.4): a local every phase and effect of
/// the running action shares.
///
/// ```json
/// "vars": { "power": { "type": "int", "default": 0 }, "landed": "bool" }
/// ```
///
/// Read through [`ActionDef::vars`](super::ActionDef::vars).
#[derive(Debug)]
pub struct Var {
    /// The name, read and assigned as `$name`.
    pub name: Located<String>,
    /// The type, as written.
    pub written_type: Located<String>,
    /// What the variable holds when the action starts: a constant of its
    /// type. With none it starts as null.
    pub default: Option<Expr<Any>>,
    ty: Type,
    slot: u16,
}

impl Var {
    /// A variable the action does not write itself but is given because
    /// of something else it declares: the locals the `strike` effect
    /// leaves in an action with `attacks` (07 §3.4). It is declared at
    /// `at`, with a type as written and a constant it starts as.
    pub(crate) fn implied(name: &str, written_type: &str, default: Expr<Any>, at: Span) -> Var {
        Var {
            name: Located {
                value: name.to_owned(),
                span: at,
            },
            written_type: Located {
                value: written_type.to_owned(),
                span: at,
            },
            default: Some(default),
            ty: Type::Any,
            slot: 0,
        }
    }

    /// The type of the variable's value: the declared one, or that or
    /// null for a variable with no default.
    pub fn ty(&self) -> &Type {
        &self.ty
    }

    /// The slot of the variable among the action's locals.
    pub fn slot(&self) -> u16 {
        self.slot
    }
}

/// The object form of a variable.
#[derive(Deserialize)]
struct VarObject {
    #[serde(rename = "type")]
    ty: Located<String>,
    #[serde(default)]
    default: Option<Expr<Any>>,
}

/// A variable's value: a type, or an object with the type in it.
struct VarValue(VarObject);

impl<'de> FromKind<'de> for VarValue {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<VarValue, A::Error> {
        match kind {
            Kind::Object => value.read::<VarObject>().map(VarValue),
            _ => Ok(VarValue(VarObject {
                ty: value.read::<Located<String>>()?,
                default: None,
            })),
        }
    }
}

impl<'de> Deserialize<'de> for VarValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<VarValue, D::Error> {
        by_kind(deserializer)
    }
}

/// The variables of an action, in the order written.
#[derive(Debug, Default)]
pub(crate) struct Vars(pub(crate) Vec<Var>);

impl<'de> Deserialize<'de> for Vars {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Vars, D::Error> {
        struct VarsVisitor;

        impl<'de> Visitor<'de> for VarsVisitor {
            type Value = Vars;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of variables, each a name and its type")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vars, A::Error> {
                let mut vars = Vec::new();
                while let Some(name) = map.next_key::<Located<String>>()? {
                    let VarValue(object) = map.next_value::<VarValue>()?;
                    vars.push(Var {
                        name,
                        written_type: object.ty,
                        default: object.default,
                        ty: Type::Any,
                        slot: 0,
                    });
                }
                Ok(Vars(vars))
            }
        }

        deserializer.deserialize_map(VarsVisitor)
    }
}

/// Where the value of one of an action's locals comes from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LocalSource {
    /// The window a reaction answers, or null if the action was not
    /// started as one (03 §7.3).
    Window,
    /// The participant in a role of a joint action, by the role's
    /// position (03 §8).
    Role(u16),
    /// A parameter, by its position.
    Param(u16),
    /// A variable of the action, by its position.
    Var(u16),
}

/// One local of an action: a name its expressions and effects write as
/// `$name`, with its type and where its value comes from.
///
/// [`ActionDef::locals`](super::ActionDef::locals) gives them in slot
/// order: a running action keeps one value for each, in this order, and
/// that is the order every expression of the action was compiled
/// against.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Local {
    /// The name, without the `$`.
    pub name: String,
    /// The type of its value.
    pub ty: Type,
    /// Where its value comes from.
    pub source: LocalSource,
}

/// Adds locals to a scope, in order.
pub(crate) fn with_locals(mut scope: Scope, locals: &[Local]) -> Scope {
    for local in locals {
        scope = scope.local(local.name.clone(), local.ty.clone());
    }
    scope
}

/// Whether `name` can be written `$name`: an id part that is not a word
/// of the expression language.
pub(crate) fn is_local_name(name: &str) -> bool {
    is_id_part(name) && !is_keyword(name)
}

/// What is wrong with a name that cannot be a local.
pub(crate) const LOCAL_NAME_RULE: &str = "a name starts with a lowercase letter, holds lowercase letters, digits and `_`, and is not a word of the expression language";

/// Reports an error of the action's own at a place in the file.
pub(crate) fn refuse(cx: &mut CompileCx<'_>, span: Span, path: &str, message: impl Into<String>) {
    let diagnostic = Diagnostic::new(code::VALIDATION, message)
        .at(cx.sources(), span)
        .with_path(path);
    cx.report(diagnostic);
}

/// Reads a type as a parameter or a variable writes it: as the language
/// writes types (05 §3), with `cell` for a location.
fn parse_written(cx: &mut CompileCx<'_>, written: &Located<String>, path: &str) -> Option<Type> {
    let name = written.value.trim();
    let (bare, nullable) = match name.strip_suffix('?') {
        Some(bare) => (bare.trim_end(), true),
        None => (name, false),
    };
    if bare == CELL_TYPE {
        let spelled = if nullable {
            format!("{CELL_KIND}?")
        } else {
            CELL_KIND.to_owned()
        };
        return cx.parse_type(&spelled, Some(written.span), path);
    }
    cx.parse_type(name, Some(written.span), path)
}

impl Param {
    /// Works out the parameter's type and list bounds. `false`, with the
    /// reasons reported, if the declaration is wrong.
    pub(crate) fn resolve(&mut self, slot: u16, cx: &mut CompileCx<'_>) -> bool {
        let path = format!("params.{}", self.name.value);
        self.slot = slot;
        let mut sound = true;
        let ty = if self.written_type.value.trim() == LIST_TYPE {
            match &self.of {
                Some(of) => parse_written(cx, of, &format!("{path}.of")).map(Type::list),
                None => {
                    refuse(
                        cx,
                        self.written_type.span,
                        &format!("{path}.type"),
                        "a list parameter says what it is a list of: add `\"of\"` with the type of its elements",
                    );
                    None
                }
            }
        } else {
            if let Some(of) = &self.of {
                refuse(
                    cx,
                    of.span,
                    &format!("{path}.of"),
                    "`of` belongs with the type `list`, which this parameter is not",
                );
                sound = false;
            }
            parse_written(cx, &self.written_type, &format!("{path}.type"))
        };
        let Some(ty) = ty else {
            return false;
        };
        let is_list = matches!(ty.non_null(), Type::List(_));
        if is_list {
            let min = self.min.as_ref().map_or(0, |min| min.value);
            let max = self.max.as_ref().map(|max| max.value);
            if let (Some(greatest), Some(written)) = (max, &self.max)
                && greatest < min
            {
                refuse(
                    cx,
                    written.span,
                    &format!("{path}.max"),
                    format!(
                        "the greatest length of this list, {greatest}, is below its least, {min}"
                    ),
                );
                sound = false;
            }
            self.list = Some(ListBounds { min, max });
        } else {
            for (bound, key) in [(&self.min, "min"), (&self.max, "max")] {
                if let Some(bound) = bound {
                    refuse(
                        cx,
                        bound.span,
                        &format!("{path}.{key}"),
                        format!(
                            "`{key}` is the length of a list, and this parameter is not one: it is `{}`",
                            cx.registry().type_name(&ty)
                        ),
                    );
                    sound = false;
                }
            }
        }
        self.ty = ty;
        sound
    }
}

impl Var {
    /// Works out the variable's type and checks its default, which must
    /// be a constant of that type. `false`, with the reasons reported,
    /// if the declaration is wrong.
    pub(crate) fn resolve(&mut self, slot: u16, cx: &mut CompileCx<'_>) -> bool {
        let path = format!("vars.{}", self.name.value);
        self.slot = slot;
        let Some(ty) = parse_written(cx, &self.written_type, &format!("{path}.type")) else {
            return false;
        };
        let mut sound = true;
        match &mut self.default {
            Some(default) => {
                cx.compile(default, &Scope::new().pure().expecting(ty.clone()));
                if default.is_compiled() && default.constant().is_none() {
                    let span = default.span().unwrap_or(self.name.span);
                    refuse(
                        cx,
                        span,
                        &format!("{path}.default"),
                        "a variable starts as a constant; set it from a phase's effects to give it a value worked out when the action runs",
                    );
                    sound = false;
                }
                self.ty = ty;
            }
            // With no default the variable is null until an effect sets
            // it.
            None => self.ty = ty.nullable(),
        }
        sound
    }
}
