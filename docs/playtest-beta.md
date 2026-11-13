# Play-testing the beta

Every system of the beta was built by its own stream and passed its own
tests. This is what happened when the game was first played from the
title to an ending, as a player plays it: what each session found, what
was fixed and in which commit, what is still wrong and how a player
meets it, and how long the game takes to answer a key.

The sessions are headless tests of the application,
`crates/clash-app/tests/play_*.rs`, driven by keys from the title screen
through the real application on the bundled profile (the powers on the
`armoury` fixture, whose game gives someone every power of the sampler,
because no bundled game does yet). Nothing in them moves the world by a
debug command: they read the game's state only to check it, and to find
the way across town the character believes leads somewhere, which is
the search its own `go_to` makes, turned into the direction keys a
player would press. What they share is `crates/clash-app/tests/session/`.

First played at commit `ced8d26` on the build branch as of `e238730`,
and played again after merging the essentials stream (`cb07bdc`:
playable characters, picking up and the inventory's actions, names for
players), which the sessions now pass on. The town life stream had not
landed: the town's residents are the settlement's people with the minds
of `mods/core`, and there is no watch and no staff.

## The sessions

| Session | Test | Plays | Takes (debug) |
|---------|------|-------|---------------|
| The errand, delivered | `play_errand.rs` | Title, new game, the introduction, the clock, the journal's way (`south, 129 m`), 60 steps through the streets, saved by name from the pause menu, quit to the title, Continue, the rest of the way, the ending `delivered` and the summary | ~3.5 min |
| The last stand, held | `play_last_stand.rs` | The lit lane, the journal (`north-east, 3 m`, the door), through the door by walking into it, the door shut from inside, quit and Continue (the game's one save), time going by behind the door | seconds |
| The last stand, until dawn | `play_last_stand.rs`, `#[ignore]` | The same, then the night an hour at a time until dawn ends it, `Survived.`, and the one save gone | ~40 s in release; run by hand |
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
below), making a character from the title (essentials), the errand
failed (below), a bandage made and a wound dressed, and a blade coated
with poison (below).

## What was found and fixed

All in commit `ced8d26`, with decisions D-870 to D-878.

| Found | How a player met it | Fixed |
|-------|---------------------|-------|
| The log filled with `Fault: \`follow_path\` declined` and `Fault: \`plan_path\` declined`, dozens a minute in the town | Walking anywhere in the market town | The movement verbs give null for what moving comes to, a refused step or no way, and say why with `step_refused` and `path_failed`; a refused step of the player's own is a line in the log (D-870) |
| Time could only be waited a second at a time | The last stand is eight hours long: 28,800 presses of `.`; the errand's deadline five | `w` waits a minute, shift+w an hour; a rule of `core` breaks the wait off when a foe comes into sight or the character is wounded, and `interrupt_plan` now ends a wait (D-871) |
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
5. **Greeting a resident gets no answer in the log.** The talk screen
   opens and offers `Greet` and `Tell them to stay back`; saying one
   leaves no line when the one spoken to is a little way off.
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
    minutes each, for the cost of the first issue.

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
| Wait a minute (`w`) | 6.0 s (7.9 s before) |
| An attack (`f` on the target) | about 100 ms (Cora at the market, out of reach: the quick attack chose Smash and asked for a place, known issue 6) |
| Opening a screen (`i`, then Esc) | under 5 ms, too little to tell from the noise |
| An hour inside the last stand's house (shift+w) | about 4 s |

A step is at the edge of instant; the minute's wait is not, and is the
first known issue.
