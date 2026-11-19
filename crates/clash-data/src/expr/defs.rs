//! The `function` and `variable` definition types (05 §6, §12), and
//! what a finished load keeps of them: [`ExprDefs`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use clash_core::define_id;
use clash_expr::{FunctionId, KindId, RuntimeWarning, Type, Value, VariableId};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

use super::ExprRegistry;
use super::codes;
use super::field::{Any, Expr, Ready};
use super::scope::Origin;
use crate::defs::Def;
use crate::diag::{Diagnostic, Severity};
use crate::fields::{FromKind, Kind, KindValue, Located, by_kind};
use crate::source::SourceMap;

define_id! {
    /// A `function` definition.
    pub struct FunctionDefId;
    /// A `variable` definition.
    pub struct VariableDefId;
}

/// One parameter of a [`FunctionDef`], as written.
#[derive(Debug)]
pub struct ParamDecl {
    /// The name, read in the body as `$name`.
    pub name: Located<String>,
    /// The type, as the language writes types (05 §3).
    pub ty: Located<String>,
    /// The value the parameter has when a call leaves it out. A
    /// constant.
    pub default: Option<Expr<Any>>,
    /// What the parameter means.
    pub doc: String,
}

/// The object form of a parameter: `{ "type": "real", "default": 1.0 }`.
#[derive(Deserialize)]
struct ParamObject {
    #[serde(rename = "type")]
    ty: Located<String>,
    #[serde(default)]
    default: Option<Expr<Any>>,
    #[serde(default)]
    doc: String,
}

/// A parameter's value: a type, or an object with the type in it.
struct ParamValue {
    ty: Located<String>,
    default: Option<Expr<Any>>,
    doc: String,
}

impl<'de> FromKind<'de> for ParamValue {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<ParamValue, A::Error> {
        match kind {
            Kind::Object => {
                let object = value.read::<ParamObject>()?;
                Ok(ParamValue {
                    ty: object.ty,
                    default: object.default,
                    doc: object.doc,
                })
            }
            // Anything else is read as a string, so that a value of the
            // wrong kind is reported as any wrong value is.
            _ => Ok(ParamValue {
                ty: value.read::<Located<String>>()?,
                default: None,
                doc: String::new(),
            }),
        }
    }
}

impl<'de> Deserialize<'de> for ParamValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ParamValue, D::Error> {
        by_kind(deserializer)
    }
}

/// The parameters of a function, in the order written: the order
/// arguments are given in (04 §5).
#[derive(Debug, Default)]
pub struct Params(pub Vec<ParamDecl>);

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
                    let value = map.next_value::<ParamValue>()?;
                    params.push(ParamDecl {
                        name,
                        ty: value.ty,
                        default: value.default,
                        doc: value.doc,
                    });
                }
                Ok(Params(params))
            }
        }

        deserializer.deserialize_map(ParamsVisitor)
    }
}

/// A function (05 §12): a named, parameterised value.
///
/// ```json
/// { "type": "function", "id": "fall_energy",
///   "params": { "height": "real", "mass": { "type": "real", "default": 70 } },
///   "returns": "real",
///   "value": "$mass * 9.81 * $height",
///   "doc": "The energy of a fall, in joules." }
/// ```
///
/// It is called from expressions as `fall_energy(3)`, and from a value
/// source as `{ "call": "fall_energy", "args": { "height": 3 } }`.
/// Functions may call each other, and themselves to a fixed depth.
/// Whether a function is random, and whether it is safe for belief, is
/// worked out from its body and from the functions it calls.
#[derive(Debug, Deserialize)]
pub struct FunctionDef {
    /// The parameters, in order. The body reads each as `$name`.
    #[serde(default)]
    pub params: Params,
    /// The type of the result. Left out, the result is `any`, which is
    /// checked when the function runs.
    #[serde(default)]
    pub returns: Option<Located<String>>,
    /// The body: an expression or a value source.
    pub value: Expr<Any>,
    /// What the function computes, for the reference.
    #[serde(default)]
    pub doc: String,
}

impl Def for FunctionDef {
    type Id = FunctionDefId;
    const TYPE: &'static str = "function";
    const DOC: &'static str = "A named, parameterised value that expressions call (05 §12).";
}

/// A variable declaration (05 §6).
///
/// ```json
/// { "type": "variable", "id": "alarm_raised",
///   "scope": "site", "value": "bool", "default": false,
///   "doc": "Set once the vault alarm has gone off." }
/// ```
#[derive(Debug, Deserialize)]
pub struct VariableDef {
    /// What the variable lives on: a kind of subject, by the names of
    /// 05 §6 (`entity`, `cell`, `site`, `global`, …).
    pub scope: Located<String>,
    /// The type of its value, as the language writes types.
    pub value: Located<String>,
    /// What it reads as until it is set. A constant of the declared
    /// type. Left out, the variable reads as null until it is set, and
    /// its type is "the declared type, or null".
    #[serde(default)]
    pub default: Option<Expr<Any>>,
    /// What the variable is for. Shown in the inspector.
    #[serde(default)]
    pub doc: String,
    /// Whether a change to it emits `var_changed`.
    #[serde(default)]
    pub watch: bool,
    /// Whether a character may read it of things other than itself, in a
    /// mind context (05 §7.3).
    #[serde(default)]
    pub belief_safe: bool,
}

impl Def for VariableDef {
    type Id = VariableDefId;
    const TYPE: &'static str = "variable";
    const DOC: &'static str =
        "A declared variable: what it lives on, its type and its default (05 §6).";
}

/// One parameter of a compiled function.
#[derive(Clone, Debug)]
pub struct FunctionParam {
    /// The name.
    pub name: String,
    /// The type an argument must have.
    pub ty: Type,
    /// The value it has when a call leaves it out, if it may be left
    /// out.
    pub default: Option<Value>,
}

/// A function definition after the compile stage.
#[derive(Clone, Debug)]
pub struct CompiledFunction {
    /// The qualified id: `ashlands:fall_energy`.
    pub name: String,
    /// The id the registry knows it by, if it can be called by name. One
    /// written in place inside another definition cannot.
    pub function: Option<FunctionId>,
    /// The parameters, in order.
    pub params: Vec<FunctionParam>,
    /// The type of the result.
    pub returns: Type,
    /// Whether it draws random numbers, itself or through what it calls.
    /// A random function is refused where purity is required.
    pub random: bool,
    /// Whether a character may call it in a mind context: whether
    /// everything it reads, itself or through what it calls, is safe for
    /// belief.
    pub belief_safe: bool,
    /// What it computes.
    pub doc: String,
    pub(crate) body: Option<Ready>,
    pub(crate) origin: u64,
}

impl CompiledFunction {
    /// Whether the body compiled. A function whose body did not is
    /// broken, and gives null.
    pub fn is_compiled(&self) -> bool {
        self.body.is_some()
    }
}

/// The compiled functions of a load, by definition id and by the id the
/// expression registry gave each.
#[derive(Clone, Debug, Default)]
pub struct Functions {
    pub(crate) items: Vec<CompiledFunction>,
    /// For each registry function id, the position in `items` plus one;
    /// zero for a function that is not defined in data.
    pub(crate) by_function: Vec<u32>,
}

impl Functions {
    /// The function defined in data that the registry knows by this id.
    /// `None` for one of the engine's.
    pub fn get(&self, function: FunctionId) -> Option<&CompiledFunction> {
        let at = *self.by_function.get(function.index())?;
        self.items.get((at as usize).checked_sub(1)?)
    }

    /// The function of a `function` definition.
    pub fn of(&self, id: FunctionDefId) -> Option<&CompiledFunction> {
        self.items.get(id.index())
    }

    /// Every function, in order of definition id.
    pub fn iter(&self) -> impl Iterator<Item = (FunctionDefId, &CompiledFunction)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| Some((FunctionDefId::from_index(index)?, item)))
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn index(&mut self, function: FunctionId, at: usize) {
        let slot = function.index();
        if self.by_function.len() <= slot {
            self.by_function.resize(slot + 1, 0);
        }
        self.by_function[slot] = crate::source::to_u32(at + 1);
    }
}

/// A variable declaration after the compile stage.
#[derive(Clone, Debug)]
pub struct VariableDecl {
    /// The qualified name: `core:alarm_raised`.
    pub name: String,
    /// The id the registry knows it by.
    pub variable: VariableId,
    /// The kind of subject it lives on.
    pub kind: KindId,
    /// The type of its value.
    pub ty: Type,
    /// What it reads as until it is set. Null if no default was
    /// declared.
    pub default: Value,
    /// Whether a change to it emits `var_changed`.
    pub watch: bool,
    /// Whether it is safe for belief.
    pub belief_safe: bool,
    /// What it is for.
    pub doc: String,
}

/// The variable declarations of a load, by qualified name.
#[derive(Clone, Debug, Default)]
pub struct Variables {
    pub(crate) items: BTreeMap<String, VariableDecl>,
}

impl Variables {
    /// The declaration of a variable, by qualified name.
    pub fn get(&self, name: &str) -> Option<&VariableDecl> {
        self.items.get(name)
    }

    /// What a variable reads as before it is set: its declared default,
    /// and null for one that is not declared or declares none. This is
    /// what a host answers `Host::variable` with for a variable that was
    /// never written.
    pub fn default_of(&self, name: &str) -> Value {
        self.items
            .get(name)
            .map_or(Value::Null, |decl| decl.default.clone())
    }

    /// Every declaration, in order of qualified name.
    pub fn iter(&self) -> impl Iterator<Item = &VariableDecl> {
        self.items.values()
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The expression side of a finished load: the registry everything was
/// compiled against, the functions and variables mods define, and where
/// each compiled expression came from.
#[derive(Clone, Debug)]
pub struct ExprDefs {
    pub(crate) registry: Arc<ExprRegistry>,
    pub(crate) functions: Functions,
    pub(crate) variables: Variables,
    pub(crate) origins: Vec<Origin>,
}

impl Default for ExprDefs {
    fn default() -> ExprDefs {
        ExprDefs {
            registry: Arc::new(ExprRegistry::with_builtins()),
            functions: Functions::default(),
            variables: Variables::default(),
            origins: Vec::new(),
        }
    }
}

impl ExprDefs {
    /// The registry: what the host supplied, with the definition ids,
    /// functions and variables of the loaded mods added.
    pub fn registry(&self) -> &Arc<ExprRegistry> {
        &self.registry
    }

    /// The functions mods define.
    pub fn functions(&self) -> &Functions {
        &self.functions
    }

    /// The variables mods declare.
    pub fn variables(&self) -> &Variables {
        &self.variables
    }

    /// Where the expression compiled with an origin number came from:
    /// the definition, the JSON path and the string that holds it.
    /// Run-time warnings carry the number (05 §4.5).
    pub fn origin(&self, origin: u64) -> Option<&Origin> {
        let index = usize::try_from(origin).ok()?.checked_sub(1)?;
        self.origins.get(index)
    }

    /// A run-time warning as a diagnostic: its code in group `069`, the
    /// definition and JSON path of the expression it came from and the
    /// position inside the string.
    pub fn warning_diagnostic(&self, sources: &SourceMap, warning: &RuntimeWarning) -> Diagnostic {
        let code = codes::for_expr_code(warning.code()).unwrap_or(codes::RUN_TYPE);
        let mut diagnostic =
            Diagnostic::new(code, warning.fault.to_string()).with_severity(Severity::Warning);
        if let Some(origin) = self.origin(warning.origin) {
            diagnostic = diagnostic
                .in_def(Some(&origin.definition))
                .with_path(origin.path.clone());
            if let Some(span) = origin.span {
                let span = sources.span_in_string(
                    span,
                    warning.span.start as usize,
                    warning.span.end as usize,
                );
                diagnostic = diagnostic.at(sources, span);
            }
        }
        diagnostic
    }
}
