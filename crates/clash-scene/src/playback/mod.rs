//! Playback: turning what the simulation reports into what the player
//! sees and reads (12 §7).
//!
//! The simulation is never waited on. It runs ahead and hands the scene
//! what happened; the scene plays it in order over real time; and until
//! an event is played, the picture of the world stays as it was.
//!
//! # For the application
//!
//! | Call | When |
//! |------|------|
//! | [`Scene::push_events`] | After each `advance` of the simulation, with what `drain_events` gave: one batch is one stretch between two decisions |
//! | [`Scene::advance`] | Every frame, with the real time that passed: it plays what falls due |
//! | [`Scene::playback_caught_up`] | To know whether the picture is of the present, so that input applies to it |
//! | [`Scene::skip_playback`] | To jump to the end. A key press does it by itself (below) |
//! | [`Scene::check_caught_up`] | In debug builds, once caught up, with a freshly initialised view model (12 §5) |
//!
//! ```no_run
//! # use clash_scene::{Scene, SceneHost};
//! # fn frame<H: SceneHost>(scene: &mut Scene<H>, drained: Vec<clash_sim::SimEvent>, seconds: f64) {
//! scene.push_events(drained);
//! scene.advance(seconds);
//! if scene.playback_caught_up() {
//!     // The picture is of the present: a decision can be asked for.
//! }
//! # }
//! ```
//!
//! # What playing an event does
//!
//! Nothing in this module knows any event. How an event is shown is
//! data: `show` definitions ([`ShowDef`]) by the event's name, each with
//! a condition, the changes it makes to the picture, an animation
//! ([`AnimDef`]), a log line worded for the reader, what to do if the
//! event was only heard, how long it holds the queue, how much it
//! matters and a sound cue. Several may match one event, and all apply.
//!
//! An event that no `show` gives changes for is applied silently:
//! everything it is about is observed again. Each cell among its
//! subjects is asked of the host ([`SceneHost::seen_cell`]) and each
//! entity too ([`SceneHost::seen_sprite`]).
//!
//! # Belief, not truth
//!
//! The scene does not decide what the party perceived: the host does
//! ([`SceneHost::perceived`]). An event that was not perceived is
//! dropped when it arrives. One that was only heard puts a marker where
//! it seemed to come from and writes a line, if a `show` says what was
//! heard, and changes no cell and no sprite. A `show`'s texts and
//! condition are compiled as a mind context and evaluated through the
//! host's reader, so a line names others only as the viewing character
//! knows them (05 §4.6).
//!
//! Everything a step needs is asked of the host when the batch arrives,
//! and playing it later asks nobody anything. So a cell is drawn as the
//! party saw it when its batch ended, which for a cell that changed
//! twice in one batch is its last state both times: events do not carry
//! what a cell holds after them. The last change a batch makes to a
//! sprite leaves it as the party perceives it when the batch ends, and
//! one no longer perceived leaves the picture at the last event that
//! moves it. That is what makes caught-up playback equal a fresh view
//! ([`check_caught_up`]).
//!
//! # The key that skips
//!
//! While anything is being played, a key press jumps playback to its
//! end before the key is used as usual. If the keymap in force gives
//! some key the name `skip` ([`NAV_SKIP`]), only that key does, and it
//! does nothing else.
//!
//! # What is not built
//!
//! - Floating text, an `anim`'s particles and effects shaped by an
//!   event's fields (12 §7.3).
//! - Looks (12 §4.4): no event reports one yet.
//! - The pacing constants other than `stretch_time` are the engine's,
//!   and `stretch_time` is set from Rust ([`Playback::set_stretch_time`])
//!   until settings exist.
//! - Events carry no duration, so a sprite glides for its show's `time`
//!   and not for as long as the move took (12 §7.2), and bodies in
//!   flight are drawn where they start, strike and land, not along
//!   their paths, which no event carries.
//! - Cutscenes (package U10) will choose when the pending events of
//!   their span are played; the queue has no way to be told yet.
//! - `message` effects of rules reach the log through
//!   [`Scene::log_mut`] once the simulation reports them.

pub mod def;
pub mod log;
pub mod queue;
pub mod shows;

use core::fmt;

use clash_core::Tick;
use clash_expr::{
    DefRef, FunctionId, Handle, Host, PropertyId, Registry, RuntimeWarning, Value as ExprValue,
};
use clash_sim::{FieldValue, Kinds, SimEvent, Subject};
use clash_world::Location;

pub use def::{
    AnimDef, AnimFrameDef, AnimId, ChangeDef, HeardDef, SUBJECT_HERE, ShowDef, ShowId,
    SpriteChangeDef, is_cue_name,
};
pub use log::{CATEGORY_EVENT, Importance, LOG_LIMIT_DEFAULT, LogLine, MessageLog};
pub use queue::{
    CATCH_UP_DEFAULT, FLASH_STRENGTH, FrameEffects, HEARD_TIME_DEFAULT, MAX_GAP_DEFAULT, Playback,
    Played, STRETCH_TIME_DEFAULT, SoundCue, TRAIL_FROM_CELLS,
};
pub use shows::{Anim, AnimError, AnimFrame, EFFECT_TIME_DEFAULT, Show, Shows};

use crate::scene::{Scene, SceneHost};
use crate::text::RichText;
use crate::theme::Theme;
use crate::ui::PreferenceDef;
use crate::view::{HeardMarker, SpriteId, ViewDifference, ViewModel};
use queue::{Effect, Line, Op, Step, Target};

/// The name a keymap gives the key that jumps playback to its end
/// (12 §7.1).
pub const NAV_SKIP: &str = "skip";

/// The bare name of the preference that scales playback: 1 is the usual
/// pace and 0 is instant.
pub const PREF_PLAYBACK_SPEED: &str = "playback_speed";

/// The bare name of the preference that says which shows' animations
/// play: `all`, `major` or `none`.
pub const PREF_EFFECTS: &str = "effects";

/// How the directly controlled characters perceived an event (15 §13).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Perceived {
    /// Not at all: the event is dropped.
    #[default]
    Not,
    /// Only heard: a marker where it seemed to come from and a line,
    /// never a picture.
    Heard {
        /// Where it seemed to come from.
        from: Location,
    },
    /// Seen: it is played into the picture.
    Seen,
}

/// How a played view model differs from a freshly initialised one: what
/// [`Scene::check_caught_up`] reports (12 §5, invariant V4).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewDrift {
    /// The first differences found, in order of grid and cell and then
    /// of sprite.
    pub differences: Vec<ViewDifference>,
    /// Whether there are more than were listed.
    pub more: bool,
}

/// How many differences a [`ViewDrift`] lists.
pub const DRIFT_LISTED: usize = 8;

impl fmt::Display for ViewDrift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the played picture is not what a fresh one would be:")?;
        for difference in &self.differences {
            write!(f, "\n  {difference}")?;
        }
        if self.more {
            f.write_str("\n  and more")?;
        }
        Ok(())
    }
}

impl std::error::Error for ViewDrift {}

/// Compares a view model that playback has kept up to date with one
/// freshly initialised from what the party remembers and perceives now
/// (12 §5). They must hold the same cells, sprites and grid shapes.
/// Markers of what was only heard are not compared: they are shown for
/// a while and are not knowledge of the world.
pub fn check_caught_up(played: &ViewModel, fresh: &ViewModel) -> Result<(), ViewDrift> {
    let mut differences = played.differences(fresh, DRIFT_LISTED + 1);
    if differences.is_empty() {
        return Ok(());
    }
    let more = differences.len() > DRIFT_LISTED;
    differences.truncate(DRIFT_LISTED);
    Err(ViewDrift { differences, more })
}

/// What an event binds in the scope its shows were compiled in: a value
/// for each subject and each field its registration lists, in order.
struct EventValues {
    subjects: Vec<ExprValue>,
    fields: Vec<ExprValue>,
}

impl EventValues {
    fn of(event: &SimEvent, registry: &Registry, kinds: Option<&Kinds>) -> EventValues {
        let mut values = EventValues {
            subjects: Vec::new(),
            fields: Vec::new(),
        };
        let Some(registered) = registry
            .event_id(event.name())
            .and_then(|id| registry.event(id))
        else {
            return values;
        };
        let value = |subject: Subject| kinds.map_or(ExprValue::Null, |kinds| kinds.value(subject));
        for (name, _) in &registered.spec.subjects {
            values
                .subjects
                .push(event.subject(name).map_or(ExprValue::Null, value));
        }
        for field in &registered.spec.fields {
            values.fields.push(match event.body.field(&field.name) {
                None | Some(FieldValue::Null) => ExprValue::Null,
                Some(FieldValue::Plain(plain)) => ExprValue::from(&plain),
                Some(FieldValue::Subject(subject)) => value(subject),
                // A kind of value added later: not readable here yet.
                Some(_) => ExprValue::Null,
            });
        }
        values
    }
}

/// What the machine asks while a `show`'s text or condition runs: the
/// event's own values from here, everything else from the host's
/// reader.
struct ShowHost<'a> {
    game: &'a mut dyn Host,
    values: &'a EventValues,
    registry: &'a Registry,
    warnings: &'a mut u64,
}

impl Host for ShowHost<'_> {
    fn subject(&mut self, slot: u16) -> ExprValue {
        self.values
            .subjects
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn local(&mut self, _slot: u16) -> ExprValue {
        ExprValue::Null
    }

    fn event_field(&mut self, slot: u16) -> ExprValue {
        self.values
            .fields
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[ExprValue]) -> ExprValue {
        self.game.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> ExprValue {
        self.game.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[ExprValue]) -> ExprValue {
        self.game.call(function, args)
    }

    fn rng(&mut self) -> Option<&mut clash_core::Rng> {
        // What is shown is pure.
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {
        *self.warnings += 1;
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.registry.def_name(id)
    }
}

/// Makes the last change a batch makes to each sprite leave it as the
/// party perceives it when the batch ends, which is what the host said
/// for every step: one that ends perceived ends where it is perceived,
/// and one that is no longer perceived leaves the picture at the last
/// event that moves it. Until then it is drawn moving where the events
/// say (12 §5).
fn settle_sprites(steps: &mut [Step]) {
    let mut last: std::collections::BTreeMap<SpriteId, (usize, usize)> =
        std::collections::BTreeMap::new();
    for (index, step) in steps.iter().enumerate() {
        for (at, op) in step.ops.iter().enumerate() {
            if let Op::Sprite { id, .. } = op {
                last.insert(*id, (index, at));
            }
        }
    }
    for (index, at) in last.into_values() {
        let op = &mut steps[index].ops[at];
        match op {
            Op::Sprite {
                look: Some(_), to, ..
            } => *to = None,
            Op::Sprite {
                id,
                look: None,
                to: Some(_),
                ..
            } => *op = Op::Gone { id: *id },
            _ => {}
        }
    }
}

/// A rendered line as the log holds it: styled by its tags where they
/// are sound, and with a capital first letter (05 §4.6).
fn finished_line(rendered: &str, theme: &Theme) -> Option<RichText> {
    if rendered.trim().is_empty() {
        return None;
    }
    let mut text =
        RichText::parse(rendered, theme).unwrap_or_else(|_| RichText::plain_text(rendered));
    text.capitalize_first();
    Some(text)
}

impl<H: SceneHost> Scene<H> {
    /// Hands the scene one batch of what the simulation reports having
    /// happened, in the order it happened: what `drain_events` gave
    /// after one stretch between two decisions (12 §7.1). Nothing is
    /// shown yet: [`Scene::advance`] plays it.
    ///
    /// Each event is asked of the host here, once: whether the party
    /// perceived it, and for one that was seen, what they see of the
    /// cells and the things it is about. What was not perceived is
    /// dropped.
    pub fn push_events(&mut self, batch: impl IntoIterator<Item = SimEvent>) {
        let mut steps = Vec::new();
        for event in batch {
            if let Some(step) = self.step_for(&event) {
                steps.push(step);
            }
        }
        settle_sprites(&mut steps);
        self.playback.push(steps);
        if !self.playback.is_caught_up() {
            // Bindings on the view show the moment playback has
            // reached, which is no longer the present.
            self.stale = true;
        }
    }

    /// What playing one event will do, or `None` if it does nothing:
    /// it was not perceived, or nothing says how it shows and it
    /// changes nothing the party sees.
    fn step_for(&mut self, event: &SimEvent) -> Option<Step> {
        let Scene {
            host,
            world,
            ui,
            shows,
            theme,
            runner,
            prefs,
            warnings,
            pref_effects,
            ..
        } = self;
        let empty = ViewModel::new();
        let picture = world.as_ref().map_or(&empty, |world| world.model());
        let perceived = host.perceived(event, picture);
        if perceived == Perceived::Not {
            return None;
        }
        let seen = perceived == Perceived::Seen;
        let mut step = Step {
            tick: event.tick,
            ..Step::default()
        };

        // The shows that apply: about this event, passing their filter
        // and their condition.
        let mut matched: Vec<(&Show, &ShowDef)> = Vec::new();
        let mut values = None;
        if let Some(ui) = ui.as_ref() {
            let defs = ui.defs();
            let exprs = defs.exprs();
            let registry = exprs.registry();
            let kinds = Kinds::of(registry);
            for show in shows.for_event(event.name()) {
                let def = defs.get::<ShowDef>(show.id);
                let passes = def.filter.iter().all(|(field, wanted)| {
                    def::filter_matches(event.body.field(field).as_ref(), &wanted.value.0)
                });
                if !passes {
                    continue;
                }
                let values =
                    values.get_or_insert_with(|| EventValues::of(event, registry, kinds.as_ref()));
                if let Some(when) = &def.when {
                    let mut reader = ShowHost {
                        game: host.reader(),
                        values,
                        registry,
                        warnings,
                    };
                    if !when.holds(&mut runner.over(exprs, &mut reader)) {
                        continue;
                    }
                }
                matched.push((show, def));
            }
            // The lines: what was seen, or what was heard.
            for (show, def) in &matched {
                let text = if seen {
                    def.log.as_ref()
                } else {
                    def.heard.as_ref().and_then(|heard| heard.log.as_ref())
                };
                let (Some(text), Some(values)) = (text, values.as_ref()) else {
                    continue;
                };
                let mut reader = ShowHost {
                    game: host.reader(),
                    values,
                    registry,
                    warnings,
                };
                let rendered = text.render(&mut runner.over(exprs, &mut reader));
                if let Some(text) = finished_line(&rendered, theme) {
                    step.lines.push(Line {
                        category: def
                            .category
                            .clone()
                            .unwrap_or_else(|| CATEGORY_EVENT.to_owned()),
                        text,
                        importance: show.importance,
                    });
                }
            }
        }

        let place = event
            .subject(SUBJECT_HERE)
            .and_then(|subject| subject.loc());
        for (_, def) in &matched {
            if let Some(sound) = &def.sound
                && (seen || def.heard.is_some())
            {
                step.cues.push(SoundCue {
                    name: sound.value.clone(),
                    tick: event.tick,
                    at: place,
                    heard: !seen,
                });
            }
        }

        if let Perceived::Heard { from } = perceived {
            // A marker and a line, never a picture.
            if let Some(heard) = matched.iter().find_map(|(_, def)| def.heard.as_ref()) {
                step.heard = Some(HeardMarker {
                    loc: from,
                    ch: heard.marker,
                });
            }
            return (!step.is_empty()).then_some(step);
        }

        // Which shows may play their animations: the player's choice.
        let chosen = pref_effects
            .as_deref()
            .and_then(|id| prefs.get(id))
            .and_then(|value| match value {
                clash_core::Value::Str(text) => Some(text.as_str()),
                _ => None,
            });
        let plays = |importance: Importance| match chosen {
            Some("none") => false,
            Some("major") => importance == Importance::Major,
            _ => true,
        };

        // The changes: what the shows say, or else everything the event
        // is about.
        let loc_of = |name: &str| event.subject(name).and_then(|subject| subject.loc());
        let entity_of = |name: &str| event.subject(name).and_then(|subject| subject.entity());
        let mut ops: Vec<Op> = Vec::new();
        let mut add = |op: Op| {
            if !ops.contains(&op) {
                ops.push(op);
            }
        };
        let mut said = false;
        for (show, def) in &matched {
            let Some(changes) = &def.change else {
                continue;
            };
            said = true;
            let glide = if plays(show.importance) {
                show.time
            } else {
                0.0
            };
            for change in changes {
                match change {
                    ChangeDef::Cell(name) => {
                        if let Some(at) = loc_of(&name.value)
                            && let Some(contents) = host.seen_cell(at)
                        {
                            add(Op::Cell { at, contents });
                        }
                    }
                    ChangeDef::Field(name) => {
                        if let Some(at) = loc_of(&name.value)
                            && let Some(contents) = host.seen_cell(at)
                        {
                            add(Op::Field {
                                at,
                                field: contents.field,
                            });
                        }
                    }
                    ChangeDef::Sprite(sprite) => {
                        if let Some(entity) = entity_of(&sprite.of.value)
                            && let Some((id, look)) = host.seen_sprite(entity)
                        {
                            let to = sprite.to.as_ref().and_then(|to| loc_of(&to.value));
                            add(Op::Sprite {
                                id,
                                look,
                                to,
                                glide,
                            });
                        }
                    }
                    ChangeDef::Gone(name) => {
                        if let Some(entity) = entity_of(&name.value)
                            && let Some((id, _)) = host.seen_sprite(entity)
                        {
                            add(Op::Gone { id });
                        }
                    }
                }
            }
        }
        if !said {
            for (_, subject) in event.subjects() {
                if let Some(at) = subject.loc() {
                    if let Some(contents) = host.seen_cell(at) {
                        add(Op::Cell { at, contents });
                    }
                } else if let Some(entity) = subject.entity()
                    && let Some((id, look)) = host.seen_sprite(entity)
                {
                    add(Op::Sprite {
                        id,
                        look,
                        to: None,
                        glide: 0.0,
                    });
                }
            }
        }
        step.ops = ops;

        // The effects and the hold.
        let target = |name: &str, host: &mut H| -> Option<Target> {
            match event.subject(name)? {
                Subject::Entity(entity) => {
                    host.seen_sprite(entity).map(|(id, _)| Target::Sprite(id))
                }
                other => other.loc().map(Target::Cell),
            }
        };
        for (show, def) in &matched {
            step.hold = step.hold.max(show.min_time);
            if !plays(show.importance) {
                continue;
            }
            let anim = show.anim.as_ref().and_then(|anim| {
                let at = def.at.as_ref().map_or(SUBJECT_HERE, |at| at.value.as_str());
                let at = target(at, host)?;
                let to = def.to.as_ref().and_then(|to| target(&to.value, host));
                Some((anim.clone(), at, to))
            });
            let effect = Effect {
                anim,
                shake: def.shake.clamp(0.0, 1.0) as f32,
                flash: show.flash,
                time: show.time,
            };
            if effect.anim.is_some() || effect.shake > 0.0 || effect.flash.is_some() {
                step.effects.push(effect);
            }
        }
        (!step.is_empty()).then_some(step)
    }

    /// Whether every event handed over has been played: the picture of
    /// the world is of the present, and input applies to it.
    /// Animations may still be finishing: see [`Playback::is_idle`].
    pub fn playback_caught_up(&self) -> bool {
        self.playback.is_caught_up()
    }

    /// Jumps playback to its end: everything queued is played now and
    /// every animation in progress ends (12 §7.1).
    pub fn skip_playback(&mut self) {
        let played = self.playback.skip(self.world.as_mut(), &mut self.log);
        if played.changed || played.events > 0 {
            self.stale = true;
        }
    }

    /// The moment playback has reached, while it is behind the
    /// simulation: the instant before the next event to be played.
    /// `None` when it has caught up. Bindings on the view show the
    /// world as of this moment (12 §10.4): the host is told it before
    /// they are evaluated ([`SceneHost::as_of`]).
    pub fn playback_moment(&self) -> Option<Tick> {
        self.playback.moment()
    }

    /// The playback queue: what is waiting, how it is paced.
    pub fn playback(&self) -> &Playback {
        &self.playback
    }

    /// The playback queue, to set its pacing or which sprite the camera
    /// follows.
    pub fn playback_mut(&mut self) -> &mut Playback {
        &mut self.playback
    }

    /// The shows in force: the `show` definitions under the theme in
    /// use.
    pub fn shows(&self) -> &Shows {
        &self.shows
    }

    /// Takes the sound cues of the events played since this was last
    /// called (12 §7.3). No frontend plays them yet.
    pub fn take_sound_cues(&mut self) -> Vec<SoundCue> {
        self.playback.take_cues()
    }

    /// How fast playback runs: the preference `playback_speed`, 1
    /// where there is none. Zero is instant.
    pub fn playback_speed(&self) -> f64 {
        let speed = self
            .pref_speed
            .as_deref()
            .and_then(|id| self.prefs.get(id))
            .and_then(|value| match value {
                clash_core::Value::Int(int) => Some(*int as f64),
                clash_core::Value::Real(real) => Some(real.get()),
                _ => None,
            });
        speed.filter(|speed| speed.is_finite()).unwrap_or(1.0)
    }

    /// The check of 12 §5: the picture playback has kept up to date
    /// must equal `fresh`, a view model initialised just now from what
    /// the party remembers and perceives. Call it once playback has
    /// caught up; a debug build of the application does so after every
    /// stretch. Markers of what was only heard are not compared. A
    /// scene with no view of the world has nothing to compare.
    pub fn check_caught_up(&self, fresh: &ViewModel) -> Result<(), ViewDrift> {
        match &self.world {
            Some(world) => check_caught_up(world.model(), fresh),
            None => Ok(()),
        }
    }

    /// Finds the preferences playback reads, by their bare names, and
    /// makes the shows for the theme in force. Called when the scene is
    /// given its definitions and when its theme changes.
    pub(crate) fn ready_playback(&mut self) {
        let Some(ui) = &self.ui else {
            return;
        };
        self.pref_speed = ui.resolve::<PreferenceDef>(PREF_PLAYBACK_SPEED, "");
        self.pref_effects = ui.resolve::<PreferenceDef>(PREF_EFFECTS, "");
        // What is wrong with a show is the linter's to report
        // ([`crate::check_defs`]): here what can be played is.
        self.shows = ui.shows(&self.theme).0;
    }
}
