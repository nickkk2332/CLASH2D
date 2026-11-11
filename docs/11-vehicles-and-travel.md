# 11 — Vehicles, Mounts, the Sea, Space and Travel

Status: design complete, ready for implementation.
Implementation plan: [plans/vehicles.md](plans/vehicles.md).
Serves scenes S9, S11, S22 and S23 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies everything that carries: mounts, small craft and
walkable vessels on land, on and under water, in the air and in space; the
physics they need from the world (liquids, pressure, buoyancy, sealed hulls
that flood and decompress); how they are crewed, built and broken; and
long-distance travel across realms and charts.

Part of what is specified here is implemented in `clash-world`: media
(section 4) and sealed volumes (section 10) are physics of cells, and extend
[02-world-model.md](02-world-model.md) without changing it.

It relies on subgrids (02 §18), travelers and charts (02 §20), containment
(01 §6), joint actions (03 §8), and devices, resources and assemblies (16).

Vehicle names and figures in examples are illustrative content.

## 2. The picture

Anything that carries someone is one of three things. A **mount** is a
creature with a rider on it. A **craft** is a machine you sit on or in but
cannot walk around inside: a motorbike, a car, a rowing boat, a fighter. A
**vessel** is a piece of world that moves, with decks and rooms: a bus, a
ship, a submarine, an airship, a starship. Vessels are subgrids, so
everything the world can do happens inside them: fights, fires, breaches.

What they share is a **motion model** per mode of travel, **parts** that
supply thrust, power and control, and **stations** from which someone
operates them. How fast a vehicle goes follows from its thrust, its mass and
the medium it is moving through. Damage a part or take on water and the
numbers change by themselves.

The world gains **media**: a cell is in gas, liquid or vacuum, with a
density and a pressure. Things float or sink by their density. Pressure
rises with depth.

A hull encloses **compartments**, each holding gas at some pressure. Where
a compartment is open to the outside, the outside comes in or the inside
goes out, at a rate set by the size of the hole and the pressure
difference. A holed ship floods compartment by compartment and sinks when
it weighs more than the water it displaces. A holed starship vents. A
submarine that goes too deep is crushed.

Journeys too long to play cell by cell happen on coarse maps, with the
vessel's interior still playable if the player wants to stay aboard.

## 3. Three forms

| Form | Is | Occupants are | Examples |
|------|----|---------------|----------|
| **Mount** | A creature | Inside it, in a `Seat` hold | Horse, wyvern, giant beetle, a tame demon |
| **Craft** | A device entity with a footprint of a few cells | Inside it, in seats | Bicycle, motorbike, car, cart, canoe, jet ski, glider, fighter, mini-sub, escape pod |
| **Vessel** | A subgrid with a hull entity | In the world, standing in its cells | Bus, lorry, train, ship, submarine, airship, starship, walking fortress, flying castle |

Every vehicle is an entity with a `Vehicle` component. A vessel's entity is
bound to its subgrid.

```rust
pub struct Vehicle {
    pub def: Option<VehicleDefId>,
    pub form: Form,                  // Craft | Vessel(SubgridId)
    pub modes: Vec<ModeId>,
    pub mode: ModeId,
    pub motion: Motion,              // section 5
    pub parts: Vec<EntityId>,        // installed devices
    pub bus: ResourceBus,            // section 6.2
    pub perf: Performance,           // section 6.3, cached
}
```

A mount has no `Vehicle` component. It is a creature that moves itself;
sections 7 and 8 cover what riding adds.

## 4. Media

Implemented in `clash-world`.

### 4.1 The medium of a cell

```rust
pub struct Medium {
    pub phase: Phase,                // Gas | Liquid | Vacuum
    pub density: Real,               // kg/m³
    pub pressure: Real,              // Pa
    pub sound_speed: Real,           // m/s
    pub clarity: Real,               // fraction of light passing per metre
    pub breathable: Option<AtmosphereId>,
}

impl Cosmos { pub fn medium(&self, loc: Location) -> Medium; }
```

| The cell | Its medium |
|----------|-----------|
| Is inside a sealed compartment (section 10) | The compartment's gas; liquid where the compartment's water level reaches |
| Holds a block flagged `liquid` (02 §5.4) | That material as a liquid |
| Holds a liquid field above two thirds full | That liquid |
| Otherwise | The stratum's atmosphere, or vacuum (02 §4.1) |

Materials used as liquids carry a `fluid` block: sound speed, clarity,
drag factor. Atmospheres carry the same.

### 4.2 Pressure

In gas: the atmosphere's pressure. In liquid: the pressure at the liquid's
surface plus `ρ · g · depth`. The surface height of each liquid column is
cached alongside the heightmap (02 §6.3). Ten metres of water adds about one
atmosphere, so a kilometre down is a hundred.

### 4.3 The sea

A realm definition may give a `sea_level`. Generators fill ocean tracts
with still liquid blocks from the seabed to that level. Large uniform
volumes of water cost what uniform rock costs: nearly nothing (02 §6.1).

The sea is still. When an edit opens space beside still liquid, the liquid
there wakes into a field and flows (02 §16). Waves are not simulated. A
region's **sea state**, **current** and **wind** are values set by the cold
simulation's weather (09) that motion models read.

### 4.4 Buoyancy and drag

Bodies in flight (02 §15) gain a `volume`. In any medium:

```
buoyancy = −gravity × medium.density × volume / mass
drag     = −½ · medium.density · drag · area · |v| · v / mass
```

Wood floats, stone sinks, a corpse sinks slowly, and a person thrown into
the sea decelerates hard. In air both terms are small and the equations of
02 §15.1 are unchanged.

Creatures moving in liquid use a `liquid` movement mode (06 §11). What
depth, cold and lack of air do to them is content reading the medium.

### 4.5 Light and sound

Light falls with depth by the medium's clarity (15 §3.3), so deep water is
dark. Sound travels in liquid at the medium's speed and with its own
falloff (15 §3.1), which is why hearing reaches much further than sight
underwater and why sonar is a sense worth having.

## 5. Motion

### 5.1 Modes

A vehicle has one or more **modes**. Each names a motion model, what it
needs from its surroundings, and which parts drive it.

```json
"modes": [
  { "id": "surfaced",  "model": "float",    "drive": ["screw"], "steer": "rudder" },
  { "id": "submerged", "model": "submerge", "drive": ["screw"], "steer": "rudder", "trim": "planes",
    "noise": "0.2 + 0.8 * throttle" }
]
```

A car has one mode. An amphibious lorry has two. A transforming machine
switches between them with an action.

### 5.2 Models

Implemented natively and configured by data.

| Model | Needs | Behaviour |
|-------|-------|-----------|
| `ground` | Standable cells under the footprint | Thrust against rolling resistance and drag. Traction from the floor's material limits acceleration and turning; beyond it, a skid. Steps up no more than its `step_up`. |
| `rail` | Elements tagged as track | As `ground`, constrained to follow the track |
| `stride` | Standable cells | Legged machines: moves in steps of its stride length; can cross what a wheel cannot |
| `float` | A liquid surface | Rides at the surface with a draft. Drag in liquid. Pushed by current and wind. Sea state reduces speed and control. |
| `submerge` | Liquid | Free in three dimensions. Rises or sinks by net buoyancy; trims with planes while moving. |
| `hover` | Gas dense enough, or any if the lift needs none | Lift opposes weight; free in three dimensions within acceleration limits. Rotorcraft, antigravity, levitation. |
| `wing` | Gas, and speed above stall | Lift from speed. Cannot stop in the air. Turns by banking, with a radius that grows with speed. |
| `aerostat` | Gas | Buoyant in the atmosphere. Slow. Pushed by wind. |
| `thrust` | Anything | Force and inertia only. With `damped: false`, velocity persists when thrust stops. With `damped: true`, it decays: the convention of most space fiction. |
| `burrow` | Solid | Moves through blocks, removing them or not, at a rate set by their material |

### 5.3 The motion process

A moving vehicle has a process on its own clock.

```rust
pub struct Motion {
    pub vel: [Real; 3],           // m/s
    pub heading: Real,            // radians
    pub offset: [Real; 3],        // position within the current cell, metres
    pub throttle: Real,           // −1..1
    pub helm: Real,               // turn demand
    pub lift: Real,               // climb or dive demand
}
```

Each wake integrates forces to the next cell boundary, or for at most 50 ms
of local time:

```
along heading:   a = (thrust × throttle − drag − resistance) / mass
turning:         heading changes at up to (lateral limit / speed), by helm
vertical:        a = (lift or buoyancy − weight) / mass, by lift demand
```

When the vehicle would enter new cells, the model checks that they suit it.
If they do, it moves: a craft through relocation, a vessel as 02 §18.2.
If something is in the way, it is a collision.

A vehicle with nobody at the controls keeps doing what it was doing.

### 5.4 Collisions

A craft striking something is a body impact (02 §15.2) with the craft's
mass and speed. The obstacle takes the packet. So does the craft, as an
item (06 §9.1), and so do its occupants, each as a hit scaled by how
exposed their seat is.

A vessel striking something resolves contact pairs as 02 §18.2.

Leaving what the model needs is not a collision. A car driven off a roof
becomes a body and falls. A boat driven onto a beach stops and takes damage
from the ground.

### 5.5 Heading and orientation

Motion is continuous: a vehicle may travel in any direction.

A craft faces one of eight directions. A vessel's grid is always at a
quarter-turn (02 §18). When a vessel's heading crosses a diagonal, its grid
**swings** a quarter-turn about its pivot, taking a time set by its size.
The swept area is checked and anything in it is a contact. So a ship
heading north-east is drawn pointing north or east. This is the cost of
exact, shear-free interiors, and it settles open question 3 of 02 §26.

## 6. Systems

### 6.1 Parts

A vehicle's capabilities come from **parts**: devices installed in it. In a
craft they are contained entities. In a vessel they are devices standing in
its cells, where they can be reached, repaired, sabotaged and shot.

A vehicle is an assembly (16 §11) and its parts are its modules. What a part
does with resources is written as for any device (16 §5.2). What it adds to
a vehicle is written in its `part` block.

```json
[
  { "type": "device", "id": "diesel_engine", "name": "diesel engine",
    "material": "steel", "mass": "900 kg",
    "converts": { "from": { "diesel": "0.012 kg/s" }, "to": { "power": "220 kW" }, "throttles": true },
    "part": { "needs_medium": "breathable", "noise": 0.8 } },
  { "type": "device", "id": "screw", "name": "propeller",
    "consumes": { "power": { "rate": "200 kW", "when": "running" } },
    "part": { "thrust": "60 kN", "in": ["liquid"] } },
  { "type": "device", "id": "fuel_tank",
    "stores": { "diesel": { "capacity": "800 kg" } } },
  { "type": "device", "id": "ballast_tank", "part": { "ballast": 40.0 } },
  { "type": "device", "id": "helm", "part": { "station": "helm" } }
]
```

| `part` field | Supplies |
|--------------|----------|
| `thrust`, `in` | Force along the heading, in the listed media |
| `lift` | Force against gravity |
| `steer`, `trim` | Turning and pitch authority |
| `traction` | Grip, for wheels, tracks and feet |
| `wing` | Lifting area |
| `ballast` | Volume that can be flooded or blown |
| `station` | A place to operate from (section 7) |
| `seat` | A place to ride, with an `exposure` |
| `sense` | A sense (15 §4) whose percepts go to whoever is at its station |
| `drive` | Ability to travel a chart lane (section 13.4) |
| `needs_medium`, `noise` | Conditions on running, and the sound it makes |

A part works in proportion to its condition and to its supply fraction
(16 §5.4). A destroyed part supplies nothing. Nothing else is needed for
damage to degrade a vehicle.

### 6.2 Resources

Resources and the networks that balance them are specified in 16 §5.

By default, everything installed in one vehicle shares one **bus**: a
single network per resource for the whole vehicle. An engine starved of
fuel makes less power, and a propeller on half power gives half thrust.
Stores are straight lines between changes, so nothing runs per tick.

A vessel may instead be wired with conduits (16 §6), in which case a cut
cable or a flooded switchboard isolates part of it.

Fuel, electricity, compressed air, coolant, ammunition and mana are all
resources content defines.

### 6.3 Performance

Derived whenever parts, mass or medium change, and cached.

| Quantity | From |
|----------|------|
| Mass | Hull elements, parts, everything aboard, and any liquid inside |
| Thrust | Working thrust parts valid in the current medium, scaled by the resources they are getting |
| Drag | `½ · ρ · drag_area · v²`, plus rolling resistance on the ground |
| Top speed | Where thrust equals drag |
| Turn rate | Lateral limit over speed: traction on the ground, rudder authority afloat, wing loading in the air |
| Lift, stall speed | Lift parts; or `½ · ρ · v² · wing × lift_coefficient` against weight |
| Buoyancy | `ρ · g ×` the volume of the hull's envelope below the surface |
| Range | Stock over consumption at cruise |

Rules read all of these (`vehicle.top_speed`, `vehicle.mass`,
`vehicle.buoyancy`).

## 7. Stations and crew

A **station** is a part someone operates from: a helm, a gunner's seat, a
sonar console, a ballast panel, an engine room.

- Being at a station grants its actions. Driving, firing a mounted weapon
  and blowing ballast are ordinary actions defined by content.
- A station action is a joint action (03 §8) between the operator and the
  vehicle. It uses the operator's hands and the vehicle's channel for that
  station, so one helm cannot be worked by two people, and a pilot who lets
  go to fight is no longer piloting.
- The engine supplies the verbs those actions call: `set_throttle`,
  `set_helm`, `set_lift`, `set_mode`, `set_ballast`, `brake`, `dock`,
  `undock`, `plot_course`.
- A sense part reports to its station. The sonar operator hears what the
  hull's hydrophones receive.

Crews are groups with roles (15 §9.2). A captain's instructions are orders
(15 §10) and have to be heard. The AI layer (09) fills stations and works
them. An autopilot is a rule.

## 8. Mounts

- A creature can carry riders if its templates give it `Seat` holds
  (06 §9.2). A rider is inside the mount and moves with it.
- **Riding** is a joint action between rider and mount, like a station
  action, except that the mount has a mind of its own. The mount's role is
  not forced: whether it obeys is the hook `order_received` (15 §10).
  Training, fear and exhaustion are content.
- An unridden or untrained mount goes where it wants.
- A rider uses their own channels while the mount uses its own. Fighting
  from the saddle needs no special support (decision D-045).
- A rider is as exposed as the seat says. `unseat` throws a rider clear as
  a body with the mount's velocity.
- Flying and swimming mounts are creatures with those movement modes.

Scene S23 is two people, one in a joint action with a horse and one with a
motorbike, each also running an attack on a free channel.

## 9. Getting on, in and across

| Situation | Mechanism |
|-----------|-----------|
| Mounting, sitting in a craft | `board`: relocation into a `Seat` hold. `alight` reverses it. |
| Walking aboard a vessel | Nothing special. A cell of the vessel is a cell (02 §18.1). |
| Hatches, airlocks | Doors. An airlock is a small compartment with a door at each end (section 10). |
| One vehicle carried by another | A craft or vessel standing on a vessel's deck is in that vessel's grid (02 §18.1 nesting). Lash it down or it slides. |
| Docking | Two vessels brought together and joined: `dock` makes one a child of the other until `undock`, and their adjoining hatches open onto each other. |
| Boarding by force | Cutting through a hull is damage to elements. What follows is section 10. |
| Towing, trains | A **coupling** joins vehicles in a chain so that the motion process treats them as one. |

## 10. Hulls, flooding and decompression

Implemented in `clash-world`.

### 10.1 Sealed volumes

```rust
pub struct SealedVolume {
    pub grid: GridId,
    pub envelope: CellSet,               // every cell enclosed by the hull, and the hull
    pub compartments: Vec<Compartment>,
    pub rated: Real,                     // Pa: the weakest hull element's crush threshold
}

pub struct Compartment {
    pub cells: CellSet,
    pub volume: Real,                    // m³
    pub gas: Option<AtmosphereId>,
    pub gas_amount: Real,                // pressure × gas volume
    pub ballast: bool,
    pub openings: Vec<Opening>,          // to outside or to another compartment
}
```

- A vessel has one sealed volume. Its **envelope** is found when the vessel
  is built or resealed: every cell that cannot be reached from outside
  without passing a fully covering element or floor. Envelope cells shadow
  the parent grid whether or not anything is in them, so the sea outside a
  submarine is not inside it.
- **Compartments** are the connected regions of the envelope separated by
  fully covering elements and closed doors. They are recomputed when
  structure inside the envelope changes or a door opens or closes.
- An **opening** is a face of an envelope cell that leads outside, or into
  another compartment, without a fully covering element: an open hatch, a
  hole.
- Gas pressure in a compartment is `gas_amount / (volume − liquid volume)`.

An area whose kind is flagged `sealed` (02 §8.3) gets a sealed volume in
the same way. Undersea bases, space stations that are part of a realm, and
airtight bunkers use this.

### 10.2 Exchange

A sealed volume with any opening to the outside runs an exchange process,
stepping every 250 ms of local time. With no openings it sleeps.

For each opening, compare pressure just outside with pressure just inside:

| Outside | Result |
|---------|--------|
| Liquid at higher pressure | Liquid enters at `0.6 · area · sqrt(2 · Δp / ρ)` m³/s. It arrives as a liquid field at the opening and flows down through the vessel by the usual field rule. |
| Gas or vacuum at lower pressure | Gas leaves at a rate proportional to area and Δp. Occupants and loose items near the opening receive an impulse toward it. |
| Gas at higher pressure | Gas enters |
| Equal | Nothing |

Consequences that need no further rules:

- **Air pockets.** Water entering a closed compartment compresses the gas
  above it. When the gas pressure equals the pressure outside, inflow
  stops. Unless the gas has somewhere to go: an opening higher up lets it
  out and the compartment fills.
- **Progressive flooding.** An open door is an opening between
  compartments. Closing it stops the spread.
- **Sinking.** Water aboard is mass. When the vessel's mass exceeds what
  its envelope displaces, a floating vessel becomes a sinking one.
- **Decompression.** A hole into vacuum empties a compartment in seconds or
  minutes according to its size. A sealed door saves the rest.
- **Breathing.** The medium in a compartment (section 4.1) is what its
  occupants breathe, at the pressure it is at.

Pumps, vents and valves are parts that move liquid or gas between
compartments, stores and the outside, driven by resources.

### 10.3 Ballast

A compartment flagged `ballast` is flooded and emptied on purpose. Flooding
adds mass. Blowing it, with pumps or compressed gas, removes it. Net
buoyancy is the difference between displaced mass and total mass, and the
`submerge` model turns that into rising or sinking. A submarine out of
compressed air, deep, with its pumps unpowered, is in trouble by
arithmetic.

### 10.4 Pressure and crush depth

Each exchange step also compares pressure across the hull, for volumes
where the difference exceeds `rated`:

- The weakest hull elements of the affected compartment receive packets of
  a `crush` damage type with intensity equal to the pressure difference.
- Below an element's threshold nothing happens (02 §12.2). Above it, damage
  accumulates until the element fails and becomes an opening.
- Bulkheads between a flooded compartment and a dry one are loaded the same
  way.

A hull has no "depth rating" number of its own. It has materials and
thicknesses, and the rating follows. Scene S22 is this section.

### 10.5 Resolve mode

An exchange process that loses its ticket (02 §10.3) resolves at once: each
compartment goes to its equilibrium, and a vessel that would sink is placed
on the bottom.

## 11. Air and space

**Air.** `wing` and `hover` need gas of sufficient density, so aircraft
have ceilings that follow from the strata (02 §4.1). `aerostat` lift is
buoyancy in air: envelope volume times air density against mass.

**Space.** In vacuum strata and in encounter realms (02 §20.2):

- Only `thrust` works. Gravity is whatever the stratum says, usually none.
- Every crewed compartment is a sealed volume holding gas against vacuum.
  Section 10 applies with the signs reversed.
- Ships meet, dock, board and fight at cell scale, as vessels anywhere do.
- Leaving a realm's top edge puts the vessel on a chart (02 §19).

Whether space flight is inertial or damped is a field on the mode
(section 5.2).

## 12. Building and breaking

### 12.1 Definitions

```json
[
  { "type": "vehicle", "id": "motorbike", "form": "craft",
    "footprint": [2, 1], "material": "steel", "mass": "190 kg",
    "hull": { "drag_area": 0.6 },
    "parts": ["bike_engine", "wheel", "wheel", "small_tank",
              { "seat": { "id": "rider", "station": "helm", "exposure": 1.0 } },
              { "seat": { "id": "pillion", "exposure": 1.0 } }],
    "modes": [ { "id": "road", "model": "ground" } ] },

  { "type": "vehicle", "id": "patrol_sub", "form": "vessel",
    "map": "patrol_sub_decks",
    "hull": { "drag_area": 7.0, "swing": "8 s" },
    "modes": [ { "id": "surfaced",  "model": "float",    "drive": ["screw"] },
               { "id": "submerged", "model": "submerge", "drive": ["screw"] } ] }
]
```

A vessel's `map` is a map in the format of document 10: its decks, cell by
cell, with its parts placed as devices.

### 12.2 Construction

A vessel is cells and devices, so it is built and altered the way anything
is: by placing and removing elements and installing devices. Performance
follows the result.

Vehicles are not only made from definitions:

| Verb | Does |
|------|------|
| `make_vehicle { subgrid }` | Gives any subgrid a hull entity and a `Vehicle` component assembled from the devices found in it |
| `reseal { vehicle }` | Recomputes the envelope after construction or repair |
| `install`, `uninstall` | Adds or removes a part |

A building cut loose with `detach` (02 §18.3) and fitted with thrusters, or
held up by a levitation trait on its hull entity, is a vehicle. So is a raft
of lashed logs.

### 12.3 Damage

Hull elements, parts and occupants are damaged by the rules that already
exist for elements (02 §12), items (06 §9.1) and bodies (06 §7). What that
does to the vehicle follows from sections 6 and 10. Fuel that burns and
magazines that explode are rules on those parts.

## 13. Travel

### 13.1 Setting out

`plot_course`, from a navigation station or a mount's rider, turns a
vehicle and everyone aboard into a traveler (02 §20.1) when the destination
is beyond local range.

### 13.2 Routes and bands

```json
{ "type": "travel_band", "id": "sea",
  "passable": ["water"], "seen_from": ["sea", "air"], "intercepted_by": ["sea", "undersea", "air"] }
```

A **band** is a way of crossing the tract grid: road, overland, sea,
undersea, air, orbit, underground. Bands are data. A route is found over
tract records by the band's passability, surveying regions as needed, and
never realises a tract.

A vehicle offers the bands its modes allow.

### 13.3 The journey

- Speed is the vehicle's cruise speed. Stocks are consumed at cruise rates.
  Running out stops the journey where it is.
- At each coarse step a `travel_step` event is raised for rules: weather,
  breakdowns, mutiny, and the event director's encounters (13).
- **Aboard or skipped.** The player may stay aboard: the vessel moves into
  a carrier realm (02 §20.2) and play continues inside it while the journey
  runs. Or skip: time advances to arrival, stopping for anything that
  interrupts.
- **Encounters.** When another traveler intercepts, both are placed at cell
  scale: in the tracts beneath them if on a realm, or in an encounter realm
  (02 §20.2), at a range and bearing the encounter's rules choose.

### 13.4 Charts

Travel between realms follows the **lanes** of a chart (02 §20.3).

```json
"lanes": [
  { "from": "earth", "to": "luna",
    "requires": "vehicle.has_part_with('drive', 'orbital')",
    "time": "distance / vehicle.drive_speed('orbital')",
    "on_enter": [], "on_arrive": [] }
]
```

A lane has a condition, a duration and effects. A star drive, a key, a
ritual, a toll and a guardian are all conditions and effects on lanes.

**Dimensions** are reached the same two ways as anywhere else: through a
portal (02 §19), or along a lane on a chart whose nodes are realms in
different dimensions. What it takes to cross is content.

## 14. Loading, storing and detail

Subgrid storage is specified in 02 §18.5.

| State | Cost |
|-------|------|
| Parked, loaded | None. No processes run. |
| Moving, loaded | A motion process, and an exchange process if holed |
| Stored | With its anchor tract |
| Travelling | A traveler record; plus a carrier realm if someone is aboard |

Motion and exchange are world processes and run at full fidelity on every
loaded tract. Crews in warm tracts are stepped as 03 §10.

## 15. Modding surface

**Definition types**: `vehicle`, `travel_band`. The `part` block on `device`
(16 §4.2). Resources are defined in 16 §5. The `fluid` block on `material` and `atmosphere`. `sea_level`
on `realm`. `sealed` on `area_kind`. `lanes` on `chart`.

**Events**: `vehicle_moved`, `vehicle_collided`, `vehicle_mode_changed`,
`part_installed`, `part_removed`, `resource_exhausted`,
`resource_restored`, `station_taken`, `station_left`, `boarded`,
`alighted`, `unseated`, `docked`, `undocked`, `hull_breached`,
`compartment_flooding`, `compartment_flooded`, `decompression`,
`hull_crushing`, `vessel_sinking`, `vessel_settled`, `travel_started`,
`travel_step`, `travel_interrupted`, `travel_arrived`, `encounter_started`.

**Hooks**: `boarding`, `travel_starting`, `collision_incoming`.

**Effects**: the verbs of sections 7 and 12.2, and `board`, `alight`,
`unseat`, `couple`, `uncouple`, `pump`, `vent`, `set_compartment_gas`,
`open_valve`, `add_resource`, `drain_resource`, `end_travel`.

**Properties**: on vehicles, `mass`, `speed`, `heading`, `mode`,
`top_speed`, `thrust`, `buoyancy`, `depth`, `altitude`, `range`,
`resource(id)`, `crew`, `station(id).operator`; on any location,
`medium.phase`, `medium.pressure`, `medium.density`, `medium.breathable`,
`compartment`, `compartment.pressure`, `compartment.flooded`.

**Queries**: `parts`, `seats`, `stations`, `compartments`, `aboard`.

## 16. Invariants

| # | Invariant |
|---|-----------|
| H1 | Vehicle performance is a function of its parts, mass and medium. No performance number is stored that those do not determine. |
| H2 | A vehicle's stores obey invariant G2 of document 16: level equals last recorded level plus rate times elapsed local time. |
| H3 | Gas and liquid are conserved by exchange: what leaves one side arrives on the other. |
| H4 | Every envelope cell belongs to exactly one compartment. |
| H5 | An occupant standing in a vessel has a location in the vessel's grid and is carried without being updated. |
| H6 | Sealed volumes sleep when they have no opening and no pressure difference above their rating. |
| H7 | A journey realises no tract it passes over. |
| H8 | Everything here survives saving and loading mid-voyage, mid-flood and mid-collision with an identical state hash. |

## 17. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Medium at a cell | under 40 ns |
| Motion step, craft | under 2 µs |
| Motion step, vessel with 500 hull cells | under 30 µs |
| Bus re-evaluation, 20 parts | under 5 µs |
| Recompute compartments, 30,000-cell vessel | under 3 ms |
| Exchange step, per opening | under 1 µs |
| Route across 2,000 tracts | under 5 ms |

## 18. Open questions

1. **Listing and capsizing.** Vessels do not tilt. A ship floods and sinks
   upright. Trim could be shown and could affect handling without rotating
   the grid.
2. **Waves.** Sea state is a number. Whether surface craft should bob in
   the vertical is a question of presentation.
3. **Very long vessels.** A swing sweeps a large area. Whether huge ships
   should be barred from turning in confined water or turn by other means.
4. **Finer wiring.** Settled: one bus per vehicle by default, or conduits
   inside the vessel (16 §6).
5. **Formations and fleets.** Many vehicles acting together. (09)
6. **Draining and flooding terrain at scale.** Woken liquid is budgeted
   (02 §16). Emptying a sea is a coarse event, not a simulation.
