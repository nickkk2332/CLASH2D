# 00 — Vision

CLASH2D is a turn-based ASCII sandbox engine for superpowered and martial-arts
combat in a destructible, living open world. All content and all game rules are
supplied by JSON mods.

Every later design document is judged against this one. If a design cannot
produce the scenes in section 3, the design is wrong.

## 1. Pillars

1. **Spectacle comes from simulation.** A punch that sends someone through a
   wall is a physical event with mass, speed, material strength and injuries.
   The engine decides what happened, then shows it well. Presentation never
   invents outcomes.
2. **Every scale of power in one ruleset.** A bar fight and a planet-cracking
   beam use the same damage model, the same materials and the same timeline.
   Only the numbers differ.
3. **One shared timeline.** Time is not divided into turns. Every action has a
   windup, an impact and a recovery on a common clock, and the game pauses when
   the player's character needs a decision. Speed, reactions, interrupts,
   clashes and time powers all come out of that one mechanism.
4. **Data over code.** The base game is a mod. The engine contains no content:
   no hard-coded materials, creatures, powers or rules of combat. Anything the
   base game does, a mod can change or replace, in JSON, with good error
   messages and hot reload.
5. **The world moves without you.** People, factions, crimes and fights carry
   on away from the player at lower detail, and their results are real when the
   player arrives.
6. **Moments.** Takedowns, clashes and executions can play as short
   choreographed sequences. They are triggered and described by data, and they
   show what the simulation already decided.
7. **Deterministic and inspectable.** The same save, mods and inputs always
   give the same result. Any cell, entity, event or rule firing can be
   inspected in-game. This is what makes a system this large debuggable, and it
   is what makes modding pleasant rather than merely possible.

## 2. What the player does

- Starts in a **sandbox** (pick or build a character, pick a world, go) or a
  **scenario** (authored start, goals, scripted events).
- Plays a hero, a villain, a wandering swordsman, or nobody in particular.
- Fights, trains, explores, travels between cities, planets and dimensions.
- Runs into **random events**: robberies, disasters, rival supers, invasions.
- **Solves crimes** using what the simulation left behind (damage, marks,
  witnesses, causes), or commits them and deals with the response.
- Uses and builds **vehicles** of any size, from a motorbike to a starship.

## 3. North-star scenes

These are the acceptance tests for the design as a whole. Each later document
states which scenes it serves.

| # | Scene | What must be true |
|---|-------|-------------------|
| S1 | **Through the wall.** A brawler punches a thug through a brick wall. | The wall breaks because the hit exceeded its strength. The thug lands in the next room with injuries that fit the impact. |
| S2 | **Three buildings.** A super-strong blow launches a tough villain through three office buildings. | Each wall costs speed. Damaged supports in the last building fail and the upper floors come down floor by floor. |
| S3 | **Faster than the blast.** A bomb goes off. A speedster carries four civilians clear before the pressure wave reaches them. | The wave travels at a real speed on the timeline. Being faster than it is enough. |
| S4 | **One cut.** A swordsman draws once. Five foes each lose what the blade passed through. | The outcome comes from the body model and the materials involved, not from a special case. |
| S5 | **The clash.** Two fighters commit heavy strikes whose impact windows overlap. | A clash is resolved, a mini-cutscene plays, and the released energy shatters windows down the block. |
| S6 | **Takedown.** From stealth, an attacker drops a guard in one paired sequence. | Attacker and victim are locked into a shared choreographed action whose result was decided by the simulation. |
| S7 | **Meanwhile.** While the player is across town, a villain robs a bank and fights a hero. | The player arrives to rubble, casualties, witnesses, evidence and news. None of it was simulated tile by tile. |
| S8 | **Planet-buster.** A beam scars a continent. | Nearby tiles are destroyed in detail. Distant regions are marked devastated and generate as wasteland when visited. |
| S9 | **Rooftop to orbit.** A flyer leaves a roof, climbs past the clouds into vacuum, boards a ship, and the fight continues inside it on the way to another world. | No loading-screen seams in the rules: one position system, one timeline. The ship's interior stays playable in transit. |
| S10 | **Time stop.** A time-stopper freezes a street. Thrown knives hang in the air and falling debris pauses. | Time is a property of places and things, and can be scaled or stopped per area. |
| S11 | **Thrown building.** A titan tears the top floors off a tower and throws them. | A piece of the world can become a single moving object and become world again on impact. |
| S12 | **A mod did that.** Everything above is defined in JSON. | A modder adds a new power, material, hazard and cutscene without touching Rust. |

## 4. Decisions already made with the project owner

| Topic | Decision |
|-------|----------|
| Language | Rust |
| Display | Glyph grid in its own window first; plain terminal frontend as a reduced-effects fallback |
| Time | Shared timeline with action phases; pauses for decisions |
| Power ceiling | Planet-buster. Tile-accurate destruction nearby, coarse destruction far away |
| Off-screen world | Layered simulation: full detail near the player, abstract elsewhere |
| Vertical space | One z-level per storey (3 m), plus an exact height for everything that moves |
| Bodies | Anatomical and data-defined: parts, tissue layers, materials, wounds, severing |
| Mod code | JSON and EOC first, rich enough to hold the base game's rules. A slot for sandboxed scripting is reserved but nothing depends on it |

The reasoning and consequences of each are recorded in [decisions.md](decisions.md).

## 5. Non-goals

- Real-time play. Presentation may animate, the simulation never waits on a
  wall clock.
- Multiplayer in the first version. Determinism keeps lockstep co-op possible
  later; nothing else is done for it now.
- True 3D rendering, or cubic 1 m voxels.
- Rigid-body rotation, joints, or accurate fluid dynamics. Physics is
  impulse-based and grid-quantised, chosen to be legible and fast.
- Graphical tilesets in the first version. The glyph renderer should not
  prevent them later.
- A fixed setting. The engine is setting-agnostic; the first content mod will
  be a modern city with supers because it exercises the most systems.
