# Plan — World Generation and the Map Format

Implements [10-world-generation.md](../10-world-generation.md).

Packages prefixed W are in [world-model.md](world-model.md); X, K and T in
[sim-kernel.md](sim-kernel.md); E, S and U in
[first-playable.md](first-playable.md); G in [devices.md](devices.md); V in
[vehicles.md](vehicles.md); I and L in [ai.md](ai.md). The rules for taking
a package are those of [world-model.md](world-model.md) and
[CLAUDE.md](../../CLAUDE.md).

Packages WG1 to WG5 and WG10 need only the world model and the data
pipeline. They can be built early, and they replace the test generators
(`flat`, `test_city`, `test_sea`) as the source of places for every other
plan's scenarios. The test generators stay, as fixtures.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| WG1 | Map format | L | W3, X4 | todo |
| WG2 | Placing and applying maps | L | WG1, W8, W17 | todo |
| WG3 | Ground | M | WG1 | todo |
| WG4 | Tract definitions and realisation | M | WG2, WG3 | todo |
| WG5 | Transforms and ruin | M | WG4, W22 | todo |
| WG6 | World generators and realm plans | L | WG4 | todo |
| WG7 | Region kinds and surveys | L | WG6 | todo |
| WG8 | Settlements | L | WG7 | todo |
| WG9 | Sites and the simulation seed | M | WG8, L1, L5 | todo |
| WG10 | Stacks and assemblies | L | WG2 | todo |
| WG11 | Vessels, designs and wiring from maps | M | WG2, V10, G8 | todo |
| WG12 | Reference worlds and map tools | L | WG9, WG10 | todo |
| WG13 | Benchmarks and budget gate | S | WG4, then recurring | todo |

## Order and parallelism

| Wave | Packages |
|------|----------|
| 1 | WG1 |
| 2 | WG2, WG3 |
| 3 | WG4, WG10, WG11 |
| 4 | WG5, WG6 |
| 5 | WG7 |
| 6 | WG8 |
| 7 | WG9 |
| 8 | WG12 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M5a Places** | WG1–WG5, WG10 | A house written as rows of characters appears in the world, turned to face its street, with a different wall material each time. A forty-storey tower from five maps. A crypt assembled from rooms, different for every seed and always walkable end to end. The same street, abandoned for a century. |
| **M5b Lands** | WG6–WG8 | An island with coast, uplands, woods and farmland; a town and villages joined by roads; a river to the sea. Regions explored in any order meet exactly at their edges. The same seed, the same island. |
| **M5c Inhabited** | WG9, WG11, WG12 | The town has a watch house with a sergeant who goes home at night, a market that is busy at noon, and a council that owns the hall. A submarine and a base design load from maps. Four small reference worlds in different genres run from the same engine. |

## Work packages

### WG1 — Map format

**Spec** 10 §3 · **Size** L

Build
- The `map` and `palette` types; levels, rows, presets; cell specs for
  every field of 10 §3.2; short forms and their ambiguity error; weighted
  alternatives and `once`.
- Parameters and the generation context: pure expressions, positional
  random functions.
- Turning and mirroring, including states with a facing.
- `place`, `set`, `areas`, `marks`, `spawns`, `wiring`, `joints`,
  `anchor`; compiled maps in `Defs`.
- Load-time validation of 10 §3.7.

Done when
- Each validation failure has a fixture and a located diagnostic.
- A map turned four times equals itself; mirrored twice equals itself
  (property test).
- The same placement seed gives the same resolved map regardless of the
  order cells are evaluated (invariant X6).
- A random function in a map draws from the placement, not the simulation
  stream, shown by an unchanged simulation hash.

### WG2 — Placing and applying maps

**Spec** 10 §4 · **Size** L

Build
- `Placement`; windows of a map per tract; `order` and overlap.
- Sitting on ground: `level`, `foundation`, `follow`, `none`.
- Nested maps resolved per placement.
- `apply_map` through `WorldEdit` with its modes; maps as pending edits.
- Outputs: spawns, areas, portals, subgrids, marks.

Done when
- A map spanning four tracts is identical whether its tracts are realised
  in any order (invariant X1).
- Each mode of `apply_map` is tested, and every change it makes is an
  ordinary edit with a cause (invariant X8).
- Scenario `aftermath_edit`: a pending map is present when its tract loads.
- A map on a slope is levelled, founded or followed as it asks.

### WG3 — Ground

**Spec** 10 §5 · **Size** M

Build
- The `ground` type and every step of 10 §5.1; surface height from a field
  with ramps; anchoring per stratum.

Done when
- A tunnel through anchoring rock stands and one through loose ground
  collapses by 02 §14.
- Over many seeds, no generated cave lies in a stratum that does not
  anchor, and no tract has anything unsupported when first realised.
- Sea fills to `sea_level` as uniform chunks.
- Scatter respects spacing and density over many seeds.

### WG4 — Tract definitions and realisation

**Spec** 10 §6 · **Size** M

Build
- The `tract` type; the realiser implementing the order of 10 §6.1 behind
  the `TractRealizer` interface; placements and paths in region records
  (02 §9.1).

Done when
- Realisation is pure and order-independent (invariant X1), by the same
  shuffled and multi-threaded test as world package W8.
- Precedence is ground, placements in order, transforms (invariant X5).
- The budget for a settled tract is benchmarked.

### WG5 — Transforms and ruin

**Spec** 10 §10 · **Size** M

Build
- The `map_transform` type and its steps; `settle` through resolve-mode
  support; ruin variants and the wasteland fallback; transforms on derived
  realms, replacing the stand-in of world package W22; `transform_area`.

Done when
- Scenario `ruined_twin`: a derived realm of a generated street, with a
  transform, has the same layout, broken windows, fallen floors where
  support was removed, and different inhabitants.
- A never-realised tract with total devastation realises as its ruin.
- A transform is pure per tract.

### WG6 — World generators and realm plans

**Spec** 10 §9 · **Size** L

Build
- The `worldgen` type; fields from noise and expressions; `kinds`.
- The realm plan: placing settlements and sites by rule, routes, rivers,
  holdings; stored with the realm.
- Authored layouts; realms created on demand from chart nodes.

Done when
- The same definition and seed give the same plan (invariant X4).
- Every planned settlement satisfies its placement rules, and every pair
  named in `routes` is joined.
- Rivers end at the sea or at the realm's edge.
- Scenario `world_on_demand`: arriving at a chart node creates its realm
  once, and arriving again finds the same one.
- An authored realm of one map realises that map.

### WG7 — Region kinds and surveys

**Spec** 10 §8.1–8.3 · **Size** L

Build
- The `region_kind` type; tract assignment in patches; site counts and
  constraints; path lines drawn per region; edge-keyed minor paths; the
  surveyor behind the `RegionSurveyor` interface.

Done when
- Scenario `edges_agree`: two neighbouring regions surveyed in either
  order agree on every path, street and site at their shared edge
  (invariant X3).
- Surveys are pure (invariant X2).
- Site counts and spacing hold over many seeds.

### WG8 — Settlements

**Spec** 10 §8.4 · **Size** L

Build
- The `settlement` type; the street patterns; blocks and lots; zones;
  choosing and turning maps to fit lots; `must_have`; walls and gates.

Done when
- Every lot's map fits and fronts a street.
- Every `must_have` site is present.
- Every building entrance is reachable from every other by walking
  (checked with world package W18's geometry).
- A settlement spanning two regions has continuous streets.

### WG9 — Sites and the simulation seed

**Spec** 10 §11 · **Size** M

Build
- The `site` type in full; roles from marks; staff, homes and presence;
  groups from settlements and holders; stocks and supplies; the seed
  carried by `region_surveyed` and its use by the simulation; settling
  time.

Done when
- A site definition naming a role its map lacks fails to load
  (invariant X7).
- Scenario `town_day`: a generated town's notable staff are at work by
  day and at home by night; its streets' anonymous population follows the
  presence table.
- Groups own the sites the definitions say they own.
- After a settling time, routines are out of step and at least one
  incident is known about somewhere.

### WG10 — Stacks and assemblies

**Spec** 10 §7 · **Size** L

Build
- The `building` type: stacks, repeats, variation, the core; the partition
  step.
- The `assembly` type: joints, the assembler, loops, limits, caps, bounds,
  depth; results stored with the site.

Done when
- Scenario `tower_of_forty`: a stacked tower's floors are joined by its
  core from basement to roof, and its ground-storey supports removed, it
  collapses by 02 §14.
- Scenario `crypt_walkable`: for many seeds, every piece of an assembly is
  reachable from its start, nothing overlaps, and nothing leaves the
  bounds.
- An assembly is a pure function of its seed.

### WG11 — Vessels, designs and wiring from maps

**Spec** 10 §3.6, §4.3; 11 §12.1; 16 §10.3 · **Size** M

Build
- Vessel maps into subgrids for vehicle package V10.
- `plan_from_map` proper for device package G8.
- Conduit layers and named device links from maps.

Done when
- A vessel defined by a map spawns sealed, with its compartments found.
- A room design creates one job per differing cell.
- A mapped machine room has working networks on first load.

### WG12 — Reference worlds and map tools

**Spec** 10 §12; 00 §6 · **Size** L

Build
- Four small reference realms in separate mods: a few modern blocks, a
  village with a crypt beneath it, an island with sea around it, a station
  in a void. Each with sites, people and at least one incident.
- The tools of 10 §12.

Done when
- Each reference world starts, runs a day, and passes scripted checks of
  its routines and one incident.
- No reference world requires another, or anything in the engine that
  names its genre.
- `map check` reports a fixture map with an unsupported floor and one with
  an overloaded pillar, each with its cell, and passes every bundled map.

### WG13 — Benchmarks and budget gate

**Spec** 10 §15 · **Size** S, recurring

## Scenario catalogue

Under `scenarios/worldgen/`.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `aftermath_edit` | WG2 | S7 | Maps as pending edits |
| `ruined_twin` | WG5 | S8 | Transforms on a derived realm |
| `world_on_demand` | WG6 | S9 | Realms created on arrival |
| `edges_agree` | WG7 | — | Agreement across region borders |
| `town_day` | WG9 | S7 | People, homes, presence, routines |
| `tower_of_forty` | WG10 | S2 | Stacks; structure that can fall |
| `crypt_walkable` | WG10 | S21 | Assemblies |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Berths and watched things in maps | WG1 | 10 §3.2 | Cell specs `berth`, `approach` and `watched` load, turn with the map, and are recorded on the site. |
| Lakes | WG3, WG6 | 10 §5.1, §9.2 | A planned lake is filled to its own level by the `liquid` step with `to: plan`, in every tract it covers, whichever is realised first. A river reaching it ends there and one leaves from the lowest point of its rim. |
| Path kinds and crossings | WG7 | 10 §8.5 | Test script `road_over_water`: a road meeting a river is carried on bridge maps from bank to bank; meeting a steep rise it is cut or tunnelled as its crossings say; two neighbouring regions surveyed in either order agree on every crossing. |
| Placement overlap | WG7 | 10 §4.1 | A survey never writes two placements whose boxes overlap in three dimensions unless the later map is marked `over` (invariant X10). A sewer piece that would pass through a cellar is moved or dropped, and the survey's report says which. |
