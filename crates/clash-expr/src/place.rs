//! Places: what the `set` and `add` effects may assign to (05 §10.3).
//!
//! Expressions only read. But an effect that writes names its target the
//! way an expression would read it: `$x`, `event.energy`,
//! `self.var.kills`, `self.var.tally['ash']`, `self.health`. A [`Place`]
//! is such a target, resolved and checked: what kind of thing it is, the
//! compiled expression for the subject it lives on, and the compiled
//! keys of the entries to step into.
//!
//! This crate does not write anything. The rule runtime evaluates the
//! subject and the keys with a [`Machine`](crate::Machine) and performs
//! the write through the engine's own mutation paths (01 R3).

use std::sync::Arc;

use crate::check::{Checker, Outcome};
use crate::code::{Compiled, compile_node};
use crate::context::Context;
use crate::diag::{Diagnostic, codes};
use crate::hir::NodeKind;
use crate::parser::parse_expr;
use crate::registry::Registry;
use crate::types::Type;
use crate::value::PropertyId;

/// What a place is, before any keys.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlaceRoot {
    /// A local, by slot: `$x`.
    Local(u16),
    /// A field of the triggering event, by slot: `event.energy`. Whether
    /// a handler may change it is the event's to say
    /// ([`EventSpec::mutable`](crate::EventSpec)).
    EventField(u16),
    /// A variable of a subject, by qualified name: `self.var.kills`.
    Variable(Arc<str>),
    /// A settable property of a subject: `self.health`.
    Property(PropertyId),
}

/// Something the `set` and `add` effects may assign to (05 §10.3).
///
/// Expressions only read. But an effect that writes names its target the
/// way an expression would read it: `$x`, `event.energy`,
/// `self.var.kills`, `self.var.tally['ash']`, `self.health`. A place is
/// such a target, resolved and checked: what kind of thing it is, the
/// compiled expression for the subject it lives on, and the compiled
/// keys of the entries to step into.
///
/// This crate does not write anything. The rule runtime evaluates the
/// subject and the keys with a [`Machine`](crate::Machine) and performs
/// the write through the engine's own mutation paths (01 R3).
///
/// ```
/// use clash_expr::testing::TestHost;
/// use clash_expr::{Machine, PlaceRoot, Type, Value, compile_place};
///
/// let mut host = TestHost::new();
/// let context = host.context();
/// let place = compile_place("target.var.kills", host.registry(), &context)
///     .into_result()
///     .unwrap();
/// assert_eq!(place.root(), &PlaceRoot::Variable("core:kills".into()));
/// assert_eq!(place.ty(), &Type::Int);
/// // Whose variable it is, is evaluated when the effect runs.
/// let whose = Machine::new().eval(place.subject().unwrap(), &mut host);
/// assert_eq!(whose, Value::Subject(host.entity(2)));
/// ```
#[derive(Clone, Debug)]
pub struct Place {
    root: PlaceRoot,
    subject: Option<Compiled>,
    keys: Vec<Compiled>,
    ty: Type,
}

impl Place {
    /// What kind of thing is assigned to.
    pub const fn root(&self) -> &PlaceRoot {
        &self.root
    }

    /// The expression for the subject a variable or a property lives on.
    /// `None` for a local or an event field.
    pub const fn subject(&self) -> Option<&Compiled> {
        self.subject.as_ref()
    }

    /// The keys to step into, outermost first: for
    /// `self.var.tally['ash']`, the one expression `'ash'`. Empty when
    /// the whole value is assigned.
    pub fn keys(&self) -> &[Compiled] {
        &self.keys
    }

    /// The type a value assigned here should have.
    pub const fn ty(&self) -> &Type {
        &self.ty
    }
}

/// Parses and checks the target of an assignment.
///
/// A place is a local, an event field, a variable or a settable
/// property, optionally followed by keys. Anything else is refused.
pub fn compile_place(source: &str, registry: &Registry, context: &Context) -> Outcome<Place> {
    let tree = match parse_expr(source) {
        Ok(tree) => tree,
        Err(error) => return Outcome::failed(vec![error]),
    };
    let mut checker = Checker::new(registry, context);
    let node = checker.expr(&tree);
    let mut diagnostics = checker.into_diagnostics();
    if diagnostics.iter().any(Diagnostic::is_error) {
        return Outcome::failed(diagnostics);
    }
    let origin = context.origin_id();
    let span = node.span;
    let ty = node.ty.non_null().clone();

    // Peel the keys off, innermost last.
    let mut keys = Vec::new();
    let mut current = *node;
    loop {
        match current.kind {
            NodeKind::Index { object, index } => {
                keys.push(index);
                current = *object;
            }
            kind => {
                current.kind = kind;
                break;
            }
        }
    }
    keys.reverse();

    let refuse = |message: String| {
        Diagnostic::error(codes::NOT_ASSIGNABLE, span, message).with_suggestion(
            "a place is `$local`, `event.field`, `subject.var.name` or a settable property, \
             optionally followed by `[key]`",
        )
    };
    let (root, subject) = match current.kind {
        NodeKind::Local(slot) => (PlaceRoot::Local(slot), None),
        NodeKind::EventField(slot) => (PlaceRoot::EventField(slot), None),
        NodeKind::Var { object, name } => (PlaceRoot::Variable(name), Some(object)),
        NodeKind::Property { object, id, args } => {
            let settable = registry
                .property(id)
                .is_some_and(|property| property.spec.settable);
            if !settable || !args.is_empty() {
                let signature = registry
                    .property(id)
                    .map(|property| registry.property_signature(property))
                    .unwrap_or_default();
                diagnostics.push(refuse(format!("`{signature}` cannot be set")));
                return Outcome::failed(diagnostics);
            }
            (PlaceRoot::Property(id), Some(object))
        }
        _ => {
            diagnostics.push(refuse("this cannot be assigned to".to_owned()));
            return Outcome::failed(diagnostics);
        }
    };

    let subject = subject.and_then(|node| compile_node(node, origin, &mut diagnostics));
    let keys: Vec<Compiled> = keys
        .into_iter()
        .filter_map(|node| compile_node(node, origin, &mut diagnostics))
        .collect();
    Outcome::new(
        Place {
            root,
            subject,
            keys,
            ty,
        },
        diagnostics,
    )
}
