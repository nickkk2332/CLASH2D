# 10 — World Generation and the Map Format

Status: design complete, ready for implementation.
Implementation plan: [plans/worldgen.md](plans/worldgen.md).
Serves scenes S2, S7, S8, S9, S12, S19 and S21 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how worlds come to exist: the map format in which
places are authored, how maps are placed and combined, how ground is
filled, how regions and settlements are laid out, how a whole realm is
planned, how places are ruined or transformed, and what generation hands to
the simulation so that the world has people, groups and routines in it from
the first moment.

Crate `clash-gen`, behind the interfaces of 02 §9.3.

Fixed from earlier documents:

- Generation is pure and per tract. A tract can be realised in any order
  and always comes out the same (02 §9.3).
- A realiser sees its tract's record and its neighbours' records, never
  their cells. What spans tracts is agreed in records first.
- World size and shape belong to each realm definition. There is no default
  planet (D-044).
- The engine has no setting (01 R10). Everything here must serve a modern
  city, a fantasy kingdom, a dungeon, an ocean floor, a space station and a
  pocket dimension equally.

Names of maps, sites and settlements in examples are illustrative content.

## 2. The picture

A place is authored as a **map**: rows of characters, one layer per storey,
with a **palette** saying what each character stands for. A house, a
dungeon room, a submarine's decks and a street corner are maps. Maps take
**parameters**, contain other maps, and may be turned and mirrored.

Maps are put into the world by **placements**: this map, here, turned this
way, with these parameters and this seed. Underneath them is **ground**,
filled by rule: strata of soil and rock, a surface, water up to sea level,
caves, trees.

Where things go is decided top down, and each level only writes records:

1. A **realm plan**, made once when the realm is created, holds what must
   be agreed everywhere: where land and sea are, where the settlements and
   the great sites are, what joins them, who holds what.
2. A **region survey**, made the first time anything needs a region, lays
   out its 4096 tracts: what kind of ground each is, and every placement in
   it. A town's streets and lots are laid out here.
3. **Realising a tract** turns its record and placements into cells.

Places larger than one map are built from parts. A **stack** piles storey
maps into a tower. An **assembly** joins rooms at their doorways into a
dungeon, a station, a sewer.

Generation also seeds the simulation. A site's map **marks** where its
counter, its beds and its guard posts are. Its definition says who works
there and who owns it. From those come the agents, populations, groups,
stocks and routines the layered simulation (09) runs on.

A world may be entirely authored, entirely generated, or anything between.
A scenario arena is one map on bare ground. A planet is a plan and a set of
rules.

## 3. Maps

### 3.1 Definition

```json
{ "type": "map", "id": "townhouse_a", "tags": ["house", "small"],
  "size": [8, 9], "front": "south",
  "params": { "wall": { "pick": ["brick", "timber"] } },
  "palette": ["house_common",
    { "#": "wall@$wall", "+": "door", "w": "window",
      "b": { "fixture": "bed", "mark": "bed" },
      "k": { "fixture": "stove", "items": { "table": "kitchen_goods", "chance": 0.7 } },
      "<": { "fixture": "stairs_up" }, ">": { "fixture": "stairs_down" } } ],
  "default": { "floor": "boards" },
  "levels": {
    "0": [ "###w####",
           "#..k...#",
           "#......w",
           "#......#",
           "#####+##",
           "#......#",
           "w.....<#",
           "#......#",
           "###+####" ],
    "1": [ "###w####",
           "#b.....#",
           "#......w",
           "#b.....#",
           "#####+##",
           "#......#",
           "w.....>#",
           "#......#",
           "########" ],
    "2": "roof" },
  "areas": [ { "kind": "home", "level": [0, 1], "box": [[0, 0], [7, 8]] } ],
  "marks": { "entrance": [3, 8, 0] },
  "spawns": [ { "population": "householders", "count": [1, 4], "in": "home" } ] }
```

| Field | Meaning |
|-------|---------|
| `size` | Width and depth in cells, each from 1 to 512. Every row has `size[0]` characters; every level has `size[1]` rows. Rows run north to south. |
| `levels` | Storeys by number, relative to the map's own ground level; at most 256 from the lowest to the highest, and at most 1,048,576 cells over them all. A level is a list of rows, or the name of a preset. A storey between two that are written and not written itself is left as it is. |
| `palette` | What each character means (section 3.2). Named palettes and inline entries, later overriding earlier. A key is one character, never a space. |
| `default` | The cell spec laid under every character that is not a space |
| `roof` | The cell spec the `roof` preset lays; the `default` if left out |
| `params` | Values chosen once per placement (section 3.4) |
| `front` | Which side faces a street, for placement: `north`, `east`, `south` or `west` |
| `place` | Other maps placed inside this one (section 3.5) |
| `set` | Cell specs applied at listed coordinates or boxes, for things awkward to draw |
| `areas`, `marks` | Areas (02 §8.3) and named points or cells |
| `spawns` | Creatures and populations (06 §12) |
| `wiring` | Conduit layers and device links (section 3.6) |
| `joints` | Where it can connect to other maps (section 7.2) |
| `anchor` | The cell a placement names when it says where the map goes: a cell or a mark. Left out, a placement says where the north-west corner at level 0 goes (section 3.5) |
| `base` | What level 0 sits on: `ground`, `sea_level`, `sea_floor`, or an absolute storey |
| `sit` | How it meets uneven ground: `level`, `foundation`, `follow` or `none` (section 4.2). With `foundation`, the field `foundation` names the block state that fills the gaps. |
| `turn`, `over` | Whether a placement may turn it (section 3.3), and whether it may overlap another (section 4.1) |
| `portals` | Portals (02 §19) between marks of this map, or from a mark to a named entry elsewhere: `{ "from": "mark:a", "to": "mark:b" }` |
| `vehicles` | Vehicles (11 §12.1) standing at marks; a vessel is placed as a subgrid: `{ "vehicle": id, "at": point, "facing": direction }` |
| `if` | A condition on the placement's parameters; if false the map is not placed |

A space character means "leave what is there". Everything else replaces:
a character writes all three elements of its cell. Each is what its cell
spec states, or else what the `default` states, or else nothing. So a
house drawn over a meadow has no grass under its boards and no boulder in
its parlour. A cell spec that wants a layer left as the world has it says
so with `keep`.

**Presets.** `roof` lays the map's `roof` cell spec, or its `default` if
it has none, over every cell that the storey below draws, a roof over a
roof included. The storey below must be written. A roof that is not the
shape of the storey under it is drawn as rows like any other level.

**Points, boxes and levels.** A point is a cell, `[x, y]` or
`[x, y, level]` with `x` east and `y` south from the north-west corner,
or a mark, `"mark:name"`, which stands for the mark's first cell. A box
is `[[x0, y0], [x1, y1]]`, both corners included, on the storeys given
beside it as `"level": n` or `"level": [lowest, highest]`; level 0 if
left out. All of them must lie inside the map.

| Entry of | Written |
|----------|---------|
| `set` | `{ "at": point, "spec": cell spec }` or `{ "box": box, "level": levels, "spec": cell spec }`. Entries are applied in order after the rows, each over what is there: a layer the spec states is replaced, and a cell no character drew is drawn. |
| `areas` | `{ "kind": area kind, "name": text, "level": levels, "box": box, "tags": [...] }`. The kind is an `area_kind` definition. `name` is what `place` and `spawns` call the area; its kind as written if left out. |
| `marks` | `"name": [x, y, level]`, or a list of such cells |
| `spawns` | `{ "creature": id }` or `{ "population": id }`, with `count` (a number, `[low, high]` or an expression; one if left out), `chance`, and either `"in": "area name"` or `"at": point` |
| `joints` | `{ "id": label, "at": cell, "dir": direction, "kind": text }`. `dir` is `north`, `east`, `south`, `west`, `up` or `down`, and the cell must be on that edge of the map: its north row for `north`, its highest storey for `up`. |

### 3.2 Palettes and cell specs

A palette maps a character to a **cell spec**.

| Spec field | Sets |
|------------|------|
| `floor`, `block`, `fixture` | Element states (02 §5.1) |
| `clear` | Empties listed layers: `floor`, `block`, `fixture`, and of what else a cell holds `conduits`, `fields`, `items`, `decals`, `vars` |
| `keep` | Leaves listed element layers (`floor`, `block`, `fixture`) as the world has them |
| `conduits` | Conduits laid in the cell (16 §6) |
| `field` | Field amounts (02 §16), by field id |
| `items` | Items, or a content table: an item id, or `{ "item": id }` or `{ "table": id }` with `count` and `chance`. In a container fixture they are latent contents (06 §9.3). |
| `spawn` | A creature or population member in this cell: a creature id, or `{ "creature": id }` or `{ "population": id }` with `count` and `chance` |
| `device` | Configuration for the element's device (16 §4.2), and a name for linking: a name, or `{ "name": text, "config": { … } }` |
| `mark` | Adds the cell to a named mark, or to each of a list of them |
| `berth` | Adds the cell to a named berth of a kind: a parking bay, a mooring, a landing pad. Cells given `approach` with the same name are the line an AI driver follows into it (09 §5.6). |
| `watched` | Marks what is placed in the cell as something the site's people keep an eye on (15 §6) |
| `area` | Adds the cell to an area of a kind, an `area_kind` definition; touching cells with the same entry are one area. Cells touch across a side, or one above the other. |
| `var`, `decal` | Cell variables and marks (02 §5.1) |

A bare string is short for the layer its definition belongs to:
`"wall@brick"` is `{ "block": "wall@brick" }`. If an id exists in more than
one of floor, block and fixture, the short form is a load error asking for
the long one. `"none"` is such an id. In the short form the definition is
written out; its material and state may come from parameters.

In a map used as a pattern (16 §4.5) an entry may instead ask for any
element carrying a tag (`block_tag`, `floor_tag`, `fixture_tag`) or for
anything at all (`"any": true`).

A palette value may be a list of weighted alternatives, chosen per cell,
or once for the whole placement with `"once": true`. A list cannot carry
a key, so the second is written as an object around the list.

```json
"t": [ { "fixture": "table", "weight": 3 }, { "fixture": "desk" }, {} ],
"u": { "one_of": [ "wall@brick", "wall@timber" ], "once": true }
```

A `weight` is a number or an expression over the map's parameters, worked
out once for the placement; one if left out. An alternative whose weight
comes to nothing or less is never chosen, and if every weight does the
first alternative is taken. Alternatives do not nest.

The `default`, the `roof` and the `spec` of a `set` entry are palette
values too.

Shared palettes are definitions:

```json
{ "type": "palette", "id": "house_common",
  "entries": { ".": {}, "c": { "fixture": "chair" }, "t": { "fixture": "table" } } }
```

### 3.3 Turning

A placement may turn a map by quarter-turns and mirror it. Element states
with a facing are turned with it. `"turn": false` on a map forbids it.

A turn mirrors first, swapping east and west, and then rotates clockwise.
The turned map is measured from its own north-west corner again: a map 7
wide and 5 deep turned a quarter is 5 wide and 7 deep, and every cell,
mark, area, joint and direction it carries has turned with it. Every
choice a map makes is made as the map is drawn, before it is turned, so
the same seed gives the same building whichever way it faces.

**Facing.** A state **faces** a direction when its name is a compass
direction (`north`, `south_west`), alone or after an underscore
(`open_east`), and its definition has the states for the other three
quarter-turns as well. Turning the map replaces such a state by the state
of the same definition that faces the turned direction: `stairs#north`
turned a quarter clockwise is `stairs#east`, and `gate#open_east` mirrored
is `gate#open_west`. A state that names no direction, or whose definition
lacks one of the four, is carried over as it is. The engine gives the
names no other meaning.

### 3.4 Parameters and chance

Parameters are chosen once for each placement: given by whoever placed the
map, or picked by the map's own rule. They may be used anywhere in the
map: `"wall@$wall"`, `"count": "$beds"`, `"if": "$wealth > 2"`.

| A parameter written as | Gets |
|------------------------|------|
| `{ "pick": [a, b, …] }` | One of the values, each equally likely |
| `{ "range": [low, high] }` | A whole number from `low` to `high`; or, if either has a fraction, a number from `low` up to `high` |
| `2`, `true`, `{ "default": "brick" }` | That value |
| `"$wealth > 2"`, `{ "expr": "…" }` | An expression over the parameters declared before it |
| `{ "type": "int" }` | Nothing: the placement must give it |

A parameter is a `bool`, an `int`, a `real` or a `string`; an id is given
as a `string`. The type follows from the rule, or is stated with `type`
beside it; a rule that gives another type than the one stated is a load
error. A parameter's text is always an expression, so a text value is
written `{ "default": "brick" }` or `"'brick'"`. A value the placement gives replaces the rule's, and must be of
the parameter's type; a whole number is accepted for a `real`. A
placement that gives a parameter the map does not declare, leaves out one
it must give, or gives one of the wrong type is refused with the reason.

In a state, `$name` is replaced by the text of a `string` parameter:
`"wall@$wall"`, `"door#$state"`. The state is checked when mods load with
every value the map's own rule can give the parameter, and when the map
is resolved for a value only a placement gives.

Expressions in maps are evaluated in a **generation context**. They are
pure. `rand`, `chance` and `pick` there draw from a generator seeded by the
placement's seed and the cell, never from the simulation's stream, so the
result depends on where and what, not on when.

The generation context binds the map's parameters as `$name` and nothing
else: no subject, no event. An expression there may use the built-in
functions that are worked out from their arguments, the functions mods
define, and the random functions. Reading a property or a variable, or
calling a function that only a running simulation can answer (`now`,
`dist`, `setting`), is a load error at the name. A shared palette is used
by maps with different parameters, so the expressions written in one see
none; a `$name` in one of its states is filled in by each map that lists
it, and must be a parameter of every one of them.

Every random choice is drawn from a generator seeded by a positional hash
(01 §4) of the placement's seed, a domain named for the kind of choice,
the cell the choice is for and a number that tells apart the choices made
at one cell. Choices made once for the placement (parameters, `if`,
weights, `once`, whether and how many nested maps) use the cell
`[0, 0, 0]`. No choice draws from a generator another choice has drawn
from, so the order in which cells are worked out decides nothing
(invariant X6).

| Domain | Seeds |
|--------|-------|
| `gen.map.param` | A parameter's `pick` and `range` |
| `gen.map.expr` | The random functions of an expression |
| `gen.map.alternative` | Which weighted alternative a cell gets |
| `gen.map.roll` | A `chance`, and a count written as a range |
| `gen.map.pick` | Which nested map of those that fit, and turned which way |
| `gen.map.scatter` | The order the cells of an area are tried in |
| `gen.map.nested` | The seed a nested map is given |

### 3.5 Maps within maps

```json
"place": [
  { "map": { "tag": "bedroom_set" }, "at": "mark:bedroom", "turn": "any", "chance": 0.9 },
  { "map": "rug", "in": "area:parlour", "count": [0, 2], "apart": 2 } ]
```

A map may place others: at coordinates, at marks, or scattered through an
area with spacing. The map placed may be named or chosen by tag. Nested
maps receive parameters and place maps of their own.

A building is usually a shell with rooms placed in it, and rooms with
furnishings placed in them.

| Field of a `place` entry | Meaning |
|--------------------------|---------|
| `map` | A map id, or `{ "tag": … }` for any map carrying the tag |
| `at` | A point (section 3.1): where the nested map goes |
| `in` | In place of `at`: the name of an area, `"area:parlour"` or `"parlour"`, to scatter copies through |
| `turn` | Degrees clockwise (`90`), `"mirror"`, `{ "rot": 90, "mirror": true }`, or `"any"`. Not turned if left out. |
| `chance` | The chance that the entry places anything |
| `count`, `apart` | With `in` only: how many copies, one if left out, and how many cells must lie between two of them |
| `params` | Parameters for the nested map, each an expression over this map's: `{ "wall": "$wall" }`. A text is written `"'brick'"`. |

**Where it goes.** A nested map that names no `anchor` is put with the
north-west corner of its turned footprint, at its level 0, on the point.
One that names an anchor is put with that cell on the point, however it
is turned: a door that must meet a path is the anchor of its map.

**Fitting.** A nested map must lie wholly inside the map that places it,
storeys included. A map named by id that cannot fit where it is put,
under the turn asked for, is a load error. Of the maps carrying a tag,
and of the turns `"any"` allows, one that fits is chosen by the
placement; if none can, that is a load error. A map that says
`"turn": false` is never turned.

**Scattering.** The cells of the area are tried in an order the seed
decides. A cell is kept if a map fits with its point there and no cell
kept before is within `apart` cells of it, until `count` are kept. The
area is every `areas` entry of that name together with the cells that
cell specs put in an area of that name.

**Laying in.** A nested map is laid over its parent after the parent's
rows and `set`, in the order of `place`. Where it draws a cell, the
layers its cell specs state replace the parent's, and a layer they do not
state keeps what the parent has there: a rug laid in a parlour keeps the
parlour's floor. Where the parent drew nothing, the nested cell replaces
like any other. Its marks join the parent's marks of the same name, and
its areas, spawns and everything else it carries become the parent's. Its
joints, its anchor and its front do not: they are for placing it by
itself. Each copy is given a seed of its own, from the parent's seed and
the cell it is put at.

### 3.6 Wiring

```json
"wiring": {
  "palette": { "=": ["copper_cable"], "~": ["water_pipe"] },
  "levels": { "0": [ "        ", " ====== ", "      = ", "..." ] },
  "links": [ { "from": "lever_a", "to": "vault_door", "resource": "control" } ] }
```

Conduits are drawn on their own layers over the same grid. Devices named
in cell specs are linked by name. A map can ship a working machine room
(scene S18).

The rows of a wiring level are the size of the map, a space lays nothing,
and every other character is an entry of the wiring's own palette. A link
names two devices that cell specs of the same map name.

### 3.7 Validation

At load: row lengths, unknown characters, unresolved parameters, nested
maps that cannot fit, maps that nest inside themselves or deeper than
`gen.max_nesting` (default 8), links to unnamed devices, joints off the
boundary.

Also at load: a state that names nothing, with every value the map can
give its parameters; a short form that could be two layers; a level that
is not a storey number or names no preset; a point, box, mark or anchor
outside the map; a mark, area or area kind that does not exist; a
parameter handed to a nested map that has none of that name; a turn
asked of a map that forbids it; and an expression that reads anything but
parameters. Each is reported at the value that is wrong, in the map that
is wrong. A map nests inside itself if any map it could place, by id or
by tag, could place it.

These checks are a build stage (04 §8.1): maps name states, and states
are known once the world's tables are built from the loaded definitions.
The linter runs them after a load without errors.

A mark can be named by a point, an anchor or a portal only if it is the
same in every placement: one from `marks`, or from a character with a
single cell spec. A mark that only some alternative gives is still
carried when the map is resolved.

### 3.8 A resolved map

Resolving a map for a placement (its parameters, its seed, its turn)
gives, for each storey, a grid of cells, each saying for floor, block and
fixture either "leave what is there" or the state to put there; and
beside the grids the areas, the marks and everything else the map asks
for, all in the coordinates of the turned map. It is a pure function of
the map, the parameters given, the seed and the turn. A map whose `if` is
false resolves to nothing.

What the world cannot yet take is carried in the resolved map as data,
with ids as written, for the package that builds the thing it belongs to:
spawns and items; conduits, fields, devices and links; berths and watched
cells; cell variables and decals; pattern cells; joints, portals and
vehicles.
With `clash-tools map check`: rooms unreachable by walking; marks a site
definition requires but its map does not supply; and whether the map
stands. The map is realised on flat ground and the support and load passes
(02 §14) are run. Anything that would fall or be crushed as drawn is
reported with its cell.

## 4. Placing maps

### 4.1 Placements

```rust
pub struct Placement {
    pub map: MapId,
    pub origin: CellPos,          // where the map's anchor goes: its north-west corner at level 0 if it names none
    pub turn: Turn,               // quarter-turns and mirror
    pub params: VarMap,
    pub seed: u64,
    pub order: u16,               // later placements overwrite earlier where they overlap
    pub site: Option<SiteId>,
}
```

Placements are records. A region survey writes them; realising a tract
reads those that overlap it. A map may be far larger than a tract: each
tract it covers realises its own window of it.

**Where a map goes.** `origin` is where the map's anchor goes (section
3.5): the cell its `anchor` names, or the north-west corner at level 0 of
its turned footprint. The world position of a map cell is the origin, less
the anchor, plus the cell.

**A window for each tract.** Resolving a map is a pure function of the
placement (section 3.8), so each tract a map covers resolves it again and
writes the cells that fall inside itself. No tract reads another's cells,
and the tracts of a large map come out the same in every order and on
every thread count (invariant X1).

**Order.** Within a tract, placements are written in `order`. Two of the
same order are written by position, then map, then seed, so the result
does not depend on the order they were gathered in.

**What a map hands on.** Besides cells:

| Asked for by the map | Becomes |
|----------------------|---------|
| `spawns`, and `spawn` in a cell spec | One spawn request (02 §9.3) for each creature, at a cell: handed over by the tract the cell is in. `what` is the creature or population id as written; its parameters are `kind` (`creature` or `population`) and `map`, the qualified id of the map that asked. A spawn that names an area is given cells of open floor in it, chosen by the placement's seed. |
| `areas`, and `area` in a cell spec | An area (02 §8.3), made once: by the tract that holds the lowest corner of its box, or its first cell |
| `marks` | Named cells in world coordinates, known from the map and the placement alone, without realising anything |
| Everything else (section 3.8) | Carried in the placed map as the map format left it, with the function that turns a map cell into a world cell |

**Where a realiser finds placements.** A realiser is shown tract records
and no region record (02 §9.3), so every placement that overlaps a tract
must follow from what it is shown. Two sources do: the placements a realm
definition makes by hand (section 6.2), which the generator holds and
which may cover any number of tracts; and those of tract definitions
(section 6), which are a pure function of the seed, the tract's position
and its definition, and lie wholly inside their tract. A survey writes
both into the region's record (02 §9.1) for everything else that asks
what stands where.

**Overlap.** Every placement has a box: its footprint and its storeys,
after turning. When a survey writes a placement it tests the box against
those already written. Two placements may overlap only if the later one's
map says `"over": true`, as a market stall on a square does. Otherwise the
later is moved as its rule allows, or dropped and noted in the survey's
report. The test is in three dimensions, so a sewer may run under a street
and may not run through a cellar.

A tract definition's placements are tested against those written before
them for the same tract, the realm's own among them. One that meets
another is dropped; no rule moves one yet.

### 4.2 Sitting on the ground

Ground is rarely flat and maps usually are.

| Map setting | Where the map's ground level meets the terrain |
|-------------|-------------------------------------------------|
| `level` (default) | The terrain under the footprint is flattened to the height at the origin, with ramps at the edges |
| `foundation` | Terrain is left. Gaps under the map are filled with a named state; terrain above its floor is cut away. |
| `follow` | Each column of the map is shifted to its own ground height. For fences, roads, low walls. |
| `none` | Nothing is done |

**The storey of level 0** is the placement's `origin`, which a survey
sets by the map's `base`: `ground` and `sea_floor` are the surface under
the anchor, as the height field gives it; `sea_level` is the realm's
(section 5.1), or the surface where the realm has no sea; a number is
that storey.

**When a tract is realised:**

- `level`: the surface under the whole footprint is the map's level 0
  before the ground is filled. The ground's own steps then lay strata
  and a surface under the map and put ramps where the surface now steps
  one storey, inside or outside the footprint. A step of more than one
  storey is left as a cliff.
- `foundation`: after the ground is filled, under every column the map
  draws, cells from the surface up to the map's level 0 are filled with
  the `foundation` state, and where the surface is higher, the cells
  from level 0 up to and including the surface cell are emptied.
- `follow`: each column is written with its level 0 at that column's
  surface.
- Nothing is scattered by the ground (section 5.1) under a footprint
  that levels or founds.

**When a map is applied to the world as it is** (section 4.3) there is
no height field, and the ground of a column is read from the cells: the
storey nearest the one the placement names, within 8 storeys, whose
cell stands on a block with a solid top. `level` cuts the ground above
level 0 within the footprint and fills gaps with the block under the
origin, without ramps; `foundation` and `follow` do as above. Only
`replace` cuts.

### 4.3 Applying a map to the world as it is

The same maps are used after generation.

| Verb | Does |
|------|------|
| `apply_map { map, at, turn, params, seed, mode }` | Writes a map into loaded cells through `WorldEdit` |
| A pending edit naming a map (02 §17.3) | The same, applied when the tract next loads |
| `plan_from_map` (16 §10.3) | Creates construction jobs for every cell where the map differs from the world |

| `mode` | Writes |
|--------|--------|
| `replace` (default) | Every layer the map states, emptying a layer where the map states nothing for it |
| `only_empty` | A cell of the map only where the world's cell holds nothing at all: no floor, no block and no fixture. Such a cell is written whole. |
| `add` | Each element the map states, only where that layer of the world's cell is empty. Nothing is ever cleared. |

`map`, `at`, `turn`, `params` and `seed` are the fields of a placement
(section 4.1): `at` is where the map's anchor goes, and a `seed` left
out is drawn from the cell.

An incident's aftermath, a conjured fortress, a scenario's set dressing and
a building design are all maps applied this way. `at` may be a mark of a
site: `site.mark('counter')`.

Every change is an ordinary edit with the cause given: it emits
`ElementChanged`, asks for stability checks and moves occupants like any
other (invariant X8). Putting an element in place asks for no stability
check (02 §7.2): whoever applies a building asks for one.

**As built**, in `clash_gen::place`:

- `apply_map(ctx, maps, grid, placement, mode, cause)` writes the cells
  of the map that lie in loaded tracts and returns a `MapApplied`: how
  many elements changed, the spawn requests and the areas it made, the
  marks in world coordinates.
- `place_map(ctx, maps, realm, placement, mode, cause)` does the same
  and, for each tract the map covers that is not loaded, leaves a
  pending edit (`PendingEdit::Map`). A tract that is neither loaded nor
  surveyed cannot hold one and is reported.
- The world cannot resolve a map, so a pending map stays in the tract's
  record when the tract loads. `apply_pending_maps(ctx, maps, realm,
  tract)` takes the maps waiting for a tract that has loaded and writes
  each, clipped to that tract, with the coarse cause. Whoever loads
  tracts calls it for each tract loaded. Pending maps are therefore
  written after the pending cell changes of the same load.
- The simulation raises `map_applied` from the `MapApplied` it is
  handed. `plan_from_map` is built with construction (16 §10.3).

## 5. Ground

### 5.1 Ground definitions

```json
{ "type": "ground", "id": "temperate_lowland",
  "params": { "moisture": 0.5 },
  "steps": [
    { "strata": [ { "depth": 1,  "block": "solid@soil",  "anchor": true },
                  { "depth": 4,  "block": "solid@clay",  "anchor": true },
                  { "block": "solid@granite", "anchor": true } ] },
    { "surface": { "floor": "grass" } },
    { "liquid": { "to": "sea_level", "block": "water" } },
    { "veins": { "block": "solid@iron_ore", "below": -6, "density": 0.02, "size": [3, 9] } },
    { "caves": { "below": -3, "density": 0.08, "width": [1, 3] } },
    { "scatter": { "map": { "tag": "tree" }, "density": "0.05 * $moisture", "apart": 2 } },
    { "scatter": { "fixture": "boulder", "density": 0.004 } } ] }
```

A ground definition is a list of **steps**, run in order for each tract.
Steps are implemented natively and configured by data.

| Step | Does |
|------|------|
| `strata` | Fills from the surface height down with layers of block states |
| `surface` | Lays a floor, and optionally low blocks, on the top |
| `liquid` | Fills open cells up to a level with still liquid (11 §4.3). `to` is `sea_level`, a storey, or `plan`: the level of whichever planned lake covers the cell (section 9.2). |
| `veins`, `pockets` | Replaces some of a stratum with another state, by noise |
| `caves` | Carves connected voids, in strata that anchor only (section 5.2) |
| `scatter` | Places maps or elements by density, with spacing |
| `paths` | Rasterises the roads, rivers, walls and rails that the region survey recorded as crossing this tract (section 8.3) |
| `float` | Raises islands of ground into the sky by noise |

Surface height comes from the realm plan's height field (section 9.2),
sampled per cell and rounded to storeys. Where neighbouring cells differ by
one storey, a ramp is placed.

**The definition.**

| Field | Meaning | Default |
|-------|---------|---------|
| `steps` | The steps, in order. Each is an object with one key, the step's name. | required |
| `params` | Parameters with their default values, each a boolean, a number or a text: `{ "trees": 0.02 }`. A tract definition may set them (section 6). The expressions of the steps see them as `$name`, in the generation context (section 3.4), with nothing else bound. | none |
| `depth` | How many storeys below the surface the ground is worked, from 1 to 128: veins, pockets and caves are made no deeper, and a `liquid` step looks no deeper for voids | 16 |
| `tags` | Free-form tags | none |

**The surface.** A column's surface is the lowest storey that is not
ground. Until realm plans are built it comes from the realm's generator
(section 6.2): its `height` field, sampled at each cell column and
rounded to the nearest storey, flattened under maps that sit by `level`
(section 4.2). The realm's `sea_level` (02 §4.1) is the lowest storey
that is not sea.

**`strata`**: a list of layers from the surface down. It is the first
step, and a ground has at most one; a ground with none is the realm's
`fill.below` up to the surface.

| Field of a stratum | Meaning | Default |
|--------------------|---------|---------|
| `block` | The block state it is made of. It must be `load_bearing` and not a liquid. | required |
| `depth` | How many storeys thick it is, from 1 to 256. Only the last stratum may leave it out: it then goes all the way down. If the last states one, the realm's `fill.below` lies under it. | all the way down |
| `anchor` | Whether it anchors (section 5.2). Stated, it selects the state of the block that has the flag or lacks it. | as the state written |

**`surface`**

| Field | Meaning | Default |
|-------|---------|---------|
| `floor` | The floor laid in the surface cell of every column | none |
| `block` | A low block put in the surface cell | none |
| `ramp` | The block put in the surface cell of a column where one of the four columns beside it is one storey higher. It must be flagged `ramp`. | no ramps |
| `where` | `land`: only columns whose surface is at or above the sea, or all of them in a realm with no sea. `sea`: only columns below it. | everywhere |

**`liquid`**

| Field | Meaning | Default |
|-------|---------|---------|
| `to` | `sea_level`: below the realm's sea level, and nothing in a realm with none. A number: below that storey. `plan`: the level of the planned lake over the cell; no lake is planned until realm plans are built, so nothing. | required |
| `block` | The still liquid put in every cell with no block from `depth` under the surface up to the level. It must be flagged `liquid`. | required |

A chunk that holds nothing but the liquid is stored as one value.

**`veins`, `pockets`**: a vein is a straight run of cells, one cell
thick; a pocket is a ball of cells.

| Field | Meaning | Default |
|-------|---------|---------|
| `block` | The state that replaces. It must be `load_bearing` and not a liquid. | required |
| `in` | The only state that is replaced | whatever the strata laid in the cell |
| `below` | The highest storey reached, counted from the surface: −1 is the top storey of the ground | −1 |
| `above` | The lowest storey reached, counted from the surface | `depth` under the surface |
| `density` | About what share of the ground in reach is replaced, from 0 to 1: a number or an expression | required |
| `size` | `[low, high]`, each from 1 to 16: the length of a vein, or the width of a pocket, in cells | `[3, 6]` |
| `if` | A condition; the step is skipped for a tract where it is false | always |

**`caves`**: a cave is a run of 36 cells that turns every 6, carved as
wide as its `width`.

| Field | Meaning | Default |
|-------|---------|---------|
| `below`, `above` | As for veins | −2; `depth` under the surface |
| `density` | About what share of the ground in reach is carved, before the rules of section 5.2 refuse any of it | required |
| `width` | `[low, high]`, each from 1 to 16: how wide a cave is, in cells | `[1, 3]` |
| `if` | As for veins | always |

**`scatter`**

| Field | Meaning | Default |
|-------|---------|---------|
| `map` | A map id, or `{ "tag": … }`, placed with its anchor in the surface cell, turned a quarter-turn chosen by position if it may be turned. Each map is at most 12 cells on a side. A scattered map does not shape the ground: it sits by `follow` if it says so, and otherwise where it is put. | one of `map` and the elements |
| `floor`, `block`, `fixture` | Element states put in the surface cell, where it has no block but a liquid and no fixture | one of `map` and the elements |
| `density` | The share of surface cells that get one, from 0 to 1: a number or an expression. With `apart` it is at most one cell in `(2 × apart + 1)²`. | required |
| `apart` | How many cells must lie between two of them, up to 8, across the edges of tracts as within them | 0 |
| `where`, `if` | As above | everywhere; always |

**`paths`** takes no fields. In the surface cell of each column a path
crosses it lays the path's floor (section 8.5: its surface, or the strip
along a side) and empties the fixture there, so a tree scattered before
it is cleared; a block there, such as a ramp, is kept, so a path follows
the ground. Nothing is laid under a map that levels or founds its ground.
The paths are the streets of the settlements near the tract and the
paths that cross the edges of the regions it touches (section 8.3), all
worked out from position, so two tracts agree on a path across their
edge. A ground without the step lays no path.

**`float`** is not built: a ground that uses it fails to load.

**Pure by position.** A vein, a pocket or a cave starts in a cell of a
coarse lattice, as many to a cell as the density asks, each drawn from
the positional hash of that lattice cell; a tract works out every one
that could reach it and keeps the cells inside itself. A scatter step
makes a cell a candidate by its own hash and keeps a candidate when no
candidate within `apart` cells has a lower hash. So two tracts agree on
whatever crosses their edge without either reading the other.

| Domain | Seeds |
|--------|-------|
| `gen.field.height` | The surface height field |
| `gen.ground.vein`, `gen.ground.pocket`, `gen.ground.cave` | The features of a lattice cell, by step |
| `gen.ground.scatter` | Whether a cell is a candidate, and its rank among them |
| `gen.ground.scatter_map` | Which map a scattered cell gets, turned how, and its seed |
| `gen.ground.expr`, `gen.tract.param` | The random functions of a step's and a tract's expressions, by tract |

### 5.2 What holds

Each stratum says whether its blocks are `anchor` (02 §5.4). The flag
belongs to a block's state, so a block used in loose strata defines a
state without it, and `"anchor": false` on a stratum selects that state.
The linter reports a stratum that asks for a state its block lacks. Rock that
anchors can be tunnelled freely. Sand, gravel and loose soil given
`"anchor": false` need support by span like any structure (02 §14), and a
tunnel through them comes down. This settles open question 2 of 02 §26: it
is a property of the ground, chosen per stratum.

Generation must leave nothing unsupported. The `caves` step carves only
where the stratum anchors, so a generated cave never hangs in loose ground
waiting for the first nearby edit to bring it down. A void in loose ground
is made by digging, in play, where the support rule applies from the first
cell removed.

The rules that keep it so:

- Every block of the ground (a stratum, a vein, a pocket) is
  `load_bearing`: ground stands on ground.
- A cave cell is carved only where the stratum laid there anchors, the
  block there now anchors, and the cell above holds a block that
  anchors or nothing at all. So no loose ground, no surface floor, no
  ramp and no liquid is left over a void, and a vein of rock through
  loose ground is no place for a cave.
- A vein or pocket of a state that does not anchor is laid one storey
  thick, between ground that anchors: never over a void, and never on
  or under more loose ground.
- A run of strata that do not anchor carries its own weight on its
  lowest storey (02 §14.5). A realm whose gravity would crush such a
  run of a ground it uses refuses the ground when its generator is
  bound, naming the ground and the storey that fails.

## 6. Tracts

```json
{ "type": "tract", "id": "meadow", "tags": ["open", "rural"],
  "ground": "temperate_lowland",
  "params": { "moisture": 0.7 },
  "ruin": { "transform": "scorched" },
  "bands": ["overland", "air"],
  "glyph": { "ch": "\"", "fg": "green" } }
```

A **tract definition** says what the ground of a tract is, which travel
bands may cross it (11 §13.2), what it becomes when ruined (section 10.2;
`ruin` may instead be keyed by kind of devastation, 02 §17.2),
and how it is drawn on the region map (12 §6.2). What stands on it is in
placements.

| Field | Meaning | Default |
|-------|---------|---------|
| `ground` | The `ground` definition its tracts are filled by | The realm's `fill.below` up to the surface, and nothing else |
| `params` | Values for the ground's parameters, by name: each a plain value or an expression, of the type of the parameter's default (a whole number is taken for a real). A text is an expression, so a text value is written `"'fen'"`. The expressions are worked out once for each tract, in the generation context with nothing bound; `region.*` joins it when regions are surveyed. | The ground's defaults |
| `place` | Maps that stand on every tract of the definition (below) | none |
| `ruin` | What it becomes when ruined: an object, `{ "transform": id }`, `{ "tract": id }`, or one of those under each kind of devastation. Carried as written until transforms are built. | none |
| `bands` | The travel bands that may cross it, by id as written | none |
| `glyph` | How it is drawn on the region map: `ch`, `fg`, `bg` | none |
| `tags` | Free-form tags | none |

A tract with dense id 0 has no definition (02 §9.1): the engine supplies
`engine:none`, a tract definition with no ground and nothing placed.

**`place`.** Until settlements lay out lots (section 8.4), a tract
definition may say what stands on its tracts. An entry is made, or not,
for each tract by the positional hash of the tract and the entry's
position in the list.

| Field of a `place` entry | Meaning | Default |
|--------------------------|---------|---------|
| `map` | A map id, or `{ "tag": … }` for any map carrying the tag | required |
| `at` | `[x, y]`: where the map's anchor goes, in cells east and south of the tract's north-west corner. The storey follows from the map's `base` (section 4.2). | required |
| `turn` | As a map's own `place` writes it (section 3.5) | not turned |
| `facing` | In place of `turn`: the direction the map's `front` is to face, `north`, `east`, `south` or `west`. The map is rotated so that it does, and never mirrored. | none |
| `chance` | The chance that it is placed at all, from 0 to 1 | 1 |
| `params` | Parameters for the map: plain values | none |
| `order` | The placement's `order` (section 4.1) | Its position in the list |

A placement of a tract definition lies wholly inside its tract. Of the
maps a tag gives and the turns `"any"` allows, those that fit are the
ones chosen among; if none fits, or the map cannot be resolved with the
parameters given, the definition fails to load at the entry. Placements
are tested for overlap in the order of the list (section 4.1).

### 6.1 Realising a tract

In order, as a pure function of the tract's record, its neighbours'
records and the placements overlapping it:

1. Ground steps.
2. Levelling and foundations for placements.
3. Placements in `order`, each clipped to the tract, with their nested maps.
4. Transforms that apply to the tract or its realm (section 10).
5. The result: chunks, fills, spawns, areas, portals, subgrids (02 §9.3).

What follows (stamps, pending edits, catching up) is 02 §9.4.

As built, levelling is done before the ground is filled: the surface
under a map that sits by `level` is its level 0 when the ground's steps
run, so that the ground lays its own strata, surface and ramps there
(section 4.2). Foundations are made after the ground and before any
placement is written. Each cell's content is still decided by one rule
of precedence: ground, then placements in order, then transforms
(invariant X5). Transforms are not built: the step is a function that
does nothing yet. Portals and subgrids are not handed on yet.

### 6.2 The `standard` generator

Until world generators and region kinds are built (sections 8 and 9), a
realm definition names the generator `standard` and says in its
`generator_params` what its land is made of (02 §9.3):

```json
{ "type": "realm", "id": "countryside", "generator": "standard",
  "generator_params": {
    "tracts": [ { "tract": "field", "weight": 5 }, { "tract": "wood", "weight": 3 } ],
    "patch": 2,
    "layout": { "at": [0, 0], "rows": [ "wff ", "llll", "fwf " ],
                "legend": { "f": "field", "w": "wood", "l": "lane" } },
    "height": { "noise": { "octaves": 3, "scale": 8 }, "range": [0, 3] } } }
```

| Parameter | Meaning | Default |
|-----------|---------|---------|
| `tracts` | The tract definitions the land is made of, each `{ "tract": id, "weight": number }`. Every tract is given one by weighted choice, from the positional hash of its patch. | Tracts with no definition |
| `patch` | Tracts along the side of a patch, from 1 to 64: the tracts of a patch are given the same definition | 1 |
| `layout` | Tract definitions written as rows of characters (section 9.3): `at` is the tract of the first character of the first row, `rows` run north to south, `legend` says which definition each character stands for, and a space leaves the tract to `tracts` | none |
| `height` | The surface: a noise field (section 9.1), sampled at each cell column at its position in tracts and rounded to the nearest storey | flat |
| `surface_z` | The storey the surface is counted from: added to `height` | 0 |
| `place` | Maps placed by hand anywhere in the realm (section 9.3): each `{ "map": id, "at": [x, y] }`, or `[x, y, z]` to say the storey of its level 0, with `turn` in degrees clockwise, `mirror`, `seed`, `params` (plain values) and `order` (its position in the list if left out). A seed left out is the positional hash of the cell. | none |
| `regions` | The region kinds (section 8.1) the realm's regions are given, each `{ "kind": id, "weight": number }`: every region is given one by weighted choice, from the positional hash of the region. A region of a kind takes its tracts' definitions from the kind, in place of `tracts`. Until world generators are built (section 9) this is the realm plan's choice of kinds. | Regions with no kind |
| `settlements` | Settlements (section 8.4) placed by hand, each `{ "settlement": id, "at": [x, y] }` with `at` its north-west tract, and `seed` (drawn from the realm's seed and `at` if left out). Each must lie inside the realm and meet no other. Until realm plans are built this is the plan's list of settlements. | none |

The sea is the realm definition's own `sea_level` (02 §4.1). A wrong
parameter is reported when the generators are bound, with the realm
definition and the parameter's path; the linter binds them.

**Surveying.** A region's survey gives each tract its definition and
writes the placements: the realm's own that touch the region, then each
tract's. It is a pure function of the seed and the position (invariant
X2). A placement dropped for overlap is not written; the generator
lists them for a region on request.

**The seam.** Region kinds (section 8.1) replace the weighted choice of
a tract's definition; settlements and sites (sections 8.4 and 11) add
placements. Whatever replaces them must stay a pure function of the
seed and the position, and must give a tract's realiser every placement
that overlaps the tract or lies within 12 cells of it, so that a
levelled lot beside the tract can shape a ramp inside it.

As built, the generator works out a **region plan** (a region's kind,
the paths crossing its edges and its kind's sites) and a **settlement
plan** (streets, blocks, lots and sites) as pure functions of the seed
and the position, and keeps those it has worked out. A realiser asks
for the plans of the regions and settlements near its tract and takes
every lot, site and path within reach; a survey asks for the same plans
and writes them into the region's record. A tract's definition is, in
order: the `layout`'s; the `tract` of a settlement whose extent holds
it; the region kind's; the realm's `tracts`. A tract definition places
nothing on a tract inside a settlement: the lots are what stands there.

## 7. Building large places from parts

### 7.1 Stacks

```json
{ "type": "building", "id": "office_tower",
  "footprint": [24, 24],
  "params": { "floors": { "range": [12, 40] } },
  "stack": [
    { "map": "tower_basement", "level": -1 },
    { "map": "tower_lobby" },
    { "map": { "tag": "office_floor" }, "repeat": "$floors", "vary": true },
    { "map": "tower_plant_floor" },
    { "map": "tower_roof" } ],
  "core": "tower_core" }
```

A **building** stacks single-storey maps. A repeated entry may vary its
choice and parameters from floor to floor. The `core` map (stairs, shafts,
risers for conduits) is repeated through every storey so the floors
connect. A forty-storey tower is five maps.

A floor map may leave a zone for the **partition** step, which divides it
into rooms by rule, cuts doors, and furnishes each room with a map chosen
by tag and size.

### 7.2 Assemblies

```json
{ "type": "assembly", "id": "old_crypt",
  "start": "crypt_stair",
  "pieces": [ { "map": { "tag": "crypt_room" }, "weight": 3 },
              { "map": { "tag": "crypt_passage" }, "weight": 5 },
              { "map": "crypt_vault", "max": 1, "min_depth": 5 } ],
  "size": [18, 40],
  "loops": 0.15,
  "cap": "crypt_dead_end",
  "bounds": { "box": [[-40, -40, -3], [40, 40, -1]] },
  "params": { "danger": "$depth / 3" } }
```

An **assembly** grows a place from pieces joined at their **joints**. A
joint is a labelled opening on a map's edge with a direction and a kind:
a doorway, a corridor mouth, a hatch, a stair up or down. The assembler
starts with one piece and repeatedly attaches a piece whose joint matches
an open one, turning it to fit, rejecting overlaps and anything outside
the bounds, until the size is reached. Open joints left over are capped.

- `loops` is the chance of joining two open joints that meet, making
  circuits instead of a tree.
- Pieces may have limits: how many, how deep from the start.
- Each piece receives `$depth`, its distance from the start, so contents
  can change with it.
- Vertical joints make multi-level places.

Dungeons, mines, sewers, cave systems, space stations and the corridors of
a great ship are assemblies of different pieces. An assembly's result is a
list of placements, computed once from its seed and kept with its site.

## 8. Regions

### 8.1 Region kinds

```json
{ "type": "region_kind", "id": "farmland",
  "tracts": [ { "tract": "field",  "weight": "4 * $fertility" },
              { "tract": "meadow", "weight": 2 },
              { "tract": "copse",  "weight": "1 + 3 * $moisture" } ],
  "patch": 6,
  "sites": [ { "site": "farmstead", "per_region": [3, 8], "apart": 8 },
             { "site": "roadside_shrine", "per_region": [0, 1], "near": "road" } ],
  "settlements": ["hamlet", "village"],
  "weather": "temperate_inland",
  "population": { "countryfolk": 0.2 },
  "glyph": { "ch": ".", "fg": "yellow" } }
```

A **region kind** is the recipe for surveying one region: which tract
definitions its ground is made of and in what proportion, how large the
patches of each are, which sites occur in it and how often, which
settlements it may hold, its climate (09 §14.1), and its thin scattering of
people outside any site.

**As built** (decision D-670). The size of a patch is `patch_size`: `patch` is a word the
loader keeps for merging definitions (04 §6.3). A weight is a number
until world generators give fields (section 9.1); the example's
expressions wait for them.

| Field | Meaning | Default |
|-------|---------|---------|
| `tracts` | The tract definitions its ground is made of: each `{ "tract": id, "weight": number }`, a weight not negative, one if left out. Every tract is given one by weighted choice from the positional hash of its patch. | Tracts with no definition |
| `patch_size` | Tracts along the side of a patch, from 1 to 64: the tracts of a patch are given the same definition | 1 |
| `sites` | The sites it holds: each `{ "site": id, "per_region": count, "apart": tracts, "near": where }`. `per_region` is a whole number or `[low, high]`, drawn for each region. `apart` (0 to 64) is how many tracts must lie between two sites of the entry: their tracts differ by more than that, east to west or north to south. `near` is `road` (within a tract of a path, and never on one), `centre` (in the middle half of the region) or `edge` (in its outer eighth); `coast` and `river` wait for realm plans and fail to load. | none |
| `settlements` | The settlements a realm plan may put in a region of the kind, by id. Carried until realm plans are built (WG6); the realm generator's `settlements` place them meanwhile. | none |
| `paths` | The paths that cross its edges (section 8.3): `{ "path": path kind, "per_edge": count }`, the count a whole number or `[low, high]`, at most 8 | none |
| `weather` | Its climate (09 §14.1), by id as written. Carried. | none |
| `population` | People outside any site, by population id and density. Carried for WG9. | none |
| `glyph` | How it is drawn on the realm map: `ch`, `fg`, `bg` | none |
| `tags` | Free-form tags | none |

### 8.2 Surveying a region

As a pure function of the realm's seed, the region's position and the realm
plan:

1. Take from the plan this region's kind, its field values, the routes and
   rivers that cross it, and any sites and settlements planned for it.
2. Assign a tract definition to each tract.
3. Lay out settlements (section 8.4).
4. Place the kind's other sites, by their counts and constraints.
5. Add minor features that cross its edges (section 8.3).
6. Write the result: tract records, placements, path lines, sites, and the
   seed for the simulation (section 11).

Sites created by a region's survey lie wholly inside it. A site that
crosses regions comes from the realm plan, and every region it touches
reads the same record.

**As built** (`clash_gen::region`, decisions D-670 and D-672 to D-674): there is no
realm plan yet. The region's kind comes from the realm generator's
`regions` and its settlements from the generator's `settlements`
(section 6.2). Paths are laid out before the kind's sites, so that a
site can be `near` a road: step 5 comes before step 4. A site of the
kind is tried in the region's tracts in an order drawn from their
positional hashes; it is placed in a tract no other site takes, outside
every settlement and the tracts beside one, not crossed by a path, and
apart from the sites of its entry as the entry says; its map, chosen
among the site's by the positional hash, turned a quarter-turn the hash
chooses, lies in the middle of the tract and must meet nothing the
tract's definition or the realm places (invariant X10). Fewer placed
than drawn is a shortfall in the survey's report. The record lists the
region's kind, every tract's definition, the placements (the realm's,
the tract definitions', the lots of every settlement that reaches the
region and the kind's sites), the path lines that cross it (each the
cells of its centre line inside the region, at the surface the height
field gives) and the sites: the kind's, and the `must_have` sites of
settlements whose box begins in the region.
`StandardGenerator::survey_report` gives the report: placements dropped
for overlap, `must_have` sites that could not be placed, and
shortfalls.

### 8.3 Agreeing at the edges

Two neighbouring regions are surveyed separately and must match.

- **Planned paths** (great roads, rivers, borders, rails) are lines in the
  realm plan in realm coordinates. Each region draws the part inside it.
- **Minor paths** cross an edge at points derived from a hash of that edge,
  which both regions compute alike without consulting each other.
- **Street grids** of a settlement spanning regions are keyed to the
  settlement's own origin and angle, so they line up.
- **Height and other fields** are functions of position.

**As built** (decision D-672). A path between two neighbouring regions crosses their
edge at points drawn from the positional hash of the edge itself (the
eastern or southern region's position and which way the edge runs), at
least an eighth of a region from either corner. How many cross is drawn
between the lower of the two kinds' `per_edge` lows and the lower of
their highs; the path is of the western or northern region's kind; a
region whose kind has no `paths`, and an edge of the realm, has none.
Each region lays its own part: from the crossing, straight in from the
edge to the row or column of the region's **hub**, then along it to
the hub. The hub is the centre of the first settlement whose centre
lies in the region, or else a cell of the region's middle half drawn
from its hash. A path is not laid inside a settlement, whose ring of
streets it meets. Neighbours never read each other, and agree because
both work out both kinds and the edge's hash alike (invariant X3).

A path's line is kept as straight stretches of its centre line, each
running east to west or north to south; the path is as wide as its
kind, about the line.

### 8.4 Settlements

```json
{ "type": "settlement", "id": "market_town",
  "size": [14, 30],
  "streets": { "pattern": "organic", "spacing": [28, 50], "main": 2 },
  "zones": [ { "zone": "centre", "to": 0.25, "lots": ["shop", "inn", "civic"] },
             { "zone": "town",   "to": 0.7,  "lots": ["house", "workshop", "shop"] },
             { "zone": "edge",   "to": 1.0,  "lots": ["house", "yard", "small_farm"] } ],
  "must_have": [ "town_hall", "watch_house", { "site": "market", "at": "centre" } ],
  "wall": { "chance": 0.4, "map": "town_wall_segment", "gates": "on_main" },
  "groups": [ "town_council", "town_watch", { "group": "street_gang", "chance": 0.5 } ],
  "population": { "townsfolk": "by_beds" } }
```

A **settlement** definition lays out streets, divides the land between
them into lots, and fills the lots.

| Part | Does |
|------|------|
| `size` | Extent in tracts |
| `streets` | A pattern (`grid`, `organic`, `radial`, `linear`, `scatter`) with spacing and how many main roads it joins to |
| `zones` | Rings or districts by distance from the centre, each naming the tags of maps and sites that may fill its lots |
| `must_have` | Sites every instance has, optionally with where |
| `wall` | A boundary assembled from a segment map, with gates where main roads cross |
| `groups` | The bodies that exist here (section 11.3) |
| `population` | How many people, usually by counting beds |

A lot is filled by a map or site whose tags fit the zone and whose
footprint fits the lot, turned so its `front` faces the street. Large
cities are several settlement definitions used as districts under one plan
entry.

A hamlet, a walled city, a space colony's dome, a goblin warren and a
military camp are settlement definitions with different patterns and maps.

**As built** (`clash_gen::settlement`, decisions D-675 and D-676):

| Field | Meaning | Default |
|-------|---------|---------|
| `size` | `[width, depth]`: its extent in tracts, each from 1 to 64 | required |
| `tract` | The tract definition its tracts are given, in place of the region's choice | the region's |
| `streets` | Its streets (below) | required |
| `gap` | Cells left between two buildings side by side, from 0 to 8 | 1 |
| `zones` | Its rings, from the centre out (below) | required |
| `must_have` | Sites every instance has: a site id, or `{ "site": id, "at": where }` with `at` one of `centre`, `edge` and `anywhere` | none |
| `wall` | A wall round it. Not built: a settlement with one fails to load. | none |
| `groups` | The bodies that exist here (section 11.3). Carried for WG9. | none |
| `population` | How many people live here. Carried for WG9. | none |
| `home_mark` | The mark that counts as a home's bed (section 13). Carried for WG9. | none |
| `tags` | Free-form tags | none |

| Field of `streets` | Meaning | Default |
|--------------------|---------|---------|
| `pattern` | `grid` or `organic`. `radial`, `linear` and `scatter` are not built and fail to load. | required |
| `spacing` | `[low, high]`: cells from one street to the next. `low` is at least the widest street's width and four cells more. | required |
| `path` | The kind of path (section 8.5) its streets are | required |
| `main` | How many main roads cross it through its centre: 0, 1 (east to west) or 2 (and north to south) | 0 |
| `main_path` | The kind of path its main roads are | `path` |

| Field of a zone | Meaning | Default |
|-----------------|---------|---------|
| `zone` | Its name | required |
| `to` | How far out it reaches: a share of the way from the centre to the edge, measured as the larger of the two, above the zone before it and at most 1. The last zone's is 1. | required |
| `lots` | The tags of the maps that may fill its lots. A map fills a lot only if it carries one of them and names a `front`. | required |
| `people` | Who lives or works in each of its lots (section 11.2) | none |

The layout, a pure function of the definitions, the settlement's
position and its seed:

1. **Streets.** Streets running east to west cross the whole extent,
   the first and last along its north and south edges, as many between
   as the spacing allows. Between two of them lies a row of blocks, cut
   by streets running north to south, the first and last along the west
   and east edges. With `grid` every row is cut at the same places and
   the spacing is even, one spacing drawn for each direction. With
   `organic` each row draws a spacing of its own and every street
   between the first and the last is moved by up to a fifth of the
   spacing, so junctions are staggered and blocks differ; a move that
   would leave a block too small is not made. Every street meets one
   that runs the whole width, so all streets join. The middle street of
   each direction is a main road; with `organic` the main road north to
   south runs straight through every row.
2. **Blocks and lots.** The land between streets is a block; one of
   fewer than four cells either way is left open. A block deep enough
   for its zone's shallowest map twice over has a row of lots along its
   north side, fronting the street to the north, and one along its
   south side, fronting the street to the south; another has one row,
   fronting north. A block's zone is the first whose `to` its centre
   lies within.
3. **Sites.** Each `must_have` site, in order, takes the start of a row:
   the row nearest the centre for `centre`, the farthest for `edge`,
   one drawn from the seed for `anywhere`. A tract holds one site at
   most (02 §8.4), so a site takes every tract its map covers, and is
   put only where no site before it took one of them, where its map
   lies in one region, and where it meets nothing the realm places. A
   site that cannot be placed is reported, and the settlement is laid
   out without it.
4. **Filling.** Along each row, from its west end, a map is drawn by
   the positional hash among the zone's maps that fit what is left of
   the row, turned so that its `front` faces the street (never
   mirrored); its front edge lies on the street. Then `gap` cells are
   left and the next is drawn, until none fits. Every map's level 0 is
   set by its `base`, at its anchor; one that meets a placement the
   realm makes by hand is dropped and reported.

A settlement's streets are path lines of its kinds, laid by the
`paths` step of its tracts' ground (section 5.1). Its lots and sites
are placements. Every region it reaches and every tract near it works
out the same plan, so a settlement across a region's edge has the same
streets on both sides.

### 8.5 Path kinds and crossings

What a path is made of, and what it does when it meets something, is a
definition.

```json
{ "type": "path_kind", "id": "road",
  "width": 3, "surface": { "floor": "paving@stone" }, "max_grade": 0.25,
  "crossings": [
    { "over": "liquid",    "use": { "maps": { "tag": "bridge_span" }, "ends": { "tag": "bridge_end" } } },
    { "over": "path:rail", "use": { "maps": { "tag": "level_crossing" } } },
    { "when": "rise > 2 and length < 40", "use": "tunnel",     "lining": "wall@stone" },
    { "when": "rise > 0",                 "use": "cutting" },
    { "when": "rise < 0",                 "use": "embankment", "fill": "solid@soil" } ] }
```

A path follows the ground while the ground is no steeper than `max_grade`,
in storeys per cell along the line. Where it is steeper, or the line meets
liquid or another path, the first crossing that applies is used for that
stretch.

| Crossing | Does |
|----------|------|
| Maps | Places the named maps end to end along the line from one side to the other, with end pieces if given |
| `tunnel` | Carves the path's width through the rise at a steady grade, lined if a lining is given |
| `cutting` | Cuts the ground down to the path's grade, open to the sky |
| `embankment` | Fills under the path up to its grade |

A path kind that is a watercourse may state a `current`: the speed at
which its water moves along its line (11 §4.7).

Crossings are decided in the survey from the line and the height field,
which neighbouring regions share, so both sides of an edge choose alike.
Map crossings are written as placements. The others are part of the path
record and are cut or filled by the `paths` step.

**As built** (decision D-671):

| Field | Meaning | Default |
|-------|---------|---------|
| `width` | How many cells across its surface is, from 1 to 15 | required |
| `surface` | `{ "floor": state }`: the floor laid along it | required |
| `edge` | `{ "floor": state, "width": cells }`: a strip of another floor along each side, 1 to 8 cells across (one if `width` is left out): a pavement | none |
| `max_grade` | The steepest it follows the ground. Carried: crossings are not built. | none |
| `crossings` | Carried as written: crossings are not built, and a path lies on the ground whatever it meets | none |
| `current` | For a watercourse, metres a second. Carried. | none |
| `glyph` | How it is drawn on the region map | none |
| `tags` | Free-form tags | none |

A path is its surface with an edge strip on each side, centred on its
line; an even width leans east or south.

## 9. Realms

### 9.1 World generators

A realm definition's `generator` names a **worldgen** definition.

```json
{ "type": "worldgen", "id": "temperate_island",
  "fields": {
    "height":   { "noise": { "octaves": 5, "scale": 90 }, "shape": "island", "range": [-12, 30] },
    "moisture": { "noise": { "octaves": 3, "scale": 40 } },
    "fertility": "clamp(moisture * (1 - abs(height) / 30), 0, 1)" },
  "kinds": [ { "if": "height < 0",              "kind": "sea" },
             { "if": "height > 20",             "kind": "upland" },
             { "if": "moisture > 0.6",          "kind": "woodland" },
             { "kind": "farmland" } ],
  "settlements": [ { "settlement": "market_town", "count": [1, 2], "on": ["farmland"], "near": "coast" },
                   { "settlement": "village",     "count": [4, 9], "apart": 6 } ],
  "sites": [ { "site": "old_crypt_site", "count": 1, "on": ["upland", "woodland"] } ],
  "routes": { "join": "settlements", "kind": "road" },
  "rivers": { "count": [2, 5], "from": "height > 15" },
  "lakes":  { "count": [0, 3], "where": "height > 2 and moisture > 0.5", "size": [4, 20] },
  "holders": [ { "group": "island_lord", "seat": "largest_settlement", "holds": "all" } ],
  "wasteland": "ash_waste" }
```

| Part | Gives |
|------|-------|
| `fields` | Named values at every position: noise with a shape, or expressions of other fields |
| `kinds` | The region kind for each region, by the first rule that holds |
| `settlements`, `sites` | What to plan, how many, and where they may go |
| `routes`, `rivers` | Lines to plan: which places to join, where rivers rise |
| `lakes` | Basins to plan: how many, where, and how wide in tracts |
| `holders` | Groups that hold territory, and what they hold |
| `wasteland` | What totally devastated tracts become (02 §17.2) |

**Noise fields.** A field written as an object is noise, given a large
form by its `shape` and stretched over its `range`:

| Key | Meaning | Default |
|-----|---------|---------|
| `noise.octaves` | How many layers of noise are added, each finer and weaker than the last: 1 to 16 | required |
| `noise.scale` | The distance across the broadest features, in tracts | required |
| `noise.lacunarity` | How much finer each layer is than the last: at least 1 | 2 |
| `noise.gain` | How much weaker each layer is than the last: above 0, at most 1 | 0.5 |
| `shape` | The large form the field takes over the realm | `flat` |
| `range` | The lowest and the highest value the field takes | `[0, 1]` |

A field is sampled at a position measured in tracts, with fractions for
positions inside a tract. Its noise is keyed by the realm's seed and the
field's name, so two fields of one realm are unrelated, and the same
field in two realms is too.

The noise `u` runs from 0 to 1. It is a sum of layers of gradient noise:
layer `i` is read at the position divided by `scale` and multiplied by
`lacunarity^i`, and is weighted by `gain^i`. The sum is divided by the
sum of the weights and moved from the range −1 to 1 to the range 0 to 1.
Its mean is one half and most of it lies near that: the ends of the range
are rare, and rarer the more octaves there are.

The shape turns `u` into `s`, also from 0 to 1. Two shapes use the
realm's extent through `b = (1 − nx²)(1 − ny²)`, where `nx` and `ny` are
the position's offsets from the middle of the realm as fractions of the
way to its edge. `b` is 1 at the middle and 0 all along the edges.

| Shape | `s` | Gives |
|-------|-----|-------|
| `flat` | `u` | The noise alone, with no large form |
| `island` | `b · (1 + u) / 2` | High ground in the middle, and the bottom of the range all along the realm's edges |
| `continent` | `u · ⁴√b` | The noise over most of the realm, sinking to the bottom of the range only close to the edges |
| `ridged` | Each layer `n` of the sum replaced by `(1 − abs(n))²` | Sharp crests with broad valleys between, and no large form |

The field's value is `low + s · (high − low)`. In the example, the middle
of the island is never below 9, half way up its range, and its whole edge
is at −12, so `height < 0` puts sea all round it.

A description with a number outside these limits, a `range` whose upper
end is below its lower, or a key that is not in the table fails to load
with a reason.

### 9.2 The realm plan

When a realm is created its plan is computed once and stored with it:

- The fields are functions; nothing is stored for them.
- Settlements and planned sites are given positions that satisfy their
  rules, in a fixed order from the realm's seed.
- Routes are found between them over region kinds, and rivers run downhill
  to the sea. Both are stored as lines.
- Lakes are stored as outlines, each with a water level of its own: the
  height of the lowest point on its rim. A river that reaches a lake ends
  there, and one leaves from that lowest point.
- Holders are given their settlements and the regions nearest them.

The plan is small: a list of places, a list of lines, a table of holdings.
It is the only step that looks at the whole realm, and it works at the
scale of regions.

### 9.3 Authored realms

A worldgen may instead be given outright:

```json
{ "type": "worldgen", "id": "arena",
  "layout": { "ground": "bare_stone",
              "place": [ { "map": "great_arena", "at": [0, 0, 0] } ],
              "sites": [ { "site": "arena_site", "map": "great_arena" } ] } }
```

or as a grid of region kinds or tract definitions written as rows of
characters, like a map. A scenario's single location, a pocket dimension, a
hand-drawn island and a ship adrift in a void are authored realms.

Authored and generated parts mix: a generated island with one hand-made
castle named in its `sites`.

### 9.4 Realms on demand

A chart node (02 §20.3) may name a realm definition and a seed rule instead
of an existing realm. The realm is created the first time someone arrives.
A galaxy of worlds, a tower of a hundred floors and an endless dungeon are
charts whose nodes are made when reached.

## 10. Ruin and transformation

### 10.1 Transforms

```json
{ "type": "map_transform", "id": "long_abandoned",
  "steps": [
    { "replace": { "match": { "fixture_tag": "lamp" }, "with": {}, "chance": 0.8 } },
    { "replace": { "match": { "block": "window" }, "with": [ { "block": "window#broken", "weight": 3 }, {} ] } },
    { "replace": { "match": { "floor_tag": "paved" }, "with": { "floor": "cracked_paving" }, "chance": 0.5 } },
    { "damage": { "share": 0.15, "amount": [0.3, 1.0] } },
    { "settle": true },
    { "scatter": { "map": { "tag": "weeds" }, "density": 0.1 } },
    { "strip": { "items": 0.7, "spawns": 1.0 } },
    { "add_spawns": [ { "population": "scavengers", "density": 0.01 } ] } ] }
```

A **transform** rewrites cells after a tract is realised.

| Step | Does |
|------|------|
| `replace` | Swaps elements that match for others, with a chance |
| `damage` | Damages or removes a share of elements |
| `settle` | Runs structural support in resolve mode (02 §14.4), so what was undermined falls |
| `scatter` | Adds maps and elements |
| `strip`, `add_spawns` | Removes and adds contents and inhabitants |
| `fields`, `decals` | Adds them |

### 10.2 Where transforms are used

| Use | How |
|-----|-----|
| **Ruin variants** | A tract definition's `ruin` names a transform or a substitute definition, used when a never-realised tract is totally devastated (02 §17.2). It may give one for each kind of devastation: scorched, drowned, shaken, buried. |
| **Lesser devastation** | A devastation kind's `apply` transform is run on tracts that were devastated by something other than a blast, scaled by how badly (02 §17.2) |
| **Derived realms** | A derived realm's `transform` (02 §21) is applied to every tract it copies. The same city, a century abandoned. The same city, wrong. |
| **Variation** | A region kind or placement may apply a transform: a snowbound village, a burnt quarter |
| **Rules** | `transform_area` applies one to loaded cells |

## 11. What generation gives the simulation

The world does not know about people. A survey's result carries a **seed
for the simulation**, which the simulation receives with `region_surveyed`
and turns into records (09).

### 11.1 Sites

```json
{ "type": "site", "id": "watch_house", "tags": ["civic", "watch"],
  "map": "watch_house_a",
  "roles": { "post": "mark:post", "cell": "mark:cell", "desk": "mark:desk", "entrance": "mark:entrance" },
  "staff": [ { "creature": "watch_sergeant", "role": "sergeant", "notable": true, "home": "nearby" },
             { "creature": "watchman", "role": "guard", "count": [3, 6], "home": "nearby" } ],
  "owner": "town_watch",
  "stocks": { "rations": 40 },
  "supplies": [],
  "incidents": ["jailbreak"] }
```

| Field | Gives the simulation |
|-------|----------------------|
| `map`, `building` or `assembly` | What stands there |
| `roles` | Named places inside it, from the map's marks. Routines (09 §10.2) and incidents (09 §11) refer to these. |
| `staff` | Who works there. Those marked `notable` become agents (06 §12); the rest are counted in the site's population. |
| `owner` | The group or character that holds it to begin with (09 §13.4) |
| `stocks`, `supplies` | 09 §13 |
| `incidents` | Kinds of incident that may happen here beyond those selected by tag |

Marks are known from the map and the placement alone, so a site's roles
exist before any of it is realised.

A definition flagged `founded` is for sites made in play (09 §13.5). It
needs no map, and its roles are given places afterwards.

**As built** (decision D-677), the part generation needs. `map` is a map
id or `{ "tag": … }`; a map stands for a site only if it names a
`front` and every role is one of its marks that is the same in every
placement (section 3.7), and a map named by id that lacks one fails to
load at the role (invariant X7). `roles` maps a name to `"mark:name"`.
`staff` lists people as section 11.2 does. `name` is how it is shown,
its id if left out. `founded` sites have no map and cannot be placed by
a survey or a settlement. `owner`, `stocks`, `supplies`, `incidents`,
`building` and `assembly` are carried for WG9 and WG10. A site the
world makes spans every tract its map covers, and its anchor is where
its map's anchor goes.

### 11.2 People

- **Homes.** A map's bed marks give it a capacity. A settlement's
  population is housed in its homes; staff with `"home": "nearby"` are given
  a bed in a home near their work.
- **Presence.** A population definition (06 §12) carries a table of where
  its members are by time of day: at home, at work, about. When a tract
  loads, the anonymous individuals instantiated in each site and on each
  street follow from it. Streets are busy at noon and empty at three in the
  morning.
- **Agents.** Notable staff are created as stored agents with their
  routine, home and workplace.

**As built** (decision D-678): a zone's `people` and a site's `staff`
ask for people in each lot of the zone and in the site's map. Each entry
is `{ "creature": id, "count": n or [low, high], "chance": share,
"in": area, "at": "mark:name", "role": text }`: one if `count` is left
out, always if `chance` is; in the areas of that name of the map, or at
the first cell of the mark, or anywhere in the map if it has no such
area or mark, or if neither is given. How many, and whether any, are
drawn from the placement's seed. The realiser adds them to the placed
map's own spawns (section 4.1), so each becomes a spawn request at a
cell of open floor, handed over by the tract the cell is in, `what`
being the id as written. The simulation answers a request that names a
template with an anonymous individual (06 §10.4). `role` is carried in
the definition; homes, presence and agents wait for WG9.

### 11.3 Groups

A settlement's `groups` and a worldgen's `holders` are created as groups
(15 §9.2), with members drawn from staff by role, their sites assigned, and
their relations to each other set from the group definitions.

### 11.4 Starting in motion

A worldgen may ask for a **settling time**: the cold simulation is run for
that long when the realm is created, before anyone arrives, so routines are
out of step, a few incidents have happened and are known about, and stocks
are no longer at their starting values.

### 11.5 Names

```json
{ "type": "names", "id": "coast_folk",
  "patterns": [ { "is": "{given} {family}", "weight": 4 }, { "is": "{given} of {place}" } ],
  "parts": { "given":  { "list": ["Mara", "Tobin", "Ysolde"] },
             "family": { "join": [["Ash", "Hal", "Wen"], ["wick", "more", "by"]] } } }
```

A **names** definition makes names from patterns and parts: lists, pieces
joined together, or a file of one name per line. Templates name their
people from one, and settlements, sites and vessels may too. `{place}` and
the like are filled by whoever asks. A name is drawn from a positional
seed, so the same world has the same names. `make_name(names, seed)` is
available to rules.

## 12. Tools

All in `clash-tools`.

| Command | Does |
|---------|------|
| `map show <id>` | Prints a map storey by storey for a seed and parameters, with nested maps resolved |
| `map check <mods>` | The checks of section 3.7 |
| `gen region <realm> <x> <y>` | Surveys a region and prints its tract map, placements and sites |
| `gen plan <realm>` | Prints a realm plan as a map of region kinds with its places and lines |
| `gen assembly <id>` | Builds an assembly for a seed and prints its layout |

Maps reload with everything else (04 §11), so a change to a house can be
seen in a running game.

Until `map show` is built, an example of the generation crate prints a
map of the bundled mods, storey by storey, one character for each cell
from the glyph of what the cell holds:

```bash
cargo run -p clash-gen --example map_ascii -- <map id> [seed] [turn]
```

`turn` is `0`, `90`, `180` or `270`, with an `m` before it to mirror
first. A space is a cell the map leaves alone, and `:` a cell it empties.

Until `gen region` is built, another example prints realised tracts of
a realm of the `standard` generator the same way, seen along the
ground:

```bash
cargo run -p clash-gen --example tract_ascii -- <realm id> [seed] [tract x] [tract y] [storey] [width] [height]
```

`tract x` and `tract y` name the north-west tract printed, and `width`
and `height` how many tracts are printed east and south from it (2 and
1 if left out). `storey` counts up from the surface of each cell
column, so 0 is what stands on the ground however the ground rolls.
The generated town of `core:town` lies across tracts 60 to 67 east and
28 to 35 south: `tract_ascii -- town 1 62 30 0 3 2` prints its centre.

## 13. Modding surface

**Definition types**: `map`, `palette`, `ground`, `tract`, `building`,
`assembly`, `region_kind`, `settlement`, `site`, `worldgen`,
`map_transform`, `path_kind`, `names`. Field `presence` on populations.

**Events**: `realm_planned` (a realm's plan is computed), `region_surveyed`,
`tract_realized` (a tract's cells were generated for the first time),
`site_created`, `map_applied` (by `apply_map` or a pending edit),
`placement_dropped` (a survey could not fit something, with what and
why).

`survey_region` surveys a region now instead of on first need.
`here.placement` is the placement whose map made the cell, if any.

A survey reports what it could not do. A `must_have` site or a planned
site that cannot be placed is an error in the survey's report, and the
settlement is laid out without it. A map chosen by tag for a site must
supply the site's roles, or it is not eligible (invariant X7); the same
holds for the pieces of an assembly taken together.

**Named choices.** `near` is one of `coast`, `road`, `river`, `centre`,
`edge`. A field's `shape` is one of `island`, `continent`, `flat`,
`ridged` (section 9.1). A holder's `seat` is `largest_settlement`, `first`, or a named
settlement. A worldgen names which of its fields is the surface height
(`"surface": "height"`). A settlement names the mark that counts as a
home (`"home_mark": "bed"`); the engine does not know what a bed is.
Fields are read as `region.<field>` everywhere; `$name` is for parameters
only. A generator of type `derived` copies another realm (02 §21).

**Fixed sets.** Ground steps, transform steps, crossings and street
patterns are implemented natively and configured by data. A mod cannot
add one. This is a known limit of the modding surface. So are the presets
a level may name (`roof`), and the layers `clear` and `keep` take.

**Gaps.** A field's noise is always a sum of gradient noise, with the
four shapes of section 9.1. Value noise, billowed noise, cellular noise
(patches round scattered points), noise in three dimensions and domain
warping exist natively for ground steps to use and cannot be asked for
in a field.

Gaps of placing and of ground: a placement does not go round a wrapped
edge of a realm, and the height field does not join across one. A map
levelled in the world as it is leaves steps without ramps. An
overlapping placement of a tract definition is dropped, never moved. A
ground's `float` step is not built and its `paths` step draws nothing
yet. With `apart`, a `scatter` step reaches at most one cell in
`(2 × apart + 1)²`, less than the closest packing would allow. A
ground's parameters are plain values with a default, not the rules a
map's may be. The ids of the areas a realm's maps ask for are given out
in the order its tracts first load; the cells are the same in every
order.

Gaps of regions and settlements (decisions D-670 to D-679): there is no realm
plan, so a realm's regions are given kinds by the `standard` generator's
`regions` and its settlements are placed by hand with its `settlements`;
a region kind's `settlements`, `weather` and `population` are carried.
A weight in a region kind is a number. `near: coast` and `near: river`,
the street patterns `radial`, `linear` and `scatter`, and a
settlement's `wall` fail to load. Crossings are not built: a path lies
on the ground whatever it meets. A path between regions runs in two
straight stretches from its crossing to the region's hub. A lot is
filled by a map of its zone, never by a site chosen by tag; a site is
placed only as a `must_have` or by a region kind. A site takes every
tract its map covers, and a placement's `site` is not set, because the
world gives a site its id after the survey. The street of a lot follows
the ground while the lot is levelled, so on rolling ground a door can
stand a storey off its street (the bundled town is flat). A lot is
dropped when it meets a placement the realm makes by hand, never moved.

Gaps of the map format: a state has a facing by its name alone, and a
definition cannot say that its numbered bits are a facing (section 3.3).
A link joins devices named in one map, not a device of a nested map to
one of its parent, and two nested copies of a map that name the same
device are not told apart. A mark that only some weighted alternative
gives cannot be named by a point. A parameter is a boolean, a number or
a text, never a list. An expression in a shared palette sees no
parameters. A map expression cannot read where its cell is.

**Settings**: `gen.max_nesting`, `gen.max_settling` (the longest settling
time a worldgen may ask for). Until settings are loaded, `gen.max_nesting`
is the limit handed to the function that builds the maps, with the
setting's default.

**Effects**: `apply_map`, `plan_from_map`, `transform_area`,
`create_realm` with a generator and parameters, `survey_region`.

**Queries**: `sites` and `placements` by region, tag and distance; `marks`
of a site.

**Properties**: `site.mark(name)`, `site.role(name)`, `site.def`,
`here.tract.def`, `here.region.kind`, `here.region.field(name)`,
`here.placement`.

**Generation context**: parameters as `$name`; `region.*` fields; `depth`
in assemblies; `rand`, `rand_int`, `chance`, `pick` seeded by placement
and cell (section 3.4). `region.*` is not offered in maps until regions
are surveyed; `depth` is a parameter the assembler gives.

## 14. Invariants

| # | Invariant |
|---|-----------|
| X1 | Realising a tract is a pure function of its record, its neighbours' records, the placements overlapping it and the definitions. |
| X2 | Surveying a region is a pure function of the realm's seed, the region's position and the realm plan. |
| X3 | Two neighbouring regions agree on everything that crosses their shared edge, whichever is surveyed first. |
| X4 | The same realm definition and seed give the same world, in every order of exploration and on every thread count. |
| X5 | Each cell's content is decided by exactly one rule of precedence: ground, then placements in order, then transforms. |
| X6 | Random choices in generation come only from positional seeds. |
| X7 | Every role a site definition names is supplied by its map, or the definition fails to load. Definitions flagged `founded` are exempt. |
| X8 | A map applied to the live world changes it only through `WorldEdit`. |
| X9 | Nothing is unsupported or overloaded (02 §14) when a tract is first realised from ground steps alone. Placed maps are checked by `map check`. |
| X10 | No two placements written by one survey overlap in three dimensions unless the later one's map is marked `over`. |

## 15. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Realise a typical settled tract | under 2 ms |
| Realise a tract of open ground | under 0.5 ms |
| Survey a rural region | under 10 ms |
| Survey a region holding a town | under 60 ms |
| Plan a realm of 512 × 256 regions | under 3 s |
| Build an assembly of 40 pieces | under 5 ms |
| Stored size of a surveyed town region | under 300 KB |

## 16. Open questions

1. **Drawing maps.** Closed by the map editor (14 §10.2).
2. **History.** Not planned beyond the barest level, which exists
   already: a place's parameters, drawn from the seed, can say how old it
   is and who built it, and transforms make it look its age or lie in
   ruins (section 10). Nothing simulates a past.
3. **Water that makes sense.** Settled for lakes: planned basins with
   their own levels (section 9.2). Erosion and catchments are not
   modelled.
4. **Roads on slopes.** Settled: path kinds with crossings (section 8.5).
5. **Places under places.** Settled: placements are tested for overlap in
   three dimensions (section 4.1, invariant X10).
6. **Very large single maps.** Closed by the map editor's sheet view
   (14 §10.2).
