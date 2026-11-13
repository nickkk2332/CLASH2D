//! Interactions in play (16 §3; 12 §3.2, §9.4): what walking into
//! something closed offers, and what a screen is told of an offer.
//!
//! **Bumping.** A step the simulation refuses because the way is closed
//! (`move.closed`: a shut door) is a bump. The frame loop asks the
//! simulation what the thing there offers by default
//! (`interaction::bump_default`): the block's, else the fixture's, else
//! the floor's default interaction that can be done now. Then, by the
//! preference named [`PREF_BUMP`]:
//!
//! | Value | Walking into it |
//! |-------|-----------------|
//! | [`BUMP_AT_ONCE`] (or no such preference) | Runs the control named [`CONTROL_BUMP`] with the offer as `it`: it does it |
//! | [`BUMP_ASK`] | Runs the control named [`CONTROL_BUMP_ASK`] with the offer as `it`: it asks first |
//! | [`BUMP_NEVER`] | Nothing: the step's refusal is told |
//!
//! The controls are data: the `ui` mod's start the action `interact`
//! with the offer's `target` and `interaction`, as any action whose
//! parameters are so named is an interaction's (16 §3.5.1). Where
//! nothing there can be done by default but something would be, its
//! refusal is told in place of the step's: walking into a locked door
//! says why it does not open.
//!
//! **Walking into someone** (decision D-790). A step toward a creature
//! the party sees standing that way is not submitted: the frame loop
//! offers to attack it instead, by the preference named
//! [`PREF_BUMP_ATTACK`]:
//!
//! | Value | Walking into someone |
//! |-------|----------------------|
//! | [`BUMP_HOSTILE`] (or no such preference) | One the character holds hostile or fears (`hostile`, `fears`, 09 §3.4) is attacked at once: the control named [`CONTROL_ATTACK_BUMP`] runs with them as `it`; anyone else is asked about first, with the control named [`CONTROL_ATTACK_BUMP_ASK`] |
//! | [`BUMP_ALWAYS`] | Anyone is attacked at once |
//! | [`BUMP_ASK`] | Anyone is asked about first |
//! | [`BUMP_NEVER`] | The step is submitted as any other: the simulation refuses it when it is taken |
//!
//! `it` is a map: `target`, the creature as the simulation's value for
//! it; `where`, the direction walked; and `hostile`, whether the
//! character holds it hostile or fears it. The `ui` mod's controls
//! start the first attack that can be made (`view.actions_with`).
//!
//! **An offer as a value** ([`offer_value`]) is a map with the fields
//! the simulation's property `offers_near` gives each row: `target`,
//! `interaction`, `id`, `name`, `default`, `available`, `why` and
//! `where`. A screen lists `view.offers_near` itself (the `ui` mod's
//! `interact` screen); the application only answers a bump.

use clash_core::EntityId;
use clash_data::Def;
use clash_expr::{DefRef, Value};
use clash_sim::Sim;
use clash_sim::interaction::{self, InteractionDef, Offer, Target};
use clash_world::{CellPos, Layer, Location};

/// The bare name of the preference that says what walking into
/// something closed does. The application acts on it wherever a loaded
/// mod defines it.
pub const PREF_BUMP: &str = "bump";
/// It is done at once.
pub const BUMP_AT_ONCE: &str = "at_once";
/// The player is asked first.
pub const BUMP_ASK: &str = "ask";
/// Nothing is done: the step is refused.
pub const BUMP_NEVER: &str = "never";

/// The bare name of the control run, with the offer as `it`, to do what
/// walking into something offers.
pub const CONTROL_BUMP: &str = "bump";
/// The bare name of the control run, with the offer as `it`, to ask
/// first.
pub const CONTROL_BUMP_ASK: &str = "bump_ask";

/// The bare name of the preference that says what walking into someone
/// the character sees does.
pub const PREF_BUMP_ATTACK: &str = "bump_attack";
/// Those the character holds hostile or fears are attacked at once,
/// anyone else after asking.
pub const BUMP_HOSTILE: &str = "hostile";
/// Anyone is attacked at once.
pub const BUMP_ALWAYS: &str = "always";
/// The bare name of the control run, with the creature as `it`, to
/// attack someone walked into.
pub const CONTROL_ATTACK_BUMP: &str = "attack_bump";
/// The bare name of the control run, with the creature as `it`, to ask
/// before attacking someone walked into.
pub const CONTROL_ATTACK_BUMP_ASK: &str = "attack_bump_ask";

/// What walking into a creature offers: the map a control is given as
/// `it` (see the [module](self) documentation).
pub fn creature_value(sim: &Sim, user: EntityId, target: EntityId, dir: &str) -> Value {
    let who = sim.subject_value(clash_sim::Subject::Entity(target));
    let me = clash_sim::Subject::Entity(user);
    let holds = |property: &str| {
        sim.property_named(me, property, std::slice::from_ref(&who)) == Value::Bool(true)
    };
    let hostile = holds("hostile") || holds("fears");
    Value::map([
        ("target", who.clone()),
        ("where", Value::from(dir.replace('_', "-").as_str())),
        ("hostile", Value::Bool(hostile)),
    ])
}

/// An offer as a screen or a control reads it: see the
/// [module](self) documentation. `place` is where the target is from
/// the user, in a word.
pub fn offer_value(sim: &Sim, target: Target, offer: &Offer, place: &str) -> Value {
    let interaction = match sim.exprs().registry().def_type_id(InteractionDef::TYPE) {
        Some(ty) => Value::Id(DefRef::new(ty, u32::from(offer.interaction.raw()))),
        None => Value::from(offer.id.as_str()),
    };
    let why = offer
        .refusal
        .as_ref()
        .map_or_else(String::new, |reason| reason.message.clone());
    Value::map([
        ("target", sim.subject_value(target.subject())),
        ("interaction", interaction),
        ("id", Value::from(offer.id.as_str())),
        ("name", Value::from(offer.name.as_str())),
        ("default", Value::Bool(offer.default)),
        ("available", Value::Bool(offer.refusal.is_none())),
        ("why", Value::from(why.as_str())),
        ("where", Value::from(place)),
    ])
}

/// What walking a way into something closed offers: the default that
/// can be done, as a value, or else why the default there cannot be
/// done, if there is one that cannot.
pub fn bumped(sim: &Sim, user: EntityId, dir: &str) -> Result<Value, Option<String>> {
    if let Some((target, offer)) = interaction::bump_default(sim, user, dir) {
        return Ok(offer_value(sim, target, &offer, &dir.replace('_', "-")));
    }
    // Nothing can be done by default: say why the default cannot.
    let next = sim.truth().location(user).and_then(|from| {
        let delta = clash_sim::movement::parse_dir(dir)?.delta();
        let cell = CellPos::new(
            from.cell.x + delta.x,
            from.cell.y + delta.y,
            from.cell.z + delta.z,
        );
        sim.truth()
            .cosmos()
            .normalize(Location::new(from.grid, cell))
            .ok()
    });
    let Some(next) = next else {
        return Err(None);
    };
    // Every default there that cannot be done says why, each once, in
    // the order offered: a locked door that will not open and needs its
    // key says both.
    let mut why: Vec<String> = Vec::new();
    for layer in [Layer::Block, Layer::Fixture, Layer::Floor] {
        for offer in interaction::interactions_for(sim, user, Target::Element(next, layer)) {
            if let (true, Some(reason)) = (offer.default, offer.refusal)
                && !why.contains(&reason.message)
            {
                why.push(reason.message);
            }
        }
    }
    Err((!why.is_empty()).then(|| why.join(" ")))
}
