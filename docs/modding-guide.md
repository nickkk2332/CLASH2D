# Modding guide

A walkthrough for a creator: make a mod, give it an item, a creature, a
power, a rule, a change to the screen and a game of its own, then check it
and test it. It needs no Rust.

Every command runs from the root of the repository. `cargo run -p
clash-tools --` is the tool suite (`docs/14-tooling.md` §9); `help` lists
its commands. What each definition type, field, event, verb and function
means is in the [modding reference](reference/README.md), which is
generated from the engine itself, so it says what the engine you have
reads.

This guide is run by the tests of `clash-tools`: they make its mod, write
each file it shows, and run each `clash-tools` command in it, in order. If
something here stops working, a test fails.

## What a mod is

Everything a game has is in mods: folders of JSON under `mods/`. The
engine knows mechanisms (materials, bodies, pools, actions, events,
effects) and names no material, creature or power of its own. A mod is

- `mod.json`: its id, version, and the mods it requires;
- `data/`: its definitions, in files laid out as you like. Each is a JSON
  object with a `type` (one of the [definition
  types](reference/types/README.md)) and an `id`, and `"//"` for a
  comment;
- `profiles/`, if it has any: a profile names the mods a game starts with
  and the screens it uses (04 §3.3).

The repository bundles three: `core` (a world: materials, bodies,
creatures, items, powers, and three game scenarios), `ui` (the screens)
and `core_ui` (core's parts of those screens).

A definition's full id is its mod's id, a colon and its own:
`hearth:ladle`. Inside a mod a bare id is looked up in that mod, then in
the mods it requires (04 §4.3), so `steel` written in `hearth` finds
`core:steel`.

## 1. Make a mod

```bash
cargo run -p clash-tools -- init-mod hearth
```

This makes `mods/hearth`:

| File | What it is |
|------|------------|
| `mod.json` | The mod's id, version and description. It requires `core` and `ui`. |
| `data/items.json`, `data/creatures.json`, `data/actions.json`, `data/rules.json`, `data/screens.json` | A small example of each common kind of definition, each with a comment saying what it is: an item, a creature template, an action, a variable with a rule that keeps it, and a line on the play screen. Change them or delete them. |
| `profiles/hearth.json` | A profile: the repository's default, with `hearth` added. |
| `.schemas/data.schema.json`, `.vscode/settings.json` | The JSON Schema of a data file, attached to `data/` for editors that read JSON Schema, so that keys, values and units are checked as they are typed. |

`init-mod` will not write over a mod that is there: to follow this guide
again, delete `mods/hearth` first.

Check it:

```bash
cargo run -p clash-tools -- lint mods/hearth
```

`lint` runs the whole loading pipeline and what each part of the engine
builds from the definitions, and prints every
[diagnostic](reference/diagnostics.md) with its file, line and path. `ok:`
and exit code 0 mean the mod is clean. Every code explains itself:

```bash
cargo run -p clash-tools -- explain E0203
```

## 2. Find your way

The [reference](reference/README.md) has a page for each definition type,
with every field: what it takes, its unit, its default, whether it is
required, what it refers to and what it means. Beside them are pages of
[events](reference/events.md), [hooks](reference/hooks.md),
[verbs](reference/verbs.md), [effects](reference/effects.md),
[functions](reference/functions.md),
[properties](reference/properties.md), [units](reference/units.md) and
[how values are written](reference/values.md).

The tools answer questions about the loaded mods. `new` prints a
definition of a type with every field it requires; `show` prints a
finished definition and where each value came from; `find` lists the
definitions of a type that meet conditions, here items over a kilogram:

```bash
cargo run -p clash-tools -- new item ladle
cargo run -p clash-tools -- show template core:dog
cargo run -p clash-tools -- find item "where=mass>1"
```

`refs` lists what refers to a definition (section 4 uses it), and
`schema --with-ids` writes schemas that also know the ids of what is
loaded.

## 3. An item

Add `mods/hearth/data/kitchen.json`:

```json mods/hearth/data/kitchen.json
[
  { "//": "A ladle: light steel, long enough to reach the bottom of a pot. What it strikes with is its bowl, a broad blunt surface.",
    "type": "item", "id": "ladle", "name": "ladle",
    "material": "steel", "mass": "0.3 kg", "length": "0.35 m",
    "surfaces": [
      { "id": "bowl", "kind": "impact", "area": 2.0e-3, "give": 0.005, "span": 0.08 } ],
    "grip": { "arm": 1 },
    "tags": ["kitchen"],
    "glyph": { "ch": "?", "fg": "white" } }
]
```

- A quantity is written with any unit of its dimension (`"0.3 kg"`,
  `"350 g"`) and kept in SI units; a bare number is already in them. The
  [units page](reference/units.md) lists them.
- `surfaces` are what the item can strike with: a
  [damage type](reference/types/damage_type.md), a contact area in square
  metres, and how far a blow with it gives (see
  [`SurfaceDef`](reference/types/item.md#surfacedef)).
- `grip` is the channel slots it takes to hold: one arm.

Every field is on the [item page](reference/types/item.md).

## 4. A creature

A creature is made from a template: the body it is built on, its pools
(reserves such as blood), its attributes, its mind and how it is drawn.
The easiest way to make one is to start from another. Add
`mods/hearth/data/animals.json`:

```json mods/hearth/data/animals.json
[
  { "//": "A sheepdog: core's dog with a little more strength and wits and a white coat. `copy_from` takes every field of `core:dog`, its mind included; the fields written here replace what it had.",
    "type": "template", "id": "sheepdog", "copy_from": "core:dog", "name": "Sheepdog",
    "attributes": { "strength": 7, "wits": 4 },
    "glyph": { "ch": "d", "fg": "white" } }
]
```

`copy_from` works for every type (04 §6.2), and so do `abstract`,
`patch` and the other [keys every definition
carries](reference/types/README.md#keys-every-definition-carries). See
who uses `core:dog` now:

```bash
cargo run -p clash-tools -- refs template core:dog
```

The [template page](reference/types/template.md) has the rest: a
template may also offer choices for the creation screen, give abilities,
and apply another template on top (`kind: role`).

## 5. A power

A power is an [ability](reference/types/ability.md): something a
character holds, with a rank, that makes actions available at a cost and
with a cooldown (08). This one has a character get their breath back. Add
`mods/hearth/data/powers.json`:

```json mods/hearth/data/powers.json
[
  { "//": "What the power does: a moment's pause, then the stamina it was given back. `amount` is a parameter, which the ability supplies.",
    "type": "action", "id": "catch_breath", "name": "Catch your breath",
    "params": { "amount": "real" },
    "tags": ["power"],
    "phases": [
      { "id": "pause", "time": "1 s", "cancel": true },
      { "id": "recover", "time": 0,
        "do": [ { "pool_add": { "who": "self", "pool": "stamina", "amount": "$amount" } } ] } ] },

  { "//": "The power: it makes the action available, giving back more stamina at higher ranks, once a minute.",
    "type": "ability", "id": "second_wind", "name": "Second wind",
    "description": "Stop for a moment and get your breath back.",
    "params": { "amount": "20 + 10 * ability.rank" },
    "grants": [
      { "action": "catch_breath", "params": { "amount": "ability.amount" }, "cooldown": "60 s" } ] },

  { "//": "Someone who has it: core's person, with stamina to spend and win back, given the power by the template.",
    "type": "template", "id": "herder", "copy_from": "core:person", "name": "Herder",
    "pools": ["blood", "pain", "stamina"],
    "abilities": ["second_wind"] }
]
```

- The [action](reference/types/action.md) does the work, in phases; a
  phase's `do` is a list of effects. `pool_add` is a
  [verb](reference/verbs.md#pool-add): each verb's page entry gives its
  arguments and types, and why it may be refused.
- The ability's `params` are numbers worked out from its rank; a grant
  passes them on to the action, which reads them as `$amount`. A grant
  may also have a `cost`, such as `{ "stamina": 10 }`, paid from a pool
  (see [`AbilityGrantDef`](reference/types/ability.md#abilitygrantdef)).
- Only a character with the `stamina` pool has stamina to win back, so
  the `herder` template lists it, and gives the ability.

## 6. A rule that reacts to an event

A rule (an `eoc`) runs effects when an event is delivered and its
condition holds (05 §9). Add `mods/hearth/data/breath.json`:

```json mods/hearth/data/breath.json
[
  { "//": "A count kept on each character, and a rule that keeps it: whenever anyone uses the power, its count grows and they are told. `self` is the event's subject, the user; `event.ability` is a field of `ability_used`.",
    "type": "variable", "id": "second_winds", "scope": "entity", "value": "int", "default": 0,
    "doc": "How many times a character has caught their second wind." },
  { "type": "eoc", "id": "count_second_winds",
    "on": "ability_used", "if": "event.ability == 'hearth:second_wind'",
    "doc": "Counts each use of the power, and tells its user.",
    "do": [
      { "add": { "self.var.second_winds": 1 } },
      { "message": { "text": "You get your second wind.", "to": "self" } } ] }
]
```

The [events page](reference/events.md#ability-used) says what each event
carries: its subjects and its fields. A rule on a
[hook](reference/hooks.md) runs before the thing happens and may change
or stop it; the hooks page says what a handler of each may change. The
[effects page](reference/effects.md) lists `add`, `set`, `if`, `for_each`
and the rest; [expressions](reference/values.md#expressions) are
documented in 05. Expressions that a character decides by may read only
what that character believes: the [functions](reference/functions.md)
and [properties](reference/properties.md) pages say which are safe for
belief.

## 7. A change to the screen

The screens are definitions too, in the `ui` mod, and they leave slots
for content. Add a bar of stamina under core_ui's blood and pain, in
`mods/hearth/data/breath_bar.json`:

```json mods/hearth/data/breath_bar.json
[
  { "//": "A bar of stamina in the play screen's status column, below core_ui's blood and pain, shown only to a character who has stamina.",
    "type": "ui_part", "id": "stamina_bar", "into": "ui:play.bars", "order": 30, "required": false,
    "if": "not is_null(view.pool.stamina)",
    "widget": { "bar": { "label": "Breath", "value": "view.pool.frac('stamina')", "style": "bar" } } }
]
```

`view` is the character the player sees through, read as they believe it.
The [ui_part page](reference/types/ui_part.md) and
`docs/12-presentation-and-ui.md` §10 say how screens, widgets, parts and
slots work (§10.6 for extending a screen); a mod may also patch a screen
or replace it in its profile.

## 8. Change what another mod defines

A mod changes another's definitions with `patch`, field by field, in load
order (04 §6.3). Add `mods/hearth/data/patches.json`:

```json mods/hearth/data/patches.json
[
  { "//": "Core's dog, changed in place: in a game with this mod every dog also carries the tag `herding`. `extend` adds to a list rather than replacing it; `scale`, `add` and `delete` work the same way (04 §6.1).",
    "type": "template", "id": "core:dog", "patch": true,
    "extend": { "tags": ["herding"] } }
]
```

A patch must load after what it changes: `hearth` requires `core`, so it
does. `override: true` replaces a definition outright.

## 9. A game scenario

A game scenario is a game the player can start from the new-game screen:
the world it makes, who the player is and where they begin, what they are
trying to do and how it ends (13). Add `mods/hearth/data/morning.json`:

```json mods/hearth/data/morning.json
[
  { "//": "A small game: the herder wakes in the yard at six with a sheepdog beside them, and has to find the ladle before breakfast. `start.do` makes the ladle a few steps away and keeps it in a global variable the marker reads; the objective is done when the herder has it, and the game then ends.",
    "type": "scenario", "id": "morning", "name": "Morning at the hearth",
    "doc": "Find the ladle before breakfast.",
    "intro": "It is six in the morning. Somebody has left the ladle in the yard again.",
    "worlds": [ { "realm": "core:testland", "seed": 1 } ],
    "start": {
      "characters": [ { "template": "herder" } ],
      "at": [ { "name": "The yard", "cell": [10, 10] } ],
      "time": { "day": 1, "at": "06:00" },
      "do": [
        { "create_item": { "def": "ladle", "at": "offset(start_cell, 4, 0, 0) ?? start_cell" } },
        { "set": { "global.var.ladle": { "first": { "entities": { "around": "self", "radius": 6, "def": "hearth:ladle" } } } } },
        { "spawn": { "template": "sheepdog", "at": "offset(start_cell, -2, 0, 0) ?? start_cell" } } ] },
    "objectives": [
      { "id": "find_ladle", "text": "Find the ladle.",
        "done": "taker.has_item('hearth:ladle')",
        "marker": { "last_known": "global.var.ladle" } } ],
    "ends": [
      { "on": "objective_changed", "filter": { "id": "find_ladle", "state": "done" }, "as": "found" } ] },

  { "type": "variable", "id": "ladle", "scope": "global", "value": "entity", "belief_safe": true,
    "doc": "The ladle of `hearth:morning`." }
]
```

- `worlds` names the [realm](reference/types/realm.md) to make and its
  seed: a number, `random`, or `ask`.
- `start` says who the party is, where they may begin and when, and runs
  `do` once they are placed, with `self` the first of them and
  `start_cell` where they stand.
- An objective's `done` and `failed` are conditions that hold of the
  world; its `marker` points where the taker knows the thing to be.
- `ends` ends the game on an event or a condition, naming an outcome.

The [scenario page](reference/types/scenario.md) has every field, and the
[mission page](reference/types/mission.md) the missions a scenario can
start. The options a scenario asks the player about are
[settings](reference/types/setting.md).

## 10. Lint and play

```bash
cargo run -p clash-tools -- lint mods/hearth
cargo run -p clash-tools -- lint hearth
```

The first checks the mod with the mods it requires; the second checks its
profile, which is what a game loads. Then play it:

```bash
cargo run -p clash-app -- --profile hearth
```

The new-game screen lists every game scenario the loaded mods define,
`Morning at the hearth` among them.

## 11. Test it with a test script

A test script is a console session with expectations, ending in the hash
of the whole state of the simulation (14 §7). Add
`mods/hearth/tests/morning.scn`:

```text mods/hearth/tests/morning.scn
# The morning at the hearth: the game begins as the scenario says, the
# power gives back stamina and the rule counts it, and picking up the
# ladle wins.
use hearth seed=1
begin hearth:morning
assert "scenario_id() == 'hearth:morning'"
assert "@self.has_ability('hearth:second_wind')"
advance 1s
assert "@self.objectives[0].where == 'east, 4 m'"
as @self do { "pool_add": { "who": "self", "pool": "core:stamina", "amount": -60 } }
assert "@self.pool.level('core:stamina') == 40"
use_ability who=@self ability=hearth:second_wind
advance 2s
assert "@self.var('hearth:second_winds') == 1"
assert "@self.pool.level('core:stamina') > 60"
as @self do { "take": { "who": "self", "what": { "first": { "entities": { "around": "self", "radius": 6, "def": "hearth:ladle" } } } } }
advance 1s
assert "objective_state('find_ladle') == 'done'"
expect event scenario_ended
expect hash
```

`use` starts a profile, `begin` a game scenario, and every verb is a
command; `assert` tests an expression, `expect event` that an event was
delivered. The last line has no hash yet: `--bless` writes it in. Then
run it:

```bash
cargo run -p clash-tools -- scenario run mods/hearth/tests --bless
cargo run -p clash-tools -- scenario run mods/hearth/tests
```

Each script is run four ways (straight, twice, with a save and a load at
every step, and with a different number of threads) and must reach the
same hash in all. When you change the mod on purpose, bless again and
look at what changed. To watch a script, give its lines to the world
viewer:

```bash
grep -v '^expect' mods/hearth/tests/morning.scn | cargo run -p clash-tools -- worldview
```

`help` in the viewer lists the console's commands; `eval "<expression>"`
prints a value.

## Where next

- The [reference](reference/README.md): every type, field, event, hook,
  verb, effect, function, property, query, setting, reason, unit and
  diagnostic code.
- `docs/04-data-and-modding.md`: loading, ids, inheritance and patching,
  profiles, diagnostics.
- `docs/05-expressions-and-eoc.md`: expressions, rules, events, hooks and
  effects.
- `docs/13-scenarios-and-events.md`: game scenarios, missions and
  endings.
- The bundled mods under `mods/`, whose comments say what each
  definition is for.
