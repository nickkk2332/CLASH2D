# 01 — Architecture

This document describes the whole engine at the level of contracts: what the
parts are, what each may depend on, and the rules every part obeys. Subsystem
documents (02 onward) fill in one part each and must stay inside these
contracts. See [roadmap.md](roadmap.md) for the list of subsystem documents.

## 1. Shape of the system

```
                 ┌───────────── frontends ─────────────┐
                 │  window (GPU glyph grid) │ terminal  │
                 └───────────────▲─────────────────────┘
                                 │ glyph frames
                          ┌──────┴──────┐
   player input ─────────►│    scene    │  camera, UI, cutscene director
        │                 └──────▲──────┘
        │ Commands               │ SimEvents (timestamped) + read-only views
        ▼                        │
  ┌─────────────────────────── sim ───────────────────────────┐
  │  entities · bodies · actions · combat · powers · AI        │
  │  EOC runtime · cold simulation                             │
  │      ┌───────────────────── world ──────────────────┐      │
  │      │ cosmos · realms · grids · fields             │      │
  │      │ damage · collapse · bodies in flight         │      │
  │      └──────────────────────────────────────────────┘      │
  │  time: ticks · clocks · event queue · processes            │
  └───────────▲─────────────────────────────▲──────────────────┘
              │ definitions                 │ load / store
          ┌───┴───┐                     ┌───┴───┐
          │ data  │◄── mods/*.json      │ save  │
          └───────┘                     └───────┘
```

The simulation is a pure state machine. It takes commands, advances time, and
emits events. Everything a player sees is produced by the scene layer from
those events and from read-only views of simulation state.

## 2. Hard rules

These apply to all code in the simulation (`clash-core`, `clash-time`,
`clash-data`, `clash-expr`, `clash-world`, `clash-gen`, `clash-sim`).

| # | Rule |
|---|------|
| R1 | **Deterministic.** Same starting state, same mods, same commands give bit-identical state. Section 4 lists what that forbids. |
| R2 | **No content in the engine.** Engine code never names a specific material, creature, power or rule. It names *kinds* of definition and reads instances from data. |
| R3 | **One way to change each kind of state.** World cells change only through `WorldEdit`. Entity positions change only through one relocation function. This is what keeps caches, events and saves correct. |
| R4 | **Events out, never calls up.** Lower layers emit events; they never call into higher layers. `clash-world` does not know that entities, EOCs or a renderer exist. |
| R5 | **The sim never waits.** No wall-clock time, no I/O, no blocking on input inside the simulation. `advance` returns when a decision is needed. |
| R6 | **Bad data is an error message, not a crash.** Mod mistakes are reported at load with file, line and JSON path. A running simulation never panics because of content. |
| R7 | **Save/load is invisible.** Running N steps, saving, loading and running M more gives the same state hash as running N+M steps. |
| R8 | **SI units.** Metres, kilograms, seconds, joules, pascals, kelvin. No bespoke units in engine code or data. |
| R9 | **Constants are named.** Dimensions such as chunk size are constants in one module and never appear as literals. |
| R10 | **Mechanism, not meaning.** The engine has no concept of magic, race, class, alignment, era, tech level, difficulty or balance. It offers general mechanisms (see 00 §2) and never scales or softens an outcome. If a feature only makes sense in one genre, it belongs in a mod. |
| R11 | **Characters act on belief, not truth.** World state and the event stream are ground truth. A character's decisions may read only what that character has perceived or been told. Truth is for physics, saves, the inspector and scoring. |

## 3. Workspace layout

A Cargo workspace. Arrows mean "may depend on". A crate may also name
directly any crate its arrows reach: `clash-scene` uses `clash-core`.

```
crates/
  clash-core            maths, ids, deterministic RNG, state hashing, errors
  clash-time      → core                 ticks, clocks, event queue, processes
  clash-expr      → core                 expression language: parse, check, compile, evaluate
  clash-data      → core, expr           mod loading, JSON → typed definitions, registries, schemas
  clash-world     → core, time, data     cosmos, realms, grids, fields, damage, collapse, bodies in flight
  clash-gen       → world                region survey and tract generation (mapgen)
  clash-sim       → world, gen, expr     entities, actions, combat, powers, AI, EOC runtime, cold sim
  clash-save      → sim, scene           save container, versioning, migration; the console's host
  clash-scene     → sim                  presentation model, camera, UI state, cutscene director
  clash-front-window → scene             window + GPU glyph renderer
  clash-front-term   → scene             terminal renderer
  clash-app       → front-*, save        the game binary
  clash-tools     → save, front-term     mod linter, schema dump, headless runner, world viewer, benches
mods/
  core/                                  minimal definitions the tests and tools need
docs/
```

An arrow carries what its target depends on: a crate may depend directly on
any crate its arrows already reach, as `clash-gen` does on `clash-core` and
`clash-tools`, through `clash-sim`, on `clash-data`. What stays forbidden is
a dependency upward or sideways, on a crate the arrows do not lead to.

`clash-save` reaches `clash-scene` because the console's host lives there,
below both the application and the tools, and loads profiles with every
definition type and draws frames through the scene's view model (D-581).
`clash-tools` reaches `clash-front-term` for the terminal world viewer
(D-582).

`clash-tools` reaches `clash-scene` because the linter must load mods with
every definition type the game has, and the interface's are the scene's.

Definition *types* live in the crate that gives them meaning (`BlockDef` in
`clash-world`, `ActionDef` in `clash-sim`). `clash-data` provides the loading
framework and knows nothing about what a block or an action is.

`clash-data` depends on `clash-expr` because expressions are compiled while
mods load (04 §7.3). The loader compiles against a registry that the
simulation fills and hands to it, so the data layer still names no kind of
subject and no property. Every crate above reaches the expression language
through `clash-data`, and `clash-expr` depends on nothing but `clash-core`.
The two definition types that belong to the expression language itself,
`function` and `variable`, live in `clash-data` with the stage that
compiles them.

## 4. Determinism contract

Determinism is the property most easily lost by accident, so the rules are
mechanical and enforced by lints and tests.

**Numbers**

- Space and time are integers: cell coordinates `i32`, ticks `i64`.
- Physical magnitudes (energy, mass, force, pressure, velocity, positions of
  bodies in flight) are `Real`, a newtype over `f64` in `clash-core`. `Real`
  exposes `+ - * /`, `sqrt`, comparisons and conversions, which are
  IEEE-exact on the supported targets (x86-64, aarch64). Anything else
  (`sin`, `pow`, `exp`, `ln`, …) goes through wrappers backed by the pure-Rust
  `libm` crate. Calling the `std` float functions directly is denied by lint.
- `Real` never holds NaN. Where IEEE arithmetic would give NaN (`0/0`,
  `∞ − ∞`, the square root of a negative number), the plain operators and
  functions give zero: that is the defined fallback. `Real::new` and the
  `try_` functions return an error instead. Infinities are allowed. `Real`
  compares as IEEE does, so the two zeros are equal, and with no NaN the
  order is total.
- A `Real` survives saving and loading bit for bit. For JSON this needs
  `serde_json` built with `float_roundtrip`, which the workspace turns on.
- The energy range the engine must handle is about 1 J to 10^33 J. `f64`
  covers this with margin. Do not introduce a logarithmic "power level"
  representation in the engine; a derived tier for UI and AI is fine.

**Randomness**

- *Positional* randomness for generation: a stateless hash of
  `(seed, domain, coordinates, salt)` seeds a local generator. The result
  depends only on where and what, never on when or in what order.
- *Stream* randomness for simulation: named generator streams stored in
  simulation state and saved. Systems draw from their own stream. A
  stream's seed depends only on the simulation's seed and the stream's
  name, never on which other streams exist or when it was first used.
- No thread-local or OS randomness anywhere in the simulation.
- The algorithms are fixed: xoshiro256\*\* seeded through SplitMix64 for
  the generator, rounds of the SplitMix64 mixer for the positional hash,
  and FNV-1a for turning a domain or stream name into a number.

**Ordering**

- Simulation state never iterates a hash map. Use `Vec`, `BTreeMap`,
  `IndexMap`, or `LookupMap` (a hash map wrapper in `clash-core` that offers
  lookup, insert and remove but no iteration). So that what it holds can
  still be saved, hashed and now and then swept, a `LookupMap` can hand out
  its contents sorted by key, and it serialises and hashes in that order.
- Simultaneous events are ordered by `(tick, sequence number)`.
- Parallelism is allowed only where the result is independent of scheduling:
  pure generation of tracts, compression, read-only queries, and two-phase
  updates (compute deltas in parallel, apply in a fixed order).

**Hashing**

- Every state type implements the `StateHash` trait, feeding a stable hasher
  in a fixed order. The result is a `Hash128`. The simulation exposes a
  total hash and per-subsystem hashes so a divergence can be localised.
- The hasher is fixed: MurmurHash3 (`x64_128`, seed 0) over the bytes fed
  to it. Integers are fed little-endian at their own width, anything of
  variable length is fed its length first, and a `Real` is fed by its exact
  bits.
- Golden-hash tests run scripted scenarios and compare hashes. A change that
  alters a golden hash must say why in its description.

**Enforcement**

- The workspace's `clippy.toml` disallows std hash maps and sets, the std
  float functions whose results differ between platforms, `std::time` and
  thread-local randomness. `cargo xtask check` denies warnings, so a use of
  any of them does not pass. A crate outside the simulation (section 2)
  that needs the wall clock carries a `clippy.toml` of its own, which
  replaces the workspace's for that crate. Inside the simulation, only a
  wrapper that makes a disallowed item safe may allow it, with a reason:
  `LookupMap` is the one such place.
- Reference vectors for the `Real` functions, the generator, the positional
  hash and the hasher are committed as tests in `clash-core`. They are the
  cross-platform contract: a vector is never changed to make a test pass.

## 5. Time

### 5.1 Ticks and the timeline

- `Tick(i64)` counts microseconds of universal time. Range is about 292,000
  years. A character a thousand times faster than a human acts in about a
  third of a millisecond, which is still hundreds of ticks.
- The **timeline** is a priority queue of scheduled wake-ups ordered by
  `(due tick, sequence number)`. Nothing in the simulation runs "every turn";
  everything that happens was scheduled.
- An entity's **action** is a sequence of phases (typically windup, impact,
  recovery) with durations. Phase boundaries are timeline events. The action
  system is specified in [03-time-and-actions.md](03-time-and-actions.md).
- A **reaction window** is a chance to respond to something in progress.
  When one opens, and whether the player is asked or a default is taken, is
  decided by data-defined rules that mods and players can change.
- Every action can resolve from defaults (the character picks technique and
  target) and can also be specified in full by the player.
- A **process** is scheduled activity that belongs to the world rather than to
  an entity: a pressure wave expanding, a tower collapsing, a body in flight,
  a fire spreading. Processes are plain serialisable state plus a `wake`
  function that does some work and says when to wake next.

### 5.2 Clocks

Every scheduled wake-up belongs to a **clock**. A clock has a rate relative to
its parent and the root clock is universal time.

```
universal
 ├─ realm "earth"            rate 1
 │   ├─ area "time-stop"     rate 0      (knives hang in the air)
 │   └─ entity "speedster"   rate 1      (speed comes from short actions, not clock rate)
 └─ realm "training-room"    rate 365    (a year inside per day outside)
```

- Durations are given in local time. The timeline converts to universal ticks
  using the product of rates up the chain.
- Changing a clock's rate, or moving something to a different parent clock,
  reschedules that clock's pending wake-ups. A rate of zero suspends them.
- Realms have a clock. Areas may have one. Entities and processes inherit the
  clock of where they are unless given their own.

This is how scenes S3 and S10 work, and how slow-motion fields, time dilation
and hyperbolic training rooms can be built by mods.

### 5.3 Advancing

```rust
pub enum Advance {
    NeedInput(DecisionRequest),   // a controlled entity must choose
    Reached(Tick),                // the requested limit was reached
    Yielded,                      // work budget spent; call again
    Request(AppRequest),          // a rule asked to save, restore or end (13 §9.1)
}

impl Sim {
    pub fn submit(&mut self, cmd: Command) -> Result<(), CommandError>;
    pub fn advance(&mut self, limit: AdvanceLimit) -> Advance;
    pub fn drain_events(&mut self) -> Vec<SimEvent>;
    pub fn view(&self) -> SimView<'_>;
    pub fn state_hash(&self) -> Hash128;
}
```

There may be one controlled entity, several (a party), or none (a headless
run). `advance` pops timeline events in order and runs them until a
controlled entity has a decision to make (its action finished, or a rule
asked for a reaction) or the limit is reached. The decision request names
which entity it is for. Between any two events is a **safe point**,
where loading, unloading and saving are allowed.

## 6. State

| Owner | State |
|-------|-------|
| `clash-world` | The **cosmos**: realms, grids of cells, fields, areas, sites, portals, subgrids, bodies in flight, tract and region records, tickets. See document 02. |
| `clash-sim` | **Entities** and their components (creatures, items, devices, bodies and wounds, inventories, minds, beliefs, relations), the cold-simulation records (agents, groups, populations), EOC variables, RNG streams. |
| `clash-time` | Timeline, clocks, process table. |
| `clash-data` | Immutable **definitions** loaded from mods, addressed by dense numeric ids. |

Entities are stored in a purpose-built component store (sparse sets keyed by
generational `EntityId`) rather than a third-party ECS. The reasons are
deterministic iteration and serialisation order, uniform reflective access for
the expression language, and a stable API for agents to build on. Document 06
specifies it.

The world does not store entities. It stores an index from cells to opaque
occupant ids, kept consistent by the single relocation function (R3).

Three structural commitments for the entity model, fixed here because other
documents rely on them:

- **Containment.** An entity is either in the world at a location or inside
  another entity: an item in a pack, a rider on a mount, a passenger in a
  seat, a person in a locker, a soul in a jar. Only the outermost entity is
  in the world's index.
- **Mind and body are separate.** What decides (a mind, a player, a command
  from an owner) is attached to what acts (a body) by a relation that
  effects can change. Possession, puppets, raised corpses, body swaps and
  remote control are all rearrangements of that relation.
- **Instances are editable.** A creature's body, a device's wiring and an
  item's properties start from definitions and are then live data. Effects
  can add, remove, reshape and re-material parts at runtime.

## 7. Data and mods

Full specification: [04-data-and-modding.md](04-data-and-modding.md). This
section is a summary; where the two differ, document 04 governs.

- A mod is a directory `mods/<mod_id>/` with `mod.json` (id, name, version,
  dependencies) and any number of `*.json` files under `data/`.
- Each data file is a JSON array of objects. Each object has `"type"` and
  `"id"`. Keys are `snake_case`. A key named `"//"` is a comment and ignored.
- Ids are namespaced: `mod_id:name`. Inside a mod, a bare `name` refers to
  that mod's own namespace when defining. When referencing, it resolves in
  the mod's own namespace first, then in the namespaces the mod imports, by
  default the mods it requires.
- `"copy_from"` inherits from another definition of the same type. Lists and
  sets support `"extend"` and `"delete"`. A later mod may change another
  mod's definition with `"patch": true` or replace it with
  `"override": true`.
- A **profile** is a named list of mods with a boot screen. It is what the
  game starts, and how a total conversion presents itself.
- Physical quantities are numbers in SI units, or strings with a unit suffix
  from a fixed list (`"250 ms"`, `"3 s"`, `"2 h"`, `"70 kPa"`, `"1.5 MJ"`,
  `"80 kg"`). Durations in data are always local time.
- Unknown keys are errors with a "did you mean" suggestion. Every error
  carries file, line, column and JSON path.
- Definitions receive dense numeric ids at load, assigned in sorted order of
  qualified id so that the same mod set always produces the same ids.
- JSON Schemas are generated from the Rust definition types so editors can
  autocomplete and validate mod files.
- Saves refer to definitions by string id through a per-save id table, never
  by runtime numeric id. Removing or renaming a definition is handled by
  migration entries and placeholder definitions, not by corrupting the save.
- Mods can be reloaded while the game runs.

## 8. Events and EOC

Full specification: [05-expressions-and-eoc.md](05-expressions-and-eoc.md).

- Every state change of interest produces a typed **event** with a payload
  (who, what, where, when, cause).
- An **EOC** (effect-on-condition) is a JSON definition: a trigger (an event
  type, a recurrence, or explicit activation), a condition, and a list of
  effects. Conditions and numeric arguments are written in the expression
  language and compiled once at load.
- Expressions and effects address named **subjects** bound by the trigger
  (`self`, `target`, `actor`, `here`, …) and **variables** at scopes:
  entity, cell, area, site, tract, region, realm, global, and local context.
- Effects are engine-provided verbs. The engine's job is to make the verb list
  broad and orthogonal enough that the rules of combat, abilities, social
  behaviour and world reactions in the bundled reference mods are written
  entirely with them. Every part of the simulation should be reachable as a
  hook.
- The reserved script hook is a second provider of conditions and effects
  behind the same interface. Nothing in the engine may require it.

Each subsystem document ends with a **modding surface** section listing the
definitions, events, effects and queries it exposes. A subsystem feature that
cannot be reached from data is unfinished.

## 9. Simulation level of detail

| Level | Space | Time | Entities | World |
|-------|-------|------|----------|-------|
| **Hot** | Loaded cells | Exact timeline events | Individually scheduled, full AI | Full physics |
| **Warm** | Loaded cells | Exact timeline events for physics | Exist, AI throttled | Full physics |
| **Cold** | Tract and region records only | Coarse steps | Important characters as abstract agents; everyone else as statistics | Devastation values, deferred edits |

There is no fixed "reality bubble". Tracts are loaded because something asked
for them through an **interest ticket**: the player, an important fight, a
body in flight, a building that has not finished collapsing. Document 02
specifies tickets. Document 09 specifies the cold simulation and how
characters are promoted to and demoted from individual simulation.

## 10. Presentation boundary

Full specification: document 12.

- The scene layer consumes `SimEvent`s, which carry simulation timestamps,
  and plays them back over real time. Slow motion, zoom, screen shake and
  glyphs sliding between cells are presentation choices made from event data.
- A **cutscene** is a data-defined choreography selected by matching event
  patterns ("a clash between two named characters with energy above X"). It
  may restage how already-decided events are shown. Sequences that need
  simulated coordination, such as a paired takedown, are **paired actions** in
  the simulation (document 07), and the cutscene presents them.
- Frontends receive a grid of glyph cells with optional sub-cell offsets and
  effects. The terminal frontend ignores what it cannot draw.
- **All UI is data.** Screens, menus and HUD elements are definitions built
  from a glyph toolkit (panels, lists, tables, bars, text, maps, input
  bindings) whose contents are expressions and whose actions are commands
  and EOC effects. The engine's own screens, including character creation
  and the main menu, are built from the same toolkit and ship as a mod, so a
  total conversion can replace any of them.
- The simulation runs headless. Tools and tests use it without any frontend.

## 11. Saving

- A save contains: the mod list with versions and a content fingerprint, the
  id table, the cosmos (per-realm, per-tract), entities, timeline, clocks,
  processes, variables and RNG streams.
- Saving happens at safe points and is atomic. World data is stored per tract
  so the cost of a save is proportional to what changed.
- The format is versioned. Each version bump ships a migration.

### 11.1 The binary encoding

State is written to disk in the engine's own binary encoding,
`clash_core::codec`. JSON remains the format of mods and of anything a
person reads. Every state type implements serde, and the codec is a serde
format, so one implementation serves both.

The encoding does not describe itself. The bytes hold values only, and the
type being read says what they mean:

| Value | Bytes |
|-------|-------|
| `bool` | One byte, `0` or `1` |
| `u8`, `i8` | One byte |
| `u16` to `u128`, lengths, `char` | LEB128: seven bits a byte, lowest first, the top bit set on every byte but the last, in the fewest bytes |
| `i16` to `i128` | Zig-zag (`0, -1, 1, -2` become `0, 1, 2, 3`), then LEB128 |
| `f32`, `f64`, so `Real` | The IEEE bits, little-endian: exact |
| Text, bytes | The length in bytes, then the bytes |
| `Option` | A byte `0` for none; a byte `1` and the value for some |
| Unit | Nothing |
| Struct, tuple, array | Each field in order, with no names and no count |
| Sequence, set | The number of items, then each item |
| Map | The number of entries, then each key and value |
| Enum | The variant's index as LEB128, then the variant's fields |

A value has exactly one encoding, so equal values give equal bytes on every
platform.

Decoding never panics, never reserves more than a small multiple of its
input and never does more work than its input is long (R6). Every length
is checked against the bytes that remain, every item of a sequence or map
must take at least one byte, nesting is limited to 128 levels, and bytes
left over are an error. What cannot be read back safely is refused when
writing, with a reason:

- a sequence or map that does not state its length first;
- a sequence or map with an item that takes no bytes, such as a list of
  units;
- a struct that leaves a field out (`skip_serializing_if`);
- a value nested deeper than the limit.

Anything that reads through serde's self-describing calls is refused when
reading: `flatten`, untagged and internally tagged enums. State types do
not use them.

Reordering, adding or removing a field or a variant changes the bytes.
That is what a blob's version is for.

**Blobs.** Whatever is stored (a tract, a region record, the main save's
sections) is framed as a blob:

| Offset | Size | Holds |
|--------|------|-------|
| 0 | 4 | The magic, `C2DB` |
| 4 | 4 | The kind: four bytes chosen by the owner of the data |
| 8 | 4 | The version of the body's layout, `u32` little-endian |
| 12 | 4 | The length of the body before compression, `u32` little-endian |
| 16 | 16 | The checksum, `u128` little-endian |
| 32 | … | The body, compressed as one LZ4 block |

The checksum is the state hasher of section 4 (MurmurHash3 `x64_128`,
seed 0) over the kind, the version as four little-endian bytes and the
uncompressed body. It catches damage and does not resist tampering. A
blob that is too short, has the wrong magic, the wrong kind, a body of
the wrong length, a body that does not decompress or a checksum that does
not match is refused, each with its own error. A reader that supports
several versions opens the frame first, then reads the body by the
version it found.

The compressed bytes are not a contract: a later compressor may pack the
same body differently. Bodies are compared and hashed, never blobs.

### 11.2 The save file

A game is saved into one file, a `redb` database (the world's
`RedbStore`, 02 §11.5) with the extension `.c2save`, written in one
transaction: a file holds all of one save or all of the one before it.
`clash-save` writes and reads it (D-580). It holds:

| Part | Blob kind | Holds |
|------|-----------|-------|
| Tract blobs, keyed by realm and tract | `TRCT` | The cells of every tract that has been loaded and changed, loaded now or not (02 §11.1) |
| `world.*` sections | the world's | Realms and region records, areas, sites, tickets, stability and activity, the loaded tracts, the save's tables of ids (02 §11.4) |
| `sim.state` | the kernel's | The simulation's own state: timeline and clocks, entities and components, rule state, settings, random streams, queues, the entities shelved with unloaded tracts, and the input log since it was last taken |
| `save.tracts` | `STRC` | The realm, column and row of every tract the save holds a blob for |
| `save.meta` | `SMET` | The metadata below |

The metadata is what a load screen lists and what a loader compares with
the definitions it has. It is read without loading the game.

| Field | Holds | Filled in by |
|-------|-------|--------------|
| `format` | The save format's version, 1 | Saving |
| `name`, `summary` | What a load screen shows | The saver |
| `profile` | The id of the profile the definitions were loaded through, or empty | Saving |
| `mods` | Each mod's id and version, in load order | Saving |
| `fingerprint` | The definitions' content fingerprint (04 §9) | Saving |
| `seed` | The seed the simulation was started with | The saver |
| `tick` | The universal time of the save | Saving |
| `experimental` | The experimental features the mods opted in to (04 §8.4) | Saving |
| `state_hash` | The simulation's state hash when saved; `save verify` compares it with what loads | Saving |
| `app` | Bytes of the saver's own, neither read nor hashed: the console keeps its cursor, active character, selection, frame size, profile name and seed there; the game keeps the start it began from, the character's name and who the character is (12 §3.6) | The saver |

Saving follows 02 §11.4: the blobs of tracts that are not loaded are
carried over from the simulation's own store, `Cosmos::save_world` writes
the world and the loaded tracts that changed, the simulation's blob and
the two save sections go in, and one commit makes it durable. Loading
reads the world first, with placeholders for definitions the mods no
longer supply, then the simulation over it from the same save, gives it
the generators and a store in memory holding the save's tract blobs, and
places every entity again. A save made with other mods loads with
warnings that say what differs; a damaged file is an error that names
the section or tract, never a panic. Saving and loading change nothing
that is hashed (R7).

## 12. Testing and performance gates

| Kind | Purpose |
|------|---------|
| Unit and property tests | Local correctness; coordinate maths, storage and codecs are property-tested. |
| Golden-hash scenarios | Scripted situations run headless; the final state hash must match. Catches accidental behaviour change and nondeterminism. |
| Save-transparency tests | R7, applied to every scenario at several split points. |
| Double-run tests | Each scenario runs twice in one process and once with a different thread count; hashes must agree. |
| Benchmarks with budgets | Each subsystem document states budgets. Benchmarks fail CI when a budget is exceeded by a set margin. |

## 13. Dependencies

Provisional choices. Adding a dependency requires an entry in
[decisions.md](decisions.md).

| Need | Crate |
|------|-------|
| Serialisation | `serde`, `serde_json` |
| JSON Schema generation | `schemars` |
| File watching for hot reload (tools only, never in simulation crates) | `notify` |
| Deterministic maths | `libm` |
| Small collections, flags | `smallvec`, `bitflags`, `indexmap`, `rustc-hash` |
| Errors | `thiserror` |
| Compression | `lz4_flex` (pure Rust, with its default features: the block codec then contains no `unsafe`) |
| Save storage | `redb` behind a `TractStore` trait |
| Parallel generation | `rayon` |
| Tests and benches | `proptest`, `criterion` |
| Terminal | `crossterm` |
| Window | `winit`, `softbuffer`, `fontdue` (frontend crate only; decision D-830); `wgpu` only if drawing ever moves to the graphics card |
