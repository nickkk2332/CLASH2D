# Plan — Combat

Implements [07-combat.md](../07-combat.md).

Packages prefixed W are in [world-model.md](world-model.md); X, K and T in
[sim-kernel.md](sim-kernel.md); E, S and U in
[first-playable.md](first-playable.md); G in [devices.md](devices.md). The
rules for taking a package are those of [world-model.md](world-model.md)
and [CLAUDE.md](../../CLAUDE.md).

With packages C1 to C8, C11 and C12 the first playable build of
[first-playable.md](first-playable.md) has fighting in it. Ranged weapons
(C9, C10) follow.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| C1 | Contests | S | X4, K5 | done |
| C2 | Surfaces and delivery | L | E5, E8, C1 | beta |
| C3 | Aim | M | C2, S4 | beta |
| C4 | Defences | L | C3, T5 | todo |
| C5 | Push | M | C2, E10 | beta |
| C6 | Clashes | L | C4, C5, W15 | todo |
| C7 | Grips and throws | L | C4, C5, T6 | todo |
| C8 | Takedowns | M | C2, T6 | todo |
| C9 | Projectiles | L | C4, W14, S2 | todo |
| C10 | Ranged weapons | M | C9, G9 | todo |
| C11 | Stances, combinations, effort, previews | M | C4, C5, E3 | todo |
| C12 | Bundled combat content | M | C6, C7, C8, C11 | todo |
| C13 | Benchmarks and budget gate | S | C2, then recurring | todo |

## Order and parallelism

| Wave | Packages |
|------|----------|
| 1 | C1 |
| 2 | C2 |
| 3 | C3, C5, C8 |
| 4 | C4 |
| 5 | C6, C7, C9, C11 |
| 6 | C10, C12 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M3d A fight** | C1–C5, C8, C11 | One key attacks from defaults; another opens every choice with a preview beside each body part. A parry made in time turns a cut; made too late it does not exist. A punch at a breastplate hurts the fist. A shove staggers; a much harder one sends someone through a wall. A takedown from behind plays as a cutscene and is stopped by a gorget. |
| **M3e Clash** | C6, C7, C12 | Two strikes meet and lock, and the windows along the street break. The stronger of two unequal strikes carries through. A grab, a throw into a wall, a human shield. Bundled techniques, stances and reaction rules make a brawl playable start to finish. |
| **M3f At range** | C9, C10 | A musket ball is stopped by a brick wall and not by a door. An arrow arcs. A character on a fast clock steps out of a bullet's path; nobody else is offered the chance. A rifle with a magazine, a scope and a suppressor is an assembly. |

## Work packages

### C1 — Contests

**Spec** 07 §4.1 · **Size** S

Build
- The `contest` type; `contest` and `odds`; both forms; draws from the rule
  stream.

Done when
- `odds` matches the observed frequency of positive margins over many draws
  within tolerance.
- A spread of zero is a pure comparison.
- `odds` is accepted in pure contexts and `contest` is refused.

### C2 — Surfaces and delivery

**Spec** 07 §3, §6 · **Size** L

Build
- The `attacks` block on actions; `with` resolution for held items, parts
  and gripped things; `effort`.
- The `strike` effect: the ordered steps, with aim, clash and defences as
  stubs filled by later packages; `sure`.
- Delivery: packet and hit construction, `span`, pass-through and exits,
  recoil into the striking surface.
- Arcs: sweep and line, ordering, `through`.
- Strikes at elements.
- Action locals and events.

Done when
- At equal energy an edge and a fist give the intensities of 07 §6.1 and
  differ in depth as 06 §7.2 predicts.
- A strike whose intensity is below the target's outer threshold returns
  its energy to the striking surface.
- Energy accounting satisfies invariant C2 over random strikes (property
  test).
- Scenario `own_fist`: a strong, fragile striker is harmed by its own blow
  against a wall.
- Scenario `one_cut`: an arc with `through` passes five unarmoured targets
  and stops at an armoured sixth.

Beta (decisions D-641, D-642): built and tested in
`crates/clash-sim/tests/combat.rs`, with `own_fist` and `one_cut` as Rust
tests (the console host refuses verbs and `do` until the rule runtime is
bound to it, so a test script cannot spawn from a template or act). Stubbed: clash and defences are
steps that find none (C6, C4); `with` a gripped thing is refused (C7);
swings meeting the world (`swing`, review row "Swings meet the world"),
contact surfaces (`the_pit`) and `closing_energy` are not built; `height`,
`lands`, `clashes`, `spill` and `spill_kind` are read and carried.

### C3 — Aim

**Spec** 07 §4.2 · **Size** M

Build
- Aim resolution: on the part, a neighbouring part, a miss; unaimed
  strikes by exposure.

Done when
- Outcome frequencies follow the contest's margin bands.
- A target that does not perceive its attacker is hit, using a content
  function that reads perception.

Beta (decision D-643): aim by margin bands, the parts beside a part, and
unaimed strikes by exposure are built and tested; the bundled `evasion`
reads `perceives`. Not built: reach in height (07 §4.3, review rows
"Reach in height" and "Reach and cover by posture"); `height` is read and
carried.

### C4 — Defences

**Spec** 07 §5 · **Size** L

Build
- The `defence` block; in-force tracking by phase; `covers`, `handles`.
- `avoid` by leaving and by slipping; `interpose` with `deflect`, `holds`
  and `brace`; the interposed surface taking the remainder and passing
  what it cannot stop.
- Guards as held phases.

Done when
- A defence applies only when in force at impact (invariant C3).
- Scenario `parry_in_time`: the first worked exchange of 07 §14, to its
  figures.
- Scenario `broken_blade`: the second, with the blade destroyed and the
  strike arriving.
- Scenario `too_slow`: the third; no defence exists.
- A defender who has moved out of reach is missed without a contest.

### C5 — Push

**Spec** 07 §7 · **Size** M

Build
- Push speed; `knockback_incoming`; stagger and launch; hand-over to world
  bodies and back; the settings.

Done when
- Push speed matches the formula, including the megajoule example.
- Scenario `bar_fight`: a punch launches a target through a wall; the
  wall's failure and the target's injuries both come from the impact.
- A braced defender is pushed by the unbraced share only.

Beta (decision D-644): push speed, `knockback_incoming`, stagger, launch
through the world's bodies and back through movement's landing,
`knock_down`, the settings, and a target in flight taking all of a push
are built; `bar_fight` and a shove into a wall are Rust tests in
`crates/clash-sim/tests/combat.rs`. Stubbed: bracing waits for defences
(C4: `brace` is 0, and the formula is tested on its own); of the review
row "In the air", a leaping attacker's strike with the leap's speed waits
for `leap` and `closing_energy`.

### C6 — Clashes

**Spec** 07 §8 · **Size** L

Build
- Detection at step 3; bringing the second strike forward; lock and
  overpower; spill as an energy release; `on_clash` phases;
  `clash_resolving` and `clash_resolved`.
- A struggle fixture as a joint action with ticks.

Done when
- Two clashing attacks are each resolved exactly once (invariant C6).
- Scenario `the_clash`: an even clash delivers neither strike and its spill
  breaks glass at the expected radius.
- An uneven clash delivers the stronger at the difference and returns the
  weaker into its own surface.
- A reaction attack timed to land with an incoming one produces a clash.

### C7 — Grips and throws

**Spec** 07 §9 · **Size** L

Build
- `seize`, `Grip`, its tie to the maintaining action, pinning, dragging.
- `release_grip`, `throw`, `drag`; carrying; gripped creatures as
  interposed defences and as striking surfaces.

Done when
- A grip ends when its action ends, for any reason (invariant C7).
- Scenario `thrown_through`: a throw launches the held at the formula's
  speed and the world resolves what it hits.
- Scenario `human_shield`: a hit lands on the gripped creature.
- A pinned arm's actions are unavailable until release.

### C8 — Takedowns

**Spec** 07 §10 · **Size** M

Build
- Joint actions with attacks and `sure` strikes; `requires` on perception
  and helplessness; the `cutscene` field raising a cue.

Done when
- Scenario `takedown`: an unaware victim is locked in and struck;
  an aware one cannot be targeted.
- The same takedown against a protected neck is stopped by the layer.

### C9 — Projectiles

**Spec** 07 §11.1–11.4 · **Size** L

Build
- The `projectile` type; `shoot`; dispersion; bodies carrying surfaces.
- Strikes on creatures by height band; defences against shots; continuing
  after exit; `projectile_passed`.
- Instant shots by ray.
- The `shot` window; the aiming helpers.

Done when
- Scenario `through_cover`: the same shot is stopped by one material and
  passes another, slower.
- Scenario `bullet_time`: a target on a fast clock is offered and takes a
  reaction to a bullet; an ordinary target is not offered one.
- A shot over a low wall can hit only parts whose band is above it.
- An arrow launched with `ballistic_dir` arrives; launched flat, it falls
  short.
- Scenario `volley`: five hundred projectiles within the budget of 07 §18.

### C10 — Ranged weapons

**Spec** 07 §11.5 · **Size** M

Build
- Fixtures for each row of 07 §11.5 as assemblies with stores and costs:
  a firearm with magazine and modules, a drawn bow, automatic fire, a
  thrown item, a timed explosive, an instant energy weapon.

Done when
- An empty magazine makes the fire action unavailable, with its reason.
- A fitted module changes spread and noise and stops doing so when
  removed.
- A bow's shot energy follows draw time up to its cap.

### C11 — Stances, combinations, effort, previews

**Spec** 07 §3.3, §12, §13 · **Size** M

Build
- Exclusive trait groups; branching and list-valued `cancel` in technique
  fixtures; default effort functions.
- `strike_preview`, computed from what the viewer could know.
- Preview data for the precise picker and reaction options.

Done when
- `strike_preview` draws nothing and changes nothing (invariant C5), and
  its `odds` and `depth` agree with the mean of many real resolutions.
- A target whose toughness is hidden previews as it appears.
- Scenario `hold_back`: a default effort function chooses an effort that
  staggers and does not maim.

### C12 — Bundled combat content

**Spec** 07 §12 · **Size** M

Build
- In the `core` mod: contests; a small set of strikes, a shove, a grab, a
  throw, a block, a parry, a dodge; two or three stances; reaction rules
  for when the player is asked and what unprompted characters do; stagger
  and exertion rules; `auto_part` and `auto_effort`.

Done when
- Two bundled characters fight to a conclusion under AI stand-in rules
  without any content outside `core`.
- Removing `core` leaves the engine able to load a mod that defines all of
  this differently.

### C13 — Benchmarks and budget gate

**Spec** 07 §18 · **Size** S, recurring

## Scenario catalogue

Under `scenarios/combat/`.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `own_fist` | C2 | — | Recoil |
| `one_cut` | C2 | S4 | Energy carried through targets |
| `parry_in_time` | C4 | S14 | A defence in force |
| `broken_blade` | C4 | S14 | The limit of what can be held |
| `too_slow` | C4 | S14 | No notice, no defence |
| `bar_fight` | C5 | S1 | Push into flight into a wall |
| `the_clash` | C6 | S5 | Lock and spill |
| `thrown_through` | C7 | S2 | Throw into the world |
| `human_shield` | C7 | — | Gripped creature interposed |
| `takedown` | C8 | S6 | Forced role, sure strike, cue |
| `through_cover` | C9 | S19 | Materials as cover |
| `bullet_time` | C9 | S3 | Time of flight against notice delay |
| `volley` | C9 | S19 | Many projectiles |
| `hold_back` | C11 | — | Effort from preview |
| `blade_in_a_corridor` | C2 | — | Swings meet the world |
| `off_its_hinges` | C7 | — | Holding the world |
| `caught_arrow` | C9 | — | Stopping a shot |
| `the_pit` | C2 | — | Contact surfaces |
| `before_they_land` | C5 | — | In the air |
| `heat_seeker` | C9 | — | Driven and guided projectiles |
| `mixed_belt` | C10 | — | Ammunition as items |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Swings meet the world | C2 | 07 §6.4 | Test script `blade_in_a_corridor`: a wide swing in a corridor strikes the wall first, spends energy on it and takes the recoil; a thrust along the same corridor is not hindered. |
| Reach in height | C3 | 07 §4.3 | A part outside the attack's height band cannot be aimed at and the refusal carries a reason. The same part is reachable after climbing or from higher footing. |
| Defending someone else | C4 | 07 §5.1; 03 §7.1 | A defence with `protects` takes a strike aimed at the character beside the defender. Completes sim-kernel test script `bodyguard` with a real strike. |
| Holding the world | C7 | 07 §9.3 | Test script `off_its_hinges`: a grip on a ledge stops a fall; `wrench` with enough energy brings a door away as an item and a larger piece away as a subgrid, which can then be swung or thrown; with too little it does neither. |
| Stopping a shot | C9 | 07 §11.6 | Test script `caught_arrow`: a character quick enough to have a `shot` window interposes, catches or intercepts a projectile, each by its own mechanism; one too slow gets no window. Two projectiles crossing do not interact. |
| Contact surfaces | C2 | 07 §6.5; 02 §5.4 | Test script `the_pit`: a fall onto spikes pierces with the energy of the fall where the same fall onto stone bruises; walking into them strikes the legs with their stated energy; the spikes take the recoil. |
| Speed behind the blow | C2 | 07 §3.5 | `closing_energy` is zero for a standing attacker, and for a mounted one equals the stated mass at the closing speed. A couched lance delivers more at a gallop than at a walk by that amount. |
| Reach and cover by posture | C3 | 07 §4.3; 06 §11.1 | A prone target behind a half-height wall cannot be hit by a level shot that hits a standing one. A high swing passes over someone prone. |
| In the air | C5 | 07 §7.1; 03 §6 | Test script `before_they_land`: a launched target is struck again in flight and its velocity changes by the push; a leaping attacker's strike lands during the leap with the leap's speed added; an action that needs footing is refused in the air with a reason. |
| Driven and guided projectiles | C9 | 07 §11.1; 02 §15 | Test script `heat_seeker`: a seeker turns toward its target within its turn rate, is drawn off by a stronger source in its cone, and loses a target that goes behind a wall. A launcher-guided shot goes ballistic when the guiding action ends. A returning projectile comes back to where its thrower then is. |
| Ammunition as items | C10 | 07 §11.5; 06 §9.1 | Test script `mixed_belt`: a magazine loaded with two kinds of round fires them in the order loaded, each with its own projectile and energy; an empty magazine refuses with a reason; `count` above one launches that many. |
| Fighting at lower detail | C11 | 07 §15 | A shot fired in a warm tract is instant. A warm entity attacked by a hot one is promoted before the strike resolves. Bulk exchanges themselves are ai package I8. |
| Order across plans | C10 | — | C10 depends on devices package G9 (assemblies), which is in a later milestone. Until G9 exists C10 is built against a magazine supplied by a test fixture. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
