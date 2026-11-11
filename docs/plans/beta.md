# Plan — The beta

The owner's instruction: build until there is a real beta. Not perfect and
not everything, but a playable, moddable game that is fun and has no
glaring holes. This plan says what that means, what is left for later, and
the order of work. It overrides the wave order of the other plans; their
package texts and "Done when" lists still apply to whatever is built.

## What the beta is

A terminal game (and a window frontend if time allows) in which a player:

- starts a game from a title screen, choosing a scenario and making a
  character;
- walks a generated town with buildings of several storeys, all of it
  destructible by the rules of document 02: damage, collapse, fire, blasts,
  bodies thrown through walls;
- meets people who perceive, remember, talk, flee, fight and work, by the
  rules of documents 15 and 09 (hot and warm detail);
- fights with weapons, fists, guns and powers, with bodies made of parts
  and no hit points (06, 07, 08);
- uses doors, devices, power, crafting and building (16);
- saves and loads; and
- can change all of it with JSON mods and rules (04, 05), with a linter,
  a console and test scripts (14).

## Left for after the beta

Vehicles and travel (11) beyond what is stubbed. The cold simulation,
incidents, the director and cases (09 §10 to §13, 13 §7, §8). Charts and
other realms. Substances, structures, accounts and trade screens unless
time allows. Experimental features. Benchmarks beyond a smoke test. The
window frontend is built last and only as a software renderer.

Each of these keeps its typed stub and a note where the beta stops.

## Working rules for agents

- Read `CLAUDE.md`, `docs/01-architecture.md` sections 2 to 4, this file,
  the sections of the specification named in your brief, and your packages
  in the plan. Search `docs/decisions.md` for your area; you need not read
  it whole.
- Work only in your own worktree and on what your brief assigns. Other
  crates are read-only unless the brief says otherwise. If you need a
  change elsewhere, make the smallest one and say so in your report.
- `cargo xtask check` must pass before you finish. Use
  `CARGO_BUILD_JOBS=2`; other agents share the machine.
- Commit in your worktree with a clear message ending with these two
  lines, and do not push:

  ```
  Co-Authored-By: Claude <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Wp6MeAyX2dfpnAu8cXReWc
  ```

- Never put a model name or version in code, comments or commit messages.
- Set your packages' status in their plan's table: `done`, or `beta` when
  the beta subset is built and the rest is stubbed, with a line under the
  package saying what is stubbed.
- Prefer a working subset with tests over a complete design without them.
  If the specification cannot be built as written, fix the document in
  the same commit and add a decision.
- Finish with a short report: what was built, what is stubbed, the public
  API others will call (names and signatures, briefly), test results, and
  anything the next wave must know.

## Waves

| Wave | Work | Status |
|------|------|--------|
| B0 | Foundation: F1, F2, F3 | done |
| B1 | Data v0 and diagnostics (F4, P1); expressions (X1 to X3); world storage (W1 to W5); frames, themes, terminal frontend (U1, U2); binary codec; noise | done |
| B2 | Edits, occupancy, queries (W6, W7, W18); data merging, profiles, typed loading (P2 to P4); expression fields (X4); toolkit layout and widgets (U6a); loader integration; console and test scripts (TL1, TL6 first form) | done |
| B3 | Kernel (K1 to K3, T1); tracts and tickets (W8, W9); damage and collapse (W12, W13); view model and world drawing (U4); map format (WG1); action definitions (T2); component store (E1); light (S1) | done |
| B4 | Kernel wiring and subjects (K4); rule runtime (K5 to K9); persistence (W10); bodies and releases (W14, W15); fields (W16); placing maps, ground, tracts (WG2 to WG4); attributes and traits (E2, E3); actions and control (T3, T4); movement, application: the first walk (E10, U11); toolkit binding and input (U6b, U7, U8); world viewer (W11); settings (P6) | in progress |
| B5 | Bodies, hits, minds, items, templates (E4 to E9); perception (S1 to S5); windows and joint actions (T5, T6); playback and bundled screens (U5, U8, U9); regions, settlements, sites (WG6 to WG9) | todo |
| B6 | Combat (C1 to C11); AI (I1 to I7); interactions, devices, resources, recipes (G1 to G3, G6); personas, relations, speech (S6 to S8) | todo |
| B7 | Abilities (A1 to A9); conduits, signals, construction, assemblies (G4, G5, G8, G9); scenarios, objectives, saving (SC1 to SC3, SC7); console and test scripts (TL1, TL6); the `core` mod | todo |
| B8 | Content and integration: a reference world with people, weapons, powers and scenarios; screens for making and building; scripted play sessions; fixing what they find | todo |
| B9 | Polish: window frontend, generated reference for modders, README and quick start, release notes | todo |

The wave list is a plan, not a promise. It is revised here as work lands.

## Log

Newest first. One line per merge: wave, what landed, what is stubbed.

- B4 (part): bodies in flight (W14, beta) and energy releases (W15, beta): `WorldCtx::launch` with entity, debris and projectile owners, ballistic flight with drag and drift, impacts through the damage rule, occupants struck and carried, driven bodies with steering, bodies leaving loaded space finished by arithmetic; `release` of a sphere, cone or beam with shielding, venting, confinement in closed rooms, blast push; joined to persistence (save ids for damage types and planes) and to fields (`conduct`); every body, release and field event mirrored for rules. Stubbed: portals, resolve mode, stamps for unloaded cells, fronts (W24).
- B4 (part): creature bodies (E4, beta) and hits and wounds (E5, beta): `body_part`, `body_plan`, `body_function`; the `Body` component with layers, sockets, scale, functions and channel slots; `apply_hit` through the layers by the world's damage rule, organs, severing, exits, wounds that leak into pools and heal, spread hits, conversion of world exposure and impacts into hits. Waits for the kernel to raise its events and call it.
- B4 (part): the rule runtime (K5, beta): the `eoc`, `rule_group` and `reason` types, the `Effects` field type used by actions, 26 core effects, the interpreter with every limit, rule state (attachments, `once`, `cooldown`, groups), delivery order, rules callable as verbs, against a `RuleHost` trait with a fake. Stubbed: binding to `Sim`, timers (K7), queries (K6), world verbs (K8), `message` and `ask` (K9).
- B4 (part): input (U7, beta): the `control` type, keymap contexts and rebinding, `act` with `needed` and `all` assembling a complete command from an action's parameters, pickers for entity, cell, direction, number, text, list and multi, mouse, decision prompts, `ui:play`. Stubbed: several characters at once, the route picker.
- B4 (part): kernel wiring and subjects (K4, beta): `Sim` holds the components, runs the world's wake-ups (collapse under `advance`), owns generators and a store and reconciles activity at every safe point, shelves and restores entities as tracts unload and load, `start_session`; 8 subject kinds, 103 properties, 9 functions, variables on every scope with `watch`, `Sim::host`, `Sim::assign`, `engine_registry()`; `lint` loads with the real registry.
- B4 (part): fields (W16, beta): the `field` and `field_reaction` types, per-chunk storage, the stepper with decay, diffuse, flow, spread, emit, convert, damage, drift and expose, `OccupantDosed`, catch-up on load; fire, smoke, steam and water in `mods/core`. Stubbed: sight blocking in the queries; about seven times over the step budget at 23,000 active cells.
- B4 (part): placing maps, ground and tracts (WG2 to WG4, beta): `apply_map` with three modes, pending maps for unloaded tracts, the `ground` and `tract` types, the `standard` generator, `core:countryside` with houses on a street; `cargo run -p clash-gen --example tract_ascii -- countryside 1 0 1`.
- B4 (part): persistence (W10, beta): tract blobs by save id with per-type id tables, placeholders that keep the original id when a mod is removed and restore the content when it returns, the pristine policy with a generator fingerprint, `RedbStore` on disk beside `MemoryStore`, `Cosmos::save_world` and `load_world` with named world sections, an opaque entity section per tract. Stubbed: migration entries (P7); region records are not separate blobs. Debug information is now off in dev and test builds, after the disk filled.
- B4 (part): UI bindings and effects (U6b beta, U8 done): bound widget properties compiled at load with `view`, `party`, `app`, `screen.name`, `$name` and `it`; ordinary screens are mind contexts and `debug` screens read truth; typed UI effects (`open`, `close`, `replace`, `set`, `focus`, `confirm`, `pick`, `answer`, `act`, `command`, `run`, `app`); a real screen stack with popups; preferences; `ui_part` slots; a second theme; the `SceneHost` and `AppState` seams with fakes. Stubbed: the `Ui` command and replay (kernel), `show_screen` from rules, pickers other than the list (U7).
- B4 (part): attributes, pools and tracks (E2, beta): the three definition types, modifiers with sources, derived attributes with dependency order, pools with rates and scheduled threshold crossings, tracks with level curves, the `Stats` component, properties and events registered for expressions. `self.pool.frac('blood')` replaces `self.pool.blood.frac` (D-440). Waits for the kernel to raise its events and forward its properties.
- B3 (part): light (S1, beta): `Cosmos::light` and the batch `fill_light`, sky light by time of day spreading through openings, fixed sources from element definitions, moving sources at query time, a per-chunk cache invalidated by edits; the `sky` definition type and `light` on elements; a lamp in `mods/core`. Stubbed: fields and carried lamps are handed in by the simulation later; no coloured light.
- B3 (part): damage and destruction (W12, beta): `DamagePacket`, the rule of 02 §12.2, `WorldEdit::damage`, `on_destroy`, rubble and heaps, `ElementDestroyed`. Structural support and collapse (W13, beta): stability checks with carriers and loads, crushing by bearing, falling masses as processes on the timeline, `CollapseStarted` and `CollapseSettled`, `OccupantUnsupported` and `OccupantExposed`, `WorldCtx::handle_wake` as the world's one wake entry. A ten-storey building is checked in about 1 ms and brought down in about 22 ms. Stubbed: debris bodies (W14), tickets held by a collapse, resolve mode.
- B3 (part): the map format (WG1, beta): the `map` and `palette` types, cell specs, presets, weighted alternatives, parameters and positional randomness, turning and mirroring with facing states, nested maps, areas and marks; `MapDefs::from_defs` and `resolve` to a `ResolvedMap`; three bundled maps and furnishing in `mods/core`; `cargo run -p clash-gen --example map_ascii -- house`. One function now registers every engine type (D-450). Stubbed: placing a resolved map into the world (WG2); spawns, wiring and the rest are carried as data.
- B3 (part): tracts and loading (W8, beta): region and tract records, the generation traits in `clash_world::generate`, generators `flat` and `test_city` in `clash-gen` chosen by a realm's `generator`, parallel realisation with identical hashes, `load_tracts` and `unload_tract` against a `TractStore` with a `MemoryStore`, a first column blob. Tickets and activity (W9, beta): `hold_ticket`, anchors, hysteresis, budget and refusal, `WorldCtx::reconcile_activity`. `cargo run -p clash-gen --example city_ascii -- 1 0` prints the city. Stubbed: stamps and ruin, save-id table and the on-disk store (W10), the kernel does not call reconcile yet.
- B3 (part): the simulation kernel (K1 done; K2, T1, K3 beta): `Sim` with entities, `Place` and containment, `relocate` as the one way a place changes, `advance` with its three limits, wake-domain handlers, `submit` with `Reason`s, the input log and replay, decisions, clocks from areas with personal clocks and anchors, the event bus with hooks, cancellation and the cascade limit, the `event` definition type, state hash, save and load with `reindex`. Stubbed: the action table, window table, rule state and plan are typed placeholders; `reconcile_activity` and the world's wake handler are marked no-ops; components are not yet held by `Sim`.
- B3 (part): action definitions (T2, beta): the `channel`, `action` and `window_kind` types with parameters, list parameters, roles, phases, transitions, windows and their expression scopes; `ChannelSet` and the dense ids; 23 documented refusal reasons; `clash_sim::register_defs`. Stubbed: effect lists are carried as written until the rule runtime (K5); nothing runs yet (T3).
- B3 (part): the view model and world drawing (U4, beta): `ViewModel` as plain data fed by observe calls or, for debugging, an omniscient fill; cell composition with look-down, light, memory and ambience; `map_view` and `section_view` through a `WorldView` painter with a camera and zoom; the `ambience` and `overlay` definition types; `cargo run -p clash-front-term --example worldview`. Stubbed: region map, chart view, subgrids, portals, particles, multi-cell shapes.
- B3 (part): the component store (E1, beta): `ComponentStore<C>` in saved dense order, the `Components` aggregate, `Identity`, `Physical` with cached mass over containment. Waits for the kernel to hold it and call it.
- B2 (part): the console language, command table (one command per registered verb), the `ConsoleHost` trait, sessions, the `.scn` test-script format and the four-mode runner with `--bless`, all in `clash_sim::console` against a fake host (TL1, TL6, beta); `clash-tools scenario run`. Waits for the simulation to implement `ConsoleHost`.
- B2 (part): expressions as definition fields (X4, beta): `Expr<T>`, `Cond`, `Text` compiled at load with a per-field `Scope`, the `function` and `variable` definition types, value sources, `Runner` for evaluation; `clash-data` now depends on `clash-expr` (D-330). Stubbed: queries, `exists` and `run` sources (K6), `Effects` (K5).
- B2 (part): the one mutation path (W6, beta): `WorldCtx`, `WorldEdit` with element, damage, decal and cell-variable edits, `Cause`, `WorldEvent`, `EventSink`, queued stability requests. Occupancy (W7, beta): occupants with footprints and planes, `place`, `relocate`, `remove`, occupant queries, areas with kinds, clocks, ambient overrides and drift, `enclose` with breach marking, sites; the `plane` and `area_kind` definition types, registered with the loader at merge. Stubbed: damage packets, fields, portal crossings on relocate, occupants are not saved.
- B2 (part): terrain queries in `clash_world::query` (W18, beta): raycast with element heights and coverage, line of sight, a symmetric field of view across storeys, pathing neighbours, footing, and a deterministic `find_path` (D-315), with caches keyed on chunk revisions. Stubbed: occupants along rays wait for W7 (a trait is ready); subgrids, portals and fields that hide sight.
- B2 (part): the loader is the one way in: the world's and the interface's definition types are registered (`register_defs`), `WorldDefs::from_defs` and `UiDefs::from_defs` build from loaded definitions, content lives under `mods/*/data/`, `profiles/default.json` loads `core` and `ui`, and `lint mods` passes. Stubbed: world references are checked in a build step after loading, not as `Ref<T>`.
- B1 (part): data loading in `clash-data`: mod discovery, profiles, the span-keeping JSON parser, registries with dense ids, inheritance and the complete merge semantics (F4 beta, P2, P3 beta), diagnostics with codes and carets (P1), typed loading with quantities, references, inline definitions, validation stages and the fingerprint (P4, beta); `clash-tools lint`, `explain`, `show`. Stubbed: schemas, text templates, settings, migration, hot reload. The definition types of the world and the scene are not registered yet, so the repository lint test is ignored until they are.
- B1 (part): the expression language in `clash-expr`: lexer, parser, printer, unit literals and text templates (X1); the registry, contexts including the mind context, name resolution and type checking (X2); constant folding, bytecode and the stack machine with null semantics and rate-limited warnings (X3, beta). Stubbed: the reader helpers and `exists` are signatures the host answers; no reactive-context check; no benchmark.
- B1 (part): world storage in `clash-world`: coordinates and topology (W1), paletted storage (W2), cell definitions and `WorldDefs::build` (W3, beta), chunks, columns and grids (W4, beta), cosmos, realms and the read API (W5, beta); the size-literal step of `cargo xtask check`. Stubbed: definitions are not yet fed by the loader; fields and occupants are placeholders; no chunk serialisation.
- B1 (part): frames, colours, themes, glyphs, styled text, input events, the `Frontend` trait, golden frames and the scene seam (U1, beta); the terminal frontend (U2); the toolkit's layout and every widget with static content (U6a, beta); `mods/ui` with a default theme and a widget gallery (`cargo run -p clash-front-term --example gallery`). Stubbed: bindings and effects (U6b), views, log, slots, popups over a play screen.
- B1 (part): the binary codec and blob frame in `clash_core::codec` (decision D-251) and lattice noise with field shapes in `clash_gen::noise` (D-252). No benchmarks.
- B0: the workspace with every crate of 01 §3, the lints and `cargo xtask check` (F1); `clash-core` (F2); `clash-time` with the timeline, clocks and process table (F3). Stubbed: every other crate is a doc comment and its dependency edges; the size-literal check waits for W1; no benchmarks.

