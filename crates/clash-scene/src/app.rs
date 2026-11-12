//! `app`: what bindings read and effects ask of the world outside the
//! simulation (12 §10.4, §10.5).
//!
//! The scene answers every `app.*` name itself. What only the
//! application knows (the saves, the profiles, the mods, the scenarios,
//! the saving policy, the character being made) it asks of an
//! [`AppState`], which the application supplies; the grid's size and
//! the preferences are the scene's own.
//!
//! [`register_expr`] puts the names into the expression registry, so
//! that a binding that misspells one is a load error and the generated
//! reference lists them:
//!
//! | `app.` | Type | Is |
//! |--------|------|----|
//! | `saves`, `profiles`, `mods`, `scenarios` | `list<app_entry>` | What there is of each |
//! | `profile`, `scenario` | `app_entry?` | The ones in use |
//! | `pref(id)` | `any` | A preference's value |
//! | `creation` | `any` | The character being made (06 §10.3) |
//! | `templates` | `list<app_entry>` | The templates a character may be made from |
//! | `keys(context)` | `list<map>` | What each name of a keymap context is bound to |
//! | `size` | `list<int>` | The grid, as `[columns, rows]` |
//! | `can_save`, `can_load` | `bool` | Whether the scenario's saving policy allows it now |
//!
//! An `app_entry` has `id`, `name`, `description` and `state`.

use std::collections::BTreeMap;

use clash_core::{Value, VarMap};
use clash_expr::{
    Handle, KindId, KindSpec, PropertyId, PropertySpec, Registry, Type, Value as ExprValue,
};

use crate::defs::UiDefs;
use crate::input::KeymapDef;
use crate::ui::{AppService, KIND_APP, KIND_APP_ENTRY, from_var};

/// The definition type whose ids `app.pref` takes.
pub(crate) const PREFERENCE_TYPE: &str = "preference";

/// One entry of a list the application offers: a save, a profile, a mod
/// or a scenario.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AppEntry {
    /// What the entry is known by: a save's file name, a qualified id.
    pub id: String,
    /// The name shown to the player.
    pub name: String,
    /// A line or two about it.
    pub description: String,
    /// Its state, in a word: `enabled`, `disabled`, `broken`, `current`.
    pub state: String,
}

/// A list the application offers through `app`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AppList {
    /// `app.saves`.
    Saves,
    /// `app.profiles`.
    Profiles,
    /// `app.mods`.
    Mods,
    /// `app.scenarios`.
    Scenarios,
}

impl AppList {
    /// Every list.
    pub const ALL: [AppList; 4] = [
        AppList::Saves,
        AppList::Profiles,
        AppList::Mods,
        AppList::Scenarios,
    ];

    /// The name the list is read by.
    pub const fn name(self) -> &'static str {
        match self {
            AppList::Saves => "saves",
            AppList::Profiles => "profiles",
            AppList::Mods => "mods",
            AppList::Scenarios => "scenarios",
        }
    }

    const fn code(self) -> u64 {
        self as u64
    }
}

/// What is in use, of the things the application offers lists of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AppCurrent {
    /// `app.profile`.
    Profile,
    /// `app.scenario`.
    Scenario,
}

impl AppCurrent {
    /// The name it is read by.
    pub const fn name(self) -> &'static str {
        match self {
            AppCurrent::Profile => "profile",
            AppCurrent::Scenario => "scenario",
        }
    }

    const fn code(self) -> u64 {
        AppList::ALL.len() as u64 + self as u64
    }
}

/// An engine service asked for with the `app` effect, with its arguments
/// worked out.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRequest {
    /// The service.
    pub service: AppService,
    /// Its arguments, by name.
    pub args: BTreeMap<String, ExprValue>,
}

impl AppRequest {
    /// An argument as text, if it was given as one.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.args.get(name).and_then(ExprValue::as_str)
    }
}

/// What the scene asks of the application (12 §10.4, §10.5): the lists
/// and flags `app` reads, and the services the `app` effect asks for.
///
/// Nothing here is the simulation's. An application that has none of it
/// yet uses [`NoApp`].
pub trait AppState {
    /// The entries of one of the lists: `app.saves`, `app.profiles`,
    /// `app.mods`, `app.scenarios`.
    fn entries(&self, list: AppList) -> Vec<AppEntry>;

    /// The profile or the scenario in use, if there is one:
    /// `app.profile`, `app.scenario`.
    fn current(&self, which: AppCurrent) -> Option<AppEntry>;

    /// Whether the scenario's saving policy allows saving now
    /// (13 §9.2): `app.can_save`.
    fn can_save(&self) -> bool {
        false
    }

    /// Whether the scenario's saving policy allows loading now:
    /// `app.can_load`.
    fn can_load(&self) -> bool {
        false
    }

    /// The character being made (06 §10.3), as a value bindings can
    /// read: `app.creation`. Null when none is.
    fn creation(&self) -> ExprValue {
        ExprValue::Null
    }

    /// The templates a character may be made from for the game being
    /// set up, each by its qualified `id` and its `name`: `app.templates`,
    /// what the creation screen begins a session with (`creation` with
    /// `begin`). An application that makes no characters has none.
    fn templates(&self) -> Vec<AppEntry> {
        Vec::new()
    }

    /// Carries out a service, or says why it cannot. The scene has
    /// already changed a preference or a key binding itself when it
    /// passes `set_pref` or `rebind` on: those are for the application
    /// to keep.
    fn service(&mut self, request: &AppRequest) -> Result<(), String>;
}

/// The application of a scene that has none: tools, tests and the widget
/// gallery. Its lists are empty, nothing can be saved or loaded, and it
/// refuses every service but the ones the scene carries out itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoApp;

impl AppState for NoApp {
    fn entries(&self, _list: AppList) -> Vec<AppEntry> {
        Vec::new()
    }

    fn current(&self, _which: AppCurrent) -> Option<AppEntry> {
        None
    }

    fn service(&mut self, request: &AppRequest) -> Result<(), String> {
        match request.service {
            AppService::SetPref | AppService::Rebind | AppService::Quit => Ok(()),
            other => Err(format!(
                "there is no application here to carry out `{}`",
                other.name()
            )),
        }
    }
}

/// Which property of `app` or of an `app_entry` an id is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppProp {
    List(AppList),
    Current(AppCurrent),
    Pref,
    Creation,
    Templates,
    Keys,
    Size,
    CanSave,
    CanLoad,
    EntryId,
    EntryName,
    EntryDescription,
    EntryState,
}

/// Puts `app` and what is read through it into an expression registry:
/// the kinds `app` and `app_entry` and their properties, each with its
/// documentation. Everything is safe for belief: none of it is the
/// simulation's. Doing it twice changes nothing.
///
/// The scene's registry step calls this while mods load; an application
/// that fills a registry of its own may call it too.
pub fn register_expr(registry: &mut Registry) {
    if registry.kind_id(KIND_APP).is_some() {
        return;
    }
    let (Ok(app), Ok(entry)) = (
        registry.add_kind(KindSpec::new(KIND_APP).doc(
            "Things outside the simulation: saves, profiles, mods, scenarios, preferences and the grid (12 §10.4). Bound as `app` in every screen.",
        )),
        registry.add_kind(KindSpec::new(KIND_APP_ENTRY).doc(
            "One entry of a list `app` offers: a save, a profile, a mod or a scenario.",
        )),
    ) else {
        return;
    };
    let pref_id = registry
        .def_type_id(PREFERENCE_TYPE)
        .map_or(Type::Str, Type::Id);
    let entries = Type::list(Type::Subject(entry));
    let specs = [
        PropertySpec::new(app, "saves", entries.clone())
            .doc("The saved games, newest first as the application lists them."),
        PropertySpec::new(app, "profiles", entries.clone())
            .doc("The profiles that can be started (04 §3.3)."),
        PropertySpec::new(app, "mods", entries.clone())
            .doc("The mods that were found, each with whether it is enabled."),
        PropertySpec::new(app, "scenarios", entries)
            .doc("The game scenarios the loaded mods define (13)."),
        PropertySpec::new(app, "profile", Type::Subject(entry).nullable())
            .doc("The profile in use."),
        PropertySpec::new(app, "scenario", Type::Subject(entry).nullable())
            .doc("The game scenario being played, if one is."),
        PropertySpec::new(app, "pref", Type::Any)
            .param("id", pref_id, "The preference's id.")
            .doc("The value of a preference (12 §10.4)."),
        PropertySpec::new(app, "creation", Type::Any)
            .doc("The character being made (06 §10.3); null when none is."),
        PropertySpec::new(app, "templates", Type::list(Type::Subject(entry)))
            .doc("The templates a character may be made from, each by its qualified `id` and its `name` (06 §10.3)."),
        PropertySpec::new(app, "keys", Type::list(Type::map(Type::Str)))
            .param("context", Type::Str, "A context of the keymap: `play`, `menu`, `targeting`.")
            .doc("What the keymap in force binds in a context: one map for each name a key there is given, in order of what it is called, with `name`; `label`, what the control of that name is called, or else the name; `doc`, what it does, or empty; and `keys`, every key given that name, as a keymap writes them, joined by commas (12 §9.2)."),
        PropertySpec::new(app, "size", Type::list(Type::Int))
            .doc("The grid, as `[columns, rows]`."),
        PropertySpec::new(app, "can_save", Type::Bool)
            .doc("Whether the scenario's saving policy allows saving now (13 §9.2)."),
        PropertySpec::new(app, "can_load", Type::Bool)
            .doc("Whether the scenario's saving policy allows loading now."),
        PropertySpec::new(entry, "id", Type::Str)
            .doc("What the entry is known by: a save's file name, a qualified id."),
        PropertySpec::new(entry, "name", Type::Str).doc("The name shown to the player."),
        PropertySpec::new(entry, "description", Type::Str).doc("A line or two about it."),
        PropertySpec::new(entry, "state", Type::Str)
            .doc("Its state, in a word: `enabled`, `disabled`, `broken`, `current`."),
    ];
    for spec in specs {
        // The kinds are new, so no name can be taken.
        let _ = registry.add_property(spec.belief_safe(true));
    }
}

/// The ids the registry gave `app` and its properties, by which the
/// scene knows a read is its own to answer.
#[derive(Clone, Debug)]
pub(crate) struct AppIds {
    app: KindId,
    entry: KindId,
    props: Vec<(PropertyId, AppProp)>,
}

impl AppIds {
    /// Looks the names up. `None` if the registry has no `app`.
    pub(crate) fn find(registry: &Registry) -> Option<AppIds> {
        let app = registry.kind_id(KIND_APP)?;
        let entry = registry.kind_id(KIND_APP_ENTRY)?;
        let mut props = Vec::new();
        let mut add = |kind: KindId, name: &str, prop: AppProp| {
            if let Some(id) = registry.property_id(kind, name) {
                props.push((id, prop));
            }
        };
        for list in AppList::ALL {
            add(app, list.name(), AppProp::List(list));
        }
        for current in [AppCurrent::Profile, AppCurrent::Scenario] {
            add(app, current.name(), AppProp::Current(current));
        }
        add(app, "pref", AppProp::Pref);
        add(app, "creation", AppProp::Creation);
        add(app, "templates", AppProp::Templates);
        add(app, "keys", AppProp::Keys);
        add(app, "size", AppProp::Size);
        add(app, "can_save", AppProp::CanSave);
        add(app, "can_load", AppProp::CanLoad);
        add(entry, "id", AppProp::EntryId);
        add(entry, "name", AppProp::EntryName);
        add(entry, "description", AppProp::EntryDescription);
        add(entry, "state", AppProp::EntryState);
        Some(AppIds { app, entry, props })
    }

    /// The value of `app`.
    pub(crate) fn subject(&self) -> ExprValue {
        ExprValue::Subject(Handle::new(self.app, 0))
    }

    /// Whether a handle is `app` or one of its entries.
    pub(crate) fn owns(&self, handle: Handle) -> bool {
        handle.kind == self.app || handle.kind == self.entry
    }
}

/// What `app` reads are answered from.
pub(crate) struct AppReader<'a> {
    pub(crate) ids: &'a AppIds,
    pub(crate) state: &'a dyn AppState,
    pub(crate) registry: &'a Registry,
    pub(crate) prefs: &'a VarMap,
    pub(crate) size: (u16, u16),
    /// The keymap in force and the namespace a bare control name in it
    /// is read from, for `app.keys`.
    pub(crate) keymap: &'a KeymapDef,
    pub(crate) keymap_ns: &'a str,
    /// The definitions, for what the controls are called.
    pub(crate) ui: &'a UiDefs,
}

/// The code of `app.templates` among the lists entries are read from.
const TEMPLATES_CODE: u64 = AppList::ALL.len() as u64 + 2;

/// How many entries of one list a handle can tell apart.
const ENTRY_BITS: u32 = 32;

impl AppReader<'_> {
    fn entry_handle(&self, code: u64, index: usize) -> ExprValue {
        ExprValue::Subject(Handle::new(
            self.ids.entry,
            (code << ENTRY_BITS) | index as u64,
        ))
    }

    fn entry(&self, handle: Handle) -> Option<AppEntry> {
        let code = handle.payload >> ENTRY_BITS;
        let index = (handle.payload & ((1 << ENTRY_BITS) - 1)) as usize;
        if let Some(list) = AppList::ALL.into_iter().find(|list| list.code() == code) {
            return self.state.entries(list).into_iter().nth(index);
        }
        if code == TEMPLATES_CODE {
            return self.state.templates().into_iter().nth(index);
        }
        [AppCurrent::Profile, AppCurrent::Scenario]
            .into_iter()
            .find(|current| current.code() == code)
            .and_then(|current| self.state.current(current))
    }

    /// `app.keys(context)`: what the keymap in force binds in a context,
    /// by name.
    fn keys(&self, context: &str) -> ExprValue {
        let Some(bound) = self.keymap.contexts.get(context) else {
            return ExprValue::list([]);
        };
        let mut by_name: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (key, name) in bound {
            by_name.entry(name.as_str()).or_default().push(key.to_string());
        }
        let mut rows: Vec<(String, &str, String, String)> = by_name
            .into_iter()
            .map(|(name, keys)| {
                let (label, doc) = self
                    .ui
                    .control_words(name, self.keymap_ns)
                    .unwrap_or_else(|| (name.to_owned(), String::new()));
                (label, name, doc, keys.join(", "))
            })
            .collect();
        // In the order a reader looks them up: by what they are called.
        rows.sort();
        ExprValue::list(rows.into_iter().map(|(label, name, doc, keys)| {
            ExprValue::map([
                ("name", ExprValue::text(name)),
                ("label", ExprValue::text(label)),
                ("doc", ExprValue::text(doc)),
                ("keys", ExprValue::text(keys)),
            ])
        }))
    }

    /// A preference's value: what the player set, or else nothing.
    fn pref(&self, id: &ExprValue) -> ExprValue {
        let name = match id {
            ExprValue::Id(def) => self.registry.def_name(*def),
            other => other.as_str(),
        };
        name.and_then(|name| self.prefs.get(name))
            .map_or(ExprValue::Null, from_var)
    }

    /// Answers a property of `app` or of one of its entries.
    pub(crate) fn property(
        &self,
        property: PropertyId,
        subject: Handle,
        args: &[ExprValue],
    ) -> ExprValue {
        let Some((_, prop)) = self.ids.props.iter().find(|(id, _)| *id == property) else {
            return ExprValue::Null;
        };
        let text = |field: fn(AppEntry) -> String| {
            self.entry(subject)
                .map_or(ExprValue::Null, |entry| ExprValue::text(field(entry)))
        };
        match *prop {
            AppProp::List(list) => ExprValue::list(
                (0..self.state.entries(list).len())
                    .map(|index| self.entry_handle(list.code(), index)),
            ),
            AppProp::Current(current) => match self.state.current(current) {
                Some(_) => self.entry_handle(current.code(), 0),
                None => ExprValue::Null,
            },
            AppProp::Pref => args.first().map_or(ExprValue::Null, |id| self.pref(id)),
            AppProp::Creation => self.state.creation(),
            AppProp::Templates => ExprValue::list(
                (0..self.state.templates().len())
                    .map(|index| self.entry_handle(TEMPLATES_CODE, index)),
            ),
            AppProp::Keys => args
                .first()
                .and_then(ExprValue::as_str)
                .map_or_else(|| ExprValue::list([]), |context| self.keys(context)),
            AppProp::Size => ExprValue::list([
                ExprValue::Int(i64::from(self.size.0)),
                ExprValue::Int(i64::from(self.size.1)),
            ]),
            AppProp::CanSave => ExprValue::Bool(self.state.can_save()),
            AppProp::CanLoad => ExprValue::Bool(self.state.can_load()),
            AppProp::EntryId => text(|entry| entry.id),
            AppProp::EntryName => text(|entry| entry.name),
            AppProp::EntryDescription => text(|entry| entry.description),
            AppProp::EntryState => text(|entry| entry.state),
        }
    }
}

/// A preference's value as the `set_pref` service carries it.
pub(crate) fn pref_request(id: &str, value: &Value) -> AppRequest {
    AppRequest {
        service: AppService::SetPref,
        args: BTreeMap::from([
            ("pref".to_owned(), ExprValue::text(id)),
            ("value".to_owned(), from_var(value)),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_is_registered_once_with_documentation_and_is_safe_for_belief() {
        let mut registry = Registry::with_builtins();
        register_expr(&mut registry);
        let kinds = registry.kinds().count();
        let properties = registry.properties().count();
        register_expr(&mut registry);
        assert_eq!(
            registry.kinds().count(),
            kinds,
            "registering twice changes nothing"
        );
        assert_eq!(registry.properties().count(), properties);

        let app = registry.kind_id(KIND_APP).unwrap();
        let entry = registry.kind_id(KIND_APP_ENTRY).unwrap();
        for kind in [app, entry] {
            assert!(!registry.kind(kind).unwrap().spec.doc.is_empty());
        }
        let names: Vec<&str> = registry
            .properties_of(app)
            .map(|property| property.spec.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "can_load",
                "can_save",
                "creation",
                "keys",
                "mods",
                "pref",
                "profile",
                "profiles",
                "saves",
                "scenario",
                "scenarios",
                "size",
                "templates"
            ]
        );
        for property in registry
            .properties_of(app)
            .chain(registry.properties_of(entry))
        {
            let name = &property.spec.name;
            assert!(
                !property.spec.doc.is_empty(),
                "`{name}` has no documentation"
            );
            assert!(property.spec.belief_safe, "`{name}` is not safe for belief");
        }
        // Every name is known to the scene, which answers it.
        let ids = AppIds::find(&registry).unwrap();
        assert_eq!(ids.props.len(), properties_of_app(&registry));
        assert!(ids.owns(Handle::new(app, 0)));
        assert!(AppIds::find(&Registry::with_builtins()).is_none());
    }

    fn properties_of_app(registry: &Registry) -> usize {
        let app = registry.kind_id(KIND_APP).unwrap();
        let entry = registry.kind_id(KIND_APP_ENTRY).unwrap();
        registry.properties_of(app).count() + registry.properties_of(entry).count()
    }

    #[test]
    fn a_scene_without_an_application_refuses_what_only_one_can_do() {
        let mut app = NoApp;
        assert!(app.entries(AppList::Saves).is_empty());
        assert!(app.templates().is_empty());
        assert!(app.current(AppCurrent::Profile).is_none());
        assert!(!app.can_save() && !app.can_load());
        let request = |service| AppRequest {
            service,
            args: BTreeMap::new(),
        };
        assert_eq!(
            app.service(&request(AppService::Save)),
            Err("there is no application here to carry out `save`".to_owned())
        );
        for own in [AppService::Quit, AppService::SetPref, AppService::Rebind] {
            assert_eq!(app.service(&request(own)), Ok(()));
        }
    }
}
