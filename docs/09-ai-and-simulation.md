# 09 — AI and the Layered Simulation

Status: design complete, ready for implementation.
Implementation plan: [plans/ai.md](plans/ai.md).
Serves scenes S7, S13, S14, S16, S19 and S24 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how characters decide what to do, find their way,
choose how to fight and how not to, do work, and act together; and how the
world carries on where nobody is looking: agents with routines, populations
as numbers, incidents that happen for reasons and leave real results,
knowledge that spreads, stocks, services and weather.

It builds on beliefs (15), plans of actions (03 §5.3), previews (07 §13),
ability hints (08 §11), jobs (16 §10.2), and the three forms of individual
(06 §12).

It does not specify how incidents are paced for the player's benefit, or
scenario scripting; that is document 13.

Fixed from earlier documents:

- Characters act on belief (01 R11). Static geometry is common knowledge;
  nothing else is (D-074).
- Characters act through the same actions, with the same costs and timing,
  as the player.
- Nothing is scaled for fairness (D-034). A character who is outmatched
  should be able to tell, and to leave.

Names of minds, goals and incidents in examples are illustrative content.
The engine has no "guard" and no "robbery" (01 R10).

**The aim is consistency, not cleverness.** A character behaves by rules
that follow from who it is, what it believes and how it feels, with a
little chance in them. A player should be able to learn how a kind of
character behaves and then rely on it: the guard who comes to look, loses
the trail and goes back to his post; the squad that calls out, moves to
the flank and falls back when it is losing.

Characters will sometimes do things that look smart, because sound rules
in the right situation do. They do not reason about the player, lay
schemes or build deceptions over time, even when they hold powers that
could, and nothing in this design tries to make them. A scheme in a story
is authored: as a plot (13 §6), an incident (section 11) or a method
(section 4).

## 2. The picture

Every character that decides has a **mind**, and a mind has **goals**. Each
goal carries an expression that scores how much the character wants it
right now, from what the character believes, needs and feels. The mind
pursues the highest. A goal knows **methods**: sequences of steps (go
there, do this, say that, work that job) that become the character's plan
of actions. When the world gets in the way, the method fails and the mind
thinks again.

Inside a method, moment-to-moment choices are made by **scoring options**:
every action available, against every sensible target, judged by what it
is expected to achieve and what it risks. Harm is judged by preview. So are
escape, control and trickery, by what their authors say they are worth.

Characters reach places by **path-finding** over the world's real geometry,
which changes when walls fall. Many characters going the same way share one
**flow field**.

A **group** can have a mind too. It decides for the group and issues
orders, which have to be heard.

All of that is for characters near enough to matter. Further off,
characters are stepped a second at a time with no fine detail. Beyond that
they are not in the world at all: an **agent** is a stored character with a
routine and long-term goals, and everyone else is a **population** count.
Things still happen there. An **incident** is something that occurs for a
reason, with a cast and stages. Where the player can see it, it is played
out by real characters. Elsewhere it is resolved in a few lines of
arithmetic, and leaves its results as changes to places, people and what
people know. Either way it happened.

## 3. Minds

### 3.1 Definition

```json
{ "type": "mind", "id": "watchman",
  "goals": ["stand_post", "patrol", "investigate", "apprehend", "fight", "flee", "report", "rest"],
  "weights": { "duty": 0.9, "caution": 0.4, "mercy": 0.6 },
  "think": "500 ms",
  "routine": "watch_shift",
  "tactics": "soldierly" }
```

A mind definition is a profile: which goals it can have, numbers that
colour its choices, how often it reconsiders, its routine (section 10.2),
and its tactics (section 6). Traits may add goals and adjust weights, so a
frightened or enraged character is the same mind with different numbers.

### 3.2 Goals

```json
{ "type": "goal", "id": "investigate",
  "score": "weight('duty') * $about.salience * (1 - self.var.fear)",
  "about": { "first": { "memories": { "of": "self",
               "where": "it.kind == 'disturbance' and not it.settled", "sort": "-it.salience" } } },
  "methods": [
    { "if": "dist(self, $about.where) > 3",
      "steps": [ { "go": { "to": "$about.where", "pace": "walk" } } ] },
    { "steps": [ { "do": "search_area", "with": { "at": "$about.where" } },
                 { "set": { "$about.settled": true } } ] } ],
  "done": "$about == null or $about.settled",
  "commit": 0.25,
  "interrupts": true }
```

| Field | Meaning |
|-------|---------|
| `score` | How much the character wants this now. Zero or below means not at all. |
| `about` | What the goal concerns, bound for the rest of the definition: a memory, a tracked persona, a job, a place |
| `methods` | Ways to pursue it; the first whose condition holds is used (section 4) |
| `done`, `fail` | When it is over, one way or the other |
| `commit` | How much another goal must outscore this one, as a fraction, to displace it once chosen |
| `interrupts` | Whether choosing it cancels what the character is doing, or waits for the current action to end |

### 3.3 Choosing

When a mind thinks, it scores its goals and adopts the highest, unless the
current goal is within its `commit` margin of the best. Then it runs a
method.

A mind thinks:

- when its plan runs out, or a step fails;
- when it perceives something (15 §6) whose salience passes a threshold set
  on the mind;
- when a rule tells it to (`think`);
- when its `think` interval has passed since it last did.

Thinking is spread out. Minds due at the same time are served in order of
tick, then entity id, and minds far from attention may have their intervals
stretched (section 9).

### 3.4 What a mind may read

Goals, methods and tactics are compiled in a **mind context**. In it:

| Available | As |
|-----------|-----|
| The character's own state | `self`, in full |
| What it believes | Tracks, memories, relations, group knowledge (15) |
| What it currently perceives | The `visible` and `heard` queries |
| Static geometry | The world, except elements tagged `hidden` and not yet perceived |
| Settings and definitions | As anywhere |

Reading anything else about another entity is a load error. A goal cannot
say `target.loc`; it must say `self.believed_loc(target)` and accept that
the answer may be stale or `null`. This is rule R11 enforced by the
compiler.

## 4. Methods and steps

A method is a list of **steps**. Steps are fed into the character's plan
(03 §5.3) as the actions they stand for.

| Step | Becomes |
|------|---------|
| `go` | Path-finding (section 5) and a sequence of movement actions (06 §11), with a pace and how close is close enough |
| `do` | An action, with parameters; missing ones take their defaults |
| `choose` | An action chosen by scoring options (section 6) under a stated purpose |
| `use` | An interaction (16 §3) with something |
| `say` | Speech (15 §11) |
| `job` | Working a job to completion or interruption (section 7) |
| `follow`, `keep_near`, `keep_from` | Staying in a relation to someone as they are believed to move |
| `wait` | Until a condition, or for a time |
| `goal` | Another goal, pursued as part of this one |
| `set`, `run` | Bookkeeping on the character's own variables and beliefs |

A step fails when its action is refused, interrupted or cannot find its
target. The method stops, and the mind thinks. Goals that keep failing are
penalised for a while so a character does not walk into the same locked
door all day.

Reactions written as rules (15 §12) and goals coexist. A rule that reacts
to a scream by planning an action is immediate and authored. A goal whose
score rises because of the memory of the scream is deliberate. Most
characters have both.

## 5. Navigation

### 5.1 Profiles

A **profile** says what a mover can cross: which movement modes it has
(06 §11), its size, what it can step up or squeeze under, whether it opens
doors, climbs, swims or flies. Profiles are derived from the character and
shared between characters that move alike.

### 5.2 Structure

Path-finding is hierarchical.

| Level | Nodes | Built from |
|-------|-------|-----------|
| Tract | Tracts, joined where their borders are passable | Tract records and realized geometry |
| Region | Connected patches of passable cells within a chunk, per profile | Chunk geometry; rebuilt when the chunk's structure revision changes |
| Cell | Cells | Searched directly, only within the regions on the route |

A search finds a route over regions, then cells within the next few
regions. Stairs, ramps, ladders, portals (02 §19) and stationary subgrids
(02 §18) are links.

When a wall comes down or a floor goes, only the chunks whose revision
changed are rebuilt, and only when next asked about. Routes in use that
passed through them are rechecked.

### 5.3 What costs

- Doors cost the time to open them. A character who believes a door locked
  routes round it or brings a key.
- Fields and hazards a character knows of add cost by its weights.
- Other creatures are obstacles to step round, not walls.
- Light and observers add cost for a character trying not to be seen
  (section 6.4).

### 5.4 Fliers and large movers

Fliers try a straight line first, checked by ray, and fall back to a search
over regions of open cells. Movers larger than a cell use clearance kept
with each region.

### 5.5 Flow fields

When many characters are going to the same place, one **flow field** is
computed for it: for each cell in the area, which way to step. A formation
of three hundred asks for one field, not three hundred paths.

**Crowds.** One creature blocks a cell (06 §11), so a column at a doorway
could jam. Three rules prevent it:

- A mover blocked by someone going the same way waits. It does not reroute.
- A mover blocked by someone standing who **yields** to it swaps places
  with them in one step (06 §11). Members of one formation yield to each
  other; rules decide who else does.
- A mover that has made no progress for several steps fails its step, and
  its mind thinks again (section 4).

Warm characters do not block one another. Each bulk step places them by
formation, or in the nearest free cell.

### 5.6 Vehicles

A vehicle is steered toward waypoints by a controller that sets throttle
and helm (11 §7) from its performance. Waypoints come from the tract-level
route and a profile for its motion model: roads and open ground, water,
air.

The controller does not manoeuvre in tight places. Maps mark **berths**
instead: a parking bay, a mooring, a landing pad, a dock, each with an
approach line (10 §3.2). A driver heads for the start of the approach line,
follows it, and stops in the berth. Leaving is the same in reverse. A place
with no berth is somewhere an AI driver stops beside, not in.

### 5.7 Long distance

Beyond loaded space there is nothing to path over cell by cell. A character
going further becomes a traveler (02 §20; 11 §13) and follows a route over
tract records.

## 6. Choosing actions

### 6.1 Options

A `choose` step names a **purpose**: harm this one, get away, stop them
without hurting them, help that one. The engine gathers every action the
character could start now (grants from abilities, traits, items and
modules, movement, interactions in reach) whose `intent` (08 §11) serves
the purpose, with the targets and positions that make sense for each.

Each option is scored by the character's **tactics**:

```json
{ "type": "tactics", "id": "soldierly",
  "harm":    "it.preview.odds * severity(it.preview) - weight('caution') * it.exposure",
  "protect": "it.preview_incoming.reduction",
  "escape":  "it.distance_gained / (1 + it.time)",
  "control": "it.value",
  "aid":     "it.value",
  "effort":  "least_effort_that(self, it, 'disables')" }
```

`it` is the option. What the engine computes for it:

| Field | Is |
|-------|----|
| `preview` | `strike_preview` (07 §13) for attacks, from what the character knows of the target |
| `value` | The grant's own scoring function, for abilities previews cannot judge |
| `time` | How long before it takes effect |
| `exposure` | How much the character is expected to suffer while committed to it, from what it believes its enemies can do |
| `cost` | What it spends, as a fraction of what is left |
| `distance_gained`, `cover_after` | For movement: where it leaves the character |

The highest score is taken. A margin of noise, set in the tactics, keeps
characters from being perfectly predictable.

Because trickery and control are options scored on the same footing as
blows, a character whose `value` functions and weights favour them will
blind, bind, frighten, deceive and run, as readily as another will hit.

Four helpers do the arithmetic most `value` functions need. Each reads only
what the character believes.

| Function | Gives |
|----------|-------|
| `threat_removed(self, x, time)` | How much of `threat(self, x)` goes away if `x` cannot act for that long |
| `harm_prevented(self, who, time)` | The harm `who` is expected to take over that time from the enemies `self` knows of |
| `escape_margin(self, loc)` | Time by which `self` reaches `loc` ahead of the fastest pursuer it knows of |
| `detection_risk(self, loc)` | The chance, from 0 to 1, that someone `self` knows of perceives it at `loc` |

### 6.2 Knowing when one is outmatched

`threat(self, x)` is how much harm `self` expects `x` could do to it: the
best preview of what `self` has seen or been told `x` can do, against
`self`'s own body and gear. What it has seen and been told is the bounded
list of observed capabilities on its track of `x` (15 §7.1). `standing(self)` is how much fight `self` has
left, from its functions and pools.

These feed the scores of goals such as fleeing, surrendering and calling
for help. An ordinary watchman who has just seen a blade shatter on
someone's skin has a `threat` he can do nothing about, and a mind with any
`caution` at all will act on that. This is decision D-034 from the other
side.

A character who has never seen what a stranger can do judges by
appearance, and may be badly wrong.

### 6.3 Reacting

When a character notices a window (03 §7), the bundled rules ask the
engine for `best_reaction`: the available reaction that would be ready in
time and most reduces the previewed harm, by its tactics. A character's
stance (07 §12) may forbid or prefer some.

### 6.4 Positions

`pick_position` scores cells within a radius by an expression. Helpers
available on a cell:

| Helper | Gives |
|--------|-------|
| `exposed_to(x)` | How much of a character standing there `x` could see or shoot, from where `x` is believed to be |
| `cover_from(x)` | The reverse |
| `reach_of(x)` | Whether `x` could strike it |
| `light`, `noise_floor` | For hiding |
| `path_cost` | How far it is to walk |
| `near(tag)` | Distance to the nearest known thing of a kind: an exit, an ally, a weapon |

Taking cover, flanking, keeping range, lurking in shadow and standing in a
doorway are expressions over these.

### 6.5 Restraint

Effort (07 §3.3) is chosen by the tactics' `effort` expression. A character
may hit as hard as it takes to stop someone and no harder, or as hard as it
can. Which, is its mind's weights.

## 7. Working

### 7.1 Taking jobs

A character with a working goal asks the **job board** of the sites it
belongs to for a job (16 §10.2).

- Jobs are filtered by what the character may do (roles, permissions,
  `needs`) and scored by priority, distance and the character's own
  preferences for kinds of work.
- A job is **claimed** by whoever takes it, up to the number of workers it
  admits, so helpers do not all walk to the same wall.
- The `job` step goes to the job, does the work (16 §8.2), and releases the
  claim on finishing or giving up.

### 7.2 Supplying jobs

A job short of inputs raises **hauling** jobs: fetch this from where it is
known to be, bring it here. Items lying outside any storage area that
accepts them raise jobs to put them away. Storage areas (16 §10.4) are
searched in order of their priority.

### 7.3 Stations

A station (11 §7) wanting an operator is a job for the crew's roles. So is
a workbench with a queued recipe, a sickbed, a watch post.

### 7.4 Orders

An order (15 §10) to work a job, or a kind of job, raises that job's score
above the rest for as long as the order stands. The player's helpers and a
foreman's crew are managed the same way.

## 8. Acting together

### 8.1 Collectives

```json
{ "type": "collective", "id": "line_company",
  "leader": "captain",
  "goals": ["hold_line", "advance", "volley", "withdraw"],
  "formation": "line_two_deep",
  "signals": ["drum", "shout"] }
```

A group (15 §9.2) may have a **collective**: a mind for the group. It has
goals and methods like any mind, read through the eyes of its leader: what
the collective believes is what the leader and the group's knowledge hold.

A collective's steps are **orders** to roles or members. An order reaches
those who hear it (15 §10). A company whose captain is dead, or whose drum
is silent, has members acting on their own goals with the last order they
received. Command can be lost.

Members obey through an ordinary goal, "do what I was ordered", whose score
depends on their relation to the one who ordered, their fear and their
state. Routing is that goal losing to another.

### 8.2 Formations

A **formation** is a set of places relative to an anchor and a facing. Each
member is assigned a place and keeps it by a `keep_near` step. A formation
moves by moving its anchor; members follow by flow field.

Formations are data: a line, a column, a ring round someone, a boarding
party's stack on a door.

### 8.3 Small groups

Two friends, a gang, a pack of animals need no collective. Shared
knowledge, relations and goals that read where allies are believed to be
are enough for them to gather, help each other and scatter.

## 9. Detail

| Level | Individuals | Deciding | Fighting |
|-------|-------------|----------|----------|
| **Hot** | In the world | As sections 3 to 8 | Fully resolved (07) |
| **Warm** | In the world | Routine and orders only; no tactics | In bulk |
| **Cold** | Agents and populations | Routines and long goals (section 10) | As incidents (section 11) |

### 9.1 Warm

Every `WARM_STEP` (03 §10) each warm character is advanced by that much
time: moved along its route, its work progressed, its routine followed.
It perceives only loud things (15 §14).

Warm characters who are opposed and in reach of each other exchange harm
**in bulk**: expected results from previews, for representative members of
each side, times numbers and time, applied to individuals chosen by
positional randomness. Bodies fall, the wounded are wounded, lines thin,
and none of it is resolved blow by blow. The previews assume each side's
usual guard: the defences its representative members have ready, at their
ordinary effort.

A battle of six hundred is, by the cap of section 9.3, about a hundred and
fifty hot characters around the player and the rest warm in formation,
trading volleys by arithmetic. Wherever the player goes, those nearby
become hot.

### 9.2 Moving between levels

| From | To | When | What happens |
|------|----|------|--------------|
| Warm | Hot | Its tract becomes hot, or a hot character involves it (03 §10) | It thinks at once |
| Hot | Warm | Its tract becomes warm | Running action finishes; plan kept as a route and a purpose |
| Loaded | Cold | Its tract unloads | Section 10.4 |
| Cold | Loaded | Its tract loads | Section 10.4 |

### 9.3 Attention

A tract's level is set by distance (02 §10), and a whole battlefield can
lie inside the radius that is hot. So the number of characters simulated
in full is capped separately.

`sim.max_hot_minds` (default 150) is the most characters that are hot at
once. When more than that stand in hot tracts they are ranked by
**attention**, and those beyond the cap are stepped as warm.

Attention, highest first:

1. Directly controlled characters, and any character holding a hot ticket
   of its own.
2. Characters sharing an action, a window or a grip with one of those.
3. Notable characters (06 §12.2), nearest first.
4. Anonymous characters, nearest first.

Nearest means nearest to a directly controlled character, or failing that
to the anchor of the nearest hot ticket. Within a class, characters the
party currently perceives (15 §13) come before those it does not.

- The ranking is redone at each warm step.
- A character that changes level keeps it for a few seconds, so nobody
  flickers at the boundary.
- A character promoted by involvement (03 §10) takes a place under the cap,
  and the lowest-ranked hot character is stepped down for it.

The cap concerns minds and fine-grained actions only. Physics does not have
levels: bodies in flight, collapses, fields and energy releases run in full
on every loaded tract, whoever is standing in them.

In a quiet town of eighty everyone is hot. In a duel both are. In a battle
the hundred and fifty nearest and most involved are, and that set moves
with the player.

## 10. Agents

### 10.1 What is kept

Everything notable that is not loaded is an **agent** (06 §12): a stored
entity, plus

```rust
pub struct AgentState {
    pub at: AgentPlace,            // a tract, a site, or travelling (02 §20)
    pub doing: Activity,           // what, where, since when, until when
    pub aim: Option<GoalId>,       // its current long goal
    pub wake: WakeId,              // when it next needs attention
}
```

An agent costs nothing between wake-ups.

### 10.2 Routines

```json
{ "type": "routine", "id": "watch_shift",
  "day": [
    { "from": "06:00", "at": { "site_role": "post" },  "doing": "on_watch",  "goal": "stand_post" },
    { "from": "18:00", "at": { "site_tag": "tavern" }, "doing": "at_leisure", "goal": "relax" },
    { "from": "22:00", "at": { "home": true },         "doing": "asleep",     "goal": "rest" } ] }
```

A **routine** says where a character is and what it is doing at each time
of day (section 14.2), and which goal that is when the character is
loaded. An agent's wake-up is the next boundary in its routine. Moving
between places takes the travel time between them.

### 10.3 Long goals

A goal may have **cold methods**: steps that work on records.

| Cold step | Does |
|-----------|------|
| `be_at` | Move to a site or tract, taking the time it takes |
| `spend` | Pass time on an activity, with effects at the end |
| `take_part` | Join or start an incident (section 11) |
| `obtain`, `deliver` | Move stock between a site and itself |
| `meet` | Be where another agent is, when they are |
| `learn` | Draw on a knowledge store (section 12) |

A thief with a goal to grow rich, a watch captain with one to keep the
peace, and a wandering swordsman with one to find a worthy opponent pursue
them while unloaded by these steps, and by the ordinary methods when
someone is there to see.

### 10.4 Coming into view, and leaving it

**Loading.** An agent whose place is in a tract being loaded becomes an
individual (06 §12). Where exactly follows from what it is doing: at its
post, in its bed, at a table, or on the street partway along its route if
it was between places. Its goal becomes the routine entry's goal, or the
hot form of its long goal, and it thinks.

**Unloading.** The individual is stored. What it was pursuing becomes its
`aim`; where it had got to becomes its `doing`. A watchman chasing someone
when the tract unloads is an agent in pursuit, not an agent who forgot.

The mapping is by goal: each goal declares what cold activity it leaves
behind, and each routine entry what goal it wakes as.

### 10.5 How many

Agents are kept while they matter. An agent with no goal beyond its
routine, no unusual state and nothing known about it by anyone notable may
be folded back into its population, leaving what it knew in the local
knowledge of its home (section 12). A cap per region applies, least
salient first.

## 11. Incidents

### 11.1 Definition

```json
{ "type": "incident", "id": "armed_robbery",
  "rate": "0.02 * site.var.wealth * site.region.var.lawlessness",
  "at": { "sites": { "where": "it.has_tag('bank') or it.has_tag('shop')" } },
  "cast": {
    "robbers": { "from": { "group_kind": "gang" }, "count": [2, 5], "else": { "creature": "thug" } },
    "staff":   { "from": { "site_population": true } },
    "guards":  { "from": { "site_role": "guard" } } },
  "stages": [
    { "id": "enter",  "time": "2 min",
      "live": { "robbers": { "goal": "seize_site" } } },
    { "id": "holdup", "time": "8 min",
      "live": { "robbers": { "goal": "loot_site" }, "staff": { "goal": "comply_or_flee" } },
      "abstract": [
        { "let": { "won": { "expr": "contest('street_fight', strength($robbers), strength($guards))" } } },
        { "if": "$won > 0",
          "then": [ { "move_stock": { "from": "site", "to": "$robbers", "what": "cash", "share": 0.6 } },
                    { "casualties": { "of": "$guards", "share": "0.2 + 0.1 * $won" } } ],
          "else": [ { "casualties": { "of": "$robbers", "share": 0.5 } },
                    { "end_incident": { "as": "foiled" } } ] },
        { "add_knowledge": { "to": "site.locals", "kind": "robbery", "roles": { "culprit": "$robbers" }, "salience": 0.9 } },
        { "pending_edit": { "at": "site", "map": "robbery_aftermath" } } ] },
    { "id": "escape", "time": "5 min",
      "live": { "robbers": { "goal": "escape_with_loot" } },
      "abstract": [ { "start_incident": { "id": "pursuit", "with": { "quarry": "$robbers" } } } ] } ],
  "responders": { "kinds": ["watch", "hero"], "hear": "alarm" } }
```

| Field | Meaning |
|-------|---------|
| `rate` | Expected occurrences per day for each eligible place. Absent for incidents that only something starts. |
| `at` | Where it can happen |
| `cast` | Who takes part, by role: drawn from agents, groups, the site's people, or created |
| `stages` | What happens, in order, each with a duration |
| `live` | For a stage: what each role's characters are given to do when it is played out |
| `abstract` | For a stage: effects that settle it when it is not |
| `responders` | Who may come once they know of it |

### 11.2 When incidents occur

- **By rate.** For each incident and eligible place, the time of the next
  occurrence is drawn from its rate and scheduled. Nothing is polled. When
  the rate's inputs change, the draw is redone.
- **By someone.** A cold step `take_part`, a collective's goal, a rule, or
  another incident starts one. A gang that needs money starts a robbery.
  The robbery was not random.
- **By the scenario or the event director.** Document 13.

### 11.3 Live or abstract

An incident whose place is loaded is **live**. Its cast are individuals:
agents made present, members of populations instantiated, others created.
Each is given its role's goal for the current stage. Then the simulation
runs. The stage ends when its time is up or its roles' goals are done or
failed, and the outcome is whatever happened.

An incident whose place is not loaded is **abstract**. Each stage's
`abstract` effects run when its time has passed. They work on records:

| Cold effect | Does |
|-------------|------|
| `casualties` | Kills or wounds a share of a cast role or population. Agents are wounded as individuals; populations by count. |
| `move_stock`, `set_stock` | Section 13.2 |
| `add_knowledge` | Section 12 |
| `pending_edit`, `stamp` | Changes the place for when it is next loaded (02 §17) |
| `set_service`, `set_site`, `set_region` | Services, tags, variables, and who holds a site |
| `set_agent` | An agent's aim, place, relations, variables |
| `start_incident`, `end_incident` | Chains and endings |

`strength(cast)` gives a cast's fighting power from previews of its
members, for use in contests. Content may supply its own.

### 11.4 Changing between the two

**Arriving.** If the place loads partway through, the incident becomes live
at its current stage. The cast are placed for that stage and take its
goals. What earlier stages did abstractly has already been written into
the place as pending edits and is there to be seen.

**Leaving.** If the place unloads while live, the surviving cast become
agents or return to their populations, and the remaining stages are
settled abstractly from where things stood.

The live and abstract forms of a stage are both written by the incident's
author and should agree on what is plausible. The engine guarantees only
that exactly one of them applies.

### 11.5 Responders

An incident that makes itself known (an alarm signal, a report reaching
group knowledge) may draw **responders**: agents and groups whose goals
accept incidents of its kind and who learn of it. They arrive after the
time it takes them, and join as a role. A fight between robbers and a
responder who can level buildings is the same incident with a larger
`strength` and, if its author says so, a `stamp`.

Scene S7 is one incident, a responder, a second stage that goes badly for
the street, and the knowledge it leaves.

### 11.6 Battles

A battle between forces that are not loaded is an incident whose cast are
groups. Its stages settle casualties from strength, move the line, and
stamp the ground. The same definition, live, gives each side's collective
its goal.

## 12. Knowledge

### 12.1 Stores

| Store | Holds what |
|-------|-----------|
| A mind's beliefs (15 §7) | That character knows |
| A group's knowledge (15 §9.2) | The group knows collectively |
| **Local knowledge** of a site or tract | People there know: what the anonymous saw |
| A **channel** | Has been published: a newspaper, a broadcast, a town crier, a rumour mill |

Local knowledge is what makes witnesses out of a crowd. When anonymous
individuals are instantiated in a place, each draws from its local
knowledge with a chance set by salience and how long ago. Someone on that
street saw it.

### 12.2 Spreading

Knowledge moves by rules listening for `knowledge_added`, with delays:
from a site's locals to the watch after a few minutes if anyone reports
it; from the watch to a channel; from a channel to every site that
receives it. Each hop may lose confidence and detail.

`add_knowledge`, `share_knowledge` and `publish` are the verbs. What
spreads where, how fast, and how distorted, is content.

### 12.3 What it is for

- A character asked what happened answers from what it holds (15 §11).
- A group acts on what it knows: who is wanted, who is owed.
- Agents learn of incidents and respond, flee, or take advantage.
- The player hears the news, sees the damage, finds marks whose causes the
  world recorded truthfully (02 §7.3), and questions people whose memories
  name the personas they perceived (15 §8). The difference between the
  record and the recollection is the investigation.

## 13. Populations, stocks and services

### 13.1 Populations

A population is a count of a kind of person or creature at a site or in a
tract (06 §12). Counts change by casualties, by `migrate` (a daily cold
rule moving people between places by their conditions), and by whatever
else content defines.

A population has homes and a **presence** table: what share of it is at
home, at work or about at each time of day (10 §11.2). The anonymous
individuals instantiated when a tract loads follow from that, so a street
is busy at noon and empty before dawn.

Sites and regions carry statistics as variables: wealth, order, mood,
whatever a mod measures. The engine gives them no meaning.

### 13.2 Stocks

A site may hold **stocks**: amounts of items and resources, with rates of
production and use from its definition, its people and its machines
(16 §13). Stocks follow the same rule as stores: straight lines between
changes.

A **shipment** is a traveler (02 §20) carrying stock from one site to
another. It takes time, follows a route, and can be met on the way.

`price(site, what)` is a content function over stock and demand.

Mods that are not about trade ignore all of this at no cost.

### 13.3 Services

Tract services (16 §4.4) are set by rules and incidents. A site that
supplies a service to a region does so while it stands, is staffed and is
supplied itself. Knock out the power station and the district goes dark by
one value changing.

### 13.4 Holdings

A site is **held** by a group or by a character: the `owner` of its
definition (10 §11.1) to begin with. `set_site { site, holder }` changes
it, as does a trade (16 §3.6), and `site_changed_hands` is raised.

Holding a site does nothing by itself. What it is worth to hold one is
written with what already exists:

| A holding | How |
|-----------|-----|
| Earns | The site's stock of an account resource is its balance (16 §5.6), with a rate from its definition, its people and its machines. A standing payment from the site to its holder passes on whatever share content says. |
| Pays its people | Standing payments to its staff. When one stops, rules on `payment_stopped` decide whether they stay. |
| Supplies its holder | The holder trades with its own site, taking from stock and having it delivered |
| Can be lost | Incidents, devastation and other holders change its stock, its staff and who holds it, like anything else |

Someone with factories, a guild with chapter houses, a gang with fronts
and a lord with manors are holders of sites.

## 14. Weather and time of day

### 14.1 Weather

```json
{ "type": "weather", "id": "temperate_coast",
  "states": {
    "clear": { "light": 1.0, "wind": [0, 6],  "sea": 2, "to": { "cloud": 0.3 } },
    "cloud": { "light": 0.7, "wind": [2, 10], "sea": 3, "to": { "clear": 0.3, "rain": 0.25 } },
    "rain":  { "light": 0.5, "wind": [4, 14], "sea": 4, "field": { "rain": 30 }, "to": { "cloud": 0.4, "storm": 0.1 } },
    "storm": { "light": 0.3, "wind": [12, 28], "sea": 7, "field": { "rain": 80 }, "to": { "rain": 0.5 } } },
  "step": "1 h" }
```

Each region has a weather state from its climate's definition. It changes
by the given chances each step. The state sets values on the region: light
(15 §3.3), wind and sea state (11 §4.3), temperature, and fields laid on
loaded cells open to the sky.

Rules may set a region's weather. A power over storms does.

### 14.2 Calendars

A realm has a **calendar**: the length of its day and year and the names
of their parts. `here.realm.time_of_day`, `day`, `season` are read by
routines, sky light and rules. A realm whose clock runs at another rate
(03 §3.3) has days to match.

## 15. Devastation

When a release reaches unloaded space the world raises `region_devastated`
(02 §17.4). The cold layer answers with rules: casualties among
populations and agents in proportion to devastation; services lost;
survivors migrating; groups whose goals accept disasters responding. The
bundled content supplies a plain set. A mod about the aftermath supplies a
richer one.

## 16. Explaining

For any character, the inspector (14) can show:

| View | Shows |
|------|-------|
| Goals | Every goal's score now, the terms of each, and which is held and why |
| Plan | The method, the steps done, the step in hand, and why the last one failed |
| Options | The last `choose`: every option considered, with its fields and score |
| Route | The path, the regions it crosses, and what forced the last reroute |
| Beliefs | Tracks, memories and how each was come by (15) |

And for the cold layer: an agent's activity and aim, an incident's stage
and cast, what each knowledge store holds.

A character that does something inexplicable is a bug in content. These
views are how it is found.

## 17. Modding surface

**Definition types**: `mind`, `goal`, `tactics`, `collective`, `formation`,
`routine`, `incident`, `weather`, `channel`. Fields `calendar` on realms;
`stocks` and `supplies` on site definitions; `cold` on goals; `intent`
and `value` on grants (08 §11).

**Events**: `goal_adopted`, `goal_done`, `goal_failed`, `step_failed`,
`job_claimed`, `job_released`, `order_followed`, `formation_broken`,
`agent_loaded`, `agent_stored`, `agent_folded`, `incident_started`,
`incident_stage`, `incident_ended`, `responder_arrived`,
`knowledge_added`, `published`, `population_changed`, `stock_changed`,
`shipment_sent`, `shipment_arrived`, `site_changed_hands`,
`weather_changed`, `day_part_changed`.

**Hooks**: `goal_scoring` (adjust a score), `option_scoring`,
`incident_starting`, `casualties_applying`.

**Effects**: `think`, `set_goal`, `clear_goal`, `claim_job`, `release_job`,
`form_up`, `disband`, `start_incident`, `end_incident`, `advance_stage`,
`casualties`, `set_agent`, `make_agent`, `fold_agent`, `add_knowledge`,
`share_knowledge`, `publish`, `migrate`, `move_stock`, `set_stock`,
`send_shipment`, `set_weather`.

**Queries**: `visible`, `heard`, `allies`, `threats` (all from belief),
`jobs`, `agents` (of a site, group or region), `incidents`, `knowledge`.

**Functions**: `weight`, `threat`, `standing`, `strength`, `severity`,
`best_reaction`, `pick_position`, `path_cost`, `least_effort_that`,
`price`, `threat_removed`, `harm_prevented`, `escape_margin`,
`detection_risk`.

**Settings**: `max_hot_minds`.

**Properties**: on minds, `goal`, `goal_about`, `step`, `orders`; on
sites, `locals`, `stock(what)`, `population(kind)`, `holder`; on regions,
`weather`, `wind`, `sea_state`; on realms, `time_of_day`, `day`, `season`;
on incidents, `stage`, `cast(role)`.

## 18. Invariants

| # | Invariant |
|---|-----------|
| N1 | A goal, method or tactic reads nothing about another entity except through belief and perception. Enforced at load. |
| N2 | A character's every act is an action, interaction or speech that the player could have chosen for it. |
| N3 | Path-finding structures always agree with the world's current geometry when consulted. |
| N4 | A notable individual is in exactly one place: loaded as an individual, or stored as an agent. |
| N5 | An incident stage is settled exactly once, live or abstract. |
| N6 | Whatever an abstract stage changes is in records or pending edits, and is present when the place loads. |
| N7 | Cold agents, incidents, weather and stocks schedule nothing between their wake-ups. |
| N8 | Results do not depend on the order minds think in within a tick, beyond the stated ordering. |

## 19. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| A mind thinking: scoring ten goals | under 20 µs |
| A `choose` among thirty options | under 80 µs |
| A path across a loaded tract, typical | under 150 µs |
| Rebuilding one chunk's regions | under 200 µs |
| A flow field over 64 × 64 cells | under 1 ms |
| One warm step for 2,000 characters | under 3 ms |
| A simulated day for 20,000 agents, 500 sites and their incidents | under 60 ms |
| An idle agent | no cost between wake-ups |

## 20. Open questions

1. **Believability of bulk fighting.** Open. Warm exchanges and abstract
   battles use expected values. Whether a battle watched from a distance
   and then walked into looks continuous has to be tested with scene S19
   and the comparison tool (14 §11).
2. **Agreement of live and abstract.** Closed by `incident compare`
   (14 §11).
3. **Minds that plan ahead.** Not a goal (section 1). Goals and methods
   are reactive; a trap or a con is authored as a method. Search-based
   planning is not provided.
4. **Learning what others can do.** Settled: a bounded list of observed
   capabilities on each track (15 §7.1).
5. **Map memory for characters.** Kept as it is. Only the player's
   characters keep it (15 §18). Characters do not get lost.
6. **Vehicle driving in tight spaces.** Settled: berths with approach
   lines (section 5.6).
