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
    { "emit_signal": { "at": "here", "kind": "sound", "strength": 40, "about": "alarm" } },
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
| `map<T>` | Values by key; keys are strings or ids | `{'ash': 3, 'iron': 1}` |
| `null` | Nothing | `null` |
| `any` | Unknown until run time | — |

Variables are stored as `Value`, which can hold any of these. Types are
checked at load wherever they can be known and at run time otherwise.

There is no duration type. Time in expressions is a `real` number of
seconds; the unit literals `300ms` and `2h` are numbers.

Two more types exist for the checker's benefit. `number` is "an `int` or a
`real`, not known until run time": what arithmetic on `any` gives, and what
a parameter that takes either is declared as. `T?` is "a `T`, or null":
the type of a property that can be missing, of a map entry and of a list
element. An `int` is accepted wherever a `real` is wanted and stays an
`int`; whoever needs a real converts it.

A map's keys are texts. An id used as a key counts as its qualified id, so
`m[here.block.material]` and `m['core:glass']` read the same entry. A map
iterates, prints and hashes in order of key.

The expression crate has its own `Value`, which holds all of these. A
reference to a subject is a **handle**: its kind and 64 bits that mean
something only to the simulation. An id is its definition type and the
dense number of 04 §7.2.

`clash_core::Value` is the value as a variable stores it (decision
D-463). It holds the same things in a form that can be saved: the four
plain kinds, a list, a map, an id as two numbers, a handle as a number
and 64 bits, and null, which occurs only inside a list or a map because
a variable that holds nothing is not set. The two convert into each
other without loss (`Value::to_stored`, `Value::from`) and hash the
same. A stored value nests lists and maps at most 32 deep.

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
          | '{' ( entry ( ',' entry )* )? '}'
args     := expr ( ',' expr )*
entry    := expr ':' expr
name     := ident ( ':' ident )?
```

Strings use single quotes so they sit comfortably inside JSON's double
quotes. A quote inside a string is written twice: `'it''s'`. There are no
other escapes; a line break or a tab comes from JSON's own. Whitespace is
free. There are no statements and no assignment: expressions only read.

Lexical details:

- An `ident` is ASCII letters, digits and `_`, not starting with a digit.
  `if then else and or not in true false null` are reserved.
- A number with a point or an exponent is a `real` (`3.5`, `1e6`); without
  either it is an `int`. A digit is needed on both sides of the point.
  There are no negative literals: `-5` is the negation of `5`.
- `namespace:name` is one name only when nothing separates its parts. A
  colon followed by anything but a letter is a colon, which is what lets
  `{x: 1}` be a map and `{event.energy:.0}` carry a format.
- A unit follows its number directly (`300ms`) or after spaces (`300 ms`).
- Comparisons do not chain: `a < b < c` is a syntax error.

Limits, so that nothing a mod writes can exhaust the compiler: brackets,
`if`, `not` and unary minus nest at most 32 deep; a syntax tree is at most
100 operations deep, which is also the longest chain of one operator; a
source string is at most 65,536 bytes. Exceeding one is a load error.

### 4.2 Names

| Form | Means |
|------|-------|
| `self`, `target`, `here`, … | A subject bound in this context (section 5) |
| `$power` | A local variable or parameter |
| `event.energy` | A field of the triggering event |
| `it` | The candidate inside a query's `where`, `sort` or `weight` |
| `global` | The global scope |
| `clamp(…)`, `ashlands:fall_damage(…)` | A function call (section 4.7, section 12) |

Bare function names resolve like ids (04 §4.3), except that an engine
function, whose name has no namespace, is found first: `clamp` is always
the engine's. A name is a function only where it is called; a bare name
that is not called is a bound subject. `subject.var` is not a property: it
is followed by a variable's name (section 6).

### 4.3 Operators

- Arithmetic on `int` and `real`; an `int` meeting a `real` becomes `real`.
  `/` always gives `real`, and so does `^`: a whole exponent is repeated
  multiplication, so `x ^ 2` is exactly `x * x`. `div(a, b)` and `%` are
  integer operations that round toward minus infinity, so that
  `div(a, b) * b + a % b == a` and `-7 % 2` is `1`.
- `+` also joins strings and concatenates lists. A text is not joined with
  anything else: use `str`.
- `and`, `or`, `not` take and give `bool`. `and` and `or` short-circuit.
  Null counts as false in all three, as it does in a condition.
- `==` and `!=` compare numbers by value (`1 == 1.0`), an id with an id of
  the same type or with a text (its qualified id), lists and maps by their
  contents, and subjects by identity. Comparing types that can never be
  equal is a load error. `<`, `<=`, `>`, `>=` order two numbers or two
  texts.
- `in` tests membership in a list or tag set, or among the keys of a map.
- `a ?? b` is `a` unless `a` is `null`, then `b`.
- `m['key']` reads a map entry, `null` if absent, and `m.key` is the same
  where `m` is a map. `set` and `add` (section 10.3) may address one:
  `"self.var.tally['ash']"`.
- Operands are evaluated left to right, an object before its arguments.

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

The rows in detail:

- An element or an entry read from `null`, and a variable of `null`, are
  `null` as a property is. The host is not asked.
- "Arithmetic" covers the arithmetic operators, unary minus and a function
  that needs a value (`sqrt(null)`). `is_null`, `str` and `??` take null as
  it is. Each operation that meets a null warns, so a null that passes
  through three operations is reported at three positions; the first is
  where it came from.
- `== null` and `!= null` are recognised where they are written: one side
  is the word `null` or something the compiler knows is always null. Any
  other comparison in which a side turns out null at run time is false,
  `!=` included, and membership (`in`) counts as a comparison.
- Null counts as false wherever a `bool` is wanted: a condition, the
  operands of `and`, `or` and `not`, and the test of `if`.

### 4.5 Numbers and errors

Division by zero, the square root of a negative number and similar produce
`null` and a run-time warning. Run-time warnings are rate-limited per site
and always name the rule, the JSON path and the position in the string.

"Similar" is: the logarithm of a number that is not positive; a power with
no real value; a result that is not a number (infinity minus infinity);
`int` arithmetic that leaves the range of a 64-bit integer; a random
function with no stream to draw from; a list, map or text that would pass
`rules.max_value_size`; and a value whose type the operation does not
accept, which can only happen where the type was `any` at load. A
comparison or a condition that faults is false rather than null. Infinity
is a number and passes through.

A **site** is one operation of one expression and one kind of fault. Its
first three warnings are reported, and after that those whose count is a
power of two, with the count: a site that warns a million times is
reported twenty-one times. The limit counts occurrences, not seconds,
because the simulation has no wall clock and the same run must report the
same warnings.

An operation on constants that would fail every time (`1 / 0`) is a
warning at load and is left to fail, and warn, when it runs.

### 4.6 Text

`Text` fields are templates: `"{the(self)} staggers back."`. Anything in
braces is an expression. `{{` and `}}` are literal braces; a lone `}` is a
load error. Numbers may be formatted: `{event.energy:.0}`. The format is
`.N`, for N digits after the point, N from 0 to 17; it applies to numbers
and leaves other values as they are. Because `{{` is a literal brace, a map
literal that opens a template expression is written in brackets:
`{({'a': 1})['a']}`. Inline styles are written as tags (12 §4.3); the
expression compiler passes them through as text.

**Who is reading.** Text shown to a player is written for a **reader**:
the controlled character in whose log or on whose screen it appears. The
same line may read differently to different readers, because they know
different things.

| Helper | Gives |
|--------|-------|
| `name(x)` | What the reader calls `x`. "you" if `x` is the reader. The name of the persona the reader perceives `x` as, if the reader knows it. Otherwise a description of what the reader made out (15 §8.4): "a tall figure in red". For a thing whose kind the reader has not learned, its definition's `unknown_name`. |
| `the(x)`, `a(x)` | The same with an article where one belongs: "the guard", "a guard", "Mara", "you" |
| `they(x)`, `them(x)`, `their(x)`, `themself(x)` | Pronouns, from the persona's set (15 §8.1); "you" and "your" for the reader |
| `verb(x, word)` | The verb agreeing with `x`: "you hit", "the guard hits" |
| `count(n, word)`, `plural(word)` | "3 arrows", "arrows" |
| `list(xs)` | "a knife, a rope and two coins" |
| `cap(text)` | A capital first letter. The first letter of a finished line is capitalised without it. |

```json
"log": "{the(self)} {verb(self, 'drive')} {their(self)} blade into {the(target)}."
```

reads "You drive your blade into the guard" to the one doing it, and "A
tall figure in red drives their blade into Mara" to someone watching who
knows only one of them.

`x.name` is the true name. Reading it for anyone but the reader in a
message, a log line, a caption or a line of speech is a lint warning: it
tells the player something their character may not know (01 R11). Debug
screens may use it freely.

Articles, plurals and verb forms come from a `language` definition. One is
bundled for English. Translation (04 §20) would supply others.

### 4.7 Built-in functions

| Group | Functions |
|-------|-----------|
| Maths | `min` `max` `clamp` `abs` `sign` `floor` `ceil` `round` `sqrt` `exp` `ln` `log10` `sin` `cos` `atan2` `lerp` `div` |
| Random | `rand(a, b)` `rand_int(a, b)` `chance(p)` `pick(list)` |
| Lists, maps and text | `len` `contains` `first` `last` `sum` `str` `keys` `values` `has`; the text helpers of section 4.6 |
| Space | `dist(a, b)` `dir(a, b)` `offset(loc, dx, dy, dz)` `los(a, b)` |
| Time | `now()` `local_now(subject)` |
| Data | `setting(id)` `has_def(type, id)` |
| Types | `is_null(x)` `exists(entity)` |

Subsystems add functions through the same registry that holds properties
(section 5.3). Mods add functions as definitions (section 12).

`xs[i]` reads a list element, counting from zero; out of range gives
`null`. `m['key']` reads a map entry. A map is written `{'ash': 3}`.
`without(xs, x)` and `without_key(m, k)` give copies with an element or a
key removed. Inside an expression only simple units may follow a number
(`ms`, `kg`, `kPa`); compound ones such as `m/s` are written in JSON unit
strings. The simple units are those of 04 §7.1 that are a scale: `us ms s
min h d`, `mm cm m km`, `ml L`, `g kg t`, `J kJ MJ GJ`, `W kW MW`, `N kN`,
`Pa kPa MPa GPa` and `K`. `C` is an offset and is not accepted, because
`-5C` would read as minus 278.15. A number with a unit is a `real` in SI
units.

Random functions draw from the simulation's rule stream and are
deterministic. They are not allowed in **pure** contexts (section 7.3).

The functions that belong to no subsystem, with their arguments. `number`
is an `int` or a `real`. All are safe for belief except `exists`. Given
null where a value is needed, each gives null with a warning.

| Function | Gives |
|----------|-------|
| `min(a: number, b: number, …)`, `max(…)` | The smallest or largest of two or more numbers; an `int` if all are |
| `clamp(x: number, low: number, high: number)` | `x` limited to the range; `high` if `low` is above it; an `int` if all three are |
| `abs(x: number)` | The magnitude, of the type of `x` |
| `sign(x: number)` | `int`: `1`, `-1` or `0` |
| `floor(x: number)`, `ceil(x)`, `round(x)` | `int`. `round` takes halves away from zero. Null for an infinity |
| `sqrt(x: real)`, `ln(x)`, `log10(x)` | `real`. Null for a negative number, or for `ln` and `log10` of zero or less |
| `exp(x: real)`, `sin(x)`, `cos(x)`, `atan2(y: real, x: real)` | `real`. Angles are in radians |
| `lerp(a: real, b: real, t: real)` | `real`: `a + (b - a) * t` |
| `div(a: int, b: int)` | `int`, rounded toward minus infinity. Null for a zero divisor |
| `rand(a: real, b: real)` | A random `real` from `a` up to but not including `b` |
| `rand_int(a: int, b: int)` | A random `int` from `a` to `b` inclusive; `a` if `b` is less |
| `chance(p: real)` | `bool`: true with probability `p` |
| `pick(list: list<T>)` | `T?`: one element, each equally likely; null, with no draw, for an empty list |
| `len(x)` | `int`: the elements of a list, the entries of a map or the characters of a text |
| `contains(xs, x)` | `bool`: whether a list has an element equal to `x`, or a text contains the text `x` |
| `first(xs: list<T>)`, `last(xs)` | `T?`: null for an empty list |
| `sum(xs: list<number>)` | The total; an `int` if all are; `0` for an empty list |
| `str(x)` | `string`: a number as written, an id as its qualified id, `null` for null |
| `keys(m: map<T>)`, `values(m)` | `list<string>` and `list<T>`, in order of key |
| `has(m: map<T>, key)` | `bool`: whether there is an entry for a text or an id |
| `without(xs: list<T>, x)` | `list<T>`: a copy without every element equal to `x` |
| `without_key(m: map<T>, key)` | `map<T>`: a copy without that entry |
| `cap(text: string)` | `string` with a capital first letter |
| `is_null(x)` | `bool` |
| `exists(x)` | `bool`: whether what a reference points at still exists. Answered by the simulation; not safe for belief |
| `name(x)`, `the(x)`, `a(x)`, `they(x)`, `them(x)`, `their(x)`, `themself(x)` | `string`, as section 4.6. Answered by the simulation for the reader |
| `verb(x, word: string)`, `count(n: number, word: string)`, `plural(word: string)`, `list(xs: list)` | `string`, as section 4.6 |

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
| `persona`, `track`, `memory` | As in 15 §7 and §8 |
| `case`, `mission`, `objective` | As in 13 |
| `action` | A running action (03 §6) |
| `part`, `wound` | A part of a body, and a wound on one (06 §6, §7.3) |
| `body` | Something in flight (02 §15): a projectile, debris, a thrown creature |
| `window` | An open window (03 §7) |
| `global` | The whole simulation |

As built (decision D-464), the simulation serves these kinds; the rest
are registered by the packages that build what they refer to. In Rust a
subject is a `Subject`; in an expression it is a handle, and
`Kinds::handle` and `Kinds::subject` convert between the two.

| Kind | The 64 bits of its handle |
|------|---------------------------|
| `entity` | The entity's id: slot and generation |
| `loc` | The location, packed from the top bit down: the grid's id in 10 bits; `x` and `y` in 21 bits each and `z` in 10, two's complement; two bits that are zero |
| `element` | As `loc`, with the layer in the lowest two bits: 1 floor, 2 block, 3 fixture |
| `area`, `site`, `realm` | The id |
| `tract` | The realm's id in 16 bits, then the tract's `x` and `y` in 24 bits each, two's complement |
| `global` | Zero |

A handle is a value and may outlive what it names. Reading through one
that names nothing any more gives null and never another thing: an
entity's handle carries its generation, ids of areas, sites and realms
are never reused, and a cell, a tract and the simulation always exist.
`exists(x)` says whether what a handle names is still there. A cell
beyond the first 1024 grids, more than 1,048,576 cells from the origin
along `x` or `y`, or more than 512 storeys from it, has no handle and
reads as null; so has a tract beyond 8,388,608 tracts from the origin or
of a realm past the first 65,536.

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

The struct above is the idea. As built, the registry lives in the
expression crate, which sits below the simulation and cannot name its
types, so it is split in two. The **registry** is plain data, filled in at
start-up and read back, in order of name, by the generated reference:

| Entry | States |
|-------|--------|
| Kind of subject | Name; documentation |
| Property | Kind; name; arguments; type (nullable if it can be missing); documentation; whether it is safe for belief; whether its value belongs to the same owner (section 7.3); whether it is settable; the events that change it |
| Function | Name (bare for the engine's, qualified for a mod's); parameters, which may be optional or, for the last, repeated; result type; documentation; the heading it is listed under; whether it is random; whether it is safe for belief; the events that change its result, if it may be used reactively |
| Event | Name; subjects in delivery order; fields with types; whether it is a hook and which fields a handler may change; documentation |
| Verb | Name; arguments with types and whether each is required; the argument that may be given directly; documentation |
| Variable | Qualified name; the kind it lives on; type; documentation; whether it is safe for belief |
| Definition type | Name; documentation; the qualified ids of that type with their dense numbers, set when mods are linked |

Each registration returns a dense id. The **getters, setters and function
bodies** stay in the simulation, keyed by those ids, and are reached while
an expression runs through one interface, the host: the value of a bound
subject, a local or an event field by slot; a property of a handle; a
variable of a handle by qualified name; a registered function; the random
stream; and where run-time warnings go. A name may be registered once;
`var` and the reserved words cannot be names.

How the simulation fills it (decision D-465):

- `clash_sim::engine_registry()` gives the registry of everything the
  simulation offers so far: the built-in functions, the kinds of section
  5.1, every property and function, and the kernel's and the world's
  events. Whoever loads mods gives it to the loader
  (`clash_scene::engine_loader` does). The simulation's registry step
  (04 §7.3) enters whatever of it a registry lacks and then the events
  mods define, so a loader that was given no registry, or a test's own,
  still ends with every entry.
- Each property is a `PropertyDef`: the kind, the name, the arguments
  and the type as the language writes types, the documentation, whether
  it is safe for belief, the events that change it, a getter and, if it
  can be set, a setter. Each function is a `FunctionDef`. A package adds
  its table to `engine_properties` or `engine_functions`. A getter is
  given the simulation to read, the subject and the arguments, and gives
  null for whatever it cannot answer. A setter calls the function the
  engine itself uses, so it emits what that function emits.
- A simulation joins the registry its definitions were compiled against
  to the tables by name (`Bindings`), so ids need not be the same from
  one load to the next.
- The host is `SimHost`. `Sim::host(bound)` reads only; inside
  `Sim::with_random(bound, …)` random functions draw from the rule
  stream. `bound` holds a value for each subject, local and event field
  of the scope the expression was compiled in, in the order the scope
  declared them. `Bindings::bind_event(event)` gives what an event
  binds. A field is evaluated through the data layer's `Runner` over the
  host; `Sim::eval(code, bound)` evaluates one compiled expression.
- `exists` is answered from the simulation. The reader's helpers of
  section 4.6 answer in plain forms (`the door`, `2 doors`) until the
  message package gives a reader and a `language`.

The core properties, on the kinds that have them:

| Property | On | Gives |
|----------|----|-------|
| `kind` | Every kind | `string`: the kind's name |
| `tags` | `entity`, `loc`, `element`, `area`, `site` | `list<string>?`: its tags in order; for a cell, those of the definitions of its three elements; null for what does not exist or is not loaded |
| `has_tag(tag: string)` | The same | `bool` |
| `def` | `entity`, `element`, `area`, `realm` | The definition it was made from: `any` for an entity or an element, whose type varies; `id<area_kind>?`; `id<realm>?` |
| `loc` | `entity`, `element`, `area`, `site`, `tract` | `loc?`: where it is; for an area its middle, for a site its anchor, for a tract its corner at storey 0 |
| `var` | Every kind but `element` | Section 6 |

The rest are listed where their subsystem is specified: an entity's in
03 §13 and 06 §13, the world's in 02 §23.

### 5.4 Truth and belief

Properties read ground truth. That is right for rules of the world: physics,
abilities resolving, scenario logic.

Rules that decide what a **character does** must not read truth the
character could not know (01 R11). Document 15 provides the belief-side
vocabulary: what `self` has perceived, believes and remembers. A rule
attached to a creature that reads `target.loc` directly, rather than where
`self` believes `target` to be, is flagged by the linter.

As registered (decision D-466), nothing is safe for belief unless it is
marked, and what is marked is arithmetic on a reference: a subject's
`kind`; a cell's `x`, `y`, `z`, `grid`, `tract` and `realm`; a tract's
`x`, `y`, `realm` and `loc`; an element's `layer` and `loc`; `global.now`;
and the functions `dist`, `dir`, `offset`, `now`, `setting` and
`has_def`. Everything about an entity is truth, which a mind context
reads of its own subject and of no other. Everything that reads a cell's
contents is truth as well, the shape that 02 §23 allows included, and so
are `los` and `light`: the tag `hidden` cannot be honoured until
perception is built, and a read that ignored it would leak. They become
safe when document 15 supplies the belief side.

Document 15 now supplies it (decisions D-616, D-617): a mind reads what
it believes through properties of its own subject (`self.believed_loc(x)`,
`self.perceives(x)`, `self.could_perceive(at, kind)`, `self.remembers(at)`,
`self.knows_element(at, layer)`; 15 §7.2), and the marks above stand.
15 §7.3 says which may later be relaxed, and how.

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

A declaration's fields:

| Field | Meaning |
|-------|---------|
| `scope` | What the variable lives on: a kind of subject, by the names of the table above. `cell` is the kind `loc`. Required. |
| `value` | The type, as section 3 writes types: `bool`, `int`, `real`, `string`, `id<material>`, `list<int>`, `map<real>`, a kind of subject. Required. |
| `default` | What the variable reads as until it is set: a constant of the declared type. Left out, the variable reads as null until it is set, and its type is the declared one or null. |
| `watch` | As above. Default false. |
| `belief_safe` | Whether a character may read it of things other than itself in a mind context (section 7.3). Default false. |
| `doc` | Shown in the inspector and the reference. |

A variable is declared at the top level of a data file, not in place
inside another definition. `strict_vars` belongs to the mod that wrote
the expression: a strict mod's rules are held to it wherever they end up,
and another mod's are not.

As stored (decision D-467):

| Scope | Kept in | Written through |
|-------|---------|-----------------|
| `entity` | The kernel entity | `Sim::set_var` |
| `cell` | The cell's sparse variables in its chunk | The world's `WorldEdit`, which emits `cell_var_changed` |
| `area`, `site` | The area's or the site's record | `Cosmos::set_area_var`, `set_site_var` |
| `tract` | The tract's record, loaded or not | `Cosmos::set_tract_var` |
| `realm` | The realm's record | `Cosmos::set_realm_var` |
| `global` | The cosmos | `Cosmos::set_global_var` |

- `Sim::var(subject, name)` reads: what was set; for a variable that was
  never set, its declared default if it is declared for that kind of
  subject; otherwise null. Nothing is read of a subject that no longer
  exists, a default included.
- `Sim::set_var(subject, name, value, cause)` is the one way a variable
  changes, and returns what it read as before. Setting null unsets. A
  declared variable is held to its declaration: it is refused on a
  subject of another kind (`vars.wrong_scope`) and with a value that is
  not of its type (`vars.wrong_type`), where an int serves as a real. An
  undeclared variable holds anything on anything. A value nested deeper
  than 32 is refused (`vars.too_deep`), as is a subject that no longer
  exists (`subject.gone`) and whatever the world refuses: a cell that is
  not loaded, a tract whose region has not been surveyed. An element has
  no variables of its own.
- A watched variable raises `var_changed` when what it reads as changes,
  with what it read as before and what it reads as now: for one that was
  not set, its default. What it lives on is bound under the name of its
  kind: an entity as `self`, a cell as `here`, an area, a site, a tract
  or a realm under those names; a global variable binds nothing. The
  fields are `name` (string), `scope` (string, the kind), `old` and `new`
  (any).

Not built: the scopes `region` and `subgrid` (their kinds are not
served yet; the world keeps a region's variables and
`Cosmos::set_region_var` writes them), `action`, `scenario` and
`legacy`, which come with their packages.

## 7. Where expressions appear

### 7.1 Expression fields

A field of type `Expr<T>` accepts:

| Written | Meaning |
|---------|---------|
| `12`, `true`, `"250 ms"` | A literal of type `T` |
| `"self.var.kills * 2"` | An expression, when `T` is not a string or id |
| `{ "expr": "…" }` | An expression, always. Needed when `T` is a string or an id. |
| A value source (section 7.4) | A computed value |

In detail:

- A JSON number is an `int` if it is written whole and a `real`
  otherwise; where an `int` is wanted, a number with a zero fraction or
  an exponent (`2.0e1`) is one. A JSON list of numbers, booleans, texts
  and lists is a list literal. `null` is a literal only where the field
  may be null.
- Where `T` is a string, a JSON string is that text; where `T` is an id,
  it is an id, resolved when mods load like any reference, and one that
  names nothing is an error with the closest id.
- Where `T` is a number, a JSON string that is a quantity of 04 §7.1
  (`"250 ms"`, `"5 m/s"`, `"-5 C"`) is that quantity in SI units, so
  compound units and offsets may be written in such a field although the
  expression language has neither. Anything else is an expression.
- A literal of the wrong type is a load error, as an expression of the
  wrong type is.

### 7.2 Conditions

A condition is an expression of type `bool`, or a structure:

```json
{ "all": [ "self.var.rage > 50", { "any": [ "here.in_area('arena')", "target.has_tag('rival')" ] } ] }
```

`all`, `any` and `not` nest freely and compile to the same thing as the
string form.

A condition may also be `true` or `false`, or `{ "expr": "…" }`. An
object has exactly one of `all`, `any`, `not`, `exists` and `expr`. Each
string of a structure is compiled by itself, so a mistake in one is
reported at that string, and the structure is evaluated left to right,
stopping as soon as the answer is known. A part that comes to null
counts as false, as it does in the string form.

### 7.3 Contexts and purity

Each place an expression appears declares its **context**: which subjects
and locals exist, and how pure it must be.

| Purity | Allowed | Used by |
|--------|---------|---------|
| `pure` | Reads only | Screen bindings (12), previews of `requires`, sorting keys |
| `random` | Reads and random draws | Conditions and effect arguments |

Expressions never write. Only effects do.

A context also says whether it reads **truth** or **belief**.

| Reads | Allowed | Used by |
|-------|---------|---------|
| Truth | Anything | World rules, effects, hooks, debug screens |
| Belief (the **mind context**) | The character's own state; what it has perceived (15 §7); static geometry (15 §7.3); and functions, properties and queries registered as safe for belief | Goals, methods, tactics and option scoring (09); an action's `requires`, defaults and `check` when they are previewed or gathered as a character's options; ordinary screens (12 §10.4); and every function or rule those reach |

Every entry in the registry (section 5.3) says whether it is safe for
belief. `dist` and `los` are, between places the character knows. Reading
another entity's state is not; neither are queries over entities, contents
or other minds. In a mind context an unsafe read is a load error. An
action's `requires` is tested against truth only when the action is
actually started; a failure then is a refusal, with its reason.

How the compiler knows "the character's own state": a mind context binds
the character as its **own** subject, usually `self`. Any property or
variable of an own subject may be read, safe or not. The value read is
itself own only if the property is registered as yielding something that
belongs to the same owner, as a part of a body does: `self.part('arm').size`
is the character's own, `self.loc.material` is the world's. Everything
else must be registered safe. Naming a bound subject, a local or an event
field is always allowed; it is reading from them that is checked.

A context may bind more than one thing as own, and a local as well as a
subject: an ordinary screen binds the party, a list, and a parameter
that is one of its members (12 §10.4). An element of an own list is
own. The compiler can be asked whether a whole expression's value is
own, so that whoever binds that value by another name elsewhere, as a
screen binds each row of a list to `it`, may bind it as own there.

Who declares the context: the definition type that owns the field, in
Rust, once for each field (04 §7.3). It lists the subjects with their
kinds, the locals with their types, the event, whether the field is pure
and whether it is a mind context, naming the character with `mind` (the
own subject, in a context that reads belief) or with `own` alone where
the same subject is bound in a context that reads truth. A mod cannot
widen what a field offers. Mistakes against the context are load errors
at the name: an unbound subject, a local or an event field that is not
there, a random function in a pure field, an unsafe read in a mind
context. The context also carries what the loader knows about the string
itself: the namespace of the mod that wrote it and that mod's imports,
for bare names, and whether that mod asked for `strict_vars`.

A condition may also be `{ "exists": query }`, true when the query
(section 11) has any result.

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
| `{ "run": { "eoc": id, "with": { … } } }` | What a rule returns (section 9.5). Allowed only where effects are; never in a pure context. |

Queries are defined in section 11.

A value source is written where an `Expr` is and is one object with
exactly one of the keys above, or `expr`. `weight` belongs with `pick`
and `of` with `sum`, `min` and `max`. The `args` of a `call` are given by
parameter name, each an `Expr` of the parameter's type compiled in the
scope of the field the source is in; a parameter with a default may be
left out. A `call` names a function definition, or a function the
simulation registered: the engine's built-in functions are called in an
expression. A source is held to its field's context like an expression:
`pick`, `run` and a `call` of a random function are refused where the
value must be pure, and a `call` of a function that is not safe for
belief is refused in a mind context.

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
re-evaluated. Reactive expressions may not call random functions or
`now()`. They may use only properties, variables and functions that are
registered with what makes them stale: `dist` and `los` by the relocation
of either end, a variable by any write to it. Anything else in a reactive
field is a load error.

Re-evaluation is the last step of the pump (03 §4.3). If telling an owner
makes others stale they are evaluated in turn, at most
`rules.max_reactive_rounds` times (default 8). What is still stale then
is reported, with its chain, and left for the next pump.

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
other. An event with `"hook": true` is delivered at once and may list
which fields handlers may change: `"mutable": ["power"]`. Used as a value
source, `emit` gives back the fields as the handlers left them and a
`cancelled` flag, so the mod that raised it can act on the answer:

```json
{ "let": { "r": { "emit": { "event": "ritual_completing", "caster": "self", "power": "$p" } } } },
{ "if": "not $r.cancelled", "then": [ … ] }
```

As built (decision D-368):

- An `event` definition has `subjects` (name to kind, in the order they
  are written, which is delivery order), `fields` (name to `bool`, `int`,
  `real`, `string` or `any`), `hook`, `mutable` and `doc`. A subject of an
  unknown kind, a field of an unknown type, a name used for both, a
  mutable field that is not declared, and mutable fields on something
  that is not a hook are load errors. The event's name is its qualified
  id: `mymod:ritual_completed`.
- What is raised is checked against the type: every declared subject must
  be given, of its kind; nothing undeclared may be; a field must be of its
  type, where an integer serves as a real. A declared field left out is
  null. The engine's own events cannot be raised by content.
- A field's value is null, a plain value (a truth value, a number, a
  text) or a reference to a subject. Lists, maps and ids as field values
  wait for the rule runtime.
- Every event also binds `actor` and `instigator` from its cause where the
  cause names them, after its own subjects.

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

As built (decision D-368). Whoever performs an operation raises its hook
and is given back the fields as the handlers left them and, if one
cancelled, the reason. A cancellation stops delivery: no later handler
runs. It is reported as the notification `event_cancelled` (03 §13).
Changing a field the hook does not declare mutable, or cancelling a
notification, is refused to the handler with a reason and reported as a
fault. A hook raised from inside eight others is skipped, with a fault
(section 13.2).

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

As built (decision D-368). Handlers are registered through two calls,
which the rule runtime will use for rules: one for a global handler of an
event type, one for a handler attached to a subject. Each states a
priority, an order (the rule's id), a filter, whether it is skipped once
the event is consumed and, for an attached one, `as`. Among handlers of
equal priority and order, the one registered first runs first. Who is
called for an event is settled before anyone is: a handler that registers
or removes another does not change the delivery under way. Handlers are
not state and are registered again after a load.

Rules (decision D-470) are found by one function, which the simulation
calls from one handler for each event type that some rule listens for.
It runs the global rules of the event's type, then for each subject of
the event, in the order the event declares them, the rules that subject
carries through its definitions and those attached to it at run time
whose `as` is that subject's place; within each group by `priority` and
then by rule id, which is the order of the rules' qualified ids. Which
rules run is settled before any does, so a rule attached or detached by
one of them is not run or dropped for the event under way. An event
type that no rule listens for costs nothing.

### 8.4 Filters

`"filter"` on a rule is a cheap test on an event's discrete fields, applied
before the condition and used to index handlers:

```json
{ "on": "element_destroyed", "filter": { "layer": "block", "material": "glass" } }
```

A rule should filter on what it can, and leave the rest to `if`.

As built: handlers are indexed by the first entry of their filter. An
event looks at the handlers with no filter and at those whose first entry
names a field the event has with that value; the rest of each filter is
then tested. A thousand handlers filtered on other values cost an event
nothing.

A rule's filter names fields of its event, each with a constant of the
field's type: a text, a number, a truth value, or the id of a definition
where the field holds one, resolved at load. A field the event does not
have, and a value that is not a constant, are load errors. Numbers are
compared by value, so `12` matches a field that holds `12.0`. Global
rules are indexed by the first entry as handlers are.

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
| `attached` | Whether the rule runs only for what it is attached to (section 9.3), which it binds as `self`. Default false for a rule at the top level of a file and true for one written in place inside another definition. |
| `as` | For attached rules: which of the event's subjects the owner must be. Defaults to the event's first subject. |
| `filter` | Section 8.4: fields of the event, each with the constant it must equal. |
| `every` | Run repeatedly at this interval instead of on an event (section 9.4). Not beside `on`. |
| `while_unloaded` | For a recurring rule: `skip`, `once` or `all` (section 9.4). Default `skip`. |
| `if` | Condition. Default true. |
| `do` | Effects when the condition holds. |
| `else` | Effects when it does not. |
| `params` | Typed parameters, for rules that are called (section 9.5). |
| `subjects` | The subjects a rule that is called is given by its caller: names with their kinds (section 9.5). |
| `returns` | The type of what `return` gives. Default `any`. |
| `priority` | Order among handlers: higher first. Default 0. |
| `once` | Run at most once per owner. |
| `cooldown` | Minimum local time between runs per owner. |
| `skip_if_consumed` | Section 8.3. |
| `ui_callable` | The rule may be run from a screen through the `Ui` command (12 §10.5). Default false. |
| `group` | A rule group this rule belongs to (section 9.6) |
| `doc` | Shown in the reference and the trace. |

As built (decisions D-472, D-474):

- **What a rule may name.** The subjects of its event, in the event's
  order; then `self`, for an attached rule; then those it declares in
  `subjects`; then `global`. Its locals are its parameters, in the order
  written, and after them whatever its effects define. `event.field`
  reads a field of its event. A rule reads truth and may draw at random.
- **`once` and `cooldown`** count the runs in which the condition held:
  a run that took `else` does not use up a `once` or start a cooldown.
  They are kept for each rule and owner. A global rule, and a rule that
  is called, has no owner and one record; its cooldown is measured in
  universal time.
- A rule with neither `on` nor `every` runs only when it is called, and
  cannot be `attached`. `as` is refused on a rule that is not attached,
  and `filter` on a rule with no `on`.
- A rule that does not load is replaced, in a lenient load, by one that
  never runs.

### 9.2 Global rules

A rule defined at the top level of a data file with an `on` is global: it
runs for every event of that type, anywhere. One that says
`"attached": true` is not: it is written at the top level so that it has
a name, and runs only for what it is attached to.

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

Rules are also attached at run time with the `attach_rule` and
`detach_rule` effects.
A curse is a rule attached to its victim.

As built. Only an attached rule can be attached, and only to a subject
of the kind its owner is: the kind of the event's subject that `as`
names. What is attached at run time is state, kept by owner, and so is
when each rule last ran for each owner; `detach_rule` forgets both, so a
rule attached again starts afresh, and both are forgotten when the owner
is destroyed. An event looks, for each of its subjects in order, at the
rules that subject's definitions carry and at those attached to it at
run time, and runs the ones whose `as` is that subject's place in the
event. The rules attached to a subject that is stored do not run
(section 13.5).

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

A called rule may `return` a value. In an effect list the caller takes it
with a value source: `{ "let": { "won": { "run": { "eoc": "arm_wrestle",
"with": { "foe": "target" } } } } }`. Rules are still not callable from
expressions. Only functions are, because expressions must not change
anything.

As built (decisions D-472, D-473):

- **Parameters.** `params` is written as a function's are (section 12): a
  name and a type, or a name and an object with `type`, `default` and
  `doc`. A default is a constant; parameters with one may be left out of
  a call and come last. A call that gives a parameter the rule does not
  have, leaves out one with no default, or gives a value of the wrong
  type is a load error where the rule is named in the call, and is
  refused when it runs where the rule's id is worked out.
- **Subjects.** A called rule is given a subject for each one it binds,
  from the subject its caller binds under the same name where the call
  is written; one the caller does not bind is null. A rule says which
  subjects it expects in `subjects`, by name and kind
  (`"subjects": { "self": "entity" }`), and calling it where no subject
  of that name is bound is a load error. A rule that listens for an
  event may be called too; it is then given its event's subjects by
  name and its event's fields read null.
- **Locals.** A called rule has locals of its own: its parameters and
  what its effects define. It cannot name its caller's, and what it
  assigns to its own does not reach its caller.
- **As a verb.** A rule with at least one parameter is entered in the
  registry as a verb under its qualified id, with its parameters as the
  verb's arguments, so it is listed in the reference with them. Where an
  effect's key is the name of one of the engine's verbs, the verb is
  meant; otherwise the key is looked up as a rule, in the mod's own
  namespace and then its imports. A rule with no parameters is run with
  `{ "run": "its_id" }`. A rule written in place has no name to be
  called by.
- **Returning.** `returns` is the type `return` must give, `any` if left
  out. What a caller takes is typed `any`. `return` in a list of effects
  that is not a rule's is a load error. `{ "run": … }` as a value is
  written as the value of a `let`; anywhere else it is a load error.
- **Once, cooldown and groups** govern a rule however it is run: a call
  of a rule that may not run now does nothing and gives null.

### 9.6 Groups

```json
{ "type": "rule_group", "id": "harsh_winter", "enabled": "setting('harsh_winter')" }
```

A rule may name a `group`. A group is on or off. While it is off, its rules
do not run and cost nothing. A group's `enabled` is a reactive condition
(section 7.5), so a setting can govern it. `set_rule_group` switches one
from a rule. Optional rule sets, difficulty options and a scenario's
special rules are groups.

| Field | Meaning |
|-------|---------|
| `enabled` | Whether the group is on, unless `set_rule_group` has switched it: a pure condition that may read `global`. Default true. |
| `doc` | Shown in the reference. |

As built (decision D-474). A group is on if `set_rule_group` switched it
on, off if it switched it off, and otherwise as `enabled` says.
`{ "set_rule_group": { "group": id, "on": null } }` leaves it to
`enabled` again. The switch is state and is saved. A group governs its
rules however they are run, called ones included. Until reactive
expressions are built (section 7.5), `enabled` is evaluated when a rule
of the group is about to run and not kept current: a group that is off
costs one condition for each of its rules an event reaches.

## 10. Effects

### 10.1 Form

An effect is an object with one key, the verb, whose value is the
arguments:

```json
{ "damage": { "at": "here", "layer": "block", "kind": "blast", "energy": 5000, "intensity": "20 kPa" } }
```

Verbs with one obvious argument accept it directly: `{ "message": "…" }`,
`{ "run": "some_rule" }`. An unknown verb or argument is a load error with
suggestions.

Effects in a list run in order. Arguments are `Expr` fields, evaluated when
the effect runs.

As built (decision D-471):

- An effect is one object. The effects of section 10.2 that take more
  than one key are told by the key that names them (`if`, `switch`,
  `for_each`, `repeat`, `while`, `after`, `every`, and `as` beside
  `do`); every other effect has exactly one key. A key that is not one
  of the effects of sections 10.2 to 10.5 is a verb in the registry, or
  else a rule with parameters (section 9.5).
- A verb takes its arguments by name, in an object. One whose
  registration names a direct argument also takes that argument alone:
  any value that is not an object, or an object none of whose keys is
  one of the verb's arguments, which is then a value source. Each
  argument is an `Expr` of the type the verb declares, so where a text
  is wanted a JSON string is the text itself (section 7.1). An argument
  that may be left out and is, reaches the verb as null.
- A verb that declines gives its reason, which is reported, and the
  rule carries on with its next effect.
- A list of effects is compiled in the scope its definition gives it:
  the subjects and locals there, in slot order. What `let`, `for_each`
  and `repeat` define takes the slots after those and goes out of scope
  where the list it is written in ends. Lists nest at most 32 deep; one
  more is a load error.
- Mistakes are load errors of group `068` (04 §8.2), each at the key or
  value that is wrong, with the closest name where a name is unknown:
  an unknown effect, an effect that is not well formed (an unknown or a
  missing argument, a body left out), an effect where it has no meaning
  (`return` outside a rule, `modify` outside a hook), and a rule that is
  not well formed. A value of the wrong type and a place that cannot be
  assigned to are the expression compiler's errors.

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
| `{ "return": expr }` | End this rule and give a value to whoever called it |

The arguments in detail (decision D-477):

| Effect | Arguments |
|--------|-----------|
| `if` | `if`: a condition (section 7.2). `then`, `else`: lists of effects; either may be left out, not both. |
| `first` | A list of entries, each with `do` and, unless it always holds, `if`. |
| `switch` | `switch`: an expression. `cases`: an object whose keys are compared with the value as a text: a text as it is, a number as it is written, `true` or `false`, an id as its qualified id. Where the value is known to be an id, each key is resolved at load and a misspelling is an error. `default`, which may be left out, runs when no key matches; null matches none. |
| `for_each` | `for_each`: an expression that gives a list, `{ "list": expr }`, or a query (section 11), which is kept as written until queries are built and reports `rule.queries_unavailable` when it runs. `as`: the name of the local each element is bound to. `do`. Null is gone through as an empty list. |
| `repeat` | `repeat`: an int. `do`, which sees `$i`, an int from zero. A count of zero or less runs nothing. |
| `while` | `while`: a condition, tested before each pass. `do`. |
| `weighted` | A list of entries, each with `weight`, a number, and `do`. One draw is made from the rule stream; an entry is chosen with the chance its weight is of the total. Weights of zero or less are never chosen, and if all are, nothing runs and nothing is drawn. |
| `as` | `as`: an object of names and the expressions each is bound to, all worked out before any is bound. `do`, in which those names are subjects; a rule called from inside is given them. |
| `stop` | `{}`. Ends the rule, or the list of effects of another definition, from however deep inside it. |
| `return` | An expression of the rule's `returns`, or `null`. A value of another type, where that could not be told at load, is reported and null is returned. |

Each effect executed and each pass of a loop is one step of the run's
budget (section 13.2).

### 10.3 Variables

| Effect | Does |
|--------|------|
| `{ "let": { "x": expr, "y": source } }` | Define locals |
| `{ "set": { "self.var.kills": expr, "$x": expr } }` | Assign variables, locals and settable properties |
| `{ "add": { "self.var.kills": 1 } }` | Add to numbers; append to lists |
| `{ "unset": [ "self.var.marked" ] }` | Remove variables |

What is assigned to is a **place**: a local (`$x`), a field of the event
(`event.energy`), a variable (`self.var.kills`, `self.var('mod:name')`) or
a settable property (`self.health`), optionally followed by keys
(`self.var.tally['ash']`). Anything else is a load error. A place is
written and checked like an expression, and the subject it lives on and
its keys are evaluated when the effect runs.

As built (decision D-477):

- **`let`** defines its locals in the order written, each for the rest
  of the list it is in; a later entry may read an earlier one, and a
  name defined again shadows the first. A local's type is that of its
  value, and a local that starts as `null` may be given anything. The
  value is an expression, a value source of section 7.4, or one of two
  that only a `let` takes: `{ "run": … }` for what a rule returns and
  `{ "emit": … }` for what came of an event (section 10.5).
- **`set`** takes a value of the place's type, or null. A field of an
  event is a place only in a rule that listens for a hook, and only if
  the hook declares it mutable; otherwise it is a load error.
- **`add`** adds a number to a number, an int if both are, and appends
  one element to a list. Something never set counts as nothing: adding
  a number to null gives that number. Anything else is a load error
  where the place's type is known and reported when it runs where it is
  not.
- **`unset`** takes a list of places written as strings. A variable is
  removed and reads as its default again; an entry of a map or a list is
  taken out; a local is made null. A property or a field of an event is
  not unset.
- **Keys.** A map is stepped into by a text or an id, a list by an
  index. Assigning to an entry a map does not have adds it, and a null
  where a map is needed becomes one, so a tally can be kept in a
  variable that was never set. A list takes an index it has, or its
  length, which appends; any other index is reported and nothing is
  written.
- **Limits.** What `let`, `set` and `add` write is held to
  `rules.max_value_size` and `rules.max_value_depth` (section 13.2); a
  value over either is not written.
- A place whose subject turns out null when the effect runs is reported
  and nothing is written. A write the simulation refuses is reported
  with its reason.

The write itself is the simulation's (decision D-468): what follows is
how it is carried out once the rule runtime is bound to it.

The write is the simulation's (decision D-468). `Sim::assign(place, how,
value, bound, cause)` evaluates the place's subject and keys against
what is bound and assigns; `Sim::assign_to` assigns where the caller has
evaluated them. `how` is `Set` or `Add`.

- A variable is written through `Sim::set_var` (section 6) and a
  property through the setter registered for it
  (`Sim::set_property`), which is the engine's own function: setting an
  entity's `loc` relocates it and emits `entity_relocated`.
- With keys, the value is read, changed at the end of the path and
  written back whole. A key into nothing makes a map; a key of a map is
  a text or an id; a key of a list is the position of an element that is
  there. An entry set to null is removed.
- `add` adds to a number, an int staying an int unless a real is added,
  and appends to a list. A variable that was never set is added to from
  its default.
- A local and a field of the event are kept by whoever runs the rule
  and are refused here (`write.not_a_subject`). The other reasons are
  `subject.gone`, `write.not_settable`, `write.bad_value`,
  `write.bad_key` and `write.cannot_add` (03 §13).

### 10.4 Time

| Effect | Does |
|--------|------|
| `{ "after": "2 s", "do": […] }` | Run later, on the owner's clock |
| `{ "every": "1 s", "times": 5, "do": […] }` | Run repeatedly |
| `{ "cancel_timer": "name" }` | Cancel a timer started with `"name"` |

`after` and `every` take optional `name`, `owner` and `while_unloaded`
(03 §9). The effects inside see the subjects and locals as they were when
the timer was set.

| Option | Values | Default |
|--------|--------|---------|
| `owner` | Any subject with a clock | The rule's `self`; for a global rule, the realm of `here`, or the simulation |
| `while_unloaded` | `pause` (stop the clock), `elapse` (time passes; fire once if overdue), and for `every`, `all` (fire for each interval missed) | `pause` |
| `name` | A name unique to the owner. Setting a timer with a name already in use replaces it. | None |

`owner.timer(name)` gives the time a named timer has left, or `null`. An
interval of zero is a load error.

As built. The three effects are read, checked and kept, the effects
inside them compiled in the scope they stand in, so that a mistake is
found at load. Timers themselves are a later package's: until then each
reports `rule.timers_unavailable` when it runs and schedules nothing.

### 10.5 Rules and events

| Effect | Does |
|--------|------|
| `{ "run": { "eoc": id, "with": { … } } }` | Call a rule |
| `{ "attach_rule": { "to": subject, "eoc": id } }`, `detach_rule` | Add or remove an attached rule. Attaching one that is already there does nothing. |
| `{ "emit": { "event": id, … } }` | Raise a custom event |
| `{ "modify": { "event.energy": expr } }` | In a hook: change a mutable field |
| `{ "cancel_event": { "reason": id, "values": { … } } }` | In a hook: prevent the operation. The reason (14 §5) is what whoever asked is told. It may be left out only where nobody is told. |
| `{ "consume": {} }` | Mark the event handled |
| `{ "queue": { "eoc": id, "with": { … } } }` | Call a rule once the rule that is running, and whatever called it, has finished |
| `{ "set_rule_group": { "group": id, "on": bool } }` | Switch a rule group (section 9.6) |

The arguments in detail (decisions D-473, D-476, D-477):

| Effect | Arguments |
|--------|-----------|
| `run` | `eoc`: the id of a rule, or `{ "expr": … }` for one worked out when the effect runs. `with`: the rule's parameters by name. `{ "run": "id" }` runs a rule that needs none. As the value of a `let` it gives what the rule returned. |
| `queue` | As `run`. The rule, its arguments and the subjects it is given are worked out when the effect runs; the rule itself runs when the outermost rule in progress has finished, before anything else happens. Queued rules run in the order they were queued, each with a step budget of its own, and one queued by a queued rule runs after it. At most `rules.max_queued` may wait at once. |
| `attach_rule`, `detach_rule` | `to`: a subject. `eoc`: the id of an attached rule (section 9.3). |
| `set_rule_group` | `group`: the id of a `rule_group`. `on`: true, false, or null to leave the group to its `enabled`. |
| `emit` | `event`: the id of an event; every other key is one of the event's subjects or fields by name. Every subject is given; a field left out is null. A notification is delivered at the next pump and a hook at once. As the value of a `let` it gives a map: the event's fields by name, as the handlers left them; `cancelled`, a boolean; and `reason`, the code of the reason it was cancelled for, or null. An event the simulation will not raise is reported and gives null. |
| `modify` | Places written `event.field`, each with a value. Only in a rule that listens for a hook, and only fields the hook declares mutable. `set` on such a field is the same. |
| `cancel_event` | `reason`: the id of a `reason` definition (14 §15). `values`: what the reason carries, by name. Both may be left out; with no reason the engine's `events.cancelled` stands in. Only in a rule that listens for a hook. No later handler runs. |
| `consume` | `{}`. Only in a rule that listens for an event. |

### 10.6 The player

| Effect | Does |
|--------|------|
| `{ "message": { "text": …, "to": subject, "style": … } }` | Put a line in the message log of whoever controls `to`; default `self` |
| `{ "ask": { "who": entity, "prompt": text, "options": [ { "label": …, "if": …, "do": […] } ] } }` | Stop and offer a choice (03 §5.2). Each option carries the effects to run if chosen. For entities not under direct control, the first available option is taken unless a rule says otherwise. |

`options` may instead be an expression giving a list of maps, each with a
`label`, the `eoc` to run if it is chosen and the values to run it `with`.
That is how options gathered at run time are offered (15 §11). A `message`
whose `to` is not bound in the rule's context is a load error.

Opening screens and other interface effects belong to document 12. A rule
asks for a screen with `show_screen` (12 §10.5).

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

The fields in detail:

| Field | Meaning |
|-------|---------|
| `params` | The parameters, in the order written, which is the order arguments are given in. Each is a name and its type (`"of": "entity"`), or a name and an object with `type`, and optionally `default` and `doc`. A parameter with a default may be left out of a call; those come last, and a default is a constant. Default: none. |
| `returns` | The type of the result. Left out, the result is `any`, which is checked when the function runs. |
| `value` | The body. It sees the parameters, as `$name`, and nothing else: no subject of whoever calls it. |
| `doc` | Shown in the reference. |

What the loader works out, from the body and from every function it
calls, directly or through others:

- **Random.** A function is random if it calls a random function, or its
  value is a `pick` or a `run` source.
- **Safe for belief.** A function is safe for belief if everything it
  reads is: its body would compile in a mind context that binds only its
  parameters, and so would the body of every function it calls. A
  function that reads a property of a parameter which is not registered
  safe is therefore not safe, even when it is called with the character
  itself. Until queries are built, a function whose value is a query is
  not safe.
- **Recursion.** A function that calls itself on every path through its
  body, directly or through others, could never finish, and is a load
  error that names the chain. A call counts as made on every path unless
  it is inside a branch of `if`, or follows `and`, `or` or `??`. Any
  other recursion is allowed to the depth `rules.max_call_depth`
  (section 13.2); the call that would go deeper is refused when it runs,
  gives null and is reported.

A function is registered under its qualified id, so a function defined in
place inside another definition cannot be called by name. A function may
not take the qualified name of one the engine registered.

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

As built (decisions D-470, D-475). The struct above is the idea. A run
is a stack of frames, one for the rule or list that was started and one
for each rule it has called, each with its subjects by slot, its locals
and the rule's owner; and a stack of the lists and loops in progress.
The interpreter does not recurse on what a mod wrote, so neither deep
nesting nor rules that call rules can exhaust the machine's stack. Only
a hook raised by `emit` re-enters, through the simulation.

For an event, the rules that answer it are found first (section 8.3)
and then run one after another, each a run of its own: the `filter`
against the event as it then is; whether the rule may run (its group,
`once`, its `cooldown`); `if`; then `do` or `else`. Nothing runs after a
hook is cancelled.

The rule runtime is written against an interface, the **rule host**,
which the simulation implements: reading through the expression host
(section 5.3); setting and removing a variable and setting a property;
carrying out a verb, which is given one value for each argument it
declared and what is running, and answers with a value or a reason;
raising an event, which answers with the fields as handlers left them
and the reason if one cancelled; the local time of an owner's clock;
the rules a subject's definitions carry; whether a subject is being
simulated; and where reports go. What is kept between runs is plain
state: the rules attached at run time, when each rule with `once` or a
`cooldown` last ran for each owner, and the groups a rule has switched.

### 13.2 Limits

| Limit | Default | On exceeding |
|-------|---------|--------------|
| Steps per run (effects executed plus loop iterations) | 100,000 | Run ends; error reported |
| Call depth | 32 | Call refused; error reported |
| Hook nesting depth | 8 | Inner hook skipped; error reported |
| Events per pump | 10,000 | 03 §4.3 |
| Wake-ups handled at one tick | 100,000 | The rest are put off to the next tick; error reported. As built, `advance` reports the error and yields instead (03 §4.3) |
| Size of one value (a list, a map or a text) | 65,536 elements, or bytes of text | In an effect: the effect fails; error reported. In an expression: the operation gives null, with a warning (section 4.5) |

| Rules waiting in the queue (`queue`, section 10.5) | 1,000 | The rule is not queued; error reported |
| Nesting of lists and maps in one value an effect writes | 32 | The effect fails; error reported |
| Nesting of lists of effects | 32 | Load error |

Exceeding a limit is always an error report with a trace, and never a hang
or a crash.

As built (decision D-475):

- The step budget is one for the whole run: a rule that is called
  spends its caller's steps, and at the limit the run ends, callers
  included. What it had done stays done. Each rule that answers an
  event, and each queued rule, is a run with a budget of its own.
- A call that would nest deeper than the call depth is refused and
  gives null; the caller carries on with its next effect. The same
  setting bounds functions that call each other (section 12).
- A value is measured when an effect writes it: by `let`, `set` and
  `add`, and when a `let` takes what a rule returned. Nesting is
  limited as well as size because a value that wraps itself in a loop
  (`"$x": "[$x]"`) would otherwise grow deep enough to exhaust the
  stack of whatever later compares, hashes or drops it.
- The limits are the settings `rules.max_steps`, `rules.max_call_depth`,
  `rules.max_hook_depth`, `rules.max_cascade`, `rules.max_queued`,
  `rules.max_value_size` and `rules.max_value_depth`.
- A report is a **fault**: a reason (14 §5) with the rule that was
  running, the definition and JSON path of the effect, and the reason
  the simulation gave if it refused something. Faults are thinned by
  count as run-time warnings are (section 4.5): a site, which is one
  effect and one kind of fault, reports its first three and then the
  occurrences whose count is a power of two.

| Reason | Reported when |
|--------|---------------|
| `rule.step_limit` | A run executed more steps than `rules.max_steps`. The run ended. |
| `rule.call_depth` | Rules called each other deeper than `rules.max_call_depth`. The call was refused. |
| `rule.queue_limit` | More rules were queued than `rules.max_queued`. The rule was not queued. |
| `rule.value_too_large` | An effect would have written a value larger than `rules.max_value_size`. Nothing was written. |
| `rule.value_too_deep` | An effect would have written a value nested deeper than `rules.max_value_depth`. Nothing was written. |
| `rule.no_subject` | An effect needed a subject and was given null or something else. |
| `rule.bad_key` | A key or an index of a place does not fit what is there. |
| `rule.type_mismatch` | A value had a type the effect does not accept, where that could not be told at load. |
| `rule.write_refused` | The simulation refused to set or remove a variable or a property. |
| `rule.verb_refused` | A verb declined. |
| `rule.emit_refused` | The simulation refused to raise an event. |
| `rule.unknown_rule` | A computed rule id names no rule. |
| `rule.bad_argument` | A rule whose id was computed was called with arguments that do not fit its parameters, or was given a subject of the wrong kind. |
| `rule.not_attachable` | `attach_rule` was given a rule that is not an attached rule, or a subject of another kind than the rule attaches to. |
| `rule.not_a_hook` | `modify` or `cancel_event` ran where no hook is being handled. |
| `rule.no_random` | `weighted` had to choose and there is no stream to draw from. |
| `rule.timers_unavailable` | A timer effect ran; timers are not built yet. |
| `rule.queries_unavailable` | `for_each` was given a query; queries are not built yet. |

In every case but the first the rule carries on with its next effect.

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

A list of effects is a field type, `Effects`, that any definition struct
may hold (04 §7.1). It keeps the list as the loader's own tree, with the
place of every value, and is compiled with the other fields of its
definition, in the scope the definition gives it; each argument inside
is read from the tree as an expression field and compiled in that scope
with whatever `let` has added by then. A definition type also asks each
of its lists to check the rules it calls, once every definition is
compiled, which is when a rule's own needs can be seen. A rule compiles
its lists itself, saying what `return` must give.

Notes on the steps:

- A diagnostic from any step carries a stable code, a severity, a message,
  the range of bytes of the source string it is about, and a suggestion
  where there is one. Codes are `X1..` for syntax, `X2..` for names, `X3..`
  for types, `X4..` for purity and belief, and `X9..` for run-time
  warnings. The data layer adds the file, line and JSON path, and gives
  each a code of its own group `06` (04 §8.2), which is the code a modder
  sees and `explain` explains.
- The field's context comes from its definition type, which states for
  each field what is in scope there (04 §7.3). The registry is the one
  the simulation filled (`clash_sim::engine_registry()`, section 5.3),
  with the ids of every definition type, the variables mods declare, the
  events mods define and the functions mods define added before anything
  is compiled. `clash-tools lint` loads with it, so a mod's expressions
  are checked against the real kinds and properties.
- A compiled expression carries an origin number, from which a run-time
  warning is traced to its definition, its JSON path and the position in
  its string.
- A function defined in data is not inlined. The machine asks its host
  for the result, and the evaluator that stands between the machine and
  the simulation runs the function's body with the arguments as locals.
- Step 2 gives every bound subject, local and event field a **slot**, its
  position in its context, which is how the machine asks for it. A text
  where an `id<T>` is wanted, compared with one, or listed against one, is
  resolved against the definitions of type `T`; one that names nothing is
  an error with the closest id. A bare name found in the mod's namespace
  and again in an import is a warning.
- Step 4 folds only what cannot fail and needs nothing from the
  simulation. A branch that can never be taken is dropped. Folding never
  changes a value, a warning or a random draw.
- Step 5 emits operations in which every jump goes forward, so evaluation
  ends. A call takes at most 255 arguments.
- What the rule runtime finds in a list of effects or in a rule has
  codes of its own in group `06`: `E0681` an effect that names nothing,
  `E0682` an effect that is not well formed, `E0683` an effect where it
  has no meaning, `E0684` a rule, a rule group or a reason that is not
  well formed.
- A direct evaluator that walks the checked tree is kept beside the stack
  machine. The two must agree on every expression, and a property test
  holds them to it.

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

**Definition types**: `eoc`, `function`, `variable`, `event`, `rule_group`,
`language`.

**Core events**: `sim_started` (once, when a game begins), `entity_created`,
`entity_destroyed`, `var_changed` (subject, name, old and new value; only
for variables declared with `"watch": true`), `setting_changed` (id, old,
new), `rule_error` (the rule, the reason, the trace).

`var_changed` is built (section 6): subjects `self?`: entity, `here?`:
loc, `area?`, `site?`, `tract?`, `realm?`, of which the one the variable
lives on is bound; fields `name`: string, `scope`: string, `old`, `new`:
any. Every event of the kernel and of the world (03 §13) is an event
type of the expression registry, with `actor` and `instigator` after its
own subjects, so that a definition's scope for an event and `event.field`
type-check. A subject an event binds only sometimes is null when it is
not bound. An event a mod defines is entered under its qualified id with
its fields as declared; a subject of a kind the
simulation does not serve yet (section 5.1) is delivered to handlers
and is not bound in expressions.

**Core effects**: sections 10.2 to 10.5: `if`, `first`, `switch`,
`for_each`, `repeat`, `while`, `weighted`, `as`, `stop`, `return`, `let`,
`set`, `add`, `unset`, `run`, `queue`, `attach_rule`, `detach_rule`,
`set_rule_group`, `emit`, `modify`, `cancel_event`, `consume`, and, read
and kept until timers are built, `after`, `every` and `cancel_timer`.
`message` and `ask` (section 10.6) are verbs of the package that builds
them.

**Reasons**: the faults of section 13.2, `rule.step_limit` to
`rule.queries_unavailable`.

**Settings**: `rules.max_steps`, `rules.max_call_depth`,
`rules.max_hook_depth`, `rules.max_cascade`, `rules.max_reactive_rounds`,
`rules.max_wakeups_per_tick`, `rules.max_value_size`,
`rules.max_value_depth`, `rules.max_queued`, `rules.max_query_cells`.

**Core functions**: section 4.7.

**Core properties**, available on every subject kind: `var`, `has_tag`,
`tags`, `def`, `kind`, and for anything with a position, `loc`. Which
kind has which is in the table of section 5.3.

**Core functions answered by the simulation** (section 4.7), with their
arguments:

| Function | Gives |
|----------|-------|
| `dist(a: any, b: any)` | `real?`: metres between two things, a storey counting three, the shorter way round on wrapped axes. Each is a cell, an element, an entity, an area (its middle) or a site (its anchor). Null if either is nowhere or they are in different grids. Safe for belief |
| `dir(from: any, to: any)` | `list<int>?`: the signs of the shortest offset east, south and up, each -1, 0 or 1. Safe for belief |
| `offset(at: loc, dx: int, dy: int, dz?: int)` | `loc?`: the cell so many cells east, south and up, wrapped; null beyond an edge. Safe for belief |
| `los(a: any, b: any)` | `bool?`: whether terrain leaves a clear line, an entity seen from the height it stands at; null if either is nowhere or the line reaches space that is not loaded |
| `light(at: any)` | `real?`: how bright a cell is now (15 §3.3) |
| `now()` | `real`: universal time in seconds. Safe for belief |
| `local_now(subject: any)` | `real?`: the local time, in seconds, of the clock an entity or a cell is on, or of an area's or a realm's own |
| `setting(id: string)` | `any`: a setting's value, or null. Safe for belief |
| `has_def(type: string, id: string)` | `bool`: whether a definition of a type with a qualified id is loaded. Safe for belief |
| `exists(x)` | `bool`: whether what a handle names is still there; false for null |

**Known limits.** Fixed in the engine and not reachable from data: the
compiler's limits on nesting, tree depth, source length and arguments
(section 4.1, section 14); how many warnings a site may raise before they
are thinned (section 4.5); the number format of a text, which is `.N` and
nothing else (section 4.6); and the set of simple units (section 4.7).

**Gaps.** Reachable only from Rust for now: what is in scope in a field,
which its definition type states (04 §7.3) and a mod cannot widen; and
whether a function is random or safe for belief, which is worked out and
cannot be declared. A property of a definition reached through an id
(`here.block.material.density`, `self.def.tags`) cannot be written yet:
the registry holds properties of kinds of subject, and an id is not one;
`element.def_id` gives the qualified id as a text meanwhile. `subject.var`
on a `region`, a `subgrid`, an `action`, the scenario and the legacy
waits for those kinds. Queries, and with them value sources over a query
and `exists` in a condition, are read and kept and give null with a
warning until they are built; `for_each` over a query reports and runs
nothing. Timers (`after`, `every`, `cancel_timer`, a rule's own `every`)
are read and kept, and schedule nothing. A `run` source is taken by a
`let` in a list of effects and gives null anywhere else. A rule's limits
are read from the rule host, which the simulation answers from its
settings once it is bound to the rule runtime; until then they are the
defaults. A verb's argument is an expression of a declared type: one
that is a text template, or a list of effects, cannot be declared yet.
What `run` gives a `let` is typed `any`, whatever the rule's `returns`.

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

Measured for the first row when package X3 was built, in a release build:
a condition of 17 operations over locals evaluates in about 130 ns, and
one of 19 operations with five reads from the host in about 200 ns. The
figures come from the ignored test `time_per_evaluation` in
`crates/clash-expr/tests/machine.rs`; the benchmark and its gate are
package T8.

Measured for the next three rows when package K5 was built, in a release
build against the fake host: an event that no rule listens for is turned
away in about 30 ns, most of it making the event; an event whose one
rule the filter index turns away costs about 190 ns; an event that one
rule answers, with its bindings, filter and condition and one `set` of a
variable, about 430 ns; and one pass of a loop with one `add` to a
local, about 90 ns. The figures come from the ignored test
`time_per_run` in `crates/clash-sim/tests/rules.rs`.

## 20. Open questions

1. **Maps as values.** Settled: maps are a value type (section 3).
2. **Returning values from rules.** Settled in section 9.5: a rule may
   `return`, and a caller in an effect list may take the value.
3. **Lambdas.** Kept out. Queries cover filtering, sorting and weighting.
4. **Rule sets as toggles.** Settled in section 9.6: rule groups.
