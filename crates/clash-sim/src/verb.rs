//! The verbs the engine offers to rules (05 §10.1, §10.7): one table
//! that every package of the simulation adds to.
//!
//! A verb is what an effect such as `{ "damage": { … } }` calls. Its
//! arguments are declared in a [`VerbSpec`], so that the loader can
//! check a rule that calls it and the console can offer it as a command
//! (14 §3.3). What carries it out is a plain function over the
//! simulation.
//!
//! # Adding verbs
//!
//! A package writes `pub fn verbs() -> Vec<VerbDef>` beside its other
//! tables and adds one line to [`engine_verbs`]. Each entry names the
//! verb, builds its specification against the registry (so that its
//! argument types can name kinds and definition types that are already
//! registered), and gives the function that runs it:
//!
//! ```ignore
//! VerbDef {
//!     name: "set_time_rate",
//!     build: |registry| {
//!         let entity = registry.kind_id("entity")?;
//!         Some(
//!             VerbSpec::new("set_time_rate")
//!                 .arg("of", Type::Subject(entity), "Whose personal clock.")
//!                 .optional_arg("rate", Type::Real, "The rate; none removes the clock.")
//!                 .doc("Gives an entity a clock of its own, or takes it away (03 §3.4)."),
//!         )
//!     },
//!     run: |sim, call, cause| { /* read call.args in declared order */ Ok(Value::Null) },
//! }
//! ```
//!
//! The function receives one value for each declared argument, in the
//! order of the specification, null for an optional one left out; it
//! must give every change it makes the cause it is handed, and it
//! refuses with a [`Reason`], never a panic. A verb changes the
//! simulation only through the simulation's own mutation paths.

use clash_expr::{Registry, Value, VerbSpec};
use clash_world::Cause;

use crate::reason::Reason;
use crate::rule::VerbCall;
use crate::sim::Sim;

/// What carries a verb out: the simulation, the call (one value for each
/// declared argument, in order) and the cause to give every change the
/// verb makes.
pub type VerbFn = fn(&mut Sim, &VerbCall<'_>, Cause) -> Result<Value, Reason>;

/// One verb of the engine: its name, its specification and what runs it.
#[derive(Clone, Copy)]
pub struct VerbDef {
    /// The key an effect is written with.
    pub name: &'static str,
    /// Builds the specification against a registry that already holds
    /// the engine's kinds and definition types. `None` when something
    /// the verb's arguments name is not registered: the verb is then
    /// left out.
    pub build: fn(&Registry) -> Option<VerbSpec>,
    /// Carries the verb out.
    pub run: VerbFn,
}

impl std::fmt::Debug for VerbDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerbDef").field("name", &self.name).finish()
    }
}

/// Every verb the simulation offers. A package that adds verbs adds its
/// table here, one line.
pub fn engine_verbs() -> Vec<VerbDef> {
    let mut verbs = Vec::new();
    verbs.extend(crate::item::verbs());
    verbs.extend(crate::movement::verbs());
    verbs.extend(crate::world_verbs::world_verbs());
    verbs
}

/// Enters the engine's verbs into a registry, leaving alone those it
/// already holds. Called when the engine's registry is built.
pub fn register_engine_verbs(registry: &mut Registry) {
    for verb in engine_verbs() {
        if registry.verb_id(verb.name).is_none()
            && let Some(spec) = (verb.build)(registry)
        {
            let _ = registry.add_verb(spec);
        }
    }
}
