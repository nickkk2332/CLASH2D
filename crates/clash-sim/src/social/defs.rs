//! The definition types of 15 §9 and §11: `relation_kind`, `group`,
//! `message_kind` and `lines`.

use std::collections::BTreeMap;

use clash_core::{Real, define_id};
use clash_data::{
    CompileCx, Cond, CrossCx, Def, DefLabel, DefsView, Diagnostic, Ref, Scope, Text, ValidateCx,
    code,
};
use serde::Deserialize;

use crate::perception::SignalKindDef;

define_id! {
    /// A `relation_kind` definition (15 §9.1).
    pub struct RelationKindId(u16);
    /// A `group` definition (15 §9.2).
    pub struct GroupDefId(u16);
    /// A `message_kind` definition (15 §11).
    pub struct MessageKindId(u16);
    /// A `lines` definition (15 §11).
    pub struct LinesId(u16);
}

/// Whether a relation kind holds a number or is only there or not.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationShape {
    /// A number within the kind's `range`.
    #[default]
    Value,
    /// 1 while held, 0 otherwise: `commands`, `guards`.
    Flag,
}

fn full_range() -> [Real; 2] {
    [-Real::ONE, Real::ONE]
}

/// A kind of relation one mind holds toward a persona or a group
/// (15 §9.1): fear, trust, loyalty, command. What each means is the
/// content's; the engine reads only the five flags.
///
/// ```json
/// { "type": "relation_kind", "id": "fear", "range": [0, 1], "default": 0, "fade": 0.02 }
/// { "type": "relation_kind", "id": "loyal", "kind": "flag", "orders": true }
/// ```
#[derive(Debug, Deserialize)]
pub struct RelationKindDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// `value` (the default), a number within `range`; or `flag`, 1
    /// while held.
    #[serde(default)]
    pub kind: RelationShape,
    /// The least and the most it can be. Default `[-1, 1]`; a flag's is
    /// `[0, 1]` whatever is written.
    #[serde(default = "full_range")]
    pub range: [Real; 2],
    /// What it is when nobody has set it, and what it fades toward.
    /// Default 0.
    #[serde(default)]
    pub default: Real,
    /// How far it moves toward `default` each minute of the holder's own
    /// time. Default 0: it does not fade.
    #[serde(default)]
    pub fade: Real,
    /// Held above its default, it makes the one it is held toward an
    /// enemy (09 §3.4).
    #[serde(default)]
    pub opposes: bool,
    /// Held above its default, it makes the one it is held toward a
    /// friend (09 §3.4).
    #[serde(default)]
    pub sides_with: bool,
    /// Held above its default by the one an order is given to, toward
    /// the one it takes for the giver, it makes the order one to obey
    /// (15 §10).
    #[serde(default)]
    pub orders: bool,
    /// Held above its default toward someone, it makes the holder a
    /// candidate for windows opened against them (15 §9.1; 03 §7.1).
    #[serde(default)]
    pub intervenes: bool,
    /// Held above its default toward someone, it makes them one to keep
    /// away from: the AI's `feared` stance (09 §6.6).
    #[serde(default)]
    pub avoids: bool,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: String,
}

impl RelationKindDef {
    /// The least and the most it can be.
    pub fn bounds(&self) -> (Real, Real) {
        match self.kind {
            RelationShape::Flag => (Real::ZERO, Real::ONE),
            RelationShape::Value => (self.range[0], self.range[1]),
        }
    }

    /// What it is when nobody has set it.
    pub fn default_value(&self) -> Real {
        match self.kind {
            RelationShape::Flag => Real::ZERO,
            RelationShape::Value => self.default,
        }
    }
}

impl Def for RelationKindDef {
    type Id = RelationKindId;
    const TYPE: &'static str = "relation_kind";
    const DOC: &'static str = "A kind of relation a mind holds toward a persona or a group: its range, default, fade, and the flags `opposes`, `sides_with`, `avoids`, `orders` and `intervenes` the engine reads (15 §9.1).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        let [low, high] = self.range;
        if !low.is_finite() || !high.is_finite() || low >= high {
            cx.error("`range` is two finite numbers, the first below the second");
        } else if self.kind == RelationShape::Value && (self.default < low || self.default > high) {
            cx.error("`default` is within `range`");
        }
        if !self.fade.is_finite() || self.fade < Real::ZERO {
            cx.error("`fade` must be a number of zero or more");
        }
        if self.opposes && self.sides_with {
            cx.error("a relation kind cannot both oppose and side with");
        }
    }
}

/// A role in a group (15 §9.2).
#[derive(Debug, Default, Deserialize)]
pub struct RoleDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// The roles of the same group whose members this role may give
    /// orders to.
    #[serde(default)]
    pub orders: Vec<String>,
    /// Whether a member in this role may pay from what the group holds
    /// (16 §5.6).
    #[serde(default)]
    pub spends: bool,
}

/// A body of people (15 §9.2): a watch, a gang, a cult, a regiment.
///
/// ```json
/// { "type": "group", "id": "watch", "name": "the Watch",
///   "roles": { "sergeant": { "orders": ["guard"], "spends": true }, "guard": {} },
///   "relations": { "gang": { "hostile": 1 } } }
/// ```
#[derive(Debug, Deserialize)]
pub struct GroupDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// Its roles, by id.
    #[serde(default)]
    pub roles: BTreeMap<String, RoleDef>,
    /// The mind that decides for it (09 §8.1), as written: for the AI.
    #[serde(default)]
    pub collective: Option<String>,
    /// What it holds toward other groups (and itself) from the start: for
    /// each group, by id, the value of each relation kind, by id.
    #[serde(default)]
    pub relations: BTreeMap<String, BTreeMap<String, Real>>,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: String,
}

impl Def for GroupDef {
    type Id = GroupDefId;
    const TYPE: &'static str = "group";
    const DOC: &'static str = "A body of people: its roles, who in it may order whom, and what it holds toward other groups from the start (15 §9.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        for role in self.roles.values_mut() {
            cx.compile(&mut role.name, &Scope::new().pure());
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        for (id, role) in &self.roles {
            for ordered in &role.orders {
                if !self.roles.contains_key(ordered) {
                    cx.error(format!(
                        "the role `{id}` orders `{ordered}`, which is not a role of the group"
                    ));
                }
            }
        }
        for values in self.relations.values() {
            if values.values().any(|value| !value.is_finite()) {
                cx.error("a value in `relations` must be a finite number");
            }
        }
    }
}

/// What a message of a kind carries for the engine to act on (15 §11).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carries {
    /// Nothing: rules listening for it do whatever they do.
    #[default]
    Nothing,
    /// A memory or a track, which the hearer gains as told.
    Report,
    /// An order (15 §10).
    Order,
}

fn half() -> Real {
    Real::of(0.5)
}

fn one() -> Real {
    Real::ONE
}

/// A kind of message (15 §11): a report, an order, a warning, a greeting.
///
/// ```json
/// { "type": "message_kind", "id": "report", "carries": "report", "belief": 0.5 }
/// ```
#[derive(Debug, Deserialize)]
pub struct MessageKindDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// `nothing` (the default), `report` or `order`.
    #[serde(default)]
    pub carries: Carries,
    /// For a report: the share of the speaker's confidence the hearer
    /// believes it with. Default 0.5.
    #[serde(default = "half")]
    pub belief: Real,
    /// For a report: a relation kind whose value, held by the hearer
    /// toward the speaker as it took them, scales `belief` by where it
    /// stands in its range. Left out, `belief` alone.
    #[serde(default)]
    pub trust: Option<Ref<RelationKindDef>>,
    /// The kind of signal it is said with. Left out, the kind flagged
    /// `noise` (15 §3.2).
    #[serde(default)]
    pub signal: Option<Ref<SignalKindDef>>,
    /// How strong it is said, in the units of the signal kind, when the
    /// one who says it gives no strength. Default 1.
    #[serde(default = "one")]
    pub strength: Real,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: String,
}

impl Def for MessageKindDef {
    type Id = MessageKindId;
    const TYPE: &'static str = "message_kind";
    const DOC: &'static str = "A kind of message said aloud: what it carries for the engine (a report, an order, or nothing), and how it is said (15 §11).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !self.belief.is_finite() || self.belief < Real::ZERO || self.belief > Real::ONE {
            cx.error("`belief` is a share from 0 to 1");
        }
        if !self.strength.is_finite() || self.strength < Real::ZERO {
            cx.error("`strength` must be a number of zero or more");
        }
    }
}

/// One line of a `lines` table.
#[derive(Debug, Deserialize)]
pub struct LineDef {
    /// What is said: a text for the hearer as reader, with `self` the
    /// speaker, `to` the one spoken to and `about` what it is about.
    pub text: Text,
    /// When it fits: a condition with the same subjects. Default true.
    #[serde(default, rename = "if")]
    pub condition: Cond,
    /// How likely it is beside the others that fit. Default 1.
    #[serde(default = "one")]
    pub weight: Real,
}

/// A table of things to say (15 §11): each line a text with a condition
/// and a weight. A line that fits is picked when something is said.
///
/// ```json
/// { "type": "lines", "id": "greetings",
///   "lines": [ { "text": "Good day, {name(to)}." }, { "text": "Morning.", "weight": 2 } ] }
/// ```
#[derive(Debug, Deserialize)]
pub struct LinesDef {
    /// The lines.
    pub lines: Vec<LineDef>,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: String,
}

/// The scope a line is written in: `self` the speaker, then `to` and
/// `about`, each an entity or null.
pub(crate) fn line_scope() -> Scope {
    Scope::new()
        .own("self", "entity")
        .subject("to", "entity")
        .subject("about", "entity")
}

impl Def for LinesDef {
    type Id = LinesId;
    const TYPE: &'static str = "lines";
    const DOC: &'static str = "A table of things to say, each a text with a condition and a weight; one that fits is picked when something is said (15 §11).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let scope = line_scope();
        for line in &mut self.lines {
            cx.compile(&mut line.text, &scope);
            cx.compile(&mut line.condition, &scope);
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.lines.is_empty() {
            cx.error("a `lines` table has at least one line");
        }
        if self
            .lines
            .iter()
            .any(|line| !line.weight.is_finite() || line.weight < Real::ZERO)
        {
            cx.error("a line's `weight` must be a number of zero or more");
        }
    }
}

/// The definition of a type a name written in data stands for, while
/// definitions load or after: a qualified id exactly; a bare one in the
/// first namespace, in sorted order, that has it.
pub(crate) fn lookup_in<T: Def>(view: &DefsView<'_>, name: &str) -> Option<T::Id> {
    if name.contains(':') {
        return view.lookup::<T>(name);
    }
    let mut found: Vec<(&str, T::Id)> = view
        .iter::<T>()
        .filter_map(|(id, _)| Some((view.name::<T>(id)?, id)))
        .filter(|(qualified, _)| {
            qualified
                .split_once(':')
                .is_some_and(|(_, bare)| bare == name)
        })
        .collect();
    found.sort_unstable_by(|a, b| a.0.cmp(b.0));
    found.first().map(|(_, id)| *id)
}

/// Warns of each group a template's `groups` names that no definition
/// answers to, or a role its group does not have: a thing made from it
/// does not join it. A warning, so that content loaded without its
/// groups still loads.
pub(crate) fn check_template_groups(cx: &mut CrossCx<'_>) {
    use crate::template::TemplateDef;
    let view = cx.defs();
    let mut missing: Vec<(DefLabel, String)> = Vec::new();
    for (id, def) in view.iter::<TemplateDef>() {
        let label = DefLabel::new(
            TemplateDef::TYPE,
            view.name::<TemplateDef>(id).unwrap_or(""),
        );
        for (group, role) in &def.groups {
            match lookup_in::<GroupDef>(view, group).and_then(|group| view.get::<GroupDef>(group)) {
                None => missing.push((label.clone(), format!("there is no group `{group}`"))),
                Some(found) if !role.is_empty() && !found.roles.contains_key(role) => {
                    missing.push((
                        label.clone(),
                        format!("the group `{group}` has no role `{role}`"),
                    ))
                }
                Some(_) => {}
            }
        }
    }
    for (label, message) in missing {
        cx.report(
            Diagnostic::new(code::VALIDATION, message)
                .with_severity(clash_data::Severity::Warning)
                .in_def(Some(&label))
                .with_path("groups"),
        );
    }
}

/// Reports each relation kind and group a group's `relations` names that
/// no definition answers to.
pub(crate) fn check_group_relations(cx: &mut CrossCx<'_>) {
    let view = cx.defs();
    let mut missing: Vec<(DefLabel, String)> = Vec::new();
    for (id, def) in view.iter::<GroupDef>() {
        let label = DefLabel::new(GroupDef::TYPE, view.name::<GroupDef>(id).unwrap_or(""));
        for (toward, values) in &def.relations {
            if lookup_in::<GroupDef>(view, toward).is_none() {
                missing.push((label.clone(), format!("there is no group `{toward}`")));
            }
            for kind in values.keys() {
                if lookup_in::<RelationKindDef>(view, kind).is_none() {
                    missing.push((label.clone(), format!("there is no relation kind `{kind}`")));
                }
            }
        }
    }
    for (label, message) in missing {
        cx.report(
            Diagnostic::new(code::VALIDATION, message)
                .in_def(Some(&label))
                .with_path("relations"),
        );
    }
}
