# Plan — First Playable: Entities, Perception, Presentation

Implements [06-entities-and-bodies.md](../06-entities-and-bodies.md),
[15-perception-and-social.md](../15-perception-and-social.md) and
[12-presentation-and-ui.md](../12-presentation-and-ui.md).

Packages prefixed F and W are in [world-model.md](world-model.md); P, X, K
and T are in [sim-kernel.md](sim-kernel.md). The rules for taking a package
are those of [world-model.md](world-model.md) and
[CLAUDE.md](../../CLAUDE.md).

When this plan is complete there is a game to sit down at: a window, a
character built from templates, a destructible place with people in it who
must notice you before they can respond to you. Fighting is added by
[combat.md](combat.md); within this plan, attacks in tests are single
actions that call `hit`.

## Status

| WP | Title | Size | Depends on | Status |
|----|-------|------|------------|--------|
| E1 | Component store and core components | M | K1, K7 | todo |
| E2 | Attributes, pools, tracks | M | E1, K5 | todo |
| E3 | Traits | M | E2 | todo |
| E4 | Bodies | L | E2, T3 | todo |
| E5 | Hits and wounds | L | E4, W12 | todo |
| E6 | Body editing | M | E4 | todo |
| E7 | Minds and vital state | M | E1, T4 | todo |
| E8 | Items and holds | L | E5 | todo |
| E9 | Templates and creation | L | E3, E6, E8 | todo |
| E10 | Movement | M | E4, W14, W18 | todo |
| E11 | Coming and going | L | E9, W8 | todo |
| E12 | Minimal devices | M | E8 | todo |
| S1 | Light | M | W18 | todo |
| S2 | Signals and senses | M | E3 | todo |
| S3 | Detection | L | S1, S2, E10 | todo |
| S4 | Percepts | M | S3, K3 | todo |
| S5 | Beliefs | M | S4, E7 | todo |
| S6 | Personas | M | S5 | todo |
| S7 | Relations, groups, orders | M | S6, T4 | todo |
| S8 | Speech | M | S7, T6 | todo |
| S9 | Party perception | M | S5 | todo |
| S10 | Coloured light (experimental) | S | S1, U4, P4 | todo |
| U1 | Scene skeleton, frames, themes | M | K2 | beta |
| U2 | Terminal frontend | S | U1 | done |
| U3 | Window frontend | L | U1 | todo |
| U4 | View model and world drawing | L | U1, S9 | todo |
| U5 | Playback | L | U4 | todo |
| U6a | Toolkit: layout and widgets | L | U1 | beta |
| U6b | Toolkit: binding and UI effects | M | U6a, X4, K6 | todo |
| U7 | Input, controls, pickers | M | U6b, T5 | todo |
| U8 | Slots and theming | S | U6b | todo |
| U9 | Bundled screens | L | U7, U8, U4, U5, E9 | todo |
| U10 | Cutscenes | M | U5, T6 | todo |
| U11 | Application | M | U3, U9, P3 | todo |
| Z1 | Benchmarks and budget gate | S | E1, then recurring | todo |

## Order and parallelism

The three tracks share few dependencies. U1 to U3 and U6a need only the
kernel and can start as soon as K2 is done.

| Wave | Entities | Perception | Presentation |
|------|----------|------------|--------------|
| 1 | E1 | S1 | U1 |
| 2 | E2, E7 | — | U2, U3, U6a |
| 3 | E3, E4 | — | U6b |
| 4 | E5, E6, E10 | S2 | U7, U8 |
| 5 | E8 | S3 | — |
| 6 | E9, E12 | S4 | — |
| 7 | E11 | S5 | — |
| 8 | — | S6, S9 | — |
| 9 | — | S7 | U4, U11 |
| 10 | — | S8 | U5 |
| 11 | — | — | U9 |
| 11 | — | — | U10 |

## Milestones

| Milestone | Packages | What can be shown |
|-----------|----------|-------------------|
| **M3a People** | E1–E12 | From the console: build a character from templates with choices, cut it and watch the arm stop working and the blood pool fall, reshape its body, move its mind to another body, have it hide in a locker. |
| **M3b Awareness** | S1–S9 | A guard hears a window break and goes to look. Someone in shadow goes unnoticed; someone in a locker is not seen. A crime done in another's face is remembered as theirs. |
| **M3c A window** | U1–U11 | Launch the game, pick a profile, build a character on the creation screen, walk around a building in a window with mouse and keys, watch a wall break in playback, read it in the log, see a takedown as a cutscene. |

## Entities

### E1 — Component store and core components

**Spec** 06 §3 · **Size** M

Build
- `ComponentStore<C>`, registration, saved dense order.
- `Identity` and `Physical` with derived, cached mass and size.
- Component saving and loading through the K7 path.

Done when
- Store operations match a simple model (property test); iteration order
  survives save and load (invariant B8).
- `Physical` equals a from-scratch recomputation after random changes.

### E2 — Attributes, pools, tracks

**Spec** 06 §4 · **Size** M

Build
- The three definition types; modifiers with sources; derived attributes
  with dependency invalidation.
- Pools with rates and scheduled threshold crossings.
- Tracks with level curves.

Done when
- Cached attributes equal recomputation under random modifier changes.
- A pool crossing fires at the analytically correct tick, including across
  a clock rate change, and an idle pool schedules nothing.
- `track_advanced` fires once per level.

### E3 — Traits

**Spec** 06 §5 · **Size** M

Build
- The `trait` type; add and remove; stacking modes; durations as timers;
  everything a trait confers; the hook `trait_adding`.

Done when
- Removing a trait removes exactly what it added.
- Durations behave across unloading as 03 §9 specifies.

### E4 — Bodies

**Spec** 06 §6.1–6.4 · **Size** L

Build
- `body_part`, `body_plan`, `body_function`; tissue extras on materials.
- The body instance, condition, functions, scale.
- Channel slots supplied by parts.

Done when
- Functions and slots equal recomputation (invariants B2, B3).
- Disabling a part interrupts an action using its slot.
- Doubling scale doubles thickness, quadruples area, multiplies mass by
  eight.

### E5 — Hits and wounds

**Spec** 06 §7 · **Size** L

Build
- `Hit`, `hit_incoming`, layer resolution, organs, severing, exits.
- Wounds; leaks and signals into pools; healing.
- Spread hits; conversion of world impact and exposure events into hits.
- Destruction and remains.

Done when
- At equal energy, a small contact area goes deeper than a large one.
- Hits below a layer's threshold never accumulate.
- A severing cut produces an item and a wound on the parent.
- Scenario `one_bad_cut`: one cut to an arm disables it, starts a drain on
  a pool, and the pool's threshold event fires.
- Scenario `through_the_wall`: a hit launches a creature, the wall breaks by
  02 §12, and the creature's injuries come from the impact events.

### E6 — Body editing

**Spec** 06 §6.5 · **Size** M

Build
- Every verb of 06 §6.5, with socket validation and recomputation.

Done when
- Invariants B2 and B3 hold after random sequences of edits (property
  test).
- Scenario `reshaping_touch`: a rule adds, removes, replaces and
  re-materials parts of a target.

### E7 — Minds and vital state

**Spec** 06 §8 · **Size** M

Build
- `Mind`; seated attributes and tracks; the four mind verbs; drivers.
- `die` and `revive` with `entity_dying`.

Done when
- A transferred mind takes control mode and mind-seated values and leaves
  body-seated ones.
- A driven body acts; a mindless undriven one does not.
- A cancelled `entity_dying` prevents death.

### E8 — Items and holds

**Spec** 06 §9 · **Size** L

Build
- The `item` type and component; surfaces; item damage; stacks.
- `Holds`, the holding verbs, capacity and grasp checks.
- Worn items as outer layers in hit resolution.
- Latent contents.

Done when
- A worn layer absorbs by the packet rule and is itself damaged.
- A held item is dropped when the holding part's slot is disabled.
- Latent contents are identical whenever they are first materialised.
- Invariant B6 holds under random holding operations.

### E9 — Templates and creation

**Spec** 06 §10 · **Size** L

Build
- `template` and `creature`; every choice kind; character specs.
- Validation of `assemble`; default and random selection.

Done when
- The same spec and seed give the same entity hash (invariant B4).
- Each choice kind has passing and failing fixtures.
- A fixture species with its own choices builds without engine changes.

### E10 — Movement

**Spec** 06 §11 · **Size** M

Build
- `movement`; `move`, `sweep`, `leap`, `set_altitude`, `teleport`; the
  occupancy rule; swapping places with a creature that yields, through
  `passage_requested`; loss of footing into a world body and back.

Done when
- Step legality is tested against a table of geometry cases.
- A leap lands where the analytic trajectory says.
- A swap relocates both creatures in one step and is refused where the
  hook does not grant it.
- Scenario `dash_through_wall`: a sweep at high speed strikes a wall as a
  body impact.

### E11 — Coming and going

**Spec** 06 §12 · **Size** L

Build
- `population`; anonymous instantiation; the notable triggers; storage
  with tracts; the agent record; `make_notable` and `make_anonymous`.

Done when
- Reloading a tract shows the same anonymous individuals (invariant B7).
- Each trigger of 06 §12.2 makes an individual notable.
- Stored individuals round-trip with identical hashes.

### E12 — Minimal devices

**Spec** 02 §5.5; 06 §9.2 · **Size** M

Build
- The `Device` component; promotion of an interactive element to a device
  on demand and demotion at default state; contents; the `Inside` hold.
- Enough interaction to open, close, put in, take out and hide inside.

Document 16 extends this with links, verbs and tools.

Done when
- Ten thousand unused lockers create no entities.
- Destroying an element with a device spills its contents and exposes
  anyone inside.

## Perception

### S1 — Light

**Spec** 15 §3.3 · **Size** M

Build
- `Cosmos::light`; sky spread; fixed sources; the per-chunk cache; moving
  sources at query time.

Done when
- The cache equals recomputation after edits (invariant P5).
- A room is lit near its windows and dark beyond the cap.

### S2 — Signals and senses

**Spec** 15 §3.1, §3.2, §4 · **Size** M

Build
- `signal_kind`, `sense`, `event_signal`; the three propagation models;
  delay; signatures; `emit_signal`; facing.

Done when
- Each model matches hand-computed cases, including blocking by materials.
- Scenario `slow_sound`: a sound arrives after distance over speed, and an
  entity on a fast clock gets there first.

### S3 — Detection

**Spec** 15 §5 · **Size** L

Build
- Awareness storage and update; the triggers of 15 §5.2; events;
  concealment through the existing factors.

Done when
- Results do not depend on evaluation order (invariant P4).
- A strong signal detects at once; a marginal one takes the time the rates
  imply.
- Scenario `the_locker`: a hidden character is not seen, is heard if it
  makes noise, and the searcher's track ends at the last known position.

### S4 — Percepts

**Spec** 15 §6 · **Size** M

Build
- The `perceived` event and its filters; `perceives`, `could_perceive`;
  use by window kinds.

Done when
- Scenario `heard_not_seen`: a rule on `perceived` sends a character to
  investigate a sound it could not see the cause of.
- A window is noticed only by candidates who perceive its source.

### S5 — Beliefs

**Spec** 15 §7 · **Size** M

Build
- Tracks and bounded memories; staleness; the properties of 15 §7.2.
- The linter check of 15 §7.3 and the `hidden` tag rule.

Done when
- Belief properties are `null` for the never-perceived.
- The linter flags a direct read of another entity's location in a rule
  attached to a creature.

### S6 — Personas

**Spec** 15 §8 · **Size** M

Build
- Personas, presentation, appearance; `identity_check`; the persona verbs;
  illusions.

Done when
- Scenario `frame_job`: witnesses' memories name the copied persona while
  the world's cause names the actor.
- Seeing through a disguise links the two personas in that observer only.

### S7 — Relations, groups, orders

**Spec** 15 §9, §10 · **Size** M

Build
- `relation_kind`, `group`; relation storage and fade; group fallback and
  claims; group knowledge; `order` and `order_received`.

Done when
- A member with no personal relation uses its group's; a claimed uniform
  applies the group's stance until seen through.
- Scenario `minion_orders`: a created servant obeys while the relation
  holds, must hear the order, and stops obeying when the relation is
  removed.

### S8 — Speech

**Spec** 15 §11 · **Size** M

Build
- `say`, `lines`, `message_kind`; `message_heard`; `understands`; reports
  transferring beliefs; exchanges; `talk_options`.

Done when
- A report gives the hearer the memory at reduced confidence and marks its
  source.
- Knowledge reaches a group and is used by a third member.

### S9 — Party perception

**Spec** 15 §13 · **Size** M

Build
- Tagging of output events by how the party perceived them; map memory; the
  omniscient switch.

Done when
- An event out of sight and earshot is tagged unperceived.
- Map memory records what was there when last seen, not what is there now.

### S10 — Coloured light (experimental)

**Spec** 15 §3.3; 12 §6.1; 04 §8.4 · **Size** S

This package is experimental. No milestone depends on it and no bundled
content may use it. Part of its purpose is to find out what it costs and
how it looks. Record what is found against the table of unresolved
questions in 15 §3.3.

Build
- `color` on light sources; `Cosmos::light_color` cached with light; the
  tint in cell drawing.
- Registration under the experimental feature `coloured_light`.

Done when
- A cell lit by two sources is tinted by their colours in proportion to
  what each contributes.
- State hashes and detection results are identical with the feature on
  and off.
- A mod that gives a light a colour without opting in fails to load, and
  one that opts in is warned.
- The cost of the cache is measured and written into 15 §17.

## Presentation

### U1 — Scene skeleton, frames, themes

**Spec** 12 §3, §4 · **Size** M

Build
- `clash-scene`: `Frame`, sprites, effects; glyph resolution; `theme`; the
  `Frontend` trait; the null frontend; the golden-frame harness.

Done when
- A frame serialises to text and compares with a stored one.
- The scene type exposes no route to the simulation except `submit`
  (invariant V1), shown by a compile-fail test.

Beta: built ahead of the simulation (K2). Stubbed: the scene's link to
the simulation is the `SceneHost` trait with `submit` and nothing else, so
there is no `SimView` and the compile-fail tests show only that the host
cannot be reached; they must be joined by tests against the real handle.
Definitions come from the mod loader (decision D-322):
`clash_scene::register_defs` registers `color`, `theme`, `keymap`,
`figure` and `screen`, and `UiDefs::from_defs` makes themes and scenes
from what loaded, by qualified id. The linter's warning for glyph states
that differ in colour alone (invariant V10) waits for the linter. A
glyph's `shape` is checked and can be read by facing; drawing by it is U4.

### U2 — Terminal frontend

**Spec** 12 §11 · **Size** S

Done when
- A frame is displayed in a terminal with colour mapping; sprites snap to
  cells; keys arrive as input events.

Done, with one caution: it was built and tested against an in-memory
writer and translated events, and has not yet been watched in a real
terminal. `cargo run -p clash-front-term --example gallery` is the thing
to try first.

### U3 — Window frontend

**Spec** 12 §11 · **Size** L

Build
- Window, glyph atlas from the theme font, grid and sprite quads, frame
  effects, keyboard and mouse with cell coordinates.

Done when
- The same frame renders in both frontends with the same characters in the
  same cells.
- The quad budget of 12 §15 is benchmarked.

### U4 — View model and world drawing

**Spec** 12 §5, §6 · **Size** L

Build
- The view model from map memory and percepts; cell composition; look-down;
  light and memory tint; subgrid compositing.
- `map_view`, `section_view`, `region_map`; camera.

Done when
- Golden frames for the test city at several storeys and in section.
- Unperceived changes do not appear; the omniscient mode shows them.

### U5 — Playback

**Spec** 12 §7 · **Size** L

Build
- Event ingestion; pacing; gliding and trails; `show` and `anim`; the log;
  skipping.

Done when
- After any scripted playback the view model equals a fresh one
  (invariant V4).
- Golden frames for `wall_punch` at fixed time steps.
- Every shown event has a log line (invariant V5).

### U6a — Toolkit: layout and widgets

**Spec** 12 §10.1–10.3 · **Size** L

Build
- The `screen` type; layout containers; every widget of 12 §10.3 with
  static content; focus and navigation.

Done when
- Layout fixtures at several sizes have golden frames.
- Each widget has a golden frame and a navigation test.

Beta: every container and widget is laid out, drawn, focused and
navigated with content written in the definition. Stubbed, each waiting
for the package that owns it: a property that is an expression is held
unparsed (`Prop::Bound`) and shown as a placeholder until an `Evaluator`
is supplied, and rows and options from a query are empty (U6b); effects
are held as written and handed back in `UiEvent`s, not run (U6b);
`map_view`, `section_view`, `region_map` and `chart_view` have their
place, focus and fields and draw an empty area unless given a
`ViewPainter` (U4); `log` shows a `MessageLog` that nothing fills yet
(U5); `slot` holds its name (U8); `console`, `canvas` and `trace_view`
have their place and focus only (tooling plan); rebuilding the screen
stack on reload (12 §10.3) is not built; a `number` is stepped, not
typed. Screens are made from loaded definitions by `UiDefs::screen` and
checked by the linter against every loaded theme; the figures and art a
screen names are plain text, not references the loader resolves, until
U6b (decision D-322).

### U6b — Toolkit: binding and UI effects

**Spec** 12 §10.4, §10.5 · **Size** M

Build
- Pure-context bindings with the names of 12 §10.4; re-evaluation; the
  perceived-only rule for ordinary screens; UI effects; the `Ui` command
  and `ui_callable`.

Done when
- A binding that would read truth about a non-party entity is a load error
  outside `debug` screens.
- A `run` effect appears in the input log and replays.

### U7 — Input, controls, pickers

**Spec** 12 §9 · **Size** M

Build
- `keymap`, `control`; `act` with `needed` and `all`; pickers; mouse
  behaviour; decision screens.

Done when
- One control starts an action from defaults in one key press; its precise
  twin asks for every parameter with defaults pre-filled.
- Rebinding a key changes behaviour without touching controls.

### U8 — Slots and theming

**Spec** 12 §10.6, §4.3 · **Size** S

Done when
- A `ui_part` from a second mod appears in a slot in order, subject to its
  condition; switching theme restyles every screen.

### U9 — Bundled screens

**Spec** 12 §10.7 · **Size** L

Build
- The `ui` mod's screens, including character creation driven by template
  data.

Done when
- Scripted input goes from title to a created character in play; golden
  frames along the way.
- A fixture species with every choice kind is fully usable on the creation
  screen without a new screen.

### U10 — Cutscenes

**Spec** 12 §8 · **Size** M

Build
- Cues from actions, definitions and the effect; shots; `release`; skip;
  tier and cooldown preferences.

Done when
- Scenario `cutscene_takedown`: golden frames; skipping yields the same
  final view model (invariant V6).

### U11 — Application

**Spec** 12 §3; 04 §3.3 · **Size** M

Build
- `clash-app`: the loop, profile boot, saves, preferences, frontend choice.

Done when
- Launching with a profile shows its boot screen; a session can be saved,
  quit and resumed.

### Z1 — Benchmarks and budget gate

**Spec** 06 §15, 15 §17, 12 §15 · **Size** S, recurring

## Scenario catalogue

Under `scenarios/people/`. Presentation scenarios also have golden frames.

| Scenario | Package | Serves | What it checks |
|----------|---------|--------|----------------|
| `one_bad_cut` | E5 | S14 | Wound to impairment to drain |
| `through_the_wall` | E5 | S1 | Launch, wall, impact injuries |
| `reshaping_touch` | E6 | S17 | Runtime body edits |
| `dash_through_wall` | E10 | S3 | Sweep as impact |
| `slow_sound` | S2 | S3 | Signal delay against a fast clock |
| `the_locker` | S3 | S13 | Hiding, last known position |
| `heard_not_seen` | S4 | S18 | Reaction to a percept |
| `frame_job` | S6 | S15 | Persona against cause |
| `minion_orders` | S7 | S16 | Command relation, orders must be heard |
| `cutscene_takedown` | U10 | S6 | Choreography over decided events |
| `creation_flow` | U9 | S20 | Data-driven character creation |
| `the_open_door` | S4 | — | Watched things and discrepancies |
| `a_tall_figure_in_red` | S6 | — | Features and descriptions |
| `the_larder` | E8 | — | Ageing |
| `gas_mask` | E2, E8 | — | Exposure by route |
| `behind_lead` | S2 | — | Signals that expose |

## Review additions

The owner's reviews of the design (decisions D-182 onward) added
the items below after the packages above were written. Each belongs to the
package named. Build it with that package and treat its "Done when" as part
of the package's. If the package is already `done`, take the addition as a
small package of its own.

| Addition | Package | Spec | Done when |
|----------|---------|------|-----------|
| Shared body parts | E4 | 06 §6.4 | A thousand unchanged bodies of one template store one set of parts; wounding one gives it its own copy and leaves the rest sharing. Measured in the E4 benchmark. |
| Stacks split and rejoin | E8 | 06 §9.1 | Damaging one of a stack of forty leaves a stack of thirty-nine and one item. Repairing it in the same hold lets them rejoin. Counts are conserved throughout. |
| Watched things and discrepancies | S4 | 15 §6; 10 §3.2 | Test script `the_open_door`: a guard who last saw a watched door shut receives a `discrepancy` percept on seeing it open, with what was expected and what was found; a passer-by with no routine at the site receives none. |
| Observed capabilities | S5 | 15 §7.1 | Seeing a persona use an attack records it on the track with its strongest result; the list never exceeds `track_capabilities`; being told of a capability records it as told. |
| Features and descriptions | S6 | 15 §8.4 | Test script `a_tall_figure_in_red`: a glimpse at low clarity yields height and dress only; `suspects` lists every known persona that matches; a later close look adds the face and merges the stranger's track into the right persona. Changing clothes changes `dress` alone. |
| Relations that intervene | S7 | 15 §9.1; 03 §7.1 | A relation kind flagged `intervenes` makes its holder a window candidate. Covered by sim-kernel test script `bodyguard`, run here with real relations. |
| Sound cue names | U5 | 12 §7.3 | A `show` definition's `sound` loads, is validated as a name, appears in the event the frontend receives, and plays nothing. |
| Several at once | U7 | 12 §9.6 | With three controlled characters selected, one move command gives each its own action in entity order, and one answer settles the same pending decision for all three. |
| Roles that spend | S7 | 15 §9.2; 16 §5.6 | A role flagged `spends` is recorded on the group definition and readable by rules. Used by devices package G13. |
| Ageing | E8 | 06 §9.4 | Test script `the_larder`: meat in the open reaches its stage and turns at the times its rate gives; the same meat in a cold box lasts longer by the stated factor; when the box loses power the wake-up moves earlier. Two items in the same stage stack. Needs temperature (vehicles package V1); until then it is supplied by a test fixture. |
| Overlays | U4 | 12 §6.2 | A `map_view` draws an overlay defined by a test mod. Ordinary overlays read the view model only. |
| Posture | E10 | 06 §11.1 | Changing posture scales height, bands, speed and signature as defined; a posture that holds leg slots makes leg actions unavailable; a body with nobody acting for it takes its `limp` posture; `set_posture` forces one. |
| Exposure by route | E2, E8 | 02 §16; 06 §9.1 | Test script `gas_mask`: in a gas that exposes by breath, an unprotected character's pool rises, a masked one's rises by the share the mask lets through, one in a sealed suit's does not rise until the suit's air is gone, and a body that does not breathe is untouched. |
| Signals that expose | S2 | 15 §3.1 | Test script `behind_lead`: a standing source adds to the pool of someone in the open by strength over distance, and adds less behind a wall by that wall's blocking. With nobody in reach the emitter schedules nothing. |
| Looks on traits and postures | E3, E10 | 06 §5, §11.1; 12 §4.4 | A trait and a posture may carry a `look`; it is reported with the events that give and remove them. |
| Colour and unknown names on items | E8 | 06 §9.1 | An item made from a definition with a list of colours carries one of them, chosen by positional seed; `set_color` changes it; `unknown_name` is stored for text. |
| Pronouns | S6 | 15 §8.1; 05 §4.6 | A persona carries a pronoun set from its template or a creation choice; a second persona of the same character may carry another. |
| Colours, fallbacks and inline styles | U1, U2 | 12 §4.3 | `color` definitions load with a word and a fallback; the terminal frontend uses the fallback; `<name>…</>` styles text; the linter warns on glyph states that differ in colour alone (invariant V10). |
| Looks, shapes, own colour and ambience | U4, U5 | 12 §4.1, §4.4, §6.4 | Frame tests: a holder of a look is drawn with it from the moment the event is played and only to viewers with the needed sense; a creature of several cells is drawn by its shape for its facing; two cloaks of one definition are drawn in their own colours, and a witness's description names the same colour; rain is drawn only on cells known to be open to the sky. |
| Route picker | U7 | 12 §9.3 | Frame test: the picker draws a numbered path as steps are added, shows the parameter's `check` beside the step that fails it, lets steps be taken back, and submits the list in order. |
| Presentation testing | U1 | 12 §12 | Golden frames as text for the bundled screens; the view model equals a fresh one after every scripted playback; a test asserts the scene reaches the simulation only through `submit`. |
| Perception at lower detail | S3 | 15 §14 | A warm entity does not sweep; a signal strong enough promotes it; a cold agent keeps its beliefs unchanged while stored. |
| Region map and chart views | U4 | 12 §6.2 | The `region_map` and `chart_view` widgets draw from what the party knows. `chart_view` needs world-model package W21. |
| Widget fields, roles, preferences, overlays | U6a, U6b, U8 | 12 §6.2, §10.3, §10.4, §10.7 | Every widget's fields are as tabled; a profile without a screen for a role fails to load; a preference defined by a test mod appears in options and is readable as `app.pref`; an overlay defined by a test mod draws only what the party has perceived. |
| Screens asked for by rules | U6b | 12 §10.5 | `show_screen` opens a screen for the player who controls `to`, with its values, when playback reaches it; for a character nobody controls it does nothing. |
| Audit additions | every package | decisions D-234 to D-242 | Each package builds what its sections of the specification now state, including the hooks, verbs, events, refusals, fields and settings the pre-build audit added. The reference-coverage test of tooling package TL9 fails for anything registered without its arguments or fields. |
