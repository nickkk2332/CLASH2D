//! [`Context`]: what an expression may name and how pure it must be
//! (05 §7.3).
//!
//! Each place an expression appears declares a context: the subjects
//! bound there, the locals, the event whose fields can be read, whether
//! random functions are allowed, and whether the expression reads truth
//! or only what a character believes. The compiler resolves every name
//! against it, so an expression that refers to something its place does
//! not offer fails at load and not in play.
//!
//! Subjects and locals are given **slots** in the order they are added.
//! At run time the machine asks the host for "subject 2" or "local 0";
//! the host must lay its values out in the order the context was built.

use crate::registry::Scope;
use crate::types::Type;
use crate::value::EventId;

/// Whether an expression may draw random numbers (05 §7.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Purity {
    /// Reads only. Screen bindings, previews, sorting keys.
    Pure,
    /// Reads and random draws. Conditions and effect arguments.
    #[default]
    Random,
}

/// Whether an expression reads ground truth or a character's belief
/// (05 §7.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Reads {
    /// Anything. World rules, effects, hooks, debug screens.
    #[default]
    Truth,
    /// The mind context: the character's own state, and whatever is
    /// registered as safe for belief.
    Belief,
}

/// What to do about a variable that no definition declares (05 §6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Undeclared {
    /// Accept it silently. For tools that have checked already.
    Allow,
    /// Accept it with a warning: a misspelt name that silently makes a
    /// new variable is the commonest bug in rules.
    #[default]
    Warn,
    /// Refuse it: the mod set `strict_vars`.
    Deny,
}

/// A name bound in a context.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Binding {
    /// The name as written: `self`, `target`, `it`.
    pub name: String,
    /// The type of its value. Usually a subject kind.
    pub ty: Type,
    /// Whether this is the character whose mind the context belongs to.
    /// In a mind context, any property of an own subject may be read.
    pub own: bool,
}

/// Where an expression appears. See the [module](self) documentation.
///
/// ```
/// use clash_expr::{Context, Purity, Reads, Type};
///
/// let context = Context::new()
///     .subject("self", Type::Any)
///     .local("power", Type::Real)
///     .pure()
///     .namespace("ashlands");
/// assert_eq!(context.subject_slot("self"), Some(0));
/// assert_eq!(context.local_slot("power"), Some(0));
/// assert_eq!(context.purity(), Purity::Pure);
/// assert_eq!(context.reads(), Reads::Truth);
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Context {
    subjects: Vec<Binding>,
    locals: Vec<Binding>,
    event: Option<EventId>,
    purity: Purity,
    reads: Reads,
    scope: Scope,
    undeclared: Undeclared,
    origin: u64,
}

impl Context {
    /// A context with nothing bound, in which random functions are
    /// allowed and truth is read.
    pub fn new() -> Context {
        Context::default()
    }

    /// Binds a subject, or any named value, and gives it the next slot.
    #[must_use]
    pub fn subject(mut self, name: impl Into<String>, ty: Type) -> Context {
        self.add_subject(name, ty, false);
        self
    }

    /// Binds the character whose mind this is. In a mind context all of
    /// its properties and variables may be read.
    #[must_use]
    pub fn own_subject(mut self, name: impl Into<String>, ty: Type) -> Context {
        self.add_subject(name, ty, true);
        self
    }

    /// Declares a local, written `$name`, and gives it the next slot.
    #[must_use]
    pub fn local(mut self, name: impl Into<String>, ty: Type) -> Context {
        self.add_local(name, ty);
        self
    }

    /// Makes the fields of an event readable as `event.field`. A field's
    /// slot is its position in the event's registration.
    #[must_use]
    pub fn event(mut self, event: EventId) -> Context {
        self.event = Some(event);
        self
    }

    /// Requires purity: random functions are refused.
    #[must_use]
    pub fn pure(mut self) -> Context {
        self.purity = Purity::Pure;
        self
    }

    /// Makes this a mind context: only belief may be read.
    #[must_use]
    pub fn belief(mut self) -> Context {
        self.reads = Reads::Belief;
        self
    }

    /// Sets the namespace bare names are looked up in first: the mod the
    /// expression belongs to.
    #[must_use]
    pub fn namespace(mut self, namespace: impl Into<String>) -> Context {
        self.scope.namespace = Some(namespace.into());
        self
    }

    /// Sets the namespaces bare names are looked up in next, in order.
    #[must_use]
    pub fn imports<S: Into<String>>(mut self, imports: impl IntoIterator<Item = S>) -> Context {
        self.scope.imports = imports.into_iter().map(Into::into).collect();
        self
    }

    /// Says what to do about undeclared variables.
    #[must_use]
    pub fn undeclared(mut self, policy: Undeclared) -> Context {
        self.undeclared = policy;
        self
    }

    /// Sets the number run-time warnings from expressions compiled here
    /// carry, by which the host knows the rule and the JSON path they
    /// came from.
    #[must_use]
    pub fn origin(mut self, origin: u64) -> Context {
        self.origin = origin;
        self
    }

    /// Binds a subject in place and returns its slot. A name bound twice
    /// refers to the later binding.
    pub fn add_subject(&mut self, name: impl Into<String>, ty: Type, own: bool) -> u16 {
        let slot = slot_for(self.subjects.len());
        self.subjects.push(Binding {
            name: name.into(),
            ty,
            own,
        });
        slot
    }

    /// Declares a local in place and returns its slot. A name declared
    /// twice refers to the later declaration.
    pub fn add_local(&mut self, name: impl Into<String>, ty: Type) -> u16 {
        let slot = slot_for(self.locals.len());
        self.locals.push(Binding {
            name: name.into(),
            ty,
            own: false,
        });
        slot
    }

    /// Declares a local that is the character's own state (05 §7.3) and
    /// returns its slot: in a mind context any property of it may be
    /// read, as of an own subject. For a parameter that is known to be
    /// one of the characters whose mind the context belongs to.
    pub fn add_own_local(&mut self, name: impl Into<String>, ty: Type) -> u16 {
        let slot = slot_for(self.locals.len());
        self.locals.push(Binding {
            name: name.into(),
            ty,
            own: true,
        });
        slot
    }

    /// The bound subjects, in slot order.
    pub fn subjects(&self) -> &[Binding] {
        &self.subjects
    }

    /// The locals, in slot order.
    pub fn locals(&self) -> &[Binding] {
        &self.locals
    }

    /// The slot of a bound subject.
    pub fn subject_slot(&self, name: &str) -> Option<u16> {
        find_slot(&self.subjects, name)
    }

    /// The slot of a local.
    pub fn local_slot(&self, name: &str) -> Option<u16> {
        find_slot(&self.locals, name)
    }

    /// The event whose fields can be read, if any.
    pub const fn event_id(&self) -> Option<EventId> {
        self.event
    }

    /// Whether random functions are allowed.
    pub const fn purity(&self) -> Purity {
        self.purity
    }

    /// Whether truth or belief is read.
    pub const fn reads(&self) -> Reads {
        self.reads
    }

    /// The namespace and imports bare names are looked up in.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// What is done about undeclared variables.
    pub const fn undeclared_policy(&self) -> Undeclared {
        self.undeclared
    }

    /// The number run-time warnings carry.
    pub const fn origin_id(&self) -> u64 {
        self.origin
    }
}

/// The slot for the binding at `index`. More bindings than a slot can
/// number all share the last slot; no context comes near that.
fn slot_for(index: usize) -> u16 {
    u16::try_from(index).unwrap_or(u16::MAX)
}

fn find_slot(bindings: &[Binding], name: &str) -> Option<u16> {
    bindings
        .iter()
        .rposition(|binding| binding.name == name)
        .map(slot_for)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_follow_the_order_of_binding() {
        let mut context = Context::new()
            .subject("self", Type::Any)
            .own_subject("me", Type::Any)
            .local("a", Type::Int)
            .local("b", Type::Real);
        assert_eq!(context.subject_slot("self"), Some(0));
        assert_eq!(context.subject_slot("me"), Some(1));
        assert_eq!(context.subject_slot("nobody"), None);
        assert_eq!(context.local_slot("a"), Some(0));
        assert_eq!(context.local_slot("b"), Some(1));
        assert_eq!(context.local_slot("self"), None);
        assert!(context.subjects()[1].own);
        assert!(!context.subjects()[0].own);
        assert_eq!(context.add_subject("target", Type::Any, false), 2);
        assert_eq!(context.add_local("c", Type::Str), 2);
        assert_eq!(context.locals().len(), 3);
        // A name bound again refers to the later binding.
        assert_eq!(context.add_local("a", Type::Str), 3);
        assert_eq!(context.local_slot("a"), Some(3));
    }

    #[test]
    fn defaults_and_settings() {
        let context = Context::new();
        assert_eq!(context.purity(), Purity::Random);
        assert_eq!(context.reads(), Reads::Truth);
        assert_eq!(context.undeclared_policy(), Undeclared::Warn);
        assert_eq!(context.event_id(), None);
        assert_eq!(context.origin_id(), 0);
        assert_eq!(context.scope(), &Scope::default());

        let context = context
            .pure()
            .belief()
            .undeclared(Undeclared::Deny)
            .event(EventId::new(3))
            .origin(42)
            .namespace("ashlands")
            .imports(["core", "extra"]);
        assert_eq!(context.purity(), Purity::Pure);
        assert_eq!(context.reads(), Reads::Belief);
        assert_eq!(context.undeclared_policy(), Undeclared::Deny);
        assert_eq!(context.event_id(), Some(EventId::new(3)));
        assert_eq!(context.origin_id(), 42);
        let namespaces: Vec<&str> = context.scope().namespaces().collect();
        assert_eq!(namespaces, ["ashlands", "core", "extra"]);
    }
}
