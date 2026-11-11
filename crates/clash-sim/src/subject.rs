//! Subjects as the expression language sees them: kinds and handles
//! (05 §5.1).
//!
//! Rules never see raw state. They see **subjects**, each of a known
//! **kind**, and ask them for properties. Inside the simulation a
//! subject is a [`Subject`]; in an expression it is a
//! [`Handle`]: the kind's number in the registry and sixty-four bits
//! that only this module reads. [`Kinds::handle`] and [`Kinds::subject`]
//! convert between the two.
//!
//! # The kinds the simulation serves
//!
//! | Kind | Refers to | The 64 bits of its handle |
//! |------|-----------|---------------------------|
//! | `entity` | Any entity | `EntityId::to_bits`: the slot and the generation |
//! | `loc` | A cell | The location, packed: see below |
//! | `element` | The floor, block or fixture of a cell | The location packed as for `loc`, with the layer in the lowest two bits: 1 floor, 2 block, 3 fixture |
//! | `area` | An area | The area's id |
//! | `site` | A site | The site's id |
//! | `tract` | A tract of a realm, loaded or not | The realm's id in the top 16 bits, then the tract's `x` and `y`, 24 bits each, two's complement |
//! | `realm` | A realm | The realm's id |
//! | `global` | The whole simulation | Zero |
//!
//! A location is packed, from the top bit down, as: the grid's id in
//! [`GRID_BITS`] bits; `x` and `y` in [`XY_BITS`] bits each and `z` in
//! [`Z_BITS`] bits, two's complement; and two bits that are zero for a
//! `loc`. So a handle can name a cell of the first 1024 grids, within
//! 1,048,576 cells of the origin east, west, north and south and within
//! 512 storeys of it. A cell beyond that has no handle and reads as
//! null. A tract's handle reaches 8,388,608 tracts each way in the
//! first 65,536 realms.
//!
//! # Handles never dangle
//!
//! A handle is a value: it may be stored in a variable and outlive what
//! it names. Reading through one that no longer names anything gives
//! null and never another thing. An entity's handle carries its
//! generation, so the entity that later takes its slot is not found
//! through it; ids of areas, sites and realms are never reused; a cell,
//! a tract and the simulation always exist.
//!
//! The kinds of 05 §5.1 that are not here (`region`, `subgrid`,
//! `network`, `job`, `ability`, `group`, `incident`, `persona`, `track`,
//! `memory`, `case`, `mission`, `objective`, `action`, `part`, `wound`,
//! `body`, `window`) are registered by the packages that build what
//! they refer to.

use clash_core::EntityId;
use clash_expr::{Handle, KindId, KindSpec, Registry, Value};
use clash_world::{AreaId, CellPos, GridId, Layer, Location, RealmId, SiteId, TractPos};

use crate::event::Subject;

/// How many bits of a location's handle hold the grid's id.
pub const GRID_BITS: u32 = 10;
/// How many bits of a location's handle hold each of `x` and `y`.
pub const XY_BITS: u32 = 21;
/// How many bits of a location's handle hold `z`.
pub const Z_BITS: u32 = 10;
/// How many bits of a location's handle say which element is meant:
/// zero for the cell itself.
pub const LAYER_BITS: u32 = 2;
/// How many bits of a tract's handle hold the realm's id.
pub const TRACT_REALM_BITS: u32 = 16;
/// How many bits of a tract's handle hold each of its coordinates.
pub const TRACT_XY_BITS: u32 = 24;

const _: () = assert!(GRID_BITS + 2 * XY_BITS + Z_BITS + LAYER_BITS == u64::BITS);
const _: () = assert!(TRACT_REALM_BITS + 2 * TRACT_XY_BITS == u64::BITS);

/// The kinds of subject the simulation serves, with what each refers to:
/// the name rules write and the line of the reference.
pub const SUBJECT_KIND_DOCS: &[(&str, &str)] = &[
    (
        "entity",
        "Any entity: a creature, an item, a device (06). A reference to one that has been removed reads as null.",
    ),
    (
        "loc",
        "A cell of a grid (02 §3.2). It need not be loaded: what cannot be read of it is null.",
    ),
    (
        "element",
        "The floor, block or fixture of a cell (16 §3.5), reached as `here.floor`, `here.block` and `here.fixture`.",
    ),
    (
        "area",
        "A named volume with tags, variables and optionally a clock of its own (02 §8.3).",
    ),
    (
        "site",
        "A named place with identity, spanning one or more tracts (02 §8.4).",
    ),
    (
        "tract",
        "A tract of a realm, loaded or not: what the engine knows of it is its record (02 §9.1).",
    ),
    ("realm", "One contiguous space with its own rules (02 §4)."),
    (
        "global",
        "The whole simulation: where global variables live (05 §6).",
    ),
];

/// The ids the registry gave the simulation's kinds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Kinds {
    /// `entity`.
    pub entity: KindId,
    /// `loc`.
    pub loc: KindId,
    /// `element`.
    pub element: KindId,
    /// `area`.
    pub area: KindId,
    /// `site`.
    pub site: KindId,
    /// `tract`.
    pub tract: KindId,
    /// `realm`.
    pub realm: KindId,
    /// `global`.
    pub global: KindId,
}

/// Packs a signed number into the low `bits` bits, if it fits.
fn pack(value: i32, bits: u32) -> Option<u64> {
    let half = 1_i64 << (bits - 1);
    let value = i64::from(value);
    (-half..half)
        .contains(&value)
        .then(|| (value as u64) & ((1 << bits) - 1))
}

/// Reads a signed number from the low `bits` bits.
fn unpack(packed: u64, bits: u32) -> i32 {
    let shift = u64::BITS - bits;
    (((packed << shift) as i64) >> shift) as i32
}

/// A location and what is meant at it, as sixty-four bits. `None` if it
/// lies beyond what a handle can name.
fn pack_loc(loc: Location, layer: u64) -> Option<u64> {
    let grid = u64::from(loc.grid.raw());
    if grid >= 1 << GRID_BITS {
        return None;
    }
    let (x, y) = (pack(loc.cell.x, XY_BITS)?, pack(loc.cell.y, XY_BITS)?);
    let z = pack(loc.cell.z, Z_BITS)?;
    Some(
        (grid << (2 * XY_BITS + Z_BITS + LAYER_BITS))
            | (x << (XY_BITS + Z_BITS + LAYER_BITS))
            | (y << (Z_BITS + LAYER_BITS))
            | (z << LAYER_BITS)
            | layer,
    )
}

/// The location in sixty-four bits packed by [`pack_loc`], and the two
/// bits that say what is meant at it.
fn unpack_loc(payload: u64) -> (Location, u64) {
    let grid = (payload >> (2 * XY_BITS + Z_BITS + LAYER_BITS)) as u32;
    let x = unpack(payload >> (XY_BITS + Z_BITS + LAYER_BITS), XY_BITS);
    let y = unpack(payload >> (Z_BITS + LAYER_BITS), XY_BITS);
    let z = unpack(payload >> LAYER_BITS, Z_BITS);
    (
        Location::new(GridId::new(grid), CellPos::new(x, y, z)),
        payload & ((1 << LAYER_BITS) - 1),
    )
}

impl Kinds {
    /// The simulation's kinds in a registry, each registered if it was
    /// not. `None` only if the registry can hold no more kinds.
    pub fn ensure(registry: &mut Registry) -> Option<Kinds> {
        for (name, doc) in SUBJECT_KIND_DOCS {
            if registry.kind_id(name).is_none() {
                registry.add_kind(KindSpec::new(*name).doc(*doc)).ok()?;
            }
        }
        Kinds::of(registry)
    }

    /// The simulation's kinds in a registry that holds them all.
    pub fn of(registry: &Registry) -> Option<Kinds> {
        Some(Kinds {
            entity: registry.kind_id("entity")?,
            loc: registry.kind_id("loc")?,
            element: registry.kind_id("element")?,
            area: registry.kind_id("area")?,
            site: registry.kind_id("site")?,
            tract: registry.kind_id("tract")?,
            realm: registry.kind_id("realm")?,
            global: registry.kind_id("global")?,
        })
    }

    /// The kind of a subject.
    pub const fn kind_of(&self, subject: &Subject) -> KindId {
        match subject {
            Subject::Entity(_) => self.entity,
            Subject::Loc(_) => self.loc,
            Subject::Element(..) => self.element,
            Subject::Area(_) => self.area,
            Subject::Site(_) => self.site,
            Subject::Tract(..) => self.tract,
            Subject::Realm(_) => self.realm,
            Subject::Global => self.global,
        }
    }

    /// The handle of a subject, for an expression. `None` for a cell or
    /// a tract that lies beyond what a handle can name (see the
    /// [module documentation](self)).
    pub fn handle(&self, subject: Subject) -> Option<Handle> {
        let payload = match subject {
            Subject::Entity(id) => id.to_bits(),
            Subject::Loc(loc) => pack_loc(loc, 0)?,
            Subject::Element(loc, layer) => pack_loc(loc, 1 + layer.index() as u64)?,
            Subject::Area(id) => u64::from(id.raw()),
            Subject::Site(id) => u64::from(id.raw()),
            Subject::Tract(realm, tract) => {
                let realm = u64::from(realm.raw());
                if realm >= 1 << TRACT_REALM_BITS {
                    return None;
                }
                (realm << (2 * TRACT_XY_BITS))
                    | (pack(tract.x, TRACT_XY_BITS)? << TRACT_XY_BITS)
                    | pack(tract.y, TRACT_XY_BITS)?
            }
            Subject::Realm(id) => u64::from(id.raw()),
            Subject::Global => 0,
        };
        Some(Handle::new(self.kind_of(&subject), payload))
    }

    /// The subject as a value: its handle, or null where it has none.
    pub fn value(&self, subject: Subject) -> Value {
        self.handle(subject).map_or(Value::Null, Value::Subject)
    }

    /// What a handle names. `None` for a handle of a kind that is not
    /// the simulation's, or one whose bits no subject has. Whether what
    /// it names still exists is for whoever reads it to find out.
    pub fn subject(&self, handle: Handle) -> Option<Subject> {
        let payload = handle.payload;
        let small = || u32::try_from(payload).ok();
        Some(match handle.kind {
            kind if kind == self.entity => Subject::Entity(EntityId::from_bits(payload)),
            kind if kind == self.loc => {
                let (loc, layer) = unpack_loc(payload);
                if layer != 0 {
                    return None;
                }
                Subject::Loc(loc)
            }
            kind if kind == self.element => {
                let (loc, layer) = unpack_loc(payload);
                let layer = *Layer::ALL.get(usize::try_from(layer).ok()?.checked_sub(1)?)?;
                Subject::Element(loc, layer)
            }
            kind if kind == self.area => Subject::Area(AreaId::new(small()?)),
            kind if kind == self.site => Subject::Site(SiteId::new(small()?)),
            kind if kind == self.tract => {
                let realm = (payload >> (2 * TRACT_XY_BITS)) as u32;
                let x = unpack(payload >> TRACT_XY_BITS, TRACT_XY_BITS);
                let y = unpack(payload, TRACT_XY_BITS);
                Subject::Tract(RealmId::new(realm), TractPos::new(x, y))
            }
            kind if kind == self.realm => Subject::Realm(RealmId::new(small()?)),
            kind if kind == self.global => Subject::Global,
            _ => return None,
        })
    }

    /// What a value names, if it is a subject of the simulation's.
    pub fn subject_of(&self, value: &Value) -> Option<Subject> {
        self.subject(value.as_subject()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds() -> (Registry, Kinds) {
        let mut registry = Registry::with_builtins();
        let kinds = Kinds::ensure(&mut registry).unwrap();
        // Asking again registers nothing twice.
        assert_eq!(Kinds::ensure(&mut registry), Some(kinds));
        assert_eq!(registry.kinds().count(), SUBJECT_KIND_DOCS.len());
        (registry, kinds)
    }

    #[test]
    fn every_subject_goes_to_a_handle_and_back() {
        let (registry, kinds) = kinds();
        let grid = GridId::new(3);
        let subjects = [
            Subject::Entity(EntityId::new(7, 2)),
            Subject::Loc(Location::new(grid, CellPos::new(0, 0, 0))),
            Subject::Loc(Location::new(grid, CellPos::new(-1, 40, -16))),
            Subject::Loc(Location::new(
                GridId::new(1023),
                CellPos::new((1 << 20) - 1, -(1 << 20), 511),
            )),
            Subject::Element(Location::new(grid, CellPos::new(5, -6, 7)), Layer::Floor),
            Subject::Element(Location::new(grid, CellPos::new(5, -6, 7)), Layer::Block),
            Subject::Element(Location::new(grid, CellPos::new(5, -6, -7)), Layer::Fixture),
            Subject::Area(AreaId::new(12)),
            Subject::Site(SiteId::new(4)),
            Subject::Tract(RealmId::new(2), TractPos::new(-3, 63)),
            Subject::Tract(
                RealmId::new(65_535),
                TractPos::new((1 << 23) - 1, -(1 << 23)),
            ),
            Subject::Realm(RealmId::new(1)),
            Subject::Global,
        ];
        for subject in subjects {
            let handle = kinds.handle(subject).unwrap();
            assert_eq!(kinds.subject(handle), Some(subject), "{subject:?}");
            assert_eq!(
                registry
                    .kind(handle.kind)
                    .map(|kind| kind.spec.name.as_str()),
                Some(subject.kind())
            );
            assert_eq!(kinds.subject_of(&kinds.value(subject)), Some(subject));
        }
        // Two subjects never share a handle.
        let mut handles: Vec<Handle> = subjects
            .iter()
            .filter_map(|subject| kinds.handle(*subject))
            .collect();
        handles.sort();
        handles.dedup();
        assert_eq!(handles.len(), subjects.len());
    }

    #[test]
    fn what_a_handle_cannot_name_has_none_and_is_null() {
        let (_, kinds) = kinds();
        let far = [
            Location::new(GridId::new(1024), CellPos::new(0, 0, 0)),
            Location::new(GridId::new(0), CellPos::new(1 << 20, 0, 0)),
            Location::new(GridId::new(0), CellPos::new(0, -(1 << 20) - 1, 0)),
            Location::new(GridId::new(0), CellPos::new(0, 0, 512)),
        ];
        for loc in far {
            assert_eq!(kinds.handle(Subject::Loc(loc)), None, "{loc}");
            assert_eq!(
                kinds.value(Subject::Element(loc, Layer::Block)),
                Value::Null
            );
        }
        let tract = Subject::Tract(RealmId::new(65_536), TractPos::new(0, 0));
        assert_eq!(kinds.handle(tract), None);
        let tract = Subject::Tract(RealmId::new(0), TractPos::new(1 << 23, 0));
        assert_eq!(kinds.handle(tract), None);

        // Bits no subject has, and kinds that are not the simulation's.
        assert_eq!(kinds.subject(Handle::new(kinds.loc, 1)), None);
        assert_eq!(kinds.subject(Handle::new(kinds.element, 0)), None);
        assert_eq!(kinds.subject(Handle::new(kinds.area, u64::MAX)), None);
        assert_eq!(kinds.subject(Handle::new(KindId::new(999), 0)), None);
        assert_eq!(kinds.subject_of(&Value::from(3)), None);
    }
}
