# Roadmap

What has been designed and in what order to build it. Every subsystem has
a specification and an implementation plan under `docs/plans/`. The design
is complete; no engine code exists yet.

Document numbers are stable. Other documents refer to them by number.

## Documents

| # | Document | Status |
|---|----------|--------|
| 00 | [Vision](00-vision.md) | Written |
| 01 | [Architecture](01-architecture.md) | Written |
| 02 | [World model](02-world-model.md) | Written |
| 03 | [Time, clocks and actions](03-time-and-actions.md) | Written |
| 04 | [Data and modding pipeline](04-data-and-modding.md) | Written |
| 05 | [Expressions and EOC](05-expressions-and-eoc.md) | Written |
| 06 | [Entities, bodies, species and items](06-entities-and-bodies.md) | Written |
| 07 | [Combat, techniques and clashes](07-combat.md) | Written |
| 08 | [Abilities and powers](08-abilities-and-powers.md) | Written |
| 09 | [AI and the layered simulation](09-ai-and-simulation.md) | Written |
| 10 | [World generation and the map format](10-world-generation.md) | Written |
| 11 | [Vehicles, mounts, the sea, space and travel](11-vehicles-and-travel.md) | Written |
| 12 | [Presentation and the UI toolkit](12-presentation-and-ui.md) | Written |
| 13 | [Scenarios, modes and events](13-scenarios-and-events.md) | Written |
| 14 | [Tooling](14-tooling.md) | Written |
| 15 | [Perception, belief and social behaviour](15-perception-and-social.md) | Written |
| 16 | [Interaction, devices, machines and building](16-devices-and-machines.md) | Written |

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

## What remains

Nothing remains to design before building can start. The first package is
F1 in [plans/world-model.md](plans/world-model.md).

Each specification ends with its open questions. They are being reviewed
with the project owner one document at a time, and the answers written back
into the specifications.

| Document | Open questions |
|----------|----------------|
| 02 World model | Reviewed and settled (decisions D-169 to D-180) |
| All others | Awaiting review |

The ones most likely to change something once code exists:

| Risk | Where | Why it matters |
|------|-------|----------------|
| Every budget is a target, not a measurement | All | The benchmark packages may show a design is too slow as specified |
| Fighting in bulk may not look continuous with fighting in full | 09 §20 | Large battles depend on it |
| An incident's live and abstract forms are written separately | 09 §20; 14 §11 | The off-screen world is only as believable as their agreement |
| Maps are authored as text | 10 §16; 14 §10.2 | Large content depends on the map editor arriving |
| Network capacity is checked per network, not per segment | 16 §17 | Automation puzzles may want finer limits |
| Vessels do not tilt | 11 §18 | Ships sink upright |
| Characters do not plan ahead or get lost | 09 §20 | Cunning is authored, not reasoned |
| "Scenario" means two things | 13 §1; decision D-157 | A rename is possible if the note proves insufficient |

## Suggested build order

| Stage | Contents | Result |
|-------|----------|--------|
| M0 | Foundation (plan: world-model, F1–F4) | Workspace, data loading, timeline |
| M1 | World model (plan: world-model, W1–W23) | A world that can be generated, viewed, wrecked and saved from a console |
| M2 | Data, rules and actions (plan: sim-kernel) | Actions on the timeline; mods and EOC complete |
| M3 | Entities, perception, presentation (plan: first-playable) and combat (plan: combat) | First playable build: in a window, a character in a destructible building with others who must notice them before they can fight them |
| M4 | Devices (plan: devices), abilities (plan: abilities), AI and the layered simulation (plan: ai) | Interactions, power, crafting, base building, automation, gadgets and suits; abilities of every family; characters who decide for themselves; large groups; a world that carries on unobserved |
| M5 | World generation (plan: worldgen) and scenarios (plan: scenarios) | Authored and generated worlds with people in them; games to start in them: sandbox, roguelike, campaign, skirmish, investigation |
| M6 | Vehicles (plan: vehicles) | Mounts, craft, ships, submarines, aircraft, spacecraft, journeys |

The water and hull physics in the vehicles plan (V1, V3, V4) depend only on
the world model and can be built much earlier than M6.

Tooling (plan: tooling) is not a stage. Its packages are built as soon as
their dependencies allow, starting in M1.

From M3 onward, each stage adds to a set of small **reference mods** in
different genres (for example a modern block with supers, a fantasy dungeon,
a horror house). Their purpose is to prove that nothing in the engine has
quietly assumed a setting.
