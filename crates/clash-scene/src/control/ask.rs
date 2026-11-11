//! Putting a command together (12 §9.3): `act` with `"needed"` or
//! `"all"`, one picker for each parameter that is asked for.
//!
//! The scene never knows an action by name. It reads the action's
//! parameter list from its definition ([`ActionDef`]), works the
//! defaults out in the order written, in the mind of the acting
//! character and through the host, and asks for what the effect says is
//! to be asked for. What it hands the host at the end is a whole
//! [`UiCommand`]: the action's qualified id and a value for every
//! parameter that has one.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::Rng;
use clash_data::{Defs, Evaluator as FieldEvaluator};
use clash_expr::{
    DefRef, FunctionId, Handle, Host, PropertyId, Registry, RuntimeWarning, Type,
    Value as ExprValue,
};
use clash_sim::action::{ActionDef, ActionDefId, ListBounds};
use clash_world::CellPos;

use crate::defs::namespace_of;
use crate::scene::{Pending, ROLE_PICK_LIST, Scene, SceneHost, UiCommand, UiOutcome, Wait};
use crate::ui::{Binding, Collect, CommandEffect, CommandKind, PickerEffect, UiEffect, to_var};
use crate::view::Camera;

/// What the roles of pickers start with: the picker for a parameter is
/// the screen of the role `picker.<name>` (12 §10.7).
pub const PICKER_ROLE_PREFIX: &str = "picker.";
/// The picker for an entity: a cursor over the map.
pub const PICKER_ENTITY: &str = "entity";
/// The picker for a cell: a cursor over the map.
pub const PICKER_CELL: &str = "cell";
/// The picker for a list of cells, built one step at a time.
pub const PICKER_ROUTE: &str = "route";
/// The picker for a number.
pub const PICKER_NUMBER: &str = "number";
/// The picker for a text.
pub const PICKER_TEXT: &str = "text";
/// The picker for one of several values.
pub const PICKER_LIST: &str = "list";
/// The picker for several of several values: a list parameter.
pub const PICKER_MULTI: &str = "multi";
/// The picker for a direction: one key.
pub const PICKER_DIRECTION: &str = "direction";

/// The kind of subject an entity is.
const KIND_ENTITY: &str = "entity";
/// The kind of subject a cell is.
const KIND_LOC: &str = "loc";

/// The command a reaction is answered with (03 §4.4).
pub const COMMAND_REACT: &str = "react";

/// What a host is asked when a picker needs the things a character
/// could choose for a parameter (12 §9.3).
#[derive(Clone, Copy, Debug)]
pub struct ParamAsk<'a> {
    /// Who would act.
    pub actor: &'a ExprValue,
    /// The action's qualified id.
    pub action: &'a str,
    /// The parameter's name.
    pub param: &'a str,
    /// The type of one value: for a list parameter, of one entry.
    pub ty: &'a Type,
    /// The picker that is asking: `entity`, `list`, `multi` and so on.
    pub picker: &'a str,
}

/// One thing asked for: a role after the first, or a parameter.
#[derive(Clone, Debug)]
pub(crate) struct Ask {
    pub(crate) name: String,
    /// Its slot among the action's locals.
    pub(crate) slot: usize,
    pub(crate) ty: Type,
    /// Its position among the action's parameters. `None` for a role.
    pub(crate) param: Option<usize>,
    pub(crate) list: Option<ListBounds>,
}

impl Ask {
    /// The type of one value: for a list, of one entry.
    pub(crate) fn entry_type(&self) -> &Type {
        match (self.list, self.ty.non_null()) {
            (Some(_), Type::List(entry)) => entry.non_null(),
            _ => &self.ty,
        }
    }
}

/// What the finished command is.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// `Act`: the active character starts the action.
    Act,
    /// `React`: the action answers a window.
    React {
        entity: ExprValue,
        window: ExprValue,
    },
}

/// Something a map picker's cursor can be put on.
#[derive(Clone, Debug)]
pub(crate) struct MapCandidate {
    pub(crate) value: ExprValue,
    pub(crate) cell: CellPos,
}

/// A command being put together: what has been settled, what is being
/// asked now and what is still to come.
#[derive(Clone, Debug)]
pub(crate) struct Collecting {
    /// The action's qualified id. Empty while it is being chosen.
    pub(crate) action: String,
    pub(crate) def: Option<ActionDefId>,
    pub(crate) mode: Collect,
    pub(crate) target: Target,
    /// Who acts: `view` when the command was begun.
    pub(crate) actor: ExprValue,
    /// One value for each local of the action, in slot order.
    pub(crate) locals: Vec<ExprValue>,
    pub(crate) asks: Vec<Ask>,
    /// For each of `asks`, whether its value is to go into the command.
    pub(crate) settled: Vec<bool>,
    /// The one being asked for, or the next to settle.
    pub(crate) next: usize,
    /// The entries of the list parameter being asked for.
    pub(crate) list: Vec<ExprValue>,
    /// What the list picker on show offers, for a list parameter.
    pub(crate) options: Vec<(String, ExprValue)>,
    /// What the cursor of the map picker on show cycles through.
    pub(crate) candidates: Vec<MapCandidate>,
    pub(crate) pickers: BTreeMap<String, String>,
    pub(crate) from: BTreeMap<String, Binding>,
    /// The screen the effect ran in, `it` there, and the effects that
    /// wait for the command.
    pub(crate) origin: Option<usize>,
    pub(crate) it: Option<ExprValue>,
    pub(crate) rest: Vec<UiEffect>,
    /// The camera as it was before a map picker took the cursor.
    pub(crate) camera: Option<Camera>,
}

/// What the machine asks while an expression of an action runs: `self`
/// is the acting character and the locals are the values settled so
/// far. Everything else is the game's to answer, as that character
/// believes it.
pub(crate) struct ActionHost<'a> {
    game: &'a mut dyn Host,
    actor: &'a ExprValue,
    locals: &'a [ExprValue],
    registry: &'a Registry,
    warnings: &'a mut u64,
}

impl Host for ActionHost<'_> {
    fn subject(&mut self, slot: u16) -> ExprValue {
        if slot == 0 {
            self.actor.clone()
        } else {
            ExprValue::Null
        }
    }

    fn local(&mut self, slot: u16) -> ExprValue {
        self.locals
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn event_field(&mut self, _slot: u16) -> ExprValue {
        ExprValue::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[ExprValue]) -> ExprValue {
        self.game.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> ExprValue {
        self.game.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[ExprValue]) -> ExprValue {
        self.game.call(function, args)
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        // The simulation's streams are not the scene's to draw from: a
        // default that draws comes to null here and is left for the
        // simulation to work out.
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {
        *self.warnings += 1;
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.registry.def_name(id)
    }
}

/// Whether a value is one a parameter of this type can hold. An `int`
/// is accepted where a `real` is wanted (05 §3), and a text where an id
/// is, for the simulation to resolve.
pub(crate) fn fits(ty: &Type, value: &ExprValue) -> bool {
    match ty {
        Type::Any => true,
        Type::Null => value.is_null(),
        Type::Nullable(inner) => value.is_null() || fits(inner, value),
        Type::Bool => value.as_bool().is_some(),
        Type::Int => value.as_int().is_some(),
        Type::Real | Type::Number => value.as_number().is_some(),
        Type::Str => value.as_str().is_some(),
        Type::Id(def_type) => {
            value.as_id().is_some_and(|id| id.ty == *def_type) || value.as_str().is_some()
        }
        Type::Subject(kind) => value
            .as_subject()
            .is_some_and(|handle| handle.kind == *kind),
        Type::List(entry) => value
            .as_list()
            .is_some_and(|items| items.iter().all(|item| fits(entry, item))),
        Type::Map(_) => value.as_map().is_some(),
    }
}

/// The picker a type is asked for with, where the effect names none
/// (12 §9.3).
fn picker_of(ty: &Type, list: bool, registry: &Registry) -> String {
    let kind_name = |ty: &Type| match ty.non_null() {
        Type::Subject(kind) => registry.kind(*kind).map(|kind| kind.spec.name.clone()),
        _ => None,
    };
    if list {
        let entry = match ty.non_null() {
            Type::List(entry) => kind_name(entry),
            _ => None,
        };
        return if entry.as_deref() == Some(KIND_LOC) {
            PICKER_ROUTE.to_owned()
        } else {
            PICKER_MULTI.to_owned()
        };
    }
    match ty.non_null() {
        Type::Subject(_) => match kind_name(ty).as_deref() {
            Some(KIND_ENTITY) => PICKER_ENTITY.to_owned(),
            Some(KIND_LOC) => PICKER_CELL.to_owned(),
            Some(other) => other.to_owned(),
            None => PICKER_LIST.to_owned(),
        },
        Type::Int | Type::Real | Type::Number => PICKER_NUMBER.to_owned(),
        Type::Str => PICKER_TEXT.to_owned(),
        _ => PICKER_LIST.to_owned(),
    }
}

/// An entry of a list to choose from as its label and its value: a map
/// with `value` is shown by its `label`, anything else by `name`.
fn option_of(entry: &ExprValue, name: impl FnOnce(&ExprValue) -> String) -> (String, ExprValue) {
    if let Some(map) = entry.as_map()
        && let Some(value) = map.get("value")
    {
        let label = map
            .get("label")
            .map_or_else(|| value.to_string(), ToString::to_string);
        return (label, value.clone());
    }
    let named = match entry {
        ExprValue::Subject(_) | ExprValue::Id(_) => name(entry),
        _ => String::new(),
    };
    let label = if named.is_empty() {
        entry.to_string()
    } else {
        named
    };
    (label, entry.clone())
}

/// The options a list picker is opened with: maps with `label`, `value`
/// and whether the value is among those `chosen`.
fn options_value(options: &[(String, ExprValue)], chosen: &[ExprValue]) -> ExprValue {
    ExprValue::list(options.iter().map(|(label, value)| {
        ExprValue::map([
            ("label", ExprValue::text(label)),
            ("value", value.clone()),
            ("chosen", ExprValue::Bool(chosen.contains(value))),
        ])
    }))
}

impl<H: SceneHost> Scene<H> {
    /// The loaded definitions, to read an action from while the scene
    /// is changed.
    pub(crate) fn loaded(&self) -> Option<Arc<Defs>> {
        self.ui.as_ref().map(|ui| Arc::clone(ui.defs()))
    }

    /// The qualified id of an action an effect names, seen from the
    /// screen that asks: a bare name is read as `open` reads a screen's
    /// (12 §10.5). `None` if there is no such action, or the scene has
    /// no action definitions at all.
    pub(crate) fn action_id(&self, name: &str, origin: Option<usize>) -> Option<String> {
        let namespace = origin
            .and_then(|at| self.screens.get(at))
            .map_or("", |screen| namespace_of(screen.id()));
        self.ui
            .as_ref()
            .and_then(|ui| ui.resolve::<ActionDef>(name, namespace))
    }

    /// Evaluates fields of an action for an actor, with the action's
    /// locals as given: in that character's mind, through the host.
    pub(crate) fn eval_action<R>(
        &mut self,
        defs: &Defs,
        actor: &ExprValue,
        locals: &[ExprValue],
        f: impl FnOnce(&mut FieldEvaluator<'_, ActionHost<'_>>) -> R,
    ) -> R {
        let Scene {
            host,
            runner,
            warnings,
            ..
        } = self;
        let mut action_host = ActionHost {
            game: host.reader(),
            actor,
            locals,
            registry: defs.exprs().registry(),
            warnings,
        };
        f(&mut runner.over(defs.exprs(), &mut action_host))
    }

    /// What the reader calls a value, as the host says.
    pub(crate) fn name_of(&mut self, value: &ExprValue) -> String {
        self.with_reader(|reader, _| reader.name_of(value))
            .unwrap_or_default()
    }

    /// Drops the command being put together, if one is, and gives the
    /// camera back. Returns the action's id.
    pub(crate) fn abandon_command(&mut self) -> Option<String> {
        let collecting = self.collecting.take()?;
        self.restore_camera(collecting.camera);
        Some(collecting.action)
    }

    /// Puts the camera back as it was before a map picker took it.
    pub(crate) fn restore_camera(&mut self, saved: Option<Camera>) {
        if let (Some(saved), Some(world)) = (saved, &mut self.world) {
            *world.camera_mut() = saved;
            world.show_line(false);
        }
    }
}

impl<H> Scene<H>
where
    H: SceneHost,
    H::Command: From<UiCommand>,
    H::Refusal: core::fmt::Display,
{
    /// Begins an `act` that asks: for the action, if it is to be picked,
    /// and then for its parameters.
    pub(crate) fn begin_act(
        &mut self,
        command: &CommandEffect,
        it: Option<ExprValue>,
        origin: Option<usize>,
        rest: Vec<UiEffect>,
        out: &mut Vec<UiOutcome>,
    ) {
        let failed = |reason: String| UiOutcome::Failed {
            effect: "act",
            reason,
        };
        if self.collecting.is_some() {
            out.push(failed(
                "another command is still being put together".to_owned(),
            ));
            return;
        }
        let mode = match command.collect {
            // An action that is picked and given no parameters takes
            // its defaults, and is asked for what has none.
            Collect::Given => Collect::Needed,
            asked => asked,
        };
        self.collecting = Some(Collecting {
            action: String::new(),
            def: None,
            mode,
            target: Target::Act,
            actor: self.host.view(),
            locals: Vec::new(),
            asks: Vec::new(),
            settled: Vec::new(),
            next: 0,
            list: Vec::new(),
            options: Vec::new(),
            candidates: Vec::new(),
            pickers: command.pickers.clone(),
            from: command.from.clone(),
            origin,
            it: it.clone(),
            rest,
            camera: None,
        });
        let Some(choose) = &command.choose else {
            self.start_collecting(&command.name, out);
            return;
        };
        // Which action: the list picker, over what the expression gives.
        let options = self.eval(origin, it.as_ref(), |reader, scope| {
            let listed = reader.bound(choose, scope);
            let entries = listed.as_list().unwrap_or(&[]).to_vec();
            let options: Vec<(String, ExprValue)> = entries
                .iter()
                .map(|entry| option_of(entry, |value| reader.name_of(value)))
                .collect();
            options
        });
        let with = BTreeMap::from([("options".to_owned(), options_value(&options, &[]))]);
        match self.open_role_with(ROLE_PICK_LIST, with, Some(Pending::bare(Wait::Action))) {
            Ok(id) => out.push(UiOutcome::Opened(id)),
            Err(reason) => {
                self.collecting = None;
                out.push(failed(reason));
            }
        }
    }

    /// Begins the reaction a decision prompt chose: its parameters are
    /// asked for as an action's are, and the command is `react`.
    pub(crate) fn begin_reaction(
        &mut self,
        action: &str,
        entity: ExprValue,
        window: ExprValue,
        out: &mut Vec<UiOutcome>,
    ) {
        self.collecting = Some(Collecting {
            action: String::new(),
            def: None,
            mode: Collect::Needed,
            target: Target::React {
                entity: entity.clone(),
                window,
            },
            actor: entity,
            locals: Vec::new(),
            asks: Vec::new(),
            settled: Vec::new(),
            next: 0,
            list: Vec::new(),
            options: Vec::new(),
            candidates: Vec::new(),
            pickers: BTreeMap::new(),
            from: BTreeMap::new(),
            origin: None,
            it: None,
            rest: Vec::new(),
            camera: None,
        });
        if self.action_id(action, None).is_some() {
            self.start_collecting(action, out);
        } else {
            // An action the scene cannot read is the simulation's to
            // complete from its defaults.
            if let Some(collecting) = &mut self.collecting {
                collecting.action = action.to_owned();
            }
            self.finish_collecting(out);
        }
    }

    /// The list picker that asked which action answered.
    pub(crate) fn action_answered(&mut self, answer: ExprValue, out: &mut Vec<UiOutcome>) {
        self.pending.pop();
        if let Some(closed) = self.pop_screen() {
            out.push(UiOutcome::Closed(closed.id().to_owned()));
        }
        let name = match &answer {
            ExprValue::Str(name) => Some(name.to_string()),
            ExprValue::Id(id) => self
                .loaded()
                .and_then(|defs| defs.exprs().registry().def_name(*id).map(str::to_owned)),
            _ => None,
        };
        match name {
            Some(name) => self.start_collecting(&name, out),
            None => {
                self.collecting = None;
                out.push(UiOutcome::Cancelled {
                    command: String::new(),
                });
            }
        }
    }

    /// Reads the action's parameter list and asks for the first that is
    /// to be asked for.
    fn start_collecting(&mut self, name: &str, out: &mut Vec<UiOutcome>) {
        let origin = self.collecting.as_ref().and_then(|c| c.origin);
        let found = self.action_id(name, origin).and_then(|id| {
            let defs = self.loaded()?;
            let def = defs.try_registry::<ActionDef>()?.lookup(&id)?;
            Some((id, def, defs))
        });
        let Some((id, def_id, defs)) = found else {
            // An action the scene has no definition of to read: the
            // command is handed over as it was written, marked as one
            // whose parameters were to be asked for, and the host
            // completes it or refuses it with its reason.
            let Some(collecting) = self.collecting.take() else {
                return;
            };
            let command = UiCommand {
                kind: CommandKind::Act,
                name: name.to_owned(),
                args: BTreeMap::new(),
                collect: collecting.mode,
            };
            if self.submit_command(command, out) && !collecting.rest.is_empty() {
                let origin = collecting.origin.filter(|at| *at < self.screens.len());
                self.run_list(&collecting.rest, collecting.it, origin, out);
            }
            return;
        };
        let def = defs.get::<ActionDef>(def_id);
        let entity = defs
            .exprs()
            .registry()
            .kind_id(KIND_ENTITY)
            .map_or(Type::Any, Type::Subject);
        let mut asks = Vec::new();
        // The first role is whoever starts the action; the others are
        // given by name, as parameters are (03 §8).
        for role in def.roles().iter().skip(1) {
            asks.push(Ask {
                name: role.name.value.clone(),
                slot: usize::from(role.slot()),
                ty: entity.clone(),
                param: None,
                list: None,
            });
        }
        for (index, param) in def.params().iter().enumerate() {
            asks.push(Ask {
                name: param.name.value.clone(),
                slot: usize::from(param.slot()),
                ty: param.ty().clone(),
                param: Some(index),
                list: param.list(),
            });
        }
        if let Some(collecting) = &mut self.collecting {
            collecting.action = id;
            collecting.def = Some(def_id);
            collecting.locals = vec![ExprValue::Null; def.locals().len()];
            // A reaction's window is the first of its locals.
            if let (Target::React { window, .. }, Some(slot)) = (
                &collecting.target,
                def.local_slot(clash_sim::action::WINDOW_LOCAL),
            ) {
                collecting.locals[usize::from(slot)] = window.clone();
            }
            // A joint action's first role is the actor.
            if let Some(first) = def.roles().first() {
                collecting.locals[usize::from(first.slot())] = collecting.actor.clone();
            }
            collecting.settled = vec![false; asks.len()];
            collecting.asks = asks;
            collecting.next = 0;
        }
        self.advance_collecting(out);
    }

    /// Settles parameters from their defaults until one is to be asked
    /// for, and opens its picker; or, with none left, submits.
    fn advance_collecting(&mut self, out: &mut Vec<UiOutcome>) {
        loop {
            let Some(collecting) = &self.collecting else {
                return;
            };
            let (Some(def_id), Some(defs)) = (collecting.def, self.loaded()) else {
                return;
            };
            let Some(ask) = collecting.asks.get(collecting.next).cloned() else {
                self.finish_collecting(out);
                return;
            };
            let def = defs.get::<ActionDef>(def_id);
            let default = ask
                .param
                .and_then(|index| def.params().get(index))
                .and_then(|param| param.default.as_ref());
            let (actor, locals) = (collecting.actor.clone(), collecting.locals.clone());
            let prefill = match default {
                Some(default) => {
                    self.eval_action(&defs, &actor, &locals, |eval| default.value(eval))
                }
                None => ExprValue::Null,
            };
            let Some(collecting) = &mut self.collecting else {
                return;
            };
            let asked = match collecting.mode {
                Collect::All => true,
                _ => default.is_none(),
            };
            if !asked {
                // From its default. One that comes to null here is left
                // out of the command, for the simulation to work out.
                collecting.settled[collecting.next] = !prefill.is_null();
                collecting.locals[ask.slot] = prefill;
                collecting.next += 1;
                continue;
            }
            collecting.locals[ask.slot] = prefill.clone();
            match self.open_picker(&ask, def, prefill) {
                Ok(id) => out.push(UiOutcome::Opened(id)),
                Err(reason) => {
                    let command = self.abandon_command().unwrap_or_default();
                    out.push(UiOutcome::Failed {
                        effect: "act",
                        reason: format!("`{command}` cannot be asked for `{}`: {reason}", ask.name),
                    });
                }
            }
            return;
        }
    }

    /// Opens the picker for one parameter, filled in with `prefill`.
    fn open_picker(
        &mut self,
        ask: &Ask,
        def: &ActionDef,
        prefill: ExprValue,
    ) -> Result<String, String> {
        let Some(defs) = self.loaded() else {
            return Err("the scene has no definitions".to_owned());
        };
        let registry = Arc::clone(defs.exprs().registry());
        let Some(collecting) = &self.collecting else {
            return Err("no command is being put together".to_owned());
        };
        let (actor, action) = (collecting.actor.clone(), collecting.action.clone());
        let (origin, it) = (collecting.origin, collecting.it.clone());
        let named = collecting.pickers.get(&ask.name).cloned();
        let from = collecting.from.get(&ask.name).cloned();
        let locals = collecting.locals.clone();
        let picker = named.unwrap_or_else(|| {
            let by_type = picker_of(&ask.ty, ask.list.is_some(), &registry);
            // A number or a text that the effect lists the values of is
            // chosen from the list.
            match (by_type.as_str(), &from) {
                (PICKER_NUMBER | PICKER_TEXT, Some(_)) => PICKER_LIST.to_owned(),
                _ => by_type,
            }
        });
        // The role's screen; a kind with no picker of its own, and a
        // route where there is no route picker, fall back.
        let role = [
            picker.as_str(),
            if picker == PICKER_ROUTE {
                PICKER_CELL
            } else {
                PICKER_LIST
            },
        ]
        .iter()
        .map(|name| format!("{PICKER_ROLE_PREFIX}{name}"))
        .find(|role| self.roles.contains_key(role))
        .unwrap_or_else(|| format!("{PICKER_ROLE_PREFIX}{picker}"));

        // What may be chosen: what the effect lists, or else what the
        // host says the character believes there is.
        let listed = match &from {
            Some(from) => Some(self.eval(origin, it.as_ref(), |reader, scope| {
                reader.bound(from, scope).as_list().unwrap_or(&[]).to_vec()
            })),
            None => self.host.candidates(&ParamAsk {
                actor: &actor,
                action: &action,
                param: &ask.name,
                ty: ask.entry_type(),
                picker: &picker,
            }),
        };
        let mut options: Vec<(String, ExprValue)> = Vec::new();
        for entry in listed.iter().flatten() {
            let option = option_of(entry, |value| self.name_of(value));
            options.push(option);
        }
        if listed.is_none() && matches!(ask.entry_type().non_null(), Type::Bool) {
            options = [true, false]
                .map(|flag| (flag.to_string(), ExprValue::Bool(flag)))
                .to_vec();
        }

        let shown_name = match &def.name {
            Some(name) => self.eval_action(&defs, &actor, &locals, |eval| name.render(eval)),
            None => action.clone(),
        };
        let chosen: Vec<ExprValue> = match (ask.list, prefill.as_list()) {
            (Some(_), Some(entries)) => entries.to_vec(),
            _ => Vec::new(),
        };
        let mut with = BTreeMap::new();
        with.insert("action".to_owned(), ExprValue::text(&shown_name));
        with.insert("param".to_owned(), ExprValue::text(&ask.name));
        with.insert(
            "title".to_owned(),
            ExprValue::text(format!("{shown_name}: {}", ask.name)),
        );
        with.insert("value".to_owned(), prefill.clone());
        with.insert("options".to_owned(), options_value(&options, &chosen));
        if let Some(bounds) = ask.list {
            with.insert("min".to_owned(), ExprValue::Int(i64::from(bounds.min)));
            if let Some(max) = bounds.max {
                with.insert("max".to_owned(), ExprValue::Int(i64::from(max)));
            }
        }
        let id = self.open_role_with(&role, with, Some(Pending::bare(Wait::Param)))?;

        // What the picker's own widgets start from.
        let whole = matches!(ask.ty.non_null(), Type::Int);
        if let Some(screen) = self.screens.last_mut() {
            let start = match (&prefill, ask.ty.non_null()) {
                (ExprValue::Null, Type::Real | Type::Number) if !whole => {
                    Some(clash_core::Value::Real(clash_core::Real::ZERO))
                }
                (ExprValue::Int(n), Type::Real | Type::Number) => {
                    Some(clash_core::Value::Real(clash_core::Real::of(*n as f64)))
                }
                (value, _) => to_var(value),
            };
            if let Some(start) = start {
                screen.set_var("value", start);
            }
            // The row of the value it is filled in with.
            if let Some(row) = options.iter().position(|(_, value)| *value == prefill) {
                screen.start_on_row(row);
            }
        }

        // A picker with a map takes the cursor.
        let has_map = self
            .screens
            .last()
            .is_some_and(|screen| screen.has_map_view());
        let mut candidates = Vec::new();
        let mut saved = None;
        if has_map && let Some(world) = &self.world {
            saved = Some(*world.camera());
            let grid = world.camera().grid();
            let subject = world.camera().subject();
            let entity = picker == PICKER_ENTITY;
            match &listed {
                // What was listed, where the party perceives it.
                Some(entries) => {
                    for (_, value) in &options {
                        let sprite = self.host.sprite_of(value);
                        let at = sprite
                            .and_then(|id| self.world.as_ref()?.model().sprite(id))
                            .filter(|sprite| sprite.loc.grid == grid)
                            .map(|sprite| sprite.loc.cell);
                        if let Some(cell) = at {
                            candidates.push(MapCandidate {
                                value: value.clone(),
                                cell,
                            });
                        }
                    }
                    let _ = entries;
                }
                // What the view model holds: perceived, and nothing else.
                None if entity => {
                    let sprites: Vec<_> = world
                        .model()
                        .sprites()
                        .filter(|(_, sprite)| sprite.loc.grid == grid && !sprite.suspected)
                        .map(|(id, sprite)| (id, sprite.loc.cell))
                        .collect();
                    for (id, cell) in sprites {
                        let value = self.host.sprite_value(id);
                        if fits(ask.entry_type(), &value) && value != actor {
                            candidates.push(MapCandidate { value, cell });
                        }
                    }
                }
                None => {}
            }
            let far = |cell: &CellPos| {
                let (dx, dy, dz) = (
                    i64::from(cell.x) - i64::from(subject.x),
                    i64::from(cell.y) - i64::from(subject.y),
                    i64::from(cell.z) - i64::from(subject.z),
                );
                dx * dx + dy * dy + dz * dz
            };
            candidates.sort_by_key(|candidate| far(&candidate.cell));
            // The cursor starts on what is filled in, or else on the
            // nearest of what could be chosen, or else on the actor.
            let start = candidates
                .iter()
                .find(|candidate| candidate.value == prefill)
                .or(candidates.first().filter(|_| entity))
                .map(|candidate| candidate.cell)
                .or_else(|| {
                    let at = self.host.cell_of(&prefill)?;
                    (at.grid == grid).then_some(at.cell)
                });
            if let Some(world) = &mut self.world {
                let camera = world.camera_mut();
                match start {
                    Some(cell) => {
                        camera.shift_storey(cell.z.saturating_sub(camera.storey()));
                        camera.set_cursor(cell.x, cell.y);
                    }
                    None => camera.move_cursor(0, 0),
                }
                world.show_line(true);
            }
        }
        if let Some(collecting) = &mut self.collecting {
            collecting.list = chosen;
            collecting.options = options;
            collecting.candidates = candidates;
            if collecting.camera.is_none() {
                collecting.camera = saved;
            }
        }
        self.push_status();
        Ok(id)
    }

    /// Closes the picker on top without abandoning the command.
    fn close_picker(&mut self, out: &mut Vec<UiOutcome>) {
        self.pending.pop();
        if let Some(closed) = self.pop_screen() {
            out.push(UiOutcome::Closed(closed.id().to_owned()));
        }
        let saved = self.collecting.as_mut().and_then(|c| c.camera.take());
        self.restore_camera(saved);
    }

    /// A picker answered: the value is taken if the parameter can hold
    /// it, and the next parameter is asked for. For a list parameter the
    /// value is one more entry and the picker stays.
    pub(crate) fn param_answered(&mut self, answer: ExprValue, out: &mut Vec<UiOutcome>) {
        let Some(collecting) = &mut self.collecting else {
            return;
        };
        let Some(ask) = collecting.asks.get(collecting.next).cloned() else {
            return;
        };
        let refused = |what: &str| UiOutcome::Failed {
            effect: "answer",
            reason: format!("`{}` cannot be {what}", ask.name),
        };
        if !fits(ask.entry_type(), &answer) {
            let what = if answer.is_null() {
                "nothing".to_owned()
            } else {
                format!("{answer}")
            };
            out.push(refused(&what));
            return;
        }
        if let Some(bounds) = ask.list {
            if bounds
                .max
                .is_some_and(|max| collecting.list.len() >= max as usize)
            {
                out.push(refused("any longer"));
                return;
            }
            collecting.list.push(answer);
            self.list_changed();
            return;
        }
        collecting.locals[ask.slot] = answer;
        collecting.settled[collecting.next] = true;
        collecting.next += 1;
        self.close_picker(out);
        self.advance_collecting(out);
    }

    /// The `picker` effect: works on the list being built.
    pub(crate) fn picker_effect(
        &mut self,
        effect: &PickerEffect,
        value: ExprValue,
        out: &mut Vec<UiOutcome>,
    ) {
        let failed = |reason: String| UiOutcome::Failed {
            effect: "picker",
            reason,
        };
        let asking = matches!(
            self.pending.last().and_then(Option::as_ref),
            Some(Pending {
                wait: Wait::Param,
                ..
            })
        );
        let ask = self
            .collecting
            .as_ref()
            .filter(|_| asking)
            .and_then(|collecting| collecting.asks.get(collecting.next).cloned());
        let (Some(ask), Some(collecting)) = (ask, &mut self.collecting) else {
            out.push(failed("no picker is building a list".to_owned()));
            return;
        };
        let Some(bounds) = ask.list else {
            out.push(failed(format!("`{}` is not a list", ask.name)));
            return;
        };
        match effect {
            PickerEffect::Undo => {
                collecting.list.pop();
                self.list_changed();
            }
            PickerEffect::Toggle(_) => {
                if let Some(at) = collecting.list.iter().position(|entry| *entry == value) {
                    collecting.list.remove(at);
                } else if !fits(ask.entry_type(), &value) {
                    out.push(failed(format!("`{}` cannot hold {value}", ask.name)));
                    return;
                } else if bounds
                    .max
                    .is_some_and(|max| collecting.list.len() >= max as usize)
                {
                    out.push(failed(format!(
                        "`{}` takes no more than {}",
                        ask.name,
                        bounds.max.unwrap_or(0)
                    )));
                    return;
                } else {
                    collecting.list.push(value);
                }
                self.list_changed();
            }
            PickerEffect::Done => {
                if !bounds.allows(collecting.list.len()) {
                    out.push(failed(format!(
                        "`{}` needs at least {}, and has {}",
                        ask.name,
                        bounds.min,
                        collecting.list.len()
                    )));
                    return;
                }
                collecting.locals[ask.slot] = ExprValue::list(collecting.list.drain(..));
                collecting.settled[collecting.next] = true;
                collecting.next += 1;
                self.close_picker(out);
                self.advance_collecting(out);
            }
        }
    }

    /// The list being built changed: the picker on show is told.
    fn list_changed(&mut self) {
        let Some(collecting) = &self.collecting else {
            return;
        };
        let options = options_value(&collecting.options, &collecting.list);
        let entries = ExprValue::list(collecting.list.iter().cloned());
        if let Some(screen) = self.screens.last_mut() {
            screen.set_param("options", options);
            screen.set_param("value", entries);
        }
        self.stale = true;
        self.push_status();
    }

    /// Every parameter is settled: the command is made and submitted,
    /// and what waited for it runs.
    fn finish_collecting(&mut self, out: &mut Vec<UiOutcome>) {
        let Some(collecting) = self.collecting.take() else {
            return;
        };
        self.restore_camera(collecting.camera);
        let mut params = BTreeMap::new();
        for (ask, settled) in collecting.asks.iter().zip(&collecting.settled) {
            if *settled {
                params.insert(ask.name.clone(), collecting.locals[ask.slot].clone());
            }
        }
        let command = match collecting.target {
            Target::Act => UiCommand {
                kind: CommandKind::Act,
                name: collecting.action,
                args: params,
                collect: Collect::Given,
            },
            Target::React { entity, window } => UiCommand {
                kind: CommandKind::Command,
                name: COMMAND_REACT.to_owned(),
                args: BTreeMap::from([
                    ("entity".to_owned(), entity),
                    ("window".to_owned(), window),
                    ("action".to_owned(), ExprValue::text(&collecting.action)),
                    ("params".to_owned(), ExprValue::map(params)),
                ]),
                collect: Collect::Given,
            },
        };
        if self.submit_command(command, out) && !collecting.rest.is_empty() {
            let origin = collecting.origin.filter(|at| *at < self.screens.len());
            self.run_list(&collecting.rest, collecting.it, origin, out);
        }
    }
}
