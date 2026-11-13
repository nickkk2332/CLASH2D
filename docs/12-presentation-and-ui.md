# 12 — Presentation and the UI Toolkit

Status: design complete, ready for implementation.
Implementation plan: [plans/first-playable.md](plans/first-playable.md).
Serves scenes S5, S6, S20 and S21 of [00-vision.md](00-vision.md), and is how
every other scene is seen.

## 1. Scope

This document specifies everything between the simulation and the player's
eyes and hands: the scene layer, how the world is drawn in glyphs, how
events are played back over real time, cutscenes, input, the UI toolkit that
all screens are built from, and the two frontends.

Crates: `clash-scene` (frontend-agnostic), `clash-front-window`,
`clash-front-term`, `clash-app`.

Fixed from earlier documents:

- Presentation never changes outcomes (01 §10). It reads a view of the
  simulation and submits commands.
- The display is a grid of glyphs. There is no pixel widget layer.
- The engine's own screens are built with the toolkit and ship as the `ui`
  mod. A profile names the screen it boots into (04 §3.3).
- The player is shown what their characters perceive (15 §13).

## 2. The picture

The simulation does not run in real time, and it finishes each stretch
between decisions almost instantly. Presentation receives the events of that
stretch, each stamped with when it happened, and **plays them back** at a
pace a person can follow: glyphs slide between cells, a wall bursts into
fragments, a line appears in the log. When playback reaches the moment a
decision is needed, the prompt appears.

To do that the scene keeps its own small mirror of the world, the **view
model**, which holds what is on screen right now and is updated only as
events are played. It never shows more than the controlled characters
perceive.

Some moments get more: a **cutscene** is a short choreography of camera,
slow motion and captions wrapped around events that have already been
decided.

Everything else on screen is **UI**: panels, lists, bars, menus, maps. Every
screen is a definition in JSON made from a fixed set of **widgets**, with
contents bound to expressions and keys bound to effects. The title screen,
character creation and the in-game display are screens like any other, and
a mod can add to them or replace them.

## 3. Structure

```
            ┌──────────── clash-app ─────────────┐
 Frontend ──┤ input ► Scene ──commands──► Sim    │
 (window or │         │  ▲                 │     │
  terminal) │  frame ◄┘  └──events, view───┘     │
            └────────────────────────────────────┘
```

```rust
pub trait Frontend {
    fn caps(&self) -> Caps;                 // sub-cell motion, zoom, alpha, mouse, colour depth, size
    fn poll(&mut self) -> Vec<InputEvent>;
    fn present(&mut self, frame: &Frame);
}
```

The application loop:

1. Pass input to the scene. The scene may submit commands.
2. If the scene wants more simulation (no decision outstanding, playback
   not far behind), call `advance(Work(n))` and give the scene the drained
   events.
3. Advance the scene by the real time elapsed.
4. Build a frame and present it.

The scene holds a read-only `SimView` and a handle that can only `submit`.
It has its own random stream and may use ordinary floating point; nothing
it computes flows back into the simulation.

### 3.1 Starting the application

The application is the crate `clash-app`: a library, and the binary
`clash`, which calls it and nothing more.

```text
clash [--profile name] [--mods dir] [--seed n] [--data dir]
      [--window | --terminal] [--headless script]
```

| Option | Means |
|--------|-------|
| `--profile name` | The profile to start: a name, looked up as 04 §3.3 says, or the path of a profile file. Left out, `default`. |
| `--mods dir` | The directory mods are read from. Left out, the first that exists of `mods` in the working directory, `mods` beside the executable, and `mods` in each of the three directories above it (where a build run from the repository finds the repository's). |
| `--seed n` | The seed the new-game screen offers. Left out, 1. |
| `--data dir` | The user's data directory, in place of the one below. |
| `--window` | Shows the game in a window of its own (section 11). |
| `--terminal` | Shows the game in the terminal. With neither this nor `--window`, the game is shown in a window when a display is available, and in the terminal otherwise. Giving both is a mistake. |
| `--headless script` | Runs without a terminal, from a script (section 3.3); `-` reads the script from standard input. |
| `--help` | Prints the usage and leaves. |

`--name=value` means the same as `--name value`. A command line that
cannot be read prints what is wrong and the usage, and leaves with code
2.

**The user's data directory** is the first of: `--data`; the
environment variable `CLASH_DATA_DIR`; `$XDG_DATA_HOME/clash2d`; on
Windows `%APPDATA%\clash2d`; on macOS
`$HOME/Library/Application Support/clash2d`; `$HOME/.local/share/clash2d`.
An empty variable counts as unset. With none of them there is nowhere
to keep anything, and choices last for the run. It holds:

| Path | Holds |
|------|-------|
| `preferences.json` | The player's preferences by qualified id, and the keys they bound, each with the keymap it was made in, the context, the key as a keymap writes it, and the name (empty for a key whose meaning was taken away). |
| `mods/` | Mods searched before the installed mods directory, so that a copy there replaces an installed mod of the same id (04 §3.1). |
| `saves/` | Saves, one file each (section 3.6). |

```json
{ "preferences": { "ui:theme": "ui:night" },
  "bindings": [ { "keymap": "ui:default", "context": "play", "key": "w", "name": "north" } ] }
```

The file is written whenever the scene reports a change through
`set_pref` or `rebind` (section 10.5), through a temporary file beside
it so that a failed write leaves the old one whole. At start each stored
preference that a loaded mod still defines is given back with
`Scene::set_pref`, and each binding made in the keymap in force with
`Scene::rebind`; the others stay in the file untouched. A file that
cannot be read, or is not what the application writes, is reported in
the log and never written over.

**Loading.** The profile is read with the engine's definition types and
expression registry. The interface's definitions are built and checked
(section 10), and the world's and generation's tables are built. A
profile that defines no `scenario` (section 3.4) need not have world
tables that build: a total conversion may be a title screen and nothing
else. Whatever is wrong is printed on standard error as located
diagnostics, each with its file, line, column and JSON path, and the
program leaves with code 1 having drawn nothing. Warnings are put in
the log.

**The boot screen.** The profile's `boot_screen` is then opened, and
the first frame presented is that screen: nothing of the engine's is
drawn before it (04 §14). It is drawn with the theme and keymap that the
preferences named `theme` and `keymap` hold; a profile that defines
neither is drawn in the plain built-in theme and moved through with the
keyboard's own navigation keys.

**A window or the terminal.** Without `--headless` the application
runs in a window or in the terminal, as `--window` and `--terminal`
say; with neither, in a window when a display is available (on Linux
and the BSDs, when `WAYLAND_DISPLAY`, `WAYLAND_SOCKET` or `DISPLAY` is
set; on macOS and Windows, always). The window is drawn with the font
of the theme in force (section 4.3) and titled with the profile's
name. A window that cannot be opened (no display, a theme with no font,
a font file missing or unreadable, which is named with its path, or a
window system that refuses) is said on standard error, and the
application runs in the terminal instead (decision D-843).

In the terminal, if standard input or output is not a terminal it says
so, names the boot screen that was ready, and leaves with success. The
terminal is given back however the application leaves: the frontend is
dropped when the loop ends, for whatever reason, and a panic restores
the terminal before its message is printed.

### 3.2 The frame loop as built

One pass of the loop is one frame, 16 ms in a terminal (decisions D-590,
D-652):

1. **Input.** What the frontend polled goes to the scene, which runs the
   effects it asks for: screens open and close, commands are submitted
   through the host, services are asked of the application. Input
   applies to the present: a key pressed while something is being
   played jumps playback to its end before it is used (section 7.1),
   and the loop does the same for a press of a mouse button and for
   pasted text.
2. **Follow-up.** A refused command and an effect that could not be run
   each get a line in the log. An `app` service that failed also opens
   the screen of the role `error` with the reason as its parameter
   `text`. A game that was started or loaded closes every screen and
   opens the one of the role `play`; a game whose play screen has been
   closed is over.
3. **The simulation.** What is due at the present tick runs first,
   even while a decision is asked: the simulation runs it before it
   asks (03 §5.2), and a sense whose owner has just been put in the
   world or has just stepped sweeps then, so that the character has
   looked before it is asked (decision D-700). While the player's
   character waits on the simulation, which is while no decision is
   asked of it and it is busy (an action, a wait, a plan) or
   unconscious, the simulation is advanced toward the character's next
   decision with `advance(Work(256))`, `UntilDecision` in slices, until
   a decision is needed, nothing is left to happen, 8 ms of the frame
   have been spent or an hour of simulation time has passed in the
   pass. A character that is free, dead or out of the world is not
   waited on. Nothing else advances the simulation.
4. **Playback.** What the simulation reported in the pass, the
   commands' events and then the advance's, is drained and handed to
   the scene as one batch (`push_events`), after what the party
   perceives as the batch ends has been taken from the simulation's
   perception (section 3.5), so that the host answers what the party
   perceived as of the end of the batch (section 7.5). A fault and a
   `message` for the player also get a line in the log at once
   (categories `fault` and `message`). Every line of the log begins
   with a capital letter, made so as it is added (decision D-866). The scene is advanced by the
   real time elapsed and plays what falls due.
5. **The present.** Once playback has caught up, the view model is
   given what the party perceives that no event said (section 3.5):
   cells that came into view or left it, and who is in view. In a
   debug build it is then checked against a fresh view model
   (section 5), and a drift is written to the log as a fault. A
   decision that is waiting gets its screen (section 9.5).
6. **The frame.** The scene builds a frame of the frontend's size, and
   the frontend presents it. The rest of the frame is slept out.

The loop never waits on the simulation: a pass that runs out of its
slice presents a frame and carries on in the next. A request a rule
makes of the application (13 §9.1) is carried out between two calls of
`advance`, which is a safe point (decision D-757): a checkpoint is saved
under its name unless the game's saving policy is `none` (section 3.6);
`RestoreCheckpoint` loads the save of that name and carries it on as a
load does; `TakeControl` makes the character the player's, the camera
following it, and says so in the log; `EndScenario` stops advancing the
game, removes its save under `single` and opens the summary (section
3.4). The loop also runs while a request waits.

The commands the play screen submits become the simulation's own
through the host (`clash_app::host::command_for`): an `act` of an
action is `Act` with `start` for the player's character, the action by
its qualified id (a bare name means the first action of that name in
order of qualified id) and its parameters as the scene assembled them;
the command `wait { dur }` is `Wait`, `cancel { action, abandon? }` is
`Cancel`, `clear_plan` is `ClearPlan`, and `choose { entity, choice,
option }`, which the choice prompt submits, is `Choose`. `react {
entity, window, action, params }` and `pass { entity, window }`, which
the reaction prompt submits, are `React` and `Pass` (03 §7.3); the
decision behind the prompt is the simulation's `Reaction` decision,
handed to the scene with its options, the time left and in all, in
the character's own time (decision D-717), and what is happening as the
character perceives it: who does what, `The brute: Punch` (decision
D-798). `use_ability { ability, action?, params? }`, which the
abilities screen and the quick slots submit, is `UseAbility` (the
ability by its id or qualified id, the grant's action by its id or
name); `set_mode { ability, mode, on }` is `SetMode` (08 §5, decision
D-795); and `cancel` with no `action` cancels the character's newest
action, which lets a held phase go. A `run` of a rule is
refused with the reason until the `Ui` command is built. A refusal is the simulation's
reason, written in the log; walking into a shut door is the one
refusal the loop answers itself (section 3.7). Tools and tests hand the
game a command of their own with `Game::submit`, through `Sim::submit`
as everything else.

Nothing stands in for a package any more. A game begins from a game
scenario (section 3.4), and the simulation gives the party its senses
as the game begins, where templates give none (section 3.5).

**The omniscient view.** While a preference whose bare name is
`omniscient` holds true, the view model is filled from truth round the
character instead (section 5, `view::omniscient`), and the simulation's
switch `perception.omniscient` is on, so that every event is seen
(15 §13). Switching it on empties the view model first; switching it
off turns the simulation's switch off and builds the view model afresh
from the party's map memory and what it sees, so nothing learnt from
truth is left behind (decision D-706). The preference is never kept
between runs. The `ui` mod defines it with the control `ui:omniscient`
on F12 in play, which switches it only while the preference
`ui:development` holds. No screen offers that one; a development
profile sets it, as `profiles/dev.json` does, so in any other profile
F12 does nothing. The toggle moves to the `tools` mod (04 §3.5) when
that exists.
### 3.3 Headless scripts

`clash --headless script` runs the same loop on the null frontend, on a
grid of 80 × 24, with a fixed step of a sixtieth of a second. Each pass
advances the simulation until the player's character is asked what it
does next (or an hour of simulation time has passed), and playback by
the fixed step, so a script gives the same frames every time (V8). A
stretch that plays over time is played on by `wait` lines, and jumped
to its end by the next key, as a player's is. The script is read a
line at a time:

| Line | Does |
|------|------|
| A key, as a keymap writes it (section 9.1): `enter`, `down`, `k`, `shift+a`, `f12` | Presses it, then one pass |
| `type text` | Presses the key of each character of the rest of the line, a pass after each |
| `wait`, `wait n` | One pass, or `n`, with no input |
| `frame` | Prints the frame as plain text: a line `--- frame ---`, then one line for each row |
| `frame golden` | Prints the frame as a golden frame is written (section 12) |
| `size cols rows` | Changes the grid, then one pass |
| `# ...`, or nothing | Nothing |

Any other line stops the run with its number and what is wrong, and the
program leaves with code 1. Once the game has ended the rest of the
script is not read.

### 3.4 Game scenarios

A game scenario (13 §3) says which world a game has, who the player is
and where they begin. The application offers the `scenario`
definitions of the loaded mods as `app.scenarios`, in order of
qualified id (decisions D-750, D-758):

```json
{ "type": "scenario", "id": "wayside", "name": "The lane",
  "doc": "Houses between fields. Their doors are shut.",
  "worlds": [ { "realm": "countryside", "seed": "ask" } ],
  "start": { "characters": [ { "template": "person" } ],
             "at": [ { "name": "The lane", "cell": [39, 45] } ] } }
```

The service `new_game { scenario, place? }` starts the game: the
scenario's world made by `clash_sim::scenario::make_world` with the seed
its `seed` chooses (the one given on the new-game screen for `ask`, one
the application draws from the clock for `random`) and its clock at
the time the scenario begins; a simulation over it with the generators
of the loaded mods and a store; the player's character, made by the
debug command `spawn` (`Sim::spawn_template`, 06 §10.4) from the
character the player finished in the creation session, or else from the
scenario's character `template` with every choice at its default and
the name given; and the command `BeginScenario`, which places it, gives
it senses and direct control, and sets the game up (13 §4.5). With no
`scenario` it takes the first. The character is drawn with its
templates' glyph.

**Setting a game up.** A scenario with more than one place, or with
options, asked for without `place`, is not begun: the application
holds it as `app.setup` and opens the screen of the role `setup` with
the parameter `scenario`. `app.setup` is a map of the scenario's `id`,
`name`, `doc`, `seed` (`ask`, `random` or the number), `places` (each
`name` and `index`) and `options` (each the setting's `id`, `name`,
`kind`, `value`, `next`, the value after it in its choices, and
`choices`). The service `creation` with the op `option`, a `key` and a
`value` answers an option, checked against the setting's definition;
`new_game` with `place` then begins the game there with the answers
given, or each setting's default.

**The introduction and the summary.** A game begun from a scenario
with an `intro` opens, over the play screen, the screen its `screens`
names for `intro`, else the screen of the role `intro`, with `title`
(the scenario's name) and `text`. When the game ends (13 §9.4) the
application opens the screen named for `summary`, else of the role
`summary`, with `title`, `outcome` and `text`: how long the game ran
and each objective and mission as the character knows them (`[x]` done,
`[-]` failed or expired, `[ ]` not). An ended game is not advanced,
cannot be saved, and saves nothing as it is left.

The bundled `core` mod has five, all of the template `core:person`
(the sandbox and the town let the player make the character, the
errand and the last stand choose one ready-made, the lane gives it;
13 §4.2):
`core:town`, the test city with its doors open; `core:wayside`, the
countryside; `core:town_life`, the generated market town to live in,
begun on its north street or in its square, carrying on as a newcomer
when the character dies; `core:the_errand`, a parcel to take across the
market town; and `core:last_stand`, a night in a house on the lane.

**The character being made** (decision D-653). The application keeps
the creation session (06 §10.3) beside the scene, with the name and the
seed the next game is made with and the character the player finished,
if they did. The interface reads it as `app.creation` and edits it with
the service `creation`:

| `op` | With | Does |
|------|------|------|
| `begin` | `template` | Starts a session over that template, every choice at its default, and drops a finished character. The name and seed given so far are kept, and the name is selected for the template's `text` choice that `sets` the name, if it has one. |
| `select` | `key`, `value` | Selects for an open choice by its key (`core:person.build`): a list of option or part ids, one id, a number or a text, as the choice takes. |
| `clear` | `key` | Takes a selection back: the choice is at its default. |
| `name` | `value` | The character's name, a text: kept, and selected for the choice that sets the name. |
| `seed` | `value` | The seed the next game's world is made with and random selections are drawn from, a whole number. |
| `finish` | | Checks the character: refused with the reason if it cannot be made, otherwise kept for `new_game`. |
| `scenario` | `value` | The game the character is made for, by its scenario's qualified id, or null: `app.templates` then offers what that game allows (13 §4.2). A change drops the session and the finished character. |

A refusal is the service's, so the application shows it on the screen
of the role `error` (section 3.2). What may be made is the game's
(decision D-860): `app.templates` lists the templates the game named
with `scenario` offers (what its character entry's `pick` lists, or
every playable template), each with its `name` and `description`, or
with no game named every playable template; `begin` takes only one of
them; in a game whose character is chosen ready-made (`create` false)
`select` and `clear` are refused but for the choice that sets the name,
and `app.creation.editable` is false. `app.creation` also has
`made_from`, what the session's template is called, and each choice a
`shown`, what is selected as a reader reads it (the options' names, the
number, the text). `new_game` makes the finished character only for the
game it was made for (or for none named, if that game offers its
template), and otherwise the scenario's template. Every op but `name` and `seed` needs
a session in progress; those two are kept without one. Any change to a
session drops a finished character, which must be finished again. Once
a game has begun the session and the finished character are dropped,
and the name and the seed kept for the next. The older service
`edit_creation { field, value }` sets the `name` or the `seed`, as the
ops of those names do. A screen that declares a variable named
`creation_<field>` has it set, when it opens, to the `name` or the
`seed`, whether or not a session is in progress, so the new-game screen
starts with the seed given on the command line, and gives both back
with `creation` before it asks for `new_game`. The name is the
character's own when its template has a choice that sets one; where it
has none, `name()` of the player's character gives it.

### 3.5 What the player perceives

The simulation decides what the party sees and hears (15 §13); the
application, in `clash_app::perception`, turns that into the view model
and into the host's answers to playback (decision D-700).

**Senses.** As the game begins, before the character is put in the
world, the command `BeginScenario` gives it senses if its templates gave
it none: those its scenario's character entry names in `senses`, or
else every `sense` the loaded mods define, in order of qualified id
(decision D-751). Once its control is direct it is of the party.

**As a batch ends**, the frames are taken (`drain_frames`), and what is
in view is worked out: the cells of the party's view as the members
last observed (`party_view`), each with what is in it now (its
elements, strongest field, most marked decal, whether it is open to the
sky, and its light by the sky and the fixed sources, as map memory
records them); the party's own members, always, where they are; and
whatever stands in a cell in view and gives off something a sense of
light could see. A creature is drawn with its templates' glyph, an item
with its definition's, anything else as the theme's `suspected` mark.

**Names on screens** (decision D-862). The text helpers a screen or a
show calls (`name`, `the`, `a`, `list`, ...) read through the reader
lens with the player's character as the reader, so that everyone else
is named as the character knows them (`the person`, a name it has
learnt) and a thing by what it was made from (`a parcel`); the
character itself is named by its own name, or the one the player gave
it, not "you".

**Playback is answered** from it: `perceived` is the simulation's tag
(`Sim::party_perceived`): seen, heard from a place, or neither;
`seen_cell` is what is in a cell of the view now, or nothing out of
view; `seen_sprite` is how a thing in view is drawn, or that a thing
once perceived is not perceived now. What was only heard is a marker
and the show's `heard` line, by playback; the `ui` mod's
`heard.json` gives footsteps one. The frames' lists of what was heard
are the same signals and are not shown again.

**Once playback has caught up** the view model is given what no event
said: each cell a frame recorded or the view left, as the member's map
memory holds it and remembered; each cell in view, as it is now and
seen; and who is in view, each thing that is not leaving the picture.
So a cell in view is drawn as it is (a door seen opening is drawn open
at once, before the next sweep records it), and a remembered cell is
always the simulation's own memory. The fresh view model a debug build
checks against is made the same way from the map memory (`snapshot`)
and what is in view. On a game begun or loaded, and when frames were
dropped, the view model is built afresh so.

### 3.6 Saving and loading

Saves are files in the `saves/` of the user's data directory, one for
each save, written and read by `clash-save` (`save_game`, `load_game`,
`read_meta`, `list_saves`; 01 §11.2) through `clash_app::saves::FileSaves`
(decision D-703). With no data directory nothing can be saved, and the
services say so.

| Service | Does |
|---------|------|
| `save { name? }` | Saves the game as `saves/<id>.c2save`, its id made of the name's letters and digits in lower case with `-` between (a name of nothing is the character's name, or else the scenario's). A save of the same id is replaced. The metadata holds the name, a summary the application writes (`Ada, The town, 0:04:12 in`: who, where the game began and how long it has run), the seed, and as the saver's own bytes the scenario, the character's name and who the character is. A line in the log says it was saved. |
| `load { save }` | Reads the game back over the loaded definitions and the generators bound to them, and carries it on: the screens give way to the play screen, the view model is built afresh from what the party remembers and sees (section 3.5), and nothing is played back. What differs from when it was saved (`load_warnings`: mods missing, added or at another version, definitions that became placeholders) is written in the log and shown on the screen of the role `notice`, with the parameters `title` and `text`. The state hash is what it was when the game was saved. |
| `delete_save { save }` | Deletes the file. |

`app.saves` lists the saves newest first, by when their files were
written. An entry's `name` is the name it was given, its `description`
the summary and when it was written (`2026-11-12 09:30`, UTC), and its
`state` `enabled` for a save made with the mods loaded now, `changed`
for one made with other mods or versions (it loads, with warnings) and
`broken` for one that cannot be read (its `description` says why).

**Saving policies** (13 §9.2). A game has the policy its scenario names
(`saving.policy`), or `free` (decisions D-704, D-757):

| Policy | `app.can_save` | `app.can_load` while playing | Saved by the application |
|--------|----------------|------------------------------|--------------------------|
| `free` | True | While there is a save | Never by itself |
| `single` | False | False | As the game is left, under the character's name: its one save, which an ending removes |
| `checkpoints` | False | False | When a rule asks for a checkpoint |
| `none` | False | False | Never |

From the title `app.can_load` holds while there is any save. A rule's
checkpoint is saved under its name under every policy but `none`.
`single` does not yet save at intervals.

### 3.7 Interactions in play

**Walking into something closed.** A step the simulation refuses
because the way is closed (`move.closed`: what stands there could
stand aside and does not, as a shut door; a step action's `requires`
refuses with its own `fail_text`, so the host asks movement what the
step would meet) is a bump (decision D-701). The application asks the
simulation what is there by default (`interaction::bump_default`: the
block's, else the fixture's, else the floor's default that can be done
now) and, by the preference whose bare name is `bump`:

| `bump` | Walking into it |
|--------|-----------------|
| `at_once` (the default, or no such preference) | Runs the control named `bump` with the offer as `it`: the `ui` mod's starts `interact` with its `target` and `interaction` |
| `ask` | Runs the control named `bump_ask`: the `ui` mod's asks with `confirm`, then does it |
| `never` | Nothing: the step's refusal is written in the log |

Where nothing there can be done by default and something would be, the
refusal of each such default is written in the log in place of the
step's: walking into a locked door without its key says "It does not
open. It is locked, and you have no key.". The offer `it` is a map
with the fields of a row of `offers_near` (below), its `where` the
direction walked.

**Walking into someone** (decision D-790). A step toward a creature the
party sees standing that way (the view of the last batch: never what it
does not perceive) is not submitted: the loop offers to strike it, by
the preference whose bare name is `bump_attack`:

| `bump_attack` | Walking into someone |
|---------------|----------------------|
| `hostile` (the default, or no such preference) | One the character holds hostile or fears (`hostile`, `fears`, 09 §3.4) is struck at once with the control named `attack_bump`; anyone else is asked about first with the control named `attack_bump_ask` |
| `always` | Anyone is struck at once |
| `ask` | Anyone is asked about first |
| `never` | The step is submitted as any other, and the simulation refuses it when it is taken |

The controls are given `it`, a map of `target` (the creature as the
simulation's value for it), `where` (the direction walked) and
`hostile`. The `ui` mod's start the first blow that can be made at the
target (`view.actions_with('strike', it.target)`, section 9.3).

**Using what is near.** The control `interact`, on `e` in play, opens
`ui:interact`: a list of the property `view.offers_near` (16 §3.5.1),
each row an interaction a thing within a step offers, with where it is
and, greyed, why it cannot be done now (decision D-702). Choosing a row
starts the action `interact` (the `core` mod's) with its `target` and
`interaction`; the simulation tests reach and `if` against truth when
it starts and refuses with the interaction's reason, which goes to the
log. Hiding in a locker, getting out of it, opening, closing, locking
and unlocking are all done so.

## 4. Glyphs and frames

### 4.1 Glyphs in definitions

Anything drawable carries a `glyph`:

```json
{ "ch": "#", "fg": "material", "bg": null, "bold": false,
  "by_state": { "open": { "ch": "'" } },
  "by_level": [ { "ch": "." }, { "ch": "^", "fg": "orange" }, { "ch": "^", "fg": "red" } ] }
```

- `ch` is one Unicode character of single width.
- `fg` and `bg` are a colour name, `#rrggbb`, `"material"`, which takes
  the colour of the thing's material, `"own"`, which takes the colour this
  particular thing carries (06 §9.1), or `"dress"`, which takes the colour
  of what a creature is seen to be wearing.
- `by_state` and `by_level` vary the glyph by element state or by field or
  damage level. Each entry gives only the parts it changes (`ch`, `fg`,
  `bg`, `bold`). Entry 0 of `by_level` is level 0; a level past the end
  uses the last entry.
- A glyph must give `ch` or a `shape`. A colour the thing does not carry
  (`"own"` on something with no colour of its own) leaves what is beneath.
- A thing that fills several cells may give a `shape`: rows of characters,
  one per cell of its footprint, for a facing. Other facings are turned
  from it unless given. Without a shape every cell shows `ch`.

```json
"glyph": { "fg": "own", "shape": { "north": ["/^\\", "|#|", "o o"] } }
```

### 4.2 Frames

```rust
pub struct Frame {
    pub cols: u16, pub rows: u16,
    pub cells: Vec<FrameCell>,        // the grid: character, foreground, background, attributes
    pub sprites: Vec<Sprite>,         // glyphs not aligned to the grid
    pub fx: FrameFx,                  // shake, flash, zoom, letterbox bars, tint
}

pub struct FrameCell { pub ch: char, pub fg: Color, pub bg: Color, pub attrs: Attrs }

pub struct Sprite { pub ch: char, pub fg: Color, pub x: f32, pub y: f32,
                    pub scale: f32, pub alpha: f32, pub layer: u8 }

pub struct Color { pub rgb: Rgb, pub fallback: Ansi16 }
```

The grid is what a terminal can show. **Sprites** are how the window
frontend goes further: a character gliding between cells, fragments flying,
floating text, afterimages. The terminal frontend draws each sprite in the
nearest cell and ignores scale, alpha and `fx`. Nothing essential may be
conveyed only by something a terminal cannot draw.

Every colour in a frame carries both the exact colour and its
sixteen-colour fallback (section 4.3), so a frontend chooses by its colour
depth and looks nothing up. The attributes are bold, dim, italic,
underline and reverse. A sprite's `x` and `y` are in cells, and whole
numbers put it exactly on a cell; sprites are drawn lowest `layer` first,
and within a layer in the order given. A frame is reused from one
turn to the next and building one allocates nothing per cell.

### 4.3 Themes

```json
{ "type": "theme", "id": "default",
  "colors": { "text": "#c8c8c8", "dim": "#707070", "accent": "#e0b040", "blood": "#b02020" },
  "styles": { "title": { "fg": "accent", "bold": true }, "selected": { "fg": "#000000", "bg": "accent" } },
  "borders": { "h": "─", "v": "│", "tl": "┌", "tr": "┐", "bl": "└", "br": "┘" },
  "font": { "file": "assets/font.ttf", "size": 16 } }
```

Colour names used by glyphs and widgets resolve through the active theme.
The terminal frontend maps colours to what the terminal supports and
ignores `font`.

| Part | Is |
|------|----|
| `colors` | Colours by name. Each value is `#rrggbb` or the name of another colour. |
| `styles` | Styles by name. A style may set `fg`, `bg`, and turn `bold`, `dim`, `italic`, `underline` and `reverse` on or off. Whatever it leaves out stays as it was beneath. |
| `borders` | The six characters frames are drawn with. Any left out keeps the engine's line-drawing character. |
| `font` | The font of the window frontend: `file`, a font file inside the mod that writes it (decision D-844), and `size`, the height of a cell in pixels at the screen's ordinary scale (section 11). |
| `marks` | Glyphs the engine draws for itself, by name: those of the world view (section 6.5) and the parts of widgets (section 10.3). |

A colour name is looked up among the theme's `colors`, then the `color`
definitions, then the sixteen terminal colours, which always exist:
`black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, and
each with `bright_` in front. A style name is one of the theme's `styles`,
or else a colour name, which colours the characters.

The colours `text` and `background` are what an empty cell is drawn in.
The toolkit draws its own parts with styles of fixed names (section 10.3).
A theme that leaves any of these out gets a plain default, never an
error.

**Colours.** A colour is a definition:

```json
{ "type": "color", "id": "crimson", "rgb": "#a01828", "word": "red", "fallback": "red" }
```

`word` is how text names it, so that what a witness describes and what is
drawn agree (15 §8.4). `fallback` is one of the sixteen terminal colours,
used when the frontend cannot show `rgb`. A theme may override `rgb`, by
giving a colour of the same name in its `colors`; the definition's `word`
and `fallback` stay. A bare `#rrggbb` is mapped to the nearest of the
sixteen. `word` left out is the definition's id; `fallback` left out is
the nearest of the sixteen. With 256 colours the terminal frontend shows
the nearest entry of that palette to `rgb`.

A `color` definition has a qualified id like any other definition
(04 §4.1): `ui:crimson`. A colour name is written by many mods, in glyphs,
styles and inline tags, and is resolved by whichever theme is active, so
it cannot be read through the imports of one mod. A qualified name means
exactly that definition. A bare name means the `color` definition of that
name in the theme's own namespace, or else the first of that name in
order of qualified id. A theme, a keymap and a screen are asked for by
their qualified ids: `ui:default`.

Colour must not be the only thing that tells two things apart. The linter
warns when two states or levels of one glyph, or two entries of one
legend, differ in colour alone.

**Inline styles.** In any `Text`, `<name>…</>` applies a theme style or a
colour to what it encloses: `"<blood>You are bleeding.</>"`. `<<` is a
literal angle bracket. An unknown name is a load error, and so is a tag
that is never closed. Tags nest, and the inner style is laid over the
outer. A text is styled after its templates are filled in.

### 4.4 Looks

A trait, an ability's mode, a posture, a device's mode or something worn
may carry a **look**: how its holder appears for as long as it lasts.

```json
"look": { "fg": "orange", "anim": "flicker",
          "aura": { "ch": "*", "fg": "red", "radius": 1 },
          "label": "burning", "priority": 3, "seen_by": "sight" }
```

| Part | Does |
|------|------|
| `ch`, `fg`, `bg`, `bold` | Replace those parts of the holder's glyph |
| `anim` | An animation that repeats on the holder and follows it |
| `aura` | Glyphs drawn on the cells around it |
| `label` | A word shown beside it and in what `look` reports |
| `priority` | Which look wins where two set the same part. Animations and auras all play. |
| `seen_by` | The sense needed to see it. An aura visible only to those with the right sight. |

A look is applied when the event that gave it is played, to viewers who
perceive the holder, and removed the same way. Something meant to be
hidden has no look. A burning man, a held guard, a charged blade, a
transformed state and a shield of force are looks.

## 5. The view model

The view model is the scene's picture of the world: for each visible or
remembered cell, what is drawn there; for each perceived character and
object, where it is and how it looks.

- It is initialised from the controlled characters' map memory and current
  percepts (15 §13).
- It changes only when an event is **played**. Until a wall's destruction
  is played, the wall is still on screen, though it is long gone in the
  simulation.
- Events the party did not perceive are not applied. Events only heard are
  applied as heard: a marker and a log line, not a picture.
- When playback is caught up, the view model must equal what a fresh
  initialisation would produce. Debug builds check this: the
  application makes a fresh view model and hands it to
  `Scene::check_caught_up`, or to `clash_scene::playback::check_caught_up`
  with the played one. The check compares cells, sprites and the shapes
  of grids, not the markers of what was only heard, which are shown for a
  while and are not knowledge, and reports the first eight differences
  (decision D-608).

An omniscient mode for debugging builds the view model from truth.

**What it holds.** The view model is plain data owned by the scene. It
is not a copy of the world: it holds what has been put into it and
nothing else.

| For | It holds |
|-----|----------|
| Each cell ever seen | The floor, block and fixture as last seen, as the world's own states; the strongest field; the most marked decal; whether several items lie there; whether it is known to be open to the sky; how well lit it was, from 0 to 255; and whether it is seen now or only remembered |
| Each perceived character or object | A **sprite**: where it is, whether it is a creature or an object, its glyph for the state it is in, the colours it carries (own and dress), which way it faces, and whether it is only a suspected presence |
| What was only heard | A marker: where it seemed to come from, and optionally the character to mark it with |
| Each grid | Its shape, so that a view can draw across a wrapped edge and stop at the realm's |

A single item lying in a cell is a sprite. Where several lie, the cell
says so and the pile marker is drawn.

**How it is filled.** Only by being told, one cell or one sprite at a
time, so that playing the destruction of a wall changes one cell:

| Call | Means |
|------|-------|
| `observe_cell` | This cell is seen now, and this is in it |
| `observe_light` | The light on a known cell changed |
| `unsee_cell`, `unsee_box`, `unsee_all` | No longer in view; what was there is kept as remembered |
| `forget_cell`, `forget_grid`, `clear` | Map memory lost |
| `upsert_sprite`, `move_sprite`, `remove_sprite` | A perceived character or object appeared, changed, moved or is gone |
| `add_heard`, `remove_heard`, `clear_heard` | Something was only heard, and its marker has been shown long enough |
| `set_grid_shape` | Where a grid ends and whether it wraps |

The perception layer (15 §13) and playback make these calls. The view
model has no way to read the world. The omniscient mode is a separate
function that is given the world and fills a box of cells with what is
really there; it is the one place in the scene that reads the world, and
no screen, widget or binding leads to it. Two view models that hold the
same knowledge are equal, however they came by it, which is what the
check above compares.

## 6. Drawing the world

### 6.1 What a cell shows

The first of these that applies:

1. A perceived creature. A suspected but undetected presence shows as a
   marker.
2. Items lying there: the topmost, or a pile marker.
3. The fixture.
4. The block.
5. A field strong enough to have a glyph. Weaker fields tint the
   background.
6. The floor.
7. For an open cell, the cell below, dimmed; and so on down for
   `look_down` storeys (default 8); then the stratum's backdrop.

Then, in order: decals tint; light scales brightness, and with the
experimental coloured light (15 §3.3) tints toward the light's colour;
ambience (section 6.4) tints the whole view; cells remembered but not
currently seen are desaturated and dimmed. Subgrids are composited as
the world does (02 §18.1).

The list decides the character and its colour. The background is that of
the highest entry that gives one: a glyph whose `bg` is left out shows
what is beneath it, so a creature on a carpet stands on the carpet's
colour. An element whose glyph gives no `ch` is not drawn and the list
goes on past it.

A cell that was never seen shows nothing. Neither does a known open cell
over one that was never seen: what is down a hole nobody has looked into
is not known. If any cell looked through is only remembered, what shows
is drawn as remembered.

| Step | Rule |
|------|------|
| The cell below | Each storey down keeps 70% of the brightness of the one above |
| Decals | The character of a floor, block or fixture, or of what shows from below, moves toward the decal's colour by up to 70% of the way, in proportion to the decal's amount. A creature or an item is not tinted. |
| A field too weak for its glyph | The background moves toward the field's tint by up to half the way, in proportion to its level |
| Light | Brightness runs in a straight line from 25% at no light to the whole at full light: a cell that is seen is never black |
| Ambience | Section 6.4 |
| Remembered | Colour keeps a quarter of its saturation, the rest going to the grey of the same lightness, and 55% of its brightness |

Brightness is carried by the exact colour, which a terminal with sixteen
colours cannot show, so the fallback (section 4.2) changes as well: the
character of a remembered cell falls back to dark grey, and that of a
cell at less than half its brightness to the darker of its colour's
pair, a bright colour to its plain one and light grey to dark grey.
Backgrounds keep their fallback. No attribute is used for this.

A creature is drawn as the persona the viewer perceives.

**Through portals.** What a viewer sees through a see-through portal
(02 §22) is drawn beyond the portal, in the direction they are looking, in
place of what is physically there. One portal deep by default.

A capsized vessel (11 §10.6) is drawn with a mark from the theme.

**Vessels from a distance.** At cell scale a vessel is drawn as its grid
lies, at a quarter-turn (11 §5.5). In a view zoomed out far enough that a
vessel is an outline, the outline is drawn at its true heading.

### 6.2 Views

| Widget | Shows |
|--------|-------|
| `map_view` | One storey from above, as section 6.1 |
| `section_view` | A vertical slice along a line: one column per metre, one row per storey. For towers, caves, ships and submarines. |
| `region_map` | Tract-scale map: one glyph per tract from its definition, with devastation and known sites |
| `chart_view` | A chart (02 §20.3) and travelers on it |

A `map_view` can zoom out by whole factors, showing the most significant
glyph of each block of cells. The window frontend can also change glyph
size.

**The map view.** The camera's centre (section 6.3) is the middle cell
of the view, the one left of and above the middle where the size is
even; east is to the right and south is down. Zoomed out by a factor
`n`, from 1 to 16, each glyph stands for a block of `n` by `n` cells,
aligned to multiples of `n` so that the picture does not shimmer as the
centre moves. The factor is the camera's times the view's own `zoom`.
The block shows the most significant of its cells: significance is the
order of section 6.1, a creature first, with what shows from a storey
below under a floor, the backdrop under that and a cell not known at all
last. Of equals, the first row by row.

**The section view.** A vertical slice along a line through the camera's
centre: one column per metre along the line, one row per storey, higher
storeys above, the camera's storey on the middle row. The line runs west
to east with east to the right, or north to south with south to the
right; the camera says which. Each cell shows what section 6.1 gives
without looking down: a cell holding nothing is air and blank. A floor
is seen edge on and is drawn with the theme mark `section_floor` in the
floor's own colours. A section is not zoomed.

A marker for something only heard is drawn in a map view on the viewed
storey, and in a section view where it lies on the line. It keeps the
background of its cell. The cell under a free cursor is drawn in the
theme style `cursor`.

A `map_view` may draw an **overlay** on top of the cells:

| Overlay | Draws |
|---------|-------|
| `networks` | The conduits of one resource, coloured by network, with each network's supply |
| `areas` | Areas by kind: storage, rooms, whatever content defines |
| `jobs` | Planned work as ghosts, with its state |
| `temperature`, `light` | The value at each cell, as shading |

In an ordinary screen an overlay is drawn from the view model, so it shows
what the character has perceived. In the debug context it shows the
truth. Mods add overlays as definitions:

```json
{ "type": "overlay", "id": "danger", "name": "Known hazards",
  "cell": "known_field(it, 'fire') > 0", "shade": "red", "legend": "Fire" }
```

| Field | Is |
|-------|----|
| `name` | What the overlay is called where the player chooses one |
| `cell` | An expression over a cell, as `it`: a condition, or a number from 0 to 1 for how strongly the cell is shaded. Required. |
| `shade` | The colour cells are shaded with: a colour name or `#rrggbb`. Required. |
| `legend` | What the shading means |

A view's `overlay` field names an overlay by its qualified id, or one of
the engine's own by its bare name. An overlay shades the background of
each known cell and leaves its character: `light` from black at no light
to the theme colour `overlay_light` (yellow unless the theme gives one)
at full light; a definition's from black to its `shade` by the value of
its `cell`. The expression is given the cell as the view model holds it
and nothing else.

The view model holds, for the party: each cell they have seen and what
was there; for each tract, its glyph, devastation and the sites they know
of; and the travelers they have perceived on a chart. Overlays, the
region map and the chart draw from that and nothing else.

### 6.3 Camera

The camera follows the active controlled character, or a free cursor in
look and targeting modes. The viewed storey follows the character and can
be shifted up and down. Cutscenes move the camera (section 8).

The cursor moves on the viewed storey. A shift of the storey stays as
the character climbs or descends, and is dropped, with the cursor, when
the character moves to another grid.

Screens and controls reach the camera with the `camera` effect
(section 10.5): it moves the cursor, shifts the storey, zooms, puts the
cursor on the next thing that could be chosen, and gives the view back
to the character. While something is being aimed (section 9.3) a map
view draws the line from the followed character to the cursor with the
theme mark `line`, on the cells between the two that show nothing but
ground: a wall or a creature on the line stays as it is drawn.

**What a screen is told of the place under the cursor.** A screen that
declares any of these variables has them kept up to date while it is
on top. Look mode and the map pickers show them.

| Variable | Holds |
|----------|-------|
| `cursor_sight` | What the view model knows of the cell: `seen`, `remembered` or `unknown` |
| `cursor_distance` | How far the cursor is from the followed character, in metres |
| `cursor_target` | What the reader calls the perceived thing there (`name`, 05 §4.6); empty if there is none |
| `cursor_things` | What the reader calls every perceived thing in the cell, the topmost first, written out as a list (`Jonas and a knife`); empty if there is none |
| `cursor_ground` | What the view model holds of the cell: its fixture, block and floor as last seen, top first, joined by semicolons, each by its definition's id without the namespace and its state's name after a comma (`door, open; boards`), with `several things` first where items lie in a pile; empty for a cell not known |
| `cursor_reach` | While a parameter is asked for: `yes` if the action's `requires` (03 §6.1) holds with the place as that parameter, as the actor believes; `no` if it does not; empty where there is nothing to say |
| `cursor_why` | With `cursor_reach` at `no`: the action's `fail_text` |
| `cursor_advice` | What the parameter's `check` (03 §6.3) says of the place; empty if it is sound |
| `cursor_index`, `cursor_count` | Which of the things that could be chosen the cursor is on, from 1, or 0; and how many there are |
| `picker_entries` | For a list parameter: how many entries it has so far |

All of it comes from the view model and from expressions evaluated in
the actor's mind (05 §7.3). Nothing here reads the world.

### 6.4 Ambience

```json
{ "type": "ambience", "id": "heavy_rain", "tint": "#8090a0", "dim": 0.2,
  "particles": { "ch": ["|", "'"], "density": 0.08, "dir": "wind", "speed": 18 } }
```

An **ambience** is how a condition looks: a tint, a dimming, and particles
that fall or drift. It is named by a weather state (09 §14.1), a medium
(11 §4: under water), a stratum's sky by time of day, or an area kind. The
scene draws the ambience of where the viewed character is. Weather
particles are drawn only on cells the character knows to be open to the
sky.

Ambience is presentation only. What rain and dusk do to perception is
already in the simulation, as light and signals (15). On a terminal it is
the tint and the dimming.

| Field | Is |
|-------|----|
| `tint` | A colour name or `#rrggbb`. The tint is a coloured filter: each channel of a cell's colours is multiplied by the tint's channel over 255, so black stays black and white becomes the tint. |
| `dim` | The share of brightness taken away after the tint, from 0 to 1 |
| `particles` | `ch`: the characters a particle may be drawn as, at least one. `density`: the share of cells holding a particle at any moment, from 0 to 1. `dir`: `wind`, `down`, or left out for no drift. `speed`: cells per second, zero or more. |

### 6.5 What the views draw with

The views draw their own parts with theme `marks` of fixed names. A
theme that leaves one out gets the default.

| Mark | Drawn | Default |
|------|-------|---------|
| `pile` | Where several items lie | `&` |
| `suspected` | A presence suspected and not detected | `?`, bold |
| `heard` | Something only heard, unless its marker brings a character | `!`, bold |
| `backdrop` | Where a view has looked down `look_down` storeys of open cells and found nothing. The stratum's backdrop, once `sky` definitions (02 §23) supply one. | A space |
| `section_floor` | A floor in a section view. Only its character is used. | `_` |
| `line` | A cell between the followed character and the cursor, while something is aimed | `*` |
| `marker` | A pin: where a marker of the journal is, as the character last perceived its target (`WorldView::set_pins`, decision D-796) | `X`, bold |

And with the theme style `cursor` (reverse unless given), the theme
style `target` for the cell of the creature selected as the target
(`WorldView::set_marked`; underlined and bold unless given, decision
D-793) and the theme colour `overlay_light` (yellow unless given).

## 7. Playback

The simulation is never waited on. It runs ahead and hands over what
happened in **batches**, a batch being one stretch between two
decisions; the scene plays the batch in order over real time, and until
an event is played the picture stays as it was. The application drives
it with a few calls on the scene (decisions D-600, D-603):

| Call | When |
|------|------|
| `push_events(batch)` | After each `advance` of the simulation, with what `drain_events` gave, in order. Each event is asked of the host here, once (section 7.5); nothing is shown yet. |
| `advance(seconds)` | Every frame, with the real time that passed: plays what falls due. |
| `playback_caught_up()` | Whether every event handed over has been played: the picture is of the present, and input applies to it. |
| `skip_playback()` | Jumps to the end: everything queued is played now and every effect in progress ends. |
| `check_caught_up(fresh)` | Once caught up, in debug builds: the check of section 5. |
| `playback_moment()` | The tick bindings are shown as of (section 10.4). |
| `take_sound_cues()` | The sound cues of what was played since last asked. |

### 7.1 Pacing

Events arrive with ticks. Playback maps simulation time to real time one
stretch at a time.

- Events the party did not perceive are dropped as they arrive (section
  7.5). A stretch with nothing perceivable in it is skipped.
- Events of one tick are played together, as one **group**.
- Between one group and the next, simulated time counts for at most two
  seconds: a long wait is not waited through.
- What is left is scaled so that the stretch plays in `stretch_time`
  (default 350 ms) from its first group that shows something to its
  last, however much simulation time it covers. One step by a slow
  character and forty by a fast one take the same real time. A group
  that only changes the picture, with no line, animation, glide, marker
  or hold, never makes a stretch longer: it is played with what shows
  around it. A stretch in which one group or none shows something is
  played at once.
- An event whose `show` has a `min_time` holds the clock for at least
  that long: nothing later is played before it has passed, in its own
  stretch or the next.
- The preference `playback_speed` scales real time: 2 is twice as fast,
  and 0 is instant, which plays everything queued at once and starts no
  effect.
- However far the simulation has run ahead, playback catches up: when
  more is queued than one second of real time would play, it runs
  faster in proportion.
- The key the keymap in force names `skip` jumps playback to its end and
  does nothing else. While no key is named so, any key pressed while
  something is being played does it, and then does what it does.
- A decision's prompt (section 9.5) opens once playback has caught up.

### 7.2 Motion

A `sprite` change (section 7.3) glides the sprite from the cell it was
drawn in to its new one over its show's `time`: it is drawn between
cells, as a frame sprite that a terminal shows in the nearest cell, and
not in either cell. A sprite glides one move at a time; a new move
begins where the last led. Something that crosses two cells or more in
one move leaves three fading afterimages behind it, which is how a much
faster character reads on screen as fast instead of as teleporting. The
camera may be told to follow one sprite as playback moves it.

Relocation events do not yet carry how long the move took, so a glide
lasts its show's `time` (decision D-606). Bodies in flight (02 §15)
report where they start, what they strike and where they come to rest,
and the bundled shows draw them there; no event carries the path between,
so they are not drawn along it.

### 7.3 Showing events

```json
{ "type": "show", "id": "glass_breaks",
  "on": "element_destroyed", "filter": { "layer": "fixture", "how": "damage" },
  "change": [ { "cell": "here" } ],
  "anim": "shatter", "at": "here",
  "log": "{the(actor)} {verb(actor, 'break')} the glass.",
  "heard": { "log": "Glass breaks somewhere close.", "marker": "!" },
  "shake": 0.1, "min_time": "120 ms", "importance": "normal", "sound": "glass_small" }
```

A `show` definition says how an event appears when it is played: the
changes it makes to the picture, an animation, a log line, screen shake,
a flash and what to do if the event was only heard. It may name a
`sound` cue. No frontend plays sound in the first version; the name is
carried so that content can be written ahead of one that does. Several
may match one event; all apply, in order of qualified id, and a change
two of them make is made once. Log lines are never dropped.

| Field | Is |
|-------|----|
| `on` | The event, by its registered name. Required. A name no event has is a load error, with the nearest suggested. |
| `filter` | Constants the event's plain fields must hold, as a rule's (05 §8.4). A field the event does not have is a load error. |
| `if` | A condition. The show applies only while it holds. |
| `change` | What the event changes in the picture when it was seen: a list of the changes below. Left out, every cell and every thing the event is about is looked at again; an empty list changes nothing. |
| `anim` | The `anim` played, by id. One that does not exist is a load error. |
| `at` | The subject the animation plays at: a cell, or the cell a thing's sprite is in when it begins. `here` if left out; an event that binds no `here` must say. |
| `to` | The subject a travelling animation goes to. Required with one, and of no use without. |
| `log` | The line written to the log. |
| `category` | The category of its lines, which a `log` widget filters by. `event` if left out. |
| `heard` | What to do if the event was only heard: `log`, a line, and `marker`, the character of the marker put where it seemed to come from (the theme mark `heard` if left out). Without it, an event only heard shows nothing. |
| `shake` | Screen shake as the event is played, from 0 to 1, dying away over `time`. |
| `flash` | A colour, by name or `#rrggbb`, washed over the whole frame and dying away over `time`. A colour the theme does not know is a load error. |
| `time` | How long the show's own effects last: the glide of a sprite it moves, its shake and its flash. 150 ms if left out. |
| `min_time` | How long the event holds the clock (section 7.1). |
| `importance` | `minor`, `normal` (the default) or `major`. A `log` widget may show only lines of some importance, and the preference `effects` (`all`, `major` or `none`) plays the animations, glides, shake and flash of all shows, of `major` ones only, or of none. |
| `sound` | A sound cue's name: letters, digits, `_`, `-`, `.` and `:`. Handed to the application with the event; nothing plays it. |

The texts and the condition are expressions in the event's scope: its
subjects and fields by name as its registration lists them, and `actor`
and `instigator`. They are pure, and compiled as a mind context with
nobody's own (05 §7.3): they read only what is safe for belief, because
a line speaks from what the viewing character knows. The reader helpers
of 05 §4.6 are answered by the host for the viewing character. A
finished line gets a capital first letter; its tags are theme styles,
and one a theme does not know is a load error.

**Changes.** Each is an object with one key, whose value names a
subject of the event; a name the event does not bind is a load error.

| Change | Does |
|--------|------|
| `{ "cell": "here" }` | The cell is looked at again: what the party sees there replaces what was known of it. A cell out of view is left as it is. |
| `{ "field": "here" }` | Only the strongest field of a known cell is taken from what the party sees there. |
| `{ "sprite": "self" }`, `{ "sprite": { "of": "self", "to": "here" } }` | The thing moved, appeared or changed: its sprite takes the look the party perceives and the cell `to` names, or where it is perceived if `to` is left out or not bound, and glides there. A thing the party never perceived is left alone. |
| `{ "gone": "self" }` | The thing's sprite is removed. |

An event with no `show` changes the picture silently: every cell and
thing it is about is looked at again, and if nothing seen changed it is
dropped. A thing looked at so takes the look the party perceives and
stays where it is drawn: only an event that says so moves it, and the
last event of a batch about it leaves it where it is perceived. An
action starting before the step it takes does not move the actor ahead
of the step (decision D-655).

**Effects.** What a show draws is drawn over the picture and is never
part of it (decision D-606):

| Effect | Comes from | Is drawn as |
|--------|------------|-------------|
| Glide | A `sprite` change | A frame sprite between cells, with afterimages (section 7.2) |
| Animation at a place | `anim`, `at` | Its keyframes over the cell, which keeps its background |
| Projectile | `anim` that travels, `at`, `to` | A frame sprite along the straight line from `at` to `to`, over the animation's time |
| Shake | `shake` | The frame's shake |
| Flash | `flash` | The frame's flash, from 0.6 of full strength down to nothing |
| Heard marker | `heard` | The marker of section 5, kept for one and a half seconds of real time |

```json
{ "type": "anim", "id": "shatter", "time": "300 ms",
  "frames": [ { "at": 0.0, "ch": "*", "fg": "white" }, { "at": 0.5, "ch": "%" } ] }
```

An `anim` is a short keyframed effect at a place, or along the way from
one place to another:

| Field | Is |
|-------|----|
| `time` | How long it lasts, above zero. Required. |
| `frames` | Its keyframes, in order, at least one. Each has `at`, when it begins as a share of `time` from 0 to 1 (0 if left out), and the parts of the glyph it changes: `ch`, one character of single width; `fg`, a colour name or `#rrggbb`; `bold`. What a frame leaves out it keeps from the one before; the first must give a `ch`. Required. |
| `travel` | Whether the glyph travels from the show's `at` to its `to` over the time. |

Not built: floating text (a show's `text`), an animation's `particles`,
larger effects that take parameters from the event such as a radius or a
direction, and dropping animations that would overrun the stretch; the
preference `effects` is what thins them for now (decision D-601).

### 7.4 The log

Lines from `show` definitions, and from `message` effects (05 §10.6)
once the simulation reports them, go to the message log with their tick,
a category and an importance (decision D-604).

- A line the same as the one before it, in text, styles, category and
  importance, is not added again: the line before counts one more and
  takes the newer tick, and is shown with the count after it, dimmed:
  `Debris crashes down. (x3)`.
- The log keeps its newest 2000 lines.
- The `log` widget wraps lines at spaces to its width, a column less
  when a scrollbar is drawn; shows only the categories its `filter`
  names and the importance its `importance` asks for at least; and
  follows the newest line unless it has been scrolled back, when it
  stays on the lines it shows as new ones arrive.

The log is complete: anything shown by animation alone also has a line,
so the game is playable by reading. A step, which the bundled shows
draw as a glide, is not a line.

### 7.5 What the party perceived

Playback applies only what the party perceived, in the manner it was
perceived (section 5). It does not decide that: the host does. The
game's host answers from the simulation's perception (15 §13; section
3.5, decision D-700); tests over the scene alone use a stand-in in
`clash_scene::testing` (decision D-603).

| The host is asked | When | It answers |
|-------------------|------|------------|
| `perceived(event, picture)` | For each event as its batch arrives, with the view model as playback has it | `Seen`, `Heard { from }` with where it seemed to come from, or `Not` |
| `seen_cell(at)` | For a cell an event that was seen changes | What the party sees there now, or nothing if it is out of view |
| `seen_sprite(entity)` | For a thing an event that was seen is about | The number of its sprite and how the party perceives it now, or that it does not; nothing for a thing never perceived |
| `as_of(moment)` | Before bindings are evaluated | Nothing: it is told the moment of section 10.4 |
| `reader()` | For each line and condition | The reader helpers and what is safe for belief, for the viewing character |

- **Seen**: the changes, the effects and the line.
- **Heard**: a marker where it seemed to come from and the `heard` line,
  and no change to any cell or sprite.
- **Not**: dropped.

Everything playing an event needs is asked when its batch arrives, and
playing it asks nobody anything. The answers are as of the end of the
batch, so a cell that changed twice in one batch is drawn in its last
state both times, and the last change a batch makes to a sprite leaves
it as the party perceives it then: one no longer perceived leaves the
picture at the last event that moves it.

## 8. Cutscenes

### 8.1 Cues

A cutscene begins with a **cue**, raised one of three ways:

| Source | For |
|--------|-----|
| An action names a `cutscene` | Takedowns, executions, signature moves. The cue fires when the action starts and carries its participants. |
| A `cutscene` definition has `on` and `if` | Moments that emerge: a clash above some energy, a tower falling, a death. The condition is evaluated in the simulation when the event occurs, like a rule whose only effect is the cue. |
| The `play_cutscene` effect | Anything else. |

A cue is an output event. It changes nothing in the simulation. It
carries how the party perceived the event that raised it (15 §13), and a
cutscene plays only for what was seen. A shot whose subject the party
does not perceive, or who is gone, is skipped.

### 8.2 Choreography

```json
{ "type": "cutscene", "id": "great_clash",
  "on": "clash_resolved", "if": "event.energy > 1e6",
  "subjects": { "a": "event.a", "b": "event.b" },
  "cooldown": "20 s", "tier": "major",
  "shots": [
    { "time": "400 ms", "camera": { "focus": ["a", "b"], "zoom": 2 }, "pace": 0.1, "bars": true },
    { "time": "250 ms", "release": ["energy_released"],
      "anim": [ { "play": "shock_ring", "at": "between(a, b)" } ],
      "shake": 0.6, "flash": "white",
      "caption": "{cap(name(a))} and {name(b)} meet." },
    { "time": "500 ms", "camera": { "zoom": 1 }, "pace": 1 }
  ] }
```

A cutscene is a list of **shots**. Each may set:

| Field | Does |
|-------|------|
| `time` | Real duration of the shot |
| `camera` | Focus on subjects or a place; zoom |
| `pace` | Playback speed for events during the shot: slow motion or a freeze |
| `release` | Which of the pending events are played in this shot |
| `anim`, `shake`, `flash`, `bars` | Effects |
| `caption` | Text, with expressions over the cue's subjects |
| `hold` | Wait for a key |

The events a cutscene covers have already happened. A cutscene chooses when
within its span each one is shown and how. Events it does not name are
played at the end, in order. It cannot change, drop or reorder what
happened in simulation time.

### 8.3 Player control

Cutscenes are always skippable. A preference sets how often they play: all,
only those of tier `major`, or none. `cooldown` stops the same one
repeating. On a terminal a cutscene degrades to its captions and its
animations at cell resolution.

## 9. Input

### 9.1 Events

Frontends deliver keys with modifiers, text, mouse movement, buttons and
wheel with the cell under the pointer, resize and close.

A key press is always delivered as a key, including one that types a
character. Text is delivered only for what did not arrive as key presses:
a paste, or the result of an input method. The terminal frontend
delivers Control-C as close, because in a terminal it would otherwise
interrupt the program and a player must always be able to leave. The
window frontend delivers Control-C as a key like any other, and
closing the window as close (decision D-842).

In data a key is written as its character (`"h"`, `"?"`) or one of
`enter`, `escape`, `backspace`, `tab`, `space`, `up`, `down`, `left`,
`right`, `home`, `end`, `page_up`, `page_down`, `insert`, `delete` and
`f1` to `f12`, with any of `shift+`, `ctrl+` and `alt+` in front. A letter
is written in lower case and `shift+a` is the capital. For any other
character the shift key is not named.

### 9.2 Keymaps and controls

Input is resolved in two steps.

A **keymap** names what a key means in a context:

```json
{ "type": "keymap", "id": "default",
  "play": { "h": "west", "a": "attack", "i": "inventory", "shift+a": "attack_precise" },
  "menu": { "up": "prev", "down": "next", "enter": "accept", "escape": "back" } }
```

Players rebind keys; the names stay.

A keymap has one field for each **context**, by the context's name:
an object from keys (section 9.1) to names. A context is whatever a
screen names as its `context` (section 10.1); the keymap need not list
one, and a key a context does not name is looked up in `menu`. The
engine gives two names a meaning: `menu`, the fallback, whose names the
toolkit navigates by (section 10.3), and `debug`, which also lets a
screen see truth. The `ui` mod uses three more:

| Context | Used by | Its names are |
|---------|---------|---------------|
| `menu` | Every screen that names no other | The toolkit's, and `done`, which ends a list in a picker |
| `play` | The play screen | Controls |
| `targeting` | Look mode, the map and the pickers that move a cursor or take a direction | The ten directions (`north` to `north_west`, `up`, `down`), `next_target`, `prev_target`, `zoom_in` and `zoom_out`, which those screens bind in their `keys` |
| `abilities` | The abilities screen | `bind_1` to `bind_9`, which put the use selected on a quick slot |

A **control** says what a name does:

```json
[
  { "type": "control", "id": "west", "name": "Walk west",
    "do": { "act": { "action": "step", "params": { "dir": "'west'" } } } },
  { "type": "control", "id": "attack",
    "do": { "act": { "action": { "pick": "view.attacks" }, "params": "needed" } } },
  { "type": "control", "id": "attack_precise",
    "do": { "act": { "action": { "pick": "view.attacks" }, "params": "all" } } },
  { "type": "control", "id": "inventory", "do": { "open": "inventory" } }
]
```

| Field | Is |
|-------|----|
| `name` | What the control is called where the player binds keys. Optional. |
| `doc` | What it does, in a sentence. Optional. |
| `do` | One UI effect (section 10.5), or a list of them. Required. They run in the screen that has the input, as a `ui_part`'s do: they see `view`, `party`, `app` and that screen's variables, unchecked, and are held to what an ordinary screen may know. Where a click on the map runs the control, `it` is the place picked (section 9.4). |

Because actions are content, so are controls. The engine has no built-in
"move" or "attack" key.

**From a key to a control.** A key press is named by the keymap in the
context of the screen on top. The name is offered to the widget with
the focus, to the hotkeys of menus and to the screen's own `keys`
(section 10.3). If none of them uses it and a control has that name,
the control's `do` runs. Only then does the toolkit act on the name
itself. So a screen's `keys` override a control of the same name, which
is how the direction keys move a cursor in a picker and walk in play.

A name in a keymap is a control's qualified id, or a bare name, which
means the control of that name in the keymap's own namespace, or else
the first of that name in order of qualified id. A mod binds a key to
its own control by patching the keymap.

**Rebinding.** The `rebind` service (section 10.5) gives a key a name in
a context of the keymap in force; an empty name takes the key's meaning
away. It takes effect at once and changes nothing but the keymap: no
control is touched. The scene then hands the service to the
application, which keeps the player's bindings between runs and gives
them back when it starts.

A control's `act` names its action as an `open` names a screen: by
qualified id, or by a bare name, which means the action of that name in
the namespace of the screen that has the input, or else the first of
that name in order of qualified id. The `ui` mod defines no actions, so
its walking controls name `step` bare and its `click` control names
`go_to`: whichever loaded mod defines one supplies it. An action that
no mod defines is not a load error. The command is handed to the
simulation as it was written, and the simulation refuses it with its
reason.

### 9.3 Collecting parameters

`act` builds an `Act` command (03 §4.4) for the active character.

| `params` | Behaviour |
|----------|-----------|
| `"needed"` | Ask only for parameters that have no default. One key press, and the action's defaults choose target, limb and spot. |
| `"all"` | Ask for every parameter, each pre-filled with its default. Full control. |

This is how decision D-037 reaches the player: the same action bound twice,
quick and precise.

Each parameter type has a **picker**: a cell, an entity, a direction, a
body part, an item, a number, an entry from a query, a route. Pickers are
screens and can be restyled or replaced.

**How the command is put together.** The scene knows no action by name.
It reads the action's parameter list from its definition (03 §6.3) and
goes through it in the order written: the roles of a joint action after
the first, which are given as parameters are (03 §8), and then the
parameters. For each, the default is worked out, in the mind of the
acting character and through the simulation, with the values settled
before it as the action's locals. With `"needed"` a parameter that has
a default takes it and is not asked for; with `"all"` every parameter
is asked for, its picker filled in with the default. When every
parameter is settled the scene submits one command: the action's
qualified id and a value for every parameter that has one. Nothing is
submitted before that.

A default that comes to null where the scene works it out (one that
draws at random does: the simulation's streams are not the scene's to
draw from) is left out of the command, and the simulation works it out
when the action starts.

Closing a picker any other way than by answering abandons the whole
command: nothing is submitted, and the effects that followed the `act`
in its list do not run. They run once the command has been accepted.

Where the action is written `{ "pick": "…" }`, an expression giving a
list of actions (ids, or maps with `value` and `label`), the list picker
asks which before anything else. Where it is written `{ "first": "…" }`
the first entry that can be done is taken without asking. An entry that
is a map may say more of itself (decision D-792): `available`, false
for one that cannot be done now, which the list picker shows greyed
with `why` and which is refused with that reason when chosen (`first`
passes it over, and is refused with the first entry's `why` when none
can be done); `ability`, the ability whose grant starts the action, so
that the command is `use_ability` with the ability, the action and the
parameters asked for; and `given`, the names of the parameters the grant
supplies, which are neither asked for nor sent. The list picker is
opened with every field of each entry beside `label`, `value` and
`chosen`.

**Values given.** The effect's `with` gives values for parameters by
name, worked out where the effect runs, before anything is asked: with
`"needed"` a parameter given a value is settled and not asked for; with
`"all"` its picker is filled in with it. A value that comes to null is
as if it were not given, and a name the action has no parameter or role
of is passed over. So one control can strike the target selected in
play, or ask for one when none is.

**What a character could do** (decision D-791). `view.actions_with(tag,
target?)` lists the actions of the loaded mods that carry a tag, as the
character may know of them: each a map with `value` (the action's id),
`label` and `name` (what a reader calls it), `id` (its qualified id),
`available` and `why`. Whether one could start is what can be said
before it is started, changing nothing (`clash_sim::offer::could_start`):
the character is in the world and its life lets it act; the
parameters not given take their defaults in its mind, without the random
streams; its `requires`, with its `fail_text`, once every parameter
without a default is given (`target` fills one of that name); every
attack's `with` finds its surface on the character, a part that answers
or something in hand with the tag (07 §3.1), and else says what the blow
needs; and the character has the slots of each channel it uses.
Footing, slots other actions hold and hooks are the start's to say. Those
that could start come first, then those that strike with something held,
then those that take fewer slots, then by qualified id: the quick blow
is the lightest that can be made.

**Which picker.** The `act` effect's `pick` names the picker for a
parameter. Without it the parameter's type does:

| Type | Picker | What it is |
|------|--------|------------|
| `entity` | `entity` | A cursor over the map that starts on the default, or else on the nearest thing that could be chosen, and cycles through the rest |
| `cell` | `cell` | A cursor over the map that starts on the default, or else on the character |
| A list of cells | `route`, or else `cell` | The same; each answer is one more step |
| Any other kind of subject (`item`, `part`) | The kind's name, or else `list` | A list of what could be chosen |
| `int`, `real`, `number` | `number` | A number to type or step; whole for an `int` |
| `string` | `text` | A line to type |
| A list of anything else | `multi` | A list in which entries are taken and put back |
| Anything else: `bool`, `id<T>`, `any` | `list` | A list of what could be chosen; for a `bool`, true and false |

A number or a text that the effect lists values for (`from`) is chosen
from the list. A direction is a text like any other, so an effect that
wants it asked for with one key says `"pick": { "dir": "direction" }`.
The picker named `x` is the screen of the role `picker.x`
(section 10.7); where the profile names none for a kind's own picker or
for `route`, the fallback of the table is used.

**What may be chosen.** The effect's `from` gives, for a parameter, an
expression whose list is what may be chosen. Without it the simulation
is asked what the acting character believes could be chosen
(`SceneHost::candidates`, with the values settled before it). Without
either, an entity picker offers what the view model holds on the viewed
grid, nearest first, and a list picker is empty. Nothing is offered that
the party does not perceive. The application answers for a parameter of
the kind `part` (07 §4.2, decision D-794): the parts on the outside of
the creature settled before it, while the character perceives it, each
by its name in the body, after a first row, `wherever it lands`, that
leaves the part to the body.

**What a picker is given and how it answers.** A picker is opened with
the parameters below, of which it declares those it has a use for. If
it declares a variable `value` that is set to the default too, and the
first list of a picker starts on the default's row.

| Parameter | Is |
|-----------|----|
| `action` | What the action is called: its `name`, or else its id |
| `param` | The parameter's name |
| `title` | The two together: `Strike: target` |
| `value` | What the parameter is filled in with, or null |
| `options` | What may be chosen: a list of maps with `label`, `value` and `chosen`, whether the value is in the list being built |
| `min`, `max` | For a list parameter: its least and greatest count |

It answers with the `answer` effect (section 10.5). An answer the
parameter cannot hold is refused with the reason and the picker stays:
choosing an empty cell for an entity is one. For a list parameter each
answer is one more entry and the picker stays until the list is full or
the `picker` effect ends it. A map picker's `map_view` answers from its
`on_pick`, where `it` is the place (section 9.4). The map pickers show
what section 6.3 tells them of the place under the cursor: whether it
is in sight, how far it is, and whether the action could be done there.

The **route** picker builds a list of cells one step at a time, drawing the
path as it grows and numbering the steps. After each step it evaluates the
parameter's `check` (03 §6.3) and shows what it says beside that step: too
far, too sharp, blocked. An unsound step is marked, and may still be kept.
Steps can be taken back. A list of any other type is picked one entry at a
time in the same way.

### 9.4 Mouse

In the window frontend: hovering shows the inspection tooltip for what is
under the pointer; a click on the map runs the `click` control, by default
"go there" or "open its interactions"; a second-button click opens a menu
of the interactions (02 §5.5) and actions that apply. All widgets respond
to clicks and the wheel. Everything remains reachable by keyboard.

A click on a `map_view` **picks** the cell under the pointer, unless a
free cursor is on show somewhere else: then the first click moves the
cursor there, and a click on the cursor picks. Picking runs the view's
`on_pick`, or, for a view with none, the control named `click`. A click
with the second button runs the control named `click_second`; the `ui`
mod's opens the screen of the role `context_menu` with the place, which
offers what can be done with it (section 10.7). `accept`,
while a map view with an `on_pick` has the focus, picks the cell under
the cursor. In all of them `it` is the place: a map with `cell`, the
cell as the simulation's value for it; `entity`, the perceived thing
drawn there, or null; and `x`, `y` and `z`. The wheel over a map view
zooms it. Hovering shows the `tip` of the widget under the pointer; the
inspection tooltip for what is under the pointer on the map comes with
look and inspect (section 10.7).

### 9.5 Decisions

When the simulation returns a decision (03 §5.2), the scene, once playback
reaches that tick, makes that entity active and opens the screen registered
for the decision kind: none for a free decision, the reaction prompt with
its options and a bar showing time remaining, or the choice screen for
`ask`.

| Kind | Role of its screen | The screen is given | The answer becomes |
|------|--------------------|---------------------|--------------------|
| Free | `decision.free`: the play screen, opened if it is not open | Nothing | Whatever the player does next |
| Reaction | `decision.reaction` | `who`; `what`, what is happening as the character perceives it (`The brute: Punch`); `options`, a list of maps with `label`, `value` (the option's number, from 0), `action` and `late` (it would not be ready before the window closes, 03 §7.3); `time_left` and `time_total`, in seconds of the character's own time | `React` with the chosen action and its parameters, or `Pass` |
| Choice | `decision.choice` | `who`; `prompt`; `options`, a list of maps with `label` and `value` | `Choose` with the option's number |

A prompt answers with the `answer` effect: the option's number. A
reaction prompt that answers null passes. The chosen reaction's
parameters are settled as an action's are with `"needed"`
(section 9.3), so a reaction that must be told something opens a picker
first. A prompt closed without an answer has not answered: the decision
is still waiting and the prompt is shown again, as it is when the
simulation refuses the answer. While a command is being put together no
prompt is opened.

The commands are handed to the simulation by name with their arguments:
`react` with `entity`, `window`, `action` and `params`; `pass` with
`entity` and `window`; `choose` with `entity`, `choice` and `option`.

### 9.6 Several at once

The player may select several directly controlled characters. A command
given then goes to each of them, in entity order, and each fills in its own
parameters from its own defaults. A decision for one of them can be
answered for all of them that have the same decision pending.

This is how a party is moved together, and how someone in several bodies
acts as one.

Not built yet: a command goes to the active character alone.

## 10. The UI toolkit

### 10.1 Screens

```json
{ "type": "screen", "id": "condition",
  "title": "{$who.name}",
  "context": "menu",
  "params": { "who": "entity" },
  "vars": { "tab": 0 },
  "min_size": [60, 20],
  "layout": { "cols": [
    { "width": 34, "frame": "Wounds",
      "list": { "rows": { "wounds": { "of": "$who" } },
                "item": "{it.part.name}: {it.kind.name}", "empty": "Unhurt." } },
    { "fill": 1, "rows": [
      { "bar": { "label": "Blood", "value": "$who.pool.frac('blood')", "style": "blood" } },
      { "figure": { "figure": "body", "shade": "part_shade($who, it)" } },
      { "slot": "condition.extra" } ] } ] },
  "keys": { "back": [ { "close": {} } ],
            "tab":  [ { "set": { "screen.tab": "(screen.tab + 1) % 3" } } ] } }
```

A screen has parameters, local variables, a layout tree, and key handlers.

| Field | Is |
|-------|----|
| `title` | The screen's title, a text |
| `context` | The keymap context (section 9.2) its keys are named in. Left out, `menu`. `debug` also lets it see truth and use the development widgets. A key the context does not name is looked up in `menu`. |
| `params` | The values it is opened with: each name and its type, written as the expression language writes types (05 §3), or `party_member` (section 10.4). Read as `$name`. A parameter no value is given for is null. |
| `vars` | Its own variables and their starting values: `true`, a number or a text. Read as `screen.name`. |
| `min_size` | The smallest grid it can be shown on, as `[columns, rows]` |
| `layout` | The layout tree: one node (section 10.2) |
| `keys` | For a key name, the UI effects run when that key is pressed and no widget used it |
| `popup` | The screen does not fill the frame: how large it is and where it sits (below) |

**The stack.** Screens are kept on a stack. Input goes to the top one.
A screen that is closed gives way to the one beneath, which is as it was
left: its variables, its focus, its selections.

**Popups.** A screen with a `popup` does not fill the frame. It is drawn
over the screens beneath it, which stay on show: the frame is drawn from
the lowest screen that still shows upward. A click outside a popup does
nothing.

```json
"popup": { "width": 44, "height": "fit", "place": "centre", "dim": true }
```

| Field | Is |
|-------|----|
| `width`, `height` | Cells, a percentage of the frame, `"fit"` for as large as its content, or `"fill"`. Left out, `"fit"`. Never more than the frame. |
| `place` | `centre`, `top`, `bottom`, `left`, `right`, `top_left`, `top_right`, `bottom_left` or `bottom_right`. Left out, `centre`. |
| `dim` | `true` to draw everything beneath it dimmed: each colour keeps 44% of its brightness, and on sixteen colours a character falls back to dark grey. Left out, `false`. |

### 10.2 Layout

| Container | Arranges |
|-----------|----------|
| `rows`, `cols` | Children one after another. Each child has `width` or `height` in cells, a percentage, `fill` with a weight, or `fit` to content. |
| `stack` | Children over one another |
| `grid` | Rows and columns |
| `tabs` | One child at a time |
| `scroll` | A child larger than its space |
| `frame` | A border and title around a child |
| `slot` | A named place where other mods add widgets (section 10.6) |

Layout is computed on the character grid. A screen states a minimum size
and is told the actual one.

**Nodes.** A layout node is a JSON object with exactly one key that says
what it is, a container above or a widget of section 10.3, whose value
holds that kind's own fields. Beside it the node may carry:

| Key | Is |
|-----|----|
| `id`, `style`, `if`, `tip` | The fields every widget has (section 10.3). For a widget they may be written inside the widget's own object instead; in both places is an error. |
| `width`, `height` | A number of cells; `"50%"` of the space its container has; `"fit"`, as large as its content; or `"fill"` or `{ "fill": weight }`, a share of what is left over |
| `fill` | Among `rows` or `cols`: a share by weight of the space left over along the container's direction |
| `fit` | Among `rows` or `cols`: `true` for as large as its content along the container's direction |
| `frame` | A border around the node: a text for a border with that title, or `true` for one without. The node's `width` and `height` include it. |

A node that says nothing about its size fits its content, unless it is of
a kind that is useful at any size, which fills: `list`, `table`, `tree`,
`inspect`, `log`, the views, `scroll`, `spacer`, the width of a `bar`, an
`input` and a `tooltip`. A container fills if anything in it does. Across
its container's direction a node takes all the space unless it gives a
size. Space left over is shared by weight, and cells lost to rounding go
to the first to fill. When there is too little room, later children get
what is left, then nothing.

| Container | Its value |
|-----------|-----------|
| `rows`, `cols`, `stack` | A list of nodes. A child of a `stack` with a `width` or `height` smaller than the stack is centred in it. |
| `grid` | `columns`: the width of each column, written as `width` is. `rows`: the height of each row; left out, a row fits its tallest child. `cells`: the nodes, row by row. |
| `tabs` | `pages`: a list of `{ "title", "content" }`, the content being one node. `selected`: the screen variable holding the shown page's number, from 0. A strip of titles is drawn above the page. |
| `scroll` | One node. It is laid out at its own size and shown through the scroll's space, with a scrollbar when it is taller. Moving the focus to something inside scrolls it into view. |
| `slot` | The slot's name. What is put into it (section 10.6) is laid out one below another; an empty slot takes no room. |

### 10.3 Widgets

| Widget | Is |
|--------|----|
| `text` | Text with templates, wrapping and inline styles |
| `list` | Rows from a query or list, with selection |
| `table` | Rows with columns and headers; sortable |
| `tree` | Collapsible hierarchy |
| `menu` | Options with hotkeys |
| `bar`, `gauge` | A value against a maximum |
| `input`, `number`, `toggle`, `choice` | Editing a value |
| `button` | Something to activate |
| `log` | The message log |
| `map_view`, `section_view`, `region_map`, `chart_view` | Section 6.2 |
| `figure` | A character-art drawing with regions coloured by expression: a body outline with each part shaded by its condition |
| `art` | Character art from a mod's asset file |
| `inspect` | A browsable list of a subject's properties |
| `tooltip`, `spacer` | As named |

The set is fixed. Widgets are composed and styled, not programmed.

| Widgets | Fields |
|---------|--------|
| All | `id`, `style`, `if` (shown when true), `tip` (a tooltip) |
| `text` | `value`; `wrap` (true unless given); `align`: `left`, `centre` or `right`. May be written as a bare string. |
| `art` | `value`: the asset file, relative to its mod, drawn as written without inline styles |
| `list`, `table`, `tree`, `menu` | `rows` (a query or a list), how each row is shown, `on_select` and `on_activate` (UI effects, with the row as `it`), `selected` (a screen variable to keep the choice in) |
| `list`, `tree`, `menu` | `item`: how a row from a query is shown, a text over `it`. `list` and `tree`: `empty`, shown when there are no rows. `tree`: `children`, an expression giving the rows beneath `it`, and `open`, `true` for its branches to start open (left out, they start closed). |
| `table` | `columns`: each with `title`, `value` (a text over `it`, for rows from a query), `width` and `align`; `empty` |
| `button` | `label`, `on_activate` |
| `input`, `number`, `toggle`, `choice` | `bind` (the screen variable or preference it edits), `on_change`, `label` |
| `input` | `max_len` |
| `number` | `min`, `max`, `step` (1 unless given). A whole number stays whole. |
| `choice` | `options`: a list, each a text or `{ "value", "label" }`, or an expression. The variable holds the chosen `value`. |
| `bar`, `gauge` | `value`, `max` (1 unless given), `label`. A `bar` draws the share filled, and its `style` colours the filled part; a `gauge` writes the two numbers. |
| `map_view` and the other views | `centre`, `zoom`, `overlay`, `on_pick` |
| `figure` | `figure` (a definition: rows of character art with named regions) and `shade`, an expression giving each region its colour, or an object from region name to colour |
| `inspect` | `of`: the subject. Or `rows` written out, each with `label`, `value` and `children`. `selected`, `on_select`, `on_activate` as for a list. |
| `log` | `filter`: a list of categories, or an expression; `importance`: the least a line must have to be shown, `minor`, `normal` or `major` (every line if left out). It wraps, folds and follows the newest line as section 7.4 says. |
| `tooltip` | `value`: shown when nothing with a `tip` has the focus or the pointer |
| `console`, `canvas`, `trace_view` | Debug context only: a command line, a paintable grid of cells, and a stream of trace records. The development screens of document 14 are built from these. |

Widgets write only to screen variables and preferences. Anything that
changes the game is a UI effect that submits a command. `bind` names a
screen variable as `screen.name` or just `name`, and a preference as
`pref.id`. A widget bound to a variable of the wrong kind, or to one the
screen does not declare, is a load error.

**Rows written out.** Where `rows` is a list, each row is a text, a list
of texts (a table row's cells), or an object:

| Field | Is |
|-------|----|
| `label` | What the row shows |
| `value` | In an `inspect`, the value beside the label |
| `cells` | In a `table`, the row's cells, one per column |
| `key` | In a `menu`, the option's hotkey, written as in a keymap |
| `children`, `open` | In a `tree` or `inspect`, the rows beneath, and whether they start shown |
| `tip` | A tooltip for the row |
| `on_activate` | UI effects run when this row is activated, before the widget's own |

`selected` holds the row's number from 0 in the order written, counted
through the whole of a tree, and unchanged by a table's sorting.

**Rows from an expression.** Where `rows` is a string it is an
expression giving a list, and each element is a row. A `list`, `tree`
or `menu` shows a row by its `item`, a text over the row as `it`, and a
`table` by each column's `value`; with neither, a row that is a map is
shown by its `label` and anything else as text. A tree's `children` is
an expression over `it` giving the rows beneath it, at most sixteen
deep. A list of more than 10,000 rows is cut there. `options` written
as a string is likewise an expression giving a list: each element is a
flag, a number or a text, which is both the value and what is shown, or
a map with `value` and `label`. A query written as an object (05 §11)
is kept and gives no rows until queries are built.

For the effects of a row (`on_select`, `on_activate`), `it` is the row:
its element where the rows come from an expression, its `label` where
they are written out. For a `figure`'s `shade` it is the region's name,
and for a view's `on_pick` the place picked.

**Typing a number.** A `number` with the focus takes the keys that type
one: digits, a minus sign first and, unless its variable holds a whole
number, one point. What is typed is shown in place of the value and
becomes the value, brought within `min` and `max`, on `accept` or as
soon as any other key is used; `erase` takes the last character back.

**A figure** is a definition:

```json
{ "type": "figure", "id": "body",
  "rows": [ " O ", "/|\\", "/ \\" ],
  "map":  [ " h ", "ata", "l l" ],
  "regions": { "h": "head", "t": "torso", "a": "arms", "l": "legs" } }
```

`rows` is the drawing. `map` has the same shape and says, by a character
that `regions` names, which region each cell belongs to; a space is none.

A `figure` widget names its figure by qualified id, or by a bare name,
which means the figure of that name in the screen's own namespace or
else the first of that name in order of qualified id. An `art` widget
names a file of the mod that defines the screen: every file under a
mod's `assets/` whose name ends in `.txt` is art. A screen is checked
against every loaded theme, because the player may choose any of them: a
style that one of them lacks is an error naming that theme.

**Focus and keys.** One widget has the focus. A key press is named by the
keymap in the screen's context and then offered, in order, to the widget
with the focus (a widget that edits text takes any key that types a
character before its name is considered); to the options of every `menu`
on show, by their hotkeys; to the screen's `keys`; to the controls
(section 9.2); and last to the toolkit itself. A `map_view` with the
focus and an `on_pick` takes `accept` as picking the place under the
cursor. The names the toolkit and its widgets understand:

| Name | Does |
|------|------|
| `prev`, `next` | Move the selection in a list, table, tree, menu or log; scroll a `scroll`. Unused, move the focus. |
| `left`, `right` | Close and open a branch of a tree; choose the column a table is sorted by; step a `number`; change a `choice`; switch `tabs`; move the cursor of an `input`. Unused, move the focus. |
| `accept` | Activate the selected row or the button; flip a `toggle`; open or close a branch |
| `toggle` | Flip a `toggle`; reverse a table's sorting; activate a button |
| `page_prev`, `page_next`, `first`, `last` | Move by a page or to an end. Unused, scroll the `scroll` around the focus. |
| `focus_next`, `focus_prev` | Move the focus to the next or previous widget that can take it |
| `erase`, `delete` | Remove the character before or at the cursor of an `input` |
| `back` | Nothing by itself. Screens bind it in `keys`. |

With no keymap, the keys a keyboard has for these mean them: the arrows,
Enter, Escape, Tab, Page Up and Down, Home and End, Space, Backspace and
Delete. A click gives a widget the focus and acts on it; the wheel
scrolls.

**What the toolkit draws with.** Its own parts use theme styles of fixed
names: `title`, `border`, `border_focus`, `selected`, `selected_idle`
(the selection of a widget without the focus), `header`, `dim`, `bar`,
`bar_empty`, `field`, `button`, `hotkey`, `tip` and `notice`. And theme
`marks` of fixed names, of which the character is used: `bar_full`,
`bar_empty`, `scroll_thumb`, `scroll_track`, `tree_open`, `tree_closed`,
`toggle_on`, `toggle_off`, `choice_prev`, `choice_next`, `sort_up`,
`sort_down`, `view_empty`. A theme that leaves one out gets a plain
default: bold, underline, reverse or dim, and a line-drawing or ASCII
character.

A screen states a `min_size`. On a smaller grid it is replaced by a plain
notice saying how large the window must be. The smallest grid the bundled
screens support is 80 by 24.

**Reloading.** When definitions are reloaded (04 §11) the screen stack is
rebuilt by screen id. Screen variables are kept where still declared. A
screen that no longer exists is closed, with a diagnostic. A cutscene in
progress is ended.

### 10.4 Binding

Every widget property that shows something is an expression in a **pure**
context (05 §7.3). The names available:

| Name | Type | Is |
|------|------|----|
| `view` | `entity` | The active controlled character. Null when there is none. |
| `party` | `list<entity>` | All directly controlled characters |
| `$name` | As `params` declares | Screen parameters |
| `screen.name` | `any` | Screen variables. A name the screen does not declare is a load error. |
| `it` | The element of the row's list | The current row in a list, table, tree or menu, and in the effects of a row (section 10.3) |
| `app` | `app` | Things outside the simulation: saves, profiles, mods, scenarios, preferences, the creation session |

Every bound property and every expression of a UI effect is compiled
when mods load, with these names and no others. A name that is not
there, a property that does not exist, a value of the wrong type, a
random function and a read the screen may not make (below) are load
errors, placed at the line and column inside the string. A text
property is a template (05 §4.6); a property that holds a number, a
condition or a list is an expression when it is written as a string or
as `{ "expr": "…" }`.

`entity` is the kind of subject the simulation registers under that
name (05 §5.1). Where none is registered, as in a tool that loads mods
without a simulation, `view` and the members of `party` are `any`, and
what is read of them is checked when it runs.

Bindings are re-evaluated before the next frame when the simulation has
advanced, when a screen variable or a preference has changed, when a
screen has been opened or closed and when the grid has changed size.
Only the screens on show are evaluated, and of them only what is on
show: a node whose `if` does not hold is passed over with everything
inside it, and so is every page of a `tabs` but the one shown. While
events are still being played back, bindings on the view show the world
as of the moment playback has reached, so a health bar does not drop
before the blow is seen: the scene tells the host that moment, the tick
before the next event to be played, or that there is none once playback
has caught up, before it evaluates bindings, and evaluates them again as
each event is played. A host that cannot answer as of a moment answers
as of the present (decision D-605). A binding that faults when it runs (05 §4.4,
§4.5) gives null: a number shows as 0, a condition as false, a list as
empty.

`app` is the same for every mod and is listed in full in the generated
reference. It is a kind of subject the scene registers, with these
properties, all safe for belief:

| `app.` | Type | Is |
|--------|------|----|
| `saves`, `profiles`, `mods`, `scenarios` | `list<app_entry>` | What there is of each: `scenarios` lists the game scenarios (section 3.4). |
| `setup` | `any` | The game being set up (section 3.4): null until a scenario with a choice is asked for |
| `profile`, `scenario` | `app_entry?` | The ones in use |
| `pref(id)` | `any` | A preference's value (below). `id` is the id of a `preference`, checked when mods load. |
| `creation` | `any` | The character being made (06 §10.3, section 3.4): null while no creation session is in progress, else the map `CreationSession::to_value` gives (`templates`; `choices`, each a map of `key`, `name`, `kind`, `depth`, `count`, `budget`, `spent`, `left`, `min`, `max`, `max_length`, `options` with `id`, `name`, `cost` and `chosen`, `parts`, `value`, `shown` and `by_default`; `complete`; `problem`) with `template`, the first template's qualified id, `made_from`, what it is called, `name`, `seed`, `finished`, whether `new_game` will make this character, and `editable`, whether the game lets the player make its choices (13 §4.2). |
| `templates` | `list<app_entry>` | The templates the game the character is made for offers (section 3.4, 13 §4.2): each by its qualified `id`, its `name` and its `description` |
| `defines(type)` | `bool` | Whether the loaded mods define anything of a definition type, by its JSON name (decision D-867) |
| `keys(context)` | `list<map>` | What the keymap in force binds in a context (section 9.2): one map for each name a key there is given, in order of what it is called, with `name`; `label`, the `name` of the control of that name, or else the name; `doc`, the control's `doc`, or empty; `keys`, every key given that name as a keymap writes it, joined by commas; and `shown`, the same as a person reads them (`Shift+J`, `Tab`; decision D-866) |
| `controls(context?)` | `list<map>` | Every control the loaded mods define, bound or not, in order of what it is called (decision D-797): one map for each with `id`, its qualified id; `label`, its `name`, or else its bare id; `doc`, or empty; `keys`, every key the keymap in force gives it in the context (left out, `play`), joined by commas, or empty; and `shown`, the same as a person reads them |
| `target` | `any` | The creature selected as the target in play (the `camera` effect's `target`), while the party perceives it; null otherwise. Read of it only what may be read of anyone the party perceives (decision D-793). |
| `talk` | `any` | What the character played could say to someone, as the service `talk` last gathered it (15 §11; decision D-799): a map with `to`, the one talked to; `options`, in the order the rules on the hook `talk_options` left them, each a map with `id`, `text`, `kind` and `lines` (the message kind and the table of lines it is said with, null where it says nothing) and `says`, whether it says something; and `why`, the reason nothing can be said, or empty. Null when nothing was gathered. |
| `size` | `list<int>` | The grid, as `[columns, rows]` |
| `can_save`, `can_load` | `bool` | Whether the scenario's saving policy allows it now |

**The creation session.** Where the application can say what the
character being made would have, it adds to `app.creation`
`attributes`: a list of maps with `name` and `value`, what the
templates and the options chosen give. The creation screen shows them
where it is given (section 10.7). The service `creation` (section
10.5) changes the session.

An `app_entry` has `id` (a save's file name, a qualified id), `name`,
`description` and `state` (a word: `enabled`, `disabled`, `broken`,
`current`), all texts. The application fills the lists so: `profiles`
from each `profiles/` beside a searched mods directory and inside each
loaded mod, `current` for the one in use and `broken` for a file that
cannot be read; `mods` from each directory of a searched mods directory
that holds a `mod.json`, `enabled` for one the profile loaded and
`disabled` for one it did not; `scenarios` in order of qualified id;
and `saves` newest first, `enabled` for a save made with the mods
loaded now, `changed` for one made with others and `broken` for one
that cannot be read (section 3.6).

**What a screen may know.** In ordinary screens, the party's own
characters are available in full. Everyone and everything else is available
only as the party perceives it: queries return perceived personas and
remembered places (15). Screens with context `debug` see truth.

In an ordinary screen a parameter of type `entity` that is not a party
member stands for what the party knows of it: its persona, its track
(15 §7.2), the features made out (15 §8.4) and whatever was appraised
(08 §7.5). Reading anything else of it is a load error. A parameter of
type `party_member` may be read in full.

An ordinary screen is a mind context (05 §7.3) in which the party is
the character whose mind it is. What may be read in full is `view`,
`party` and its members, a parameter of type `party_member`, and
whatever is reached from those through properties registered as
yielding something of the same owner, as a character's parts and what
it carries are. A row of such a list is the party's own too: in a list
whose `rows` is `party`, `it` may be read in full. Of everything else,
only what is registered as safe for belief may be read. A `ui_part`
(section 10.6) is held to this whatever screen it is placed in.

**Preferences.** A `preference` is a player's choice about display and
input, kept outside saves and invisible to rules:

```json
{ "type": "preference", "id": "damage_numbers", "kind": "flag", "default": false,
  "name": "Show damage numbers", "doc": "Float the energy of each hit over its target." }
```

| Field | Is |
|-------|----|
| `kind` | What it holds: `flag`, `number`, `text`, or `choice`, one of the texts in `options`. Required. |
| `default` | Its value until the player or a profile sets one. Required, and of the kind. |
| `options` | For a `choice`: the texts it may hold |
| `min`, `max` | For a `number`: the least and the greatest it may hold |
| `name` | What it is called where the player sets it |
| `doc` | What it does |

The built-in ones are defined the same way, in the `ui` mod: `theme`
and `keymap`, each the qualified id of a definition of that type,
`playback_speed`, a number, `effects` (all, major, none) and
`cutscenes` (all, major, none). A profile may set defaults in its
`preferences`, by id.

The scene keeps the preferences' values, by qualified id, and tells the
application of each change so that it can keep the choice between
runs. A value the preference cannot hold is refused, with the reason.
Playback reads `playback_speed` and `effects` by their bare names,
whichever mod defines them (section 7). The engine itself acts on two,
by their bare names, whichever mod defines them: changing `theme` restyles every open screen, each keeping
its variables, focus and selections, and changing `keymap` changes what
the keys mean. A theme or a keymap that cannot be used is refused and
the preference keeps its value. A widget's `bind` and a `set` effect
name a preference as `pref.id`, where a bare id means the preference of
that name in the screen's own namespace, or else the first of that name
in order of qualified id.

### 10.5 UI effects

Keys, clicks and widget events run lists of UI effects.

| Effect | Does |
|--------|------|
| `open`, `close`, `replace` | Push, pop or swap screens on the stack |
| `set` | Change screen variables |
| `focus` | Move focus |
| `pick` | Open a picker and store its result |
| `act`, `command` | Build and submit a simulation command |
| `run` | Run a rule marked `ui_callable`, through the `Ui` command so it is logged and replayable |
| `confirm` | Ask before continuing |
| `answer` | Close a screen opened by `confirm` or `pick`, giving its answer |
| `app` | Engine services: new game, save, load, delete save, quit, change a preference, rebind a key, select a profile, reload mods, edit the creation session. Saving and loading are offered only as the scenario's saving policy allows (13 §9.2). |
| `camera` | Move the cursor and the view of the world (section 6.3) |
| `picker` | Work on the list a picker is building (section 9.3) |

UI effects cannot change the simulation except by submitting commands.

A list of effects is a JSON list of objects, each with one key, the
effect's name. An effect that does not exist, or an argument it does
not take, is a load error where it is written. A **value** is `true`,
`false`, a number, or a string holding an expression, compiled with the
screen's bindings (section 10.4): a text is written `"'like this'"`.

| Effect | Written | Arguments |
|--------|---------|-----------|
| `open` | `"id"`, or an object | `screen`: the screen's id. `with`: by name, a value for each of its parameters, and for any of its variables (section 10.1) that should start with something else than it declares: a screen that edits something starts from what it edits. A variable is given only a value of its own kind, a flag, a number or a text. |
| `close` | `{}` | None. Closes the top screen. |
| `replace` | As `open` | As `open`. Closes the top screen and opens another in its place. |
| `set` | An object | For each key, `screen.name` or `pref.id`, the value it is given. A screen variable holds a flag, a number or a text. |
| `focus` | `"id"` | The `id` of the widget to focus |
| `confirm` | `"text"`, or an object | `text`: the question. `yes`, `no`: what the two answers are called. |
| `pick` | An object | `from`: an expression giving the list to choose from. `item`: how an entry is shown, a text over the entry as `it`. `into`: the screen variable or preference the choice is stored in. `title`. |
| `answer` | A value | The answer: for a `confirm`, true to go on; for a `pick`, the choice, or null for none |
| `act` | An object | `action`: the action's id; `{ "pick": "…" }`, an expression giving the actions to choose from; or `{ "first": "…" }`, the same list, of which the first that can be done is taken (section 9.3). `params`: a value for each parameter by name, or `"needed"` or `"all"` (section 9.3). With `"needed"` or `"all"`: `pick`, for a parameter by name, the picker that asks for it; `from`, for a parameter by name, an expression giving the values it may be chosen from; and `with`, for a parameter by name, a value it is given before anything is asked. |
| `command` | `"name"`, or an object | `name`: the command. `args`: its arguments by name. |
| `run` | `"rule"`, or an object | `rule`: the rule's id. `args`: its arguments by name. |
| `app` | `"service"`, or an object with one key, the service, holding its arguments | See below |
| `camera` | `"release"`, `"turn"`, or an object | `move`: `[east, south]`, the cells the free cursor moves by; it starts at the followed character. `storey`: how many storeys the view shifts up, or down if negative. `zoom`: how many factors it zooms out, or in if negative. `cycle`: `1` or `-1`, the cursor goes to the next or the previous thing that could be chosen: a picker's candidates, or else what the party perceives on the viewed storey, nearest first. `release`: `true` drops the cursor and the storey shift. `turn`: `true` turns the line a section view cuts along. `target`: `1` or `-1`, the next or the previous creature the party perceives on the followed character's storey, nearest first and never the party's own, is selected as the target (`app.target`); with none in sight it fails, saying so. `untarget`: `true` drops the target. All whole numbers, written outright. |
| `picker` | `"done"`, `"undo"`, or `{ "toggle": value }` | `done` ends the list a picker is building, and is refused while it has fewer than its least count. `undo` takes the last entry back. `toggle` adds a value, or takes it out if it is there. |

| Service | Arguments |
|---------|-----------|
| `new_game` | `scenario` (may be left out): the scenario's id |
| `save` | `name` (may be left out) |
| `load`, `delete_save` | `save`: the save's id |
| `quit`, `reload_mods` | None |
| `set_pref` | `pref`: the preference's id. `value`. |
| `rebind` | `context`, `key`, `name`: as a keymap writes them (section 9.2) |
| `select_profile` | `profile`: the profile's id |
| `creation` | `op`; and for the op `begin`, `template`: a template's qualified id, which begins a session over it, every choice at its default; `select`, `key`: a choice's `key`, and `value`: a list of option or part ids, a number or a text; `clear`, `key`, which puts a choice back to its default; `name` and `seed`, `value`: a text and a whole number, kept for the game; `finish`, which checks the character and keeps it for `new_game`, or is refused with the reason. A selection the choice does not take is refused, with the reason, and changes nothing. |
| `edit_creation` | `field`, `value`: the older form of `creation`'s `name` and `seed` |
| `talk` | `to` (may be left out): the one to talk to. Gathers what the character played could say to them into `app.talk`: the bundled application raises the hook `talk_options` on a copy of the simulation, so that asking changes nothing in the game. Someone the character does not perceive now gets no options and a `why`. Null or left out, what was gathered is forgotten. |

A list is run in order, in the screen that asked: its expressions read
that screen's variables and parameters, and `it` where the event has a
row (section 10.3). An effect that cannot be run, a screen that does
not exist, a value a variable cannot hold, a command the simulation
refuses, a service the application cannot carry out, is reported with
its reason and the list goes on. Once the screen that asked has closed,
what is left of its list runs with no screen: `set` and `focus` fail
and the rest still run.

`open` and `replace` name a screen by its qualified id, or by a bare
name, which means the screen of that name in the namespace of the
screen that asks, or else the first of that name in order of qualified
id.

`confirm` and `pick` open a screen and stop: the rest of the list waits
for its answer. `confirm` opens the screen of the role `confirm`
(section 10.7) with the parameters `text`, `yes` and `no`. `pick` opens
the screen of the role `picker.list` with `options`, a list of maps
each with `label`, the entry as `item` shows it, and `value`, the
entry, and with `title`. The opened screen answers with the `answer`
effect, which closes it. On an answer of true to a `confirm` the rest
of the list runs. On an answer that is not null to a `pick` the choice
is stored in `into`, if that is given and the choice is a flag, a
number or a text, and the rest of the list runs with the choice as
`it`. Any other answer, and closing the screen any other way, drops
what was waiting.

`act`, `command` and `run` each build a command, its name and its
arguments as values, and submit it. What the command is to the
simulation is the simulation's business: it takes it or refuses it,
with a reason. For `act` with `"needed"` or `"all"` the scene asks the
player for the parameters first and submits the whole command when it
has them (section 9.3); the rest of the list waits for that, as it does
for a `confirm`. Only where the scene has no definition of the action
to read is the command handed over as written, saying that its
parameters were to be asked for.

`answer` closes the screen it answers, except in a picker for a
parameter or a decision prompt, where the scene takes the answer and
decides: the picker of a list parameter stays open for the next entry,
and an answer that cannot be taken leaves the screen as it is.

The scene carries out `set_pref`, `rebind` and `quit` itself and then
passes every service on to the application.

**From the simulation.** A rule may ask for a screen:
`{ "show_screen": { "screen": "trade", "to": "self", "with": { … } } }`.
Like a cutscene's cue (section 8.1) it is an output event and changes
nothing in the simulation. The interface of whoever controls `to` opens
the screen with those values; with no `to`, the player's does. If nobody
controls `to`, nothing happens. This is how
an interaction opens a shop, a book, a terminal or a mod's own screen.

### 10.6 Extending screens

```json
{ "type": "ui_part", "id": "mana_bar", "into": "ui:hud.bars", "order": 20,
  "if": "view.has_trait('attuned')",
  "widget": { "bar": { "label": "Mana", "value": "view.pool.frac('mana')", "style": "mana" } } }
```

A screen places `slot`s where others may add. A `ui_part` puts a widget
into a slot. Every bundled screen and its slots are listed in the
generated reference. A part aimed at a slot that does not exist is a load
error, unless it says `"required": false`. Any layout node may carry an
`id`, so that a patch can change one node of a screen and leave the rest
(04 §6.1). A mod that adds a resource adds its bar without replacing the
whole display. A mod that wants a different display overrides the screen.

A slot is known by its qualified name: the namespace of the screen that
places it, a colon, and the name the screen gives it. The screen
`ui:hud` places `{ "slot": "hud.bars" }`, and that slot is
`ui:hud.bars`. A slot's name is free; by convention it starts with its
screen's.

| Field | Is |
|-------|----|
| `into` | The slot's qualified name. A bare name means a slot of the part's own mod. Required. |
| `order` | Where the part comes among the others in the slot: lower first, and parts of equal order by qualified id. Left out, 0. |
| `if` | The part is shown, and takes space, only while this holds |
| `required` | Left out, true: the slot must exist. With `false`, a part whose slot no screen places is simply not shown. |
| `widget` | One layout node (section 10.2): a widget, or a container of them. Required. |

The parts of a slot are laid out one below another. A part's bindings
are compiled by themselves, since a part may be placed in any screen:
they see `view`, `party`, `app` and the screen's variables as
`screen.name`, which are not checked against any screen and read as
null where the screen has none of that name, and they do not see the
screen's parameters. A part is always held to what an ordinary screen
may know (section 10.4).

### 10.7 The bundled screens

The `ui` mod provides: title, profile and mod list, new game and scenario
choice, character creation, the play screen (map, log, status, slots),
inventory, condition, abilities, look and inspect, the reaction prompt, the
choice prompt, the pickers, options and key binding, save and load, trade
(16 §3.6), fitting, make, build, work, device and know-how; the journal,
case file and codex (14 §13); a scenario's introduction and summary
(13 §3); the helm (a station's readouts, 11 §15); and the map, which
hosts the region map and the chart for plotting a course.

Which screen answers each need is set by the profile (04 §3.3), so a
profile without the `ui` mod supplies its own:

**Make** lists the recipes the character knows that could be done here:
by hand, and at each station within reach. Each shows what it takes, what
of that is at hand, and what it gives. One that cannot be started says
why (14 §5). Choosing one starts the work, or queues it at the station as
a job.

**Build** is a mode over the map. The player picks a building recipe or a
design (16 §10), places and turns it, and sees its ghost and a warning
wherever what is planned would not stand (02 §14). Confirming creates the
jobs. The same mode marks storage areas, encloses rooms (02 §8.3), sets a
site's roles, and cancels or dismantles.

**Work** lists a site's jobs with their state, who has claimed them and
what blocks them, and lets the player set priorities and each helper's
preference for kinds of work (09 §7).

**Device** is one screen for every device. It shows its mode, its
settings by kind (flags, numbers, choices, filters, links, logic tables),
its stores, how well each resource is supplied, what it is making and how
far along, and its structure if it has one (16 §4.5).

**Know-how** lists what the character knows, what each thing lets them
make, and what they could learn next (16 §8.5).

All five are built from definitions, so a mod's machines, recipes and
know-how appear in them with no screen written.

| Role | Opened when |
|------|-------------|
| `play`, `creation`, `summary` | A game is being played, a character made, a game ended (`title`, `outcome`, `text`; section 3.4) |
| `setup`, `intro` | A scenario with a choice is being set up (`scenario`); a game whose scenario has an introduction begins (`title`, `text`) |
| `decision.free`, `decision.reaction`, `decision.choice` | The simulation asks for a decision of that kind (section 9.5). `decision.free` is the play screen. |
| `picker.<name>` | A parameter is being chosen with that picker (section 9.3): `entity`, `cell`, `route`, `direction`, `number`, `text`, `list`, `multi`, or a kind of subject's name. `picker.list` is also what the `pick` effect opens (section 10.5). |
| `confirm` | The `confirm` effect asks |
| `tooltip`, `context_menu` | Something is hovered or clicked (section 9.4) |
| `error` | An `app` service failed: the screen is opened over whatever is on show with the reason as its parameter `text` (section 3.2). Left out, the reason goes to the log alone. |
| `notice` | A game was loaded from a save made with other mods: the screen is opened over the play screen with `title` and `text`, what differs (section 3.6). Left out, the log alone says it. |

A role that names a screen which does not exist is a load error. A
role the profile leaves out is found out when it is needed: the effect
that wanted its screen fails with the reason.

The bundled profile names the `ui` mod's screens: `ui:play` for `play`
and `decision.free`, `ui:create` for `creation`, `ui:setup`, `ui:intro`
and `ui:summary` for the roles of those names, `ui:reaction`, `ui:choice`, `ui:confirm`,
`ui:error`, `ui:notice`, `ui:pick_list`, `ui:pick_entity`, `ui:pick_cell`,
`ui:pick_direction`, `ui:pick_number`, `ui:pick_text`, `ui:pick_multi`
and `ui:context_menu`; its `boot_screen` is `ui:title`.

**The screens a game is started and left from** are in
`mods/ui/data/app.json`. `ui:title` offers a new game, Continue (the
newest save) and Load (`ui:load`) when there is a save, Options and
Quit; Esc quits. `ui:new_game`
takes a name and a seed into `app.creation` and lists `app.scenarios`;
F2 gives both to the creation session, names the game selected to it
(`creation` with `scenario`) and opens `ui:create`;
activating one starts that game, or opens `ui:setup` for a scenario
with a choice of place or options (section 3.4). `ui:options` sets the preferences
`theme`, `playback_speed` and, while the loaded mods define a cutscene
(`app.defines('cutscene')`, decision D-867), `cutscenes`, has the slot
`ui:options.prefs` for a mod's own (the `ui` mod puts `bump` there),
and gives a key a meaning: the
context, the key and the name, through the `rebind` service.
`ui:pause`, opened in play by the control `ui:pause` on Esc, resumes,
saves (`ui:save_game`) and loads (`ui:load`), each only while
`app.can_save` or `app.can_load` holds, opens the options, and leaves
to the title after asking: closing the play screen ends the game.
`ui:error` shows a reason and a button to close it.

**Saving and loading** are in `mods/ui/data/saving.json` (section 3.6):
`ui:save_game` takes a name and asks for `save`, then closes itself and
the pause menu; `ui:load` lists `app.saves` with each one's name,
summary and time, greyed where it was made with other mods or cannot be
read, and loads the one chosen; `ui:notice`, which the bundled profiles
name for the role `notice`, shows its `title` and `text` and a button
to carry on.

**Game scenarios** are in `mods/ui/data/scenario.json` (section 3.4,
13 §12): `ui:setup` shows `app.setup`, how the world's seed is chosen in
words (the seed given, a new one each game, or always the same), its
options as a list in which Enter takes the next value, and its places, Enter on one beginning the
game there; `ui:intro` and `ui:summary` are popups over the play screen,
the summary's button and Esc returning to the title, which ends the
game; and `ui:journal`, opened by the control `journal` (`shift+j`),
lists `view.objectives` (14 §13): each objective and mission as the
character knows it, `[x]` done, `[-]` failed, `[ ]` still to do,
indented under its mission, with which way and how far its marker is.

**Using things** is in `mods/ui/data/interact.json` (section 3.7): the
control `interact`, on `e` in play, opens `ui:interact`, the list of
`view.offers_near`, with above it what lies loose within reach that the
character sees (`view.loose_near`, 06 §9.2), each picked up with the
action `pick_up` (decision D-861); the control `pick_up`, on `g` and `,`
in play, picks up the one thing within reach, asks which with the list
picker where there are more, and with nothing there is refused saying
so; the controls `bump` and `bump_ask` are what walking
into something closed runs, by the preference `bump` (`at_once`, `ask`
or `never`).

**Making** is in `mods/ui/data/making.json` (16 §8.6; decision D-828):
the control `make` opens `ui:make`, the list of `view.recipe_options`:
each recipe by hand and at each station within a step, greyed with its
reason where it cannot be started, with where it is made, what it takes
and what it gives; Enter starts the action `make` with the row's
`recipe` and `station`. The control `know_how` opens `ui:know_how`, a
table of `view.know_how_list`: each piece of know-how, whether it is
known or could be learnt next, and what it lets one make. The default
keymap binds `make` to Shift+M and `know_how` to Shift+K.

**The play screen** (`ui:play`) is a `map_view`, the `log` beneath it
and a status column: who the active character is, by `name(view)`; the
slot `ui:play.bars` for a character's reserves; what it holds in a
grasp (`view.held('grasp')`, written out by `list`); the slot
`ui:play.side` for anything else; and a line naming the keys. Its
context is `play`, and it binds no key itself: every key in play is a
control. The `ui` mod's controls are the ten directions (`north` to
`north_west`, `up`, `down`), which start the action `step` with a
`dir`; `wait`, the command of that name for one second; `look`, which
opens look mode (`ui:look`); `inventory` and `condition`, which open
`ui:inventory` and `ui:condition`; `abilities` and `map`, which open
`ui:abilities` and `ui:map`; `journal`, which opens `ui:journal`;
`help`, which opens `ui:help`; `interact`, on `e`, which opens
`ui:interact` (section 3.7); `options`, which opens `ui:options`;
`pause`, which opens `ui:pause`; `omniscient`, the development
profile's view of the truth (section 3.2); `click` and
`click_second` (section 9.4); the fighting controls, the quick slots
and `talk` below. Every screen they open is left by Esc, back to play.
The status column names the target, while there is one
(`app.target`), and its line of keys names Tab, `f` and `s`.

**Fighting** is in `mods/ui/data/fighting.json` (07; decisions D-790 to
D-794). Tab and shift+Tab (`next_target`, `prev_target`) select the
next or the previous creature the character sees as the target, nearest
first, with the `camera` effect's `target`; the map draws its cell in
the theme's `target` style. What can be done is read from the loaded
mods' actions by their tags (`view.actions_with`, section 9.3): `strike`
for a blow and `shot` for a shot. Each is bound twice, quick and
precise (D-079): `attack` (`f`) takes the first blow that can be made at
the target with the action's defaults, and `attack_precise` (shift+f)
lists the blows, greyed with their reasons where they cannot be made,
and asks for every parameter: the target, filled in with the one
selected; the part, from the parts the character sees on the target; the
effort. `shoot` (`s`) and `aim` (shift+s) do the same with shots: the
place aimed at is filled in with where the character believes the
target is, the line drawn to it, and an empty weapon's shot is refused
with the action's own reason. `reload` (`r`) and `throw` (`t`) name
those actions bare, as `step` is named, and `stop` (`z`) cancels the
character's newest action, which lets a held draw go. `grapple` (`v`)
lists the holds, throws of people and takedowns (the tags `grip` and
`takedown`, 07 §9, §10) and tries the one chosen on the target. With no target,
the quick controls ask for one. `attack_bump` and `attack_bump_ask` are
what walking into someone runs, by the preference `bump_attack`
(section 3.7), which the options screen sets.

**Abilities** (`ui:abilities`, `mods/ui/data/abilities.json`; 08 §5;
decision D-795) list `view.ability_uses`, one row for each use of each
ability the character holds (`clash_sim::ability::options`), with its
rank, cost, state (`ready`, `cooling` with the seconds left, `dormant`,
`suppressed`, `unavailable`) and the quick slot it is on, greyed with its reason where
it cannot be used; and `view.ability_modes`, each mode on or off. Enter
on a use closes the screen and uses it with `act` (`{ "first": "[it]" }`):
its parameters are asked for, but for those its grant supplies, and the
command is `use_ability`. Enter on a mode submits `set_mode`. The
screen's context is `abilities`, in which a number key puts the use
selected on that quick slot: the preferences `slot_1` to `slot_9`, kept
between runs with the player's other choices; in play the same key
(`slot_1` to `slot_9`) uses the ability on it, with the target given to
one that takes a `target`. An action is put on a key by rebinding.

**The map** (`ui:map`; decision D-796) is a `map_view` zoomed out by
three over the view model, which holds only what the party has seen and
remembers: the character where it is, and the journal's markers pinned
with the theme mark `marker` where the character last perceived their
targets, which the application gives the view from the character's
journal. Beside it are the sites the character knows by name
(`view.known_sites`: those whose anchor its map memory holds, each with
which way and how far) and the objectives with which way and how far
their markers are. The direction keys move the view six cells at a
time and << and > change storey. The region map and the chart are not
built.

**Talking** (`ui:talk`, `mods/ui/data/talk.json`; decision D-799) is
opened by the control `talk` (shift+t) about the target, or from the
context menu about the creature clicked, each after asking for the
service `talk` with whom. It lists `app.talk.options`, greying one that
says nothing, or says why nothing can be said; Enter says the one
chosen with the action `say` (named bare: the `core` mod's takes `to`,
`kind` and `lines`), which anyone in earshot hears. What is said back
is written in the log by the `core_ui` mod's shows `heard_said`, the
line as it was worded for the character that heard it, and `said_to`,
that a character the player plays spoke to someone, without the words,
which were worded for the hearer.

**The context menu** (`ui:context_menu`, the role `context_menu`) is
what the second button opens over the map with the place clicked: for a
creature, Attack and Shoot (the first blow or shot that can be made at
it) and Talk; and anywhere, Go there (`go_to`), Use what is near
(`ui:interact`) and Look around.

**Look mode** (`ui:look`) is a free cursor over the map, the line of
what the party knows of the place under it, and a side panel: whether
the place is in sight or remembered and how far it is, its ground
(`cursor_ground`) and the things perceived there (`cursor_things`),
all from the view model and named by the reader (section 6.3), and the
slot `ui:look.side`.

**Inventory** (`ui:inventory`) lists what the active character holds,
wears and carries (`view.carried`): each container before what is in
it, what is in a container indented under it, each with how it is held
(`in hand`, `worn`, `in` its container) and, for a container, how much
it holds of what it can (`carried_mass` of `capacity_mass`). Beside it
is the body as a tree of the parts on the outside, each by what a reader
calls it (`part.label`, 06 §6.2), with what is worn over it
(`view.worn_on`), and beneath, the mass carried. Enter on
a thing opens `ui:item` with the thing, how it is held and what it is
called: a button for each item action that applies to how it is held, each
named bare as the walking controls name `step`, with the thing as
`what`: `drop`; `wear` unless worn; `take_off` (Take off) if worn;
`stow` unless stowed; `put_in` (Put in), which asks which of what the
character carries with the `pick` effect and gives it as `container`;
`take_out` if stowed; and `give`, given the thing and asked to whom
(`"needed"`). The `core` mod defines them (`holding.json`, decision
D-861); one that cannot be done is refused with its reason, which the
log shows, and an action no loaded mod defines is refused by the
simulation with its reason. The popup closes once the action is taken. The
slot `ui:inventory.extra` is beneath the list.

**Condition** (`ui:condition`) shows the active character's body as a
tree of parts by their labels (`view.parts`, opened from the root by `attached` and
`inside`), each with its condition as a share, its state and how many
wounds it has; every wound (`view.wounds`) with its part, its kind, its
state and how many layers deep it goes or that it is a stump; and the
slots `ui:condition.pools` and `ui:condition.attributes`, which content
fills with its reserves as bars and its attributes, as it fills
`ui:play.bars` (`mods/core_ui/data/bars.json`), and `ui:condition.extra`.
Everything it reads is the character's own.

**Keys** (`ui:help`) lists every control the loaded mods define, bound
or not, in order of what it is called, with every key the keymap in
force gives it in play, written as a person reads it (`Shift+J`, `Tab`,
`F1`, `Esc`: the rows' `shown`, decision D-866), and what it does
(`app.controls('play')`), so
that a key the player binds shows as it is and a mod's controls appear
with no screen written (decision D-797). `app.keys(context)` still lists
the names a context binds, for a screen that wants them.

The **item** popup (`ui:item`) also throws the thing (Throw...): the
action `throw` with the thing as `what` and the target as `target`,
whichever the loaded mod's action accepts; the `core` mod's throws only
what is made for throwing and refuses anything else with its reason.

**Fitting** shows an assembly (16 §11): its slots, what is in each, what
could go in each from what is at hand, and what the whole offers and draws
of each resource on its bus. It serves a torch, a suit, an implant set and
a vehicle alike, and installs and removes through the interactions the
content defines.

**Character creation** edits a character spec (06 §10.3) held in
`app.creation`. The screen lists the templates the scenario allows, shows
each choice with the widget that fits its kind, and shows the result:
attributes, traits, and the body as a `figure` and a `tree` of parts. The
`assemble` choice gets a part browser and a socket list. All of it is
driven by the templates' data, so a new species with new choices needs no
new screen.

As built, `ui:create` is the screen of the role `creation`, opened from
the new-game screen with F2 (the name `more`) for the scenario selected
there, which it is given as `scenario`. With no session in progress it
lists `app.templates`, each with its description, and Enter begins one
(`creation` with `begin`); where the game offers none it says the
player is given who they are.
Then it has the name and the seed, which `Roll` draws afresh from the
one before, each passed to the session as it changes; what it is made
from; every choice open, indented by depth, with what is chosen (`shown`),
which for a ready-made character (`app.creation.editable` false) is a
list that cannot be changed; the result, `app.creation.attributes` where the application gives them,
and the slot `ui:create.result`; whether the character is complete or
the session's `problem`; `Begin`, shown only while it is complete,
which finishes it and asks for `new_game` for the scenario; and `Start
over`, which picks another template to begin from. Enter on a choice
opens `ui:create_choice` with the choice's number in `app.creation.choices`
and its value to start from: the options of a `pick` or a `points`
choice, each taken or put back with Enter (a pick puts the new option
in place of the first chosen once it has its count); the parts an
`assemble` choice allows, each with how many are chosen, Enter adding
one and Backspace taking every one of it back; a `number` within a
`range` choice's bounds, stepped by a tenth of them; an `input` for a
`text`. Enter takes the number or the text. Whatever the session
refuses opens the screen of the role `error` with the reason, and
changes nothing. Not built: the body as a `figure` and a tree of parts
in the result, traits, a part browser with sockets for `assemble`,
which waits for `add_part`, and a text's `suggestions`.

**Every bundled screen** of the `ui` mod, with the role a profile names
it for, the parameters it is opened with, and its slots:

| Screen | Role | Parameters | Slots | Is |
|--------|------|------------|-------|----|
| `ui:title` | the profile's `boot_screen` | | | New game, Continue, Options, Quit |
| `ui:new_game` | | | | Name, seed and the game to play; F2 opens `ui:create` |
| `ui:setup` | `setup` | `scenario` | | A scenario's places and options |
| `ui:intro` | `intro` | `title`, `text` | | A scenario's introduction |
| `ui:summary` | `summary` | `title`, `outcome`, `text` | | How a game ended |
| `ui:journal` | | | | The objectives and missions as the character knows them |
| `ui:create` | `creation` | `scenario` | `ui:create.result` | Character creation |
| `ui:create_choice` | | `index` | | One choice of the session, by kind |
| `ui:options` | | | `ui:options.prefs` | Preferences and binding a key |
| `ui:pause` | | | | Resume, save, load, options, leave |
| `ui:load` | | | | The saved games |
| `ui:error` | `error` | `text` | | Why a service failed |
| `ui:play` | `play`, `decision.free` | | `ui:play.bars`, `ui:play.side` | The map, the log and the status column |
| `ui:look` | | | `ui:look.side` | Look mode with its side panel |
| `ui:inventory` | | | `ui:inventory.extra` | What is held, worn and carried |
| `ui:item` | | `item`, `hold`, `name` | | What can be done with one thing |
| `ui:condition` | | | `ui:condition.pools`, `ui:condition.attributes`, `ui:condition.extra` | The body, wounds, reserves and attributes |
| `ui:help` | | | | The keys of play and every control |
| `ui:abilities` | | | | The uses and modes of the character's abilities, and the quick slots |
| `ui:map` | | | | What the character knows of the place, with the journal's markers and the sites it knows |
| `ui:talk` | | `who` | | Talking to someone: what can be said to them (`app.talk`), or why nothing can |
| `ui:context_menu` | `context_menu` | `place` | | What can be done with the place or creature clicked |
| `ui:confirm` | `confirm` | `text`, `yes`, `no` | | A question |
| `ui:pick_list` | `picker.list` | `options`, `title` | | One of a list |
| `ui:pick_entity`, `ui:pick_cell` | `picker.entity`, `picker.cell` | `title` | | A cursor over the map |
| `ui:pick_direction` | `picker.direction` | `title`, `value` | | One key |
| `ui:pick_number`, `ui:pick_text` | `picker.number`, `picker.text` | `title` | | A number, a line |
| `ui:pick_multi` | `picker.multi` | `title`, `options`, `min`, `max` | | Several of a list |
| `ui:reaction` | `decision.reaction` | `who`, `what`, `options`, `time_left`, `time_total` | | The reaction prompt: what is happening, the reactions with whether each would be too late, the time left as a bar |
| `ui:choice` | `decision.choice` | `who`, `prompt`, `options` | | The choice prompt |
| `ui:gallery`, `ui:gallery_more` | | `who` (the second) | `ui:gallery.extra`, `ui:gallery_more.extra` | Every widget, for looking at and testing |

## 11. Frontends

| | Window | Terminal |
|---|--------|----------|
| Crates | `winit`, `softbuffer`, `fontdue` | `crossterm` |
| Grid | Yes | Yes |
| Sprites at sub-cell positions, scale, alpha | Yes | Snapped to cells |
| Shake, flash, zoom, bars, tint | Yes | Ignored |
| Colour | 24-bit | Whatever the terminal offers |
| Mouse | Yes, with the cell under the pointer | Optional |
| Fonts | From the theme | The terminal's |
| Leaving | Closing the window | Control-C, or what the screens offer |

The window frontend draws on the processor, not the graphics card
(decision D-830): each frame is drawn into a buffer of pixels by copying
glyphs from an atlas that `fontdue` rasterises from the theme's font,
with sprites, scale, alpha and the frame effects applied in the same
pass, and `softbuffer` shows the buffer in a `winit` window. The same
drawing can write the buffer to an image instead of a window, so the
window's frames are tested headless, and compared cell by cell with the
terminal's.

A third, **null** frontend records frames for tests.

**The window as built** (crate `clash-front-window`, package U3,
decisions D-840 to D-844):

- **The cell.** A cell is the theme font's `size` in logical pixels
  high (section 4.3), times the screen's scale, and as wide as the
  font's advance at that height, rounded down: 8 by 16 pixels for the
  bundled DejaVu Sans Mono at 16. The font is scaled so that the line
  fills the cell from its top edge, and the line-drawing characters of
  borders meet from cell to cell. The window holds as many whole cells
  as fit, with the grid centred and the theme's background round it;
  resizing it changes the grid, which the frontend reports as a
  `Resize` and in its `caps`.
- **Glyphs.** Each character is rasterised once per style and height
  and kept. Bold, italic and underline are made from the one font file;
  dim moves the character's colour nearly halfway to its background;
  reverse exchanges the two. The line-drawing characters of one weight
  (light, heavy and double lines, corners, tees and crosses) and the
  half and full blocks are drawn by the frontend in whole pixels, so
  that borders are crisp and unbroken at any size; the rest of that
  range comes from the font. A character the font lacks is drawn as a
  hollow box, never as nothing, and a control character as nothing, as
  in the terminal. Every character the bundled mods write is in the
  bundled font.
- **Sprites** are centred where a cell at their position would be
  centred, at their scale (glyphs rasterised at that size, not
  magnified) and opacity, lowest layer first.
- **Effects.** The tint and then the flash wash every colour toward
  theirs by their strength. A shake of 1 moves the whole picture by
  half a cell, in a pattern of eight directions taken in turn with
  each frame. The zoom magnifies the picture about the grid's centre,
  where the views put the camera (section 6.3), pixel by pixel, from a
  quarter to eight times. The letterbox bars are black, half of their
  share of the height at the top and half at the bottom.
- **Input.** Keys get the names and modifiers the terminal gives them
  (section 9.1); Control-C is an ordinary key, and closing the window
  is `Close`. The mouse is reported with the cell under the pointer as
  the picture on screen laid it out, zoom included: a move when the
  pointer reaches another cell, presses and releases of the three
  buttons, the wheel in notches. Losing the focus forgets the
  modifiers held; it is no event of its own. Text comes only from an
  input method; there is no paste.
- **The loop stays the application's.** The frontend's `poll` pumps
  the window system's events without waiting, and `present` draws and
  shows a frame at once.

Each frame is drawn in full, with no allocation once the glyphs it uses
are made; section 15 gives its cost. A frame the same as the one on
screen, in a window of the same size, is not drawn again unless it
shakes.

**Running the game in a window.** `cargo run -p clash-app --release`
opens a window when a display is available, and `--window` asks for
one (section 3.1); `--terminal` keeps the game in the terminal. If the
window cannot be opened the reason is printed and the game runs in the
terminal. `cargo run -p clash-front-window --example effects` opens a
window that shows sprites gliding and each frame effect in turn, which
is the quickest check that a machine can show the window at all.

| System | Needs |
|--------|-------|
| Linux and the BSDs, X11 | `DISPLAY` set; at run time `libX11`, `libxcb`, `libXcursor`, `libXrandr`, `libXi` and `libxkbcommon-x11` (Debian and Ubuntu: `libxkbcommon-x11-0`). Nothing is needed to build: every library is loaded when the program runs. |
| Linux and the BSDs, Wayland | `WAYLAND_DISPLAY` set, with `libwayland-client` and `libxkbcommon` at run time. Wayland is tried before X11 when both are named. |
| macOS | Nothing beyond the system. Run it from the main thread, as the binary does. |
| Windows | Nothing beyond the system. |

A theme names its font as a file inside the mod that writes it
(decision D-844); the bundled themes name `mods/ui/assets/font.ttf`,
DejaVu Sans Mono, whose licence is beside it in `font-LICENSE.txt`.

## 12. Testing

- The scene is driven with a fixed time step and scripted input.
- **Golden frames**: a frame is written as text (characters plus colour
  indices) and compared with a stored one. Bundled screens and playback of
  the world and kernel scenarios have golden frames. The text holds the
  grid of characters, each cell's foreground and background as indices
  into a palette listed with it, each cell's attributes, the sprites and
  the frame's effects. Running the tests with `BLESS=1` in the environment
  writes the stored frames instead of comparing with them; a change that
  alters one says which and why.
- The check of section 5 (view model equals fresh initialisation) runs after
  every scripted playback.
- A test asserts that the scene's only route to the simulation is `submit`.

## 13. Modding surface

**Definition types**: `screen`, `ui_part`, `theme`, `color`, `ambience`,
`overlay`, `preference`, `figure`, `keymap`, `control`, `show`, `anim`,
`cutscene`. Game scenarios are 13's `scenario` (section 3.4). Theme `marks`: the glyphs the engine draws for itself (a pile
of items, a suspected presence, something only heard, a capsized vessel).
A glyph may give a `layer` to be drawn over or under its usual place in
the order of section 6.1. Functions `between(a, b)` and `dir(a, b)`. The `glyph`
field on everything drawable. The `look` field on traits, modes, postures
and worn items. The `cutscene` field on actions.

**Simulation effects**: `play_cutscene`, `show_screen`. **Rule field**:
`ui_callable`. **Command**: `Ui { rule, args }`.

**Input**: the fields of `keymap` and `control`, the contexts and how a
bare name is read: section 9.2. The pickers, what each is given and the
`pick` and `from` of `act`: section 9.3. The two controls a click runs
and what `it` is: section 9.4. The decision roles and what each prompt
is given: section 9.5. The variables a screen is told of the place under
the cursor, and the theme mark `line`: sections 6.3 and 6.5. The
effects `camera` and `picker`: section 10.5. The roles: section 10.7.

**Settings**: `stretch_time`, `look_down`, and the player's preferences
for playback speed, which effects play and how often cutscenes play.

**Playback**: the fields of `show` and `anim`, the changes a show makes
and the effects it draws: section 7.3. The scope a show's texts and
condition are compiled in, and what the host answers for them: sections
7.3 and 7.5. The bundled shows are `mods/ui/data/playback.json`, one or
more for each event of the kernel and the world.

**The application**: the command line, the user's data directory and
what it holds, and the headless script: sections 3.1 and 3.3. The roles
`error` and `notice`, the commands the play screen submits, the services
`creation` and `edit_creation`, `app.creation` and the
`creation_<field>` variables: sections 3.2, 3.4 and 10.4. The roles
`setup`, `intro` and `summary`, `app.setup` and the op `option`:
section 3.4. The saves, their ids, summaries and
states, and the saving policies: section 3.6. The preference `bump` and
the controls `bump`, `bump_ask` and `interact`, what each is given and
the property `offers_near` they read: section 3.7. The preference
`bump_attack` and the controls `attack_bump` and `attack_bump_ask`:
section 3.7. The properties `actions_with`, `ability_uses`,
`ability_modes` and `known_sites`, `app.target` and `app.controls`, the
`act` effect's `first` and `with`, the fields an entry of a list of
actions may carry, and the commands `use_ability` and `set_mode`:
sections 3.2, 9.3 and 10.4. The fighting controls, the quick slots
(the preferences `slot_1` to `slot_9`), the theme mark `marker` and
style `target`: sections 6.5 and 10.7. The shows of combat and powers
are the `core_ui` mod's (`mods/core_ui/data/shows.json`).

**Widgets**: section 10.3. **UI effects**: section 10.5.
**Binding names**: section 10.4. **Navigation names**, and the theme
styles and marks the toolkit draws with: section 10.3. **Key names**:
section 9.1. The fields of `ambience` and `overlay`, the built-in
overlay names, and the theme marks, style and colour the views draw
with: sections 6.2, 6.4 and 6.5.

**Gaps.** The notice shown on a grid smaller than a screen's `min_size`
is the engine's own text and cannot be changed by a mod. The set of
widgets, and how each is drawn beyond its styles and marks, is fixed.
The shares by which a cell is dimmed for depth, light and memory
(section 6.1) are the engine's constants, not settings. A mod moves
the camera only through the `camera` effect, whose
arguments are whole numbers written outright, and a view's `centre` is
compiled and not yet followed. A name in a keymap that is neither a
control's nor one a screen or the toolkit uses is not reported: the key
does nothing. An action a control names is not checked when mods load
(section 9.2). A picker cannot be told a parameter's type or its `check`
beyond the variables of section 6.3, and the route picker's numbered
path is not drawn: a route is asked for with the cell picker, one step
at a time. Several characters cannot be given one command
(section 9.6). The words a picker's title is made of, the action's name
and the parameter's, are joined by the engine with a colon. The share of its
brightness that a dimming popup leaves beneath it is the engine's
constant. What is put into a slot is always laid out one below another.
A screen's `rows` cannot yet come from a query (05 §11), only from an
expression. An `inspect` cannot yet list the properties of a subject
given by `of`. Which preferences the engine itself acts on, `theme` and
`keymap`, and the application, `omniscient` and `bump`, is fixed. A profile cannot
be switched, nor mods read again, while the application runs: the
services `select_profile` and `reload_mods` are refused, saying to start
again. The grid of a headless run, its time step, the frame time and the
share of it given to the simulation, the wake-ups one call of `advance`
may handle and the hour of simulation time one pass may cover are the
application's constants. The senses a character made without any is
given are named by its scenario, or are every sense loaded (15 §15). A save's
id is made from its name by the application's own rule, and `single`
saves only as a game is left, not at intervals. The words with which a
load's warnings are introduced, and the summary a save is given, are
the application's. A `message` to the
player and a fault are written in the log by the application as they
are reported, in the categories `message` and `fault`, and not as they
are played; no `show` can word or pace them.
The bundled screens read what the simulation registers, and it
registers no property for a character's vital state (06 §8.4), no
list of the pools and attributes an entity has and no pool's
thresholds: the screens show none of these themselves, and content
puts its reserves and attributes into the slots `ui:play.bars`,
`ui:condition.pools` and `ui:condition.attributes`, as the mod `core_ui` does for `core`, so that neither `core` nor `ui` needs the other.
What a thing is called comes from the reader helpers the host answers,
which call an item with no name of its own `something` until they name
it by its definition; a wound's kind and an assemble choice's parts are
shown by their qualified ids, and an element in look mode by its id
without the namespace. The body tree grows from a body's first part
where that is its root, and lists every part flat where it is not. The
inventory names the item actions `drop`, `wear`, `remove`, `stow`,
`put_in`, `take_out` and `give` bare, with the parameters `what`,
`container` and `to`, and a mod that defines them under other
parameters is not asked for them. Rolling a seed on the creation screen
draws the next one from the last by a fixed rule in the screen, since
a binding may not be random.
The words `Yes` and `No` of the bundled confirm
screen are that screen's, and a mod changes them by overriding it.
How playback is paced beyond `stretch_time` and `playback_speed` is
the engine's: that a long wait counts for two seconds, that a backlog
of more than a second plays faster, how long a heard marker stays, how
strongly a flash begins and how many afterimages a fast move leaves.
`stretch_time` itself is set by the application until settings exist
(P6). Which effects a show can draw is the fixed set of section 7.3,
and an animation is a glyph's keyframes: floating text, particles and
effects shaped by an event's fields are not built. No event is marked as
one the player must read about, so the linter's warning of invariant V5
is not given. The words of a line come from the reader helpers the host
answers, and the bundled shows name a destroyed block a wall, since an
event says nothing of what was destroyed that the reader may know.
Which actions the fighting controls offer is said by four tags,
`strike`, `shot`, `grip` and `takedown`, written in the `ui` mod's
controls; a stance (`take_stance`) is not offered, since nothing lists
the stances its `id<trait>` parameter could be chosen from; `reload` and `throw` are
named bare, and `target` is the name of the parameter `with` and
`actions_with` fill with the creature. The order in which
`actions_with` lists them, and so which blow the quick key strikes, is
the engine's (decision D-791): a mod cannot rank its own attacks.
Whether an action could start is judged without its footing, the slots
other actions hold and its hooks. The parts offered for a blow are
those on the outside of the target's body as it is, while the character
perceives it: no previews of odds or harm (07 §13) are shown, since
the function `strike_preview` reads the target as it is and a screen,
compiled in the mind context, may not call it; and a part's appearance
(15 §8.4) is not consulted. Quick slots hold
abilities only, and the keys `1` to `9` only those; an action is put on
a key by rebinding. The map is a map view zoomed out by a fixed three:
the region map and the chart (section 6.2) are not built, and a pin is
drawn only where a marker falls inside the view. What can be said is
gathered once, as the talk screen opens, and is not gathered again
while it is open; an option says a message kind with a table of lines,
since that is all `talk_options` gives (15 §11), and the `say` it
starts takes a second.
The context menu's offers are written out in the screen, not listed
from what the place offers. A held phase (a bow drawn, a charge) runs
to its `max` in one pass, since the loop advances the simulation as far
as it can while the character is busy; `stop` lets it go only between
passes.
How the window frontend draws is the engine's (section 11): a theme
names one font file, and bold, italic and underline are made from it;
which line-drawing characters and blocks the frontend draws itself, and
how; the box drawn for a missing character, how far dim fades, how far and
in what pattern a shake moves the picture, the range of the zoom and
the colour of the letterbox bars are fixed. The window keeps the font
of the theme in force when it opened: a theme chosen during play
changes the colours and not the font. The window's title is the
profile's name, and the grid it opens with is the frontend's.

## 14. Invariants

| # | Invariant |
|---|-----------|
| V1 | The scene changes the simulation only through `submit`. |
| V2 | Nothing computed by presentation is read by the simulation. |
| V3 | The view model shows only what controlled characters perceived, except in debug contexts. |
| V4 | Caught-up playback equals a fresh view of the simulation, and until then every binding on the view shows the moment playback has reached. |
| V5 | Every player-relevant event produces a log line. Nothing is conveyed only by an effect a terminal cannot show. The linter warns of an event type registered as player-relevant that no `show` gives a `log`. |
| V6 | A cutscene never alters, drops or reorders events in simulation time. |
| V7 | Every bundled screen is a definition that a mod can override. |
| V8 | With a fixed time step and input script, frames are reproducible. |
| V9 | Text shown to a player names others only as the reader knows them. |
| V10 | Nothing in bundled content is told apart by colour alone. |

## 15. Budgets

Targets, unmeasured but where a figure is given: those rows are in
`budgets.toml` and measured by `cargo xtask bench` (14 §6.3, D-730).

| Operation | Target |
|-----------|--------|
| Build a 200 × 60 frame | under 1.5 ms (measured by `cargo xtask bench`: 1.7 to 2.1 ms for the map of a generated city alone, from the view model, without the widgets round it) |
| Ingest 10,000 events | under 5 ms |
| Re-evaluate bindings of a busy screen | under 0.5 ms |
| Window frontend, a frame of 20,000 glyphs and sprites drawn into the pixel buffer at 1920 × 1080 | under 16 ms: 60 frames per second (measured by `cargo xtask bench`: 4.2 to 9.5 ms on a shared, loaded machine for 240 × 67 cells of 8 × 16 pixels and 3,920 sprites, a third of them half clear and a tenth half as large again; 5.1 to 9.3 ms with zoom, flash and shake) |
| Input to visible response | within one frame |

## 16. Open questions

1. **Sound.** Reserved, not built. `show` definitions may name a `sound`
   cue (section 7.3), which nothing plays yet.
2. **Tilesets.** Kept out of the first version. A sprite sheet in place of
   a font is a small step for the window frontend.
3. **Wide characters.** Deferred, with translation (04 §20). Glyphs are
   single-width.
4. **Screen readers.** Deferred. The log is complete by invariant V5, so a
   plain-text mode for assistive tools is feasible.
5. **Controllers.** Deferred. Keymaps name what a key means (section 9.2),
   so another device can be mapped to the same names.
