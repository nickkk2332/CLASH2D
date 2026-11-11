//! The `standard` generator: tracts realised from tract definitions,
//! grounds and placed maps (10 §6), behind the interfaces of 02 §9.3.
//!
//! A realm definition names it and says what its land is made of:
//!
//! ```json
//! { "type": "realm", "id": "countryside", "generator": "standard",
//!   "generator_params": {
//!     "tracts": [ { "tract": "field", "weight": 5 }, { "tract": "wood", "weight": 2 } ],
//!     "layout": { "at": [0, 0], "rows": [ "fwf", "sss" ],
//!                 "legend": { "f": "field", "w": "wood", "s": "lane" } },
//!     "height": { "noise": { "octaves": 3, "scale": 6 }, "range": [0, 3] } } }
//! ```
//!
//! | Parameter | Meaning | Default |
//! |-----------|---------|---------|
//! | `tracts` | The tract definitions the land is made of, each `{ "tract": id, "weight": number }`: every tract is given one by weighted choice | None: a tract with no definition is the realm's fill and nothing else |
//! | `patch` | Tracts along the side of a patch: the tracts of a patch are given the same definition | 1 |
//! | `layout` | Tract definitions written as rows of characters (10 §9.3): `at` is the tract of the first character, `rows` run north to south, `legend` says which definition each character stands for, and a space leaves the tract to `tracts` | None |
//! | `height` | The surface: a noise field (10 §9.1), sampled at each cell in tracts and rounded to storeys | Flat |
//! | `surface_z` | The storey the surface is counted from: added to `height` | 0 |
//! | `place` | Maps placed by hand, anywhere in the realm (10 §9.3): each `{ "map": id, "at": [x, y] }` or `[x, y, z]`, with `turn` in degrees, `mirror`, `seed`, `params` and `order` | None |
//!
//! # Realising a tract
//!
//! In the order of 10 §6.1, as a pure function of the tract's record,
//! its neighbours' records and the realm's seed:
//!
//! 1. The surface is worked out for the tract and a margin round it:
//!    the height field, flattened under every map that sits by `level`.
//! 2. The ground's steps fill the tract.
//! 3. Foundations: gaps under maps that sit by `foundation` are filled
//!    and ground above their floors is cut.
//! 4. Placements are written in `order`, each clipped to the tract.
//! 5. Transforms would run here. They are a later package: see
//!    [`StandardGenerator::transforms`].
//! 6. What the maps ask for besides cells is handed on: spawns whose
//!    cell is in the tract, and areas whose home is.
//!
//! # Where placements come from
//!
//! A realiser is shown tract records and no region record (02 §9.3),
//! so the placements that overlap a tract must follow from what it is
//! shown. There are two sources, and both do:
//!
//! - **The realm's own `place`**: known to the generator, the same for
//!   every tract, however many tracts a map covers.
//! - **Tract definitions**: a tract's placements are a pure function of
//!   the seed, its position and its definition, and lie wholly inside
//!   it. A tract works its neighbours' out too, from their records,
//!   since a neighbour's levelled lot may touch the shared edge and
//!   decide where a ramp goes.
//!
//! A survey writes the same placements into the region's record
//! (02 §9.1), for everything else that asks what stands where.
//!
//! # The seam for regions and settlements
//!
//! Two functions decide what a region holds, and the packages that
//! survey regions replace them: [`StandardGenerator::tract_def`]
//! (which definition a tract is given) and
//! [`StandardGenerator::placements_of`] (what stands on a tract).
//! Whatever replaces them must stay a pure function of the seed and
//! the position, and must give a tract's realiser every placement that
//! overlaps the tract or lies within [`GATHER_MARGIN`] cells of it:
//! either worked out again from position, as here, or looked up in a
//! region's plan that the generator keeps.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{HashDomain, IVec2, Real, Rot4, Turn, VarMap, pos_hash, pos_rng};
use clash_world::dims::{REGION_CELLS, REGION_TRACT_COUNT, REGION_TRACTS, TRACT_XY};
use clash_world::{
    CellBox, CellPos, ColumnBuilder, GenCtx, GenError, GeneratorRegistry, GeneratorRequest,
    Generators, Placement, RealizedTract, RealmInfo, RegionPos, RegionRecord, RegionSurvey,
    RegionSurveyResult, RegionSurveyor, TractDefId, TractPos, TractRealizer, TractRecord,
    TractRecords3x3, WorldDefs, WorldGenerator,
};

use crate::ground::{Ground, GroundRun, MAX_SCATTER_MAP_SIDE};
use crate::map::{Base, MapId, Sit, nested_origin};
use crate::noise::{Extent, FieldSpec};
use crate::place::{
    self, Canvas, Heights, MapMode, PlacedMap, TractCanvas, boxes_overlap, placement_box,
};
use crate::tractdef::{GenDefs, TractKind};

/// The name realm definitions give this generator by.
pub const STANDARD_GENERATOR: &str = "standard";

/// How many cells round a tract its realiser gathers placements from:
/// far enough for a levelled lot beside the tract to shape a ramp
/// inside it, and for a scattered map to know it stands on a lot.
pub const GATHER_MARGIN: i32 = MAX_SCATTER_MAP_SIDE;

/// The most a storey of the surface may be given, either way.
const MAX_SURFACE_Z: i64 = 1 << 20;

/// The most maps a realm definition may place by hand.
const MAX_AUTHORED: usize = 2000;

/// The widest a layout of tract definitions may be, in tracts.
const MAX_LAYOUT_SIDE: usize = 512;

/// The domain of the realm's height field.
const HEIGHT_DOMAIN: HashDomain = HashDomain::new("gen.field.height");
/// The domain of which definition a tract is given.
const TRACT_DOMAIN: HashDomain = HashDomain::new("gen.survey.tract");
/// The domain of whether, and as which map, a tract definition's
/// placement is made.
const PLACE_DOMAIN: HashDomain = HashDomain::new("gen.survey.place");
/// The domain of the seed a placement is given.
const SEED_DOMAIN: HashDomain = HashDomain::new("gen.survey.seed");

const TRACTS: &str = "tracts";
const PATCH: &str = "patch";
const LAYOUT: &str = "layout";
const HEIGHT: &str = "height";
const SURFACE_Z: &str = "surface_z";
const PLACE: &str = "place";

/// Every parameter the generator takes.
const PARAMS: [&str; 6] = [TRACTS, PATCH, LAYOUT, HEIGHT, SURFACE_Z, PLACE];

/// Tract definitions written as rows of characters.
#[derive(Clone, Debug)]
struct Layout {
    /// The tract of the first character of the first row.
    at: TractPos,
    /// For each row, north to south, the definition of each tract, or
    /// `None` for one left to the weighted choice.
    rows: Vec<Vec<Option<TractDefId>>>,
}

impl Layout {
    fn get(&self, tract: TractPos) -> Option<TractDefId> {
        let dx = usize::try_from(i64::from(tract.x) - i64::from(self.at.x)).ok()?;
        let dy = usize::try_from(i64::from(tract.y) - i64::from(self.at.y)).ok()?;
        *self.rows.get(dy)?.get(dx)?
    }
}

/// A map a realm definition places by hand.
#[derive(Clone, Debug)]
struct Authored {
    map: MapId,
    x: i32,
    y: i32,
    /// The storey of the map's level 0, or `None` for what the map's
    /// `base` says.
    z: Option<i32>,
    turn: Turn,
    seed: Option<u64>,
    params: VarMap,
    order: u16,
}

/// A placement a survey could not write: its box met one written
/// before it, and its map does not say `over` (10 §4.1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DroppedPlacement {
    /// The tract whose definition asked for it.
    pub tract: TractPos,
    /// The placement.
    pub placement: Placement,
    /// The box it would have filled.
    pub bounds: CellBox,
    /// The box, written before it, that it met.
    pub met: CellBox,
}

/// The `standard` generator. See the module documentation.
#[derive(Clone, Debug)]
pub struct StandardGenerator {
    defs: Arc<GenDefs>,
    /// The definitions tracts are given, with their weights.
    tracts: Vec<(TractDefId, Real)>,
    total: Real,
    patch: i32,
    layout: Option<Layout>,
    height: Option<FieldSpec>,
    surface_z: i32,
    sea_level: Option<i32>,
    authored: Vec<Authored>,
    /// The ground of a tract that has none: the realm's fill.
    bare: Ground,
}

/// A placement gathered for a tract, with what its realiser needs to
/// know before resolving it.
#[derive(Clone, Debug)]
struct Near {
    placement: Placement,
    bounds: CellBox,
    sit: Sit,
    /// The storey of the map's level 0.
    level: i32,
}

impl Near {
    fn covers(&self, x: i32, y: i32) -> bool {
        (self.bounds.min.x..=self.bounds.max.x).contains(&x)
            && (self.bounds.min.y..=self.bounds.max.y).contains(&y)
    }
}

/// The map a realm definition names: a qualified id, or a bare name
/// looked up in the definition's own namespace first and then, if only
/// one mod has a map of that name, there.
fn map_named(maps: &crate::map::MapDefs, name: &str, namespace: Option<&str>) -> Option<MapId> {
    if name.contains(':') {
        return maps.lookup(name);
    }
    namespace
        .and_then(|namespace| maps.lookup(&format!("{namespace}:{name}")))
        .or_else(|| {
            let suffix = format!(":{name}");
            let mut found = maps.iter().filter(|(_, map)| map.name().ends_with(&suffix));
            match (found.next(), found.next()) {
                (Some((id, _)), None) => Some(id),
                _ => None,
            }
        })
}

/// The box of a tract's cells, at every storey.
fn tract_box(tract: TractPos, margin: i32) -> CellBox {
    let origin = tract.origin(0);
    CellBox::new(
        CellPos::new(origin.x - margin, origin.y - margin, i32::MIN),
        CellPos::new(
            origin.x + TRACT_XY - 1 + margin,
            origin.y + TRACT_XY - 1 + margin,
            i32::MAX,
        ),
    )
}

/// The order placements are written in: by `order`, then by where and
/// what they are, so that two of one order are still written the same
/// way every time.
fn written_order(placement: &Placement) -> (u16, i32, i32, i32, u32, u64) {
    (
        placement.order,
        placement.origin.y,
        placement.origin.x,
        placement.origin.z,
        placement.map.raw(),
        placement.seed,
    )
}

impl StandardGenerator {
    /// Makes the generator a realm definition asks for, as a registry
    /// holds it.
    pub fn from_request(
        defs: &Arc<GenDefs>,
        request: &GeneratorRequest<'_>,
    ) -> Result<Arc<dyn WorldGenerator>, GenError> {
        Ok(Arc::new(StandardGenerator::new(defs, request)?))
    }

    /// The generator of a realm definition by its qualified id, for
    /// tools and tests that ask it what it would place.
    pub fn of_realm(defs: &Arc<GenDefs>, realm: &str) -> Result<StandardGenerator, GenError> {
        let world = defs.world();
        let missing = || GenError::NoGenerator {
            realm: realm.to_owned(),
        };
        let id = world.realms().lookup(realm).ok_or_else(missing)?;
        let kind = world.realms().get(id).ok_or_else(missing)?;
        StandardGenerator::new(
            defs,
            &GeneratorRequest {
                defs: world,
                realm_def: id,
                realm_name: realm,
                kind,
            },
        )
    }

    /// Reads the parameters of a realm definition, or says what is
    /// wrong with them.
    pub fn new(
        defs: &Arc<GenDefs>,
        request: &GeneratorRequest<'_>,
    ) -> Result<StandardGenerator, GenError> {
        request.only(&PARAMS)?;
        let namespace = request
            .realm_name
            .split_once(':')
            .map(|(namespace, _)| namespace);
        let params = &request.kind.generator_params;
        let tract_named = |param: &str, name: &str| {
            defs.resolve_tract(name, namespace).ok_or_else(|| {
                request.error(param, format!("there is no tract definition `{name}`"))
            })
        };

        let mut tracts = Vec::new();
        if let Some(value) = params.get(TRACTS) {
            let wrong = || {
                request.error(
                    TRACTS,
                    "must be a list of `{ \"tract\": id, \"weight\": number }`",
                )
            };
            for entry in value.as_array().ok_or_else(wrong)? {
                let object = entry.as_object().ok_or_else(wrong)?;
                if object.keys().any(|key| key != "tract" && key != "weight") {
                    return Err(wrong());
                }
                let name = object
                    .get("tract")
                    .and_then(|v| v.as_str())
                    .ok_or_else(wrong)?;
                let weight = match object.get("weight") {
                    None => Real::ONE,
                    Some(weight) => weight
                        .as_f64()
                        .and_then(|weight| Real::new(weight).ok())
                        .filter(|weight| *weight >= Real::ZERO && weight.is_finite())
                        .ok_or_else(|| {
                            request.error(TRACTS, "a weight is a number that is not negative")
                        })?,
                };
                tracts.push((tract_named(TRACTS, name)?, weight));
            }
        }
        let total = tracts
            .iter()
            .fold(Real::ZERO, |sum, (_, weight)| sum + *weight);

        let patch = request.int(PATCH, 1, 1..=i64::from(REGION_TRACTS))?;

        let layout = match params.get(LAYOUT) {
            None => None,
            Some(value) => {
                let wrong = |what: &str| request.error(LAYOUT, what.to_owned());
                let object = value
                    .as_object()
                    .ok_or_else(|| wrong("must be an object with `at`, `rows` and `legend`"))?;
                if let Some(unknown) = object
                    .keys()
                    .find(|key| !["at", "rows", "legend"].contains(&key.as_str()))
                {
                    return Err(wrong(&format!(
                        "has no key `{unknown}`; it takes `at`, `rows` and `legend`"
                    )));
                }
                let at = match object.get("at") {
                    None => TractPos::new(0, 0),
                    Some(at) => {
                        let pair: Option<Vec<i32>> = at.as_array().map(|list| {
                            list.iter()
                                .filter_map(|n| n.as_i64().and_then(|n| i32::try_from(n).ok()))
                                .collect()
                        });
                        match pair.as_deref() {
                            Some([x, y]) if at.as_array().is_some_and(|list| list.len() == 2) => {
                                TractPos::new(*x, *y)
                            }
                            _ => return Err(wrong("`at` is `[x, y]`, a tract")),
                        }
                    }
                };
                let mut legend: BTreeMap<char, TractDefId> = BTreeMap::new();
                let entries = object
                    .get("legend")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        wrong("`legend` says which tract definition each character stands for")
                    })?;
                for (key, name) in entries {
                    let mut chars = key.chars();
                    let (Some(ch), None) = (chars.next(), chars.next()) else {
                        return Err(wrong(&format!(
                            "a legend entry is named by one character, and `{key}` is not one"
                        )));
                    };
                    let name = name
                        .as_str()
                        .ok_or_else(|| wrong("a legend entry names a tract definition"))?;
                    legend.insert(ch, tract_named(LAYOUT, name)?);
                }
                let rows = object
                    .get("rows")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| wrong("`rows` is a list of rows of characters"))?;
                if rows.len() > MAX_LAYOUT_SIDE {
                    return Err(wrong(&format!("has at most {MAX_LAYOUT_SIDE} rows")));
                }
                let mut read = Vec::new();
                for row in rows {
                    let row = row
                        .as_str()
                        .ok_or_else(|| wrong("a row is a text of characters"))?;
                    if row.chars().count() > MAX_LAYOUT_SIDE {
                        return Err(wrong(&format!(
                            "a row has at most {MAX_LAYOUT_SIDE} characters"
                        )));
                    }
                    let mut cells = Vec::new();
                    for ch in row.chars() {
                        cells.push(match ch {
                            ' ' => None,
                            ch => Some(*legend.get(&ch).ok_or_else(|| {
                                wrong(&format!("the character `{ch}` is not in the legend"))
                            })?),
                        });
                    }
                    read.push(cells);
                }
                Some(Layout { at, rows: read })
            }
        };

        let height = match params.get(HEIGHT) {
            None => None,
            Some(value) => {
                let mut field: FieldSpec = serde_json::from_value(value.clone())
                    .map_err(|error| request.error(HEIGHT, error.to_string()))?;
                field
                    .validate()
                    .map_err(|error| request.error(HEIGHT, error.to_string()))?;
                if field.shape.needs_extent() {
                    let topology = request.kind.topology;
                    let side = |extent: clash_world::Extent| match extent {
                        clash_world::Extent::Bounded(size) | clash_world::Extent::Wrapped(size) => {
                            Some(Real::from_i64(i64::from(size)) / Real::from_i64(TRACT_XY.into()))
                        }
                        clash_world::Extent::Unbounded => None,
                    };
                    let (Some(width), Some(depth)) = (side(topology.x), side(topology.y)) else {
                        return Err(request.error(
                            HEIGHT,
                            "this shape fills its realm, and the realm has no extent",
                        ));
                    };
                    field = field.within(Extent::new(Real::ZERO, Real::ZERO, width, depth));
                }
                Some(field)
            }
        };
        let surface_z = request.int(SURFACE_Z, 0, -MAX_SURFACE_Z..=MAX_SURFACE_Z)? as i32;
        // The sea is the realm definition's own (02 §4.1).
        let sea_level = request.kind.sea_level;

        // What is generated must stand (10 §5.2, X9): a run of strata
        // that do not anchor carries its own weight on its lowest
        // storey, in this realm's gravity (02 §14.5).
        let gravity = request
            .kind
            .strata
            .iter()
            .fold(Real::ZERO, |most, stratum| most.max(stratum.gravity));
        let used = tracts
            .iter()
            .map(|(def, _)| (TRACTS, *def))
            .chain(layout.iter().flat_map(|layout: &Layout| {
                layout
                    .rows
                    .iter()
                    .flatten()
                    .flatten()
                    .map(|def| (LAYOUT, *def))
            }));
        for (param, def) in used {
            let ground = defs
                .tract(def)
                .and_then(TractKind::ground)
                .and_then(|ground| defs.ground(ground));
            if let Some(why) = ground.and_then(|ground| ground.crushed_by(defs.world(), gravity)) {
                return Err(request.error(param, why));
            }
        }

        let mut authored = Vec::new();
        if let Some(value) = params.get(PLACE) {
            let wrong = |what: String| request.error(PLACE, what);
            let list = value
                .as_array()
                .ok_or_else(|| wrong("must be a list of placements".to_owned()))?;
            if list.len() > MAX_AUTHORED {
                return Err(wrong(format!("has at most {MAX_AUTHORED} placements")));
            }
            let maps = defs.maps();
            for (index, entry) in list.iter().enumerate() {
                let object = entry
                    .as_object()
                    .ok_or_else(|| wrong("a placement is an object".to_owned()))?;
                const KEYS: [&str; 7] = ["map", "at", "turn", "mirror", "seed", "params", "order"];
                if let Some(unknown) = object.keys().find(|key| !KEYS.contains(&key.as_str())) {
                    return Err(wrong(format!(
                        "a placement has no key `{unknown}`; it takes: {}",
                        KEYS.join(", ")
                    )));
                }
                let name = object
                    .get("map")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| wrong("a placement names its `map`".to_owned()))?;
                let map = map_named(maps, name, namespace)
                    .ok_or_else(|| wrong(format!("there is no map `{name}`")))?;
                let at: Vec<i32> = object
                    .get("at")
                    .and_then(|v| v.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|n| n.as_i64().and_then(|n| i32::try_from(n).ok()))
                            .collect()
                    })
                    .unwrap_or_default();
                let (x, y, z) = match at.as_slice() {
                    [x, y] => (*x, *y, None),
                    [x, y, z] => (*x, *y, Some(*z)),
                    _ => {
                        return Err(wrong(
                            "`at` is `[x, y]` or `[x, y, z]`, a cell of the realm".to_owned(),
                        ));
                    }
                };
                let rot = match object.get("turn") {
                    None => Rot4::R0,
                    Some(turn) => match turn.as_i64() {
                        Some(degrees) if degrees.rem_euclid(90) == 0 => {
                            Rot4::from_quarter_turns((degrees.rem_euclid(360) / 90) as i32)
                        }
                        _ => {
                            return Err(wrong(
                                "`turn` is degrees clockwise: 0, 90, 180 or 270".to_owned(),
                            ));
                        }
                    },
                };
                let mirror = match object.get("mirror") {
                    None => false,
                    Some(mirror) => mirror
                        .as_bool()
                        .ok_or_else(|| wrong("`mirror` is true or false".to_owned()))?,
                };
                let seed = match object.get("seed") {
                    None => None,
                    Some(seed) => Some(
                        seed.as_u64()
                            .ok_or_else(|| wrong("`seed` is a whole number".to_owned()))?,
                    ),
                };
                let order = match object.get("order") {
                    None => u16::try_from(index).unwrap_or(u16::MAX),
                    Some(order) => order
                        .as_u64()
                        .and_then(|order| u16::try_from(order).ok())
                        .ok_or_else(|| wrong("`order` is from 0 to 65535".to_owned()))?,
                };
                let mut given = VarMap::new();
                if let Some(values) = object.get("params") {
                    let values = values
                        .as_object()
                        .ok_or_else(|| wrong("`params` is an object of plain values".to_owned()))?;
                    for (param, value) in values {
                        let value = match value {
                            serde_json::Value::Bool(value) => clash_core::Value::Bool(*value),
                            serde_json::Value::String(text) => clash_core::Value::Str(text.clone()),
                            serde_json::Value::Number(number) => match number.as_i64() {
                                Some(int) => clash_core::Value::Int(int),
                                None => clash_core::Value::Real(
                                    number
                                        .as_f64()
                                        .and_then(|real| Real::new(real).ok())
                                        .ok_or_else(|| {
                                            wrong(format!(
                                                "the parameter `{param}` is not a number"
                                            ))
                                        })?,
                                ),
                            },
                            _ => {
                                return Err(wrong(format!(
                                    "the parameter `{param}` is a number, a boolean or a text"
                                )));
                            }
                        };
                        given.set(param.as_str(), value);
                    }
                }
                let turn = Turn::new(rot, mirror);
                // A dry run: the placement must resolve.
                maps.resolve(map, &given, seed.unwrap_or(0), turn)
                    .map_err(|error| wrong(error.to_string()))?;
                authored.push(Authored {
                    map,
                    x,
                    y,
                    z,
                    turn,
                    seed,
                    params: given,
                    order,
                });
            }
        }

        Ok(StandardGenerator {
            defs: Arc::clone(defs),
            tracts,
            total,
            patch: patch as i32,
            layout,
            height,
            surface_z,
            sea_level,
            authored,
            bare: Ground {
                name: String::new(),
                params: Vec::new(),
                depth: 0,
                strata: Vec::new(),
                steps: Vec::new(),
                scatter_reach: 0,
            },
        })
    }

    /// The definitions the generator realises from.
    pub fn defs(&self) -> &Arc<GenDefs> {
        &self.defs
    }

    /// The lowest storey that is not sea, if the realm has a sea.
    pub fn sea_level(&self) -> Option<i32> {
        self.sea_level
    }

    /// The surface the height field gives at a cell column: the lowest
    /// storey that is not ground, before any map has levelled it. A
    /// pure function of the seed and the position.
    pub fn surface(&self, seed: u64, x: i32, y: i32) -> i32 {
        let Some(field) = &self.height else {
            return self.surface_z;
        };
        // A field is sampled at a position in tracts (10 §9.1).
        let tracts = Real::from_i64(TRACT_XY.into());
        let value = field.sample(
            seed,
            HEIGHT_DOMAIN,
            Real::from_i64(x.into()) / tracts,
            Real::from_i64(y.into()) / tracts,
        );
        self.surface_z
            .saturating_add((value + Real::of(0.5)).floor().to_i32())
    }

    /// The definition a tract is given: the layout's, or else one of
    /// the realm's list by weight, drawn from the positional hash of
    /// the tract's patch.
    ///
    /// This is where a region's kind will decide (10 §8.1, §8.2): the
    /// packages that survey regions replace it.
    pub fn tract_def(&self, seed: u64, tract: TractPos) -> TractDefId {
        if let Some(def) = self.layout.as_ref().and_then(|layout| layout.get(tract)) {
            return def;
        }
        if self.tracts.is_empty() || self.total <= Real::ZERO {
            return TractDefId::new(0);
        }
        let patch = (
            tract.x.div_euclid(self.patch),
            tract.y.div_euclid(self.patch),
        );
        let mut left = pos_rng(seed, TRACT_DOMAIN, patch.0, patch.1, 0, 0).unit() * self.total;
        for (def, weight) in &self.tracts {
            if left < *weight {
                return *def;
            }
            left -= *weight;
        }
        self.tracts
            .last()
            .map_or(TractDefId::new(0), |(def, _)| *def)
    }

    /// The storey a map's level 0 goes at, by what the map sits on
    /// (10 §3.1, `base`).
    fn base_level(&self, seed: u64, base: Base, x: i32, y: i32) -> i32 {
        match base {
            Base::Ground | Base::SeaFloor => self.surface(seed, x, y),
            Base::SeaLevel => self.sea_level.unwrap_or_else(|| self.surface(seed, x, y)),
            Base::Storey(z) => z,
        }
    }

    /// The realm's own placements whose boxes meet a box.
    fn authored_in(&self, seed: u64, within: CellBox) -> Vec<Placement> {
        let maps = self.defs.maps();
        self.authored
            .iter()
            .enumerate()
            .filter_map(|(index, authored)| {
                let compiled = maps.get(authored.map)?;
                let z = authored.z.unwrap_or_else(|| {
                    self.base_level(seed, compiled.base(), authored.x, authored.y)
                });
                let placement = Placement {
                    map: place::world_map_id(authored.map),
                    origin: CellPos::new(authored.x, authored.y, z),
                    turn: authored.turn,
                    params: authored.params.clone(),
                    seed: authored.seed.unwrap_or_else(|| {
                        pos_hash(seed, SEED_DOMAIN, authored.x, authored.y, 0, index as u64)
                    }),
                    order: authored.order,
                    site: None,
                };
                let bounds = placement_box(maps, &placement)?;
                boxes_overlap(bounds, within).then_some(placement)
            })
            .collect()
    }

    /// What stands on a tract of a definition, besides what the realm
    /// places by hand: the definition's own placements, each made or
    /// not by its chance, its map and turn chosen, and tested against
    /// those written before it (10 §4.1, X10). Returns the placements
    /// written, in the order of the definition, and those dropped.
    ///
    /// A pure function of the seed, the position and the definition.
    /// This is where a settlement's lots will be written (10 §8.4).
    pub fn placements_of(
        &self,
        seed: u64,
        tract: TractPos,
        def: TractDefId,
    ) -> (Vec<Placement>, Vec<DroppedPlacement>) {
        let (mut written, mut dropped) = (Vec::new(), Vec::new());
        let Some(kind) = self.defs.tract(def).filter(|kind| !kind.place.is_empty()) else {
            return (written, dropped);
        };
        let maps = self.defs.maps();
        let origin = tract.origin(0);
        let mut boxes: Vec<CellBox> = self
            .authored_in(seed, tract_box(tract, 0))
            .iter()
            .filter_map(|placement| placement_box(maps, placement))
            .collect();
        for (index, entry) in kind.place.iter().enumerate() {
            let mut rng = pos_rng(seed, PLACE_DOMAIN, tract.x, tract.y, 0, index as u64);
            if !rng.chance(entry.chance) {
                continue;
            }
            let Some(&(map, turn)) = rng.pick(&entry.options) else {
                continue;
            };
            let Some(compiled) = maps.get(map) else {
                continue;
            };
            let (x, y) = (origin.x + entry.at.x, origin.y + entry.at.y);
            let placement = Placement {
                map: place::world_map_id(map),
                origin: CellPos::new(x, y, self.base_level(seed, compiled.base(), x, y)),
                turn,
                params: entry.params.clone(),
                seed: pos_hash(seed, SEED_DOMAIN, x, y, 1, index as u64),
                order: entry.order,
                site: None,
            };
            let Some(bounds) = placement_box(maps, &placement) else {
                continue;
            };
            let met = boxes
                .iter()
                .copied()
                .find(|other| boxes_overlap(bounds, *other))
                .filter(|_| !compiled.over());
            match met {
                Some(met) => dropped.push(DroppedPlacement {
                    tract,
                    placement,
                    bounds,
                    met,
                }),
                None => {
                    boxes.push(bounds);
                    written.push(placement);
                }
            }
        }
        (written, dropped)
    }

    /// The placements a survey of a region could not write, with what
    /// each met: the survey's report (10 §13, `placement_dropped`).
    pub fn dropped_in(&self, seed: u64, region: RegionPos) -> Vec<DroppedPlacement> {
        (0..REGION_TRACT_COUNT)
            .flat_map(|index| {
                let tract = region.tract(index);
                self.placements_of(seed, tract, self.tract_def(seed, tract))
                    .1
            })
            .collect()
    }

    fn near(&self, placement: Placement) -> Option<Near> {
        let maps = self.defs.maps();
        let compiled = maps.get(place::gen_map_id(placement.map))?;
        let bounds = placement_box(maps, &placement)?;
        let level = nested_origin(
            compiled.size(),
            compiled.anchor(),
            placement.turn,
            placement.origin.to_ivec3(),
        )
        .z;
        Some(Near {
            sit: compiled.sit(),
            placement,
            bounds,
            level,
        })
    }

    /// Where transforms run (10 §6.1 step 4; §10): after the ground and
    /// the placements, before the result is handed on. Transforms are a
    /// later package; nothing is done here yet.
    pub fn transforms(&self, _realm: &RealmInfo, _tract: TractPos, _rec: &TractRecord) {}

    fn kind_and_ground(&self, def: TractDefId) -> (Option<&TractKind>, &Ground) {
        let kind = self.defs.tract(def);
        let ground = kind
            .and_then(TractKind::ground)
            .and_then(|ground| self.defs.ground(ground))
            .unwrap_or(&self.bare);
        (kind, ground)
    }
}

impl RegionSurveyor for StandardGenerator {
    fn survey(&self, _: &GenCtx<'_>, realm: &RealmInfo, pos: RegionPos) -> RegionSurveyResult {
        let mut record = RegionRecord::new(RegionSurvey::default());
        let first = pos.origin().origin(0);
        let whole = CellBox::new(
            CellPos::new(first.x, first.y, i32::MIN),
            CellPos::new(
                first.x + REGION_CELLS - 1,
                first.y + REGION_CELLS - 1,
                i32::MAX,
            ),
        );
        // What the realm places by hand is listed in every region it
        // touches.
        record.placements = self.authored_in(realm.seed, whole);
        record.placements.sort_by_key(written_order);
        for index in 0..REGION_TRACT_COUNT {
            let tract = pos.tract(index);
            let def = self.tract_def(realm.seed, tract);
            record.tract_mut(tract).def = def;
            record
                .placements
                .extend(self.placements_of(realm.seed, tract, def).0);
        }
        RegionSurveyResult::new(record)
    }
}

impl TractRealizer for StandardGenerator {
    fn realize(
        &self,
        cx: &GenCtx<'_>,
        realm: &RealmInfo,
        pos: TractPos,
        rec: &TractRecord,
        around: &TractRecords3x3,
    ) -> RealizedTract {
        let seed = realm.seed;
        let maps = self.defs.maps();
        let world: &WorldDefs = cx.defs();

        // Every placement that overlaps the tract or lies close to it.
        let mut near: Vec<Near> = self
            .authored_in(seed, tract_box(pos, GATHER_MARGIN))
            .into_iter()
            .filter_map(|placement| self.near(placement))
            .collect();
        for (dx, dy) in TractRecords3x3::OFFSETS {
            let record = if (dx, dy) == (0, 0) {
                Some(rec)
            } else {
                around.get(dx, dy)
            };
            let Some(record) = record else { continue };
            let tract = pos.offset(IVec2::new(dx, dy));
            let (written, _) = self.placements_of(seed, tract, record.def);
            near.extend(
                written
                    .into_iter()
                    .filter_map(|placement| self.near(placement)),
            );
        }
        near.sort_by_key(|near| written_order(&near.placement));

        // 1. The surface: the height field, flat under levelled maps.
        let height = |x: i32, y: i32| {
            near.iter()
                .rev()
                .find(|near| near.sit == Sit::Level && near.covers(x, y))
                .map_or_else(|| self.surface(seed, x, y), |near| near.level)
        };
        let built = |x: i32, y: i32| {
            near.iter().any(|near| {
                matches!(near.sit, Sit::Level | Sit::Foundation(_)) && near.covers(x, y)
            })
        };
        let (kind, ground) = self.kind_and_ground(rec.def);
        let heights = Heights::new(pos, ground.margin(), height);
        let (bottom, _) = ground.bottom(realm.below_fill);
        let origin = pos.origin(0);
        let lowest = heights.lowest(origin);
        let mut canvas = TractCanvas {
            builder: ColumnBuilder::new(world, bottom, realm.above_fill, lowest),
            origin,
            heights: &heights,
        };

        // 2. The ground.
        let run = GroundRun {
            world,
            maps,
            exprs: self.defs.exprs(),
            seed,
            tract: pos,
            params: kind.map_or_else(Vec::new, |kind| {
                self.defs.ground_params(kind, ground, seed, pos)
            }),
            sea_level: self.sea_level,
            realm_fill: realm.below_fill,
            built: &built,
        };
        let mut placed: Vec<PlacedMap> = ground.run(&run, &mut canvas);

        // 3 and 4. Foundations, then the placements in order, each
        // clipped to the tract.
        let own = tract_box(pos, 0);
        // The tract's box takes in every storey, so this tests the
        // footprint: a map that follows the ground may lie outside the
        // storeys of its box.
        let drawn: Vec<PlacedMap> = near
            .iter()
            .filter(|near| boxes_overlap(near.bounds, own))
            .filter_map(|near| PlacedMap::resolve(maps, &near.placement).ok().flatten())
            .collect();
        for map in &drawn {
            place::prepare(&mut canvas, map, MapMode::Replace);
        }
        for map in &drawn {
            place::draw(&mut canvas, map, MapMode::Replace);
        }
        placed.extend(drawn);

        // 5. Transforms.
        self.transforms(realm, pos, rec);

        // 6. What the maps ask for besides cells.
        let mut spawns = Vec::new();
        let mut areas = Vec::new();
        for map in &placed {
            spawns.extend(
                map.spawns()
                    .into_iter()
                    .filter(|spawn| canvas.covers(spawn.at.x, spawn.at.y)),
            );
            areas.extend(
                map.areas(realm.base_grid)
                    .into_iter()
                    .filter(|(home, _)| canvas.covers(home.x, home.y))
                    .map(|(_, spec)| spec),
            );
        }
        let mut realised = RealizedTract::new(canvas.builder.build());
        realised.spawns = spawns;
        realised.areas = areas;
        realised
    }
}

/// Registers the `standard` generator, built from these definitions,
/// under the name realm definitions give it by.
pub fn register_standard(registry: &mut GeneratorRegistry, defs: &Arc<GenDefs>) {
    let defs = Arc::clone(defs);
    registry.register_maker(
        STANDARD_GENERATOR,
        Arc::new(move |request: &GeneratorRequest<'_>| {
            StandardGenerator::from_request(&defs, request)
        }),
    );
}

/// Stands in for the `standard` generator in a registry that was given
/// no generation definitions: it says how to get one.
pub(crate) fn standard_unbound(
    request: &GeneratorRequest<'_>,
) -> Result<Arc<dyn WorldGenerator>, GenError> {
    Err(GenError::BadParam {
        realm: request.realm_name.to_owned(),
        path: "generator".to_owned(),
        message: format!(
            "the generator `{STANDARD_GENERATOR}` is built from maps, grounds and tract \
             definitions: bind the generators with `clash_gen::generators_with`"
        ),
    })
}

/// A registry holding every generator of this crate, the `standard`
/// one built from these definitions, which realises batches of tracts
/// on the threads of the process's pool.
pub fn registry_with(defs: &Arc<GenDefs>) -> GeneratorRegistry {
    let mut registry = crate::registry();
    register_standard(&mut registry, defs);
    registry
}

/// Every generator of this crate, bound to a set of definitions: what
/// loading tracts takes. The world's tables are those the generation
/// definitions were built against.
pub fn generators_with(defs: &Arc<GenDefs>) -> Generators {
    registry_with(defs).bind(defs.world())
}
