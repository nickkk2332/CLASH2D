# 13 — Scenarios, Modes and Events

Status: design complete, ready for implementation.
Implementation plan: [plans/scenarios.md](plans/scenarios.md).
Serves scenes S7, S15, S19, S20 and S21 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies what a game is: the scenario a player starts, the
worlds and characters it begins with, the rules that make it a sandbox, a
roguelike, a campaign or a single battle, objectives and missions, plots,
the director that paces what happens around the player, offences, cases
and investigation, and death, saving, endings and what carries over between
games.

It adds little machinery. A scenario is mostly a place to put definitions
that other documents already give meaning to.

**A note on the word.** In this document a *scenario* is a definition: a
game the player can choose from the new-game screen. The implementation
plans also use "scenario" for scripted regression tests with golden
hashes. They are unrelated. Where both could be meant, this document says
*test script* for the latter.

Fixed from earlier documents:

- A mode is a set of rules, not an engine feature (roadmap; 01 R10).
- Incidents (09 §11) are the only kind of thing that happens. The director
  starts and paces them; it has no events of its own.
- Nothing is scaled for fairness (D-034).
- Turning the whole world back is done by saving and loading. A scenario's
  rules use it for checkpoints and for returning after death. An ability
  may use it too, where an author accepts what it costs (section 9.5).

Names of scenarios, missions and directors in examples are illustrative
content.

## 2. The picture

A **scenario** says: these worlds, these characters or this way of making
them, starting here at this time, under these rules. Everything else is the
simulation doing what it does.

What kind of game it is follows from a few fields. A scenario with free
character creation, no objectives and no ending is a **sandbox**. One whose
save is deleted when the character dies, and which makes a fresh dungeon
each time, is a **roguelike**. One that ends when a side is eliminated is a
**skirmish**. One that leads to another, carrying its characters along, is
a **campaign**.

The player may be given **objectives**. A **mission** is an objective
someone in the world hands out: a bounty, a delivery, a job. A **plot** is
a small set of beats that fire in order when their conditions come true.

Left alone, the world produces incidents at its own rates, mostly far from
the player. A **director** is a pacing rule that starts incidents near the
player, with lulls between. It chooses when and where, and what kind. It
does not choose how hard.

When someone is hurt or something is broken, the world records who really
did it. People remember who they saw. A **case** is the gap between the
two, and it can be worked from either side: by a player piecing together
what happened, or by a watch piecing together what the player did.

When the player's characters are gone, the scenario says what follows:
the end, another character in the same world, or a return to an earlier
moment that perhaps only the character remembers.

## 3. Scenarios

```json
{ "type": "scenario", "id": "ashfall", "name": "Ashfall",
  "doc": "Wake in the shelter. The island is not as you left it.",
  "tags": ["survival", "open"],
  "worlds": [ { "realm": "ash_island", "as": "home", "seed": "ask" } ],
  "options": [ "starting_season", "island_size" ],
  "settings": { "core:reaction_prompts": "threats" },
  "locked": [ "core:permadeath" ],
  "start": {
    "characters": [ { "create": { "kinds": { "species": { "tag": "playable" }, "background": { "tag": "survivor" } },
                                   "holding": "shelter_kit" } } ],
    "at": [ { "name": "The shelter", "site": { "tag": "shelter" }, "role": "bed" },
            { "name": "The shore",   "site": { "tag": "wreck" },   "role": "deck" } ],
    "time": { "day": 1, "at": "06:00" },
    "do": [ { "apply_map": { "map": "shelter_stocked", "at": "start.site.mark('store')" } } ] },
  "rules": [ "ash_storms", "hunger_and_thirst" ],
  "director": { "choose": ["steady", "harsh", "quiet"] },
  "objectives": [ { "id": "first_week", "text": "Survive seven days.", "done": "realm.day > 7" } ],
  "ends": [ { "if": "party.count == 0", "as": "lost" } ],
  "saving": { "policy": "single", "on_party_gone": "end" },
  "screens": { "summary": "ashfall_summary" } }
```

| Field | Meaning |
|-------|---------|
| `worlds` | Realms created when the game starts (section 4.1) |
| `options` | Settings (04 §10) the player is asked about before starting |
| `settings`, `locked` | Values this scenario starts with, and those the player may not change in it |
| `start` | Who, where, when, and what is set up (section 4) |
| `rules` | Rules (05 §9) that are in force only in this scenario |
| `director` | Section 7 |
| `objectives`, `plot` | Sections 5 and 6 |
| `ends` | Section 9.4 |
| `saving` | Section 9 |
| `screens` | Screens (12) to use in place of the bundled ones: creation, play, summary, an introduction |
| `then` | For campaigns: what follows (section 10.3) |

A scenario's state (objectives, plot, director, cases) is part of the
simulation and is saved with it.

## 4. Starting

### 4.1 Worlds

Each entry names a realm definition (02 §4.1) and a seed: fixed, random,
or asked of the player. The realm is created, planned (10 §9.2), and given
its settling time (10 §11.4). Realms reached later through charts are made
on demand (10 §9.4).

A scenario may name no generated world at all, only an authored one
(10 §9.3): an arena, a single building, a ship.

### 4.2 Characters

| Entry | The player |
|-------|-----------|
| `create` | Builds a character on the creation screen (12 §10.7) within stated limits: which template kinds and tags are offered, budgets, things fixed or forbidden |
| `spec` | Is given a ready-made character (06 §10.3) |
| `pick` | Chooses among several ready-made ones |
| `roster` | Brings a character kept from an earlier game (section 10.2) |
| `become` | Takes over someone who already lives in the world: chosen from a query over its notable inhabitants, or at random |

Several entries make a party (03 §5.1). Each has a control mode.

`become` works because every notable in the world is a full character
(06 §12). A game about being nobody in particular in a town that does not
know you are special is `become` with a random townsperson.

### 4.3 Where and when

`at` is a place: a role at a site chosen by query (10 §11.1), a named
entry of a realm, or coordinates. A list offers the player a choice.
`time` sets the calendar (09 §14.2).

### 4.4 Setting up

`start.do` runs once, after the characters are placed: apply maps
(10 §4.3), start incidents, set relations, grant abilities, open an
introduction, move the character somewhere they have never been. A story
that begins with being pulled into another world is a `teleport` here.

## 5. Objectives and missions

### 5.1 Objectives

```rust
pub struct Objective {
    pub id: ObjectiveId,
    pub text: Text,
    pub state: ObjectiveState,        // Hidden | Active | Done | Failed
    pub done: Option<Cond>,           // reactive (05 §7.5)
    pub failed: Option<Cond>,
    pub marker: Option<Marker>,       // a site, a place, a persona, shown on maps
    pub parent: Option<ObjectiveId>,
    pub on_done: Effects, pub on_failed: Effects,
}
```

An objective is something the player is told they are trying to do. Its
conditions are watched without polling. Completing or failing it runs
effects and raises an event. Objectives nest.

Conditions may read the truth of the world or what the player's characters
know (15 §7). "Find out who did it" should be the second kind.

An objective obliges nothing. A sandbox may have none.

### 5.2 Missions

```json
{ "type": "mission", "id": "bounty", "name": "Bounty: {$quarry.name}",
  "params": { "quarry": "persona", "poster": "group", "reward": "real" },
  "objectives": [
    { "id": "find",  "text": "Find {$quarry.name}.", "done": "taker.knows($quarry) and taker.confidence($quarry) > 0.8",
      "marker": { "last_known": "$quarry" } },
    { "id": "bring", "text": "Bring them to {$poster.name}.", "after": "find",
      "done": "delivered($quarry, $poster)" } ],
  "expires": "10 d",
  "on_done": [ { "pay": { "of": "credits", "to": "taker", "from": "$poster", "amount": "$reward" } } ] }
```

A **mission** is a template for objectives, started with parameters.
Starting one gives the player its objectives; several may run at once.

Missions come from the world:

| Source | How |
|--------|-----|
| A person | A `talk` option (15 §11) whose effect is `start_mission` |
| A **posting** | A piece of published knowledge (09 §12) that carries a mission. A notice board, a terminal or a town crier shows what its outlet holds; accepting one starts it. |
| An incident | A stage's effects offer one: a robbery in progress, a call for help |
| A group | A collective's goal (09 §8.1) posts one when it wants something done |
| A scenario | Directly |

Because postings are knowledge, they spread and go stale like any other.
The bounty on the board may be for someone already caught. It may be for
you.

## 6. Plots

```json
"plot": [
  { "id": "arrival",     "when": "start",
    "do": [ { "show_screen": { "screen": "ashfall_intro" } } ] },
  { "id": "first_night", "after": ["arrival"], "when": "realm.time_of_day == 'night'",
    "do": [ { "start_incident": { "id": "scavengers_come", "at": "start.site" } } ] },
  { "id": "signal",      "after": ["first_night"],
    "when": { "exists": { "items": { "held_by": "party", "where": "it.has_tag('radio_part')" } } },
    "do": [ { "start_mission": { "mission": "repair_the_radio" } } ] } ]
```

A **plot** is a set of **beats**. A beat fires once, when the beats it
comes after have fired and its condition holds. Its effects are ordinary:
start an incident, give a mission, apply a map, change a relation, show
something.

A plot is a convenience over rules and variables for stories with an
order. It does not constrain the player, who may be elsewhere doing
something else when a beat's condition never comes true.

## 7. The director

### 7.1 What it is for

The world's own incidents (09 §11.2) happen where their rates put them,
which is mostly not where the player is. A **director** makes sure the
player's part of the world is not dull, and not relentless.

```json
{ "type": "director", "id": "steady", "name": "Steady",
  "tension": {
    "fade": "0.15 per hour",
    "raise": [ { "on": "strike_landed", "involving": "party", "by": 0.04 },
               { "on": "incident_ended", "within": 300, "by": 0.25 } ] },
  "band": [0.15, 0.6],
  "lull": ["40 min", "4 h"],
  "pool": [
    { "incident": "street_robbery", "weight": "2 * here.region.var.lawlessness",
      "if": "realm.time_of_day == 'night'", "within": [40, 250], "again": "6 h" },
    { "incident": "house_fire",     "weight": 1, "within": [80, 600] },
    { "incident": "stranger_in_town", "weight": 0.5, "again": "3 d" } ],
  "on_travel": [ { "incident": "ambush_on_the_road", "weight": 1, "bands": ["road"] } ] }
```

### 7.2 Tension

The director keeps one number, **tension**, for the party. Listed events
raise it. It fades with time. It is a variable rules can read and set.
Below the band a lull is counted, and something is started when it ends.
Above the band nothing is started. Inside it nothing is started and the
lull does not run.

### 7.3 Starting things

When tension has been below the band for a time drawn from `lull`, the
director starts one incident:

1. Entries whose condition holds and whose `again` time has passed are
   weighed, and one is drawn.
2. A place for it is found within the entry's distance from the party,
   satisfying the incident's own `at` (09 §11.1).
3. The incident is started there, through the hook `incident_starting`.

While tension is above the band, the director starts nothing.

An entry's distance may be a length or a time. As a time, it is how far
the party would get at the pace it has kept over the last hour of its own
time. "Ten minutes away" is a few streets for someone on foot and a
province for someone who flies, so a party that crosses a continent in a
minute still has things happen where it can reach them.

On `travel_step` (11 §13.3) it does the same from `on_travel`, for
journeys.

The incident is then the simulation's. It may be live or abstract
(09 §11.3). The party may hear an alarm, read of it later, walk into it,
or never know.

### 7.4 When and where, not how hard

The bundled directors weigh incidents by the state of the place: how
lawless, how rich, what time it is. They do not look at how strong the
party is. An incident near a party that can level a city is the same
incident as near one that cannot win a bar fight.

Weights are expressions, and an author can write any expression. A
director that matches threats to the player is possible. It is not what
this engine is for (D-034), and the bundled content does not do it.

### 7.5 Choosing one

A scenario names a director, offers several for the player to choose from,
or has none. Different directors are different temperaments: steady, harsh,
quiet, erratic.

## 8. Offences, cases and investigation

### 8.1 What the world records

Every change to the world and every hit carries a cause: who physically
did it and who is responsible (02 §7.3). Events that sum many changes (an
energy release, a collapse, a devastated tract) carry totals by
instigator: what was destroyed, who was caught in it.

This record is true. No character has access to it (01 R11).

`cause_facts(x)` extracts what could be learned from a cause by someone
with the means: when; by what kind of act and with what kind of surface;
how much energy; the persona the actor was presenting at the time; and,
separately, who it really was.

### 8.2 Offences

What counts as an offence is content: a rule that listens for deaths,
thefts, destruction, trespass, and decides that this one matters to
someone. When it does, it opens a case.

### 8.3 Cases

```rust
pub struct Case {
    pub id: CaseId,
    pub kind: CaseKindId,
    pub about: CaseSubject,            // an incident, an event, a place and time
    pub truth: Cause,                  // never shown
    pub holders: Vec<Holder>,          // who is working it: a group, a persona
    pub findings: BTreeMap<Holder, Vec<Finding>>,
    pub state: BTreeMap<Holder, CaseState>,   // each holder's own: Open | Closed { blamed }
}
```

A **case** ties together a thing that happened, the true record of it, and
what each party looking into it has found. Several parties may hold the
same case and know different things.

### 8.4 Evidence

Evidence is whatever the event left that carries information:

| Evidence | Carries |
|----------|---------|
| Marks: blood, footprints, scorching (02 §5.1) | Their cause |
| Damage to elements and wounds on bodies | Their cause; the kind and energy of what made them |
| Things left behind | What they are; who held them last |
| Witnesses (15 §7) | Memories naming the personas they perceived |
| Local knowledge (09 §12) | What people there saw |
| Recorders | A device with senses (15 §4) keeps what it perceives as memories. A camera is a witness that does not forget. |

Examining evidence is an interaction (16 §3). What it yields is content: a
rule takes `cause_facts` and the examiner's means (a skill, an ability, a
tool) and adds **findings**. A layman sees blood. A surgeon sees a narrow
blade driven upward. An ability may see the face of whoever held it.

A finding is knowledge. It goes into the examiner's beliefs (15 §7) and
into the case under that holder.

### 8.5 Closing a case

`accuse { case, persona }` puts a name to it. The hook `case_judging`
decides whether the holder accepts: by what it has found, by how it regards
the accused, by whatever its rules are. Accepted, the case closes with that
persona blamed, and the holder's group acts on it.

The engine does not check the accusation against the truth. A watch that
has three witnesses to a face will convict the owner of that face. If
someone else was wearing it (15 §8.3), that is scene S15, and the true
record is still there for whoever looks harder.

A closed case can be opened again. When a holder gains a finding about a
case it has closed, `case_judging` runs again with `reopening` set. If its
rules no longer accept the earlier blame, the case reopens and
`case_reopened` is raised. `reopen_case { case }` does the same directly.
What was already done to the one blamed is for rules to undo, or not.

### 8.6 From the other side

Everything above applies to what the player's characters do. An offence
with witnesses becomes knowledge (09 §12), a group opens a case, its agents
gather findings by the same means (09 §10.3), and if they settle on a
persona the player presents, that persona is wanted. Being a villain is
not a mode. It is what happens when the cases are about you.

A character who is held to account for what their powers broke is the same
mechanism: the totals of section 8.1, in a case held by whoever cares.

## 9. Death, saving and endings

### 9.1 Requests to the application

Saving, restoring and ending act on the simulation as a whole, so the
simulation does not perform them. A rule raises a request; `advance`
returns it (03 §4.3); the application carries it out at a safe point.

| Effect | Requests |
|--------|----------|
| `checkpoint { name }` | A save the player cannot make by hand |
| `restore_checkpoint { name, carry }` | A return to it |
| `end_scenario { as }` | An ending |
| `next_scenario { scenario, carry }` | An ending that leads on |
| `keep_world_history { span, every }` | Experimental. Continuous snapshots, so the world can be turned back (section 9.5) |
| `rewind_world { by, carry }` | Experimental. A return of everything to an earlier moment |

### 9.2 Saving

| `policy` | The player |
|----------|-----------|
| `free` | Saves and loads at will, any number |
| `single` | Has one save, written on leaving and at intervals; cannot go back |
| `checkpoints` | Has only the saves the scenario's rules make |
| `none` | Cannot save: a single sitting |

### 9.3 When the party is gone

`on_party_gone` says what follows when no directly controlled character
remains able to act.

| Value | Result |
|-------|--------|
| `end` | An ending. Under `single`, the save is removed. |
| `continue` | The player takes another character in the same world: one of their own, a new one made on the creation screen and placed by rule, or by `become` |
| `restore` | The last checkpoint is restored |

`on_party_gone` may also be written in full:

```json
"on_party_gone": { "when": "party.count == 0", "then": "continue",
                   "by": ["own", "create"], "as": "lost", "do": [] }
```

`when` defaults to there being no directly controlled character alive
with a working mind. `by` says which of the ways of carrying on are
offered. `restore` with no checkpoint to go back to is refused, with its
reason, and the scenario ends as `end` would.

`restore_checkpoint` may **carry** things across: legacy variables
(section 10.1), and the controlled characters' memories, which arrive
marked as foreknowledge. A character who dies, wakes at the last dawn and
remembers is a scenario with one line of configuration.

### 9.4 Endings

```json
"ends": [
  { "if": "party.count == 0", "as": "lost" },
  { "on": "objective_changed", "filter": { "id": "escape_the_island", "state": "done" }, "as": "escaped",
    "then": { "scenario": "mainland", "carry": ["party"] } } ]
```

An ending names an outcome. The application shows the scenario's summary
screen, writes legacy variables, applies the save policy, and either
returns to the title, offers to continue playing where that makes sense, or
starts what follows.

### 9.5 Turning back the whole world (experimental)

> **Experimental.** This feature is specified so that it can be tried, not
> because it is known to work well. Its cost is unmeasured and may prove
> prohibitive. Its interactions with the rest of the simulation have not
> been worked through. It may change in incompatible ways or be withdrawn.
> No bundled content uses it. A mod must opt in to use it (04 §8.4), and is
> warned at every load that it has.

A checkpoint is a moment someone chose in advance. An author who wants the
world itself turned back by some span, at any time, asks for **history**.

```
keep_world_history { span, every }
rewind_world { by, carry }
```

- While history is kept, the application takes a snapshot of the whole
  simulation every `every` of universal time and keeps those within `span`,
  together with the input log.
- `rewind_world` loads the latest snapshot at or before the moment `by`
  ago and replays the log up to that moment exactly (14 §8.3). The world is
  then as it was, to the tick.
- `carry` is then applied, as for `restore_checkpoint`: legacy variables;
  and for named characters, their memories of the abandoned span, marked as
  foreknowledge, and any named variables. Nobody else has lived anything
  after that moment.
- The commands the player gave after that moment are discarded with the
  future they belonged to.
- If history does not reach back as far as asked, the rewind goes as far as
  it does and says so. With no history kept, it is refused with a reason.
- Keeping history may be bound to an ability (08 §7.3), so that it is kept
  only while someone who can use it exists.

It is independent of the saving policy (section 9.2). It is something that
happens in the world, not the player reloading.

This is the expensive way to go back. Snapshots of everything, taken
continuously, cost memory and time in proportion to how much is loaded and
how often they are taken. An area rewind (08 §7.4) costs almost nothing by
comparison, and can make those inside it forget, which is enough for most
purposes. `rewind_world` is for the author who needs everyone, everywhere,
to have forgotten.

Known to be unresolved, and part of why it is experimental:

| Question | State |
|----------|-------|
| What it costs on a realistically loaded world | Unmeasured. The budgets in section 15 are guesses. |
| Legacy variables written during the abandoned span | They have already reached the player's profile. Whether they revert unless carried is undecided. |
| Realms created or destroyed, journeys begun, and definitions reloaded during the span | Expected to be undone with everything else. Not examined case by case. |
| Realms whose clocks run at other rates | `by` is universal time. What that means to someone inside a fast realm is not worked out. |
| Use by several characters, or again during a replay | Undefined. |
| Whether a player finds it comprehensible | Untested. |

## 10. Between games

### 10.1 Legacy

A variable declared with scope `legacy` (05 §6) belongs to the player's
profile, not to a save. Rules read and write it like any other.

- At the start of a game and on every load, legacy values are copied into
  the simulation, so reading one is deterministic.
- Writes are applied to the simulation at once and to the profile at the
  next safe point.

Legacy is how a roguelike remembers what has been unlocked, how a scenario
knows it has been finished before, and how a world can change a little
because of what an earlier character did in it.

### 10.2 The roster

`keep_character { who, name }` stores a character, complete, in the
player's profile. A scenario whose start allows `roster` lets the player
bring one in.

### 10.3 Campaigns

An ending's `then`, or `next_scenario`, starts another scenario. `carry`
lists what goes with it: the party, particular items, named variables. The
next scenario places what was carried as its start says.

A campaign is scenarios joined this way. Each can have its own worlds,
rules and saving.

## 11. Modes

No mode is built in. This table is a map for authors.

| A game that is | Is a scenario with |
|----------------|--------------------|
| **Sandbox** | `create` with everything offered; a choice of starting places; no objectives; no ends; a director; `free` saving |
| **Roguelike** | A realm made fresh from a random seed, often by assembly (10 §7.2) or on demand; `single` saving; `on_party_gone: end`; legacy for what carries over |
| **Campaign** | Objectives, a plot, `then` |
| **Skirmish, arena** | An authored realm; ready-made sides; an end when one side is gone; `none` saving |
| **Warzone** | A realm with battle incidents (09 §11.6) between holders; the player anywhere in it |
| **Survival** | Pools with rates, recipes, building (16 §12); a harsh director; an end on death |
| **Investigation** | Incidents that open cases; missions to close them |
| **Horror** | One thing far beyond the party (D-034), places to hide (15 §5.3), and no means to win a fight |
| **Villainy** | Any of the above, with the player causing the incidents |
| **Another world** | A `teleport` in `start.do`; templates with extraordinary gifts (08) |
| **Someone else's life** | `become` |
| **Return after death** | `on_party_gone: restore` with memories carried |
| **Generations** | `on_party_gone: continue` |

A total conversion (04 §14) ships its own scenarios and may ship only one.

## 12. From title to play

1. The profile's boot screen (04 §3.3).
2. New game: scenarios offered by the loaded mods, by tag.
3. The scenario's options.
4. Characters: the creation screen, a choice, or nothing.
5. Worlds are created. A realm plan takes a moment (10 §15).
6. Characters are placed; `start.do` runs; `scenario_started` is raised.
7. Play.

Every step that shows something is a screen (12) the scenario may replace.

## 13. Modding surface

**Definition types**: `scenario`, `mission`, `director`, `case_kind`.
Scope `legacy` for variables.

**Events**: `scenario_started`, `objective_changed`, `mission_started`,
`mission_done`, `mission_failed`, `mission_expired`, `beat_fired`,
`director_started`, `case_opened`, `finding_added`, `case_closed`,
`case_reopened`,
`party_gone`, `checkpoint_made`, `restored`, `world_rewound`,
`scenario_ended`.

**Hooks**: `director_choosing`, `case_judging`, `scenario_ending`.

**Effects**: `add_objective`, `complete_objective`, `fail_objective`,
`start_mission`, `post_mission`, `fire_beat`, `set_tension`, `open_case`,
`add_finding`, `accuse`, `close_case`, `reopen_case`, `checkpoint`,
`restore_checkpoint`,
`end_scenario`, `next_scenario`, `keep_character`, `take_control`.

**Experimental effects** (section 9.5; opt-in by 04 §8.4):
`keep_world_history`, `rewind_world`.

**Queries**: `objectives`, `missions`, `postings`, `cases`, `findings`.

**Functions**: `cause_facts`.

**Where expressions are read.** An objective's and a mission's
expressions have `taker` bound, the character who took it on, and read
what the taker believes, which is why a marker follows belief. A beat's
`when`, a director's weights and an ending's conditions read truth, with
`party`, `scenario` and `start` bound.

**Further verbs and fields.** `fail_mission` and `cancel_mission`; on a
mission, `failed` (a condition), `on_failed` and `on_expired`. An
objective with `after` is hidden until those it names are done. A
posting is withdrawn with `remove_knowledge` (09 §17). `set_director`
changes the director, or removes it. `take_control { who }` gives the
player direct control of a character. `accuse`, `open_case` and
`close_case` name the `holder` concerned, and each holder closes its own
view of a case. A `case_kind` says what opens one, who may hold it, and
what its holders do with it. If no rule answers `case_judging`, the
accusation is refused. Agents working a case gain findings by the cold
step `examine` (09 §10.3).

| Hook | Runs before | A handler may |
|------|-------------|---------------|
| `director_choosing` | The director starts what it drew, where it chose | Change the entry or the place; cancel |
| `case_judging` | A holder accepts or refuses an accusation | Decide it |
| `scenario_ending` | An ending takes effect | Change which ending; cancel |

**Properties**: `scenario.id`, `scenario.var.*`, `scenario.director`,
`party`, `party.count`, `party.tension`, `start.site`, `start.realm`,
`legacy.var.*`; on cases, `holders`, `findings(holder)`, `state`, `blamed`.

## 14. Invariants

| # | Invariant |
|---|-----------|
| Z1 | A scenario changes the simulation only through effects that exist independently of it. |
| Z2 | The director starts incidents and does nothing else. |
| Z3 | A case's true record is never readable in a mind context or an ordinary screen. |
| Z4 | An objective's state always equals a fresh evaluation of its conditions, until it is done or failed, after which it does not change. |
| Z5 | A beat fires at most once. |
| Z6 | Saving, restoring and ending happen only at safe points, by request. |
| Z7 | Reading a legacy variable gives the same value on replay. |
| Z8 | Scenario state survives saving and loading with an identical hash. |

## 15. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| A director's decision | under 200 µs |
| Watching fifty objectives while nothing changes | no cost |
| Opening a case | under 20 µs |
| Starting a game in a small authored realm | under 1 s |
| Starting a game in a generated island realm | under 5 s |
| Taking a world-history snapshot of the default loaded set (experimental; a guess) | under 150 ms |
| `rewind_world` by ten seconds (experimental; a guess) | under 2 s |

## 16. Open questions

1. **Seeing ahead.** Deferred until the experimental rewind of section 9.5
   has been tried (08 §15).
2. **Shared worlds.** Kept as it is: a save is a world. One world visited
   by several characters is one save with several characters in it
   (section 9.3, `continue`).
3. **Deduction.** Closed by the case file (14 §13).
4. **Trials.** Settled as far as the engine goes: closed cases can be
   judged again on new findings (section 8.5). Courts, appeals and
   sentences are content.
5. **Director and distance.** Settled in section 7.3: distance may be
   travel time at the party's recent pace.
6. **Cheaper history.** Deferred, with the rest of section 9.5's
   unresolved questions.
