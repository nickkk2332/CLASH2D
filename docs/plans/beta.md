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
| B3 | Kernel (K1 to K4, T1); tracts, tickets, persistence (W8 to W10); damage and collapse (W12, W13); bodies and releases (W14, W15); fields (W16); view model and world drawing (U4); map format and placing (WG1, WG2); ground and tracts (WG3, WG4); world viewer (W11); settings (P6) | in progress |
| B4 | Rules runtime (K5 to K9); actions and control (T1 to T4); components, attributes, traits (E1 to E3); movement, view model, application: the first walk (E10, U4, U11); toolkit binding and input (U6b, U7) | todo |
| B5 | Bodies, hits, minds, items, templates (E4 to E9); perception (S1 to S5); windows and joint actions (T5, T6); playback and bundled screens (U5, U8, U9); regions, settlements, sites (WG6 to WG9) | todo |
| B6 | Combat (C1 to C11); AI (I1 to I7); interactions, devices, resources, recipes (G1 to G3, G6); personas, relations, speech (S6 to S8) | todo |
| B7 | Abilities (A1 to A9); conduits, signals, construction, assemblies (G4, G5, G8, G9); scenarios, objectives, saving (SC1 to SC3, SC7); console and test scripts (TL1, TL6); the `core` mod | todo |
| B8 | Content and integration: a reference world with people, weapons, powers and scenarios; screens for making and building; scripted play sessions; fixing what they find | todo |
| B9 | Polish: window frontend, generated reference for modders, README and quick start, release notes | todo |

The wave list is a plan, not a promise. It is revised here as work lands.

## Log

Newest first. One line per merge: wave, what landed, what is stubbed.

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

