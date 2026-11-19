//! The `show` and `anim` definition types (12 §7.3): how an event
//! appears when it is played.
//!
//! Nothing here knows any event. A `show` names its event by `on`, and
//! everything it reads of the event it reads by name: the subjects the
//! event binds and the fields it carries, as the event's registration
//! lists them (05 §8.1). Its texts and its condition are compiled when
//! mods load, in the event's scope, pure, and as a mind context with
//! nobody's own: they may read what is safe for belief and nothing
//! else, because a log line speaks from what the viewing character
//! knows (01 R11).
//!
//! # A load with no events
//!
//! A tool or a test that loads mods without the simulation's registry
//! has no events at all. There a `show` is read and checked as far as
//! it can be without one, and is not compiled: it shows nothing. Where
//! any event is registered, a `show` about an event that is not is a
//! load error.

use std::collections::BTreeMap;

use clash_core::{Value, define_id};
use clash_data::defs::ValidateCx;
use clash_data::{
    CompileCx, Cond, Def, Diagnostic, FromKind, Kind, KindValue, Located, Span, Text, Time,
    by_kind, code,
};
use serde::Deserialize;
use serde::de::MapAccess;

use super::log::Importance;
use crate::color::ColorRef;
use crate::frame::is_single_width;
use crate::text::tag_names;
use crate::ui::Literal;

define_id! {
    /// A `show` definition.
    pub struct ShowId(u16);
    /// An `anim` definition.
    pub struct AnimId(u16);
}

/// The subject a `show` plays its animation at unless it says where.
pub const SUBJECT_HERE: &str = "here";

/// One change a `show` makes to the picture of the world when its event
/// was seen (12 §7.3): an object with one key.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeDef {
    /// `{ "cell": "here" }`: the cell a subject names is observed
    /// again: what the party sees there replaces what was known of it.
    Cell(Located<String>),
    /// `{ "field": "here" }`: only the strongest field of the cell a
    /// subject names is taken from what the party sees there.
    Field(Located<String>),
    /// `{ "sprite": "self" }` or `{ "sprite": { "of": "self", "to":
    /// "here" } }`: the perceived thing a subject names moved, appeared
    /// or changed.
    Sprite(SpriteChangeDef),
    /// `{ "gone": "self" }`: the sprite of the thing a subject names is
    /// removed.
    Gone(Located<String>),
}

/// The `sprite` change: which thing, and where it now is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteChangeDef {
    /// The subject that names the thing.
    pub of: Located<String>,
    /// The subject that names the cell it is in now. Left out, or not
    /// bound by the event, it is where the party perceives it.
    pub to: Option<Located<String>>,
}

/// The object form of [`SpriteChangeDef`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpriteChangeObject {
    /// The subject of the event that names the thing whose sprite changes.
    of: Located<String>,
    /// The subject that names the cell it is in now. Left out, or not bound by
    /// the event, it is where the party perceives it.
    #[serde(default)]
    to: Option<Located<String>>,
}

impl<'de> FromKind<'de> for SpriteChangeDef {
    fn from_kind<A: MapAccess<'de>>(kind: Kind, value: KindValue<'_, A>) -> Result<Self, A::Error> {
        match kind {
            Kind::String => value.read().map(|of| SpriteChangeDef { of, to: None }),
            Kind::Object => value
                .read::<SpriteChangeObject>()
                .map(|whole| SpriteChangeDef {
                    of: whole.of,
                    to: whole.to,
                }),
            other => Err(other.unexpected("a subject's name, or an object with `of` and `to`")),
        }
    }
}

impl<'de> Deserialize<'de> for SpriteChangeDef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Read through the loader, which keeps where each part was
        // written (see `clash_data::de`).
        by_kind(deserializer)
    }
}

/// What a `show` does when its event was only heard (12 §5): a marker
/// where it seemed to come from, and a log line. Never a picture.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeardDef {
    /// The line written to the log, a template in the event's scope.
    #[serde(default)]
    pub log: Option<Text>,
    /// The character the marker is drawn with. Left out, the theme's
    /// `heard` mark.
    #[serde(default)]
    pub marker: Option<char>,
    /// How long nothing this show is about must have been heard before
    /// its line is written again (12 §7.4, decision D-945): footsteps
    /// heard while someone walks about are told once, and again only
    /// after a quiet. The marker is put each time. Left out, every one
    /// is told.
    #[serde(default)]
    pub quiet: Option<Time>,
}

/// A `show` definition (12 §7.3): how an event appears when it is
/// played.
///
/// ```json
/// { "type": "show", "id": "wall_breaks",
///   "on": "element_destroyed", "filter": { "layer": "block" },
///   "change": [ { "cell": "here" } ],
///   "anim": "burst", "at": "here",
///   "log": "{cap(name(actor))} {verb(actor, 'break')} through.",
///   "heard": { "log": "Something breaks nearby.", "marker": "!" },
///   "shake": 0.1, "min_time": "120 ms", "importance": "normal", "sound": "break" }
/// ```
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowDef {
    /// The event this is about, by its registered name.
    pub on: Located<String>,
    /// A cheap test on the event's plain fields: each named field must
    /// hold the constant given (05 §8.4).
    #[serde(default)]
    pub filter: BTreeMap<String, Located<Literal>>,
    /// `if`: a condition in the event's scope. The show applies only
    /// while it holds.
    #[serde(default, rename = "if")]
    pub when: Option<Cond>,
    /// The changes to the picture of the world, made when the event
    /// was seen. Left out, everything the event is about is observed
    /// again; an empty list changes nothing.
    #[serde(default)]
    pub change: Option<Vec<ChangeDef>>,
    /// The animation played, by the id of an `anim` definition.
    #[serde(default)]
    pub anim: Option<Located<String>>,
    /// The subject the animation plays at. Left out, `here`.
    #[serde(default)]
    pub at: Option<Located<String>>,
    /// The subject a travelling animation ends at.
    #[serde(default)]
    pub to: Option<Located<String>>,
    /// The line written to the log, a template in the event's scope
    /// worded for the reader (05 §4.6).
    #[serde(default)]
    pub log: Option<Text>,
    /// The category of its lines, which a `log` widget filters by.
    /// Left out, `event`.
    #[serde(default)]
    pub category: Option<String>,
    /// What to do if the event was only heard.
    #[serde(default)]
    pub heard: Option<HeardDef>,
    /// Screen shake as the event is played, from 0 to 1, dying away
    /// over `time`.
    #[serde(default)]
    pub shake: f64,
    /// A colour flashed over the whole frame, dying away over `time`.
    #[serde(default)]
    pub flash: Option<ColorRef>,
    /// How long the show's own effects last: the glide of a sprite it
    /// moves, its shake and its flash. Left out, 150 ms.
    #[serde(default)]
    pub time: Option<Time>,
    /// How long the event holds the queue: nothing later is played
    /// before this has passed.
    #[serde(default)]
    pub min_time: Option<Time>,
    /// How much the event matters: `minor`, `normal` or `major`.
    #[serde(default)]
    pub importance: Importance,
    /// The name of a sound cue. It is handed on with the event and
    /// nothing plays it yet.
    #[serde(default)]
    pub sound: Option<Located<String>>,
    /// Whether the texts and the condition were compiled against the
    /// event's registration.
    #[serde(skip)]
    compiled: bool,
}

/// One keyframe of an `anim`: the parts of the glyph it changes from
/// the frame before.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimFrameDef {
    /// When the frame begins, as a share of the animation's time from
    /// 0 to 1. Left out, 0.
    #[serde(default)]
    pub at: f64,
    /// The character.
    #[serde(default)]
    pub ch: Option<char>,
    /// The colour of the character.
    #[serde(default)]
    pub fg: Option<ColorRef>,
    /// Whether the character is drawn bold.
    #[serde(default)]
    pub bold: Option<bool>,
}

/// An `anim` definition (12 §7.3): a short keyframed effect at a place,
/// or along the way from one place to another.
///
/// ```json
/// { "type": "anim", "id": "burst", "time": "300 ms",
///   "frames": [ { "ch": "*", "fg": "white" }, { "at": 0.5, "ch": "%" } ] }
/// ```
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimDef {
    /// How long it lasts.
    pub time: Time,
    /// The keyframes, in order. At least one, and the first gives a
    /// character.
    pub frames: Vec<AnimFrameDef>,
    /// Whether the glyph travels in a straight line from the show's
    /// `at` to its `to` over the time: a projectile. Left out, it stays
    /// on the cell at `at`.
    #[serde(default)]
    pub travel: bool,
}

/// The characters a sound cue's name may be made of beside letters and
/// digits.
const CUE_PUNCTUATION: &[char] = &['_', '-', '.', ':'];

/// Whether a text is a sound cue's name: letters, digits, `_`, `-`, `.`
/// and `:`, and at least one of them.
pub fn is_cue_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || CUE_PUNCTUATION.contains(&ch))
}

impl ShowDef {
    /// The subject names the definition uses, each with where it was
    /// written and the path of its field.
    fn subject_uses(&self) -> Vec<(&Located<String>, String)> {
        let mut uses = Vec::new();
        for (index, change) in self.change.iter().flatten().enumerate() {
            match change {
                ChangeDef::Cell(name) => uses.push((name, format!("change[{index}].cell"))),
                ChangeDef::Field(name) => uses.push((name, format!("change[{index}].field"))),
                ChangeDef::Gone(name) => uses.push((name, format!("change[{index}].gone"))),
                ChangeDef::Sprite(sprite) => {
                    uses.push((&sprite.of, format!("change[{index}].sprite.of")));
                    if let Some(to) = &sprite.to {
                        uses.push((to, format!("change[{index}].sprite.to")));
                    }
                }
            }
        }
        if let Some(at) = &self.at {
            uses.push((at, "at".to_owned()));
        }
        if let Some(to) = &self.to {
            uses.push((to, "to".to_owned()));
        }
        uses
    }

    /// The texts the definition writes to the log, each with the path
    /// of its field.
    pub(crate) fn texts(&self) -> Vec<(&Text, &'static str)> {
        let mut texts = Vec::new();
        if let Some(log) = &self.log {
            texts.push((log, "log"));
        }
        if let Some(log) = self.heard.as_ref().and_then(|heard| heard.log.as_ref()) {
            texts.push((log, "heard.log"));
        }
        texts
    }

    /// Whether the show's texts and condition were compiled: whether it
    /// was loaded against a registry that knows its event.
    pub fn is_compiled(&self) -> bool {
        self.compiled
    }
}

/// An error at a value of a definition.
fn at_value(cx: &CompileCx<'_>, span: Span, path: &str, message: String) -> Diagnostic {
    Diagnostic::new(code::VALIDATION, message)
        .with_path(path)
        .at(cx.sources(), span)
}

impl Def for ShowDef {
    type Id = ShowId;
    const TYPE: &'static str = "show";
    const DOC: &'static str = "How an event appears when it is played back: changes to the picture, an animation, a log line, what to do if it was only heard (12 §7.3).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let registry = cx.registry();
        // A load without the simulation has no events at all: the show
        // is kept as written and shows nothing.
        if registry.events().next().is_none() {
            return;
        }
        let event = self.on.value.clone();
        let Some(found) = registry.event_id(&event).and_then(|id| registry.event(id)) else {
            let names = registry.events().map(|event| event.spec.name.as_str());
            let near = clash_data::diag::closest(&event, names).map(str::to_owned);
            let mut diagnostic = at_value(
                cx,
                self.on.span,
                "on",
                format!("there is no event `{event}`"),
            );
            if let Some(near) = near {
                diagnostic = diagnostic.suggest(format!("did you mean `{near}`?"));
            }
            cx.report(diagnostic);
            return;
        };
        let subjects: Vec<&str> = found
            .spec
            .subjects
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        let fields: Vec<&str> = found
            .spec
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        let mut problems = Vec::new();
        for (name, path) in self.subject_uses() {
            if !subjects.contains(&name.value.as_str()) {
                let mut diagnostic = at_value(
                    cx,
                    name.span,
                    &path,
                    format!("the event `{event}` binds no subject `{}`", name.value),
                );
                if let Some(near) = clash_data::diag::closest(&name.value, subjects.iter().copied())
                {
                    diagnostic = diagnostic.suggest(format!("did you mean `{near}`?"));
                } else {
                    diagnostic = diagnostic.suggest(format!("it binds: {}", subjects.join(", ")));
                }
                problems.push(diagnostic);
            }
        }
        if self.anim.is_some() && self.at.is_none() && !subjects.contains(&SUBJECT_HERE) {
            problems.push(
                Diagnostic::new(
                    code::VALIDATION,
                    format!(
                        "the event `{event}` binds no `{SUBJECT_HERE}` to play the animation at: say where with `at`"
                    ),
                )
                .with_path("anim"),
            );
        }
        for (field, value) in &self.filter {
            if !fields.contains(&field.as_str()) {
                let mut diagnostic = at_value(
                    cx,
                    value.span,
                    &format!("filter.{field}"),
                    format!("the event `{event}` has no field `{field}`"),
                );
                if let Some(near) = clash_data::diag::closest(field, fields.iter().copied()) {
                    diagnostic = diagnostic.suggest(format!("did you mean `{near}`?"));
                }
                problems.push(diagnostic);
            }
        }
        for problem in problems {
            cx.report(problem);
        }
        // What the player reads is worded from what the viewing
        // character knows: belief only, and nothing drawn at random.
        let scope = cx.event_scope(&event).pure().belief();
        cx.compile(&mut self.when, &scope);
        cx.compile(&mut self.log, &scope);
        if let Some(heard) = &mut self.heard {
            cx.compile(&mut heard.log, &scope);
        }
        self.compiled = true;
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !(0.0..=1.0).contains(&self.shake) {
            cx.error(format!(
                "`shake` must be from 0 to 1, and is {}",
                self.shake
            ));
        }
        let quiet = self.heard.as_ref().and_then(|heard| heard.quiet);
        for (time, name) in [
            (self.time, "time"),
            (self.min_time, "min_time"),
            (quiet, "heard.quiet"),
        ] {
            if time.is_some_and(|time| time.dur().0 < 0) {
                cx.error(format!("`{name}` cannot be negative"));
            }
        }
        if let Some(sound) = &self.sound
            && !is_cue_name(&sound.value)
        {
            cx.error_at(
                sound.span,
                format!(
                    "`{}` is not a sound cue's name: letters, digits, `_`, `-`, `.` and `:`",
                    sound.value
                ),
            );
        }
        if let Some(marker) = self.heard.as_ref().and_then(|heard| heard.marker)
            && !is_single_width(marker)
        {
            cx.error(format!(
                "`heard.marker` must be one character of single width, and `{marker}` is not"
            ));
        }
        if self.to.is_some() && self.anim.is_none() {
            cx.warn("`to` says where a travelling animation ends, and there is no `anim`");
        }
        for (text, path) in self.texts() {
            if let Err(error) = tag_names(text.raw()) {
                let message = format!("`{path}`: {error}");
                match text.span() {
                    Some(span) => cx.error_at(span, message),
                    None => cx.error(message),
                }
            }
        }
        if matches!(
            self.flash,
            Some(ColorRef::Material | ColorRef::Own | ColorRef::Dress)
        ) {
            cx.error("`flash` is a colour name or `#rrggbb`: `material`, `own` and `dress` only mean something in a glyph");
        }
    }
}

impl Def for AnimDef {
    type Id = AnimId;
    const TYPE: &'static str = "anim";
    const DOC: &'static str = "A short keyframed effect at a place, or along the way from one place to another, played by a `show` (12 §7.3).";

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.time.dur().0 <= 0 {
            cx.error("`time` must be above zero");
        }
        if self.frames.is_empty() {
            cx.error("`frames` must list at least one frame");
        }
        if self.frames.first().is_some_and(|frame| frame.ch.is_none()) {
            cx.error("the first frame must give a character, `ch`");
        }
        let mut before = 0.0;
        for (index, frame) in self.frames.iter().enumerate() {
            if !(0.0..=1.0).contains(&frame.at) {
                cx.error(format!(
                    "`frames[{index}].at` is a share of the time from 0 to 1, and is {}",
                    frame.at
                ));
            } else if frame.at < before {
                cx.error(format!(
                    "`frames[{index}].at` is {} and the frame before it begins at {before}: frames are written in order",
                    frame.at
                ));
            }
            before = frame.at.max(before);
            if let Some(ch) = frame.ch
                && !is_single_width(ch)
            {
                cx.error(format!(
                    "`frames[{index}].ch` must be one character of single width, and `{ch}` is not"
                ));
            }
            if matches!(
                frame.fg,
                Some(ColorRef::Material | ColorRef::Own | ColorRef::Dress)
            ) {
                cx.error(format!(
                    "`frames[{index}].fg` is a colour name or `#rrggbb`: an animation has no material and no colour of its own"
                ));
            }
        }
    }
}

/// Whether an event's field holds the constant a filter gives. Numbers
/// are compared by value, so `12` matches a field that holds `12.0`.
pub(crate) fn filter_matches(field: Option<&clash_sim::FieldValue>, wanted: &Value) -> bool {
    let Some(field) = field else {
        return false;
    };
    match (field.plain(), wanted) {
        (Some(Value::Bool(have)), Value::Bool(want)) => have == want,
        (Some(Value::Str(have)), Value::Str(want)) => have == want,
        (Some(Value::Int(_) | Value::Real(_)), Value::Int(_) | Value::Real(_)) => {
            let want = match wanted {
                Value::Int(int) => Some(clash_core::Real::from_i64(*int)),
                Value::Real(real) => Some(*real),
                _ => None,
            };
            field.as_real() == want
        }
        _ => false,
    }
}
