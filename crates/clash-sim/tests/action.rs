//! Action definitions from outside (plans/sim-kernel.md, package T2):
//! the examples of 03 §6 and §8 load and compile, and every mistake the
//! definitions can hold is a diagnostic with file, line, column and JSON
//! path.
//!
//! The fixtures are under `tests/fixtures/action/mods`: `spec` holds the
//! examples of document 03, `broken` one definition for each mistake.
//! The names the examples borrow from later documents (`strike`,
//! `sweep`, `message`, `auto_part`, `perceives`, `attr`, `route_fault`)
//! are stubbed in the registry made here, and `takedown_result` is a
//! rule of the `spec` mod that does nothing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::{Dur, Real, Rng};
use clash_data::{
    DefTypes, Defs, Diagnostic, LoadOutcome, Loader, Runner, Severity, Time, explain,
};
use clash_expr::{
    FunctionId, FunctionSpec, Handle, Host, KindId, KindSpec, PropertyId, PropertySpec, Registry,
    RuntimeWarning, Type, Value, VerbSpec,
};
use clash_sim::action::{
    ActionDef, ActionDefId, CancelRule, ChannelDef, ChannelId, ChannelSet, Effects, ListBounds,
    LocalSource, Next, PhaseIndex, PhaseTime, Slots, WindowKindDef, WindowKindId,
};
use clash_sim::register_defs;
use clash_sim::rule::EffectKind;

// ---- the stubs ----------------------------------------------------------

/// The registry a simulation would give the loader, cut down to what the
/// fixtures name.
fn registry() -> Registry {
    let mut registry = Registry::with_builtins();
    let mut kind = |name: &str, doc: &str| {
        registry
            .add_kind(KindSpec::new(name).doc(doc))
            .expect("the kinds have distinct names")
    };
    let entity = kind("entity", "Any entity.");
    let loc = kind("loc", "A cell.");
    let part = kind("part", "A part of a body.");
    let window = kind("window", "An open window.");
    let attributes = kind("attributes", "A character's attributes.");
    let _ = window;
    let subject = Type::Subject;
    let properties = [
        PropertySpec::new(entity, "attr", subject(attributes))
            .yields_own(true)
            .doc("Its attributes."),
        PropertySpec::new(entity, "loc", subject(loc)).doc("Where it truly is."),
        PropertySpec::new(attributes, "quickness", Type::Real).doc("How quick."),
        PropertySpec::new(attributes, "reaction_time", Type::Real).doc("Seconds to notice."),
    ];
    for spec in properties {
        registry
            .add_property(spec)
            .expect("the properties have distinct names");
    }
    let functions = [
        FunctionSpec::new("dist", Type::Real)
            .param("a", subject(entity), "One end.")
            .param("b", subject(entity), "The other.")
            .doc("The distance between two things the character knows of."),
        FunctionSpec::new("auto_part", subject(part))
            .param("target", subject(entity), "Who is aimed at.")
            .doc("The part a character would aim for."),
        FunctionSpec::new("route_fault", Type::Str.nullable())
            .param("who", subject(entity), "Who would run it.")
            .param("route", Type::list(subject(loc)), "The cells, in order.")
            .doc("Why a route cannot be run, or null."),
        FunctionSpec::new("perceives", Type::Bool)
            .param("who", subject(entity), "The one perceiving.")
            .param("whom", subject(entity), "The one perceived.")
            .belief_safe(false)
            .doc("Whether one has noticed the other. Truth."),
    ];
    for spec in functions {
        registry
            .add_function(spec)
            .expect("the functions have distinct names");
    }
    let verbs = [
        VerbSpec::new("strike")
            .arg("target", subject(entity), "Who is struck.")
            .optional_arg("part", subject(part), "Where.")
            .doc("Stands for whatever an action does."),
        VerbSpec::new("sweep")
            .arg("who", subject(entity), "Who moves.")
            .arg("to", subject(loc).nullable(), "Where to.")
            .doc("Stands for moving along a route."),
        VerbSpec::new("message")
            .arg("text", Type::Str, "The line.")
            .direct("text")
            .doc("Stands for a line in the log."),
    ];
    for spec in verbs {
        registry
            .add_verb(spec)
            .expect("the verbs have distinct names");
    }
    registry
}

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    register_defs(&mut types);
    types
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/action/mods")
}

fn load(mods: &[&str]) -> LoadOutcome {
    let types = types();
    let outcome = Loader::new(&types)
        .expr_registry(&registry())
        .search_path(fixtures())
        .load_mods(mods);
    for diagnostic in &outcome.diagnostics {
        assert!(
            explain(&diagnostic.code).is_some(),
            "code {} has no explanation",
            diagnostic.code
        );
    }
    outcome
}

fn spec() -> Arc<Defs> {
    let outcome = load(&["spec"]);
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("a clean load gives definitions")
}

fn action<'a>(defs: &'a Defs, id: &str) -> &'a ActionDef {
    let id = defs
        .lookup::<ActionDef>(id)
        .unwrap_or_else(|| panic!("no action `{id}`"));
    defs.get::<ActionDef>(id)
}

fn channel(defs: &Defs, id: &str) -> ChannelId {
    defs.lookup::<ChannelDef>(id)
        .unwrap_or_else(|| panic!("no channel `{id}`"))
}

fn window_kind(defs: &Defs, id: &str) -> WindowKindId {
    defs.lookup::<WindowKindDef>(id)
        .unwrap_or_else(|| panic!("no window kind `{id}`"))
}

fn seconds(value: f64) -> Real {
    Real::of(value)
}

/// A host that answers the stubs: the actor and the locals by slot, and
/// fixed answers for the properties and functions the fixtures call.
struct Stub {
    registry: Arc<Registry>,
    actor: Value,
    locals: Vec<Value>,
    distance: Real,
    warnings: Vec<RuntimeWarning>,
}

impl Stub {
    fn new(defs: &Defs, locals: Vec<Value>) -> Stub {
        let registry = Arc::clone(defs.expr_registry());
        let actor = Value::Subject(Handle::new(kind_of(&registry, "entity"), 1));
        Stub {
            registry,
            actor,
            locals,
            distance: Real::ONE,
            warnings: Vec::new(),
        }
    }

    fn entity(&self, number: u64) -> Value {
        Value::Subject(Handle::new(kind_of(&self.registry, "entity"), number))
    }

    fn cell(&self, number: u64) -> Value {
        Value::Subject(Handle::new(kind_of(&self.registry, "loc"), number))
    }
}

fn kind_of(registry: &Registry, name: &str) -> KindId {
    registry.kind_id(name).expect("the kind is registered")
}

impl Host for Stub {
    fn subject(&mut self, slot: u16) -> Value {
        if slot == 0 {
            self.actor.clone()
        } else {
            Value::Null
        }
    }

    fn local(&mut self, slot: u16) -> Value {
        self.locals
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, _args: &[Value]) -> Value {
        let name = self
            .registry
            .property(property)
            .map(|property| property.spec.name.clone())
            .unwrap_or_default();
        match name.as_str() {
            "attr" => Value::Subject(Handle::new(
                kind_of(&self.registry, "attributes"),
                subject.payload,
            )),
            "quickness" => Value::Real(Real::of(2.0)),
            "reaction_time" => Value::Real(Real::of(0.25)),
            _ => Value::Null,
        }
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        let name = self
            .registry
            .function(function)
            .map(|function| function.spec.name.clone())
            .unwrap_or_default();
        match name.as_str() {
            "dist" => Value::Real(self.distance),
            "auto_part" => Value::Subject(Handle::new(kind_of(&self.registry, "part"), 7)),
            "route_fault" => match args.get(1).and_then(Value::as_list) {
                Some(route) if route.len() > 2 => Value::text("too far"),
                _ => Value::Null,
            },
            "perceives" => Value::Bool(false),
            _ => Value::Null,
        }
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.warnings.push(warning);
    }
}

// ---- Done when: the examples of 03 §6 and §8 load -----------------------

#[test]
fn the_examples_of_document_03_load() {
    let defs = spec();
    assert_eq!(defs.registry::<ChannelDef>().len(), 4);
    assert_eq!(defs.registry::<WindowKindDef>().len(), 2);
    assert_eq!(defs.registry::<ActionDef>().len(), 7);
    // Dense ids in order of qualified id (04 §7.2).
    assert_eq!(channel(&defs, "spec:arm"), ChannelId::new(0));
    assert_eq!(channel(&defs, "spec:voice"), ChannelId::new(3));
    let legs = defs.get::<ChannelDef>(channel(&defs, "spec:legs"));
    assert_eq!(legs.name.as_ref().map(|name| name.raw()), Some("legs"));
    assert!(
        defs.get::<ChannelDef>(channel(&defs, "spec:arm"))
            .name
            .is_none()
    );
}

#[test]
fn a_punch_is_phases_by_index_and_channels_as_a_set() {
    let defs = spec();
    let punch = action(&defs, "spec:heavy_punch");
    let arm = channel(&defs, "spec:arm");
    assert_eq!(
        punch.name.as_ref().map(|name| name.raw()),
        Some("Heavy punch")
    );

    // Channels: a bitset, and the slots needed of each.
    assert_eq!(punch.uses().set(), ChannelSet::of(arm));
    assert_eq!(punch.channels(), ChannelSet::of(arm));
    assert_eq!(punch.uses().need(arm), Some(Slots::Count(1)));
    assert_eq!(punch.uses().need(channel(&defs, "spec:legs")), None);
    assert!(!punch.is_joint());

    // Defaults of the fields left out.
    assert!(punch.foreground && punch.footing);
    assert!(!punch.on_behalf && !punch.low_attention);
    assert!(punch.reaction_to().is_empty() && punch.ready().is_none());
    assert_eq!(punch.requires_at_start.constant_value(), Some(true));
    assert!(punch.on_start.is_empty() && punch.on_abandon.is_empty());

    // Phases: by index, each leading to the one written after it.
    let phases = punch.phases();
    assert_eq!(
        phases.iter().map(|phase| phase.name()).collect::<Vec<_>>(),
        ["windup", "impact", "recovery"]
    );
    assert_eq!(punch.phase_index("impact"), Some(PhaseIndex::new(1)));
    assert_eq!(punch.phase_index("nowhere"), None);
    assert!(punch.phase(PhaseIndex::new(3)).is_none());
    assert_eq!(phases[0].after(), Next::Phase(PhaseIndex::new(1)));
    assert_eq!(phases[1].after(), Next::Phase(PhaseIndex::new(2)));
    assert_eq!(phases[2].after(), Next::Finish);
    assert!(phases[0].next.is_empty() && phases[0].falls_through());
    assert_eq!(
        phases[0].exits().collect::<Vec<_>>(),
        [Next::Phase(PhaseIndex::new(1))]
    );

    // Durations are known at load where they are constants.
    let duration = |index: usize| {
        phases[index]
            .time
            .duration()
            .and_then(|time| time.constant())
            .and_then(Value::as_number)
    };
    assert_eq!(duration(0), Some(seconds(0.3)));
    assert_eq!(duration(1), Some(Real::ZERO));
    assert_eq!(duration(2), Some(seconds(0.4)));
    assert!(phases[0].cancel.at_will());
    assert_eq!(phases[1].cancel, CancelRule::Never);
    assert!(!phases[0].fixed && !phases[0].uninterruptible);

    // The window: its kind by id, and the phase that closes it by index.
    let window = phases[0]
        .window
        .as_ref()
        .expect("the windup opens a window");
    assert_eq!(window.kind_id(), window_kind(&defs, "spec:strike"));
    assert_eq!(window.closes.index(), PhaseIndex::new(1));
    assert_eq!(window.closes.name(), "impact");
    assert_eq!(window.aimed_at.len(), 1);

    // The effects are compiled against the action's layout: the actor
    // is the one subject, and the locals are the action's.
    let compiled = phases[1].on_enter.block().expect("the list is compiled");
    assert_eq!(compiled.len(), 1);
    assert_eq!(&*compiled.effects()[0].path, "phases.impact.do[0]");
    assert!(matches!(
        &compiled.effects()[0].kind,
        EffectKind::Verb { args, .. } if args.iter().all(Option::is_some)
    ));
    assert_eq!(phases[1].on_enter.subject_names().len(), 1);
    assert_eq!(&*phases[1].on_enter.subject_names()[0], "self");
    assert_eq!(phases[1].on_enter.frame_size(), punch.locals().len());

    // They are also kept as written, in order, with their places.
    let effects = &phases[1].on_enter;
    assert_eq!(effects.len(), 1);
    assert_eq!(effects.path(), "phases.impact.do");
    let strike = effects.effects()[0]
        .as_object()
        .expect("an effect is an object");
    assert_eq!(strike[0].key, "strike");
    let args = strike[0].value.as_object().expect("its arguments");
    assert_eq!(
        args.iter().map(|arg| arg.key.as_str()).collect::<Vec<_>>(),
        ["target", "part"]
    );
    assert_eq!(args[1].value.as_str(), Some("$part"));
    let location = defs.sources().locate(args[1].value.span);
    assert!(location.file.ends_with("spec/data/actions.json"));
    assert_eq!(
        location.source_line.trim(),
        r#""do": [ { "strike": { "target": "$target", "part": "$part" } } ] },"#
    );
}

#[test]
fn parameters_take_slots_in_the_order_the_scope_uses() {
    let defs = spec();
    let punch = action(&defs, "spec:heavy_punch");
    let entity = Type::Subject(kind_of(defs.expr_registry(), "entity"));
    let part = Type::Subject(kind_of(defs.expr_registry(), "part"));
    let locals = punch.locals();
    assert_eq!(
        locals
            .iter()
            .map(|local| (local.name.as_str(), &local.ty, local.source))
            .collect::<Vec<_>>(),
        [
            ("target", &entity, LocalSource::Param(0)),
            ("part", &part, LocalSource::Param(1)),
        ]
    );
    assert_eq!(punch.local_slot("part"), Some(1));
    assert_eq!(punch.local_slot("nobody"), None);
    let target = punch.param("target").expect("a parameter");
    assert!(target.is_required() && target.list().is_none());
    assert_eq!((target.slot(), target.ty()), (0, &entity));
    let aimed = punch.param("part").expect("a parameter");
    assert!(!aimed.is_required());
    assert_eq!((aimed.slot(), aimed.ty()), (1, &part));

    // Evaluated against a host that lays the locals out in that order.
    let mut runner = Runner::new();
    let mut host = Stub::new(&defs, Vec::new());
    host.locals = vec![host.entity(2), Value::Null];
    // The default of `part` reads the parameter before it.
    let default = aimed.default.as_ref().expect("a default");
    let chosen = default.value(&mut runner.on(&defs, &mut host));
    assert_eq!(
        chosen.as_subject().map(|handle| handle.payload),
        Some(7),
        "auto_part($target)"
    );
    // The same default written as a value source, by the function's
    // name, with its argument compiled in the same scope.
    let jab = action(&defs, "spec:jab");
    assert_eq!(jab.param("target").map(|target| target.ty()), Some(&entity));
    let named = jab
        .param("part")
        .and_then(|part| part.default.as_ref())
        .expect("a default");
    let chosen = named.value(&mut runner.on(&defs, &mut host));
    assert_eq!(chosen.as_subject().map(|handle| handle.payload), Some(7));
    // `requires` holds at one metre and not at two.
    assert!(punch.requires.holds(&mut runner.on(&defs, &mut host)));
    host.distance = Real::of(2.0);
    assert!(!punch.requires.holds(&mut runner.on(&defs, &mut host)));
    // Tempo reads the actor's own attribute.
    assert_eq!(
        punch.tempo.eval(&mut runner.on(&defs, &mut host)),
        Some(Real::of(2.0))
    );
    // The window is aimed at whoever `$target` is.
    let window = punch.phases()[0].window.as_ref().expect("a window");
    assert_eq!(
        window.aimed_at[0].value(&mut runner.on(&defs, &mut host)),
        host.entity(2)
    );
    assert!(host.warnings.is_empty());
}

#[test]
fn an_action_uses_several_channels_and_every_slot_of_one() {
    let defs = spec();
    let bellow = action(&defs, "spec:bellow");
    let (arm, legs, voice) = (
        channel(&defs, "spec:arm"),
        channel(&defs, "spec:legs"),
        channel(&defs, "spec:voice"),
    );
    let set: ChannelSet = [arm, legs, voice].into_iter().collect();
    assert_eq!(bellow.uses().set(), set);
    // In order of channel id, whatever order they were written in.
    assert_eq!(
        bellow
            .uses()
            .needs()
            .iter()
            .map(|need| (need.channel, need.slots))
            .collect::<Vec<_>>(),
        [
            (arm, Slots::Count(2)),
            (legs, Slots::All),
            (voice, Slots::Count(1))
        ]
    );
    // Two actions can be in each other's way only if their sets meet.
    let sidestep = action(&defs, "spec:sidestep");
    assert!(bellow.channels().intersects(sidestep.channels()));
    let punch = action(&defs, "spec:heavy_punch");
    assert!(!punch.channels().intersects(sidestep.channels()));

    assert!(!bellow.foreground && !bellow.footing && bellow.low_attention);
    let loud = defs.tags().lookup("loud").expect("the tag is interned");
    assert!(bellow.tags.contains(loud));
    let roar = &bellow.phases()[0];
    assert!(roar.uninterruptible && roar.fixed);
    assert!(
        roar.tags
            .contains(defs.tags().lookup("telegraph").expect("a tag"))
    );
    // Every effect list of 03 §6.1 and §13.
    assert_eq!(bellow.on_start.len(), 1);
    assert_eq!(bellow.on_start.path(), "on_start");
    assert!(bellow.on_finish.is_empty() && bellow.on_finish.node().is_some());
    assert_eq!(bellow.on_cancel.len(), 1);
    assert_eq!(bellow.on_interrupt.len(), 1);
    assert_eq!(bellow.on_abandon.len(), 1);
    assert_eq!(Effects::empty().len(), 0);
    // And each is compiled: a call of the verb, with its one argument.
    let block = bellow.on_start.block().expect("the list is compiled");
    assert_eq!(block.len(), 1);
    assert_eq!(block.path(), "on_start");
    assert!(matches!(
        &block.effects()[0].kind,
        EffectKind::Verb { args, .. } if args.len() == 1 && args[0].is_some()
    ));
    assert!(
        bellow
            .on_finish
            .block()
            .is_some_and(|block| block.is_empty())
    );
    assert_eq!(bellow.effect_lists().count(), 7);
}

// ---- Review addition: list parameters -----------------------------------

#[test]
fn a_list_parameter_has_bounds_and_is_indexable() {
    let defs = spec();
    let run = action(&defs, "spec:planned_run");
    let loc = Type::Subject(kind_of(defs.expr_registry(), "loc"));
    let route = run.param("route").expect("a parameter");
    assert_eq!(route.ty(), &Type::list(loc.clone()));
    let bounds = route.list().expect("a list has bounds");
    assert_eq!(
        bounds,
        ListBounds {
            min: 1,
            max: Some(24)
        }
    );
    assert!(!bounds.allows(0));
    assert!(bounds.allows(1) && bounds.allows(24));
    assert!(!bounds.allows(25));
    assert!(route.is_required());
    assert!(run.param("pace").expect("a parameter").list().is_none());

    // Locals: the parameters, then the variables.
    assert_eq!(
        run.locals()
            .iter()
            .map(|local| (local.name.as_str(), local.source))
            .collect::<Vec<_>>(),
        [
            ("route", LocalSource::Param(0)),
            ("pace", LocalSource::Param(1)),
            ("i", LocalSource::Var(0)),
            ("last", LocalSource::Var(1)),
        ]
    );
    let vars = run.vars();
    assert_eq!(vars[0].ty(), &Type::Int);
    assert_eq!(
        vars[0]
            .default
            .as_ref()
            .and_then(|default| default.constant()),
        Some(&Value::Int(0))
    );
    // A variable with no default starts as null, and its type says so.
    assert_eq!(vars[1].ty(), &loc.nullable());
    assert!(vars[1].default.is_none());
    assert_eq!((vars[0].slot(), vars[1].slot()), (2, 3));

    // The step loops while there is a cell left: `$route[$i]`.
    let mut runner = Runner::new();
    let mut host = Stub::new(&defs, Vec::new());
    let cells = Value::list([host.cell(10), host.cell(11), host.cell(12)]);
    let step = &run.phases()[0];
    let again = step.next[0].when.as_ref().expect("a condition");
    assert_eq!(step.next[0].to(), Next::Phase(PhaseIndex::new(0)));
    assert_eq!(step.next[1].to(), Next::Finish);
    assert!(step.next[1].is_unconditional() && !step.falls_through());
    host.locals = vec![
        cells.clone(),
        Value::Real(Real::ONE),
        Value::Int(1),
        Value::Null,
    ];
    assert!(again.holds(&mut runner.on(&defs, &mut host)));
    host.locals[2] = Value::Int(3);
    assert!(!again.holds(&mut runner.on(&defs, &mut host)));

    // `check` is advice, evaluated on request: it gives a reason or
    // nothing and refuses nothing by itself.
    let check = route.check.as_ref().expect("a check");
    assert_eq!(
        check.value(&mut runner.on(&defs, &mut host)),
        Value::text("too far")
    );
    host.locals[0] = Value::list([host.cell(10)]);
    assert!(check.value(&mut runner.on(&defs, &mut host)).is_null());
    assert!(host.warnings.is_empty());
}

// ---- 03 §6.4 ------------------------------------------------------------

#[test]
fn a_held_phase_branches_by_index() {
    let defs = spec();
    let blast = action(&defs, "spec:charged_blast");
    let phases = blast.phases();
    let charge = &phases[0];
    assert!(charge.is_held() && matches!(charge.time, PhaseTime::Hold));
    assert!(charge.time.duration().is_none());
    assert_eq!(
        charge
            .max
            .as_ref()
            .and_then(|max| max.constant())
            .and_then(Value::as_number),
        Some(seconds(3.0))
    );
    let tick = charge.tick.as_ref().expect("a tick");
    assert_eq!(
        tick.every.constant().and_then(Value::as_number),
        Some(seconds(0.25))
    );
    assert_eq!(tick.effects.len(), 1);
    assert_eq!(tick.effects.path(), "phases.charge.tick.do");
    assert!(charge.cancel.at_will());

    // `next`: the first entry that holds, by index.
    assert_eq!(charge.next.len(), 2);
    assert_eq!(charge.next[0].to(), Next::Phase(PhaseIndex::new(1)));
    assert_eq!(charge.next[1].to(), Next::Phase(PhaseIndex::new(2)));
    assert!(!charge.next[0].is_unconditional() && charge.next[1].is_unconditional());
    assert!(!charge.falls_through());
    assert_eq!(
        charge.exits().collect::<Vec<_>>(),
        [
            Next::Phase(PhaseIndex::new(1)),
            Next::Phase(PhaseIndex::new(2))
        ]
    );
    // `release` finishes the action and does not run on into `fizzle`.
    assert_eq!(phases[1].exits().collect::<Vec<_>>(), [Next::Finish]);
    assert_eq!(phases[1].on_exit.len(), 1);
    assert_eq!(phases[2].after(), Next::Finish);

    // A recovery that only a follow-up may cut short.
    let tag = |name: &str| defs.tags().lookup(name).expect("the tag is interned");
    let CancelRule::For(allowed) = &phases[2].cancel else {
        panic!("`fizzle` lists tags");
    };
    assert!(allowed.contains(tag("combo")) && allowed.contains(tag("dodge")));
    let sidestep = action(&defs, "spec:sidestep");
    assert!(phases[2].cancel.allows(&sidestep.tags));
    assert!(!phases[2].cancel.allows(&blast.tags));
    assert!(!phases[2].cancel.at_will());
    assert!(!phases[1].cancel.allows(&sidestep.tags));

    // The variable is a local of every phase.
    let mut runner = Runner::new();
    let mut host = Stub::new(&defs, vec![Value::Int(3)]);
    let until = charge.until.as_ref().expect("a condition");
    let enough = charge.next[0].when.as_ref().expect("a condition");
    assert!(!until.holds(&mut runner.on(&defs, &mut host)));
    assert!(!enough.holds(&mut runner.on(&defs, &mut host)));
    host.locals[0] = Value::Int(10);
    assert!(until.holds(&mut runner.on(&defs, &mut host)));
    assert!(enough.holds(&mut runner.on(&defs, &mut host)));
    assert_eq!(
        blast.fail_text.as_ref().map(|text| text.raw()),
        Some("{cap(the(self))} cannot gather the power.")
    );
    assert!(
        blast
            .fail_text
            .as_ref()
            .is_some_and(|text| text.compiled().is_some())
    );
}

// ---- 03 §7.1, §7.3 ------------------------------------------------------

#[test]
fn a_window_kind_says_who_notices_and_how_soon() {
    let defs = spec();
    let strike = defs.get::<WindowKindDef>(window_kind(&defs, "spec:strike"));
    assert_eq!(strike.reach.get(), Real::of(6.0));
    assert!(strike.notice.compiled().is_some());
    let mut runner = Runner::new();
    let mut host = Stub::new(&defs, Vec::new());
    assert_eq!(
        strike.notice.eval(&mut runner.on(&defs, &mut host)),
        Some(seconds(0.25))
    );
    assert!(strike.noticed_if.holds(&mut runner.on(&defs, &mut host)));

    // Everything left out: noticed at once, by anyone, with no reach.
    let plain = defs.get::<WindowKindDef>(window_kind(&defs, "spec:plain"));
    assert_eq!(
        plain.notice.constant().and_then(Value::as_number),
        Some(Real::ZERO)
    );
    assert_eq!(plain.noticed_if.constant_value(), Some(true));
    assert_eq!(plain.reach.get(), Real::ZERO);
    // A duration in data stays what the loader reads elsewhere.
    assert_eq!(Time(Dur::from_millis(250)).dur(), Dur::from_millis(250));
}

#[test]
fn a_reaction_lists_the_windows_it_answers() {
    let defs = spec();
    let sidestep = action(&defs, "spec:sidestep");
    let (strike, plain) = (
        window_kind(&defs, "spec:strike"),
        window_kind(&defs, "spec:plain"),
    );
    // In order of id, whatever order they were written in.
    assert_eq!(sidestep.reaction_to(), [plain, strike]);
    assert!(sidestep.reacts_to(strike) && sidestep.reacts_to(plain));
    assert!(sidestep.on_behalf);
    assert_eq!(sidestep.ready(), Some(PhaseIndex::new(1)));
    let punch = action(&defs, "spec:heavy_punch");
    assert!(!punch.reacts_to(strike));

    // The window being answered is the first local, `$window`.
    let window = Type::Subject(kind_of(defs.expr_registry(), "window")).nullable();
    assert_eq!(sidestep.locals().len(), 1);
    assert_eq!(sidestep.locals()[0].name, "window");
    assert_eq!(sidestep.locals()[0].ty, window);
    assert_eq!(sidestep.locals()[0].source, LocalSource::Window);
    let mut runner = Runner::new();
    let mut host = Stub::new(&defs, vec![Value::Null]);
    assert!(!sidestep.requires.holds(&mut runner.on(&defs, &mut host)));
    host.locals[0] = Value::Subject(Handle::new(kind_of(defs.expr_registry(), "window"), 4));
    assert!(sidestep.requires.holds(&mut runner.on(&defs, &mut host)));
}

// ---- 03 §8 --------------------------------------------------------------

#[test]
fn a_joint_action_has_roles_bound_under_their_names() {
    let defs = spec();
    let takedown = action(&defs, "spec:takedown");
    let (arm, legs) = (channel(&defs, "spec:arm"), channel(&defs, "spec:legs"));
    assert!(takedown.is_joint());
    assert!(takedown.uses().is_empty());
    let roles = takedown.roles();
    assert_eq!(
        roles
            .iter()
            .map(|role| (role.name.value.as_str(), role.forced, role.slot()))
            .collect::<Vec<_>>(),
        [("attacker", false, 0), ("victim", true, 1)]
    );
    assert_eq!(roles[0].uses.need(arm), Some(Slots::All));
    assert_eq!(roles[0].uses.set(), ChannelSet::of(arm));
    assert_eq!(
        roles[1].uses.set(),
        ChannelSet::of(arm) | ChannelSet::of(legs)
    );
    assert_eq!(
        takedown.channels(),
        ChannelSet::of(arm) | ChannelSet::of(legs)
    );

    // Each role is a local of kind `entity`, before any parameter.
    let entity = Type::Subject(kind_of(defs.expr_registry(), "entity"));
    assert_eq!(
        takedown
            .locals()
            .iter()
            .map(|local| (local.name.as_str(), &local.ty, local.source))
            .collect::<Vec<_>>(),
        [
            ("attacker", &entity, LocalSource::Role(0)),
            ("victim", &entity, LocalSource::Role(1)),
        ]
    );

    // `requires_at_start` reads truth: `perceives` is not safe for belief.
    let mut runner = Runner::new();
    let mut host = Stub::new(&defs, Vec::new());
    host.locals = vec![host.entity(1), host.entity(2)];
    assert!(
        takedown
            .requires_at_start
            .holds(&mut runner.on(&defs, &mut host))
    );
    assert_eq!(takedown.requires.constant_value(), Some(true));
    assert_eq!(
        takedown
            .phases()
            .iter()
            .map(|phase| phase.name())
            .collect::<Vec<_>>(),
        ["seize", "finish", "settle"]
    );
    assert_eq!(takedown.phases()[1].on_enter.len(), 1);
}

// ---- the ids ------------------------------------------------------------

#[test]
fn ids_are_dense_and_the_types_are_documented() {
    let defs = spec();
    let ids: Vec<ActionDefId> = defs.registry::<ActionDef>().ids().collect();
    assert_eq!(
        ids.iter().map(|id| id.index()).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6]
    );
    let names: BTreeSet<&str> = types().names().into_iter().collect();
    for name in ["action", "channel", "window_kind"] {
        assert!(names.contains(name), "`{name}` is not registered");
    }
    for info in types().infos() {
        assert!(!info.doc.is_empty(), "`{}` has no documentation", info.name);
    }
    // The loader makes the ids of the types known to expressions.
    assert!(defs.expr_registry().parse_type("id<action>").is_some());
    assert!(defs.expr_registry().parse_type("id<channel>").is_some());
    assert!(defs.expr_registry().parse_type("id<window_kind>").is_some());
}

// ---- Done when: each validation failure has a located diagnostic --------

/// One diagnostic the `broken` mod is expected to give: the data file
/// under `broken/data/`, the definition in the namespace `broken`, the
/// text the caret starts at (the first place it appears after the
/// definition's id), the code, the JSON path and part of the message.
type Expected = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const PHASES: &str = "phases.json";
const PARAMS: &str = "params.json";
const CHANNELS: &str = "channels.json";

/// Every mistake in the `broken` mod: where it is and what is said.
#[rustfmt::skip]
const BROKEN: &[Expected] = &[
    // Phases (03 §6.4).
    (PHASES, "two_alike", r#""swing", "time": "200 ms""#, "E0701", "phases.swing.id", "two phases called `swing`"),
    (PHASES, "goto_nowhere", r#""impcat""#, "E0701", "phases.windup.next[0].goto", "no phase `impcat`"),
    (PHASES, "closes_nowhere", r#""landing""#, "E0701", "phases.windup.window.closes", "no phase `landing`"),
    (PHASES, "closes_itself", r#""windup" } }"#, "E0701", "phases.windup.window.closes", "the phase that opens it"),
    (PHASES, "closes_behind", r#""ready" } }"#, "E0701", "phases.windup.window.closes", "can never close"),
    (PHASES, "never_entered", r#""flourish""#, "E0701", "phases.flourish", "is never entered"),
    (PHASES, "goes_nowhere", r#""true""#, "E0701", "phases.windup.next[0]", "says where to go"),
    (PHASES, "goes_two_ways", r#""impact", "finish""#, "E0701", "phases.windup.next[0].goto", "it does one"),
    (PHASES, "max_unheld", r#""3 s""#, "E0701", "phases.charge.max", "the longest a held phase lasts"),
    (PHASES, "until_unheld", r#""true""#, "E0701", "phases.charge.until", "ends a held phase"),
    (PHASES, "until_untested", r#""$power >= 10""#, "E0701", "phases.charge.until", "has no `tick`"),
    (PHASES, "no_phases", r#""no_phases""#, "E0701", "phases", "at least one phase"),
    (PHASES, "bad_phase_id", r#""Wind Up""#, "E0701", "phases.Wind Up.id", "is not a phase id"),
    (PHASES, "negative_time", r#""-100 ms""#, "E0701", "phases.windup.time", "less than no time"),
    (PHASES, "no_interval", "0 } }", "E0701", "phases.burn.tick.every", "above zero"),
    (PHASES, "no_tempo", "0,", "E0701", "tempo", "above zero"),
    (PHASES, "time_unknown", "soon", "E0621", "phases.windup.time", "unknown name `soon`"),
    (PHASES, "cancel_maybe", r#""maybe""#, "E0201", "phases.windup.cancel", "a list of action tags"),
    (PHASES, "effects_not_a_list", r#"{ "strike""#, "E0201", "phases.impact.do", "expected a list of effects"),
    (PHASES, "local_unknown", "$pwer", "E0622", "phases.charge.until", "unknown local `$pwer`"),
    (PHASES, "aimed_at_a_part", "$part", "E0645", "phases.windup.window.for[0]", "must be entity?"),
    (PHASES, "dead_entry", "true", "W0702", "phases.windup.next[1]", "never tried"),
    // Parameters and variables (03 §6.3, §6.4).
    (PARAMS, "type_unknown", r#""entty""#, "E0671", "params.target.type", "is not a type"),
    (PARAMS, "list_of_what", r#""list""#, "E0701", "params.route.type", "what it is a list of"),
    (PARAMS, "of_without_list", r#""cell""#, "E0701", "params.target.of", "belongs with the type `list`"),
    (PARAMS, "element_unknown", r#""sell""#, "E0671", "params.route.of", "is not a type"),
    (PARAMS, "length_of_one", "3 }", "E0701", "params.target.max", "is the length of a list"),
    (PARAMS, "length_backwards", "2 }", "E0701", "params.route.max", "below its least"),
    (PARAMS, "default_of_wrong_type", "3 }", "E0645", "params.part.default", "must be part"),
    (PARAMS, "default_gives_wrong_type", "dist(self, $target)", "E0645", "params.part.default", "must be part"),
    (PARAMS, "default_reads_ahead", "$target", "E0622", "params.part.default", "unknown local `$target`"),
    (PARAMS, "default_reads_truth", "$target.loc", "E0662", "params.spot.default", "not safe for belief"),
    (PARAMS, "requires_reads_truth", "perceives", "E0662", "requires", "not safe for belief"),
    (PARAMS, "requires_draws", "chance", "E0661", "requires", "must be pure"),
    (PARAMS, "name_not_a_local", r#""Target""#, "E0701", "params.Target", "cannot be written `$Target`"),
    (PARAMS, "name_reserved", r#""if""#, "E0701", "vars.if", "cannot be written `$if`"),
    (PARAMS, "name_taken", r#""power": {"#, "E0701", "vars.power", "already names a parameter"),
    (PARAMS, "name_is_the_window", r#""window""#, "E0701", "params.window", "already names the window a reaction answers"),
    (PARAMS, "var_of_wrong_type", "true", "E0645", "vars.power.default", "must be int"),
    (PARAMS, "var_type_unknown", r#""integer""#, "E0671", "vars.power.type", "is not a type"),
    // Channels, windows and roles (03 §6.2, §7, §8).
    (CHANNELS, "channel_unknown", r#""amr""#, "E0501", "uses.amr", "no channel \"amr\""),
    (CHANNELS, "no_slots", "0 }", "E0207", "uses.arm", "at least one slot"),
    (CHANNELS, "slots_in_words", r#""both""#, "E0207", "uses.arm", "a number of slots, or \"all\""),
    (CHANNELS, "channel_twice", r#""spec:arm""#, "E0701", "uses", "named twice"),
    (CHANNELS, "window_kind_unknown", r#""stirke""#, "E0501", "phases.windup.window.kind", "no window_kind \"stirke\""),
    (CHANNELS, "reaction_to_unknown", r#""shove""#, "E0501", "reaction_to[1]", "no window_kind \"shove\""),
    (CHANNELS, "ready_nowhere", r#""guard""#, "E0701", "ready", "no phase `guard`"),
    (CHANNELS, "ready_for_nothing", r#""ready_for_nothing""#, "W0702", "on_behalf", "answers no window"),
    (CHANNELS, "ready_for_nothing", r#""set""#, "W0702", "ready", "answers no window"),
    (CHANNELS, "reaction_twice", r#""spec:strike""#, "W0702", "reaction_to[1]", "listed twice"),
    (CHANNELS, "one_role", r#"{ "attacker""#, "E0701", "roles", "at least 2 roles"),
    (CHANNELS, "uses_beside_roles", r#""arm""#, "E0701", "uses", "what each role uses"),
    (CHANNELS, "first_role_forced", "true", "E0701", "roles.attacker.forced", "cannot be forced"),
    (CHANNELS, "role_and_parameter", r#""victim": "entity""#, "E0701", "params.victim", "already names a role"),
    (CHANNELS, "role_channel_unknown", r#""tail""#, "E0501", "roles.victim.uses.tail", "no channel \"tail\""),
    (CHANNELS, "role_unknown", "$victm", "E0622", "requires_at_start", "unknown local `$victm`"),
    (CHANNELS, "key_unknown", r#""footting""#, "E0203", "", "unknown key \"footting\""),
    (CHANNELS, "key_unknown", r#""uninterruptable""#, "E0203", "phases.act", "unknown key \"uninterruptable\""),
    (CHANNELS, "reach_backwards", r#""reach_backwards""#, "E0701", "reach", "cannot be below zero"),
    (CHANNELS, "notice_backwards", r#""-250 ms""#, "E0701", "notice", "cannot be below zero"),
    (CHANNELS, "notice_of_whom", "target", "E0621", "notice", "unknown name `target`"),
    (CHANNELS, "window_key_unknown", r#""reech""#, "E0203", "", "unknown key \"reech\""),
    (CHANNELS, "channel_key_unknown", r#""slots""#, "E0203", "", "unknown key \"slots\""),
];

/// The line and the column, both from one, at which `needle` first
/// appears in `text` after the definition `def`.
fn position(text: &str, def: &str, needle: &str) -> (u32, u32) {
    let marker = format!("\"id\": \"{def}\"");
    let from = text
        .find(&marker)
        .unwrap_or_else(|| panic!("the fixture has no definition `{def}`"));
    let at = from
        + text[from..]
            .find(needle)
            .unwrap_or_else(|| panic!("`{def}` does not hold `{needle}`"));
    let line = text[..at].matches('\n').count();
    let line_start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let column = text[line_start..at].chars().count();
    (line as u32 + 1, column as u32 + 1)
}

/// What identifies a diagnostic: file, line, column, definition, code and
/// JSON path.
type Place = (String, u32, u32, String, String, String);

#[test]
fn every_mistake_is_a_diagnostic_with_file_line_column_and_path() {
    let outcome = load(&["spec", "broken"]);
    assert!(
        outcome.defs.is_none(),
        "a load with errors gives no definitions"
    );

    let mut expected: Vec<(Place, &str)> = Vec::new();
    for (file, def, at, code, path, says) in BROKEN {
        let text = std::fs::read_to_string(fixtures().join("broken/data").join(file))
            .expect("the fixture can be read");
        let (line, column) = position(&text, def, at);
        let place = (
            format!("broken/data/{file}"),
            line,
            column,
            format!("broken:{def}"),
            (*code).to_owned(),
            (*path).to_owned(),
        );
        expected.push((place, says));
    }

    let mut found: Vec<(Place, &Diagnostic)> = Vec::new();
    for diagnostic in &outcome.diagnostics {
        let location = diagnostic
            .location
            .as_ref()
            .unwrap_or_else(|| panic!("no place:\n{}", diagnostic.render()));
        let file = location
            .file
            .rsplit_once("mods/")
            .map_or(location.file.as_str(), |(_, file)| file);
        let definition = diagnostic
            .definition
            .as_ref()
            .unwrap_or_else(|| panic!("no definition:\n{}", diagnostic.render()));
        let place = (
            file.to_owned(),
            location.line,
            location.column,
            definition.id.clone(),
            diagnostic.code.clone(),
            diagnostic.path.clone(),
        );
        found.push((place, diagnostic));
    }

    // Each expected diagnostic is there, at its place, saying its piece.
    for (place, says) in &expected {
        let Some((_, diagnostic)) = found.iter().find(|(at, _)| at == place) else {
            panic!(
                "nothing at {place:?}; the load said:\n{}",
                outcome.diagnostics.render()
            );
        };
        assert!(
            diagnostic.message.contains(*says),
            "at {place:?}: expected `{says}` in `{}`",
            diagnostic.message
        );
        let warning = place.4.starts_with('W');
        assert_eq!(
            diagnostic.severity == Severity::Warning,
            warning,
            "{place:?}"
        );
    }
    // And nothing else is said: one mistake, one diagnostic.
    let expected_places: BTreeSet<&Place> = expected.iter().map(|(place, _)| place).collect();
    for (place, diagnostic) in &found {
        assert!(
            expected_places.contains(place),
            "unexpected:\n{}",
            diagnostic.render()
        );
    }
    assert_eq!(found.len(), expected.len());
}

#[test]
fn a_misspelt_name_comes_with_the_closest_one() {
    let outcome = load(&["spec", "broken"]);
    let suggestion = |def: &str, code: &str| {
        outcome
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == code
                    && diagnostic
                        .definition
                        .as_ref()
                        .is_some_and(|label| label.id == format!("broken:{def}"))
            })
            .and_then(|diagnostic| diagnostic.suggestion.clone())
    };
    assert_eq!(
        suggestion("goto_nowhere", "E0701").as_deref(),
        Some("did you mean \"impact\"?")
    );
    assert_eq!(
        suggestion("channel_unknown", "E0501").as_deref(),
        Some("did you mean \"spec:arm\"?")
    );
    assert_eq!(
        suggestion("type_unknown", "E0671").as_deref(),
        Some("did you mean \"entity\"?")
    );
    assert_eq!(
        suggestion("local_unknown", "E0622").as_deref(),
        Some("did you mean `$power`?")
    );
    // The rendered form of 04 §8.2: code, file, line, caret, definition
    // and path.
    let rendered = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.path == "phases.windup.next[0].goto")
        .map(Diagnostic::render)
        .expect("the diagnostic is there");
    assert!(rendered.starts_with("error[E0701]: this action has no phase `impcat`\n"));
    assert!(rendered.contains("broken/data/phases.json:10:63"));
    assert!(rendered.contains("^^^^^^^^ did you mean \"impact\"?"));
    assert!(rendered.contains("= in: action broken:goto_nowhere, at phases.windup.next[0].goto"));
}

#[test]
fn a_lenient_load_puts_an_inert_action_in_place_of_a_broken_one() {
    let types = types();
    let outcome = Loader::new(&types)
        .expr_registry(&registry())
        .search_path(fixtures())
        .lenient(true)
        .load_mods(&["spec", "broken"]);
    assert!(outcome.has_errors());
    let defs = outcome.defs.expect("a lenient load carries on");
    let actions = defs.registry::<ActionDef>();
    let broken = defs
        .lookup::<ActionDef>("broken:goto_nowhere")
        .expect("it is still there");
    assert!(actions.is_placeholder(broken));
    let inert = actions.get(broken);
    // It can never start, uses nothing and has one phase of no length.
    assert_eq!(inert.requires.constant_value(), Some(false));
    assert!(inert.channels().is_empty() && inert.locals().is_empty());
    assert_eq!(inert.phases().len(), 1);
    assert_eq!(inert.phases()[0].after(), Next::Finish);
    assert_eq!(
        inert.phases()[0]
            .time
            .duration()
            .and_then(|time| time.constant())
            .and_then(Value::as_number),
        Some(Real::ZERO)
    );
    // One whose expressions did not compile is replaced as well.
    let uncompiled = defs
        .lookup::<ActionDef>("broken:local_unknown")
        .expect("it is still there");
    assert!(actions.is_placeholder(uncompiled));
    // A sound one beside them is itself.
    let punch = defs
        .lookup::<ActionDef>("spec:heavy_punch")
        .expect("it loaded");
    assert!(!actions.is_placeholder(punch));
    // A window kind that did not load is one nobody notices.
    let kinds = defs.registry::<WindowKindDef>();
    let kind = defs
        .lookup::<WindowKindDef>("broken:window_key_unknown")
        .expect("it is still there");
    assert!(kinds.is_placeholder(kind));
    assert_eq!(kinds.get(kind).noticed_if.constant_value(), Some(false));
}

#[test]
fn more_channels_than_a_set_holds_is_a_located_error() {
    let mut data = String::from("[\n");
    for index in 0..=ChannelSet::CAPACITY {
        let comma = if index == 0 { "" } else { ",\n" };
        data.push_str(&format!(
            "{comma}  {{ \"type\": \"channel\", \"id\": \"c{index:03}\" }}"
        ));
    }
    data.push_str("\n]\n");
    let types = types();
    let outcome = Loader::new(&types)
        .expr_registry(&registry())
        .memory_mod(
            "many",
            r#"{ "id": "many" }"#,
            &[("data/channels.json", &data)],
        )
        .load_all();
    assert!(outcome.defs.is_none());
    let found = outcome
        .diagnostics
        .with_code(clash_data::code::TOO_MANY_DEFINITIONS);
    assert_eq!(found.len(), 1, "{}", outcome.diagnostics.render());
    assert!(
        found[0]
            .message
            .contains("257 definitions of type `channel`")
    );
    let location = found[0].location.as_ref().expect("it has a place");
    assert!(location.file.ends_with("many/data/channels.json"));
    // The last of them: the one the set has no bit for.
    assert_eq!(location.line, 258);

    // One fewer is every bit of the set, and loads.
    let (kept, _) = data.rsplit_once(",\n").expect("more than one channel");
    let exact = format!("{kept}\n]\n");
    let outcome = Loader::new(&types)
        .expr_registry(&registry())
        .memory_mod(
            "many",
            r#"{ "id": "many" }"#,
            &[("data/channels.json", &exact)],
        )
        .load_all();
    assert_eq!(outcome.diagnostics.render(), "");
    let defs = outcome.defs.expect("it loads");
    assert_eq!(defs.registry::<ChannelDef>().len(), ChannelSet::CAPACITY);
    let all: ChannelSet = defs.registry::<ChannelDef>().ids().collect();
    assert_eq!(all.len(), ChannelSet::CAPACITY);
}

// ---- every refusal has a reason, and the document lists it --------------

#[test]
fn document_03_lists_every_reason_of_the_action_system() {
    let document = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/03-time-and-actions.md");
    let text = std::fs::read_to_string(document).expect("document 03 can be read");
    assert!(!clash_sim::action::reason::REASONS.is_empty());
    for reason in clash_sim::action::reason::REASONS {
        let name = reason
            .code
            .strip_prefix("engine:")
            .expect("a reason of the engine's");
        assert!(
            text.contains(&format!("`{name}`")),
            "03 §13 does not list the reason `{name}`"
        );
    }
    // The refusals the review addition and 14 §5 name are among them.
    for code in [
        "engine:action_no_footing",
        "engine:action_requires",
        "engine:action_channel_busy",
        "engine:action_list_length",
    ] {
        assert!(clash_sim::action::reason::find(code).is_some(), "{code}");
    }
}

// ---- actions are patched like anything else (04 §6.1) -------------------

#[test]
fn another_mod_changes_one_phase_without_restating_the_rest() {
    let tweak = r#"[
  { "type": "action", "id": "spec:heavy_punch", "patch": true,
    "scale": { "phases.windup.time": 0.5, "tempo": 1.0 },
    "phases": [
      { "id": "flourish", "time": "50 ms", "after": "impact" },
      { "id": "recovery", "cancel": ["dodge"] }
    ] },
  { "type": "action", "id": "spec:bellow", "patch": true,
    "scale": { "tempo": 1.5 } }
]"#;
    let manifest = r#"{ "id": "tweak", "requires": ["spec"] }"#;
    let types = types();
    let outcome = Loader::new(&types)
        .expr_registry(&registry())
        .search_path(fixtures())
        .memory_mod("tweak", manifest, &[("data/tweak.json", tweak)])
        .load_mods(&["spec", "tweak"]);
    // `tempo` of the punch is an expression, which cannot be scaled.
    assert_eq!(
        outcome.diagnostics.error_count(),
        1,
        "{}",
        outcome.diagnostics.render()
    );
    assert_eq!(outcome.diagnostics.as_slice()[0].code, "E0403");

    let tweak = tweak.replace(r#", "tempo": 1.0 }"#, " }");
    let outcome = Loader::new(&types)
        .expr_registry(&registry())
        .search_path(fixtures())
        .memory_mod("tweak", manifest, &[("data/tweak.json", &tweak)])
        .load_mods(&["spec", "tweak"]);
    assert_eq!(outcome.diagnostics.render(), "");
    let defs = outcome.defs.expect("it loads");
    let punch = action(&defs, "spec:heavy_punch");
    let phases = punch.phases();
    assert_eq!(
        phases.iter().map(|phase| phase.name()).collect::<Vec<_>>(),
        ["windup", "impact", "flourish", "recovery"]
    );
    let duration = |index: usize| {
        phases[index]
            .time
            .duration()
            .and_then(|time| time.constant())
            .and_then(Value::as_number)
    };
    // Half of 300 ms, still a duration.
    assert_eq!(duration(0), Some(seconds(0.15)));
    assert_eq!(duration(2), Some(seconds(0.05)));
    // Indices follow the new order: the window still closes at `impact`,
    // and `impact` now leads to the phase put after it.
    let window = phases[0].window.as_ref().expect("a window");
    assert_eq!(window.closes.index(), PhaseIndex::new(1));
    assert_eq!(phases[1].after(), Next::Phase(PhaseIndex::new(2)));
    assert_eq!(phases[3].after(), Next::Finish);
    assert!(matches!(phases[3].cancel, CancelRule::For(_)));
    assert_eq!(duration(3), Some(seconds(0.4)));
    // A field left out is scaled from the default the type declares.
    let bellow = action(&defs, "spec:bellow");
    assert_eq!(
        bellow.tempo.constant().and_then(Value::as_number),
        Some(Real::of(1.5))
    );
}
