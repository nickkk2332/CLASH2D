# 03 — Time, Clocks and Actions

Status: design complete, ready for implementation.
Implementation plan: [plans/sim-kernel.md](plans/sim-kernel.md).
Serves scenes S3, S4, S5, S6, S10, S14, S16 and S19 of [00-vision.md](00-vision.md).

## 1. Scope

This document specifies how time advances and how anything gets done in it:
the timeline and clocks (`clash-time`), the simulation kernel that owns
`advance` (`clash-sim`), control and decisions, actions, reaction windows,
joint actions, timers, and stepping at lower detail.

It does not specify what actions *do*. Striking, moving, casting and talking
are effects (05) defined by content and by documents 06 to 08. It does not
specify AI (09), perception (15) or how any of this is drawn (12).

Code blocks are sketches. Names are normative; signatures may be adjusted by
the implementing work package if this document is updated in the same change.

Attribute names in examples (`self.attr.quickness`) are illustrative. The
engine has no built-in attributes (01 R10).

## 2. The picture

There are no turns. There is one **timeline**: a queue of wake-ups ordered by
time. Everything that will happen is in it. The simulation pops the earliest
wake-up, runs it, delivers the events it caused, and repeats, until a
character the player controls needs a decision.

A wake-up belongs to a **clock**. Clocks form a tree and each has a rate.
Stop a clock and everything on it hangs where it is. This is the whole of
time stop, slow fields, hasted people and rooms where a year passes in a day.

Characters do things through **actions**. An action is a small state machine
of **phases**, each with a duration: wind up, impact, recover. An action
occupies **channels** such as an arm or the voice, so a character can walk
and shoot, or sustain a spell while fighting, and loses options when a limb
is lost.

When an action announces itself, it opens a **window**. Others who notice it
in time may **react** with actions of their own. Whether a player is asked,
answers automatically, or never gets the chance is decided by rules written
in data, and by whether they were fast enough to notice at all.

A character may hold a **plan**: a queue of things to do next. The player is
asked for a decision only when the plan runs out or a rule interrupts it.

## 3. Timeline and clocks

Crate `clash-time`. Depends only on `clash-core`.

### 3.1 Types

```rust
pub struct Tick(pub i64);      // universal time, microseconds
pub struct Dur(pub i64);       // a span of local time on some clock, microseconds
pub struct ClockId(u32);       // ClockId::UNIVERSAL is the root; ids are never reused
pub struct WakeId(u64);        // stable handle; survives rescheduling; it is the sequence number

pub struct Wake {
    pub domain: WakeDomain,    // who handles it: World, Action, Timer, Rule, Stepper, …
    pub id: u64,
    pub arg: u64,
}

impl Timeline {
    pub fn now(&self) -> Tick;
    pub fn local_now(&self, clock: ClockId) -> Result<Dur, TimeError>;

    pub fn schedule(&mut self, clock: ClockId, after: Dur, wake: Wake) -> Result<WakeId, TimeError>;
    pub fn cancel(&mut self, id: WakeId) -> bool;
    pub fn remaining(&self, id: WakeId) -> Option<Dur>;         // local time left, rounded up
    pub fn rehome(&mut self, id: WakeId, to: ClockId) -> Result<(), TimeError>;   // keeps remaining local time

    pub fn peek(&self) -> Option<Tick>;
    pub fn pop(&mut self, limit: Tick) -> Option<(WakeId, Wake)>;   // advances now to the wake-up's due tick
    pub fn advance_to(&mut self, tick: Tick) -> Tick;               // advances now; never passes a queued wake-up

    pub fn create_clock(&mut self, parent: ClockId, rate: Real) -> Result<ClockId, TimeError>;
    pub fn set_rate(&mut self, clock: ClockId, rate: Real) -> Result<(), TimeError>;
    pub fn reparent(&mut self, clock: ClockId, parent: ClockId) -> Result<(), TimeError>;
    pub fn destroy_clock(&mut self, clock: ClockId) -> Result<(), TimeError>;   // its wake-ups and child clocks go to the parent
}
```

The timeline never panics on what it is given. An unknown clock or wake-up,
a rate that is negative or not finite, an attempt to change, move or destroy
the universal clock, and a move that would put a clock under itself are
refused with a `TimeError` that says which, and change nothing.

`pop` returns `None` and leaves `now` where it is when nothing is due by the
limit. The kernel then calls `advance_to(limit)` to finish an advance to a
tick. An advance with no limit must not move time when the queue is empty,
which is why `pop` does not do it.

Read-only queries complete the interface: a clock's `parent`, `rate` and
`effective_rate`; a wake-up's payload, clock and due tick, and whether it is
parked; the clocks, and the wake-ups on a clock, in order.

### 3.2 Ordering

Wake-ups pop in order of `(universal due tick, sequence number)`. The
sequence number is assigned at scheduling and never changes, so two wake-ups
due at the same tick always run in the order they were scheduled, across
saves and rate changes.

### 3.3 Clocks

- A clock has a parent and a rate ≥ 0. Its **effective rate** is the product
  of rates up to the root. The universal clock's rate is always 1.
- A clock keeps its own local time, which starts at zero when the clock is
  created and advances at its effective rate.
- `schedule(clock, after, …)` means "when `after` more local time has passed
  on this clock". The universal due tick is
  `now + ceil(after / effective_rate)`.
- When a clock's rate or parent changes, the clock and all its descendants
  first bring their local time up to date at the old rate, then every
  wake-up on them gets a new universal due tick from its remaining local
  time. Relative order on one clock is preserved.
- Setting the rate a clock already has, or the parent it already has,
  changes nothing.
- At effective rate 0 a clock's wake-ups are **parked**: they are not in the
  queue and never fire. Raising the rate re-queues them with their remaining
  local time intact. A wake-up with no local time left is due now whatever
  the rate.
- Destroying a clock moves its wake-ups to its parent, each keeping its
  remaining local time, and makes its child clocks children of its parent,
  each keeping its own rate and local time.
- A wake-up's remaining local time is kept exactly, as a real number of
  microseconds. Only its universal due tick is rounded, up to a whole tick,
  and it is worked out afresh at each change. So a wake-up fires within one
  tick of the exact moment however many times its clocks change. A clock
  keeps the fraction of a microsecond of its local time for the same reason:
  bringing it up to date never loses time.

### 3.4 Which clock a thing is on

| Thing | Clock |
|-------|-------|
| Realm | Its own, child of universal, at the realm's `time_rate` |
| Area with a clock | Its own, child of the realm clock unless created under another area's clock |
| A location | The clock of the highest-priority area containing it that has a clock (ties: lowest area id), else the realm clock |
| Entity without a personal clock | The clock of its location |
| Entity with a personal clock | Its own, child of its location's clock, or of its **anchor** if it has one |
| Body in flight, world process | The clock of its location |
| A contained entity | As its outermost container, unless it has a personal clock |
| An entity that is not in the world | Its personal clock if it has one, else none. Nothing can be scheduled for it (invariant T9) |

When an entity, body or process moves to a location with a different clock,
its wake-ups are rehomed. Movement goes through the single relocation
function (01 R3), which does this.

An **anchor** pins an entity's personal clock to a chosen parent regardless
of where the entity is. Anchoring to the realm clock is how someone keeps
moving inside a bubble of stopped time.

As built (decision D-363):

- The kernel does not store which clock an entity is on. It works the
  answer out from where the entity is, every time, and stores only which
  wake-ups are an entity's. Whatever schedules for an entity does it
  through `Sim::schedule_for`, so that the wake-up follows the entity from
  clock to clock; one put on the timeline directly stays where it was put.
- A contained entity without a personal clock is on the clock its
  outermost container is on, which is that container's personal clock if
  it has one. What a hasted courier carries is hasted with them.
- An area that is made, moved, reshaped or removed over an entity rehomes
  it exactly as a move does: the kernel follows the world's `area_entered`
  and `area_exited`. `set_clock_rate` on an area that gains or loses its
  clock puts whoever is inside on the clock that now applies. A change of
  an area's priority is not reported by the world, so it takes effect for
  an entity the next time one of those happens.
- Anchoring an entity that has no personal clock gives it one at rate 1.
  Taking the personal clock away (`set_time_rate` with no rate) lifts the
  anchor too. An anchor whose clock has been destroyed counts as none.
- An entity has **own time**: the local time that has passed for it, on
  whatever clocks it has been on, while it was in the world. It is what
  `DecisionRequest.elapsed` is measured in. Time spent on an area's clock
  that was destroyed by removing the area, rather than through
  `set_clock_rate`, is counted only up to the entity's last change of
  clock.

### 3.5 Two kinds of fast

The engine offers two independent ways to be quick, and content chooses:

| Mechanism | What it does | Fits |
|-----------|--------------|------|
| **Tempo** (section 6.5) | Divides the duration of an action's phases | Skill, training, a quick fighter |
| **Personal clock rate** | Speeds up everything about the entity: actions, timers, bleeding, cooldowns, how fast poison works | Speedsters, haste, slow curses, bullet time |

A character a thousand times faster than a person takes a third of a
millisecond over a punch that takes someone else a third of a second.
Nothing else is needed for scene S3: the pressure wave is on the realm
clock, the speedster is quicker than it.

### 3.6 Processes

A process (01 §5.1) is plain state that the timeline wakes. Each wake it
does some work and says what comes next.

```rust
pub struct ProcessId(u32);     // never reused within one table

pub enum Next {
    Sleep(Dur),                // wake me after this much local time on my clock
    Done,                      // I have finished; forget me
}

impl<P> ProcessTable<P> {      // P is the owning crate's process type, usually an enum
    pub fn new(domain: WakeDomain) -> Self;
    pub fn spawn(&mut self, timeline: &mut Timeline, clock: ClockId, first_wake: Dur, process: P)
        -> Result<ProcessId, TimeError>;
    pub fn process_of(&self, wake: &Wake) -> Option<ProcessId>;   // which process a popped wake-up is for

    // Run in place, when the work needs nothing that holds the table.
    pub fn wake(&mut self, timeline: &mut Timeline, id: ProcessId, step: impl FnOnce(&mut P) -> Next)
        -> Result<Option<Next>, TimeError>;

    // Or take the state out, run it against the state that holds the table, and hand it back.
    pub fn take(&mut self, timeline: &mut Timeline, id: ProcessId) -> Option<P>;
    pub fn settle(&mut self, timeline: &mut Timeline, id: ProcessId, process: P, next: Next)
        -> Result<(), TimeError>;

    pub fn remove(&mut self, timeline: &mut Timeline, id: ProcessId) -> Option<P>;   // force-resolve
    pub fn rehome(&mut self, timeline: &mut Timeline, id: ProcessId, to: ClockId) -> Result<(), TimeError>;
}
```

- The table keeps exactly one wake-up scheduled for each process, carrying
  the table's domain and the process's id. It is saved and hashed with the
  rest of the state.
- `remove` is how a process is **force-resolved** (02 §10.3): its wake-up
  is cancelled and its state is returned, for its owner to finish at once.
- A process runs on the clock it was spawned on until `rehome` moves it.
  The owner rehomes the processes of a clock before destroying the clock.
  One that is missed still wakes, because the timeline moves its wake-up to
  the parent clock, but it cannot be put back to sleep on a clock that is
  gone: the table reports the error and leaves it unscheduled, for the
  owner to rehome or remove.

## 4. The simulation kernel

Crate `clash-sim`. The kernel is the part of the simulation that every other
document builds on.

### 4.1 State

```rust
pub struct Sim {
    defs: Arc<Defs>,
    cosmos: Cosmos,                  // 02
    timeline: Timeline,
    entities: EntityTable,
    pub actions: ActionTable,
    pub windows: WindowTable,
    pub rules: RuleState,            // 05: attachments, continuations, cooldowns
    settings: VarMap,                // 04 §10
    pub rng: RngStreams,
    events: EventQueue,              // 05 §8
    decisions: DecisionQueue,
    requests: VecDeque<AppRequest>,  // 13 §9.1
    out: Vec<SimEvent>,              // for presentation
    log: InputLog,
}
```

As built (decisions D-360, D-369):

- The world, the timeline and the entity table are private, because
  changing them behind the kernel's back breaks what it keeps true. They
  are read through `Sim::truth()`, which gives a `Truth`: the one type
  through which ground truth is read (01 R11). Nothing a character decides
  by may hold one. `Sim::timeline()` and `Sim::now()` read time.
- Simulation code changes the world with `Sim::world(|ctx| ...)`, which
  hands out the world's `WorldCtx` and, when the closure returns, passes
  on what the world reported and brings the kernel into step with it.
- `Sim::new(defs, seed)` makes an empty world. `Sim::from_parts(defs,
  cosmos, timeline, seed)` takes a world that has already been made, which
  is how a generated world is handed over.
- The simulation also keeps which entity each of its wake-ups belongs to
  and how many wake-ups it has handled. The handlers of wake-up domains
  and of events are not state: they are set up with the simulation and
  again after a load.
- `Sim::state_hash()` is the total of `Sim::hashes()`, which gives one hash
  for each of: the world, the timeline, the entities, actions with windows
  and rule state, settings, the random streams, and the kernel's own
  queues. The output waiting to be drained and the input log are records
  of what has happened, not state that decides what will: they are saved
  and not hashed.
- `Sim::save()` writes everything but the definitions and the world, which
  have stores of their own, as one blob. `Sim::load(defs, cosmos, blob)`
  reads it back over the world it was saved with, and `Sim::reindex()`
  then places every entity in the world's occupant index again, which the
  world neither saves nor hashes (02 §8.1).

### 4.2 Kernel entities

Document 06 specifies entities in full. The kernel fixes the part that time,
actions and rules depend on, and document 06 extends it without changing it.

```rust
pub struct KernelEntity {
    pub id: EntityId,
    pub place: Place,                  // At { loc, alt_mm } | In(EntityId) | Stored
    pub tags: TagSet,
    pub vars: VarMap,
    pub control: Control,              // None | Direct | Delegated
    pub clock: Option<ClockId>,        // personal clock
    pub anchor: Option<ClockId>,
    pub channels: ChannelSet,          // section 6.2
    pub actions: SmallVec<[ActionId; 2]>,
    pub plan: Plan,                    // section 5.3
    pub attached: Vec<EocId>,          // 05 §9.3
}
```

`Place` implements containment (01 §6). `Stored` means the entity is not in
the world: it is cold, or saved with an unloaded tract.

`Sim::relocate` is the only function that changes `place`. It updates the
world's occupant index, rehomes wake-ups when the clock changes, and emits
movement, area and portal events.

As built (decisions D-361, D-362):

- The kernel entity also holds a **presence**: what the world is told
  about it (kind, planes, mass, flags, footprint). The world does not save
  occupants, so the kernel keeps this to place the entity again after a
  load. Document 06's `Physical` component writes it with
  `Sim::set_presence`.
- `tags` are strings, kept in order, not the numbers the loader interns: a
  rule may give an entity a tag no definition carries, and a save names
  tags by their strings.
- `place`, `presence`, `control`, `clock` and `anchor` are read through
  accessors and changed only by `relocate`, `set_presence`, `set_control`,
  `set_time_rate` and `anchor_clock`. `place` is private to the module
  that holds `relocate`, and no owned kernel entity ever leaves the crate,
  so nothing else can change it. The rest (`tags`, `vars`, `channels`,
  `actions`, `plan`, `attached`) is open to whoever holds the entity
  mutably until its packages give it owners.
- `actions` is a plain list. `ChannelSet`, `Plan`, the action table, the
  window table and the rule state are empty typed placeholders that
  already serialise and hash.
- Ids are generational. A freed slot is reused, lowest slot first, a
  generation on. The table visits entities in id order.
- `create_entity` makes an entity that is `Stored` and emits
  `entity_created`; `spawn` makes one and puts it somewhere, or makes
  nothing if it cannot be put there. `remove_entity` first puts what the
  entity held where the entity was (what cannot stand there leaves the
  world), then takes the entity out of the world, cancels its wake-ups,
  destroys its personal clock, forgets any decision it was asked for, and
  emits `entity_removed`. A component store follows those two events.

What `relocate` does, in full:

- It refuses, changing nothing and with a reason, an unknown entity or
  container, a container that is the entity itself or inside it, nesting
  deeper than 64, and a cell the world refuses: outside the realm, or
  with any cell of the footprint not loaded. A move to where the entity
  already is does nothing at all.
- Only an entity `At` a cell is in the occupant index, at that cell. What
  is inside it is wherever it is, moves with it, and is not reported
  separately.
- The entity and everything inside it are put on the clocks that now
  apply (section 3.4), each wake-up keeping its remaining local time.
- An entity that leaves the world (`Stored`, or inside something that
  is), and everything inside it, has its wake-ups cancelled (invariant
  T9). Whoever stores an entity takes what should survive, such as
  timers, off the timeline first (section 9).
- It emits `entity_relocated` after the world's own `area_exited` and
  `area_entered`. Portal crossings wait for the world to have portals.
- When the world moves an occupant out of the way of an edit (02 §23),
  the kernel follows: the entity's place becomes where the world put it,
  and `entity_relocated` is emitted after the world's
  `occupant_displaced`.
- After a load, `reindex` rebuilds the occupant index from the places. An
  entity that cannot be placed where its place says leaves the world,
  with `entity_relocated`, and is listed in what `reindex` returns.

### 4.3 Advance

```rust
pub enum AdvanceLimit {
    UntilDecision,
    Until(Tick),
    Work(u32),                 // at most this many wake-ups; lets a frontend stay responsive
}

pub enum Advance {
    NeedInput(DecisionRequest),
    Reached(Tick),
    Yielded,                   // work budget spent; call again
    Request(AppRequest),       // a rule asked for a checkpoint, a restore or an ending (13 §9.1)
}
```

One iteration of `advance`:

1. If a rule has asked the application for something, return `Request`.
2. Pop the next wake-up within the limit. While a decision is pending for
   a directly controlled entity, only what is still due at the current
   tick is popped. If there is none, return: `NeedInput` if a decision is
   pending, else `Reached`.
3. Dispatch it to its domain's handler.
4. **Pump events.** Drain world events into the event queue, then deliver
   queued events in order to rules (05 §8). Rules may change state and emit
   more events; those join the back of the queue. Continue until the queue
   is empty. Then re-evaluate reactive expressions that have gone stale
   (05 §7.5), and pump again if that raised anything.
5. **Safe point.** If flagged: reconcile tickets (02 §10.2), run requested
   saves, apply a pending definition reload (04 §11).

Everything between two pops happens at one tick.

If one pump delivers more than `rules.max_cascade` events (default 10,000),
the remainder is dropped, the offending rule chain is reported with its
trace, and the simulation carries on. Content cannot hang the engine.

As built (decision D-364):

- **Limits.** `UntilDecision` runs until a decision is needed or nothing
  is left to happen, and does not move time when nothing is scheduled.
  `Until(tick)` also moves time on to the tick. `Work(n)` handles at most
  `n` wake-ups and returns `Yielded` if more is due, else `Reached`. A run
  made of many `Work(n)` calls handles the same wake-ups in the same order
  as one long call. `Reached` carries the current time.
- **Domain handlers.** A wake-up's domain picks its handler: a plain
  function given the simulation, the wake-up's handle and what it carried.
  A package that schedules wake-ups adds its handler to the defaults every
  simulation starts with; a test or a tool registers one with
  `Sim::set_wake_handler`. The world's wake-ups (the `World` domain) go
  through the same loop: their handler hands the wake-up to the world
  through `Sim::world`. A wake-up whose domain has no handler is dropped
  and reported as a fault.
- **Faults.** What the engine contains (an unhandled wake-up, a limit
  exceeded) is reported as a `fault` event that goes straight to the
  output and is not delivered to handlers.
- **One tick.** If one call handles 100,000 wake-ups without time moving,
  it reports a fault and returns `Yielded`. Putting the rest off to the
  next tick (05 §13.2) is not built.
- **The pump** also runs when a command is accepted, so events are never
  left waiting between calls. A cut-off cascade's report lists the last
  eight events delivered.
- **The safe point** calls `reconcile_activity`, which does nothing until
  tickets and activity (02 §10.2) are joined to the kernel. Requested
  saves are returned to the application as `Advance::Request`.

### 4.4 Commands

```rust
pub enum Command {
    Act     { entity: EntityId, action: ActionDefId, params: Params, how: Start | Enqueue },
    Wait    { entity: EntityId, dur: Dur },
    Cancel  { entity: EntityId, action: ActionId },
    ClearPlan { entity: EntityId },
    React   { entity: EntityId, window: WindowId, action: ActionDefId, params: Params },
    Pass    { entity: EntityId, window: WindowId },
    Choose  { entity: EntityId, choice: ChoiceId, option: u32 },
    Setting { key: VarKey, value: Value },
    Ui      { rule: EocId, args: Params },     // a rule marked ui_callable, run from a screen (12 §10.5)
    Debug(DebugCommand),
}
```

- `submit` validates a command against current state and either applies it
  or returns a `CommandError` with a reason that can be shown to the player.
- Commands may address entities under `Direct` or `Delegated` control.
- Every accepted command is appended to the **input log** with its tick. A
  save plus its input log replays to the same state.
- Commands are the only way anything outside the simulation changes it.

As built (decision D-365):

- `Command` and `DebugCommand` are open sets. The kernel honours the debug
  commands: `CreateEntity { spec, place }`, `MoveEntity { entity, to }`,
  `RemoveEntity { entity }`, `SetElement { loc, layer, state }`,
  `SetControl { entity, control }` and `Schedule { entity, after, wake }`.
  The rest of the list above arrives with the packages that can honour
  each. A debug command is a command like any other: validated, logged
  and replayed.
- A `CommandError` holds a `Reason` (14 §5): a stable code, a sentence and
  the values involved. A refused command leaves the state exactly as it
  was and is not logged.
- A log entry carries the tick and the number of wake-ups the simulation
  had handled when the command was accepted. The two together fix the
  command's place among the wake-ups of its tick, so a session that
  yielded in the middle of a tick replays exactly.
- `Sim::replay(log)` advances to each entry's place and submits its
  command. Run on a simulation set up as the logged one was, it ends in
  the state the session was in when its last command was accepted. An
  entry that cannot be reached or is refused stops the replay with a
  reason that names it. `take_input_log` starts a new log, as a save does.
  The log does not yet carry state hashes at intervals (14 §8.1).

## 5. Control, decisions and plans

### 5.1 Control

| Mode | Meaning |
|------|---------|
| `None` | The AI layer decides (09). |
| `Direct` | The player decides. The simulation stops and asks. |
| `Delegated` | The AI layer decides, following orders the player may give through commands. Party members on autopilot, minions, summons. |

Any number of entities may be `Direct`, including none. Control is changed
by effects, so scenarios and abilities decide who the player is: a party, a
possessed body, a different character each chapter.

### 5.2 Decisions

```rust
pub struct DecisionRequest {
    pub entity: EntityId,
    pub kind: DecisionKind,
    pub elapsed: Dur,                 // local time since this entity last decided
}

pub enum DecisionKind {
    Free { why: FreeReason },         // Started | Finished(ActionId) | Interrupted(Cause) | PlanEmpty
    Reaction { window: WindowId, deadline: Tick, options: Vec<ReactionOption> },
    Choice { choice: ChoiceId, prompt: Text, options: Vec<ChoiceOption> },   // from the `ask` effect, 05 §10
}
```

A directly controlled entity needs a **free decision** when a foreground
action of its own ends and its plan is empty, or when a rule interrupts it.
It does not need one merely because a channel is idle.

If several entities need decisions at the same tick, requests are returned
one at a time in entity id order, and time does not move until each has been
answered. `Wait` is an answer.

As built (decision D-364). The kernel owns the queue and its order, and
packages T4, T5 and K9 add what raises requests and the commands that
answer them:

- `Sim::request_decision(entity, kind, cause)` asks an entity under
  `Direct` control and emits `decision_requested`. For any other entity it
  asks nothing and says so, and the caller turns to the AI layer.
  `Sim::answer_decision(entity)` takes the entity's oldest request as
  answered; the commands that answer call it.
- So that every request raised at one tick is there to be returned in
  entity order, `advance` runs what is still due at the current tick
  before it returns `NeedInput`. Time does not move.
- `elapsed` is the entity's own time (section 3.4) since it last answered.
- Of the kinds, only `Free` exists yet. `PlanEmpty` carries the reason a
  step could not be started, if that is why. An entity that stops being
  directly controlled, or is removed, is no longer waiting for anything.

### 5.3 Plans

```rust
pub struct Plan { steps: VecDeque<PlanStep> }
pub struct PlanStep { action: ActionDefId, params: Params }
```

`Act { how: Enqueue }` appends a step. When a foreground action ends, the
next step whose channels are free starts automatically. If a step's
`requires` fails when it is due, the plan is cleared and a free decision is
requested with the reason. A step whose channels are busy waits for them.

The AI layer uses plans in the same way. Travelling along a path, repeating
an attack, crafting for an hour and following an order are all plans.

Plans are interrupted by the `interrupt_plan` effect. What interrupts them (a
new threat in view, being hurt, a noise) is decided by rules, not by the
engine.

## 6. Actions

### 6.1 Definition

```json
{
  "type": "action",
  "id": "heavy_punch",
  "name": "Heavy punch",
  "uses": { "arm": 1 },
  "params": {
    "target": { "type": "entity" },
    "part":   { "type": "part", "default": "auto_part($target)" }
  },
  "requires": "dist(self, $target) <= 1.5",
  "tempo": "self.attr.quickness",
  "phases": [
    { "id": "windup", "time": "300 ms", "cancel": true,
      "window": { "kind": "strike", "for": ["$target"], "closes": "impact" } },
    { "id": "impact", "time": 0,
      "do": [ { "strike": { "target": "$target", "part": "$part" } } ] },
    { "id": "recovery", "time": "400 ms" }
  ]
}
```

The `strike` verb belongs to document 07. Here it stands for "whatever this
action does".

| Field | Meaning |
|-------|---------|
| `uses` | Channels occupied for the whole action (section 6.2). |
| `params` | Typed parameters. Each may have a `default`, an expression evaluated when the command omits it (section 6.3). |
| `requires` | Condition that must hold to start. `fail_text` gives the reason shown when it does not. |
| `requires_at_start` | A condition tested only when the action begins, and never shown beforehand. If it fails the action is interrupted at once and says why. For things the actor cannot know in advance (01 R11): whether a victim has noticed them. |
| `tempo` | Expression dividing phase durations (section 6.5). Default 1. |
| `foreground` | Whether finishing this action can prompt a decision. Default true. A sustained aura or a conversation is background. |
| `footing` | Whether the actor must be standing on something. Default true. An action without it can be done in the air (07 §7.1). |
| `reaction_to` | Window kinds this action may answer (section 7). |
| `on_behalf` | Whether, as a reaction, it may answer a window aimed at someone else. Default false. |
| `phases` | Section 6.4. |
| `on_start`, `on_finish`, `on_cancel`, `on_interrupt` | Effect lists. |
| `roles` | Present on joint actions (section 8). |

### 6.2 Channels

A **channel** is something an action needs exclusive use of.

```json
{ "type": "channel", "id": "arm" }
```

An entity has a set of channel **slots**. Document 06 derives them from the
body: two arms give two `arm` slots, a mouth gives `voice`, a mind gives
`focus`. A creature with six arms has six. A device has whatever its
definition gives it.

- `"uses": { "arm": 2, "legs": 1 }` needs two free arm slots and one legs
  slot. `{ "arm": "all" }` needs every arm slot the entity has, and at least
  one.
- A parameter of type `part` may name which slot is used; otherwise any free
  one is taken.
- Actions whose slots do not overlap run at the same time.
- A slot can be **disabled** (the limb is broken, bound or gone). Disabling a
  slot interrupts any action using it. This is half of scene S14: with the
  sword arm out, every action that needs it is unavailable.

### 6.3 Parameters and defaults

Every parameter that has a sensible automatic choice declares a `default`.
A command that supplies only the action and a target is complete; one that
names the technique, the limb and the exact spot is equally valid. This is
the engine side of decision D-037.

Defaults are expressions and may call functions (05 §12), so the choice of
target or body part can be as clever as content wants.

A parameter may be a **list** of another type, with a least and a greatest
length. A list of cells is a route:

```json
"route": { "type": "list", "of": "cell", "max": 24, "check": "route_fault(self, $route)" }
```

`check` is an expression that gives a reason when the value is not sound,
or nothing when it is. It is advice: pickers show it as the value is built
(12 §9.3). Whether an unsound value is refused is for `requires`, and what
happens to someone who goes ahead anyway is for the action's own effects.

### 6.4 Phases

```json
{ "id": "charge", "time": "hold", "max": "3 s",
  "tick": { "every": "250 ms", "do": [ { "set": { "$power": "$power + 1" } } ] },
  "until": "$power >= 10",
  "cancel": true,
  "next": [ { "if": "$power >= 4", "goto": "release" }, { "goto": "fizzle" } ] }
```

| Field | Meaning |
|-------|---------|
| `time` | Duration expression, or `"hold"`. Evaluated when the phase starts. `0` runs and moves on within the same tick. |
| `do` | Effects run when the phase starts. |
| `on_exit` | Effects run when the phase ends normally. |
| `tick` | Effects repeated at an interval while the phase lasts. |
| `until`, `max` | For held phases: end when the condition is true (checked at ticks) or the maximum is reached. A `Cancel` command also ends a held phase and counts as release, not as cancellation. `Cancel` with `abandon` gives it up instead, where the phase allows `cancel`. |
| `cancel` | Whether the actor may abandon the action during this phase. Default false. A list of action tags instead of `true` allows abandoning it only to start an action with one of those tags: a recovery that can be cut short by a follow-up and nothing else. |
| `uninterruptible` | The `interrupt` effect cannot end the action during this phase. |
| `fixed` | Tempo does not apply to this phase. |
| `window` | Opens a reaction window when the phase starts (section 7). |
| `tags` | Free-form, queryable by rules: `vulnerable`, `telegraph`, `airborne`. |
| `next` | Where to go afterwards: the first entry whose condition holds. Default is the following phase; after the last, the action finishes. |

`next` makes an action a small state machine: follow-ups that branch on
whether a blow landed, charge-and-release, loops that repeat while a
condition holds. More than 1,000 zero-duration transitions in one tick ends
the action with an error.

Action instances have local variables (`$name`), shared by all their phases
and effects.

### 6.5 Tempo

`duration = time / tempo`, with `tempo` evaluated at the start of each
phase. Once a phase is scheduled, a later change of tempo does not move it.
A change of clock rate does.

### 6.6 Lifecycle

```
            requires ok                 last phase ends
 command ──────────────► running ───────────────────────► finished
                          │  │
            Cancel (if    │  │ interrupt effect, or a
            phase allows) │  │ channel slot is disabled
                          ▼  ▼
                    cancelled  interrupted
```

- **Start**: evaluate parameter defaults, check `requires`, claim channel
  slots, run `on_start`, enter the first phase.
- **Finish**: run `on_finish`, release slots.
- **Cancel**: voluntary. Runs `on_cancel`, releases slots.
- **Interrupt**: involuntary. Runs `on_interrupt`, releases slots.

Each transition emits an event. A phase that has been scheduled is one
wake-up on the actor's clock.

Moving is an action like any other. Its effect calls relocation. A dash that
covers many cells is one action whose effect sweeps the path; document 06
defines it.

## 7. Windows and reactions

### 7.1 Windows

A **window** is a published opportunity with a deadline: something is about
to happen, and those who notice may respond before it does.

```json
{ "type": "window_kind", "id": "strike",
  "notice": "self.attr.reaction_time",
  "noticed_if": "true",
  "reach": 6 }
```

```rust
pub struct Window {
    pub id: WindowId,
    pub kind: WindowKindId,
    pub source: EntityId,
    pub action: ActionId,
    pub candidates: Vec<EntityId>,
    pub opened: Tick,
    pub closes: WakeId,            // the phase transition that ends it
    pub vars: VarMap,
}
```

A phase's `window` field opens one when the phase starts. `for` names
those it is aimed at. `closes` names the later phase whose start is the
deadline. The `open_window` effect opens one from any rule; having no
phase to close it, it states how long it stays open.

**Candidates.** A window is offered to:

- those it names;
- anyone who holds, toward one of those, a relation whose kind is flagged
  `intervenes` (15 §9.1): a bodyguard, a sworn protector, a parent;
- any directly controlled character.

The second and third are candidates only within the window kind's `reach`
of the source or of someone it names. All candidates then go through
noticing (section 7.2). So an attack on someone else can be answered by
someone who cares and is close enough and quick enough to step in.

### 7.2 Noticing

For each candidate, when the window opens:

1. Evaluate the kind's `noticed_if` with `self` as the candidate. If false,
   the candidate never learns of it. Document 15 supplies real perception
   tests; until then content can write its own.
2. Evaluate `notice`, a duration. Schedule a wake-up that far ahead **on the
   candidate's clock**.
3. If that wake-up fires before the window closes, emit `WindowNoticed`.
   If the window closes first, it is cancelled and the candidate had no
   chance.

So an ordinary person with a quarter-second reaction time can answer a
telegraphed swing and cannot answer a blow that lands a millisecond after it
starts. Someone whose clock runs a thousand times faster notices almost
everything. The gap in decision D-034 is in the arithmetic.

### 7.3 Reacting

The engine does nothing by itself when a window is noticed. What happens is
decided by rules (05) listening for `WindowNoticed`:

| Effect | Result |
|--------|--------|
| `react` | Start a reaction action immediately, with the window bound as `$window`. |
| `ask_reaction` | For a directly controlled entity: stop and return a `Reaction` decision listing its options. |
| nothing | The candidate does not respond. |

The bundled content supplies default rules: when to ask the player, when to
answer automatically from the character's stance, when to ignore. Players
tune them through settings; mods replace them. This is decision D-035.

A **reaction** is an ordinary action whose definition lists the window kind
in `reaction_to`. To start, it needs free channel slots like any other. If
the reactor's current actions hold the slots it needs and are in phases that
allow `cancel`, they are cancelled to make room; otherwise the reaction is
unavailable. Someone caught in recovery cannot block.

A reaction definition may name a `ready` phase. Each `ReactionOption` given
to the player states whether that phase would begin before the deadline, so
a block that would arrive too late is shown as such. It may still be chosen.

What a reaction achieves is not special-cased. A dodge moves the reactor, so
the blow finds nobody there. A block puts the reactor in a phase that the
strike's effects check for. A counter lands its own impact first and
interrupts the attacker. Document 07 builds the rules of combat from these.

## 8. Joint actions

A **joint action** is one action instance with several participants.

```json
{
  "type": "action",
  "id": "takedown",
  "roles": {
    "attacker": { "uses": { "arm": "all" } },
    "victim":   { "uses": { "arm": "all", "legs": "all" }, "forced": true }
  },
  "requires_at_start": "not perceives($victim, $attacker)",
  "phases": [
    { "id": "seize",  "time": "200 ms" },
    { "id": "finish", "time": "600 ms", "do": [ { "run": "takedown_result" } ] },
    { "id": "settle", "time": "300 ms" }
  ]
}
```

`perceives` is supplied by document 15 and `takedown_result` by content.

- Each role claims slots on its own participant. Phases run once, on the
  clock of the first role.
- A role marked `forced` is entered without that participant's consent. The
  rules that start the action decide whether that is allowed; typically a
  contest that the initiator has just won.
- A role without `forced` requires the participant to accept: a `Reaction`
  decision for players, a rule for everyone else.
- Interrupting or disabling any participant ends the action for all.
- All participants are locked together for its duration, which is what lets
  presentation show it as one choreographed sequence (scene S6).
- Each role is bound as a subject under its name (`$attacker`, `$victim`)
  in every expression and effect of the action. `self` is whoever started
  it. A rule the action calls is passed them with `with`.
- A role that is not forced is **offered**: `RoleOffered` is raised for
  that participant, and `accept_role` or `decline_role` answers it. A
  rule answers for characters the player does not control. An offer not
  answered by the end of the action's first phase is declined.

Joint actions are also how grapples, carrying someone, mounting, rituals with
several casters and handshakes work.

## 9. Timers and unloading

Anything that happens later and is not an action phase is a **timer**: a
status wearing off, a cooldown, a delayed or repeating effect (05 §10.4).

```rust
pub struct Timer {
    pub owner: Owner,              // an entity, area, site, realm, or global
    pub wake: WakeId,
    pub then: Continuation,        // 05 §13.4
    pub while_unloaded: Unloaded,  // Pause | Elapse
}
```

Timers run on their owner's clock.

When an owner leaves the simulation (its tract unloads, or it becomes a cold
agent):

- Its running actions end without effects and emit `ActionAbandoned`. Its
  plan is handed to the cold simulation (09).
- Its timers are stored with their remaining local time.
- When it returns, `Pause` timers resume as they were. `Elapse` timers have
  the local time that passed subtracted; any now overdue fire once,
  immediately, in order.

## 10. Stepping at lower detail

| Level | How entities are stepped |
|-------|--------------------------|
| Hot | Individually: actions, windows and timers on the timeline as specified above. |
| Warm | In bulk. One stepper process per realm wakes every `WARM_STEP` (1 s local) and hands the AI layer each warm entity to advance by that interval. Warm entities finish actions already running but start no new ones and receive no windows. |
| Cold | Not in the entity table. Stepped as agents and populations by region, at intervals of minutes, by the cold simulation (09). |

An entity's level is its tract's activity level (02 §10), lowered to warm
for characters beyond the attention cap (09 §9.3). Giving an entity a hot
ticket of its own promotes it. The kernel promotes a warm entity
automatically when it is named as a participant, target or window candidate
of a hot entity's action.

## 11. Long waits

Sleeping, travelling and waiting are plans or held actions. Time passes as
fast as wake-ups can be processed, and `AdvanceLimit::Work` keeps the
frontend responsive while it does.

An action may be flagged `low_attention`. While every directly controlled
entity is in such an action, the simulation may lower their tickets from hot
to warm, so eight hours of sleep does not cost eight hours of full detail
for the neighbourhood. Any interruption restores them.

## 12. What presentation is given

- Every `SimEvent` carries the universal tick, the local time of the clock
  it happened on, its subjects and its cause.
- `DecisionRequest.elapsed` says how much of the entity's own time passed
  since it last chose.
- Phase events carry start and scheduled end, so motion can be interpolated.

As built: a `SimEvent` is a stamp and a body. The stamp is the universal
tick, the clock the event happened on with that clock's local time, and
the cause. The clock is the entity's for an event about an entity, the
location's for one about a place, and universal time otherwise. The body
says what happened and names its subjects in order; `actor` and
`instigator` are added from the cause when it names them.

How a thousand actions between two decisions are shown (played, blurred or
summarised) is document 12's concern, constrained by what the viewing
character could perceive (15).

## 13. Modding surface

**Definition types**: `action`, `channel`, `window_kind`.

**Events**: `ActionStarted`, `PhaseEntered`, `PhaseExited`, `ActionFinished`,
`ActionCancelled`, `ActionInterrupted`, `ActionAbandoned`, `PlanCleared`,
`ActionRefused` (who, which action, and the reason: a failed `requires`,
busy channels, a veto), `RoleOffered`,
`WindowOpened`, `WindowNoticed`, `WindowClosed`, `DecisionRequested`,
`ControlChanged`, `ClockRateChanged`, `EntityRelocated`,
`EntityPromoted`, `EntityDemoted`.

**Hooks** (events whose handlers may alter or cancel them, 05 §8.2):

| Hook | Runs before | A handler may |
|------|-------------|---------------|
| `ActionStarting` | An action claims its channels and enters its first phase | Change its parameters; cancel, with a reason |
| `PhaseTiming` | A phase is scheduled | Change its duration |

Every event and hook is registered with its subjects and fields
(05 §8.1), and the generated reference lists them (04 §13). Where this
document names one without them, the package that builds it writes them
here.

The kernel's events, as built. Each also binds `actor` and `instigator`
from its cause where the cause names them, and carries what section 12
lists. A subject marked `?` is bound only when there is one.

| Event | Subjects | Fields | Raised when |
|-------|----------|--------|-------------|
| `entity_created` | `self`: entity | none | An entity came to exist. It is not yet anywhere |
| `entity_removed` | `self`: entity | none | An entity ceased to exist. What it held was put where it was first |
| `entity_relocated` | `self`: entity; `here?`: loc | `from`, `to`: a place (a cell and a height, a container, or not in the world) | An entity's place changed. `here` is bound when `to` is a cell. What it holds moves with it and is not reported |
| `control_changed` | `self`: entity | `old`, `new`: `none`, `direct` or `delegated` | Who decides for an entity changed |
| `clock_rate_changed` | `self?`: entity; `area?`: area | `clock`: int; `old`, `new`: real, or null where there was or is no such clock | The rate of a personal, area or realm clock changed, or a clock was given or taken away |
| `decision_requested` | `self`: entity | `kind`: what is asked | A directly controlled entity must choose |
| `fault` | none | `code`, `message`: string | The engine contained a fault. Goes straight to the output; no handler receives it |
| `event_cancelled` | none | `event`, `code`, `message`: string | A hook was cancelled (05 §8.2) |

The world's events (02 §7.4) are passed on under their own names. Their
subjects are: `here` for a cell; for `area_entered` and `area_exited`,
`area`, then `target` (the occupant), then `here`; for `area_breached` and
`area_closed`, `area` and `here`; for `occupant_displaced` and
`occupant_entombed`, `target` and `here`. The fields a filter can test
are `layer` on `element_changed` and `element_damaged`, `old` and `new`
on `element_damaged`, `count` on `decals_removed`, and `key`, `old` and
`new` on `cell_var_changed`.

**Reasons** (14 §5) of the kernel, by code: `entity.unknown`,
`entity.stored`, `entity.ids_exhausted`, `relocate.inside_itself`,
`relocate.too_deep`, `world.not_loaded`, `world.outside_realm`,
`world.unknown_grid`, `world.unknown_area`, `world.unknown_realm`,
`world.refused`, `clock.unknown`, `clock.bad_rate`, `clock.loop`,
`time.refused`, `advance.unhandled_wake`, `advance.tick_limit`,
`events.cascade_limit`, `events.hook_depth`, `events.unknown_type`,
`events.bad_payload`, `events.field_fixed`, `events.cancelled`,
`decision.none_pending`, `replay.diverged`. Each is documented in the
table the generated reference reads.

**Not yet reachable from data.** The operations `set_time_rate`,
`anchor_clock`, `set_clock_rate` and `set_control` are kernel functions;
their effects are bound by the rule runtime. The queries of this section
(`now`, local time and clock rate at a subject, control mode) are not
registered as properties until subjects are. The debug commands are not
yet console commands. A handler of an event can only be written in Rust
until rules exist.

**Settings**: `rules.max_cascade`, `rules.max_transitions` (zero-duration
phase changes in one tick, default 1,000). `WARM_STEP` is a named
constant.

**Further action fields**: `tags` (free-form, queryable), `low_attention`
(section 11), `on_abandon` (effects run when an action ends because its
actor left the simulation, section 9), and `ready` on a reaction
(section 7.3).

**Effects**:

| Effect | Does |
|--------|------|
| `start_action`, `enqueue_action` | Begin or plan an action for an entity |
| `cancel_action`, `interrupt` | End actions, by action, channel or tag |
| `clear_plan`, `interrupt_plan` | Empty a plan; with `interrupt_plan`, also request a decision |
| `disable_channel`, `enable_channel` | Take away or restore slots |
| `open_window`, `close_window` | Publish or withdraw an opportunity |
| `react`, `ask_reaction` | Answer a noticed window |
| `set_control` | Change who decides for an entity |
| `accept_role`, `decline_role` | Answer an offered role in a joint action (section 8) |
| `set_time_rate`, `anchor_clock` | Give an entity a personal clock rate; pin its clock |
| `set_clock_rate` | Change the rate of an area's or realm's clock |
| `wait` | Occupy an entity for a duration |

**Queries** for expressions: an entity's running actions and their phase,
tags and time remaining; whether a channel is free; plan length; control
mode; `now`, local time and clock rate at a subject; time left on a window.

## 14. Invariants

| # | Invariant |
|---|-----------|
| T1 | Wake-ups pop in `(due tick, sequence)` order. Sequence numbers never change. |
| T2 | A wake-up's remaining local time is preserved across rate changes, reparenting, rehoming, saving and loading, to within one tick per change. |
| T3 | A parked wake-up never fires. |
| T4 | A channel slot is held by at most one action. |
| T5 | Simulation state changes from outside only through `submit`. |
| T6 | A save plus its input log replays to an identical state hash. |
| T7 | `advance` always returns. No rule chain, zero-length phase loop or rate change can prevent it. |
| T8 | Time does not move while a directly controlled entity has an unanswered decision. |
| T9 | Every entity that leaves the simulation has no running action and no queued wake-up. |

## 15. Budgets

Targets, unmeasured, on one thread. The benchmark package replaces them with
measured figures.

| Operation | Target |
|-----------|--------|
| Schedule plus pop | under 300 ns |
| Rate change on a clock with 1,000 wake-ups | under 200 µs |
| Phase transition, excluding its effects | under 2 µs |
| Starting an action, excluding effects | under 3 µs |
| Idle entities (waiting or mid-phase) | no cost between their wake-ups |
| One second of local time, 500 hot entities at two actions per second, excluding effects | under 5 ms |

## 16. Open questions

1. **Rewinding.** Settled: snapshots, area journals and restoring a place
   as generated (08 §7.4); the whole world by saving and loading with
   memories carried (13 §9.3, §9.5).
2. **Windows without named candidates.** Settled in section 7.1: windows
   are also offered to those who guard a named candidate and to controlled
   characters, within reach. Bystanders who merely witness react through
   `perceived` (15 §6).
3. **Clash detection.** Settled in 07 §8: strikes at each other due within
   a tolerance are brought together and resolved once.
4. **Warm fidelity.** Settled: no. Warm characters have no windows. Bulk
   exchanges (09 §9.1) count each side's usual guard in their expected
   values.
