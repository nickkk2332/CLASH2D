//! Substances (16 §9.5; package G19, as far as food, drink, medicine and
//! poison need): one stuff, whatever it is in, carried by volume.
//!
//! An item whose definition `contains` a volume holds amounts of any
//! substances as a mixture; one `filled` is made holding them, and a
//! loaf is an item made of what it is filled with. What an item holds,
//! and what is coated on it, is its [`Substances`] component, in cubic
//! metres, for each member of a stack: equal items stack only when they
//! hold the same.
//!
//! | Verb | Does |
//! |------|------|
//! | `pour { from, to, amount? }` | Moves a share of a mixture into another container, or onto a cell, where it is its `loose` field |
//! | `fill { container, from }` | Takes from another container, or from a cell's field that is a substance's `loose` form |
//! | `dose { who, from, route, amount?, substance? }` | Puts a mixture into a body by a route |
//! | `coat { surface, from, amount }` | Puts some of a mixture on an item that strikes |
//! | `take_in { who, what, route }` | Takes one member of an item into a body whole: what it holds goes in, and it is gone |
//!
//! **In a body.** For each substance of what goes in, the route's factor
//! times its millilitres gets in. For each pool it `adds` to, the hook
//! `exposure_incoming` runs with the pool, the amount, the route and the
//! substance (as `field`), and what its handlers leave is added to the
//! pool, stopping at its bounds; a body without that pool takes nothing.
//! Each trait it gives is given for its time where enough got in.
//! `dosed` is raised for each substance; a dose by `ingest` raises
//! `tasted` with the strongest `taste` in it. What the pool then does is
//! the pool's own rates, thresholds and stops (06 §4.2).
//!
//! **Coatings.** A hit with an item that carries a coating, whose blow
//! got through the outermost layer of the body's own part it struck
//! (06 §7.2; what is worn does not count), puts the substance's `share`
//! of each coating in by `inject`. Every hit then wipes `wipes` of what
//! is left off, breaching or not: what is wiped is lost, the one
//! exception to invariant G15.
//!
//! **Not built.** Pipes, tanks and standing liquid in cells (16 §9.5
//! `piped`, `still`, wells): pouring onto a cell adds to its `loose`
//! field, `levels_per_litre` levels to the litre, and the field's own
//! rules take it from there; a field that is a form of a substance does
//! not expose by the substance's routes; pools that name a substance
//! spill nothing.

mod defs;

use clash_core::{EntityId, Real, impl_state_hash};
use clash_data::{Def, DefTypes};
use clash_expr::{EventSpec, Registry, Type, Value};
use clash_world::{Cause, Location};
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::bus::{EventPayload, EventType, FieldSpec, FieldType, FieldValue};
use crate::event::{CAUSE_SUBJECTS, Subject, Where};
use crate::item::{Item, ItemDef};
use crate::reason::{Reason, ReasonInfo};
use crate::registry::PropertyDef;
use crate::verb::VerbDef;
use crate::world_verbs::{ArgRow, arg, spec};

pub use defs::{AddsDef, FormsDef, GivesTraitDef, InBodyDef, ROUTES, SubstanceDef, SubstanceId};

/// Millilitres in a cubic metre: what `per_ml` and `at_least_ml` count
/// in.
pub const ML_PER_M3: Real = Real::of(1.0e6);
/// Litres in a cubic metre.
const L_PER_M3: Real = Real::of(1.0e3);
/// The highest level of a field.
const FIELD_MAX: Real = Real::of(255.0);
/// Less than this, in cubic metres, is nothing: a hundredth of a
/// millilitre.
const TRACE: Real = Real::of(1.0e-8);

/// Registers the `substance` definition type.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<SubstanceDef>();
}

/// Enters the `substance` definition type into an expression registry
/// if it lacks it.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(SubstanceDef::TYPE).is_none() {
        let _ = registry.add_def_type(SubstanceDef::TYPE, SubstanceDef::DOC);
    }
}

// ---- The component -------------------------------------------------------------

/// What an item holds and what is coated on it (16 §9.5): amounts of
/// substances in cubic metres, for each member of its stack, in order
/// of substance.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Substances {
    /// What it holds.
    held: Vec<(SubstanceId, Real)>,
    /// What is on its striking surfaces.
    coat: Vec<(SubstanceId, Real)>,
}

impl_state_hash!(Substances { held, coat });

impl Substances {
    /// What it holds, by substance, in cubic metres.
    pub fn held(&self) -> &[(SubstanceId, Real)] {
        &self.held
    }

    /// What is coated on it, by substance, in cubic metres.
    pub fn coat(&self) -> &[(SubstanceId, Real)] {
        &self.coat
    }

    fn is_empty(&self) -> bool {
        self.held.is_empty() && self.coat.is_empty()
    }
}

/// A mixture: amounts by substance, in order of substance.
type Mixture = Vec<(SubstanceId, Real)>;

fn total(mixture: &[(SubstanceId, Real)]) -> Real {
    mixture
        .iter()
        .fold(Real::ZERO, |sum, (_, volume)| sum + *volume)
}

fn add_into(mixture: &mut Mixture, substance: SubstanceId, volume: Real) {
    if volume <= Real::ZERO {
        return;
    }
    match mixture.binary_search_by_key(&substance, |(id, _)| *id) {
        Ok(at) => {
            if let Some(entry) = mixture.get_mut(at) {
                entry.1 += volume;
            }
        }
        Err(at) => mixture.insert(at, (substance, volume)),
    }
}

fn tidy(mixture: &mut Mixture) {
    mixture.retain(|(_, volume)| *volume > TRACE);
}

// ---- Reasons -------------------------------------------------------------------

/// The stable codes of the reasons substances refuse for.
pub mod codes {
    /// No substance has that id. Values: `substance`.
    pub const UNKNOWN: &str = "substance.unknown";
    /// The thing holds no substances. Values: `entity`.
    pub const NOT_A_CONTAINER: &str = "substance.not_a_container";
    /// There is nothing in it to move. Values: `entity`.
    pub const EMPTY: &str = "substance.empty";
    /// There is no room for it. Values: `entity`, `room`.
    pub const NO_ROOM: &str = "substance.no_room";
    /// An amount is not finite and above zero. Values: `amount`.
    pub const BAD_AMOUNT: &str = "substance.bad_amount";
    /// The route is not one of `ingest`, `inject`, `skin`, `breath`.
    /// Values: `route`.
    pub const BAD_ROUTE: &str = "substance.bad_route";
    /// A verb's argument is missing or of the wrong kind. Values:
    /// `verb`, `arg`.
    pub const BAD_ARGUMENT: &str = "substance.bad_argument";
    /// The definitions were loaded without the `substance` type.
    pub const NOT_REGISTERED: &str = "substance.not_registered";
}

/// The reasons substances refuse for, for the generated reference.
pub const SUBSTANCE_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::UNKNOWN,
        doc: "The id is not a `substance` definition. Values: `substance`.",
    },
    ReasonInfo {
        code: codes::NOT_A_CONTAINER,
        doc: "What was to be poured from or into, filled or coated cannot hold substances: it has no `contains`, or is not an item. Values: `entity`.",
    },
    ReasonInfo {
        code: codes::EMPTY,
        doc: "There is nothing to pour, dose, coat or fill from. Values: `entity`.",
    },
    ReasonInfo {
        code: codes::NO_ROOM,
        doc: "The container is full. Values: `entity`, `room` (in cubic metres).",
    },
    ReasonInfo {
        code: codes::BAD_AMOUNT,
        doc: "An amount is not finite and above zero. Values: `amount`.",
    },
    ReasonInfo {
        code: codes::BAD_ROUTE,
        doc: "A route is not one of `ingest`, `inject`, `skin` and `breath`. Values: `route`.",
    },
    ReasonInfo {
        code: codes::BAD_ARGUMENT,
        doc: "A verb of substances was given an argument that is missing or of the wrong kind. Values: `verb`, `arg`.",
    },
    ReasonInfo {
        code: codes::NOT_REGISTERED,
        doc: "The definitions were loaded without the `substance` definition type. Values: none.",
    },
];

fn refuse(code: &'static str, message: impl Into<String>) -> Reason {
    Reason::new(code, message)
}

fn bad_argument(verb: &str, name: &str, wanted: &str) -> Reason {
    refuse(
        codes::BAD_ARGUMENT,
        format!("`{verb}`: the argument `{name}` must be {wanted}"),
    )
    .with("verb", verb)
    .with("arg", name)
}

// ---- Definitions ---------------------------------------------------------------

fn registry(sim: &Sim) -> Option<&clash_data::Registry<SubstanceDef>> {
    sim.defs.try_registry::<SubstanceDef>()
}

/// A substance's definition.
pub fn def(sim: &Sim, id: SubstanceId) -> Option<&SubstanceDef> {
    registry(sim)?.try_get(id)
}

/// A substance's qualified id.
pub fn qualified(sim: &Sim, id: SubstanceId) -> String {
    registry(sim).map_or_else(String::new, |registry| registry.name(id).to_owned())
}

/// What a reader calls a substance: its `name`, else its id.
pub fn name_of(sim: &Sim, id: SubstanceId) -> String {
    let name = qualified(sim, id);
    def(sim, id).map_or(name.clone(), |def| {
        crate::recipe::label(sim, def.name.as_ref(), &name)
    })
}

/// The substance a value names: an id of the type, or its qualified id.
pub fn substance_of(sim: &Sim, value: &Value) -> Option<SubstanceId> {
    match value {
        Value::Id(id) if sim.exprs.registry().def_type_id(SubstanceDef::TYPE) == Some(id.ty) => {
            u16::try_from(id.index).ok().map(SubstanceId::new)
        }
        Value::Str(name) => registry(sim)?.lookup(name),
        _ => None,
    }
}

// ---- Containers ----------------------------------------------------------------

fn item_def(sim: &Sim, item: EntityId) -> Option<&ItemDef> {
    sim.components
        .get::<Item>(item)
        .and_then(|it| crate::item::def_of(&sim.defs, it))
}

/// The volume one member of an item may hold, in cubic metres: its
/// `contains`, or for one `filled` and not a container, what it was
/// filled with. `None` for what holds no substances.
pub fn capacity(sim: &Sim, item: EntityId) -> Option<Real> {
    let def = item_def(sim, item)?;
    match def.contains {
        Some(volume) => Some(volume.get()),
        None if !def.filled.is_empty() => Some(
            def.filled
                .values()
                .fold(Real::ZERO, |sum, volume| sum + volume.get()),
        ),
        None => None,
    }
}

/// What one member of an item holds, by substance.
pub fn held(sim: &Sim, item: EntityId) -> Mixture {
    sim.components
        .get::<Substances>(item)
        .map_or_else(Vec::new, |found| found.held.clone())
}

/// How much of a substance one member of an item holds, in cubic metres.
pub fn amount_of(sim: &Sim, item: EntityId, substance: SubstanceId) -> Real {
    sim.components
        .get::<Substances>(item)
        .and_then(|found| found.held.iter().find(|(id, _)| *id == substance))
        .map_or(Real::ZERO, |(_, volume)| *volume)
}

/// How much one member of an item holds in all, in cubic metres.
pub fn fill_of(sim: &Sim, item: EntityId) -> Real {
    total(&held(sim, item))
}

/// The room left in one member of an item, in cubic metres.
pub fn room_in(sim: &Sim, item: EntityId) -> Option<Real> {
    capacity(sim, item).map(|capacity| (capacity - fill_of(sim, item)).max(Real::ZERO))
}

fn store(sim: &mut Sim, item: EntityId, held: Mixture, coat: Option<Mixture>) {
    let mut found = sim
        .components
        .get::<Substances>(item)
        .cloned()
        .unwrap_or_default();
    found.held = held;
    tidy(&mut found.held);
    if let Some(mut coat) = coat {
        tidy(&mut coat);
        found.coat = coat;
    }
    if found.is_empty() {
        sim.components.remove::<Substances>(item);
    } else {
        sim.components.insert(item, found);
    }
}

/// A single member of an item to work on: the item itself if it is one,
/// else one split off its stack.
fn single(sim: &mut Sim, item: EntityId, cause: Cause) -> Result<EntityId, Reason> {
    let count = sim.components.get::<Item>(item).map_or(1, Item::count);
    if count > 1 {
        crate::item::split(sim, item, 1, cause)
    } else {
        Ok(item)
    }
}

/// Takes a volume of a mixture out of one member of an item, each
/// substance in its share. Returns what was taken.
fn take_share(sim: &mut Sim, item: EntityId, volume: Real) -> Mixture {
    let mut mixture = held(sim, item);
    let whole = total(&mixture);
    if whole <= Real::ZERO || volume <= Real::ZERO {
        return Vec::new();
    }
    let share = (volume / whole).min(Real::ONE);
    let mut taken = Vec::new();
    for (id, amount) in &mut mixture {
        let part = *amount * share;
        *amount -= part;
        taken.push((*id, part));
    }
    store(sim, item, mixture, None);
    tidy(&mut taken);
    taken
}

/// Takes an amount of one substance out of one member of an item: what
/// a recipe draws. Returns what was taken.
pub(crate) fn draw(sim: &mut Sim, item: EntityId, substance: SubstanceId, volume: Real) -> Real {
    let mut mixture = held(sim, item);
    let Some(entry) = mixture.iter_mut().find(|(id, _)| *id == substance) else {
        return Real::ZERO;
    };
    let taken = entry.1.min(volume).max(Real::ZERO);
    entry.1 -= taken;
    store(sim, item, mixture, None);
    taken
}

/// Puts a mixture into one member of an item, as far as there is room.
/// Returns what went in.
pub(crate) fn put(sim: &mut Sim, item: EntityId, mixture: &[(SubstanceId, Real)]) -> Mixture {
    let Some(room) = room_in(sim, item) else {
        return Vec::new();
    };
    let whole = total(mixture);
    if whole <= Real::ZERO {
        return Vec::new();
    }
    let share = (room / whole).min(Real::ONE);
    let mut have = held(sim, item);
    let mut went = Vec::new();
    for (id, volume) in mixture {
        let part = *volume * share;
        add_into(&mut have, *id, part);
        went.push((*id, part));
    }
    store(sim, item, have, None);
    went
}

/// Fills a new item as its definition says (16 §9.5 `filled`). Called
/// when an item is made.
pub(crate) fn on_made(sim: &mut Sim, item: EntityId) {
    let Some(def) = item_def(sim, item) else {
        return;
    };
    if def.filled.is_empty() {
        return;
    }
    let mut mixture = Vec::new();
    for (substance, volume) in &def.filled {
        add_into(&mut mixture, substance.id(), volume.get());
    }
    store(sim, item, mixture, None);
}

// ---- Moving it -----------------------------------------------------------------

fn container_arg(sim: &Sim, item: EntityId) -> Result<(), Reason> {
    if capacity(sim, item).is_none() {
        return Err(
            refuse(codes::NOT_A_CONTAINER, "that cannot hold anything poured")
                .with("entity", item.to_string()),
        );
    }
    Ok(())
}

fn amount_ok(amount: Option<Real>) -> Result<Option<Real>, Reason> {
    match amount {
        Some(amount) if !(amount.is_finite() && amount > Real::ZERO) => Err(refuse(
            codes::BAD_AMOUNT,
            format!("{amount} is not an amount that can be moved"),
        )
        .with("amount", amount)),
        other => Ok(other),
    }
}

/// Where poured stuff goes: another container or a cell.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PourTo {
    /// A container.
    Item(EntityId),
    /// A cell, where it is its `loose` field.
    Cell(Location),
}

/// Pours a share of what one member of an item holds (16 §9.5 `pour`):
/// all of it with no amount. Into a container it goes as far as there
/// is room; onto a cell, each substance with a `loose` form adds to that
/// field `levels_per_litre` levels to the litre, and the rest soaks
/// away. Raises `poured`. Returns the volume that left the container.
pub fn pour(
    sim: &mut Sim,
    from: EntityId,
    to: PourTo,
    amount: Option<Real>,
    cause: Cause,
) -> Result<Real, Reason> {
    container_arg(sim, from)?;
    let amount = amount_ok(amount)?;
    let from = single(sim, from, cause)?;
    let have = fill_of(sim, from);
    if have <= Real::ZERO {
        return Err(refuse(codes::EMPTY, "there is nothing in it").with("entity", from.to_string()));
    }
    let volume = amount.unwrap_or(have).min(have);
    let moved = match to {
        PourTo::Item(into) => {
            container_arg(sim, into)?;
            let room = room_in(sim, into).unwrap_or(Real::ZERO);
            if room <= Real::ZERO {
                return Err(refuse(codes::NO_ROOM, "it is full")
                    .with("entity", into.to_string())
                    .with("room", room));
            }
            let into = single(sim, into, cause)?;
            let taken = take_share(sim, from, volume.min(room));
            let went = put(sim, into, &taken);
            total(&went)
        }
        PourTo::Cell(loc) => {
            let taken = take_share(sim, from, volume);
            for (id, part) in &taken {
                spill(sim, loc, *id, *part, cause);
            }
            total(&taken)
        }
    };
    let payload = EventPayload::new(POURED)
        .subject("self", Subject::Entity(from))
        .with("amount", moved)
        .with(
            "onto",
            match to {
                PourTo::Item(into) => FieldValue::Subject(Subject::Entity(into)),
                PourTo::Cell(loc) => FieldValue::Subject(Subject::Loc(loc)),
            },
        );
    let _ = sim.raise(Where::Entity(from), cause, payload);
    Ok(moved)
}

/// Spills a volume of a substance onto a cell: its `loose` field gains
/// `levels_per_litre` levels to the litre.
fn spill(sim: &mut Sim, loc: Location, id: SubstanceId, volume: Real, cause: Cause) {
    let Some(def) = def(sim, id) else {
        return;
    };
    let Some(field_name) = def.forms.loose.clone() else {
        return;
    };
    let owner = qualified(sim, id);
    let namespace = owner.split_once(':').map(|(namespace, _)| namespace);
    let Some(field) = sim.cosmos.defs().fields().resolve(&field_name, namespace) else {
        return;
    };
    let levels = (volume * L_PER_M3 * def.levels_per_litre)
        .min(FIELD_MAX)
        .max(Real::ZERO);
    let levels = i16::try_from(levels.floor().to_i64()).unwrap_or(0);
    if levels > 0 {
        let _ = sim.world(|ctx| ctx.edit(cause).add_field(loc, field, levels));
    }
}

/// Fills one member of an item (16 §9.5 `fill`): from another container,
/// as `pour` from it would; or from a cell, taking up to the room left
/// of each substance whose `loose` field is there, a level for every
/// `1 / levels_per_litre` litres. Returns the volume that went in.
pub fn fill(
    sim: &mut Sim,
    container: EntityId,
    from: Subject,
    cause: Cause,
) -> Result<Real, Reason> {
    container_arg(sim, container)?;
    match from {
        Subject::Entity(source) => pour(sim, source, PourTo::Item(container), None, cause),
        Subject::Loc(loc) | Subject::Element(loc, _) => {
            let container = single(sim, container, cause)?;
            let mut room = room_in(sim, container).unwrap_or(Real::ZERO);
            if room <= Real::ZERO {
                return Err(refuse(codes::NO_ROOM, "it is full")
                    .with("entity", container.to_string())
                    .with("room", room));
            }
            let mut went = Real::ZERO;
            let ids: Vec<SubstanceId> = registry(sim)
                .map(|registry| registry.iter().map(|(id, _)| id).collect())
                .unwrap_or_default();
            for id in ids {
                let Some(def) = def(sim, id) else {
                    continue;
                };
                let (Some(field_name), per_litre) = (def.forms.loose.clone(), def.levels_per_litre)
                else {
                    continue;
                };
                if per_litre <= Real::ZERO {
                    continue;
                }
                let owner = qualified(sim, id);
                let namespace = owner.split_once(':').map(|(namespace, _)| namespace);
                let Some(field) = sim.cosmos.defs().fields().resolve(&field_name, namespace) else {
                    continue;
                };
                let level = sim.cosmos.field(loc, field).unwrap_or(0);
                if level == 0 || room <= Real::ZERO {
                    continue;
                }
                let available = Real::from_i64(i64::from(level)) / per_litre / L_PER_M3;
                let volume = available.min(room);
                let levels =
                    i16::try_from((volume * L_PER_M3 * per_litre).floor().to_i64()).unwrap_or(0);
                if levels <= 0 {
                    continue;
                }
                let volume = Real::from_i64(i64::from(levels)) / per_litre / L_PER_M3;
                let _ = sim.world(|ctx| ctx.edit(cause).add_field(loc, field, -levels));
                put(sim, container, &[(id, volume)]);
                room -= volume;
                went += volume;
            }
            if went <= Real::ZERO {
                return Err(
                    refuse(codes::EMPTY, "there is nothing here to fill it with")
                        .with("entity", container.to_string()),
                );
            }
            Ok(went)
        }
        _ => Err(bad_argument("fill", "from", "a container or a cell")),
    }
}

/// Puts some of a mixture on an item that strikes (16 §9.5 `coat`).
/// Returns what went on.
pub fn coat(
    sim: &mut Sim,
    surface: EntityId,
    from: EntityId,
    amount: Real,
    cause: Cause,
) -> Result<Real, Reason> {
    container_arg(sim, from)?;
    amount_ok(Some(amount))?;
    if sim.components.get::<Item>(surface).is_none() {
        return Err(bad_argument("coat", "surface", "an item"));
    }
    let from = single(sim, from, cause)?;
    if fill_of(sim, from) <= Real::ZERO {
        return Err(refuse(codes::EMPTY, "there is nothing in it").with("entity", from.to_string()));
    }
    let surface = single(sim, surface, cause)?;
    let taken = take_share(sim, from, amount);
    let mut coating = sim
        .components
        .get::<Substances>(surface)
        .map_or_else(Vec::new, |found| found.coat.clone());
    for (id, volume) in &taken {
        add_into(&mut coating, *id, *volume);
    }
    let kept = held(sim, surface);
    store(sim, surface, kept, Some(coating));
    Ok(total(&taken))
}

/// What is coated on an item, by substance.
pub fn coating_of(sim: &Sim, item: EntityId) -> Mixture {
    sim.components
        .get::<Substances>(item)
        .map_or_else(Vec::new, |found| found.coat.clone())
}

// ---- Into a body ---------------------------------------------------------------

fn route_ok(route: &str) -> Result<(), Reason> {
    if ROUTES.contains(&route) {
        Ok(())
    } else {
        Err(refuse(
            codes::BAD_ROUTE,
            format!("`{route}` is not a way into a body"),
        )
        .with("route", route))
    }
}

/// Puts a mixture into a body by a route (16 §9.5): see the module
/// documentation. Returns what got in, in millilitres, all substances
/// together.
pub fn dose_body(
    sim: &mut Sim,
    who: EntityId,
    mixture: &[(SubstanceId, Real)],
    route: &str,
    cause: Cause,
) -> Result<Real, Reason> {
    route_ok(route)?;
    sim.known(who)?;
    let mut got_in = Real::ZERO;
    let mut strongest: Option<(Real, SubstanceId)> = None;
    for (id, volume) in mixture {
        let Some(def) = def(sim, *id) else {
            continue;
        };
        let factor = def.in_body.routes.get(route).copied().unwrap_or(Real::ZERO);
        let ml = *volume * ML_PER_M3 * factor;
        if def.taste > Real::ZERO && strongest.is_none_or(|(taste, _)| def.taste > taste) {
            strongest = Some((def.taste, *id));
        }
        let adds: Vec<(crate::stats::PoolId, Real)> = def
            .in_body
            .adds
            .iter()
            .map(|adds| (adds.pool.id(), adds.per_ml * ml))
            .collect();
        let traits: Vec<(String, Real, Option<clash_core::Dur>)> = def
            .in_body
            .traits
            .iter()
            .map(|given| {
                (
                    given.given.clone(),
                    given.at_least_ml,
                    given.lasts.map(|lasts| lasts.0),
                )
            })
            .collect();
        let name = qualified(sim, *id);
        if ml > Real::ZERO {
            for (pool, amount) in adds {
                dose_pool(sim, who, pool, amount, route, &name, cause)?;
            }
            for (written, at_least, lasts) in traits {
                if ml < at_least {
                    continue;
                }
                let found = crate::traits::lookup(&sim.defs, &written).or_else(|| {
                    name.split_once(':').and_then(|(namespace, _)| {
                        crate::traits::lookup(&sim.defs, &format!("{namespace}:{written}"))
                    })
                });
                if let Some(trait_id) = found {
                    let _ = crate::traits::add_trait(
                        sim,
                        who,
                        trait_id,
                        crate::traits::Giver::RULE,
                        lasts,
                        cause,
                    );
                }
            }
        }
        got_in += ml.max(Real::ZERO);
        let payload = EventPayload::new(DOSED)
            .subject("self", Subject::Entity(who))
            .with("substance", FieldValue::text(&name))
            .with("amount", ml.max(Real::ZERO))
            .with("route", FieldValue::text(route));
        let _ = sim.raise(Where::Entity(who), cause, payload);
    }
    if route == "ingest"
        && let Some((taste, id)) = strongest
    {
        let payload = EventPayload::new(TASTED)
            .subject("self", Subject::Entity(who))
            .with("substance", FieldValue::text(qualified(sim, id)))
            .with("taste", taste);
        let _ = sim.raise(Where::Entity(who), cause, payload);
    }
    Ok(got_in)
}

/// Adds a dose to a pool after the hook `exposure_incoming`, as the
/// world's doses are (06 §4.2): nothing for a body without the pool.
fn dose_pool(
    sim: &mut Sim,
    who: EntityId,
    pool: crate::stats::PoolId,
    amount: Real,
    route: &str,
    substance: &str,
    cause: Cause,
) -> Result<Real, Reason> {
    let has = sim
        .components
        .stats
        .get(who)
        .is_some_and(|stats| stats.has_pool(pool));
    if !has || !amount.is_finite() || amount == Real::ZERO {
        return Ok(Real::ZERO);
    }
    let defs = sim.life.stat_defs()?;
    let here = sim.entities.location(who);
    let mut payload = crate::life::events::about("exposure_incoming", who)
        .with(
            "pool",
            crate::life::events::id_field(Some(defs.def_types().pool), pool.index()),
        )
        .with("amount", amount)
        .with("route", FieldValue::text(route))
        .with("field", FieldValue::text(substance));
    if let Some(here) = here {
        payload = payload.subject("here", Subject::Loc(here));
    }
    let raised = sim.raise(Where::Entity(who), cause, payload)?;
    if raised.cancelled.is_some() {
        return Ok(Real::ZERO);
    }
    let amount = raised
        .field("amount")
        .and_then(FieldValue::as_real)
        .filter(|amount| amount.is_finite())
        .unwrap_or(amount);
    sim.dose_pool(who, pool, amount, cause)
}

/// Doses a body from one member of an item (16 §9.5 `dose`): the amount
/// (all with none) of its mixture goes in by the route. Returns what got
/// in, in millilitres.
pub fn dose_from(
    sim: &mut Sim,
    who: EntityId,
    from: EntityId,
    amount: Option<Real>,
    route: &str,
    cause: Cause,
) -> Result<Real, Reason> {
    route_ok(route)?;
    container_arg(sim, from)?;
    let amount = amount_ok(amount)?;
    let from = single(sim, from, cause)?;
    let have = fill_of(sim, from);
    if have <= Real::ZERO {
        return Err(refuse(codes::EMPTY, "there is nothing in it").with("entity", from.to_string()));
    }
    let taken = take_share(sim, from, amount.unwrap_or(have).min(have));
    dose_body(sim, who, &taken, route, cause)
}

/// Takes one member of an item into a body whole (`take_in`): what it
/// holds goes in by the route, and the member is gone. Returns what got
/// in, in millilitres.
pub fn take_in(
    sim: &mut Sim,
    who: EntityId,
    what: EntityId,
    route: &str,
    cause: Cause,
) -> Result<Real, Reason> {
    route_ok(route)?;
    if sim.components.get::<Item>(what).is_none() {
        return Err(bad_argument("take_in", "what", "an item"));
    }
    let one = single(sim, what, cause)?;
    let mixture = held(sim, one);
    crate::item::destroy(sim, one, cause)?;
    dose_body(sim, who, &mixture, route, cause)
}

/// What a hit with a coated item does (16 §9.5): see the module
/// documentation. The combat package calls it for each hit an item
/// lands on a body.
pub fn after_hit(
    sim: &mut Sim,
    item: EntityId,
    target: EntityId,
    outcome: &crate::body::HitOutcome,
    cause: Cause,
) {
    let coating = coating_of(sim, item);
    if coating.is_empty() || outcome.cancelled.is_some() {
        return;
    }
    let breached = outcome
        .report
        .struck
        .first()
        .is_some_and(|part| part.depth >= 1);
    let mut delivered = Vec::new();
    let mut left = Vec::new();
    for (id, volume) in coating {
        let (share, wipes) =
            def(sim, id).map_or((Real::ZERO, Real::ONE), |def| (def.share, def.wipes));
        let goes = if breached { volume * share } else { Real::ZERO };
        if goes > Real::ZERO {
            delivered.push((id, goes));
        }
        let rest = volume - goes;
        left.push((id, rest - rest * wipes));
    }
    let kept = held(sim, item);
    store(sim, item, kept, Some(left));
    if !delivered.is_empty()
        && let Err(reason) = dose_body(sim, target, &delivered, "inject", cause)
    {
        sim.fault(reason);
    }
}

// ---- Events --------------------------------------------------------------------

/// Something was poured.
pub const POURED: &str = "poured";
/// A body took a substance in.
pub const DOSED: &str = "dosed";
/// Something swallowed tasted of something.
pub const TASTED: &str = "tasted";

struct EventRow {
    name: &'static str,
    doc: &'static str,
    subjects: &'static [(&'static str, &'static str)],
    fields: &'static [(&'static str, &'static str, &'static str)],
}

const EVENTS: &[EventRow] = &[
    EventRow {
        name: POURED,
        doc: "Something was poured out of a container (16 §9.5): into another, or onto a cell. `self` is what it came out of.",
        subjects: &[("self", "entity")],
        fields: &[
            (
                "amount",
                "real",
                "The volume that left it, in cubic metres.",
            ),
            ("onto", "any", "Where it went: the container, or the cell."),
        ],
    },
    EventRow {
        name: DOSED,
        doc: "A body took a substance in by a route (16 §9.5): once for each substance of a dose, after its pools and traits.",
        subjects: &[("self", "entity")],
        fields: &[
            ("substance", "string", "The substance's qualified id."),
            (
                "amount",
                "real",
                "What got in, in millilitres: its volume times the route's factor.",
            ),
            ("route", "string", "`ingest`, `inject`, `skin` or `breath`."),
        ],
    },
    EventRow {
        name: TASTED,
        doc: "Something swallowed tasted of something (16 §9.5): the substance with the strongest `taste` in what was taken in by `ingest`, for rules to notice.",
        subjects: &[("self", "entity")],
        fields: &[
            ("substance", "string", "The substance's qualified id."),
            ("taste", "real", "How strongly, from 0 to 1."),
        ],
    },
];

/// The event types of this module, for the simulation's bus.
pub fn event_types() -> Vec<EventType> {
    EVENTS
        .iter()
        .map(|row| {
            let mut ty = EventType::new(row.name).doc(row.doc);
            for (name, kind) in row.subjects {
                ty = ty.subject(*name, *kind);
            }
            ty.fields = row
                .fields
                .iter()
                .map(|(name, written, doc)| FieldSpec {
                    name: (*name).to_owned(),
                    ty: (*written).to_owned(),
                    check: Some(FieldType::parse(written).unwrap_or(FieldType::Any)),
                    doc: (*doc).to_owned(),
                })
                .collect();
            ty
        })
        .collect()
}

/// Enters this module's events into an expression registry.
pub fn register_events(registry: &mut Registry) {
    for row in EVENTS {
        if registry.event_id(row.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(row.name).doc(row.doc);
        for (name, kind) in row.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in row.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        let _ = registry.add_event(spec);
    }
}

// ---- Properties and verbs ------------------------------------------------------

fn mixture_value(sim: &Sim, mixture: &[(SubstanceId, Real)]) -> Value {
    Value::map(
        mixture
            .iter()
            .map(|(id, volume)| (qualified(sim, *id), Value::Real(*volume))),
    )
}

/// The properties of substances (16 §9.5, §14). None is safe for belief;
/// a character reads what it holds of its own.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "mixture",
            "map<real>",
            "What one member of an item holds (16 §9.5): the volume of each substance in cubic metres, by qualified id. Empty for one that holds nothing.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => mixture_value(sim, &held(sim, id)),
                _ => Value::map(Vec::<(String, Value)>::new()),
            },
        )
        .own(),
        PropertyDef::new(
            "entity",
            "holds_substance",
            "real",
            "How much of a substance one member of an item holds, in cubic metres.",
            |sim, subject, args| match (subject, args.first().and_then(|value| substance_of(sim, value))) {
                (Subject::Entity(id), Some(substance)) => Value::Real(amount_of(sim, id, substance)),
                _ => Value::Real(Real::ZERO),
            },
        )
        .arg("substance", "id<substance>", "The substance."),
        PropertyDef::new(
            "entity",
            "fill",
            "real",
            "How much one member of an item holds in all, in cubic metres.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => Value::Real(fill_of(sim, id)),
                _ => Value::Real(Real::ZERO),
            },
        ),
        PropertyDef::new(
            "entity",
            "room",
            "real?",
            "The room left in one member of an item for substances, in cubic metres. Null for what holds none.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => room_in(sim, id).map_or(Value::Null, Value::Real),
                _ => Value::Null,
            },
        ),
        PropertyDef::new(
            "entity",
            "coating",
            "map<real>",
            "What is coated on an item's striking surfaces (16 §9.5 `coat`): the volume of each substance in cubic metres, by qualified id.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => mixture_value(sim, &coating_of(sim, id)),
                _ => Value::map(Vec::<(String, Value)>::new()),
            },
        )
        .own(),
    ]
}

fn entity(
    sim: &Sim,
    verb: &str,
    call: &crate::rule::VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<EntityId, Reason> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Entity(id)) => Ok(id),
        _ => Err(bad_argument(verb, name, "an entity")),
    }
}

fn optional_volume(
    verb: &str,
    call: &crate::rule::VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Option<Real>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        value => value
            .as_number()
            .map(Some)
            .ok_or_else(|| bad_argument(verb, name, "a volume")),
    }
}

fn route_arg(verb: &str, call: &crate::rule::VerbCall<'_>, index: usize) -> Result<String, Reason> {
    arg(call, index)
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| bad_argument(verb, "route", "a route"))
}

const ROUTE_DOC: &str = "How it gets in: `ingest`, `inject`, `skin` or `breath`.";

/// The verbs of substances, for [`crate::verb::engine_verbs`].
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "pour",
            build: |registry| {
                spec(
                    registry,
                    "pour",
                    "Pours a share of what a container holds (16 §9.5): into another container as far as there is room, or onto a cell, where each substance with a `loose` form adds `levels_per_litre` levels a litre to that field. One member of a stack is split off first. Raises `poured`; gives the volume that left it. Refused with `substance.not_a_container`, `substance.empty`, `substance.no_room` and `substance.bad_amount`.",
                    &[
                        ("from", "entity", false, "The container poured from."),
                        ("to", "any", false, "Another container, or a cell."),
                        (
                            "amount",
                            "real",
                            true,
                            "How much, in cubic metres (`\"100 ml\"`). Left out, all of it.",
                        ),
                    ] as &[ArgRow],
                )
            },
            run: |sim, call, cause| {
                let from = entity(sim, "pour", call, 0, "from")?;
                let to = match sim.subject_of(arg(call, 1)) {
                    Some(Subject::Entity(id)) => PourTo::Item(id),
                    Some(Subject::Loc(loc) | Subject::Element(loc, _)) => PourTo::Cell(loc),
                    _ => return Err(bad_argument("pour", "to", "a container or a cell")),
                };
                let amount = optional_volume("pour", call, 2, "amount")?;
                pour(sim, from, to, amount, cause).map(Value::Real)
            },
        },
        VerbDef {
            name: "fill",
            build: |registry| {
                spec(
                    registry,
                    "fill",
                    "Fills a container (16 §9.5): from another container, as `pour` from it would, or from a cell, taking up each substance whose `loose` field is there, a level for every `1 / levels_per_litre` litres, as far as there is room. Gives the volume that went in. Refused with `substance.not_a_container`, `substance.no_room` and `substance.empty`.",
                    &[
                        ("container", "entity", false, "What is filled."),
                        ("from", "any", false, "Another container, or a cell."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let container = entity(sim, "fill", call, 0, "container")?;
                let from = sim
                    .subject_of(arg(call, 1))
                    .ok_or_else(|| bad_argument("fill", "from", "a container or a cell"))?;
                fill(sim, container, from, cause).map(Value::Real)
            },
        },
        VerbDef {
            name: "dose",
            build: |registry| {
                spec(
                    registry,
                    "dose",
                    "Puts something into a body by a route (16 §9.5): an amount of what a container holds (all of it with none), or with `substance` an amount of that substance from nowhere, for a sting or a spell. Each substance adds to the pools it names, after the hook `exposure_incoming`, and gives its traits; `dosed` is raised for each, and `tasted` for what is swallowed. Gives what got in, in millilitres. Refused with `substance.not_a_container`, `substance.empty`, `substance.bad_route` and `substance.bad_amount`.",
                    &[
                        ("who", "entity", false, "The body."),
                        ("from", "entity", true, "The container it comes from."),
                        ("route", "string", false, ROUTE_DOC),
                        (
                            "amount",
                            "real",
                            true,
                            "How much, in cubic metres. Left out, all the container holds.",
                        ),
                        (
                            "substance",
                            "id<substance>",
                            true,
                            "Instead of `from`: a substance, of which `amount` goes in.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, "dose", call, 0, "who")?;
                let route = route_arg("dose", call, 2)?;
                let amount = optional_volume("dose", call, 3, "amount")?;
                match substance_of(sim, arg(call, 4)) {
                    Some(substance) => {
                        let amount = amount_ok(amount)?.ok_or_else(|| {
                            bad_argument("dose", "amount", "a volume, with `substance`")
                        })?;
                        dose_body(sim, who, &[(substance, amount)], &route, cause).map(Value::Real)
                    }
                    None => {
                        let from = entity(sim, "dose", call, 1, "from")?;
                        dose_from(sim, who, from, amount, &route, cause).map(Value::Real)
                    }
                }
            },
        },
        VerbDef {
            name: "coat",
            build: |registry| {
                spec(
                    registry,
                    "coat",
                    "Puts some of what a container holds on an item that strikes (16 §9.5 `coat`): a poisoned blade. A hit with it that gets through the outermost layer of the body's own part puts the substance's `share` in by `inject`, and every hit wipes `wipes` of what is left off. Gives what went on. Refused with `substance.not_a_container`, `substance.empty` and `substance.bad_amount`.",
                    &[
                        ("surface", "entity", false, "The item coated."),
                        ("from", "entity", false, "The container it comes from."),
                        ("amount", "real", false, "How much, in cubic metres."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let surface = entity(sim, "coat", call, 0, "surface")?;
                let from = entity(sim, "coat", call, 1, "from")?;
                let amount = optional_volume("coat", call, 2, "amount")?
                    .ok_or_else(|| bad_argument("coat", "amount", "a volume"))?;
                coat(sim, surface, from, amount, cause).map(Value::Real)
            },
        },
        VerbDef {
            name: "take_in",
            build: |registry| {
                spec(
                    registry,
                    "take_in",
                    "Takes one member of an item into a body whole: eating a loaf, swallowing a pill (16 §9.5). What it holds goes in by the route, as `dose` puts it, and the member is gone. Gives what got in, in millilitres. Refused with `substance.bad_route` and `substance.bad_argument`.",
                    &[
                        ("who", "entity", false, "The body."),
                        (
                            "what",
                            "entity",
                            false,
                            "The item; one member of a stack is taken.",
                        ),
                        ("route", "string", false, ROUTE_DOC),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, "take_in", call, 0, "who")?;
                let what = entity(sim, "take_in", call, 1, "what")?;
                let route = route_arg("take_in", call, 2)?;
                take_in(sim, who, what, &route, cause).map(Value::Real)
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut codes: Vec<&str> = SUBSTANCE_REASONS.iter().map(|info| info.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn a_mixture_is_shared_out_in_proportion() {
        let mut mixture = Vec::new();
        add_into(&mut mixture, SubstanceId::new(1), Real::of(2.0));
        add_into(&mut mixture, SubstanceId::new(0), Real::of(1.0));
        add_into(&mut mixture, SubstanceId::new(1), Real::of(1.0));
        assert_eq!(
            mixture,
            vec![
                (SubstanceId::new(0), Real::of(1.0)),
                (SubstanceId::new(1), Real::of(3.0))
            ]
        );
        assert_eq!(total(&mixture), Real::of(4.0));
    }
}
