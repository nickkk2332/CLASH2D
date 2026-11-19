//! [`FakeHost`]: a small deterministic host for tests.
//!
//! It is a counter world: named whole numbers called cells, a clock,
//! timers that add to a cell at intervals, and events for whatever
//! happens. It implements [`ConsoleHost`] in full, save and load
//! included, so that the console and the test-script runner can be
//! tested before the simulation exists, and so that later crates can
//! test their own use of the console against something small. None of
//! it is engine content: it exists only here.
//!
//! # Commands
//!
//! [`FakeHost::table`] is the built-in commands of 14 §3.3 and these:
//!
//! | Command | Does | Refused for |
//! |---------|------|-------------|
//! | `new <name> [value=0]` | Makes a cell and gives it back, to be named with `as=` | `cell_exists` |
//! | `set <cell> <value>` | Sets a cell | `cell_locked`, `no_such_cell` |
//! | `add <cell> [amount=1]` | Adds to a cell | `cell_locked`, `out_of_range`, `no_such_cell` |
//! | `lock <cell>` | Makes a cell refuse `set` and `add` | `no_such_cell` |
//! | `focus <cell>` | Makes a cell `@self` | `no_such_cell` |
//! | `timer <cell> <every> [times]` | Adds one to a cell at each interval, for ever or that many times | `bad_interval`, `no_such_cell` |
//!
//! Of the built-in commands it carries out `use` (any profile), `gen
//! flat size=N` (cells `c0` to `cN-1`), `step`, `check`, `inspect` and
//! `do`, besides those the console carries out through the trait. A
//! command that comes from a verb is counted and reported as a `verb`
//! event with its arguments, which is all a test of the console needs
//! of it.
//!
//! # References
//!
//! `@self` is the cell in focus and `@sel` the cell last changed.
//! `@here` is the first cell. `@c:x,0,0` and `@e:x` are the cell made
//! `x`-th, counting from zero. `@cell:name` and a plain `@name` are the
//! cell of that name.
//!
//! # Expressions
//!
//! The expression language of 05 with one kind, `cell`, whose
//! properties are `value` (int), `name` (string) and `locked` (bool);
//! `self`, the cell in focus; and the functions `cell(name)`, `clock()`
//! (seconds) and `cells()` (how many).
//!
//! # Events and traces
//!
//! `cell_made` and `cell_changed` (fields `cell`, `name`, `value`),
//! `timer_fired` (the same), `verb` (`name` and the verb's arguments)
//! and `effect` (`verb`). A timer that fires also leaves a `timer` trace
//! record with `cell` and `value`.
//!
//! # Deliberate flaws
//!
//! [`Flaws`] switches on three defects, one for each of the runner's
//! modes to catch.

use std::cell::Cell;
use std::rc::Rc;

use clash_core::{Dur, Hash128, Rng, StateHash, StateHasher, Tick, codec};
use clash_expr::{
    Compiled, Context, DefRef, FunctionId, FunctionSpec, Handle, KindId, KindSpec, Machine,
    PropertyId, PropertySpec, Registry, RuntimeWarning, Type, Value, compile,
};
use serde::{Deserialize, Serialize};

use super::command::{
    ArgValue, BoundCommand, CommandSource, CommandSpec, CommandTable, ParamSpec, ParamType,
};
use super::host::{ConsoleHost, Expression, HostError, HostResult, Output, Record, Refusal};
use super::syntax::Reference;

/// The defects a [`FakeHost`] can be given, so that a test can show the
/// runner catching each.
#[derive(Clone, Debug, Default)]
pub struct Flaws {
    /// Saving leaves the timers out. Caught by the save-and-load mode
    /// (rule R7).
    pub save_forgets_timers: bool,
    /// A counter shared by every host that is given it, as a static
    /// would be: each new cell starts that much higher, and the counter
    /// goes up. Caught by the twice mode.
    pub leak: Option<Rc<Cell<i64>>>,
    /// A timer adds the thread count instead of one. Caught by the
    /// threads mode (rule R1).
    pub threads_matter: bool,
}

/// A named whole number.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct CellState {
    name: String,
    value: i64,
    locked: bool,
}

/// Something that adds to a cell at intervals.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct Timer {
    cell: u32,
    /// The interval, in ticks.
    every: i64,
    /// When it next fires.
    next: i64,
    /// How many more times it fires, if it stops.
    left: Option<u32>,
}

/// Everything a fake host saves and hashes.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct State {
    profile: String,
    seed: u64,
    clock: i64,
    cells: Vec<CellState>,
    timers: Vec<Timer>,
    /// How many verbs and effects have been run.
    calls: u64,
    focus: Option<u32>,
    selected: Option<u32>,
}

impl State {
    fn new(profile: &str, seed: u64) -> State {
        State {
            profile: profile.to_owned(),
            seed,
            clock: 0,
            cells: Vec::new(),
            timers: Vec::new(),
            calls: 0,
            focus: None,
            selected: None,
        }
    }
}

impl StateHash for State {
    fn state_hash(&self, hasher: &mut StateHasher) {
        let option = |hasher: &mut StateHasher, value: Option<u32>| {
            hasher.write_bool(value.is_some());
            hasher.write_u32(value.unwrap_or(0));
        };
        hasher.write_str(&self.profile);
        hasher.write_u64(self.seed);
        hasher.write_i64(self.clock);
        hasher.write_len(self.cells.len());
        for cell in &self.cells {
            hasher.write_str(&cell.name);
            hasher.write_i64(cell.value);
            hasher.write_bool(cell.locked);
        }
        hasher.write_len(self.timers.len());
        for timer in &self.timers {
            hasher.write_u32(timer.cell);
            hasher.write_i64(timer.every);
            hasher.write_i64(timer.next);
            option(hasher, timer.left);
        }
        hasher.write_u64(self.calls);
        option(hasher, self.focus);
        option(hasher, self.selected);
    }
}

/// What the fake host registered with the expression language.
#[derive(Clone, Copy)]
struct Ids {
    cell: KindId,
    value: PropertyId,
    name: PropertyId,
    locked: PropertyId,
    cell_of: FunctionId,
    clock: FunctionId,
    cells: FunctionId,
}

/// The profile a fake host starts in before any `use`.
const DEFAULT_PROFILE: &str = "fake";

/// A small deterministic host: see the [module](self).
///
/// ```
/// use clash_sim::console::testing::FakeHost;
/// use clash_sim::console::{ConsoleHost, HostKind, Session};
///
/// let table = FakeHost::table();
/// let mut host = FakeHost::new();
/// let mut session = Session::new(HostKind::File);
/// for line in ["new a as=a", "timer @a every=1s", "advance 3s"] {
///     session.run_line(&table, &mut host, line).unwrap();
/// }
/// assert_eq!(host.cell("a"), Some(3));
///
/// // Saved and loaded into another host, it is the same state.
/// let saved = host.save().unwrap();
/// let mut other = FakeHost::new();
/// other.load(&saved).unwrap();
/// assert_eq!(other.state_hash(), host.state_hash());
/// ```
pub struct FakeHost {
    state: State,
    records: Vec<Record>,
    last_refusal: Option<Refusal>,
    threads: u32,
    flaws: Flaws,
    registry: Registry,
    ids: Ids,
    machine: Machine,
}

impl Default for FakeHost {
    fn default() -> FakeHost {
        FakeHost::new()
    }
}

impl FakeHost {
    /// A host with no cells and no flaws.
    pub fn new() -> FakeHost {
        FakeHost::with_flaws(Flaws::default())
    }

    /// A host with these flaws.
    ///
    /// # Panics
    ///
    /// Never in practice: only if the fixed names this host registers
    /// with the expression language were refused.
    pub fn with_flaws(flaws: Flaws) -> FakeHost {
        const FIXED: &str = "the fake host's fixed registrations are valid";
        let mut registry = Registry::with_builtins();
        let cell = registry
            .add_kind(KindSpec::new("cell").doc("A named whole number."))
            .expect(FIXED);
        let mut property = |name: &str, ty: Type, doc: &str| {
            registry
                .add_property(PropertySpec::new(cell, name, ty).doc(doc))
                .expect(FIXED)
        };
        let value = property("value", Type::Int, "The number.");
        let name = property("name", Type::Str, "The cell's name.");
        let locked = property("locked", Type::Bool, "Whether it refuses changes.");
        let cell_of = registry
            .add_function(
                FunctionSpec::new("cell", Type::Subject(cell).nullable())
                    .param("name", Type::Str, "The cell's name.")
                    .doc("The cell of that name."),
            )
            .expect(FIXED);
        let clock = registry
            .add_function(FunctionSpec::new("clock", Type::Real).doc("The time, in seconds."))
            .expect(FIXED);
        let cells = registry
            .add_function(FunctionSpec::new("cells", Type::Int).doc("How many cells there are."))
            .expect(FIXED);
        FakeHost {
            state: State::new(DEFAULT_PROFILE, 0),
            records: Vec::new(),
            last_refusal: None,
            threads: 1,
            flaws,
            registry,
            ids: Ids {
                cell,
                value,
                name,
                locked,
                cell_of,
                clock,
                cells,
            },
            machine: Machine::new(),
        }
    }

    /// The commands a fake host has: the built-in ones and its own.
    ///
    /// # Panics
    ///
    /// Never in practice: only if one of its own fixed command names
    /// were a built-in command's.
    pub fn table() -> CommandTable {
        use ParamSpec as P;
        use ParamType as T;
        let cell = || P::required("cell", T::Subject, "The cell.");
        let int = |value: i64| ArgValue::Value(Value::Int(value));
        let mut table = CommandTable::builtin();
        for spec in [
            CommandSpec::new("new")
                .doc("Makes a cell and gives it back, to be named with as=.")
                .param(P::required("name", T::Word, "The cell's name."))
                .param(P::optional("value", T::Int, "What it starts at.").default(int(0))),
            CommandSpec::new("set")
                .doc("Sets a cell.")
                .param(cell())
                .param(P::required("value", T::Int, "The new value.")),
            CommandSpec::new("add")
                .doc("Adds to a cell.")
                .param(cell())
                .param(P::optional("amount", T::Int, "How much.").default(int(1))),
            CommandSpec::new("lock")
                .doc("Makes a cell refuse set and add.")
                .param(cell()),
            CommandSpec::new("focus")
                .doc("Makes a cell @self.")
                .param(cell()),
            CommandSpec::new("timer")
                .doc("Adds one to a cell at each interval.")
                .param(cell())
                .param(P::required("every", T::Duration, "The interval."))
                .param(P::optional(
                    "times",
                    T::Int,
                    "How many times. For ever if left out.",
                )),
        ] {
            table
                .add(spec.changes())
                .expect("the fake host's command names are its own");
        }
        table
    }

    /// The commands a fake host has, and one for each verb of a
    /// registry.
    pub fn table_with_verbs(registry: &Registry) -> CommandTable {
        let mut table = FakeHost::table();
        table.add_verbs(registry);
        table
    }

    /// The value of the cell of this name.
    pub fn cell(&self, name: &str) -> Option<i64> {
        self.state
            .cells
            .iter()
            .find(|cell| cell.name == name)
            .map(|cell| cell.value)
    }

    /// How many verbs and effects have been run.
    pub const fn calls(&self) -> u64 {
        self.state.calls
    }

    /// How many threads the host was last told to use.
    pub const fn threads(&self) -> u32 {
        self.threads
    }

    fn handle(&self, index: usize) -> Value {
        Value::Subject(Handle::new(self.ids.cell, index as u64))
    }

    /// The index of the cell a value refers to.
    fn index_of(&self, value: &Value) -> Option<usize> {
        let handle = value.as_subject()?;
        let index = usize::try_from(handle.payload).ok()?;
        (handle.kind == self.ids.cell && index < self.state.cells.len()).then_some(index)
    }

    fn no_such_cell(what: impl core::fmt::Display) -> HostError {
        HostError::refused("no_such_cell", format!("there is no cell {what}"))
    }

    fn now_tick(&self) -> Tick {
        Tick(self.state.clock)
    }

    /// The cell a command's argument names.
    fn cell_arg(&mut self, command: &BoundCommand, name: &str) -> HostResult<usize> {
        let value = match command.arg(name) {
            Some(ArgValue::Value(value)) => value.clone(),
            Some(ArgValue::Expr(expression)) => {
                let subject = subject_of(command);
                self.eval(expression, subject.as_ref())?
            }
            _ => return Err(HostError::failed(format!("`{name}` has no value"))),
        };
        self.index_of(&value)
            .ok_or_else(|| FakeHost::no_such_cell(format_args!("`{value}`")))
    }

    fn changed(&mut self, index: usize, event: &str) {
        let cell = &self.state.cells[index];
        let record = Record::event(event, self.now_tick())
            .with("cell", self.handle(index))
            .with("name", cell.name.as_str())
            .with("value", cell.value);
        self.records.push(record);
    }

    /// Checks that a cell may be changed.
    fn unlocked(&self, index: usize) -> HostResult<()> {
        let cell = &self.state.cells[index];
        if cell.locked {
            Err(
                Refusal::new("cell_locked", format!("the cell `{}` is locked", cell.name))
                    .with("cell", self.handle(index))
                    .into(),
            )
        } else {
            Ok(())
        }
    }

    /// Fires the timer that is due next, if one is due by `limit`.
    fn fire_next(&mut self, limit: i64) -> bool {
        let Some(at) = (0..self.state.timers.len())
            .min_by_key(|index| (self.state.timers[*index].next, *index))
            .filter(|index| self.state.timers[*index].next <= limit)
        else {
            return false;
        };
        let timer = &mut self.state.timers[at];
        let index = timer.cell as usize;
        self.state.clock = timer.next;
        timer.next = timer.next.saturating_add(timer.every);
        let spent = match &mut timer.left {
            Some(left) => {
                *left = left.saturating_sub(1);
                *left == 0
            }
            None => false,
        };
        if spent {
            self.state.timers.remove(at);
        }
        let step = if self.flaws.threads_matter {
            i64::from(self.threads)
        } else {
            1
        };
        if let Some(cell) = self.state.cells.get_mut(index)
            && !cell.locked
        {
            cell.value = cell.value.saturating_add(step);
            let value = cell.value;
            self.changed(index, "timer_fired");
            self.records.push(
                Record::trace("timer", self.now_tick())
                    .with("cell", self.handle(index))
                    .with("value", value),
            );
        }
        true
    }

    /// Compiles an expression for this host: the code, what `self` is,
    /// and the values of its locals.
    fn compile(
        &self,
        expression: &Expression,
        subject: Option<&Value>,
    ) -> HostResult<(Compiled, Value, Vec<Value>)> {
        let mut context = Context::new().subject("self", Type::Subject(self.ids.cell).nullable());
        let mut locals = Vec::new();
        for local in &expression.locals {
            let ty = match &local.value {
                Value::Subject(handle) => Type::Subject(handle.kind),
                Value::Int(_) => Type::Int,
                Value::Real(_) => Type::Real,
                Value::Bool(_) => Type::Bool,
                Value::Str(_) => Type::Str,
                _ => Type::Any,
            };
            context.add_local(local.name.as_str(), ty);
            locals.push(local.value.clone());
        }
        let own = subject
            .cloned()
            .or_else(|| self.state.focus.map(|index| self.handle(index as usize)))
            .unwrap_or(Value::Null);
        compile(&expression.source, &self.registry, &context, &Type::Any)
            .into_result()
            .map(|code| (code, own, locals))
            .map_err(|errors| {
                HostError::failed(format!(
                    "`{}` does not compile: {}",
                    expression.written,
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                ))
            })
    }

    fn run_code(&mut self, code: &Compiled, own: &Value, locals: &[Value]) -> Value {
        let mut reader = Reader {
            state: &self.state,
            ids: self.ids,
            own,
            locals,
        };
        self.machine.eval(code, &mut reader)
    }

    /// Carries out a command, without recording a refusal.
    fn carry_out(&mut self, command: &BoundCommand) -> HostResult<Output> {
        if command.source == CommandSource::Verb {
            self.state.calls += 1;
            let mut record =
                Record::event("verb", self.now_tick()).with("name", command.name.as_str());
            for (name, arg) in &command.args {
                let value = match arg {
                    ArgValue::Value(value) => value.clone(),
                    ArgValue::Expr(expression) => {
                        self.eval(expression, subject_of(command).as_ref())?
                    }
                    other => Value::text(other.shown()),
                };
                record = record.with(name.as_str(), value);
            }
            self.records.push(record);
            return Ok(Output::text(format!("ran {}", command.name)));
        }
        let int = |name: &str| {
            command
                .int(name)
                .ok_or_else(|| HostError::failed(format!("`{name}` has no value")))
        };
        match command.name.as_str() {
            "new" => {
                let name = command.word("name").unwrap_or_default();
                if self.cell(name).is_some() {
                    return Err(HostError::refused(
                        "cell_exists",
                        format!("there is already a cell `{name}`"),
                    ));
                }
                let mut value = int("value")?;
                if let Some(leak) = &self.flaws.leak {
                    value = value.saturating_add(leak.get());
                    leak.set(leak.get() + 1);
                }
                self.state.cells.push(CellState {
                    name: name.to_owned(),
                    value,
                    locked: false,
                });
                let index = self.state.cells.len() - 1;
                self.changed(index, "cell_made");
                Ok(Output::text(format!("{name} = {value}")).with_subject(self.handle(index)))
            }
            "set" | "add" => {
                let index = self.cell_arg(command, "cell")?;
                self.unlocked(index)?;
                let cell = &self.state.cells[index];
                let value = if command.name == "set" {
                    int("value")?
                } else {
                    cell.value.checked_add(int("amount")?).ok_or_else(|| {
                        HostError::refused(
                            "out_of_range",
                            format!("`{}` would no longer fit a whole number", cell.name),
                        )
                    })?
                };
                self.state.cells[index].value = value;
                self.state.selected = u32::try_from(index).ok();
                self.changed(index, "cell_changed");
                Ok(Output::text(format!(
                    "{} = {value}",
                    self.state.cells[index].name
                )))
            }
            "lock" => {
                let index = self.cell_arg(command, "cell")?;
                self.state.cells[index].locked = true;
                Ok(Output::none())
            }
            "focus" => {
                let index = self.cell_arg(command, "cell")?;
                self.state.focus = u32::try_from(index).ok();
                Ok(Output::none().with_subject(self.handle(index)))
            }
            "timer" => {
                let index = self.cell_arg(command, "cell")?;
                let every = command.dur("every").unwrap_or(Dur::ZERO).as_micros();
                if every <= 0 {
                    return Err(HostError::refused(
                        "bad_interval",
                        "a timer's interval is more than nothing",
                    ));
                }
                let left = match command.int("times") {
                    Some(times) => Some(
                        u32::try_from(times)
                            .ok()
                            .filter(|times| *times > 0)
                            .ok_or_else(|| {
                                HostError::refused("bad_interval", "a timer fires at least once")
                            })?,
                    ),
                    None => None,
                };
                self.state.timers.push(Timer {
                    cell: u32::try_from(index).unwrap_or(u32::MAX),
                    every,
                    next: self.state.clock.saturating_add(every),
                    left,
                });
                Ok(Output::none())
            }
            "step" => {
                for _ in 0..int("count")?.max(0) {
                    if !self.fire_next(i64::MAX) {
                        return Err(HostError::refused(
                            "nothing_scheduled",
                            "there is no timer to fire",
                        ));
                    }
                }
                Ok(Output::none())
            }
            "check" => Ok(Output::text("0 violations")),
            "inspect" => {
                let index = match command.arg("subject") {
                    Some(_) => self.cell_arg(command, "subject")?,
                    None => self
                        .state
                        .selected
                        .map(|index| index as usize)
                        .ok_or_else(|| {
                            HostError::refused("nothing_selected", "nothing is selected")
                        })?,
                };
                let cell = &self.state.cells[index];
                Ok(Output::text(format!(
                    "name    string  {}\nvalue   int     {}\nlocked  bool    {}",
                    cell.name, cell.value, cell.locked
                )))
            }
            "do" => {
                let verb = match command.arg("effect") {
                    Some(ArgValue::Json(serde_json::Value::Object(effect)))
                        if effect.len() == 1 =>
                    {
                        effect.keys().next().cloned().unwrap_or_default()
                    }
                    _ => {
                        return Err(HostError::failed(
                            "an effect is an object with one key, the verb",
                        ));
                    }
                };
                self.state.calls += 1;
                self.records
                    .push(Record::event("effect", self.now_tick()).with("verb", verb.as_str()));
                Ok(Output::text(format!("ran {verb}")))
            }
            other => Err(HostError::failed(format!(
                "the fake host has no command `{other}`"
            ))),
        }
    }

    /// Notes whether a command was refused, for `last_refusal`.
    fn noted<T>(&mut self, result: HostResult<T>) -> HostResult<T> {
        self.last_refusal = match &result {
            Err(HostError::Refused(refusal)) => Some(refusal.clone()),
            _ => None,
        };
        result
    }
}

/// What `self` is for a command: its resolved `as <reference>`.
fn subject_of(command: &BoundCommand) -> Option<Value> {
    match &command.subject {
        Some(ArgValue::Value(value)) => Some(value.clone()),
        _ => None,
    }
}

/// Answers an expression's questions from a fake host's state.
struct Reader<'a> {
    state: &'a State,
    ids: Ids,
    own: &'a Value,
    locals: &'a [Value],
}

impl clash_expr::Host for Reader<'_> {
    fn subject(&mut self, _slot: u16) -> Value {
        self.own.clone()
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
        let cell = usize::try_from(subject.payload)
            .ok()
            .filter(|_| subject.kind == self.ids.cell)
            .and_then(|index| self.state.cells.get(index));
        match cell {
            Some(cell) if property == self.ids.value => Value::Int(cell.value),
            Some(cell) if property == self.ids.name => Value::text(&cell.name),
            Some(cell) if property == self.ids.locked => Value::Bool(cell.locked),
            _ => Value::Null,
        }
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        if function == self.ids.cell_of {
            args.first()
                .and_then(Value::as_str)
                .and_then(|name| self.state.cells.iter().position(|cell| cell.name == name))
                .map_or(Value::Null, |index| {
                    Value::Subject(Handle::new(self.ids.cell, index as u64))
                })
        } else if function == self.ids.clock {
            Value::Real(Dur(self.state.clock).as_secs_real())
        } else if function == self.ids.cells {
            Value::Int(i64::try_from(self.state.cells.len()).unwrap_or(i64::MAX))
        } else {
            Value::Null
        }
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {}

    fn def_name(&self, _id: DefRef) -> Option<&str> {
        None
    }
}

impl ConsoleHost for FakeHost {
    fn start(&mut self, profile: &str, seed: u64) -> HostResult<Output> {
        self.state = State::new(profile, seed);
        self.records.clear();
        self.last_refusal = None;
        Ok(Output::text(format!(
            "started `{profile}` with seed {seed}"
        )))
    }

    fn generate(&mut self, kind: &str, size: u32) -> HostResult<Output> {
        let result = if kind == "flat" {
            for _ in 0..size {
                let index = self.state.cells.len();
                self.state.cells.push(CellState {
                    name: format!("c{index}"),
                    value: 0,
                    locked: false,
                });
                self.changed(index, "cell_made");
            }
            Ok(Output::text(format!("made {size} cells")))
        } else {
            Err(Refusal::new(
                "unknown_world_kind",
                format!("the fake host cannot build a `{kind}` world, only `flat`"),
            )
            .with("kind", kind)
            .into())
        };
        self.noted(result)
    }

    fn execute(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let result = self.carry_out(command);
        self.noted(result)
    }

    fn resolve(&mut self, reference: &Reference) -> HostResult<Value> {
        let cells = self.state.cells.len();
        let nth = |index: Option<usize>| index.filter(|index| *index < cells);
        let by_name = |name: &str| self.state.cells.iter().position(|cell| cell.name == name);
        let (index, refusal) = match reference {
            Reference::Here => (nth(Some(0)), FakeHost::no_such_cell("yet")),
            Reference::Active => (
                self.state.focus.map(|index| index as usize),
                HostError::refused("nothing_in_focus", "no cell is in focus; `focus` sets one"),
            ),
            Reference::Selection => (
                self.state.selected.map(|index| index as usize),
                HostError::refused("nothing_selected", "nothing is selected"),
            ),
            Reference::Cell { x, y: 0, z: 0 } => (
                nth(usize::try_from(*x).ok()),
                FakeHost::no_such_cell(format_args!("at {reference}")),
            ),
            Reference::Cell { .. } => (
                None,
                HostError::refused(
                    "no_such_place",
                    format!("{reference} is outside the fake host's one row of cells"),
                ),
            ),
            Reference::Entity { slot, .. } => (
                nth(usize::try_from(*slot).ok()),
                FakeHost::no_such_cell(format_args!("numbered {slot}")),
            ),
            Reference::Of { kind, name } if kind == "cell" => (
                by_name(name),
                FakeHost::no_such_cell(format_args!("named `{name}`")),
            ),
            Reference::Of { kind, .. } => (
                None,
                HostError::refused(
                    "unknown_reference_kind",
                    format!("the fake host has nothing of kind `{kind}`, only `cell`"),
                ),
            ),
            Reference::Named(name) => (
                by_name(name),
                HostError::refused(
                    "unknown_name",
                    format!(
                        "nothing is named `{name}`; `as={name}` on a command names what it gives"
                    ),
                ),
            ),
        };
        index.map(|index| self.handle(index)).ok_or(refusal)
    }

    fn eval(&mut self, expression: &Expression, subject: Option<&Value>) -> HostResult<Value> {
        let (code, own, locals) = self.compile(expression, subject)?;
        Ok(self.run_code(&code, &own, &locals))
    }

    fn advance(&mut self, time: Dur) -> HostResult<()> {
        let target = self.state.clock.saturating_add(time.as_micros().max(0));
        while self.fire_next(target) {}
        self.state.clock = target;
        Ok(())
    }

    fn advance_until(
        &mut self,
        condition: &Expression,
        subject: Option<&Value>,
        max: Dur,
    ) -> HostResult<bool> {
        let (code, own, locals) = self.compile(condition, subject)?;
        let target = self.state.clock.saturating_add(max.as_micros().max(0));
        loop {
            if self.run_code(&code, &own, &locals) == Value::Bool(true) {
                return Ok(true);
            }
            if !self.fire_next(target) {
                self.state.clock = target;
                return Ok(self.run_code(&code, &own, &locals) == Value::Bool(true));
            }
        }
    }

    fn now(&self) -> Tick {
        self.now_tick()
    }

    fn state_hash(&self) -> Hash128 {
        Hash128::of(&self.state)
    }

    fn save(&mut self) -> HostResult<Vec<u8>> {
        let mut state = self.state.clone();
        if self.flaws.save_forgets_timers {
            state.timers.clear();
        }
        codec::to_bytes(&state)
            .map_err(|error| HostError::failed(format!("could not save: {error}")))
    }

    fn load(&mut self, bytes: &[u8]) -> HostResult<()> {
        self.state = codec::from_bytes(bytes).map_err(|error| {
            HostError::failed(format!("this is not a fake host's save: {error}"))
        })?;
        self.last_refusal = None;
        Ok(())
    }

    fn drain_records(&mut self) -> Vec<Record> {
        core::mem::take(&mut self.records)
    }

    fn last_refusal(&self) -> Option<Refusal> {
        self.last_refusal.clone()
    }

    fn frame(&mut self) -> HostResult<String> {
        let mut out = format!(
            "fake host  profile={} seed={}  t={}\n",
            self.state.profile,
            self.state.seed,
            Dur(self.state.clock)
        );
        for cell in &self.state.cells {
            out.push_str(&format!(
                "{} = {}{}\n",
                cell.name,
                cell.value,
                if cell.locked { " (locked)" } else { "" }
            ));
        }
        Ok(out)
    }

    fn set_threads(&mut self, threads: u32) -> HostResult<()> {
        if threads == 0 {
            return Err(HostError::failed("a host needs at least one thread"));
        }
        self.threads = threads;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::{HostKind, Session};

    fn run(host: &mut FakeHost, lines: &[&str]) -> Session {
        let table = FakeHost::table();
        let mut session = Session::new(HostKind::File);
        for line in lines {
            session
                .run_line(&table, host, line)
                .unwrap_or_else(|error| panic!("{line}: {error}"));
        }
        session
    }

    #[test]
    fn every_reference_form_resolves_or_is_refused_with_a_reason() {
        let mut host = FakeHost::new();
        run(
            &mut host,
            &["gen flat size=3", "focus @c:1,0,0", "add @e:2"],
        );
        let cell = |host: &mut FakeHost, reference: Reference| {
            host.resolve(&reference)
                .map(|value| value.as_subject().unwrap().payload)
        };
        assert_eq!(cell(&mut host, Reference::Here), Ok(0));
        assert_eq!(cell(&mut host, Reference::Active), Ok(1));
        assert_eq!(cell(&mut host, Reference::Selection), Ok(2));
        assert_eq!(cell(&mut host, Reference::Cell { x: 2, y: 0, z: 0 }), Ok(2));
        assert_eq!(
            cell(
                &mut host,
                Reference::Entity {
                    slot: 1,
                    generation: None
                }
            ),
            Ok(1)
        );
        assert_eq!(cell(&mut host, Reference::Named("c2".into())), Ok(2));
        assert_eq!(
            cell(
                &mut host,
                Reference::Of {
                    kind: "cell".into(),
                    name: "c0".into()
                }
            ),
            Ok(0)
        );
        for (reference, code) in [
            (Reference::Cell { x: 9, y: 0, z: 0 }, "no_such_cell"),
            (Reference::Cell { x: 0, y: 1, z: 0 }, "no_such_place"),
            (
                Reference::Entity {
                    slot: 7,
                    generation: None,
                },
                "no_such_cell",
            ),
            (Reference::Named("zed".into()), "unknown_name"),
            (
                Reference::Of {
                    kind: "site".into(),
                    name: "mill".into(),
                },
                "unknown_reference_kind",
            ),
            (
                Reference::Of {
                    kind: "cell".into(),
                    name: "zed".into(),
                },
                "no_such_cell",
            ),
        ] {
            match host.resolve(&reference) {
                Err(HostError::Refused(refusal)) => {
                    assert_eq!(refusal.code, code);
                    assert!(!refusal.message.is_empty());
                }
                other => panic!("{reference}: {other:?}"),
            }
        }
        let mut empty = FakeHost::new();
        for reference in [Reference::Here, Reference::Active, Reference::Selection] {
            assert!(matches!(
                empty.resolve(&reference),
                Err(HostError::Refused(_))
            ));
        }
    }

    #[test]
    fn saving_and_loading_gives_the_same_state_and_the_same_future() {
        let mut host = FakeHost::new();
        run(
            &mut host,
            &[
                "use fake seed=9",
                "new a value=4 as=a",
                "new b as=b",
                "timer @a every=300ms",
                "timer @b every=1s times=2",
                "lock @b",
                "focus @a",
                "advance 1s",
            ],
        );
        let saved = host.save().unwrap();
        let mut other = FakeHost::new();
        other.load(&saved).unwrap();
        assert_eq!(other.state_hash(), host.state_hash());
        assert_eq!(other.frame().unwrap(), host.frame().unwrap());
        for host in [&mut host, &mut other] {
            host.advance(Dur::from_secs(5)).unwrap();
        }
        assert_eq!(other.state_hash(), host.state_hash());
        assert_eq!(host.cell("a"), Some(4 + 20));
        assert_eq!(host.cell("b"), Some(0));
        assert_eq!(
            host.frame().unwrap(),
            "fake host  profile=fake seed=9  t=6s\na = 24\nb = 0 (locked)\n"
        );
        assert!(matches!(other.load(b"nonsense"), Err(HostError::Failed(_))));
    }

    #[test]
    fn the_hash_covers_everything_that_is_saved() {
        let hash = |lines: &[&str]| {
            let mut host = FakeHost::new();
            run(&mut host, lines);
            host.state_hash()
        };
        let base = hash(&["new a"]);
        for lines in [
            &["new b"][..],
            &["new a value=1"],
            &["new a", "lock @a"],
            &["new a", "focus @a"],
            &["new a", "set @a 0"],
            &["new a", "advance 1us"],
            &["new a", "timer @a every=1s"],
            &["use fake seed=1", "new a"],
            &["use other", "new a"],
            &["new a", "do { \"x\": {} }"],
        ] {
            assert_ne!(hash(lines), base, "{lines:?}");
        }
        assert_eq!(hash(&["new a"]), base);
    }

    #[test]
    fn its_own_commands_and_their_refusals() {
        let mut host = FakeHost::new();
        let mut session = run(
            &mut host,
            &["new a value=2 as=a", "timer @a every=1s times=2", "step 2"],
        );
        assert_eq!(host.cell("a"), Some(4));
        assert_eq!(host.now(), Tick(2_000_000));
        let table = FakeHost::table();
        for (line, code) in [
            ("new a", "cell_exists"),
            ("step", "nothing_scheduled"),
            ("timer @a every=0s", "bad_interval"),
            ("timer @a every=1s times=0", "bad_interval"),
            ("add @a 9223372036854775807", "out_of_range"),
            ("gen ring", "unknown_world_kind"),
            ("inspect \"cell('nobody')\"", "no_such_cell"),
        ] {
            match session.run_line(&table, &mut host, line) {
                Err(crate::console::ConsoleError::Refused(refusal)) => {
                    assert_eq!(refusal.code, code, "{line}");
                    assert_eq!(host.last_refusal().unwrap().code, code, "{line}");
                }
                other => panic!("{line}: {other:?}"),
            }
        }
        let output = session.run_line(&table, &mut host, "inspect @a").unwrap();
        assert!(output.text.contains("value   int     4"));
        assert_eq!(host.last_refusal(), None);
        assert_eq!(
            session.run_line(&table, &mut host, "check").unwrap().text,
            "0 violations"
        );
        assert!(session.run_line(&table, &mut host, "profile on").is_err());
        assert!(host.set_threads(0).is_err());
        host.set_threads(8).unwrap();
        assert_eq!(host.threads(), 8);
    }

    #[test]
    fn expressions_that_do_not_compile_are_failures_with_the_reason() {
        let mut host = FakeHost::new();
        let mut session = run(&mut host, &["new a as=a"]);
        let table = FakeHost::table();
        let error = session
            .run_line(&table, &mut host, "eval @a.colour")
            .unwrap_err();
        let crate::console::ConsoleError::Failed(message) = error else {
            panic!("{error:?}")
        };
        assert!(
            message.contains("`@a.colour` does not compile"),
            "{message}"
        );
        assert_eq!(
            session
                .run_line(&table, &mut host, "eval self")
                .unwrap()
                .text,
            "null"
        );
        assert_eq!(
            session
                .run_line(&table, &mut host, "eval cells() + 1")
                .unwrap()
                .text,
            "2"
        );
    }
}
