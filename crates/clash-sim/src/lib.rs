//! The CLASH2D simulation: the kernel that owns `advance`, entities and
//! their components, bodies, actions and reaction windows, combat,
//! abilities, perception, AI, the rule (EOC) runtime and the cold
//! simulation. It is a pure state machine that takes commands, advances
//! time and emits events, and it never waits on input or the wall clock.
//! Specified in `docs/03-time-and-actions.md` and documents 05 to 09, 13,
//! 15 and 16.
//!
//! This is a stub. It is filled in by K1 onward of `docs/plans/sim-kernel.md` and the plans that follow it.

pub mod action;
pub mod component;
pub mod console;

/// Registers every definition type the simulation owns with the loader.
/// The application and the tools call this once (04 §9).
pub fn register_defs(types: &mut clash_data::DefTypes) {
    action::register_defs(types);
}
