# 08 — Abilities and Powers

Status: design complete, ready for implementation.
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
      "do": [ { "track_add": { "who": "self", "track": "pyrokinesis", "amount": 1 } } ] } ] }
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
says `"energy": "ability.power * $effort"` and never needs to know the
rank.

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

`revoke_ability` removes an ability. So does the end of its last source.
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
| `create_area`, `open_portal`, `attach` with `"bound": true` | As named |

A binding ends when its duration runs out, when it is dismissed, when the
ability is suppressed or revoked, or when its owner stops (06 §8.3), as
the ability's definition chooses. On ending:

| `on_end` | Does |
|----------|------|
| `remove` | The trait is removed; the entity is destroyed; the area, portal or rule goes |
| `restore` | For an edit: the element is put back as it was, if it is still what was conjured |
| `release` | The thing stays, no longer bound: a summoned beast now free, a conjured wall now ordinary |

`dispel { where }` ends bindings by ability tag, by owner or by place.

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
`WorldEdit`. Fields are restored from copies the journal takes each second.
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

**Hooks**: `ability_using`, `ability_suppressing`, `binding_ending`,
`contest_resolving`.

**Effects**: `grant_ability`, `revoke_ability`, `set_rank`,
`suppress_abilities`, `restore_abilities`, `start_mode`, `end_mode`,
`summon`, `conjure`, `dispel`, `dismiss`, `snapshot`, `restore`,
`rewind_area`, `restore_as_generated`,
`create_area`, `move_area`, `relay_senses`, `appraise`, `move_field`.

**Queries**: `abilities` (of an entity, by tag), `bindings` (of an ability
or owner), `modes`.

**Properties**: on abilities, `rank`, each parameter by name, `owner`,
`state`, `mode(id).on`, `cooldown(action)`, `var.*`; on entities,
`has_ability(id)`, `ability(id)`, `abilities_with(tag)`.

**Functions**: `magnitude`.

**Settings**: `max_snapshot_cells`, `max_journal_changes`, `max_bindings`.

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
