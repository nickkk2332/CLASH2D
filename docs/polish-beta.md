# Polishing the beta

Everything the polish audit of 21 November 2026 found in the beta, merged,
checked and ranked, for a developer to work from. Nothing here is fixed yet.
Seven finders looked at the game through different lenses (two code audits,
the bundled content, the documents and modding, a new player, and two
players trying to break the game). They wrote about 230 findings, and
these are merged here into 172 entries. Repository at `b3e6e5f`. Prebuilt
release binaries were used, played headless.

Each entry has a stable id, `PB-001` to `PB-172`, given in rank order:
blockers, then majors, minors and polish; within each, the entries most
players would meet, and meet worst, come first. In each severity the
entries are grouped by area, so the ids within a group are not
consecutive. The mark on each entry says how it was checked:

- **verified**: reproduced in the game, the console or the tools during
  consolidation.
- **verified (code)**: confirmed by reading the code or the data.
- **not checked**: the finder's evidence stands, but it was not run again.
- **not reproduced**: what was seen instead is described in the entry.

Entries already listed in `docs/release-notes-beta.md` or
`docs/playtest-beta.md` are marked **known**, with what is new.

## Summary

| Area | Blocker | Major | Minor | Polish | All |
|------|--------:|------:|------:|-------:|----:|
| saves | 2 | 1 | 3 | 3 | 9 |
| combat | | 9 | 8 | 5 | 22 |
| powers | | 4 | 11 | 3 | 18 |
| ui | | 3 | 22 | 18 | 43 |
| town and ai | | 3 | 4 | | 7 |
| crafting and items | | 1 | 9 | 7 | 17 |
| scenarios | | 1 | 3 | 1 | 5 |
| content and text | | 4 | 4 | 2 | 10 |
| performance | | 1 | | | 1 |
| window | | | | | 0 |
| docs | | 1 | 9 | 10 | 20 |
| modding and tools | | 3 | 7 | 7 | 17 |
| engine | | 1 | 2 | | 3 |
| **All** | **2** | **32** | **82** | **56** | **172** |

97 entries were checked during consolidation: every blocker and major
(34), 42 minors and 21 polish entries. 21 of the 97 were checked by
reading code or data.

### Fix these ten first

- **PB-001**: A game's own save overwrites, and its ending deletes, any other game's save with the same name.
- **PB-002**: Closing the window, or pressing Ctrl+C, in the last stand saves nothing, so the night is lost.
- **PB-003**: On the errand's route past the market, each key takes 0.3 s and a minute's wait takes 10 s.
- **PB-004**: After one punch the watch beats the character to death, and keeps striking after the death.
- **PB-005**: Walking into a stranger asks "Attack?" with Attack already selected.
- **PB-006**: At 80×24 the condition screen shows no wounds for anyone without powers.
- **PB-007**: Showing the title rewrites every save file, so Continue loads the wrong save.
- **PB-008**: An unnamed character is called "Person", so the log reads "The person strikes the person."
- **PB-010**: While the character is knocked out, nothing happens until `.` is pressed.
- **PB-011**: Force push and fire bolt aimed at the next cell fault and still spend the reserve.

### Themes

- **Saves are keyed by name and opened for writing:** PB-001, 002, 007,
  060, 096, 097, 134, 135.
- **No feedback for an action:** crouching, hiding, picking up, a blow or
  binding broken off, a mode switched on, a power used, a sleeper waking,
  being laid prone, one's own death (PB-020, 022, 040, 041, 043, 044, 066,
  073, 074, 076, 078, 117, 121).
- **Internal ids and engine wording in player text:** `Fault:` lines
  (PB-011, 087), "`core:dash` of `core:dash`" (PB-046),
  `core:adept.powers` (PB-072), "tool: cutting 1" (PB-039), "Fire bolt:
  to" (PB-118), "(ada)" (PB-134), and Rust types in the reference
  (PB-101).
- **Text cut at 80 columns, or no room for it:** PB-006, 036, 047, 070,
  150, 153.
- **Content that nothing reaches or that does nothing:** throwing (PB-012),
  a club blow (PB-013), the oven's loaf (PB-024), Float (PB-027),
  training (PB-064), Distil, Torch and Grow herbs (PB-079), bread at a
  hearth (PB-080), the rifle and gear (PB-084), Null field (PB-093),
  gravity well (PB-094), street lamps and empty bottles (PB-148), two empty
  games on the list (PB-062).
- **Pickers ignore the chosen target, offer yourself or things, or cycle
  wrongly:** PB-014, 016, 021, 050, 065, 086, 118, 124.
- **The market town starts the player in the watchman's cell** (a known
  issue of the sixth round). Several entries follow from it: PB-014, 015,
  040, 087, 089, 098.
- **Results far beyond what the action suggests:** the watch kills over a
  punch (PB-004); a rank-0 bolt fells a terrace (PB-018); a carried load
  knocks you out (PB-019); fists at a door kill (PB-023); a house burns
  away in a minute (PB-026); a rank-1 push kills (PB-092).
- **Mistakes a modder makes that pass lint, and documents that call built
  things unbuilt:** PB-032, 033, 103, 104, 106, 110, 111, 112, 164.
- **Powers are anonymous on screen:** what was used, what is on, and what
  it costs are not shown (PB-046, 049, 055, 073, 117).

### What the audit did not cover

- Everything was played headless. Nobody saw the window frontend or a
  real terminal: mouse, the context menu on the second button, sprites
  gliding, shake and flash, fonts, paste. The Windows and macOS builds were
  not run, so PB-097 comes from the code.
- Code not read in depth: substance and device internals, social orders,
  groups and personas, ability binding, reactive and suppression rules,
  body editing and definitions, ranged flight, field stepping and light
  solving, collapse maths, and the internals of `clash-tools scenario`,
  `replay` and `worldview`. Everything else was read, or grepped for
  panics, hash-map iteration, wall clock and unfinished notes.
- Not played: the pistol emptied and reloaded (the sixth round played it);
  a bow's quiver emptied; losing the errand by giving the parcel away;
  distilling at the still; the sleeping draught at a hearth; interrupting
  a long wait with a key (a headless pass runs the whole wait); saving
  during a reaction prompt; a hood destroyed while worn.
- Lines spoken by minds as raw text ("Help!", "Woof!", "What are you
  looking at?") were never seen in a log. Body, tissue and blood numbers
  were read but not judged against play.
- The window's default grid size was not checked. PB-006 and PB-036 are
  at the terminal's 80×24.

### How to reproduce

Run the game from the repository with a data directory of your own:
`clash --headless script.txt --data <dir> --seed N`, where `clash` is
`target/release/clash`. For most entries, put
`{"bindings":[],"preferences":{"ui:playback_speed":0}}` in
`<dir>/preferences.json`. At the default playback speed a plain headless
`frame` leaves out sprites while they glide (PB-099). Keys are written
here comma-separated; a script takes one per line. These prefixes recur:

| Name | Keys | Starts |
|------|------|--------|
| `LS` | `enter, type Mara, tab, tab, enter, enter` | Last stand, an ordinary person |
| `LSA` | `enter, type Mara, tab, tab, f2, enter, tab, tab, tab, tab, enter, enter` | Last stand, the ready-made adept |
| `LSM` | `enter, type Mara, tab, tab, f2, down, down, enter, tab, tab, tab, tab, enter, enter` | Last stand, the ready-made marksman |
| `ER` | `enter, type Mara, tab, tab, down, enter, enter` | The errand |
| `SQ` | `enter, type Mara, tab, tab, down, down, down, enter, down, enter, enter` | The market town, in the square, an ordinary person |
| `SQG` | `enter, type Mara, tab, tab, down, down, down, f2, down, down, down, down, enter, tab, tab, tab, tab, enter, down, enter, enter` | The same as the Gifted person |

The console is `clash-tools worldview` (lines on standard input, or
`--script file`). Scripts named `audit/…` are the finders', kept in the
session's scratch directory
`/tmp/claude-0/-home-user-CLASH2D/c530ecd0-83be-56a3-8562-0a52d34d02ff/scratchpad/audit/`.
That directory may not last, so every short repro is also written out in
full here.

## Blockers

| Id | Area | Title |
|----|------|-------|
| PB-001 | saves | A game's own save overwrites, and its ending deletes, any other game's save with the same name |
| PB-002 | saves | Closing the window, or Ctrl+C, in the last stand writes no save, so the night is lost |

### Saves

#### PB-001 A game's own save overwrites, and its ending deletes, any other game's save with the same name
*Blocker · saves · code, newplayer · verified*
- **What happens:** The last stand saves on leaving under the
  character's name, which is also the default name for a manual save in
  any game. So an errand or market-town save of that name is replaced
  without a word, and the other way round. A last stand that ends deletes
  whatever save has that id ("Its one save (ada) is gone."). Checkpoints
  share the same id namespace.
- **Where:** `crates/clash-app/src/game.rs:928-950` (`save_on_leaving`
  uses `default_save_name()`), `:705-707` (`end_scenario` deletes that
  id under `single`), `app.rs:802-805`, `saves.rs:205-234`.
- **Repro (seed 4):** `ER` as Ada (`enter, type Ada, tab, tab, down,
  enter, enter`); then `escape, down, enter, enter`, which shows "Saved
  as “Ada” (ada)." and writes 1 056 768 bytes; then `escape, down, down,
  down, down, enter, enter` (Quit to title, Leave); then `enter, enter,
  enter, wait` (New game, Enter in the kept name begins Last stand,
  Begin); then `escape, down, down, enter, enter` (Quit to title, Leave);
  then `down, down, enter`. The Load list shows only "Ada  Ada, Last
  stand, 0:00:00 in", and `saves/ada.c2save` is now 180 224 bytes. The
  deletion on an ending was checked in the code.
- **Fix:** Give single saves and checkpoints an id of their own
  (scenario, character and seed) and keep it in the session. Never
  overwrite or delete a save whose `SaveMeta.scenario` differs, and ask
  before a manual save replaces an existing one.

#### PB-002 Closing the window, or Ctrl+C, in the last stand writes no save, so the night is lost
*Blocker · saves · code, newplayer · verified · known in part*
- **What happens:** Under the `single` policy the one save is written
  only by Quit to title. Closing the window, or Ctrl+C in a terminal
  (both arrive as `InputEvent::Close`), ends the loop without saving. The
  README recommends Ctrl+C to leave, and how-to-play says the save is
  "written when you leave".
- **Where:** `crates/clash-app/src/game.rs:1207-1243` (`pass`: Close only
  returns false), `:1323-1347` (`run_frontend`), `:904-924`
  (`end_if_left`); `lib.rs:142`, `:201`; `clash-front-term/src/lib.rs:211-253`.
- **Repro:** headless stand-in `enter, type Ada, enter, enter, wait, k,
  k`, then the end of the script. The data directory has no `saves/`
  afterwards. The window close takes the same path in the code.
- **Fix:** Treat Close as leaving: call `save_on_leaving` (and ask first
  under `free`) before the session is dropped.
- **Known:** The release notes say "a crash loses the night". New:
  closing the window does not count as leaving either.

## Majors

| Id | Area | Title |
|----|------|-------|
| PB-003 | performance | On the errand's route past the market, each key takes 0.3 s and a minute's wait about 10 s |
| PB-004 | town and ai | After one punch the watch beats the senseless character to death, and keeps striking after it |
| PB-005 | ui | Walking into a stranger asks "Attack the person?" with Attack selected |
| PB-006 | ui | At 80×24 the condition screen has no room for wounds |
| PB-007 | saves | Showing the title rewrites every save, so Continue loads the alphabetically first one |
| PB-008 | content and text | An unnamed character is "Person": "The person strikes the person." |
| PB-009 | combat | Struck from behind, you cannot see, target, face or hit the attacker |
| PB-010 | combat | While knocked out with a decision pending, nothing happens until `.` is pressed |
| PB-011 | powers | Force push and fire bolt at the next cell fault and still spend the reserve |
| PB-012 | combat | Nothing in the bundled games can be thrown |
| PB-013 | combat | There is no blow with a club |
| PB-014 | combat | With the target in your own cell, the precise attack offers only yourself |
| PB-015 | combat | After a reaction, `f` keeps picking Kick while the legs are busy |
| PB-016 | ui | Tab in a target picker sticks on the first candidate, and Shift+Tab skips one |
| PB-017 | content and text | Adepts and gifted persons cannot be poisoned, drugged or starved |
| PB-018 | powers | The fire bolt always fires fully charged: one rank-0 bolt empties a terrace |
| PB-019 | content and text | Carrying things lowers Blood; a wounded character who picks up 8 kg falls senseless |
| PB-020 | town and ai | After sixteen hours awake the player's character is laid prone without a word |
| PB-021 | combat | Knock out and Choke are never aimed at the chosen target |
| PB-022 | combat | Knock out from behind does not knock out |
| PB-023 | combat | Two bare-handed blows at a door disable both hands and kill |
| PB-024 | crafting and items | Bread baked in the oven cannot be taken out |
| PB-025 | content and text | The Heavy build makes a brawler weaker than Average |
| PB-026 | engine | A fire in a wooden house burns it all away in a minute and a half, and nobody reacts |
| PB-027 | powers | Float does not let its user step off a ledge |
| PB-028 | town and ai | Taking the hood off in front of the watchman clears you on the spot |
| PB-029 | docs | The downloads carry docs that need the source tree, and no `clash-tools` |
| PB-030 | modding and tools | `init-mod --dir` outside `mods/`: the next step it prints fails with eleven errors |
| PB-031 | scenarios | A game scenario in `core:testland` starts eight hours late |
| PB-032 | modding and tools | A misspelt event in a scenario's `ends` passes lint, and the game never ends |
| PB-033 | modding and tools | A misspelt `mind` passes lint and makes a creature that never thinks |
| PB-034 | powers | A shot fired inside one's own time stop falls to the ground |

### Saves

#### PB-007 Showing the title rewrites every save: dates are "now", and Continue loads the wrong save
*Major · saves · code, systems2 · verified*
- **What happens:** Listing the saves, which the title does every time it
  is shown, opens each save file read-write and rewrites it. So the Load
  list dates every save at the moment it was listed. Continue takes the
  first save in a list sorted by that time and then by id, so it loads
  the alphabetically first save, not the newest one.
- **Where:** `crates/clash-world/src/redb_store.rs:80-99` (`create`, not a
  read-only open); `clash-save/src/game.rs:294-327`; `tools.rs:122-143`;
  `clash-app/src/saves.rs:251-298`; `mods/ui/data/app.json:17-20`.
- **Repro (seed 4):** run `audit/s2/sv2.txt`, which is `SQ` as Ada, Save
  as `alpha` at 9:00, `p, p`, Save as `zeta` at 9:02, Quit, then Continue.
  Continue loads `alpha` (Time 9:00). Then `touch` the two files to older
  dates and show the title once: both files' modification times and md5
  sums change.
- **Fix:** Open saves read-only for listing, and keep the time of writing
  in `SaveMeta`.

### Combat

#### PB-009 Struck from behind by someone next to you, you cannot see, target, face or hit them
*Major · combat · newplayer · verified*
- **What happens:** A stranger who comes in by the back door stands at the
  character's south-west in a lit room and strikes. On the map he is only
  a heard `!`. Tab and `f` say "Nobody is in sight to choose.", walking
  into him says "Someone is in the way.", and no key turns the character
  round.
- **Where:** the sight arc of 200° (`mods/core/data/perception/senses.json:4`);
  `mods/ui/data/controls.json` has no control to face a direction.
- **Repro (seed 1):** `LSA`, then `l, l, k, k, k, k, k, l, g, enter, g, p,
  escape`, which gets her struck; then `tab, f, b`. The log shows "Nobody
  is in sight to choose. (x2)" and "Someone is in the way.".
- **Fix:** Turn the character toward an unseen occupant it walks into, and
  toward a blow it feels; add a control to face a direction.

#### PB-010 While knocked out with a decision pending, nothing happens until `.` is pressed
*Major · combat · systems · verified*
- **What happens:** If the character is knocked out while the player's
  free decision is still pending (the usual case in a fight), the blows,
  the watch and the clock all stand still. Other keys answer
  "Unconscious, and cannot act until they come to".
- **Where:** `crates/clash-app/src/session.rs:106-117`
  (`waits_on_simulation` is false whenever a decision is pending);
  `game.rs:958-990`.
- **Repro (seed 4):** run `audit/sys/mk7.txt`: `SQ` as Cy, `tab`, then
  `f, escape, p, escape` six times until "Cy falls senseless.", then
  `wait 3000`. The time stays at 9:00. Pressing `.` then gives "Cy comes
  to.".
- **Fix:** Withdraw the pending decision when the player's mind stops, or
  keep advancing while the player is unconscious whatever is pending.

#### PB-012 Nothing in the bundled games can be thrown: `t` and Throw… refuse whatever is held
*Major · combat · content, newplayer, systems2 · verified*
- **What happens:** `t` ("throws what is in your hand") and the
  inventory's Throw… refuse a club, a bandage or anything else with
  "There is nothing in hand to throw, or nowhere to throw it." The Throw
  picker offers yourself first.
- **Where:** `mods/core/data/combat/ranged.json:131-140` (`throw` requires
  `self.wielding('throwing')`). Only the throwing knife and the charge
  carry that tag, and nothing places either (PB-084).
- **Repro (seed 4):** `enter, enter, enter, l, l, k, k, k, k, u, e, enter,
  b, j, j, ., ., ., ., ., tab, t`, which picks up the club, waits for a
  stranger seven cells off, and throws.
- **Fix:** Let `throw` take any held item, with the tag deciding only the
  point and the fuse, or put throwing knives in the armoury.

#### PB-013 There is no blow with a club: the brawler's weapon only parries
*Major · combat · newplayer · verified (code)*
- **What happens:** No attack uses a held club. Punch, Kick, Shove and
  Smash strike with the body, and Slash, Stab and Whirl need a blade. With
  the brawler's or the last stand's club in hand, `f` kicks or punches,
  although how-to-play promises "a blow with a club".
- **Where:** `mods/core/data/combat/fighting.json:33-166` (the `with` of
  each attack); the club's `head` impact surface in `items.json:41-47`;
  `docs/how-to-play.md:48`, `:381-383`.
- **Repro:** Shift+F with a club in hand lists Kick, Punch, Slash (needs a
  blade in hand), Stab, Whirl, Shove, Smash.
- **Fix:** Add a club blow (impact, the held club's surface) that `f`
  picks first when a club is in hand.

#### PB-014 With the target in your own cell, the precise attack offers only yourself, and the blow does nothing
*Major · combat · systems · verified*
- **What happens:** Begun in the square, the player shares the
  watchman's cell (a known issue). Tab targets him, but Shift+F's picker
  offers only "Mara (1 of 1), 0.0 m". After Enter, a part and an effort,
  nothing happens and nothing is said.
- **Where:** the entity picker for a strike, which does not offer the
  other occupants of the user's own cell.
- **Repro (seed 4):** `SQ`, then `tab, shift+f, down, enter`, which shows
  "Punch: target  Mara (1 of 1), 0.0 m. In reach."; then `enter, enter,
  enter`. The log stays empty.
- **Fix:** Offer the creatures in the user's own cell, and never the user.

#### PB-015 After a reaction, `f` keeps picking Kick while the legs are busy, and is refused each time
*Major · combat · systems · verified*
- **What happens:** Right after a Dodge, `f` picks Kick and is refused
  "You need 1 leg free and have 0." at every press, with no time passing,
  instead of punching. Shift+F does not grey Kick out.
- **Repro (seed 4):** `SQ`, then `tab, f, f, enter` (Dodge), then `f` five
  times. The log shows "You need 1 leg free and have 0. (x5)".
- **Fix:** Pick the first blow that can start now (Punch), and grey Kick
  out with its reason.

#### PB-021 Knock out and Choke are never aimed at the chosen target; without one, the picker offers yourself or a stick
*Major · combat · content, systems, systems2 · verified*
- **What happens:** With a target chosen, `v` lists "Knock out: Knock out
  is not aimed at someone." and the same for Choke. Without a target, the
  "Knock out: victim" picker offers yourself, a stick, or a person in
  front of you, but never the watchman in your own cell. This is why the
  takedowns from behind cannot be done from the key.
- **Where:** `crates/clash-sim/src/offer.rs:392-421` (a row takes a target
  only if a parameter or role is named `target`);
  `mods/core/data/combat/takedowns.json:17-55` (the role is `victim`);
  `mods/ui/data/fighting.json:29-30`.
- **Repro (seed 4):** `SQ`, then `tab, v`.
- **Fix:** Name the role `target`, or have the control pass
  `victim: app.target`.

#### PB-022 Knock out from behind does not knock anyone out; while the victim still perceives you, it fails silently
*Major · combat · systems2 · verified*
- **What happens:** One step behind the watchman, Knock out (the picker
  shows "Watchman (1 of 1), 1.0 m") does nothing and says nothing while he
  still perceives her. Ten seconds later it lands as an ordinary blow:
  "Mara strikes the watchman.", "The watchman is wounded.", "Mara is
  wounded." (her own fist). The townsfolk shout, he turns and kicks, and
  he never falls. Choke from the same place works.
- **Where:** `mods/core/data/combat/takedowns.json:17-35` (a fist to the
  head at `strength * 15`, 150 J); `scenarios/combat/takedown.scn` checks
  only that the victim is wounded.
- **Repro (seed 4):** `SQ`, then `j, j, k`, `.` ten times, then `v, down,
  down, down, down, down, enter, enter, wait 5, ., ., .`.
- **Fix:** Make it a blow that drops an unarmoured person without breaking
  the striker's hand, and refuse it in words when the victim knows you are
  there.

#### PB-023 Two bare-handed blows at a door disable both hands, and the character bleeds to death
*Major · combat · systems · verified*
- **What happens:** Shift+F, Smash at a shut wooden door, twice, at the
  default effort leaves both hands "40% disabled, 1 wound" with open
  wounds. The door is untouched and the log says "Cy strikes something.".
  Within three minutes "You carry on as someone else."
- **Where:** `mods/core/data/combat/fighting.json:136` (fist energy
  `strength * 8`, 80 J at effort 1).
- **Repro (seed 4):** run `audit/sys/bot_smash.script`: `SQ` as Cy, `k, k,
  k, u, l`, then twice `shift+f, down, down, enter, u, enter, enter`; then
  `c` at `size 100 40`; then `.` ten times and `p, p, p`.
- **Fix:** Weigh what comes back into the hand against the door's give, so
  that a fist bruises; name the door in the log.

### Powers

#### PB-011 Force push and fire bolt at the next cell fault, do nothing, and still spend the reserve
*Major · powers · newplayer, systems · verified*
- **What happens:** Aimed at a cell one step away (the picker says "In
  reach."), each writes "Fault: `release` declined (world.refused) in
  action core:force_push at phases.push.do[0]", nothing moves, and the
  30 (or 20) reserve is spent. Pushing the person in front of you is the
  first thing anyone tries. From two cells away both work.
- **Where:** `mods/core/data/powers/powers.json:44-74`: the release is put
  at `offset(self.loc, dir(self, $to))` with `"dir": "$to"`, so its origin
  and its direction are the same cell.
- **Repro (seed 1):** `SQG`, then `l, l, j, j, j` (the watchman is now
  south-west), then `a, down, enter, b, enter`. For the bolt: `a, down,
  down, enter, b, enter`.
- **Fix:** Give the release a direction from the user, or place it at the
  user's cell. Refuse in the picker any cell that cannot work. Keep
  `Fault:` lines out of the player's log, and add test scripts at
  distance 1.

#### PB-018 The fire bolt always fires fully charged: one rank-0 bolt at a door empties a terrace of three houses
*Major · powers · systems · verified · known in part*
- **What happens:** The bolt's charge phase is "hold" and nothing lets it
  go, so every bolt charges for the full three seconds: 13 times its
  energy, and fire 255. One bolt for 20 reserve at a terrace door breaks
  the door, the floors give way, and the ground storeys of all three
  houses are emptied. Pressing `z` after Enter says "There is nothing
  being done to stop."
- **Where:** `mods/core/data/powers/powers.json:58-75` (`"time": "hold",
  "max": "3 s"`, one charge every 250 ms), `:114`.
- **Repro (seed 4, `--profile dev`):** `SQG`, then `f12, a, down, down,
  enter, k, k, k, k, k, l, l, l, enter` (the picker shows "In sight, 5.8 m.
  In reach."), then `., ., ., ., ., p, p, p, p`. The log shows "The floor
  gives way." and "Debris strikes the cupboard. (x3)", and F12 shows
  `#= =##= =##= =##` with the three ground storeys empty.
- **Fix:** Fire an uncharged bolt by default and let the player release
  the charge (the same fix as for the bow); then judge what a full charge
  should do.
- **Known:** The release notes say "A bow's draw runs to its end in one
  go". New: the fire bolt has the same cause.

#### PB-027 Float does not let its user step off a ledge
*Major · powers · systems2 · verified*
- **What happens:** With Float on, standing in a broken upstairs window, a
  step out over the street is refused with "There is nothing to stand on
  there.", exactly as with Float off. Nothing Float changes can be seen in
  play. Its description says "Nothing pulls you down while you float".
- **Where:** `float` in `mods/core/data/powers/areas.json`. The step is
  refused before gravity is consulted.
- **Repro:** run `audit/s2/fl1.py` to `fl6.py` (`SQG` at seed 4; the
  middle house of the terrace north of the square; up the stairs; Break
  through the window to the south; `j` into it; Float on; `j`).
  Alternatively, load `audit/data-systems2/gift/saves/cp037.c2save`
  (Float on, standing in the window) and press `j`, which gives "There is
  nothing to stand on there. (x2)".
- **Fix:** Let a step into the air be taken while nothing pulls the
  character down, and fall when the reserve runs dry; or say that it
  matters only when thrown.

#### PB-034 A shot fired inside one's own time stop falls to the ground and never reaches the target
*Major · powers · systems2 · verified*
- **What happens:** With time stop on, a 500 J pistol round at someone
  10 m away never wounds them. The run's one `projectile_hit` is on the
  ground, with 0.08 J. Without the time stop the same shot wounds within
  100 ms. Players will rarely meet this: no ready-made character has both
  a gun and time stop.
- **Where:** `core:time_stop` (an area of clock rate 0.001) and a
  projectile's flight inside it. Probably gravity or drag acts at the
  realm's rate while the motion is slowed.
- **Repro:** `clash-tools scenario run audit/s2/scnrun` fails at line 21
  ("found … body=0 … energy=0.0822"). The control script without the time
  stop, `audit/s2/scn/ts_control.scn`, wounds the target.
- **Fix:** Run gravity and drag on the projectile's own clock.

### UI

#### PB-005 Walking into a stranger asks "Attack the person?" with Attack selected, so Enter starts a fight
*Major · ui · newplayer · verified*
- **What happens:** With the default "Into someone: Strike foes, ask
  else", walking into a stranger asks "Attack the person, south?" with
  [ Attack ] focused. The walk's next keys are swallowed by the dialog,
  and the first Enter strikes. The townsfolk shout for the watch, and
  PB-004 follows. The errand's way to the town hall runs through the
  market crowd, so the dialog comes up again and again.
- **Where:** `mods/ui/data/fighting.json:48-52` (`attack_bump_ask`).
- **Repro (seed 1):** `ER`, then `l` 21 times and `j` 110 times, then
  `enter, ., .`. The log shows `The shopkeeper shouts: "Watch! Over
  here!"`.
- **Fix:** Focus "Leave them be" by default, or refuse the step quietly
  for someone who is not a foe.

#### PB-006 At 80×24 the condition screen has no room for wounds
*Major · ui · newplayer, systems · verified*
- **What happens:** For anyone whose status shows Fed and Toxin (the
  ordinary person, the brawler, the marksman), the Wounds box is drawn on
  the screen's last row with no rows inside it. So whether a wound is open
  cannot be read, though the guide and the release notes send the player
  to `c` for it. The body tree also cuts "disabled, 1 wou".
- **Where:** `mods/ui/data/condition.json:17-24`: Reserves and Attributes
  are sized to fit, and Wounds gets what is left.
- **Repro (seed 1):** `LS`, then `l, l, k, k, k, k, k, l, g, enter, g, p,
  escape` ("Mara is wounded."), then `c`. Then `size 80 30` shows "left
  arm: impact, open, 2 deep".
- **Fix:** Give Wounds a minimum height, or shorten the reserves'
  explanations.

#### PB-016 Tab in a target picker sticks on the first candidate, and Shift+Tab skips one
*Major · ui · systems · verified*
- **What happens:** When two candidates share a cell, Tab always comes
  back to the first. Heal's picker shows "Loaf (1 of 4)" after every Tab,
  and Shift+Tab goes 4, 3, 1, 4, so candidate 2 can never be reached.
- **Where:** `crates/clash-scene/src/picker.rs:403-447`, which cycles by
  the cursor's cell.
- **Repro (seed 4):** run `audit/sys/h6.txt`: `SQG`, walk beside the
  terrace door, Smash it once to hurt a hand, then `a`, Heal, Shift+Tab
  four times, Tab twice.
- **Fix:** Cycle by candidate index.

### Town and AI

#### PB-004 After one punch the watch beats the senseless character to death, and keeps striking after it
*Major · town and ai · newplayer, systems2 · verified*
- **What happens:** After one punch at the watchman, or any scuffle, he
  strikes the character until they fall senseless and then until they
  die. He goes on striking after "You carry on as someone else.", and a
  minute later the console still shows his goal as `core:confront` about
  the dead body. The guide says only that a watchman "lays hands on you
  and strikes you".
- **Where:** `mods/core/data/ai/goals.json:87-104` (`confront` strikes
  whoever it holds hostile and saw in the last 3 s; nothing in it is about
  a foe who is down).
- **Repro (seed 4):** run `audit/s2/die2.keys`: `SQ` as Cy, `tab, f`,
  then `.`, with `escape` at each reaction prompt. The log shows "Cy falls
  senseless.", about thirty "The watchman strikes Cy.", "You carry on as
  someone else.", then "The watchman strikes Cy. (x2)" and "The watchman
  strikes the person. (x2)".
- **Fix:** Leave out `it.helpless` and the dead in `confront`'s track, or
  in its step's `if`.

#### PB-020 After sixteen hours awake the town's rest rule lays the player's character prone, without a word
*Major · town and ai · content · verified*
- **What happens:** The rule `rest_when_tired` is attached to every
  person when it is made, the player's character included. After sixteen
  hours up it sets the posture to prone, and it stands the person up
  again only at weariness 0. The player is not told and walks at a crawl:
  60 steps take four minutes instead of under one. Crouching and standing
  gets the character up again. The release notes say there is no way to
  lie prone.
- **Where:** `mods/core/data/rules/town.json:50-71` (`rest_when_tired`
  and `people_rest`; neither checks who controls the character).
- **Repro (seed 4):** `enter, tab, tab, down, down, down, enter, enter,
  enter`, then Shift+P 17 times, then `l, h` 30 times: the clock goes
  from 2:00 to 2:04. With Shift+P twice instead: 11:00 to 11:00.
- **Fix:** Add `"if": "self.control != 'direct'"` to `people_rest`, or
  tell the player and let them choose.

#### PB-028 Taking the hood off in front of the watchman who is after the hooded stranger clears you on the spot
*Major · town and ai · systems2 · verified*
- **What happens:** After a blow struck in the hood, the person struck
  fears the hood and the watchman holds it hostile. When she takes the hood
  off in their sight, both drop it within two seconds: `hostile` false,
  `fears` false, and the blow no longer remembered of her. The guide says
  "Take it off where nobody sees you", which implies being seen matters.
- **Where:** `hood_down` in `mods/core/data/making/disguise.json`, which
  does `drop_persona` and nothing else.
- **Repro:** `clash-tools worldview --script audit/s2/scn/hood_view.scn`
  (`scenarios/social/hooded_blow.scn` up to the blow, then the cloak
  taken off with `remove` in their sight).
- **Fix:** Whoever perceives the hood come off ties the hood's persona to
  its wearer, as whoever watches it go on already does.

### Crafting and items

#### PB-024 Bread baked in the oven cannot be taken out: the oven has no Take out
*Major · crafting and items · systems2 · verified (code)*
- **What happens:** "Ada finishes the work: what it gave is in the oven."
  The oven offers only Stoke, Light and Put out, so the loaf stays in the
  oven's device for good. The still and the workbench offer Take out; the
  hearth does not either.
- **Where:** `mods/core/data/making/stations.json:20-25`, `:38` (the oven
  and hearth fixtures have no `store` or `retrieve`).
  `crates/clash-app/tests/play_crafting.rs:115-117` passes only because
  its Take out reaches the cupboard next to the oven.
- **Repro:** run `audit/s2/bake*.py`: bake bread at the bakery, then press
  `e` beside the oven. There is no Take out row; the console shows the
  loaf in the oven's `contents`.
- **Fix:** Add `retrieve` (and `store`) to the oven and the hearth, and
  make the test check that the loaf is in hand.

### Content and text

#### PB-008 An unnamed character is "Person": the log reads "The person strikes the person."
*Major · content and text · content · verified · known in part*
- **What happens:** If the name is left empty (Enter in the name field
  begins at once, PB-037), the character is called "Person", or "Brawler"
  for a brawler. The last stand's strangers are persons too, so a fight
  reads "The person strikes the person." and "The person misses the
  person. (x3)".
- **Where:** `mods/ui/data/app.json:41`;
  `mods/core/data/life/templates.json:3`, `:11-12` (the name has
  suggestions but no default); `scenarios/last_stand.json:62`.
- **Repro (seed 4):** `enter, enter, enter`, then `p, escape` eight
  times, then `frame`.
- **Fix:** When no name is given, draw one from the template's
  suggestions; give the strangers a name of their own.
- **Known:** The release notes say the log names your own character by
  name. New: what that name is when none was typed.

#### PB-017 Adepts and gifted persons cannot be poisoned, drugged or starved
*Major · content and text · newplayer, systems2 · verified*
- **What happens:** The two power templates give their pools outright,
  which replaces the person's pools. So they have no nourishment, toxin or
  sedation: they never get hungry, and nightshade, a poisoned blade and
  the sleeping draught do nothing to them. Their status column has no Fed
  bar.
- **Where:** `mods/core/data/playable.json:27` and `powers/powers.json:140`
  (`"pools": [...]`); `substances/substances.json:21-23` (which extends
  the person's pools).
- **Repro:** `clash-tools worldview --script audit/s2/scn/gifted_poison.scn`
  gives each of the three 20 ml of nightshade by mouth: the gifted person
  stays awake, the adept stays awake, and the ordinary person falls
  unconscious. The gifted person's toxin is `null`.
- **Fix:** Use `extend` for the pools in the two templates.

#### PB-019 Carrying things lowers Blood; a wounded character who picks up 8 kg falls senseless
*Major · content and text · systems2 · verified*
- **What happens:** Blood's maximum is 7% of the body's mass, and that
  mass includes what is carried. Blood's limits are fractions of that
  maximum. So picking up four logs lowers the Blood bar with nothing said,
  and a wounded character who picks up 8 kg goes from 0.661 to 0.598 and
  falls unconscious. By the formula, a load of two thirds of one's own mass
  would knock anyone out.
- **Where:** `mods/core/data/life/stats.json:3-8` (`"max": "(self.mass ??
  0) * 0.07"`).
- **Repro:** `clash-tools worldview --script audit/s2/scn/load_blood.scn`
  bleeds a person to 0.661 and gives them four logs: 0.598, unconscious.
- **Fix:** Use `own_mass` in the blood pool's `max`.

#### PB-025 The Heavy build makes a brawler weaker (Strength 12) than Average (15)
*Major · content and text · content · verified*
- **What happens:** The build choice's attributes replace the brawler's
  own Strength of 15 instead of adding to it: Slight gives 8, Heavy 12.
- **Where:** `mods/core/data/playable.json:10`,
  `life/templates.json:17-19`.
- **Repro (seed 4):** `enter, type Ada, tab, tab, down, down, down, f2,
  down, enter, tab, tab, tab, down, down, enter, down, down, enter,
  escape, tab, enter, down, enter, enter, c` shows "Strength 12". Without
  the change of build it shows 15.
- **Fix:** Give the build as an addition, or give the brawler a build
  choice of its own.

### Scenarios

#### PB-031 A game scenario in `core:testland` (the modding guide's example) starts eight hours late
*Major · scenarios · docs · verified*
- **What happens:** The guide's scenario starts `"at": "06:00"`. Its
  introduction says "It is six in the morning." while the status column
  shows 14:00. The console gives `now()` as 21 630 s and `time_of_day` as
  50 430 s. Moved to `core:open_town`, the same scenario starts at 6:00.
- **Where:** `crates/clash-sim/src/scenario/world.rs:98-114` (the sky is
  probed at `z_max`); `mods/core/data/world/realms.json:3-15` (testland's
  top stratum has no sky).
- **Repro:** with the guide's mod (`audit/docs-work/mods/hearth`) beside
  `core`, `ui` and `core_ui`, run `use hearth seed=1`, `begin
  hearth:morning`, `eval "@self.loc.time_of_day"` in the console.
- **Fix:** Read the sky where the party will stand.

### Performance

#### PB-003 On the errand's route past the market, each key takes 0.3 s and a minute's wait about 10 s
*Major · performance · newplayer, systems · verified · known*
- **What happens:** At 14:00, by the market crowd the errand walks through
  (the target picker counts 113 people), every key costs about 0.29 s
  and a minute's wait about 10 s. So the errand's five-hour clock cannot
  be waited out there. Elsewhere: in the square a minute takes 1.3 to
  1.5 s and Shift+P 10.6 to 12 s; on the errand's north street a minute
  takes 2.8 s and an hour about 18 s.
- **Where:** the errand at seed 1, about 130 steps from the start.
- **Repro (release build, four cores):** `audit/np/er3_prefix.txt` (`ER`,
  `l` 21 times, `j` 126 times), with `ui:bump_attack` set to "never",
  takes 17.0 s. With 20 `.` added it takes 22.8 s, and with 3 `p` added
  46.0 s.
- **Fix:** Profile the crowd at 14:00. Blocked minds plan again and again
  (round one's first known issue), and PB-063 packs every cell.
- **Known:** The release notes say "not quick to wait in by day". New:
  figures ten times the stated 0.8 s a minute, on the errand's main
  route.

### Engine

#### PB-026 A fire in a wooden house burns floors, roof and walls away within a minute and a half, and nobody reacts
*Major · engine · systems2 · verified*
- **What happens:** A fire set in one cell of a house is burning three
  cells away ten seconds later. By thirty seconds the floors and the roof
  are gone, and by ninety the walls. In play (from a fire bolt) nobody
  shouts or flees. The log says only "The floor gives way." and
  "Something breaks to pieces. (x30)", never fire or smoke.
- **Where:** `mods/core/data/world/fields.json:3-11` (fire spreads to
  fuel at 30 a step from 60; heat of 10 J a level every 500 ms).
- **Repro (console):** `use default seed=4`, `begin core:town_life
  place=1`, `as @self do { "add_field": { "at": "offset(self.loc, -14, 2,
  0)", "field": "core:fire", "amount": 240 } }`, `advance 10s`, `look
  @c:1990,995,0` (fire 120), then `advance 20s` (the floor is gone) and
  `advance 60s` (the walls are gone).
- **Fix:** Weigh heat damage against what each material can take, slow
  the spread through walls, and add shows for fire and smoke and a
  response to them (fleeing, shouting).

### Docs

#### PB-029 The downloads carry docs whose every modding step needs the source tree; no `clash-tools` is packaged
*Major · docs · docs · verified (code)*
- **What happens:** The build workflow packages the game, `mods`,
  `profiles`, the README and three guides, but not `clash-tools` or
  `docs/reference`. The README still begins "You need Rust, installed
  with rustup" and never mentions the downloads. The modding guide's
  steps are `cargo run -p clash-tools -- …` (12 of them), and its links
  into `docs/reference`, and the README's links to the specification,
  lead nowhere in the package.
- **Where:** `.github/workflows/build.yml:38-53`; `README.md:42`;
  `docs/modding-guide.md`.
- **Fix:** Package `clash-tools` and the reference and say how to run them
  from a download, or say plainly that the download is for playing only.

### Modding and tools

#### PB-030 `init-mod --dir` outside `mods/`: the next step it prints fails with eleven errors, the cause last
*Major · modding and tools · docs · verified*
- **What happens:** The command prints "Check it with `clash-tools lint
  <dir>/mods/hearth`". Running that gives nine `E0501 no channel "arm"`,
  `no body_plan …`, `no material "wood"` errors, and only then `E0004 mod
  "hearth" requires "core", which is not installed` (11 errors in all).
  `lint hearth` says no such profile, and `scenario run` takes a single
  `--mods`. The game finds such a mod only in `<data>/mods`, which the
  guide never mentions.
- **Where:** `crates/clash-tools/src/scaffold.rs` (the closing hint);
  modding guide §1, §10 and §11.
- **Repro:** `clash-tools init-mod hearth --dir <dir>/mods`, then the
  printed command. `lint <dir>/mods/hearth --mods mods` passes.
- **Fix:** Print the `--mods` form when the bundled mods are not beside
  `--dir`; report an unmet `requires` first and hold back the errors it
  causes; accept several `--mods`; say in the guide where a mod outside
  the repository goes.

#### PB-032 A misspelt event or filter key in a scenario's `ends` passes lint, and the game never ends
*Major · modding and tools · docs · verified*
- **What happens:** `"on": "objective_change"` (for `objective_changed`),
  or a filter key `idd`, lints with "ok: 821 definitions". The same
  misspelling in a rule is E0684, with "did you mean".
- **Where:** `crates/clash-sim/src/scenario/def.rs` (`EndEntry`'s `on`
  and `filter` are not checked).
- **Repro:** `clash-tools lint audit/docs-work/mut/bad_end_event/hearth
  --mods mods`, and the same for `bad_filter_key`.
- **Fix:** Check `ends[].on` against the event registry and the filter's
  keys against the event's fields; warn on unknown objective ids and
  states.

#### PB-033 A misspelt template `mind` passes lint and makes a creature that never thinks
*Major · modding and tools · docs · verified*
- **What happens:** `"mind": "dgo"` on the guide's sheepdog lints `ok`. In
  play its `mind_def` and `goal` are `null`; with the right spelling they
  are `core:dog` and `core:go_about`. A misspelt sense in the same
  template is caught.
- **Where:** `crates/clash-sim/src/template/defs.rs:30-44`, `ai/mod.rs:285-297`,
  `ai/think.rs:125-136`.
- **Repro:** with `audit/docs-work/mut/bad_mind/hearth`, run `use hearth
  seed=1`, `begin hearth:morning`, `advance 30s`, `eval "[@e:2.def,
  @e:2.mind_def, @e:2.goal]"` in the console. It gives `[id#22, null,
  null]`.
- **Fix:** Check a template's `mind` as its senses are checked, with "did
  you mean".

## Minors

The 82 minors by area: ui 22, powers 11, crafting and items 9, docs 9,
combat 8, modding and tools 7, town and ai 4, content and text 4, saves 3,
scenarios 3, engine 2. Ids are in rank order. The first fifteen:

| Id | Area | Title |
|----|------|-------|
| PB-035 | ui | The log of the last game carries over into the next, and into a loaded game |
| PB-036 | ui | At 80 columns the New game and Make a character lists cut every description |
| PB-037 | ui | Typing a name and pressing Enter starts the last stand at once |
| PB-038 | ui | "Use what is near" never names the thing each row acts on |
| PB-039 | crafting and items | The make list reads like a parts list |
| PB-040 | town and ai | Death in the market town is told badly |
| PB-041 | scenarios | Losing the last stand: no line says the character died, and the summary marks nothing failed |
| PB-042 | combat | The reaction prompt offers reactions that cannot be made at all, marked "(too late)" |
| PB-043 | combat | A blow or a binding the player starts can end with no line |
| PB-044 | ui | Crouching gives no sign, and neither does a sleeper waking |
| PB-045 | content and text | Map glyphs collide, and how-to-play's legend is wrong |
| PB-046 | powers | Ability refusals put ids and raw numbers in the log |
| PB-047 | ui | Rows are cut off on Use, Make, help, Know-how and the map |
| PB-048 | ui | The setup screen always says the seed was 1 |
| PB-049 | ui | The abilities screen never says what a power does or what a mode drains |

### Saves

#### PB-060 Load in the middle of a game replaces it without a question
*Minor · saves · newplayer · not checked*
- In a free-save game, pause, Load and Enter replace the unsaved game at
  once. Quit to title asks first ("What is not saved is lost."). Nothing
  says what was loaded. **Fix:** ask the same question, and log
  "Loaded …".

#### PB-096 A save from an older build is listed as loadable, and loading it fails with an internal message
*Minor · saves · code · verified (code)*
- The list reads only the metadata, which checks `FORMAT_VERSION` (1), so
  a save from a build with another `SAVE_VERSION` (now 22) is enabled and
  Continue offers it. Loading it then says "the saved simulation has
  layout version 21, and this build reads version 22". There is no
  migration. **Where:** `crates/clash-app/src/saves.rs:258-296`;
  `clash-save/src/game.rs:311-322`; `clash-sim/src/sim.rs:174-176`.
  **Fix:** stamp `SAVE_VERSION` in `SaveMeta`, list such a save as broken
  with a sentence a player can read, and have Continue skip it.

#### PB-097 A save named like a Windows device (`Con`, `Nul`, `Com1`) cannot be written on Windows
*Minor · saves · code · verified (code); not run on Windows*
- `save_id` keeps letters and digits, so the file becomes
  `saves/con.c2save`, which is a device on Windows. **Where:**
  `crates/clash-app/src/saves.rs:205-234`. **Fix:** add a suffix to
  reserved names.

### Combat

#### PB-042 The reaction prompt offers reactions that cannot be made at all, marked "(too late)"
*Minor · combat · content, newplayer, systems · verified*
- The prompt says "Mara can react, with 0.1 s to spare." over rows that
  are all "(too late)". It offers Parry against a grab and with empty
  hands, and Raise shield with no shield. The reason a reaction cannot be
  made (its `fail_text`) is never shown. **Where:**
  `mods/ui/data/pickers.json:138-139`. **Repro (seed 1):** `LSA` as in
  PB-009: "Raise shield (too late)" with no shield. **Fix:** grey out what
  cannot be made, with its reason; keep "(too late)" for what is only
  late; say so when nothing can be in time.

#### PB-043 A blow or a binding the player starts can end with no line in the log
*Minor · combat · newplayer, systems2 · verified · known in part*
- An attack (from the bump dialog, or `f` at a target in reach) or a
  binding can end with no "strikes", "misses" or "stops" line, and the
  next `f` opens a picker as if no target had been chosen. Probably it is
  broken off by `core:stop_for_a_foe`; this was not confirmed. **Repro:**
  the PB-005 repro: after Enter on Attack and two `.`, the log shows only
  the shouts. `audit/s2/band*.py` shows a first binding that wrote
  nothing. **Fix:** every action the player starts ends in a line, "Mara
  stops: …" included. **Known:** sixth round, known issue 11 (a binding
  broken off). New: blows too.

#### PB-057 One blast or fire bolt fills the log: "wounded (x12)", "staggers aside (x7)" while senseless and flying
*Minor · combat · newplayer, content · not checked · known in part*
- The lines come in this order: "The person is wounded. (x12)", "falls
  senseless", "goes flying", "staggers aside. (x7)", "lands hard". Mara's
  own line has already scrolled out of the five-line log. **Repro
  (seed 1):** `LSA`, `p`, `.` five times, `tab, a, down, enter, tab,
  enter`. **Fix:** fold one blast's wounds into one line, in the order
  they happened, and do not report a senseless body staggering.
  **Known:** sixth round, known issue 1 (consequences before the blow).

#### PB-086 Aim (Shift+S) needs someone in sight: a shot cannot be aimed at a place
*Minor · combat · systems2 · not checked*
- With nobody in sight it says "Nobody is in sight to choose.", though the
  guide says Shift+S chooses "the place aimed at". **Fix:** open a cell
  picker when nobody is targeted.

#### PB-087 Throwing someone who stands in your own cell faults
*Minor · combat · systems2 · not checked*
- Tab, `v`, Throw on the watchman in your own cell gives "Ada strikes the
  watchman." and then "Fault: `throw` declined (ranged.no_aim) in action
  core:throw_person at phases.heave.on_exit[0]". From the next cell it
  works. **Fix:** throw away from the thrower's facing, or refuse in words
  first, as D-973 does for shots.

#### PB-088 A grab is told as a blow, and Drag with nothing held does nothing, unsaid
*Minor · combat · systems · not checked*
- Grab logs "Cy strikes the watchman."; Drag with nothing held says
  nothing. **Repro:** `audit/sys/gr2.txt`. **Fix:** log "Cy grabs the
  watchman."; refuse Drag in words.

#### PB-089 A target who dies in your own cell stays the target, and is struck again
*Minor · combat · systems · not checked · known in part*
- "The watchman dies." is followed by "Cy strikes the watchman.", and the
  status column still reads "Target The watchman". **Repro:**
  `audit/sys/mk1.txt`. **Known:** round 2, known issue 7, and D-944
  dropped dead targets. New: not when they share your cell.

#### PB-090 After a blocked kick the bow needs "2 arms free and have 1" for good, with both arms shown whole
*Minor · combat · systems2 · not checked (cause unknown)*
- After "The person cannot strike: …", every `s` (133 times) says "You
  need 2 arms free and have 1.", while `c` shows both arms and hands whole
  and only the bow held. **Repro:** `audit/s2/lsbow2.py` (the first 35
  lines of `lsbow2.keys`). **Fix:** find what holds the arm, and show it
  on the condition screen.

### Powers

#### PB-046 Ability refusals put ids and raw numbers in the log
*Minor · powers · code2, systems · verified*
- Examples: "`core:dash` of `core:dash` is ready again in 7.42381 s" and
  "It costs 25 of `core:reserve`, and there is 22: 3 short". The
  suppressed, rank, "is on already" and upkeep refusals are the same.
  **Where:** `crates/clash-sim/src/ability/using.rs:161-171`, `:419-463`;
  `suppress.rs:312-318`; `modes.rs:110-145`; `ops.rs:46-53`. **Repro
  (seed 1):** `SQG`, `a, enter, h, h, a, enter`. **Fix:** build the text
  from the ability's and the pool's names, with whole seconds.

#### PB-050 Heal's picker leaves yourself out, and from the abilities screen Heal goes to the attack target, even a foe
*Minor · powers · newplayer, systems · verified*
- The picker cycles a loaf, a stick, a person and a tavern keeper 27 m
  off, never the user, who is reached only by moving the cursor onto her
  own cell ("0 of 4"). With a target chosen, Heal goes to it without
  asking, even the watchman she is fighting. **Where:**
  `mods/ui/data/abilities.json` (`"with": { "target": "app.target" }`).
  **Repro:** `audit/sys/h6.txt`. **Fix:** put yourself first among Heal's
  candidates, offer only bodies, and never take the attack target for a
  power that aids.

#### PB-055 Float and Shadow cost nothing; with Shadow on, standing on the lane wins the last stand
*Minor · powers · content, systems2 · verified*
- The reserve regenerates 2 a second, and Float drains 1.5 and Shadow 1,
  so both last for ever, though Float's description says "drains your
  reserve slowly". The gifted person with Shadow on survives the last
  stand by standing on the lane pressing Shift+P ("Survived. … [x] Hold
  out until dawn."); without it she is wounded within minutes. **Where:**
  `mods/core/data/powers/powers.json:3-5`, `areas.json:79`, `:95`.
  **Repro:** `audit/s2/lsg_sh2.txt` at seed 4. **Fix:** an upkeep above
  the regeneration, or no regeneration while a mode is up. This
  contradicts the release notes' "Last stand is a gamble every way".

#### PB-065 Summon hound on your own cell spends the reserve and summons nothing
*Minor · powers · newplayer · verified*
- The picker starts on her own cell ("In sight, 0.0 m. In reach."). Enter
  gives "Mara uses a power.", 40 reserve is spent, and no hound appears.
  **Repro (seed 1):** `SQG`, then `a`, `down` five times, `enter, enter`.
  **Fix:** refuse your own cell in the picker, or use the nearest free
  cell.

#### PB-073 A mode that is on shows nowhere in play
*Minor · powers · newplayer · not checked*
- With Shield or Time stop on, only the Rsrv bar shrinking shows it; only
  the abilities screen's `[on]` says which. **Fix:** a status line for
  each active mode ("Shield raised").

#### PB-074 A time stop that runs dry during a minute's wait ends without a word
*Minor · powers · systems · not checked*
- Waited second by second, it logs "Ada runs dry, and a power fails.", but
  inside a `p` nothing is logged. **Repro:** `audit/sys/ts2.txt`. **Fix:**
  log the mode's end whenever it happens.

#### PB-091 Shadow by day blacks out six metres round its user
*Minor · powers · systems2 · not checked*
- Shadow is a light of −2 with radius 6. In the square at nine in the
  morning the watchman in her own cell loses her (`last_seen` becomes
  null), Tab finds nobody, and look on her own cell says "Remembered".
  **Repro:** `audit/s2/shadow1.py`, `shadow2.py`. **Fix:** a smaller
  radius, dimming daylight rather than removing it, or an override on the
  user only.

#### PB-092 One rank-1 force push (3 MJ) throws a passer-by, wounds them a dozen times and kills them
*Minor · powers · content · verified (code) (the 3 MJ; the death not checked)*
- The push's power is `3.0e6 * 1.5 ^ (rank - 1)`, while the bundled
  charge's blast is 0.4 MJ. In play: "wounded (x12)", "goes flying", and
  some seconds later "dies". **Where:** `mods/core/data/powers/powers.json:104`.
  **Fix:** a shove that knocks people down at rank 1.

#### PB-093 Null field, worth two of an adept's three points, has nothing to suppress in any bundled game
*Minor · powers · content · not checked*
- Nobody in any bundled game has a power, and nothing says so.
  **Where:** `playable.json:40`, `areas.json:24-41`. **Fix:** say so on
  the option, or give some opponent powers.

#### PB-094 Gravity well on the watchman does nothing to him, and he does not notice it
*Minor · powers · systems2 · not checked*
- Gravity 6 for 8 s, radius 2: the log shows only "Ada uses a power.",
  and he stays unhurt, not hostile, at his post. **Repro:**
  `audit/s2/gw.py`. **Fix:** slow or stagger those inside, or describe
  what it does.

#### PB-095 With a hound summoned, fighting the watchman in your cell brings up "The dog: Kick" for you to answer
*Minor · powers · systems · not checked (whose blow it is is unknown)*
- **Repro:** `audit/sys/s3.txt`. A test script
  (`audit/sys/scn/hound_samecell.scn`) shows the hound landing no blow
  there. **Fix:** find whose blow the prompt is for.

### UI

#### PB-035 The log of the last game carries over into the next game, and into a loaded one
*Minor · ui · newplayer, systems, systems2 · verified*
- After an ending, a new game's intro is drawn over the old log ("The
  game has ended: lost."). Continue and Load keep the log of the game
  being left, including lines from after the loaded save ("Saved as
  “zeta”" in a game loaded from `alpha`). **Repro:** PB-001's keys (the
  errand's line over the last stand's intro); `audit/s2/sv2.txt`.
  **Fix:** clear the log when a game starts or is loaded.

#### PB-036 At 80 columns the New game and Make a character lists cut every description mid-word
*Minor · ui · content, newplayer · verified*
- Examples: "Last stand  A house on the lane at night, and strangers on
  the road. Hold out" and "Adept  Born with two or three powers of your
  choosing, and the reserves to pa". The one text a newcomer chooses by is
  cut, with fourteen empty rows below it. **Where:**
  `mods/ui/data/app.json:51`, `character.json:13`. **Repro:** `enter`,
  then `f2`. **Fix:** show the selected entry's whole description under
  the list.

#### PB-037 Typing a name and pressing Enter starts the last stand at once
*Minor · ui · newplayer · verified*
- `enter, type Mara, enter` begins the first game on the list (Last
  stand, the hardest) as an ordinary person, without the list or F2 being
  seen. The sandbox is fourth on the list. **Where:** the new-game
  screen's `keys.accept` (D-876). **Fix:** Enter in the name field moves
  to the list, and the market town goes first or the selection is
  remembered.

#### PB-038 "Use what is near" never names the thing each row acts on
*Minor · ui · content, newplayer · verified*
- The list reads "Open  north", "Close  north: It cannot be shut now.",
  "Hide inside  north", "Switch  west": which thing (the wardrobe, the
  lamp) each acts on, and whether Switch turns the lamp on or off, must be
  worked out from the map. **Where:** `mods/ui/data/interact.json:16`;
  `interactions.json:18`. **Repro (seed 1):** `LS`, then `l, l, k, k, k,
  k, k, e`. **Fix:** "Open the wardrobe  north", "Put out the lamp".

#### PB-044 Crouching gives no sign at all, and neither does a sleeper waking
*Minor · ui · newplayer, systems, systems2 · verified*
- Pressing Shift+C to crouch and again to stand leaves the frames exactly
  the same. Crouching does work: crouched at two metres the healer sleeps
  on, while upright she wakes. But her waking says nothing either.
  **Repro:** `SQ`, `shift+c, frame, shift+c, frame`. **Fix:** log "Mara
  crouches.", show "Crouching" in the status column, and log "The healer
  stirs.".

#### PB-047 Rows are cut off on Use, Make, help, Know-how and the map, and popups do not widen
*Minor · ui · newplayer, content · verified*
- At 80×24 rows run off the edge: "Put in  north: It must be open, and you
  must have some", help's "What it does" column, Know-how's "[Healing
  salve, Nightshade ext", and the map's "Markers  south, 129 m Take the
  parc". At 120×40 help, Make and Use keep their 76-column width and are
  cut the same. Make shows the selected row's reason underneath; the
  others do not. **Repro:** `SQ`, `shift+m`; `?` at `size 120 40`.
  **Fix:** wrap the text or show the selected row in full, and let
  popups grow with the window.

#### PB-048 The setup screen always says the world was made "from the seed you gave, 1."
*Minor · ui · newplayer, systems · verified*
- The world is made from the right seed; only the line is wrong.
  **Where:** `mods/ui/data/scenario.json:10`;
  `crates/clash-app/src/game.rs:616`, `:637-639` (`open_role` never calls
  `fill_creation`). **Repro:** `--seed 7`, `enter, type Ada, tab, tab,
  down, down, down, enter`. **Fix:** fill in the creation values after
  `open_role`.

#### PB-049 The abilities screen never says what a power does or what a mode drains
*Minor · ui · content, newplayer, systems · verified*
- Uses show Rank, Cost and State, but modes show only "[on] Float:
  Floating" or "[  ] Null field: Null field". No description is shown
  anywhere, though every definition has one, and no upkeep (1 to 8
  reserve a second). **Where:** `mods/ui/data/abilities.json`. **Fix:**
  show the selected row's description and upkeep under the list.

#### PB-052 A stranger in sight, targeted and shot, is drawn as a heard `!`, not `@`
*Minor · ui · newplayer · verified*
- Look says "Person. In sight, 14.0 m away." and Tab targets him, but the
  map shows a bold `!` there and the golden frame lists no sprite. He
  appears only at about 9 m. **Repro (seed 1):** `LSM`, `p, x, tab`, then
  `escape, tab, frame golden` ("sprites 0"). **Fix:** draw whatever is
  seen as what it is; a heard mark never covers someone in sight.

#### PB-053 Make a character: "The result" panel never shows anything
*Minor · ui · content, newplayer · verified*
- It says "Nothing to show yet." before and after every choice, because it
  reads `app.creation.attributes`, which the creation service does not
  provide. **Where:** `mods/ui/data/character.json:51-56`;
  `crates/clash-app/src/creation.rs:12-23`. **Fix:** provide the
  attributes, or drop the panel.

#### PB-054 Make a character: the choices say nothing about what they do
*Minor · ui · newplayer · verified*
- The Build, Training, Stature and Powers popups list names and costs
  only ("[ ] Runner  (2)", "From 0.9 to 1.1. Value < 1 >"). The powers'
  descriptions exist but are not shown. **Fix:** show the selected
  option's description, and stature as a height.

#### PB-056 Walking into a post, a tree or furniture says only "Something is in the way."
*Minor · ui · content, newplayer · verified*
- A diagonal step past a post says "Something is in the way of cutting
  the corner.". In the last stand the journal points north-east to the
  door, through the porch lamp, which is drawn `!` like a sound. At seed 1,
  `u` at the start gives "Something is in the way.". **Where:**
  `mods/core/data/movement.json:24`. **Fix:** name the obstacle when it is
  seen.

#### PB-058 The log cannot be scrolled back
*Minor · ui · newplayer · not checked*
- The log shows five lines and a scrollbar, but no key scrolls it, and a
  fight writes several lines a blow. **Fix:** a key or a screen for the
  history.

#### PB-061 The watchman's blue `@` and the crimson warnings are hard to read on the default background
*Minor · ui · content · verified (code)*
- Against the background `#101010`: blue 2.0:1, the crimson of "Hungry",
  the bars and the prompt 2.4:1, dark grey streets 2.7:1, wood 3.2:1. The
  Night theme keeps the same colours. **Where:**
  `mods/core/data/town/people.json:10`, `mods/ui/data/theme.json:7`,
  `:31`, `colors.json:4`. **Fix:** at least 4.5:1 for text and 3:1 for
  glyphs (the palette's `sky`, `#5090d0`, gives 5.7:1).

#### PB-066 Hidden in a wardrobe, nothing says so
*Minor · ui · newplayer · not checked*
- Hide inside writes no line and the `@` is drawn on the wardrobe; only
  look and the `e` list ("Get out") tell. **Fix:** log "Mara hides in the
  wardrobe." and show it in the status column.

#### PB-067 Heard marks pile up and stay on cells in plain sight
*Minor · ui · newplayer · not checked · known in part*
- After five minutes in the square the street carries dozens of `!`, and
  they stay when she turns to face the empty street in daylight.
  **Known:** round 2, known issue 10 ("Footsteps, somewhere close." is the
  commonest line). **Fix:** fade the marks, and clear them when the cell
  is seen.

#### PB-068 The map and look mode never say which storey is shown
*Minor · ui · newplayer · not checked*
- `<` and `>` change the storey without a word. One storey up, the roofs
  are `~` and the cell above is "In sight" with "Nothing known". **Fix:**
  name the storey on both screens.

#### PB-069 Zooming out in look mode leaves the play map zoomed out
*Minor · ui · newplayer · not checked*
- After `x, -, -, escape` the play map stays coarse, the porch lamp
  vanishes, and play has no zoom key. **Fix:** restore the zoom on leaving
  look mode, or add a zoom key in play.

#### PB-070 Below 80×24 the play screen is hidden but keys still play the game
*Minor · ui · newplayer · not checked*
- At 60×20 the screen says it needs at least 80 by 24, yet `j` moves the
  character and Enter dismisses an introduction unseen. **Fix:** ignore
  every key but Esc while the play screen cannot be drawn.

#### PB-071 Key rebinding accepts any name, and says "Bound" after a refusal
*Minor · ui · newplayer · not checked*
- `W` is refused ("`W` is not a key: …", with no full stop), and "Bound
  W." appears beside the refusal. "Walk north" (the name the help gives)
  is saved and then does nothing; an empty name says "Bound k." and
  unbinds `k`. **Fix:** choose the control from a list, and check names.

#### PB-072 Taking a power with no points left raises an error box with internal ids
*Minor · ui · newplayer · not checked*
- The box says "the selection for `core:adept.powers` does not suit it:
  the options cost 5, more than its budget of 3". **Fix:** grey out rows
  that cannot be afforded, and say why in a sentence.

#### PB-085 A lit oven, an opened cupboard or a shelf taken from vanishes from the map
*Minor · ui · systems2 · not checked*
- Once the oven is lit, its cell `Ω` is drawn blank, while look still says
  "Oven."; the same happens to `[` and `∩`. **Fix:** keep the fixture's
  glyph when it becomes a device.

### Town and AI

#### PB-040 Death in the market town is told badly
*Minor · town and ai · newplayer, systems, systems2 · verified*
- "You carry on as someone else." comes first, and the old body's lines
  after it ("The watchman strikes Cy.") read as the newcomer being
  beaten. There is never a "Cy dies.". The newcomer has the same name, is
  made in the body's cell beside the killer, and look opens on the body.
  **Where:** `crates/clash-app/src/game.rs:752-777` (`take_control`); the
  market town's death rule. **Repro:** the PB-004 repro. **Fix:** tell the
  death first, give the newcomer a new name, make them somewhere away from
  the fight, and move the view to them.

#### PB-063 At two in the afternoon some 140 townsfolk fill every cell of the market and close the street
*Minor · town and ai · systems2 · not checked*
- 137 people stand in the 21 × 18 cells round the market, in every cell of
  it and of the street on its west side. Give says "The person is out of
  reach." as they shift, and the planner finds no way through. **Where:**
  `mods/core/data/routines/town.json:8-10`. **Repro:**
  `audit/s2/give2.py`, `give3.py` (the errand at seed 4). **Fix:** spread
  the crowd out and keep the streets open (see PB-003).

#### PB-077 The healer stands in her own doorway and keeps the house shut for minutes
*Minor · town and ai · systems2 · not checked (how often is unknown)*
- Her goal is `keep_shop` and her `last_failure` is `ai.no_method`; every
  step in asks "Attack the healer, south?". It cleared after a `p` in the
  doorway. While it lasts she blocks the herbal, her cupboard and the
  poison recipe. **Repro:** `audit/s2/heal1.py` to `heal3.py`. **Fix:**
  someone keeping shop steps aside for a customer.

#### PB-078 Goods taken from a stall in front of its keepers: nobody minds, and nothing is said
*Minor · town and ai · systems2 · not checked*
- Nine keepers saw it; none fears her, objects or calls the watch, and no
  line says what was taken. **Where:**
  `mods/core/data/world/stores.json:34` ("to be taken"). **Repro:**
  `audit/s2/steal8.py`. **Fix:** a rule for goods taken in the sight of
  their keeper, or a guide that says taking is free (the release notes say
  "no trade or money").

### Crafting and items

#### PB-039 The make list reads like a parts list
*Minor · crafting and items · code, code2, content, newplayer, systems · verified*
- Examples: "Bandage  hand: 1 strip of cloth gives 2 bandage: 1 more of
  strip of cloth is wanted", "tool: cutting 1", and the reason "it needs
  know-how of Herbalism". The bandage's description says "into a bandage"
  and the recipe makes two. **Where:**
  `crates/clash-sim/src/recipe/options.rs:146-165`, `:215-229`;
  `recipe/supply.rs:229-237`, `:316-323`; `recipes.json:8`. **Repro:**
  `SQ`, `shift+m`. **Fix:** use the reader's `count(n, word)`, name
  qualities for what they do ("something that cuts"), and write sentences.

#### PB-051 An opened wardrobe, cupboard or chest can never be shut again
*Minor · crafting and items · newplayer, systems2 · verified*
- Afterwards the list reads "Open: It does not open." and "Close: It
  cannot be shut now." for good. The opened container's device occupies
  its own cell, so `core:close`'s `len(here.occupants) == 0` never holds.
  **Where:** `mods/core/data/interactions.json:9-15`. **Repro (seed 1):**
  `LS`, then `l, l, k, k, k, k, k, e, enter, e, down, enter`. **Fix:**
  count only creatures in `close`'s `if`.

#### PB-059 "Bandage your wounds" is offered, and uses up a bandage, when every wound is already closed
*Minor · crafting and items · code, content, systems2 · verified (code)*
- `len(self.wounds) > 0` counts closed wounds too, so with nothing open
  the action closes nothing, uses a bandage and logs "Ada binds their
  wounds.". Pressed twice in a fight, it spends both of the last stand's
  bandages for one binding. **Where:**
  `mods/core/data/making/actions.json:53-62`. **Repro:**
  `audit/s2/band*.py` (three bandages became one). **Fix:** an
  `open_wounds` property, and `len(self.open_wounds) > 0` in the `if`.

#### PB-075 Put in… offers the thing itself, and not the containers within a step
*Minor · crafting and items · content, systems2 · not checked*
- Choosing the thing itself gives "The parcel cannot go inside itself.".
  The bench and racks a step away are not listed, though the guide says
  "or that stands within a step". **Where:**
  `mods/ui/data/inventory.json:49`. **Fix:** list containers carried or
  within a step, other than the thing.

#### PB-076 Take out with both hands full does nothing and says nothing
*Minor · crafting and items · systems2 · not checked*
- **Fix:** grey the row out with "Your hands are full.".

#### PB-079 Recipes that give nothing: Distil, Torch, Grow herbs
*Minor · crafting and items · content · verified (code)*
- Distil turns 400 ml of ale into 100 ml of the same ale; a torch cannot
  be lit; Grow herbs needs water on the cell, and nothing in the world puts
  it there. How-to-play lists all three. **Where:** `recipes.json:13-17`,
  `:89-105`; `making/items.json:88`. **Fix:** a spirit, a torch that can
  be lit, and a way to water a bed; or take the three out of the guide.

#### PB-080 Bread cannot be made at a hearth, though how-to-play says so
*Minor · crafting and items · systems, systems2 · verified (code)*
- Bread's `"at"` is `["oven"]`, and the hearth device carries only the tag
  `hearth`, so the tavern hearth's Make list has no Bread, not even greyed
  out. **Where:** `recipes.json:47`; `stations.json:33-38`. **Fix:**
  `["oven", "hearth"]`, or correct the guide; and list greyed the recipes
  a station cannot make.

#### PB-081 The salve's bench is in the healer's house, but nothing there serves as its hammer
*Minor · crafting and items · systems2 · not checked*
- The recipe needs "hammering 1", which only a club gives; clubs are in
  the locked armoury, or made at a bench. **Where:**
  `maps/trades.json:116`; `making/items.json:160`. **Fix:** a pestle in
  the healer's house.

#### PB-082 Nightshade extract poured into a mug or a waterskin can never coat a blade
*Minor · crafting and items · content · not checked*
- Only the vial offers Coat a blade, and a quarter of games have no vial
  in the healer's cupboard. **Where:** `making/items.json:100`, `:112`,
  `:120`; `tables/tables.json:98-99`. **Fix:** `coat_blade` on every
  vessel, or a recipe that asks for a vial.

### Content and text

#### PB-045 Map glyphs collide, and how-to-play's legend is wrong
*Minor · content and text · content, newplayer, systems · verified (code)*
- Stairs are drawn `^ > v <` by the way they face, while the legend gives
  `^` as fire and has no `v`. Ground ramps are ribbons of `<`, and roofing
  is `~`, the legend's smoke or water. Loose things reuse fixtures'
  glyphs: a club or knife is `/` (an open window), a bandage `+` (a shut
  door), a bottle or vial `!` (a lamp, and the heard mark), a parcel, log
  or book `=` (a shut window). `o`, `I` and `O` are not in the legend.
  **Where:** `mods/core/data/world/furnishing.json:8-19`,
  `blocks.json:41-44`, `items.json:39`, `:47`, `making/items.json:86`,
  `:122`; `docs/how-to-play.md:91-101`. **Fix:** one glyph for stairs (or
  explain the arrows), distinct glyphs for loose items, and a legend made
  from the data.

#### PB-064 The Training choice (Runner, Reader) does nothing
*Minor · content and text · content · verified (code)*
- Three creation points go on the tracks `athletics` and `lore`, which
  nothing reads. **Where:** `life/templates.json:20-23`,
  `stats.json:22-23`. **Fix:** make something read them, or leave the
  choice out.

#### PB-083 The herb beds, where the nightshade grows, look exactly like the grass
*Minor · content and text · content · verified (code)*
- The bed is `"` in green, and so is grass. Finding the one bed that grows
  nightshade means looking at every grass cell with `x`. **Where:**
  `making/stations.json:77`, `world/floors.json:22`. **Fix:** a glyph of
  its own, and a line in the legend.

#### PB-084 Weapons and gear that no bundled game offers
*Minor · content and text · content · verified (code)*
- The rifle, its magazine and rounds, hollow-point pistol rounds, the
  scope, the suppressor, the throwing knife, the charge and the gorget are
  defined but placed by nothing, though the release notes promise rifles
  and throwing. **Where:** `combat/arms.json`, `combat/takedowns.json:3`.
  **Fix:** stock them in the armoury, or say they are for mods and test
  scripts.

### Scenarios

#### PB-041 Losing the last stand: no line says the character died, and the summary marks nothing failed
*Minor · scenarios · newplayer, systems · verified*
- The log runs "falls senseless" … "The game has ended: lost." with no
  "dies" line, because an unconscious character perceives nothing. The
  summary shows "Lost." with "[ ] Hold out until dawn." still unmarked.
  **Repro:** `audit/sys/ld2.txt` (Shift+P, Enter, repeated, at seed 4).
  **Fix:** tell the party its own death whatever it perceives; mark the
  failed objective `[-]` and say how it was lost.

#### PB-062 Two of the five games on the new-game list are empty test places
*Minor · scenarios · content · verified*
- "The town" ("A grid of streets. Every door stands open.") has no people
  and no objective; "The lane" has nobody in it. The release notes say
  "Three game scenarios". **Where:**
  `mods/core/data/scenarios/towns.json:19-33`. **Fix:** move them to the
  dev profile, or label them as test places.

#### PB-098 The "town square" start is one cell outside the square, in the watchman's cell
*Minor · scenarios · content · not checked · known*
- The game begins at `@c:2007,994,0`, on the pavement off the square's
  corner, with the watchman in the same cell. The square's `centre` mark
  is not used. **Where:** `town_life.json:12`, `maps/town.json:169`.
  **Repro (console):** `use default seed=4`, `begin core:town_life
  place=1`, `look @self`. **Known:** sixth round, known issue 4. New: the
  start is not in the square at all.

### Docs

#### PB-100 The release notes' Known issues leave out what the sixth round found
*Minor · docs · docs · verified (code)*
- Missing: the log telling a blow's consequences before the blow; a
  projectile called "Something"; no pronouns; the start in the watchman's
  cell; Take out's order; `e`'s short second list; a binding broken off
  unsaid. Commit `5b10e9c` updated only the round's highlights. **Fix:**
  add them.

#### PB-101 The generated reference flattens tables, drops code blocks and speaks Rust to modders
*Minor · docs · docs · verified (downgraded from major: the page reads, if roughly, and nothing in play depends on it)*
- In `docs/reference/values.md`, the page the guide links as how values
  are written, tables come out on one line; sentences ending in ":" lose
  the code block that followed; "## Value sources" is empty; there is a
  dead link, `[value source](ValueSource)`; and it names `Ref`,
  `BTreeMap`, `Def::compile` and `Text::render`. The same happens in
  `types/action.md:128`, `goal.md:27`, `:66`, `method.md:31` and
  `movement.md:25`. **Fix:** keep table rows and code blocks, and write
  the value pages for modders.

#### PB-102 The reference says `{ "exists": query }` is not built and always false; it works
*Minor · docs · docs · verified (code)*
- `values.md` "Conditions" says "not built yet: it counts as false", but
  `crates/clash-data/src/expr/field.rs:1329-1335` evaluates it. **Fix:**
  remove the note.

#### PB-103 The reference marks fields "Carried until X is built" where X has long been built
*Minor · docs · docs · not checked*
- Such notes are on 18 pages: a trait's `goals`, `weights`, `senses`,
  `bound` and `grants`; an item's `traits`, `ages` and `keeps`. A modder
  cannot tell which of these fields still do nothing, and none draws a
  warning. Item `traits` do work now (`item/confer.rs:156`). **Fix:** one
  marker for "accepted, does nothing yet", and a lint warning when such a
  field is given.

#### PB-104 The reference says a template's `items` are made; nothing reads them, and their ids are not checked
*Minor · docs · docs · verified (code) · known*
- **Where:** `docs/reference/types/template.md:29`, `:103`. **Known:** the
  release notes say starting items are not made. New: the reference says
  the opposite, and lint is silent.

#### PB-105 Verb entries have no examples, and half do not say why they may be refused
*Minor · docs · docs · not checked*
- 174 entries; 81 never mention a refusal (`pool_add` among them), though
  the guide promises it. Events and hooks have no examples either.

#### PB-106 The roadmap calls built parts "not built" and lists `beta` packages as the work left
*Minor · docs · docs · not checked*
- Rows 03, 06 and 09 against I8, L1, L2, T7 and E11; "After the beta" in
  `docs/roadmap.md`.

#### PB-107 The test scripts' README shows an example that does not run
*Minor · docs · docs · not checked*
- `use profile=test_core` fails with E0008; `gen flat` and a made-up hash
  follow; the `known_failures/` directory it names does not exist.
  **Where:** `scenarios/README.md:13-27`.

#### PB-108 The modding guide's example game is played in an empty void
*Minor · docs · docs · not checked*
- On `core:testland` only `d @ ?` is drawn on black, and Look says "Ground
  / Nothing known.". **Fix:** use `core:open_town` or the countryside
  (see PB-031).

### Modding and tools

#### PB-099 Headless plain frames leave out sprites during playback, the player's own `@` included
*Minor · modding and tools · systems · verified*
- At the default playback speed, `frame` has no `@` after a step, while
  `frame golden` lists it. Agents and tests read these frames; the content
  lens took it for a bug in play (see the dropped entries). **Where:**
  `crates/clash-app/src/headless.rs`, as against the terminal's
  `snap_sprites`. **Repro (seed 4):** `enter, enter, enter, l, l, frame`
  with and without `ui:playback_speed` 0. **Fix:** snap sprites into plain
  frames, as the terminal does.

#### PB-109 Diagnostics put a cascade before its cause, or point at the wrong place
*Minor · modding and tools · docs · not checked*
- (a) An expression type error is preceded by an E0701 with no file or
  line. (b) A misspelt rule event (E0684) is followed by two false E0621
  errors about `self`. (c) The caret for a misspelt sense is under the
  template's id. (d) A broken JSON file causes E0626 errors in another
  file without saying it did not load.

#### PB-110 `"required": false` on a screen part, as the guide and `init-mod` teach, hides a misspelt slot
*Minor · modding and tools · docs · not checked*
- `ui:play.barz` lints `ok` and shows nothing. With `true` it is E0501,
  with "did you mean `ui:play.bars`".

#### PB-111 Pool and track properties are typed never-null but documented as null, so W0644 misses them
*Minor · modding and tools · docs · not checked*
- `self.pool.level('core:stamina') * 2` lints clean, while
  `self.carried_mass * 2` warns. **Where:**
  `docs/reference/properties.md:119`, `476`, `588-595`, `637-639`.

#### PB-112 `new <type>` skeletons do not lint, and the checks speak of work packages
*Minor · modding and tools · docs · not checked*
- `new scenario` gives only `type` and `id`; lint then says "several
  worlds wait for charts (10 §9.4)" and "parties wait for package SC2",
  with carets under `"id"`.

#### PB-113 `core_ui` does not require the mods it reads: alone it gives 34 errors
*Minor · modding and tools · docs · verified*
- `mods/core_ui/mod.json` has `"requires": []`, so `lint mods/core_ui`
  gives 34 errors, none saying that core and ui were not loaded. **Fix:**
  `"requires": ["core", "ui"]`.

#### PB-114 `save verify` without `--mods` takes the data directory's own `mods/` and fails
*Minor · modding and tools · docs · not checked*
- It reports "the save's profile `default` does not load: E0008".
  **Where:** `crates/clash-tools/src/save.rs:91`. **Fix:** search for mods
  as the game does, or document `--mods`.

### Engine

#### PB-115 The step and the interaction list still read the true world: a hidden element is found by walking into it
*Minor · engine · code · verified (code)*
- `believed_step` ("Stand-in: reads the world as it is") and
  `interaction/belief.rs` ("reads ground truth for now") are still in
  place, though perception is built and `believed_path` uses it (D-619).
  A secret door not yet noticed refuses with "the way is closed" and
  offers its real Open; a truth `if` greys out Close for someone unseen.
  No bundled mod has hidden elements. **Where:**
  `crates/clash-sim/src/movement/belief.rs:44-51`;
  `interaction/belief.rs:1-46`; `crates/clash-app/src/host.rs:529-541`.
  **Fix:** route both through perception's check for noticed hidden
  elements (`perception/paths.rs:97-112`).

#### PB-116 Mind-context functions locate an entity where it truly is
*Minor · engine · code, code2 · verified (code)*
- `away_from` and `aside_from` are registered as safe, with no belief
  form. The entity forms of `could_perceive`, `remembers` and
  `knows_element` use the entity's true cell. So a goal, method or tactic
  learns where someone out of sight really is. The bundled mods use them
  only in truth contexts. **Where:** `action/windows.rs:1085-1117`,
  `combat/mod.rs:250-283`, `perception/props.rs:57-66`. **Fix:** resolve
  entities through `believed_loc` in a mind context (D-780).

## Polish

The 56 polish entries by area: ui 18, docs 10, crafting and items 7,
modding and tools 7, combat 5, powers 3, saves 3, content and text 2,
scenarios 1. The first fifteen:

| Id | Area | Title |
|----|------|-------|
| PB-117 | powers | The log never names the power: "Mara uses a power." |
| PB-118 | ui | Pickers name the parameter, describe nothing, and start on yourself |
| PB-119 | crafting and items | Use lists actions on things not held, with a lower-case reason |
| PB-120 | crafting and items | Refusal texts that do not fit the case |
| PB-121 | ui | Picking up, dropping, opening and taking say nothing |
| PB-122 | saves | Leaving the last stand warns "What is not saved is lost", then saves |
| PB-123 | crafting and items | The inventory offers Wear for a club or the parcel |
| PB-124 | ui | Pickers that offer yourself or "Nobody there" |
| PB-125 | ui | The title shows the game's name twice |
| PB-126 | ui | The screens' own key hints are never shown |
| PB-127 | ui | The help screen: one alphabetical list, ",, g", and "-" for mouse-only controls |
| PB-128 | ui | The same things have different names on different screens |
| PB-129 | ui | Small wording on the play, title and options screens |
| PB-130 | ui | Options: walking preferences under Display, no Effects choice, lower-case labels |
| PB-131 | ui | Condition: "57% whole", "2 deep", and no sedation row |

### Saves

#### PB-122 Leaving the last stand warns "What is not saved is lost", then saves it
*Polish · saves · content, newplayer, systems2 · verified*
- **Where:** `mods/ui/data/app.json:121`. **Fix:** a text for each saving
  policy ("It is saved, and Continue carries it on.").

#### PB-134 Save and load lines show the file's id and an odd play time; loading says nothing
*Polish · saves · newplayer · verified*
- "Saved as “Ada” (ada)."; the list line "Ada  Ada, The market town,
  0:00:00 in · …" (the name twice, and no game clock).

#### PB-135 Saves cannot be deleted in the game, and the list's date is the file's
*Polish · saves · newplayer · not checked*
- There is a `delete_save` service (`crates/clash-app/src/app.rs:32`) that
  no screen uses.

### Combat

#### PB-138 A parried blow is told as "cannot strike: what was put in the way stopped all of it"
*Polish · combat · newplayer, systems2 · not checked*
- The line has no full stop and does not name the parry. **Where:**
  `core_ui/data/shows.json:25-27`.

#### PB-139 The precise attack asks again for the chosen target, then asks "effort: Value 1"
*Polish · combat · newplayer · verified*
- It shows "Kick: target  Person (1 of 1)" again, then "Kick: effort
  Value < 1 >" with no scale (0 to 1).

#### PB-140 With the target out of reach, blade blows head the precise list with "out of reach" as their reason
*Polish · combat · content · not checked*
- Empty-handed, it lists "Slash: That is out of reach." before Kick. With
  the target in reach the list is right ("needs a blade in hand").
  **Where:** `combat/fighting.json:67-111`, `154-176`.

#### PB-141 `v` and Shift+S with nobody in sight open their lists anyway, titled "Choose"
*Polish · combat · newplayer · not checked*
- `f`, `s` and `t` say "Nobody is in sight to choose." in the same case.

#### PB-142 Reload with a bow says "There is nothing in hand to load."; the quick shot gives one reason for three
*Polish · combat · systems2 · not checked · known in part*
- The quick shot says "There is no bow in hand, no arrow to nock, or
  nowhere to aim." whichever it is. **Known:** round 2, known issue 6
  (since reworded).

### Powers

#### PB-117 The log never names the power used, called up or let go
*Polish · powers · content, newplayer, systems · verified · known in part*
- Every power gives "Mara uses a power." and every mode "calls up a
  power.". The hound and the stone wall appear with no line. A broken
  window is "breaks the wall", and a fist at a door "strikes something".
  **Where:** `core_ui/data/shows.json:49-58`. **Known:** sixth round,
  known issue 2 (a projectile is "something"). New: powers, a door and a
  window.

#### PB-136 Quick slots are kept in the preferences, shared by every character
*Polish · powers · newplayer · not checked*
- A slot set as the adept shows for another character in another game.

#### PB-137 Fire bolt shows Rank 0 beside powers of Rank 1
*Polish · powers · newplayer · verified*
- Its rank is its pyromancy track's level.

### UI

#### PB-118 Pickers name the parameter, describe nothing at the cursor, and start on yourself
*Polish · ui · content, newplayer, systems · verified*
- The titles are "Fire bolt: to", "Knock out: victim" and "Smash: at", and
  a cell is described only as "In sight, 1.4 m. In reach.". Power pickers
  start on your own cell even with a target chosen, and the footer says
  "Ctrl-D" (`pickers.json:12`, `:15`). **Fix:** ask a question ("where?"),
  say what is at the cursor, and start on the target.

#### PB-121 Picking up, dropping, opening, taking and breaking a window say nothing
*Polish · ui · newplayer, systems2 · not checked*
- Only failures are told, so a player learns to read silence as success.

#### PB-124 Pickers that offer yourself or "Nobody there" when nobody fits
*Polish · ui · newplayer, systems2 · not checked · known in part*
- Examples: "Give: to  Mara (0 of 0) … You have it already.", Throw's
  first candidate being yourself, and "Kick: target  Nobody there (1 of
  1)". **Known:** round 1, known issue 6, since fixed for `f`, `s` and `t`.

#### PB-125 The title shows the game's name twice
*Polish · ui · newplayer · verified*
- "C L A S H 2 D" is shown over "CLASH2D", the profile's name
  (`app.json:11-12`).

#### PB-126 The screens' own key hints are never shown
*Polish · ui · newplayer · not checked*
- The title, New game and Make a character each have a hint, but the tip
  row always shows the focused widget's tip instead.

#### PB-127 The help screen: one alphabetical list, the comma key as ",, g", and "-" for mouse-only controls
*Polish · ui · content, newplayer · verified*
- The context menu ("What can be done here  -") has no key at all.
  **Where:** `help.json:67`, `controls.json:56`.

#### PB-128 The same things have different names on different screens
*Polish · ui · newplayer · not checked*
- The hints say "i things", "c body" and "f hit" where the screens say
  Inventory, Condition and Attack; "Use what is near" is also "Use
  something near"; Abilities are also Powers; "Stam" is also "stamina";
  and the target is written four ways.

#### PB-129 Small wording on the play, title and options screens
*Polish · ui · content · not checked*
- "Target The watchman"; New game's tip "Choose where to begin."; the
  themes Day and Night, both light text on near-black; "Its doors are
  shut. Live there."; "the Watch". **Where:** `play.json:15`,
  `app.json:15`, `:77-78`, `town_life.json:4`, `groups.json:87`.

#### PB-130 Options: walking preferences under "Display and playback", no Effects choice, lower-case labels
*Polish · ui · content · verified*
- The `effects` preference (shake and flash) is defined
  (`playback.json:3-6`) but not offered; "the key" and "means" are in
  lower case.

#### PB-131 The condition screen: "57% whole", "2 deep", and no row for sedation
*Polish · ui · newplayer, content · verified*
- The status column shows Drowsy, but the condition screen has no matching
  row (`core_ui/data/bars.json:21-23`, `:38-65`).

#### PB-132 The look panel names things without articles: "Club and 2 bandages."
*Polish · ui · content · not checked*
- **Where:** `play.json:42`, `:46`.

#### PB-133 "Unconscious, and cannot act until they come to": no full stop, "they", and written for every key
*Polish · ui · newplayer · verified*

#### PB-143 An empty quick slot says "There is nothing to do."
*Polish · ui · newplayer · not checked*

#### PB-151 Stature's value hides its step, so Right seems to do nothing
*Polish · ui · newplayer · not checked*
- It shows "< 1.0 >" twice and then 1.04.

#### PB-152 Two different seeds, both labelled "Seed"
*Polish · ui · newplayer · not checked*
- New game's seed is the world's; Make a character's (with Roll) is for
  choices left to chance.

#### PB-153 With a save present, the title at 80×24 drops its tip line
*Polish · ui · newplayer · not checked*

#### PB-154 After an ending, New game keeps the last name, and keys typed at the title land in it
*Polish · ui · newplayer · not checked*
- The name becomes "Marap", and Enter begins the last stand.

#### PB-155 Wide characters in a name push the borders out; a seed past the maximum is clamped silently
*Polish · ui · newplayer · not checked*

### Crafting and items

#### PB-119 The Use list offers actions on things not held, with a lower-case "that is out of reach"
*Polish · crafting and items · content, newplayer, systems · verified*
- Examples: "Bandage your wounds  north-east: that is out of reach"
  (unhurt, with bandages on the table) and "Drink  east: that is out of
  reach" for a waterskin on the table. **Where:** `actions.json:54-55`
  (`reach: held`). **Fix:** `shown_if` for what is not carried, or "Pick
  it up first.".

#### PB-120 Refusal texts that do not fit the case
*Polish · crafting and items · content, systems · verified*
- "It does not open." for an open door, "It cannot be shut now." for a
  shut wardrobe or window, "It needs fuel to light, and must be cold."
  for a lit oven, "you need its key" when any key will do. **Where:**
  `interactions.json:6`, `:14`; `stations.json:92`; `locks.json:20`.

#### PB-123 The inventory offers Wear for a club or the parcel
*Polish · crafting and items · newplayer, systems · verified*
- Choosing it gives "The club is not something to wear.".

#### PB-144 Eating and drinking lines: "eats something", "2 loafs", "drinks the waterskin"
*Polish · crafting and items · systems2 · not checked*
- The engine's fixed list of plurals (`reader.rs:180-197`) has no "loaf",
  and a mod cannot give one.

#### PB-145 "Coat a blade" runs with any second thing in hand, and says it coated one
*Polish · crafting and items · code · not checked*
- **Where:** `actions.json:72-79`. **Fix:** test for a held blade
  (`self.wielding('blade')`).

#### PB-146 Item refusals say "is" of a stack: "2 bandages is not something to wear."
*Polish · crafting and items · code · not checked*
- **Where:** `crates/clash-sim/src/item/fault.rs:243-360`.

#### PB-147 The know-how screen prints a list in brackets and never says where to learn
*Polish · crafting and items · content, newplayer · not checked*
- **Where:** `making.json:30-32`.

### Content and text

#### PB-148 Things the world is furnished with that do nothing: empty bottles, rags, coins, the street-lamp rule
*Polish · content and text · content · verified (code)*
- One kitchen drink in six is an empty bottle. The rules that light street
  lamps and sound fire alarms act on fixtures no map places, so the town is
  unlit at night. **Where:** `goods.json:139-152`, `rules/town.json:14-48`.

#### PB-149 Greetings say "Morning." at any hour
*Polish · content and text · content · not checked*
- **Where:** `social/lines.json:62`.

### Scenarios

#### PB-150 The errand's town hall is "told of", but the map's Places list stays empty, and its marker text is cut
*Polish · scenarios · content · not checked*
- **Where:** `the_errand.json:2`, `:21`; `play.json:78-80`.

### Docs

#### PB-156 How-to-play's last-stand tip says the sky lightens before dawn; it does not
*Polish · docs · content · not checked*
- The sky's light is 0.03 from 19:00 to 5:00 (`docs/how-to-play.md:601-602`,
  `lighting.json:28-33`).

#### PB-157 Small mismatches between the guides and the game
*Polish · docs · docs, newplayer · verified (the pause menu)*
- The pause menu has Load only once a save exists (how-to-play:548). The
  guide's example screen (how-to-play:66-84) differs from the game's.
  `clash --help` says the data directory keeps only "preferences and key
  bindings" (`cli.rs:36-37`).

#### PB-158 Counts out of date: the README's 41 test scripts, and the mods' "1.2 MB in 127 files"
*Polish · docs · docs · verified*
- There are 52 scripts, and 128 files with 0.80 MB of content
  (`README.md:194`).

#### PB-159 The units page garbles its rates table and repeats an anchor
*Polish · docs · docs · not checked*
- **Where:** `docs/reference/units.md:22-32`.

#### PB-160 A code comment cites D-952, which does not exist (D-951 is meant)
*Polish · docs · docs · not checked*
- **Where:** `crates/clash-sim/src/ai/think.rs:1752`.

#### PB-161 Plans: stale notes in packages marked `done`, and a beta definition the beta does not meet
*Polish · docs · docs · not checked*
- `sim-kernel.md` K1; `first-playable.md` U2; `beta.md` "What the beta is".
  Eleven `beta` packages depend on packages still `todo`.

#### PB-162 The largest reference pages have no index, and section references are not links
*Polish · docs · docs · not checked*
- `verbs.md` has 174 entries, `events.md` 187 and `reasons.md` 259.

#### PB-163 The guide's stamina bar duplicates one core_ui already shows
*Polish · docs · docs · not checked*

#### PB-164 Code notes and one reference line still say grips, perception, recurring rules and collapse are not built
*Polish · docs · code, code2, docs · verified (code)*
- **Where:** `movement/belief.rs:16-21`, `interaction/belief.rs:5-8`,
  `rule/def.rs:153`, `combat/reason.rs:45` (copied into
  `docs/reference/reasons.md:63`), `combat/mod.rs:34-37`,
  `strike.rs:128`, `push.rs:19-21`, `ranged/mod.rs:28-32`,
  `clash-world/src/edit.rs:73-76`; and the console's `help spawn`.

#### PB-165 Stale mod descriptions and docs a modder reads
*Polish · docs · content · not checked*
- Examples: core "Used by the engine's tests and tools", ui's "talking (a
  stub)", "a brawler of strength 80", and "all five" powers where there are
  twelve. **Where:** `mods/core/mod.json:6`, `mods/ui/mod.json:6`,
  `grips.json:15`, `powers.json:2`.

### Modding and tools

#### PB-166 `init-mod` accepts a bundled mod's id and writes a mod that requires itself
*Polish · modding and tools · docs · not checked*

#### PB-167 `clash-tools` panics with a backtrace when its output is piped to `head`
*Polish · modding and tools · docs · verified*
- `clash-tools schema --type item | head -c 100` gives "failed printing to
  stdout: Broken pipe".

#### PB-168 `find` with a misspelt field or an unreadable value finds nothing, silently
*Polish · modding and tools · docs · not checked*
- `find item "where=mas>1"` prints "0 definitions" and exits 0.

#### PB-169 Console help has lines up to 477 characters wide, and stale words
*Polish · modding and tools · docs · not checked*

#### PB-170 `explain` does not know the `X…` and `E14…` codes the console prints
*Polish · modding and tools · docs · verified*
- `explain X204` says "there is no diagnostic code `X204`" and exits 1.

#### PB-171 `save list` of a folder that does not exist says nothing and succeeds
*Polish · modding and tools · code2 · verified*
- **Where:** `crates/clash-save/src/tools.rs:122-125`.

#### PB-172 Two engine fallbacks put an interaction's qualified id in a refusal
*Polish · modding and tools · code · not checked*
- They read "`core:switch` cannot be done now" and "that does not offer
  `core:open`" (`interaction/mod.rs:505-531`). No bundled interaction
  triggers either.

## Dropped

- **"Your own `@` is not drawn when you stand in the dark"** (content,
  major). Not reproduced. With playback speed 0 the `@` is drawn at every
  step into the dark. At the default speed the plain headless frame drops
  sprites in mid-glide, and `frame golden` still lists the `@`. This is a
  headless artefact, kept as PB-099.
- **"Save and load at odd moments hold"** (systems): a record that nothing
  was wrong, kept under "Played and found sound".
- **"Crouching past a sleeper works as the guide says"** (systems2): it
  works. The missing sign is folded into PB-044.

Merged rather than dropped: the fire bolt's charge (into PB-018); the five
lenses' make-list wording (PB-039); the three lenses' throwing (PB-012);
the takedown findings (PB-021); and the deaths in the town (PB-004,
PB-040).

## Known issues the audit found no longer true

- **"Last stand is a gamble every way"** (release notes): not for the
  gifted person with Shadow on, who survives by standing on the lane
  (PB-055).
- **"A minute waited in the square takes under a second"** (release
  notes; the Measured table says 0.8 s a minute and about 6 s an hour):
  the audit measured 1.3 to 1.5 s a minute and 10.6 to 12 s an hour in
  the square, and 10 s a minute by the errand's market (PB-003), on a
  shared four-core machine.
- **"There is no key to lie prone"** (release notes): still true of keys,
  but the town's rest rule lays the player prone (PB-020).
- **"The bundled mods: 1.2 MB in 127 files"** (release notes, Measured):
  128 files, 0.80 MB (PB-158).
- **"The errand cannot be lost by a player in practice"** (playtest, round
  1, known issue 2): it was lost at nightfall by five Shift+P from the
  start, in 92 s of real time (systems2).
- **"With nobody in sight, the quick attack opens a target picker on the
  character itself"** (round 1, known issue 6): `f`, `s` and `t` now say
  "Nobody is in sight to choose."; other pickers still offer yourself
  (PB-124).
- **"Struck by the player, the watch does not answer"** (round 2, known
  issue 1): the watch now answers at once, and kills (PB-004).
- **"The quick shot says the pistol's reason when the bow cannot shoot"**
  (round 2, known issue 6): it now gives one sentence for the bow's three
  reasons (PB-142).
- **"The target cycle includes the dead"** (round 2, known issue 7): fixed
  (D-944), except for a target who dies in your own cell (PB-089).
- **"The bundled world has no source of nightshade"** (round 4, known
  issue 4), and the earlier rounds' "a bandage made, a poisoned blade, a
  disguise were not played": all were played, by the sixth round and
  again in this audit (systems2).

## Played and found sound

- Saves verify (`clash-tools save verify`) at odd moments: in a time stop,
  while knocked out, after a death, mid-fight, at night, after a fire.
  Continue resumes a time stop. `replay` from one save to a later one
  reports every subsystem the same. All 52 bundled test scripts pass in all
  four modes with the audit's binary.
- The errand was delivered and was failed at nightfall. The last stand was
  survived and lost. Quit, Continue and the one save being gone at dawn all
  happen as documented.
- Played in full: bread baked (up to PB-024), nightshade pressed, a blade
  coated (one cut drops the healer), salve rubbed on, arrows made, the
  hooded cloak made, a loaf eaten, a waterskin drunk, herbalism learnt, and
  `recipe.no_room` refused in words.
- Time stop then eight blows: the watchman falls and stays down. A throw
  from the next cell works, as does a choke from behind and crouching past
  a sleeper.
- No panic reachable from content, saves or input, and no determinism
  hazard, were found by reading the code. Content that could freeze the
  game is refused at load. `clash-tools docs` regenerates the reference
  with no difference, and `clash-tools lint mods` is clean.
- Every screen key in the market town answers in under 15 ms.
