//! Queries over the simulation (05 §11): the kinds the simulation
//! registers, and how it answers them.
//!
//! A query is compiled where it is written, by the data layer
//! (`clash_data::compile_query`), which also answers the two kinds that
//! belong to the language, `list` and `defs`, and applies `where`,
//! `sort` and `limit` to every kind. The simulation registers the kinds
//! below ([`register_queries`]) and gives each one's candidates, in its
//! default order, through [`Sim::answer_query`], which every host over
//! the simulation calls.
//!
//! | Kind | Selects | Arguments | Default order |
//! |------|---------|-----------|---------------|
//! | `entities` | Entities in the world, standing in loaded cells | One shape of `around` with `radius`, `box`, `in_area`, `in_site`, `at`, or none for every entity in the world; and `plane`, `kind`, `tag`, `def`, `has` | Nearest to `around` first; else by cell (storey, row, column), each entity at its first cell; ties and the whole world by id |
//! | `cells` | Loaded cells | One shape of `around` with `radius`, `box`, `in_area`, `at` | Nearest first; else by cell |
//! | `contents` | Entities inside an entity | `of`; `deep` | By id; with `deep`, each followed by what is inside it |
//! | `areas` | Areas | `at`, or `around` with `radius`, or neither for every area | Nearest middle first, then by id; else by id |
//! | `sites` | Sites | `at`, or `around` with `radius`, or neither | Nearest anchor first, then by id; else by id |
//!
//! Every spatial query covers loaded space only and is held to the
//! setting `rules.max_query_cells`: one that would cover more cells is
//! refused with a run-time warning, and one whose size is known when
//! mods load is a load error ([`QuerySpec::extent`]). Distances are as
//! `dist` measures them: metres between cell centres, a storey counting
//! three, the shorter way round on wrapped axes.
//!
//! All of them read truth. None is registered as safe for belief, so a
//! mind context cannot write one: the perception package answers them
//! from what a character believes, through the observer of
//! [`QueryCall`], when it registers them as safe.

use clash_core::{EntityId, IVec3, Real};
use clash_expr::{DEFAULT_MAX_QUERY_CELLS, Fault, QueryCall, QuerySpec, Registry, Type, Value};
use clash_world::dims::{STOREY_M, TRACT_XY};
use clash_world::{
    AreaId, CellBox, CellPos, Location, OccupantFilter, OccupantKind, PlaneMask, SiteId, distance_m,
};

use crate::Sim;
use crate::component::ComponentKind;
use crate::component::identity::Identity;
use crate::entity::Place;
use crate::event::Subject;

/// The setting that caps how many cells a query may cover (05 §13.2).
pub const MAX_QUERY_CELLS_SETTING: &str = "rules.max_query_cells";

/// What answers one kind of query: the simulation and one value for each
/// argument, in the order the kind declares them; the candidates go to
/// `out` in the kind's default order.
pub type QueryFn = fn(&Sim, &[Value], &mut Vec<Value>) -> Result<(), Fault>;

/// One kind of query the simulation answers.
#[derive(Clone, Copy)]
pub struct QueryDef {
    /// The key a query of this kind is written under.
    pub name: &'static str,
    /// Builds the specification against a registry that holds the
    /// simulation's kinds. `None` if what it names is missing.
    pub build: fn(&Registry) -> Option<QuerySpec>,
    /// Answers it.
    pub run: QueryFn,
}

impl std::fmt::Debug for QueryDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryDef")
            .field("name", &self.name)
            .finish()
    }
}

/// Every kind of query the simulation answers.
pub fn engine_queries() -> Vec<QueryDef> {
    vec![
        QueryDef {
            name: "entities",
            build: entities_spec,
            run: entities,
        },
        QueryDef {
            name: "cells",
            build: cells_spec,
            run: cells,
        },
        QueryDef {
            name: "contents",
            build: contents_spec,
            run: contents,
        },
        QueryDef {
            name: "areas",
            build: |registry| places_spec(registry, "areas", "area"),
            run: areas,
        },
        QueryDef {
            name: "sites",
            build: |registry| places_spec(registry, "sites", "site"),
            run: sites,
        },
    ]
}

/// Enters the simulation's kinds of query into a registry, leaving
/// alone those it already holds.
pub fn register_queries(registry: &mut Registry) {
    for query in engine_queries() {
        if registry.query_id(query.name).is_none()
            && let Some(spec) = (query.build)(registry)
        {
            let _ = registry.add_query(spec);
        }
    }
}

impl Sim {
    /// The candidates of a query of one of the simulation's kinds, in
    /// its default order (05 §11): what every host over the simulation
    /// answers [`Host::query`](clash_expr::Host::query) with.
    ///
    /// Refused, with a fault that becomes a run-time warning, for a kind
    /// the simulation does not answer, for arguments the kind cannot use
    /// and for a query that would cover more than `rules.max_query_cells`
    /// cells. A query in a mind context is refused: none of these is
    /// answered from belief yet.
    pub fn answer_query(&self, call: &QueryCall<'_>, out: &mut Vec<Value>) -> Result<(), Fault> {
        if call.observer.is_some() {
            return Err(Fault::Unavailable("queries answered from belief"));
        }
        let registry = self.exprs.registry();
        let Some(kind) = registry.query(call.query) else {
            return Err(Fault::Query(
                "the simulation does not know this kind of query",
            ));
        };
        let Some(query) = engine_queries()
            .into_iter()
            .find(|query| query.name == kind.spec.name)
        else {
            return Err(Fault::Query(
                "the simulation does not answer this kind of query",
            ));
        };
        (query.run)(self, call.args, out)
    }

    /// The most cells a query may cover now.
    fn max_query_cells(&self) -> u64 {
        let default = i64::try_from(DEFAULT_MAX_QUERY_CELLS).unwrap_or(i64::MAX);
        u64::try_from(self.setting_int(MAX_QUERY_CELLS_SETTING, default).max(0)).unwrap_or(0)
    }

    /// Refuses a query that would cover more cells than it may.
    fn within_cap(&self, cells: u64) -> Result<(), Fault> {
        if cells > self.max_query_cells() {
            Err(Fault::Query(
                "it would cover more cells than `rules.max_query_cells` allows",
            ))
        } else {
            Ok(())
        }
    }
}

// ---- arguments -------------------------------------------------------------

/// The arguments of `entities`, in order.
const E_AROUND: usize = 0;
const E_RADIUS: usize = 1;
const E_BOX: usize = 2;
const E_IN_AREA: usize = 3;
const E_IN_SITE: usize = 4;
const E_AT: usize = 5;
const E_PLANE: usize = 6;
const E_KIND: usize = 7;
const E_TAG: usize = 8;
const E_DEF: usize = 9;
const E_HAS: usize = 10;

/// The arguments of `cells`, in order.
const C_AROUND: usize = 0;
const C_RADIUS: usize = 1;
const C_BOX: usize = 2;
const C_IN_AREA: usize = 3;
const C_AT: usize = 4;

/// The arguments of `areas` and `sites`, in order.
const P_AT: usize = 0;
const P_AROUND: usize = 1;
const P_RADIUS: usize = 2;

/// How many cells a radius covers: the box round its centre, a storey
/// counting [`STOREY_M`].
pub fn radius_cells(radius: Real) -> u64 {
    if radius < Real::ZERO {
        return 0;
    }
    let reach = u64::try_from(radius.ceil().to_i32().max(0)).unwrap_or(0);
    let storeys = u64::try_from((radius / STOREY_M).ceil().to_i32().max(0)).unwrap_or(0);
    let side = 2 * reach + 1;
    side.saturating_mul(side).saturating_mul(2 * storeys + 1)
}

/// The extent of a query whose radius is the argument at `at`.
fn radius_extent(constants: &[Option<Value>], at: usize) -> Option<u64> {
    constants.get(at)?.as_ref()?.as_number().map(radius_cells)
}

fn arg(args: &[Value], at: usize) -> &Value {
    args.get(at).unwrap_or(&Value::Null)
}

fn given(args: &[Value], at: usize) -> bool {
    !arg(args, at).is_null()
}

/// Where something is, as `dist` reads it: a cell; an element's cell; an
/// entity's own cell, or that of what it is inside; an area's middle; a
/// site's anchor.
pub(crate) fn position(sim: &Sim, value: &Value) -> Option<Location> {
    match sim.subject_of(value)? {
        Subject::Entity(id) => {
            let top = sim.entities.outermost(id)?;
            match sim.entities.get(top)?.place() {
                Place::At { loc, .. } => Some(loc),
                Place::In(_) | Place::Stored => None,
            }
        }
        Subject::Loc(loc) | Subject::Element(loc, _) => Some(loc),
        Subject::Area(id) => {
            let area = sim.cosmos.area(id)?;
            Some(Location::new(area.grid, area.shape.centre()?))
        }
        Subject::Site(id) => {
            let site = sim.cosmos.site(id)?;
            let grid = sim.cosmos.realm(site.realm)?.base_grid;
            Some(Location::new(grid, site.anchor))
        }
        Subject::Tract(..) | Subject::Realm(_) | Subject::Global => None,
    }
}

/// The distance in metres between two cells of one grid, as `dist`
/// measures it.
fn metres(sim: &Sim, a: Location, b: Location) -> Option<Real> {
    sim.cosmos.delta(a, b).map(distance_m)
}

/// The centre and radius of `around` and `radius`. `Ok(None)` for a
/// centre that is nowhere: such a query has no results.
fn centre_and_radius(
    sim: &Sim,
    args: &[Value],
    around: usize,
    radius: usize,
) -> Result<Option<(Location, Real)>, Fault> {
    let Some(reach) = arg(args, radius).as_number() else {
        return Err(Fault::Query(
            "`around` needs a `radius`, a number of metres",
        ));
    };
    if reach < Real::ZERO {
        return Err(Fault::Query("a radius is not below zero"));
    }
    sim.within_cap(radius_cells(reach))?;
    Ok(position(sim, arg(args, around)).map(|centre| (centre, reach)))
}

/// The box between two cells of one grid, given as a list of two.
fn box_of(sim: &Sim, value: &Value) -> Result<Option<(clash_world::GridId, CellBox)>, Fault> {
    let corners: Vec<Location> = match value {
        Value::List(items) => items
            .iter()
            .filter_map(|item| match sim.subject_of(item)? {
                Subject::Loc(loc) | Subject::Element(loc, _) => Some(loc),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let [a, b] = corners.as_slice() else {
        return Err(Fault::Query("`box` is a list of two cells, its corners"));
    };
    if a.grid != b.grid {
        return Ok(None);
    }
    let cells = CellBox::new(a.cell, b.cell);
    sim.within_cap(cells.volume())?;
    Ok(Some((a.grid, cells)))
}

/// The ids of a list of occupied cells, each once, in the order they
/// first appear.
fn first_appearance(found: impl IntoIterator<Item = EntityId>) -> Vec<EntityId> {
    let mut seen = std::collections::BTreeSet::new();
    found.into_iter().filter(|id| seen.insert(*id)).collect()
}

// ---- entities --------------------------------------------------------------

fn entities_spec(registry: &Registry) -> Option<QuerySpec> {
    let entity = Type::Subject(registry.kind_id("entity")?);
    let loc = Type::Subject(registry.kind_id("loc")?);
    let area = Type::Subject(registry.kind_id("area")?);
    let site = Type::Subject(registry.kind_id("site")?);
    Some(
        QuerySpec::new("entities", entity)
            .arg("around", Type::Any, "The centre: a cell, an element, an entity, an area (its middle) or a site (its anchor). With `radius`.")
            .arg("radius", Type::Real, "With `around`: how far from the centre, in metres, a storey counting three.")
            .arg("box", Type::list(loc.clone()), "Two cells of one grid, the corners of a box, both inside it.")
            .arg("in_area", area, "An area: the entities with a cell inside it.")
            .arg("in_site", site, "A site: the entities standing in its loaded tracts.")
            .arg("at", loc, "One cell: the entities with a cell there.")
            .arg("plane", Type::Str, "Only entities on this plane, by its qualified id: `engine:default`.")
            .arg("kind", Type::Str, "Only occupants of this kind: `creature`, `item`, `device` or `other`.")
            .arg("tag", Type::Str, "Only entities with this tag.")
            .arg("def", Type::Str, "Only entities made from the definition with this qualified id.")
            .arg("has", Type::Str, "Only entities with a component of this type: `body`, `stats`, `item`.")
            .one_of(&["around", "box", "in_area", "in_site", "at"])
            .needs("radius", "around")
            .needs("around", "radius")
            .extent(|constants| radius_extent(constants, E_RADIUS))
            .doc("Entities in the world: those standing in loaded cells, and not what is inside them. Shaped by one of `around` with `radius`, `box`, `in_area`, `in_site` and `at`, or by none for every entity in the world; narrowed by `plane`, `kind`, `tag`, `def` and `has`. Nearest to `around` first, by the nearest cell each fills; with another shape by cell, storey by storey and row by row, each entity where it first appears; ties, and the whole world, by id."),
    )
}

/// The filters of `entities` beside its shape.
struct EntityFilter {
    plane: Option<PlaneMask>,
    kind: Option<OccupantKind>,
    tag: Option<String>,
    def: Option<String>,
    has: Option<ComponentKind>,
}

impl EntityFilter {
    fn read(sim: &Sim, args: &[Value]) -> Result<EntityFilter, Fault> {
        let text = |at: usize| arg(args, at).as_str().map(str::to_owned);
        let plane = match text(E_PLANE) {
            Some(name) => Some(
                sim.cosmos
                    .defs()
                    .planes()
                    .lookup(&name)
                    .map(PlaneMask::of)
                    .ok_or(Fault::Query("`plane` names no plane"))?,
            ),
            None => None,
        };
        let kind = match text(E_KIND).as_deref() {
            None => None,
            Some("creature") => Some(OccupantKind::Creature),
            Some("item") => Some(OccupantKind::Item),
            Some("device") => Some(OccupantKind::Device),
            Some("other") => Some(OccupantKind::Other),
            Some(_) => {
                return Err(Fault::Query(
                    "`kind` is one of `creature`, `item`, `device` and `other`",
                ));
            }
        };
        let has = match text(E_HAS) {
            Some(name) => Some(
                ComponentKind::from_name(&name)
                    .ok_or(Fault::Query("`has` names no type of component"))?,
            ),
            None => None,
        };
        Ok(EntityFilter {
            plane,
            kind,
            tag: text(E_TAG),
            def: text(E_DEF),
            has,
        })
    }

    fn admits(&self, sim: &Sim, id: EntityId) -> bool {
        let Some(entity) = sim.entities.get(id) else {
            return false;
        };
        if self.plane.is_some() || self.kind.is_some() {
            let Some(record) = sim.cosmos.occupant(id) else {
                return false;
            };
            if self
                .plane
                .is_some_and(|plane| !record.occupant.plane.shares(plane))
            {
                return false;
            }
            if self.kind.is_some_and(|kind| record.occupant.kind != kind) {
                return false;
            }
        }
        if let Some(tag) = &self.tag
            && !entity.tags.contains(tag)
        {
            return false;
        }
        if let Some(def) = &self.def {
            let named = sim
                .components
                .get::<Identity>(id)
                .and_then(Identity::def)
                .and_then(|key| sim.exprs.registry().def_name(clash_expr::DefRef::from(key)));
            if named != Some(def.as_str()) {
                return false;
            }
        }
        if let Some(kind) = self.has
            && !sim.components.has(kind, id)
        {
            return false;
        }
        true
    }
}

/// `entities`: see the [module documentation](self).
fn entities(sim: &Sim, args: &[Value], out: &mut Vec<Value>) -> Result<(), Fault> {
    let filter = EntityFilter::read(sim, args)?;
    let world = |error: clash_world::WorldError| {
        let _ = error;
        Fault::Query("the world refused it")
    };
    let found: Vec<EntityId> = if given(args, E_AROUND) || given(args, E_RADIUS) {
        let Some((centre, reach)) = centre_and_radius(sim, args, E_AROUND, E_RADIUS)? else {
            return Ok(());
        };
        let occupied = sim
            .cosmos
            .occupants_within(centre, reach, &OccupantFilter::ANY)
            .map_err(world)?;
        // Each entity at its nearest cell; nearest first, then by id.
        let mut nearest: Vec<(Real, EntityId)> = Vec::new();
        for place in occupied {
            let Some(distance) = metres(sim, centre, Location::new(centre.grid, place.cell)) else {
                continue;
            };
            match nearest.iter_mut().find(|(_, id)| *id == place.id) {
                Some(entry) if distance < entry.0 => entry.0 = distance,
                Some(_) => {}
                None => nearest.push((distance, place.id)),
            }
        }
        nearest.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        nearest.into_iter().map(|(_, id)| id).collect()
    } else if given(args, E_BOX) {
        let Some((grid, cells)) = box_of(sim, arg(args, E_BOX))? else {
            return Ok(());
        };
        let occupied = sim
            .cosmos
            .occupants_in_box(grid, cells, &OccupantFilter::ANY)
            .map_err(world)?;
        first_appearance(occupied.into_iter().map(|place| place.id))
    } else if given(args, E_IN_AREA) {
        let Some(Subject::Area(area)) = sim.subject_of(arg(args, E_IN_AREA)) else {
            return Ok(());
        };
        let Some(bounds) = sim.cosmos.area(area).and_then(|area| area.shape.bounds()) else {
            return Ok(());
        };
        sim.within_cap(bounds.volume())?;
        let mut occupied = sim
            .cosmos
            .occupants_in_area(area, &OccupantFilter::ANY)
            .map_err(world)?;
        occupied.sort_by_key(|place| (place.cell, place.id));
        first_appearance(occupied.into_iter().map(|place| place.id))
    } else if given(args, E_IN_SITE) {
        let Some(Subject::Site(site)) = sim.subject_of(arg(args, E_IN_SITE)) else {
            return Ok(());
        };
        site_occupants(sim, site)?
    } else if given(args, E_AT) {
        let Some(at) = position(sim, arg(args, E_AT)) else {
            return Ok(());
        };
        let occupied = sim
            .cosmos
            .occupants_at(at, &OccupantFilter::ANY)
            .map_err(world)?;
        first_appearance(occupied.into_iter().map(|place| place.id))
    } else {
        let all = sim.cosmos.occupants();
        sim.within_cap(u64::try_from(all.len()).unwrap_or(u64::MAX))?;
        all.into_iter().map(|record| record.occupant.id).collect()
    };
    out.extend(
        found
            .into_iter()
            .filter(|id| filter.admits(sim, *id))
            .map(|id| sim.subject_value(Subject::Entity(id))),
    );
    Ok(())
}

/// The entities standing in the loaded tracts of a site, by cell and
/// then by id, each where it first appears.
fn site_occupants(sim: &Sim, site: SiteId) -> Result<Vec<EntityId>, Fault> {
    let Some(record) = sim.cosmos.site(site) else {
        return Ok(Vec::new());
    };
    let side = u64::try_from(TRACT_XY).unwrap_or(u64::MAX);
    let tracts = u64::try_from(record.tracts.len()).unwrap_or(u64::MAX);
    sim.within_cap(tracts.saturating_mul(side * side))?;
    let Some(grid) = sim.cosmos.realm(record.realm).map(|realm| realm.base_grid) else {
        return Ok(Vec::new());
    };
    let mut found: Vec<(CellPos, EntityId)> = Vec::new();
    for occupant in sim.cosmos.occupants() {
        let id = occupant.occupant.id;
        let cells: Vec<CellPos> = occupant
            .footprint
            .cells(occupant.loc.cell)
            .cells()
            .filter_map(|cell| sim.cosmos.normalize(Location::new(grid, cell)).ok())
            .filter(|cell| cell.grid == grid && occupant.loc.grid == grid)
            .map(|cell| cell.cell)
            .filter(|cell| record.tracts.binary_search(&cell.tract()).is_ok())
            .collect();
        if let Some(first) = cells.into_iter().min() {
            found.push((first, id));
        }
    }
    found.sort();
    Ok(found.into_iter().map(|(_, id)| id).collect())
}

// ---- cells -----------------------------------------------------------------

fn cells_spec(registry: &Registry) -> Option<QuerySpec> {
    let loc = Type::Subject(registry.kind_id("loc")?);
    let area = Type::Subject(registry.kind_id("area")?);
    Some(
        QuerySpec::new("cells", loc.clone())
            .arg("around", Type::Any, "The centre, as for `entities`. With `radius`.")
            .arg("radius", Type::Real, "With `around`: how far from the centre, in metres, a storey counting three.")
            .arg("box", Type::list(loc.clone()), "Two cells of one grid, the corners of a box, both inside it.")
            .arg("in_area", area, "An area: the cells it covers.")
            .arg("at", loc, "One cell.")
            .one_of(&["around", "box", "in_area", "at"])
            .needs("radius", "around")
            .needs("around", "radius")
            .extent(|constants| radius_extent(constants, C_RADIUS))
            .doc("Loaded cells, shaped by one of `around` with `radius`, `box`, `in_area` and `at`: a cell that is not loaded is left out. Narrow them with `where`, on `it`. Nearest to `around` first; with another shape storey by storey and row by row; ties in that order too."),
    )
}

/// `cells`: see the [module documentation](self).
fn cells(sim: &Sim, args: &[Value], out: &mut Vec<Value>) -> Result<(), Fault> {
    let mut push = |loc: Location| {
        if let Ok(loc) = sim.cosmos.normalize(loc)
            && sim.cosmos.is_loaded(loc)
        {
            out.push(sim.subject_value(Subject::Loc(loc)));
        }
    };
    if given(args, C_AROUND) || given(args, C_RADIUS) {
        let Some((centre, reach)) = centre_and_radius(sim, args, C_AROUND, C_RADIUS)? else {
            return Ok(());
        };
        let cells = radius_box(centre.cell, reach);
        let mut found: Vec<(Real, CellPos, Location)> = Vec::new();
        for cell in cells.cells() {
            let Ok(loc) = sim.cosmos.normalize(Location::new(centre.grid, cell)) else {
                continue;
            };
            let Some(distance) = metres(sim, centre, loc) else {
                continue;
            };
            if distance <= reach {
                found.push((distance, loc.cell, loc));
            }
        }
        found.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        // A wrapped axis can reach one cell twice.
        found.dedup_by_key(|(_, cell, _)| *cell);
        for (_, _, loc) in found {
            push(loc);
        }
    } else if given(args, C_BOX) {
        let Some((grid, cells)) = box_of(sim, arg(args, C_BOX))? else {
            return Ok(());
        };
        for cell in cells.cells() {
            push(Location::new(grid, cell));
        }
    } else if given(args, C_IN_AREA) {
        let Some(Subject::Area(area)) = sim.subject_of(arg(args, C_IN_AREA)) else {
            return Ok(());
        };
        area_cells(sim, area, &mut push)?;
    } else if given(args, C_AT) {
        if let Some(at) = position(sim, arg(args, C_AT)) {
            push(at);
        }
    } else {
        return Err(Fault::Query(
            "`cells` takes a shape: `around` with `radius`, `box`, `in_area` or `at`",
        ));
    }
    Ok(())
}

/// The box a radius round a cell reaches into.
fn radius_box(centre: CellPos, reach: Real) -> CellBox {
    let across = reach.ceil().to_i32();
    let storeys = (reach / STOREY_M).ceil().to_i32();
    CellBox::around(centre, IVec3::new(across, across, storeys))
}

/// The cells an area covers, storey by storey and row by row.
fn area_cells(sim: &Sim, area: AreaId, push: &mut impl FnMut(Location)) -> Result<(), Fault> {
    let Some(record) = sim.cosmos.area(area) else {
        return Ok(());
    };
    let Some(bounds) = record.shape.bounds() else {
        return Ok(());
    };
    sim.within_cap(bounds.volume())?;
    let Ok(view) = sim.cosmos.grid(record.grid) else {
        return Ok(());
    };
    let topology = view.topology();
    let mut cells: Vec<CellPos> = bounds
        .cells()
        .filter_map(|cell| topology.normalize(cell).ok())
        .filter(|cell| record.shape.contains(topology, *cell))
        .collect();
    cells.sort();
    cells.dedup();
    for cell in cells {
        push(Location::new(record.grid, cell));
    }
    Ok(())
}

// ---- contents --------------------------------------------------------------

fn contents_spec(registry: &Registry) -> Option<QuerySpec> {
    let entity = Type::Subject(registry.kind_id("entity")?);
    Some(
        QuerySpec::new("contents", entity.clone())
            .arg("of", entity, "The entity whose contents are wanted. Required.")
            .arg("deep", Type::Bool, "Whether what is inside its contents is wanted too. Default false.")
            .doc("What is inside an entity (01 §6): in a pack, in a hand, worn, carried. In order of id; with `deep`, each thing followed by what is inside it."),
    )
}

/// `contents`: see the [module documentation](self).
fn contents(sim: &Sim, args: &[Value], out: &mut Vec<Value>) -> Result<(), Fault> {
    let Value::Subject(_) = arg(args, 0) else {
        if arg(args, 0).is_null() {
            return Ok(());
        }
        return Err(Fault::Query("`contents` needs `of`, an entity"));
    };
    let Some(Subject::Entity(of)) = sim.subject_of(arg(args, 0)) else {
        return Ok(());
    };
    let deep = arg(args, 1).as_bool().unwrap_or(false);
    let inside: Vec<EntityId> = if deep {
        sim.entities.contents_deep(of)
    } else {
        sim.entities.contents(of).to_vec()
    };
    out.extend(
        inside
            .into_iter()
            .map(|id| sim.subject_value(Subject::Entity(id))),
    );
    Ok(())
}

// ---- areas and sites -------------------------------------------------------

fn places_spec(registry: &Registry, name: &str, kind: &str) -> Option<QuerySpec> {
    let element = Type::Subject(registry.kind_id(kind)?);
    let loc = Type::Subject(registry.kind_id("loc")?);
    let doc = if kind == "area" {
        "Areas (02 §8.3): those covering a cell with `at`; those whose middle is within `radius` of `around`, nearest first; or, with neither, every area. Ties, and the rest, by id."
    } else {
        "Sites (02 §8.4): the one holding a cell with `at`; those whose anchor is within `radius` of `around`, nearest first; or, with neither, every site. Ties, and the rest, by id."
    };
    Some(
        QuerySpec::new(name, element)
            .arg("at", loc, "A cell: those that hold it.")
            .arg(
                "around",
                Type::Any,
                "The centre, as for `entities`. With `radius`.",
            )
            .arg("radius", Type::Real, "With `around`: how far, in metres.")
            .one_of(&["at", "around"])
            .needs("radius", "around")
            .needs("around", "radius")
            .doc(doc),
    )
}

/// Orders places nearest first, then by id, keeping those in reach.
fn nearest_first<I: Ord + Copy>(
    sim: &Sim,
    centre: Location,
    reach: Real,
    places: impl IntoIterator<Item = (I, Option<Location>)>,
) -> Vec<I> {
    let mut found: Vec<(Real, I)> = places
        .into_iter()
        .filter_map(|(id, at)| {
            let distance = metres(sim, centre, at?)?;
            (distance <= reach).then_some((distance, id))
        })
        .collect();
    found.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    found.into_iter().map(|(_, id)| id).collect()
}

/// `areas`: see the [module documentation](self).
fn areas(sim: &Sim, args: &[Value], out: &mut Vec<Value>) -> Result<(), Fault> {
    let found: Vec<AreaId> = if given(args, P_AT) {
        let Some(at) = position(sim, arg(args, P_AT)) else {
            return Ok(());
        };
        sim.cosmos.areas_at(at).unwrap_or_default()
    } else if given(args, P_AROUND) || given(args, P_RADIUS) {
        let Some((centre, reach)) = centre_and_radius(sim, args, P_AROUND, P_RADIUS)? else {
            return Ok(());
        };
        nearest_first(
            sim,
            centre,
            reach,
            sim.cosmos.areas().map(|area| {
                (
                    area.id,
                    area.shape
                        .centre()
                        .map(|cell| Location::new(area.grid, cell)),
                )
            }),
        )
    } else {
        let mut all: Vec<AreaId> = sim.cosmos.areas().map(|area| area.id).collect();
        all.sort();
        all
    };
    out.extend(
        found
            .into_iter()
            .map(|id| sim.subject_value(Subject::Area(id))),
    );
    Ok(())
}

/// `sites`: see the [module documentation](self).
fn sites(sim: &Sim, args: &[Value], out: &mut Vec<Value>) -> Result<(), Fault> {
    let found: Vec<SiteId> = if given(args, P_AT) {
        let Some(at) = position(sim, arg(args, P_AT)) else {
            return Ok(());
        };
        sim.cosmos.site_of(at).ok().flatten().into_iter().collect()
    } else if given(args, P_AROUND) || given(args, P_RADIUS) {
        let Some((centre, reach)) = centre_and_radius(sim, args, P_AROUND, P_RADIUS)? else {
            return Ok(());
        };
        nearest_first(
            sim,
            centre,
            reach,
            sim.cosmos.sites().map(|site| {
                let grid = sim.cosmos.realm(site.realm).map(|realm| realm.base_grid);
                (site.id, grid.map(|grid| Location::new(grid, site.anchor)))
            }),
        )
    } else {
        let mut all: Vec<SiteId> = sim.cosmos.sites().map(|site| site.id).collect();
        all.sort();
        all
    };
    out.extend(
        found
            .into_iter()
            .map(|id| sim.subject_value(Subject::Site(id))),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_radius_covers_the_box_round_its_centre() {
        assert_eq!(radius_cells(Real::ZERO), 1);
        // One metre: three by three on one storey and the storeys above
        // and below.
        assert_eq!(radius_cells(Real::ONE), 27);
        assert_eq!(radius_cells(Real::from_i64(-1)), 0);
        assert!(radius_cells(Real::from_i64(200)) > DEFAULT_MAX_QUERY_CELLS);
        assert!(radius_cells(Real::from_i64(40)) < DEFAULT_MAX_QUERY_CELLS);
    }
}
