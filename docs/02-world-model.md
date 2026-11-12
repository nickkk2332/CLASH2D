# 02 — World Model

Status: design complete, ready for implementation.
Implementation plan: [plans/world-model.md](plans/world-model.md).
Serves scenes S1, S2, S3, S7, S8, S9, S10, S11, S12, and the world's share of
S13, S18 and S19, of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how space, matter and place are represented, stored,
loaded, changed and destroyed. It covers the `clash-world` crate, which
holds the generator interface, and the test generators of `clash-gen`,
which implements it (section 9.3).

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
    portals: PortalTable,
    areas: AreaTable,                          // every area, of every grid (section 8.3)
    sites: SiteTable,                          // every site, of every realm (section 8.4)
    occupants: LookupMap<EntityId, OccupantRecord>,   // where each occupant is (section 8.1); not state
    stability: StabilityQueue,                 // checks asked for and not yet made (section 14.2)
    activity: WorldActivity,                   // the world's processes: checks, falling masses, bodies in flight, releases (sections 13 to 15)
    vars: VarMap,                              // global variables
    save: SaveState,                           // the save's id tables and entity sections waiting (section 11); not state
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
| `generator_params` | Optional. What the realm tells its generator: values by name, which the generator reads (section 9.3). |
| `fill` | What implicit cells contain below and above the generated range of a tract. |
| `strata` | Ambient conditions by altitude: gravity (m/s², along −z), atmosphere (a definition giving density, pressure, sound speed, breathability; `null` is vacuum), sky (a `sky` definition: the light the sky gives by time of day and a backdrop, 15 §3.3; left out, the stratum has no sky light), and temperature: `temp_offset` added to the region's, or `temp` replacing it (11 §4.6). |
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
- `calendar` and boundary links are stored as written until the
  packages that give them meaning are built. `generator` and
  `generator_params` are stored as written and read when the generators
  are bound (section 9.3): a realm with no generator, or one nobody
  registered, loads and cannot have tracts generated.
- A stratum's `sky` names a `sky` definition. Naming one that does not
  exist is a load error.

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

A damage type may be flagged `severs`: across the whole width of a body
part, a hit of that type takes the part off (06 §7.2). The world stores
the flag and gives it no meaning.

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
| `friction` | — | The coefficient of sliding friction of a surface of it (section 15.3). Absent, a half. |
| `changes` | — | What it becomes by temperature. See below. |
| `flammable` | — | Shorthand for the tag `flammable`, for a field rule or reaction that asks for a material tag (section 16). |
| `tags` | — | Free-form tags. A field's `spread` and a field reaction name one to say what they act on. |
| `leak`, `signal`, `heal` | — | What it does as the tissue of a body: drains a pool when breached, adds to a pool when damaged, and recovers by itself (06 §6.1). The world stores them and gives them no meaning. |

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

`light` makes an element a fixed source of light, and a state may give
another light or, with an intensity of zero, none (15 §3.3):

```json
{ "type": "fixture", "id": "lamp", "material": "steel", "height": 2.5, "fill": 0.02,
  "light": { "intensity": 1.0, "radius": "8 m" },
  "states": { "on": {}, "off": { "light": { "intensity": 0 } } } }
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
`RevStamp` (section 22). Counters do not start at zero for every loading:
a column that joins a grid starts all of its counters one above the
highest any counter stood in a column that has left that grid, and a chunk
height without storage counts as standing where its column started
(`Column::rev_base`).

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
    pub fn set_state_bits(&mut self, loc: Location, layer: Layer, bits: u16) -> Result<ElementState, WorldError>;   // another state of the same element
    pub fn destroy(&mut self, loc: Location, layer: Layer, kind: DamageTypeId) -> Result<bool, WorldError>;         // section 12.3, outright
    pub fn set_damage(&mut self, loc: Location, layer: Layer, damage: u8) -> Result<u8, WorldError>;
    pub fn damage(&mut self, loc: Location, layer: Layer, p: DamagePacket) -> Result<DamageOutcome, WorldError>;   // section 12
    pub fn damage_along(&mut self, loc: Location, layer: Layer, p: DamagePacket, dir: Option<[Real; 3]>) -> Result<DamageOutcome, WorldError>;
    pub fn add_field(&mut self, loc: Location, field: FieldId, amount: i16) -> Result<u8, WorldError>;   // section 16
    pub fn set_field(&mut self, loc: Location, field: FieldId, level: u8) -> Result<u8, WorldError>;
    pub fn conduct(&mut self, loc: Location, p: DamagePacket) -> Result<u32, WorldError>;         // a packet arrived in a cell: section 16
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
  `set_state_bits` changes only the state: a door that is opened keeps
  its definition, its material and its damage (decision D-620).
- **`destroy`** destroys an element outright, as section 12.3 does for a
  packet that is more than it can take, by a kind of damage and with
  nothing absorbed: it becomes its `on_destroy` state or is removed, its
  matter is left as rubble, no fragments fly, and `ElementDestroyed`
  follows. It is how a rule destroys by other means than energy.
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
cell, and when it contacts an occupant. A structural edit also wakes the
fields asleep in its cell and in the cells beside, above and below it
(section 16).

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
| `OccupantDisplaced` | `occupant`, `from`, `to` | The world moved an occupant: an edit left it no room (section 23), or it is a body in flight and was carried into another cell (section 15) |
| `OccupantEntombed` | `occupant`, `loc` | An edit left an occupant no room and there was nowhere to go |
| `ElementDestroyed` | `loc`, `layer`, `old`, `new` (states; `new` is the `on_destroy` state or nothing), `how` (`damage` with its type, `crushed` or `fell`), `absorbed`, `passed` (J; zero unless by damage) | A floor, block or fixture was destroyed (section 12.3). It follows the `ElementChanged` events of the change and of the rubble left |
| `OccupantUnsupported` | `occupant`, `loc` (where it is) | What held an occupant up has gone (section 7.2) |
| `OccupantExposed` | `occupant`, `loc` (the cell struck), `kind` (damage type), `energy` (J), `intensity` (Pa) | Something struck the cell an occupant is in: a mass that landed there (section 14.3), the front of a release or what a closed space held of one (section 13.2), a field's `damage` rule, once a step for the strongest cell of the field the occupant fills, or damage a field passed on (section 16) |
| `FieldAppeared` | `loc`, `field`, `level` | A field came to be in a cell that held none of it (section 16). Other changes of level are not reported |
| `FieldVanished` | `loc`, `field` | The last of a field left a cell |
| `FieldContact` | `occupant`, `loc`, `field`, `level` | A field and an occupant came to share a cell: the field appeared where the occupant is, or the occupant was placed or moved into a cell it did not fill before that holds the field. Once for each field and cell, not at every step |
| `OccupantDosed` | `occupant`, `loc` (the cell of the field), `field`, `level`, `pool` (as the rule writes it), `amount` (`level × per_level`), `route` (`breath`, `skin` or `any`) | An occupant is in a cell of a field with an `expose` rule: once for each step of the field, for the strongest cell the occupant fills. The world applies no route |
| `FieldBudgetExceeded` | `loc` (the first cell of the step that found it), `active`, `max` | A step found more active field cells than `field.max_active_cells`, and the step before did not |
| `CollapseStarted` | `collapse` (id), `loc` (the cell whose change started it) | A stability check found something giving way where no collapse was in progress |
| `CollapseSettled` | `collapse`, `loc`, `bounds` (the box of cells it touched), `tallies` (by instigator: `elements` destroyed, `mass` brought down in kg, occupants `exposed`) | Nothing of a collapse is in flight and no check is waiting near it |
| `TractLoaded` | `realm`, `tract`, `loc` (the tract's cell with the lowest coordinates, at storey 0), `first` (never loaded before) | A tract has been loaded, after the changes waiting for it were made (section 9.4) |
| `TractUnloading` | `realm`, `tract`, `loc` | A tract is about to be unloaded, or is to be and still holds occupants (section 9.5) |
| `TicketRefused` | `ticket` (the id it had), `realm`, `loc` (the tract it was centred on), `reason` (who held it) | The tickets wanted more tracts than may be loaded and this one was dropped (section 10.3) |
| `ReleaseStarted` | `release` (id), `loc` (the origin), `kind` (damage type), `energy` (J), `reach` (m, with nothing in the way) | Energy was let go (section 13); its cause is what let it go |
| `EnergyReleased` | `release`, `loc`, `kind`, `energy`, `absorbed` (J elements took), `reach` (m the front got to), `cells` (visited), `cut_off` (stopped for the budget), `tallies` (by instigator: `elements` destroyed, their `mass` in kg, occupants `exposed` and `thrown`, J `absorbed`) | A release's front has gone as far as it goes (section 13.2). Its cause is the release's doing |
| `BodyLaunched` | `body` (id), `owner` (what it is), `loc`, `pos` (m), `vel` (m/s), `mass` (kg) | Something was set flying (section 15); its cause is what launched it |
| `BodyImpact` | `body`, `owner`, `loc` (the cell struck), `layer`, `state` (the element as it was), `pos`, `energy` (J the body lost), `intensity` (Pa), `result` (unaffected, damaged, destroyed), `contact` (the element's striking surface, if it has one) | A body struck a floor, block or fixture (section 15.2). Its cause, and that of the three below, is the body's doing |
| `BodyHitOccupant` | `body`, `owner`, `occupant`, `loc`, `pos`, `packet` (kind, energy, intensity, cause: the event's cause is the packet's), `vel` (the body's as it struck), `push` (m/s the occupant takes if free to move) | A body struck an occupant (section 15.2) |
| `BodyLanded` | `body`, `owner`, `loc`, `pos` | A body came to rest and is a body no longer (section 15.3) |
| `BodyLeftLoaded` | `body`, `owner`, `loc` (the last loaded cell it was in), `lands` (where it comes down or back in, if it does), `at` (when), `returns` (it comes back in and flies on), `energy` (J as it arrives) | A body's flight took it out of loaded space and was finished by arithmetic (section 15.4) |

The three tract events carry no cause of their own: `cause()` gives the
coarse cause for them. An event is kept small, at most 176 bytes, since
the simulation holds it inline beside small things: what would make one
larger is boxed, as `BodyHitOccupant`'s packet and `BodyImpact`'s contact
are. What a field does of itself has the cause kind
`Field` with, as instigator, whoever is responsible for the field in
that cell (section 16).

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

pub struct TractRecord {                      // 24 bytes; a surveyed region is about 100 KB
    pub def: TractDefId,                      // what kind of place this is
    pub rot: u8, pub variant: u8,
    pub state: TractState,                    // Surveyed | Realized { pristine: bool }
    pub devastation: u8,                      // section 17
    pub devastation_kind: Option<DevastationKindId>,   // of whichever delivered the most; the later wins a tie
    pub site: Option<SiteIdx>,                // a position in its region's `sites`
    pub extra: Option<Box<TractExtra>>,
}

pub struct TractExtra {
    pub pending: Vec<PendingEdit>,            // applied in order at next load
    pub stamp_watermark: u64,                 // stamps up to this sequence are already applied
    pub last_loaded: Tick,                    // when it was last unloaded, in the local time of its realm's clock
    pub vars: VarMap,
    pub generated: Option<Hash128>,           // the generator fingerprint its cells were last generated with (section 11.3)
}
```

A realm keeps a region's record from the moment the region is surveyed, in
`Realm::regions`. `Cosmos::region` and `Cosmos::tract_record` read them;
they are state, hashed and saved with the realm. A tract of an unsurveyed
region has no record.

What the records carry that later packages give meaning is typed and
stored now: `RegionSurvey` (a kind and field values), `Placement` as 10 §4.1
gives it, `PathLine` (a kind and the cells it runs through), and `Stamp`
(a tick and a sequence number; its release joins it with section 17).
`TractDefId` is the dense id of a `tract` definition (10 §6), and 0 is
the engine's `engine:none`: a tract with no definition.

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

pub struct RealmInfo {                        // plain facts; it holds no cells
    pub id: RealmId, pub def: RealmDefId, pub seed: u64, pub base_grid: GridId,
    pub topology: Topology, pub below_fill: BlockState, pub above_fill: BlockState,
}

pub struct RegionSurveyResult {
    pub record: RegionRecord,
    pub sites: Vec<SiteSpec>,                 // made by the world and listed in the record
}

pub struct RealizedTract {
    pub column: Column,                       // chunks and the two fills, built with a ColumnBuilder
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

**Where the interfaces live.** In `clash-world` (`clash_world::generate`),
because loading a tract calls them and the world cannot depend on the crate
that implements them (01 §3). `clash-gen` implements them and re-exports
them.

**Enforced by types.** Every input of a realiser is definitions, plain facts
or records: none holds a cell, a grid or the cosmos. A generator is a
`WorldGenerator`: a surveyor and a realiser that is `Send`, `Sync` and
`'static`, so it cannot borrow the world either. The records a realiser is
shown are as the survey wrote them, with the devastation: how far along its
life a tract is, and what is waiting for it, are left out
(`TractRecord::as_surveyed`), so that what is generated cannot depend on
which tracts happen to have been loaded before. A neighbour beyond an edge
of the realm has no record.

**Choosing a generator.** A realm definition names its generator and may
give it parameters, which the realm carries as written and the generator
reads:

```json
{ "type": "realm", "id": "plain", "generator": "flat",
  "generator_params": { "surface_z": 0, "floor": "grass" } }
```

Generators register a factory by name in a `GeneratorRegistry`
(`clash_gen::register_generators` adds this crate's).
`GeneratorRegistry::bind(defs)` builds the generator of every realm
definition and gives a `Generators`, which is what loading takes. A name
nobody registered, a missing generator, an unknown parameter, a parameter
out of range and a state that does not exist are each a `GenError` naming
the realm definition and the parameter's path (`generator_params.wall`). A
realm whose generator cannot be had is refused when one of its tracts is to
be generated, not before: a world may hold realms that are never generated.
Parameters are checked when the generators are bound;
`Generators::param_errors` lists what is wrong.

A generator built from definitions the world's tables do not hold (maps,
grounds, tract definitions) registers a closure that owns them
(`GeneratorRegistry::register_maker`), which takes the place of a plain
factory of the same name. `clash_gen::generators_with(defs)` binds every
generator of that crate this way, the `standard` one of 10 §6.2 among
them.

**In parallel.** `realize_batch(defs, runner, jobs)` realises a batch of
tracts and returns the results in the order of the jobs. A `BatchRunner`
decides on which threads: `SerialRunner` on the calling thread, and
`clash_gen::RayonRunner` on a pool. Each job is a pure function of what it
holds and each result has a place of its own, so the outcome is the same
for every order and every thread count (invariant W6).
`clash_gen::realize_tracts` does this without a cosmos, for tools and
tests.

Document 10 specifies the real generators and the JSON map format. The world
model ships two test generators, used by every destruction test. Neither
names content: every state comes from a parameter (invariant W9).

| `flat` parameter | Meaning | Default |
|------------------|---------|---------|
| `surface_z` | The lowest storey that is not ground | 0 |
| `ground` | The block state below the surface | The realm's `fill.below` |
| `floor` | A floor state laid over the surface | None |

`test_city` is a grid of streets with sidewalks, and on every block a
building that fills its lot: outer walls with a window in every second
cell and one or two doors onto the sidewalk; a slab on every storey and a
roof; two partitions that cross, with a door in each of their four
stretches; pillars; a flight of stairs from every storey but the top one,
with an opening in the slab above it; and a few desks. What a building is
like is drawn from the positional hash of its block of the street grid, so
a building that straddles two tracts agrees with itself.

| `test_city` parameter | Meaning | Default |
|-----------------------|---------|---------|
| `surface_z` | The storey of the streets and the ground storeys | 0 |
| `street_spacing` | Cells from one street to the next | 16 |
| `street_width`, `sidewalk_width` | Cells across a street; across a sidewalk | 3; 1 |
| `storeys` | The fewest and the most storeys of a building, as a list | `[1, 3]` |
| `column_spacing` | Cells between pillars | 4 |
| `ground` | The block state under the surface | The realm's `fill.below` |
| `street`, `sidewalk`, `slab`, `roof` | Floor states | required |
| `wall`, `window`, `door`, `partition`, `pillar`, `stairs` | Block states | required |
| `desk` | A fixture state | required |

`cargo run -p clash-gen --example city_ascii -- [seed] [storey]` prints
one storey of a block of it as plain characters.

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

```rust
pub struct TractIo<'a> {                      // what loading needs besides the world
    pub generators: &'a Generators,
    pub store: &'a mut dyn TractStore,        // section 11
}

impl WorldCtx<'_> {
    pub fn load_tracts(&mut self, io: &mut TractIo<'_>, tracts: &[(RealmId, TractPos)])
        -> Vec<Result<LoadedTract, LoadError>>;       // one outcome for each tract, in the order given
    pub fn load_tract(&mut self, io: &mut TractIo<'_>, realm: RealmId, tract: TractPos)
        -> Result<LoadedTract, LoadError>;
    pub fn unload_tract(&mut self, io: &mut TractIo<'_>, realm: RealmId, tract: TractPos)
        -> Result<(), LoadError>;
}

pub struct LoadedTract {
    pub realm: RealmId, pub tract: TractPos,
    pub first: bool,                          // never loaded before
    pub from_store: bool,                     // read, not generated
    pub spawns: Vec<SpawnRequest>,            // for the simulation
    pub areas: Vec<AreaId>,                   // made from what the generator asked for
    pub entities: Vec<u8>,                    // the simulation's entity section, handed back once (section 11.1)
    pub regenerated: bool,                    // generated again from other inputs: new content (section 11.3)
}
```

As built:

- A batch is planned, realised and installed. Planning surveys the regions
  of each tract and of its neighbours and decides where its cells come
  from. The tracts that need generating are realised together (section
  9.3). Then each tract is installed in the order given: its column joins
  the grid, the changes waiting for it are made, and `TractLoaded` is
  emitted.
- A tract is read from the store when the store holds cells for it:
  always when its record says `Realized { pristine: false }`, and for a
  pristine tract when everything is stored. Otherwise it is generated
  (section 11.3). The entity section the store holds for it is handed
  over either way.
- What a generator asks for besides cells is acted on once, the first time
  a tract is loaded: its areas are made (they are the cosmos's and outlive
  the loading) and its spawn requests are handed over. A pristine tract
  generated again gives its cells only.
- Pending cell changes are made through `WorldEdit` with the coarse cause,
  so they emit `ElementChanged`, ask for stability checks and move
  occupants like any edit, and they make the tract one that is stored. A
  pending spawn is handed over with the tract. Both are then gone from the
  record. A pending stamp stays until section 17 is built.
- A pending map stays in the record too: the world cannot resolve a map
  (01 §3). `Cosmos::take_pending_maps(realm, tract)` hands the maps
  waiting for a tract to whoever holds the maps, and
  `clash_gen::apply_pending_maps` writes them through `WorldEdit` with
  the coarse cause (10 §4.3). Whoever loads tracts calls it for each
  tract loaded, so a pending map is written after the pending cell
  changes of the same load.
- A tract that is loaded already, or named twice in one batch, is refused,
  as is one outside its realm. A refusal changes nothing and says why
  (`LoadError`).
- Step 4 runs for a tract read from the store, by each field's
  `catch_up` (section 16), and the fields still active are stepped
  again. A tract that is generated holds no field.
- Not built yet: stamps and total devastation (section 17).

`Cosmos::add_pending_edit(realm, tract, edit)` is how a change is left for
a tract that is not loaded. It is refused for a tract that is loaded, a
tract whose region has not been surveyed (`Cosmos::survey_region` surveys
one), and a cell change that names a cell of another tract.

### 9.5 Unloading a tract

1. Emit `TractUnloading`. The simulation removes every occupant: persistent
   entities are stored with the tract, important characters become cold
   agents, anonymous population returns to statistics.
2. The world asserts the tract has no occupants, bodies or processes.
3. Dirty chunks are written. `last_loaded` is set.

A tract with a process still running in it is not unloaded; the process holds
a ticket (section 10).

The world cannot remove entities and never calls upward (01 R4), so the
three steps are not one call:

- `unload_tract` is refused, changing nothing and emitting nothing, while
  any occupant has a cell in the tract (`LoadError::Occupied`, naming one).
  On a tract with no occupant it emits `TractUnloading` and unloads at
  once.
- Reconciliation (section 10.2) is what brings the two together. For a
  tract that is to go and holds occupants it emits `TractUnloading` and
  leaves the tract loaded. The simulation answers the event by taking its
  entities out, and the next reconciliation unloads the tract without a
  second event.

A column that has changed since it was loaded is written whole, and its
record becomes `Realized { pristine: false }`. One that has not is not
written, unless everything is stored or the simulation handed in an
entity section for the tract (section 11). The checks for bodies and
processes come with them (sections 14 and 15).

The simulation's side of this, and of `TractLoaded`, is 03 §4.3: it
reconciles at every safe point, shelves the entities of a tract that is
to go and puts them back when the tract is loaded again.

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

```rust
impl Cosmos {
    pub fn hold_ticket(&mut self, ticket: Ticket) -> Result<TicketId, WorldError>;
    pub fn release_ticket(&mut self, id: TicketId) -> Option<Ticket>;
    pub fn move_ticket(&mut self, id: TicketId, to: TractPos) -> Result<(), WorldError>;   // a fixed anchor
    pub fn ticket(&self, id: TicketId) -> Option<&Ticket>;
    pub fn tickets(&self) -> &TicketTable;
    pub fn activity(&self, realm: RealmId, tract: TractPos) -> Option<Activity>;
    pub fn set_max_loaded_tracts(&mut self, max: usize);
}
```

- A ticket reaches `radius` tracts from its anchor, counted the longer way
  along either axis; a hot ticket reaches `WARM_MARGIN` further, and the
  tracts of that margin are warm. A tract beyond an edge of the realm is
  not covered, and on a wrapped axis a ticket reaches round.
- Holding a ticket loads nothing and releasing one unloads nothing:
  reconciliation does. A ticket id is never reused.
- Anchors built so far are `Fixed` and `Entity`. A ticket anchored to an
  entity is centred on the tract its occupant is in, follows it, and takes
  on its realm when it crosses into another. While the entity is not in
  the world the ticket stays where it was last, and one that has never
  been seen covers nothing. A body holds a ticket with a fixed anchor
  and moves it ahead of itself with `move_ticket` (section 15.1): a
  `Body` anchor is not needed. A `Subgrid` anchor comes with subgrids.
- `TicketReason::Eoc` carries the rule's number, which the world gives no
  meaning.
- `Cosmos::activity` gives a tract's level as of the last reconciliation:
  the highest of the tickets covering it, warm for a tract kept only by the
  unloading margin, and nothing for a tract the tickets do not keep.

### 10.2 Reconciliation

At safe points the cosmos compares the tracts tickets want with the tracts
loaded and loads or unloads the difference. Order is by priority, then
distance to the ticket anchor, then `TractPos`, so it is deterministic.

```rust
impl WorldCtx<'_> {
    pub fn reconcile_activity(&mut self, io: &mut TractIo<'_>) -> Reconciled;
}

pub struct Reconciled {
    pub loaded: Vec<LoadedTract>,             // in the order they were loaded
    pub unloaded: Vec<(RealmId, TractPos)>,   // by realm, then row by row
    pub refused: Vec<RefusedTicket>,          // { id, ticket }: dropped for the budget
    pub expired: Vec<RefusedTicket>,          // lapsed at their tick
    pub waiting: Vec<(RealmId, TractPos)>,    // to be unloaded, and still occupied
    pub failed: Vec<TractFailure>,            // { realm, tract, error }: could not be loaded or unloaded
}
```

The simulation calls it at every safe point. In order:

1. Tickets whose `expires` has come are dropped, and tickets anchored to an
   entity follow it. If nothing has changed since the last call it returns
   here: an idle call costs a look at each ticket.
2. The budget (section 10.3).
3. Unloading. A tract is loaded at a ticket's reach and unloaded only once
   it lies more than `UNLOAD_MARGIN` (1) tract beyond every ticket's reach,
   so an anchor pacing across a tract boundary loads and unloads nothing. A
   tract with no occupant is unloaded at once. One that holds occupants is
   announced with `TractUnloading`, once, and reported as waiting until a
   reconciliation finds it empty (section 9.5). If it is wanted again by
   then it is still unloaded, and loaded again in the same call with
   `TractLoaded`, so the simulation sees the two events in pairs; if it is
   wanted again and nobody has left, it simply stays.
4. Loading what is wanted and not loaded, by priority (highest first), then
   distance from the anchor of the ticket of that priority nearest to it,
   then position, then realm. The batch is realised in parallel
   (section 9.3).

A tract that could not be loaded is reported with the reason and tried
again when something changes.

Reconciliation counts and unloads only the tracts the loading procedure
loaded. A column put into the world by `test_support` is outside it.

### 10.3 Budget

`max_loaded_tracts` (default 1600) bounds memory. When tickets want more,
the lowest-priority tickets are refused. A process that loses its ticket is
told to **force-resolve**: finish instantly in resolve mode and record the
result. A body that loses its ticket is resolved coarsely: its landing point
is computed, damage along its path becomes a stamp, and its owner is placed
at the destination as a cold agent or pending spawn.

- While the tracts the tickets cover outnumber the budget, one ticket is
  refused: the lowest priority, and of equals the one made last. A ticket
  of priority 255 is pinned and never refused.
- A refused ticket is gone from the table. Its holder is told twice over:
  by `TicketRefused`, and in what reconciliation returns, where the
  ticket's `reason` says who held it. Forcing a process or a body to
  resolve is the holder's part (sections 14 and 15).
- Tracts kept only by the unloading margin are the first to go when room is
  short.
- If pinned tickets alone want more than the budget, none is refused and
  the budget still holds: the tracts that do not fit, last in loading
  order, are not loaded, are reported as failed with `LoadError::OverBudget`
  and are tried again at the next call.

With defaults, the player's ticket loads 17 × 17 tracts (544 m across). The
budget leaves room for about four more such areas.

## 11. Persistence

- A save is kept behind a trait:

```rust
pub trait TractStore {
    fn get(&self, realm: RealmId, pos: TractPos) -> Result<Option<Vec<u8>>, StoreError>;
    fn put(&mut self, realm: RealmId, pos: TractPos, blob: &[u8]);
    fn delete(&mut self, realm: RealmId, pos: TractPos);
    fn commit(&mut self) -> Result<(), StoreError>;        // atomic
    fn section(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn put_section(&mut self, name: &str, blob: &[u8]);
    fn delete_section(&mut self, name: &str);
    fn section_names(&self) -> Result<Vec<String>, StoreError>;   // in order
}
```

  It holds tract blobs by realm and tract, and named sections for
  everything of a save that is not a tract. The default implementation
  uses `redb` (`RedbStore`). Tests use an in-memory map (`MemoryStore`).
  `put` and `delete` take effect for later reads at once; `commit` makes
  what was done since the last commit durable, all of it or none.
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

### 11.1 The tract blob

Kind `TRCT`, version 2. `encode_tract(defs, ids, cells, entities)` writes
one and `decode_tract(defs, ids, blob)` reads it. The body is two fields:

| Field | Holds |
|-------|-------|
| `cells` | Optional. The column: its two fills, the chunk height of its first stored chunk, and its chunks without those at either end that hold only the fill; each chunk is its three element layers with compacted palettes and its four sparse layers. Absent for a tract whose cells are generated again |
| `entities` | The simulation's entity section, as bytes |

Equal columns written against equal tables give equal bodies. Version 1
held dense ids and no entity section and was never written to disk; it
is refused, with the version it states.

**The entity section** is bytes the world neither reads nor drops. The
simulation hands it in for a loaded tract with
`Cosmos::set_tract_entities(realm, tract, bytes)`, when it answers
`TractUnloading` by taking its entities out (section 9.5). The world
writes it with the tract when the tract is unloaded or the world is
saved, and hands it back in `LoadedTract::entities` when the tract is
next loaded. It is handed over once: as it is, the section is taken out
of the store, so that what the simulation then holds live is not also
waiting in the tract. A tract that has never been changed and has an
entity section is stored as that section alone.

**Fields in the blob.** Each chunk also holds its fields (section 16): for
each field, the cells that hold it with their level, whether they are
active and who is responsible. A chunk that holds a field is never one
that "holds only the fill", and a change to a field marks its tract as
one to store.

### 11.2 Save ids and placeholders

A cosmos keeps the tables of its save (`SaveIds`): one for each kind of
definition that saved state names (`DefKind`: `floor`, `block`,
`fixture`, `material`, `area_kind`, `realm`, `damage_type`, `plane`),
each the qualified ids in the order the save first named them. Save id 0
of each element type and of materials is the built-in `engine:none`.

- Writing turns dense ids into save ids and adds a definition to its
  table the first time the save names it. The tables only grow.
- Reading turns save ids into the dense ids of the definitions loaded
  now. A save id its table does not hold is damage, and an error.
- The tables are not state and are not hashed. They are written with
  the world, in `world.ids`, each under the name of its definition
  type, so that a build with more kinds reads the saves of one with
  fewer and keeps the tables of kinds it does not know.
- Everything the world saves is walked for the ids it holds: element
  states in cells, fills and waiting changes, a realm's definition, an
  area's kind, the materials of a falling mass, the material of a
  fragment in flight, the damage type of a projectile and of a release
  in progress, the planes a body is on, and what a body that came down
  unseen was (a waiting impact, section 15.4).
- A set of planes is a mask with a bit for each plane's id, and is
  turned plane by plane. The mask of every plane names none of them and
  is written as it is. A mask has thirty-two bits, so a save can name
  thirty-two planes over its life, those that have gone included.
- Written as they are, because their definition types are not built
  and the world gives the numbers no meaning: decal, conduit, site,
  tract, devastation kind, map, region kind and path kind ids. A named
  state of an element is written as its position among the
  definition's states.

**Placeholders.** When a save is loaded, an id whose definition is no
longer there (04 §12.3) is given a placeholder: a definition of the
right type, added after every real one in a copy of the definitions,
which keeps the id the save wrote as its own. `Cosmos::defs` of a loaded
world holds them; every definition that was passed in keeps its dense
id. A placeholder is written back under the id it keeps, so a save made
without the mod loses nothing, and when the mod is loaded again the id
finds its definition and the content is as it was. A placeholder is not
found by name: a reference in data to something that does not exist
stays a mistake. What a placeholder is like is the engine's, and the
same for every id:

| Type | The placeholder |
|------|-----------------|
| `floor` | No thickness. It can be stood on |
| `block` | No volume; `load_bearing`, `anchor` and `passable`: it does not fall, holds up what stood on it, and is neither seen through nor walked into |
| `fixture` | No volume |
| `material` | No density, no toughness, the default resistance of every damage type, never crushed, and a span that reaches as far as a check looks |
| `area_kind` | Nothing: an area keeps the tags, variables, clock and priority it was made with |
| `realm` | No strata, no generator, barrier edges, and the extent the realm was saved with. Its stored tracts load; nothing more is generated |
| `damage_type` | A default resistance that nothing gets past: it harms no element. It is not `kinetic` and leaves no devastation. A release or a projectile of it goes on, and what it does to an occupant is for the simulation |
| `plane` | Nothing: a plane with nothing on it but what the save put there. It takes the next free bit, and a save that names more planes than a mask has bits is refused |

Each element placeholder is drawn as `?` and carries the tag
`placeholder`, as a placeholder material, damage type and plane do. An element that is a
placeholder, or is made of one, has no capacity: the first blow that
takes effect destroys it, and it leaves no rubble. Destroyed, it is
gone for good.

`Cosmos::take_save_warnings` lists each missing definition with where
it was met: among the save's ids, in a section of the world, or in the
cells of a tract, as the tract is read. Migration entries (04 §12.1)
are consulted before a placeholder is made, once the `migration` type
is built.

### 11.3 The pristine policy

- A tract's record says `Realized { pristine: true }` until the tract
  is unloaded with cells that have changed since they were generated or
  read. Such a tract writes no blob, when it is unloaded and when the
  world is saved, unless it has an entity section.
- **The fingerprint.** `Cosmos::generator_fingerprint(realm)` is a hash
  of what a realm's tracts are generated from: the content fingerprint
  of the definitions (04 §9; `WorldDefs::fingerprint`), the generator
  the realm's definition names with its parameters, and the realm's
  seed. A tract's record keeps the fingerprint its cells were last
  generated with (`TractExtra::generated`). When a pristine tract is
  generated again under another fingerprint, its cells are new content:
  `LoadedTract::regenerated` says so, and the record takes the new
  fingerprint. Only the cells are made again. The areas and spawn
  requests of the first generation were acted on then and are not
  repeated, and whatever the simulation kept for the tract may no
  longer fit what stands there.
- **`save.store_pristine`** (`Cosmos::set_store_pristine`, off by
  default) stores every tract when it is unloaded and every loaded
  tract when the world is saved. A tract stored this way stays
  pristine in its record. Cells that are in a tract's blob are what the
  tract loads with, whatever the setting is now and whatever the
  fingerprint says: with everything stored, the world is kept as it
  was seen, and content that has gone shows as placeholders.

### 11.4 Saving and loading the world

`Cosmos::save_world(store)` writes the whole world into a store, and
`Cosmos::load_world(defs, io)` makes a cosmos of what a store holds. The
cosmos that comes back hashes equal to the one that was saved, and the
two go on alike (invariant W7).

| Section | Kind | Holds |
|---------|------|-------|
| `world.cosmos` | `WCOS` | The realms with their region and tract records, the table of grids, the global variables, and `store_pristine` |
| `world.areas` | `WARE` | Every area |
| `world.sites` | `WSIT` | Every site |
| `world.tickets` | `WTKT` | The tickets, which tracts they keep, and what the last reconciliation worked out |
| `world.stability` | `WSTB` | The stability checks waiting |
| `world.activity` | `WACT` | The world's processes (settle checks, falling masses, field steps, bodies in flight, releases whose fronts are spreading), the collapses in progress, and what is kept about fields, bodies and releases beside them |
| `world.tracts` | `WTRC` | Which tracts are loaded, and for each whether its cells are stored and whether it had changed |
| `world.ids` | `WIDS` | The tables of save ids |

Each is a blob with a kind and a version of its own, at version 1.
The sections are a list in the code (`WORLD_SECTIONS` in
`clash_world::save`): a world subsystem that has state adds an entry
there, with a function that writes it and one that puts it back. A
section a save does not hold leaves its subsystem as it is in a new
cosmos. Names that begin with `world.` are the world's: a section of
that prefix which this build does not know is refused when loading,
because loading would drop it, and one that a save does not write is
deleted from the store. The simulation's sections go beside them under
other names.

The cells of loaded tracts go where the cells of unloaded tracts are. A
loaded tract that has changed, or that no generator can make again, is
written as its tract blob. A loaded tract that has not changed is not
written: `load_world` generates it again, with the generators it is
given, as any pristine tract is. Saving changes nothing that is hashed
and unloads nothing.

**Saving**, at a safe point:

1. The simulation hands in the entity section of any tract it is about
   to let go of.
2. `cosmos.save_world(store)`.
3. The simulation writes its own sections: the timeline with its
   clocks, its entities and the rest.
4. `store.commit()`.

A store is committed only after `save_world`: tract blobs written
between saves name definitions through tables that `save_world` writes.

**Loading**, in this order:

1. The mods are loaded and the world's definitions built.
2. The generators are bound to them.
3. `Cosmos::load_world(&defs, &mut TractIo)`. From here on the
   definitions to use are `Cosmos::defs`, which may hold placeholders.
4. The timeline is restored from the simulation's section. The world
   does not touch the timeline while it loads: the clocks of realms and
   areas and the wake-ups of its processes are in the timeline as it
   was saved, and the cosmos holds their ids. The two must come from
   the same save.
5. The simulation places its entities again with `WorldCtx::place`, into
   an event sink that is thrown away. Placing changes nothing that is
   hashed.
6. `Cosmos::take_save_warnings` says what has become a placeholder.

Loading reads the store and changes nothing in it. Anything missing or
damaged is a `SaveError` that names the section or the tract.

### 11.5 The file

`RedbStore::open(path)` opens a save file, or makes an empty one. The
file is a `redb` database with two tables: tract blobs keyed by realm,
row and column, and sections keyed by name. What is put and deleted is
kept in memory until `commit`, which writes it in one transaction, so
the file holds all of one save or all of the one before it, and a game
that is not saved leaves its file as it was. The file is checked as it
is opened: one that has been cut short or written over is an error, as
is any blob in it that fails its own checksum. The store is read and
written only at safe points (01 R5).

Not built: region records as blobs of their own (`world.cosmos` is
written whole at every save); migration entries; the columns of grids
that are not a realm's base grid (section 18.5); benchmarks.

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
fixture.

**Debris.** An element destroyed by a packet with a direction
(`damage_along`) and energy to spare throws fragments:

- up to three (`DEBRIS_PER_ELEMENT`), fewer when fewer than that are
  left of the budget: at most `debris.max_bodies` (256) fragments are in
  flight at once, and past that what is destroyed is left whole;
- a quarter of the mass the element lost between them
  (`DEBRIS_MASS_SHARE`), the rest left as rubble where it stood;
- a hundredth of the energy the packet had left over between them
  (`DEBRIS_ENERGY_SHARE`), as the energy of their motion. It is taken
  out of what passes and counted as absorbed, so that `absorbed + passed`
  is still the packet's energy (W8). Whatever the packet goes on to, a
  body or a blast, goes on with the rest;
- each from the middle of the cell, along the direction of the hit
  turned by up to 0.35 of a unit on each axis (`DEBRIS_SCATTER`), by a
  positional hash of the cell, the tick and which fragment it is.

A fragment is a body (section 15) on every plane, with the cross-section
of a lump of its volume. It strikes what it meets and is left as rubble
of its material where it comes to rest. A blow with no direction, rubble
that is destroyed, an element that fell or was crushed and a wreck that
is carried off by what broke it (section 14.3) throw none.

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
    pub shape: ReleaseShape,          // Sphere | Cone { dir, half_angle } | Beam { dir, radius, range, burst } | Front { … } (section 13.5)
    pub speed: Option<Real>,          // m/s of the front; none takes the local sound speed
    pub cause: Cause,
}

impl WorldCtx<'_> {
    pub fn release(&mut self, r: EnergyRelease) -> Result<ReleaseId, WorldError>;
}
```

`release` is the single entry point for explosions, shockwaves from clashes,
energy blasts and beams at every scale. It decides, per tract, whether to act
in detail or coarsely.

- A release's `kind` is any damage type: a blast, heat, cold, whatever the
  definitions hold. The kind decides which threshold each thing meets it
  with, and nothing else. There is no definition type for releases.
- `EnergyRelease::sphere(origin, kind, energy, cause)` is a release in
  every direction from the middle of a cell, 1.5 m up.
- With no `speed` the front moves at the speed of sound of the atmosphere
  at its origin, and in vacuum at `release.vacuum_speed` (1000 m/s).
- Refused: energy that is not above zero and finite, a kind that does not
  exist, an origin that is not loaded, a speed that is not above zero, a
  cone with no direction or a half angle not above zero and at most π, a
  beam with no direction, radius or range.
- `ReleaseStarted` is emitted at once, with the origin, the kind, the
  energy and how far it will reach with nothing in the way.
  `EnergyReleased` is emitted when the front has gone as far as it goes,
  with what the release came to (section 7.4).
- What a release does carries its doing as cause: a process, with the
  actor and instigator of what let it go.

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

As built:

- **Shells.** Shell `k` is the cells whose centres lie from `k` up to
  `k + 1` metres from the origin point, a storey counting three. The
  process wakes for shell 0 at once and for each further shell `1 m /
  speed` of local time later, on the clock of its origin, and handles
  the cells of a shell storey by storey, row by row. Intensity is taken
  at the distance of the cell's centre.
- **The map** has six faces of `n × n` bins, with `n` chosen so that a
  bin at the reach is about a cell across: `⌈R · π / 2⌉`, at least 16 and
  at most 48. A box, of a cell or of an element, covers the bins whose
  middles lie in the rectangle that bounds it on each face, or else the
  one bin its own middle lies in. So things side by side share no bin
  and leave none between them, and whatever lies wholly behind a thing
  covers no bin the thing does not. `T` of a cell is the mean over the
  bins its box covers.
- **Shade falls when the shell is done.** Everything of one shell reads
  the map as the shells before left it: things at one distance stand
  side by side and do not shelter one another.
- **What an element covers.** A block or fixture covers its cell's
  footprint from the base up to its height, and closes
  `coverage × 3 / height` of the bins it covers, at most all (the share
  a ray meets, section 22). A floor covers a slab of its thickness at
  the base of its cell, and closes all of it.
- **Packets.** A block or fixture receives `energy = I × 3 m³ ×
  coverage`. A floor receives `I × 3 m³` times how squarely the wave
  meets it: the upward part of the direction from the origin to the
  middle of the floor. No element is offered more than the part of the
  release that is going its way (its energy times the share of all bins
  the element covers), nor more than the release has left. Each packet
  has the direction away from the origin, so that what it destroys
  throws its fragments outward (section 12.3).
- **A floor above the origin** is struck before the rest of its cell,
  and shelters it: what is in the cell receives what the floor let
  through. A floor at or below the origin is struck after.
- **Unaffected, damaged, destroyed.** An element that is not affected
  closes its bins and turns the packet back (see confinement). One that
  is damaged closes its bins and has taken the packet. One that is
  destroyed lets through the share of the packet that passed.
- **The account.** The release keeps what it has left: its energy less
  everything elements have absorbed, fragments' share included. A
  packet never offers more than that, and the release ends when nothing
  is left. So contract 1 holds exactly, and `EnergyReleased.absorbed` is
  the energy less what was left.
- **Occupants.** Each occupant of a reached cell is exposed once by the
  front, the first time a cell it is in is reached: `OccupantExposed`
  with the cell's intensity and `energy = I × 3 m³`, what the wave
  carries through a cell, and no more than was let go. An occupant
  whose cell the front has not reached when it leaves is not exposed.
- **Fields.** That same packet, the cell's intensity with what the wave
  carries through a cell, is what arrives in each cell the front
  reaches: the release calls `WorldEdit::conduct` for it once, however
  many of the cell's elements it strikes, and a field there that reacts
  to the kind of damage passes it on (section 16). A cell that holds
  only a field is reached like any other. So does each cell a beam
  passes through, and each cell of a closed space with what the space
  held; the elements that bound the space are pressed on and their
  cells are told nothing. What a field passes on is the field's doing
  with the release's cause: it is not taken from the release's energy,
  not counted in its totals or against `release.max_cells`, and every
  wet cell the front reaches passes its own packet on, so a cell of a
  pond is passed one from each wet cell within the reaction's reach.
- **The push.** The wave gives a thing the energy it holds in a slab of
  the thing's cross-section, `release.push_depth` (1 cm) deep:
  `speed = √(2 · I · area · 0.01 m / mass)`, straight away from the
  origin. A body in flight takes it as an impulse. An occupant has the
  cross-section of a lump of its mass (as `Body::of_mass`); if the speed
  is at least `release.throw_speed` (3 m/s, the `launch_at` of 07 §7)
  the world throws it: it becomes a body in flight (section 15) from a
  metre above the base of its cell, with `BodyLaunched`. Slower, it is
  left where it is: staggering is the simulation's. An occupant the
  world was told no mass for, and a device, are never moved. The
  simulation must not launch for the same exposure again.
- **Confinement.** What the front could not mark turned it back. When
  the front is done, if the space round the origin closes (the fill of
  section 8.3, within 4000 cells), the energy turned back, and no more
  than is left, is held in it: `I = held / (cells × 3 m³)`. If that is
  at least `world.min_intensity`, every occupant in the space is exposed
  to it once more, and every fixture in it, every block that bounds it
  and every floor over and under it receives a packet at that intensity.
  A space that does not close holds nothing: the wave has left through
  the opening. So a charge in a closed room does more than the same
  charge in the open, and an open door or a wall that fails lets it out.
- **Unloaded and beyond.** Cells that are not loaded or lie beyond an
  edge are passed over. Their stamps are the coarse layer's (section
  17).
- **The budget.** Before each shell, the cells it will visit (its volume
  over 3 m³) are added to those visited. If that passes
  `release.max_cells` (2,000,000) the release is **cut off**: that shell
  and those beyond are not played out, `EnergyReleased` says so with
  `cut_off` and the reach it got to, and confinement is still settled.
  Resolve mode for the shells left out comes with the coarse layer.
- **The tiers** are not built: every non-empty cell within reach is
  handled with shadowing, and the budget bounds the work.
- **A cone** reaches the cells whose centres lie within its half angle
  of its axis, and the cell of its origin.
- **A beam** handles each wake the cells whose centres lie within
  `radius` of its axis in the next metre of it. Each occupant there is
  exposed, each block and fixture receives `I × 3 m³ × coverage` and
  each floor that much times how steeply the beam climbs or dives, never
  more than the beam has left; what they absorb is taken from it. It
  ends where something that closes its whole cell stands after being
  struck, at the edge of loaded space, at its range or when nothing is
  left. With `burst`, what is left is let go there as a sphere, a
  release of its own.

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
(`Cosmos::processes`; section 11.4 saves it), and every one of their
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
    pub owner: BodyOwner,        // Entity(EntityId) | Debris { material, volume } | Projectile { kind, area, give, tag }
    pub grid: GridId,
    pub pos: [Real; 3],          // m
    pub vel: [Real; 3],          // m/s
    pub mass: Real,              // kg
    pub area: Real,              // m², cross-section
    pub volume: Real,            // m³, for buoyancy (11 §4.4)
    pub drag: Real,              // drag coefficient
    pub depth: Real,             // m, stopping distance for impact intensity; default 0.1
    pub restitution: Real,       // 0..1
    pub drive: Option<Drive>,    // { accel (m/s²), burn (s left), turn (rad/s) } (07 §11.1)
    pub plane: PlaneMask,        // it strikes only occupants that share a plane
    pub cause: Cause,
}

impl WorldCtx<'_> {
    pub fn launch(&mut self, body: Body) -> Result<BodyId, WorldError>;
    pub fn impulse(&mut self, id: BodyId, j: [Real; 3]) -> Result<(), WorldError>;   // kg·m/s
    pub fn arrest(&mut self, id: BodyId) -> Result<Body, WorldError>;     // caught, or recovered control
    pub fn steer(&mut self, id: BodyId, toward: Option<[Real; 3]>) -> Result<(), WorldError>;
}
```

Bodies are point masses with a cross-section. There is no rotation.

A body with a `drive` gains speed along its path while its thrust lasts,
and turns its path toward the point last given to `steer`, no faster than
its turn rate. The world does not know what it is aiming at. The
simulation tells it where to turn (07 §11.1). With no point it does not
turn.

**What a body is.**

| Owner | What it is | It strikes with |
|-------|-----------|-----------------|
| `Entity` | A creature thrown, an item hurled | The damage type flagged `kinetic`, over its own `area` and `depth` |
| `Debris` | A fragment of something destroyed: so much of a material | The same. Where it rests it is left as that much rubble |
| `Projectile` | A shot (07 §11.1), with a `tag` the simulation gives it and the world reports back | Its surface: a damage type, an area and a give |

`Body::of_mass` fills in a body whose size the world is not told: the
volume its mass has at 1000 kg/m³, the cross-section of a lump of that
volume, drag 1, depth 0.1 m, no bounce.

- `launch` is refused for a mass that is not above zero, a number that
  is not finite, a depth or restitution out of range, a fragment's
  material or a projectile's damage type that does not exist, a place
  that is not loaded and an entity already in flight. A bit of `plane`
  that names no plane is dropped, since nothing is on it; the mask of
  every plane stays as it is. `impulse`, `steer` and
  `arrest` are refused for a body that is not in flight, and the first
  two for one that is beyond loaded space (section 15.4).
- `impulse` and `steer` first bring the body up to now, then change it.
  `arrest` takes it out where it is now and emits nothing.
- An entity that is an occupant stays one while it is a body. The world
  carries it: whenever the body enters another cell the occupant is moved
  there with `relocate`, and each move is reported as
  `OccupantDisplaced`, as when an edit moves it. An occupant that fills
  several cells has the body at its middle. So someone in flight is where
  their body is, can be struck there (07 §7.1), and enters and leaves
  areas as they pass.
- What a body breaks carries its doing as cause: a process, with the
  entity it is as actor (or else the actor of what launched it) and the
  instigator of what launched it (section 7.3).

### 15.1 Motion

Each body is a process on the clock of where it is. It sleeps for the
time it takes to cross a cell at its speed, at least 10 µs and at most
20 ms of local time, so that gravity and drag integrate accurately. When
it wakes it has flown for the time it slept:

```
v   = v + ½ · g · dt                   (g: ambient gravity as a vector)
v   = v + thrust along v               (while the drive burns)
w   = v − drift                        (the medium's own movement, 11 §4.7)
w   = w / (1 + k · |w| · dt)           (k = ½ · ρ · drag · area / mass)
v   = w + drift + ½ · g · dt
v   turned toward the steered point by at most turn · dt
pos = pos + ½ · (v_before + v_after) · dt
```

With no drag this is exact for any step, so a throw in vacuum is the
parabola. The drag step is the closed form for drag alone and cannot
overshoot however light the body. Terminal velocity, vacuum and zero
gravity all follow from the strata. The straight stretch it covered is
then followed through the cells (section 15.2).

- A body wakes at ticks its own flight gives, and what it does at each
  is a function of its state and the world: nothing depends on how the
  timeline is advanced.
- Ambient conditions are read where the body is when it wakes. After
  each wake it goes onto the clock of the cell it is now in, and when an
  area's clock comes, goes or changes it does so at once: time stopped
  round a body leaves it hanging, and it resumes with the time it had
  left. A body that flies into stopped time stops within a step of the
  edge.
- Drift is what `Cosmos::ambient` reports for the cell. A flow is a
  velocity. A vortex turns about the middle of its area, from east toward
  south, at `speed`, draws inward at `inflow` and rises at `lift`.
- Where nothing pulls on a body (no gravity, no thrust, no drift worth
  the name) and it is slower than `REST_SPEED`, it has stopped: it
  hangs there and lands (section 15.3). A body is never state that never
  ends.
- A body holds a ticket ahead of itself: warm, radius 1, on the tract one
  tract's length ahead along its heading, when that tract is not loaded.
  The ticket has a fixed anchor that the body moves (section 10.1), and
  is released when the body ends.

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

- **The path** of one wake is a straight stretch, followed with the
  projectile ray of section 22. The first thing struck ends the stretch
  there: the body is put a hair's breadth short of what turned it back,
  or past what it went through, and flies on from there at its next
  wake. So an impact is dealt with at the first wake after it happened,
  never before: whoever has left the cell by then is not struck.
- **The direction of entry** is the axis along which the path entered
  the cell: east–west, north–south or up–down. A floor is met from above
  or below, and so is anything the path comes down onto inside its cell.
  Only the motion along that axis counts and only it is changed. A body
  that turned back within the stretch, and is already leaving what its
  path ran into, only touches it: nothing is struck and nothing is
  reported.
- **What closes part of a cell** (section 22) is struck by its share
  widened by the body's own width, `share + √area / 1 m`, on a roll
  drawn from the body's own seed and the cell: the same body meets the
  same thing the same way however often it is asked. What closes the
  whole width is always struck.
- **A projectile** delivers its surface's damage type at
  `KE / (surface area × give)`. Anything else delivers the damage type
  flagged `kinetic` at `KE / (area × depth)`; with none loaded it harms
  nothing and is still turned back.
- **Through.** Past a block or fixture it destroyed the body is in that
  cell. A floor lies between two cells: the body stays on the near side
  and meets what is in the cell beyond as it enters it. The fragments of
  what it destroyed take their share of what passed (section 12.3).
- **An occupant** is struck when the path enters its cell, if it is
  flagged `COLLIDES` and shares a plane with the body. `BodyHitOccupant`
  carries a packet of the whole energy of the body's motion at the
  intensity its surface gives, the body's velocity, and the `push`: the
  velocity the occupant takes if it is free to move. With the occupant's
  mass `M`, the body's `m` and restitution `e`, along the body's path:

  ```
  body:      v ← v · (m − e·M) / (m + M)
  push:          v · m · (1 + e) / (m + M)
  ```

  The world changes nothing else: what the blow does to the occupant,
  and whether it flies in turn, is the simulation's (07 §7, §11.1). A
  shot that should pass through is given what exits with `impulse`.
  Where the world was told no mass for the occupant there is nothing to
  push against: the body goes on as it was and the push is zero. The
  world knows no occupant's shape: whoever is in the cell is struck, at
  the height the event gives, once while the body passes.
- **`contact`.** `BodyImpact` carries the striking surface of the
  element struck, if it has one (section 5.4), for the simulation to
  strike the body with in place of the blunt impact (07 §6.5).
- **Fields.** The packet of a blow arrives in the cell of the element
  or the occupant struck: the body calls `WorldEdit::conduct` for it
  once, and a field there that reacts to the kind of damage passes it
  on (section 16). A dart that carries a shock into someone standing in
  water reaches whoever else stands in it.
- **An edge of the realm.** A barrier turns a body back like a wall
  nothing can break. By any other edge it leaves: `BodyLeftLoaded` with
  nowhere to land.

### 15.3 Landing

A body that hits a standable surface and cannot break it loses its vertical
velocity (times restitution). Remaining horizontal velocity decays by
friction as it slides, still colliding with obstacles. When it stops,
`BodyLanded` is emitted and the body is removed.

- A body comes to **rest** on whatever it came down onto and could not
  break, once its speed back up is below `REST_SPEED` (0.5 m/s). While
  it rests it slides: each wake it looks for what is under it, loses
  `friction × g × dt` of speed, and strikes what is in its way as in
  flight. `friction` is the `friction` of the material it lies on, or a
  half where the material states none. With nothing under it any more it
  falls again.
- It has **landed** when it is slower than `REST_SPEED` or friction
  would stop it within the step. A fragment is left as rubble of its
  material where something dropped there comes to rest (section 12.3).
  An entity is in the cell `BodyLanded` names, back in the simulation's
  hands. A projectile is gone: what it leaves, if anything, is the
  simulation's.

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

- The ticket is put on at most `body.max_ticket_tracts` tracts that were
  not loaded. A body that comes to space that is still not loaded when
  it arrives, for whatever reason, is finished by arithmetic from there.
- The arithmetic is the motion of section 15.1 in steps of 20 ms with
  nothing in the way, for at most about six hours of flight.
- **Coming back in.** As soon as a step ends in a loaded tract the body
  is a body again: it sleeps until the moment it arrives there and then
  flies on in full. Until then it is `away`: it is listed, and cannot be
  pushed or steered. An occupant it is waits in the last loaded cell and
  is carried on when the body wakes.
- **Coming down.** Otherwise it comes down where it reaches the ground,
  and is a body no longer. Its impact is added to that tract's pending
  edits as `PendingEdit::Impact`: the cell, the tick, what it was, its
  mass, and the velocity and energy it arrived with. It stays there
  when the tract loads, for the coarse layer to play out (section 17).
  A tract whose region has never been surveyed has no record to keep it.
- **The ground** is taken to lie where it did under the last loaded cell
  the body was in: the tract records keep no heights yet.
- `BodyLeftLoaded` says which: where and when it comes back in or comes
  down, whether it `returns`, and the energy it arrives with. The
  simulation takes an entity that does not return out of the world and
  leaves it for that tract (06 §12).

## 16. Fields

A field is a per-cell value from 0 to 255 that evolves by rule. The
engine knows what a field is and what each rule does. It never knows
what fire or water is: every behaviour below comes from a definition.

```json
{
  "type": "field",
  "id": "fire",
  "step": "500 ms",
  "rules": [
    { "rule": "decay", "amount": 12 },
    { "rule": "spread", "needs": "fuel", "chance": 0.3, "amount": 30, "from": 60 },
    { "rule": "emit", "field": "smoke", "amount": 12 },
    { "rule": "damage", "kind": "heat", "energy_per_level": 10, "intensity_per_level": "10 kPa" },
    { "rule": "drift", "share": 0.0, "spread": 0.25 }
  ],
  "catch_up": "burn_out",
  "light": { "intensity": 1.5, "radius": "10 m", "from": 20 },
  "glyph": { "ch": "^", "fg": ["red", "yellow", "white"] },
  "tags": ["light_source"]
}
```

| Field of `field` | Meaning |
|------------------|---------|
| `step` | How often the field is stepped, in the local time of where it is. Required; at least 1 ms. |
| `rules` | Its rules, applied to each active cell each step, in the order written. |
| `catch_up` | What becomes of it in a tract loaded after time has passed: `keep` (the default), `none`, `burn_out`, `disperse` or `settle`. See below. |
| `warms` | The kelvin each level adds to the temperature of its cell (11 §4.6), negative to cool. Stored and reported; it does nothing until temperature is built. |
| `changes` | What it becomes by temperature, like a material's (section 5.4): lava that cools leaves rock. Carried as written until change of state is built (package W25). |
| `light` | The light it gives off: `intensity` is that of a cell at level 255, and a lower level gives in proportion; `radius` as for an element (15 §3.3); `from` is the least level that gives light, default 1. |
| `opaque_from` | The level from which a cell of it hides what is behind it, as something opaque taller than the eye would. Absent: it never hides. |
| `glyph` | How it is drawn: `ch`, and `fg` and `bg` as one colour or a list of colours spread evenly from the lowest level to the highest. Stored and reported only. |
| `tags` | Free-form tags. |

**Rules.** Implemented natively and configured by data. Each is an
object with `rule` and that rule's parameters; a parameter the rule does
not take is a load error. A rule that moves the field moves whole
levels, rounded down, and only what the rules before it in the list
left in the cell: `decay` then `drift` then `diffuse` is not the same
field as the three in another order. Where a rule would move more than
is left, each neighbour gets its share of what there is.

| Rule | Parameters | Behaviour |
|------|------------|-----------|
| `decay` | `amount` (levels, required) | Lose `amount` each step. |
| `diffuse` | `rate` (required, more than 0 to 0.25); `rise` or `sink` (a share, more than 0 to 1) | Where a neighbour's level is lower, `rate` of the difference moves to it: to the four cells beside and, with neither `rise` nor `sink`, to those above and below. With `rise`, that share of the level moves to the cell above each step, as far as there is room, and nothing goes down; `sink` is the mirror. |
| `flow` | `rate` (more than 0 to 0.25; default 0.25) | Liquid behaviour. All that fits falls to the cell below. What cannot fall levels out sideways as `diffuse` does. |
| `spread` | `needs` (a material tag); `chance` (0 to 1, default 1); `amount` (the level a neighbour gets, default 1); `from` (default 1) | A cell at `from` or more may ignite each of its six neighbours that holds none of the field, each with `chance` a step. With `needs`, only a neighbour that holds a floor, block or fixture of a material carrying that tag. |
| `emit` | `field`, `amount` (both required); `from` | Add `amount` to another field in the same cell each step. |
| `convert` | `with` (a field, required); `uses`, `gives`, `max` | React with another field: shorthand for a reaction between this field and `with` (below). |
| `damage` | `kind` (a damage type), `energy_per_level` (J), `intensity_per_level` (Pa) (all required); `from` | Each step, send a packet of `level × energy_per_level` at `level × intensity_per_level` to the floor, the block and the fixture of the cell, and tell each occupant in it with `OccupantExposed`. |
| `drift` | `share` (0 to 1, default 1); `spread` (0 or more, default 0) | Be carried by the movement of the medium (11 §4.7): each step the share of the level that the medium carries across the cell, `speed × step / size × share` and at most all of it, moves to the next cell that way, along each axis, as far as there is room. The chance of this field's `spread` to a neighbour is multiplied by `1 + spread × v`, `v` being the metres a second the medium moves toward that neighbour, negative when it moves away, and never below nothing. |
| `expose` | `pool`, `per_level` (both required); `route` (`breath`, `skin` or `any`; default `any`); `from` | Tell each occupant in the cell of a dose for one of its pools. See below. |

`from` is the least level at which a rule acts, default 1. A level never
passes 255: what would take it further is lost.

**Fuel is a tag.** `spread` ignites what carries the tag its `needs`
names, and a material flagged `flammable` counts as carrying the tag
`flammable`. What makes a fire grow where it has fuel is a reaction
with that tag (below), and what consumes the fuel is the field's own
`damage`: an element destroyed by it becomes its `on_destroy` state and
leaves its rubble like any other (section 12.3), and the cause of that
is the field, for whoever is responsible for it.

**Where a field can go.** Fields cannot enter a cell whose block has
`coverage` of 0.95 or more. They pass between storeys only where there
is no floor or the floor is a `grate`: the floor in question is the
upper cell's. An edge of the grid and space that is not loaded let
nothing through. Fixtures and occupants stop nothing. The one
exception is `spread` with `needs`: it reaches a neighbour that it
could not otherwise when what stops it, the covering block or the floor
between, is itself of a material it needs. So a fire burns its way into
a wooden wall and through a wooden floor, and stops at stone. An edit
may put a field into any loaded cell.

**Storage and activity.** Storage is sparse per chunk and per field:
for each cell that holds a field, its level, whether it is **active**,
and who is responsible for it. Only active cells are stepped, and a
chunk with no field costs one pointer. A cell is active while it is
awake:

- A cell that gains a field is awake, if anything steps the field: it
  has a rule or takes part in a reaction.
- A cell of a field whose rules only move it (`flow`, `diffuse`), and
  which reacts with no material, goes to sleep at a step that moves
  nothing into or out of it and changes nothing in it. Water that has
  found its level lies still and costs nothing.
- A sleeping cell wakes when the level of its field changes in a cell
  beside, above or below it; when any field's level changes in its own
  cell; and when an element changes in its cell or one beside, above or
  below it.
- A cell of any other field is awake for as long as it lasts.

**Stepping.** One process of the world steps the active cells of one
clock and one step interval. It wakes at whole multiples of the
interval in the clock's local time, and the number of a step is that
local time in intervals, so a field in stopped time does not burn, one
in slowed time burns slowly, and nothing depends on when the process
began. Cells inside an area with a clock of its own are stepped by the
process of that clock.

Each step has three parts. First everything is read and nothing
changed: the reactions and then the rules of every active cell, in
order of grid, tract, chunk from the lowest, field and cell, say what
they would add and take, strike and dose. Then the changes of level
are summed for each field in each cell and written in order of cell
and field. Last the elements are struck, in the order the cells were
read, and the occupants told, in order of id. So the result does not
depend on the order cells are visited or were lit in, or on how the
simulation's `advance` is sliced. The one random choice, whether a
field spreads, is the positional hash (01 §4) of the realm's seed, the
domain `world.field.spread`, the cell, and the step number with the
field, the rule and the direction.

**Responsibility.** The instigator of the edit that first puts a field
in a cell is responsible for it there, and stays so while the cell
holds any. What a field brings into another cell by any rule or
reaction carries the instigator along, where that cell has none; of
several, the lowest id. Everything a field then does (`ElementDamaged`,
`ElementDestroyed`, `OccupantExposed`, `OccupantDosed`, `FieldAppeared`,
`FieldVanished`) has the cause kind `Field` and that instigator: what
burns down traces to whoever lit it.

**Catch-up.** A tract's fields are stored with its cells. When it is
read back, each field catches up on the local time that passed since it
was unloaded, by its definition's `catch_up`:

| `catch_up` | What happens |
|------------|--------------|
| `keep` | It is as it was. |
| `none` | It does not outlast unloading: it is gone. |
| `burn_out` | It is gone if as much as one of its steps has passed. What it would have burnt is not burnt. |
| `disperse` | It loses its `decay` for each step that passed, and half of what is left for every 16 steps. |
| `settle` | It is as it was, and every cell of it is awake, so that it goes on to find its level. A true closed-form settling is not built. |

Changes made by catching up have the coarse cause.

**The budget.** `field.max_active_cells` (default 200,000) bounds cost.
At a step that finds more active cells than that in the cosmos, the
weakest decay faster: with `n` cells too many, every active cell at or
below the level of the `n`th weakest loses 16 levels in that step on
top of its own rules. `FieldBudgetExceeded` is emitted at the first
step that finds it so, and again only after the count has been back
under.

**Reactions.** What fields do to each other and to what they touch is
written once, as **field reactions**, not separately in each field.

```json
[
  { "type": "field_reaction", "id": "quench",
    "when": ["water", "fire"],
    "uses": { "water": 1, "fire": 3 }, "gives": { "steam": 2 } },
  { "type": "field_reaction", "id": "burning",
    "when": ["fire", { "material_tag": "fuel" }],
    "gives": { "fire": 30 } },
  { "type": "field_reaction", "id": "current_through_water",
    "when": [ { "damage": "shock" }, "water" ],
    "spreads": { "share": 0.8, "reach": 6 } }
]
```

| Field of `field_reaction` | Meaning |
|---------------------------|---------|
| `when` | The two things that react: each a field id, `{ "material_tag": … }` or `{ "damage": … }`. Required. |
| `uses` | What one reaction takes of each field that takes part, by field. |
| `gives` | What one reaction adds to the cell, by field. |
| `max` | The most reactions in a cell in one step. Default: no limit. |
| `spreads` | For a reaction to damage: `share`, 0 to 1, and `reach`, in cells. |

| Form | Happens |
|------|---------|
| Two fields | Each step, in every cell where both are present, as many times as the levels there allow: the lesser of each field's level over what one reaction uses of it, and at most `max`. With no `uses`, once. It runs at the steps of the field named first where that one is awake, and at the other's where it is not. |
| A field and a material tag | Each step of the field, where it shares a cell with a floor, block or fixture of a material so tagged: once, or with `uses` as many times as the field's level allows. |
| A damage type and a field | When a packet of that type arrives in a cell holding the field. `spreads` passes a packet with `share` of the energy and of the intensity to every cell joined to that one through the field, by steps north, east, south, west, up and down through cells that hold it, at most `reach` steps away and 4096 cells: its elements are struck and its occupants told with `OccupantExposed`. The cell the packet arrived in gets nothing more. |

Ten of water and ninety of fire in a cell leave, after one step of the
water, no water, sixty of fire and twenty of steam. A reaction uses only
fields that take part in it; one to damage takes `spreads` and nothing
else. Whoever delivers a packet to a cell calls
`WorldEdit::conduct(loc, packet)` once for it: a release for each cell
its front reaches (section 13.2), and a body for the cell of the element
or the occupant it strikes, with the packet of the blow (section 15.2).
A reaction that also `leave`s an element in the cell (lava meeting water
gives steam and leaves basalt) comes with change of state (package W25).

Reactions are compiled into the field stepper and follow its rules:
buffered writes, and nothing used twice in one step. The `convert` rule
in a field's own definition is shorthand for a reaction between two
fields, with the field itself named first.

**Exposure.** Not all harm is a blow. The `expose` rule adds to a pool
of each occupant in the cell, in proportion to the field's level, by a
**route**:

```json
{ "rule": "expose", "pool": "toxin", "per_level": 0.02, "route": "breath" }
```

| Route | Reaches |
|-------|---------|
| `breath` | Those who breathe the cell's medium. Not those with air of their own: a sealed suit or volume (11 §10) while its air lasts. Not bodies that do not breathe. |
| `skin` | The share of the body not under a worn layer that guards against it |
| `any` | Everyone in the cell |

The world applies no route and no guard. At each step of the field it
emits `OccupantDosed` for each occupant in a cell of it, of whatever
kind and plane, with the pool as written, the amount
`level × per_level` and the route; an occupant that fills several cells
of the field is told once, for the strongest. The simulation decides
what reaches the pool: worn items may state `guards`, the share they
keep out by route (06 §9.1), and the hook `exposure_incoming` lets rules
change the amount. What the pool then does to its owner is content
(06 §4.2): a poison, a sickness, a slow burn. `OccupantExposed` from a
`damage` rule is reported the same way, once a step for the strongest
cell.

Harm that passes through walls is not a field. It is a signal that
exposes (15 §3.1).

**Sight and light.** `Cosmos::field_hides_sight(loc)` says whether a
field in a cell is at or above its `opaque_from`.
`Cosmos::field_lights(grid, box)` lists the light the fields in a box
give off as moving sources, one for each cell, for whoever asks for
light to hand in (15 §3.3): the world keeps no light for fields.

**Reading and writing.** `WorldEdit::add_field(loc, field, amount)`
adds to a level, or with a negative amount takes from it, and
`set_field(loc, field, level)` sets it; each returns the level the cell
had, and is refused for a cell outside its grid, not loaded or out of
reach, and for a field no definition has. `Cosmos::field` reads a level,
`fields_at` every field of a cell, `field_cell` the level with whether
it is active and who is responsible, and `fields_in_box` every field in
a box of cells, by cell and then by field. `Cosmos::field_drift` gives
the velocity of the medium at a cell where an area sets one; the
region's wind and current join it with the weather (09 §14.1), and a
vortex turns from east toward south about the middle of its area.

Anything more exotic subscribes an EOC to `FieldContact` or
`FieldAppeared`.

Still bodies of liquid (lakes, seas) are blocks flagged `liquid`, not fields.
When an edit opens space beside one, cells along the opening are converted to
the corresponding field so the liquid can move, within the same budget.
This is not built: it follows once fields are stable.

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
    Impact(PendingImpact),                  // a body that came down here unseen (section 15.4)
}
```

The cold simulation and EOCs use these to change places nobody is looking at:
board up a shop, leave a body in an alley, put a crater where the fight was.

As built, the map is `PendingEdit::Map(PendingMap { placement, mode })`:
a placement as 10 §4.1 gives it, seed included, so that every tract a
map covers takes its own window of the same map, and a `MapMode` of
`replace`, `only_empty` or `add` (10 §4.3). `Liquid`, `Devastation` and
`Eoc` join the list with the packages that give them meaning.

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
earlier loading of that tract left them. A grid remembers one number, one
more than the highest revision of any column that has left it, and a
column that joins starts there (section 6.2).

Still to come, with the packages that build them:

- All queries composite subgrids. Sight, rays and signals cross a
  subgrid's boundary without limit: a subgrid is part of the scene.
- They pass a portal only if it is `see_through`, only for a viewer within
  its `view_range`, and only `sight.portal_depth` portals deep (default 1).
  While a see-through portal's near side is loaded and something that could
  see through is within range, the portal holds a warm ticket on its far
  side (section 10).
- Fields that hide, such as smoke, do not yet block rays, lines of sight
  or fields of view. `Cosmos::field_hides_sight(loc)` says whether a
  cell's fields hide what is behind it (section 16), and whoever needs
  it asks cell by cell. Wiring it into the queries waits for a field of
  view that light does not share: light's cache is keyed on structure
  revisions alone (15 §3.3).

Illumination (`Cosmos::light`) is specified in 15 §3.3 and the medium of a
cell (`Cosmos::medium`) in 11 §4. Both are world queries with caches keyed
on chunk revisions. Light's cache is a table of the cosmos beside the
chunks, behind a lock, since reads take the cosmos by shared reference:
it keeps, for each chunk, what the chunk looks like to light, and works
a chunk's light out again only when that has changed within light's
reach. Loading or unloading a tract tells it to read that tract's cells
again whatever their revisions say.

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

**The queries as properties** (05 §5.3; decision D-465). Null where
the cell is not loaded or the thing does not exist. None is safe for
belief except those marked.

| Property | Gives |
|----------|-------|
| `loc.x`, `loc.y`, `loc.z`, `loc.grid` | `int`: the coordinates and the grid's id. Safe |
| `loc.realm`, `loc.tract` | `realm?`, `tract?`. Safe |
| `loc.site` | `site?`: the site its tract belongs to |
| `loc.areas` | `list<area>`: the areas that cover it, in id order |
| `loc.in_area(kind: id<area_kind>)`, `loc.in_area_tagged(tag: string)` | `bool`: whether an area of a kind, or one carrying a tag, covers it |
| `loc.floor`, `loc.block`, `loc.fixture` | `element?`: null where the cell has none |
| `loc.is_loaded` | `bool` |
| `loc.standable`, `loc.is_open`, `loc.under_open_sky` | `bool?` (sections 5.2, 7.1) |
| `loc.obstacle_height` | `real?`: metres |
| `loc.has_flag(flag: string)` | `bool?`: one of `walk_blocked`, `opaque`, `has_floor`, `solid_top`, `fragile` (section 6.2) |
| `loc.tags`, `loc.has_tag(tag: string)` | The tags of the definitions of its three elements |
| `loc.light` | `real?`: how bright it is now (15 §3.3), without moving sources |
| `loc.gravity` | `real?`: ambient gravity in m/s² |
| `loc.atmosphere` | `id<atmosphere>?`: null is vacuum |
| `loc.time_of_day` | `real?`: seconds since midnight by the sky of its stratum |
| `loc.occupants` | `list<entity>`: who has a cell of their footprint there |
| `loc.clock_rate`, `loc.local_time` | `real?`: the rate against universal time, and the local time in seconds, of the clock things there run on |
| `element.layer`, `element.loc` | `string` (`floor`, `block`, `fixture`), `loc`. Safe |
| `element.def`, `element.def_id` | Its definition: an id of `floor`, `block` or `fixture` by layer, typed `any`; and its qualified id as a `string?` |
| `element.material` | `id<material>?` |
| `element.state`, `element.state_name` | `int?`: the state's bits; `string?`: the name of a declared state |
| `element.damage` | `int?`: 0 to 255 |
| `element.height` | `real?`: metres |
| `element.has_flag(flag: string)` | `bool?`: one of the engine flags of section 5.4 |
| `element.tags`, `element.has_tag(tag: string)` | The tags of its definition |
| `area.def` | `id<area_kind>?` |
| `area.tags`, `area.has_tag(tag: string)` | Its tags |
| `area.site`, `area.loc`, `area.grid`, `area.priority` | `site?`; `loc?`, its middle; `int?`; `int?` |
| `area.contains(at: any)` | `bool?`: whether it covers a cell, or where something is |
| `area.occupants` | `list<entity>` |
| `area.breached` | `bool?` |
| `area.clock_rate`, `area.local_time` | `real?`: the rate of its own clock and that clock's local time; null if it has none. `clock_rate` can be set: 03 §13 |
| `site.name` | `string?`. Can be set |
| `site.tags`, `site.has_tag(tag: string)` | Its tags |
| `site.realm`, `site.loc`, `site.tracts` | `realm?`; `loc?`, its anchor; `list<tract>` |
| `tract.realm`, `tract.x`, `tract.y`, `tract.loc` | `realm`, `int`, `int`, `loc?`: its corner at storey 0. Safe |
| `tract.loaded`, `tract.surveyed` | `bool` |
| `tract.activity` | `string?`: `hot`, `warm`, or null |
| `tract.devastation` | `int?`: 0 to 255 |
| `tract.site` | `site?` |
| `realm.def`, `realm.grid` | `id<realm>?`, `int?` |
| `realm.clock_rate`, `realm.local_time` | `real?`. `clock_rate` can be set: 03 §13 |

Distance and line of sight are the functions `dist` and `los`, and
`light(at)` reads the light (05 §17). Variables at cell, area, site,
tract, realm and global scope are read and written as 05 §6 lays down.

Not bound yet, each for a reason: a field's value (`loc.field(kind)`),
because fields are being built and have no definition type an id could
name; ambient temperature, because the world has no function that gives
the temperature of a cell; drift, because it is a vector with a medium
and there is no value for one; the marks in a cell (`decals`), which
want a query; a region's kind, field values and variables, because the
kind `region` is not served; and the properties of a definition reached
through an id, such as a material's density (05 §17).

**Verb names.** The effects above are written in data as: `set_floor`,
`set_block`, `set_fixture`, `set_material`, `damage`, `release`, `tremor`,
`devastate`, `add_field`, `add_decal`, `remove_decal`, `set_var`,
`launch`, `impulse`, `arrest`, `detach`, `merge`, `apply_map`,
`open_portal`, `close_portal`, `create_realm`, `destroy_realm`,
`create_area`, `remove_area`, `set_clock_rate`, `enclose`, `hold_ticket`,
`release_ticket`, `add_pending_edit`, `transfer`, `set_plane`,
`set_liquid_level`.

**The verbs as bound** (package K8, decision D-620;
`clash_sim::world_verbs`). Each calls the one function of the world
named beside it, inside `Sim::world`, with the cause the rule runtime
hands it, so that every change carries that cause and the world's own
events follow. A refusal is the world's (`world.not_loaded`,
`world.outside_realm`, `world.unknown_area`, `world.refused` with the
world's message) or one of `world.bad_argument` (`verb`, `arg`),
`world.unknown_state` (`state`), `world.unknown_def` (`def`),
`world.no_element` (`layer`) and `world.not_in_world` (`entity`). An id
argument also takes the qualified id as a text.

| Verb | Arguments | Returns | Calls | Raises |
|------|-----------|---------|-------|--------|
| `set_floor`, `set_block`, `set_fixture` | `at: loc`; `to: string?`, a state as data writes it (`def`, `def@material`, `def#state`), null to clear | null | `WorldEdit::set_element` | `element_changed`; `occupant_displaced` or `occupant_entombed` |
| `set_material` | `of: element`, `material: id<material>` | null | `set_material` | `element_changed` |
| `set_state` | `of: element`, `state: string` (a declared state's name, or the bits written as a number) | null | `set_state_bits` | `element_changed` |
| `damage` | `at: any` (a cell, or an element, which names its layer), `layer: string?` (default `block`), `kind: id<damage_type>`, `energy: real` (J), `intensity: real` (Pa), `dir: any?` (three numbers, or something to strike toward) | `string`: `unaffected`, `damaged`, `destroyed` | `damage_along` | `element_damaged` or `element_destroyed` |
| `destroy` | `of: element`, `kind: id<damage_type>` | `bool`: there was one | `destroy` | `element_destroyed` |
| `check_stability` | `at: loc` | null | `WorldCtx::check_stability` | `collapse_started` if something gives way |
| `release` | `at: any` (a cell, an element's cell, an entity where it stands), `kind: id<damage_type>`, `energy: real`, `shape: string?` (`sphere`, `cone`, `beam`), `dir: any?`, `half_angle: real?`, `radius: real?`, `range: real?`, `burst: bool?`, `speed: real?`, `height: real?` (m above the cell's base) | null | `WorldCtx::release` | `release_started`, later `energy_released` |
| `add_field` | `at: loc`, `field: id<field>`, `amount: int` | `int`: the level before | `add_field` | `field_appeared`, `field_vanished` |
| `set_field` | `at: loc`, `field: id<field>`, `level: int` | `int`: the level before | `set_field` | the same |
| `launch` | `what: entity` standing in the world, `vel: any` (three numbers, m/s) | null | `WorldCtx::launch`, as `Body::of_mass` with its `Physical` | `body_launched` |
| `impulse` | `what: entity` in flight, `j: any` (three numbers, kg·m/s) | null | `WorldCtx::impulse` | — |
| `arrest` | `what: entity` in flight | null | `WorldCtx::arrest` | — |
| `create_area` | `at: loc`, `to: loc?` (a box's other corner), `radius: real?`, `kind: id<area_kind>?`, `tags: list<string>?`, `clock_rate: real?`, `priority: int?` | `area` | `WorldCtx::create_area` | `area_entered` |
| `remove_area` | `area: area` | null | `WorldCtx::remove_area` | `area_exited` |
| `enclose` | `from: loc`, `kind: id<area_kind>?`, `limit: int?` (default 65,536) | `area` | `WorldCtx::enclose` | `area_entered` |

A release's shape is three flat arguments rather than the nested object
of the sketch above, so that each is checked against its type at load.
Cell variables are the kernel's `set` and `unset` (05 §10.3), which
write through `WorldEdit::set_var`.

**Not bound yet, each for a reason** (D-620): `tremor` and `devastate`,
whose world functions are not built (W24, W17); `detach` and `merge`
(subgrids, section 18); `open_portal` and `close_portal` (section 19);
`create_realm`, `destroy_realm` and `transfer` (section 21); `apply_map`
at a location, which wants a map id the simulation's registry does not
yet name and a placement of the generator's; `hold_ticket`,
`release_ticket` and `add_pending_edit`, which want an anchor and a
pending edit as values rules can write; `set_plane`, which is the
kernel's presence and waits for the planes kind; `set_liquid_level`
(11 §4.4); `add_decal` and `remove_decal`, because no definition type
names a kind of mark; `steer`, because a body is not a subject kind;
and `set_clock_rate` of an area, which the kernel's clock verbs (T1)
are to take.

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

As registered so far (decision D-466), the shape is not marked safe
either: nothing can honour `hidden` until perception is built, and a
property that ignored it would tell a mind what it has not seen. Only
what follows from a reference alone is safe: a cell's coordinates, grid,
tract and realm, a tract's coordinates and realm, an element's layer and
cell.

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
thrown. Sliding friction is the `friction` of the material a body lies
on, a half (`BODY_DEFAULT_FRICTION`) where the material states none.
`REST_SPEED` is a named constant, 0.5 m/s.

**Releases.** `release` takes the fields of `EnergyRelease` (section
13), with the shape's own as arguments beside it: see the table of
bound verbs above. Reachable only from Rust so far: the
releases still spreading (`releases`), `intensity_at` and `reach` of a
release, and the settings, which are constants until settings are
bound: `world.min_intensity` (`WORLD_MIN_INTENSITY`, 5 kPa),
`release.max_cells` (2,000,000; `Cosmos::set_release_max_cells`),
`release.vacuum_speed` (1000 m/s), `release.push_depth` (1 cm) and
`release.throw_speed` (3 m/s). The least distance (0.5 m), the height a
thrown occupant starts at (1 m), the bins of the map (16 to 48 a side)
and the cells a confining space may hold (4000) are named constants.

**Bodies.** As bound, `launch`, `impulse` and `arrest` take the entity
that is or becomes the body (the table above): a body is not yet a kind
of subject. A projectile, a fragment and a body with a drive are
launched from Rust with the fields of `Body` (section 15), and `steer
{ body, toward }` waits for the kind `body`, a `BodyId`. Reachable
only from Rust so far: the bodies in flight and the body an entity is
(`bodies`, `body`, `body_flight`, `body_of`), the number of fragments
in flight (`debris_in_flight`), and the constants of flight and debris:
`BODY_MAX_STEP` (20 ms), `BODY_MIN_STEP` (10 µs), `BODY_DEFAULT_DEPTH_M`,
`BODY_DEFAULT_DRAG`, `BODY_ASSUMED_DENSITY`, and for fragments
`DEBRIS_PER_ELEMENT`, `DEBRIS_MASS_SHARE`, `DEBRIS_ENERGY_SHARE`,
`DEBRIS_SCATTER`, `DEBRIS_DRAG` and `DEBRIS_RESTITUTION`. The settings
`body.max_ticket_tracts` and `debris.max_bodies` are the constants
`BODY_MAX_TICKET_TRACTS` (3) and `DEBRIS_MAX_BODIES` (256) until settings
are bound.

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
`burn_out`, `disperse`, `settle`, `keep`, `none`. For fields (section
16): the levels the weakest cells lose over budget
(`FIELD_OVER_BUDGET_DECAY`, 16), the steps in which a dispersing field
halves (`DISPERSE_HALF_STEPS`, 16), the most cells one field passes a
packet on to (`FIELD_CONDUCT_MAX_CELLS`, 4096), the shortest step
(`FIELD_MIN_STEP`, 1 ms) and the largest `rate` (`FIELD_MAX_RATE`,
0.25). `field.max_active_cells` is `Cosmos::set_field_max_active_cells`
until settings are bound.

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

**Generator parameters.** A realm definition's `generator_params` is a
map of values its generator reads (section 9.3). A parameter the generator
does not know is an error, not ignored.

**Tickets and pending edits from Rust.** `hold_ticket`, `release_ticket`
and `add_pending_edit` are functions of `Cosmos` with the arguments of
sections 10.1 and 9.4. As verbs they are bound with the rule runtime. A
tract's activity and its record's variables are read as `tract.activity`
and `tract.var`.

**Variables of records.** `Cosmos::set_global_var(key, value)`,
`set_realm_var(realm, key, value)`, `set_tract_var(realm, tract, key,
value)` and `set_region_var(realm, region, key, value)` set a variable,
or clear it with no value, and return what it held; `tract_var(realm,
tract, key)` reads one. A tract or a region that has not been surveyed
has no record and is refused. They emit nothing: the simulation raises
`var_changed` for watched variables (05 §6).

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
damage and collapse (asking for a stability check is the verb
`check_stability`): what a check would find, with carriers and loads (`assess_stability`);
where something dropped comes to rest (`resting_cell`); the amount a
pile of rubble holds (`rubble_units`); the direction of a hit
(`damage_along`); and the collapses in progress and masses in flight
(`collapses`, `falling_masses`). An area's gravity is changed with a
check through `WorldCtx::set_area_ambient`; the `Cosmos` function of
that name asks for none. Of the light at a cell, `fill_light` and
`add_moving_light` are reachable only from Rust; the light itself is the
property `loc.light` and the function `light(at)`. From fields (section 16): setting a level
outright (`set_field`); telling the fields of a cell that a packet
arrived (`conduct`), which whatever delivers damage to a cell must do
for a reaction to damage to happen; every field of a cell or of a box
(`fields_at`, `fields_in_box`); whether a cell's fields hide sight
(`field_hides_sight`); the light fields give (`field_lights`); the
drift at a cell (`field_drift`); who is responsible for a field in a
cell and whether it is active (`field_cell`); and the count of active
cells (`active_field_cells`). Items are not fuel to the world, which
does not know them: a rule that wants a burning item to feed a fire adds
to the field itself.

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
| W9 | No definition id appears in engine code. Fills and rubble are named by data, and a placeholder by the id the save wrote (section 11.2). |
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
