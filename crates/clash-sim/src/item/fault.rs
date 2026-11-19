//! What a character could do with things, as it believes, before it
//! tries (06 §9.2; decision D-865): the properties `hold_fault` and
//! `loose_near`, which content's item actions read in their `requires`,
//! `fail_text` and parameter lists.
//!
//! The verbs of [`super::hold`] refuse in the engine's words, for
//! rules and the console. A player is told why in the reader's: these
//! checks make the verbs' own, changing nothing, on what the character
//! knows of itself and on what it perceives within reach, and say what
//! is wrong as a sentence for the character to read ("Your hands are
//! full.").
//!
//! **What is perceived.** A thing lying loose is within reach when it
//! lies in the character's cell or one of the eight round it on its
//! storey. For a member of the party it must also lie in a cell the
//! member had in view at its last look, or in its own cell; for anyone
//! else whatever lies within reach counts. Items give off nothing a
//! sense of sight sweeps for, so the view of the cell stands for seeing
//! what lies in it, as the application draws it (12 §3.5).

use clash_core::EntityId;
use clash_expr::Value;
use clash_world::{CellPos, Location, OccupantFilter, distinct_ids};

use super::hold::{self, Hold, Holds};
use super::{Item, def_of};
use crate::Sim;
use crate::body::Body;
use crate::entity::Place;
use crate::event::Subject;

/// The verbs `hold_fault` answers for, as rules write them.
pub const FAULT_VERBS: [&str; 9] = [
    "take", "grasp", "drop", "wear", "remove", "stow", "put_in", "take_out", "give",
];

/// What the reader calls a thing, with the article it takes.
fn the(sim: &Sim, reader: EntityId, of: EntityId) -> String {
    let value = sim.subject_value(Subject::Entity(of));
    sim.reader_text("the", &value, None, Some(reader))
        .unwrap_or_else(|| "it".to_owned())
}

/// The same, with a capital first letter, to begin a sentence.
fn the_cap(sim: &Sim, reader: EntityId, of: EntityId) -> String {
    crate::reader::capitalised(&the(sim, reader, of))
}

/// What the reader calls a thing with the indefinite article.
fn a(sim: &Sim, reader: EntityId, of: EntityId) -> String {
    let value = sim.subject_value(Subject::Entity(of));
    sim.reader_text("a", &value, None, Some(reader))
        .unwrap_or_else(|| "something".to_owned())
}

/// Whether two cells are within reach of each other: the same cell, or
/// one of the eight round it on the same storey.
fn within_reach(sim: &Sim, from: Location, to: Location) -> bool {
    sim.cosmos
        .delta(from, to)
        .is_some_and(|delta| delta.z == 0 && delta.x.abs() <= 1 && delta.y.abs() <= 1)
}

/// Whether `who` perceives what lies in a cell: its own, one its last
/// look had in view if it is of the party, any if it is not.
fn sees_cell(sim: &Sim, who: EntityId, at: Location, loc: Location) -> bool {
    at == loc || crate::perception::party::member_sees(sim, who, loc).unwrap_or(true)
}

/// Where a thing lying loose is, if it lies loose: in a cell, inside
/// nothing.
fn lying_at(sim: &Sim, what: EntityId) -> Option<Location> {
    match sim.entities.get(what)?.place() {
        Place::At { .. } => sim.truth().location(what),
        _ => None,
    }
}

/// Whether `who` perceives a thing within its reach: lying loose there,
/// or carried by itself.
fn reaches(sim: &Sim, who: EntityId, what: EntityId) -> bool {
    if sim.entities.contents_deep(who).contains(&what) {
        return true;
    }
    let (Some(at), Some(there)) = (sim.truth().location(who), sim.truth().location(what)) else {
        return false;
    };
    within_reach(sim, at, there) && sees_cell(sim, who, at, there)
}

/// The things `who` perceives lying loose within reach, its own cell
/// first, then nearest, then by id: `entity.loose_near`.
pub fn loose_near(sim: &Sim, who: EntityId) -> Vec<EntityId> {
    let Some(at) = sim.truth().location(who) else {
        return Vec::new();
    };
    let mut found: Vec<(i32, EntityId)> = Vec::new();
    for dy in -1..=1 {
        for dx in -1..=1 {
            let cell = CellPos::new(at.cell.x + dx, at.cell.y + dy, at.cell.z);
            let Ok(loc) = sim.cosmos.normalize(Location::new(at.grid, cell)) else {
                continue;
            };
            if !sees_cell(sim, who, at, loc) {
                continue;
            }
            let Ok(occupants) = sim
                .cosmos
                .occupants_at(loc, &OccupantFilter::ANY.except(who))
            else {
                continue;
            };
            for id in distinct_ids(&occupants) {
                if sim.components.get::<Item>(id).is_some() && lying_at(sim, id).is_some() {
                    found.push((dx.abs() + dy.abs(), id));
                }
            }
        }
    }
    found.sort();
    found.dedup_by_key(|(_, id)| *id);
    found.into_iter().map(|(_, id)| id).collect()
}

/// Where a thing within reach is from `who`, in a word: `here`, or one
/// of the eight compass directions.
fn where_from(sim: &Sim, who: EntityId, what: EntityId) -> &'static str {
    let (Some(from), Some(to)) = (sim.truth().location(who), sim.truth().location(what)) else {
        return "";
    };
    match sim
        .cosmos
        .delta(from, to)
        .map(|delta| (delta.x.signum(), delta.y.signum()))
    {
        Some((0, 0)) => "here",
        Some((0, -1)) => "north",
        Some((1, -1)) => "north-east",
        Some((1, 0)) => "east",
        Some((1, 1)) => "south-east",
        Some((0, 1)) => "south",
        Some((-1, 1)) => "south-west",
        Some((-1, 0)) => "west",
        Some((-1, -1)) => "north-west",
        _ => "",
    }
}

/// `entity.loose_near`, as rows a list shows and a picker takes: each a
/// map of `value` (the thing), `label` (what the reader calls it, with
/// its article: `a parcel`) and `where` (`here`, `north`, ...).
pub fn loose_near_value(sim: &Sim, who: EntityId) -> Value {
    Value::list(loose_near(sim, who).into_iter().map(|id| {
        Value::map([
            ("value", sim.subject_value(Subject::Entity(id))),
            ("label", Value::from(a(sim, who, id).as_str())),
            ("where", Value::from(where_from(sim, who, id))),
        ])
    }))
}

/// Why `who` could not take a thing into a hand now: it has nothing to
/// hold it with, the thing has no grip, or its hands are full.
fn grasp_fault(sim: &Sim, who: EntityId, what: EntityId) -> Option<String> {
    if sim.components.get::<Body>(who).is_none() {
        return Some("You have nothing to hold it with.".to_owned());
    }
    let grip = hold::grip_of(sim, what);
    if grip.is_empty() {
        return Some(format!(
            "{} cannot be held in a hand.",
            the_cap(sim, who, what)
        ));
    }
    let in_hand = hold::holder_of(sim, what) == Some(who)
        && matches!(hold::hold_of(sim, what), Some(Hold::Grasp(_)));
    if in_hand || super::stack::grasped_equal(sim, who, what).is_some() {
        return None;
    }
    hold::allocate(sim, who, &grip, None, Some(what))
        .err()
        .map(|_| "Your hands are full: put something down or away first.".to_owned())
}

/// The first container `who` wears or holds that would take a thing,
/// as `stow` finds it.
fn stow_into(sim: &Sim, who: EntityId, what: EntityId) -> Option<EntityId> {
    sim.entities
        .contents(who)
        .iter()
        .copied()
        .find(|container| {
            *container != what
                && hold::hold_of(sim, *container).is_some_and(|hold| hold.in_use())
                && hold::check_inside(sim, what, *container).is_ok()
                && hold::fits(sim, *container, what).is_ok()
        })
}

/// Why a container would not take a thing, in the reader's words.
fn fit_fault(sim: &Sim, who: EntityId, what: EntityId, container: EntityId) -> Option<String> {
    let reason = hold::check_inside(sim, what, container)
        .and_then(|()| hold::fits(sim, container, what))
        .err()?;
    let (thing, into) = (the_cap(sim, who, what), the(sim, who, container));
    Some(match reason.code.as_str() {
        super::reason::NOT_A_CONTAINER => {
            format!("{} holds nothing.", the_cap(sim, who, container))
        }
        super::reason::NOT_ACCEPTED => format!(
            "{} does not take {}.",
            the_cap(sim, who, container),
            a(sim, who, what)
        ),
        super::reason::DOES_NOT_FIT => format!("{thing} does not fit in {into}."),
        _ => format!("{thing} cannot go in {into}."),
    })
}

/// Why `who`, as it believes, could not do one of the holding verbs to
/// a thing now: `None` if it could. See [`FAULT_VERBS`] and the property
/// `hold_fault` for what each checks.
pub fn hold_fault(
    sim: &Sim,
    who: EntityId,
    verb: &str,
    what: Option<EntityId>,
    other: Option<EntityId>,
) -> Option<String> {
    let Some(what) = what.filter(|what| sim.entities.contains(*what)) else {
        return Some("There is nothing there.".to_owned());
    };
    if what == who {
        return Some(if verb == "take" {
            "There is nothing within reach to pick up.".to_owned()
        } else {
            "You cannot do that to yourself.".to_owned()
        });
    }
    let item = sim.components.get::<Item>(what);
    if item.is_none() {
        return Some(format!(
            "{} is not something you can carry.",
            the_cap(sim, who, what)
        ));
    }
    let carried = sim.entities.contents_deep(who).contains(&what);
    let holder = hold::holder_of(sim, what);
    let hold = hold::hold_of(sim, what);
    let not_had = || format!("You do not have {}.", the(sim, who, what));
    match verb {
        "take" => {
            if !carried {
                if let Some(holder) = holder {
                    return Some(if sim.components.mind.contains(holder) {
                        format!("{} has it.", the_cap(sim, who, holder))
                    } else {
                        format!("It is in {}: take it out of that.", the(sim, who, holder))
                    });
                }
                if !reaches(sim, who, what) {
                    return Some(format!("{} is out of reach.", the_cap(sim, who, what)));
                }
            } else if holder == Some(who) && matches!(hold, Some(Hold::Grasp(_))) {
                return Some(format!("You already have {} in hand.", the(sim, who, what)));
            }
            // What no hand holds goes into something carried that takes it.
            if hold::grip_of(sim, what).is_empty() {
                return stow_into(sim, who, what)
                    .is_none()
                    .then(|| format!("You have nothing to carry {} in.", the(sim, who, what)));
            }
            grasp_fault(sim, who, what)
        }
        "grasp" => grasp_fault(sim, who, what),
        "drop" => (!carried).then(not_had),
        "wear" => {
            if !carried && !reaches(sim, who, what) {
                return Some(format!("{} is out of reach.", the_cap(sim, who, what)));
            }
            if holder == Some(who) && matches!(hold, Some(Hold::Worn(_))) {
                return Some(format!("You are already wearing {}.", the(sim, who, what)));
            }
            let def = item.and_then(|item| def_of(&sim.defs, item));
            let Some(def) = def.filter(|def| def.is_wearable()) else {
                return Some(format!(
                    "{} is not something to wear.",
                    the_cap(sim, who, what)
                ));
            };
            let targets = match hold::wear_targets(sim, who, def) {
                Ok(targets) if !targets.is_empty() => targets,
                _ => {
                    return Some(format!(
                        "{} does not go on you anywhere.",
                        the_cap(sim, who, what)
                    ));
                }
            };
            let holds = sim.components.get::<Holds>(who)?;
            holds.iter().find_map(|(other, hold)| {
                let Hold::Worn(on) = hold else { return None };
                let clash = other != what
                    && on.iter().any(|have| {
                        targets
                            .iter()
                            .any(|want| want.part == have.part && want.layer == have.layer)
                    });
                clash.then(|| {
                    format!(
                        "You are already wearing {} there: take it off first.",
                        the(sim, who, other)
                    )
                })
            })
        }
        "remove" => {
            if holder != Some(who) || !matches!(hold, Some(Hold::Worn(_))) {
                return Some(format!("You are not wearing {}.", the(sim, who, what)));
            }
            grasp_fault(sim, who, what)
        }
        "stow" => {
            if !carried {
                return Some(not_had());
            }
            if matches!(hold, Some(Hold::Stowed)) {
                return Some(format!("{} is put away already.", the_cap(sim, who, what)));
            }
            stow_into(sim, who, what)
                .is_none()
                .then(|| format!("You carry nothing {} would go in.", the(sim, who, what)))
        }
        "put_in" => {
            if !carried {
                return Some(not_had());
            }
            let Some(container) = other.filter(|other| sim.entities.contains(*other)) else {
                return Some("There is nothing there to put it in.".to_owned());
            };
            if container == what {
                return Some(format!(
                    "{} cannot go inside itself.",
                    the_cap(sim, who, what)
                ));
            }
            if container != who && !reaches(sim, who, container) {
                return Some(format!("{} is out of reach.", the_cap(sim, who, container)));
            }
            fit_fault(sim, who, what, container)
        }
        "take_out" => {
            let in_something = matches!(hold, Some(Hold::Stowed | Hold::Inside));
            let container = holder.filter(|_| in_something);
            let Some(container) = container.filter(|container| *container != who) else {
                return Some(format!("{} is not in anything.", the_cap(sim, who, what)));
            };
            if !reaches(sim, who, container) {
                return Some(format!("{} is out of reach.", the_cap(sim, who, container)));
            }
            grasp_fault(sim, who, what)
        }
        "give" => {
            if !carried {
                return Some(not_had());
            }
            let Some(to) = other.filter(|other| sim.entities.contains(*other)) else {
                return Some("There is nobody there to give it to.".to_owned());
            };
            if to == who {
                return Some("You have it already.".to_owned());
            }
            let (Some(at), Some(there)) = (sim.truth().location(who), sim.truth().location(to))
            else {
                return Some(format!("{} is out of reach.", the_cap(sim, who, to)));
            };
            if !within_reach(sim, at, there) || !sees_cell(sim, who, at, there) {
                return Some(format!("{} is out of reach.", the_cap(sim, who, to)));
            }
            if sim.components.get::<Body>(to).is_none() {
                return Some(format!("{} cannot take it.", the_cap(sim, who, to)));
            }
            let grip = hold::grip_of(sim, what);
            (grip.is_empty() || hold::allocate(sim, to, &grip, None, Some(what)).is_err())
                .then(|| format!("{} has no hand free for it.", the_cap(sim, who, to)))
        }
        other => Some(format!(
            "`{other}` is none of the holding verbs: {}.",
            FAULT_VERBS.join(", ")
        )),
    }
}
