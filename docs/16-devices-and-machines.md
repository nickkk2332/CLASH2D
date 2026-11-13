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

### 3.5.1 As built

Package G1, decision D-622; `clash_sim::interaction`.

**The definition.** `interaction` has the fields of section 3.1 and
these, all optional:

| Field | Type | Meaning |
|-------|------|---------|
| `name` | text | What a menu calls it; else its id |
| `target` | `element` (default) or `entity` | What it is done to, and so the kind `target` is bound as |
| `reach` | `held`, `same_cell`, `adjacent` (default), `network` or `{ "range": metres }` | Section 3.3 |
| `uses` | channel slots | Carried for the action that does it |
| `time` | duration | How long it takes in the doer's own time (default 0) |
| `noise` | number | Carried for perception |
| `tools` | list of texts | Carried for package G6 |
| `if` | condition, truth | Tested when it is attempted, with `self` (the doer), `target` and `here` (the target's cell) |
| `shown_if` | condition, mind context of `self`, pure | Whether it is listed at all |
| `fail_text` | text, truth, pure | What a refusal for `if` says |
| `do` | effects, truth | What it does, with `self`, `target` and `here` |
| `default` | bool | Used when the thing is bumped into or clicked |
| `offered_by` | `any` | Offered by every target of its kind |

**Who offers what.** An element's `interactions` and a device's
resolve in the namespace of the definition that lists them; an element
that lists one that does not exist is warned of at load, and offers
nothing by it. A device definition's
and an item definition's `interactions` are done to the entity. A
creature offers only what is `offered_by: any`, until templates carry
`interactions`.

**Reach**, tested against where things stand (an entity inside another
stands where its outermost container does): `held`, the target is
inside the doer at any depth; `same_cell`; `adjacent`, the same cell or
one of the eight round it on its storey, or straight above or below
where no floor lies between or either cell has stairs, a ramp or a
ladder; `range`, the distance in metres (a storey counting three) with
a clear line of sight; `network`, a stub until networks exist: the
doer operates a device (one it holds, one within a step, or one an
element within a step names) whose definition carries a tag beginning
`network:` that the target's device definition carries too.

**What is offered.** `interactions_for(sim, who, target)` lists every
interaction the target offers whose `shown_if` holds, each with the
reason it cannot be done now (`interaction.out_of_reach`, or
`interaction.not_now` with its `fail_text`), or none. It changes
nothing and raises nothing, and it reads what `who` believes through
the seam `interaction::belief`, which reads truth until perception is
built. `default_for` is the first default that can be done;
`bump_default(sim, who, dir)` is that of the block, else the fixture,
else the floor of the next cell in one of the ten directions of a step;
`offers_near(sim, who)` lists what everything within a step offers,
and what `who` holds, for a context menu.

**Doing it.** The bundled action `core:interact` takes `target` and
`interaction`. Any action whose parameters are so named is an
interaction's: when it starts, the simulation's handler of
`action_starting` checks against truth that the target offers it, that
the doer reaches it (raising `interaction_reach`) and that `if` holds,
then raises `interaction_starting`; any refusal refuses the action with
its reason (`action_refused`), which the player is told. The action's
phase lasts `time`, and its effect `complete_interaction` tests reach
and `if` again, runs `do` through the rule runtime, raises `interacted`
and lets a device the target has settle. The verb `interact` does all
of it at once: what a device's rule calls.

| Event | Subjects | Fields | Raised |
|-------|----------|--------|--------|
| `interaction_reach` (hook) | `self`; `target?` (an entity) or `element?` | `interaction`, `reach` (`string`); `reached` (`bool`, mutable) | When reach is tested for an attempt. A handler sets `reached` |
| `interaction_starting` (hook) | the same | `interaction` | Before it begins. A handler cancels with a reason |
| `interacted` | the same | `interaction` | Its effects have run |

| Verb | Arguments | Does |
|------|-----------|------|
| `interact` | `who: entity`, `target: any`, `interaction: id<interaction>` | Begins and completes an interaction at once |
| `complete_interaction` | the same | What ends an interaction's action |
| `promote` | `of: element` | Section 4.3; gives the device |
| `put_inside` | `what: entity`, `into: element` | Promotes the element and puts the thing inside its device (`put_in`) |
| `set_state`, `set_material`, `damage`, `destroy` | 02 §23 | On an element through `WorldEdit` |

| Property | Gives |
|----------|-------|
| `entity.can_interact(target: any, interaction: id<interaction>)` | `bool`: as the entity believes, it can be done now. An interaction's action `requires` it |
| `entity.interaction_fault(target: any, interaction: id<interaction>)` | `string?`: why not, or null |
| `entity.interaction_time(interaction: id<interaction>)` | `real?`: seconds |
| `entity.has_item(def: id<item>)` | `bool`: it holds, wears or carries one at any depth |
| `entity.offers_near` | `list<map<any>>`: what the entity, as it believes, could do within a step (`offers_near`), for a menu: each row has `target`, `interaction` (its id), `id` (its qualified id), `name`, `default`, `available`, `why` (the refusal's sentence, or empty) and `where` (`here`, `held`, a compass direction such as `north-east`, `up` or `down`). The `ui` mod's `interact` screen lists it (12 §3.7; decision D-702) |
| `element.interactions` | `list<string>`: what its definition lists |
| `element.device`, `entity.element`, `entity.is_device` | Section 4.3 |

None is safe for belief; the entity's are read of the character itself.
Reasons: `interaction.unknown`, `interaction.not_offered`,
`interaction.out_of_reach` (`interaction`, `reach`),
`interaction.not_now` (`interaction`), `interaction.no_target`,
`interaction.bad_argument` (`verb`, `arg`),
`interaction.not_registered`.

Not built yet: the channels of `uses` are claimed only by what the
action that does it states; `tools`; `interactions` on templates; true
networks; the query `interactions`: what a screen lists is the property
`offers_near` of the character itself, and `interactions_for` and
`bump_default` stay reachable only from Rust, the application answering
a bump with the latter (12 §3.7).

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
| `climate` | Warms or cools the sealed volume it stands in (11 §4.6) |
| `rules` | Attached rules (05 §9.3) |

The device screen (12 §10.7) is built from `config` and the rest of the
definition, so a new machine needs no new screen.

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

**As built** (package E12, decision D-621; `clash_sim::device`). The
minimal device, which package G2 extends with modes, ports, stores,
config, processes, slots and passive loads:

- **The definition** `device` has `name` (a text), `holds` (`volume`,
  `mass`, `accepts`, as an item's, 06 §9.2: what can be put inside it;
  left out, nothing) and `tags` (the device entity carries them). An
  element's `device` names one, as written in the element's namespace;
  one that names nothing is warned of at load at the element's
  `device`, and is never promoted.
- **The component** `Device` holds the definition and the element it is
  bound to (`bound`: a cell and a layer). The entity is an occupant of
  the kind `device`, with no mass and no flags, in the element's cell,
  and carries an `Identity` naming its definition.
- **Promotion.** An element whose definition names a device is promoted
  when it is in any state but its first (the default), and when a rule
  asks with the verb `promote { of: element }`, which returns the
  device. The device is found again by its element: `element.device`.
- **Contents.** `put_in` takes a device as a container with the room
  its `holds` gives: an item is stowed in it, anything else (someone
  hiding) is enclosed (`Hold::Inside`), out of the world's occupant
  index. `take_out` puts it back where the device stands.
- **Demotion.** A device whose element is in its default state, with
  nothing inside it, no variables, no rules attached and no tags beyond
  its definition's, is removed and the element is plain again: tested
  when something leaves it, when its element's state is set, and when
  an interaction that reached it ends.
- **Destruction.** When the element is destroyed, or replaced by
  another element or by nothing, everything inside the device is put
  where something dropped into the cell comes to rest, each with
  `device_spilled`, and the device is removed. Whoever hid there is in
  the world again, where the next front of a blast can reach them.

| Event | Subjects | Fields | Raised when |
|-------|----------|--------|-------------|
| `device_promoted` | `self` (the device), `element` | — | An element became a device entity |
| `device_demoted` | `self`, `element` | — | A device was removed: plain again, or its element gone. `self` no longer exists when it is delivered |
| `device_spilled` | `self` (what came out), `device`, `here` (loc) | `layer` (`string`) | Something came out of a device whose element was destroyed or replaced |

| Property | Gives |
|----------|-------|
| `element.device` | `entity?`: the device bound to it, if promoted |
| `entity.element` | `element?`: the element a device is bound to |
| `entity.is_device` | `bool` |

None is safe for belief. Reasons: `device.none` (`element`: the element
names no device), `device.unknown_def` (`device`), `device.no_element`
(`layer`), `device.not_registered`.

**As built (package G2, beta; decision D-820).** The device model as
far as stations need; `clash_sim::device`. Networks are package G3's:
until they exist a device draws on its own stores.

| Field of `device` | Type | Meaning |
|-------------------|------|---------|
| `modes` | list of texts | Its operating states; it starts in the first. Two of one name are a load error |
| `config` | list of `{ id, name?, kind, default?, options?, min?, max? }` | Settings: `kind` is `flag`, `number`, `choice` (one of `options`, which it must have) or `text`; `default` must be a value of the kind (left out: false, 0, the first option, empty) |
| `stores` | map of resource name to `{ capacity, start?, rate? }` | Quantities it holds and recipes at it draw on: at most `capacity` (above zero), `start` when it is made (default 0), and with `rate` no more than that a second |
| `speed` | number, or `{ value, when }` | How fast it works recipes (default 1); with `when`, nothing in any other mode |
| `auto` | `true`, `false`, or `{ when }` | Whether it starts recipes by itself (section 8.2), and in which modes |
| `provides` | list of know-how ids, or `{ know_how, when }` | Know-how whoever works at it has (section 8.5) |
| `recipes` | list of recipe ids | What it runs by itself, in order of preference; left out, every recipe with `auto` whose `at` names one of its tags, in order of id |

A `when` names modes of the definition's `modes`; one that does not is a
load error. The `Device` component holds, beside its definition and
element, the position of its mode, the settings set (a setting set back
to its default is forgotten) and the level of each store that has been
filled or drawn on. A device whose mode is its first, with nothing set,
every store at its start and no recipe under way, is as it was made:
with nothing inside it and its element in its default state it is
demoted as before. A recipe under way at it is its `Process` component
(section 8.6).

**Passive loads.** An element's `load` and `when_supplied` are not read
until networks exist (G3); the counting they describe is what promotion
already gives: four hundred lamps in their default state are four
hundred cells and no entity, and switching one off makes exactly one
device.

| Event | Subjects | Fields | Raised when |
|-------|----------|--------|-------------|
| `device_mode_changed` | `self` (the device) | `old`, `new` (`string`): the modes | `set_mode` changed its mode |
| `device_configured` | `self` | `key` (`string`), `old`, `new` (`any`) | `set_config` set a setting |

| Verb | Arguments | Does |
|------|-----------|------|
| `set_mode` | `of: any` (a device, or an element whose definition names one, which is promoted), `mode: string` | Puts it in the mode, raises `device_mode_changed`; recipes at it are worked out again. Gives whether it changed |
| `set_config` | `of: any`, `key: string`, `value: any` | Sets a setting, raises `device_configured` |
| `add_resource` | `to: any`, `resource: string`, `amount: real` | Adds to a store, within its capacity; gives what was added |
| `drain_resource` | `from: any`, `resource: string`, `amount: real` | Takes from a store, down to nothing; gives what was taken |

| Property | Gives |
|----------|-------|
| `entity.mode` | `string?`: a device's mode; null for anything else or a device with no modes |
| `entity.config` | `map<any>`: its settings by id, set or default: `self.config.enabled` |
| `entity.store(resource: string)` | `real?`: what a store holds now, less what a recipe under way has drawn |
| `element.mode` | `string?`: its device's mode, or the first of its device definition's `modes` while it is plain |
| `element.store(resource: string)` | `real?`: its device's store, or the definition's `start` while it is plain |

None is safe for belief. Reasons: `device.not_a_device` (`entity`),
`device.unknown_mode` (`mode`), `device.unknown_config` (`key`),
`device.bad_config_value` (`key`: a value of the wrong kind, a number
outside `min` and `max`, a choice not offered), `device.no_store`
(`resource`), `device.bad_amount` (`amount`), `device.bad_argument`
(`verb`, `arg`). Reachable only from Rust: `device::device_for`,
`device_now`, `def_for`, `speed_now`, `speed_of`, `auto_now`,
`provided_by_def`. Not built: ports, port and store states on networks,
`produces`, `consumes`, `converts`, signals, slots, `rules` on devices,
`climate`, `keeps`, the device screen.

### 4.4 Services

Whole-city grids are not wired cell by cell. A tract may carry **services**
in its record: mains power, water, data. A **service entry** is a device
that offers a service to its local network for as long as the tract has it.
The cold simulation (09) and rules turn services on and off. A blackout is
one value changing on some tracts.

### 4.5 Structures

A device is bound to one element in one cell. A machine that is larger, or
that works only when built in the right shape, has a **structure**.

```json
{ "type": "device", "id": "blast_furnace", "name": "blast furnace",
  "structure": [ { "id": "small", "map": "blast_furnace_shape", "core": "mark:core",
                   "ports": { "mark:tuyere": "air", "mark:tap": "out" },
                   "holds": { "mark:charge_door": "in" } } ] }
```

The map (10 §3) draws what must stand around the device, storey by storey.
A palette entry may ask for an exact element, anything with a tag
(`{ "block_tag": "refractory" }`) or anything at all.

- The device is **formed** while the cells around it match the map in one
  of its turns. It works only while formed. An unformed one can say what
  is missing, and where.
- The match is tested when the device is placed and whenever an element
  inside the map's box changes. Changes in one step are tested once.
- While formed, the cells of the structure are the machine. An
  interaction on any of them reaches the device. Ports and holds named by
  marks are at those cells, so a pipe joins at the tap and a loader feeds
  the charge door.
- Destroying or removing a member un-forms it and raises
  `structure_broken`. What follows is for rules: a kiln goes cold, and a
  reactor does something worse.
- A device may list several structures. The first that matches is the one
  it has, and `self.structure` is its id, so a larger build can give more
  capacity or speed. `self.missing` lists the cells an unformed device
  still wants, and what it wants there.

Building one cell by cell and placing one whole are the same thing: its
map is a design (section 10.3). A furnace of firebrick, a ritual circle of
candles and chalk, a reactor, a gate that needs its pillars and a windmill
with its sails are structures.

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

A rate, a capacity or a speed may be an expression of the device and the
place it stands:

```json
"produces": { "power": { "rate": "250 * light(here) * self.condition" } }
```

Such an expression is reactive (05 §7.5): the network balances again when
what it reads changes. Light, wind, current and temperature are region
values that change at known moments, so a field of panels or a row of
mills costs one rebalance when the weather turns and nothing in between.
A sun panel, a windmill, a water wheel in a river's current (11 §4.7) and
a boiler that needs heat are written this way.

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

Where several qualify, the first in the device's list is taken, unless
its configuration names one.

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
(default 6 cells) that the worker may use and knows the contents of; and
in any item network the station is on (section 9). A worker knows what is
in its own site's storage areas and in containers it has looked into
(15 §7.3). Looking for inputs opens nothing and does not create latent
contents (06 §9.3). Crafting at a bench draws on the shelves around
it.

### 8.4 Growing things

A crop, a culture and a slow reaction are recipes whose `while` is a
condition on the place instead of a resource: enough light (15 §3.3), a
field of water, warmth. A planted fixture is a small device running such a
recipe through its growth states. Harvesting is an interaction.

### 8.5 Know-how

What someone knows how to make is **know-how**: a set of named things held
by a mind (06 §8). It goes where the mind goes.

```json
{ "type": "know_how", "id": "smelting", "name": "Smelting",
  "needs": ["fire_making"], "group": "metalwork" }
```

A recipe's `needs` asks for it: `"needs": "knows_how('smelting')"`. It is met
by the worker, or by something at the station that `provides` it: a manual
on the bench, a pattern loaded into a machine. A machine working
unattended has only what is loaded into it.

| Gained by | How |
|-----------|-----|
| Birth, upbringing, a trade | Listed on a template (06 §10) |
| Being taught | A `talk` option or a joint action whose effect is `teach` |
| Reading | An interaction on an item that `teaches` |
| Research | A recipe whose `result` teaches, or whose output is a pattern that provides |

`teach { who, what }` adds it, and is refused with a reason if what it
`needs` is not already known. `unlearn` removes it.

Abilities (08), trade offers and anything else with a condition can ask
`knows_how` in the same way. The engine attaches no ladder to this (01 R10).
An age of bronze and an age of starships are two sets of definitions that
one world may hold side by side.

The **know-how** screen (12 §10.7) lists what a character knows, what each
thing lets them make, and what is now within reach.

### 8.6 As built

Packages G6 (recipes and work) and G17 (know-how), beta; decisions D-821
to D-825; `clash_sim::recipe`.

**The definition.** `recipe` has these fields, all optional:

| Field | Type | Meaning |
|-------|------|---------|
| `name`, `description` | text | What a menu calls it, and a closer look |
| `at` | list of tags | The station: a device whose definition carries one of them. `["hand"]` (the default) for none |
| `inputs` | list of `{ item, count }`, `{ tag, count }` or `{ substance, amount }` | Used up when it starts: items of a definition, items carrying a tag (`count` default 1), or a volume of a substance drawn from containers |
| `tools` | list of `{ quality, level }`, `{ item }` or `{ tag }` | At hand and not used up: an item whose `qualities` has the quality at `level` (default 1) or more, of a definition, or carrying a tag |
| `know_how` | list of know-how ids | What it needs known (section 8.5): by the worker, or provided at the station. The structured form of `needs: knows_how(...)` |
| `needs`, `fail_text` | condition, truth; text | A condition on the worker, `self`, with `station` (the device, or null) and `here`; and what its refusal says |
| `while` | map of resource to a quantity a second, or a condition | Rates drawn from the station's stores while it works, or a condition on the place (section 8.4), pure, with `self`, `station` and `here` |
| `work` | duration | The time it takes at a rate of 1 |
| `rate` | number expression, pure | The worker's rate (skill, tools, injuries), of `self`; left out, 1 |
| `check_every` | duration | How often a process is looked at again (below); left out, the setting `recipe.step` |
| `outputs` | list of `{ item, count, chance }` or `{ substance, amount, chance }` | What it gives; `chance` from 0 to 1, drawn from the stream `recipes` |
| `repairs` | `{ tag, share }` | It mends the most damaged item at hand carrying the tag by a share of its capacity |
| `result` | effects, truth | Run when it completes, with `self`, `station`, `here` |
| `emits` | `{ field: { id: levels }, noise }` | Field levels added at the place when it starts and at each step while it advances; `noise` is carried |
| `auto` | bool | A machine may run it by itself |

An input names one of `item`, `tag` and `substance`, a substance one an
`amount` above zero, a tool one of `quality`, `item` and `tag`, and an
output one of `item` and `substance`; anything else is a load error.

**Starting.** `work::check(sim, worker, recipe, station)` says whether a
recipe can be started or resumed now, and why not: a recipe made at a
station needs one carrying a tag of `at` (`recipe.no_station`) within a
step of the worker (`recipe.out_of_reach`), with nothing else under way
there (`recipe.busy`); its know-how is known or provided
(`recipe.unknown_how`); the station's speed applies in its mode
(`recipe.station_idle`); `needs` holds (`recipe.needs`); then, unless it
resumes work under way where it would be kept, its inputs, tools and
what it mends are at hand (`recipe.missing_input`,
`recipe.missing_tool`, `recipe.nothing_to_mend`) and there is room in
containers at hand for the substances it gives (`recipe.no_room`).
`work::start` does what it says against truth, promotes the station,
raises the hook `recipe_starting`, uses up the inputs of a new start and
raises `recipe_started`.

**Supply.** Inputs and tools are looked for in what the worker holds,
wears and carries at any depth; in the station's device; then in the
contents of devices and items that hold things within
`recipe.supply_range` cells (default 6) of where the work is done, on
the same storey, nearest last by id. A container whose contents are
still latent is passed by. A machine working alone looks in its own
holds only. What a character is told is worked out through the seam
`recipe::supply::belief`, which reads truth until perception keeps what
characters know of containers' contents.

**Doing the work.** A recipe under way is the `Process` component of the
station's device, or of the worker for work by hand: the recipe, the
worker and the action it works by, the station, the work done when it
was last recorded, at what moment of the own time of the worker (while
one works) or the station, the rate and the share of its `while` the
stores give, its one wake-up, why it does not advance, and what it
mends. Its rate is the worker's `rate` (1 for a machine alone whose
`auto` applies; nothing for a process nobody works) times the station's
speed, times the least share its stores give (a store that is empty
gives none; one with a `rate` gives at most that over what is wanted),
times one if its condition on the place holds and none if it does not.
Its wake-up is the earliest of its completion at the present rate, the
moment a store it draws on runs dry, and, for a recipe with a `rate`, a
condition on the place, `emits` or `check_every`, its next step; a
process with none of these schedules nothing while its rate stands
(invariant G7). Whenever the station's mode, settings or stores change,
or its worker stops, the work done and the stores drawn are brought up
to date at the old rate and the wake-up scheduled again at the new
(invariant G5). Completing raises the hook `recipe_completing`, whose
handlers may set `spoiled` or cancel; unless spoiled, the outputs are
made (into the station's holds where they fit, else where it stands; by
hand into the worker's grasp where it can take them, else where it
stands; substances into containers at hand with room), what it mends is
repaired and `result` runs; `recipe_completed` follows, the worker's
held action is let go, and a machine may start its next recipe.

**Working.** An action whose parameters are named `recipe` (an id, or a
qualified id as a text) and `station` (an element, a device or null) is
a work action: when it starts, the simulation's handler of
`action_starting` refuses it with the reason `check` gives. The bundled
`core:make` has one held phase whose `do` is the verb `work`. When the
action ends before the work is done (cancelled, interrupted, its worker
gone) the work stops where it is kept, and starting the same recipe
there again goes on with it. Work by hand left unfinished is kept on
the worker until it starts another recipe by hand.

**Machines.** A device whose `auto` applies, with nothing under way,
starts the first of its recipes that it can from what is in its own
holds and with the know-how it provides, when something is put into it
and when its mode, settings or stores change; and again when a recipe
completes.

**Know-how.** `know_how` has `name`, `description`, `needs` (know-how
ids) and `group` (a text). What a mind knows is the `KnowHow` component
of the entity it is in; `transfer_mind` moves it with the mind. A
template's `know_how` (ids as written) is given to what is made from it
when `entity_spawned` is raised, without asking what each needs. An
item's `teaches` is taught by `learn_from`, and its `provides` is known
at a station it lies in; a device's `provides` in its mode now.

| Event | Subjects | Fields | Raised |
|-------|----------|--------|--------|
| `recipe_starting` (hook) | `self` (the worker, or the machine), `station?` (the device) | `recipe` (`string`) | Before a recipe begins or resumes. A handler cancels with a reason |
| `recipe_started` | the same | `recipe`; `resumed` (`bool`); `work` (`real`, seconds at rate 1) | It began, its inputs used up, or resumed |
| `recipe_stalled` | the same | `recipe`; `why` (`string`): `stopped`, `station`, `supply`, `condition`, `worker` or `cancelled` | It stopped advancing |
| `recipe_completing` (hook) | the same | `recipe`; `spoiled` (`bool`, mutable) | Its work is done, before its outputs. A handler may spoil it |
| `recipe_completed` | the same | `recipe`; `spoiled` (`bool`) | Its outputs are made |
| `know_how_gained` | `self` | `know_how` (`string`); `how` (`string`): `template`, `taught`, `read`, or what a rule said | A mind came to know something |
| `know_how_lost` | `self` | `know_how`; `how` (`unlearned`) | A mind no longer knows it |

| Verb | Arguments | Does |
|------|-----------|------|
| `work` | `who: entity`, `recipe: any`, `at: any?` | Starts or resumes a recipe by a worker, from its held action; refused, it interrupts that action |
| `start_recipe` | `at: any`, `recipe: any` | Starts a recipe at a machine with nobody working it |
| `cancel_recipe` | `at: any` | Gives up what is under way at a station, or a worker's work by hand; gives whether there was one |
| `teach` | `who: entity`, `what: id<know_how>`, `how: string?` | Teaches it; refused with `know_how.needs` while what it needs is not known; gives whether it is new |
| `unlearn` | `who: entity`, `what: id<know_how>` | Takes it away; gives whether it was known |
| `learn_from` | `who: entity`, `from: entity` | Teaches what an item `teaches`; gives how many were new |

| Property | Gives |
|----------|-------|
| `entity.recipe_options` | `list<map<any>>`: what the entity, as it believes, could make here, by hand and at each station within a step; each row `recipe`, `id`, `name`, `station`, `where` (`hand`, `here`, a compass direction), `available`, `resumes`, `why`, `work`, `inputs` (maps of `what`, `need`, `have`, `text`), `takes`, `gives`. The make screen lists it |
| `entity.can_make(recipe: any, station: any)` | `bool`: as the entity believes, it could start it now |
| `entity.make_fault(recipe: any, station: any)` | `string?`: why not |
| `entity.making` | `map<any>?`: the recipe under way it keeps: `recipe`, `name`, `progress` (0 to 1 now), `rate`, `worker`, `why` |
| `entity.knows_how(what: id<know_how>)` | `bool` |
| `entity.know_how` | `list<string>`: what it knows |
| `entity.know_how_list` | `list<map<any>>`: every know-how, as the entity stands to it: `id`, `name`, `group`, `known`, `ready`, `makes`, `needs`. The know-how screen lists it |

None is safe for belief; a character reads its own. Settings:
`recipe.supply_range` (cells, default 6), `recipe.step` (seconds,
default 10). Reasons: `recipe.unknown` (`recipe`), `recipe.no_station`
(`recipe`, `at`), `recipe.out_of_reach`, `recipe.busy` (`recipe`,
`other`), `recipe.unknown_how` (`recipe`, `know_how`),
`recipe.station_idle` (`recipe`, `mode`), `recipe.needs`,
`recipe.missing_input` (`recipe`, `what`, `need`, `have`),
`recipe.missing_tool` (`recipe`, `what`), `recipe.nothing_to_mend`
(`recipe`, `tag`), `recipe.no_room`, `recipe.bad_argument` (`verb`,
`arg`), `know_how.unknown` (`know_how`), `know_how.needs` (`know_how`,
`needs`), `know_how.no_mind` (`entity`), `know_how.teaches_nothing`
(`item`). The `Stepper` wake-up domain is recipes'.

Reachable only from Rust: `work::check`, `work::start`,
`recipe::progress`, `recipe_options`, `stations_near`,
`supply::at_hand`, `supply::plan`. Not built: jobs and work that several
share (G8), `while` rates drawn from networks, resource and account
inputs, the work rate of tools, light and injuries other than through
`rate`, `noise` as a signal, the query `recipes`, construction.

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

Every `conveyance.step` (default 250 ms), whatever stands or lies on it is
carried along by its speed times the step: creatures, loose items, a
crate, a body. This is a relocation, not a blow: it does not stagger.
Something braced against being moved (a rule on `passage_requested`,
06 §13) stays put, and something that cannot go further because the next
cell is blocked stays where it is.

This is a walkway, a belt that can be ridden, a treadmill, an escalator on
a ramp. It is separate from item conduits (section 9.2), whose goods are a
queue and are not in the world while they travel. A line that must carry
both goods and people uses moving floors, with movers at its ends.

### 9.5 Substances

A **substance** is one stuff, whatever form it is in.

```json
{ "type": "substance", "id": "nightshade", "name": "nightshade extract",
  "unknown_name": "a dark liquid", "color": "violet", "taste": 0.6,
  "forms": { "loose": "nightshade_spill", "vapour": null, "piped": null, "still": null },
  "in_body": { "routes": { "ingest": 1.0, "inject": 1.5, "skin": 0.05 },
               "adds": [ { "pool": "nightshade", "per_ml": 1.0 } ] } }
```

| `forms` | The substance as |
|---------|------------------|
| `loose` | A field (02 §16): spilled, splashed, sprayed |
| `vapour` | A field in the air |
| `piped` | A resource in pipes and tanks (section 5) |
| `still` | A material: standing liquid, or frozen solid (02 §5.4) |

Amounts carry over between forms by volume. A breached pipe spills its
substance as its loose field with no rule written. A drain takes the field
back. A field that is a form of a substance exposes those in it by the
substance's routes (02 §16) without an `expose` rule of its own.

**Containers.** An item, a device or a body part that `contains` a volume
holds amounts of any substances as a mixture. A drink is an item filled
when it is made: `"filled": { "water": 290, "ethanol": 40 }`.

| Verb | Does |
|------|------|
| `pour { from, to, amount }` | Moves a share of a mixture into another container, into a tank, or onto a cell, where it is loose |
| `fill { container, from }` | Takes from a cell's field, from standing liquid, or from a tap |
| `dose { who, from, amount, route }` | Puts it into a body |
| `coat { surface, from, amount }` | Puts it on a striking surface (07 §3.2) |

**In a body.** A dose adds to the pools the substance names, times the
factor for its route. What the pool then does is the pool's own rates,
thresholds and rules (06 §4.2). An antidote is a substance that adds a
negative amount to another's pool. The hook `exposure_incoming` runs for
every route.

| Route | How it happens |
|-------|----------------|
| `ingest` | Eating and drinking: interactions whose effect is `dose` from the container |
| `inject` | A needle; or a coated surface whose hit breaches the outermost living layer (06 §7.2). A share of the coating goes in, and every hit wipes some off. |
| `skin`, `breath` | From a field, as 02 §16 |

A venomous bite is a gland that contains venom and a fang that is coated
from it. Blood that burns is a pool that names a substance (06 §6.1).

**Still water.** Poured into a cell of standing liquid, a substance is
recorded on that cell, and whatever is drawn from the cell takes its
share. A well can be poisoned. Spreading through a lake is not modelled.

Recipes may take substances as inputs and give them as outputs, by
volume, from and into containers at the station. Brewing, distilling and
alchemy are recipes. What something tastes of when swallowed is raised as
`tasted`, with the strongest `taste` in the mixture, for rules to notice.

**As built** (package G19, beta, as far as food, drink, medicine and
poison need; decisions D-826, D-827; `clash_sim::substance`).

| Field of `substance` | Type | Meaning |
|----------------------|------|---------|
| `name`, `unknown_name` | text | What a reader calls it; what someone who does not know it does (carried) |
| `color` | text | Carried |
| `taste` | 0 to 1 | How strongly it tastes |
| `forms` | `{ loose, vapour, piped, still }`, each an id or null | `loose` is read: the field a spill of it is; the others are carried |
| `levels_per_litre` | number, default 10 | Levels of its `loose` field a litre poured onto a cell adds, at most 255 |
| `in_body` | `{ routes, adds, traits }` | `routes`: a factor for each of `ingest`, `inject`, `skin`, `breath`; `adds`: `{ pool, per_ml }`, negative for an antidote; `traits`: `{ trait, at_least_ml, for }` |
| `share` | 0 to 1, default 0.5 | The share of a coating a breaching hit puts in |
| `wipes` | 0 to 1, default 0.25 | The share of a coating every hit wipes off |

On items: `contains` (a volume) and `filled` (a volume for each
substance) of 06 §9.1. An item filled and with no `contains` holds what
it was filled with: a loaf is made of bread. What one member of an item
holds, and what is coated on it, is its `Substances` component, in
cubic metres; equal items stack only when they hold the same, and a
member split off holds what the others do.

A dose by a route: for each substance, its volume in millilitres times
the route's factor gets in; for each pool it `adds` to, the hook
`exposure_incoming` runs with `pool`, `amount`, `route` and the
substance's qualified id as `field`, and what its handlers leave is
added to the pool if the body has it; each trait whose `at_least_ml`
got in is given for its `for` (or its own duration); `dosed` is raised
for each substance, and `tasted` once for a dose by `ingest`.

A hit an item lands on a body (07 §3.4, step 6) delivers its coating:
if the blow got through the outermost layer of the body's own part it
struck (what is worn does not count), `share` of each coated substance
goes in by `inject`; then `wipes` of what is left comes off, breaching
or not.

| Event | Subjects | Fields | Raised |
|-------|----------|--------|--------|
| `poured` | `self` (what it came out of) | `amount` (`real`, m³); `onto` (`any`: the container or the cell) | Something was poured |
| `dosed` | `self` (the body) | `substance` (`string`); `amount` (`real`, ml that got in); `route` (`string`) | A body took a substance in |
| `tasted` | `self` | `substance` (`string`); `taste` (`real`) | Something swallowed tasted of something |

| Verb | Arguments | Does |
|------|-----------|------|
| `pour` | `from: entity`, `to: any` (a container or a cell), `amount: real?` | Pours a share of a mixture (all with none), as far as there is room; onto a cell, as its `loose` field. Gives the volume that left |
| `fill` | `container: entity`, `from: any` | From another container, as `pour`; from a cell, each substance whose `loose` field is there, a level for every `1 / levels_per_litre` litres |
| `dose` | `who: entity`, `from: entity?`, `route: string`, `amount: real?`, `substance: id<substance>?` | Puts an amount of a container's mixture into a body, or with `substance` an amount from nowhere. Gives the millilitres that got in |
| `coat` | `surface: entity`, `from: entity`, `amount: real` | Puts some of a mixture on an item |
| `take_in` | `who: entity`, `what: entity`, `route: string` | One member of an item goes into a body whole, with what it holds: eating. (`consume` is a reserved effect name) |

| Property | Gives |
|----------|-------|
| `entity.mixture` | `map<real>`: what one member holds, by substance, in m³ |
| `entity.holds_substance(substance: id<substance>)` | `real`: how much of one |
| `entity.fill` | `real`: how much in all |
| `entity.room` | `real?`: the room left; null for what holds none |
| `entity.coating` | `map<real>`: what is on its surfaces |
| `loc.field(field: id<field>)` | `int`: the level of a field in the cell, 0 to 255 |

None is safe for belief. Reasons: `substance.unknown` (`substance`),
`substance.not_a_container` (`entity`), `substance.empty` (`entity`),
`substance.no_room` (`entity`, `room`), `substance.bad_amount`
(`amount`), `substance.bad_route` (`route`), `substance.bad_argument`
(`verb`, `arg`), `substance.not_registered`. Not built: devices and
body parts that contain, pipes and tanks (`piped`), standing liquid and
wells (`still`), fields that are a form of a substance exposing by its
routes, pools that name a substance spilling it, `unknown_name`.

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
| Rooms that hold warmth or air | Sealed areas (11 §10) and fields. `enclose` makes one from a closed space (02 §8.3). |
| A base the world knows about | `found_site` (09 §13.5): a job board, staff, stock, a holder, and a place for things to happen to |
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
| Cold, heat, bad air | Temperature (11 §4.6) and the medium (11 §4) read by rules; shelter is a sealed area with a `climate` device |
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
`recipe`, `know_how`, `substance`. `structure`, `provides` and `teaches` on devices and
items. Fields `interactions`, `device`, `load`, `when_supplied`,
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
`stood_up`, `stood_down`, `structure_formed`, `structure_broken`,
`know_how_gained`, `poured`, `dosed`, `tasted`.

**Hooks**: `interaction_starting`, `interaction_reach`,
`network_overloaded`, `recipe_starting`, `recipe_completing`,
`job_starting`, `module_installing`, `trade_offers`, `trade_agreeing`,
`payment_making`, `standing_up`.

| Hook | Runs before | A handler may |
|------|-------------|---------------|
| `interaction_starting` | An interaction's action begins | Cancel, with a reason: a lock, a permission |
| `interaction_reach` | Reach is tested | Extend or deny it |
| `network_overloaded` | A network or junction is asked for more than it carries | Decide what gives: limit, trip, burn |
| `recipe_starting` | A recipe begins, by hand or by machine | Cancel, with a reason |
| `recipe_completing` | Its outputs are made | Change or spoil them |
| `job_starting` | Work begins on a job | Cancel, with a reason |
| `module_installing` | A module goes into a slot | Cancel, with a reason |
| `trade_offers` | Offers are shown | Add, remove and reprice offers |
| `trade_agreeing` | A trade is carried out | Cancel, with a reason |
| `payment_making` | `pay`, `deposit`, `withdraw`, or the start of a standing payment | Change the amount (a tax, a fee); cancel (a frozen account) |
| `standing_up` | An assembly stands alone | Cancel |

`job_blocked` is raised when a job cannot go on, with why.

**Further fields.** A device may `transmit` a resource over a range, for
ports that connect by range. A `service_entry` names the tract service it
offers. A signal port may be `wireless`, on a channel set in `config`. An
output may have a `delay`, of at least `signal.min_delay`. An item
conduit `carries` items at a `speed`; receiving ports state what they
`accept` and a `priority`. An `intake` takes a named field from its cell
into a network. A device may `act_for` a character, group or site in
trades. `priority` on a consumer may be an expression. A passive `load`
has a `priority` and is cut or kept as one consumer. `min_supply`
defaults to 0. A recipe's `salvage` may state its own `work`, `tools` and
`needs`; a recipe with `repairs` restores damage for a share of its
inputs. A substance states what `share` of a coating a breaching hit
delivers and what each hit `wipes`; what is wiped is lost, which is the
one exception to invariant G15.

On an element, `when_supplied` is a small map of what it gains (`light`,
`conveys`). On a module it is the conferral block of section 11.2.

**Effects**: `interact`, `set_state`, `set_mode`, `set_config`,
`add_resource`, `drain_resource`, `link`, `unlink`, `lay_conduit`,
`cut_conduit`, `drive_signal`, `work`, `start_recipe`, `cancel_recipe`,
`create_job`, `cancel_job`, `plan_from_map`, `install`, `uninstall`,
`plug_in`, `unplug`, `set_service`, `hold_ticket`, `trade`, `pay`,
`start_payment`, `stop_payment`, `deposit`, `withdraw`, `stand_up`,
`stand_down`, `teach`, `unlearn`, `pour`, `fill`, `dose`, `coat`.

**Queries**: `interactions`, `devices`, `networks`, `network_members`,
`jobs`, `modules`, `recipes`, `offers`, `payments`.

**Settings**: `supply_range`, `network.max_loop_rounds` (how many times a
loop of converters is iterated before it is reported and left as it
stands), `signal.min_delay`, `conveyance.step`.

**Properties**: on devices, `mode`, `config.*`, `supply(resource)`,
`store(resource)` with `level` and `frac`, `network(port)`, `process` with
`recipe` and `progress`, `input(name)`, `slot(id)`, `bearer`, `assembly`;
on networks, `offered`, `wanted`, `stored`, `fraction`; on elements,
section 3.5; on any bearer, `supplied(module)`; on characters, groups and
sites, `balance(resource)`; on devices, `formed` and `structure`; on
minds, `knows_how(what)`; on devices, `condition` (0 to 1, from damage).
`interactions_for(who, target)` lists what applies. In a mind context it
and an interaction's `if` are worked out from what `who` believes; truth
is tested when the interaction is attempted (05 §7.3).

**As built so far** (packages E12 and G1; sections 3.5.1 and 4.3): the
types `interaction` and `device` (`name`, `holds`, `tags`,
`interactions`), and `interactions` on items; the events `interacted`,
`device_promoted`, `device_demoted` and `device_spilled`; the hooks
`interaction_starting` and `interaction_reach`; the effects `interact`,
`complete_interaction`, `promote`, `put_inside` and `set_state`; the
properties `element.device`, `element.interactions`, `entity.element`,
`entity.is_device`, `entity.can_interact`, `entity.interaction_fault`,
`entity.interaction_time`, `entity.has_item` and `entity.offers_near`
(what `offers_near` gives, for a menu). Reachable only from
Rust, each a gap until queries are built (05 §11):
`interaction::interactions_for`, `default_for`, `bump_default`,
`reaches`, and `device::device_at`, `promote`, `settle`
and `unbind`.

Packages G2, G6, G17 and G19 (sections 4.3, 8.6 and 9.5, each with its
tables) add the types `recipe`, `know_how` and `substance`; on devices
`modes`, `config`, `stores`, `speed`, `auto`, `provides` and `recipes`;
on items `filled`, `qualities`, `provides` and `teaches`; on templates
`know_how`; the events `device_mode_changed`, `device_configured`,
`recipe_started`, `recipe_stalled`, `recipe_completed`,
`know_how_gained`, `know_how_lost`, `poured`, `dosed` and `tasted`; the
hooks `recipe_starting` and `recipe_completing`; the effects `set_mode`,
`set_config`, `add_resource`, `drain_resource`, `work`, `start_recipe`,
`cancel_recipe`, `teach`, `unlearn`, `learn_from`, `pour`, `fill`,
`dose`, `coat` and `take_in`; the settings `recipe.supply_range` and
`recipe.step`. Reachable only from Rust, each a gap until queries are
built: `recipe::work::check`, `recipe::recipe_options` (whose rows the
property `recipe_options` gives) and `recipe::supply::at_hand`.

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
| G14 | A device with a structure is formed exactly when the cells in its box match one of its maps in some turn. |
| G15 | A substance is conserved across its forms: what is poured, spilled, drained or drawn is neither made nor lost, except by what a body takes in and what a field's own rules remove. |

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
| Testing a structure of 200 cells | under 20 µs |
| 1,000 formed structures with nothing changing near them | no cost |

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
