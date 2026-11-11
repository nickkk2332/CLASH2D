# Plan — Tooling

Implements [14-tooling.md](../14-tooling.md).

Packages prefixed W are in [world-model.md](world-model.md); X, K, P and T
in [sim-kernel.md](sim-kernel.md); E, S and U in
[first-playable.md](first-playable.md); C in [combat.md](combat.md); G in
[devices.md](devices.md); A in [abilities.md](abilities.md); I and L in
[ai.md](ai.md); WG in [worldgen.md](worldgen.md); SC in
[scenarios.md](scenarios.md). The rules for taking a package are those of
[world-model.md](world-model.md) and [CLAUDE.md](../../CLAUDE.md).

Tooling is not a stage that comes last. Each package here should be built
as soon as its dependencies allow, because every other package is easier to
build and to check with it. TL1, TL3, TL5, TL6 and TL7 in particular pay
for themselves at once.

Several packages here generalise something an earlier plan built in a
first form: the console and runner of W11, the tracing of K10, the
diagnostics and reference of P1 and P5, the per-plan benchmark gates. Each
says which.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| TL1 | Console | M | W11, K10 | todo |
| TL2 | Reasons | M | K2, T3 | todo |
| TL3 | Traces | M | K10 | todo |
| TL4 | Inspector | L | U6b, TL3, K4 | todo |
| TL5 | Profiler and the budgets file | M | K2, W23 | todo |
| TL6 | Test scripts | M | TL1, U1 | todo |
| TL7 | Invariant checkers | M | TL6 | todo |
| TL8 | Replay, divergence, diff, rewind | M | K7, TL6 | todo |
| TL9 | Mod authoring suite | M | P5, P8, U6b | todo |
| TL10 | Map editor and capture | L | WG2, U7 | todo |
| TL11 | Comparing incident forms | M | L3, I10 | todo |
| TL12 | Player aids | M | SC3, SC6, A9 | todo |
| TL13 | Save tools | S | K7, P7 | todo |
| TL14 | Matchup and compat reports | S | C11, P4 | todo |
| TL15 | Stepping through a rule | S | TL1, P8 | todo |

## Order and parallelism

Packages are listed with the earliest milestone of the other plans at which
they can be built.

| Earliest | Packages |
|----------|----------|
| M2, with the kernel | TL1, TL2, TL3, TL5, TL13 |
| M2, after TL1 | TL6, then TL7, TL8 |
| M3, with screens | TL4, TL9 |
| M3, with combat | TL14 |
| Any time after hot reload | TL15 |
| M4, with the layered simulation | TL11 |
| M5, with maps and scenarios | TL10, TL12 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **Tools for building** | TL1, TL3, TL5, TL6, TL7, TL8 | One console in the game, in the terminal and in test files. A trace of a rule and of a strike. Every budget in every specification measured in one report. A test script run four ways. Every invariant checked on demand. A session replayed from its log to the tick where two runs first differ, and the difference shown. |
| **Tools for modding** | TL2, TL4, TL9, TL13, TL14 | Point at anything and read every property it has, with documentation, and follow it to what it is connected to. Ask why an action is unavailable and be told which cost is short by how much. Change a file and see it take effect, or see the error over the game. See which of two mods wins a disputed field. See that nothing in a mod can hurt its own boss. |
| **Tools for making worlds** | TL10, TL11 | Paint a house on a grid and save it as a map. Build a base in play and capture it as a design. Run a robbery two hundred times each way and see where the abstract arithmetic disagrees with what happens when it is played. |
| **Aids for players** | TL12 | A case file that groups what the character has found, flags what cannot both be true, and accuses from a list of suspects. |

## Work packages

### TL1 — Console

**Spec** 14 §3 · **Size** M · Generalises W11's console and K10's evaluation

Build
- The language of 14 §3.2: arguments, values, references, `as`.
- Registration of console commands alongside verbs, with generated help;
  the commands of 14 §3.3.
- The three hosts: game overlay, terminal viewer, file.
- Changes as `Debug` commands; the profile switch.
- The `console_command` type.

Done when
- Every registered verb has a console command without further code.
- A console session saved as a file replays to the same hash.
- A command from a mod runs its rule with its parameters.

### TL2 — Reasons

**Spec** 14 §5 · **Size** M

Build
- The reason type and a registry of codes with documentation.
- Reasons on every refusal listed in 14 §5, in whatever subsystems exist
  when this is built; a test that fails on a refusal path without one, to
  hold later packages to it.
- `explain` for actions, recipes and jobs.

Done when
- Each row of 14 §5 that can be exercised has a test script using
  `expect reason`.
- Reasons appear in the generated reference (invariant Y3).

### TL3 — Traces

**Spec** 14 §6.1 · **Size** M · Generalises K10's rule trace

Build
- The record type; kinds; switching by kind, subject and place; sinks to
  panel, file and tests; adoption by each subsystem present.

Done when
- With tracing off, benchmark results and state hashes are unchanged
  (invariant Y2).
- A trace of a strike shows the steps of 07 §3.4 with their numbers.
- Custom events are traceable without code.

### TL4 — Inspector

**Spec** 14 §4 · **Size** L

Build
- Debug-context screens: picking; the generated property listing with
  following, editing and pinning; the why-views of 14 §4.3 for whatever
  subsystems exist; the four views of 14 §4.4.

Done when
- A property registered by a test fixture appears in the inspector with no
  inspector change.
- An edit made in the inspector is a logged command.
- A split view shows a disguised character as one persona on one side and
  another on the other.

### TL5 — Profiler and the budgets file

**Spec** 14 §6.2, §6.3 · **Size** M · Unifies the per-plan budget gates

Build
- Time by subsystem and by rule; counts; the live budgets view.
- `budgets.toml` holding every budget row of every specification, each
  with its benchmark and limit; `cargo xtask bench --check` over all of
  them.

Done when
- Every row of every specification's budget table is in the file, checked
  by a test that reads the documents.
- A budget exceeded by more than its margin fails the gate.

### TL6 — Test scripts

**Spec** 14 §7.1, §7.2 · **Size** M · Generalises W11's runner

Build
- The expectations of 14 §7.1, including traces, reasons and frames.
- The four run modes; `--bless`; `--paranoid`; scripts shipped by mods.

Done when
- A script that passes straight and fails under save-and-load is reported
  as such, naming the split point.
- A mod's own scripts run from its directory.

### TL7 — Invariant checkers

**Spec** 14 §7.3 · **Size** M

Build
- A registry of checkers keyed by invariant id; `check`; paranoid mode.
- A test that reads every specification's invariant table and fails for
  any id with neither a checker nor an entry naming the test that covers
  it.

Done when
- Deliberately corrupting state in a fixture makes the matching checker
  report it.
- The coverage test passes for every specification.

### TL8 — Replay, divergence, diff, rewind

**Spec** 14 §8 · **Size** M

Build
- Hashes at intervals in the input log; `replay`; stopping at the first
  differing interval with the subsystem named; `diff` of two states;
  `report`.
- Periodic snapshots in development builds and `rewind`. The same
  snapshot-and-replay machinery serves the experimental `rewind_world` (scenarios package
  SC11), so it must not depend on anything only a development build has.

Done when
- A fixture with a deliberate nondeterminism is caught at the right
  interval and subsystem (invariant Y6).
- `diff` names the entity or cell that differs.
- After `rewind` and `advance` the hash equals the original at that tick.

### TL9 — Mod authoring suite

**Spec** 14 §9.1, §9.2 · **Size** M · Extends P5 and P8

Build
- Errors shown over the game on a failed reload; the reference browser
  screen; `refs` and `find`.

Done when
- A broken edit leaves the game running on the old definitions with the
  diagnostic displayed.
- `refs` finds every reference to a definition in the bundled mods.

### TL10 — Map editor and capture

**Spec** 14 §10.2, §10.3 · **Size** L

Build
- The editor screen: canvas, palette, tools, layers, parameters, nested
  maps, live checks.
- Reading and writing the map format with stable formatting.
- `capture_map`.

Done when
- A map written by the editor loads, and loading and writing it again
  changes nothing (invariant Y7).
- A structure built in play and captured can be placed by generation and
  planned as jobs, and the result matches the original.

### TL11 — Comparing incident forms

**Spec** 14 §11 · **Size** M

Build
- `incident compare`: setting up, running live without a display and
  abstractly, many times; measures, spreads, tolerance, report.
- The same for bulk against full fighting.

Done when
- A fixture incident whose abstract form is deliberately too lenient is
  flagged on the right measure.
- The bundled incidents pass at the stated tolerance, or their differences
  are recorded as accepted.

### TL12 — Player aids

**Spec** 14 §13 · **Size** M

Build
- Journal, case file, codex and look as ordinary-context screens in the
  `ui` mod.

Done when
- Nothing in them reads what the character does not know, enforced at load
  (invariant Y8).
- Two findings that cannot both be true are flagged.
- A suspect marked in the case file can be accused from it.

### TL13 — Save tools

**Spec** 14 §12 · **Size** S

Done when
- `save verify` fails on a save with a corrupted tract and names it.
- `save migrate` produces a save that loads under renamed definitions.

### TL14 — Matchup and compat reports

**Spec** 14 §9.3, §9.4 · **Size** S

Done when
- `matchup` between two fixture creatures prints previews for every attack
  against every part, both ways, and flags a creature harmed by its own
  blow.
- `compat` reports two mods patching one field to different values and
  names the winner.

### TL15 — Stepping through a rule

**Spec** 14 §6.4 · **Size** S · Added by the review. Nothing depends on it.

Build
- `break`, `next`, `continue`, `break off`; the in-memory save taken at a
  break; running the rule from it up to a given effect; the console view of
  bindings and the next effect.

Done when
- A break stops before the rule's first effect and shows its bindings.
- After any number of `next` and then `continue`, the state hash equals
  that of the same run with no break set.
- Breaks are unavailable outside development profiles and absent from the
  input log.

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Sheet view | TL10 | 14 §10.2 | A map made of placed maps is shown whole; a piece can be moved and turned, and opening it puts it on the canvas. May follow the rest of TL10. |
| Stepping through a rule | TL15 | 14 §6.4 | See the package. |
| Map and generation previews | worldgen WG12 | 14 §10.1; 10 §12 | Built with worldgen package WG12; listed here so that every section of 14 has an owner. |
| Logs and failures | TL3 | 14 §14 | The structured log carries time, tick, subsystem, severity and fields; rule warnings are rate-limited per site; a simulated panic is caught at the top and writes a report. |
| Reasons from mods; interface commands; reference coverage | TL1, TL2, TL9 | 14 §3.3, §9.2, §15 | A mod's `reason` is carried by its `cancel_event` and listed in the reference. A script presses a key, opens a screen and compares a frame. The reference lists every hook with what a handler may change and every verb with its arguments, and a test fails for any registered without them. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
