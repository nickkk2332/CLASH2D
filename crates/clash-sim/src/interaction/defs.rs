//! The `interaction` definition type (16 §3.1) and the check that
//! every element's `interactions` names one.

use std::fmt;

use clash_core::{Real, define_id};
use clash_data::{
    CompileCx, Cond, CrossCx, Def, DefLabel, Diagnostic, Scope, Text, Time, ValidateCx, code,
};
use clash_world::defs::{BlockDef, FixtureDef, FloorDef};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

use crate::action::def::ChannelUse;
use crate::device::defs_names;
use crate::rule::Effects;

define_id! {
    /// An `interaction` definition (16 §3.1).
    pub struct InteractionId(u16);
}

/// The name the user is bound under in an interaction's expressions.
pub const USER_SUBJECT: &str = "self";
/// The name the thing it is done to is bound under.
pub const TARGET_SUBJECT: &str = "target";
/// The name the target's cell is bound under.
pub const HERE_SUBJECT: &str = "here";

/// How close the user must be (16 §3.3).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Reach {
    /// The target is inside the user, at any depth: in a hand, worn,
    /// in a pack.
    Held,
    /// The target is in the user's cell.
    SameCell,
    /// The target is in the user's cell or one of the eight round it on
    /// the same storey, or directly above or below where the two cells
    /// are joined: no floor between, or stairs, a ramp or a ladder.
    #[default]
    Adjacent,
    /// Within a distance in metres, a storey counting three, with a
    /// clear line of sight.
    Range(Real),
    /// Through a device the user operates that shares a network with
    /// the target (the stub of 16 §3.3 until networks exist).
    Network,
}

impl Reach {
    /// The name as data writes it: `held`, `same_cell`, `adjacent`,
    /// `range`, `network`.
    pub const fn name(self) -> &'static str {
        match self {
            Reach::Held => "held",
            Reach::SameCell => "same_cell",
            Reach::Adjacent => "adjacent",
            Reach::Range(_) => "range",
            Reach::Network => "network",
        }
    }
}

impl<'de> Deserialize<'de> for Reach {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Reach, D::Error> {
        struct ReachVisitor;

        impl<'de> Visitor<'de> for ReachVisitor {
            type Value = Reach;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "`held`, `same_cell`, `adjacent`, `network`, or `{ \"range\": metres }`",
                )
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Reach, E> {
                match text {
                    "held" => Ok(Reach::Held),
                    "same_cell" => Ok(Reach::SameCell),
                    "adjacent" => Ok(Reach::Adjacent),
                    "network" => Ok(Reach::Network),
                    other => Err(E::unknown_variant(
                        other,
                        &["held", "same_cell", "adjacent", "network", "range"],
                    )),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Reach, A::Error> {
                let mut range = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key != "range" {
                        return Err(serde::de::Error::unknown_field(&key, &["range"]));
                    }
                    range = Some(map.next_value::<f64>()?);
                }
                let range = range.ok_or_else(|| serde::de::Error::missing_field("range"))?;
                Ok(Reach::Range(Real::of(range)))
            }
        }

        deserializer.deserialize_any(ReachVisitor)
    }
}

/// What an interaction is done to (16 §3.2): an element of a cell, or an
/// entity (a device, an item, a creature). It decides the kind `target`
/// is bound as.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// A floor, block or fixture.
    #[default]
    Element,
    /// An entity.
    Entity,
}

impl TargetKind {
    /// The kind of subject `target` is bound as.
    pub const fn subject_kind(self) -> &'static str {
        match self {
            TargetKind::Element => "element",
            TargetKind::Entity => "entity",
        }
    }
}

/// Who offers an interaction besides what lists it (16 §3.2).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfferedBy {
    /// Every target of its kind, where `shown_if` holds: examine, smash,
    /// climb.
    Any,
}

/// Something a character can do to a thing (16 §3.1): open, pull, load,
/// hack, drink from.
///
/// ```json
/// { "type": "interaction", "id": "open", "name": "Open",
///   "reach": "adjacent", "time": "600 ms", "noise": 0.2,
///   "if": "target.state_name == 'closed'",
///   "do": [ { "set_state": { "of": "target", "state": "open" } } ],
///   "default": true }
/// ```
#[derive(Debug, Deserialize)]
pub struct InteractionDef {
    /// What a menu calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What it is done to: `element` (the default) or `entity`.
    #[serde(default)]
    pub target: TargetKind,
    /// How close the user must be.
    #[serde(default)]
    pub reach: Reach,
    /// The channel slots doing it occupies (03 §6.2). Carried for the
    /// action that does it.
    #[serde(default)]
    pub uses: ChannelUse,
    /// How long it takes, in the user's own time.
    #[serde(default)]
    pub time: Time,
    /// How loud it is, as a factor for signals (15 §3). Carried for
    /// perception.
    #[serde(default)]
    pub noise: Real,
    /// The tool qualities it needs (16 §8.3), as written. Carried for
    /// package G6.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether it can be done now: tested against truth when it is
    /// attempted, with `self` the user, `target` the thing and `here`
    /// the thing's cell.
    #[serde(default, rename = "if")]
    pub condition: Cond,
    /// Whether it is listed at all: a mind context of the user, read
    /// when a menu is made.
    #[serde(default)]
    pub shown_if: Cond,
    /// What a refusal for `if` says. Left out, a sentence of the
    /// engine's.
    #[serde(default)]
    pub fail_text: Option<Text>,
    /// What it does: effects with `self`, `target` and `here`.
    #[serde(default, rename = "do")]
    pub effects: Effects,
    /// Whether it is the one used when the thing is bumped into or
    /// clicked without a choice.
    #[serde(default)]
    pub default: bool,
    /// `any`: offered by every target of its kind, where `shown_if`
    /// holds.
    #[serde(default)]
    pub offered_by: Option<OfferedBy>,
}

impl InteractionDef {
    /// The scope of `if`, `fail_text` and `do`: the user as `self`, the
    /// target as `target` of its kind, its cell as `here`. Truth.
    pub fn scope(&self) -> Scope {
        Scope::new()
            .own(USER_SUBJECT, "entity")
            .subject(TARGET_SUBJECT, self.target.subject_kind())
            .subject(HERE_SUBJECT, "loc")
    }

    /// The scope of `shown_if`: the same subjects, as a mind context of
    /// the user, pure.
    pub fn mind_scope(&self) -> Scope {
        Scope::new()
            .mind(USER_SUBJECT, "entity")
            .subject(TARGET_SUBJECT, self.target.subject_kind())
            .subject(HERE_SUBJECT, "loc")
            .pure()
    }
}

impl Def for InteractionDef {
    type Id = InteractionId;
    const TYPE: &'static str = "interaction";
    const DOC: &'static str = "Something a character can do to a thing: its reach, time, conditions and effects (16 §3.1).";

    fn finish(&mut self, cx: &mut clash_data::FinishCx<'_>) {
        self.uses.finish("uses", cx);
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        let truth = self.scope();
        cx.compile(&mut self.condition, &truth);
        cx.compile(&mut self.fail_text, &truth.clone().pure());
        let mind = self.mind_scope();
        cx.compile(&mut self.shown_if, &mind);
        cx.compile(&mut self.effects, &truth);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if let Reach::Range(range) = self.reach
            && !(range.is_finite() && range >= Real::ZERO)
        {
            cx.error("`reach.range` is a distance in metres of zero or more");
        }
        if !(self.noise.is_finite() && self.noise >= Real::ZERO) {
            cx.error("`noise` is a factor of zero or more");
        }
        if self.time.0.as_micros() < 0 {
            cx.error("`time` is a duration of zero or more");
        }
        self.effects.validate(cx);
    }
}

/// Reports every element whose `interactions` names something that is
/// not an `interaction` definition (02 §5.5). A cross check of the
/// loader.
pub fn check_element_interactions(cx: &mut CrossCx<'_>) {
    let defs = cx.defs();
    // A load with no interactions at all (the world's content alone, for
    // a test or a tool) is not one to offer any.
    if defs.iter::<InteractionDef>().next().is_none() {
        return;
    }
    let mut missing: Vec<(&'static str, String, String)> = Vec::new();
    let mut look = |ty: &'static str, name: &str, listed: &[String]| {
        for written in listed {
            if !defs_names::<InteractionDef>(cx, name, written) {
                missing.push((ty, name.to_owned(), written.clone()));
            }
        }
    };
    for (id, def) in defs.iter::<FloorDef>() {
        look(
            FloorDef::TYPE,
            defs.name::<FloorDef>(id).unwrap_or_default(),
            &def.interactions,
        );
    }
    for (id, def) in defs.iter::<BlockDef>() {
        look(
            BlockDef::TYPE,
            defs.name::<BlockDef>(id).unwrap_or_default(),
            &def.interactions,
        );
    }
    for (id, def) in defs.iter::<FixtureDef>() {
        look(
            FixtureDef::TYPE,
            defs.name::<FixtureDef>(id).unwrap_or_default(),
            &def.interactions,
        );
    }
    for (ty, element, interaction) in missing {
        let diagnostic = Diagnostic::new(
            code::VALIDATION_WARNING,
            format!("there is no interaction `{interaction}` for this element to offer"),
        )
        .in_def(Some(&DefLabel::new(ty, element)))
        .with_path("interactions");
        cx.report(diagnostic);
    }
}
