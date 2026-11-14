# 15 — Perception, Belief and Social Behaviour

Status: design complete; sections 3 to 7, 13 and 14 built for the beta
(decisions D-610 to D-619), as their "As built" notes say.
Implementation plan: [plans/first-playable.md](plans/first-playable.md).
Serves scenes S6, S7, S13, S15 and S16 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how characters come to know things and what they do
about each other: signals and senses, light, detection, percepts, beliefs
and memory, identity and disguise, relations and groups, orders, speech, and
the pattern for authoring reactions.

It is the implementation of rule R11: characters act on belief, not truth.

It does not specify how characters choose goals and plan (09), nor how what
the player's characters perceive is drawn (12).

Names of senses, relations and message kinds in examples are illustrative
content.

## 2. The picture

Things give off **signals**: light reflected from a body, the sound of a
footstep or a breaking window, a scent hanging in the air. A character has
**senses**, each tuned to a kind of signal. When enough of a signal reaches
a sense, the character **perceives** something, and a `perceived` event
fires for rules to act on.

What is perceived goes into the character's **beliefs**: where they last saw
someone, what they saw happen, who they think did it. Beliefs can be stale,
partial and wrong. Everything a character decides is decided from them.

Characters do not perceive entities as such. They perceive a **persona**:
who that appears to be. A disguise presents a different persona. A witness
remembers the persona they saw, and that is who gets blamed.

Characters hold **relations** toward personas and **groups**: fear, trust,
loyalty, the right to give orders. They **speak**, and speech carries
messages that move beliefs from one mind to another.

None of this prescribes behaviour. It gives authors the facts a character
has and the events to hang reactions on.

## 3. Signals

### 3.1 Kinds

```json
[
  { "type": "signal_kind", "id": "sight", "model": "line",    "falloff": 1, "uses_light": true,
    "standing": 1, "contained": 0, "identifies": true },
  { "type": "signal_kind", "id": "sound", "model": "through", "falloff": 2, "delay": true,
    "noise": true, "block": { "default": 0.3, "materials": { "glass": 0.5 }, "tags": { "mineral": 0.8 } } },
  { "type": "signal_kind", "id": "scent", "model": "field",   "field": "scent" }
]
```

| Model | How strength reaches an observer |
|-------|----------------------------------|
| `line` | Needs line of sight (02 §22). `received = strength × exposed × light / max(1, d)^falloff`. `exposed` is the fraction of the source not hidden by intervening elements, from their heights: of four points spread up the source's height (`EXPOSURE_SAMPLES`), the share a line of sight from the observer's eye reaches. A point source, such as a signal that was given off, is seen or not. A field that hides sight (02 §16, `opaque_from`) in a cell strictly between the two hides the source. `light` is the light at the source's cell now (§3.3), for a kind with `uses_light`, and 1 otherwise. |
| `through` | Passes obstacles with loss. `received = strength / max(1, d)^falloff × Π (1 − block × coverage)` over each block and fixture in the cells strictly between source and observer on the straight line from the one to the other, `block` being what the element's material holds back of the kind (`block` below) and `coverage` the share of its cell's cross-section the element closes. A floor the line passes through between storeys counts with a coverage of 1. Space that is not loaded passes nothing. |
| `field` | The observer reads a field (02 §16) in its own cell: `received` is its level over 255. Direction comes from which neighbouring cell on its storey has more; where none has, the observer's own cell. |

`d` is the distance in metres between the two cells' centres, a storey
counting three, the shorter way round on wrapped axes; closer than a
metre counts as a metre. An observer's eye is nine tenths of the way up
its height (its `Physical`), or 1.5 m above the base of its cell for one
whose body gives no height; a source with no height of its own is taken
to be 1.7 m tall (`SOURCE_HEIGHT_MM`).

With `"delay": true` the signal arrives after `distance / speed`, where
speed is the kind's own `speed` or else the speed of sound of the
atmosphere where it starts (02 §4.1; 11 §4.1). In vacuum such a signal
goes nowhere. Sound is slower than a fast enough character. The signal
is sent to each observer within reach when it is given off; when its
front reaches where that observer stood, an observer still there, or
nearer, receives it, from where it now stands; one that has moved
further off is followed by the front, until it is out of its sense's
range. An observer that comes within reach after the signal was given
off does not receive it.

Mods add kinds: heat, vibration, a magical aura, radio.

The fields of a `signal_kind`:

| Field | Meaning |
|-------|---------|
| `model` | `line`, `through` or `field`. Required. |
| `falloff` | The power of `max(1, d)` strength is divided by. Default 1. |
| `uses_light` | For `line` only: whether `received` is scaled by the light at the source. Default false. |
| `delay` | Whether it takes `distance / speed` to arrive. Not for `field`. Default false. |
| `speed` | The speed a delayed signal travels at, in m/s. Left out, the speed of sound where it starts. |
| `field` | For `field`, and only for it: the field read. Required for `field`. |
| `block` | For `through`: what holds it back, each a share from 0 to 1. `materials` names materials; `tags` gives a share to every material that carries a tag, the largest where a material carries several; `default` is for every other material, 0 if left out. A named material takes its own share before its tags. |
| `standing` | The standing signature of a creature that has not been given one of this kind (§3.2). Default 0. |
| `contained` | The share of a standing signature that leaves a container its owner is inside, and of a signal that reaches an observer inside one: 0 for sight, 1 for sound (§5.3). Default 1. |
| `noise` | Whether a bare `noise` number means this kind (§3.2). At most one kind may say so; a second is a load error. Default false. |
| `identifies` | Whether receiving a signal of this kind tells who stands where it came from (§6). Default false: a sound tells where, not who. |
| `exposes` | What a standing source does to the pools of those it reaches, below. |
| `name`, `doc` | What a reader calls it; what it is, for the reference. |

As built (decision D-611), what stands in the way holds a signal back by
the table on the signal kind and not by a property of the material: a
mod that adds a material gives it a tag the kinds it matters to already
name, or patches those kinds.

A signal kind may **expose** as well as inform:

```json
{ "type": "signal_kind", "id": "radiation", "model": "through", "falloff": 2,
  "block": { "tags": { "metal": 0.9 } },
  "exposes": { "pool": "dose", "per_strength": 0.001, "step": "1 s" } }
```

Each step, every occupant within reach of a standing source of that kind
has the strength it receives, after distance and whatever stands between,
added to the pool. Walls of the right material shelter. An emitter steps
only while someone is within its reach. This is how radiation, a baleful
aura and a cold that goes through stone are made.

As built, `exposes` is read and checked (`pool`, `per_strength`, `step`
above zero) and does nothing yet: the emitter's step waits for the
stats to be wired to the kernel, which is what adds to a pool.

### 3.2 Where signals come from

| Source | How |
|--------|-----|
| **Presence** | Every entity in the world has a standing signature per signal kind, an expression on its templates and traits: how visible it is, how much noise it makes standing still. |
| **Actions** | An action or phase may declare `signals`, emitted when it starts. Movement definitions declare `noise` (06 §11). A bare `noise` number, wherever a definition has one, is the strength of a signal of the kind flagged `"noise": true`. |
| **Events** | An `event_signal` definition maps an event type to a signal: breaking glass makes a sound in proportion to the energy involved. |
| **Rules** | The `emit_signal` effect, with any strength and any payload, true or not. |

A signal carries what it is about (an entity, an event, or nothing in
particular), where it came from, and its strength.

As built (decision D-612):

- **Standing signatures** are the `Signature` component: a strength for
  each kind an entity has been given (`set_signature`), and for a kind
  it has not, the kind's `standing` if it is a creature and nothing
  otherwise. Inside a container the signature is multiplied by the
  kind's `contained`. Templates and traits will give signatures from
  their expressions (06); until then the simulation's own functions do.
- **Giving one off**: the function `emit_signal(sim, signal)` and the
  effect of the same name, `{ "emit_signal": { "kind": "sound", "at":
  "self", "strength": 0.5, "about": "self" } }`. `kind` is a signal
  kind's id, qualified or bare; `at` is a cell or something that is
  somewhere; `strength` is a number of zero or more; `about`, an entity,
  is optional. The effect returns how many observers it was sent to. A
  `field` kind cannot be given off (`perception.field_not_sent`). A
  signal goes to every entity within its kind's reach (the furthest any
  sense of the kind reaches) that has a sense of its kind, those inside
  containers included, but not to what it is about.
- **Events** give off signals by an `event_signal`:

```json
{ "type": "event_signal", "id": "element_breaks", "event": "element_destroyed",
  "signal": "sound", "strength": "1 + event.absorbed / 2000", "salience": 0.6 }
```

| Field | Meaning |
|-------|---------|
| `event` | The event type: an engine event by its name, a mod's by its qualified id. Required. |
| `signal` | The kind given off. Required. |
| `strength` | An expression in the event's scope, pure. Required. A strength that is null, not finite or not above zero gives off nothing. |
| `if` | A condition in the event's scope. Default true. |
| `about` | The name of one of the event's subjects that the signal is about: whose awareness it builds in those who receive it. Left out, it is about nothing: breaking glass tells where, not who broke it. Naming a subject the event does not have is a load error. |
| `salience` | The salience, 0 to 1, a percept of it is remembered with (§7.1); 0, the default, is not remembered. |
| `doc` | What it is, for the reference. |

  The signal comes from the event's `here`, or else from where the
  subject it is about stands, or else from the first of its subjects
  that is somewhere, and carries the event's type and the actor of its
  cause.
- **What happens to oneself** is not seen or heard through a signal
  about oneself, since a signal is not given to what it is about.
  Whoever a blow lands on perceives it only through a signal that is
  about nothing, or about someone else, and reaches it: the bundled
  `blow_felt` (decision D-940) is a signal of the kind `touch`, about
  nothing, from where the one struck stands, which the sense `touch`
  (half a metre) receives, so that the one struck alone feels it. It
  makes out who struck it as any percept does: only if it perceives
  them through some sense already.
- `signals` on actions and phases and `noise` on movement wait for the
  packages that run actions and movement, which call `emit_signal`.

### 3.3 Light

```rust
impl Cosmos {
    pub fn light(&self, loc: Location, time: SkyTime) -> Result<Real, WorldError>;
    pub fn light_with(&self, loc: Location, time: SkyTime, moving: &[MovingLight]) -> Result<Real, WorldError>;
    pub fn add_moving_light(&self, loc: Location, level: Real, moving: &[MovingLight]) -> Result<Real, WorldError>;
    pub fn fill_light(&self, grid: GridId, cells: CellBox, time: SkyTime,
                      moving: &[MovingLight], out: &mut Vec<Real>) -> Result<(), WorldError>;
    pub fn static_light(&self, loc: Location) -> Result<StaticLight, WorldError>;
    pub fn sky(&self, loc: Location) -> Result<Option<&Sky>, WorldError>;
}
```

`light` gives illumination from 0 (dark) upward, with no upper end. 1 is
what a cell open to the sky gets from a sky at level 1: full daylight, by
convention. It is the sum of what the sky gives the cell and what the
sources that see it give it, and is never below zero.

**The moment.** The world does not hold the timeline, so the moment is
given: a `SkyTime` carries the local time of the realm's clock
(`Timeline::local_now(realm.clock)`) and the share of the sky's light
that gets through, 1 unless the weather says less (09 §14.1).
`SkyTime::now(cosmos, timeline, grid)` reads the present moment. The
scene may ask for the light of the moment it is playing back.

**Sky.** A stratum names a `sky` definition, which gives ambient light by
local time.

```json
{ "type": "sky", "id": "day_night",
  "day": "24 h", "starts_at": "8 h",
  "light": [ { "at": "5 h",  "level": 0.03 }, { "at": "7 h",  "level": 1.0 },
             { "at": "17 h", "level": 1.0 },  { "at": "19 h", "level": 0.03 } ],
  "spread": { "loss": 0.25, "reach": 8 },
  "backdrop": { "ch": " ", "bg": "sky" } }
```

| Field | Meaning |
|-------|---------|
| `day` | The length of a day. Default 24 hours. Until calendars are built (09 §14.2) a realm's day comes from here. |
| `starts_at` | The time of day when the realm's clock reads zero. Default midnight. |
| `light` | Levels by time of day, in order of time. Between two entries the level runs in a straight line, and from the last round midnight to the first. One entry is a sky that never changes; none is a sky that gives no light, as is a stratum that names no sky. |
| `spread.loss` | The fraction of the light lost for each cell it travels under cover. Default 0.25. |
| `spread.reach` | How many cells it travels at most. Default 8; at most 16. |
| `backdrop` | What is drawn where a view looks down into nothing (12 §6.1). Stored and reported only. |

- Cells open to the sky (02 §6.3 heightmap) receive the sky's level in
  full.
- From them it spreads, cell to cell, through cells that do not stop it.
  A cell stops light when something `opaque` in it stands taller than
  1.5 m (`LIGHT_EYE_MM`): half a storey, the height light is taken to
  travel at. A window, an open door and a hole are alike: nothing in
  them is opaque, and light passes at the ordinary loss.
- A step to one of the four neighbours beside a cell counts as one cell.
  A step to a diagonal neighbour counts as a cell and a half, and needs
  both cells beside the corner clear, so that light never slips between
  two walls that touch at their corners. A step to the storey above or
  below counts as three cells, a storey being three metres, and needs
  the upper cell to have no floor.
- After `n` cells the light is `level × (1 − loss)^n`, the odd half cell
  counting as the square root of a cell's share. Past `reach` cells it is
  nothing: a room is lit near its windows and dark beyond the cap.
- A cell that stops light is lit by its neighbours and passes nothing
  on. One sees the wall of a lit room, from either side.

**Sources.** A floor, block or fixture may give off light, and a state of
it may give off another or none:

```json
{ "type": "fixture", "id": "lamp", "material": "steel", "height": 2.5, "fill": 0.02,
  "light": { "intensity": 1.0, "radius": "8 m" },
  "states": { "on": {}, "off": { "light": { "intensity": 0 } } } }
```

- A source gives `intensity / (1 + d²)` to every cell it sees within its
  radius, `d` being the metres between the centres of the two cells with
  a storey counting three. An intensity may be negative: a source of
  darkness.
- A source sees what an eye in its cell would at 1.5 m: the field of
  view of 02 §22, with its walls that are seen and its sight down and up
  through cells that nothing closes. Something opaque no taller than
  1.5 m casts no shadow on its own storey, though as for sight it
  closes a hole between storeys.
- `radius` is at most 16 m. Left out, it is where the source's light
  falls to 0.01: `√(|intensity| / 0.01 − 1)`, and never more than 16 m.
- The sources that reach a cell are added in a fixed order: chunk by
  chunk, storey by storey and row by row, and within a chunk in cell
  order. A chunk counts at most 128 sources, the first in cell order: a
  cavern of glowing rock is a budget, not a mistake. The fill of a tract
  outside its stored cells gives no light.
- **Moving sources** are not part of the cells: a carried lamp, a muzzle
  flash, a glowing creature. Whoever asks for light hands in the ones
  that matter as a list of `MovingLight` (a cell, an intensity, a
  radius), and each adds in the same way, in the order of the list. The
  world keeps none. Items, creatures and fields tagged as light sources
  reach light this way, supplied by the simulation.
- **Sources areas carry** (02 §8.3; decision D-771): an area, or its
  kind, may carry a source at the middle of its shape. The world counts
  them as moving sources wherever light is read, before those handed
  in. An area anchored to someone takes it along: darkness round a
  person is an anchored area with a negative intensity, and a lantern
  in someone's hand is an area of no size anchored to them with a
  positive one, made and removed by rules on the lantern being taken up
  and put down (`core:lantern`, decision D-960).
- The sky's part and the fixed sources' part together are never below
  zero, and nor is the whole with moving sources added.

**The cache.** How far each cell is from the open sky, in half cells,
and what the fixed sources give it do not change with the time of day.
They are kept for each chunk asked about, in a table beside the chunks
keyed by grid and chunk position, and `static_light` reads them.

- For each chunk the cache keeps its **shape**: per cell, whether it
  stops light, holds anything opaque at all, has a floor and is open to
  the sky, and the sources in it. A shape is read from the cells again when the chunk's structure
  revision moves, or that of any chunk of its tract, since a roof far
  above changes what is open to the sky. When what is read differs from
  what was kept, the box of cells that changed is noted.
- A chunk's light is worked out from the shapes of the chunk and its
  twenty-six neighbours. It is worked out again only when a shape among
  them has changed within light's reach of the chunk: the furthest any
  loaded sky spreads or any loaded source shines. An edit that moves
  nothing opaque, no floor and no source costs nothing, and nor does one
  out of reach.
- Each answer is a function of the cells within reach of its chunk and
  nothing else, so no result depends on the order chunks were asked
  about, and the cache always equals working the light out from nothing
  (invariant P5).
- The cache is derived. It is not saved and not hashed, a copy of the
  cosmos starts with none, and it drops what has not been asked about
  lately when it holds more than 1024 chunks.

**Many cells.** `fill_light` fills a buffer the caller keeps with the
light of every cell of a box, storey by storey and row by row: a screen,
or the box round a field of view. Each cell gets exactly what
`light_with` gives it, at a few nanoseconds a cell.

**Unloaded space.** `light` of a cell whose tract is not loaded is the
error of 02 §7.1; `fill_light` writes zero for such cells. Light neither
comes out of space that is not loaded nor passes through it.

Light is truth about the world. It scales `line` signals whose kind uses
it, and tints what the player sees (12); what a character then believes
is its own.

Not built yet: the weather's share of the sky's light is a number the
caller gives; a realm's day comes from its sky until calendars exist;
and glass that passes only part of the light. `light(at)` is a function
for rules (05 §17). A field's light (02 §16) is not in the cache:
`Cosmos::field_lights(grid, box)` lists it as moving sources, one for
each cell of a field that gives light, with the intensity its level
gives, and whoever asks for light hands them in. A large fire is
thousands of sources: the caller takes those that matter.

**Coloured light (experimental).**

> This is an experimental feature, named `coloured_light` (04 §8.4). A mod
> must opt in to use it, is warned at every load, and nothing bundled uses
> it. It may change or be withdrawn.

A light source may name a colour: `"light": { "intensity": 1.2, "color":
"amber" }`. `Cosmos::light_color(loc)` gives the colour of the light
reaching a cell: the colours of the sources that light it, weighted by how
much each contributes, with the sky's colour by time of day. The scene
tints the cell toward it (12 §6.1).

It is derived data, read only by the scene. Detection uses intensity
alone, as before, and nothing in the simulation reads a light's colour.

| Unresolved | |
|------------|---|
| Whether colour should change what is seen | A red cloak under blue light looks black. Descriptions (section 8.4) do not account for it. |
| Cost | Three cached values per cell where there was one |
| How it combines with ambience, decals and remembered cells | The order of tints in 12 §6.1 was chosen without it |
| Terminals | Sixteen colours cannot show a blend |

## 4. Senses

```json
{ "type": "sense", "id": "human_sight", "signal": "sight",
  "range": 60, "arc": 200,
  "threshold": "0.02 / max(self.fn.sight, 0.01)",
  "gain": 4.0, "decay": 0.5,
  "interval": "500 ms" }
```

| Field | Meaning |
|-------|---------|
| `signal` | The kind this sense receives. Required. |
| `range` | Beyond this nothing is considered, in metres. Required. |
| `arc` | The whole angle, in degrees, that the sense covers, centred on the facing; 360 for hearing. Above 0 and at most 360; default 360. |
| `threshold` | Minimum received strength to register. An expression about the owner (`self`), pure, so injury, darkness adaptation and traits matter. Required. Worked out at each sweep; one that comes to null receives nothing. |
| `gain`, `decay` | How fast awareness builds and fades (section 5), per second. Defaults 4 and 0.5. |
| `interval` | How often the sense sweeps for standing signatures, in the owner's time. Default 500 ms. |
| `suspect` | The awareness at which something is suspected (section 5.1), above 0 and below 1. Default 0.3. |
| `function` | A `body_function` the sense works by. An owner with a body whose capacity of it is zero receives nothing through the sense; an owner with no body is not asked. A creature without a working seeing function does not see: by data, never by naming eyes. |
| `planes` | Whether the sense receives from every plane (02 §8.2), not only those its owner is on. Default false. |
| `scrutiny` | How hard it looks through a disguise; given to `identity_check` (section 8.2). Default 0. |
| `notices` | The kinds of thing a sweep considers in range, of `creature`, `item`, `device` and `other` (what the world calls an occupant's kind). Left out, every kind. A sense given only `creature` passes over the furniture and goods about it when it picks the nearest `perception.max_candidates` (section 5.1); what is inside something it notices is noticed by its own kind; what it already holds awareness of is brought up to date whatever its kind; a party member's sweep considers every kind (D-923). |
| `stopped_by` | The states of its owner in which the sense receives nothing: `{ "vital": [...], "tags": [...] }`, vital states of its mind (`awake`, `unconscious`, `dead`: 06 §8.4) and tags it carries, of its own or from a trait, an item or a template. Left out, none. While one holds, nothing reaches the owner through the sense (a sweep looks at no one and what was held fades, a signal is not received, a party member's view through it is empty), and from the moment none does it works again (D-930). Which states stop which senses is data: the bundled sight is stopped by `unconscious`, `dead` and the tag `asleep`, the bundled hearing by `unconscious` and `dead`. |
| `name`, `doc` | What a reader calls it; what it is, for the reference. |

Senses come from templates and traits (06). A creature may have any number.
A device may have senses too: a camera, a tripwire, sonar (11).

`relay_senses { from, to }` delivers one entity's percepts to another mind,
which perceives and remembers them as its own (08 §7.5). Ending the relay
stops the flow; what was learned stays.

A device with senses may keep what it perceives as memories without having
a mind: a recorder. Its memories can be read by whoever can operate it
(13 §8.4).

The `Senses` component holds which senses an entity has and its **facing**.

As built (decision D-610): facing is in degrees clockwise from north
(the direction of falling `y`), from 0 to 360. An entity with senses
that moves from one cell of its grid to another turns to face the way it
moved; the effect `face { who, toward }` turns it to face a cell,
something that is somewhere, or a number of degrees, and the function
`set_facing` does the same. A sense is given with `attach_sense` and
taken away with `detach_sense`, which forgets what was known through it
(both functions of the simulation; a template's `senses` gives them
when a character is made, decision D-696, and traits will too). Refusals: `perception.unknown_sense`, `perception.has_sense`,
`perception.no_sense`, `perception.no_senses`, `perception.bad_value`.

`relay_senses` and recorders are not built.

## 5. Detection

### 5.1 Awareness

For each observer and each thing it might notice there is an **awareness**
level from 0 to 1.

```
ratio = received / threshold
if ratio ≥ 1:   level += gain × (ratio − 1 + ε) × dt        (faster the more obvious)
else:           level −= decay × dt
```

| Level | Event | Meaning |
|-------|-------|---------|
| crosses `suspect` (default 0.3) | `suspicion_raised` | Something is there |
| reaches 1 | `detected` | Knows what and where; a track is created or updated |
| falls to 0 after detection | `lost_track` | No longer perceiving it |

A signal several times over threshold detects at once. A figure in shadow
at the edge of vision takes seconds, and may slip away first. Thresholds
and rates are data on the sense.

As built (decision D-614):

- `ε` is 0.1 (`AWARENESS_EPSILON`): a signal exactly at its threshold
  still builds, slowly. The level is kept from 0 to 1.
- For a sweep, `dt` is the time since the sense last swept, in the
  observer's own time. A signal that was given off has no length of its
  own: it counts as standing for one interval of the sense that receives
  it.
- Awareness is the `Awareness` component of the observer: for each
  target and sense, the level, when it was brought up to date, and
  whether it is suspected and detected. A pair is made only when
  something is received at or over the threshold, and dropped when its
  level falls to 0. An observer keeps at most `perception.max_awareness`
  pairs (default 128); past that the faintest that is not detected goes
  first, then the longest unchanged. A sweep considers at most
  `perception.max_candidates` of the entities in range (default 32), the
  nearest first and then by id, besides those already in the observer's
  awareness through the sense. They are found nearest first, a band of
  cells at a time, and the search stops at the first band farther than
  the farthest kept (D-920): the same candidates as gathering everyone
  in range, without looking at the furniture of the rest of it.
- The events bind the observer as `self` and the target as `about`:
  `suspicion_raised` (fields `sense`, the sense's qualified id, and
  `level`), `detected` (`sense`, `where`: the cell it was perceived in)
  and `lost_track` (`sense`, `where`: the last known place, where the
  track ends). Detection also raises `perceived` (section 6) and brings
  the observer's track up to date (section 7). While a target stays
  detected and is received, its track follows it. The cause of every
  perception event names no actor.
- `set_awareness { who, of, sense, level }` sets a level through a sense
  the observer has, raising what crossing it raises; the function
  `set_awareness` does the same. `perceives(a, b)` is true from
  detection until the level falls to 0.

### 5.2 When detection is evaluated

Nothing scans every tick.

| Trigger | Checks |
|---------|--------|
| An entity moves, acts, or its signature changes | That entity against observers within range |
| An observer moves or turns | That observer against candidates within range |
| A sense's `interval` elapses | That observer's standing awareness, staggered across observers |
| A signal is emitted | Observers within the signal's reach |
| An explicit `search` or `listen` action | The observer, with whatever bonus the action's rules give |

Only hot entities sweep; warm ones receive only strong signals
(section 14). Pairs with nothing received and no awareness are
not stored.

As built (decision D-614):

| Trigger | What runs |
|---------|-----------|
| A sense's `interval` elapses | The sense sweeps: each entity within its range (those inside containers too), and each target already in the observer's awareness through it, is brought up to date. One wake-up of the domain `Perception` for each observer and sense, on the observer's clock. A sense's first sweep falls at a point of its first interval drawn from a positional hash of the observer and the sense, never from the order senses were given. |
| An observer moves or turns | Each of its senses that has not swept for a quarter of its interval sweeps at the next wake-up, at the same tick. |
| An entity moves, acts, or its signature changes | Caught by the next sweep of every observer within range: at most one evaluation of a pair for each interval. A move makes noise only through the signals it gives off (an `event_signal` on `entity_relocated`, a movement's `noise`). |
| A signal is given off | Its observers within reach, at once or when a delayed one arrives (§3.1). |
| An explicit `search` or `listen` | The function `search(sim, observer, bonus)`: every sense of the observer sweeps now with its threshold divided by `bonus` (1 for none). The actions that call it come with the actions package. |

Whatever the order observers sweep and signals arrive in, each pair's
level depends only on that pair (invariant P4). Events of one sweep are
raised in order of target.

Who cannot perceive through a sense (D-930, which closes D-924's
question): a sense whose `function` its owner's body cannot do, whose
`threshold` comes to null, or one of whose `stopped_by` states holds of
its owner receives nothing; its sweep looks at no candidates and only
fades, interval by interval, what was already held through it, as an
awake observer's awareness fades, and a signal that reaches it is not
received. A party member whose sense of a kind that uses light is
stopped sees nothing: its view through the sense is empty (section 13).
The owner's decision (D-930): the dead perceive nothing, the unconscious
perceive nothing, and a sleeper does not see but hears, and is woken by
a sound loud enough at its ear. All of it is data. Being asleep is no
state of the engine's: in the bundled content a mind that takes up the
`sleep` goal (09 §10.6) is given the trait `asleep`, which carries the
tag `asleep`; `human_sight` is `stopped_by` it, and `human_hearing`'s
threshold is twenty times higher while its owner carries it, so that
only a sound that loud at the ear is heard; the rule
`core:woken_by_a_noise` wakes whoever hears anything asleep, to lie
awake a minute (the trait `woken`) before dropping off again. A sneak
whose steps are quieter (the bundled footsteps are a quarter as loud
for one carrying the tag `soft_steps`, which crouching and lying prone
give through the trait `treading_softly`) passes a sleeper unseen.

A threshold that reads its owner (not a constant) is worked out afresh
for each signal that reaches it, not taken from the last sweep, so that
falling asleep or waking counts at once, and an observer that does not
sweep (a warm one, section 14) receives by what it is now (D-930).

A sweep is not skipped when nothing round the observer has changed
(D-925): what it does depends on the time since the last sweep for every
pair it holds, and the awareness it keeps records when each was last
brought up to date, so a sweep left out would have to be worked out
later exactly as it would have been. Sweeps whose readings could be kept
are those of an observer that has not moved or turned, in whose range
nobody moved and no cell or light changed; in the market town by day
someone in range moves at nearly every sweep.

### 5.3 Concealment

Being hard to perceive is not a special state. It follows from the numbers:

| Factor | Effect |
|--------|--------|
| Darkness | Lowers `light`, so lowers received sight |
| Cover | Lowers `exposed` |
| Distance, walls | Falloff and blocking |
| Posture, size | Lower signature |
| Stillness, slow movement | Lower sound signature |
| Outside the arc | Not received by that sense |
| Inside a container (06 §9.2 `Inside`) | No sight signature at all; sound still leaves |
| Traits | Camouflage, invisibility, silence: modifiers on signature |
| Another plane (02 §8.2) | Not received unless the sense spans planes |

Scene S13 is the row about containers plus section 7.2.

As built: inside a container is the kind's `contained` (0 for the
bundled sight, 1 for sound), both for a target inside one and for an
observer looking out of one; posture and size are what signatures
data gives; modifiers on signature from traits wait for traits (E3).

## 6. Percepts

When awareness of something reaches detection, when a signal carrying an
event is received, or when a message is heard, the observer gets a
**percept** and the event `perceived` is raised.

```
perceived
  self      the observer
  sense     which sense
  about     a persona, a location, or nothing
  event     the underlying event, if the signal carried one: its type,
            and its roles as this observer made them out. They are
            personas, never entities. `about_actor` is whoever the
            observer took the actor for, or null. The true actor and
            instigator (02 §7.3) are not in it.
  where     where it seemed to come from
  clarity   received / threshold
```

Filters (05 §8.4) select by sense, by underlying event type, and by tags of
what it is about:

```json
{ "on": "perceived", "attached": true,
  "filter": { "event": "element_destroyed", "signal": "core:sound" },
  "if": "event.clarity > 2 and not self.has_trait('deaf')",
  "do": [ { "enqueue_action": { "entity": "self", "action": "investigate", "params": { "at": "event.where" } } } ] }
```

As built (decision D-615), `perceived` binds `self`, the observer, and
`about`, an `entity`, when the observer made out who or what it was
about: the entity itself, which a mind may read only through its belief
properties (section 7.2), and which those translate to the persona the
observer takes it for (section 8; decision D-760). Its fields:

| Field | Type | Holds |
|-------|------|-------|
| `percept` | `string` | `detection`: awareness of `about` reached 1; `signal`: a signal that was given off was received |
| `sense` | `string` | The sense's qualified id |
| `signal` | `string` | The qualified id of the kind of signal that carried it |
| `event` | `string?` | The type of the event the signal carried |
| `about_actor` | `entity?` | Whom the observer took the event's actor for, or null |
| `where` | `loc?` | Where it seemed to come from |
| `clarity` | `real` | `received / threshold` |

A filter names a field with a constant: ids are written qualified. The
observer makes out what a signal is about, and the actor of the event it
carries, only if it has them detected through some sense already, or the
signal's kind `identifies` and they stand where the signal came from;
otherwise `about` is unbound and `about_actor` null. The cause of
`perceived` names no actor, so `actor` and `instigator` are not bound
either. A signal received builds the observer's awareness of what it is
about (section 5), raises `perceived`, has the observer remember the
event it carries if the `event_signal` gave a salience (section 7.1),
and, for a controlled character, notes it in the party's next frame
(section 13).

Two functions give rules access to perception:

| Function | True when |
|----------|-----------|
| `perceives(a, b)` | `a` currently has `b` detected |
| `could_perceive(a, loc, kind)` | A signal of that kind at that place would reach `a` now |

Window kinds (03 §7.1) use `perceives(self, source)` as their `noticed_if`.

As built: `perceives(a: entity, b: entity) -> bool?` and
`could_perceive(a: entity, at: any, kind: string) -> bool?`, where `at`
is a cell or something that is somewhere and `kind` a signal kind's id,
qualified or bare; each is null for an argument that names nothing.
`could_perceive` asks whether a signal of the kind's `standing`
strength (or 1, where that is 0) at that place, at the height given,
would reach one of `a`'s senses at or over its threshold, within its
arc; a `field` kind is never reached from another place. Both read
truth about `a` and are not safe for belief: a mind asks the properties
of the same names of itself (section 7.2). Whoever builds windows asks
`notices_source(sim, candidate, source)` before a candidate's
`noticed_if`: a window with no source is noticed, a candidate notices
its own, and otherwise only a candidate that currently perceives the
source does.

**Something is not as it was.** A map may mark doors, containers, fixtures
and placed items as `watched` (10 §3.2). A character whose routine belongs
to that site keeps, for each watched thing, the state it last perceived it
in: open or shut, locked or not, present or gone. When it perceives the
thing in another state, it receives a percept of type `discrepancy`, with
what, where, what was expected and what was found.

That is all the engine does. What an open door means, and whether to look
into it, shout or shrug, is for rules. This is map memory in miniature:
only for watched things, and only for those whose place it is to mind them.

Not built: watched things and the `discrepancy` percept wait for maps to
mark things `watched` (10 §3.2) and for routines.

## 7. Beliefs

```rust
pub struct Beliefs {
    tracks: BTreeMap<PersonaId, Track>,
    memories: Vec<Memory>,
    map: Option<MapMemory>,
    noted: Vec<Noted>,          // bounded: things noticed about the world
}
```

### 7.1 What is believed

| Kind | Holds |
|------|-------|
| **Track** | What this mind knows about one persona: where and when last perceived, what they were doing, how it knows (saw, heard, was told, inferred, or carried back from a time that was undone, 13 §9.3), and a confidence. A track may also hold **facts**: true values of particular properties, recorded by `appraise` (08 §7.5) at the moment of appraisal. It also holds **observed capabilities**: for each action or ability this mind has seen the persona use or been told of, its id, the strongest result seen, and when. The list is bounded by the setting `track_capabilities` (default 8); the weakest and oldest go first. `threat` (09 §6.2) reads it. |
| **Memory** | Something that happened: the event type, where, when, which personas played which roles, how it knows, confidence, and salience. |
| **Map memory** | What places looked like when last seen. Kept for controlled characters. |

**Noted things** are what a character has noticed about the world
itself: that a door was locked, that a room was on fire, that a crate
stood by the gate. Each holds what, where, its state and when. The list
is bounded (`mind.max_noted`), oldest and least used first to go. Watched
things (section 6) are noted from the start by those who mind them.
Belief-side queries (`visible`, `heard`, `tracks`, `threats`, `allies`)
give personas, so a face kept in a variable is still only a face.

Free-form beliefs are variables on the mind.

Memories are bounded. The least salient are forgotten first. Salience is set
by the rule or event mapping that created the memory and fades with time.

As built (decision D-616), beliefs are the `Beliefs` component of a
mind:

- **Tracks** are kept by persona (`PersonaId`, section 8): the persona
  the mind takes an entity for, so a disguise changes what is believed
  without changing how it is kept (invariant P2). A track holds the last known
  cell, the mind's local time it was last perceived or heard of, how
  (`Knowing`: perceived through a kind of signal, told, inferred,
  carried) and the confidence then. At most `mind.max_tracks` (default
  64): the least confident, then the oldest, go first. The effects
  `add_track { who, of, at?, how?, confidence? }` (`how` is `told`, the
  default, `inferred` or `carried`; confidence 0 to 1, default 1) and
  `clear_track { who, of }` (returns whether there was one) change
  them; a mind keeps no track of itself (`perception.self_track`). A
  track told of (§11) records who told it, as the mind took them
  (`source`).
- **Observed capabilities** are recorded on a track by the function
  `observe_capability(sim, mind, entity, what, result, told)`: the
  action's or ability's id, the strongest result, when, and whether it
  was told rather than seen (seen once stays seen). At most
  `track_capabilities` (default 8), strongest first; the weakest, then
  the oldest, go. The actions package calls it when an action resolves
  in view.
- **Memories** hold the event type, the cell, the mind's local time,
  roles as persona or none, how it is known, confidence, salience and,
  for one told (§11), its `source`: the persona the mind took the teller
  for. The `actor` of `remember` is the persona the mind takes it for. At
  most `mind.max_memories` (default 32): the least salient now, then the
  oldest, is forgotten, with `memory_forgotten` (`self`; `event`,
  `where`). Remembering raises `memory_added` (`self`; `event`, `where`,
  `salience`). The effect `remember { who, event, at?, salience?, actor?
  }` remembers as inferred (salience default 0.5); a percept of an event
  whose `event_signal` gives a salience is remembered as perceived.
- **Salience** halves every `mind.salience_halflife` seconds of the
  mind's time (default 600) for choosing what to forget; **confidence**
  halves every `mind.confidence_halflife` seconds (default 60).
- **Noted things** hold the cell, the layer, the element's state and
  when; at most `mind.max_noted` (default 32), the oldest going first.
  The function `note_element(sim, mind, loc, layer)` notes what is there
  now; the search and interaction packages call it.

### 7.2 Staleness

A track is a fact about the past. Confidence decays. The region where the
persona might now be grows with time since last perceived. A searcher works
outward from the last known position because that is all it has.

| Property | Gives |
|----------|-------|
| `self.knows(x)` | Whether there is a track |
| `self.believed_loc(x)` | Last known location, or `null` |
| `self.last_seen(x)` | Local time since |
| `self.confidence(x)` | 0 to 1 |
| `self.awareness(x)` | Current awareness level |

`x` is an entity in the rule's bindings; the engine translates it to the
persona `self` perceives it as. If `self` has never perceived it, the
answers are `null`.

As built (decision D-616), these are properties of an `entity`, read of
the character itself, with these types and companions:

| Property | Type | Gives |
|----------|------|-------|
| `self.knows(x: entity)` | `bool` | Whether there is a track; false for the never-perceived |
| `self.believed_loc(x: entity)` | `loc?` | Last known cell |
| `self.last_seen(x: entity)` | `real?` | Seconds of the mind's own time since it last perceived `x` or heard of it |
| `self.last_seen_at(x: entity)` | `real?` | The mind's local time, in seconds, when it last did |
| `self.confidence(x: entity)` | `real?` | 0 to 1, halved every `mind.confidence_halflife` since |
| `self.awareness(x: entity)` | `real?` | The highest awareness through any sense |
| `self.seen_how(x: entity)` | `string?` | The qualified id of the kind of signal it was last perceived by, or `told`, `inferred`, `carried` |
| `self.perceives(x: entity)` | `bool` | Whether `x` is detected now |
| `self.could_perceive(at: any, kind: string)` | `bool?` | As the function of section 6, for `self` |
| `self.remembers(at: any)` | `bool` | Whether its map memory holds the cell (section 13) |
| `self.knows_element(at: any, layer: string)` | `bool?` | Whether it knows the floor, block or fixture of a cell for what it is (section 7.3) |
| `self.facing` | `real?` | Which way it faces, in degrees |

None is marked safe for belief: a mind context reads them because they
are read of its own subject (05 §7.3), and read of another entity they
would tell one mind what another believes. Their results are not the
character's own: `self.believed_loc(x).x` is safe, and
`self.believed_loc(x).light` is refused.

### 7.3 What rules may read

A rule or behaviour that decides what a character does must read these
properties, not `x.loc`. The linter flags direct reads of another entity's
state in anything evaluated to decide what a character does: rules
attached to minds, templates, creatures, traits, abilities and items, and
the defaults and conditions of actions.

One deliberate exception: **static geometry is common knowledge.**
Path-finding and movement use the true layout of walls, floors and doors.
Elements tagged `hidden` (a secret door, a trap) are excluded until the
character has perceived them. Dynamic facts are never free: who is where,
what is inside a container, whether a door is locked.

As built (decision D-617):

- **The linter.** Each rule attached to an entity (`attached`, or
  written inside another definition) whose `self` is an entity has its
  condition compiled again in the mind context, `self` as its own
  subject. What that refuses as unsafe for belief is reported as a
  warning on the rule, naming the belief property to read instead. The
  rule still loads and runs. Effect arguments are not checked yet.
- **Hidden elements.** An element whose definition carries the tag
  `hidden` is known for what it is only by a mind that has noted it
  (section 7.1). Paths over belief (decision D-619): the function
  `believed_move_rules(sim, mind)` gives movement rules that take
  something hidden and unnoted, which would have to be opened to pass,
  for a wall; `believed_move_rules_with(sim, mind, rules)` wraps the
  mover's own rules the same way. A character with map memory plans
  through cells it has never seen at 1.5 times the cost
  (`UNKNOWN_CELL_FACTOR`). How a hidden element looks is its
  definition's: the view model is given it as it is.
- **What may now be relaxed.** The kernel marked a cell's contents,
  `los` and `light` unsafe for belief until perception was built
  (decision D-466). Their believed counterparts are the properties
  above: `self.could_perceive(at, kind)` for whether a place is in view,
  `self.perceives(x)` and `self.believed_loc(x)` for where someone is,
  `self.remembers(at)` and `self.knows_element(at, layer)` for what a
  place holds. A cell's shape (`standable`, `is_open`,
  `obstacle_height`, `has_flag`, `under_open_sky`) and `los` between two
  cells may be marked safe once their getters answer by the static
  layout with elements tagged `hidden` taken for what they appear to be;
  what a cell holds (`block`, `fixture`, `floor`, `occupants`,
  `tags`), `light` and everything about entities stay truth.

## 8. Identity

### 8.1 Personas

```rust
pub struct Persona {
    pub id: PersonaId,
    pub name: Text,
    pub pronouns: PronounsId,        // how text refers to them (05 §4.6)
    pub appearance: Appearance,      // what an observer can go by
    pub tags: TagSet,                // uniform, insignia, role
    pub claims: Vec<GroupId>,        // groups this persona presents as belonging to
}
```

Every character has a true persona, may have others, and **presents** one at
a time. A persona also presents an **apparent kind**: the template whose
ordinary body an observer takes it to have. Previews (07 §13) are worked
out against that body and what is visibly worn, corrected by whatever the
observer has appraised (section 7.1). Appearance is derived from the body and what is worn, with whatever
the persona overrides: a mask, a costume, another face.

Tracks, memories and relations refer to personas. A masked vigilante and the
clerk who is secretly that vigilante are two personas with separate
reputations, friends and enemies, until someone connects them.

### 8.2 Recognition

When an observer detects an entity presenting persona P:

1. The hook `identity_check` runs. Its fields include the disguise's
   quality, the observer's scrutiny and how well the observer knows the true
   persona. Rules set the outcome.
2. If the disguise holds, the observer perceives P. If not, the observer
   perceives the true persona, learns that the two are the same, and
   `disguise_seen_through` is raised.
3. If the observer has no track for what it perceives, it gains one for a
   stranger, holding the features it made out (section 8.4).

### 8.3 Verbs

| Verb | Does |
|------|------|
| `assume_persona` | Present another of one's own personas |
| `create_persona` | Make a new one: a cover identity, a costume |
| `copy_persona` | Present someone else's appearance: a shapeshifter, a perfect mask |
| `reveal_persona` | Link two personas in an observer's beliefs |
| `create_illusion` | Create an entity that is only signals: it can be perceived by chosen senses and has no body |

A crime committed while presenting a copied persona is remembered by its
witnesses as committed by that persona. The world's cause records
(02 §7.3) still name the real actor. Scene S15 is the gap between the two.

As built (decisions D-760, D-761), in `clash_sim::social`:

- **A persona** is `PersonaId`: its owner, the entity whose true
  persona it is or who made it, and a number, 0 for the true persona.
  Rules write it as text, `7v1/0` (the owner's slot and generation,
  then the number). A record (`PersonaRecord`) holds what a persona is
  beyond its number: `name`, `pronouns` (kept, not yet used by text),
  `tags`, `claims` (group keys, §9.2), `quality` (0 to 1) and `apparent`
  (a template's qualified id). A true persona needs none until something
  is set on it; its name is its owner's own (`Identity`'s).
- **Presenting.** An entity presents its true persona until it assumes
  one of its own it made, or copies what another presents. A copy has a
  quality of its own (default 1, a perfect likeness).
- **Whom an observer takes an entity for** (`persona_of`): what it
  decided when it last detected it, while the entity presents the same;
  else what it presents, followed through the links the observer holds.
  A mind takes itself for its true persona. Every belief property of
  §7.2 and every relation (§9) is read through it, so rules keep naming
  entities (`self.knows(x)`) and the answer is about the persona.
  `perceived` still binds `about` and `about_actor` as entities (D-615).
- **Recognition.** `identity_check` runs when an observer detects an
  entity (awareness reaches 1, §5.1) that presents a persona other than
  its true one and that the observer has not linked to it. It binds the
  observer as `self` and the entity as `about`, with the fields `sense`
  (string), `quality` (real), `scrutiny` (real: the sense's), `familiarity`
  (real: the confidence of the observer's track of the true persona, 0
  for none) and `holds` (bool, mutable, true). A handler sets `holds`;
  with none the disguise holds. Not held, the observer takes the entity
  for its true persona while it goes on perceiving it, and where the
  persona shown is one the entity made it **links** the two in its own
  beliefs for good; a copy of someone else's persona is not linked,
  because that persona is truly another's, and is checked again at the
  next detection. `disguise_seen_through` (`self`, `about`; `shown`,
  `seen`: personas as text; `how`: `checked` or `revealed`) follows.
  When an observer stops perceiving an entity through every sense it
  forgets what it decided, and takes it for whatever it presents at the
  next detection.
- **A change in view is seen.** Whoever perceives an entity at the moment
  it presents another persona goes on taking it for whom it took it
  for, and links a persona the entity made to its true one if it knew
  who it was. A disguise is put on out of sight.
- **Verbs**: `create_persona { who, name?, tags?, claims?, quality?,
  apparent? }` gives the persona as text (quality default 0.5);
  `assume_persona { who, persona }` (one of its own: refused with
  `social.not_own_persona`, `social.unknown_persona`); `drop_persona {
  who }`; `copy_persona { who, of, quality? }` presents what `of`
  presents and gives it; `reveal_persona { who, of }` links, in `who`
  only, what `of` presents to its true persona, raises
  `disguise_seen_through` and gives whether anything was hidden;
  `set_persona { who, persona?, name?, tags?, claims?, quality?,
  apparent? }`; `create_illusion { at, signals, of? }` makes an entity
  of kind `other`, tagged `illusion`, that blocks nothing and gives off
  only the standing signals `signals` names (a map of signal kind to
  strength), presenting what `of` presents; `learn_name { who, of }`
  (the reader lens, §13).
- **Properties** of an entity read of itself: `persona` and
  `presenting` (string), `persona_of(x)` (string?), `knows_name(x)`
  (bool).
- **Not built**: features and descriptions (§8.4): there is no
  `appearance_feature` type yet, a stranger is described by the name of
  the template it looks like, and `suspects` and the merging of a
  stranger's track wait for features; pronoun sets (the `pronouns` type)
  are carried on a record and not read by text; a template or creation
  choice does not yet give a persona its pronouns.

### 8.4 Features and descriptions

An appearance is a set of **features**, each defined by content:

```json
[
  { "type": "appearance_feature", "id": "height", "from": "self.height",
    "bands": [[0, "short"], [1.6, "middling"], [1.85, "tall"]], "clarity": 1 },
  { "type": "appearance_feature", "id": "dress",  "from": "worn_tags(self, 'outer')", "clarity": 1 },
  { "type": "appearance_feature", "id": "voice",  "sense": "hearing", "clarity": 2, "identifying": true },
  { "type": "appearance_feature", "id": "face",   "clarity": 3, "identifying": true }
]
```

`from` derives the value from the body and what is worn. A feature without
`from` is a value the persona carries. A persona may override any of them.

A perception yields only the features whose `clarity` it reaches (section
6) through a sense that can carry them. A glimpse across a dark street
gives height and dress. A close look gives the face.

What a witness got is a **description**: some features, not all. A track
for a stranger holds one, and it grows as more is perceived. A description
**matches** every persona whose appearance agrees with it on each feature
it has, so "a tall figure in red" matches everyone tall and in red.

| When | Then |
|------|------|
| A description gains an `identifying` feature that matches a persona the mind knows | The stranger's track is merged into that persona's |
| It matches several and identifies none | It stays a stranger. The query `suspects(description)` lists the known personas that match. |
| A description is passed on in speech or a report | The hearer gains the same partial track |

Changing clothes changes `dress` and nothing else. A mask hides `face`; it
does not hide height or voice.

## 9. Relations and groups

### 9.1 Relations

```json
[
  { "type": "relation_kind", "id": "fear",     "range": [0, 1], "default": 0, "fade": 0.02 },
  { "type": "relation_kind", "id": "trust",    "range": [-1, 1], "default": 0 },
  { "type": "relation_kind", "id": "commands", "kind": "flag", "orders": true },
  { "type": "relation_kind", "id": "guards",   "kind": "flag", "intervenes": true }
]
```

A relation is a value held by one mind toward a persona or a group.
`self.rel('fear', x)` reads it; verbs set, add and clear it. `fade` moves it
toward its default per minute of local time.

A kind flagged `opposes` or `sides_with` is how the AI layer knows
enemies from friends (09 §3.4).

A kind flagged `intervenes` makes its holder a candidate for windows
opened by actions against the one it is held toward (03 §7.1): the
bodyguard may answer the blow aimed at the person guarded.

Hostility, friendship, debt, love and ownership are relation kinds that
content defines and that the AI layer (09) is told how to read.

### 9.2 Groups

```rust
pub struct Group {
    pub id: GroupId,
    pub def: GroupDefId,
    pub name: Text,
    pub members: BTreeMap<PersonaId, RoleId>,
    pub relations: RelationMap,       // toward personas and other groups
    pub knowledge: Beliefs,           // what the group collectively knows
    pub vars: VarMap,
}
```

A group is any body of people: a watch, a gang, a cult, a regiment, a hive,
an adventuring party. Group definitions name roles and which roles may give
orders to which. A role flagged `spends` may pay from what the group holds
(16 §5.6). A group may have a collective, a mind that decides for it
(09 §8.1).

- A member with no relation of its own toward someone uses its group's.
- A group's stance toward a persona also applies through that persona's
  `claims`. Wearing the uniform gets you treated as one of them, until
  someone sees through it.
- Group knowledge is how "the watch is looking for you" works. Members add
  to it by reporting and consult it when they recognise someone. It counts
  as told to every member the moment it is added. That is a deliberate
  simplification. A mod that wants news to travel adds to it only when a
  report reaches someone who can pass it on.

```json
{ "type": "group", "id": "town_watch", "name": "the Watch",
  "roles": { "sergeant": { "orders": ["guard"], "spends": true }, "guard": {} },
  "collective": "watch_command",
  "relations": { "smugglers": { "hostile": 1 } } }
```

Groups are also made in play: `create_group { def, name }`,
`dissolve_group`, and `set_role { who, group, role }`. A gang, a cult, a
party and a raised horde begin that way.

As built (decisions D-761, D-762, D-765), in `clash_sim::social`:

- **`relation_kind`**: `name?` (text), `kind` (`value`, the default, or
  `flag`), `range` (two numbers, default `[-1, 1]`; a flag's is `[0,
  1]`), `default` (real, 0; a flag's is 0), `fade` (real, 0: how far it
  moves toward `default` each minute of the holder's own time), and the
  flags `opposes`, `sides_with`, `avoids`, `orders`, `intervenes` (bool,
  false). `avoids` (an addition, decision D-762) marks one to keep away
  from: what the AI takes for its `feared` stance (09 §6.6).
  A kind cannot both oppose and side with.
- **Storage.** A mind holds each relation toward a persona or a group (by
  key), of a kind, with the value it was set to and when, in its own
  time. Fade is worked out when it is read; a relation at its kind's
  default is not held. Setting one raises `relation_changed` (`self`;
  `kind`: qualified id, `toward`: a persona as text or a group's key,
  `group`: bool, `value`, `was`: real). Fading raises nothing.
- **Reading** (`self.rel(kind, x)`, `x` an entity or a group's key or
  id): the mind's own relation toward the persona it takes `x` for;
  else the strongest of its own relations toward the groups that
  persona claims; else the strongest of what its groups hold: each
  group's relation toward the persona, else the strongest of the
  group's relations toward the groups the persona claims, the group's
  own toward each before what its definition gives; else the kind's
  default. The strongest is the value furthest from the default, the
  first in order of key of equals. `self.stance_to(x)` is `opposes`
  when a kind flagged `opposes` is held above its default by more than
  any flagged `sides_with`, `sides_with` the other way round, else
  `neutral`.
- **`group`**: `name?` (text), `roles` (by id: `name?`, `orders`: the
  roles of the group it may order, `spends`: bool), `collective` (kept
  for the AI), `relations` (for each group by id, each kind by id, a
  value). A group is named by its **key**: a definition's group by the
  definition's qualified id (`core:watch`), one made in play by the
  definition's id and a number (`core:gang#1`). A definition's group
  exists from the start; what it keeps of its own is made when needed.
- **Members are entities**, not personas (15 §9.2 wrote personas): a
  member belongs openly or in secret. A true persona **claims** every
  group its owner belongs to openly; a persona made with
  `create_persona` claims what it is given. A uniform is a persona that
  claims the group: the group's stance applies to its wearer until the
  observer sees through it (§8.2).
- **Knowledge.** `report_to_group { who, group, event?, about? }`: a
  member's memories of an event type, or naming whom it takes `about`
  for (all of them if neither is given), and its track of `about`, are
  added to the group's knowledge and told at once to every other member
  (as told, by the reporter as each takes them, at the reporter's
  confidence), and to whoever joins later.
- **Verbs**: `set_relation { who, toward, kind, value }`, `add_relation
  { who, toward, kind, amount }` (to its own value or the default) and
  `clear_relation { who, toward, kind? }` (its own only; gives how many
  went), `toward` an entity, as the persona `who` takes it for, or a
  group's key or id; `join_group { who, group, role?, secret? }`
  (`group_joined`: `self`; `group`, `role`), `leave_group { who, group
  }` (`group_left`, the same fields), `set_role { who, group, role? }`
  (`role_changed`: `self`; `group`, `role`, `was`), `create_group {
  def, name? }` (gives the key; `group_created`: `group`, `def`),
  `dissolve_group { group }` (members leave with `group_left`;
  `group_dissolved`: `group`), `set_group_relation { group, toward,
  kind, value }` (`toward` a group, or an entity as the persona it
  presents).
- **Properties** of an entity read of itself: `rel(kind, x)` (real?),
  `stance_to(x)` (string), `in_group(group)` (bool), `role_in(group)`
  (string?). The function `intervenes(a, b)` (bool?) is whether `a`
  holds toward whom it takes `b` for a kind flagged `intervenes` above
  its default: what windows ask (03 §7.1), bound with every simulation.
- **Templates** name the groups a thing made from them joins: `groups`,
  group id to role (`""` for none), joined openly when `entity_spawned`
  is delivered. A group a template names that no definition answers to
  is a warning at load, and is skipped.
- **Not built**: a role's `orders` is recorded and not enforced (an order
  is obeyed by relation, §10); `spends` is recorded for G13; group
  variables; a collective's mind (09 §8.1).

## 10. Orders

```
order { from, to, plan, until }     hook order_received (may refuse or alter)
                                    → the order is kept in `to.orders`
```

What a character does about an order is a goal (09 §8.1). The bundled
"do as ordered" goal makes the order's plan its own for as long as it
scores highest, and a job named by an order scores above the rest while
the order stands. `cancel_order` withdraws one, and `until` ends it by
itself. `order` and `say` may name the `signal` kind that carries them
and its `strength`.

- A relation kind with `"orders": true` is what makes an order something to
  obey. Whether it is obeyed is still a rule: loyalty, fear, a contrary
  order from higher up.
- An order has to arrive. By default it is spoken (section 11) and must be
  heard. A telepathic bond is a sense that receives orders at any distance.
- The player's commands to `Delegated` characters (03 §5.1) are orders.
- A summoned or raised servant is created with a relation of a kind
  flagged `orders` toward its master. Lose the relation and it is no
  longer a servant.

As built (decision D-763), in `clash_sim::social::order`:

- `order { from, to, action, with?, until?, kind?, signal?, strength? }`
  says the order aloud (§11) as a message of `kind`, or the first kind,
  in order of id, that carries orders; `action` is an action's id and
  `with` its parameters; `until` is how long it stands once received,
  in seconds. It gives how many it was sent to. Refused with
  `social.not_an_order` for a kind that does not carry orders.
- **Obeyed by relation, as the receiver believes.** The one it is given
  to, if it hears it and makes out who spoke, obeys when it holds toward
  the persona it takes the giver for a relation of a kind flagged
  `orders` above its default: the servant's relation toward its master,
  read as §9 reads every relation. A voice taken for the master's is
  obeyed. The hook `order_received` (`self`, `giver`; `action`: the
  action's id, `with`: its parameters, `obeys`: bool, mutable) may
  change that or cancel. Obeyed, the order is kept and `order_given`
  (`self`, `giver?`; `order`: int, its number, `action`, `with`,
  `until`: seconds or null) follows; not, `order_refused` (`self`,
  `giver?`; `action`, `reason`: `social.not_obeyed`,
  `social.giver_unknown` or a handler's code, `message`).
- **Doing it** is a goal's (09 §8.1). Until the AI has one, the bundled
  rule `core:do_as_ordered` puts the action in the plan when
  `order_given` is delivered.
- **Ending.** `cancel_order { who, order? }` withdraws one by number, or
  all (`social.unknown_order` for one not kept). An order past its
  `until` no longer stands (`self.standing_orders` does not count it)
  and is dropped, with `order_cancelled`, when the orders are next
  changed. When a relation flagged `orders` falls to its default, the
  orders kept from whom it was held toward that nothing else makes one
  to obey are withdrawn. `order_cancelled` (`self`; `order`, `how`:
  `cancelled`, `expired` or `relation_lost`); the bundled
  `core:stop_when_released` clears the plan on `relation_lost`.
- **Not built**: a role's `orders` (who in a group may order whom) is not
  checked; a telepathic bond is a sense of a signal kind that carries
  orders, and needs nothing more; the player's commands to delegated
  characters are commands (03 §5), not orders.

## 11. Speech

```json
{ "say": { "who": "self", "kind": "report", "about": "$memory", "line": "watch_report_lines", "to": "$sergeant" } }
```

`say` emits a signal of a sound kind carrying a **message**: a kind, what it
is about, and optionally a line of text. Anyone who perceives the signal
and passes the `understands` hook receives `message_heard`.

| Message kind | Default handling (bundled rules) |
|--------------|-----------------------------------|
| `report` | The hearer gains the memory or track, marked as told, with confidence scaled by its trust in the speaker |
| `order` | Section 10 |
| `warn`, `threaten`, `greet`, `ask`, … | Whatever rules listening for them do |

Message kinds are data. Mods add their own.

**Lines.** A `lines` definition is a table of text templates with
conditions and weights. Characters do not converse in trees. They say
things when rules tell them to, and what they say is picked from lines that
fit the moment.

**Exchanges.** A short back-and-forth is a joint action (03 §8) whose
phases have each participant say a line.

**Talking to someone.** The `talk` interaction raises the hook
`talk_options`. Rules add options to it according to who is talking to whom
and what each believes. The result is offered through `ask` (05 §10.6).
Each option carries its own effects. A mod that wants conversation trees
can build them from this; the engine does not.

**Spreading.** Beliefs move by `report`. A witness tells a guard, the guard
adds it to the watch's knowledge, and other guards act on it. Away from the
player the same thing happens abstractly (09).

As built (decisions D-764, D-765), in `clash_sim::social::speech`:

- **`message_kind`**: `name?` (text), `carries` (`nothing`, the default,
  `report` or `order`), `belief` (0 to 1, default 0.5: the share of the
  speaker's confidence a report is believed at), `trust` (a relation
  kind: scales `belief` by where the hearer's relation toward the
  speaker stands in its range), `signal` (a signal kind; left out, the
  kind flagged `noise`), `strength` (default 1) and `calls` (default
  false: a call that involves whoever hears it, below and section 14;
  the bundled `help` is one).
- **`lines`**: `lines`, each `text` (a text with `self` the speaker, `to`
  and `about`, each an entity or null), `if` (a condition, default
  true) and `weight` (default 1). A line that fits is picked by weight
  from the random stream `social` when it is said, and written for each
  hearer as reader (05 §4.6) when heard.
- `say { who, kind, to?, group?, about?, event?, line?, language?,
  signal?, strength? }` gives off a signal from the speaker's eye height
  about the speaker, carrying the message; it gives how many it was sent
  to. A report carries, as the speaker holds them when it speaks, its
  latest memories (at most four, `MAX_REPORT_MEMORIES`) of the event
  type `event`, or naming whom it takes `about` for, and its track of
  `about`. Refused with `social.unknown_message_kind`,
  `social.unknown_lines`, `social.unknown_group` and `social.no_voice`.
- **Hearing.** Whoever perceives the signal (§6) is asked the hook
  `understands` (`self`, `speaker?`; `kind`, `language`: string or null,
  `understood`: bool, mutable); with no rule answering it understands.
  Then `message_heard` (`self`, `speaker?` if made out, `about?`, `to?`;
  `kind`, `text`: the line for this hearer or null, `addressed`: bool,
  `report`: bool, `clarity`: real, `where`: loc). A message is
  **addressed** to its `to`, to the members of its `group`, or, with
  neither, to everyone; others overhear it.
- **Reports.** Whom a report is addressed to gains its memories and its
  track as told (`Knowing::Told`), with confidence times `belief` (times
  the trust share), now in its own time, each marked with its
  **source**: the persona it took the speaker for, or none. `Memory`
  and `Track` carry `source`.
- **Orders** addressed to the hearer are received as §10 says.
- **Talking to someone.** `social::talk_options(sim, who, to)` raises
  the hook `talk_options` (`self`, `to`; `options`: a list, mutable,
  starting empty) and gives the options rules left, each a map with
  `id`, `text`, and if it says something `kind` and `lines`. The
  application gathers them with its service `talk`, on a copy of the
  simulation so that asking changes nothing, lists them on `ui:talk`
  and says the one chosen with the action `say` (12 §10.7, decision
  D-799).
- **Bundled** (`mods/core/data/talk.json`): the action `say`
  (`to`: entity, `kind`: a message kind's id, `lines`: a table of lines,
  optional), a second long, whose phase runs the verb `say`; the rules
  `offer_greeting` and `offer_warning` on `talk_options`, a greeting
  (`core:greet` with `core:greetings`) and a warning to stay back
  (`core:warn` with `core:warnings`) to anyone; and `greet_back`, in the
  group `social_life`: someone a player does not decide for, greeted
  and having made out who spoke, greets the speaker back, at most every
  half minute (the variable `greeted_back_at`).
- **Properties** of an entity read of itself: `remembers_actor(x,
  event)` (bool: a memory of the type, `''` for any, names whom it takes
  `x` for as the actor), `memory_confidence(event)` (real?),
  `memory_source(event)` (string?: a persona as text), `told_of(event)`
  (bool), `standing_orders` (int).
- **Not built**: exchanges, joint actions (T6) whose phases say lines, are not yet written; there is no
  `talk` interaction yet; a capability told of in a report is not
  recorded (§7.1); languages are names that `understands` handlers
  read, with no definition type.

## 12. Authoring reactions

The pattern for making characters respond to the world:

1. Decide what they must perceive. Listen for `perceived` with a filter.
2. Condition on what they believe and how they relate to those involved.
3. Have them say something, change a relation, remember, and start or plan
   an action.

```json
{ "type": "trait", "id": "law_abiding_citizen",
  "rules": [
    { "on": "perceived", "filter": { "event": "entity_died" },
      "if": "event.clarity > 1",
      "do": [
        { "add_relation": { "kind": "fear", "toward": "event.about_actor", "amount": 0.6 } },
        { "remember": { "event": "event", "salience": 0.9 } },
        { "say": { "kind": "exclaim", "line": "horror_lines" } },
        { "first": [
            { "if": "self.rel('fear', event.about_actor) > 0.8",
              "do": [ { "enqueue_action": { "entity": "self", "action": "flee", "params": { "from": "event.where" } } } ] },
            { "do": [ { "enqueue_action": { "entity": "self", "action": "fetch_help" } } ] } ] }
      ] }
  ] }
```

Attached to a trait or template, a rule like this applies to every character
that has it. Characters built from several templates accumulate reactions
from each. Stories come from their interference.

As built, the bundled answer to a blow struck without cause (decisions
D-940 to D-942; `mods/core/data/social/offences.json`, in the rule group
`social_life`) is written this way, and names nothing the engine knows:

- **The one struck** feels the blow (`blow_felt`, §3.2) and, if it
  perceives who struck it, takes against them; so does **a witness**
  who sees or hears a blow and makes out who struck it, against someone
  it neither holds hostile nor fears (`core:answer_a_blow`, on
  `perceived` with `hit_landed`). The jar of a blow back into the
  striker's own hand is no offence.
- **Taking against** someone (`core:take_against`) is a relation by the
  mind's own weights: `hostile` as much as its `duty` where that
  outweighs its `caution` (the watch), else `fearful` as much as its
  caution (the townsfolk). Held toward the persona it took the striker
  for, so a disguised striker is held to account as the disguise. The
  AI reads it as a stance (09 §6.6): the watch goes for them, the
  townsfolk keep away. A friendship stronger than the new hostility
  outweighs it: a watchman who sees another strike someone does not
  turn on him.
- The first time a mind puts a striker to such a blow it **remembers**
  it, as `core:unprovoked_blow` naming the persona; a member of the
  watch **reports** it to the watch (`report_to_group`), every member
  being told at once; one of the townsfolk not played by anyone that
  took fright (its caution not outweighed by its duty: not a brawler)
  **shouts for the watch**: an alarm (`core:alarm`, said at strength 8,
  a call that carries a report) that whoever hears it is told, and that
  a watchman answers and passes to the watch
  (`core:pass_alarms_to_the_watch`). Calling for help and running
  remain its mind's goals (`flee_danger`), which a watchman who hears
  answers (`answer_call`).
- **Passing a call on** (decision D-948): one of the townsfolk who hears
  a call for help (`core:help`) shouts for the watch in turn, at most
  once in half a minute, as an `alarm` (`calls`, carrying a report),
  which the watch answers as a call (`core:heed_alarm`) and the
  townsfolk do not pass on: a call reaches twice a voice's reach. The
  townsfolk's own shout at a blow is an alarm too.
- **Recognising** (§9.2): whoever comes to perceive a persona it
  remembers, seen or told, as having struck without cause takes against
  it (`core:recognise_an_offender`, on `perceived` with `detection`); so
  does whoever is told while it already perceives them
  (`core:told_of_an_offender`, on `memory_added`).
- A blow nobody made out the striker of has no consequence: struck
  from behind by someone unseen, the one struck knows it was struck and
  not by whom. (Turning to where the blow came from was tried and left
  out: in the last stand's dark it let the strangers see whoever struck
  them and block every blow after, which changed the night's balance.)

## 13. What the player perceives

- Each `SimEvent` is tagged with how the directly controlled characters
  perceived it: seen, heard, both, or not at all. Presentation shows only
  what was perceived, in the manner it was perceived.
- **Map memory** records, per chunk, which cells a controlled character has
  seen and what was there at the time. Presentation draws remembered cells
  as remembered.
- An omniscient mode for debugging shows truth.

The player is subject to the same disguises as anyone. What is shown about
another character is the persona the controlled character perceives.

As built (decision D-766): every simulation is bound with the social
reader lens (`social::social_lens`, 05 §4.6). For a reader, someone (an
entity with a mind, or presenting another's persona) is called by the
name of the persona the reader takes them for if the reader knows it
(`learn_name`, or its own), else described by the name of the template
that persona looks like (`the person`), else `someone`; anything else
goes by its own name, as before.

As built (decision D-618):

- **The party** is every entity whose control is direct (03 §5.1) and
  that has senses. Each time a member's sense of a kind that uses light
  sweeps, the member **observes**: the cells it sees are those in its
  field of view (02 §22) within the sense's range and arc, from one
  storey above its own to three below (`PARTY_VIEW_UP`,
  `PARTY_VIEW_DOWN`), where a creature standing in the cell would be
  received at or over the threshold by the light there.
- **Map memory** is the member's `MapMemory` component: for each cell it
  has seen, its floor, block and fixture, the strongest field and its
  level, the most marked decal, whether it was open to the sky, its
  light from 0 to 255 for 0 to 1, and when. It is kept in slabs of one
  storey of one tract, at most `perception.map_slabs_max` (default
  1024), the slab seen longest ago going first. What is remembered is
  what was there when it was last seen, not what is there now.
- **Frames.** Each observation queues a `PartyFrame`: the cells that came
  into the party's view or changed since the member last saw them, with
  what is there; the cells that left the view of the whole party; every
  entity standing in a cell in view, with its cell and height, but for
  those given no signature of a kind that uses light; and what members
  heard since the last frame (who, from where, the kind, the clarity,
  the event). The application takes them with `drain_frames(sim)`,
  which also says whether frames were dropped (more than 256 waited);
  `snapshot(sim, member)` gives a member's whole map memory, each cell
  marked in view or not, for a fresh view model. A hidden element is
  given as it is: its look is its definition's.
- **Tags.** Every event delivered is tagged as the party perceived it:
  `Seen` if it happened to a member or where the party's view was at
  their last observations; `Heard { from }` if a signal it gave off (an
  `event_signal`) would reach a member's sense at or over its threshold;
  `Not` otherwise. `Sim::party_perceived(&event)` answers for the last
  8192 events delivered (`PARTY_TAGS_KEPT`); playback asks it.
- **The omniscient switch** is the setting `perception.omniscient`
  (`set_omniscient`): with it on, every event is `Seen`. A debug profile
  sets it; the application then fills the view model from truth.
- Frames and tags are records of output: saved, so that a loaded game
  answers for events delivered before, and not hashed. The party's view
  is hashed with the rest.
- **The application** gives the player's character its senses, feeds
  the view model from the frames, the map memory and the cells of the
  party's view, answers playback with the tags, and sets the
  omniscient switch with its debugging view (12 §3.5, decision D-700).
- Not built: moving sources of light (carried lamps, fields that give
  light) are not counted in what a member sees; neither are they in
  `light` for the line model.

## 14. Level of detail

| Level | Perception |
|-------|------------|
| Hot | As specified |
| Warm | Does not sweep. Receives only emitted signals strong enough to exceed a multiple of threshold; receiving one promotes the entity (03 §10) |
| Cold | None. Beliefs are stored with the agent. The cold simulation moves knowledge between agents and groups at its own scale (09). |

As built: an observer standing in a warm tract does not sweep, and
receives a signal only at or over `perception.warm_factor` times its
threshold (default 4). An entity that leaves the world with its tract
keeps its components, beliefs included, unchanged; its sweeps begin
again when it returns (invariant P6).

A character stepped warm by the attention cap (09 §9.4) is a warm
observer too: it receives only signals over the warm factor times its
threshold, and its sweeps stop until it is promoted, when they begin
again. Receiving a loud signal makes its mind think, at the warm
stepper's next step; promotion follows attention (09 §9.3). The party
perceives only hot characters: a warm character standing in a cell a
member sees, at the member's sweep or by its own bulk walk, is promoted
at once (D-891).

A message that **involves** a listener (D-934) is heard by it at its
ordinary threshold however warm it is: one said to it, or to a group it
belongs to, or of a kind that `calls` (section 11). Hearing it promotes
a character stepped warm by the attention cap (03 §10: promotion by
involvement), so a watchman out of the party's sight answers a call for
help as one in sight would.

A delayed signal is sent only to the listeners it could reach at or over
their thresholds (times the warm factor, for a warm listener not
involved by what it carries) from where
they stand when it is given off: the most of it that could arrive, with
nothing in its way, is weighed once against each listener's threshold,
and no front is sent to one that could not hear it (D-894). A listener
that comes closer while the front is on its way is not reached by it.

## 15. Modding surface

**Definition types**: `signal_kind`, `sense`, `event_signal`,
`relation_kind`, `group`, `lines`, `message_kind`, `appearance_feature`,
`pronouns`, `sky` (section 3.3; registered by the world, 02 §23).
`exposes` on signal kinds.
Fields `signals` on
actions and phases; `signature` and `senses` on templates and traits;
`light` on elements, fields, items and traits; `sound_block` on materials.

**Events**: `perceived`, `suspicion_raised`, `detected`, `lost_track`,
`message_heard`, `disguise_seen_through`, `relation_changed`,
`group_joined`, `group_left`, `order_given`, `order_refused`,
`memory_added`, `memory_forgotten`.

**Hooks**: `identity_check`, `order_received`, `understands`,
`talk_options`, `signal_receiving`.

**Effects**: `relay_senses`, `appraise`, `emit_signal`, `say`, `remember`, `forget_memory`,
`add_track`, `clear_track`, `set_awareness`, `set_relation`,
`add_relation`, `clear_relation`, `join_group`, `leave_group`,
`set_group_relation`, `report_to_group`, `order`, `face`, and the persona
verbs of section 8.3, `set_persona` (its name, tags, claims and what it
overrides), `create_group`, `dissolve_group`, `set_role`, `cancel_order`.

A persona has a `quality` (how well it holds as a disguise) and a sense a
`scrutiny`; `identity_check` is given both. If no rule answers, the
disguise holds.

**Subjects**: `persona`, `track`, `memory` and `group` are subject kinds
(05 §5.1). A persona gives `name`, `tags`, `claims`, `pronouns` and its
features; a track the properties of section 7.2; a memory its type,
place, time, roles, how it is known, confidence, salience and a `var`
map.

**Events**: `group_created`, `group_dissolved`, `role_changed`,
`order_cancelled`.

**Settings**: `mind.max_noted`, `mind.max_memories`, `mind.max_tracks`,
`perception.warm_factor` (the multiple of threshold a signal must exceed
to reach a warm character). How sky light spreads is not a setting: it is
`spread` on each `sky` definition (section 3.3).

**Queries**: `tracks`, `memories`, `observers` (who currently perceives a
subject), `members`, `suspects`.

**Settings**: `track_capabilities`.

**Functions and properties**: `perceives`, `could_perceive`, `light(loc)`;
on minds, the properties of section 7.2 and `rel`, `in_group`, `role_in`,
`persona`, `presenting`, `facing`.

**Reachable only from Rust** so far: adding a moving source of light.
The light at a cell is the function `light(at)` and the property
`loc.light` (02 §23), neither safe for belief, and neither counts moving
sources, which the simulation does not hand in yet; the constants of section 3.3
(the height light travels at, the level at which a source's light stops
mattering, the count of sources a chunk takes) are named constants, not
settings.

**As built** (decisions D-610 to D-619), of the lists above:

- Definition types: `signal_kind` (§3.1), `sense` (§4), `event_signal`
  (§3.2), with every field stated there. `exposes` is read and does
  nothing yet.
- Events: `perceived` (§6), `suspicion_raised`, `detected`, `lost_track`
  (§5.1), `memory_added`, `memory_forgotten` (§7.1), each with the
  subjects and fields stated there. They are the engine's: content
  cannot raise them.
- Effects: `emit_signal { kind: string, at: any, strength: real, about?:
  entity }` (§3.2); `face { who: entity, toward: any }` (§4);
  `set_awareness { who: entity, of: entity, sense: string, level: real
  }` (§5.1); `attach_sense { who: entity, sense: string }`, which gives
  an entity a sense, refused with `perception.has_sense` for one it has
  (§4; decision D-715); `add_track { who: entity, of: entity, at?: any, how?:
  string, confidence?: real }`, `clear_track { who: entity, of: entity
  }`, `remember { who: entity, event: string, at?: any, salience?: real,
  actor?: entity }` (§7.1).
- Functions: `perceives(a, b)`, `could_perceive(a, at, kind)` (§6).
- Properties of `entity`: those of §7.2 as built, and `facing`.
- Settings: `perception.warm_factor` (default 4), `perception.max_awareness`
  (128), `perception.max_candidates` (32), `perception.omniscient`
  (false), `perception.map_slabs_max` (1024), `mind.max_tracks` (64),
  `mind.max_memories` (32), `mind.max_noted` (32), `track_capabilities`
  (8), `mind.confidence_halflife` (60 s), `mind.salience_halflife`
  (600 s).
- Refusals: `perception.unknown_sense`, `perception.unknown_kind`,
  `perception.has_sense`, `perception.no_sense`, `perception.no_senses`,
  `perception.bad_value`, `perception.field_not_sent`,
  `perception.self_track`.
- Effects `attach_sense { who, sense }` and `detach_sense { who, sense }`
  (§4), for rules that give or take a sense in play, beside what
  templates and minds give (decisions D-696, D-767).
- Not built: the hook `signal_receiving`; `relay_senses`, `appraise`,
  `forget_memory`; the queries `tracks`, `memories`, `observers`.

**As built** (decisions D-760 to D-769), of sections 8 to 11, each with
the arguments, subjects and fields stated there:

- Definition types: `relation_kind`, `group` (§9), `message_kind`,
  `lines` (§11); `groups` on templates (§9.2). Not `appearance_feature`
  or `pronouns`.
- Events: `disguise_seen_through` (§8), `relation_changed`,
  `group_joined`, `group_left`, `group_created`, `group_dissolved`,
  `role_changed` (§9), `order_given`, `order_refused`,
  `order_cancelled` (§10), `message_heard` (§11). They are the engine's.
- Hooks: `identity_check` (§8), `order_received` (§10), `understands`,
  `talk_options` (§11).
- Effects: `create_persona`, `assume_persona`, `drop_persona`,
  `copy_persona`, `reveal_persona`, `set_persona`, `create_illusion`,
  `learn_name` (§8); `set_relation`, `add_relation`, `clear_relation`,
  `join_group`, `leave_group`, `set_role`, `create_group`,
  `dissolve_group`, `set_group_relation`, `report_to_group` (§9);
  `order`, `cancel_order` (§10); `say` (§11).
- Properties of `entity`, read of the mind itself: `persona`,
  `presenting`, `persona_of`, `knows_name`, `rel`, `stance_to`,
  `in_group`, `role_in`, `standing_orders`, `remembers_actor`,
  `memory_confidence`, `memory_source`, `told_of`. Function:
  `intervenes(a, b)`.
- Subjects: `persona`, `track`, `memory` and `group` are not subject kinds
  yet (decision D-761): a persona is written as text (`7v1/0`), a group
  by its key, and a mind's tracks and memories are read through the
  properties above.
- Refusals: `social.unknown_persona`, `social.not_own_persona`,
  `social.unknown_relation_kind`, `social.unknown_group`,
  `social.unknown_role`, `social.not_member`,
  `social.unknown_message_kind`, `social.unknown_lines`,
  `social.no_voice`, `social.bad_argument`, `social.not_obeyed`,
  `social.giver_unknown`, `social.not_an_order`, `social.unknown_order`,
  `social.not_registered`.
- Not built: features and descriptions (§8.4) and `suspects`; pronoun
  sets; exchanges (T6); the `talk` interaction; collectives; group
  variables; a role's `orders`; the queries `members` and `suspects`.

**Reachable only from Rust**, for the packages that will offer them to
content: giving senses from traits (`attach_sense` and
`detach_sense` are verbs; templates and minds give senses, D-696);
setting a standing signature (`set_signature`); the list of what one
can say to another (`social::talk_options`), which a rule cannot read,
since it raises a hook;
an explicit search or listen with a bonus (`search`, for the actions);
noting an element (`note_element`); recording an observed capability
(`observe_capability`); the omniscient switch (`set_omniscient`); and
the frames of §13 (`drain_frames`, `snapshot`). The list of what one
can say to another is reached by the application's service `talk`
(12 §10.5), not by content.

## 16. Invariants

| # | Invariant |
|---|-----------|
| P1 | A percept exists only if a signal reached a sense at or above threshold. |
| P2 | Tracks, memories and relations refer to personas, never to entities. |
| P3 | Nothing in this document reads another character's state on a character's behalf except through signals. |
| P4 | Detection results are independent of the order observers and sources are evaluated. |
| P5 | Light and awareness caches always equal a from-scratch recomputation. |
| P6 | A stored character keeps its beliefs and relations exactly. |

## 17. Budgets

Targets, with what `cargo xtask bench` measures (14 §6.3, D-730): the
best of several runs on a shared and loaded build machine, in a release
build. Each row with a figure is in `budgets.toml`.

| Operation | Target |
|-----------|--------|
| Light at a cell, cached, in a fill | under 30 ns (measured: 2 ns) |
| Light at one cell asked for alone, cached | under 1 µs (measured: 0.06 to 0.09 µs; 0.6 µs before D-731) |
| Light of an 80 × 50 view after one edit that light cares about | under 1 ms (measured: 0.5 ms in a street of the test city, a wall put up and taken down) |
| One observer-target evaluation, including line of sight at 30 m | under 2 µs (measured: 0.6 µs, from 3 µs: the four heights of `exposed` walked together, D-732) |
| Emitting a signal to 50 observers in range | under 100 µs (measured: 21 µs to send a delayed one, 0.5 ms for all fifty to receive it with their events, from 1.1 ms) |
| One simulated second of perception for 300 hot characters in a crowd | under 4 ms (measured: 9.9 to 11.4 ms in the best of each of several runs, for fifty walking in one street, all in range of each other, with sight and hearing and footsteps; 15 to 23 ms before D-894, 22 to 32 ms before D-731 to D-733; 13.7 to 14 ms on a busier machine after performance 3, 14 to 14.2 ms before it in the same runs) |
| Memory per mind, typical | under 4 KB |

The ignored test `timing_of_perception_in_a_city_block` of
`crates/clash-sim/tests/perception.rs` prints the same figures for its own
scene. The performance pass (D-731 to D-733, D-737) left every result as it
was and halved the instructions counted for the crowd: one evaluation of sight walks the
cells of its four lines once, light at a cell is read from the cache
without comparing what is round it while nothing has changed, a sweep
reads each target's light once, and a delayed signal's arrival no longer
moves the others. A crowd is still over its target, by work that is in
the model rather than in its arithmetic: every footstep is a delayed
signal to each listener in reach, received with its `perceived` event,
and each sight sweep brings up to `perception.max_candidates` pairs up to
date. Fifty in one street is the worst case of that: in a town the
listeners in reach are fewer.

Performance 2 (D-894, D-897): a footstep is now sent only to those who
could hear it from where they stand, so most of a crowd's fronts and
their arrivals are gone, with the same percepts in the crowd; a sweep
gathers who is in range in one unordered pass and keeps the nearest
`perception.max_candidates` without putting the rest in order, with the
same candidates as before (tested against the first forms). In the market town a sweep looks at a range full of furniture
and goods, and the 150 hot characters' sweeps are still the larger part
of a step there (09 §19). A party member's sweep, its view of 60 m,
map memory and frame, takes 0.9 ms again from where it stands, and
8.5 ms the first time, with the light cache and map memory cold (9 ms
and more before).

Performance 3 (D-920, D-921, D-923 to D-925), every result as it was: a
sweep finds its candidates nearest first, a band of cells at a time, and
stops at the first band farther than the farthest it keeps, so it no
longer looks at the furniture of the rest of its range; a party member
standing still keeps its field of view while no cell changes and its
view while the light over it reads the same. In the market square by
day the sweeps of a minute's wait cost a third fewer instructions and
the party's own half; a party member's sweep again from where it stands
takes 0.2 ms (0.9 ms before) and the first 6.7 ms (8.5 ms). The crowd of
fifty is as it was (13.7 to 14 ms, against 14 to 14.2 ms in the same
runs before): its cost is in what is received, not in finding who is
there. Who cannot see and sweeps that change nothing are section 5.2;
`notices` (section 4) lets content keep a sense's candidates to the
kinds that matter to it.

## 18. Open questions

1. **Who keeps map memory.** Kept as it is: only controlled characters.
   Watched things (section 6) give other characters the small part of it
   they need. (09)
2. **Appearance detail.** Settled in section 8.4: features, descriptions
   and partial matches.
3. **Inference.** Settled as far as the engine goes: discrepancy percepts
   (section 6). What a character concludes from one is left to rules.
4. **Light colour.** Offered as an experimental feature (section 3.3):
   a tint for the player, with no effect on perception.
