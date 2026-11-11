# Decisions

A decision recorded here is settled. To change one, add a new entry that
supersedes it and update the affected documents in the same change.

"Owner" means the project owner chose it. "Design" means it was chosen during
design and can be challenged with evidence.

| # | Decision | By | Why | Consequence |
|---|----------|----|-----|-------------|
| D-001 | The engine is written in Rust. | Owner | Performance, control over memory layout, and a compiler that catches most cross-module mistakes in a large codebase written by many agents. | Slower builds; no garbage collector to lean on. |
| D-002 | The primary frontend is a glyph grid in its own window. A terminal frontend exists with reduced effects. | Owner | Cutscenes need shake, zoom, slow motion and sub-cell motion. | The simulation outputs an abstract glyph scene; frontends are thin. |
| D-003 | Time is one shared timeline. Actions have phases with durations. The game pauses for decisions. | Owner | Speed, reactions, interrupts and clashes come from one mechanism. | No "turns" anywhere in engine code. |
| D-004 | The power ceiling is planet-buster. Nearby destruction is per cell; distant destruction is coarse and realised on arrival. | Owner | The fiction demands it, and the coarse layer is needed for off-screen events anyway. | Stamps and devastation (02 §17). Magnitudes span about 33 orders. |
| D-005 | The world away from the player runs as a layered, abstract simulation. | Owner | A world that moves without the player, at bounded cost. | Hot, warm and cold levels; promotion and demotion of characters. |
| D-006 | One z-level is one storey of 3 m. Moving things also carry an exact height. | Owner | Maps stay authorable and readable; motion stays smooth. | Cells are 1 × 1 × 3 m. Collapse and craters are storey-grained. |
| D-007 | Bodies are anatomical and data-defined. | Owner | Injuries, severing and executions are simulated outcomes. | Tissue shares the material table and damage rule with terrain. |
| D-008 | Modding is JSON and EOC first. A scripting hook is reserved; nothing depends on it. | Owner | Deterministic, safe, reloadable, and familiar from CDDA. | The EOC verb list must be broad enough to hold all of the bundled content's rules. |
| D-009 | The simulation is deterministic. | Design | Replays, golden tests, reproducible bugs, cutscenes that cannot disagree with outcomes, a path to lockstep co-op. | The rules in 01 §4, enforced by lints and tests. |
| D-010 | A tick is one microsecond, stored as `i64`. | Design | Fine enough for characters thousands of times faster than human; range of 292,000 years. | Durations in data are converted at load. |
| D-011 | Scheduled work belongs to clocks arranged in a tree, each with a rate. | Design | Time stop, slow fields and time-dilated realms become one feature. | The timeline must reschedule on rate change (F3). |
| D-012 | Space and time are integers. Physical magnitudes are `Real`, a deterministic `f64`, in SI units. | Design | Exact where exactness matters; wide range where it is needed; numbers a modder can look up. | No std transcendental functions; `libm` only. |
| D-013 | Chunk 32 × 32 × 8 cells; tract 32 × 32 column; region 64 × 64 tracts. | Design | Powers of two; tract footprint equals chunk footprint; a cell index fits in 13 bits. | Constants live in `dims` and nowhere else. |
| D-014 | A cell has three element layers (floor, block, fixture). An element's state is definition + material + bits, stored in per-chunk palettes. | Design | Independent destruction of floor and wall; any shape in any material; compact storage. | Natural ground has no floor layer; standing uses the solid top below. |
| D-015 | One material table serves terrain, items and bodies. | Design | One damage rule everywhere. | Material definitions carry properties for all three uses. |
| D-016 | Cells change only through `WorldEdit`, which records cause and emits events. | Design | Caches, saves, events and attribution cannot drift. | Chunk mutators are crate-private. |
| D-017 | `clash-world` never calls into the simulation or EOC. It emits events. | Design | The world can be built, tested and reasoned about alone. | No veto hooks from data in the world; use flags and definitions. |
| D-018 | Tracts are loaded on demand by interest tickets. There is no fixed bubble. | Design | Fights range over kilometres; processes must be able to finish. | A loaded-tract budget with force-resolve as the pressure valve. |
| D-019 | Changed tracts are stored in full. Unchanged tracts are regenerated. | Design | Robust to generator changes where it matters; small saves. | Pristine tracts follow mod updates. Configurable. |
| D-020 | Damage is a packet of energy and intensity against a threshold and a capacity. | Design | Distinguishes many weak hits from one strong one; conserves energy through obstacles; scales without special cases. | Callers must supply intensity. |
| D-021 | Blast intensity is released energy over swept volume, `E / (4/3·π·r³)`. | Design | Matches cube-root scaling of real explosions from grenades to impacts with one formula. | Thresholds for pressure waves are in pascals. |
| D-022 | Structural support is span propagation per storey, bottom up. No stress analysis. | Design | Cheap, deterministic, good enough to make collapse emergent. | No load limits initially (02 §26). |
| D-023 | Bodies in flight are point masses with a cross-section. No rotation. | Design | Legible, fast, deterministic. | Tumbling is presentation only. |
| D-024 | A stamp is stored once at the coarsest level containing its footprint and applied lazily at load. | Design | Planet-scale events cost one record. | Tracts track a stamp watermark. |
| D-025 | Subgrids are the single abstraction for moving pieces of world. | Design | Vehicles, ships, elevators and torn-off buildings share one mechanism. | Every position is a `Location` with a `GridId` from the start. |
| D-026 | Ambient conditions vary by altitude within a realm. | Design | Ground to orbit without changing realm. | Zero gravity in orbit is by declaration, not orbital mechanics. |
| D-027 | Planes are interaction masks on occupants. | Design | Astral and mirror layers without duplicating space. | Terrain exists on all planes. |
| D-028 | All bundled content is mods. Engine code names no content. | Design | Everything is replaceable; the modding surface is proven by use. | Fills, rubble and placeholders are named in data. |
| D-029 | Definition ids are namespaced `mod:name`. | Design | Mods cannot collide by accident; overrides are explicit. | Reference resolution rules in 01 §7. |
| D-030 | Entities use a purpose-built component store, not a third-party ECS. | Design, provisional | Deterministic order, controlled serialisation, reflective access, API stability. | Revisit in document 06 with a prototype. |
| D-031 | Tract storage is behind a trait; the default backend is `redb`. | Design, provisional | Atomic, incremental saves without designing a file format. | Replaceable if measurements say so. |
| D-032 | Documents are the source of truth. Code that must differ changes the document in the same change set. | Design | Many agents, no shared memory. | Reviews check documents and code together. |
| D-033 | CLASH2D is a genre-neutral engine kit: an ASCII immersive sim and superpower sandbox. Bundled content is small reference mods, not a flagship game. | Owner | The engine provides tools; creators provide results. | Rule R10. Supersedes the "first content mod is a modern city" note. Reference mods span several genres. |
| D-034 | The engine enforces no fairness. Nothing is scaled, softened or levelled. Authors balance their own content. | Owner | Ordinary people should be ordinary, and the gap to everything else should be felt. | No difficulty scaling, level scaling or damage caps in engine code. |
| D-035 | When a reaction window opens, and whether the player is asked, is decided by data-defined rules. | Owner | Different content wants different rhythms. | Document 03 defines the rule hooks and player-adjustable defaults. |
| D-036 | Zero, one or several controlled characters. Party control is optional and enabled per scenario. | Owner | Party play should be possible without being imposed. | `advance` returns decision requests that name the entity. |
| D-037 | Every action resolves from defaults and can also be fully specified. | Owner | Quick play and granular play are both wanted. | Document 07 must define defaults for every choice it exposes. |
| D-038 | Progression schemes are data. The engine supplies attributes, pools, counters and events. | Owner | Use-based skills, training, point-buy, levels and cheat skills should all be buildable and mixable. | No built-in experience or level system. |
| D-039 | All UI is built from a data-defined glyph toolkit, including the engine's own screens and main menu. | Owner | Megamods and total conversions need their own menus and screens. | Document 12 specifies the toolkit; bundled screens ship as a mod. |
| D-040 | Hitting many weak foes at once is an ability defined in data, not an engine shortcut. | Owner | Keeps one ruleset. | No "mook" concept in the engine. |
| D-041 | The world records truth; characters act on beliefs formed by perception. | Design | Stealth, disguise, deception, framing and manipulation need the two to differ. | Rule R11. AI may not read ground truth it has not perceived. |
| D-042 | Minds and bodies are separate and joined by a relation. Entities can contain entities. Instances are editable at runtime. | Design | Possession, raised corpses, soul transfer, mounts, hiding places and body-reshaping abilities become rearrangements of data. | Fixed in 01 §6 for document 06. |
| D-043 | Interactive props are ordinary elements that promote to device entities on demand. | Design | A world full of usable things must cost little until used. | 02 §5.5; document 16. |
| D-044 | World size and shape are per-realm data. There is no default planet. | Design | The owner had no preference, and the engine should not have one either. | Reference mods use small realms. |
