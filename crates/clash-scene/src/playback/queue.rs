//! [`Playback`]: the queue of what happened, played over real time
//! (12 §7.1, §7.2).
//!
//! The simulation is never waited on. It runs ahead and hands over what
//! happened in batches, a batch being one stretch between two
//! decisions; each event of a batch has already been turned into a
//! step, which says what playing it does. The queue plays the steps
//! in order as real time passes, and until a step is played the picture
//! of the world stays as it was.
//!
//! # Pacing
//!
//! Steps of one tick are played together: they are one **group**. A
//! stretch is laid out when it arrives:
//!
//! 1. The simulated time between one group and the next counts for at
//!    most [`Playback::max_gap`] (two simulated seconds unless set): a
//!    long wait is not waited through.
//! 2. What is left is scaled so that the stretch takes
//!    [`Playback::stretch_time`] of real time (350 ms unless set) from
//!    its first group that shows something to its last, however much
//!    simulated time it covers. A stretch with one such group, or none,
//!    takes no time: it is played at once.
//! 3. A group holds the queue for the longest `min_time` among its
//!    steps: nothing later is played before that has passed.
//!
//! A step that only changes the picture, with no line, no effect, no
//! marker and no hold, is **silent**. Silent steps never make a stretch
//! longer: they are played with what shows around them.
//!
//! Real time is scaled by a speed, the `playback_speed` preference:
//! 2 is twice as fast, and 0 is **instant**, which plays everything
//! queued at once and starts no animation. When more is queued than
//! [`Playback::catch_up`] of real time (one second unless set) would
//! play, the queue runs faster in proportion, so that however far the
//! simulation has run ahead, playback catches up.
//!
//! [`Playback::skip`] plays everything queued now and ends every
//! animation: what the key named `skip` does.
//!
//! # What is drawn
//!
//! Effects in progress are not part of the picture. Each frame they are
//! written into the view's [`ViewFx`](crate::view::ViewFx) and the frame's
//! [`FrameFx`](crate::FrameFx) by [`Playback::render`]: a sprite on its
//! way between two cells, with afterimages behind it when it crosses
//! several cells at once; an animation's glyph on a cell, or travelling
//! from one cell to another; shake; a flash.

use std::collections::VecDeque;
use std::sync::Arc;

use clash_core::{Dur, Tick};
use clash_world::Location;

use super::log::{Importance, MessageLog};
use super::shows::{Anim, MICROS_PER_SECOND};
use crate::color::Color;
use crate::frame::Wash;
use crate::glyph::{GlyphInputs, resolve_part};
use crate::text::RichText;
use crate::view::{
    CellContents, CellFx, EntitySprite, FieldSeen, HeardId, HeardMarker, SpriteId, WorldSprite,
    WorldView,
};

/// The real time a stretch takes unless set, in seconds: the default of
/// the setting `stretch_time` (12 §7.1).
pub const STRETCH_TIME_DEFAULT: f64 = 0.35;

/// The most simulated time between two groups of a stretch that is
/// played out, unless set: the rest of a longer wait is skipped.
pub const MAX_GAP_DEFAULT: Dur = Dur(2_000_000);

/// The real time within which playback has always nearly caught up,
/// unless set, in seconds: a longer backlog plays faster in proportion.
pub const CATCH_UP_DEFAULT: f64 = 1.0;

/// How long the marker of something only heard stays, unless set, in
/// seconds of real time.
pub const HEARD_TIME_DEFAULT: f64 = 1.5;

/// How strongly a flash shows when it begins. It dies away from there.
pub const FLASH_STRENGTH: f32 = 0.6;

/// How many cells a sprite must cross in one move to leave afterimages
/// behind it (12 §7.2).
pub const TRAIL_FROM_CELLS: f32 = 2.0;

/// The afterimages of a sprite that crosses several cells at once: how
/// far behind it each is, as a share of the move, and how faint.
const TRAIL: [(f32, f32); 3] = [(0.15, 0.6), (0.3, 0.4), (0.45, 0.2)];

/// The frame layer gliding sprites are drawn on; their afterimages are
/// one below, and travelling animations one above.
const LAYER_GLIDE: u8 = 2;

/// A sound a `show` names (12 §7.3), handed on when its event is
/// played. Nothing plays it yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundCue {
    /// The cue's name.
    pub name: String,
    /// The tick of the event.
    pub tick: Tick,
    /// Where it came from, if the event has a place.
    pub at: Option<Location>,
    /// Whether the event was only heard.
    pub heard: bool,
}

/// One change to the picture of the world.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Op {
    /// A cell is observed again.
    Cell {
        /// The cell.
        at: Location,
        /// What the party sees there.
        contents: CellContents,
    },
    /// The strongest field of a known cell changed.
    Field {
        /// The cell.
        at: Location,
        /// The field seen there now.
        field: Option<FieldSeen>,
    },
    /// A perceived thing moved, appeared, changed or is no longer
    /// perceived.
    Sprite {
        /// The number its sprite is known by.
        id: SpriteId,
        /// How it looks, if the party perceives it.
        look: Option<EntitySprite>,
        /// The cell the event says it is in now.
        to: Option<Location>,
        /// How long its move is drawn for, in seconds. Zero: it jumps.
        glide: f64,
    },
    /// A sprite is removed.
    Gone {
        /// The number its sprite is known by.
        id: SpriteId,
    },
}

/// Where an effect plays: a cell, or wherever a sprite is when the
/// effect begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A cell.
    Cell(Location),
    /// The cell a sprite is in.
    Sprite(SpriteId),
}

/// What a step shows beside its changes and its lines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Effect {
    /// An animation, where it plays and, if it travels, where to.
    pub(crate) anim: Option<(Arc<Anim>, Target, Option<Target>)>,
    /// Screen shake as it begins, 0 to 1.
    pub(crate) shake: f32,
    /// A colour flashed over the frame.
    pub(crate) flash: Option<Color>,
    /// How long the shake and the flash take to die away, in seconds.
    pub(crate) time: f64,
}

/// A line for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) category: String,
    pub(crate) text: RichText,
    pub(crate) importance: Importance,
}

/// What playing one event does. It was worked out when the event
/// arrived: playing it asks nobody anything.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct Step {
    /// When the event happened.
    pub(crate) tick: Tick,
    /// The changes to the picture.
    pub(crate) ops: Vec<Op>,
    /// The lines written to the log.
    pub(crate) lines: Vec<Line>,
    /// The effects begun.
    pub(crate) effects: Vec<Effect>,
    /// How long it holds the queue, in seconds.
    pub(crate) hold: f64,
    /// The marker put where something only heard seemed to come from.
    pub(crate) heard: Option<HeardMarker>,
    /// The sound cues handed on.
    pub(crate) cues: Vec<SoundCue>,
}

impl Step {
    /// Whether playing the step does nothing at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.is_silent() && self.cues.is_empty()
    }

    /// Whether the step shows nothing but a changed picture: no line,
    /// no effect, no marker, no hold.
    pub(crate) fn is_silent(&self) -> bool {
        self.lines.is_empty()
            && self.effects.is_empty()
            && self.heard.is_none()
            && self.hold <= 0.0
            && !self
                .ops
                .iter()
                .any(|op| matches!(op, Op::Sprite { glide, .. } if *glide > 0.0))
    }
}

/// The steps of one tick, and when they are due within their stretch.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    tick: Tick,
    /// Seconds from the start of the stretch, at the usual pace.
    due: f64,
    steps: Vec<Step>,
}

/// One batch: what happened between two decisions.
#[derive(Clone, Debug, PartialEq)]
struct Stretch {
    groups: VecDeque<Group>,
    /// How far into the stretch playback is, in seconds at the usual
    /// pace.
    clock: f64,
    /// When the stretch is over: its last group played and its hold
    /// passed.
    end: f64,
}

/// What an effect in progress is.
#[derive(Clone, Debug, PartialEq)]
enum Running {
    /// A sprite on its way from a cell to where it now is.
    Glide { id: SpriteId, from: Location },
    /// An animation at a cell, or on its way to another.
    Anim {
        anim: Arc<Anim>,
        at: Location,
        to: Option<Location>,
    },
    /// Screen shake of a strength, dying away.
    Shake(f32),
    /// A flash of a colour, dying away.
    Flash(Color),
}

/// An effect in progress.
#[derive(Clone, Debug, PartialEq)]
struct Active {
    what: Running,
    /// Seconds since it began, at the usual pace.
    age: f64,
    /// How long it lasts.
    time: f64,
}

impl Active {
    /// How much of it has passed, 0 to 1.
    fn share(&self) -> f32 {
        if self.time <= 0.0 {
            1.0
        } else {
            (self.age / self.time).clamp(0.0, 1.0) as f32
        }
    }
}

/// What a frame is given beside its cells and sprites: see
/// [`Playback::render`].
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FrameEffects {
    /// Screen shake, 0 to 1.
    pub shake: f32,
    /// A flash of colour over everything.
    pub flash: Option<Wash>,
}

/// What an advance of the queue did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Played {
    /// How many events were played.
    pub events: usize,
    /// Whether the picture of the world or the log changed.
    pub changed: bool,
}

/// The playback queue. See the [module](self).
#[derive(Clone, Debug)]
pub struct Playback {
    stretches: VecDeque<Stretch>,
    active: Vec<Active>,
    /// The markers of what was only heard, with how long each has been
    /// shown, in seconds of real time.
    heard: Vec<(HeardId, f64)>,
    cues: Vec<SoundCue>,
    /// The tick of the last event played.
    reached: Option<Tick>,
    /// The sprite the camera follows as playback moves it.
    follow: Option<SpriteId>,
    stretch_time: f64,
    max_gap: Dur,
    catch_up: f64,
    heard_time: f64,
    /// How many events have been played since the queue was made.
    played: u64,
}

impl Default for Playback {
    fn default() -> Playback {
        Playback::new()
    }
}

impl Playback {
    /// An empty queue with the default pacing.
    pub fn new() -> Playback {
        Playback {
            stretches: VecDeque::new(),
            active: Vec::new(),
            heard: Vec::new(),
            cues: Vec::new(),
            reached: None,
            follow: None,
            stretch_time: STRETCH_TIME_DEFAULT,
            max_gap: MAX_GAP_DEFAULT,
            catch_up: CATCH_UP_DEFAULT,
            heard_time: HEARD_TIME_DEFAULT,
            played: 0,
        }
    }

    /// The real time a stretch takes, in seconds: the setting
    /// `stretch_time`.
    pub fn stretch_time(&self) -> f64 {
        self.stretch_time
    }

    /// Sets the real time a stretch takes, in seconds. Stretches
    /// already queued keep the pace they were laid out with.
    pub fn set_stretch_time(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds >= 0.0 {
            self.stretch_time = seconds;
        }
    }

    /// The most simulated time between two groups that is played out.
    pub fn max_gap(&self) -> Dur {
        self.max_gap
    }

    /// Sets the most simulated time between two groups that is played
    /// out.
    pub fn set_max_gap(&mut self, gap: Dur) {
        self.max_gap = Dur(gap.0.max(0));
    }

    /// The real time within which playback has always nearly caught up,
    /// in seconds.
    pub fn catch_up(&self) -> f64 {
        self.catch_up
    }

    /// Sets the real time within which playback has always nearly
    /// caught up, in seconds, above zero.
    pub fn set_catch_up(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds > 0.0 {
            self.catch_up = seconds;
        }
    }

    /// How long the marker of something only heard stays, in seconds.
    pub fn heard_time(&self) -> f64 {
        self.heard_time
    }

    /// Sets how long the marker of something only heard stays.
    pub fn set_heard_time(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds >= 0.0 {
            self.heard_time = seconds;
        }
    }

    /// Makes the camera follow a sprite as playback moves it, or stops
    /// it following any.
    pub fn follow(&mut self, sprite: Option<SpriteId>) {
        self.follow = sprite;
    }

    /// The sprite the camera follows.
    pub fn followed(&self) -> Option<SpriteId> {
        self.follow
    }

    /// Whether every event handed over has been played: the picture is
    /// of the present, and input applies to it.
    pub fn is_caught_up(&self) -> bool {
        self.stretches
            .iter()
            .all(|stretch| stretch.groups.is_empty())
    }

    /// Whether nothing is left to play or to finish: no event, no hold
    /// and no animation.
    pub fn is_idle(&self) -> bool {
        self.stretches.is_empty() && self.active.is_empty()
    }

    /// How many events are waiting to be played.
    pub fn pending(&self) -> usize {
        self.stretches
            .iter()
            .flat_map(|stretch| &stretch.groups)
            .map(|group| group.steps.len())
            .sum()
    }

    /// How many events have been played since the queue was made.
    pub fn played(&self) -> u64 {
        self.played
    }

    /// The moment playback has reached, while it is behind: the instant
    /// before the next event to be played. `None` when it has caught up
    /// and the picture is of the present (12 §10.4).
    pub fn moment(&self) -> Option<Tick> {
        let next = self
            .stretches
            .iter()
            .find_map(|stretch| stretch.groups.front())?;
        Some(Tick(next.tick.0.saturating_sub(1)))
    }

    /// The tick of the last event played, if any has been.
    pub fn reached(&self) -> Option<Tick> {
        self.reached
    }

    /// Takes the sound cues of the events played since this was last
    /// called, in the order they were played.
    pub fn take_cues(&mut self) -> Vec<SoundCue> {
        core::mem::take(&mut self.cues)
    }

    /// How much real time what is queued takes at the usual pace, in
    /// seconds.
    pub fn backlog(&self) -> f64 {
        self.stretches
            .iter()
            .map(|stretch| (stretch.end - stretch.clock).max(0.0))
            .sum()
    }

    /// Queues one batch: the steps of what happened between two
    /// decisions, in the order it happened. Steps that do nothing are
    /// left out, and a batch of nothing queues nothing.
    pub(crate) fn push(&mut self, steps: Vec<Step>) {
        let mut groups: Vec<Group> = Vec::new();
        for step in steps {
            if step.is_empty() {
                continue;
            }
            match groups.last_mut() {
                // Ticks never go backward in a batch; one that did
                // would be played with the group before it.
                Some(group) if step.tick <= group.tick => group.steps.push(step),
                _ => groups.push(Group {
                    tick: step.tick,
                    due: 0.0,
                    steps: vec![step],
                }),
            }
        }
        if groups.is_empty() {
            return;
        }
        // Where each group falls when long waits are cut short, in
        // seconds of simulated time.
        let mut paced = Vec::with_capacity(groups.len());
        let mut at = 0.0;
        for (index, group) in groups.iter().enumerate() {
            if index > 0 {
                let gap = group.tick.0.saturating_sub(groups[index - 1].tick.0);
                at += gap.clamp(0, self.max_gap.0) as f64 / MICROS_PER_SECOND;
            }
            paced.push(at);
        }
        // The stretch runs from the first group that shows something
        // to the last: silent ones never make it longer.
        let shows = |group: &Group| group.steps.iter().any(|step| !step.is_silent());
        let first = groups.iter().position(shows);
        let last = groups.iter().rposition(shows);
        let (lo, hi) = match (first, last) {
            (Some(first), Some(last)) => (paced[first], paced[last]),
            _ => (0.0, 0.0),
        };
        let span = hi - lo;
        let mut before = 0.0;
        for (group, paced) in groups.iter_mut().zip(&paced) {
            let offset = if span > 0.0 {
                (paced.clamp(lo, hi) - lo) / span * self.stretch_time
            } else {
                0.0
            };
            group.due = offset.max(before);
            let hold = group
                .steps
                .iter()
                .map(|step| step.hold)
                .fold(0.0_f64, f64::max);
            before = group.due + hold;
        }
        self.stretches.push_back(Stretch {
            groups: groups.into(),
            clock: 0.0,
            end: before,
        });
    }

    /// Advances the queue by real time, in seconds, at a speed: plays
    /// what falls due into the picture and the log, ages the effects in
    /// progress and removes the markers that have been shown long
    /// enough. A speed of zero is instant: everything queued is played
    /// now and no animation begins.
    pub fn advance(
        &mut self,
        seconds: f64,
        speed: f64,
        mut world: Option<&mut WorldView>,
        log: &mut MessageLog,
    ) -> Played {
        let seconds = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        let mut played = Played::default();
        // Markers are shown for real time, whatever the pace.
        let heard_time = self.heard_time;
        let before = self.heard.len();
        self.heard.retain_mut(|(id, age)| {
            *age += seconds;
            if *age < heard_time {
                return true;
            }
            if let Some(world) = world.as_deref_mut() {
                world.model_mut().remove_heard(*id);
            }
            false
        });
        played.changed |= self.heard.len() != before;

        if !(speed.is_finite() && speed > 0.0) {
            let all = self.finish(world, log);
            played.events += all.events;
            played.changed |= all.changed;
            return played;
        }
        let paced = seconds * speed;
        for active in &mut self.active {
            active.age += paced;
        }
        self.active.retain(|active| active.age < active.time);

        // More queued than can be caught up with in time plays faster.
        let boost = (self.backlog() / self.catch_up).max(1.0);
        let mut budget = paced * boost;
        while let Some(stretch) = self.stretches.front_mut() {
            let target = stretch.clock + budget;
            let mut due = Vec::new();
            while stretch
                .groups
                .front()
                .is_some_and(|group| group.due <= target)
            {
                due.extend(stretch.groups.pop_front());
            }
            let (over, end) = (
                stretch.groups.is_empty() && target >= stretch.end,
                stretch.end,
            );
            if over {
                budget = target - end;
                self.stretches.pop_front();
            } else {
                stretch.clock = target;
            }
            for group in due {
                let done = self.play(group, true, world.as_deref_mut(), log);
                played.events += done.events;
                played.changed |= done.changed;
            }
            if !over {
                break;
            }
        }
        played
    }

    /// Jumps playback to its end: everything queued is played now, and
    /// every animation in progress ends. The markers of what was heard
    /// stay for their time.
    pub fn skip(&mut self, world: Option<&mut WorldView>, log: &mut MessageLog) -> Played {
        self.finish(world, log)
    }

    /// Plays everything queued, starting no effect, and ends those in
    /// progress.
    fn finish(&mut self, mut world: Option<&mut WorldView>, log: &mut MessageLog) -> Played {
        let mut played = Played {
            events: 0,
            changed: !self.active.is_empty(),
        };
        self.active.clear();
        while let Some(mut stretch) = self.stretches.pop_front() {
            while let Some(group) = stretch.groups.pop_front() {
                let done = self.play(group, false, world.as_deref_mut(), log);
                played.events += done.events;
                played.changed |= done.changed;
            }
        }
        played
    }

    /// Plays the steps of one tick.
    fn play(
        &mut self,
        group: Group,
        effects: bool,
        mut world: Option<&mut WorldView>,
        log: &mut MessageLog,
    ) -> Played {
        let mut played = Played::default();
        for step in group.steps {
            played.events += 1;
            self.played += 1;
            self.reached = Some(step.tick);
            if let Some(world) = world.as_deref_mut() {
                for op in &step.ops {
                    played.changed |= self.apply(op, effects, world);
                }
                if let Some(marker) = step.heard {
                    let id = world.model_mut().add_heard(marker);
                    self.heard.push((id, 0.0));
                    played.changed = true;
                }
                if effects {
                    for effect in &step.effects {
                        self.begin(effect, world);
                    }
                }
            }
            for line in step.lines {
                log.add(&line.category, line.text, Some(step.tick), line.importance);
                played.changed = true;
            }
            self.cues.extend(step.cues);
        }
        played
    }

    /// Makes one change to the picture. Returns whether anything
    /// changed.
    fn apply(&mut self, op: &Op, effects: bool, world: &mut WorldView) -> bool {
        match *op {
            Op::Cell { at, contents } => {
                let model = world.model_mut();
                let same = model
                    .cell(at)
                    .is_some_and(|known| known.seen() && known.contents() == contents);
                if !same {
                    model.observe_cell(at, contents);
                }
                !same
            }
            Op::Field { at, field } => {
                let model = world.model_mut();
                let Some(known) = model.cell(at).copied() else {
                    return false;
                };
                let field = field.filter(|field| field.level > 0);
                if known.field == field {
                    return false;
                }
                model.observe_cell(
                    at,
                    CellContents {
                        field,
                        ..known.contents()
                    },
                );
                if !known.seen() {
                    model.unsee_cell(at);
                }
                true
            }
            Op::Sprite {
                id,
                look,
                to,
                glide,
            } => {
                let known = world.model().sprite(id).copied();
                let now = match (known, look, to) {
                    // It moved, and may look different now.
                    (Some(old), look, Some(to)) => Some(EntitySprite {
                        loc: to,
                        ..look.unwrap_or(old)
                    }),
                    // It appeared.
                    (None, Some(look), Some(to)) => Some(EntitySprite { loc: to, ..look }),
                    // The event says where something is that the party
                    // never perceived: nothing to draw.
                    (None, None, Some(_)) => return false,
                    // It is where the party perceives it.
                    (_, Some(look), None) => Some(look),
                    // It is no longer perceived.
                    (_, None, None) => None,
                };
                match now {
                    Some(now) => {
                        if known == Some(now) {
                            return false;
                        }
                        world.model_mut().upsert_sprite(id, now);
                        if self.follow == Some(id) {
                            world.camera_mut().follow(now.loc);
                        }
                        if let Some(old) = known
                            && effects
                            && glide > 0.0
                            && old.loc != now.loc
                            && old.loc.grid == now.loc.grid
                            && old.loc.cell.z == now.loc.cell.z
                        {
                            // One glide for a sprite at a time: a new
                            // move begins where the last one led.
                            self.active.retain(|active| {
                                !matches!(active.what, Running::Glide { id: of, .. } if of == id)
                            });
                            self.active.push(Active {
                                what: Running::Glide { id, from: old.loc },
                                age: 0.0,
                                time: glide,
                            });
                        }
                        true
                    }
                    None => world.model_mut().remove_sprite(id).is_some(),
                }
            }
            Op::Gone { id } => world.model_mut().remove_sprite(id).is_some(),
        }
    }

    /// Begins the effects of a step.
    fn begin(&mut self, effect: &Effect, world: &WorldView) {
        let place = |target: Target| match target {
            Target::Cell(loc) => Some(loc),
            Target::Sprite(id) => world.model().sprite(id).map(|sprite| sprite.loc),
        };
        if let Some((anim, at, to)) = &effect.anim
            && let Some(at) = place(*at)
        {
            let to = to.and_then(place);
            if !anim.travel || to.is_some() {
                self.active.push(Active {
                    what: Running::Anim {
                        anim: Arc::clone(anim),
                        at,
                        to: to.filter(|_| anim.travel),
                    },
                    age: 0.0,
                    time: anim.time,
                });
            }
        }
        if effect.shake > 0.0 && effect.time > 0.0 {
            self.active.push(Active {
                what: Running::Shake(effect.shake),
                age: 0.0,
                time: effect.time,
            });
        }
        if let Some(color) = effect.flash
            && effect.time > 0.0
        {
            self.active.push(Active {
                what: Running::Flash(color),
                age: 0.0,
                time: effect.time,
            });
        }
    }

    /// Writes the effects in progress into the view, for the frame
    /// about to be built, and returns what the frame itself is given:
    /// shake and a flash. With no view only the frame's effects are
    /// worked out.
    pub fn render(&self, world: Option<&mut WorldView>) -> FrameEffects {
        let mut frame = FrameEffects::default();
        for active in &self.active {
            let left = 1.0 - active.share();
            match &active.what {
                Running::Shake(strength) => frame.shake = frame.shake.max(strength * left),
                Running::Flash(color) => {
                    let strength = FLASH_STRENGTH * left;
                    if frame.flash.is_none_or(|flash| flash.strength < strength) {
                        frame.flash = Some(Wash {
                            color: color.rgb,
                            strength,
                        });
                    }
                }
                Running::Glide { .. } | Running::Anim { .. } => {}
            }
        }
        let Some(world) = world else {
            return frame;
        };
        let mut fx = core::mem::take(world.fx_mut());
        fx.clear();
        let base = world.glyphs().base();
        for active in &self.active {
            let share = active.share();
            match &active.what {
                Running::Glide { id, from } => {
                    let Some(sprite) = world.model().sprite(*id) else {
                        continue;
                    };
                    let to = sprite.loc;
                    if to.grid != from.grid || to.cell.z != from.cell.z {
                        continue;
                    }
                    let part = if sprite.suspected {
                        world.glyphs().marks().suspected
                    } else {
                        sprite.glyph
                    };
                    let inputs = GlyphInputs {
                        own: sprite.own,
                        dress: sprite.dress,
                        ..GlyphInputs::default()
                    };
                    let drawn = resolve_part(part, &inputs, base);
                    let (dx, dy) = (
                        (to.cell.x - from.cell.x) as f32,
                        (to.cell.y - from.cell.y) as f32,
                    );
                    let at = |share: f32| WorldSprite {
                        grid: to.grid,
                        x: from.cell.x as f32 + dx * share,
                        y: from.cell.y as f32 + dy * share,
                        z: to.cell.z,
                        ch: drawn.ch,
                        fg: drawn.fg,
                        alpha: 1.0,
                        layer: LAYER_GLIDE,
                    };
                    fx.hidden.push(*id);
                    // Something that crosses several cells at once
                    // leaves afterimages, so that it reads as fast and
                    // not as vanishing and appearing.
                    if dx.abs().max(dy.abs()) >= TRAIL_FROM_CELLS {
                        for (behind, alpha) in TRAIL.iter().rev() {
                            let share = share - behind;
                            if share > 0.0 {
                                fx.sprites.push(WorldSprite {
                                    alpha: *alpha,
                                    layer: LAYER_GLIDE - 1,
                                    ..at(share)
                                });
                            }
                        }
                    }
                    fx.sprites.push(at(share));
                }
                Running::Anim { anim, at, to } => {
                    let part = anim.part_at(share);
                    match to {
                        None => fx.cells.push(CellFx { loc: *at, part }),
                        Some(to) if to.grid == at.grid && to.cell.z == at.cell.z => {
                            let drawn = resolve_part(part, &GlyphInputs::default(), base);
                            fx.sprites.push(WorldSprite {
                                grid: at.grid,
                                x: at.cell.x as f32 + (to.cell.x - at.cell.x) as f32 * share,
                                y: at.cell.y as f32 + (to.cell.y - at.cell.y) as f32 * share,
                                z: at.cell.z,
                                ch: drawn.ch,
                                fg: drawn.fg,
                                alpha: 1.0,
                                layer: LAYER_GLIDE + 1,
                            });
                        }
                        Some(_) => {}
                    }
                }
                Running::Shake(_) | Running::Flash(_) => {}
            }
        }
        *world.fx_mut() = fx;
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Line {
        Line {
            category: "t".to_owned(),
            text: RichText::plain_text(text),
            importance: Importance::Normal,
        }
    }

    fn said(tick: i64, text: &str) -> Step {
        Step {
            tick: Tick(tick),
            lines: vec![line(text)],
            ..Step::default()
        }
    }

    fn texts(log: &MessageLog) -> Vec<String> {
        log.lines().iter().map(|line| line.shown()).collect()
    }

    #[test]
    fn a_stretch_takes_the_stretch_time_however_long_it_was() {
        for scale in [1, 1_000, 60_000_000] {
            let mut queue = Playback::new();
            let mut log = MessageLog::new();
            queue.push(vec![
                said(0, "one"),
                said(scale, "two"),
                said(2 * scale, "three"),
            ]);
            assert!(!queue.is_caught_up());
            assert_eq!(queue.moment(), Some(Tick(-1)));
            // What is due now is played by an advance of no time.
            assert_eq!(queue.advance(0.0, 1.0, None, &mut log).events, 1);
            assert_eq!(texts(&log), ["one"]);
            queue.advance(0.17, 1.0, None, &mut log);
            assert_eq!(texts(&log), ["one"], "{scale}");
            queue.advance(0.01, 1.0, None, &mut log);
            assert_eq!(texts(&log), ["one", "two"], "{scale}");
            assert_eq!(queue.moment(), Some(Tick(2 * scale - 1)));
            queue.advance(0.16, 1.0, None, &mut log);
            assert_eq!(texts(&log), ["one", "two"]);
            queue.advance(0.02, 1.0, None, &mut log);
            assert_eq!(texts(&log), ["one", "two", "three"]);
            assert!(queue.is_caught_up() && queue.is_idle());
            assert_eq!(queue.moment(), None);
            assert_eq!(queue.reached(), Some(Tick(2 * scale)));
        }
    }

    #[test]
    fn a_long_wait_is_cut_short_and_what_follows_keeps_its_spacing() {
        let mut queue = Playback::new();
        let mut log = MessageLog::new();
        // An hour of nothing, then two blows a second apart.
        let hour = 3_600_000_000;
        queue.push(vec![
            said(0, "sleep"),
            said(hour, "blow"),
            said(hour + 1_000_000, "blow again"),
        ]);
        // The wait counts for two seconds and the blows for one: the
        // first blow falls at two thirds of the stretch, not at its end.
        queue.advance(0.22, 1.0, None, &mut log);
        assert_eq!(texts(&log), ["sleep"]);
        queue.advance(0.02, 1.0, None, &mut log);
        assert_eq!(texts(&log), ["sleep", "blow"]);
        queue.advance(0.12, 1.0, None, &mut log);
        assert_eq!(texts(&log), ["sleep", "blow", "blow again"]);
    }

    #[test]
    fn events_of_one_tick_are_played_together_and_one_group_takes_no_time() {
        let mut queue = Playback::new();
        let mut log = MessageLog::new();
        queue.push(vec![said(7, "a"), said(7, "b"), said(7, "c")]);
        assert_eq!(queue.pending(), 3);
        assert_eq!(queue.backlog(), 0.0);
        assert_eq!(queue.advance(0.001, 1.0, None, &mut log).events, 3);
        assert!(queue.is_caught_up());
        assert_eq!(queue.played(), 3);
        // A batch of nothing queues nothing.
        queue.push(vec![Step::default(), Step::default()]);
        assert!(queue.is_idle());
    }

    #[test]
    fn a_hold_keeps_what_follows_waiting() {
        let mut queue = Playback::new();
        let mut log = MessageLog::new();
        let mut held = said(0, "crash");
        held.hold = 1.0;
        queue.push(vec![held, said(1, "after")]);
        queue.advance(0.9, 1.0, None, &mut log);
        assert_eq!(texts(&log), ["crash"]);
        queue.advance(0.2, 1.0, None, &mut log);
        assert_eq!(texts(&log), ["crash", "after"]);
        // A hold on the last event holds the next stretch.
        let mut last = said(5, "boom");
        last.hold = 0.5;
        queue.push(vec![last]);
        queue.push(vec![said(6, "then")]);
        queue.advance(0.4, 1.0, None, &mut log);
        assert_eq!(texts(&log).last().unwrap(), "boom");
        assert!(!queue.is_caught_up());
        queue.advance(0.2, 1.0, None, &mut log);
        assert_eq!(texts(&log).last().unwrap(), "then");
    }

    #[test]
    fn speed_scales_and_zero_is_instant() {
        let mut fast = Playback::new();
        let mut log = MessageLog::new();
        fast.push(vec![said(0, "a"), said(10, "b")]);
        fast.advance(0.18, 2.0, None, &mut log);
        assert_eq!(texts(&log), ["a", "b"], "twice as fast");

        let mut instant = Playback::new();
        let mut log = MessageLog::new();
        let mut held = said(0, "a");
        held.hold = 10.0;
        instant.push(vec![held, said(10, "b")]);
        instant.push(vec![said(20, "c")]);
        assert_eq!(instant.advance(0.0, 0.0, None, &mut log).events, 3);
        assert_eq!(texts(&log), ["a", "b", "c"]);
        assert!(instant.is_idle());
    }

    #[test]
    fn a_backlog_plays_faster_so_that_playback_always_catches_up() {
        let mut queue = Playback::new();
        let mut log = MessageLog::new();
        // Forty stretches: fourteen seconds at the usual pace.
        for stretch in 0..40 {
            queue.push(vec![
                said(stretch * 10, "tick"),
                said(stretch * 10 + 5, "tock"),
            ]);
        }
        assert!((queue.backlog() - 14.0).abs() < 1e-9);
        let mut seconds = 0.0;
        while !queue.is_caught_up() {
            queue.advance(1.0 / 60.0, 1.0, None, &mut log);
            seconds += 1.0 / 60.0;
            assert!(seconds < 5.0, "still behind after {seconds} s");
        }
        // The backlog shrinks by its own share each second until one
        // second is left: about 1 + ln(14) seconds in all.
        assert!((3.0..4.2).contains(&seconds), "{seconds}");
        assert_eq!(log.lines().len(), 80);
    }

    #[test]
    fn skipping_plays_everything_now() {
        let mut queue = Playback::new();
        let mut log = MessageLog::new();
        let mut held = said(0, "a");
        held.hold = 30.0;
        held.cues.push(SoundCue {
            name: "bang".to_owned(),
            tick: Tick(0),
            at: None,
            heard: false,
        });
        queue.push(vec![held, said(3, "b")]);
        queue.push(vec![said(9, "c")]);
        queue.advance(0.01, 1.0, None, &mut log);
        assert_eq!(texts(&log), ["a"]);
        let played = queue.skip(None, &mut log);
        assert_eq!(played.events, 2);
        assert_eq!(texts(&log), ["a", "b", "c"]);
        assert!(queue.is_idle());
        assert_eq!(queue.take_cues().len(), 1);
        assert!(queue.take_cues().is_empty());
    }

    #[test]
    fn silent_steps_never_make_a_stretch_longer() {
        let cell = |tick: i64| Step {
            tick: Tick(tick),
            ops: vec![Op::Gone {
                id: SpriteId::new(1),
            }],
            ..Step::default()
        };
        assert!(cell(0).is_silent() && !cell(0).is_empty());
        // Only silent steps: played at once.
        let mut queue = Playback::new();
        let mut log = MessageLog::new();
        queue.push(vec![cell(0), cell(1_000_000)]);
        assert_eq!(queue.backlog(), 0.0);
        assert_eq!(queue.advance(0.0, 1.0, None, &mut log).events, 2);
        // A silent step after the last that shows is played with it,
        // and one between two that show at its own time.
        queue.push(vec![
            said(0, "a"),
            cell(1_000_000),
            said(2_000_000, "b"),
            cell(9_000_000),
        ]);
        assert!((queue.backlog() - STRETCH_TIME_DEFAULT).abs() < 1e-9);
        assert_eq!(queue.advance(0.17, 1.0, None, &mut log).events, 1);
        assert_eq!(queue.advance(0.01, 1.0, None, &mut log).events, 1);
        assert_eq!(queue.advance(0.18, 1.0, None, &mut log).events, 2);
        assert!(queue.is_caught_up());
    }
}
