//! [`RuleState`]: what the rule runtime keeps between runs, and
//! [`RuleSet`]: the rules of a set of definitions, indexed by the event
//! each listens for.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use clash_core::impl_state_hash;
use clash_data::{DefLabel, Defs, Diagnostic, Severity};
use clash_expr::{DefTypeId, EventId, Handle, KindId, Value};
use serde::{Deserialize, Serialize};

use clash_world::Cause;

use super::def::{EOC_TYPE, EocDef, ReasonDef, RuleGroupDef};
use super::host::RuleFault;
use super::tree::{Ask, AskOptions, Block, EffectKind};
use super::{EocId, ReasonDefId, RuleGroupId};
use crate::decision::ChoiceId;
use crate::reason::Reason;

/// A subject as state keeps it: the number of its kind and the
/// simulation's own reference. The same as a [`Handle`], in a form that
/// is saved.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct SubjectKey {
    /// The kind of subject.
    pub kind: u16,
    /// The simulation's reference.
    pub payload: u64,
}

impl_state_hash!(SubjectKey { kind, payload });

impl From<Handle> for SubjectKey {
    fn from(handle: Handle) -> SubjectKey {
        SubjectKey {
            kind: handle.kind.raw(),
            payload: handle.payload,
        }
    }
}

impl From<SubjectKey> for Handle {
    fn from(key: SubjectKey) -> Handle {
        Handle::new(KindId::new(key.kind), key.payload)
    }
}

/// When a rule last ran for an owner, and how often it has.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct RunRecord {
    /// How many times it has run with its condition holding.
    pub runs: u64,
    /// The local time of the owner's clock at the last such run, in
    /// microseconds.
    pub last: i64,
}

impl_state_hash!(RunRecord { runs, last });

/// Where an effect is written: the definition it is in and its JSON
/// path there. This, and not a number, is what state keeps of a place
/// in the definitions, so that it still names the place after a load.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct EffectSite {
    /// The JSON name of the definition's type: `eoc`, `action`.
    pub def_type: String,
    /// The definition's qualified id.
    pub def: String,
    /// The JSON path of the effect inside it: `do[2]`.
    pub path: String,
}

impl_state_hash!(EffectSite {
    def_type,
    def,
    path
});

/// One option of a choice that is waiting for its answer.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct StoredOption {
    /// What it says, as it was rendered for the one who chooses.
    pub label: String,
    /// Whether it may be taken.
    pub available: bool,
    /// For an option gathered when the `ask` ran: the qualified id of
    /// the rule to run if it is chosen. `None` for one written in the
    /// `ask`.
    pub rule: Option<String>,
    /// The values to run that rule with, a map by parameter name; null
    /// otherwise.
    pub with: clash_core::Value,
}

impl_state_hash!(StoredOption {
    label,
    available,
    rule,
    with
});

/// A choice a rule's `ask` put to someone who has not answered yet
/// (05 §10.6): what the chosen option's effects need to run with the
/// subjects and locals of where the `ask` is written.
///
/// It is the first continuation the rule runtime keeps (05 §13.4): plain
/// data, saved with the simulation.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PendingChoice {
    /// Who chooses.
    pub who: SubjectKey,
    /// The question, as it was rendered.
    pub prompt: String,
    /// The options, in the order written.
    pub options: Vec<StoredOption>,
    /// Where the `ask` is written.
    pub site: EffectSite,
    /// What the rule that asked is attached to, if it is attached.
    pub owner: Option<SubjectKey>,
    /// The names of the subjects bound where the `ask` is written, by
    /// slot.
    pub names: Vec<String>,
    /// Their values.
    pub subjects: Vec<clash_core::Value>,
    /// The locals of the list the `ask` is written in, by slot.
    pub locals: Vec<clash_core::Value>,
    /// The name of the event that was being handled, and its fields.
    pub event: Option<(String, Vec<clash_core::Value>)>,
    /// What the option's effects are caused by.
    pub cause: Cause,
}

impl_state_hash!(PendingChoice {
    who,
    prompt,
    options,
    site,
    owner,
    names,
    subjects,
    locals,
    event,
    cause
});

/// A rule waiting in the queue: `queue` worked out which rule, its
/// arguments and the subjects it is given when the effect ran.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Queued {
    pub(crate) rule: EocId,
    pub(crate) subjects: Vec<Value>,
    pub(crate) locals: Vec<Value>,
}

/// What exists only while rules are running. It is empty at every safe
/// point, so it is neither saved nor hashed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct Running {
    /// How many runs are in progress inside one another.
    pub(crate) depth: u32,
    /// Rules queued and not yet run, in order.
    pub(crate) queue: VecDeque<Queued>,
    /// How many have been queued since the queue was last empty at the
    /// end of an outermost run.
    pub(crate) queued: usize,
}

/// What the rule runtime keeps between runs (05 §9, invariant E6):
/// rules attached at run time, when each rule that counts last ran for
/// each owner, and the rule groups a rule has switched.
///
/// It is plain state: saved with the simulation, hashed, and restored
/// by a load. Nothing in it is derived from the definitions; the index
/// of rules by event is a [`RuleSet`], which is built from them.
///
/// ```
/// use clash_core::Hash128;
/// use clash_expr::{Handle, KindId};
/// use clash_sim::rule::{EocId, RuleState};
///
/// let guard = Handle::new(KindId::new(0), 7);
/// let curse = EocId::new(3);
/// let mut state = RuleState::default();
/// assert!(state.attach(guard, curse));
/// assert!(!state.attach(guard, curse), "attaching twice does nothing");
/// assert_eq!(state.attached_to(guard), [curse]);
/// assert_ne!(Hash128::of(&state), Hash128::of(&RuleState::default()));
/// assert!(state.detach(guard, curse));
/// assert_eq!(state, RuleState::default());
/// ```
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct RuleState {
    /// Rules attached at run time, by what they are attached to, each
    /// list in order of rule id.
    attached: BTreeMap<SubjectKey, Vec<EocId>>,
    /// When each rule with `once` or a `cooldown` last ran, by rule and
    /// owner; the owner is `None` for a global rule and for a rule that
    /// was called.
    records: BTreeMap<(EocId, Option<SubjectKey>), RunRecord>,
    /// Rule groups switched by `set_rule_group`, whatever their
    /// `enabled` says.
    groups: BTreeMap<RuleGroupId, bool>,
    /// The choices put by `ask` that are waiting for an answer.
    choices: BTreeMap<ChoiceId, PendingChoice>,
    /// How many choices have been put: the number of the next.
    next_choice: u64,
    #[serde(skip)]
    pub(crate) running: Running,
}

impl_state_hash!(RuleState {
    attached,
    records,
    groups,
    choices,
    next_choice
});

impl RuleState {
    /// Attaches a rule to a subject. Returns whether it was not there
    /// already (05 §10.5).
    pub fn attach(&mut self, owner: Handle, rule: EocId) -> bool {
        let rules = self.attached.entry(owner.into()).or_default();
        match rules.binary_search(&rule) {
            Ok(_) => false,
            Err(at) => {
                rules.insert(at, rule);
                true
            }
        }
    }

    /// Takes a rule off a subject and forgets when it last ran there.
    /// Returns whether it was attached.
    pub fn detach(&mut self, owner: Handle, rule: EocId) -> bool {
        let key = SubjectKey::from(owner);
        let Some(rules) = self.attached.get_mut(&key) else {
            return false;
        };
        let Ok(at) = rules.binary_search(&rule) else {
            return false;
        };
        rules.remove(at);
        if rules.is_empty() {
            self.attached.remove(&key);
        }
        self.records.remove(&(rule, Some(key)));
        true
    }

    /// The rules attached to a subject at run time, in order of id.
    pub fn attached_to(&self, owner: Handle) -> &[EocId] {
        self.attached
            .get(&SubjectKey::from(owner))
            .map_or(&[], Vec::as_slice)
    }

    /// Every subject with a rule attached at run time, with its rules,
    /// in order.
    pub fn attachments(&self) -> impl Iterator<Item = (Handle, &[EocId])> {
        self.attached
            .iter()
            .map(|(key, rules)| (Handle::from(*key), rules.as_slice()))
    }

    /// Forgets everything kept for a subject: the rules attached to it
    /// and when rules last ran for it. For a subject that has been
    /// destroyed.
    pub fn forget(&mut self, owner: Handle) {
        let key = SubjectKey::from(owner);
        self.attached.remove(&key);
        self.records.retain(|(_, have), _| *have != Some(key));
        self.choices.retain(|_, choice| choice.who != key);
    }

    /// The number of the next choice: no two choices of one simulation
    /// have the same.
    pub fn next_choice(&mut self) -> ChoiceId {
        let id = ChoiceId::new(self.next_choice);
        self.next_choice = self.next_choice.saturating_add(1);
        id
    }

    /// Keeps a choice until it is answered.
    pub fn keep_choice(&mut self, id: ChoiceId, choice: PendingChoice) {
        self.choices.insert(id, choice);
    }

    /// A choice that is waiting for an answer.
    pub fn choice(&self, id: ChoiceId) -> Option<&PendingChoice> {
        self.choices.get(&id)
    }

    /// Takes a choice out: it has been answered, or will not be.
    pub fn take_choice(&mut self, id: ChoiceId) -> Option<PendingChoice> {
        self.choices.remove(&id)
    }

    /// The choices waiting for one subject's answer, oldest first.
    pub fn choices_of(&self, who: Handle) -> Vec<ChoiceId> {
        let key = SubjectKey::from(who);
        self.choices
            .iter()
            .filter(|(_, choice)| choice.who == key)
            .map(|(id, _)| *id)
            .collect()
    }

    /// When a rule last ran for an owner, if it is one that counts: a
    /// rule with `once` or a `cooldown`.
    pub fn record(&self, rule: EocId, owner: Option<Handle>) -> Option<RunRecord> {
        self.records
            .get(&(rule, owner.map(SubjectKey::from)))
            .copied()
    }

    pub(crate) fn note_run(&mut self, rule: EocId, owner: Option<Handle>, now: i64) {
        let record = self
            .records
            .entry((rule, owner.map(SubjectKey::from)))
            .or_default();
        record.runs = record.runs.saturating_add(1);
        record.last = now;
    }

    /// How `set_rule_group` has switched a group: on, off, or `None`
    /// for one left to its `enabled`.
    pub fn group_switch(&self, group: RuleGroupId) -> Option<bool> {
        self.groups.get(&group).copied()
    }

    /// Switches a group on or off, or with `None` leaves it to its
    /// `enabled` again.
    pub fn set_group(&mut self, group: RuleGroupId, on: Option<bool>) {
        match on {
            Some(on) => {
                self.groups.insert(group, on);
            }
            None => {
                self.groups.remove(&group);
            }
        }
    }

    /// Whether no rule is running: true at every safe point.
    pub fn is_idle(&self) -> bool {
        self.running.depth == 0 && self.running.queue.is_empty()
    }
}

/// An event on its way through the rules that answer it: what
/// [`RuleSet::deliver`] is given and leaves its answer in.
///
/// The subjects and the fields are in the order the event's type
/// declares them in the expression registry, which is the order a rule
/// was compiled with. A subject the event does not have is null.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Occurrence {
    /// The event type.
    pub event: EventId,
    /// One value for each subject the type declares, in its order.
    pub subjects: Vec<Value>,
    /// One value for each field the type declares, in its order. For a
    /// hook, rules change these.
    pub fields: Vec<Value>,
    /// Whether this is a hook: something about to happen, whose mutable
    /// fields rules may change and which they may cancel.
    pub hook: bool,
    /// Whether a rule consumed the event.
    pub consumed: bool,
    /// For a hook a rule cancelled, the reason.
    pub cancelled: Option<Reason>,
    changed: Vec<u16>,
}

impl Occurrence {
    /// A notification of this type with these subjects and fields.
    pub fn new(event: EventId, subjects: Vec<Value>, fields: Vec<Value>) -> Occurrence {
        Occurrence {
            event,
            subjects,
            fields,
            hook: false,
            consumed: false,
            cancelled: None,
            changed: Vec::new(),
        }
    }

    /// The same, as a hook.
    #[must_use]
    pub fn as_hook(mut self) -> Occurrence {
        self.hook = true;
        self
    }

    /// The slots of the fields rules changed, in the order they were
    /// first changed.
    pub fn changed(&self) -> &[u16] {
        &self.changed
    }

    pub(crate) fn set_field(&mut self, slot: u16, value: Value) {
        if let Some(field) = self.fields.get_mut(usize::from(slot)) {
            *field = value;
            if !self.changed.contains(&slot) {
                self.changed.push(slot);
            }
        }
    }
}

/// The rules that listen for one event: the global ones, indexed by
/// the first entry of their filters (05 §8.4), each list in delivery
/// order, and whether any attached rule listens too.
#[derive(Clone, Debug, Default)]
struct Listeners {
    /// The global rules with no filter.
    open: Vec<EocId>,
    /// The global rules with a filter, by the slot of its first field
    /// and the key of the value that field must have.
    keyed: BTreeMap<(u16, String), Vec<EocId>>,
    /// Whether some attached rule listens for the event, so that its
    /// subjects are worth looking at.
    attached: bool,
}

/// A text that is the same for values a filter counts as equal and
/// different otherwise: a whole real and an int are one number.
pub(crate) fn filter_key(value: &Value) -> String {
    match value {
        Value::Int(int) => format!("n{:016x}", clash_core::Real::from_i64(*int).get().to_bits()),
        Value::Real(real) => format!("n{:016x}", real.get().to_bits()),
        Value::Bool(value) => format!("b{value}"),
        Value::Str(text) => format!("s{text}"),
        Value::Id(id) => format!("d{}:{}", id.ty, id.index),
        Value::Subject(handle) => format!("@{}:{}", handle.kind, handle.payload),
        Value::Null => "0".to_owned(),
        other => format!("?{other}"),
    }
}

/// The rules of a set of definitions, ready to run: the compiled `eoc`
/// definitions, and the global ones indexed by the event each listens
/// for.
///
/// It is derived from the definitions and holds no state: build one
/// when the simulation is set up and again after a load, and share it.
/// The functions that run rules are its methods:
/// [`deliver`](RuleSet::deliver) for an event,
/// [`run_effects`](RuleSet::run_effects) for a list of effects of
/// another definition, and [`call`](RuleSet::call) for one rule.
#[derive(Clone, Debug)]
pub struct RuleSet {
    defs: Arc<Defs>,
    eoc_type: Option<DefTypeId>,
    by_event: Vec<Listeners>,
    roots: Vec<EffectRoots>,
}

/// Finds the compiled lists of effects of one definition that is not a
/// rule: given the definitions, the JSON name of a definition type and
/// a qualified id, it adds every list of that definition to `out`, and
/// nothing if the type is not its own. A package whose definitions hold
/// [`Effects`](super::Effects) writes one and lists it where the
/// simulation builds its [`RuleSet`], so that an effect written in one
/// of them can be found again from an [`EffectSite`].
pub type EffectRoots = for<'a> fn(&'a Defs, &str, &str, &mut Vec<&'a Block>);

/// Looks for the `ask` written at a path in a block and the blocks
/// inside it.
fn ask_at<'a>(block: &'a Block, path: &str) -> Option<&'a Ask> {
    for effect in block.effects() {
        // An effect inside this one has a path that starts with its.
        if !path.starts_with(&*effect.path) {
            continue;
        }
        let inner: Vec<&Block> = match &effect.kind {
            EffectKind::Ask(ask) => {
                if &*effect.path == path {
                    return Some(ask);
                }
                match &ask.options {
                    AskOptions::Listed(options) => {
                        options.iter().map(|option| &option.body).collect()
                    }
                    AskOptions::Computed(_) => Vec::new(),
                }
            }
            EffectKind::If {
                then, otherwise, ..
            } => vec![then, otherwise],
            EffectKind::First { entries } => entries.iter().map(|(_, body)| body).collect(),
            EffectKind::Switch { cases, default, .. } => cases
                .iter()
                .map(|(_, body)| body)
                .chain([default])
                .collect(),
            EffectKind::ForEach { body, .. }
            | EffectKind::Repeat { body, .. }
            | EffectKind::While { body, .. }
            | EffectKind::Rebind { body, .. } => vec![body],
            EffectKind::Weighted { entries } => entries.iter().map(|(_, body)| body).collect(),
            EffectKind::Timer(timer) => vec![&timer.body],
            _ => Vec::new(),
        };
        if let Some(found) = inner.into_iter().find_map(|block| ask_at(block, path)) {
            return Some(found);
        }
    }
    None
}

impl RuleSet {
    /// The rules of these definitions.
    pub fn new(defs: Arc<Defs>) -> RuleSet {
        let eoc_type = defs.expr_registry().def_type_id(EOC_TYPE);
        let mut by_event: Vec<Listeners> = Vec::new();
        if let Some(rules) = defs.try_registry::<EocDef>() {
            // In order of id, so each list is filled in order of id and
            // sorting by priority keeps that order within a priority.
            for (id, rule) in rules.iter() {
                let Some(event) = rule.event() else {
                    continue;
                };
                if by_event.len() <= event.index() {
                    by_event.resize_with(event.index() + 1, Listeners::default);
                }
                let listeners = &mut by_event[event.index()];
                if rule.is_attached() {
                    listeners.attached = true;
                    continue;
                }
                let list = match rule.filter().first() {
                    None => &mut listeners.open,
                    Some((slot, value)) => listeners
                        .keyed
                        .entry((*slot, filter_key(value)))
                        .or_default(),
                };
                list.push(id);
            }
            for listeners in &mut by_event {
                let rank = |id: &EocId| core::cmp::Reverse(rules.get(*id).priority);
                listeners.open.sort_by_key(rank);
                for list in listeners.keyed.values_mut() {
                    list.sort_by_key(rank);
                }
            }
        }
        RuleSet {
            defs,
            eoc_type,
            by_event,
            roots: Vec::new(),
        }
    }

    /// These rules, knowing also where the lists of effects of other
    /// definition types are: see [`EffectRoots`].
    #[must_use]
    pub fn with_roots(mut self, roots: Vec<EffectRoots>) -> RuleSet {
        self.roots = roots;
        self
    }

    /// The `ask` written at a place in the definitions, if it is still
    /// there.
    pub(crate) fn ask_at(&self, site: &EffectSite) -> Option<&Ask> {
        let mut blocks: Vec<&Block> = Vec::new();
        if site.def_type == EOC_TYPE {
            let rule = self.lookup(&site.def).and_then(|id| self.rule(id))?;
            blocks.extend(rule.effects.block());
            blocks.extend(rule.otherwise.block());
        } else {
            for roots in &self.roots {
                roots(&self.defs, &site.def_type, &site.def, &mut blocks);
            }
        }
        blocks
            .into_iter()
            .find_map(|block| ask_at(block, &site.path))
    }

    /// Where the effect with an origin number is written.
    pub fn site_of(&self, origin: u64) -> Option<EffectSite> {
        let origin = self.defs.exprs().origin(origin)?;
        Some(EffectSite {
            def_type: origin.definition.type_name.clone(),
            def: origin.definition.id.clone(),
            path: origin.path.clone(),
        })
    }

    /// Whether any rule listens for an event type: a global one, or an
    /// attached one that something might carry. An event nothing
    /// listens for need not be handed to [`deliver`](RuleSet::deliver)
    /// at all.
    pub fn listens(&self, event: EventId) -> bool {
        self.by_event.get(event.index()).is_some_and(|listeners| {
            listeners.attached || !listeners.open.is_empty() || !listeners.keyed.is_empty()
        })
    }

    /// Whether some attached rule listens for an event type.
    pub(crate) fn attached_listen(&self, event: EventId) -> bool {
        self.by_event
            .get(event.index())
            .is_some_and(|listeners| listeners.attached)
    }

    /// The definitions the rules belong to.
    pub fn defs(&self) -> &Arc<Defs> {
        &self.defs
    }

    /// A rule, by id. `None` for an id that is not one of these
    /// definitions'.
    pub fn rule(&self, id: EocId) -> Option<&EocDef> {
        self.defs.try_registry::<EocDef>()?.try_get(id)
    }

    /// The id of the rule with this qualified id.
    pub fn lookup(&self, qualified: &str) -> Option<EocId> {
        self.defs.try_registry::<EocDef>()?.lookup(qualified)
    }

    /// The qualified id of a rule.
    pub fn name(&self, id: EocId) -> Option<&str> {
        let rules = self.defs.try_registry::<EocDef>()?;
        rules.try_get(id).map(|_| rules.name(id))
    }

    /// The rule an id value names, if it is the id of a rule.
    pub fn rule_of(&self, value: &Value) -> Option<EocId> {
        let id = value.as_id()?;
        (Some(id.ty) == self.eoc_type)
            .then(|| EocId::from_index(id.index as usize))
            .flatten()
    }

    /// A rule's id as a value of the expression language, to give as
    /// the `eoc` of a call or of `attach_rule`.
    pub fn rule_value(&self, id: EocId) -> Value {
        match (self.eoc_type, u32::try_from(id.index())) {
            (Some(ty), Ok(index)) => Value::Id(clash_expr::DefRef::new(ty, index)),
            _ => Value::Null,
        }
    }

    pub(crate) fn group(&self, id: RuleGroupId) -> Option<&RuleGroupDef> {
        self.defs.try_registry::<RuleGroupDef>()?.try_get(id)
    }

    pub(crate) fn reason(&self, id: ReasonDefId) -> Option<(&str, &ReasonDef)> {
        let reasons = self.defs.try_registry::<ReasonDef>()?;
        reasons.try_get(id).map(|def| (reasons.name(id), def))
    }

    /// The global rules an event may concern, in delivery order: those
    /// with no filter and those indexed under a value the event has.
    /// The rest of each one's filter is still to be tested.
    pub(crate) fn global_candidates(&self, event: EventId, fields: &[Value]) -> Vec<EocId> {
        let Some(listeners) = self.by_event.get(event.index()) else {
            return Vec::new();
        };
        let mut found = listeners.open.clone();
        let mut merged = false;
        let mut last_slot: Option<u16> = None;
        for (slot, _) in listeners.keyed.keys() {
            if last_slot == Some(*slot) {
                continue;
            }
            last_slot = Some(*slot);
            let Some(value) = fields.get(usize::from(*slot)) else {
                continue;
            };
            if let Some(list) = listeners.keyed.get(&(*slot, filter_key(value))) {
                found.extend(list.iter().copied());
                merged = true;
            }
        }
        if merged {
            // Stable: within a priority, rules stay in order of id only
            // if sorted by both.
            found.sort_by_key(|id| {
                let priority = self.rule(*id).map_or(0, |rule| rule.priority);
                (core::cmp::Reverse(priority), *id)
            });
        }
        found
    }

    /// A fault as a reason (14 §5): its code and sentence, with the
    /// rule that was running, the definition and JSON path of the
    /// effect, and the code of the reason the simulation gave if it
    /// refused something.
    pub fn fault_reason(&self, fault: &RuleFault) -> Reason {
        let mut reason = Reason::new(fault.code, fault.message.clone());
        if let Some(name) = fault.rule.and_then(|rule| self.name(rule)) {
            reason = reason.with("rule", name);
        }
        if let Some(origin) = self.defs.exprs().origin(fault.origin) {
            reason = reason
                .with("definition", origin.definition.to_string())
                .with("path", origin.path.as_str());
        }
        if let Some(refusal) = &fault.refusal {
            reason = reason.with("reason", refusal.code.as_str());
        }
        reason
    }

    /// A fault as a diagnostic of the form the loader's have: a
    /// warning with the definition, the JSON path and the place in the
    /// file of the effect that faulted.
    pub fn fault_diagnostic(&self, fault: &RuleFault) -> Diagnostic {
        let mut message = fault.message.clone();
        if let Some(refusal) = &fault.refusal {
            message = format!("{message}: {} ({})", refusal.message, refusal.code);
        }
        let mut diagnostic = Diagnostic::new(clash_data::expr::codes::RUN_UNAVAILABLE, message)
            .with_severity(Severity::Warning);
        fault.code.clone_into(&mut diagnostic.code);
        match self.defs.exprs().origin(fault.origin) {
            Some(origin) => {
                diagnostic = diagnostic
                    .in_def(Some(&origin.definition))
                    .with_path(origin.path.clone());
                if let Some(span) = origin.span {
                    diagnostic = diagnostic.at(self.defs.sources(), span);
                }
            }
            None => {
                if let Some(name) = fault.rule.and_then(|rule| self.name(rule)) {
                    diagnostic = diagnostic.in_def(Some(&DefLabel::new(EOC_TYPE, name)));
                }
            }
        }
        diagnostic
    }
}
