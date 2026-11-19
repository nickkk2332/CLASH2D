# 08 — Abilities and Powers

Status: design complete. Sections 3 to 6 and 7.1 to 7.3 are built
(packages A1 to A7); what was built, and where it differs from the
design, is in section 12.1.
Implementation plan: [plans/abilities.md](plans/abilities.md).
Serves scenes S3, S10, S15, S16, S17 and S20 of [00-vision.md](00-vision.md),
and supplies the means for most of the rest.

## 1. Scope

This document specifies what an ability is, how it is gained, grown, used,
paid for, suppressed and lost, the few engine mechanisms that abilities need
and earlier documents did not supply, and a catalogue showing how each
family of power is assembled from what exists.

Most of what a power *does* is already specified: striking (07), energy
releases (02 §13), fields (02 §16), bodies (06), minds and perception (15),
time (03), devices (16). This document adds no second way to do any of it.

"Magic", "superpower", "psionics", "ki", "skill", "technology" and "divine
gift" are labels content puts on abilities. The engine does not distinguish
them (01 R10). A mundane sword style and a planet-cracking beam are the
same kind of definition.

Names of abilities, pools and tracks in examples are illustrative content.

## 2. The picture

An **ability** is a bundle: actions it lets its owner take, things it does
passively, what using it costs, and how it grows.

A character **has** an ability because something conferred it: a template
they were built from, a trait, an item or module they carry, a rule that
granted it. The ability has a **rank** that follows a track, and
**parameters**, expressions of rank and whatever else, that its actions
read. A fire bolt's energy is `ability.power`, and `ability.power` doubles
with each rank, or grows with rage, or with how close the sea is.

Using an ability is taking one of its actions. Actions cost: from the
owner's pools, an item's stores, or things consumed. Sustained abilities
are **modes** with an upkeep. When the pool is empty, the mode ends.

An ability can go **dormant**: under water, in an anti-magic field, without
its focus. A dormant ability confers nothing until the condition clears.

Abilities leave things behind: a curse on someone, a wall of ice, a
summoned servant. These are **bound** to the ability that made them, and
end when it does.

The rest of the document is a catalogue. Flight is a movement mode
conferred by a trait. A force field is a pool that absorbs hits through a
hook. Stopping time is an area with a clock at rate zero that follows its
maker, who is anchored outside it. None of these is an engine feature.
Each is a few lines of content, and a modder who wants one that is not
listed has the same parts to build it from.

## 3. Abilities

### 3.1 Definition

```json
{ "type": "ability", "id": "pyrokinesis", "name": "Pyrokinesis",
  "tags": ["elemental", "fire"],
  "seat": "mind",
  "track": "pyrokinesis",
  "params": {
    "power": "2.0e4 * 2 ^ ability.rank",
    "range": "10 + 5 * ability.rank" },
  "always": { "traits": ["heat_tolerant"] },
  "grants": [
    { "action": "fire_bolt",  "cost": { "heat_reserve": "ability.power / 5000" } },
    { "action": "flame_wall", "at_rank": 3, "cost": { "heat_reserve": 40 }, "cooldown": "20 s" } ],
  "modes": [
    { "id": "burning_aura", "at_rank": 2, "trait": "burning_aura",
      "upkeep": { "heat_reserve": 2.0 } } ],
  "while": "self.loc.medium.phase != 'liquid'",
  "rules": [
    { "on": "action_finished", "if": "event.action.ability == ability",
      "do": [ { "advance_track": { "who": "self", "track": "pyrokinesis", "amount": 1 } } ] } ] }
```

| Field | Meaning |
|-------|---------|
| `tags` | What it is, for suppression, counters and rules that care |
| `seat` | `body` or `mind`: which it stays with when they part (06 §8) |
| `track` | The track (06 §4.3) whose level is the ability's rank |
| `params` | Named expressions its actions and traits read (section 4.2) |
| `always` | Traits, senses and modifiers conferred while the ability is active |
| `grants` | Actions it makes available (section 5.1) |
| `modes` | Sustained states that can be switched on and off (section 5.4) |
| `while` | Condition for being active (section 6.1) |
| `rules` | Attached rules, with `self` the owner and `ability` this instance |

### 3.2 Instances

```rust
pub struct AbilityInst {
    pub def: AbilityId,
    pub source: AbilitySource,      // Template | Trait(id) | Item(EntityId) | Rule(Cause)
    pub state: Active | Dormant | Suppressed,
    pub modes_on: SmallVec<[ModeId; 1]>,
    pub cooldowns: SmallVec<[(ActionDefId, WakeId); 2]>,
    pub vars: VarMap,
    pub bound: Vec<Binding>,        // what it has left in the world (section 7.3)
}
```

An entity's abilities are a component (06 §3.1). An ability instance is a
subject of kind `ability`: inside its actions, rules and traits, `ability`
is bound to it, and `ability.rank`, `ability.power`, `ability.owner` and
`ability.var.x` can be read.

As built (section 12.1, decision D-723): the instance is bound as
`ability` in what the ability works out about itself: its parameters,
`while`, its grants' parameters, costs, cooldowns and conditions, and
its modes' upkeep, costs and effects. An action reads the ability's
numbers as the parameters its grant hands it, and anything may read an
instance through `self.ability('id')`. `ability.var.x` is not built.

### 3.3 Where abilities come from

| Source | How | Ends when |
|--------|-----|-----------|
| Templates (06 §10) | `abilities` on a species, background, origin or their options | Never, unless revoked |
| Traits (06 §5) | A trait lists `abilities`. A blessing, a mutation, a transformation, a drug. | The trait does |
| Equipment (16 §11) | A module or item lists `abilities` under `when_supplied`, `when_unsupplied` or `always` | Removed, broken or unsupplied |
| Rules | `grant_ability`: a pact, an awakening, a lesson, a scroll, a theft | `revoke_ability`, or what the rule arranged |

One ability from several sources is one instance for as long as any source
remains.

Innate, learned, bestowed and equipped abilities are the same thing. A
powered suit's repulsors and a mutant's eye beams differ only in the source
and in what pays the cost.

### 3.4 Seat

A body-seated ability stays with the body. A mind-seated one travels with
the mind. A mind moved into another body (06 §8.2) brings what it knows and
gains what that body can do.

## 4. Rank, parameters and growth

### 4.1 Rank

`ability.rank` is the level of the ability's track. With no track, rank is
a number set when the ability is granted and changed by `set_rank`.

Entries in `grants`, `modes` and `always` may carry `at_rank`. They apply
from that rank up.

### 4.2 Parameters

Parameters are how one definition serves a novice and a god. An action
says `"energy": "ability.power"` and never needs to know the rank. Effort
is applied by the strike itself (07 §6.1).

Parameters can be modified exactly as attributes can (06 §4.1). A modifier
names an ability, or a tag, and a parameter:

```json
{ "ability_param": { "tag": "fire", "param": "power" }, "mul": 1.5 }
```

A staff that strengthens fire, a rival's curse that weakens it, a rage
that doubles everything, and one ability feeding another are all modifiers
from traits and items.

### 4.3 Growth

How an ability grows is how its track is fed, which is rules:

| Pattern | Rule |
|---------|------|
| By use | On finishing one of its actions, add to the track |
| By success | On `strike_landed`, on a contest won |
| By training | A training action or recipe (16 §8) whose result adds to the track |
| By teaching | A joint action (03 §8) between teacher and pupil |
| By points | A choice (05 §10.6) that spends a counter |
| By events | A threshold of a pool crossed, a brush with death, a rite |
| Not at all | No rule. It is what it is. |

A mod may use different patterns for different abilities (decision D-038).

## 5. Using abilities

### 5.1 Grants

A **grant** makes an action (03 §6) available to the owner. It may carry:

| Field | Meaning |
|-------|---------|
| `cost` | What starting the action spends (section 5.2) |
| `cooldown` | Local time before it can be used again |
| `at_rank` | Minimum rank |
| `if` | A further condition |

Grants on traits (06 §5) and on modules (16 §11.2) take the same fields.

An action unavailable for lack of cost, rank, cooldown or condition says
why, to the player and to anything asking.

### 5.2 Costs

A cost names a pool (06 §4.2), a store (16 §5), or items, and an amount.

```json
"cost": { "mana": 30, "focus_crystal": { "item": true, "count": 1 } }
```

- Costs are taken when the action starts. A phase tick may take more, for
  things that cost as long as they last.
- If the amount is not there, the action cannot start, unless the cost
  allows **overdraw**:

```json
"cost": { "mana": { "amount": 30, "overdraw": { "from": "blood", "ratio": 2.0 } } }
```

  Whatever is missing is taken from the other pool at the ratio. Pushing
  past one's limits by burning oneself is a field in a data file.
- A cost may be drawn `from` something bound to the ability (section 7.3)
  in place of the owner: a companion, a familiar, a mount.

```json
"cost": { "resolve": { "amount": 30, "from": "ability.bound('companion')" } }
```

  If that thing is gone, or has not enough, the action is unavailable and
  says so, unless the cost also allows overdraw from the owner.

### 5.3 Concentration and gesture

Abilities that need hands, a voice or attention need them as channels
(03 §6.2). An incantation uses `voice`. A sustained working uses a `focus`
slot for as long as it lasts, and a mind has only so many.

So a gagged caster cannot incant, a bound one cannot gesture, and one who
is struck hard enough to be interrupted (07 §7) loses what they were
holding. Nothing about spellcasting is special-cased.

### 5.4 Modes

A **mode** is a state an ability can be switched into: an aura, a
transformation, a stance of power, flight.

- Switching on adds the mode's trait and starts its `upkeep`, a drain rate
  on a pool or store.
- Switching off removes both.
- When the pool or store empties, the mode ends. This uses the pool's
  scheduled crossing (06 §4.2); nothing is checked per tick.
- A mode may claim channel slots for as long as it is on.
- A mode may carry a `look` (12 §4.4).

Switching is an action the mode supplies, with whatever windup its
definition gives it. A mode with no windup is a toggle.

### 5.5 Charging

An action that grows stronger the longer it is held is a held phase with a
tick that raises a local (03 §6.4), and an effect that reads it. Releasing
the hold delivers it.

## 6. Conditions, suppression and loss

### 6.1 Dormancy

`while` is a **reactive condition** (05 §7.5). The engine re-evaluates it
when something it reads changes. While it is false the ability is dormant:
its grants are unavailable, its modes end, its `always` traits are removed.
When it becomes true again the ability returns.

Traits take `while` in the same way (06 §5).

Fire that does not work under water, strength that needs sunlight, a gift
that fails in the presence of iron and a familiar that must be within a
hundred paces are all `while` conditions.

### 6.2 Suppression

```json
{ "suppress_abilities": { "who": "target", "tags": ["elemental"], "for": "30 s" } }
```

Suppression is imposed from outside: by an effect, by a trait on the owner,
or by an area. An area kind (02 §8.3) may list tags it suppresses in
everyone inside.

A suppressed ability behaves as a dormant one. Anti-magic fields, power
dampeners, null collars and a rival whose gift is silencing gifts are all
suppression by tag.

The hook `ability_suppressing` lets an ability resist.

### 6.3 Loss

`revoke_ability` removes an ability whatever its sources, and it does not
come back while they remain. So does the end of its last source.
Either ends its modes and everything bound to it (section 7.3).

Abilities can be taken as well as lost: `grant_ability` to one character of
what was revoked from another, at the same rank, is theft.

## 7. Mechanisms this document adds

Each is general. None knows what a power is.

### 7.1 Reactive expressions

Specified in 05 §7.5. A pure expression that the engine re-evaluates when
something it read has changed. Used here by `while`; used by device outputs
(16 §7).

### 7.2 Areas that move, and what areas can change

Specified in 02 §8.3 as amended.

- An area may be **anchored** to an entity or a subgrid. It moves with its
  anchor. Entering and leaving are worked out as the area moves, not only
  as others do.
- An area may override **ambient** conditions inside it: gravity, as a
  scale or a vector, in addition to the clock it could already carry.
- An area kind may **suppress** ability tags (section 6.2).
- A light contribution may be negative (15 §3.3), so an area or an entity
  can darken.

An aura is an anchored area with rules on entering. A bubble of stopped
time is an anchored area with a clock. A gravity well is an area with an
ambient override. A zone of silence is an area and a rule on the hook
`signal_receiving` (15).

### 7.3 Bindings

Things an ability leaves in the world are **bound** to it.

```rust
pub enum Bound {
    Trait   { on: EntityId, trait_: TraitInstId },
    Entity  { id: EntityId },
    Edit    { at: Location, layer: Layer, before: ElementState },
    Area    { id: AreaId },
    Portal  { id: PortalId },
    Rule    { on: Handle, eoc: EocId },
}

pub struct Binding { pub what: Bound, pub ends: Option<WakeId>, pub on_end: OnEnd }
```

| Verb | Leaves |
|------|--------|
| `add_trait` with `"bound": true` | A trait on someone: a curse, a blessing, a mark |
| `summon` | An entity, created by `spawn` (06), with whatever relations are given (15 §9) |
| `conjure` | An element placed in the world, remembering what was there |
| `create_area`, `open_portal`, `attach_rule` with `"bound": true` | As named |

A binding ends when its duration runs out, when it is dismissed, when the
mode that made it ends, when the ability is suppressed or revoked, or
when its owner stops (06 §8.3). Which of these apply is said where the
binding is made:

```json
"bound": { "for": "2 min", "ends_on": ["mode", "suppressed", "owner_stops"], "on_end": "restore" }
```

`"bound": true` is short for ending on all of them and removing what was
left. On ending:

| `on_end` | Does |
|----------|------|
| `remove` | The trait is removed; the entity is destroyed; the area, portal or rule goes |
| `restore` | For an edit: the element is put back as it was, if it is still what was conjured |
| `release` | The thing stays, no longer bound: a summoned beast now free, a conjured wall now ordinary |

`dispel { where }` ends bindings by ability tag, by owner or by place.

A binding may be named when it is made (`"as": "companion"`).
`ability.bound('companion')` is then the thing bound: the summoned
creature, the area, the conjured element.

### 7.4 Snapshots

```
snapshot { of: entity | box, name, keep }      →  stores state
restore  { name }                               →  puts it back
```

- An **entity snapshot** copies its components: place, body, wounds, pools,
  traits, holdings. Restoring puts them back, moving the entity if needed.
- A **box snapshot** copies a bounded volume of cells: elements, damage,
  conduits, fields. Restoring writes them back through `WorldEdit`, so
  every change is an ordinary change with events and a cause. Occupants are
  included only if asked for.
- Snapshots are owned by an ability instance, counted, capped in size
  (`rules.max_snapshot_cells`), expire after `keep`, and are saved.

Snapshots are what recall, undoing an injury, mending what was broken and
turning back a room are made of. They are local. Turning back the whole
world is a different, much costlier and experimental mechanism (13 §9.5).

A snapshot must be taken before it can be restored. Two further verbs go
back without one.

**Undoing the last moments.** An area with a **journal** (02 §8.3) keeps
every cell change made inside it for a stated span, with the state before.

```
rewind_area { area, by, with_occupants, with_minds }
```

undoes those changes, newest first, back to the moment `by` ago, through
`WorldEdit`. Fields are restored from copies the journal takes at an
interval (`rules.journal_copy_interval`, default one second).
Collapses and releases that began inside the area within that span are
ended. With `with_occupants`, characters and objects inside are put back
from snapshots the journal took of them at the same interval.

By default minds are not touched: whoever watched a wall fall and stand up
again remembers both. With `with_minds`, the characters inside the area are
also given back the beliefs, memories and relations they had at that
moment, so they do not remember the span that was undone.

That is as far as it reaches. Anyone who watched from outside still
remembers. So does anyone who was told, any group whose knowledge was added
to, and the place's own local knowledge (09 §12). An area rewind can make
those inside forget. It cannot make the world forget.

A journal costs memory in proportion to what changes inside it and nothing
when nothing does. `rules.max_journal_changes` bounds it; past the bound
the oldest are dropped and the span that can be undone shortens. An
anchored area with a journal is "the last ten seconds, wherever I am".

**Restoring a place as it was made.** Generation is pure (10 §6.1), so the
cells a place was created with can always be computed again.

```
restore_as_generated { box }
```

realises the tracts concerned afresh, to one side, and writes the cells of
the box from them through `WorldEdit`. It needs no snapshot and costs what
generation costs. It restores what generation made, not what anyone built
there since: a repaired street, and a cleared building site.

### 7.5 Senses at a distance, and knowing the truth

Specified in 15 as amended.

- `relay_senses { from, to }` delivers the percepts of one entity's senses
  to another mind. A scrying eye, a familiar's sight, a bugged room, a
  possessed bird.
- `appraise { who, about, show }` records true values of listed properties
  of something in `who`'s track of it. Screens (12 §10.4) may show
  appraised values. This is how an ability that reveals what a thing really
  is reaches the player without breaking rule R11: the character now knows.

### 7.6 Moving fields

`move_field { field, from, to, amount }` transfers field value between
cells or regions of cells, within what is there. Water drawn up into a
wave, smoke pulled aside, fire thrown.

### 7.7 Force without a thing

An attack's `with` (07 §3.1) may be an inline surface instead of an item or
a part, and its `reach` may be long. A blow of force from across a room is
a strike like any other: aimed, defended, clashing, previewed.

`seize` (07 §9.1) takes the same. A grip made at range, by will, holds,
drags, lifts and throws as a hand does.

### 7.8 Tilting chance

The hook `contest_resolving` runs when a contest (07 §4.1) is drawn and may
shift its margin. Luck, fate, a hex and a guardian spirit are rules on it.

### 7.9 Hooks on use

| Hook | Runs | Rules may |
|------|------|-----------|
| `ability_using` | Before a granted action starts | Change its cost and parameters for this use; cancel it |
| `ability_suppressing` | Before suppression takes hold | Resist |
| `binding_ending` | Before a binding ends | Prolong it; change how it ends |

Altering a working as it is cast, at a price, is a rule on the first.

## 8. Catalogue

How each family of power is built. Every entry uses only mechanisms that
exist. Section references are where the mechanism is specified.

### 8.1 Force

| Power | Built from |
|-------|-----------|
| Great strength | Attributes that feed attack `energy` (07 §3). Throwing and lifting conditions on mass. |
| Great durability | Tissue materials with high thresholds and toughness (06 §6.1); or a rule on `hit_incoming` |
| Energy bolt | `shoot` (07 §11) with a projectile whose impact rule calls `release` (02 §13) |
| Bolts that seek, weapons that return | A projectile with guidance (07 §11.1) |
| Beam | An instant shot, or `release` shaped as a beam; sustained by a ticking phase |
| Explosion, shockwave | `release` as a sphere or cone |
| Blow of force at range | Section 7.7 |
| Charging up | Section 5.5 |
| Absorbing and returning energy | A rule on `hit_incoming` that cancels the hit and adds its energy to a pool; a release that spends the pool |
| Holding back | `effort` (07 §3.3) |

### 8.2 Movement

| Power | Built from |
|-------|-----------|
| Flight, hovering | A movement mode (06 §11) granted by a mode's trait |
| Speed | A personal clock rate (03 §3.5); or tempo |
| A dash through things | `sweep` (06 §11) |
| Great leaps | `leap` |
| Movement planned ahead and then carried out | A committed action with a route (section 9.5) |
| Wall-walking, burrowing, swimming | Movement modes over other surfaces |
| Passing through matter | A movement mode over `solid`; or another plane (02 §8.2) |
| Teleporting | `teleport`. Where to is a location the character knows (15 §7). |
| Gates | `open_portal` (02 §19), bound |
| A private space | `create_realm` (02 §21) and a portal to it |
| Size | `scale_body` (06 §6.5) |
| Reach | Modifiers on `self.reach`; the hook `interaction_reach` (16 §3.3) |

### 8.3 Time

| Power | Built from |
|-------|-----------|
| Stopping time around oneself | An anchored area with a clock at rate 0 (section 7.2); the user anchored to the realm clock (03 §3.4); upkeep |
| Stopping time somewhere | The same, not anchored |
| Slowing, hastening | Area clocks at other rates; `set_time_rate` on a target |
| Freezing someone | `set_time_rate` to 0 on them |
| Ageing, decay, ripening | A high clock rate on the target, so its own timers and pools run ahead |
| Reflexes | Notice delay (03 §7.2) |
| Recall, undoing | Snapshots (section 7.4) |
| Undoing the last moments in a place | An area with a journal, and `rewind_area` (section 7.4). Those inside may be made to forget. |
| Turning back the world | Experimental. `rewind_world` (13 §9.5): everything returns to an earlier moment, and only those chosen remember |
| Sensing danger | A window kind's `noticed_if` and `notice` reading a trait |

### 8.4 Matter and elements

| Power | Built from |
|-------|-----------|
| Fire, frost, lightning, acid, gas | Fields (02 §16) placed with `add_field`; releases of those damage types. What they do to each other is field reactions (02 §16). |
| Freezing, melting, boiling | Fields that warm or cool; materials and fields change state by temperature (02 §5.4) |
| Moving water, air, smoke | `move_field` (section 7.6) |
| Wind, whirlwinds, whirlpools | An area whose ambient override sets a drift (11 §4.7) |
| Shaking the ground | `tremor` (02 §14.6) |
| A great wave, an avalanche, a wall of force | A `release` shaped as a front (02 §13.5) |
| Raising walls, pillars, bridges | `conjure` (section 7.3); or a map fragment stamped in place (02 §23) |
| Moving earth and stone | `detach` a piece, move it as a subgrid, `merge` (02 §18.3) |
| Throwing things by will | A grip at range (section 7.7), then `throw`; or `impulse` (02 §15) |
| Transmutation | `set_material` on elements (16 §3.5), items and tissue |
| Disintegration | A damage type few materials resist; or removal by edit |
| Mending | `damage` reversed by rule; a box snapshot restored; or `restore_as_generated` (section 7.4) |
| Gravity | Area ambient override (section 7.2) |
| Light and darkness | Light sources, positive and negative (15 §3.3) |
| Weather | Region values the cold simulation keeps (09) |
| Growth | Recipes with a place condition (16 §8.4), hurried by a clock rate |
| Machines and power | Verbs on devices and networks (16): `add_resource`, `drain_resource`, `set_mode`, and `network` reach by trait |

### 8.5 Bodies

| Power | Built from |
|-------|-----------|
| Healing, regeneration | `heal_part`; pool rates; tissue `heal` raised by modifiers |
| Regrowing limbs | `add_part` (06 §6.5) |
| Changing shape | `set_body_plan`, `scale_body`, `set_layer`; `copy_persona` (15 §8.3) to look the part |
| Taking on a material | `set_layer` to the material of what was touched |
| Extra limbs, wings, weapons | `add_part`, each bringing channels, movement and surfaces |
| Reshaping another | The same verbs on a target (scene S17) |
| Turning to stone | `set_layer` on every part, and a clock rate of 0 |
| Copies of oneself | `summon` with the owner's character spec (06 §10.3), driven by the owner (06 §8.2) or with minds of their own |
| Draining life | A pool rate on the target and its opposite on the user |
| Not dying | A rule on `entity_dying` (06 §8.3) |
| Raising the dead | `create_mind` in a body; or `set_driver` (06 §8.2) |
| Souls | `transfer_mind` to an item or another body; the event `mind_released` |

### 8.6 Minds and perception

| Power | Built from |
|-------|-----------|
| Seeing in the dark, through walls, heat, tremors | Senses (15 §4) with other signal kinds and blocking rules |
| Seeing far away | `relay_senses` (section 7.5) from a summoned or conjured observer |
| Reading minds | Queries on another's tracks and memories (15 §7), by a verb that copies them as told |
| Speaking mind to mind | `say` (15 §11) on a signal kind only some can sense |
| Knowing what a thing is | `appraise` (section 7.5) |
| Seeing through disguise | The hook `identity_check` (15 §8.2) |
| Invisibility, silence | Signature modifiers (15 §3.2); the hook `signal_receiving` |
| Illusions | `create_illusion`, `emit_signal` (15 §8.3, §3.2) |
| Disguise | `assume_persona`, `copy_persona` |
| Planting and erasing memories | `remember` with chosen personas; `forget_memory` |
| Charm, terror, calm | `set_relation`, `add_relation` (15 §9.1); auras as anchored areas |
| Commanding | A `commands` relation (15 §10) |
| Dominating | `set_driver`; or `set_control` (03 §5.1) to make them the player's |
| Possessing | `transfer_mind` |
| Luck | The hook `contest_resolving` (section 7.8) |

### 8.7 Making and calling

| Power | Built from |
|-------|-----------|
| Summoning | `summon` (section 7.3) with relations |
| Familiars, bonded beasts | A summoned or befriended creature, a relation, relayed senses |
| Conjured weapons and armour | Items created bound; armour as worn layers (06 §9.2) |
| Animating objects | `create_mind` in an item or device; a movement mode |
| Wards | An area with rules on entering; a rule on `interaction_starting` (16 §3.4) |
| A space imposed on the world, with laws of its own | Section 9.6 |
| Curses and blessings | Bound traits; bound attached rules |
| Enchanting | Installing a module (16 §11); adding traits or abilities to an item |

### 8.8 Protection and negation

| Power | Built from |
|-------|-----------|
| A barrier that takes hits | A pool and a rule on `hit_incoming` that spends it; or an interposing defence held up (07 §5) with a conjured surface |
| Turning attacks back | A rule on `hit_incoming` or `strike_resolving` that cancels and strikes |
| A shielded area | An area with a rule on hits to those inside; or conjured elements |
| Nullifying powers | Suppression by tag (section 6.2) |
| Dispelling | `dispel` (section 7.3) |
| Immunities | Thresholds; rules on the relevant hook |

### 8.9 Powers over powers

| Power | Built from |
|-------|-----------|
| Copying | `grant_ability` of what the target has, by a query on its abilities |
| Stealing | The same, with `revoke_ability` |
| Bestowing | `grant_ability` on another, bound or not |
| Amplifying, weakening | Modifiers on ability parameters (section 4.2) |
| Altering a working as it is used | The hook `ability_using` |
| An absurd gift with a status screen | A template (06 §10) granting abilities with high parameters, and an interface part (12 §10.6) |

## 9. Worked examples

### 9.1 Stopping time

```json
[
  { "type": "ability", "id": "stillness", "name": "Stillness",
    "seat": "mind", "tags": ["time"],
    "modes": [ { "id": "stopped", "trait": "outside_time",
                 "upkeep": { "resolve": 12.0 },
                 "on_start": [
                   { "create_area": { "kind": "stopped_time", "anchor": "self",
                                      "shape": { "radius": "40 * ability.rank" }, "bound": true } },
                   { "anchor_clock": { "who": "self", "to": "self.loc.realm" } } ],
                 "on_end": [ { "anchor_clock": { "who": "self", "to": null } } ] } ] },

  { "type": "area_kind", "id": "stopped_time", "clock_rate": 0, "priority": 100 }
]
```

Switching the mode on makes an area around the user whose clock does not
run, and pins the user's own clock to the realm's. Everything else inside
hangs: people mid-step, bodies in flight, falling masonry, fire. The user
walks among them. `resolve` drains at twelve a second of the user's time.
When it is gone the mode ends, the binding removes the area, and everything
resumes from where it was.

What the user does meanwhile is ordinary. A knife thrown inside the area
is a body on a stopped clock: it leaves the hand and hangs. A push is an
impulse delivered when time returns. Scene S10 needs nothing else.

### 9.2 A reshaping touch

```json
{ "type": "action", "id": "reshape", "name": "Reshape",
  "uses": { "arm": 1, "focus": 1 },
  "params": { "target": { "type": "entity" }, "form": { "type": "choice", "from": "ability.var.forms" } },
  "attacks": { "touch": { "with": { "part": "hand", "surface": "palm" }, "reach": 1.2, "energy": 0,
                           "aim": { "contest": "melee_aim", "by": "touch_skill(self)", "against": "evasion(target, self)" } } },
  "phases": [
    { "id": "reach", "time": "250 ms", "window": { "kind": "strike", "for": ["$target"], "closes": "touch" } },
    { "id": "touch", "time": 0,
      "do": [ { "strike": { "attack": "touch", "target": "$target" } },
              { "if": "$landed", "then": [ { "run": { "eoc": "apply_form", "with": { "who": "$target", "form": "$form" } } } ] } ] },
    { "id": "recover", "time": "300 ms" } ] }
```

The touch is an attack with no energy, so it can be dodged, blocked and
seen coming. If it lands, `apply_form` runs body verbs on the target:
`remove_part`, `add_part`, `set_layer`, `scale_part`. The target is then
whatever those leave, with the functions, channels and wounds that follow
(06 §6).

### 9.3 A style of the sword

```json
{ "type": "ability", "id": "river_style", "name": "River Style",
  "seat": "mind", "tags": ["style", "blade"],
  "track": "river_style",
  "params": { "control": "8 + 2 * ability.rank" },
  "always": { "traits": ["river_footwork"] },
  "grants": [
    { "action": "river_cut" },
    { "action": "returning_water", "at_rank": 2 },
    { "action": "one_stroke", "at_rank": 6, "cost": { "breath": 30 } } ],
  "while": "self.held_with_tag('blade') != null" }
```

Nothing here is supernatural and nothing about the definition differs from
section 9.1. It is dormant without a blade in hand. `one_stroke` is an
attack with an arc and `through` (07 §6.4): scene S4, bought with thirty
breath and six ranks of practice.

### 9.4 A barrier

```json
{ "type": "trait", "id": "aegis_up",
  "rules": [
    { "on": "hit_incoming", "as": "target", "priority": 50,
      "if": "self.pool.aegis > 0",
      "do": [
        { "let": { "take": "min(event.energy, self.pool.aegis * 1000)" } },
        { "pool_add": { "who": "self", "pool": "aegis", "amount": "-$take / 1000" } },
        { "modify": { "event.energy": "event.energy - $take" } },
        { "if": "event.energy <= 0", "then": [ { "cancel_event": {} } ] } ] } ] }
```

Each point of `aegis` stops a kilojoule. A hit larger than what is left
comes through at the difference. There is no rule that a barrier holds
until it breaks: it holds what it has the energy to hold.

### 9.5 A second planned in advance

```json
{ "type": "action", "id": "planned_second", "name": "Planned second",
  "uses": { "legs": "all", "focus": 1 },
  "params": { "route": { "type": "list", "of": "cell", "max": 24,
                          "check": "route_fault(self, $route)" } },
  "phases": [
    { "id": "commit", "time": 0,
      "do": [ { "set": { "$i": 0 } },
              { "if": "route_fault(self, $route) != null",
                "then": [ { "set_time_rate": { "who": "self", "rate": 0 } },
                          { "after": "1 s", "owner": "self.loc.realm",
                            "do": [ { "set_time_rate": { "who": "self", "rate": 1 } } ] },
                          { "cancel_action": {} } ] } ] },
    { "id": "run", "time": "1 s", "fixed": true,
      "tick": { "every": "41.667 ms",
                "do": [ { "sweep": { "who": "self", "to": "$route[$i]" } },
                        { "set": { "$i": "$i + 1" } } ] } } ],
  "on_finish":    [ { "add_trait": { "to": "self", "trait": "momentum" } } ],
  "on_interrupt": [ { "remove_trait": { "from": "self", "trait": "momentum" } } ] }
```

The player draws up to twenty-four steps with the route picker, which
shows as each is added whether it can be done. Then the action runs them,
one every twenty-fourth of a second. No phase allows `cancel`, so once
begun it cannot be changed: the user goes where they said they would, and
an opponent who reads the route can be waiting.

`route_fault` is a function of the mod's own (05 §12). Here it would
reject a step longer than the user's `momentum` allows, or a turn too
sharp for the speed. A plan that breaks its rule stops the user's own
clock for a second. Someone whose clock is stopped notices nothing and
answers nothing (03 §7.2), so nothing more needs to be written for them to
be helpless.

Each step is a `sweep` (06 §11), so the user arrives with their real mass
and speed and strikes whatever is in the way. `momentum` is a stacking
trait that lets each plan go further than the last, and an interruption
takes it away.

The same rule can be laid on someone else: a trait whose rule on the hook
`ActionStarting` (03 §13) lets an action begin only if its owner wins a
contest, and stops their clock if not.

### 9.6 An imposed space

```json
[
  { "type": "ability", "id": "walled_court", "name": "Walled Court",
    "seat": "mind", "tags": ["imposed_space"],
    "params": { "radius": "8 + 2 * ability.rank" },
    "modes": [ { "id": "open", "trait": "holding_court",
                 "upkeep": { "resolve": 6.0 },
                 "on_start": [
                   { "create_area": { "kind": "walled_court", "at": "self.loc",
                                      "shape": { "radius": "ability.radius" },
                                      "journal": "5 min", "vars": { "owner": "self" },
                                      "bound": true, "as": "court" } },
                   { "apply_map": { "map": "court_interior", "at": "self.loc", "mode": "replace" } } ],
                 "on_end": [
                   { "suppress_abilities": { "who": "self", "tags": ["imposed_space"], "for": "30 s" } } ] } ] },

  { "type": "area_kind", "id": "walled_court", "tags": ["imposed_space"],
    "ambience": "court_light",
    "rules": [
      { "every": "1 s",
        "do": [ { "for_each": { "entities": { "in_area": "self",
                                              "where": "it != self.var.owner and it.pool.resolve > 0" } },
                  "as": "x",
                  "do": [ { "hit": { "target": "$x", "kind": "cut", "energy": 400, "intensity": "2 MPa" } } ] } ] },
      { "on": "binding_ending",
        "do": [ { "rewind_area": { "area": "self", "by": "5 min" } } ] } ] }
]
```

| Part | Built from |
|------|-----------|
| The space | An area bound to the ability, with a journal for as long as it may last |
| What it is like inside | A map applied over it: floor, furniture, and the shell that closes it. An ambience (12 §6.4). |
| What it does to those inside | Rules attached to the area kind (05 §9.3): on entering, on a schedule, or on what those inside do. A rule may deliver a `hit` directly (06 §7.1), which is not aimed and which no defence answers. It may leave out whoever it likes, by any condition: here, anyone with no `resolve` at all. |
| Getting out | The shell is elements, as strong as the map makes them, from either side |
| Two meeting | A rule run when one is made looks for others it overlaps and settles it by a `contest` (07 §4.1) between the two abilities. The loser's binding is ended with `dispel`, or both stand, as content chooses. |
| When it ends | `rewind_area` puts back the cells that were there before, without moving anyone. Suppressing the ability for a time afterwards is the cost of having used it. |
| A small one against a large one | A second ability whose anchored area suppresses the first's tag within arm's reach (section 6.2) |

When the inside should be larger than the outside, or have its own
gravity or clock, make it somewhere else: `create_realm` from an authored
map (10 §9.3), transfer everyone inside the area to it by a fixed offset
(02 §19), leave the shell standing, and at the end transfer them back by
the same offset and `destroy_realm`.

## 10. Power and its price

The engine imposes costs that content does not have to write, and offers
none of the protections content might assume.

| Fact | From |
|------|------|
| A blow the body cannot survive delivering harms the one who delivers it | Recoil (07 §6.3) |
| Energy released near oneself reaches oneself | Releases have no owner to spare (02 §13) |
| Everything broken has a cause recorded | Causes (02 §7.3). Collateral is attributable. |
| Witnesses remember | Memories and personas (15) |
| A working that needs attention ends when attention is broken | Channels (section 5.3) |
| Nothing scales to the opposition | Decision D-034 |

Durability, control and immunity to one's own power are things content
gives an ability on purpose: tougher tissue, a `hit_incoming` rule that
exempts the user, a restraint habit through `effort`. An ability without
them is dangerous to hold, which is a story a mod is free to tell.

## 11. Use by rules, the AI layer and screens

An ability carries what others need to reason about it without knowing
what it is:

| Field | For |
|-------|-----|
| `tags` | Suppression, counters, filters |
| `intent` on grants | A hint to the AI layer (09): `harm`, `protect`, `escape`, `control`, `aid`, `utility` |
| `value` on grants | An optional pure function scoring a use in a situation, for abilities that previews (07 §13) cannot assess |
| `name`, `doc`, cost and cooldown | The abilities screen (12 §10.7), built from the instance |

`magnitude(x)` returns the base-ten logarithm of a quantity. Content uses
it to name tiers of power for display and for characters deciding whether
they are outmatched.

## 12. Modding surface

**Definition type**: `ability`. Fields `abilities` on templates, traits,
items and modules; `while` and `bound` on traits; `cost`, `cooldown`,
`at_rank`, `if`, `intent`, `value` on grants; `suppresses`, `ambient`,
`clock_rate`, `priority` on area kinds; inline surfaces in `with`.

**Events**: `ability_granted`, `ability_revoked`, `ability_dormant`,
`ability_active`, `ability_suppressed`, `ability_restored`, `mode_started`,
`mode_ended`, `rank_changed`, `binding_created`, `binding_ended`,
`snapshot_taken`, `snapshot_restored`, `area_moved`.

**Hooks**: `ability_using`, `ability_granting`, `ability_revoking`,
`ability_suppressing`, `binding_ending`, `contest_resolving`.

| Hook | Runs before | A handler may |
|------|-------------|---------------|
| `ability_using` | A granted action starts | Change its cost and parameters; cancel |
| `ability_granting`, `ability_revoking` | An ability is given or taken by a rule | Cancel: a gift that cannot be stolen |
| `ability_suppressing` | Suppression takes hold | Cancel: resist it |
| `binding_ending` | Something bound ends | Change how it ends; cancel |
| `contest_resolving` | A contest is decided (07 §4.1) | Change either side's value |

`restore_abilities` lifts a suppression early. `move_area` moves an
unanchored area.

**Refusals.** These are refused with a reason: `restore` of a snapshot
that has expired; a snapshot or journal past its limit; a binding past
`rules.max_bindings`; `restore_as_generated` over more than
`rules.max_restore_cells`.

**Effects**: `grant_ability`, `revoke_ability`, `set_rank`,
`suppress_abilities`, `restore_abilities`, `start_mode`, `end_mode`,
`summon`, `conjure`, `dispel`, `dismiss`, `snapshot`, `restore`,
`rewind_area`, `restore_as_generated`,
`create_area`, `move_area`, `relay_senses`, `appraise`, `move_field`.

**Queries**: `abilities` (of an entity, by tag), `bindings` (of an ability
or owner), `modes`.

**Properties**: on abilities, `rank`, each parameter by name, `owner`,
`state`, `mode(id).on`, `cooldown(action)`, `var.*`, `bound(name)`; on
entities,
`has_ability(id)`, `ability(id)`, `abilities_with(tag)`.

**Functions**: `magnitude`.

**Settings**: `rules.max_snapshot_cells`, `rules.max_journal_changes`,
`rules.max_bindings`, `rules.max_restore_cells`,
`rules.journal_copy_interval`.

### 12.1 As built

Packages A1 to A4 (decisions D-720 to D-729), in `clash_sim::ability`.
What is not listed here is not built yet: see the end of this section.

**The definition** (`ability`):

| Field | Type | Meaning |
|-------|------|---------|
| `name`, `description` | text | What a reader calls it, and what a closer look says |
| `tags` | list of text | What it is, for parameter modifiers and rules |
| `seat` | `body` or `mind` | Which it stays with when they part. Default `body` |
| `track` | id of a `track` | Its rank is the owner's level of this track |
| `rank` | int, 0 to 10,000 | With no track, the rank it is granted at unless the grant says. Default 1; a load error beside `track` |
| `params` | map of name to number expression | Its numbers, read as `ability.<name>` (section 4.2). A name is lowercase letters, digits and `_`; it may not be `rank`, `owner`, `state`, `def`, `id`, `tags`, `mode_on`, `cooldown`, `kind` or `var`; parameters that read each other in a circle are a load error |
| `always` | an object, or a list of them | What it confers while active: each `{ at_rank?, traits, modifiers, params, senses }`, from `at_rank` up. `traits` are given with the instance's entry as giver; `modifiers` (`{ attr, add, mul }`) apply under the entry as a stats source; `params` are parameter modifiers (below); `senses` are carried for perception |
| `grants` | list | Actions it makes available (below) |
| `modes` | list | Modes (below), at most 255 |
| `while` | condition | Kept current (05 §7.5); while false the ability is dormant |
| `rules` | rules | Attached to the owner while the ability is active, with the owner as `self` |

What an ability works out about itself (`params`, `while`, a grant's
`params`, `cost` and `cooldown`, a mode's `upkeep` and `cost`) binds the
owner as `self` and the instance as `ability`, and reads truth; all but
effects are pure. A grant's `if` and `value` are in the owner's mind
(05 §7.3): its own state, its own ability, and what is safe for belief.
A mode's `on_start` and `on_end` are effects with the same two subjects.

A **grant** is an action's id, or:

| Field | Type | Meaning |
|-------|------|---------|
| `action` | id of an `action` | What it starts. One action is granted at most once by an ability |
| `params` | map of name to expression | Parameters of the action the ability supplies, standing in for any of the same name a command gives: how an action reads the ability's numbers (`"energy": "ability.power"`) |
| `cost` | map | Below |
| `cooldown` | number expression, seconds | Of the owner's own time (03 §3.4) before the grant can be used again |
| `at_rank` | int | The least rank |
| `if` | condition | A further condition, in the owner's mind |
| `intent` | `harm`, `protect`, `escape`, `control`, `aid` or `utility` | A hint to whoever chooses for a character |
| `value` | number expression | A score of a use, for the AI layer. Carried |

A **cost** is keyed by the pool it is paid from, or by an item's id with
`"item": true` (a qualified id, or a bare one looked up in sorted
namespace order), and is an amount (a number, or an expression), or
`{ amount, overdraw?: { from: pool, ratio: number (default 1) }, from?: expression, item?: bool, count?: int (default 1) }`.
`from` gives the entity that pays in the owner's place, such as the
companion an ability is bound to (`ability.bound('companion')` once
bindings are built; any expression that gives an entity meanwhile).
Overdraw always comes from the owner's pool. A cost naming a pool or an
item that is not defined is a load error.

A **mode**:

| Field | Type | Meaning |
|-------|------|---------|
| `id` | name | Its name within the ability |
| `name` | text | What a reader calls it |
| `at_rank` | int | The least rank it can be on at |
| `trait` | id of a `trait` | Given while it is on, with the mode as giver |
| `upkeep` | map of pool to number expression | Drained per second of the owner's time while it is on, worked out when it is switched on |
| `cost` | map | What switching it on spends, as a grant's |
| `uses` | map of channel to count | Carried: a mode holds no slots yet; its trait may disable some with `channels` meanwhile |
| `on_start`, `on_end` | effects | Run as it comes on, and as it ends however it ends |
| `look` | text | Carried for presentation |

Traits list `abilities` and `ability_params` (06 §5), and templates and
their options `abilities` (06 §10.4).

**Instances.** The component `Abilities`, the seventeenth (06 §3.1),
holds an entity's instances in the order they were made: each with its
number, its definition, its sources (`template`, `trait` with the trait
instance's key, `rule`), whether it was revoked, its rank set directly,
its state (`active` or `dormant`), the modes that are on with the
wake-up of each, its cooldowns as the owner's own time at which each
grant is ready, and the entries of `always` it applies now. A number is
given once in the simulation's life and never again: the simulation
keeps the next and which entity holds each (`AbilityIndex`, saved and
hashed with the kernel's state; save layout 11). A handle of the kind
`ability` is that number, so it never names another instance.

**Holding** (sections 3.3, 3.4, 6.3):

- A template's abilities are given once the entity is made, with the
  template as source. A trait instance that is not dormant is a source
  of each ability its definition lists, and stops being one when it
  ends or goes dormant. `grant_ability` makes a rule a source. One
  ability from several sources is one instance (invariant A8).
- An instance with no source left ends: its modes end (`lost`), what it
  conferred is taken back, and `ability_revoked` is raised (`how`
  `lost`).
- `revoke_ability` ends it whatever its sources (`how` `revoked`). One
  that traits still give stays, revoked: it is not held and confers
  nothing until a rule grants it again; another trait listing it does
  not bring it back.
- When a mind moves (`mind_transferred`), the mind-seated abilities the
  entity held only from rules and templates go with it, keeping their
  number, rank and cooldowns (`ability_revoked` `moved` at the old
  body, `ability_granted` at the new); for a swap, both ways. What a body
  has through its traits stays with it.

**Rank and parameters.** A track's level is read as it stands; each
`track_advanced` of that track raises `rank_changed` for the abilities
it ranks. When the rank changes, what applies from a rank in `always` is
brought into line and modes above it end (`rank`). A parameter is read
fresh each time: its expression, then `(value + Σ add) × Π mul` over the
`ability_params` of the owner's traits that confer (once for each stack)
and the `params` of the entries of `always` its abilities apply, each
naming this ability or one of its tags.

**Using** (`use_ability`, section 5): the instance must be held and
active, the grant's `at_rank` met, its cooldown over, its `if` true and
every cost payable: refused otherwise, changing nothing. The grant's
`params` are worked out and stand in for the command's. If anything
listens, `ability_using` is raised. The action is then prepared by the
action runtime (03 §6.6), every cost is taken in full (invariant A4),
the cooldown starts, the action starts and `ability_used` is raised. A
refusal after the hook ran is raised as `ability_refused`, and a start a
handler of `action_starting` refuses gives the pools back. A granted
action can still be started with `Act` like any other action, without
its ability; whoever offers actions offers granted ones through their
abilities (the bundled ones are tagged `power`).

**Modes.** Switching on takes the mode's cost, gives its trait, applies
each pool of its upkeep as a rate of minus the amount under the mode's
own stats source, runs `on_start` and raises `mode_started`. The moment
the first such pool comes to nothing is worked out from its level and
net rate and scheduled once on the owner's clock (a `Timer` wake-up with
bit 60 of `arg` set); it is worked out again when the owner's pools are
heard to move (`pool_crossed`, `attr_changed`, traits added or removed,
a cost paid, the owner moving into the world). At that moment the mode
ends with `how` `empty`. Switching is a command or a verb: a toggle with
no windup. A mode that should take time is a grant whose action calls
`start_mode`.

**Charging** is the action runtime's held phase (section 5.5): a `tick`
raises a variable, and `Cancel` (a command) or `cancel_action` (an
effect) lets the hold go, which counts as release.

**Suppression** (section 6.2; package A5, decision D-770). Three things
suppress an instance, each by the tags of the abilities it names (an
ability is reached when it has any of them):

| Suppressor | Lasts | Made by |
|------------|-------|---------|
| `effect` | Until its `for` runs out on the owner's own time, or `restore_abilities` lifts it | `suppress_abilities`; with no `tags`, every ability |
| `trait` | While the owner has a trait instance whose definition lists `suppresses`, and it is not dormant | Giving the trait (06 §5) |
| `area` | While the owner is inside an area whose kind lists `suppresses` (02 §8.3) | The owner moving in, or the area being made, moved or reshaped over it |

Before a suppression takes hold of an instance, the hook
`ability_suppressing` runs once for that instance and that suppressor;
a handler that cancels makes it **resist**, and that suppressor passes
it over for as long as it lasts. A suppressed instance confers nothing
(invariant A2): its modes end (`how` `suppressed`), what it conferred is
taken back, what is bound to end on suppression ends (section 7.3), and
its grants and modes are refused with `ability.suppressed`. When its
last suppressor lifts (`ability_restored`) it confers again with its
rank, cooldowns and bindings that did not end as they were; its modes
stay off. An instance gained while a suppression lasts is suppressed
from the start, before it confers anything (`ability_suppressed`
follows `ability_granted`). Its `state` reads `suppressed` while it is
active and suppressed. What suppresses each instance and what it
resisted are kept in it, and the imposed suppressions in the owner's
`Abilities` (save layout 16), so nothing is derived.

Dormancy (section 6.1) is as A1 to A4 built it: an ability's `while` is
kept current, and a trait's where everything it reads names its events.
The rounds of re-evaluation are counted afresh by each pump.

**Areas that move, and what areas change** (section 7.2; package A6,
decision D-771): built in the world (02 §8.3 as built). An area
anchored to an entity follows it as `relocate` moves it, and whoever it
comes over or leaves is told exactly as when they move the other way;
its maker is inside it throughout and is told nothing. An area with a
clock rehomes whoever and whatever it comes over: a maker who anchors
its own clock to the realm's (`anchor_clock`) walks on while everything
else inside hangs. An area or its kind may set gravity (a scale, or a
vector), which bodies in flight read where they are at each wake, and
may carry a light at its middle, negative for darkness, which moves
with it; light is never below zero. `create_area` takes `anchor`,
`gravity`, `light` and `light_radius`.

**Bindings** (section 7.3; package A7, decision D-772). What an
ability leaves in the world is kept by the simulation as a binding with
a number given once in its life: the instance it belongs to and its
owner, what it holds, its name, the mode whose `on_start` made it, when
it ends and what then happens.

| Holds (`what`) | Made by | `remove` | `restore` | `release` |
|----------------|---------|----------|-----------|-----------|
| `trait`: a trait on someone, given with the binding as its giver | `bind_trait` | Taken away | As `remove` | Stays, given by no one who takes it back |
| `entity`: one made from a template | `summon` | Removed from the simulation | As `remove` | Stays, free |
| `edit`: elements placed in cells of one layer, with what was there | `conjure` | Taken away where the cell is still what was conjured | What was there put back where the cell is still what was conjured | Left |
| `area` | `create_area` with `bound` or any term | Removed | As `remove` | Stays |
| `rule`: a rule attached to someone | `bind_rule` | Detached | As `remove` | Stays attached |

The terms are arguments of each verb that binds, beside its own (not
the object `bound` of section 7.3, which an argument written as an
object cannot be: 05 §6 reads it as a value source):

| Argument | Type | Meaning |
|----------|------|---------|
| `ability` | `ability?` | The instance it is bound to. Left out, the `ability` bound where the effect is written: a mode's `on_start` and `on_end`, what an ability works out about itself. An action is handed its ability by its grant (`"params": { "by": "ability" }`, a parameter of type `ability`) |
| `for` | `real?` | Seconds of the owner's own time it lasts; it stands still while the owner is out of the world |
| `ends_on` | `list<string>?` | Of `mode` (the mode that made it ends), `suppressed`, `dormant` (its ability is), `owner_stops` (its owner dies). Left out, all four |
| `on_end` | `string?` | `remove`, `restore` or `release`. Left out, `restore` for `conjure`, `remove` for the rest |
| `as` | `string?` | Its name, for `ability.bound(name)` |

A binding also always ends, whatever `ends_on` says, when its ability
is revoked (`revoked`) or loses its last source (`lost`), moves with a
mind to an entity that already holds it (`moved`; otherwise it moves
with the instance), or its owner is removed from the simulation
(`owner_removed`): invariant A3. A binding whose thing is removed by
something else is forgotten. Before a binding ends the hook
`binding_ending` runs; a handler may change `on_end`, set `prolong` to
keep it that many more seconds, or cancel to keep it (one whose time ran
out then lasts until something else ends it); when its ability is gone
neither keeps it. Each ability holds at most `rules.max_bindings`
bindings (default 64).

`summon { template, at, mind?, obey?, … }` makes the entity through the
template service (06 §10.3), thinking by `mind` if given. With `obey`
(default true) it takes the stances its summoner's mind holds toward
others (09 §3.4), holds none toward its summoner, and is given a track
of its summoner as if told (15 §7); it is given the senses its mind
names, and thinks at once. `conjure { at, to?, layer?, what, … }`
places what as data writes a state in the cell, or in each cell of the
straight line from `at` to `to` (at most 64), through `WorldEdit`,
refusing before anything changes if a cell cannot be read.
`dismiss { who, ability, as?, how? }` ends an owner's bindings of an
ability, or the one of a name, `how` in place of each `on_end`.
`dispel { tags?, owner?, at?, radius? }` ends the bindings whose
ability has any of the tags, whose owner is `owner` and whose thing is
within `radius` metres of `at`. Portals are not built (02 §19): nothing
is bound to one.

**Loss** (section 6.3). Revoking an instance, or losing its last
source, ends its modes (`revoked`, `lost`) and everything bound to it
(section 7.3, invariant A3), then takes back what it conferred.

**Events**, each with the owner as `self`:

| Event | Fields |
|-------|--------|
| `ability_granted` | `ability` (string, qualified id), `source` (string: `template`, `trait`, `rule`), `rank` (int) |
| `ability_revoked` | `ability`, `how` (string: `revoked`, `lost`, `moved`) |
| `ability_dormant`, `ability_active` | `ability` |
| `rank_changed` | `ability`, `old`, `new` (int) |
| `ability_suppressed` | `ability`, `by` (string: `effect`, `trait`, `area`, what took hold first) |
| `ability_restored` | `ability` |
| `binding_created` | `ability`, `name` (string, empty for none), `what` (string: `trait`, `entity`, `edit`, `area`, `rule`), `on_end` (string) |
| `binding_ended` | `ability`, `name`, `what`, `why` (string: `duration`, `dismissed`, `dispelled`, `mode`, `suppressed`, `dormant`, `owner_stops`, `revoked`, `lost`, `moved`, `owner_removed`), `how` (string: the `on_end` carried out), `thing` (any: the entity, the area, the one a trait or rule was on, or the first conjured cell) |
| `mode_started` | `ability`, `mode` (string) |
| `mode_ended` | `ability`, `mode`, `how` (string: `ended`, `empty`, `dormant`, `suppressed`, `revoked`, `lost`, `rank`, `moved`) |
| `ability_used` | `ability`, `action` (string), `cost` (map of each pool's or item's qualified id to what was spent, overdraw included) |
| `ability_refused` | `ability`, `action`, `reason` (string, the code), `message` (string) |

**Hooks**, each with the owner as `self`:

| Hook | Fields | A handler may |
|------|--------|---------------|
| `ability_using` | `ability`, `action` (string); `params` (map of name to value); `cost` (map of qualified id to amount) | Change `params` (each held to its type again) and `cost` (an amount of zero or more), for this use; cancel |
| `ability_granting` | `ability`, `source` (`rule`) | Cancel. Raised by `grant_ability` only |
| `ability_revoking` | `ability` | Cancel. Raised by `revoke_ability` only, for an ability held |
| `ability_suppressing` | `ability`; `by` (string: `effect`, `trait`, `area`); `source` (any: the area for `area`, the trait's qualified id for `trait`, null for `effect`) | Cancel: the ability resists that suppression while it lasts |
| `binding_ending` | `ability`, `name`, `what`, `why` (as `binding_ended`); `on_end` (string); `prolong` (real, 0) | Change `on_end`; set `prolong` above zero to keep it that many seconds more; cancel to keep it. Neither keeps one whose ability is gone |

**Verbs:**

| Verb | Arguments | Returns |
|------|-----------|---------|
| `grant_ability` | `to: entity`, `ability: id<ability>`, `rank: int?` | `ability`, the instance |
| `revoke_ability` | `from: entity`, `ability: id<ability>` | `bool`, whether it was held |
| `set_rank` | `who: entity`, `ability: id<ability>`, `rank: int` | `int`, the rank before |
| `use_ability` | `who: entity`, `ability: id<ability>`, `action: id<action>?`, `params: any?` (a map) | `bool`, whether it started; when not, `ability_refused` says why |
| `start_mode` | `who: entity`, `ability: id<ability>`, `mode: string` | `bool`, true |
| `end_mode` | `who: entity`, `ability: id<ability>`, `mode: string` | `bool`, true |
| `suppress_abilities` | `who: entity`, `tags: list<string>?` (left out: every ability), `for: real?` (seconds of `who`'s own time; left out: until lifted) | `int`, how many of its abilities it suppresses now |
| `restore_abilities` | `who: entity`, `tags: list<string>?` | `int`, how many imposed suppressions were lifted: those naming any of `tags`, or naming none, or all with no `tags` |
| `summon` | `template: id<template>`, `at: loc`, `mind: string?`, `obey: bool?`, and the terms | `entity`, the one made |
| `conjure` | `at: loc`, `to: loc?`, `layer: string?` (default `block`), `what: string`, and the terms | `int`, how many cells |
| `bind_trait` | `to: entity`, `trait: id<trait>`, and the terms | `int`, the stacks it has |
| `bind_rule` | `to: entity`, `eoc: id<eoc>` (a rule with `on`), and the terms | `bool`, whether it was attached; one attached already binds nothing |
| `dismiss` | `who: entity`, `ability: id<ability>`, `as: string?`, `how: string?` | `int`, how many ended |
| `dispel` | `tags: list<string>?`, `owner: entity?`, `at: loc?`, `radius: real?` (default 0) | `int`, how many ended |
| `create_area` | as 02 §23 lists it, with `bound: bool?` and the terms | `area` |

**Commands** (03 §4.4): `UseAbility { entity, ability, action, params }`
and `SetMode { entity, ability, mode, on }`.

**Properties.** On an entity: `has_ability(ability: id<ability>)`
(`bool`); `ability(ability: id<ability>)` (`ability?`, the instance, the
owner's own); `abilities` (`list<id<ability>>`, in the order gained);
`abilities_with(tag: string)` (`list<ability>`); `bound_by`
(`entity?`: the owner of the ability a summoned entity is bound to,
readable of oneself in a mind; it moves with bindings and cannot be kept
current); `cannot_use(ability:
id<ability>, action: id<action>?)` (`string?`, the code of the reason a
use would be refused now, null if it would not). The first four change
with `ability_granted` and `ability_revoked`. On the kind `ability`:
`rank` (`int`, changed by `rank_changed`), `owner` (`entity?`),
`state` (`string?`: `active`, `dormant` or `suppressed`; by
`ability_dormant`, `ability_active`, `ability_suppressed` and
`ability_restored`), `def`
(`id<ability>?`), `id` (`string?`), `tags` (`list<string>?`),
`mode_on(mode: string)` (`bool`, by `mode_started` and `mode_ended`),
`bound(name: string)` (`any`: what its binding of that name holds, as
`binding_ended`'s `thing` says, and for an `edit` every cell as a list
of `loc`; by `binding_created` and `binding_ended`),
`cooldown(action: id<action>)` (`real`, seconds left, 0 when ready), and
each parameter by name (`real`). None is safe for belief or settable.
`cooldown`, `cannot_use` and parameters move with time and anything
else, so they cannot be kept current.

**Reasons:** `ability.unknown` (`ability`), `ability.not_held`
(`entity`, `ability`), `ability.no_grant` (`ability`, `action`),
`ability.dormant` (`ability`), `ability.rank_too_low` (`ability`,
`rank`, `needed`), `ability.cooling_down` (`ability`, `action`, `left`
seconds), `ability.condition` (`ability`, `action`),
`ability.cost_short` (`entity` who pays, `pool`, `wanted`, `have`,
`short`), `ability.cost_source_gone` (`ability`, `pool`),
`ability.items_short` (`item`, `wanted`, `have`), `ability.cost_unknown`
(`cost`), `ability.mode_unknown`, `ability.mode_on`, `ability.mode_off`
(`ability`, `mode`), `ability.upkeep` (`ability`, `mode`, `pool`),
`ability.rank_from_track` (`ability`), `ability.bad_rank` (`rank`),
`ability.bad_argument` (`arg`), `ability.not_registered`,
`ability.suppressed` (`ability`, `by`), `ability.bad_duration`
(`seconds`), `ability.no_binder`, `ability.too_many_bindings`
(`limit`), `ability.bad_binding` (`arg`). The faults of 05 §7.5: `reactive.rounds`,
`reactive.unwatchable`.

**For the application and the AI layer.** `ability::options(sim,
entity)` lists every grant of every ability held, with its action, its
intent, what it costs now, the cooldown left and the reason it cannot be
used now, if any; `ability::available(sim, entity, ability, action)`
asks one. Both read the owner's own state and the grant's `if` in its
mind. A choice is carried out with `UseAbility` and `SetMode` (or the
verbs, from a rule).

**Not built.** Snapshots and
journals (A8); relays, appraisal, moving fields, contest bias (A9); the
reference abilities of section 8 (A10). Costs from a store (16 §5).
Bindings to portals (02 §19 is not built); `bound` on `add_trait` and
`attach_rule` (`bind_trait` and `bind_rule` do it); orders to a summoned
servant (until relations, 15 §10). The effect `move_area` (the world's
`WorldCtx::move_area` is reachable only from Rust) and the queries
`abilities`, `bindings` and `modes`: a rule reads `self.abilities`,
`self.ability(id)`, `ability.bound(name)` and `ability.mode_on(mode)`
meanwhile.
Variables on the kind `ability` (`ability.var.x`). A rule attached by an
ability does not bind `ability`. Abilities of items and body parts
except through the traits they give. `uses` on a mode. The mind-seated
abilities a trait gives do not move with a mind until traits' seats do.
A level of a pool changed by a verb without crossing a threshold is not
heard by a mode's wake-up until the next crossing or wake-up.
Benchmarks (A11).

## 13. Invariants

| # | Invariant |
|---|-----------|
| A1 | An ability changes the simulation only through verbs that exist independently of it. |
| A2 | A dormant or suppressed ability confers nothing. |
| A3 | Everything bound to an ability ends, by its declared `on_end`, when the ability is revoked. |
| A4 | A cost is taken in full, or the action does not start. |
| A5 | A mode's upkeep and its trait begin and end together. |
| A6 | A reactive condition's value always equals a fresh evaluation. |
| A7 | Restoring a snapshot changes the world only through `WorldEdit` and the entity only through its components' own functions. |
| A8 | One ability from several sources is one instance, and lasts exactly as long as any source. |

## 14. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Checking an action's availability, including cost | under 1 µs |
| Granting or revoking an ability | under 10 µs |
| Re-evaluating a reactive condition | under 500 ns, and only when a dependency changed |
| Moving an anchored area of radius 40 one cell | under 50 µs |
| Snapshot or restore of a 16 × 16 × 4 box | under 300 µs |
| Abilities and bindings while nothing changes | no cost |

## 15. Open questions

1. **Turning back the world.** Offered as an experimental feature in
   13 §9.5: a rule, including an ability's, may ask for the whole
   simulation to be returned to an earlier moment with memories carried,
   where world history is being kept. Its cost and several of its
   interactions are unresolved and listed there.
2. **Seeing the future.** Deferred until the experimental whole-world
   rewind (13 §9.5) has been tried. Running a copy ahead needs the same
   machinery and has the same cost.
3. **Shared minds.** Settled in 12 §9.6: several controlled characters can
   be given one command together.
4. **Judging non-harmful abilities.** Settled. Scoring helpers are
   supplied (09 §6.1), and that is as far as it goes. A character uses
   trickery and control when its rules and scores say to, the same way
   each time. It is not expected to build schemes out of them (09 §1).
5. **Reactions between elements.** Settled in 02 §16: field reactions.
