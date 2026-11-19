//! The character the player is making for the next game: the creation
//! session (06 §10.3) as the interface reads and edits it (12 §10.4,
//! decision D-653).
//!
//! The application keeps a [`Creation`] beside the scene: the
//! simulation's [`CreationSession`] while one is in progress, the name
//! and the seed the next game is made with, and the spec the player
//! finished, if they did.
//!
//! # What a screen reads: `app.creation`
//!
//! Null while no session is in progress. Otherwise the map
//! [`CreationSession::to_value`] gives (`templates`, `choices`,
//! `complete`, `problem`), with four more entries:
//!
//! | Entry | Is |
//! |-------|----|
//! | `template` | The qualified id of the session's first template |
//! | `made_from` | What that template is called: `Brawler` |
//! | `name` | The name the character is to have, as last given |
//! | `seed` | The seed the next game is made with |
//! | `finished` | Whether the player has finished: `new_game` will use this character |
//! | `editable` | Whether the game the character is made for lets the player make its choices (`create`), or only choose it ready-made and name it (13 §4.2) |
//!
//! # What a screen asks: the service `creation`
//!
//! | `op` | With | Does |
//! |------|------|------|
//! | `begin` | `template` | Starts a session over that template, every choice at its default, and drops a finished character. The name and seed given so far are kept, and the name is selected for the template's text choice that sets the name, if it has one |
//! | `select` | `key`, `value` | Selects for an open choice: a list of option or part ids, one id, a number or a text, as the choice takes |
//! | `clear` | `key` | Takes a selection back: the choice is at its default |
//! | `name` | `value` | The character's name: kept, and selected for the template's text choice that sets the name |
//! | `seed` | `value` | The seed the next game is made with, and the one random selections are drawn from |
//! | `finish` | | Checks the character. Refused with the reason if it cannot be made; otherwise it is kept for `new_game` |
//! | `scenario` | `value` | The game the character is made for, by the qualified id of its scenario, or null for none: `app.templates` offers what that game allows (13 §4.2, decision D-860). A change drops the session and the finished character |
//!
//! A refusal is the service's reason, which the application shows on the
//! screen of the role `error` (12 §3.2). Every op but `name` and `seed`
//! needs a session in progress; those two are kept without one, for the
//! next session and for the new-game screen.
//!
//! # What may be made (13 §4.2; decision D-860)
//!
//! The templates offered, `app.templates`, are those the game the
//! character is made for offers (`CharacterEntry::offered`: what its
//! `pick` lists, or every template marked `playable`), or with no game
//! named every playable template. `begin` takes only one of them. A game
//! whose character is chosen ready-made (`create` false) lets the
//! player choose and name it and nothing more: `select` and `clear` are
//! refused there, but for the choice that sets the name.
//!
//! # What `new_game` makes
//!
//! The finished character if there is one and it was made for this game
//! (or for no game in particular, and the game offers its template);
//! otherwise the scenario's template with every choice at its default
//! and the name given ([`Creation::spec_for`]). Either way the seed given
//! is the world's and the spec's, and once the game has begun the
//! session and the finished character are dropped.

use clash_core::Real;
use clash_data::Defs;
use clash_expr::Value;
use clash_scene::{AppEntry, AppRequest};
use clash_sim::scenario::ScenarioDef;
use clash_sim::template::{
    CharacterSpec, ChoiceKind, CreationSession, Selection, Sets, TemplateDef,
};

/// The ops of the service `creation`.
pub const OP_BEGIN: &str = "begin";
/// Selects for a choice.
pub const OP_SELECT: &str = "select";
/// Takes a selection back.
pub const OP_CLEAR: &str = "clear";
/// Gives the character's name.
pub const OP_NAME: &str = "name";
/// Gives the seed.
pub const OP_SEED: &str = "seed";
/// Finishes the character.
pub const OP_FINISH: &str = "finish";
/// Names the game the character is made for.
pub const OP_SCENARIO: &str = "scenario";

/// The fields of the character being made that a screen variable named
/// `creation_<field>` is set to when its screen opens (12 §3.4).
pub const FIELD_NAME: &str = "name";
/// The seed, as such a field.
pub const FIELD_SEED: &str = "seed";

/// What the player has chosen for the next game. See the
/// [module](self) documentation.
#[derive(Clone, Debug, Default)]
pub struct Creation {
    session: Option<CreationSession>,
    /// The name the character is to have.
    name: String,
    /// The seed the next game is made with.
    seed: u64,
    /// The character the player finished, for `new_game`.
    finished: Option<CharacterSpec>,
    /// The game the character is being made for, by the qualified id of
    /// its scenario: what `app.templates` offers.
    scenario: Option<String>,
    /// The game the finished character was made for, if any was named.
    made_for: Option<String>,
}

/// The scenario's character entry by the scenario's qualified id, with
/// its namespace.
fn character_of<'a>(
    defs: &'a Defs,
    scenario: &'a str,
) -> Option<(&'a clash_sim::scenario::def::CharacterEntry, &'a str)> {
    let scenarios = defs.try_registry::<ScenarioDef>()?;
    let def = scenarios.get(scenarios.lookup(scenario)?);
    let namespace = clash_sim::scenario::def::namespace_of(scenario);
    def.character().map(|character| (character, namespace))
}

/// Every template marked playable, in order of qualified id.
fn every_playable(defs: &Defs) -> Vec<String> {
    let Some(templates) = defs.try_registry::<TemplateDef>() else {
        return Vec::new();
    };
    let mut ids: Vec<String> = templates
        .iter()
        .filter(|(_, def)| def.playable)
        .map(|(id, _)| templates.name(id).to_owned())
        .collect();
    ids.sort();
    ids
}

/// The qualified ids of the templates a game offers the player's
/// character from (13 §4.2): with no game named, every playable one.
pub fn offered(defs: &Defs, scenario: Option<&str>) -> Vec<String> {
    match scenario {
        Some(scenario) => character_of(defs, scenario)
            .map(|(character, namespace)| character.offered(defs, namespace))
            .unwrap_or_default(),
        None => every_playable(defs),
    }
}

/// Whether a game lets the player make its character's choices: with
/// no game named, or a game that names no character, it does.
pub fn editable(defs: &Defs, scenario: Option<&str>) -> bool {
    scenario
        .and_then(|scenario| character_of(defs, scenario))
        .is_none_or(|(character, _)| character.editable())
}

/// The templates offered, as `app.templates` lists them: each by its
/// qualified `id`, its `name` and its `description`.
pub fn template_entries(defs: &Defs, scenario: Option<&str>) -> Vec<AppEntry> {
    let Some(templates) = defs.try_registry::<TemplateDef>() else {
        return Vec::new();
    };
    offered(defs, scenario)
        .into_iter()
        .filter_map(|id| {
            let def = templates.get(templates.lookup(&id)?);
            let text = |text: Option<&clash_data::Text>| {
                text.map(|text| text.raw().trim().to_owned())
                    .filter(|text| !text.is_empty())
            };
            Some(AppEntry {
                name: text(def.name.as_ref())
                    .unwrap_or_else(|| id.rsplit(':').next().unwrap_or(&id).replace('_', " ")),
                description: text(def.description.as_ref()).unwrap_or_default(),
                state: "enabled".to_owned(),
                id,
            })
        })
        .collect()
}

/// What a scenario is called, for a sentence.
fn scenario_name(defs: &Defs, scenario: &str) -> String {
    defs.try_registry::<ScenarioDef>()
        .and_then(|scenarios| {
            let id = scenarios.lookup(scenario)?;
            Some(scenarios.get(id).shown_name(scenario))
        })
        .unwrap_or_else(|| "this game".to_owned())
}

/// What a template is called, for a sentence.
fn template_shown(defs: &Defs, template: &str) -> String {
    defs.try_registry::<TemplateDef>()
        .and_then(|templates| {
            let def = templates.get(templates.lookup(template)?);
            def.name.as_ref().map(|name| name.raw().trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            template
                .rsplit(':')
                .next()
                .unwrap_or(template)
                .replace('_', " ")
        })
}

/// The qualified id of a template a session or a start names, if it is
/// loaded.
fn template_name(defs: &Defs, name: &str) -> Option<String> {
    let templates = defs.try_registry::<TemplateDef>()?;
    if let Some(id) = templates.lookup(name) {
        return Some(templates.name(id).to_owned());
    }
    if name.contains(':') {
        return None;
    }
    // A bare name: the first of that name in order of qualified id.
    templates
        .names()
        .iter()
        .filter(|qualified| qualified.rsplit(':').next() == Some(name))
        .min()
        .cloned()
}

/// The key of the text choice of a template that sets the name, if it
/// has one: `core:person.name`.
fn name_choice(defs: &Defs, template: &str) -> Option<String> {
    let qualified = template_name(defs, template)?;
    let templates = defs.try_registry::<TemplateDef>()?;
    let def = templates.get(templates.lookup(&qualified)?);
    def.choices
        .iter()
        .find(|choice| choice.kind == ChoiceKind::Text && choice.sets == Some(Sets::Name))
        .map(|choice| format!("{qualified}.{}", choice.id.value))
}

/// A value as a selection for a choice of a kind, if it can be one.
fn selection_of(kind: ChoiceKind, value: &Value) -> Option<Selection> {
    match (kind, value) {
        (ChoiceKind::Pick | ChoiceKind::Points | ChoiceKind::Assemble, Value::List(items)) => {
            let ids: Option<Vec<String>> = items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect();
            ids.map(Selection::Options)
        }
        (ChoiceKind::Pick | ChoiceKind::Points | ChoiceKind::Assemble, Value::Str(id)) => {
            Some(Selection::Options(vec![id.to_string()]))
        }
        (ChoiceKind::Range, number) => number.as_number().map(Selection::Number),
        (ChoiceKind::Text, Value::Str(text)) => Some(Selection::Text(text.to_string())),
        _ => None,
    }
}

/// A seed as a screen gives it: a whole number, its magnitude.
fn seed_of(value: &Value) -> Option<u64> {
    match value {
        Value::Int(int) => Some(int.unsigned_abs()),
        Value::Real(real) if *real >= Real::ZERO => Some(real.get() as u64),
        _ => None,
    }
}

impl Creation {
    /// Nothing made yet, the next game to be made with a seed.
    pub fn new(seed: u64) -> Creation {
        Creation {
            seed,
            ..Creation::default()
        }
    }

    /// The session in progress, if there is one.
    pub fn session(&self) -> Option<&CreationSession> {
        self.session.as_ref()
    }

    /// The character the player finished, if they did.
    pub fn finished(&self) -> Option<&CharacterSpec> {
        self.finished.as_ref()
    }

    /// The name the character is to have.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The seed the next game is made with.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The game the character is being made for, if one was named.
    pub fn scenario(&self) -> Option<&str> {
        self.scenario.as_deref()
    }

    /// `app.templates`: the templates the game the character is made for
    /// offers, each by its qualified id, its name and its description.
    pub fn templates(&self, defs: &Defs) -> Vec<AppEntry> {
        template_entries(defs, self.scenario.as_deref())
    }

    /// Names the game the character is made for. A change drops the
    /// session and the finished character.
    pub fn set_scenario(&mut self, scenario: Option<String>) {
        if self.scenario != scenario {
            self.session = None;
            self.finished = None;
            self.made_for = None;
        }
        self.scenario = scenario;
    }

    /// Why a selection for a choice is refused in a game whose character
    /// is chosen ready-made: every choice but the one that sets the name.
    fn fixed(&self, defs: &Defs, key: &str) -> Result<(), String> {
        if editable(defs, self.scenario.as_deref()) {
            return Ok(());
        }
        let names = self
            .session
            .iter()
            .flat_map(|session| session.spec().templates.iter())
            .filter_map(|template| name_choice(defs, template))
            .any(|name| name == key);
        if names {
            return Ok(());
        }
        let game = self
            .scenario
            .as_deref()
            .map_or_else(|| "this game".to_owned(), |id| scenario_name(defs, id));
        Err(format!(
            "In {game} you choose who you are, ready-made: you may name them, and nothing more."
        ))
    }

    /// What a screen variable named `creation_<field>` is set to when
    /// its screen opens: the name and the seed, whether or not a session
    /// is in progress.
    pub fn fields(&self) -> [(&'static str, Value); 2] {
        [
            (FIELD_NAME, Value::text(&self.name)),
            (
                FIELD_SEED,
                Value::Int(i64::try_from(self.seed).unwrap_or(i64::MAX)),
            ),
        ]
    }

    /// `app.creation`: see the [module](self) documentation.
    pub fn value(&self, defs: &Defs) -> Value {
        let Some(session) = &self.session else {
            return Value::Null;
        };
        let mut entries: Vec<(String, Value)> = session
            .to_value(defs)
            .as_map()
            .map(|map| {
                map.iter()
                    .map(|(key, value)| (key.to_string(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let template = session
            .spec()
            .templates
            .first()
            .and_then(|name| template_name(defs, name))
            .map_or(Value::Null, Value::text);
        let made_from = session
            .spec()
            .templates
            .first()
            .and_then(|name| template_name(defs, name))
            .map_or(Value::Null, |id| Value::text(template_shown(defs, &id)));
        entries.push(("template".to_owned(), template));
        entries.push(("made_from".to_owned(), made_from));
        for (field, value) in self.fields() {
            entries.push((field.to_owned(), value));
        }
        entries.push(("finished".to_owned(), Value::Bool(self.finished.is_some())));
        entries.push((
            "editable".to_owned(),
            Value::Bool(editable(defs, self.scenario.as_deref())),
        ));
        Value::map(entries)
    }

    /// Gives the character a name: kept, and selected for the text
    /// choice that sets the name if a session is in progress. Refused
    /// if that choice does not take it.
    pub fn set_name(&mut self, defs: &Defs, name: &str) -> Result<(), String> {
        let name = name.trim().to_owned();
        if let Some(session) = &mut self.session {
            let key = session
                .spec()
                .templates
                .iter()
                .find_map(|template| name_choice(defs, template));
            if let Some(key) = key {
                if name.is_empty() {
                    session.clear(defs, &key);
                } else {
                    session
                        .select(defs, &key, Selection::Text(name.clone()))
                        .map_err(|reason| reason.message)?;
                }
                self.finished = None;
            }
        }
        self.name = name;
        Ok(())
    }

    /// Sets the seed the next game is made with.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
        if let Some(finished) = &mut self.finished {
            finished.seed = seed;
        }
    }

    /// The service `creation`: see the [module](self) documentation.
    pub fn service(&mut self, defs: &Defs, request: &AppRequest) -> Result<(), String> {
        let op = request
            .text("op")
            .ok_or("`creation` needs an `op`: begin, select, clear, name, seed or finish")?;
        let value = request.args.get("value").cloned().unwrap_or_default();
        match op {
            OP_BEGIN => {
                let template = request.text("template").ok_or(
                    "`creation` with `begin` needs the `template` to make a character from",
                )?;
                let qualified = template_name(defs, template)
                    .ok_or_else(|| format!("there is no template `{template}`"))?;
                if !offered(defs, self.scenario.as_deref()).contains(&qualified) {
                    let game = self
                        .scenario
                        .as_deref()
                        .map_or_else(|| "this game".to_owned(), |id| scenario_name(defs, id));
                    return Err(format!(
                        "{game} does not offer a character made as a {}.",
                        template_shown(defs, &qualified).to_lowercase()
                    ));
                }
                let session =
                    CreationSession::new(defs, [qualified]).map_err(|reason| reason.message)?;
                self.session = Some(session);
                self.finished = None;
                let name = self.name.clone();
                if !name.is_empty() {
                    self.set_name(defs, &name)?;
                }
                Ok(())
            }
            OP_SELECT => {
                let key = request
                    .text("key")
                    .ok_or("`creation` with `select` needs the `key` of the choice")?;
                self.fixed(defs, key)?;
                let session = self.session_mut()?;
                let kind = session
                    .choices(defs)
                    .into_iter()
                    .find(|choice| choice.key == key)
                    .map(|choice| choice.kind)
                    .ok_or_else(|| format!("there is no choice `{key}` open now"))?;
                let selection = selection_of(kind, &value).ok_or_else(|| {
                    format!(
                        "`{value}` is not a selection the choice `{key}` takes: it takes {}",
                        match kind {
                            ChoiceKind::Pick | ChoiceKind::Points => "option ids",
                            ChoiceKind::Assemble => "part ids",
                            ChoiceKind::Range => "a number",
                            ChoiceKind::Text => "a text",
                        }
                    )
                })?;
                session
                    .select(defs, key, selection)
                    .map_err(|reason| reason.message)?;
                self.finished = None;
                Ok(())
            }
            OP_CLEAR => {
                let key = request
                    .text("key")
                    .ok_or("`creation` with `clear` needs the `key` of the choice")?
                    .to_owned();
                self.fixed(defs, &key)?;
                self.session_mut()?.clear(defs, &key);
                self.finished = None;
                Ok(())
            }
            OP_NAME => {
                let name = value
                    .as_str()
                    .ok_or("`creation` with `name` needs the name as a text `value`")?
                    .to_owned();
                self.set_name(defs, &name)
            }
            OP_SEED => {
                let seed =
                    seed_of(&value).ok_or("`creation` with `seed` needs a whole number `value`")?;
                self.set_seed(seed);
                Ok(())
            }
            OP_FINISH => {
                let session = self.session.as_ref().ok_or(NO_SESSION)?;
                let mut spec = session.finish(defs).map_err(|reason| reason.message)?;
                spec.seed = self.seed;
                self.finished = Some(spec);
                self.made_for = self.scenario.clone();
                Ok(())
            }
            OP_SCENARIO => {
                let scenario = match &value {
                    Value::Null => None,
                    value => Some(
                        value
                            .as_str()
                            .ok_or("`creation` with `scenario` needs the scenario's id as a text `value`, or null")?
                            .to_owned(),
                    ),
                };
                if let Some(id) = &scenario
                    && defs
                        .try_registry::<ScenarioDef>()
                        .and_then(|scenarios| scenarios.lookup(id))
                        .is_none()
                {
                    return Err(format!("there is no game scenario `{id}`"));
                }
                self.set_scenario(scenario);
                Ok(())
            }
            other => Err(format!(
                "`creation` has no op `{other}`: it has begin, select, clear, name, seed, finish and scenario"
            )),
        }
    }

    fn session_mut(&mut self) -> Result<&mut CreationSession, String> {
        self.session.as_mut().ok_or_else(|| NO_SESSION.to_owned())
    }

    /// The character a new game of `scenario` makes for the player: the
    /// finished one, if it was made for this game, or for none named and
    /// this game offers its template; or else `default` (the scenario's
    /// template) with every choice at its default and the name given.
    /// Refused if there is neither, and for a finished character made for
    /// no game in particular from a template this game does not offer.
    pub fn spec_for(
        &self,
        defs: &Defs,
        scenario: &str,
        default: Option<&str>,
    ) -> Result<CharacterSpec, String> {
        let for_this = self
            .made_for
            .as_deref()
            .is_none_or(|made_for| made_for == scenario);
        if let Some(finished) = self.finished.as_ref().filter(|_| for_this) {
            let first = finished
                .templates
                .first()
                .and_then(|template| template_name(defs, template));
            let allowed = offered(defs, Some(scenario));
            if let Some(first) = first
                && !allowed.contains(&first)
            {
                return Err(format!(
                    "{} does not offer a character made as a {}: make or choose another with F2.",
                    scenario_name(defs, scenario),
                    template_shown(defs, &first).to_lowercase()
                ));
            }
            return Ok(CharacterSpec {
                seed: self.seed,
                ..finished.clone()
            });
        }
        let template = default.ok_or(
            "this start names no template to make the character from, and no character has been made",
        )?;
        let mut spec = CharacterSpec::of([template]);
        spec.seed = self.seed;
        if !self.name.is_empty()
            && let Some(key) = name_choice(defs, template)
        {
            spec = spec.with(key, Selection::Text(self.name.clone()));
        }
        Ok(spec)
    }

    /// Forgets the character once a game has begun with it: the next
    /// game is made afresh, with the same name and seed.
    pub fn begun(&mut self) {
        self.session = None;
        self.finished = None;
        self.made_for = None;
        self.scenario = None;
    }
}

/// Why an op that needs a session is refused without one.
const NO_SESSION: &str =
    "no character is being made: `creation` with `begin` and a template starts one";
