//! The CLASH2D simulation: the kernel that owns `advance`, entities and
//! their components, bodies, actions and reaction windows, combat,
//! abilities, perception, AI, the rule (EOC) runtime and the cold
//! simulation. It is a pure state machine that takes commands, advances
//! time and emits events, and it never waits on input or the wall clock.
//! Specified in `docs/03-time-and-actions.md` and documents 05 to 09, 13,
//! 15 and 16.
//!
//! This is a stub. It is filled in by K1 onward of `docs/plans/sim-kernel.md` and the plans that follow it.

pub mod component;
pub mod console;
pub mod stats;

/// Registers the definition types of this crate with the loader, with
/// the checks and registry steps that go with them. The application and
/// the tools call this beside each other crate's function of the same
/// name.
pub fn register_defs(types: &mut clash_data::DefTypes) {
    stats::register_defs(types);
}
