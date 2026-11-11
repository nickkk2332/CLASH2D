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
pub struct ClockId(u32);       // ClockId::UNIVERSAL is the root
pub struct WakeId(u64);        // stable handle; survives rescheduling

pub struct Wake {
    pub domain: WakeDomain,    // who handles it: World, Action, Timer, Rule, Stepper, …
    pub id: u64,
    pub arg: u64,
}

impl Timeline {
    pub fn now(&self) -> Tick;
    pub fn local_now(&self, clock: ClockId) -> Dur;

    pub fn schedule(&mut self, clock: ClockId, after: Dur, wake: Wake) -> WakeId;
    pub fn cancel(&mut self, id: WakeId) -> bool;
    pub fn remaining(&self, id: WakeId) -> Option<Dur>;         // local time left
    pub fn rehome(&mut self, id: WakeId, to: ClockId);          // keeps remaining local time

    pub fn peek(&self) -> Option<Tick>;
    pub fn pop(&mut self, limit: Tick) -> Option<(WakeId, Wake)>;   // advances now

    pub fn create_clock(&mut self, parent: ClockId, rate: Real) -> ClockId;
    pub fn set_rate(&mut self, clock: ClockId, rate: Real);
    pub fn reparent(&mut self, clock: ClockId, parent: ClockId);
    pub fn destroy_clock(&mut self, clock: ClockId);            // its wake-ups rehome to the parent
}
```

### 3.2 Ordering

Wake-ups pop in order of `(universal due tick, sequence number)`. The
sequence number is assigned at scheduling and never changes, so two wake-ups
due at the same tick always run in the order they were scheduled, across
saves and rate changes.

### 3.3 Clocks

- A clock has a parent and a rate ≥ 0. Its **effective rate** is the product
  of rates up to the root.
- A clock keeps its own local time, which advances at its effective rate.
- `schedule(clock, after, …)` means "when `after` more local time has passed
  on this clock". The universal due tick is
  `now + ceil(after / effective_rate)`.
- When a clock's rate or parent changes, the clock and all its descendants
  first bring their local time up to date at the old rate, then every
  wake-up on them gets a new universal due tick from its remaining local
  time. Relative order on one clock is preserved.
- At effective rate 0 a clock's wake-ups are **parked**: they are not in the
  queue and never fire. Raising the rate re-queues them with their remaining
  local time intact.
- Rounding may shift a wake-up by at most one tick per rate change.

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

When an entity, body or process moves to a location with a different clock,
its wake-ups are rehomed. Movement goes through the single relocation
function (01 R3), which does this.

An **anchor** pins an entity's personal clock to a chosen parent regardless
of where the entity is. Anchoring to the realm clock is how someone keeps
moving inside a bubble of stopped time.

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

## 4. The simulation kernel

Crate `clash-sim`. The kernel is the part of the simulation that every other
document builds on.

### 4.1 State

```rust
pub struct Sim {
    pub defs: Arc<Defs>,
    pub cosmos: Cosmos,              // 02
    pub timeline: Timeline,
    pub entities: EntityTable,
    pub actions: ActionTable,
    pub windows: WindowTable,
    pub rules: RuleState,            // 05: attachments, continuations, cooldowns
    pub settings: VarMap,            // 04 §10
    pub rng: RngStreams,
    events: EventQueue,              // 05 §8
    decisions: DecisionQueue,
    out: Vec<SimEvent>,              // for presentation
    log: InputLog,
}
```

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
}
```

One iteration of `advance`:

1. If a decision is pending for a directly controlled entity, return
   `NeedInput`.
2. Pop the next wake-up within the limit. If there is none, return.
3. Dispatch it to its domain's handler.
4. **Pump events.** Drain world events into the event queue, then deliver
   queued events in order to rules (05 §8). Rules may change state and emit
   more events; those join the back of the queue. Continue until the queue
   is empty.
5. **Safe point.** If flagged: reconcile tickets (02 §10.2), run requested
   saves, apply a pending definition reload (04 §11).

Everything between two pops happens at one tick.

If one pump delivers more than `rules.max_cascade` events (default 10,000),
the remainder is dropped, the offending rule chain is reported with its
trace, and the simulation carries on. Content cannot hang the engine.

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

### 5.3 Plans

```rust
pub struct Plan { steps: VecDeque<PlanStep> }
pub struct PlanStep { action: ActionDefId, params: Params }
```

`Act { how: Enqueue }` appends a step. When a foreground action ends, the
next step whose channels are free starts automatically. If a step's
`requires` fails when it is due, the plan is cleared and a free decision is
requested with the reason.

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
| `tempo` | Expression dividing phase durations (section 6.5). Default 1. |
| `foreground` | Whether finishing this action can prompt a decision. Default true. A sustained aura or a conversation is background. |
| `reaction_to` | Window kinds this action may answer (section 7). |
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
| `until`, `max` | For held phases: end when the condition is true (checked at ticks) or the maximum is reached. A `Cancel` command also ends a held phase and counts as release, not as cancellation. |
| `cancel` | Whether the actor may abandon the action during this phase. Default false. |
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
  "noticed_if": "true" }
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

A phase's `window` field opens one when the phase starts. `for` lists the
candidates. `closes` names the later phase whose start is the deadline.
The `open_window` effect opens one from any rule.

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
  "requires": "not perceives($victim, $attacker)",
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

An entity's level is its tract's activity level (02 §10). Giving an entity a
hot ticket of its own promotes it. The kernel promotes a warm entity
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

How a thousand actions between two decisions are shown (played, blurred or
summarised) is document 12's concern, constrained by what the viewing
character could perceive (15).

## 13. Modding surface

**Definition types**: `action`, `channel`, `window_kind`.

**Events**: `ActionStarted`, `PhaseEntered`, `PhaseExited`, `ActionFinished`,
`ActionCancelled`, `ActionInterrupted`, `ActionAbandoned`, `PlanCleared`,
`WindowOpened`, `WindowNoticed`, `WindowClosed`, `DecisionRequested`,
`ControlChanged`, `ClockRateChanged`, `EntityRelocated`,
`EntityPromoted`, `EntityDemoted`.

**Hooks** (events whose handlers may alter or cancel them, 05 §8.2):
`ActionStarting` (parameters, veto), `PhaseTiming` (duration).

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

1. **Rewinding.** A deterministic input log makes "restore a snapshot and
   replay differently" possible. Whether to expose it as an ability
   mechanism is undecided. (08)
2. **Windows without named candidates.** Bystanders reacting to something
   not aimed at them needs perception. (15)
3. **Clash detection.** Two impacts that coincide within a tolerance should
   resolve together. The hook is `ActionStarting` and phase tags; the rule
   is combat's. (07)
4. **Warm fidelity.** Whether warm entities should keep windows for attacks
   from other warm entities. Decide with measurements. (09)
