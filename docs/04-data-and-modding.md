# 04 — Data and Modding Pipeline

Status: design complete, ready for implementation.
Implementation plan: [plans/sim-kernel.md](plans/sim-kernel.md).
Serves scenes S12, S20 and S21 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how JSON on disk becomes the immutable definitions
the simulation runs on: mods and profiles, ids, inheritance and patching,
typed loading, diagnostics, settings, hot reload, migration, and the tools
that make writing mods pleasant.

It completes the conventions fixed in 01 §7. What each definition type
*means* is specified by the document that owns it.

The standard this pipeline is held to: a modder who makes a mistake is told
what, where and what was probably meant, before the game starts; a modder
who makes a change sees it without restarting.

## 2. The picture

```
 profile ─► discover mods ─► order ─► parse files ─► collect objects by (type, id)
                                                          │
     Defs ◄─ validate ◄─ typed load ◄─ link ids ◄─ resolve inheritance ◄─ fold patches
       │
       └─► schemas · reference docs · content fingerprint
```

Everything up to "resolve inheritance" works on JSON trees and knows nothing
about what the definitions mean. Typed loading turns each finished tree into
a Rust value. Errors from every stage are collected and reported together.

## 3. Mods and profiles

### 3.1 Layout

```
mods/<mod_id>/
  mod.json
  data/**/*.json        definitions (any directory structure)
  scenarios/**/*.scn    golden scenarios the mod ships (optional)
  assets/               fonts and glyph sheets (12)
```

Mods are searched for in the install directory and in the user's data
directory. A mod is a directory. Reading mods from archives is not in the
first version.

The directories are searched in a stated order, and the first to hold a
given mod id supplies it. A copy in a later directory is ignored, with a
warning: that is how a working copy replaces an installed mod. Two
directories in one search directory that declare the same id are an error.
A mod loads under the id in its `mod.json`; a directory named otherwise is
a warning. Files and directories whose names start with a dot are not read.

### 3.2 `mod.json`

```json
{
  "id": "ashlands",
  "name": "The Ashlands",
  "version": "0.3.0",
  "engine": ">=0.1",
  "description": "A dead world and the things that live on it.",
  "authors": ["someone"],
  "requires": [ { "id": "core", "version": ">=0.1" } ],
  "after": ["weather_plus"],
  "conflicts": ["old_ashlands"],
  "imports": ["core"],
  "experimental": []
}
```

| Field | Meaning |
|-------|---------|
| `id` | The mod's namespace. Lowercase letters, digits, underscores. |
| `requires` | Mods that must be present. Loaded before this one. |
| `after` | Mods loaded before this one if present. For compatibility patches. |
| `conflicts` | Mods that may not be loaded alongside. |
| `imports` | Namespaces searched for bare references, in order (section 4.3). Defaults to the ids in `requires`. |
| `experimental` | Experimental features this mod opts in to (section 8.4). Empty by default. |
| `strict_vars` | `true` makes a variable that no definition declares an error in everything this mod writes, where it is otherwise a warning (05 §6). |

Only `id` is required. `name` defaults to the id and `version` to `0`. An
entry of `requires` is a mod id, or an object with `id` and `version`. A
version is numbers separated by dots, and a missing number counts as zero.
A requirement is one or more comparisons separated by commas, each `>=`,
`>`, `<=`, `<` or `=` and a version: `">=1.0, <2.0"`. A bare version means
exactly that version.

### 3.3 Profiles

A **profile** is a named set of mods and how the game starts with them.

```json
{
  "id": "ashlands_game",
  "name": "The Ashlands",
  "mods": ["core", "ui", "ashlands"],
  "boot_screen": "ashlands:title",
  "settings": { "core:reaction_prompts": "threats" }
}
```

A profile may also give `screens`, naming the screen for each role the
interface needs (12 §10.7), and `preferences`, defaults for the player's
display choices. The default profile is the one named `default` in
`profiles/`.

A profile is a file `<name>.json`. Profiles live in `profiles/` beside the
mods directory, or in the `profiles/` directory of a mod. The game starts a
profile: named on the command line, by file or by name (12 §3.1), or
chosen on the launcher screen of the default profile (not yet: the
application cannot switch profiles while it runs). `boot_screen` names
the screen definition (12) shown first, and is required of a profile
the application starts.

A profile is the whole list of what a game is made of. A mod that one of
its mods requires must be listed too: it is never added silently, and
leaving it out is an error that says to add it.

This is how a megamod or total conversion presents itself as its own game:
it ships a profile with its own title screen, menus and mod list, and need
not include any bundled content at all.

A save records the profile's mod list and versions.

### 3.4 Load order

Mods are ordered so that everything in `requires` and every present mod in
`after` comes first. Remaining ties are broken by position in the profile's
list, then by id. A cycle is an error naming the mods involved.

Load order affects exactly two things: the order in which patches to the
same definition are applied, and which mod's `override` wins.

### 3.5 Bundled mods

| Mod | Contents |
|-----|----------|
| `core` | Definitions most worlds want: common damage types, materials, atmospheres, channels, basic elements. Used by the engine's tests and tools. |
| `ui` | The default screens built with the toolkit (12). |
| `tools` | The in-game development tools (14): console, inspector, traces, profiler, reference browser, map editor. A profile that lists it is a **development profile**. |
| Reference mods | Small examples in different genres. |

The bundled mods keep their definition files under `data/` like any
other: the world content of `core` is in `mods/core/data/world/`, and it
includes two small realms, `testland` and `training_room`, that the
engine's tests and tools stand worlds in. The repository's default
profile, `profiles/default.json`, lists `core` and `ui` and boots into
`ui:title`. `profiles/dev.json` is a development profile: until the
`tools` mod exists it lists the same two mods and sets the preference
`ui:development`, which lets the omniscient view of 12 §3.2 be switched
on.

None is required by the engine. Placeholders for missing content
(section 12.3) are synthesised by the engine in the reserved namespace
`engine`, so that rule R2 holds even with no mods loaded.

## 4. Ids and namespaces

### 4.1 Form

A qualified id is `namespace:name`. Both parts match `[a-z][a-z0-9_]*`.
Ids are unique per definition type: `core:glass` the material and
`core:glass` the block do not collide.

### 4.2 Defining

- In a mod's files, `"id": "ash"` defines `<this mod>:ash`.
- A mod may define new ids only in its own namespace.
- A qualified id in another namespace is allowed only with `"patch": true`
  or `"override": true` (section 6).
- Defining an id that already exists without either is an error that says
  where the first definition is.

### 4.3 Referencing

A bare name in a reference is looked up in the mod's own namespace, then in
each namespace in `imports`, in order. The first match wins. If a later
import also has that name, the linter warns and suggests qualifying it.
A qualified reference is looked up exactly.

This replaces the "then `core`" rule sketched in 01 §7. A mod that requires
`core` gets the same behaviour by default; a total conversion without `core`
is not left with a special case.

A reference that resolves to nothing is an error with the closest existing
ids of that type as suggestions.

### 4.4 Reserved

The namespace `engine` is reserved. Dense id 0 of every element type is the
built-in `none`.

A definition type may supply definitions of the engine's own in that
namespace. They take the first dense ids of their type, ahead of every
mod's. A bare reference that matches nothing in the mod's own namespace or
its imports is looked up in `engine` last, so `"none"` can be written bare.
No mod may have `engine` as its id, define anything in it, or patch or
override what is there. The namespaces the engine keeps its settings in
(section 10) are reserved as mod ids in the same way.

## 5. Definition files

A data file is a JSON array of objects, or a single object.

```json
[
  { "//": "Anything under a // key is a comment.",
    "type": "material", "id": "ash", "density": 600, "toughness": 2.0e4 },

  { "type": "material", "id": "base_stone", "abstract": true,
    "density": 2600, "toughness": 1.5e6, "tags": ["mineral"] },

  { "type": "block", "id": "ash_drift", "material": "ash",
    "if_mod": "weather_plus", "height": 1.0, "fill": 0.3 }
]
```

| Key | Meaning |
|-----|---------|
| `type` | Definition type. Required. |
| `id` | Required. |
| `//` | Comment. Ignored anywhere. |
| `abstract` | Exists only to be inherited from. Never instantiated, not referable except by `copy_from`. |
| `if_mod`, `unless_mod` | Include this object only if the named mod is, or is not, in the profile. Each takes a mod id or a list of them: every one must be there for `if_mod`, none for `unless_mod`. An object left out is not looked at any further. |
| `obsolete` | Section 12.2. |
| `copy_from`, `patch`, `optional`, `override`, `extend`, `delete`, `scale`, `add` | Section 6. |

Keys are `snake_case`. Unknown keys are errors.

The order of keys in an object is kept, and keys added by a patch come
after those already there. Where order carries meaning (the roles of a
joint action, the subjects of an event, the parameters of a function) the
first is the first as written.

Where a definition is expected inside another (an EOC inside an action, a
map piece inside a tract), it may be written inline instead of by id. Inline
definitions get the id `<parent id>#<path>` and cannot be referenced from
elsewhere. An inline definition may be given an `id` of its own so that
another mod can patch or remove it.

The path is the JSON path from the parent (section 8.2):
`ashlands:sift#phases.sift.on_end`, `ashlands:sift#rules[1]`. An inline
definition is loaded as a definition of its own type and takes a dense id
like any other. One nested in another gets its id from the inline
definition it is in. An `id` of its own is a bare name, defined in the
namespace of the mod that wrote it.

## 6. Inheritance, patching and overriding

All three are operations on JSON trees, applied before anything is
interpreted.

### 6.1 The operations

Given a base object and a change object, the result is the base with:

| In the change | Effect |
|---------------|--------|
| A scalar or string field | Replaces the base value. |
| An object field | Merged field by field, recursively. |
| A list field | Replaces the base list. |
| `"extend": { "tags": ["x"] }` | Appends to a list or set; merges into a map. |
| `"delete": { "tags": ["y"] }` | Removes entries from a list or set; keys from a map. |
| `"scale": { "density": 1.2 }` | Multiplies a number. |
| `"add": { "span": 2 }` | Adds to a number. |

Keys inside `extend`, `delete`, `scale` and `add` may be dotted paths:
`"scale": { "resist.impact.threshold": 2.0 }`.

Deleting something that is not there, or scaling something that is not a
number, is an error. Lists of objects that carry an `id` field (phases of an
action, states of an element) are matched by that `id` when merged, so one
phase can be changed without restating the rest.

- In such a list a new entry is appended, unless it says `"before": id` or
  `"after": id`. `delete` takes entry ids. A dotted path may step through
  an entry by its id: `"scale": { "phases.windup.time": 0.5 }`. `before`
  and `after` are read only where the list is already there in what is
  being changed; an entry already in the list that names one is moved.
- Within one change the order is fixed, whatever order the keys are
  written in: plain fields, then `delete`, then `extend`. `extend` on a
  list or map that is not there yet starts it empty.
- `scale` and `add` are kept pending while definitions are folded
  together and applied once the final value is known, after inheritance:
  `scale` first, then `add`. A value written with a unit is converted
  before either, and keeps its dimension, so the field it lands in still
  checks it. An amount to `add` may itself be written with a unit; one in
  degrees Celsius is a difference of that many kelvin. On a field the
  base left out, they act on the default the definition type declares for
  it; on a field with none they are an error. A type declares defaults
  for the fields a mod might scale, and fields of list entries have none.
- `abstract` and `obsolete` are not inherited.
- A bare id, and a bare name inside an expression, is looked up in the
  namespace of the file that wrote it, wherever the value ends up after
  patching and inheritance (section 4.3).

### 6.2 `copy_from`

```json
{ "type": "material", "id": "granite", "copy_from": "base_stone",
  "toughness": 2.5e6, "extend": { "tags": ["igneous"] } }
```

The result is the parent's finished definition with this object applied as a
change. The parent must be of the same type. It may be defined later, or in
another mod. Cycles are errors.

### 6.3 `patch`

```json
{ "type": "material", "id": "core:concrete", "patch": true,
  "toughness": 2.0e6, "extend": { "tags": ["rebar"] } }
```

Changes an existing definition in place. Several mods may patch the same
definition; patches apply in load order. Patching something that does not
exist is an error, unless `"optional": true` is set, in which case the patch
is skipped.

A patch, like an override, must load after what it changes. A mod that
patches another's definition lists that mod in `requires` or `after`; a
patch that loads before its target is the same error, with a note saying
where the target is defined.

**Disputed fields.** Two mods that `scale`, `add` to or `extend` the same
field compose. Two mods that *replace* the same field with different
values do not: the later wins. That is reported as a warning naming both
mods, the field and the winner, unless the later mod lists the earlier in
its `requires` or `after`, which shows the choice was deliberate.

### 6.4 `override`

```json
{ "type": "block", "id": "core:wall", "override": true,
  "material": "brick", "height": 3.0, "fill": 0.25 }
```

Replaces the definition completely. Earlier patches to it are discarded;
later ones apply to the replacement.

### 6.5 Order

1. For each `(type, id)`, fold its objects in load order into one **authored
   object**: the original, then each patch; an override restarts the fold.
2. Resolve `copy_from` over authored objects, parents first.

Because inheritance is resolved after patches are folded, patching a parent
changes its children. A mod that makes concrete tougher makes everything
copied from concrete tougher.

Every value in a finished tree remembers the file and line it came from, so
an error in an inherited field points at the line that supplied it.

## 7. Typed loading

Each definition type is a Rust struct. A finished tree is deserialised into
it through a `serde` deserialiser written over the spanned JSON tree, which
knows the current path and source location at every point.

### 7.1 Field types

| Rust type | In JSON |
|-----------|---------|
| `bool`, integers, `Real` | Literals |
| `Time`, `Length`, `Mass`, `Energy`, `Pressure`, `Speed`, `Temperature`, `Power`, `Force`, `Volume` | A number in SI units, or a string with a unit: `"250 ms"`, `"70 kPa"`, `"1.5 MJ"`, `"-5 C"`. `Time` holds a `Dur`. |
| `Rate<Q>`, `Frequency` | A quantity per time, or a plain rate: `"0.012 kg/s"`, `"0.15 /h"` |
| `Ref<T>` | An id of a definition of type `T`. Resolved to a dense id during loading. May be the key of a map. |
| `StateRef` | An element state such as `"wall@brick#open"` (02 §5.1) |
| `Tag`, `TagSet` | Free-form strings; interned |
| `Text` | Player-facing text. May contain `{expression}` templates (05 §4.6) |
| `Expr<T>` | A literal of type `T`, an expression string, `{ "expr": "…" }` or a value source (05 §7). `T` is `Number`, `Int`, `Bool`, `Str`, `Any` or `IdOf<D>` for the id of a definition of type `D` |
| `Cond` | A condition (05 §7.2) |
| `Effects` | A list of effects (05 §10) |
| `Inline<T>` | An id, or an inline definition of type `T` |
| `Located<T>` | Any value, kept with where it was written, for a field that is checked or compiled after loading |
| `Gated<F, T>` | A field that is an experimental feature `F` (section 8.4) |
| Enums | One of a fixed set of strings; a value that carries data is an object with one key |
| `Option<T>` | Absent or `null` |

A definition type is a plain struct that derives `serde::Deserialize`. It
needs no `deny_unknown_fields`: the loader checks keys itself. It must not
use `flatten`, `untagged` or an internal `tag`, which make `serde` buffer
the value and lose its positions. A field that takes more than one shape,
an id or an object, a number or a string, is read with
`clash_data::by_kind`: the loader says what kind of value is there and the
type reads it as the type that kind calls for.

A duration in a definition is a `Time`, or a `Dur` field read through
`clash_data::dur`: `clash_core::Dur` by itself reads as a bare count of
microseconds, which is right for saves and wrong for data. A whole number
may be written with a zero fraction or an exponent (`2.0e4`).

**Writing a definition type.** The rules a definition struct follows,
in one place:

- Derive `serde::Deserialize` on a plain struct and implement
  `clash_data::Def` for it: the dense id type, made with
  `clash_core::define_id!`, the JSON name, and a line of documentation.
  Register it in the crate's `register_defs`.
- No `flatten`, no `untagged`, no internal `tag`. A field of more than
  one shape is read with `by_kind`, or by a visitor of its own that
  takes each kind of value as the loader hands it over.
- `deny_unknown_fields` is not needed and does no harm: the loader
  reports an unknown key itself, with a suggestion, and goes on.
- A quantity is one of the quantity types, never a bare `Real` where the
  JSON may carry a unit, and a duration is a `Time` or goes through
  `clash_data::dur`, never a bare `Dur`. A bare `Real` is for a number
  with no unit: a fraction, a factor, a count.
- A reference to another definition is a `Ref<T>`, or an `Inline<T>`
  where it may be written in place. A reference in a syntax of the
  type's own is a `Located<String>` resolved in `finish`.
- The source of an expression, and any other string that is checked or
  compiled after loading, is a `Located<String>`, so that the later
  error can point into the file. A field type of a crate's own that is
  compiled keeps its place and its JSON path, read with
  `clash_data::by_site`; one that is also read from plain JSON, by a
  test with `serde_json`, is read with `clash_data::by_site_or_plain`,
  which gives it its place under the loader and none otherwise.
- A field with a default that a mod might `scale` or `add` to is listed
  in `Def::DEFAULTS` (section 6.1).
- What the engine itself supplies, such as the `none` of an element
  type, comes from `Def::builtins`, so that it takes the first dense id.
- A check that needs only the definition goes in `validate`. A check
  that needs tables built from many definitions goes in the crate's
  build step (section 8.1).

The world's and the interface's types do not yet follow all of these:
their references are text resolved when their tables are built (02 §5.4,
12 §4.3), and the world reads its own quantities, among them densities
and energies per volume, which the unit list below does not have.

The unit list is fixed and documented: `us ms s min h d`; `mm cm m km`;
`ml L`; `g kg t`; `J kJ MJ GJ`; `W kW MW`; `N kN`; `Pa kPa MPa GPa`;
`K C`; `m/s km/h`. A field that holds a rate takes any unit of the right
quantity followed by `/s`, `/min`, `/h` or `/d`: `"0.012 kg/s"`,
`"0.02 K/s"`, `"0.15 /h"`. The rate of a
resource whose unit is its own (16 §5.1) is a bare number, per second. A
unit of the wrong dimension for the field is an error.

Units are case-sensitive. A speed is a length per time and a power is an
energy per time, so `m/s` and `km/h` are the rate rule applied to lengths
and a power may be written `"5 J/s"`. A rate of temperature is a
difference, so `"2 C/s"` is two kelvin per second. A bare number is in the
SI unit of the field: seconds, metres, cubic metres, kilograms, joules,
watts, newtons, pascals, kelvin, and each of those per second for a rate.

Tags are free-form, so a misspelt one loads. The linter warns of a tag
that is tested somewhere and given to nothing, or given and never tested.

All player-facing strings use `Text`, so a translation layer can be added
later without touching definitions. Translation itself is not in the first
version. When it comes it will be files per language, keyed by definition
id and field path, laid over `Text` fields, so a mod needs no change to be
translatable.

### 7.2 Linking

All ids of all types are collected and assigned dense numbers before any
definition is deserialised. A `Ref<T>` therefore resolves the moment it is
read, and a dangling reference is reported with the exact location of the
string.

Dense ids are assigned in sorted order of qualified id, per type, after
the engine's own (section 4.4). The same set of mods always yields the same
numbers. A type's ids are as wide as its Rust id type; more definitions
than that can count is an error.

Inline definitions and tags are not known until their parents are read. A
first pass reads every definition only to find them, resolving nothing and
reporting nothing, so that they are numbered in sorted order with the
rest before the pass that counts.

A reference written in a syntax of a type's own, such as a state
reference, is read as a `Located` string and resolved by the type after
its definition is read, through the same lookup a `Ref` uses.

### 7.3 Expressions

`Expr`, `Cond`, `Effects` and `Text` templates are compiled during loading
(05 §14). Each field declares what subjects and variables are in scope, so
an expression that refers to something unavailable there fails at load, not
in play.

**How a field declares its scope.** An expression field holds what was
written, where it was written and its JSON path. Its definition type
implements `Def::compile`, which the loader calls once on each definition
after every definition has been read. There the type builds a **scope**
for each field and hands the field over:

```rust
fn compile(&mut self, cx: &mut CompileCx<'_>) {
    let scope = Scope::new().subject("self", "entity").subject("here", "loc");
    cx.compile(&mut self.when, &scope);                 // a Cond
    cx.compile(&mut self.energy, &scope);               // an Expr<Number>
    cx.compile(&mut self.shown, &scope.clone().pure()); // what a screen shows
}
```

A scope states, in order: the subjects bound there with their kinds; the
locals with their types; the event whose fields can be read; whether the
value must be pure; whether it is a mind context, and which subject is
the character whose mind it is (05 §7.3); and, where the Rust type does
not say, the type the value must have. Kinds and types are named as a
mod writes them (`"entity"`, `"real"`, `"id<material>"`, `"entity?"`),
because the crate that owns a definition type need not know the
registry's numbers. Subjects and locals take slots in the order they are
added, which is the order their values are supplied in when the field is
evaluated. `cx.compile` takes a field, or an `Option`, a list or a map of
them. A field that is not handed over is not compiled: it evaluates to
null and a text renders as written.

Bare names in an expression (functions, variables, ids) are looked up in
the namespace of the mod that wrote the string, then its imports, then
`engine` (section 4.3), wherever patching and inheritance carried the
value.

**The registry.** Expressions are checked against the registry of 05
§5.3. The loader is given one already filled with the engine's kinds of
subject, properties, functions, events and verbs
(`Loader::expr_registry`); the simulation fills it. Before compiling, the
loader adds to a copy of it what the mods define:

1. the ids of every definition type, so that a text compared with an id
   is checked and a misspelling is an error with the closest id;
2. whatever a crate's own definitions declare, through a step the crate
   adds with `DefTypes::add_registry_step` (the events mods define, for
   one);
3. `variable` declarations (05 §6);
4. `function` definitions (05 §12), by signature first so that bodies may
   call each other in any order.

**Effects.** A list of effects is a field like the others: the type
holds it in an `Effects`, hands it to `cx.compile` with the scope its
effects run in, and in `validate` calls the list's own `validate`, which
checks the rules it calls against what those rules need (05 §14). A
piece of a definition that is kept as a tree and read when it is
compiled, as an effect's arguments are, is read with `from_node_at`,
which is given the JSON path of the piece so that what is found in it
is reported at its whole path.

`function` and `variable` are definition types of the data layer itself,
registered with `clash_data::expr::register_defs`. The finished registry,
the compiled functions and the variable declarations are part of `Defs`.

**Diagnostics.** What the expression compiler reports is given the form
of section 8.2: a code of group `06`, the file, the line and the column
inside the string with its escapes accounted for, the definition and the
JSON path. An error breaks the definition it is in.

**Evaluating.** A compiled field is evaluated against a host, which
supplies the values of subjects and locals and answers properties
(05 §5.3). A `Runner` holds what evaluation reuses, and
`runner.on(&defs, &mut host)` gives the evaluator a field takes:
`Expr::eval` gives the typed value, `Cond::holds` a boolean and
`Text::render` the text. The evaluator runs the functions mods define
and passes everything else to the host.

## 8. Validation and diagnostics

### 8.1 Stages

| Stage | Catches |
|-------|---------|
| Syntax | Malformed JSON |
| Structure | Missing `type` or `id`; unknown type; duplicate id; bad `patch` or `override` target |
| Merge | Bad `extend`, `delete`, `scale`, `add`; inheritance cycles; type mismatch with parent |
| Typed load | Unknown keys; wrong types; bad units; missing required fields |
| Link | Unknown references |
| Compile | Expression syntax and type errors; unknown verbs, events, properties, functions and ids; undeclared variables; purity and belief; malformed functions and variable declarations (section 7.3) |
| Semantic | Each type's own `validate`: ranges, mutually exclusive fields, limits such as 65,536 states |
| Cross | Whole-set checks: unreachable abstract definitions, references to obsolete definitions, shadowed imports, disputed fields (section 6.3) |
| Build | What a crate finds when it builds its own tables from the loaded definitions: the world's references and ranges, a theme's colours, a screen's styles, figures and art |

A stage runs for every definition that survived the previous one. Loading
never stops at the first error.

The build stage is not the loader's. It runs over a finished `Defs`,
so only once every earlier stage has passed without an error. Each crate
that builds something from definitions offers one function that does it
and returns the result or the diagnostics (`WorldDefs::from_defs`,
`UiDefs::from_defs`), and one that only checks (`check_defs`). The game
calls the first before it starts; the linter calls the second. A
diagnostic of this stage is placed like any other, at the value in the
file that last wrote it, found through the finished tree the loader keeps
for every definition.

### 8.2 Form

```
error[E0203]: unknown key "thougness"
  --> mods/ashlands/data/materials.json:14:5
   |
14 |     "thougness": 2.0e4,
   |     ^^^^^^^^^^^ did you mean "toughness"?
   = in: material ashlands:ash
```

Every diagnostic has:

- a severity: `error`, `warning` or `note`;
- a stable code, which the generated reference explains;
- the file, line and column of the offending JSON, and for expressions the
  position inside the string;
- the definition and JSON path it is in;
- where possible, a suggestion.

The same structure is available as JSON for editors and tools.

A JSON path is keys joined with dots. An entry of a list is named by its
`id` if it has one, so that its path does not change when another entry is
put before it, and by `[index]` otherwise: `resist.impact.threshold`,
`phases.windup.time`, `flags[1]`. The path follows `in:` when it is not the
definition's top level: `= in: material ashlands:ash, at resist.impact`.

Diagnostics are given in reading order: by file path, then by position.
Typed loading goes on past a mistake wherever it can, so that one load
reports every unknown key, every wrong value and every missing field of a
definition.

A code is a letter for its usual severity (`E`, `W`, `N`) and four digits.
The first two digits group codes by what they are about: `00` mods and
profiles, `01` files and syntax, `02` values and fields, `03` the keys every
definition carries, `04` merging, `05` references, `06` expressions, `07` a
type's own checks, `08` whole-set checks, `09` lenient loading. A crate that
adds a stage declares its codes with their explanations in a table of its
own, which `explain` and the reference read.

The compile stage has such a table. Each code of the expression compiler
(05 §14) has a code in group `06`: `060` and `061` for syntax, `062` and
`063` for names, `064` for types, `066` for purity and belief. `067` is
what the stage finds itself (a bad type name, a function that always
calls itself, a malformed function, variable or value source), `068` is
what the rule runtime finds in a list of effects or a rule (05 §10.1),
and `069` stands for what goes wrong while an expression runs.

### 8.3 Outcome

Errors prevent a profile from starting. Warnings do not. `--lenient`
replaces each broken definition with a placeholder and carries on, for
working on a large mod with something temporarily broken.

The errors are still reported, with a note for each definition replaced,
and the linter still exits with failure. The placeholder is supplied by
the definition type and is inert. A type that supplies none stops a
lenient load at a broken definition of it, and says so.

### 8.4 Experimental features

Anything registered with the engine (a verb, an event, a definition type,
a field) may be marked **experimental**, under a feature name. An
experimental feature is one that is specified so it can be tried. It may
change incompatibly or be removed, and nothing bundled depends on it.

- A mod that uses an experimental feature must name it in the
  `experimental` list of its `mod.json`. Using one without naming it is a
  load error that says which feature and where.
- A mod that names one is warned at every load, by name, that it relies on
  something experimental.
- The generated reference (section 13) marks experimental items and lists
  them together.
- A save records which experimental features its mods used.

Opting in is per mod: what counts is the mod that wrote the value that
uses the feature, wherever patching and inheritance carry it.

Bundled mods name none, and a test fails if one does.

Experimental features at present: `world_rewind` (13 §9.5) and
`coloured_light` (15 §3.3).

## 9. Registries

```rust
pub struct Defs { /* one Registry<T> per definition type */ }

impl<T: Def> Registry<T> {
    pub fn get(&self, id: T::Id) -> &T;
    pub fn lookup(&self, qualified: &str) -> Option<T::Id>;
    pub fn name(&self, id: T::Id) -> &str;
    pub fn iter(&self) -> impl Iterator<Item = (T::Id, &T)>;     // in id order
}
```

`Defs` is immutable and shared. The simulation holds it behind an `Arc`.

Each definition type is registered by the crate that owns it, with its Rust
type, its JSON name, its validation function and its documentation.
`clash-data` drives the pipeline without knowing any of them.

A type implements the `Def` trait: its dense id type and its JSON name, and
where it has them its documentation, the defaults a mod may `scale` or
`add` to (section 6.1), a `finish` that resolves what plain reading cannot,
a `compile` that says what is in scope in each expression field
(section 7.3), a `validate`, a placeholder (section 8.3), the engine's own
definitions (section 4.4) and the experimental feature it is part of. Each
crate offers one function that registers its types in a `DefTypes`, and may
add whole-set checks and registry steps (section 7.3) to it. The data layer
has such a function itself, for `function` and `variable`
(`clash_data::expr::register_defs`). The application, the tools and the examples load with
`clash_scene::engine_types()`, which calls each crate's function in turn, so
all of them read mods with the same types. An unknown type is an error;
a loader may be told to pass such definitions over
(`Loader::only_known_types`), which is for the tests and tools of a crate
below the owner of some types a bundled mod holds (D-450). A crate that keeps tables of its own, as the world does, builds
them from the `Defs` with the loader's dense ids (section 8.1, the build
stage): an id means the same definition in the registry and in the
crate's table. A registry also says whether a definition is a placeholder or
obsolete, and `Defs` holds the tags, the mods in load order, the profile,
the files it was read from and every finished tree.

The **content fingerprint** is a hash of all finished trees in canonical
form. It is stored in saves and used by the pristine policy (02 §11).

Canonical form is each definition's type, id and values in order, without
positions: moving a definition within a file, or between files of one mod,
changes nothing. Key order is kept, because it can carry meaning
(section 5). A string carries the namespace of the mod that wrote it,
because that decides what a bare reference in it means (section 6.1).
Abstract definitions are left out: they reach the fingerprint through what
copies from them.

## 10. Settings

```json
{ "type": "setting", "id": "reaction_prompts",
  "name": "Ask before reacting",
  "kind": "choice", "options": ["always", "threats", "never"],
  "default": "threats",
  "doc": "When the game stops to ask how you respond to an incoming action." }
```

Settings are how content exposes choices to players: difficulty-like knobs,
prompt behaviour, optional rules. They appear in the options screens (12)
and are read in expressions as `setting('reaction_prompts')`.

A setting that rules can read is **simulation state**. It is stored in the
save, and it changes only through the `Setting` command (03 §4.4), so
replays stay exact. Kinds: `flag`, `choice`, `number` with a range.

Preferences that affect only display and input belong to document 12 and
are not visible to rules.

As built (decision D-753, package P6 in part): the `setting` definition
type has `name`, `kind` (`flag`, `choice` or `number`), `options` (a
choice's values), `range` (a number's least and greatest), `default`
and `doc`; a default the setting does not take is a load error. The
`Setting { key, value }` command (03 §4.4) checks a value against the
definition of its key, where there is one (`setting.bad_value`), and is
refused for a setting the scenario being played locks
(`setting.locked`); a change raises `setting_changed` with `key`, `old`
and `new`. A scenario's `settings`, `locked` and `options` name settings
by key (13 §3). Not built: showing settings on the options screens, and
a setting's range on an engine setting.

**Engine settings.** The limits and thresholds the specifications name
(`world.min_intensity`, `rules.max_cascade`, `sim.max_hot_minds` and the
rest) are settings too, registered by the engine in reserved namespaces
named after their subsystem and listed in each document's modding
surface. A profile or a scenario may set them like any other setting. A
mod may not define a setting in a reserved namespace. They are stored in
the save, so a replay uses the values the game was played with.

## 11. Hot reload

Reloading definitions while playing is a development feature, started by a
console command or by a file watcher in development builds.

1. Run the whole pipeline again. If there are errors, show them and keep
   the current definitions.
2. At the next safe point, write the entire simulation to memory using the
   save format.
3. Swap in the new `Defs`.
4. Load the simulation back from memory.

Saves refer to definitions by string id, so step 4 already does everything a
reload needs: dense ids are remapped, removed definitions become
placeholders, and migrations run. No part of the simulation needs
reload-specific code, and reload exercises the same path that rule R7
tests.

Consequences to know about:

- A running action whose definition changed continues with the new
  definition; if its current phase no longer exists, it ends.
- A stored continuation (05 §13.4) whose rule or effect path no longer
  exists is dropped with a warning.
- A reload is recorded in the input log. Replays are exact up to it, and
  exact after it only if the same files are present.

## 12. Migration and obsoletion

### 12.1 Migrations

```json
[
  { "type": "migration", "of": "block", "from": "old_wall", "to": "wall@brick" },
  { "type": "migration", "of": "material", "from": "unobtainium", "to": null },
  { "type": "migration", "of": "variable", "from": "kill_count", "to": "kills" }
]
```

When a save refers to an id that no longer exists, migrations for that type
are consulted. Chains are followed. `null` removes the thing. Types that
carry more state than an id (entities, items) may add fields to a migration;
their documents define them.

### 12.2 Obsolete definitions

`"obsolete": true` keeps a definition loadable so that existing saves work,
while excluding it from generation, spawning and lists. Referring to an
obsolete definition from a non-obsolete one is a warning.

### 12.3 Placeholders

An id with no definition and no migration becomes a placeholder of the right
type, supplied by the definition type as in section 8.3. The placeholder
keeps the original id as its own, so it is written back under that id.
If the definition returns in a later session, the content is restored.
Placeholders are inert and visibly marked. What the world's are like, and
how a save reports them, is in 02 §11.2.

### 12.4 Format versions

Changes to the save format itself are engine migrations, one per version
step, and are not the modder's concern.

## 13. Tools

All in `clash-tools`.

| Command | Does |
|---------|------|
| `lint <profile or dir>` | Runs the pipeline, then each crate's build check (section 8.1), and prints every diagnostic. Non-zero exit on errors. `--json` for machines. The directory is a directory of mods, all of which are loaded, or one mod's directory, which is loaded with what it requires. A profile is a file or a name. `--mods <dir>` names where mods are searched for, and `--lenient` is section 8.3. |
| `schema --out <dir>` | Writes a JSON Schema per definition type and one for data files as a whole. `--profile <p> --with-ids` also lists every existing id per type, so editors can complete references. |
| `docs --out <dir>` | Generates the modding reference: every definition type and field, unit, diagnostic code, event, effect, property and function, with their documentation. |
| `init-mod <id>` | Creates a mod skeleton with editor settings that attach the schemas. |
| `explain <code>` | Prints the long explanation of a diagnostic. |
| `show <type> <id>` | Prints a finished definition with the origin of each value: the file and line that last set it and the mod that file belongs to. The type may be left out. Abstract definitions can be shown. |
| `scenario run` | Runs golden scenarios, including those shipped by mods (plans/world-model W11). |

Schemas and the reference are generated from the same registries the loader
uses. They cannot be out of date. A feature that is not in the generated
reference does not exist.

The full set of tools, including the console, inspector, traces, test
scripts and the map editor, is specified in
[14-tooling.md](14-tooling.md).

## 14. Total conversions

A total conversion is a profile that:

- lists its own mods and may omit every bundled one;
- names its own `boot_screen`;
- defines or overrides whatever screens, settings, scenarios and rules it
  wants.

The engine places no content of its own between starting the profile and
showing that screen.

## 15. Safety

- Mods are data. Loading a mod executes no code from it.
- File references inside a mod resolve inside that mod's directory.
- Loading has size limits (file size, nesting depth, definition count) with
  clear errors, so a malformed file cannot exhaust memory.
- When the script hook (05 §16) exists, it will be sandboxed and opt-in.

## 16. Modding surface

**Definition types**: `setting`, `migration`. **Files**: `mod.json`,
profiles.

**Keys available on every definition**: section 5.

**Expression function**: `setting(id)`. **Command**: `Setting`.

**Known limits.** These are fixed in the engine and cannot be changed from
data: the list of units (section 7.1); the definition types and their
fields; which fields have a default that `scale` and `add` can act on when
a definition leaves them out (section 6.1), so a mod that wants to scale
any other field writes its value in full; each type's own checks and the
whole-set checks (section 8.1). An inline definition that has an `id` of
its own can be referred to but not yet patched from another mod.

## 17. Invariants

| # | Invariant |
|---|-----------|
| M1 | The same profile and files always produce the same `Defs`, dense ids and fingerprint, regardless of file system order or thread count. |
| M2 | Loading reports every error it can find in one pass. |
| M3 | Every diagnostic names a file, line, column and definition. |
| M4 | `Defs` is immutable once built. |
| M5 | Nothing in the simulation holds a dense id across a definition swap; swaps go through save and load. |
| M6 | The schemas and reference are generated from the registries used for loading. |
| M7 | The engine runs with zero mods loaded. |

## 18. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Load 50,000 definitions across 2,000 files | under 2 s |
| Lint a single-file change in a mod of that size | under 2 s |
| Hot reload of a simulation with the default loaded set | under 1 s |
| Lookup by dense id | an array index |

Files are parsed in parallel. Folding and linking run in deterministic
order.

## 19. What is built first

Work package F4 (plans/world-model) builds the subset the world model needs:
sections 3.1, 3.2, 3.4, 4, 5 (without inline definitions and `if_mod`), 6.1
(`extend` and `delete`), 6.2, 6.4, 7.1 (without `Expr`, `Cond`, `Effects`),
7.2, 8 (without codes and the compile stage), 9, and the `lint` and `schema`
commands. The rest is built by the packages in plans/sim-kernel.

For the beta, F4 and packages P1 to P4 were built together. Not built
yet: the `schema` and `docs` commands and `init-mod` (P5); the `setting`
and `migration` types, the warning for a reference to an obsolete
definition and placeholders for ids a save names (P6, P7); hot reload
(P8); `Expr`, `Cond`, `Effects` and the compiling of `Text` templates,
which the expression packages add (a `Text` holds its string and where it
was written until then); patching, from another mod, an inline definition
that has an `id` of its own; the warning for tags given and never tested;
reading files in parallel.

## 20. Open questions

1. **Translation.** Deferred. The direction is fixed in section 7.1: files
   per language keyed by definition id and field path.
2. **Archives.** Deferred. Loading a mod from a single compressed file
   changes nothing else.
3. **Incremental reload.** Kept as it is: the whole pipeline is rerun,
   unless the budget in section 18 is missed.
4. **Patch conflicts.** Settled in section 6.3: disputed fields are a
   warning unless the later mod declares that it knows of the earlier.
