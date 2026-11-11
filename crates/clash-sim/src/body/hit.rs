//! Hits (06 §7.1, §7.2, §7.4): everything that harms a body, and how it
//! works its way in.
//!
//! # Through the layers
//!
//! For the struck part, the layers are whatever is worn over it,
//! outermost first, then the part's own. The packet meets each in turn
//! by the world's rule (02 §12.2, [`resolve_packet`]):
//!
//! ```text
//! contact  = min(hit.area, the part's area)
//! cover    = min(1, hit.span / the part's girth)
//! capacity = material.toughness × contact × thickness
//!
//! for each layer, outside in:
//!     a worn layer the hit found a gap in is skipped
//!     a layer of the part that is wholly gone (damage 255) is not in the way
//!     apply the packet rule with that capacity and the layer's damage
//!     unaffected:  stop. The energy stays with whatever delivered it.
//!     damaged:     the layer gains cover × the 255ths the rule gives; stop
//!     breached:    the layer gains cover × 255; energy = what passed
//! ```
//!
//! The rule is applied to the volume under the contact, so a small
//! contact spends little in each layer and goes deep, and a wide one
//! spends its energy in the first layers. What is recorded on the layer
//! is how much of the part's width the breach crosses, because that is
//! what the part's working depends on: a blade drawn across a limb
//! undoes it, and a needle through it does not.
//!
//! If every layer is breached, one of the parts inside may be struck
//! with what remains, each with a chance in proportion to its size. If
//! the damage type severs and the hit's span is at least the part's
//! girth, the part comes off, with everything on it, and leaves a wound
//! on its parent. Whatever energy is left exits, and is returned so that
//! it can be carried on to what is behind.
//!
//! A part whose every layer is gone is destroyed: it comes off as a
//! ruin, and what was on it comes off with it. The root so destroyed is
//! the body in pieces.
//!
//! # Chance
//!
//! Which part an unaimed hit finds, which inner part is struck and
//! whether a hit finds a gap in what is worn are drawn from the stream
//! the caller hands in, and from nowhere else. A hit at a named part
//! with nothing worn loosely and nothing inside it draws nothing.

use clash_core::{EntityId, Real, Rng, StateHash, StateHasher, impl_state_hash};
use clash_expr::Host;
use clash_world::defs::{DamageTypeId, MaterialId};
use clash_world::{Cause, DamagePacket, DamageResult, resolve_packet};
use serde::{Deserialize, Serialize};

use super::defs::DAMAGE_PARTS;
use super::wound::{Wound, WoundState};
use super::{
    Body, BodyCx, BodyDefs, BodyError, Change, Changes, Detach, Detached, PartInst, PartKey,
    Relation, Work, body_mut, finish, present_part, take_off, wound,
};
use crate::stats;

/// The setting that says how many times what a whole body could absorb
/// a spread hit must carry to destroy it outright (06 §7.4).
pub const OVERKILL_SETTING: &str = "hit.overkill_factor";

/// The default of [`OVERKILL_SETTING`].
pub const DEFAULT_OVERKILL_FACTOR: Real = Real::of(10.0);

/// Where a hit arrives (06 §7.1).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitAt {
    /// At this part.
    Part(PartKey),
    /// At this height above the target's footing, in metres: on one
    /// outer part whose band holds it, chosen by exposed area.
    Height(Real),
    /// Anywhere: on one outer part, chosen by exposed area.
    Any,
    /// Everywhere: over every outer part, each taking a share of the
    /// energy by its exposed area (06 §7.4).
    Spread,
}

impl StateHash for HitAt {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            HitAt::Part(part) => {
                hasher.write_u8(0);
                part.state_hash(hasher);
            }
            HitAt::Height(height) => {
                hasher.write_u8(1);
                height.state_hash(hasher);
            }
            HitAt::Any => hasher.write_u8(2),
            HitAt::Spread => hasher.write_u8(3),
        }
    }
}

/// Something that harms a body (06 §7.1): a strike, a field, an impact
/// after being thrown, exposure to a pressure wave.
///
/// The fields are public so that the hook `hit_incoming` can be raised
/// from a hit and what its handlers changed written back before
/// [`apply_hit`] is called.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Hit {
    /// Whose body.
    pub target: EntityId,
    /// Where it arrives.
    pub at: HitAt,
    /// The kind of damage, the joules delivered, how concentrated they
    /// are, and who and what is behind it (02 §12.1).
    pub packet: DamagePacket,
    /// The area of contact, in square metres. Not used by a spread hit,
    /// which washes over the whole of each part.
    pub area: Real,
    /// The length of contact across the part, in metres (07 §3.2). Not
    /// used by a spread hit.
    pub span: Real,
    /// The direction it travels in: east, south and up. Carried on with
    /// whatever exits.
    pub dir: Option<[Real; 3]>,
}

impl_state_hash!(Hit {
    target,
    at,
    packet,
    area,
    span,
    dir
});

/// A layer worn over a part (06 §9.2): armour, clothing. Whoever holds
/// the items hands these in, outermost first for each part, and is told
/// what each took.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WornLayer {
    /// The part it covers.
    pub part: PartKey,
    /// The item it is a layer of.
    pub item: EntityId,
    /// What it is made of.
    pub material: MaterialId,
    /// How thick it is there, in metres.
    pub thickness: Real,
    /// The share of hits on the part that it is in the way of, from 0
    /// to 1.
    pub coverage: Real,
    /// The item's damage, in 255ths.
    pub damage: u8,
}

/// What a worn layer took of a hit: damage to that item (06 §7.2).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WornHit {
    /// The item.
    pub item: EntityId,
    /// The part it was struck over.
    pub part: PartKey,
    /// What the packet rule said of the stuff under the contact:
    /// unaffected, damaged, or breached.
    pub result: DamageResult,
    /// Joules it took.
    pub absorbed: Real,
}

/// What a hit did to one part.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PartHit {
    /// The part.
    pub part: PartKey,
    /// Joules that reached the part's own layers.
    pub arrived: Real,
    /// Joules those layers took.
    pub absorbed: Real,
    /// Joules that went on beyond them: into a part inside, or out.
    pub passed: Real,
    /// How many of its layers the hit got through.
    pub depth: u8,
    /// Whether it got through all of them.
    pub through: bool,
}

/// Energy that left the far side of a body (06 §7.2): what the world or
/// combat carries on to whatever is behind.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Exit {
    /// The part it left through. `None` if the hit met no part at all.
    pub part: Option<PartKey>,
    /// What is left: the hit's kind, intensity and cause, and the joules
    /// remaining.
    pub packet: DamagePacket,
    /// The area of contact it had.
    pub area: Real,
    /// The span it had.
    pub span: Real,
    /// The direction it travels in.
    pub dir: Option<[Real; 3]>,
}

/// What [`apply_hit`] reads of the settings.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HitSettings {
    /// [`OVERKILL_SETTING`].
    pub overkill_factor: Real,
}

impl Default for HitSettings {
    fn default() -> HitSettings {
        HitSettings {
            overkill_factor: DEFAULT_OVERKILL_FACTOR,
        }
    }
}

/// What a hit did.
///
/// Energy is accounted for: `absorbed`, what the worn layers took,
/// `returned` and what the exits carry add up to what the hit
/// delivered.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct HitReport {
    /// What changed, in order, for events.
    pub changes: Changes,
    /// What it did to each part it reached, outermost first.
    pub struck: Vec<PartHit>,
    /// What each worn layer in its way took.
    pub worn: Vec<WornHit>,
    /// Joules the body's own layers took.
    pub absorbed: Real,
    /// Joules that were stopped by a layer they could not affect. They
    /// stay with whatever delivered them (07 §6.3).
    pub returned: Real,
    /// What left the far side.
    pub exits: Vec<Exit>,
    /// The parts that came off, for whoever makes items of them.
    pub detached: Vec<Detached>,
    /// Whether the body is in pieces: its root was destroyed.
    pub destroyed: bool,
}

/// What a body-impact report says (02 §15.2), as far as a body needs
/// it: the energy the flying body lost against what it struck, and how
/// concentrated that was.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Impact {
    /// The kind of damage: the world's kinetic type, or the contact
    /// surface's.
    pub kind: DamageTypeId,
    /// Joules the body lost.
    pub energy: Real,
    /// How concentrated, in pascals.
    pub intensity: Real,
    /// The direction it was travelling in.
    pub dir: Option<[Real; 3]>,
    /// The surface it landed on, if that has one of its own (07 §6.5):
    /// its contact area and span. Without one the blow is blunt and
    /// spread over the body.
    pub contact: Option<(Real, Real)>,
    /// Who and what is behind it.
    pub cause: Cause,
}

/// What a field's `damage` rule delivers to an occupant in one step
/// (02 §16).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FieldDose {
    /// The kind of damage.
    pub kind: DamageTypeId,
    /// The field's level in the cell, from 0 to 255.
    pub level: u8,
    /// Joules for each level.
    pub energy_per_level: Real,
    /// Pascals for each level.
    pub intensity_per_level: Real,
    /// Who and what is behind it.
    pub cause: Cause,
}

/// The hit an occupant takes from what struck its cell
/// (`OccupantExposed`, 02 §13.2, §14.3): spread over the body.
///
/// The cell was delivered `energy` at `intensity`. A body takes what
/// that intensity puts into its own volume, and never more than was
/// delivered.
pub fn exposure_hit(
    body: &Body,
    target: EntityId,
    kind: DamageTypeId,
    energy: Real,
    intensity: Real,
    cause: Cause,
) -> Hit {
    let share = (intensity * body.volume()).min(energy).max(Real::ZERO);
    Hit {
        target,
        at: HitAt::Spread,
        packet: DamagePacket::new(kind, share, intensity, cause),
        area: body.cross_section(),
        span: Real::ZERO,
        dir: None,
    }
}

/// The hit a creature takes when it strikes something in flight
/// (`BodyImpact`, 02 §15.2): all of the energy it lost, spread over the
/// body, or with the surface it landed on if that has one (07 §6.5).
pub fn impact_hit(target: EntityId, impact: &Impact) -> Hit {
    let packet = DamagePacket::new(impact.kind, impact.energy, impact.intensity, impact.cause);
    match impact.contact {
        Some((area, span)) => Hit {
            target,
            at: HitAt::Any,
            packet,
            area,
            span,
            dir: impact.dir,
        },
        None => Hit {
            target,
            at: HitAt::Spread,
            packet,
            area: Real::ZERO,
            span: Real::ZERO,
            dir: impact.dir,
        },
    }
}

/// The hit an occupant takes from a field's `damage` rule in one step
/// (02 §16): the field's level times what each level delivers, taken as
/// [`exposure_hit`] takes it.
pub fn field_hit(body: &Body, target: EntityId, dose: &FieldDose) -> Hit {
    let level = Real::from_i64(i64::from(dose.level));
    exposure_hit(
        body,
        target,
        dose.kind,
        level * dose.energy_per_level,
        level * dose.intensity_per_level,
        dose.cause,
    )
}

/// One blow at one part.
#[derive(Clone, Copy)]
struct Blow {
    part: PartKey,
    energy: Real,
    area: Real,
    span: Real,
    /// Whether it can take the part off.
    severs: bool,
}

/// What a part looked like before a blow, and what the blow did to it.
struct Struck {
    hit: PartHit,
    /// Each layer's damage before.
    before: Vec<u8>,
    /// How many layers this blow breached, as opposed to found gone.
    breached: u8,
    /// Whether it is the part the blow was aimed at, not one inside it.
    outer: bool,
    /// Whether the blow could take it off.
    severs: bool,
    /// The tissue the blow laid open: the contact area, or for a
    /// contact narrower than it went deep, the face of the cut.
    opened: Real,
}

/// What a hit has done so far.
#[derive(Default)]
struct Tally {
    struck: Vec<Struck>,
    worn: Vec<WornHit>,
    /// Joules absorbed, by the material that absorbed them, for signals.
    signals: Vec<(MaterialId, Real)>,
    absorbed: Real,
    returned: Real,
}

/// What a hit needs of its surroundings while it works inward.
struct Strike<'a> {
    defs: &'a BodyDefs,
    kind: DamageTypeId,
    intensity: Real,
    worn: &'a [WornLayer],
    rng: &'a mut Rng,
}

impl Strike<'_> {
    /// Carries a blow into a part: through what is worn over it, its
    /// own layers, and one of the parts inside it. Returns the joules
    /// that leave the far side.
    fn enter(&mut self, body: &mut Body, blow: Blow, outer: bool, tally: &mut Tally) -> Real {
        let defs = self.defs;
        let Some(part) = body.part(blow.part) else {
            return blow.energy;
        };
        let area = body.part_area(defs, part);
        let girth = body.part_girth(defs, part);
        let contact = blow.area.min(area);
        let cover = if girth > Real::ZERO {
            (blow.span / girth).min(Real::ONE)
        } else {
            Real::ONE
        };
        let mut energy = blow.energy;

        // What is worn over it, outermost first.
        if outer {
            let worn = self.worn;
            for layer in worn.iter().filter(|layer| layer.part == blow.part) {
                if layer.coverage <= Real::ZERO {
                    continue;
                }
                if layer.coverage < Real::ONE && !self.rng.chance(layer.coverage) {
                    // It found a gap.
                    continue;
                }
                let Some(tissue) = defs.tissue(layer.material) else {
                    continue;
                };
                let capacity = tissue.toughness * contact * layer.thickness;
                let (outcome, _) = resolve_packet(
                    tissue.resist(self.kind),
                    capacity,
                    layer.damage,
                    energy,
                    self.intensity,
                );
                let took = match outcome.result {
                    DamageResult::Unaffected => Real::ZERO,
                    DamageResult::Damaged | DamageResult::Destroyed => outcome.absorbed,
                };
                tally.worn.push(WornHit {
                    item: layer.item,
                    part: blow.part,
                    result: outcome.result,
                    absorbed: took,
                });
                match outcome.result {
                    DamageResult::Unaffected => {
                        tally.returned += energy;
                        return Real::ZERO;
                    }
                    DamageResult::Damaged => return Real::ZERO,
                    DamageResult::Destroyed => energy = outcome.passed,
                }
            }
        }

        // The part's own layers.
        let arrived = energy;
        let before: Vec<u8> = part.layers().iter().map(|layer| layer.damage()).collect();
        let whole = Real::from_i64(i64::from(DAMAGE_PARTS));
        let mut absorbed = Real::ZERO;
        let mut depth: u8 = 0;
        let mut breached: u8 = 0;
        let mut through = true;
        for index in 0..before.len() {
            let Some(layer) = body
                .part(blow.part)
                .and_then(|part| part.layers().get(index))
            else {
                break;
            };
            let layer = *layer;
            if layer.damage() == DAMAGE_PARTS {
                // Wholly gone: nothing is in the way.
                depth += 1;
                continue;
            }
            let Some(tissue) = defs.tissue(layer.material()) else {
                depth += 1;
                continue;
            };
            let capacity = tissue.toughness * contact * layer.thickness();
            let (outcome, local) = resolve_packet(
                tissue.resist(self.kind),
                capacity,
                layer.damage(),
                energy,
                self.intensity,
            );
            let gained = match outcome.result {
                DamageResult::Unaffected => {
                    tally.returned += energy;
                    energy = Real::ZERO;
                    through = false;
                    break;
                }
                DamageResult::Damaged => {
                    Real::from_i64(i64::from(local.saturating_sub(layer.damage())))
                }
                DamageResult::Destroyed => whole,
            };
            // At least one part, as the rule has it: what affects a
            // layer never leaves it as it was.
            let gained = (gained * cover).ceil().max(Real::ONE).to_i64();
            let limit = match outcome.result {
                DamageResult::Destroyed => i64::from(DAMAGE_PARTS),
                _ => i64::from(DAMAGE_PARTS) - 1,
            };
            let after = (i64::from(layer.damage()) + gained).min(limit);
            if let Some(part) = body.part_mut(blow.part) {
                part.layers[index].damage = u8::try_from(after).unwrap_or(DAMAGE_PARTS);
            }
            absorbed += outcome.absorbed;
            tally.signals.push((layer.material(), outcome.absorbed));
            if outcome.result == DamageResult::Damaged {
                energy = Real::ZERO;
                through = false;
                break;
            }
            energy = outcome.passed;
            depth += 1;
            breached += 1;
        }
        tally.absorbed += absorbed;
        // What it laid open: the contact, or the face of the cut, which
        // is as wide as the contact crosses the part and as deep as the
        // layers it went through.
        let mut went_through = Real::ZERO;
        if let Some(part) = body.part(blow.part) {
            for layer in part.layers().iter().take(usize::from(depth)) {
                went_through += layer.thickness();
            }
        }
        let opened = contact.max(blow.span.min(girth) * went_through);
        tally.struck.push(Struck {
            hit: PartHit {
                part: blow.part,
                arrived,
                absorbed,
                passed: energy,
                depth,
                through,
            },
            before,
            breached,
            outer,
            severs: blow.severs,
            opened,
        });
        if !through || energy <= Real::ZERO {
            return Real::ZERO;
        }

        // One of the parts inside, by its size.
        let inside: Vec<(PartKey, Real)> = body
            .inside(blow.part)
            .map(|organ| (organ.key(), body.part_area(defs, organ)))
            .collect();
        if !inside.is_empty() && area > Real::ZERO {
            let roll = self.rng.unit() * area;
            let mut reached = Real::ZERO;
            for (organ, size) in inside {
                reached += size;
                if roll < reached {
                    let blow = Blow {
                        part: organ,
                        energy,
                        severs: false,
                        ..blow
                    };
                    energy = self.enter(body, blow, false, tally);
                    break;
                }
            }
        }
        energy
    }
}

/// Picks one of the candidates by weight with one draw. `None` if there
/// is nothing to pick from.
fn pick(candidates: &[(PartKey, Real)], rng: &mut Rng) -> Option<PartKey> {
    let mut total = Real::ZERO;
    for (_, weight) in candidates {
        total += *weight;
    }
    if total <= Real::ZERO {
        return None;
    }
    let roll = rng.unit() * total;
    let mut reached = Real::ZERO;
    for (part, weight) in candidates {
        reached += *weight;
        if roll < reached {
            return Some(*part);
        }
    }
    candidates
        .iter()
        .rev()
        .find(|(_, weight)| *weight > Real::ZERO)
        .map(|(part, _)| *part)
}

/// Refuses a hit that carries a number no hit can.
fn check(hit: &Hit) -> Result<(), BodyError> {
    let amounts = [
        ("energy", hit.packet.energy),
        ("area", hit.area),
        ("span", hit.span),
    ];
    for (what, value) in amounts {
        if !value.is_finite() || value < Real::ZERO {
            return Err(BodyError::BadHit { what });
        }
    }
    // A contact with no area concentrates without limit, and that is a
    // number the packet rule can use.
    if hit.packet.intensity < Real::ZERO {
        return Err(BodyError::BadHit { what: "intensity" });
    }
    if let HitAt::Height(height) = hit.at
        && !height.is_finite()
    {
        return Err(BodyError::BadHit { what: "height" });
    }
    if let Some(dir) = hit.dir
        && dir.iter().any(|axis| !axis.is_finite())
    {
        return Err(BodyError::BadHit { what: "direction" });
    }
    Ok(())
}

/// The outer parts of a body that are there, each with the area it
/// offers to a hit that is not aimed.
fn exposed(body: &Body, defs: &BodyDefs) -> Vec<(PartKey, Real)> {
    body.parts()
        .iter()
        .map(|part| (part.key(), body.exposed_area(defs, part)))
        .filter(|(_, area)| *area > Real::ZERO)
        .collect()
}

/// What it would take to breach every layer of every part that is
/// there, in joules, if the hit has full effect on all of them. `None`
/// if it has less than full effect on any.
fn whole_body_capacity(
    body: &Body,
    defs: &BodyDefs,
    kind: DamageTypeId,
    intensity: Real,
) -> Option<Real> {
    let whole = Real::from_i64(i64::from(DAMAGE_PARTS));
    let mut needed = Real::ZERO;
    for part in body.parts().iter().filter(|part| part.is_present()) {
        let area = body.part_area(defs, part);
        for layer in part.layers() {
            let tissue = defs.tissue(layer.material())?;
            let resist = tissue.resist(kind);
            let gain = (intensity / resist.threshold - Real::ONE).clamp(Real::ZERO, Real::ONE);
            if gain < Real::ONE || resist.factor <= Real::ZERO {
                return None;
            }
            let remaining = tissue.toughness
                * area
                * layer.thickness()
                * (Real::ONE - Real::from_i64(i64::from(layer.damage())) / whole);
            needed += remaining / resist.factor;
        }
    }
    Some(needed)
}

/// Records a breach of a part as a wound: a new one, or one of the same
/// kind already open there made worse.
fn inflict(
    body: &mut Body,
    defs: &BodyDefs,
    entity: EntityId,
    mut fresh: Wound,
    out: &mut Changes,
) {
    let limit = body
        .part(fresh.part)
        .map_or(fresh.area, |part| body.part_area(defs, part));
    let closes_in = body.closing_time(defs, fresh.part, fresh.depth);
    let open = body.wounds.iter().position(|wound| {
        wound.part == fresh.part
            && wound.kind == fresh.kind
            && wound.state == WoundState::Open
            && wound.lost.is_none()
            && fresh.lost.is_none()
    });
    // Every id taken: the breach joins whatever wound the part has.
    let merge = open.or_else(|| {
        (body.wounds.len() >= usize::from(u16::MAX))
            .then(|| {
                body.wounds
                    .iter()
                    .position(|wound| wound.part == fresh.part)
            })
            .flatten()
    });
    let at = match merge {
        Some(at) => {
            let depth = body.wounds[at].depth.max(fresh.depth);
            let closes_in = body.closing_time(defs, fresh.part, depth);
            let wound = &mut body.wounds[at];
            wound.depth = depth;
            wound.area = (wound.area + fresh.area).min(limit);
            wound.energy += fresh.energy;
            wound.cause = fresh.cause;
            wound.state = WoundState::Open;
            wound.closes_in = closes_in;
            at
        }
        None => {
            let Some(id) = body.free_wound_id() else {
                return;
            };
            fresh.id = id;
            fresh.closes_in = closes_in;
            let at = body.wounds.partition_point(|wound| wound.id < id);
            body.wounds.insert(at, fresh);
            at
        }
    };
    let wound = &body.wounds[at];
    out.push(Change::Wounded {
        entity,
        wound: wound.id,
        part: wound.part,
        kind: wound.kind,
        depth: wound.depth,
        area: wound.area,
    });
}

/// A wound as a hit first makes it.
fn fresh_wound(part: PartKey, kind: DamageTypeId, cause: Cause) -> Wound {
    Wound {
        id: super::WoundId::new(0),
        part,
        kind,
        depth: 0,
        area: Real::ZERO,
        energy: Real::ZERO,
        age: clash_core::Dur::ZERO,
        state: WoundState::Open,
        closes_in: None,
        lost: None,
        cause,
        leaks: Vec::new(),
    }
}

/// Takes a part off because a hit severed or destroyed it, and leaves
/// the wound on what it was joined to.
#[allow(clippy::too_many_arguments)]
fn lose_part(
    body: &mut Body,
    defs: &BodyDefs,
    entity: EntityId,
    part: PartKey,
    how: Detach,
    packet: &DamagePacket,
    energy: Real,
    work: &mut Work,
    detached: &mut Vec<Detached>,
) {
    let Some(found) = body.part(part) else {
        return;
    };
    let girth = body.part_girth(defs, found);
    let (parent, relation) = (found.parent(), found.relation());
    let Some(taken) = take_off(body, defs, entity, part, how, packet.cause, work) else {
        return;
    };
    detached.push(taken);
    match how {
        Detach::Destroyed => work.changes.push(Change::PartDestroyed { entity, part }),
        _ => work.changes.push(Change::PartSevered {
            entity,
            part,
            parent,
        }),
    }
    // The stump: everything the parent is made of is open there, over
    // the cross-section of what came off. An organ leaves none: the
    // wound that reached it is already on what held it.
    let Some(parent) = parent.filter(|_| relation == Relation::On) else {
        return;
    };
    let Some(layers) = body.part(parent).map(|parent| parent.layers().len()) else {
        return;
    };
    let mut stump = fresh_wound(parent, packet.kind, packet.cause);
    stump.depth = u8::try_from(layers).unwrap_or(u8::MAX);
    stump.area = girth * girth;
    stump.energy = energy;
    stump.lost = Some(part);
    inflict(body, defs, entity, stump, &mut work.changes);
}

/// Destroys every part of a body at once (06 §7.4): what a hit many
/// times what the whole body could absorb does.
fn destroy_outright(body: &mut Body, entity: EntityId, work: &mut Work) {
    work.gone.extend(body.wounds.iter().map(Wound::id));
    body.wounds.clear();
    for part in &mut body.parts {
        if !part.is_present() {
            continue;
        }
        part.layers.clear();
        part.state = super::PartState::Destroyed;
        work.changes.push(Change::PartDestroyed {
            entity,
            part: part.key(),
        });
    }
    work.changes.push(Change::BodyDestroyed { entity });
}

/// Applies a hit to a body (06 §7.2 to §7.5) and says what it did.
///
/// `worn` is every layer worn over the parts the hit may reach,
/// outermost first for each part. `rng` is the stream chance is drawn
/// from: which part an unaimed hit finds, which inner part is struck,
/// and whether a worn layer is in the way.
///
/// The hook `hit_incoming` is the caller's to raise first. Whether what
/// the hit did ends the creature is not decided here (06 §8.3).
///
/// Refused, with nothing changed, if the target has no body, if the hit
/// names a part the body does not have or has lost, or if one of its
/// numbers is negative or not finite. A hit that meets no part at all
/// is not refused: all of it exits.
pub fn apply_hit<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    hit: &Hit,
    worn: &[WornLayer],
    rng: &mut Rng,
    settings: &HitSettings,
) -> Result<HitReport, BodyError> {
    check(hit)?;
    let entity = hit.target;
    let defs = cx.defs;
    {
        let body = super::body_of(cx.bodies, entity)?;
        if let HitAt::Part(part) = hit.at {
            present_part(body, entity, part)?;
        }
    }
    let mut work = Work::default();
    wound::sync(cx, entity, &mut work)?;
    let body = body_mut(cx.bodies, entity)?;
    let packet = hit.packet;
    let mut report = HitReport::default();
    let exit = |part: Option<PartKey>, energy: Real| Exit {
        part,
        packet: DamagePacket::new(packet.kind, energy, packet.intensity, packet.cause),
        area: hit.area,
        span: hit.span,
        dir: hit.dir,
    };

    // Where it lands.
    let whole_part = |body: &Body, part: &PartInst, energy: Real| Blow {
        part: part.key(),
        energy,
        area: body.part_area(defs, part),
        span: body.part_girth(defs, part),
        severs: false,
    };
    let aimed = |part: PartKey| Blow {
        part,
        energy: packet.energy,
        area: hit.area,
        span: hit.span,
        severs: true,
    };
    let blows: Vec<Blow> = match hit.at {
        HitAt::Part(part) => match body.part(part) {
            // It was there when the hit was checked; time has passed
            // since only for healing.
            Some(found) if found.is_present() => vec![aimed(part)],
            _ => Vec::new(),
        },
        HitAt::Height(height) => {
            let candidates: Vec<(PartKey, Real)> = body
                .parts_at_height(defs, height)
                .map(|part| (part.key(), body.exposed_area(defs, part)))
                .collect();
            pick(&candidates, rng).map(aimed).into_iter().collect()
        }
        HitAt::Any => pick(&exposed(body, defs), rng)
            .map(aimed)
            .into_iter()
            .collect(),
        HitAt::Spread => {
            let parts = exposed(body, defs);
            let mut total = Real::ZERO;
            for (_, area) in &parts {
                total += *area;
            }
            parts
                .iter()
                .filter_map(|(key, area)| {
                    let part = body.part(*key)?;
                    Some(whole_part(body, part, packet.energy * *area / total))
                })
                .collect()
        }
    };
    if blows.is_empty() {
        // Nothing was there: all of it passes.
        report.exits.push(exit(None, packet.energy));
        finish(cx, entity, &mut work)?;
        report.changes = work.changes;
        return Ok(report);
    }

    // Many times what the whole body could absorb: no layers resolved.
    if hit.at == HitAt::Spread
        && packet.energy > Real::ZERO
        && let Some(needed) = whole_body_capacity(body, defs, packet.kind, packet.intensity)
        && packet.energy >= needed * settings.overkill_factor.max(Real::ONE)
    {
        destroy_outright(body, entity, &mut work);
        report.absorbed = needed;
        report.destroyed = true;
        report.exits.push(exit(None, packet.energy - needed));
        finish(cx, entity, &mut work)?;
        report.changes = work.changes;
        return Ok(report);
    }

    let mut tally = Tally::default();
    let mut strike = Strike {
        defs,
        kind: packet.kind,
        intensity: packet.intensity,
        worn,
        rng,
    };
    for blow in &blows {
        let out = strike.enter(body, *blow, true, &mut tally);
        if out > Real::ZERO {
            report.exits.push(exit(Some(blow.part), out));
        }
    }

    // What that left: wounds, and parts cut off or destroyed.
    let mut lost: Vec<(PartKey, Detach, Real)> = Vec::new();
    for struck in &tally.struck {
        let part = struck.hit.part;
        let Some(found) = body.part(part) else {
            continue;
        };
        if struck.hit.absorbed > Real::ZERO || struck.hit.depth > 0 {
            work.changes.push(Change::HitLanded {
                entity,
                part,
                kind: packet.kind,
                absorbed: struck.hit.absorbed,
                passed: struck.hit.passed,
                depth: struck.hit.depth,
            });
        }
        let girth = body.part_girth(defs, found);
        let cut_off = struck.outer
            && struck.severs
            && struck.hit.through
            && defs.severs(packet.kind)
            && hit.span >= girth
            && found.parent().is_some()
            && found.relation() == Relation::On;
        if cut_off {
            // The cut is where it joins: the part comes off as it was.
            if let Some(found) = body.part_mut(part) {
                for (layer, before) in found.layers.iter_mut().zip(&struck.before) {
                    layer.damage = *before;
                }
            }
            lost.push((part, Detach::Severed, struck.hit.absorbed));
            continue;
        }
        let ruined = !found.layers().is_empty()
            && found
                .layers()
                .iter()
                .all(|layer| layer.damage() == DAMAGE_PARTS);
        if ruined {
            lost.push((part, Detach::Destroyed, struck.hit.absorbed));
            continue;
        }
        if struck.breached > 0 {
            let mut fresh = fresh_wound(part, packet.kind, packet.cause);
            fresh.depth = struck.hit.depth;
            fresh.area = struck.opened;
            fresh.energy = struck.hit.absorbed;
            inflict(body, defs, entity, fresh, &mut work.changes);
        }
    }
    let root = body.root().map(PartInst::key);
    for (part, how, energy) in lost {
        if Some(part) == root {
            // The body is in pieces: everything on the root comes off,
            // and the root is a ruin where it was.
            let on_root: Vec<PartKey> = body
                .children(part)
                .filter(|child| child.is_present() && child.relation() == Relation::On)
                .map(PartInst::key)
                .collect();
            for child in on_root {
                if let Some(taken) = take_off(
                    body,
                    defs,
                    entity,
                    child,
                    Detach::Severed,
                    packet.cause,
                    &mut work,
                ) {
                    report.detached.push(taken);
                    work.changes.push(Change::PartSevered {
                        entity,
                        part: child,
                        parent: Some(part),
                    });
                }
            }
            // What is left is the root and what is inside it.
            for key in body.subtree(part) {
                if let Some(found) = body.part_mut(key)
                    && found.is_present()
                {
                    found.layers.clear();
                    found.state = super::PartState::Destroyed;
                    work.changes
                        .push(Change::PartDestroyed { entity, part: key });
                }
            }
            work.gone.extend(body.wounds.iter().map(Wound::id));
            body.wounds.clear();
            work.changes.push(Change::BodyDestroyed { entity });
            report.destroyed = true;
            continue;
        }
        lose_part(
            body,
            defs,
            entity,
            part,
            how,
            &packet,
            energy,
            &mut work,
            &mut report.detached,
        );
    }

    report.struck = tally.struck.iter().map(|struck| struck.hit).collect();
    report.worn = tally.worn;
    report.absorbed = tally.absorbed;
    report.returned = tally.returned;

    // Pain and the like: what each damaged tissue adds to a pool for
    // the joules it took.
    for (material, absorbed) in tally.signals {
        let Some(signal) = defs.tissue(material).and_then(|tissue| tissue.signal) else {
            continue;
        };
        let amount = signal.per_joule * absorbed;
        if !amount.is_finite() || amount == Real::ZERO {
            continue;
        }
        match stats::pool_add(&mut cx.stats, entity, signal.pool, amount) {
            Ok(added) => work
                .changes
                .extend(added.changes.into_iter().map(Change::Stats)),
            Err(
                stats::StatsError::NoStats { .. }
                | stats::StatsError::NoSuchPool { .. }
                | stats::StatsError::UnknownStat { .. },
            ) => {}
            Err(error) => return Err(BodyError::Stats(error)),
        }
    }

    finish(cx, entity, &mut work)?;
    report.changes = work.changes;
    Ok(report)
}
