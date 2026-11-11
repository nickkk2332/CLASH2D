# Plan — Simulation Kernel: Data, Rules and Actions

Implements [03-time-and-actions.md](../03-time-and-actions.md),
[04-data-and-modding.md](../04-data-and-modding.md) and
[05-expressions-and-eoc.md](../05-expressions-and-eoc.md).

The three are planned together because they depend on each other: actions
run effects, effects are compiled by the data pipeline, and rules need
entities and time to act on.

Section references such as "05 §8" point into those documents. Packages
prefixed F and W are in [world-model.md](world-model.md).

## How to use this plan

The rules of [world-model.md](world-model.md) apply unchanged: one package
per change set, dependencies `done` first, the specification is the source
of truth, do not build ahead, meet the definition of done in
[CLAUDE.md](../../CLAUDE.md).

Packages P1 to P5 and X1 to X3 need only the foundation (F1 to F4) and can
start while the world model is being built.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| P1 | Diagnostics | M | F4 | todo |
| P2 | Complete merge semantics | M | F4 | todo |
| P3 | Profiles and running without mods | S | F4 | todo |
| P4 | Typed loading and validation stages | M | P2 | todo |
| P5 | Schemas, reference, scaffolding | M | P1, P4 | todo |
| P6 | Settings | S | K5 | todo |
| P7 | Migration and obsoletion | M | P4, W10 | todo |
| P8 | Hot reload | M | K7, P7 | todo |
| X1 | Expression parser | M | F2 | done |
| X2 | Registry, contexts, type checking | L | X1, F4 | in progress |
| X3 | Bytecode and evaluation | M | X2 | todo |
| X4 | Expression fields, functions, variables | M | X3, P4 | todo |
| K1 | Simulation skeleton and kernel entities | M | W7, F3 | todo |
| K2 | Advance, commands, input log | M | K1, W9 | todo |
| K3 | Events and hooks | M | K2, X2 | todo |
| K4 | Subjects, properties, variable storage | L | K1, X3 | todo |
| K5 | Rule runtime | L | K3, K4, X4 | todo |
| K6 | Queries and value sources | M | K5 | todo |
| K7 | Timers, continuations, simulation saves | L | K5, W10 | todo |
| K8 | World verbs | M | K5 | todo |
| K9 | Messages and choices | S | K5 | todo |
| K10 | Tracing, explain, profile, console | M | K5, W11 | todo |
| T1 | Clock assignment | M | K1 | todo |
| T2 | Action definitions | M | X4 | todo |
| T3 | Action runtime | L | T2, K5 | todo |
| T4 | Control, plans, free decisions | M | T3 | todo |
| T5 | Windows and reactions | M | T4, T1 | todo |
| T6 | Joint actions | M | T5 | todo |
| T7 | Unloading and lower detail | M | T3, K7 | todo |
| T8 | Benchmarks and budget gate | S | X3, then recurring | todo |

## Order and parallelism

Packages in the same wave do not depend on each other.

| Wave | Packages | Needs from the world plan |
|------|----------|---------------------------|
| 1 | P1, P2, P3, X1 | F2, F4 |
| 2 | P4, X2 | — |
| 3 | P5, X3 | — |
| 4 | X4, K1, T8 | W7, F3 |
| 5 | T2, K2, K4, T1 | W9 |
| 6 | K3 | — |
| 7 | K5 | — |
| 8 | K6, K7, K8, K9, K10, T3, P6 | W10, W11 |
| 9 | T4, T7, P7 | — |
| 10 | T5, P8 | — |
| 11 | T6 | — |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M2a Mods that explain themselves** | P1–P5 | A deliberately broken mod produces precise, located diagnostics with suggestions. `show` prints where each value of a patched definition came from. An editor completes keys and ids from generated schemas. The reference is generated. |
| **M2b Rules** | X1–X4, K1–K6, K8–K10 | From the console: break a vault wall and watch a rule set a site variable, message the player and send guards. Trace the rule. Ask why another did not fire. |
| **M2c Things happen in time** | K7, T1–T7 | A punch defined in JSON winds up, lands and breaks a wall. A slow character fails to notice a fast attack. An action freezes mid-windup inside stopped time. A party of two is asked for decisions in order. A takedown locks two characters together. |
| **M2d Living mods** | P6–P8 | Change a setting in play. Edit a rule on disk and see it take effect without restarting. Load a save after renaming a definition. |

## Data pipeline

### P1 — Diagnostics

**Spec** 04 §8 · **Size** M

Build
- A diagnostic type with severity, stable code, source span, definition,
  JSON path, message and suggestion.
- Rendering in the form of 04 §8.2, and as JSON.
- A registry of codes with long explanations; `clash-tools explain`.
- Conversion of every existing F4 error to the new form.
- `--lenient`.

Done when
- Each error class in F4's fixtures renders with a caret under the right
  characters.
- Every code has an explanation, and a test fails if one is added without.
- JSON output round-trips.

### P2 — Complete merge semantics

**Spec** 04 §4.3, §5, §6 · **Size** M

Build
- `patch`, `optional`, `scale`, `add`, dotted paths, id-matched lists.
- `abstract`, `if_mod`, `unless_mod`.
- `imports` and the reference resolution order, with the shadowing warning.
- Origin tracking for every value in a finished tree.
- `clash-tools show`.

Done when
- Fixtures cover each operation and each error in 04 §6.1.
- Patching a parent changes its children.
- An override discards earlier patches and accepts later ones.
- `show` attributes every value of a three-mod fixture to the right file
  and line.
- Results are independent of file system enumeration order.

### P3 — Profiles and running without mods

**Spec** 04 §3.3–3.5, §4.4, §12.3 · **Size** S

Build
- Profile files; mod search paths; load order ties from the profile.
- The `engine` namespace and its synthesised placeholders.

Done when
- A profile with zero mods builds an empty, valid `Defs`.
- A dependency cycle names the mods in it.
- Two orderings of the same profile list give the specified, different,
  patch order.

### P4 — Typed loading and validation stages

**Spec** 04 §7.1, §7.2, §8.1, §9 · **Size** M

Build
- `Text` (templates compile in X4), `Inline<T>` with generated ids, enums,
  the remaining quantity types and dimension checking.
- The semantic and cross stages; a `validate` hook per definition type.
- The content fingerprint.
- Experimental marking (04 §8.4): features named on registered items; the
  `experimental` list in `mod.json`; the load error for use without
  opting in and the warning for use with it; the list recorded in saves.

Done when
- A unit of the wrong dimension is rejected with the field's expected
  dimension.
- Inline definitions receive stable ids and cannot be referenced.
- The fingerprint is identical across runs and changes when any finished
  tree changes.
- A fixture verb marked experimental is refused to a mod that has not opted
  in, with the feature named, and accepted with a warning from one that has.
- A test fails if any bundled mod names an experimental feature.

### P5 — Schemas, reference, scaffolding

**Spec** 04 §13 · **Size** M

Build
- Schema output per type and for data files; `--with-ids`.
- `clash-tools docs`: definition types and fields, units, diagnostic codes.
  The generator is driven by registries, so events, verbs, properties and
  functions appear as later packages register them.
- `clash-tools init-mod`.

Done when
- Generated schemas validate the bundled mods and reject each broken
  fixture.
- A test fails if a registered item has no documentation.
- A scaffolded mod lints clean.

### P6 — Settings

**Spec** 04 §10 · **Size** S

Build
- The `setting` definition type; storage in simulation state; the
  `setting()` function; the `Setting` command; `setting_changed`.

Done when
- A setting changed by command is saved, replayed from the input log, and
  visible to rules.
- A value outside a setting's range is refused.

### P7 — Migration and obsoletion

**Spec** 04 §12 · **Size** M

Build
- The `migration` definition type; chains; removal.
- `obsolete`; the warning for live references.
- Integration with the save-id remapping of W10.

Done when
- A save made before a rename loads with the new id.
- A chain of three migrations resolves in one step.
- An id with no definition and no migration becomes a placeholder and is
  restored when the definition returns.

### P8 — Hot reload

**Spec** 04 §11 · **Size** M

Build
- Reload as pipeline, in-memory save, swap, load, at a safe point.
- The console command; a file watcher in development builds of the tools.
- The input log marker.

Done when
- Scenario `reload_midfight`: a definition is changed during a running
  action and a pending timer; state is consistent afterwards and the
  specified consequences of 04 §11 hold.
- A reload with errors leaves the running definitions untouched.

## Expressions

### X1 — Expression parser

**Spec** 05 §4.1–4.6 · **Size** M

Build
- Crate `clash-expr`: lexer and parser producing a syntax tree with
  positions; unit literals; text templates.
- Syntax errors as diagnostics positioned inside the string.

Done when
- Every production of the grammar has passing and failing fixtures.
- Printing a parsed tree and reparsing it gives the same tree (property
  test).
- A syntax error inside a JSON string reports the right column of the file.

### X2 — Registry, contexts, type checking

**Spec** 05 §3, §5.3, §7.3, §14 steps 2–3 · **Size** L

Build
- The registry of subject kinds, properties, functions, events and verbs,
  each with types and documentation.
- Contexts: bound subjects, locals, event fields, purity.
- Name resolution and type checking, including `any`, `null` warnings, and
  compile-time resolution of id comparisons.

Done when
- Each kind of name error and type error has a fixture with the expected
  diagnostic and suggestion.
- A random function in a pure context is rejected.
- `x == 'glas'` against a material is a load error suggesting `glass`.

### X3 — Bytecode and evaluation

**Spec** 05 §4.3–4.7, §14 steps 4–5 · **Size** M

Build
- Constant folding; bytecode; the stack machine.
- Null semantics; run-time warnings with rate limiting.
- Built-in functions for maths, lists, text and types; random functions
  through a supplied stream.

Done when
- The machine agrees with a direct tree-walking evaluator on random
  expressions (property test).
- Every row of 05 §4.4 is tested.
- Benchmarks exist for the first row of 05 §19.

### X4 — Expression fields, functions, variables

**Spec** 05 §6, §7, §12; 04 §7.3 · **Size** M

Build
- `Expr<T>`, `Cond` and `Text` as definition field types with the compile
  stage.
- `function` and `variable` definition types; the undeclared-variable
  warning and `strict_vars`.
- The shape of value sources (queries arrive in K6).

Done when
- A definition with an expression field that refers to a subject outside
  its context fails to load, pointing at the name.
- Functions call functions; recursion past the depth limit is an error.
- A random function cannot be used where purity is required.

## Kernel

### K1 — Simulation skeleton and kernel entities

**Spec** 03 §4.1, §4.2 · **Size** M

Build
- Crate `clash-sim`: `Sim`, `EntityTable`, `KernelEntity`, `Place` with
  containment, `relocate`, RNG streams, `StateHash`, the `SimEvent` output.

Done when
- `relocate` keeps the world's occupant index consistent under random moves,
  containment changes and removals (property test).
- A contained entity is absent from the index and reports its container's
  location.
- Nothing outside `relocate` can change `place`.

### K2 — Advance, commands, input log

**Spec** 03 §4.3, §4.4, §5.2 · **Size** M

Build
- The advance loop with domain dispatch, safe points and all three limits.
- World processes running under it.
- `submit`, validation, `CommandError`, the input log, replay.
- The decision queue.

Done when
- Every world scenario from the world plan runs under `Sim::advance` with
  an unchanged hash.
- Replaying an input log reproduces the hash (invariant T6).
- `Work(n)` returns `Yielded` and resumes exactly.

### K3 — Events and hooks

**Spec** 05 §8; 03 §4.3 step 4 · **Size** M

Build
- Event type registration; the `event` definition type.
- The notification queue and pump; world events forwarded into it.
- Hooks with mutable fields and cancellation.
- Delivery order, `consume`, filter indexing, the cascade limit.

Done when
- Delivery order matches 05 §8.3 for global and attached handlers.
- A hook handler changes a field and the operation uses the changed value;
  a cancelled hook prevents it.
- Scenario `rule_cascade_limit`: a self-raising event is cut off, reported
  with a trace, and the simulation continues.

### K4 — Subjects, properties, variable storage

**Spec** 05 §5, §6 · **Size** L

Build
- Handles and subject kinds; the core properties; properties for
  everything listed under queries in 02 §23 and 03 §13.
- Variable storage on every scope, declared defaults, typed access, `watch`.

Done when
- Every property listed in those sections is registered, documented and
  tested.
- Property chains across kinds work (`self.loc.tract.devastation`).
- Setters go through the engine's mutation paths (shown by events emitted).
- A watched variable raises `var_changed` with old and new values.

### K5 — Rule runtime

**Spec** 05 §9, §10.1–10.3, §10.5, §13.1–13.3 · **Size** L

Build
- The `eoc` definition type and `Effects` field type; compiled effect
  trees.
- Global and attached rules; `as`, `priority`, `once`, `cooldown`,
  `skip_if_consumed`.
- Control flow, variable and rule effects; parameters; rules as verbs;
  `attach` and `detach`.
- All limits of 05 §13.2.

Done when
- Each effect of 05 §10.2, §10.3 and §10.5 has a test.
- A rule with parameters is callable as a verb and cannot see its caller's
  locals.
- Each limit, when exceeded, reports and continues.
- Scenario `rule_alarm`.

### K6 — Queries and value sources

**Spec** 05 §7.4, §11 · **Size** M

Build
- Every query kind of 05 §11 that the kernel can serve; `where`, `sort`,
  `limit`, `weight`; value sources; the extent cap.

Done when
- Query results match a brute-force reference and have the specified order
  (property test).
- An over-large constant query is a load error.

### K7 — Timers, continuations, simulation saves

**Spec** 05 §9.4, §10.4, §13.4; 03 §9 · **Size** L

Build
- Timers and continuations; `after`, `every`, `cancel_timer`; recurring
  rules with staggering and `while_unloaded`.
- Saving and loading of all simulation state through the W10 store.
- Save transparency in the scenario runner for simulation scenarios.

Done when
- Scenario `timer_survives_save`: timers and continuations fire identically
  with and without an intervening save.
- `Pause` and `Elapse` behave as 03 §9 specifies across an unload.
- A thousand owners of one recurring rule do not fire on the same tick.

### K8 — World verbs

**Spec** 02 §23 · **Size** M

Build
- A verb for each effect in 02 §23 that has an implemented world function.
- A list, in 02 §23, of any that are not yet bound and why.

Done when
- Each bound verb has a test showing the world event it produces.
- No verb mutates the world except through `WorldCtx`.

### K9 — Messages and choices

**Spec** 05 §10.6; 03 §5.2 · **Size** S

Build
- `message`; `ask` with `Choice` decisions and the `Choose` command.

Done when
- A choice offered to a directly controlled entity stops time until
  answered; the chosen option's effects run with the original subjects.
- A choice offered to another entity takes the first available option.

### K10 — Tracing, explain, profile, console

**Spec** 05 §15 · **Size** M

Build
- Trace, explain and profile; console evaluation of expressions and effect
  lists; expression assertions in scenario scripts.

Done when
- A trace of `rule_alarm` shows trigger, bindings, condition values,
  effects with arguments, and raised events.
- With tracing off, the rule benchmarks are unchanged.

## Time and actions

### T1 — Clock assignment

**Spec** 03 §3.4, §3.5 · **Size** M

Build
- Location clocks from areas; personal clocks; anchors.
- Rehoming on relocation; the effects `set_time_rate`, `anchor_clock`,
  `set_clock_rate`.

Done when
- Entering and leaving an area with a clock rehomes an entity's wake-ups
  with remaining local time preserved.
- An anchored entity is unaffected by the area it stands in.
- Overlapping areas resolve by priority, then id.

### T2 — Action definitions

**Spec** 03 §6.1–6.4, §7.1, §8 · **Size** M

Build
- The `channel`, `action` and `window_kind` definition types.
- Validation: phase ids, `next` targets, `closes` targets, parameter types,
  roles.

Done when
- Each validation failure has a fixture and a located diagnostic.
- The examples in 03 §6 and §8 load, with the names they borrow from later
  documents (`strike`, `auto_part`, `perceives`, `attr`, `takedown_result`)
  stubbed in the test fixture.

### T3 — Action runtime

**Spec** 03 §6 · **Size** L

Build
- Channel slots; action instances; phases, tempo, held phases, ticks,
  `next`; action locals.
- Start, finish, cancel, interrupt; events; the hooks `ActionStarting` and
  `PhaseTiming`.
- The action effects of 03 §13.

Done when
- Scenario `punch_timing`: phases begin at the expected ticks for several
  tempos and clock rates.
- Scenario `one_arm`: disabling a slot interrupts the action using it and
  blocks actions that need it.
- Scenario `time_stop_windup`: an action inside a rate-0 area freezes and
  resumes.
- Scenario `speedster_outruns_blast`: an entity with a fast personal clock
  relocates beyond a pressure wave's front before it arrives (needs W15).
- A zero-duration phase loop ends with an error after the transition limit.

### T4 — Control, plans, free decisions

**Spec** 03 §5 · **Size** M

Build
- Control modes; free decisions; plans; `Act`, `Wait`, `Cancel`,
  `ClearPlan`; `interrupt_plan`.

Done when
- Scenario `party_order`: simultaneous decisions arrive in entity id order
  and time does not move between them.
- A plan step whose `requires` fails clears the plan and requests a
  decision with the reason.
- A delegated entity accepts orders and is never asked.

### T5 — Windows and reactions

**Spec** 03 §7 · **Size** M

Build
- Windows; noticing with latency on the candidate's clock; closing.
- `react`, `ask_reaction`, `Reaction` decisions, `React` and `Pass`.
- Cancelling for a reaction; the `ready` phase and option timing.

Done when
- Scenario `too_fast_to_notice`: the same candidate notices a slow windup
  and does not notice a fast one; a candidate on a fast clock notices both.
- A candidate in a phase that does not allow cancelling has no reaction
  available.
- With no rule listening, nothing happens when a window is noticed.

### T6 — Joint actions

**Spec** 03 §8 · **Size** M

Build
- Roles; forced and consenting entry; shared phases; ending for all.

Done when
- Scenario `takedown_lock`: both participants' slots are held for the
  action's duration and released together.
- Interrupting either participant ends the action for both.
- A non-forced role produces a decision or a rule query before entry.

### T7 — Unloading and lower detail

**Spec** 03 §9–§11 · **Size** M

Build
- Abandoning actions and storing timers on unload; resuming on load.
- The warm stepper with a callback for the AI layer.
- Automatic promotion; `low_attention`.

Done when
- Invariant T9 is asserted on every unload.
- A warm entity named as the target of a hot entity's action is promoted
  before the action starts.
- A long `low_attention` action lowers and restores tickets.

### T8 — Benchmarks and budget gate

**Spec** 03 §15, 04 §18, 05 §19 · **Size** S, recurring

Build
- A benchmark for each row of those tables as packages land; thresholds in
  the gate introduced by W23; measured figures written back into the
  documents.

## Scenario catalogue

Under `scenarios/kernel/`, run in all modes of the scenario runner.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `rule_alarm` | K5, K8 | S12, S18 | Event to rule to world verb, variables, queries |
| `rule_cascade_limit` | K3 | — | Runaway rules are contained |
| `timer_survives_save` | K7 | — | Continuations across save and load |
| `punch_timing` | T3 | S1 | Phases, tempo, clock rate |
| `one_arm` | T3 | S14 | Channel loss |
| `time_stop_windup` | T3, T1 | S10 | Actions on area clocks |
| `speedster_outruns_blast` | T3 | S3 | Personal clock against a world process |
| `party_order` | T4 | — | Several directly controlled entities |
| `too_fast_to_notice` | T5 | S14, S19 | Notice latency; the gap is real |
| `takedown_lock` | T6 | S6 | Joint action |
| `reload_midfight` | P8 | S12 | Hot reload under load |
| `winter_rules` | K5 | — | Rule groups |
| `bodyguard` | T5 | — | Who is offered a window |
| `who_did_it` | K9, X4 | — | Text for a reader |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Disputed fields | P2 | 04 §6.3 | Two mods replacing one field with different values give a warning naming both, the field and the winner. The warning is absent when the later mod lists the earlier in `requires` or `after`. Two mods that `scale` or `extend` the same field give none. |
| Maps as values | X2, X4 | 05 §3, §5 | `map<T>` type-checks; `keys`, `values`, `has` and indexing by key work; iteration order is by key, shown by a state hash that does not vary with insertion order. |
| Returning from rules | K5, K6 | 05 §9.5 | A rule's `return` value is taken by a `run` value source in an effect list. A rule is still refused in an expression, with a diagnostic. |
| Rule groups | K5 | 05 §9.6 | Test script `winter_rules`: rules in a group do not run while it is off; a setting that its `enabled` reads turns it on; `set_rule_group` switches it from a rule; the state survives save and load. |
| Who is offered a window | T5 | 03 §7.1; 15 §9.1 | Test script `bodyguard`: a blow aimed at one character opens a window for another who holds an `intervenes` relation toward them and is within `reach`, and a reaction flagged `on_behalf` is offered. A holder beyond reach is offered none. A directly controlled character within reach is offered one. Until first-playable package S7 exists the relation is supplied by a test fixture. |
| Footing | T2 | 03 §6 | An action whose `footing` is true is refused, with a reason, for an actor with nothing under it. One without it runs. |
| Text for a reader | K9, X4 | 05 §4.6 | Test script `who_did_it`: one event's log line reads "You…" to the one who did it, names a known person by name to a second reader, and describes a stranger by their features to a third. The grammar helpers have unit tests against the bundled `language`. Reading another's `.name` in a message is a lint warning. Until first-playable packages S5 and S6 exist, what a reader knows is supplied by a test fixture. |
| List parameters | T2 | 03 §6.3 | A parameter of type `list` accepts between its least and greatest number of values of its element type, is indexable in the action's expressions, and survives save and load mid-action. `check` is evaluated on request and never refuses by itself. |
| What presentation is given | K2 | 03 §12 | Every event carries the universal tick, the local time of its clock, its subjects and its cause; a decision request carries the time elapsed for that entity; phase events carry their start and scheduled end. |
| Total conversions | P3 | 04 §14 | A profile that lists none of the bundled mods starts at its own boot screen, and a frame test shows nothing of the engine's is drawn before it. |
| Safety limits | P4 | 04 §15 | A file over the size limit, nesting over the depth limit and a definition count over its limit each give a diagnostic and no crash. A file reference that leaves the mod's directory is refused. |
| Rules at lower detail | K5, T7 | 05 §13.5 | Attached rules on a stored entity do not run; recurring ones follow `while_unloaded`; nothing cold raises a cell-level event. |
| The script hook | none | 05 §16 | Reserved. No package builds it, and nothing may depend on it. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
