//! Supply (16 §8.3): where a recipe's inputs and tools are looked for,
//! and the plan of what it would use.
//!
//! Inputs and tools are looked for, in order: in the worker's holds
//! (everything it holds, wears and carries, at any depth); in the
//! station's holds; and in containers within `recipe.supply_range` cells
//! of where the work is done, on the same storey: devices that hold
//! things, and items with `holds` lying there. Looking opens nothing:
//! a container whose contents are still latent (06 §9.3) is passed by.
//! A machine working unattended looks in its own holds alone.
//!
//! What a character is told about a recipe it might start is worked out
//! through [`belief`]: what it knows is at hand. Until perception keeps
//! what characters know of the contents of containers (15 §7.3), that
//! seam reads truth, as interactions' does (decision D-622).

use clash_core::{EntityId, Real};
use clash_world::{Location, OccupantFilter, OccupantKind, OccupantKinds, distinct_ids};

use super::defs::{InputDef, RecipeDef, ToolDef};
use super::{codes, recipe_name};
use crate::Sim;
use crate::item::{Item, ItemDef};
use crate::reason::Reason;
use crate::substance::{self, SubstanceId};

/// The setting that says how many cells from where the work is done
/// containers are looked in for inputs and tools (16 §8.3).
pub const SUPPLY_RANGE_SETTING: &str = "recipe.supply_range";

/// The default of [`SUPPLY_RANGE_SETTING`].
pub const DEFAULT_SUPPLY_RANGE: i64 = 6;

/// The seam through which a character's view of what is at hand is
/// read. Truth until perception keeps the contents of containers a
/// character has looked into.
pub mod belief {
    use clash_core::EntityId;

    use crate::Sim;

    /// The things a character believes are at hand for a recipe, in the
    /// order they are looked in.
    pub fn at_hand(
        sim: &Sim,
        worker: Option<EntityId>,
        station: Option<EntityId>,
        place: Option<clash_world::Location>,
    ) -> Vec<EntityId> {
        super::at_hand(sim, worker, station, place)
    }
}

/// Every item that might supply a recipe, in the order they are looked
/// in: what the worker holds, what the station holds, then what
/// containers within range hold. Each once.
pub fn at_hand(
    sim: &Sim,
    worker: Option<EntityId>,
    station: Option<EntityId>,
    place: Option<Location>,
) -> Vec<EntityId> {
    let mut found: Vec<EntityId> = Vec::new();
    let push = |found: &mut Vec<EntityId>, id: EntityId| {
        if !found.contains(&id) && sim.components.get::<Item>(id).is_some() {
            found.push(id);
        }
    };
    if let Some(worker) = worker {
        for id in open_contents(sim, worker) {
            push(&mut found, id);
        }
    }
    if let Some(station) = station {
        for id in open_contents(sim, station) {
            push(&mut found, id);
        }
    }
    // A machine alone has only what is in it.
    let (Some(place), Some(_)) = (place, worker) else {
        return found;
    };
    let range = sim
        .setting_int(SUPPLY_RANGE_SETTING, DEFAULT_SUPPLY_RANGE)
        .clamp(0, 64);
    let filter = OccupantFilter::ANY.of_kinds(OccupantKinds::of(&[
        OccupantKind::Device,
        OccupantKind::Item,
    ]));
    // Metres to the far corner of the square of cells, and a little.
    let radius = Real::from_i64(range) * Real::of(1.5) + Real::ONE;
    let Ok(near) = sim.cosmos.occupants_within(place, radius, &filter) else {
        return found;
    };
    for id in distinct_ids(&near) {
        let Some(loc) = sim.entities.location(id) else {
            continue;
        };
        let within = sim.cosmos.delta(place, loc).is_some_and(|delta| {
            delta.z == 0 && i64::from(delta.x.abs().max(delta.y.abs())) <= range
        });
        if !within || Some(id) == station || Some(id) == worker {
            continue;
        }
        let holds = crate::device::holds_of(sim, id).is_some()
            || sim
                .components
                .get::<Item>(id)
                .and_then(|item| crate::item::def_of(&sim.defs, item))
                .is_some_and(|def| def.holds.is_some());
        if holds {
            for inside in open_contents(sim, id) {
                push(&mut found, inside);
            }
        }
    }
    found
}

/// What is inside something at any depth, passing by what is inside a
/// container whose contents are still latent.
fn open_contents(sim: &Sim, holder: EntityId) -> Vec<EntityId> {
    let mut out = Vec::new();
    let mut stack: Vec<EntityId> = sim
        .entities
        .contents(holder)
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(id) = stack.pop() {
        out.push(id);
        let latent = sim.components.get::<Item>(id).is_some_and(Item::is_latent);
        if !latent {
            stack.extend(sim.entities.contents(id).iter().rev().copied());
        }
    }
    out
}

/// What a recipe would use: so many of each item, amounts of substances
/// from containers, the tools, and what it mends.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Items used up, each with how many of its members.
    pub takes: Vec<(EntityId, u32)>,
    /// Substances drawn: the container, the substance and the volume in
    /// cubic metres.
    pub draws: Vec<(EntityId, SubstanceId, Real)>,
    /// The tools, not used up.
    pub tools: Vec<EntityId>,
    /// The item it mends.
    pub mends: Option<EntityId>,
}

/// How one input stands: what it is, how much is wanted and how much is
/// at hand.
#[derive(Clone, Debug, PartialEq)]
pub struct InputStanding {
    /// What a reader calls it.
    pub what: String,
    /// How much is wanted: a count, or millilitres.
    pub need: Real,
    /// How much is at hand, at most what is wanted.
    pub have: Real,
}

fn item_def<'a>(sim: &'a Sim, id: EntityId) -> Option<&'a ItemDef> {
    sim.components
        .get::<Item>(id)
        .and_then(|item| crate::item::def_of(&sim.defs, item))
}

fn item_count(sim: &Sim, id: EntityId) -> u32 {
    sim.components.get::<Item>(id).map_or(0, Item::count)
}

fn matches_input(sim: &Sim, input: &InputDef, id: EntityId) -> bool {
    if let Some(def) = input.item {
        return sim
            .components
            .get::<Item>(id)
            .is_some_and(|item| item.def() == Some(def.id()));
    }
    if let Some(tag) = &input.tag {
        return sim
            .entities
            .get(id)
            .is_some_and(|entity| entity.tags.contains(tag));
    }
    false
}

fn matches_tool(sim: &Sim, tool: &ToolDef, id: EntityId) -> bool {
    if let Some(def) = tool.item {
        return sim
            .components
            .get::<Item>(id)
            .is_some_and(|item| item.def() == Some(def.id()));
    }
    if let Some(tag) = &tool.tag {
        return sim
            .entities
            .get(id)
            .is_some_and(|entity| entity.tags.contains(tag));
    }
    if let Some(quality) = &tool.quality {
        return item_def(sim, id).is_some_and(|def| {
            def.qualities
                .get(quality)
                .is_some_and(|level| *level >= tool.level)
        });
    }
    false
}

/// What a reader calls an input.
pub fn input_name(sim: &Sim, input: &InputDef) -> String {
    if let Some(def) = input.item {
        return super::item_def_name(sim, def.id());
    }
    if let Some(substance) = input.substance {
        return substance::name_of(sim, substance.id());
    }
    input.tag.clone().unwrap_or_default()
}

/// What a reader calls a tool.
pub fn tool_name(sim: &Sim, tool: &ToolDef) -> String {
    if let Some(def) = tool.item {
        return super::item_def_name(sim, def.id());
    }
    if let Some(quality) = &tool.quality {
        return format!("{quality} {}", tool.level);
    }
    tool.tag.clone().unwrap_or_default()
}

/// How each input of a recipe stands against what is at hand, in order.
pub fn standing(sim: &Sim, recipe: &RecipeDef, items: &[EntityId]) -> Vec<InputStanding> {
    let mut used: Vec<(EntityId, u32)> = Vec::new();
    let mut drawn: Vec<(EntityId, Real)> = Vec::new();
    recipe
        .inputs
        .iter()
        .map(|input| {
            let (need, have) = match input.substance {
                Some(substance) => {
                    let need = input.amount.map_or(Real::ZERO, |amount| amount.get());
                    let mut have = Real::ZERO;
                    for id in items {
                        let left = need - have;
                        if left <= Real::ZERO {
                            break;
                        }
                        let already = drawn
                            .iter()
                            .filter(|(container, _)| container == id)
                            .fold(Real::ZERO, |sum, (_, volume)| sum + *volume);
                        let there = substance::amount_of(sim, *id, substance.id()) - already;
                        let take = there.min(left).max(Real::ZERO);
                        if take > Real::ZERO {
                            drawn.push((*id, take));
                            have += take;
                        }
                    }
                    (need * substance::ML_PER_M3, have * substance::ML_PER_M3)
                }
                None => {
                    let need = input.count;
                    let mut have = 0;
                    for id in items {
                        if have >= need || !matches_input(sim, input, *id) {
                            continue;
                        }
                        let already: u32 = used
                            .iter()
                            .filter(|(item, _)| item == id)
                            .map(|(_, count)| *count)
                            .sum();
                        let take = item_count(sim, *id)
                            .saturating_sub(already)
                            .min(need - have);
                        if take > 0 {
                            used.push((*id, take));
                            have += take;
                        }
                    }
                    (
                        Real::from_i64(i64::from(need)),
                        Real::from_i64(i64::from(have)),
                    )
                }
            };
            InputStanding {
                what: input_name(sim, input),
                need,
                have,
            }
        })
        .collect()
}

/// What a recipe would use of what is at hand, or why it cannot be
/// started: an input short (`recipe.missing_input`), a tool missing
/// (`recipe.missing_tool`), nothing to mend (`recipe.nothing_to_mend`).
pub fn plan(
    sim: &Sim,
    id: super::RecipeId,
    recipe: &RecipeDef,
    items: &[EntityId],
) -> Result<Plan, Reason> {
    let name = recipe_name(sim, id);
    let mut plan = Plan::default();
    for (input, standing) in recipe.inputs.iter().zip(standing(sim, recipe, items)) {
        if standing.have < standing.need {
            return Err(Reason::new(
                codes::MISSING_INPUT,
                format!(
                    "{} more of {} is wanted",
                    format_amount(standing.need - standing.have, input.substance.is_some()),
                    standing.what
                ),
            )
            .with("recipe", name.as_str())
            .with("what", standing.what.as_str())
            .with("need", standing.need)
            .with("have", standing.have));
        }
        match input.substance {
            Some(substance) => {
                let mut left = input.amount.map_or(Real::ZERO, |amount| amount.get());
                for item in items {
                    if left <= Real::ZERO {
                        break;
                    }
                    let already = plan
                        .draws
                        .iter()
                        .filter(|(container, _, _)| container == item)
                        .fold(Real::ZERO, |sum, (_, _, volume)| sum + *volume);
                    let there = substance::amount_of(sim, *item, substance.id()) - already;
                    let take = there.min(left).max(Real::ZERO);
                    if take > Real::ZERO {
                        plan.draws.push((*item, substance.id(), take));
                        left -= take;
                    }
                }
            }
            None => {
                let mut left = input.count;
                for item in items {
                    if left == 0 {
                        break;
                    }
                    if !matches_input(sim, input, *item) {
                        continue;
                    }
                    let already: u32 = plan
                        .takes
                        .iter()
                        .filter(|(taken, _)| taken == item)
                        .map(|(_, count)| *count)
                        .sum();
                    let take = item_count(sim, *item).saturating_sub(already).min(left);
                    if take > 0 {
                        plan.takes.push((*item, take));
                        left -= take;
                    }
                }
            }
        }
    }
    for tool in &recipe.tools {
        let found = items.iter().copied().find(|item| {
            matches_tool(sim, tool, *item)
                && !plan.tools.contains(item)
                && !plan.takes.iter().any(|(taken, _)| taken == item)
        });
        match found {
            Some(item) => plan.tools.push(item),
            None => {
                let what = tool_name(sim, tool);
                return Err(
                    Reason::new(codes::MISSING_TOOL, format!("a tool is wanted: {what}"))
                        .with("recipe", name.as_str())
                        .with("what", what.as_str()),
                );
            }
        }
    }
    if let Some(repairs) = &recipe.repairs {
        let mends = items
            .iter()
            .copied()
            .filter(|item| {
                sim.entities
                    .get(*item)
                    .is_some_and(|entity| entity.tags.contains(&repairs.tag))
                    && sim
                        .components
                        .get::<Item>(*item)
                        .is_some_and(|it| it.damage() > 0)
                    && !plan.takes.iter().any(|(taken, _)| taken == item)
            })
            .max_by_key(|item| {
                (
                    sim.components.get::<Item>(*item).map_or(0, Item::damage),
                    std::cmp::Reverse(*item),
                )
            });
        match mends {
            Some(item) => plan.mends = Some(item),
            None => {
                return Err(Reason::new(
                    codes::NOTHING_TO_MEND,
                    format!("there is nothing damaged at hand that is {}", repairs.tag),
                )
                .with("recipe", name.as_str())
                .with("tag", repairs.tag.as_str()));
            }
        }
    }
    Ok(plan)
}

/// An amount as a reader is told it: a count, or millilitres.
pub fn format_amount(amount: Real, millilitres: bool) -> String {
    let value = amount.get();
    if millilitres {
        format!("{value:.0} ml")
    } else {
        format!("{value:.0}")
    }
}
