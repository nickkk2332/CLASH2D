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
| `font` | The font file and glyph size for the window frontend. |
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
  initialisation would produce. Debug builds check this.

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

And with the theme style `cursor` (reverse unless given) and the theme
colour `overlay_light` (yellow unless given).

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
- The key bound to `skip` in the keymap jumps playback to its end. By
  default that is any key.

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
Several may match one event; all apply. When their animations together
would overrun the stretch, those of lower `weight` are dropped first.
Log lines are never dropped. Events with no `show` simply update
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
interrupt the program and a player must always be able to leave.

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
body part, an item, a number, an entry from a query, a route. Pickers are
screens and can be restyled or replaced.

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
| `params` | The values it is opened with: each name and its type. Read as `$name`. |
| `vars` | Its own variables and their starting values: `true`, a number or a text. Read as `screen.name`. |
| `min_size` | The smallest grid it can be shown on, as `[columns, rows]` |
| `layout` | The layout tree: one node (section 10.2) |
| `keys` | For a key name, the UI effects run when that key is pressed and no widget used it |

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
| `slot` | The slot's name |

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
| `list`, `tree`, `menu` | `item`: how a row from a query is shown, a text over `it`. `list` and `tree`: `empty`, shown when there are no rows. `tree`: `children`, an expression giving the rows beneath `it`. |
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
| `log` | `filter`: a list of categories, or an expression |
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
on show, by their hotkeys; to the screen's `keys`; and last to the toolkit
itself. The names the toolkit and its widgets understand:

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

| Name | Is |
|------|----|
| `view` | The active controlled character |
| `party` | All directly controlled characters |
| `$name` | Screen parameters |
| `screen.name` | Screen variables |
| `it` | The current row in a list, table or tree |
| `app` | Things outside the simulation: saves, profiles, mods, scenarios, preferences, the creation session |

Bindings are re-evaluated when the simulation has advanced or a screen
variable has changed, for widgets that are visible. While events are
still being played back, bindings on the view show the world as of the
moment playback has reached, so a health bar does not drop before the
blow is seen.

`app` is the same for every mod and is listed in full in the generated
reference:

| `app.` | Is |
|--------|----|
| `saves`, `profiles`, `mods`, `scenarios` | Lists, each entry with its name, description and state |
| `profile`, `scenario` | The ones in use |
| `pref(id)` | A preference (below) |
| `creation` | The character being made (06 §10.3) |
| `size` | The grid in columns and rows |
| `can_save`, `can_load` | Whether the scenario's saving policy allows it now |

**What a screen may know.** In ordinary screens, the party's own
characters are available in full. Everyone and everything else is available
only as the party perceives it: queries return perceived personas and
remembered places (15). Screens with context `debug` see truth.

In an ordinary screen a parameter of type `entity` that is not a party
member stands for what the party knows of it: its persona, its track
(15 §7.2), the features made out (15 §8.4) and whatever was appraised
(08 §7.5). Reading anything else of it is a load error. A parameter of
type `party_member` may be read in full.

**Preferences.** A `preference` is a player's choice about display and
input, kept outside saves and invisible to rules:

```json
{ "type": "preference", "id": "damage_numbers", "kind": "flag", "default": false,
  "name": "Show damage numbers", "doc": "Float the energy of each hit over its target." }
```

The built-in ones are defined the same way: `theme`, `keymap`,
`playback_speed`, `cutscenes` (all, major, none). A profile may set
defaults.

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
error, unless it says `"optional": true`. Any layout node may carry an
`id`, so that a patch can change one node of a screen and leave the rest
(04 §6.1). A mod that adds a resource adds its bar without replacing the
whole display. A mod that wants a different display overrides the screen.

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
| `play`, `creation`, `summary` | A game is being played, a character made, a game ended |
| `reaction`, `choice` | The simulation asks for a decision |
| `picker.<type>` | A parameter of that type is being chosen |
| `tooltip`, `context_menu` | Something is hovered or clicked (section 9.4) |

A role with no screen is a load error.

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
`cutscene`. Theme `marks`: the glyphs the engine draws for itself (a pile
of items, a suspected presence, something only heard, a capsized vessel).
A glyph may give a `layer` to be drawn over or under its usual place in
the order of section 6.1. Functions `between(a, b)` and `dir(a, b)`. The `glyph`
field on everything drawable. The `look` field on traits, modes, postures
and worn items. The `cutscene` field on actions.

**Simulation effects**: `play_cutscene`, `show_screen`. **Rule field**:
`ui_callable`. **Command**: `Ui { rule, args }`.

**Settings**: `stretch_time`, `look_down`, and the player's preferences
for playback speed and how often cutscenes play.

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
(section 6.1) are the engine's constants, not settings. A mod cannot
choose which line a section view cuts along or put a camera anywhere:
those are the scene's until controls and UI effects reach the camera
(packages U6b, U7).

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
