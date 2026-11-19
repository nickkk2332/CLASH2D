//! Making and unmaking items (06 §9.1).
//!
//! An item is an entity of the kind `item` with an [`Item`] component,
//! an `Identity` naming its definition, the tags of its definition as
//! its own, and a `Physical` of one member's mass and volume times its
//! count. It lies flat: its height is zero, and the area it presents is
//! its volume over its length.

use std::sync::Arc;

use clash_core::{EntityId, Real, Rng};
use clash_data::{Def, Length, Mass, Volume};
use clash_world::{Cause, Footprint, OccupantKind};

use super::{ColorChoice, Item, ItemDef, ItemId, hold, item_of, reason, refuse};
use crate::Sim;
use crate::component::{DefKey, Identity, Physical, Shape};
use crate::entity::{EntitySpec, Place};
use crate::reason::Reason;

/// The random stream items draw their seeds from (01 §4).
pub const ITEM_STREAM: &str = "items";

/// What an item's seed is mixed with before its colour is picked, so
/// that the colour and the latent contents are drawn apart.
const COLOR_SALT: u64 = 0x636f_6c6f_7572_0001;

/// The colour an item of a definition carries, picked by its seed.
fn pick_color(choice: &ColorChoice, seed: u64) -> Option<String> {
    match choice {
        ColorChoice::None => None,
        ColorChoice::One(name) => Some(name.clone()),
        ColorChoice::AnyOf(names) => Rng::from_seed(seed ^ COLOR_SALT).pick(names).cloned(),
    }
}

/// The space one member of a definition takes up, in cubic metres: what
/// the definition says, or its mass over its material's density.
pub(crate) fn unit_volume(sim: &Sim, def: &ItemDef) -> Real {
    if let Some(volume) = def.volume {
        return volume.get();
    }
    let density = sim
        .cosmos
        .defs()
        .materials()
        .get(def.material.id())
        .map_or(Real::ZERO, |material| material.density);
    if density > Real::ZERO {
        def.mass.get() / density
    } else {
        Real::ZERO
    }
}

/// What `count` members of a definition measure together.
pub(crate) fn shape_of(sim: &Sim, def: &ItemDef, count: u32) -> Shape {
    let count = Real::from_i64(i64::from(count));
    let volume = unit_volume(sim, def) * count;
    let length = def.length.get();
    Shape {
        mass: Mass::new(def.mass.get() * count),
        volume: Volume::new(volume),
        height: Length::new(Real::ZERO),
        footprint: Footprint::SINGLE,
        cross_section: if length > Real::ZERO {
            volume / length
        } else {
            Real::ZERO
        },
    }
}

/// The space one member of an item takes up, from its `Physical`.
pub(crate) fn member_volume(sim: &Sim, item: EntityId) -> Real {
    let count = sim
        .components
        .get::<Item>(item)
        .map_or(1, |item| item.count().max(1));
    sim.components
        .get::<Physical>(item)
        .map_or(Real::ZERO, |physical| {
            physical.volume().get() / Real::from_i64(i64::from(count))
        })
}

/// Sets how many members an item has, and its mass and size to match.
pub(crate) fn set_count(
    sim: &mut Sim,
    item: EntityId,
    count: u32,
    cause: Cause,
) -> Result<(), Reason> {
    let defs = Arc::clone(&sim.defs);
    let def = sim
        .components
        .get::<Item>(item)
        .and_then(|item| super::def_of(&defs, item));
    if let Some(def) = def {
        let shape = shape_of(sim, def, count);
        sim.set_shape(item, shape, cause)?;
    }
    if let Some(item) = sim.components.get_mut::<Item>(item) {
        item.count = count;
    }
    Ok(())
}

/// Whether `count` items of a definition could be made.
fn check(sim: &Sim, def: ItemId, count: u32) -> Result<(), Reason> {
    let registry = sim.defs.try_registry::<ItemDef>().ok_or_else(|| {
        refuse(
            reason::NOT_REGISTERED,
            "the definitions were loaded without the `item` type",
        )
    })?;
    let item_def = registry.try_get(def).ok_or_else(|| {
        refuse(
            reason::UNKNOWN_DEF,
            "there is no item definition with this id",
        )
        .with("def", i64::from(def.raw()))
    })?;
    if count == 0 {
        return Err(refuse(reason::BAD_COUNT, "an item has at least one member")
            .with("count", 0_i64)
            .with("have", 0_i64));
    }
    if count > 1 && !item_def.stack {
        return Err(refuse(
            reason::NOT_STACKABLE,
            format!(
                "`{}` does not stack: each is an entity of its own",
                registry.name(def)
            ),
        )
        .with("item", registry.name(def)));
    }
    Ok(())
}

/// Makes `count` items of a definition as one entity and puts it
/// somewhere (06 §9.1). More than one needs a definition that stacks.
/// Its seed, from which its colour and its latent contents are drawn,
/// comes from the stream [`ITEM_STREAM`].
///
/// Put `In` a container it is merely inside it: [`put_in`](super::put_in)
/// stows it.
///
/// Refused, with nothing made, for an id that is not an item definition
/// (`item.unknown_def`), a count of zero (`item.bad_count`), more than
/// one of something that does not stack (`item.not_stackable`), and a
/// place the kernel refuses.
pub fn create(
    sim: &mut Sim,
    def: ItemId,
    count: u32,
    place: Place,
    cause: Cause,
) -> Result<EntityId, Reason> {
    check(sim, def, count)?;
    sim.check_place(None, Footprint::SINGLE, place)?;
    let seed = sim.rng.stream(ITEM_STREAM).next_u64();
    create_seeded(sim, def, count, place, seed, cause)
}

/// [`create`] with the item's seed given: for generation, which seeds by
/// position so that the same drawer always holds the same things
/// (06 §9.3).
pub fn create_seeded(
    sim: &mut Sim,
    def: ItemId,
    count: u32,
    place: Place,
    seed: u64,
    cause: Cause,
) -> Result<EntityId, Reason> {
    check(sim, def, count)?;
    let defs = Arc::clone(&sim.defs);
    let item_def = defs.registry::<ItemDef>().get(def);
    let mut spec = EntitySpec::new(OccupantKind::Item);
    for tag in item_def.tags.iter() {
        spec.tags.insert(defs.tags().name(tag));
    }
    let id = sim.spawn(spec, place, cause)?;
    sim.components.insert(
        id,
        Item {
            def: Some(def),
            material: item_def.material.id(),
            count,
            damage: 0,
            color: pick_color(&item_def.color, seed),
            seed,
            latent: !item_def.contents.is_empty(),
            remains: None,
        },
    );
    let key = sim
        .exprs
        .registry()
        .def_type_id(ItemDef::TYPE)
        .map(|ty| DefKey::new(ty, def.raw()));
    sim.components.insert(id, Identity::new(key));
    let shape = shape_of(sim, item_def, count);
    if let Err(reason) = sim.set_shape(id, shape, cause) {
        let _ = sim.remove_entity(id, cause);
        return Err(reason);
    }
    // What it is filled with when it is made (16 §9.5).
    crate::substance::on_made(sim, id);
    Ok(id)
}

/// Unmakes an item. What it held is first put down where it was
/// (06 §9.2); contents it was never opened to show are never made.
/// Refused with `item.not_an_item` for anything else.
pub fn destroy(sim: &mut Sim, item: EntityId, cause: Cause) -> Result<(), Reason> {
    item_of(sim, item)?;
    for inside in sim.entities.contents(item).to_vec() {
        // What cannot be put down there is left to the kernel, which
        // puts it where the item was.
        let _ = hold::drop_forced(sim, inside, cause);
    }
    sim.remove_entity(item, cause)
}

/// Uses up members of an item (07 §11.5: a round fired): a stack loses
/// `count` of them and keeps its place and hold; an item left with none
/// is unmade, as [`destroy`] unmakes it. Refused with `item.bad_count`
/// for none, or more than it has, and `item.not_an_item` for anything
/// else.
pub fn spend(sim: &mut Sim, item: EntityId, count: u32, cause: Cause) -> Result<(), Reason> {
    let have = item_of(sim, item)?.count;
    if count == 0 || count > have {
        return Err(refuse(
            reason::BAD_COUNT,
            format!("{count} cannot be used up of an item that has {have}"),
        )
        .with("count", i64::from(count))
        .with("have", i64::from(have)));
    }
    if count == have {
        return destroy(sim, item, cause);
    }
    set_count(sim, item, have - count, cause)
}

/// Changes the colour an item carries (06 §9.1), or with `None` takes
/// it away. Every member of a stack takes it.
pub fn set_color(sim: &mut Sim, item: EntityId, color: Option<&str>) -> Result<(), Reason> {
    item_of(sim, item)?;
    if let Some(item) = sim.components.get_mut::<Item>(item) {
        item.color = color.map(str::to_owned);
    }
    Ok(())
}
