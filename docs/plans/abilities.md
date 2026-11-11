# Plan — Abilities and Powers

Implements [08-abilities-and-powers.md](../08-abilities-and-powers.md) and
the amendments it makes to 02 §8.3, 05 §7.5, 06 §5 and 15.

Packages prefixed W are in [world-model.md](world-model.md); X, K and T in
[sim-kernel.md](sim-kernel.md); E, S and U in
[first-playable.md](first-playable.md); C in [combat.md](combat.md); G in
[devices.md](devices.md). The rules for taking a package are those of
[world-model.md](world-model.md) and [CLAUDE.md](../../CLAUDE.md).

Package A1 is also needed by the devices plan (G5) and has no dependency on
the rest of this plan. Build it early.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| A1 | Reactive expressions | M | K4, X3 | todo |
| A2 | Abilities: definition, sources, rank, parameters | L | E3, E9 | todo |
| A3 | Grants, costs, cooldowns, hooks on use | M | A2, T3 | todo |
| A4 | Modes, upkeep, charging | M | A3 | todo |
| A5 | Dormancy, suppression, loss | M | A2, A1 | todo |
| A6 | Anchored areas and ambient overrides | M | W7, T1, S1 | todo |
| A7 | Bindings: bound traits, summons, conjuring, dispelling | M | A5, S7 | todo |
| A8 | Snapshots, journals, restoring as generated | L | E8, W6, K7, W8 | todo |
| A9 | Relays, appraisal, field movement, force at range, contest bias | M | S5, C7, W16 | todo |
| A10 | Reference abilities | L | A4, A6, A7, A8, A9, C12 | todo |
| A11 | Benchmarks and budget gate | S | A2, then recurring | todo |

## Order and parallelism

| Wave | Packages |
|------|----------|
| 1 | A1, A2, A6, A8, A9 |
| 2 | A3, A5 |
| 3 | A4, A7 |
| 4 | A10 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M4e A gift** | A1–A5 | The same ability conferred by a template, by a trait and by an item. It ranks up with use and its bolt hits harder. A sustained mode drains a pool and ends when the pool is empty. Under water it goes dormant; in a nulling field it is suppressed; out of both it returns. A cost that cannot be met is paid from blood by overdraw. |
| **M4f Things left behind** | A6–A8 | An aura follows its owner and frightens whoever it comes to cover. A summoned servant vanishes when its summoner stops. A conjured wall gives the street back when it ends. A fighter returns to where and how they were ten seconds ago. |
| **M4g The catalogue** | A9, A10 | One working ability from each family of 08 §8, including time stopped over a street, a scrying eye, a planted memory, a dominated guard, a barrier that fails at the right hit, and a sword style that is the same kind of definition as all of them. |

## Work packages

### A1 — Reactive expressions

**Spec** 05 §7.5 · **Size** M

Build
- Read recording during evaluation; the `changes_on` metadata on
  properties; subscription and staleness; re-evaluation at the end of the
  pump in a defined order; notification of owners.
- A load error for random functions in reactive positions.

Done when
- A reactive value always equals a fresh evaluation after random sequences
  of changes (invariant A6, property test).
- An expression is re-evaluated exactly when a dependency changed, and at
  most once per pump.
- A property registered without `changes_on` fails a test.

### A2 — Abilities: definition, sources, rank, parameters

**Spec** 08 §3, §4 · **Size** L

Build
- The `ability` type; the `Abilities` component; the `ability` subject kind
  and its properties.
- Conferral by templates, traits, equipment and `grant_ability`; one
  instance per ability across sources; `revoke_ability`; seat.
- Rank from a track or set directly; `at_rank`; parameters as modifiable
  values with modifiers by ability and by tag; `always`.

Done when
- An ability held from two sources survives the loss of one and ends with
  the second (invariant A8).
- A mind moved to another body keeps mind-seated abilities and gains that
  body's.
- A parameter read in an action changes with rank and with a modifier from
  an unrelated trait.

### A3 — Grants, costs, cooldowns, hooks on use

**Spec** 08 §5.1–5.3, §7.9 · **Size** M

Build
- Grant objects with `cost`, `cooldown`, `at_rank`, `if`, on abilities,
  traits and modules alike; availability with reasons.
- Costs from pools, stores and items; overdraw.
- `ability_using`.

Done when
- A cost is taken in full or the action does not start (invariant A4).
- Overdraw takes exactly the shortfall at the ratio.
- A rule on `ability_using` raises a parameter and the cost for one use
  only.
- A gagged character cannot use an action that needs `voice`.

### A4 — Modes, upkeep, charging

**Spec** 08 §5.4, §5.5 · **Size** M

Build
- Modes with traits, upkeep rates, channel claims, start and end effects;
  ending on an empty pool through the pool's scheduled crossing.
- A charged-action fixture.

Done when
- A mode's trait and upkeep begin and end together (invariant A5).
- A mode ends at the analytically correct tick when its pool empties, with
  nothing scheduled in between.
- Charge delivered follows time held.

### A5 — Dormancy, suppression, loss

**Spec** 08 §6 · **Size** M

Build
- `while` on abilities and on traits; dormant state and its consequences.
- `suppress_abilities`, `restore_abilities`; suppression by area kind and by
  trait; `ability_suppressing`.
- What revoking and losing sources ends.

Done when
- A dormant or suppressed ability confers nothing (invariant A2) and
  returns intact.
- Scenario `null_field`: entering a suppressing area ends a running mode;
  leaving restores the ability but not the mode.
- A rule on `ability_suppressing` resists.

### A6 — Anchored areas and ambient overrides

**Spec** 02 §8.3; 08 §7.2 · **Size** M

Build
- Area anchors; movement with the anchor; enter and exit events caused by
  the area moving; radius shapes.
- Ambient override for gravity; `Cosmos::ambient` consulting areas by
  priority.
- Negative light sources.

Done when
- Moving an anchored area raises the same events as the occupants moving
  the other way (property test).
- A body falling through a zero-gravity area stops accelerating inside it.
- An anchored area with a clock rehomes what it comes to cover.
- Light is never negative.

### A7 — Bindings: bound traits, summons, conjuring, dispelling

**Spec** 08 §7.3 · **Size** M

Build
- `Binding` storage on ability instances; bound traits, areas, portals and
  rules; `summon`; `conjure` with remembered state; durations; `on_end`;
  `dispel`, `dismiss`; `binding_ending`.

Done when
- Revoking an ability ends everything bound to it by its declared `on_end`
  (invariant A3).
- Scenario `summon_expires`: a servant obeys, then is destroyed when its
  summoner stops.
- Scenario `conjured_wall`: a wall placed over a street is removed and the
  street restored; a cell that was since altered is left alone.
- `release` leaves the thing in place, unbound.

### A8 — Snapshots, journals, restoring as generated

**Spec** 08 §7.4; 02 §8.3 · **Size** L

Build
- Entity and box snapshots; storage on ability instances; caps, expiry,
  saving; restoration through component functions and `WorldEdit`.
- Area journals: recording cell changes with their prior state for the
  area's span, periodic copies of fields and of occupants, the bound, and
  `rewind_area`.
- `restore_as_generated`: realising afresh to one side and writing the box
  through `WorldEdit`.

Done when
- Scenario `recall`: an entity restored to a snapshot matches it in place,
  body, pools and holdings.
- A box restore emits ordinary change events with a cause (invariant A7).
- A box over the cap is refused with a reason.
- Scenario `undo_the_blast`: a wall destroyed inside a journalled area
  stands again after `rewind_area`, a collapse begun in that span is ended,
  and a character who watched remembers both.
- An area with no journal records nothing, and one in which nothing changes
  uses no memory beyond its own record.
- `restore_as_generated` returns a wrecked street to the hash of the same
  street freshly realised, and removes what was built there since.

### A9 — Relays, appraisal, field movement, force at range, contest bias

**Spec** 08 §7.5–7.8 · **Size** M

Build
- `relay_senses`; `appraise` and facts on tracks; exposure of appraised
  values to ordinary screens.
- `move_field`.
- Inline surfaces in attacks; long-reach `seize`.
- `contest_resolving`.

Done when
- Scenario `scrying_eye`: a remote observer's percepts create tracks and
  memories in its owner.
- An appraised value appears on an ordinary screen; an unappraised one is a
  load error there.
- A grip at range holds, drags and throws.
- A rule on `contest_resolving` shifts outcomes by the expected amount over
  many draws.

### A10 — Reference abilities

**Spec** 08 §8, §9, §11 · **Size** L

Build
- A reference mod with at least one working ability per table of 08 §8,
  and the four worked examples of 08 §9 exactly as written.
- Data for the abilities screen; `intent` on every grant.

Done when
- Scenario `time_stop`: inside the area people, bodies in flight and
  falling debris hang; the user moves; a thrown knife leaves the hand and
  hangs; when the pool empties everything resumes.
- Scenario `reshaped`: a landed touch changes the target's parts and its
  functions follow.
- Scenario `barrier`: hits are stopped while the pool lasts and the hit
  that exceeds it comes through at the difference.
- Scenario `planted_memory`: a character acts on a memory it was given of
  something that did not happen.
- Scenario `dominated`: a driven guard opens a door for the driver.
- Scenario `cheat_skill`: a template grants an ability with extreme
  parameters and an interface part.
- The reference mod loads with no engine change, and removing it leaves
  the engine without any notion of the abilities it defined.

### A11 — Benchmarks and budget gate

**Spec** 08 §14 · **Size** S, recurring

## Scenario catalogue

Under `scenarios/abilities/`.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `null_field` | A5 | — | Suppression by area |
| `summon_expires` | A7 | S16 | Bound entity |
| `conjured_wall` | A7 | — | Edit binding and restoration |
| `recall` | A8 | — | Entity snapshot |
| `undo_the_blast` | A8 | S10 | Area journal and rewinding |
| `scrying_eye` | A9 | — | Sense relay |
| `time_stop` | A10 | S10 | Anchored clock area, anchor, upkeep |
| `reshaped` | A10 | S17 | Touch attack and body verbs |
| `barrier` | A10 | — | Pool spent through a hook |
| `planted_memory` | A10 | S15 | Belief without perception |
| `dominated` | A10 | S16 | Driver |
| `cheat_skill` | A10 | S20 | Template, parameters, interface part |
