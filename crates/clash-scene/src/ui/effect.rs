//! Interface effects (12 §10.5): what keys, clicks and widget events
//! run. A list of effects is read into typed [`UiEffect`]s when its
//! definition loads, so an effect that does not exist or an argument it
//! does not take is a load error where it was written, and its
//! expressions are compiled with the rest of the screen's bindings.
//!
//! | Effect | Written | Does |
//! |--------|---------|------|
//! | `open` | `"id"`, or `{ "screen", "with" }` | Opens a screen on top of the stack, with values for its parameters |
//! | `close` | `{}` | Closes the top screen |
//! | `replace` | as `open` | Closes the top screen and opens another in its place |
//! | `set` | `{ "screen.name": value, "pref.id": value }` | Changes screen variables and preferences |
//! | `focus` | `"widget id"` | Moves the focus |
//! | `confirm` | `"text"`, or `{ "text", "yes", "no" }` | Asks; the rest of the list runs only on yes |
//! | `pick` | `{ "from", "item", "into", "title" }` | Opens a picker; the rest of the list runs with the choice as `it` |
//! | `answer` | value | Closes a screen opened by `confirm` or `pick`, giving its answer |
//! | `act` | `{ "action", "params" }` | Submits an action for the active character |
//! | `command` | `"name"`, or `{ "name", "args" }` | Submits a simulation command |
//! | `run` | `"rule"`, or `{ "rule", "args" }` | Runs a `ui_callable` rule through the `Ui` command |
//! | `app` | `"service"`, or `{ "service": { args } }` | An engine service: see [`AppService`] |
//!
//! A value is `true`, `false`, a number, or a string holding an
//! expression: a text is written `"'like this'"`.

use core::fmt;
use std::collections::BTreeMap;

use clash_expr::Type;
use serde::Deserialize;
use serde::de::value::MapAccessDeserializer;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};

use super::bind::{BindCx, Binding, ItType, element_type};
use super::def::{Literal, Prop, TextProp};
use super::screen::VarRef;

/// A value in an effect: a literal, or an expression.
pub type ValueProp = Prop<Literal>;

/// The values an effect is given by name: the parameters of a screen,
/// the arguments of a command.
pub type Args = BTreeMap<String, ValueProp>;

/// Which screen to open, and what with.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct OpenEffect {
    /// The screen: a qualified id, or a bare name, which means the
    /// screen of that name in the namespace of the screen that asks, or
    /// else the first of that name in order of qualified id.
    pub screen: String,
    /// The values of the opened screen's parameters, by name.
    #[serde(default)]
    pub with: Args,
}

/// `confirm`: a question to ask before the rest of a list runs.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ConfirmEffect {
    /// The question.
    pub text: TextProp,
    /// What the answer that goes on is called. Left out, the confirm
    /// screen's own word.
    #[serde(default)]
    pub yes: Option<TextProp>,
    /// What the answer that stops is called.
    #[serde(default)]
    pub no: Option<TextProp>,
}

/// `pick`: a choice among a list of values.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickEffect {
    /// An expression giving the list to choose from.
    pub from: Binding,
    /// How each entry is shown: a text over the entry, `it`. Left out,
    /// the entry as text.
    #[serde(default)]
    pub item: Option<TextProp>,
    /// The screen variable or preference the choice is stored in. It
    /// must be able to hold it: a flag, a number or a text.
    #[serde(default)]
    pub into: Option<String>,
    /// The picker's title.
    #[serde(default)]
    pub title: Option<TextProp>,
}

/// How the parameters of an action are gathered (12 §9.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Collect {
    /// The effect gives them; the action's defaults fill in the rest.
    #[default]
    Given,
    /// `"needed"`: ask only for those with no default.
    Needed,
    /// `"all"`: ask for every one, each filled in with its default.
    All,
}

/// Which command of the simulation an effect builds (03 §4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandKind {
    /// `act`: an action of the active character.
    Act,
    /// `command`: any other command, by name.
    Command,
    /// `run`: the `Ui` command, which runs a `ui_callable` rule.
    Rule,
}

impl CommandKind {
    /// The effect that builds this kind of command.
    pub const fn effect(self) -> &'static str {
        match self {
            CommandKind::Act => "act",
            CommandKind::Command => "command",
            CommandKind::Rule => "run",
        }
    }
}

/// `act`, `command` or `run`: a command to build and submit.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandEffect {
    /// Which command.
    pub kind: CommandKind,
    /// The action, the command or the rule.
    pub name: String,
    /// Its arguments, by name.
    pub args: Args,
    /// For `act`: how its parameters are gathered.
    pub collect: Collect,
}

/// An engine service (12 §10.5), asked for with the `app` effect. The
/// scene changes a preference and a key binding itself; every service is
/// then handed to the application through
/// [`AppState::service`](crate::AppState::service).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AppService {
    /// `new_game`: starts a game. Arguments: `scenario` (optional), the
    /// scenario's id.
    NewGame,
    /// `save`: saves the game. Arguments: `name` (optional).
    Save,
    /// `load`: loads a save. Arguments: `save`, its id.
    Load,
    /// `delete_save`: deletes a save. Arguments: `save`, its id.
    DeleteSave,
    /// `quit`: leaves the program.
    Quit,
    /// `set_pref`: changes a preference. Arguments: `pref`, its id, and
    /// `value`.
    SetPref,
    /// `rebind`: gives a key another meaning. Arguments: `context`,
    /// `key` and `name`, as a keymap writes them.
    Rebind,
    /// `select_profile`: chooses the profile to start. Arguments:
    /// `profile`, its id.
    SelectProfile,
    /// `reload_mods`: reads the mods again.
    ReloadMods,
    /// `edit_creation`: changes the character being made. Arguments:
    /// `field` and `value`.
    EditCreation,
}

impl AppService {
    /// Every service, in the order of 12 §10.5.
    pub const ALL: [AppService; 10] = [
        AppService::NewGame,
        AppService::Save,
        AppService::Load,
        AppService::DeleteSave,
        AppService::Quit,
        AppService::SetPref,
        AppService::Rebind,
        AppService::SelectProfile,
        AppService::ReloadMods,
        AppService::EditCreation,
    ];

    /// The name the service is asked for by.
    pub const fn name(self) -> &'static str {
        match self {
            AppService::NewGame => "new_game",
            AppService::Save => "save",
            AppService::Load => "load",
            AppService::DeleteSave => "delete_save",
            AppService::Quit => "quit",
            AppService::SetPref => "set_pref",
            AppService::Rebind => "rebind",
            AppService::SelectProfile => "select_profile",
            AppService::ReloadMods => "reload_mods",
            AppService::EditCreation => "edit_creation",
        }
    }

    /// The service's arguments: each name and whether it must be given.
    pub const fn args(self) -> &'static [(&'static str, bool)] {
        match self {
            AppService::NewGame => &[("scenario", false)],
            AppService::Save => &[("name", false)],
            AppService::Load | AppService::DeleteSave => &[("save", true)],
            AppService::Quit | AppService::ReloadMods => &[],
            AppService::SetPref => &[("pref", true), ("value", true)],
            AppService::Rebind => &[("context", true), ("key", true), ("name", true)],
            AppService::SelectProfile => &[("profile", true)],
            AppService::EditCreation => &[("field", true), ("value", true)],
        }
    }

    fn parse(name: &str) -> Option<AppService> {
        AppService::ALL
            .into_iter()
            .find(|service| service.name() == name)
    }
}

/// `app`: an engine service and its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct AppEffect {
    /// The service.
    pub service: AppService,
    /// Its arguments, by name.
    pub args: Args,
}

/// One interface effect (12 §10.5). In JSON it is an object with one
/// key, the effect's name.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEffect {
    /// `open`.
    Open(OpenEffect),
    /// `close`.
    Close,
    /// `replace`.
    Replace(OpenEffect),
    /// `set`: each variable and its new value, in order of name.
    Set(Vec<(String, ValueProp)>),
    /// `focus`: the id of the widget to focus.
    Focus(String),
    /// `confirm`.
    Confirm(Box<ConfirmEffect>),
    /// `pick`.
    Pick(Box<PickEffect>),
    /// `answer`.
    Answer(ValueProp),
    /// `act`, `command` and `run`.
    Submit(CommandEffect),
    /// `app`.
    App(AppEffect),
}

/// The names of the effects, in the order of 12 §10.5.
pub const EFFECT_NAMES: &[&str] = &[
    "open", "close", "replace", "set", "focus", "pick", "act", "command", "run", "confirm",
    "answer", "app",
];

impl UiEffect {
    /// The effect's name in data.
    pub fn name(&self) -> &'static str {
        match self {
            UiEffect::Open(_) => "open",
            UiEffect::Close => "close",
            UiEffect::Replace(_) => "replace",
            UiEffect::Set(_) => "set",
            UiEffect::Focus(_) => "focus",
            UiEffect::Confirm(_) => "confirm",
            UiEffect::Pick(_) => "pick",
            UiEffect::Answer(_) => "answer",
            UiEffect::Submit(command) => command.kind.effect(),
            UiEffect::App(_) => "app",
        }
    }
}

/// Implements `Deserialize` for a type written as a bare string or as
/// an object, whose object form is derived with `remote = "Self"`.
macro_rules! text_or_object {
    ($ty:ident, $expecting:literal, $text:ident => $from:expr) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$ty, D::Error> {
                struct Shorthand;
                impl<'de> Visitor<'de> for Shorthand {
                    type Value = $ty;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str($expecting)
                    }
                    fn visit_str<E: de::Error>(self, $text: &str) -> Result<$ty, E> {
                        Ok($from)
                    }
                    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<$ty, A::Error> {
                        $ty::deserialize(MapAccessDeserializer::new(map))
                    }
                }
                deserializer.deserialize_any(Shorthand)
            }
        }
    };
}

text_or_object!(OpenEffect, "a screen's id, or an object with `screen` and `with`",
    text => OpenEffect { screen: text.to_owned(), with: Args::new() });

text_or_object!(ConfirmEffect, "a question, or an object with `text`, `yes` and `no`",
    text => ConfirmEffect { text: super::def::text_prop(text), yes: None, no: None });

/// `{}`: what `close` is given.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

/// The parameters of an `act`: given by name, or `"needed"` or `"all"`.
enum ActParams {
    Given(Args),
    Ask(Collect),
}

impl<'de> Deserialize<'de> for ActParams {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ActParams, D::Error> {
        struct ParamsVisitor;
        impl<'de> Visitor<'de> for ParamsVisitor {
            type Value = ActParams;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the action's parameters by name, or `\"needed\"` or `\"all\"`")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<ActParams, E> {
                match text {
                    "needed" => Ok(ActParams::Ask(Collect::Needed)),
                    "all" => Ok(ActParams::Ask(Collect::All)),
                    other => Err(E::invalid_value(de::Unexpected::Str(other), &self)),
                }
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<ActParams, A::Error> {
                Args::deserialize(MapAccessDeserializer::new(map)).map(ActParams::Given)
            }
        }
        deserializer.deserialize_any(ParamsVisitor)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActBody {
    action: String,
    #[serde(default)]
    params: Option<ActParams>,
}

#[derive(Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
struct CommandBody {
    name: String,
    #[serde(default)]
    args: Args,
}

text_or_object!(CommandBody, "a command's name, or an object with `name` and `args`",
    text => CommandBody { name: text.to_owned(), args: Args::new() });

#[derive(Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
struct RunBody {
    rule: String,
    #[serde(default)]
    args: Args,
}

text_or_object!(RunBody, "a rule's id, or an object with `rule` and `args`",
    text => RunBody { rule: text.to_owned(), args: Args::new() });

impl<'de> Deserialize<'de> for AppEffect {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<AppEffect, D::Error> {
        struct AppVisitor;
        fn service<E: de::Error>(name: &str) -> Result<AppService, E> {
            AppService::parse(name).ok_or_else(|| {
                let known: Vec<&str> = AppService::ALL.iter().map(|s| s.name()).collect();
                E::custom(format!(
                    "unknown service `{name}` (known: {})",
                    known.join(", ")
                ))
            })
        }
        fn checked<E: de::Error>(service: AppService, args: Args) -> Result<AppEffect, E> {
            let known = service.args();
            for name in args.keys() {
                if !known.iter().any(|(known, _)| known == name) {
                    let names: Vec<&str> = known.iter().map(|(name, _)| *name).collect();
                    return Err(E::custom(format!(
                        "`{}` takes no argument `{name}` (it takes: {})",
                        service.name(),
                        if names.is_empty() {
                            "none".to_owned()
                        } else {
                            names.join(", ")
                        }
                    )));
                }
            }
            for (name, required) in known {
                if *required && !args.contains_key(*name) {
                    return Err(E::custom(format!(
                        "`{}` needs the argument `{name}`",
                        service.name()
                    )));
                }
            }
            Ok(AppEffect { service, args })
        }
        impl<'de> Visitor<'de> for AppVisitor {
            type Value = AppEffect;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a service's name, or an object with one service and its arguments")
            }
            fn visit_str<E: de::Error>(self, name: &str) -> Result<AppEffect, E> {
                checked(service(name)?, Args::new())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<AppEffect, A::Error> {
                let Some(name) = map.next_key::<String>()? else {
                    return Err(de::Error::custom("`app` needs the name of a service"));
                };
                let service = service(&name)?;
                let args = map.next_value::<Args>()?;
                if map.next_key::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(
                        "`app` asks for one service at a time: write one `app` effect for each",
                    ));
                }
                checked(service, args)
            }
        }
        deserializer.deserialize_any(AppVisitor)
    }
}

impl<'de> Deserialize<'de> for UiEffect {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<UiEffect, D::Error> {
        struct EffectVisitor;
        impl<'de> Visitor<'de> for EffectVisitor {
            type Value = UiEffect;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an effect: an object with one key, the effect's name")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<UiEffect, A::Error> {
                let Some(name) = map.next_key::<String>()? else {
                    return Err(de::Error::custom(
                        "an effect needs one key that names it, such as `close` or `set`",
                    ));
                };
                let effect = match name.as_str() {
                    "open" => UiEffect::Open(map.next_value()?),
                    "replace" => UiEffect::Replace(map.next_value()?),
                    "close" => {
                        map.next_value::<Nothing>()?;
                        UiEffect::Close
                    }
                    "set" => UiEffect::Set(map.next_value::<Args>()?.into_iter().collect()),
                    "focus" => UiEffect::Focus(map.next_value()?),
                    "confirm" => UiEffect::Confirm(Box::new(map.next_value()?)),
                    "pick" => UiEffect::Pick(Box::new(map.next_value()?)),
                    "answer" => UiEffect::Answer(map.next_value()?),
                    "act" => {
                        let body = map.next_value::<ActBody>()?;
                        let (args, collect) = match body.params {
                            None => (Args::new(), Collect::Given),
                            Some(ActParams::Given(args)) => (args, Collect::Given),
                            Some(ActParams::Ask(collect)) => (Args::new(), collect),
                        };
                        UiEffect::Submit(CommandEffect {
                            kind: CommandKind::Act,
                            name: body.action,
                            args,
                            collect,
                        })
                    }
                    "command" => {
                        let body = map.next_value::<CommandBody>()?;
                        UiEffect::Submit(CommandEffect {
                            kind: CommandKind::Command,
                            name: body.name,
                            args: body.args,
                            collect: Collect::Given,
                        })
                    }
                    "run" => {
                        let body = map.next_value::<RunBody>()?;
                        UiEffect::Submit(CommandEffect {
                            kind: CommandKind::Rule,
                            name: body.rule,
                            args: body.args,
                            collect: Collect::Given,
                        })
                    }
                    "app" => UiEffect::App(map.next_value()?),
                    other => {
                        return Err(de::Error::custom(format!(
                            "unknown effect `{other}` (known: {})",
                            EFFECT_NAMES.join(", ")
                        )));
                    }
                };
                if map.next_key::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(format!(
                        "an effect is one thing, but this one is `{name}` and something else: write one object for each",
                    )));
                }
                Ok(effect)
            }
        }
        deserializer.deserialize_map(EffectVisitor)
    }
}

/// A list of interface effects (12 §10.5), run in order.
#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
#[serde(transparent)]
pub struct UiEffects(pub Vec<UiEffect>);

impl UiEffects {
    /// Whether the list has no effects.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The name of each effect in the list, such as `close`.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().map(UiEffect::name)
    }

    /// Compiles the expressions of every effect in the list. `it` is
    /// whatever the list's event binds; after a `pick` it is the choice.
    pub(crate) fn compile(&mut self, bx: &mut BindCx<'_, '_>) {
        let before = bx.it.clone();
        for effect in &mut self.0 {
            match effect {
                UiEffect::Open(open) | UiEffect::Replace(open) => compile_args(&mut open.with, bx),
                UiEffect::Close | UiEffect::Focus(_) => {}
                UiEffect::Set(entries) => {
                    for (name, value) in entries.iter_mut() {
                        compile_value(value, bx);
                        check_target(name, value, bx);
                    }
                }
                UiEffect::Confirm(confirm) => {
                    confirm.text.compile(bx);
                    if let Some(yes) = &mut confirm.yes {
                        yes.compile(bx);
                    }
                    if let Some(no) = &mut confirm.no {
                        no.compile(bx);
                    }
                }
                UiEffect::Pick(pick) => {
                    pick.from.compile_expr(bx, &Type::list(Type::Any));
                    let it = ItType {
                        ty: element_type(pick.from.ty()),
                        own: pick.from.is_own(),
                    };
                    bx.with_it(Some(it.clone()), |bx| {
                        if let Some(item) = &mut pick.item {
                            item.compile(bx);
                        }
                    });
                    if let Some(title) = &mut pick.title {
                        title.compile(bx);
                    }
                    if let Some(into) = &pick.into
                        && let VarRef::Screen(name) = VarRef::parse(into)
                        && let Some(vars) = &bx.vars
                        && !vars.contains_key(&name)
                    {
                        let site = pick.from.site().clone();
                        bx.mistake(
                            &site,
                            format!("the screen has no variable `{name}` to pick into: declare it in `vars`"),
                        );
                    }
                    // What follows runs with the choice as `it`.
                    bx.it = Some(it);
                }
                UiEffect::Answer(value) => compile_value(value, bx),
                UiEffect::Submit(command) => compile_args(&mut command.args, bx),
                UiEffect::App(app) => compile_args(&mut app.args, bx),
            }
        }
        bx.it = before;
    }
}

fn compile_value(value: &mut ValueProp, bx: &mut BindCx<'_, '_>) {
    if let Prop::Bound(binding) = value {
        binding.compile_expr(bx, &Type::Any);
    }
}

fn compile_args(args: &mut Args, bx: &mut BindCx<'_, '_>) {
    for value in args.values_mut() {
        compile_value(value, bx);
    }
}

/// Checks that what a `set` assigns to is a variable the screen has.
fn check_target(name: &str, value: &ValueProp, bx: &mut BindCx<'_, '_>) {
    let VarRef::Screen(var) = VarRef::parse(name) else {
        return;
    };
    let Some(vars) = &bx.vars else { return };
    if vars.contains_key(&var) {
        return;
    }
    let message = format!("the screen has no variable `{var}` to set: declare it in `vars`");
    match value {
        Prop::Bound(binding) => {
            let site = binding.site().clone();
            bx.mistake(&site, message);
        }
        Prop::Static(_) => bx.mistake(&clash_data::Site::default(), message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effects(json: &str) -> Result<UiEffects, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn every_effect_is_read_into_its_type() {
        let list = effects(
            r#"[ { "open": "inventory" },
                 { "open": { "screen": "condition", "with": { "who": "view" } } },
                 { "close": {} },
                 { "replace": "title" },
                 { "set": { "screen.tab": "(screen.tab + 1) % 3", "pref.sound": false } },
                 { "focus": "name" },
                 { "confirm": "Really?" },
                 { "pick": { "from": "party", "item": "{it.name}", "into": "who" } },
                 { "answer": true },
                 { "act": { "action": "step", "params": { "dir": "'west'" } } },
                 { "act": { "action": "attack", "params": "needed" } },
                 { "command": "wait" },
                 { "run": { "rule": "ashlands:ring_bell", "args": { "times": 3 } } },
                 { "app": "quit" },
                 { "app": { "load": { "save": "it.id" } } } ]"#,
        )
        .unwrap();
        let names: Vec<&str> = list.names().collect();
        assert_eq!(
            names,
            [
                "open", "open", "close", "replace", "set", "focus", "confirm", "pick", "answer",
                "act", "act", "command", "run", "app", "app"
            ]
        );
        let UiEffect::Open(open) = &list.0[1] else {
            panic!("open")
        };
        assert_eq!(open.screen, "condition");
        assert_eq!(open.with["who"], Prop::Bound("view".into()));
        let UiEffect::Set(entries) = &list.0[4] else {
            panic!("set")
        };
        assert_eq!(entries[0].0, "pref.sound");
        assert_eq!(
            entries[0].1,
            Prop::Static(Literal(clash_core::Value::Bool(false)))
        );
        let UiEffect::Submit(act) = &list.0[10] else {
            panic!("act")
        };
        assert_eq!((act.kind, act.collect), (CommandKind::Act, Collect::Needed));
        let UiEffect::Submit(run) = &list.0[12] else {
            panic!("run")
        };
        assert_eq!(run.name, "ashlands:ring_bell");
        assert!(run.args.contains_key("times"));
        let UiEffect::App(app) = &list.0[14] else {
            panic!("app")
        };
        assert_eq!(app.service, AppService::Load);
    }

    #[test]
    fn a_wrong_effect_says_what_is_wrong() {
        let wrong = |json: &str| effects(json).unwrap_err().to_string();
        assert!(wrong(r#"[ { "cloze": {} } ]"#).contains("unknown effect `cloze`"));
        assert!(wrong(r#"[ { "close": { "now": true } } ]"#).contains("unknown field `now`"));
        assert!(wrong(r#"[ { "close": {}, "open": "x" } ]"#).contains("one thing"));
        assert!(wrong(r#"[ { "app": "explode" } ]"#).contains("unknown service `explode`"));
        assert!(wrong(r#"[ { "app": { "load": {} } } ]"#).contains("needs the argument `save`"));
        assert!(
            wrong(r#"[ { "app": { "quit": { "hard": true } } } ]"#)
                .contains("takes no argument `hard`")
        );
        assert!(wrong(r#"[ { "act": { "action": "step", "params": "some" } } ]"#).contains("needed"));
        assert!(wrong(r#"[ { "open": { "scren": "x" } } ]"#).contains("scren"));
        assert!(wrong(r#"[ "close" ]"#).contains("an effect"));
        for service in AppService::ALL {
            assert_eq!(AppService::parse(service.name()), Some(service));
        }
    }
}
