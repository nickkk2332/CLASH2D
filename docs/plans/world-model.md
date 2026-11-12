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
| F1 | Workspace and gates | S | — | done |
| F2 | `clash-core` | M | F1 | done |
| F3 | `clash-time` | M | F2 | done |
| F4 | `clash-data` v0 | L | F2 | beta |
| W1 | Dimensions and coordinates | S | F2 | done |
| W2 | Paletted storage | S | F2 | done |
| W3 | Cell definitions | M | F4, W1 | beta |
| W4 | Chunks and grids | M | W2, W3 | beta |
| W5 | Cosmos, realms, views | M | W4, F3 | beta |
| W6 | Edits, causes, events | M | W5 | beta |
| W7 | Occupancy, planes, areas, sites | M | W6 | beta |
| W8 | Tracts, regions, generation, loading | L | W7 | beta |
| W9 | Tickets and activity | M | W8 | beta |
| W10 | Persistence | M | W8 | beta |
| W11 | World viewer and scenario runner | M | W8 | beta |
| W12 | Damage and destruction | M | W6 | beta |
| W13 | Structural support and collapse | L | W12, W9 | beta |
| W14 | Bodies | L | W12, W7, W9 | beta |
| W15 | Energy releases | L | W13, W14 | beta |
| W16 | Fields | L | W12 | beta |
| W17 | Coarse layer | L | W15, W10 | todo |
| W18 | Queries | M | W7 | beta |
| W19 | Portals, boundary links, realm transfer | M | W14, W18 | todo |
| W20a | Subgrids: storage, compositing, movement | L | W18, W14 | todo |
| W20b | Subgrids: detach, merge, free flight | M | W20a | todo |
| W21 | Travelers, carriers, charts | M | W19, W20b, W17 | todo |
| W22 | Derived realms and realm lifecycle | S | W19, W10 | todo |
| W23 | Benchmarks and budget gate | S | W4, then recurring | todo |
| W24 | Fronts, tremors and kinds of devastation | M | W15, W17 | todo |
| W25 | Change of state | M | W16, W13, V1 | todo |

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
| 14 | W21, W24 |
| 15 | W25 |

W24 and W25 were added after a review of what the design supports for the
elements and disasters (decisions D-202 to D-207). W25 needs temperature
from vehicles package V1, which itself depends only on W14 and W16.

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M0 Foundation** | F1–F4 | `cargo xtask check` passes; a mod with a mistake produces a precise error. |
| **M1a A world you can look at** | W1–W11 | Generate the test city, browse it storey by storey in the terminal viewer, save, load, same hash. |
| **M1b A world you can wreck** | W12–W18 | From the viewer console: punch out a wall, drop a tower, set off a car bomb, throw a body through three buildings, start a fire. |
| **M1d A world with weather in it** | W24, W25 | Shake a district and watch the buildings nearest their limits fall. Send a wave up a street from the shore. Freeze a lake, walk across it, and go through where a cart is too heavy. Melt a wall of ice with a fire. |
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

As built (decision D-250). The workspace holds every crate of 01 §3, the
ones not yet started as compiling stubs with their dependency edges, so
that later packages fill a crate in rather than add one. The disallow lists
are in the root `clippy.toml`; the frontends, `clash-app`, `clash-tools`
and `xtask` opt out with a `clippy.toml` of their own. The check that size
literals appear only in `clash_world::dims` joined `cargo xtask check` with
W1, as its fifth step, `dims` (decision D-285).

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

As built (decisions D-244 to D-248). `Rot4` is the four quarter-turns and
`Turn` adds the mirror (10 §3.3). The positional hash takes a typed
`HashDomain`. `Value` is `#[non_exhaustive]` so that document 05 can add
kinds. A `VarMap` is keyed by `VarKey`, a qualified name. Parsing of
quantities and durations from text is left to `clash-data`.

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

As built (decision D-249). The interface is in 03 §3.1 and §3.6. Besides
the naive model, a second property test counts time one tick at a time in
integers and requires the timeline to agree to the tick. No benchmark is
committed: the budgets of 03 §15 are measured by W23, which the beta plan
leaves for later. A rough timing on the build machine gave about 250 ns
for schedule plus pop with 1,000 wake-ups pending and 340 ns with 10,000
(target 300 ns), and about 135 µs for a rate change on a clock with 1,000
wake-ups (target 200 µs).

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

As built (decisions D-260 to D-268). Built together with packages P1 to P4
of [sim-kernel.md](sim-kernel.md), so the loader is past the subset of
04 §19. A definition type is a plain struct that derives
`serde::Deserialize` and implements `clash_data::Def`; the crate that owns
it registers it in a `DefTypes`, and a `Loader` turns a profile, a
directory of mods or mods given as text into an `Arc<Defs>`. The fixtures
are under `crates/clash-data/tests/fixtures/`, and the definition types
they use are test types: the engine's own are registered by the crates
that own them, in `clash-tools`' `engine_types`.

Stubbed: JSON Schema generation. `clash-tools schema` answers that it is
not built yet; package P5 builds it, by hand from the registered types
since `schemars` is not among the workspace's dependencies. Files are read
one after another, not in parallel, and no benchmark is committed. A rough
timing on the build machine, optimised and on one thread: 50,000
definitions of four types with 20,000 inline ones, in 2,000 files given
from memory, loaded in about 0.95 s (target 2 s, 04 §18).

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

As built (decisions D-284, D-285). Positions order by `z`, then `y`, then
`x`. A realm's cells run from 0 to its size on each horizontal axis.
`CellBox` was added for boxes of cells. `cargo xtask check` gained the
`dims` step: the literals 32, 64, 1024, 2048, 4096 and 8192 may appear in
the source of `clash-world` and `clash-gen` only in `dims.rs`, in comments,
in strings, in test modules, or on a line marked `dims-ok`. The literal 8
is not looked for.

### W2 — Paletted storage

**Spec** 02 §6.1 · **Size** S

Build
- `Paletted<T>`, `SparseCells<T>`, fixed 8192-bit bitsets; growth, compaction,
  uniform detection; serialisation; `StateHash` independent of palette order.

Done when
- Property test against a plain `Vec<T>` model.
- A uniform layer is under 200 bytes; a 16-entry palette uses 4 bits per cell.
- Two layers with equal contents but different palette histories hash equal.

As built (decision D-281). The representation of `Paletted` is private.
The serialised form is the compacted, canonical one.

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

Status `beta` (decisions D-283, D-320, D-321). Built: the definition
types as plain `serde` structs; `WorldDefs::build`, which sorts, numbers,
resolves and derives, and reports every mistake; the state reference
parser; the content, as definition files under `mods/core/data/world/`;
`contact` on elements (review addition). The types are registered with
the loader by `clash_world::register_defs`, and
`WorldDefs::from_defs(&Defs)` builds the tables from loaded definitions
with the loader's dense ids. A mistake found then is a diagnostic at the
value in the file that wrote it. `clash-tools lint mods` passes, and runs
that step. Stubbed:

- Schema generation is the loader's (P5), and is not checked here.
- Quantities with units are still read by a private helper in
  `clash-world`, which also reads densities and energies per volume. The
  loader's quantity types replace it for the dimensions they have.
- References are still a newtype around the id, resolved when the tables
  are built and not by the loader: a bare name is looked up in the
  definition's own namespace, then in every namespace in sorted order,
  which stands in for the import list of the mod that wrote it. Making
  them `clash_data::Ref`s moves the check into the loader.
- No placeholder is supplied for lenient loading.
- Not read yet, each left to the package that gives it meaning: `changes`,
  rubble by damage type or as a field (W25); `fluid` (vehicles V1);
  `load`, `when_supplied`, `conveys` (devices plan). A material's
  `friction` is read since W14.

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

Status `beta` (decisions D-280, D-281). Built as specified in 02 §6.2 and
§6.3 as they now read: per-cell derived flags kept on every write, chunks
held inline in their column, `ColumnBuilder` for cells not yet in the
world. Stubbed:

- Benchmarks. The beta plan leaves W23 for later. Two ignored timing tests
  in `crates/clash-world/tests/cosmos.rs` print rough figures instead. On
  the build machine, for cells scattered at random over 64 tracts: a flag
  read 10 ns through a `GridView` and 12 ns through `Cosmos` (target 10 ns),
  a read of all three states 18 ns (target 30 ns). A uniform chunk is 184
  bytes (target 200).
- `FieldStore` and `OccupantIndex` were empty typed placeholders until
  W16 and W7 built them.
- Chunks and columns are not serialised: the tract blob codec is W10's.
- The write primitives for damage, decals and cell variables exist and are
  tested, and nothing calls them until `WorldEdit` (W6).

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

Status `beta` (decisions D-282, D-284). Everything listed is built; one
thing is stubbed, the last below. The read API returns
`Result<_, WorldError>`. `GridView` gives the same reads for many cells of
one grid. `Realm` holds what exists so far: id, definition, seed, clock,
base grid, variables and state; region records, stamps, areas, sites,
travelers and the plan join it with their packages. `ambient` reads the
stratum only; areas are consulted from W7. Until tracts load through
tickets (W8, W9), `clash_world::test_support` puts whole columns into a
cosmos for tests. `Cause` and `Decal` were defined here, ahead of W6,
because the decal layer stores them. Stubbed: the `realm` definition type is in
the same state as those of W3: registered with the loader, with its
references resolved when the tables are built. `mods/core` holds two small
realms, `testland` and `training_room`, for tests and tools.

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

Status `beta` (decisions D-300, D-301, D-305). Built as 02 §7.2 to §7.4
now read: `WorldCtx`, `WorldEdit` with the element setters, `set_material`,
`set_damage`, marks and cell variables; `WorldEvent` and `EventSink`; the
bookkeeping, with a stability queue that W13 empties; and what 02 §23 says
of an edit that meets an occupant. Whole columns still enter and leave the
world through `test_support`, until tracts load (W8). Stubbed:

- No benchmark for the single-edit budget (W23).
- `damage` with a packet, `add_field` and the conduit edits are W12's,
  W16's and the devices plan's.
- `edit.displace_range` is a named constant until settings are bound (P6).
- Events state their fields in 02 §7.4; the generated reference that would
  check them is tooling package TL9's.

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

Status `beta` (decisions D-302 to D-304, D-306). Built as 02 §8 now
reads: the occupant index with `place`, `relocate`, `remove`, box
footprints and the four queries; planes; the area table with its per-tract
lists, enter and exit events, clocks, ambient overrides and anchors; the
`area_kind` and `plane` definition types; the site table; and the two
review additions, drift in areas and `enclose`. Stubbed:

- `relocate` computes no portal crossings (W19).
- An area's journal span and a subgrid anchor are stored and do nothing
  (08 §7.4; W20a). A `sealed` kind is a flag: sealed volumes are the
  vehicles plan's.
- `Cosmos::ambient` reports an area's drift as set. The velocity at a
  cell, and drift from wind and current, come with the medium (V1).
- `area_kind` and `plane` are in the same state as the definitions of W3:
  built by `WorldDefs::build`, not registered with the loader. The `site`
  definition type is document 10's: a site holds a bare `SiteDefId`.
- A change of ambient gravity over an area asked for no stability check.
  It does since W13: see that package.
- No benchmarks (W23).

### W8 — Tracts, regions, generation, loading

**Spec** 02 §9 · **Size** L

Build
- `RegionRecord`, `TractRecord`, `TractExtra`; lifecycle states.
- `RegionSurveyor`, `TractRealizer`, `GenCtx`: defined in `clash-world`,
  which calls them, and implemented and re-exported by `clash-gen`.
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

Status `beta` (decisions D-370 to D-375). Built as 02 §9 now reads: the
records, kept by each realm; the generation interfaces in
`clash_world::generate`, with a registry that binds the generator a realm
definition names and the parameters it gives (`generator_params`); `flat`
and `test_city` in `clash-gen`, built only from states their parameters
name, with stairs, a sidewalk floor, a roof and the realm `core:test_city`
added to `mods/core`; `WorldCtx::load_tracts` and `unload_tract` against a
`TractStore`, with `MemoryStore` and a column codec behind
`encode_column` and `decode_column`; realisation in batches on a
`BatchRunner`, with `clash_gen::RayonRunner`; and the rule of 02 §22 that a
tract loaded again does not look unchanged to a cache.
`cargo run -p clash-gen --example city_ascii -- [seed] [storey]` prints a
block of the city, and a test holds the map for seed 1 against
`crates/clash-gen/tests/golden/`. The tests are in
`crates/clash-gen/tests/tracts.rs` and `crates/clash-world/tests/store.rs`.
Stubbed:

- Stamps and total devastation at load, and with them the ruin variant of
  `test_city` (W17). A pending stamp is stored and stays pending.
- The catching up of fields at load (W16), and the checks that no body or
  process is left in a tract that unloads (W14, W13).
- Pending edits are explicit cell changes, spawn requests and stamps. A
  map, a liquid level, devastation and a rule to run join `PendingEdit`
  with their packages.
- `RegionSurvey`, `Placement`, `PathLine`, `Stamp`, `SpawnRequest`,
  `PortalSpec` and `SubgridSpec` are typed and carried and nothing reads
  them. `TractDefId` is a bare id: the `tract` definition type is
  document 10's. A generator's portals and subgrids are dropped.
- The tract blob held runtime ids, no entity section and no
  fingerprint. W10 has since built the final layout, the save-id tables
  and the `redb` store.
- A mistake in a realm's `generator_params` names the definition and the
  parameter's path and is not yet a diagnostic with file and line: the
  linter does not bind generators (`clash-tools` was closed to this
  change). `Generators::param_errors` is what it would call.
- No benchmark (W23). An ignored timing test in
  `crates/clash-gen/tests/tracts.rs` prints rough figures instead. On the
  build machine: realising a `test_city` tract takes about 0.4 ms on one
  thread and 0.1 ms a tract on four (target 2 ms); storing and loading a
  changed tract about 0.3 ms together (target 1 ms to load); a stored
  city tract is about 1 KB.
- No test script: the runner is W11's.

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

Status `beta` (decisions D-376 to D-378). Built as 02 §10 now reads: the
ticket table in the cosmos, `hold_ticket`, `release_ticket` and
`move_ticket`, fixed and entity anchors, levels with the warm margin, the
unloading margin, and `WorldCtx::reconcile_activity`, the one function the
simulation calls at a safe point, with the budget, the refusal order and
`TicketRefused`. The tests are in `crates/clash-gen/tests/tickets.rs`,
beside the generators they load with. Stubbed:

- `Body` and `Subgrid` anchors (W14, W20a): their holders move a fixed
  anchor until then.
- Forcing a process or a body whose ticket was refused to resolve is the
  holder's part (W13, W14). The world reports the refusal.
- `max_loaded_tracts` and `world.hot_radius` are a field of the ticket
  table and a named constant until settings are bound (P6).
- `hold_ticket` and `release_ticket` are functions, not yet verbs: the rule
  runtime binds them.
- No benchmark (W23) and no test script (W11).

### W10 — Persistence

**Spec** 02 §11 · **Size** M

Build
- `TractStore`; in-memory and `redb` implementations.
- Versioned tract blob codec with LZ4; save-id table; remapping on load;
  placeholders that keep the original string id. The encoding and the
  blob frame are already built, in `clash_core::codec` (01 §11.1, D-251):
  this package chooses the tract blob's kind and version and writes its
  body with them.
- Pristine policy and generator fingerprint.
- Serialisation of realm and cosmos world state.

Done when
- Random columns round-trip (property test).
- A whole cosmos hashes equal after save and load.
- Removing a mod yields placeholders; restoring it yields the original
  content.
- A pristine tract writes no blob.
- A corrupted blob produces an error, not a panic.

Status `beta` (decisions D-480 to D-486). Built as 02 §11 now reads: the
tract blob at version 2, with states by save id, an optional column and
the simulation's entity section (`encode_tract`, `decode_tract`,
`Cosmos::set_tract_entities`, `LoadedTract::entities`); `SaveIds` and
`DefKind`, with placeholders added to the definitions when a save is
loaded and `Cosmos::take_save_warnings`; named sections on `TractStore`,
`MemoryStore`, and `RedbStore`, one database file for each save; the
generator fingerprint in each tract's record, `LoadedTract::regenerated`
and `Cosmos::set_store_pristine`; and `Cosmos::save_world` and
`Cosmos::load_world`, with the sections listed in `WORLD_SECTIONS`. The
save in the middle of `tower_pancake` is a real one. The tests are in
`crates/clash-world/tests/save.rs` and `tests/store.rs`. A save of a
world with nine loaded tracts, three of them changed, is about 1.5 KB;
a stored `test_city` tract is about 1 KB. Stubbed:

- Migration entries (04 §12.1) are not consulted: the `migration` type
  is P7's. `SaveIds::bind` is where they go.
- Region records are not blobs of their own: `world.cosmos` holds every
  realm's records and is written whole at every save, about 37 KB a
  surveyed region before compression and a few hundred bytes after.
- Decal, conduit, site, tract, devastation kind, map, region kind and
  path kind ids are written as numbers: their definition types are not
  built. A named state is written as its position.
- The columns of grids that are not a realm's base grid are not saved
  (subgrids, W20a). Fields join the chunk's stored form and bodies a
  section of their own with W16 and W14.
- `save.store_pristine` is a function of the cosmos until settings are
  bound (P6).
- No test script (W11) and no benchmark (W23). An ignored test in
  `tests/save.rs` prints the sizes of a save.

### W11 — World viewer and scenario runner

**Spec** 01 §12; 14 §3, §7 · **Size** M

This package builds the first form of the console and the test script
runner. Packages TL1 and TL6 of [tooling.md](tooling.md) generalise them.

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
  scenario straight, twice, with save and load at the split points of
  14 §7.2, and with two thread counts. `--bless` rewrites the expected
  hash. The format and the runner are built: `clash_sim::console`
  (tooling packages TL1 and TL6). This package supplies the host they run
  against.

Done when
- The test city can be generated and browsed.
- `smoke_city` runs in CI in all four modes.

Beta: built in `clash-save` (the console's host, `clash_save::host::GameHost`)
and `clash-tools` (`worldview`, `scenario run`). Built: `clash-tools
worldview`, a terminal viewer of one storey round the cursor through the
scene's `WorldView` in omniscient mode, with pan, storey up and down and a
console line, and a script mode that runs console lines from a file or a
pipe and prints frames as text; the host's commands `gen`, `spawn`, `move`,
`remove`, `set`, `control`, `session`, `damage`, `field`, `look`, `frame`,
`advance`, `step`, `save`, `load`, `hash`, `check`, `inspect` (14 §3.3);
`scenario run` against the real simulation in the four modes, with
`--bless`; scripts under `scenarios/world/`, `smoke_city` among them, run
by `cargo xtask check`. Both "Done when" items hold. Not built: the region
map of tract kinds and devastation; decals and causes in the cell
inspector (`look`); the commands `ticket`, `release` and `launch` (`session`
and `gen` hold tickets; nothing launches a body from the console yet).

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

Status `beta` (decisions D-380 to D-382, D-386). Built as 02 §12 now
reads: `DamagePacket`, `DamageOutcome`, the rule as `resolve_packet`,
`WorldEdit::damage` and `damage_along`; destruction with `on_destroy`,
rubble that keeps matter, piles up and becomes a heap, and
`ElementDestroyed`; `OccupantUnsupported` from any edit that takes an
occupant's footing away. Stubbed:

- The debris hook threw nothing until W14 filled it in: a destruction
  with a direction now throws fragments, within `debris.max_bodies`
  (02 §12.3, D-493).
- A material names one rubble fixture. Rubble by damage type and rubble
  that is a field are W25's.
- The body of `wall_punch` is the test `a_wall_is_punched_through` in
  `crates/clash-world/tests/damage.rs`. The script is owed to the
  script runner (W11).
- No benchmark (W23).

### W13 — Structural support and collapse

**Spec** 02 §14 · **Size** L

Build
- The stability queue consumer with merging and bounded island discovery.
- Storey-by-storey evaluation, recording each element's carrier.
- The load pass of 02 §14.5: loads summed down carriers, crushing by
  `bearing` reduced by damage, repeated until stable; checks queued when an
  area's ambient gravity changes.
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
- A pillar that bears its load whole is crushed when damaged to the
  fraction 02 §14.5 predicts.
- Removing every second ground-storey support of a tower overloads those
  that remain, and the tower comes down without any further blow.
- A structure that stands under normal gravity is crushed when its area's
  gravity is raised enough, and nothing is crushed at zero gravity.
- A material with no `bearing` is never crushed.
- Scenario `tower_pancake`, including a save in mid-collapse.
- Scenario `weakened_columns`: damage short of destruction brings a
  building down by load alone.

Status `beta` (decisions D-383 to D-386). Built as 02 §14.1 to §14.3
and §14.5 now read: the consumer of the stability queue with the settle
delay on the clock of the change; `Cosmos::assess_stability` with island
discovery, the support pass with carriers and the load pass; crushing;
falling masses as processes of the world, with landing blows, cascades,
rubble and `OccupantExposed`; collapses with `CollapseStarted` and
`CollapseSettled`; checks when gravity changes over an area; and
`WorldCtx::handle_wake`, the world's one entry point for wake-ups, which
the kernel calls. Stubbed:

- No ticket is held (W9): the place is marked `W9:` in
  `collapse.rs`. A mass that arrives where nothing is loaded is dropped.
- Resolve mode (02 §14.4) is not built. W15 and W17 need it, and
  force-resolving a collapse with it.
- Floating floors (02 §14.1) are W25's, and the tremor's factors on
  span and bearing W24's.
- The save in the middle of `tower_pancake` was the timeline and
  `Cosmos::processes` with copied cells. Since W10 it is a real save:
  `Cosmos::save_world` and `load_world`.
- The bodies of `tower_pancake` and `weakened_columns` are tests of
  those names in `crates/clash-world/tests/collapse.rs`. The scripts are
  owed to the script runner (W11).
- A timing test prints figures (`timing_of_a_check_and_of_a_collapse`,
  ignored); the benchmark and its budget are W23's.

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

Status `beta` (decisions D-490 to D-494). Built as 02 §15 and the
debris paragraph of §12.3 now read: `Body` with its three kinds of
owner, `launch`, `impulse`, `steer` and `arrest`; flight as a process on
the clock of where the body is, exact in vacuum, with drag against the
moving medium and a drive; impacts along the projectile ray with the
roll for what closes part of a cell, on elements through
`WorldEdit::damage_along` and on occupants as `BodyHitOccupant`; rest,
sliding by the `friction` of materials, and landing; occupants carried
cell by cell; debris from the hook of W12 within `debris.max_bodies`;
the ticket ahead; and flights that leave loaded space finished by
arithmetic, with `PendingEdit::Impact` for those that come down unseen.
With the review additions drift on bodies, driven bodies, contact on
elements and the W14 half of leaving loaded space. Stubbed:

- A body that is a subgrid (W20b), portals and boundary links in a
  body's way, and `LeftRealm` at a void edge (W19): a body that leaves
  by an edge is reported by `BodyLeftLoaded` with nowhere to land.
- The ticket has a fixed anchor that the body moves: `TicketAnchor`
  gained no `Body` variant. A body whose ticket is refused is not told:
  it is finished by arithmetic when it comes to unloaded space, which
  is what refusal asks for.
- The tract records keep no ground heights, so the arithmetic takes the
  ground to lie where it did under the last loaded cell. Applying a
  pending impact when its tract loads is the coarse layer's (W17), as
  is leaving the entity there (06 §12).
- The world knows no occupant's shape: whoever is in a cell a body
  enters is struck. `volume` is stored for buoyancy (11 §4.4) and not
  used.
- The bodies of `three_buildings`, `time_stop_street` and
  `over_the_horizon` are tests of those names in
  `crates/clash-world/tests/bodies.rs`. The scripts are owed to the
  script runner (W11).
- Saved with the world (W10, D-499): a flight is a process in
  `world.activity` and a waiting impact is in its tract's record, with
  the material, damage type and planes they name written as save ids.
  The save in mid-flight and the one in `over_the_horizon` are real
  saves. The simulation places the entity a body carries again after
  loading, as it does every occupant.
- A timing test prints figures
  (`timing_of_a_projectile_across_thirty_metres`, ignored): about 13 µs
  for a flight of 30 m, 450 ns a step. The benchmark and its budget are
  W23's.

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

Status `beta` (decisions D-495 to D-498). Built as 02 §13 to §13.2 now
read: `release` for a sphere, a cone and a beam, of any damage type; the
front as a process that handles a shell a wake on the clock of its
origin; the transmission map, with shade that falls when a shell is
done; packets to floors, blocks and fixtures within what the release has
left, so that no more is absorbed than was released; `OccupantExposed`
once for each occupant the front reaches, the push on bodies and
occupants and the throwing of occupants the wave moves fast enough;
confinement, by which a charge in a closed space does more than in the
open and an opening lets it out; a beam that pays for what it breaks and
may burst where it ends; the budget, as a cut-off; `ReleaseStarted` and
`EnergyReleased` with totals by instigator. Stubbed:

- The three tiers: every non-empty cell in reach is handled with
  shadowing, and nothing is cleared in bulk. A megaton is cut off by the
  budget long before its reach.
- Resolve mode (02 §13.4) and force-resolve: a release over
  `release.max_cells` is cut off at the shell that would pass it and
  says so (contract point 4 is tested as that). Coarse mode (02 §13.3):
  cells that are not loaded are passed over, with no stamp (W17).
- No ticket is held (W9), so a release does not keep its tracts loaded.
- Fronts (02 §13.5) are W24's.
- "Wall destruction radii match the table" is tested as the radius out
  to which walls are marked: the table gives the radius at the wall's
  threshold, where a wall is affected and far from destroyed.
- The bodies of `car_bomb_street` and `beam_through_city` are tests of
  those names in `crates/clash-world/tests/releases.rs`, the first with
  a save while the front is spreading. The scripts are owed to the
  script runner (W11).
- Saved with the world (W10, D-499): a release in progress is a process
  in `world.activity`, with its damage type written as a save id. The
  save in `car_bomb_street` is a real one, and
  `what_is_in_the_air_keeps_what_it_names_when_the_mods_change` loads
  one without the mod that defined a damage type, a material and a
  plane, and with it again.
- A release tells the fields of each cell it reaches (`conduct`, D-499).
  A reaction to damage is the field's: with core's water and a shock,
  every wet cell the front reaches passes a packet to every wet cell
  within six steps, which over a large body of water is many packets
  for each cell and is not bounded by `release.max_cells`.
- A timing test prints figures (`timing_of_a_blast_in_a_city_block`,
  ignored): a 2 GJ blast among nine five-storey buildings visits 136,000
  cells in about 28 ms, 200 ns a cell, and what follows it (collapse and
  debris) takes about 15 ms. The benchmark and its budget are W23's.

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

Status `beta` (decisions D-500 to D-509). Built as 02 §16 now reads: the
`field` and `field_reaction` definition types; sparse storage per chunk
and field with levels, activity and who is responsible, stored with the
tract; `WorldEdit::add_field`, `set_field` and `conduct`; the stepper as
a process of the world for each clock and step interval, with every rule
(`decay`, `diffuse`, `flow`, `spread`, `emit`, `convert`, `damage`,
`drift`, `expose`), reactions, sleeping and waking, the budget and
catch-up at load; `FieldAppeared`, `FieldVanished`, `FieldContact`,
`OccupantDosed`, `FieldBudgetExceeded`, and `OccupantExposed` from
fields; `field_hides_sight` and `field_lights`; fire, smoke, steam and
water in `mods/core/data/world/fields.json`. The review additions "Field
reactions", "Drift on bodies and fields" (the fields half) and "Exposure
from fields" are built with it. Stubbed:

- No `heat` field in `core`, and `warms` and `changes` on a field are
  stored only: temperature is V1's and change of state W25's, as is
  `leave` on a reaction.
- Sight is not blocked by fields in rays, lines of sight or fields of
  view: `Cosmos::field_hides_sight` is the hook (02 §22).
- Drift is what areas set. The region's wind and current come with the
  weather.
- `settle` keeps and wakes a liquid; it does not settle it in closed
  form. Catch-up is tested by itself, and a tract's fields through its
  blob; a test that unloads and reloads a burning tract through a store
  belongs with the generators' tests in `clash-gen`.
- Releases and bodies call `conduct` (W15, W14). Attacks must too, for a
  reaction to damage to happen.
- The bodies of `office_fire` and `quench` are tests of those names in
  `crates/clash-world/tests/fields.rs`. The scripts are owed to the
  script runner (W11).
- A timing test prints figures (`timing_of_a_building_fire`, ignored):
  about 0.75 microseconds for each active cell each step, so a fire of
  23,600 active cells takes about 20 ms a step: well over the budget of
  02 §25. The time goes in reading six neighbours through the sparse
  layers and in summing a step's changes; a dense form for chunks that
  are mostly full is the remedy. The benchmark and the budget are W23's.

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

Status `beta` (decisions D-310 to D-316). Built in
`clash_world::query` as 02 §22 now reads: rays with heights and three
kinds of filter, lines of sight, field of view into a `FovSet` kept by
the caller, `neighbours` and `footing`, `RevStamp` and the caches on it.
Added beyond the list: a search over cells, `find_path`, with the mover's
rules as a trait (D-315). Built alongside W7, not after it. Stubbed:

- Occupants along a ray. The ray takes a `RayObstacles`; the occupant
  index (W7) implements it. Until then rays, lines of sight and views are
  terrain only.
- Subgrids and portals in queries, which are W20a's and W19's own items,
  and fields that hide (W16).
- Benchmarks (W23). Two ignored timing tests in
  `crates/clash-world/tests/query.rs` print rough figures instead. On the
  build machine: a field of view of radius 30 takes about 70 µs in an
  open field under open sky, 45 µs on one open storey under a ceiling,
  3 µs inside a building of small rooms, and 0.1 µs when asked again of
  an unchanged world; a line of sight over 30 m 0.2 to 0.4 µs; a path of
  60 steps about 50 µs in the open and 60 µs through a building.
- No test script: the runner is W11's. The queries change no state, so
  there is no golden hash to keep.
- Nothing is registered with the expression runtime, which does not
  exist yet. `line of sight` (02 §23) is bound when it does.
- A tract unloaded and loaded again could look unchanged to a cache. W8
  has since built the rule in 02 §22 that prevents it (decision D-375).

### W19 — Portals, boundary links, realm transfer

**Spec** 02 §19 · **Size** M

Build
- Portal table; traversal for occupants and bodies; filters; events.
- Edge behaviour: barrier, void, and realm and entry exits. The chart exit
  emits an event until W21.
- The realm transfer function.
- Sight, rays and signals through see-through portals by the rule of
  02 §22: within `view_range`, to `sight.portal_depth`, with the far-side
  ticket.

Done when
- An occupant walks between realms through a portal pair and its clock
  changes with the realm.
- A body keeps its speed through a portal, with direction rotated by facing.
- A void edge removes with an event; a barrier edge stops a body with an
  impact.
- A viewer within range sees one portal deep and no further; out of range,
  not at all; and the far side is loaded only while someone could see it.

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

### W24 — Fronts, tremors and kinds of devastation

**Spec** 02 §13.5, §14.6, §17.2 · **Size** M

Build
- `ReleaseShape::Front`: intensity by swept volume; a base that follows
  the surface; `downhill`; the transmission map laid across the front; the
  push along `dir`; `leaves`; stamps for fronts in unloaded tracts.
- `tremor`: the process, the factor on span and bearing, checks spread
  within the edit budget, `OccupantShaken`, devastation for unloaded
  tracts.
- The `devastation_kind` type; `devastate`; `PendingEdit::Devastation`;
  the kind on `RegionDevastated`; ruin variants chosen by kind.

Done when
- Energy absorbed by a front never exceeds energy released (invariant W8),
  and a wall that survives shelters what is behind it.
- Test script `the_wave`: a front runs up a street from the shore, breaks
  what its intensity exceeds, pushes occupants along its direction, leaves
  water behind it and stops at ground higher than itself. An unloaded
  tract in its path shows the same devastation when loaded.
- Test script `the_tremor`: in a district of mixed buildings, those
  nearest their span and load limits fall and the rest stand. When it ends
  a building that stood is as strong as before. Anchored rock does not
  fall.
- A tract devastated while unloaded and one devastated identically while
  loaded end with the same value and kind (invariant W12).

### W25 — Change of state

**Spec** 02 §5.4, §12.3, §14.1, §16; 11 §4.6 · **Size** M

Build
- `changes` on materials and fields; the three moments at which they are
  tested; the scheduled crossing and the sweep within the edit budget;
  `cover`.
- Rubble by damage type, including fields.
- `warms` on fields; `leave` on reactions.
- Floating floors as supports, and what they can carry.

Done when
- Nothing is scheduled for temperature while no threshold is near
  (invariant W11), shown by a benchmark of an idle frozen region.
- Test script `the_thaw`: the region cools below the threshold and open
  water gains a sheet of ice; a person crosses a wide sheet and a loaded
  cart goes through it; the region warms and the sheet goes. A tract
  loaded only afterwards matches one that was loaded throughout.
- Test script `ice_wall`: a fire's heat melts a wall of ice into water,
  which quenches part of the fire; lava reaching water leaves rock.
- Ice destroyed by heat leaves water, and by impact leaves broken ice.

## Scenario catalogue

Each scenario is a console script with a golden hash, run in all four modes.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `smoke_city` | W11 | — | Generate, browse, save, load |
| `wall_punch` | W12 | S1 | Threshold, capacity, rubble |
| `tower_pancake` | W13 | S2 | Support loss, cascade, mid-collapse save |
| `weakened_columns` | W13 | S2 | Load on damaged supports |
| `three_buildings` | W14 | S2 | Flight, wall-by-wall energy loss, landing |
| `car_bomb_street` | W15 | S3, S5 | Shells over time, shadowing, exposure |
| `beam_through_city` | W15 | S8 | Beam march and terminal release |
| `office_fire` | W16 | — | Spread, fuel, smoke, burn-out |
| `offscreen_fight` | W17 | S7 | Pending edits and stamps applied at load |
| `continent_scar` | W17 | S8 | Realm stamp, devastation, ruin generation |
| `time_stop_street` | W14 | S10 | Bodies freeze in a rate-0 area and resume |
| `thrown_building` | W20b | S11 | Detach, flight, merge |
| `rooftop_to_orbit` | W21 | S9 | Strata, edge exit, carrier, arrival |
| `the_wave` | W24 | S8 | A front: intensity, shelter, push, what it leaves |
| `the_tremor` | W24 | — | Weakened span and bearing; devastation by kind |
| `the_thaw` | W25 | — | State change by region temperature; floating floors |
| `ice_wall` | W25 | — | State change by fields; rubble by damage type |
| `quench` | W16 | — | Field reactions |
| `over_the_horizon` | W14, W17 | — | Leaving loaded space |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Field reactions | W16 | 02 §16 | Test script `quench`: water and fire in one cell leave steam in the stated amounts; a field meeting a tagged material reacts; a shock packet arriving in water reaches every connected water cell within reach at the stated share. A field's own `convert` gives the same result as the equivalent reaction. |
| Liquid level as a pending edit | W17 | 02 §17.3; 11 §4.3 | A tract given `PendingEdit::Liquid` while unloaded loads filled or drained to that storey, with the same state hash as the same change made while it was loaded. |
| Drift in areas | W7 | 02 §8.3; 11 §4.7 | An area's ambient override may set a steady flow or a vortex, and `Cosmos::ambient` returns it. |
| Drift on bodies and fields | W14, W16 | 02 §15.1, §16; 11 §4.7 | A light body thrown across a wind lands downwind of where it lands in still air. Smoke with the `drift` rule moves downwind, and fire spreads further downwind than upwind in the same time. Until vehicles package V1 exists, drift is supplied by a test fixture. |
| Enclosing | W7 | 02 §8.3 | `enclose` in a closed room creates an area over exactly its open cells; in an open one it is refused and names a leak. Removing a wall cell marks the area `breached`; replacing it clears the mark. |
| Driven bodies in flight | W14 | 02 §15 | A body with a `drive` gains speed for its burn time and turns toward the point given to `steer` no faster than its turn rate. With no point it flies straight. |
| Leaving loaded space | W14, W17 | 02 §15.4 | Test script `over_the_horizon`: a shell fired across twenty tracts loads none of them, comes down at the place and time its flight gives, and its impact is found when that tract is loaded. One that comes down in a loaded tract lands in full. A person thrown the same distance is there afterwards. |
| Exposure from fields | W16 | 02 §16 | The `expose` rule raises `OccupantDosed` for each occupant with the amount its level gives. The world applies no routes; the simulation does. |
| Contact on elements | W3, W14 | 02 §5.4, §15.2 | An element's `contact` loads and is reported with a body impact against it. |
| Timeline ordering and the clock tree | F3 | 03 §3.2, §3.3; 01 §5 | Wake-ups pop in (tick, sequence) order; a clock's rate change preserves the remaining local time of everything on it (invariants T1, T2). |
| Mod layout, `mod.json` and ids | F4 | 04 §3.1, §3.2, §4.1, §4.2 | A mod directory with a `mod.json` loads; ids are namespaced and an id that breaks the form is a diagnostic. |
| Interiors that do not fit | W20a | 02 §18.4 | A vessel map whose interior is larger than its envelope is a load error that says to use a realm behind a portal. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
