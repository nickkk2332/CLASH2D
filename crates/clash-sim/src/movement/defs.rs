//! The definition types of 06 §11: `movement`, a way of moving, and
//! `posture`, how a body holds itself.

use clash_core::{Real, define_id};
use clash_data::expr::Number;
use clash_data::{CompileCx, Def, Expr, FinishCx, Length, Ref, Scope, Text, ValidateCx};
use clash_world::defs::WALK_STEP_M;
use clash_world::query::PROFILE_ROUGH_FACTOR;
use serde::Deserialize;

use crate::action::def::ChannelUse;

define_id! {
    /// A `movement` definition (06 §11).
    pub struct MovementId(u16);
    /// A `posture` definition (06 §11.1).
    pub struct PostureId(u16);
}

/// The kind of subject a mover is: what `self` is in a movement's
/// `speed`.
pub const ENTITY_KIND: &str = "entity";

/// Where a way of moving takes its mover (06 §11).
///
/// | Surface | The mover may be in | Steps it makes |
/// |---------|---------------------|----------------|
/// | `standable` | A cell with footing, nothing in the way taller than `step_up`, not liquid | On the level; up from and down into a cell that holds a ramp or stairs |
/// | `climbable` | A cell that holds a ramp, stairs or ladder, or the cell at the top of one | Up from and down into a cell that holds any of them |
/// | `liquid` | A liquid cell | On the level into liquid; up and down between liquid cells |
/// | `open` | A cell with nothing in the way taller than `step_up` | On the level; up and down where no floor and no solid top lies between |
/// | `solid` | Any cell | Any |
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// On what can be stood on: walking, crawling, running.
    #[default]
    Standable,
    /// Up and down what can be climbed.
    Climbable,
    /// Through liquid: swimming.
    Liquid,
    /// Through open space: flying.
    Open,
    /// Through anything: burrowing, phasing.
    Solid,
}

impl Surface {
    /// The name as data writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Surface::Standable => "standable",
            Surface::Climbable => "climbable",
            Surface::Liquid => "liquid",
            Surface::Open => "open",
            Surface::Solid => "solid",
        }
    }
}

fn default_step_up() -> Length {
    Length::new(WALK_STEP_M)
}

fn default_rough() -> Real {
    PROFILE_ROUGH_FACTOR
}

fn one() -> Real {
    Real::ONE
}

/// A way of moving (06 §11): where it takes its mover, how fast, and
/// what it needs.
///
/// ```json
/// { "type": "movement", "id": "walk",
///   "surface": "standable",
///   "speed": "1.4 / (1 + self.carried_mass / 60)",
///   "step_up": 0.6,
///   "uses": { "leg": "all" },
///   "noise": 0.3,
///   "default": true }
/// ```
#[derive(Debug, Deserialize)]
pub struct MovementDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// Where it takes its mover.
    #[serde(default)]
    pub surface: Surface,
    /// How fast, in metres for each second of the mover's own time: an
    /// expression about the mover (`self`), worked out whenever a step
    /// is priced. Zero, less or null means the mover cannot move this
    /// way now.
    pub speed: Expr<Number>,
    /// The tallest thing in a cell that the mover steps over, in metres.
    /// Left out, what ordinary walking steps over.
    #[serde(default = "default_step_up")]
    pub step_up: Length,
    /// How many times longer a step into a rough cell takes. At least 1.
    #[serde(default = "default_rough")]
    pub rough: Real,
    /// How many times longer a step between storeys takes. At least 1.
    #[serde(default = "one")]
    pub climb: Real,
    /// The channel slots the mover must have working to move this way
    /// (03 §6.2). Checked against a body's slots; a mover with no body
    /// is not asked.
    #[serde(default)]
    pub(super) uses: ChannelUse,
    /// How loud a step is, as a factor for signals (15 §3). Carried for
    /// the perception package.
    #[serde(default)]
    pub noise: Real,
    /// Whether a mover that lists no ways of moving of its own moves
    /// this way.
    #[serde(default)]
    pub default: bool,
}

impl MovementDef {
    /// The channel slots the mover must have working.
    pub fn uses(&self) -> &ChannelUse {
        &self.uses
    }

    /// The scope of `speed`: the mover as `self`, read as it is, with
    /// nothing drawn at random.
    pub fn scope() -> Scope {
        Scope::new().own("self", ENTITY_KIND).pure()
    }
}

impl Def for MovementDef {
    type Id = MovementId;
    const TYPE: &'static str = "movement";
    const DOC: &'static str =
        "A way of moving: where it takes its mover, how fast, and what it needs (06 §11).";

    fn finish(&mut self, cx: &mut FinishCx<'_>) {
        self.uses.finish("uses", cx);
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        cx.compile(&mut self.speed, &MovementDef::scope());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        let step_up = self.step_up.get();
        if !(step_up.is_finite() && step_up >= Real::ZERO) {
            cx.error("`step_up` is a height in metres of zero or more");
        }
        for (name, factor) in [("rough", self.rough), ("climb", self.climb)] {
            if !(factor.is_finite() && factor >= Real::ONE) {
                cx.error(format!(
                    "`{name}` says how many times longer a step takes: a number of at least 1"
                ));
            }
        }
        if !(self.noise.is_finite() && self.noise >= Real::ZERO) {
            cx.error("`noise` is a factor of zero or more");
        }
        if self
            .speed
            .constant()
            .and_then(clash_expr::Value::as_number)
            .is_some_and(|speed| speed <= Real::ZERO)
        {
            cx.warn("a `speed` of zero or less means nothing ever moves this way");
        }
    }
}

/// How a body holds itself (06 §11.1).
///
/// ```json
/// { "type": "posture", "id": "prone", "height": 0.2, "speed": 0.15,
///   "signature": 0.3, "movement": ["crawl"], "occupies": { "leg": "all" } }
/// ```
#[derive(Debug, Deserialize)]
pub struct PostureDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// The share of its standing height the body has in this posture.
    /// Above zero.
    #[serde(default = "one")]
    pub height: Real,
    /// A factor on the speed of every way of moving. Zero or more.
    #[serde(default = "one")]
    pub speed: Real,
    /// The ways of moving allowed in this posture. Empty for all.
    #[serde(default)]
    pub movement: Vec<Ref<MovementDef>>,
    /// A factor on how easily the body is perceived (15 §5.3). Carried
    /// for the perception package.
    #[serde(default = "one")]
    pub signature: Real,
    /// The channel slots that cannot be used while in this posture
    /// (03 §6.2).
    #[serde(default)]
    pub(super) occupies: ChannelUse,
    /// Traits the posture carries, by id, as written. Carried until
    /// traits are joined to postures.
    #[serde(default)]
    pub traits: Vec<String>,
    /// How the body is drawn in it (12 §4.4), as written. Reported with
    /// `posture_changed`.
    #[serde(default)]
    pub look: Option<String>,
}

impl PostureDef {
    /// The channel slots that cannot be used while in this posture.
    pub fn occupies(&self) -> &ChannelUse {
        &self.occupies
    }

    /// Whether a way of moving is allowed in this posture.
    pub fn allows(&self, movement: MovementId) -> bool {
        self.movement.is_empty() || self.movement.iter().any(|allowed| allowed.id() == movement)
    }
}

impl Def for PostureDef {
    type Id = PostureId;
    const TYPE: &'static str = "posture";
    const DOC: &'static str = "How a body holds itself: what that makes of its height, its speed, how it is perceived and what it can use (06 §11.1).";

    fn finish(&mut self, cx: &mut FinishCx<'_>) {
        self.occupies.finish("occupies", cx);
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !(self.height.is_finite() && self.height > Real::ZERO) {
            cx.error(
                "`height` is the share of its standing height a body has: a number above zero",
            );
        }
        for (name, factor) in [("speed", self.speed), ("signature", self.signature)] {
            if !(factor.is_finite() && factor >= Real::ZERO) {
                cx.error(format!("`{name}` is a factor of zero or more"));
            }
        }
    }
}
