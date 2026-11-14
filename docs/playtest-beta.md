# Play-testing the beta

Every system of the beta was built by its own stream and passed its own
tests. This is what happened when the game was first played from the
title to an ending, as a player plays it: what each session found, what
was fixed and in which commit, what is still wrong and how a player
meets it, and how long the game takes to answer a key.

The sessions are headless tests of the application,
`crates/clash-app/tests/play_*.rs`, driven by keys from the title screen
through the real application on the bundled profile (the powers on the
`armoury` fixture, whose game gives someone every power of the sampler:
when the sessions were written no bundled game let a player have
powers). Nothing in them moves the world by a
debug command: they read the game's state only to check it, and to find
the way across town the character believes leads somewhere, which is
the search its own `go_to` makes, turned into the direction keys a
player would press. What they share is `crates/clash-app/tests/session/`.

First played at commit `ced8d26` on the build branch as of `e238730`,
and played again after merging the essentials stream (`cb07bdc`:
playable characters, picking up and the inventory's actions, names for
players), which the sessions now pass on, and merged again with the
town life stream (`6218d6f`: residents as people with daily routines,
the watch, staff for the trades, minds opening doors). The findings and
timings below were made before town life; the sessions were run again
on it, but there was no time left to play the fight in the market with
the watch answering, or a day of routines, which its own test script
`town_disturbance` and tests cover.

## The sessions

| Session | Test | Plays | Takes (debug) |
|---------|------|-------|---------------|
| The errand, delivered | `play_errand.rs` | Title, new game, the introduction, the clock, the journal's way (`south, 129 m`), 60 steps through the streets, saved by name from the pause menu, quit to the title, Continue, the rest of the way, the ending `delivered` and the summary | ~3.5 min |
| The last stand, inside | `play_last_stand.rs` | The lit lane, the journal (`north-east, 3 m`, the door), through the door by walking into it, the door shut from inside, quit and Continue (the game's one save), and waiting: since town life the strangers open the door and come in | seconds |
| The last stand, lost | `play_last_stand.rs` | Staying on the lane and hitting back: the strangers' knives end it in about half a minute of game time, `Lost.`, the one save gone | seconds |
| Powers | `play_powers.rs` | Each of the sampler's twelve powers once from the abilities screen: dash, force push, fire bolt, heal, gravity well, summon hound (a dog appears), stone wall (the street changes), and the five modes switched on and off with their lines in the log; stamina and reserve shown | ~10 s |
| Crafting | `play_crafting.rs` | To the market town's bakery, wood out of its rack, the oven stoked and lit, its stores opened, bread made (two minutes of work in one key), the loaf taken out of the oven | ~5 min |
| The market town | `play_town.rs` | Begun in the square (setup, introduction), look mode, to the market by the streets, saved and loaded from the pause menu, a resident targeted with Tab and talked to, a shut door opened by walking into it, a home's cupboard opened and emptied into the hands | ~1 min |

Each save and load in them checks that the state hash is what it was,
and that the character stands where it stood. Golden frames are kept for
the moments that matter: `play_errand_delivered`, `play_stand_lane`,
`play_stand_lost`, `play_town_square`.

Not played: the fight in the market with the watch answering (there is
no watch until town life lands), residents keeping their routines
through a day (town life; and a day of waiting costs too much real time,
below), making a character from the title (essentials' own tests play
it), the errand
failed (below), a bandage made and a wound dressed, and a blade coated
with poison (below).

## What was found and fixed

In commit `ced8d26`, with decisions D-870 to D-878, but for the last
stand's dawn (D-879) and the keys of the long waits, which came after.

| Found | How a player met it | Fixed |
|-------|---------------------|-------|
| The log filled with `Fault: \`follow_path\` declined` and `Fault: \`plan_path\` declined`, dozens a minute in the town | Walking anywhere in the market town | The movement verbs give null for what moving comes to, a refused step or no way, and say why with `step_refused` and `path_failed`; a refused step of the player's own is a line in the log (D-870) |
| Time could only be waited a second at a time | The last stand is eight hours long: 28,800 presses of `.`; the errand's deadline five | `p` passes a minute, shift+p an hour; a rule of `core` breaks the wait off when a foe comes into sight or the character is wounded, and `interrupt_plan` now ends a wait (D-871) |
| No screen said what time it was | The errand is due by nightfall, the last stand ends at dawn | `Time 14:05` in the status column (D-872) |
| What powers are paid from was not shown | Abilities cost `25 stamina`, `30 reserve` | Bars for stamina and reserve, and for toxin and sedation while they are above nothing; the condition screen lists nourishment and toxin (D-872) |
| The last stand's marker stood on a wall | The journal said `north, 2 m`; walking there said `Something is in the way.` | The refuge is the nearest door (D-873) |
| The last stand's lane was dark but for the character's own cell | Nothing to see, nothing to do | A lamp is lit by the door: the lane before the house can be seen, and the strangers seen coming (D-873) |
| The strangers never came, and the last stand could not be lost | Waiting an hour on the lane: nothing | They were blind: a mind given a stance before it first thinks never gets its mind's senses (a fault of the AI's package, worked round in the scenario). They are given senses, knives, and the house as their beat (D-873) |
| A fault said nothing anyone could act on | `Fault: \`set_fixture\` declined` | The line names the refusal's code and the definition and path of the effect (D-874) |
| Enter in a name field did nothing | Saving: typing a name and pressing Enter | Enter saves on the save screen and begins the game on the new-game screen (D-876) |
| Work said nothing | Bread took two minutes in one key press; the loaf was in the oven and nothing said so | Lines when work starts, stops (with why) and is done, and where what it gave is (D-877) |
| Make cut its reasons off | `Bread  south: … gives 1 loaf: it does not w` | The selected recipe's whole reason under the list (D-877) |
| A power held up by its reserve ended without a word | Time stop runs dry in twelve seconds | Lines when a mode is called up, let go or runs dry (D-877) |
| The abilities screen hid its fifth mode | Shadow was below the edge | The modes list has room (D-877) |
| Take out faulted once the hands were full | `Fault: \`take\` declined (item.no_free_grasp)` after emptying a cupboard | It takes while a hand is free and leaves the rest (D-877) |
| Talking to the target was refused | Tab chose Cora across the market, shift+t said `You cannot see or hear them now.` | Someone seen in view can be talked to (D-878) |
| Every creature's sense sweeps sorted all the occupancy entries in sight range, twice a second | A minute's wait in the market town took 7.9 s | Candidates gathered by id without the sort, the same results (D-875) |
| The last stand could not be won: its dawn timer, set in the scenario's `start.do`, faulted when it fell due (the rule runtime looked for a timer's effects only in actions and rules), and the game never ended | Waiting out the night behind the door: dawn came and nothing happened (found by the full-night session) | A scenario's and a mission's effects are found again where they are written (D-879, a later commit) |
| A refusal named the character by its number | Pressing keys while knocked out: `0v0 is unconscious` | `unconscious, and cannot act until they come to` (D-878) |

## Known issues

Worst first, with how a player meets each.

1. **Waiting in the market town is slow.** A minute waited takes about
   six seconds of real time, an hour some six minutes (release build,
   the timings below). The town's 469 residents each sweep two senses
   twice a second and think every second; minds whose way is blocked
   plan again and again, each failing search settling thousands of
   cells. The sweeps' gathering was made cheaper (D-875); the rest is the
   number of sweeps and searches, a model change (09's hot and warm
   detail) for the AI's package. In the countryside an hour takes about
   five seconds.
2. **The errand cannot be lost by a player in practice.** It fails if
   the parcel is lost (destroyed or out of the world) or five hours
   pass. Dropping it (essentials) does not lose it, nothing destroys it,
   and five hours of waiting in the town is half an hour or more of real
   time (above). Its failure was not played.
3. **A mind given a stance before it first thinks never gets its
   senses** (`ai::think` gives senses only when it makes the mind's
   state, and `set_stance` makes it first). The last stand works round
   it; any content that gives a stance at spawn meets it. Reported to the
   AI's stream.
4. **The last stand is brutal outside.** Three strangers with knives end
   it in about half a minute of game time for someone with bare hands
   who stays on the lane; getting in and shutting the door is the way to
   live. That is the scenario's design (an ordinary person is ordinary),
   but a player has little warning beyond the introduction.
5. **Since town life, a shut door does not keep the last stand's
   strangers out**: minds open doors on their way. Before town life the
   way to win was to get in, shut the door and wait for dawn (dawn
   itself never ended the game until D-879). Now an ordinary person
   inside is reached and, with bare hands against three knives, loses.
   Whether the night can be won now (a chosen character, a marksman or
   an adept; hiding in a wardrobe; a stone wall across the door) was not
   played. That dawn ends the game is tested in the simulation's own
   tests.
6. **The quick attack on someone out of reach picks a blow at the
   world.** With a resident across the market as the target, `f` chose
   Smash and asked for a place to strike, rather than saying the target
   is out of reach. With nobody in sight it opens a target picker on the
   character itself (`Kick: target Ada (0 of 0), 0.0 m. In reach.`).
7. **Hunger does nothing.** Food fills the `Fed` bar, which falls by the
   hour, and nothing happens when it is empty.
8. **A bandage, a wound dressed and a poisoned blade were not played
   through.** The recipes are there (cloth from a wardrobe or the market,
   the healer's herb beds and the herbal for herbalism, a vial for the
   extract), but a session of them is a long walk across a slow town and
   a wound to dress; the test script `making/poisoned_blade` covers the
   coating.
9. **For modders:** a local made by `let` from a query (`$porch`) has no
   type, so `$porch.x` reads as a map's field and faults (`a list or a
   map was needed and this is a subject`); `dist(it, $porch)` works. A
   verb's text argument is text as written, not an expression:
   `"to": "core:lamp"`, not `"'core:lamp'"`.
10. **The play sessions are slow in a debug build**: the town's ones take
    minutes each (the errand some eight minutes once town life made the
    whole town hot), for the cost of the first issue.
11. **Since town life, a fault from a resident's door reaches the
    player's log**: `Fault: \`complete_interaction\` declined
    (interaction.not_now) in action core:interact`, when a mind's way
    through a door is already open or shut by someone else. The same kind
    of fault D-870 took out of moving; for the interaction and AI
    streams.

Greeting a resident, which before town life got no answer, is answered
since (`The stall keeper says: "Morning."`).

Played before the essentials stream landed and fixed by it: character
creation offered nothing (so no bundled game gave powers: they were
played on the test fixture; a player can now be an adept), things and
people were called "something" (`Holding something` is now `Holding a
parcel`, the strangers `The person`), the inventory's verbs were refused
and nothing loose could be picked up (now `g`). The sessions were not
extended to these.

## Responsiveness

Measured with the release build on the market town at seed 4 with every
resident, headless, the time from a key to the frame that shows what it
did (the pass that carries the key out and draws the frame), on a shared
machine under load from other builds; each figure is the mean over its
repetitions less the time to reach the town.

| Key | Takes |
|-----|-------|
| A step (`j`, `k`) | 77 ms (98 ms before D-875) |
| Wait a second (`.`) | 93 ms (120 ms before) |
| Wait a minute (`p`) | 6.0 s (7.9 s before) |
| An attack (`f` on the target) | about 100 ms (Cora at the market, out of reach: the quick attack chose Smash and asked for a place, known issue 6) |
| Opening a screen (`i`, then Esc) | under 5 ms, too little to tell from the noise |
| An hour inside the last stand's house (shift+p) | about 4 s |

A step is at the edge of instant; the minute's wait is not, and is the
first known issue.

## The second round

Played from `08f2178` on the build branch (stream "play-testing 2"):
the first round's known issues that are not performance were fixed, at
their causes, what was not played was played, and the last stand was
played to dawn and tuned. The performance stream ("performance 2",
making the populated town fast) had not landed on the build branch when
most of this round was played: sessions in the market town were kept
short, and their timings are as slow as the first round's. It landed
(`3bc76b9`, D-890 to D-899) near the end of the round and was merged;
the last stand's sessions were run again on it and pass as before, and
the market fight and the morning of a day of routines were played on it
(below, and *After performance 2*).

### Sessions

| Session | Test | Plays | Takes (debug) |
|---------|------|-------|---------------|
| The errand, failed | `play_errand.rs` | The parcel dropped from the inventory on the north street, and the minutes waited with `p`: ten minutes on, `You left the parcel behind. The errand has failed.`, the ending `failed` and `[-]` in the summary | ~11 min (the populated town, after performance 2) |
| An adept, made | `play_powers.rs` | The town from the title, F2, the adept, the powers choice: the three given at first put back, Force push and Stone wall taken; Begin; the abilities screen lists those two and no others; a stone wall raised across the street | ~1 s |
| The quick attack | `fighting.rs` | `f` at someone six cells away: `That is out of reach.`, nothing struck, Smash greyed and last in the precise list; `f` with nobody in sight: `Nobody is in sight to choose.` | seconds |
| A marksman, inside | `play_last_stand.rs` | Chosen ready-made (the bow); `r` with a bow (`There is nothing in hand to load.`: each shot nocks from the quiver); inside, the door opened onto the lit porch; the long wait broken off by a stranger in sight, Tab and `s`, a dodge or a step back at each blow: twelve arrows into the first stranger, who dies at the door | ~1 min |
| The night at the door | `play_last_stand.rs` | An ordinary person and a brawler, each inside with what lies on the table taken up (by the lamp in the room), holding the door with `f`, the long wait between strangers | ~1 min each |
| The night in the dark | `play_last_stand.rs` | Inside, the lamp in the room switched off, waiting with Shift+P: never found, and at five `The game has ended: survived.`, `[x] Hold out until dawn.` | ~4 min |
| The armoury | `play_market.rs` | From the square to the guardhouse by the streets, `g` on the key lying on the sergeant's desk (`Holding a key`), the locked door of the armoury walked into: unlocked, then opened; in, and Take out from a rack: `Holding a key and a club` | ~7 min |
| A quarrel in the square | `play_market.rs` | The market town begun in the square; Tab to the nearest, up to them, Shift+T and a greeting: `Ada speaks to the watchman.` `The watchman says: "Morning."` (a stranger to him); struck with `f`: `Ada strikes the watchman.` `The watchman is wounded.`, and he walks on; half a minute; upstairs in a house by the square to its wardrobe, Hide inside, three minutes waited: nobody came | ~8 min |

The first round's sessions were run again on this round's fixes; the
golden frames that moved are named with the fixes below.

### After performance 2

| Session | Test | Plays | Found |
|---------|------|-------|-------|
| The market fight | `play_market.rs` (`a_fight_in_the_square_and_the_watch_answers`, ignored for the known issue) | Tab to someone in the square, up to them, `f` until a blow lands; then three minutes waited a second at a time, watching for the watch | `Ada strikes the watchman.` `The watchman staggers.` `The watchman is wounded.` (Tab had moved to the watchman on the way), and no watchman held her hostile in three minutes: the watch does not answer the player (known issue 2) |
| The watch, by script | (probes, not kept) | A watchman struck in the square by a rowdy; and by the player's character made delegated, with the same punches | Both answer: the watchman takes up `confront`. Only the player's own blows go unanswered |
| A day of routines, its morning | `play_market.rs` (`a_morning_and_a_noon_in_the_square`, ignored as slow) | Begun at nine in the square with the stalls kept; Shift+P three times, to noon | Too slow for the suite in a debug build: three hours among 150 hot minds had not finished after 33 minutes of real time on a loaded machine (stopped there). The quarrel and the armoury sessions pass on the merge. The routines themselves are checked by the test script `scenarios/town/routine_day.scn`, which watches from outside the town |
| The last stand, again | `play_last_stand.rs` | All six sessions on the merged build | Pass unchanged: 320 s in a debug build |
| Bread, at the cap of 50 (performance 3) | `play_crafting.rs` | The bakery session with the cap of 50 hot minds D-927 proposes | The walk to the wood met someone standing just inside the bakery's door, with no way round, and pressed its 300 keys within one minute of the clock. The sessions' walk now waits a second behind someone it cannot go round; the session passes (72 s in a debug build) |
| The errand, at the cap of 50 | `play_errand.rs` | Both errand sessions with the cap | The parcel left behind fails the errand as before; delivered, the walk heard footsteps five times rather than twelve, so the last frame's log no longer scrolls. The delivered session took 39 s in a debug build, the failed one under four minutes |
| The watch, at the cap of 50 | `clash-save` `town_life.rs` | The check with the cap given to the five bundled games | A brawl in the market: someone calls for help and the watchmen 40 m off stay at their posts (without the cap they come). The cap is held back (D-911; known issue 2) and the frame and hashes stay as they are without it |
| The errand, delivered, again | `play_errand.rs` | The first round's session on the merged build | Passes in about three minutes in a debug build. Its golden frame `play_errand_delivered`, blessed for performance 2's route, was blessed again: a resident caught mid-step under the summary is now played out before the ending (D-908), so the frame holds no moving sprite; nothing else in it changed |

### What was found and fixed

In commits `8dd05c9` (the known issues, D-900 to D-908) and `88fa0cc`
(the last stand, the log's lines for fights, D-909, D-910).

| Found | How a player met it | Fixed |
|-------|---------------------|-------|
| The errand could not be lost in practice | Dropping the parcel lost nothing, nothing destroyed it, nightfall was five hours of slow waiting | Content of the scenario: the parcel destroyed, in other hands away from the hall, or left lying (or put away) ten minutes fails it, each with a line that says why; nightfall says so too; the introduction says to keep it close (D-900) |
| A mind given a stance before its first thought got no senses | The last stand's strangers were blind unless the scenario attached senses itself | One function makes a mind's state and gives its senses, whatever makes it first (D-901); the last stand's workaround is gone |
| A resident's fault reached the player's log | `Fault: complete_interaction declined (interaction.not_now) in action core:interact` in the market town | A rule's fault names whose doing it was; the log takes only the party's own and nobody's (D-902) |
| The quick attack out of reach picked Smash and asked for a place; with nobody in sight it targeted the character itself | `f` at a resident across the market | With a target, an action not aimed at someone is not available (`Smash is not aimed at someone.`); `f` out of reach says `That is out of reach.`; an entity picker with nobody to offer is not opened (D-904). The specification gives no closing in, so none is offered |
| A local made by `let` from a query had no type | `$porch.x` faulted | The local has the type of what the query gives (D-905); bundled content and the armoury fixture now take nullable results through `for_each` or `??` |
| Hunger did nothing | The `Fed` bar fell to nothing | Content of `core`: hungry below a quarter, weak below a tenth, starving at nothing (blood drained: the body falls a day later and dies half a day after), for characters someone plays, in the status column (D-906) |
| An unconscious player was asked to react, could answer nothing, and the game stood still for good | The marksman knocked out at the door: `Unconscious, and cannot act until they come to` at every key, the clock stopped | A stopped mind is asked nothing (D-907) |
| The ending's line came before the blows that brought it | `The game has ended: lost.` above `The person strikes Ada.` | Playback catches up before the line (D-908) |
| Nobody's death, nobody's knock-out and no wound was told | A stranger died at the door and the character went on striking the body; blows "stopped" on the forearm cut it, and the character bled to death without a word | Lines for falling senseless, coming to, dying and being wounded, and for what is done with something held: eating, drinking, binding wounds (D-910) |
| An interaction out of reach named itself by id | `Bandage your wounds  east: that is out of reach for \`core:apply_bandage\`` | `that is out of reach` (D-864's rule) |
| The last stand was decided by darkness and numbers | See below | Tuned as content (D-909) |
| The errand's parcel left on a street did not fail it | The session's first runs on the merge: the parcel dropped at two, the clock at a quarter past, the errand still open | The rules compared the parcel's site with the hall's with `!=`; on a street the site is null and a comparison with null is false (05 §4.4). They now take a null site as away from the hall (D-900). Then the failure was told as `Night has fallen, and the town hall has shut.`: the line now reads what has become of the parcel, not the variable the timer had just set (D-900; known issue 11) |

### The last stand's balance

Played to its end from the title at seed 4, before and after tuning.

Before: three strangers with knives came at once. On the lane, hitting
back with bare hands, the character was dead in 36 seconds. Inside, a
fight at the door went on for hours with the strangers hindering one
another in the doorway (`The person strikes the person.`). Nothing in
the house could be found in the dark (`e` offered nothing on the
table), the marksman could see nobody to shoot until they were on her,
and a character hidden in the dark lived until the sky began to
lighten at five, when the strangers found her, and died in the last
hour before a dawn set at six.

After (D-909): the strangers come one by one (at ten, half past eleven
and one); a club and two bandages lie on the table, a lamp lights the
room and can be put out; dawn comes at five.

| Played as | Comes to |
|-----------|----------|
| An ordinary person on the lane, hitting back | Lost in 47 seconds |
| An ordinary person at the door, club and bandages taken up | The first stranger knocked out and beaten to death; the character falls to the wounds of the fight minutes later: lost |
| A brawler at the door | All three strangers killed, the last at one; the wounds were never bound (a hand is needed for it, and both held something) and the character falls after the third: lost |
| A marksman inside, shooting through the open door | The first stranger killed with the quiver's twelve arrows; then nothing to shoot with |
| An ordinary person hidden in the dark house, the lamp out | Never found; alive at five: won |

So the night can be won and lost by an ordinary person, and the
choice that decides it is a player's: stay on the lane and die, fight
at the door and bleed unless the wounds are bound, or hide in the dark.
It feels fair in that the danger comes one at a time, is heard before
it is seen, and the house holds what is needed; it is still brutal, and
two things make it less fair than it should be, below (known issues 4
and 5), and hiding may win it too surely (known issue 8).

### Known issues that remain

Worst first.

1. **Struck by the player, the watch does not answer.** Greeted, the
   watchman answered; struck with bare hands until a blow landed
   (`The watchman is wounded.`), he walked on, no watchman held the
   player hostile in three minutes, and three minutes in a wardrobe
   upstairs passed without anyone looking. The watch's minds hold
   hostile whoever they see hurt someone (`hostile_after`), and a test
   script shows it working for a rowdy's blows and for the same
   character's when it is delegated. In the session, read from truth
   when the blow had landed, the watchman struck perceived her, did not
   hold her hostile, and held the goal `answer_call` (someone had called
   for help); the four other watchmen did not perceive her. What
   differs for a directly controlled striker lies between the blow's
   signal and the stance it should give (`perception::percept::receive`,
   `ai::on_custom`), which this stream keeps out of; it is the AI
   stream's to find. The market fight with the watch answering, fleeing and being
   caught cannot be played until it is found; its session is kept,
   ignored, to show it (`a_fight_in_the_square_and_the_watch_answers`).
2. **Waiting among the town's people is over its budget at the bundled
   cap, and the cap that meets it breaks the watch.** After performance
   3 a minute's wait in the square takes 1.89 s in release at the
   default cap of 150 (D-927's figure). At 50 hot minds it takes 0.77 to
   0.79 s, but then a call for help no longer reaches the watch 40 m off
   (probably: warm listeners hear only at four times their threshold),
   so the cap is held back (D-911). A debug build is ten times slower,
   so a day of routines watched from the square is still no session for
   the suite.
3. **Part of what this round set out to play was not played**, for the
   cost of the town and of time: a bandage *made* (the recipe; one was
   bound on in the last stand), a blade coated with poison and someone
   cut with it, a pistol's shot and reload in a fight (the marksman
   ready-made has the bow), a disguise, and the afternoon and night of
   a day of routines.
4. **Blocking a blade with the forearm stops the blow and cuts the
   arm**, and Block is first in the reaction prompt: a player who
   blocks every blow bleeds to death from blows the log says were
   stopped. The log now says `Ada is wounded.`, but not where or how
   badly (another's body is not the reader's to read; one's own could
   be, and is not yet).
5. **Binding wounds needs a free hand**, and an ordinary person with a
   club and bandages has none: the club must be put down (inventory,
   Drop), the bandage put on, the club picked up again, in the middle
   of a fight.
6. **The quick shot says the pistol's reason when the bow cannot
   shoot**: with no arrow left, `s` says `There is no loaded weapon in
   hand, or nowhere to aim.`, the first refusal of the list, not the
   bow's.
7. **The target cycle includes the dead**, so Tab can select a body;
   the log now says who died.
8. **Hiding may win the last stand too surely.** Inside with the lamp
   out, the ordinary person was never found by the strangers who walk
   the lane; whether one who saw her go in comes for her in the dark,
   and how often, was not played beyond this one seed.
9. **Residents do not eat**: hunger is only for characters someone
   plays (D-906).
10. **`Footsteps, somewhere close.`** is the commonest line in a town's
   log: every step heard and not seen writes it (folded when it
   repeats).
11. **A global variable set just before `fail_objective` read as unset
   in the objective's `on_failed`** (for modders): the errand's timer
   set `errand_lost` to `left` and failed the objective in one effect,
   and `on_failed` chose the nightfall line. The errand no longer
   depends on it (D-900); why it happens was not found.

### Responsiveness, measured again

As the first round measured it (release build, the market town at seed
4 begun in the square, headless, the mean time of a key's pass), at
`88fa0cc`, before performance 2, on a machine more loaded than the
first round's (a load average near five on four processors, with other
builds and a debug session running): the figures are worse for the
load, not for this round's changes, which touch nothing on these paths.

| Key | Takes | First round |
|-----|-------|-------------|
| A step (`j`, `k`) | 68 to 79 ms | 77 ms |
| Wait a second (`.`) | 189 ms | 93 ms |
| Wait a minute (`p`) | 11.0 s | 6.0 s |
| The quick attack (`f` on the target) | 61 ms (now refused out of reach without asking anything, or struck) | about 100 ms |
| Opening a screen (`i`, then Esc) | under 1 ms | under 5 ms |
| An hour in the last stand (Shift+P) | broken off at once on the lane by the stranger in sight; not measured inside | about 4 s |

The debug build the sessions run in is ten to twenty times slower: the
market town's sessions take minutes (above).

After performance 3, with the cap of 50 hot minds given to the market
town (held back after, D-911), on the stream's merge (`clash-tools bench`, release, the town at seed 7
from the square, two runs on a quiet machine): a minute's wait by day
773 and 786 ms (budget 1 s), at night 202 and 178 ms, a step in the
square 13.4 ms (budget 100 ms): each within its budget.

## The third round

Played from `510128b` on the build branch (stream "consequences"): why
the town did not answer the player's blows, found and fixed as content,
the market fight played to its end, and the smaller issues of both
rounds closed. Decisions D-940 to D-947.

### Why the watch ignored the player

Nothing singled out the player. The one a blow lands on never perceived
it: the bundled `blow_seen` and `blow_heard` are signals *about* the one
struck, and a signal is not given to what it is about (15 §3.2), so a
watchman's `hostile_after` never fired for a blow on himself, whoever
struck it. Played with the second round's market fight instrumented,
the townsfolk round the square perceived each blow with the player made
out as its actor and came to fear her, and the watchman struck had no
percept of it at all. The test scripts' watch answered a rowdy because
it saw the rowdies strike *each other*; in a probe the one struck held
his striker hostile only because he saw the jar of the punch into her
own fist, a `hit_landed` about her.

Now (D-940, D-941), all as content:

- **The one struck feels the blow**: the signal kind and the sense
  `touch` (half a metre: only whoever it lands on), given by every
  bundled mind, and the event signal `blow_felt`. It makes out who
  struck it only if it perceives them by another sense: struck from
  behind by someone unseen, it blames nobody. (Turning round to where
  the blow came from was tried and left out: in the last stand's dark
  it let the strangers see whoever struck them and block every blow
  after, and the night could no longer be won at the door.)
- **Whoever perceives a blow** and makes out the striker, against itself
  or against someone it neither holds hostile nor fears, takes against
  the striker by its own mind's weights (`core:take_against`): the
  watch, whose duty outweighs its caution, holds them hostile; the
  townsfolk fear them. Toward the persona it saw, so a disguised striker
  is remembered as the disguise. A relation fades, and a friendship
  stronger than it outweighs it, so a watchman who sees another subdue
  someone does not turn on him (the minds' unconditional `hostile_after`
  had that fault; the town's minds no longer take a stance from a blow).
- **The first time**, it remembers the striker for it
  (`core:unprovoked_blow`): a watchman reports it to the watch, whose
  every member is told at once (15 §9.2); one of the townsfolk shouts
  for the watch (`Watch! Over here!`), a report any watchman who hears
  passes on. Running and calling for help remain their goals.
- **The watch answers by its goals**: whoever recognises a persona it
  remembers so, seen or told, takes against it; a watchman goes for
  them, lays hands on them (`apprehend`, now the watchman's too) and
  strikes them; lost, he looks where he last believed them to be, and
  gives up.

### Sessions and test scripts

| Session | Test | Plays | Found |
|---------|------|-------|-------|
| The market fight | `play_market.rs` (`a_fight_in_the_square_and_the_watch_answers`, no longer ignored) | Begun in the square at seed 4; Tab to someone not of the watch, up to them, `f` until a blow lands; the watch watched for; then to the nearest wardrobe in a house by the square, in, three minutes waited | Tab had moved to the watchman, who was struck: he held Ada hostile at once and went for her (`The watchman strikes Ada.`, `Ada is wounded.`); three of the townsfolk round the square shouted for the watch (`The person shouts: "Watch! Over here!"`, `The shopkeeper shouts …`); all four other watchmen knew of the blow. She got upstairs into the wardrobe; the watchman lost her, looked where he had last seen her and went back to his post. Escaped. Passes on either outcome, caught or hidden (D-942). 67 s in a debug build |
| A blow struck without cause | `scenarios/social/unprovoked_blow.scn` | A townsperson struck by a character a player plays, before a watchman and another townsperson; a watchman fifty metres off | The one struck and the witness fear her (0.70); the watchman holds her hostile (0.90) and takes up `confront`; the shout is heard; the far watchman is told, and holds her hostile the moment he sees her |
| A blow nobody saw | `scenarios/social/unseen_blow.scn` | Struck from behind, standing still, nobody else about | Felt; no fear, no memory naming her, the watch told nothing |
| A disguised blow | `scenarios/social/disguised_blow.scn` | Tess in Vic's face strikes someone before a watchman | Both remember Vic; the far watchman is told of Vic; Tess, her face her own again, walks up to him unknown; Vic is held hostile the moment he is seen (scene S15) |
| A bandage with both hands full | `scenarios/making/bandage_hands_full.scn` | A club in one hand, two bandages in the other, a cut | `Bandage your wounds` offered and the wound closed; one bandage used, one left (both were used before) |

Not played, for want of time: a bandage *made* at a bench and put on, a
blade coated with poison and someone cut with it, the pistol fired,
emptied and reloaded in a fight, and a disguise worn by the player in
the application (the test script above plays it in the simulation;
`core` offers no item that gives a persona). The last stand's strangers
were not taught to search the house (known issue 1 below).

### What was found and fixed

| Found | How a player met it | Fixed |
|-------|---------------------|-------|
| The one struck never perceived the blow | Struck by the player, the watchman walked on (the second round's issue 1) | Touch, `blow_felt`, and the rules of 15 §12 (D-940, D-941) |
| A watchman took a stance against whoever he saw strike anyone, for good | Not met in play; in the test scripts watchmen confronting a brawl could go for each other | Relations by the mind's weights, outweighed by friendship (D-941) |
| Block came first in the reaction prompt with nothing in hand | Blocking every knife blow on the forearm, bleeding from blows the log called stopped | An action's `offer` (D-943): Parry first with a blade or club in hand, Block first with a shield, last on the bare forearm (after Dodge and Step back) |
| With no arrow left the quick shot said the pistol's reason | `s` after the quiver ran out: "There is no loaded weapon in hand" | The same `offer`: the shot that suits what is in hand comes first, and says why it cannot (D-943) |
| Tab selected the dead | A stranger dead at the door, struck again and again | A body seen lying dead is drawn as a thing; the target cycle passes over it, and a target that dies is no longer the target (D-944) |
| `Footsteps, somewhere close.` filled the log | Every step heard and not seen | A show's `heard.quiet` (D-945): footsteps are told once, and again after twenty seconds without any; the `!` marks each |
| A variable set just before `fail_objective` read as unset in `on_failed` | The errand failed as "Night has fallen" when the parcel was left behind | Not the scenario runtime, which reads it set: the errand wrote `"'left'"`, which a text field takes as written, quotes and all, so `== 'left'` was false. The loader warns of quoted text (`W0679`, D-946); the errand writes `left` |
| Binding a wound "needed a free hand" | The brawler at the door bled to death with bandages on the table | It never did (16 §3.3: `held` is anything carried); the brawler held two clubs and had no hand to take the bandages up. One binding used up both bandages of a stack: now one (D-947) |

### The watch at the cap of 50

Smooth play set the bundled games' attention cap to 50 hot minds but for
the market town, where the watch then answered no brawl: the watchmen
39 to 43 m off were warm, the call for help was given beyond hearing's
40 m, and warm characters do not see blows. A call is now passed on once
(D-948): one of the townsfolk who hears it shouts for the watch, an
alarm the watch answers and the townsfolk do not pass on again. With it
the brawl of `town_life.rs` at 50 draws a watchman who arrives and goes
for the brawlers (without the relay, at 50, none comes), and
`town_life` runs at 50 like the other bundled games.

### What changed in the earlier sessions

- **Golden frames blessed again**, each for a change looked at:
  `struck` and `empty_weapon` (someone struck or shot now shouts for the
  watch, and the one shot steps away, afraid), `reaction_prompt` (the
  reactions listed by `offer`: Block, with no shield, last, which brings
  Step back into the list's view), `play_errand_delivered` (footsteps
  told once, not `(x2)`).
- **The last stand's sessions** answered a blow with the first of Step
  back, Dodge, Step aside, Parry and Block that the prompt showed; Step
  back was off the end of its list, so a dodge was what was played.
  Listed by `offer`, Step back came into view, and stepping back from
  the door let the strangers in: the night at the door was lost before
  the first stranger died. The sessions now ask for a dodge first, which
  is what they played; their outcomes are as before.
- **The quarrel in the square** (`a_word_a_blow_the_watch_and_a_wardrobe`)
  no longer waits half a minute beside the watchman struck: he answers
  now. It runs for the wardrobe as the market fight does.
- **Hearing behind a wall** (`perceiving.rs`): a step heard within
  twenty seconds of the one before is marked and not told.

### Known issues that remain

Worst first.

1. **Hiding still wins the last stand surely.** The strangers do not
   search a house they believe someone is in: no method looks in rooms
   or opens wardrobes. Not built in this round.
2. **A watchman does not look inside a wardrobe**, in the town either:
   hiding unseen ends a pursuit. The market fight may also end with the
   player caught, by the seed; it was not played to that end.
3. **What the town knows is told everywhere at once** (15 §9.2's
   simplification): a watchman across town holds the striker hostile
   the first time he sees them. No cases, evidence or accusation (13
   §8.3 to §8.5).
4. **Part of what was to be played was not**: a bandage made, a poisoned
   blade, the pistol in a fight, a disguise in the application.
5. **A call for help reaches twice a voice's reach and no further**: a
   brawl far from any watchman, with nobody between to pass the call
   on, is still answered by nobody.
6. **Residents do not eat** (D-906).

## The fourth round

Played from `d3d1395` on the build branch (stream "final polish"):
the third round's first known issue, the last stand's strangers who
never searched the house. Decisions D-950 and D-951. The four sessions
not yet played (a bandage made, the poisoned blade, the pistol in a
fight, a disguise worn), the final release notes and their
measurements were not started in this round: they are known issue 4
below.

### How the strangers search

As content (D-950): one new function, `places`, tells a mind where the
places a person could hide in are about a cell (static geometry, safe
for belief); the interaction `core:search` opens such a place and brings
out whoever hides in it; the `guard` mind's goal
`core:search_hiding_places` goes to each place within six metres of
where its quarry is believed to be, nearest first, searches it, and
stands two to four seconds looking and listening, then looks round and
gives up, all while the belief is fresh (four minutes). Walking the
rooms it sees whoever stands in the open as far as the light lets it,
and hears whoever moves. The last stand puts a wardrobe in the house
for one to hide in. A stranger searches only a house he believes the
character to be in from what he perceived: seen going in, or heard
inside. Found in the first play of it: a method's `do` step gave the
interaction's id as text, which the action refused (D-951).

### Played

Each night played from the title to its end (or 600 keys) as an
ordinary person, with the room's lamp put out first; "found" is when a
stranger brought her out of the wardrobe or saw her. Seeds 1 to 3 and 6
build the same house (a hut of one room), seed 4 the kept sessions'.

| Played as | Seed 1 | Seed 2 | Seed 3 | Seed 4 |
|-----------|--------|--------|--------|--------|
| Hidden in the dark by the lamp put out | survived | survived | survived | survived (`hidden_in_the_dark_house_until_dawn`) |
| Hidden in the wardrobe | found, still fighting at 600 keys | found, lost | found, missed, lost again in the dark: survived | found (`hidden_in_the_wardrobe_the_stranger_searches_it`) |
| At the door, an ordinary person (kept session) | | | | survived, the first stranger killed |
| At the door, a brawler (kept session) | | | | survived, the first stranger killed |

Before the search, a careful fight at the door (wounds bound as soon as
there was a bandage in hand) won two nights of four as an ordinary
person and one of four as a brawler (seeds 1, 2, 3 and 6), and hiding in
the dark won every one.

What decides it now: the first stranger, on the lit lane at ten, sees
her go in and searches the hut: the wardrobe is the first place he
searches, and whoever hides in it is found, every time. With the
room's lamp out the hut is dark: no light at all with the door shut,
and with the door he opens the porch lamp gives under a hundredth of
daylight in the corner by the lamp and up to a fifth by the door. In
that corner a still hider is not seen even from the wardrobe, two
steps off, and is not found. The two after him never perceive her in
the house, so they never search it, and walk the lane until dawn. The
night was not tuned to the aim of this round: hiding in the dark still
wins as surely as before, and the fight at the door got easier at seed
4 (known issue 1).

Tried and left out: telling each stranger as he came that she was in
the house (a told track at the door, D-950). The first then searched
before she had hidden and found the wardrobe first every time, the
dark hid her from all three just the same, and at the door the
strangers stood in the light outside, unable to see her, while the
scripted fighter, unable to reach them, never saw the night end.

### Known issues that remain

1. **Hiding in the dark still wins the last stand surely, and the fight
   at the door is easier than before.** A searcher sees only by light,
   and the strangers carry none; only the first stranger, who sees her
   go in, ever searches. Content could give them a light to search by
   (an area of light anchored to each, as the bundled `shadow` anchors
   darkness) or a way to learn she is inside; either changes the fight
   at the door too, and was not played.
2. **A stranger with both arms disabled lingers.** In a fight at the
   door at seed 4 the first stranger, both arms useless, held
   `apprehend` (which needs two arms) for hours beside the character,
   and each long wait was broken off by him at once (seen in a probe of
   the fight, not in the kept sessions).
3. **The play sessions' fighter does not walk up to a foe out of
   reach**: it waits, and the wait is broken off by the foe in sight.
   The kept door sessions now play 3000 keys to reach dawn.
4. **Not started in this round**: the four sessions of the brief (a
   bandage made at the workshop's bench and a wound dressed; a blade
   coated with poison and someone cut with it; the pistol fired,
   emptied and reloaded in a fight; a disguise worn and a witness who
   remembers it), the final release notes with the release build's
   measurements, and the check of the README and how-to-play against
   the game. Found while looking at them: the bundled world has no
   source of nightshade or a vial of it, so a poisoned blade cannot be
   made in play, and no item a player can wear presents another
   persona.

## The fifth round

Played from `7283bb1` on the build branch (stream "last stand
balance"): the fourth round's first three known issues, and the night
played every way at sixteen seeds. Decisions D-960 to D-964.

### What the strangers carry and do

As content:

- **A lantern** (`core:lantern`, `mods/core/data/lights.json`, D-960): an
  item that, taken into someone's hand, lights the ground round them:
  an area of no size anchored to the bearer carrying a light of 0.3 (half
  of that a metre off, a fifth two metres off, a tenth three metres off)
  out to six metres, the way `shadow` carries darkness (02 §8.3, 08 §7.2). Put
  down, stowed, put away or taken by someone else, the light leaves
  whoever had it and goes with the lantern's new bearer. Each stranger
  holds one beside his knife. The player sees them coming from far off
  down the dark lane; they see whatever the lantern lights, a step or
  two round them, and no further. The test script
  `scenarios/world/lantern.scn` checks the light taken up, carried, put
  down and taken up by someone else.
- **They tell each other** (D-962): the strangers are a group,
  `core:strangers`. One who loses the character reports where he last
  perceived her to the group (`report_to_group`), and whoever comes
  later is told what the group knows as he comes: a stranger at half
  past eleven searches where the first one lost her, by the search of
  the fourth round (`search_hiding_places`), lantern in hand. Nobody
  knows where she is unless he perceived it or was told. (Reporting
  also where he first saw her was tried: the 96 nights came out the
  same.)
- **The first comes from the east** (D-963): from 26 m down the lane,
  not 14 m west in the porch lamp's light, so that whether he sees her
  go in depends on the house and the lane at that seed, and how quickly
  she is in.
- **The house has a back door** in place of the window farthest from
  the front door.
- **The fallen are stepped over** (D-964): someone lying dead or
  senseless yields the cell to whoever steps into it (`passage_requested`,
  `core:step_over_the_fallen`): the two change places. Before, a body
  in a doorway sealed it both ways (below).

### The two faults, and two found on the way

- **A stranger with both arms useless stood for hours trying to grab
  her** (the fourth round's issue 2). `apprehend` chose among every
  action tagged `grip`, the last of which (breaking free) is refused
  because nothing holds him; that refusal could pass, so the method
  failed, its fallback (look at her a second) ran, and the goal was
  taken up again for ever. Now a step whose action is refused for a
  reason waiting will not change (too few limbs of a channel at all,
  an action that cannot take its target or parameters) fails its goal,
  which is set aside for its `retry_after`, and the mind chooses again
  at once (09 §4.1, D-961), and `apprehend` grabs with `grab`. Tested in
  `crates/clash-sim/tests/ai.rs`
  (`a_guard_with_no_arm_to_grab_with_gives_up_the_grab_and_does_something_else`):
  he gives it up, a few times in thirty seconds, and kicks.
- **The sessions' fighter did not walk up to a foe out of reach**
  (issue 3). `strike_target` in `crates/clash-app/tests/session/`
  strikes with `f`, and with the foe out of reach walks up to them
  first, a step at a time along the way the character believes leads
  there, as a player would; with no way there the session waits a
  minute (D-963). The last stand's `hold_out` uses it.
- **A body in a doorway sealed it both ways.** Found with the fighter
  that walks up: a dead stranger is an occupant that still blocks
  movement, so the next stranger could not get in (`ai.not_arrived`)
  and she could not get out to him, and they stood three cells apart
  from one in the morning until dawn. Played before the fix, this
  "won" 10 of the dark's 12 winning nights and 21 of the door's 30
  (seeds 1 to 16). Fixed as content (D-964, above).
- **`use_near` picked up what lay on the floor instead of what it was
  asked to do.** The list of what is near is two lists, what lies loose
  and what can be done, and Down moves within one: with a dead
  stranger's knife and lantern lying by her, "Bandage your wounds" was
  never reached and Enter picked up the knife, so the sessions' fighter
  never bound a wound once a stranger had fallen near her. It now
  Tabs to the second list. Found with it: the fighter binds with the
  club in hand (D-947) and takes up a club put down; screens left open
  when a blow came are closed.

### Played

Each night played from the title to dawn or death by
`the_night_played_many_ways` (`crates/clash-app/tests/play_last_stand.rs`,
ignored; `NIGHT_WAYS`, `NIGHT_SEEDS`, in release about two minutes for
the ninety-six nights). Every way first gets inside and takes up what
lies on the table (the club, for whoever has nothing in hand, and the
bandages). In the dark: the lamp put out and the long wait by it. In
the wardrobe: the lamp put out, in, and the wait. At the door: the
lamp left burning, `hold_out` (fight whoever is in sight, walking up to
them, bind wounds). The marksman: shoot whoever is in sight, then
fight. Out the back: through the back door and thirty metres on into
the dark, and the wait. A hider who is found (a blow coming, a wound,
brought out of the wardrobe) fights as `hold_out` does. W is survived,
L lost, – not played to an end (the script met a screen it does not
answer or no way to walk: seed 5 out the back, seed 14 in the dark and
out the back).

| Played as | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | Won |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| In the dark corner, lamp out | W | L | L | L | W | W | L | W | L | L | L | L | L | – | W | W | 6 of 15 |
| In the wardrobe | L | W | L | L | W | L | L | W | L | L | L | L | L | L | W | W | 5 of 16 |
| At the door, an ordinary person | L | W | L | W | W | L | L | L | L | L | L | L | W | L | L | W | 5 of 16 |
| At the door, a brawler | W | L | L | L | W | L | L | L | L | L | W | W | W | L | L | W | 6 of 16 |
| At the door, a marksman | W | L | L | W | W | L | W | W | W | W | W | W | L | L | W | W | 11 of 16 |
| Out the back door | L | L | L | L | – | L | L | W | L | L | L | L | L | – | W | W | 3 of 14 |

Played again at seeds 17 to 32 as a check, after the last change: the
dark won 6 of 16, the wardrobe 7 of 16, the door 6 (an ordinary
person), 11 (a brawler) and 12 (a marksman) of 16, out the back 6 of 13
(three not played to an end). Over the 32 seeds: the dark 12 of 31, the
wardrobe 12 of 32, the door 11, 17 and 23 of 32, out the back 9 of 27.

Each way wins and each way loses, decided by the world: the house and
lane the seed builds, whether the first stranger saw her go in, where a
lantern's light falls, the fight. The dark corner wins two nights in
five, the door a third (an ordinary person or a brawler) to two thirds
(a marksman, who shoots them as they come into the door's light). At
seeds 5 and 16 nobody ever perceives her and the strangers walk the
lane till dawn, every way; at most others the first stranger sees her
go in, searches the house and tells the others where he lost her, and
the night is a fight with each in turn. Losses come mostly within a
few minutes of a stranger's coming: the first at ten, the second at
half past eleven and the third at one, each told where she was.

Tuned to this, each step a run of the nights: with lanterns of 0.4 and
the first stranger 14 m west as before, he saw her go in every time and
every way was a fight with him in the first three minutes (seeds 1 to
8: the dark won 0 of 7, the wardrobe 0 of 7, the ordinary person at the
door 0 of 8); with lanterns of 0.2 and the first 36 m east, he never saw
her and nobody searched (the wardrobe won 8 of 8, the dark 6 of 7, the
flight 1 of 7); 26 m east with 0.3 is the table (0.4 there lost the
flight 13 of 15 and changed the rest little). Then the fallen stepped
over and the bandaging fixed. Before this round, with no lanterns and
the first stranger 14 m west, the dark won 7 of 8 seeds and the
wardrobe was found every time.

The kept sessions play the night at the door as an ordinary person at
seed 4 (she kills all three), as a brawler, in the dark (found, and she
fights them off) and in the wardrobe (found) at seed 1, and the
marksman's at seed 8; at seed 4, before this round's tuning, every way
lost.

### Known issues that remain

1. **The flight out the back wins one night in five**: a stranger who
   sees her go follows, and the others are told where she was last
   seen and go there with their lanterns. A player who keeps moving, or
   goes further, was not played: the script walks thirty metres and
   waits.
2. **The scripted hider never moves once hidden**, so a searcher finds
   her only by light or by the wardrobe; a player who creeps away from
   a lantern, or out of the wardrobe once he has gone, was not played.
3. **Binding with both hands full** worked in the session once the list
   was reached (D-947); whether a stack of two can always be split into
   a full hand was not looked into further.
4. **The scripts that drive the night still stop** at a screen they do
   not answer or a place with no way to it, three nights of ninety-six.
5. **Of the brief's reasons to look inside a house**, the strangers act
   on what they saw (the fourth round's search), what they heard (the
   `guard` mind's `investigate_noise`: to where the noise was, a look
   round with the lantern) and what another told them (D-962). A light
   seen in the house earlier, or a door seen shut, is not built: what
   state a lamp or a door is in is not static geometry, and what a mind
   has noted of such things (09 §3.4, 15 §7.1) cannot yet be read in a
   mind context.

## The sixth round

Played from `7283bb1` on the build branch (stream "content and
sessions"): the four sessions the third and fourth rounds left
unplayed, after the two things that kept two of them from being played
at all were put in the world as content: nightshade to make a poison
from (D-970), and a hooded cloak a player can wear as a disguise
(D-971). Decisions D-970 to D-973. Every session is played from the
title in the market town at seed 4, begun in the square, by keys alone;
the walks across town go by the square, the believed way being searched
no further than 20,000 cells.

### Sessions

| Session | Test | Plays | Found |
|---------|------|-------|-------|
| A bandage made, a wound dressed | `play_making.rs` (`a_bandage_made_at_the_workshop_bench_dresses_a_wound`) | To the workshop by the streets (153 keys); `e`, Take out at a rack of stuff: four strips of cloth and five sticks in hand; at the bench, `M`, Bandage (by hand, five seconds); the sticks put down, `g`, the two bandages picked up; back to the square, someone struck, the watch comes and strikes back (`Ada is wounded.`); `e`, Bandage your wounds, the blows let come: broken off once (known issue 11), then `Ada binds their wounds.`, the wound closed, one bandage left | Made with both hands full, the bandages were put down at her feet and the log said only `Ada finishes the work.`; the inventory read `4 strip of cloths x4  in hand` (both fixed, D-972). About two minutes in a debug build |
| A poisoned blade | `play_making.rs` (`nightshade_gathered_pressed_and_a_coated_blade_drops_someone`) | A knife from the workshop's chest (Open, Take out, the club put down); to the healer's house by way of the square; the herbal from her shelf and Read (`Ada reads the herbal.`); two nightshade from the bed grown with it; her cupboard opened; `M`, Nightshade extract by hand (twenty seconds): `Ada finishes the work: what it gave is in the vial.`; the vial taken out of the cupboard, what came out before it (a pot of salve, bandages, herbs, a vial of sleeping draught) put down one by one; Coat a blade: `Ada coats a blade.`, ten millilitres on the knife; out of the door, the first person in sight cut once: `The person is wounded.`, `The person falls senseless.` | The extract went first into the healer's vial of sleeping draught, ten millilitres of it into the draught's last ten of room, and the line said nothing of where it went (fixed, D-972); four millilitres on a blade could never have dropped anyone (D-970). With ten, one cut through the skin put 60 of `toxin` in, past the 50 that knocks out, and some was left on the blade. About three minutes |
| The pistol in a fight | `play_pistol.rs` | The market town, F2, the marksman, the weapon changed to the pistol and spare rounds; begun in the square, `Holding a pistol`, twelve rounds in it, twelve in the backpack; Tab: the watchman at his post; `s`: refused, `There is no loaded weapon in hand, or nowhere to aim.`, a step back; `s` twelve times (`Ada shoots.`, `Something strikes the watchman.`), the townsfolk shouting for the watch, the watchman striking back, then falling senseless; empty: `s` refused with the same sentence; `r`: `Ada reloads.`, twelve rounds from the backpack; and fired again | The game begins her in the cell of the watchman whose post is the middle of the square, and Tab chooses him first: every `s` started a shot that faulted at its last phase, `Fault: shoot declined (ranged.no_aim) ...`, and the pistol never fired (fixed: a target in one's own cell is nowhere to aim, refused in words, D-973). Shots and the reload said nothing in the log (fixed, D-973). Seven seconds |
| A disguise | `play_disguise.rs` | The knife and two strips of cloth at the workshop; at the bench, `M`, Hooded cloak (a minute): `what it gave is in the workbench.`; Take out at the bench; `i`, the cloak, Wear: `hooded cloak  worn`; back to the square, a townsperson struck until a blow lands; the watch watched for; up to the nearest wardrobe in a house by the square, Hide inside; `i`, Take off and Drop, inside; three minutes; Get out; Tab to a watchman, up to him, ten seconds beside him | The one struck took her for the hooded stranger, a persona made for her when she put the hood on, and remembered the blow as the stranger's; the watch was told and held the hooded stranger hostile, every watchman that held her hostile taking her for the hood. Out of the wardrobe without it, the watchman she walked up to perceived her as herself, held nothing against her, remembered nothing of her, and walked on. Nothing to fix. About three minutes |

The disguise is tested in the simulation by the test script
`social/hooded_blow` too: struck in the hood before a watchman, the far
watchman is told of the hood, lets her by without it, and knows the
same hood again when she puts it back on out of his sight.

### What was found and fixed

| Found | How a player met it | Fixed |
|-------|---------------------|-------|
| No source of nightshade | Nothing to press the extract from; no poisoned blade could be made | One of the healer's herb beds grows it; her cupboard keeps a vial of the extract half the time (D-970) |
| A coated blade could never drop anyone | Four millilitres a coat put at most 3.2 ml in over any number of cuts: 38 of the 50 `toxin` that knocks out | `Coat a blade` puts on ten: the first cut through the skin drops someone (D-970) |
| No item presented another persona | No disguise could be worn | The hooded cloak, made at a workbench or found in a wardrobe; one engine addition, `entity.personas` (D-971) |
| What was made by hand with full hands was put down unsaid | `Ada finishes the work.`, and the bandages at her feet | `recipe_completed`'s `left`: `their hands are full, and what it gave lies at their feet.` (D-972) |
| A made liquid went into a vessel that held something else | The extract poured into the healer's sleeping draught | A recipe's substance goes only into what holds nothing else; `filled` names it: `what it gave is in the vial.` (D-972) |
| A stack was named twice and its plural put on the wrong word | `4 strip of cloths x4  in hand` | `4 strips of cloth  in hand` (D-972) |
| A shot at someone in one's own cell faulted | `Fault: shoot declined (ranged.no_aim) in action core:shoot at phases.fire.do[0]`, twelve times, and no shot | Every shot and throw requires its aim not to be one's own cell, and says `... or nowhere to aim.` (D-973) |
| Shooting and reloading said nothing | A pistol emptied in a fight: only `Something strikes the watchman.` for the shots that hit | `Ada shoots.` (heard: `A shot, somewhere close.`), `Ada reloads.` (D-973) |

### What changed in the earlier sessions

- **The golden frame `empty_weapon`** (`shooting.rs`) was blessed again,
  for a change looked at: its log now begins `Ada shoots.` where it had
  `The person shouts: "Watch! Over here!"`, the shot told (D-973);
  nothing else in it changed.
- **Every test script's hash** was blessed again, only for the
  definitions added and changed (nightshade's bed and vial, the cloak
  and its rules, the shows, the shots' `requires`); the new script
  `social/hooded_blow` plays the disguise in the simulation.
- The earlier sessions pass unchanged on this round's changes.

### Known issues that remain

1. **The log tells a blow's consequences before the blow.** `The person
   is wounded.`, `The person falls senseless.`, `Ada strikes the
   person.`: a line whose show plays an animation is written when the
   animation ends. Seen in every fight of this round.
2. **A projectile is "something".** `Something strikes the watchman.`
   for a pistol's round, as for an arrow.
3. **Text has no pronouns**: `Ada binds their wounds.` (15 §8.3, pronoun
   sets not built).
4. **The market town begins the player in a watchman's cell**, the
   middle of the square where he stands at his post: where a game
   scenario places its party is not checked against who stands there.
   Tab chooses him first; a shot at him is now refused in words.
5. **Take out takes what a container holds in the order it was put
   there**, as much as the hands hold, and a player cannot choose: to get
   the vial out of the healer's cupboard the salve, the bandages, the
   herbs and the sleeping draught came out first and were put down one
   by one. The inventory's Take out reaches only what one carries.
6. **With things lying loose about the character, `e`'s second list is
   short**: four things at her feet put Take out below the popup's edge,
   reached by Tab and scrolling.
7. **A hood destroyed while worn** leaves its wearer presenting the
   hooded stranger: nothing raises `item_removed` (06 §9.2, D-971).
8. **The poison is strong**: one cut through the skin drops someone,
   and by the substance's numbers three more would kill (D-970); a cut
   that does not reach the skin (a coat sleeve) puts nothing in. Not
   played beyond the first cut.
9. **A stranger in a hood is described as anyone is** (`the person`):
   there are no features to describe a hood by (15 §8.4).
10. **The sessions' helper `use_near` pressed Enter in the wrong list**
   when something lay loose within a step: with a pot of salve at her
   feet, Take out at the healer's cupboard picked the pot up again,
   eight times (a fault of the helper, not of the game). The new
   sessions find a row by the list's own order (`town::use_offer`),
   which also reaches a row scrolled out of view; `use_near` itself was
   mended by the fifth round, merged before this one ended.
11. **Binding a wound in a fight was broken off once, unsaid.** With the
   watchman striking at her and every blow let come, the first `Bandage
   your wounds` ended without binding anything and with nothing in the
   log but `The watchman misses Ada.`; the second, a moment later, bound
   the wound. Whether a foe coming into sight again (`core:stop_for_a_foe`
   interrupts what a played character is doing) or the blow broke it
   off was not looked into.
