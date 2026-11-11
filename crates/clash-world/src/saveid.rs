//! Save ids: how saved state names definitions (02 §11).
//!
//! A definition's dense id is good for one loading of one set of mods.
//! Saved state therefore names a definition by its **save id**: a
//! position in a table of qualified ids that belongs to the save and
//! only ever grows. There is one table for each [`DefKind`].
//!
//! - Writing turns dense ids into save ids, adding to a table the first
//!   time a definition is written (so writing changes a [`SaveIds`]).
//! - Reading turns save ids back into the dense ids of the definitions
//!   loaded now. An id whose definition is no longer there is given a
//!   **placeholder** that keeps the id as text, so that loading the mod
//!   again finds the definition and what was made from it is restored
//!   (04 §12.3).
//!
//! ```
//! use clash_world::{DefKind, RawWorldDefs, SaveIds, WorldDefs};
//!
//! let defs = WorldDefs::build(RawWorldDefs::default()).unwrap();
//! let ids = SaveIds::new(&defs);
//! // Nothing has been written yet but the built-in "nothing here".
//! assert_eq!(ids.names(DefKind::Block), ["engine:none"]);
//! assert!(ids.names(DefKind::Realm).is_empty());
//! ```
//!
//! # Which state names definitions
//!
//! Everything the world saves is walked by `MapIds`, a trait of this
//! module, which hands each dense id it holds to an `IdMap`. A state
//! type that holds the id of a definition implements `MapIds`; a new
//! one must, or what it holds will mean something else after the mods
//! change.
//!
//! Left as they are, because their definition types are not built and
//! the world gives the numbers no meaning: `DecalId`, `ConduitId`,
//! `SiteDefId`, `TractDefId`, `DevastationKindId`, `MapId`,
//! `RegionKindId` and `PathKindId`. A named state of an element is its
//! position among the definition's states, and is written as that
//! number.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::area::AreaTable;
use crate::collapse::FallingMass;
use crate::cosmos::Realm;
use crate::defs::{
    AreaKindId, DefKind, ElementState, Layer, MaterialId, NONE_ID, RealmDefId, WorldDefs,
};
use crate::grid::ColumnImage;
use crate::process::{WorldActivity, WorldProcess};
use crate::store::StoreError;
use crate::topology::Topology;
use crate::tract::{PendingEdit, RegionRecord};

/// The mark of a definition that has no save id yet.
const UNSET: u32 = u32::MAX;

/// One kind's table: the qualified ids by save id, and the way between
/// save ids and the dense ids of the definitions loaded now.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct IdTable {
    /// The qualified id of each save id. Only ever added to.
    names: Vec<String>,
    /// The dense id each save id means now.
    to_dense: Vec<u32>,
    /// The save id of each dense id, or [`UNSET`].
    to_save: Vec<u32>,
}

impl IdTable {
    fn save_of(&self, dense: u32) -> Option<u32> {
        let found = *self.to_save.get(usize::try_from(dense).ok()?)?;
        (found != UNSET).then_some(found)
    }

    fn link(&mut self, save: u32, dense: u32) {
        let Ok(at) = usize::try_from(dense) else {
            return;
        };
        if self.to_save.len() <= at {
            self.to_save.resize(at + 1, UNSET);
        }
        self.to_save[at] = save;
    }
}

/// A definition that a save refers to and no loaded mod supplies. It
/// has been given a placeholder (04 §12.3).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct MissingDef {
    /// The definition type.
    pub kind: DefKind,
    /// The qualified id the save wrote.
    pub id: String,
}

/// The tables of a save's ids, as they are stored: each kind's name and
/// the qualified ids in save-id order. Tables are found by name, so a
/// build that knows more kinds reads the saves of one that knew fewer.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct SaveIdsImage {
    tables: Vec<(String, Vec<String>)>,
}

/// The save-wide tables of qualified ids that saved state names
/// definitions through (02 §11). One for each [`DefKind`]; they only
/// ever grow.
///
/// A cosmos keeps the tables of its save. They are not state: two
/// worlds that hold the same things hash equal whatever their tables
/// say. They are written with the world and must be read back with it,
/// because every tract blob in the store is written against them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SaveIds {
    tables: [IdTable; DefKind::COUNT],
    /// The tables of kinds this build does not know, kept as they were
    /// read so that saving does not lose them.
    other: Vec<(String, Vec<String>)>,
}

impl SaveIds {
    /// The tables of a save that has written nothing yet. Save id 0 of
    /// each element type and of materials is the built-in `engine:none`,
    /// as dense id 0 is.
    pub fn new(defs: &WorldDefs) -> SaveIds {
        let mut ids = SaveIds {
            tables: Default::default(),
            other: Vec::new(),
        };
        for kind in DefKind::ALL {
            if kind.has_none() {
                ids.intern(defs, kind, 0);
            }
        }
        ids
    }

    /// The qualified ids of a kind, by save id.
    pub fn names(&self, kind: DefKind) -> &[String] {
        &self.tables[kind.index()].names
    }

    /// How many ids there are in all.
    pub fn len(&self) -> usize {
        self.tables.iter().map(|table| table.names.len()).sum()
    }

    /// Whether there are no ids at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The save id of a definition, added to the kind's table if the
    /// save has not named the definition before. `None` for a dense id
    /// that names no definition, and when the table is full.
    pub fn intern(&mut self, defs: &WorldDefs, kind: DefKind, dense: u32) -> Option<u32> {
        if let Some(save) = self.tables[kind.index()].save_of(dense) {
            return Some(save);
        }
        let name = defs.def_name(kind, dense)?;
        let table = &mut self.tables[kind.index()];
        // A definition that has come back under the id a placeholder
        // keeps is the same entry.
        let save = match table.names.iter().position(|have| have == name) {
            Some(at) => {
                table.to_dense[at] = dense;
                u32::try_from(at).ok()?
            }
            None => {
                let save = u32::try_from(table.names.len()).ok()?;
                if save == UNSET {
                    return None;
                }
                table.names.push(name.to_owned());
                table.to_dense.push(dense);
                save
            }
        };
        table.link(save, dense);
        Some(save)
    }

    /// The dense id a save id means now. `None` for a save id the table
    /// does not hold: what named it is damaged.
    pub fn dense(&self, kind: DefKind, save: u32) -> Option<u32> {
        self.tables[kind.index()]
            .to_dense
            .get(usize::try_from(save).ok()?)
            .copied()
    }

    /// The tables as they are stored.
    pub fn image(&self) -> SaveIdsImage {
        let mut tables: Vec<(String, Vec<String>)> = DefKind::ALL
            .into_iter()
            .map(|kind| (kind.name().to_owned(), self.names(kind).to_vec()))
            .collect();
        tables.extend(self.other.iter().cloned());
        SaveIdsImage { tables }
    }

    /// The tables of a save, bound to the definitions loaded now.
    ///
    /// Each id is looked up among the definitions. One that is not
    /// there is given a placeholder, added to a copy of the definitions:
    /// the definitions returned are the ones the world must use from
    /// here on, and hold every definition of those passed in under the
    /// same dense id. `realm_shapes` gives the extent of each realm
    /// definition the save uses, by save id, for the placeholder of a
    /// realm definition that has gone.
    ///
    /// Migration entries (04 §12.1) are consulted here, before a
    /// placeholder is made, once the `migration` definition type exists.
    pub fn bind(
        image: SaveIdsImage,
        defs: &Arc<WorldDefs>,
        realm_shapes: &BTreeMap<u32, Topology>,
    ) -> Result<(SaveIds, Arc<WorldDefs>, Vec<MissingDef>), StoreError> {
        let mut ids = SaveIds {
            tables: Default::default(),
            other: Vec::new(),
        };
        let mut extended: Option<WorldDefs> = None;
        let mut missing = Vec::new();
        for (name, names) in image.tables {
            let Some(kind) = DefKind::from_name(&name) else {
                ids.other.push((name, names));
                continue;
            };
            if !ids.tables[kind.index()].names.is_empty() {
                return Err(StoreError::Malformed("a kind of definition has two tables"));
            }
            if kind.has_none() && names.first().map(String::as_str) != Some(NONE_ID) {
                return Err(StoreError::Malformed(
                    "a table of ids does not begin with the built-in none",
                ));
            }
            for (save, name) in names.iter().enumerate() {
                let save = u32::try_from(save)
                    .map_err(|_| StoreError::Malformed("a table of ids is too long"))?;
                let current: &WorldDefs = extended.as_ref().unwrap_or(defs);
                let dense = match current.def_lookup(kind, name) {
                    Some(dense) => dense,
                    None => {
                        let made = extended
                            .get_or_insert_with(|| WorldDefs::clone(defs))
                            .add_placeholder(kind, name, realm_shapes.get(&save).copied())
                            .ok_or(StoreError::TooManyDefinitions(kind))?;
                        missing.push(MissingDef {
                            kind,
                            id: name.clone(),
                        });
                        made
                    }
                };
                let table = &mut ids.tables[kind.index()];
                if table.save_of(dense).is_some() {
                    return Err(StoreError::Malformed("a table of ids holds an id twice"));
                }
                table.to_dense.push(dense);
                table.link(save, dense);
            }
            ids.tables[kind.index()].names = names;
        }
        // A save written before a kind had a table.
        let defs = extended.map_or_else(|| Arc::clone(defs), Arc::new);
        for kind in DefKind::ALL {
            if kind.has_none() && ids.tables[kind.index()].names.is_empty() {
                ids.intern(&defs, kind, 0);
            }
        }
        Ok((ids, defs, missing))
    }
}

/// Turns the dense ids of definitions into something else and back:
/// into save ids when state is written, into the dense ids of the
/// definitions loaded now when it is read.
pub(crate) trait IdMap {
    /// What an id of a kind becomes.
    fn map(&mut self, kind: DefKind, id: u32) -> u32;
    /// Notes that an id could not be turned: the state walked is not to
    /// be used.
    fn fail(&mut self);
}

/// An id that is stored in sixteen bits, turned.
fn map16(map: &mut dyn IdMap, kind: DefKind, id: u16) -> u16 {
    match u16::try_from(map.map(kind, u32::from(id))) {
        Ok(turned) => turned,
        Err(_) => {
            map.fail();
            0
        }
    }
}

/// An element state with its definition and material turned. No element
/// stays no element.
pub(crate) fn map_state(map: &mut dyn IdMap, layer: Layer, state: ElementState) -> ElementState {
    if state.is_none() {
        return ElementState::NONE;
    }
    ElementState {
        def: map16(map, DefKind::of_layer(layer), state.def),
        material: MaterialId::new(map16(map, DefKind::Material, state.material.raw())),
        bits: state.bits,
    }
}

/// Dense ids into save ids, for writing. Adds to the tables.
pub(crate) struct ToSave<'a> {
    defs: &'a WorldDefs,
    ids: &'a mut SaveIds,
    failed: bool,
}

impl<'a> ToSave<'a> {
    pub(crate) fn new(defs: &'a WorldDefs, ids: &'a mut SaveIds) -> ToSave<'a> {
        ToSave {
            defs,
            ids,
            failed: false,
        }
    }

    /// Whether every id was turned. An id that was not names no
    /// definition, or its table is full.
    pub(crate) fn finish(self) -> Result<(), StoreError> {
        if self.failed {
            Err(StoreError::Malformed(
                "state names a definition that does not exist, or a table of ids is full",
            ))
        } else {
            Ok(())
        }
    }
}

impl IdMap for ToSave<'_> {
    fn map(&mut self, kind: DefKind, id: u32) -> u32 {
        match self.ids.intern(self.defs, kind, id) {
            Some(save) => save,
            None => {
                self.failed = true;
                0
            }
        }
    }

    fn fail(&mut self) {
        self.failed = true;
    }
}

/// Save ids into the dense ids of the definitions loaded now, for
/// reading. Notes the placeholders it meets.
pub(crate) struct ToDense<'a> {
    defs: &'a WorldDefs,
    ids: &'a SaveIds,
    failed: bool,
    met: Vec<(DefKind, u32)>,
}

impl<'a> ToDense<'a> {
    pub(crate) fn new(defs: &'a WorldDefs, ids: &'a SaveIds) -> ToDense<'a> {
        ToDense {
            defs,
            ids,
            failed: false,
            met: Vec::new(),
        }
    }

    /// The placeholders met, each once, in the order they were met; or
    /// an error if a save id was not in its table.
    pub(crate) fn finish(self) -> Result<Vec<MissingDef>, StoreError> {
        if self.failed {
            return Err(StoreError::Malformed(
                "state names a definition by a save id its table does not hold",
            ));
        }
        Ok(self
            .met
            .into_iter()
            .filter_map(|(kind, dense)| {
                Some(MissingDef {
                    kind,
                    id: self.defs.def_name(kind, dense)?.to_owned(),
                })
            })
            .collect())
    }
}

impl IdMap for ToDense<'_> {
    fn map(&mut self, kind: DefKind, id: u32) -> u32 {
        match self.ids.dense(kind, id) {
            Some(dense) => {
                if self.defs.is_placeholder(kind, dense) && !self.met.contains(&(kind, dense)) {
                    self.met.push((kind, dense));
                }
                dense
            }
            None => {
                self.failed = true;
                0
            }
        }
    }

    fn fail(&mut self) {
        self.failed = true;
    }
}

/// State that names definitions by dense id, and can have every such
/// id turned (see the module documentation).
pub(crate) trait MapIds {
    /// Hands every id of a definition held to the map and keeps what it
    /// gives back.
    fn map_ids(&mut self, map: &mut dyn IdMap);
}

impl MapIds for ColumnImage {
    fn map_ids(&mut self, map: &mut dyn IdMap) {
        self.map_states(&mut |layer, state| map_state(map, layer, state));
    }
}

impl MapIds for RegionRecord {
    fn map_ids(&mut self, map: &mut dyn IdMap) {
        for record in self.tracts.iter_mut() {
            let Some(extra) = record.extra.as_mut() else {
                continue;
            };
            for edit in &mut extra.pending {
                if let PendingEdit::Cells(cells) = edit {
                    for change in cells {
                        change.state = map_state(map, change.layer, change.state);
                    }
                }
            }
        }
    }
}

impl MapIds for Realm {
    fn map_ids(&mut self, map: &mut dyn IdMap) {
        self.def = RealmDefId::new(map16(map, DefKind::Realm, self.def.raw()));
        for region in self.regions.values_mut() {
            region.map_ids(map);
        }
    }
}

impl MapIds for AreaTable {
    fn map_ids(&mut self, map: &mut dyn IdMap) {
        self.map_kinds(&mut |kind| AreaKindId::new(map16(map, DefKind::AreaKind, kind.raw())));
    }
}

impl MapIds for FallingMass {
    fn map_ids(&mut self, map: &mut dyn IdMap) {
        for part in &mut self.parts {
            part.material = MaterialId::new(map16(map, DefKind::Material, part.material.raw()));
        }
        // Its parts are kept in order of material.
        self.parts.sort_by_key(|part| part.material);
    }
}

impl MapIds for WorldProcess {
    // No wildcard arm: a new kind of process must say which definitions
    // it names, or name none.
    fn map_ids(&mut self, map: &mut dyn IdMap) {
        match self {
            WorldProcess::Settle(_) => {}
            WorldProcess::Fall(mass) => mass.map_ids(map),
            // A field step names a clock and an interval, no definition.
            WorldProcess::FieldStep(_) => {}
        }
    }
}

impl MapIds for WorldActivity {
    fn map_ids(&mut self, map: &mut dyn IdMap) {
        let ids: Vec<_> = self.processes.ids().collect();
        for id in ids {
            if let Some(process) = self.processes.get_mut(id) {
                process.map_ids(map);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::RawWorldDefs;

    fn defs() -> Arc<WorldDefs> {
        Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap_or_else(|_| unreachable!()))
    }

    #[test]
    fn a_missing_id_becomes_a_placeholder_and_keeps_its_place() {
        let defs = defs();
        let mut ids = SaveIds::new(&defs);
        assert_eq!(ids.intern(&defs, DefKind::Block, 0), Some(0));
        assert_eq!(ids.intern(&defs, DefKind::Block, 7), None);
        assert_eq!(ids.dense(DefKind::Block, 0), Some(0));
        assert_eq!(ids.dense(DefKind::Block, 1), None);

        // A save that named two blocks these definitions do not have.
        let mut image = ids.image();
        for (name, names) in &mut image.tables {
            if name == "block" {
                names.push("gone:gate".to_owned());
                names.push("gone:arch".to_owned());
            }
        }
        image
            .tables
            .push(("later_kind".to_owned(), vec!["x:y".to_owned()]));
        let (bound, extended, missing) =
            SaveIds::bind(image.clone(), &defs, &BTreeMap::new()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            missing,
            [
                MissingDef {
                    kind: DefKind::Block,
                    id: "gone:gate".to_owned()
                },
                MissingDef {
                    kind: DefKind::Block,
                    id: "gone:arch".to_owned()
                }
            ]
        );
        assert_eq!(bound.dense(DefKind::Block, 1), Some(1));
        assert_eq!(bound.dense(DefKind::Block, 2), Some(2));
        assert!(extended.is_placeholder(DefKind::Block, 2));
        assert_eq!(extended.def_name(DefKind::Block, 2), Some("gone:arch"));
        // Written again, the placeholder has the save id it came from,
        // and a table this build does not know is kept.
        let mut again = bound.clone();
        assert_eq!(again.intern(&extended, DefKind::Block, 2), Some(2));
        assert_eq!(again.image(), image);

        // Mapping a state both ways.
        let state = ElementState {
            def: 2,
            material: MaterialId::new(0),
            bits: 5,
        };
        let mut to_save = ToSave::new(&extended, &mut again);
        let saved = map_state(&mut to_save, Layer::Block, state);
        assert!(to_save.finish().is_ok());
        let mut to_dense = ToDense::new(&extended, &bound);
        assert_eq!(map_state(&mut to_dense, Layer::Block, saved), state);
        assert_eq!(
            to_dense.finish().map(|met| met.len()),
            Ok(1),
            "the placeholder is noted"
        );
        // A save id past the table is damage, not a panic.
        let mut to_dense = ToDense::new(&extended, &bound);
        let wild = ElementState { def: 900, ..state };
        map_state(&mut to_dense, Layer::Block, wild);
        assert!(to_dense.finish().is_err());
    }

    #[test]
    fn tables_that_cannot_be_are_refused() {
        let defs = defs();
        let good = SaveIds::new(&defs).image();
        let mut twice = good.clone();
        twice.tables.push(twice.tables[0].clone());
        assert!(SaveIds::bind(twice, &defs, &BTreeMap::new()).is_err());
        let mut headless = good.clone();
        headless.tables[1].1[0] = "core:wall".to_owned();
        assert!(SaveIds::bind(headless, &defs, &BTreeMap::new()).is_err());
        let mut doubled = good;
        doubled.tables[1].1.push(NONE_ID.to_owned());
        assert!(SaveIds::bind(doubled, &defs, &BTreeMap::new()).is_err());
    }
}
