# Roadmap

What has been designed, what comes next, and in what order. Each remaining
document gets the same treatment the world model got: a specification, then
an implementation plan under `docs/plans/`.

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
| 13 | Scenarios, modes and events | Next |
| 14 | Tooling: console, inspector, profiler, mod tools | Grows with everything |
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
| [vehicles](plans/vehicles.md) | 11 | M6 |

## What each remaining document must settle

"Fixed" items are already decided and are not reopened without a new entry
in [decisions.md](decisions.md). Every document must respect rule R10 of the
architecture: mechanism, not meaning.

### 13 — Scenarios, modes and events

Scope: scenario definitions (worlds, start, controlled characters, rules,
goals, win and loss conditions, save policy); sandbox starts; roguelike
runs; single battles; the random event director; investigations built on
causes, decals and witness beliefs.

Fixed:
- A mode such as "roguelike" is a set of scenario rules, not an engine
  feature.
- A scenario supplies character specs or the templates a player may choose
  from (06 §10.3), the screens to use (12), and the profile settings.
- Journeys raise `travel_step` for the event director (11 §13.3).
- Survival play is content built from pools, recipes, construction and
  networks (16 §12). A survival scenario needs no engine support beyond
  them.
- Returning to an earlier moment of the whole world (a checkpoint, a
  return after death) is a scenario rule over saving and loading, not an
  ability effect (08 §15).
- A scenario's worlds are realm definitions with world generators
  (10 §9): generated, authored, or made on demand as they are reached.
  Set dressing is maps applied to the world (10 §4.3).
- Incidents (09 §11) are the unit of things happening. The event director
  starts and paces them for the player; it does not have a second kind of
  event. Investigations rest on causes (02 §7.3), memories and personas
  (15), and knowledge stores (09 §12).

## Suggested build order

| Stage | Contents | Result |
|-------|----------|--------|
| M0 | Foundation (plan: world-model, F1–F4) | Workspace, data loading, timeline |
| M1 | World model (plan: world-model, W1–W23) | A world that can be generated, viewed, wrecked and saved from a console |
| M2 | Data, rules and actions (plan: sim-kernel) | Actions on the timeline; mods and EOC complete |
| M3 | Entities, perception, presentation (plan: first-playable) and combat (plan: combat) | First playable build: in a window, a character in a destructible building with others who must notice them before they can fight them |
| M4 | Devices (plan: devices), abilities (plan: abilities), AI and the layered simulation (plan: ai) | Interactions, power, crafting, base building, automation, gadgets and suits; abilities of every family; characters who decide for themselves; large groups; a world that carries on unobserved |
| M5 | World generation (plan: worldgen), then document 13 | Authored and generated worlds with people in them; scenarios and modes |
| M6 | Vehicles (plan: vehicles) | Mounts, craft, ships, submarines, aircraft, spacecraft, journeys |

The water and hull physics in the vehicles plan (V1, V3, V4) depend only on
the world model and can be built much earlier than M6.

The documents still to write are 13 (scenarios, modes and events) and 14
(tooling). Tooling has collected requests along the way: a paint tool that
writes the map format (10 §16), a comparison of an incident's live and
abstract forms (09 §20), the inspector views of 09 §16, and rule tracing
(05 §15).

From M3 onward, each stage adds to a set of small **reference mods** in
different genres (for example a modern block with supers, a fantasy dungeon,
a horror house). Their purpose is to prove that nothing in the engine has
quietly assumed a setting.
