//! Compiled maps: [`MapDefs`], built from loaded definitions and the
//! world's tables (04 §8.1, the build stage; 10 §3.7).
//!
//! Building resolves every state a map names to a typed state, every
//! character to its palette entry and every nested map to its
//! definition, and checks what can be checked before any map is placed.
//! Each mistake is a diagnostic at the value in the file that wrote it.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dir8, IVec2, IVec3, Real, Turn, Vertical};
use clash_data::expr::{Any, ExprDefs, Int, Number};
use clash_data::{
    Cond, DefLabel, Defs, Diagnostic, Diagnostics, Expr, Located, Site, Span, code, diag,
};
use clash_world::{AreaKindId, BlockState, ElementState, Layer, StateRefError, WorldDefs};

use super::def::{
    AREA_PREFIX, AltDef, AreaDef, Base, CountDef, EntryDef, LevelDef, MARK_PREFIX, MapDef, MapId,
    MapPickDef, PARAM_TYPES, PaletteDef, PaletteItem, ParamRule, PointDef, SpecDef, TurnDef,
};
use super::facing::FacingTable;

/// The widest and deepest a map may be, in cells.
pub const MAX_MAP_SIDE: i64 = 512;

/// The most storeys a map may have.
pub const MAX_MAP_LEVELS: i64 = 256;

/// The most cells a map may have in all, over every storey: what
/// resolving one map may be asked to hold.
pub const MAX_MAP_CELLS: i64 = 1 << 20;

/// How deep maps may nest unless a limit says otherwise: the default of
/// the setting `gen.max_nesting` (10 §3.7).
pub const DEFAULT_MAX_NESTING: u32 = 8;

/// The most combinations of parameter values a state written with
/// parameters is tried with when maps are built. Past this, the rest
/// are checked when a map is resolved.
const MAX_TEMPLATE_COMBINATIONS: usize = 256;

/// The preset that roofs the storey below it (10 §3.1).
pub const PRESET_ROOF: &str = "roof";

/// The presets a level may name.
pub const PRESETS: [&str; 1] = [PRESET_ROOF];

/// The character that leaves a cell as it is.
pub const LEAVE: char = ' ';

/// The entry index of a cell no character draws.
pub(crate) const NO_ENTRY: u16 = u16::MAX;

/// Limits on maps that settings will supply (10 §13).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MapLimits {
    /// How deep maps may nest: the setting `gen.max_nesting`.
    pub max_nesting: u32,
}

impl Default for MapLimits {
    fn default() -> MapLimits {
        MapLimits {
            max_nesting: DEFAULT_MAX_NESTING,
        }
    }
}

/// The type of a parameter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamType {
    /// `true` or `false`.
    Bool,
    /// A whole number.
    Int,
    /// A number.
    Real,
    /// A text, which is also how an id is given.
    Str,
}

impl ParamType {
    /// The type by its name in data.
    pub fn from_name(name: &str) -> Option<ParamType> {
        match name {
            "bool" => Some(ParamType::Bool),
            "int" => Some(ParamType::Int),
            "real" => Some(ParamType::Real),
            "string" => Some(ParamType::Str),
            _ => None,
        }
    }

    /// The type's name in data.
    pub const fn name(self) -> &'static str {
        match self {
            ParamType::Bool => "bool",
            ParamType::Int => "int",
            ParamType::Real => "real",
            ParamType::Str => "string",
        }
    }

    /// The value as this type, if it is one: a whole number is accepted
    /// where a number is wanted.
    pub fn accept(self, value: &clash_core::Value) -> Option<clash_core::Value> {
        use clash_core::Value as V;
        match (self, value) {
            (ParamType::Bool, V::Bool(_))
            | (ParamType::Int, V::Int(_))
            | (ParamType::Real, V::Real(_))
            | (ParamType::Str, V::Str(_)) => Some(value.clone()),
            (ParamType::Real, V::Int(int)) => Some(V::Real(Real::of(*int as f64))),
            _ => None,
        }
    }
}

/// How a parameter gets its value when the placement gives none.
#[derive(Clone, Debug)]
pub(crate) enum CompiledRule {
    Pick(Vec<clash_core::Value>),
    RangeInt(i64, i64),
    RangeReal(Real, Real),
    Default(clash_core::Value),
    Expr(Expr<Any>),
    Required,
}

/// A parameter of a compiled map.
#[derive(Clone, Debug)]
pub struct CompiledParam {
    /// Its name, written `$name` in the map.
    pub name: String,
    /// Its type.
    pub ty: ParamType,
    pub(crate) rule: CompiledRule,
}

impl CompiledParam {
    /// Whether a placement must give this parameter.
    pub fn is_required(&self) -> bool {
        matches!(self.rule, CompiledRule::Required)
    }
}

/// A piece of a state written with parameters.
#[derive(Clone, Debug)]
pub(crate) enum TemplatePart {
    Text(String),
    Param(usize),
}

/// What a cell spec says about one element layer.
#[derive(Clone, Debug)]
pub(crate) enum StateSlot {
    /// Nothing: the layer takes what lies under the spec, and is empty
    /// if nothing does.
    Unset,
    /// The layer is left as the world has it.
    Keep,
    /// This state.
    Fixed(ElementState),
    /// A state written with parameters, resolved for each placement.
    Template(Vec<TemplatePart>),
}

/// Where items come from.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ItemSource {
    /// One item definition, by id as written.
    Item(String),
    /// A content table, by id as written.
    Table(String),
}

/// Who a spawn brings.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum SpawnWho {
    /// A creature definition, by id as written.
    Creature(String),
    /// A member of a population, by id as written.
    Population(String),
}

/// A device's name and settings, as a map gives them (10 §3.2, §3.6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapDevice {
    /// The name links refer to it by.
    pub name: Option<String>,
    /// Settings handed to the device, by key.
    pub config: Vec<(String, clash_core::Value)>,
}

/// What a cell of a pattern asks for in place of a state (10 §3.2;
/// 16 §4.5).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PatternMatch {
    /// Any floor carrying this tag.
    pub floor_tag: Option<String>,
    /// Any block carrying this tag.
    pub block_tag: Option<String>,
    /// Any fixture carrying this tag.
    pub fixture_tag: Option<String>,
    /// Anything at all.
    pub any: bool,
}

/// A layer of a cell, other than its three elements, that a cell spec
/// empties with `clear`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ClearedLayer {
    /// The conduits in the cell.
    Conduits,
    /// The fields in the cell.
    Fields,
    /// The items lying in the cell.
    Items,
    /// The decals on the cell.
    Decals,
    /// The cell's variables.
    Vars,
}

impl ClearedLayer {
    fn from_name(name: &str) -> Option<ClearedLayer> {
        match name {
            "conduits" => Some(ClearedLayer::Conduits),
            "fields" => Some(ClearedLayer::Fields),
            "items" => Some(ClearedLayer::Items),
            "decals" => Some(ClearedLayer::Decals),
            "vars" => Some(ClearedLayer::Vars),
            _ => None,
        }
    }
}

/// The names `clear` and `keep` take.
const LAYER_NAMES: [&str; 8] = [
    "floor", "block", "fixture", "conduits", "fields", "items", "decals", "vars",
];

/// How a map meets uneven ground (10 §4.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sit {
    /// The terrain under the footprint is flattened.
    #[default]
    Level,
    /// Terrain is left; gaps under the map are filled with this state.
    Foundation(Option<BlockState>),
    /// Each column is shifted to its own ground height.
    Follow,
    /// Nothing is done.
    None,
}

/// Which way a joint opens.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum JointDir {
    /// Sideways, through an edge of the map.
    Side(Dir8),
    /// Up or down, through its top or bottom storey.
    Vertical(Vertical),
}

#[derive(Clone, Debug)]
pub(crate) enum CompiledCount {
    Range(i64, i64),
    Expr(Expr<Int>),
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledItems {
    pub(crate) source: ItemSource,
    pub(crate) count: Option<CompiledCount>,
    pub(crate) chance: Option<Expr<Number>>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledSpawn {
    pub(crate) who: SpawnWho,
    pub(crate) count: Option<CompiledCount>,
    pub(crate) chance: Option<Expr<Number>>,
}

/// Where a map-level spawn goes.
#[derive(Clone, Debug)]
pub(crate) enum SpawnAt {
    Cell(IVec3),
    Area(String),
    Anywhere,
}

/// Everything a cell spec says besides its three elements.
#[derive(Clone, Debug, Default)]
pub(crate) struct Extras {
    pub(crate) conduits: Vec<String>,
    pub(crate) fields: Vec<(String, Real)>,
    pub(crate) items: Option<CompiledItems>,
    pub(crate) spawn: Option<CompiledSpawn>,
    pub(crate) device: Option<MapDevice>,
    pub(crate) marks: Vec<String>,
    pub(crate) berth: Option<(String, String)>,
    pub(crate) approach: Option<String>,
    pub(crate) watched: bool,
    pub(crate) area: Option<(String, Option<AreaKindId>)>,
    pub(crate) vars: Vec<(String, clash_core::Value)>,
    pub(crate) decals: Vec<String>,
    pub(crate) pattern: Option<PatternMatch>,
    pub(crate) cleared: Vec<ClearedLayer>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledSpec {
    pub(crate) floor: StateSlot,
    pub(crate) block: StateSlot,
    pub(crate) fixture: StateSlot,
    pub(crate) extras: Option<Box<Extras>>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledAlt {
    pub(crate) spec: CompiledSpec,
    pub(crate) weight: Option<Expr<Number>>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledEntry {
    pub(crate) alternatives: Vec<CompiledAlt>,
    pub(crate) once: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum CompiledLevel {
    /// No such storey was written: every cell is left as it is.
    Empty,
    /// The entry of each cell, row by row, [`NO_ENTRY`] for a space.
    Rows(Vec<u16>),
    /// The `roof` preset.
    Roof,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledSet {
    pub(crate) min: IVec3,
    pub(crate) max: IVec3,
    pub(crate) entry: u16,
}

#[derive(Clone, Debug)]
pub(crate) enum PlaceAt {
    Cell(IVec3),
    Area(String),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TurnRule {
    Fixed(Turn),
    Any,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledPlace {
    pub(crate) candidates: Vec<MapId>,
    pub(crate) at: PlaceAt,
    pub(crate) turn: TurnRule,
    pub(crate) chance: Option<Expr<Number>>,
    pub(crate) count: Option<CompiledCount>,
    pub(crate) apart: u32,
    pub(crate) params: Vec<(String, Expr<Any>)>,
    site: Site,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledArea {
    pub(crate) name: String,
    pub(crate) kind: Option<AreaKindId>,
    pub(crate) tags: Vec<String>,
    pub(crate) min: IVec3,
    pub(crate) max: IVec3,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledMapSpawn {
    pub(crate) spawn: CompiledSpawn,
    pub(crate) at: SpawnAt,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledJoint {
    pub(crate) id: Option<String>,
    pub(crate) at: IVec3,
    pub(crate) dir: JointDir,
    pub(crate) kind: String,
}

/// The far end of a portal a map declares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PortalTo {
    /// A mark of the same map.
    Mark(String),
    /// A named entry elsewhere, as written.
    Entry(String),
}

/// A map made ready to resolve: every state it names is a typed state,
/// every character an entry, every nested map a definition.
#[derive(Clone, Debug)]
pub struct CompiledMap {
    pub(crate) id: MapId,
    pub(crate) name: String,
    pub(crate) namespace: String,
    pub(crate) tags: Vec<String>,
    pub(crate) size: IVec2,
    pub(crate) lowest: i32,
    pub(crate) levels: Vec<CompiledLevel>,
    pub(crate) params: Vec<CompiledParam>,
    pub(crate) condition: Option<Cond>,
    pub(crate) entries: Vec<CompiledEntry>,
    pub(crate) default: Option<u16>,
    pub(crate) roof: Option<u16>,
    pub(crate) set: Vec<CompiledSet>,
    pub(crate) place: Vec<CompiledPlace>,
    pub(crate) areas: Vec<CompiledArea>,
    pub(crate) marks: BTreeMap<String, Vec<IVec3>>,
    pub(crate) spawns: Vec<CompiledMapSpawn>,
    pub(crate) wiring: Vec<(IVec3, String)>,
    pub(crate) links: Vec<(String, String, Option<String>)>,
    pub(crate) joints: Vec<CompiledJoint>,
    pub(crate) anchor: Option<IVec3>,
    pub(crate) front: Option<Dir8>,
    pub(crate) base: Base,
    pub(crate) sit: Sit,
    pub(crate) may_turn: bool,
    pub(crate) over: bool,
    pub(crate) portals: Vec<(String, PortalTo)>,
    pub(crate) vehicles: Vec<(String, IVec3, Option<Dir8>)>,
}

impl CompiledMap {
    /// The map's dense id.
    pub fn id(&self) -> MapId {
        self.id
    }

    /// The map's qualified id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The map's tags, sorted.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Width and depth in cells, as drawn: before any turn.
    pub fn size(&self) -> IVec2 {
        self.size
    }

    /// The lowest and the highest storey the map has, relative to its
    /// own ground level.
    pub fn level_range(&self) -> (i32, i32) {
        (self.lowest, self.lowest + self.levels.len() as i32 - 1)
    }

    /// The parameters, in the order they are declared.
    pub fn params(&self) -> &[CompiledParam] {
        &self.params
    }

    /// The cell a placement names when it says where the map goes, as
    /// drawn. `None` if the map names none: a placement then says
    /// where the north-west corner of the map, as turned, goes.
    pub fn anchor(&self) -> Option<IVec3> {
        self.anchor
    }

    /// Which side faces a street, as drawn.
    pub fn front(&self) -> Option<Dir8> {
        self.front
    }

    /// What level 0 sits on.
    pub fn base(&self) -> Base {
        self.base
    }

    /// How the map meets uneven ground.
    pub fn sit(&self) -> Sit {
        self.sit
    }

    /// Whether a placement may turn the map.
    pub fn may_turn(&self) -> bool {
        self.may_turn
    }

    /// Whether the map may overlap another placement.
    pub fn over(&self) -> bool {
        self.over
    }

    /// The cells of the marks that are the same in every placement: the
    /// map's `marks` and the marks of characters that have one cell
    /// spec. As drawn.
    pub fn marks(&self) -> &BTreeMap<String, Vec<IVec3>> {
        &self.marks
    }

    pub(crate) fn level(&self, z: i32) -> &CompiledLevel {
        usize::try_from(z - self.lowest)
            .ok()
            .and_then(|index| self.levels.get(index))
            .unwrap_or(&CompiledLevel::Empty)
    }

    pub(crate) fn cell_index(&self, x: i32, y: i32) -> usize {
        (y * self.size.x + x) as usize
    }

    /// The entry a character draws at a cell, following a roof down to
    /// the storey it covers: whether the cell is drawn at all does not
    /// depend on any choice.
    pub(crate) fn drawn(&self, x: i32, y: i32, z: i32) -> bool {
        let mut z = z;
        loop {
            match self.level(z) {
                CompiledLevel::Empty => return false,
                CompiledLevel::Rows(cells) => {
                    return cells
                        .get(self.cell_index(x, y))
                        .copied()
                        .unwrap_or(NO_ENTRY)
                        != NO_ENTRY;
                }
                CompiledLevel::Roof => z -= 1,
            }
        }
    }
}

/// Every map of a load, compiled, with the world's tables they were
/// resolved against (10 §3).
///
/// Built once by [`MapDefs::from_defs`] and shared behind an `Arc`. A
/// [`MapId`] is the loader's dense id: `defs.lookup::<MapDef>("core:hut")`
/// and [`MapDefs::lookup`] agree.
#[derive(Clone, Debug)]
pub struct MapDefs {
    pub(crate) world: Arc<WorldDefs>,
    pub(crate) exprs: ExprDefs,
    pub(crate) maps: Vec<CompiledMap>,
    by_name: BTreeMap<String, MapId>,
    pub(crate) facing: FacingTable,
    pub(crate) limits: MapLimits,
}

// `MapDefs` is shared between threads behind an `Arc`: tracts are
// realised in parallel.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<MapDefs>();
};

impl MapDefs {
    /// Builds the compiled maps from loaded definitions and the world's
    /// tables, with the default limits.
    ///
    /// Every mistake found is returned as a located diagnostic. This
    /// never panics, whatever the definitions hold.
    pub fn from_defs(defs: &Defs, world: &Arc<WorldDefs>) -> Result<Arc<MapDefs>, Diagnostics> {
        MapDefs::from_defs_with(defs, world, MapLimits::default())
    }

    /// Builds the compiled maps with limits given, as the settings will
    /// give them.
    pub fn from_defs_with(
        defs: &Defs,
        world: &Arc<WorldDefs>,
        limits: MapLimits,
    ) -> Result<Arc<MapDefs>, Diagnostics> {
        let mut builder = Builder {
            defs,
            world,
            diagnostics: Vec::new(),
        };
        let registry = defs.try_registry::<MapDef>();
        let mut maps = Vec::new();
        let mut by_name = BTreeMap::new();
        if let Some(registry) = registry {
            for (id, def) in registry.iter() {
                let name = registry.name(id).to_owned();
                by_name.insert(name.clone(), id);
                maps.push(builder.map(id, &name, def));
            }
        }
        if let Some(palettes) = defs.try_registry::<PaletteDef>() {
            for (id, palette) in palettes.iter() {
                let label = DefLabel::new("palette", palettes.name(id));
                for (key, entry) in &palette.entries {
                    builder.palette_key(&label, key, &entry.site);
                }
            }
        }
        builder.nesting(&mut maps, limits);
        if builder.diagnostics.iter().any(Diagnostic::is_error) {
            return Err(builder.finish());
        }
        Ok(Arc::new(MapDefs {
            world: Arc::clone(world),
            exprs: defs.exprs().clone(),
            maps,
            by_name,
            facing: FacingTable::build(world),
            limits,
        }))
    }

    /// The world's tables the maps were resolved against.
    pub fn world(&self) -> &Arc<WorldDefs> {
        &self.world
    }

    /// The limits the maps were built with.
    pub fn limits(&self) -> MapLimits {
        self.limits
    }

    /// The dense id of a map by its qualified id.
    pub fn lookup(&self, qualified: &str) -> Option<MapId> {
        self.by_name.get(qualified).copied()
    }

    /// The compiled map with an id.
    pub fn get(&self, id: MapId) -> Option<&CompiledMap> {
        self.maps.get(id.index())
    }

    /// The qualified id of a map, or `""` for an id that names none.
    pub fn name(&self, id: MapId) -> &str {
        self.get(id).map_or("", |map| map.name.as_str())
    }

    /// Every map with its id, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (MapId, &CompiledMap)> + '_ {
        self.maps.iter().map(|map| (map.id, map))
    }

    /// The maps that carry a tag, in id order.
    pub fn with_tag<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = MapId> + 'a {
        self.maps
            .iter()
            .filter(move |map| map.tags.iter().any(|have| have == tag))
            .map(|map| map.id)
    }

    /// How many maps there are.
    pub fn len(&self) -> usize {
        self.maps.len()
    }

    /// Whether there are no maps.
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }
}

/// The part of a qualified id before the colon.
fn namespace_of(qualified: &str) -> &str {
    qualified
        .split_once(':')
        .map_or("", |(namespace, _)| namespace)
}

/// The one character a palette key is, if it is one.
fn single_char(key: &str) -> Option<char> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

/// A `$name` found in a text: the bytes it covers, `$` included, and
/// the name; or the bytes of a `$` that no name follows.
type ParamUse<'a> = Result<(usize, usize, &'a str), (usize, usize)>;

/// The parameter names written in a text as `$name`, with the byte
/// range of each including its `$`.
fn template_parts(text: &str) -> Vec<ParamUse<'_>> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'$' {
            let start = at;
            at += 1;
            let from = at;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            if at == from {
                found.push(Err((start, at)));
            } else {
                found.push(Ok((start, at, &text[from..at])));
            }
        } else {
            at += 1;
        }
    }
    found
}

struct Builder<'a> {
    defs: &'a Defs,
    world: &'a Arc<WorldDefs>,
    diagnostics: Vec<Diagnostic>,
}

/// What building one map carries about.
struct MapCx<'a> {
    label: DefLabel,
    namespace: String,
    params: Vec<CompiledParam>,
    palette: BTreeMap<char, &'a EntryDef>,
    entries: Vec<CompiledEntry>,
    by_char: BTreeMap<char, u16>,
    devices: Vec<String>,
    area_names: Vec<String>,
}

impl<'a> Builder<'a> {
    fn finish(self) -> Diagnostics {
        let mut seen: Vec<(Option<String>, String)> = Vec::new();
        let mut diagnostics = Diagnostics::new();
        for diagnostic in self.diagnostics {
            // A shared palette is checked once for each map that uses
            // it; say each thing once.
            let key = (
                diagnostic.location.as_ref().map(|at| at.short()),
                diagnostic.message.clone(),
            );
            if diagnostic.location.is_some() && seen.contains(&key) {
                continue;
            }
            seen.push(key);
            diagnostics.push(diagnostic);
        }
        diagnostics.sort();
        diagnostics
    }

    fn report(
        &mut self,
        code: diag::Code,
        label: &DefLabel,
        span: Option<Span>,
        path: &str,
        message: impl Into<String>,
    ) -> &mut Diagnostic {
        let mut diagnostic = Diagnostic::new(code, message)
            .in_def(Some(label))
            .with_path(path);
        if let Some(span) = span {
            diagnostic = diagnostic.at(self.defs.sources(), span);
        }
        self.diagnostics.push(diagnostic);
        let last = self.diagnostics.len() - 1;
        &mut self.diagnostics[last]
    }

    fn error(&mut self, label: &DefLabel, site: &Site, message: impl Into<String>) {
        self.report(code::VALIDATION, label, site.span, &site.path, message);
    }

    fn error_in(
        &mut self,
        label: &DefLabel,
        text: &Located<String>,
        path: &str,
        within: (usize, usize),
        message: impl Into<String>,
    ) {
        let span = self
            .defs
            .sources()
            .span_in_string(text.span, within.0, within.1);
        self.report(code::VALIDATION, label, Some(span), path, message);
    }

    fn palette_key(&mut self, label: &DefLabel, key: &str, site: &Site) -> Option<char> {
        match single_char(key) {
            Some(LEAVE) => {
                self.error(
                    label,
                    site,
                    "a space always means \"leave what is there\" and cannot be a palette entry",
                );
                None
            }
            Some(ch) => Some(ch),
            None => {
                self.error(
                    label,
                    site,
                    format!("a palette entry is named by one character, and `{key}` is not one"),
                );
                None
            }
        }
    }

    fn resolve_state(
        &self,
        layer: Layer,
        text: &str,
        namespace: &str,
    ) -> Result<ElementState, StateRefError> {
        let namespace = Some(namespace).filter(|namespace| !namespace.is_empty());
        match layer {
            Layer::Floor => self
                .world
                .floor_state(text, namespace)
                .map(|state| state.raw()),
            Layer::Block => self
                .world
                .block_state(text, namespace)
                .map(|state| state.raw()),
            Layer::Fixture => self
                .world
                .fixture_state(text, namespace)
                .map(|state| state.raw()),
        }
    }

    /// Whether a layer has a definition a bare or qualified id means.
    fn has_def(&self, layer: Layer, id: &str, namespace: &str) -> bool {
        let namespace = Some(namespace).filter(|namespace| !namespace.is_empty());
        match layer {
            Layer::Floor => self.world.floors().resolve(id, namespace).is_some(),
            Layer::Block => self.world.blocks().resolve(id, namespace).is_some(),
            Layer::Fixture => self.world.fixtures().resolve(id, namespace).is_some(),
        }
    }

    /// One layer of a cell spec: a state, or a state written with
    /// parameters.
    fn state_slot(
        &mut self,
        cx: &MapCx<'_>,
        layer: Layer,
        text: &str,
        span: Span,
        path: &str,
    ) -> StateSlot {
        let located = Located {
            value: text.to_owned(),
            span,
        };
        if !text.contains('$') {
            return match self.resolve_state(layer, text, &cx.namespace) {
                Ok(state) => StateSlot::Fixed(state),
                Err(error) => {
                    let code = match error {
                        StateRefError::Malformed(_) => code::VALIDATION,
                        _ => code::UNKNOWN_REFERENCE,
                    };
                    self.report(code, &cx.label, Some(span), path, error.to_string());
                    StateSlot::Unset
                }
            };
        }
        let mut parts = Vec::new();
        let mut used: Vec<usize> = Vec::new();
        let mut rest = 0;
        let mut sound = true;
        for found in template_parts(text) {
            match found {
                Ok((start, end, name)) => {
                    if start > rest {
                        parts.push(TemplatePart::Text(text[rest..start].to_owned()));
                    }
                    rest = end;
                    match cx.params.iter().position(|param| param.name == name) {
                        Some(index) if cx.params[index].ty == ParamType::Str => {
                            parts.push(TemplatePart::Param(index));
                            if !used.contains(&index) {
                                used.push(index);
                            }
                        }
                        Some(index) => {
                            sound = false;
                            let ty = cx.params[index].ty.name();
                            self.error_in(
                                &cx.label,
                                &located,
                                path,
                                (start, end),
                                format!(
                                    "the parameter `${name}` is a {ty}, and a state is written \
                                     with a text: an id"
                                ),
                            );
                        }
                        None => {
                            sound = false;
                            let names = cx.params.iter().map(|param| param.name.as_str());
                            let suggestion = diag::did_you_mean(diag::closest(name, names))
                                .map(|text| text.replacen('"', "\"$", 1));
                            let span = self.defs.sources().span_in_string(span, start, end);
                            let message = format!(
                                "unresolved parameter `${name}`: the map `{}` declares no \
                                 parameter of that name",
                                cx.label.id
                            );
                            let diagnostic =
                                self.report(code::VALIDATION, &cx.label, Some(span), path, message);
                            if let Some(suggestion) = suggestion {
                                *diagnostic = diagnostic.clone().suggest(suggestion);
                            }
                        }
                    }
                }
                Err((start, end)) => {
                    sound = false;
                    self.error_in(
                        &cx.label,
                        &located,
                        path,
                        (start, end.max(start + 1)),
                        "a `$` is followed by the name of a parameter",
                    );
                }
            }
        }
        if rest < text.len() {
            parts.push(TemplatePart::Text(text[rest..].to_owned()));
        }
        if !sound {
            return StateSlot::Unset;
        }
        // Try the state with every value the map itself can give its
        // parameters. A value only a placement gives is tried when the
        // map is resolved.
        let mut choices: Vec<Vec<&str>> = Vec::new();
        for &index in &used {
            let values: Vec<&str> = match &cx.params[index].rule {
                CompiledRule::Pick(values) => values.iter().filter_map(|v| v.as_str()).collect(),
                CompiledRule::Default(value) => value.as_str().into_iter().collect(),
                _ => Vec::new(),
            };
            if values.is_empty() {
                return StateSlot::Template(parts);
            }
            choices.push(values);
        }
        let combinations: usize = choices.iter().map(Vec::len).product();
        if combinations <= MAX_TEMPLATE_COMBINATIONS {
            let mut pick = vec![0_usize; choices.len()];
            'all: loop {
                let mut written = String::new();
                let mut given = String::new();
                for part in &parts {
                    match part {
                        TemplatePart::Text(piece) => written.push_str(piece),
                        TemplatePart::Param(index) => {
                            let slot = used.iter().position(|have| have == index).unwrap_or(0);
                            let value = choices[slot][pick[slot]];
                            written.push_str(value);
                            if !given.is_empty() {
                                given.push_str(", ");
                            }
                            given.push_str(&format!("`${}` as `{value}`", cx.params[*index].name));
                        }
                    }
                }
                if let Err(error) = self.resolve_state(layer, &written, &cx.namespace) {
                    self.report(
                        code::UNKNOWN_REFERENCE,
                        &cx.label,
                        Some(span),
                        path,
                        format!("with {given}, `{text}` is `{written}`: {error}"),
                    );
                    return StateSlot::Unset;
                }
                let mut slot = 0;
                loop {
                    if slot == pick.len() {
                        break 'all;
                    }
                    pick[slot] += 1;
                    if pick[slot] < choices[slot].len() {
                        break;
                    }
                    pick[slot] = 0;
                    slot += 1;
                }
            }
        }
        StateSlot::Template(parts)
    }

    fn count(count: &Option<CountDef>) -> Option<CompiledCount> {
        count.as_ref().map(|count| match count {
            CountDef::Range(low, high) => CompiledCount::Range(*low, *high),
            CountDef::Expr(expr) => CompiledCount::Expr((**expr).clone()),
        })
    }

    fn spec(&mut self, cx: &mut MapCx<'_>, spec: &SpecDef) -> CompiledSpec {
        let label = cx.label.clone();
        let path = spec.site.path.clone();
        let mut slots = [StateSlot::Unset, StateSlot::Unset, StateSlot::Unset];
        if let (Some(short), Some(span)) = (&spec.short, spec.site.span) {
            // The short form: the layer is the one whose definition the
            // id names.
            let id = short.split(['@', '#']).next().unwrap_or("").trim();
            if id.contains('$') {
                self.error(
                    &label,
                    &spec.site,
                    format!(
                        "the short form needs the definition written out to tell its layer: \
                         write the long form, such as `{{ \"block\": \"{short}\" }}`"
                    ),
                );
            } else {
                let layers: Vec<Layer> = Layer::ALL
                    .into_iter()
                    .filter(|layer| self.has_def(*layer, id, &cx.namespace))
                    .collect();
                match layers.as_slice() {
                    [layer] => {
                        slots[layer.index()] = self.state_slot(cx, *layer, short, span, &path);
                    }
                    [] => {
                        self.report(
                            code::UNKNOWN_REFERENCE,
                            &label,
                            Some(span),
                            &path,
                            format!("there is no floor, block or fixture `{id}`"),
                        );
                    }
                    several => {
                        let names: Vec<&str> = several.iter().map(|layer| layer.name()).collect();
                        self.error(
                            &label,
                            &spec.site,
                            format!(
                                "`{id}` is both a {}: the short form cannot tell which is \
                                 meant. Write the long form, such as `{{ \"{}\": \"{short}\" }}`",
                                names.join(" and a "),
                                names[0],
                            ),
                        );
                    }
                }
            }
        }
        for (layer, text) in [
            (Layer::Floor, &spec.floor),
            (Layer::Block, &spec.block),
            (Layer::Fixture, &spec.fixture),
        ] {
            if let Some(text) = text {
                let at = format!("{path}.{}", layer.name());
                slots[layer.index()] = self.state_slot(cx, layer, &text.value, text.span, &at);
            }
        }
        let mut extras = Extras::default();
        for (names, keep) in [(&spec.clear, false), (&spec.keep, true)] {
            let key = if keep { "keep" } else { "clear" };
            for name in names {
                let layer = Layer::ALL
                    .into_iter()
                    .find(|layer| layer.name() == name.value);
                match (layer, ClearedLayer::from_name(&name.value)) {
                    (Some(layer), _) => {
                        if !matches!(slots[layer.index()], StateSlot::Unset) {
                            self.report(
                                code::VALIDATION,
                                &label,
                                Some(name.span),
                                &format!("{path}.{key}"),
                                format!(
                                    "the {} is given a state and also listed in `{key}`",
                                    layer.name()
                                ),
                            );
                        }
                        slots[layer.index()] = if keep {
                            StateSlot::Keep
                        } else {
                            StateSlot::Fixed(ElementState::NONE)
                        };
                    }
                    (None, Some(cleared)) if !keep => extras.cleared.push(cleared),
                    _ => {
                        let known: &[&str] = if keep {
                            &LAYER_NAMES[..3]
                        } else {
                            &LAYER_NAMES
                        };
                        let suggestion =
                            diag::did_you_mean(diag::closest(&name.value, known.iter().copied()));
                        let message = format!(
                            "`{}` is not a layer `{key}` takes: {}",
                            name.value,
                            known.join(", ")
                        );
                        let diagnostic = self.report(
                            code::VALIDATION,
                            &label,
                            Some(name.span),
                            &format!("{path}.{key}"),
                            message,
                        );
                        if let Some(suggestion) = suggestion {
                            *diagnostic = diagnostic.clone().suggest(suggestion);
                        }
                    }
                }
            }
        }
        extras.conduits.clone_from(&spec.conduits);
        extras.fields = spec
            .field
            .iter()
            .map(|(name, amount)| (name.clone(), *amount))
            .collect();
        if let Some(items) = &spec.items {
            let source = match (&items.item, &items.table) {
                (Some(item), _) => ItemSource::Item(item.clone()),
                (None, Some(table)) => ItemSource::Table(table.clone()),
                (None, None) => ItemSource::Item(String::new()),
            };
            extras.items = Some(CompiledItems {
                source,
                count: Builder::count(&items.count),
                chance: items.chance.clone(),
            });
        }
        if let Some(spawn) = &spec.spawn {
            extras.spawn = Some(CompiledSpawn {
                who: spawn_who(&spawn.creature, &spawn.population),
                count: Builder::count(&spawn.count),
                chance: spawn.chance.clone(),
            });
        }
        if let Some(device) = &spec.device {
            if let Some(name) = &device.name
                && !cx.devices.contains(name)
            {
                cx.devices.push(name.clone());
            }
            extras.device = Some(MapDevice {
                name: device.name.clone(),
                config: device
                    .config
                    .iter()
                    .map(|(key, value)| (key.clone(), value.0.clone()))
                    .collect(),
            });
        }
        extras.marks.clone_from(&spec.mark);
        extras.berth = spec
            .berth
            .as_ref()
            .map(|berth| (berth.name.clone(), berth.kind.clone()));
        extras.approach.clone_from(&spec.approach);
        extras.watched = spec.watched;
        if let Some(area) = &spec.area {
            let kind = self.area_kind(&label, area, &format!("{path}.area"), &cx.namespace);
            if !cx.area_names.contains(&area.value) {
                cx.area_names.push(area.value.clone());
            }
            extras.area = Some((area.value.clone(), kind));
        }
        extras.vars = spec
            .var
            .iter()
            .map(|(name, value)| (name.clone(), value.0.clone()))
            .collect();
        extras.decals.clone_from(&spec.decal);
        if spec.any
            || spec.floor_tag.is_some()
            || spec.block_tag.is_some()
            || spec.fixture_tag.is_some()
        {
            extras.pattern = Some(PatternMatch {
                floor_tag: spec.floor_tag.clone(),
                block_tag: spec.block_tag.clone(),
                fixture_tag: spec.fixture_tag.clone(),
                any: spec.any,
            });
        }
        let plain = extras.conduits.is_empty()
            && extras.fields.is_empty()
            && extras.items.is_none()
            && extras.spawn.is_none()
            && extras.device.is_none()
            && extras.marks.is_empty()
            && extras.berth.is_none()
            && extras.approach.is_none()
            && !extras.watched
            && extras.area.is_none()
            && extras.vars.is_empty()
            && extras.decals.is_empty()
            && extras.pattern.is_none()
            && extras.cleared.is_empty();
        let [floor, block, fixture] = slots;
        CompiledSpec {
            floor,
            block,
            fixture,
            extras: (!plain).then(|| Box::new(extras)),
        }
    }

    fn area_kind(
        &mut self,
        label: &DefLabel,
        kind: &Located<String>,
        path: &str,
        namespace: &str,
    ) -> Option<AreaKindId> {
        let namespace = Some(namespace).filter(|namespace| !namespace.is_empty());
        let found = self.world.area_kinds().resolve(&kind.value, namespace);
        if found.is_none() {
            self.report(
                code::UNKNOWN_REFERENCE,
                label,
                Some(kind.span),
                path,
                format!("there is no area_kind `{}`", kind.value),
            );
        }
        found
    }

    fn entry(&mut self, cx: &mut MapCx<'_>, entry: &EntryDef) -> u16 {
        let alternatives: Vec<CompiledAlt> = entry
            .alternatives
            .iter()
            .map(|alt: &AltDef| CompiledAlt {
                spec: self.spec(cx, &alt.spec),
                weight: alt.weight.clone(),
            })
            .collect();
        cx.entries.push(CompiledEntry {
            alternatives,
            once: entry.once,
        });
        // Fewer than 65,535 entries: a map has at most one for each
        // character and each `set`.
        u16::try_from(cx.entries.len() - 1).unwrap_or(NO_ENTRY - 1)
    }

    fn entry_of_char(&mut self, cx: &mut MapCx<'a>, ch: char) -> Option<u16> {
        if let Some(index) = cx.by_char.get(&ch) {
            return Some(*index);
        }
        let entry = *cx.palette.get(&ch)?;
        let index = self.entry(cx, entry);
        cx.by_char.insert(ch, index);
        Some(index)
    }

    /// A cell of a map as written: in the footprint and on a storey the
    /// map has.
    fn cell_in_map(
        &mut self,
        label: &DefLabel,
        site: &Site,
        cell: IVec3,
        size: IVec2,
        levels: (i32, i32),
        what: &str,
    ) -> bool {
        let inside = cell.x >= 0 && cell.y >= 0 && cell.x < size.x && cell.y < size.y;
        if !inside {
            self.error(
                label,
                site,
                format!(
                    "{what} is at [{}, {}], outside the map, which is {} by {}",
                    cell.x, cell.y, size.x, size.y
                ),
            );
            return false;
        }
        if cell.z < levels.0 || cell.z > levels.1 {
            self.error(
                label,
                site,
                format!(
                    "{what} is on level {}, and the map has levels {} to {}",
                    cell.z, levels.0, levels.1
                ),
            );
            return false;
        }
        true
    }

    fn point(
        &mut self,
        label: &DefLabel,
        point: &PointDef,
        marks: &BTreeMap<String, Vec<IVec3>>,
        size: IVec2,
        levels: (i32, i32),
        what: &str,
    ) -> Option<IVec3> {
        match point {
            PointDef::Cell(cell, site) => self
                .cell_in_map(label, site, *cell, size, levels, what)
                .then_some(*cell),
            PointDef::Mark(name, site) => {
                let found = marks.get(name).and_then(|cells| cells.first()).copied();
                if found.is_none() {
                    self.unknown_mark(label, site, name, marks);
                }
                found
            }
        }
    }

    fn unknown_mark(
        &mut self,
        label: &DefLabel,
        site: &Site,
        name: &str,
        marks: &BTreeMap<String, Vec<IVec3>>,
    ) {
        let suggestion = diag::did_you_mean(diag::closest(name, marks.keys().map(String::as_str)));
        let message = format!(
            "the map has no mark `{name}` that is the same in every placement: a mark comes \
             from `marks`, or from a character with one cell spec"
        );
        let diagnostic = self.report(
            code::UNKNOWN_REFERENCE,
            label,
            site.span,
            &site.path,
            message,
        );
        if let Some(suggestion) = suggestion {
            *diagnostic = diagnostic.clone().suggest(suggestion);
        }
    }

    /// A box `[[x0, y0], [x1, y1]]` on a span of storeys.
    fn bounds(
        &mut self,
        label: &DefLabel,
        site: &Site,
        corners: &[Vec<i64>],
        levels: (i32, i32),
        size: IVec2,
        map_levels: (i32, i32),
    ) -> Option<(IVec3, IVec3)> {
        let corner = |corner: &Vec<i64>| match corner.as_slice() {
            [x, y] => Some(IVec2::new(i32::try_from(*x).ok()?, i32::try_from(*y).ok()?)),
            _ => None,
        };
        let (Some(a), Some(b)) = (
            corners.first().and_then(corner),
            corners.get(1).and_then(corner),
        ) else {
            self.error(label, site, "a box is `[[x0, y0], [x1, y1]]`");
            return None;
        };
        if corners.len() != 2 {
            self.error(label, site, "a box is `[[x0, y0], [x1, y1]]`");
            return None;
        }
        let min = a.component_min(b).with_z(levels.0);
        let max = a.component_max(b).with_z(levels.1);
        (self.cell_in_map(label, site, min, size, map_levels, "the box's first corner")
            && self.cell_in_map(label, site, max, size, map_levels, "the box's far corner"))
        .then_some((min, max))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one pass over one definition, field by field"
    )]
    fn map(&mut self, id: MapId, name: &str, def: &'a MapDef) -> CompiledMap {
        let label = DefLabel::new("map", name);
        let namespace = namespace_of(name).to_owned();

        // Size.
        let size = match def.size.value.as_slice() {
            [w, d] if (1..=MAX_MAP_SIDE).contains(w) && (1..=MAX_MAP_SIDE).contains(d) => {
                IVec2::new(*w as i32, *d as i32)
            }
            _ => {
                self.error(
                    &label,
                    &def.size.site,
                    format!("`size` is `[width, depth]` in cells, each from 1 to {MAX_MAP_SIDE}"),
                );
                IVec2::new(1, 1)
            }
        };

        // Parameters.
        let mut params: Vec<CompiledParam> = Vec::new();
        for (param_name, param) in &def.params.0 {
            let declared = match &param.ty {
                Some(ty) => match ParamType::from_name(&ty.value) {
                    Some(found) => Some(found),
                    None => {
                        let suggestion = diag::did_you_mean(diag::closest(&ty.value, PARAM_TYPES));
                        let message = format!(
                            "`{}` is not a type a parameter can have: {}",
                            ty.value,
                            PARAM_TYPES.join(", ")
                        );
                        let path = format!("params.{param_name}.type");
                        let diagnostic =
                            self.report(code::VALIDATION, &label, Some(ty.span), &path, message);
                        if let Some(suggestion) = suggestion {
                            *diagnostic = diagnostic.clone().suggest(suggestion);
                        }
                        None
                    }
                },
                None => None,
            };
            let inferred = match &param.rule {
                ParamRule::Expr(expr) => expr
                    .compiled()
                    .map(|code| self.defs.expr_registry().type_name(code.ty().non_null()))
                    .or_else(|| {
                        expr.constant().and_then(|value| match value {
                            clash_expr::Value::Bool(_) => Some("bool".to_owned()),
                            clash_expr::Value::Int(_) => Some("int".to_owned()),
                            clash_expr::Value::Real(_) => Some("real".to_owned()),
                            clash_expr::Value::Str(_) => Some("string".to_owned()),
                            _ => None,
                        })
                    }),
                _ => param.rule_type(),
            };
            let found = inferred.as_deref().and_then(ParamType::from_name);
            let ty = declared.or(found).unwrap_or(ParamType::Real);
            if let (Some(declared), Some(found), ParamRule::Expr(_)) =
                (declared, found, &param.rule)
                && declared != found
                && !(declared == ParamType::Real && found == ParamType::Int)
            {
                self.error(
                    &label,
                    &param.site,
                    format!(
                        "the parameter `${param_name}` is declared a {}, and its expression \
                         gives a {}",
                        declared.name(),
                        found.name()
                    ),
                );
            }
            let mismatch = |builder: &mut Builder<'a>, value: &clash_core::Value| {
                if ty.accept(value).is_none() {
                    builder.error(
                        &label,
                        &param.site,
                        format!(
                            "the parameter `${param_name}` is a {}, and `{value}` is a {}",
                            ty.name(),
                            value.type_name()
                        ),
                    );
                }
            };
            let rule = match &param.rule {
                ParamRule::Pick(values) => {
                    values.iter().for_each(|value| mismatch(self, &value.0));
                    CompiledRule::Pick(
                        values
                            .iter()
                            .map(|value| ty.accept(&value.0).unwrap_or_else(|| value.0.clone()))
                            .collect(),
                    )
                }
                ParamRule::Range(low, high) => {
                    use clash_core::Value as V;
                    let real = |value: &V| match value {
                        V::Int(int) => Some(Real::of(*int as f64)),
                        V::Real(real) => Some(*real),
                        _ => None,
                    };
                    match (&low.0, &high.0, ty) {
                        (V::Int(a), V::Int(b), ParamType::Int) if a <= b => {
                            CompiledRule::RangeInt(*a, *b)
                        }
                        (a, b, ParamType::Real)
                            if real(a).zip(real(b)).is_some_and(|(a, b)| a <= b) =>
                        {
                            CompiledRule::RangeReal(
                                real(a).unwrap_or(Real::ZERO),
                                real(b).unwrap_or(Real::ZERO),
                            )
                        }
                        _ => {
                            self.error(
                                &label,
                                &param.site,
                                format!(
                                    "`range` is `[low, high]`, two numbers with `low` first, \
                                     and whole numbers for an int: `${param_name}` is a {}",
                                    ty.name()
                                ),
                            );
                            CompiledRule::Required
                        }
                    }
                }
                ParamRule::Default(value) => {
                    mismatch(self, &value.0);
                    CompiledRule::Default(ty.accept(&value.0).unwrap_or_else(|| value.0.clone()))
                }
                ParamRule::Expr(expr) => CompiledRule::Expr((**expr).clone()),
                ParamRule::Required => CompiledRule::Required,
            };
            params.push(CompiledParam {
                name: param_name.clone(),
                ty,
                rule,
            });
        }

        // The palette: named palettes and entries written in place,
        // later overriding earlier.
        let mut palette: BTreeMap<char, &'a EntryDef> = BTreeMap::new();
        for item in &def.palette {
            match item {
                PaletteItem::Named(palette_ref) => {
                    let shared = self.defs.get::<PaletteDef>(palette_ref.id());
                    for (key, entry) in &shared.entries {
                        if let Some(ch) = single_char(key).filter(|ch| *ch != LEAVE) {
                            palette.insert(ch, entry);
                        }
                    }
                }
                PaletteItem::Inline(entries, _) => {
                    for (key, entry) in entries {
                        if let Some(ch) = self.palette_key(&label, key, &entry.site) {
                            palette.insert(ch, entry);
                        }
                    }
                }
            }
        }
        let mut cx = MapCx {
            label: label.clone(),
            namespace: namespace.clone(),
            params,
            palette,
            entries: Vec::new(),
            by_char: BTreeMap::new(),
            devices: Vec::new(),
            area_names: Vec::new(),
        };

        // Levels.
        let mut written: BTreeMap<i32, &'a LevelDef> = BTreeMap::new();
        for (key, level) in &def.levels {
            let site = match level {
                LevelDef::Rows(_, site) | LevelDef::Preset(_, site) => site,
            };
            match key.parse::<i32>() {
                Ok(z) if i64::from(z).abs() <= MAX_MAP_LEVELS => {
                    written.insert(z, level);
                }
                _ => self.error(
                    &label,
                    site,
                    format!(
                        "a level is named by its storey, a whole number relative to the map's \
                         ground level, and `{key}` is not one"
                    ),
                ),
            }
        }
        let lowest = written.keys().next().copied().unwrap_or(0);
        let highest = written.keys().next_back().copied().unwrap_or(0);
        if written.is_empty() {
            self.report(
                code::VALIDATION,
                &label,
                None,
                "levels",
                "a map needs at least one level",
            );
        }
        let span_levels = i64::from(highest) - i64::from(lowest) + 1;
        let mut levels: Vec<CompiledLevel> = Vec::new();
        let mut marks: BTreeMap<String, Vec<IVec3>> = BTreeMap::new();
        if span_levels > MAX_MAP_LEVELS {
            self.report(
                code::VALIDATION,
                &label,
                None,
                "levels",
                format!(
                    "a map has at most {MAX_MAP_LEVELS} storeys from its lowest to its highest"
                ),
            );
        } else if span_levels * i64::from(size.x) * i64::from(size.y) > MAX_MAP_CELLS {
            self.error(
                &label,
                &def.size.site,
                format!(
                    "this map has {} cells over its {span_levels} storeys, and a map has at \
                     most {MAX_MAP_CELLS}: build a larger place from several maps",
                    span_levels * i64::from(size.x) * i64::from(size.y)
                ),
            );
        } else {
            for z in lowest..=highest {
                let level = match written.get(&z) {
                    None => CompiledLevel::Empty,
                    Some(LevelDef::Preset(preset, site)) => {
                        if preset != PRESET_ROOF {
                            let suggestion = diag::did_you_mean(diag::closest(preset, PRESETS));
                            let message = format!(
                                "`{preset}` is not a preset a level can be: {}",
                                PRESETS.join(", ")
                            );
                            let diagnostic = self.report(
                                code::VALIDATION,
                                &label,
                                site.span,
                                &site.path,
                                message,
                            );
                            if let Some(suggestion) = suggestion {
                                *diagnostic = diagnostic.clone().suggest(suggestion);
                            }
                            CompiledLevel::Empty
                        } else if !written.contains_key(&(z - 1)) {
                            self.error(
                                &label,
                                site,
                                format!(
                                    "a roof covers the storey below it, and the map has no \
                                     level {}",
                                    z - 1
                                ),
                            );
                            CompiledLevel::Empty
                        } else {
                            CompiledLevel::Roof
                        }
                    }
                    Some(LevelDef::Rows(rows, site)) => {
                        if rows.len() != size.y as usize {
                            self.error(
                                &label,
                                site,
                                format!(
                                    "level {z} has {} rows, and the map is {} deep: every \
                                     level has `size[1]` rows",
                                    rows.len(),
                                    size.y
                                ),
                            );
                        }
                        let mut cells = vec![NO_ENTRY; (size.x * size.y) as usize];
                        for (y, row) in rows.iter().enumerate().take(size.y as usize) {
                            let path = format!("{}[{y}]", site.path);
                            let length = row.value.chars().count();
                            if length != size.x as usize {
                                self.report(
                                    code::VALIDATION,
                                    &label,
                                    Some(row.span),
                                    &path,
                                    format!(
                                        "this row has {length} characters, and the map is {} \
                                         wide: every row has `size[0]` characters",
                                        size.x
                                    ),
                                );
                                continue;
                            }
                            for (x, (offset, ch)) in row.value.char_indices().enumerate() {
                                if ch == LEAVE {
                                    continue;
                                }
                                match self.entry_of_char(&mut cx, ch) {
                                    Some(index) => {
                                        cells[y * size.x as usize + x] = index;
                                        let entry = &cx.entries[index as usize];
                                        if let [only] = entry.alternatives.as_slice()
                                            && let Some(extras) = &only.spec.extras
                                        {
                                            for mark in &extras.marks {
                                                marks
                                                    .entry(mark.clone())
                                                    .or_default()
                                                    .push(IVec3::new(x as i32, y as i32, z));
                                            }
                                        }
                                    }
                                    None => {
                                        let known: String = cx.palette.keys().collect();
                                        self.error_in(
                                            &label,
                                            row,
                                            &path,
                                            (offset, offset + ch.len_utf8()),
                                            format!(
                                                "unknown character `{ch}`: the palette has no \
                                                 entry for it. It has `{known}`"
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                        CompiledLevel::Rows(cells)
                    }
                };
                levels.push(level);
            }
        }
        if levels.is_empty() {
            levels.push(CompiledLevel::Empty);
        }
        let level_range = (lowest, lowest + levels.len() as i32 - 1);

        let default = def.default.as_ref().map(|entry| self.entry(&mut cx, entry));
        let roof = def.roof.as_ref().map(|entry| self.entry(&mut cx, entry));
        if roof.is_none()
            && default.is_none()
            && let Some(LevelDef::Preset(_, site)) = written
                .values()
                .find(|level| matches!(level, LevelDef::Preset(..)))
        {
            self.error(
                &label,
                site,
                "this level is a roof, and the map says what a roof is made of with neither \
                 `roof` nor `default`",
            );
        }

        // Marks written out.
        for (mark, cells) in &def.marks {
            for cell in &cells.cells {
                if self.cell_in_map(
                    &label,
                    &cells.site,
                    *cell,
                    size,
                    level_range,
                    &format!("the mark `{mark}`"),
                ) {
                    marks.entry(mark.clone()).or_default().push(*cell);
                }
            }
        }
        for cells in marks.values_mut() {
            cells.sort_by_key(|cell| (cell.z, cell.y, cell.x));
            cells.dedup();
        }

        // Cell specs at cells and over boxes.
        let mut set = Vec::new();
        for item in &def.set {
            let entry = self.entry(&mut cx, &item.spec);
            let site = &item.spec.site;
            match (&item.at, &item.bounds) {
                (Some(point), None) => {
                    if let Some(cell) =
                        self.point(&label, point, &marks, size, level_range, "the cell")
                    {
                        set.push(CompiledSet {
                            min: cell,
                            max: cell,
                            entry,
                        });
                    }
                }
                (None, Some(bounds)) => {
                    let span = item.level.map_or((0, 0), |level| (level.0, level.1));
                    if let Some((min, max)) =
                        self.bounds(&label, &bounds.site, &bounds.value, span, size, level_range)
                    {
                        set.push(CompiledSet { min, max, entry });
                    }
                }
                _ => self.error(
                    &label,
                    site,
                    "an entry of `set` says where with one of `at` and `box`",
                ),
            }
        }

        // Areas.
        let mut areas = Vec::new();
        for area in &def.areas {
            let area: &AreaDef = area;
            let kind = self.area_kind(&label, &area.kind, "areas.kind", &namespace);
            let span = area.level.map_or((0, 0), |level| (level.0, level.1));
            let name = area.name.clone().unwrap_or_else(|| area.kind.value.clone());
            if !cx.area_names.contains(&name) {
                cx.area_names.push(name.clone());
            }
            if let Some((min, max)) = self.bounds(
                &label,
                &area.bounds.site,
                &area.bounds.value,
                span,
                size,
                level_range,
            ) {
                let mut tags = area.tags.clone();
                tags.sort();
                tags.dedup();
                areas.push(CompiledArea {
                    name,
                    kind,
                    tags,
                    min,
                    max,
                });
            }
        }

        // Spawns.
        let mut spawns = Vec::new();
        for (index, spawn) in def.spawns.iter().enumerate() {
            if spawn.creature.is_some() == spawn.population.is_some() {
                self.report(
                    code::VALIDATION,
                    &label,
                    None,
                    &format!("spawns[{index}]"),
                    "a spawn names one of `creature` and `population`",
                );
            }
            let at = match (&spawn.at, &spawn.within) {
                (Some(point), None) => self
                    .point(&label, point, &marks, size, level_range, "the spawn")
                    .map_or(SpawnAt::Anywhere, SpawnAt::Cell),
                (None, Some(area)) => {
                    SpawnAt::Area(self.area_name(&cx, area, &format!("spawns[{index}].in")))
                }
                (None, None) => SpawnAt::Anywhere,
                (Some(point), Some(_)) => {
                    self.error(&label, point.site(), "a spawn has one of `at` and `in`");
                    SpawnAt::Anywhere
                }
            };
            spawns.push(CompiledMapSpawn {
                spawn: CompiledSpawn {
                    who: spawn_who(&spawn.creature, &spawn.population),
                    count: Builder::count(&spawn.count),
                    chance: spawn.chance.clone(),
                },
                at,
            });
        }

        // Wiring.
        let mut wiring = Vec::new();
        let mut links = Vec::new();
        if let Some(def_wiring) = &def.wiring {
            let mut conduits: BTreeMap<char, &Vec<String>> = BTreeMap::new();
            for (key, lines) in &def_wiring.palette {
                if let Some(ch) = self.palette_key(&label, key, &lines.site) {
                    conduits.insert(ch, &lines.value);
                }
            }
            for (key, rows) in &def_wiring.levels {
                let Ok(z) = key.parse::<i32>() else {
                    self.error(
                        &label,
                        &rows.site,
                        format!("a wiring level is named by its storey, and `{key}` is not one"),
                    );
                    continue;
                };
                if z < level_range.0 || z > level_range.1 {
                    self.error(
                        &label,
                        &rows.site,
                        format!(
                            "wiring is drawn on level {z}, and the map has levels {} to {}",
                            level_range.0, level_range.1
                        ),
                    );
                    continue;
                }
                if rows.value.len() != size.y as usize {
                    self.error(
                        &label,
                        &rows.site,
                        format!(
                            "wiring level {z} has {} rows, and the map is {} deep",
                            rows.value.len(),
                            size.y
                        ),
                    );
                }
                for (y, row) in rows.value.iter().enumerate().take(size.y as usize) {
                    let path = format!("{}[{y}]", rows.site.path);
                    let length = row.value.chars().count();
                    if length != size.x as usize {
                        self.report(
                            code::VALIDATION,
                            &label,
                            Some(row.span),
                            &path,
                            format!(
                                "this wiring row has {length} characters, and the map is {} wide",
                                size.x
                            ),
                        );
                        continue;
                    }
                    for (x, (offset, ch)) in row.value.char_indices().enumerate() {
                        if ch == LEAVE {
                            continue;
                        }
                        match conduits.get(&ch) {
                            Some(lines) => {
                                for line in *lines {
                                    wiring.push((IVec3::new(x as i32, y as i32, z), line.clone()));
                                }
                            }
                            None => self.error_in(
                                &label,
                                row,
                                &path,
                                (offset, offset + ch.len_utf8()),
                                format!(
                                    "unknown character `{ch}`: the wiring palette has no entry \
                                     for it"
                                ),
                            ),
                        }
                    }
                }
            }
        }

        // Compile every entry of the palette, used or not, so that a
        // device named by an unused character is still known and a
        // mistake in it is still found.
        let unused: Vec<char> = cx
            .palette
            .keys()
            .copied()
            .filter(|ch| !cx.by_char.contains_key(ch))
            .collect();
        for ch in unused {
            self.entry_of_char(&mut cx, ch);
        }
        if let Some(def_wiring) = &def.wiring {
            for (index, link) in def_wiring.links.iter().enumerate() {
                for (end, name) in [("from", &link.from), ("to", &link.to)] {
                    if !cx.devices.contains(&name.value) {
                        let suggestion = diag::did_you_mean(diag::closest(
                            &name.value,
                            cx.devices.iter().map(String::as_str),
                        ));
                        let message = format!(
                            "this link runs {end} `{}`, and no cell spec of the map names a \
                             device that: a device is named with `\"device\": {{ \"name\": … }}`",
                            name.value
                        );
                        let path = format!("wiring.links[{index}].{end}");
                        let diagnostic = self.report(
                            code::UNKNOWN_REFERENCE,
                            &label,
                            Some(name.span),
                            &path,
                            message,
                        );
                        if let Some(suggestion) = suggestion {
                            *diagnostic = diagnostic.clone().suggest(suggestion);
                        }
                    }
                }
                links.push((
                    link.from.value.clone(),
                    link.to.value.clone(),
                    link.resource.clone(),
                ));
            }
        }

        // Joints.
        let mut joints = Vec::new();
        for joint in &def.joints {
            let cell = match joint.at.value.as_slice() {
                [x, y] => i32::try_from(*x)
                    .ok()
                    .zip(i32::try_from(*y).ok())
                    .map(|(x, y)| IVec3::new(x, y, 0)),
                [x, y, z] => i32::try_from(*x)
                    .ok()
                    .zip(i32::try_from(*y).ok())
                    .zip(i32::try_from(*z).ok())
                    .map(|((x, y), z)| IVec3::new(x, y, z)),
                _ => None,
            };
            let Some(cell) = cell else {
                self.error(
                    &label,
                    &joint.at.site,
                    "a joint is at `[x, y]` or `[x, y, level]`",
                );
                continue;
            };
            if !self.cell_in_map(&label, &joint.at.site, cell, size, level_range, "the joint") {
                continue;
            }
            let dir = match joint.dir.value.as_str() {
                "north" => Some(JointDir::Side(Dir8::North)),
                "east" => Some(JointDir::Side(Dir8::East)),
                "south" => Some(JointDir::Side(Dir8::South)),
                "west" => Some(JointDir::Side(Dir8::West)),
                "up" => Some(JointDir::Vertical(Vertical::Up)),
                "down" => Some(JointDir::Vertical(Vertical::Down)),
                _ => None,
            };
            let Some(dir) = dir else {
                self.report(
                    code::VALIDATION,
                    &label,
                    Some(joint.dir.span),
                    "joints.dir",
                    format!(
                        "`{}` is not a way a joint can open: north, east, south, west, up, down",
                        joint.dir.value
                    ),
                );
                continue;
            };
            let on_edge = match dir {
                JointDir::Side(Dir8::North) => cell.y == 0,
                JointDir::Side(Dir8::East) => cell.x == size.x - 1,
                JointDir::Side(Dir8::South) => cell.y == size.y - 1,
                JointDir::Side(Dir8::West) => cell.x == 0,
                JointDir::Side(_) => false,
                JointDir::Vertical(Vertical::Up) => cell.z == level_range.1,
                JointDir::Vertical(Vertical::Down) => cell.z == level_range.0,
            };
            if !on_edge {
                self.error(
                    &label,
                    &joint.at.site,
                    format!(
                        "this joint opens {} from [{}, {}, {}], which is not on that edge of \
                         the map: a joint is an opening on the boundary",
                        joint.dir.value, cell.x, cell.y, cell.z
                    ),
                );
                continue;
            }
            joints.push(CompiledJoint {
                id: joint.id.clone(),
                at: cell,
                dir,
                kind: joint.kind.clone(),
            });
        }

        // The anchor.
        let anchor = def
            .anchor
            .as_ref()
            .and_then(|point| self.point(&label, point, &marks, size, level_range, "the anchor"));

        // The front.
        let front = def.front.as_ref().and_then(|front| {
            if front.value.is_cardinal() {
                Some(front.value)
            } else {
                self.report(
                    code::VALIDATION,
                    &label,
                    Some(front.span),
                    "front",
                    "a map's front is one of its four sides: north, east, south or west",
                );
                None
            }
        });

        // How it sits.
        let foundation = def.foundation.as_ref().and_then(|state| {
            match self.resolve_state(Layer::Block, &state.value, &namespace) {
                Ok(state) => Some(BlockState::from_raw(state)),
                Err(error) => {
                    self.report(
                        code::UNKNOWN_REFERENCE,
                        &label,
                        Some(state.span),
                        "foundation",
                        error.to_string(),
                    );
                    None
                }
            }
        });
        let sit = match def.sit.as_ref().map(|sit| (sit.value.as_str(), sit)) {
            None | Some(("level", _)) => Sit::Level,
            Some(("foundation", _)) => Sit::Foundation(foundation),
            Some(("follow", _)) => Sit::Follow,
            Some(("none", _)) => Sit::None,
            Some((other, sit)) => {
                self.report(
                    code::VALIDATION,
                    &label,
                    Some(sit.span),
                    "sit",
                    format!("`{other}` is not a way to sit: level, foundation, follow, none"),
                );
                Sit::Level
            }
        };
        if let (Some(state), false) = (&def.foundation, matches!(sit, Sit::Foundation(_))) {
            self.report(
                code::VALIDATION,
                &label,
                Some(state.span),
                "foundation",
                "`foundation` names the fill of `\"sit\": \"foundation\"`, which this map does \
                 not ask for",
            );
        }

        // Portals and vehicles.
        let mut portals = Vec::new();
        for (index, portal) in def.portals.iter().enumerate() {
            let mark_of = |builder: &mut Builder<'a>, text: &Located<String>, end: &str| {
                let name = text.value.strip_prefix(MARK_PREFIX)?;
                if !marks.contains_key(name) {
                    let site = Site {
                        span: Some(text.span),
                        path: format!("portals[{index}].{end}"),
                    };
                    builder.unknown_mark(&label, &site, name, &marks);
                }
                Some(name.to_owned())
            };
            let Some(from) = mark_of(self, &portal.from, "from") else {
                self.report(
                    code::VALIDATION,
                    &label,
                    Some(portal.from.span),
                    &format!("portals[{index}].from"),
                    "a portal starts at a mark of the map: `\"mark:name\"`",
                );
                continue;
            };
            let to = match mark_of(self, &portal.to, "to") {
                Some(mark) => PortalTo::Mark(mark),
                None => PortalTo::Entry(portal.to.value.clone()),
            };
            portals.push((from, to));
        }
        let mut vehicles = Vec::new();
        for vehicle in &def.vehicles {
            if let Some(cell) = self.point(
                &label,
                &vehicle.at,
                &marks,
                size,
                level_range,
                "the vehicle",
            ) {
                vehicles.push((vehicle.vehicle.clone(), cell, vehicle.facing));
            }
        }

        // Nested maps: what can be checked from this map alone. Whether
        // they fit and whether they nest in circles is checked once
        // every map is built.
        let mut place = Vec::new();
        for (index, item) in def.place.iter().enumerate() {
            let candidates = match &item.map {
                MapPickDef::Id(map, _) => vec![map.id()],
                MapPickDef::Tag(tag, site) => {
                    let found: Vec<MapId> = self
                        .defs
                        .tags()
                        .lookup(tag)
                        .and_then(|tag| {
                            let registry = self.defs.try_registry::<MapDef>()?;
                            Some(
                                registry
                                    .iter()
                                    .filter(|(_, map)| map.tags.contains(tag))
                                    .map(|(id, _)| id)
                                    .collect(),
                            )
                        })
                        .unwrap_or_default();
                    if found.is_empty() {
                        self.report(
                            code::UNKNOWN_REFERENCE,
                            &label,
                            site.span,
                            &site.path,
                            format!("no map carries the tag `{tag}`"),
                        );
                    }
                    found
                }
            };
            let at = match (&item.at, &item.within) {
                (Some(point), None) => self
                    .point(&label, point, &marks, size, level_range, "the nested map")
                    .map(PlaceAt::Cell),
                (None, Some(area)) => Some(PlaceAt::Area(self.area_name(
                    &cx,
                    area,
                    &format!("place[{index}].in"),
                ))),
                _ => {
                    self.error(
                        &label,
                        item.map.site(),
                        "a nested map says where it goes with one of `at` and `in`",
                    );
                    None
                }
            };
            let Some(at) = at else { continue };
            if matches!(at, PlaceAt::Cell(_)) && (item.count.is_some() || item.apart.is_some()) {
                self.error(
                    &label,
                    item.map.site(),
                    "`count` and `apart` scatter copies through an area and belong with `in`; \
                     a map placed with `at` is placed once",
                );
            }
            if candidates.is_empty() {
                continue;
            }
            place.push(CompiledPlace {
                candidates,
                at,
                turn: match item.turn {
                    None => TurnRule::Fixed(Turn::IDENTITY),
                    Some(TurnDef::Fixed(turn)) => TurnRule::Fixed(turn),
                    Some(TurnDef::Any) => TurnRule::Any,
                },
                chance: item.chance.clone(),
                count: Builder::count(&item.count),
                apart: item.apart.unwrap_or(0),
                params: item
                    .params
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
                site: item.map.site().clone(),
            });
        }

        let mut tags: Vec<String> = def
            .tags
            .iter()
            .map(|tag| self.defs.tags().name(tag).to_owned())
            .collect();
        tags.sort();

        CompiledMap {
            id,
            name: name.to_owned(),
            namespace,
            tags,
            size,
            lowest,
            levels,
            params: cx.params,
            condition: def.condition.clone(),
            entries: cx.entries,
            default,
            roof,
            set,
            place,
            areas,
            marks,
            spawns,
            wiring,
            links,
            joints,
            anchor,
            front,
            base: def.base.unwrap_or_default(),
            sit,
            may_turn: def.turn,
            over: def.over,
            portals,
            vehicles,
        }
    }

    /// The name of an area a `place` or a spawn is put in, checked
    /// against the areas the map can have.
    fn area_name(&mut self, cx: &MapCx<'_>, area: &Located<String>, path: &str) -> String {
        let name = area
            .value
            .strip_prefix(AREA_PREFIX)
            .unwrap_or(&area.value)
            .to_owned();
        if !cx.area_names.contains(&name) {
            let suggestion = diag::did_you_mean(diag::closest(
                &name,
                cx.area_names.iter().map(String::as_str),
            ));
            let message = format!(
                "the map has no area `{name}`: an area comes from `areas`, by its name or its \
                 kind, or from cell specs with `area`"
            );
            let diagnostic = self.report(
                code::UNKNOWN_REFERENCE,
                &cx.label,
                Some(area.span),
                path,
                message,
            );
            if let Some(suggestion) = suggestion {
                *diagnostic = diagnostic.clone().suggest(suggestion);
            }
        }
        name
    }

    /// The checks that need every map: that each nested map can fit,
    /// that the parameters it is handed are its own, that no map nests
    /// inside itself, and that none nests deeper than the limit
    /// (10 §3.7).
    fn nesting(&mut self, maps: &mut [CompiledMap], limits: MapLimits) {
        for map in maps.iter() {
            let label = DefLabel::new("map", map.name.clone());
            for place in &map.place {
                let mut any_fits = false;
                for &candidate in &place.candidates {
                    let Some(child) = maps.get(candidate.index()) else {
                        continue;
                    };
                    for (name, value) in &place.params {
                        if !child.params.iter().any(|param| param.name == *name) {
                            let names = child.params.iter().map(|param| param.name.as_str());
                            let suggestion = diag::did_you_mean(diag::closest(name, names));
                            let message = format!(
                                "the map `{}` has no parameter `{name}` to be given",
                                child.name
                            );
                            let site = value.site().clone();
                            let diagnostic = self.report(
                                code::VALIDATION,
                                &label,
                                site.span,
                                &site.path,
                                message,
                            );
                            if let Some(suggestion) = suggestion {
                                *diagnostic = diagnostic.clone().suggest(suggestion);
                            }
                        }
                    }
                    let (size, levels) = (child.size, child.level_range());
                    let (anchor, may_turn) = (child.anchor, child.may_turn);
                    if let TurnRule::Fixed(turn) = place.turn
                        && turn != Turn::IDENTITY
                        && !may_turn
                    {
                        self.error(
                            &label,
                            &place.site,
                            format!(
                                "the map `{}` says `\"turn\": false` and is placed turned",
                                child.name
                            ),
                        );
                        continue;
                    }
                    let fits = allowed_turns(place.turn, may_turn).into_iter().any(|turn| {
                        match &place.at {
                            PlaceAt::Cell(at) => fits_at(map, size, levels, anchor, turn, *at),
                            PlaceAt::Area(_) => {
                                let turned = turned_size(size, turn);
                                let (low, high) = map.level_range();
                                turned.x <= map.size.x
                                    && turned.y <= map.size.y
                                    && levels.1 - levels.0 <= high - low
                            }
                        }
                    });
                    any_fits |= fits;
                    if !fits && place.candidates.len() == 1 {
                        let (low, high) = map.level_range();
                        self.error(
                            &label,
                            &place.site,
                            format!(
                                "the nested map `{}` cannot fit: it is {} by {} on levels {} \
                                 to {}, and this map is {} by {} on levels {low} to {high}",
                                child.name,
                                size.x,
                                size.y,
                                levels.0,
                                levels.1,
                                map.size.x,
                                map.size.y,
                            ),
                        );
                    }
                }
                if !any_fits && place.candidates.len() > 1 {
                    self.error(
                        &label,
                        &place.site,
                        "none of the maps carrying this tag can fit where it is placed",
                    );
                }
            }
        }

        // Circles and depth, by a walk from each map.
        let count = maps.len();
        let mut state = vec![0_u8; count];
        let mut depth = vec![0_u32; count];
        let mut circular = vec![false; count];
        for start in 0..count {
            let mut trail: Vec<usize> = Vec::new();
            self.walk(
                maps,
                start,
                &mut state,
                &mut depth,
                &mut circular,
                &mut trail,
            );
        }
        for (index, map) in maps.iter().enumerate() {
            if circular[index] || depth[index] != limits.max_nesting + 1 {
                continue;
            }
            let label = DefLabel::new("map", map.name.clone());
            let site = map.place.first().map(|place| place.site.clone());
            self.error(
                &label,
                &site.unwrap_or_default(),
                format!(
                    "maps nest {} deep below `{}`, and `gen.max_nesting` is {}",
                    depth[index], map.name, limits.max_nesting
                ),
            );
        }
        // A map in a circle is never resolved through it.
        for (index, map) in maps.iter_mut().enumerate() {
            if circular[index] {
                map.place.clear();
            }
        }
    }

    fn walk(
        &mut self,
        maps: &[CompiledMap],
        at: usize,
        state: &mut [u8],
        depth: &mut [u32],
        circular: &mut [bool],
        trail: &mut Vec<usize>,
    ) {
        const UNSEEN: u8 = 0;
        const OPEN: u8 = 1;
        const DONE: u8 = 2;
        if state[at] != UNSEEN {
            return;
        }
        state[at] = OPEN;
        trail.push(at);
        let mut deepest = 0;
        for place in &maps[at].place {
            for candidate in &place.candidates {
                let child = candidate.index();
                if child >= maps.len() {
                    continue;
                }
                if state[child] == OPEN {
                    let from = trail.iter().position(|seen| *seen == child).unwrap_or(0);
                    let mut names: Vec<&str> = trail[from..]
                        .iter()
                        .map(|index| maps[*index].name.as_str())
                        .collect();
                    names.push(maps[child].name.as_str());
                    for index in &trail[from..] {
                        circular[*index] = true;
                    }
                    let label = DefLabel::new("map", maps[at].name.clone());
                    self.error(
                        &label,
                        &place.site,
                        format!("a map nests inside itself: {}", names.join(" → ")),
                    );
                    continue;
                }
                self.walk(maps, child, state, depth, circular, trail);
                deepest = deepest.max(depth[child] + 1);
            }
        }
        depth[at] = deepest;
        trail.pop();
        state[at] = DONE;
    }
}

fn spawn_who(creature: &Option<String>, population: &Option<String>) -> SpawnWho {
    match (creature, population) {
        (Some(creature), _) => SpawnWho::Creature(creature.clone()),
        (None, Some(population)) => SpawnWho::Population(population.clone()),
        (None, None) => SpawnWho::Creature(String::new()),
    }
}

/// The turns a nested map may be given.
pub(crate) fn allowed_turns(rule: TurnRule, may_turn: bool) -> Vec<Turn> {
    match rule {
        TurnRule::Fixed(turn) => vec![turn],
        TurnRule::Any if may_turn => Turn::ALL.to_vec(),
        TurnRule::Any => vec![Turn::IDENTITY],
    }
}

/// Width and depth after a turn.
pub(crate) fn turned_size(size: IVec2, turn: Turn) -> IVec2 {
    if turn.rot.quarter_turns() % 2 == 0 {
        size
    } else {
        IVec2::new(size.y, size.x)
    }
}

/// Where a cell of a map of some size lands when the map is turned and
/// moved back so that its north-west corner is at the origin.
pub(crate) fn turned_cell(cell: IVec2, size: IVec2, turn: Turn) -> IVec2 {
    let far = IVec2::new(size.x - 1, size.y - 1);
    let a = turn.apply(IVec2::new(0, 0));
    let b = turn.apply(far);
    let corner = a.component_min(b);
    let moved = turn.apply(cell);
    IVec2::new(moved.x - corner.x, moved.y - corner.y)
}

/// Where the north-west corner of a nested map, as turned, goes when
/// the map is placed at `at`: there, or, for a map that names an
/// anchor, where that puts the anchor on `at`.
pub(crate) fn nested_origin(size: IVec2, anchor: Option<IVec3>, turn: Turn, at: IVec3) -> IVec3 {
    match anchor {
        None => at,
        Some(anchor) => {
            let turned = turned_cell(anchor.xy(), size, turn);
            IVec3::new(at.x - turned.x, at.y - turned.y, at.z - anchor.z)
        }
    }
}

/// Whether a nested map, turned and anchored at a cell, lies wholly
/// inside its parent.
pub(crate) fn fits_at(
    parent: &CompiledMap,
    size: IVec2,
    levels: (i32, i32),
    anchor: Option<IVec3>,
    turn: Turn,
    at: IVec3,
) -> bool {
    let origin = nested_origin(size, anchor, turn, at);
    let turned = turned_size(size, turn);
    let (low, high) = parent.level_range();
    origin.x >= 0
        && origin.y >= 0
        && origin.x + turned.x <= parent.size.x
        && origin.y + turned.y <= parent.size.y
        && origin.z + levels.0 >= low
        && origin.z + levels.1 <= high
}
