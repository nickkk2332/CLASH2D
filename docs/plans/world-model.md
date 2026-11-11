# Plan — Foundation and World Model

Implements [01-architecture.md](../01-architecture.md) sections 3 to 7 (the
parts the world needs) and all of [02-world-model.md](../02-world-model.md).

Section references such as "02 §14" point into those documents.

## How to use this plan

- One work package (WP) is one change set by one agent. Take a package whose
  dependencies are all `done`, set it to `in progress` in the status table in
  the same commit that starts the work, and set it to `done` when it merges.
- The specification is the source of truth. If a package cannot be built as
  specified, change the specification in the same change set, add an entry to
  [decisions.md](../decisions.md), and say so in the description.
- "Done when" lists are minimums. Every package also meets the definition of
  done in [CLAUDE.md](../../CLAUDE.md).
- Do not build ahead. If a later package's concern leaks into yours, leave a
  typed stub and a note, not an implementation.
- Sizes: **S** a few hundred lines, **M** about 1,500, **L** about 3,000
  including tests. Split an L if it grows.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| F1 | Workspace and gates | S | — | todo |
| F2 | `clash-core` | M | F1 | todo |
| F3 | `clash-time` | M | F2 | todo |
| F4 | `clash-data` v0 | L | F2 | todo |
| W1 | Dimensions and coordinates | S | F2 | todo |
| W2 | Paletted storage | S | F2 | todo |
| W3 | Cell definitions | M | F4, W1 | todo |
| W4 | Chunks and grids | M | W2, W3 | todo |
| W5 | Cosmos, realms, views | M | W4, F3 | todo |
| W6 | Edits, causes, events | M | W5 | todo |
| W7 | Occupancy, planes, areas, sites | M | W6 | todo |
| W8 | Tracts, regions, generation, loading | L | W7 | todo |
| W9 | Tickets and activity | M | W8 | todo |
| W10 | Persistence | M | W8 | todo |
| W11 | World viewer and scenario runner | M | W8 | todo |
| W12 | Damage and destruction | M | W6 | todo |
| W13 | Structural support and collapse | L | W12, W9 | todo |
| W14 | Bodies | L | W12, W7, W9 | todo |
| W15 | Energy releases | L | W13, W14 | todo |
| W16 | Fields | L | W12 | todo |
| W17 | Coarse layer | L | W15, W10 | todo |
| W18 | Queries | M | W7 | todo |
| W19 | Portals, boundary links, realm transfer | M | W14 | todo |
| W20a | Subgrids: storage, compositing, movement | L | W18, W14 | todo |
| W20b | Subgrids: detach, merge, free flight | M | W20a | todo |
| W21 | Travelers, carriers, charts | M | W19, W20b, W17 | todo |
| W22 | Derived realms and realm lifecycle | S | W19, W10 | todo |
| W23 | Benchmarks and budget gate | S | W4, then recurring | todo |

## Order and parallelism

Packages in the same wave have no dependencies on each other and can be
built at the same time.

| Wave | Packages |
|------|----------|
| 1 | F1 |
| 2 | F2 |
| 3 | F3, F4, W1, W2 |
| 4 | W3 |
| 5 | W4 |
| 6 | W5 |
| 7 | W6 |
| 8 | W7, W12 |
| 9 | W8, W16, W18 |
| 10 | W9, W10, W11 |
| 11 | W13, W14, W23 |
| 12 | W15, W19, W20a |
| 13 | W17, W20b, W22 |
| 14 | W21 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M0 Foundation** | F1–F4 | `cargo xtask check` passes; a mod with a mistake produces a precise error. |
| **M1a A world you can look at** | W1–W11 | Generate the test city, browse it storey by storey in the terminal viewer, save, load, same hash. |
| **M1b A world you can wreck** | W12–W18 | From the viewer console: punch out a wall, drop a tower, set off a car bomb, throw a body through three buildings, start a fire. |
| **M1c A world that connects** | W19–W22 | Walk through a portal to another realm, ride a moving subgrid, tear off and throw a building top, scar a continent and visit the damage, fly from a roof to another world. |

## Foundation

### F1 — Workspace and gates

**Spec** 01 §3, §4, §12 · **Size** S

Build
- Cargo workspace with `clash-core`, `clash-time`, `clash-data`, `clash-world`,
  `clash-gen`, `clash-tools` and an `xtask` crate. Other crates are added by
  the plans that need them.
- Pinned stable toolchain, edition 2024.
- Workspace lints: `unsafe_code` denied; clippy `disallowed_types` and
  `disallowed_methods` covering the determinism contract (std hash maps and
  sets outside `LookupMap`, std float transcendentals, `std::time`,
  thread-local randomness); warnings denied in CI.
- `cargo xtask check`: format check, clippy, tests, doc build, and a check
  that the literals for chunk, tract and region sizes appear only in
  `clash_world::dims`.
- CI workflow running `cargo xtask check`.
- `mods/core/mod.json`.

Done when
- A fresh clone passes `cargo xtask check`.
- The change description shows clippy rejecting a hash-map iteration and an
  `f64::sin` call placed in `clash-world`.

### F2 — `clash-core`

**Spec** 01 §4 · **Size** M

Build
- `Real` with checked construction, exact operators and `libm`-backed
  functions.
- `IVec2`, `IVec3`, `Dir8`, vertical directions, `Rot4`.
- `define_id!` for typed dense ids; generational `EntityId`.
- `Tick`, `Dur`.
- `LookupMap` (no iteration API), sorted-vec helpers.
- Positional hash `pos_hash(seed, domain, x, y, z, salt)`; a small PRNG
  implemented in-crate with a fixed algorithm; named `RngStreams`.
- `StateHash` trait and a fixed 128-bit hasher.
- `Value` (bool, int, real, string) and `VarMap`. Document 05 extends these.
- Error conventions with `thiserror`.

Done when
- Committed reference vectors for `Real` functions, the PRNG, `pos_hash` and
  the hasher pass on CI. These vectors are the cross-platform contract.
- `LookupMap` cannot be iterated (shown by a compile-fail test).
- Property tests cover vector and rotation algebra.

### F3 — `clash-time`

**Spec** 01 §5, 03 §3 · **Size** M

Build
- `Timeline` with the API of 03 §3.1: schedule, cancel, remaining, rehome,
  peek, pop in `(tick, sequence)` order, stable `WakeId`s.
- Clock tree with rates; local to universal conversion; `set_rate`,
  `reparent`, `destroy_clock`; parking at rate 0; rescheduling of affected
  wake-ups.
- Opaque `Wake { domain, id, arg }` payloads.
- `ProcessTable<P>` generic over the owning crate's process enum; `wake`
  returns sleep-for or done; a force-resolve entry point.
- Serialisation and `StateHash`.

Done when
- Same-tick events pop in scheduling order.
- A 10 s local wait whose clock drops to rate 0.5 after 4 s fires at
  universal 16 s. At rate 0 it never fires, and resumes with 6 s remaining.
- Nested clock rates multiply; reparenting preserves remaining local time.
- A property test drives random schedule, cancel, rate and reparent
  operations against a naive reference model.
- Saving and loading mid-run preserves pop order.

### F4 — `clash-data` v0

**Spec** 01 §7, 04 §19 · **Size** L

Build
- Mod discovery, `mod.json`, dependency ordering.
- JSON parsing that keeps line and column for every value.
- `Def` trait and `Registry<T>`; dense ids assigned in sorted order of
  qualified id; namespace resolution; `copy_from`, `extend`, `delete`,
  `override`.
- Quantity parsing with the unit list.
- Unknown-key detection with suggestions; all errors collected, not just the
  first.
- JSON Schema generation; `clash-tools schema`.
- `clash-tools lint <dir>`.

Not in this package: everything in 04 outside the subset listed in 04 §19.
The rest is built by packages P1 to P8 of [sim-kernel.md](sim-kernel.md).

Done when
- A fixture mod set loads; each error class (unknown key, bad unit, missing
  reference, inheritance cycle, duplicate id, type mismatch) produces the
  expected message with file, line, column and path.
- Ids are identical across runs and independent of directory enumeration
  order.

## World

### W1 — Dimensions and coordinates

**Spec** 02 §3, §4.2 · **Size** S

Build
- `dims`; position types and conversions; `CellIdx`; `GridId`, `Location`,
  `PointM`.
- `Topology`: normalise, delta, edges.
- Iteration helpers with defined order: boxes, shells by distance, the eight
  and ten neighbours, and 3D traversal of a segment through 1 × 1 × 3 m cells.

Done when
- Property tests cover conversions for negative coordinates, wrap, and delta
  symmetry.
- Segment traversal visits exactly the cells a brute-force reference finds.

### W2 — Paletted storage

**Spec** 02 §6.1 · **Size** S

Build
- `Paletted<T>`, `SparseCells<T>`, fixed 8192-bit bitsets; growth, compaction,
  uniform detection; serialisation; `StateHash` independent of palette order.

Done when
- Property test against a plain `Vec<T>` model.
- A uniform layer is under 200 bytes; a 16-entry palette uses 4 bits per cell.
- Two layers with equal contents but different palette histories hash equal.

### W3 — Cell definitions

**Spec** 02 §5 · **Size** M

Build
- Definition types `damage_type`, `material`, `atmosphere`, `floor`, `block`,
  `fixture`, including states, geometry, resist overrides, flags and tags,
  and the `interactions` and `device` fields of 02 §5.5 (stored and
  reported; the world gives them no behaviour).
- `ElementState` and the typed wrappers; the state reference parser.
- Derived per-state tables: flags, volume, mass, capacity, resolved resist
  per damage type.
- `mods/core` data sufficient for tests: the eight damage types; granite,
  soil, concrete, brick, glass, wood, steel; solid, wall, window, door,
  pillar, ramp, rubble heap; slab, asphalt, grass; rubble, desk; air and thin
  air.

Done when
- `clash-tools lint mods` passes.
- Derived values for a concrete wall match 02 §12.2 (0.6 m³, 0.6 MJ).
- Schemas are generated for every type.

### W4 — Chunks and grids

**Spec** 02 §6.2, §6.3 · **Size** M

Build
- `Chunk` with all layers except fields and occupants (typed placeholders),
  `DerivedCache`, revisions.
- `Column` with fills and heightmap; `Grid`.
- Crate-private read and write primitives.

Done when
- Reads outside stored z return the fill without allocating; writes there
  create storage.
- Heightmap and derived bitsets equal a from-scratch recomputation after
  random edits (property test).
- Benchmarks exist for flag and state reads.

### W5 — Cosmos, realms, views

**Spec** 02 §4, §7.1 · **Size** M

Build
- `realm` definition type; `Realm`, `Cosmos`, `create_realm`; a clock per
  realm.
- Strata lookup and `ambient`.
- The read API of 02 §7.1 including `is_loaded` and the unloaded error.

Done when
- Edge normalisation, strata boundaries and realm isolation are tested.
- Reading an unloaded cell returns the error value.

### W6 — Edits, causes, events

**Spec** 02 §7.2–7.4 · **Size** M

Build
- `WorldCtx`, `WorldEdit` (element setters, decals, cell variables), `Cause`,
  `WorldEvent`, `EventSink`.
- Bookkeeping on every structural change: storage, heightmap, revision,
  dirty flag, stability-check request (queue only; W13 consumes it).

Done when
- Each effective edit emits exactly one event with old state, new state and
  cause; an edit that changes nothing emits none and bumps nothing.
- Chunk mutation is not reachable from outside the crate except through
  `WorldEdit`.

### W7 — Occupancy, planes, areas, sites

**Spec** 02 §8 · **Size** M

Build
- `OccupantIndex`; `place`, `relocate`, `remove`; box footprints; the
  occupant kinds of 02 §8.1 including `Device`.
- Queries at cell, box, radius and ray, with plane and kind filters and
  defined result order.
- Area table with per-tract lookup, enter and exit events, optional clocks;
  the `area_kind` definition type (its `rules` field is carried as data
  until the rule runtime exists).
- Site table.

Done when
- Queries match a brute-force list on random populations (property test).
- Enter and exit events are correct for diagonal moves and multi-cell
  footprints.
- Occupants on disjoint planes are invisible to each other's queries.

### W8 — Tracts, regions, generation, loading

**Spec** 02 §9 · **Size** L

Build
- `RegionRecord`, `TractRecord`, `TractExtra`; lifecycle states.
- `RegionSurveyor`, `TractRealizer`, `GenCtx` in `clash-gen`.
- Test generators `flat` and `test_city` (parameters: street spacing, storey
  range, column spacing; a ruin variant for W17).
- Load and unload procedures of 02 §9.4 and §9.5 against an in-memory store.
- Parallel realisation.

Done when
- Realising the same tracts in shuffled order and with different thread
  counts gives identical hashes.
- A realiser cannot read neighbouring cells (enforced by the type of its
  inputs).
- Load, unload, load without changes yields identical cells.
- Unloading a tract that still has an occupant fails the assertion.

### W9 — Tickets and activity

**Spec** 02 §10 · **Size** M

Build
- `TicketTable`, anchors, levels, warm margin, hysteresis.
- Reconciliation at safe points in the specified order.
- Budget, refusal order, and notification of refused holders.

Done when
- A moving anchor loads and unloads exactly the expected tracts.
- Oscillating across a tract boundary causes no load or unload.
- Over budget, the lowest priority is refused first and pinned tickets never
  are.

### W10 — Persistence

**Spec** 02 §11 · **Size** M

Build
- `TractStore`; in-memory and `redb` implementations.
- Versioned tract blob codec with LZ4; save-id table; remapping on load;
  placeholders that keep the original string id.
- Pristine policy and generator fingerprint.
- Serialisation of realm and cosmos world state.

Done when
- Random columns round-trip (property test).
- A whole cosmos hashes equal after save and load.
- Removing a mod yields placeholders; restoring it yields the original
  content.
- A pristine tract writes no blob.
- A corrupted blob produces an error, not a panic.

### W11 — World viewer and scenario runner

**Spec** 01 §12 · **Size** M

Build
- `clash-tools worldview`: terminal viewer of one storey of loaded cells
  using definition glyphs; pan; storey up and down; a cell inspector (states,
  damage, fields, occupants, decals and their causes); a region map showing
  tract kind and devastation.
- A command console whose commands map one-to-one onto world API calls:
  `gen`, `ticket`, `set`, `damage`, `release`, `launch`, `advance`, `save`,
  `load`, `hash`. Later packages add their commands.
- Scenario files (console commands plus an expected hash) under
  `scenarios/world/`, and `clash-tools scenario run`, which runs each
  scenario straight, twice, with save and load at three split points, and
  with two thread counts. `--bless` rewrites the expected hash.

Done when
- The test city can be generated and browsed.
- `smoke_city` runs in CI in all four modes.

### W12 — Damage and destruction

**Spec** 02 §12 · **Size** M

Build
- `DamagePacket`, `DamageOutcome`, the rule of 02 §12.2, the damage layer.
- Destruction: `on_destroy`, rubble accumulation and heaps, events.
- A debris hook that W14 fills in.

Done when
- Unit tests reproduce both worked examples in 02 §12.2 to the stated
  figures.
- Ten thousand below-threshold packets change nothing.
- Repeated above-threshold packets destroy at the predicted count.
- Scenario `wall_punch`.

### W13 — Structural support and collapse

**Spec** 02 §14 · **Size** L

Build
- The stability queue consumer with merging and bounded island discovery.
- Storey-by-storey evaluation.
- Falling masses on the timeline, landing impacts, cascades, rubble,
  `OccupantUnsupported`.
- The collapse process, its ticket, and resolve mode.

Done when
- A slab of span 4 across walls 6 cells apart stands; 10 apart, the middle
  falls.
- Removing one interior column of a tower causes local failure only;
  removing all ground-storey supports brings the whole tower down.
- An occupant relocated out from under a falling mass before it lands is not
  exposed.
- A falling mass inside a rate-0 area hangs and resumes.
- Scenario `tower_pancake`, including a save in mid-collapse.

### W14 — Bodies

**Spec** 02 §15 · **Size** L

Build
- `BodyTable`; `launch`, `impulse`, `arrest`.
- The stepping process with gravity and drag from strata.
- Impacts with elements, floors and occupants; sliding; landing; events.
- The leading ticket; re-parenting to area clocks.
- Debris bodies from W12's hook, with a budget.

Done when
- A launch in vacuum matches the analytic parabola within tolerance; in air,
  terminal velocity matches the analytic value within tolerance.
- The 600 m/s example crosses walls with energy accounting that satisfies
  invariant W8; the 20 m/s example stops at the first wall.
- A body inside a rate-0 area hangs and resumes.
- Scenarios `three_buildings` and `time_stop_street`.

### W15 — Energy releases

**Spec** 02 §13.1, §13.2, §13.4 · **Size** L

Build
- `release` for sphere, cone and beam.
- The shell process, transmission map, three tiers, exposure events.
- Budget fallback and resolve mode for loaded tracts; force-resolve.

Done when
- Contract points 1 to 4 of 02 §13.2 are each tested.
- In an open test layout, wall destruction radii match the table in
  02 §13.1 to within one cell.
- An occupant relocated beyond the front before it arrives is not exposed.
- Scenarios `car_bomb_street` and `beam_through_city`, including a save
  mid-release.

### W16 — Fields

**Spec** 02 §16 · **Size** L

Build
- `field` definition type; `FieldStore`; active sets.
- The stepping process with buffered writes and positional randomness.
- Rules `decay`, `diffuse`, `flow`, `spread`, `emit`, `convert`, `damage`.
- Blocking by coverage and floors; contact events; catch-up; budget.
- Core data: fire, smoke, heat, water, steam.

Not in this package: waking still liquid blocks into fields. Track it as a
follow-up once W16 is stable.

Done when
- Diffusion conserves the total apart from decay.
- Liquid falls and levels.
- Fire spreads only where there is fuel and burns out.
- Shuffling the visiting order of cells and chunks leaves the result
  unchanged.
- Scenario `office_fire`.

### W17 — Coarse layer

**Spec** 02 §13.3, §17, §10.3 · **Size** L

Build
- Stamp storage at tract, region and realm level; watermarks; lazy pull-down
  at load.
- Devastation: calculation, cache, `devastation_at`, the ruin shortcut.
- `PendingEdit`: `Stamp` and `Cells` applied by the world; `Map`, `Spawn`
  and `Eoc` stored and surfaced through events for later layers.
- `release` dispatch between detailed and coarse.
- Coarse resolution of bodies that lose their ticket.
- `TractDevastated` and `RegionDevastated`.

Done when
- A release straddling loaded and unloaded tracts handles each side in the
  right mode, once.
- Loading a stamped tract shows damage consistent with its distance; loading
  it again applies nothing further.
- A planet-scale release creates one realm stamp, and its cost does not
  depend on realm size.
- Scenarios `offscreen_fight` and `continent_scar`, including a save with
  stamps pending.

### W18 — Queries

**Spec** 02 §22 · **Size** M

Build
- `raycast` with element heights; `line_of_sight`; `field_of_view`;
  `neighbours`.
- Caches keyed on chunk revisions.

Done when
- Field of view is symmetric on random maps (property test).
- A ray at 1.5 m passes a 1 m fence and stops at a wall.
- Cached results are invalidated by an edit and by nothing else.

### W19 — Portals, boundary links, realm transfer

**Spec** 02 §19 · **Size** M

Build
- Portal table; traversal for occupants and bodies; filters; events.
- Edge behaviour: barrier, void, and realm and entry exits. The chart exit
  emits an event until W21.
- The realm transfer function.

Done when
- An occupant walks between realms through a portal pair and its clock
  changes with the realm.
- A body keeps its speed through a portal, with direction rotated by facing.
- A void edge removes with an event; a barrier edge stops a body with an
  impact.

### W20a — Subgrids: storage, compositing, movement

**Spec** 02 §18.1, §18.2 · **Size** L

Build
- `Subgrid`, transforms, per-chunk overlap lists.
- Compositing in views, occupancy and every query of W18.
- Occupants riding; nesting to depth four.
- Stepped movement with contact resolution.

Done when
- Every query gives the same answer on a world with a stationary subgrid as
  on the equivalent world with those cells merged in (property test).
- An occupant on a moving subgrid keeps its local location.
- A subgrid driven into a wall breaks it or stops, according to the damage
  rule, with energy accounted for.

### W20b — Subgrids: detach, merge, free flight

**Spec** 02 §18.3 · **Size** M

Build
- `detach` and `merge`; mass maintenance; free flight through a body.

Done when
- Detach followed by merge in place restores the original hash.
- Scenario `thrown_building`.

### W21 — Travelers, carriers, charts

**Spec** 02 §20 · **Size** M

Build
- `chart` definition type; traveler table and its coarse stepping process.
  A traveler's band is an opaque id here; bands, routing and journeys are
  built by package V11 of [vehicles.md](vehicles.md).
- Arrival with a ticket; carrier realm creation and subgrid transfer; the
  chart exit of realm edges.

Done when
- A traveler crossing 500 tracts realises none of them.
- A carrier interior stays loaded and its timeline keeps running in transit.
- Scenario `rooftop_to_orbit`.

### W22 — Derived realms and realm lifecycle

**Spec** 02 §21 · **Size** S

Build
- The derived generator in `pristine` and `live` modes with a palette-swap
  transform as a stand-in. Package WG5 of [worldgen.md](worldgen.md)
  replaces the stand-in with real transforms.
- `destroy_realm` with its policies; storage deletion at save.

Done when
- A pristine derived realm ignores later edits to its source; a live one
  reflects them at first realisation only.
- Destroying a realm transfers or removes its contents according to policy
  and leaves no storage behind.

### W23 — Benchmarks and budget gate

**Spec** 02 §25 · **Size** S, recurring

Build
- A benchmark for each row of 02 §25 as the relevant package lands.
- `cargo xtask bench --check` against a thresholds file.
- Replace the targets in 02 §25 with measured figures and agreed limits.

Done when
- Every row of the table has a benchmark and a recorded figure.

## Scenario catalogue

Each scenario is a console script with a golden hash, run in all four modes.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `smoke_city` | W11 | — | Generate, browse, save, load |
| `wall_punch` | W12 | S1 | Threshold, capacity, rubble |
| `tower_pancake` | W13 | S2 | Support loss, cascade, mid-collapse save |
| `three_buildings` | W14 | S2 | Flight, wall-by-wall energy loss, landing |
| `car_bomb_street` | W15 | S3, S5 | Shells over time, shadowing, exposure |
| `beam_through_city` | W15 | S8 | Beam march and terminal release |
| `office_fire` | W16 | — | Spread, fuel, smoke, burn-out |
| `offscreen_fight` | W17 | S7 | Pending edits and stamps applied at load |
| `continent_scar` | W17 | S8 | Realm stamp, devastation, ruin generation |
| `time_stop_street` | W14 | S10 | Bodies freeze in a rate-0 area and resume |
| `thrown_building` | W20b | S11 | Detach, flight, merge |
| `rooftop_to_orbit` | W21 | S9 | Strata, edge exit, carrier, arrival |
