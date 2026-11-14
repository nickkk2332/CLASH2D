# Release notes: the beta

The first beta of CLASH2D: a playable, moddable game in a window or a
terminal, built on an engine whose every world, creature, power, rule
and screen is JSON in a mod. It is not finished and not balanced. This
page says what is in it, how to run it, and what a player will run into.

[How to play](how-to-play.md) is the player's guide; the
[modding guide](modding-guide.md) is the creator's. What was built, in
order, is the Log of [the beta plan](plans/beta.md).

## How to run it

With Rust installed through [rustup](https://rustup.rs), from a clone of
the repository:

```bash
cargo run -p clash-app --release
cargo run -p clash-app --release -- --terminal
```

The first opens a window when a display is available and the terminal
otherwise; the second keeps it in the terminal. On Linux the window needs
the Wayland or X11 libraries at run time (the [README](../README.md#quick-start)
lists them). `-- --help` lists the options. Saves and preferences are
kept in the user's data directory (`~/.local/share/clash2d` on Linux).

## Highlights

**The world.** Cells of 1 m on storeys of 3 m, made of materials that
take damage by one rule from a fist to a blast. Walls break, floors give
way, and a building whose supports fail comes down floor by floor. Fire,
smoke, steam and water spread as fields; blasts push, and bodies are
thrown through walls. Light comes from the sky by the time of day,
through windows and doors, and from lamps.

**The town.** A market town generated from a seed: streets, blocks and
lots, houses of several storeys with kitchens, pantries, wardrobes and
chests, a bakery, a tavern, a guardhouse with an armoury behind a locked
door, a workshop, a market of stalls and a healer's house. Containers
are filled when first opened.

**People.** Townsfolk, guards and dogs with minds written as data: they
go about near home, look into noises they hear, flee danger, and keep
away from anyone they saw hurt someone; guards go after anyone they hold
hostile or saw hurt someone, and search where they last saw them. They
see by light and line of sight and hear through walls; they remember
what they saw, and are fooled by what they did not. They belong to
groups (the townsfolk, the watch, the gang), hold relations that fade,
obey orders from those they answer to, and pass on what they saw when
they speak, at less certainty.

**Bodies.** Bodies of parts and tissue layers with no hit points. Hits
pass through clothing and armour into skin, muscle and bone; wounds
bleed, hurt and heal; parts can be severed. Too little blood and you
fall, then die; too much pain and you fall.

**Fighting.** One shared timeline: every blow has a wind-up, an impact
and a recovery. A blow seen coming in time stops the game and asks how
to answer it: parry, block, dodge or step back. Punches, kicks, cuts,
thrusts, shoves; grips, drags, throws of a person, a held person as a
shield, and takedowns from behind; pistols, rifles, bows and throwing,
with magazines, quivers and reloading; stances. From the keyboard each
is quick (`f` strikes the target with the first blow that can be made)
or chosen in detail (`F` lists every blow, greyed with why it cannot be
made, and asks for the part aimed at and how hard).

**Who you are.** Make your character, or choose one ready-made, as
each game allows: an ordinary person, a brawler with a club, a marksman
with a bow or a pistol, an adept with two or three powers of your
choosing, or someone with every power.

**Powers.** Twelve powers built from the same parts as everything else:
dash, force push, fire bolt, shield, heal, time stop, null field,
gravity well, float, summon hound, stone wall, shadow. Costs, cooldowns,
modes that drain a reserve, areas where time or gravity changes, and
summons and conjured walls that last as long as their power.

**Using the world.** Doors open when walked into; locks need their key;
lockers can be hidden in; containers, racks and stalls hold things;
what lies about is picked up (`g`), and what you carry is worn, stowed,
put in, taken out, given or dropped from the inventory.
Recipes at workbenches, ovens, hearths and stills, by hand or left to
run; know-how learnt from books; bread, ale, salves, bandages and
poisons, whose substances do what they do in the body.

**Games.** Three game scenarios (the market town to live in, an errand
against the clock, a last stand until dawn), each with its own saving
policy and its own answer to death; objectives with markers in a
journal; an introduction and a summary. Saving and loading from the
pause menu, and Continue on the title.

**Screens.** The title, new game and setup, the play screen with its
log and status, look mode, inventory, condition, use, make, know-how,
abilities with quick slots, talk, the map with the journal's markers,
the journal, saving and loading, options with key rebinding, a menu on
the second mouse button, and help listing every control; two themes. The same frames in a terminal or in a window
drawn by the processor, with sprites gliding between cells, shake and
flash.

**Modding.** Everything above is JSON in `mods/`. A linter with located
errors and explanations, a reference generated from the engine,
JSON Schemas for editors, `init-mod` and `new` to start from, `refs` and
`find` to look round, test scripts with golden hashes run four ways, a
world viewer with a console, and save tools.

## Known issues

What a player will meet: the stubs the plan's Log records that show in
play, and what trying the game headless on the build branch showed.

**Missing in play**

- **No crouching or lying down, and no stances,** though posture
  changes how loud and how visible a character is.
- **A bow's draw runs to its end in one go,** so there is no moment to
  hold it or let it go with `z`.
- **The log names your own character by name** ("Ada strikes the
  person."), not as "you"; everyone else is named as your character
  knows them.

**Screens**

- **Making a character is offered, not asked.** F2 on the new-game
  screen makes or chooses who you are; a game begun with Enter alone
  gives you an ordinary person. A template's starting `items` are not
  made: the bundled characters' weapons are given by a rule.

**The world**

- **No lockers in the bundled maps,** and lamps only in Last stand.
  Lockers exist as definitions; wardrobes are the places to hide.
- **Last stand is dark,** but for a lamp by the door and one in the
  room: what is not lit is heard (`!`), not seen. The strangers come
  one by one; whoever stays on the lane dies quickly, and whoever hides
  in the dark house with the lamp out lives to see dawn.
- **Residents do not eat.** Going hungry, weak and starving is for the
  character you play.

**Fighting**

- **Blocking a blade with your bare forearm stops the blow and cuts
  the arm.** It is offered last now, after Dodge and Step back (Block
  comes first with a shield in hand, Parry with a blade or a club). The
  log says `… is wounded.`, not where; `c` shows your wounds.
- **The town's answer to a blow is fear and the watch, nothing more:**
  there are no cases, accusations or wanted posters yet. What the watch
  was told it knows at once, everywhere (15 §9.2), and it forgets you
  after an hour or so; townsfolk who heard of you keep away from you.
- **A watchman does not look inside a wardrobe**: hiding where no one
  saw you go works. In Last stand the strangers do not search the
  house either.

**Saving and running**

- **A single save is written only when the game is left.** Last stand
  keeps one save; it is not written at intervals, so a crash loses the
  night.
- **The market town is still not quick to wait in by day.** A minute
  waited in the square takes under a second of real time (release
  build), and an hour about six seconds by day and one at night: the
  country beyond your sight and hearing is simulated coarsely while you
  wait, and any key stops a wait. The target is five seconds an hour.
- **The window was not seen on a real screen** before the beta: it is
  tested headless, image by image. If it misbehaves, `--terminal` plays
  the same game. Changing the theme in play does not change the
  window's font, and there is no paste.
- **No sound.**

<!-- PLAY-TESTING: the play-testing stream adds its findings here. -->

## Play-testing

Three rounds of play from the title, recorded in
[Play-testing the beta](playtest-beta.md): every game played to an
ending, as a player plays it, with what each round found and fixed.
Among the fixes: the last stand can be won and lost, by hiding in the
dark house or by fighting at its door; the errand can be lost (the
parcel given away, left behind or destroyed, or night falling);
hunger matters; an unconscious character no longer freezes the game;
the log says who is wounded, falls senseless or dies; the quick attack
says when its target is out of reach. A third round made the town answer
a blow: the one struck and whoever saw it fear you or hold you hostile,
the townsfolk shout for the watch, and the watch comes.
