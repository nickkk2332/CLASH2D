//! The creation session (06 §10.3): the state a character-creation
//! screen edits. It holds a [`CharacterSpec`] and answers, for the
//! templates it names, which choices are open now (a choice inside an
//! option opens once the option is chosen), what each offers, what is
//! selected and what is left of a budget. Every change is checked as it
//! is made, so a session always holds a spec that resolves.
//!
//! The session is not state of the simulation: the application keeps
//! it while the screen is open and hands [`CreationSession::to_value`]
//! to the interface as `app.creation` (12). When the player is done,
//! [`CreationSession::finish`] gives the complete spec to spawn.

use clash_core::Real;
use clash_data::{Def, Defs, Text};
use clash_expr::Value;

use super::defs::{ChoiceDef, ChoiceKind, TemplateDef};
use super::spec::{CharacterSpec, Selection, check, default_of, resolve};
use super::{TemplateError, template_by_name};
use crate::body::BodyPartDef;
use crate::reason::Reason;

/// One option of a choice, as a creation screen shows it.
#[derive(Clone, PartialEq, Debug)]
pub struct OptionView {
    /// Its id.
    pub id: String,
    /// What a reader calls it: its `name` as written, or its id.
    pub name: String,
    /// What it costs from a `points` budget.
    pub cost: Real,
    /// Whether it is chosen now.
    pub chosen: bool,
}

/// One open choice, as a creation screen shows it.
#[derive(Clone, PartialEq, Debug)]
pub struct ChoiceView {
    /// How a selection names it ([`spec`](super::spec) module).
    pub key: String,
    /// What a reader calls it: its `name` as written, or its id.
    pub name: String,
    /// What sort of choice it is.
    pub kind: ChoiceKind,
    /// How deep it sits inside options: 0 for a template's own.
    pub depth: usize,
    /// For `pick`: how many are picked.
    pub count: u32,
    /// For `points` and `assemble`: what may be spent.
    pub budget: Real,
    /// For `points` and `assemble`: what the selection spends.
    pub spent: Real,
    /// For `range`: the bounds.
    pub min: Real,
    /// For `range`: the bounds.
    pub max: Real,
    /// For `text`: the most characters it takes.
    pub max_length: u32,
    /// For `pick` and `points`: the options.
    pub options: Vec<OptionView>,
    /// For `assemble`: the qualified ids of the parts it allows.
    pub parts: Vec<String>,
    /// What is selected now.
    pub selection: Selection,
    /// Whether nobody has selected for it: the selection is its default.
    pub by_default: bool,
}

fn name_of(name: Option<&Text>, id: &str) -> String {
    name.map_or_else(|| id.to_owned(), |text| text.raw().to_owned())
}

/// The state a character-creation screen edits (06 §10.3).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CreationSession {
    spec: CharacterSpec,
}

impl CreationSession {
    /// A session over templates, every choice at its default. Refused
    /// for a template that does not exist.
    pub fn new<S: Into<String>>(
        defs: &Defs,
        templates: impl IntoIterator<Item = S>,
    ) -> Result<CreationSession, Reason> {
        let spec = CharacterSpec::of(templates);
        for name in &spec.templates {
            if template_by_name(defs, name).is_none() {
                return Err(TemplateError::Unknown { what: name.clone() }.reason());
            }
        }
        Ok(CreationSession { spec })
    }

    /// The spec as it stands: what has been selected, and nothing for
    /// what has not.
    pub fn spec(&self) -> &CharacterSpec {
        &self.spec
    }

    /// Every choice open now, in order: each template's, each followed
    /// by those its chosen options open.
    pub fn choices(&self, defs: &Defs) -> Vec<ChoiceView> {
        let mut out = Vec::new();
        let Some(templates) = defs.try_registry::<TemplateDef>() else {
            return out;
        };
        for name in &self.spec.templates {
            if let Some(id) = template_by_name(defs, name) {
                let prefix = templates.name(id).to_owned();
                self.walk(defs, &templates.get(id).choices, &prefix, 0, &mut out);
            }
        }
        out
    }

    fn walk(
        &self,
        defs: &Defs,
        choices: &[ChoiceDef],
        prefix: &str,
        depth: usize,
        out: &mut Vec<ChoiceView>,
    ) {
        for choice in choices {
            let key = format!("{prefix}.{}", choice.id.value);
            let (selection, by_default) = match self.spec.select.get(&key) {
                Some(selection) => (selection.clone(), false),
                None => (default_of(choice), true),
            };
            let chosen: Vec<String> = match &selection {
                Selection::Options(ids) => ids.clone(),
                _ => Vec::new(),
            };
            let options: Vec<OptionView> = choice
                .options
                .iter()
                .map(|option| OptionView {
                    id: option.id.value.clone(),
                    name: name_of(option.name.as_ref(), &option.id.value),
                    cost: option.cost(),
                    chosen: chosen.contains(&option.id.value),
                })
                .collect();
            let spent = match choice.kind {
                ChoiceKind::Points => {
                    let mut spent = Real::ZERO;
                    for option in options.iter().filter(|option| option.chosen) {
                        spent += option.cost;
                    }
                    spent
                }
                ChoiceKind::Assemble => Real::from_i64(chosen.len() as i64),
                _ => Real::ZERO,
            };
            let parts = defs
                .try_registry::<BodyPartDef>()
                .map(|registry| {
                    choice
                        .parts
                        .iter()
                        .map(|part| registry.name(part.id()).to_owned())
                        .collect()
                })
                .unwrap_or_default();
            out.push(ChoiceView {
                key: key.clone(),
                name: name_of(choice.name.as_ref(), &choice.id.value),
                kind: choice.kind,
                depth,
                count: choice.count(),
                budget: choice.budget(),
                spent,
                min: choice.min.unwrap_or(Real::ZERO),
                max: choice.max.unwrap_or(Real::ZERO),
                max_length: choice.max_length(),
                options,
                parts,
                selection,
                by_default,
            });
            if choice.kind != ChoiceKind::Assemble {
                for option in &choice.options {
                    if chosen.contains(&option.id.value) {
                        let prefix = format!("{key}.{}", option.id.value);
                        self.walk(defs, &option.choices, &prefix, depth + 1, out);
                    }
                }
            }
        }
    }

    /// The choice open now under a key.
    fn find<'d>(&self, defs: &'d Defs, key: &str) -> Option<&'d ChoiceDef> {
        let templates = defs.try_registry::<TemplateDef>()?;
        for name in &self.spec.templates {
            let Some(id) = template_by_name(defs, name) else {
                continue;
            };
            let prefix = templates.name(id).to_owned();
            if let Some(found) = self.find_in(&templates.get(id).choices, &prefix, key) {
                return Some(found);
            }
        }
        None
    }

    fn find_in<'d>(
        &self,
        choices: &'d [ChoiceDef],
        prefix: &str,
        key: &str,
    ) -> Option<&'d ChoiceDef> {
        for choice in choices {
            let here = format!("{prefix}.{}", choice.id.value);
            if here == key {
                return Some(choice);
            }
            if !key.starts_with(&here) || choice.kind == ChoiceKind::Assemble {
                continue;
            }
            let chosen = match self.spec.select.get(&here).cloned() {
                Some(selection) => selection,
                None => default_of(choice),
            };
            let Selection::Options(ids) = chosen else {
                continue;
            };
            for option in &choice.options {
                if ids.contains(&option.id.value) {
                    let prefix = format!("{here}.{}", option.id.value);
                    if let Some(found) = self.find_in(&option.choices, &prefix, key) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// Selects for an open choice. Refused, changing nothing, for a key
    /// that names no open choice and for a selection the choice does not
    /// take. Selections for choices that the change closes are dropped.
    pub fn select(&mut self, defs: &Defs, key: &str, selection: Selection) -> Result<(), Reason> {
        let choice = self.find(defs, key).ok_or_else(|| {
            TemplateError::UnknownChoice {
                choice: key.to_owned(),
            }
            .reason()
        })?;
        check(defs, key, choice, &selection).map_err(|error| error.reason())?;
        self.spec.select.insert(key.to_owned(), selection);
        self.prune(defs);
        Ok(())
    }

    /// Takes back a selection: the choice is at its default again.
    pub fn clear(&mut self, defs: &Defs, key: &str) {
        self.spec.select.remove(key);
        self.prune(defs);
    }

    /// Drops the selections for choices that are no longer open.
    fn prune(&mut self, defs: &Defs) {
        let open: Vec<String> = self
            .choices(defs)
            .into_iter()
            .map(|view| view.key)
            .collect();
        self.spec.select.retain(|key, _| open.contains(key));
    }

    /// The complete spec: every open choice with what is selected for
    /// it. Refused if it does not resolve.
    pub fn finish(&self, defs: &Defs) -> Result<CharacterSpec, Reason> {
        let resolved = resolve(defs, &self.spec).map_err(|error| error.reason())?;
        Ok(CharacterSpec {
            select: resolved.select,
            ..self.spec.clone()
        })
    }

    /// The session as the interface reads it under `app.creation`: a
    /// map with `templates` (a list of ids), `choices` (a list of maps,
    /// one for each open choice: `key`, `name`, `kind`, `depth`,
    /// `count`, `budget`, `spent`, `left`, `min`, `max`, `max_length`,
    /// `options` (maps of `id`, `name`, `cost`, `chosen`), `parts`,
    /// `value`, `by_default`), and `complete` with `problem`, whether
    /// the spec resolves and why not.
    pub fn to_value(&self, defs: &Defs) -> Value {
        let choices = self
            .choices(defs)
            .into_iter()
            .map(|view| {
                let options = view
                    .options
                    .iter()
                    .map(|option| {
                        Value::map([
                            ("id", Value::from(option.id.as_str())),
                            ("name", Value::from(option.name.as_str())),
                            ("cost", Value::Real(option.cost)),
                            ("chosen", Value::Bool(option.chosen)),
                        ])
                    })
                    .collect::<Vec<_>>();
                let parts = view
                    .parts
                    .iter()
                    .map(|part| Value::from(part.as_str()))
                    .collect::<Vec<_>>();
                Value::map([
                    ("key", Value::from(view.key.as_str())),
                    ("name", Value::from(view.name.as_str())),
                    ("kind", Value::from(view.kind.name())),
                    ("depth", Value::Int(view.depth as i64)),
                    ("count", Value::Int(i64::from(view.count))),
                    ("budget", Value::Real(view.budget)),
                    ("spent", Value::Real(view.spent)),
                    ("left", Value::Real(view.budget - view.spent)),
                    ("min", Value::Real(view.min)),
                    ("max", Value::Real(view.max)),
                    ("max_length", Value::Int(i64::from(view.max_length))),
                    ("options", Value::List(options.into())),
                    ("parts", Value::List(parts.into())),
                    ("value", view.selection.to_value()),
                    ("by_default", Value::Bool(view.by_default)),
                ])
            })
            .collect::<Vec<_>>();
        let problem = resolve(defs, &self.spec).err();
        Value::map([
            (
                "templates",
                Value::List(
                    self.spec
                        .templates
                        .iter()
                        .map(|name| Value::from(name.as_str()))
                        .collect::<Vec<_>>()
                        .into(),
                ),
            ),
            ("choices", Value::List(choices.into())),
            ("complete", Value::Bool(problem.is_none())),
            (
                "problem",
                problem.map_or(Value::Null, |problem| Value::from(problem.to_string())),
            ),
        ])
    }
}

// The session reads the `template` type.
const _: &str = TemplateDef::TYPE;
