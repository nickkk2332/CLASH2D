//! The interpreter: runs compiled effects (05 §13).
//!
//! A run is a stack of **frames**, one for the list or rule that was
//! started and one for each rule it has called, and a stack of
//! **activations**, one for each list and loop in progress. Nothing here
//! recurses on what a mod wrote: a rule that calls a rule that calls a
//! rule is three frames on a `Vec`, and a loop inside a branch inside a
//! loop is three activations. Only a hook raised by `emit` re-enters,
//! through the host, and hooks nest at most eight deep.
//!
//! Every run ends. Each effect executed and each pass of a loop is a
//! step, and a run has `rules.max_steps` of them, shared with the rules
//! it calls; calls nest at most `rules.max_call_depth` deep; a value an
//! effect writes is held to `rules.max_value_size` and
//! `rules.max_value_depth`. Whatever goes wrong is a [`RuleFault`]
//! handed to the host, after which the run, or the simulation, carries
//! on.

use std::sync::Arc;

use clash_core::{Real, Rng};
use clash_data::{Cond, Runner, Text};
use clash_expr::{
    Compiled, DefRef, FunctionId, Handle, Host, Limits, Place, PlaceRoot, PropertyId,
    RuntimeWarning, Value,
};

use super::compile::{EMIT_CANCELLED, EMIT_REASON};
use super::def::{EocDef, SubjectSource};
use super::effects::Effects;
use super::host::{Raise, RuleFault, RuleHost, RuleLimits, RunView, VerbCall};
use super::reason;
use super::state::{Occurrence, Queued, RuleSet};
use super::tree::{Arg, Block, Call, Effect, EffectKind, Items, Target};
use super::value::{self, ValueFault, conforms, measure};
use super::{EocId, RuleGroupId};
use crate::reason::{Reason, codes as kernel_codes};

/// What came of running a list of effects or a rule.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RunOutcome {
    /// What `return` gave, or null.
    pub value: Value,
    /// Whether the run was ended by a limit before it finished. What it
    /// had done by then stays done.
    pub aborted: bool,
}

/// One list of effects or rule in progress.
struct Frame<'r> {
    rule: Option<(EocId, &'r EocDef)>,
    owner: Option<Handle>,
    /// The names of the bound subjects, by slot. Longer than the
    /// frame's own while an `as` block is running.
    names: Vec<Arc<str>>,
    subjects: Vec<Value>,
    locals: Vec<Value>,
    /// Whether this is the rule handling the event of the run.
    event: bool,
    /// The height of the activation stack when the frame began.
    base: usize,
    /// The local of the frame below that takes what this one returns.
    ret: Option<u16>,
}

/// One list or loop in progress.
enum Act<'r> {
    Seq {
        block: &'r Block,
        at: usize,
    },
    Repeat {
        body: &'r Block,
        slot: u16,
        next: i64,
        times: i64,
        origin: u64,
    },
    While {
        cond: &'r Cond,
        body: &'r Block,
        origin: u64,
    },
    ForEach {
        body: &'r Block,
        slot: u16,
        items: Arc<[Value]>,
        next: usize,
        origin: u64,
    },
    /// Marks where an `as` block ends: the subjects it bound go.
    Rebind {
        restore: usize,
    },
}

/// What expressions are evaluated against: the subjects, locals and
/// event fields of the frame, and the simulation for everything else.
struct FrameHost<'f, H: ?Sized> {
    host: &'f mut H,
    subjects: &'f [Value],
    locals: &'f [Value],
    event: &'f [Value],
}

impl<H: RuleHost + ?Sized> Host for FrameHost<'_, H> {
    fn subject(&mut self, slot: u16) -> Value {
        self.subjects
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn local(&mut self, slot: u16) -> Value {
        self.locals
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn event_field(&mut self, slot: u16) -> Value {
        self.event
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        self.host.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        self.host.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        self.host.call(function, args)
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        self.host.rng()
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.host.warn(warning);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.host.def_name(id)
    }
}

/// Lays out the locals of a rule that is called: its parameters, each
/// the value given, or else its default.
fn bind_params(rule: &EocDef, name: &str, given: &[(&str, Value)]) -> Result<Vec<Value>, String> {
    let params = rule.params();
    if let Some((unknown, _)) = given
        .iter()
        .find(|(have, _)| !params.iter().any(|param| *param.name == **have))
    {
        return Err(format!("the rule `{name}` has no parameter `{unknown}`"));
    }
    let mut locals = vec![Value::Null; rule.frame_size().max(params.len())];
    for (slot, param) in params.iter().enumerate() {
        locals[slot] = match given.iter().find(|(have, _)| **have == *param.name) {
            Some((_, value)) => {
                if !conforms(value, &param.ty) {
                    return Err(format!(
                        "the rule `{name}` was given a {} for its parameter `{}`",
                        value.type_name(),
                        param.name
                    ));
                }
                value.clone()
            }
            None => match &param.default {
                Some(default) => default.clone(),
                None => {
                    return Err(format!(
                        "the rule `{name}` needs a value for its parameter `{}`",
                        param.name
                    ));
                }
            },
        };
    }
    Ok(locals)
}

/// Whether a field of an event has the value a filter wants: numbers by
/// value, anything else by equality.
fn filter_matches(have: &Value, want: &Value) -> bool {
    match (have.as_number(), want.as_number()) {
        (Some(have), Some(want)) => have == want,
        _ => have == want,
    }
}

/// A value as a reason carries it: the four plain kinds as they are, an
/// id as its qualified id, anything else as it prints.
fn plain(value: &Value, registry: &clash_expr::Registry) -> clash_core::Value {
    match value {
        Value::Bool(value) => clash_core::Value::Bool(*value),
        Value::Int(value) => clash_core::Value::Int(*value),
        Value::Real(value) => clash_core::Value::Real(*value),
        Value::Str(text) => clash_core::Value::from(&**text),
        Value::Id(id) => match registry.def_name(*id) {
            Some(name) => clash_core::Value::from(name),
            None => clash_core::Value::from(value.to_string().as_str()),
        },
        other => clash_core::Value::from(other.to_string().as_str()),
    }
}

struct Exec<'r, H: RuleHost + ?Sized> {
    set: &'r RuleSet,
    host: &'r mut H,
    /// The event being delivered, for the rule that handles it.
    occurrence: Option<&'r mut Occurrence>,
    runner: Runner,
    limits: RuleLimits,
    frames: Vec<Frame<'r>>,
    stack: Vec<Act<'r>>,
    /// Steps taken in the run in progress.
    steps: u64,
    /// What the frame at the bottom returned.
    returned: Value,
    /// The locals of the frame at the bottom, handed back when it ends.
    bottom_locals: Vec<Value>,
    aborted: bool,
}

impl<'r, H: RuleHost + ?Sized> Exec<'r, H> {
    fn new(set: &'r RuleSet, host: &'r mut H, occurrence: Option<&'r mut Occurrence>) -> Self {
        let limits = host.rule_limits();
        let mut runner = Runner::new();
        runner.set_limits(Limits {
            max_value_size: limits.max_value_size,
        });
        runner.set_max_call_depth(limits.max_call_depth);
        Exec {
            set,
            host,
            occurrence,
            runner,
            limits,
            frames: Vec::new(),
            stack: Vec::new(),
            steps: 0,
            returned: Value::Null,
            bottom_locals: Vec::new(),
            aborted: false,
        }
    }

    // ---- evaluating ------------------------------------------------------

    /// What the frame on top offers expressions.
    fn bindings<'f>(
        frames: &'f [Frame<'r>],
        occurrence: Option<&'f Occurrence>,
    ) -> (&'f [Value], &'f [Value], &'f [Value]) {
        match frames.last() {
            Some(frame) => {
                let event = match occurrence {
                    Some(occurrence) if frame.event => occurrence.fields.as_slice(),
                    _ => &[],
                };
                (&frame.subjects, &frame.locals, event)
            }
            None => (&[], &[], &[]),
        }
    }

    fn eval(&mut self, arg: &Arg) -> Value {
        let exprs = self.set.defs().exprs();
        let (subjects, locals, event) = Self::bindings(&self.frames, self.occurrence.as_deref());
        let mut view = FrameHost {
            host: &mut *self.host,
            subjects,
            locals,
            event,
        };
        arg.value(&mut self.runner.over(exprs, &mut view))
    }

    fn eval_code(&mut self, code: &Compiled) -> Value {
        let exprs = self.set.defs().exprs();
        let (subjects, locals, event) = Self::bindings(&self.frames, self.occurrence.as_deref());
        let mut view = FrameHost {
            host: &mut *self.host,
            subjects,
            locals,
            event,
        };
        self.runner.over(exprs, &mut view).value(code)
    }

    fn holds(&mut self, cond: &Cond) -> bool {
        let exprs = self.set.defs().exprs();
        let (subjects, locals, event) = Self::bindings(&self.frames, self.occurrence.as_deref());
        let mut view = FrameHost {
            host: &mut *self.host,
            subjects,
            locals,
            event,
        };
        cond.holds(&mut self.runner.over(exprs, &mut view))
    }

    fn render(&mut self, text: &Text) -> String {
        let exprs = self.set.defs().exprs();
        let (subjects, locals, event) = Self::bindings(&self.frames, self.occurrence.as_deref());
        let mut view = FrameHost {
            host: &mut *self.host,
            subjects,
            locals,
            event,
        };
        text.render(&mut self.runner.over(exprs, &mut view))
    }

    /// Evaluates a condition or a text that binds its own subjects and
    /// nothing of the run's.
    fn apart<T>(&mut self, subjects: Vec<Value>, work: impl FnOnce(&mut Self) -> T) -> T {
        self.frames.push(Frame {
            rule: None,
            owner: None,
            names: Vec::new(),
            subjects,
            locals: Vec::new(),
            event: false,
            base: self.stack.len(),
            ret: None,
        });
        let result = work(self);
        self.frames.pop();
        result
    }

    // ---- reporting -------------------------------------------------------

    fn fault(&mut self, code: &'static str, origin: u64, message: impl Into<String>) {
        self.fault_with(code, origin, message, None);
    }

    fn fault_with(
        &mut self,
        code: &'static str,
        origin: u64,
        message: impl Into<String>,
        refusal: Option<Reason>,
    ) {
        let rule = self
            .frames
            .iter()
            .rev()
            .find_map(|frame| frame.rule.map(|(id, _)| id));
        self.host.fault(RuleFault {
            code,
            message: message.into(),
            origin,
            rule,
            refusal,
        });
    }

    fn value_fault(&mut self, origin: u64, fault: ValueFault) {
        self.fault(fault.0, origin, fault.1);
    }

    /// Counts a step. At the limit the run is ended.
    fn step(&mut self, origin: u64) -> bool {
        self.steps += 1;
        if self.steps <= self.limits.max_steps {
            return true;
        }
        self.fault(
            reason::STEP_LIMIT,
            origin,
            format!(
                "the run executed more than {} effects and loop iterations and was ended",
                self.limits.max_steps
            ),
        );
        self.aborted = true;
        if let Some(bottom) = self.frames.first_mut() {
            self.bottom_locals = core::mem::take(&mut bottom.locals);
        }
        self.frames.clear();
        self.stack.clear();
        false
    }

    // ---- frames ----------------------------------------------------------

    fn set_local(&mut self, slot: u16, value: Value) {
        if let Some(local) = self
            .frames
            .last_mut()
            .and_then(|frame| frame.locals.get_mut(usize::from(slot)))
        {
            *local = value;
        }
    }

    fn push_seq(&mut self, block: &'r Block) {
        if !block.is_empty() {
            self.stack.push(Act::Seq { block, at: 0 });
        }
    }

    /// Ends the frame on top, giving `value` to whoever called it.
    fn finish_frame(&mut self, value: Value) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        self.stack.truncate(frame.base);
        if self.frames.is_empty() {
            self.returned = value;
            self.bottom_locals = frame.locals;
            return;
        }
        if let Some(slot) = frame.ret {
            match measure(&value, &self.limits) {
                Ok(()) => self.set_local(slot, value),
                Err(fault) => self.value_fault(0, fault),
            }
        }
    }

    /// The `global` subject, if a rule binds it.
    fn global_for(&mut self, rule: &EocDef) -> Value {
        if rule
            .subjects()
            .iter()
            .any(|subject| subject.source == SubjectSource::Global)
        {
            self.host.global()
        } else {
            Value::Null
        }
    }

    /// Whether a rule group is on.
    fn group_on(&mut self, id: RuleGroupId) -> bool {
        if let Some(on) = self.host.rule_state().group_switch(id) {
            return on;
        }
        let set = self.set;
        let Some(group) = set.group(id) else {
            return true;
        };
        if let Some(constant) = group.enabled.constant_value() {
            return constant;
        }
        let global = self.host.global();
        self.apart(vec![global], |exec| exec.holds(&group.enabled))
    }

    /// Whether a rule may run now for an owner: its group is on, it has
    /// not already run if it runs `once`, and its cooldown has passed.
    fn gate(&mut self, id: EocId, rule: &EocDef, owner: Option<Handle>) -> bool {
        if let Some(group) = rule.group()
            && !self.group_on(group)
        {
            return false;
        }
        let cooldown = rule.cooldown();
        if !rule.once && cooldown.is_none() {
            return true;
        }
        let Some(record) = self.host.rule_state().record(id, owner) else {
            return true;
        };
        if rule.once && record.runs > 0 {
            return false;
        }
        if let Some(cooldown) = cooldown {
            let now = self.host.local_now(owner).as_micros();
            if now.saturating_sub(record.last) < cooldown.as_micros() {
                return false;
            }
        }
        true
    }

    /// Starts a rule: tests whether it may run, pushes its frame,
    /// evaluates its condition and sets its `do` or its `else` going.
    #[allow(clippy::too_many_arguments)]
    fn enter_rule(
        &mut self,
        id: EocId,
        rule: &'r EocDef,
        owner: Option<Handle>,
        subjects: Vec<Value>,
        mut locals: Vec<Value>,
        event: bool,
        ret: Option<u16>,
        origin: u64,
    ) {
        if self.frames.len() >= self.limits.max_call_depth {
            self.fault(
                reason::CALL_DEPTH,
                origin,
                format!(
                    "rules call each other more than {} deep; the call was refused",
                    self.limits.max_call_depth
                ),
            );
            return;
        }
        if !self.gate(id, rule, owner) {
            return;
        }
        if locals.len() < rule.frame_size() {
            locals.resize(rule.frame_size(), Value::Null);
        }
        self.frames.push(Frame {
            rule: Some((id, rule)),
            owner,
            names: rule.subject_names().to_vec(),
            subjects,
            locals,
            event,
            base: self.stack.len(),
            ret,
        });
        let block = if self.holds(&rule.condition) {
            if rule.once || rule.cooldown().is_some() {
                let now = self.host.local_now(owner).as_micros();
                self.host.rule_state().note_run(id, owner, now);
            }
            rule.effects.block()
        } else {
            rule.otherwise.block()
        };
        if let Some(block) = block {
            self.push_seq(block);
        }
    }

    // ---- the loop --------------------------------------------------------

    /// Runs until every frame has ended.
    fn run(&mut self) {
        while let Some(frame) = self.frames.last() {
            if self.stack.len() <= frame.base {
                self.finish_frame(Value::Null);
                continue;
            }
            let Some(act) = self.stack.last_mut() else {
                self.finish_frame(Value::Null);
                continue;
            };
            match act {
                Act::Seq { block, at } => {
                    let block: &'r Block = block;
                    let index = *at;
                    *at = index + 1;
                    match block.effects().get(index) {
                        Some(effect) => {
                            if self.step(effect.origin) {
                                self.exec(effect);
                            }
                        }
                        None => {
                            self.stack.pop();
                        }
                    }
                }
                Act::Repeat {
                    body,
                    slot,
                    next,
                    times,
                    origin,
                } => {
                    let (body, slot, index, origin): (&'r Block, u16, i64, u64) =
                        (body, *slot, *next, *origin);
                    if index >= *times {
                        self.stack.pop();
                        continue;
                    }
                    *next = index + 1;
                    if self.step(origin) {
                        self.set_local(slot, Value::Int(index));
                        self.push_seq(body);
                    }
                }
                Act::While { cond, body, origin } => {
                    let (cond, body, origin): (&'r Cond, &'r Block, u64) = (cond, body, *origin);
                    if !self.step(origin) {
                        continue;
                    }
                    if self.holds(cond) {
                        self.push_seq(body);
                    } else {
                        self.stack.pop();
                    }
                }
                Act::ForEach {
                    body,
                    slot,
                    items,
                    next,
                    origin,
                } => {
                    let (body, slot, origin): (&'r Block, u16, u64) = (body, *slot, *origin);
                    let Some(item) = items.get(*next).cloned() else {
                        self.stack.pop();
                        continue;
                    };
                    *next += 1;
                    if self.step(origin) {
                        self.set_local(slot, item);
                        self.push_seq(body);
                    }
                }
                Act::Rebind { restore } => {
                    let restore = *restore;
                    self.stack.pop();
                    if let Some(frame) = self.frames.last_mut() {
                        frame.names.truncate(restore);
                        frame.subjects.truncate(restore);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn exec(&mut self, effect: &'r Effect) {
        let origin = effect.origin;
        match &effect.kind {
            EffectKind::If {
                cond,
                then,
                otherwise,
            } => {
                let block = if self.holds(cond) { then } else { otherwise };
                self.push_seq(block);
            }
            EffectKind::First { entries } => {
                for (cond, block) in entries {
                    if cond.as_ref().is_none_or(|cond| self.holds(cond)) {
                        self.push_seq(block);
                        break;
                    }
                }
            }
            EffectKind::Switch { on, cases, default } => {
                let value = self.eval(on);
                let key = self.switch_key(&value);
                let block = key
                    .and_then(|key| cases.iter().find(|(case, _)| **case == *key))
                    .map_or(default, |(_, block)| block);
                self.push_seq(block);
            }
            EffectKind::ForEach { items, slot, body } => match items {
                Items::List(list) => match self.eval(list) {
                    Value::List(items) => self.stack.push(Act::ForEach {
                        body,
                        slot: *slot,
                        items,
                        next: 0,
                        origin,
                    }),
                    // Nothing to go through.
                    Value::Null => {}
                    other => self.fault(
                        reason::TYPE_MISMATCH,
                        origin,
                        format!("`for_each` goes through a list, and this is a {}", other.type_name()),
                    ),
                },
                Items::Query(_) => self.fault(
                    reason::QUERIES_UNAVAILABLE,
                    origin,
                    "`for_each` over a query: queries are not available yet, so the body did not run",
                ),
            },
            EffectKind::Repeat { times, slot, body } => match self.eval(times) {
                Value::Int(times) => self.stack.push(Act::Repeat {
                    body,
                    slot: *slot,
                    next: 0,
                    times,
                    origin,
                }),
                other => self.fault(
                    reason::TYPE_MISMATCH,
                    origin,
                    format!("`repeat` takes an int, and this is a {}", other.type_name()),
                ),
            },
            EffectKind::While { cond, body } => self.stack.push(Act::While { cond, body, origin }),
            EffectKind::Weighted { entries } => self.weighted(entries, origin),
            EffectKind::Rebind { binds, body } => {
                let mut values = Vec::with_capacity(binds.len());
                for (_, arg) in binds {
                    values.push(self.eval(arg));
                }
                let Some(frame) = self.frames.last_mut() else {
                    return;
                };
                self.stack.push(Act::Rebind {
                    restore: frame.subjects.len(),
                });
                for ((name, _), value) in binds.iter().zip(values) {
                    frame.names.push(name.clone());
                    frame.subjects.push(value);
                }
                self.push_seq(body);
            }
            EffectKind::Stop => self.finish_frame(Value::Null),
            EffectKind::Return { value } => {
                let mut value = value.as_ref().map_or(Value::Null, |value| self.eval(value));
                let returns = self
                    .frames
                    .last()
                    .and_then(|frame| frame.rule)
                    .map(|(_, rule)| rule.returns());
                if let Some(returns) = returns
                    && !conforms(&value, returns)
                {
                    self.fault(
                        reason::TYPE_MISMATCH,
                        origin,
                        format!(
                            "the rule returns {}, and this is a {}; null was returned",
                            self.set.defs().expr_registry().type_name(returns),
                            value.type_name()
                        ),
                    );
                    value = Value::Null;
                }
                self.finish_frame(value);
            }
            EffectKind::Let { slot, value } => {
                let value = self.eval(value);
                match measure(&value, &self.limits) {
                    Ok(()) => self.set_local(*slot, value),
                    Err(fault) => self.value_fault(origin, fault),
                }
            }
            EffectKind::Set { place, value } => {
                let value = self.eval(value);
                self.assign(place, Some(value), false, origin);
            }
            EffectKind::Add { place, value } => {
                let value = self.eval(value);
                self.assign(place, Some(value), true, origin);
            }
            EffectKind::Unset { place } => self.assign(place, None, false, origin),
            EffectKind::Run { call, into } => {
                if let Some(slot) = into {
                    self.set_local(*slot, Value::Null);
                }
                if let Some((id, rule, subjects, locals)) = self.prepare(call, origin) {
                    self.enter_rule(id, rule, None, subjects, locals, false, *into, origin);
                }
            }
            EffectKind::Queue { call } => {
                let Some((rule, _, subjects, locals)) = self.prepare(call, origin) else {
                    return;
                };
                let limit = self.limits.max_queued;
                if self.host.rule_state().running.queued >= limit {
                    self.fault(
                        reason::QUEUE_LIMIT,
                        origin,
                        format!("more than {limit} rules were queued; this one was not"),
                    );
                    return;
                }
                let running = &mut self.host.rule_state().running;
                running.queued += 1;
                running.queue.push_back(Queued {
                    rule,
                    subjects,
                    locals,
                });
            }
            EffectKind::AttachRule { to, rule } => self.attach(to, rule, true, origin),
            EffectKind::DetachRule { to, rule } => self.attach(to, rule, false, origin),
            EffectKind::SetRuleGroup { group, on } => match self.eval(on) {
                Value::Bool(on) => self.host.rule_state().set_group(*group, Some(on)),
                Value::Null => self.host.rule_state().set_group(*group, None),
                other => self.fault(
                    reason::TYPE_MISMATCH,
                    origin,
                    format!(
                        "`set_rule_group` takes true, false or null, and this is a {}",
                        other.type_name()
                    ),
                ),
            },
            EffectKind::Emit { emit, into } => {
                let mut subjects = Vec::with_capacity(emit.subjects.len());
                for arg in &emit.subjects {
                    subjects.push(arg.as_ref().map_or(Value::Null, |arg| self.eval(arg)));
                }
                let mut fields = Vec::with_capacity(emit.fields.len());
                for arg in &emit.fields {
                    fields.push(arg.as_ref().map_or(Value::Null, |arg| self.eval(arg)));
                }
                let result = match self.frames.last() {
                    Some(frame) => self.host.raise(&Raise {
                        event: emit.event,
                        subjects: &subjects,
                        fields: &fields,
                        by: RunView {
                            rule: frame.rule.map(|(id, _)| id),
                            owner: frame.owner,
                            names: &frame.names,
                            values: &frame.subjects,
                        },
                    }),
                    None => return,
                };
                let registry = self.set.defs().expr_registry();
                let outcome = match result {
                    Ok(raised) => {
                        let names = registry
                            .event(emit.event)
                            .map(|event| event.spec.fields.as_slice())
                            .unwrap_or_default();
                        let mut entries: Vec<(&str, Value)> = names
                            .iter()
                            .zip(raised.fields)
                            .map(|(field, value)| (field.name.as_str(), value))
                            .collect();
                        entries.push((EMIT_CANCELLED, Value::Bool(raised.cancelled.is_some())));
                        entries.push((
                            EMIT_REASON,
                            raised
                                .cancelled
                                .map_or(Value::Null, |reason| Value::from(reason.code)),
                        ));
                        Value::map(entries)
                    }
                    Err(refusal) => {
                        let name = registry
                            .event(emit.event)
                            .map_or("?", |event| event.spec.name.as_str());
                        self.fault_with(
                            reason::EMIT_REFUSED,
                            origin,
                            format!("the event `{name}` was not raised"),
                            Some(refusal),
                        );
                        Value::Null
                    }
                };
                if let Some(slot) = into {
                    self.set_local(*slot, outcome);
                }
            }
            EffectKind::CancelEvent { reason, values } => {
                let mut given = Vec::with_capacity(values.len());
                for (name, arg) in values {
                    given.push((name.clone(), self.eval(arg)));
                }
                self.cancel(*reason, &given, origin);
            }
            EffectKind::Consume => {
                let handling = self.frames.last().is_some_and(|frame| frame.event);
                if let (true, Some(occurrence)) = (handling, self.occurrence.as_deref_mut()) {
                    occurrence.consumed = true;
                }
            }
            EffectKind::Verb { verb, args } => {
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(arg.as_ref().map_or(Value::Null, |arg| self.eval(arg)));
                }
                let result = match self.frames.last() {
                    Some(frame) => self.host.call_verb(&VerbCall {
                        verb: *verb,
                        args: &values,
                        by: RunView {
                            rule: frame.rule.map(|(id, _)| id),
                            owner: frame.owner,
                            names: &frame.names,
                            values: &frame.subjects,
                        },
                    }),
                    None => return,
                };
                if let Err(refusal) = result {
                    let name = self
                        .set
                        .defs()
                        .expr_registry()
                        .verb(*verb)
                        .map_or("?", |verb| verb.spec.name.as_str());
                    self.fault_with(
                        reason::VERB_REFUSED,
                        origin,
                        format!("`{name}` declined"),
                        Some(refusal),
                    );
                }
            }
            EffectKind::Timer(_) => self.fault(
                reason::TIMERS_UNAVAILABLE,
                origin,
                "a timer effect ran: timers are not available yet, so nothing was scheduled",
            ),
        }
    }

    // ---- particular effects ----------------------------------------------

    /// The text a case of `switch` is compared with.
    fn switch_key(&self, value: &Value) -> Option<String> {
        match value {
            Value::Str(text) => Some(text.to_string()),
            Value::Int(_) | Value::Real(_) | Value::Bool(_) => Some(value.to_string()),
            Value::Id(id) => self
                .set
                .defs()
                .expr_registry()
                .def_name(*id)
                .map(str::to_owned),
            _ => None,
        }
    }

    fn weighted(&mut self, entries: &'r [(Arg, Block)], origin: u64) {
        let mut weights = Vec::with_capacity(entries.len());
        let mut total = Real::ZERO;
        for (weight, _) in entries {
            let value = self.eval(weight);
            let weight = match value.as_number() {
                Some(weight) if weight > Real::ZERO => weight,
                Some(_) => Real::ZERO,
                None => {
                    if !value.is_null() {
                        self.fault(
                            reason::TYPE_MISMATCH,
                            origin,
                            format!("a weight is a number, and this is a {}", value.type_name()),
                        );
                    }
                    Real::ZERO
                }
            };
            total += weight;
            weights.push(weight);
        }
        if total <= Real::ZERO {
            return;
        }
        if !total.get().is_finite() {
            self.fault(
                reason::TYPE_MISMATCH,
                origin,
                "the weights do not add up to a finite number, so no entry was chosen",
            );
            return;
        }
        let Some(rng) = self.host.rng() else {
            self.fault(
                reason::NO_RANDOM,
                origin,
                "`weighted` chooses at random, and there is no stream to draw from",
            );
            return;
        };
        let mut roll = rng.unit() * total;
        let mut chosen: Option<&'r Block> = None;
        for ((_, block), weight) in entries.iter().zip(&weights) {
            if *weight <= Real::ZERO {
                continue;
            }
            chosen = Some(block);
            if roll < *weight {
                break;
            }
            roll -= *weight;
        }
        if let Some(block) = chosen {
            self.push_seq(block);
        }
    }

    /// The subject a place lives on and the keys to step into, worked
    /// out now. `Err` if the place names a subject and there is none.
    fn target(&mut self, place: &Place, origin: u64) -> Result<(Option<Handle>, Vec<Value>), ()> {
        let handle = match place.subject() {
            Some(code) => match self.eval_code(code) {
                Value::Subject(handle) => Some(handle),
                other => {
                    self.fault(
                        reason::NO_SUBJECT,
                        origin,
                        format!(
                            "what is assigned to lives on a subject, and this is {}",
                            if other.is_null() {
                                "null".to_owned()
                            } else {
                                format!("a {}", other.type_name())
                            }
                        ),
                    );
                    return Err(());
                }
            },
            None => None,
        };
        let mut keys = Vec::with_capacity(place.keys().len());
        for key in place.keys() {
            keys.push(self.eval_code(key));
        }
        Ok((handle, keys))
    }

    /// What a place holds now, before its keys.
    fn read_root(&mut self, place: &Place, handle: Option<Handle>) -> Value {
        match (place.root(), handle) {
            (PlaceRoot::Local(slot), _) => self
                .frames
                .last()
                .and_then(|frame| frame.locals.get(usize::from(*slot)))
                .cloned()
                .unwrap_or_default(),
            (PlaceRoot::EventField(slot), _) => {
                let handling = self.frames.last().is_some_and(|frame| frame.event);
                self.occurrence
                    .as_deref()
                    .filter(|_| handling)
                    .and_then(|occurrence| occurrence.fields.get(usize::from(*slot)))
                    .cloned()
                    .unwrap_or_default()
            }
            (PlaceRoot::Variable(name), Some(handle)) => self.host.variable(handle, name),
            (PlaceRoot::Property(property), Some(handle)) => {
                self.host.property(*property, handle, &[])
            }
            _ => Value::Null,
        }
    }

    /// Puts a value in a place, or with `None` removes what is there.
    fn write_root(
        &mut self,
        place: &Place,
        handle: Option<Handle>,
        value: Option<Value>,
        origin: u64,
    ) {
        let refused = match (place.root(), handle) {
            (PlaceRoot::Local(slot), _) => {
                self.set_local(*slot, value.unwrap_or_default());
                return;
            }
            (PlaceRoot::EventField(slot), _) => {
                let handling = self.frames.last().is_some_and(|frame| frame.event);
                match self.occurrence.as_deref_mut() {
                    Some(occurrence) if handling && occurrence.hook => {
                        occurrence.set_field(*slot, value.unwrap_or_default());
                    }
                    _ => self.fault(
                        reason::NOT_A_HOOK,
                        origin,
                        "a field of an event is changed only while its hook is being handled",
                    ),
                }
                return;
            }
            (PlaceRoot::Variable(name), Some(handle)) => match value {
                Some(value) => self.host.set_variable(handle, name, value),
                None => self.host.unset_variable(handle, name),
            },
            (PlaceRoot::Property(property), Some(handle)) => {
                self.host
                    .set_property(*property, handle, value.unwrap_or_default())
            }
            _ => return,
        };
        if let Err(refusal) = refused {
            self.fault_with(
                reason::WRITE_REFUSED,
                origin,
                "the simulation refused the write",
                Some(refusal),
            );
        }
    }

    /// `set`, `add` and `unset`: works out the new value of the place
    /// and writes it, holding it to the limits.
    fn assign(&mut self, place: &Place, value: Option<Value>, add: bool, origin: u64) {
        let Ok((handle, keys)) = self.target(place, origin) else {
            return;
        };
        let registry = self.set.defs().expr_registry();
        let names = |id: DefRef| registry.def_name(id).map(str::to_owned);
        let needs_current = add || !keys.is_empty();
        let current = if needs_current {
            self.read_root(place, handle)
        } else {
            Value::Null
        };
        let entry = match (value, add) {
            (Some(amount), true) => match value::add(&value::get(&current, &keys, &names), &amount)
            {
                Ok(sum) => Some(sum),
                Err(fault) => {
                    self.value_fault(origin, fault);
                    return;
                }
            },
            (value, _) => value,
        };
        if keys.is_empty() {
            if let Some(entry) = &entry
                && let Err(fault) = measure(entry, &self.limits)
            {
                self.value_fault(origin, fault);
                return;
            }
            self.write_root(place, handle, entry, origin);
            return;
        }
        match value::put(&current, &keys, entry, &names) {
            Ok(whole) => match measure(&whole, &self.limits) {
                Ok(()) => self.write_root(place, handle, Some(whole), origin),
                Err(fault) => self.value_fault(origin, fault),
            },
            Err(fault) => self.value_fault(origin, fault),
        }
    }

    /// Works out which rule a call runs, its locals and the subjects it
    /// is given. `None`, with the fault reported, if it cannot be
    /// called.
    fn prepare(
        &mut self,
        call: &'r Call,
        origin: u64,
    ) -> Option<(EocId, &'r EocDef, Vec<Value>, Vec<Value>)> {
        let set = self.set;
        let id = match &call.rule {
            Target::Rule(id) => *id,
            Target::Computed(arg) => {
                let value = self.eval(arg);
                match set.rule_of(&value) {
                    Some(id) => id,
                    None => {
                        self.fault(
                            reason::UNKNOWN_RULE,
                            origin,
                            format!(
                                "what should be the id of a rule is {}",
                                if value.is_null() {
                                    "null".to_owned()
                                } else {
                                    format!("a {}", value.type_name())
                                }
                            ),
                        );
                        return None;
                    }
                }
            }
        };
        let Some(rule) = set.rule(id) else {
            self.fault(
                reason::UNKNOWN_RULE,
                origin,
                "the rule that was called did not load",
            );
            return None;
        };
        let name = set.name(id).unwrap_or("?");
        let mut given: Vec<(&str, Value)> = Vec::with_capacity(call.with.len());
        for (param, arg) in &call.with {
            given.push((param, self.eval(arg)));
        }
        let locals = match bind_params(rule, name, &given) {
            Ok(locals) => locals,
            Err(message) => {
                self.fault(reason::BAD_ARGUMENT, origin, message);
                return None;
            }
        };
        // A called rule takes each of its subjects from the subject its
        // caller binds under the same name.
        let global = self.global_for(rule);
        let mut mismatched: Option<Arc<str>> = None;
        let subjects: Vec<Value> = rule
            .subjects()
            .iter()
            .map(|subject| {
                if subject.source == SubjectSource::Global {
                    return global.clone();
                }
                let found = self.frames.last().and_then(|frame| {
                    let at = frame.names.iter().rposition(|name| *name == subject.name)?;
                    frame.subjects.get(at).cloned()
                });
                match (found, subject.kind) {
                    (Some(Value::Subject(handle)), Some(kind)) if handle.kind != kind => {
                        mismatched = Some(subject.name.clone());
                        Value::Null
                    }
                    (found, _) => found.unwrap_or_default(),
                }
            })
            .collect();
        if let Some(subject) = mismatched {
            self.fault(
                reason::BAD_ARGUMENT,
                origin,
                format!(
                    "the subject `{subject}` bound where `{name}` is called is of another kind than the rule takes; the rule was given null"
                ),
            );
        }
        Some((id, rule, subjects, locals))
    }

    fn attach(&mut self, to: &Arg, rule: &Arg, attach: bool, origin: u64) {
        let Value::Subject(owner) = self.eval(to) else {
            self.fault(
                reason::NO_SUBJECT,
                origin,
                "a rule is attached to a subject, and `to` is not one",
            );
            return;
        };
        let value = self.eval(rule);
        let set = self.set;
        let Some((id, def)) = set.rule_of(&value).and_then(|id| Some((id, set.rule(id)?))) else {
            self.fault(
                reason::UNKNOWN_RULE,
                origin,
                "`eoc` is not the id of a rule that loaded",
            );
            return;
        };
        if !attach {
            self.host.rule_state().detach(owner, id);
            return;
        }
        let name = set.name(id).unwrap_or("?");
        if !def.is_attached() {
            self.fault(
                reason::NOT_ATTACHABLE,
                origin,
                format!(
                    "`{name}` is not an attached rule: give it `\"attached\": true` and an event to listen for"
                ),
            );
            return;
        }
        let takes = def
            .subjects()
            .iter()
            .find(|subject| subject.source == SubjectSource::Owner)
            .and_then(|subject| subject.kind);
        if takes.is_some_and(|kind| kind != owner.kind) {
            self.fault(
                reason::NOT_ATTACHABLE,
                origin,
                format!("`{name}` attaches to another kind of subject than `to` is"),
            );
            return;
        }
        self.host.rule_state().attach(owner, id);
    }

    fn cancel(
        &mut self,
        reason_id: Option<super::ReasonDefId>,
        values: &[(Arc<str>, Value)],
        origin: u64,
    ) {
        let handling = self.frames.last().is_some_and(|frame| frame.event);
        let hook = self
            .occurrence
            .as_deref()
            .filter(|occurrence| handling && occurrence.hook)
            .map(|occurrence| occurrence.event);
        let Some(event) = hook else {
            self.fault(
                reason::NOT_A_HOOK,
                origin,
                "`cancel_event` ran where no hook is being handled",
            );
            return;
        };
        let set = self.set;
        let registry = set.defs().expr_registry();
        let mut reason = match reason_id.and_then(|id| set.reason(id)) {
            Some((code, def)) => {
                // The sentence reads each value by its name, in the
                // order the reason declares them.
                let bound: Vec<Value> = def
                    .value_names()
                    .iter()
                    .map(|name| {
                        values
                            .iter()
                            .find(|(have, _)| have == name)
                            .map(|(_, value)| value.clone())
                            .unwrap_or_default()
                    })
                    .collect();
                let sentence = self.apart(bound, |exec| exec.render(&def.text));
                Reason::new(code, sentence)
            }
            None => {
                let name = registry
                    .event(event)
                    .map_or("?", |event| event.spec.name.as_str());
                Reason::new(
                    kernel_codes::EVENTS_CANCELLED,
                    format!("`{name}` was cancelled by a rule"),
                )
                .with("event", name)
            }
        };
        for (name, value) in values {
            reason = reason.with(&**name, plain(value, registry));
        }
        if let Some(occurrence) = self.occurrence.as_deref_mut() {
            occurrence.cancelled = Some(reason);
        }
    }

    // ---- an event --------------------------------------------------------

    /// Runs the rules that answer the event, in delivery order
    /// (05 §8.3).
    fn deliver(&mut self) {
        let set = self.set;
        let Some(occurrence) = self.occurrence.as_deref() else {
            return;
        };
        let event = occurrence.event;
        let event_subjects = occurrence.subjects.clone();
        // Who is called is settled before anyone is.
        let mut calls: Vec<(EocId, Option<Handle>)> = set
            .global_candidates(event, &occurrence.fields)
            .into_iter()
            .map(|id| (id, None))
            .collect();
        let mut found: Vec<EocId> = Vec::new();
        for (index, subject) in event_subjects.iter().enumerate() {
            let Value::Subject(owner) = subject else {
                continue;
            };
            if !self.host.simulated(*owner) {
                continue;
            }
            found.clear();
            self.host.rules_of(*owner, &mut found);
            found.extend_from_slice(self.host.rule_state().attached_to(*owner));
            found.retain(|id| {
                set.rule(*id).is_some_and(|rule| {
                    rule.is_attached()
                        && rule.event() == Some(event)
                        && usize::from(rule.role()) == index
                })
            });
            found.sort_by_key(|id| {
                let priority = set.rule(*id).map_or(0, |rule| rule.priority);
                (core::cmp::Reverse(priority), *id)
            });
            found.dedup();
            calls.extend(found.iter().map(|id| (*id, Some(*owner))));
        }

        for (id, owner) in calls {
            let Some(rule) = set.rule(id) else {
                continue;
            };
            let Some(occurrence) = self.occurrence.as_deref() else {
                return;
            };
            // A cancellation stops delivery.
            if occurrence.cancelled.is_some() {
                break;
            }
            if rule.skip_if_consumed && occurrence.consumed {
                continue;
            }
            // The whole filter, against the event as it now is.
            let passes = rule.filter().iter().all(|(slot, want)| {
                occurrence
                    .fields
                    .get(usize::from(*slot))
                    .is_some_and(|have| filter_matches(have, want))
            });
            if !passes {
                continue;
            }
            let mut subjects: Vec<Value> = rule
                .subjects()
                .iter()
                .map(|subject| match subject.source {
                    SubjectSource::Event(at) => occurrence
                        .subjects
                        .get(usize::from(at))
                        .cloned()
                        .unwrap_or_default(),
                    SubjectSource::Owner => owner.map_or(Value::Null, Value::Subject),
                    SubjectSource::Caller | SubjectSource::Global => Value::Null,
                })
                .collect();
            let global = self.global_for(rule);
            for (value, subject) in subjects.iter_mut().zip(rule.subjects()) {
                if subject.source == SubjectSource::Global {
                    *value = global.clone();
                }
            }
            let mut locals = vec![Value::Null; rule.frame_size()];
            for (slot, param) in rule.params().iter().enumerate() {
                if let (Some(local), Some(default)) = (locals.get_mut(slot), &param.default) {
                    *local = default.clone();
                }
            }
            self.steps = 0;
            self.enter_rule(id, rule, owner, subjects, locals, true, None, 0);
            self.run();
        }
    }
}

impl RuleSet {
    /// Runs what answers an event: the function the simulation calls
    /// from its bus for every event raised (05 §8.3).
    ///
    /// Global rules that listen for the event's type run first, then
    /// the rules attached to each of its subjects, in the order the
    /// event declares its subjects; within each group, higher
    /// `priority` first and then by rule id. Each rule's `filter` is
    /// tested against the event as it then is, then whether it may run
    /// (its group, `once`, its `cooldown`), then its `if`. A rule marked
    /// `skip_if_consumed` is skipped once the event is consumed, and
    /// nothing runs after a hook is cancelled.
    ///
    /// What the rules did to the event is left in `occurrence`: the
    /// fields they changed, whether it was consumed, and the reason it
    /// was cancelled for.
    pub fn deliver<H: RuleHost + ?Sized>(&self, host: &mut H, occurrence: &mut Occurrence) {
        host.rule_state().running.depth += 1;
        Exec::new(self, host, Some(occurrence)).deliver();
        self.leave(host);
    }

    /// Runs a list of effects that belongs to another definition, such
    /// as an action's `on_start`.
    ///
    /// `subjects` and `locals` are the values of what the list's scope
    /// bound, in the order it bound them. The locals are changed in
    /// place by `set` and `add`, and are as long afterwards as they were
    /// given. A list that was never compiled does nothing.
    pub fn run_effects<H: RuleHost + ?Sized>(
        &self,
        host: &mut H,
        effects: &Effects,
        subjects: &[Value],
        locals: &mut Vec<Value>,
        owner: Option<Handle>,
    ) -> RunOutcome {
        let Some(compiled) = effects.compiled() else {
            return RunOutcome::default();
        };
        host.rule_state().running.depth += 1;
        let given = locals.len();
        let mut frame_locals = core::mem::take(locals);
        if frame_locals.len() < compiled.frame {
            frame_locals.resize(compiled.frame, Value::Null);
        }
        let mut exec = Exec::new(self, host, None);
        exec.frames.push(Frame {
            rule: None,
            owner,
            names: compiled.subjects.to_vec(),
            subjects: subjects.to_vec(),
            locals: frame_locals,
            event: false,
            base: 0,
            ret: None,
        });
        exec.push_seq(&compiled.block);
        exec.run();
        let outcome = RunOutcome {
            value: core::mem::take(&mut exec.returned),
            aborted: exec.aborted,
        };
        *locals = core::mem::take(&mut exec.bottom_locals);
        locals.resize(given, Value::Null);
        self.leave(host);
        outcome
    }

    /// Calls one rule from outside any rule: for the console, a screen,
    /// a recurring rule's interval. `subjects` are the subjects the
    /// rule is given, by name, and `args` its parameters, by name.
    ///
    /// What comes back is what the rule returned, null if it returned
    /// nothing or did not run (its group is off, it runs `once` and has,
    /// its cooldown has not passed). It is refused, with a reason, if
    /// there is no such rule or the arguments do not fit its
    /// parameters.
    pub fn call<H: RuleHost + ?Sized>(
        &self,
        host: &mut H,
        rule: EocId,
        subjects: &[(&str, Value)],
        args: &[(&str, Value)],
    ) -> Result<Value, Reason> {
        let Some(def) = self.rule(rule) else {
            return Err(Reason::new(
                reason::UNKNOWN_RULE,
                format!("there is no rule {rule}"),
            ));
        };
        let name = self.name(rule).unwrap_or("?");
        let locals = bind_params(def, name, args)
            .map_err(|message| Reason::new(reason::BAD_ARGUMENT, message).with("rule", name))?;
        let bound: Vec<Value> = def
            .subjects()
            .iter()
            .map(|subject| {
                if subject.source == SubjectSource::Global {
                    return host.global();
                }
                subjects
                    .iter()
                    .rev()
                    .find(|(have, _)| **have == *subject.name)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default()
            })
            .collect();
        host.rule_state().running.depth += 1;
        let mut exec = Exec::new(self, host, None);
        exec.enter_rule(rule, def, None, bound, locals, false, None, 0);
        exec.run();
        let value = core::mem::take(&mut exec.returned);
        self.leave(host);
        Ok(value)
    }

    /// Ends an outermost run: once nothing is running, the rules that
    /// were queued run, in the order they were queued.
    fn leave<H: RuleHost + ?Sized>(&self, host: &mut H) {
        let running = &mut host.rule_state().running;
        running.depth = running.depth.saturating_sub(1);
        if running.depth > 0 {
            return;
        }
        // A rule queued by a queued rule runs after it, not inside it.
        running.depth = 1;
        while let Some(queued) = host.rule_state().running.queue.pop_front() {
            let Some(rule) = self.rule(queued.rule) else {
                continue;
            };
            let mut exec = Exec::new(self, host, None);
            exec.enter_rule(
                queued.rule,
                rule,
                None,
                queued.subjects,
                queued.locals,
                false,
                None,
                0,
            );
            exec.run();
        }
        let running = &mut host.rule_state().running;
        running.depth = 0;
        running.queued = 0;
    }
}
