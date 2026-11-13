//! What a character could make here (16 §8, 12 §10.7 Make): every
//! recipe done by hand, and every recipe of each station within a step,
//! with what it takes, what of that is at hand, what it gives and why it
//! cannot be started if it cannot. Worked out from what the character
//! believes is at hand (`supply::belief`); changes nothing.

use clash_core::{EntityId, Real};
use clash_expr::Value;
use clash_world::{
    CellPos, Layer, Location, OccupantFilter, OccupantKind, OccupantKinds, distinct_ids,
};

use super::defs::{RecipeDef, RecipeId};
use super::supply;
use super::work::{self, Starting};
use crate::Sim;
use crate::device;
use crate::event::Subject;
use crate::interaction::Target;
use crate::reason::Reason;

/// One recipe a character might start, at one station or by hand.
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeOption {
    /// The recipe.
    pub recipe: RecipeId,
    /// Where it would be made: a station, or `None` by hand.
    pub station: Option<Target>,
    /// Why it cannot be started now, or `None` if it can.
    pub refusal: Option<Reason>,
    /// Whether starting it would go on with work under way.
    pub resumes: bool,
}

/// The stations within a step of a character: each element of its cell
/// and the eight round it on its storey whose definition names a device
/// that carries a tag, and each device entity standing there, once, in
/// order of cell and layer.
pub fn stations_near(sim: &Sim, who: EntityId) -> Vec<Target> {
    let Some(at) = sim.entities.location(who) else {
        return Vec::new();
    };
    let mut found: Vec<Target> = Vec::new();
    for dy in -1..=1 {
        for dx in -1..=1 {
            let cell = CellPos::new(at.cell.x + dx, at.cell.y + dy, at.cell.z);
            let Ok(loc) = sim.cosmos.normalize(Location::new(at.grid, cell)) else {
                continue;
            };
            for layer in [Layer::Block, Layer::Fixture, Layer::Floor] {
                let target = Target::Element(loc, layer);
                let Some(def) = device::def_for(sim, target) else {
                    continue;
                };
                if def.tags.is_empty() {
                    continue;
                }
                // A promoted element is listed as its device.
                let listed = device::device_now(sim, target).map_or(target, Target::Entity);
                if !found.contains(&listed) {
                    found.push(listed);
                }
            }
            let filter = OccupantFilter::ANY.of_kinds(OccupantKinds::of(&[OccupantKind::Device]));
            if let Ok(here) = sim.cosmos.occupants_at(loc, &filter) {
                for id in distinct_ids(&here) {
                    let target = Target::Entity(id);
                    let carries =
                        device::def_for(sim, target).is_some_and(|def| !def.tags.is_empty());
                    if carries && !found.contains(&target) {
                        found.push(target);
                    }
                }
            }
        }
    }
    found
}

/// What a character could make here (16 §8; 12 §10.7): each recipe by
/// hand, then each recipe of each station within a step, in order of
/// recipe; each with the reason it cannot be started, if it cannot.
pub fn recipe_options(sim: &Sim, who: EntityId) -> Vec<RecipeOption> {
    let Some(registry) = sim.defs.try_registry::<RecipeDef>() else {
        return Vec::new();
    };
    let stations = stations_near(sim, who);
    let mut options = Vec::new();
    for (id, recipe) in registry.iter() {
        let places: Vec<Option<Target>> = if recipe.by_hand() {
            vec![None]
        } else {
            stations
                .iter()
                .copied()
                .filter(|station| {
                    device::def_for(sim, *station)
                        .is_some_and(|def| def.tags.iter().any(|tag| recipe.at.contains(tag)))
                })
                .map(Some)
                .collect()
        };
        for station in places {
            let checked = work::check(sim, Some(who), id, station);
            options.push(RecipeOption {
                recipe: id,
                station,
                resumes: matches!(checked, Ok(Starting::Resume)),
                refusal: checked.err(),
            });
        }
    }
    options
}

/// Where a station is from a character, in a word: `hand`, `here`, a
/// compass direction.
fn where_from(sim: &Sim, who: EntityId, station: Option<Target>) -> &'static str {
    let Some(station) = station else {
        return "hand";
    };
    let (Some(from), Some((to, _))) = (
        sim.entities.location(who),
        crate::world_verbs::position_of(sim, station.subject()),
    ) else {
        return "";
    };
    let Some(delta) = sim.cosmos.delta(from, to) else {
        return "";
    };
    match (delta.x.signum(), delta.y.signum()) {
        (0, 0) => "here",
        (0, -1) => "north",
        (1, -1) => "north-east",
        (1, 0) => "east",
        (1, 1) => "south-east",
        (0, 1) => "south",
        (-1, 1) => "south-west",
        (-1, 0) => "west",
        (-1, -1) => "north-west",
        _ => "",
    }
}

/// What a recipe gives, in words: `2 arrow`, `200 ml ale`.
fn gives(sim: &Sim, recipe: &RecipeDef) -> String {
    let mut parts: Vec<String> = Vec::new();
    for output in &recipe.outputs {
        if let Some(item) = output.item {
            parts.push(format!(
                "{} {}",
                output.count,
                super::item_def_name(sim, item.id())
            ));
        }
        if let (Some(substance), Some(amount)) = (output.substance, output.amount) {
            parts.push(format!(
                "{} {}",
                supply::format_amount(amount.get() * crate::substance::ML_PER_M3, true),
                crate::substance::name_of(sim, substance.id())
            ));
        }
    }
    if let Some(repairs) = &recipe.repairs {
        parts.push(format!("mends something {}", repairs.tag));
    }
    parts.join(", ")
}

/// [`recipe_options`] as the property `recipe_options` gives it: a list
/// of maps.
pub(crate) fn options_value(sim: &Sim, who: EntityId) -> Value {
    let recipe_type = sim
        .exprs
        .registry()
        .def_type_id(<RecipeDef as clash_data::Def>::TYPE);
    let Some(registry) = sim.defs.try_registry::<RecipeDef>() else {
        return Value::list([]);
    };
    let mut rows = Vec::new();
    for option in recipe_options(sim, who) {
        let Some(recipe) = registry.try_get(option.recipe) else {
            continue;
        };
        let device = option
            .station
            .and_then(|station| device::device_now(sim, station));
        let here = option
            .station
            .and_then(|station| crate::world_verbs::position_of(sim, station.subject()))
            .map(|(loc, _)| loc)
            .or_else(|| sim.entities.location(who));
        let items = supply::belief::at_hand(sim, Some(who), device, here);
        let inputs: Vec<Value> = supply::standing(sim, recipe, &items)
            .into_iter()
            .zip(&recipe.inputs)
            .map(|(standing, input)| {
                let millilitres = input.substance.is_some();
                Value::map([
                    ("what", Value::text(&standing.what)),
                    ("need", Value::Real(standing.need)),
                    ("have", Value::Real(standing.have)),
                    (
                        "text",
                        Value::text(format!(
                            "{} {} ({} at hand)",
                            supply::format_amount(standing.need, millilitres),
                            standing.what,
                            supply::format_amount(standing.have, millilitres)
                        )),
                    ),
                ])
            })
            .collect();
        let takes: Vec<String> = supply::standing(sim, recipe, &items)
            .into_iter()
            .zip(&recipe.inputs)
            .map(|(standing, input)| {
                format!(
                    "{} {}",
                    supply::format_amount(standing.need, input.substance.is_some()),
                    standing.what
                )
            })
            .chain(
                recipe
                    .tools
                    .iter()
                    .map(|tool| format!("tool: {}", supply::tool_name(sim, tool))),
            )
            .collect();
        let recipe_value = recipe_type.map_or_else(
            || Value::text(super::recipe_name(sim, option.recipe)),
            |ty| Value::Id(clash_expr::DefRef::new(ty, u32::from(option.recipe.raw()))),
        );
        let station_value = option
            .station
            .map_or(Value::Null, |station| sim.subject_value(station.subject()));
        let why = option
            .refusal
            .as_ref()
            .map_or_else(String::new, |reason| reason.message.clone());
        rows.push(Value::map([
            ("recipe", recipe_value),
            ("id", Value::text(super::recipe_name(sim, option.recipe))),
            ("name", Value::text(super::recipe_label(sim, option.recipe))),
            ("station", station_value),
            ("where", Value::text(where_from(sim, who, option.station))),
            ("available", Value::Bool(option.refusal.is_none())),
            ("resumes", Value::Bool(option.resumes)),
            ("why", Value::text(why)),
            (
                "work",
                Value::Real(recipe.work.0.as_secs_real().max(Real::ZERO)),
            ),
            ("inputs", Value::list(inputs)),
            ("takes", Value::text(takes.join(", "))),
            ("gives", Value::text(gives(sim, recipe))),
        ]));
    }
    Value::list(rows)
}

/// What the character knows how to do, and what it might learn, for the
/// know-how screen (12 §10.7): one row for each `know_how` definition,
/// in order of id, with what each lets it make.
pub(crate) fn know_how_value(sim: &Sim, who: EntityId) -> Value {
    let Some(registry) = sim.defs.try_registry::<super::KnowHowDef>() else {
        return Value::list([]);
    };
    let recipes = sim.defs.try_registry::<RecipeDef>();
    let mut rows = Vec::new();
    for (id, def) in registry.iter() {
        let known = super::know_how::knows(sim, who, id);
        let ready = def
            .needs
            .iter()
            .all(|needed| super::know_how::knows(sim, who, needed.id()));
        let makes: Vec<Value> = recipes
            .into_iter()
            .flat_map(|registry| registry.iter())
            .filter(|(_, recipe)| recipe.know_how.iter().any(|needed| needed.id() == id))
            .map(|(recipe, _)| Value::text(super::recipe_label(sim, recipe)))
            .collect();
        let needs: Vec<Value> = def
            .needs
            .iter()
            .map(|needed| Value::text(super::know_how::know_how_label(sim, needed.id())))
            .collect();
        rows.push(Value::map([
            ("id", Value::text(super::know_how::know_how_name(sim, id))),
            (
                "name",
                Value::text(super::know_how::know_how_label(sim, id)),
            ),
            ("group", Value::text(def.group.clone().unwrap_or_default())),
            ("known", Value::Bool(known)),
            ("ready", Value::Bool(known || ready)),
            ("makes", Value::list(makes)),
            ("needs", Value::list(needs)),
        ]));
    }
    Value::list(rows)
}

/// The station an expression value names: an element or an entity; null
/// for none.
pub(crate) fn station_of(sim: &Sim, value: &Value) -> Option<Target> {
    sim.subject_of(value).and_then(|subject| match subject {
        Subject::Element(..) | Subject::Entity(_) => Target::of(subject),
        _ => None,
    })
}
