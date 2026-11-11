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
content is built from them.

### 4.1 Attributes

```json
{ "type": "attribute", "id": "strength", "name": "Strength",
  "seat": "body", "base": 10, "min": 0 }
```

An attribute is a number with a base and a stack of **modifiers**.

- `base` is a literal or an expression (a derived attribute:
  `"base": "self.attr.strength * 0.5 + self.fn.locomotion * 4"`).
- Modifiers come from traits, held items and rules: `add`, then `mul`,
  applied in that order. Each remembers its source and is removed with it.
- `seat` is `body` or `mind`. It decides which goes where when the two are
  separated (section 8). Strength stays with the body; knowledge leaves with
  the mind.
- Values are cached and recomputed when a dependency changes.

Read as `self.attr.strength`.

### 4.2 Pools

```json
{ "type": "pool", "id": "blood", "name": "Blood",
  "max": "self.mass * 0.07", "start": "max", "regen": "0.00002 * max",
  "thresholds": [0.0, 0.4, 0.7] }
```

A pool is a bounded quantity that fills, drains and regenerates.

- A pool has a current level and a net rate, the sum of its `regen` and
  every drain and feed currently applied. Between changes the level is a
  straight line, so nothing runs per tick: when a rate changes the level is
  brought up to date, and the next threshold crossing is scheduled as a
  single wake-up.
- Crossing a threshold (a fraction of `max`) emits `pool_crossed`. What
  happens at 40 % blood is a rule.
- Verbs add, drain, and add or remove named rates.

Read as `self.pool.blood` and `self.pool.blood.frac`.

### 4.3 Tracks

```json
{ "type": "track", "id": "swordsmanship", "name": "Swordsmanship",
  "seat": "mind", "levels": "100 * $level ^ 1.6" }
```

A track is an accumulating counter with levels. `levels` gives the amount
needed for each level. Reaching one emits `track_advanced`.

How a track gains is entirely rules: on use, from training actions, from
points spent, from a quest, never. Use-based skills, experience levels,
training and cheat skills are all tracks with different rules feeding them,
and a mod may mix them freely (decision D-038).

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
| `layers` | Outermost first. Each is a material (02 §5.4) and a thickness. `weight` says how much the part's working depends on that layer. |
| `provides` | Contribution to body functions (section 6.3) |
| `channels` | Channel slots this part supplies (03 §6.2) |
| `sockets` | Where other parts may attach, how many, and which kinds |
| `exposure` | Relative likelihood of being struck by an unaimed hit |
| `surfaces` | What the part can strike with: fists, feet, claws, teeth, horns (07 §3.2) |
| `band` | The share of the body's height the part occupies, from the ground up. Decides what a shot at a given height can hit, and what cover protects (07 §11.1). The body's height is scaled by its posture (section 11.1). |
| `inside` | For organs: listed under the part that contains them |

Materials used as tissue carry three optional extras:

| Property | Meaning |
|----------|---------|
| `leak` | When breached, drains a named pool at a rate per square metre. If the pool names a substance (16 §9.5), what drains is spilled where the body is. |
| `signal` | When damaged, adds to a named pool per joule absorbed (pain, alarm, heat) |
| `heal` | Rate at which damage recovers unaided |

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

A plan is only a starting point. The instance owns its own tree.

### 6.3 Functions

```json
{ "type": "body_function", "id": "manipulation", "combine": "sum" }
```

A **function** is something the body as a whole can do to some degree:
manipulate, move about, see, breathe, circulate, think. Its capacity is the
combination (`sum`, `min`, `max` or `mean`) of every working part's
contribution, each scaled by that part's condition.

Functions are read as `self.fn.manipulation`, feed derived attributes, and
emit `function_changed` when they move. What losing a function means is a
rule: bundled content says a body with no circulation dies; a skeleton's
content says no such thing.

### 6.4 The instance

```rust
pub struct Body {
    plan: BodyPlanId,                // where it started
    scale: Real,                     // overall size factor
    parts: Vec<PartInst>,            // the live tree
    wounds: Vec<Wound>,
    functions: Vec<Real>,            // cached capacities
}

pub struct PartInst {
    id: PartKey,
    def: BodyPartId,
    parent: Option<PartIdx>, relation: On | In,
    scale: Real,
    layers: SmallVec<[LayerInst; 4]>,   // material (overridable), thickness, damage 0..=255
    state: PartState,                   // Whole | Disabled | Severed | Destroyed
}
```

A part's **condition** is `1 − Σ (layer weight × layer damage fraction)`.
Below its `disable_below` (default 0.5) the part is disabled: it contributes
nothing and its channel slots are disabled, which interrupts any action
using them (03 §6.2).

Size matters physically. Thickness scales with `scale`, area with its
square, mass with its cube. A giant is harder to cut through because there
is more of it.

A body that has never been wounded or edited does not hold a copy of its
parts. It shares them with every other unchanged body of the same plan and
scale, and is given its own at the first change. Rules cannot tell the
difference. A crowd of the unhurt costs little.

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

Every edit recomputes functions, channel slots, mass and attributes, and
emits `body_changed`. Scene S17 is a rule that calls several of these on its
target.

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

## 7. Hits and wounds

### 7.1 A hit

```rust
pub struct Hit {
    pub target: EntityId,
    pub part: Option<PartKey>,     // None: spread over the body
    pub packet: DamagePacket,      // kind, energy, intensity, cause (02 §12.1)
    pub area: Real,                // m² of contact
    pub span: Real,                // m of contact across the part (07 §3.2)
    pub dir: Option<[Real; 3]>,
}
```

Everything that harms a body arrives as a `Hit`: a strike (07), a field
(02 §16), an impact after being thrown (02 §15.2), exposure to a pressure
wave (02 §13.2). The hook `hit_incoming` lets rules alter, redirect or
cancel it first.

### 7.2 Through the layers

For the struck part, the layers are: anything worn over it, outermost first
(section 9.2), then the part's own.

```
energy = packet.energy
for each layer, outside in:
    if the layer is worn and the hit found a gap in its coverage: skip it
    capacity = material.toughness × area × thickness
    apply the packet rule of 02 §12.2 with that capacity
    record what the layer absorbed as damage to it
    if the layer was not breached: stop
    energy = what passed
```

If every layer is breached:

- Parts `inside` this one may be struck with what remains, each with a
  chance in proportion to its size.
- If the damage type is flagged `severs` and the hit's `span` is at least
  the part's girth, the part is **severed**: it and everything on it become
  an item, and the wound is on the parent.
- Otherwise what remains exits, and may strike whatever is behind.

A worn layer's damage is damage to that item.

This is the rule that broke the wall in 02 §12.2, applied to a smaller
volume. A blade concentrates its energy on a tiny area, so its intensity is
far above skin's threshold and its energy goes a long way in. A fist spreads
the same energy and stops in muscle. Tissue with an enormous threshold is
what "bulletproof" means.

### 7.3 Wounds

```rust
pub struct Wound {
    pub part: PartKey,
    pub kind: DamageTypeId,
    pub depth: u8,                 // layers breached
    pub area: Real,
    pub energy: Real,              // absorbed
    pub age: Dur,
    pub state: WoundState,         // Open | Closed | Healed
    pub cause: Cause,
}
```

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

Wounds close and heal over time at their tissues' `heal` rates, as timers.
Treatment, regeneration and scarring are verbs and rules: `close_wound`,
`reopen_wound` and `heal_part`, with the events `wound_closed` and
`wound_healed`.

Which part an unaimed hit finds, which inner part is struck, and whether a
hit finds a gap in what is worn are drawn from the simulation's random
stream. A rule on `hit_incoming` may set the part outright.

### 7.4 Spread hits

A hit with no part is distributed. Parts are chosen in proportion to
exposure, limited to those facing `dir` if given, and each receives a share
of the energy at the hit's intensity. A hit whose energy exceeds what the
whole body could absorb many times over destroys it outright without
resolving layers.

### 7.5 Destruction and remains

A destroyed or severed part becomes an item made of its materials, or
nothing, as its definition says. A body destroyed entirely leaves whatever
its template's `remains` names. A body that stops working but is not
destroyed is still that body: see section 8.3.

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

### 12.3 Stored entities

A stored entity keeps every component. Its timers follow 03 §9. Its place is
`Stored`. Loading restores it exactly; rule R7 (01 §2) covers it.

## 13. Modding surface

**Definition types**: `attribute`, `pool`, `track`, `trait`, `body_part`,
`body_plan`, `body_function`, `template`, `creature`, `mind`, `item`,
`movement`, `posture`, `population`, `content_table`. Tissue extras on
`material`. `seat` on attributes, pools, tracks and traits. `limp` and
`postures` on body plans; `remains` on templates.
`ages` on items and `keeps` on containers.
Body flags on `damage_type` (`severs`).

**Events**: `entity_created`, `entity_destroyed`, `entity_died`,
`entity_revived`, `mind_released`, `mind_transferred`, `hit_landed`,
`wound_inflicted`, `wound_healed`, `part_disabled`, `part_restored`,
`part_severed`, `part_destroyed`, `body_changed`, `body_cells_damaged`,
`wound_closed`, `function_changed`,
`pool_crossed`, `track_advanced`, `trait_added`, `trait_removed`,
`item_taken`, `item_dropped`, `item_worn`, `item_removed`, `item_damaged`,
`item_broken`, `item_stage_reached`, `item_turned`, `posture_changed`,
`contents_materialised`, `became_notable`.

**Hooks**: `hit_incoming`, `exposure_incoming`, `entity_dying`,
`trait_adding`, `trait_removing`, `pool_changing`, `item_taking`,
`item_leaving`, `body_changing`, `mind_transferring`, `posture_changing`,
`passage_requested`. Each is described in the table below.

**Effects**: the verbs of sections 6.5, 8.2, 8.3, 9.2 and 11, and:
`hit`, `add_trait`, `remove_trait`, `set_attr_base`, `add_modifier`,
`remove_modifier`, `pool_add`, `pool_rate`, `advance_track`, `spawn`,
`destroy`, `apply_template`, `make_notable`, `make_anonymous`, `rename`,
`set_color`, `close_wound`, `reopen_wound`.

| Hook | Runs before | A handler may |
|------|-------------|---------------|
| `hit_incoming` | A hit is applied to a body (section 7.1) | Change its energy, intensity, part and direction; cancel |
| `exposure_incoming` | A dose reaches a body by any route (02 §16; 16 §9.5) | Change the amount; cancel |
| `entity_dying` | An entity stops for good (section 8.3) | Cancel |
| `trait_adding`, `trait_removing` | A trait is added or removed | Cancel |
| `pool_changing` | A verb changes a pool's level | Change the amount; cancel |
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

**Settings**: `hit.overkill_factor` (how many times what a whole body could
absorb destroys it outright, section 7.4).

**Queries** (05 §11): `parts`, `wounds`, `traits`, `held`.

**Properties**: `attr.*`, `pool.*`, `track.*`, `fn.*`, `mass`, `height`,
`volume`, `scale`, `part(key)` and its `condition`, `state`, `layers`,
`exposed`; `has_trait`, `trait(id).stacks`, `can(action)`, `is_alive`,
`has_mind`, `driver`, `holder`, `held(hold)`, `carried_mass`, `is_notable`,
`templates`, `name`, `posture`, `velocity`, `color`; on items, `age`.

**Gaps.** Reachable only from Rust for now: everything of section 3. The
stores, `Identity` and `Physical` are built, and the properties they back
(`mass`, `carried_mass`, `volume`, `height`, `name`, `templates`) are
registered when the kernel's subjects are. Whether an entity has a
component of a given kind can be asked in Rust and is not a property.

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
