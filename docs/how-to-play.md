# How to play

CLASH2D's bundled game puts you in one body in a town drawn in letters
and symbols. Nobody has hit points: people are made of parts that bleed,
hurt and break. Everyone, you included, sees only what their eyes and
ears give them, and acts on what they remember. Time moves only when you
act, but everything happens on one clock, so a blow you see coming can
be answered before it lands.

This guide describes the beta as it is built. What is not in it yet is
in the [release notes](release-notes-beta.md#known-issues).

## Starting a game

```bash
cargo run -p clash-app --release
```

starts the game: in a window if a display is available, in the terminal
otherwise (`-- --terminal` or `-- --window` to choose; the
[README](../README.md#quick-start) has what each needs).

**The title.** New game; Continue (the newest save) and Load, once
there is a save; Options; Quit. Up and Down choose, Enter takes, Esc
quits.

**New game.** Type your character's name, choose a seed (the same seed
makes the same town; `--seed n` on the command line sets the one
offered), and choose what to play from the list:

| Game | What it is | When you die |
|------|------------|--------------|
| **Last stand** | Ten at night, a house on a country lane, and three strangers with knives who mean you harm: one on the lane already, the next at half past eleven, the last at one. Get inside and hold out until dawn, at five. A club, two bandages and a lamp are in the house. | The game ends, and its one save (written when you leave, resumed with Continue) goes with it. |
| **The errand** | Two in the afternoon. Take the parcel in your hands to the town hall in the middle of the market town before nightfall; you have five hours. Keep it with you: handed to someone else, or left lying for ten minutes, it is lost, and so is the errand. | The game ends. Save and load as you like. |
| **The town** | A grid of streets with every door open. Nothing to do but walk. | The game ends. |
| **The market town** | The sandbox: a generated town and its people, no goals, no end. Begin on the north street or in the town square. | You carry on as a newcomer, made fresh where you first began; your body stays where it fell. |
| **The lane** | Houses between fields, their doors shut. Nothing to do but walk. | The game ends. |

Tab moves between the name, the seed and the list; Enter on a game
begins it. The market town first asks where to begin.

**Who you are.** F2 on the new-game screen, with a game selected, shows
who that game lets you be:

| Who | What they are |
|-----|---------------|
| **Adept** | Born with two or three powers of your choosing (the lesser cost a point each, the greater two, from three points), and the reserves to pay for them |
| **Brawler** | Strong, and quick in a fight with fists or a club. Starts with a club in hand |
| **Marksman** | A steady eye and hand. Starts with a bow and a quiver of a dozen arrows, or a loaded pistol and spare rounds in a backpack |
| **Ordinary person** | No powers and no weapon: a build and a little learning of your choosing |
| **Gifted person** | Every power there is |

In the market town and the town you make the character: Enter on one,
then each of its choices (Enter on a choice changes it: the build, the
training, the powers, the weapon), and Begin. In the errand and the last
stand you choose one ready-made: you may name them, and nothing more.
On the lane you are an ordinary person. Without F2, every game gives you
an ordinary person with nothing in your hands but what the game gives
you.

A game with a story opens with a few lines of introduction; Begin, or
Esc, closes them.

## The screen

```text
""Y"""Y"""""""""""""""""""Y"""...""Y""""""*"Y"Y*Y"""┌──────────────────────┐
""""""""""Y"Y"""""Y"""Y"""""""...Y""""""Y"""""""""""│Ada                   │
*""""""""""""""Y"""""""""Y""""..."""""Y"*"""""""""Y"│Blood  ███████████████│
,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,│Pain   ░░░░░░░░░░░░░░░│
....................................................│Holding a parcel      │
........................@...........................│                      │
...................           ......................│                      │
,,,,,,,,,,,,,                       ,,,,,,,.....,,,,│                      │
# # ###+#                                   ,.....,#│                      │
┌ Log ─────────────────────────────────────────────┐│                      │
│You have the parcel. The town hall is in the      ││x look  i things  c   │
│middle of the town.                               ││? keys  Esc menu      │
└──────────────────────────────────────────────────┘└──────────────────────┘
```

The start of the errand, on the north street of the market town: fields
to the north, the street, and the first houses to the south.

**The map** is what your character knows of the place, centred on them
(`@`). What they see now is drawn as it is; what they saw before and
cannot see now is drawn as they remember it, dimmer; what they never
saw is blank. Something heard but not seen shows as a mark where the
sound came from. Common symbols:

| Symbol | Is | Symbol | Is |
|--------|----|--------|----|
| `@` | a person (you are one) | `d` | a dog |
| `#` | wall | `=` `/` | window, shut and open |
| `+` `'` | door, shut and open | `>` `<` | stairs and ramps |
| `.` `,` | floor, ground, paving | `"` `Y` `*` | grass, tree, bush |
| `T` `h` `b` | table, chair, bed | `n` `c` | desk, counter |
| `Θ` | wardrobe (hide in it) | `≡` `∩` `[` | chest, cupboard, shelf or rack |
| `π` `Ω` `&` `δ` | workbench, oven, hearth, still | `τ` | market stall |
| `!` | where something was heard | `^` `~` | fire; smoke, steam or water |
| `%` `;` | rubble | | |

**The log**, under the map, says what happened that your character
perceived: what they saw, and what they only heard ("Footsteps,
somewhere close."). It also says why something you tried could not be
done.

**The status column** has your name, the target you chose, your
**Blood** and **Pain**, the reserves powers are paid from (**Stam** and
**Rsrv**, for those who have them), **Toxin** and **Drowsy** while
something is in your blood, **Fed** and how hungry you are once you go
short of food, what is in your hands, and the time of day.

## Keys

`?` or F1 in play lists every key of the keymap in force, written as
on the keyboard (`Shift+J`, `Tab`). Options (`o`,
or from the title or pause menu) gives a key another meaning: choose the
context (Play, Looking and aiming, Menus), type the key as the keymap
writes it (`w`, `shift+a`, `f5`) and the name of what it should do
(`north`, `wait`), and Bind. Bindings are kept between runs.

The tables below are the bundled keymap (`mods/ui/data/theme.json`) and
controls, read from the data.

### In play

| Keys | Does | What it does |
|------|------|--------------|
| `h`, `Left` | Walk west |  |
| `j`, `Down` | Walk south |  |
| `k`, `Up` | Walk north |  |
| `l`, `Right` | Walk east |  |
| `y` | Walk north-west |  |
| `u` | Walk north-east |  |
| `b` | Walk south-west |  |
| `n` | Walk south-east |  |
| `<` | Go up |  |
| `>` | Go down |  |
| `.` | Wait | Let a second pass. |
| `p` | Wait a minute | Let a minute pass, unless something breaks it off. |
| `Shift+P` | Wait an hour | Let an hour pass, unless something breaks it off. |
| `x` | Look around | Opens Look. |
| `e` | Use something near | List what can be done with the things within a step, and do one. |
| `g`, `,` | Pick up | Pick up what lies at your feet or within reach; with more than one thing there, choose which. |
| `i` | Inventory | Opens Inventory. |
| `c` | Condition | Opens Condition. |
| `a` | Abilities | Opens Abilities. |
| `m` | Map | Opens Map. |
| `Shift+J` | Journal | Opens Journal. |
| `Shift+M` | Make something | List what can be made here, by hand and at the stations within a step, and start one. |
| `Shift+K` | Know-how | List what the character knows how to do and what it could learn next. |
| `o` | Options | Opens Options. |
| `?`, `F1` | Keys | Show every control and its keys in play. |
| `Esc` | Pause | Open the pause menu. |
| `F12` | See everything | Switch between what the character sees and what is really there, in a development profile. |
| `Tab` | Next target | Select the nearest creature in sight, or the next one after the target. |
| `Shift+Tab` | Previous target | Select the previous creature in sight. |
| `f` | Attack | Strike the target with the first blow that can be made now; with no target, choose one. |
| `Shift+F` | Attack, choosing how | Choose the blow, the target, the part aimed at and how hard. |
| `s` | Shoot | Shoot or throw at the target with the first shot that can be made now; with no target, choose one. |
| `Shift+S` | Aim | Choose the shot, the target, the place aimed at and the height. |
| `r` | Reload | Load the weapon in hand from what is carried. |
| `t` | Throw | Throw what is in hand at the target; with no target, choose one. |
| `z` | Stop | Stop what the character is doing: a held draw or charge is let go. |
| `v` | Grab or take down | Choose a hold, a throw or a takedown to try on the target. |
| `Shift+T` | Talk | Talk to the target: list what the character could say to them, and say one. |
| `1` to `9` | Quick slots 1 to 9 | Use the ability on the quick slot of that number. |

These have no key: the application runs the first four when you walk
into someone or into something closed, and the mouse the last two.

| Control | Does | What it does |
|---------|------|--------------|
| `attack_bump` | Attack who is in the way | Strike someone walked into with the first blow that can be made. |
| `attack_bump_ask` | Ask before attacking who is in the way | Ask, then strike someone walked into. |
| `bump` | Use what is in the way | Do what the thing walked into offers by default: open a shut door. |
| `bump_ask` | Ask before using what is in the way | Ask, then do what the thing walked into offers by default. |
| `click` | Go there |  |
| `click_second` | What can be done here | Opens Here. |

`Shift+J` is a capital J, and so on. F12 shows the world as it
really is, not as your character sees it; it works only in the
development profile (`-- --profile dev`).

In a window the mouse works too: the first button on the map walks
there, and the second opens a menu of what can be done with what is
there.

### Looking and choosing a place

Look mode (`x`) and the pickers that ask for a place, a target or a
direction use these keys:

| Keys | Name |
|------|------|
| `h`, `Left` | `west` |
| `j`, `Down` | `south` |
| `k`, `Up` | `north` |
| `l`, `Right` | `east` |
| `y` | `north_west` |
| `u` | `north_east` |
| `b` | `south_west` |
| `n` | `south_east` |
| `<` | `up` |
| `>` | `down` |
| `Tab` | `next_target` |
| `Shift+Tab` | `prev_target` |
| `+` | `zoom_in` |
| `-` | `zoom_out` |

In look mode the direction keys move the cursor, `<` and `>` change
storey, Tab goes from one thing in sight to the next, and `+` and `-`
zoom. The panel says what is under the cursor, whether it is in sight
or remembered, and how far. Esc leaves.

### Menus and screens

| Keys | Name |
|------|------|
| `Up` | `prev` |
| `Down` | `next` |
| `Left` | `left` |
| `Right` | `right` |
| `Enter` | `accept` |
| `Space` | `toggle` |
| `Esc`, `q` | `back` |
| `Tab` | `focus_next` |
| `Shift+Tab` | `focus_prev` |
| `Page Up` | `page_prev` |
| `Page Down` | `page_next` |
| `Home` | `first` |
| `End` | `last` |
| `Backspace` | `erase` |
| `Delete` | `delete` |
| `Ctrl+D` | `done` |
| `F2` | `more` |

Lists also scroll with Page Up, Page Down, Home and End. Most screens
say their keys on their last line.

## Time

The game waits for you. Each thing you do takes time on your
character's own clock, and while it runs everyone else acts on the same
clock: a step at walking pace takes about two thirds of a second (more
with a heavy load), `.` waits one second, a door takes about half a
second to open, eating takes three. When it is done the game waits for
you again. `p` waits a minute and Shift+P an hour; either is broken off
when someone you hold a foe comes into sight or you are hurt.

What happened in that time is played back on the map. Any key skips to
the end of it. Options sets how fast it plays (0 shows it at once).

**Reactions.** A blow has a wind-up before it lands. If your character
notices it in time (an ordinary person needs about a fifth of a second),
the game stops and asks how to answer, with a bar of the time left:
**Parry** (needs something in hand with a flat, such as a blade or a
club), **Block** (on a shield, or the forearm), **Dodge** (duck out of
the line), **Step back** (out of reach), **Step aside** (across its
line), **Raise shield**. An arrow or a thrown thing seen coming asks the
same, and can be stepped aside from, stopped on a shield or caught; a
bullet arrives before anyone can begin. A choice marked "too late" will
not be ready in time. **Let it happen**, or Esc, takes the blow. A blow
you did not see coming, from behind or in the dark, simply lands.

## Getting about and using things

Walk with `h` `j` `k` `l` or the arrows, and `y` `u` `b` `n` diagonally.
Stairs go up with `<` and down with `>`.

**Walking into a shut door opens it.** A locked one says so; with its
key in your hands or a pocket, `e` offers Unlock. Options has "Into a
door": open it, ask first, or stop.

**`e` uses what is near**: everything within a step, and everything you
hold; above the list, what lies loose within a step that you can see,
each to pick up. What cannot be done now is greyed with the reason. What the
bundled world offers:

| Thing | What you can do |
|-------|-----------------|
| Doors | Open, Close; Lock and Unlock with the key |
| Windows | Open, Close, Break through (loudly) |
| Chests, cupboards, wardrobes | Open, Close; Take out what is inside, into your hands; Put in what you hold |
| Racks, stalls, workbenches | Take out, Put in |
| Wardrobes | Hide inside, and Get out |
| Ovens, hearths, stills | Stoke (put the wood you hold in), Light, Put out |
| What you hold | Eat, Drink, Bandage your wounds, Rub on salve, Coat a blade, Read |
| What lies loose (on a table, a desk, the street) | Pick up |

What a container holds is decided the first time anyone opens it:
pantry cupboards hold flour and herbs, wardrobes clothes, chests
valuables, the healer's house salves and bandages, the workshop's chest
knives and clubs, the racks of the guardhouse's armoury clubs,
bucklers, bows and arrows (the armoury door is locked: its key lies on
the sergeant's desk). **`g` or `,` picks up** what lies at your feet or
within a step, on a table, a desk or the street; with more than one
thing there it asks which. A thing too big for your hands goes into
something you carry that takes it, if anything does.

**`i` is your inventory**: what you carry, how, and what you wear on
each part of your body. Enter on a thing lists what to do with it:
**Drop** it where you stand, **Wear** it, **Take off** what you wear
(into a hand), **Stow** it in something you wear or hold, **Put in**
something you carry or that stands within a step, **Take out** what is
stowed (into a hand), **Throw** it, or **Give** it to someone within a
step, who takes it unless they hold you an enemy or are afraid of you.
Each takes a moment, and what cannot be done says why in the log: your
hands are full, it does not fit, it is not something to wear.

**`m` is the map**: the place as your character knows it, zoomed out,
with the journal's markers (`X`) where they last knew the thing to be,
and beside it the places they know by name and their goals, with which
way and how far each is. The direction keys move it; `<` and `>` change
storey.

## Your body

`c` shows your body part by part, how whole each part is, and every
wound. Wounds bleed until they close. **Blood** drains as you bleed:
below 60% you fall, below 40% you die. **Pain** rises with every hurt:
above 90% you fall. An arm too badly hurt to hold drops what it held.
Someone who has fallen is not dead, and can still bleed to death.

A bandage, from `e` while you hold it, closes every open wound and is
used up. Salve mends a little. Bread, ale and water are eaten and drunk
the same way.

**Food.** What you eat keeps you going for about a day and a half.
Below a quarter you are **hungry** (a little weaker), below a tenth
**weak with hunger** (weaker and slower), and with nothing left
**starving**: your body begins to fail, and about a day later you fall,
and later die, unless you eat. The status column shows it once you go
short; `c` shows how fed you are. A loaf is a good meal.

## Fighting

Fights are short and ugly. There are no hit points: a blow lands on a
part of the body, through what is worn there, and does what its edge,
point or weight does to skin, muscle and bone. An arm cut badly enough
lets go of what it held, and a deep wound bleeds until it is bound.

What decides a blow is who sees it coming. Someone who notices the
wind-up in time can parry, block, dodge or step back (see
[Reactions](#time)); someone caught from behind, in the dark or busy
with something else is simply hit. Facing matters: people see in an arc
ahead of them.

**Choosing whom.** Tab selects the nearest creature in sight as your
target, and Tab again the next one; Shift+Tab goes back. The status
column names the target and the map marks it. Only someone your
character sees can be chosen.

**Hand to hand.** `f` strikes the target with the first blow you can
make now: a punch or a kick with empty hands, a cut or a thrust with a
knife, a blow with a club. With no target it asks for one among those
in sight. A target out of reach is not struck ("That is out of
reach."): walk up first. With nobody in sight it says so. `F`
(Shift+F) lists every blow, greying those you cannot make with the
reason, and asks how: the target, the part of them you aim at, and how
hard. Walking into someone you hold hostile or fear strikes them;
walking into anyone else asks first ("Into someone" in Options changes
that).

**At range.** `s` shoots or throws at the target with the first shot
that can be made; `S` (Shift+S) chooses the shot, the place aimed at and
the height, with the line drawn to it. `r` reloads the weapon in hand
from what you carry, `t` throws what is in your hand, and `z` stops what
you are doing. An empty weapon says so.

**Holds and takedowns.** `v` lists the holds, throws and takedowns you
could try on the target, greyed with the reason where you cannot: grab,
drag, throw, hold up as a shield, break free, knock out, choke.

In a window, the second mouse button on someone offers Attack, Shoot
and Talk, beside Go there, Use what is near and Look around.

What you hold matters. A knife or a club can parry; a buckler blocks.
The workshop's chest keeps knives and clubs, and the racks of the
guardhouse's armoury clubs, bucklers, bows and arrows.

## Powers

`a` lists your abilities: each use with its rank, what it costs, and
whether it is ready, cooling down or cannot be used now, and why. Enter
uses one, asking for whatever it needs. Modes, such as a shield kept up,
are switched on and off there. On that screen a number key puts the use
selected on a quick slot; in play the same number uses it, at the target
if it takes one.

| Keys | Name |
|------|------|
| `1` to `9` | `bind_1` to `bind_9` |

An ordinary person has no powers, so the list is empty. Make an adept
(two or three powers of your choosing) or a gifted person (all twelve)
with F2 on the new-game screen, in the market town or the town, or
choose one in the errand or the last stand.

## Talking

`T` (Shift+T) talks to the target: it lists what you could say to them
(in the bundled game, a greeting, or telling them to stay back) and
Enter says it. What is said back is in the log. Anyone in earshot hears
what is said, and people pass on to each other what they saw.

## Making things

`M` lists what you could make here: by hand, and at each workbench,
oven, hearth or still within a step. Each line says what it takes and
gives, and greys out what you cannot make now with the reason (missing
inputs, missing tools, a cold oven, know-how you lack). Enter starts the
work; it goes on until it is done or you do something else, and work
left half done stays where it was. A lit still or a herb bed keeps
working with nobody at it.

The bundled recipes: bandages from cloth and torches from a stick and
cloth, by hand; clubs from a log and arrows from sticks and feathers, at
a workbench with a knife; bread from flour and water at an oven or a
hearth; mending a blade at a workbench with something to hammer with;
distilling ale in a still; herbs grown in a herb bed with water about
it; and, for those who know herbalism, healing salve at a workbench,
nightshade extract by hand and a sleeping draught at a hearth. An oven,
a hearth or a still must be stoked with wood and lit first.

`K` lists know-how: what you know, what each lets you make, and what
you could learn next. Reading a book that teaches something (the
healer's herbal teaches herbalism) teaches it.

## Hiding and being seen

People see and hear as you do, and act on what they perceived, not on
what is true.

- **Sight** reaches about sixty metres, in an arc ahead of the eyes, and
  needs light. In daylight a person is seen at once; in deep shadow, not
  beyond a step or two. Night, unlit rooms and walls hide you; a window
  or an open door lets the sky's light in. Lamps can be switched off.
- **Hearing** reaches about forty metres all round, muffled through
  walls. Footsteps, doors, breaking things and fighting make noise; a
  loud noise brings people to look.
- **Awareness builds.** A glimpse at the edge of sight makes someone
  suspect something; it takes longer to be sure, and you can slip away
  first. Once they lose you they remember where you were.
- **Hiding inside.** In a wardrobe you cannot be seen at all, though
  sound still leaves.
- **Memory.** Townsfolk run from danger close by and keep away from
  anyone they saw hurt someone. Those who think as watchmen do (the
  strangers of Last stand are such) go after anyone they hold hostile or
  saw hurt someone; once they lose them they go to where they last
  believed them to be, look round, and give up. People tell each other
  what they saw, at a little less certainty each time. (In this beta
  the town's watch does not yet come for you when it is your character
  who strikes: see the release notes' known issues.)

Your character works the same way: the map shows what they have seen,
and someone you cannot see is not drawn.

## Saving

Esc in play opens the pause menu: Resume, Save, Load, Options and Quit
to title. Save asks for a name (left empty, your character's); a save of
the same name is replaced. Load lists the saves, newest first; one made
with other mods is marked, loads, and says what differs. Continue on the
title loads the newest.

Each game decides how it is saved. The errand, the market town and the
two walks let you save and load as you like. Last stand has one save,
written as you leave the game and removed when the game ends; Save and
Load are not offered while you play it, and Continue resumes it.

Saves are files in the `saves` directory of your data directory
(`~/.local/share/clash2d` on Linux; the [README](../README.md#saves-and-settings)
lists the others).

## When it ends

The errand ends when the parcel reaches the town hall, or when it is
lost: destroyed, in someone else's hands, or left lying (or put away)
for ten minutes; or when night falls. The log says which. Last stand
ends at dawn. The summary says how the
game ended, how long it ran and how each goal stood (`[x]` done, `[-]`
failed, `[ ]` not), and returns to the title. What happens when you die
is in the table under [Starting a game](#starting-a-game).

The journal (`J`) shows the goals as your character knows them, with
which way and how far each one is.

## Tips

- Look before you walk in: `x`, then Tab, goes through what is in sight.
- Tab before you fight: a target chosen is a blow aimed, and `F` shows
  every blow you could make, and why the others cannot be.
- Light gives you away. At night, out of the light of windows and
  doors, you are hard to see; so is everyone else.
- People remember. If someone saw you hurt someone, leave their sight
  and stay out of it until they give up.
- An ordinary person is ordinary. Two strangers who mean it can kill
  you; run, shut a door between you, or hide.
- Bleeding kills slowly and surely. Bandage early: the healer's house
  keeps bandages, and a piece of cloth makes two by hand (`M`).
- In Last stand it is dark and you see little beyond the lamp by the
  house's door, a few steps north-east (the journal, `J`, says which
  way); get in and shut it. Footsteps you hear show as `!` where they
  seemed to be, and the lamps show as a yellow `!` too. The lamp inside
  lets you find the club and the bandages on the table, and lets them
  see you: switch it off (`e`) to hide. They find you more easily as
  the sky lightens before dawn.
- Blocking a knife with your forearm stops the blow and cuts the arm:
  the log says when you are wounded. Stepping back or dodging costs
  nothing when it works. Binding wounds takes a free hand.
- The same seed gives the same town. If a game went badly, try it again
  knowing where things are.
