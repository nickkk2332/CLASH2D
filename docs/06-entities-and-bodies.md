# 06 — Entities, Bodies, Species and Items

Status: design complete, ready for implementation.
Implementation plan: [plans/first-playable.md](plans/first-playable.md).
Serves scenes S1, S4, S13, S14, S16, S17 and S20 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies what an entity is made of: the component store,
attributes and pools, traits, bodies and how they are hurt, minds, items and
how things hold other things, templates such as species, creation, movement,
and how individuals come and go as the world loads and unloads.

It extends the kernel entity of 03 §4.2 and changes nothing in it.

It does not specify how attacks are made (07), what abilities exist (08), how
characters decide (09), what they perceive (15) or what devices do (16).

Names of attributes, pools, tissues and body parts in examples are
illustrative content. The engine knows none of them (01 R10).

## 2. The picture

An **entity** is an id with **components**. A creature has a body, a mind,
attributes and traits. A sword has an item component. A locker that someone
has hidden in has a device component and contents.

A **body** is a tree of **parts**. Each part is built of **layers** of
material: skin over muscle over bone, or plating over a frame. A hit is a
damage packet (02 §12) that works inward through those layers by the same
rule that breaks walls. What it breaches becomes a **wound**. There are no
hit points. Wounds reduce what parts can do, parts supply the body's
**functions** and the channel slots that actions need, breached tissue
drains **pools**, and content decides what running low on any of that means.

A body is live data. Effects add parts, remove them, reshape them and change
what they are made of.

A **mind** is what decides for a body. It can be moved to another body, kept
in a jar, or replaced.

What a creature *is* comes from **templates**: bundles of body, attributes,
traits and choices. A species is a template. So is a background, a class, or
anything else content wants to call them. A character is built by applying
templates and the choices made within them.

## 3. The component store

```rust
pub struct EntityTable {
    kernel: Vec<Slot<KernelEntity>>,       // indexed by EntityId.index, checked by generation
}

pub struct Components {                    // held beside the entity table
    pub identity: ComponentStore<Identity>,
    pub physical: ComponentStore<Physical>,
    // one ComponentStore<C> for each component type, in a fixed order
}

pub struct ComponentStore<C> {
    dense: Vec<C>,
    owners: Vec<EntityId>,                 // owners[i] owns dense[i]
    sparse: Pages<u32>,                    // entity slot → position in dense, in pages
}
```

- Component types are a fixed set defined by the engine. Each has a name, a
  serialiser, a `StateHash` implementation and its properties (05 §5.3).
  The set is one list in the engine's source: a line in it gives a type its
  store, its `ComponentKind`, its place in saves and in the hash, and
  removal with its entity.
- A store holds at most one component for an entity slot. It keeps the
  whole id of each owner and checks it on every lookup, so an id whose slot
  has since been given to another entity finds nothing. Whoever destroys an
  entity removes its components from every store, in one call, before the
  slot is given out again.
- Iteration is in dense order. A new component goes to the end, a replaced
  one keeps its position, and a removed one's position is taken by the
  last. Dense order is saved and restored, so it is identical across runs
  and across saves, and it is hashed as it is: two stores holding the same
  components in different orders are different states. The index from slot
  to position is rebuilt on loading and is neither saved nor hashed.
- A component's fields are private to the module that owns it. Other code
  changes them through that module's functions, which report what changed
  so that events can be raised. There is no separate change log. Rules
  reach components only through properties and verbs.
- Mods do not add component types. They add variables (05 §6), attributes,
  traits and rules, which cover the same ground with full tooling support.

As held by the simulation (decision D-460). `Sim::components()` reads
every store and `Sim::components_mut()` hands them out to change. A
`Physical` is the exception: the masses of the containers round it and
the world's occupant record depend on it, so it is given and changed
only through `Sim::set_shape(entity, shape, cause)` and
`Sim::set_own_mass(entity, mass, cause)`. Both work out again the mass of
the entity and of every container round it and write each new mass to
the kernel's record of the entity and, for one that stands in the world,
to the world's occupant record (02 §8.1); `set_shape` also gives the
entity the shape's footprint and is refused if that does not fit where
it stands. Relocation does the same for the container an entity left and
the one it entered. Removing an entity removes its components. An entity
shelved with an unloaded tract (03 §4.3) keeps its id, so its components
stay where they are. The reasons are `component.missing` and
`physical.bad_measure` (03 §13).

### 3.1 Components

| Component | On | Holds |
|-----------|----|-------|
| Kernel (03 §4.2) | Everything | Place, tags, variables, control, clock, channels, plan, attachments |
| `Identity` | Everything | Definition, name, templates applied, description |
| `Physical` | Anything in the world | Mass, volume, height, footprint, cross-section |
| `Stats` | Creatures, some items | Attributes, pools, tracks (section 4) |
| `Traits` | Anything | Trait instances (section 5) |
| `Abilities` | Anything | Ability instances (08 §3.2) |
| `Body` | Creatures | Parts, wounds, functions (section 6) |
| `Mind` | Anything that decides | Section 8 |
| `Holds` | Anything that carries | What is held where (section 9.2) |
| `Item` | Items | Section 9.1 |
| `Device` | Devices | Document 16 |
| `Vehicle` | Vehicles | Document 11 |
| `Senses`, `Beliefs`, `Relations` | Minds | Document 15 |

**Identity.** The definition is named by its type and its id within the
type, since an entity may be made from a creature, an item or any other
type of definition. The name and the description are the entity's own and
are usually absent: a thing with none goes by its definition's. Templates
are listed in the order they were applied. Tags are the kernel entity's.

**Physical.** It holds what the thing itself measures: its own mass,
volume, height, footprint and cross-section, supplied by whatever knows
its make-up (the body, the item). Each is finite and at least zero. From
these and from containment (01 §6) it derives and caches the mass of
everything inside the thing:

```
mass(e)    = own mass(e) + carried(e)
carried(e) = Σ mass(c), for each c directly inside e, in order of id
```

`mass` is what the world uses when an entity becomes a body in flight
(02 §15) or collides with one, and `carried_mass` is the second term. An
entity with no `Physical` weighs nothing itself and passes on the mass of
what is inside it.

The cache is brought up to date whenever a thing's own mass changes and
whenever something enters or leaves a container, for that container and
every container around it. A sum is always made again from its parts and
never adjusted by a difference, so that the cache is bit for bit what a
from-scratch recomputation gives (invariant B3). It is saved and hashed
with the component. Containment deeper than 64 levels is refused.

## 4. Attributes, pools and tracks

Three numeric primitives. Every stat, resource and progression scheme in any
content is built from them. An entity that has them carries a `Stats`
component, and everything about them is saved and hashed with it.

### 4.1 Attributes

```json
{ "type": "attribute", "id": "strength", "name": "Strength",
  "seat": "body", "base": 10, "min": 0 }
```

An attribute is a number with a base and a stack of **modifiers**.

| Field | Meaning |
|-------|---------|
| `name` | What a reader calls it. |
| `seat` | `body` (the default) or `mind`. It decides which goes where when the two are separated (section 8). Strength stays with the body; knowledge leaves with the mind. |
| `base` | The value before modifiers: a literal, or an expression about the owner, bound as `self` (a **derived** attribute: `"base": "self.attr.strength * 0.5 + self.fn.locomotion * 4"`). Pure. Zero if left out. |
| `min`, `max` | The least and the most the value can be after modifiers. Each may be left out. |

- The value is `bound((base + Σ add) × Π mul)`: every addition, then every
  multiplication, then `min` and `max`. A base that comes to null counts as
  zero.
- Modifiers come from traits, held items, wounds, abilities, templates and
  rules. A modifier has an `add`, a `mul` or both. Each remembers its
  **source** and is removed with it: removing a source removes exactly what
  it added, and a source may add several to one attribute. They are applied
  in order of source, and those of one source in the order they were added,
  so that a sum of the same modifiers is always the same number.
- An entity may be given a base of its own (`set_attr_base`; a template's
  `attributes`). It stands in for the definition's, an expression included,
  until it is taken back.
- An entity with stats has every attribute that is defined. One it was never
  given a base or a modifier for has its definition's base.
- A derived attribute **depends on** the attributes its `base` reads
  directly, as `self.attr.<id>`. That is known when mods load, and an
  attribute that depends on itself, directly or through others, is a load
  error at its `base`. When an attribute's value changes, exactly those that
  depend on it are worked out again, each after what it reads, and then the
  pools whose `max` or `regen` read one that changed. Nothing else is
  touched.
- A `base` may read more than attributes: `self.mass`, a body function, a
  variable. Whatever changes one of those has every attribute and pool of
  the entity worked out again (section 6.5 does). An attribute of the owner
  read inside a function, where loading cannot see it, is null with a
  warning, so that no value can go stale unnoticed.
- Values are cached, and the cache is bit for bit what working everything
  out from scratch gives (invariant B3).

Read as `self.attr.strength`. A change is the event `attr_changed`.

### 4.2 Pools

```json
{ "type": "pool", "id": "blood", "name": "Blood",
  "max": "self.mass * 0.07", "start": "max", "regen": "0.00002 * max",
  "thresholds": [0.0, 0.4, 0.7] }
```

A pool is a bounded quantity that fills, drains and regenerates.

| Field | Meaning |
|-------|---------|
| `name` | What a reader calls it. |
| `seat` | `body` (the default) or `mind`, as for an attribute. |
| `max` | The most it holds: an expression about the owner, `self`. Pure. Required. Null, a negative number and anything not finite count as zero. |
| `start` | What it holds when its owner is given it. May name `max`. Zero if left out. |
| `regen` | What it gains in a second of its owner's time with nothing else acting on it; negative to decay. May name `max`. Zero if left out. |
| `thresholds` | Fractions of `max`, from 0 to 1, in any order, none twice; at most 256. |

- An entity has the pools it was given and no others.
- A pool has a current level and a net rate, the sum of its `regen` and
  every drain and feed currently applied. Between changes the level is a
  straight line, kept between zero and `max`, so nothing runs per tick:
  when a rate changes the level is brought up to date, and the next
  threshold crossing is scheduled as a single wake-up.
- A rate is applied under a source, as a modifier is, and goes with it. One
  source has one rate on a pool; applying another replaces it.
- `max` and `regen` are worked out when the pool is given, when an attribute
  they read directly changes, and whenever the entity's attributes are
  worked out from scratch (section 4.1). A level above a new `max` comes
  down to it.
- Crossing a threshold emits `pool_crossed`, whatever moved the level: a
  rate, a verb, or a change of `max`. A level that rises to a threshold
  exactly has crossed it upward, and one that falls to it exactly has
  crossed it downward. One change that passes several reports each, in the
  order they were passed. A pool is not said to cross the thresholds it
  starts beyond, and one whose `max` is zero is below them all. What
  happens at 40 % blood is a rule.
- The wake-up is for the moment the line meets the next threshold in its
  way, rounded up to a whole tick, on the owner's clock. A pool with no net
  rate, or with no threshold ahead of it, schedules nothing. When the
  clock's rate changes the timeline moves the wake-up and keeps its
  remaining local time (03 §3.3), so the crossing still falls when the
  owner's own time says it should.
- Adding to a pool stops at its bounds, and says how much was added. A cost
  (08 §5.2) is taken in full or refused with the reason `pool_too_low`.
- When the owner leaves the simulation (03 §9) its pools are brought up to
  date and their wake-ups cancelled. When it returns, the local time that
  passed for them is applied in one step: none if they were paused. The
  thresholds passed are reported once each, in order.
- Verbs add, drain, and add or remove named rates.

Read as `self.pool.blood`, the level, and `self.pool.frac('blood')`, the
level as a fraction of `max`.

### 4.3 Tracks

```json
{ "type": "track", "id": "swordsmanship", "name": "Swordsmanship",
  "seat": "mind", "levels": "100 * $level ^ 1.6" }
```

A track is an accumulating counter with levels.

| Field | Meaning |
|-------|---------|
| `name` | What a reader calls it. |
| `seat` | `body` (the default) or `mind`, as for an attribute. |
| `levels` | The total the counter must reach for the track to be at level `$level`. An expression about the owner, `self`, asked for each level in turn from 1. Pure. Required. |
| `max_level` | The highest level, if there is one. No track passes level 10,000. |
| `on_advance` | Effects run for each level reached, after `track_advanced`, with the owner as `self` and the level as `$level`. |

The track is at the highest level whose total it has reached. Reaching one
emits `track_advanced`, once for that level; an advance that passes several
emits one for each, lowest first. A level reached is kept, and a track never
advanced is at level 0 with nothing accumulated. The counter only grows.

How a track gains is entirely rules: on use, from training actions, from
points spent, from a quest, never. Use-based skills, experience levels,
training and cheat skills are all tracks with different rules feeding them,
and a mod may mix them freely (decision D-038).

Read as `self.track.swordsmanship`, the level.

### 4.4 Reading and changing them

`attr`, `pool` and `track` are properties of an entity that give its
attributes, pools and tracks: subjects of the kinds `attrs`, `pools` and
`tracks`. Each definition is a property of its kind, under its qualified
id (`self.attr.core:strength`) and, where no other definition of the type
and no property of the kind has the same name, under its bare id. A
misspelt name is a load error.

| Property | Type | Is |
|----------|------|----|
| `entity.attr`, `entity.pool`, `entity.track` | `attrs`, `pools`, `tracks` | The entity's stats of that sort |
| `attrs.<id>` | `real` | An attribute's value |
| `attrs.value(attribute)` | `real` | The same, for an attribute chosen at run time |
| `pools.<id>`, `pools.level(pool)` | `real` | A pool's level now |
| `pools.frac(pool)` | `real` | The level as a fraction of `max`; 0 for a pool that holds nothing |
| `pools.max(pool)` | `real` | The most it holds |
| `pools.rate(pool)` | `real` | Its net rate, per second of its owner's time |
| `tracks.<id>`, `tracks.level(track)` | `int` | A track's level |
| `tracks.amount(track)` | `real` | What it has accumulated |

None is settable: stats change through the verbs, so that hooks run and
crossings are reported. None is safe for belief: a character reads its own
and no one else's (05 §7.3). Anything read of an entity with no stats, of a
pool it does not have, or by an id that names nothing is null. An
attribute's value and a track's level name the events that change them, so
they may be read in reactive expressions (05 §7.5). A pool's level moves
all the time and may not: react to its thresholds.

| Event | Subject | Fields |
|-------|---------|--------|
| `attr_changed` | `self` | `attr` (`id<attribute>`), `old`, `new` (`real`) |
| `pool_crossed` | `self` | `pool` (`id<pool>`); `threshold` (`real`), the fraction listed; `rising` (`bool`), true if the level rose to it or past it; `level` (`real`), the level afterwards |
| `track_advanced` | `self` | `track` (`id<track>`), `level` (`int`), the level reached |
| `pool_changing` (hook) | `self` | `pool` (`id<pool>`), `amount` (`real`), which a handler may change |

| Verb | Arguments | Does |
|------|-----------|------|
| `set_attr_base` | `who`, `attr`, `base` | Gives the entity a base of its own; `null` gives the definition's back |
| `add_modifier` | `who`, `attr`, `add`, `mul`, `key` | Adds a modifier under the rule's source and `key` |
| `remove_modifier` | `who`, `key`, `attr` | Removes what was added under `key`, to one attribute or to all |
| `pool_add` | `who`, `pool`, `amount` | Adds to a pool, negative to drain, after the hook `pool_changing` |
| `pool_rate` | `who`, `pool`, `rate`, `key` | Applies a rate under `key`; `null` removes it |
| `advance_track` | `who`, `track`, `amount` | Adds to a track's counter |

These are refused with a reason (14 §5) and change nothing:

| Reason | When | Values |
|--------|------|--------|
| `no_stats` | The entity has no stats | `entity` |
| `has_stats` | It is given stats and has them | `entity` |
| `unknown_stat` | The id is not an attribute, pool or track | `what` |
| `no_such_pool` | It does not have the pool | `entity`, `pool` |
| `has_pool` | It is given a pool it has | `entity`, `pool` |
| `pool_too_low` | A cost is more than the pool holds | `entity`, `pool`, `wanted`, `have` |
| `bad_amount` | An amount is not finite, or negative where it may not be: a cost, an advance | `amount` |
| `no_clock` | The clock the pools run on is gone | — |
| `stats_not_registered` | The definitions were loaded without the stats' definition types and properties | — |

As built (decision D-550). The simulation holds the stats and raises
`attr_changed`, `pool_crossed` and `track_advanced` from every change,
in order; `pool_changing` is raised by `pool_add` before the level
moves. Each event binds `self` and the cause's `actor` and
`instigator`. The verbs take their arguments in this order:
`set_attr_base { who, attr, base? }`, `add_modifier { who, attr, key,
add?, mul? }` (`add` defaults to 0 and `mul` to 1), `remove_modifier {
who, key, attr? }`, `pool_add { who, pool, amount }`, which gives what
was in fact added, `pool_rate { who, pool, key, rate? }` and
`advance_track { who, track, amount }`. `attr`, `pool` and `track` are
ids of their types, or texts holding qualified ids. A `key` is a text:
whatever is added under it is removed by the same key, whichever rule
asks (the source of kind `rule` numbered by the key's 64-bit FNV-1a
hash). An argument that is null where it is needed, or of the wrong
sort, is refused with `verb.bad_argument` and the value `argument`.

Stats are worked out again from scratch when a function of the body
moves, when the entity's mass or the mass of what it holds changes, and
when a variable of the entity changes that an attribute's `base` or a
pool's `max` or `regen` reads directly. Pools run on the clock the
entity is on and follow it to another; they stand still while the
entity is out of the world, and no time passes for them there: a pool
is a `Pause` timer (03 §9; decision D-552).

A pool's `stops` (section 8.3) are a field of the pool definition:

| Field | Meaning |
|-------|---------|
| `stops` | A list of `{ at, when, state }`: while the level is `below` (the default) or `above` the threshold `at`, which must be one of the pool's `thresholds`, an owner with a mind is `unconscious` or `dead`. |

## 5. Traits

A **trait** is anything an entity *has* that changes what it is or can do:
a species feature, a mutation, a perk, a blessing, a curse, a drug, a stun.

```json
{ "type": "trait", "id": "adrenaline",
  "name": "Adrenaline",
  "duration": "20 s", "stacking": "refresh",
  "modifiers": [ { "attr": "quickness", "mul": 1.3 }, { "attr": "pain_felt", "mul": 0.3 } ],
  "tags": ["stimulated"],
  "grants": ["reckless_charge"],
  "rules": [ { "on": "trait_removed", "do": [ { "add_trait": { "to": "self", "trait": "crash" } } ] } ] }
```

| Field | Meaning |
|-------|---------|
| `modifiers` | Attribute modifiers while present |
| `rates` | Pool rates while present |
| `tags` | Added to the entity |
| `grants` | Actions the entity may use (03 §6). An entry may carry a cost, a cooldown and a condition (08 §5.1). |
| `abilities` | Abilities conferred while the trait is present (08 §3.3) |
| `channels` | Channel slots added or disabled |
| `senses` | Senses added or altered (15) |
| `rules` | Attached rules (05 §9.3) |
| `on_apply`, `on_remove` | Effect lists, which may edit the body |
| `duration` | If present, the trait ends by itself. A timer (03 §9). |
| `stacking` | `none`, `refresh`, or `stack` with a `max`; a stacked trait has a `stacks` count its expressions can read |
| `group` | Traits in the same group exclude each other: adding one removes the others. Stances, forms, moods. |
| `while` | A reactive condition (05 §7.5). While false the trait is dormant and confers nothing. |
| `bound` | When added by an ability: the trait ends when that ability does (08 §7.3) |
| `look` | How its holder appears while it lasts (12 §4.4) |
| `goals`, `weights` | Goals added to the holder's mind, and changes to the weights it scores them by (09 §3) |

There is no separate concept of "status effect", "mutation" or "perk".
A trait with a duration is a status. Without one it lasts until removed.

## 6. Bodies

### 6.1 Parts

```json
{ "type": "body_part", "id": "arm", "name": "arm",
  "mass": "4 kg", "length": "0.7 m", "girth": "0.09 m",
  "layers": [
    { "material": "skin",   "thickness": "2 mm",  "weight": 0.05 },
    { "material": "muscle", "thickness": "30 mm", "weight": 0.50 },
    { "material": "bone",   "thickness": "20 mm", "weight": 0.45 }
  ],
  "provides": { "manipulation": 0.4 },
  "channels": { "arm": 1 },
  "sockets": { "wrist": { "count": 1, "accepts": ["hand"] } },
  "surfaces": [ { "id": "elbow", "kind": "impact", "area": 1.2e-3, "give": 0.01, "span": 0.04 } ],
  "band": [0.55, 0.85],
  "tags": ["limb"],
  "exposure": 1.0 }
```

| Field | Meaning |
|-------|---------|
| `name` | What a reader calls it. |
| `mass` | Its mass at scale 1. Required. |
| `length`, `girth` | How long it is and how far across, at scale 1. Required, above zero. The **area** a part presents is its silhouette, `length × girth`, and every layer of it has that area. A cut whose span is at least the girth goes all the way through (section 7.2). |
| `layers` | Outermost first, from one to 16. Each is a `material` (02 §5.4) and a `thickness`. `weight`, from 0 to 1, says how much the part's working depends on that layer; the weights of a part add up to at most 1. Layers that leave it out share what the others leave of 1, by thickness. |
| `provides` | Contribution to body functions (section 6.3): for each function, what the part gives in perfect condition. |
| `channels` | Channel slots this part supplies while it works (03 §6.2): for each channel, how many. |
| `sockets` | Where other parts may attach, by name: `count`, how many it takes (default 1), and `accepts`, the part definitions it takes (empty for any). |
| `exposure` | Relative likelihood of being struck by an unaimed hit, as a factor on the part's area. Default 1. |
| `surfaces` | What the part can strike with: fists, feet, claws, teeth, horns (07 §3.2). Each has an `id`, a `kind` (a damage type), an `area` in square metres, a `give` and a `span`. |
| `band` | The share of the body's height the part occupies, from the ground up: `[low, high]`, each from 0 to 1. Decides what a shot at a given height can hit, and what cover protects (07 §11.1). The body's height is scaled by its posture (section 11.1). A part with no band is never found by height. |
| `disable_below` | The condition below which the part stops working (section 6.4). Default 0.5. |
| `inside` | Organs that come with the part wherever it is used: a list of `{ "id", "part" }`. In a body each is a part `in` this one, named `<this part's name>.<id>`. A plan may instead list an organ itself, with `in`. |
| `leaves` | What the part leaves when it is severed or destroyed (section 7.5): `item` (the default), or `nothing`. |
| `tags` | Free-form tags. |

Materials used as tissue carry three optional extras. They are fields of
the world's `material` type, which reads them and gives them no meaning:

| Property | Meaning |
|----------|---------|
| `leak` | `{ "pool", "rate", "closes" }`. While a wound through a layer of this material is open, it drains the named pool by `rate` for each second and each square metre of the layer the wound lays open (section 7.3). `closes` is how long such a breach leaks before it closes by itself; left out, it leaks until something closes it. If the pool names a substance (16 §9.5), what drains is spilled where the body is. |
| `signal` | `{ "pool", "per_joule" }`. When a layer of this material absorbs energy from a hit, the named pool gains `per_joule` for each joule (pain, alarm, heat), stopping at its bounds. It is added as it is: the hook `pool_changing` is for verbs (section 4.4). |
| `heal` | The share of a layer that recovers each second, unaided: `"0.5 /d"`. Left out, damage to it stays until something mends it. |

The pool is named by id, bare or qualified. A bare id is looked up in the
namespace of the material's own definition and then in every namespace in
sorted order, as the world's own references are (decision D-283). A pool
that does not exist is a load error. A body with no such pool, or with no
stats at all, simply does not leak or signal: a robot built of flesh by
mistake bleeds nothing.

A damage type may be flagged `severs` (section 7.2).

A robot's plating leaks nothing and signals nothing. A slime is one layer.

### 6.2 Plans

```json
{ "type": "body_plan", "id": "humanoid",
  "parts": [
    { "id": "torso", "part": "torso" },
    { "id": "head",  "part": "head",  "on": "torso", "socket": "neck" },
    { "id": "l_arm", "part": "arm",   "on": "torso", "socket": "shoulder", "side": "left" },
    { "id": "l_hand","part": "hand",  "on": "l_arm", "socket": "wrist" },
    { "id": "heart", "part": "heart", "in": "torso" },
    { "id": "brain", "part": "brain", "in": "head" }
  ] }
```

`on` attaches a part externally: lose the parent and it goes too. `in` puts
it inside: it can be reached only through the parent's layers.

| Field | Meaning |
|-------|---------|
| `name` | What a reader calls the plan. |
| `height` | How tall a body of this plan stands at scale 1. |
| `footprint` | The cells it fills, `[x, y, storeys]`. Default one cell. |
| `parts` | The parts, each with: `id`, its name in the body; `part`, its definition; `on` or `in`, the name of the part it is joined to; `socket`, the socket of that part it takes, for a part that is `on`; `side`, a free-form label that tells it from its twin; `scale`, its size relative to the rest of the body (default 1). |

Exactly one part is joined to nothing: the **root**. Names are not
repeated. A socket must exist on the parent's definition, accept the
part's definition, and have room. No part is joined to itself, directly or
round in a circle. A body has at most 1024 parts, organs included, and
organs nest at most eight deep by their definitions' `inside` lists. Each
of these is a load error at the entry that breaks it.

A part's place in the plan gives it its **key** in a body made from it:
the plan's parts in the order written, then the organs their definitions
bring. A key is never given to another part of the same body, so it means
the same part after others have come and gone.

A plan is only a starting point. The instance owns its own tree.

### 6.3 Functions

```json
{ "type": "body_function", "id": "manipulation", "combine": "sum" }
```

A **function** is something the body as a whole can do to some degree:
manipulate, move about, see, breathe, circulate, think. Its capacity is the
combination (`sum`, the default, `min`, `max` or `mean`) of every working
part's contribution, each scaled by that part's condition. It may have a
`name`.

Every part that `provides` the function counts. One that works contributes
what it provides times its condition. One that is disabled or gone
contributes nothing and still counts: with `min` the function is then
zero, and with `mean` it falls by that part's share. Contributions are
combined in order of part key. A function nothing provides is zero. A
part taken off by an edit (section 6.5) no longer counts at all: the body
simply has no such part.

Functions are read as `self.fn.manipulation`, feed derived attributes, and
emit `function_changed` when they move. The body does not work the
attributes out again itself: whoever is told of the change does
(section 4.1). What losing a function means is a
rule: bundled content says a body with no circulation dies; a skeleton's
content says no such thing.

### 6.4 The instance

```rust
pub struct Body {
    plan: BodyPlanId,                // where it started
    scale: Real,                     // overall size factor
    height_factor: Real,             // what its posture makes of its height
    parts: Vec<PartInst>,            // the live tree, in order of key
    wounds: Vec<Wound>,              // in order of id
    derived: Derived,                // cached: functions, slots, mass and size
    clock: ClockId, synced: Dur,     // the time wounds close and heal on (section 7.3)
    wake: Option<WakeId>,
}

pub struct PartInst {
    key: PartKey,                       // its number in this body
    name: String,                       // its name: the plan's `id`
    def: BodyPartId,
    parent: Option<PartKey>, relation: On | In,
    socket: Option<String>, side: Option<String>,
    scale: Real,                        // relative to the body's
    layers: Vec<LayerInst>,             // material (overridable), thickness, weight, damage 0..=255
    state: PartState,                   // Whole | Disabled | Severed | Destroyed
    held: bool,                         // disabled from outside
}
```

A part's **condition** is `1 − Σ (layer weight × layer damage fraction)`,
kept between 0 and 1. Below its `disable_below` (default 0.5) the part is
disabled: it contributes nothing and its channel slots are disabled, which
interrupts any action using them (03 §6.2). A part may also be disabled
from outside, whatever its condition, by `disable_part`, and is then
disabled until `restore_part`.

| State | Means |
|-------|-------|
| `whole` | There, and working. |
| `disabled` | There, and not working: its condition is below `disable_below`, or something disabled it. |
| `severed` | Gone: cut off, or gone with the part it was on. |
| `destroyed` | Gone: destroyed where it was (section 7.5). |

A part that is gone stays listed, with no layers, so that what a body has
lost can be read and still counts, as nothing, toward its functions
(section 6.3).

Size matters physically. Thickness and length scale with `scale`, area
with its square, mass with its cube. A giant is harder to cut through
because there is more of it. A part's size factor is the body's times its
own.

**What follows from the parts** is cached, saved and hashed, and always
equals a recomputation (invariants B2, B3):

| Value | Is |
|-------|----|
| Functions | Section 6.3. |
| Channel slots | For each channel, the sum over working parts of the slots they supply; and the set of those channels. When a part stops working or comes off, the change that reports it names the part, the channels and how many slots of each went, so that an action holding one can be interrupted. |
| Mass | The sum of the masses of the parts that are there. |
| Volume | For each part that is there, its mass over the mean density of its layers, by thickness. |
| Height | The plan's `height` × `scale` × the posture's factor (section 11.1). |
| Cross-section | The sum of the areas of the parts on the outside. |

Mass, volume, height and cross-section are handed to the entity's
`Physical` (section 3.1) whenever they change. The footprint is the plan's
when the body is made, and whatever `Physical` holds afterwards.

A body's **reach** is the greatest, among its working parts that have a
striking surface, of the part's own length and that of every part between
it and the root.

A body that has never been wounded or edited does not hold a copy of its
parts. It shares them with every other unchanged body of the same plan and
scale, and is given its own at the first change. Rules cannot tell the
difference. A crowd of the unhurt costs little. (Not built yet: every
body holds its own parts.)

### 6.5 Editing bodies

Because the instance owns its tree, effects can change it at any time:

| Verb | Does |
|------|------|
| `add_part` | Attach a new part to a free socket, or inside a part |
| `remove_part` | Detach a part and everything on it; optionally leave it as an item |
| `replace_part` | Swap a part's definition, keeping what is attached |
| `graft_part` | Attach a severed part, from this body or another |
| `set_layer` | Change a layer's material or thickness, on one part or many |
| `scale_part`, `scale_body` | Resize |
| `set_body_plan` | Rebuild from another plan, mapping parts by id where possible |
| `heal_part`, `damage_part` | Direct changes to layer damage |
| `disable_part`, `restore_part` | Disable a part from outside, whatever its condition, and let it go again |

Every edit recomputes functions, channel slots, mass and attributes, and
emits `body_changed`. Scene S17 is a rule that calls several of these on its
target.

Built so far, as functions in Rust: `disable_part` and `restore_part`;
`remove_part`, which returns the part and everything on it and in it as
data for whoever makes an item of it (section 7.5), and either leaves them
listed as gone or, as an edit, takes them out of the body altogether; and
`heal_part { part, share }`, which takes a share of a whole layer off the
damage of every layer of a part. Removing the root is refused.

As verbs (decision D-550): a part and a wound are subjects of their own
kinds that say whose body they are of, so these verbs take no `who`.
`close_wound { wound }`, `reopen_wound { wound }`, `heal_part { part,
share }`, `disable_part { part }`, `restore_part { part }` and
`remove_part { part, sever? }`: with `sever` true the parts stay listed
as gone, as a hit that severs leaves them, and left out the body no
longer has them. Each gives null. What came off is reported as
`part_detached` (section 7.6).

### 6.6 Bodies made of cells

Most creatures occupy a cell, or a box of cells a few wide and a few
storeys tall, and have a body as described above. A creature meant to be
**stood on, climbed or entered** is built differently. It is a subgrid
(02 §18) whose cells are its flesh.

```json
{ "type": "body_plan", "id": "colossus",
  "cells": "colossus_form",
  "parts": [ { "id": "torso", "part": "colossus_torso" },
             { "id": "l_arm", "part": "colossus_arm", "on": "torso", "side": "left" },
             { "id": "heart", "part": "colossus_heart", "in": "torso" } ] }
```

`cells` names a map (10 §3) that draws the creature storey by storey. Each
cell spec names the part it belongs to, and its elements are made of that
part's materials.

- The creature is the subgrid's hull entity (11 §3). It has a `Body` whose
  parts each own a set of cells.
- A part's condition is the share of its cells that remain, weighted by
  their damage. Functions, channel slots, leaks and signals follow as in
  sections 6.3 and 7.3. A breached cell of tissue that leaks, leaks.
- Harm reaches it as harm to cells (02 §12). There is no aiming at it: an
  attacker damages the cells they can reach. Because that is the world's
  work, `hit_incoming` does not run for it and cannot veto it (05 §8.2).
  The creature's defences are the thresholds of what it is made of. The
  simulation raises `body_cells_damaged`, with the part and what it lost,
  for rules to answer afterwards. Severing a limb is `detach`
  (02 §18.3) of its cells, after which the limb is a piece of world.
- It moves by a motion model (11 §5), usually `stride`, steered by its own
  mind as a crew steers a vessel, and turns in quarter-turns (11 §5.5).
- It does not flex. Its limbs do not move relative to its body. Its attacks
  are strikes and energy releases (07; 02 §13) that originate at a part's
  cells, with the reach and arc their definitions give.
- Whatever stands on it or inside it is in its grid and is carried
  (02 §18.1). **Clinging** to its side is being in an open cell of its grid
  beside a solid one for as long as a held gripping action lasts (07 §9).
  Letting go where there is no floor is falling out of its grid into the
  world.
- Inside, it may have compartments (11 §10) and fields.
- When it stops (section 8.3), content decides whether it remains a subgrid
  or is merged into the world as terrain of bone and hide.

The dividing line is use, not size. A giant three storeys tall that nobody
is meant to climb is an occupant. Something the size of a house, ridden
into battle with archers on its back, is a body made of cells.

A power that grows a creature across that line is a change of body plan
(section 6.5).

### 6.7 Reading a body

`fn` is a property of an entity that gives what its body can do: a subject
of the kind `functions`, the same entity seen as its functions. Each
`body_function` definition is a property of that kind, under its qualified
id and, where nothing else has the name, under its bare id, as stats are
(section 4.4). A part and a wound are subjects of their own kinds, `part`
and `wound` (05 §5.1).

| Property | Type | Is |
|----------|------|----|
| `entity.fn` | `functions` | What the entity's body can do |
| `functions.<id>`, `functions.value(function)` | `real` | A function's capacity |
| `entity.scale` | `real?` | The body's overall size factor |
| `entity.reach` | `real?` | How far it can reach to strike, in metres (section 6.4) |
| `entity.part(name)` | `part?` | A part by its name in the body, one that is gone included |
| `entity.parts` | `list<part>` | Every part that is there, in order of key |
| `entity.wounds` | `list<wound>` | Every wound, in order of id |
| `part.name` | `string` | Its name in its body |
| `part.def` | `id<body_part>` | What kind of part it is |
| `part.owner` | `entity` | Whose body it is a part of |
| `part.parent` | `part?` | The part it is joined to |
| `part.side` | `string?` | Its side label |
| `part.condition` | `real` | From 0 to 1 (section 6.4); 0 for a part that is gone |
| `part.state` | `string` | `whole`, `disabled`, `severed` or `destroyed` |
| `part.working` | `bool` | Whether it is there and not disabled |
| `part.exposed` | `bool` | Whether it is there and on the outside: it sits inside no other part |
| `part.size` | `real` | The area it presents, in square metres |
| `part.length`, `part.girth`, `part.mass` | `real` | Scale applied; a part that is gone has no mass |
| `part.reach` | `real` | Its own length and that of every part between it and the root |
| `part.layers` | `int` | How many layers it is built of |
| `part.layer_damage(layer)` | `real?` | A layer's damage, from 0 to 1; layer 0 is the outermost |
| `part.layer_material(layer)` | `id<material>?` | What a layer is made of |
| `part.layer_thickness(layer)` | `real?` | How thick a layer is, in metres |
| `part.has_tag(tag)` | `bool` | Whether its definition carries a tag |
| `part.wounds` | `list<wound>` | The wounds on it |
| `part.inside`, `part.attached` | `list<part>` | The parts that are there and sit in it, or on it |
| `wound.part` | `part` | The part it is on |
| `wound.owner` | `entity` | Whose wound it is |
| `wound.kind` | `id<damage_type>` | The kind of damage that made it |
| `wound.depth` | `int` | How many of the part's layers it goes through |
| `wound.area`, `wound.energy` | `real` | Section 7.3 |
| `wound.age` | `real` | Seconds of its owner's time since it was made |
| `wound.state` | `string` | `open` or `closed` |
| `wound.rate` | `real` | What it drains from pools each second, every pool summed |
| `wound.stump` | `bool` | Whether it is where a part came off |

None is settable: a body changes through hits and the body verbs, so that
hooks run and what follows is reported. None is safe for belief: what a
body can do, the state of its parts and the wounds it carries are its
owner's to know and anyone else's to perceive (15). A character reads its
own (05 §7.3). Anything read of an entity with no body, of a part or a
wound that is not there, or by an id that names nothing is null. A
function's capacity, a part's state and the lists name the events that
change them, so they may be read in reactive expressions (05 §7.5). A
part's condition moves a 255th at a time as it heals and may not: react
to its state.

## 7. Hits and wounds

### 7.1 A hit

```rust
pub struct Hit {
    pub target: EntityId,
    pub at: HitAt,                 // Part(key) | Height(m) | Any | Spread
    pub packet: DamagePacket,      // kind, energy, intensity, cause (02 §12.1)
    pub area: Real,                // m² of contact
    pub span: Real,                // m of contact across the part (07 §3.2)
    pub dir: Option<[Real; 3]>,    // east, south, up
}
```

Everything that harms a body arrives as a `Hit`: a strike (07), a field
(02 §16), an impact after being thrown (02 §15.2), exposure to a pressure
wave (02 §13.2). The hook `hit_incoming` lets rules alter, redirect or
cancel it first (section 7.6).

| A hit `at` | Lands |
|------------|-------|
| `Part` | On that part. |
| `Height` | On one outer part whose band (section 6.1) holds that height above the target's footing, chosen by exposed area. If none does, on nothing. |
| `Any` | On one outer part, chosen by exposed area. |
| `Spread` | On every outer part (section 7.4). |

A part's **exposed area** is its area times its `exposure`, and nothing
for a part that is gone or sits inside another. A hit that lands on
nothing is not refused: all of its energy exits.

### 7.2 Through the layers

For the struck part, the layers are: anything worn over it, outermost first
(section 9.2), then the part's own.

```
contact = min(hit.area, the part's area)
cover   = min(1, hit.span / the part's girth)
energy  = packet.energy
for each layer, outside in:
    if the layer is worn and the hit found a gap in its coverage: skip it
    if the layer is the part's own and wholly gone (damage 255): it is not in the way
    capacity = material.toughness × contact × thickness
    apply the packet rule of 02 §12.2 with that capacity and the layer's damage
    unaffected:  stop. The energy stays with whatever delivered it (07 §6.3)
    damaged:     the layer gains cover × the 255ths the rule gives; stop
    breached:    the layer gains cover × 255; energy = what passed
```

The rule is applied to the volume under the contact, which is why a small
contact goes deep. What is recorded on the layer is the share of the
part's width the breach crosses, because that is what the part's working
depends on: a blade drawn across a limb undoes it, and a needle through it
does not. A layer that is affected always gains at least one 255th, and
one that is damaged and not breached stops at 254. A hit below a layer's
threshold changes nothing, however often it is repeated.

If every layer is breached:

- One of the parts `inside` this one may be struck with what remains: each
  has a chance equal to its area over this part's, and at most one is
  struck. It is resolved in the same way, and what it passes goes on.
- If the hit named or found this one part, the damage type is flagged
  `severs`, the hit's `span` is at least the part's girth, and the part is
  `on` another, the part is **severed**: it and everything on it and in it
  come off as they were before the hit, and the wound is on the parent
  (section 7.3). The root, and a part `in` another, cannot be severed.
- Whatever energy remains exits, and is returned so that it can be carried
  on to whatever is behind.

A worn layer is handed in by whoever holds the items, with its material,
thickness, `coverage` and the item's damage. It is resolved by the same
rule and its own damage is not changed here: what it absorbed, and whether
it was unaffected, damaged or breached, is handed back as damage to that
item. A layer whose coverage is between 0 and 1 is in the way with that
chance.

Energy is accounted for: what the body's layers absorbed, what worn
layers absorbed, what was stopped by a layer it could not affect and what
exits add up to what the hit delivered.

This is the rule that broke the wall in 02 §12.2, applied to a smaller
volume. A blade concentrates its energy on a tiny area, so its intensity is
far above skin's threshold and its energy goes a long way in. A fist spreads
the same energy and stops in muscle. Tissue with an enormous threshold is
what "bulletproof" means.

### 7.3 Wounds

```rust
pub struct Wound {
    pub id: WoundId,               // its number in this body
    pub part: PartKey,
    pub kind: DamageTypeId,
    pub depth: u8,                 // layers it goes through, from the outside
    pub area: Real,                // m² of tissue it lays open
    pub energy: Real,              // absorbed
    pub age: Dur,
    pub state: WoundState,         // Open | Closed | Healed
    pub closes_in: Option<Dur>,    // local time left before it closes by itself
    pub lost: Option<PartKey>,     // for a stump: the part that came off
    pub cause: Cause,
}
```

A hit that breaches at least one layer of a part leaves a wound on it. Its
`depth` is how many layers the hit got through. Its `area` is the tissue
it lays open: the contact area, or, where that is larger, the face of the
cut, which is as wide as the contact crosses the part (the lesser of the
hit's `span` and the part's girth) and as deep as the layers it went
through. A breach of a part that already has an open wound of the same
kind makes that wound worse (deeper, larger, its time to close begun
again) and does not add another. Damage that breaches nothing leaves no
wound, and heals all the same.

A part that is severed, or destroyed while `on` another, leaves a **stump**
on the part it was joined to: a wound through every layer of that part,
with the area of the lost part's cross-section, girth × girth.

A wound has consequences, all through mechanisms already described:

| Consequence | Mechanism |
|-------------|-----------|
| The part works less well, or not at all | Layer damage lowers condition (section 6.4) |
| Actions that need the part are lost | Disabled channel slots |
| The body can do less | Functions fall (section 6.3) |
| Bleeding, leaking | Breached layers with `leak` add a drain rate to a pool |
| Pain, alarm | Layers with `signal` add to a pool |
| Everything gets worse from there | Rules on pool thresholds and functions apply traits whose modifiers slow, weaken and distract |

The last row is the death spiral of scene S14, and it is content. The engine
guarantees only that nothing is cushioned: there is no buffer between being
cut and being impaired.

A wound is open, then closed, then gone:

| State | Means |
|-------|-------|
| Open | It leaks. Each layer it goes through has its share of the wound's area, by thickness, and one whose tissue has a `leak` drains its pool by `rate × that share`. The sum for each pool is a rate on the pool with the wound as its source (section 4.2). The layers it goes through do not heal. |
| Closed | Nothing leaks: its rates are removed. Its layers recover at their tissues' `heal` rates. |
| Healed | Every layer it went through is whole again. The wound is removed and reported once. |

- An open wound closes by itself after the longest `closes` among the
  leaking tissues it goes through, if each gives one. With a tissue that
  gives none it stays open until `close_wound`. A wound through nothing
  that leaks is closed from the start.
- A damaged layer whose tissue has a `heal` rate recovers one 255th at a
  time, each after `1 / (255 × heal)` seconds of its owner's time, unless
  an open wound goes through it. As it recovers the part's condition
  rises, and a part that was disabled by its condition works again
  (`part_restored`).
- A stump is healed as soon as it has closed.
- Nothing runs per tick. A body with something ahead keeps one wake-up,
  on its owner's clock, for the earliest wound to close or layer to
  recover a 255th. It is a timer of 03 §9: when its owner leaves the
  simulation it stands still, and when the owner returns the local time
  that passed is applied in one step, each closing and healing in order.
- `close_wound { who, wound }` closes an open wound. `reopen_wound { who,
  wound }` opens a closed one again: it leaks as it did and its time to
  close starts over. `heal_part` (section 6.5) takes damage off layers
  directly, and a wound whose layers are whole again is healed whatever
  state it was in.

Which part an unaimed hit finds, which inner part is struck, and whether a
hit finds a gap in what is worn are drawn from the simulation's random
stream. A rule on `hit_incoming` may set the part outright. A hit at a
named part, with nothing worn loosely over it and nothing inside it, draws
nothing.

### 7.4 Spread hits

A spread hit is distributed. Every outer part that is there receives a
share of the energy in proportion to its exposed area (section 7.1), at
the hit's intensity, over the whole of the part: its contact is the part's
area and its cover is 1. Each part is resolved as in section 7.2. A spread
hit severs nothing: a part it takes every layer of is destroyed.

`dir`, if given, is meant to limit the hit to the parts that face it.
Nothing yet says which way a body or a part of it faces, so every exposed
part takes its share, and `dir` is carried on with whatever exits.

A hit whose energy exceeds what the whole body could absorb many times
over destroys it outright without resolving layers. That is so when the
hit has full effect on every layer of every part that is there (its
intensity is at least twice each threshold, and no factor is zero) and its
energy is at least `hit.overkill_factor` (default 10, and never less than
1) times the sum, over those layers, of `toughness × area × thickness ×
(1 − damage) / factor`. Every part is then destroyed and that sum is what
the body absorbed.

**What the world reports** becomes a hit:

| Report | Hit |
|--------|-----|
| `OccupantExposed { kind, energy, intensity }` (02 §13.2, §14.3) | Spread. The cell was delivered `energy`; the body takes what that intensity puts into its own volume, `intensity × volume`, and never more than was delivered. |
| A body impact (02 §15.2): the energy the flying creature lost, and the intensity | Spread, with all of that energy and the direction of travel. If what it struck has a contact surface of its own (07 §6.5), a hit `Any` with that surface's kind, area and span instead. |
| A field's `damage` rule (02 §16) | As `OccupantExposed`, with the field's level times its `energy_per_level` and `intensity_per_level`. |

As built (decision D-553). The simulation answers the world's
`occupant_exposed` after it has passed on its own `entity_exposed`: an
entity with a body takes the spread hit through `hit_incoming` and the
layers like any other, and one with none is not hurt. The world's
`body_impact` whose body is an entity hurts that entity with all the
energy it lost: spread with no direction, or with the element's contact
surface, its kind, area and span (the square root of its area if it
gives none), as a hit `Any`; a world with no kinetic damage type hurts
nothing. The world's `body_hit_occupant` hurts the occupant struck with
the blow's packet, spread, travelling the way the body was. Both are
`Sim::body_impact`. The world's `occupant_dosed` (02 §16) raises the hook
`exposure_incoming` and then adds what its handlers left of the amount
to the named pool, stopping at its bounds; a body with no stats or
without that pool takes nothing. The pool is text, resolved as a
tissue's is (section 6.1). The route is told to the hook and applied by
no one else: a mask is a handler (06 §9.1 `guards`, the item
package's).

What exits a body with a direction is carried on: the block of the next
cell along that direction is struck with it by the world's own rule
(02 §12.2), and `hit_exited` reports it. Which cell is next: each axis
steps whose share of the direction is at least half the greatest.

### 7.5 Destruction and remains

A part every layer of which is gone is **destroyed**. If it is `on`
another it comes off, with everything on it and in it, and leaves a stump
(section 7.3). If it is `in` another it is gone where it was. If it is the
root, the body is **in pieces**: everything `on` the root comes off as it
is, the root and what is inside it are destroyed, and its wounds are gone
with it.

A destroyed or severed part becomes an item made of its materials, or
nothing, as its definition's `leaves` says. What the body model hands on
is the part as it was: the keys, names, layers and damage of the part and
of everything that went with it, the wounds that were on them, their mass,
how it came off (severed, destroyed, or removed by an edit) and the cause.
Making the item is the item package's.

A body destroyed entirely leaves whatever its template's `remains` names.
The body model reports that the body is in pieces and decides nothing
more: whether that ends the creature is section 8.3. A body that stops
working but is not destroyed is still that body.

### 7.6 Events, the hook and refusals

| Event | Subject | Fields |
|-------|---------|--------|
| `hit_incoming` (hook) | `self` | `part` (`part?`), the part aimed at; `height` (`real?`), for a hit at a height; `spread` (`bool`); `kind` (`id<damage_type>`); `energy`, `intensity`, `area`, `span` (`real`); `dir` (`list<real>?`). A handler may change `energy`, `intensity`, `part` and `dir`, or cancel. `actor` and `instigator` are whoever is behind the hit. |
| `hit_landed` | `self` | `part` (`part`); `kind` (`id<damage_type>`); `absorbed`, `passed` (`real`), the joules the part's layers took and those that went on; `depth` (`int`), the layers the hit got through. Once for each part reached: the part struck, then a part inside it. |
| `wound_inflicted` | `self` | `wound` (`wound`), `part` (`part`), `kind` (`id<damage_type>`), `depth` (`int`), `area` (`real`). Also raised when a wound is made worse, and for a stump. |
| `wound_closed`, `wound_reopened` | `self` | `wound` (`wound`), `part` (`part`) |
| `wound_healed` | `self` | `part` (`part`), `kind` (`id<damage_type>`) |
| `part_disabled`, `part_restored`, `part_destroyed` | `self` | `part` (`part`) |
| `part_severed` | `self` | `part` (`part`), `parent` (`part?`). For the part cut, not for what went with it. |
| `function_changed` | `self` | `function` (`id<body_function>`), `old`, `new` (`real`) |
| `slots_lost`, `slots_gained` | `self` | `part` (`part`), the part that stopped or started working or came off; `channels` (`list<id<channel>>`), in order of channel; `counts` (`list<int>`), how many slots of each. The action runtime interrupts whatever held a slot that went (03 §6.2). |
| `body_destroyed` | `self` | None. The root was destroyed: the body is in pieces (section 7.5). |
| `part_detached` | `self`, `here?` | `part` (`string`), the name of the part that came off; `how` (`string`), `severed`, `destroyed` or `removed`; `mass` (`real`), kilograms of it and everything that went with it; `parts` (`int`), how many parts came off; `leaves_item` (`bool`), whether its definition says it leaves an item. In Rust it carries the part itself, for whoever makes an item of it (section 7.5). |
| `hit_exited` | `self`, `here?` | `part` (`part?`), the part it left through; `kind` (`id<damage_type>`); `energy`, `intensity` (`real`); `dir` (`list<real>?`) |
| `exposure_incoming` (hook) | `self`, `here?` | `pool` (`id<pool>`), `amount` (`real`), `route` (`string`: `breath`, `skin` or `any`), `field` (`string`, the qualified id of the field). A handler may change `amount`, or cancel. |

The hit is the verb `hit { target, kind, energy, intensity, part?,
height?, spread?, area?, span?, dir? }`: with `spread` true it is spread
over the body, with a `part` (of the target's body) it is aimed at it,
with a `height` it arrives at that height, and otherwise it lands
anywhere. It gives a map: `absorbed`, `returned` and `exited` (joules),
`destroyed` and `cancelled` (`bool`). Every event above binds the cause's
`actor` and `instigator` after its own subjects.

These are refused with a reason (14 §5) and change nothing:

| Reason | When | Values |
|--------|------|--------|
| `no_body` | The entity has no body | `entity` |
| `has_body` | It is given a body and has one | `entity` |
| `unknown_plan` | The id is not a plan a body can be built from | — |
| `no_such_part` | The body has no part with that key | `entity`, `part` |
| `part_missing` | The part is gone | `entity`, `part` |
| `root_part` | The part is the root, which cannot be taken off | `entity`, `part` |
| `no_such_wound` | The body has no wound with that id | `entity`, `wound` |
| `wound_not_open`, `wound_not_closed` | `close_wound` of one that is not open; `reopen_wound` of one that is not closed | `entity`, `wound` |
| `bad_amount` | A scale, a share or a factor is not finite and above zero | `amount` |
| `bad_hit` | A hit's energy, intensity, area, span, height or direction is negative or not finite | `what` |
| `no_clock` | The clock the body runs on is gone | — |
| `bad_shape` | The body's mass and size could not be handed on (section 3.1) | — |
| `bodies_not_registered` | The definitions were loaded without the bodies' definition types and properties | — |

## 8. Minds

### 8.1 What a mind is

```rust
pub struct Mind {
    def: Option<MindDefId>,         // behaviour profile for the AI layer (09)
    active: bool,
    stats: MindStats,               // attributes and tracks seated in the mind
    // beliefs, relations, senses in use: document 15
}
```

A creature normally has a `Body` and a `Mind` on the same entity. They are
separate components so that they can part company.

### 8.2 Moving minds

| Verb | Does |
|------|------|
| `transfer_mind { from, to }` | Moves the mind component, with its attributes, tracks, beliefs and relations, to another entity. Control mode goes with it. |
| `create_mind { in, def }` | Gives an entity a new mind |
| `remove_mind { from }` | Removes and discards it |
| `set_driver { of, driver }` | Makes one entity act for another's body without moving any mind |

- Moving a mind into another body is possession or a body swap.
- Moving it into an item is a soul in a jar. The item is then an entity with
  a mind and no body. It can perceive if it has senses and can do nothing
  that needs channels.
- A body with no active mind does not act. With a **driver** it acts as the
  driver directs: a puppet, a remote drone, a raised corpse on strings.
- A seat inside a body may make whoever sits in it that body's driver
  (11 §8.1): a machine with a pilot.
- A raised corpse that acts for itself is a body given a new mind by
  `create_mind`.

Pools, tracks and traits may state a `seat`, as attributes do. Those
seated in the mind go with it; the rest stay with the body. After a
transfer, where abilities come from is worked out again for both: one
whose source stayed behind is lost to the mind, and one whose source came
along is kept (08 §3.4).

All of scene S16 is these four verbs plus relations (15).

### 8.3 Stopping and starting

```
die { who, cause }      hook entity_dying (cancellable)  →  mind.active = false
                        actions interrupted, plan cleared, entity_died emitted
revive { who }          mind.active = true, entity_revived emitted
```

`die` is a mechanism: this body stops being driven by its mind. When it
happens is content. Bundled content calls it when vital functions reach
zero. An undead template simply does not include that rule.

After `die` the entity is still there with its body, wounds and holdings. It
can be carried, searched, eaten, raised, or have its mind taken. The
`mind_released` event at death is the hook for anything that collects them.

### 8.4 As built

Decisions D-554 and D-555. A mind is the component `Mind`: the
behaviour profile it was given, as text, and its **vital state**,
`awake`, `unconscious` or `dead`. What is seated in a mind is kept with
the rest of the entity's stats and moved with it. A driver is the
component `Driver` of the body's entity: who acts for it.

**When a creature stops is data.** Two fields say it, and the engine
knows nothing of blood or brains:

| Definition | Field | Says |
|------------|-------|------|
| `body_plan` | `needs: { <function>: <state> }` | While the body can do none of a function (its capacity is zero), its owner is in that state. |
| `pool` | `stops: [ { at, when, state } ]` | Section 4.4. |

where a state is `unconscious` or `dead`. Whenever a pool crosses a
threshold, a body changes, a mind arrives or a creature is made, the
simulation asks what these say and the worst holds: `dead` is `die`;
`unconscious` stops the mind until nothing says so (`entity_unconscious`,
then `entity_woke`). Only `revive` starts the dead; one that still
lacks what it needs stops again at once. An entity with no mind is
never stopped. A body in pieces (section 7.5) is stopped by whatever its
plan `needs`, since none of its functions is left.

`die` releases control of the entity (`control_changed`), forgets any
decision it was asked for, and raises `entity_died` and then
`mind_released`. Its body, wounds and holdings remain. Ending its
running actions is the action runtime's answer to `entity_died`.

| Verb | Arguments | Does |
|------|-----------|------|
| `die` | `who` | Section 8.3, after the hook `entity_dying` |
| `revive` | `who` | Section 8.3 |
| `create_mind` | `in`, `def?` | A new mind, awake; `def` is the behaviour profile, as text |
| `remove_mind` | `from` | Removes it with what is seated in it; control is released |
| `transfer_mind` | `from`, `to`, `swap?` | After the hook `mind_transferring`; with `swap` a mind `to` has goes the other way. The mind's vital state goes with it, and is then asked afresh of the body it arrives in |
| `set_driver` | `of`, `driver?` | After the hook `mind_transferring`; left out, nobody drives it |

Each gives null.

| Event | Subject | Fields |
|-------|---------|--------|
| `entity_dying` (hook) | `self` | None. A handler may cancel. |
| `entity_died`, `mind_released`, `entity_revived`, `entity_unconscious`, `entity_woke` | `self` | None |
| `mind_transferring` (hook) | `self`, `target?` | `swap` (`bool`). A handler may cancel. |
| `mind_transferred` | `self`, `target` | `swap` (`bool`) |
| `driver_changed` | `self`, `driver?` | None |

Whoever turns a command into an action asks `Sim::can_act(entity)`: an
entity acts if its mind is awake or it has a driver. These are refused
with a reason and change nothing:

| Reason | When | Values |
|--------|------|--------|
| `no_mind` | It has no mind (and, for acting, no driver) | `entity` |
| `has_mind` | `create_mind` in, or `transfer_mind` without `swap` into, something that has a mind | `entity` |
| `entity_dead` | It is dead: it cannot act, and cannot die again | `entity` |
| `entity_unconscious` | It is unconscious: it cannot act | `entity` |
| `not_dead` | `revive` of one that is not dead | `entity` |
| `same_entity` | An entity named as its own driver or as where its own mind goes | `entity` |

Not built: beliefs, relations and senses moving with a mind (15); the
`mind` definition type (09); where abilities come from after a transfer
(08 §3.4).

## 9. Items and holding

### 9.1 Items

```json
{ "type": "item", "id": "longsword", "name": "longsword",
  "material": "steel", "mass": "1.4 kg", "volume": 0.0006, "length": "1.1 m",
  "surfaces": [
    { "id": "edge",   "kind": "cut",    "area": 2.0e-5, "give": 0.002, "span": 0.9 },
    { "id": "point",  "kind": "pierce", "area": 4.0e-6, "give": 0.002, "span": 0.03 },
    { "id": "flat",   "kind": "impact", "area": 1.0e-3, "give": 0.005, "span": 0.9 },
    { "id": "pommel", "kind": "impact", "area": 9.0e-4, "give": 0.005, "span": 0.03 }
  ],
  "grip": { "arm": 1 },
  "grants": ["slash", "thrust"],
  "tags": ["blade"] }
```

An item is an entity with an `Item` component. Items are made of materials
and take damage by the element rule (02 §12.2) with a capacity from their
material and volume. Weapons notch and break; armour wears through.

| Field | Meaning |
|-------|---------|
| `surfaces` | Parts of the item that can be brought to bear, each with a damage type, contact area, give and span (07 §3.2). Combat turns energy over that area into intensity. |
| `grip` | Channel slots needed to wield it |
| `wear` | Which body parts it covers, in which layer order, its thickness there and its coverage (the fraction of hits it intercepts) |
| `holds` | Capacity as a container: volume, mass, what kinds |
| `stack` | Items equal in every respect (definition, material, damage, traits, variables, contents) may share one entity with a count. Anything that would change one member first splits it off as its own entity. Items that become equal again in the same hold may rejoin. |
| `grants`, `traits`, `rules` | Given to whoever holds or wears it |
| `guards` | While worn, the share of exposure it keeps out, by route: `{ "breath": 0.95 }` for a mask (02 §16) |
| `contains` | A volume it can hold of any substances, as a mixture (16 §9.5): a flask, a syringe, a cup |
| `color` | A colour this particular item carries: fixed, or picked from a list when it is made. `set_color` changes it. Glyphs draw it with `"fg": "own"` and descriptions name it (12 §4.3). Creatures may carry one too. |
| `unknown_name` | What it is called by someone who has not learned what it is (05 §4.6) |
| `look` | How whoever wears or holds it appears (12 §4.4) |
| `round` | For ammunition: the projectile it fires and with what energy (07 §11.5) |
| `interactions`, `device` | As for elements (02 §5.5; 16) |

### 9.2 Holding

Containment (01 §6) says an entity may be inside another. `Holds` says how.

| Hold | Meaning |
|------|---------|
| `Grasp(part)` | In a hand or equivalent; occupies that part's grasp |
| `Worn(parts, layer)` | Covering parts; acts as outer layers for hits |
| `Stowed` | Inside a container's capacity |
| `Seat(n)` | Riding or seated (11) |
| `Inside` | Enclosed: a person in a locker, something swallowed |
| `Fixed` | Attached: a mounted device, an implant, a grafted tool |

Moving something between holds is done by verbs (`take`, `drop`, `stow`,
`wear`, `remove`, `give`, `put_in`, `take_out`), which check capacity and
free grasps and go through relocation.

When a part can no longer hold, because it is disabled, severed or
destroyed, what it grasped is dropped where the holder is, with
`item_dropped` and the cause. What was worn on a severed part goes with
the part. When a holder is destroyed, everything it held, wore, stowed or
enclosed is left where it was, whole.

Whether a contained entity can perceive, be perceived or be hit depends on
the hold and on the container, and is defined with the relevant system: an
exposed rider can be hit; a person in a closed locker cannot be seen.

### 9.3 Latent contents

A container that generation filled does not create its items until
something looks. It keeps a reference to a content table and the positional
seed. The first search, opening or hit materialises the items, and they are
always the same items. A city of unopened drawers costs nothing.

### 9.4 Ageing

An item may age.

```json
{ "type": "item", "id": "raw_meat", "name": "raw meat",
  "ages": { "life": "2 d",
            "rate": "clamp((here.temperature - 271) / 20, 0.02, 3)",
            "stages": [ { "at": 0.7, "trait": "going_off" } ],
            "into": "rotten_meat" } }
```

Its **age** runs from 0 to 1. Between changes it is a straight line:
`rate × time / life`. Reaching a stage, and reaching 1, are each one
scheduled wake-up, moved when the rate changes. At 1 the item becomes
`into`, or is removed.

- `rate` is evaluated for where the item is. In the open that is its cell.
  In a container that states what it `keeps`, it is that instead:

```json
"when_supplied": { "keeps": { "temperature": "3 C" } }
```

  A cold box with power keeps its contents cold. Without, it keeps nothing
  and they read the room. A container may also keep `ageing` at a factor:
  a sealed jar, a stasis locker.
- The rate is worked out again when the item is moved, when its container
  changes what it keeps, and at each weather step of its region. For time
  spent unloaded, the rate found at loading is used for the whole span.
- For stacking (section 9.1), ages in the same stage count as equal, and
  a stack takes the age of its oldest member.

Food spoiling, iron rusting in the wet, wine improving, medicine expiring,
ice melting in a pack and a corpse going over are items that age.

## 10. Templates, choices and creation

### 10.1 Templates

```json
{ "type": "template", "id": "lizardfolk", "kind": "species", "name": "Lizardfolk",
  "body": "humanoid",
  "body_edits": [
    { "add_part": { "id": "tail", "part": "tail", "on": "torso", "socket": "tail" } },
    { "set_layer": { "parts": "all", "layer": 0, "material": "scale_hide" } }
  ],
  "attributes": { "strength": 11 },
  "traits": ["cold_blooded"],
  "choices": [
    { "id": "crest", "name": "Crest", "kind": "pick", "count": 1,
      "options": [
        { "id": "none",  "name": "None" },
        { "id": "frill", "name": "Frill", "traits": ["threat_display"],
          "body_edits": [ { "add_part": { "id": "frill", "part": "frill", "on": "head" } } ] } ] },
    { "id": "stature", "name": "Stature", "kind": "range", "min": 0.85, "max": 1.2, "default": 1.0,
      "apply": [ { "scale_body": "$value" } ] },
    { "id": "extras", "name": "Extras", "kind": "points", "budget": 6,
      "options": [ { "id": "venom", "cost": 4, "traits": ["venom_glands"] },
                   { "id": "claws", "cost": 2, "body_edits": [ { "replace_part": { "parts": "hand", "with": "clawed_hand" } } ] } ] },
    { "id": "limbs", "name": "Limbs", "kind": "assemble",
      "parts": ["arm", "tentacle", "wing"], "budget": 8 }
  ] }
```

A template contributes a body or edits to one, attributes, pools, tracks,
traits, abilities (08), senses, rules, starting holdings, and **choices**.

`kind` is a free-form label. "Species", "background", "class", "origin" and
"cheat skill" are kinds that content invents. The engine treats all
templates alike.

### 10.2 Choices

| Kind | The chooser |
|------|-------------|
| `pick` | Picks some number of listed options |
| `points` | Spends a budget on listed options |
| `range` | Sets a number |
| `text` | Types something: a name, a description |
| `assemble` | Builds freely: adds parts from an allowed set to any free compatible socket, within a budget |

An option contributes whatever a template can. Options may carry conditions
and may themselves contain choices.

`assemble` is free part-by-part construction, bounded only by what the
template's author allows. A template that allows a great deal lets a player
build nearly any body. A template that offers three options offers three.

### 10.3 Creation

A **character spec** is an ordered list of templates and the selection made
for each of their choices. Applying a spec to a new entity is
deterministic. Specs are plain data: they are what a creation screen (12)
edits, what a scenario supplies for pre-made characters, and what a
`creature` definition holds for everyone else.

```json
{ "type": "creature", "id": "town_guard", "name": "guard",
  "templates": ["human", "soldier"],
  "select": { "soldier.rank": "private" },
  "unselected": "random",
  "mind": "guard_mind",
  "holding": "guard_kit",
  "glyph": { "ch": "g", "fg": "blue" } }
```

`unselected` says what to do with choices the definition does not fix:
`default` or `random` (positionally seeded).

### 10.4 As built

Decisions D-556 to D-558. A `template` has these fields, every one
optional:

| Field | Meaning |
|-------|---------|
| `kind`, `name` | A free-form label and what a reader calls it |
| `body` | A body plan. A later template's replaces an earlier one's |
| `attributes` | `{ <attribute>: <number> }`: bases of its own. A later contribution replaces an earlier |
| `pools` | Pools it is given, at their `start` |
| `tracks` | `{ <track>: <amount> }`: added to the counter |
| `tags` | Tags of the entity |
| `variables` | `{ <name>: <constant expression> }` |
| `rules` | Rules (`eoc`) attached to the entity (05 §9.3) |
| `traits`, `items` | Ids as written, carried for whoever gives traits and makes items: `entity_spawned` names them |
| `scale` | A factor on the body's size |
| `mind` | `true` for a mind, `false` for none, or the id of a behaviour profile as text |
| `glyph` | `{ ch, fg?, bg? }`: one character and colours by name, for presentation. A later template's replaces an earlier one's |
| `remains` | What is left of a body destroyed entirely, as written |
| `blocks` | Whether it blocks movement through its cell. Default true for one with a body |
| `choices` | Section 10.2 |

A choice has an `id` (no `.`), a `name`, a `kind` and a `default`; a
`pick` has `options` and `count` (default 1); `points` has `options`,
each with a `cost` (default 1), and a `budget`; `range` has `min`,
`max` and either `sets: "scale"` (the body's size is multiplied by the
value) or an `attribute` whose base the value becomes; `text` has
`max_length` (default 64), `suggestions` (what a random choice picks
from) and `sets: "name"` or `"description"`; `assemble` has the `parts`
it allows and a `budget`, one point a part. An option has an `id`, a
`name`, a `cost`, everything a template gives except `body`, `glyph`,
`remains` and `blocks`, and `choices` of its own, at most eight deep.

A **character spec** is `{ templates, select, unselected, seed,
anonymous }`. A choice is named by its template's id, qualified or
bare, and its own: `core:person.stature`; one inside an option by the
choice, the option and its own: `core:person.craft.smith.metal`. A
selection is a list of option or part ids, a number or a text. Every
choice is visited in the order written, nested choices right after the
option that opens them; one not selected takes its `default` (a
`pick`'s first `count` options, a `range`'s `min`, nothing for the
rest) or, with `unselected: random`, a selection drawn from a stream
seeded by `seed` alone. A selection for a choice that is not open, and
one that does not suit its choice, are refused.

Spawning makes the entity out of the world with its tags and
variables; gives it its body, then stats if anything gives a body or a
stat, bases, pools and tracks; its `Identity`, naming the first
template as its definition, every template applied, and its own name
and description; a mind; its rules; puts it in its place; and raises
`entity_spawned`. It is `Sim::spawn_template`, the verb `spawn`, the
debug command `spawn`, and the answer to a loaded tract's
`spawn_requested` whose `what` names a template, which makes an
anonymous individual whose random selections are seeded by where it
stands.

| Verb | Arguments | Does |
|------|-----------|------|
| `spawn` | `template`, `at?`, `in?`, `also?` (more templates), `random?`, `seed?`, `anonymous?` | Gives the entity made; with neither `at` nor `in` it is made out of the world |
| `make_notable` | `who` | Section 12.2; gives null |

| Event | Subject | Fields |
|-------|---------|--------|
| `entity_spawned` | `self`, `here?` | `templates`, `items`, `traits` (`list<string>`): the qualified ids applied, and what it is to start with, as written |
| `became_notable` | `self` | None |

| Reason | When | Values |
|--------|------|--------|
| `templates_not_registered` | The definitions were loaded without the `template` type | — |
| `no_templates` | A spec names no template | — |
| `unknown_template` | An id names no template | `what` |
| `unknown_choice` | A selection names a choice the spec's templates do not open | `choice` |
| `bad_selection` | A selection does not suit its choice: an option it lacks, too many or too few, over budget, out of range, too long, of the wrong shape | `choice`, `why` |

The **creation session** is the state a creation screen edits: a spec
whose every change is checked as it is made. It lists the choices open
now with their options, costs, budget spent and current selection,
drops selections for choices a change closes, and gives the complete
spec when the player is done. The application hands it to the interface
as `app.creation`: a map of `templates`, `choices` (each a map of `key`,
`name`, `kind`, `depth`, `count`, `budget`, `spent`, `left`, `min`,
`max`, `max_length`, `options`, `parts`, `value`, `by_default`),
`complete` and `problem`.

Not built: the `creature` type; `apply_template` on an entity that
exists; adding the parts an `assemble` choice chose, which waits for
`add_part`; conditions on options; a range's `apply` effects and
`body_edits`, which wait for the rule runtime and body editing.

## 11. Movement

Moving is an action (03 §6.6) whose effect is one of these verbs.

```json
{ "type": "movement", "id": "walk",
  "surface": "standable",
  "speed": "1.4 * self.fn.locomotion",
  "step_up": 0.6,
  "uses": { "legs": "all" },
  "noise": 0.3 }
```

A `movement` definition says where a mover may go (`standable`, `liquid`,
`open`, `climbable`, `solid` for burrowers and phasers), how fast, and what
it costs. The engine checks a step against the world's geometry
(02 §22 `neighbours`) and the mover's size.

| Verb | Does |
|------|------|
| `move` | One step to an adjacent cell by a movement mode |
| `sweep` | Many cells in one go along a line. Stops at the first obstruction and strikes it as a body impact (02 §15.2) with the mover's mass and speed. A speedster's dash. |
| `leap` | Launches the mover as a body with a computed velocity (02 §15). Lands where physics says. |
| `set_altitude` | Changes height within the cell for fliers and climbers |
| `teleport` | Realm transfer (02 §19) |

An entity whose footing goes (`OccupantUnsupported`), or who is struck hard
enough, becomes a world body and is the world's until it lands or is caught.

One creature that blocks movement may occupy a cell per plane. A mover may
**swap** places with a creature that yields to it: both are relocated
together, and it counts as the mover's step. Who yields is decided by the
hook `passage_requested`; members of a formation yield to each other
(09 §5.5). Shoving and forcing past are actions content defines.

### 11.1 Posture

A character is in one **posture** at a time.

```json
[
  { "type": "posture", "id": "standing", "height": 1.0 },
  { "type": "posture", "id": "crouched", "height": 0.6, "speed": 0.5, "signature": 0.6 },
  { "type": "posture", "id": "prone",    "height": 0.2, "speed": 0.15, "signature": 0.3,
    "movement": ["crawl"], "occupies": { "leg": "all" }, "traits": ["steadied"] }
]
```

| Field | Effect |
|-------|--------|
| `height` | Scales the body's height, and with it the height of every part's band. So it decides what cover hides (02 §5.3), what a shot at a given height meets (07 §11.1), and what can be reached (07 §4.3). |
| `speed`, `movement` | A factor on movement, and which movement modes are allowed |
| `signature` | A factor on how easily the body is perceived (15 §5.3) |
| `occupies` | Channel slots unavailable while in it |
| `traits` | Whatever else content wants it to carry: a steadier aim, a weaker guard |
| `look` | How the body is drawn in it (12 §4.4) |

Changing posture is an action, with a time and a noise. `set_posture`
forces one: a rule may put someone on the ground when a stagger is severe
enough (07 §7). A body with nobody acting for it takes the posture its
plan names as `limp`.

Getting up takes time, and someone lying down is below most swings and
within reach of every boot. Postures are content. A body plan lists which
it has: a horse has its own, and a snake has one.

## 12. Coming and going

### 12.1 Three ways to exist

| Form | Is | Cost |
|------|----|------|
| **Population** | A count of a kind of creature in a tract or region, with a table of creature definitions and a table of where its members are by time of day (10 §11.2) | A few bytes |
| **Individual** | A full entity | Its components |
| **Agent** | A stored individual with goals, stepped by the cold simulation (09) | A serialised entity and a small record |

### 12.2 Anonymous and notable

When a tract loads, its population is instantiated as **anonymous**
individuals, seeded positionally, so the same street has the same people in
it each time it loads.

An anonymous individual becomes **notable** the moment something about it
would be lost by forgetting it:

- it took part in an action with a controlled or notable entity;
- it was wounded, or gained or lost a trait, item or relation;
- a variable on it was set;
- a rule called `make_notable` on it.

When a tract unloads:

| Individual | Becomes |
|------------|---------|
| Anonymous | Nothing. Its population count is adjusted if it died or left. |
| Notable | An agent (09 §10). With a routine or goals it goes on living while unloaded. With neither it waits where it was. |

Named characters created by scenarios or generation are notable from the
start. `make_anonymous` makes an individual anonymous again.

As built: an `Identity` says whether its entity is anonymous; anything
without one is notable. A spawn is notable unless it asks to be
anonymous, and a template's answer to `spawn_requested` is anonymous. A
wound and a variable set on it make an anonymous individual notable,
and so does `make_notable`, each raising `became_notable`. Not built:
populations, trait, item and relation changes as triggers, and
`make_anonymous`.

### 12.3 Stored entities

A stored entity keeps every component. Its timers follow 03 §9. Its place is
`Stored`. Loading restores it exactly; rule R7 (01 §2) covers it.

## 13. Modding surface

**Definition types**: `attribute`, `pool`, `track`, `trait`, `body_part`,
`body_plan`, `body_function`, `template`, `creature`, `mind`, `item`,
`movement`, `posture`, `population`, `content_table`. Tissue extras on
`material` (`leak`, `signal`, `heal`; section 6.1). `seat` on attributes,
pools, tracks and traits. `limp`, `postures` and `needs` on body plans;
`stops` on pools; `remains` on templates.
`ages` on items and `keeps` on containers.
Body flags on `damage_type` (`severs`).

**Events**: `entity_created`, `entity_destroyed`, `entity_died`,
`entity_revived`, `mind_released`, `mind_transferred`, `hit_landed`,
`wound_inflicted`, `wound_healed`, `part_disabled`, `part_restored`,
`part_severed`, `part_destroyed`, `body_changed`, `body_cells_damaged`,
`wound_closed`, `wound_reopened`, `function_changed`, `attr_changed`,
`pool_crossed`, `track_advanced`, `trait_added`, `trait_removed`,
`item_taken`, `item_dropped`, `item_worn`, `item_removed`, `item_damaged`,
`item_broken`, `item_stage_reached`, `item_turned`, `posture_changed`,
`contents_materialised`, `became_notable`; as built also
`slots_lost`, `slots_gained`, `body_destroyed`, `part_detached`,
`hit_exited`, `entity_unconscious`, `entity_woke`, `driver_changed`
and `entity_spawned`.

**Hooks**: `hit_incoming`, `exposure_incoming`, `entity_dying`,
`trait_adding`, `trait_removing`, `pool_changing`, `item_taking`,
`item_leaving`, `body_changing`, `mind_transferring`, `posture_changing`,
`passage_requested`. Each is described in the table below,
`pool_changing` with its fields in section 4.4, and `hit_incoming` with
its fields, and the events of bodies and wounds with theirs, in
section 7.6.

**Effects**: the verbs of sections 6.5, 8.2, 8.3, 9.2 and 11, and:
`hit`, `add_trait`, `remove_trait`, `set_attr_base`, `add_modifier`,
`remove_modifier`, `pool_add`, `pool_rate`, `advance_track`, `spawn`,
`destroy`, `apply_template`, `make_notable`, `make_anonymous`, `rename`,
`set_color`, `close_wound`, `reopen_wound`, `disable_part`,
`restore_part`.

| Hook | Runs before | A handler may |
|------|-------------|---------------|
| `hit_incoming` | A hit is applied to a body (section 7.1) | Change its energy, intensity, part and direction; cancel |
| `exposure_incoming` | A dose reaches a body by any route (02 §16; 16 §9.5) | Change the amount; cancel |
| `entity_dying` | An entity stops for good (section 8.3) | Cancel |
| `trait_adding`, `trait_removing` | A trait is added or removed | Cancel |
| `pool_changing` | A verb changes a pool's level (section 4.4). Rates and regeneration do not raise it. | Change the amount; cancel |
| `item_taking`, `item_leaving` | An item enters or leaves a hold | Cancel: a thing that will not be picked up, or will not come off |
| `body_changing` | Any body verb of section 6.5 | Cancel: a body that cannot be reshaped |
| `mind_transferring` | `transfer_mind`, `set_driver` | Cancel: a mind that cannot be moved or ridden |
| `posture_changing` | A posture is forced | Cancel |
| `passage_requested` | A mover asks to swap places | Decide who yields |

**Refusals.** These are refused with a reason (14 §5) and change nothing:
`add_part` with no free socket; removing a body's root part; `create_mind`
in something that has a mind; `transfer_mind` into something that has a
mind, unless it says `"swap": true`, in which case the two change places.
Parts of an old plan that `set_body_plan` cannot map are dropped as items.
The refusals of attributes, pools and tracks are tabled in section 4.4,
and those of bodies, hits and wounds in section 7.6.

**Settings**: `hit.overkill_factor` (how many times what a whole body could
absorb destroys it outright, section 7.4).

**Queries** (05 §11): `parts`, `wounds`, `traits`, `held`.

**Properties**: `attr.*`, `pool.*`, `track.*` (section 4.4); `fn.*`,
`scale`, `reach`, `part(name)`, `parts`, `wounds` and the properties of a
part and of a wound (section 6.7); `mass`, `height`, `volume`;
`has_trait`, `trait(id).stacks`, `can(action)`, `is_alive`,
`has_mind`, `driver`, `holder`, `held(hold)`, `carried_mass`, `is_notable`,
`templates`, `name`, `posture`, `velocity`, `color`; on items, `age`.

**Registered so far** (05 §5.3), from `Identity` and `Physical`. All
read truth; none is safe for belief.

| Property | Gives |
|----------|-------|
| `entity.mass` | `real?`: kilograms, with everything inside it |
| `entity.own_mass` | `real?`: kilograms, of the entity alone; null without a `Physical`. **Settable**: every container round it and the world's record of each follow (section 3) |
| `entity.carried_mass` | `real?`: kilograms, of everything inside it |
| `entity.volume`, `entity.height` | `real?`: cubic metres; metres |
| `entity.name`, `entity.description` | `string?`: the entity's own, null for one that goes by its definition's |
| `entity.def` | `any`: the definition it was made from, as an id of whatever type that is; null for none |
| `entity.has_component(component: string)` | `bool`: by the component's name, `identity` or `physical` |

**Gaps.** Reachable only from Rust for now: `templates`, until
`template` is a definition type an id can name; the name and the
description an entity takes from its definition, which need a way to
read a field of a definition of any type; and `scale`, `color`,
`velocity` and the rest of the list above, which wait for the
components that hold them.
Whether an entity has a component of a given kind is the property
`entity.has_component`, by the component's name: `identity`,
`physical`, `stats`, `body`, `mind` or `driver`.

Built and reachable from rules once the rule runtime is bound to the
simulation: attributes, pools and tracks with their definition types,
properties, events, hook and verbs (section 4.4); bodies, hits and
wounds with theirs (sections 6.5, 6.7 and 7), harm from the world
(section 7.4); minds and vital state (section 8.4); templates, specs,
spawning and the creation session (section 10.4). The reasons of each
are tabled in its section; a verb's argument that is null or of the
wrong sort is `verb.bad_argument`, with the value `argument`. A stat's
expression that raises a run-time warning while it is worked out is
reported as the fault `stats.expression_warning`, with `entity`,
`count` and `warning`.

**Gaps.** Reachable only from Rust for now: `templates` as a property,
the name and description an entity takes from its definition, and
`scale`, `color`, `velocity`, `is_alive`, `has_mind`, `driver` and
`is_notable` as properties (the simulation answers `Sim::vital`,
`Sim::driver` and `Sim::is_notable`); a template's glyph
(`Sim::glyph`); worn layers in a hit, which wait for the item package
(`Sim::hit_with`); and the creation session, which the application
holds. A track's `on_advance` is kept and not yet run. A pool cannot
yet name a substance (16 §9.5). The queries `parts` and `wounds`
(05 §11) are list properties until queries are built. Not built: the
other verbs of section 6.5, bodies made of cells (section 6.6), shared
parts (section 6.4), which parts face a hit's direction (section 7.4),
what a pool that names a substance spills, the `creature` and `mind`
types, and populations.

## 14. Invariants

| # | Invariant |
|---|-----------|
| B1 | Nothing reduces a hit's effect except layers, hooks and rules. There is no hidden health value. |
| B2 | A part's channel slots are enabled exactly when the part is whole and nothing holds them: a grip that pins, a posture, a trait, or `disable_channel`. |
| B3 | Function capacities, mass and attribute caches always equal a from-scratch recomputation. |
| B4 | Applying the same character spec with the same seed yields the same entity. |
| B5 | A mind is on exactly one entity. Moving it leaves none behind. |
| B6 | An entity is in exactly one place: at a location, inside one entity, or stored. |
| B7 | Anonymous individuals are a pure function of population, tract and seed. Anything that makes one differ makes it notable. |
| B8 | Component iteration order is identical across runs, saves and loads. |

## 15. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Component lookup | under 20 ns |
| Cached attribute read | under 30 ns |
| Resolve a hit on one part | under 3 µs |
| Apply a creature's spec (spawn) | under 30 µs |
| Instantiate a tract's population of 50 | under 2 ms |
| Memory, anonymous creature | under 2 KB |
| Memory, simple item | under 300 bytes |
| Pools and timers of idle entities | no cost between wake-ups |

## 16. Open questions

1. **Reference body detail.** Kept: the bundled humanoid uses limbs, hands
   and feet, head features and major organs. Finer plans are possible and
   untested for cost.
2. **Very large creatures.** Settled in section 6.6.
3. **Crowds.** Settled: the number simulated in full is capped (09 §9.3),
   and unchanged bodies share their parts (section 6.4).
4. **Item stacks with state.** Settled in section 9.1: a member that would
   change is split off first, and equal items may rejoin.
