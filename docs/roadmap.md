# Roadmap

What has been designed, what comes next, and in what order. Each remaining
document gets the same treatment the world model got: a specification, then
an implementation plan under `docs/plans/`.

## Documents

| # | Document | Status |
|---|----------|--------|
| 00 | [Vision](00-vision.md) | Written |
| 01 | [Architecture](01-architecture.md) | Written |
| 02 | [World model](02-world-model.md) and [plan](plans/world-model.md) | Written |
| 03 | Time, clocks and actions | Next |
| 04 | Data and modding pipeline | Next |
| 05 | Expressions and EOC | Next |
| 06 | Entities, bodies and items | After 03–05 |
| 07 | Combat, techniques and clashes | After 06 |
| 08 | Powers | After 07 |
| 09 | AI and the layered simulation | After 06 |
| 10 | World generation and map format | After 04 |
| 11 | Vehicles, travel, space and dimensions | After 06, 10 |
| 12 | Presentation: scene, renderers, cutscenes, UI | After 03; needed for the first playable build |
| 13 | Scenarios, missions, crimes and random events | After 05, 09 |
| 14 | Tooling: console, inspector, profiler, mod tools | Grows with everything |

## What each remaining document must settle

Constraints listed here are already decided and are not reopened without a
new entry in [decisions.md](decisions.md).

### 03 — Time, clocks and actions

Scope: action definitions and phases; scheduling; how speed shortens phases;
reaction windows and when the player is asked; interrupts and cancels;
decision requests; how warm and cold entities are stepped; what the player
sees when someone acts hundreds of times between their own decisions.

Fixed: microsecond ticks; clocks with rates (01 §5); `advance` returns when a
decision is needed (01 §5.3).

For the project owner:
- How often should the game stop to ask for a reaction (every incoming
  attack you could perceive, or only ones that matter, by a rule you set)?
- One controlled character, or the ability to control a party or swap
  between characters?

### 04 — Data and modding pipeline

Scope: the complete loading rules started in 01 §7; patching another mod's
definitions; hot reload; migration and obsoletion; the linter; schema
publication; mod packaging; error message standards.

Fixed: namespaced ids; dense ids in sorted order; saves refer to string ids.

### 05 — Expressions and EOC

Scope: the expression language (types, operators, functions, subject and
variable scopes); triggers; conditions; the effect verb list; control flow
(sequences, branches, loops, weighted choices, delayed and recurring
effects); context passing; how EOCs are debugged and traced; the reserved
script hook's interface.

Fixed: compiled at load; deterministic; every subsystem publishes a modding
surface that this document binds.

For the project owner:
- Which CDDA EOC features do you rely on most, and which frustrate you most?
  The answer shapes syntax and tooling priorities.

### 06 — Entities, bodies and items

Scope: the component store; creature definitions; body plans, parts, tissue
layers, wounds, bleeding, severing, healing; how damage packets act on
bodies; attributes and skills; items, materials and inventories; promotion
between statistical population, cold agent and full entity.

Fixed: anatomical, data-defined bodies; tissue uses the world's material
table and damage rule (02 §5.4, §12); bodies in flight are world bodies
(02 §15).

For the project owner:
- How should characters grow: use-based skills, training, point-buy, power
  acquisition events, or a mix?
- How graphic should injury text be by default, and should it be a setting?

### 07 — Combat, techniques and clashes

Scope: strikes, grapples, throws and weapons as actions that emit damage
packets; techniques and styles as data; stances; targeting; defence and
reactions; clash detection and resolution; paired actions for takedowns and
executions; knockback into the world's body system.

Fixed: outcomes come from packets against materials and bodies; no separate
"hit points versus damage roll" layer.

For the project owner:
- How much is chosen per exchange: a technique and a target each time, or a
  stance and intent with the character choosing techniques?
- Is a master effortlessly cutting down weak foes meant to be fast to play
  (one input, several kills) as well as fast in the fiction?

### 08 — Powers

Scope: a power as a bundle of actions, passive effects, resources and
scaling; energy sources and costs; common building blocks (flight, strength,
speed, durability, energy projection, telekinesis, matter control, time
control, teleportation, transformation); how powers compose and are
acquired, trained and lost.

Fixed: powers act through the same effects as everything else; nothing is a
special case in the engine.

### 09 — AI and the layered simulation

Scope: hot AI (perception, goals, tactics, path-finding over the world's
query interface); warm throttling; cold agents and their coarse steps;
factions, populations and economies as statistics; crimes and incidents as
generated events with real consequences (pending edits, casualties,
evidence); news and reputation; promotion and demotion.

Fixed: tickets (02 §10); pending edits and stamps (02 §17); causes (02 §7.3).

### 10 — World generation and map format

Scope: planet and region survey; city layout; the JSON map format (rows,
palettes, nested pieces, parametric buildings); placement of sites; ruin
variants; mapgen transforms for derived realms; mapgen updates.

Fixed: generation is pure and per tract (02 §9.3).

For the project owner:
- How big should the default world be, and is it one city done deeply first
  or a whole planet done thinly first?

### 11 — Vehicles, travel, space and dimensions

Scope: vehicles as multi-cell occupants or subgrids; driving and flight
models; turning; boarding and interiors; travel rules, interception and
encounters; chart content; ships; dimension travel conditions.

Fixed: subgrids (02 §18); travelers, carriers and charts (02 §20).

### 12 — Presentation

Scope: the scene model; camera and multi-storey view; event playback over
real time; cutscene definitions and matching; the window renderer; the
terminal renderer; input; UI screens.

Fixed: presentation never changes outcomes (01 §10).

### 13 — Scenarios, missions, crimes and random events

Scope: scenario and sandbox start definitions; mission structures; the
random event director; investigations built on causes, decals and witnesses.

## Suggested build order

| Stage | Contents | Result |
|-------|----------|--------|
| M0 | Foundation (plan: world-model, F1–F4) | Workspace, data loading, timeline |
| M1 | World model (W1–W23) | A world that can be generated, viewed, wrecked and saved from a console |
| M2 | Documents 03, 04, 05 and their plans | Actions on the timeline; mods and EOC complete |
| M3 | Documents 06, 07, 12 | First playable build: a fight in a destructible city block, in a window |
| M4 | Documents 08, 09, 10 | Powers; a generated city that lives |
| M5 | Documents 11, 13 | Vehicles, space, dimensions, scenarios |

Documents 03, 04 and 05 can be designed while M1 is being built. They do not
depend on world code, only on the contracts in 01.
