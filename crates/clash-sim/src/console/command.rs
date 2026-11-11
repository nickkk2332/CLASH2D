//! Console commands: how one is registered, the table that holds them,
//! binding a parsed line to one, and the help generated from the
//! registrations (14 §3.3).
//!
//! A [`CommandSpec`] is plain data. The [`CommandTable`] holds the
//! built-in commands of 14 §3.3 and derives one command from each verb
//! in a [`clash_expr::Registry`], so that registering a verb is all it
//! takes for the console to have it.
//!
//! # How a line is bound
//!
//! 1. The command is looked up by its first two words (`map export`) and
//!    then by its first.
//! 2. A `key=value` argument goes to the parameter of that name. `as=`
//!    is not a parameter: it names what the command gives back, on any
//!    command.
//! 3. A bare word that is the name of a parameter not yet given, with a
//!    positional value after it, names that parameter: `spawn town_guard
//!    at @c:1,2,0` is `at=@c:1,2,0`. A value that happens to be such a
//!    word is given by name instead.
//! 4. Every other positional value goes to the next parameter in order
//!    that has no value yet. A verb's direct argument (05 §10.1) is its
//!    first parameter. A parameter marked `rest` takes every positional
//!    value from there on, joined by single spaces: `eval 1 + 2`.
//! 5. Each value is checked against its parameter's type, parameters
//!    left out take their defaults, and everything wrong with the line
//!    is reported at once.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use clash_core::Dur;
use clash_expr::{Quantity, Registry, Type, VerbSpec};

use super::diag::{Diagnostic, Span, codes, did_you_mean};
use super::host::Expression;
use super::syntax::{
    self, Arg, Line, Numeral, Reference, ValueKind, is_reference_name, is_word, print_value,
};

/// The three places a console runs (14 §3.1).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum HostKind {
    /// The overlay in the game.
    Game,
    /// The terminal world viewer.
    Viewer,
    /// A file read line by line: a test script or a saved session.
    File,
}

/// Which hosts offer a command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hosts {
    /// The overlay in the game.
    pub game: bool,
    /// The terminal world viewer.
    pub viewer: bool,
    /// A file.
    pub file: bool,
}

impl Hosts {
    /// Offered everywhere.
    pub const ALL: Hosts = Hosts {
        game: true,
        viewer: true,
        file: true,
    };
    /// Offered where a person is typing: the game and the viewer.
    pub const INTERACTIVE: Hosts = Hosts {
        game: true,
        viewer: true,
        file: false,
    };
    /// Offered only in files.
    pub const FILE: Hosts = Hosts {
        game: false,
        viewer: false,
        file: true,
    };

    /// Whether a host of this kind offers the command.
    pub const fn offers(self, kind: HostKind) -> bool {
        match kind {
            HostKind::Game => self.game,
            HostKind::Viewer => self.viewer,
            HostKind::File => self.file,
        }
    }

    fn describe(self) -> &'static str {
        match (self.game, self.viewer, self.file) {
            (true, true, true) => "the game, the viewer and files",
            (true, true, false) => "the game and the viewer",
            (false, false, true) => "files",
            (true, false, true) => "the game and files",
            (false, true, true) => "the viewer and files",
            (true, false, false) => "the game",
            (false, true, false) => "the viewer",
            (false, false, false) => "no host",
        }
    }
}

/// What a parameter takes.
#[derive(Clone, PartialEq, Debug)]
pub enum ParamType {
    /// A whole number without a unit.
    Int,
    /// A number; one with a unit is converted to SI.
    Real,
    /// `true` or `false`.
    Bool,
    /// A time: a number with a unit of time, or a bare number of
    /// seconds.
    Duration,
    /// A name: a word, or anything in quotes.
    Word,
    /// One of a fixed list of words.
    Choice(Vec<String>),
    /// A text: quoted, or a single unquoted value as written.
    Text,
    /// An expression (05 §4), quoted or not. It is checked for syntax
    /// when the line is bound and compiled by the host.
    Expr,
    /// JSON in braces.
    Json,
    /// A reference; or an expression that gives a subject.
    Subject,
    /// A value of a type of the expression language, as a verb's
    /// arguments are. A number or a word is taken as written, a
    /// reference as what it names, and anything quoted or unquoted as
    /// an expression — except that a quoted value given to a `string`
    /// is the text itself.
    Value {
        /// The type.
        ty: Type,
        /// The type as the language writes it: `id<material>`.
        shown: String,
    },
}

impl ParamType {
    /// A value of any type.
    pub fn any() -> ParamType {
        ParamType::Value {
            ty: Type::Any,
            shown: "any".to_owned(),
        }
    }

    /// One of these words.
    pub fn choice<S: Into<String>>(options: impl IntoIterator<Item = S>) -> ParamType {
        ParamType::Choice(options.into_iter().map(Into::into).collect())
    }

    /// The type in a few words, for help and for messages: "a time".
    pub fn describe(&self) -> String {
        match self {
            ParamType::Int => "a whole number".to_owned(),
            ParamType::Real => "a number".to_owned(),
            ParamType::Bool => "true or false".to_owned(),
            ParamType::Duration => "a time".to_owned(),
            ParamType::Word => "a name".to_owned(),
            ParamType::Choice(options) => format!("one of {}", options.join(", ")),
            ParamType::Text => "a text".to_owned(),
            ParamType::Expr => "an expression".to_owned(),
            ParamType::Json => "JSON in braces".to_owned(),
            ParamType::Subject => "a reference".to_owned(),
            ParamType::Value { shown, .. } => format!("a value of type {shown}"),
        }
    }
}

/// The value of a bound argument.
#[derive(Clone, PartialEq, Debug)]
pub enum ArgValue {
    /// A value known as it was typed: a number, a truth value, a text;
    /// and, once the console has resolved it, what a reference names.
    Value(clash_expr::Value),
    /// A time.
    Dur(Dur),
    /// A name, an id or a keyword, for the host to look up as the
    /// parameter's type says.
    Word(String),
    /// An expression for the host to compile and evaluate.
    Expr(Expression),
    /// A reference the console has not resolved yet. A host never sees
    /// one: the console replaces it with [`ArgValue::Value`].
    Ref(Reference),
    /// JSON.
    Json(serde_json::Value),
}

impl ArgValue {
    /// The argument as it would be typed.
    pub fn shown(&self) -> String {
        match self {
            ArgValue::Value(value) => value.to_string(),
            ArgValue::Dur(time) => time.to_string(),
            ArgValue::Word(word) => word.clone(),
            ArgValue::Expr(expression) => expression.written.clone(),
            ArgValue::Ref(reference) => reference.to_string(),
            ArgValue::Json(json) => json.to_string(),
        }
    }
}

/// One parameter of a command.
#[derive(Clone, PartialEq, Debug)]
pub struct ParamSpec {
    /// The name, for `name=value`.
    pub name: String,
    /// What it takes.
    pub ty: ParamType,
    /// What it means.
    pub doc: String,
    /// Whether it must be given.
    pub required: bool,
    /// What it is when left out, if it has a default.
    pub default: Option<ArgValue>,
    /// Whether it takes every positional value from its place on.
    pub rest: bool,
}

impl ParamSpec {
    /// A parameter that must be given.
    pub fn required(name: impl Into<String>, ty: ParamType, doc: impl Into<String>) -> ParamSpec {
        ParamSpec {
            name: name.into(),
            ty,
            doc: doc.into(),
            required: true,
            default: None,
            rest: false,
        }
    }

    /// A parameter that may be left out, and is then absent.
    pub fn optional(name: impl Into<String>, ty: ParamType, doc: impl Into<String>) -> ParamSpec {
        ParamSpec {
            required: false,
            ..ParamSpec::required(name, ty, doc)
        }
    }

    /// The same parameter, taking this value when left out.
    #[must_use]
    pub fn default(mut self, value: ArgValue) -> ParamSpec {
        self.required = false;
        self.default = Some(value);
        self
    }

    /// The same parameter, taking every positional value from its place
    /// on.
    #[must_use]
    pub fn rest(mut self) -> ParamSpec {
        self.rest = true;
        self
    }

    fn usage(&self) -> String {
        let dots = if self.rest { "…" } else { "" };
        match (&self.default, self.required) {
            (_, true) => format!("<{}{dots}>", self.name),
            (Some(default), false) => format!("[{}={}]", self.name, default.shown()),
            (None, false) => format!("[{}{dots}]", self.name),
        }
    }
}

/// Where a command comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CommandSource {
    /// One of the commands of 14 §3.3, or an engine function a
    /// subsystem registered.
    Builtin,
    /// A verb (05 §10): the command runs the effect of that name.
    Verb,
    /// A `console_command` definition (14 §3.4): the command runs this
    /// rule with its arguments as the rule's parameters.
    Rule(String),
}

/// A console command as it is registered.
///
/// ```
/// use clash_sim::console::{CommandSpec, ParamSpec, ParamType};
///
/// let spec = CommandSpec::new("stock_shelter")
///     .doc("Fill the starting shelter's stores.")
///     .param(ParamSpec::optional("days", ParamType::Int, "How many days of food."))
///     .changes();
/// assert_eq!(spec.usage(), "stock_shelter [days]");
/// assert!(spec.help().contains("Fill the starting shelter's stores."));
/// ```
#[derive(Clone, PartialEq, Debug)]
pub struct CommandSpec {
    /// The name: one word, or two for a command such as `map export`.
    pub name: String,
    /// What it does.
    pub doc: String,
    /// Its parameters, in positional order.
    pub params: Vec<ParamSpec>,
    /// Whether it changes the simulation, and so is a `Debug` command
    /// (03 §4.4).
    pub changes: bool,
    /// Which hosts offer it.
    pub hosts: Hosts,
    /// Whether it also takes named arguments that are not parameters,
    /// as `expect event` takes an event's fields.
    pub open: bool,
    /// Where it comes from.
    pub source: CommandSource,
}

impl CommandSpec {
    /// A built-in command with no parameters that changes nothing and
    /// is offered everywhere.
    pub fn new(name: impl Into<String>) -> CommandSpec {
        CommandSpec {
            name: name.into(),
            doc: String::new(),
            params: Vec::new(),
            changes: false,
            hosts: Hosts::ALL,
            open: false,
            source: CommandSource::Builtin,
        }
    }

    /// The command of a verb: the verb's arguments are its parameters,
    /// the direct argument first.
    pub fn from_verb(verb: &VerbSpec, registry: &Registry) -> CommandSpec {
        let mut params: Vec<ParamSpec> = verb
            .args
            .iter()
            .map(|arg| ParamSpec {
                name: arg.name.clone(),
                ty: ParamType::Value {
                    ty: arg.ty.clone(),
                    shown: registry.type_name(&arg.ty),
                },
                doc: arg.doc.clone(),
                required: !arg.optional,
                default: None,
                rest: false,
            })
            .collect();
        if let Some(direct) = &verb.direct
            && let Some(at) = params.iter().position(|param| param.name == *direct)
        {
            let param = params.remove(at);
            params.insert(0, param);
        }
        CommandSpec {
            name: verb.name.clone(),
            doc: verb.doc.clone(),
            params,
            changes: true,
            hosts: Hosts::ALL,
            open: false,
            source: CommandSource::Verb,
        }
    }

    /// Sets the documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> CommandSpec {
        self.doc = doc.into();
        self
    }

    /// Adds a parameter.
    #[must_use]
    pub fn param(mut self, param: ParamSpec) -> CommandSpec {
        self.params.push(param);
        self
    }

    /// Marks the command as changing the simulation.
    #[must_use]
    pub fn changes(mut self) -> CommandSpec {
        self.changes = true;
        self
    }

    /// Sets which hosts offer it.
    #[must_use]
    pub fn hosts(mut self, hosts: Hosts) -> CommandSpec {
        self.hosts = hosts;
        self
    }

    /// Lets the command take named arguments that are not parameters.
    #[must_use]
    pub fn open(mut self) -> CommandSpec {
        self.open = true;
        self
    }

    /// Sets where the command comes from.
    #[must_use]
    pub fn source(mut self, source: CommandSource) -> CommandSpec {
        self.source = source;
        self
    }

    fn param_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|param| param.name == name)
    }

    /// The command as it is typed: `until <condition…> <max>`.
    pub fn usage(&self) -> String {
        let mut out = self.name.clone();
        for param in &self.params {
            out.push(' ');
            out.push_str(&param.usage());
        }
        if self.open {
            out.push_str(" [field=value…]");
        }
        out
    }

    /// The command's help: its usage, its documentation, each parameter
    /// with its type and documentation, whether it changes the
    /// simulation and where it is offered.
    pub fn help(&self) -> String {
        let mut out = format!("{}\n", self.usage());
        if !self.doc.is_empty() {
            let _ = writeln!(out, "    {}", self.doc);
        }
        let width = self
            .params
            .iter()
            .map(|param| param.name.chars().count())
            .max()
            .unwrap_or(0);
        for param in &self.params {
            let _ = write!(out, "    {:<width$}  {}", param.name, param.ty.describe());
            if let Some(default) = &param.default {
                let _ = write!(out, ", default {}", default.shown());
            } else if !param.required {
                out.push_str(", optional");
            }
            if !param.doc.is_empty() {
                let _ = write!(out, ". {}", param.doc);
            }
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "    {} Offered in {}.",
            if self.changes {
                "Changes the simulation."
            } else {
                "Changes nothing."
            },
            self.hosts.describe()
        );
        out
    }

    /// The first sentence of the documentation.
    fn summary(&self) -> &str {
        match self.doc.find(". ") {
            Some(end) => &self.doc[..=end],
            None => &self.doc,
        }
    }
}

/// Why a command could not be added to a table.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum TableError {
    /// The table already has a command of this name.
    #[error("there is already a command named `{0}`")]
    Duplicate(String),
    /// The registration is not one the console can bind.
    #[error("the command `{name}` cannot be registered: {problem}")]
    BadSpec {
        /// The command's name.
        name: String,
        /// What is wrong with it.
        problem: String,
    },
}

/// A line bound to a command: which command, what `self` is, and a
/// value for each parameter that has one.
#[derive(Clone, PartialEq, Debug)]
pub struct BoundCommand {
    /// The command's name.
    pub name: String,
    /// Where the command comes from.
    pub source: CommandSource,
    /// Whether it changes the simulation.
    pub changes: bool,
    /// What `self` is bound to: the `as <reference>` prefix.
    pub subject: Option<ArgValue>,
    /// The arguments by parameter name, in the order of the command's
    /// parameters, defaults included. A parameter left out that has no
    /// default is absent.
    pub args: Vec<(String, ArgValue)>,
    /// Named arguments that are not parameters, for a command that is
    /// open to them, in the order written.
    pub fields: Vec<(String, ArgValue)>,
    /// The name to give what the command gives back: `as=`.
    pub result_name: Option<String>,
}

impl BoundCommand {
    /// The argument for a parameter, if it has one.
    pub fn arg(&self, name: &str) -> Option<&ArgValue> {
        self.args
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, value)| value)
    }

    /// An argument that is a value known as typed, or a resolved
    /// reference.
    pub fn value(&self, name: &str) -> Option<&clash_expr::Value> {
        match self.arg(name)? {
            ArgValue::Value(value) => Some(value),
            _ => None,
        }
    }

    /// An argument that is a whole number.
    pub fn int(&self, name: &str) -> Option<i64> {
        self.value(name)?.as_int()
    }

    /// An argument that is a word.
    pub fn word(&self, name: &str) -> Option<&str> {
        match self.arg(name)? {
            ArgValue::Word(word) => Some(word),
            _ => None,
        }
    }

    /// An argument that is a text.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.value(name)?.as_str()
    }

    /// An argument that is a time.
    pub fn dur(&self, name: &str) -> Option<Dur> {
        match self.arg(name)? {
            ArgValue::Dur(time) => Some(*time),
            _ => None,
        }
    }

    /// An argument that is an expression.
    pub fn expr(&self, name: &str) -> Option<&Expression> {
        match self.arg(name)? {
            ArgValue::Expr(expression) => Some(expression),
            _ => None,
        }
    }
}

/// The commands a console has.
///
/// ```
/// use clash_expr::{Registry, Type, VerbSpec};
/// use clash_sim::console::{CommandTable, HostKind, parse_line};
///
/// let mut registry = Registry::with_builtins();
/// registry
///     .add_verb(
///         VerbSpec::new("shove")
///             .doc("Pushes something.")
///             .arg("force", Type::Real, "How hard, in newtons.")
///             .direct("force"),
///     )
///     .unwrap();
/// let mut table = CommandTable::builtin();
/// assert!(table.add_verbs(&registry).is_empty());
///
/// let line = parse_line("shove 2kN").unwrap();
/// let bound = table.bind(&line, HostKind::File).unwrap().unwrap();
/// assert_eq!(bound.value("force").unwrap().as_number().unwrap().get(), 2000.0);
/// assert!(table.help("shove").unwrap().contains("How hard"));
/// ```
#[derive(Clone, Debug, Default)]
pub struct CommandTable {
    commands: BTreeMap<String, CommandSpec>,
}

impl CommandTable {
    /// A table with no commands.
    pub fn new() -> CommandTable {
        CommandTable::default()
    }

    /// A table with the built-in commands of 14 §3.3.
    pub fn builtin() -> CommandTable {
        let mut table = CommandTable::new();
        for spec in builtin_specs() {
            // The built-in registrations are checked by a test.
            let _ = table.add(spec);
        }
        table
    }

    /// Adds a command.
    pub fn add(&mut self, spec: CommandSpec) -> Result<(), TableError> {
        let bad = |problem: String| TableError::BadSpec {
            name: spec.name.clone(),
            problem,
        };
        let words: Vec<&str> = spec.name.split(' ').collect();
        if words.len() > 2 || !words.iter().all(|word| is_word(word)) {
            return Err(bad("a command's name is one word or two".to_owned()));
        }
        for (index, param) in spec.params.iter().enumerate() {
            if syntax::ident_only(&param.name).is_none() {
                return Err(bad(format!(
                    "the parameter name `{}` is not letters, digits and underscores",
                    param.name
                )));
            }
            if spec.params[..index]
                .iter()
                .any(|earlier| earlier.name == param.name)
            {
                return Err(bad(format!("it has two parameters named `{}`", param.name)));
            }
            if param.rest && !matches!(param.ty, ParamType::Expr | ParamType::Text) {
                return Err(bad(format!(
                    "`{}` takes the rest of the line, which only an expression or a text can",
                    param.name
                )));
            }
        }
        if self.commands.contains_key(&spec.name) {
            return Err(TableError::Duplicate(spec.name));
        }
        self.commands.insert(spec.name.clone(), spec);
        Ok(())
    }

    /// Adds one command for each verb of a registry (14 §3.3). A verb
    /// whose name a command already has is left out, because the
    /// built-in commands keep their names; the names left out are
    /// returned, and such a verb is still reached with `do`.
    pub fn add_verbs(&mut self, registry: &Registry) -> Vec<String> {
        let mut shadowed = Vec::new();
        for verb in registry.verbs() {
            let spec = CommandSpec::from_verb(&verb.spec, registry);
            if self.add(spec).is_err() {
                shadowed.push(verb.spec.name.clone());
            }
        }
        shadowed
    }

    /// The command of this name.
    pub fn get(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name)
    }

    /// Every command, in order of name.
    pub fn commands(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.values()
    }

    /// How many commands there are.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether the table has no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// The help of one command, or `None` if there is none of that name.
    pub fn help(&self, name: &str) -> Option<String> {
        self.get(name).map(CommandSpec::help)
    }

    /// One line for each command, in order of name: how it is typed and
    /// the first sentence of what it does.
    pub fn help_all(&self) -> String {
        let width = self
            .commands
            .values()
            .map(|spec| spec.usage().chars().count())
            .max()
            .unwrap_or(0);
        let mut out = String::new();
        for spec in self.commands.values() {
            let _ = writeln!(out, "{:<width$}  {}", spec.usage(), spec.summary());
        }
        out
    }

    /// "did you mean" for a command name nobody registered.
    fn suggest(&self, name: &str) -> Option<String> {
        did_you_mean(name, self.commands.keys().map(String::as_str))
    }

    /// Binds a parsed line to its command. Gives `None` for a line with
    /// no command, and every diagnostic at once for a line that does not
    /// fit its command.
    pub fn bind(
        &self,
        line: &Line,
        host: HostKind,
    ) -> Result<Option<BoundCommand>, Vec<Diagnostic>> {
        let Some(command) = &line.command else {
            return Ok(None);
        };
        let mut args: &[Arg] = &command.args;
        let mut name_span = command.name_span;
        let mut spec = None;
        if let Some(Arg {
            key: None,
            value:
                syntax::Value {
                    kind: ValueKind::Word(second),
                    span,
                },
        }) = args.first()
            && let Some(found) = self.commands.get(&format!("{} {second}", command.name))
        {
            spec = Some(found);
            name_span = name_span.to(*span);
            args = &args[1..];
        }
        let Some(spec) = spec.or_else(|| self.commands.get(&command.name)) else {
            return Err(vec![
                Diagnostic::new(
                    codes::UNKNOWN_COMMAND,
                    name_span,
                    format!("there is no command `{}`", command.name),
                )
                .with_suggestion(
                    self.suggest(&command.name)
                        .unwrap_or_else(|| "`help` lists the commands".to_owned()),
                ),
            ]);
        };
        if !spec.hosts.offers(host) {
            return Err(vec![Diagnostic::new(
                codes::NOT_OFFERED,
                name_span,
                format!(
                    "`{}` is not offered here: it is offered in {}",
                    spec.name,
                    spec.hosts.describe()
                ),
            )]);
        }
        Binder {
            spec,
            name_span,
            errors: Vec::new(),
        }
        .bind(line, args)
    }
}

struct Binder<'a> {
    spec: &'a CommandSpec,
    name_span: Span,
    errors: Vec<Diagnostic>,
}

impl Binder<'_> {
    fn bind(mut self, line: &Line, args: &[Arg]) -> Result<Option<BoundCommand>, Vec<Diagnostic>> {
        let spec = self.spec;
        let mut bound: Vec<Option<ArgValue>> = vec![None; spec.params.len()];
        // Which parameters an argument was written for, whether or not
        // its value turned out to fit.
        let mut given = vec![false; spec.params.len()];
        let mut fields = Vec::new();
        let mut result_name: Option<String> = None;
        // The values a `rest` parameter has gathered so far.
        let mut rest: Vec<&syntax::Value> = Vec::new();
        let mut rest_at = None;
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            index += 1;
            let mut value = &arg.value;
            let mut key = arg.key.as_ref().map(|key| (key.name.as_str(), key.span));
            // A bare word that names a parameter still to be given,
            // with a positional value after it.
            if key.is_none()
                && let ValueKind::Word(word) = &value.kind
                && let Some(param) = spec.param_index(word)
                && !given[param]
                && let Some(next) = args.get(index)
                && next.key.is_none()
            {
                key = Some((word.as_str(), value.span));
                value = &next.value;
                index += 1;
            }
            match key {
                Some(("as", at)) => match &value.kind {
                    _ if result_name.is_some() => self.errors.push(Diagnostic::new(
                        codes::DUPLICATE_ARGUMENT,
                        at,
                        "`as=` is given twice",
                    )),
                    ValueKind::Word(name) if is_reference_name(name) => {
                        result_name = Some(name.clone());
                    }
                    other => self.errors.push(
                        Diagnostic::new(
                            codes::BAD_NAME,
                            value.span,
                            format!("`{}` cannot be a name", print_value(other)),
                        )
                        .with_suggestion(
                            "a name is letters, digits and underscores, and not here, self, sel, c or e",
                        ),
                    ),
                },
                Some((name, at)) => match spec.param_index(name) {
                    Some(param) if given[param] => {
                        self.errors.push(Diagnostic::new(
                            codes::DUPLICATE_ARGUMENT,
                            at,
                            format!("`{name}` is given twice"),
                        ));
                    }
                    Some(param) => {
                        given[param] = true;
                        bound[param] = self.convert(&spec.params[param], value);
                    }
                    None if spec.open => {
                        if let Some(field) = self.report(field_value(value)) {
                            fields.push((name.to_owned(), field));
                        }
                    }
                    None => {
                        let names = spec
                            .params
                            .iter()
                            .map(|param| param.name.as_str())
                            .chain(["as"]);
                        self.errors.push(
                            Diagnostic::new(
                                codes::UNKNOWN_ARGUMENT,
                                at,
                                format!("`{}` has no parameter `{name}`", spec.name),
                            )
                            .with_suggestion(
                                did_you_mean(name, names)
                                    .unwrap_or_else(|| format!("it is typed `{}`", spec.usage())),
                            ),
                        );
                    }
                },
                None => {
                    let next = rest_at.or_else(|| given.iter().position(|given| !given));
                    match next {
                        Some(param) if spec.params[param].rest => {
                            given[param] = true;
                            rest_at = Some(param);
                            rest.push(value);
                        }
                        Some(param) => {
                            given[param] = true;
                            bound[param] = self.convert(&spec.params[param], value);
                        }
                        None => self.errors.push(
                            Diagnostic::new(
                                codes::TOO_MANY_ARGUMENTS,
                                value.span,
                                format!(
                                    "`{}` takes {} and this is one more",
                                    spec.name,
                                    match spec.params.len() {
                                        0 => "no arguments".to_owned(),
                                        1 => "one argument".to_owned(),
                                        count => format!("{count} arguments"),
                                    }
                                ),
                            )
                            .with_suggestion(format!("it is typed `{}`", spec.usage())),
                        ),
                    }
                }
            }
        }
        if let Some(param) = rest_at {
            bound[param] = match rest.as_slice() {
                [one] => self.convert(&spec.params[param], one),
                many => {
                    let joined: Vec<String> =
                        many.iter().map(|value| print_value(&value.kind)).collect();
                    let whole = syntax::Value {
                        kind: ValueKind::Raw(joined.join(" ")),
                        span: many
                            .iter()
                            .map(|value| value.span)
                            .reduce(Span::to)
                            .unwrap_or(self.name_span),
                    };
                    self.convert(&spec.params[param], &whole)
                }
            };
        }
        let mut out = Vec::new();
        for ((param, value), given) in spec.params.iter().zip(bound).zip(given) {
            match value.or_else(|| param.default.clone()) {
                Some(value) => out.push((param.name.clone(), value)),
                // A value that did not fit has been reported already.
                None if param.required && !given => {
                    self.errors.push(
                        Diagnostic::new(
                            codes::MISSING_ARGUMENT,
                            self.name_span,
                            format!(
                                "`{}` needs `{}`: {}",
                                spec.name,
                                param.name,
                                param.ty.describe()
                            ),
                        )
                        .with_suggestion(format!("it is typed `{}`", spec.usage())),
                    );
                }
                None => {}
            }
        }
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        Ok(Some(BoundCommand {
            name: spec.name.clone(),
            source: spec.source.clone(),
            changes: spec.changes,
            subject: line
                .subject
                .as_ref()
                .map(|subject| ArgValue::Ref(subject.reference.clone())),
            args: out,
            fields,
            result_name,
        }))
    }

    fn report(&mut self, result: Result<ArgValue, Diagnostic>) -> Option<ArgValue> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.errors.push(error);
                None
            }
        }
    }

    fn convert(&mut self, param: &ParamSpec, value: &syntax::Value) -> Option<ArgValue> {
        let result = convert(param, value);
        self.report(result)
    }
}

/// An expression written as a value, reported at the value.
fn expression(text: &str, at: Span) -> Result<ArgValue, Diagnostic> {
    Expression::parse(text)
        .map(ArgValue::Expr)
        .map_err(|mut error| {
            error.span = at;
            error
        })
}

/// What a value is, in a few words, for a message.
fn found(value: &ValueKind) -> String {
    match value {
        ValueKind::Number(_) => format!("the number `{}`", print_value(value)),
        ValueKind::Word(word) => format!("the word `{word}`"),
        ValueKind::Ref(reference) => format!("the reference `{reference}`"),
        ValueKind::Quoted(_) => "a quoted value".to_owned(),
        ValueKind::Json(_) => "JSON".to_owned(),
        ValueKind::Raw(text) => format!("`{text}`"),
    }
}

/// The value of a named argument that is not a parameter: a number or a
/// word as written, a reference, an expression, or JSON.
pub(crate) fn field_value(value: &syntax::Value) -> Result<ArgValue, Diagnostic> {
    convert(&ParamSpec::optional("field", ParamType::any(), ""), value)
}

/// A value that must be a time, for a parameter of this name.
pub(crate) fn duration(name: &str, value: &syntax::Value) -> Result<Dur, Diagnostic> {
    let param = ParamSpec::required(name, ParamType::Duration, "");
    match convert(&param, value)? {
        ArgValue::Dur(time) => Ok(time),
        other => Err(Diagnostic::new(
            codes::WRONG_TYPE,
            value.span,
            format!("`{name}` takes a time, and this is `{}`", other.shown()),
        )),
    }
}

/// Checks a value against a parameter's type.
fn convert(param: &ParamSpec, value: &syntax::Value) -> Result<ArgValue, Diagnostic> {
    use clash_expr::Value as V;
    let at = value.span;
    let wrong = |hint: Option<String>| {
        Diagnostic::new(
            codes::WRONG_TYPE,
            at,
            format!(
                "`{}` takes {}, and this is {}",
                param.name,
                param.ty.describe(),
                found(&value.kind)
            ),
        )
        .with_suggestion_opt(hint)
    };
    // A number with letters after it that are not a unit.
    let unit_hint = |text: &str| {
        let digits = text
            .trim_start_matches(['+', '-'])
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        (digits.len() < text.len() && !digits.is_empty()).then(|| {
            let units = || clash_expr::Unit::all().map(clash_expr::Unit::symbol);
            // `sec` is most likely `s`, `mins` `min`: the longest unit
            // the letters start with, and failing that the closest.
            let starts = units()
                .filter(|unit| digits.starts_with(unit))
                .max_by_key(|unit| unit.len());
            match starts.or_else(|| clash_expr::closest_name(digits, units())) {
                Some(unit) => format!("`{digits}` is not a unit; did you mean `{unit}`?"),
                None => format!("`{digits}` is not a unit"),
            }
        })
    };
    match (&param.ty, &value.kind) {
        (ParamType::Int, ValueKind::Number(number)) => match (number.value, number.unit) {
            (Numeral::Int(whole), None) => Ok(ArgValue::Value(V::Int(whole))),
            _ => Err(wrong(None)),
        },
        (ParamType::Real, ValueKind::Number(number)) => Ok(ArgValue::Value(V::Real(number.si()))),
        (ParamType::Int | ParamType::Real | ParamType::Duration, ValueKind::Raw(text)) => {
            Err(wrong(unit_hint(text)))
        }
        (ParamType::Bool, ValueKind::Word(word)) if word == "true" || word == "false" => {
            Ok(ArgValue::Value(V::Bool(word == "true")))
        }
        (ParamType::Duration, ValueKind::Number(number)) => match number.unit {
            Some(unit) if unit.quantity() != Quantity::Time => Err(wrong(Some(format!(
                "`{}` is not a unit of time; a time is written `2s`, `400ms` or `5min`",
                unit.symbol()
            )))),
            _ => Ok(ArgValue::Dur(Dur::from_secs_real(number.si()))),
        },
        (
            ParamType::Word,
            ValueKind::Word(text) | ValueKind::Quoted(text) | ValueKind::Raw(text),
        ) => Ok(ArgValue::Word(text.clone())),
        (ParamType::Choice(options), ValueKind::Word(word)) => {
            if options.contains(word) {
                Ok(ArgValue::Word(word.clone()))
            } else {
                Err(wrong(did_you_mean(
                    word,
                    options.iter().map(String::as_str),
                )))
            }
        }
        (ParamType::Text, ValueKind::Quoted(text)) => Ok(ArgValue::Value(V::text(text))),
        (
            ParamType::Text,
            kind @ (ValueKind::Word(_) | ValueKind::Raw(_) | ValueKind::Number(_)),
        ) => Ok(ArgValue::Value(V::text(print_value(kind)))),
        (ParamType::Expr, ValueKind::Quoted(text)) => expression(text, at),
        (
            ParamType::Expr,
            kind @ (ValueKind::Word(_)
            | ValueKind::Raw(_)
            | ValueKind::Number(_)
            | ValueKind::Ref(_)),
        ) => expression(&print_value(kind), at),
        (ParamType::Json, ValueKind::Json(json)) => Ok(ArgValue::Json(json.clone())),
        (ParamType::Json, _) => Err(wrong(Some(
            "JSON is written in braces: `{ \"verb\": { … } }`".to_owned(),
        ))),
        (ParamType::Subject, ValueKind::Ref(reference)) => Ok(ArgValue::Ref(reference.clone())),
        (ParamType::Subject, ValueKind::Quoted(text) | ValueKind::Raw(text)) => {
            expression(text, at)
        }
        (ParamType::Subject, ValueKind::Word(word)) => Err(wrong(Some(format!(
            "a reference begins with `@`: `@{word}`"
        )))),
        (ParamType::Value { ty, .. }, kind) => {
            let ty = ty.non_null();
            match kind {
                ValueKind::Number(number) => {
                    let literal = match (number.value, number.unit) {
                        (Numeral::Int(whole), None) => V::Int(whole),
                        _ => V::Real(number.si()),
                    };
                    let fits = match ty {
                        Type::Any | Type::Number | Type::Real => true,
                        Type::Int => matches!(literal, V::Int(_)),
                        _ => false,
                    };
                    if fits {
                        Ok(ArgValue::Value(literal))
                    } else {
                        Err(wrong(None))
                    }
                }
                ValueKind::Word(word) => match (ty, word.as_str()) {
                    (Type::Bool | Type::Any, "true" | "false") => {
                        Ok(ArgValue::Value(V::Bool(word == "true")))
                    }
                    (Type::Any, "null") => Ok(ArgValue::Value(V::Null)),
                    (Type::Id(_) | Type::Str | Type::Any, _) => Ok(ArgValue::Word(word.clone())),
                    // A bare word is a bound subject, as it is in an
                    // expression: `to=self`.
                    (Type::Subject(_), _) => expression(word, at),
                    _ => Err(wrong(None)),
                },
                ValueKind::Ref(reference) => match ty {
                    Type::Any | Type::Subject(_) => Ok(ArgValue::Ref(reference.clone())),
                    _ => Err(wrong(Some(format!(
                        "to read something of it, write an expression: \"{reference}.…\""
                    )))),
                },
                ValueKind::Quoted(text) if *ty == Type::Str => Ok(ArgValue::Value(V::text(text))),
                ValueKind::Quoted(text) | ValueKind::Raw(text) => expression(text, at),
                ValueKind::Json(json) => match ty {
                    Type::Any | Type::List(_) | Type::Map(_) => Ok(ArgValue::Json(json.clone())),
                    _ => Err(wrong(None)),
                },
            }
        }
        _ => Err(wrong(None)),
    }
}

/// The built-in commands of 14 §3.3.
fn builtin_specs() -> Vec<CommandSpec> {
    use ParamSpec as P;
    use ParamType as T;
    let cmd = CommandSpec::new;
    let switch = || T::choice(["on", "off"]);
    vec![
        // Help.
        cmd("help")
            .doc("Lists the commands, or describes one.")
            .param(P::optional("command", T::Text, "The command to describe.").rest()),
        // Expressions and rules.
        cmd("eval")
            .doc("Evaluates an expression and prints its value.")
            .param(P::required("expression", T::Expr, "The expression (05 §4). It may name references.").rest()),
        cmd("do")
            .doc("Runs one effect, written as it is in a rule.")
            .param(P::required("effect", T::Json, "The effect: an object with one key, the verb (05 §10.1)."))
            .changes(),
        cmd("run")
            .doc("Runs a rule.")
            .param(P::required("rule", T::Word, "The rule's id."))
            .param(P::optional("with", T::Json, "Values for the rule's parameters (05 §9.5)."))
            .changes(),
        cmd("explain")
            .doc("Says why something is or is not available: an action, a recipe, a job (14 §5).")
            .param(P::required("what", T::Word, "The kind of thing: action, recipe, job."))
            .param(P::required("name", T::Word, "Its id."))
            .param(P::optional("subject", T::Subject, "Who would do it. The active character if left out.")),
        cmd("break")
            .doc("Marks a rule. The next time it is about to run the game pauses on it (14 §6.4).")
            .param(P::required("rule", T::Word, "The rule's id."))
            .param(P::optional("if", T::Expr, "Pause only when this holds."))
            .hosts(Hosts::INTERACTIVE),
        cmd("break off")
            .doc("Removes the mark from a rule.")
            .param(P::required("rule", T::Word, "The rule's id."))
            .hosts(Hosts::INTERACTIVE),
        cmd("next")
            .doc("At a break, shows the state after one more effect.")
            .hosts(Hosts::INTERACTIVE),
        cmd("continue")
            .doc("At a break, lets the rule finish and play go on.")
            .hosts(Hosts::INTERACTIVE),
        // Time.
        cmd("advance")
            .doc("Lets time pass.")
            .param(P::required("time", T::Duration, "How long: `2s`, `400ms`."))
            .changes(),
        cmd("step")
            .doc("Runs the next wake-ups of the timeline, one at a time.")
            .param(P::optional("count", T::Int, "How many.").default(ArgValue::Value(clash_expr::Value::Int(1))))
            .changes(),
        cmd("until")
            .doc("Lets time pass until a condition holds. Refused with `until_not_reached` if the maximum passes first.")
            .param(P::required("condition", T::Expr, "The condition, checked between events.").rest())
            .param(P::required("max", T::Duration, "The longest to wait."))
            .changes(),
        cmd("rewind")
            .doc("Goes back by a span of time, to look at the past (14 §8.3).")
            .param(P::required("time", T::Duration, "How far back."))
            .changes()
            .hosts(Hosts::INTERACTIVE),
        // Looking.
        cmd("select")
            .doc("Selects things. The selection is `@sel`.")
            .param(P::required("what", T::Word, "The kind of thing: entities, cells, items."))
            .param(P::optional("around", T::Subject, "The centre of the search."))
            .param(P::optional("radius", T::Real, "How far from the centre, in metres."))
            .param(P::optional("where", T::Expr, "A condition on each candidate, `it`.")),
        cmd("inspect")
            .doc("Lists every property of a subject (14 §4.2).")
            .param(P::optional("subject", T::Subject, "What to inspect. The selection if left out.")),
        cmd("watch")
            .doc("Pins an expression, which then stays current.")
            .param(P::required("expression", T::Expr, "The expression.").rest()),
        cmd("trace")
            .doc("Switches a trace on or off for a kind, a subject, a place or a combination (14 §6.1).")
            .param(P::required("switch", switch(), "On or off."))
            .param(P::optional("kind", T::Word, "The kind of record: rule, strike, mind."))
            .param(P::optional("subject", T::any(), "A rule's id, or a reference."))
            .param(P::optional("at", T::Subject, "A place.")),
        cmd("profile")
            .doc("Switches the profiler on or off, or prints its report (14 §6.2).")
            .param(P::required("switch", T::choice(["on", "off", "report"]), "On, off or report.")),
        cmd("view")
            .doc("Chooses whose view of the world is shown (14 §4.4).")
            .param(P::required("which", T::choice(["truth", "party", "character", "split"]), "The view."))
            .param(P::optional("of", T::Subject, "The character, for a character's view.")),
        // State.
        cmd("save")
            .doc("Keeps the whole state under a name, in memory.")
            .param(P::optional("name", T::Word, "The name.").default(ArgValue::Word("quick".to_owned()))),
        cmd("load")
            .doc("Goes back to a state kept with `save`.")
            .param(P::optional("name", T::Word, "The name.").default(ArgValue::Word("quick".to_owned())))
            .changes(),
        cmd("hash")
            .doc("Prints the state hash.")
            .param(P::optional("subsystem", T::Word, "One subsystem's hash instead of the whole.")),
        cmd("check").doc("Runs every invariant checker (14 §7.3)."),
        cmd("diff")
            .doc("Compares two states kept with `save`, piece by piece.")
            .param(P::required("a", T::Word, "One name."))
            .param(P::required("b", T::Word, "The other.")),
        // Setting up.
        cmd("use")
            .doc("Starts again from a profile and a seed.")
            .param(P::required("profile", T::Word, "The profile's name."))
            .param(P::optional("seed", T::Int, "The simulation's seed.").default(ArgValue::Value(clash_expr::Value::Int(0))))
            .changes(),
        cmd("gen")
            .doc("Builds a small world for a test to happen in.")
            .param(P::required("kind", T::Word, "The kind of world: flat."))
            .param(P::optional("size", T::Int, "How large, in the kind's own measure.").default(ArgValue::Value(clash_expr::Value::Int(1))))
            .changes(),
        cmd("find")
            .doc("Lists the definitions of a type that match a condition.")
            .param(P::required("type", T::Word, "The definition type."))
            .param(P::optional("where", T::Expr, "A condition on each definition, `it`.")),
        // The interface.
        cmd("key")
            .doc("Presses a key, as a player would.")
            .param(P::required("chord", T::Text, "The key: `enter`, `ctrl+s`.")),
        cmd("open")
            .doc("Opens a screen.")
            .param(P::required("screen", T::Word, "The screen's id.")),
        cmd("size")
            .doc("Sets the size of the presented frame.")
            .param(P::required("cols", T::Int, "Columns."))
            .param(P::required("rows", T::Int, "Rows.")),
        cmd("scene step")
            .doc("Lets presentation time pass, so that playback and effects move on.")
            .param(P::required("time", T::Duration, "How long.")),
        // Maps.
        cmd("map export")
            .doc("Writes a captured map into a mod's directory (14 §10.3).")
            .param(P::required("name", T::Word, "The captured map's name."))
            .param(P::required("mod", T::Word, "The mod to write it into.")),
        // Tests.
        cmd("assert")
            .doc("Fails unless an expression is true now.")
            .param(P::required("condition", T::Expr, "The condition.").rest()),
        cmd("expect")
            .doc("States what a test script expects (14 §7.1): `expect event <type> <fields> within=<time>`, `expect trace <kind> <fields>`, `expect reason <code>`, `expect hash <value>`, `expect frame <file>`.")
            .param(P::required("what", T::choice(["event", "trace", "reason", "hash", "frame"]), "The kind of expectation."))
            .param(P::optional("name", T::Text, "The event's type, the trace's kind, the reason's code, the hash or the file."))
            .open()
            .hosts(Hosts::FILE),
        // Mods.
        cmd("reload").doc("Reloads the mods (04 §11).").changes(),
        cmd("lint").doc("Runs the loading pipeline and prints every diagnostic."),
        cmd("show")
            .doc("Prints a definition after every patch, and where each value came from.")
            .param(P::required("type", T::Word, "The definition type."))
            .param(P::required("id", T::Word, "The definition's id.")),
        cmd("refs")
            .doc("Lists what refers to a definition, and what it refers to.")
            .param(P::required("type", T::Word, "The definition type."))
            .param(P::required("id", T::Word, "The definition's id.")),
    ]
}

#[cfg(test)]
mod tests {
    use clash_core::Real;
    use clash_expr::{KindSpec, Value};

    use super::*;
    use crate::console::parse_line;

    fn registry() -> Registry {
        let mut registry = Registry::with_builtins();
        let entity = registry.add_kind(KindSpec::new("entity")).unwrap();
        let material = registry.add_def_type("material", "A material.").unwrap();
        registry
            .add_verb(
                VerbSpec::new("damage")
                    .doc("Damages what is at a place. Ends there.")
                    .arg("at", Type::Subject(entity), "Where.")
                    .arg("energy", Type::Real, "How much, in joules.")
                    .optional_arg("kind", Type::Id(material), "The kind of damage.")
                    .optional_arg("count", Type::Int, "How many times.")
                    .optional_arg("tags", Type::list(Type::Str), "Tags.")
                    .optional_arg("flag", Type::Bool, "A flag."),
            )
            .unwrap();
        registry
            .add_verb(
                VerbSpec::new("message")
                    .doc("Puts a line in the log.")
                    .arg("to", Type::Subject(entity).nullable(), "Whose log.")
                    .arg("text", Type::Str, "The line.")
                    .direct("text"),
            )
            .unwrap();
        registry
            .add_verb(
                VerbSpec::new("run")
                    .doc("Calls a rule.")
                    .arg("eoc", Type::Str, "The rule."),
            )
            .unwrap();
        registry
    }

    fn table() -> CommandTable {
        let mut table = CommandTable::builtin();
        assert_eq!(table.add_verbs(&registry()), ["run"]);
        table
    }

    fn bind(source: &str) -> BoundCommand {
        let line = parse_line(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        table()
            .bind(&line, HostKind::File)
            .unwrap_or_else(|errors| panic!("{source}: {errors:?}"))
            .unwrap()
    }

    fn errors(source: &str) -> Vec<Diagnostic> {
        let line = parse_line(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        table().bind(&line, HostKind::File).unwrap_err()
    }

    fn fail(source: &str) -> Diagnostic {
        let mut all = errors(source);
        assert_eq!(all.len(), 1, "{source}: {all:?}");
        all.remove(0)
    }

    #[test]
    fn the_built_in_commands_are_all_registered() {
        let mut table = CommandTable::new();
        for spec in builtin_specs() {
            assert!(!spec.doc.is_empty(), "{} has no documentation", spec.name);
            for param in &spec.params {
                assert!(
                    !param.doc.is_empty(),
                    "{} {} has none",
                    spec.name,
                    param.name
                );
            }
            table
                .add(spec.clone())
                .unwrap_or_else(|error| panic!("{error}"));
        }
        // Every command 14 §3.3 names.
        for name in [
            "eval",
            "do",
            "run",
            "explain",
            "break",
            "next",
            "continue",
            "advance",
            "step",
            "until",
            "rewind",
            "select",
            "inspect",
            "watch",
            "trace",
            "profile",
            "view",
            "save",
            "load",
            "hash",
            "check",
            "diff",
            "use",
            "gen",
            "find",
            "key",
            "open",
            "size",
            "scene step",
            "map export",
            "assert",
            "expect",
            "reload",
            "lint",
            "show",
            "refs",
            "help",
        ] {
            assert!(table.get(name).is_some(), "{name} is missing");
        }
        assert_eq!(table.len(), CommandTable::builtin().len());
        assert!(!table.is_empty());
    }

    #[test]
    fn a_verb_becomes_a_command_with_its_direct_argument_first() {
        let table = table();
        let damage = table.get("damage").unwrap();
        assert_eq!(damage.source, CommandSource::Verb);
        assert!(damage.changes);
        assert_eq!(
            damage.usage(),
            "damage <at> <energy> [kind] [count] [tags] [flag]"
        );
        let message = table.get("message").unwrap();
        assert_eq!(message.usage(), "message <text> <to>");
        // The built-in `run` keeps its name.
        assert_eq!(table.get("run").unwrap().source, CommandSource::Builtin);
    }

    #[test]
    fn positional_named_and_prepositional_arguments() {
        let bound = bind("damage @here 5kJ kind=blast");
        assert_eq!(bound.arg("at"), Some(&ArgValue::Ref(Reference::Here)));
        assert_eq!(bound.value("energy"), Some(&Value::Real(Real::of(5000.0))));
        assert_eq!(bound.word("kind"), Some("blast"));
        assert_eq!(bound.arg("count"), None);
        assert_eq!(bound.args.len(), 3);

        // `at @here` is `at=@here`; the next positional is `energy`.
        let bound = bind("damage at @here 1e9 kind core:blast");
        assert_eq!(bound.arg("at"), Some(&ArgValue::Ref(Reference::Here)));
        assert_eq!(bound.value("energy"), Some(&Value::Real(Real::of(1e9))));
        assert_eq!(bound.word("kind"), Some("core:blast"));

        let bound = bind("as @g1 message \"The door creaks.\" to=self as=m");
        assert_eq!(
            bound.subject,
            Some(ArgValue::Ref(Reference::Named("g1".into())))
        );
        assert_eq!(bound.text("text"), Some("The door creaks."));
        assert_eq!(bound.expr("to").unwrap().source, "self");
        assert_eq!(bound.result_name.as_deref(), Some("m"));
    }

    #[test]
    fn values_of_verb_arguments_are_checked() {
        let bound = bind("damage @e:4 \"self.mass * 2\" count=3 tags={\"a\": 1} flag=true");
        assert_eq!(bound.expr("energy").unwrap().written, "self.mass * 2");
        assert_eq!(bound.int("count"), Some(3));
        assert!(matches!(bound.arg("tags"), Some(ArgValue::Json(_))));
        assert_eq!(bound.value("flag"), Some(&Value::Bool(true)));
        // An unquoted expression with a reference in it.
        let bound = bind("damage @e:4 @g1.mass*2");
        let energy = bound.expr("energy").unwrap();
        assert_eq!(energy.source, "$at0.mass*2");
        assert_eq!(energy.locals[0].reference, Reference::Named("g1".into()));

        for (source, param) in [
            ("damage 12 5", "at"),
            ("damage @here high", "energy"),
            ("damage @here @here", "energy"),
            ("damage @here 1 count=1.5", "count"),
            ("damage @here 1 count=2s", "count"),
            ("damage @here 1 flag=maybe", "flag"),
            ("damage @here 1 kind=3", "kind"),
            ("damage @here {\"a\": 1}", "energy"),
        ] {
            let error = fail(source);
            assert_eq!(error.code, codes::WRONG_TYPE, "{source}");
            assert!(
                error.message.contains(&format!("`{param}`")),
                "{}",
                error.message
            );
        }
        let error = fail("damage @here \"1 +\"");
        assert_eq!(error.code, codes::BAD_EXPRESSION);
        assert_eq!(error.span, Span::new(13, 18));
    }

    #[test]
    fn built_in_parameter_types() {
        assert_eq!(
            bind("advance 400ms").dur("time"),
            Some(Dur::from_millis(400))
        );
        assert_eq!(bind("advance 2").dur("time"), Some(Dur::from_secs(2)));
        assert_eq!(bind("advance 1.5min").dur("time"), Some(Dur::from_secs(90)));
        let error = fail("advance 2sec");
        assert_eq!(error.code, codes::WRONG_TYPE);
        assert_eq!(
            error.suggestion.as_deref(),
            Some("`sec` is not a unit; did you mean `s`?")
        );
        assert!(
            fail("advance 2kg")
                .suggestion
                .unwrap()
                .contains("not a unit of time")
        );
        assert_eq!(fail("advance soon").code, codes::WRONG_TYPE);

        assert_eq!(bind("step").int("count"), Some(1));
        assert_eq!(bind("step 5").int("count"), Some(5));
        assert_eq!(bind("use test_core").int("seed"), Some(0));
        let bound = bind("use profile=test_core seed=7");
        assert_eq!(
            (bound.word("profile"), bound.int("seed")),
            (Some("test_core"), Some(7))
        );
        assert!(bound.changes);

        assert_eq!(
            bind("trace on rule vault_alarm").word("subject"),
            Some("vault_alarm")
        );
        assert_eq!(
            bind("trace off strike @e:41").arg("subject"),
            Some(&ArgValue::Ref(Reference::Entity {
                slot: 41,
                generation: None
            }))
        );
        let error = fail("trace onn");
        assert_eq!(error.suggestion.as_deref(), Some("did you mean `on`?"));

        let bound = bind("explain action slash @self");
        assert_eq!(
            bound.arg("subject"),
            Some(&ArgValue::Ref(Reference::Active))
        );
        assert!(
            fail("explain action slash guard")
                .suggestion
                .unwrap()
                .contains("@guard")
        );

        let bound = bind("select entities around=@self radius=10 where=\"it.has_tag('guard')\"");
        assert_eq!(bound.value("radius"), Some(&Value::Real(Real::of(10.0))));
        assert_eq!(bound.expr("where").unwrap().source, "it.has_tag('guard')");

        assert!(matches!(
            bind("do { \"stop\": {} }").arg("effect"),
            Some(ArgValue::Json(_))
        ));
        assert!(fail("do stop").suggestion.unwrap().contains("braces"));
        assert_eq!(bind("save").word("name"), Some("quick"));
        assert_eq!(bind("save before_fight").word("name"), Some("before_fight"));
        assert_eq!(bind("key ctrl+s").text("chord"), Some("ctrl+s"));
        assert_eq!(bind("size 80 24").int("rows"), Some(24));
    }

    #[test]
    fn the_rest_of_the_line_is_one_expression() {
        assert_eq!(
            bind("eval self.pool.blood.frac")
                .expr("expression")
                .unwrap()
                .source,
            "self.pool.blood.frac"
        );
        assert_eq!(
            bind("eval 1 + 2 == 3").expr("expression").unwrap().source,
            "1 + 2 == 3"
        );
        assert_eq!(
            bind("watch dist(self, @sel)")
                .expr("expression")
                .unwrap()
                .source,
            "dist(self, $at0)"
        );
        assert_eq!(
            bind("eval \"1 + 2\"").expr("expression").unwrap().source,
            "1 + 2"
        );
        let bound = bind("until \"party.count == 0\" max=5min");
        assert_eq!(bound.dur("max"), Some(Dur::from_secs(300)));
        let bound = bind("until party.count == 0 max 5min");
        assert_eq!(bound.expr("condition").unwrap().source, "party.count == 0");
        assert_eq!(bound.dur("max"), Some(Dur::from_secs(300)));
        assert_eq!(fail("eval 1 +").code, codes::BAD_EXPRESSION);
        assert_eq!(bind("help map export").text("command"), Some("map export"));
    }

    #[test]
    fn two_word_commands() {
        let bound = bind("map export tower my_mod");
        assert_eq!(bound.name, "map export");
        assert_eq!(
            (bound.word("name"), bound.word("mod")),
            (Some("tower"), Some("my_mod"))
        );
        assert_eq!(bind("scene step 200ms").name, "scene step");
        let error = fail("map import x");
        assert_eq!(error.code, codes::UNKNOWN_COMMAND);
    }

    #[test]
    fn unknown_missing_duplicate_and_surplus_arguments() {
        let error = fail("advnce 2s");
        assert_eq!(error.code, codes::UNKNOWN_COMMAND);
        assert_eq!(error.span, Span::new(0, 6));
        assert_eq!(error.suggestion.as_deref(), Some("did you mean `advance`?"));
        assert_eq!(
            fail("frobnicate").suggestion.as_deref(),
            Some("`help` lists the commands")
        );

        let error = fail("damage @here 1 knd=blast");
        assert_eq!(error.code, codes::UNKNOWN_ARGUMENT);
        assert_eq!(error.span, Span::new(15, 18));
        assert_eq!(error.suggestion.as_deref(), Some("did you mean `kind`?"));
        assert!(
            fail("check zzz=1")
                .suggestion
                .unwrap()
                .contains("it is typed `check`")
        );

        let error = fail("damage @here");
        assert_eq!(error.code, codes::MISSING_ARGUMENT);
        assert!(error.message.contains("`energy`"), "{}", error.message);
        assert!(error.suggestion.unwrap().contains("damage <at> <energy>"));
        assert_eq!(errors("damage").len(), 2);

        let error = fail("damage @here 1 at=@self");
        assert_eq!(error.code, codes::DUPLICATE_ARGUMENT);
        assert_eq!(
            fail("damage @here 1 as=a as=b").code,
            codes::DUPLICATE_ARGUMENT
        );

        let error = fail("check 1");
        assert_eq!(error.code, codes::TOO_MANY_ARGUMENTS);
        assert!(error.message.contains("no arguments"));
        assert_eq!(fail("advance 1s 2s").code, codes::TOO_MANY_ARGUMENTS);

        for name in ["here", "self", "1x", "\"a b\"", "@g"] {
            assert_eq!(
                fail(&format!("check as={name}")).code,
                codes::BAD_NAME,
                "{name}"
            );
        }
        // Several mistakes are reported together.
        let all = errors("damage 12 high knd=1");
        let found: Vec<Code> = all.iter().map(|error| error.code).collect();
        assert_eq!(
            found,
            [
                codes::WRONG_TYPE,
                codes::WRONG_TYPE,
                codes::UNKNOWN_ARGUMENT
            ]
        );
    }

    use crate::console::diag::Code;

    #[test]
    fn hosts_offer_what_they_are_registered_for() {
        let table = table();
        let line = parse_line("expect hash 00").unwrap();
        assert!(table.bind(&line, HostKind::File).is_ok());
        let errors = table.bind(&line, HostKind::Game).unwrap_err();
        assert_eq!(errors[0].code, codes::NOT_OFFERED);
        assert!(errors[0].message.contains("files"));
        let line = parse_line("next").unwrap();
        assert!(table.bind(&line, HostKind::Viewer).is_ok());
        assert_eq!(
            table.bind(&line, HostKind::File).unwrap_err()[0].code,
            codes::NOT_OFFERED
        );
        assert_eq!(
            table.bind(&parse_line("# only").unwrap(), HostKind::File),
            Ok(None)
        );
    }

    #[test]
    fn open_commands_take_fields() {
        let bound = bind("expect event strike_defended target=@b turned=162 within=400ms");
        assert_eq!(bound.word("what"), Some("event"));
        assert_eq!(bound.text("name"), Some("strike_defended"));
        assert_eq!(bound.fields.len(), 3);
        assert_eq!(
            bound.fields[0],
            (
                "target".to_owned(),
                ArgValue::Ref(Reference::Named("b".into()))
            )
        );
        assert_eq!(bound.fields[1].1, ArgValue::Value(Value::Int(162)));
    }

    #[test]
    fn help_is_generated_and_sorted() {
        let table = table();
        let help = table.help("until").unwrap();
        assert_eq!(
            help,
            "until <condition…> <max>\n    \
             Lets time pass until a condition holds. Refused with `until_not_reached` if the maximum passes first.\n    \
             condition  an expression. The condition, checked between events.\n    \
             max        a time. The longest to wait.\n    \
             Changes the simulation. Offered in the game, the viewer and files.\n"
        );
        let help = table.help("damage").unwrap();
        assert!(
            help.contains("kind    a value of type id<material>, optional. The kind of damage."),
            "{help}"
        );
        assert!(
            table
                .help("step")
                .unwrap()
                .contains("a whole number, default 1. How many.")
        );
        assert!(table.help("nothing").is_none());

        let all = table.help_all();
        let names: Vec<&str> = table.commands().map(|spec| spec.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert_eq!(all.lines().count(), table.len());
        let damage = all
            .lines()
            .find(|line| line.starts_with("damage "))
            .unwrap();
        assert!(damage.ends_with("Damages what is at a place."), "{damage}");
        let first = all.lines().next().unwrap();
        assert!(first.starts_with("advance <time>"), "{first}");
    }

    #[test]
    fn registrations_the_console_cannot_bind_are_refused() {
        let mut table = CommandTable::new();
        table.add(CommandSpec::new("a")).unwrap();
        assert_eq!(
            table.add(CommandSpec::new("a")),
            Err(TableError::Duplicate("a".into()))
        );
        for spec in [
            CommandSpec::new("a b c"),
            CommandSpec::new(""),
            CommandSpec::new("1x"),
            CommandSpec::new("b").param(ParamSpec::required("x y", ParamType::Int, "")),
            CommandSpec::new("b")
                .param(ParamSpec::required("x", ParamType::Int, ""))
                .param(ParamSpec::required("x", ParamType::Int, "")),
            CommandSpec::new("b").param(ParamSpec::required("x", ParamType::Int, "").rest()),
        ] {
            assert!(matches!(table.add(spec), Err(TableError::BadSpec { .. })));
        }
    }
}
