//! Tract definitions (10 §6): the `tract` definition type, and
//! [`GenDefs`], everything generation is built from.
//!
//! ```json
//! { "type": "tract", "id": "lane", "tags": ["settled"],
//!   "ground": "lowland",
//!   "params": { "trees": 0.0 },
//!   "place": [ { "map": "lane_paving", "at": [0, 14] },
//!              { "map": { "tag": "house" }, "at": [4, 4], "facing": "south", "chance": 0.9 } ],
//!   "bands": ["overland"],
//!   "glyph": { "ch": "=", "fg": "gray" } }
//! ```
//!
//! A tract definition says what the ground of a tract is, what stands
//! on it, which travel bands may cross it, what it becomes when ruined
//! and how it is drawn on the region map.
//!
//! # From files to generators
//!
//! 1. [`register_defs`](crate::register_defs) gives the loader the
//!    `map`, `palette`, `ground` and `tract` types.
//! 2. [`GenDefs::from_defs`] builds them against the world's tables:
//!    every state typed, every map found, every placement checked to
//!    fit and to resolve. Each mistake is a located diagnostic.
//! 3. [`generators_with`](crate::generators_with) binds the generators,
//!    the `standard` one among them, which realises tracts from these
//!    definitions.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dir8, IVec2, IVec3, Real, Rot4, Turn, VarMap, pos_rng};
use clash_data::expr::{Any, ExprDefs};
use clash_data::{CompileCx, Def, DefLabel, Defs, Diagnostics, Expr, Ref, Runner, Scope, TagSet};
use clash_world::dims::TRACT_XY;
use clash_world::{TractDefId, TractPos, WorldDefs};
use serde::Deserialize;

use crate::ground::{Ground, GroundDef, GroundId, Report, build_ground};
use crate::map::{
    GenCompiler, GenHost, Lit, MapDefs, MapId, MapPickDef, Sited, TurnDef, nested_origin,
    turned_size,
};

/// The name of the tract definition that is no definition: a tract with
/// dense id 0, as the world's records have it (02 §9.1).
pub const NONE_TRACT: &str = "none";

/// The domain of the generator the random functions of a tract's
/// parameter expressions draw from.
const PARAM_DOMAIN: clash_core::HashDomain = clash_core::HashDomain::new("gen.tract.param");

/// How a tract is drawn on the region map (12 §6.2). A presentation
/// hint: generation carries it and gives it no meaning.
#[derive(Clone, PartialEq, Eq, Debug, Default, Deserialize)]
pub struct TractGlyph {
    /// The character.
    #[serde(default)]
    pub ch: Option<String>,
    /// The foreground colour, by name.
    #[serde(default)]
    pub fg: Option<String>,
    /// The background colour, by name.
    #[serde(default)]
    pub bg: Option<String>,
}

/// One entry of a tract definition's `place`: a map that stands on
/// every tract of the definition.
#[derive(Clone, Debug, Deserialize)]
pub struct TractPlaceDef {
    /// Which map: an id, or `{ "tag": … }` for any map carrying the tag.
    pub map: MapPickDef,
    /// `[x, y]`: where the map's anchor goes, in cells east and south
    /// of the tract's north-west corner.
    pub at: Sited<Vec<i64>>,
    /// How it is turned, as a map's own `place` writes it. Not turned
    /// if left out.
    #[serde(default)]
    pub turn: Option<TurnDef>,
    /// In place of `turn`: the way the map's `front` is to face. The
    /// map is rotated so that it does.
    #[serde(default)]
    pub facing: Option<Sited<Dir8>>,
    /// The chance that it is placed at all, from 0 to 1.
    #[serde(default)]
    pub chance: Option<Sited<Real>>,
    /// Parameters for the map: plain values.
    #[serde(default)]
    pub params: BTreeMap<String, Lit>,
    /// Later placements overwrite earlier where they overlap. Its
    /// position in the list if left out.
    #[serde(default)]
    pub order: Option<u16>,
}

/// What kind of place a tract is (10 §6).
#[derive(Debug, Default, Deserialize)]
pub struct TractDef {
    /// Free-form tags.
    #[serde(default)]
    pub tags: TagSet,
    /// What its ground is. Left out, the tract is the realm's own fill
    /// up to the surface and nothing else.
    #[serde(default)]
    pub ground: Option<Ref<GroundDef>>,
    /// Values for the ground's parameters: each a plain value or an
    /// expression.
    #[serde(default)]
    pub params: BTreeMap<String, Expr<Any>>,
    /// The maps that stand on it.
    #[serde(default)]
    pub place: Vec<TractPlaceDef>,
    /// What it becomes when ruined (10 §10.2). Carried as written for
    /// the package that builds transforms.
    #[serde(default)]
    pub ruin: Option<Sited<serde_json::Value>>,
    /// The travel bands that may cross it (11 §13.2), by id as written.
    #[serde(default)]
    pub bands: Vec<String>,
    /// How it is drawn on the region map.
    #[serde(default)]
    pub glyph: Option<TractGlyph>,
}

impl Def for TractDef {
    type Id = TractDefId;
    const TYPE: &'static str = "tract";
    const DOC: &'static str = "What kind of place a tract is: its ground, the maps that stand on \
                               it, the travel bands that cross it, its ruin and its glyph \
                               (10 §6).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        // The generation context with nothing bound: `region.*` joins
        // it when regions are surveyed (10 §13).
        let mut gen_cx = GenCompiler {
            cx,
            scope: Scope::new(),
        };
        for value in self.params.values_mut() {
            gen_cx.expr(value);
        }
    }

    fn builtins() -> Vec<(&'static str, TractDef)> {
        // Dense id 0 is a tract with no definition (02 §9.1).
        vec![(NONE_TRACT, TractDef::default())]
    }
}

/// One map a tract definition places, made ready.
#[derive(Clone, Debug)]
pub struct TractPlace {
    /// The maps it may be, each with the turn it would be given. Every
    /// one fits inside the tract and resolves.
    pub(crate) options: Vec<(MapId, Turn)>,
    /// Where the anchor goes, from the tract's north-west corner.
    pub(crate) at: IVec2,
    pub(crate) chance: Real,
    pub(crate) params: VarMap,
    pub(crate) order: u16,
}

/// A tract definition made ready to realise tracts from.
#[derive(Clone, Debug)]
pub struct TractKind {
    pub(crate) name: String,
    pub(crate) tags: Vec<String>,
    pub(crate) ground: Option<GroundId>,
    /// For each parameter of the ground, in its order: what the tract
    /// gives it, if anything.
    pub(crate) params: Vec<Option<Expr<Any>>>,
    pub(crate) place: Vec<TractPlace>,
    pub(crate) ruin: Option<serde_json::Value>,
    pub(crate) bands: Vec<String>,
    pub(crate) glyph: Option<TractGlyph>,
}

impl TractKind {
    /// The definition's qualified id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its tags, sorted.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Its ground, if it has one.
    pub fn ground(&self) -> Option<GroundId> {
        self.ground
    }

    /// The travel bands that may cross it, as written.
    pub fn bands(&self) -> &[String] {
        &self.bands
    }

    /// How it is drawn on the region map.
    pub fn glyph(&self) -> Option<&TractGlyph> {
        self.glyph.as_ref()
    }

    /// What it becomes when ruined, as written (10 §10.2).
    pub fn ruin(&self) -> Option<&serde_json::Value> {
        self.ruin.as_ref()
    }

    /// How many maps it places.
    pub fn place_count(&self) -> usize {
        self.place.len()
    }
}

/// A parameter value of the type of a default, or `None` if it is of
/// another type. A whole number is taken for a real.
fn accept(default: &clash_core::Value, given: clash_core::Value) -> Option<clash_core::Value> {
    use clash_core::Value;
    match (default, given) {
        (Value::Real(_), Value::Int(int)) => Some(Value::Real(Real::from_i64(int))),
        (default, given) if default.type_name() == given.type_name() => Some(given),
        _ => None,
    }
}

fn core_value(value: &clash_expr::Value) -> Option<clash_core::Value> {
    match value {
        clash_expr::Value::Bool(value) => Some(clash_core::Value::Bool(*value)),
        clash_expr::Value::Int(value) => Some(clash_core::Value::Int(*value)),
        clash_expr::Value::Real(value) => Some(clash_core::Value::Real(*value)),
        clash_expr::Value::Str(value) => Some(clash_core::Value::Str(value.to_string())),
        _ => None,
    }
}

/// Everything generation is built from: the compiled maps, grounds and
/// tract definitions of a load, with the world's tables they were
/// resolved against. Built once and shared behind an `Arc`.
#[derive(Clone, Debug)]
pub struct GenDefs {
    maps: Arc<MapDefs>,
    world: Arc<WorldDefs>,
    exprs: ExprDefs,
    grounds: Vec<Ground>,
    ground_names: BTreeMap<String, GroundId>,
    tracts: Vec<TractKind>,
    tract_names: BTreeMap<String, TractDefId>,
}

// `GenDefs` is shared between threads: tracts are realised in parallel.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<GenDefs>();
};

impl GenDefs {
    /// Builds the maps, grounds and tract definitions of a load against
    /// the world's tables.
    ///
    /// Every mistake found is returned as a located diagnostic. This
    /// never panics, whatever the definitions hold.
    pub fn from_defs(defs: &Defs, world: &Arc<WorldDefs>) -> Result<Arc<GenDefs>, Diagnostics> {
        let maps = MapDefs::from_defs(defs, world)?;
        let mut report = Report {
            defs,
            world,
            diagnostics: Vec::new(),
        };
        let mut grounds = Vec::new();
        let mut ground_names = BTreeMap::new();
        if let Some(registry) = defs.try_registry::<GroundDef>() {
            for (id, def) in registry.iter() {
                let name = registry.name(id);
                ground_names.insert(name.to_owned(), id);
                grounds.push(build_ground(&mut report, &maps, name, def));
            }
        }
        let mut tracts = Vec::new();
        let mut tract_names = BTreeMap::new();
        if let Some(registry) = defs.try_registry::<TractDef>() {
            for (id, def) in registry.iter() {
                let name = registry.name(id);
                tract_names.insert(name.to_owned(), id);
                let label = DefLabel::new(TractDef::TYPE, name);
                tracts.push(build_tract(
                    &mut report,
                    &maps,
                    &grounds,
                    defs,
                    &label,
                    name,
                    def,
                ));
            }
        }
        if !report.diagnostics.is_empty() {
            let mut diagnostics = Diagnostics::new();
            for diagnostic in report.diagnostics {
                diagnostics.push(diagnostic);
            }
            diagnostics.sort();
            return Err(diagnostics);
        }
        Ok(Arc::new(GenDefs {
            maps,
            world: Arc::clone(world),
            exprs: defs.exprs().clone(),
            grounds,
            ground_names,
            tracts,
            tract_names,
        }))
    }

    /// The compiled maps.
    pub fn maps(&self) -> &Arc<MapDefs> {
        &self.maps
    }

    /// The world's tables everything was resolved against.
    pub fn world(&self) -> &Arc<WorldDefs> {
        &self.world
    }

    pub(crate) fn exprs(&self) -> &ExprDefs {
        &self.exprs
    }

    /// A ground by its dense id.
    pub fn ground(&self, id: GroundId) -> Option<&Ground> {
        self.grounds.get(id.index())
    }

    /// The dense id of a ground by its qualified id.
    pub fn lookup_ground(&self, qualified: &str) -> Option<GroundId> {
        self.ground_names.get(qualified).copied()
    }

    /// A tract definition by its dense id. Id 0 is the definition that
    /// is none: no ground and nothing placed.
    pub fn tract(&self, id: TractDefId) -> Option<&TractKind> {
        self.tracts.get(id.index())
    }

    /// The dense id of a tract definition by its qualified id.
    pub fn lookup_tract(&self, qualified: &str) -> Option<TractDefId> {
        self.tract_names.get(qualified).copied()
    }

    /// The dense id of a tract definition named in the namespace of a
    /// mod: a qualified id, or a bare name looked up in the namespace
    /// first.
    pub fn resolve_tract(&self, name: &str, namespace: Option<&str>) -> Option<TractDefId> {
        if name.contains(':') {
            return self.lookup_tract(name);
        }
        namespace
            .and_then(|namespace| self.lookup_tract(&format!("{namespace}:{name}")))
            .or_else(|| {
                let suffix = format!(":{name}");
                let mut found = self
                    .tract_names
                    .iter()
                    .filter(|(qualified, _)| qualified.ends_with(&suffix));
                match (found.next(), found.next()) {
                    (Some((_, id)), None) => Some(*id),
                    _ => None,
                }
            })
    }

    /// Every tract definition with its id, in id order.
    pub fn tracts(&self) -> impl Iterator<Item = (TractDefId, &TractKind)> + '_ {
        self.tracts
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| Some((TractDefId::from_index(index)?, kind)))
    }

    /// The values of a ground's parameters for a tract of a definition:
    /// what the definition gives, worked out for the tract, and the
    /// ground's defaults for the rest.
    pub(crate) fn ground_params(
        &self,
        kind: &TractKind,
        ground: &Ground,
        seed: u64,
        tract: TractPos,
    ) -> Vec<clash_expr::Value> {
        let mut runner = Runner::new();
        let mut host = GenHost {
            params: Vec::new(),
            rng: clash_core::Rng::from_seed(0),
            warnings: 0,
        };
        ground
            .params
            .iter()
            .enumerate()
            .map(|(index, (_, default))| {
                let given = kind
                    .params
                    .get(index)
                    .and_then(Option::as_ref)
                    .and_then(|expr| {
                        host.rng = pos_rng(seed, PARAM_DOMAIN, tract.x, tract.y, 0, index as u64);
                        let value = expr.value(&mut runner.over(&self.exprs, &mut host));
                        accept(default, core_value(&value)?)
                    });
                clash_expr::Value::from(given.unwrap_or_else(|| default.clone()))
            })
            .collect()
    }
}

/// The rotation that brings one compass direction to another, if both
/// are cardinal.
fn rotation_between(from: Dir8, to: Dir8) -> Option<Rot4> {
    Rot4::ALL.into_iter().find(|rot| from.rotated(*rot) == to)
}

/// Whether a map, turned and with its anchor at a cell of a tract,
/// lies wholly inside the tract.
fn fits_in_tract(maps: &MapDefs, map: MapId, turn: Turn, at: IVec2) -> bool {
    let Some(compiled) = maps.get(map) else {
        return false;
    };
    let corner = nested_origin(
        compiled.size(),
        compiled.anchor(),
        turn,
        IVec3::new(at.x, at.y, 0),
    );
    let size = turned_size(compiled.size(), turn);
    corner.x >= 0 && corner.y >= 0 && corner.x + size.x <= TRACT_XY && corner.y + size.y <= TRACT_XY
}

fn build_tract(
    report: &mut Report<'_>,
    maps: &MapDefs,
    grounds: &[Ground],
    defs: &Defs,
    label: &DefLabel,
    name: &str,
    def: &TractDef,
) -> TractKind {
    let ground_id = def.ground.map(Ref::id);
    let ground = ground_id.and_then(|id| grounds.get(id.index()));
    let mut params: Vec<Option<Expr<Any>>> = vec![None; ground.map_or(0, |g| g.params.len())];
    for (param, value) in &def.params {
        let slot = ground.and_then(|ground| {
            ground
                .params
                .iter()
                .position(|(declared, _)| declared == param)
        });
        let Some((slot, ground)) = slot.zip(ground) else {
            report.error(
                label,
                value.site(),
                match ground {
                    Some(ground) => format!(
                        "the ground `{}` has no parameter `{param}`; it has: {}",
                        ground.name,
                        ground
                            .params
                            .iter()
                            .map(|(declared, _)| declared.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None => format!(
                        "the tract has no ground, so there is no parameter `{param}` to give"
                    ),
                },
            );
            continue;
        };
        let default = &ground.params[slot].1;
        if let Some(constant) = value.constant()
            && core_value(constant)
                .and_then(|given| accept(default, given))
                .is_none()
        {
            report.error(
                label,
                value.site(),
                format!(
                    "the parameter `{param}` of the ground `{}` is a {}, and was given `{constant}`",
                    ground.name,
                    default.type_name()
                ),
            );
            continue;
        }
        params[slot] = Some(value.clone());
    }

    let mut place = Vec::new();
    for (index, entry) in def.place.iter().enumerate() {
        let at = match entry.at.value.as_slice() {
            [x, y]
                if (0..i64::from(TRACT_XY)).contains(x) && (0..i64::from(TRACT_XY)).contains(y) =>
            {
                IVec2::new(*x as i32, *y as i32)
            }
            _ => {
                report.error(
                    label,
                    &entry.at.site,
                    format!(
                        "`at` is `[x, y]`, a cell of the tract: each from 0 to {}",
                        TRACT_XY - 1
                    ),
                );
                continue;
            }
        };
        let chance = match &entry.chance {
            None => Real::ONE,
            Some(chance) if (Real::ZERO..=Real::ONE).contains(&chance.value) => chance.value,
            Some(chance) => {
                report.error(label, &chance.site, "a chance is from 0 to 1");
                continue;
            }
        };
        let mut given = VarMap::new();
        for (param, value) in &entry.params {
            given.set(param.as_str(), value.0.clone());
        }
        let candidates: Vec<MapId> = match &entry.map {
            MapPickDef::Id(map, _) => vec![map.id()],
            MapPickDef::Tag(tag, _) => maps.with_tag(tag).collect(),
        };
        let by_id = matches!(entry.map, MapPickDef::Id(..));
        let site = entry.map.site();
        if entry.turn.is_some() && entry.facing.is_some() {
            report.error(
                label,
                site,
                "a placement gives `turn` or `facing`, not both: `facing` is a turn worked out \
                 from the map's `front`",
            );
            continue;
        }
        let mut options = Vec::new();
        let mut why_not = None;
        for map in candidates {
            let Some(compiled) = maps.get(map) else {
                continue;
            };
            let turns: Vec<Turn> = match (&entry.facing, entry.turn) {
                (Some(facing), _) => match compiled
                    .front()
                    .and_then(|front| rotation_between(front, facing.value))
                {
                    Some(rot) => vec![Turn::new(rot, false)],
                    None => {
                        why_not = Some(format!(
                            "the map `{}` names no `front` that could face that way",
                            compiled.name()
                        ));
                        continue;
                    }
                },
                (None, Some(TurnDef::Fixed(turn))) => vec![turn],
                (None, Some(TurnDef::Any)) if compiled.may_turn() => Turn::ALL.to_vec(),
                (None, _) => vec![Turn::IDENTITY],
            };
            for turn in turns {
                if turn != Turn::IDENTITY && !compiled.may_turn() {
                    why_not = Some(format!("the map `{}` may not be turned", compiled.name()));
                    continue;
                }
                if !fits_in_tract(maps, map, turn, at) {
                    why_not = Some(format!(
                        "the map `{}` does not fit inside the tract there: a tract definition \
                         places maps that lie wholly within one tract",
                        compiled.name()
                    ));
                    continue;
                }
                // A dry run: the placement must resolve.
                match maps.resolve(map, &given, 0, turn) {
                    Ok(_) => options.push((map, turn)),
                    Err(error) => why_not = Some(error.to_string()),
                }
            }
        }
        if options.is_empty() || (by_id && why_not.is_some()) {
            report.error(
                label,
                site,
                match why_not {
                    Some(why) => format!("this map cannot be placed: {why}"),
                    None => "no map carries this tag".to_owned(),
                },
            );
            continue;
        }
        place.push(TractPlace {
            options,
            at,
            chance,
            params: given,
            order: entry
                .order
                .unwrap_or(u16::try_from(index).unwrap_or(u16::MAX)),
        });
    }

    let ruin = def.ruin.as_ref().and_then(|ruin| {
        if ruin.value.is_object() {
            Some(ruin.value.clone())
        } else {
            report.error(
                label,
                &ruin.site,
                "`ruin` is an object: `{ \"transform\": id }`, `{ \"tract\": id }`, or one of \
                 those for each kind of devastation",
            );
            None
        }
    });
    let mut tags: Vec<String> = def
        .tags
        .iter()
        .map(|tag| defs.tags().name(tag).to_owned())
        .collect();
    tags.sort();
    TractKind {
        name: name.to_owned(),
        tags,
        ground: ground_id,
        params,
        place,
        ruin,
        bands: def.bands.clone(),
        glyph: def.glyph.clone(),
    }
}
