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
| 07 | Combat, techniques and clashes | Next |
| 08 | Abilities and powers | After 07 |
| 09 | AI and the layered simulation | After 07 |
| 10 | World generation and map format | Any time |
| 11 | [Vehicles, mounts, the sea, space and travel](11-vehicles-and-travel.md) | Written |
| 12 | [Presentation and the UI toolkit](12-presentation-and-ui.md) | Written |
| 13 | Scenarios, modes and events | After 09 |
| 14 | Tooling: console, inspector, profiler, mod tools | Grows with everything |
| 15 | [Perception, belief and social behaviour](15-perception-and-social.md) | Written |
| 16 | [Interaction, devices, machines and building](16-devices-and-machines.md) | Written |

## Plans

| Plan | Covers | Milestone |
|------|--------|-----------|
| [world-model](plans/world-model.md) | 01 (foundation), 02 | M0, M1 |
| [sim-kernel](plans/sim-kernel.md) | 03, 04, 05 | M2 |
| [first-playable](plans/first-playable.md) | 06, 15, 12 | M3 |
| [devices](plans/devices.md) | 16 | M4 |
| [vehicles](plans/vehicles.md) | 11 | M6 |

## What each remaining document must settle

"Fixed" items are already decided and are not reopened without a new entry
in [decisions.md](decisions.md). Every document must respect rule R10 of the
architecture: mechanism, not meaning.

### 07 — Combat, techniques and clashes

Scope: strikes, grapples, throws and weapons as actions that emit damage
packets; techniques and styles as data; stances; targeting; defence and
reactions; clash detection and resolution; paired actions for takedowns and
executions; knockback into the world's body system; ranged weapons across
technology levels.

Fixed:
- Attacks, defences and reactions are actions, windows and joint actions
  as specified in 03. Combat adds definitions and verbs, not a second
  timing system.
- An attack ends in a `Hit` (06 §7). Combat's job is to turn a technique, a
  striking surface (06 §9.1) and the attacker's attributes into the hit's
  energy, intensity, area and target part.
- Signature moves name their cutscene (12 §8.1).
- Weapons are items and may be assemblies (16 §11): a magazine is a store,
  a shot is an action with a cost, a scope is a module.
- Outcomes come from packets against materials and bodies. There is no
  separate layer of hit points and damage rolls.
- Simple defaults for every exchange, with full control of technique and
  target for players who want it.
- Hitting several foes in one action is something an ability does, defined
  in data. The engine has no "weak enemy" shortcut.
- Nothing is scaled for fairness.

### 08 — Abilities and powers

Scope: an ability as a bundle of actions, passive effects, resources and
scaling; costs and sources; composition; acquisition, training and loss.
The document must show that the same building blocks cover force (strength,
speed, energy projection), movement (flight, teleportation), control of
matter, bodies and time, and **non-violent** abilities: stealth, illusion,
disguise, mind influence, command, summoning and binding, reading and
planting beliefs.

Fixed:
- Abilities act through the same effects as everything else. "Magic",
  "superpower", "skill" and "technology" are labels chosen by content.
- An ability is conferred by a trait (06 §5): its actions are grants, its
  costs are pools, its growth is a track.
- Abilities over minds and perception use the verbs of 15: personas,
  memories, relations, signals, orders.
- An ability may come from equipment: modules confer traits and actions
  while supplied (16 §11.2). Innate and equipped abilities are the same
  kind of thing.

### 09 — AI and the layered simulation

Scope: decision-making for hot characters, built on beliefs (15); goals,
tactics and path-finding over the world's query interface; groups and
formations so that large battles are affordable (scene S19); warm
throttling; cold agents and coarse steps; populations, organisations and
economies as statistics; incidents as generated events with real
consequences; promotion and demotion.

Fixed:
- Tickets (02 §10); pending edits and stamps (02 §17).
- Decisions read beliefs, tracks, memories, relations and group knowledge
  (15). Static geometry is common knowledge; nothing else is.
- Characters act through plans of actions (03 §5.3) like the player.
- Anonymous, notable and agent forms are as 06 §12; this document gives
  agents their behaviour.
- Crews work stations (11 §7); orders must be heard (15 §10).
- Work is taken from the job list (16 §10.2). This document decides who
  does which job, hauling to supply jobs, and how storage areas are used.
- Tract services (16 §4.4) are switched by the cold simulation.

### 10 — World generation and map format

Scope: region survey; settlement and site layout for any kind of world; the
JSON map format (rows, palettes, nested pieces, parametric structures);
dungeons and interiors; ruin variants; mapgen transforms for derived realms;
mapgen updates.

Fixed:
- Generation is pure and per tract (02 §9.3).
- World size and shape are properties of each realm definition. There is no
  default planet. Reference mods use small realms.
- The map format also describes vessels (11 §12.1), places devices
  (02 §5.5), areas and sites, and fills seas to `sea_level` (11 §4.3).
- Populations and content tables (06 §12, §9.3) are assigned by generation.
- Maps place conduits, devices and service entries (16), and a map piece
  can be used as a building design (16 §10.3).

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

## Suggested build order

| Stage | Contents | Result |
|-------|----------|--------|
| M0 | Foundation (plan: world-model, F1–F4) | Workspace, data loading, timeline |
| M1 | World model (plan: world-model, W1–W23) | A world that can be generated, viewed, wrecked and saved from a console |
| M2 | Data, rules and actions (plan: sim-kernel) | Actions on the timeline; mods and EOC complete |
| M3 | Entities, perception, presentation (plan: first-playable), then document 07 | First playable build: in a window, a character in a destructible building with others who must notice them before they can fight them |
| M4 | Devices (plan: devices), then documents 08 and 09 | Interactions, power, crafting, base building, automation, gadgets and suits; abilities; characters with lives; large groups |
| M5 | Documents 10, 13 | Generated worlds; scenarios and modes |
| M6 | Vehicles (plan: vehicles) | Mounts, craft, ships, submarines, aircraft, spacecraft, journeys |

The water and hull physics in the vehicles plan (V1, V3, V4) depend only on
the world model and can be built much earlier than M6.

The next document to write is 07 (combat), then 08, 09, 10 and 13.

From M3 onward, each stage adds to a set of small **reference mods** in
different genres (for example a modern block with supers, a fantasy dungeon,
a horror house). Their purpose is to prove that nothing in the engine has
quietly assumed a setting.
