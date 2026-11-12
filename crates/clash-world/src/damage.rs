//! Damage and destruction (02 §12).
//!
//! Every source of harm arrives as a [`DamagePacket`]: so much energy,
//! delivered so concentrated. [`resolve_packet`] is the rule of 02 §12.2:
//! intensity against a threshold decides whether the target is affected
//! at all, and energy against what is left of its capacity decides how
//! much. [`WorldEdit::damage`] applies a packet to one element of a cell,
//! and destroys the element when the packet is enough: it becomes its
//! `on_destroy` state or is removed, what it lost is left as rubble, and
//! `ElementDestroyed` is emitted.
//!
//! ```
//! use clash_core::Real;
//! use clash_world::defs::Resist;
//! use clash_world::{DamageResult, resolve_packet};
//!
//! // The wall of 02 §12.2: 0.6 MJ of capacity behind a 20 MPa threshold.
//! let wall = Resist { threshold: Real::of(20.0e6), factor: Real::ONE };
//! let capacity = Real::of(0.6e6);
//!
//! // Someone thrown into it at 20 m/s: 16 kJ at 0.3 MPa. Nothing happens.
//! let (slow, _) = resolve_packet(wall, capacity, 0, Real::of(16.0e3), Real::of(0.3e6));
//! assert_eq!(slow.result, DamageResult::Unaffected);
//!
//! // At 600 m/s: 14.4 MJ at 290 MPa. The wall goes and 13.8 MJ carries on.
//! let (fast, _) = resolve_packet(wall, capacity, 0, Real::of(14.4e6), Real::of(290.0e6));
//! assert_eq!(fast.result, DamageResult::Destroyed);
//! assert!((fast.passed.get() - 13.8e6).abs() < 1.0);
//! ```

use std::sync::Arc;

use clash_core::{Real, StateHash, StateHasher};
use serde::{Deserialize, Serialize};

use crate::cause::Cause;
use crate::chunk::DAMAGE_MAX;
use crate::cosmos::Cosmos;
use crate::defs::{
    BlockState, CellFlags, DamageTypeId, ElementState, FixtureId, FixtureState, Layer, MaterialId,
    Resist, WorldDefs,
};
use crate::dims::STOREY_MM;
use crate::edit::WorldEdit;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::grid::{HEIGHT_FULL, HEIGHT_NONE};
use crate::pos::Location;

/// The parts an element's capacity is counted in: accumulated damage is
/// stored in 255ths (02 §12.2).
pub const DAMAGE_PARTS: u16 = 255;

/// How many units of rubble make a cubic metre. A fixture that piles up
/// (one that names a `heap`) keeps its amount in its state's bits, in
/// these units: litres.
pub const RUBBLE_UNITS_PER_M3: u32 = 1000;

/// The units of rubble that fill a cell: its three cubic metres. A cell
/// is one metre square, so a litre is a millimetre of its height.
pub const RUBBLE_CELL_UNITS: u32 = STOREY_MM as u32;

/// How many cells rubble will climb over or fall past to find where it
/// comes to rest, before what is left of it is dropped.
const RUBBLE_MAX_STEPS: u32 = 512;

/// One delivery of harm: so much energy, so concentrated (02 §12.1).
///
/// Two numbers are needed because energy alone cannot tell a thousand
/// taps from one hammer blow. Intensity decides whether the target is
/// affected at all; energy decides how much. Whoever sends a packet
/// works out its intensity (02 §12.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DamagePacket {
    /// The kind of damage.
    pub kind: DamageTypeId,
    /// Joules delivered.
    pub energy: Real,
    /// How concentrated the delivery is, in J/m³, which is numerically
    /// pascals.
    pub intensity: Real,
    /// Who and what is behind it. What the packet does carries this
    /// cause.
    pub cause: Cause,
}

impl DamagePacket {
    /// A packet of a kind, with its energy in joules and its intensity
    /// in pascals.
    pub const fn new(kind: DamageTypeId, energy: Real, intensity: Real, cause: Cause) -> Self {
        DamagePacket {
            kind,
            energy,
            intensity,
            cause,
        }
    }
}

clash_core::impl_state_hash!(DamagePacket {
    kind,
    energy,
    intensity,
    cause
});

/// What a packet did to its target.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageResult {
    /// Nothing: the packet was not concentrated enough, or carried no
    /// energy, or there was nothing there.
    Unaffected,
    /// The target took damage and stands.
    Damaged,
    /// The target was destroyed.
    Destroyed,
}

impl StateHash for DamageResult {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// The outcome of applying a packet (02 §12.1).
///
/// A target that is affected takes all of the packet that does not
/// pass: `absorbed + passed` is the packet's energy. A target that is
/// not affected takes none, and none passes: the energy stays with
/// whatever delivered it. Where there is nothing to hit, all of it
/// passes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DamageOutcome {
    /// What happened to the target.
    pub result: DamageResult,
    /// Joules taken by the target.
    pub absorbed: Real,
    /// Joules that continue beyond it.
    pub passed: Real,
}

impl DamageOutcome {
    /// Nothing happened and nothing passed.
    pub const UNAFFECTED: DamageOutcome = DamageOutcome {
        result: DamageResult::Unaffected,
        absorbed: Real::ZERO,
        passed: Real::ZERO,
    };

    /// There was nothing to hit: all of the energy passes.
    pub const fn through(energy: Real) -> DamageOutcome {
        DamageOutcome {
            result: DamageResult::Unaffected,
            absorbed: Real::ZERO,
            passed: energy,
        }
    }
}

clash_core::impl_state_hash!(DamageOutcome {
    result,
    absorbed,
    passed
});

/// The rule of 02 §12.2: what a packet of `energy` joules at `intensity`
/// pascals does to something that resists it with `resist`, absorbs
/// `capacity` joules whole and has taken `damage` 255ths already.
/// Returns the outcome and the damage afterwards.
///
/// ```text
/// gain      = clamp(intensity / threshold − 1, 0, 1)
/// effective = energy × factor × gain
/// remaining = capacity × (1 − damage / 255)
///
/// effective ≤ 0:          Unaffected. passed = 0
/// effective ≥ remaining:  Destroyed.  passed = energy × (1 − remaining / effective)
/// otherwise:              Damaged.    damage += ceil(255 × effective / capacity); passed = 0
/// ```
///
/// Below the threshold nothing happens, however often it is repeated.
/// Full effect is reached at twice the threshold. A target that is
/// damaged and not destroyed keeps at least one part of its capacity:
/// its damage stops at 254. The same rule serves body tissue (06).
pub fn resolve_packet(
    resist: Resist,
    capacity: Real,
    damage: u8,
    energy: Real,
    intensity: Real,
) -> (DamageOutcome, u8) {
    let parts = Real::from_i64(i64::from(DAMAGE_PARTS));
    let damage = damage.min(DAMAGE_MAX);
    let energy = energy.max(Real::ZERO);
    let capacity = capacity.max(Real::ZERO);
    let gain = (intensity / resist.threshold - Real::ONE).clamp(Real::ZERO, Real::ONE);
    let effective = energy * resist.factor.max(Real::ZERO) * gain;
    if effective <= Real::ZERO {
        return (DamageOutcome::UNAFFECTED, damage);
    }
    let remaining = capacity * (Real::ONE - Real::from_i64(i64::from(damage)) / parts);
    if effective >= remaining {
        let passed = energy * (Real::ONE - remaining / effective);
        let outcome = DamageOutcome {
            result: DamageResult::Destroyed,
            absorbed: energy - passed,
            passed,
        };
        return (outcome, damage);
    }
    let added = (parts * effective / capacity).ceil().to_i64();
    let after = (i64::from(damage) + added).clamp(0, i64::from(DAMAGE_MAX));
    let outcome = DamageOutcome {
        result: DamageResult::Damaged,
        absorbed: energy,
        passed: Real::ZERO,
    };
    (outcome, u8::try_from(after).unwrap_or(DAMAGE_MAX))
}

/// How an element came to be destroyed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Destruction {
    /// A packet of this kind of damage was more than it could take.
    Damage(DamageTypeId),
    /// It was carrying more than it could bear (02 §14.5).
    Crushed,
    /// Nothing held it up, and it fell (02 §14.3).
    Fell,
}

impl StateHash for Destruction {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Destruction::Damage(kind) => {
                hasher.write_u8(0);
                kind.state_hash(hasher);
            }
            Destruction::Crushed => hasher.write_u8(1),
            Destruction::Fell => hasher.write_u8(2),
        }
    }
}

/// What a destruction offers to be thrown: the fragments of an element
/// that a packet destroyed with energy to spare (02 §12.3). Part of the
/// mass flies as debris bodies with part of the energy; the rest of the
/// element is left as rubble.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DebrisSpec {
    /// The cell of the element that was destroyed.
    pub from: Location,
    /// Which of the cell's elements it was.
    pub layer: Layer,
    /// What the fragments are made of.
    pub material: MaterialId,
    /// The mass, in kilograms, that the element lost.
    pub mass: Real,
    /// The energy, in joules, that the packet had left over.
    pub energy: Real,
    /// The kind of damage that destroyed it.
    pub kind: DamageTypeId,
    /// The direction of the hit, east, south and up, if whoever struck
    /// gave one.
    pub dir: Option<[Real; 3]>,
    /// Who and what destroyed it.
    pub cause: Cause,
}

/// What was left of an element that was destroyed: the matter it lost.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Wreck {
    /// What it was made of.
    pub material: MaterialId,
    /// Cubic metres lost.
    pub volume: Real,
    /// Kilograms lost.
    pub mass: Real,
}

impl WorldDefs {
    /// Whether a fixture definition piles up: it names a `heap`, and its
    /// state's bits are an amount in units of rubble
    /// ([`RUBBLE_UNITS_PER_M3`]).
    pub fn piles_up(&self, fixture: FixtureId) -> bool {
        self.fixtures()
            .get(fixture)
            .is_some_and(|element| element.heap.is_some())
    }

    /// The units of rubble a fixture state holds, if its definition
    /// piles up. An amount of zero, as a state written without one has,
    /// stands for the volume the definition gives.
    pub fn rubble_units(&self, state: FixtureState) -> Option<u32> {
        if state.is_none() || !self.piles_up(state.def()) {
            return None;
        }
        Some(match state.bits() {
            0 => units_of(self.volume(Layer::Fixture, state.raw())),
            amount => u32::from(amount),
        })
    }

    /// The matter in an element, in cubic metres: its volume, or for a
    /// fixture that piles up the amount it holds.
    pub fn bulk(&self, layer: Layer, state: ElementState) -> Real {
        let piled = match layer {
            Layer::Fixture => self.rubble_units(FixtureState::from_raw(state)),
            _ => None,
        };
        match piled {
            Some(units) => Real::from_i64(i64::from(units)) / per_m3(),
            None => self.volume(layer, state),
        }
    }

    /// The mass of the matter in an element, in kilograms: its
    /// [`bulk`](WorldDefs::bulk) times the density of its material.
    pub fn bulk_mass(&self, layer: Layer, state: ElementState) -> Real {
        self.bulk(layer, state) * self.density(state.material)
    }

    /// The density of a material in kg/m³. Zero for one that is unknown.
    pub fn density(&self, material: MaterialId) -> Real {
        self.materials()
            .get(material)
            .map_or(Real::ZERO, |material| material.density)
    }

    /// Whether an element is rubble already: a fixture that piles up, or
    /// the heap block of one. Rubble that is destroyed leaves none.
    fn is_rubble(&self, layer: Layer, state: ElementState) -> bool {
        match layer {
            Layer::Floor => false,
            Layer::Fixture => self.piles_up(FixtureState::from_raw(state).def()),
            Layer::Block => {
                let block = BlockState::from_raw(state).def();
                self.fixtures()
                    .iter()
                    .any(|(_, fixture)| fixture.heap == Some(block))
            }
        }
    }
}

fn per_m3() -> Real {
    Real::from_i64(i64::from(RUBBLE_UNITS_PER_M3))
}

/// A volume in cubic metres as units of rubble, rounded.
fn units_of(volume: Real) -> u32 {
    let units = (volume.max(Real::ZERO) * per_m3()).round().to_i64();
    u32::try_from(units.max(0)).unwrap_or(u32::MAX)
}

impl Cosmos {
    /// Where something dropped into a cell comes to rest: that cell if
    /// it can be stood in, or else the nearest below that can. Where the
    /// loaded world ends first, the last cell reached.
    pub fn resting_cell(&self, at: Location) -> Result<Location, WorldError> {
        let view = self.grid(at.grid)?;
        let mut cell = view.normalize(at.cell)?;
        // Past open sky in one step: everything above the top of the
        // column is open.
        let top = view.height(cell)?;
        if top == HEIGHT_NONE {
            cell.z = cell.z.min(view.topology().z_min);
        } else if top != HEIGHT_FULL && cell.z > top {
            cell.z = top;
        }
        while !view.standable(cell)? {
            match view.normalize(cell.below()) {
                Ok(below) if view.is_loaded(below) => cell = below,
                _ => break,
            }
        }
        Ok(Location::new(at.grid, cell))
    }
}

impl WorldEdit<'_> {
    /// Applies a packet of damage to one element of a cell, by the rule
    /// of 02 §12.2 ([`resolve_packet`]), and says what it did.
    ///
    /// What the packet does carries the packet's cause, whatever cause
    /// this edit was begun for.
    ///
    /// - **Unaffected**: nothing changes and nothing is emitted. Where
    ///   the cell has no element on the layer, all of the energy passes.
    /// - **Damaged**: the element's accumulated damage rises, as by
    ///   [`set_damage`](WorldEdit::set_damage): `ElementDamaged`, and a
    ///   stability check for a floor or block.
    /// - **Destroyed** (02 §12.3): the element becomes its `on_destroy`
    ///   state, or is removed; the matter it lost is added to the cell
    ///   as its material's rubble, or to the nearest cell below that can
    ///   be stood in; `ElementDestroyed` is emitted after the
    ///   `ElementChanged` events of those changes; and a stability check
    ///   is asked for.
    ///
    /// Refused, changing nothing, for a cell outside its grid or not
    /// loaded.
    pub fn damage(
        &mut self,
        loc: Location,
        layer: Layer,
        packet: DamagePacket,
    ) -> Result<DamageOutcome, WorldError> {
        self.damage_along(loc, layer, packet, None)
    }

    /// As [`damage`](WorldEdit::damage), for a hit that has a direction:
    /// east, south and up. Fragments of what the packet destroys are
    /// thrown along it as debris bodies (02 §12.3), and the energy of
    /// their motion is taken out of what passes: it is counted as
    /// absorbed, so that the two still add up to the packet's energy.
    pub fn damage_along(
        &mut self,
        loc: Location,
        layer: Layer,
        packet: DamagePacket,
        dir: Option<[Real; 3]>,
    ) -> Result<DamageOutcome, WorldError> {
        self.strike(loc, layer, packet, dir, true)
            .map(|(outcome, _)| outcome)
    }

    /// Destroys the element of a cell on a layer outright, as though a
    /// packet of damage of `kind` had been more than it could take
    /// (02 §12.3): it becomes its `on_destroy` state or is removed, the
    /// matter it lost is left as rubble, and `ElementDestroyed` is
    /// emitted with nothing absorbed and nothing passed. No fragments
    /// fly: there is no blow to throw them. Returns whether there was
    /// an element to destroy.
    ///
    /// For rules that destroy by other means than energy (02 §23). What
    /// is destroyed this way carries this edit's cause.
    ///
    /// Refused, changing nothing, for a cell outside its grid or not
    /// loaded.
    pub fn destroy(
        &mut self,
        loc: Location,
        layer: Layer,
        kind: DamageTypeId,
    ) -> Result<bool, WorldError> {
        let wrecked = self.wreck(
            loc,
            layer,
            Destruction::Damage(kind),
            DamageOutcome {
                result: DamageResult::Destroyed,
                absorbed: Real::ZERO,
                passed: Real::ZERO,
            },
            None,
            true,
        )?;
        Ok(wrecked.is_some())
    }

    /// Applies a packet, and returns with the outcome what was left of
    /// the element if it was destroyed. With `leave_rubble` unset the
    /// wreck is not left in the cell: the caller carries it off.
    pub(crate) fn strike(
        &mut self,
        loc: Location,
        layer: Layer,
        packet: DamagePacket,
        dir: Option<[Real; 3]>,
        leave_rubble: bool,
    ) -> Result<(DamageOutcome, Option<Wreck>), WorldError> {
        let view = self.ctx.cosmos.grid(loc.grid)?;
        let cell = view.normalize(loc.cell)?;
        let loc = Location::new(loc.grid, cell);
        let state = view.element(cell, layer)?;
        if state.is_none() {
            return Ok((DamageOutcome::through(packet.energy.max(Real::ZERO)), None));
        }
        let damage = view.damage(cell)?[layer.index()];
        let defs = self.ctx.cosmos.defs();
        let resist = defs.resist(layer, state, packet.kind);
        let toughness = defs
            .materials()
            .get(state.material)
            .map_or(Real::ZERO, |material| material.toughness);
        let capacity = defs.bulk(layer, state) * toughness;
        let (outcome, after) =
            resolve_packet(resist, capacity, damage, packet.energy, packet.intensity);

        let mut edit = self.ctx.edit(packet.cause);
        match outcome.result {
            DamageResult::Unaffected => Ok((outcome, None)),
            DamageResult::Damaged => {
                edit.set_damage(loc, layer, after)?;
                Ok((outcome, None))
            }
            DamageResult::Destroyed => {
                let how = Destruction::Damage(packet.kind);
                let wrecked = edit.wreck_thrown(loc, layer, how, outcome, dir, leave_rubble)?;
                Ok(match wrecked {
                    Some((wreck, outcome)) => (outcome, Some(wreck)),
                    None => (outcome, None),
                })
            }
        }
    }

    /// Destroys the element of a cell on a layer (02 §12.3) and returns
    /// the matter it lost, or `None` if there was no element.
    ///
    /// An element that fell is removed whole. Otherwise it becomes its
    /// `on_destroy` state, or is removed. With `leave_rubble`, what it
    /// lost is left as rubble. `ElementDestroyed` is emitted last.
    pub(crate) fn wreck(
        &mut self,
        loc: Location,
        layer: Layer,
        how: Destruction,
        outcome: DamageOutcome,
        dir: Option<[Real; 3]>,
        leave_rubble: bool,
    ) -> Result<Option<Wreck>, WorldError> {
        let wrecked = self.wreck_thrown(loc, layer, how, outcome, dir, leave_rubble)?;
        Ok(wrecked.map(|(wreck, _)| wreck))
    }

    /// As [`wreck`](WorldEdit::wreck), returning with the matter lost
    /// the outcome as it stands once fragments have been thrown: the
    /// energy they carry off is taken out of what passed and counted
    /// as absorbed, so that the two still add up to the packet's
    /// energy (02 §12.1, invariant W8).
    ///
    /// Fragments are thrown only by a packet with a direction and
    /// energy to spare, and only where the wreck is left in the cell:
    /// whoever carries the wreck off takes all of it.
    pub(crate) fn wreck_thrown(
        &mut self,
        loc: Location,
        layer: Layer,
        how: Destruction,
        mut outcome: DamageOutcome,
        dir: Option<[Real; 3]>,
        leave_rubble: bool,
    ) -> Result<Option<(Wreck, DamageOutcome)>, WorldError> {
        let tick = self.ctx.timeline.now();
        let cause = self.cause;
        let defs = Arc::clone(self.ctx.cosmos.defs());
        let view = self.ctx.cosmos.grid(loc.grid)?;
        let loc = Location::new(loc.grid, view.normalize(loc.cell)?);
        let old = view.element(loc.cell, layer)?;
        if old.is_none() {
            return Ok(None);
        }
        let new = match how {
            Destruction::Fell => ElementState::NONE,
            Destruction::Damage(_) | Destruction::Crushed => defs
                .element(layer, old)
                .and_then(|element| element.on_destroy)
                .unwrap_or(ElementState::NONE)
                .normalised(),
        };
        let lost = (defs.bulk(layer, old) - defs.bulk(layer, new)).max(Real::ZERO);
        let mut wreck = Wreck {
            material: old.material,
            volume: lost,
            mass: lost * defs.density(old.material),
        };

        if new == old {
            // It becomes itself again, undamaged.
            self.set_damage(loc, layer, 0)?;
        } else {
            self.set_element(loc, layer, new)?;
        }

        if let Destruction::Damage(kind) = how
            && leave_rubble
            && outcome.passed > Real::ZERO
            && wreck.mass > Real::ZERO
            && !defs.is_rubble(layer, old)
        {
            let (thrown, carried) = self.ctx.throw_debris(&DebrisSpec {
                from: loc,
                layer,
                material: old.material,
                mass: wreck.mass,
                energy: outcome.passed,
                kind,
                dir,
                cause,
            });
            let thrown = thrown.clamp(Real::ZERO, wreck.mass);
            if thrown > Real::ZERO {
                wreck.volume *= Real::ONE - thrown / wreck.mass;
                wreck.mass -= thrown;
            }
            let carried = carried.clamp(Real::ZERO, outcome.passed);
            outcome.passed -= carried;
            outcome.absorbed += carried;
        }
        if leave_rubble && !defs.is_rubble(layer, old) {
            self.leave_rubble(loc, wreck.material, wreck.volume)?;
        }

        self.ctx.events.emit(WorldEvent::ElementDestroyed {
            tick,
            loc,
            layer,
            old,
            new,
            how,
            absorbed: outcome.absorbed,
            passed: outcome.passed,
            cause,
        });
        Ok(Some((wreck, outcome)))
    }

    /// Leaves a volume of a material as its rubble, where something
    /// dropped into the cell comes to rest. A material that names no
    /// rubble leaves nothing.
    pub(crate) fn leave_rubble(
        &mut self,
        at: Location,
        material: MaterialId,
        volume: Real,
    ) -> Result<(), WorldError> {
        let rubble = self
            .ctx
            .cosmos
            .defs()
            .materials()
            .get(material)
            .and_then(|material| material.rubble);
        let units = units_of(volume);
        match rubble {
            Some(fixture) if units > 0 => self.deposit_rubble(at, fixture, material, units),
            _ => Ok(()),
        }
    }

    /// Adds units of a rubble fixture to where something dropped into a
    /// cell comes to rest. Amounts add up. A cell that fills, and has no
    /// block, becomes the fixture's heap block, and what is over goes on
    /// top of it. A fixture of another kind in the way is buried: it
    /// joins the pile.
    fn deposit_rubble(
        &mut self,
        at: Location,
        fixture: FixtureId,
        material: MaterialId,
        units: u32,
    ) -> Result<(), WorldError> {
        let defs = Arc::clone(self.ctx.cosmos.defs());
        let heap = defs.fixtures().get(fixture).and_then(|def| def.heap);
        let mut at = self.ctx.cosmos.resting_cell(at)?;
        let mut left = units;
        let mut material = material;
        for _ in 0..RUBBLE_MAX_STEPS {
            let view = self.ctx.cosmos.grid(at.grid)?;
            let cell = view.cell(at.cell)?;
            if view.flags(at.cell)?.contains(CellFlags::SOLID_TOP) {
                // The cell is full. The pile goes on top of it.
                match view.normalize(at.cell.above()) {
                    Ok(above) if view.is_loaded(above) => {
                        at = Location::new(at.grid, above);
                        continue;
                    }
                    _ => return Ok(()),
                }
            }
            let there = cell.fixture;
            let have = if there.def() == fixture {
                defs.rubble_units(there).unwrap_or(0)
            } else {
                units_of(defs.bulk(Layer::Fixture, there.raw()))
            };
            // The pile is of whatever most of it is.
            if there.def() == fixture && have >= left {
                material = there.material();
            }
            let total = have.saturating_add(left);
            if let Some(heap) = heap
                && total >= RUBBLE_CELL_UNITS
                && cell.block.is_none()
            {
                self.set_fixture(at, FixtureState::NONE)?;
                self.set_block(at, BlockState::new(heap, material, 0))?;
                left = total - RUBBLE_CELL_UNITS;
                if left == 0 {
                    return Ok(());
                }
                continue;
            }
            let amount = u16::try_from(total).unwrap_or(u16::MAX).max(1);
            self.set_fixture(at, FixtureState::new(fixture, material, amount))?;
            return Ok(());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resist(threshold: f64, factor: f64) -> Resist {
        Resist {
            threshold: Real::of(threshold),
            factor: Real::of(factor),
        }
    }

    #[test]
    fn the_worked_examples_of_the_specification() {
        // 0.6 m³ of concrete: 0.6 MJ behind a 20 MPa impact threshold.
        let wall = resist(20.0e6, 1.0);
        let capacity = Real::of(0.6) * Real::of(1.0e6);

        // 80 kg at 20 m/s: 16 kJ, delivered at about 0.3 MPa.
        let energy = Real::of(0.5 * 80.0 * 20.0 * 20.0);
        assert_eq!(energy, Real::of(16.0e3));
        let (slow, damage) = resolve_packet(wall, capacity, 0, energy, Real::of(0.3e6));
        assert_eq!(slow, DamageOutcome::UNAFFECTED);
        assert_eq!(damage, 0);

        // 80 kg at 600 m/s: 14.4 MJ at about 290 MPa.
        let energy = Real::of(0.5 * 80.0 * 600.0 * 600.0);
        assert_eq!(energy, Real::of(14.4e6));
        let (fast, _) = resolve_packet(wall, capacity, 0, energy, Real::of(290.0e6));
        assert_eq!(fast.result, DamageResult::Destroyed);
        assert!((fast.absorbed.get() - 0.6e6).abs() < 1.0e-3, "{fast:?}");
        assert!((fast.passed.get() - 13.8e6).abs() < 1.0e-3, "{fast:?}");
        assert_eq!(fast.absorbed + fast.passed, energy);
    }

    #[test]
    fn below_the_threshold_nothing_happens_however_often() {
        let wall = resist(20.0e6, 1.0);
        let capacity = Real::of(0.6e6);
        let mut damage = 0;
        for _ in 0..10_000 {
            let (outcome, after) =
                resolve_packet(wall, capacity, damage, Real::of(1.0e9), Real::of(20.0e6));
            assert_eq!(outcome, DamageOutcome::UNAFFECTED);
            damage = after;
        }
        assert_eq!(damage, 0);
    }

    #[test]
    fn full_effect_is_reached_at_twice_the_threshold() {
        let wall = resist(10.0, 1.0);
        let capacity = Real::of(1000.0);
        let hit = |intensity: f64| {
            resolve_packet(wall, capacity, 0, Real::of(100.0), Real::of(intensity)).1
        };
        // Half way between: half the energy counts. 50 of 1000 J is
        // 12.75 parts of 255, rounded up.
        assert_eq!(hit(15.0), 13);
        // At twice the threshold and beyond, all of it: 25.5 parts.
        assert_eq!(hit(20.0), 26);
        assert_eq!(hit(2000.0), 26);
    }

    #[test]
    fn repeated_packets_destroy_at_the_predicted_count() {
        // Each packet is worth 26 parts of 255. The element is destroyed
        // by the first packet that is at least what remains: 100 J
        // against 1000 × (1 − 26n / 255), which is the tenth.
        let wall = resist(10.0, 1.0);
        let capacity = Real::of(1000.0);
        let mut damage = 0;
        let mut packets = 0;
        loop {
            packets += 1;
            let (outcome, after) =
                resolve_packet(wall, capacity, damage, Real::of(100.0), Real::of(50.0));
            if outcome.result == DamageResult::Destroyed {
                break;
            }
            assert_eq!(outcome.result, DamageResult::Damaged);
            assert_eq!(after, damage + 26);
            assert_eq!(outcome.absorbed, Real::of(100.0));
            assert_eq!(outcome.passed, Real::ZERO);
            damage = after;
        }
        assert_eq!(packets, 10);
        assert_eq!(damage, 234);
    }

    #[test]
    fn a_factor_scales_what_gets_through() {
        let tough = resist(10.0, 0.2);
        let capacity = Real::of(100.0);
        // 100 J counts as 20: a fifth of the capacity, 51 parts.
        let (outcome, damage) =
            resolve_packet(tough, capacity, 0, Real::of(100.0), Real::of(100.0));
        assert_eq!(outcome.result, DamageResult::Damaged);
        assert_eq!(damage, 51);
        // 1000 J counts as 200: twice the capacity, so half passes.
        let (outcome, _) = resolve_packet(tough, capacity, 0, Real::of(1000.0), Real::of(100.0));
        assert_eq!(outcome.result, DamageResult::Destroyed);
        assert_eq!(outcome.passed, Real::of(500.0));
        assert_eq!(outcome.absorbed, Real::of(500.0));
    }

    #[test]
    fn damage_stops_short_of_destruction_unless_the_rule_destroys() {
        let wall = resist(1.0, 1.0);
        let capacity = Real::of(255.0);
        // One part is left; half a joule is less than it, and rounds up
        // to the part: the element stays, at the most damage it can hold.
        let (outcome, damage) = resolve_packet(wall, capacity, 254, Real::of(0.5), Real::of(100.0));
        assert_eq!(outcome.result, DamageResult::Damaged);
        assert_eq!(damage, DAMAGE_MAX);
        // Something with no capacity is destroyed by anything that
        // affects it, and by nothing that does not.
        let (outcome, _) = resolve_packet(wall, Real::ZERO, 0, Real::of(5.0), Real::of(100.0));
        assert_eq!(outcome.result, DamageResult::Destroyed);
        assert_eq!(outcome.passed, Real::of(5.0));
        let (outcome, _) = resolve_packet(wall, Real::ZERO, 0, Real::of(5.0), Real::of(0.5));
        assert_eq!(outcome, DamageOutcome::UNAFFECTED);
        // No threshold at all: any intensity is enough, and none is not.
        let (outcome, _) =
            resolve_packet(Resist::NONE, capacity, 0, Real::of(5.0), Real::of(1.0e-9));
        assert_eq!(outcome.result, DamageResult::Damaged);
        let (outcome, _) = resolve_packet(Resist::NONE, capacity, 0, Real::of(5.0), Real::ZERO);
        assert_eq!(outcome, DamageOutcome::UNAFFECTED);
    }
}
