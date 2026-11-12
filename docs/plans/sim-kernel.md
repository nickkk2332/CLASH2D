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
| P1 | Diagnostics | M | F4 | done |
| P2 | Complete merge semantics | M | F4 | done |
| P3 | Profiles and running without mods | S | F4 | beta |
| P4 | Typed loading and validation stages | M | P2 | beta |
| P5 | Schemas, reference, scaffolding | M | P1, P4 | todo |
| P6 | Settings | S | K5 | todo |
| P7 | Migration and obsoletion | M | P4, W10 | todo |
| P8 | Hot reload | M | K7, P7 | todo |
| X1 | Expression parser | M | F2 | done |
| X2 | Registry, contexts, type checking | L | X1, F4 | done |
| X3 | Bytecode and evaluation | M | X2 | beta |
| X4 | Expression fields, functions, variables | M | X3, P4 | beta |
| K1 | Simulation skeleton and kernel entities | M | W7, F3 | done |
| K2 | Advance, commands, input log | M | K1, W9 | beta |
| K3 | Events and hooks | M | K2, X2 | beta |
| K4 | Subjects, properties, variable storage | L | K1, X3 | beta |
| K5 | Rule runtime | L | K3, K4, X4 | beta |
| K6 | Queries and value sources | M | K5 | beta |
| K7 | Timers, continuations, simulation saves | L | K5, W10 | done |
| K8 | World verbs | M | K5 | todo |
| K9 | Messages and choices | S | K5 | beta |
| K10 | Tracing, explain, profile, console | M | K5, W11 | beta |
| T1 | Clock assignment | M | K1 | beta |
| T2 | Action definitions | M | X4 | beta |
| T3 | Action runtime | L | T2, K5 | beta |
| T4 | Control, plans, free decisions | M | T3 | beta |
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

As built (decision D-267). Built with F4, so there was no older error form
to convert. A code cannot be declared without a title and an explanation:
they are arguments of the macro that declares it, and a test checks that
none is empty. A crate that adds a stage keeps a table of its own;
`clash-tools explain` reads every table. The `06` group of codes is left
for the expression packages.

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

As built (decisions D-262, D-264). Every value of a finished tree keeps
the span it was last written at, which is all `show` needs; a value
changed by `scale` or `add` is attributed to the patch that changed it.
The disputed-field warning of the review additions is built here.

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

As built (decisions D-264, D-265). Beta: the three items above are built
and tested, with the engine's own definitions (`engine:none`) and the
placeholders of lenient loading. Stubbed: the frame test of the "Total
conversions" review addition, which needs the scene layer; what is tested
here is that a profile listing none of the bundled mods loads only its own
and carries its boot screen. Placeholders for ids that a save names and no
mod defines (04 §12.3) come with package P7. A profile's `boot_screen`,
`settings`, `screens` and `preferences` are kept as written for the crates
that own screens and settings.

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

As built (decisions D-260, D-261, D-263, D-266). Beta: every item above is
built and tested, with the safety limits of the review additions. The
fixture verb is a rule's effect that the fixture type reports through
`FinishCx::use_experimental`, which is the call a real verb will make; a
whole type (`Def::EXPERIMENTAL`) and a field (`Gated`) can be marked too.
Stubbed: `Text` holds its string and where it was written, and its
templates are compiled by X4. An inline definition given an `id` of its
own can be referred to but not yet patched from another mod. The list of
experimental features is on `Defs` for the save format to record. A
feature named in `mod.json` that nothing registers is not reported.

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

Built: `clash-expr` reports a span of bytes inside the source string and
offers `offset_in_json_string` to carry it into a file; turning that into
a file, line and column in a diagnostic is the data layer's (P1). Inline
style tags (12 §4.3) pass through the template parser as text.

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

Built with it: the mind context of 05 §7.3 (decision D-273), declared
variables in the registry with the undeclared-variable warning that X4
switches by `strict_vars`, places for `set` and `add` (decision D-275),
the map type of the review addition, and a test host under
`clash_expr::testing`. Left for others: the registry holds what makes a
property or function stale, and the check that a reactive expression uses
only such entries is abilities package A1; the space, time and data
functions of 05 §4.7 are registered by the simulation (K4).

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

Beta: everything above is built except the benchmark. In its place the
ignored test `time_per_evaluation` in `crates/clash-expr/tests/machine.rs`
prints the time per evaluation, and the figures are written into 05 §19;
the criterion benchmark and its gate are T8. The reader's text helpers of
05 §4.6 and `exists` are registered with their signatures and answered by
the host, so they do nothing until the simulation supplies them (K9, K4).

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

As built (decisions D-330 to D-336), in `clash_data::expr`. Beta: every
item above is built and tested in `crates/clash-data/tests/expr.rs`, with
the map variables of the review addition, the mind context, the
conversion of the compiler's diagnostics to group `06` and of run-time
warnings to their definition and JSON path. A definition type states the
scope of each field in `Def::compile`; the registry is given to the
loader with `Loader::expr_registry`; a crate adds what its own
definitions declare with `DefTypes::add_registry_step`. Stubbed: queries,
so a value source over a query, `{ "exists": query }` and `{ "run": … }`
are read, kept and give null with a run-time warning (K6); `Effects`
fields (K5); the reactive-context check (abilities A1). Which queries a
character may make is not decided: a function whose value is a query
counts as not safe for belief. The test script `who_did_it` and the lint
warning for another's `.name` in a message are K9's. `function` and
`variable` have no placeholder, so a lenient load stops at a broken one.
The application and the tools must call `clash_data::expr::register_defs`
beside the other crates' functions of that name.

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

As built (decisions D-360 to D-363, D-366, D-367, D-369). Everything above,
with entity creation and removal, the `Reason` type the whole simulation
uses, saving and loading of the simulation's own state, and `reindex`,
which rebuilds the world's occupant index after a load. The world, the
timeline and the entity table are read through `Sim::truth()`. Action
table, window table, rule state, channel set and plan are empty typed
placeholders. Tests are in `crates/clash-sim/tests/kernel.rs`; that
nothing outside `relocate` can change `place` is a privacy argument, with
two `compile_fail` examples in `clash_sim::entity`. Not built: portal
events, which wait for the world to have portals (W19). There is no test
script under `scenarios/` until the runner exists (W11).

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

As built (decisions D-364, D-365). Beta: the loop with its three limits,
handlers by wake-up domain, the pump, commands with reasons, the input
log, replay and the decision queue are built and tested
(`crates/clash-sim/tests/advance.rs`), with the review addition "What
presentation is given" except phase events, which are T3's. Replay, the
exact resumption of `Work(n)` and saving in the middle of a session are
property tests over random sessions. Stubbed: no world scenario exists
to run under `advance` until the runner does (W11); a world wake-up is
shown running under the loop by a test's own handler, and the `World`
domain's default handler does nothing until the world has a process
(W13). `reconcile_activity` at the safe point does nothing until tickets
(W9) are joined. The commands are the debug commands; the rest arrive
with their packages. Past 100,000 wake-ups at one tick `advance` reports
and yields rather than putting the rest off to the next tick. The log
does not carry state hashes at intervals.

Joined since (decisions D-460 to D-462), tested in
`crates/clash-sim/tests/handoffs.rs`: the `World` domain's handler calls
the world's `handle_wake`, so a tower knocked out by debug commands comes
down under `advance`, and a save in the middle with the world's process
state in the blob ends in the same hash; the safe point reconciles
tickets, shelves and restores the entities of tracts that unload and
load, and reports spawn requests; `Sim::start_session` and the debug
command of that name start a session at a place that is not loaded yet.
Still stubbed: world scenarios as test scripts (W11); timers of a
shelved entity (T7); the cosmos has no store of its own, so a test of
saving copies it (W10).

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

As built (decision D-368). Beta: everything in the list is built and
tested (`crates/clash-sim/tests/events.rs`), with handlers as Rust
closures registered through `Sim::on` and `Sim::on_subject`, which the
rule runtime (K5) will use for rules. Stubbed: `rule_cascade_limit` is a
test of that name, not yet a test script, until the runner exists. Event
types are kept by the simulation (`Sim::event_types`); K4 enters them in
the expression registry.
A field's value is null, a plain value or a subject. `clash-tools` must
call `clash_sim::register_defs` for the linter to know the `event` type.

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

As built (decisions D-463 to D-469). Beta: the kinds `entity`, `loc`,
`element`, `area`, `site`, `tract`, `realm` and `global` with their
handles; `clash_sim::engine_registry()` and the loader's registry step;
about a hundred properties and nine functions, with `exists`, each
documented, tested in
`crates/clash-sim/tests/subjects.rs` and stated in 05 §5.3 and §17,
02 §23, 03 §13 and 06 §13; eight settable properties, each through the
engine's own function; the host (`Sim::host`, `Sim::with_random`); the
write side for the rule runtime (`Sim::assign`, `Sim::set_var`,
`Sim::set_property`); variables on every scope the kernel serves, with
declared defaults, types and `watch`; belief marks; the kernel's, the
world's and mods' events as event types of the expression registry.
`clash-tools lint` loads with the real registry, and `mods/core` holds a
variable and three functions that use real properties. Stubbed, each
listed as a gap in its document: a field's value, ambient temperature,
drift and decals (02 §23); the queries over actions, channels, plans and
windows (03 §13); properties of a definition reached through an id, and
the scopes `region`, `subgrid`, `action`, `scenario` and `legacy`
(05 §17); `templates` and names taken from definitions (06 §13); the
reader's text helpers, which answer in plain forms (K9). No test script,
until expressions can be asserted from one (K10), and no benchmark.

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

As built (decisions D-470 to D-479), in `clash_sim::rule`. Beta: every
item above is built and tested in `crates/clash-sim/tests/rules.rs`, with
fixtures under `crates/clash-sim/tests/fixtures/rule/`, against a host
trait, `RuleHost`, and a fake host, as the console was. Built: the `eoc`,
`rule_group` and `reason` definition types; the `Effects` field type,
which action definitions now use; compiled effect trees for every effect
of 05 §10.2, §10.3 and §10.5, `queue` among them; verbs checked against
their `VerbSpec`; rules with parameters as verbs; the interpreter with an
explicit stack and the limits of 05 §13.2; `RuleState` (attachments, run
records, switched groups) with save and hash; `RuleSet::deliver`, which
runs what answers an event in the order of 05 §8.3. The review additions
"Returning from rules", "Rule groups" and the attached half of "Rules at
lower detail" are built; the bodies of the test scripts `rule_alarm` and
`winter_rules` are tests of those names. The binding to `Sim` is built
(decision D-540): the simulation is the rule host, one bus handler per
event type a rule listens for, verbs by registry id from
`engine_verbs()`, faults as thinned `fault` events, the save layout at
version 5 with the action table and the stores of items; `crates/clash-sim/tests/live_rules.rs` runs `rule_alarm` and
`winter_rules` against the real simulation from fixture mods, across a
save and a replay, and the kernel's verbs (D-541). Waits: rules carried
by definitions (`rules_of`) until a definition type of the simulation
carries `rules`; timers and continuations, `after`,
`every`, `cancel_timer` and a rule's own `every`, which are read, kept
and report when they run (K7); queries, so `for_each` over one reports
and a value source over one gives null (K6); the world's verbs (K8);
`message` and `ask` (K9); tracing (K10). The two test scripts become
`.scn` files once the simulation implements the console's host. No
benchmark.

### K6 — Queries and value sources

**Spec** 05 §7.4, §11 · **Size** M

Build
- Every query kind of 05 §11 that the kernel can serve; `where`, `sort`,
  `limit`, `weight`; value sources; the extent cap.

Done when
- Query results match a brute-force reference and have the specified order
  (property test).
- An over-large constant query is a load error.

Beta (decisions D-680 to D-683). Built: queries compiled where they are
written by `clash_data::compile_query`, with `where`, `sort`, `limit`,
`it`, and the value sources `count`, `collect`, `first`, `pick` (with
`weight`), `sum`, `min`, `max` (with `of`), `{ "exists": query }` and
`for_each` over a query; the kinds `list` and `defs`, and the
simulation's `entities`, `cells`, `contents`, `areas` and `sites`
(`clash_sim::query`), registered as `QuerySpec`s and answered through
`Host::query`; the extent cap `rules.max_query_cells` at load and when
running. `crates/clash-sim/tests/queries.rs` covers every kind and
source, the load errors and the cap, and holds `entities` and `cells`
to a brute-force reference (property tests). A `run` source taken by a
`let` was built with K5 ("Returning from rules"). Stubbed: the shapes
`line` and `cone`, the kinds `actions`, `windows` and `parts`; no query
is answered from belief, so a mind context may write only `list` and
`defs`. Perception landed on the build branch while this was built:
answering `entities` from the observer's tracks (`perception::Beliefs`)
is the next step, through the seam `QuerySpec::safe` and
`QueryCall::observer` (D-683), with the filters that read truth of
others (`tag`, `def`, `has`, `kind`, `plane`) refused in a mind
context. No benchmark.

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

Done. The saving part: `clash-save` saves the whole game (the world
through the W10 store and the simulation's own blob, with metadata) into
one file and loads it back, invisibly (01 §11.2, D-580), and the
scenario runner's save-and-load mode runs through it for every script.
Timers (decisions D-684 to D-686): `after`, `every`, `cancel_timer` and
`owner.timer(name)`, kept as `TimerRecord`s with continuations in the
rule state (save layout 7), on the owner's clock, wake-ups of the `Rule`
domain; a rule's own `every`, global ones from the start and attached
ones from `attach_rule`, staggered by a positional hash of owner and
rule; `while_unloaded` (`pause`, `elapse`, `all`, `skip`, `once`) as an
entity leaves and re-enters the world. `crates/clash-sim/tests/timers.rs`
holds the body of `timer_survives_save`, `Pause` and `Elapse` across
leaving the world, a slowed area, and the thousand owners; the test
script `scenarios/rules/timer_survives_save.scn` runs in the four modes.
Rules carried by definitions start no `every` until a definition type of
the simulation carries `rules`. No benchmark: the budget "save with 50
dirty tracts" (02 §25) is measured by W23.

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

As built (decision D-542). Beta: both items are built and tested in
`crates/clash-sim/tests/messages.rs` with the fixture mod `talk` under
`crates/clash-sim/tests/fixtures/act/mods/`: `message` and `ask` are
effects of the rule compiler, a kept choice is plain rule state that
survives a save and a replay, `Choose` answers it, and anyone not
directly controlled takes the first available option at once. Of the
review addition "Text for a reader", the K9 half is built as the test
`who_did_it`, with what a reader knows supplied by a fixture lens
(`Sim::set_reader_lens`) and the grammar helpers in `clash_sim::reader`
with unit tests. Stubbed: the `language` definition type (the English
forms are built in), the lint warning for reading another's `.name` in
a message (no such property is registered yet), and the `.scn` form of
`who_did_it`, which waits for the simulation to implement the console's
host.

### K10 — Tracing, explain, profile, console

**Spec** 05 §15 · **Size** M

Build
- Trace, explain and profile; console evaluation of expressions and effect
  lists; expression assertions in scenario scripts.

Done when
- A trace of `rule_alarm` shows trigger, bindings, condition values,
  effects with arguments, and raised events.
- With tracing off, the rule benchmarks are unchanged.

Beta: the console part is built. The console's host evaluates expressions
against the simulation with `self` bound (the subject given, else the
active character), and test scripts assert expressions (`assert`,
`until`). Effect lists (`do`, `run`, verbs) reach one function of the host,
`GameHost::run_verb`, which carries them out through the rule runtime
(D-687); the test scripts under `scenarios/rules/` use it. Not built:
traces, `explain`, `profile`; both "Done when" items wait for them.

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

As built (decision D-363). Beta: clock assignment, rehoming and the three
operations are built and tested (`crates/clash-sim/tests/clocks.rs`).
Stubbed: `set_time_rate`, `anchor_clock` and `set_clock_rate` are kernel
functions; their effects are bound by the rule runtime (K5). Since K4,
an entity's `time_rate` and an area's and a realm's `clock_rate` are
settable properties that call the first and the last. Bodies in flight
and world processes are rehomed by the world, when it has them.

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

As built (decisions D-400 to D-407), in `clash_sim::action`. Beta: both
items above are built and tested in `crates/clash-sim/tests/action.rs`,
with fixtures under `crates/clash-sim/tests/fixtures/action/`. A loaded
action is read without a lookup by name: phases by index, channels as a
`ChannelSet`, locals in one fixed order of slots that every expression was
compiled against. The reasons for the refusals of 03 §6 to §8 are
constants in `clash_sim::action::reason`. `clash_sim::action::register_defs`
registers the three types and is called by `clash-tools`. Effect lists
are `Effects` fields of the rule runtime (K5), compiled in
`ActionDef::effect_scope` with the action's locals. Of the review
additions, the definition halves are built: `footing`
with its reason, and list parameters with their bounds, element type and
indexing. The run-time halves are T3's: refusing an actor with nothing
under it, holding a list to its bounds when a command gives it, and
keeping it across a save. The events, hooks, effects and queries of 03 §13
are registered by the packages that raise and run them. The fields other
documents add to an action (`attacks`, `defence`, `cutscene`, the `choice`
parameter type) are not accepted yet.

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

As built (decisions D-543, D-545). Beta: tested in
`crates/clash-sim/tests/acting.rs` with the fixture mod `acts` under
`crates/clash-sim/tests/fixtures/act/mods/`: `punch_timing` for five
tempos and clock rates, `one_arm`, `time_stop_windup`, the transition
limit, held phases with ticks, `until` and release, cancel rules and a
follow-up breaking off a recovery, every refusal with its reason and no
change of state, `footing`, `requires_at_start`, both hooks, list
parameters across a save (the review additions "Footing" and "List
parameters"), abandoning on leaving the world, an `ask` in an action's
list answered after a save, every action verb, and a save and a replay
in the middle of actions. Stubbed: `speedster_outruns_blast` (needs
W15); slots are counted per channel, so a `part` parameter does not name
a slot; a joint action claims only its first role's slots and offers no
roles (T6); a phase's window is raised as `window_opened` and offered to
nobody (T5); the queries of 03 §13 are not registered as properties;
the test scripts are tests until the simulation implements the
console's host; no benchmark.

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

As built (decisions D-544, D-545). Beta: tested in
`crates/clash-sim/tests/acting.rs`: a character starts a two-phase action
from one command and is asked again exactly when it ends, while a
second, on a clock twice as fast and paced by a rule standing in for the
AI, interleaves; `party_order`; a plan step whose `requires` fails clears
the plan and asks with the reason; a delegated character takes orders,
is never asked and idles while something listens for
`decision_needed`; `Wait`; commands to an entity nobody controls are
refused. Stubbed: the AI that answers `decision_needed` (09); plans are
not handed to the cold simulation (T7).

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
