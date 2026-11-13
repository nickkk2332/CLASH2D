//! The checker: resolves every name of a syntax tree against a
//! [`Context`] and the [`Registry`], and checks types (05 §14, steps 2
//! and 3).
//!
//! What it produces is a checked tree in which nothing is left to look
//! up. What it reports is a list of [`Diagnostic`]s; checking carries on
//! after an error where it can, so that one load shows every mistake in
//! an expression.
//!
//! # What is checked
//!
//! - **Names.** A bare name is a bound subject; `$name` is a local;
//!   `event.field` is a field of the context's event; `subject.name` is a
//!   property of the subject's kind; `subject.var.name` is a variable;
//!   `name(…)` is a function. Anything else is an error, with the closest
//!   name that would have worked.
//! - **Types.** Operators and arguments get the types they accept. `any`
//!   passes everywhere and is checked when the expression runs.
//! - **Null.** A value the checker knows may be null, used where
//!   arithmetic or a function needs a value, is a warning that suggests
//!   `??` (05 §4.4).
//! - **Ids.** A text compared with an `id<T>`, or given where one is
//!   wanted, is resolved against the definitions of that type at load.
//!   One that names nothing is an error (05 §5.2).
//! - **Purity and belief.** A random function in a pure context and a
//!   read that is not safe for belief in a mind context are errors
//!   (05 §7.3).

use std::sync::Arc;

use crate::ast::{BinaryOp, Expr, ExprKind, Format, Name, Number, TemplatePart, UnaryOp};
use crate::context::{Context, Purity, Reads, Undeclared};
use crate::diag::{Code, Diagnostic, Span, closest_name, codes};
use crate::hir::{ArithOp, CmpOp, Node, NodeKind};
use crate::parser::{parse_expr, parse_template};
use crate::registry::{ParamSpec, Registry};
use crate::types::Type;
use crate::unit::Unit;
use crate::value::{DefTypeId, KindId, Value};
use crate::{Severity, builtins};

/// What a compilation step produced, and what it had to say.
///
/// `value` is `None` exactly when one of the diagnostics is an error.
#[derive(Clone, Debug)]
pub struct Outcome<T> {
    /// What was produced, unless there was an error.
    pub value: Option<T>,
    /// Errors, warnings and notes, in the order they were found.
    pub diagnostics: Vec<Diagnostic>,
}

impl<T> Outcome<T> {
    pub(crate) fn new(value: T, diagnostics: Vec<Diagnostic>) -> Outcome<T> {
        let failed = diagnostics.iter().any(Diagnostic::is_error);
        Outcome {
            value: (!failed).then_some(value),
            diagnostics,
        }
    }

    pub(crate) fn failed(diagnostics: Vec<Diagnostic>) -> Outcome<T> {
        Outcome {
            value: None,
            diagnostics,
        }
    }

    /// Whether any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// The diagnostics that are errors.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    /// The diagnostics that are warnings.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
    }

    /// What was produced, or the diagnostics if there was an error.
    /// Warnings are dropped: read `diagnostics` first to keep them.
    pub fn into_result(self) -> Result<T, Vec<Diagnostic>> {
        match self.value {
            Some(value) => Ok(value),
            None => Err(self.diagnostics),
        }
    }

    /// The same outcome with its value changed.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Outcome<U> {
        Outcome {
            value: self.value.map(f),
            diagnostics: self.diagnostics,
        }
    }
}

/// An expression whose names are resolved and whose types are checked.
///
/// It can be compiled to bytecode, or walked directly, which is slower
/// and is what tools that explain an expression build on.
#[derive(Clone, Debug)]
pub struct Checked {
    pub(crate) root: Box<Node>,
    pub(crate) origin: u64,
}

impl Checked {
    /// The type of the expression's value.
    pub fn ty(&self) -> &Type {
        &self.root.ty
    }

    /// The whole expression's place in its source.
    pub fn span(&self) -> Span {
        self.root.span
    }

    /// Whether the expression's value is the character's own (05 §7.3):
    /// it was reached from a subject or a local bound as own, through
    /// properties registered as yielding something of the same owner and
    /// through the elements of lists. Whoever binds this value by a name
    /// in another context, as a screen binds a row to `it`, may bind it
    /// as own there.
    pub fn is_own(&self) -> bool {
        self.root.own
    }
}

/// A text template whose expressions are checked.
#[derive(Clone, Debug)]
pub struct CheckedTemplate {
    pub(crate) parts: Vec<CheckedPart>,
}

/// One piece of a checked template.
#[derive(Clone, Debug)]
pub(crate) enum CheckedPart {
    Text(Arc<str>),
    Expr {
        expr: Checked,
        format: Option<Format>,
    },
}

/// Parses and checks an expression.
///
/// `expected` is the type the field wants: [`Type::Any`] for no
/// constraint, `Type::Bool.nullable()` for a condition (a condition that
/// is null counts as false). A text where an id is expected is resolved
/// at load.
///
/// ```
/// use clash_expr::testing::TestHost;
/// use clash_expr::{Type, check};
///
/// let host = TestHost::new();
/// let context = host.context();
/// let outcome = check("self.mass * 2 > 100", host.registry(), &context, &Type::Bool);
/// assert!(outcome.value.is_some());
///
/// let outcome = check("self.mas", host.registry(), &context, &Type::Any);
/// assert_eq!(outcome.diagnostics[0].code.as_str(), "X203");
/// assert_eq!(outcome.diagnostics[0].suggestion.as_deref(), Some("did you mean `mass`?"));
/// ```
pub fn check(
    source: &str,
    registry: &Registry,
    context: &Context,
    expected: &Type,
) -> Outcome<Checked> {
    let tree = match parse_expr(source) {
        Ok(tree) => tree,
        Err(error) => return Outcome::failed(vec![error]),
    };
    let mut checker = Checker {
        registry,
        context,
        diagnostics: Vec::new(),
    };
    let root = checker.check_root(&tree, expected);
    Outcome::new(
        Checked {
            root,
            origin: context.origin_id(),
        },
        checker.diagnostics,
    )
}

/// Parses and checks a text template. Each expression in it may have any
/// type; what is not text is turned into text when the template is
/// rendered.
pub fn check_template(
    source: &str,
    registry: &Registry,
    context: &Context,
) -> Outcome<CheckedTemplate> {
    let template = match parse_template(source) {
        Ok(template) => template,
        Err(errors) => return Outcome::failed(errors),
    };
    let mut checker = Checker {
        registry,
        context,
        diagnostics: Vec::new(),
    };
    let mut parts = Vec::new();
    for part in &template.parts {
        parts.push(match part {
            TemplatePart::Text(text) => CheckedPart::Text(Arc::from(text.as_str())),
            TemplatePart::Expr { expr, format, .. } => CheckedPart::Expr {
                expr: Checked {
                    root: checker.check_root(expr, &Type::Any),
                    origin: context.origin_id(),
                },
                format: *format,
            },
        });
    }
    Outcome::new(CheckedTemplate { parts }, checker.diagnostics)
}

const USE_COALESCE: &str = "use `??` to give a value for when it is null";

pub(crate) struct Checker<'a> {
    registry: &'a Registry,
    context: &'a Context,
    diagnostics: Vec<Diagnostic>,
}

fn is_var(name: &Name) -> bool {
    name.namespace.is_none() && name.name == "var"
}

fn did_you_mean<'c>(name: &str, candidates: impl IntoIterator<Item = &'c str>) -> Option<String> {
    closest_name(name, candidates).map(|close| format!("did you mean `{close}`?"))
}

impl<'a> Checker<'a> {
    pub(crate) const fn new(registry: &'a Registry, context: &'a Context) -> Checker<'a> {
        Checker {
            registry,
            context,
            diagnostics: Vec::new(),
        }
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn error(&mut self, code: Code, span: Span, message: String) {
        self.diagnostics
            .push(Diagnostic::error(code, span, message));
    }

    fn error_with(&mut self, code: Code, span: Span, message: String, hint: Option<String>) {
        self.diagnostics
            .push(Diagnostic::error(code, span, message).with_suggestion_opt(hint));
    }

    fn warning(&mut self, code: Code, span: Span, message: String, hint: &str) {
        self.diagnostics
            .push(Diagnostic::warning(code, span, message).with_suggestion(hint));
    }

    fn name_of(&self, ty: &Type) -> String {
        self.registry.type_name(ty)
    }

    /// What stands in for a subtree that could not be checked. Its type
    /// is `any`, so that one mistake is reported once.
    fn broken(span: Span) -> Box<Node> {
        Node::new(NodeKind::Const(Value::Null), Type::Any, span)
    }

    fn check_root(&mut self, tree: &Expr, expected: &Type) -> Box<Node> {
        let node = self.expr(tree);
        let node = self.coerce(node, expected);
        if !expected.accepts(&node.ty) {
            let message = format!(
                "this must be {}, and it is {}",
                self.name_of(expected),
                self.name_of(&node.ty)
            );
            self.error(codes::RESULT_TYPE, node.span, message);
        } else if node.ty.may_be_null() && !expected.may_be_null() && *expected != Type::Any {
            let message = format!(
                "this may be null where {} is needed",
                self.name_of(expected)
            );
            self.warning(codes::NULL_IN_ARITHMETIC, node.span, message, USE_COALESCE);
        }
        node
    }

    pub(crate) fn expr(&mut self, expr: &Expr) -> Box<Node> {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Null => Node::new(NodeKind::Const(Value::Null), Type::Null, span),
            ExprKind::Bool(value) => {
                Node::new(NodeKind::Const(Value::Bool(*value)), Type::Bool, span)
            }
            ExprKind::Number { value, unit } => Self::number(*value, *unit, span),
            ExprKind::Str(text) => Node::new(NodeKind::Const(Value::text(text)), Type::Str, span),
            ExprKind::Name(name) => self.name(name, span),
            ExprKind::Local(name) => self.local(name, span),
            ExprKind::List(items) => self.list(items, span),
            ExprKind::Map(entries) => self.map(entries, span),
            ExprKind::Member { object, name } => self.member(object, name, span),
            ExprKind::Call { callee, args } => self.call(callee, args, span),
            ExprKind::Index { object, index } => self.index(object, index, span),
            ExprKind::Unary { op, operand } => self.unary(*op, operand, span),
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs, span),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => self.if_else(cond, then, otherwise, span),
        }
    }

    fn number(value: Number, unit: Option<Unit>, span: Span) -> Box<Node> {
        let (value, ty) = match (value, unit) {
            (Number::Int(int), None) => (Value::Int(int), Type::Int),
            (Number::Real(real), None) => (Value::Real(real), Type::Real),
            (Number::Int(int), Some(unit)) => (
                Value::Real(unit.apply(clash_core::Real::from_i64(int))),
                Type::Real,
            ),
            (Number::Real(real), Some(unit)) => (Value::Real(unit.apply(real)), Type::Real),
        };
        Node::new(NodeKind::Const(value), ty, span)
    }

    // ---- names -----------------------------------------------------------

    fn name(&mut self, name: &Name, span: Span) -> Box<Node> {
        let written = name.to_string();
        if !name.is_qualified() {
            if let Some(slot) = self.context.subject_slot(&name.name) {
                let binding = &self.context.subjects()[usize::from(slot)];
                let mut node = Node::new(NodeKind::Subject(slot), binding.ty.clone(), span);
                node.own = binding.own;
                return node;
            }
            if name.name == "event" {
                let message = "`event` is read one field at a time: `event.name`".to_owned();
                self.error(codes::UNKNOWN_EVENT_FIELD, span, message);
                return Self::broken(span);
            }
        }
        let hint = if self
            .registry
            .resolve_function(&written, self.context.scope())
            .is_some()
        {
            Some(format!(
                "`{written}` is a function; call it: `{written}(…)`"
            ))
        } else if !name.is_qualified() && Unit::from_symbol(&name.name).is_some() {
            Some("compound units such as `m/s` are written in JSON unit strings".to_owned())
        } else {
            let bound = self.context.subjects().iter().map(|b| b.name.as_str());
            did_you_mean(&written, bound)
        };
        let message = format!("unknown name `{written}`: nothing of that name is bound here");
        self.error_with(codes::UNKNOWN_NAME, span, message, hint);
        Self::broken(span)
    }

    fn local(&mut self, name: &Name, span: Span) -> Box<Node> {
        let written = name.to_string();
        if let Some(slot) = self.context.local_slot(&written) {
            let binding = &self.context.locals()[usize::from(slot)];
            let mut node = Node::new(NodeKind::Local(slot), binding.ty.clone(), span);
            node.own = binding.own;
            return node;
        }
        let locals = self.context.locals().iter().map(|b| b.name.as_str());
        let hint = closest_name(&written, locals).map(|close| format!("did you mean `${close}`?"));
        let message = format!("unknown local `${written}`");
        self.error_with(codes::UNKNOWN_LOCAL, span, message, hint);
        Self::broken(span)
    }

    fn event_field(&mut self, name: &Name, span: Span) -> Box<Node> {
        let written = name.to_string();
        let event = self
            .context
            .event_id()
            .and_then(|id| self.registry.event(id));
        let Some(event) = event else {
            let message = format!("`event.{written}`: there is no event here to read a field of");
            self.error(codes::UNKNOWN_EVENT_FIELD, span, message);
            return Self::broken(span);
        };
        let found = event
            .spec
            .fields
            .iter()
            .position(|field| field.name == written);
        if let Some(index) = found {
            let slot = u16::try_from(index).unwrap_or(u16::MAX);
            let ty = event.spec.fields[index].ty.clone();
            return Node::new(NodeKind::EventField(slot), ty, span);
        }
        let fields = event.spec.fields.iter().map(|field| field.name.as_str());
        let hint = did_you_mean(&written, fields);
        let message = format!("event `{}` has no field `{written}`", event.spec.name);
        self.error_with(codes::UNKNOWN_EVENT_FIELD, span, message, hint);
        Self::broken(span)
    }

    // ---- members, calls, indexing ------------------------------------------

    fn member(&mut self, object: &Expr, name: &Name, span: Span) -> Box<Node> {
        if let ExprKind::Name(root) = &object.kind
            && !root.is_qualified()
            && root.name == "event"
            && self.context.subject_slot("event").is_none()
        {
            return self.event_field(name, span);
        }
        if let ExprKind::Member {
            object: owner,
            name: var,
        } = &object.kind
            && is_var(var)
        {
            return self.variable(owner, &name.to_string(), span);
        }
        if is_var(name) {
            // Check the owner for its own mistakes before complaining.
            self.expr(object);
            let message = "`var` is followed by a variable's name".to_owned();
            let hint = Some("write `.var.name`, or `.var('mod:name')` to name one exactly".into());
            self.error_with(codes::UNKNOWN_PROPERTY, span, message, hint);
            return Self::broken(span);
        }
        let object = self.expr(object);
        match object.ty.non_null().clone() {
            Type::Subject(kind) => self.property(object, kind, name, None, span),
            Type::Map(value) => Self::entry(object, &name.to_string(), value.nullable(), span),
            Type::Any => Self::entry(object, &name.to_string(), Type::Any, span),
            Type::Null => Self::entry(object, &name.to_string(), Type::Null, span),
            other => {
                let message = format!("`.{name}`: {} has no properties", self.name_of(&other));
                self.error(codes::NO_PROPERTIES, span, message);
                Self::broken(span)
            }
        }
    }

    /// `object.name` on a map: the entry for the key `name`.
    fn entry(object: Box<Node>, key: &str, ty: Type, span: Span) -> Box<Node> {
        let index = Node::new(NodeKind::Const(Value::text(key)), Type::Str, span);
        Node::new(NodeKind::Index { object, index }, ty, span)
    }

    fn property(
        &mut self,
        object: Box<Node>,
        kind: KindId,
        name: &Name,
        args: Option<&[Expr]>,
        span: Span,
    ) -> Box<Node> {
        let registry = self.registry;
        let written = name.to_string();
        let kind_name = registry
            .kind(kind)
            .map_or("?", |kind| kind.spec.name.as_str());
        let property = registry
            .property_id(kind, &written)
            .and_then(|id| registry.property(id));
        let Some(property) = property else {
            let names = registry
                .properties_of(kind)
                .map(|property| property.spec.name.as_str())
                .chain(["var"]);
            let hint = did_you_mean(&written, names);
            let message = format!("`{kind_name}` has no property `{written}`");
            self.error_with(codes::UNKNOWN_PROPERTY, span, message, hint);
            for arg in args.unwrap_or(&[]) {
                self.expr(arg);
            }
            return Self::broken(span);
        };
        let spec = &property.spec;
        if self.context.reads() == Reads::Belief && !spec.belief_safe && !object.own {
            let message = format!(
                "`{kind_name}.{written}` is not safe for belief: a character may read it only \
                 of itself"
            );
            let hint = Some("read what the character has perceived instead (15 §7)".to_owned());
            self.error_with(codes::NOT_BELIEF_SAFE, span, message, hint);
        }
        let what = format!("property `{kind_name}.{written}`");
        let signature = registry.property_signature(property);
        let args = self.arguments(
            &what,
            &signature,
            &spec.params,
            false,
            args.unwrap_or(&[]),
            span,
        );
        let ty = if object.ty.may_be_null() {
            spec.ty.clone().nullable()
        } else {
            spec.ty.clone()
        };
        let own = object.own && spec.yields_own;
        let mut node = Node::new(
            NodeKind::Property {
                object,
                id: property.id,
                args,
            },
            ty,
            span,
        );
        node.own = own;
        node
    }

    /// `owner.var.name` or `owner.var('mod:name')`.
    fn variable(&mut self, owner: &Expr, name: &str, span: Span) -> Box<Node> {
        let registry = self.registry;
        let scope = self.context.scope();
        let object = self.expr(owner);
        let kind = match object.ty.non_null() {
            Type::Subject(kind) => Some(*kind),
            Type::Any | Type::Null => None,
            other => {
                let message = format!(
                    "`.var`: {} has no variables; only subjects do",
                    self.name_of(other)
                );
                self.error(codes::NO_PROPERTIES, object.span, message);
                return Self::broken(span);
            }
        };
        let declared = registry
            .resolve_variable(name, scope)
            .and_then(|resolved| Some((registry.variable(resolved.found)?, resolved)));
        let (qualified, ty, safe) = match declared {
            Some((variable, resolved)) => {
                if let Some(other) = &resolved.also_in {
                    let message = format!(
                        "variable `{name}` is `{}`, and `{other}` has one of that name too",
                        resolved.name
                    );
                    self.warning(codes::AMBIGUOUS_NAME, span, message, "qualify the name");
                }
                if let Some(kind) = kind
                    && kind != variable.spec.kind
                {
                    let message = format!(
                        "variable `{}` is declared on `{}`, and this is `{}`",
                        resolved.name,
                        self.name_of(&Type::Subject(variable.spec.kind)),
                        self.name_of(&Type::Subject(kind))
                    );
                    self.error(codes::VARIABLE_SCOPE, span, message);
                }
                (
                    resolved.name,
                    variable.spec.ty.clone(),
                    variable.spec.belief_safe,
                )
            }
            None => {
                let qualified = match (&scope.namespace, name.contains(':')) {
                    (Some(namespace), false) => format!("{namespace}:{name}"),
                    _ => name.to_owned(),
                };
                let policy = self.context.undeclared_policy();
                if policy != Undeclared::Allow {
                    let all = registry.variables().map(|v| v.spec.name.as_str());
                    let candidates = Registry::candidates(all, scope);
                    let hint = did_you_mean(name, candidates)
                        .unwrap_or_else(|| "declare it with a `variable` definition".to_owned());
                    let message = format!("variable `{qualified}` is not declared");
                    let diagnostic = if policy == Undeclared::Deny {
                        Diagnostic::error(codes::UNDECLARED_VARIABLE, span, message)
                    } else {
                        Diagnostic::warning(codes::UNDECLARED_VARIABLE, span, message)
                    };
                    self.diagnostics.push(diagnostic.with_suggestion(hint));
                }
                (qualified, Type::Any, false)
            }
        };
        if self.context.reads() == Reads::Belief && !safe && !object.own {
            let message = format!(
                "variable `{qualified}` is not safe for belief: a character may read it only \
                 of itself"
            );
            self.error(codes::NOT_BELIEF_SAFE, span, message);
        }
        let ty = if object.ty.may_be_null() {
            ty.nullable()
        } else {
            ty
        };
        Node::new(
            NodeKind::Var {
                object,
                name: Arc::from(qualified),
            },
            ty,
            span,
        )
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Box<Node> {
        match &callee.kind {
            ExprKind::Name(name) => self.function(name, callee.span, args, span),
            ExprKind::Member { object, name } if is_var(name) => match args {
                [
                    Expr {
                        kind: ExprKind::Str(text),
                        ..
                    },
                ] => self.variable(object, text, span),
                _ => {
                    self.expr(object);
                    let message = "`var(…)` takes one text: the variable's name".to_owned();
                    let hint = Some("write `.var('mod:name')`".to_owned());
                    self.error_with(codes::ARGUMENT_COUNT, span, message, hint);
                    Self::broken(span)
                }
            },
            ExprKind::Member { object, name } => {
                let object = self.expr(object);
                if let Type::Subject(kind) = object.ty.non_null() {
                    let kind = *kind;
                    return self.property(object, kind, name, Some(args), span);
                }
                // Of something whose kind is not known here, as `view` in
                // a load without the simulation's registry, a property
                // with arguments is read as `.name` is, as an entry of a
                // map, and then by each argument in turn, and checked
                // when it runs.
                if matches!(object.ty.non_null(), Type::Any) {
                    let mut node = Self::entry(object, &name.to_string(), Type::Any, span);
                    for arg in args {
                        let index = self.expr(arg);
                        node = Node::new(
                            NodeKind::Index {
                                object: node,
                                index,
                            },
                            Type::Any,
                            span,
                        );
                    }
                    return node;
                }
                let message = format!(
                    "`.{name}(…)`: {} has no properties to call",
                    self.name_of(&object.ty)
                );
                self.error(codes::NOT_CALLABLE, callee.span, message);
                for arg in args {
                    self.expr(arg);
                }
                Self::broken(span)
            }
            _ => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
                let message = "only a function or a property can be called".to_owned();
                self.error(codes::NOT_CALLABLE, callee.span, message);
                Self::broken(span)
            }
        }
    }

    fn function(&mut self, name: &Name, name_span: Span, args: &[Expr], span: Span) -> Box<Node> {
        let registry = self.registry;
        let scope = self.context.scope();
        let written = name.to_string();
        let found = registry
            .resolve_function(&written, scope)
            .and_then(|resolved| Some((registry.function(resolved.found)?, resolved)));
        let Some((function, resolved)) = found else {
            if !name.is_qualified() && self.context.subject_slot(&name.name).is_some() {
                let message = format!("`{written}` is a subject, not a function");
                self.error(codes::NOT_CALLABLE, name_span, message);
            } else {
                let all = registry.functions().map(|f| f.spec.name.as_str());
                let hint = did_you_mean(&written, Registry::candidates(all, scope));
                let message = format!("unknown function `{written}`");
                self.error_with(codes::UNKNOWN_FUNCTION, name_span, message, hint);
            }
            for arg in args {
                self.expr(arg);
            }
            return Self::broken(span);
        };
        if let Some(other) = &resolved.also_in {
            let message = format!(
                "function `{written}` is `{}`, and `{other}` has one of that name too",
                resolved.name
            );
            self.warning(
                codes::AMBIGUOUS_NAME,
                name_span,
                message,
                "qualify the name",
            );
        }
        let spec = &function.spec;
        // In a mind context, a function that reads where things are is
        // its belief form, read through the context's own character.
        if self.context.reads() == Reads::Belief
            && let Some(form) = spec.belief_form.clone()
        {
            let name = spec.name.clone();
            return self.belief_form(&name, &form, name_span, args, span);
        }
        if spec.random && self.context.purity() == Purity::Pure {
            let message = format!(
                "`{}` draws random numbers, which is not allowed where the value must be pure",
                spec.name
            );
            self.error(codes::RANDOM_IN_PURE, name_span, message);
        }
        if !spec.belief_safe && self.context.reads() == Reads::Belief {
            let message = format!(
                "`{}` is not safe for belief: it reads what a character may not know",
                spec.name
            );
            self.error(codes::NOT_BELIEF_SAFE, name_span, message);
        }
        let what = format!("function `{}`", spec.name);
        let signature = registry.signature(function);
        let args = self.arguments(&what, &signature, &spec.params, spec.variadic, args, span);
        let Some(builtin) = function.builtin else {
            return Node::new(
                NodeKind::Call {
                    id: function.id,
                    args,
                },
                spec.returns.clone(),
                span,
            );
        };
        let types: Vec<Type> = args.iter().map(|arg| arg.ty.clone()).collect();
        if let Some(problem) = builtins::check_args(builtin, &types) {
            let found: Vec<String> = types.iter().map(|ty| self.name_of(ty)).collect();
            let message = format!("{problem}, found {}", found.join(" and "));
            self.error(codes::ARGUMENT_TYPE, span, message);
        }
        let ty = builtins::result_type(builtin, &types, &spec.returns);
        Node::new(NodeKind::Builtin { f: builtin, args }, ty, span)
    }

    /// A call in a mind context of a function that has a belief form
    /// (05 §7.3): the form, called with the context's own character
    /// before the arguments written. A context with no own character
    /// cannot read where anything is, and the call is refused.
    fn belief_form(
        &mut self,
        name: &str,
        form: &str,
        name_span: Span,
        args: &[Expr],
        span: Span,
    ) -> Box<Node> {
        let registry = self.registry;
        let own = self.own_node(span);
        let found = registry
            .resolve_function(form, self.context.scope())
            .and_then(|resolved| registry.function(resolved.found));
        let (Some(own), Some(function)) = (own, found) else {
            let message = format!(
                "`{name}` reads where things are, which a character knows only as it believes \
                 them, and nobody here is the character whose belief it would read"
            );
            self.error(codes::NOT_BELIEF_SAFE, name_span, message);
            for arg in args {
                self.expr(arg);
            }
            return Self::broken(span);
        };
        let spec = &function.spec;
        let params = spec.params.get(1..).unwrap_or(&[]);
        let what = format!("function `{name}`");
        let signature = registry.signature(function);
        let mut all = vec![*own];
        all.extend(self.arguments(&what, &signature, params, spec.variadic, args, span));
        Node::new(
            NodeKind::Call {
                id: function.id,
                args: all,
            },
            spec.returns.clone(),
            span,
        )
    }

    /// The context's own character as a node: its first own subject,
    /// else its first own local.
    fn own_node(&self, span: Span) -> Option<Box<Node>> {
        if let Some((slot, binding)) = self
            .context
            .subjects()
            .iter()
            .enumerate()
            .find(|(_, binding)| binding.own)
        {
            let mut node = Node::new(
                NodeKind::Subject(u16::try_from(slot).ok()?),
                binding.ty.clone(),
                span,
            );
            node.own = true;
            return Some(node);
        }
        let (slot, binding) = self
            .context
            .locals()
            .iter()
            .enumerate()
            .find(|(_, binding)| binding.own)?;
        let mut node = Node::new(
            NodeKind::Local(u16::try_from(slot).ok()?),
            binding.ty.clone(),
            span,
        );
        node.own = true;
        Some(node)
    }

    /// Checks the arguments of a call against its parameters.
    fn arguments(
        &mut self,
        what: &str,
        signature: &str,
        params: &[ParamSpec],
        variadic: bool,
        args: &[Expr],
        span: Span,
    ) -> Vec<Node> {
        let least = params.iter().filter(|param| !param.optional).count();
        let most = if variadic { usize::MAX } else { params.len() };
        if args.len() < least || args.len() > most {
            let wanted = match (least == most, variadic) {
                (true, _) => format!("{least}"),
                (false, true) => format!("at least {least}"),
                (false, false) => format!("{least} to {most}"),
            };
            let message = format!(
                "{what} takes {wanted} argument{}, and {} {} given",
                if wanted == "1" { "" } else { "s" },
                args.len(),
                if args.len() == 1 { "is" } else { "are" },
            );
            self.error_with(
                codes::ARGUMENT_COUNT,
                span,
                message,
                Some(format!("its signature is `{signature}`")),
            );
        }
        let mut checked = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let node = self.expr(arg);
            let param = params
                .get(index)
                .or(if variadic { params.last() } else { None });
            let Some(param) = param else {
                checked.push(*node);
                continue;
            };
            let node = self.coerce(node, &param.ty);
            if !param.ty.accepts(&node.ty) {
                let message = format!(
                    "argument `{}` of {what} must be {}, and this is {}",
                    param.name,
                    self.name_of(&param.ty),
                    self.name_of(&node.ty)
                );
                self.error(codes::ARGUMENT_TYPE, node.span, message);
            } else if node.ty.may_be_null() && !param.ty.may_be_null() && param.ty != Type::Any {
                let message = format!(
                    "argument `{}` of {what} may be null, which makes the result null",
                    param.name
                );
                self.warning(codes::NULL_IN_ARITHMETIC, node.span, message, USE_COALESCE);
            }
            checked.push(*node);
        }
        checked
    }

    fn index(&mut self, object: &Expr, index: &Expr, span: Span) -> Box<Node> {
        let object = self.expr(object);
        let index = self.expr(index);
        let key = index.ty.non_null().clone();
        let ty = match object.ty.non_null() {
            Type::List(element) => {
                if !Type::Int.accepts(&key) {
                    let message = format!(
                        "a list is indexed by an int, and this is {}",
                        self.name_of(&key)
                    );
                    self.error(codes::OPERAND_TYPE, index.span, message);
                }
                (**element).clone().nullable()
            }
            Type::Map(value) => {
                if !matches!(key, Type::Str | Type::Id(_) | Type::Any | Type::Null) {
                    let message = format!(
                        "a map's key is a text or an id, and this is {}",
                        self.name_of(&key)
                    );
                    self.error(codes::OPERAND_TYPE, index.span, message);
                }
                (**value).clone().nullable()
            }
            Type::Any => Type::Any,
            Type::Null => Type::Null,
            other => {
                let message = format!(
                    "only a list or a map can be indexed, and this is {}",
                    self.name_of(other)
                );
                self.error(codes::NOT_INDEXABLE, object.span, message);
                return Self::broken(span);
            }
        };
        // An element of the character's own list is the character's own.
        let own = object.own;
        let mut node = Node::new(NodeKind::Index { object, index }, ty, span);
        node.own = own;
        node
    }

    // ---- lists and maps ------------------------------------------------------

    fn list(&mut self, items: &[Expr], span: Span) -> Box<Node> {
        let mut nodes = Vec::with_capacity(items.len());
        let mut element: Option<Type> = None;
        for item in items {
            let node = self.expr(item);
            element = Some(match element {
                None => node.ty.clone(),
                Some(ty) => ty.join(&node.ty),
            });
            nodes.push(*node);
        }
        let ty = Type::list(element.unwrap_or(Type::Any));
        // A list made of the character's own things is the character's
        // own, as an element of an own list is (05 §7.3). An empty one
        // holds nothing to read.
        let own = nodes.iter().all(|node| node.own);
        let mut node = Node::new(NodeKind::List(nodes), ty, span);
        node.own = own;
        node
    }

    fn map(&mut self, entries: &[(Expr, Expr)], span: Span) -> Box<Node> {
        let mut nodes = Vec::with_capacity(entries.len());
        let mut value_type: Option<Type> = None;
        for (key, value) in entries {
            let key = self.expr(key);
            if !matches!(key.ty.non_null(), Type::Str | Type::Id(_) | Type::Any) {
                let message = format!(
                    "a map's key is a text or an id, and this is {}",
                    self.name_of(&key.ty)
                );
                self.error(codes::OPERAND_TYPE, key.span, message);
            }
            let value = self.expr(value);
            value_type = Some(match value_type {
                None => value.ty.clone(),
                Some(ty) => ty.join(&value.ty),
            });
            nodes.push((*key, *value));
        }
        let ty = Type::map(value_type.unwrap_or(Type::Any));
        Node::new(NodeKind::Map(nodes), ty, span)
    }

    // ---- operators -------------------------------------------------------------

    /// Reports an operand of `and`, `or`, `not` or `if` that cannot be a
    /// bool. Null is allowed: it counts as false.
    fn want_bool(&mut self, node: &Node, what: &str) {
        if !matches!(node.ty.non_null(), Type::Bool | Type::Any | Type::Null) {
            let message = format!(
                "{what} needs a bool, and this is {}",
                self.name_of(&node.ty)
            );
            self.error(codes::OPERAND_TYPE, node.span, message);
        }
    }

    /// Warns of an operand of arithmetic that may be null.
    fn warn_null(&mut self, node: &Node, what: &str) {
        if node.ty.may_be_null() {
            let message = if node.ty == Type::Null {
                format!("this is null, which makes the result of {what} null")
            } else {
                format!("this may be null, which makes the result of {what} null")
            };
            self.warning(codes::NULL_IN_ARITHMETIC, node.span, message, USE_COALESCE);
        }
    }

    fn unary(&mut self, op: UnaryOp, operand: &Expr, span: Span) -> Box<Node> {
        let operand = self.expr(operand);
        match op {
            UnaryOp::Not => {
                self.want_bool(&operand, "`not`");
                Node::new(NodeKind::Not(operand), Type::Bool, span)
            }
            UnaryOp::Neg => {
                self.warn_null(&operand, "`-`");
                let ty = match operand.ty.non_null() {
                    Type::Int => Type::Int,
                    Type::Real => Type::Real,
                    Type::Number | Type::Any => Type::Number,
                    Type::Null => Type::Null,
                    other => {
                        let message =
                            format!("`-` needs a number, and this is {}", self.name_of(other));
                        self.error(codes::OPERAND_TYPE, operand.span, message);
                        Type::Any
                    }
                };
                Node::new(NodeKind::Neg(operand), ty, span)
            }
        }
    }

    fn binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr, span: Span) -> Box<Node> {
        let lhs = self.expr(lhs);
        let rhs = self.expr(rhs);
        match op {
            BinaryOp::And | BinaryOp::Or => {
                let what = if op == BinaryOp::And { "`and`" } else { "`or`" };
                self.want_bool(&lhs, what);
                self.want_bool(&rhs, what);
                let kind = if op == BinaryOp::And {
                    NodeKind::And(lhs, rhs)
                } else {
                    NodeKind::Or(lhs, rhs)
                };
                Node::new(kind, Type::Bool, span)
            }
            BinaryOp::Coalesce => {
                let rhs = self.coerce(rhs, &lhs.ty);
                let ty = Self::coalesce_type(&lhs.ty, &rhs.ty);
                Node::new(NodeKind::Coalesce(lhs, rhs), ty, span)
            }
            BinaryOp::Add => self.arith(ArithOp::Add, lhs, rhs, span),
            BinaryOp::Sub => self.arith(ArithOp::Sub, lhs, rhs, span),
            BinaryOp::Mul => self.arith(ArithOp::Mul, lhs, rhs, span),
            BinaryOp::Div => self.arith(ArithOp::Div, lhs, rhs, span),
            BinaryOp::Rem => self.arith(ArithOp::Rem, lhs, rhs, span),
            BinaryOp::Pow => self.arith(ArithOp::Pow, lhs, rhs, span),
            BinaryOp::Eq => self.equality(CmpOp::Eq, lhs, rhs, span),
            BinaryOp::Ne => self.equality(CmpOp::Ne, lhs, rhs, span),
            BinaryOp::Lt => self.ordering(CmpOp::Lt, lhs, rhs, span),
            BinaryOp::Le => self.ordering(CmpOp::Le, lhs, rhs, span),
            BinaryOp::Gt => self.ordering(CmpOp::Gt, lhs, rhs, span),
            BinaryOp::Ge => self.ordering(CmpOp::Ge, lhs, rhs, span),
            BinaryOp::In => self.membership(lhs, rhs, span),
        }
    }

    fn coalesce_type(lhs: &Type, rhs: &Type) -> Type {
        match lhs {
            Type::Null => rhs.clone(),
            Type::Any => Type::Any,
            // The left side's null is what `??` replaces, so only the
            // right side's can remain.
            other => {
                let joined = other.non_null().join(rhs.non_null());
                if rhs.may_be_null() {
                    joined.nullable()
                } else {
                    joined
                }
            }
        }
    }

    fn arith(&mut self, op: ArithOp, lhs: Box<Node>, rhs: Box<Node>, span: Span) -> Box<Node> {
        let what = format!("`{}`", op.symbol());
        self.warn_null(&lhs, &what);
        self.warn_null(&rhs, &what);
        let a = lhs.ty.non_null().clone();
        let b = rhs.ty.non_null().clone();
        let number = |ty: &Type| ty.is_numeric() || matches!(ty, Type::Any | Type::Null);
        let either_null = a == Type::Null || b == Type::Null;
        let ty = match op {
            _ if either_null && (number(&a) && number(&b) || op == ArithOp::Add) => Type::Null,
            ArithOp::Add => match (&a, &b) {
                (Type::Any, Type::Any) => Some(Type::Any),
                (a, b) if number(a) && number(b) => Some(Type::numeric([a, b])),
                (Type::Str, Type::Str | Type::Any) | (Type::Any, Type::Str) => Some(Type::Str),
                (Type::List(x), Type::List(y)) => Some(Type::list(x.join(y))),
                (Type::List(_), Type::Any) => Some(a.clone()),
                (Type::Any, Type::List(_)) => Some(b.clone()),
                _ => None,
            }
            .unwrap_or_else(|| {
                let hint = (a == Type::Str || b == Type::Str)
                    .then(|| "use `str(…)` to join a text with something else".to_owned());
                let message = format!(
                    "`+` adds numbers, joins texts or joins lists, and these are {} and {}",
                    self.name_of(&a),
                    self.name_of(&b)
                );
                self.error_with(codes::OPERAND_TYPE, span, message, hint);
                Type::Any
            }),
            ArithOp::Rem => {
                let whole = |ty: &Type| matches!(ty, Type::Int | Type::Number | Type::Any);
                if !(whole(&a) && whole(&b)) {
                    let message = format!(
                        "`%` is an integer operation, and these are {} and {}",
                        self.name_of(&a),
                        self.name_of(&b)
                    );
                    let hint = Some("round with `floor`, `ceil` or `round` first".to_owned());
                    self.error_with(codes::OPERAND_TYPE, span, message, hint);
                }
                Type::Int
            }
            ArithOp::Sub | ArithOp::Mul | ArithOp::Div | ArithOp::Pow => {
                if !(number(&a) && number(&b)) {
                    let message = format!(
                        "{what} needs numbers, and these are {} and {}",
                        self.name_of(&a),
                        self.name_of(&b)
                    );
                    self.error(codes::OPERAND_TYPE, span, message);
                    Type::Any
                } else if matches!(op, ArithOp::Div | ArithOp::Pow) {
                    Type::Real
                } else {
                    Type::numeric([&a, &b])
                }
            }
        };
        Node::new(NodeKind::Arith { op, lhs, rhs }, ty, span)
    }

    /// Whether values of the two types can ever be equal.
    fn comparable(a: &Type, b: &Type) -> bool {
        match (a.non_null(), b.non_null()) {
            (Type::Any | Type::Null, _) | (_, Type::Any | Type::Null) => true,
            (a, b) if a.is_numeric() && b.is_numeric() => true,
            (Type::Id(_), Type::Str) | (Type::Str, Type::Id(_)) => true,
            (Type::List(a), Type::List(b)) | (Type::Map(a), Type::Map(b)) => Self::comparable(a, b),
            (a, b) => a == b,
        }
    }

    fn equality(&mut self, op: CmpOp, lhs: Box<Node>, rhs: Box<Node>, span: Span) -> Box<Node> {
        let negated = op == CmpOp::Ne;
        if lhs.ty == Type::Null {
            let kind = NodeKind::IsNull {
                operand: rhs,
                negated,
            };
            return Node::new(kind, Type::Bool, span);
        }
        if rhs.ty == Type::Null {
            let kind = NodeKind::IsNull {
                operand: lhs,
                negated,
            };
            return Node::new(kind, Type::Bool, span);
        }
        let rhs = self.coerce(rhs, &lhs.ty);
        let lhs = self.coerce(lhs, &rhs.ty);
        if !Self::comparable(&lhs.ty, &rhs.ty) {
            let message = format!(
                "{} and {} can never be equal",
                self.name_of(lhs.ty.non_null()),
                self.name_of(rhs.ty.non_null())
            );
            self.error(codes::UNRELATED_COMPARISON, span, message);
        }
        Node::new(NodeKind::Compare { op, lhs, rhs }, Type::Bool, span)
    }

    fn ordering(&mut self, op: CmpOp, lhs: Box<Node>, rhs: Box<Node>, span: Span) -> Box<Node> {
        let a = lhs.ty.non_null();
        let b = rhs.ty.non_null();
        let open = |ty: &Type| matches!(ty, Type::Any | Type::Null);
        let numbers = (a.is_numeric() || open(a)) && (b.is_numeric() || open(b));
        let texts = (*a == Type::Str || open(a)) && (*b == Type::Str || open(b));
        if !numbers && !texts {
            let message = format!(
                "`{}` orders numbers or texts, and these are {} and {}",
                op.symbol(),
                self.name_of(a),
                self.name_of(b)
            );
            self.error(codes::OPERAND_TYPE, span, message);
        }
        Node::new(NodeKind::Compare { op, lhs, rhs }, Type::Bool, span)
    }

    fn membership(&mut self, item: Box<Node>, collection: Box<Node>, span: Span) -> Box<Node> {
        let (item, collection) = match collection.ty.non_null().clone() {
            Type::List(element) => {
                let item = self.coerce(item, &element);
                let collection = self.coerce(collection, &Type::list(item.ty.clone()));
                let element = collection.ty.element().cloned().unwrap_or(Type::Any);
                if !Self::comparable(&item.ty, &element) {
                    let message = format!(
                        "{} is never in a list of {}",
                        self.name_of(item.ty.non_null()),
                        self.name_of(&element)
                    );
                    self.error(codes::UNRELATED_COMPARISON, span, message);
                }
                (item, collection)
            }
            Type::Map(_) => {
                if !matches!(
                    item.ty.non_null(),
                    Type::Str | Type::Id(_) | Type::Any | Type::Null
                ) {
                    let message = format!(
                        "a map's key is a text or an id, and this is {}",
                        self.name_of(&item.ty)
                    );
                    self.error(codes::OPERAND_TYPE, item.span, message);
                }
                (item, collection)
            }
            Type::Any | Type::Null => (item, collection),
            other => {
                let message = format!(
                    "`in` needs a list or a map on its right, and this is {}",
                    self.name_of(&other)
                );
                let hint = (other == Type::Str)
                    .then(|| "use `contains(text, part)` for a text in a text".to_owned());
                self.error_with(codes::OPERAND_TYPE, collection.span, message, hint);
                (item, collection)
            }
        };
        Node::new(NodeKind::In { item, collection }, Type::Bool, span)
    }

    fn if_else(&mut self, cond: &Expr, then: &Expr, otherwise: &Expr, span: Span) -> Box<Node> {
        let cond = self.expr(cond);
        self.want_bool(&cond, "`if`");
        let then = self.expr(then);
        let otherwise = self.expr(otherwise);
        let otherwise = self.coerce(otherwise, &then.ty);
        let then = self.coerce(then, &otherwise.ty);
        let ty = then.ty.join(&otherwise.ty);
        // Whichever branch is taken, it is the character's own if both
        // are.
        let own = then.own && otherwise.own;
        let mut node = Node::new(
            NodeKind::If {
                cond,
                then,
                otherwise,
            },
            ty,
            span,
        );
        node.own = own;
        node
    }

    // ---- ids ----------------------------------------------------------------------

    /// Turns a constant text into an id where an id is wanted: directly,
    /// as an element of a list literal, or as a branch of `if` or `??`.
    fn coerce(&mut self, mut node: Box<Node>, expected: &Type) -> Box<Node> {
        let wanted = expected.non_null();
        match (&mut node.kind, wanted) {
            (NodeKind::Const(Value::Str(text)), Type::Id(def_type)) => {
                let text = Arc::clone(text);
                match self.resolve_id(&text, *def_type, node.span) {
                    Some(value) => {
                        node.kind = NodeKind::Const(value);
                        node.ty = Type::Id(*def_type);
                    }
                    // Reported. `any` keeps it from being reported again.
                    None => node.ty = Type::Any,
                }
            }
            (NodeKind::List(items), Type::List(element))
                if matches!(element.non_null(), Type::Id(_)) =>
            {
                let mut joined: Option<Type> = None;
                for item in items.iter_mut() {
                    let taken = core::mem::replace(item, *Self::broken(item.span));
                    *item = *self.coerce(Box::new(taken), element);
                    joined = Some(match joined {
                        None => item.ty.clone(),
                        Some(ty) => ty.join(&item.ty),
                    });
                }
                node.ty = Type::list(joined.unwrap_or(Type::Any));
            }
            (
                NodeKind::If {
                    then, otherwise, ..
                },
                Type::Id(_) | Type::List(_),
            ) => {
                let span = then.span;
                let taken = core::mem::replace(then, Self::broken(span));
                *then = self.coerce(taken, expected);
                let taken = core::mem::replace(otherwise, Self::broken(span));
                *otherwise = self.coerce(taken, expected);
                node.ty = then.ty.join(&otherwise.ty);
            }
            (NodeKind::Coalesce(lhs, rhs), Type::Id(_) | Type::List(_)) => {
                let span = lhs.span;
                let taken = core::mem::replace(lhs, Self::broken(span));
                *lhs = self.coerce(taken, expected);
                let taken = core::mem::replace(rhs, Self::broken(span));
                *rhs = self.coerce(taken, expected);
                node.ty = Self::coalesce_type(&lhs.ty, &rhs.ty);
            }
            _ => {}
        }
        node
    }

    fn resolve_id(&mut self, text: &str, def_type: DefTypeId, span: Span) -> Option<Value> {
        let registry = self.registry;
        let scope = self.context.scope();
        let type_name = registry
            .def_type(def_type)
            .map_or("?", |def_type| def_type.name.as_str());
        match registry.resolve_def(def_type, text, scope) {
            Some(resolved) => {
                if let Some(other) = &resolved.also_in {
                    let message = format!(
                        "`{text}` is the {type_name} `{}`, and `{other}` has one of that name too",
                        resolved.name
                    );
                    self.warning(codes::AMBIGUOUS_NAME, span, message, "qualify the id");
                }
                Some(Value::Id(resolved.found))
            }
            None => {
                let ids = registry
                    .def_type(def_type)
                    .into_iter()
                    .flat_map(|def_type| def_type.ids())
                    .map(|(name, _)| name);
                let hint = did_you_mean(text, Registry::candidates(ids, scope))
                    .map(|hint| hint.replace('`', "'"));
                let message = format!("there is no {type_name} with the id '{text}'");
                self.error_with(codes::UNKNOWN_ID, span, message, hint);
                None
            }
        }
    }
}
