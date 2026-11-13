//! Furnishing (10 §3.2, §4.1; 06 §9.3; 16 §4.3): what the maps of a
//! tract put in the world besides cells and creatures, made when the
//! tract is first loaded.
//!
//! A placed map hands on, as spawn requests of the kinds `item`,
//! `table` and `device` (`clash_gen::place::PlacedMap::furnishings`),
//! the items its cell specs name and the settings they give devices.
//! The world hands them over once, the first time the tract is loaded
//! (02 §9.4), and the simulation answers here:
//!
//! | The request | Becomes |
//! |-------------|---------|
//! | Items or a table, in a cell whose element is a container (its device `holds` things) | The element's **latent contents**: kept here, keyed by the element, with the seed they are drawn from, and made the first time the element becomes a device (opened, searched, something put in, a rule's `promote`, a packet that damages it) or `materialise_contents` is called on it. While latent they cost nothing and nothing knows what they are. |
//! | Items or a table, anywhere else | Items lying in the cell, made now |
//! | A device with settings | The element is promoted now and given them: its settings differ from its definition's, so it has state of its own (16 §4.3). A device a map only names stays a plain element |
//!
//! What items a request gives depends on its seed alone, which
//! generation drew from the placement and the cell, never from a stream
//! of the simulation: the same seed gives the same things whichever
//! tract loads first and however often the game is saved and loaded.
//! Requests are handed over once, so nothing is made twice: what was
//! made is in the world, shelved with its tract when the tract unloads
//! (02 §9.5), and what is still latent stays here until it is opened.
//!
//! A latent container whose element is replaced or destroyed (any
//! element of another definition, or none, in its place) spills its
//! contents where something dropped into the cell comes to rest, as a
//! device's contents do (16 §4.3).

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{EntityId, Rng, StateHash, StateHasher, Value as Stored};
use clash_data::{CrossCx, Def, DefLabel, Defs, Diagnostic, code};
use clash_gen::map::{EntryDef, MapDef, PaletteDef, PaletteItem};
use clash_gen::place::{
    SPAWN_KIND_DEVICE, SPAWN_KIND_ITEM, SPAWN_KIND_TABLE, SPAWN_PARAM_CONFIG, SPAWN_PARAM_COUNT,
    SPAWN_PARAM_KIND, SPAWN_PARAM_MAP, SPAWN_PARAM_SEED,
};
use clash_world::{Cause, CauseKind, ElementState, Layer, Location, SpawnRequest, WorldEvent};
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::bus::EventPayload;
use crate::device::{DeviceDef, device_at, element_device, promote};
use crate::entity::Place;
use crate::event::{EventBody, SimEvent, Subject, Where};
use crate::item::{
    CONTENTS_MATERIALISED, ContentTableDef, ContentTableId, ItemDef, ItemId, create_seeded,
};
use crate::reason::Reason;
use crate::registry::PropertyDef;

/// The cause of what is made when a tract is first loaded.
const FURNISH_CAUSE: Cause = Cause::new(CauseKind::Coarse);

/// The layers a cell's container is looked for in, in order.
const CONTAINER_LAYERS: [Layer; 3] = [Layer::Fixture, Layer::Block, Layer::Floor];

/// Where a lot of items comes from: one definition, or a content table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Source {
    /// Items of one definition.
    Item(ItemId),
    /// Draws from a content table (06 §9.3).
    Table(ContentTableId),
}

impl StateHash for Source {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Source::Item(id) => {
                0_u8.state_hash(hasher);
                id.state_hash(hasher);
            }
            Source::Table(id) => {
                1_u8.state_hash(hasher);
                id.state_hash(hasher);
            }
        }
    }
}

/// One lot of latent contents: what, how many (items, or draws from a
/// table), and the seed they are drawn from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Draw {
    /// What.
    pub source: Source,
    /// How many items, or how many draws.
    pub count: u32,
    /// What they are drawn from.
    pub seed: u64,
}

clash_core::impl_state_hash!(Draw {
    source,
    count,
    seed
});

/// The latent contents of one element (06 §9.3): the definition of the
/// element they belong to, and the lots, in the order maps gave them.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Latent {
    /// The number of the element's definition within its layer: if
    /// another stands there, the container is gone.
    pub def: u16,
    /// The lots.
    pub draws: Vec<Draw>,
}

clash_core::impl_state_hash!(Latent { def, draws });

/// What furnishing keeps: the latent contents of elements, by element.
/// State: saved and hashed.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct FurnishState {
    latent: BTreeMap<(Location, Layer), Latent>,
}

impl StateHash for FurnishState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.latent.state_hash(hasher);
    }
}

impl FurnishState {
    /// Whether nothing is latent: a simulation that was never furnished
    /// hashes as one from before furnishing was kept.
    pub fn is_empty(&self) -> bool {
        self.latent.is_empty()
    }

    /// The latent contents of an element, if it has any.
    pub fn latent(&self, loc: Location, layer: Layer) -> Option<&Latent> {
        self.latent.get(&(loc, layer))
    }

    /// How many elements hold latent contents.
    pub fn len(&self) -> usize {
        self.latent.len()
    }
}

// ---- Answering requests ----------------------------------------------------------

/// Answers a loaded tract's request if it is a furnishing (an item, a
/// table or a device), and says whether it was. A request that cannot
/// be answered (an id that names nothing, a setting the device does not
/// have, a cell that cannot take an item) is reported as a fault with
/// its reason, the map that asked and the cell.
pub fn honour(sim: &mut Sim, loc: Location, request: &SpawnRequest) -> bool {
    let kind = request
        .params
        .get(SPAWN_PARAM_KIND)
        .and_then(Stored::as_str)
        .unwrap_or_default();
    let outcome = match kind {
        SPAWN_KIND_ITEM | SPAWN_KIND_TABLE => furnish_items(sim, loc, request, kind),
        SPAWN_KIND_DEVICE => configure(sim, loc, request),
        _ => return false,
    };
    if let Err(reason) = outcome {
        let map = request
            .params
            .get(SPAWN_PARAM_MAP)
            .and_then(Stored::as_str)
            .unwrap_or_default()
            .to_owned();
        sim.fault(
            reason
                .with("what", request.what.as_str())
                .with("map", map)
                .with("x", i64::from(loc.cell.x))
                .with("y", i64::from(loc.cell.y))
                .with("z", i64::from(loc.cell.z)),
        );
    }
    true
}

/// The namespace of the map that asked, from its qualified id.
fn namespace(request: &SpawnRequest) -> &str {
    request
        .params
        .get(SPAWN_PARAM_MAP)
        .and_then(Stored::as_str)
        .and_then(|map| map.split_once(':'))
        .map_or("", |(namespace, _)| namespace)
}

/// Looks up an id a map wrote: as written if qualified, else in the
/// map's namespace, else as written.
fn lookup<T: Def>(defs: &Defs, written: &str, namespace: &str) -> Option<T::Id> {
    let registry = defs.try_registry::<T>()?;
    if !written.contains(':')
        && let Some(id) = registry.lookup(&format!("{namespace}:{written}"))
    {
        return Some(id);
    }
    registry.lookup(written)
}

/// The source a request names.
fn source_of(sim: &Sim, request: &SpawnRequest, kind: &str) -> Result<Source, Reason> {
    let namespace = namespace(request);
    let found = if kind == SPAWN_KIND_TABLE {
        lookup::<ContentTableDef>(&sim.defs, &request.what, namespace).map(Source::Table)
    } else {
        lookup::<ItemDef>(&sim.defs, &request.what, namespace).map(Source::Item)
    };
    found.ok_or_else(|| {
        Reason::new(
            crate::item::reason::UNKNOWN_DEF,
            format!(
                "a map names `{}`, which is not {} definition",
                request.what,
                if kind == SPAWN_KIND_TABLE {
                    "a `content_table`"
                } else {
                    "an `item`"
                }
            ),
        )
        .with("def", request.what.as_str())
    })
}

/// An integer parameter of a request.
fn int_param(request: &SpawnRequest, name: &str) -> i64 {
    request
        .params
        .get(name)
        .and_then(Stored::as_int)
        .unwrap_or(0)
}

/// The container element of a cell, if it has one: an element whose
/// device holds things. The layer and the element's state.
pub fn container_at(sim: &Sim, loc: Location) -> Option<(Layer, ElementState)> {
    let grid = sim.cosmos.grid(loc.grid).ok()?;
    CONTAINER_LAYERS.into_iter().find_map(|layer| {
        let state = grid.element(loc.cell, layer).ok()?;
        if state.is_none() {
            return None;
        }
        let device = element_device(sim, layer, state).ok().flatten()?;
        let def = sim.defs.try_registry::<DeviceDef>()?.try_get(device)?;
        def.holds.is_some().then_some((layer, state))
    })
}

/// Items or a table in a cell: latent contents of its container, or
/// items lying there.
fn furnish_items(
    sim: &mut Sim,
    loc: Location,
    request: &SpawnRequest,
    kind: &str,
) -> Result<(), Reason> {
    let source = source_of(sim, request, kind)?;
    let count = u32::try_from(int_param(request, SPAWN_PARAM_COUNT).max(0)).unwrap_or(u32::MAX);
    let seed = int_param(request, SPAWN_PARAM_SEED) as u64;
    if count == 0 {
        return Ok(());
    }
    let loc = sim
        .cosmos
        .normalize(loc)
        .map_err(|error| Reason::from_world(&error))?;
    let draw = Draw {
        source,
        count,
        seed,
    };
    match container_at(sim, loc) {
        Some((layer, state)) => match device_at(sim, loc, layer) {
            // A container that is a device already takes them at once.
            Some(device) => make(sim, draw, Some(device), loc, FURNISH_CAUSE).map(|_| ()),
            None => {
                sim.furnish
                    .latent
                    .entry((loc, layer))
                    .or_insert_with(|| Latent {
                        def: state.def,
                        draws: Vec::new(),
                    })
                    .draws
                    .push(draw);
                Ok(())
            }
        },
        None => make(sim, draw, None, loc, FURNISH_CAUSE).map(|_| ()),
    }
}

/// Settings a map gives the device of the element in a cell: the
/// element is promoted and each setting set, in order of key.
fn configure(sim: &mut Sim, loc: Location, request: &SpawnRequest) -> Result<(), Reason> {
    let Some(Stored::Map(config)) = request.params.get(SPAWN_PARAM_CONFIG) else {
        return Ok(());
    };
    let config = config.clone();
    let grid = sim
        .cosmos
        .grid(loc.grid)
        .map_err(|error| Reason::from_world(&error))?;
    let layer = CONTAINER_LAYERS.into_iter().find(|layer| {
        grid.element(loc.cell, *layer)
            .ok()
            .filter(|state| !state.is_none())
            .and_then(|state| element_device(sim, *layer, state).ok().flatten())
            .is_some()
    });
    let Some(layer) = layer else {
        return Err(Reason::new(
            crate::device::codes::NONE,
            "a map gives a device settings in a cell with no element that names a device",
        )
        .with("element", ""));
    };
    let device = promote(sim, loc, layer, FURNISH_CAUSE)?;
    for (key, value) in &config {
        crate::device::set_config(
            sim,
            device,
            key,
            &clash_expr::Value::from(value),
            FURNISH_CAUSE,
        )?;
    }
    Ok(())
}

/// Makes a lot of items: lying in a cell, or stowed in a container. A
/// definition that stacks is made as one stack of the count; one that
/// does not, as so many items, each with a seed of its own drawn from
/// the lot's. Returns what was made.
fn make(
    sim: &mut Sim,
    draw: Draw,
    container: Option<EntityId>,
    loc: Location,
    cause: Cause,
) -> Result<Vec<EntityId>, Reason> {
    let defs = Arc::clone(&sim.defs);
    let lots: Vec<(ItemId, u32, u64)> = match draw.source {
        Source::Item(item) => vec![(item, draw.count, draw.seed)],
        Source::Table(table) => crate::item::table::draw(&defs, table, draw.count, draw.seed)
            .into_iter()
            .map(|drawn| (drawn.item, drawn.count, drawn.seed))
            .collect(),
    };
    let registry = defs.try_registry::<ItemDef>();
    let mut made = Vec::new();
    for (item, count, seed) in lots {
        let stacks = registry
            .and_then(|registry| registry.try_get(item))
            .is_some_and(|def| def.stack);
        let (entities, members) = if stacks { (1, count) } else { (count, 1) };
        let mut seeds = Rng::from_seed(seed);
        for _ in 0..entities {
            let place = match container {
                Some(container) => Place::In(container),
                None => Place::at(loc),
            };
            let id = create_seeded(sim, item, members, place, seeds.next_u64(), cause)?;
            if let Some(container) = container {
                crate::item::hold::set_entry(sim, container, id, crate::item::Hold::Stowed);
            }
            made.push(id);
        }
    }
    Ok(made)
}

// ---- Opening latent contents --------------------------------------------------------

/// Whether an element holds latent contents (06 §9.3).
pub fn is_latent(sim: &Sim, loc: Location, layer: Layer) -> bool {
    sim.cosmos
        .normalize(loc)
        .is_ok_and(|loc| sim.furnish.latent.contains_key(&(loc, layer)))
}

/// What happens when an element becomes a device: if it holds latent
/// contents, they are made, stowed in the device, and
/// `contents_materialised` is raised. The device module calls this
/// from `promote`.
pub(crate) fn on_promoted(
    sim: &mut Sim,
    device: EntityId,
    loc: Location,
    layer: Layer,
    cause: Cause,
) {
    let Some(latent) = sim.furnish.latent.remove(&(loc, layer)) else {
        return;
    };
    let now = sim
        .cosmos
        .grid(loc.grid)
        .and_then(|grid| grid.element(loc.cell, layer))
        .ok();
    if now.is_none_or(|state| state.is_none() || state.def != latent.def) {
        return;
    }
    let mut made = 0_usize;
    for draw in latent.draws {
        match make(sim, draw, Some(device), loc, cause) {
            Ok(list) => made += list.len(),
            Err(reason) => sim.fault(reason),
        }
    }
    let payload = EventPayload::new(CONTENTS_MATERIALISED)
        .subject("self", Subject::Entity(device))
        .with("count", i64::try_from(made).unwrap_or(i64::MAX));
    let _ = sim.raise(Where::Entity(device), cause, payload);
}

/// Opens an element's latent contents (06 §9.3): promotes it to its
/// device, which makes them, and returns what was made. Nothing for an
/// element that holds none. Refused as `promote` refuses.
pub fn open_element(
    sim: &mut Sim,
    loc: Location,
    layer: Layer,
    cause: Cause,
) -> Result<Vec<EntityId>, Reason> {
    let loc = sim
        .cosmos
        .normalize(loc)
        .map_err(|error| Reason::from_world(&error))?;
    if !sim.furnish.latent.contains_key(&(loc, layer)) {
        return Ok(Vec::new());
    }
    let device = promote(sim, loc, layer, cause)?;
    Ok(sim.entities.contents(device).to_vec())
}

/// Latent contents whose element has gone: made where something
/// dropped into the cell comes to rest.
fn spill(sim: &mut Sim, loc: Location, latent: Latent, cause: Cause) {
    let rest = sim.cosmos.resting_cell(loc).unwrap_or(loc);
    for draw in latent.draws {
        if let Err(reason) = make(sim, draw, None, rest, cause) {
            sim.fault(reason);
        }
    }
}

/// What furnishing does about an event, once its handlers have run:
/// the kernel calls this for every event it delivers, before devices
/// do.
///
/// - `element_changed`: if the element that holds latent contents is
///   no longer there (another definition stands there, or none), they
///   spill.
/// - `element_damaged`: a packet that affects a latent container opens
///   it: it is promoted, and its contents made inside it.
pub(crate) fn on_event(sim: &mut Sim, event: &SimEvent) {
    if sim.furnish.latent.is_empty() {
        return;
    }
    match &event.body {
        EventBody::World(WorldEvent::ElementChanged { loc, layer, .. }) => {
            let Some(latent) = sim.furnish.latent.get(&(*loc, *layer)) else {
                return;
            };
            let now = sim
                .cosmos
                .grid(loc.grid)
                .and_then(|grid| grid.element(loc.cell, *layer))
                .ok();
            if now.is_some_and(|state| !state.is_none() && state.def == latent.def) {
                return;
            }
            if let Some(latent) = sim.furnish.latent.remove(&(*loc, *layer)) {
                spill(sim, *loc, latent, event.cause);
            }
        }
        EventBody::World(WorldEvent::ElementDamaged { loc, layer, .. })
            if sim.furnish.latent.contains_key(&(*loc, *layer)) =>
        {
            if let Err(reason) = promote(sim, *loc, *layer, event.cause) {
                sim.fault(reason);
            }
        }
        _ => {}
    }
}

// ---- Properties -----------------------------------------------------------------

/// The properties of furnishing: `element.latent`. It reads truth and
/// is not safe for belief.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "element",
            "latent",
            "bool",
            "Whether generation left contents in the element that have not been made yet (06 §9.3): they are made when it becomes a device (opened, searched, something put in it, damaged) or `materialise_contents` is called on it. False for an element whose contents were made, and for one that never had any.",
            |sim, subject, _| match subject {
                Subject::Element(loc, layer) => clash_expr::Value::Bool(is_latent(sim, loc, layer)),
                _ => clash_expr::Value::Bool(false),
            },
        )
        .changed_by(&["device_promoted", CONTENTS_MATERIALISED]),
    ]
}

// ---- Checking maps ---------------------------------------------------------------

/// Reports every `items` of a map or a palette that names neither an
/// `item` nor a `content_table` definition, at the cell spec that names
/// it: such items could never be made. A load that defines no item at
/// all is not checked. A cross check of the loader.
pub fn check_map_items(cx: &mut CrossCx<'_>) {
    let defs = cx.defs();
    if defs.iter::<ItemDef>().next().is_none() {
        return;
    }
    let mut wrong: Vec<(clash_data::Site, String, String)> = Vec::new();
    let mut look = |owner: &str, entry: &EntryDef| {
        let namespace = owner.split_once(':').map_or("", |(namespace, _)| namespace);
        for alt in &entry.alternatives {
            let Some(items) = &alt.spec.items else {
                continue;
            };
            let found = |written: &str, table: bool| {
                let qualified = if written.contains(':') {
                    written.to_owned()
                } else {
                    format!("{namespace}:{written}")
                };
                if table {
                    defs.lookup::<ContentTableDef>(&qualified).is_some()
                        || defs.lookup::<ContentTableDef>(written).is_some()
                } else {
                    defs.lookup::<ItemDef>(&qualified).is_some()
                        || defs.lookup::<ItemDef>(written).is_some()
                }
            };
            if let Some(item) = &items.item
                && !found(item, false)
            {
                wrong.push((
                    alt.spec.site.clone(),
                    owner.to_owned(),
                    format!("there is no item `{item}`"),
                ));
            }
            if let Some(table) = &items.table
                && !found(table, true)
            {
                wrong.push((
                    alt.spec.site.clone(),
                    owner.to_owned(),
                    format!("there is no content table `{table}`"),
                ));
            }
        }
    };
    for (id, def) in defs.iter::<PaletteDef>() {
        let name = defs.name::<PaletteDef>(id).unwrap_or_default();
        for entry in def.entries.values() {
            look(name, entry);
        }
    }
    for (id, def) in defs.iter::<MapDef>() {
        let name = defs.name::<MapDef>(id).unwrap_or_default();
        for item in &def.palette {
            if let PaletteItem::Inline(entries, _) = item {
                for entry in entries.values() {
                    look(name, entry);
                }
            }
        }
        for entry in def.default.iter().chain(def.roof.iter()) {
            look(name, entry);
        }
        for set in &def.set {
            look(name, &set.spec);
        }
    }
    for (site, owner, message) in wrong {
        let ty = if defs.lookup::<MapDef>(&owner).is_some() {
            MapDef::TYPE
        } else {
            PaletteDef::TYPE
        };
        let mut diagnostic = Diagnostic::new(code::UNKNOWN_REFERENCE, message)
            .in_def(Some(&DefLabel::new(ty, &owner)))
            .with_path(if site.path.is_empty() {
                "items".to_owned()
            } else {
                format!("{}.items", site.path)
            });
        if let Some(span) = site.span {
            diagnostic = diagnostic.at(cx.sources(), span);
        }
        cx.report(diagnostic);
    }
}
