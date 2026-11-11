# 15 — Perception, Belief and Social Behaviour

Status: design complete, ready for implementation.
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
  { "type": "signal_kind", "id": "sight", "model": "line",    "falloff": 1, "uses_light": true },
  { "type": "signal_kind", "id": "sound", "model": "through", "falloff": 2, "delay": true },
  { "type": "signal_kind", "id": "scent", "model": "field",   "field": "scent" }
]
```

| Model | How strength reaches an observer |
|-------|----------------------------------|
| `line` | Needs line of sight (02 §22). `received = strength × exposed × light / max(1, d)^falloff`. `exposed` is the fraction of the source not hidden by intervening elements, from their heights. |
| `through` | Passes obstacles with loss. `received = strength / max(1, d)^falloff × Π (1 − block)`, where each element on the straight path blocks by its material's `sound_block` (or the named property) scaled by its coverage. |
| `field` | The observer reads a field (02 §16) in its own cell. Direction comes from which neighbouring cell has more. |

With `"delay": true` the signal arrives after `distance / speed`, where
speed is the medium's (02 §4.1; 11 §7). Sound is slower than a fast enough
character.

Mods add kinds: heat, vibration, a magical aura, radio.

### 3.2 Where signals come from

| Source | How |
|--------|-----|
| **Presence** | Every entity in the world has a standing signature per signal kind, an expression on its templates and traits: how visible it is, how much noise it makes standing still. |
| **Actions** | An action or phase may declare `signals`, emitted when it starts. Movement definitions declare `noise` (06 §11). |
| **Events** | An `event_signal` definition maps an event type to a signal: breaking glass makes a sound in proportion to the energy involved. |
| **Rules** | The `emit_signal` effect, with any strength and any payload, true or not. |

A signal carries what it is about (an entity, an event, or nothing in
particular), where it came from, and its strength.

### 3.3 Light

`Cosmos::light(loc) -> Real` gives illumination from 0 (dark) upward.

- **Sky.** The stratum's sky definition gives ambient light by local time.
  Cells open to the sky (02 §6.3 heightmap) receive it in full. It spreads
  from them through non-opaque cells, losing a fixed fraction per cell, to a
  capped distance. Rooms with windows are lit near the windows.
- **Sources.** Elements, fields, items and creatures tagged as light sources
  contribute `intensity / (1 + d²)` to cells they have line of sight to,
  within their radius. An intensity may be negative: a source of darkness.
  Light at a cell is never below zero.
- The static part is cached per chunk and rebuilt when the chunk's
  structure revision or its fixed sources change. Moving sources are added
  at query time.

Light scales `line` signals whose kind uses it, and tints what the player
sees (12).

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
| `signal` | The kind this sense receives |
| `range` | Beyond this nothing is considered |
| `arc` | Degrees either side of facing that the sense covers; 360 for hearing |
| `threshold` | Minimum received strength to register. An expression, so injury, darkness adaptation and traits matter. |
| `gain`, `decay` | How fast awareness builds and fades (section 5) |
| `interval` | How often the sense sweeps for standing signatures |

Senses come from templates and traits (06). A creature may have any number.
A device may have senses too: a camera, a tripwire, sonar (11).

`relay_senses { from, to }` delivers one entity's percepts to another mind,
which perceives and remembers them as its own (08 §7.5). Ending the relay
stops the flow; what was learned stays.

A device with senses may keep what it perceives as memories without having
a mind: a recorder. Its memories can be read by whoever can operate it
(13 §8.4).

The `Senses` component holds which senses an entity has and its **facing**.

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

### 5.2 When detection is evaluated

Nothing scans every tick.

| Trigger | Checks |
|---------|--------|
| An entity moves, acts, or its signature changes | That entity against observers within range |
| An observer moves or turns | That observer against candidates within range |
| A sense's `interval` elapses | That observer's standing awareness, staggered across observers |
| A signal is emitted | Observers within the signal's reach |
| An explicit `search` or `listen` action | The observer, with whatever bonus the action's rules give |

Only hot entities observe. Pairs with nothing received and no awareness are
not stored.

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

Scene S13 is the fourth-from-last row plus section 7.2.

## 6. Percepts

When awareness of something reaches detection, when a signal carrying an
event is received, or when a message is heard, the observer gets a
**percept** and the event `perceived` is raised.

```
perceived
  self      the observer
  sense     which sense
  about     a persona, a location, or nothing
  event     the underlying event, if the signal carried one
  where     where it seemed to come from
  clarity   received / threshold
```

Filters (05 §8.4) select by sense, by underlying event type, and by tags of
what it is about:

```json
{ "on": "perceived", "filter": { "event": "element_destroyed", "sense": "sound" },
  "if": "event.clarity > 2 and not self.has_trait('deaf')",
  "do": [ { "enqueue_action": { "entity": "self", "action": "investigate", "params": { "at": "event.where" } } } ] }
```

Two functions give rules access to perception:

| Function | True when |
|----------|-----------|
| `perceives(a, b)` | `a` currently has `b` detected |
| `could_perceive(a, loc, kind)` | A signal of that kind at that place would reach `a` now |

Window kinds (03 §7.1) use `perceives(self, source)` as their `noticed_if`.

## 7. Beliefs

```rust
pub struct Beliefs {
    tracks: BTreeMap<PersonaId, Track>,
    memories: Vec<Memory>,
    map: Option<MapMemory>,
}
```

### 7.1 What is believed

| Kind | Holds |
|------|-------|
| **Track** | What this mind knows about one persona: where and when last perceived, what they were doing, how it knows (saw, heard, was told, inferred), and a confidence. A track may also hold **facts**: true values of particular properties, recorded by `appraise` (08 §7.5) at the moment of appraisal. |
| **Memory** | Something that happened: the event type, where, when, which personas played which roles, how it knows, confidence, and salience. |
| **Map memory** | What places looked like when last seen. Kept for controlled characters. |

Free-form beliefs are variables on the mind.

Memories are bounded. The least salient are forgotten first. Salience is set
by the rule or event mapping that created the memory and fades with time.

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

### 7.3 What rules may read

A rule or behaviour that decides what a character does must read these
properties, not `x.loc`. The linter flags direct reads of another entity's
state in rules attached to minds, templates and creature definitions.

One deliberate exception: **static geometry is common knowledge.**
Path-finding and movement use the true layout of walls, floors and doors.
Elements tagged `hidden` (a secret door, a trap) are excluded until the
character has perceived them. Dynamic facts are never free: who is where,
what is inside a container, whether a door is locked.

## 8. Identity

### 8.1 Personas

```rust
pub struct Persona {
    pub id: PersonaId,
    pub name: Text,
    pub appearance: Appearance,      // what an observer can go by
    pub tags: TagSet,                // uniform, insignia, role
    pub claims: Vec<GroupId>,        // groups this persona presents as belonging to
}
```

Every character has a true persona, may have others, and **presents** one at
a time. Appearance is derived from the body and what is worn, with whatever
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
   stranger of that appearance.

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

## 9. Relations and groups

### 9.1 Relations

```json
[
  { "type": "relation_kind", "id": "fear",     "range": [0, 1], "default": 0, "fade": 0.02 },
  { "type": "relation_kind", "id": "trust",    "range": [-1, 1], "default": 0 },
  { "type": "relation_kind", "id": "commands", "kind": "flag", "orders": true }
]
```

A relation is a value held by one mind toward a persona or a group.
`self.rel('fear', x)` reads it; verbs set, add and clear it. `fade` moves it
toward its default per minute of local time.

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
orders to which. A group may have a collective, a mind that decides for it
(09 §8.1).

- A member with no relation of its own toward someone uses its group's.
- A group's stance toward a persona also applies through that persona's
  `claims`. Wearing the uniform gets you treated as one of them, until
  someone sees through it.
- Group knowledge is how "the watch is looking for you" works. Members add
  to it by reporting and consult it when they recognise someone.

## 10. Orders

```
order { from, to, plan }     hook order_received (may refuse or alter)
                             → the plan is enqueued on `to` (03 §5.3)
```

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

## 14. Level of detail

| Level | Perception |
|-------|------------|
| Hot | As specified |
| Warm | Does not sweep. Receives only emitted signals strong enough to exceed a multiple of threshold; receiving one promotes the entity (03 §10) |
| Cold | None. Beliefs are stored with the agent. The cold simulation moves knowledge between agents and groups at its own scale (09). |

## 15. Modding surface

**Definition types**: `signal_kind`, `sense`, `event_signal`,
`relation_kind`, `group`, `lines`, `message_kind`. Fields `signals` on
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
verbs of section 8.3.

**Queries**: `tracks`, `memories`, `observers` (who currently perceives a
subject), `members`.

**Functions and properties**: `perceives`, `could_perceive`, `light(loc)`;
on minds, the properties of section 7.2 and `rel`, `in_group`, `role_in`,
`persona`, `presenting`, `facing`.

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
| Light at a cell, cached | under 30 ns |
| One observer-target evaluation, including line of sight at 30 m | under 2 µs |
| Emitting a signal to 50 observers in range | under 100 µs |
| One simulated second of perception for 300 hot characters in a crowd | under 4 ms |
| Memory per mind, typical | under 4 KB |

## 18. Open questions

1. **Who keeps map memory.** Only controlled characters do. If AI
   characters should get lost in unfamiliar places, they need it too, at a
   cost. (09)
2. **Appearance detail.** How many descriptors an appearance has, and how
   partial matches work ("a tall figure in red").
3. **Inference.** A character concluding something it did not perceive
   ("the door is open, so someone came through") is left to rules. Whether
   to provide helpers.
4. **Light colour.** Intensity only for now.
