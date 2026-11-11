# 16 — Interaction, Devices, Machines and Building

Status: design complete, ready for implementation.
Implementation plan: [plans/devices.md](plans/devices.md).
Serves scenes S13, S18, S24, S25 and S26 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how characters use the world and how the world is
made to do work: interactions, devices, resources and the networks that
carry them, conduits, signals and control, recipes for crafting and
automatic processing, moving items about, construction and base building,
assemblies (gadgets, powered suits, implants and anything else built from
modules), and accounts and trade.

It is the engine side of survival, building and automation play in the
manner of CDDA bases, RimWorld colonies and the tech and magic mods of
Minecraft, and of the equipment a superpowered setting needs.

It extends the minimal device of plans/first-playable E12, and supplies the
resource bus that vehicles use (11 §6.2). It adds one sparse layer to cells
(02 §5.1).

Names of resources, machines and modules in examples are illustrative
content. The engine does not know what electricity is, or money
(01 R10).

## 2. The picture

A character acts on the world through **interactions**: open, pull, load,
hack, drink from. An interaction is a definition with a reach, a duration
and effects. Elements, items, devices and creatures offer them.

A **device** is anything with behaviour: a lamp, a generator, a furnace, a
door actuator, a pressure plate, a pocket torch, a suit of armour. Devices
have **ports**. Ports join **networks**.

A **resource** is anything that is produced, moved, stored and used up:
power, fuel, water, steam, mana, data, torque. Resources are data, so a mod
defines its own. A **network** is the set of devices sharing one resource:
because they are wired together with **conduits**, because they are parts
of one **assembly**, or because they are in range of each other. A network
balances what is offered against what is wanted, sheds load by priority
when there is not enough, and fills or drains its stores. A device that
needs forty kilowatts does not run without forty kilowatts.

**Signals** carry values instead of quantities: this plate is pressed, that
tank is full. Devices drive them and respond to them. That is all the logic
a trap, an alarm or an automated factory needs.

A **recipe** turns inputs into outputs over time. A character follows one
at a workbench; a machine follows one unattended; and when the output is a
wall where there was none, the recipe is **construction**. Unfinished work
is kept as a **job** that anyone may continue.

An **assembly** is a device with **slots** for modules that share its
resources. A torch with a battery is an assembly. So is a rifle with a
scope, a powered suit, an implant set, and a vehicle.

Nothing here runs every tick. Stores change at known rates, processes
finish at known times, and the timeline holds one wake-up for each.

## 3. Interactions

### 3.1 Definition

```json
{ "type": "interaction", "id": "open", "name": "Open",
  "reach": "adjacent",
  "uses": { "arm": 1 }, "time": "600 ms", "noise": 0.2,
  "if": "target.state == 'closed'",
  "do": [ { "set_state": { "of": "target", "state": "open" } } ],
  "default": true }
```

| Field | Meaning |
|-------|---------|
| `reach` | How close the user must be (section 3.3) |
| `uses`, `time`, `noise` | Channel slots (03 §6.2), duration and sound of doing it |
| `tools` | Tool qualities required (section 8.3) |
| `if` | Whether it can be done now. `shown_if` decides whether it is listed at all. |
| `do` | Effects, with `self` the user and `target` the thing |
| `default` | Used when the thing is bumped into or clicked without choosing |

The engine has no built-in "open" or "use". Every interaction is content.

### 3.2 Who offers what

| Source | How |
|--------|-----|
| Elements | `interactions` on floors, blocks and fixtures (02 §5.5) |
| Devices and items | `interactions` on their definitions |
| Creatures | `interactions` on templates: talk, search, ride, feed |
| Anything | An interaction with `"offered_by": "any"` and a condition: examine, smash, climb |

`interactions_for(who, target)` lists what applies. The context menu
(12 §9.4) shows it.

### 3.3 Reach

| Reach | Means |
|-------|-------|
| `held` | The target is in the user's hold |
| `same_cell`, `adjacent` | By position. Adjacent includes diagonals and the cells above and below where there is a connection. |
| `{ "range": 6 }` | Within a distance, with line of sight |
| `network` | Through a device the user is operating that shares a network with the target: a terminal, a remote |

The hook `interaction_reach` lets traits change reach: telekinesis, a long
tongue, a hacking implant.

### 3.4 Doing it

The `interact` verb starts an action (03 §6) built from the interaction:
its channels, a single phase of its duration that may be cancelled, then
its effects. The hook `interaction_starting` runs first and may cancel with
a reason. Locks, alarms, permissions and traps are rules on that hook.

A device can interact too. An actuator that opens a door, an arm that loads
a furnace and a turret that pulls a trigger are devices whose rules call
`interact`.

### 3.5 Elements as subjects

A floor, block or fixture is a subject of kind `element` (05 §5.1), reached
as `here.block` and so on. Its properties are its definition, material,
state, damage, tags, and its device if it has one. `set_state`,
`set_material` and `damage` act on it through `WorldEdit`.

### 3.6 Trade

`trade` exchanges things between two parties, all at once or not at all.

```json
{ "trade": { "between": ["self", "target"],
             "gives": [ { "account": "credits", "amount": 1200 } ],
             "takes": [ { "item": "$rifle" }, { "item": "rifle_round", "count": 40 } ] } }
```

| Entry | Moves |
|-------|-------|
| `{ "item", "count" }` | Items between the parties' holds, or into and out of a site's stock (09 §13.2) |
| `{ "account", "amount" }` | An amount between their balances (section 5.6) |
| `{ "payment", "rate", "name" }` | Starts a standing payment (section 5.6): a wage, a rent, a retainer |
| `{ "site" }` | Makes the receiver the holder of a site (09 §13.4) |

A party is a character, a group acting through a member who may spend for
it, a site, or a device set to act for one of those: a till, a vending
machine, a terminal.

Everything is checked before anything moves: the items are there and can be
received, the balances cover the amounts. If any part fails, nothing
happens and the refusal says which part (14 §5). The hook `trade_agreeing`
runs first, and rules there may refuse: this seller does not deal with that
face. A trade may carry `then`, effects run when it completes. Hiring
someone is a trade that gives a standing payment and then sets a relation.

**Offers.** The hook `trade_offers` collects what one party will trade
with another. Rules add offers, each a `gives` and `takes` pair, according
to who is asking and what each believes. The bundled rules offer a site's
stock at `price(site, what)` (09 §13.2) through anyone who works its
counter. The trade screen (12 §10.7) shows the offers, and each choice made
there is a `trade`. The engine does not know what a shop is.

**Delivery.** When what is taken is not where the taker is, because it was
bought through a terminal, from a site elsewhere, or is too large to carry,
the trade names `deliver_to`: a site, a berth or a mark. The goods are sent
as a shipment (09 §13.2). They arrive when it does, and it can be met on
the way.

Characters trade among themselves by the same verb. Away from the player,
trade between sites is movement of stock (09 §13.2).

## 4. Devices

### 4.1 What a device is

```rust
pub struct Device {
    def: DeviceDefId,
    bound: Option<ElementRef>,         // the element it gives behaviour to, if any (02 §5.5)
    mode: ModeIdx,                     // off, idle, running, broken: named by the definition
    ports: SmallVec<[PortState; 2]>,
    stores: SmallVec<[Store; 2]>,
    process: Option<Running>,          // section 8
    config: VarMap,                    // what its user has set
    slots: SmallVec<[SlotState; 2]>,   // section 11
}
```

A device is an entity (06 §3). It may be:

| Kind | Example | Where it is |
|------|---------|-------------|
| Bound to an element | Furnace, door, lamp, terminal | In that element's cell |
| An item | Torch, radio, grapple gun, suit | Held, worn, stowed or lying about |
| A module | Battery, scope, thruster, engine | Fixed in an assembly's slot |
| Free-standing | Drone, turret on a tripod | In the world like any entity |

A device may have senses (15 §4), channels and a mind (06 §8). A turret is
a device with a sense, a mounted weapon and a simple mind. A drone adds a
movement mode.

### 4.2 Definition

```json
{ "type": "device", "id": "arc_furnace", "name": "arc furnace",
  "material": "steel", "mass": "1200 kg",
  "tags": ["furnace"],
  "modes": ["off", "idle", "running"],
  "ports": [ { "id": "mains", "resource": "power", "connect": "conduit" } ],
  "consumes": { "power": { "rate": "40 kW", "when": "running", "priority": 2 } },
  "holds": { "in":  { "volume": 0.2, "accepts": ["ore", "flux"] },
             "out": { "volume": 0.2 } },
  "auto": true, "speed": 1.0,
  "config": [ { "id": "enabled", "kind": "flag", "default": true } ],
  "outputs": { "busy": "self.mode == 'running'" },
  "interactions": ["toggle", "load", "unload", "configure"],
  "light": null,
  "rules": [] }
```

| Field | Meaning |
|-------|---------|
| `modes` | Named operating states. Fields that say `when` apply in those modes. |
| `ports` | Section 5.3 |
| `produces`, `consumes`, `stores`, `converts` | Section 5 |
| `holds` | Item storage with filters (06 §9.2) |
| `auto`, `speed` | Whether it runs recipes by itself, and how fast (section 8) |
| `config` | Settings a user may change: flags, numbers, choices, filters, a linked target, a logic table (section 7). Read as `self.config.enabled`. |
| `inputs`, `outputs` | Signals (section 7) |
| `slots`, `bus`, `module` | Assemblies (section 11) |
| `part` | What it adds to a vehicle (11 §6.1) |
| `rules` | Attached rules (05 §9.3) |

A generic configuration screen (12) is built from `config`, so a new machine
needs no new screen.

### 4.3 Promotion and passive loads

An interactive element becomes a device entity only when it needs state of
its own (02 §5.5). To keep large places cheap, an element may also declare a
**passive load**:

```json
{ "type": "fixture", "id": "ceiling_lamp", "load": { "power": "60 W" },
  "when_supplied": { "light": 0.8 }, "glyph": { "ch": "°" } }
```

Passive loads on a network are counted, not instantiated. They are all
supplied or all not, with the network. A building with four hundred lamps
has one number, until someone smashes a lamp or switches one off, and that
one becomes a device.

### 4.4 Services

Whole-city grids are not wired cell by cell. A tract may carry **services**
in its record: mains power, water, data. A **service entry** is a device
that offers a service to its local network for as long as the tract has it.
The cold simulation (09) and rules turn services on and off. A blackout is
one value changing on some tracts.

## 5. Resources and networks

### 5.1 Resources

```json
[
  { "type": "resource", "id": "power",  "unit": "J",  "name": "Power" },
  { "type": "resource", "id": "diesel", "unit": "kg", "name": "Diesel",
    "on_breach": [ { "add_field": { "at": "here", "field": "fuel_spill", "amount": "event.rate * 40" } } ] },
  { "type": "resource", "id": "mana",   "unit": "thaum", "name": "Mana" },
  { "type": "resource", "id": "rounds", "unit": "count", "discrete": true }
]
```

A resource has a quantity, in its unit, and a rate, which is quantity per
second. Power is joules and watts. Fuel is kilograms and kilograms per
second. Whatever else a mod defines works the same way.

`discrete` resources come in whole units and move one at a time.

### 5.2 What devices do with them

| Field | Meaning |
|-------|---------|
| `produces` | Offers up to a rate, from nothing: a solar panel, a ley line tap, a service entry |
| `consumes` | Wants a rate, with a `priority` and the modes in which it applies |
| `stores` | Holds up to a capacity; charges and discharges up to given rates |
| `converts` | A continuous conversion: takes rates of some resources and gives rates of others, in proportion. A generator, a pump, a transformer, a mana altar. With `throttles`, it runs only as hard as demand requires. |

```json
"converts": { "from": { "diesel": "0.012 kg/s" }, "to": { "power": "220 kW" }, "throttles": true }
```

### 5.3 Ports

A port is where a device meets a network.

| `connect` | Joins |
|-----------|-------|
| `conduit` | The network of any conduit of that resource in the device's own cell |
| `adjacent` | As `conduit`, and directly to devices with matching ports in neighbouring cells |
| `{ "range": 8 }` | The nearest transmitter of that resource within range: wireless power, an aura |
| `link` | One other device, paired explicitly by an interaction or rule |
| `bus` | The assembly it is installed in (section 11) |

### 5.4 Balancing

Each network balances rates. At any moment:

1. Total what producers and converters offer and what consumers want.
2. If there is a surplus, it charges stores, up to their charge rates.
   Anything left is not produced: throttling converters ease off and the
   rest is wasted.
3. If there is a shortfall, stores discharge to cover it, up to their rates
   and while they hold anything.
4. If there is still a shortfall, consumers are cut in order of priority,
   lowest first. The class at the margin receives a fraction, and devices
   in it run at that fraction.

Then every store's level is a straight line until something changes. The
moment the first store empties or fills is scheduled as one wake-up, and
the network balances again then.

A network is also rebalanced when a member changes mode, is damaged, joins
or leaves, or when a converter's own supply changes. A converter is a
member of two networks, so shortage propagates: no fuel, no power, no
pump, no fuel. Chains are evaluated in dependency order; loops are iterated
a fixed number of times.

Each consumer knows its **supply fraction**. A machine at half supply works
at half speed. A device may state a `min_supply` below which it does not
run at all.

### 5.5 Limits

Stores and ports have maximum rates. A conduit has a `capacity`, and every
conduit in one network has the same (section 6.2). When what a network
carries exceeds it, the hook `network_overloaded` runs. Whether a breaker
trips, a fuse blows or a cable burns is a rule.

### 5.6 Accounts

A resource flagged `account` is not carried by conduits. It is held as
**balances**.

```json
{ "type": "resource", "id": "credits", "unit": "cr", "account": true, "floor": 0 }
```

| Holder | Its balance |
|--------|-------------|
| A character | Its own, kept with it in every form (06 §12) |
| A group | Held in common. Members whose role is flagged `spends` (15 §9.2) may pay from it. |
| A site | Its stock of that resource (09 §13.2). What a site earns and spends are that stock's rates. |

| Verb | Does |
|------|------|
| `pay { of, amount, from, to }` | Moves an amount from one balance to another. Refused with a reason if it would take the payer below the resource's `floor`. With no `from` the amount is created; with no `to` it is destroyed. |
| `start_payment { name, of, rate, from, to }` | A **standing payment**: an amount per second of universal time (01 §5.2), moved continuously |
| `stop_payment { name, from }` | Ends one |
| `deposit { who, items }`, `withdraw { who, of, amount }` | Turn tokens into balance and back |

Standing payments make a balance a straight line between changes, as a
store's level is. The moment a payer would reach its floor is one
scheduled wake-up: `account_exhausted` is raised, its standing payments
stop, and `payment_stopped` is raised for each. What the unpaid do about it
is for rules.

`x.balance('credits')` reads a balance. A mind may read its own, and a
group's if its role spends. Nobody knows what is in another's account
unless told.

**Tokens.** An item may state what it is worth:
`"worth": { "credits": 100 }`. Tokens are ordinary items. They can be
carried, dropped, hidden, stolen and looted. `deposit` and `withdraw` are
verbs that content puts on interactions: a bank counter, a cash machine. A
mod whose money is only ever coins needs no accounts at all: its prices are
item entries in trades.

**Work that costs.** A recipe input may be an amount from an account. It is
taken from the job's `payer`, or from the worker if the job names none,
when work starts, and returned if the job is cancelled before any work is
done.

The engine gives an account no meaning (01 R10). Money, a guild's favour,
ration points and a research budget are account resources with different
names.

## 6. Conduits

### 6.1 The layer

Cells gain a fourth, sparse layer: **conduits**. A cell may hold several.

```json
{ "type": "conduit", "id": "copper_cable", "name": "cable",
  "carries": ["power"], "capacity": "50 kW",
  "material": "copper", "host": "any",
  "glyph": { "ch": "·", "fg": "orange" } }
```

Conduits are cables, pipes, ducts, shafts, belts, ley lines and control
wire. They run through the cell without occupying its floor, block or
fixture, so a wire can pass under a floor or inside a wall.

- A conduit is hosted by a floor or a block. If its host is destroyed, it
  is severed. It can also be cut by an interaction.
- Conduits change only through `WorldEdit`, like every other layer, and
  emit `conduit_changed`.
- Subgrids have the layer too. A vessel may be wired instead of using one
  bus (11 §6.2).
- A conduit may state a `loss` per metre. A network's loss is the sum over
  its conduits, drawn as a standing load ahead of every consumer while the
  network is active. It does not depend on what flows where. It makes a
  long run cost something without tracking flow along it.

### 6.2 Networks from conduits

Conduits carrying the same resource at the same capacity in neighbouring
cells (the four sides, above and below) are connected. Each connected set
is one network. Devices join through their ports.

Where conduits of different capacity meet there is a **junction**. The two
sides are separate networks, and the junction passes the resource between
them at no more than the smaller capacity, as a converter of the resource
into itself would. A thin wire feeding a heavy load therefore starves the
load, and the thick network behind it is unaffected. When more is wanted
through a junction than it passes, `network_overloaded` runs for the
thinner side, and a rule may burn the wire instead of letting it limit.

When a conduit is added, removed or severed, the affected networks are
split or merged by searching outward from the change. Cutting one wire in a
wall can leave half a building dark.

### 6.3 Breaches

When a conduit carrying a resource with `on_breach` is severed while the
network is active, those effects run at the break: fuel spills, steam
escapes as a field, a live cable arcs. What escapes comes off the network's
stores.

## 7. Signals

A **signal** is a value on a network rather than a quantity. Signal
resources are declared with `"signal": true` and a rule for combining
several drivers (`max`, `sum`, `any`).

```json
{ "type": "resource", "id": "control", "signal": true, "combine": "max" }
```

- A device **drives** a signal with an `outputs` entry: a reactive
  expression (05 §7.5) over the device, re-evaluated when anything it reads
  changes. A pressure
  plate drives 1 when something stands on it. A tank gauge drives its fill
  fraction.
- A device **reads** a signal with an `inputs` entry, bound to a name its
  fields and rules can use: `"enable": "in"` makes a machine run only while
  the signal is non-zero. A change raises `signal_changed` on every reader.
- Signals travel in conduits that carry them, or wirelessly on a channel
  number set in `config`.
- A device may delay its output by a duration. With delay, loops make
  clocks and latches. A loop with no delay anywhere is cut by the cascade
  limit (03 §4.3) and the linter warns about it.

A `config` entry of kind `logic` is a table the user fills in during play:

```json
{ "id": "program", "kind": "logic", "rows": 8,
  "inputs": ["a", "b"], "outputs": ["x", "y"] }
```

Each row reads *input, comparison, value, output, value to drive*: "when
`a` is above 0.8, drive `x` to 1". Rows are compiled to reactive
expressions over the device's inputs, and the first row that holds for an
output drives it. The configuration screen shows the table. A player
programs a controller this way without writing rules, and cannot make it
do anything a device definition could not.

Sensors are devices with senses (15 §4) whose rules drive a signal on a
percept. Gates, comparators, timers and counters are small devices defined
in content with expressions. The bundled `core` mod supplies a basic set.

Scene S18 is three networks: power from a generator to a door actuator, a
signal from a lever to the same actuator, and a terminal whose `network`
reach lets its user operate both.

## 8. Recipes

### 8.1 Definition

```json
{ "type": "recipe", "id": "smelt_iron", "name": "Smelt iron",
  "at": ["furnace"],
  "inputs":  [ { "item": "iron_ore", "count": 2 }, { "resource": "flux", "amount": "0.1 kg" } ],
  "tools":   [ { "quality": "tongs", "level": 1 } ],
  "while":   { "power": "40 kW" },
  "work": "30 s",
  "outputs": [ { "item": "iron_bar", "count": 1 }, { "item": "slag", "count": 1, "chance": 0.5 } ],
  "emits":   { "field": { "heat": 20 }, "noise": 0.4 },
  "auto": true }
```

| Field | Meaning |
|-------|---------|
| `at` | Tags of the station it needs; `"hand"` for none |
| `inputs` | Items and resource amounts used up, and amounts paid from an account (section 5.6) |
| `tools` | Qualities that must be present and are not used up (section 8.3) |
| `needs` | A condition on the worker: knowledge, a track level, a trait |
| `while` | Resource rates required for as long as the work goes on |
| `work` | How much work it takes, as time at a work rate of 1 |
| `outputs` | Items and resource amounts produced, with optional chances |
| `result` | Effects run on completion: anything outputs cannot express |
| `emits` | Fields, light and noise while under way |
| `auto` | Whether a machine may run it unattended |
| `builds`, `on`, `salvage` | Construction (section 10) |

One definition type covers crafting by hand, processing by machine, cooking,
brewing, charging, growing, research and construction.

### 8.2 Doing the work

A recipe needs an amount of **work**. Work is done at a **rate**.

| Worker | Rate |
|--------|------|
| A character | An expression set by content (skill, tools, light, injuries) times the station's `speed`, for as long as their work action continues |
| A machine | Its `speed` times its supply fraction for the `while` resources |

Progress is `work done / work needed`. Completion is one scheduled wake-up,
moved whenever the rate changes. A furnace on half power takes twice as
long and nothing checks on it in between.

A machine with `auto` starts a recipe whenever one of its recipes has its
inputs in the machine's input holds, room for its outputs, and nothing
forbidding it (its `enable` input, its `config`). It stops when that ceases
to be true.

Interrupted work is not lost. It stays on the machine, or on the **job**
(section 10.2).

The hook `recipe_completing` lets rules alter or spoil the result.

### 8.3 Tools and supply

Items declare **qualities**: `"qualities": { "cutting": 2, "hammering": 1 }`.
A requirement names a quality and a level. Qualities are data.

Inputs and tools are looked for, in order: in the worker's holds; in the
station's holds; in containers and storage areas within `supply_range`
(default 6 cells) that the worker may use; and in any item network the
station is on (section 9). Crafting at a bench draws on the shelves around
it.

### 8.4 Growing things

A crop, a culture and a slow reaction are recipes whose `while` is a
condition on the place instead of a resource: enough light (15 §3.3), a
field of water, warmth. A planted fixture is a small device running such a
recipe through its growth states. Harvesting is an interaction.

## 9. Moving items

### 9.1 Movers

A **mover** is a device that transfers items between two holds next to it.

```json
{ "type": "device", "id": "loader_arm",
  "moves": { "from": "back", "to": "front", "every": "1 s", "accepts": "self.config.filter" },
  "consumes": { "power": { "rate": "200 W", "when": "running" } } }
```

It takes one matching item (or one stack unit) from the hold on one side and
puts it in the hold on the other, at its interval, while supplied and while
there is something to take and room to put it. Each transfer is one
wake-up. Hoppers, inserters, sorters and dispensers are movers.

### 9.2 Item conduits

Belts, tubes and chutes are conduits that carry items. An item put on an
item network at one device arrives at another after
`path length / speed`. Items in transit are held by the network as a queue
with arrival times. They are not moved cell by cell. Presentation draws
them where they would be.

Routing is by filter and priority at the receiving ports. A full
destination blocks, and the queue backs up.

Severing an item conduit drops what was in transit near the break.

### 9.3 Fluids and gases

Fluids and gases are ordinary resources in pipes and tanks. Released into
the world, deliberately or by a breach, they become fields (02 §16), and a
drain or intake takes a field back into a network.

### 9.4 Moving floors

A floor or fixture may state `conveys`: a speed. Its facing gives the
direction.

```json
{ "type": "floor", "id": "walkway", "conveys": "1.5 m/s",
  "load": { "power": "200 W" }, "when_supplied": "conveys" }
```

Each step, whatever stands or lies on it is pushed along (07 §7) by its
speed times the step: creatures, loose items, a crate, a body. It is a
push like any other, made through the relocation functions, and what can
resist a push resists it.

This is a walkway, a belt that can be ridden, a treadmill, an escalator on
a ramp. It is separate from item conduits (section 9.2), whose goods are a
queue and are not in the world while they travel. A line that must carry
both goods and people uses moving floors, with movers at its ends.

## 10. Construction

### 10.1 Building is a recipe

```json
{ "type": "recipe", "id": "build_brick_wall", "name": "Brick wall",
  "at": ["hand"],
  "inputs": [ { "item": "brick", "count": 40 }, { "item": "mortar", "count": 2 } ],
  "tools":  [ { "quality": "trowel", "level": 1 } ],
  "work": "20 min",
  "on":     { "block": "none" },
  "builds": { "block": "wall@brick" },
  "salvage": [ { "item": "brick", "count": 30 } ] }
```

`on` says what must be at the target. `builds` says what is there
afterwards:

| `builds` | Result |
|----------|--------|
| A floor, block or fixture state | The element is placed or replaced |
| A conduit | Laid in the cell |
| A device | Its element is placed and the device installed and connected |
| A module or part | Installed in an assembly or vehicle (11 §12.2) |
| `none` for a layer | The element is removed: digging, dismantling |

For every building recipe with `salvage`, a dismantling recipe exists
without being written. Repair restores an element's damage for a share of
its inputs in proportion.

What is built is real. A floor built with nothing under it falls
(02 §14). A wall of mud is as strong as mud.

### 10.2 Jobs

```rust
pub struct Job {
    pub id: JobId,
    pub recipe: RecipeId,
    pub target: Location,
    pub params: VarMap,
    pub supplied: Vec<EntityId>,       // inputs already delivered
    pub done: Real,                    // work done so far
    pub claims: Vec<EntityId>,         // who has taken it on (09 §7.1)
    pub state: JobState,               // Planned | Supplied | Under way | Blocked(reason)
    pub priority: u8,
    pub payer: Option<Holder>,         // whose account pays its costs (section 5.6)
    pub cause: Cause,
}
```

A **job** is a piece of work to be done somewhere. Designating a wall,
queueing a bill at a workbench and ordering a repair all create jobs.

- Jobs belong to a site or an area and are saved.
- Anyone who may can deliver inputs and do work. Several workers add their
  rates. Work done stays if they are interrupted.
- The player works a job with an action. Who else works which jobs is
  decided by the AI layer (09) from priorities and orders (15 §10). The job
  list is the interface between this document and that one.
- A planned job is drawn as a ghost of what it will build.

### 10.3 Designs

`plan_from_map` creates jobs for every cell in which a map piece
(document 10) differs from what is there. A design for a room, a workshop
or a whole base is a map piece, and a mod may ship them.

### 10.4 Places

Bases use what already exists:

| Need | Mechanism |
|------|-----------|
| Storage with rules about what goes where | Areas (02 §8.3) of a storage kind, with filters in their variables |
| Rooms that hold warmth or air | Sealed areas (11 §10) and fields |
| Ownership and who may use what | Sites, groups and relations (15 §9), checked by hooks |
| Keeping a base running while away | Section 13 |

## 11. Assemblies

### 11.1 Slots and modules

```json
[
  { "type": "device", "id": "hand_torch", "name": "torch",
    "item": { "mass": "0.2 kg", "grip": { "arm": 1 } },
    "slots": [ { "id": "cell", "accepts": ["small_cell"] } ],
    "bus": ["power"],
    "consumes": { "power": { "rate": "3 W", "when": "on" } },
    "modes": ["off", "on"],
    "when_supplied": { "light": 1.2 },
    "interactions": ["toggle", "swap_cell"] },

  { "type": "device", "id": "small_cell", "name": "power cell",
    "module": { "fits": ["small_cell"] },
    "stores": { "power": { "capacity": "40 kJ", "rate": "50 W" } } }
]
```

An **assembly** is a device with `slots`. A **module** is a device that
fits a slot. Everything in an assembly shares one network for each resource
in its `bus`.

An assembly may have a port to the outside. Plugged in, its bus joins that
network and its stores charge.

A vehicle is an assembly whose slots are wherever its parts are installed
(11 §6). The vehicle bus and the torch's are the same mechanism.

### 11.2 What modules give

A module, or the assembly itself, may confer things on its **bearer** (the
one holding, wearing or implanted with it), conditionally:

```json
{ "type": "device", "id": "repulsor", "name": "repulsor",
  "module": { "fits": ["arm_mount"], "exposure": 0.6 },
  "consumes": { "power": { "rate": "2 kW", "when": "idle" } },
  "when_supplied": {
    "grants": [ { "action": "repulsor_blast", "cost": { "power": "150 kJ" } } ],
    "traits": ["repulsor_ready"] } }
```

| Field | Gives the bearer |
|-------|------------------|
| `when_supplied` | While the module's supply fraction meets its `min_supply` |
| `when_unsupplied` | While it does not |
| `always` | While installed |

Each may list traits (06 §5), abilities (08), granted actions, senses
(15 §4), attribute modifiers and interface parts (12 §10.6).

A granted action may have a `cost`: an amount drawn from the assembly's
stores when the action starts. If it is not there, the action is
unavailable and says why. A gadget with three charges is a module with a
store of three and a cost of one.

### 11.3 Gadgets

A gadget is a small assembly that is an item: a torch, a radio, a grapple
gun, a stun baton, a jet pack, a wand with a crystal in it. Nothing further
is needed to define one. Cells are swapped by an interaction, recharged in a
charger (a device that moves a resource between stores), or refilled by
whatever the content says refills them.

### 11.4 Powered suits

A suit is an assembly that is worn.

```json
{ "type": "device", "id": "assault_frame", "name": "assault frame",
  "item": { "wear": { "parts": "all", "layer": 3, "material": "alloy", "thickness": "12 mm", "coverage": 0.95 } },
  "slots": [ { "id": "core",  "accepts": ["suit_core"],  "on_part": "torso" },
             { "id": "arm_l", "accepts": ["arm_mount"],  "on_part": "l_arm" },
             { "id": "back",  "accepts": ["back_mount"], "on_part": "torso" },
             { "id": "helm",  "accepts": ["helm_system"],"on_part": "head" } ],
  "bus": ["power", "coolant", "air"],
  "sealed": { "atmosphere": "air", "volume": 0.12 },
  "when_supplied":   { "traits": ["frame_assist"] },
  "when_unsupplied": { "traits": ["dead_weight"] } }
```

Everything a suit does is something already specified:

| The suit | Is |
|----------|----|
| Armour | Worn layers (06 §9.2), hit before the wearer's own |
| Strength, speed | Traits with attribute modifiers, conferred while supplied |
| Flight | A thruster module granting a movement mode (06 §11) |
| Weapons, tools | Modules granting actions with costs |
| Sensors, a display | Modules granting senses and interface parts |
| Life support | A `sealed` assembly is a one-compartment sealed volume around its wearer (11 §10). The wearer's medium is the suit's gas, not the sea's or the vacuum's, for as long as its `air` lasts and its layers hold. A breach is an opening and leaks at the rate 11 §10.2 gives. Outside pressure beyond what its layers can take crushes it by 11 §10.4. |
| Taking damage | A hit that breaches a worn layer may strike a module mounted `on_part` there, by its `exposure`. A damaged module works in proportion to its condition. A destroyed core is a suit with no power. |
| Running out | `when_unsupplied`: the wearer is inside a great weight of metal that does nothing |
| An onboard intelligence | A mind on the suit. As a driver (06 §8.2) it can walk an unconscious wearer home. |
| Putting it on | An action; or a joint action with the suit if it assembles itself |

A suit one wears augments one's own body and actions. A machine one sits
in and operates from a station is a craft or vessel (11 §3). A machine that
fights with limbs of its own, with someone inside deciding for it, is a
driven body (11 §8.1). A suit that acts with nobody in it is section 11.6.

### 11.5 Implants

An implant set is an assembly whose host is a body, with slots on parts and
modules held `Fixed`. A store may be backed by a pool (06 §4.2), so
implants can run on the body's own reserves.

Installing and removing are recipes whose `result` uses the body verbs of
06 §6.5.

### 11.6 Standing alone

An assembly that is an item may be able to act with nobody wearing or
holding it.

```json
"stand_alone": { "body": "frame_body", "mind": "frame_keeper",
                 "movement": ["walk", "thruster_flight"], "while": "supplied" }
```

`stand_up { assembly }` gives the same entity a body built from the named
plan (06 §6), those movement modes, and the named mind if it has none.

- Each part of that body is the assembly's own layer over the matching
  part: the same material, thickness and damage. A dent taken standing is
  there when it is next worn.
- While it stands it is its own bearer. What its modules confer, they
  confer on it.
- It is then a character like any other. It perceives through its sense
  modules and nothing else. It follows orders that reach it (15 §10), so
  calling it from across a city needs something that carries the call. It
  can be driven by someone elsewhere (06 §8.2). It fights by document 07
  and decides by document 09.
- `stand_down` puts the body away and it is an item again. The same
  happens when `while` stops holding: a suit that runs out of power on its
  feet becomes a heap of metal where it stood.
- Putting on a standing suit is a joint action between it and the wearer.
  When it ends the body is put away and the suit is worn. Stepping out may
  leave it standing.

A suit that comes when called, one that stands guard, a rank of them
fighting together under one collective (09 §8.1), a chest that walks and a
sword that fights by itself are this section.

## 12. Survival and bases

No part of survival play needs a mechanism of its own. This table is a map
for authors.

| Need | Built from |
|------|-----------|
| Hunger, thirst, fatigue, warmth | Pools with rates and threshold rules (06 §4.2) |
| Food, drink, medicine | Items whose interactions add to pools, add traits, treat wounds |
| Cold, heat, bad air | The medium (11 §4) and fields (02 §16) read by rules; shelter is a sealed area with a heater |
| Light | Light sources (15 §3.3), including powered ones |
| Tools and crafting | Recipes, qualities, stations |
| Farming, brewing, curing | Recipes with a condition on the place (section 8.4) |
| Shelter and fortification | Construction; real materials with real strength |
| Power, water, heat distribution | Resources, conduits, networks |
| Automation | Machines with `auto`, movers, item conduits, signals |
| Defence | Devices with senses and minds; signals; traps on hooks |
| Helpers | Characters under orders working jobs (15 §10; 09) |
| Magic workshops, ritual circles, ley lines | The same, with different names |

## 13. Away from the player

### 13.1 Storage

Devices are notable entities (06 §12.2) and are stored with their tracts.
Networks, jobs and queued items are stored with their site or subgrid.

A network that crosses into an unloaded tract keeps a frozen summary of
that part: what it offered, wanted and stored when it unloaded.

### 13.2 Catching up

When a site loads after time has passed, its machines **catch up**: the
same event-driven logic that runs them live is run on a private timeline
for the elapsed local time, with no world effects other than their own
stores, holds and jobs. Runs of identical consecutive cycles are applied in
one step, so a day of smelting costs a handful of operations, not
thousands. Emissions that would have gone into the world (heat, smoke) are
handled by the fields' own catch-up rules (02 §16).

Catch-up is deterministic, and for machines that depend only on their own
networks it gives the same stores and outputs as having stayed.

### 13.3 Staying loaded

A device or rule may hold a ticket (02 §10) to keep its surroundings
loaded, at the cost of the loaded-tract budget. Content decides whether to
offer such a device.

### 13.4 Detail

Everything in this document is event-driven. A loaded base with nothing
changing costs nothing. Warm and hot tracts run devices identically.

## 14. Modding surface

**Definition types**: `interaction`, `device`, `resource`, `conduit`,
`recipe`. Fields `interactions`, `device`, `load`, `when_supplied`,
`conveys` on elements; `loss` on conduits; config kind `logic` and
`stand_alone` on devices; `account` and `floor` on resources; `qualities`
and `worth` on items; `services` on tract definitions.

**Events**: `interacted`, `device_mode_changed`, `device_configured`,
`network_changed`, `network_supplied`, `network_starved`,
`store_emptied`, `store_filled`, `conduit_changed`, `conduit_breached`,
`signal_changed`, `recipe_started`, `recipe_completed`, `recipe_stalled`,
`item_moved`, `job_created`, `job_supplied`, `job_completed`,
`job_cancelled`, `module_installed`, `module_removed`, `module_supplied`,
`module_unsupplied`, `suit_breached`, `service_changed`, `paid`,
`payment_started`, `payment_stopped`, `account_exhausted`, `traded`,
`stood_up`, `stood_down`.

**Hooks**: `interaction_starting`, `interaction_reach`,
`network_overloaded`, `recipe_completing`, `job_starting`,
`module_installing`, `trade_offers`, `trade_agreeing`.

**Effects**: `interact`, `set_state`, `set_mode`, `set_config`,
`add_resource`, `drain_resource`, `link`, `unlink`, `lay_conduit`,
`cut_conduit`, `drive_signal`, `work`, `start_recipe`, `cancel_recipe`,
`create_job`, `cancel_job`, `plan_from_map`, `install`, `uninstall`,
`plug_in`, `unplug`, `set_service`, `hold_ticket`, `trade`, `pay`,
`start_payment`, `stop_payment`, `deposit`, `withdraw`, `stand_up`,
`stand_down`.

**Queries**: `interactions`, `devices`, `networks`, `members`, `jobs`,
`modules`, `recipes`, `offers`, `payments`.

**Properties**: on devices, `mode`, `config.*`, `supply(resource)`,
`store(resource)` with `level` and `frac`, `network(port)`, `process` with
`recipe` and `progress`, `input(name)`, `slot(id)`, `bearer`, `assembly`;
on networks, `offered`, `wanted`, `stored`, `fraction`; on elements,
section 3.5; on any bearer, `supplied(module)`; on characters, groups and
sites, `balance(resource)`.

## 15. Invariants

| # | Invariant |
|---|-----------|
| G1 | Resources are conserved. A network never delivers more than is offered and discharged. |
| G2 | A store's level at any time equals its last recorded level plus its rate times elapsed local time, within its bounds. |
| G3 | A consumer's supply fraction is a function of its network's state and its priority. |
| G4 | Network membership always equals a from-scratch computation from conduits, ports, links and assemblies. |
| G5 | A recipe's progress equals work done over work needed, and its completion is always scheduled at the right time for its current rate. |
| G6 | Work done on a job is never lost by interruption, saving or unloading. |
| G7 | Devices, networks, movers and recipes schedule nothing while nothing about them is changing. |
| G8 | Conduits change only through `WorldEdit`. |
| G9 | Catch-up for a machine that depends only on its own networks gives the same result as staying loaded. |
| G10 | Everything a module confers is removed when it is removed, destroyed or unsupplied. |
| G11 | A payment between two holders conserves the total. A balance at any time equals its last recorded value plus the net rate of its standing payments times elapsed universal time. |
| G12 | A trade happens wholly or not at all. |
| G13 | An assembly is one entity whether worn, lying or standing. Its damage, stores and modules are the same in each. |

## 16. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Rebalance a network of 200 members | under 50 µs |
| Split or merge after one conduit change in a 5,000-cell network | under 1 ms |
| Recipe start or completion | under 3 µs |
| One mover transfer | under 2 µs |
| Listing interactions for a target | under 5 µs |
| 10,000 idle devices | no cost |
| Catch-up of a 100-machine base over a day | under 20 ms |
| Memory per promoted device | under 600 bytes |
| A payment, or a trade of ten entries | under 10 µs |
| 10,000 standing payments with nothing changing | no cost |

## 17. Open questions

1. **True flow limits.** Settled in section 6.2: conduits of different
   capacity are separate networks joined by a junction that passes no more
   than the smaller.
2. **Loss over distance.** Settled in section 6.1: an optional `loss` per
   metre, drawn as a standing load.
3. **Programmable devices.** Settled in section 7: logic tables in a
   device's configuration.
4. **Belts you can stand on.** Settled in section 9.4: moving floors.
5. **Catch-up across sites.** Kept as an accepted approximation. A machine
   fed by another site's output catches up from that site's summary, which
   may itself be stale.
