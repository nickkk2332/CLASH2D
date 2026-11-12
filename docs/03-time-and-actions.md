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

As joined to the packages built beside it (decisions D-460 to D-462):

- The simulation holds every component of every entity (06 §3) in one
  `Components`, read with `Sim::components()`. It is saved in the blob and
  hashed as a part of its own, `SimHashes::components`;
  `Sim::component_hash(kind)` gives one store's hash.
- The simulation owns what loading a tract needs besides the world: the
  generators and a tract store (02 §9.4). Neither is state. A new
  simulation has the generators of `clash-gen` bound to its world's
  definitions and a store in memory; `Sim::with_io(generators, store)`
  gives it others, and a loaded simulation is given the store it was
  saved with. A store is anything that implements the world's
  `TractStore` and can be copied and printed (`SimStore`).
- The blob also carries what the world has under way: its processes and
  the collapses they belong to (`Cosmos::processes`), because the
  timeline that holds their wake-ups is the simulation's. `Sim::load`
  puts it back into the cosmos it is given. It refuses a blob that holds
  a component of an entity that does not exist.
- The blob carries the entities shelved with unloaded tracts (section
  4.3). They are hashed with the kernel's own queues.
- After a load, what is not state is as a new simulation has it: the
  default wake-up handlers and the event types of the definitions.
  Handlers and event types registered on top, the generators and the
  store are set up again by whoever set them up.

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
    pub channels: ChannelSlots,        // section 6.2
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
  so nothing else can change it. `vars` is changed by `Sim::set_var`
  (05 §6), which holds a declared variable to its declaration and raises
  `var_changed`; the field itself stays open for setting a scene up. The
  rest (`tags`, `channels`, `actions`, `plan`, `attached`) is open to
  whoever holds the entity mutably until its packages give it owners.
- `actions` is a plain list. `ChannelSlots` (an entity's slots and who
  holds each), `Plan`, the action table, the window table and the rule
  state are empty typed placeholders that already serialise and hash. A
  set of channel ids, which is what an action definition uses, is the
  action module's `ChannelSet`.
- Ids are generational. A freed slot is reused, lowest slot first, a
  generation on. The table visits entities in id order.
- `create_entity` makes an entity that is `Stored` and emits
  `entity_created`; `spawn` makes one and puts it somewhere, or makes
  nothing if it cannot be put there. `remove_entity` first puts what the
  entity held where the entity was (what cannot stand there leaves the
  world), then takes the entity out of the world, cancels its wake-ups,
  destroys its personal clock, forgets any decision it was asked for, and
  emits `entity_removed`. Its components are removed with it, before its
  slot can be given out again.

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
- The container it left and the container it entered have their mass
  worked out again, and so has every container round them (06 §3.1). Each
  of those that stands in the world has the new mass written to the
  world's occupant record.
- An entity that is given a place is no longer waiting for an unloaded
  tract (section 4.3).
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
- **The safe point** reconciles tickets with what is loaded
  (`Sim::reconcile_activity`, 02 §10.2) and then delivers whatever that
  reported. It follows every wake-up, and every accepted command, so that
  a ticket follows what a command moved. Requested saves are returned to
  the application as `Advance::Request`.
- **The world's wake-ups.** The handler of the `World` domain hands the
  wake-up to `WorldCtx::handle_wake` inside `Sim::world`. A stability
  check and every step of a collapse run under `advance` like anything
  else, and what they report comes out of `drain_events`. A process the
  world could not carry on is reported as a fault.

What the safe point does about entities (decision D-461). The world
cannot take entities out of a tract it wants to unload (02 §9.5): it
says so, and the simulation answers.

| The world says | The simulation |
|----------------|----------------|
| `tract_unloading` | Takes every entity with a cell in the tract out of the world. Its place becomes `Stored`, and where it stood is kept on the **shelf** under that tract. `entity_relocated` is emitted for each; what an entity holds stays inside it |
| `tract_loaded` | Puts back the entities shelved under the tract, each where it stood, with `entity_relocated`. One whose footprint reaches into a tract that is not loaded goes back on the shelf, under that tract |
| A loaded tract's spawn requests | Emits `spawn_requested` for each, for the package that makes entities from definitions |
| A tract that could not be loaded or unloaded | Reports a fault with the reason `world.tract_failed` |

A shelved entity keeps its id, its components and its contents. Its
wake-ups are cancelled as for any entity that leaves the world (T9):
taking timers off the timeline first and catching up on return is
section 9's, and not built. One safe point reconciles up to four times,
so that a tract announced as going is gone before anything else runs.

`Sim::start_session(entity, at, cause)` starts a session at a place that
may not be loaded: it holds a fixed ticket over the place, reconciles,
puts the entity there, holds the ticket of a controlled character for it
(`Ticket::player`) and releases the fixed one. An entity's ticket covers
nothing until the entity has been seen in the world, which is why the
fixed ticket comes first. `Sim::hold_ticket` and `Sim::release_ticket`
hold and release any other.

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
  `SetControl { entity, control }`, `StartSession { entity, at }` and
  `Schedule { entity, after, wake }`. `StartSession` is refused for an
  unknown entity or a place outside its realm; once tracts have been
  surveyed and loaded for it, a failure is a fault, not a refusal.
  The rest of the list above arrives with the packages that can honour
  each. A debug command is a command like any other: validated, logged
  and replayed.
- `Choose { entity, choice, option }` (package K9, decision D-542)
  answers a choice put by a rule's `ask` (05 §10.6) with the position of
  an option, from zero. It is refused with `decision.none_pending` if
  the entity is not waiting for that choice, and with
  `rule.choice_refused` for an option the choice does not have or may
  not take.
- `Act { entity, action, params, how }`, `Wait { entity, dur }`,
  `Cancel { entity, action, abandon }` and `ClearPlan { entity }`
  (packages T3 and T4, decisions D-543, D-544). `action` is an action
  definition's dense id, `params` the parameters by name (stored values),
  and `how` is `start` or `enqueue`. They address an entity under
  `Direct` or `Delegated` control and are refused with
  `command.not_controlled` for any other. `Act` with `start` is refused
  with the reason the action cannot start (section 6.6); one that a
  handler of `action_starting` refuses is taken, since the handlers ran,
  and raises `action_refused`. `Cancel` with `abandon` false lets a held
  phase go; otherwise it abandons the action where the phase allows it
  (`engine:action_cannot_cancel`, `engine:action_cancel_only_for`), and
  it is refused with `action.not_running` for an action that is not the
  entity's. `Wait` is refused with `command.bad_argument` for less than
  no time. `Act` and `Wait` answer a free decision.
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
- Of the kinds, `Free` and `Choice` exist. `PlanEmpty` carries the
  reason a step could not be started, if that is why. An entity that
  stops being directly controlled, or is removed, is no longer waiting
  for anything; a choice it was asked is then settled with its first
  option that may be taken, or forgotten if it was removed (05 §10.6).
- `Choice { choice, prompt, options }`: `choice` is a `ChoiceId`, never
  given twice in one simulation; `prompt` and each option's `label` are
  rendered for the entity; each option says whether it may be taken.

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

As built (decision D-544), in `clash_sim::action::plan`:

- An entity is **free** when it is in the world, no foreground action of
  its own runs, its plan is empty and it is not waiting
  (`Sim::is_free`). When it becomes free its plan is consulted first;
  then it is asked. It becomes free as it comes under direct control or
  enters the world under it (`started`), when a foreground action of its
  own finishes or is cancelled (`finished`) or is interrupted
  (`interrupted`), when a plan step could not start (`plan_empty`, with
  the reason) and when a wait is over (`waited`). A background action
  ending only consults the plan.
- A **directly controlled** entity is asked with a `Free` request, at
  most one at a time. **Anyone else** is not asked: `decision_needed` is
  raised (section 13), the hook the AI layer (09) fills. If something
  listens for it, an entity that nobody gives anything to do idles for
  `actions.idle_wait` of its own time (1 s by default) and is asked
  again; starting an action or a wait ends the idle. If nothing listens,
  nobody could answer, and the entity stays free with nothing scheduled.
  An entity that enters the world under no one's direct control is not
  asked: whatever made it decides when it starts deciding. A delegated
  entity takes commands and is never asked.
- A **wait** (`Wait`, the `wait` verb) occupies an entity for a length of
  its own time; when it is over the entity is free. A wait of no length
  answers a decision and asks nothing more: the entity is free and
  nothing is due.
- **Plans.** `Act` with `enqueue` and `enqueue_action` append a step and
  start it at once if nothing is in the way. The front step starts when
  no foreground action of the entity runs; a step whose slots are held
  waits for them; a step that cannot start for any other reason clears
  the plan (`plan_cleared` with the reason) and the entity is asked with
  `plan_empty`. A step is checked when it starts, with its defaults
  worked out then. `ClearPlan`, `clear_plan` and `interrupt_plan` empty a
  plan; `interrupt_plan` also asks, whatever is running.
- An entity that leaves the world keeps its plan, and takes it up again
  when it comes back; one that is removed loses it.

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
  "attacks": {
    "fist": { "with": { "part": "hand", "surface": "fist" }, "energy": 120, "push": 0.1 }
  },
  "phases": [
    { "id": "windup", "time": "300 ms", "cancel": true,
      "window": { "kind": "strike", "for": ["$target"], "closes": "impact" } },
    { "id": "impact", "time": 0,
      "do": [ { "strike": { "target": "$target", "part": "$part" } } ] },
    { "id": "recovery", "time": "400 ms" }
  ]
}
```

The `attacks` block and the `strike` effect belong to document 07 (§3.1,
§3.4): `strike` resolves the action's one attack against `$target`, and
leaves whether it landed in `$landed`.

| Field | Meaning |
|-------|---------|
| `name` | What the action is called, shown to players. A text. Optional. |
| `uses` | Channels occupied for the whole action (section 6.2): an object of channel ids, each with a number of slots or `"all"`. Default none. Not allowed beside `roles`. |
| `params` | Typed parameters, by name. Each may have a `default`, an expression evaluated when the command omits it (section 6.3). |
| `vars` | The action's own variables, by name, each with a type and a starting value (section 6.4). |
| `requires` | Condition that must hold to start. Default true. |
| `fail_text` | The text shown when `requires` does not hold. Optional. |
| `requires_at_start` | A condition tested only when the action begins, and never shown beforehand. If it fails the action is interrupted at once and says why. For things the actor cannot know in advance (01 R11): whether a victim has noticed them. Default true. |
| `tempo` | Expression dividing phase durations (section 6.5): a number above zero. Default 1. |
| `foreground` | Whether finishing this action can prompt a decision. Default true. A sustained aura or a conversation is background. |
| `footing` | Whether the actor must be standing on something. Default true. An action without it can be done in the air (07 §7.1). |
| `reaction_to` | Window kinds this action may answer (section 7): a list of `window_kind` ids. Default none. |
| `on_behalf` | Whether, as a reaction, it may answer a window aimed at someone else. Default false. |
| `ready` | For a reaction: the id of the phase from which it counts as ready (section 7.3). Optional. |
| `phases` | Section 6.4. A list of at least one phase. Required. |
| `on_start`, `on_finish`, `on_cancel`, `on_interrupt`, `on_abandon` | Effect lists (section 6.6; `on_abandon` section 9). Default empty. |
| `roles` | Present on joint actions (section 8). |
| `tags` | Free-form tags, queryable by rules and named by a phase's `cancel`. Default none. |
| `low_attention` | Section 11. Default false. |

**What an action's expressions may name.** Every expression of an action
binds one subject, `self`, the actor, and the action's **locals**, written
`$name`: the window being answered (`$window`, on an action with
`reaction_to`; null when it was not started as a reaction), then the roles,
the parameters and the variables, each in the order written. A running
action keeps one value for each. The names must differ from each other,
start with a lowercase letter, hold lowercase letters, digits and `_`, and
not be a word of the expression language. A local that is none of these is
a load error where it is read.

| Field | Reads (05 §7.3) | Pure |
|-------|-----------------|------|
| `requires`, `fail_text`, a parameter's `check`, `name` | Belief: the mind context of the actor | Yes |
| A parameter's `default` | Belief | No |
| `requires_at_start`, `tempo`, a phase's `time`, `max` and `until`, a tick's `every`, the conditions of `next`, a window's `for` | Truth | No |
| A variable's `default` | Nothing: it is a constant | Yes |
| Effect lists | Truth | No |

`name` sees no locals: it is shown before any are chosen. `requires` is
compiled in the mind context so that it can be previewed, and is tested
against truth only when the action is started (05 §7.3).

An effect list (05 §10) is compiled with the action's locals in their
slots, so `set` and `add` change the running action's own values. What a
list defines with `let` takes the slots after them and lasts until the
list ends. `return` has no meaning in an action's list and is a load
error; `stop` ends the list.

### 6.2 Channels

A **channel** is something an action needs exclusive use of.

```json
{ "type": "channel", "id": "arm" }
```

A channel has one field, `name`: a text, what the channel is called where
a player is told of it, as in the reason an action cannot start. It is
optional; without it the id is shown. There may be at most 256 channel
definitions, so that a set of channels is a small fixed bitset; one more
is a load error.

An entity has a set of channel **slots**. Document 06 derives them from the
body: two arms give two `arm` slots, a mouth gives `voice`, a mind gives
`focus`. A creature with six arms has six. A device has whatever its
definition gives it.

- `"uses": { "arm": 2, "legs": 1 }` needs two free arm slots and one legs
  slot. `{ "arm": "all" }` needs every arm slot the entity has, and at least
  one. A number of slots is a whole number of at least one, and a channel
  is named once.
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

A parameter is written as its type alone, `"target": "entity"`, or as an
object:

| Field | Meaning |
|-------|---------|
| `type` | The type of the value, as 05 §3 writes types: `entity`, `part`, `real`, `id<material>`, `entity?`. `cell` is a location, the kind `loc`. `list` is a list of what `of` says. Required. |
| `of` | For `list`: the type of the elements. A list may also be written whole, `list<loc>`, without it. |
| `min`, `max` | For a list: the least and the greatest number of values. `min` defaults to 0. With no `max` a list is as long as any value may be (`rules.max_value_size`). |
| `default` | An expression of the parameter's type, in the mind context of the actor (05 §7.3). It may read the parameters, and roles, written before it: defaults are worked out in the order the parameters are written. A parameter with none must be given by the command. |
| `check` | An expression in the mind context of the actor, pure, that may read the parameter and those before it. It gives null when the value is sound; anything else is the reason shown: a text, or the id of a `reason` definition (14 §15). |

A type that is not one, a default that cannot give a value of the
parameter's type, `of`, `min` or `max` on something that is not a list and
a `max` below its `min` are load errors.

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
| `id` | The phase's name within its action: lowercase letters, digits and `_`, starting with a letter. No two phases of an action share one. Required. |
| `time` | Duration expression, in seconds of the actor's local time, or `"hold"`. Evaluated when the phase starts. `0` runs and moves on within the same tick. It cannot be below zero. Required. |
| `do` | Effects run when the phase starts. |
| `on_exit` | Effects run when the phase ends normally. |
| `tick` | Effects repeated at an interval while the phase lasts: `every`, a duration expression above zero, evaluated when the phase starts, and `do`, the effects, which may be left out for a tick that only tests `until`. |
| `until`, `max` | For held phases: end when the condition is true (checked at ticks, so a phase with `until` has a `tick`) or the maximum, a duration expression, is reached. A `Cancel` command also ends a held phase and counts as release, not as cancellation. `Cancel` with `abandon` gives it up instead, where the phase allows `cancel`. |
| `cancel` | Whether the actor may abandon the action during this phase. Default false. A list of action tags instead of `true` allows abandoning it only to start an action with one of those tags: a recovery that can be cut short by a follow-up and nothing else. |
| `uninterruptible` | The `interrupt` effect cannot end the action during this phase. Default false. |
| `fixed` | Tempo does not apply to this phase. Default false. |
| `window` | Opens a reaction window when the phase starts (section 7.1). |
| `tags` | Free-form, queryable by rules: `vulnerable`, `telegraph`, `airborne`. |
| `next` | Where to go afterwards: the first entry whose condition holds. Default is the following phase; after the last, the action finishes. |

An entry of `next` has `if`, a condition, which may be left out for an
entry that always holds, and says where to go: `goto` with the id of a
phase, or `"finish": true`, which ends the action there. If no entry holds,
the default applies: the following phase. So a branch that is not the last
phase written ends the action with `"next": [ { "finish": true } ]`.

`next` makes an action a small state machine: follow-ups that branch on
whether a blow landed, charge-and-release, loops that repeat while a
condition holds. More than 1,000 zero-duration transitions in one tick ends
the action with an error.

These are load errors: a `goto` that names no phase of the action; a phase
that no path from the first phase leads to; `max` or `until` on a phase
that is not held; `until` on a phase with no `tick`. An entry written after
one that always holds is never tried, which is a warning.

Action instances have local variables (`$name`), shared by all their phases
and effects. Those that are not parameters or roles are declared in the
action's `vars`, each as its type or as an object with `type` and
`default`, a constant of that type:

```json
"vars": { "power": { "type": "int", "default": 0 }, "landed": "bool" }
```

A variable starts as its default when the action starts, and as null if it
has none; its type is then "the declared type, or null". Effects assign it
with `set` and `add` (05 §10.3). A verb that leaves results in locals of
the action, as `strike` does (07 §3.4), needs them declared like any
other.

### 6.5 Tempo

`duration = time / tempo`, with `tempo` evaluated at the start of each
phase. Once a phase is scheduled, a later change of tempo does not move it.
A change of clock rate does.

Tempo divides `time` and nothing else: a held phase's `max` and a tick's
`every` are local time as written.

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

As built (decision D-543), in `clash_sim::action::run`. A running action
is plain state in the simulation's action table: the actor, the
definition, one value for each local in slot order, the phase, the slots
it holds, the wake-ups of its phase and its tick, and when the phase
began and is due. It is saved and hashed with the simulation.

- **Starting** checks, in this order, and refuses with the first reason
  that applies, changing nothing: the actor is known and in the world
  (`entity.stored`) and the definition is loaded (`action.unknown`); a
  parameter given is one of the action's (or a role's name) and every
  parameter has a value of its type, from the command or from its
  `default` worked out in order, and a list is within its bounds; a role
  other than the first is given an entity (`engine:role_unfilled`);
  `footing`; `requires`, with `fail_text` as the sentence; slots. A
  default that draws at random is drawn again after a refusal: the
  random streams are put back. A parameter's `check` is never tested.
- **Slots.** An entity has, of each channel, the slots its body supplies
  (06), or one of every channel if it has no body. `disable_channel`
  takes some away. An action needs, of each channel it uses, as many
  enabled slots as it names (`all`: every enabled slot, and at least
  one), else `engine:action_channel_lacking`, and that many free, else
  `engine:action_channel_busy`. Slots held by an action whose phase
  allows `cancel` for the new action (`true`, or a list holding one of
  its tags) count as free: starting the new action cancels that one
  (`on_cancel`, `action_cancelled`). A plan step never cancels anything
  to make room. Slots are counted, not named: a `part` parameter does
  not choose one yet.
- `action_starting` is raised only if something listens for it, after
  the checks. A handler may change `params`, which are held to their
  types again, or cancel; a refusal from here on is raised as
  `action_refused`, with `engine:action_vetoed` when the handler gave no
  reason of its own. Then the slots are claimed, a wait of the actor is
  ended, `action_started` is raised, `requires_at_start` is tested
  against truth (failing, the action is interrupted at once with
  `engine:action_start_condition`), `on_start` runs and the first phase
  is entered.
- **Entering a phase** works out its duration (its `time` over the
  tempo unless `fixed`, or a held phase's `max`), raises `phase_timing`
  if something listens, schedules one wake-up on the actor's clock,
  raises `phase_entered` with the duration and due tick and
  `window_opened` for a phase with a window, runs `do`, and schedules
  the first tick. A phase of no length ends at once, in the same tick. A
  tempo of zero or below, a duration below zero and an interval of zero
  or below are faults (`action.bad_time`): the tempo counts as 1, the
  duration as none, the tick does not repeat. Entering more than
  `rules.max_transitions` phases in one tick interrupts the action with
  `engine:action_transition_limit`.
- **A tick** runs its effects and then tests `until`; holding, the phase
  ends. **Leaving a phase** runs `on_exit`, raises `phase_exited` and
  goes where the first entry of `next` that holds says, else to the
  following phase, else out of the action.
- **Ending.** Finishing runs `on_finish`; cancelling `on_cancel`;
  interrupting `on_interrupt`; abandoning `on_abandon`. Then the slots
  are released, the wake-ups cancelled, and the event raised. `interrupt`
  passes over a phase that is `uninterruptible`; `cancel_action` and
  `Cancel` respect the phase's `cancel`. A slot disabled under an action
  interrupts it, newest first, with `engine:action_channel_disabled`;
  `Sim::check_slots` does this for whatever takes slots away, a body
  that loses a part among them. An actor that leaves the world, by being
  stored, shelved with its tract or removed, abandons its actions
  (section 9).
- Effect lists run with the action's locals, which `set` and `add`
  change in place, and are caused by the actor with the kind `action`.

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

| Field of a `window_kind` | Meaning |
|--------------------------|---------|
| `notice` | How long a candidate takes to notice: a duration expression, in seconds of the candidate's own local time. Default 0: noticed the moment the window opens. It cannot be below zero. |
| `noticed_if` | Whether a candidate can notice at all: a condition. Default true. |
| `reach` | How far, in metres, a candidate the window does not name may be from the source or from someone it names. Default 0. |

`notice` and `noticed_if` are evaluated once for each candidate when a
window of the kind opens (section 7.2). They read truth and bind three
subjects, in this order: `self`, the candidate, an `entity`; `source`,
whose action or rule opened the window, an `entity` or null; and `window`,
the window. Which actions may answer a window of a kind is said by the
actions, in `reaction_to`.

| Field of a phase's `window` | Meaning |
|-----------------------------|---------|
| `kind` | The id of a `window_kind`. Required. |
| `for` | Those it is aimed at: a list of expressions in the action's scope, each giving an entity. One that comes to null names nobody. Default empty. |
| `closes` | The id of a phase of the same action. It must be another phase than the one that opens the window, and one that some path from that phase leads to; otherwise the window could never close, which is a load error. Required. |

If the action ends without entering that phase, the window closes when the
action ends.

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
   tests; until then content can write its own. As built (decision
   D-615), whoever opens windows first asks perception's
   `notices_source(candidate, source)`: a candidate that does not
   perceive the source is not offered the window.
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

- A role has two fields: `uses`, the slots claimed on its participant,
  written as an action's `uses` is (default none), and `forced` (default
  false). A joint action has at least two roles and no `uses` of its own.
- The first role written is whoever starts the action: its participant is
  `self`, and it cannot be `forced`. The participants of the other roles
  are given with the command, by the role's name, as parameters are.
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
| `entity_unsupported` | `self`: entity; `here`: loc | none | What held an entity up has gone (02 §14.3). It is where it was: what falling does is for the movement package. Follows the world's `occupant_unsupported` |
| `entity_exposed` | `self`: entity; `here`: loc | `kind`: string, the qualified id of a damage type; `energy`: real, joules; `intensity`: real, pascals | Something struck the cell an entity is in (02 §14.3). What that does to it is for the body model. Follows the world's `occupant_exposed` |
| `spawn_requested` | `here`: loc | `what`: string, the id the request names; `realm`: int | A tract that has just been loaded asks for something to be put at `here` (02 §9.4) |
| `var_changed` | `self?`: entity; `here?`: loc; `area?`; `site?`; `tract?`; `realm?` | `name`, `scope`: string; `old`, `new`: any | A variable declared with `"watch": true` changed (05 §6). What it lives on is bound under the name of its kind |
| `fault` | none | `code`, `message`: string | The engine contained a fault. Goes straight to the output; no handler receives it |
| `event_cancelled` | none | `event`, `code`, `message`: string | A hook was cancelled (05 §8.2) |

The events of acting, as built (decisions D-541 to D-545). Each binds
the entity it is about as `self`, and `actor` and `instigator` from its
cause. `action` is the qualified id of the action's definition, `id`
the number of the running action (an int no other action of the
simulation ever has), `phase` the id of a phase, and `code` and
`message` the reason's code and sentence.

| Event | Fields | Raised when |
|-------|--------|-------------|
| `action_started` | `action`, `id` | An action began: its slots are claimed and its `on_start` is about to run |
| `phase_entered` | `action`, `id`, `phase`; `held`: bool; `duration`: real?, seconds of the actor's clock; `due`: int?, the universal tick in microseconds it is due to end at as the clocks now run | A phase began. Its stamp is the start; `duration` and `due` are null for a held phase with no `max` (section 12) |
| `phase_exited` | `action`, `id`, `phase` | A phase ended normally: its `on_exit` has run |
| `action_finished` | `action`, `id` | An action ran to its end: `on_finish` has run and its slots are free |
| `action_cancelled` | `action`, `id`, `phase` | The actor abandoned an action in a phase that allows it: `on_cancel` has run |
| `action_interrupted` | `action`, `id`, `phase`, `code`, `message` | An action was ended against the actor's will: `on_interrupt` has run |
| `action_abandoned` | `action`, `id` | An action ended because its actor left the world or was removed (section 9): `on_abandon` has run |
| `action_refused` | `action`, `code`, `message` | An action that an effect or a plan asked for did not start, or a handler of `action_starting` refused one a command asked for |
| `plan_cleared` | `code?`, `message?` | A plan was emptied, with the reason if a step could not start or a rule interrupted it |
| `window_opened` | `action`, `id`; `kind`: string; `closes`: string, a phase id; `count`: int, how many it is aimed at | A phase that opens a window began. Until windows are built (T5) nobody is offered it |
| `decision_needed` | `control`: `none` or `delegated`; `why`: `started`, `finished`, `interrupted`, `plan_empty` or `waited` | An entity that is not directly controlled is free to act (section 5.2) |
| `message` | `text`: string, rendered for `self` as its reader; `style`: string | A rule's `message` (05 §10.6): a line for the log of whoever controls `self` |
| `choice_made` | `choice`: int; `option`: int, from zero; `label`: string | A choice put by `ask` was answered, by `Choose` or at once; the option's effects run next |
| `tag_changed` | `tag`: string; `added`: bool | An entity was given a tag it did not have, or lost one it had |

The two hooks, as built: `action_starting` binds `self` and has the
fields `action` and `params`, the parameters by name as a map with the
defaults filled in, which a handler may change; `phase_timing` binds
`self` and has `action`, `id`, `phase` and `duration`, seconds after
tempo, which a handler may change (a value below zero counts as zero).

The world's events (02 §7.4) are passed on under their own names. Their
subjects are: `here` for a cell; for `area_entered` and `area_exited`,
`area`, then `target` (the occupant), then `here`; for `area_breached` and
`area_closed`, `area` and `here`; for `occupant_displaced`,
`occupant_entombed`, `occupant_unsupported` and `occupant_exposed`,
`target` and `here`. For `tract_loaded`, `tract_unloading` and
`ticket_refused`, `here` is the tract's cell with the lowest horizontal
coordinates at storey 0; for `collapse_started` and `collapse_settled`,
the cell whose change started the collapse. The fields a filter can test
are `layer` on `element_changed`, `element_damaged` and
`element_destroyed`; `old` and `new` on `element_damaged`; `how`
(`damage`, `crushed` or `fell`), `absorbed` and `passed` on
`element_destroyed`; `count` on `decals_removed`; `key`, `old` and `new`
on `cell_var_changed`; `collapse` (int) on the two collapse events;
`energy` and `intensity` on `occupant_exposed`; `realm` (int) on the
tract and ticket events, with `first` (bool) on `tract_loaded` and
`ticket` (int) on `ticket_refused`.

The events of fields, bodies and releases are passed on the same way.
`field_contact`, `occupant_dosed` and `body_hit_occupant` bind `target`
(the occupant) and `here`. `body_launched`, `body_impact`, `body_landed`
and `body_left_loaded` bind `target` when the body is an entity, and
`here`: the cell it starts in, the cell of the element it struck, the
cell it came to rest in, the last loaded cell it was in. The rest bind
`here`. Their fields are `level` (int) on `field_appeared`,
`field_contact` and `occupant_dosed`, with `pool` (string), `amount`
(real) and `route` (`breath`, `skin` or `any`) on the last; `active` and
`max` (int) on `field_budget_exceeded`; on every body event `body`
(int), `owner` (`entity`, `debris` or `projectile`) and `tag` (int, a
projectile's, else null), with `mass` and `speed` (real) on
`body_launched`, `layer`, `energy`, `intensity` and `result`
(`unaffected`, `damaged` or `destroyed`) on `body_impact`, `energy` and
`intensity` on `body_hit_occupant`, and `returns` (bool) and `energy` on
`body_left_loaded`; `release` (int), `energy` and `reach` (real) on
`release_started` and `energy_released`, and on the second also
`absorbed` (real), `cells` (int), `cut_off` (bool) and its totals over
everyone responsible: `elements`, `exposed` and `thrown` (int) and
`mass` (real). A definition an event names by id (a field, a damage
type, a material) is not a field a filter can test: the kernel's own
event names it where there is one, as `entity_exposed` does.

**Reasons** (14 §5) of the kernel, by code: `entity.unknown`,
`entity.stored`, `entity.ids_exhausted`, `relocate.inside_itself`,
`relocate.too_deep`, `world.not_loaded`, `world.outside_realm`,
`world.unknown_grid`, `world.unknown_area`, `world.unknown_realm`,
`world.refused`, `world.tract_failed`, `component.missing`,
`physical.bad_measure`, `clock.unknown`, `clock.bad_rate`, `clock.loop`,
`time.refused`, `advance.unhandled_wake`, `advance.tick_limit`,
`events.cascade_limit`, `events.hook_depth`, `events.unknown_type`,
`events.bad_payload`, `events.field_fixed`, `events.cancelled`,
`decision.none_pending`, `replay.diverged`; and, from writing variables
and properties (05 §6, §10.3), `subject.gone`, `vars.wrong_scope`,
`vars.wrong_type`, `vars.too_deep`, `write.not_settable`,
`write.bad_value`, `write.bad_key`, `write.cannot_add`,
`write.not_a_subject`; and, from acting (decisions D-543, D-544),
`action.unknown`, `action.not_running`, `action.interrupted`,
`action.bad_time`, `command.not_controlled` and `command.bad_argument`.
Each is documented in the table the generated reference reads.

**The queries as properties** (05 §5.3). All read truth and none is safe
for belief; a mind reads them of itself.

| Property | Gives |
|----------|-------|
| `entity.loc` | `loc?`: the cell it is in, its own or that of what it is inside; null out of the world. **Settable**: relocates it to stand in the cell (`entity_relocated`, after the world's area events); null takes it out of the world |
| `entity.container` | `entity?`: what it is directly inside. **Settable**: puts it inside another; null takes it out, to where its container is |
| `entity.outermost` | `entity?`: itself, or the container everything it is nested in is inside |
| `entity.contents` | `list<entity>`: what is directly inside it, in id order |
| `entity.in_world` | `bool` |
| `entity.altitude` | `real?`: metres above the base of its cell |
| `entity.occupant_kind` | `string`: `creature`, `item`, `device` or `other` |
| `entity.tags`, `entity.has_tag(tag: string)` | Its tags |
| `entity.control` | `string`: `none`, `direct` or `delegated`. **Settable** (`control_changed`) |
| `entity.time_rate` | `real?`: the rate of its personal clock, or null. **Settable**: a rate gives it a personal clock or changes it, null takes the clock away (`clock_rate_changed`) |
| `entity.clock_rate` | `real?`: how fast time passes for it against universal time |
| `entity.local_time` | `real?`: the local time of the clock it is on, in seconds |
| `entity.own_time` | `real?`: its own time (section 3.4), in seconds |
| `area.clock_rate`, `realm.clock_rate` | `real?`: the rate of the area's or the realm's own clock. **Settable** (`clock_rate_changed`); null takes an area's clock away and is refused for a realm |
| `area.local_time`, `realm.local_time`, `loc.clock_rate`, `loc.local_time` | `real?`: 02 §23 |
| `global.now` | `real`: universal time in seconds. Safe for belief |

`now()` and `local_now(subject)` are functions (05 §17).

**Not yet reachable from data.** Queries of what an entity is doing
are not registered as properties yet: its running actions with their
phase, tags and time remaining, whether a channel is free and the length
of a plan are read in Rust (`Sim::actions_of`, `Sim::is_acting`,
`Sim::is_free`, `Sim::enabled_slots`, `Sim::held_slots`, the plan of the
kernel entity), and the time left on a window waits for windows (T5). The
debug commands are not yet console commands. What a reader knows of
others (05 §4.6) is supplied in Rust.

**Settings**: `rules.max_cascade`, `rules.max_transitions` (zero-duration
phase changes in one tick, default 1,000), `actions.idle_wait` (seconds
an entity nobody gave anything to do idles before it is asked again,
default 1; zero or below asks nothing more). `WARM_STEP` and
`DEFAULT_SLOTS` (one slot of every channel for an entity with no body)
are named constants.

**Fields**: every field of the three types is in a table of this
document: an action's in section 6.1, a parameter's in 6.3, a phase's, an
entry of `next`'s and a variable's in 6.4, a channel's in 6.2, a window
kind's and a phase's window's in 7.1, and a role's in section 8. Fields
that later documents add to an action (`attacks` and `defence` in 07,
`cutscene` in 12, the `choice` parameter type in 08 §9.2) are added by the
packages that build them. A cost, a cooldown and a rank belong to a grant
(08 §5.1), not to the action.

**Reasons** (14 §5), each with the values it carries, in the `engine`
namespace:

| Reason | Given when |
|--------|-----------|
| `action_parameter_missing`, `action_parameter_invalid`, `action_list_length` | A command leaves out a parameter that has no default, gives one a value of the wrong type, or gives a list outside its `min` and `max` |
| `action_requires` | `requires` does not hold when the action is to start, or when a step of a plan comes due. The action's `fail_text` is the sentence shown |
| `action_no_footing` | `footing` is true and nothing is under the actor |
| `action_channel_busy`, `action_channel_lacking`, `action_slot_unavailable` | Too few slots of a channel are free; too few are enabled at all; the slot a parameter names cannot be used |
| `action_vetoed` | A handler of `ActionStarting` cancels without a reason of its own |
| `action_start_condition` | `requires_at_start` does not hold: the action is interrupted at once |
| `action_cannot_cancel`, `action_cancel_only_for` | `Cancel` in a phase that does not allow it, or allows it only for other tags |
| `action_transition_limit` | `rules.max_transitions` is passed |
| `action_channel_disabled` | A slot the action held is disabled |
| `reaction_not_listed`, `reaction_not_on_behalf`, `reaction_cannot_cancel` | A reaction does not list the window's kind; the window names someone else and the reaction does not allow `on_behalf`; the reactor's own actions hold the slots and cannot be cancelled |
| `window_not_noticed`, `window_noticed_late`, `window_closed` | A candidate never noticed, noticed after the window closed, or answers a window that has closed |
| `role_unfilled`, `role_declined`, `role_participant_left` | A role has nobody in it; an offered role was declined or not answered; a participant left a joint action |

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

The verbs as built (decisions D-541 and D-545), each with its arguments.
An argument marked `?` may be left out, and is then null. A verb that
declines says why (`rule.verb_refused` carries the reason), and the rule
carries on. Whatever a verb changes is caused by the rule that called
it (05 §13.1).

| Verb | Arguments | Does | Raises |
|------|-----------|------|--------|
| `set_time_rate` | `who`: entity; `rate?`: real, zero or more | Gives `who` a personal clock at `rate`, or changes the rate of the one it has. With no rate the personal clock, and any anchor, is taken away | `clock_rate_changed` |
| `anchor_clock` | `who`: entity; `to?`: an area that has a clock, a realm, a cell or an entity | Pins the personal clock of `who` under the clock `to` stands for: the area's, the realm's, the cell's location's, the clock the entity is on. An entity with no personal clock is given one at rate 1. With no `to` the anchor is lifted | `clock_rate_changed`, when a clock is given |
| `set_clock_rate` | `of`: an area or a realm; `rate?`: real, zero or more | Changes the rate of the area's or the realm's clock. With no rate an area's clock is taken away; refused for a realm | `clock_rate_changed` |
| `set_control` | `who`: entity; `mode`: string, `none`, `direct` or `delegated` | Changes who decides for `who` (section 5.1) | `control_changed`; `decision_requested` or `decision_needed` if it is then free |
| `add_tag` | `to`: entity; `tag`: string | Gives an entity a tag | `tag_changed`, if it did not have it |
| `remove_tag` | `from`: entity; `tag`: string | Takes a tag away | `tag_changed`, if it had it |
| `start_action` | `who`: entity; `action`: id of an action; `with?`: map, the parameters by name | Starts an action as `Act` does (section 6.6). Gives whether it started; one that cannot start is not a fault | `action_started` and what follows, or `action_refused` |
| `enqueue_action` | `who`; `action`; `with?` | Puts an action at the end of the plan (section 5.3) | as the plan goes on |
| `cancel_action` | `who`; `action?`: id of an action; `channel?`: id of a channel; `tag?`: string | Cancels the running actions of `who` that match (all if none is named), each in a phase whose `cancel` allows it at will; a held phase is given up. Gives how many | `action_cancelled` |
| `interrupt` | `who`; `action?`; `channel?`; `tag?` | Interrupts the matching actions not in an `uninterruptible` phase, with `action.interrupted`. Gives how many | `action_interrupted` |
| `clear_plan` | `who` | Empties the plan | `plan_cleared`, if it held anything |
| `interrupt_plan` | `who` | Empties the plan and asks what to do next, whatever is running | `plan_cleared`; `decision_requested` or `decision_needed` |
| `disable_channel` | `who`; `channel`: id of a channel; `count?`: int, default 1 | Disables slots; what then holds more than are enabled is interrupted, newest first | `action_interrupted` |
| `enable_channel` | `who`; `channel`; `count?` | Enables disabled slots again, never more than were disabled | none |
| `wait` | `who`; `time`: real, seconds of its own time, zero or more | Occupies `who` doing nothing (section 5.3) | `decision_requested` or `decision_needed` when it is over |

A verb given an argument it cannot use (null where an entity is needed,
a word that is no mode) declines with `verb.bad_argument`, which carries
`verb` and `argument`.

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
