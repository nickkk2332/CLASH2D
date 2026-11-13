# 07 — Combat, Techniques and Clashes

Status: design complete, ready for implementation.
Implementation plan: [plans/combat.md](plans/combat.md).
Serves scenes S1, S2, S4, S5, S6, S14, S19 and S23 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies fighting: how an attack becomes a hit, how a hit is
avoided, turned or stopped, what happens when two attacks meet, holding and
throwing, takedowns, and weapons that act at a distance.

It adds no timing system and no damage system. Attacks, defences and
reactions are actions, windows and joint actions (03). What a hit does to a
body is 06 §7, and to the world, 02 §12. Combat's work is everything
between a decision to attack and a `Hit`.

Fixed from earlier documents:

- No hit points and no damage rolls. Outcomes come from energy and
  intensity against materials (decisions D-020, D-067).
- Nothing is scaled for fairness (D-034).
- Every choice has a default, and every choice can be made by hand (D-037).
- Striking several foes at once is something a technique does, not
  something the engine grants against the weak (D-040).

Names of techniques, attributes and contests in examples are illustrative
content. The engine has no "sword skill" (01 R10).

## 2. The picture

A **technique** is an action (03 §6) with an **attack** in it. The attack
names a **surface** to strike with (a fist, an edge, a point), how much
**energy** goes into it, and how much of that energy shoves the target
instead of hurting it.

When the action reaches its impact, the strike is resolved in order. Is the
target in reach? Is the target striking back at the same instant? Did the
strike find the part it was aimed at? Is a **defence** in the way? Whatever
energy is left arrives as a hit on a body part, and the body model decides
what that does. Then the shove moves the target, perhaps a step, perhaps
through a wall.

A defence is an action too, usually a reaction (03 §7). It works in one of
two ways. It **avoids**: the defender is not where the strike lands. Or it
**interposes** something, a shield, a blade, an arm, which takes the strike
first and can turn some of it aside. What a defence can turn aside is
limited by the defender's strength. Past that limit the strike comes
through the shield, and through whatever is behind it.

When two fighters strike at each other within an instant, the strikes
**clash**. Evenly matched, both stop and the energy goes into the street.
Otherwise the stronger carries through, lessened by the weaker.

Bullets, arrows and thrown things are bodies in flight (02 §15). They take
time to arrive. Someone fast enough can step aside, and nobody else can.

## 3. Attacks

### 3.1 Definition

An action declares its attacks and strikes with them in a phase.

```json
{ "type": "action", "id": "slash", "name": "Slash",
  "uses": { "arm": 1 },
  "params": {
    "target": { "type": "entity" },
    "part":   { "type": "part", "default": "auto_part($target)" },
    "effort": { "type": "real", "default": "auto_effort(self, $target)" } },
  "attacks": {
    "cut": {
      "with":   { "held": "blade", "surface": "edge" },
      "reach":  "limb.reach + weapon.length",
      "energy": "swing_energy(self, weapon)",
      "push":   0.05,
      "aim":    { "contest": "melee_aim", "by": "blade_skill(self)", "against": "evasion(target, self)" },
      "clashes": true } },
  "phases": [
    { "id": "windup", "time": "300 ms", "cancel": true,
      "window": { "kind": "strike", "for": ["$target"], "closes": "impact" } },
    { "id": "impact", "time": 0,
      "do": [ { "strike": { "attack": "cut", "target": "$target", "part": "$part", "effort": "$effort" } } ],
      "next": [ { "if": "$landed", "goto": "follow" }, { "goto": "recover" } ] },
    { "id": "follow",  "time": "150 ms", "cancel": ["combo"] },
    { "id": "recover", "time": "350 ms" } ] }
```

`attacks` is an object of attacks by id: an id is lowercase letters,
digits and `_`, starting with a letter, and no two of an action's are
alike.

| Attack field | Type | Meaning |
|--------------|------|---------|
| `with` | object, required | The striking surface: on a held item, on a body part, on a gripped creature or object, or given inline for a strike made with no physical thing (08 §7.7). Below. |
| `reach` | expression, metres | The target must be within it at impact (section 3.4, step 2). Left out: the reach of the part that strikes (06 §6.4); for a held item, the reach of the part holding it plus the item's `length`; for an inline surface, the attacker's body's. |
| `energy` | expression, joules, required | Joules at full effort |
| `push` | expression, 0 to 1 | Share of the energy that moves the target instead of harming it (section 7). Default 0. |
| `lift` | expression, degrees | Upward angle of the push. Default 0. |
| `aim` | object | How the strike finds its mark (section 4): `contest`, the id of a `contest`; `by`, the attacker's value; `against`, the target's; both expressions. Absent means it cannot miss. |
| `arc` | object | For strikes at several targets (section 6.4): `sweep`, degrees, or `line`, a length, one of the two; `max`, at least 1, required; `through`, a bool, default false. |
| `knock_down` | `id<posture>` | A posture a target the push launches is put in (section 7). |
| `swing` | number, degrees | Degrees the surface travels through before it lands. Obstacles in that arc are struck first (section 6.4). Absent for thrusts and shoves. Read and carried: not built. |
| `height` | `[low, high]`, metres | The band of heights, in metres above the attacker's footing, that the attack can reach (section 4.3). Defaults to the attacker's own height and a little over. Read and checked (`low ≤ high`): not built. |
| `lands` | phase id | The phase at whose start this attack strikes. By default it is found when the action loads: the phase whose `do` strikes with it. An attack struck from a `tick` or under a condition has no such phase, never clashes, and has no time to impact. Read and carried for clashes (C6). |
| `clashes` | bool | Whether this attack can meet another (section 8). Read and carried. |
| `spill`, `spill_kind` | number, text | What a locked clash releases (section 8). Read and carried. |
| `on_land`, `on_miss`, `on_stopped` | effects | Run once for each target the strike lands on, misses, or reaches and delivers nothing to (section 3.4, step 8). They bind `self` and `target` and the action's locals, and run on a copy of the locals with what came of that target in `$landed`, `$absorbed` and `$passed`: what they `set` there does not last. They may not `strike`. |

An attack's expressions are compiled with the action's own scope, reading
truth (03 §6.1), and three subjects more after `self`: `weapon`
(`entity`), the item whose surface strikes; `limb` (`part`), the body
part whose surface strikes; and `target` (`entity`), the one being
struck. Each is null where there is none: `weapon` for a part's own
surface, `limb` for an item's or an inline one, `target` for a strike at
a cell. They are evaluated when the strike resolves, `reach`, `push` and
`lift` and the aim's values once for each target. Whatever the actor
believed when it chose to strike, the strike is resolved in truth.

An action with attacks has three more locals, as variables, unless it
declares them itself (decision D-401): `$landed` (`bool`, false at
start), `$absorbed` and `$passed` (`real`, 0 at start). One it declares
must be able to hold what `strike` leaves in it.

**`with`.** One source of a surface, and the surface's id:

| Key | Finds |
|-----|-------|
| `part` | The first working part of the attacker's body, by key, that answers to the name (its name in the body, `r_hand`; the id of its definition, `hand` or `core:hand`; or a tag its definition carries) and has the surface: one that grasps nothing before one that does. |
| `held` | The first item, by id, held in a grasp (06 §9.2) that carries the tag and whose definition has the surface. An empty tag takes any item held. |
| `gripped` | `true`: the first creature the attacker grips, in the order the grips were taken (section 9.4). With `surface`, the first working part of it, by key, that has that surface, scaled as a part's is: it strikes, and takes the recoil. Nothing gripped is a refusal (`combat.nothing_gripped`). In a defence's `with` the surface may be left out: the creature is put in the way whole. |
| `surface` | The surface's id among those of the part or the item. Required with the three above, but for `gripped` in a defence. |
| `kind`, `area`, `give`, `span` | An inline surface (08 §7.7): a damage type, an area above zero, a give above zero and a span, default 0. It takes no recoil. |

A part's surface scales with the part (06 §6.4): its area with the square
of the part's size, its give and its span with the size. An item's is as
its definition writes it. Nothing found is a refusal
(`combat.no_surface`).

### 3.2 Surfaces

Items and body parts both carry striking surfaces (06 §9.1, §6.1), each
with an `id`. The bundled humanoid strikes with the `fist` of its hands,
the `heel` of its feet, its `elbow`, and, given by
`mods/core/data/life/surfaces.json`, the `palm` of a hand, the `sole` of
a foot and the `brow` of its head.

| Field | Meaning |
|-------|---------|
| `kind` | The damage type it delivers: cut, pierce, impact |
| `area` | Contact area, m² |
| `give` | Metres over which the strike is stopped. A hard edge gives almost nothing; a gloved fist gives centimetres. |
| `span` | Metres of contact across the target. An edge's length. What decides whether a cut can sever. |

### 3.3 Effort

Every strike takes an `effort` from 0 to 1. Energy is multiplied by it.
One outside 0 to 1, or not a number, is refused (`combat.bad_number`):
it is never brought into range.

Effort is how a character holds back. Someone able to punch through a wall
needs to be able to tap a pickpocket on the shoulder. The default is a
function (05 §12), so content can make restraint the habit of a hero and
full force the habit of a brute, and the player can override either at any
time.

### 3.4 The strike, in order

The `strike` effect resolves one attack against one target:

| Step | Section | May end the strike |
|------|---------|--------------------|
| 1. Hook `strike_resolving` | — | A rule cancels or alters it |
| 2. Reach | — | Target is not there: miss |
| 3. Clash | 8 | Locked, or overpowered |
| 4. Aim | 4 | Miss |
| 5. Defences | 5 | Avoided; or all energy turned aside or absorbed |
| 6. Delivery | 6 | — |
| 7. Push | 7 | — |
| 8. Results | — | Events; `on_*` lists; the action's locals `$landed`, `$absorbed`, `$passed` |

A strike at a cell rather than a creature skips steps 3 to 5 and delivers
its whole energy to the element as a packet (02 §12): the cell's block,
else its fixture, else its floor, by `WorldEdit::damage_along` in the
direction from the attacker. Reach applies to the cell as to anyone
standing in it; out of reach, `strike_missed` is raised with no target. An element it does not affect returns all of
it as recoil (section 6.3); what passes an element it destroys is spent
there.

**The effect.** `strike` is read by the rule compiler, not a verb
(decision D-641): it leaves what came of it in the action's locals, which
a verb cannot reach, and it resolves an attack of the action whose list
it is written in. It is a load error in a rule, in an attack's own
`on_land`, `on_miss` and `on_stopped`, and in an action without the
attack it names.

```json
{ "strike": { "attack": "cut", "target": "$target", "part": "$part", "effort": "$effort", "sure": false } }
{ "strike": { "attack": "fist", "at": "$cell" } }
```

| Argument | Type | Meaning |
|----------|------|---------|
| `attack` | id | The attack, by its id in the action's `attacks`. May be left out of an action with exactly one. |
| `target` | `entity?` | Whom it strikes. One of `target` and `at`. |
| `at` | `loc?` | The cell it strikes instead. |
| `part` | `part?` | The part of the target it is aimed at; null for none. |
| `effort` | `real?` | From 0 to 1; default 1. |
| `sure` | `bool?` | Skips aim: it cannot miss its mark (section 10). Default false. |

It leaves `$landed` (whether it landed on anyone or anything), `$absorbed`
(the joules what it struck took: bodies, what they wear, items,
elements) and `$passed` (the joules that went on beyond the last thing
struck). A refusal is reported as a fault, and the locals keep their
values.

**Reach.** A target is within reach when the distance between where the
two stand, the middles of their cells at their footing, less one cell
width (half for each of them), is at most the attack's `reach`.
Neighbours are always within reach; a diagonal neighbour is 0.41 m away.
A target not in the world, or in another grid, is out of reach.

**Hook.** `strike_resolving` is raised once, before the targets are
found, with the attack's energy at full effort. A handler may change
`target`, `part`, `energy` and `effort`, or cancel: then nothing is struck
and `$landed` stays false.

**Depth.** A strike resolved inside eight others (an `on_land` that starts
an action that strikes at once, and so on) is refused
(`combat.too_deep`).

Where several defences are in force at step 5 they are tried in a fixed
order: first those of others who protect the target, then the target's
own; within each, in the order they came into force, and by entity id
where that is equal. Each takes what it can, and the next meets what is
left.

### 3.5 Speed behind the blow

`closing_energy(self, target, mass)` is the kinetic energy of that mass at
the speed `self` is closing on `target`: `½ · mass · v²`, with `v` the part
of their relative velocity along the line between them. `self.velocity` is
the mover's own while it is stepping, the mount's or vehicle's for a
rider, and the body's for anything in flight.

An attack's `energy` may add a share of it. A couched lance carries the
horse. A diving blow carries the fall. A punch thrown while standing still
carries nothing extra.

## 4. Aim

### 4.1 Contests

A **contest** compares two numbers, with as much or as little chance as
content wants.

```json
{ "type": "contest", "id": "melee_aim", "form": "difference", "factor": 4, "spread": 1.0, "near": 0.5 }
```

`contest(id, a, b)` returns a **margin**: the scaled difference (or
log-ratio, for `"form": "ratio"`) between `a` and `b`, plus noise of the
given spread. A spread of zero is a pure comparison. `odds(id, a, b)` is
the chance the margin is at or above zero; it draws nothing and is pure.

Contests are the only place chance enters combat, and how much is a number
in a data file.

| Field | Meaning |
|-------|---------|
| `name` | What a reader calls it. A text; optional. |
| `form` | `difference` (the default): `d = factor × (a − b)`. `ratio`: `d = factor × (ln a − ln b)`; a value at or below zero is less than any above zero, so `d` is minus infinity when only `a` is, plus infinity when only `b` is, and zero when both are. |
| `factor` | What the comparison is multiplied by. Above zero; default 1. (`scale` is the loader's word for multiplying a field, 04 §6.1, so it cannot name a field.) |
| `spread` | The scale of the noise. Zero or more; default 1. |
| `near` | For aim (section 4.2): how far below zero a margin may fall and still land beside the part aimed at. Zero or more; default 0. |

The noise is logistic with the spread as its scale: a margin is
`d + spread × ln(u / (1 − u))` for one number `u` drawn uniformly from
the open interval (0, 1), so the odds are `1 / (1 + e^(−d / spread))`.
With a spread of zero the margin is `d`, and the odds are 1 when `d` is
at or above zero and 0 otherwise. Every contest draws exactly one number
from the rule stream, whatever its spread.

| Function | Arguments | Gives |
|----------|-----------|-------|
| `contest` | `contest: id<contest>`, `a: real`, `b: real` | `real?`: a margin, drawn from the rule stream. Refused in a pure context (05 §7.3). Safe for belief: it reads nothing of the world. |
| `odds` | `contest: id<contest>`, `a: real`, `b: real` | `real?`: the chance a margin is at or above zero, from 0 to 1. Pure, and safe for belief. |

Both give null for a null value. A function the simulation registers
that draws at random is computed with the stream of whoever evaluates
it, and gives null where nothing may be drawn.

### 4.2 Finding the mark

An attack's `aim` names a contest, the attacker's value and the target's.

| Margin | Result |
|--------|--------|
| At or above 0 | Lands on the part aimed at |
| Below 0, within the contest's `near` | Lands on a neighbouring part, chosen by exposure |
| Lower | Misses |

A strike with no part chosen lands on a part picked by exposure.

What the target's value is, is content. The usual pattern is a function
that is zero for a target who does not perceive the attacker (15 §6), is
held, or cannot move. Such a target is simply hit. Striking from where the
target cannot see is an advantage because of perception, not because of a
rule about backs.

As built (decision D-643). The attacker's and the target's values are
evaluated with the target bound, and one margin is drawn from the rule
stream. The parts **beside** a part are those joined to it on the
outside: the part it is `on`, and the parts `on` it, that are there and
sit inside no other. One of them is chosen by exposed area (06 §7.1),
with one more draw from the rule stream; where there are none, or no part
was aimed at, the body picks one by exposure. A part aimed at that is gone
is no part: the body picks. A strike that is `sure`, or whose attack has
no `aim`, does not aim and lands on its mark. The margin is reported in
`strike_landed` and `strike_missed`.

The bundled `melee` contest compares by **ratio** (`factor` 3, `spread` 1,
`near` 1.5): between equals half of all blows land on the mark, a third
beside it, and the rest miss. Its `evasion(who, from)` is
`fighting(who)` for someone who perceives the attacker and zero for
someone who does not, and in a ratio nothing is below anything: an
unaware target is always hit where it was aimed at.

### 4.3 Reach in height

An attack reaches a band of heights above the attacker's footing. A part
can be aimed at only if its own band (06 §6.1), scaled to the target's
height in its present posture (06 §11.1) and shifted by the difference in where the two are standing,
overlaps it. A strike with no part chosen picks among the parts in reach.
If nothing is in reach the attack cannot be made, and says so.

The head of something four storeys tall is out of reach from the ground.
It can be reached by leaping, by climbing (06 §6.6), by a longer weapon, or
by bringing it down. A low sweep cannot be aimed at a head.

## 5. Defences

### 5.1 Definition

A defence is an action with a `defence` block. It is in force during one of
its phases.

```json
{ "type": "action", "id": "parry", "name": "Parry",
  "reaction_to": ["strike"], "uses": { "arm": 1 }, "ready": "guard",
  "phases": [ { "id": "set",   "time": "80 ms" },
              { "id": "guard", "time": "250 ms" },
              { "id": "recover", "time": "150 ms" } ],
  "defence": {
    "during": "guard",
    "form": "interpose",
    "with": { "held": "blade", "surface": "flat" },
    "covers": { "arc": 120 },
    "handles": 1,
    "contest": { "id": "control", "by": "blade_skill(self)", "against": "blade_skill(attacker)" },
    "deflect": 0.9,
    "holds": "self.attr.strength * 60",
    "brace": 0.3 } }
```

| Field | Meaning |
|-------|---------|
| `during` | The phase in which it is in force |
| `form` | `avoid` or `interpose` |
| `with` | For `interpose`: what is put in the way. A held item, a body part, a gripped creature. |
| `protects` | Whom it defends. By default the defender. A defence may instead protect a character the defender is beside, taking strikes aimed at them. With a reaction that allows `on_behalf` (03 §7.1), this is stepping in front of a blow meant for someone else. |
| `covers` | Which strikes it applies to: by direction, by the part they are aimed at |
| `handles` | How many strikes it can deal with while in force; `"all"` for a shield wall |
| `contest` | Whether it succeeds. Absent means it always does. |
| `deflect` | Share of the strike's energy turned aside on success |
| `holds` | The most energy, in joules, the defender can turn aside |
| `brace` | Share of the push the defender resists |
| `evasion` | What it adds, while in force, to the defender's value in the aim of a strike at it (section 4.2): a sway makes someone harder to land on squarely |

A defence need not be a reaction. Held up in advance, as a held phase
(03 §6.4), it is a guard: in force for as long as it is kept, at the cost of
the channel it occupies.

### 5.2 Avoiding

There are two ways not to be hit.

**Leaving.** A dodge, a sidestep or a leap is a movement action. If the
defender is out of the strike's reach when it lands, step 2 misses. No
contest is involved. The defender had to have noticed in time and been able
to move.

**Slipping.** A defence of form `avoid` stays in the cell: a duck, a sway.
If its contest succeeds, the strike misses.

### 5.3 Interposing

If an interposing defence succeeds:

```
turned    = deflect × min(energy, holds)
energy    = energy − turned
the rest lands on the interposed surface as a packet
what that passes continues to the part originally aimed at
```

- A parry with a blade turns most of a strike it can hold. The remainder
  lands on the blade, which is an item made of a material (06 §9.1).
- A shield is the same with a larger surface and a larger `holds`.
- An arm raised to block takes the hit on the arm.
- A strike many times stronger than `holds` is barely turned at all. It
  lands on the shield at nearly full energy, the shield takes what its
  material can, and the rest arrives at the defender.

If the contest fails, the defence does nothing.

### 5.4 What defends automatically

Nothing. A character defends if a defence of theirs is in force when the
strike lands. It is in force because they held it up beforehand, or because
they noticed the attack's window in time (03 §7.2) and a rule or the player
started a reaction that became ready before impact.

Someone too slow to notice has no defence. Someone caught in a recovery
phase that cannot be cancelled has none either.

### 5.5 As built

Package C4 (decisions D-712, D-713), in `clash_sim::combat::defence`.
The block's fields, as read:

| Field | Type | Meaning as built |
|-------|------|------------------|
| `during` | phase id, required | The phase in which it is in force, and only then (invariant C3). A held phase is a guard, in force while it is held |
| `form` | `avoid` or `interpose`, required | |
| `with` | object, or a list of them | For `interpose`, required: a held item or a part, written as an attack's `with` (section 3.1), not inline. Of a list the first that answers when the strike lands is used: a shield if one is held, else the arm. Not allowed for `avoid` |
| `protects` | expression, `entity?` | Whom it defends, read when the strike lands with `self` and the action's locals. Left out: the defender. Someone else is defended only while the defender stands beside them (no gap between them) |
| `covers` | `{ "arc": degrees, "parts": [names] }` | `arc`, centred on the defender's facing (15 §4), the strike must come from; left out, or for one with no senses, every direction. `parts`, each a part's name, the id of its definition or a tag, the strike must land on; left out, or where the body picks the part, any |
| `handles` | int at least 1, or `"all"` | How many strikes it is tried against while in force: each phase it comes into force in counts afresh. Default 1 |
| `contest` | `{ "id": contest, "by": expr, "against": expr }` | At or above zero it succeeds; drawn from the rule stream. Left out: it always succeeds |
| `deflect`, `brace` | expression, 0 to 1 | Default 0. A value outside is brought to the nearest end |
| `holds` | expression, joules | Left out: no limit |
| `evasion` | expression | Default none |

`contest`, `deflect`, `holds`, `brace` and `evasion` read truth when the
strike lands, with the subjects `self` (the defender), `attacker` and
`target` (the one struck), then the action's locals.

**Step 5.** The defences in force for the target are tried in the order
of section 3.4: others' that protect it, then its own, each group by
when its phase began, then entity, then action. One is tried if it has
not been tried `handles` times in this phase, covers the strike and,
for `interpose`, finds what it puts in the way now. The hook
`defence_resolving` may change it; `strike_defended` reports it.

- `avoid` that succeeds: the strike misses (`strike_missed` with `why`
  `defence`), and nothing after it is tried.
- `interpose` that succeeds: `turned = deflect × min(energy, holds)` is
  taken off, and the next meets the rest. The push share of what is
  left after turning pushes the target, less the largest `brace` of
  those that succeeded; the rest lands on each interposed surface in
  turn as a packet of the strike's kind at the energy over **that
  surface's own** area times its give (a broad, yielding face spreads
  it); what passes the last goes on to the part aimed at, struck by the
  attacker's surface. What a surface was not affected by comes back
  into the striking surface as recoil. A part put in the way is struck
  with no direction: what exits it goes on to the part aimed at, not to
  the wall behind.
- If the surfaces took all of it, nothing reached the body: the strike
  is stopped (`strike_stopped` with `combat.defended`, `on_stopped`),
  `$landed` stays false, and the push and the recoil still happen.

`evasion`: before aim, the target's own defences in force that cover the
direction the strike comes from add their `evasion` to the aim's
`against`.

**Leaving** needs nothing: a defender out of reach at impact is missed
at step 2 with no contest. A reaction turns its reactor to face the
window's source when it starts (03 §7.3), which is what a parry's arc is
measured from.

`entity.defending` (`bool`) is whether a defence of its own is in force
now. What defences in force have dealt with is kept, saved and hashed,
in the simulation's window table.

A defence's `with` may be `{ "gripped": true }`: the creature its
holder grips is put in the way, and takes what is left of the blow on
its body where it is exposed, struck with the blow's own surface; what
exits it goes on to the part aimed at (section 9.4, decision D-813).
Defences meet shots too (section 11.6). Not built: the query
`defences`, and previews of defences.

## 6. Delivery

### 6.1 The hit

What remains of the strike becomes a `Hit` (06 §7.1):

```
packet.kind      = surface.kind
packet.energy    = energy × effort × (1 − push)
packet.intensity = packet.energy / (surface.area × surface.give)
hit.area         = surface.area
hit.span         = surface.span
hit.part         = the part found by aim
```

Intensity is why the same blow from an edge and from a fist differ. An edge
has a ten-thousandth of the area and a tenth of the give, so its intensity
is far above what skin and muscle can resist and little energy is spent in
each layer on the way through. A fist's is not, and its energy stays in the
first layers.

As built, `energy × effort` is the energy that reaches the target (for a
target later in a `through` arc, what passed the one before), and
`hit.dir` is the direction from the attacker's place to the target's,
of length one. The hit goes through `Sim::hit`: the hook `hit_incoming`,
the layers the target wears (06 §9.2), the body's own (06 §7.2). A
target with no body that is an item takes the packet as an item does
(06 §9.1); anything else is missed (`why` `nothing`). A hit a handler of
`hit_incoming` cancels delivers nothing: the strike is **stopped**
(`strike_stopped`, `on_stopped`), and nothing pushes or recoils.

### 6.2 Severing and passing through

A part is severed when the damage type is flagged `severs`, every layer is
breached, and `hit.span` is at least the part's girth (06 §7.2). A short
blade cuts deep into a thigh. A long one takes it off.

Energy that exits the far side continues along the strike's direction to
whatever is next: another part, another creature, a wall.

As built (decision D-642): what exits a body first strikes the block of
the next cell along the strike's direction (06 §7.4, the body wiring's).
Where that cell holds a block, what the block did not stop is spent there
and the strike ends. Where it holds none, the energy carries on: with an
`arc` and `through`, to the arc's next target; with no `arc`, to the
first creature with a body in that cell, and on from there, up to 16
creatures beyond the first. What goes through is not aimed, needs no
reach and pushes nothing: all of it is delivered, on the part of the
same name as the one aimed at, or where the body offers. An arc without
`through` strikes each of its targets with the attack's whole energy,
each a strike of its own, and what exits each is the body wiring's
alone.

### 6.3 Recoil

Energy a strike delivers that the target neither absorbs nor passes comes
back into the striking surface as a packet of its own.

- Punch a breastplate whose threshold the fist's intensity does not reach,
  and the fist receives the punch.
- Strike stone with a sword and the sword takes the damage.
- Strength great enough to break a wall, in a body not tough enough to
  survive doing it, breaks the arm. Durability is not optional for the
  strong. This is a consequence, not a rule added for it.

As built: what came back is the hit's `returned` (06 §7.2), or all of a
packet an item or an element was not affected by. It is a packet of the
surface's own kind, at `returned / (surface.area × surface.give)`: a hit
on the attacker's own part whose surface struck, with its contact area,
span, and the strike's direction turned round; or damage to the item
(06 §9.1). An inline surface takes nothing. `strike_recoiled` reports it.

### 6.4 Several targets

```json
"arc": { "sweep": 140, "max": 5, "through": true }
```

An attack with an `arc` strikes every target in a sweep of the given angle,
or along a `line` of a given length, up to `max`, in the order the surface
would meet them. Each target is resolved by steps 2 to 8.

With `through`, the energy that reaches each target is what **passed** the
one before. A blade keeps going through whoever does not stop it. Five
unarmoured men absorb a little each. The sixth, in mail, absorbs the rest.

A sweep also meets the world. Blocks and fixtures standing in an attack's
`arc`, or in the `swing` it travels through on the way to a single target,
and tall enough to be in the surface's path, are struck in their turn as
strikes at a cell (section 3.4). A long blade swung in a corridor finds the
wall first, spends energy on it, and takes the recoil. A thrust has no
swing and is not hindered.

This is scene S4, and the reason a master cuts down a rank of ordinary foes
is the reason given by the numbers: enough energy on a fine enough edge
that flesh takes little of it.

As built (decision D-642), an arc's targets are the creatures with bodies,
other than the attacker, and always the target named:

- A **line** passes through the cells whose middles lie within half a
  cell of the line from the attacker's place through the target's, ahead
  of the attacker, no further than `line` metres beyond the cell between;
  they are met in order of distance.
- A **sweep** reaches those within the attack's reach (as to the target
  named) whose bearing from the attacker lies within half its angle of
  the target named's; they are met in order of bearing, turning from east
  toward south.

At most `max` are struck; the target named takes the place of the last
if it would not otherwise be among them. The part aimed at names a part
of the target named; each other target is aimed at its part of the same
name, if it has one, and at none otherwise. Swings meeting the world
(`swing`, and blocks standing in an arc) are not built.

### 6.5 Contact with the world

A floor, block or fixture may carry a **contact** surface (02 §5.4):
spikes, wire, broken glass, a hot plate. It strikes whatever touches it.

| When | The hit |
|------|---------|
| Something walks or crawls into the cell | Made with the element's surface and its stated `energy`, on the parts whose bands reach its height |
| Something lands on it or is thrown into it | Made with the element's surface and the energy the body arrived with (02 §15.2), in place of the blunt impact |
| Something stays on it, if it says `each` | Made again at that interval |

Intensity comes from the surface's area, so a fall onto spikes pierces
where a fall onto flagstones bruises. The element takes the recoil
(section 6.3) and may break.

## 7. Push

The share of energy set aside by `push` moves the target.

```
speed = sqrt(2 × energy × effort × push × (1 − brace) / target.mass)
```

in the direction of the strike, raised by `lift`. The hook
`knockback_incoming` lets rules change it: rooted stances, anchoring
abilities, footing.

| Speed | Result |
|-------|--------|
| Below `combat.stagger_at` (default 0.5 m/s) | Nothing |
| Below `combat.launch_at` (default 3 m/s) | `staggered`, with a severity. What staggering does is content: usually it interrupts and unbalances. |
| At or above | The target becomes a body in flight (02 §15) at that speed. Walls, floors and bystanders in the way are dealt with by the world. Injuries on impact arrive as hits (06 §7). |

As built: `energy × effort` is the energy that reached the target, and
the mass is its `Physical`'s. The direction is the strike's on the level,
raised by `lift` degrees, of length one; straight up or down for a
strike with no level direction. `knockback_incoming` carries `speed`
and `dir`, and a handler may change either or cancel. A launched target
is first put in the attack's `knock_down` posture if it names one, then
handed to the world as a body (`WorldCtx::launch`, owner the entity)
from its cell's middle at half its height, with its mass, volume,
cross-section and planes; movement takes it back when it lands
(06 §11, `landed`), and what it struck on the way is the world's
`body_impact` (06 §7.4). `launched` reports it. `brace` is the largest of
the defences that put something in the strike's way (section 5.5).
Push is applied only for a strike that landed, or that a defence put
something in the way of.

A shove is an attack whose `push` is nearly 1. A cut's is nearly 0. A
punch from someone who can put a megajoule into it, with a third set aside
as push, sends eighty kilograms away at ninety metres a second. That is
scene S1, and with a tougher target, S2.

Mass is in the denominator. Heavy things are hard to move.

### 7.1 In the air

Someone who has been launched is a body in flight (02 §15) and is still a
creature. They can be struck by anything that reaches where they are at
the moment of impact. They cannot leave (section 5.2), though a defence
that needs no footing still works. A strike that lands adds its push to
their velocity. Hitting someone again before they come down is this.

As built: a target the world carries as a body takes all of the push, at
any speed: no thresholds apply, and nothing braces. It is an impulse of
`mass × speed` along the direction (`WorldCtx::impulse`), and `launched`
is raised with `added` true.

Someone who launched themselves with `leap` (06 §11) keeps their channels.
An action that does not need `footing` (03 §6) can be run during the
flight, and its strike reaches whoever is within reach when it falls due:
a flying kick, a diving stab, a blow brought down from a jump. Its energy
may include the speed of the leap (section 3.5). Someone thrown, who has
not recovered, cannot.

## 8. Clashes

### 8.1 When

At step 3 of a strike by A against B, there is a clash if:

- A's attack and an attack of B's are both marked `clashes`;
- B's attack is aimed at A, or its arc covers A; and
- B's attack is due to land within `clash_window` (default 80 ms) of now.

B's strike is brought forward to this instant and the two are resolved
together. Neither is resolved separately.

A clash can be sought. A reaction that is itself an attack, started in time
to land with the incoming one, meets it.

### 8.2 Resolution

Each side's **power** is its attack's `clash` expression, by default its
energy at the effort given. The hook `clash_resolving` may alter both.

| Powers | Result |
|--------|--------|
| Within `lock_ratio` of each other (default 1.5) | **Lock.** Neither strike is delivered. Each side is pushed by the other's push share. If the attacks name a `spill_kind`, an energy release (02 §13) of `spill` times the sum of both energies occurs between them. Each action goes to its `on_clash` phase if it has one. |
| Otherwise | **Overpower.** The weaker strike is cancelled and its energy returns into its own surface as recoil (section 6.3). The stronger continues from step 4 with its energy reduced by the weaker's. |

`clash_resolved` carries both fighters, both energies and the outcome. It
is what cutscenes key on (12 §8).

A locked clash between two people each able to level a building releases
half of two buildings' worth of energy at the point where their fists met.
The street takes it. That is scene S5.

### 8.3 Struggles

What follows a lock is content. The usual form is a joint action (03 §8)
with a held phase: blades bound, beams pressing against each other. Each
tick both sides spend from a pool and may be offered choices: press,
disengage, try something else. When one side's sustained power falls
behind by more than the lock ratio, the action ends and the other strikes.

## 9. Holds and throws

### 9.1 Grips

```rust
pub struct Grip {
    pub holder: EntityId,
    pub held: EntityId,
    pub on: Option<PartKey>,       // the part held, if a particular one
    pub action: ActionId,          // the holder's held action that maintains it
    pub pins: bool,
}
```

`seize` is resolved like a strike with no energy: reach, aim, defences. If
it lands, a grip exists for as long as the holder's gripping action
continues. That action occupies the channels it gripped with.

While gripped:

- The held cannot move away without breaking the grip. If the holder
  moves, the held is dragged, if the holder can move that mass.
- With `pins`, the channel slots of the gripped part are disabled.
- Either may attack the other. The held's `evasion` is whatever content
  says it is for someone held.

Breaking free is an action with a contest, defined by content, that ends in
`release_grip`.

### 9.2 What a grip allows

| Technique | Mechanism |
|-----------|-----------|
| Throw | `throw` ends the grip and launches the held as a body with `speed = sqrt(2 × energy / mass)` |
| Slam | A throw aimed at the floor or a wall a cell away |
| Choke, lock | A trait on the held with pool rates, or repeated hits on the gripped part |
| Carry | Taking the held into a `Grasp` hold (06 §9.2), if content's condition on strength and mass allows |
| Shield | A defence of form `interpose` whose `with` is the gripped creature |
| Strike with | An attack whose `with` is the gripped creature or object, using its mass and whatever surface it presents |

Grappling positions and transitions are joint actions and traits defined by
content.

### 9.3 Holding the world

`seize` may also take hold of a part of the world: an element of a cell, or
a cell of a subgrid.

- **Hanging on.** While the grip lasts the holder does not fall. A ledge
  caught on the way down, a rope, the side of a moving vehicle, the hide of
  something huge (06 §6.6).
- **Wrenching.** `wrench` applies the holder's energy to what is held as a
  packet of a damage type content names for tearing. If that is enough to
  destroy it as a fixed thing (02 §12.2), it comes away in the holder's
  grip instead of becoming rubble: a small thing as an item, a larger
  connected piece as a subgrid by `detach` (02 §18.3). What is then held
  can be carried, swung as a weapon (section 9.2) or thrown, if the holder
  can bear its mass.

A door off its hinges, a lamp post, a car door used as a shield and the
corner of a building are one verb with different amounts of energy.

### 9.4 As built

Package C7 (decisions D-812, D-813, D-814), in
`clash_sim::combat::grip`. A grip is plain state in the simulation's
`GripTable` (saved, hashed once anything has been gripped):

```rust
pub struct Grip {
    pub holder: EntityId,
    pub held: EntityId,
    pub on: Option<PartKey>,           // the part held
    pub action: ActionId,              // the holder's action that maintains it
    pub pins: bool,
    pub pinned: Vec<(ChannelId, u16)>, // the slots the pin disabled
    pub drags: Option<Real>,           // the most mass dragged, kg; None for any
}
```

**Taking hold.** `seize` is a verb, run from one of the holder's own
actions: the grip belongs to the action whose list runs it. A grab is
an attack with no energy whose `on_land` runs `seize`, so that it is
resolved like a strike first: reach, aim, defences (a strike with no
energy is still resolved against its target). `seize` checks, and
refuses having changed nothing: an action of the holder's runs it
(`combat.no_action`); what is held is something else in the world
(`combat.no_target`); the part is its (`combat.not_its_part`); it is
within the reach of the holder's body (`combat.out_of_reach`); then
the hook `seize_resolving` may change the part and whether it pins, or
cancel (`combat.seize_failed`). `seized` reports it.

| Verb | Arguments | Returns | Does |
|------|-----------|---------|------|
| `seize` | `who: entity?` (default `self`), `target: entity`, `part: part?`, `pins: bool?` (default false), `drags: real?` (kilograms; default any) | `bool`, true | Takes hold, as above |
| `release_grip` | `who: entity?` (default `self`), `other: entity?` | `int`, how many ended | Ends the grips `who` holds or is held by, only those with `other` if given |

**How long** (invariant C7). A grip never outlives its action: when the
action ends, by any path, its grips end (`grip_broken`, `how`
`ended`). A grip ended by anything else interrupts its action, so that
the action never goes on holding nothing: `release_grip` by the held
(`how` `broken`, the action interrupted with `combat.grip_broken`);
by the holder outside the maintaining action's own lists (`released`,
`combat.grip_released`; from its own lists the action goes on); a throw
(`thrown`); the two coming apart, no longer beside each other, either
launched, or either leaving the world (`lost`, `combat.grip_lost`).

**While it lasts.**

- With `pins` and a part, the channel slots that part and the parts on
  it supply (06 §6.1) are disabled (03 §6.2), and given back as it
  ends: the actions using them are interrupted, and those that need
  them are refused with `engine:action_channel_lacking`.
- A step of the held to a cell not beside its holder is refused
  (`combat.gripped`): it cannot move away without breaking the grip.
  A step of the holder dragging something heavier than its `drags` is
  refused (`combat.too_heavy`); otherwise, when the holder steps, the
  held is moved (`move_to`) into the cell the holder left. Movement asks
  `grip::may_step` before it moves anyone.
- An attack or a defence `with` `{ "gripped": true }` uses the held
  (section 3.1): swung, it strikes with a surface of its own parts and
  takes the recoil there; put in the way, it takes the blow on its body
  with the blow's own surface.
- `throw` of a creature the thrower grips ends the grip and launches the
  held as a body (02 §15) from the middle of its body where it stands,
  at `speed` or `√(2 × energy / mass)`, toward `at` or along `dir`;
  aimed at itself, away from the thrower on the level. The world
  resolves what it meets; movement takes it back when it lands.
  `launched` and `thrown` report it.

**Properties**, of the kind `entity`, readable of oneself in a mind
context: `entity.gripping` (`entity?`), the first creature or thing it
grips; `entity.gripped_by` (`entity?`), who grips it, the first to have
taken hold.

Not built: carrying (a creature taken into a `Grasp` hold, 06 §9.2),
holding the world (section 9.3, `wrench`), the query `grips`, chokes and
locks other than as content's traits.

## 10. Takedowns

A takedown, an execution or a finishing move is a joint action (03 §8) with
a forced role for the victim.

```json
{ "type": "action", "id": "blade_takedown", "name": "Takedown",
  "roles": { "attacker": { "uses": { "arm": "all" } },
             "victim":   { "uses": { "arm": "all", "legs": "all" }, "forced": true } },
  "requires_at_start": "not perceives($victim, $attacker) or $victim.has_trait('helpless')",
  "cutscene": "takedown_from_behind",
  "attacks": { "throat": { "with": { "held": "blade", "surface": "edge" }, "energy": 60, "push": 0 } },
  "phases": [
    { "id": "seize",  "time": "200 ms" },
    { "id": "finish", "time": "500 ms",
      "do": [ { "strike": { "attack": "throat", "target": "$victim", "part": "neck", "sure": true } } ] },
    { "id": "settle", "time": "300 ms" } ] }
```

- `requires_at_start` (03 §6.1) is what makes it a takedown and not an
  attack: the victim does not know, or cannot resist. It is tested only
  when the attempt is made. The player is not told beforehand whether an
  unseen guard has in fact noticed them.
- Being in the forced role holds the victim's channels, so the victim has
  no defence.
- `"sure": true` skips aim. The victim is being held still.
- The strike is still a strike. It is delivered to the neck as a hit and
  the body model decides the result. A gorget is a layer.
- The action names its cutscene (12 §8.1).

A takedown on something whose neck a blade cannot cut is a mistake the
attacker is locked into for a second. Previews (section 13) exist so it
need not be made blind.

As built (package C8, decision D-816), in `clash_sim::combat::takedown`
and the action runtime's joint actions (03 §8 as built). Nothing is a
takedown to the engine: a joint action whose victim's role is forced,
a `sure` strike, and a `requires_at_start`. An attempt on a victim that
does not satisfy it is interrupted at once with
`engine:action_start_condition`, before anything is struck or cued. The
`cutscene` field of an action (03 §6.1) raises, as the action begins
and once its `requires_at_start` has held:

| Event | Subjects | Fields |
|-------|----------|--------|
| `cutscene_cued` | `self`, who started it; `other?`, the participant of its second role | `cutscene` (`string`), as written; `action` (`string`), the qualified id; `id` (`int`), the running action's number; `participants` (`list<entity>`), everyone, in the order of the roles |

`entity.helpless` (`bool`, reads truth) is whether it cannot resist:
its mind is not awake, or someone grips it with a pin. The bundled
takedowns (`mods/core/data/combat/takedowns.json`) are `knock_out` (a
sure blow of the fist to the head, `strength × 15` joules) and `choke`
(a trait, `choked`, that raises pain 25 a second for four seconds), both
with `requires_at_start`
`not (perceives($victim, $attacker) ?? false) or $victim.helpless`; and
a steel `gorget` worn over the head, which stops the fist.

## 11. Ranged weapons

### 11.1 Projectiles

`shoot` launches a body (02 §15) carrying a striking surface.

```json
{ "shoot": { "from": "self", "at": "aim_point(self, $target, $part)",
             "projectile": "musket_ball", "speed": "weapon.muzzle_speed",
             "spread": "weapon.spread + shake(self)", "count": 1 } }
```

```json
{ "type": "projectile", "id": "musket_ball",
  "mass": "0.03 kg", "area": 2.4e-4, "drag": 0.5,
  "surface": { "kind": "pierce", "area": 2.4e-4, "give": 0.01, "span": 0.017 } }
```

- **Direction** is toward the aim point, turned by a random angle drawn
  from `spread` (radians). The shot goes where it goes.
- **Flight** is the world's: gravity, drag, the medium (11 §4). An arrow
  arcs. A bullet in water stops within metres.
- **Elements** in the way are struck by the body impact rule (02 §15.2).
  A projectile with enough energy and intensity goes through and carries
  on, slower. Cover is whatever the material in between can stop.
- **Creatures** in the way are struck at the height the projectile is at.
  Body parts have a height **band** (06 §6.1); the part hit is one whose
  band contains that height, by exposure. The hit's energy is the
  projectile's kinetic energy; its surface is the projectile's.
- **Defences** apply if in force: a shield interposes, a moving target is
  not where the shot was sent.
- **Passing through** a creature, the projectile continues with what exits.
- **Near misses** raise `projectile_passed` for anyone it flew close to.
  What being shot at does to a person is content.

Thrown things are projectiles that are items: a knife, a rock, a spear, a
car. `throw` launches the item itself with its own mass and surfaces.

A projectile may also drive and steer itself:

```json
{ "type": "projectile", "id": "seeker_rocket",
  "mass": "6 kg", "area": 0.008, "drag": 0.3,
  "thrust": { "accel": 120, "burn": "2 s" },
  "guidance": { "by": "seeker", "sense": "heat", "cone": 40, "range": 300, "turn": 1.2 },
  "on_impact": [ { "release": { "kind": "blast", "energy": "3 MJ" } } ] }
```

- `thrust` accelerates it along its path for the burn time. After that it
  is ballistic.
- `guidance` turns its path toward a point, by up to `turn` radians a
  second. It is still a body: it is slowed by drag, stopped by walls and
  can be intercepted (section 11.6).

| `by` | The point it steers for |
|------|-------------------------|
| `point` | A place fixed at launch |
| `seeker` | The strongest source of the named signal (15 §3) within its cone and range. It follows what it can sense, so a hotter decoy draws it off and a target behind a wall is lost. |
| `launcher` | Where the launcher believes the target is (15 §7), for as long as the launcher keeps a guiding action going. When that ends it flies on unguided. |
| `return` | The launcher, once it has passed its target or its range |

`on_impact` runs when it strikes or when its fuse time passes.

### 11.2 Time of flight

A projectile is on the timeline. `shoot` may name a window: its kind, and
whom it is for, by default whoever was aimed at. The window opens when
the shot is fired and closes when it arrives. Its `$window.body` is the
projectile, a subject of kind `body` (05 §5.1), which `arrest` and
`intercept` act on and the `projectile_*` events carry.

| Shot | Speed | Across 50 m | A person who needs 250 ms to notice |
|------|-------|-------------|--------------------------------------|
| Thrown rock | 25 m/s | 2 s | Sees it coming |
| Arrow | 60 m/s | 0.8 s | May react |
| Musket ball | 400 m/s | 125 ms | Cannot |
| Rifle bullet | 900 m/s | 55 ms | Cannot |

Someone who notices in a millisecond can respond to all four. Nothing in
the engine says bullets cannot be dodged. The arithmetic says who can.

### 11.3 Instant shots

With `"speed": "instant"` nothing flies. A ray is cast along the direction
and what it meets is struck at once: elements by packet, creatures by hit,
continuing with what passes. Beams and anything too fast for flight time to
matter use this. So does shooting in warm tracts (03 §10).

### 11.4 Aiming

`aim_point` and `spread` are content. The engine supplies helpers:

| Helper | Gives |
|--------|-------|
| `lead_point(self, target, speed)` | Where the target will be, from where and how fast `self` believes it to be moving (15 §7) |
| `ballistic_dir(from, to, speed)` | The direction, at the elevation needed to reach a point under gravity |
| `band_point(target, part)` | A point at the height of a part |

Because the default aims at a point and the shot then disperses, range,
target size, movement and cover all matter without a formula for any of
them.

### 11.5 Weapons

A ranged weapon is an item, and usually an assembly (16 §11).

| Weapon | Built from |
|--------|-----------|
| Firearm | A fire action that takes the next round from the magazine. Projectile and energy from the round; speed from energy, barrel efficiency and projectile mass. |
| Bow, sling | A held draw phase that charges; energy from the archer, capped by the bow |
| Automatic fire | A phase with a tick that shoots and spends each time |
| Scopes, stocks, suppressors | Modules that modify spread, noise (15 §3.2) and so on while fitted |
| Energy weapon | A cost in a power store; an instant shot, or an energy release shaped as a beam (02 §13) |
| Grenade, shell | A projectile whose impact or timer triggers an energy release |
| Flame, spray | A cone of field (02 §16) |
| Spell | Any of the above with a different pool |

**Ammunition** is items. A round names what it fires:

```json
{ "type": "item", "id": "rifle_round_ap", "tags": ["rifle_round"], "stack": true,
  "round": { "projectile": "ap_bullet", "energy": "3.4 kJ", "count": 1 } }
```

A magazine is a hold (06 §9.2) that accepts rounds by tag and gives them
up in order. Firing takes the next one, launches its projectile with its
energy, and leaves its casing if it names one. Loads may be mixed: every
fifth round a tracer. A quiver and a belt are holds too. Loading is the
verb `load`, which an action or an interaction runs. `count` above one is
shot. A weapon that draws on a store instead (a power cell, a pool) uses
a cost (16 §11.2). Section 11.7 says how this is built.

Recoil is an impulse on the shooter and a rule that widens spread.

Technology is the content of these definitions. The engine does not know
what century it is.

### 11.6 Stopping a shot

Bodies in flight do not collide with each other. Stopping a projectile is
something a character does, as a reaction to its `shot` window, and so only
for those quick enough to have one (section 11.2).

| Way | Mechanism |
|-----|-----------|
| Put something in its path | An interposing defence (section 5.3). The shot lands on the blade or shield as any strike would. By `protects`, in front of someone else. |
| Catch it | A reaction whose effect is `arrest` (02 §15) on a contest it names. The projectile ends in the catcher's hand. |
| Knock it aside or shoot it down | `intercept`: an attack aimed at the projectile itself, by a contest against its size and speed. If it lands, the projectile is given an impulse, or ended. |

Two attacks made at range that are both marked `clashes` meet as any two
do (section 8): a beam against a beam is two strikes with inline surfaces
(08 §7.7).

As built (decisions D-810, D-811):

- **The window.** `shoot` and `throw` given `window` (the id of a
  `window_kind`) open one when the shot leaves (`Sim::open_window`),
  named for the entity aimed at, or for those with bodies standing in
  the cell aimed at; none for an instant shot or one aimed along a
  direction. It stays open for the time the shot is expected to take,
  its speed over the distance to the aim point, and closes then, or
  once nothing it was opened for is in flight. Its candidates notice it
  as any window's (03 §7.2): only those who perceive the shooter, after
  their `notice`, on their own clock.
- **Defences meet shots.** Before a shot strikes a creature, in flight
  or by ray, the defences in force for it are tried as at a strike's
  step 5, the shooter as the attacker; `strike_defended` and
  `defence_resolving` name the projectile's qualified id as `action`
  and `shot` as `attack`. One that avoids lets the shot fly by at full
  speed. One that interposes turns its share aside, and each surface it
  puts in the way takes a packet at the projectile's own intensity,
  over the projectile's contact: the point meets the shield, as it
  would meet a wall (a blow is spread by the surface, D-712; a shot is
  not). What passes goes on into the body.
- **Leaving.** A dodge that moves the defender out of the shot's path
  before it arrives is missed by the world: nothing else is needed.
- **Catching** is the verb `catch_shot`, run by a reaction:

| Verb | Arguments | Returns | Does |
|------|-----------|---------|------|
| `catch_shot` | `who: entity?` (default `self`), `window: window?`, `reach: real?` (metres; default 1.5) | `bool`, whether anything was caught | Of what the window was opened for still in flight, the projectiles in the order fired and then an item thrown, the first whose body is within `reach` of the middle of `who`'s body is taken out of the air where it is (`arrest`, 02 §15); its window closes once nothing of it flies. An item thrown is taken into `who`'s hand; a projectile becomes the item its definition's `item` names, in the hand, or nothing; with no hand free it falls where `who` stands. Nothing within reach is not a refusal. Refused with `ranged.not_in_world`, `ranged.bad_number` and `ranged.bad_argument` |

- `intercept` is not built; projectiles crossing do not interact.

### 11.7 As built

Packages C9 and C10 (decisions D-740 to D-749). Nothing below is a gun,
a bow or a bullet: a projectile is a mass with a surface, a round is an
item that names one, a store is a container that keeps an order, and a
weapon is an item with a `shot` block.

**The `projectile` type.**

```json
{ "type": "projectile", "id": "tracer_bullet", "name": "tracer bullet",
  "mass": "9 g", "area": 4.8e-5, "drag": 0.25,
  "surface": { "kind": "pierce", "area": 4.8e-5, "give": "1 cm", "span": "8 mm" },
  "leaves": { "field": "fire", "amount": 8 } }
```

| Field | Type | Meaning |
|-------|------|---------|
| `name` | text | What a reader calls it |
| `mass` | mass, required, above zero | Its mass |
| `area` | number, m², zero or more | Its cross-section: what drag acts on, and how wide it is to what closes part of a cell (02 §15.2). Default the surface's area |
| `drag` | number, zero or more | Its drag coefficient. Default 1 |
| `restitution` | number, 0 to 1 | How much of its speed it keeps when it is turned back. Default 0: what it does not go through holds it |
| `surface` | object, required | What it strikes with: `kind` (`id<damage_type>`), `area` (m², above zero), `give` (length, above zero), `span` (length, default 0) |
| `leaves` | object | A field it leaves where it comes to an end: `field` (`id<field>`), `amount` (levels, 1 to 255, default 1) |
| `on_impact` | effects | Run once, at the first thing it strikes, with `self` the one who fired it, `here` the cell and `target` the creature struck (null for an element); it ends there |
| `item` | `id<item>` | What it is as an item once it is caught in flight (section 11.6, `catch_shot`): an arrow caught is an arrow in the hand. Left out, one caught becomes nothing |
| `thrust` | object | A drive along its path (02 §15): `accel` (m/s²), `burn` (time) |
| `guidance`, `fuse` | | Read and carried: not built |

**Items.** Three blocks of the `item` type (06 §9.1) serve ranged
combat:

| Block | Field | Type | Meaning |
|-------|-------|------|---------|
| `round` | `projectile` | `id<projectile>`, required | What it launches |
| | `energy` | energy, required, above zero | The energy each projectile is given, before the weapon's efficiency |
| | `count` | int, at least 1 | How many projectiles it launches at once. Default 1 |
| | `casing` | `id<item>` | What it leaves where it was fired from |
| `shot` | `spread` | number, radians | The standard deviation of each of a shot's two angles of dispersion |
| | `efficiency` | number | The share of a round's energy that goes into its projectile |
| | `noise` | number | How loud a shot is, in the units of the sound kind content uses |
| | `flash` | number | How bright a shot is |
| | `energy` | energy | The most it puts into a shot of its own: a bow at full draw |
| | `fits` | list of tags | What may be fitted to it: an item that carries one of them, one of each |
| `holds` | `order` | `first_in` or `last_in` | Makes a container a **store**: it gives up what is stowed in it in the order it was put in, first or last. Left out, a container keeps no order |

A shot through an item takes the **product** of the item's own `shot`
number and those of everything fitted to it (held `fixed` in it) that
states it; a number none states is null. So a rifle's `spread` is in
radians and a scope's `"spread": 0.3` narrows it to three tenths, and a
suppressor's `"noise": 0.15` takes most of the sound out of it. They give
it while they are fitted and not once they are taken off.

A store keeps what is stowed in it in the order it was put in. An equal
round put in joins only the stack put in just before it, so that two
kinds loaded in turn stay in turn; a member split off a stack keeps the
stack's place. An item's **feed** is the item itself if it is a store,
else the first store fitted to it, in order of id.

**The effects.** Each is a verb (05 §10.7), written in any effect list.

| Verb | Arguments | Returns | Does |
|------|-----------|---------|------|
| `shoot` | `from: entity?` (default `self`), `at: any?`, `dir: any?`, `part: part?`, `height: real?`, `projectile: id<projectile>?`, `store: entity?`, `with: entity?`, `energy: real?`, `speed: any?`, `spread: real?`, `count: int?`, `noise: real?`, `flash: real?`, `lob: bool?`, `range: real?`, `window: id<window_kind>?` | `int`, the projectiles fired | Below; with `window`, opens a `shot` window (section 11.6) |
| `throw` | `who: entity?` (default `self`), `what: entity`, `at: any?`, `dir: any?`, `part: part?`, `height: real?`, `energy: real?`, `speed: real?`, `spread: real?`, `surface: string?`, `lob: bool?`, `window: id<window_kind>?` | null | Throws an item `who` holds, or a creature `who` grips (section 9.4); with `window`, opens a `shot` window (section 11.6). An item: it is put down where `who` stands and launched as a body that is the item (owner the entity), with its own mass and cross-section, from the height a shot leaves. It strikes what it meets with its surface named `surface` through the same delivery as a projectile's; with none, with the world's blunt blow. Raises `thrown` |
| `load` | `who: entity?` (default `self`), `into: entity`, `from: entity?`, `count: int?` | `int`, the rounds loaded | Puts rounds into the feed of `into`, one member at a time by `put_in`: from `from`, a container (its rounds in order of id) or a stack of rounds itself; with none, from what `who` carries at any depth, in the order `carried` gives. Only rounds the store accepts, as many as fit, at most `count` |
| `fit` | `what: entity`, `into: entity` | `entity` | Fits an item to another: it is held `fixed` in it. Taking it into a grasp takes it off |

`shoot`, in order:

1. Where it leaves from: the middle of the shooter's cell, `height`
   metres above its footing, or 0.8 of its body's height
   (`SHOT_HEIGHT_SHARE`), or for something with no body the middle of
   the storey.
2. Where it is aimed: along `dir` (a list of three numbers); or at `at`:
   an entity, at the middle of the band of `part`, or `height` above its
   footing, or the middle of its body; a cell, `height` above its floor
   or level with where the shot leaves; a point, a list of three numbers
   in metres in the shooter's grid.
3. For each of `count` shots (1 to `shoot.max_count`): with a `store`,
   the next round of its feed, its projectile (or `projectile`, if
   given) and its energy; with none, `projectile`. A store that runs dry
   part way fires what it had.
4. Its energy: `energy`, or the round's times the efficiency of `with`
   (its `shot` product, 1 where none is stated). Its speed: `speed`, or
   `√(2 E / m)`.
5. Its direction: at the aim point, or with `lob` along the lower arc
   that comes down on it under the gravity where it leaves
   (`ballistic_dir`; where no arc reaches, at 45 degrees). Each
   projectile of a round is then turned by dispersion: across and up by
   two angles each drawn from a normal distribution whose standard
   deviation is `spread` (or the spread of `with`, or none), four
   numbers drawn from the stream `shots` for each projectile whatever
   the spread.
6. It is launched as a body (02 §15) whose owner is a `Projectile` with
   the definition's surface and a tag the simulation keeps, on the
   shooter's planes, with the definition's cross-section, drag,
   restitution and thrust. With `speed` `instant`, a ray is cast instead
   (section 11.3, below). `projectile_fired` is raised for each, with
   `noise` and `flash` (the arguments, or the `shot` products of `with`,
   or 0).
7. The round is spent (`item::spend`: a stack loses one member, the last
   is unmade) and leaves its casing in the shooter's cell.

It is refused, having changed nothing, with `ranged.not_in_world`,
`ranged.too_many`, `ranged.empty`, `ranged.no_projectile`,
`ranged.no_energy`, `ranged.no_aim`, `ranged.not_a_store` and
`ranged.bad_number`, and with the world's refusal of the launch.

**In flight.** What the world says of a projectile's body, or of an item
thrown, the simulation answers:

| The world says | The simulation |
|----------------|----------------|
| `body_hit_occupant` | In place of the blunt blow of 06 §7.4: the creature is struck at the height above its footing that the body met it, on a part whose band holds that height (06 §7.1, `HitAt::Height`), by a hit of the body's whole energy with the projectile's surface, through what it wears and its body's layers (`Sim::hit`, as section 6.1). The hit has no direction, so that the body wiring does not also carry its exits to the block behind (06 §7.4): what exits flies on, the body given the speed that energy has along its way (`WorldCtx::impulse`). A creature with no part at that height is passed by at full speed; an item is struck as an item is (06 §9.1). With nothing left a projectile is lodged where it is and ends. `projectile_hit` |
| `body_impact` | `projectile_hit` at the cell, with what it lost there. A projectile that did not go through and has no restitution is lodged there and ends: a bullet in a wall, an arrow in the ground |
| `body_landed`, `body_left_loaded` (gone) | The shot ends |

At each of these, the stretch of its path since it was last known (where
it was fired from, or last struck something) is looked along: everyone
with a body whose middle lies within `projectile.near_miss` of it, other
than the shooter and whoever it struck, is told once by
`projectile_passed`. A projectile's `on_impact` runs at the first thing
it strikes, and it ends there. Where a projectile ends it leaves the
field it names.

A thing with a body collides with bodies in flight: a template that
gives a body gives its occupant the flag `COLLIDES` (02 §15.2).

**Instant shots** (section 11.3). One ray is cast from where the shot
leaves, along its direction, up to `range` (default
`shoot.instant_range`), through the cells and every creature with a
body, the shooter aside. In order: an element closing the whole of the
ray's path, or the share it closes on a draw from `shots`, takes a
packet of what is left at `E / (area × give)` (`damage_along`); one it
destroys passes `passed` on, and anything else ends the ray. A creature
is struck as in flight; what exits goes on. `projectile_hit` with
`instant` true and no `body`.

**Helpers** (section 11.4).

| Function | Arguments | Gives |
|----------|-----------|-------|
| `ballistic_dir` | `from: any`, `to: any`, `speed: real` | `list<real>?`: the direction, of length one, along the lower arc from `from` to `to` at that speed under the gravity where `from` is, with no drag. An entity is taken at the height a shot leaves it from, a cell at its floor, a list of three numbers as a point. Null where no arc reaches. Reads truth |
| `band_point` | `target: entity`, `part: part?` | `list<real>?`: the point, in metres in its grid, at the middle of the band of a part where the target stands; with no part, the middle of its body. Reads truth |

`lead_point` waits for believed velocities (15 §7).

**Properties**, all of the kind `entity`, readable of one's own things in
a mind context (05 §7.3):

| Property | Type | Is |
|----------|------|----|
| `entity.shot(name: string)` | `real?` | The `shot` product of an item and what is fitted to it: `spread`, `efficiency`, `noise`, `flash` or `energy` |
| `entity.feed` | `entity?` | The store an item fires from |
| `entity.rounds` | `int` | The members of the rounds in its feed; zero for none |
| `entity.next_round` | `entity?` | The round its feed gives up next |
| `entity.store_of(tag: string)` | `entity?` | The first store, itself, then what it holds in any way but enclosed (in order of id) and what is fitted to those, that accepts the tag: a quiver worn, a rifle's magazine |
| `entity.wielding(tag: string)` | `entity?` | The first thing it holds in a grasp, in order of id, that carries the tag; any, for an empty one |

**Events.** Each also binds `actor` and `instigator` from its cause.

| Event | Subjects | Fields |
|-------|----------|--------|
| `projectile_fired` | `self`, the shooter; `target?`, whom it was aimed at; `weapon?` | `projectile` (`string`), its qualified id; `body` (`int?`), its body, null for an instant shot; `speed` (`real`); `energy` (`real`); `noise`, `flash` (`real`); `instant` (`bool`). Raised for each projectile, before any of it strikes. |
| `projectile_hit` | `self?`, the shooter, while it exists; `target?`, the creature or item struck; `here`, the cell | `projectile`, `body`; `part` (`part?`), the part struck; `energy` (`real`), joules it arrived with; `intensity` (`real`); `absorbed` (`real`), what what it struck took; `passed` (`real`), what it went on with; `instant` (`bool`). An item thrown has an empty `projectile`. |
| `projectile_passed` | `self`, the one it passed; `shooter?` | `projectile`, `body`; `distance` (`real`), metres from `self`'s middle, zero for one passed by at a height where nothing of it was. Once for each projectile. |
| `thrown` | `self`, the thrower; `what`, the item; `target?` | `speed` (`real`); `energy` (`real`). |

**Refusals.**

| Reason | When | Values |
|--------|------|--------|
| `ranged.empty` | The store holds no round to give up next | `store` |
| `ranged.no_projectile` | No projectile that is loaded, and no store to take a round from | — |
| `ranged.no_energy` | Neither a speed nor an energy, and no round | — |
| `ranged.bad_number` | An energy, spread, noise or flash below zero or not finite; a speed not above zero; a thing thrown with no mass | `what`, `value` |
| `ranged.no_aim` | Nothing to aim at: no `at` or `dir`, a target not in the world, another grid, the point it leaves from | `at` |
| `ranged.not_in_world` | The shooter or thrower is not standing in the world | `entity` |
| `ranged.too_many` | A count of none, or above `shoot.max_count` | `count`, `limit` |
| `ranged.not_a_store` | What is fired from or loaded is no store and has none fitted | `store` |
| `ranged.nothing_to_load` | No rounds the store takes where `load` looked | `store` |
| `ranged.cannot_fit` | None of the item's tags is in the other's `shot.fits`, or one carrying that tag is fitted already | `item`, `into` |
| `ranged.not_held` | A thing to be thrown is not held by the thrower | `item`, `who` |
| `ranged.no_surface` | A thing is thrown striking with a surface it does not have | `item`, `surface` |
| `ranged.bad_argument` | A verb's argument is missing or of the wrong kind | `verb`, `arg` |

**Settings.**

| Setting | Meaning |
|---------|---------|
| `projectile.near_miss` | Metres: how close a shot must pass to raise `projectile_passed`. Default 2. |
| `shoot.max_count` | The most shots one `shoot` fires. Default 1000. |
| `shoot.instant_range` | Metres an instant shot goes when it gives no `range`. Default 1000. |

**Seams.** The `shot` window, defences against shots and catching are
section 11.6's (`bullet_time`, `caught_arrow`); `intercept` is not built.
`guidance`, `fuse` and `projectile_lost_lock` are not built (`heat_seeker`);
a timed charge is an item thrown with a rule's `after`. A weapon that
draws on a power store (16 §11.2) waits for assemblies (package G9).
Recoil is content's rule; the bundled content has none. `$window.body`
waits for a subject kind `body`: events carry the body's number.

**Bundled.** `mods/core/data/combat/ranged.json` holds the projectiles
`pistol_bullet`, `hollow_bullet`, `rifle_bullet`, `tracer_bullet` (it
leaves fire) and `arrow`; the contest `marksmanship` and the functions
`marksman`, `unsteadiness`, `draw_power` and `throw_energy`; and the
actions `shoot` (a firearm held, `count` shots at whoever the shooter
believes is there, unavailable while it is empty), `aim_shot` (slower,
under half the spread), `reload` (`load` from whatever the shooter
carries), `draw_and_loose` (a held draw that adds 50 ms a tick, up to
1.5 s; energy `draw_power × seconds`, capped by the bow's `shot.energy`;
the arrow is lobbed) and `throw` (what is held tagged `throwing`: a
blade strikes with its point; a `charge` bursts three seconds after it
leaves the hand, by `after`). `mods/core/data/combat/arms.json` holds a
`pistol` and a `rifle` with a `pistol_magazine` and a `rifle_magazine`
(stores, `first_in`), the rounds `pistol_round`, `pistol_round_hollow`,
`rifle_round` and `rifle_round_tracer` with a `casing`, a `scope`
(`sight`, spread 0.3), a `suppressor` (`muzzle`, noise 0.15, flash 0.1,
efficiency 0.97), a `bow` (80 J), `arrow`s and a `quiver` (a store worn),
a `throwing_knife` and a `charge`. `mods/core/data/perception/shots.json`
gives `projectile_fired` off as a sound of its `noise` and a sight of its
`flash` about the shooter, and `projectile_hit` as a sound. The bundled
`shoot`, `aim_shot`, `draw_and_loose` and `throw` open a `shot` window.
`mods/core/data/combat/shots.json` (decision D-818) holds the window kind
`shot` (noticed in `2 / wits` seconds of the candidate's own time, reach
3 m); the `arrow` projectile's `item`, and a grip for the arrow item;
the contest `catching`; the reactions `catch` (a hand that tries at each
5 ms tick to close on what the window was opened for, its contest
`catching` of `fighting(self)` against 12 drawn once), `step_aside` (a
step across the line from the window's source, `aside_from`, for shots
and blows) and `raise_shield` (a held shield's face, put in the way of
shots and blows, `handles` all); and the rules to attach
`catch_on_notice`, `step_aside_on_notice` and `shield_on_notice`.

## 12. Stances, styles and combinations

None of these are engine concepts. They are built as follows.

| Idea | Built from |
|------|-----------|
| **Stance** | A trait in an exclusive `group` (06 §5), so taking one drops another. It carries modifiers, a guard held up, and the variables that reaction rules read to decide what the character does unprompted when attacked. Changing stance is an action. |
| **Style** | A trait that grants a set of techniques and stances, with a track (06 §4.3) for progress |
| **Combination** | Phases that branch on `$landed` (03 §6.4), and recovery phases whose `cancel` lists what they may be abandoned for |
| **Feint** | An action whose window announces one attack and whose phases deliver another, or none |
| **Counter** | A reaction that is an attack. If it lands first, its stagger interrupts the attacker. If it lands together, a clash. If later, both land. |
| **Disarm, trip, bind** | Attacks with no energy whose `on_land` effects drop an item, unbalance, or seize |
| **Exertion** | Pools (06 §4.2) drained by actions' start effects |
| **Improvised weapons** | Any item has mass and can be given a default blunt surface by content |

The bundled content supplies a basic set of each. They are the defaults
that make one-key fighting work, and the first thing a combat mod replaces.

As built (packages C11 and C12, decision D-817): nothing new in the
engine but exclusive trait groups, which traits already have (06 §5).
`mods/core/data/combat/stances.json` holds the stance traits
`aggressive` (strength × 1.15) and `defensive` (strength × 0.9; every
blow noticed in time is answered with a `block`), both of the group
`fighting_stance`; the action `take_stance` (a trait given as the shift
ends); the functions `auto_part()` (null: the blow lands where the
body offers) and `auto_effort(guarded)` (1, or 0.7 for one
guarded), which the bundled `punch`, `kick`, `slash`, `stab`, `shove` and
`whirl` take as their defaults, given `self.has_tag('defensive')` so that
they stay safe for belief; and the rule `stagger_breaks_blow` (a stagger
of severity 0.5 or more interrupts the staggered's actions tagged
`strike`; off with the setting `core:staggering` false).
`mods/core/data/exertion.json`, beside the `stamina` pool of
`traits.json`, holds `exertion` (a blow that lands from someone
aggressive costs 5 stamina) and `winded` (below a quarter, the stance is
dropped), for those who have the pool. `mods/core/data/combat/grips.json`
holds the contest `grapple`, the functions `drag_limit` (12 kg for each
point of strength) and `heave_energy` (30 J for each point), the actions
`grab` (one hand, pinning the part named, held up to 20 s), `throw_person`
(both hands, thrown away from the thrower), `drag`, `human_shield` (both
hands; its `defence` puts the held in the way of every blow and shot) and
`break_free` (strength against the holder's, ending in `release_grip`);
the goal `apprehend` (a grab on someone held hostile within two metres),
which an optional patch adds to the `guard` mind, whose `confront` then
strikes with the other hand. Two bundled guards, one an outlaw, fight
each other to a conclusion with nothing outside `core`.

## 13. Previews

A player choosing where to cut, and a character deciding whether to fight
at all, both need to know what a strike is likely to do.

`strike_preview(self, attack, target, part, effort)` is a pure function. It
draws no random numbers and changes nothing. It returns:

| Field | Meaning |
|-------|---------|
| `odds` | Chance of landing on the part |
| `depth` | Layers it would breach |
| `disables`, `severs` | Whether the part would be disabled or severed |
| `passes` | Energy that would exit |
| `speed` | Push speed, and whether that staggers or launches |
| `recoil` | Whether the striking surface would be damaged |

A preview runs hook handlers **dry**: what a handler would change about
the strike or the hit is applied, and everything else it would do is
skipped. Only handlers on things `self` knows the target has are run
(15 §7.1). So a barrier that works by a rule on `hit_incoming` shows in
the preview of anyone who knows it is there, and not otherwise. Handlers
that draw random numbers are skipped.

It is computed from what `self` could know: the target as it appears
(15 §8.1) and what it is visibly wearing. Something that looks like an
ordinary man and is not previews as an ordinary man.

Uses: the precise targeting picker (12 §9.3) shows a preview per part;
default effort functions choose the most that does not maim; the AI layer
(09) weighs techniques; and bulk resolution of fights away from attention
uses expected outcomes in place of full resolution.

As built (package C11, decision D-815), in `clash_sim::combat::preview`:

| Function | Arguments | Gives |
|----------|-----------|-------|
| `strike_preview` | `who: entity`, `action: any` (an action's id), `target: entity`, `part: any` (of the kind `part`; optional), `effort: real` (optional, default 1), `attack: string` (optional, for an action with several) | `map<any>?`: `in_reach` (`bool`), whether the target is within the attack's reach now; `odds` (`real`), the chance it lands on the part aimed at, the `odds` of its aim's contest, 1 for one that does not aim; `odds_near` (`real`), on it or a part beside it; `energy` (`real`), joules of harm at the effort, less the push; `intensity` (`real`); `push_speed` (`real`), unbraced; `staggers` and `launches` (`bool`), by the settings of section 7. Null for an action with no such attack, or nothing to strike with |

It finds the attack and what strikes as the strike would, and
evaluates the attack's numbers with the attacker, the weapon or limb and
the target bound and the locals a command would give (`$target`,
`$part`, `$effort`; the second role of a joint action is the target),
through a host that may not draw: invariant C5. It reads the target as
it is, not as it appears, and is not safe for belief. Not built: `depth`,
`disables`, `severs`, `passes` and `recoil`, which need a dry run of the
body model; handlers run dry; appearances.

## 14. A worked exchange

Illustrative numbers.

**A swing and a parry.** A starts `slash` at B. Windup is 300 ms.

| Time | Event |
|------|-------|
| 0 ms | Window opens for B |
| 180 ms | B notices (B's notice delay; B perceives A) |
| 180 ms | A rule on B's stance starts `parry`; its guard begins at 260 ms |
| 300 ms | Impact. In reach. No clash. Aim margin +0.4: on the arm. Parry covers it; control margin favours B. |

The slash carries 180 J. B can hold 600 J and deflects 0.9, so 162 J is
turned aside. The remaining 18 J lands on the flat of B's blade at an
intensity steel does not notice. Nothing reaches B.

**The same, with A a hundred times stronger.** The slash carries 400 kJ.
B still holds 600 J, so 540 J is turned aside. The other 399 kJ lands on
B's blade, which absorbs what a few cubic centimetres of steel can and
breaks. Nearly all of it continues to the arm behind, breaches every layer
at a span longer than the arm's girth, severs it, and exits. The push share
launches B.

B did everything right. Decision D-034 is not a slogan.

**Had B been slower.** With a notice delay of 320 ms the window closes
before B knows it opened, and there is no parry at all.

## 15. Detail

| Level | Fighting |
|-------|----------|
| Hot | As specified |
| Warm | Entities are stepped in bulk (03 §10). The AI layer resolves their exchanges from previews. Shots are instant. |
| Cold | Battles are events of the cold simulation (09) with casualties and damage as outcomes |

A warm entity attacked by a hot one is promoted first (03 §10).

## 16. Modding surface

**Definition types**: `contest`, `projectile`. Blocks `attacks` and
`defence` and the field `cutscene` on actions (03 §6.1); `surfaces` and
`band` on body parts; `give` and `span` on surfaces; `group` on traits;
`thrust`, `guidance`, `on_impact` and `item` on projectiles; `round` on
items; `contact` on elements.

**Events**: `strike_landed`, `strike_missed`, `strike_stopped`,
`strike_defended`, `strike_recoiled`, `staggered`, `launched`,
`clash_resolved`, `seized`,
`grip_broken`, `thrown`, `cutscene_cued`, `projectile_fired`,
`projectile_hit`, `projectile_passed`, `projectile_lost_lock`,
`contact_struck`.

**Hooks**: `strike_resolving`, `defence_resolving`, `knockback_incoming`,
`clash_resolving`, `seize_resolving`.

| Hook | Runs before | A handler may |
|------|-------------|---------------|
| `strike_resolving` | Step 1 of a strike | Change its energy, target, part and effort; cancel |
| `defence_resolving` | A defence is applied at step 5 | Change how much it turns; make it fail |
| `knockback_incoming` | A push moves its target (section 7) | Change the speed and direction; cancel |
| `clash_resolving` | Two strikes are resolved together | Change either side's energy; decide the outcome |
| `seize_resolving` | A grip is taken (section 9.1) | Make it fail; change what it pins |

`shove` is an attack with no surface that only pushes. `drag` moves a
gripped thing with the one gripping it.

**Effects**: `strike`, `shoot`, `throw`, `seize`, `release_grip`, `drag`,
`shove`, `wrench`, `intercept`, `catch_shot`.

**Functions**: `contest`, `odds`, `strike_preview`, `lead_point`,
`ballistic_dir`, `band_point`, `closing_energy`, `aside_from`.

**Queries**: `grips` (of or on an entity), `defences` (in force on an
entity), `in_arc`.

**Properties**: `self.gripping`, `self.gripped_by`, `self.defending`,
`self.helpless`, `self.wielding_surface(surface)`, `self.reach`; on
running actions, `attack(id)` with its current `energy`
and time to impact. In a mind context these are readable only for the
mind's own actions.

**Settings**: `clash_window`, `lock_ratio`, `combat.stagger_at`,
`combat.launch_at`, `projectile.near_miss` (how close a shot must pass to
raise `projectile_passed`), `shoot.max_count`.

### 16.1 As built

Packages C1, C2, C3 and C5 (contests; surfaces and delivery; aim; push)
register what follows. Every event also binds `actor` and `instigator`
from its cause: for a strike, the attacker.

| Event | Subjects | Fields |
|-------|----------|--------|
| `strike_resolving` (hook) | `self`, the attacker | `action` (`string`), the qualified id of the action; `attack` (`string`), the attack's id; `target` (`entity?`), whom it strikes, null for a cell; `part` (`part?`), the part aimed at; `energy` (`real`), joules at full effort; `effort` (`real`). A handler may change `target`, `part`, `energy` and `effort`, or cancel: nothing is struck. |
| `strike_landed` | `self`; `target?`, the one struck; `here?`, the cell, for a strike at a cell | `action`, `attack` (`string`); `part` (`part?`), the part it landed on; `kind` (`id<damage_type>`); `energy` (`real`), joules delivered as harm, the push taken out; `intensity` (`real`), pascals; `absorbed` (`real`), joules the target's layers and what it wears, or the element, took; `passed` (`real`), joules that went on beyond it; `returned` (`real`), joules that came back as recoil; `push` (`real`), joules set aside to push; `effort` (`real`); `margin` (`real?`), the margin of its aim, null where it did not aim. |
| `strike_missed` | `self`; `target?` | `action`, `attack` (`string`); `why` (`string`): `reach`, `aim`, `defence` (a defence that avoids succeeded) or `nothing` (a target with no body that is not an item); `margin` (`real?`), of the aim, or of the defence's contest. |
| `strike_stopped` | `self`; `target?` | `action`, `attack` (`string`); `code`, `message` (`string`): why nothing was delivered. A handler of `hit_incoming` cancelled the hit, or what defences put in the way took all of it (`combat.defended`). |
| `strike_defended` | `self`, the attacker; `target`, the one struck; `defender`, whose defence it is | `action`, `attack` (`string`); `defence` (`string`), the qualified id of the defending action; `form` (`string`), `avoid` or `interpose`; `succeeded` (`bool`); `margin` (`real?`), of its contest; `turned` (`real`), joules it turned aside. Raised for each defence tried at step 5, in order. |
| `defence_resolving` (hook) | `self`, the defender; `attacker`; `target` | `action` (`string`), the defending action; `form` (`string`); `succeeded` (`bool`); `margin` (`real?`); `deflect` (`real`); `holds` (`real?`), null for no limit; `brace` (`real`). A handler may change `succeeded`, `deflect`, `holds` and `brace`; cancelling makes it fail. |
| `strike_recoiled` | `self`; `weapon?`, the item that took it | `action`, `attack` (`string`); `part` (`part?`), the part of `self` that took it; `energy` (`real`), joules that came back; `absorbed` (`real`), joules of it the part or the item took. Not raised for an inline surface. |
| `knockback_incoming` (hook) | `self`, the one pushed | `speed` (`real`), metres a second; `dir` (`list<real>`), east, south, up, of length one. A handler may change both, or cancel: nothing moves it. |
| `staggered` | `self` | `speed` (`real`); `severity` (`real`), where the speed lies between `combat.stagger_at` and `combat.launch_at`, from 0 to 1; `dir` (`list<real>`). |
| `launched` | `self` | `speed` (`real`); `dir` (`list<real>`); `added` (`bool`), true if it was in flight already and the push was added to its velocity. |

Grips (package C7, section 9.4) add:

| Event | Subjects | Fields |
|-------|----------|--------|
| `seize_resolving` (hook) | `self`, the holder; `target`, what is to be held | `part` (`part?`), the part to be held; `pins` (`bool`). A handler may change both, or cancel: the grip fails (`combat.seize_failed`). |
| `seized` | `self`, the holder; `target` | `part` (`part?`); `pins` (`bool`); `action` (`int`), the number of the holder's action that maintains it. |
| `grip_broken` | `self`, the holder; `target?`, what was held | `part` (`part?`); `how` (`string`): `ended` (its action ended), `released` (the holder let go), `broken` (the held broke free), `thrown`, `lost` (they came apart). |

Takedowns (package C8) add `cutscene_cued` (section 10). Shots meet the
defences' events with `action` the projectile's qualified id and
`attack` `shot` (section 11.6).

`clash_resolved` and the rest wait for their packages. The properties
`entity.defending` (`bool`, section 5.5), `entity.gripping`,
`entity.gripped_by` (section 9.4), `entity.helpless` (section 10) and
`entity.wielding_surface` are built:

| Property | Type | Is |
|----------|------|----|
| `entity.wielding_surface(surface: string)` | `entity?` | The first thing it holds in a grasp, in order of id, whose definition has a striking surface of that id: the knife or club whose `flat` a parry puts in the way. Readable of oneself in a mind context, so that the bundled `parry` requires one (decision D-819) |

| Reason | When | Values |
|--------|------|--------|
| `combat.no_attack` | A `strike` is written in a list that belongs to no action, names an attack its action does not have, or names none in an action with several | `action`, `attack` |
| `combat.no_target` | A `strike` names neither an entity nor a cell, or a target not in the world | `target` |
| `combat.no_surface` | Nothing answers to the attack's `with` | `action`, `attack`, `with` |
| `combat.nothing_gripped` | The attack strikes with something gripped, and the attacker grips nothing | `action`, `attack` |
| `combat.bad_number` | An energy or a reach below zero or not finite, a push or an effort outside 0 to 1, a lift not finite | `what`, `value` |
| `combat.not_its_part` | The part aimed at is not the target's | `target` |
| `combat.nothing_there` | A strike at a cell found no floor, block or fixture there, or the cell not loaded | `cell` |
| `combat.too_deep` | A strike was resolved inside eight others | `limit` |
| `combat.defended` | What defences put in a strike's way took all of it: carried by `strike_stopped` | `target` |
| `combat.no_action` | `seize` was run outside an action of the holder's | `holder` |
| `combat.out_of_reach` | What was to be seized is further than the holder's body reaches | `target`, `reach` |
| `combat.seize_failed` | A handler of `seize_resolving` made the grip fail | `target` |
| `combat.gripped` | A step by a creature held would take it away from its holder | `holder` |
| `combat.too_heavy` | A step by a holder would drag more than its grip's `drags` | `what`, `mass`, `most` |
| `combat.grip_lost` | A grip's two came apart: the action that kept it is interrupted | `target` |
| `combat.grip_broken` | The held broke free: the action that kept the grip is interrupted | `target` |
| `combat.grip_released` | The holder let go other than by the action that kept the grip, which is interrupted | `target` |
| `combat.not_gripped` | A creature to be thrown is not one the thrower grips | `who`, `what` |

| Setting | Meaning |
|---------|---------|
| `combat.stagger_at` | Metres a second: a slower push moves nothing. Default 0.5. |
| `combat.launch_at` | Metres a second: a push at least this fast launches its target. Default 3. |

The functions `contest` and `odds` are stated in section 4.1,
`strike_preview` in section 13, the effect `strike` in section 3.4, and:

| Function | Arguments | Gives |
|----------|-----------|-------|
| `aside_from` | `who: any`, `from: any`, `right: bool` (optional, default false) | `string?`: the step across the line from `from` to `who`, out of the way of what comes along it (section 5.2, 11.6): `north`, `north_east` and so on, to the left of someone facing straight away from `from`, or to the right. Null for two in one column, or either nowhere. Safe for belief |

The `core` mod (`mods/core/data/combat/fighting.json`) bundles the window
kind `strike`, the contest `melee`, the functions `fighting(who)` and
`evasion(who, from)`, and the actions `punch` (fist), `kick` (heel),
`slash` (a held blade's edge), `stab` (its point), `shove` (both palms,
nearly all push, `knock_down` prone), `smash` (a fist at a cell) and
`whirl` (a held blade's edge in a sweep of 360 degrees with `through`),
each with a windup that opens a `strike` window, an impact and a
recovery. `mods/core/data/life/surfaces.json` gives the humanoid's hands
a `palm`, its feet a `sole` and its head a `brow`, and
`mods/core/data/world/severing.json` flags `cut` as severing.
`mods/core/data/combat/reactions.json` (decision D-714) gives the
`strike` window kind a `notice` of `2 / wits` seconds (a fifth of a
second at wits 10) and a `reach` of 3 m; the contests `control` and
`slip`; the reactions `parry` (the flat of anything held that has one,
deflect 0.9, holds 60 J for each point of strength, brace 0.3;
unavailable, saying why, with nothing such in hand), `block` (a held
shield's face, else the forearm), `dodge` (avoid, with `evasion`) and
`step_back` (a step straight away from the window's source); the guard
`guard`, a held phase; the rule `ask_player_to_react`; and the stances
`parry_on_notice`, `block_on_notice`, `dodge_on_notice` and
`step_back_on_notice`, rules to attach. `mods/core/data/shields.json`
gives the knife and the club a `flat`, the arm a `forearm`, and adds a
`buckler`. Ranged combat (`projectile`, `round`, `shot`, the effects
`shoot` and `throw`, the `projectile_*` events, `ballistic_dir` and
`band_point`) is stated as built in section 11.7; grips in section 9.4;
takedowns and the `cutscene` field in section 10; stances, exertion and
the default part and effort in section 12; `strike_preview` in section
13. Not built yet: `lead_point`, `intercept`, `wrench`, the effect `drag`
(a holder that steps drags what it grips by itself), the queries
`grips`, `defences` and `in_arc`, `entity.reach` and an action's
`attack(id)`, swings meeting the world, contact surfaces (section 6.5),
reach in height (section 4.3) and `closing_energy` (section 3.5).

**Gaps.** Reachable only from Rust for now: every grip there is, and the
grips an action keeps (`Sim::grips`, `GripTable::of_action`), where
content reads only the first a creature holds or is held by
(`entity.gripping`, `entity.gripped_by`) until the query `grips`; who is
locked into a running joint action and the slots each role holds
(`ActionTable::joined`, `held_of`), which content sees only in
`cutscene_cued`; which shots in flight a `shot` window was opened for
(`RangedState::of_window`), which `catch_shot` reads; and the preview as
a struct (`combat::preview::strike_preview`), the same numbers the
function gives.

## 17. Invariants

| # | Invariant |
|---|-----------|
| C1 | Every strike that harms does so through a `Hit` or a damage packet. Combat changes no body or element directly. |
| C2 | The energy delivered, turned aside, pushed, spilled and returned by a strike sums to no more than the energy put into it. |
| C3 | A defence applies only if it is in force at the instant of impact. |
| C4 | Chance enters only through contests, shot dispersion and where on a body a hit falls (06 §7.3), all from the simulation's random stream. |
| C5 | `strike_preview` and `odds` are pure. |
| C6 | Two attacks that clash are resolved once, together. |
| C7 | A grip exists exactly as long as the action maintaining it: it ends when the action ends, for any reason, and a grip ended otherwise interrupts the action (section 9.4). |
| C8 | A projectile's effect on what it meets depends only on its mass, velocity and surface at that moment. |

## 18. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Resolve a strike with one defence | under 8 µs |
| Resolve a five-target arc | under 30 µs |
| Projectile striking a creature | under 5 µs |
| `strike_preview` | under 4 µs |
| A volley of 500 projectiles over 100 m | under 40 ms |

Measured (package C9): the volley, fifty shooters each firing ten pistol
bullets at a wall a hundred metres off, took about 80 ms in a release
build on a loaded shared machine: about twice the target. Nearly all of
it is the world stepping its bodies (02 §15.1, a wake for every cell
crossed); the test is `volley` in `crates/clash-sim/tests/ranged.rs`,
ignored, run with `--release --ignored`.

## 19. Open questions

1. **Projectiles meeting projectiles.** Settled in section 11.6: stopping
   a shot is a reaction. There is no general collision between bodies in
   flight.
2. **Reach in height.** Settled in section 4.3.
3. **Swinging in confined space.** Settled in section 6.4: a swing meets
   whatever stands in its arc.
4. **Gripping things that are not creatures.** Settled in section 9.3 and
   06 §6.6.
5. **Mass battles.** Open. Bulk resolution from previews is sketched in
   section 15. Whether it gives believable results for scene S19 can only
   be answered by testing. (09)
