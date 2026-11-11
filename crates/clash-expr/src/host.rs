//! [`Host`]: what an expression is evaluated against.
//!
//! This crate knows what `self.mass * 2` means and nothing about what
//! `self` is or how heavy. Everything outside the expression itself is
//! asked of the host through this trait: the values of bound subjects,
//! locals and event fields, properties of subjects, variables, the
//! functions the host registered, and random numbers.
//!
//! The host is the simulation. A test host is in
//! [`testing`](crate::testing).

use clash_core::Rng;

use crate::value::{DefRef, FunctionId, Handle, PropertyId, Value};
use crate::warn::RuntimeWarning;

/// What an expression is evaluated against.
///
/// This crate knows what `self.mass * 2` means and nothing about what
/// `self` is or how heavy. Everything outside the expression itself is
/// asked of the host: the values of bound subjects, locals and event
/// fields, properties of subjects, variables, the functions the host
/// registered, and random numbers. The host is the simulation; a host
/// for tests is in [`testing`](crate::testing).
///
/// Slots are the positions at which subjects, locals and event fields
/// were added to the [`Context`](crate::Context) the expression was
/// compiled in. Ids are the ones the [`Registry`](crate::Registry) gave
/// out when the host registered its properties and functions.
///
/// None of these may panic on content. Whatever cannot be answered (a
/// stale handle, an unloaded cell, a slot the host does not have) is
/// answered with [`Value::Null`], which the language is built to carry
/// (05 §4.4).
///
/// Arguments have been checked against the registered signature where
/// their types were known at load, and not otherwise: an argument typed
/// `any` arrives as whatever it turned out to be, and any argument may
/// be null. An int is accepted where a real is declared and arrives as
/// an int, so read a number with [`Value::as_number`].
///
/// The trait has no way to write. Expressions only read (05 invariant
/// E1); what an effect assigns to is described by a
/// [`Place`](crate::Place), and the write is the rule runtime's to make.
pub trait Host {
    /// The value of a bound subject.
    fn subject(&mut self, slot: u16) -> Value;

    /// The value of a local.
    fn local(&mut self, slot: u16) -> Value;

    /// The value of a field of the triggering event.
    fn event_field(&mut self, slot: u16) -> Value;

    /// A property of a subject. `args` are the property's arguments, if
    /// it takes any. The machine never asks for a property of null: that
    /// is null without asking.
    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value;

    /// A variable of a subject, by qualified name. A declared variable
    /// that was never set reads as its default; an undeclared one as
    /// null.
    fn variable(&mut self, subject: Handle, name: &str) -> Value;

    /// The result of a function the host registered. Arguments left out
    /// of an optional tail are simply absent from `args`.
    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value;

    /// The stream random functions draw from, or `None` if this
    /// evaluation has none. A random function then gives null, with a
    /// warning.
    fn rng(&mut self) -> Option<&mut Rng>;

    /// Receives a run-time warning. The host knows which rule is running
    /// and decides, usually through a
    /// [`WarningLimiter`](crate::WarningLimiter), whether to report it.
    fn warn(&mut self, warning: RuntimeWarning);

    /// The qualified id of a definition, for turning an id into text and
    /// for comparing an id with a text that was not known at load.
    /// Usually [`Registry::def_name`](crate::Registry::def_name).
    fn def_name(&self, id: DefRef) -> Option<&str> {
        let _ = id;
        None
    }
}
