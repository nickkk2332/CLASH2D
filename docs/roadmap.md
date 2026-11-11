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
| 02 | [World model](02-world-model.md) and [plan](plans/world-model.md) | Written |
| 03 | Time, clocks and actions | Next |
| 04 | Data and modding pipeline | Next |
| 05 | Expressions and EOC | Next |
| 06 | Entities, bodies, species and items | After 03–05 |
| 07 | Combat, techniques and clashes | After 06 |
| 08 | Abilities and powers | After 07 |
| 09 | AI and the layered simulation | After 06, 15 |
| 10 | World generation and map format | After 04 |
| 11 | Vehicles, mounts, travel, space and dimensions | After 06, 10 |
| 12 | Presentation and the UI toolkit | After 03; needed for the first playable build |
| 13 | Scenarios, modes and events | After 05, 09 |
| 14 | Tooling: console, inspector, profiler, mod tools | Grows with everything |
| 15 | Perception, belief and social behaviour | After 06 |
| 16 | Interaction, devices and machines | After 05, 06 |

## What each remaining document must settle

"Fixed" items are already decided and are not reopened without a new entry
in [decisions.md](decisions.md). Every document must respect rule R10 of the
architecture: mechanism, not meaning.

### 03 — Time, clocks and actions

Scope: action definitions and phases; scheduling; how speed shortens phases;
reaction windows; interrupts and cancels; decision requests for one or
several controlled characters; how warm and cold entities are stepped; what
the player sees when someone acts hundreds of times between their own
decisions.

Fixed:
- Microsecond ticks; clocks with rates (01 §5).
- When a reaction window opens and whether the player is asked is decided by
  data-defined rules, with sensible defaults that players can adjust.
- Zero, one or several controlled characters. Party control is enabled by
  the scenario.
- Every action can resolve from defaults or be specified in full.

### 04 — Data and modding pipeline

Scope: the complete loading rules started in 01 §7; patching another mod's
definitions; hot reload; migration and obsoletion; the linter; schema
publication; mod packaging, including total conversions that replace the
bundled screens and start-up flow; error message standards.

Fixed: namespaced ids; dense ids in sorted order; saves refer to string ids.

### 05 — Expressions and EOC

Scope: the expression language (types, operators, functions, subject and
variable scopes); triggers; conditions; the effect verb list; control flow
(sequences, branches, loops, weighted choices, delayed and recurring
effects); context passing; tracing and debugging; the reserved script hook.

Fixed:
- Compiled at load; deterministic.
- Every subsystem publishes a modding surface that this document binds.
- The goal is CDDA's EOC power with every part of the simulation exposed as
  hooks. Breadth and orthogonality of the verb list matter more than
  brevity of syntax.

### 06 — Entities, bodies, species and items

Scope: the component store; containment; creature definitions; **species**
as composable definitions (body plan, tissues, senses, innate abilities,
variants, player-facing choices at creation); bodies as live data (parts,
tissue layers, wounds, bleeding, pain, severing, healing, runtime reshaping);
how damage packets act on bodies; minds and the relations that attach them to
bodies; ownership and command relations; attributes, pools and counters as
general-purpose primitives for progression; items and inventories; promotion
between statistical population, cold agent and full entity.

Fixed:
- Anatomical, data-defined bodies sharing the world's material table and
  damage rule (02 §5.4, §12).
- No hit-point cushion. Wounds impair function, and impairment compounds
  (scene S14).
- Containment, mind and body separation, editable instances (01 §6).
- Progression schemes are data; the engine supplies primitives and events.
- Species must give players a great deal of expressive room.

To confirm when written: default level of injury description, and whether it
is a player setting.

### 07 — Combat, techniques and clashes

Scope: strikes, grapples, throws and weapons as actions that emit damage
packets; techniques and styles as data; stances; targeting; defence and
reactions; clash detection and resolution; paired actions for takedowns and
executions; knockback into the world's body system; ranged weapons across
technology levels.

Fixed:
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

Fixed: abilities act through the same effects as everything else. "Magic",
"superpower", "skill" and "technology" are labels chosen by content.

### 09 — AI and the layered simulation

Scope: decision-making for hot characters, built on beliefs (15); goals,
tactics and path-finding over the world's query interface; groups and
formations so that large battles are affordable (scene S19); warm
throttling; cold agents and coarse steps; populations, organisations and
economies as statistics; incidents as generated events with real
consequences; promotion and demotion.

Fixed: tickets (02 §10); pending edits and stamps (02 §17); characters act
on belief (01 R11).

### 10 — World generation and map format

Scope: region survey; settlement and site layout for any kind of world; the
JSON map format (rows, palettes, nested pieces, parametric structures);
dungeons and interiors; ruin variants; mapgen transforms for derived realms;
mapgen updates.

Fixed:
- Generation is pure and per tract (02 §9.3).
- World size and shape are properties of each realm definition. There is no
  default planet. Reference mods use small realms.

### 11 — Vehicles, mounts, travel, space and dimensions

Scope: mounts as creatures that carry riders; vehicles as multi-cell
occupants or subgrids; driving and flight models; turning; boarding and
interiors; travel rules, interception and encounters; chart content; ships
and combat between them; dimension travel conditions.

Fixed: subgrids (02 §18); travelers, carriers, encounters and charts
(02 §20); containment (01 §6).

### 12 — Presentation and the UI toolkit

Scope: the scene model; camera and multi-storey view; event playback over
real time; cutscene definitions and matching; the window renderer; the
terminal renderer; input and key binding; and the **UI toolkit**: the widget
set, layout, data binding to expressions, actions bound to commands and EOC
effects, and how mods define screens, HUD elements and menus.

Fixed:
- Presentation never changes outcomes (01 §10).
- Glyph grid only.
- The engine's own screens are built with the toolkit and ship as a mod.

### 13 — Scenarios, modes and events

Scope: scenario definitions (worlds, start, controlled characters, rules,
goals, win and loss conditions, save policy); sandbox starts; roguelike
runs; single battles; the random event director; investigations built on
causes, decals and witness beliefs.

Fixed: a mode such as "roguelike" is a set of scenario rules, not an engine
feature.

### 15 — Perception, belief and social behaviour

Scope: senses as data (sight, hearing, smell, and whatever mods add);
detection and stealth; what a character knows and how it becomes stale or
wrong; perceived identity versus true identity; disguise and illusion;
memory of events; relations between characters (trust, fear, loyalty, debt,
ownership); **reaction rules** that authors write to make characters
respond to what they perceive; short exchanges and barks; how beliefs
spread (witnesses, reports, rumour).

Fixed:
- The world records truth; characters hold beliefs (01 R11).
- No dialogue-tree system. Responses are rules.

### 16 — Interaction, devices and machines

Scope: interaction verbs and reach; device entities and their lifecycle;
contents and hiding places; links between devices (power, signal,
mechanical) and how networks are evaluated on the timeline; locks, traps,
switches, terminals, lights; tools; how devices are built, broken, repaired
and tampered with.

Fixed: interactive elements promote to devices on demand (02 §5.5); every
verb is bound to rules, never to engine code.

## Suggested build order

| Stage | Contents | Result |
|-------|----------|--------|
| M0 | Foundation (plan: world-model, F1–F4) | Workspace, data loading, timeline |
| M1 | World model (W1–W23) | A world that can be generated, viewed, wrecked and saved from a console |
| M2 | Documents 03, 04, 05 | Actions on the timeline; mods and EOC complete |
| M3 | Documents 06, 12, 15, 07 | First playable build: in a window, a character in a destructible building with others who must notice them before they can fight them |
| M4 | Documents 16, 08, 09 | Devices and machines; abilities; characters with lives; large groups |
| M5 | Documents 10, 13 | Generated worlds; scenarios and modes |
| M6 | Document 11 | Mounts, vehicles, space, dimensions |

Documents 03, 04 and 05 can be designed while M1 is being built. They do not
depend on world code, only on the contracts in 01.

From M3 onward, each stage adds to a set of small **reference mods** in
different genres (for example a modern block with supers, a fantasy dungeon,
a horror house). Their purpose is to prove that nothing in the engine has
quietly assumed a setting.
