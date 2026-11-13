# Plan — First Playable: Entities, Perception, Presentation

Implements [06-entities-and-bodies.md](../06-entities-and-bodies.md),
[15-perception-and-social.md](../15-perception-and-social.md) and
[12-presentation-and-ui.md](../12-presentation-and-ui.md).

Packages prefixed F and W are in [world-model.md](world-model.md); P, X, K
and T are in [sim-kernel.md](sim-kernel.md). The rules for taking a package
are those of [world-model.md](world-model.md) and
[CLAUDE.md](../../CLAUDE.md).

When this plan is complete there is a game to sit down at: a window, a
character built from templates, a destructible place with people in it who
must notice you before they can respond to you. Fighting is added by
[combat.md](combat.md); within this plan, attacks in tests are single
actions that call `hit`.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| E1 | Component store and core components | M | K1, K7 | beta |
| E2 | Attributes, pools, tracks | M | E1, K5 | beta |
| E3 | Traits | M | E2 | beta |
| E4 | Bodies | L | E2, T3 | beta |
| E5 | Hits and wounds | L | E4, W12 | beta |
| E6 | Body editing | M | E4 | beta |
| E7 | Minds and vital state | M | E1, T4 | beta |
| E8 | Items and holds | L | E5 | beta |
| E9 | Templates and creation | L | E3, E6, E8 | beta |
| E10 | Movement | M | E4, W14, W18 | beta |
| E11 | Coming and going | L | E9, W8 | beta |
| E12 | Minimal devices | M | E8 | beta |
| S1 | Light | M | W18 | beta |
| S2 | Signals and senses | M | E3 | beta |
| S3 | Detection | L | S1, S2, E10 | beta |
| S4 | Percepts | M | S3, K3 | beta |
| S5 | Beliefs | M | S4, E7 | beta |
| S6 | Personas | M | S5 | beta |
| S7 | Relations, groups, orders | M | S6, T4 | beta |
| S8 | Speech | M | S7, T6 | beta |
| S9 | Party perception | M | S5 | beta |
| S10 | Coloured light (experimental) | S | S1, U4, P4 | todo |
| U1 | Scene skeleton, frames, themes | M | K2 | beta |
| U2 | Terminal frontend | S | U1 | done |
| U3 | Window frontend | L | U1 | done |
| U4 | View model and world drawing | L | U1, S9 | beta |
| U5 | Playback | L | U4 | beta |
| U6a | Toolkit: layout and widgets | L | U1 | beta |
| U6b | Toolkit: binding and UI effects | M | U6a, X4, K6 | beta |
| U7 | Input, controls, pickers | M | U6b, T5 | beta |
| U8 | Slots and theming | S | U6b | done |
| U9 | Bundled screens | L | U7, U8, U4, U5, E9 | beta |
| U10 | Cutscenes | M | U5, T6 | todo |
| U11 | Application | M | U3, U9, P3 | beta |
| Z1 | Benchmarks and budget gate | S | E1, then recurring | beta |

## Order and parallelism

The three tracks share few dependencies. U1 to U3 and U6a need only the
kernel and can start as soon as K2 is done.

| Wave | Entities | Perception | Presentation |
|------|----------|------------|--------------|
| 1 | E1 | S1 | U1 |
| 2 | E2, E7 | — | U2, U3, U6a |
| 3 | E3, E4 | — | U6b |
| 4 | E5, E6, E10 | S2 | U7, U8 |
| 5 | E8 | S3 | — |
| 6 | E9, E12 | S4 | — |
| 7 | E11 | S5 | — |
| 8 | — | S6, S9 | — |
| 9 | — | S7 | U4, U11 |
| 10 | — | S8 | U5 |
| 11 | — | — | U9 |
| 11 | — | — | U10 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M3a People** | E1–E12 | From the console: build a character from templates with choices, cut it and watch the arm stop working and the blood pool fall, reshape its body, move its mind to another body, have it hide in a locker. |
| **M3b Awareness** | S1–S9 | A guard hears a window break and goes to look. Someone in shadow goes unnoticed; someone in a locker is not seen. A crime done in another's face is remembered as theirs. |
| **M3c A window** | U1–U11 | Launch the game, pick a profile, build a character on the creation screen, walk around a building in a window with mouse and keys, watch a wall break in playback, read it in the log, see a takedown as a cutscene. |

## Entities

### E1 — Component store and core components

**Spec** 06 §3 · **Size** M

Build
- `ComponentStore<C>`, registration, saved dense order.
- `Identity` and `Physical` with derived, cached mass and size.
- Component saving and loading through the K7 path.

Done when
- Store operations match a simple model (property test); iteration order
  survives save and load (invariant B8).
- `Physical` equals a from-scratch recomputation after random changes.

Beta: built ahead of the kernel (K1), in `clash_sim::component`, keyed by
`EntityId` and knowing nothing of the simulation: `ComponentStore<C>`;
`Components`, every store behind one value, generated with
`ComponentKind` and `ComponentBundle` from one list, so that a later
package adds a component type with one line; `Identity`; and `Physical`,
whose carried mass is derived from a `Containment` trait that the kernel
implements over its places. Stores, bundles and both components serialise
through `clash_core::codec` and hash. Stubbed, each waiting for the
kernel: holding a `Components` in the simulation and writing it through
the simulation's save path (K7), with definition ids going through the
save's id table; the calls on entity removal, storage and relocation,
which the module documentation lists; registering the properties of
06 §13 that these components back (K4); raising events from what the
components' functions report. A shape's size is supplied by whoever owns
it (the body in E4, the item in E8) and is not derived here. No test
script, since nothing here can be reached from the console until the
kernel holds it, and no benchmark (Z1).

Joined to the kernel since (decision D-460): the simulation holds the
`Components`, saves them in its blob, hashes them as a part of its own,
implements `Containment` over places, refreshes masses on relocation and
removal and writes changed masses to the world's occupant records
(`Sim::set_shape`, `Sim::set_own_mass`); the properties these components
back are registered (06 §13). Still stubbed: definition ids in saved
components go through no id table (K7); `templates` is not a property.

Since (decisions D-550, D-558): the `Mind` and `Driver` components
follow `Body` in the list, and `Identity` says whether an individual is
anonymous; the save layout version is 5.

### E2 — Attributes, pools, tracks

**Spec** 06 §4 · **Size** M

Build
- The three definition types; modifiers with sources; derived attributes
  with dependency invalidation.
- Pools with rates and scheduled threshold crossings.
- Tracks with level curves.

Done when
- Cached attributes equal recomputation under random modifier changes.
- A pool crossing fires at the analytically correct tick, including across
  a clock rate change, and an idle pool schedules nothing.
- `track_advanced` fires once per level.

Beta: built ahead of the kernel, in `clash_sim::stats`, as functions that
take the store of `Stats`, the definitions, the timeline and an expression
host, and return what changed (decisions D-440 to D-449). Built: the
`attribute`, `pool` and `track` definition types with their fields,
scopes and checks, a dependency cycle being a load error at each `base`;
`StatDefs`, the tables worked out after loading; the `Stats` component,
the third in the component list, saved and hashed; modifiers and pool
rates with sources, and `remove_source`; derived attributes worked out in
dependency order with exact invalidation, and `refresh` from scratch;
pools whose crossings are one scheduled wake-up, with `on_wake`,
`set_clock`, `suspend` and `resume`; `pool_add` and all-or-nothing
`pool_spend`; tracks with level curves; the kinds, properties and events
of 06 §4.4 registered through a registry step, and `stats::property` to
answer them; reasons for every refusal. Fixtures are under
`crates/clash-sim/tests/fixtures/stats`. Stubbed, each waiting for the
kernel or a later package: holding the store in the simulation and
calling these functions (the module documentation lists when); raising
`attr_changed`, `pool_crossed` and `track_advanced` from the changes
returned, and the hook `pool_changing`; the host that forwards property
reads; writing definition ids through the save's id table (K7);
registering and running the six verbs and a track's `on_advance` (K5);
calling `refresh` when a reactive input changes (05 §7.5); moving what is
seated in a mind (E7); a pool that names a substance (G19). No test
script, since nothing here can be reached from the console until the
kernel holds it, so `gas_mask` waits with E8; and no benchmark (Z1).

Joined to the simulation since (decisions D-550 to D-552): the tables
are built at set-up and after a load; `Sim::attach_stats`,
`give_pool`, `pool_add` (after `pool_changing`), `pool_spend`,
`pool_rate`, `set_attr_base`, `add_modifier`, `remove_modifier`,
`advance_track` and `refresh_stats` raise every change as its event in
order; the `Timer` domain runs crossings; pools follow their entity's
clock and stand still while it is away (`Pause`); the properties are
answered by the simulation; stats are worked out again when a body
function, the mass or a variable they read changes; the six verbs are
in the engine's verb table; seated stats move with a mind
(`stats::take_seated`, `give_seated`). Still stubbed: `on_advance`
(K5 binding), a pool that names a substance (G19), definition ids
through the save's id table (K7).

### E3 — Traits

**Spec** 06 §5 · **Size** M

Build
- The `trait` type; add and remove; stacking modes; durations as timers;
  everything a trait confers; the hook `trait_adding`.

Done when
- Removing a trait removes exactly what it added.
- Durations behave across unloading as 03 §9 specifies.

Beta: built in `clash_sim::traits` (decisions D-630 to D-635). Built: the
`trait` definition type with every field of 06 §5 and the new
`while_unloaded`; the `Traits` component, the eighth, with instances by
giver; `add_trait`, `remove_trait`, `withdraw` and the verbs `add_trait`
and `remove_trait`; stacking `none`, `refresh` and `stack` with `max`;
groups; dormancy by `while`; modifiers and pool rates under the instance
as source, tags and attached rules recorded and taken back exactly;
durations as wake-ups on the holder's clock, taken off the timeline when
it leaves the world and put back when it returns, `pause` or `elapse`;
the hooks `trait_adding` and `trait_removing`, the events `trait_added`,
`trait_removed` and `trait_expired`, five properties and five reasons;
`look` reported with the events; the traits of items and postures given
and withdrawn with them; plain traits in `mods/core/data/traits.json`.
Tests in `crates/clash-sim/tests/traits.rs`, a property test among them.
`on_apply` and `on_remove` run against the simulation. Stubbed: what
`grants`, `abilities`, `senses`, `bound`, `goals`, `weights` and `seat`
give; applying `channels` (read by `traits::channel_change`); keeping
`while` current until reactive expressions; no `.scn` test script and
no benchmark.

### E4 — Bodies

**Spec** 06 §6.1–6.4 · **Size** L

Build
- `body_part`, `body_plan`, `body_function`; tissue extras on materials.
- The body instance, condition, functions, scale.
- Channel slots supplied by parts.

Done when
- Functions and slots equal recomputation (invariants B2, B3).
- Disabling a part interrupts an action using its slot.
- Doubling scale doubles thickness, quadruples area, multiplies mass by
  eight.

Beta: built ahead of the kernel, in `clash_sim::body`, as functions that
take the store of `Body`, the definitions, the store of `Physical`,
containment and the stats' context, and return what changed (decisions
D-520 to D-523). Built: the `body_part`, `body_plan` and `body_function`
definition types with every field of 06 §6.1 to §6.3 and their checks,
the tissue extras and `severs` as fields of the world's `material` and
`damage_type`; `BodyDefs`, the tables worked out after loading; the
`Body` component, the fourth in the component list, saved and hashed;
`attach` from a plan at a scale; parts read by key, name, socket, tag and
definition, what covers what, reach, bands and exposed area; condition
and state; functions with their four combinations; channel slots by part
and in total, with `SlotsLost` and `SlotsGained` naming what went and
came back; mass and size handed to `Physical`; `disable`, `restore` and
`detach`, which returns the subtree as data; the kinds, properties and
events of 06 §6.7 registered through a registry step, and
`body::property` to answer them; reasons for every refusal. Fixtures are
under `crates/clash-sim/tests/fixtures/body`: a humanoid and a blob.
Stubbed, each waiting for the kernel or a later package: holding the
store in the simulation and calling these functions (the module
documentation lists when); raising the events and interrupting the
actions whose slots went (T3), which is how "disabling a part interrupts
an action" is finished; `stats::refresh` after `function_changed`; the
other verbs of 06 §6.5 (E6); postures, which call `set_height_factor`
(E10); shared parts for unchanged bodies, so no benchmark of them (Z1);
writing definition ids through the save's id table (K7).

### E5 — Hits and wounds

**Spec** 06 §7 · **Size** L

Build
- `Hit`, `hit_incoming`, layer resolution, organs, severing, exits.
- Wounds; leaks and signals into pools; healing.
- Spread hits; conversion of world impact and exposure events into hits.
- Destruction and remains.

Done when
- At equal energy, a small contact area goes deeper than a large one.
- Hits below a layer's threshold never accumulate.
- A severing cut produces an item and a wound on the parent.
- Scenario `one_bad_cut`: one cut to an arm disables it, starts a drain on
  a pool, and the pool's threshold event fires.
- Scenario `through_the_wall`: a hit launches a creature, the wall breaks by
  02 §12, and the creature's injuries come from the impact events.

Beta: built ahead of the kernel, in `clash_sim::body::hit` and
`body::wound` (decisions D-524 to D-529). Built: `Hit` with its four
ways of arriving and the hook `hit_incoming` described with its fields;
`apply_hit`, which resolves layers by the world's `resolve_packet` from
the outside in, with worn layers handed in by the caller, organs inside
parts, severing, destruction, exits and an account of all the energy;
wounds with their leaks as rates on pools under the wound as source,
signals into pools, closing and healing by one wake-up for each body
(`on_wake`, `suspend`, `resume`, `set_clock`), `close_wound`,
`reopen_wound` and `heal_part`; spread hits by exposed area and outright
destruction; `exposure_hit`, `impact_hit` and `field_hit`, which turn
what the world reports into hits; `Detached`, the part that came off as
data for the item package. Stubbed, each waiting for the kernel or a
later package: raising `hit_incoming` and the events, and handing on
exits and detached parts; making items of what came off and applying
`WornHit`s to worn items (E8); deciding when a body in pieces ends its
creature (E7); which parts face a hit's direction; a pool that names a
substance (G19). The scripts `one_bad_cut` and `through_the_wall` are
integration tests in `crates/clash-sim/tests/body.rs` until the kernel
holds bodies: the first in full against a timeline the test drives, the
second only as far as impact reports becoming hits; both scripts under
`scenarios/people/` are owed. No benchmark (Z1).

E4 and E5 joined to the simulation since (decisions D-550, D-553):
`Sim::attach_body` hands mass and size to `Physical` and the world;
`Sim::hit` raises `hit_incoming`, applies the hit with the `hits`
stream, raises every change, reports what came off as `part_detached`
and carries exits on as `hit_exited`; `slots_lost` and `slots_gained`
are raised for the action runtime to interrupt what held a slot;
`function_changed` works the stats out again; the world's
`occupant_exposed` and `occupant_dosed` become a hit and a dose
(through `exposure_incoming`), and so do `body_impact` of an entity
and `body_hit_occupant` (`Sim::body_impact`); `hit`, `close_wound`, `reopen_wound`,
`heal_part`, `disable_part`, `restore_part` and `remove_part` are
verbs. `one_bad_cut` and a collapse that hurts whoever stands under it
are integration tests against the real simulation
(`crates/clash-sim/tests/life.rs`). Hits go through what the item package says is worn, damage it, and
make items of parts that came off. Still stubbed: the other verbs of
06 §6.5 (E6), test
scripts, since the simulation does not implement the console's host.

### E6 — Body editing

**Spec** 06 §6.5 · **Size** M

Build
- Every verb of 06 §6.5, with socket validation and recomputation.

Done when
- Invariants B2 and B3 hold after random sequences of edits (property
  test).
- Scenario `reshaping_touch`: a rule adds, removes, replaces and
  re-materials parts of a target.

Beta: built in `clash_sim::body::edit` and `clash_sim::reshape`
(decisions D-636 to D-639), over the kernel's body wiring (D-550). Built:
every edit of 06 §6.5 as a function of the body model, and `add_part`,
`replace_part`, `graft_part`, `set_layer`, `scale_part`, `scale_body`,
`set_body_plan` and `damage_part` as verbs beside the body package's
`heal_part`, `disable_part`, `restore_part` and `remove_part`; socket and
name checks; the hook `body_changing` and the event `body_changed`; the
events of 06 §7.6 for what changed, `part_detached` and items for what
comes off, what is grasped and worn let go; nine new reasons. Tests in
`crates/clash-sim/tests/reshape.rs`: a property test of B2, B3 and the
tree under random edits, and `reshaping_touch`, which runs the fixture
rule `reshapefix:apply_form` against the simulation. Stubbed: a footprint
that follows a change of plan; a `.scn` script; no benchmark.

### E7 — Minds and vital state

**Spec** 06 §8 · **Size** M

Build
- `Mind`; seated attributes and tracks; the four mind verbs; drivers.
- `die` and `revive` with `entity_dying`.

Done when
- A transferred mind takes control mode and mind-seated values and leaves
  body-seated ones.
- A driven body acts; a mindless undriven one does not.
- A cancelled `entity_dying` prevents death.

Beta (decisions D-554, D-555): `clash_sim::mind`: the `Mind` and
`Driver` components; vital state derived from data, `needs` on body
plans and `stops` on pools; `die` (after `entity_dying`), `revive`,
`create_mind`, `remove_mind`, `transfer_mind` (after
`mind_transferring`, with seated stats and control) and `set_driver`
as functions and verbs; `Sim::can_act` with the reasons a command for a
dead, unconscious or mindless entity is refused. Tested in
`crates/clash-sim/tests/life.rs`. Stubbed: beliefs, relations and
senses moving with a mind (15); the `mind` type (09); abilities after a
transfer (08 §3.4). Joined to the action runtime (decision D-650):
every start is checked against `can_act` for anything with a mind or a
body, death abandons running actions and unconsciousness interrupts
them, asking no one, and waking asks again; tested in
`crates/clash-sim/tests/acting_alive.rs`.

### E8 — Items and holds

**Spec** 06 §9 · **Size** L

Build
- The `item` type and component; surfaces; item damage; stacks.
- `Holds`, the holding verbs, capacity and grasp checks.
- Worn items as outer layers in hit resolution.
- Latent contents.

Done when
- A worn layer absorbs by the packet rule and is itself damaged.
- A held item is dropped when the holding part's slot is disabled.
- Latent contents are identical whenever they are first materialised.
- Invariant B6 holds under random holding operations.

Beta, in `clash_sim::item`, tested in `crates/clash-sim/tests/items.rs`
over the fixtures in `crates/clash-sim/tests/fixtures/item`: the `item`
type with every field of 06 §9.1 that has a meaning yet, `modifiers` and
`holder_tags` (D-561), and `contains`, `guards`, `grants`, `traits`,
`rules`, `ages` and `keeps` carried; the `Item` and `Holds` components;
`create`, a debug command `CreateItem`, colours picked by seed;
grasps on the channel slots of working parts, wearing at layers over the
parts a `wear` selects, containers with volume, mass and tags, all through
`relocate`, which now tells the holder when anything leaves it (D-560);
what a grasped or worn item confers, applied and taken back exactly
(a property test against recomputation); stacks that split and rejoin
(D-562); item damage, broken items, worn layers handed to `body::apply_hit`
and damaged by what they absorbed (D-563); what a disabled, severed or
destroyed part held, and parts that come off as items (D-565); latent
contents from a container's own list and seed (D-564); fourteen verbs,
eighteen properties, ten events and two hooks, twenty-one reasons
(06 §9.2, §13); plain items in `mods/core/data/items.json`, with the
channel `arm` they grip by. Stubbed, each waiting for what it needs:
ageing and the script `the_larder` (temperature, V1); `gas_mask` and what
`guards` keep out (exposure by route); `round` and ammunition (C-series),
`look` (U4), `interactions` and `device` (E12, G1); `content_table`
(WG8); the kernel applying hits and body changes, which is when it calls
`worn_layers_of`, `apply_worn_hits`, `part_item` and `on_body_changes`;
the kernel joining stats, when `confer::with_stats` stops building the
tables for each change and the `attr_changed` of conferring is raised;
seats and fixed holds (vehicles, devices). A simple item measures about
400 bytes with the kernel's record, over the 300 of 06 §15; the item's
own component is 56.

### E9 — Templates and creation

**Spec** 06 §10 · **Size** L

Build
- `template` and `creature`; every choice kind; character specs.
- Validation of `assemble`; default and random selection.

Done when
- The same spec and seed give the same entity hash (invariant B4).
- Each choice kind has passing and failing fixtures.
- A fixture species with its own choices builds without engine changes.

Beta (decisions D-556 to D-558): `clash_sim::template`: the `template`
type with every kind of choice and options that nest; character specs
with default and seeded random selection; `Sim::spawn_template`, the
verb `spawn`, the debug command `spawn` and the answer to
`spawn_requested`; the creation session for `app.creation`, which the
application serves with the service `creation` (D-653); anonymous
and notable as far as spawning needs; `person` and `dog` in
`mods/core/data/life`. Stubbed: the `creature` type; adding the parts
an `assemble` choice chose (E6); option conditions and a range's
`apply`; traits and items, which are carried as ids for E3 and E8;
`apply_template` on an existing entity; test scripts.

### E10 — Movement

**Spec** 06 §11 · **Size** M

Build
- `movement`; `move`, `sweep`, `leap`, `set_altitude`, `teleport`; the
  occupancy rule; swapping places with a creature that yields, through
  `passage_requested`; loss of footing into a world body and back.

Done when
- Step legality is tested against a table of geometry cases.
- A leap lands where the analytic trajectory says.
- A swap relocates both creatures in one step and is refused where the
  hook does not grant it.
- Scenario `dash_through_wall`: a sweep at high speed strikes a wall as a
  body impact.

Beta: built ahead of the action runtime, in
`clash_sim::movement` (decisions D-570 to D-576). Built: the `movement`
and `posture` definition types with every field of 06 §11 and §11.1;
the `mover` component (ways of moving, posture, path, where a fall set
off), saved and hashed;
a mover's rules as the world's `MoveRules`, from its definitions, its
posture, its body's slots and its `speed` expressions; the verbs `step`
(06 §11's `move`), `move_to`, `set_posture`, `plan_path`, `follow_path`,
`clear_path` and `set_movement`; `step_time` and the properties, events,
hooks and reasons tabled in 06 §11; the occupancy rule and the swap
through `passage_requested`; paths planned from belief behind
`movement::belief`; loss of footing on `entity_unsupported` into a world
body (W14) and back on `body_landed`; and the
actions `step`, `go_to` and `set_posture` in `mods/core/data/movement.json`.
Step legality is tested against a table in `movement::rules`, the rest in
`crates/clash-sim/tests/movement.rs` over the test city. Stubbed, each
waiting for the package that owns it: `sweep`, `leap` and the scenario
`dash_through_wall` (they launch a mover on purpose, as a fall now
does), `set_altitude` and `teleport` (realm transfer); catching a
falling entity; belief reads truth (perception, S2); a posture
scales the `Physical` height but not a body's bands, and gives no traits
(bodies in the simulation). The tests of time and paths start `step`
and `go_to` with `Command::Act` through the action runtime (T3); the
hand-made loop that stood in for it is gone.

### E11 — Coming and going

**Spec** 06 §12 · **Size** L

Build
- `population`; anonymous instantiation; the notable triggers; storage
  with tracts; the agent record; `make_notable` and `make_anonymous`.

Done when
- Reloading a tract shows the same anonymous individuals (invariant B7).
- Each trigger of 06 §12.2 makes an individual notable.
- Stored individuals round-trip with identical hashes.

Beta (decisions D-783, D-789), in `clash_sim::agent`: a settlement's
people are residents keyed on home and index with seeds of their own,
made the same each time (B7); anonymous ones forgotten and notable ones
stored as agents when their tract unloads; every trigger of 06 §12.2,
`make_anonymous`. Tested against `core:town` in
`crates/clash-save/tests/town_life.rs`. Stubbed: populations as counts
with presence tables; a population count adjusted by deaths (a resident
is `gone` instead).

### E12 — Minimal devices

**Spec** 02 §5.5; 06 §9.2 · **Size** M

Build
- The `Device` component; promotion of an interactive element to a device
  on demand and demotion at default state; contents; the `Inside` hold.
- Enough interaction to open, close, put in, take out and hide inside.

Document 16 extends this with links, verbs and tools.

Done when
- Ten thousand unused lockers create no entities.
- Destroying an element with a device spills its contents and exposes
  anyone inside.

Beta (decision D-621), in `clash_sim::device`: the `device` type with
`name`, `holds` and `tags`; the `Device` component; promotion when an
element's state leaves its first and by the verb `promote`; demotion
when it is plain again; contents stowed or enclosed (`Inside`) through
`put_in` and `take_out`; destruction or replacement spills them with
`device_spilled`; three events, three properties, four reasons, and a
load check of every element's `device`. `tests/devices.rs` covers both
items above and a save in the middle. The interactions that open, close,
put in, take out and hide inside are package G1's content. Stubbed:
links, variables and timers of devices beyond the kernel's, and
everything of package G2.

## Perception

### S1 — Light

**Spec** 15 §3.3 · **Size** M

Build
- `Cosmos::light`; sky spread; fixed sources; the per-chunk cache; moving
  sources at query time.

Done when
- The cache equals recomputation after edits (invariant P5).
- A room is lit near its windows and dark beyond the cap.

Beta: everything in the list above is built, in `clash_world::light`
(decisions D-410 to D-415), with the `sky` definition type and `light`
on elements. Stubbed, each waiting for the package that owns it: items,
creatures and fields that give light are handed in by the simulation as
moving sources (fields give none until W16); the weather's share of sky
light is a number the caller gives; a realm's day comes from its sky
until calendars exist; `light(loc)` is not yet a function for rules; no
test script (the console has no simulation behind it yet) and no
benchmark, only a timing test that prints.

### S2 — Signals and senses

**Spec** 15 §3.1, §3.2, §4 · **Size** M

Build
- `signal_kind`, `sense`, `event_signal`; the three propagation models;
  delay; signatures; `emit_signal`; facing.

Done when
- Each model matches hand-computed cases, including blocking by materials.
- Scenario `slow_sound`: a sound arrives after distance over speed, and an
  entity on a fast clock gets there first.

Beta: built in `clash_sim::perception` (decisions D-610 to D-613): the
three types, the three models with blocking by a table on the signal
kind (D-611), delay with a front that follows a moving observer,
standing signatures, `emit_signal` as function and verb, facing, and
sight, hearing and the loud events of the world in `mods/core`. Covered
by `crates/clash-sim/tests/perception.rs`, `slow_sound` among them, as a
Rust test (the console has no simulation behind it yet). Stubbed:
`exposes` is read and does nothing until the stats are wired (so
`behind_lead` is not built); signatures from templates and traits,
`signals` on actions and `noise` on movement wait for those packages,
which call `set_signature` and `emit_signal`; no bundled `field` kind.

### S3 — Detection

**Spec** 15 §5 · **Size** L

Build
- Awareness storage and update; the triggers of 15 §5.2; events;
  concealment through the existing factors.

Done when
- Results do not depend on evaluation order (invariant P4).
- A strong signal detects at once; a marginal one takes the time the rates
  imply.
- Scenario `the_locker`: a hidden character is not seen, is heard if it
  makes noise, and the searcher's track ends at the last known position.

Beta (D-614): awareness, sweeps staggered by hash, the triggers of
15 §5.2 bounded as that decision says, the three events, concealment by
light, cover, arc, planes, containers and fields; warm observers do not
sweep. Each "Done when" is a test, `the_locker` as a Rust test; an
ignored timing test measures 15 §17 (over budget for a crowd). Stubbed:
promoting a warm observer that receives a strong signal waits for
attention (09 §9.3); the `search` and `listen` actions call `search`
when the actions package builds them.

### S4 — Percepts

**Spec** 15 §6 · **Size** M

Build
- The `perceived` event and its filters; `perceives`, `could_perceive`;
  use by window kinds.

Done when
- Scenario `heard_not_seen`: a rule on `perceived` sends a character to
  investigate a sound it could not see the cause of.
- A window is noticed only by candidates who perceive its source.

Beta (D-615): `perceived` with what the observer made out, `perceives`
and `could_perceive` as functions and as a mind's own properties, and
`notices_source` for whoever builds windows. `heard_not_seen` is a test
that a window broken out of sight is heard with what a rule needs, and
that such a rule loads and is not flagged; the rule as 15 §12 writes it
is kept in `tests/fixtures/perception/heard_not_seen.json` until
`enqueue_action` exists. Stubbed: watched things and `discrepancy`
(review addition "Watched things and discrepancies") wait for maps to
mark things `watched` and for routines; windows call `notices_source`
when package T5 builds them.

### S5 — Beliefs

**Spec** 15 §7 · **Size** M

Build
- Tracks and bounded memories; staleness; the properties of 15 §7.2.
- The linter check of 15 §7.3 and the `hidden` tag rule.

Done when
- Belief properties are `null` for the never-perceived.
- The linter flags a direct read of another entity's location in a rule
  attached to a creature.

Beta (D-616, D-617, D-619): tracks by persona (the entity's own until
S6), bounded memories and noted things, confidence and salience that
fade, observed capabilities on tracks (review addition), the properties
of 15 §7.2 and their companions read of the mind itself, the linter
check, the `hidden` tag rule, and believed movement rules. Stubbed:
personas; a capability told of in speech waits for S8; the queries
`tracks` and `memories`; the linter does not yet check effect
arguments.

### S6 — Personas

**Spec** 15 §8 · **Size** M

Build
- Personas, presentation, appearance; `identity_check`; the persona verbs;
  illusions.

Done when
- Scenario `frame_job`: witnesses' memories name the copied persona while
  the world's cause names the actor.
- Seeing through a disguise links the two personas in that observer only.

Beta (D-760, D-761, D-766, D-769): personas with records, presenting,
copies, `identity_check` at detection, links in one observer only, a
change of face in view seen, illusions, the persona verbs, tracks and
memories by the persona the observer takes someone for, and the reader
lens that names a known persona and describes a stranger by the
template it looks like. Both "Done when" items are tests in
`crates/clash-sim/tests/social.rs`, and `frame_job` is a test script
(`scenarios/social/frame_job.scn`). Stubbed: features and descriptions
(review addition "Features and descriptions", test script
`a_tall_figure_in_red`) and `suspects`; pronoun sets are carried, not
read (review addition "Pronouns").

### S7 — Relations, groups, orders

**Spec** 15 §9, §10 · **Size** M

Build
- `relation_kind`, `group`; relation storage and fade; group fallback and
  claims; group knowledge; `order` and `order_received`.

Done when
- A member with no personal relation uses its group's; a claimed uniform
  applies the group's stance until seen through.
- Scenario `minion_orders`: a created servant obeys while the relation
  holds, must hear the order, and stops obeying when the relation is
  removed.

Beta (D-762, D-763, D-765, D-767): `relation_kind` and `group`, relations
with fade, group fallback by the strongest, claims (a uniform holds until
seen through), group knowledge, groups made and dissolved in play,
templates' `groups`, `order` heard and obeyed by the hearer's relation
flagged `orders`, `order_received`, `cancel_order`, `until`, orders
withdrawn when the relation goes; `relation_kind`s, groups and the rules
`do_as_ordered` and `stop_when_released` in `mods/core/data/social`. Each
"Done when" is a test, and `minion_orders` a test script. Review
additions: "Relations that intervene" is `social::intervenes`, which
windows ask: a test opens a window against someone and their bodyguard is
offered it by relation. "Roles that spend" is recorded on the group definition, not yet
readable by rules. Stubbed: a role's `orders` is not enforced;
collectives; group variables. Minds (09 §6.6) take their stances from
relations once they have perceived someone: hostile where `stance_to` is
`opposes`, feared where a kind flagged `avoids` is held (D-762).

### S8 — Speech

**Spec** 15 §11 · **Size** M

Build
- `say`, `lines`, `message_kind`; `message_heard`; `understands`; reports
  transferring beliefs; exchanges; `talk_options`.

Done when
- A report gives the hearer the memory at reduced confidence and marks its
  source.
- Knowledge reaches a group and is used by a third member.

Beta (D-764, D-765): `message_kind` and `lines`, `say` carried by a
signal and heard through perception, `understands`, `message_heard`,
reports that give memories and tracks as told at reduced confidence
with their source, overheard messages, `talk_options` as a function for
the application; message kinds, lines for greetings, warnings and calls
for help, and the rules `heed_help` and `pass_reports_to_the_watch` in
`mods/core/data/social`. Both "Done when" items are tests. Stubbed:
exchanges (joint actions whose phases say lines, not yet written), the `talk` interaction, capabilities told
of, languages as definitions. The application's third pass (D-799)
offers `talk_options` on `ui:talk` and says the one chosen with the
action `say`, with the rules `offer_greeting`, `offer_warning` and
`greet_back` in `mods/core/data/talk.json`.

### S9 — Party perception

**Spec** 15 §13 · **Size** M

Build
- Tagging of output events by how the party perceived them; map memory; the
  omniscient switch.

Done when
- An event out of sight and earshot is tagged unperceived.
- Map memory records what was there when last seen, not what is there now.

Beta (D-618): tags (`Sim::party_perceived`), map memory, frames for the
view model and a snapshot, the omniscient setting; both "Done when"
items are tests. Stubbed: moving sources of light are not counted in
what the party sees. The application drains the frames, feeds the view
model from them, the map memory and the cells in view, and answers
playback with the tags (12 §3.5, decision D-700;
`crates/clash-app/tests/perceiving.rs`).

### S10 — Coloured light (experimental)

**Spec** 15 §3.3; 12 §6.1; 04 §8.4 · **Size** S

This package is experimental. No milestone depends on it and no bundled
content may use it. Part of its purpose is to find out what it costs and
how it looks. Record what is found against the table of unresolved
questions in 15 §3.3.

Build
- `color` on light sources; `Cosmos::light_color` cached with light; the
  tint in cell drawing.
- Registration under the experimental feature `coloured_light`.

Done when
- A cell lit by two sources is tinted by their colours in proportion to
  what each contributes.
- State hashes and detection results are identical with the feature on
  and off.
- A mod that gives a light a colour without opting in fails to load, and
  one that opts in is warned.
- The cost of the cache is measured and written into 15 §17.

## Presentation

### U1 — Scene skeleton, frames, themes

**Spec** 12 §3, §4 · **Size** M

Build
- `clash-scene`: `Frame`, sprites, effects; glyph resolution; `theme`; the
  `Frontend` trait; the null frontend; the golden-frame harness.

Done when
- A frame serialises to text and compares with a stored one.
- The scene type exposes no route to the simulation except `submit`
  (invariant V1), shown by a compile-fail test.

Beta: built ahead of the simulation (K2). Stubbed: the scene's link to
the simulation is the `SceneHost` trait with `submit` and nothing else, so
there is no `SimView` and the compile-fail tests show only that the host
cannot be reached; they must be joined by tests against the real handle.
Definitions come from the mod loader (decision D-322):
`clash_scene::register_defs` registers `color`, `theme`, `keymap`,
`figure` and `screen`, and `UiDefs::from_defs` makes themes and scenes
from what loaded, by qualified id. The linter's warning for glyph states
that differ in colour alone (invariant V10) waits for the linter. A
glyph's `shape` is checked and can be read by facing; nothing draws by
it yet (see U4).

### U2 — Terminal frontend

**Spec** 12 §11 · **Size** S

Done when
- A frame is displayed in a terminal with colour mapping; sprites snap to
  cells; keys arrive as input events.

Done, with one caution: it was built and tested against an in-memory
writer and translated events, and has not yet been watched in a real
terminal. `cargo run -p clash-front-term --example gallery` is the thing
to try first.

### U3 — Window frontend

**Spec** 12 §11 · **Size** L

Build
- Window, glyph atlas from the theme font, the grid and sprites drawn on
  the processor into a pixel buffer shown with `softbuffer` (D-830), frame
  effects, keyboard and mouse with cell coordinates; drawing to an image
  file for tests.

Done when
- The same frame renders in both frontends with the same characters in the
  same cells.
- A frame drawn headless to an image matches its golden image.
- The drawing budget of 12 §15 is benchmarked.

Done, with one caution: the window itself has not been seen on a
screen. The renderer, the atlas, the input mapping and the choice of
frontend are tested without one (decisions D-840 to D-844): golden
images for a plain frame, sprites and each effect; the widget gallery
drawn by the window compared cell by cell with the terminal's golden
frame; the benchmark at 4.2 to 9.5 ms against 16 ms. Where it was
built there is no display, and under a virtual X server the window
system's library `libxkbcommon-x11` was missing, so `winit` refused (the
application then fell back to the terminal, as it should). On a machine
with a display, `cargo run -p clash-front-window --example effects` is
the thing to try first, then `cargo run -p clash-app --release`. Left
as gaps: a theme chosen during play does not change the window's font;
no paste; a font for bold or italic of its own.

### U4 — View model and world drawing

**Spec** 12 §5, §6 · **Size** L

Build
- The view model from map memory and percepts; cell composition; look-down;
  light and memory tint; subgrid compositing.
- `map_view`, `section_view`, `region_map`; camera.

Done when
- Golden frames for the test city at several storeys and in section.
- Unperceived changes do not appear; the omniscient mode shows them.

Beta: built ahead of perception (S9) and playback (U5), which will feed
it. Built, in `clash_scene::view`: the view model as plain data with its
observe calls and the omniscient fill from a `Cosmos` (decision D-350);
the glyph tables compiled once per theme from the world's definitions
(`WorldGlyphs`); cell composition by 12 §6.1 with look-down, light,
ambience and memory; the camera; `map_view` with zoom, cursor, heard
markers and overlays, and `section_view`, both drawn by `WorldView`,
which is the toolkit's `ViewPainter` once `Scene::set_world` has given
it to the scene; the `ambience` and `overlay` definition types, loaded
and checked. `cargo run -p clash-front-term --example worldview` shows a
small test world; its golden frames are the test town at each storey, in
section both ways, zoomed out, half remembered, looking down a hole and
in low light, and its tests show that what is put into the world shows
only once observed or after an omniscient refill. Stubbed, each waiting
for what it needs: nothing but tests and the omniscient fill puts
anything into the view model, and the check that caught-up playback
equals a fresh initialisation has only the equality it will use (S9,
U5); `region_map` and `chart_view` draw an empty area with their name
(W8 to W10, W21); subgrids are not composited and nothing is drawn
through portals (the world has neither); shapes of several cells,
`by_level` and `layer` are not in the world's `GlyphSpec` and are not
drawn for elements; fields and decals have no definitions, so their
looks are set on the glyph tables by hand (W16); the omniscient fill
records no fields, occupants or items and takes light to be full (W7,
W16, S1); an ambience's particles are kept and not drawn (U5); looks
(12 §4.4) and coloured light are not drawn (U5, S10); an overlay's
`cell` is evaluated through the `OverlayValues` interface, which only a
test implements (U6b), and `networks`, `areas`, `jobs` and `temperature`
draw nothing; the camera is moved by the program, not by controls (U7).
No benchmark: a 200 by 60 view of known cells measured once at 0.5 ms,
and 0.9 ms looking down six storeys everywhere, against the 1.5 ms of
12 §15.

### U5 — Playback

**Spec** 12 §7 · **Size** L

Build
- Event ingestion; pacing; gliding and trails; `show` and `anim`; the log;
  skipping.

Done when
- After any scripted playback the view model equals a fresh one
  (invariant V4).
- Golden frames for `wall_punch` at fixed time steps.
- Every shown event has a log line (invariant V5).

Beta: built in `clash_scene::playback`. The application calls
`Scene::push_events` with each drained batch, `Scene::advance` every
frame, `Scene::playback_caught_up` to know whether input applies to the
present and `Scene::skip_playback`; in debug builds
`Scene::check_caught_up` with a fresh view model. The queue paces each
stretch by 12 §7.1 (groups by tick, long waits cut to two seconds,
`stretch_time`, `min_time`, the `playback_speed` preference with 0
instant, catching up from any backlog, the `skip` key); the `show` and
`anim` definition types say how each event changes the picture, what it
animates, its line worded for the reader, what is shown if it was only
heard, its importance and a sound cue (decisions D-600 to D-602); the
host says what the party perceived and what it sees
(`SceneHost::perceived`, `seen_cell`, `seen_sprite`, D-603); the
message log folds
repeated lines, keeps a scrollback and is wrapped, filtered by
importance and kept at the newest line by the `log` widget (D-604);
bindings are asked as of the moment playback has reached
(`SceneHost::as_of`, D-605); glides with afterimages, animations at a
cell or along a path, shake and flash are drawn over the picture
(D-606); `mods/ui/data/playback.json` has shows for every kernel and
world event and the preference `effects` (D-607); a decision's prompt
waits for playback. Tested through the null frontend in
`crates/clash-scene/tests/playback.rs`: golden frames of a wall broken
three ticks after a step at fixed time steps, of a heard event and of a
projectile, and a property test that caught-up playback equals a fresh
view (D-608). The game binary plays every batch through it, its host
answering from the simulation's perception (D-700), and an event without a
show no longer moves a thing ahead of the step that moves it (D-655,
D-656). Stubbed, each waiting for what it needs: perception
answers through a stand-in in `clash_scene::testing` in the scene's
own tests (the game's through the simulation); events carry
no duration, so a glide lasts its show's `time`, and bodies in flight are
drawn where they start, strike and land and not along their paths, which
no event carries (W14); floating text, particles and effects
shaped by an event's fields are not built; cutscenes cannot yet choose
when the events of their span are played (U10); `message` effects reach
the log through the application, at once and not as they are played;
no event is marked player-relevant, so the linter's V5 warning is
not given and steps write no line; events of other packages (hits,
wounds) have no bundled show; `stretch_time` is set from Rust until
settings exist (P6); looks (12 §4.4) are not drawn, since no event
reports one yet (E3, E10); no benchmark of ingesting events.

### U6a — Toolkit: layout and widgets

**Spec** 12 §10.1–10.3 · **Size** L

Build
- The `screen` type; layout containers; every widget of 12 §10.3 with
  static content; focus and navigation.

Done when
- Layout fixtures at several sizes have golden frames.
- Each widget has a golden frame and a navigation test.

Beta: every container and widget is laid out, drawn, focused and
navigated with content written in the definition. Stubbed, each waiting
for the package that owns it: a property that is an expression is held
unparsed (`Prop::Bound`) and shown as a placeholder until an `Evaluator`
is supplied, and rows and options from a query are empty (U6b); effects
are held as written and handed back in `UiEvent`s, not run (U6b);
`map_view`, `section_view`, `region_map` and `chart_view` have their
place, focus and fields and draw an empty area unless given a
`ViewPainter` (U4); `log` shows a `MessageLog` that nothing fills yet
(U5); `slot` holds its name (U8); `console`, `canvas` and `trace_view`
have their place and focus only (tooling plan); rebuilding the screen
stack on reload (12 §10.3) is not built; a `number` is stepped, not
typed. Screens are made from loaded definitions by `UiDefs::screen` and
checked by the linter against every loaded theme; the figures and art a
screen names are plain text, not references the loader resolves, until
U6b (decision D-322).

Since U6b and U8: bound properties are compiled and evaluated, rows and
options come from expressions, effects are typed and run, slots are
filled and a `number` can be typed into. Still stubbed: rows from a
query (K6), the views beyond U4, the log's content (U5), the
development widgets, rebuilding the stack on reload, and the figures
and art a screen names, which are still plain text (D-322).

### U6b — Toolkit: binding and UI effects

**Spec** 12 §10.4, §10.5 · **Size** M

Build
- Pure-context bindings with the names of 12 §10.4; re-evaluation; the
  perceived-only rule for ordinary screens; UI effects; the `Ui` command
  and `ui_callable`.

Done when
- A binding that would read truth about a non-party entity is a load error
  outside `debug` screens.
- A `run` effect appears in the input log and replays.

Beta: every bound property and every expression of an effect is
compiled when mods load, by `ScreenDef` and `UiPartDef` in
`Def::compile`, in a pure context with `view`, `party`, `app`,
`screen.name`, `$name` and `it`, and outside `debug` screens in a mind
context whose own are the party; mistakes are placed inside the string.
The scene evaluates what is on show when it is told the simulation has
advanced or a variable has changed, against a host it is given
(`SceneHost::view`, `party`, `reader`) and the application's state
(`AppState`). Effects are read into typed effects at load and run on a
screen stack with popups; `confirm` and `pick` open the screens of the
roles `confirm` and `picker.list` and go on when they answer; `act`,
`command` and `run` hand the host a whole `UiCommand`; `app` services
reach the application. Tested against the fake host and application of
`clash_scene::testing`, since no simulation exists to run against.
Stubbed, each waiting for what it needs: the `Ui` command itself,
`ui_callable`, the input log and replay are the kernel's (K3, K7): the
scene's part is that the command it hands over is complete and the
same every time, which a test shows; `show_screen` from a rule waits
for the event stream (K2, U5); `act` with `"needed"` or `"all"` only
marks the command, and choosing an action with `pick` and the other
pickers are U7; bindings show the simulation as it is, not the moment
playback has reached (U5); rows cannot come from a query (K6); a
view's `centre` and an `inspect`'s `of` are compiled and not used;
`app.creation` is whatever the application says (U9, E9); no benchmark
of re-evaluation. Decisions D-430 to D-436.

### U7 — Input, controls, pickers

**Spec** 12 §9 · **Size** M

Build
- `keymap`, `control`; `act` with `needed` and `all`; pickers; mouse
  behaviour; decision screens.

Done when
- One control starts an action from defaults in one key press; its precise
  twin asks for every parameter with defaults pre-filled.
- Rebinding a key changes behaviour without touching controls.

Beta: the `control` definition type, run after a screen's `keys` and
before the toolkit; keymap contexts `play` and `targeting`; `rebind`
at once, reported to the application, with `Scene::rebind` to give
stored bindings back. `act` with `"needed"` and `"all"` reads the
action's parameter list from `clash_sim::action::ActionDef`, works
defaults out in the actor's mind through the host, opens one picker for
each parameter asked for and submits one whole command; closing a
picker abandons it. Pickers in `mods/ui` for an entity and a cell (a
cursor over the map with reach, sight and the line), a direction, a
number, a text, one of a list and several of a list; the `camera` and
`picker` effects; clicks and the wheel on a map view; prompts for
reactions and choices from a `PendingDecision` the host hands over; the
play screen, look mode and placeholder screens; the default profile's
roles. Tested through the null frontend with scripted input and golden
frames against the fake host, with actions from a fixture mod, since
the action runtime (T3, T4) does not exist. Stubbed: several characters
at once (12 §9.6) and the route picker's numbered path with `check`
beside each step (review additions; a list of cells is asked for with
the cell picker, one step at a time); the inspection tooltip on the map
(U9); the simulation's side of `pending_decision`, `candidates` and
the `react`, `pass` and `choose` commands (T4, T5, K9); a keymap name
that nothing uses and a control's missing action are not reported at
load. Decisions D-530 to D-537.

### U8 — Slots and theming

**Spec** 12 §10.6, §4.3 · **Size** S

Done when
- A `ui_part` from a second mod appears in a slot in order, subject to its
  condition; switching theme restyles every screen.

Done: the `ui_part` and `preference` definition types; a slot is filled
with the parts aimed at it, in order, each shown while its `if` holds,
and a part aimed at a slot no screen places is a load error unless it
says `"required": false`; the `theme` preference restyles every open
screen, each keeping its state; the `ui` mod defines the built-in
preferences and a second theme. A test loads a part from a second mod,
and a golden frame shows the second gallery in each theme.

### U9 — Bundled screens

**Spec** 12 §10.7 · **Size** L

Build
- The `ui` mod's screens, including character creation driven by template
  data.

Done when
- Scripted input goes from title to a created character in play; golden
  frames along the way.
- A fixture species with every choice kind is fully usable on the creation
  screen without a new screen.

Beta: character creation (`ui:create`, the role `creation`, and its
popup `ui:create_choice`) over the creation session through
`app.creation`, `app.templates` and the service `creation`, every
choice kind of 06 §10.2 on the one screen; the inventory (`ui:inventory`)
with what is held, worn and carried and the item verbs as actions
(`ui:item`); the condition (`ui:condition`) with the body as a tree of
parts, its wounds, and slots for reserves and attributes, which the new mod `core_ui`
fills; look mode's side panel (`cursor_things`, `cursor_ground`); the
keys of play (`ui:help`, `app.keys`); the play screen's status column
with what is held and a line of keys. Every bundled screen is listed
with its role, parameters and slots in 12 §10.7. Tested against the
fake host over the real registry and the fake application's real
creation session, with golden frames (`clash-scene` test `screens`):
from the title through creation into play, each choice kind, a
refused selection, each screen, and the way to each from play and
back. Third pass (decisions D-790 to D-799): `ui:abilities` lists
`view.ability_uses` and `view.ability_modes`, uses an ability with its
parameters asked for but those its grant supplies, switches a mode, and
puts a use on a quick slot (the preferences `slot_1` to `slot_9`, the
keys 1 to 9 in play); `ui:map` draws what the character knows zoomed
out with the journal's markers pinned, the sites it knows by name
(`view.known_sites`) and the objectives; `ui:help` lists every control,
bound or not (`app.controls`); `ui:context_menu` offers attack, shoot,
talk, go there, use and look; `ui:talk` lists what can be said
(`app.talk`, which the service `talk` gathers) and says one with the
`core` mod's action `say`; the reaction prompt shows `what` is happening; the pick list greys
an action that cannot be done with its reason; the fighting controls
are in `fighting.json`; the item popup throws. The `core_ui` mod plays
blows, powers and speech back. Tested in `clash-scene` (`controls`, `screens`)
and the application's tests; the golden frames `help`,
`inventory_item`, `reaction_prompt`, `play_first`, `play_status`, the
pickers' and the playback's changed for the status column's line of
keys, the keys screen, the Throw button and the prompt's line of what
is happening. Stubbed: the vital state, a pool's thresholds and the list of an
entity's pools and attributes, which the simulation does not register
(D-665); the result's body as a figure and a tree of parts, traits, a
part browser for `assemble` and a text's suggestions; the application's
side of `app.templates` and of `app.creation.attributes`, which the
game binary does not give yet: until it lists the templates, the
creation screen in the game has nothing to begin a character from;
the actions the inventory names, which no mod
defines yet; an item named by its definition, which waits for the
reader; the case file, codex, trade, fitting, make, build, work,
device, know-how and the helm; the region map and the chart.
Decisions D-660 to D-669, D-790 to D-799.

### U10 — Cutscenes

**Spec** 12 §8 · **Size** M

Build
- Cues from actions, definitions and the effect; shots; `release`; skip;
  tier and cooldown preferences.

Done when
- Scenario `cutscene_takedown`: golden frames; skipping yields the same
  final view model (invariant V6).

### U11 — Application

**Spec** 12 §3; 04 §3.3 · **Size** M

Build
- `clash-app`: the loop, profile boot, saves, preferences, frontend choice.

Done when
- Launching with a profile shows its boot screen; a session can be saved,
  quit and resumed.

Beta (the first walk): `clash` loads a profile, shows located load errors
and leaves with failure, opens the boot screen, offers the `start`
definitions on `ui:new_game`, makes a world and a character, and runs the
frame loop in a terminal or headless from a script (12 §3). Preferences
and key bindings are kept in the user's data directory. Golden frames
cover the title, the new-game screen, the first play frame, inside a
house and at a wall; a total conversion and a broken mod are tested.
Joined to the real systems (decisions D-652 to D-656): the play
screen's commands are the simulation's `Act`, `Wait`, `Cancel`,
`ClearPlan` and `Choose` through `host::command_for`, and the loop
follows its decisions; events are played back by the scene
(`push_events`), with the host answering what the party perceived; the
player is spawned from a template, the start's or the one made in the
creation session, which the interface reads as `app.creation` and edits
with the service `creation`. Tested in `crates/clash-app/tests/`
(`first_walk.rs`, `playing.rs`, `creation.rs`); the golden frames
`first_play`, `inside_house` and `wall` changed for the template's glyph
and movement's reason. Second pass (decisions D-700 to D-706): the
stand-ins are gone. The player sees through the simulation's
perception: senses given when the character is made, the view model fed
from the frames, the map memory and the cells in view, playback
answered with the party's tags, the omniscient toggle through
`set_omniscient` (12 §3.5; `perception_shim` is deleted). Walking into
a shut door does its bump default at once or asks, by the preference
`bump`; the key `e` lists `view.offers_near` and starts `interact`;
refusals are told with their reason (12 §3.7). Saving and loading go
through `clash-save` into the data directory's `saves/`: the pause
menu's Save, the title's Continue and Load, the list with name, summary,
time and whether the mods match, a load's warnings on the role
`notice`, and the start's saving policy; a loaded game draws the same
frame and has the same state hash as when it was saved (12 §3.6). Both
"Done when" items are covered: `tests/first_walk.rs` and
`tests/saving.rs`, with `perceiving.rs` and `interacting.rs`. The
golden frames `first_play`, `inside_house` and `wall` changed to what
the character's sight takes in; `door_opened`, `through_door`,
`door_locked` and `carried_on` are new. Game scenarios (decisions
D-750 to D-759): the `start` type is gone; a game begins from a
`scenario` (13 §3): `app.scenarios` lists them, a scenario with a
choice of place or options is set up on the screen of the role `setup`
(`app.setup`), its introduction is shown on `intro`, the simulation
gives the party senses as the game begins (`BeginScenario`), and the
loop carries out the simulation's requests at safe points: checkpoints,
going back to one, the character played now, and an ending, which
stops the game, removes a `single` game's save and shows the role
`summary`. The journal (`ui:journal`, `shift+j`) lists
`view.objectives`. Tested in `tests/scenarios.rs` with the new golden
frames `errand_intro`, `errand_journal`, `errand_summary` and `setup`;
`new_game` changed to list the scenarios. Not built: the window
frontend and frontend choice, switching profiles or reloading mods
while running, and `Ui` commands from screens, saving at intervals
under `single`, a benchmark of the loop.

Third pass (decisions D-790 to D-799): everything the bundled content
offers is reachable from the keyboard. Tab selects a target among the
creatures the party perceives (`app.target`, marked on the map); `f`
and shift+f strike it, quick and precise, from `view.actions_with` (the
simulation's additive `clash_sim::offer`: what could start, with the
reason where not); the precise blow asks for the part from the parts
seen on the target; walking into someone strikes or asks by the
preference `bump_attack`; `s` and shift+s shoot (the place aimed at
filled in with where the target is believed to be, the line drawn), `r`
reloads, `t` throws, `z` stops; the reaction prompt says what is
happening; `use_ability` and `set_mode` reach the simulation as
`UseAbility` and `SetMode`; the application answers a picker for a part
and pins the journal's markers on the view; talking gathers what can
be said on a copy of the simulation (the service `talk`) and says it
with `say`, and the one greeted greets back in the log. Tested in
`tests/fighting.rs`, `shooting.rs`, `powers.rs`, `play_screens.rs`,
`talking.rs` and `reacting.rs`, over the bundled profile and the fixture `armoury` (a
game with a pistol and one with every power), with the new golden
frames `target_selected`, `struck`, `attack_choose`, `attack_part`,
`bump_ask`, `context_menu`, `empty_weapon`, `aim_choose`, `aim_cell`,
`abilities`, `abilities_cooling`, `errand_map`, `help_controls`,
`talk_options`, `talk_reply` and `reaction_prompt`; `first_play`, `inside_house`,
`wall`, `door_opened`, `through_door`, `door_locked`, `carried_on`,
`errand_intro` and `errand_summary` changed for the status column's new
line of keys. Stubbed: previews of a blow (07 §13), since
`strike_preview` reads the target as it is and a screen may not call
it; the region map.

### Z1 — Benchmarks and budget gate

**Spec** 06 §15, 15 §17, 12 §15 · **Size** S, recurring

Status `beta` (D-730 to D-733). Built: benchmarks in `budgets.toml` for
the light rows of 15 §17, one evaluation of sight at 30 m, a signal to
fifty and a simulated second of fifty people walking in a street, and a
200 × 60 frame of the map (12 §15), with their figures in the documents;
a party member's sweep is printed with no row. The performance pass
brought sight under its target and the rest of perception down by about
a third, every result unchanged. Stubbed: 06 §15, memory per mind, the
other rows of 12 §15; the crowd is still over its target (15 §17).

## Scenario catalogue

Under `scenarios/people/`. Presentation scenarios also have golden frames.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `one_bad_cut` | E5 | S14 | Wound to impairment to drain |
| `through_the_wall` | E5 | S1 | Launch, wall, impact injuries |
| `reshaping_touch` | E6 | S17 | Runtime body edits |
| `dash_through_wall` | E10 | S3 | Sweep as impact |
| `slow_sound` | S2 | S3 | Signal delay against a fast clock |
| `the_locker` | S3 | S13 | Hiding, last known position |
| `heard_not_seen` | S4 | S18 | Reaction to a percept |
| `frame_job` | S6 | S15 | Persona against cause |
| `minion_orders` | S7 | S16 | Command relation, orders must be heard |
| `cutscene_takedown` | U10 | S6 | Choreography over decided events |
| `creation_flow` | U9 | S20 | Data-driven character creation |
| `the_open_door` | S4 | — | Watched things and discrepancies |
| `a_tall_figure_in_red` | S6 | — | Features and descriptions |
| `the_larder` | E8 | — | Ageing |
| `gas_mask` | E2, E8 | — | Exposure by route |
| `behind_lead` | S2 | — | Signals that expose |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Shared body parts | E4 | 06 §6.4 | A thousand unchanged bodies of one template store one set of parts; wounding one gives it its own copy and leaves the rest sharing. Measured in the E4 benchmark. |
| Stacks split and rejoin | E8 | 06 §9.1 | Damaging one of a stack of forty leaves a stack of thirty-nine and one item. Repairing it in the same hold lets them rejoin. Counts are conserved throughout. |
| Watched things and discrepancies | S4 | 15 §6; 10 §3.2 | Test script `the_open_door`: a guard who last saw a watched door shut receives a `discrepancy` percept on seeing it open, with what was expected and what was found; a passer-by with no routine at the site receives none. |
| Observed capabilities | S5 | 15 §7.1 | Seeing a persona use an attack records it on the track with its strongest result; the list never exceeds `track_capabilities`; being told of a capability records it as told. |
| Features and descriptions | S6 | 15 §8.4 | Test script `a_tall_figure_in_red`: a glimpse at low clarity yields height and dress only; `suspects` lists every known persona that matches; a later close look adds the face and merges the stranger's track into the right persona. Changing clothes changes `dress` alone. |
| Relations that intervene | S7 | 15 §9.1; 03 §7.1 | A relation kind flagged `intervenes` makes its holder a window candidate. Covered by sim-kernel test script `bodyguard`, run here with real relations. |
| Sound cue names | U5 | 12 §7.3 | A `show` definition's `sound` loads, is validated as a name, appears in the event the frontend receives, and plays nothing. |
| Several at once | U7 | 12 §9.6 | With three controlled characters selected, one move command gives each its own action in entity order, and one answer settles the same pending decision for all three. |
| Roles that spend | S7 | 15 §9.2; 16 §5.6 | A role flagged `spends` is recorded on the group definition and readable by rules. Used by devices package G13. |
| Ageing | E8 | 06 §9.4 | Test script `the_larder`: meat in the open reaches its stage and turns at the times its rate gives; the same meat in a cold box lasts longer by the stated factor; when the box loses power the wake-up moves earlier. Two items in the same stage stack. Needs temperature (vehicles package V1); until then it is supplied by a test fixture. |
| Overlays | U4 | 12 §6.2 | A `map_view` draws an overlay defined by a test mod. Ordinary overlays read the view model only. |
| Posture | E10 | 06 §11.1 | Changing posture scales height, bands, speed and signature as defined; a posture that holds leg slots makes leg actions unavailable; a body with nobody acting for it takes its `limp` posture; `set_posture` forces one. |
| Exposure by route | E2, E8 | 02 §16; 06 §9.1 | Test script `gas_mask`: in a gas that exposes by breath, an unprotected character's pool rises, a masked one's rises by the share the mask lets through, one in a sealed suit's does not rise until the suit's air is gone, and a body that does not breathe is untouched. |
| Signals that expose | S2 | 15 §3.1 | Test script `behind_lead`: a standing source adds to the pool of someone in the open by strength over distance, and adds less behind a wall by that wall's blocking. With nobody in reach the emitter schedules nothing. |
| Looks on traits and postures | E3, E10 | 06 §5, §11.1; 12 §4.4 | A trait and a posture may carry a `look`; it is reported with the events that give and remove them. |
| Colour and unknown names on items | E8 | 06 §9.1 | An item made from a definition with a list of colours carries one of them, chosen by positional seed; `set_color` changes it; `unknown_name` is stored for text. |
| Pronouns | S6 | 15 §8.1; 05 §4.6 | A persona carries a pronoun set from its template or a creation choice; a second persona of the same character may carry another. |
| Colours, fallbacks and inline styles | U1, U2 | 12 §4.3 | `color` definitions load with a word and a fallback; the terminal frontend uses the fallback; `<name>…</>` styles text; the linter warns on glyph states that differ in colour alone (invariant V10). |
| Looks, shapes, own colour and ambience | U4, U5 | 12 §4.1, §4.4, §6.4 | Frame tests: a holder of a look is drawn with it from the moment the event is played and only to viewers with the needed sense; a creature of several cells is drawn by its shape for its facing; two cloaks of one definition are drawn in their own colours, and a witness's description names the same colour; rain is drawn only on cells known to be open to the sky. |
| Route picker | U7 | 12 §9.3 | Frame test: the picker draws a numbered path as steps are added, shows the parameter's `check` beside the step that fails it, lets steps be taken back, and submits the list in order. |
| Presentation testing | U1 | 12 §12 | Golden frames as text for the bundled screens; the view model equals a fresh one after every scripted playback; a test asserts the scene reaches the simulation only through `submit`. |
| Perception at lower detail | S3 | 15 §14 | A warm entity does not sweep; a signal strong enough promotes it; a cold agent keeps its beliefs unchanged while stored. |
| Region map and chart views | U4 | 12 §6.2 | The `region_map` and `chart_view` widgets draw from what the party knows. `chart_view` needs world-model package W21. |
| Widget fields, roles, preferences, overlays | U6a, U6b, U8 | 12 §6.2, §10.3, §10.4, §10.7 | Every widget's fields are as tabled; a profile without a screen for a role fails to load; a preference defined by a test mod appears in options and is readable as `app.pref`; an overlay defined by a test mod draws only what the party has perceived. |
| Screens asked for by rules | U6b | 12 §10.5 | `show_screen` opens a screen for the player who controls `to`, with its values, when playback reaches it; for a character nobody controls it does nothing. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
