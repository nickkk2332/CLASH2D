//! Finding what strikes (07 §3.1, §3.2): the surface an attack's `with`
//! names, on an item the attacker holds, on a part of its own body, or
//! given inline.
//!
//! A surface of a part is scaled with the part, as the part is (06 §6.4):
//! its area with the square of the part's size, its give and its span
//! with the size. A surface of an item is as its definition writes it.

use clash_core::{EntityId, Real};
use clash_world::defs::DamageTypeId;

use super::attack::{WithDef, WithSource};
use super::reason;
use crate::Sim;
use crate::body::{Body, BodyDefs, BodyPartDef, PartInst, PartKey, SurfaceDef};
use crate::item::{Hold, Holds, Item, ItemDef};
use crate::reason::Reason;

/// What supplies a striking surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Striker {
    /// A part of the attacker's body.
    Part(PartKey),
    /// An item the attacker holds, and the part holding it.
    Item {
        /// The item.
        item: EntityId,
        /// The first part whose slots its grasp takes.
        holder: Option<PartKey>,
    },
    /// Nothing: a surface given inline (08 §7.7). It takes no recoil.
    Inline,
}

/// A striking surface (07 §3.2), found for one strike.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Surface {
    /// The damage type it delivers.
    pub kind: DamageTypeId,
    /// Its contact area, in square metres.
    pub area: Real,
    /// The distance over which a strike with it is stopped, in metres.
    pub give: Real,
    /// Its length of contact across the target, in metres.
    pub span: Real,
    /// What it is a surface of.
    pub striker: Striker,
    /// How far it reaches when the attack says nothing (07 §3.1): the
    /// reach of the part that strikes (06 §6.4); for an item, that of
    /// the part holding it and the item's length; for an inline
    /// surface, the attacker's body's.
    pub reach: Real,
}

/// Whether a part answers to a name: its name in the body, the id of
/// its definition (qualified or bare) or a tag its definition carries.
pub(crate) fn answers(sim: &Sim, part: &PartInst, name: &str) -> bool {
    if part.name() == name {
        return true;
    }
    let Some(parts) = sim.defs().try_registry::<BodyPartDef>() else {
        return false;
    };
    let qualified = parts.name(part.def());
    let bare = qualified
        .rsplit_once(':')
        .map_or(qualified, |(_, bare)| bare);
    if qualified == name || bare == name {
        return true;
    }
    let tags = sim.defs().tags();
    parts
        .try_get(part.def())
        .is_some_and(|def| tags.lookup(name).is_some_and(|tag| def.tags.contains(tag)))
}

/// The parts of an entity that hold something in a grasp.
fn grasping(sim: &Sim, entity: EntityId) -> Vec<PartKey> {
    let Some(holds) = sim.components().get::<Holds>(entity) else {
        return Vec::new();
    };
    let mut parts: Vec<PartKey> = holds
        .iter()
        .filter_map(|(_, hold)| match hold {
            Hold::Grasp(slots) => Some(slots.iter().map(|slot| slot.part).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect();
    parts.sort_unstable();
    parts.dedup();
    parts
}

/// The surface of an id among a list.
fn named<'a>(surfaces: &'a [SurfaceDef], id: Option<&str>) -> Option<&'a SurfaceDef> {
    let id = id?;
    surfaces.iter().find(|surface| surface.id == id)
}

/// The body of an entity and the tables to read it.
fn body_of(sim: &Sim, entity: EntityId) -> Option<(&Body, std::sync::Arc<BodyDefs>)> {
    let body = sim.components().body.get(entity)?;
    let defs = sim.life.body_defs().ok()?;
    Some((body, defs))
}

fn no_surface(action: &str, attack: &str, with: &str) -> Reason {
    Reason::new(
        reason::NO_SURFACE,
        format!(
            "nothing answers to what the attack `{attack}` of `{action}` strikes with ({with})"
        ),
    )
    .with("action", action)
    .with("attack", attack)
    .with("with", with)
}

/// Finds the surface an attack's `with` names for an attacker (07 §3.1).
///
/// - `part`: the first working part, by key, that answers to the name
///   and has the surface, one that grasps nothing before one that does.
/// - `held`: the first item, by id, held in a grasp, that carries the
///   tag (any, for an empty tag) and whose definition has the surface.
/// - `gripped`: refused, grips are not built.
/// - inline: as written.
pub fn find(
    sim: &Sim,
    attacker: EntityId,
    with: &WithDef,
    action: &str,
    attack: &str,
) -> Result<Surface, Reason> {
    let Some(source) = with.source() else {
        return Err(no_surface(action, attack, "nothing"));
    };
    match source {
        WithSource::Inline => {
            let kind = with
                .kind
                .as_ref()
                .map(|kind| kind.id())
                .ok_or_else(|| no_surface(action, attack, "an inline surface with no kind"))?;
            let reach = body_of(sim, attacker).map_or(Real::ZERO, |(body, defs)| body.reach(&defs));
            Ok(Surface {
                kind,
                area: with.area.unwrap_or(Real::ZERO),
                give: with.give.map_or(Real::ZERO, |give| give.get()),
                span: with.span.map_or(Real::ZERO, |span| span.get()),
                striker: Striker::Inline,
                reach,
            })
        }
        WithSource::Gripped => Err(Reason::new(
            reason::NOTHING_GRIPPED,
            format!(
                "the attack `{attack}` of `{action}` strikes with something gripped, and nothing is"
            ),
        )
        .with("action", action)
        .with("attack", attack)),
        WithSource::Part(name) => {
            let what = format!("the part `{name}`");
            let (body, defs) =
                body_of(sim, attacker).ok_or_else(|| no_surface(action, attack, &what))?;
            let busy = grasping(sim, attacker);
            let candidates = body.parts().iter().filter(|part| {
                part.is_working()
                    && answers(sim, part, name)
                    && defs
                        .part(part.def())
                        .is_some_and(|def| named(&def.surfaces, with.surface.as_deref()).is_some())
            });
            let (free, held): (Vec<&PartInst>, Vec<&PartInst>) =
                candidates.partition(|part| !busy.contains(&part.key()));
            let part = free
                .first()
                .or_else(|| held.first())
                .ok_or_else(|| no_surface(action, attack, &what))?;
            let def = defs
                .part(part.def())
                .and_then(|def| named(&def.surfaces, with.surface.as_deref()))
                .ok_or_else(|| no_surface(action, attack, &what))?;
            let scale = body.part_scale(part);
            Ok(Surface {
                kind: def.kind.id(),
                area: def.area * scale * scale,
                give: def.give.get() * scale,
                span: def.span.get() * scale,
                striker: Striker::Part(part.key()),
                reach: body.part_reach(&defs, part.key()),
            })
        }
        WithSource::Held(tag) => {
            let what = if tag.is_empty() {
                "an item held".to_owned()
            } else {
                format!("an item held that is `{tag}`")
            };
            let holds = sim
                .components()
                .get::<Holds>(attacker)
                .ok_or_else(|| no_surface(action, attack, &what))?;
            for (item, hold) in holds.iter() {
                let Hold::Grasp(slots) = hold else { continue };
                let tagged = tag.is_empty()
                    || sim
                        .truth()
                        .entity(item)
                        .is_some_and(|record| record.tags.contains(tag));
                if !tagged {
                    continue;
                }
                let Some(def) = sim
                    .components()
                    .get::<Item>(item)
                    .and_then(|component| crate::item::def_of(sim.defs(), component))
                else {
                    continue;
                };
                let Some(surface) = named(&def.surfaces, with.surface.as_deref()) else {
                    continue;
                };
                let holder = slots.first().map(|slot| slot.part);
                return Ok(Surface {
                    kind: surface.kind.id(),
                    area: surface.area,
                    give: surface.give.get(),
                    span: surface.span.get(),
                    striker: Striker::Item { item, holder },
                    reach: item_reach(sim, attacker, holder, def),
                });
            }
            Err(no_surface(action, attack, &what))
        }
    }
}

/// How far an item held reaches: the reach of the part holding it and
/// the item's length.
fn item_reach(sim: &Sim, attacker: EntityId, holder: Option<PartKey>, def: &ItemDef) -> Real {
    let arm = match (holder, body_of(sim, attacker)) {
        (Some(part), Some((body, defs))) => body.part_reach(&defs, part),
        (None, Some((body, defs))) => body.reach(&defs),
        _ => Real::ZERO,
    };
    arm + def.length.get()
}
