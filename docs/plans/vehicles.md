# Plan — Vehicles, the Sea and Travel

Implements [11-vehicles-and-travel.md](../11-vehicles-and-travel.md) and the
additions it makes to the world model (02 §18.5, media, sealed volumes).

Packages prefixed W are in [world-model.md](world-model.md); K and T in
[sim-kernel.md](sim-kernel.md); E and S in
[first-playable.md](first-playable.md); G in [devices.md](devices.md). The rules for taking a package are
those of [world-model.md](world-model.md) and [CLAUDE.md](../../CLAUDE.md).

Packages V1, V3 and V4 are world physics and belong to `clash-world`. They
need nothing from the entity model and can be built as soon as the world
packages they depend on are done.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| V1 | Media, pressure, buoyancy | M | W14, W16 | todo |
| V2 | Subgrid storage and hull entities | M | W20b, W10, E1 | todo |
| V3 | Sealed volumes | L | W20a, V1 | todo |
| V4 | Exchange: flooding, venting, crush | L | V3 | todo |
| V5 | Vehicles, parts, performance | M | V2, G3 | todo |
| V6a | Motion: process, ground, float, submerge, collisions | L | V5, V1 | todo |
| V6b | Motion: air, space, rail, stride, burrow, swing | L | V6a | todo |
| V7 | Stations, seats, boarding | M | V5, T6, E8 | todo |
| V8 | Mounts | M | E10, T6, S7 | todo |
| V9 | Docking, carrying, couplings | M | V6a | todo |
| V10 | Definitions and construction | M | V5, V3 | todo |
| V11 | Routes and journeys | M | W21, V6a | todo |
| V12 | Chart lanes | S | V11 | todo |
| V13 | Benchmarks and budget gate | S | V1, then recurring | todo |
| V14 | Bodies made of cells | L | V6b, V3, E5, C7 | todo |
| V15 | Driven bodies | M | V8, E7, C4, G10 | todo |

## Order and parallelism

| Wave | Packages |
|------|----------|
| 1 | V1, V2 |
| 2 | V3, V5, V8 |
| 3 | V4, V6a, V7, V10 |
| 4 | V6b, V9, V11 |
| 5 | V12, V14, V15 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M6a Water** | V1, V3, V4 | From the console: drop things in the sea and watch them float or sink. Hole a sealed box underwater and watch it flood to an air pocket. Open a second hole higher up and watch it fill. Take a hull below its limit and watch it fail. |
| **M6b Machines that move** | V2, V5–V10 | Ride a horse. Drive a car into a wall. Sail a ship with a car on its deck. Dive a submarine, flood a compartment, seal the door, blow ballast. Bolt thrusters to a torn-off building and fly it. |
| **M6c Journeys** | V11, V12 | Plot a course across a sea; stay aboard or skip; be intercepted. Leave a world along a chart lane and arrive at another. |

## Work packages

### V1 — Media, pressure, buoyancy

**Spec** 11 §4 · **Size** M

Build
- `Cosmos::medium`; the `fluid` block on materials and atmospheres.
- The liquid surface cache beside the heightmap; hydrostatic pressure.
- `sea_level` on realms; a `test_sea` generator with a sloping seabed.
- `volume` on bodies; buoyancy and medium drag in the body process.
- Light attenuation by clarity; medium sound speed for delayed signals.

Done when
- Pressure at depth matches `surface + ρ·g·depth` in still water and in a
  woken field column.
- Bodies lighter than the liquid rise to the surface and stay; denser ones
  sink at the analytic terminal speed within tolerance.
- The liquid surface cache equals recomputation after edits.
- An ocean tract of uniform water meets the uniform-chunk memory budget.

### V2 — Subgrid storage and hull entities

**Spec** 02 §18.5; 11 §3 · **Size** M

Build
- Anchor tracts, overlap references, the footprint ticket, loading by any
  overlapped tract, storing occupants with the subgrid.
- The binding between a subgrid and its hull entity.

Done when
- Loading any tract a stored subgrid overlaps loads all of it.
- A subgrid and its occupants round-trip through save and load with an
  identical hash.
- A subgrid is never observable half-loaded.

### V3 — Sealed volumes

**Spec** 11 §10.1 · **Size** L

Build
- Envelope computation; shadowing of the parent by envelope cells.
- Compartments, recomputed on structural change and door state; openings.
- Compartment gas and the medium inside; sealed areas in base grids.

Done when
- Every envelope cell is in exactly one compartment (invariant H4), after
  random edits (property test).
- Closing a door splits a compartment; opening it merges them and combines
  their gas.
- A sealed subgrid placed in water has no water inside it.

### V4 — Exchange: flooding, venting, crush

**Spec** 11 §10.2–10.5 · **Size** L

Build
- The exchange process; inflow and outflow at openings; compression of
  trapped gas; escape of gas through higher openings.
- Liquid aboard as mass; ballast compartments; pumps and vents as verbs.
- Pressure loading of hull elements and bulkheads with the `crush` type.
- Decompression impulses; sleeping; resolve mode; events.

Done when
- Gas and liquid are conserved across every step (invariant H3).
- Scenario `air_pocket`: a single low hole floods to the level where gas
  pressure equals outside pressure, and stops. Adding a high hole fills the
  compartment.
- Scenario `blown_out`: a compartment opened to vacuum loses its gas at the
  expected rate; a closed door preserves its neighbour.
- A hull taken past its weakest element's threshold fails there first.
- A volume with no openings and no excess pressure runs no process
  (invariant H6).

### V5 — Vehicles, parts, performance

**Spec** 11 §3, §6 · **Size** M

Build
- The `Vehicle` component; the `part` block on devices.
- The vehicle as an assembly on the network evaluator of package G3 in
  [devices.md](devices.md).
- Derived performance, recomputed on change.

Done when
- Halving available power halves thrust; destroying the only engine stops
  everything it fed.
- Performance equals recomputation after random part damage (invariant H1).

### V6a — Motion: process, ground, float, submerge, collisions

**Spec** 11 §5.1–5.4 · **Size** L

Build
- Modes; the motion process; the `ground`, `float` and `submerge` models.
- Collisions for craft and vessels; leaving the model's surface.
- Sinking when mass exceeds displacement.

Done when
- Top speed matches thrust equals drag within tolerance in each model.
- A craft driven into a wall damages wall, craft and occupants by the
  impact rule, scaled by seat exposure.
- A boat reaching a beach stops; a car leaving a roof falls as a body.
- Scenario `the_deep` (with V4, V7): dive, breach, flood, seal, blow
  ballast, surface. A variant without compressed air does not surface.

### V6b — Motion: air, space, rail, stride, burrow, swing

**Spec** 11 §5.2, §5.5, §11 · **Size** L

Build
- The remaining models; inertial and damped thrust; ceilings from strata.
- Quarter-turn swings with swept contact.

Done when
- A `wing` vehicle below stall speed loses height; above it, holds.
- A `hover` vehicle's ceiling is where air density no longer gives lift.
- An undamped `thrust` vehicle keeps its velocity with engines off.
- A swing into an obstruction resolves as contact.

### V7 — Stations, seats, boarding

**Spec** 11 §7, §9 · **Size** M

Build
- Station and seat parts; station actions as joint actions; the control
  verbs; sense parts reporting to stations; `board`, `alight`, `unseat`.

Done when
- Two entities cannot hold one station; leaving it ends the action and the
  vehicle carries on as it was.
- An occupant standing in a moving vessel is never updated (invariant H5).

### V8 — Mounts

**Spec** 11 §8 · **Size** M

Build
- Riding as a joint action with a minded mount; obedience through
  `order_received`; unseating.

Done when
- A trained mount obeys; an untrained one refuses by rule; a riderless one
  acts for itself.
- Scenario `the_chase` (with V6a): a rider and a driver each attack on a
  free channel while their rides move.

### V9 — Docking, carrying, couplings

**Spec** 11 §9 · **Size** M

Done when
- Scenario `ferry_crossing`: a car boards a ferry, is carried in the
  ferry's grid, and drives off the other side.
- Two docked vessels move as one and separate cleanly.
- A coupled train follows its track as one body.

### V10 — Definitions and construction

**Spec** 11 §12 · **Size** M

Build
- The `vehicle` type for craft and for vessels; `make_vehicle`, `reseal`,
  `install`, `uninstall`.

Until document 10's map format exists, vessel maps in tests are built by
console script.

Done when
- A detached building fitted with lift and thrust parts becomes a vehicle
  and flies (scene S11's sequel).
- Resealing after a repair restores a compartment's integrity.

### V11 — Routes and journeys

**Spec** 11 §13.1–13.3 · **Size** M

Build
- `travel_band`; routing over tract records; journeys with stock
  consumption and `travel_step`; aboard and skipped; encounters.

Done when
- A journey realises no tract (invariant H7).
- Running out of fuel stops the journey at the right place.
- Scenario `voyage`: the same journey taken aboard and skipped arrives at
  the same time with the same stocks.
- An interception places both parties at cell scale.

### V12 — Chart lanes

**Spec** 11 §13.4 · **Size** S

Done when
- A lane's condition blocks a vehicle without the required part, and its
  duration and effects apply to one with it.
- Scenario `orbit_to_luna`: top edge of one realm, lane, arrival in another
  with the interior played throughout.

### V14 — Bodies made of cells

**Spec** 06 §6.6 · **Size** L

Build
- `cells` on body plans; parts owning cells; condition from the cells that
  remain; functions, slots, leaks and signals driven by damage to cells.
- The creature as a hull entity moved by a motion model and steered by its
  own mind.
- Strikes and releases originating at a part's cells.
- Clinging: a held grip that keeps a climber in the body's grid beside a
  solid cell, and falling out of the grid on release.
- Severing by `detach`; what a stopped body becomes.

Done when
- Destroying half the cells of a limb halves its condition, and destroying
  enough disables it and its channel slots.
- A character standing on its back is carried without being updated, and
  one clinging to its side is carried while the grip holds and falls when
  it ends.
- A severed limb is a subgrid of its own and the body's functions follow.
- Scenario `the_colossus`: climb it, cut into it, be thrown off, and bring
  it down by destroying what its vital function depends on.

### V15 — Driven bodies

**Spec** 11 §8.1; 06 §8.2 · **Size** M · Added after the first review of
what the design supports (decision D-201)

Build
- Seats that `drive`: the joint action, the occupant as driver, the
  seat's `uses` held on the driver's own body.
- Commands and AI options routed to the driven body; `self.driver` and
  `driving`; senses relayed to the driver.
- Hits through the part holding the seat reaching the occupant by
  exposure; the body going still when the driver stops or is thrown out.

Done when
- Test script `in_the_cockpit`: a pilot in a metal body with no mind
  strikes, grips and throws with its limbs by document 07; loses an arm of
  the machine and the actions that needed it; is reached by a hit that
  breaches the torso; is knocked out, and the machine stops where it
  stands.
- A seat that uses no channels leaves the driver able to act with their
  own body at the same time; one that uses hands does not.
- With its power store empty the body is inert by `when_unsupplied`.
- An AI driver chooses among the body's attacks, not its own.

### V13 — Benchmarks and budget gate

**Spec** 11 §17 · **Size** S, recurring

## Scenario catalogue

Under `scenarios/vehicles/`.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `air_pocket` | V4 | S22 | Inflow, gas compression, equilibrium |
| `blown_out` | V4 | S9 | Venting to vacuum, bulkheads |
| `the_deep` | V6a | S22 | Depth, crush, flooding, ballast |
| `ferry_crossing` | V9 | — | Nested grids, boarding a vessel by driving |
| `the_chase` | V8 | S23 | Mount and craft, free channels |
| `flying_building` | V10 | S11 | Vehicle from a detached subgrid |
| `voyage` | V11 | S9 | Aboard against skipped |
| `orbit_to_luna` | V12 | S9 | Edge exit, lane, arrival |
| `the_colossus` | V14 | S11 | A creature that is a place |
| `in_the_cockpit` | V15 | S25 | A machine with limbs and a pilot |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Setting a liquid level | V1 | 11 §4.3; 02 §17.3 | `set_liquid_level` fills or drains loaded tracts through `WorldEdit` within the edit budget and gives unloaded ones a pending edit; `liquid_level_changed` is raised once. |
| List and capsizing | V4 | 11 §10.6 | Test script `she_rolls`: flooding one side raises `list` toward that side; counter-flooding lowers it; past `list_shift` loose things slide; past `capsize` the vessel capsizes, floods and puts those on deck in the water. The grid never turns. |
| Refused swings | V6b | 11 §5.5 | A long vessel in a channel narrower than its swing is refused the swing with a reason and keeps its quarter-turn. Forced, it swings and the contact is resolved as a collision. |
| Fleets | V7 | 11 §7; 09 §8 | Test script `convoy`: three vehicles in a collective keep formation on their leader through a turn, and an order to the collective reaches each commander. |
| Temperature | V1 | 11 §4.6; 02 §4.1 | `here.temperature` is the region's adjusted by stratum in the open, moves toward the liquid's deep value with depth, is the stratum's in vacuum, and adds what warming and cooling fields contribute. Nothing is scheduled to maintain it. |
| Drift | V1 | 11 §4.7 | Test script `downstream`: a swimmer and a floating crate in a river are carried along its line at its current; an occupant in a storm-force wind is staggered and one in a breeze is not; under a roof neither is touched. |
| Held temperature | V3 | 11 §4.6 | A closed sealed area with a supplied `climate` device reaches its set point in the time its volume and rate give, by one scheduled wake-up. Opened, it takes the outside temperature. Unpowered, it moves toward the outside at its `leak`. |
| Lifts | V6b | 11 §5.2 | Test script `going_up`: a platform on vertical track answers a call signal, stops level with the storey, and carries those standing on it. With its hoist destroyed it falls and lands as an impact. |
