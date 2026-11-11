# Roadmap

What has been designed, what remains, and in what order to build. Each
subsystem has a specification and an implementation plan under
`docs/plans/`.

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
| 14 | Tooling: console, inspector, profiler, mod tools | Remaining |
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

## What remains to design

### 14 — Tooling

The one subsystem document not yet written. Much of its content has been
committed to elsewhere and needs gathering, ordering and filling in.

Scope: the in-game console; the inspector; rule, mind and incident traces;
the profiler; map and world previews; a paint tool for maps; mod
scaffolding, linting and the generated reference; test scripts and golden
frames; replay from input logs.

Already specified, to be collected:

| Tool | Specified in |
|------|--------------|
| `lint`, `schema`, `docs`, `init-mod`, `explain`, `show` | 04 §13 |
| Rule trace, explain, profile, console evaluation, assertions | 05 §15 |
| World viewer and test script runner | plans/world-model W11 |
| Golden frames | 12 §12 |
| Views of a character's goals, plan, options, route and beliefs | 09 §16 |
| Map and generation previews and checks | 10 §12 |

Asked for along the way and not yet specified:

| Request | From |
|---------|------|
| A paint tool that writes the map format | 10 §16 |
| A comparison of an incident's live and abstract forms over many runs | 09 §20 |
| A deduction aid for cases | 13 §16 |
| Omniscient and perceived views side by side | 12 §5; 15 §13 |
| Replaying a save and input log to a divergence | 03 §4.4; 01 §4 |

Fixed:
- Tools read the simulation and change it only through commands
  (03 §4.4), like any other frontend.
- Debug screens are built with the UI toolkit in a `debug` context
  (12 §10.4).
- Anything the generated reference does not list does not exist (04 §13).

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

Every subsystem that play depends on is now specified and planned. The
remaining document, 14 (tooling), gathers tools that are mostly committed
to already.

From M3 onward, each stage adds to a set of small **reference mods** in
different genres (for example a modern block with supers, a fantasy dungeon,
a horror house). Their purpose is to prove that nothing in the engine has
quietly assumed a setting.
