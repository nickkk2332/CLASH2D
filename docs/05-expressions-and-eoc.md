# 05 — Expressions and EOC

Status: design complete, ready for implementation.
Implementation plan: [plans/sim-kernel.md](plans/sim-kernel.md).
Serves scene S12 directly, and every other scene of
[00-vision.md](00-vision.md) indirectly: this is how content makes them
happen.

## 1. Scope

This document specifies the rule system: the expression language, subjects
and properties, variables, events, EOC definitions, effects, queries,
functions, how rules execute, and how they are debugged.

It does not list every event, effect and property in the engine. Each
subsystem document ends with a modding surface section that does that for
its own part. This document defines what those lists plug into, and the
core vocabulary that belongs to no subsystem.

The aim is the power of CDDA's effect-on-condition system with the whole
simulation exposed as hooks. Breadth and regularity of the vocabulary matter
more than brevity.

## 2. The picture

An **EOC** (effect on condition) is a rule: *when* something happens, *if* a
condition holds, *do* a list of effects.

```json
{
  "type": "eoc",
  "id": "vault_alarm",
  "on": "element_destroyed",
  "if": "here.in_area('vault') and not here.site.var.alarm_raised",
  "do": [
    { "set": { "here.site.var.alarm_raised": true } },
    { "message": "An alarm starts to wail." },
    { "for_each": { "entities": { "in_site": "here.site", "where": "it.has_tag('guard')" } },
      "as": "g",
      "do": [
        { "enqueue_action": { "entity": "$g", "action": "investigate", "params": { "at": "here" } } }
      ] },
    { "after": "90 s",
      "do": [ { "emit": { "event": "reinforcements_due", "here": "here" } } ] }
  ]
}
```

The pieces:

- **Events** say what happened. The engine emits them; mods may define their
  own.
- **Subjects** are the things a rule can talk about: `here`, `self`,
  `target`, an area, a site. The event that triggered the rule binds them.
- **Expressions** read the simulation: `here.site.var.alarm_raised`.
- **Effects** change it. Each is a verb with arguments.
- **Variables** hold whatever content wants to remember, on any subject.

Rules are compiled when mods load. Mistakes in them are load errors with
positions (04 §8), not surprises during play.

## 3. Values and types

| Type | Holds | Literal |
|------|-------|---------|
| `bool` | | `true`, `false` |
| `int` | 64-bit integer | `42` |
| `real` | Deterministic float, SI units | `3.5`, `1e6`, `300ms`, `70kPa` |
| `string` | Text | `'ash'` |
| `id<T>` | A definition of type `T` | `'concrete'` where an id is expected |
| `entity` | A reference to an entity, possibly no longer existing | — |
| `loc` | A location (02 §3.2) | — |
| `area`, `site`, `tract`, `region`, `realm`, `subgrid`, `action`, `window` | References to those things | — |
| `list<T>` | Ordered values | `[1, 2, 3]` |
| `null` | Nothing | `null` |
| `any` | Unknown until run time | — |

Variables are stored as `Value`, which can hold any of these. Types are
checked at load wherever they can be known and at run time otherwise.

There is no duration type. Time in expressions is a `real` number of
seconds; the unit literals `300ms` and `2h` are numbers.

## 4. The expression language

Expressions are written as strings inside JSON.

### 4.1 Grammar

```
expr     := 'if' expr 'then' expr 'else' expr | or
or       := and  ( 'or' and )*
and      := not  ( 'and' not )*
not      := 'not' not | compare
compare  := coalesce ( ( '==' | '!=' | '<' | '<=' | '>' | '>=' | 'in' ) coalesce )?
coalesce := sum ( '??' sum )*
sum      := term ( ( '+' | '-' ) term )*
term     := unary ( ( '*' | '/' | '%' ) unary )*
unary    := '-' unary | power
power    := postfix ( '^' unary )?
postfix  := primary ( '.' name | '(' args? ')' | '[' expr ']' )*
primary  := number unit? | string | 'true' | 'false' | 'null'
          | name | '$' name | '(' expr ')' | '[' args? ']'
name     := ident ( ':' ident )?
```

Strings use single quotes so they sit comfortably inside JSON's double
quotes. Whitespace is free. There are no statements and no assignment:
expressions only read.

### 4.2 Names

| Form | Means |
|------|-------|
| `self`, `target`, `here`, … | A subject bound in this context (section 5) |
| `$power` | A local variable or parameter |
| `event.energy` | A field of the triggering event |
| `it` | The candidate inside a query's `where`, `sort` or `weight` |
| `global` | The global scope |
| `clamp(…)`, `ashlands:fall_damage(…)` | A function call (section 4.7, section 12) |

Bare function names resolve like ids (04 §4.3).

### 4.3 Operators

- Arithmetic on `int` and `real`; an `int` meeting a `real` becomes `real`.
  `/` always gives `real`. `div(a, b)` and `%` are integer operations.
- `+` also joins strings and concatenates lists.
- `and`, `or`, `not` take and give `bool`. `and` and `or` short-circuit.
- `in` tests membership in a list or tag set.
- `a ?? b` is `a` unless `a` is `null`, then `b`.

### 4.4 Null

Things go missing: a target dies, a variable was never set, a cell is not
loaded. The rules are:

- A property of `null` is `null`.
- Arithmetic with `null` is `null` and raises a run-time warning naming the
  rule and position.
- A comparison with `null` is `false`, except `== null` and `!= null`.
- A condition that evaluates to `null` counts as `false`.

A rule never crashes because something was missing. Where the compiler can
see that a value may be `null` and is used arithmetically, it warns and
suggests `??`.

### 4.5 Numbers and errors

Division by zero, the square root of a negative number and similar produce
`null` and a run-time warning. Run-time warnings are rate-limited per site
and always name the rule, the JSON path and the position in the string.

### 4.6 Text

`Text` fields are templates: `"{self.name} staggers back."`. Anything in
braces is an expression. `{{` is a literal brace. Numbers may be formatted:
`{event.energy:.0}`.

### 4.7 Built-in functions

| Group | Functions |
|-------|-----------|
| Maths | `min` `max` `clamp` `abs` `sign` `floor` `ceil` `round` `sqrt` `exp` `ln` `log10` `sin` `cos` `atan2` `lerp` `div` |
| Random | `rand(a, b)` `rand_int(a, b)` `chance(p)` `pick(list)` |
| Lists and text | `len` `contains` `first` `last` `sum` `str` |
| Space | `dist(a, b)` `dir(a, b)` `offset(loc, dx, dy, dz)` `los(a, b)` |
| Time | `now()` `local_now(subject)` |
| Data | `setting(id)` `has_def(type, id)` |
| Types | `is_null(x)` `exists(entity)` |

Subsystems add functions through the same registry that holds properties
(section 5.3). Mods add functions as definitions (section 12).

Random functions draw from the simulation's rule stream and are
deterministic. They are not allowed in **pure** contexts (section 7.3).

## 5. Subjects and properties

### 5.1 Subjects

A **subject** is a reference to something in the simulation, of a known
**kind**. Rules never see raw state; they see subjects and ask them for
properties.

| Kind | Refers to |
|------|-----------|
| `entity` | Any entity: creature, item, device (06) |
| `loc` | A cell |
| `element` | A floor, block or fixture of a cell (16 §3.5) |
| `area`, `site`, `tract`, `region`, `realm`, `subgrid` | As in 02 |
| `network`, `job` | As in 16 |
| `ability` | An ability instance (08 §3.2) |
| `group`, `incident` | As in 15 and 09 |
| `action` | A running action (03 §6) |
| `window` | An open window (03 §7) |
| `global` | The whole simulation |

Each context binds some subjects by name. An event binds the ones it
declares (section 8.1). By convention:

| Name | Usually |
|------|---------|
| `self` | The entity the rule belongs to or is about |
| `target` | The entity acted upon |
| `actor`, `instigator` | From a cause (02 §7.3) |
| `here` | Where it happened |

### 5.2 Properties

```
self.loc                    here.block.material.density
self.loc.tract.devastation  here.field('fire')
self.container              here.in_area('vault')
self.has_tag('guard')       here.site.var.alarm_raised
self.action('block').phase  here.realm.var.season
```

A property has a name, a type, a getter, optionally a setter, and
documentation. Properties may take arguments. A property whose type is a
subject kind can be followed further, which is how `self.loc.tract` works.

Definitions are reachable from their instances: `here.block.def.tags`,
`self.def.id`. Comparing an `id<T>` with a string resolves the string at
load: `here.block.material == 'glass'` is checked against the material
registry, and a misspelling is a load error.

### 5.3 The property table

Every property, function, event and effect is registered by the crate that
implements it, with its types and documentation. The registry is what the
compiler checks rules against, what `clash-tools docs` prints, and what the
in-game inspector shows.

```rust
pub struct Property {
    pub kind: SubjectKind,
    pub name: &'static str,
    pub args: &'static [ParamSpec],
    pub ty: Type,
    pub get: fn(&SimView, Handle, &[Value]) -> Value,
    pub set: Option<fn(&mut EffectCtx, Handle, Value) -> Result<(), RuleError>>,
    pub doc: &'static str,
}
```

Each property also lists the events that can change its value, which is
what reactive expressions (section 7.5) subscribe to.

Getters take a read-only view. Setters go through the same functions the
engine uses, so rule R3 holds for rules too.

### 5.4 Truth and belief

Properties read ground truth. That is right for rules of the world: physics,
abilities resolving, scenario logic.

Rules that decide what a **character does** must not read truth the
character could not know (01 R11). Document 15 provides the belief-side
vocabulary: what `self` has perceived, believes and remembers. A rule
attached to a creature that reads `target.loc` directly, rather than where
`self` believes `target` to be, is flagged by the linter.

## 6. Variables

Content stores its own state in **variables** on subjects.

```json
{ "type": "variable", "id": "alarm_raised",
  "scope": "site", "value": "bool", "default": false,
  "doc": "Set once the vault alarm has gone off." }
```

| Scope | Lives on | Written as |
|-------|----------|-----------|
| `entity` | An entity | `self.var.kills` |
| `cell` | A cell (sparse) | `here.var.sealed` |
| `area`, `site`, `tract`, `region`, `realm`, `subgrid` | That thing | `here.site.var.alarm_raised` |
| `action` | A running action | `$power` inside the action |
| `global` | The simulation | `global.var.day_of_reckoning` |
| local | One rule run | `$g` |
| `scenario` | The game in progress (13) | `scenario.var.wave` |
| `legacy` | The player's profile, across games (13 §10.1) | `legacy.var.towers_cleared` |

- `subject.var.name` refers to a variable in the mod's own namespace, or an
  import. `subject.var('other_mod:name')` names one exactly.
- Reading a declared variable that was never set gives its default.
- Declared variables are type-checked, appear in the inspector with their
  documentation, and are migrated by name (04 §12.1).
- Using an undeclared variable works and is a linter warning, because a
  misspelt variable name that silently creates a new variable is the most
  common bug in rule systems of this kind. `"strict_vars": true` in
  `mod.json` makes it an error.
- `"watch": true` on a declaration makes changes emit `var_changed`, so
  rules can trigger when a value changes or crosses a threshold.

All variables are saved.

## 7. Where expressions appear

### 7.1 Expression fields

A field of type `Expr<T>` accepts:

| Written | Meaning |
|---------|---------|
| `12`, `true`, `"250 ms"` | A literal of type `T` |
| `"self.var.kills * 2"` | An expression, when `T` is not a string or id |
| `{ "expr": "…" }` | An expression, always. Needed when `T` is a string or an id. |
| A value source (section 7.4) | A computed value |

### 7.2 Conditions

A condition is an expression of type `bool`, or a structure:

```json
{ "all": [ "self.var.rage > 50", { "any": [ "here.in_area('arena')", "target.has_tag('rival')" ] } ] }
```

`all`, `any` and `not` nest freely and compile to the same thing as the
string form.

### 7.3 Contexts and purity

Each place an expression appears declares its **context**: which subjects
and locals exist, and how pure it must be.

| Purity | Allowed | Used by |
|--------|---------|---------|
| `pure` | Reads only | Screen bindings (12), previews of `requires`, sorting keys |
| `random` | Reads and random draws | Conditions and effect arguments |

Expressions never write. Only effects do.

### 7.4 Value sources

Where a value is wanted and an expression string cannot say it, a **value
source** object can:

| Source | Gives |
|--------|-------|
| `{ "count": query }` | How many |
| `{ "collect": query }` | A list |
| `{ "first": query }` | The first, or `null` |
| `{ "pick": query }` | One at random; with `"weight": "…"`, weighted |
| `{ "sum": query, "of": "it.mass" }` | A total; likewise `min`, `max` |
| `{ "call": "fn_id", "args": { … } }` | A function result |

Queries are defined in section 11.

### 7.5 Reactive expressions

Some fields hold a pure expression whose value must be kept current: a
device's output (16 §7), the `while` condition of a trait or ability
(08 §6.1). These are **reactive**.

- When a reactive expression is evaluated, the engine records which
  properties of which subjects it read.
- Every property is registered with the events that can change it
  (section 5.3). When one of those events occurs for a subject a reactive
  expression read, the expression is marked stale.
- Stale expressions are re-evaluated at the end of the event pump
  (03 §4.3), once each, in a defined order. If the value changed, whatever
  owns the expression is told.

Nothing is polled. An expression whose inputs do not change is never
re-evaluated. Reactive expressions may not call random functions.

## 8. Events

### 8.1 Event types

Every event type is registered with its subjects, its fields, and whether it
is a notification or a hook. In Rust they are named `ElementDestroyed`; in
data, `element_destroyed`.

Mods define their own:

```json
{ "type": "event", "id": "ritual_completed",
  "subjects": { "caster": "entity", "here": "loc" },
  "fields": { "power": "real" },
  "doc": "A ritual circle finished charging." }
```

and raise them with `emit`. Custom events are how mods offer hooks to each
other.

### 8.2 Notifications and hooks

| | Notification | Hook |
|---|---|---|
| Says | Something happened | Something is about to happen |
| Delivered | After the current rule finishes, in order of emission | Immediately, before the operation proceeds |
| Handlers can | React | Change the declared mutable fields, or cancel |
| Named like | `entity_damaged` | `damage_incoming` |

Hooks are what let content reach into outcomes: armour that halves a hit, a
ward that turns it aside, a curse that makes every door refuse to open. A
hook handler uses `modify` and `cancel_event`.

Hooks exist only for operations the simulation layer performs. Operations
inside the world crate (02) cannot be vetoed by rules (decision D-017);
content shapes those through definitions.

### 8.3 Delivery

For each event, handlers run in this order:

1. Global rules listening for that event type.
2. Rules attached to each of the event's subjects, in the order the event
   declares its subjects (section 9.3).

Within each group: higher `priority` first, then by rule id. A handler may
`consume` an event; later handlers marked `"skip_if_consumed": true` are
then skipped.

Notifications raised while handling an event join the back of the queue.
The pump of 03 §4.3 delivers until the queue is empty, subject to the
cascade limit.

### 8.4 Filters

`"filter"` on a rule is a cheap test on an event's discrete fields, applied
before the condition and used to index handlers:

```json
{ "on": "element_destroyed", "filter": { "layer": "block", "material": "glass" } }
```

A rule should filter on what it can, and leave the rest to `if`.

### 8.5 Perceived events

For rules about how characters respond, document 15 provides the event
`perceived`: `self` perceived something. Its filter names the underlying
event. A guard reacting to breaking glass listens for `perceived` with
`{ "event": "element_destroyed" }`, and so reacts only if they heard or saw
it.

## 9. EOC definitions

### 9.1 Fields

| Field | Meaning |
|-------|---------|
| `on` | Event type that triggers the rule. Omit for rules that are only called. |
| `as` | For attached rules: which of the event's subjects the owner must be. Defaults to the event's first subject. |
| `filter` | Section 8.4. |
| `every` | Run repeatedly at this interval instead of on an event (section 9.4). |
| `if` | Condition. Default true. |
| `do` | Effects when the condition holds. |
| `else` | Effects when it does not. |
| `params` | Typed parameters, for rules that are called (section 9.5). |
| `priority` | Order among handlers. Default 0. |
| `once` | Run at most once per owner. |
| `cooldown` | Minimum local time between runs per owner. |
| `skip_if_consumed` | Section 8.3. |
| `ui_callable` | The rule may be run from a screen through the `Ui` command (12 §10.5). Default false. |
| `doc` | Shown in the reference and the trace. |

### 9.2 Global rules

A rule defined at the top level of a data file with an `on` is global: it
runs for every event of that type, anywhere.

### 9.3 Attached rules

Any definition whose instances are subjects may carry `rules`: a list of
rule ids or inline rules. In an attached rule, `self` is the instance.

```json
{ "type": "area_kind", "id": "consecrated_ground",
  "rules": [
    { "on": "area_entered", "if": "target.has_tag('undead')",
      "do": [ { "ignite": { "who": "target" } } ] }
  ] }
```

Attached rules run only for events in which their owner takes part, so a
thousand rules on a thousand creatures cost nothing until one of those
creatures is involved in something.

Rules are also attached at run time with the `attach` and `detach` effects.
A curse is a rule attached to its victim.

### 9.4 Recurring rules

```json
{ "every": "10 s", "while_unloaded": "skip", "if": "…", "do": [ … ] }
```

- An attached recurring rule runs per owner, on the owner's clock. A global
  one runs on the universal clock.
- First runs are staggered by a positional hash of the owner so that a
  crowd does not all fire on the same tick.
- `while_unloaded` says what happens for owners that left the simulation:
  `skip` (default), `once` (run once on return if at least one interval
  passed), or `all` (run for each missed interval, capped).

Prefer events to polling. A recurring rule on many owners is the usual way
rule systems become slow, and the profiler (section 15) will say so.

### 9.5 Parameters and calling

```json
{ "type": "eoc", "id": "ignite",
  "params": { "who": "entity", "heat": { "type": "real", "default": 1.0 } },
  "do": [ { "add_field": { "at": "$who.loc", "field": "fire", "amount": "40 * $heat" } } ] }
```

A rule with parameters can be used as a verb: `{ "ignite": { "who":
"target" } }`. Engine verbs take precedence over rule ids; rule ids resolve
like any reference. `run` calls a rule whose id is computed.

A called rule sees its caller's subjects and its own parameters as locals.
It does not see or change its caller's locals.

Libraries of parameterised rules are how large mods build their own
vocabulary.

## 10. Effects

### 10.1 Form

An effect is an object with one key, the verb, whose value is the
arguments:

```json
{ "damage_cell": { "at": "here", "layer": "block", "kind": "blast", "energy": 5000, "intensity": "20 kPa" } }
```

Verbs with one obvious argument accept it directly: `{ "message": "…" }`,
`{ "run": "some_rule" }`. An unknown verb or argument is a load error with
suggestions.

Effects in a list run in order. Arguments are `Expr` fields, evaluated when
the effect runs.

### 10.2 Control flow

| Effect | Does |
|--------|------|
| `{ "if": cond, "then": […], "else": […] }` | Branch |
| `{ "first": [ { "if": c1, "do": […] }, { "do": […] } ] }` | Run the first entry whose condition holds |
| `{ "switch": expr, "cases": { "a": […], "b": […] }, "default": […] }` | Branch on a value |
| `{ "for_each": query, "as": "x", "do": […] }` | Run for each result, bound to `$x` |
| `{ "repeat": n, "do": […] }` | Run `n` times; `$i` counts |
| `{ "while": cond, "do": […] }` | Loop, within the step budget |
| `{ "weighted": [ { "weight": 3, "do": […] }, … ] }` | Run one entry at random |
| `{ "as": { "self": "$g" }, "do": […] }` | Rebind subjects for a block |
| `{ "stop": {} }` | End this rule |

### 10.3 Variables

| Effect | Does |
|--------|------|
| `{ "let": { "x": expr, "y": source } }` | Define locals |
| `{ "set": { "self.var.kills": expr, "$x": expr } }` | Assign variables, locals and settable properties |
| `{ "add": { "self.var.kills": 1 } }` | Add to numbers; append to lists |
| `{ "unset": [ "self.var.marked" ] }` | Remove variables |

### 10.4 Time

| Effect | Does |
|--------|------|
| `{ "after": "2 s", "do": […] }` | Run later, on the owner's clock |
| `{ "every": "1 s", "times": 5, "do": […] }` | Run repeatedly |
| `{ "cancel_timer": "name" }` | Cancel a timer started with `"name"` |

`after` and `every` take optional `name`, `owner` and `while_unloaded`
(03 §9). The effects inside see the subjects and locals as they were when
the timer was set.

### 10.5 Rules and events

| Effect | Does |
|--------|------|
| `{ "run": { "eoc": id, "with": { … } } }` | Call a rule |
| `{ "attach": { "to": subject, "eoc": id } }`, `detach` | Add or remove an attached rule |
| `{ "emit": { "event": id, … } }` | Raise a custom event |
| `{ "modify": { "event.energy": expr } }` | In a hook: change a mutable field |
| `{ "cancel_event": {} }` | In a hook: prevent the operation |
| `{ "consume": {} }` | Mark the event handled |

### 10.6 The player

| Effect | Does |
|--------|------|
| `{ "message": { "text": …, "to": subject, "style": … } }` | Put a line in the message log of whoever controls `to`; default `self` |
| `{ "ask": { "who": entity, "prompt": text, "options": [ { "label": …, "if": …, "do": […] } ] } }` | Stop and offer a choice (03 §5.2). Each option carries the effects to run if chosen. For entities not under direct control, the first available option is taken unless a rule says otherwise. |

Opening screens and other interface effects belong to document 12.

### 10.7 Subsystem verbs

Everything else is registered by the subsystem that implements it and listed
in that document's modding surface: the world's verbs in 02 §23, time and
action verbs in 03 §13, and so on. The generated reference lists them all in
one place.

A verb's implementation calls the same engine functions as native code and
is bound by the same rules. There is no back door.

## 11. Queries

A query selects a set of things.

```json
{ "entities": { "around": "self", "radius": 12,
                "where": "it != self and it.has_tag('guard')",
                "sort": "dist(it, self)", "limit": 3 } }
```

| Query | Selects | Shaped by |
|-------|---------|-----------|
| `entities` | Entities in the world | `around` + `radius`, `box`, `in_area`, `in_site`, `at`; `plane` |
| `cells` | Locations | `around` + `radius`, `box`, `line`, `cone`, `in_area` |
| `contents` | Entities inside an entity | `of`; `deep` |
| `areas`, `sites` | Those containing or near a location | `at`, `around` |
| `actions`, `windows` | Running actions of an entity; windows open to it | `of` |
| `defs` | Definitions of a type | `type` |
| `list` | Elements of a list value | — |

Document 06 adds `parts` and others.

Every query accepts `where` (a condition on `it`), `sort` (an expression on
`it`, ascending), `limit`, and for `pick`, `weight`.

Results have a defined order: by `sort` if given, otherwise nearest first
for spatial queries, with ties broken by id. Rules that iterate get the same
order every run.

Spatial queries cover loaded space only. Their extent is capped
(`rules.max_query_cells`); a query that would exceed the cap is a load error
if its size is constant and a run-time warning otherwise.

## 12. Functions

A **function** is a named, parameterised value.

```json
{ "type": "function", "id": "auto_part",
  "params": { "of": "entity" },
  "returns": "part",
  "value": { "pick": { "parts": { "of": "$of", "where": "it.exposed" } }, "weight": "it.size" } }
```

`value` is an expression or a value source. Functions are called from
expressions (`auto_part($target)`) and may call each other. Recursion is
allowed to a fixed depth.

A function is pure unless it uses a random function or `pick`, in which case
it is random and cannot be used in pure contexts. Functions never change
state.

Functions are how content supplies the defaults of decision D-037, scoring
for AI (09), derived statistics, and any formula used in more than one
place.

## 13. Execution

### 13.1 A run

```rust
pub struct Run<'a> {
    pub rule: EocId,
    pub owner: Option<Handle>,
    pub subjects: SubjectMap,        // name → handle
    pub locals: VarMap,
    pub event: Option<&'a mut Event>,
    pub depth: u8,
    pub steps_left: u32,
}
```

A run evaluates `filter`, then `if`, then executes `do` or `else`.

### 13.2 Limits

| Limit | Default | On exceeding |
|-------|---------|--------------|
| Steps per run (effects executed plus loop iterations) | 100,000 | Run ends; error reported |
| Call depth | 32 | Call refused; error reported |
| Hook nesting depth | 8 | Inner hook skipped; error reported |
| Events per pump | 10,000 | 03 §4.3 |

Exceeding a limit is always an error report with a trace, and never a hang
or a crash.

### 13.3 Determinism

- Random draws come from one named stream in simulation state.
- Handler order, query order and effect order are all defined.
- Rules cannot observe wall-clock time, thread identity or memory addresses.

### 13.4 Continuations

`after` and `every` store a **continuation**: the rule id, the path to the
effect list inside it, the subjects and the locals. Continuations are plain
data. They are saved, survive loading, and survive a definition reload if
the path still exists.

### 13.5 Level of detail

- Event-triggered rules run only for events, and events come only from
  simulated things. Nothing cold raises cell-level events.
- Attached rules on a stored entity do not run. Recurring ones follow
  `while_unloaded`.
- The cold simulation (09) has its own events at its own scale, and rules
  may listen to them.

## 14. Compilation

At load, for every `Expr`, `Cond`, `Effects` and `Text` field:

1. Parse to a syntax tree, keeping positions.
2. Resolve names against the field's context: subjects, locals, event
   fields, properties, functions, verbs, variables, ids.
3. Check types.
4. Fold constants.
5. Emit: expressions to a compact bytecode for a small stack machine;
   effects to a tree of compiled nodes that the interpreter walks with an
   explicit stack, so any position in it can be named by a path.

Compiled rules live in `Defs` and are immutable.

## 15. Tracing and debugging

Rules that cannot be debugged are not a feature.

| Tool | Shows |
|------|-------|
| **Trace** | For a chosen rule, subject or event type: every run, with its trigger, bindings, the value of each sub-expression of its condition, every effect with its evaluated arguments, and every event it raised. Viewable in the in-game inspector and written to a log. |
| **Explain** | For one condition against the current state: a tree of sub-expressions and their values. "Why did this not fire?" |
| **Profile** | Runs, steps and time per rule; handlers per event type. Finds the recurring rule that is eating the frame. |
| **Console** | Evaluate an expression or run an effect list against chosen subjects, live. |
| **Assertions** | Scenario scripts (plans/world-model W11) can assert expressions, so mods can ship tests of their own rules. |

Run-time warnings and errors carry the rule id, the JSON path, the position
in the expression, and the values of the subjects involved.

## 16. The script hook

Reserved, not built. When it is:

- A script provider registers additional verbs and functions under its own
  namespace, through the registry of section 5.3.
- Scripts read through properties and write through verbs. They get no
  other access.
- They are sandboxed, deterministic (no clock, no I/O, no unseeded
  randomness) and metered by the same step budget.

Nothing in the engine or the bundled content may depend on it.

## 17. Modding surface

**Definition types**: `eoc`, `function`, `variable`, `event`.

**Core events**: `sim_started`, `entity_created`, `entity_destroyed`,
`var_changed`, `setting_changed`, `rule_error`.

**Core effects**: sections 10.2 to 10.6.

**Core functions**: section 4.7.

**Core properties**, available on every subject kind: `var`, `has_tag`,
`tags`, `def`, `kind`, and for anything with a position, `loc`.

## 18. Invariants

| # | Invariant |
|---|-----------|
| E1 | Expressions never change state. |
| E2 | Rules change state only through verbs, and verbs only through the engine's own mutation paths. |
| E3 | Every name in a rule is resolved at load. A rule that loads cannot fail at run time for a name that does not exist. |
| E4 | A rule run always terminates. |
| E5 | The order of handlers, query results and effects is defined and identical across runs. |
| E6 | Everything a rule can leave behind (variables, attachments, timers, continuations) is saved. |
| E7 | Every registered event, verb, property and function appears in the generated reference. |
| E8 | No run-time condition in a rule panics the simulation. |

## 19. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Evaluate a condition of about ten operations | under 300 ns |
| Deliver an event with no handlers | under 50 ns |
| Start a rule run (bindings, filter) | under 500 ns |
| Simple effect (set a variable) | under 200 ns plus its expressions |
| Compile 20,000 rules | under 1 s |
| Tracing disabled | no measurable cost |

## 20. Open questions

1. **Maps as values.** Lists are in; keyed maps are not. Add if content
   needs them.
2. **Returning values from rules.** Rules do not return; functions do. If
   content keeps wanting a rule that both acts and answers, add `out`
   parameters.
3. **Lambdas.** Queries cover filtering, sorting and weighting. Anonymous
   functions in expressions are deliberately left out.
4. **Rule sets as toggles.** Enabling and disabling groups of rules through
   settings without `if` on each one.
