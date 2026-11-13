//! What ranged combat reads from definitions (07 §11): the `projectile`
//! type, and the `round` and `shot` blocks of items.
//!
//! None of it is a gun, a bow or a bullet. A projectile is a mass with a
//! cross-section and a striking surface; a round is an item that names a
//! projectile and an energy; a `shot` block is the numbers a shot fired
//! through an item takes. Content makes firearms, bows and slings of
//! them.

use clash_core::{Real, define_id};
use clash_data::{CompileCx, Def, Energy, Length, Mass, Ref, Scope, Text, Time, ValidateCx};
use clash_world::defs::DamageTypeDef;
use clash_world::field::FieldDef;
use serde::Deserialize;

use crate::item::ItemDef;
use crate::rule::Effects;

define_id! {
    /// A `projectile` definition (07 §11.1).
    pub struct ProjectileId;
}

/// The name `self` takes in a projectile's `on_impact`: whoever fired
/// it.
pub const SHOOTER_SUBJECT: &str = "self";
/// The name the cell where a projectile struck takes in its
/// `on_impact`.
pub const HERE_SUBJECT: &str = "here";
/// The name the creature a projectile struck takes in its `on_impact`.
pub const STRUCK_SUBJECT: &str = "target";

fn one_count() -> u16 {
    1
}

fn default_drag() -> Real {
    clash_world::BODY_DEFAULT_DRAG
}

/// Whether a number is finite and above zero.
fn positive(value: Real) -> bool {
    value.is_finite() && value > Real::ZERO
}

/// Whether a number is finite and at least zero.
fn amount(value: Real) -> bool {
    value.is_finite() && value >= Real::ZERO
}

/// What a projectile strikes with (07 §11.1, §3.2): a damage type, a
/// contact area and a give, and a span across what it strikes.
#[derive(Debug, Deserialize)]
pub struct ProjectileSurface {
    /// The damage type it delivers.
    pub kind: Ref<DamageTypeDef>,
    /// Contact area, in square metres.
    pub area: Real,
    /// The distance over which it is stopped.
    pub give: Length,
    /// The length of contact across what it strikes: what decides
    /// whether it can sever. Default 0.
    #[serde(default)]
    pub span: Length,
}

/// A field a projectile leaves where it comes to an end (07 §11.1): fire
/// where an incendiary lands, smoke where a canister bursts.
#[derive(Debug, Deserialize)]
pub struct LeavesDef {
    /// The field.
    pub field: Ref<FieldDef>,
    /// How much of it is added, in levels from 1 to 255. Default 1.
    #[serde(default = "one_level")]
    pub amount: i16,
}

fn one_level() -> i16 {
    1
}

/// Thrust along a projectile's path for a time (07 §11.1): the world's
/// drive (02 §15).
#[derive(Debug, Deserialize)]
pub struct ThrustDef {
    /// The acceleration it gives along the path, in m/s².
    pub accel: Real,
    /// How long it lasts.
    pub burn: Time,
}

/// How a projectile steers (07 §11.1). Read and carried: guidance is
/// not built.
#[derive(Debug, Deserialize)]
pub struct GuidanceDef {
    /// What it steers for: `point`, `seeker`, `launcher` or `return`.
    pub by: String,
    /// For a seeker, the kind of signal it follows.
    #[serde(default)]
    pub sense: Option<String>,
    /// For a seeker, the cone it senses in, in degrees.
    #[serde(default)]
    pub cone: Option<Real>,
    /// For a seeker, how far it senses, in metres.
    #[serde(default)]
    pub range: Option<Real>,
    /// How fast it turns, in radians a second.
    #[serde(default)]
    pub turn: Real,
}

/// A kind of projectile (07 §11.1): what flies when something shoots.
///
/// ```json
/// { "type": "projectile", "id": "musket_ball",
///   "mass": "0.03 kg", "area": 2.4e-4, "drag": 0.5,
///   "surface": { "kind": "pierce", "area": 2.4e-4, "give": 0.01, "span": 0.017 } }
/// ```
///
/// | Field | Meaning |
/// |-------|---------|
/// | `name` | What a reader calls it |
/// | `mass` | Its mass, required |
/// | `area` | Its cross-section, m²: what drag acts on and how wide it is to what closes part of a cell. Default its surface's area |
/// | `drag` | Its drag coefficient. Default 1 |
/// | `restitution` | How much of its speed it keeps when it is turned back, 0 to 1. Default 0 |
/// | `surface` | What it strikes with, required |
/// | `leaves` | A field it leaves where it comes to an end |
/// | `on_impact` | Effects run at the first thing it strikes; it ends there |
/// | `thrust` | A drive along its path for a time |
/// | `guidance`, `fuse` | Read and carried |
/// | `item` | What it is as an item once caught in flight |
#[derive(Debug, Deserialize)]
pub struct ProjectileDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// Its mass.
    pub mass: Mass,
    /// Its cross-section, in square metres. Left out, its surface's
    /// area.
    #[serde(default)]
    pub area: Option<Real>,
    /// Its drag coefficient.
    #[serde(default = "default_drag")]
    pub drag: Real,
    /// How much of its speed along the direction it struck in it keeps
    /// when it is turned back, from 0 to 1.
    #[serde(default)]
    pub restitution: Real,
    /// What it strikes with.
    pub surface: ProjectileSurface,
    /// A field it leaves where it comes to an end.
    #[serde(default)]
    pub leaves: Option<LeavesDef>,
    /// Effects run at the first thing it strikes, with `self` the one
    /// who fired it, `here` the cell and `target` the creature struck,
    /// if it was one. It ends there.
    #[serde(default)]
    pub on_impact: Effects,
    /// A drive along its path.
    #[serde(default)]
    pub thrust: Option<ThrustDef>,
    /// How it steers. Read and carried.
    #[serde(default)]
    pub guidance: Option<GuidanceDef>,
    /// When it bursts on its own. Read and carried: a timed charge is
    /// thrown and timed by a rule's `after` (07 §11.5).
    #[serde(default)]
    pub fuse: Option<Time>,
    /// What it is as an item once it is caught in flight (07 §11.6,
    /// `catch_shot`): an arrow caught is an arrow in the hand. Left out,
    /// a projectile caught is stopped and becomes nothing.
    #[serde(default)]
    pub item: Option<Ref<ItemDef>>,
}

impl ProjectileDef {
    /// Its cross-section, in square metres.
    pub fn cross_section(&self) -> Real {
        self.area.unwrap_or(self.surface.area)
    }

    /// The scope its `on_impact` is compiled in: `self`, the one who
    /// fired it; `here`, the cell where it struck; `target`, the creature
    /// struck, null for an element.
    pub fn impact_scope() -> Scope {
        Scope::new()
            .own(SHOOTER_SUBJECT, "entity")
            .subject(HERE_SUBJECT, "loc")
            .subject(STRUCK_SUBJECT, "entity")
    }
}

impl Def for ProjectileDef {
    type Id = ProjectileId;
    const TYPE: &'static str = "projectile";
    const DOC: &'static str = "A kind of projectile: a mass with a cross-section and a striking surface that flies as a body when something shoots (07 §11.1).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        cx.compile(&mut self.on_impact, &ProjectileDef::impact_scope());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !positive(self.mass.get()) {
            cx.error("`mass` must be above zero");
        }
        if self.area.is_some_and(|area| !amount(area)) {
            cx.error("`area` must be a finite amount of zero or more");
        }
        if !amount(self.drag) {
            cx.error("`drag` must be a finite amount of zero or more");
        }
        if !(Real::ZERO..=Real::ONE).contains(&self.restitution) {
            cx.error("`restitution` is a share, from 0 to 1");
        }
        if !positive(self.surface.area) || !positive(self.surface.give.get()) {
            cx.error("surface: `area` and `give` must be above zero");
        }
        if !amount(self.surface.span.get()) {
            cx.error("surface: `span` must not be negative");
        }
        if let Some(leaves) = &self.leaves
            && !(1..=255).contains(&leaves.amount)
        {
            cx.error("leaves: `amount` is a number of levels, from 1 to 255");
        }
        if let Some(thrust) = &self.thrust
            && (!amount(thrust.accel) || thrust.burn.dur() < clash_core::Dur::ZERO)
        {
            cx.error("thrust: `accel` and `burn` must not be negative");
        }
        self.on_impact.validate(cx);
    }
}

/// What a round fires (07 §11.5): the `round` block of an item.
///
/// ```json
/// "round": { "projectile": "ap_bullet", "energy": "3.4 kJ", "count": 1 }
/// ```
#[derive(Debug, Deserialize)]
pub struct RoundDef {
    /// The projectile it launches.
    pub projectile: Ref<ProjectileDef>,
    /// The energy it gives each projectile, before the weapon's
    /// efficiency.
    pub energy: Energy,
    /// How many projectiles it launches at once: shot in a cartridge.
    /// Default 1.
    #[serde(default = "one_count")]
    pub count: u16,
    /// What it leaves where it was fired from, if anything: a casing.
    #[serde(default)]
    pub casing: Option<Ref<ItemDef>>,
}

impl RoundDef {
    /// Checks the block, as part of its item.
    pub fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !positive(self.energy.get()) {
            cx.error("round: `energy` must be above zero");
        }
        if self.count == 0 {
            cx.error("round: `count` must be at least 1");
        }
    }
}

/// The numbers a shot fired through an item takes (07 §11.5): the `shot`
/// block of an item.
///
/// On what fires (a firearm, a bow) they are what it gives a shot; on
/// what is fitted to it (a scope, a suppressor) they are factors. A
/// shot through an item takes the product of the item's and those of
/// everything fitted to it (held `fixed` in it) that state the number.
///
/// ```json
/// "shot": { "spread": 0.002, "efficiency": 0.9, "noise": 140, "flash": 1, "fits": ["sight", "muzzle"] }
/// "shot": { "spread": 0.3 }
/// ```
#[derive(Debug, Deserialize)]
pub struct ShotDef {
    /// The spread of a shot, in radians: the standard deviation of each
    /// of its two angles of dispersion (07 §11.1).
    #[serde(default)]
    pub spread: Option<Real>,
    /// The share of a round's energy that goes into its projectiles.
    #[serde(default)]
    pub efficiency: Option<Real>,
    /// How loud a shot is: the strength of the sound it gives off, in
    /// the units of the signal kind content uses (15 §3.2).
    #[serde(default)]
    pub noise: Option<Real>,
    /// How bright a shot is: the strength of the light it gives off.
    #[serde(default)]
    pub flash: Option<Real>,
    /// The most energy it puts into a shot of its own: a bow at full
    /// draw, a sling.
    #[serde(default)]
    pub energy: Option<Energy>,
    /// The tags of what may be fitted to it, one of each: `sight`,
    /// `muzzle`, a magazine's.
    #[serde(default)]
    pub fits: Vec<String>,
}

/// The names of the numbers of a `shot` block, as `entity.shot(name)`
/// takes them.
pub const SHOT_NUMBERS: [&str; 5] = ["spread", "efficiency", "noise", "flash", "energy"];

impl ShotDef {
    /// One of its numbers, by name, if it states it.
    pub fn number(&self, name: &str) -> Option<Real> {
        match name {
            "spread" => self.spread,
            "efficiency" => self.efficiency,
            "noise" => self.noise,
            "flash" => self.flash,
            "energy" => self.energy.map(|energy| energy.get()),
            _ => None,
        }
    }

    /// Checks the block, as part of its item.
    pub fn validate(&self, cx: &mut ValidateCx<'_>) {
        for name in SHOT_NUMBERS {
            if self.number(name).is_some_and(|value| !amount(value)) {
                cx.error(format!(
                    "shot: `{name}` must be a finite amount of zero or more"
                ));
            }
        }
        if self.fits.iter().any(String::is_empty) {
            cx.error("shot: `fits` names tags, and one is empty");
        }
    }
}
