# Roadmap

What has been designed, what is built, and what comes next. Every
subsystem has a specification and an implementation plan under
`docs/plans/`. The design is complete and the beta is built:
[plans/beta.md](plans/beta.md) records what landed, in order, and what
each part left stubbed, and the [release notes](release-notes-beta.md)
say what a player meets.

Document numbers are stable. Other documents refer to them by number.

## Documents

Every specification is written. "Built" says how much of it the beta
has; each plan's status table says it package by package (`done`, `beta`
for a working subset with the rest stubbed, `todo`).

| # | Document | Built in the beta |
|---|----------|-------------------|
| 00 | [Vision](00-vision.md) | The pillars hold; the scenes of force, fragility and guile can be played in small, the scenes of vehicles, travel and the off-screen world cannot yet |
| 01 | [Architecture](01-architecture.md) | The crates, the hard rules, determinism and the gate |
| 02 | [World model](02-world-model.md) | Storage, edits, damage, collapse, bodies in flight, blasts, fields, loading and saving. Not built: the coarse layer, portals, subgrids, other realms |
| 03 | [Time, clocks and actions](03-time-and-actions.md) | The timeline, clocks per area, actions, reactions, joint actions. Not built: lower detail for unloaded actors |
| 04 | [Data and modding pipeline](04-data-and-modding.md) | Loading, merging, profiles, diagnostics, settings, schemas. Not built: hot reload, migration |
| 05 | [Expressions and EOC](05-expressions-and-eoc.md) | Expressions, rules, events, hooks, queries, timers |
| 06 | [Entities, bodies, species and items](06-entities-and-bodies.md) | Bodies, wounds, items, traits, reshaping, templates, creation. Not built: coming and going between places |
| 07 | [Combat, techniques and clashes](07-combat.md) | Strikes, defences, push, grips, throws, takedowns, ranged weapons, stances. Not built: clashes |
| 08 | [Abilities and powers](08-abilities-and-powers.md) | Abilities, costs, modes, suppression, areas, bindings, twelve powers. Not built: snapshots, relays, the reference abilities |
| 09 | [AI and the layered simulation](09-ai-and-simulation.md) | Minds, goals, methods, navigation over belief. Not built: tactics, work, collectives, the warm and cold simulation, incidents, weather |
| 10 | [World generation and the map format](10-world-generation.md) | Maps, ground, tracts, settlements, furnishing. Not built: realm plans, ruin, reference worlds |
| 11 | [Vehicles, mounts, the sea, space and travel](11-vehicles-and-travel.md) | Not built |
| 12 | [Presentation and the UI toolkit](12-presentation-and-ui.md) | The application in a terminal or a window, playback, input, the toolkit and the bundled screens. Not built: cutscenes |
| 13 | [Scenarios, modes and events](13-scenarios-and-events.md) | Game scenarios, objectives, missions, saving policies, endings. Not built: plots, the director, cases, campaigns |
| 14 | [Tooling](14-tooling.md) | Console, test scripts, world viewer, save tools, benchmarks, the generated reference and mod tools. Not built: the inspector, traces, the map editor |
| 15 | [Perception, belief and social behaviour](15-perception-and-social.md) | Light, senses, detection, beliefs, personas, relations, groups, orders, speech. Not built: coloured light |
| 16 | [Interaction, devices, machines and building](16-devices-and-machines.md) | Interactions, devices, recipes, know-how, substances. Not built: resource networks, construction, gadgets, suits, accounts and trade |

## Plans

| Plan | Covers | Milestone |
|------|--------|-----------|
| [world-model](plans/world-model.md) | 01 (foundation), 02 | M0, M1 |
| [sim-kernel](plans/sim-kernel.md) | 03, 04, 05 | M2 |
| [first-playable](plans/first-playable.md) | 06, 15, 12 | M3 |
| [combat](plans/combat.md) | 07 | M3 |
| [devices](plans/devices.md) | 16 | M4 |
| [abilities](plans/abilities.md) | 08 | M4 |
| [ai](plans/ai.md) | 09 | M4 |
| [worldgen](plans/worldgen.md) | 10 | M5 |
| [scenarios](plans/scenarios.md) | 13 | M5 |
| [vehicles](plans/vehicles.md) | 11 | M6 |
| [tooling](plans/tooling.md) | 14 | Alongside every stage |
| [beta](plans/beta.md) | The order in which the above was built first | Built |

## After the beta

First, what play-testing the beta finds (the release notes' known
issues). Then the packages still `todo` that matter most, roughly in
this order:

| Work | Packages | Why |
|------|----------|-----|
| Incidents and the director | ai L1 to L4, scenarios SC4 to SC6 | The town carries on away from the player, and things happen that were not waiting for them: pillar 7 and scene S7 |
| Tactics, work and collectives | ai I2, I5 to I7, I10 | People who take cover, work at stations, staff the shops and move as groups |
| Networks and construction | devices G3 to G5, G7, G8, G11, G18 | Power and other resources over networks, machines that run unattended, building: scenes S18 and S24 |
| Hot reload | sim-kernel P8, P7 | Change a mod while the game runs, and keep old saves loading as mods change |
| More game scenarios and reference worlds | scenarios SC2, SC8, SC9; worldgen WG5, WG6, WG12 | Characters chosen at the start, campaigns, worlds in other genres to prove the engine assumes none |
| Clashes and cutscenes | combat C6, first-playable U10 | Scenes S5 and S6 played as moments |
| Vehicles and travel | vehicles V1 to V15, world-model W19 to W21 | Mounts, craft, ships and journeys: scenes S9, S22, S23 |
| Kit | devices G9, G10, G13 to G16 | Gadgets, suits, money and trade: scenes S25 and S26 |
| Inspection | tooling TL3, TL4, TL8 | Traces, the inspector and replay tools, so that any rule firing can be seen in the game |
| Budgets | the benchmark packages of each plan | Crowds, fields and paths are still over budget (see the beta's Log) |
| Flight and a flying hero | movement (a mode through open air, with up and down from the keyboard); abilities A9 (force at range) | A bundled "flying brick" built from parts as a showcase for modders: strength as a trait (blows are joules, so walls break), speed as a fast clock in an area anchored to the hero, a heavy landing that releases a blast scaled by the fall, and flight. Every piece but flight already exists as content |

## Open questions

Each specification ends with its open questions. All have been reviewed
with the project owner and the answers written back into the
specifications. Work the review added is listed at the end of each plan
under "Review additions".

| Document | Open questions |
|----------|----------------|
| 02 World model | Reviewed and settled (decisions D-169 to D-181) |
| 03 to 16 | Reviewed (decisions D-182 to D-195) |
| All | Audited before building, by scripted cross-checks and six independent readings: identifiers, plan coverage, hooks, what minds and players may know, and the interface toolkit (decisions D-234 to D-242) |

What the review left unanswered on purpose:

| State | Questions |
|-------|-----------|
| **Open until it can be tested** | Whether fighting in bulk looks continuous with fighting in full (07 §19, 09 §20). |
| **Deferred** | Translation, wide characters and translation tools (04, 12, 14). Mods as single archives (04). Seeing the future, and cheaper snapshots for the experimental rewind (08, 13). Sound, screen readers, controllers (12). A remote inspector (14). |
| **Kept out** | Lambdas in expressions (05). Characters that scheme or plan ahead; map memory for characters the player does not control (09). A simulated past for generated worlds (10). Simulated waves (11). Tilesets in the first version (12). Shared worlds across saves (13). Distributing mods (14). |

The ones most likely to change something as building goes on:

| Risk | Where | Why it matters |
|------|-------|----------------|
| Budgets measured over target | All | The first performance pass met most budgets; a crowd of fifty, the field step, long paths and a full frame are still over, and footsteps may need a different model for crowds of hundreds |
| Fighting in bulk may not look continuous with fighting in full | 09 §20 | Large battles depend on it |
| An incident's live and abstract forms are written separately | 09 §20; 14 §11 | The off-screen world is only as believable as their agreement |
| Maps are authored as text | 10 §16; 14 §10.2 | Large content depends on the map editor arriving |
| Flow limits are by junction, not along every segment | 16 §6.2 | Automation puzzles may still want finer limits |
| Vessels list as a number and are drawn upright | 11 §10.6 | Capsizing is an event, not something seen to happen |
| "Scenario" means two things | 13 §1; decision D-157 | A rename is possible if the note proves insufficient |
| Coloured light is experimental | 15 §3.3; decision D-230 | It tints what the player sees and changes nothing else. Its cost and its look are untried. |
| Whole-world rewind is experimental | 13 §9.5; decision D-181 | Its cost is unmeasured and several interactions are unresolved. It may change or be withdrawn. Nothing else depends on it. |

## The build order

The stages the plans were written for. The beta built across them in the
order of [plans/beta.md](plans/beta.md), which took from each stage what a
playable town needed.

| Stage | Contents | Result | In the beta |
|-------|----------|--------|-------------|
| M0 | Foundation (plan: world-model, F1–F4) | Workspace, data loading, timeline | Built |
| M1 | World model (plan: world-model, W1–W24; W25 once vehicles package V1 exists) | A world that can be generated, viewed, wrecked and saved from a console | Built, without the coarse layer, portals and subgrids |
| M2 | Data, rules and actions (plan: sim-kernel) | Actions on the timeline; mods and EOC complete | Built, without hot reload and migration |
| M3 | Entities, perception, presentation (plan: first-playable) and combat (plan: combat) | First playable build: in a window, a character in a destructible building with others who must notice them before they can fight them | Built, without clashes and cutscenes |
| M4 | Devices (plan: devices), abilities (plan: abilities), AI and the layered simulation (plan: ai) | Interactions, power, crafting, base building, automation, gadgets and suits, accounts and trade; abilities of every family; characters who decide for themselves; large groups; a world that carries on unobserved | Interactions, crafting, abilities and minds; the rest after the beta |
| M5 | World generation (plan: worldgen) and scenarios (plan: scenarios) | Authored and generated worlds with people in them; games to start in them: sandbox, roguelike, campaign, skirmish, investigation | A generated town and three games; the rest after the beta |
| M6 | Vehicles (plan: vehicles) | Mounts, craft, ships, submarines, aircraft, spacecraft, journeys | After the beta |

Tooling (plan: tooling) is not a stage. Its packages are built as soon as
their dependencies allow.

Each stage adds to a set of small **reference mods** in different genres
(for example a modern block with supers, a fantasy dungeon, a horror
house). Their purpose is to prove that nothing in the engine has quietly
assumed a setting. The beta ships one, `core`, a market town with a watch, a
gang and a sampler of powers; the others come with the reference worlds
after it.
