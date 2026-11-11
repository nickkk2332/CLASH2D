//! Ground (10 §5): the `ground` definition type, and the steps that fill
//! a tract with strata, a surface, liquid, veins, pockets, caves and
//! scattered things.
//!
//! ```json
//! { "type": "ground", "id": "lowland",
//!   "params": { "trees": 0.02 },
//!   "steps": [
//!     { "strata": [ { "depth": 1, "block": "solid@soil" }, { "block": "solid@granite" } ] },
//!     { "surface": { "floor": "grass", "ramp": "ramp@soil" } },
//!     { "liquid": { "to": "sea_level", "block": "water" } },
//!     { "caves": { "below": -3, "density": 0.05, "width": [1, 3] } },
//!     { "scatter": { "fixture": "tree", "density": "$trees", "apart": 2 } } ] }
//! ```
//!
//! A ground is a list of steps, run in order for each tract. The steps
//! are native and configured by data; a mod cannot add one (10 §13).
//! Nothing here names a material or a plant: every state comes from the
//! definition (01 R2).
//!
//! # Pure by position
//!
//! Every choice is drawn from a positional hash of the realm's seed, a
//! domain named for the step, and a position: a cell for scatter, a
//! cell of a coarse lattice for a vein, a pocket or a cave. A tract
//! works out every feature that could reach it and keeps the cells that
//! fall inside it, so two tracts agree on a cave that crosses their
//! edge without either reading the other (10 §14, X1, X6).
//!
//! # What holds
//!
//! Generation leaves nothing unsupported (10 §5.2, X9):
//!
//! - every state of a stratum, a vein or a pocket must be
//!   `load_bearing`, so ground stands on ground;
//! - a cave is carved only where the stratum laid there anchors, in a
//!   block that anchors, and only under a block that anchors or under
//!   nothing at all, so no loose ground, no surface floor and no liquid
//!   is left over a void;
//! - a vein or a pocket of a state that does not anchor is laid one
//!   storey thick, between ground that anchors: never over a void, and
//!   never on or under more loose ground;
//! - a run of strata that do not anchor must bear its own weight in
//!   the realm's gravity ([`Ground::crushed_by`]): the generator of a
//!   realm where it would not refuses the ground.

use clash_core::{HashDomain, IVec3, Real, Rng, Rot4, Turn, define_id, pos_hash, pos_rng};
use clash_data::expr::{ExprDefs, Number};
use clash_data::{
    CompileCx, Cond, Def, DefLabel, Defs, Diagnostic, Expr, Runner, Scope, Site, TagSet, code,
};
use clash_world::dims::TRACT_XY;
use clash_world::{
    BlockState, CellPos, EngineFlag, FixtureState, FloorState, StateRefError, TractPos, WorldDefs,
};
use serde::Deserialize;

use crate::map::{GenCompiler, GenHost, Lit, MapDefs, MapId, MapPickDef, Ordered, Sited};
use crate::place::{self, Canvas, PlacedMap, TractCanvas};

define_id! {
    /// A `ground` definition.
    pub struct GroundId(u16);
}

/// How many storeys below the surface a ground is worked unless it
/// says: veins, pockets and caves are made no deeper.
pub const DEFAULT_GROUND_DEPTH: i64 = 16;

/// The deepest a ground may be worked, in storeys below the surface.
pub const MAX_GROUND_DEPTH: i64 = 128;

/// The thickest a stratum may be, in storeys.
pub const MAX_STRATUM_DEPTH: i64 = 256;

/// The widest a pocket or a cave, and the longest a vein, in cells.
pub const MAX_FEATURE_SIZE: i64 = 16;

/// The most cells that must lie between two scattered things.
pub const MAX_SCATTER_APART: u32 = 8;

/// The widest and deepest a map scattered by a ground may be, in cells.
/// A tract works out the scatter of this many cells round it, so that a
/// tree whose trunk stands in the next tract still shades this one.
pub const MAX_SCATTER_MAP_SIDE: i32 = 12;

/// The side of the lattice caves start on, in cells.
const CAVE_LATTICE_CELLS: i32 = 16;
/// The height of the lattice caves start on, in storeys.
const CAVE_LATTICE_STOREYS: i32 = 8;
/// How many cells long a cave is.
const CAVE_STEPS: i32 = 36;
/// How many cells a cave runs before it turns.
const CAVE_SEGMENT: i32 = 6;
/// The finest a heading is told apart: a heading is a whole number of
/// these along each axis.
const HEADING_UNITS: i64 = 8;
/// The most a cave's heading changes at a turn, in heading units.
const CAVE_TURN: i64 = 4;
/// The steepest a cave or a vein runs, in heading units of rise.
const HEADING_RISE: i64 = 2;
/// The smallest side of the lattice veins and pockets start on.
const MIN_LATTICE: i32 = 8;
/// The most features one lattice cell starts.
const MAX_FEATURES_PER_CELL: u32 = 8;
/// How many salts a step keeps for the features of one lattice cell.
const SALTS_PER_STEP: u64 = 16;

/// The domain of the features of veins.
const VEIN_DOMAIN: HashDomain = HashDomain::new("gen.ground.vein");
/// The domain of the features of pockets.
const POCKET_DOMAIN: HashDomain = HashDomain::new("gen.ground.pocket");
/// The domain of the features of caves.
const CAVE_DOMAIN: HashDomain = HashDomain::new("gen.ground.cave");
/// The domain of which cells a scatter step picks.
const SCATTER_DOMAIN: HashDomain = HashDomain::new("gen.ground.scatter");
/// The domain of which map a scattered cell gets, turned how, and its
/// seed.
const SCATTER_MAP_DOMAIN: HashDomain = HashDomain::new("gen.ground.scatter_map");
/// The domain of the generator the random functions of a step's
/// expressions draw from.
const EXPR_DOMAIN: HashDomain = HashDomain::new("gen.ground.expr");

/// `where` on a step: only where the surface is at or above the sea.
pub const WHERE_LAND: &str = "land";
/// `where` on a step: only where the surface is below the sea.
pub const WHERE_SEA: &str = "sea";
/// `to` on a `liquid` step: the realm's sea level.
pub const TO_SEA_LEVEL: &str = "sea_level";
/// `to` on a `liquid` step: the level of the planned lake over the
/// cell (10 §9.2). No lake is planned until realm plans are built.
pub const TO_PLAN: &str = "plan";

/// One layer of a `strata` step.
#[derive(Clone, Debug, Deserialize)]
pub struct StratumDef {
    /// How many storeys thick it is. The last stratum may leave it out:
    /// it then goes all the way down.
    #[serde(default)]
    pub depth: Option<Sited<i64>>,
    /// The block state it is made of.
    pub block: Sited<String>,
    /// Whether it anchors (02 §5.4). Stated, it selects the state of
    /// the block that has the flag, or lacks it.
    #[serde(default)]
    pub anchor: Option<Sited<bool>>,
}

/// A `surface` step.
#[derive(Clone, Debug, Deserialize)]
pub struct SurfaceDef {
    /// The floor laid on the top.
    #[serde(default)]
    pub floor: Option<Sited<String>>,
    /// A low block laid on the top.
    #[serde(default)]
    pub block: Option<Sited<String>>,
    /// The block put where the surface steps up one storey to a
    /// neighbouring cell. It must be flagged `ramp`.
    #[serde(default)]
    pub ramp: Option<Sited<String>>,
    /// `land` or `sea`; everywhere if left out.
    #[serde(default, rename = "where")]
    pub wher: Option<Sited<String>>,
}

/// A `liquid` step.
#[derive(Clone, Debug, Deserialize)]
pub struct LiquidDef {
    /// Up to where: `sea_level`, `plan`, or a storey.
    pub to: Sited<serde_json::Value>,
    /// The still liquid. It must be flagged `liquid`.
    pub block: Sited<String>,
}

/// A `veins` or `pockets` step.
#[derive(Clone, Debug, Deserialize)]
pub struct SeamDef {
    /// The state that replaces.
    pub block: Sited<String>,
    /// The only state that is replaced. Any stratum if left out.
    #[serde(default, rename = "in")]
    pub within: Option<Sited<String>>,
    /// The highest storey it reaches, counted from the surface: −1 is
    /// the top storey of the ground.
    #[serde(default)]
    pub below: Option<Sited<i64>>,
    /// The lowest storey it reaches, counted from the surface.
    #[serde(default)]
    pub above: Option<Sited<i64>>,
    /// About what share of the ground in reach it replaces.
    pub density: Expr<Number>,
    /// `[low, high]`: the length of a vein, or the width of a pocket,
    /// in cells.
    #[serde(default)]
    pub size: Option<Sited<Vec<i64>>>,
    /// The step is run only if this holds.
    #[serde(default, rename = "if")]
    pub condition: Option<Cond>,
}

/// A `caves` step.
#[derive(Clone, Debug, Deserialize)]
pub struct CavesDef {
    /// The highest storey a cave reaches, counted from the surface.
    #[serde(default)]
    pub below: Option<Sited<i64>>,
    /// The lowest storey a cave reaches, counted from the surface.
    #[serde(default)]
    pub above: Option<Sited<i64>>,
    /// About what share of the ground in reach is carved.
    pub density: Expr<Number>,
    /// `[low, high]`: how wide a cave is, in cells.
    #[serde(default)]
    pub width: Option<Sited<Vec<i64>>>,
    /// The step is run only if this holds.
    #[serde(default, rename = "if")]
    pub condition: Option<Cond>,
}

/// A `scatter` step.
#[derive(Clone, Debug, Deserialize)]
pub struct ScatterDef {
    /// A map to place, by id or by tag.
    #[serde(default)]
    pub map: Option<MapPickDef>,
    /// A floor to lay.
    #[serde(default)]
    pub floor: Option<Sited<String>>,
    /// A block to put.
    #[serde(default)]
    pub block: Option<Sited<String>>,
    /// A fixture to put.
    #[serde(default)]
    pub fixture: Option<Sited<String>>,
    /// The share of surface cells that get one.
    pub density: Expr<Number>,
    /// How many cells must lie between two of them.
    #[serde(default)]
    pub apart: Option<Sited<u32>>,
    /// `land` or `sea`; everywhere if left out.
    #[serde(default, rename = "where")]
    pub wher: Option<Sited<String>>,
    /// The step is run only if this holds.
    #[serde(default, rename = "if")]
    pub condition: Option<Cond>,
}

/// A `paths` step. It takes no settings.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PathsDef {}

/// One step of a ground: an object with one key, the step's name.
#[derive(Clone, Debug, Deserialize)]
pub struct GroundStepDef {
    /// Layers of block states, from the surface down.
    #[serde(default)]
    pub strata: Option<Vec<StratumDef>>,
    /// A floor, a low block and ramps on the top.
    #[serde(default)]
    pub surface: Option<SurfaceDef>,
    /// Still liquid in open cells up to a level.
    #[serde(default)]
    pub liquid: Option<LiquidDef>,
    /// Thin runs of another state through the ground.
    #[serde(default)]
    pub veins: Option<SeamDef>,
    /// Lumps of another state in the ground.
    #[serde(default)]
    pub pockets: Option<SeamDef>,
    /// Voids carved in ground that anchors.
    #[serde(default)]
    pub caves: Option<CavesDef>,
    /// Maps or elements put on the surface by density.
    #[serde(default)]
    pub scatter: Option<ScatterDef>,
    /// The paths a survey recorded as crossing the tract.
    #[serde(default)]
    pub paths: Option<PathsDef>,
    /// Islands of ground raised into the sky. Not built.
    #[serde(default)]
    pub float: Option<Sited<serde_json::Value>>,
}

/// What the ground of a tract is, and how it is filled (10 §5.1).
#[derive(Debug, Deserialize)]
pub struct GroundDef {
    /// Free-form tags.
    #[serde(default)]
    pub tags: TagSet,
    /// Parameters with their defaults, which a tract definition may
    /// set. The expressions of the steps see them as `$name`.
    #[serde(default)]
    pub params: Ordered<Sited<Lit>>,
    /// How many storeys below the surface the ground is worked.
    #[serde(default)]
    pub depth: Option<Sited<i64>>,
    /// The steps, run in order for each tract.
    pub steps: Vec<Sited<GroundStepDef>>,
}

impl Def for GroundDef {
    type Id = GroundId;
    const TYPE: &'static str = "ground";
    const DOC: &'static str = "What the ground of a tract is made of: strata, a surface, liquid, \
                               veins, pockets, caves and scattered things, as steps run in order \
                               for each tract (10 §5).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        // The generation context: the ground's own parameters as
        // `$name`, and nothing of the world.
        let mut scope = Scope::new();
        for (name, value) in &self.params.0 {
            scope = scope.local(name.clone(), value.value.0.type_name());
        }
        let mut gen_cx = GenCompiler { cx, scope };
        for step in &mut self.steps {
            let step = &mut step.value;
            for seam in [&mut step.veins, &mut step.pockets].into_iter().flatten() {
                gen_cx.expr(&mut seam.density);
                if let Some(condition) = &mut seam.condition {
                    gen_cx.cond(condition);
                }
            }
            if let Some(caves) = &mut step.caves {
                gen_cx.expr(&mut caves.density);
                if let Some(condition) = &mut caves.condition {
                    gen_cx.cond(condition);
                }
            }
            if let Some(scatter) = &mut step.scatter {
                gen_cx.expr(&mut scatter.density);
                if let Some(condition) = &mut scatter.condition {
                    gen_cx.cond(condition);
                }
            }
        }
    }
}

/// One layer of ground, compiled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stratum {
    /// How many storeys thick it is, or `None` for all the way down.
    pub depth: Option<i32>,
    /// What it is made of.
    pub block: BlockState,
    /// Whether it anchors.
    pub anchor: bool,
}

/// Where on the surface a step applies.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Where {
    #[default]
    Anywhere,
    Land,
    Sea,
}

impl Where {
    fn holds(self, height: i32, sea_level: Option<i32>) -> bool {
        match self {
            Where::Anywhere => true,
            Where::Land => sea_level.is_none_or(|sea| height >= sea),
            Where::Sea => sea_level.is_some_and(|sea| height < sea),
        }
    }
}

/// Up to where a `liquid` step fills.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LiquidTo {
    SeaLevel,
    Plan,
    Storey(i32),
}

/// What a `scatter` step puts on a cell.
#[derive(Clone, Debug)]
pub(crate) enum ScatterWhat {
    Element {
        floor: Option<FloorState>,
        block: Option<BlockState>,
        fixture: Option<FixtureState>,
    },
    Maps(Vec<MapId>),
}

/// A step of a ground, compiled.
#[derive(Clone, Debug)]
pub(crate) enum Step {
    Strata,
    Surface {
        floor: Option<FloorState>,
        block: Option<BlockState>,
        ramp: Option<BlockState>,
        wher: Where,
    },
    Liquid {
        to: LiquidTo,
        block: BlockState,
    },
    Seam {
        pockets: bool,
        block: BlockState,
        within: Option<BlockState>,
        below: i32,
        above: Option<i32>,
        density: Expr<Number>,
        size: (i32, i32),
        /// The cells one feature covers, on average.
        mean_cells: Real,
        condition: Option<Cond>,
    },
    Caves {
        below: i32,
        above: Option<i32>,
        density: Expr<Number>,
        width: (i32, i32),
        mean_cells: Real,
        condition: Option<Cond>,
    },
    Scatter {
        what: ScatterWhat,
        density: Expr<Number>,
        apart: i32,
        wher: Where,
        condition: Option<Cond>,
    },
    Paths,
}

/// A ground made ready to fill tracts with.
#[derive(Clone, Debug)]
pub struct Ground {
    pub(crate) name: String,
    /// The parameters: name and default, in the order declared.
    pub(crate) params: Vec<(String, clash_core::Value)>,
    pub(crate) depth: i32,
    pub(crate) strata: Vec<Stratum>,
    pub(crate) steps: Vec<Step>,
    /// How far beyond a tract a scattered map may stand and still reach
    /// into it.
    pub(crate) scatter_reach: i32,
}

impl Ground {
    /// The ground's qualified id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The strata, from the surface down.
    pub fn strata(&self) -> &[Stratum] {
        &self.strata
    }

    /// How many storeys below the surface the ground is worked.
    pub fn depth(&self) -> i32 {
        self.depth
    }

    /// The parameters with their defaults, in the order declared.
    pub fn params(&self) -> &[(String, clash_core::Value)] {
        &self.params
    }

    /// The stratum a cell `depth` storeys under the surface lies in,
    /// the top storey of the ground being depth 0. `None` below every
    /// stratum listed, where the realm's own fill begins.
    pub fn stratum_at(&self, depth: i32) -> Option<&Stratum> {
        let mut top = 0;
        for stratum in &self.strata {
            match stratum.depth {
                Some(thick) if depth < top + thick => return Some(stratum),
                Some(thick) => top += thick,
                None => return Some(stratum),
            }
        }
        None
    }

    /// What lies below every stratum that states a depth, and how many
    /// storeys those strata take together.
    pub(crate) fn bottom(&self, realm_fill: BlockState) -> (BlockState, i32) {
        let total = self.strata.iter().filter_map(|stratum| stratum.depth).sum();
        let bottom = match self.strata.last() {
            Some(last) if last.depth.is_none() => last.block,
            _ => realm_fill,
        };
        (bottom, total)
    }

    /// Why the ground would not stand under a gravity, if it would not:
    /// a run of strata that do not anchor carries its own weight on
    /// its lowest storey, and is crushed there if that is more than
    /// its material bears (02 §14.5). `None` if every such run holds.
    ///
    /// Strata that anchor are never crushed and pass nothing down, so
    /// each run of loose strata between them is weighed alone.
    pub fn crushed_by(&self, world: &WorldDefs, gravity: Real) -> Option<String> {
        let mut carried = Real::ZERO;
        for stratum in &self.strata {
            let Some(thick) = stratum.depth.filter(|_| !stratum.anchor) else {
                carried = Real::ZERO;
                continue;
            };
            let state = stratum.block.raw();
            carried += world.mass(clash_world::Layer::Block, state) * Real::from_i64(thick.into());
            let bearing = world
                .materials()
                .get(stratum.block.material())
                .and_then(|material| material.bearing);
            let Some(bearing) = bearing else { continue };
            // One cell is a square metre across.
            let holds = bearing * world.block_info(stratum.block).fill;
            let pressed = carried * gravity;
            if pressed > holds {
                return Some(format!(
                    "the ground `{}` would be crushed by its own weight here: a run of strata \
                     that do not anchor presses {pressed} N on a storey of `{}` that bears \
                     {holds} N under {gravity} m/s² (02 §14.5). Make the run thinner, or of \
                     something that anchors",
                    self.name,
                    world.state_text(clash_world::Layer::Block, state),
                ));
            }
        }
        None
    }

    /// How many cells round a tract its surface heights are needed for.
    pub(crate) fn margin(&self) -> i32 {
        self.scatter_reach.max(1)
    }
}

/// What a ground is run with for one tract.
pub(crate) struct GroundRun<'a> {
    pub(crate) world: &'a WorldDefs,
    pub(crate) maps: &'a MapDefs,
    pub(crate) exprs: &'a ExprDefs,
    pub(crate) seed: u64,
    pub(crate) tract: TractPos,
    /// The values of the ground's parameters, in the order declared.
    pub(crate) params: Vec<clash_expr::Value>,
    pub(crate) sea_level: Option<i32>,
    /// What the realm holds below everything a ground lays.
    pub(crate) realm_fill: BlockState,
    /// Whether a column lies under a map that levels or founds its
    /// ground: nothing is scattered there.
    pub(crate) built: &'a dyn Fn(i32, i32) -> bool,
}

/// A number below one from a hash.
fn unit(hash: u64) -> Real {
    const SPARE_BITS: u32 = 11;
    const STEP: f64 = 1.0 / 9_007_199_254_740_992.0;
    Real::of((hash >> SPARE_BITS) as f64 * STEP)
}

/// How many cells a ball `width` cells across covers.
fn ball_cells(width: i32) -> i64 {
    let reach = width / 2;
    let mut count = 0;
    for dz in -reach..=reach {
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if in_ball(dx, dy, dz, width) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Whether an offset lies within a ball `width` cells across.
fn in_ball(dx: i32, dy: i32, dz: i32, width: i32) -> bool {
    let (dx, dy, dz, width) = (
        i64::from(dx),
        i64::from(dy),
        i64::from(dz),
        i64::from(width),
    );
    4 * (dx * dx + dy * dy + dz * dz) <= width * width
}

/// How many cells of a row a ball `width` cells across crosses.
fn disc_cells(width: i32) -> i64 {
    let reach = width / 2;
    let mut count = 0;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            if in_ball(dx, dy, 0, width) {
                count += 1;
            }
        }
    }
    count
}

/// A heading: so many heading units along each axis for each step.
#[derive(Clone, Copy)]
struct Heading {
    x: i64,
    y: i64,
    z: i64,
}

impl Heading {
    fn draw(rng: &mut Rng) -> Heading {
        let mut heading = Heading {
            x: rng.range(-HEADING_UNITS, HEADING_UNITS),
            y: rng.range(-HEADING_UNITS, HEADING_UNITS),
            z: rng.range(-HEADING_RISE, HEADING_RISE),
        };
        heading.settle();
        heading
    }

    /// Makes the heading one that goes somewhere: a full step along at
    /// least one of the two level axes.
    fn settle(&mut self) {
        if self.x.abs().max(self.y.abs()) < HEADING_UNITS {
            if self.x.abs() >= self.y.abs() && self.x != 0 {
                self.x = HEADING_UNITS * self.x.signum();
            } else if self.y != 0 {
                self.y = HEADING_UNITS * self.y.signum();
            } else {
                self.x = HEADING_UNITS;
            }
        }
    }

    fn turn(&mut self, rng: &mut Rng) {
        self.x = (self.x + rng.range(-CAVE_TURN, CAVE_TURN)).clamp(-HEADING_UNITS, HEADING_UNITS);
        self.y = (self.y + rng.range(-CAVE_TURN, CAVE_TURN)).clamp(-HEADING_UNITS, HEADING_UNITS);
        self.z = rng.range(-HEADING_RISE, HEADING_RISE);
        self.settle();
    }
}

/// A point that moves along headings, in heading units.
#[derive(Clone, Copy)]
struct Walker {
    x: i64,
    y: i64,
    z: i64,
}

impl Walker {
    fn at(cell: IVec3) -> Walker {
        Walker {
            x: i64::from(cell.x) * HEADING_UNITS,
            y: i64::from(cell.y) * HEADING_UNITS,
            z: i64::from(cell.z) * HEADING_UNITS,
        }
    }

    fn step(&mut self, heading: Heading) {
        self.x += heading.x;
        self.y += heading.y;
        self.z += heading.z;
    }

    fn cell(self) -> IVec3 {
        let part = |value: i64| {
            i32::try_from((value + HEADING_UNITS / 2).div_euclid(HEADING_UNITS)).unwrap_or(0)
        };
        IVec3::new(part(self.x), part(self.y), part(self.z))
    }
}

/// The lattice the features of a step start on, and how far one
/// reaches from where it starts.
#[derive(Clone, Copy)]
struct Lattice {
    side: i32,
    storeys: i32,
    reach: i32,
    /// How far below the surface the ground is worked.
    depth: i32,
}

impl Ground {
    /// Fills a tract. Maps scattered by the ground are written here and
    /// returned, for what else they ask for.
    pub(crate) fn run(&self, run: &GroundRun<'_>, canvas: &mut TractCanvas<'_>) -> Vec<PlacedMap> {
        let mut scattered = Vec::new();
        let mut runner = Runner::new();
        let mut host = GenHost {
            params: run.params.clone(),
            rng: Rng::from_seed(0),
            warnings: 0,
        };
        let mut laid = false;
        for (index, step) in self.steps.iter().enumerate() {
            let salt = index as u64;
            host.rng = pos_rng(run.seed, EXPR_DOMAIN, run.tract.x, run.tract.y, 0, salt);
            let mut eval = runner.over(run.exprs, &mut host);
            let holds = |condition: &Option<Cond>,
                         eval: &mut clash_data::Evaluator<'_, GenHost>| {
                condition.as_ref().is_none_or(|cond| cond.holds(eval))
            };
            let number = |expr: &Expr<Number>, eval: &mut clash_data::Evaluator<'_, GenHost>| {
                expr.eval(eval)
                    .unwrap_or(Real::ZERO)
                    .clamp(Real::ZERO, Real::ONE)
            };
            match step {
                Step::Strata => {
                    self.lay_strata(run, canvas);
                    laid = true;
                }
                Step::Surface {
                    floor,
                    block,
                    ramp,
                    wher,
                } => {
                    if !laid {
                        self.lay_strata(run, canvas);
                        laid = true;
                    }
                    lay_surface(run, canvas, *floor, *block, *ramp, *wher);
                }
                Step::Liquid { to, block } => {
                    let level = match to {
                        LiquidTo::SeaLevel => run.sea_level,
                        LiquidTo::Storey(z) => Some(*z),
                        // No lake is planned until realm plans are
                        // built (WG6).
                        LiquidTo::Plan => None,
                    };
                    if let Some(level) = level {
                        self.fill_liquid(canvas, level, *block);
                    }
                }
                Step::Seam {
                    pockets,
                    block,
                    within,
                    below,
                    above,
                    density,
                    size,
                    mean_cells,
                    condition,
                } => {
                    if !holds(condition, &mut eval) {
                        continue;
                    }
                    let share = number(density, &mut eval);
                    let rule = SeamRule {
                        ground: self,
                        world: run.world,
                        realm_fill: run.realm_fill,
                        block: *block,
                        within: *within,
                        below: *below,
                        above: *above,
                    };
                    let side = if *pockets { 2 * size.1 } else { size.1 }.max(MIN_LATTICE);
                    let lattice = Lattice {
                        side,
                        storeys: side,
                        reach: size.1,
                        depth: self.depth,
                    };
                    let domain = if *pockets { POCKET_DOMAIN } else { VEIN_DOMAIN };
                    features(run, canvas, lattice, domain, salt, share, *mean_cells, {
                        let size = *size;
                        let pockets = *pockets;
                        move |rng: &mut Rng, start: IVec3, canvas: &mut TractCanvas<'_>| {
                            let extent = i32::try_from(rng.range(size.0.into(), size.1.into()))
                                .unwrap_or(size.0);
                            if pockets {
                                ball(start, extent, &mut |cell| rule.replace(canvas, cell));
                            } else {
                                let heading = Heading::draw(rng);
                                let mut walker = Walker::at(start);
                                for _ in 0..extent {
                                    rule.replace(canvas, walker.cell());
                                    walker.step(heading);
                                }
                            }
                        }
                    });
                }
                Step::Caves {
                    below,
                    above,
                    density,
                    width,
                    mean_cells,
                    condition,
                } => {
                    if !holds(condition, &mut eval) {
                        continue;
                    }
                    let share = number(density, &mut eval);
                    let lattice = Lattice {
                        side: CAVE_LATTICE_CELLS,
                        storeys: CAVE_LATTICE_STOREYS,
                        reach: CAVE_STEPS + width.1,
                        depth: self.depth,
                    };
                    let rule = CaveRule {
                        ground: self,
                        world: run.world,
                        realm_fill: run.realm_fill,
                        below: *below,
                        above: *above,
                    };
                    features(
                        run,
                        canvas,
                        lattice,
                        CAVE_DOMAIN,
                        salt,
                        share,
                        *mean_cells,
                        {
                            let width = *width;
                            move |rng: &mut Rng, start: IVec3, canvas: &mut TractCanvas<'_>| {
                                let across =
                                    i32::try_from(rng.range(width.0.into(), width.1.into()))
                                        .unwrap_or(width.0);
                                let mut heading = Heading::draw(rng);
                                let mut walker = Walker::at(start);
                                for step in 0..CAVE_STEPS {
                                    if step > 0 && step % CAVE_SEGMENT == 0 {
                                        heading.turn(rng);
                                    }
                                    ball(walker.cell(), across, &mut |cell| {
                                        rule.carve(canvas, cell)
                                    });
                                    walker.step(heading);
                                }
                            }
                        },
                    );
                }
                Step::Scatter {
                    what,
                    density,
                    apart,
                    wher,
                    condition,
                } => {
                    if !holds(condition, &mut eval) {
                        continue;
                    }
                    let share = number(density, &mut eval);
                    scatter(
                        run,
                        canvas,
                        salt,
                        what,
                        share,
                        *apart,
                        *wher,
                        &mut scattered,
                    );
                }
                // Nothing records paths until regions are surveyed
                // (WG7): there are none to draw.
                Step::Paths => {}
            }
        }
        if !laid {
            self.lay_strata(run, canvas);
        }
        scattered
    }

    /// Lays the strata of every column, from the surface down.
    fn lay_strata(&self, run: &GroundRun<'_>, canvas: &mut TractCanvas<'_>) {
        let (bottom, total) = self.bottom(run.realm_fill);
        let lowest = canvas.heights.lowest(canvas.origin);
        for (x, y) in columns(canvas.origin) {
            let height = canvas.height(x, y);
            for z in (height - total).min(lowest)..height {
                let block = self
                    .stratum_at(height - 1 - z)
                    .map_or(bottom, |stratum| stratum.block);
                // Below the lowest surface of the tract the column
                // already holds what lies at the bottom.
                if z < lowest && block == bottom {
                    continue;
                }
                canvas.builder.set_block(CellPos::new(x, y, z), block);
            }
        }
    }

    /// Fills open cells below a level with a still liquid.
    fn fill_liquid(&self, canvas: &mut TractCanvas<'_>, level: i32, liquid: BlockState) {
        for (x, y) in columns(canvas.origin) {
            let height = canvas.height(x, y);
            for z in (height - self.depth)..level {
                let at = CellPos::new(x, y, z);
                if canvas.cell(at).block.is_none() {
                    canvas.builder.set_block(at, liquid);
                }
            }
        }
    }
}

/// The cell columns of a tract, row by row.
fn columns(origin: CellPos) -> impl Iterator<Item = (i32, i32)> {
    (0..TRACT_XY).flat_map(move |dy| (0..TRACT_XY).map(move |dx| (origin.x + dx, origin.y + dy)))
}

/// Lays a floor, a low block and ramps on the top of every column.
fn lay_surface(
    run: &GroundRun<'_>,
    canvas: &mut TractCanvas<'_>,
    floor: Option<FloorState>,
    block: Option<BlockState>,
    ramp: Option<BlockState>,
    wher: Where,
) {
    for (x, y) in columns(canvas.origin) {
        let height = canvas.height(x, y);
        if !wher.holds(height, run.sea_level) {
            continue;
        }
        let at = CellPos::new(x, y, height);
        if let Some(floor) = floor {
            canvas.builder.set_floor(at, floor);
        }
        let steps_up = [(0, -1), (1, 0), (0, 1), (-1, 0)]
            .into_iter()
            .any(|(dx, dy)| canvas.height(x + dx, y + dy) == height + 1);
        match (ramp, block) {
            (Some(ramp), _) if steps_up => {
                canvas.builder.set_block(at, ramp);
            }
            (_, Some(block)) => {
                canvas.builder.set_block(at, block);
            }
            _ => {}
        }
    }
}

/// Calls `visit` for every cell of a ball round a cell.
fn ball(centre: IVec3, width: i32, visit: &mut dyn FnMut(IVec3)) {
    let reach = width / 2;
    for dz in -reach..=reach {
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if in_ball(dx, dy, dz, width) {
                    visit(IVec3::new(centre.x + dx, centre.y + dy, centre.z + dz));
                }
            }
        }
    }
}

/// Works out every feature of a step that could reach a tract: for
/// each cell of a coarse lattice near it, so many features by the
/// positional hash of that cell, each started somewhere inside it.
#[allow(
    clippy::too_many_arguments,
    reason = "one step's settings, passed straight through"
)]
fn features(
    run: &GroundRun<'_>,
    canvas: &mut TractCanvas<'_>,
    lattice: Lattice,
    domain: HashDomain,
    salt: u64,
    share: Real,
    mean_cells: Real,
    make: impl Fn(&mut Rng, IVec3, &mut TractCanvas<'_>),
) {
    if share <= Real::ZERO || mean_cells <= Real::ZERO {
        return;
    }
    let (side, storeys) = (lattice.side.max(1), lattice.storeys.max(1));
    let cells = Real::from_i64(i64::from(side) * i64::from(side) * i64::from(storeys));
    // How many features a lattice cell starts, so that together they
    // cover about the share asked for.
    let wanted = share * cells / mean_cells;
    let count = (wanted
        .ceil()
        .to_i64()
        .clamp(1, i64::from(MAX_FEATURES_PER_CELL))) as u32;
    let chance = (wanted / Real::from_i64(i64::from(count))).min(Real::ONE);
    let origin = canvas.origin;
    let (low, high) = canvas.heights.range(origin);
    let range = |from: i32, to: i32, size: i32| {
        (from - lattice.reach).div_euclid(size)..=(to + lattice.reach).div_euclid(size)
    };
    for k in range(low - lattice.depth, high, storeys) {
        for j in range(origin.y, origin.y + TRACT_XY - 1, side) {
            for i in range(origin.x, origin.x + TRACT_XY - 1, side) {
                for which in 0..count {
                    let mut rng = pos_rng(
                        run.seed,
                        domain,
                        i,
                        j,
                        k,
                        salt * SALTS_PER_STEP + u64::from(which),
                    );
                    if !rng.chance(chance) {
                        continue;
                    }
                    let within = |size: i32, rng: &mut Rng| {
                        i32::try_from(rng.below(size.max(1) as u64)).unwrap_or(0)
                    };
                    let start = IVec3::new(
                        i * side + within(side, &mut rng),
                        j * side + within(side, &mut rng),
                        k * storeys + within(storeys, &mut rng),
                    );
                    make(&mut rng, start, canvas);
                }
            }
        }
    }
}

/// What a vein or a pocket replaces, and with what.
#[derive(Clone, Copy)]
struct SeamRule<'a> {
    ground: &'a Ground,
    world: &'a WorldDefs,
    realm_fill: BlockState,
    block: BlockState,
    within: Option<BlockState>,
    below: i32,
    above: Option<i32>,
}

impl SeamRule<'_> {
    fn replace(&self, canvas: &mut TractCanvas<'_>, cell: IVec3) {
        if !canvas.covers(cell.x, cell.y) {
            return;
        }
        let height = canvas.height(cell.x, cell.y);
        let from_surface = cell.z - height;
        if from_surface > self.below
            || from_surface < -self.ground.depth
            || self.above.is_some_and(|above| from_surface < above)
        {
            return;
        }
        let at = CellPos::new(cell.x, cell.y, cell.z);
        let have = canvas.cell(at).block;
        if have.is_none() || have == self.block {
            return;
        }
        let laid = self
            .ground
            .stratum_at(-1 - from_surface)
            .map_or(self.realm_fill, |stratum| stratum.block);
        let replaceable = match self.within {
            Some(within) => have == within,
            None => have == laid,
        };
        if !replaceable {
            return;
        }
        if !anchors(self.world, self.block) {
            // Loose ground is laid one storey thick, between ground
            // that anchors: never over a void, and never on or under
            // more loose ground, where a run of it could grow past what
            // its lowest storey bears (02 §14.5).
            let under = canvas.cell(at.below()).block;
            let over = canvas.cell(at.above()).block;
            if !anchors(self.world, under) || !(over.is_none() || anchors(self.world, over)) {
                return;
            }
        }
        canvas.builder.set_block(at, self.block);
    }
}

/// Whether a block is ground that is always supported: a block flagged
/// `anchor` that is not a liquid.
fn anchors(world: &WorldDefs, block: BlockState) -> bool {
    let flags = world.block_info(block).flags;
    !block.is_none() && flags.has(EngineFlag::Anchor) && !flags.has(EngineFlag::Liquid)
}

/// Where a cave may be carved.
#[derive(Clone, Copy)]
struct CaveRule<'a> {
    ground: &'a Ground,
    world: &'a WorldDefs,
    realm_fill: BlockState,
    below: i32,
    above: Option<i32>,
}

impl CaveRule<'_> {
    /// Carves one cell of a cave, if the stratum there anchors and
    /// nothing that needs holding up lies over it (10 §5.2).
    fn carve(&self, canvas: &mut TractCanvas<'_>, cell: IVec3) {
        if !canvas.covers(cell.x, cell.y) {
            return;
        }
        let from_surface = cell.z - canvas.height(cell.x, cell.y);
        if from_surface > self.below
            || from_surface < -self.ground.depth
            || self.above.is_some_and(|above| from_surface < above)
        {
            return;
        }
        // The stratum as it was laid: a vein of rock through loose
        // ground is not a place for a cave.
        let stratum_anchors = self
            .ground
            .stratum_at(-1 - from_surface)
            .map_or_else(|| anchors(self.world, self.realm_fill), |s| s.anchor);
        if !stratum_anchors {
            return;
        }
        let at = CellPos::new(cell.x, cell.y, cell.z);
        let have = canvas.cell(at);
        if !anchors(self.world, have.block) || !have.floor.is_none() || !have.fixture.is_none() {
            return;
        }
        let over = canvas.cell(at.above());
        if !over.floor.is_none() || !over.fixture.is_none() {
            return;
        }
        if !over.block.is_none() && !anchors(self.world, over.block) {
            return;
        }
        canvas.builder.set_block(at, BlockState::NONE);
    }
}

/// The chance that a cell is a candidate, so that after thinning by
/// `apart` the share of cells kept is `density`: a candidate is kept
/// when no candidate within `apart` cells has a lower hash, which
/// leaves `(1 − (1 − c)^n) / n` of the cells, `n` being the cells of
/// the square a candidate is compared within.
fn candidate_chance(density: Real, apart: i32) -> Real {
    if apart <= 0 {
        return density;
    }
    let side = Real::from_i64(i64::from(2 * apart + 1));
    let cells = side * side;
    let covered = density * cells;
    if covered >= Real::ONE {
        return Real::ONE;
    }
    Real::ONE - (Real::ONE - covered).powf(Real::ONE / cells)
}

/// Puts things on the surface by density, with spacing (10 §5.1).
#[allow(
    clippy::too_many_arguments,
    reason = "one step's settings, passed straight through"
)]
fn scatter(
    run: &GroundRun<'_>,
    canvas: &mut TractCanvas<'_>,
    salt: u64,
    what: &ScatterWhat,
    density: Real,
    apart: i32,
    wher: Where,
    scattered: &mut Vec<PlacedMap>,
) {
    if density <= Real::ZERO {
        return;
    }
    let chance = candidate_chance(density, apart);
    let reach = match what {
        ScatterWhat::Element { .. } => 0,
        ScatterWhat::Maps(_) => MAX_SCATTER_MAP_SIDE - 1,
    };
    // The hashes of the tract's cells, of those a map could reach in
    // from, and of those either could be crowded out by.
    let margin = reach + apart;
    let side = TRACT_XY + 2 * margin;
    let (west, north) = (canvas.origin.x - margin, canvas.origin.y - margin);
    let mut hashes = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        for x in 0..side {
            hashes.push(pos_hash(
                run.seed,
                SCATTER_DOMAIN,
                west + x,
                north + y,
                0,
                salt,
            ));
        }
    }
    let hash = |x: i32, y: i32| hashes[((y - north) * side + (x - west)) as usize];
    let candidate = |x: i32, y: i32| unit(hash(x, y)) < chance;
    let kept = |x: i32, y: i32| {
        if !candidate(x, y) {
            return false;
        }
        let own = (hash(x, y), y, x);
        for dy in -apart..=apart {
            for dx in -apart..=apart {
                let (nx, ny) = (x + dx, y + dy);
                if (dx, dy) != (0, 0) && candidate(nx, ny) && (hash(nx, ny), ny, nx) < own {
                    return false;
                }
            }
        }
        true
    };
    let (from_x, from_y) = (canvas.origin.x - reach, canvas.origin.y - reach);
    let span = TRACT_XY + 2 * reach;
    for y in from_y..from_y + span {
        for x in from_x..from_x + span {
            if !kept(x, y) || (run.built)(x, y) {
                continue;
            }
            let Some(height) = canvas.heights.get(x, y) else {
                continue;
            };
            if !wher.holds(height, run.sea_level) {
                continue;
            }
            let at = CellPos::new(x, y, height);
            match what {
                ScatterWhat::Element {
                    floor,
                    block,
                    fixture,
                } => {
                    let have = canvas.cell(at);
                    let wet = run
                        .world
                        .block_info(have.block)
                        .flags
                        .has(EngineFlag::Liquid);
                    if !(have.block.is_none() || wet) || !have.fixture.is_none() {
                        continue;
                    }
                    if let Some(floor) = floor {
                        canvas.builder.set_floor(at, *floor);
                    }
                    if let Some(block) = block.filter(|_| have.block.is_none()) {
                        canvas.builder.set_block(at, block);
                    }
                    if let Some(fixture) = fixture {
                        canvas.builder.set_fixture(at, *fixture);
                    }
                }
                ScatterWhat::Maps(maps) => {
                    let mut rng = pos_rng(run.seed, SCATTER_MAP_DOMAIN, x, y, 0, salt);
                    let Some(&map) = rng.pick(maps) else { continue };
                    let Some(compiled) = run.maps.get(map) else {
                        continue;
                    };
                    let mut placement = place::placement(map, at);
                    if compiled.may_turn() {
                        let quarter = rng.below(Rot4::ALL.len() as u64) as usize;
                        placement.turn =
                            Turn::new(Rot4::ALL.get(quarter).copied().unwrap_or(Rot4::R0), false);
                    }
                    placement.seed = rng.next_u64();
                    let Ok(Some(mut placed)) = PlacedMap::resolve(run.maps, &placement) else {
                        continue;
                    };
                    // A scattered map does not shape the ground it
                    // stands on: the scatter is worked out from the
                    // ground as it is.
                    if placed.sit != crate::map::Sit::Follow {
                        placed.sit = crate::map::Sit::None;
                    }
                    place::draw(canvas, &placed, place::MapMode::Replace);
                    scattered.push(placed);
                }
            }
        }
    }
}

/// Collects what is wrong with grounds while they are built.
pub(crate) struct Report<'a> {
    pub(crate) defs: &'a Defs,
    pub(crate) world: &'a WorldDefs,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

impl Report<'_> {
    /// Reports a mistake at a value of a definition.
    pub(crate) fn error(&mut self, label: &DefLabel, site: &Site, message: impl Into<String>) {
        let mut diagnostic = Diagnostic::new(code::VALIDATION, message)
            .in_def(Some(label))
            .with_path(site.path.clone());
        if let Some(span) = site.span {
            diagnostic = diagnostic.at(self.defs.sources(), span);
        }
        self.diagnostics.push(diagnostic);
    }

    fn state_error(&mut self, label: &DefLabel, text: &Sited<String>, error: &StateRefError) {
        self.error(
            label,
            &text.site,
            format!("the state `{}` cannot be used: {error}", text.value),
        );
    }

    pub(crate) fn floor(
        &mut self,
        label: &DefLabel,
        namespace: &str,
        text: &Sited<String>,
    ) -> Option<FloorState> {
        match self.world.floor_state(&text.value, Some(namespace)) {
            Ok(state) => Some(state),
            Err(error) => {
                self.state_error(label, text, &error);
                None
            }
        }
    }

    pub(crate) fn block(
        &mut self,
        label: &DefLabel,
        namespace: &str,
        text: &Sited<String>,
    ) -> Option<BlockState> {
        match self.world.block_state(&text.value, Some(namespace)) {
            Ok(state) => Some(state),
            Err(error) => {
                self.state_error(label, text, &error);
                None
            }
        }
    }

    pub(crate) fn fixture(
        &mut self,
        label: &DefLabel,
        namespace: &str,
        text: &Sited<String>,
    ) -> Option<FixtureState> {
        match self.world.fixture_state(&text.value, Some(namespace)) {
            Ok(state) => Some(state),
            Err(error) => {
                self.state_error(label, text, &error);
                None
            }
        }
    }

    /// A block of the ground itself: it must bear what lies on it.
    fn ground_block(
        &mut self,
        label: &DefLabel,
        namespace: &str,
        text: &Sited<String>,
    ) -> Option<BlockState> {
        let state = self.block(label, namespace, text)?;
        let flags = self.world.block_info(state).flags;
        if state.is_none() || !flags.has(EngineFlag::LoadBearing) || flags.has(EngineFlag::Liquid) {
            self.error(
                label,
                &text.site,
                format!(
                    "the ground is made of blocks that bear what lies on them, and `{}` is not \
                     `load_bearing`: ground laid on it would fall (10 §5.2)",
                    text.value
                ),
            );
            return None;
        }
        Some(state)
    }

    fn wher(&mut self, label: &DefLabel, text: &Option<Sited<String>>) -> Where {
        match text.as_ref().map(|text| (text.value.as_str(), text)) {
            None => Where::Anywhere,
            Some((WHERE_LAND, _)) => Where::Land,
            Some((WHERE_SEA, _)) => Where::Sea,
            Some((other, text)) => {
                self.error(
                    label,
                    &text.site,
                    format!("`where` is `{WHERE_LAND}` or `{WHERE_SEA}`, not `{other}`"),
                );
                Where::Anywhere
            }
        }
    }

    /// A pair `[low, high]` of whole numbers from 1 to a limit.
    fn pair(
        &mut self,
        label: &DefLabel,
        given: &Option<Sited<Vec<i64>>>,
        default: (i32, i32),
        what: &str,
    ) -> (i32, i32) {
        let Some(given) = given else { return default };
        match given.value.as_slice() {
            [low, high] if 1 <= *low && low <= high && *high <= MAX_FEATURE_SIZE => {
                (*low as i32, *high as i32)
            }
            _ => {
                self.error(
                    label,
                    &given.site,
                    format!(
                        "`{what}` is `[low, high]`, two whole numbers from 1 to \
                         {MAX_FEATURE_SIZE}, the lower first"
                    ),
                );
                default
            }
        }
    }

    /// A storey counted from the surface, which must lie in the ground.
    fn storeys_down(
        &mut self,
        label: &DefLabel,
        given: &Option<Sited<i64>>,
        what: &str,
    ) -> Option<i32> {
        let given = given.as_ref()?;
        if (-MAX_GROUND_DEPTH..=-1).contains(&given.value) {
            return Some(given.value as i32);
        }
        self.error(
            label,
            &given.site,
            format!(
                "`{what}` counts storeys from the surface down, from -1 (the top storey of the \
                 ground) to -{MAX_GROUND_DEPTH}, and {} is not one",
                given.value
            ),
        );
        None
    }

    fn density(&mut self, label: &DefLabel, density: &Expr<Number>) {
        if let Some(value) = density.constant().and_then(clash_expr::Value::as_real)
            && !(Real::ZERO..=Real::ONE).contains(&value)
        {
            self.error(
                label,
                density.site(),
                format!("a density is a share, from 0 to 1, and {value} is not"),
            );
        }
    }
}

/// The default length of a vein and width of a pocket, in cells.
const DEFAULT_SEAM_SIZE: (i32, i32) = (3, 6);
/// The default width of a cave, in cells.
const DEFAULT_CAVE_WIDTH: (i32, i32) = (1, 3);

/// Builds one ground from its definition.
pub(crate) fn build_ground(
    report: &mut Report<'_>,
    maps: &MapDefs,
    name: &str,
    def: &GroundDef,
) -> Ground {
    let label = DefLabel::new(GroundDef::TYPE, name);
    let namespace = name.split_once(':').map_or("", |(namespace, _)| namespace);
    let world = report.world;
    let depth = match &def.depth {
        None => DEFAULT_GROUND_DEPTH,
        Some(depth) if (1..=MAX_GROUND_DEPTH).contains(&depth.value) => depth.value,
        Some(depth) => {
            report.error(
                &label,
                &depth.site,
                format!("`depth` is from 1 to {MAX_GROUND_DEPTH} storeys"),
            );
            DEFAULT_GROUND_DEPTH
        }
    } as i32;
    let mut ground = Ground {
        name: name.to_owned(),
        params: def
            .params
            .0
            .iter()
            .map(|(name, value)| (name.clone(), value.value.0.clone()))
            .collect(),
        depth,
        strata: Vec::new(),
        steps: Vec::new(),
        scatter_reach: 0,
    };
    for (index, step) in def.steps.iter().enumerate() {
        let body = &step.value;
        let given = [
            body.strata.is_some(),
            body.surface.is_some(),
            body.liquid.is_some(),
            body.veins.is_some(),
            body.pockets.is_some(),
            body.caves.is_some(),
            body.scatter.is_some(),
            body.paths.is_some(),
            body.float.is_some(),
        ];
        if given.iter().filter(|given| **given).count() != 1 {
            report.error(
                &label,
                &step.site,
                "a step is an object with one key, the step's name: `strata`, `surface`, \
                 `liquid`, `veins`, `pockets`, `caves`, `scatter` or `paths`",
            );
            continue;
        }
        if let Some(strata) = &body.strata {
            if index != 0 {
                report.error(
                    &label,
                    &step.site,
                    "the `strata` step is the first step of a ground, and a ground has one: \
                     every other step works on what it lays",
                );
                continue;
            }
            for (at, stratum) in strata.iter().enumerate() {
                let last = at + 1 == strata.len();
                let thick = match &stratum.depth {
                    Some(depth) if (1..=MAX_STRATUM_DEPTH).contains(&depth.value) => {
                        Some(depth.value as i32)
                    }
                    Some(depth) => {
                        report.error(
                            &label,
                            &depth.site,
                            format!("a stratum is from 1 to {MAX_STRATUM_DEPTH} storeys thick"),
                        );
                        Some(1)
                    }
                    None if last => None,
                    None => {
                        report.error(
                            &label,
                            &stratum.block.site,
                            "only the last stratum may leave out `depth`: it goes all the way \
                             down, and nothing could lie under it",
                        );
                        Some(1)
                    }
                };
                let Some(mut block) = report.ground_block(&label, namespace, &stratum.block) else {
                    continue;
                };
                let anchors =
                    |state: BlockState| world.block_info(state).flags.has(EngineFlag::Anchor);
                if let Some(want) = &stratum.anchor
                    && anchors(block) != want.value
                {
                    // The flag belongs to a state: take the state of
                    // the same block that has it, or lacks it.
                    let states = world
                        .element(clash_world::Layer::Block, block.raw())
                        .map_or(0, |element| element.states.len());
                    let other = (0..states)
                        .filter_map(|bits| u16::try_from(bits).ok())
                        .map(|bits| block.with_bits(bits))
                        .find(|state| {
                            let flags = world.block_info(*state).flags;
                            anchors(*state) == want.value && flags.has(EngineFlag::LoadBearing)
                        });
                    match other {
                        Some(state) => block = state,
                        None => {
                            let asked = if want.value {
                                "anchors"
                            } else {
                                "does not anchor"
                            };
                            report.error(
                                &label,
                                &want.site,
                                format!(
                                    "the stratum asks for a state of `{}` that {asked}, and the \
                                     block has none: give the block such a state (10 §5.2)",
                                    stratum.block.value
                                ),
                            );
                        }
                    }
                }
                ground.strata.push(Stratum {
                    depth: thick,
                    block,
                    anchor: anchors(block),
                });
            }
            ground.steps.push(Step::Strata);
        } else if let Some(surface) = &body.surface {
            let floor = surface
                .floor
                .as_ref()
                .and_then(|text| report.floor(&label, namespace, text));
            let block = surface
                .block
                .as_ref()
                .and_then(|text| report.block(&label, namespace, text));
            let ramp = surface.ramp.as_ref().and_then(|text| {
                let state = report.block(&label, namespace, text)?;
                if !world.block_info(state).flags.has(EngineFlag::Ramp) {
                    report.error(
                        &label,
                        &text.site,
                        format!(
                            "`ramp` is the block put where the surface steps up a storey, and \
                             `{}` is not flagged `ramp`: nobody could walk up it",
                            text.value
                        ),
                    );
                    return None;
                }
                Some(state)
            });
            let wher = report.wher(&label, &surface.wher);
            ground.steps.push(Step::Surface {
                floor,
                block,
                ramp,
                wher,
            });
        } else if let Some(liquid) = &body.liquid {
            let to = match &liquid.to.value {
                serde_json::Value::String(text) if text == TO_SEA_LEVEL => Some(LiquidTo::SeaLevel),
                serde_json::Value::String(text) if text == TO_PLAN => Some(LiquidTo::Plan),
                serde_json::Value::Number(number) => number
                    .as_i64()
                    .and_then(|storey| i32::try_from(storey).ok())
                    .map(LiquidTo::Storey),
                _ => None,
            };
            if to.is_none() {
                report.error(
                    &label,
                    &liquid.to.site,
                    format!("`to` is `{TO_SEA_LEVEL}`, `{TO_PLAN}` or a storey"),
                );
            }
            let block = report
                .block(&label, namespace, &liquid.block)
                .filter(|state| {
                    let wet = world.block_info(*state).flags.has(EngineFlag::Liquid);
                    if !wet {
                        report.error(
                            &label,
                            &liquid.block.site,
                            format!(
                                "a `liquid` step fills with a still liquid, and `{}` is not \
                                 flagged `liquid`: as structure it would fall (02 §14.1)",
                                liquid.block.value
                            ),
                        );
                    }
                    wet
                });
            if let (Some(to), Some(block)) = (to, block) {
                ground.steps.push(Step::Liquid { to, block });
            }
        } else if let Some((seam, pockets)) = body
            .veins
            .as_ref()
            .map(|seam| (seam, false))
            .or(body.pockets.as_ref().map(|seam| (seam, true)))
        {
            let block = report.ground_block(&label, namespace, &seam.block);
            let within = seam
                .within
                .as_ref()
                .and_then(|text| report.block(&label, namespace, text));
            let below = report
                .storeys_down(&label, &seam.below, "below")
                .unwrap_or(-1);
            let above = report.storeys_down(&label, &seam.above, "above");
            let size = report.pair(&label, &seam.size, DEFAULT_SEAM_SIZE, "size");
            report.density(&label, &seam.density);
            let count = i64::from(size.1 - size.0 + 1);
            let total: i64 = (size.0..=size.1)
                .map(|extent| {
                    if pockets {
                        ball_cells(extent)
                    } else {
                        i64::from(extent)
                    }
                })
                .sum();
            if let Some(block) = block {
                ground.steps.push(Step::Seam {
                    pockets,
                    block,
                    within,
                    below,
                    above,
                    density: seam.density.clone(),
                    size,
                    mean_cells: Real::from_i64(total) / Real::from_i64(count),
                    condition: seam.condition.clone(),
                });
            }
        } else if let Some(caves) = &body.caves {
            let below = report
                .storeys_down(&label, &caves.below, "below")
                .unwrap_or(-2);
            let above = report.storeys_down(&label, &caves.above, "above");
            let width = report.pair(&label, &caves.width, DEFAULT_CAVE_WIDTH, "width");
            report.density(&label, &caves.density);
            let count = i64::from(width.1 - width.0 + 1);
            let total: i64 = (width.0..=width.1).map(disc_cells).sum();
            ground.steps.push(Step::Caves {
                below,
                above,
                density: caves.density.clone(),
                width,
                mean_cells: Real::from_i64(total * i64::from(CAVE_STEPS)) / Real::from_i64(count),
                condition: caves.condition.clone(),
            });
        } else if let Some(scatter) = &body.scatter {
            let wher = report.wher(&label, &scatter.wher);
            report.density(&label, &scatter.density);
            let apart = match &scatter.apart {
                None => 0,
                Some(apart) if apart.value <= MAX_SCATTER_APART => apart.value as i32,
                Some(apart) => {
                    report.error(
                        &label,
                        &apart.site,
                        format!("`apart` is at most {MAX_SCATTER_APART} cells"),
                    );
                    0
                }
            };
            let element =
                scatter.floor.is_some() || scatter.block.is_some() || scatter.fixture.is_some();
            let what = match (&scatter.map, element) {
                (Some(_), true) | (None, false) => {
                    report.error(
                        &label,
                        &step.site.child("scatter"),
                        "a `scatter` step puts either a `map`, or elements: a `floor`, a \
                         `block`, a `fixture`, or several of them",
                    );
                    continue;
                }
                (Some(pick), false) => {
                    let found: Vec<MapId> = match pick {
                        MapPickDef::Id(map, _) => vec![map.id()],
                        MapPickDef::Tag(tag, _) => maps.with_tag(tag).collect(),
                    };
                    let fits = |map: &MapId| {
                        maps.get(*map).is_some_and(|map| {
                            map.size().x.max(map.size().y) <= MAX_SCATTER_MAP_SIDE
                        })
                    };
                    if found.is_empty() || !found.iter().all(fits) {
                        report.error(
                            &label,
                            pick.site(),
                            format!(
                                "a `scatter` step needs at least one map, and each at most \
                                 {MAX_SCATTER_MAP_SIDE} cells on a side: a tract works out the \
                                 scatter of that many cells round it"
                            ),
                        );
                        continue;
                    }
                    ground.scatter_reach = MAX_SCATTER_MAP_SIDE - 1;
                    ScatterWhat::Maps(found)
                }
                (None, true) => ScatterWhat::Element {
                    floor: scatter
                        .floor
                        .as_ref()
                        .and_then(|text| report.floor(&label, namespace, text)),
                    block: scatter
                        .block
                        .as_ref()
                        .and_then(|text| report.block(&label, namespace, text)),
                    fixture: scatter
                        .fixture
                        .as_ref()
                        .and_then(|text| report.fixture(&label, namespace, text)),
                },
            };
            ground.steps.push(Step::Scatter {
                what,
                density: scatter.density.clone(),
                apart,
                wher,
                condition: scatter.condition.clone(),
            });
        } else if body.paths.is_some() {
            ground.steps.push(Step::Paths);
        } else if let Some(float) = &body.float {
            report.error(
                &label,
                &float.site,
                "the `float` step is not built yet: islands of ground in the sky wait for a \
                 later package (10 §5.1)",
            );
        }
    }
    ground
}
