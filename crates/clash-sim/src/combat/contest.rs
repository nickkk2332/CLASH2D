//! Contests (07 §4.1): two numbers compared, with as much or as little
//! chance as content gives them.
//!
//! A [`ContestDef`] says how to compare: by their difference or by the
//! logarithm of their ratio, scaled, with noise of a given spread. The
//! **margin** is that comparison plus the noise, and the chance it is at
//! or above zero is the contest's **odds**:
//!
//! ```text
//! d      = factor × (a − b)              "form": "difference"
//!        = factor × (ln a − ln b)        "form": "ratio"
//! margin = d + spread × ln(u / (1 − u))  u drawn uniformly from (0, 1)
//! odds   = 1 / (1 + e^(−d / spread))     the chance that margin ≥ 0
//! ```
//!
//! The noise is logistic: its spread is the logistic distribution's
//! scale, so that the odds have the closed form above and a margin can
//! be drawn with one number. A spread of zero is a pure comparison: the
//! margin is `d` and the odds are 1 when `d` is at or above zero and 0
//! otherwise.
//!
//! In the ratio form, a value at or below zero is less than any value
//! above zero: `d` is minus infinity if only `a` is, plus infinity if
//! only `b` is, and zero if both are.
//!
//! Every contest draws exactly one number, whatever its spread, so that
//! the stream it draws from stays in step when content changes a spread.

use clash_core::{Real, Rng, define_id};
use clash_data::{CompileCx, Def, Scope, Text, ValidateCx};
use serde::Deserialize;

define_id! {
    /// A `contest` definition (07 §4.1).
    pub struct ContestId(u16);
}

/// What a contest compares (07 §4.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContestForm {
    /// The difference of the two values, scaled.
    #[default]
    Difference,
    /// The natural logarithm of their ratio, scaled: what matters is how
    /// many times one is the other.
    Ratio,
}

fn one() -> Real {
    Real::ONE
}

/// A way to compare two numbers with chance (07 §4.1).
///
/// ```json
/// { "type": "contest", "id": "melee_aim", "form": "difference",
///   "factor": 4, "spread": 1.0, "near": 0.5 }
/// ```
///
/// | Field | Meaning |
/// |-------|---------|
/// | `name` | What a reader calls it. Optional. |
/// | `form` | `difference` (the default) or `ratio` |
/// | `factor` | What the difference or log-ratio is multiplied by. Above zero; default 1. (Not `scale`, which is the loader's word for multiplying a field, 04 §6.1.) |
/// | `spread` | The scale of the logistic noise added. Zero or more; default 1. Zero is a pure comparison. |
/// | `near` | For aim (07 §4.2): how far below zero a margin may be and still land on a part beside the one aimed at. Zero or more; default 0. |
#[derive(Debug, Deserialize)]
pub struct ContestDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What is compared.
    #[serde(default)]
    pub form: ContestForm,
    /// What the comparison is multiplied by.
    #[serde(default = "one")]
    pub factor: Real,
    /// The scale of the noise added to the comparison.
    #[serde(default = "one")]
    pub spread: Real,
    /// How far below zero a margin of aim may be and still land beside
    /// the part aimed at (07 §4.2).
    #[serde(default)]
    pub near: Real,
}

impl ContestDef {
    /// The comparison without noise: the scaled difference, or the
    /// scaled log-ratio.
    pub fn difference(&self, a: Real, b: Real) -> Real {
        match self.form {
            ContestForm::Difference => self.factor * (a - b),
            ContestForm::Ratio => {
                let (low_a, low_b) = (a <= Real::ZERO, b <= Real::ZERO);
                let d = match (low_a, low_b) {
                    (true, true) => Real::ZERO,
                    (true, false) => -Real::INFINITY,
                    (false, true) => Real::INFINITY,
                    (false, false) => a.ln() - b.ln(),
                };
                if d.is_finite() { self.factor * d } else { d }
            }
        }
    }

    /// A margin (07 §4.1): the comparison plus noise of the contest's
    /// spread, drawn with one number from `rng`.
    pub fn margin(&self, a: Real, b: Real, rng: &mut Rng) -> Real {
        let noise = logistic(rng);
        let d = self.difference(a, b);
        if self.spread > Real::ZERO && d.is_finite() {
            d + self.spread * noise
        } else {
            d
        }
    }

    /// The chance that a margin is at or above zero. Pure: it draws
    /// nothing.
    pub fn odds(&self, a: Real, b: Real) -> Real {
        let d = self.difference(a, b);
        if self.spread > Real::ZERO && d.is_finite() {
            Real::ONE / (Real::ONE + (-d / self.spread).exp())
        } else if d >= Real::ZERO {
            Real::ONE
        } else {
            Real::ZERO
        }
    }
}

/// One draw of standard logistic noise: `ln(u / (1 − u))` for `u`
/// uniform on the open interval (0, 1).
fn logistic(rng: &mut Rng) -> Real {
    // Fifty-three bits, and half a step more, so that `u` is never 0 or 1.
    const STEP: Real = Real::of(1.0 / 9_007_199_254_740_992.0);
    let bits = Real::from_i64(i64::try_from(rng.next_u64() >> 11).unwrap_or(0));
    let u = (bits + Real::of(0.5)) * STEP;
    (u / (Real::ONE - u)).ln()
}

impl Def for ContestDef {
    type Id = ContestId;
    const TYPE: &'static str = "contest";
    const DOC: &'static str = "A comparison of two numbers with chance: their difference or log-ratio, scaled, with logistic noise of a given spread (07 §4.1).";
    const DEFAULTS: &'static str = r#"{ "factor": 1, "spread": 1, "near": 0 }"#;

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !(self.factor.is_finite() && self.factor > Real::ZERO) {
            cx.error("`factor` multiplies the comparison: a number above zero");
        }
        if !(self.spread.is_finite() && self.spread >= Real::ZERO) {
            cx.error("`spread` is the scale of the noise: a number of zero or more");
        }
        if !(self.near.is_finite() && self.near >= Real::ZERO) {
            cx.error("`near` is how far below zero a margin may land beside its mark: a number of zero or more");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contest(form: ContestForm, scale: f64, spread: f64) -> ContestDef {
        ContestDef {
            name: None,
            form,
            factor: Real::of(scale),
            spread: Real::of(spread),
            near: Real::ZERO,
        }
    }

    #[test]
    fn odds_match_the_frequency_of_margins_at_or_above_zero() {
        let mut rng = Rng::from_seed(17);
        let draws = 40_000;
        for (form, scale, spread, a, b) in [
            (ContestForm::Difference, 4.0, 1.0, 1.2, 1.0),
            (ContestForm::Difference, 1.0, 2.0, 0.0, 3.0),
            (ContestForm::Difference, 1.0, 0.5, 5.0, 5.0),
            (ContestForm::Ratio, 2.0, 1.0, 30.0, 10.0),
            (ContestForm::Ratio, 1.0, 0.25, 4.0, 5.0),
        ] {
            let def = contest(form, scale, spread);
            let (a, b) = (Real::of(a), Real::of(b));
            let hits = (0..draws)
                .filter(|_| def.margin(a, b, &mut rng) >= Real::ZERO)
                .count();
            let observed = hits as f64 / f64::from(draws);
            let odds = def.odds(a, b).get();
            // Four standard errors of a frequency over this many draws.
            let tolerance = 4.0 * (odds * (1.0 - odds) / f64::from(draws)).sqrt() + 1e-9;
            assert!(
                (observed - odds).abs() <= tolerance.max(0.005),
                "{form:?} {a} {b}: odds {odds}, observed {observed}"
            );
        }
    }

    #[test]
    fn a_spread_of_zero_is_a_pure_comparison() {
        let def = contest(ContestForm::Difference, 3.0, 0.0);
        let mut rng = Rng::from_seed(5);
        for _ in 0..100 {
            assert_eq!(
                def.margin(Real::of(2.0), Real::of(1.5), &mut rng),
                Real::of(1.5)
            );
            assert_eq!(
                def.margin(Real::of(1.0), Real::of(1.5), &mut rng),
                Real::of(-1.5)
            );
        }
        assert_eq!(def.odds(Real::of(2.0), Real::of(1.5)), Real::ONE);
        assert_eq!(def.odds(Real::of(1.5), Real::of(1.5)), Real::ONE);
        assert_eq!(def.odds(Real::of(1.0), Real::of(1.5)), Real::ZERO);
    }

    #[test]
    fn a_ratio_puts_nothing_below_anything() {
        let def = contest(ContestForm::Ratio, 1.0, 1.0);
        assert_eq!(def.odds(Real::ZERO, Real::of(3.0)), Real::ZERO);
        assert_eq!(def.odds(Real::of(3.0), Real::ZERO), Real::ONE);
        assert_eq!(def.odds(Real::ZERO, Real::ZERO), Real::of(0.5));
        let mut rng = Rng::from_seed(9);
        assert_eq!(
            def.margin(Real::ZERO, Real::of(3.0), &mut rng),
            -Real::INFINITY
        );
    }

    #[test]
    fn every_contest_draws_once() {
        let pure = contest(ContestForm::Difference, 1.0, 0.0);
        let noisy = contest(ContestForm::Difference, 1.0, 1.0);
        let (mut a, mut b) = (Rng::from_seed(3), Rng::from_seed(3));
        pure.margin(Real::ONE, Real::ZERO, &mut a);
        noisy.margin(Real::ONE, Real::ZERO, &mut b);
        assert_eq!(a.next_u64(), b.next_u64());
    }
}
