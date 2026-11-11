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
| `size` | Width and depth in cells. Every row has `size[0]` characters; every level has `size[1]` rows. Rows run north to south. |
| `levels` | Storeys by number, relative to the map's own ground level. A level is a list of rows, or a named preset such as `roof` (a floor over everything that had a block below). |
| `palette` | What each character means (section 3.2). Named palettes and inline entries, later overriding earlier. |
| `default` | The cell spec laid under every character that is not a space |
| `params` | Values chosen once per placement (section 3.4) |
| `front` | Which side faces a street, for placement |
| `place` | Other maps placed inside this one (section 3.5) |
| `set` | Cell specs applied at listed coordinates or boxes, for things awkward to draw |
| `areas`, `marks` | Areas (02 §8.3) and named points or cells |
| `spawns` | Creatures and populations (06 §12) |
| `wiring` | Conduit layers and device links (section 3.6) |
| `joints` | Where it can connect to other maps (section 7.2) |
| `anchor` | What level 0 sits on: `ground`, `sea_level`, `sea_floor`, or an absolute storey |

A space character means "leave what is there". Everything else replaces.

### 3.2 Palettes and cell specs

A palette maps a character to a **cell spec**.

| Spec field | Sets |
|------------|------|
| `floor`, `block`, `fixture` | Element states (02 §5.1) |
| `clear` | Removes listed layers |
| `conduits` | Conduits laid in the cell (16 §6) |
| `field` | Field amounts (02 §16) |
| `items` | Items, or a content table. In a container fixture they are latent contents (06 §9.3). |
| `spawn` | A creature or population member in this cell |
| `device` | Configuration for the element's device (16 §4.2), and a name for linking |
| `mark` | Adds the cell to a named mark |
| `berth` | Adds the cell to a named berth of a kind: a parking bay, a mooring, a landing pad. Cells given `approach` with the same name are the line an AI driver follows into it (09 §5.6). |
| `watched` | Marks what is placed in the cell as something the site's people keep an eye on (15 §6) |
| `area` | Adds the cell to an area of a kind; touching cells with the same entry are one area |
| `var`, `decal` | Cell variables and marks (02 §5.1) |

A bare string is short for the layer its definition belongs to:
`"wall@brick"` is `{ "block": "wall@brick" }`. If an id exists in more than
one of floor, block and fixture, the short form is a load error asking for
the long one.

A palette value may be a list of weighted alternatives, chosen per cell,
or once for the whole placement with `"once": true`.

```json
"t": [ { "fixture": "table", "weight": 3 }, { "fixture": "desk" }, {} ]
```

Shared palettes are definitions:

```json
{ "type": "palette", "id": "house_common",
  "entries": { ".": {}, "c": { "fixture": "chair" }, "t": { "fixture": "table" } } }
```

### 3.3 Turning

A placement may turn a map by quarter-turns and mirror it. Element states
with a facing are turned with it. `"turn": false` on a map forbids it.

### 3.4 Parameters and chance

Parameters are chosen once for each placement: given by whoever placed the
map, or picked by the map's own rule. They may be used anywhere in the
map: `"wall@$wall"`, `"count": "$beds"`, `"if": "$wealth > 2"`.

Expressions in maps are evaluated in a **generation context**. They are
pure. `rand`, `chance` and `pick` there draw from a generator seeded by the
placement's seed and the cell, never from the simulation's stream, so the
result depends on where and what, not on when.

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

### 3.7 Validation

At load: row lengths, unknown characters, unresolved parameters, nested
maps that cannot fit, links to unnamed devices, joints off the boundary.
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
    pub origin: CellPos,          // where the map's north-west corner at level 0 goes
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

**Overlap.** Every placement has a box: its footprint and its storeys,
after turning. When a survey writes a placement it tests the box against
those already written. Two placements may overlap only if the later one's
map says `"over": true`, as a market stall on a square does. Otherwise the
later is moved as its rule allows, or dropped and noted in the survey's
report. The test is in three dimensions, so a sewer may run under a street
and may not run through a cellar.

### 4.2 Sitting on the ground

Ground is rarely flat and maps usually are.

| Map setting | Where the map's ground level meets the terrain |
|-------------|-------------------------------------------------|
| `level` (default) | The terrain under the footprint is flattened to the height at the origin, with ramps at the edges |
| `foundation` | Terrain is left. Gaps under the map are filled with a named state; terrain above its floor is cut away. |
| `follow` | Each column of the map is shifted to its own ground height. For fences, roads, low walls. |
| `none` | Nothing is done |

### 4.3 Applying a map to the world as it is

The same maps are used after generation.

| Verb | Does |
|------|------|
| `apply_map { map, at, turn, params, mode }` | Writes a map into loaded cells through `WorldEdit` |
| A pending edit naming a map (02 §17.3) | The same, applied when the tract next loads |
| `plan_from_map` (16 §10.3) | Creates construction jobs for every cell where the map differs from the world |

`mode` is `replace`, `only_empty`, or `add` (place elements only where the
layer is empty, and never clear).

An incident's aftermath, a conjured fortress, a scenario's set dressing and
a building design are all maps applied this way. `at` may be a mark of a
site: `site.mark('counter')`.

## 5. Ground

### 5.1 Ground definitions

```json
{ "type": "ground", "id": "temperate_lowland",
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

### 5.2 What holds

Each stratum says whether its blocks are `anchor` (02 §5.4). Rock that
anchors can be tunnelled freely. Sand, gravel and loose soil given
`"anchor": false` need support by span like any structure (02 §14), and a
tunnel through them comes down. This settles open question 2 of 02 §26: it
is a property of the ground, chosen per stratum.

Generation must leave nothing unsupported. The `caves` step carves only
where the stratum anchors, so a generated cave never hangs in loose ground
waiting for the first nearby edit to bring it down. A void in loose ground
is made by digging, in play, where the support rule applies from the first
cell removed.

## 6. Tracts

```json
{ "type": "tract", "id": "meadow", "tags": ["open", "rural"],
  "ground": "temperate_lowland",
  "params": { "moisture": "region.moisture" },
  "ruin": { "transform": "scorched" },
  "bands": ["overland", "air"],
  "glyph": { "ch": "\"", "fg": "green" } }
```

A **tract definition** says what the ground of a tract is, which travel
bands may cross it (11 §13.2), what it becomes when ruined (section 10.2;
`ruin` may instead be keyed by kind of devastation, 02 §17.2),
and how it is drawn on the region map (12 §6.2). What stands on it is in
placements.

### 6.1 Realising a tract

In order, as a pure function of the tract's record, its neighbours'
records and the placements overlapping it:

1. Ground steps.
2. Levelling and foundations for placements.
3. Placements in `order`, each clipped to the tract, with their nested maps.
4. Transforms that apply to the tract or its realm (section 10).
5. The result: chunks, fills, spawns, areas, portals, subgrids (02 §9.3).

What follows (stamps, pending edits, catching up) is 02 §9.4.

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

### 8.3 Agreeing at the edges

Two neighbouring regions are surveyed separately and must match.

- **Planned paths** (great roads, rivers, borders, rails) are lines in the
  realm plan in realm coordinates. Each region draws the part inside it.
- **Minor paths** cross an edge at points derived from a hash of that edge,
  which both regions compute alike without consulting each other.
- **Street grids** of a settlement spanning regions are keyed to the
  settlement's own origin and angle, so they line up.
- **Height and other fields** are functions of position.

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

## 13. Modding surface

**Definition types**: `map`, `palette`, `ground`, `tract`, `building`,
`assembly`, `region_kind`, `settlement`, `site`, `worldgen`,
`map_transform`, `path_kind`, `names`. Field `presence` on populations.

**Events**: `realm_planned`, `region_surveyed`, `tract_realized`,
`site_created`, `map_applied`.

**Effects**: `apply_map`, `plan_from_map`, `transform_area`,
`create_realm` with a generator and parameters, `survey_region`.

**Queries**: `sites` and `placements` by region, tag and distance; `marks`
of a site.

**Properties**: `site.mark(name)`, `site.role(name)`, `site.def`,
`here.tract.def`, `here.region.kind`, `here.region.field(name)`,
`here.placement`.

**Generation context**: parameters as `$name`; `region.*` fields; `depth`
in assemblies; `rand`, `chance`, `pick` seeded by placement and cell.

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
