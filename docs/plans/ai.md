# Plan — AI and the Layered Simulation

Implements [09-ai-and-simulation.md](../09-ai-and-simulation.md).

Packages prefixed W are in [world-model.md](world-model.md); X, K and T in
[sim-kernel.md](sim-kernel.md); E, S and U in
[first-playable.md](first-playable.md); C in [combat.md](combat.md); G in
[devices.md](devices.md); A in [abilities.md](abilities.md); V in
[vehicles.md](vehicles.md). The rules for taking a package are those of
[world-model.md](world-model.md) and [CLAUDE.md](../../CLAUDE.md).

Packages prefixed I concern individual characters. Packages prefixed L
concern the layered simulation. The two tracks meet at L2.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| I1 | Navigation | L | W18, E10 | beta |
| I2 | Flow fields and steering | M | I1 | todo |
| I3 | Minds and goals | L | S5, K5 | beta |
| I4 | Methods and steps | L | I3, I1, T4 | beta |
| I5 | Options, threat, positions, reactions | L | I4, C11, A3 | beta (reactions subset) |
| I6 | Work, hauling, stations | M | I4, G8 | todo |
| I7 | Collectives, orders, formations | M | I4, I2, S7 | todo |
| I8 | Warm stepping and bulk exchanges | M | I4, T7, C11 | todo |
| I9 | Explaining | S | I5, K10 | todo |
| I10 | Bundled minds | M | I5, I6, C12 | beta |
| L1 | Cold records, scheduling, calendars | M | E11, K7 | beta |
| L2 | Routines, long goals, coming and going | L | L1, I4 | beta |
| L3 | Incidents | L | L2, W17 | todo |
| L4 | Knowledge stores and spreading | M | L1, S8 | todo |
| L5 | Populations, stocks, shipments, services | M | L1, G4, W21 | todo |
| L6 | Weather | M | L1, S1, W16 | todo |
| L7 | Devastation and battles | M | L3, I8 | todo |
| L8 | Scale scenarios and budget gate | M | I8, L7, then recurring | todo |

## Order and parallelism

| Wave | Individuals | Layered simulation |
|------|-------------|--------------------|
| 1 | I1, I3 | L1 |
| 2 | I2, I4 | L4, L5, L6 |
| 3 | I5, I6, I7, I8 | L2 |
| 4 | I9, I10 | L3 |
| 5 | — | L7 |
| 6 | — | L8 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M4h Minds** | I1–I6, I9, I10 | A watchman patrols, hears something, goes to look, and returns. He takes cover when shot at. Shown a blade breaking on someone's skin, he runs. A helper takes a building job, fetches bricks from the shelf, and builds. Every one of those choices can be opened in the inspector and read. A path is rerouted the moment a wall falls across it. |
| **M4i Many** | I7, I8 | A company forms a line, advances by drum, and fires on order. Kill the drummer and the line stops receiving orders. A battle of six hundred runs with the player in the middle of it. |
| **M4j Meanwhile** | L1–L7 | A watchman is at his post by day, in the tavern at dusk, in bed at night, and is met on the street between them. Across town a robbery happens while the player is elsewhere: on arrival there is broken glass, a body, people who saw someone, and a watch that is looking for that someone. Arriving halfway finds it in progress. A storm rolls in and the sea rises. |

## Individuals

### I1 — Navigation

**Spec** 09 §5.1–5.4 · **Size** L

Build
- Profiles from movement modes and size.
- Chunk regions per profile, rebuilt lazily on structure revision; region
  and tract graphs; links for stairs, ramps, ladders, portals and
  stationary subgrids.
- Hierarchical search; cell search within regions; costs for doors, known
  hazards and creatures; clearance for large movers; the flier fallback.
- Exclusion of `hidden` elements a character has not perceived.

Done when
- Routes equal those of a brute-force search on random maps for each
  profile, within a stated tolerance of optimal (property test).
- After any edit, structures consulted agree with geometry (invariant N3).
- Scenario `wall_falls_reroute`: a route in use is rechecked and replaced
  when a collapse blocks it.
- Budgets of 09 §19 for paths and region rebuilds are benchmarked.

Beta (09 §5.4a): paths are searched cell by cell over belief, with costs
for doors, creatures seen and remembered hazards and room for wide
footprints; routes in use are rechecked when elements change; tested
against brute force on random walls, `wall_falls_reroute` in
`tests/ai.rs`, a path timed there. Stubbed: profiles beyond the mover's
own rules, chunk regions and tract graphs, links over portals and
subgrids, the flier's straight line, `hidden` elements beyond what
perception already excludes, region rebuild budgets.

### I2 — Flow fields and steering

**Spec** 09 §5.5, §5.6 · **Size** M

Build
- Flow fields over an area for a goal region and profile, cached and
  shared; following by movement actions.
- The crowd rules of 09 §5.5: waiting behind those going the same way,
  swapping with those who yield, failing a step after no progress.
- A waypoint-following controller for vehicles, where package V6a exists.

Done when
- Three hundred followers of one field each reach the goal, and one field
  was computed.
- A field is invalidated by the same revisions that invalidate regions.
- Scenario `through_the_gate`: a column of a hundred passes a doorway one
  cell wide, and two columns meeting head-on in a corridor two cells wide
  pass each other, without anyone stuck.

### I3 — Minds and goals

**Spec** 09 §3 · **Size** L

Build
- The `mind` and `goal` types; weights with trait adjustments.
- The mind context: what may be read, enforced at load.
- Scoring, `about` binding, `commit`, `interrupts`.
- Think scheduling: triggers, ordering, staggering, stretched intervals.

Done when
- A goal that reads another entity's state directly fails to load, naming
  the read (invariant N1).
- With equal inputs, the adopted goal is the same across runs and
  independent of other minds' think order (invariant N8).
- `commit` prevents flapping between two close goals.

Beta (09 §3.5): `mind` and `goal` with every field listed there, the
mind context enforced at load, `about` from tracks, memories or a place,
`commit`, `interrupts`, `noise`, think batches with triggers, stretched
and staggered intervals; all three "Done when" tested in `tests/ai.rs`.
Stubbed: `routine` and `tactics` on minds, the hook `goal_scoring`,
traits adjusting minds (an `adjust` condition stands in), the effects
`set_goal` and `clear_goal` (verbs for now), queries for `about`.

### I4 — Methods and steps

**Spec** 09 §4 · **Size** L

Build
- Every step kind of 09 §4, feeding plans; `done` and `fail`; failure
  penalties; coexistence with reaction rules.

Done when
- Scenario `patrol_and_investigate`: routine, a percept, a detour, a
  return.
- A failed step makes the mind think, and a repeatedly failing goal is set
  aside for a time.
- Every act of a character is an action, interaction or speech a player
  could have issued (invariant N2), shown by replaying a character's acts
  as commands and reaching the same state.

Beta (09 §4.1): the `method` type; the steps `go`, `do`, `choose`,
`wait`, `look`, `say`, `set`, `settle`, `stance`, `keep_near`,
`keep_from`; failure, fallback methods and goals set aside; the events
of 09 §17 that are listed as built. `patrol_and_investigate` is the
test script `scenarios/ai/investigate.scn`, beside the guard and
townsperson tests of `tests/ai.rs`. Interactions are done by a `do` step of the
interaction action. Stubbed: the `use` step as a step of its own, the
`job`, `goal`, `follow` and `run` steps (jobs, queries), paces of `go`,
minds opening doors in their way.

### I5 — Options, threat, positions, reactions

**Spec** 09 §6 · **Size** L

Build
- The `tactics` type; option gathering by intent across all grant sources;
  the option fields; scoring with noise.
- `threat`, `standing`, `severity`, `least_effort_that`; `best_reaction`.
- `pick_position` and its helpers.

Done when
- Scenario `flee_when_outmatched`: a character who has witnessed an attack
  it cannot survive adopts a fleeing goal; one who has not, does not.
- Scenario `use_cover`: under fire, a character moves to a cell with
  cover from the believed position of the shooter.
- Scenario `trickster`: a character with control and escape abilities and
  the weights for them uses those in preference to striking.
- A character judging a disguised threat by appearance is wrong in the
  expected direction.

Beta, reactions only (09 §6.6): threat read from memories and tracks,
stances standing in for relations, fleeing by `keep_from`, fighting back
by `choose` over action tags (the bundled guard strikes with the first
of combat's attacks tagged `strike` that can start), investigating what was heard and not
seen, giving up by `settle`; the bundled `townsperson`, `guard` and
`dog` minds. Stubbed: the `tactics` type, options and their fields,
`threat`, `standing`, `severity`, `least_effort_that`, `best_reaction`,
`pick_position` and its helpers, the scoring helpers; none of this
package's "Done when" is met yet.

### I6 — Work, hauling, stations

**Spec** 09 §7 · **Size** M

Build
- Job board queries, claims, the `job` step; hauling and storage jobs;
  station and bench jobs; orders raising a job's score.

Done when
- Scenario `haul_and_build`: two helpers build a room from a design,
  fetching inputs, without both claiming one job.
- Loose items are put away into the highest-priority storage that accepts
  them.
- An order to do a kind of work is followed ahead of other jobs and lapses
  when withdrawn.

### I7 — Collectives, orders, formations

**Spec** 09 §8 · **Size** M

Build
- The `collective` and `formation` types; group goals through the leader's
  beliefs and group knowledge; steps as orders; the obedience goal;
  formation places and keeping them.

Done when
- Scenario `line_holds`: a company forms, advances and fires on order.
- Scenario `drum_falls_silent`: with its signaller gone, members stop
  receiving orders and act on the last one.
- A member whose obedience goal is outscored breaks formation, and the
  collective learns of it only through perception.

### I8 — Warm stepping and bulk exchanges

**Spec** 09 §9 · **Size** M

Build
- The AI side of the warm stepper (03 §10): routes, routines and work
  advanced in bulk.
- Bulk exchanges between opposed warm characters from previews, applied to
  individuals by positional randomness.
- Transitions between hot and warm.
- The attention cap of 09 §9.3: ranking, the cap, dwell time, promotion
  taking a place under the cap.

Done when
- A warm character promoted to hot thinks at once and continues its
  purpose.
- Bulk exchange results over many runs match the mean of fully resolved
  exchanges for the same forces, within a stated tolerance.
- With more characters in hot tracts than the cap, exactly the cap are
  hot, they are the highest ranked by 09 §9.3, and physics on those tracts
  is unaffected.
- A character at the boundary does not change level more often than the
  dwell time allows.
- Scenario `six_hundred`: a battle wholly inside the hot radius runs with
  the cap's number hot and the rest warm, within the budgets of 09 §19.

### I9 — Explaining

**Spec** 09 §16 · **Size** S

Build
- Data for each view of 09 §16, exposed to the inspector and to scenario
  assertions.

Done when
- A scenario can assert which goal a character holds and why the last step
  failed.

### I10 — Bundled minds

**Spec** 09 §3–§8 · **Size** M

Build
- In the `core` mod: minds, goals and tactics for a bystander, a guard, a
  brawler, a worker, a beast and a predator; routines for each; reaction
  rules that hand over to goals.

Done when
- A small place populated only with bundled minds behaves plausibly through
  a day and through a disturbance, by scripted checks.
- Removing `core` leaves the engine able to run minds defined otherwise.

Beta (decisions D-784, D-786): in `mods/core/data/ai/`, `routines/` and
`town/`, a townsperson keeping `townsfolk_day`, a shopkeeper, a watchman,
a brawler, a house dog, the trades' people of the furnished town, and
the goals of a day (`keep_routine`, `linger`, `keep_shop`, `stand_post`,
`sleep`, `answer_call`, `pick_fight`, `follow_owner`,
`bark_at_stranger`); the settlement's data gives them to its residents
by role. A day and a disturbance in the town are checked in
`crates/clash-save/tests/town_life.rs` and the test scripts
`town/routine_day` and `town/town_disturbance`. Stubbed: tactics (none
of the minds has any), the worker and the predator, the "twice running
from different seeds" scripts for each mind.

## Layered simulation

### L1 — Cold records, scheduling, calendars

**Spec** 09 §10.1, §13.1, §14.2 · **Size** M

Build
- `AgentState`, population counts, site and region statistics; wake-up
  scheduling for the cold layer; the `calendar` field and its properties.

Done when
- Ten thousand idle agents schedule nothing beyond their next wake-up
  (invariant N7).
- Time of day follows the realm's clock rate.

Beta (decisions D-782, D-785), in `clash_sim::agent`: `AgentState`, one
schedule and one timeline wake-up for every cold resident, the
`calendar` type and the realm's `time_of_day`, `day`, `day_length`,
`day_part` and `season`; tested in `crates/clash-sim/tests/agents.rs`.
Stubbed: population counts, site and region statistics (a town's
residents are its population).

### L2 — Routines, long goals, coming and going

**Spec** 09 §10.2–10.5 · **Size** L

Build
- The `routine` type; routine wake-ups and travel time.
- Cold methods and steps.
- Loading and unloading: placement from activity; goal from routine or
  aim; storing aim and activity; folding and the regional cap.

Done when
- Scenario `routine_day`: an agent is found at the right place for each
  part of the day, and on the street between places at the right times.
- Scenario `pursuit_continues`: a character chasing another when unloaded
  is still pursuing as an agent, and resumes on loading.
- A notable individual is in exactly one form at all times
  (invariant N4).

Beta (decisions D-781, D-786, D-787): the `routine` type with places by
role, chance, jitter, beats and travel time; routine wake-ups; a loaded
resident's goal from its routine, its doors opened on the way; an
unloaded one placed by its routine when its tract loads, at its place or
on the street between; a disturbance overriding the routine through the
mind's goals. `routine_day` holds in `crates/clash-save/tests/town_life.rs`
and as a test script. Stubbed: cold methods and long goals,
`pursuit_continues` (an agent's `aim` is kept and not acted on), folding
and the regional cap, walking out of view toward an unloaded place.

### L3 — Incidents

**Spec** 09 §11.1–11.5 · **Size** L

Build
- The `incident` type; occurrence by rate without polling; starting by
  steps, collectives and rules; cast selection.
- Stages; live play with goals; abstract settlement with the cold effects;
  `strength`.
- Changing between live and abstract in both directions; responders;
  chains.

Done when
- Each stage is settled exactly once (invariant N5).
- Scenario `meanwhile`: an abstract robbery and response leave stock
  moved, casualties, local knowledge, and pending edits that are present on
  loading (invariant N6).
- Scenario `arrive_midway`: loading during the second stage finds the cast
  placed and acting, and the first stage's results in the world.
- Scenario `leave_midway`: unloading during a live stage settles the rest
  abstractly from the survivors.
- Observed occurrences over a long run match the rate.

### L4 — Knowledge stores and spreading

**Spec** 09 §12 · **Size** M

Build
- Local knowledge on sites and tracts; the `channel` type; the knowledge
  verbs; drawing on local knowledge when instantiating anonymous
  individuals.

Done when
- Anonymous individuals in a place where something happened hold memories
  of it in proportion to salience and recency.
- Knowledge added at a site reaches a group and a channel by content rules
  with their delays, losing confidence at each hop.

### L5 — Populations, stocks, shipments, services

**Spec** 09 §13 · **Size** M

Build
- Population changes and `migrate`; site stocks with rates; shipments as
  travelers; `price` as a content function; services set by rules and by
  supplying sites.

Done when
- Stock levels are analytic between changes.
- A shipment intercepted on its route does not arrive.
- Scenario `power_station_down`: disabling the supplying site switches the
  region's service off and every network fed by it goes unsupplied.

### L6 — Weather

**Spec** 09 §14.1 · **Size** M

Build
- The `weather` type; state per region and its stepping; the values it
  sets; fields laid on loaded open cells; `set_weather`.

Done when
- State frequencies over a long run match the transition chances.
- Scenario `storm_at_sea`: a storm lowers light, raises wind and sea state,
  and a floating vessel's handling changes by its model.

### L7 — Devastation and battles

**Spec** 09 §11.6, §15 · **Size** M

Build
- Bundled rules answering `region_devastated`; battle incidents between
  groups with collectives live and strength contests abstract.

Done when
- A stamp over a populated region reduces populations and wounds agents in
  proportion to devastation, once.
- Scenario `distant_battle`: an unloaded battle ends with casualties, a
  moved line and stamped ground; walking into it mid-course finds both
  sides formed and fighting.

### L8 — Scale scenarios and budget gate

**Spec** 09 §19 · **Size** M, recurring

Build
- Benchmarks for each row of 09 §19; a simulated day for twenty thousand
  agents; the six-hundred battle; measured figures written back.

## Scenario catalogue

Under `scenarios/ai/`.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `wall_falls_reroute` | I1 | S2 | Navigation follows destruction |
| `patrol_and_investigate` | I4 | S13 | Goals, steps, percepts |
| `flee_when_outmatched` | I5 | S14 | Threat from belief |
| `use_cover` | I5 | S19 | Positions |
| `trickster` | I5 | S15 | Non-harmful options chosen |
| `haul_and_build` | I6 | S24 | Jobs, claims, hauling |
| `line_holds` | I7 | S19 | Collective, formation, orders |
| `drum_falls_silent` | I7 | S19 | Orders must be heard |
| `through_the_gate` | I2 | S19 | Crowds that do not jam |
| `six_hundred` | I8 | S19 | Attention cap and bulk exchanges at scale |
| `routine_day` | L2 | S7 | Agents in the right place |
| `pursuit_continues` | L2 | — | Purpose survives unloading |
| `meanwhile` | L3 | S7 | Abstract incident and what it leaves |
| `arrive_midway` | L3 | S7 | Abstract to live |
| `leave_midway` | L3 | — | Live to abstract |
| `power_station_down` | L5 | S7 | Services |
| `storm_at_sea` | L6 | S22 | Weather into vehicles |
| `distant_battle` | L7 | S19 | Battles in both forms |
| `parked` | I2 | — | Berths |
| `homestead` | L5 | — | Founding a site |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Berths | I2 | 09 §5.6; 10 §3.2 | Test script `parked`: an AI driver reaches the start of a berth's approach line, follows it and stops in the berth, then leaves by it in reverse. With no berth it stops beside the destination. Until worldgen package WG1 exists the berth is supplied by a test fixture. |
| Scoring helpers | I5 | 09 §6.1 | `threat_removed`, `harm_prevented`, `escape_margin` and `detection_risk` are registered, read belief only (refused in a load test if given truth), and each has a unit test against a hand-computed case. |
| Threat from observed capabilities | I5 | 09 §6.2; 15 §7.1 | `threat` rises after the character sees an attack it had not seen before and is unchanged by one it cannot have perceived. |
| Formations of vehicles | I7 | 09 §8; 11 §7 | A collective whose members are vehicles holds a formation behind its leader. Completed with vehicles package V7. |
| Consistent, not clever | I10 | 09 §1 | Each bundled mind's behaviour is stated in a sentence or two in its documentation, and a test script shows it doing that twice running from different seeds, differing only where its rules draw chance. No bundled mind reads anything that singles out the player. |
| Usual guard in bulk exchanges | I8 | 09 §9.1 | Bulk previews include the defences each side's representative members have ready. A shielded line loses fewer to a volley than an unshielded one of the same number. |
| Holdings | L5 | 09 §13.4; 16 §5.6 | A site's stock of an account resource changes at its stated rate while unloaded; a standing payment from the site to its holder moves that share; changing the holder redirects it. Completed with devices packages G13 and G14. |
| Region temperature and cover | L6 | 09 §14.1 | Temperature equals the mean plus the seasonal, daily and state terms at any tick, and the time it will cross a threshold is computed, not found by stepping. Snow laid over a day while a tract was unloaded is on its ground when it loads, and is gone after a thaw. |
| Kinds of devastation; reference disasters | L7 | 09 §15; 02 §17.2 | Cold rules receive the kind. The reference earthquake, flood, wildfire, eruption, storm and falling star each run live and abstractly, and pass `incident compare` (14 §11) on casualties and devastation. |
| Founding a site | L5 | 09 §13.5 | Test script `homestead`: a site founded over a hand-built camp has a job board its members use, is chosen by an incident that matches its tags, and after a day unloaded shows the stock its machines made, less what a shipment took. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
