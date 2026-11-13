//! The `substance` definition type (16 §9.5).

use std::collections::BTreeMap;

use clash_core::{Real, define_id};
use clash_data::{CompileCx, Def, Ref, Scope, Text, Time, ValidateCx};
use serde::Deserialize;

use crate::stats::PoolDef;

define_id! {
    /// A `substance` definition (16 §9.5).
    pub struct SubstanceId(u16);
}

/// The routes into a body (16 §9.5): how a dose gets in.
pub const ROUTES: &[&str] = &["ingest", "inject", "skin", "breath"];

/// What a substance is as other things (16 §9.5 `forms`): the ids of the
/// field it is loose and as vapour, the resource it is piped as and the
/// material it is still. Only `loose` is read: pouring onto a cell adds
/// to that field.
#[derive(Debug, Default, Deserialize)]
pub struct FormsDef {
    /// The field it is as a spill.
    #[serde(default)]
    pub loose: Option<String>,
    /// The field it is in the air. Carried.
    #[serde(default)]
    pub vapour: Option<String>,
    /// The resource it is in pipes and tanks. Carried until networks.
    #[serde(default)]
    pub piped: Option<String>,
    /// The material it is as standing liquid or frozen solid. Carried.
    #[serde(default)]
    pub still: Option<String>,
}

/// One pool a dose adds to (16 §9.5 `adds`).
#[derive(Debug, Deserialize)]
pub struct AddsDef {
    /// The pool.
    pub pool: Ref<PoolDef>,
    /// What each millilitre that gets in adds; negative to take away
    /// (an antidote).
    pub per_ml: Real,
}

/// A trait a dose gives for a time (16 §9.5; 06 §5).
#[derive(Debug, Deserialize)]
pub struct GivesTraitDef {
    /// The trait, by id as written.
    #[serde(rename = "trait")]
    pub given: String,
    /// The least that must get in, in millilitres, for it to be given
    /// (default anything at all).
    #[serde(default)]
    pub at_least_ml: Real,
    /// How long it lasts. Left out, the trait's own `duration`.
    #[serde(default, rename = "for")]
    pub lasts: Option<Time>,
}

/// What a substance does once it is in a body (16 §9.5 `in_body`).
#[derive(Debug, Default, Deserialize)]
pub struct InBodyDef {
    /// For each route, the factor on what gets in: `ingest`, `inject`,
    /// `skin`, `breath`. A route left out gets nothing in.
    #[serde(default)]
    pub routes: BTreeMap<String, Real>,
    /// The pools it adds to.
    #[serde(default)]
    pub adds: Vec<AddsDef>,
    /// The traits it gives.
    #[serde(default)]
    pub traits: Vec<GivesTraitDef>,
}

/// One stuff, whatever form it is in (16 §9.5): what it is called, what
/// it tastes of, what it is as a spill, and what it does in a body.
///
/// ```json
/// { "type": "substance", "id": "nightshade", "name": "nightshade extract",
///   "taste": 0.6, "forms": { "loose": "nightshade_spill" },
///   "in_body": { "routes": { "ingest": 1.0, "inject": 1.5, "skin": 0.05 },
///                "adds": [ { "pool": "toxin", "per_ml": 1.0 } ] } }
/// ```
#[derive(Debug, Deserialize)]
pub struct SubstanceDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What someone who does not know it calls it. Carried.
    #[serde(default)]
    pub unknown_name: Option<Text>,
    /// Its colour, by name. Carried.
    #[serde(default)]
    pub color: Option<String>,
    /// How strongly it tastes, from 0 to 1: `tasted` reports the
    /// strongest in a mixture swallowed.
    #[serde(default)]
    pub taste: Real,
    /// What it is as other things.
    #[serde(default)]
    pub forms: FormsDef,
    /// Field levels a litre of it poured onto a cell adds to its
    /// `loose` field, at most 255 (default 10).
    #[serde(default = "levels_per_litre")]
    pub levels_per_litre: Real,
    /// What it does in a body.
    #[serde(default)]
    pub in_body: InBodyDef,
    /// The share of a coating a hit that breaches the outermost living
    /// layer puts in (default 0.5).
    #[serde(default = "half")]
    pub share: Real,
    /// The share of a coating every hit wipes off (default 0.25).
    #[serde(default = "quarter")]
    pub wipes: Real,
}

fn levels_per_litre() -> Real {
    Real::of(10.0)
}

fn half() -> Real {
    Real::of(0.5)
}

fn quarter() -> Real {
    Real::of(0.25)
}

/// Whether a number is a share from 0 to 1.
fn share(value: Real) -> bool {
    value.is_finite() && value >= Real::ZERO && value <= Real::ONE
}

impl Def for SubstanceDef {
    type Id = SubstanceId;
    const TYPE: &'static str = "substance";
    const DOC: &'static str = "One stuff, whatever form it is in: what it tastes of, what it is as a spill, and what a dose of it does in a body by each route (16 §9.5).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let pure = Scope::new().pure();
        cx.compile(&mut self.name, &pure);
        cx.compile(&mut self.unknown_name, &pure);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !share(self.taste) {
            cx.error("`taste` is from 0 to 1");
        }
        if !share(self.share) {
            cx.error("`share` is from 0 to 1");
        }
        if !share(self.wipes) {
            cx.error("`wipes` is from 0 to 1");
        }
        if !(self.levels_per_litre.is_finite() && self.levels_per_litre >= Real::ZERO) {
            cx.error("`levels_per_litre` is zero or more");
        }
        for (route, factor) in &self.in_body.routes {
            if !ROUTES.contains(&route.as_str()) {
                cx.error(format!(
                    "in_body.routes: `{route}` is not a route: `ingest`, `inject`, `skin` or `breath`"
                ));
            }
            if !(factor.is_finite() && *factor >= Real::ZERO) {
                cx.error(format!("in_body.routes.{route}: a factor is zero or more"));
            }
        }
        for (index, adds) in self.in_body.adds.iter().enumerate() {
            if !adds.per_ml.is_finite() {
                cx.error(format!("in_body.adds {index}: `per_ml` is finite"));
            }
        }
        for (index, given) in self.in_body.traits.iter().enumerate() {
            if !(given.at_least_ml.is_finite() && given.at_least_ml >= Real::ZERO) {
                cx.error(format!("in_body.traits {index}: `at_least_ml` is zero or more"));
            }
            if given.lasts.is_some_and(|lasts| lasts.0.as_micros() <= 0) {
                cx.error(format!("in_body.traits {index}: `for` is above zero"));
            }
        }
    }
}
