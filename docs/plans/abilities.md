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
| A1 | Reactive expressions | M | K4, X3 | done |
| A2 | Abilities: definition, sources, rank, parameters | L | E3, E9 | beta |
| A3 | Grants, costs, cooldowns, hooks on use | M | A2, T3 | beta |
| A4 | Modes, upkeep, charging | M | A3 | beta |
| A5 | Dormancy, suppression, loss | M | A2, A1 | done |
| A6 | Anchored areas and ambient overrides | M | W7, T1, S1 | in progress |
| A7 | Bindings: bound traits, summons, conjuring, dispelling | M | A5, S7 | in progress |
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

Done (decisions D-720, D-721; 05 §7.5 as built). `changes_on` is the
registry's `changed_by`. Watched: the `while` of abilities, and of traits
whose condition can be watched. Device outputs (16 §7) are package G5's
to watch. Tests: `crates/clash-sim/tests/abilities.rs`.

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

Beta (decisions D-722, D-723, D-728; 08 §12.1). Stubbed: items, modules
and body parts confer abilities only through the traits they give (no
`abilities` field of their own); `ability.var.x`; `ability` bound in an
ability's attached rules; the seat of trait-given abilities, which stay
with the body until traits have seats.

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

Beta (decisions D-724 to D-726; 08 §12.1). Built on abilities: grants with
`params`, `cost`, `cooldown`, `at_rank`, `if`, `intent`, `value`; costs
from pools and items, overdraw, and `from` another payer (the review
addition, with any expression until `ability.bound(name)` exists);
`ability_using`; `use_ability`, the commands `UseAbility` and `SetMode`;
`ability::options` and `available` for the application and the AI.
Stubbed: the `grants` of traits and modules are still carried, not
offered; costs from stores (16 §5); `value` is carried for the AI layer.

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

Beta (decision D-727; 08 §12.1). A mode ends at a wake-up of its own,
worked out from the pool's line, rather than at the pool's crossing.
Stubbed: channel claims (`uses` is carried; a mode's trait may disable
slots meanwhile); a switching action supplied by the mode (a mode is a
toggle; a windup is a grant whose action calls `start_mode`). The
bundled powers of `mods/core/data/powers` (D-729) and the test scripts
under `scenarios/powers/` exercise A2 to A4 in the test city.

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

Done (decision D-770; 08 §12.1). Suppression by `suppress_abilities`
(for a time on the owner's own time, or until `restore_abilities`), by
a trait's `suppresses` and by an area kind's `suppresses`; the hook
`ability_suppressing`, where a cancel resists for as long as the
suppression lasts. Tests: `crates/clash-sim/tests/abilities_world.rs`;
the test script `scenarios/powers/null_field.scn`.

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
- With `with_minds`, a character inside the area holds no memory of the
  undone span afterwards, while a character who watched from outside, and
  the place's local knowledge, still do.
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
  and the worked examples of 08 §9 exactly as written.
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
| `as_planned` | A10 | — | Planned movement and imposed spaces |
| `the_court` | A10 | — | Planned movement and imposed spaces |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Field reactions in reference abilities | A10 | 02 §16; 08 §8 | The reference elemental abilities rely on field reactions (world-model package W16) for what fire, water and lightning do to each other, and define none of it themselves. |
| Elemental reference abilities | A10 | 08 §8.4 | The reference mod includes a freezing power that lays a sheet a person can walk on over water, a quake, a wave and a gust, each built only from the mechanisms the catalogue names. |
| Costs from something bound | A3, A7 | 08 §5.2, §7.3 | A binding made with a name is reachable as `ability.bound(name)`. An action whose cost is drawn from a bound companion spends the companion's pool, and is unavailable with a reason when the companion is gone or short. |
| Planned movement and imposed spaces | A10 | 08 §9.5, §9.6 | Test script `as_planned`: a route of twenty-four steps is carried out in one second and cannot be changed once begun; an unsound route stops the user's clock for a second, during which they notice no window. Test script `the_court`: an imposed space replaces what was there, hits everyone inside with the pool it names and nobody without it, loses a contest to a stronger one, and when it ends the place is as it was and its owner's ability is suppressed. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
