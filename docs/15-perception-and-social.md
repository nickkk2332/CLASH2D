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
(both functions of the simulation until templates and traits give
senses). Refusals: `perception.unknown_sense`, `perception.has_sense`,
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
  awareness through the sense.
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
about: until personas (section 8) the entity itself, which a mind may
read only through its belief properties (section 7.2). Its fields:

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

- **Tracks** are kept by persona. Until personas are built (section 8),
  every entity presents one persona whose id is the entity's
  (`PersonaId`), so a disguise will change what is believed without
  changing how it is kept (invariant P2). A track holds the last known
  cell, the mind's local time it was last perceived or heard of, how
  (`Knowing`: perceived through a kind of signal, told, inferred,
  carried) and the confidence then. At most `mind.max_tracks` (default
  64): the least confident, then the oldest, go first. The effects
  `add_track { who, of, at?, how?, confidence? }` (`how` is `told`, the
  default, `inferred` or `carried`; confidence 0 to 1, default 1) and
  `clear_track { who, of }` (returns whether there was one) change
  them; a mind keeps no track of itself (`perception.self_track`).
- **Observed capabilities** are recorded on a track by the function
  `observe_capability(sim, mind, entity, what, result, told)`: the
  action's or ability's id, the strongest result, when, and whether it
  was told rather than seen (seen once stays seen). At most
  `track_capabilities` (default 8), strongest first; the weakest, then
  the oldest, go. The actions package calls it when an action resolves
  in view.
- **Memories** hold the event type, the cell, the mind's local time,
  roles as persona or none, how it is known, confidence and salience. At
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
- A summoned or raised servant is created with a `commands` relation from
  its master. Lose the relation and it is no longer a servant.

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
threshold (default 4). Promoting it (03 §10) waits for the attention
of 09 §9.3. An entity that leaves the world with its tract keeps its
components, beliefs included, unchanged; its sweeps begin again when it
returns (invariant P6).

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
  }` (§5.1); `add_track { who: entity, of: entity, at?: any, how?:
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
- Not built: the hooks `identity_check`, `understands`, `talk_options`,
  `order_received` and `signal_receiving`; `relay_senses`, `appraise`,
  `forget_memory`; the queries `tracks`, `memories`, `observers`; and
  everything of sections 8 to 11.

**Reachable only from Rust**, for the packages that will offer them to
content: giving and taking senses (`attach_sense`, `detach_sense`, for
templates and traits); setting a standing signature (`set_signature`);
an explicit search or listen with a bonus (`search`, for the actions);
noting an element (`note_element`); recording an observed capability
(`observe_capability`); the omniscient switch (`set_omniscient`); and
the frames of §13 (`drain_frames`, `snapshot`).

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

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Light at a cell, cached, in a fill | under 30 ns (measured: 1 to 3 ns) |
| Light at one cell asked for alone, cached | under 1 µs (measured: 0.5 µs) |
| Light of an 80 × 50 view after one edit that light cares about | under 1 ms (measured: 0.03 ms in the open, 0.2 ms in a building) |
| One observer-target evaluation, including line of sight at 30 m | under 2 µs (measured: 5 to 8 µs: four lines of sight for `exposed`) |
| Emitting a signal to 50 observers in range | under 100 µs (measured: 33 µs to send a delayed one, 2 ms for all fifty to receive it with their events) |
| One simulated second of perception for 300 hot characters in a crowd | under 4 ms (measured: 23 ms for fifty walking in one street, all in range of each other, with sight and hearing and footsteps) |
| Memory per mind, typical | under 4 KB |

Measured with the ignored test `timing_of_perception_in_a_city_block`
of `crates/clash-sim/tests/perception.rs`, in a release build. A crowd
is over its target: a sweep considers at most `perception.max_candidates`
of those in range, the nearest first, and a target whose signal could
not reach the threshold with nothing in the way costs no line of sight,
but each evaluation in view still follows four.

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
