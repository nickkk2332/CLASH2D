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

Positions order storey by storey, row by row: by `z`, then `y`, then `x`.
That is the order of a chunk's cell array, and the order of every result
that lists cells. Tracts and regions order row by row: by `y`, then `x`.

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
  `z_m = cell.z * 3 + altitude`. A point in metres with its grid is a
  `PointM { grid, pos }`. A point exactly on a boundary belongs to the cell
  on its positive side.

### 3.3 Distance

Gameplay distance is physical: metres between cell centres, with z scaled by
`STOREY_M`. Grid stepping uses the 8 horizontal neighbours plus up and down;
a diagonal step costs √2. On wrapped axes, deltas take the shorter way round.

Walks over cells have a defined order (`clash_world::iter`): a box storey by
storey and row by row; the eight neighbours clockwise from north, then up,
then down; the shell of radius `r`, which is the cells whose distance from
the centre is at least `r` and less than `r + 1` metres. A straight segment
visits the cell that holds its start, then every cell it runs through for
some length, then the cell that holds its end; where it passes exactly
through an edge or a corner it steps straight to the cell diagonally across.

## 4. Cosmos and realms

```rust
pub struct Cosmos {
    realms: BTreeMap<RealmId, Realm>,
    grids: Vec<Option<Box<GridSlot>>>,         // base grids and subgrids, indexed by GridId
    charts: BTreeMap<ChartId, Chart>,
    tickets: TicketTable,
    bodies: BodyTable,
    portals: PortalTable,
    areas: AreaTable,                          // every area, of every grid (section 8.3)
    sites: SiteTable,                          // every site, of every realm (section 8.4)
    occupants: LookupMap<EntityId, OccupantRecord>,   // where each occupant is (section 8.1); not state
    stability: StabilityQueue,                 // checks asked for and not yet made (section 14.2)
    activity: WorldActivity,                   // the world's processes and the collapses in progress (section 14.3)
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
    pub travelers: TravelerTable,
    pub vars: VarMap,
    pub plan: RealmPlan,                       // computed once when the realm is made (10 §9.2)
    pub state: RealmState,                     // Active | Destroyed
}
```

Areas and sites are tables of the cosmos, not of each realm: an area
names its grid and a site its realm. Reading a cell's ambient conditions
must find the areas that cover it without first finding its realm, and an
area that moves with its anchor can follow it into another grid.

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
    { "from_z": 4000,  "gravity": 9.81, "atmosphere": "thin_air", "sky": "high_altitude", "temp_offset": -40 },
    { "from_z": 33000, "gravity": 0.0,  "atmosphere": null,       "sky": "orbit", "temp": "-120 C" }
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
| `sea_level` | Optional. The storey up to which ocean tracts are filled with still liquid (11 §4.3). |
| `calendar` | Optional. The length of the day and year and the names of their parts (09 §14.2). |
| `wrap` | Axes that wrap around. A planet wraps `x`. A torus wraps both. |
| `z_range` | Lowest and highest storey. Storage is sparse, so a tall range costs nothing. |
| `generator` | Which world generator plans the realm, surveys regions and realises tracts (section 9; 10 §9). |
| `fill` | What implicit cells contain below and above the generated range of a tract. |
| `strata` | Ambient conditions by altitude: gravity (m/s², along −z), atmosphere (a definition giving density, pressure, sound speed, breathability; `null` is vacuum), sky (a presentation hint), and temperature: `temp_offset` added to the region's, or `temp` replacing it (11 §4.6). |
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

What a definition must say, and what follows when it does not:

- `size_regions`, `z_range` and `strata` are required. A realm needs at
  least one stratum, no two may start at the same storey, and a storey
  below the lowest stratum takes the lowest.
- A face not named in `edges` is a barrier. Naming a face of a wrapped
  axis is a load error.
- `fill` defaults to nothing below and nothing above. `time_rate` defaults
  to 1 and must be zero or more.
- `generator`, `calendar`, `sky` and boundary links are stored as written
  until the packages that give them meaning are built.

`Cosmos::create_realm(timeline, def, seed)` makes the realm, a clock for it
under universal time at `time_rate`, and its empty base grid.

### 4.2 Topology

A grid's shape is a `Topology`: its extent along `x` and `y`, each
unbounded, bounded or wrapped, and its lowest and highest storey. A realm's
cells run from 0 up to its size on each horizontal axis. A subgrid's own
space is unbounded.

`Topology::normalize(CellPos) -> Result<CellPos, Edge>` wraps coordinates on
wrapped axes and reports which edge was crossed otherwise. A position
beyond several edges reports the first of west, east, north, south, bottom,
top. All world access normalises first. `Topology::delta(a, b)` gives the
shortest offset between two cells, respecting wrap, and
`delta(a, b) = −delta(b, a)` always: exactly half way round a wrapped axis,
the lower coordinate goes forward and the higher goes back.
`Cosmos::normalize` and `Cosmos::delta` do the same for `Location`s.

Grid ids are never reused, and the cosmos keeps its grids in a table
indexed by id, so that reading a cell costs no lookup for its grid.

## 5. Cells

### 5.1 Layers

| Layer | Holds | Storage | Examples |
|-------|-------|---------|----------|
| floor | The surface at the base of the cell | Paletted | concrete slab, asphalt, grass, carpet |
| block | What occupies the cell's volume | Paletted | wall, solid rock, window, door, tree trunk |
| fixture | A static prop standing in the cell | Paletted | desk, street lamp, rubble pile |
| conduits | Things that carry resources through the cell (16 §6) | Sparse | cable, pipe, duct, belt, ley line |
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
The bits may also be given as a number, `"rubble#3"`, for a definition that
uses them as an amount. `"none"` is no element.

For a definition that declares named `states`, `bits` is the index of the
active one, and bits past the last state mean the first. For any other
definition the bits are its own. Every form of "no element" is the one
value `NONE`: definition 0, material 0, bits 0. Material 0, like
definition 0 of each element type, is the built-in `engine:none`.

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
  Each joins its cell to the cell directly above it. Whoever is in that
  upper cell stands at the top of the connection, which holds them up as
  a floor would: they are not unsupported, though the cell has no floor
  and is not standable in the sense above. `Cosmos::footing` says what
  holds up someone in a cell (section 22).

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
  "bearing": "30 MPa",
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
| `bearing` | Pa | The stress it can carry in compression before it is crushed (section 14.5). Absent means it is never crushed. |
| `rubble` | fixture id | What destroyed elements of this material leave behind. May be given per damage type, and may be a field: ice destroyed by heat leaves water. |
| `changes` | — | What it becomes by temperature. See below. |

**Change of state.** A material may say what it becomes when the
temperature of its cell (11 §4.6) passes a threshold.

```json
[
  { "type": "material", "id": "ice",
    "changes": [ { "above": "0 C", "to": { "field": "water", "amount": 200 } } ] },
  { "type": "material", "id": "water",
    "changes": [ { "below": "-2 C", "cover": "ice_sheet" } ] },
  { "type": "material", "id": "basalt",
    "changes": [ { "above": "1100 C", "to": { "block": "liquid@lava" } } ] }
]
```

| Outcome | Does |
|---------|------|
| `to` a material | The element stays, made of the other material |
| `to` a floor, block or fixture state | Replaces the element |
| `to` a field | Removes the element and adds the field |
| `cover` | Lays a floor on top and leaves the element as it is. The floor goes when the condition stops holding. A frozen lake is water with a sheet of ice on it. |

Nothing polls temperature. A change is tested at three moments only, and
made through `WorldEdit`:

- when a field that warms or cools (section 16) changes level in the cell;
- when the region's temperature crosses a threshold that some loaded
  material uses. Region temperature is a known function of time
  (09 §14.1), so the crossing is one scheduled wake-up. It is followed by
  a sweep, within the edit budget, of the elements of that material that
  are open to the weather;
- when a tract loads, for elements open to the weather.

Fields change state in the same way (section 16). A material and what it
turns into should not share a threshold; the linter warns when they do.

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
      "open": { "add_flags": ["passable"], "remove_flags": ["opaque"], "coverage": 0.0, "glyph": { "ch": "'" } }
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
active state's index is stored in `bits`. The first state written is the
default. A state that changes `height` without giving a `coverage` takes
the default coverage for its new height, unless the definition fixed one.

A block or fixture must give `material`, `height` and `fill`; a floor must
give `material` and `thickness`. Heights and thicknesses run from 0 to 3 m,
and `fill` and `coverage` from 0 to 1; a value outside its range is a load
error. A material's `resist` and an element's are keyed by damage type id.

`on_destroy` may name what the element becomes. Without it, the element is
removed and its material's `rubble` fixture is left behind.

`contact` gives an element a striking surface of its own, for whatever
walks into it, lands on it or is thrown against it (07 §6.5):

```json
{ "type": "fixture", "id": "spikes", "material": "iron", "height": 0.5, "fill": 0.05,
  "contact": { "surface": { "kind": "pierce", "area": 3.0e-5, "give": 0.003, "span": 0.02 },
               "energy": 60 } }
```

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
definition fields, the conduit layer, cell variables, element events, and
the `Device` occupant kind.

An element may also declare a passive `load` on a resource and what it does
`when_supplied` (16 §4.3). The world stores these and gives them no
behaviour.

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

The representation is private to the type, so that a layer can never hold
an index its palette lacks. A palette that fills up at 8 bits or wider is
first cleared of entries no cell uses, and widened only if that frees less
than a quarter of it. Compaction is canonical: palette entries end up in
the order their values first appear, cell by cell, and a layer whose cells
all hold one value becomes `Uniform`.

Two layers that hold the same values are equal, serialise alike and hash
equal, whatever order the values were written in. The hash is over the runs
of equal values in cell order, so it does not see the palette at all.

### 6.2 Chunks

```rust
pub struct Chunk {
    floors: Paletted<FloorState>,
    blocks: Paletted<BlockState>,
    fixtures: Paletted<FixtureState>,
    damage: SparseCells<[u8; 3]>,        // per element, 0..=254 of 255
    conduits: SparseCells<ConduitSet>,   // 16 §6
    decals: SparseCells<DecalList>,
    vars: SparseCells<VarMap>,
    fields: FieldStore,                  // section 16
    occupants: OccupantIndex,            // section 8; rebuilt on load, never saved
    derived: DerivedCache,               // kept up to date by every structural write
    rev: ChunkRev,                       // { structure, fields, occupants }: u32 counters
    dirty: bool,                         // differs from what is on disk
}
```

`SparseCells<T>` is a sorted `Vec<(CellIdx, T)>`. The four sparse layers are
kept together behind one pointer that is null when all are empty, as it is
for most chunks.

`DerivedCache` holds the answers to the hot queries for every cell: five
flags, kept as the bits of one byte per cell (`CellFlags`), so that reading
all of a cell's flags is one load. A chunk whose three element layers are
uniform keeps a single value instead and costs nothing.

| Flag | Set when |
|------|----------|
| `walk_blocked` | The block or fixture stands in the way of ordinary walking: it is taller than a step (`WALK_STEP_M`, 0.5 m) and is not `passable`, a `ramp`, `stairs` or `liquid` |
| `opaque` | The block or fixture is `opaque` |
| `has_floor` | The cell has a floor |
| `solid_top` | The block can be stood on from the cell above: full height, fill of at least 0.95, not `liquid` |
| `fragile` | Any of the three elements is `fragile` |

`walk_blocked` is a first answer for inner loops. The movement rules, which
know the mover, read the heights themselves (section 5.3).

The cache is kept up to date by every structural write, which looks up the
contribution of each of the cell's three states in the per-state tables
built with the definitions. It is never rebuilt lazily: reads take the
chunk by shared reference and may run in parallel. It is rebuilt whole only
when a chunk is compacted. Inner loops for movement and sight read the
cache, not palettes.

Revision counters are how every cache outside the chunk (paths, field of view,
light, rendering) knows whether it is stale. A write that changes nothing
bumps nothing. Revisions, caches and the dirty flag are not state: they are
not hashed and not saved. A cache keeps the revisions it was read from in a
`RevStamp` (section 22).

### 6.3 Grids

```rust
pub struct Grid {
    columns: LookupMap<TractPos, Column>,
}

pub struct Column {
    z_lo: i32,                            // chunk z of chunks[0]; the stored range is z_lo .. z_lo + chunks.len()
    chunks: Vec<Chunk>,                   // indexed by chunk z − z_lo
    below_fill: BlockState,               // implicit content under the stored range
    above_fill: BlockState,               // implicit content over it; usually none
    heightmap: Box<[i32; 1024]>,          // per cell column: highest z that is not open
}
```

Cell lookup hashes the tract and indexes the chunk by z. Reading outside
the stored range returns the fill without allocating. Writing there creates
the chunk, and any between it and the stored range, so that the range stays
unbroken. A column with nothing stored still knows where its two fills
meet.

The heightmap is maintained on every structural edit. It answers "is this cell
under open sky", lets falling bodies and pressure waves skip empty air, and
bounds how far down a top-down view must look. Its entry for a cell column
is the highest storey that is not open: the highest that holds a floor, a
block or a fixture, or that can be stood in because the block below has a
solid top. It is `HEIGHT_NONE` where every cell is open and `HEIGHT_FULL`
where the fill above is not empty. A cell at or above its column's entry is
under open sky.

A column hashes by what its cells hold, not by how much of it happens to be
stored: a stored chunk that holds exactly the fill hashes like an unstored
one.

A chunk or column that is not yet part of the world is built with a
`ColumnBuilder`: start from the fills, set cells, and `build`. Generation
and tests make cells this way. Once a column is part of a grid its cells
change only through `WorldEdit`.

Subgrids use the same `Grid` type with their own small coordinate space.

## 7. Reading and editing

### 7.1 Views

All gameplay-facing reads go through `Cosmos`, never through `Chunk`
directly, so that subgrid compositing (section 18) is transparent to callers.

```rust
impl Cosmos {
    pub fn cell(&self, loc: Location) -> Result<CellView, WorldError>;      // states of all three elements
    pub fn flags(&self, loc: Location) -> Result<CellFlags, WorldError>;    // fast path from the derived cache
    pub fn standable(&self, loc: Location) -> Result<bool, WorldError>;
    pub fn obstacle_height(&self, loc: Location) -> Result<Real, WorldError>;   // metres
    pub fn field(&self, loc: Location, field: FieldId) -> Result<u8, WorldError>;
    pub fn ambient(&self, loc: Location) -> Result<Ambient, WorldError>;    // gravity, atmosphere, clock
    pub fn is_loaded(&self, loc: Location) -> bool;
    pub fn grid(&self, grid: GridId) -> Result<GridView<'_>, WorldError>;   // the same reads for many cells of one grid
}
```

Reading an unloaded cell is an error value, not a panic and not a silent
load. Code that may touch unloaded space must go through the coarse API
(section 17) or hold a ticket.

The error values are `WorldError::NotLoaded` with the normalised location,
`OutsideRealm` with the location and the edge it lies beyond, and
`UnknownGrid`. `ambient` needs the cell to be inside its grid and does not
need its tract loaded. `obstacle_height` is the height of the tallest block
or fixture in the cell that is not `passable`. Also read through `Cosmos`
and `GridView`: `is_open`, `height` (the heightmap), `under_open_sky`,
`damage`, `decals`, `var`, `element` (the state on one layer) and
`areas_at`. A `GridView` is a read-only handle on one
grid for code that reads many of its cells. `ambient` gives what the
stratum says with what the areas covering the cell change (section 8.3).

### 7.2 Edits

```rust
pub struct WorldCtx<'a> {
    pub cosmos: &'a mut Cosmos,                // which holds the definitions it was made from
    pub timeline: &'a mut Timeline,
    pub events: &'a mut EventSink<WorldEvent>,
}

impl WorldCtx<'_> {
    pub fn edit(&mut self, cause: Cause) -> WorldEdit<'_>;
}

impl WorldEdit<'_> {
    // Each returns what was there before.
    pub fn set_floor(&mut self, loc: Location, s: FloorState) -> Result<FloorState, WorldError>;
    pub fn set_block(&mut self, loc: Location, s: BlockState) -> Result<BlockState, WorldError>;
    pub fn set_fixture(&mut self, loc: Location, s: FixtureState) -> Result<FixtureState, WorldError>;
    pub fn set_element(&mut self, loc: Location, layer: Layer, s: ElementState) -> Result<ElementState, WorldError>;
    pub fn set_material(&mut self, loc: Location, layer: Layer, m: MaterialId) -> Result<ElementState, WorldError>;
    pub fn set_damage(&mut self, loc: Location, layer: Layer, damage: u8) -> Result<u8, WorldError>;
    pub fn damage(&mut self, loc: Location, layer: Layer, p: DamagePacket) -> Result<DamageOutcome, WorldError>;   // section 12
    pub fn damage_along(&mut self, loc: Location, layer: Layer, p: DamagePacket, dir: Option<[Real; 3]>) -> Result<DamageOutcome, WorldError>;
    pub fn add_field(&mut self, loc: Location, field: FieldId, amount: i16);                       // section 16
    pub fn lay_conduit(&mut self, loc: Location, c: ConduitId) -> Result<(), WorldError>;         // 16 §6
    pub fn cut_conduit(&mut self, loc: Location, c: ConduitId) -> Result<(), WorldError>;
    pub fn add_decal(&mut self, loc: Location, kind: DecalId, amount: u8) -> Result<(), WorldError>;
    pub fn remove_decals(&mut self, loc: Location, kind: DecalId) -> Result<usize, WorldError>;
    pub fn set_var(&mut self, loc: Location, key: VarKey, value: Value) -> Result<Option<Value>, WorldError>;
    pub fn clear_var(&mut self, loc: Location, key: VarKey) -> Result<Option<Value>, WorldError>;
}
```

`WorldEdit` is the only way cells change (rule R3). Every structural change:

1. updates storage and the heightmap, and bumps the chunk's revision;
2. marks the chunk dirty;
3. queues a stability check if a structural element was removed or weakened
   (section 14);
4. emits a `WorldEvent` carrying old state, new state and cause.

An edit that changes nothing does none of this: it emits nothing and bumps
nothing. Every edit normalises its location first and is refused, changing
nothing, for a cell outside its grid, a cell that is not loaded, or a cell
further from the stored cells of its tract than a column will stretch
(`COLUMN_MAX_CHUNKS`, about 780 km of height): `WorldError::OutOfReach`.
An edit is never refused because someone is standing there (section 23).

- **A replaced element starts undamaged.** Setting a floor, block or
  fixture clears the accumulated damage of that layer. `set_material`
  transmutes: the element keeps its definition, its state and its damage.
- **`set_damage`** writes an element's accumulated damage, in 255ths and
  at most 254, and emits `ElementDamaged`. It is what `damage` and
  destruction are built on (section 12); where there is no element it
  does nothing.
- **Removed or weakened** means, for a block or floor: it was removed,
  replaced by another definition or made of another material; or it
  changed state and with that stopped being `load_bearing` or an `anchor`,
  or lost volume; or it took more damage. Putting an element where there
  was none asks for no check, and nor does a door opening.
- **The queue** holds one request for each cell, the first made, with its
  tick and cause, in order of location. Edits only add to it;
  `Cosmos::take_stability_requests` hands the requests to the collapse
  process (section 14.2). It is state: it is hashed and saved. A request
  that is new also sees to it that the check will be made: a wake-up of
  the world's waits out the settle delay on the clock of that cell.
- **Footing.** An edit that takes away what an occupant stood on (the
  floor of its cell, or the solid top or the ramp, stairs or ladder of
  the block or fixture below it) raises `OccupantUnsupported` for each
  occupant standing there that `Cosmos::footing` now finds nothing
  under. An occupant that fills several cells is held up while any cell
  of its lowest storey is. The occupant is not moved: falling is the
  simulation's business.
- **Marks and variables** are not structure. `add_decal` stamps the mark
  with the edit's cause and the tick. Each emits its own event when it
  changes something.

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

`ElementChanged`, `ElementDamaged`, `ElementDestroyed`, `DecalAdded`,
`DecalsRemoved`, `CellVarChanged`, `ConduitChanged`,
`FieldAppeared`,
`FieldVanished`, `FieldContact`, `EnergyReleased`, `CollapseStarted`,
`CollapseSettled`, `BodyLaunched`, `BodyImpact`, `BodyHitOccupant`,
`BodyLanded`, `BodyLeftLoaded`, `OccupantUnsupported`, `OccupantExposed`,
`OccupantDosed`, `OccupantShaken`,
`TremorStarted`, `TremorEnded`, `StateChanged` (an element or field
changed state by temperature), `OccupantDisplaced`, `OccupantEntombed`,
`LeftRealm` (something crossed a `void` edge and is gone),
`FieldBudgetExceeded`, `TicketRefused`, `AreaBreached`, `AreaClosed`, `TractLoaded`,
`TractUnloading`, `TractDevastated`, `RegionDevastated`, `AreaEntered`,
`AreaExited`, `PortalTraversed`, `RealmCreated`, `RealmDestroyed`,
`SubgridMoved`, `SubgridDetached`, `SubgridMerged`.

Each carries the tick, location, cause and type-specific data. The simulation
forwards them to the EOC runtime and the presentation stream.

`WorldEvent` is an enum, and the world puts events into an
`EventSink<WorldEvent>`: a list in the order they were emitted, which
whoever owns it drains. Rules name an event in snake case
(`element_changed`). The fields of those built so far, besides `tick` and
`cause`:

| Event | Fields | Emitted when |
|-------|--------|--------------|
| `ElementChanged` | `loc`, `layer`, `old`, `new` (states) | A floor, block or fixture was set, replaced, transmuted or removed |
| `ElementDamaged` | `loc`, `layer`, `state`, `old`, `new` (255ths) | An element's accumulated damage changed and the element did not |
| `DecalAdded` | `loc`, `decal` (kind, amount, cause, tick) | A mark was left |
| `DecalsRemoved` | `loc`, `kind`, `count` | Every mark of a kind was removed from a cell that had some |
| `CellVarChanged` | `loc`, `key`, `old`, `new` (each a value or nothing) | A cell variable was set, changed or cleared |
| `AreaEntered`, `AreaExited` | `area`, `occupant`, `loc` (where the occupant is), `clock` (the area's, if it has one) | An occupant came to be inside an area or stopped being (section 8.3) |
| `AreaBreached` | `area`, `leak` (a cell) | An enclosed area no longer closes |
| `AreaClosed` | `area`, `loc` (where it was enclosed from) | A breached area closes again |
| `OccupantDisplaced` | `occupant`, `from`, `to` | An edit left an occupant no room and it was moved (section 23) |
| `OccupantEntombed` | `occupant`, `loc` | An edit left an occupant no room and there was nowhere to go |
| `ElementDestroyed` | `loc`, `layer`, `old`, `new` (states; `new` is the `on_destroy` state or nothing), `how` (`damage` with its type, `crushed` or `fell`), `absorbed`, `passed` (J; zero unless by damage) | A floor, block or fixture was destroyed (section 12.3). It follows the `ElementChanged` events of the change and of the rubble left |
| `OccupantUnsupported` | `occupant`, `loc` (where it is) | What held an occupant up has gone (section 7.2) |
| `OccupantExposed` | `occupant`, `loc` (the cell struck), `kind` (damage type), `energy` (J), `intensity` (Pa) | Something struck the cell an occupant is in: so far, a mass that landed there (section 14.3) |
| `CollapseStarted` | `collapse` (id), `loc` (the cell whose change started it) | A stability check found something giving way where no collapse was in progress |
| `CollapseSettled` | `collapse`, `loc`, `bounds` (the box of cells it touched), `tallies` (by instigator: `elements` destroyed, `mass` brought down in kg, occupants `exposed`) | Nothing of a collapse is in flight and no check is waiting near it |

Events that sum many changes (`EnergyReleased`, `CollapseSettled`,
`TractDevastated`) carry totals by instigator: elements destroyed, mass
brought down, occupants exposed. Rules that keep account of what someone
has broken listen to these, not to every element (13 §8.1).

## 8. Occupancy

### 8.1 Spatial index

The world does not own entities. It owns an index from cells to occupants.

```rust
pub struct Occupant {
    pub id: EntityId,
    pub kind: OccupantKind,      // Creature | Item | Device | Other
    pub plane: PlaneMask,        // u32 bitmask, a bit for each plane; default bit 0
    pub mass: Real,              // kg, for collisions with bodies
    pub flags: OccupantFlags,    // BLOCKS_MOVE, BLOCKS_SIGHT, COLLIDES
}

pub struct Footprint { pub x: u8, pub y: u8, pub z: u8 }   // cells east, south and up; each at least 1

pub struct OccupantRecord {      // what Cosmos::occupant(id) gives
    pub occupant: Occupant,
    pub loc: Location,           // the footprint's corner with the lowest coordinates, normalised
    pub footprint: Footprint,
    pub areas: Vec<AreaId>,      // the areas it is inside, in id order
}
```

Each chunk's `OccupantIndex` maps `CellIdx` to a small list. Large occupants
register a box footprint and appear in every cell of it. Anything larger than
a few cells per side should be a subgrid: a footprint is at most
`FOOTPRINT_MAX_SIDE` (16) cells along a side.

The index is a list of `(cell, occupant)` pairs kept in order of cell and
then id, each with the occupant's kind, planes and flags, so that a query
filters without looking anything up. What it holds never depends on the
order things arrived in. An occupant standing or flying where its tract has
no stored chunk is indexed beside the chunks and needs no storage; the
index moves into the chunk if one is created. The cosmos keeps each
occupant's record by id. None of this is state: it is not hashed and not
saved, and the simulation places its entities again after a load.

Only entities that are in the world appear in the index. An entity inside
another (01 §6) does not: a rider on a mount, a passenger, an item in a pack,
a person in a locker. For the world's purposes it is wherever its outermost
container is.

Queries: at a cell, in a box, within a radius, along a ray, each with an
optional plane mask and kind filter. Results are returned in a defined order
(by cell, then by id).

```rust
impl Cosmos {
    pub fn occupant(&self, id: EntityId) -> Option<&OccupantRecord>;
    pub fn occupants_at(&self, loc: Location, filter: &OccupantFilter) -> Result<Vec<Occupied>, WorldError>;
    pub fn is_occupied(&self, loc: Location, filter: &OccupantFilter) -> Result<bool, WorldError>;   // allocates nothing
    pub fn occupants_in_box(&self, grid: GridId, cells: CellBox, filter: &OccupantFilter) -> Result<Vec<Occupied>, WorldError>;
    pub fn occupants_within(&self, centre: Location, radius_m: Real, filter: &OccupantFilter) -> Result<Vec<Occupied>, WorldError>;
    pub fn occupants_along(&self, from: PointM, to: [Real; 3], filter: &OccupantFilter) -> Result<Vec<Occupied>, WorldError>;
}
```

- A result lists pairs: an occupant that fills several cells is listed
  once for each of them the query covers. `distinct_ids` reduces a result
  to one entry for each occupant, in the order they first appear.
- A box and a radius answer by cell (storey by storey, row by row), then
  by id. A radius is in metres between cell centres, a storey counting
  three, the shorter way round. Along a line the cells are those of the
  segment walk of section 3.3, in the order the line reaches them, and
  occupants within a cell by id.
- An `OccupantFilter` asks for a plane mask (occupants sharing a bit with
  it; with none, every occupant whatever its planes), a set of kinds,
  flags an occupant must all have, and one occupant to leave out, usually
  the one asking.
- Every occupant is in loaded space (W4), so a query needs nothing
  loaded: a cell that is not loaded or lies outside the realm holds no
  one. Only an unknown grid is an error. A box over more than
  `QUERY_MAX_TRACTS` (4096) tracts, or a line through more than
  `QUERY_MAX_LINE_CELLS` (65,536) cells, is refused.

The index is changed only by `place`, `relocate` and `remove`, called from
the simulation's single relocation function. They are functions of
`WorldCtx`, because they emit events and stamp them with the tick:

```rust
impl WorldCtx<'_> {
    pub fn place(&mut self, occupant: Occupant, at: Location, footprint: Footprint, cause: Cause) -> Result<(), WorldError>;
    pub fn relocate(&mut self, id: EntityId, to: Location, cause: Cause) -> Result<Location, WorldError>;   // returns where it was
    pub fn relocate_as(&mut self, id: EntityId, to: Location, footprint: Footprint, cause: Cause) -> Result<Location, WorldError>;
    pub fn remove(&mut self, id: EntityId, cause: Cause) -> Result<OccupantRecord, WorldError>;
    pub fn update_occupant(&mut self, occupant: Occupant) -> Result<(), WorldError>;   // kind, planes, mass, flags; not position
}
```

Every cell of the footprint must be inside the grid and loaded, or the call
is refused and nothing changes. `relocate` to another grid is the same
call. `relocate_as` also changes the footprint, for something that turns,
grows or lies down. A move to where the occupant already is does nothing.
`relocate` computes area crossings and emits their events (section 8.3);
portal crossings join it with portals (section 19).

### 8.2 Planes

Two occupants interact (collide, see, hit) only if their plane masks share a
bit. Terrain exists on all planes. An astral projection, a ghost, or a fight
in a mirror layer over the same streets is a plane change, not a realm change.
Walking through walls is a movement ability, not a plane.

Planes are named by definitions (`{ "type": "plane", "id": "astral" }`;
at most thirty-two). `set_plane { who, planes }` sets which an occupant
is on, and `who.planes` reads it.

A plane's id is the number of its bit in a `PlaneMask`. Plane 0 is built
in, `engine:default`: what everything is on unless it says otherwise. It
counts among the thirty-two, so data may define thirty-one, which are
numbered in order of id. A plane definition may carry `tags` and nothing
else. An occupant whose mask is empty interacts with nothing.

### 8.3 Areas

```rust
pub struct Area {
    pub id: AreaId,                  // unique in the cosmos, never reused
    pub kind: Option<AreaKindId>,    // definition supplying defaults and attached rules
    pub grid: GridId,
    pub shape: AreaShape,            // Box(CellBox) | Cells(sorted Vec<CellPos>) | Radius { centre, radius }
    pub anchor: Option<AreaAnchor>,  // an entity or subgrid it moves with
    pub ambient: Option<AmbientOverride>,   // gravity inside it, as a scale or a vector; drift (11 §4.7)
    pub journal: Option<Dur>,        // keep cell changes made inside it for this long (08 §7.4)
    pub tags: Vec<String>,           // sorted
    pub vars: VarMap,
    pub clock: Option<ClockId>,      // things inside run on this clock
    pub site: Option<SiteId>,
    pub priority: i32,               // which applies where areas overlap: the highest
    pub enclosure: Option<Enclosure>,       // how it was made, if by enclosing: { from, limit, breached, leak }
}
```

Areas mark volumes that mean something: a vault, a crime scene, an arena, a
no-fly zone, a bubble of stopped time. They are created by generation, by
EOCs and by powers. Lookup is by a per-tract list. Entering and leaving emit
events.

A shape is in its grid's coordinates. A box may reach across a wrapped
edge. A radius is in metres from the centre of a cell, a storey counting
three, the shorter way round. An area may touch at most `AREA_MAX_TRACTS`
(4096) tracts and list at most `AREA_MAX_CELLS` (65,536) cells: something
larger is a realm, or a change to a realm's clock.

```rust
impl WorldCtx<'_> {
    pub fn create_area(&mut self, spec: AreaSpec, cause: Cause) -> Result<AreaId, WorldError>;
    pub fn remove_area(&mut self, id: AreaId, cause: Cause) -> Result<Area, WorldError>;
    pub fn move_area(&mut self, id: AreaId, by: IVec3, cause: Cause) -> Result<(), WorldError>;
    pub fn set_area_shape(&mut self, id: AreaId, shape: AreaShape, cause: Cause) -> Result<(), WorldError>;
    pub fn set_area_clock_rate(&mut self, id: AreaId, rate: Option<Real>) -> Result<(), WorldError>;
    pub fn enclose(&mut self, from: Location, kind: Option<AreaKindId>, limit: u32, cause: Cause) -> Result<AreaId, WorldError>;
}
```

An `AreaSpec` gives the grid and the shape, and optionally a kind, an
anchor, an ambient override, a journal span, tags, variables, a clock rate,
a priority and a site. Tags, variables and the override are changed in
place through `Cosmos` (`set_area_tag`, `set_area_var`, `set_area_ambient`,
`set_area_priority`, `set_area_anchor`). A change of gravity that should
weigh on what stands in the area is made with
`WorldCtx::set_area_ambient(id, ambient, cause)`, which also asks for
the stability check of section 14.5. `Cosmos::areas_at` lists the areas
that cover a cell and `Cosmos::occupants_in_area` who is inside one.

**Inside.** An occupant is inside an area when any cell of its footprint
is. `AreaEntered` and `AreaExited` are emitted when that changes, whichever
of the two moved: the occupant, by `place`, `relocate` or `remove`; or the
area, by being made, removed, moved or reshaped. When an occupant moves,
the areas it left are reported first and then those it entered, each in id
order. When an area changes, the occupants it left are reported first and
then those it came over, each in id order. Each event carries the area's
clock.

An area may have a kind. An `area_kind` definition supplies default tags,
variables, an optional clock rate and priority, and rules attached to every
area of that kind (05 §9.3). A kind flagged `sealed` gives its areas a
sealed volume with compartments and pressure (11 §10).

| Field of `area_kind` | Meaning |
|----------------------|---------|
| `tags` | Tags every area of the kind carries. The area's own are added. |
| `vars` | Variables every area of the kind starts with: each a boolean, a number or a text. The area's own are laid over them. |
| `clock_rate` | The rate of a clock of the area's own, relative to its realm's. Absent: no clock. |
| `priority` | Which area applies where several overlap. Default 0. |
| `sealed` | The area is a sealed volume (11 §10). The world stores the flag. |
| `leak` | How fast a sealed area's held temperature moves toward the outside (11 §4.6). Stored as written. |
| `ambience` | How it looks inside (12 §6.4). Stored as written. |
| `rules` | Rule ids or inline rules (05 §9.3). The world carries them as written for the rule runtime. |

An area with a clock is how scene S10 works. Occupants, bodies and processes
inside it are re-parented to the area's clock on entry and back on exit.
The world makes the clock, under its realm's, and says which clock applies:
`Cosmos::ambient` gives it for a cell, and `AreaEntered` and `AreaExited`
carry it. Whoever schedules for an occupant moves its wake-ups across.
Removing an area destroys its clock, and whatever was still waiting on it
goes back to the clock above.

An **anchored** area moves with its anchor. When it moves, what it now
covers and no longer covers is worked out, and the same enter and exit
events are raised as when an occupant moves. Auras, carried fields and
bubbles around a person are anchored areas (08 §7.2). An area anchored to
an entity moves when `relocate` moves that entity, by as much, and into
another grid if the entity goes there; the entity itself neither leaves
nor enters it. An area that cannot follow, because it would leave the
grid, stays where it is. An entity that is not an occupant is moved for
with `move_area`. An anchor that is a subgrid is stored until subgrids are
built (section 18).

An area with a **journal** keeps, for the stated span of local time, every
cell change made inside it together with the state before the change. They
can be undone in reverse order (08 §7.4). An area without one keeps nothing.
The span is stored on the area; the journal itself is built with 08 §7.4.

**Enclosing.** `enclose { from, kind, limit }` makes an area out of a
closed space. It fills outward from a cell through open cells, stopped by
whatever stops a field (section 16): blocks that cover the cell, and
floors. If the space closes within `limit` cells, an area of that kind is
created over it. If not, it is refused, and the refusal names a cell
through which it leaks.

- The fill steps north, east, south, west, up and down, in that order,
  breadth first. It does not enter a cell whose block has a `coverage` of
  0.95 or more (`FIELD_BLOCK_COVERAGE`), and it passes between two storeys
  only where the upper cell has no floor or a floor that is a `grate`.
  Fixtures and occupants do not stop it. A barrier edge of the realm
  closes a space like a wall.
- The leak named is the cell from which the fill would cross a `void` edge
  or a boundary link; or else the first cell under open sky that the fill
  reached; or else the first cell past the limit.
- Also refused: a starting cell that is itself covered (there is no space
  there), a fill that reaches a cell that is not loaded (there is no
  telling), and a `limit` above `AREA_MAX_CELLS`.

An area made this way remembers how it was made. When an element inside
it or on its edge changes, its shape is worked out again. If it no longer
closes, it keeps its last shape and is marked `breached`, and a sealed
kind treats the gap as any opening (11 §10.2). A room built by hand
becomes a room when someone encloses it, and stops holding its air when
the wall is knocked through.

- Only a change that matters to the fill counts: a block starting or
  ceasing to cover its cell, or a floor starting or ceasing to stop what
  passes it. A cell is on an area's edge when it is beside, above or below
  a cell of the area.
- The fill starts again from the cell it first started from, or, if that
  cell has been walled up, from the first cell of the area that is still
  open. Occupants who are no longer inside, or newly are, get the usual
  events.
- When it first fails to close, `AreaBreached` is emitted with the leak.
  When it closes again the mark is cleared and `AreaClosed` is emitted. A
  change that only opens a cell of an area that is already breached is
  not looked at: it cannot close it.
- If part of the space is not loaded when a change is made, the area is
  left as it was.

An area's **ambient override** replaces the stratum's gravity inside it.
It may also set how the medium inside it is moving (11 §4.7): a steady
flow, or a vortex.
`Cosmos::ambient` consults areas before strata. Where areas overlap, the
one with the highest priority applies.

```rust
pub struct AmbientOverride {
    pub gravity: Option<GravityOverride>,   // Scale(Real) | Vector([Real; 3])
    pub drift: Option<Drift>,               // Flow([Real; 3]) | Vortex { speed, inflow, lift }
}

pub struct Ambient {                        // what Cosmos::ambient gives
    pub gravity: Real,                      // m/s² along −z
    pub atmosphere: Option<AtmosphereId>,
    pub clock: ClockId,
    pub gravity_vector: [Real; 3],          // m/s² east, south and up
    pub drift: Option<AreaDrift>,           // { area, drift, centre }: as the area set it
}
```

- A gravity scale multiplies the stratum's gravity; a vector replaces it,
  and `gravity` is then what is left of it along −z.
- Gravity, drift and the clock are each decided separately, among the
  areas covering the cell that set that thing: the highest priority, and
  of equals the area made first.
- `ambient` reports the drift as the area set it, with the area and the
  middle of its shape, which a vortex turns about. What velocity that is
  at a given cell is worked out with the medium (11 §4.7).

### 8.4 Sites

```rust
pub struct Site {
    pub id: SiteId,                  // unique in the cosmos, never reused
    pub def: SiteDefId,
    pub name: String,
    pub realm: RealmId,
    pub tracts: Vec<TractPos>,       // sorted
    pub anchor: CellPos,
    pub vars: VarMap,
    pub tags: Vec<String>,           // sorted
}
```

A tract belongs to at most one site: making a site over a tract that has
one, or claiming such a tract, is refused and names the site.
`Cosmos::create_site` and `remove_site` make and remove sites,
`claim_site_tract` and `release_site_tract` change what a site spans,
`set_site_name`, `set_site_tag` and `set_site_var` change the rest, and
`site_at(realm, tract)` and `site_of(location)` read the map. Sites raise
no world events.

The simulation attaches to a site what the cold layer needs: its people,
its stocks, its jobs, and its **local knowledge**, which is what the people
there know (09 §12, §13). The world stores the site and maps tracts to it.

A site gives a place identity that survives being unloaded: a bank, a
headquarters, a lair. The cold simulation (document 09) attaches owners,
occupants and events to sites. The world only stores them and maps tracts to
them.

## 9. Tracts and regions

### 9.1 Records

Everything the engine knows about unloaded space is in records.

```rust
pub struct RegionRecord {
    pub survey: RegionSurvey,                 // region kind and field values (10 §8)
    pub tracts: Box<[TractRecord; 4096]>,
    pub placements: Vec<Placement>,           // maps placed in this region, indexed by tract (10 §4.1)
    pub paths: Vec<PathLine>,                 // roads, rivers, walls crossing it (10 §8.3)
    pub stamps: Vec<Stamp>,                   // region-wide stamps
    pub sites: Vec<SiteId>,
    pub vars: VarMap,
}

pub struct TractRecord {                      // about 16 bytes; a surveyed region is about 64 KB
    pub def: TractDefId,                      // what kind of place this is
    pub rot: u8, pub variant: u8,
    pub state: TractState,                    // Surveyed | Realized { pristine: bool }
    pub devastation: u8,                      // section 17
    pub devastation_kind: Option<DevastationKindId>,   // of whichever delivered the most; the later wins a tie
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
- The codec is versioned per blob. A blob is the frame of 01 §11.1: a
  kind, a version, a checksum and a body in the engine's binary encoding.
  A blob that fails any check is an error, never a panic.

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

What a packet does carries the packet's cause, whatever cause the edit
that applies it was begun for. A target that is affected takes all of
the packet that does not pass: `absorbed + passed` is the packet's
energy. A target that is not affected takes none and passes none: the
energy stays with whatever delivered it. Where there is nothing to hit,
all of it passes. So energy absorbed never exceeds energy delivered
(invariant W8), whatever a `factor` is.

### 12.2 Applying a packet to an element

```
resist    = definition override, else material, else damage type default
gain      = clamp(intensity / resist.threshold − 1, 0, 1)
effective = energy × resist.factor × gain
capacity  = volume × material.toughness
remaining = capacity × (1 − damage / 255)

if effective ≤ 0:
    Unaffected. passed = 0
elif effective ≥ remaining:
    Destroyed.  passed = energy × (1 − remaining / effective)
else:
    Damaged.    damage += ceil(255 × effective / capacity);  passed = 0
```

Below threshold nothing happens, however often it is repeated. Full effect is
reached at twice the threshold. Accumulated damage is stored in 255ths.

- The test for "unaffected" comes first, so that something with no
  capacity is destroyed by whatever affects it and by nothing that does
  not.
- An element that is damaged and not destroyed keeps at least one part
  of its capacity: its damage stops at 254. Any packet that affects it
  adds at least one part.
- With no threshold, any intensity above zero has full effect.
- The rule alone is the function `resolve_packet(resist, capacity,
  damage, energy, intensity)`, which the body model shares (document 06).
  `WorldEdit::damage(loc, layer, packet)` applies it to one element of a
  cell; `damage_along` is the same with the direction of the hit, for
  debris. A fixture that piles up (section 12.3) has the capacity of the
  amount it holds.

Worked example with the illustrative values above. A wall cell holds
0.6 m³ of concrete, so its capacity is 0.6 MJ. An 80 kg person thrown into it
at 20 m/s carries 16 kJ, delivered at about 0.3 MPa: below the 20 MPa impact
threshold, so the wall is unharmed and the person absorbs all of it. The same
person at 600 m/s carries 14.4 MJ at about 290 MPa: the wall is destroyed,
0.6 MJ is spent, and 13.8 MJ carries on to the next wall. That is scene S2.

Callers compute intensity. Conventions:

| Source | Intensity |
|--------|-----------|
| Strike with a body part or weapon | `energy / (surface area × give)` (07 §6.1) |
| Kinetic impact of a body | `KE / (area × depth)`, section 15 |
| Pressure wave | Energy density at that distance, section 13 |
| Field contact | Per field definition, section 16 |

### 12.3 Destruction

When an element is destroyed:

1. It is replaced by its `on_destroy` state, or removed.
2. Its material's rubble for the damage type that destroyed it is added to
   the cell (or the nearest standable cell below): a fixture, or a field. Rubble amounts add up; a full cell of rubble becomes
   the fixture's `heap` block.
3. If the packet passed energy on and the debris budget (`debris.max_bodies`)
   allows, part of the
   element's mass is launched as debris bodies along the direction of the hit
   (section 15). Debris does secondary damage.
4. `ElementDestroyed` is emitted, and a stability check is queued.

**Rubble.** Matter is kept. What an element leaves is the volume it
lost: all of it if it is removed, and the difference if its `on_destroy`
state is smaller.

- A fixture that names a `heap` **piles up**: the bits of its state are
  an amount, in litres (`RUBBLE_UNITS_PER_M3`, 1000 to the cubic metre).
  Its mass and its capacity follow the amount, not the definition's
  `fill`. A state written with no amount (`"rubble"`) holds the volume
  its definition gives. The pile is made of the material of what was
  destroyed (`rubble@brick#600`); where piles of two materials meet, the
  larger gives the pile its material.
- Rubble comes to rest where something dropped into the cell would
  (`Cosmos::resting_cell`): the cell itself if it can be stood in, or
  else the nearest below that can. A fixture of another kind lying
  there is buried: its volume joins the pile.
- A cell is full at its three cubic metres (`RUBBLE_CELL_UNITS`, 3000).
  A pile that reaches that in a cell with no block becomes the heap
  block, of the pile's material, and what is over goes on top of it, in
  the cell above. Beside a block that stands in the cell the pile only
  grows.
- Rubble that is destroyed leaves none: neither a fixture that piles up
  nor the heap block of one.
- What was destroyed by falling (section 14.3) leaves nothing where it
  stood. It is carried down and left where the mass comes to rest.

Rubble by damage type, and rubble that is a field, come with change of
state (section 5.4; package W25): until then a material names one
fixture. Debris bodies come with section 15: until then `damage` offers
the fragments (the mass the element lost, its material, the energy the
packet had left, the direction of the hit) to a hook that throws
nothing, and all of the element is left as rubble.

A fixture left with nothing under it, because the floor or the block it
stood on was destroyed, comes down at the stability check the
destruction asked for (section 14.3).

## 13. Energy releases

```rust
pub struct EnergyRelease {
    pub origin: Location,
    pub origin_alt_mm: u16,
    pub energy: Real,                 // J
    pub kind: DamageTypeId,
    pub shape: ReleaseShape,          // Sphere | Cone { dir, half_angle } | Beam { dir, radius, range } | Front { … } (section 13.5)
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

| Release | Energy | Radius at 70 kPa (the example wall's threshold) |
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

### 13.5 Fronts

A **front** is a release shaped as a wall that advances in one direction: a
great wave, an avalanche, a flow of ash, a wall of force.

```json
{ "release": { "at": "$shore", "kind": "impact", "energy": "4e12 J", "speed": "40 m/s",
    "shape": { "front": { "dir": "east", "width": 2000, "height": 9, "range": 3000,
                          "leaves": { "field": "water", "amount": 255 } } } } }
```

- Intensity at distance `d` is the energy spread over the volume swept, as
  for a sphere: `I(d) = E / (width × height × d)`, for `d` of a metre or
  more. It falls with distance, not with its cube, so a front carries far.
- Its base in each column is the top of the ground or of still liquid
  there. It does not enter a column whose surface stands more than
  `height` above the one it came from. With `"downhill": true` it enters
  only columns no higher than the last.
- Shadowing is as in section 13.2, with the transmission map laid across
  the front instead of around a point. What survives shelters what is
  behind it.
- Occupants and bodies it reaches are exposed as by any release, and are
  pushed along `dir`.
- `leaves` adds a field at the base of each column it passes. What that
  field does next is its own rules.
- Unloaded tracts in its path receive a stamp, applied in resolve mode by
  distance along `dir`.

## 14. Structural support and collapse

The model is deliberately simple: no stress analysis. It answers two
questions cheaply and deterministically: "is this element still held up?"
(section 14.1) and "can what holds it up bear it?" (section 14.5). Falling
debris does the rest.

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

- "Horizontally adjacent" is the eight cells round a cell: a step
  across a corner counts one, like a step along a side. A slab of span
  4 reaches a square of nine cells by nine from one column, which is
  the 39 tonnes of section 14.5.
- A material that states no `span` has a span of zero: an element of it
  must stand on something.
- A block flagged `liquid` is not structure. It does not fall, it is
  never crushed and it holds nothing up, except as floating says below.
- A block flagged `anchor` is always supported, and so is a floor laid
  on one, and so is the solid fill outside a tract's stored cells. They
  are sources and are never examined: the ground under a street does
  not join every building on it into one structure.
- What lies beyond the reach of a check, or in a tract that is not
  loaded, is assumed supported, and what is next to it can hang from
  it.
- Where ambient gravity is zero or less, nothing falls: an element
  with no support stays where it is, and holds nothing up.
- Sources are found in order of position and support spreads breadth
  first, a step at a time, the eight neighbours taken clockwise from
  north, so that which of two equally near supports carries an element
  is the same on every machine.

**Floating.** A floor lying on still liquid denser than its own material is
a source: it floats. It carries no blocks. It holds up occupants, items
and fixtures up to the mass that it and the floating floors joined to it
within its `span` can float: the liquid they would displace, less their
own mass. Past that it gives way as if destroyed. A wide sheet of ice
bears a cart. A single floe does not bear a person.

### 14.2 When it runs

Removing or damaging a structural element queues a stability check for its
location. Checks within `SETTLE_DELAY` (50 ms local) are merged, so one blast
is one evaluation. The check examines elements at or above the change that
are structurally connected to it, within a bounded box. Elements outside the
box are assumed supported.

- **The delay** is local time where the change was made. The first
  request made where a clock applies starts a wake-up on that clock,
  `SETTLE_DELAY` later. When it fires, every request then waiting where
  that clock applies is taken together. A request where another clock
  applies waits for its own wake-up: inside stopped time a check waits
  until time moves.
- **What is examined.** The requests taken together are grouped: each
  group is the requests within reach of the first of them. For a group,
  the check starts from what each change could have been holding up
  (what is in its cell, in the eight cells round it and in the cell
  above) and follows the structure along every way support or load can
  pass: between the floor and the block of a cell, to like elements of
  the eight cells round it, up from a block to what stands on it, and
  down to the block under a floor or a block. Going down matters: the
  load that a lost support carried lands on other supports, all the way
  to the ground. This is the **island**. It never includes anchors or
  floors laid on them.
- **Bounds.** The island is cut off `support.reach` cells (24) sideways
  and `support.reach_storeys` storeys (40) up and down from the changes
  of its group, and at `support.max_elements` elements (131,072).
- **Asking.** `WorldCtx::check_stability(loc, cause)` asks for a check
  as an edit does. Putting an element in place asks for none (section
  7.2), so whoever builds asks, as `map check` does (10 §3.7).
- **Reading.** `Cosmos::assess_stability(grid, cells)` makes the same
  evaluation and changes nothing. Its `Assessment` says what is crushed
  and what falls, and for every element examined whether it is
  supported, its carrier, its distance from support and its load.

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

**A falling mass** is the unsupported floors and blocks of one vertical
run of cells, with the fixtures in those cells. A fixture that has
nothing under it, in a changed cell or the cell over one, falls the
same way. The elements leave the world when the check finds them
(`ElementDestroyed`, `how: fell`), and the mass is a process of the
world on the clock of the cell its base was in.

- **Where it lands** is where something dropped from its base comes to
  rest (`Cosmos::resting_cell`): the first cell, from its base down,
  that has a floor or lies on a solid top. That is looked at again when
  it arrives. If something has been put in the way it lands there; if
  what it was to land on has gone it keeps going, with the speed it
  has.
- **The blow.** It strikes the floor of the cell it lands in, or else
  the block below, with one packet of the damage type flagged
  `kinetic`: `energy = carried + m · g · h`, and `intensity = energy /
  (1 m² × 0.1 m)`, the cross-section of a cell and the default stopping
  depth of a body (section 15). Every occupant of that cell gets
  `OccupantExposed` with the same numbers, once. With no kinetic damage
  type loaded it strikes nothing.
- **Through.** If what it struck is destroyed, the mass takes the
  wreck with it, keeps the energy that passed and goes on down from
  there, starting at the speed that energy gives it. If not, it comes
  to rest in that cell: each material it is made of is left as that
  material's rubble (section 12.3).
- **Clocks.** When an area's clock is made, removed, moved or given
  another rate, every mass in flight goes onto the clock of where it
  is. Time stopped round a falling mass leaves it hanging, and it
  resumes with the time it had left.

**A collapse** is everything that follows from one failure. A check
that finds something giving way joins the collapse already in progress
in the same grid whose box of touched cells it meets or adjoins, or
starts one (`CollapseStarted`). The collapse ends (`CollapseSettled`)
when none of its masses is in flight and no stability request is
waiting in or beside its box. Its cause is a process, with the
instigator of the earliest of the requests that found it: every element
it crushes, drops or breaks carries that cause. `CollapseSettled`
carries its totals by instigator.

The world's processes are one table, saved and hashed with the cosmos
(`Cosmos::activity`, `Cosmos::restore_activity`), and every one of their
wake-ups carries the world's domain. `WorldCtx::handle_wake(wake)` is
the one entry point: the kernel calls it for each such wake-up it pops.
A collapse in progress is nothing but that state and the timeline, so a
save taken in the middle of one resumes it.

### 14.4 Resolve mode

The same evaluation is run repeatedly to a fixed point with landings computed
directly, capped at a fixed number of passes. Past the cap, everything above
the lowest unsupported storey in the affected footprint becomes rubble.

### 14.5 Load

Being held up is not the same as being able to bear it. After the support
pass of section 14.1, a second pass asks what each load-bearing block is
carrying.

- During the support pass each element records which element its support
  came from: its **carrier**. A floor's carrier is the block it rests on, or
  the neighbour its support came through. A block's carrier is the block
  beneath it, the floor it stands on, or the neighbour it hangs from.
- Working from the top storey down, an element's **load** is its own mass
  plus the loads of everything it carries. Fixtures and rubble count with
  the cell they are in.
- A load-bearing block is **crushed** when

```
load × gravity / area  >  bearing × (1 − damage / 255)
```

  where `area = fill × 1 m²` and `bearing` is its material's strength in
  compression. Blocks flagged `anchor` are not checked.
- A crushed block is destroyed, with the collapse as its cause, and the
  check runs again for what it was carrying.
- Every block that is overloaded in one pass is crushed together, and
  both passes run again without them, within the same check, up to
  `support.max_passes` times (48). Whatever is still overloaded then is
  found by the check that the crushing itself asks for. A crushed block
  becomes its `on_destroy` state or rubble, like any destroyed element
  (`ElementDestroyed`, `how: crushed`).
- A fixture counts with the floor of its cell, or, in a cell without
  one, with the block below.
- An element that is not `load_bearing` is never crushed, though it
  passes on what hangs from it.

What follows from it:

- **Damage matters before destruction.** A column damaged to half strength
  carries half as much. Supports that survive a blow can still fail under
  what is above them. This is the "damaged supports fail" of scene S2.
- **Losing some supports overloads the rest.** What the missing ones carried
  finds another carrier within span, or falls. Either way the survivors
  carry more, and may go in turn.
- **Weight is local.** `gravity` is the ambient value where the block is
  (section 4.1; areas, section 8.3). Nothing is crushed in free fall. A
  change of ambient gravity over an area queues a check for what stands in
  it, so a field of heavy gravity can bring a building down. The change
  is one made through `WorldCtx`: an area that sets gravity being made,
  removed, moved or reshaped, or `WorldCtx::set_area_ambient`. Each
  floor and each block that is not an anchor in the loaded cells of the
  area is asked about, up to about a million cells looked at.
- **What is built must stand.** A tower on one mud pillar does not.

With the illustrative values of section 5.4, a solid concrete pillar one
metre square bears about 3,000 tonnes. A slab reaching four cells each way
from it weighs about 39 tonnes a storey, so that pillar alone carries
several dozen storeys of bare floor, and fewer of furnished ones. A wall
cell, a fifth as thick, bears a fifth as much.

Resolve mode (section 14.4) includes this pass.

### 14.6 Tremors

`tremor { at, strength, radius, time, kind }` shakes the ground.

For its duration it is a process that lowers the `span` and `bearing` of
every structural element in loaded tracts within its radius, by a factor
that is `strength` at the centre and falls to nothing at the edge. It
queues stability checks for them, spread over steps within the edit
budget. What no longer holds falls (section 14.3). What can no longer bear
its load is crushed (section 14.5). When it ends the factor is removed,
and what fell stays fallen.

- Occupants standing in it receive `OccupantShaken` with the local
  strength. Whether they keep their feet is the simulation's business.
- Blocks flagged `anchor` do not fall. Ground that does not anchor
  (10 §5.2) can come down: a landslide, a cave-in.
- Unloaded tracts within the radius are given devastation of the named
  kind by the same falloff (section 17.2).

A tremor does not move the ground. It finds out what was only just
standing.

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
    pub volume: Real,            // m³, for buoyancy (11 §4.4)
    pub drag: Real,              // drag coefficient
    pub depth: Real,             // m, stopping distance for impact intensity; default 0.1
    pub restitution: Real,       // 0..1
    pub drive: Option<Drive>,    // thrust along its path for a time; a turn rate (07 §11.1)
    pub cause: Cause,
}

impl WorldCtx<'_> {
    pub fn launch(&mut self, body: Body) -> BodyId;
    pub fn impulse(&mut self, id: BodyId, j: [Real; 3]);
    pub fn arrest(&mut self, id: BodyId) -> Body;     // caught, or recovered control
    pub fn steer(&mut self, id: BodyId, toward: Option<[Real; 3]>);   // where a driven body turns to
}
```

Bodies are point masses with a cross-section. There is no rotation.

A body with a `drive` gains speed along its path while its thrust lasts,
and turns its path toward the point last given to `steer`, no faster than
its turn rate. The world does not know what it is aiming at. The
simulation tells it where to turn (07 §11.1).

### 15.1 Motion

Each body is a process on the clock of where it is. It advances from cell
boundary to cell boundary, with a maximum step of 20 ms local so that gravity
and drag integrate accurately:

```
w   = v − drift                    (the medium's own movement, 11 §4.7)
a   = gravity(strata) − ½ · ρ_air · drag · area · |w| · w / mass
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

### 15.4 Leaving loaded space

The ticket a body carries ahead of itself is small: enough for a thrown
person or a rifle shot. A body whose path runs further than
`body.max_ticket_tracts` (default 3) beyond what is otherwise loaded is
**finished by arithmetic**. Its flight is worked out over the ground
heights the tract records give, without loading anything, to find where
and when it comes down. Then it is removed.

| Where it comes down | What happens |
|---------------------|--------------|
| In a loaded tract | It is put back as a body just short of impact, at the right time, and lands in full |
| In an unloaded tract | A pending edit (section 17.3) carries its impact: a release of the energy it arrived with, and whatever it was set to do on impact. A creature or item it was, or carried, is left there (06 §12). |

Buildings in unloaded tracts along the way are not struck. A shell fired
over the horizon and a person punched across a city both arrive.

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
| `drift` | Be carried by the movement of the medium (11 §4.7). Contents move downwind or downstream, and `spread` is likelier that way. |
| `expose` | Add to a pool (06 §4.2) of each occupant, by a route. See below. |

A field may state `warms`: the kelvin each level adds to the temperature
of its cell (11 §4.6), negative to cool. A field may have `changes` like a
material's (section 5.4), tested on its active cells: lava that cools
below its threshold leaves rock, and standing water that freezes leaves
ice.

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

**Reactions.** What fields do to each other and to what they touch is
written once, as **field reactions**, not separately in each field.

```json
[
  { "type": "field_reaction", "id": "quench",
    "when": ["water", "fire"],
    "uses": { "water": 1, "fire": 3 }, "gives": { "steam": 2 } },
  { "type": "field_reaction", "id": "oil_feeds_fire",
    "when": ["fire", { "material_tag": "oily" }],
    "gives": { "fire": 30, "smoke": 20 } },
  { "type": "field_reaction", "id": "current_through_water",
    "when": [ { "damage": "shock" }, "water" ],
    "spreads": { "share": 0.8, "reach": 6 } }
]
```

| Form | Happens |
|------|---------|
| Two fields | Each step, in every cell where both are present |
| A field and a material tag | Each step, where the field shares a cell with an element or item of a material so tagged |
| A damage type and a field | When a packet of that type arrives in a cell holding the field. `spreads` passes a share of the packet to every connected cell of that field within reach. |

A reaction may also `leave` an element in the cell: lava meeting water
gives steam and leaves basalt.

**Exposure.** Not all harm is a blow. The `expose` rule adds to a pool of
each occupant in the cell, in proportion to the field's level, by a
**route**:

```json
{ "rule": "expose", "pool": "toxin", "per_level": 0.02, "route": "breath" }
```

| Route | Reaches |
|-------|---------|
| `breath` | Those who breathe the cell's medium. Not those with air of their own: a sealed suit or volume (11 §10) while its air lasts. Not bodies that do not breathe. |
| `skin` | The share of the body not under a worn layer that guards against it |
| `any` | Everyone in the cell |

Worn items may state `guards`, the share they keep out by route (06 §9.1):
a mask, a filter, a coat. The hook `exposure_incoming` lets rules change
the amount. What the pool then does to its owner is content (06 §4.2): a
poison, a sickness, a slow burn.

Harm that passes through walls is not a field. It is a signal that
exposes (15 §3.1).

Reactions are compiled into the field stepper and follow its rules:
buffered writes and positional randomness. The `convert` rule in a field's
own definition is shorthand for a reaction between two fields.

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

**Kinds.** Devastation has a kind. A release gives the kind its damage
type names, `blast` unless it says otherwise. Other causes write
devastation directly:

```json
{ "type": "devastation_kind", "id": "flood", "apply": "flood_damage", "ruin": "drowned" }
```

`devastate { where, amount, kind, cause }` raises the devastation of
tracts to at least `amount` and records the kind. `where` is a list of
tracts, a region, or a centre with a radius and a falloff.
`RegionDevastated` is raised as in section 17.4, carrying the kind.

| The tract is | What happens to it |
|--------------|--------------------|
| Loaded | The kind's `apply` transform (10 §10) is run on it through `WorldEdit`, scaled by the amount, unless the caller says the damage was already done |
| Unloaded, realised before | A pending edit does the same when it next loads |
| Never realised | Generated, then the same; at total devastation, generated from the tract's ruin variant for that kind |

An earthquake, a flood, a wildfire and a fall of ash each leave their own
mark on places nobody was watching, and the cold layer (09 §15) reads one
number and one kind for all of them.

### 17.3 Other pending edits

```rust
pub enum PendingEdit {
    Stamp(Stamp),
    Cells(Vec<CellEdit>),                   // explicit small changes
    Map { map: MapId, at: CellPos, turn: Turn, params: VarMap, mode: MapMode },   // 10 §4.3
    Spawn(SpawnRequest),
    Liquid { level: i32, block: BlockState },   // fill or drain to a storey (11 §4.3)
    Devastation { kind: DevastationKindId, amount: u8 },   // section 17.2
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
    pub entity: Option<EntityId>,  // its hull entity, if it is a vehicle (11 §3)
    pub sealed: Option<SealedId>,  // its sealed volume, if it has an enclosed interior (11 §10)
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
  subgrid cell **shadows** the parent cell. So does every cell of a sealed
  subgrid's envelope, empty or not (11 §10.1): the sea outside a submarine
  is not inside it.
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

### 18.5 Loading and storing

- A subgrid is stored with the tract that contains its origin, its
  **anchor** tract.
- Every tract a stored subgrid overlaps records a reference to it. Loading
  any of those tracts loads the subgrid.
- A loaded subgrid holds a warm ticket over its whole footprint, so it is
  never half-present.
- A subgrid is unloaded when its anchor tract is, provided nothing on it is
  holding it loaded.
- Occupants standing in a subgrid are stored with it.

## 19. Portals and boundary links

```rust
pub struct Portal {
    pub id: PortalId,
    pub a: PortalEnd,                 // { loc: Location, facing: Dir }
    pub b: PortalEnd,
    pub two_way: bool,
    pub see_through: bool,
    pub view_range: Real,             // m: how near a viewer must be to see through it
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
between grids, updates clocks and tickets, and emits events. It takes a
ticket for where the mover is going. If that tract is not loaded the
mover is held out of the world until the next safe point has loaded it,
and then placed. It is refused, with a reason, only where there is nowhere
to arrive: a barrier edge, a destroyed realm. Portals,
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
    pub band: TravelBandId,              // data-defined: road, sea, undersea, air, orbit, … (11 §13.2)
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
realized; if the source tract is not loaded it is loaded for the copy, at
a safe point. `transform` is a map transform (10 §10) applied on top:
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
    pub fn raycast_into(&self, from: PointM, dir: [Real; 3], max_m: Real, filter: RayFilter,
                        obstacles: &mut dyn RayObstacles, out: &mut RayHits);
    pub fn line_of_sight(&self, a: PointM, b: PointM) -> Result<bool, WorldError>;
    pub fn field_of_view(&self, origin: Location, eye_alt_mm: u16, radius_m: Real,
                         out: &mut FovSet) -> Result<Freshness, WorldError>;
    pub fn neighbours(&self, loc: Location) -> Result<NeighbourGeometry, WorldError>;   // raw data for movement rules
    pub fn footing(&self, loc: Location) -> Result<Footing, WorldError>;
    pub fn find_path<R: MoveRules + ?Sized>(&self, from: Location, to: Location, rules: &R,
                                            limits: PathLimits) -> Result<Path, PathFail>;
}
```

All of them read truth. What a character decides by comes from what it
has perceived (01 R11): a mind's rules wrap these and never hand them to
a mind. None loads a tract or panics on one that is missing, and each
says what it does at the end of loaded space. On a wrapped axis each
carries on round. Results come in a defined order: hits nearest first,
cells storey by storey and row by row, steps clockwise from north, then
up, then down.

**Rays.** A ray steps through 1 × 1 × 3 m cells in the order of the
segment walk of section 3.3 and respects element heights: a block or
fixture fills the footprint of its cell from the base up to its height,
and a ray that stays at or above that height passes over it. A ray at
1.5 m passes over a 1 m fence.

| Ray | Stopped by | Also reports |
|-----|------------|--------------|
| Sight | A block or fixture flagged `opaque`; a floor | — |
| Movement | A block or fixture that is not `passable`; a floor | — |
| Projectile | A block or fixture that closes the whole width of its cell below its top; a floor | One that closes part of it, with its share |

- A hit says which cell, which element in what state, how far along the
  ray, the point, the share of the ray's path the thing closes, and
  whether the ray ended there. `RayHits` lists them nearest first and
  says how the ray ended: it went its whole length, something stopped
  it, or it was refused, with the reason.
- The share a projectile is given is the chance that a thin projectile
  at that height strikes the thing. The world rolls no dice in a query:
  whoever fires does. An element's `coverage` is the share of its cell's
  whole cross-section that it closes (section 5.3), and a ray that meets
  it is already known to be below its top, so the share is
  `coverage × 3 / height`, at most 1. A 1 m fence with the default
  coverage closes everything below 1 m and nothing above; a pillar of
  full height with coverage 0.5 is struck half the time; an open door,
  with coverage 0, is not met.
- A floor is met where a ray passes between two storeys. It is the floor
  of the upper cell, and it stops every kind of ray.
- The block and fixture of the cell a ray starts in never stop it:
  whoever casts it is taken to be clear of what shares its cell.
- A ray is followed for at most 10 km (`RAY_MAX_M`). Where it comes to
  space that is not loaded, or to an edge of the realm, it ends, keeps
  what it met, and is marked refused with the error of section 7.1.
- Terrain exists on every plane (section 8.2), so a ray takes no plane
  mask. Occupants join a ray through `RayObstacles`, which is asked for
  each cell the ray passes, with the stretch of the ray inside it. The
  occupant index answers it with its own plane and kind filters, so that
  one ray meets walls and bodies in the order they stand along it.

**Line of sight** between two points is clear when nothing `opaque`
stands in a cell strictly between them taller than the line passes, and
the line passes through no floor. What is in the two cells that hold the
points does not block: one sees out of one's own cell, and one sees the
thing one looks at, a wall included. The answer is the same whichever
point is given first. Through space that is not loaded it is an error,
not a guess; between grids, and beyond the length of a ray, it is false.

**Field of view** fills a `FovSet`, a buffer the caller keeps: the cells
seen from a cell by an eye at a height, out to a radius, held as offsets
from the viewer.

- *On the viewer's storey* it is symmetric shadowcasting over whole
  numbers. A cell blocks sight when something `opaque` in it stands
  taller than the eye. A cell that blocks is seen if any of it is in
  view. A cell that does not is seen if the line from the middle of the
  viewer's cell to its own middle is clear.
- *Down.* Under every seen cell of that storey that has no floor and
  holds nothing opaque, the cell below is seen, and so on down while the
  same holds of each. The last is what the eye rests on: the ground, a
  floor, a roof. Under its own cell the viewer looks past whatever
  shares that cell.
- *Up.* While the cell over the viewer's head has no floor, the storey
  above is seen as from that cell, and the next in the same way, as long
  as the cells passed through hold nothing opaque. Under open sky that is
  every storey within the radius; indoors it is whatever shows through a
  hole in the ceiling.
- Looking up is the mirror of looking down, so sight is symmetric
  between storeys as on one: of two cells that do not block sight, with
  eyes at one height, each sees the other or neither does.
- Distance is between cell centres with a storey counting 3 m, and a
  cell at exactly the radius is seen. Cells that are not loaded are not
  seen and hide what is behind them. The radius is cut to 255 cells.
- A view counts a cell by its middle and a line of sight by every cell
  it passes through, so the two can differ by a cell at a corner.

**Neighbours** is raw geometry for movement rules: the cell, the eight
round it, the one above and the one below, each with its derived flags,
its **footing**, the height of what stands in the way of walking, any
vertical connection, and whether it is liquid or rough. A neighbour that
is not loaded or lies beyond an edge is absent.

- A ramp, stairs or ladder joins its cell to the cell directly above
  it. The step up starts in the cell that holds it and the step down
  ends there (section 5.2).
- Footing is what holds up someone in a cell: its floor, the solid top
  of the block below, or a vertical connection in the cell below.
- A cell also says whether what is in its way `opens`: every element in
  the way has a declared state in which it is not. A closed door has
  one; a wall has not.
- Which steps a mover can make, and at what cost, is for a `MoveRules`.
  `MoveProfile` is a plain walker: it needs footing, steps over what
  `walk_blocked` allows or a height of its own, goes between storeys
  only by a connection, and never takes a diagonal step past a cell
  that is in the way. A step costs its length in metres (section 3.3),
  doubled into a rough cell.

**Paths.** `find_path` is a search over cells (A*) on those steps. It
returns a cheapest path, or the reason there is none: no route, the goal
cannot be occupied, too dear, or the budget of cells was spent (20,000
unless the caller says). Where paths cost the same the choice is fixed:
cells are settled in order of the estimated cost of the whole path, then
of what is left, then of position, and steps are tried in their listed
order. The hierarchy of document 09 is built over this search and decides
which cells it is let loose on. The world's other obligations to
path-finding are revision counters and change events precise enough to
update a hierarchical path graph incrementally.

**Caches.** A kept answer carries a `RevStamp`: the structure revisions
of the chunks its reads were confined to. It is stale when one of them
has moved, or when one of their tracts has been loaded or unloaded, and
for no other reason. A chunk that holds only its tract's fill has never
been written and counts as revision zero. A `FovSet` asked the same
question of an unchanged world returns at once; a `Path` says whether it
`is_current`. A tract that is unloaded and loaded again must not come
back looking unchanged: loading starts its chunks' revisions where no
earlier loading of that tract left them.

Still to come, with the packages that build them:

- All queries composite subgrids. Sight, rays and signals cross a
  subgrid's boundary without limit: a subgrid is part of the scene.
- They pass a portal only if it is `see_through`, only for a viewer within
  its `view_range`, and only `sight.portal_depth` portals deep (default 1).
  While a see-through portal's near side is loaded and something that could
  see through is within range, the portal holds a warm ticket on its far
  side (section 10).
- Fields that hide, such as smoke, do not yet block sight.

Illumination (`Cosmos::light`) is specified in 15 §3.3 and the medium of a
cell (`Cosmos::medium`) in 11 §4. Both are world queries with caches keyed
on chunk revisions.

## 23. Modding surface

**Definition types**: `material`, `damage_type`, `atmosphere`, `floor`,
`block`, `fixture`, `conduit` (16), `field`, `field_reaction`,
`devastation_kind`, `realm`, `area_kind`, `chart`,
`site`, `tract` (document 10).
Elements may declare `interactions` and a `device` (section 5.5).

**Events** an EOC can trigger on: every world event in section 7.4.

**Effects** an EOC can perform (bound to the functions in this document):

| Effect | Function |
|--------|----------|
| Set or transmute an element | `set_floor`, `set_block`, `set_fixture` |
| Damage a cell | `damage` |
| Explosion, shockwave, beam, advancing front | `release` |
| Shake the ground | `tremor` |
| Devastate a place by other means | `devastate` |
| Add or remove a field | `add_field` |
| Mark a cell | `add_decal`, `set_var` |
| Throw, push, catch | `launch`, `impulse`, `arrest` |
| Tear loose, set down | `detach`, `merge` |
| Stamp a map fragment | `apply_map` at a location, live or pending (10 §4.3) |
| Open or close a portal | portal table |
| Create or destroy a realm | `create_realm`, `destroy_realm` |
| Create an area; set its clock rate | area table, clock |
| Make an area from a closed space | `enclose` |
| Keep somewhere loaded | ticket |
| Change somewhere unloaded | `PendingEdit` |
| Teleport or transfer | realm transfer |

**Queries** available to expressions: element definition, material, state,
flags and tags at a location; field value; damage; standable; obstacle
height; ambient gravity, atmosphere, temperature and drift; distance; line
of sight; area and
site membership; tract devastation; variables at cell, area, site, tract,
region, realm and global scope.

**Verb names.** The effects above are written in data as: `set_floor`,
`set_block`, `set_fixture`, `set_material`, `damage`, `release`, `tremor`,
`devastate`, `add_field`, `add_decal`, `remove_decal`, `set_var`,
`launch`, `impulse`, `arrest`, `detach`, `merge`, `apply_map`,
`open_portal`, `close_portal`, `create_realm`, `destroy_realm`,
`create_area`, `remove_area`, `set_clock_rate`, `enclose`, `hold_ticket`,
`release_ticket`, `add_pending_edit`, `transfer`, `set_plane`,
`set_liquid_level`.

**When an edit meets an occupant.** An edit never fails because someone is
standing there. If it puts something an occupant cannot share a cell with
into that cell, the occupant is displaced to the nearest cell it can
occupy within `edit.displace_range` (default 2 cells), with
`OccupantDisplaced`. If there is none it stays where it is and
`OccupantEntombed` is raised; what being walled in does is for rules.

The world knows no mover, so its rule is plain. A block or fixture that
makes a cell `walk_blocked` (section 6.2), where it was not before, leaves
no room for the creatures, items and other occupants in that cell. A
device stays: it stands in the cell of its own element. Each occupant, in
id order, goes to the first position, by rings of growing distance and
within a ring storey by storey and row by row, at which none of its cells
is `walk_blocked`, none holds an occupant on a shared plane that
`BLOCKS_MOVE`, and which can be stood in if where it stands now can be.
It is moved by `relocate`, so area events follow.

**Errors.** World operations that cannot be done return one of: not
loaded; outside the realm; a barrier; not connected (for `detach`); too
deeply nested (subgrids nest four deep); over budget. `detach` and
`transfer` report them as refusals with reasons (14 §5).

Also: an unknown grid, realm, area, area kind, site or occupant; an
occupant already placed; a tract that already has a site; a space that
does not close, with its leak, or a cell with no space to enclose;
something larger than the engine allows, naming what and the limit; a
value that makes no sense, saying why; a cell out of reach of its tract's
storage.

**What a mind may read** (05 §7.3). Safe for belief: the shape of the
world, meaning what a cell's elements are made of and whether they can be
stood on, passed or seen through. Not safe: an element's state, damage
and variables, fields, decals, who is where, and anything about areas and
sites the character has not perceived. The tag `hidden` is reserved: an
element, area, site or mark that carries it is left out of even the
shape until perceived.

**Content named by data, not by the engine.** A damage type flagged
`"kinetic": true` is the one bodies and falling masses deliver; exactly
one must be loaded if anything can be thrown or fall. A damage type names
the kind of devastation it leaves (`"devastation": "blast"`). A fluid or
atmosphere names the damage type its pressure does (11 §10.4). Without
them the load fails with a diagnostic; the engine supplies no default id
of its own (invariant W9).

**Definition types added here**: `plane`, `decal`, `sky` (ambient light by
time of day, and a backdrop; 15 §3.3, 12 §6.1). The query `decals` reads
the marks in a cell with what left them.

**Body properties.** `depth` and `restitution` are fields of `projectile`
definitions (07 §11.1), and of items and templates for when they are
thrown. Sliding friction is the floor material's `friction`. `REST_SPEED`
is a named constant.

**Fixed sets.** Field rules, release shapes, exposure routes and engine
flags are implemented natively. A mod configures and combines them and
cannot add one. Where a new behaviour is needed, `FieldContact` and the
other events are the way in. This is a known limit of the modding
surface.

**Constants.** Where this document says "a wide margin", "a flat shelter
factor", "a bounded box" or "a fixed number of passes", each is a named
setting under `release.` or `support.`, as is the hot radius around a
controlled character (`world.hot_radius`) and the vacuum default for a
release's speed (`release.vacuum_speed`). Devastation is scaled between
`world.min_intensity` and `devastation.max_intensity`. `spread` ignites
materials carrying the tag its `needs` names. `catch_up` is one of
`burn_out`, `disperse`, `settle`, `keep`, `none`.

**Settings**: `world.min_intensity`, `release.max_cells`,
`field.max_active_cells`, `body.max_ticket_tracts`, `max_loaded_tracts`,
`sight.portal_depth`, and the debris budget of section 12.3
(`debris.max_bodies`). For support and collapse (section 14):
`support.settle_delay` (50 ms), `support.reach` (24 cells),
`support.reach_storeys` (40), `support.max_elements` (131,072),
`support.max_passes` (48) and `support.impact_depth` (0.1 m, the
stopping depth of a falling mass). Until settings are bound they are the
constants `SETTLE_DELAY`, `SUPPORT_REACH_CELLS`, `SUPPORT_REACH_STOREYS`,
`SUPPORT_MAX_ELEMENTS`, `SUPPORT_MAX_PASSES` and `FALL_IMPACT_DEPTH_M`.
The units of rubble (a litre) and the fill of a cell (3000 of them) are
fixed.

**Reachable only from Rust** so far, each wanting a verb or a property
when the rule runtime binds this document: the queries of section 22 other
than line of sight (a rule that needs what a ray met, the cells a viewer
sees or the geometry round a cell has no expression for it; `path_cost`
(09 §6.4) reaches paths); giving an area another shape (`set_area_shape`),
override, priority or anchor after it is made; an occupant's footprint
(`relocate_as`) and its mass and flags (`update_occupant`); writing an
element's damage directly (`set_damage`); the fill that `enclose` uses, as
a query (`enclosed_cells`); and the occupant queries of section 8.1, which
rules reach only through their own queries over entities (05). From
damage and collapse: asking for a stability check (`check_stability`);
what a check would find, with carriers and loads (`assess_stability`);
where something dropped comes to rest (`resting_cell`); the amount a
pile of rubble holds (`rubble_units`); the direction of a hit
(`damage_along`); and the collapses in progress and masses in flight
(`collapses`, `falling_masses`). An area's gravity is changed with a
check through `WorldCtx::set_area_ambient`; the `Cosmos` function of
that name asks for none.

## 24. Invariants

| # | Invariant |
|---|-----------|
| W1 | Cells change only through `WorldEdit`. The occupant index changes only through `place`, `relocate`, `remove`. |
| W2 | Generation is pure: a function of definitions, realm seed, position and records. A derived realm in `live` mode also reads its source's cells as they are (section 21); that is its purpose and the one exception. |
| W3 | Loading a tract is a pure function of cosmos state, tract and tick. |
| W4 | Every occupant, body and process is in a loaded tract. |
| W5 | A process holds a ticket for the tracts it touches, or has been resolved. |
| W6 | No result depends on hash iteration order, chunk load order or thread count. |
| W7 | Save, load and continue equals continue. |
| W8 | Energy absorbed by a release or impact never exceeds energy delivered. |
| W9 | No definition id appears in engine code. Fills, rubble and placeholders are named by data. |
| W10 | Reading unloaded space returns an error value. It never loads and never panics. |
| W11 | Temperature is never polled. A change of state happens only at the three moments of section 5.4. |
| W12 | A tract's devastation and its kind are the same whether the cause was played out while it was loaded or applied when it next loaded. |

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
| Front reaching 100,000 non-empty cells | under 30 ms in total |
| One step of a state-change sweep or a tremor over the default loaded set | under 2 ms |
| Save with 50 dirty tracts | under 50 ms |

## 26. Open questions

These do not block implementation. Each names the document expected to
settle it.

1. **Crush failure.** Settled in section 14.5: load-bearing blocks have a
   limit, reduced by damage.
2. **Cave-ins.** Settled in 10 §5.2: whether ground anchors is chosen per
   stratum. Rock holds; loose ground needs support and can come down.
3. **Turning subgrids.** Settled in 11 §5.5: motion is in any direction;
   the grid itself swings in quarter-turns.
4. **Very large creatures.** Settled in 06 §6.6: a creature that can be
   stood on, climbed or entered is a subgrid whose cells are its body;
   anything smaller is an occupant filling several cells.
5. **Sight through portals and across subgrid boundaries.** Settled in
   section 22: always across subgrids; one portal deep, within range,
   through portals that allow it.
6. **Rewinding time.** Settled: snapshots, area journals and restoring a
   place as generated in 08 §7.4; the whole world in 13 §9.3 and §9.5.
7. **Scale of battle.** Settled in 09 §9.3 (a cap on fully simulated
   characters, by attention) and 09 §5.5 (crowds that do not jam).

Settled since first draft: there is no default planet. World size is a
property of each realm definition (section 4.1), chosen by whoever writes
the world. Reference mods use small realms.
