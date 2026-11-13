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
| **Last stand** | Ten at night, a house on a country lane, three strangers who mean you harm. Get inside and hold out until dawn, eight hours on. | The game ends, and its one save (written when you leave, resumed with Continue) goes with it. |
| **The errand** | Two in the afternoon. Take the parcel in your hands to the town hall in the middle of the market town before nightfall; you have five hours. | The game ends. Save and load as you like. |
| **The town** | A grid of streets with every door open. Nothing to do but walk. | The game ends. |
| **The market town** | The sandbox: a generated town and its people, no goals, no end. Begin on the north street or in the town square. | You carry on as a newcomer, made fresh where you first began; your body stays where it fell. |
| **The lane** | Houses between fields, their doors shut. Nothing to do but walk. | The game ends. |

Tab moves between the name, the seed and the list; Enter on a game
begins it. The market town first asks where to begin.

You play an ordinary person: no powers, no training, nothing in your
hands but what the game gives you.

A game with a story opens with a few lines of introduction; Begin, or
Esc, closes them.

## The screen

```text
""Y"""Y"""""""""""""""""""Y"""...""Y""""""*"Y"Y*Y"""┌──────────────────────┐
""""""""""Y"Y"""""Y"""Y"""""""...Y""""""Y"""""""""""│Ada                   │
*""""""""""""""Y"""""""""Y""""..."""""Y"*"""""""""Y"│Blood  ███████████████│
,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,│Pain   ░░░░░░░░░░░░░░░│
....................................................│Holding something     │
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
perceived, worded as they would know it: a stranger is "someone" until
you know them. It also says why something you tried could not be done.

**The status column** has your name, your **Blood** and **Pain**, and
what is in your hands.

## Keys

`?` or F1 in play lists every key of the keymap in force. Options (`o`,
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
| `x` | Look around | Opens Look. |
| `e` | Use something near | List what can be done with the things within a step, and do one. |
| `i` | Inventory | Opens Inventory. |
| `c` | Condition | Opens Condition. |
| `a` | Abilities | Opens Abilities. |
| `m` | Map | Opens Map. |
| `Shift+J` | Journal | Opens Journal. |
| `Shift+M` | Make something | List what can be made here, by hand and at the stations within a step, and start one. |
| `Shift+K` | Know-how | List what the character knows how to do and what it could learn next. |
| `o` | Options | Opens Options. |
| `?`, `F1` | Keys | Show what each key does in play. |
| `Esc` | Pause | Open the pause menu. |
| `F12` | See everything | Switch between what the character sees and what is really there, in a development profile. |

These have no key: the application runs the first two when you walk
into something closed, and the mouse the last two.

| Control | Does | What it does |
|---------|------|--------------|
| `bump` | Use what is in the way | Do what the thing walked into offers by default: open a shut door. |
| `bump_ask` | Ask before using what is in the way | Ask, then do what the thing walked into offers by default. |
| `click` | Go there |  |
| `click_second` | What can be done here | Opens Here. |

`Shift+J` is a capital J, and so on. F12 shows the world as it
really is, not as your character sees it; it works only in the
development profile (`-- --profile dev`).

In a window the mouse works too: the first button on the map walks
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
with a heavy load), `.` waits one second, a door takes half a
second to open, eating takes three. When it is done the game waits for
you again.

What happened in that time is played back on the map. Any key skips to
the end of it. Options sets how fast it plays (0 shows it at once).

**Reactions.** A blow has a wind-up before it lands. If your character
notices it in time (an ordinary person needs about a fifth of a second),
the game stops and asks how to answer, with a bar of the time left:
**Parry** (needs something in hand with a flat, such as a blade or a
club), **Block** (on a shield, or the forearm), **Dodge** (duck out of
the line), **Step back** (out of reach). A choice marked "too late" will
not be ready in time. **Let it happen**, or Esc, takes the blow. A blow
you did not see coming, from behind or in the dark, simply lands.

## Getting about and using things

Walk with `h` `j` `k` `l` or the arrows, and `y` `u` `b` `n` diagonally.
Stairs go up with `<` and down with `>`.

**Walking into a shut door opens it.** A locked one says so; with its
key in your hands or a pocket, `e` offers Unlock. Options has "Into a
door": open it, ask first, or stop.

**`e` uses what is near**: everything within a step, and everything you
hold. What cannot be done now is greyed with the reason. What the
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

What a container holds is decided the first time anyone opens it:
pantry cupboards hold flour and herbs, wardrobes clothes, chests
valuables, the healer's house salves and bandages, the workshop's chest
knives and clubs, the racks of the guardhouse's armoury clubs,
bucklers, bows and arrows (the armoury door is locked). What lies loose
on a table, a desk or the street cannot be picked up yet.

**`i` is your inventory**: what you carry, how, and what you wear on
each part of your body.

## Your body

`c` shows your body part by part, how whole each part is, and every
wound. Wounds bleed until they close. **Blood** drains as you bleed:
below 60% you fall, below 40% you die. **Pain** rises with every hurt:
above 90% you fall. An arm too badly hurt to hold drops what it held.
Someone who has fallen is not dead, and can still bleed to death.

A bandage, from `e` while you hold it, closes every open wound and is
used up. Salve mends a little. Bread, ale and water are eaten and drunk
the same way.

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

What you hold matters. A knife or a club can parry; a buckler blocks.
The workshop's chest keeps knives and clubs, and the racks of the
guardhouse's armoury clubs, bucklers, bows and arrows.

The default keymap does not yet start a blow, a shot or a power of your
own: the [release notes](release-notes-beta.md#known-issues) say what is
missing.

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
  what they saw, at a little less certainty each time.

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
lost or night falls. Last stand ends at dawn. The summary says how the
game ended, how long it ran and how each goal stood (`[x]` done, `[-]`
failed, `[ ]` not), and returns to the title. What happens when you die
is in the table under [Starting a game](#starting-a-game).

The journal (`J`) shows the goals as your character knows them, with
which way and how far each one is.

## Tips

- Look before you walk in: `x` and Tab go through what is in sight.
- Light gives you away. At night, out of the light of windows and
  doors, you are hard to see; so is everyone else.
- People remember. If someone saw you hurt someone, leave their sight
  and stay out of it until they give up.
- An ordinary person is ordinary. Two strangers who mean it can kill
  you; run, shut a door between you, or hide.
- Bleeding kills slowly and surely. Bandage early: the healer's house
  keeps bandages, and a piece of cloth makes two by hand (`M`).
- In Last stand it is dark and you see almost nothing. The house's door
  is two steps north (the journal, `J`, says which way); get in and shut
  it. Footsteps you hear show as `!` where they seemed to be.
- The same seed gives the same town. If a game went badly, try it again
  knowing where things are.
