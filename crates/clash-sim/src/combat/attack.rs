//! The `attacks` block of an action (07 §3.1): what a technique strikes
//! with, how hard, how much of it pushes, how it finds its mark and
//! whom it reaches.
//!
//! ```json
//! "attacks": {
//!   "cut": {
//!     "with":   { "held": "blade", "surface": "edge" },
//!     "reach":  "limb.reach + weapon.length",
//!     "energy": "self.attr.strength * 5",
//!     "push":   0.05,
//!     "aim":    { "contest": "melee_aim", "by": "melee_skill(self)", "against": "evasion(target, self)" },
//!     "arc":    { "sweep": 140, "max": 5, "through": true } } }
//! ```
//!
//! The block is read with the action, and its expressions are compiled
//! with the action's own scope (03 §6.1) and three more subjects:
//! `weapon`, the item whose surface strikes; `limb`, the body part whose
//! surface strikes; and `target`, the one being struck. Each is null
//! where there is none. They read truth: the strike is resolved in
//! truth, whatever its actor believed when choosing it.

use std::fmt;

use clash_core::Real;
use clash_data::expr::Number;
use clash_data::{
    CompileCx, Diagnostic, Expr, FinishCx, Length, Located, Ref, Scope, ValidateCx, code,
};
use clash_world::defs::DamageTypeDef;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

use super::contest::ContestDef;
use crate::movement::PostureDef;
use crate::rule::Effects;

/// The subject an attack's expressions read the item that strikes as.
pub const WEAPON_SUBJECT: &str = "weapon";
/// The subject an attack's expressions read the part that strikes as.
pub const LIMB_SUBJECT: &str = "limb";
/// The subject an attack's expressions read the one struck as.
pub const TARGET_SUBJECT: &str = "target";

/// What an attack strikes with (07 §3.1, §3.2): a surface of an item
/// held in a grasp, of a part of the actor's own body, or of something
/// gripped; or a surface given inline, for a strike made with no
/// physical thing (08 §7.7).
///
/// ```json
/// "with": { "held": "blade", "surface": "edge" }
/// "with": { "part": "hand", "surface": "fist" }
/// "with": { "kind": "impact", "area": 0.05, "give": "5 cm", "span": "30 cm" }
/// ```
#[derive(Debug, Default, Deserialize)]
pub struct WithDef {
    /// A tag an item held in a grasp carries: the first such item, by
    /// id, that has the surface. An empty text takes any item held.
    #[serde(default)]
    pub held: Option<String>,
    /// A part of the actor's body: its name in the body (`r_hand`), the
    /// id of its definition (`hand`) or a tag its definition carries.
    /// The first working part that answers and has the surface, by key,
    /// one that grasps nothing before one that does.
    #[serde(default)]
    pub part: Option<String>,
    /// Something the actor grips (07 §9). Grips are not built: a strike
    /// with it is refused (`combat.nothing_gripped`).
    #[serde(default)]
    pub gripped: bool,
    /// The id of the surface, among the item's or the part's.
    #[serde(default)]
    pub surface: Option<String>,
    /// An inline surface: the damage type it delivers.
    #[serde(default)]
    pub kind: Option<Ref<DamageTypeDef>>,
    /// An inline surface: its contact area, in square metres.
    #[serde(default)]
    pub area: Option<Real>,
    /// An inline surface: the distance over which it is stopped.
    #[serde(default)]
    pub give: Option<Length>,
    /// An inline surface: its length of contact across the target.
    #[serde(default)]
    pub span: Option<Length>,
}

/// Where a [`WithDef`] finds its surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WithSource<'a> {
    /// An item held in a grasp that carries this tag; any, if it is
    /// empty.
    Held(&'a str),
    /// A part of the actor's body that answers to this.
    Part(&'a str),
    /// Something gripped.
    Gripped,
    /// The surface given inline.
    Inline,
}

impl WithDef {
    /// Where the surface is found. `None` for a `with` that names none,
    /// or more than one: a load error.
    pub fn source(&self) -> Option<WithSource<'_>> {
        let named = [
            self.held.is_some(),
            self.part.is_some(),
            self.gripped,
            self.kind.is_some(),
        ];
        if named.iter().filter(|named| **named).count() != 1 {
            return None;
        }
        Some(match (&self.held, &self.part, self.gripped) {
            (Some(tag), _, _) => WithSource::Held(tag),
            (_, Some(part), _) => WithSource::Part(part),
            (_, _, true) => WithSource::Gripped,
            _ => WithSource::Inline,
        })
    }

    fn validate(&self, path: &str, cx: &mut ValidateCx<'_>) {
        let Some(source) = self.source() else {
            cx.error(format!(
                "{path}: `with` names one source of a surface: `held`, `part`, `gripped`, or an inline surface with `kind`"
            ));
            return;
        };
        let inline_fields = self.area.is_some() || self.give.is_some() || self.span.is_some();
        match source {
            WithSource::Inline => {
                if self.surface.is_some() {
                    cx.error(format!(
                        "{path}: an inline surface is given by `kind`, `area`, `give` and `span`, and names no `surface`"
                    ));
                }
                let area = self.area.unwrap_or(Real::ZERO);
                let give = self.give.map_or(Real::ZERO, Length::get);
                if !(area.is_finite() && area > Real::ZERO) {
                    cx.error(format!(
                        "{path}: an inline surface needs an `area` above zero"
                    ));
                }
                if !(give.is_finite() && give > Real::ZERO) {
                    cx.error(format!(
                        "{path}: an inline surface needs a `give` above zero"
                    ));
                }
                if self
                    .span
                    .is_some_and(|span| !(span.get().is_finite() && span.get() >= Real::ZERO))
                {
                    cx.error(format!("{path}: `span` is a length of zero or more"));
                }
            }
            _ => {
                if self.surface.as_deref().is_none_or(str::is_empty) {
                    cx.error(format!(
                        "{path}: `surface` names the surface struck with, among those of the item or the part"
                    ));
                }
                if inline_fields {
                    cx.error(format!(
                        "{path}: `area`, `give` and `span` belong to an inline surface; this one is found on {}",
                        match source {
                            WithSource::Held(_) => "an item held",
                            WithSource::Part(_) => "a part",
                            _ => "what is gripped",
                        }
                    ));
                }
            }
        }
    }
}

/// How an attack finds its mark (07 §4.2): a contest between the
/// attacker's value and the target's.
#[derive(Debug, Deserialize)]
pub struct AimDef {
    /// The contest.
    pub contest: Ref<ContestDef>,
    /// The attacker's value.
    pub by: Expr<Number>,
    /// The target's value.
    pub against: Expr<Number>,
}

/// The strikes at several targets of one attack (07 §6.4).
///
/// ```json
/// "arc": { "sweep": 140, "max": 5, "through": true }
/// "arc": { "line": "4 m", "max": 6, "through": true }
/// ```
#[derive(Debug, Deserialize)]
pub struct ArcDef {
    /// The angle, in degrees, of a sweep centred on the target named.
    #[serde(default)]
    pub sweep: Option<Real>,
    /// The length of a line from the attacker through the target named.
    #[serde(default)]
    pub line: Option<Length>,
    /// The most targets struck. At least one.
    pub max: u16,
    /// Whether the energy that reaches each target is what passed the
    /// one before.
    #[serde(default)]
    pub through: bool,
}

/// The shape of an [`ArcDef`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ArcShape {
    /// A sweep of this many degrees.
    Sweep(Real),
    /// A line of this many metres.
    Line(Real),
}

impl ArcDef {
    /// The shape: a sweep or a line. `None` for both or neither, a load
    /// error.
    pub fn shape(&self) -> Option<ArcShape> {
        match (self.sweep, self.line) {
            (Some(degrees), None) => Some(ArcShape::Sweep(degrees)),
            (None, Some(length)) => Some(ArcShape::Line(length.get())),
            _ => None,
        }
    }
}

fn zero() -> Expr<Number> {
    Expr::literal(0)
}

/// One attack of an action (07 §3.1).
///
/// | Field | Read |
/// |-------|------|
/// | `with` | [`WithDef`]. Required. |
/// | `reach` | Metres, an expression. Left out: the reach of the part that strikes (06 §6.4); for a held item, that of the part holding it plus the item's `length`; for an inline surface, the actor's. |
/// | `energy` | Joules at full effort, an expression. Required. |
/// | `push` | The share of the energy that moves the target, from 0 to 1, an expression. Default 0. |
/// | `lift` | Degrees above level the push is raised, an expression. Default 0. |
/// | `aim` | [`AimDef`]. Left out, the strike cannot miss its mark. |
/// | `arc` | [`ArcDef`]. Left out, one target. |
/// | `knock_down` | A posture a target is put in when the push launches it. |
/// | `on_land`, `on_miss`, `on_stopped` | Effect lists, run once for each target. |
/// | `swing`, `height`, `lands`, `clashes`, `spill`, `spill_kind` | Read and carried for the packages that give them meaning (07 §4.3, §6.4, §8). |
#[derive(Debug)]
pub struct AttackDef {
    /// Its id within the action.
    pub id: Located<String>,
    /// What strikes.
    pub with: WithDef,
    /// How far it reaches.
    pub reach: Option<Expr<Number>>,
    /// Joules at full effort.
    pub energy: Expr<Number>,
    /// The share of the energy that pushes.
    pub push: Expr<Number>,
    /// Degrees above level the push is raised.
    pub lift: Expr<Number>,
    /// How it finds its mark.
    pub aim: Option<AimDef>,
    /// Several targets.
    pub arc: Option<ArcDef>,
    /// Degrees the surface travels through before it lands (07 §6.4).
    /// Carried.
    pub swing: Option<Real>,
    /// The band of heights, in metres above the attacker's footing, it
    /// reaches (07 §4.3). Carried.
    pub height: Option<[Real; 2]>,
    /// The phase at whose start it lands (07 §3.1), as written. Carried.
    pub lands: Option<Located<String>>,
    /// Whether it can meet another (07 §8). Carried.
    pub clashes: bool,
    /// What a locked clash releases (07 §8.2). Carried.
    pub spill: Option<Real>,
    /// The kind of release a locked clash spills. Carried.
    pub spill_kind: Option<String>,
    /// A posture a target the push launches is put in.
    pub knock_down: Option<Ref<PostureDef>>,
    /// Effects run when it lands on a target.
    pub on_land: Effects,
    /// Effects run when it misses a target.
    pub on_miss: Effects,
    /// Effects run when it reaches a target and nothing of it is
    /// delivered.
    pub on_stopped: Effects,
}

/// An attack as written, before its id is attached.
#[derive(Deserialize)]
struct AttackObject {
    /// What strikes (07 §3.1): an item held, a part of the actor's body, or a
    /// surface given inline. Required.
    with: WithDef,
    /// How far it reaches, in metres: an expression. Left out, the reach of the
    /// part that strikes; for a held item, that of the part holding it plus the
    /// item's `length`; for an inline surface, the actor's.
    #[serde(default)]
    reach: Option<Expr<Number>>,
    /// Joules at full effort: an expression. Required.
    energy: Expr<Number>,
    /// The share of the energy that moves the target, from 0 to 1: an
    /// expression. Default 0.
    #[serde(default = "zero")]
    push: Expr<Number>,
    /// Degrees above level the push is raised: an expression. Default 0.
    #[serde(default = "zero")]
    lift: Expr<Number>,
    /// How it finds its mark (07 §4). Left out, the strike cannot miss its
    /// mark.
    #[serde(default)]
    aim: Option<AimDef>,
    /// Several targets: a sweep or a line (07 §3.3). Left out, one target.
    #[serde(default)]
    arc: Option<ArcDef>,
    /// Degrees the surface travels through before it lands (07 §6.4). Read and
    /// carried for the package that gives it meaning.
    #[serde(default)]
    swing: Option<Real>,
    /// The band of heights it reaches, in metres above the attacker's footing,
    /// lowest first (07 §4.3). Carried.
    #[serde(default)]
    height: Option<[Real; 2]>,
    /// The id of the phase at whose start it lands (07 §3.1). Carried.
    #[serde(default)]
    lands: Option<Located<String>>,
    /// Whether it can meet another strike in the air (07 §8). Default false.
    /// Carried.
    #[serde(default)]
    clashes: bool,
    /// What a locked clash releases, in joules (07 §8.2). Carried.
    #[serde(default)]
    spill: Option<Real>,
    /// The kind of release a locked clash spills. Carried.
    #[serde(default)]
    spill_kind: Option<String>,
    /// A posture a target the push launches is put in.
    #[serde(default)]
    knock_down: Option<Ref<PostureDef>>,
    /// Effects run once for each target it lands on.
    #[serde(default)]
    on_land: Effects,
    /// Effects run once for each target it misses.
    #[serde(default)]
    on_miss: Effects,
    /// Effects run once for each target it reaches without delivering anything.
    #[serde(default)]
    on_stopped: Effects,
}

/// The `attacks` of an action, in the order written.
#[derive(Debug, Default)]
pub struct Attacks(pub Vec<AttackDef>);

impl<'de> Deserialize<'de> for Attacks {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Attacks, D::Error> {
        struct AttacksVisitor;

        impl<'de> Visitor<'de> for AttacksVisitor {
            type Value = Attacks;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of attacks, each an id and what it strikes with")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Attacks, A::Error> {
                let mut attacks = Vec::new();
                while let Some(id) = map.next_key::<Located<String>>()? {
                    let object = map.next_value::<AttackObject>()?;
                    attacks.push(AttackDef {
                        id,
                        with: object.with,
                        reach: object.reach,
                        energy: object.energy,
                        push: object.push,
                        lift: object.lift,
                        aim: object.aim,
                        arc: object.arc,
                        swing: object.swing,
                        height: object.height,
                        lands: object.lands,
                        clashes: object.clashes,
                        spill: object.spill,
                        spill_kind: object.spill_kind,
                        knock_down: object.knock_down,
                        on_land: object.on_land,
                        on_miss: object.on_miss,
                        on_stopped: object.on_stopped,
                    });
                }
                Ok(Attacks(attacks))
            }
        }

        deserializer.deserialize_map(AttacksVisitor)
    }
}

/// The locals the `strike` effect leaves in the action that runs it
/// (07 §3.4): each name and the type it is declared with when the
/// action does not declare it itself.
pub const RESULT_LOCALS: [(&str, &str); 3] =
    [("landed", "bool"), ("absorbed", "real"), ("passed", "real")];

impl Attacks {
    /// Whether the action has no attacks.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The attacks, in the order written.
    pub fn iter(&self) -> impl Iterator<Item = &AttackDef> {
        self.0.iter()
    }

    /// The attack with this id, and its position.
    pub fn find(&self, id: &str) -> Option<(usize, &AttackDef)> {
        self.0
            .iter()
            .enumerate()
            .find(|(_, attack)| attack.id.value == id)
    }

    /// Checks the ids: well formed and no two alike.
    pub(crate) fn finish(&self, cx: &mut FinishCx<'_>) {
        for (at, attack) in self.0.iter().enumerate() {
            let path = format!("attacks.{}", attack.id.value);
            if !clash_data::is_id_part(&attack.id.value) {
                let diagnostic = Diagnostic::new(
                    code::VALIDATION,
                    format!(
                        "\"{}\" is not an attack id: an id starts with a lowercase letter and holds lowercase letters, digits and `_`",
                        attack.id.value
                    ),
                )
                .at(cx.sources(), attack.id.span)
                .with_path(path.clone());
                cx.report(diagnostic);
            }
            if self.0[..at]
                .iter()
                .any(|before| before.id.value == attack.id.value)
            {
                let diagnostic = Diagnostic::new(
                    code::VALIDATION,
                    format!("this action has two attacks called `{}`", attack.id.value),
                )
                .at(cx.sources(), attack.id.span)
                .with_path(path);
                cx.report(diagnostic);
            }
        }
    }

    /// Compiles every attack: its expressions in `exprs`, which binds
    /// `self`, `weapon`, `limb` and `target` and the action's locals,
    /// and its effect lists in `effects`, which binds `self` and
    /// `target` and the locals.
    pub(crate) fn compile(&mut self, cx: &mut CompileCx<'_>, exprs: &Scope, effects: &Scope) {
        for attack in &mut self.0 {
            cx.compile(&mut attack.reach, exprs);
            cx.compile(&mut attack.energy, exprs);
            cx.compile(&mut attack.push, exprs);
            cx.compile(&mut attack.lift, exprs);
            if let Some(aim) = &mut attack.aim {
                cx.compile(&mut aim.by, exprs);
                cx.compile(&mut aim.against, exprs);
            }
            cx.compile(&mut attack.on_land, effects);
            cx.compile(&mut attack.on_miss, effects);
            cx.compile(&mut attack.on_stopped, effects);
        }
    }

    /// The type's own checks of every attack.
    pub(crate) fn validate(&self, cx: &mut ValidateCx<'_>) {
        for attack in &self.0 {
            let path = format!("attacks.{}", attack.id.value);
            attack.with.validate(&format!("{path}.with"), cx);
            for (name, expr) in [("push", &attack.push), ("energy", &attack.energy)] {
                if let Some(value) = expr.constant().and_then(|value| value.as_number())
                    && !(value.is_finite() && value >= Real::ZERO)
                {
                    cx.error(format!("{path}.{name}: cannot be below zero"));
                }
            }
            if let Some(push) = attack.push.constant().and_then(|value| value.as_number())
                && push > Real::ONE
            {
                cx.error(format!(
                    "{path}.push: is the share of the energy that pushes, from 0 to 1"
                ));
            }
            if let Some(arc) = &attack.arc {
                match arc.shape() {
                    None => cx.error(format!(
                        "{path}.arc: an arc is a `sweep` of some degrees or a `line` of some length, one of the two"
                    )),
                    Some(ArcShape::Sweep(degrees))
                        if !(degrees.is_finite()
                            && degrees > Real::ZERO
                            && degrees <= Real::of(360.0)) =>
                    {
                        cx.error(format!(
                            "{path}.arc.sweep: is an angle in degrees, above 0 and at most 360"
                        ));
                    }
                    Some(ArcShape::Line(length)) if !(length.is_finite() && length > Real::ZERO) => {
                        cx.error(format!("{path}.arc.line: is a length above zero"));
                    }
                    Some(_) => {}
                }
                if arc.max == 0 {
                    cx.error(format!(
                        "{path}.arc.max: an arc strikes at least one target"
                    ));
                }
            }
            if let Some([low, high]) = attack.height
                && !(low.is_finite() && high.is_finite() && low <= high)
            {
                cx.error(format!(
                    "{path}.height: is `[low, high]` in metres above the attacker's footing"
                ));
            }
        }
    }

    /// Every effect list of every attack, in the order written: for each
    /// attack `on_land`, `on_miss` and `on_stopped`.
    pub fn effect_lists(&self) -> impl Iterator<Item = &Effects> {
        self.0
            .iter()
            .flat_map(|attack| [&attack.on_land, &attack.on_miss, &attack.on_stopped])
    }
}
