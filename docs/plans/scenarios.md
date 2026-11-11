# Plan — Scenarios, Modes and Events

Implements [13-scenarios-and-events.md](../13-scenarios-and-events.md).

Packages prefixed W are in [world-model.md](world-model.md); X, K, P and T
in [sim-kernel.md](sim-kernel.md); E, S and U in
[first-playable.md](first-playable.md); G in [devices.md](devices.md); A in
[abilities.md](abilities.md); I and L in [ai.md](ai.md); WG in
[worldgen.md](worldgen.md); V in [vehicles.md](vehicles.md). The rules for
taking a package are those of [world-model.md](world-model.md) and
[CLAUDE.md](../../CLAUDE.md).

In this plan "scenario" means the definition type of document 13. The
regression tests each package adds are called test scripts here.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| SC1 | Scenario definitions and starting a game | M | WG6, K5, P6 | todo |
| SC2 | Characters at the start | M | SC1, E9, U9, L2 | todo |
| SC3 | Objectives and missions | M | SC1, A1, L4 | todo |
| SC4 | Plots | S | SC3 | todo |
| SC5 | The director | M | SC1, L3 | todo |
| SC6 | Offences, cases and findings | L | L4, S6, G1 | todo |
| SC7 | Saving, loss of the party, endings | M | SC1, U11 | todo |
| SC8 | Legacy, roster, campaigns | M | SC7 | todo |
| SC9 | Bundled scenarios | L | SC2–SC8, WG12, I10 | todo |
| SC10 | Benchmarks and budget gate | S | SC1, then recurring | todo |
| SC11 | Turning back the whole world (experimental) | M | SC7, TL8, P4 | todo |

## Order and parallelism

| Wave | Packages |
|------|----------|
| 1 | SC1, SC6 |
| 2 | SC2, SC3, SC5, SC7 |
| 3 | SC4, SC8, SC11 |
| 4 | SC9 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M5d A game to start** | SC1, SC2, SC7 | The new-game screen lists scenarios. One offers character creation and a choice of two starting places; another drops the player into the life of a random townsperson. A single-save scenario removes its save when the character dies; another lets the player carry on as someone else. |
| **M5e Things to do** | SC3–SC5 | A bounty read from a notice board becomes objectives with a marker at the quarry's last known position. A plot's beats fire in order. With the steady director, something happens nearby after a lull and nothing happens while tension is high. |
| **M5f Who did it** | SC6 | A killing leaves marks, a wound and witnesses. Examining them yields findings that depend on who is looking. Accusing the face the witnesses saw closes the case on the wrong person. Done by the player instead, the watch opens a case, gathers findings, and comes looking. |
| **M5g Games between games** | SC8, SC9 | A roguelike run remembers what earlier runs unlocked. A character kept from one game starts another. A campaign's second chapter begins with the first chapter's party. Each reference world ships at least a sandbox and one other mode. |

## Work packages

### SC1 — Scenario definitions and starting a game

**Spec** 13 §3, §4.1, §4.3, §4.4, §12 · **Size** M

Build
- The `scenario` type; scenario state in the simulation, saved.
- Creating worlds with seeds fixed, random or asked; placing at a site
  role, an entry or coordinates, with a choice; setting the calendar;
  `start.do`.
- Scenario rules in force only in that scenario; `settings`, `locked`,
  `options`.
- The new-game flow of 13 §12 driven by data.

Done when
- Test script `fresh_start`: a scenario with an asked seed starts in the
  chosen place at the stated time with its setup applied, and the same
  answers give the same state hash.
- A locked setting cannot be changed by the `Setting` command in that
  scenario.
- A scenario's rules do not run in another scenario.

### SC2 — Characters at the start

**Spec** 13 §4.2 · **Size** M

Build
- Limits on the creation session from `create`; `spec`, `pick`, `roster`
  (stubbed until SC8), `become`; parties and control modes.

Done when
- The creation screen offers exactly what the scenario allows.
- Test script `become_anyone`: the player takes over a random notable
  inhabitant, with that character's body, beliefs, routine and relations
  intact.
- A party of three starts with each member's control mode as stated.

### SC3 — Objectives and missions

**Spec** 13 §5 · **Size** M

Build
- `Objective` with reactive conditions, markers, nesting and effects; the
  `mission` type with parameters, ordering and expiry; `start_mission`;
  postings carried by published knowledge and shown by devices; data for
  the journal screen.

Done when
- An objective's state equals a fresh evaluation until it settles, then
  stays (invariant Z4).
- Test script `mission_from_posting`: a posting read from a board starts a
  mission; its marker follows what the character knows of the quarry, not
  where the quarry is.
- A posting about something already resolved can still be read and
  accepted, and fails or completes as its conditions say.

### SC4 — Plots

**Spec** 13 §6 · **Size** S

Done when
- Test script `plot_in_order`: beats fire once each, only after their
  predecessors, and a beat whose condition never holds never fires
  (invariant Z5).

### SC5 — The director

**Spec** 13 §7 · **Size** M

Build
- The `director` type; tension with fading and raising; the band and
  lulls; the pool with weights, conditions and `again`; placing by
  distance from the party; `on_travel`; `director_choosing`; choosing a
  director at the start.

Done when
- Test script `director_lulls`: over a long run, incidents start only
  after a lull below the band and never while above it.
- The director's only effect on the simulation is `start_incident`
  (invariant Z2).
- Drawn frequencies match the pool's weights.
- The bundled directors' weights reference no property of the party's
  strength, checked by a lint over their expressions.

### SC6 — Offences, cases and findings

**Spec** 13 §8; 02 §7.4; 15 §4 · **Size** L

Build
- Totals by instigator on summing world events.
- `cause_facts`; the `case_kind` type; `Case` with holders and findings;
  `open_case`, `add_finding`, `accuse`, `close_case`; `case_judging`.
- Recorder devices: senses and memories without a mind.
- Bundled rules: a plain set of offences, examinations that yield findings
  by means, and a watch that works cases through the cold layer.

Done when
- A case's true record cannot be read in a mind context or an ordinary
  screen (invariant Z3), shown by load errors.
- Test script `what_the_surgeon_sees`: the same wound yields different
  findings to examiners with different means.
- Test script `framed`: witnesses to a copied persona, an accusation, a
  closed case naming the wrong owner, and the true record unchanged.
- Test script `wanted`: an offence by the player with a witness becomes a
  case held by a group, findings accumulate through the cold layer, and
  responders are sent for the persona the player presented.
- A recorder's memories can be read by an operator and name what it
  perceived.

### SC7 — Saving, loss of the party, endings

**Spec** 13 §9 · **Size** M

Build
- `Advance::Request` and its handling by the application at safe points.
- Saving policies enforced in the application and its screens.
- `party_gone` and its three outcomes; `checkpoint`, `restore_checkpoint`
  with carried memories; `ends`, `end_scenario`, summary screens.

Done when
- Requests are carried out only at safe points (invariant Z6).
- Test script `permadeath_run`: under `single` with `end`, the save is
  gone after the ending.
- Test script `carry_on`: under `continue`, the player resumes as another
  character in the same world with the first one's body still where it
  fell.
- Test script `remembered_dawn`: after `restore` with memories carried,
  the character holds memories of events that, in the restored world, have
  not happened.

### SC8 — Legacy, roster, campaigns

**Spec** 13 §10 · **Size** M

Build
- The `legacy` scope with copy-in on start and load and write-back at safe
  points; `keep_character` and `roster` starts; `then`, `next_scenario`
  and `carry`.

Done when
- A legacy read gives the same value on replay (invariant Z7).
- A kept character starts a different scenario with body, abilities and
  holdings intact.
- Test script `second_chapter`: a campaign's next scenario starts with the
  carried party placed by its own start.

### SC9 — Bundled scenarios

**Spec** 13 §11 · **Size** L

Build
- For the reference worlds of worldgen package WG12: a sandbox in each; a
  roguelike descent in the crypt; a skirmish in an authored arena; an
  investigation in the modern blocks; a survival start on the island; a
  two-chapter campaign joining two of them.
- Directors `steady`, `harsh` and `quiet`.

Done when
- Each starts from the new-game screen, runs under scripted input to an
  ending or for a day, and passes checks of its mode's defining behaviour.
- No scenario relies on engine behaviour that names its mode.

### SC11 — Turning back the whole world (experimental)

**Spec** 13 §9.5; 04 §8.4 · **Size** M · Uses the snapshot-and-replay
machinery of tooling package TL8

This package is experimental. No milestone depends on it, no bundled
scenario may use it, and it should be built last. Part of its purpose is to
find out what the feature costs and where it breaks. Record what is found
against the table of unresolved questions in 13 §9.5.

Build
- `keep_world_history` and `rewind_world` as requests to the application;
  history kept by span and interval, optionally bound to an ability;
  carrying legacy variables, memories as foreknowledge and named variables;
  discarding the later input log; reasons when history is short or absent.

Done when
- After `rewind_world`, the state hash before `carry` is applied equals
  the hash recorded at that tick in the original run.
- Test script `relived_minute`: a character exempted by `carry` holds
  memories of the abandoned span; a character who was not holds none; and
  the world's cells and everyone's positions are as they were.
- With no history kept, `rewind_world` is refused with its reason; with
  too little, it goes back as far as there is and reports how far.
- History bound to an ability stops being kept when the ability is revoked.
- Both effects are registered under the experimental feature
  `world_rewind`: a mod that uses them without opting in fails to load, and
  one that opts in is warned.
- The cost of keeping history on the default loaded set is measured and
  written into 13 §15 in place of the guesses there.

### SC10 — Benchmarks and budget gate

**Spec** 13 §15 · **Size** S, recurring

## Test script catalogue

Under `scenarios/games/`.

| Test script | Package | Serves | What it checks |
|-------------|---------|--------|----------------|
| `fresh_start` | SC1 | S21 | Worlds, place, time, setup |
| `become_anyone` | SC2 | — | Taking over an inhabitant |
| `mission_from_posting` | SC3 | S7 | Postings as knowledge; markers from belief |
| `plot_in_order` | SC4 | — | Beats |
| `director_lulls` | SC5 | S7 | Pacing |
| `what_the_surgeon_sees` | SC6 | S15 | Findings by means |
| `framed` | SC6 | S15 | Belief convicts; truth persists |
| `wanted` | SC6 | S15 | Cases about the player |
| `permadeath_run` | SC7 | S21 | Saving policy |
| `carry_on` | SC7 | — | Another character, same world |
| `remembered_dawn` | SC7 | — | Restore with memories carried |
| `second_chapter` | SC8 | S21 | Campaign carry |
| `relived_minute` | SC11 | S10 | Experimental. World rewind with memories carried |

## Review additions

The owner's review of the open questions (decisions D-182 to D-195) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Distance as travel time | SC5 | 13 §7.3 | With an entry's distance given as ten minutes, incidents are placed a few streets from a party on foot and far further from one that has been flying, measured over the last hour of its own time. |
| Reopening a case | SC6 | 13 §8.5 | Test script `second_look`: a finding gained after a case is closed runs `case_judging` again with `reopening` set; rules that no longer accept the blame reopen it and `case_reopened` is raised; `reopen_case` does the same directly. |
