//! The definition types of the rule system: `eoc` (05 §9),
//! `rule_group` (05 §9.6) and `reason` (14 §15).

use std::sync::Arc;

use clash_core::{Dur, define_id};
use clash_data::diag::{closest, did_you_mean};
use clash_data::expr::{Any, Params, TypeSpec, Written, codes};
use clash_data::{
    CompileCx, Cond, Def, DefLabel, DefTypes, Diagnostic, Expr, Inline, Located, Ref, RegistryCx,
    Scope, Site, Span, Text, Time, ValidateCx,
};
use clash_expr::{EventId, EventSpec, KindId, Type, Value, VerbSpec, is_keyword};
use clash_world::defs::OrderedMap;
use serde::Deserialize;

use super::compile::resolve_event;
use super::effects::Effects;
use crate::bus::{EventDef, FieldType};

define_id! {
    /// A rule definition (05 §9): the dense id of an `eoc`.
    pub struct EocId;
    /// A `rule_group` definition (05 §9.6).
    pub struct RuleGroupId;
    /// A `reason` definition (14 §15).
    pub struct ReasonDefId;
}

/// The JSON name of the rule definition type.
pub const EOC_TYPE: &str = "eoc";
/// The JSON name of the rule group definition type.
pub const RULE_GROUP_TYPE: &str = "rule_group";
/// The JSON name of the reason definition type.
pub const REASON_TYPE: &str = "reason";

/// The name an attached rule's owner is bound under (05 §9.3).
pub const OWNER_SUBJECT: &str = "self";
/// The subject that stands for the whole simulation: bound in every
/// rule, and what a rule group's `enabled` may read.
pub const GLOBAL_SUBJECT: &str = "global";

/// The rules a definition carries for its instances (05 §9.3): a list
/// of rule ids or of rules written in place. A definition type whose
/// instances are subjects holds one in a field named `rules`, and the
/// simulation answers [`RuleHost::rules_of`](super::RuleHost::rules_of)
/// from it.
pub type Rules = Vec<Inline<EocDef>>;

/// What a recurring rule does for an owner that left the simulation
/// (05 §9.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhileUnloaded {
    /// Nothing: intervals missed are lost.
    #[default]
    Skip,
    /// Run once on return if at least one interval passed.
    Once,
    /// Run for each interval missed, up to a cap.
    All,
}

/// Where a subject bound in a rule gets its value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SubjectSource {
    /// A subject of the event the rule listens for, by its position in
    /// the event's declaration.
    Event(u16),
    /// What the rule is attached to.
    Owner,
    /// Whoever calls the rule, from the subject it binds under the same
    /// name.
    Caller,
    /// The simulation: the `global` subject, which every rule binds.
    Global,
}

/// A subject bound in a rule.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuleSubject {
    /// The name the rule's expressions write.
    pub name: Arc<str>,
    /// Where its value comes from when the rule is triggered. A rule
    /// that is called takes each of its subjects but `global` from its
    /// caller, by name.
    pub source: SubjectSource,
    /// Its kind.
    pub kind: Option<KindId>,
}

/// A parameter of a rule (05 §9.5).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuleParam {
    /// The name, read in the rule as `$name`.
    pub name: Arc<str>,
    /// The type an argument must have.
    pub ty: Type,
    /// The value it has when a call leaves it out. A parameter with
    /// none must be given.
    pub default: Option<Value>,
    /// What it means.
    pub doc: String,
}

/// What compiling a rule works out.
#[derive(Debug)]
struct Plan {
    event: Option<EventId>,
    attached: bool,
    role: u16,
    subjects: Vec<RuleSubject>,
    names: Arc<[Arc<str>]>,
    params: Vec<RuleParam>,
    returns: Type,
    filter: Vec<(u16, Value)>,
    frame: usize,
}

impl Default for Plan {
    fn default() -> Plan {
        Plan {
            event: None,
            attached: false,
            role: 0,
            subjects: Vec::new(),
            names: Arc::from(Vec::new()),
            params: Vec::new(),
            returns: Type::Any,
            filter: Vec::new(),
            frame: 0,
        }
    }
}

/// A rule (05 §9): when something happens, if a condition holds, do a
/// list of effects.
///
/// ```json
/// { "type": "eoc", "id": "vault_alarm",
///   "on": "element_destroyed",
///   "filter": { "layer": "block" },
///   "if": "not here.site.var.alarm_raised",
///   "do": [ { "set": { "here.site.var.alarm_raised": true } } ] }
/// ```
///
/// | Field | Meaning |
/// |-------|---------|
/// | `on` | The event that triggers the rule. Left out, the rule runs only when it is called. |
/// | `attached` | Whether the rule runs only for what it is attached to (05 §9.3), and binds it as `self`. Default: false for a rule at the top level of a file, true for one written in place inside another definition. |
/// | `as` | For an attached rule: which of the event's subjects its owner must be. Default: the first. |
/// | `filter` | Fields of the event and the constants they must equal (05 §8.4). |
/// | `every`, `while_unloaded` | For a recurring rule (05 §9.4). Read and kept; scheduling is a later package's. |
/// | `if` | The condition. Default true. |
/// | `do`, `else` | The effects when it holds, and when it does not. |
/// | `params` | The parameters of a rule that is called (05 §9.5): a name and a type, or `{ "type", "default", "doc" }`. Those with a default come last, and a default is a constant. |
/// | `subjects` | The subjects a rule that is called is given by its caller, by name, each with its kind. |
/// | `returns` | The type of what `return` gives. Default `any`. |
/// | `priority` | Order among the rules that answer one event: higher first. Default 0. |
/// | `once` | Run at most once for each owner. |
/// | `cooldown` | The least time, on the owner's clock, between two runs for one owner. |
/// | `skip_if_consumed` | Not run for an event an earlier rule consumed. |
/// | `ui_callable` | Whether a screen may run it (12 §10.5). |
/// | `group` | The rule group it belongs to (05 §9.6). |
/// | `doc` | For the reference and the trace. |
///
/// # What a rule's expressions may name
///
/// The subjects of its event, in the event's order; then `self`, for an
/// attached rule; then those it declares in `subjects`; then `global`,
/// the simulation itself. Its locals are
/// its parameters, in the order written, and after them whatever its
/// effects define. `event.field` reads a field of its event. Everything
/// reads truth and may draw at random.
#[derive(Debug, Deserialize)]
pub struct EocDef {
    /// The event that triggers the rule (05 §9.1). Left out for a rule that is
    /// only called, or that recurs with `every`.
    #[serde(default)]
    on: Option<Located<String>>,
    /// For an attached rule: which of the event's subjects the owner must be.
    /// Defaults to the event's first subject.
    #[serde(default, rename = "as")]
    role: Option<Located<String>>,
    /// Whether the rule runs only for what it is attached to, which it binds as
    /// `self` (05 §9.3). Default false for a rule written at the top level of a
    /// file and true for one written in place inside another definition.
    #[serde(default)]
    attached: Option<bool>,
    /// Fields of the event, each with the constant it must equal, tested before
    /// the condition and used to index handlers (05 §8.4).
    #[serde(default)]
    filter: OrderedMap<Expr<Any>>,
    /// Run repeatedly at this interval instead of on an event (05 §9.4). Not
    /// beside `on`.
    #[serde(default)]
    every: Option<Located<Time>>,
    /// What a recurring rule does for an owner that left the
    /// simulation.
    #[serde(default)]
    pub while_unloaded: WhileUnloaded,
    /// The condition. A run executes `do` when it holds and `else` when
    /// it does not.
    #[serde(default, rename = "if")]
    pub condition: Cond,
    /// The effects when the condition holds.
    #[serde(default, rename = "do")]
    pub effects: Effects,
    /// The effects when it does not.
    #[serde(default, rename = "else")]
    pub otherwise: Effects,
    /// Typed parameters, for a rule that is called (05 §9.5): each a type, or
    /// an object with `type`, `default` and `doc`. A rule with at least one is
    /// a verb under its qualified id.
    #[serde(default)]
    params: Params,
    /// The subjects a called rule is given by its caller, by name with their
    /// kinds: `{ "self": "entity" }` (05 §9.5).
    #[serde(default)]
    subjects: OrderedMap<Located<String>>,
    /// The type of what `return` gives. Default `any`.
    #[serde(default)]
    returns: Option<Located<String>>,
    /// Order among the rules that answer one event: higher first, then
    /// by rule id.
    #[serde(default)]
    pub priority: i32,
    /// Whether the rule runs at most once for each owner.
    #[serde(default)]
    pub once: bool,
    /// The least local time between runs for each owner. Only runs in which the
    /// condition held count.
    #[serde(default)]
    cooldown: Option<Located<Time>>,
    /// Whether the rule is skipped for an event an earlier rule
    /// consumed.
    #[serde(default)]
    pub skip_if_consumed: bool,
    /// Whether a screen may run the rule through the `Ui` command
    /// (12 §10.5).
    #[serde(default)]
    pub ui_callable: bool,
    /// The rule group the rule belongs to (05 §9.6): while the group is off,
    /// the rule does not run.
    #[serde(default)]
    group: Option<Ref<RuleGroupDef>>,
    /// What the rule is for.
    #[serde(default)]
    pub doc: String,
    #[serde(skip)]
    plan: Plan,
}

fn is_local_name(name: &str) -> bool {
    clash_data::is_id_part(name) && !is_keyword(name)
}

const NAME_RULE: &str = "a name starts with a lowercase letter, holds lowercase letters, digits and `_`, and is not a word of the expression language";

/// Reports that a rule, a group or a reason is not well formed.
fn bad(
    cx: &mut CompileCx<'_>,
    span: Option<Span>,
    path: &str,
    message: impl Into<String>,
    suggestion: Option<String>,
) {
    let mut diagnostic = Diagnostic::new(codes::BAD_RULE, message)
        .with_path(path)
        .suggest_opt(suggestion);
    if let Some(span) = span {
        diagnostic = diagnostic.at(cx.sources(), span);
    }
    cx.report(diagnostic);
}

impl EocDef {
    /// The event that triggers the rule, if one does.
    pub fn event(&self) -> Option<EventId> {
        self.plan.event
    }

    /// Whether the rule runs only for what it is attached to, bound as
    /// `self` (05 §9.3).
    pub fn is_attached(&self) -> bool {
        self.plan.attached
    }

    /// Whether the rule runs for every event of its type, anywhere
    /// (05 §9.2).
    pub fn is_global(&self) -> bool {
        self.plan.event.is_some() && !self.plan.attached
    }

    /// For an attached rule: the position, among the event's subjects,
    /// of the one its owner must be.
    pub fn role(&self) -> u16 {
        self.plan.role
    }

    /// The subjects the rule binds, in slot order.
    pub fn subjects(&self) -> &[RuleSubject] {
        &self.plan.subjects
    }

    /// The names of the subjects the rule binds, in slot order.
    pub fn subject_names(&self) -> &Arc<[Arc<str>]> {
        &self.plan.names
    }

    /// The subjects the rule declares that whoever calls it gives it.
    pub fn needs_from_caller(&self) -> impl Iterator<Item = &str> {
        self.plan
            .subjects
            .iter()
            .filter(|subject| subject.source == SubjectSource::Caller)
            .map(|subject| &*subject.name)
    }

    /// The parameters, in the order written: the rule's first locals.
    pub fn params(&self) -> &[RuleParam] {
        &self.plan.params
    }

    /// The type of what the rule returns.
    pub fn returns(&self) -> &Type {
        &self.plan.returns
    }

    /// The fields of the event the rule filters on, by slot, and the
    /// value each must have (05 §8.4).
    pub fn filter(&self) -> &[(u16, Value)] {
        &self.plan.filter
    }

    /// For a recurring rule, the interval (05 §9.4).
    pub fn every(&self) -> Option<Dur> {
        self.every.as_ref().map(|every| every.value.dur())
    }

    /// The least time between two runs for one owner, on the owner's
    /// clock.
    pub fn cooldown(&self) -> Option<Dur> {
        self.cooldown.as_ref().map(|cooldown| cooldown.value.dur())
    }

    /// The rule group the rule belongs to, if any.
    pub fn group(&self) -> Option<RuleGroupId> {
        self.group.map(Ref::id)
    }

    /// How many locals a run of the rule needs room for.
    pub fn frame_size(&self) -> usize {
        self.plan.frame
    }

    /// A rule that never runs and does nothing: what a lenient load
    /// puts in place of a broken one.
    fn inert() -> EocDef {
        EocDef {
            on: None,
            role: None,
            attached: None,
            filter: OrderedMap::default(),
            every: None,
            while_unloaded: WhileUnloaded::Skip,
            condition: Cond::constant(false),
            effects: Effects::empty(),
            otherwise: Effects::empty(),
            params: Params::default(),
            subjects: OrderedMap::default(),
            returns: None,
            priority: 0,
            once: false,
            cooldown: None,
            skip_if_consumed: false,
            ui_callable: false,
            group: None,
            doc: String::new(),
            plan: Plan::default(),
        }
    }

    /// Reads the parameters into the plan and the scope.
    fn compile_params(&mut self, cx: &mut CompileCx<'_>, mut scope: Scope) -> Scope {
        let mut defaulted: Option<String> = None;
        for param in &mut self.params.0 {
            let name = param.name.value.clone();
            let path = format!("params.{name}");
            if !is_local_name(&name) {
                bad(
                    cx,
                    Some(param.name.span),
                    &path,
                    format!("\"{name}\" cannot be written `${name}`: {NAME_RULE}"),
                    None,
                );
                continue;
            }
            if self.plan.params.iter().any(|have| *have.name == *name) {
                bad(
                    cx,
                    Some(param.name.span),
                    &path,
                    format!("the parameter `{name}` is given twice"),
                    None,
                );
                continue;
            }
            let ty = cx
                .parse_type(&param.ty.value, Some(param.ty.span), &path)
                .unwrap_or(Type::Any);
            let default = match &mut param.default {
                Some(default) => {
                    let constant = Scope::new().pure().expecting(TypeSpec::Exact(ty.clone()));
                    cx.compile(default, &constant);
                    let value = default.constant().cloned();
                    if value.is_none() && default.is_compiled() {
                        bad(
                            cx,
                            default.span(),
                            &format!("{path}.default"),
                            "a parameter's default is a constant; work a value out in the rule with `??`",
                            None,
                        );
                    }
                    defaulted = Some(name.clone());
                    value
                }
                None => {
                    if let Some(earlier) = &defaulted {
                        bad(
                            cx,
                            Some(param.name.span),
                            &path,
                            format!(
                                "`{name}` has no default and comes after `{earlier}`, which has one: parameters that may be left out come last"
                            ),
                            None,
                        );
                    }
                    None
                }
            };
            scope = scope.local(name.clone(), TypeSpec::Exact(ty.clone()));
            self.plan.params.push(RuleParam {
                name: Arc::from(name),
                ty,
                default,
                doc: param.doc.clone(),
            });
        }
        scope
    }

    /// Reads the filter: each entry a field of the event and a constant.
    fn compile_filter(&mut self, cx: &mut CompileCx<'_>) {
        let registry = cx.registry();
        let spec = self
            .plan
            .event
            .and_then(|event| registry.event(event))
            .map(|event| &event.spec);
        for (field, value) in &mut self.filter.0 {
            let path = format!("filter.{field}");
            let Some(spec) = spec else {
                bad(
                    cx,
                    value.span(),
                    &path,
                    "`filter` tests fields of the event a rule listens for, and this rule has no `on`",
                    None,
                );
                continue;
            };
            let Some(slot) = spec.fields.iter().position(|have| have.name == *field) else {
                let known = spec.fields.iter().map(|have| have.name.as_str());
                bad(
                    cx,
                    value.span(),
                    &path,
                    format!("the event `{}` has no field `{field}`", spec.name),
                    did_you_mean(closest(field, known)),
                );
                continue;
            };
            let declared = &spec.fields[slot].ty;
            let wanted = match declared.non_null() {
                Type::Bool | Type::Int | Type::Real | Type::Number | Type::Str | Type::Id(_) => {
                    declared.clone()
                }
                // A text where the type is not known is the text itself.
                _ if matches!(value.written(), Written::Text(_)) => Type::Str,
                _ => Type::Any,
            };
            cx.compile(
                value,
                &Scope::new().pure().expecting(TypeSpec::Exact(wanted)),
            );
            match value.constant() {
                Some(constant) => {
                    let slot = u16::try_from(slot).unwrap_or(u16::MAX);
                    self.plan.filter.push((slot, constant.clone()));
                }
                None if value.is_compiled() => bad(
                    cx,
                    value.span(),
                    &path,
                    "a filter compares a field with a constant; test anything else in `if`",
                    None,
                ),
                None => {}
            }
        }
    }
}

impl Def for EocDef {
    type Id = EocId;
    const TYPE: &'static str = EOC_TYPE;
    const DOC: &'static str = "A rule: when an event happens, or when it is called, if a condition holds, do a list of effects (05 §9).";
    const DEFAULTS: &'static str = r#"{ "priority": 0 }"#;

    #[allow(clippy::too_many_lines)]
    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let registry = cx.registry();
        // A rule written in place inside another definition belongs to
        // that definition's instances.
        let inline = cx.id().contains('#');
        self.plan = Plan {
            attached: self.attached.unwrap_or(inline),
            ..Plan::default()
        };
        let mut scope = Scope::new();

        // The event, and the subjects it binds.
        let mut spec: Option<&EventSpec> = None;
        if let Some(on) = &self.on {
            let site = Site {
                span: Some(on.span),
                path: "on".to_owned(),
            };
            let namespaces = cx.context(&Scope::new(), &site).scope().clone();
            match resolve_event(registry, &on.value, &namespaces) {
                Some(event) => {
                    spec = registry.event(event).map(|event| &event.spec);
                    self.plan.event = Some(event);
                }
                None => {
                    let known: Vec<&str> = registry
                        .events()
                        .map(|event| event.spec.name.as_str())
                        .collect();
                    let bare = known
                        .iter()
                        .filter_map(|name| name.split_once(':').map(|(_, bare)| bare));
                    bad(
                        cx,
                        Some(on.span),
                        "on",
                        format!("there is no event `{}`", on.value),
                        did_you_mean(closest(&on.value, known.iter().copied().chain(bare))),
                    );
                }
            }
        }
        if let Some(spec) = spec {
            scope = cx.event_scope(&spec.name);
            for (index, (name, kind)) in spec.subjects.iter().enumerate() {
                self.plan.subjects.push(RuleSubject {
                    name: Arc::from(name.as_str()),
                    source: SubjectSource::Event(u16::try_from(index).unwrap_or(u16::MAX)),
                    kind: Some(*kind),
                });
            }
        }
        if let (Some(every), Some(on)) = (&self.every, &self.on) {
            bad(
                cx,
                Some(every.span),
                "every",
                format!(
                    "a rule is triggered by an event or by an interval, not both: this one has `on` (`{}`) and `every`",
                    on.value
                ),
                None,
            );
        }
        if let Some(every) = &self.every
            && every.value.dur() <= Dur::ZERO
        {
            bad(
                cx,
                Some(every.span),
                "every",
                "the interval of a recurring rule is above zero",
                None,
            );
        }
        if let Some(cooldown) = &self.cooldown
            && cooldown.value.dur().is_negative()
        {
            bad(
                cx,
                Some(cooldown.span),
                "cooldown",
                "a cooldown cannot be less than no time",
                None,
            );
        }

        // `as`, and the owner of an attached rule.
        let mut owner_kind: Option<KindId> = None;
        if let Some(role) = &self.role {
            let found = spec.and_then(|spec| {
                spec.subjects
                    .iter()
                    .position(|(name, _)| *name == role.value)
            });
            match (found, spec) {
                (Some(at), _) => self.plan.role = u16::try_from(at).unwrap_or(u16::MAX),
                (None, Some(spec)) => {
                    let known = spec.subjects.iter().map(|(name, _)| name.as_str());
                    bad(
                        cx,
                        Some(role.span),
                        "as",
                        format!("the event `{}` has no subject `{}`", spec.name, role.value),
                        did_you_mean(closest(&role.value, known)),
                    );
                }
                (None, None) => bad(
                    cx,
                    Some(role.span),
                    "as",
                    "`as` names a subject of the event a rule listens for, and this rule has no `on`",
                    None,
                ),
            }
            if !self.plan.attached {
                bad(
                    cx,
                    Some(role.span),
                    "as",
                    "`as` says which of an event's subjects an attached rule's owner must be, and this rule is not attached: it runs for every event of its type. Add `\"attached\": true`",
                    None,
                );
            }
        }
        if self.plan.attached {
            if let Some(spec) = spec {
                match spec.subjects.get(usize::from(self.plan.role)) {
                    Some((_, kind)) => owner_kind = Some(*kind),
                    None => bad(
                        cx,
                        self.on.as_ref().map(|on| on.span),
                        "on",
                        format!(
                            "the event `{}` has no subjects, so no rule can be attached for it",
                            spec.name
                        ),
                        None,
                    ),
                }
            } else if self.every.is_none() && self.on.is_none() {
                bad(
                    cx,
                    None,
                    "",
                    "an attached rule listens for an event (`on`) or runs at an interval (`every`); a rule that is only called is not attached to anything",
                    None,
                );
            }
        }

        // The subjects a caller gives. For a recurring attached rule,
        // `self` among them says what kind of thing its owner is.
        let mut declared: Vec<(Arc<str>, Type, Option<KindId>)> = Vec::new();
        for (name, kind) in &self.subjects.0 {
            let path = format!("subjects.{name}");
            if !is_local_name(name) {
                bad(
                    cx,
                    Some(kind.span),
                    &path,
                    format!("\"{name}\" cannot name a subject: {NAME_RULE}"),
                    None,
                );
                continue;
            }
            let Some(ty) = cx.parse_type(&kind.value, Some(kind.span), &path) else {
                continue;
            };
            let Type::Subject(kind_id) = ty.non_null() else {
                bad(
                    cx,
                    Some(kind.span),
                    &path,
                    format!("`{}` is not a kind of subject", kind.value),
                    None,
                );
                continue;
            };
            let kind_id = *kind_id;
            let owner = self.plan.attached && name == OWNER_SUBJECT;
            if owner && spec.is_none() {
                owner_kind = Some(kind_id);
                continue;
            }
            if owner
                || self
                    .plan
                    .subjects
                    .iter()
                    .any(|subject| *subject.name == **name)
                || declared.iter().any(|(have, _, _)| **have == **name)
            {
                bad(
                    cx,
                    Some(kind.span),
                    &path,
                    format!("the subject `{name}` is already bound in this rule"),
                    None,
                );
                continue;
            }
            declared.push((Arc::from(name.as_str()), ty, Some(kind_id)));
        }
        if self.plan.attached {
            match owner_kind {
                Some(kind) => {
                    scope = scope.subject(OWNER_SUBJECT, Type::Subject(kind));
                    self.plan.subjects.push(RuleSubject {
                        name: Arc::from(OWNER_SUBJECT),
                        source: SubjectSource::Owner,
                        kind: Some(kind),
                    });
                }
                None if spec.is_none() && self.every.is_some() => bad(
                    cx,
                    self.every.as_ref().map(|every| every.span),
                    "every",
                    "a recurring attached rule says what kind of thing it is attached to: `\"subjects\": { \"self\": \"entity\" }`",
                    None,
                ),
                None => {}
            }
        }
        for (name, ty, kind) in declared {
            scope = scope.subject(&*name, ty);
            self.plan.subjects.push(RuleSubject {
                name,
                source: SubjectSource::Caller,
                kind,
            });
        }
        // Every rule may name the simulation itself.
        if let Some(kind) = registry.kind_id(GLOBAL_SUBJECT)
            && !self
                .plan
                .subjects
                .iter()
                .any(|subject| &*subject.name == GLOBAL_SUBJECT)
        {
            scope = scope.subject(GLOBAL_SUBJECT, Type::Subject(kind));
            self.plan.subjects.push(RuleSubject {
                name: Arc::from(GLOBAL_SUBJECT),
                source: SubjectSource::Global,
                kind: Some(kind),
            });
        }
        self.plan.names = self
            .plan
            .subjects
            .iter()
            .map(|subject| subject.name.clone())
            .collect();

        // Parameters, the type returned, the filter.
        scope = self.compile_params(cx, scope);
        if let Some(returns) = &self.returns {
            self.plan.returns = cx
                .parse_type(&returns.value, Some(returns.span), "returns")
                .unwrap_or(Type::Any);
        }
        self.compile_filter(cx);

        // The condition and the effects.
        cx.compile(&mut self.condition, &scope);
        let returns = self.plan.returns.clone();
        self.effects.compile_in_rule(cx, &scope, &returns);
        self.otherwise.compile_in_rule(cx, &scope, &returns);
        self.plan.frame = self
            .plan
            .params
            .len()
            .max(self.effects.frame_size())
            .max(self.otherwise.frame_size());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        self.effects.validate(cx);
        self.otherwise.validate(cx);
    }

    fn placeholder() -> Option<EocDef> {
        Some(EocDef::inert())
    }
}

/// A rule group (05 §9.6): rules that are switched on and off together.
///
/// ```json
/// { "type": "rule_group", "id": "harsh_winter", "enabled": "setting('harsh_winter')" }
/// ```
///
/// | Field | Meaning |
/// |-------|---------|
/// | `enabled` | Whether the group is on, unless `set_rule_group` has switched it. A pure condition that may read `global`. Default true. |
/// | `doc` | For the reference. |
#[derive(Debug, Deserialize)]
pub struct RuleGroupDef {
    /// Whether the group is on, unless `set_rule_group` has switched
    /// it.
    #[serde(default)]
    pub enabled: Cond,
    /// What the group is for.
    #[serde(default)]
    pub doc: String,
}

impl Def for RuleGroupDef {
    type Id = RuleGroupId;
    const TYPE: &'static str = RULE_GROUP_TYPE;
    const DOC: &'static str = "A set of rules that is switched on and off together (05 §9.6).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let mut scope = Scope::new().pure();
        if let Some(global) = cx.registry().kind_id(GLOBAL_SUBJECT) {
            scope = scope.subject(GLOBAL_SUBJECT, Type::Subject(global));
        }
        cx.compile(&mut self.enabled, &scope);
    }

    fn placeholder() -> Option<RuleGroupDef> {
        Some(RuleGroupDef {
            enabled: Cond::constant(false),
            doc: String::new(),
        })
    }
}

/// A reason a mod gives for a refusal of its own (14 §15): what
/// `cancel_event` names (05 §10.5).
///
/// ```json
/// { "type": "reason", "id": "no_fuel",
///   "text": "{the(target)} is out of fuel.",
///   "values": { "target": "entity" },
///   "doc": "Raised by engines with an empty tank." }
/// ```
///
/// | Field | Meaning |
/// |-------|---------|
/// | `text` | The sentence whoever asked is told: a text template for a reader, pure, in which each value is read by its name. Required. |
/// | `values` | The values the reason carries, by name, each with its type as the language writes types. Default none. |
/// | `doc` | When it is given, for the reference. |
#[derive(Debug, Deserialize)]
pub struct ReasonDef {
    /// The sentence, as a text template.
    pub text: Text,
    /// The values the reason carries, by name, each with its type as the
    /// language writes types (`entity`, `real`, `id<channel>`). Default none.
    #[serde(default)]
    values: OrderedMap<Located<String>>,
    /// When the reason is given.
    #[serde(default)]
    pub doc: String,
    #[serde(skip)]
    names: Vec<Arc<str>>,
}

impl ReasonDef {
    /// The names of the values the reason carries, in the order
    /// declared: the order the sentence's subjects are bound in.
    pub fn value_names(&self) -> &[Arc<str>] {
        &self.names
    }
}

impl Def for ReasonDef {
    type Id = ReasonDefId;
    const TYPE: &'static str = REASON_TYPE;
    const DOC: &'static str = "A reason a mod gives for a refusal of its own: a sentence and the values it carries (14 §15).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let mut scope = Scope::new().pure();
        self.names.clear();
        for (name, ty) in &self.values.0 {
            let path = format!("values.{name}");
            if !is_local_name(name) {
                bad(
                    cx,
                    Some(ty.span),
                    &path,
                    format!("\"{name}\" cannot name a value: {NAME_RULE}"),
                    None,
                );
                continue;
            }
            let Some(parsed) = cx.parse_type(&ty.value, Some(ty.span), &path) else {
                continue;
            };
            scope = scope.subject(name.clone(), parsed);
            self.names.push(Arc::from(name.as_str()));
        }
        cx.compile(&mut self.text, &scope);
    }

    fn placeholder() -> Option<ReasonDef> {
        Some(ReasonDef {
            text: Text::plain("it was refused"),
            values: OrderedMap::default(),
            doc: String::new(),
            names: Vec::new(),
        })
    }
}

/// Registers the definition types of the rule system with the loader:
/// `reason`, `rule_group` and `eoc`, and the two steps that tell the
/// expression registry what they declare.
///
/// The steps run after every definition is read and before anything is
/// compiled (04 §7.3):
///
/// 1. Each `event` definition is entered as an event type, with its
///    subjects, fields and whether it is a hook, unless an event of
///    that name is there already. One whose subjects name a kind the
///    registry does not have is left out.
/// 2. Each rule with parameters is entered as a verb under its
///    qualified id, which is what lets it be written as an effect and
///    its arguments be checked where it is called (05 §9.5).
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<ReasonDef>()
        .register::<RuleGroupDef>()
        .register::<EocDef>()
        .add_registry_step(register_events)
        .add_registry_step(register_rule_verbs);
}

/// The type of an event's field as the expression language has it.
fn field_type(written: &str) -> Type {
    match FieldType::parse(written) {
        Some(FieldType::Bool) => Type::Bool,
        Some(FieldType::Int) => Type::Int,
        Some(FieldType::Real) => Type::Real,
        Some(FieldType::Str) => Type::Str,
        Some(FieldType::Any) | None => Type::Any,
    }
}

fn register_events(cx: &mut RegistryCx<'_>) {
    let defs = cx.defs();
    let mut specs: Vec<EventSpec> = Vec::new();
    for (id, def) in defs.iter::<EventDef>() {
        let Some(name) = defs.name::<EventDef>(id) else {
            continue;
        };
        let registry = cx.registry();
        if registry.event_id(name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(name).doc(def.doc.clone());
        let mut known = true;
        for (subject, kind) in &def.subjects.0 {
            match registry.kind_id(kind) {
                Some(kind) => spec = spec.subject(subject.clone(), kind),
                None => known = false,
            }
        }
        for (field, ty) in &def.fields.0 {
            spec = spec.field(field.clone(), field_type(ty), "");
        }
        if def.hook {
            spec = spec.hook(def.mutable.iter().cloned());
        }
        if known {
            specs.push(spec);
        }
    }
    for spec in specs {
        let label = DefLabel::new("event", spec.name.clone());
        if let Err(error) = cx.registry().add_event(spec) {
            cx.report(Diagnostic::new(codes::BAD_RULE, error.to_string()).in_def(Some(&label)));
        }
    }
}

fn register_rule_verbs(cx: &mut RegistryCx<'_>) {
    let defs = cx.defs();
    let mut specs: Vec<(VerbSpec, Span)> = Vec::new();
    for (id, rule) in defs.iter::<EocDef>() {
        let Some(name) = defs.name::<EocDef>(id) else {
            continue;
        };
        // A rule written in place has no name to be called by.
        let (Some(first), false) = (rule.params.0.first(), name.contains('#')) else {
            continue;
        };
        let registry = cx.registry();
        let mut spec = VerbSpec::new(name).doc(rule.doc.clone());
        for param in &rule.params.0 {
            // A type that is not one is reported when the rule is
            // compiled.
            let ty = registry.parse_type(&param.ty.value).unwrap_or(Type::Any);
            spec = if param.default.is_some() {
                spec.optional_arg(param.name.value.clone(), ty, param.doc.clone())
            } else {
                spec.arg(param.name.value.clone(), ty, param.doc.clone())
            };
        }
        specs.push((spec, first.name.span));
    }
    for (spec, span) in specs {
        let label = DefLabel::new(EOC_TYPE, spec.name.clone());
        if let Err(error) = cx.registry().add_verb(spec) {
            let diagnostic = Diagnostic::new(
                codes::BAD_RULE,
                format!("this rule cannot be used as a verb: {error}"),
            )
            .at(cx.sources(), span)
            .in_def(Some(&label));
            cx.report(diagnostic);
        }
    }
}
