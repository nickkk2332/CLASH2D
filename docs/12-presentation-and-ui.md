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

## 4. Glyphs and frames

### 4.1 Glyphs in definitions

Anything drawable carries a `glyph`:

```json
{ "ch": "#", "fg": "material", "bg": null, "bold": false,
  "by_state": { "open": { "ch": "'" } },
  "by_level": [ { "ch": "." }, { "ch": "^", "fg": "orange" }, { "ch": "^", "fg": "red" } ] }
```

- `ch` is one Unicode character of single width.
- `fg` and `bg` are a theme colour name, `#rrggbb`, or `"material"`, which
  takes the colour of the thing's material.
- `by_state` and `by_level` vary the glyph by element state or by field or
  damage level.

### 4.2 Frames

```rust
pub struct Frame {
    pub cols: u16, pub rows: u16,
    pub cells: Vec<FrameCell>,        // the grid: character, foreground, background, attributes
    pub sprites: Vec<Sprite>,         // glyphs not aligned to the grid
    pub fx: FrameFx,                  // shake, flash, zoom, letterbox bars, tint
}

pub struct Sprite { pub ch: char, pub fg: Rgb, pub x: f32, pub y: f32,
                    pub scale: f32, pub alpha: f32, pub layer: u8 }
```

The grid is what a terminal can show. **Sprites** are how the window
frontend goes further: a character gliding between cells, fragments flying,
floating text, afterimages. The terminal frontend draws each sprite in the
nearest cell and ignores scale, alpha and `fx`. Nothing essential may be
conveyed only by something a terminal cannot draw.

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
  initialisation would produce. Debug builds check this.

An omniscient mode for debugging builds the view model from truth.

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

Then, in order: decals tint; light scales brightness; cells remembered but
not currently seen are desaturated and dimmed. Subgrids are composited as
the world does (02 §18.1).

A creature is drawn as the persona the viewer perceives.

**Through portals.** What a viewer sees through a see-through portal
(02 §22) is drawn beyond the portal, in the direction they are looking, in
place of what is physically there. One portal deep by default.

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

A `map_view` may draw an **overlay** on top of the cells:

| Overlay | Draws |
|---------|-------|
| `networks` | The conduits of one resource, coloured by network, with each network's supply |
| `areas` | Areas by kind: storage, rooms, whatever content defines |
| `jobs` | Planned work as ghosts, with its state |
| `temperature`, `light` | The value at each cell, as shading |

In an ordinary screen an overlay is drawn from the view model, so it shows
what the character has perceived. In the debug context it shows the
truth. Mods add overlays as definitions.

### 6.3 Camera

The camera follows the active controlled character, or a free cursor in
look and targeting modes. The viewed storey follows the character and can
be shifted up and down. Cutscenes move the camera (section 8).

## 7. Playback

### 7.1 Pacing

Events arrive with ticks. Playback maps simulation time to real time one
stretch at a time, a stretch being what happened between two decisions.

- A stretch with nothing perceivable in it is skipped.
- Otherwise it plays in about `stretch_time` (default 350 ms), however much
  simulation time it covers. One step by a slow character and forty by a
  fast one take the same real time.
- An event whose presentation has a `min_time` holds the clock for at least
  that long.
- A player setting scales everything, down to instant.
- Any key during playback jumps to its end.

### 7.2 Motion

Relocation events carry where from, where to and how long the move took.
The sprite glides accordingly. Something that crosses many cells in one
frame leaves a trail of afterimages, which is how a much faster character
reads on screen as fast instead of as teleporting.

Bodies in flight (02 §15) report their path and are drawn along it.

### 7.3 Showing events

```json
{ "type": "show", "id": "glass_breaks",
  "on": "element_destroyed", "filter": { "material": "glass" },
  "anim": "shatter", "at": "event.here",
  "text": { "say": "KSSH!", "style": "sfx" },
  "log": "Glass shatters.",
  "heard": { "log": "Glass breaks somewhere to the {dir(view, event.here)}.", "marker": "!" },
  "shake": 0.1, "min_time": "120 ms", "weight": 2, "sound": "glass_small" }
```

A `show` definition says how an event appears: an animation, floating text,
a log line, screen shake, and what to do if the event was only heard. It
may name a `sound` cue. No frontend plays sound in the first version; the
name is carried so that content can be written ahead of one that does.
Several may match one event; all apply. Events with no `show` simply update
the view model.

```json
{ "type": "anim", "id": "shatter", "time": "300 ms",
  "frames": [ { "at": 0.0, "ch": "*", "fg": "white" }, { "at": 0.5, "ch": "%" } ],
  "particles": { "count": 8, "ch": ["'", ",", "`"], "speed": [2, 6], "spread": 360, "gravity": 9, "fade": true } }
```

An `anim` is a short keyframed effect at a place: glyph changes on the cell
and particles as sprites. Larger effects (a pressure wave's ring, a beam)
take parameters from the event, such as radius and direction.

### 7.4 The log

`message` effects (05 §10.6) and `log` lines from `show` definitions go to
the message log with their tick and a category. The `log` widget displays
and filters them. The log is complete: anything shown by animation alone
also has a line, so the game is playable by reading.

## 8. Cutscenes

### 8.1 Cues

A cutscene begins with a **cue**, raised one of three ways:

| Source | For |
|--------|-----|
| An action names a `cutscene` | Takedowns, executions, signature moves. The cue fires when the action starts and carries its participants. |
| A `cutscene` definition has `on` and `if` | Moments that emerge: a clash above some energy, a tower falling, a death. The condition is evaluated in the simulation when the event occurs, like a rule whose only effect is the cue. |
| The `play_cutscene` effect | Anything else. |

A cue is an output event. It changes nothing in the simulation.

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
      "caption": "{a.name} and {b.name} meet." },
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

### 9.2 Keymaps and controls

Input is resolved in two steps.

A **keymap** names what a key means in a context:

```json
{ "type": "keymap", "id": "default",
  "map":  { "h": "west", "a": "attack", "i": "inventory", "shift+a": "attack_precise" },
  "menu": { "up": "prev", "down": "next", "enter": "accept", "escape": "back" } }
```

Players rebind keys; the names stay.

A **control** says what a name does during play:

```json
[
  { "type": "control", "id": "west",
    "do": { "act": { "action": "step", "params": { "dir": "west" } } } },
  { "type": "control", "id": "attack",
    "do": { "act": { "action": { "pick": "attack_actions" }, "params": "needed" } } },
  { "type": "control", "id": "attack_precise",
    "do": { "act": { "action": { "pick": "attack_actions" }, "params": "all" } } },
  { "type": "control", "id": "inventory", "do": { "open": "inventory" } }
]
```

Because actions are content, so are controls. The engine has no built-in
"move" or "attack" key.

### 9.3 Collecting parameters

`act` builds an `Act` command (03 §4.4) for the active character.

| `params` | Behaviour |
|----------|-----------|
| `"needed"` | Ask only for parameters that have no default. One key press, and the action's defaults choose target, limb and spot. |
| `"all"` | Ask for every parameter, each pre-filled with its default. Full control. |

This is how decision D-037 reaches the player: the same action bound twice,
quick and precise.

Each parameter type has a **picker**: a cell, an entity, a direction, a
body part, an item, a number, an entry from a query. Pickers are screens and
can be restyled or replaced.

### 9.4 Mouse

In the window frontend: hovering shows the inspection tooltip for what is
under the pointer; a click on the map runs the `click` control, by default
"go there" or "open its interactions"; a second-button click opens a menu
of the interactions (02 §5.5) and actions that apply. All widgets respond
to clicks and the wheel. Everything remains reachable by keyboard.

### 9.5 Decisions

When the simulation returns a decision (03 §5.2), the scene, once playback
reaches that tick, makes that entity active and opens the screen registered
for the decision kind: none for a free decision, the reaction prompt with
its options and a bar showing time remaining, or the choice screen for
`ask`.

### 9.6 Several at once

The player may select several directly controlled characters. A command
given then goes to each of them, in entity order, and each fills in its own
parameters from its own defaults. A decision for one of them can be
answered for all of them that have the same decision pending.

This is how a party is moved together, and how someone in several bodies
acts as one.

## 10. The UI toolkit

### 10.1 Screens

```json
{ "type": "screen", "id": "condition",
  "title": "{$who.name}",
  "context": "menu",
  "params": { "who": "entity" },
  "vars": { "tab": 0 },
  "layout": { "cols": [
    { "width": 34, "frame": "Wounds",
      "list": { "of": { "wounds": { "of": "$who" } },
                "item": "{it.part.name}: {it.kind.name}", "empty": "Unhurt." } },
    { "fill": 1, "rows": [
      { "bar": { "label": "Blood", "value": "$who.pool.blood.frac", "style": "blood" } },
      { "figure": { "of": "$who" } },
      { "slot": "condition.extra" } ] } ] },
  "keys": { "back": [ { "close": {} } ],
            "tab":  [ { "set": { "screen.tab": "(screen.tab + 1) % 3" } } ] } }
```

A screen has parameters, local variables, a layout tree, and key handlers.

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

### 10.4 Binding

Every widget property that shows something is an expression in a **pure**
context (05 §7.3). The names available:

| Name | Is |
|------|----|
| `view` | The active controlled character |
| `party` | All directly controlled characters |
| `$name` | Screen parameters |
| `screen.name` | Screen variables |
| `it` | The current row in a list, table or tree |
| `app` | Things outside the simulation: saves, profiles, mods, scenarios, preferences, the creation session |

Bindings are re-evaluated when the simulation has advanced or a screen
variable has changed, for widgets that are visible.

**What a screen may know.** In ordinary screens, the party's own
characters are available in full. Everyone and everything else is available
only as the party perceives it: queries return perceived personas and
remembered places (15). Screens with context `debug` see truth.

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
| `app` | Engine services: new game, save, load, delete save, quit, change a preference, rebind a key, select a profile, reload mods, edit the creation session. Saving and loading are offered only as the scenario's saving policy allows (13 §9.2). |

UI effects cannot change the simulation except by submitting commands.

### 10.6 Extending screens

```json
{ "type": "ui_part", "id": "mana_bar", "into": "ui:hud.bars", "order": 20,
  "if": "view.has_trait('attuned')",
  "widget": { "bar": { "label": "Mana", "value": "view.pool.mana.frac", "style": "mana" } } }
```

A screen places `slot`s where others may add. A `ui_part` puts a widget
into a slot. A mod that adds a resource adds its bar without replacing the
whole display. A mod that wants a different display overrides the screen.

### 10.7 The bundled screens

The `ui` mod provides: title, profile and mod list, new game and scenario
choice, character creation, the play screen (map, log, status, slots),
inventory, condition, abilities, look and inspect, the reaction prompt, the
choice prompt, the pickers, options and key binding, save and load, trade
(16 §3.6), fitting, make, build, work, device and know-how.

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

## 11. Frontends

| | Window | Terminal |
|---|--------|----------|
| Crates | `winit`, `wgpu`, a font rasteriser | `crossterm` |
| Grid | Yes | Yes |
| Sprites at sub-cell positions, scale, alpha | Yes | Snapped to cells |
| Shake, flash, zoom, bars | Yes | Ignored |
| Colour | 24-bit | Whatever the terminal offers |
| Mouse | Yes | Optional |
| Fonts | From the theme | The terminal's |

The window frontend draws the grid and sprites as textured quads from a
glyph atlas built from the theme's font.

A third, **null** frontend records frames for tests.

## 12. Testing

- The scene is driven with a fixed time step and scripted input.
- **Golden frames**: a frame is written as text (characters plus colour
  indices) and compared with a stored one. Bundled screens and playback of
  the world and kernel scenarios have golden frames.
- The check of section 5 (view model equals fresh initialisation) runs after
  every scripted playback.
- A test asserts that the scene's only route to the simulation is `submit`.

## 13. Modding surface

**Definition types**: `screen`, `ui_part`, `theme`, `figure`, `keymap`,
`control`, `show`, `anim`, `cutscene`. The `glyph` field on everything
drawable. The `cutscene` field on actions.

**Simulation effect**: `play_cutscene`. **Rule field**: `ui_callable`.
**Command**: `Ui { rule, args }`.

**Widgets**: section 10.3. **UI effects**: section 10.5.
**Binding names**: section 10.4.

## 14. Invariants

| # | Invariant |
|---|-----------|
| V1 | The scene changes the simulation only through `submit`. |
| V2 | Nothing computed by presentation is read by the simulation. |
| V3 | The view model shows only what controlled characters perceived, except in debug contexts. |
| V4 | Caught-up playback equals a fresh view of the simulation. |
| V5 | Every player-relevant event produces a log line. Nothing is conveyed only by an effect a terminal cannot show. |
| V6 | A cutscene never alters, drops or reorders events in simulation time. |
| V7 | Every bundled screen is a definition that a mod can override. |
| V8 | With a fixed time step and input script, frames are reproducible. |

## 15. Budgets

Targets, unmeasured.

| Operation | Target |
|-----------|--------|
| Build a 200 × 60 frame | under 1.5 ms |
| Ingest 10,000 events | under 5 ms |
| Re-evaluate bindings of a busy screen | under 0.5 ms |
| Window frontend, 20,000 quads | 60 frames per second |
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
