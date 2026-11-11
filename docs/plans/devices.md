# Plan — Devices, Networks, Building and Kit

Implements [16-devices-and-machines.md](../16-devices-and-machines.md) and
the conduit layer it adds to the world model (02 §5.1, §6.2).

Packages prefixed W are in [world-model.md](world-model.md); K and T in
[sim-kernel.md](sim-kernel.md); E and S in
[first-playable.md](first-playable.md); V in [vehicles.md](vehicles.md).
The rules for taking a package are those of
[world-model.md](world-model.md) and [CLAUDE.md](../../CLAUDE.md).

Package G3 builds the network evaluator that vehicles use as their bus.
Vehicle package V5 depends on it.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| G1 | Interactions | M | E12, T3 | todo |
| G2 | Device model | M | E12 | todo |
| G3 | Resources and network balancing | L | G2, K7 | todo |
| G4 | Conduits and ports | L | G3, W6 | todo |
| G5 | Signals and control | M | G4, A1 | todo |
| G6 | Recipes and work | L | G3, E8 | todo |
| G7 | Movers and item conduits | M | G6, G4 | todo |
| G8 | Construction and jobs | L | G6, W13 | todo |
| G9 | Assemblies and gadgets | M | G3, E3, E8 | todo |
| G10 | Suits and implants | M | G9, E5, V4 | todo |
| G11 | Storage and catch-up | L | G6, G4, W10 | todo |
| G12 | Benchmarks and budget gate | S | G3, then recurring | todo |
| G13 | Accounts and standing payments | M | G3, K7, S7 | todo |
| G14 | Trade and delivery | M | G13, G1, E8, L5, U9 | todo |
| G15 | Assemblies that stand alone | M | G10, E6, E7, I3 | todo |

## Order and parallelism

| Wave | Packages |
|------|----------|
| 1 | G1, G2 |
| 2 | G3 |
| 3 | G4, G6, G9 |
| 4 | G5, G7, G8, G10, G11 |
| 5 | G13, G15 |
| 6 | G14 |

G13 to G15 were added after the first review of what the design supports
(decisions D-198 to D-200). G14 needs the cold layer's shipments (ai
package L5).

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M4a Things you can use** | G1, G2 | Doors, lockers and switches offer interactions in a menu; a locked door refuses through a hook; a configurable device shows its settings on the generic screen. |
| **M4b Power** | G3–G5 | A generator burns fuel and lights a building. Demand beyond supply sheds the low-priority loads. Cutting a cable darkens half of it. The fuel runs out and everything stops in order. A lever opens a powered door; a terminal does the same from another room. |
| **M4c Workshop and base** | G6–G8, G11 | Craft at a bench from nearby shelves. A furnace fed by a loader runs unattended and slows on low power. Designate a room, deliver bricks, build it with a helper, stop halfway and resume. Leave for a day and return to the right number of bars. |
| **M4d Kit** | G9, G10 | A torch dies when its cell does and lives when the cell is swapped. A suit grants strength and flight while powered, keeps its wearer alive in vacuum, loses a module to a hit, and becomes dead weight when the core fails. |

| **M4e Means** | G13–G15 | Buy a rifle at a counter and be refused when the balance is short. Order a vehicle through a terminal and find it in the berth next day, or find the lorry robbed. Stop being able to pay the staff and watch them leave. Call a suit from its bay and have it walk to you, then step into it. |

## Work packages

### G1 — Interactions

**Spec** 16 §3 · **Size** M

Build
- The `interaction` type; offering by elements, devices, items, creatures
  and `any`; `interactions_for`.
- Reach kinds; the hooks `interaction_reach` and `interaction_starting`.
- The `interact` verb and the action it builds; defaults on bump.
- The `element` subject kind and its properties and verbs.

Done when
- Each reach kind is tested, including `network` through a terminal stub.
- A rule on `interaction_starting` cancels with a reason that reaches the
  player.
- A device's rule can `interact` with a neighbouring element.

### G2 — Device model

**Spec** 16 §4.1–4.3 · **Size** M

Build
- The full `Device` component and `device` definition: modes, `when`
  clauses, config, port and store state.
- Passive loads on elements; promotion to a device when one diverges.
- Data for the generic configuration screen.

Done when
- Four hundred passive lamps create no entities; switching one off creates
  exactly one.
- A config change through the `set_config` verb is saved and raises
  `device_configured`.

### G3 — Resources and network balancing

**Spec** 16 §5 · **Size** L

Build
- The `resource` type, including `discrete`.
- The network evaluator: offers, wants, stores, priorities, marginal
  fractions, `min_supply`; analytic store levels with scheduled crossings.
- `converts` with throttling; chains across networks; bounded iteration of
  loops.
- Capacity and `network_overloaded`; events.
- Network membership by assembly `bus` (used by G9 and by vehicles).

Done when
- Resources are conserved over random sequences of changes (invariant G1,
  property test).
- Store crossings fire at the analytic tick, including across a clock rate
  change (invariant G2).
- Scenario `brownout`: demand above supply sheds by priority and the
  marginal class runs at the right fraction.
- Scenario `fuel_runs_out`: generator, pump and lights stop in dependency
  order at the right ticks.
- An idle network schedules nothing (invariant G7).

### G4 — Conduits and ports

**Spec** 16 §6, §5.3, §4.4; 02 §5.1, §6.2 · **Size** L

Build
- The conduit layer in chunks, with `WorldEdit` operations, events and
  saving; the `conduit` type.
- Connected sets as networks; split and merge on change; severing with the
  host element; breaches.
- Port joining by `conduit`, `adjacent`, range and `link`.
- Tract services and service entries.

Done when
- Membership equals a from-scratch computation after random edits
  (invariant G4, property test).
- Scenario `cut_the_wire`: one severed cable splits a network and only the
  far side loses supply.
- Destroying a wall severs the conduits it hosted.
- Scenario `blackout`: switching a tract's service off unsupplies every
  network fed by its entries.
- Conduits in a subgrid form networks that move with it.

### G5 — Signals and control

**Spec** 16 §7 · **Size** M

Build
- Signal resources and combining; `outputs` as reactive expressions
  (package A1 of [abilities.md](abilities.md)); `inputs`; `signal_changed`;
  delays; wireless channels.
- The bundled basic set: switch, plate, gate, comparator, timer.
- Sensors as devices with senses.

Done when
- An output is re-evaluated exactly when something it reads changes.
- A delayed loop oscillates at its period; an undelayed loop is cut and
  reported, and the linter warns of it at load.
- Scenario `machine_room`: lever, generator, powered door and terminal
  give three ways through, none scripted.

### G6 — Recipes and work

**Spec** 16 §8 · **Size** L

Build
- The `recipe` type; work and rate; scheduled completion that follows the
  rate.
- Manual work through the `work` verb; machines with `auto`.
- Hold filters; `qualities`; supply from holds, station, nearby storage.
- `emits`; `recipe_completing`; place-conditioned recipes for growing.

Done when
- Completion time tracks every change of rate exactly (invariant G5).
- A machine on half supply takes twice as long.
- A bench recipe finds inputs on a shelf within range and not beyond it.
- A crop advances only while its condition holds.

### G7 — Movers and item conduits

**Spec** 16 §9 · **Size** M

Build
- Movers with filters and intervals; item conduits with transit queues,
  routing, blocking and severing; drains and intakes for fluids.

Done when
- Scenario `smelter_line`: ore in a chest reaches a furnace by loader and
  belt, bars reach another chest, and the line stalls and recovers
  correctly when the output fills and is emptied.
- Items are neither created nor lost by transport (property test).

### G8 — Construction and jobs

**Spec** 16 §10 · **Size** L

Build
- `builds`, `on`, `salvage`; derived dismantling and repair.
- `Job` storage, delivery, multi-worker progress, states and events; ghost
  data for presentation.
- `plan_from_map` against test map pieces until document 10.

Done when
- Scenario `build_a_room`: walls then a roof; a roof slab placed beyond its
  span from the walls falls by 02 §14.
- Scenario `interrupted_work`: progress survives interruption, saving and
  unloading (invariant G6).
- Two workers finish in half the time of one.
- Dismantling returns the salvage and restores what `on` required.

### G9 — Assemblies and gadgets

**Spec** 16 §11.1–11.3 · **Size** M

Build
- Slots, modules, the shared bus; `when_supplied`, `when_unsupplied`,
  `always`; granted actions with costs; plugging in; chargers.

Done when
- Scenario `dead_torch`: light while the cell holds charge, none after,
  light again with a fresh cell.
- An action with a cost is unavailable, with its reason, when the store is
  short; using it draws exactly the cost.
- Everything a module confers is gone when it is removed (invariant G10).

### G10 — Suits and implants

**Spec** 16 §11.4, §11.5 · **Size** M

Build
- Worn assemblies; modules struck through breached layers by location and
  exposure; the personal sealed volume and its exchange; pool-backed
  stores; a suit mind as driver.

Done when
- Scenario `the_suit`: supplied, the wearer has the conferred traits and
  actions; a hit destroys the arm module and its action goes; the core
  fails and `when_unsupplied` applies.
- Scenario `suit_in_vacuum`: sealed and intact, the wearer breathes; a
  breach leaks air at the specified rate.
- An implant draws from a pool and stops when the pool is empty.

### G11 — Storage and catch-up

**Spec** 16 §13 · **Size** L

Build
- Saving of devices, networks, jobs and transit queues; frozen summaries
  for unloaded parts of networks.
- Private-timeline catch-up with batching of repeated cycles.
- `hold_ticket`.

Done when
- Scenario `away_from_base`: stores and outputs after a day away equal
  those of the same day spent loaded (invariant G9).
- Catching up a hundred machines over a day meets the budget in 16 §16.
- A ticket-holding device keeps its tracts loaded within the budget rules
  of 02 §10.3.

### G12 — Benchmarks and budget gate

**Spec** 16 §16 · **Size** S, recurring

### G13 — Accounts and standing payments

**Spec** 16 §5.6; 15 §9.2 · **Size** M

Build
- `account` resources and balances for characters, groups and sites, a
  site's being its stock; saving; balances kept with agents in every form.
- `pay`, `start_payment`, `stop_payment`; balances as straight lines; the
  wake-up at the floor; `account_exhausted` and `payment_stopped`.
- `worth` on items; `deposit` and `withdraw`.
- Account amounts as recipe inputs; `payer` on jobs.
- Reading a balance in a mind context: one's own, or a group's through a
  role that spends.

Done when
- A payment between two holders conserves the total, and a balance equals
  its last value plus net rate times elapsed time (invariant G11).
- Test script `payday`: a holder with three standing payments runs out at
  the scheduled moment, all three stop, and the events are raised once.
- A payment that would go below the floor is refused with a reason.
- A recipe with an account input takes it at the start and returns it if
  cancelled before any work.
- A rule in a mind context reading someone else's balance fails to load.
- Ten thousand standing payments with nothing changing schedule nothing.

### G14 — Trade and delivery

**Spec** 16 §3.6; 09 §13.2, §13.4; 12 §10.7 · **Size** M

Build
- `trade` with item, account, payment and site entries; checking before
  moving; `trade_agreeing`; `then`.
- Parties: characters, groups through a spending member, sites, devices
  acting for one.
- `trade_offers`; the bundled rules offering a site's stock at its price;
  the trade screen in the `ui` mod.
- `deliver_to`: goods sent as a shipment and put down on arrival.
- `set_site` with a holder; `site_changed_hands`.

Done when
- A trade with one entry that cannot be met moves nothing and names the
  entry (invariant G12).
- Test script `the_till`: buying from a site's counter moves items out of
  its stock and the price into its balance; a seller's rule refuses a
  persona it will not deal with.
- Test script `mail_order`: something bought through a terminal arrives at
  the named berth when its shipment does; with the shipment intercepted, it
  does not, and the payment is not undone.
- A site bought in a trade has a new holder, and a standing payment from it
  now pays that holder.

### G15 — Assemblies that stand alone

**Spec** 16 §11.6 · **Size** M

Build
- `stand_alone`; `stand_up` and `stand_down` on one entity; the body's
  parts taken from the assembly's layers; the assembly as its own bearer;
  standing down when `while` fails.
- Putting on a standing suit as a joint action; stepping out and leaving
  it standing.

Done when
- Damage taken standing is on the layer when worn, and the reverse; stores
  and modules are unchanged by standing up or down (invariant G13).
- Test script `come_here`: a suit in its bay receives an order by a signal
  its sense module can hear, walks to its owner and is put on; a suit with
  no such module does not come.
- A standing suit whose core is destroyed stands down where it is.
- Three standing suits in a collective hold a formation and fight.

## Scenario catalogue

Under `scenarios/devices/`.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `brownout` | G3 | S24 | Priority shedding, marginal fraction |
| `fuel_runs_out` | G3 | S24 | Chains across networks |
| `cut_the_wire` | G4 | S18 | Split by severing |
| `blackout` | G4 | S7 | Tract services |
| `machine_room` | G5 | S18 | Power, signal and remote operation together |
| `smelter_line` | G7 | S24 | Unattended production, stall and recovery |
| `build_a_room` | G8 | S24 | Construction obeys structural support |
| `interrupted_work` | G8 | — | Job progress persists |
| `dead_torch` | G9 | S25 | Supply-conditional behaviour, cell swap |
| `the_suit` | G10 | S25 | Conferral, module damage, power loss |
| `suit_in_vacuum` | G10 | S25 | Personal sealed volume |
| `away_from_base` | G11 | S24 | Catch-up equals staying |
| `payday` | G13 | S26 | Standing payments and running out |
| `the_till` | G14 | S26 | Trade against a site's stock; refusal |
| `mail_order` | G14 | S26 | Delivery by shipment; interception |
| `come_here` | G15 | S26 | A suit that acts unworn |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Junctions between capacities | G4 | 16 §6.2, §5.5 | Test script `thin_wire`: a heavy load behind a thin conduit is starved to the thin conduit's capacity while the thick network's other loads are fully supplied. A rule on `network_overloaded` can burn the wire instead. |
| Loss on conduits | G4 | 16 §6.1 | A network's loss equals the sum of `loss` times length over its conduits, is drawn ahead of every consumer while the network is active, and is zero while it is idle. |
| Logic tables | G5 | 16 §7 | A controller's table edited in play drives its outputs as its rows say, re-evaluates when an input changes, and survives save and load. A row cannot name anything the device's definition does not expose. |
| Moving floors | G7 | 16 §9.4 | Test script `walkway`: a creature and a crate on a supplied moving floor are carried along at its speed; unsupplied, they are not; something anchored against a push stays put. |
| Fitting screen | G9 | 12 §10.7 | One screen shows the slots, contents, candidates and resource budget of a torch, a suit and a vehicle, with no screen written for any of them. Built when first-playable package U9 exists. |
