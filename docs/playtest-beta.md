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
