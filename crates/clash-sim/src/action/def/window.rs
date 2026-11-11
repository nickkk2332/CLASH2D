//! The `window_kind` definition type (03 §7.1).

use clash_core::Real;
use clash_data::expr::Number;
use clash_data::{CompileCx, Cond, Def, Diagnostic, Expr, Length, Scope, ValidateCx, code};
use serde::Deserialize;

use crate::action::ids::WindowKindId;

/// The name the candidate is bound under in a window kind's expressions.
pub const CANDIDATE_SUBJECT: &str = "self";
/// The name the one whose action or rule opened the window is bound
/// under.
pub const SOURCE_SUBJECT: &str = "source";
/// The name the window itself is bound under.
pub const WINDOW_SUBJECT: &str = "window";

/// A kind of reaction window (03 §7.1): how soon and whether a candidate
/// notices one, and how far the offer of it reaches.
///
/// ```json
/// { "type": "window_kind", "id": "strike",
///   "notice": "self.attr.reaction_time",
///   "noticed_if": "true",
///   "reach": 6 }
/// ```
///
/// `notice` and `noticed_if` are evaluated once for each candidate when a
/// window of the kind opens, reading truth, with these subjects in this
/// order: `self`, the candidate (an `entity`); `source`, whose action or
/// rule opened the window (an `entity`, or null); and `window`, the
/// window (a `window`). Which actions may answer a window of the kind is
/// said by the actions: each lists the kinds in its `reaction_to`.
#[derive(Debug, Deserialize)]
pub struct WindowKindDef {
    /// How long the candidate takes to notice: seconds of the candidate's
    /// own local time. Left out, it is zero: noticed the moment the
    /// window opens.
    #[serde(default = "no_delay")]
    pub notice: Expr<Number>,
    /// Whether the candidate can notice at all. Left out, it always can.
    #[serde(default)]
    pub noticed_if: Cond,
    /// How far, in metres, from the source or from someone the window
    /// names, a candidate the window does not name may be: one who holds
    /// an `intervenes` relation toward someone it names, or a directly
    /// controlled character. Left out, it is zero.
    #[serde(default)]
    pub reach: Length,
}

fn no_delay() -> Expr<Number> {
    Expr::literal(0)
}

impl WindowKindDef {
    /// The scope `notice` and `noticed_if` are compiled in, which is the
    /// order their subjects are supplied in.
    pub fn scope() -> Scope {
        Scope::new()
            .own(CANDIDATE_SUBJECT, "entity")
            .subject(SOURCE_SUBJECT, "entity?")
            .subject(WINDOW_SUBJECT, "window")
    }
}

impl Def for WindowKindDef {
    type Id = WindowKindId;
    const TYPE: &'static str = "window_kind";
    const DOC: &'static str = "A kind of reaction window: how soon and whether it is noticed, and how far its offer reaches (03 §7.1).";
    const DEFAULTS: &'static str = r#"{ "notice": 0, "reach": 0 }"#;

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let scope = WindowKindDef::scope();
        cx.compile(&mut self.notice, &scope);
        cx.compile(&mut self.noticed_if, &scope);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.reach.get() < Real::ZERO {
            cx.report(
                Diagnostic::new(
                    code::VALIDATION,
                    "`reach` is a distance and cannot be below zero",
                )
                .with_path("reach"),
            );
        }
        let negative = self
            .notice
            .constant()
            .and_then(|value| value.as_number())
            .is_some_and(|seconds| seconds < Real::ZERO);
        if negative {
            let mut diagnostic = Diagnostic::new(
                code::VALIDATION,
                "`notice` is a delay and cannot be below zero",
            )
            .with_path("notice");
            if let Some(span) = self.notice.span() {
                diagnostic = diagnostic.at(cx.sources(), span);
            }
            cx.report(diagnostic);
        }
    }

    fn placeholder() -> Option<WindowKindDef> {
        // Inert: nobody notices a window of a kind that did not load.
        Some(WindowKindDef {
            notice: no_delay(),
            noticed_if: Cond::constant(false),
            reach: Length::default(),
        })
    }
}
