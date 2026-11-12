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
//! | `name` | The name the character is to have, as last given |
//! | `seed` | The seed the next game is made with |
//! | `finished` | Whether the player has finished: `new_game` will use this character |
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
//!
//! A refusal is the service's reason, which the application shows on the
//! screen of the role `error` (12 §3.2). Every op but `name` and `seed`
//! needs a session in progress; those two are kept without one, for the
//! next session and for the new-game screen.
//!
//! # What `new_game` makes
//!
//! The finished character if there is one; otherwise the start's
//! template with every choice at its default and the name given
//! ([`Creation::spec_for`]). Either way the seed given is the world's and
//! the spec's, and once the game has begun the session and the finished
//! character are dropped.

use clash_core::Real;
use clash_data::Defs;
use clash_expr::Value;
use clash_scene::AppRequest;
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
        entries.push(("template".to_owned(), template));
        for (field, value) in self.fields() {
            entries.push((field.to_owned(), value));
        }
        entries.push(("finished".to_owned(), Value::Bool(self.finished.is_some())));
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
                Ok(())
            }
            other => Err(format!(
                "`creation` has no op `{other}`: it has begin, select, clear, name, seed and finish"
            )),
        }
    }

    fn session_mut(&mut self) -> Result<&mut CreationSession, String> {
        self.session.as_mut().ok_or_else(|| NO_SESSION.to_owned())
    }

    /// The character a new game makes for the player: the finished one,
    /// or else `default` (the start's template) with every choice at its
    /// default and the name given. Refused if there is neither.
    pub fn spec_for(&self, defs: &Defs, default: Option<&str>) -> Result<CharacterSpec, String> {
        if let Some(finished) = &self.finished {
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
    }
}

/// Why an op that needs a session is refused without one.
const NO_SESSION: &str =
    "no character is being made: `creation` with `begin` and a template starts one";
