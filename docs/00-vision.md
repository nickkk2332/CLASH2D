# 00 — Vision

CLASH2D is an engine kit: a turn-based ASCII immersive sim and superpower
sandbox in which every world, creature, power, rule and screen is supplied by
JSON mods. **The engine provides tools. Creators provide the results.**

It is not one game. The same engine should run a superhero city, a fantasy
world of demons and cheat skills, a horror scenario spent hiding in lockers, a
Napoleonic battlefield visited with a modern rifle, a roguelike dungeon, and a
grounded campaign where a half-decent swordsman dies to one bad cut.

In a sense, CLASH2D is very much an engine for those who want a tile based, simulation sandbox. 

Due to this, coherence often falls onto the mod creator. The engine provides powerful tools to use and a comprehensive physics engine that can be tweaked to ones liking. 

All kinds of genres, provided it is a turn based tile based singleplayer top down view game with a player avatar, are possible. 

Every later design document is judged against this one. If a design cannot
produce the scenes in section 4, the design is wrong.

## 1. Pillars

1. **Mechanism, not meaning.** The engine knows about matter, bodies, time,
   perception, events and screens. It does not know what magic is, what a
   race is, what era it is, who the good guys are, or what is balanced. Those
   are content. Section 2 draws the line.
2. **Outcomes come from simulation.** A punch that sends someone through a
   wall is a physical event with mass, speed, material strength and injuries.
   The engine decides what happened, then shows it well. Presentation never
   invents outcomes.
3. **One ruleset at every scale, and no pretence of fairness.** A bar fight
   and a planet-cracking beam use the same damage model and the same
   timeline. Only the numbers differ, and the numbers are allowed to be
   brutal. An ordinary person is ordinary: they should fight other ordinary
   people and run from everything else. The engine never scales, softens or
   levels an encounter. Balance is the author's job.
4. **One shared timeline.** Time is not divided into turns. Every action has
   a windup, an impact and a recovery on a common clock, and the game pauses
   when a controlled character needs a decision. Speed, reactions,
   interrupts, clashes and time powers all come out of that one mechanism.
5. **Everything is interactive.** This is an immersive sim. Doors, lockers,
   lights, machines, containers, vehicles and terrain do what they look like
   they do, and they interact with each other. Problems have several
   solutions because the systems overlap, not because an author listed them:
   fight, sneak, trick, bribe, command, cut the power, or go through the
   wall.
6. **Characters perceive, believe and react.** No deep dialogue trees.
   Characters notice things through their senses, form beliefs that may be
   wrong, and respond by rules that authors write. Players make up the story
   from what emerges. The world records what is true; characters act on what
   they think is true.
7. **The world moves without you.** People, factions, incidents and battles
   carry on away from the player at lower detail, and their results are real
   when the player arrives.
8. **Moments.** Takedowns, clashes and executions can play as short
   choreographed sequences. They are triggered and described by data, and
   they show what the simulation already decided.
9. **Expression.** For players: who you are (species, body, abilities,
   background) and how you play are wide open. For creators: anything the
   bundled content does, a mod can change or replace, in JSON, with good
   error messages and hot reload. That includes the menus.
10. **Deterministic and inspectable.** The same save, mods and inputs always
    give the same result. Any cell, entity, event or rule firing can be
    inspected in-game. This is what makes a system this large debuggable, and
    what makes modding pleasant rather than merely possible.

## 2. What the engine provides and what mods decide

| The engine provides | Mods decide |
|---------------------|-------------|
| Matter, materials, damage, physics | What exists and how strong it is |
| Bodies made of parts and tissue, editable at runtime | Species, races, monsters, machines, undead, and what can be done to them |
| Minds separable from bodies; ownership and command relations | Souls, possession, minions, summoning, necromancy of every flavour |
| Actions with phases on a timeline | Techniques, spells, powers, skills, and when the player is asked to react |
| Senses, perception and belief | What characters do about what they notice; stealth, disguise, deception, persuasion |
| Attributes, pools and counters | Progression: training, use-based skills, point-buy, levels, cheat skills, or none |
| Events, conditions, effects, variables | Every rule of the game |
| Devices, links between them, interaction verbs | Machines, locks, traps, computers, rituals, wiring |
| Resources, networks, recipes, construction, assemblies | Power grids, mana grids, automation, crafting, base building, gadgets, powered suits, implants |
| Realms, charts, generation, layered simulation | Worlds, eras, tech levels, dimensions, armies, economies |
| A glyph UI toolkit | Screens, menus, HUDs, character creation, the main menu itself |
| A scenario framework | Sandbox, roguelike, campaign, warzone, and their win, loss and save rules |

"Tech level", "magic", "alignment", "class" and "difficulty" do not appear in
engine code. If a mod wants them, they are tags, variables and rules.

## 3. What players do

- Start in a **sandbox** (build a character, pick a world, go) or a
  **scenario** (authored start, goals, rules). A scenario can be an open
  world, a roguelike run with permadeath, a single battle, or anything else
  its author defines.
- Control one character or, where the scenario allows, a party.
- Play anyone the content allows: a hero, a villain, a mage, a necromancer,
  a demon, a wandering swordsman, or a nobody trying to survive.
- Fight with simple defaults or with full control over technique and target.
- Avoid fighting: hide, sneak, deceive, manipulate, recruit, command.
- Use the world: machines, vehicles, mounts, ships, tools, terrain.
- Survive and build: gather, craft, construct a base, wire it up, automate
  it, defend it.
- Equip: gadgets, modular gear, powered suits, implants.
- Explore, travel between places, planets and dimensions, and run into
  events that were not waiting for them.

## 4. North-star scenes

These are the acceptance tests for the design as a whole. Each later document
states which scenes it serves.

### Force

| # | Scene | What must be true |
|---|-------|-------------------|
| S1 | **Through the wall.** A brawler punches a thug through a brick wall. | The wall breaks because the hit exceeded its strength. The thug lands in the next room with injuries that fit the impact. |
| S2 | **Three buildings.** A super-strong blow launches a tough villain through three office buildings. | Each wall costs speed. Damaged supports in the last building fail and the upper floors come down floor by floor. |
| S3 | **Faster than the blast.** A bomb goes off. A speedster carries four civilians clear before the pressure wave reaches them. | The wave travels at a real speed on the timeline. Being faster than it is enough. |
| S4 | **One cut.** A swordsman draws once. Five foes each lose what the blade passed through. | The outcome comes from the body model and the materials involved. Cutting down several foes at once is an ability defined in data, not an engine shortcut. |
| S5 | **The clash.** Two fighters commit heavy strikes whose impact windows overlap. | A clash is resolved, a mini-cutscene plays, and the released energy shatters windows down the block. |
| S6 | **Takedown.** From stealth, an attacker drops a guard in one paired sequence. | Attacker and victim are locked into a shared choreographed action whose result was decided by the simulation. |
| S7 | **Meanwhile.** While the player is across town, a robbery turns into a fight that wrecks two blocks. | The player arrives to rubble, casualties, witnesses, evidence and news. None of it was simulated tile by tile. |
| S8 | **Planet-buster.** A beam scars a continent. | Nearby tiles are destroyed in detail. Distant regions are marked devastated and generate as wasteland when visited. |
| S9 | **Rooftop to orbit.** A flyer leaves a roof, climbs into vacuum, boards a ship, and the fight continues inside it on the way to another world. | One position system, one timeline. The ship's interior stays playable in transit. |
| S10 | **Time stop.** A time-stopper freezes a street. Thrown knives hang in the air and falling debris pauses. | Time is a property of places and things, and can be scaled or stopped per area. |
| S11 | **Thrown building.** A titan tears the top floors off a tower and throws them. | A piece of the world can become a single moving object and become world again on impact. |

### Fragility

| # | Scene | What must be true |
|---|-------|-------------------|
| S13 | **The locker.** An ordinary person hides in a locker while something searches the room. | A prop can hold a person. The searcher works from sight, sound and a last known position, not from the truth. Its search behaviour is authored in data. |
| S14 | **Death spiral.** A decent swordsman takes one bad cut to the sword arm. | There is no hit-point cushion. The wound impairs the arm, the weapon drops, blood loss and pain mount, and the fight is effectively over. |
| S19 | **Wrong century.** A normal person with a modern rifle stands on a Napoleonic battlefield among hundreds of soldiers. | Technology is content. Large battles run. The rifle is a real advantage and the person is still one musket ball from dead. |

### Guile and command

| # | Scene | What must be true |
|---|-------|-------------------|
| S15 | **The frame job.** A trickster commits a crime wearing someone else's face. | Witnesses believe what they perceived. The guards go after the wrong person. The engine still knows who did it. |
| S16 | **Two necromancers.** One raises the fallen as servants and gives them orders. Another pulls a soul out of one body and puts it in a different one. | Minds and bodies are separable. Ownership and command are relations a mod can create. Neither necromancer is an engine feature. |
| S17 | **Reshaping touch.** A power rewrites a victim's body: parts added, removed, reshaped, made of something else. | A body is live data that effects can edit, not a fixed template. |
| S18 | **The machine room.** A lever, a generator, a powered door, a terminal. | Devices are linked. Cut the power and the door fails; or use the terminal; or break the wall. Nobody scripted the list of solutions. |

### Authorship

| # | Scene | What must be true |
|---|-------|-------------------|
| S12 | **A mod did that.** Everything above is defined in JSON. | A modder adds a new power, material, hazard, creature, device and cutscene without touching Rust. |
| S20 | **Cheat skill.** A character arrives in a fantasy world as a custom species chosen at creation, with an overpowered ability and a status screen, all from one mod. | Species, progression and screens are data. |
| S21 | **A different game.** A total-conversion mod replaces the main menu, character creation, HUD and rules, and ships a roguelike with permadeath. | The engine's own screens and modes are built from the same toolkit mods use. |

### Machines and making

| # | Scene | What must be true |
|---|-------|-------------------|
| S24 | **The workshop.** A survivor builds a generator, wires it to a workshop, sets a furnace and a loader running, and leaves. A day later the bars are stacked, or the fuel ran out halfway. | Devices need real supply from real sources. Resources are balanced over networks that content defines. Machines work unattended, and what they did while nobody watched is correct. What is built is made of materials and obeys the world's physics. |
| S25 | **The suit.** A powered suit gives its wearer strength, flight and air to breathe in vacuum. A hit takes out an arm's weapon. Then the core fails. | Equipment is built from modules that share power. What a module gives lasts only while it is supplied and intact. An unpowered suit is a great weight of metal with someone inside. |

### Vehicles

| # | Scene | What must be true |
|---|-------|-------------------|
| S22 | **The deep.** A submarine dives past what its hull can take. A seam fails, a compartment floods, the crew seal the bulkhead and blow ballast. | Pressure rises with depth. Hulls are materials with limits. Water enters through holes at a rate, compartment by compartment, and the boat's weight decides whether it comes back up. |
| S23 | **The chase.** A rider on a galloping horse trades blows with a pursuer on a motorbike, weaving through traffic. | Riding and driving occupy some of a character's channels and leave others free. A mount has a mind of its own. Vehicles have speed, grip and momentum. |

## 5. Decisions made with the project owner

| Topic | Decision |
|-------|----------|
| Language | Rust |
| Display | Glyph grid in its own window first; plain terminal frontend as a reduced-effects fallback |
| Time | Shared timeline with action phases; pauses for decisions |
| Power ceiling | Planet-buster. Tile-accurate destruction nearby, coarse destruction far away |
| Off-screen world | Layered simulation: full detail near the player, abstract elsewhere |
| Vertical space | One z-level per storey (3 m), plus an exact height for everything that moves |
| Bodies | Anatomical and data-defined: parts, tissue layers, materials, wounds, severing |
| Mod code | JSON and EOC first, rich enough to hold the game's rules. A slot for sandboxed scripting is reserved but nothing depends on it |
| Identity | A genre-neutral engine kit. Bundled content is reference material, not "the game" |
| Fairness | None enforced. Ordinary people are ordinary. Authors balance their own content |
| Reaction prompts | Decided by data-defined rules, not fixed by the engine |
| Control | One character by default; party control is optional and set by the scenario |
| Combat input | Simple defaults for every action, with full granular control available |
| Progression | Data-defined; mods mix schemes freely |
| UI | An ASCII toolkit that mods use for their own screens, up to replacing the main menu |
| Building | Survival, base building and automation in the manner of CDDA, RimWorld and Minecraft's tech and magic mods are first-class: custom resource grids, devices that need real supply, powerful automation |
| Kit | Gadgets of every kind and powered suits must be possible |
| Character AI | Consistent, not clever. Characters follow readable rules that depend on who they are, with some chance in them, in the manner of Hitman, F.E.A.R. and immersive sims. They can do things that look smart. They do not scheme or plan around the player, even with powers that could |
| World history | Not simulated, beyond the barest level of a few facts given to places when a world is made |

The reasoning and consequences of each are recorded in [decisions.md](decisions.md).

## 6. Limits and non-goals

- **Always turn-based.** Presentation may animate; the simulation never
  waits on a wall clock.
- **Always glyphs.** The UI toolkit is rich within a character grid: panels,
  lists, tables, bars, maps, text, input. It is not a pixel widget library.
- **No dialogue-tree system.** Short exchanges and reactions are authored as
  rules. A mod could build trees from the UI toolkit and EOCs; the engine
  does not aim at them.
- Multiplayer in the first version. Determinism keeps lockstep co-op possible
  later; nothing else is done for it now. It is also not planned at the moment.
- Characters that plot. The AI is built to be consistent and readable, not
  to outwit the player. A scheme in a story is authored content.
- A simulated past. Generated worlds are not run through years of history.
- True 3D rendering, or cubic 1 m voxels.
- Rigid-body rotation, joints, or accurate fluid dynamics. Physics is
  impulse-based and grid-quantised, chosen to be legible and fast.
- Graphical tilesets in the first version. The glyph renderer should not
  prevent them later.
- A bundled flagship game. The repository ships small reference mods in
  different genres whose job is to prove the toolkit and serve as examples.
