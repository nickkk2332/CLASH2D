//! The kernel's own verbs (03 §13): what a rule does to clocks, control
//! and tags.
//!
//! | Verb | Does | Raises |
//! |------|------|--------|
//! | `set_time_rate { who, rate }` | Gives an entity a personal clock at a rate, or with a null rate takes it away | `clock_rate_changed` |
//! | `anchor_clock { who, to }` | Pins an entity's personal clock under the clock of an area, a realm, a cell or another entity; null lifts the anchor | `clock_rate_changed`, when the entity is given a clock by it |
//! | `set_clock_rate { of, rate }` | Changes the rate of an area's or a realm's clock; null takes an area's clock away | `clock_rate_changed` |
//! | `set_control { who, mode }` | Changes who decides for an entity: `none`, `direct` or `delegated` | `control_changed` |
//! | `add_tag { to, tag }`, `remove_tag { from, tag }` | Gives an entity a tag, or takes one away | `tag_changed`, if that changed anything |
//!
//! Each goes through the function the engine itself uses, with the
//! cause it is handed, and declines with a [`Reason`]. The verbs that
//! start, end and plan actions are the action runtime's
//! ([`action`](crate::action)); those of entities, bodies, items and
//! movement are their packages'.

use clash_core::{EntityId, Real};
use clash_expr::{Type, Value, VerbSpec};
use clash_time::ClockId;
use clash_world::Cause;

use crate::Sim;
use crate::action::ActEvent;
use crate::clock::ClockTarget;
use crate::entity::Control;
use crate::event::{EventBody, Subject, Where};
use crate::reason::{Reason, ReasonInfo};
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// A verb was given an argument it cannot use: null where something is
/// needed, or a subject of another kind.
pub const BAD_ARGUMENT: &str = "verb.bad_argument";

/// The reasons the engine's verbs decline with beside those of the
/// functions they call, for the generated reference.
pub const VERB_REASONS: &[ReasonInfo] = &[ReasonInfo {
    code: BAD_ARGUMENT,
    doc: "A verb was given an argument it cannot use: null where a value is needed, a subject of another kind than the verb takes, or a word that is not one of those it knows. Nothing was done. Values: `verb`, `argument`.",
}];

/// The reason for an argument a verb cannot use.
pub(crate) fn bad_argument(verb: &str, argument: &str, what: impl Into<String>) -> Reason {
    Reason::new(BAD_ARGUMENT, format!("`{verb}`: {}", what.into()))
        .with("verb", verb)
        .with("argument", argument)
}

/// The argument at a position, or null.
pub(crate) fn arg<'a>(call: &'a VerbCall<'_>, at: usize) -> &'a Value {
    call.args.get(at).unwrap_or(&Value::Null)
}

/// An argument that must name an entity that exists.
pub(crate) fn entity_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    verb: &str,
    name: &str,
) -> Result<EntityId, Reason> {
    match sim.subject_of(arg(call, at)) {
        Some(Subject::Entity(id)) if sim.truth().entity(id).is_some() => Ok(id),
        _ => Err(bad_argument(
            verb,
            name,
            format!("`{name}` must be an entity that exists"),
        )),
    }
}

/// An argument that must be a text.
pub(crate) fn text_arg<'a>(
    call: &'a VerbCall<'_>,
    at: usize,
    verb: &str,
    name: &str,
) -> Result<&'a str, Reason> {
    arg(call, at)
        .as_str()
        .ok_or_else(|| bad_argument(verb, name, format!("`{name}` must be a text")))
}

/// A rate, or `None` for null.
fn rate_arg(call: &VerbCall<'_>, at: usize, verb: &str) -> Result<Option<Real>, Reason> {
    match arg(call, at) {
        Value::Null => Ok(None),
        value => value.as_number().map(Some).ok_or_else(|| {
            bad_argument(
                verb,
                "rate",
                "`rate` must be a number, or null for no clock",
            )
        }),
    }
}

fn set_time_rate(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "set_time_rate", "who")?;
    let rate = rate_arg(call, 1, "set_time_rate")?;
    sim.set_time_rate(who, rate, cause)?;
    Ok(Value::Null)
}

/// The clock a subject stands for, as something to anchor under.
fn clock_under(sim: &Sim, to: &Value) -> Result<ClockId, Reason> {
    let no_clock = || {
        bad_argument(
            "anchor_clock",
            "to",
            "`to` must be an area with a clock, a realm, a cell or an entity in the world",
        )
    };
    let truth = sim.truth();
    match sim.subject_of(to).ok_or_else(no_clock)? {
        Subject::Area(id) => truth
            .cosmos()
            .area(id)
            .and_then(|area| area.clock)
            .ok_or_else(no_clock),
        Subject::Realm(id) => truth
            .cosmos()
            .realm(id)
            .map(|realm| realm.clock)
            .ok_or_else(no_clock),
        Subject::Loc(loc) | Subject::Element(loc, _) => Ok(sim.location_clock(loc)),
        Subject::Entity(id) => truth.clock_of(id).ok_or_else(no_clock),
        _ => Err(no_clock()),
    }
}

fn anchor_clock(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "anchor_clock", "who")?;
    let anchor = match arg(call, 1) {
        Value::Null => None,
        to => Some(clock_under(sim, to)?),
    };
    sim.anchor_clock(who, anchor, cause)?;
    Ok(Value::Null)
}

fn set_clock_rate(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let target = match sim.subject_of(arg(call, 0)) {
        Some(Subject::Area(id)) => ClockTarget::Area(id),
        Some(Subject::Realm(id)) => ClockTarget::Realm(id),
        _ => {
            return Err(bad_argument(
                "set_clock_rate",
                "of",
                "`of` must be an area or a realm",
            ));
        }
    };
    let rate = rate_arg(call, 1, "set_clock_rate")?;
    sim.set_clock_rate(target, rate, cause)?;
    Ok(Value::Null)
}

/// A control mode by the name data writes.
pub(crate) fn control_named(name: &str) -> Option<Control> {
    Some(match name {
        "none" => Control::None,
        "direct" => Control::Direct,
        "delegated" => Control::Delegated,
        _ => return None,
    })
}

fn set_control(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "set_control", "who")?;
    let mode = text_arg(call, 1, "set_control", "mode")?;
    let control = control_named(mode).ok_or_else(|| {
        bad_argument(
            "set_control",
            "mode",
            format!("`{mode}` is not a mode: it is `none`, `direct` or `delegated`"),
        )
    })?;
    sim.set_control(who, control, cause)?;
    Ok(Value::Null)
}

fn add_tag(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let to = entity_arg(sim, call, 0, "add_tag", "to")?;
    let tag = text_arg(call, 1, "add_tag", "tag")?;
    Ok(Value::Bool(sim.set_tag(to, tag, true, cause)?))
}

fn remove_tag(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let from = entity_arg(sim, call, 0, "remove_tag", "from")?;
    let tag = text_arg(call, 1, "remove_tag", "tag")?;
    Ok(Value::Bool(sim.set_tag(from, tag, false, cause)?))
}

impl Sim {
    /// Gives an entity a tag, or takes one away. Returns whether that
    /// changed anything, and emits `tag_changed` if it did. Refused for
    /// an unknown entity.
    pub fn set_tag(
        &mut self,
        entity: EntityId,
        tag: &str,
        present: bool,
        cause: Cause,
    ) -> Result<bool, Reason> {
        self.known(entity)?;
        let changed = self.entity_mut(entity).is_some_and(|record| {
            if present {
                record.tags.insert(tag)
            } else {
                record.tags.remove(tag)
            }
        });
        if changed {
            self.emit(
                Where::Entity(entity),
                cause,
                EventBody::Act(ActEvent::TagChanged {
                    entity,
                    tag: tag.to_owned(),
                    added: present,
                }),
            );
        }
        Ok(changed)
    }
}

/// The kernel's verbs. See the [module documentation](self).
pub fn kernel_verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "set_time_rate",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_time_rate")
                        .arg("who", Type::Subject(entity), "Whose personal clock.")
                        .optional_arg(
                            "rate",
                            Type::Real,
                            "Its rate against the clock of where the entity is: zero or more. Null, or left out, takes the personal clock away, and any anchor with it.",
                        )
                        .doc("Gives an entity a clock of its own at a rate, changes the rate of the one it has, or takes it away (03 §3.5). Everything about the entity runs on it. Raises `clock_rate_changed`."),
                )
            },
            run: set_time_rate,
        },
        VerbDef {
            name: "anchor_clock",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("anchor_clock")
                        .arg("who", Type::Subject(entity), "Whose personal clock.")
                        .optional_arg(
                            "to",
                            Type::Any,
                            "What to pin it under: an area that has a clock, a realm, a cell (the clock of that place) or an entity (the clock it is on). Null, or left out, lifts the anchor.",
                        )
                        .doc("Pins an entity's personal clock under a chosen clock, whatever the entity stands in (03 §3.4). An entity with no personal clock is given one at rate 1, which raises `clock_rate_changed`."),
                )
            },
            run: anchor_clock,
        },
        VerbDef {
            name: "set_clock_rate",
            build: |_| {
                Some(
                    VerbSpec::new("set_clock_rate")
                        .arg("of", Type::Any, "An area or a realm.")
                        .optional_arg(
                            "rate",
                            Type::Real,
                            "The rate against the clock above it: zero or more. Null, or left out, takes an area's clock away and is refused for a realm.",
                        )
                        .doc("Changes the rate of an area's or a realm's clock (03 §13). Whoever is inside an area that gains or loses its clock is put on the clock that now applies. Raises `clock_rate_changed`."),
                )
            },
            run: set_clock_rate,
        },
        VerbDef {
            name: "set_control",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_control")
                        .arg("who", Type::Subject(entity), "Whose.")
                        .arg("mode", Type::Str, "`none`, `direct` or `delegated`.")
                        .doc("Changes who decides for an entity (03 §5.1). Raises `control_changed` if that changes anything; one that comes under direct control with nothing to do is asked what to do."),
                )
            },
            run: set_control,
        },
        VerbDef {
            name: "add_tag",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("add_tag")
                        .arg("to", Type::Subject(entity), "Who is given the tag.")
                        .arg("tag", Type::Str, "The tag: any text.")
                        .doc("Gives an entity a tag. Raises `tag_changed` if it did not have it."),
                )
            },
            run: add_tag,
        },
        VerbDef {
            name: "remove_tag",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("remove_tag")
                        .arg("from", Type::Subject(entity), "Who loses the tag.")
                        .arg("tag", Type::Str, "The tag.")
                        .doc("Takes a tag away from an entity. Raises `tag_changed` if it had it."),
                )
            },
            run: remove_tag,
        },
    ]
}
