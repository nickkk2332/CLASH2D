# 14 — Tooling

Status: design complete, ready for implementation.
Implementation plan: [plans/tooling.md](plans/tooling.md).
Serves scene S12 of [00-vision.md](00-vision.md): a modder builds without
touching Rust, and can see what they built doing what it does.

## 1. Scope

This document specifies the tools around the engine: the console, the
inspector, traces, the profiler, test scripts, replay, the tools for
writing mods and maps, and a few aids for players that are built the same
way.

Several were promised by other documents. This one gathers them, fills
them in, and adds what was asked for along the way.

| Promised in | What |
|-------------|------|
| 04 §13 | `lint`, `schema`, `docs`, `init-mod`, `explain`, `show` |
| 05 §15 | Rule trace, explain, profile, console evaluation, assertions |
| 09 §16 | Views of a character's goals, plan, options, route and beliefs |
| 10 §12 | Map and generation previews and checks |
| 12 §12 | Golden frames |
| plans/world-model W11 | The world viewer and test script runner |

Crate `clash-tools` for the command line. In-game tools are screens (12)
in the `debug` context, shipped as a mod.

There are three audiences. **Modders** need to know what they wrote, what
it did, and why. **Builders of the engine**, who are mostly agents with no
memory between sessions, need failures to be reproducible and to say where
they are. **Players** need a few of the same views, limited to what their
characters know.

## 2. Principles

| # | Principle |
|---|-----------|
| 1 | **A tool is a frontend.** It reads the simulation through views and changes it only through commands (03 §4.4). No tool has a private way in. |
| 2 | **Generated, not written.** The reference, the schemas, the inspector's contents and the console's help come from the registries the engine itself uses (04 §13; 05 §5.3). |
| 3 | **Everything that can be refused says why.** Section 5. |
| 4 | **One language.** The in-game console, headless tools and test scripts speak the same commands. |
| 5 | **Reproducible.** Anything seen once can be seen again: from a save, an input log and a seed. |
| 6 | **Free when off.** Tracing, profiling and checking cost nothing unless switched on. |
| 7 | **Text first.** Every tool's output can be read in a terminal and compared by a program. |

## 3. The console

### 3.1 Where it is

The same console runs in three places: as an overlay in the game, in the
terminal world viewer, and line by line from a file as a test script
(section 7).

In the game it is available when the profile allows it. A profile for
players may switch it off.

### 3.2 Language

```
release at @here energy=1e9 kind=blast
spawn town_guard at @c:120,44,0 as=g1
as @g1 act slash target=@self effort=0.2
eval self.pool.blood.frac
do { "add_trait": { "to": "self", "trait": "adrenaline" } }
select entities around=@self radius=10 where="it.has_tag('guard')"
watch dist(self, @sel)
trace on rule vault_alarm
explain action slash @self
advance 2s
until "party.count == 0" max=5min
save before_fight
check
```

- A line is a command name and arguments. Arguments are positional or
  `key=value`. Values are numbers with units, ids, references, quoted
  expressions, or JSON in braces.
- **References** begin with `@`: `@here` (the cursor's cell), `@self` (the
  active character), `@sel` (the last selection), `@c:x,y,z` (a cell),
  `@e:17` (an entity by id), `@site:name`, and names given with `as=`.
- `as <reference> <command>` runs a command with `self` bound to something
  else.

### 3.3 What it can do

| Kind | Commands |
|------|----------|
| The world and simulation | One command for each verb (05 §10) and each engine function a subsystem exposes. They are registered with the verbs, so a new verb is a new command. |
| Expressions and rules | `eval`, `do`, `run`, `explain` |
| Time | `advance`, `step`, `until`, `rewind` (section 8.3) |
| Looking | `select`, `inspect`, `watch`, `trace`, `profile`, `view` |
| State | `save`, `load`, `hash`, `check`, `diff` |
| Tests | `assert`, `expect` (section 7) |
| Mods | `reload`, `lint`, `show`, `refs` |

Every command has help generated from its registration.

Console commands that change the simulation are `Debug` commands
(03 §4.4). They are validated, logged and replayed like any other.

### 3.4 Commands from mods

```json
{ "type": "console_command", "id": "stock_shelter",
  "doc": "Fill the starting shelter's stores.",
  "params": { "days": { "type": "int", "default": 7 } },
  "run": "stock_shelter_rule" }
```

A mod may add commands that run its rules. Large mods use these for their
own testing and set-up.

## 4. The inspector

### 4.1 Picking

The inspector opens on whatever is under the cursor or named in the
console: a cell, an element, an entity, an item in a hold, a body part, an
area, a site, a network, a job, an incident, a group, a case, an ability,
a running action, a trait.

### 4.2 What it shows

For any subject, every property registered for its kind (05 §5.3), with
its value, its type and its documentation. Nothing is listed by hand, so a
property added by a subsystem or by a later document appears without
anyone touching the inspector.

- Properties that are subjects can be followed: from a creature to its
  held item, to that item's assembly, to its power cell, to the network it
  is charging from.
- Values can be edited where a setter exists. An edit is a command.
- Variables show their declarations and defaults (05 §6).
- Definitions show where each value came from (04 §13 `show`).
- Any expression can be pinned and stays current.

### 4.3 Why

Beside the plain listing, each kind of subject has views that answer
"why":

| Subject | View |
|---------|------|
| A rule | Its last runs: trigger, bindings, each part of its condition with its value, effects with their arguments, events raised (05 §15) |
| A character | Goal scores and terms; the plan and why the last step failed; the options last weighed; the route (09 §16) |
| A character's senses | Each thing it might perceive: signal received, threshold, awareness, and what is reducing it (15 §5) |
| A character's beliefs | Tracks and memories with how each was come by, set against the truth |
| A hit | Layer by layer: threshold, intensity, what was absorbed, what passed (06 §7.2) |
| A strike | The steps of 07 §3.4 with each contest's inputs and margin |
| A network | Offers, wants, stores, priorities, each member's supply fraction (16 §5.4) |
| A machine or job | What it is waiting for (section 5) |
| An incident | Stage, cast, what started it, the inputs to its rate (09 §11) |
| A cell | What produced it: ground step, placement, transform, later edits with their causes (10 §6.1; 02 §7.3) |
| A wake-up | Its clock, its remaining local time, the chain of clock rates (03 §3) |

### 4.4 Truth and belief

Presentation normally shows what the party perceives (12 §5). The
inspector can show:

| View | Shows |
|------|-------|
| Truth | The world as it is |
| The party's | What the player is ordinarily shown |
| A character's | Where that character believes things to be: its tracks drawn on the map, with confidence and age |
| Split | Any two side by side |

A disguise that works, a stale track and a guard searching the wrong room
are visible at a glance in a split view.

## 5. Reasons

Anything the engine declines to do returns a **reason**: a stable code, a
sentence, and the values involved.

| Refusal | Reason says |
|---------|-------------|
| A command rejected | Which check failed |
| An action unavailable | Which requirement, cost, cooldown, channel or condition, and by how much |
| A reaction not offered | Not noticed, noticed too late, no free channel, cannot cancel |
| A recipe not starting or stalled | Which input, tool, supply or space |
| A job blocked | What it lacks and where that might be found |
| A placement rejected in generation | Which rule |
| A path not found | Where it stopped |
| A definition not loaded | 04 §8 |

Reasons are what the player is told, what a character's mind sees when a
step fails, and what `explain` prints. They are listed in the generated
reference. A refusal without a reason is a defect.

## 6. Traces and the profiler

### 6.1 Traces

A **trace** is a stream of records about what a part of the engine is
doing.

```
tick=4812004 kind=rule    subject=vault_alarm  run=17 if=true  effects=4 raised=2
tick=4812004 kind=strike  subject=e:41         attack=cut target=e:9 aim=+0.4 defence=parry turned=162 landed=18
tick=4812350 kind=mind    subject=e:9          goal=flee score=0.83 was=fight reason=threat
```

- Kinds: rules, events, actions, strikes, hits, perception, minds,
  networks, recipes, incidents, generation, loading, clocks.
- A trace is switched on for a kind, a subject, a place, or a combination.
- Records go to a panel in the game, to a file as one JSON object per
  line, and to test scripts, which can assert on them.
- Custom events (05 §8.1) are traceable without further work.

With no trace switched on, no record is built (principle 6).

### 6.2 The profiler

| View | Shows |
|------|-------|
| Time | Where `advance` spends its time, by subsystem and, within rules, by rule (05 §15) |
| Counts | Events per pump, wake-ups, loaded tracts, entities by form, active field cells, networks, minds thinking |
| Budgets | Every budget in every specification, with its target and the latest measurement |

The profiler is a tool, outside the simulation. It measures wall-clock
time and nothing it measures is visible to rules.

### 6.3 Budgets

The budget tables at the end of each specification are collected into one
file, `budgets.toml`, each row with its benchmark and its limit.
`cargo xtask bench --check` runs them and fails on any over its limit by
more than its margin. When a target is replaced by a measured figure, the
specification's table and this file change together.

## 7. Test scripts

A **test script** is a console session with expectations, kept in a file.
(The plans call these scenarios; see the note in `CLAUDE.md`.)

```
# scenarios/combat/parry_in_time.scn
use profile=test_core seed=7
gen flat size=1
spawn duellist at @c:10,10,0 as=a
spawn duellist at @c:11,10,0 as=b
as @b set_trait stance_guarded
as @a act slash target=@b
advance 400ms
assert "@b.wounds.count == 0"
expect event strike_defended target=@b within=400ms
expect trace strike turned=162 landed=18
expect hash 9c41f0aa…
```

### 7.1 Expectations

| Line | Passes when |
|------|-------------|
| `assert <expression>` | The expression is true now |
| `expect event <type> <fields> within=<time>` | Such an event occurred in that span |
| `expect trace <kind> <fields>` | Such a trace record was produced |
| `expect reason <code>` | The previous command was refused for that reason |
| `expect hash <value>` | The simulation's state hash is that value |
| `expect frame <file>` | The presented frame equals the stored one (12 §12) |

### 7.2 How scripts are run

`clash-tools scenario run` runs each script in four ways and requires the
same result from all:

| Mode | Checks |
|------|--------|
| Straight | The expectations |
| Twice in one process | No hidden state |
| With a save and load at several points | Rule R7 |
| With different thread counts | Rule R1 |

`--bless` rewrites expected hashes and frames, and the change description
must say why they changed. `--paranoid` also runs every invariant checker
(section 7.3) after every command.

Mods ship scripts beside their data (04 §3.1) and run them the same way.

### 7.3 Invariant checkers

Each specification ends with a table of invariants. Each invariant has a
**checker**: a function that examines the simulation and reports
violations. `check` in the console runs them all.

An invariant that cannot be checked at run time (one enforced by the type
system, or by a load-time rule) is listed as such, with the test that
covers it. A test fails if an invariant in any specification has neither.

## 8. Replay

### 8.1 Replaying

A save and the input log that followed it reproduce a session exactly
(03 §4.4). `clash-tools replay <save> <log>` runs it without a display. The
log carries the state hash at intervals.

### 8.2 Finding a divergence

If a replayed hash differs from the logged one, the replay stops at the
first interval where it does, and reports which subsystem's hash differs
(01 §4). `clash-tools diff <a> <b>` then compares two states piece by
piece: which entities, which cells, which wake-ups.

This is how nondeterminism is found, and how a bug seen once on someone
else's machine is seen again.

`clash-tools report` gathers a save, its log, the mod list with
fingerprints and the tail of the traces into one file for sending.

### 8.3 Going back

In development builds the game keeps a snapshot every so often. `rewind
5s` loads the latest snapshot before that moment and replays the log up to
it. Then the inspector can be opened on the past. `advance` from there
replays the same future, unless something is changed first.

This is a tool. It is not offered to rules and is not how abilities turn
back time (08 §7.4; 13 §9.3).

## 9. Writing mods

### 9.1 The loop

1. `init-mod` makes a mod with editor settings that attach the schemas, so
   keys and ids complete as they are typed (04 §13).
2. Edit JSON. Save.
3. The running game reloads (04 §11). If there are errors they appear over
   the game, in the form of 04 §8.2, and the old definitions stay in force.
4. Look at the result. Open the inspector on it. Trace the rule.
5. Write a test script for it.

### 9.2 Knowing what exists

| Tool | Answers |
|------|---------|
| The reference (`docs`) | What definition types, fields, units, events, verbs, properties, functions, reasons, diagnostics and console commands exist, with their documentation |
| The reference browser | The same, searchable, in the game |
| `show <type> <id>` | What a definition is after every patch, and where each value came from |
| `refs <type> <id>` | What refers to it, and what it refers to |
| `find <type> where=<condition>` | Which definitions match |

### 9.3 Knowing what a mod set does together

`clash-tools compat <profile>` reports, for the mods in a profile: ids
overridden and by whom; fields that two mods patch to different values,
where the later silently wins; references to obsolete definitions;
variables written by one mod and read by none; events raised with no
listeners.

### 9.4 Knowing who can hurt whom

The engine does not balance anything (D-034), so authors need to see what
they have made. `clash-tools matchup <creature> <creature>` prints, for
each attack of each against each part of the other, the preview (07 §13):
chance to land, depth, whether it disables or severs, what it does to the
striker. A creature that nothing in the mod can injure, or that dies to
its own punch, is found here and not in play.

## 10. Maps and worlds

### 10.1 Previews

`map show`, `map check`, `gen plan`, `gen region` and `gen assembly` are
specified in 10 §12.

### 10.2 The map editor

Maps are grids of characters and so is the screen. The map editor is a
screen.

| Part | Does |
|------|------|
| Canvas | One storey of the map; move between storeys; the storey below shown dim |
| Palette | The map's palette: pick a character to paint with; add an entry by choosing a floor, block and fixture from the registries |
| Tools | Point, line, box, fill, pick from the canvas, move and copy a selection |
| Layers | Elements; wiring (10 §3.6); marks; areas; joints |
| Parameters | Set a placement's parameters and seed and see the result |
| Nested maps | Place, turn and preview maps within the map |
| Check | The checks of 10 §3.7, as it is edited |

It reads and writes the map format. Its output is formatted consistently,
so a map edited by hand and then in the editor differs only where it was
changed.

### 10.3 Capturing

`capture_map` writes a box of the live world out as a map: its elements,
wiring and devices, with a palette made for it. Something built in play,
by hand or by helpers, becomes a design that can be planned elsewhere
(16 §10.3), shipped in a mod, or placed by generation.

## 11. Checking that incidents agree

An incident's stages each have a live form and an abstract form (09 §11),
written separately. Nothing in the engine makes them agree.

`clash-tools incident compare <id> --runs <n>` sets the incident up at a
suitable site in a test realm and runs it both ways many times, the live
runs without a display. It compares what each form produces: casualties
by role, stock moved, knowledge added, time taken, devastation. It reports
each measure's spread under both forms and marks those that differ by more
than a stated tolerance.

The same comparison is available for fighting in bulk against fighting in
full (09 §9.1).

An author uses this to tune the abstract form's arithmetic until a robbery
nobody watched tends to end as the ones that were watched do.

## 12. Saves

| Command | Does |
|---------|------|
| `save inspect` | Mods and versions, scenario, realms, sizes by subsystem, counts of entities by form |
| `save verify` | Loads it, runs every invariant checker, and compares the stored hash |
| `save migrate` | Applies format and content migrations (04 §12) and writes the result |

## 13. Aids for players

Built from the same parts, in ordinary screens, so they show only what the
character knows.

| Aid | Is |
|-----|----|
| **Journal** | Objectives and missions (13 §5), with markers |
| **Case file** | For each case the character holds: findings grouped by what they bear on (who, when, how, where), each with its source. The player may mark personas as suspects and mark two findings as bearing on each other. Findings that cannot both be true are flagged. Accusing is done from here (13 §8.5). |
| **Codex** | Entries for kinds of thing the character has appraised or been told about (08 §7.5), generated from their definitions and limited to what was learned |
| **Look** | The inspector's listing for something in view, in the ordinary context: what the character can perceive of it and no more |

The notes a player makes in a case file are the character's own
variables. They are saved, and a rule may read them.

## 14. Logs and failures

- The engine writes a structured log: time, tick, subsystem, severity,
  message, fields.
- Run-time warnings from rules (05 §4.5) go there, limited in rate per
  site, with the rule, path and position.
- In development, warnings also appear in the game.
- A panic in the simulation is an engine bug. The application catches it
  at the top, writes a report (section 8.2) and says so.

## 15. Modding surface

**Definition type**: `console_command`.

**Console commands**: section 3.3, and one for each registered verb.

**Command-line tools** (`clash-tools`): `lint`, `explain`, `show`, `refs`,
`find`, `schema`, `docs`, `init-mod`, `compat`, `matchup`, `map show`,
`map check`, `gen plan`, `gen region`, `gen assembly`, `scenario run`,
`replay`, `diff`, `report`, `incident compare`, `save inspect`,
`save verify`, `save migrate`, `worldview`.

**Screens** in the `debug` context: console, inspector, traces, profiler,
reference browser, map editor. All are definitions and can be replaced or
extended through slots (12 §10.6).

**Effects**: `capture_map`.

## 16. Invariants

| # | Invariant |
|---|-----------|
| Y1 | A tool changes the simulation only through commands, which are logged. |
| Y2 | With traces, profiling and checking off, the simulation's speed and state hash are the same as if the tools did not exist. |
| Y3 | Every registered verb, event, property, function, reason, diagnostic and console command appears in the generated reference with documentation. |
| Y4 | Every refusal carries a reason. |
| Y5 | Every invariant in every specification has a checker, or is listed as covered otherwise. |
| Y6 | A save and its input log replay to the logged hashes. |
| Y7 | A map written by the editor loads, and loading then writing it again changes nothing. |
| Y8 | Ordinary-context screens, including player aids, show nothing the character does not know. |

## 17. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| A console command, excluding what it does | under 50 µs |
| Opening the inspector on a subject | under 2 ms |
| Tracing one kind on one subject | under 5 % added to what is traced |
| `lint` on a single-file change | under 2 s (04 §18) |
| Running all invariant checkers on the default loaded set | under 200 ms |
| Replay | at least as fast as the original run |
| `rewind 5s` | under 1 s |

## 18. Open questions

1. **Stepping through a rule.** Traces show what a rule did. Pausing one
   mid-run and stepping effect by effect is not offered.
2. **A remote inspector.** Attaching the inspector to a game running
   elsewhere, or headless, over a connection.
3. **Editing very large maps.** The editor shows one storey of one map.
   Editing a whole settlement at once needs something more.
4. **Sharing mods.** Packaging, versions and distribution beyond a
   directory are outside this design.
5. **Translation tools.** Follows whatever 04 decides for translation.
