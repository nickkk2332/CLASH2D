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
      "reach":  "self.reach + weapon.length",
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

| Attack field | Meaning |
|--------------|---------|
| `with` | The striking surface: on a held item, on a body part, on a gripped creature or object, or given inline for a strike made with no physical thing (08 §7.7) |
| `reach` | Metres. The target must be within it at impact. |
| `energy` | Joules at full effort |
| `push` | Share of the energy that moves the target instead of harming it (section 7) |
| `lift` | Upward angle of the push |
| `aim` | How the strike finds its mark (section 4). Absent means it cannot miss. |
| `arc` | For strikes at several targets (section 6.4) |
| `clashes` | Whether this attack can meet another (section 8) |
| `spill`, `spill_kind` | What a locked clash releases (section 8) |
| `on_land`, `on_miss`, `on_stopped` | Effect lists |

Inside an attack's expressions, `weapon` is the item or part supplying the
surface, and `target` is the one being struck.

### 3.2 Surfaces

Items and body parts both carry striking surfaces (06 §9.1, §6.1).

| Field | Meaning |
|-------|---------|
| `kind` | The damage type it delivers: cut, pierce, impact |
| `area` | Contact area, m² |
| `give` | Metres over which the strike is stopped. A hard edge gives almost nothing; a gloved fist gives centimetres. |
| `span` | Metres of contact across the target. An edge's length. What decides whether a cut can sever. |

### 3.3 Effort

Every strike takes an `effort` from 0 to 1. Energy is multiplied by it.

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
its whole energy to the element as a packet (02 §12).

## 4. Aim

### 4.1 Contests

A **contest** compares two numbers, with as much or as little chance as
content wants.

```json
{ "type": "contest", "id": "melee_aim", "form": "difference", "scale": 4, "spread": 1.0, "near": 0.5 }
```

`contest(id, a, b)` returns a **margin**: the scaled difference (or
log-ratio, for `"form": "ratio"`) between `a` and `b`, plus noise of the
given spread. A spread of zero is a pure comparison. `odds(id, a, b)` is
the chance the margin is positive; it draws nothing and is pure.

Contests are the only place chance enters combat, and how much is a number
in a data file.

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
| `covers` | Which strikes it applies to: by direction, by the part they are aimed at |
| `handles` | How many strikes it can deal with while in force; `"all"` for a shield wall |
| `contest` | Whether it succeeds. Absent means it always does. |
| `deflect` | Share of the strike's energy turned aside on success |
| `holds` | The most energy, in joules, the defender can turn aside |
| `brace` | Share of the push the defender resists |

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

### 6.2 Severing and passing through

A part is severed when the damage type is flagged `severs`, every layer is
breached, and `hit.span` is at least the part's girth (06 §7.2). A short
blade cuts deep into a thigh. A long one takes it off.

Energy that exits the far side continues along the strike's direction to
whatever is next: another part, another creature, a wall.

### 6.3 Recoil

Energy a strike delivers that the target neither absorbs nor passes comes
back into the striking surface as a packet of its own.

- Punch a breastplate whose threshold the fist's intensity does not reach,
  and the fist receives the punch.
- Strike stone with a sword and the sword takes the damage.
- Strength great enough to break a wall, in a body not tough enough to
  survive doing it, breaks the arm. Durability is not optional for the
  strong. This is a consequence, not a rule added for it.

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

This is scene S4, and the reason a master cuts down a rank of ordinary foes
is the reason given by the numbers: enough energy on a fine enough edge
that flesh takes little of it.

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
| Below `stagger_at` (default 0.5 m/s) | Nothing |
| Below `launch_at` (default 3 m/s) | `staggered`, with a severity. What staggering does is content: usually it interrupts and unbalances. |
| At or above | The target becomes a body in flight (02 §15) at that speed. Walls, floors and bystanders in the way are dealt with by the world. Injuries on impact arrive as hits (06 §7). |

A shove is an attack whose `push` is nearly 1. A cut's is nearly 0. A
punch from someone who can put a megajoule into it, with a third set aside
as push, sends eighty kilograms away at ninety metres a second. That is
scene S1, and with a tougher target, S2.

Mass is in the denominator. Heavy things are hard to move.

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

## 10. Takedowns

A takedown, an execution or a finishing move is a joint action (03 §8) with
a forced role for the victim.

```json
{ "type": "action", "id": "blade_takedown", "name": "Takedown",
  "roles": { "attacker": { "uses": { "arm": "all" } },
             "victim":   { "uses": { "arm": "all", "legs": "all" }, "forced": true } },
  "requires": "not perceives($victim, $attacker) or $victim.has_trait('helpless')",
  "cutscene": "takedown_from_behind",
  "attacks": { "throat": { "with": { "held": "blade", "surface": "edge" }, "energy": 60, "push": 0 } },
  "phases": [
    { "id": "seize",  "time": "200 ms" },
    { "id": "finish", "time": "500 ms",
      "do": [ { "strike": { "attack": "throat", "target": "$victim", "part": "neck", "sure": true } } ] },
    { "id": "settle", "time": "300 ms" } ] }
```

- `requires` is what makes it a takedown and not an attack: the victim does
  not know, or cannot resist.
- Being in the forced role holds the victim's channels, so the victim has
  no defence.
- `"sure": true` skips aim. The victim is being held still.
- The strike is still a strike. It is delivered to the neck as a hit and
  the body model decides the result. A gorget is a layer.
- The action names its cutscene (12 §8.1).

A takedown on something whose neck a blade cannot cut is a mistake the
attacker is locked into for a second. Previews (section 13) exist so it
need not be made blind.

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

### 11.2 Time of flight

A projectile is on the timeline. A window of kind `shot` opens for the
target when it is fired, and closes when it arrives.

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
| `ballistic_dir(from, to, speed)` | The elevation needed to reach a point under gravity |
| `band_point(target, part)` | A point at the height of a part |

Because the default aims at a point and the shot then disperses, range,
target size, movement and cover all matter without a formula for any of
them.

### 11.5 Weapons

A ranged weapon is an item, and usually an assembly (16 §11).

| Weapon | Built from |
|--------|-----------|
| Firearm | A fire action with a cost of one round from a magazine store. Projectile and energy from the ammunition; speed from energy, barrel efficiency and projectile mass. |
| Bow, sling | A held draw phase that charges; energy from the archer, capped by the bow |
| Automatic fire | A phase with a tick that shoots and spends each time |
| Scopes, stocks, suppressors | Modules that modify spread, noise (15 §3.2) and so on while fitted |
| Energy weapon | A cost in a power store; an instant shot, or an energy release shaped as a beam (02 §13) |
| Grenade, shell | A projectile whose impact or timer triggers an energy release |
| Flame, spray | A cone of field (02 §16) |
| Spell | Any of the above with a different pool |

Recoil is an impulse on the shooter and a rule that widens spread.

Technology is the content of these definitions. The engine does not know
what century it is.

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

It is computed from what `self` could know: the target as it appears
(15 §8) and what it is visibly wearing. Something that looks like an
ordinary man and is not previews as an ordinary man.

Uses: the precise targeting picker (12 §9.3) shows a preview per part;
default effort functions choose the most that does not maim; the AI layer
(09) weighs techniques; and bulk resolution of fights away from attention
uses expected outcomes in place of full resolution.

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

**Definition types**: `contest`, `projectile`. Blocks `attacks`, `defence`
and `cutscene` on actions; `surfaces` and `band` on body parts; `give` and
`span` on surfaces; `group` on traits.

**Events**: `strike_landed`, `strike_missed`, `strike_defended`,
`strike_recoiled`, `staggered`, `launched`, `clash_resolved`, `seized`,
`grip_broken`, `thrown`, `projectile_fired`, `projectile_hit`,
`projectile_passed`.

**Hooks**: `strike_resolving`, `defence_resolving`, `knockback_incoming`,
`clash_resolving`, `seize_resolving`.

**Effects**: `strike`, `shoot`, `throw`, `seize`, `release_grip`, `drag`,
`shove`.

**Functions**: `contest`, `odds`, `strike_preview`, `lead_point`,
`ballistic_dir`, `band_point`.

**Queries**: `grips` (of or on an entity), `defences` (in force on an
entity), `in_arc`.

**Properties**: `self.gripping`, `self.gripped_by`, `self.defending`,
`self.reach`; on running actions, `attack(id)` with its current `energy`
and time to impact.

**Settings**: `clash_window`, `lock_ratio`, `stagger_at`, `launch_at`.

## 17. Invariants

| # | Invariant |
|---|-----------|
| C1 | Every strike that harms does so through a `Hit` or a damage packet. Combat changes no body or element directly. |
| C2 | The energy delivered, turned aside, pushed, spilled and returned by a strike sums to no more than the energy put into it. |
| C3 | A defence applies only if it is in force at the instant of impact. |
| C4 | Chance enters only through contests and shot dispersion, both from the simulation's random stream. |
| C5 | `strike_preview` and `odds` are pure. |
| C6 | Two attacks that clash are resolved once, together. |
| C7 | A grip exists exactly as long as the action maintaining it. |
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

## 19. Open questions

1. **Projectiles meeting projectiles.** Bodies do not collide with each
   other in flight. Shooting an arrow out of the air needs that.
2. **Reach in height.** A strike aimed at a part far above or below the
   attacker's reach is not prevented. Height bands could be applied to
   melee as they are to shots.
3. **Swinging in confined space.** A long weapon in a corridor is not
   hindered by the walls.
4. **Gripping things that are not creatures.** Ledges, vehicles, subgrids.
   Climbing a giant is a grip on a subgrid.
5. **Mass battles.** Bulk resolution from previews is sketched in
   section 15. Whether it gives believable results for scene S19 has to be
   tested. (09)
