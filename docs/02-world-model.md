# 02 — World Model

Status: design complete, ready for implementation.
Implementation plan: [plans/world-model.md](plans/world-model.md).
Serves scenes S1, S2, S3, S7, S8, S9, S10, S11, S12, and the world's share of
S13, S18 and S19, of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how space, matter and place are represented, stored,
loaded, changed and destroyed. It covers the `clash-world` crate and the
generator interface of `clash-gen`.

It does not specify entity internals (06), actions and reactions (03), the EOC
language (05), generation content (10), the cold simulation's logic (09),
vehicles (11), rendering (12), perception and belief (15) or devices (16).

The world model is genre-neutral (01 R10). Examples in this document lean on
cities and explosions because they stress the design hardest; nothing here
assumes a setting, an era or a power level. Where those systems need something from the
world, the interface is defined here.

Code blocks are sketches. Type and field names are normative. Signatures may
be adjusted by the implementing work package, provided this document is
updated in the same change.

Numeric values for materials are illustrative. Real values live in mod data.

## 2. The picture

The **cosmos** holds every **realm**. A realm is one contiguous space with its
own rules: a planet from bedrock to orbit, a pocket dimension, the inside of a
training room where a year passes in a day.

A realm is a grid of **cells**, each 1 m × 1 m and one storey (3 m) tall. A
cell has a **floor**, a **block** and a **fixture**, each made of a
**material**, plus sparse extras: damage, marks, **fields** such as fire and
smoke, and mod variables. Cells are stored in **chunks**. Chunks are grouped
into **tracts**, the 32 m × 32 m columns that are generated, loaded, saved and
coarsely simulated as units. Tracts are grouped into **regions**.

Only tracts that something has asked for are loaded. Requests are **tickets**.
Everything else exists as small records: what kind of tract it is, how
devastated it is, and a list of things that have happened to it and will be
applied when it is next loaded.

Matter changes through one door, `WorldEdit`. Each change records its cause
and produces an event. Destruction is physical: a **damage packet** carries
energy and intensity, a material has a strength and a capacity, and whatever
energy is not absorbed carries on. **Processes** on the shared timeline expand
pressure waves, bring down unsupported structures, and fly **bodies** through
walls.

Pieces of world can move. A **subgrid** is a self-contained grid embedded in a
realm: a bus, a starship, an elevator, the top of a tower after someone tore
it off. **Portals** and **boundary links** connect cells and realms.
**Travelers** cross long distances on coarse maps without loading the ground
beneath them.

### Glossary

| Term | Meaning |
|------|---------|
| Cosmos | All realms, charts and their connections. |
| Realm | One contiguous cell-scale space with its own rules. |
| Grid | A chunked container of cells. Each realm has one base grid; each subgrid is another. |
| Cell | 1 m × 1 m × 3 m. The smallest addressable volume. |
| Element | One of a cell's floor, block or fixture. The thing that takes damage. |
| State | `(definition, material, bits)` of an element. |
| Material | Physical properties shared by terrain, items and body tissue. |
| Chunk | 32 × 32 × 8 cells. Unit of storage and caching. |
| Tract | A 32 × 32-cell column over all z. Unit of generation, loading, saving and coarse simulation. |
| Region | 64 × 64 tracts (2048 m square). Unit of survey and regional simulation. |
| Field | A per-cell quantity that evolves: fire, smoke, water, heat. |
| Process | Scheduled world activity on the timeline. |
| Body | A point mass in flight. |
| Ticket | A request to keep tracts loaded. |
| Stamp | A recorded energy release waiting to be applied to unloaded tracts. |
| Pending edit | Any deferred change applied when a tract is next loaded. |
| Subgrid | A movable grid embedded in a realm. |
| Portal | A connection between two cells. |
| Boundary link | A connection from a realm edge to somewhere else. |
| Area | A named volume with tags, variables and optionally its own clock. |
| Site | A named place with identity, spanning one or more tracts. |
| Plane | An interaction layer; occupants on disjoint planes ignore each other. |
| Chart | A coarse map above cell scale used for travel. |
| Traveler | A group crossing a chart or a realm's tract grid without being simulated cell by cell. |

## 3. Scales and coordinates

### 3.1 Units

| Unit | Size | Type | Used for |
|------|------|------|----------|
| Cell | 1 m × 1 m × 3 m | `CellPos { x: i32, y: i32, z: i32 }` | Addressing |
| Chunk | 32 × 32 × 8 cells | `ChunkPos { x: i32, y: i32, z: i32 }` | Storage, caches, dirty tracking |
| Tract | 32 × 32 cells, all z | `TractPos { x: i32, y: i32 }` | Generation, loading, saving, coarse simulation |
| Region | 64 × 64 tracts | `RegionPos { x: i32, y: i32 }` | Survey, regional simulation |

All sizes are powers of two and defined once in `clash_world::dims`
(`CHUNK_XY_BITS = 5`, `CHUNK_Z_BITS = 3`, `REGION_TRACT_BITS = 6`,
`STOREY_M = 3.0`, `CELL_M = 1.0`). Conversions use arithmetic shifts, which
floor correctly for negative coordinates. A tract's footprint equals a chunk's
footprint by design.

Axes: `+x` east, `+y` south, `+z` up. A cell index inside a chunk is
`CellIdx(u16) = lx | ly << 5 | lz << 10`.

### 3.2 Locations

```rust
pub struct GridId(u32);                      // unique in the cosmos; a realm's base grid or a subgrid
pub struct Location { pub grid: GridId, pub cell: CellPos }   // 16 bytes
```

Every position in the engine is a `Location`. Positions are never bare
coordinates, because the same coordinates mean different places in different
grids. This is fixed from the first line of code so that subgrids (section 18)
do not require a rewrite later.

Things that need more precision than a cell carry it separately:

- An entity at rest has `alt_mm: u16`, its height above the base of its cell
  (0 to 2999). This is the "exact height" used for jumps, launches and flight
  within a storey.
- A body in flight has `pos: [Real; 3]` in metres in its grid's frame, where
  `z_m = cell.z * 3 + altitude`.

### 3.3 Distance

Gameplay distance is physical: metres between cell centres, with z scaled by
`STOREY_M`. Grid stepping uses the 8 horizontal neighbours plus up and down;
a diagonal step costs √2. On wrapped axes, deltas take the shorter way round.

## 4. Cosmos and realms

```rust
pub struct Cosmos {
    realms: BTreeMap<RealmId, Realm>,
    grids: LookupMap<GridId, GridSlot>,        // base grids and subgrids
    charts: BTreeMap<ChartId, Chart>,
    tickets: TicketTable,
    bodies: BodyTable,
    portals: PortalTable,
    vars: VarMap,                              // global variables
}

pub struct Realm {
    pub id: RealmId,
    pub def: RealmDefId,
    pub seed: u64,
    pub clock: ClockId,
    pub base_grid: GridId,
    pub regions: BTreeMap<RegionPos, RegionRecord>,   // only surveyed regions
    pub stamps: Vec<Stamp>,                    // realm-wide stamps (section 17)
    pub areas: AreaTable,
    pub sites: SiteTable,
    pub travelers: TravelerTable,
    pub vars: VarMap,
    pub state: RealmState,                     // Active | Destroyed
}
```

### 4.1 Realm definitions

A realm has no hard-coded "kind". What a realm is follows from its properties.

```json
{
  "type": "realm",
  "id": "earth",
  "size_regions": [512, 256],
  "wrap": ["x"],
  "z_range": [-256, 140000],
  "generator": "earthlike",
  "fill": { "below": "solid@granite", "above": null },
  "strata": [
    { "from_z": -256,  "gravity": 9.81, "atmosphere": "air",      "sky": "day_night" },
    { "from_z": 4000,  "gravity": 9.81, "atmosphere": "thin_air", "sky": "high_altitude" },
    { "from_z": 33000, "gravity": 0.0,  "atmosphere": null,       "sky": "orbit" }
  ],
  "time_rate": 1.0,
  "edges": {
    "top": { "exit": "chart", "chart": "sol", "node": "earth" },
    "bottom": "barrier",
    "north": "barrier",
    "south": "barrier"
  }
}
```

| Property | Meaning |
|----------|---------|
| `size_regions` | Horizontal extent. The example is about 1050 km × 525 km. |
| `wrap` | Axes that wrap around. A planet wraps `x`. A torus wraps both. |
| `z_range` | Lowest and highest storey. Storage is sparse, so a tall range costs nothing. |
| `generator` | Which generator surveys regions and realises tracts (section 9). |
| `fill` | What implicit cells contain below and above the generated range of a tract. |
| `strata` | Ambient conditions by altitude: gravity (m/s², along −z), atmosphere (a definition giving density, pressure, sound speed, breathability; `null` is vacuum), and sky (a presentation hint). |
| `time_rate` | Rate of the realm's clock relative to universal time. |
| `edges` | Per face: `"barrier"` (impassable and indestructible), `"void"` (whatever leaves is removed and an event is emitted), or a boundary link (section 19). Wrapped axes have no edges. |

Strata are why scene S9 needs no realm change between the roof and orbit: one
column of space runs from bedrock through air to vacuum, and conditions change
with height. Zero gravity in orbit is genre physics, not orbital mechanics.

A pocket dimension is a small realm with barrier edges and whatever rules its
author wants:

```json
{
  "type": "realm",
  "id": "training_room",
  "size_regions": [1, 1],
  "wrap": [],
  "z_range": [0, 64],
  "generator": "white_void",
  "strata": [ { "from_z": 0, "gravity": 98.1, "atmosphere": "air" } ],
  "time_rate": 365.0,
  "edges": { "top": "barrier", "bottom": "barrier", "north": "barrier",
             "south": "barrier", "east": "barrier", "west": "barrier" }
}
```

Realms are created when a world is created and may be created and destroyed
at runtime (section 21).

### 4.2 Topology

`Realm::normalize(CellPos) -> Result<CellPos, Edge>` wraps coordinates on
wrapped axes and reports which edge was crossed otherwise. All world access
normalises first. `Realm::delta(a, b)` gives the shortest offset between two
cells, respecting wrap.

## 5. Cells

### 5.1 Layers

| Layer | Holds | Storage | Examples |
|-------|-------|---------|----------|
| floor | The surface at the base of the cell | Paletted | concrete slab, asphalt, grass, carpet |
| block | What occupies the cell's volume | Paletted | wall, solid rock, window, door, tree trunk |
| fixture | A static prop standing in the cell | Paletted | desk, street lamp, rubble pile |
| damage | Accumulated damage per element | Sparse | |
| decals | Surface marks with provenance | Sparse | blood, scorch, footprints |
| vars | Mod variables on the cell | Sparse | |
| fields | Evolving quantities | Sparse, per field | fire, smoke, water, heat |

Floor, block and fixture are the three **elements**. Each is independently
destructible and each has a **state**:

```rust
#[repr(C)]
pub struct ElementState {      // 6 bytes; FloorState, BlockState, FixtureState are typed wrappers
    pub def: u16,              // FloorId / BlockId / FixtureId; 0 = none
    pub material: MaterialId,  // u16; the definition's default unless overridden
    pub bits: u16,             // definition-specific: named state index, amount, facing
}
```

Separating definition from material means any shape can be made of any
material. An ice wall, a wall of gold and a brick wall are one definition with
three materials, and a power that transmutes matter is a material change.

In data, a state is written `"wall"` (default material and state),
`"wall@brick"` (material), `"door#open"` (named state), or `"door@steel#open"`.

### 5.2 Standing, falling and climbing

- A cell is **standable** if it has a floor, or the cell below has a block
  with a solid top (fill ≥ 0.95 and full height).
- A cell that is neither standable nor filled is **open**. Unsupported things
  in open cells fall.
- Natural ground needs no floor layer. Standing on bare rock is standing on
  the solid top of the block below. Roads, slabs and grass are floors laid
  over it. Destroying a floor over solid ground exposes the ground; destroying
  the ground below that makes a hole.
- Ramps, stairs and ladders are elements flagged as vertical connections.

### 5.3 Geometry

Elements carry simple geometry used by movement, cover, sight and physics.

| Property | On | Meaning |
|----------|----|---------|
| `height` | block, fixture | Metres from the cell base, 0 to 3. A 1 m fence can be seen and thrown over. |
| `fill` | block, fixture | Fraction of the cell volume occupied. A wall is about 0.2, solid rock 1.0. |
| `thickness` | floor | Metres. |
| `coverage` | block, fixture | Fraction of the cell's cross-section blocked, for blasts and projectiles. Defaults to `height / 3`. |

Derived per state: `volume` (m³; `fill × 3` for blocks and fixtures,
`thickness × 1` for floors), `mass = volume × density`, and
`capacity = volume × toughness` (section 12).

The world reports geometry. Whether a particular mover can pass an obstacle is
decided by the movement rules in the simulation, which know the mover.

### 5.4 Definitions

**Damage types** are data. The core mod defines at least `impact`, `cut`,
`pierce`, `blast`, `heat`, `cold`, `shock` and `corrode`. Mods add more.

```json
{ "type": "damage_type", "id": "blast",
  "default_resist": { "threshold": "50 kPa", "factor": 1.0 } }
```

**Materials** are shared by terrain, items and body tissue (document 06).

```json
{
  "type": "material",
  "id": "concrete",
  "density": 2400,
  "toughness": 1.0e6,
  "resist": {
    "impact": { "threshold": "20 MPa" },
    "cut":    { "threshold": "200 MPa", "factor": 0.2 },
    "blast":  { "threshold": "100 MPa" },
    "heat":   { "threshold": "50 MPa", "factor": 0.1 }
  },
  "span": 4,
  "flammable": false,
  "rubble": "rubble",
  "color": "gray",
  "tags": ["mineral"]
}
```

| Property | Unit | Meaning |
|----------|------|---------|
| `density` | kg/m³ | Mass of elements and debris. |
| `toughness` | J/m³ | Energy to destroy one cubic metre. |
| `resist.<type>.threshold` | Pa (equivalently J/m³) | Intensity below which this damage type does nothing. |
| `resist.<type>.factor` | — | Multiplier on energy that gets through. Default 1. |
| `span` | cells | How far an element of this material can reach sideways from support (section 14). |
| `rubble` | fixture id | What destroyed elements of this material leave behind. |

**Blocks, floors and fixtures**:

```json
[
  { "type": "block", "id": "solid", "material": "granite",
    "height": 3.0, "fill": 1.0,
    "flags": ["opaque", "load_bearing", "anchor"],
    "glyph": { "ch": " ", "bg": "material" } },

  { "type": "block", "id": "wall", "material": "concrete",
    "height": 3.0, "fill": 0.2,
    "flags": ["opaque", "load_bearing"],
    "resist": { "blast": { "threshold": "70 kPa" } },
    "glyph": { "ch": "#", "fg": "material" } },

  { "type": "block", "id": "door", "material": "wood",
    "height": 3.0, "fill": 0.05,
    "flags": ["opaque"],
    "states": {
      "closed": {},
      "open": { "add_flags": ["passable"], "remove_flags": ["opaque"], "glyph": { "ch": "'" } }
    },
    "glyph": { "ch": "+", "fg": "material" } },

  { "type": "floor", "id": "slab", "material": "concrete", "thickness": 0.2,
    "resist": { "impact": { "threshold": "300 kPa" }, "blast": { "threshold": "50 kPa" } },
    "glyph": { "ch": ".", "fg": "material" } },

  { "type": "fixture", "id": "rubble", "material": "concrete",
    "height": 0.5, "fill": 0.15, "flags": ["rough"], "heap": "rubble_heap",
    "glyph": { "ch": ";", "fg": "material" } }
]
```

A definition's `resist` overrides its material's for that damage type. This is
how a thin wall panel fails under a pressure wave that solid rock of the same
material would ignore. Resolution order: element definition, then material,
then the damage type's default.

`states` lists named variants that adjust flags, geometry and glyph. The
active state's index is stored in `bits`.

`on_destroy` may name what the element becomes. Without it, the element is
removed and its material's `rubble` fixture is left behind.

**Engine flags** (fixed set, stored as a bitset per state):

| Flag | Meaning |
|------|---------|
| `opaque` | Blocks sight. |
| `passable` | Does not block ordinary walking regardless of height. |
| `load_bearing` | Can support blocks and floors above it. |
| `anchor` | Always supported. Natural solid ground. |
| `ramp`, `stairs`, `ladder` | Vertical connection to z+1. |
| `liquid` | The cell is filled with a still liquid. |
| `grate` | A floor that lets fields and small things through. |
| `rough` | Movement penalty. |
| `fragile` | Affected by the light-damage tier of large energy releases (section 13). |

Mods add free-form `tags`, which are queryable but have no engine meaning.

### 5.5 Interactive elements and devices

Most fixtures are inert props. Some can be used: a locker, a lever, a
terminal, a generator, a lamp, an altar. They stay fixtures in the grid, so a
street of ten thousand doors and lockers costs no more than ten thousand
desks, and they gain behaviour only when it is needed.

```json
{ "type": "fixture", "id": "locker", "material": "steel",
  "height": 2.0, "fill": 0.1, "flags": ["opaque"],
  "states": { "closed": {}, "open": { "remove_flags": ["opaque"] } },
  "interactions": ["open", "close", "hide_inside", "search"],
  "device": "locker_device",
  "glyph": { "ch": "]", "fg": "material" } }
```

- `interactions` lists the verbs the element offers to whatever can reach
  it. The world stores and reports them. What a verb does is defined by
  rules (documents 05 and 16).
- `device` names an entity definition. When the element needs more state
  than its 16 bits (something is put inside, someone hides in it, it is
  linked to another device, a rule asks), the simulation creates a **device
  entity** in the element's cell and binds the two. The device holds
  contents, links, variables and timers. When it is back in its default
  state it may be dropped again.
- The element remains the physical object. Destroying it emits
  `ElementDestroyed`, and the simulation deals with the device: spill the
  contents, expose whoever was hiding.
- Blocks and floors can carry `interactions` and `device` in the same way: a
  door, a vault wall, a pressure plate.

Links between devices (power, signal, mechanical) and the rules of
interaction are specified in document 16. The world's obligations are these
definition fields, cell variables, element events, and the `Device` occupant
kind.

## 6. Chunks and grids

### 6.1 Paletted storage

Each element layer in a chunk is a `Paletted<T>`:

```rust
pub enum Paletted<T> {
    Uniform(T),
    Packed { palette: Vec<T>, bits: u8, data: Box<[u64]> },   // bits ∈ {1, 2, 4, 8, 16}
}
```

A chunk of solid rock or empty air is `Uniform` and costs a few bytes. A
typical city chunk needs 4 or 8 bits per cell per layer. Palettes grow on
write and are compacted on save.

### 6.2 Chunks

```rust
pub struct Chunk {
    floors: Paletted<FloorState>,
    blocks: Paletted<BlockState>,
    fixtures: Paletted<FixtureState>,
    damage: SparseCells<[u8; 3]>,        // per element, 0..=254 of 255
    decals: SparseCells<DecalList>,
    vars: SparseCells<VarMap>,
    fields: FieldStore,                  // section 16
    occupants: OccupantIndex,            // section 8; rebuilt on load, never saved
    derived: DerivedCache,               // lazily rebuilt
    rev: ChunkRev,                       // { structure, fields, occupants }: u32 counters
    dirty: bool,                         // differs from what is on disk
}
```

`SparseCells<T>` is a sorted `Vec<(CellIdx, T)>`.

`DerivedCache` holds one bitset (8192 bits) per hot query: `walk_blocked`,
`opaque`, `has_floor`, `solid_top`, `fragile`. They are rebuilt from the
palettes when `rev.structure` changes. Inner loops for movement and sight read
bitsets, not palettes. Per-palette-entry flags are computed once when an entry
is added.

Revision counters are how every cache outside the chunk (paths, field of view,
light, rendering) knows whether it is stale.

### 6.3 Grids

```rust
pub struct Grid {
    columns: LookupMap<TractPos, Column>,
}

pub struct Column {
    z_lo: i32, z_hi: i32,                 // chunk-z range that has storage
    chunks: Vec<Box<Chunk>>,              // indexed by chunk z − z_lo
    below_fill: BlockState,               // implicit content under z_lo
    above_fill: BlockState,               // implicit content over z_hi; usually none
    heightmap: Box<[i32; 1024]>,          // per cell column: highest z that is not open
}
```

Cell lookup hashes the tract and indexes the chunk by z. Reading outside
`z_lo..=z_hi` returns the fill without allocating. Writing there creates the
chunk.

The heightmap is maintained on every structural edit. It answers "is this cell
under open sky", lets falling bodies and pressure waves skip empty air, and
bounds how far down a top-down view must look.

Subgrids use the same `Grid` type with their own small coordinate space.

## 7. Reading and editing

### 7.1 Views

All gameplay-facing reads go through `Cosmos`, never through `Chunk`
directly, so that subgrid compositing (section 18) is transparent to callers.

```rust
impl Cosmos {
    pub fn cell(&self, loc: Location) -> CellView;          // states of all three elements
    pub fn flags(&self, loc: Location) -> CellFlags;        // fast path from derived bitsets
    pub fn standable(&self, loc: Location) -> bool;
    pub fn obstacle_height(&self, loc: Location) -> Real;   // metres
    pub fn field(&self, loc: Location, field: FieldId) -> u8;
    pub fn ambient(&self, loc: Location) -> Ambient;        // gravity, atmosphere, clock
    pub fn is_loaded(&self, loc: Location) -> bool;
}
```

Reading an unloaded cell is an error value, not a panic and not a silent
load. Code that may touch unloaded space must go through the coarse API
(section 17) or hold a ticket.

### 7.2 Edits

```rust
pub struct WorldCtx<'a> {
    pub cosmos: &'a mut Cosmos,
    pub timeline: &'a mut Timeline,
    pub events: &'a mut EventSink<WorldEvent>,
    pub defs: &'a Defs,
}

impl WorldCtx<'_> {
    pub fn edit(&mut self, cause: Cause) -> WorldEdit<'_>;
}

impl WorldEdit<'_> {
    pub fn set_floor(&mut self, loc: Location, s: FloorState) -> Result<(), WorldError>;
    pub fn set_block(&mut self, loc: Location, s: BlockState) -> Result<(), WorldError>;
    pub fn set_fixture(&mut self, loc: Location, s: FixtureState) -> Result<(), WorldError>;
    pub fn damage(&mut self, loc: Location, layer: Layer, p: DamagePacket) -> DamageOutcome;
    pub fn add_field(&mut self, loc: Location, field: FieldId, amount: i16);
    pub fn add_decal(&mut self, loc: Location, decal: Decal);
    pub fn set_var(&mut self, loc: Location, key: VarKey, value: Value);
}
```

`WorldEdit` is the only way cells change (rule R3). Every structural change:

1. updates storage and the heightmap, and bumps the chunk's revision;
2. marks the chunk dirty;
3. queues a stability check if a structural element was removed or weakened
   (section 14);
4. emits a `WorldEvent` carrying old state, new state and cause.

Field value changes are too numerous to report per cell. They bump the field
revision; events are emitted only when a field appears in or vanishes from a
cell, and when it contacts an occupant.

### 7.3 Causes

```rust
pub struct Cause {
    pub actor: Option<EntityId>,        // what physically did it; may be a thrown body
    pub instigator: Option<EntityId>,   // who is responsible
    pub kind: CauseKind,                // Action | Process | Field | Eoc | Generator | Coarse | Debug
}
```

Processes pass the instigator along. When a hero throws a villain through a
bank, the wall's destruction has the villain as actor and the hero as
instigator. Decals store their cause. This is the raw material for collateral
damage tracking, reputation and investigation, which later documents build.

Causes are ground truth. What witnesses believe happened is a separate
matter (01 R11, document 15), and the two can differ.

### 7.4 World events

`ElementChanged`, `ElementDamaged`, `ElementDestroyed`, `FieldAppeared`,
`FieldVanished`, `FieldContact`, `EnergyReleased`, `CollapseStarted`,
`CollapseSettled`, `BodyLaunched`, `BodyImpact`, `BodyHitOccupant`,
`BodyLanded`, `OccupantUnsupported`, `OccupantExposed`, `TractLoaded`,
`TractUnloading`, `TractDevastated`, `RegionDevastated`, `AreaEntered`,
`AreaExited`, `PortalTraversed`, `RealmCreated`, `RealmDestroyed`,
`SubgridMoved`, `SubgridDetached`, `SubgridMerged`.

Each carries the tick, location, cause and type-specific data. The simulation
forwards them to the EOC runtime and the presentation stream.

## 8. Occupancy

### 8.1 Spatial index

The world does not own entities. It owns an index from cells to occupants.

```rust
pub struct Occupant {
    pub id: EntityId,
    pub kind: OccupantKind,      // Creature | Item | Device | Other
    pub plane: PlaneMask,        // u8 bitmask; default bit 0
    pub mass: Real,              // kg, for collisions with bodies
    pub flags: OccupantFlags,    // BLOCKS_MOVE, BLOCKS_SIGHT, COLLIDES
}
```

Each chunk's `OccupantIndex` maps `CellIdx` to a small list. Large occupants
register a box footprint and appear in every cell of it. Anything larger than
a few cells per side should be a subgrid.

Only entities that are in the world appear in the index. An entity inside
another (01 §6) does not: a rider on a mount, a passenger, an item in a pack,
a person in a locker. For the world's purposes it is wherever its outermost
container is.

Queries: at a cell, in a box, within a radius, along a ray, each with an
optional plane mask and kind filter. Results are returned in a defined order
(by cell, then by id).

The index is changed only by `place`, `relocate` and `remove` on `Cosmos`,
called from the simulation's single relocation function. `relocate` computes
area and portal crossings and emits their events.

### 8.2 Planes

Two occupants interact (collide, see, hit) only if their plane masks share a
bit. Terrain exists on all planes. An astral projection, a ghost, or a fight
in a mirror layer over the same streets is a plane change, not a realm change.
Walking through walls is a movement ability, not a plane.

### 8.3 Areas

```rust
pub struct Area {
    pub id: AreaId,
    pub kind: Option<AreaKindId>,    // definition supplying defaults and attached rules
    pub shape: AreaShape,            // Box { min, max } | Cells(sorted Vec<CellPos>)
    pub grid: GridId,
    pub tags: TagSet,
    pub vars: VarMap,
    pub clock: Option<ClockId>,      // things inside run on this clock
    pub site: Option<SiteId>,
}
```

Areas mark volumes that mean something: a vault, a crime scene, an arena, a
no-fly zone, a bubble of stopped time. They are created by generation, by
EOCs and by powers. Lookup is by a per-tract list. Entering and leaving emit
events.

An area may have a kind. An `area_kind` definition supplies default tags,
variables, an optional clock rate and priority, and rules attached to every
area of that kind (05 §9.3).

An area with a clock is how scene S10 works. Occupants, bodies and processes
inside it are re-parented to the area's clock on entry and back on exit.

### 8.4 Sites

```rust
pub struct Site {
    pub id: SiteId,
    pub def: SiteDefId,
    pub name: String,
    pub tracts: Vec<TractPos>,
    pub anchor: CellPos,
    pub vars: VarMap,
    pub tags: TagSet,
}
```

A site gives a place identity that survives being unloaded: a bank, a
headquarters, a lair. The cold simulation (document 09) attaches owners,
occupants and events to sites. The world only stores them and maps tracts to
them.

## 9. Tracts and regions

### 9.1 Records

Everything the engine knows about unloaded space is in records.

```rust
pub struct RegionRecord {
    pub survey: RegionSurvey,                 // biome, elevation, settlement, roads: generator-defined summary
    pub tracts: Box<[TractRecord; 4096]>,
    pub stamps: Vec<Stamp>,                   // region-wide stamps
    pub sites: Vec<SiteId>,
    pub vars: VarMap,
}

pub struct TractRecord {                      // about 16 bytes; a surveyed region is about 64 KB
    pub def: TractDefId,                      // what kind of place this is
    pub rot: u8, pub variant: u8,
    pub state: TractState,                    // Surveyed | Realized { pristine: bool }
    pub devastation: u8,                      // section 17
    pub site: Option<SiteIdx>,
    pub extra: Option<Box<TractExtra>>,
}

pub struct TractExtra {
    pub pending: Vec<PendingEdit>,            // applied in order at next load
    pub stamp_watermark: u64,                 // stamps up to this sequence are already applied
    pub last_loaded: Tick,                    // local time
    pub vars: VarMap,
}
```

### 9.2 Lifecycle

```
Unsurveyed ──survey region──► Surveyed ──realize──► Realized
                                                     │   ▲
                                              unload ▼   │ load
                                                   (on disk)
```

Independently, a realized tract is either **loaded** (hot or warm) or not.

### 9.3 Generators

```rust
pub trait RegionSurveyor {
    fn survey(&self, cx: &GenCtx, realm: &RealmInfo, pos: RegionPos) -> RegionSurveyResult;
}

pub trait TractRealizer {
    fn realize(&self, cx: &GenCtx, realm: &RealmInfo, pos: TractPos,
               rec: &TractRecord, around: &TractRecords3x3) -> RealizedTract;
}

pub struct RealizedTract {
    pub chunks: Vec<(i32, Chunk)>,            // by chunk z
    pub below_fill: BlockState,
    pub above_fill: BlockState,
    pub spawns: Vec<SpawnRequest>,            // handed to the simulation
    pub areas: Vec<AreaSpec>,
    pub portals: Vec<PortalSpec>,
    pub subgrids: Vec<SubgridSpec>,
}
```

Generation is **pure**. `GenCtx` provides definitions and positional
randomness only. A realizer sees its tract's record and the eight neighbouring
*records*, never neighbouring *cells*. Features that span tracts (roads,
large buildings, rivers) are agreed at survey time and written into the
records. Because of this, tracts can be generated in any order and in
parallel, and always come out the same.

Surveying a region assigns a `TractDefId`, rotation and variant to each of its
4096 tracts and creates its sites.

Document 10 specifies the real generators and the JSON map format. The world
model ships two test generators: `flat` (ground and nothing else) and
`test_city` (a street grid with box buildings of configurable height, with
walls, slabs and interior columns), used by every destruction test.

### 9.4 Loading a tract

At a safe point, in this order:

1. Survey its region if needed.
2. If realized and stored: read its chunks. Otherwise: run the realizer, or
   select the ruin variant directly if devastation is total (section 17).
3. Apply stamps newer than the watermark and pending edits, merged in
   sequence order, in resolve mode (section 13.4).
4. Catch up: each field's catch-up rule runs for the local time elapsed since
   `last_loaded`.
5. Emit `TractLoaded`. The simulation instantiates stored entities, cold
   agents located here, and statistical population, and runs its own
   catch-up. Randomness used here is positional, keyed on the tract and the
   load tick.

Loading is a pure function of cosmos state, tract and tick.

### 9.5 Unloading a tract

1. Emit `TractUnloading`. The simulation removes every occupant: persistent
   entities are stored with the tract, important characters become cold
   agents, anonymous population returns to statistics.
2. The world asserts the tract has no occupants, bodies or processes.
3. Dirty chunks are written. `last_loaded` is set.

A tract with a process still running in it is not unloaded; the process holds
a ticket (section 10).

## 10. Activity

### 10.1 Tickets

```rust
pub struct Ticket {
    pub realm: RealmId,
    pub anchor: TicketAnchor,     // Fixed(TractPos) | Entity(EntityId) | Body(BodyId) | Subgrid(SubgridId)
    pub radius: u8,               // tracts, Chebyshev
    pub level: Activity,          // Hot | Warm
    pub priority: u8,             // 255 = pinned
    pub reason: TicketReason,     // Player | Process(ProcessId) | Body | Eoc(EocId) | Debug
    pub expires: Option<Tick>,
}
```

A tract's activity is the highest level of any ticket covering it. A hot
ticket also keeps a warm margin of `WARM_MARGIN = 2` tracts around itself.
Tracts are unloaded once they fall outside every ticket by more than one
tract, which prevents thrashing at the boundary.

Who holds tickets:

| Holder | Ticket |
|--------|--------|
| The controlled character | Hot, radius 6, pinned |
| Other characters the simulation wants at full detail | Hot, small radius |
| A body in flight | Warm, radius 1, placed ahead along its velocity |
| A collapse, blast or other process | Warm, covering its extent, until it settles |
| EOCs and scenarios | Whatever they request |

The world runs physics at full fidelity on every loaded tract. The difference
between hot and warm matters only to the simulation, which throttles AI and
ambient spawning in warm tracts.

### 10.2 Reconciliation

At safe points the cosmos compares the tracts tickets want with the tracts
loaded and loads or unloads the difference. Order is by priority, then
distance to the ticket anchor, then `TractPos`, so it is deterministic.

### 10.3 Budget

`max_loaded_tracts` (default 1600) bounds memory. When tickets want more,
the lowest-priority tickets are refused. A process that loses its ticket is
told to **force-resolve**: finish instantly in resolve mode and record the
result. A body that loses its ticket is resolved coarsely: its landing point
is computed, damage along its path becomes a stamp, and its owner is placed
at the destination as a cold agent or pending spawn.

With defaults, the player's ticket loads 17 × 17 tracts (544 m across). The
budget leaves room for about four more such areas.

## 11. Persistence

- World data is stored per tract behind a trait:

```rust
pub trait TractStore {
    fn get(&self, realm: RealmId, pos: TractPos) -> Result<Option<Vec<u8>>, StoreError>;
    fn put(&mut self, realm: RealmId, pos: TractPos, blob: &[u8]);
    fn delete(&mut self, realm: RealmId, pos: TractPos);
    fn commit(&mut self) -> Result<(), StoreError>;        // atomic
}
```

  The default implementation uses `redb`. Tests use an in-memory map.
- A tract blob holds its column (chunks with compacted palettes, sparse
  layers, fields, fills) and an opaque entity section owned by the simulation.
  It is compressed with LZ4.
- States are stored by **save id**, an index into a save-wide, append-only
  table of string ids. On load, save ids are mapped to current runtime ids.
  A string id that no longer exists goes through migration entries and
  otherwise becomes a placeholder that keeps the original string, so
  re-adding the mod restores the content.
- **Pristine policy.** A realized tract that has never been changed is marked
  pristine and is not stored; it is regenerated on load. The record keeps a
  fingerprint of the generator inputs. If the fingerprint no longer matches
  (mods changed), the tract regenerates with the new content. Setting
  `save.store_pristine = true` stores everything instead.
- Region records, realm state and cosmos state are stored in the main save
  alongside simulation state.
- The codec is versioned per blob.

Rule R7 (save/load is invisible) is tested for every world scenario,
including saves taken in the middle of a collapse and a blast.

## 12. Damage

### 12.1 Packets

```rust
pub struct DamagePacket {
    pub kind: DamageTypeId,
    pub energy: Real,        // J delivered
    pub intensity: Real,     // J/m³, numerically Pa: how concentrated the delivery is
    pub cause: Cause,
}

pub struct DamageOutcome {
    pub result: DamageResult,    // Unaffected | Damaged | Destroyed
    pub absorbed: Real,          // J taken by the element
    pub passed: Real,            // J that continues beyond it
}
```

Every source of harm in the engine, from a jab to a planet-cracking beam,
arrives as packets. The same packets are applied to body tissue by the body
model (document 06).

Two numbers are needed because energy alone cannot distinguish a thousand
taps from one hammer blow. **Intensity** decides whether the target is
affected at all. **Energy** decides how much.

### 12.2 Applying a packet to an element

```
resist    = definition override, else material, else damage type default
gain      = clamp(intensity / resist.threshold − 1, 0, 1)
effective = energy × resist.factor × gain
capacity  = volume × material.toughness
remaining = capacity × (1 − damage / 255)

if effective ≥ remaining:
    Destroyed.  passed = energy × (1 − remaining / effective)
elif effective > 0:
    Damaged.    damage += ceil(255 × effective / capacity);  passed = 0
else:
    Unaffected. passed = 0
```

Below threshold nothing happens, however often it is repeated. Full effect is
reached at twice the threshold. Accumulated damage is stored in 255ths.

Worked example with the illustrative values above. A wall cell holds
0.6 m³ of concrete, so its capacity is 0.6 MJ. An 80 kg person thrown into it
at 20 m/s carries 16 kJ, delivered at about 0.3 MPa: below the 20 MPa impact
threshold, so the wall is unharmed and the person absorbs all of it. The same
person at 600 m/s carries 14.4 MJ at about 290 MPa: the wall is destroyed,
0.6 MJ is spent, and 13.8 MJ carries on to the next wall. That is scene S2.

Callers compute intensity. Conventions:

| Source | Intensity |
|--------|-----------|
| Strike with a body part or weapon | Force over contact area (document 07) |
| Kinetic impact of a body | `KE / (area × depth)`, section 15 |
| Pressure wave | Energy density at that distance, section 13 |
| Field contact | Per field definition, section 16 |

### 12.3 Destruction

When an element is destroyed:

1. It is replaced by its `on_destroy` state, or removed.
2. Its material's rubble fixture is added to the cell (or the nearest
   standable cell below). Rubble amounts add up; a full cell of rubble becomes
   the fixture's `heap` block.
3. If the packet passed energy on and the debris budget allows, part of the
   element's mass is launched as debris bodies along the direction of the hit
   (section 15). Debris does secondary damage.
4. `ElementDestroyed` is emitted, and a stability check is queued.

## 13. Energy releases

```rust
pub struct EnergyRelease {
    pub origin: Location,
    pub origin_alt_mm: u16,
    pub energy: Real,                 // J
    pub kind: DamageTypeId,
    pub shape: ReleaseShape,          // Sphere | Cone { dir, half_angle } | Beam { dir, radius, range }
    pub speed: Real,                  // m/s of the front; defaults to the local sound speed
    pub cause: Cause,
}

impl WorldCtx<'_> {
    pub fn release(&mut self, r: EnergyRelease);
}
```

`release` is the single entry point for explosions, shockwaves from clashes,
energy blasts and beams at every scale. It decides, per tract, whether to act
in detail or coarsely.

### 13.1 Intensity law

For a spherical release, intensity at distance `r` is the released energy
spread over the volume it has swept:

```
I(r) = E / (4/3 · π · r³)        for r ≥ 0.5 m
```

A cone divides by its fraction of the full sphere. This is the strong-shock
scaling, and it makes radius grow with the cube root of energy, which matches
real explosions across many orders of magnitude and keeps every scale in one
formula:

| Release | Energy | Radius at 70 kPa (walls fail) |
|---------|--------|-------------------------------|
| Hand grenade | 8 × 10⁵ J | 1.4 m |
| Car bomb | 2 × 10⁹ J | 19 m |
| One kiloton | 4 × 10¹² J | 240 m |
| One megaton | 4 × 10¹⁵ J | 2.4 km |
| Continent-scarring strike | 10²³ J | 700 km |
| Planet's binding energy | 2 × 10³² J | Everything, everywhere |

The release reaches out to `R`, where intensity falls to
`world.min_intensity` (default 5 kPa).

### 13.2 Detailed mode

Used for loaded tracts.

- The front expands on the timeline at `speed`. The release is a process that
  wakes once per shell. Something faster than the front can leave before it
  arrives (scene S3). The process runs on the clock of its origin.
- Cells are handled in order of distance from the origin.
- **Shadowing.** The process keeps a transmission map over directions (a cube
  map of bins around the origin, so no trigonometry is needed). A cell
  receives `I(r) × T(direction)`. After a cell's elements are resolved, the
  bins they cover are multiplied by what got through: an element that
  survives blocks its `coverage`; an element that is destroyed passes the
  unabsorbed fraction. A wall strong enough to survive shelters what is
  behind it. A wall that fails does not.
- Each element in a reached cell receives one packet:
  `intensity = I(r) × T`, `energy = intensity × 3 m³ × coverage`.
- Each occupant in a reached cell gets an `OccupantExposed` event with the
  same numbers. The simulation turns that into injury and knockback.
- Three tiers keep large releases affordable:

| Tier | Where | Work |
|------|-------|------|
| Annihilation | Intensity exceeds every loaded material's threshold by a wide margin | Whole chunks are cleared in bulk |
| Structural | Above the weakest structural threshold | Every non-empty cell, with shadowing |
| Light | Down to `min_intensity` | Only occupants and `fragile` elements, each with a line-of-sight check |

- Empty space is skipped using chunk uniformity and column heightmaps.
- A beam marches along its axis. In each slab it destroys what its intensity
  (`E / (π · radius² · 1 m)`) exceeds, subtracting what that costs from `E`.
  It ends when `E` is spent or `range` is reached, and may release whatever
  remains as a sphere.

Contract, whatever the implementation details:

1. Total energy absorbed never exceeds energy released.
2. An occupant fully behind an element that survives receives no more than
   that element's leakage.
3. Results are identical across runs and independent of chunk iteration
   order.
4. A release whose work exceeds `release.max_cells` falls back to resolve
   mode for its outer shells.

### 13.3 Coarse mode

Tracts within `R` that are not loaded receive a stamp (section 17).

### 13.4 Resolve mode

Resolve mode applies a release to a whole tract instantly. Intensity comes
from distance alone, with a flat shelter factor for cells that have something
overhead. Elements are damaged and destroyed by the same packet rule. No
debris bodies fly; rubble is placed directly. Collapse runs to a fixed point
without the timeline (section 14.4). One `TractDevastated` event is emitted.

Resolve mode is used when loading a tract with stamps, when a release exceeds
its budget, and when a process is force-resolved.

## 14. Structural support and collapse

The model is deliberately simple: no stress analysis, no load paths. It
answers one question, "is this element still held up?", cheaply and
deterministically, and lets falling debris do the rest.

### 14.1 Support rule

Structural elements are blocks and floors. Evaluate one storey at a time,
bottom up. Within a storey `z`:

- **Sources** (distance 0):
  - a block flagged `anchor`;
  - a block standing on a supported `load_bearing` block at `z − 1`, or on
    implicit solid fill;
  - a floor resting on a supported `load_bearing` block at `z − 1`.
- **Propagation.** Support spreads between a floor and a block in the same
  cell, between floors in horizontally adjacent cells, and between blocks in
  horizontally adjacent cells. Each step adds 1 to the distance. An element
  accepts support only if the resulting distance is at most its `span`.
- Elements in the storey that receive no support **fall**.

So a slab reaches `span` cells from the walls or columns under it, a wall
stands on the wall below it or on a supported slab, and knocking out a column
drops whatever was relying on it alone.

### 14.2 When it runs

Removing or damaging a structural element queues a stability check for its
location. Checks within `SETTLE_DELAY` (50 ms local) are merged, so one blast
is one evaluation. The check examines elements at or above the change that
are structurally connected to it, within a bounded box. Elements outside the
box are assumed supported.

### 14.3 Falling

- Falling elements in the same vertical column are merged into one debris
  mass per contiguous run.
- The mass falls on the timeline (`t = sqrt(2h / g)` under local gravity),
  on the clock of where it is. Things fast enough can get out from under it,
  or get someone else out. Inside stopped time it hangs.
- On landing it delivers an impact packet with energy `m · g · h` to the
  floor or solid top it lands on, and exposes occupants there.
- If the floor fails, the debris continues with the remaining energy, plus
  the floor's own mass, and a new stability check is queued. Progressive
  floor-by-floor collapse is not scripted; it happens when the numbers say so.
- Where it comes to rest it becomes rubble.
- Occupants whose floor fell get `OccupantUnsupported`.

The collapse is a process. It holds a ticket over its extent and emits
`CollapseStarted` and `CollapseSettled`.

### 14.4 Resolve mode

The same evaluation is run repeatedly to a fixed point with landings computed
directly, capped at a fixed number of passes. Past the cap, everything above
the lowest unsupported storey in the affected footprint becomes rubble.

## 15. Bodies

A **body** is anything in uncontrolled flight: a character who has just been
hit very hard, a thrown car, shrapnel, a falling mass, a detached subgrid.

```rust
pub struct Body {
    pub owner: BodyOwner,        // Entity(EntityId) | Debris(DebrisSpec) | Subgrid(SubgridId)
    pub grid: GridId,
    pub pos: [Real; 3],          // m
    pub vel: [Real; 3],          // m/s
    pub mass: Real,              // kg
    pub area: Real,              // m², cross-section
    pub drag: Real,              // drag coefficient
    pub depth: Real,             // m, stopping distance for impact intensity; default 0.1
    pub restitution: Real,       // 0..1
    pub cause: Cause,
}

impl WorldCtx<'_> {
    pub fn launch(&mut self, body: Body) -> BodyId;
    pub fn impulse(&mut self, id: BodyId, j: [Real; 3]);
    pub fn arrest(&mut self, id: BodyId) -> Body;     // caught, or recovered control
}
```

Bodies are point masses with a cross-section. There is no rotation.

### 15.1 Motion

Each body is a process on the clock of where it is. It advances from cell
boundary to cell boundary, with a maximum step of 20 ms local so that gravity
and drag integrate accurately:

```
a   = gravity(strata) − ½ · ρ_air · drag · area · |v| · v / mass
v  += a · dt
pos += v · dt
```

Terminal velocity, vacuum and zero gravity all follow from the strata. A body
carries a small ticket ahead of itself. Runs of implicit empty cells may be
integrated in one step if the result is identical.

Self-propelled flight is not a body. A flying character moves by actions.
They become a body when something takes control away from them, and stop
being one when they recover or are caught.

### 15.2 Impacts

When a body would enter a cell containing an obstacle (an element taller than
the body's altitude, a floor when moving vertically, a colliding occupant, a
subgrid cell):

```
KE        = ½ · mass · v_n²                 (v_n: speed along the direction of entry)
intensity = KE / (area × depth)
outcome   = damage(element, impact, KE, intensity)

Destroyed:  body keeps going; its KE along that axis becomes outcome.passed
otherwise:  v_n ← −restitution · v_n; below REST_SPEED it stops
```

`BodyImpact` reports the energy the body lost and the intensity. If the owner
is an entity, the simulation applies that to its body model. Whether the
thrown person or the wall gives way is decided by the same rule on both
sides: an ordinary person is destroyed by an impact a wall ignores, and a
sufficiently tough one goes through the wall unharmed.

A colliding occupant is struck with conservation of momentum, emits
`BodyHitOccupant`, and may become a body itself.

### 15.3 Landing

A body that hits a standable surface and cannot break it loses its vertical
velocity (times restitution). Remaining horizontal velocity decays by
friction as it slides, still colliding with obstacles. When it stops,
`BodyLanded` is emitted and the body is removed.

## 16. Fields

A field is a per-cell value from 0 to 255 that evolves by rule.

```json
{
  "type": "field",
  "id": "fire",
  "step": "200 ms",
  "rules": [
    { "rule": "decay", "amount": 2 },
    { "rule": "spread", "needs": "fuel", "chance": 0.3 },
    { "rule": "emit", "field": "smoke", "amount": 20 },
    { "rule": "emit", "field": "heat", "amount": 40 },
    { "rule": "damage", "kind": "heat", "energy_per_level": 2000, "intensity_per_level": "10 kPa" }
  ],
  "catch_up": "burn_out",
  "glyph": { "ch": "^", "fg": ["yellow", "orange", "red"] },
  "tags": ["light_source"]
}
```

Built-in rules, implemented natively and configured by data:

| Rule | Behaviour |
|------|-----------|
| `decay` | Lose value per step. |
| `diffuse` | Even out with neighbours. Optional vertical bias for gases that rise or sink. |
| `flow` | Liquid behaviour: fall, then spread sideways. |
| `spread` | Ignite neighbours that have fuel (flammable materials in elements, fixtures or items). |
| `emit` | Add to another field in the same cell. |
| `convert` | React with another field: water and fire make steam. |
| `damage` | Send packets to elements in the cell and exposure events to occupants. |

Anything more exotic subscribes an EOC to `FieldContact` or `FieldAppeared`.

Fields cannot enter a cell whose block has `coverage` of 0.95 or more. They
pass between storeys only where there is no floor or the floor is a `grate`.

Storage is sparse per chunk and per field, with a list of **active** cells.
Only active cells are stepped. A chunk with no active cells costs nothing.

Stepping is a process per realm and step interval. Each step reads the state
at the start of the step and writes all changes at the end, so the result
does not depend on the order cells are visited. Random choices use positional
randomness keyed on field, cell and step number.

`catch_up` names a closed-form rule applied when a tract is loaded after time
has passed: fires burn out, gases disperse, liquids settle.

`field.max_active_cells` (default 200,000) bounds cost. Past it, the weakest
cells decay faster and a diagnostic event is emitted.

Still bodies of liquid (lakes, seas) are blocks flagged `liquid`, not fields.
When an edit opens space beside one, cells along the opening are converted to
the corresponding field so the liquid can move, within the same budget.

## 17. The coarse layer

This is what lets destruction happen where nothing is loaded: far from the
player (scene S7) and across whole continents (scene S8).

### 17.1 Stamps

```rust
pub struct Stamp {
    pub release: EnergyRelease,
    pub tick: Tick,
    pub seq: u64,              // cosmos-wide, increasing
}
```

When `release` covers unloaded tracts, it records one stamp at the coarsest
level that contains the unloaded part of its footprint:

| Footprint | Stored in |
|-----------|-----------|
| Up to 64 tracts | Each tract's `pending` list |
| Up to 16 regions | Each region's `stamps` |
| Larger | The realm's `stamps` |

A planet-wide event is therefore one record, not millions.

Stamps are pulled down lazily. When a tract is loaded it gathers the stamps
from its own list, its region and its realm that are newer than its
watermark and reach it, merges them with its other pending edits by
sequence, applies them in resolve mode, and advances its watermark. Region
and realm stamps are kept permanently. They are small, and they are history.

### 17.2 Devastation

Each tract has a devastation value from 0 to 255, derived from the highest
intensity any stamp or detailed release has delivered to it:

```
devastation = clamp(255 · log10(I / 5 kPa) / log10(100 MPa / 5 kPa), 0, 255)
```

It is cached in the record, shown on region maps, read by the cold simulation
to estimate casualties and reactions, and read by generation.

`Cosmos::devastation_at(realm, tract)` evaluates region and realm stamps on
demand for tracts that have never been surveyed.

When a tract that has never been realized has total devastation, loading it
skips normal generation and stamp application and asks the generator for the
tract definition's ruin variant, or the realm's wasteland definition.

### 17.3 Other pending edits

```rust
pub enum PendingEdit {
    Stamp(Stamp),
    Cells(Vec<CellEdit>),                   // explicit small changes
    Mapgen { update: MapgenUpdateId, at: CellPos, params: VarMap },
    Spawn(SpawnRequest),
    Eoc { id: EocId, context: VarMap },     // run when the tract loads
}
```

The cold simulation and EOCs use these to change places nobody is looking at:
board up a shop, leave a body in an alley, put a crater where the fight was.

### 17.4 Events for the cold simulation

A release that reaches unloaded space emits `RegionDevastated` for each
affected surveyed region (capped), or a single realm-wide event beyond the
cap. Document 09 specifies what the cold simulation does with them.

## 18. Subgrids

A subgrid is a `Grid` with its own coordinates, placed in a parent grid.

```rust
pub struct Subgrid {
    pub id: SubgridId,
    pub grid: GridId,              // its own cells
    pub parent: GridId,            // a realm's base grid or another subgrid
    pub origin: CellPos,           // where local (0,0,0) sits in the parent
    pub rot: Rot4,                 // 0, 90, 180, 270 degrees about z
    pub mass: Real,                // maintained from its elements
    pub body: Option<BodyId>,      // present while in free flight
    pub vars: VarMap,
}
```

The transform between parent and local coordinates is a translation and a
quarter-turn, so it is exact integer arithmetic.

One abstraction serves vehicles with interiors, starships, elevators, moving
platforms, floating islands, creatures large enough to stand on, and pieces
of the world that have been torn loose.

### 18.1 Compositing

- Each parent chunk keeps a list of subgrids overlapping it.
- A read of a parent cell first checks overlapping subgrids. A non-empty
  subgrid cell **shadows** the parent cell.
- Occupants standing in a subgrid have locations in the subgrid's `GridId`.
  When the subgrid moves, they move with it and nothing about them needs
  updating. This is what makes fighting on a moving train or inside a ship
  work.
- Queries that cross the boundary (rays, sight, blasts, bodies) transform
  coordinates as they go.
- Subgrids may nest to a depth of four: a car on a ferry.

### 18.2 Movement

A subgrid moves in whole-cell steps, timed by its speed. For each step, every
non-empty subgrid cell whose destination is obstructed forms a contact pair.
The subgrid's kinetic energy is shared across the pairs; each pair exchanges
impact packets, with the subgrid's element and the world's element both
taking damage by the usual rule. If every obstruction is destroyed the step
completes with the energy spent deducted. Otherwise the subgrid stops, or
carries on without the cells it lost. Occupants in contact cells are exposed.

A subgrid in free flight (thrown, or falling) is driven by a body.

Turning is in quarter-turns. How vehicles steer within that constraint is a
question for document 11.

### 18.3 Detach and merge

```rust
impl WorldCtx<'_> {
    pub fn detach(&mut self, grid: GridId, cells: &CellSet, cause: Cause) -> SubgridId;
    pub fn merge(&mut self, id: SubgridId, cause: Cause);
}
```

`detach` moves a connected set of cells, with their damage, fields and
occupants, out of a grid and into a new subgrid in the same place. `merge`
writes a subgrid's cells back into its parent where it currently sits; cells
that collide with existing matter resolve as impacts.

Scene S11 is `detach`, `launch`, and `merge` on landing.

### 18.4 Interiors that do not fit

Something bigger inside than outside is not a subgrid interior. It is a
separate realm reached by a portal.

## 19. Portals and boundary links

```rust
pub struct Portal {
    pub id: PortalId,
    pub a: PortalEnd,                 // { loc: Location, facing: Dir }
    pub b: PortalEnd,
    pub two_way: bool,
    pub see_through: bool,
    pub filter: PortalFilter,         // plane mask, required tags
    pub vars: VarMap,
}
```

Moving into a portal cell moves the occupant, body or subgrid to the other
end. Velocity is rotated by the difference in facing. `PortalTraversed` is
emitted so EOCs can react or impose conditions.

A **boundary link** is an `edges` entry that sends whatever crosses a realm
edge somewhere else:

| Exit | Result |
|------|--------|
| `{ "exit": "realm", "realm": …, "map": { "scale": [n, d], "offset": [x, y, z] } }` | Arrive in another realm at mapped coordinates. A hollow world under bedrock, a sky realm over the clouds. |
| `{ "exit": "entry", "realm": …, "entry": … }` | Arrive at a named entry point. |
| `{ "exit": "chart", "chart": …, "node": … }` | Leave cell scale and become a traveler on a chart (section 20). |

**Realm transfer** is one function that moves an occupant, body or subgrid
between grids, updates clocks and tickets, and emits events. Portals,
boundary links, teleport effects and travel all use it.

## 20. Travel

Loading every tract under a character flying across a continent at Mach 5
would be absurd. Long-distance movement happens on coarse maps.

### 20.1 Travelers

```rust
pub struct Traveler {
    pub id: TravelerId,
    pub members: Vec<EntityId>,          // stored cold while travelling
    pub carrier: Option<RealmId>,        // a loaded interior that stays playable
    pub position: ChartPos,              // on a realm's tract grid, or on a chart
    pub route: Route,
    pub speed: Real,
    pub band: TravelBand,                // Ground | Air | Orbit; who can see and intercept it
}
```

A traveler on a realm's tract grid moves through tract coordinates in coarse
time steps without realizing anything beneath it. On arrival, or when the
cold simulation decides it has been intercepted, its members are placed at
the corresponding location with a ticket.

### 20.2 Carrier realms

When the thing travelling is a subgrid with people inside who should keep
playing, the subgrid is transferred into a small temporary **carrier realm**
(empty space, appropriate sky, the ship's own gravity) and the traveler
refers to it. The interior stays hot and on the shared timeline throughout.
On arrival the subgrid is transferred into the destination realm and the
carrier is destroyed. This is the second half of scene S9.

When travelers meet (an interception, a boarding action, a battle between
ships), their subgrids and members are transferred into one shared realm
created for the encounter, so the meeting plays out at cell scale.

### 20.3 Charts

A chart is a data-defined map above cell scale: a star system, a galaxy, a
web of dimensions.

```json
{
  "type": "chart",
  "id": "sol",
  "unit": "1e9 m",
  "nodes": [
    { "id": "earth", "pos": [150, 0], "realm": "earth", "arrive": { "edge": "top" } },
    { "id": "luna",  "pos": [150.4, 0], "realm": "luna", "arrive": { "edge": "top" } }
  ],
  "parent": { "chart": "local_stars", "node": "sol" }
}
```

Travel time between nodes is distance over speed, or follows explicit lanes
with conditions. Charts nest. A chart of dimensions is the same structure
with different conditions on its lanes.

This document fixes the data model and the hand-off between cells and charts.
Document 11 specifies travel rules, encounters and vehicles.

## 21. Dimensions and realm lifecycle

Three mechanisms cover what fiction calls dimensions:

| Fiction | Mechanism |
|---------|-----------|
| Another world, a pocket space, a training room | A separate realm, reached by portal, boundary link or chart |
| The same place on another layer (astral, ghostly) | Planes (section 8.2) |
| A twisted copy of this world | A **derived realm** |

A derived realm uses a generator that takes another realm as input:

```json
{ "generator": { "type": "derived", "source": "earth",
                 "mode": "pristine", "transform": "upside_down" } }
```

`pristine` reproduces the source's generated tracts, ignoring changes since.
`live` copies the source tract's current cells the first time each tract is
realized. `transform` is a mapgen transform (document 10) applied on top:
swap palettes, add growths, remove people.

**Creation.** `create_realm(def, seed, params) -> RealmId` at any safe point.

**Destruction.** `destroy_realm(id, policy)` marks the realm destroyed, emits
`RealmDestroyed`, and applies the policy to everything loaded inside it:
transfer to a named entry elsewhere, or remove. Its storage is deleted at the
next save. A planet being destroyed is an EOC reacting to a sufficiently
large release by destroying the realm and creating a debris-field realm in
its place on the chart.

## 22. Queries

The world answers geometric questions. Perception, AI and movement rules,
which interpret the answers, are specified in later documents.

```rust
impl Cosmos {
    pub fn raycast(&self, from: PointM, dir: [Real; 3], max_m: Real, filter: RayFilter) -> RayHits;
    pub fn line_of_sight(&self, a: PointM, b: PointM, planes: PlaneMask) -> bool;
    pub fn field_of_view(&self, origin: Location, eye_alt_mm: u16, radius_m: Real, out: &mut FovSet);
    pub fn neighbours(&self, loc: Location) -> NeighbourGeometry;   // raw data for movement rules
}
```

- Rays step through 1 × 1 × 3 m cells and respect element heights: a ray at
  1.5 m passes over a 1 m fence.
- Field of view is symmetric shadowcasting per storey, extended downward
  through cells with no floor and upward through open sky.
- All queries composite subgrids, respect wrap, and optionally continue
  through see-through portals.
- Results that are cached are keyed on chunk revisions.

Path-finding is specified in document 09. The world's obligations are the
`neighbours` query, revision counters, and change events precise enough to
update a hierarchical path graph incrementally.

## 23. Modding surface

**Definition types**: `material`, `damage_type`, `atmosphere`, `floor`,
`block`, `fixture`, `field`, `realm`, `area_kind`, `chart`, `site`, `tract`
(document 10).
Elements may declare `interactions` and a `device` (section 5.5).

**Events** an EOC can trigger on: every world event in section 7.4.

**Effects** an EOC can perform (bound to the functions in this document):

| Effect | Function |
|--------|----------|
| Set or transmute an element | `set_floor`, `set_block`, `set_fixture` |
| Damage a cell | `damage` |
| Explosion, shockwave, beam | `release` |
| Add or remove a field | `add_field` |
| Mark a cell | `add_decal`, `set_var` |
| Throw, push, catch | `launch`, `impulse`, `arrest` |
| Tear loose, set down | `detach`, `merge` |
| Stamp a map fragment | mapgen update at a location, live or pending |
| Open or close a portal | portal table |
| Create or destroy a realm | `create_realm`, `destroy_realm` |
| Create an area; set its clock rate | area table, clock |
| Keep somewhere loaded | ticket |
| Change somewhere unloaded | `PendingEdit` |
| Teleport or transfer | realm transfer |

**Queries** available to expressions: element definition, material, state,
flags and tags at a location; field value; damage; standable; obstacle
height; ambient gravity and atmosphere; distance; line of sight; area and
site membership; tract devastation; variables at cell, area, site, tract,
region, realm and global scope.

Nothing in this document is reachable only from Rust.

## 24. Invariants

| # | Invariant |
|---|-----------|
| W1 | Cells change only through `WorldEdit`. The occupant index changes only through `place`, `relocate`, `remove`. |
| W2 | Generation is pure: a function of definitions, realm seed, position and records. |
| W3 | Loading a tract is a pure function of cosmos state, tract and tick. |
| W4 | Every occupant, body and process is in a loaded tract. |
| W5 | A process holds a ticket for the tracts it touches, or has been resolved. |
| W6 | No result depends on hash iteration order, chunk load order or thread count. |
| W7 | Save, load and continue equals continue. |
| W8 | Energy absorbed by a release or impact never exceeds energy delivered. |
| W9 | No definition id appears in engine code. Fills, rubble and placeholders are named by data. |
| W10 | Reading unloaded space returns an error value. It never loads and never panics. |

## 25. Budgets

Targets on one simulation thread of an ordinary desktop CPU. They are
unmeasured. The benchmark work package establishes real numbers, after which
this table is revised and enforced.

| Operation | Target |
|-----------|--------|
| Cell flag read | under 10 ns |
| Cell state read | under 30 ns |
| Single element edit, including bookkeeping | under 300 ns |
| Chunk memory, typical city | under 12 KB |
| Chunk memory, uniform | under 200 bytes |
| Realize a `test_city` tract | under 2 ms |
| Load a stored tract | under 1 ms |
| Default loaded set (289 tracts, city) | under 100 MB |
| Stability check, 30 × 30 footprint, 20 storeys | under 3 ms |
| Detailed release reaching 100,000 non-empty cells | under 30 ms in total |
| Body step | under 500 ns |
| Field step, 50,000 active cells | under 5 ms |
| Save with 50 dirty tracts | under 50 ms |

## 26. Open questions

These do not block implementation. Each names the document expected to
settle it.

1. **Crush failure.** The support model has no load limit: a column carries
   any weight. Add a compressive load check per column if collapses look
   wrong without it. (This document, after the first collapse scenarios run.)
2. **Cave-ins.** Natural ground is `anchor` and never collapses. Decide
   whether tunnelling should be able to bring down what is above. (10)
3. **Turning subgrids.** Quarter-turns are exact. Decide whether vehicles
   need eighth-turns and how. (11)
4. **Very large creatures.** Decide whether a creature tens of metres tall is
   a multi-cell occupant or a subgrid with a body plan mapped onto its cells.
   (06, 11)
5. **Sight through portals and across subgrid boundaries** has a cost; decide
   how far it is followed. (12)
6. **Rewinding time.** Determinism makes restoring a snapshot of an area
   possible in principle. Not designed. (03)
7. **Scale of battle.** Scene S19 puts hundreds of active characters in a few
   loaded tracts. Occupancy queries and tickets are designed for that; the
   entity and AI budgets that make it affordable are set elsewhere. (06, 09)

Settled since first draft: there is no default planet. World size is a
property of each realm definition (section 4.1), chosen by whoever writes
the world. Reference mods use small realms.
