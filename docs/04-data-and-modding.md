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

Profiles live in `profiles/` or inside a mod. The game starts a profile:
named on the command line, or chosen on the launcher screen of the default
profile. `boot_screen` names the screen definition (12) shown first.

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
| `if_mod`, `unless_mod` | Include this object only if the named mod is, or is not, in the profile. |
| `obsolete` | Section 12.2. |
| `copy_from`, `patch`, `override`, `extend`, `delete`, `scale`, `add` | Section 6. |

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
  an entry by its id: `"scale": { "phases.windup.time": 0.5 }`.
- `scale` and `add` are kept pending while definitions are folded
  together and applied once the final value is known, after inheritance:
  `scale` first, then `add`. A value written with a unit is converted
  before either. On a field the base left out, they act on the field's
  default; on a field with no default they are an error.
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
| `Dur`, `Length`, `Mass`, `Energy`, `Pressure`, `Speed`, `Temperature` | A number in SI units, or a string with a unit: `"250 ms"`, `"70 kPa"`, `"1.5 MJ"`, `"-5 C"` |
| `Ref<T>` | An id of a definition of type `T`. Resolved to a dense id during loading. |
| `StateRef` | An element state such as `"wall@brick#open"` (02 §5.1) |
| `Tag`, `TagSet` | Free-form strings; interned |
| `Text` | Player-facing text. May contain `{expression}` templates (05 §4.6) |
| `Expr<T>` | A literal of type `T`, an expression string, or `{ "expr": "…" }` (05 §7) |
| `Cond` | A condition (05 §7.2) |
| `Effects` | A list of effects (05 §10) |
| `Inline<T>` | An id, or an inline definition of type `T` |
| Enums | One of a fixed set of strings |
| `Option<T>` | Absent or `null` |

The unit list is fixed and documented: `us ms s min h d`; `mm cm m km`;
`ml L`; `g kg t`; `J kJ MJ GJ`; `W kW MW`; `N kN`; `Pa kPa MPa GPa`;
`K C`; `m/s km/h`. A field that holds a rate takes any unit of the right
quantity followed by `/s`: `"0.012 kg/s"`, `"0.02 K/s"`. The rate of a
resource whose unit is its own (16 §5.1) is a bare number, per second. A
unit of the wrong dimension for the field is an error.

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

Dense ids are assigned in sorted order of qualified id, per type. The same
set of mods always yields the same numbers.

### 7.3 Expressions

`Expr`, `Cond`, `Effects` and `Text` templates are compiled during loading
(05 §14). Each field declares what subjects and variables are in scope, so
an expression that refers to something unavailable there fails at load, not
in play.

## 8. Validation and diagnostics

### 8.1 Stages

| Stage | Catches |
|-------|---------|
| Syntax | Malformed JSON |
| Structure | Missing `type` or `id`; unknown type; duplicate id; bad `patch` or `override` target |
| Merge | Bad `extend`, `delete`, `scale`, `add`; inheritance cycles; type mismatch with parent |
| Typed load | Unknown keys; wrong types; bad units; missing required fields |
| Link | Unknown references |
| Compile | Expression syntax and type errors; unknown verbs, events, properties, variables |
| Semantic | Each type's own `validate`: ranges, mutually exclusive fields, limits such as 65,536 states |
| Cross | Whole-set checks: unreachable abstract definitions, references to obsolete definitions, shadowed imports, undeclared variables, disputed fields (section 6.3) |

A stage runs for every definition that survived the previous one. Loading
never stops at the first error.

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

### 8.3 Outcome

Errors prevent a profile from starting. Warnings do not. `--lenient`
replaces each broken definition with a placeholder and carries on, for
working on a large mod with something temporarily broken.

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

The **content fingerprint** is a hash of all finished trees in canonical
form. It is stored in saves and used by the pristine policy (02 §11).

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
type from the `engine` namespace. The placeholder remembers the original id.
If the definition returns in a later session, the content is restored.
Placeholders are inert and visibly marked.

### 12.4 Format versions

Changes to the save format itself are engine migrations, one per version
step, and are not the modder's concern.

## 13. Tools

All in `clash-tools`.

| Command | Does |
|---------|------|
| `lint <profile or dir>` | Runs the pipeline and prints every diagnostic. Non-zero exit on errors. `--json` for machines. |
| `schema --out <dir>` | Writes a JSON Schema per definition type and one for data files as a whole. `--profile <p> --with-ids` also lists every existing id per type, so editors can complete references. |
| `docs --out <dir>` | Generates the modding reference: every definition type and field, unit, diagnostic code, event, effect, property and function, with their documentation. |
| `init-mod <id>` | Creates a mod skeleton with editor settings that attach the schemas. |
| `explain <code>` | Prints the long explanation of a diagnostic. |
| `show <type> <id>` | Prints a finished definition with the origin of each value: which file set it, which patch changed it. |
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

## 20. Open questions

1. **Translation.** Deferred. The direction is fixed in section 7.1: files
   per language keyed by definition id and field path.
2. **Archives.** Deferred. Loading a mod from a single compressed file
   changes nothing else.
3. **Incremental reload.** Kept as it is: the whole pipeline is rerun,
   unless the budget in section 18 is missed.
4. **Patch conflicts.** Settled in section 6.3: disputed fields are a
   warning unless the later mod declares that it knows of the earlier.
