//! The expression fields: [`Expr<T>`], [`Cond`] and the compiling of
//! [`Text`] (04 §7.1; 05 §7).

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use clash_core::{DenseId, Real};
use clash_expr::{Compiled, FunctionId, Host, Type, Value};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess};

use super::ExprRegistry;
use super::codes;
use super::eval::{Evaluator, Runner};
use super::scope::{CompileCx, Scope, SpanMap};
use crate::defs::{Def, Defs};
use crate::diag::{Diagnostic, closest, did_you_mean};
use crate::fields::{FromSite, Kind, KindValue, Located, Site, Text, by_site};
use crate::source::Span;
use crate::units::parse_quantity;

/// Something [`CompileCx::compile`] can compile against a [`Scope`]: an
/// expression field, or a container of them.
pub trait Compile {
    /// Compiles this against `scope`, reporting through `cx`.
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope);
}

impl<F: Compile> Compile for Option<F> {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        if let Some(field) = self {
            field.compile_in(cx, scope);
        }
    }
}

impl<F: Compile + ?Sized> Compile for Box<F> {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        (**self).compile_in(cx, scope);
    }
}

impl<F: Compile> Compile for [F] {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        for field in self {
            field.compile_in(cx, scope);
        }
    }
}

impl<F: Compile> Compile for Vec<F> {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        self.as_mut_slice().compile_in(cx, scope);
    }
}

impl<K: Ord, F: Compile> Compile for BTreeMap<K, F> {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        for field in self.values_mut() {
            field.compile_in(cx, scope);
        }
    }
}

/// What an [`Expr<T>`] gives: the type its value must have, and how the
/// value is handed back to Rust.
pub trait ExprType: 'static {
    /// What evaluating gives.
    type Out;

    /// The type the field wants, for the checker.
    fn expected(registry: &ExprRegistry) -> Type;

    /// The typed result out of a value; `None` for null or a value of
    /// another type.
    fn from_value(value: Value) -> Option<Self::Out>;
}

/// An [`Expr`] that gives a number: a `real`, for which an `int` is
/// accepted. Evaluates to a [`Real`].
#[derive(Clone, Copy, Debug)]
pub struct Number;

/// An [`Expr`] that gives a whole number. Evaluates to an `i64`.
#[derive(Clone, Copy, Debug)]
pub struct Int;

/// An [`Expr`] that gives `true` or `false`. For a condition, which may
/// also be a structure and counts null as false, use [`Cond`].
#[derive(Clone, Copy, Debug)]
pub struct Bool;

/// An [`Expr`] that gives a text. A JSON string in such a field is the
/// text itself; an expression is written `{ "expr": "…" }` (05 §7.1).
#[derive(Clone, Copy, Debug)]
pub struct Str;

/// An [`Expr`] that may give anything. Evaluates to a [`Value`].
#[derive(Clone, Copy, Debug)]
pub struct Any;

/// An [`Expr`] that gives the id of a definition of type `T`. A JSON
/// string in such a field is an id, checked when mods load; an
/// expression is written `{ "expr": "…" }` (05 §7.1). Evaluates to
/// `T::Id`.
pub struct IdOf<T: Def>(PhantomData<fn() -> T>);

impl ExprType for Number {
    type Out = Real;

    fn expected(_registry: &ExprRegistry) -> Type {
        Type::Real
    }

    fn from_value(value: Value) -> Option<Real> {
        value.as_number()
    }
}

impl ExprType for Int {
    type Out = i64;

    fn expected(_registry: &ExprRegistry) -> Type {
        Type::Int
    }

    fn from_value(value: Value) -> Option<i64> {
        value.as_int()
    }
}

impl ExprType for Bool {
    type Out = bool;

    fn expected(_registry: &ExprRegistry) -> Type {
        Type::Bool
    }

    fn from_value(value: Value) -> Option<bool> {
        value.as_bool()
    }
}

impl ExprType for Str {
    type Out = Arc<str>;

    fn expected(_registry: &ExprRegistry) -> Type {
        Type::Str
    }

    fn from_value(value: Value) -> Option<Arc<str>> {
        match value {
            Value::Str(text) => Some(text),
            _ => None,
        }
    }
}

impl ExprType for Any {
    type Out = Value;

    fn expected(_registry: &ExprRegistry) -> Type {
        Type::Any
    }

    fn from_value(value: Value) -> Option<Value> {
        Some(value)
    }
}

impl<T: Def> ExprType for IdOf<T> {
    type Out = T::Id;

    fn expected(registry: &ExprRegistry) -> Type {
        // A type the loader was not given has no ids to check against.
        registry.def_type_id(T::TYPE).map_or(Type::Any, Type::Id)
    }

    fn from_value(value: Value) -> Option<T::Id> {
        value
            .as_id()
            .and_then(|id| T::Id::from_index(id.index as usize))
    }
}

/// A query (05 §11), kept as it was written. Queries are built by a
/// later package; until then one is read, kept and gives null with a
/// run-time warning if it is evaluated.
#[derive(Clone, PartialEq, Debug)]
pub struct Query {
    /// The query as JSON.
    pub raw: serde_json::Value,
    /// Where it was written.
    pub span: Span,
}

impl<'de> Deserialize<'de> for Query {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Query, D::Error> {
        let located = Located::<serde_json::Value>::deserialize(deserializer)?;
        Ok(Query {
            raw: located.value,
            span: located.span,
        })
    }
}

/// What a query's results are made into (05 §7.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueryOp {
    /// How many.
    Count,
    /// A list of them.
    Collect,
    /// The first, or null.
    First,
    /// One at random.
    Pick,
    /// A total.
    Sum,
    /// The least.
    Min,
    /// The greatest.
    Max,
}

impl QueryOp {
    /// The key the source is written under.
    pub const fn key(self) -> &'static str {
        match self {
            QueryOp::Count => "count",
            QueryOp::Collect => "collect",
            QueryOp::First => "first",
            QueryOp::Pick => "pick",
            QueryOp::Sum => "sum",
            QueryOp::Min => "min",
            QueryOp::Max => "max",
        }
    }
}

/// A value source (05 §7.4): an object that says a value an expression
/// string cannot. It is written where an [`Expr`] is, and read as part
/// of one.
#[derive(Clone, Debug)]
pub enum ValueSource {
    /// `{ "count": query }` and the like.
    Query {
        /// What is made of the results.
        op: QueryOp,
        /// The query.
        query: Query,
        /// For `sum`, `min` and `max`: the expression on `it` whose
        /// values are folded.
        of: Option<Located<String>>,
        /// For `pick`: the expression on `it` that weights the choice.
        weight: Option<Located<String>>,
    },
    /// `{ "call": "fn_id", "args": { … } }`: a function's result, with
    /// its arguments by parameter name.
    Call {
        /// The function, as written.
        function: Located<String>,
        /// The arguments.
        args: BTreeMap<String, Expr<Any>>,
    },
    /// `{ "run": { "eoc": id, "with": { … } } }`: what a rule returns.
    /// Allowed only where effects are.
    Run(Query),
}

/// What an [`Expr`] was written as (05 §7.1).
#[derive(Clone, Debug)]
pub enum Written {
    /// A JSON number, boolean, null or list: a literal.
    Literal(Value),
    /// A JSON string: an expression, or the literal itself where the
    /// field wants a text or an id.
    Text(String),
    /// `{ "expr": "…" }`: an expression, always.
    Code {
        /// The expression.
        source: String,
        /// Where the string that holds it was written.
        site: Site,
    },
    /// A value source.
    Source(Box<ValueSource>),
}

/// A compiled field, ready to evaluate.
#[derive(Clone, Debug)]
pub(crate) enum Ready {
    /// Known at load.
    Constant(Value),
    /// Bytecode.
    Code(Compiled),
    /// A call of a function with compiled arguments, in parameter order.
    Call(Arc<ReadyCall>),
    /// Something the engine does not offer yet; the text says what.
    Unavailable(&'static str, u64),
}

#[derive(Debug)]
pub(crate) struct ReadyCall {
    pub(crate) function: FunctionId,
    pub(crate) args: Vec<Ready>,
}

/// A string and where it was written, path included.
struct SitedString {
    text: String,
    site: Site,
}

impl<'de> FromSite<'de> for SitedString {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        _kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<SitedString, A::Error> {
        Ok(SitedString {
            text: value.read::<String>()?,
            site,
        })
    }
}

impl<'de> Deserialize<'de> for SitedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<SitedString, D::Error> {
        by_site(deserializer)
    }
}

/// A JSON literal read into a [`Value`]: numbers keep whether they were
/// whole, and a list holds literals.
struct Literal(Value);

impl<'de> Deserialize<'de> for Literal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Literal, D::Error> {
        struct LiteralVisitor;

        impl<'de> de::Visitor<'de> for LiteralVisitor {
            type Value = Literal;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number, a boolean, a text or a list of them")
            }

            fn visit_unit<E: de::Error>(self) -> Result<Literal, E> {
                Ok(Literal(Value::Null))
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Literal, E> {
                Ok(Literal(Value::Bool(value)))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Literal, E> {
                Ok(Literal(Value::Int(value)))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Literal, E> {
                Ok(Literal(match i64::try_from(value) {
                    Ok(int) => Value::Int(int),
                    // Too large for an int: the nearest real.
                    Err(_) => Value::Real(Real::of(value as f64)),
                }))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Literal, E> {
                Real::new(value)
                    .map(|real| Literal(Value::Real(real)))
                    .map_err(|_| E::custom("this is not a number"))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Literal, E> {
                Ok(Literal(Value::text(value)))
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Literal, A::Error> {
                let mut items = Vec::new();
                while let Some(Literal(item)) = seq.next_element::<Literal>()? {
                    items.push(item);
                }
                Ok(Literal(Value::list(items)))
            }

            fn visit_map<A: MapAccess<'de>>(self, _map: A) -> Result<Literal, A::Error> {
                Err(<A::Error as de::Error>::custom(
                    "a list written here holds numbers, booleans, texts and lists; write a map as `{ \"expr\": \"{…}\" }`",
                ))
            }
        }

        deserializer.deserialize_any(LiteralVisitor)
    }
}

/// The object forms of an expression field, as written.
#[derive(Deserialize)]
struct SourceObject {
    #[serde(default)]
    expr: Option<SitedString>,
    #[serde(default)]
    count: Option<Query>,
    #[serde(default)]
    collect: Option<Query>,
    #[serde(default)]
    first: Option<Query>,
    #[serde(default)]
    pick: Option<Query>,
    #[serde(default)]
    sum: Option<Query>,
    #[serde(default)]
    min: Option<Query>,
    #[serde(default)]
    max: Option<Query>,
    #[serde(default)]
    of: Option<Located<String>>,
    #[serde(default)]
    weight: Option<Located<String>>,
    #[serde(default)]
    call: Option<Located<String>>,
    #[serde(default)]
    args: Option<BTreeMap<String, Expr<Any>>>,
    #[serde(default)]
    run: Option<Query>,
}

impl SourceObject {
    fn into_written<E: de::Error>(self) -> Result<Written, E> {
        let SourceObject {
            expr,
            count,
            collect,
            first,
            pick,
            sum,
            min,
            max,
            of,
            weight,
            call,
            args,
            run,
        } = self;
        let queries = [
            (QueryOp::Count, count),
            (QueryOp::Collect, collect),
            (QueryOp::First, first),
            (QueryOp::Pick, pick),
            (QueryOp::Sum, sum),
            (QueryOp::Min, min),
            (QueryOp::Max, max),
        ];
        let heads = usize::from(expr.is_some())
            + usize::from(call.is_some())
            + usize::from(run.is_some())
            + queries.iter().filter(|(_, query)| query.is_some()).count();
        if heads != 1 {
            return Err(E::custom(
                "an object here is `{ \"expr\": \"…\" }` or one value source: it has exactly one of `expr`, `call`, `run`, `count`, `collect`, `first`, `pick`, `sum`, `min` and `max`",
            ));
        }
        let stray = |key: &str, belongs: &str| {
            E::custom(format!("`{key}` belongs with {belongs}, which is not here"))
        };
        if args.is_some() && call.is_none() {
            return Err(stray("args", "`call`"));
        }
        if let Some(expr) = expr {
            if of.is_some() || weight.is_some() {
                return Err(stray("of", "a query"));
            }
            return Ok(Written::Code {
                source: expr.text,
                site: expr.site,
            });
        }
        if let Some(function) = call {
            return Ok(Written::Source(Box::new(ValueSource::Call {
                function,
                args: args.unwrap_or_default(),
            })));
        }
        if let Some(rule) = run {
            return Ok(Written::Source(Box::new(ValueSource::Run(rule))));
        }
        for (op, query) in queries {
            let Some(query) = query else { continue };
            if weight.is_some() && op != QueryOp::Pick {
                return Err(stray("weight", "`pick`"));
            }
            if of.is_some() && !matches!(op, QueryOp::Sum | QueryOp::Min | QueryOp::Max) {
                return Err(stray("of", "`sum`, `min` or `max`"));
            }
            return Ok(Written::Source(Box::new(ValueSource::Query {
                op,
                query,
                of,
                weight,
            })));
        }
        Err(E::custom("this object is not a value source"))
    }
}

/// An expression field (04 §7.1; 05 §7.1): a value worked out when it is
/// asked for, of the type `T` says.
///
/// | Written | Meaning |
/// |---------|---------|
/// | `12`, `true`, `[1, 2]` | A literal |
/// | `"self.var.kills * 2"` | An expression, when `T` is not a text or an id |
/// | `"glass"` | The text or the id itself, when `T` is [`Str`] or [`IdOf`] |
/// | `"250 ms"`, `"5 m/s"` | A quantity in SI units, when `T` is a number |
/// | `{ "expr": "…" }` | An expression, always |
/// | `{ "call": "fn", "args": { … } }`, `{ "count": query }`, … | A [value source](ValueSource) |
///
/// `T` is one of [`Number`], [`Int`], [`Bool`], [`Str`], [`Any`] and
/// [`IdOf<D>`](IdOf). The field holds what was written and where; the
/// definition type that owns it compiles it in
/// [`Def::compile`](crate::Def::compile), saying what is in scope there,
/// and after that [`Expr::eval`] gives its typed value.
///
/// ```ignore
/// #[derive(serde::Deserialize)]
/// pub struct TrapDef {
///     pub energy: Expr<Number>,
/// }
///
/// impl Def for TrapDef {
///     // …
///     fn compile(&mut self, cx: &mut CompileCx<'_>) {
///         cx.compile(&mut self.energy, &Scope::new().subject("self", "entity"));
///     }
/// }
/// ```
pub struct Expr<T: ExprType = Any> {
    written: Written,
    site: Site,
    pub(crate) ready: Option<Ready>,
    marker: PhantomData<fn() -> T>,
}

impl<T: ExprType> Expr<T> {
    /// A field that is a constant, written nowhere: for a default or a
    /// placeholder made in Rust. It needs no compiling.
    pub fn literal(value: impl Into<Value>) -> Expr<T> {
        let value = value.into();
        Expr {
            written: Written::Literal(value.clone()),
            site: Site::default(),
            ready: Some(Ready::Constant(value)),
            marker: PhantomData,
        }
    }

    /// What was written.
    pub fn written(&self) -> &Written {
        &self.written
    }

    /// The expression's source, if the field was written as one.
    pub fn source(&self) -> Option<&str> {
        match &self.written {
            Written::Text(source) | Written::Code { source, .. } => Some(source),
            _ => None,
        }
    }

    /// Where the field was written, if it was read from a file.
    pub fn span(&self) -> Option<Span> {
        self.site.span
    }

    /// Where the field was written and at what JSON path.
    pub fn site(&self) -> &Site {
        &self.site
    }

    /// Whether the field has been compiled. One that has not evaluates
    /// to null.
    pub fn is_compiled(&self) -> bool {
        self.ready.is_some()
    }

    /// The bytecode, if the field compiled to an expression. A literal
    /// and a value source have none.
    pub fn compiled(&self) -> Option<&Compiled> {
        match &self.ready {
            Some(Ready::Code(code)) => Some(code),
            _ => None,
        }
    }

    /// The field's value, if it was worked out entirely at load.
    pub fn constant(&self) -> Option<&Value> {
        match &self.ready {
            Some(Ready::Constant(value)) => Some(value),
            Some(Ready::Code(code)) => code.constant(),
            _ => None,
        }
    }

    /// Evaluates the field. Null if it was never compiled.
    pub fn value<H: Host + ?Sized>(&self, eval: &mut Evaluator<'_, H>) -> Value {
        match &self.ready {
            Some(ready) => eval.run(ready),
            None => Value::Null,
        }
    }

    /// Evaluates the field to its typed result: `None` for null, or for
    /// a value of another type, which can only happen where the type was
    /// `any` at load.
    pub fn eval<H: Host + ?Sized>(&self, eval: &mut Evaluator<'_, H>) -> Option<T::Out> {
        T::from_value(self.value(eval))
    }

    /// Evaluates the field once against a host, with a [`Runner`] made
    /// for the occasion. For tools and tests; where fields are evaluated
    /// over and over, keep a runner and use [`Expr::eval`].
    pub fn eval_on<H: Host + ?Sized>(&self, defs: &Defs, host: &mut H) -> Option<T::Out> {
        self.eval(&mut Runner::new().on(defs, host))
    }
}

impl<T: ExprType> Clone for Expr<T> {
    fn clone(&self) -> Expr<T> {
        Expr {
            written: self.written.clone(),
            site: self.site.clone(),
            ready: self.ready.clone(),
            marker: PhantomData,
        }
    }
}

impl<T: ExprType> fmt::Debug for Expr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Expr")
            .field("written", &self.written)
            .field("site", &self.site)
            .field("compiled", &self.ready.is_some())
            .finish()
    }
}

impl<'de, T: ExprType> FromSite<'de> for Expr<T> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Expr<T>, A::Error> {
        let written = match kind {
            Kind::String => Written::Text(value.read::<String>()?),
            Kind::Object => value.read::<SourceObject>()?.into_written()?,
            Kind::Null | Kind::Bool | Kind::Number | Kind::List => {
                Written::Literal(value.read::<Literal>()?.0)
            }
        };
        Ok(Expr {
            written,
            site,
            ready: None,
            marker: PhantomData,
        })
    }
}

impl<'de, T: ExprType> Deserialize<'de> for Expr<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Expr<T>, D::Error> {
        by_site(deserializer)
    }
}

impl<T: ExprType> Compile for Expr<T> {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        let expected = cx.expected(scope, T::expected(cx.registry()));
        self.ready = compile_written(&mut self.written, &self.site, cx, scope, &expected);
    }
}

/// The type of a literal.
fn literal_type(value: &Value) -> Type {
    match value {
        Value::Null => Type::Null,
        Value::Bool(_) => Type::Bool,
        Value::Int(_) => Type::Int,
        Value::Real(_) => Type::Real,
        Value::Str(_) => Type::Str,
        Value::List(items) => {
            let mut joined: Option<Type> = None;
            for item in items.iter() {
                let ty = literal_type(item);
                joined = Some(match joined {
                    None => ty,
                    Some(so_far) => so_far.join(&ty),
                });
            }
            Type::list(joined.unwrap_or(Type::Any))
        }
        _ => Type::Any,
    }
}

/// A literal as the expression language writes it, for the cases the
/// checker must see: a text that is an id, a list that holds one.
fn literal_source(value: &Value, out: &mut String) -> bool {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(int) if *int >= 0 => out.push_str(&int.to_string()),
        Value::Str(text) => {
            out.push('\'');
            out.push_str(&text.replace('\'', "''"));
            out.push('\'');
        }
        Value::List(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if !literal_source(item, out) {
                    return false;
                }
            }
            out.push(']');
        }
        // Negative and real numbers are not literals of the language
        // (`-5` is a negation); they are kept as values instead.
        _ => return false,
    }
    true
}

/// Whether a type is, or holds, an id: what only the checker can turn a
/// text into.
fn wants_id(ty: &Type) -> bool {
    match ty.non_null() {
        Type::Id(_) => true,
        Type::List(element) => wants_id(element),
        _ => false,
    }
}

fn compile_written(
    written: &mut Written,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
    expected: &Type,
) -> Option<Ready> {
    match written {
        Written::Literal(value) => compile_literal(value, site, cx, scope, expected),
        Written::Text(text) => {
            let wanted = expected.non_null();
            if *wanted == Type::Str {
                // The text itself (05 §7.1), however long.
                return Some(Ready::Constant(Value::text(text.as_str())));
            }
            if matches!(wanted, Type::Id(_)) {
                // The id itself, which the checker resolves.
                let mut source = String::new();
                literal_source(&Value::text(text.as_str()), &mut source);
                return compile_code(&source, site, cx, scope, expected, SpanMap::Whole);
            }
            if wanted.is_numeric()
                && let Ok(quantity) = parse_quantity(text)
                && let Ok(real) = Real::new(quantity.value)
            {
                return compile_literal(&Value::Real(real), site, cx, scope, expected);
            }
            compile_code(text, site, cx, scope, expected, SpanMap::InString)
        }
        Written::Code { source, site } => {
            compile_code(source, site, cx, scope, expected, SpanMap::InString)
        }
        Written::Source(source) => compile_source(source, site, cx, scope, expected),
    }
}

fn compile_code(
    source: &str,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
    expected: &Type,
    map: SpanMap,
) -> Option<Ready> {
    let context = cx.context(scope, site);
    let outcome = clash_expr::compile(source, cx.registry(), &context, expected);
    cx.report_expr(outcome.diagnostics, site, map);
    outcome.value.map(Ready::Code)
}

fn compile_literal(
    value: &Value,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
    expected: &Type,
) -> Option<Ready> {
    // A text that must become an id is the checker's to resolve.
    if wants_id(expected) && matches!(value, Value::List(_)) {
        let mut source = String::new();
        if literal_source(value, &mut source) {
            return compile_code(&source, site, cx, scope, expected, SpanMap::Whole);
        }
    }
    // A whole number may be written with a zero fraction or an exponent
    // (04 §7.1).
    let whole = match value {
        Value::Real(real)
            if *expected.non_null() == Type::Int
                && real.get().fract() == 0.0
                && real.get().abs() < 9.0e15 =>
        {
            Some(Value::Int(real.get() as i64))
        }
        _ => None,
    };
    let value = whole.as_ref().unwrap_or(value);
    let found = literal_type(value);
    let fits = expected.accepts(&found)
        && (!value.is_null() || expected.may_be_null() || *expected == Type::Any);
    if !fits {
        let registry = cx.registry();
        let message = format!(
            "this must be {}, and it is {}",
            registry.type_name(expected),
            registry.type_name(&found)
        );
        cx.error_at(codes::RESULT_TYPE, site, message);
        return None;
    }
    Some(Ready::Constant(value.clone()))
}

fn compile_source(
    source: &mut ValueSource,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
    expected: &Type,
) -> Option<Ready> {
    match source {
        ValueSource::Query { op, .. } => {
            if *op == QueryOp::Pick && scope.is_pure() {
                cx.error_at(
                    codes::RANDOM_IN_PURE,
                    site,
                    "`pick` chooses at random, which is not allowed where the value must be pure",
                );
                return None;
            }
            Some(Ready::Unavailable("queries", cx.origin(site)))
        }
        ValueSource::Run(_) => {
            if scope.is_pure() {
                cx.error_at(
                    codes::RANDOM_IN_PURE,
                    site,
                    "`run` runs a rule, which is not allowed where the value must be pure",
                );
                return None;
            }
            Some(Ready::Unavailable(
                "rules as value sources",
                cx.origin(site),
            ))
        }
        ValueSource::Call { function, args } => {
            compile_call(function, args, site, cx, scope, expected)
        }
    }
}

fn compile_call(
    function: &Located<String>,
    args: &mut BTreeMap<String, Expr<Any>>,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
    expected: &Type,
) -> Option<Ready> {
    let registry = cx.registry();
    let sources = cx.sources();
    let at = Site {
        span: Some(function.span),
        path: site.child("call").path,
    };
    let context = cx.context(scope, &at);
    let resolved = registry
        .resolve_function(&function.value, context.scope())
        .and_then(|resolved| registry.function(resolved.found));
    let Some(found) = resolved else {
        let all = registry
            .functions()
            .filter(|known| !known.is_builtin())
            .map(|known| known.spec.name.as_str());
        let diagnostic = Diagnostic::new(
            codes::UNKNOWN_FUNCTION,
            format!("unknown function \"{}\"", function.value),
        )
        .at(sources, function.span)
        .with_path(at.path.clone())
        .suggest_opt(did_you_mean(closest(&function.value, all)));
        cx.report(diagnostic);
        for arg in args.values_mut() {
            arg.compile_in(cx, scope);
        }
        return None;
    };
    let spec = &found.spec;
    let mut sound = true;
    if found.is_builtin() || spec.variadic {
        cx.error_at(
            codes::BAD_SOURCE,
            &at,
            format!(
                "`{0}` is one of the engine's own functions: call it in an expression, `{0}(…)`",
                spec.name
            ),
        );
        for arg in args.values_mut() {
            arg.compile_in(cx, scope);
        }
        return None;
    }
    if spec.random && scope.is_pure() {
        cx.error_at(
            codes::RANDOM_IN_PURE,
            &at,
            format!(
                "`{}` draws random numbers, which is not allowed where the value must be pure",
                spec.name
            ),
        );
        sound = false;
    }
    if !spec.belief_safe && scope.is_belief() {
        cx.error_at(
            codes::NOT_BELIEF_SAFE,
            &at,
            format!(
                "`{}` is not safe for belief: it reads what a character may not know",
                spec.name
            ),
        );
        sound = false;
    }
    if !expected.accepts(&spec.returns) {
        cx.error_at(
            codes::RESULT_TYPE,
            &at,
            format!(
                "this must be {}, and `{}` gives {}",
                registry.type_name(expected),
                spec.name,
                registry.type_name(&spec.returns)
            ),
        );
        sound = false;
    }
    for (name, arg) in args.iter() {
        if spec.params.iter().all(|param| param.name != *name) {
            let known = spec.params.iter().map(|param| param.name.as_str());
            let mut diagnostic = Diagnostic::new(
                codes::ARGUMENT_COUNT,
                format!("`{}` has no parameter `{name}`", spec.name),
            )
            .with_path(arg.site().path.clone())
            .suggest_opt(did_you_mean(closest(name, known)));
            if let Some(span) = arg.span() {
                diagnostic = diagnostic.at(sources, span);
            }
            cx.report(diagnostic);
            sound = false;
        }
    }
    let defaults = cx.functions().get(found.id);
    let mut ready = Vec::with_capacity(spec.params.len());
    for (index, param) in spec.params.iter().enumerate() {
        match args.get_mut(&param.name) {
            Some(arg) => {
                arg.ready = compile_written(&mut arg.written, &arg.site, cx, scope, &param.ty);
                match &arg.ready {
                    Some(compiled) => ready.push(compiled.clone()),
                    None => sound = false,
                }
            }
            None if param.optional => {
                let default = defaults
                    .and_then(|function| function.params.get(index))
                    .and_then(|param| param.default.clone());
                ready.push(Ready::Constant(default.unwrap_or_default()));
            }
            None => {
                cx.error_at(
                    codes::ARGUMENT_COUNT,
                    &at,
                    format!(
                        "`{}` needs an argument for `{}`: {}",
                        spec.name,
                        param.name,
                        registry.signature(found)
                    ),
                );
                sound = false;
            }
        }
    }
    sound.then(|| {
        Ready::Call(Arc::new(ReadyCall {
            function: found.id,
            args: ready,
        }))
    })
}

/// A condition (05 §7.2): an expression that gives a boolean, or a
/// structure of them.
///
/// | Written | Meaning |
/// |---------|---------|
/// | `"self.var.rage > 50"` | An expression |
/// | `true`, `false` | A constant |
/// | `{ "all": [ … ] }`, `{ "any": [ … ] }`, `{ "not": … }` | A structure; the three nest freely |
/// | `{ "expr": "…" }` | An expression |
/// | `{ "exists": query }` | Whether a query has any result (not built yet: it counts as false) |
///
/// A condition that comes to null counts as false (05 §4.4).
/// `Cond::default()` always holds, which is what a rule with no `if`
/// means.
#[derive(Clone, Debug)]
pub struct Cond {
    kind: CondKind,
    site: Site,
}

#[derive(Clone, Debug)]
enum CondKind {
    Constant(bool),
    Code {
        source: String,
        compiled: Option<Compiled>,
    },
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
    Exists(Query, u64),
}

impl Default for Cond {
    fn default() -> Cond {
        Cond::constant(true)
    }
}

/// The object forms of a condition, as written.
#[derive(Deserialize)]
struct CondObject {
    #[serde(default)]
    all: Option<Vec<Cond>>,
    #[serde(default)]
    any: Option<Vec<Cond>>,
    #[serde(default)]
    not: Option<Box<Cond>>,
    #[serde(default)]
    exists: Option<Query>,
    #[serde(default)]
    expr: Option<SitedString>,
}

impl Cond {
    /// A condition that always or never holds, written nowhere.
    pub fn constant(holds: bool) -> Cond {
        Cond {
            kind: CondKind::Constant(holds),
            site: Site::default(),
        }
    }

    /// Where the condition was written, if it was read from a file.
    pub fn span(&self) -> Option<Span> {
        self.site.span
    }

    /// Where the condition was written and at what JSON path.
    pub fn site(&self) -> &Site {
        &self.site
    }

    /// The expression's source, if the condition is one expression.
    pub fn source(&self) -> Option<&str> {
        match &self.kind {
            CondKind::Code { source, .. } => Some(source),
            _ => None,
        }
    }

    /// The bytecode, if the condition is one expression and has been
    /// compiled.
    pub fn compiled(&self) -> Option<&Compiled> {
        match &self.kind {
            CondKind::Code { compiled, .. } => compiled.as_ref(),
            _ => None,
        }
    }

    /// The query, if the condition is `{ "exists": query }`.
    pub fn query(&self) -> Option<&Query> {
        match &self.kind {
            CondKind::Exists(query, _) => Some(query),
            _ => None,
        }
    }

    /// Whether the condition is the same every time, and which way.
    pub fn constant_value(&self) -> Option<bool> {
        match &self.kind {
            CondKind::Constant(holds) => Some(*holds),
            _ => None,
        }
    }

    /// Whether the condition holds, asked once against a host, with a
    /// [`Runner`] made for the occasion. For tools and tests.
    pub fn holds_on<H: Host + ?Sized>(&self, defs: &Defs, host: &mut H) -> bool {
        self.holds(&mut Runner::new().on(defs, host))
    }

    /// Whether the condition holds. Null counts as false, and so does a
    /// condition that was never compiled.
    pub fn holds<H: Host + ?Sized>(&self, eval: &mut Evaluator<'_, H>) -> bool {
        match &self.kind {
            CondKind::Constant(holds) => *holds,
            CondKind::Code { compiled, .. } => {
                compiled.as_ref().is_some_and(|code| eval.condition(code))
            }
            CondKind::All(parts) => parts.iter().all(|part| part.holds(eval)),
            CondKind::Any(parts) => parts.iter().any(|part| part.holds(eval)),
            CondKind::Not(inner) => !inner.holds(eval),
            CondKind::Exists(_, origin) => {
                eval.unavailable("queries", *origin);
                false
            }
        }
    }
}

impl<'de> FromSite<'de> for Cond {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Cond, A::Error> {
        let kind = match kind {
            Kind::Bool => CondKind::Constant(value.read::<bool>()?),
            Kind::Object => {
                let object = value.read::<CondObject>()?;
                let CondObject {
                    all,
                    any,
                    not,
                    exists,
                    expr,
                } = object;
                let heads = usize::from(all.is_some())
                    + usize::from(any.is_some())
                    + usize::from(not.is_some())
                    + usize::from(exists.is_some())
                    + usize::from(expr.is_some());
                if heads != 1 {
                    return Err(<A::Error as de::Error>::custom(
                        "a condition written as an object has exactly one of `all`, `any`, `not`, `exists` and `expr`",
                    ));
                }
                if let Some(parts) = all {
                    CondKind::All(parts)
                } else if let Some(parts) = any {
                    CondKind::Any(parts)
                } else if let Some(inner) = not {
                    CondKind::Not(inner)
                } else if let Some(query) = exists {
                    CondKind::Exists(query, 0)
                } else if let Some(expr) = expr {
                    return Ok(Cond {
                        kind: CondKind::Code {
                            source: expr.text,
                            compiled: None,
                        },
                        site: expr.site,
                    });
                } else {
                    CondKind::Constant(false)
                }
            }
            // Anything else is read as a string, so that a value of the
            // wrong kind is reported as any wrong value is.
            _ => CondKind::Code {
                source: value.read::<String>()?,
                compiled: None,
            },
        };
        Ok(Cond { kind, site })
    }
}

impl<'de> Deserialize<'de> for Cond {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Cond, D::Error> {
        by_site(deserializer)
    }
}

impl Compile for Cond {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        match &mut self.kind {
            CondKind::Constant(_) => {}
            CondKind::Code { source, compiled } => {
                let context = cx.context(scope, &self.site);
                let expected = Type::Bool.nullable();
                let outcome = clash_expr::compile(source, cx.registry(), &context, &expected);
                cx.report_expr(outcome.diagnostics, &self.site, SpanMap::InString);
                *compiled = outcome.value;
            }
            CondKind::All(parts) | CondKind::Any(parts) => parts.compile_in(cx, scope),
            CondKind::Not(inner) => inner.compile_in(cx, scope),
            CondKind::Exists(_, origin) => *origin = cx.origin(&self.site),
        }
    }
}

impl Compile for Text {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        let context = cx.context(scope, &self.site);
        let outcome = clash_expr::compile_template(&self.raw, cx.registry(), &context);
        cx.report_expr(outcome.diagnostics, &self.site, SpanMap::InString);
        self.compiled = outcome.value;
    }
}

impl Text {
    /// The text as it reads now: the template rendered against what the
    /// evaluator's host supplies. A text that was never compiled renders
    /// as it was written.
    pub fn render<H: Host + ?Sized>(&self, eval: &mut Evaluator<'_, H>) -> String {
        match &self.compiled {
            Some(compiled) => eval.text(compiled),
            None => self.raw.clone(),
        }
    }

    /// Renders the text once against a host, with a [`Runner`] made for
    /// the occasion. For tools and tests.
    pub fn render_on<H: Host + ?Sized>(&self, defs: &Defs, host: &mut H) -> String {
        self.render(&mut Runner::new().on(defs, host))
    }
}
