//! Character specs (06 §10.3): an ordered list of templates and the
//! selection made for each of their choices, and how a spec is resolved
//! into what an entity is made of.
//!
//! # Naming a choice
//!
//! A choice is named by its template's id and its own, joined by a dot:
//! `core:person.stature`, or with the template's bare id,
//! `person.stature`. A choice inside an option is named by the choice
//! the option belongs to, the option and its own id:
//! `core:person.origin.coast.trade`.
//!
//! # What resolving does
//!
//! Every choice of every template is visited in the order written,
//! nested choices right after the option that opens them. A choice the
//! spec selects for is checked; one it does not is given its default,
//! or with `unselected: random` a random selection drawn from a stream
//! seeded by the spec's own `seed` (06 §10.3), so the same spec and seed
//! always make the same thing (invariant B4). A selection that names a
//! choice the templates do not have is refused, and so is one a choice
//! does not take.

use std::collections::BTreeMap;

use clash_core::{Real, Rng};
use clash_data::{Def, Defs};
use clash_expr::Value;
use serde::{Deserialize, Serialize};

use super::defs::{ChoiceDef, ChoiceDefault, ChoiceKind, Gives, MindField, Sets, TemplateDef};
use super::{TemplateError, template_by_name};
use crate::body::{BodyPartDef, BodyPartId, BodyPlanId};
use crate::component::TemplateId;
use crate::rule::EocId;
use crate::stats::{AttributeId, PoolId, TrackId};

/// What is done with a choice a spec does not select for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unselected {
    /// Its default.
    #[default]
    Default,
    /// A random selection, from the spec's seed.
    Random,
}

/// What was selected for one choice.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    /// Option ids for `pick` and `points`; part ids for `assemble`.
    Options(Vec<String>),
    /// A number, for `range`.
    Number(Real),
    /// A text, for `text`.
    Text(String),
}

impl Selection {
    /// The selection as the expression language and the interface read
    /// it: a list of texts, a number or a text.
    pub fn to_value(&self) -> Value {
        match self {
            Selection::Options(ids) => {
                Value::List(ids.iter().map(|id| Value::from(id.as_str())).collect())
            }
            Selection::Number(value) => Value::Real(*value),
            Selection::Text(text) => Value::from(text.as_str()),
        }
    }
}

/// An ordered list of templates and the selection made for each of
/// their choices (06 §10.3): what a creation screen edits, what a
/// scenario supplies for a made character, what a spawn names. Plain
/// data, saved in the input log as it is.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct CharacterSpec {
    /// The templates, by qualified or bare id, in the order applied.
    pub templates: Vec<String>,
    /// Selections, by choice ([module documentation](self)).
    #[serde(default)]
    pub select: BTreeMap<String, Selection>,
    /// What is done with the choices `select` leaves out.
    #[serde(default)]
    pub unselected: Unselected,
    /// What random selections are drawn from.
    #[serde(default)]
    pub seed: u64,
    /// Whether the entity is made anonymous (06 §12.2): one of a
    /// population, forgotten when its tract unloads unless something
    /// makes it notable first. Default false: notable from the start.
    #[serde(default)]
    pub anonymous: bool,
}

impl CharacterSpec {
    /// A spec of templates, every choice at its default.
    pub fn of<S: Into<String>>(templates: impl IntoIterator<Item = S>) -> CharacterSpec {
        CharacterSpec {
            templates: templates.into_iter().map(Into::into).collect(),
            ..CharacterSpec::default()
        }
    }

    /// This spec with one more selection.
    #[must_use]
    pub fn with(mut self, choice: impl Into<String>, selection: Selection) -> CharacterSpec {
        self.select.insert(choice.into(), selection);
        self
    }

    /// This spec with random selections, from a seed, for every choice
    /// it leaves out.
    #[must_use]
    pub fn random(mut self, seed: u64) -> CharacterSpec {
        self.unselected = Unselected::Random;
        self.seed = seed;
        self
    }
}

/// What a spec makes: everything its templates and the options chosen
/// give, gathered in order (06 §10.1).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Resolved {
    /// The templates, in the order applied.
    pub templates: Vec<TemplateId>,
    /// The body plan: the last template's that names one.
    pub body: Option<BodyPlanId>,
    /// The body's size: every `scale` given and every `range` that sets
    /// it, multiplied.
    pub scale: Real,
    /// Bases of its own for attributes: a later one replaces an earlier.
    pub attributes: BTreeMap<AttributeId, Real>,
    /// Pools, in the order first given.
    pub pools: Vec<PoolId>,
    /// What each track starts with, summed, in order of track.
    pub tracks: BTreeMap<TrackId, Real>,
    /// Tags, in the order first given.
    pub tags: Vec<String>,
    /// Variables: a later value replaces an earlier.
    pub variables: BTreeMap<String, Value>,
    /// Rules attached, in the order first given.
    pub rules: Vec<EocId>,
    /// Traits it starts with, as written, in order.
    pub traits: Vec<String>,
    /// Items it starts with, as written, in order.
    pub items: Vec<String>,
    /// A mind: `None` for none, `Some(profile)` for one.
    pub mind: Option<Option<String>>,
    /// Its own name, from a `text` choice that sets it.
    pub name: Option<String>,
    /// Its own description, from a `text` choice that sets it.
    pub description: Option<String>,
    /// The parts `assemble` choices added, in order.
    pub assembled: Vec<BodyPartId>,
    /// Whether it blocks movement.
    pub blocks: bool,
    /// What is left of its body destroyed entirely, as written.
    pub remains: Option<String>,
    /// Every choice and what was selected for it: the spec made
    /// complete.
    pub select: BTreeMap<String, Selection>,
}

/// Gathers what one template or option gives into a [`Resolved`].
fn gather(out: &mut Resolved, gives: Gives<'_>) {
    for (attr, base) in gives.attributes {
        out.attributes.insert(attr.id(), *base);
    }
    for pool in gives.pools {
        if !out.pools.contains(&pool.id()) {
            out.pools.push(pool.id());
        }
    }
    for (track, amount) in gives.tracks {
        *out.tracks.entry(track.id()).or_insert(Real::ZERO) += *amount;
    }
    for tag in gives.tags {
        if !out.tags.contains(tag) {
            out.tags.push(tag.clone());
        }
    }
    for (name, value) in gives.variables {
        if let Some(value) = value.constant() {
            out.variables.insert(name.clone(), value.clone());
        }
    }
    for rule in gives.rules {
        if !out.rules.contains(&rule.id()) {
            out.rules.push(rule.id());
        }
    }
    out.traits.extend(gives.traits.iter().cloned());
    out.items.extend(gives.items.iter().cloned());
    if let Some(scale) = gives.scale {
        out.scale *= scale;
    }
    match gives.mind {
        None => {}
        Some(MindField::Has(false)) => out.mind = None,
        Some(MindField::Has(true)) => {
            out.mind.get_or_insert(None);
        }
        Some(MindField::Profile(profile)) => out.mind = Some(Some(profile.clone())),
    }
}

/// The refusal that a selection does not suit its choice.
fn bad(choice: &str, why: impl Into<String>) -> TemplateError {
    TemplateError::BadSelection {
        choice: choice.to_owned(),
        why: why.into(),
    }
}

/// The qualified id of a part an `assemble` choice allows, if `id`
/// names one: exactly, or by its bare id.
fn allowed_part(defs: &Defs, choice: &ChoiceDef, id: &str) -> Option<BodyPartId> {
    let parts = defs.try_registry::<BodyPartDef>()?;
    choice.parts.iter().map(|part| part.id()).find(|part| {
        let name = parts.name(*part);
        name == id || name.rsplit(':').next() == Some(id)
    })
}

/// Checks a selection against its choice. `key` names the choice.
pub(crate) fn check(
    defs: &Defs,
    key: &str,
    choice: &ChoiceDef,
    selection: &Selection,
) -> Result<(), TemplateError> {
    match (choice.kind, selection) {
        (ChoiceKind::Pick | ChoiceKind::Points, Selection::Options(ids)) => {
            let mut spent = Real::ZERO;
            for (at, id) in ids.iter().enumerate() {
                let option = choice
                    .option(id)
                    .ok_or_else(|| bad(key, format!("`{id}` is not one of its options")))?;
                if ids[..at].contains(id) {
                    return Err(bad(key, format!("`{id}` is chosen twice")));
                }
                spent += option.cost();
            }
            if choice.kind == ChoiceKind::Pick && ids.len() != choice.count() as usize {
                return Err(bad(
                    key,
                    format!("it picks {} options, not {}", choice.count(), ids.len()),
                ));
            }
            if choice.kind == ChoiceKind::Points && spent > choice.budget() {
                return Err(bad(
                    key,
                    format!(
                        "the options cost {spent}, more than its budget of {}",
                        choice.budget()
                    ),
                ));
            }
            Ok(())
        }
        (ChoiceKind::Assemble, Selection::Options(ids)) => {
            for id in ids {
                if allowed_part(defs, choice, id).is_none() {
                    return Err(bad(
                        key,
                        format!("`{id}` is not one of the parts it allows"),
                    ));
                }
            }
            if Real::from_i64(ids.len() as i64) > choice.budget() {
                return Err(bad(
                    key,
                    format!(
                        "{} parts are more than its budget of {}",
                        ids.len(),
                        choice.budget()
                    ),
                ));
            }
            Ok(())
        }
        (ChoiceKind::Range, Selection::Number(value)) => {
            let (min, max) = (
                choice.min.unwrap_or(Real::ZERO),
                choice.max.unwrap_or(Real::ZERO),
            );
            if value.is_finite() && *value >= min && *value <= max {
                Ok(())
            } else {
                Err(bad(key, format!("{value} is not between {min} and {max}")))
            }
        }
        (ChoiceKind::Text, Selection::Text(text)) => {
            if text.chars().count() > choice.max_length() as usize {
                Err(bad(
                    key,
                    format!("it takes at most {} characters", choice.max_length()),
                ))
            } else if text.chars().any(char::is_control) {
                Err(bad(key, "it takes no control characters"))
            } else {
                Ok(())
            }
        }
        (kind, _) => Err(bad(
            key,
            format!(
                "a `{}` choice takes {}",
                kind.name(),
                match kind {
                    ChoiceKind::Pick | ChoiceKind::Points => "a list of option ids",
                    ChoiceKind::Assemble => "a list of part ids",
                    ChoiceKind::Range => "a number",
                    ChoiceKind::Text => "a text",
                }
            ),
        )),
    }
}

/// The selection for a choice nobody made: its default.
pub(crate) fn default_of(choice: &ChoiceDef) -> Selection {
    match (choice.kind, &choice.default) {
        (_, Some(ChoiceDefault::Ids(ids))) => Selection::Options(ids.clone()),
        (_, Some(ChoiceDefault::Number(value))) => Selection::Number(*value),
        (_, Some(ChoiceDefault::Text(text))) => Selection::Text(text.clone()),
        (ChoiceKind::Pick, None) => Selection::Options(
            choice
                .options
                .iter()
                .take(choice.count() as usize)
                .map(|option| option.id.value.clone())
                .collect(),
        ),
        (ChoiceKind::Points | ChoiceKind::Assemble, None) => Selection::Options(Vec::new()),
        (ChoiceKind::Range, None) => Selection::Number(choice.min.unwrap_or(Real::ZERO)),
        (ChoiceKind::Text, None) => Selection::Text(String::new()),
    }
}

/// A random selection for a choice.
fn random_of(choice: &ChoiceDef, rng: &mut Rng) -> Selection {
    let in_order = |mut picked: Vec<usize>| {
        picked.sort_unstable();
        Selection::Options(
            picked
                .into_iter()
                .map(|at| choice.options[at].id.value.clone())
                .collect(),
        )
    };
    match choice.kind {
        ChoiceKind::Pick => {
            let mut order: Vec<usize> = (0..choice.options.len()).collect();
            rng.shuffle(&mut order);
            order.truncate(choice.count() as usize);
            in_order(order)
        }
        ChoiceKind::Points => {
            let mut order: Vec<usize> = (0..choice.options.len()).collect();
            rng.shuffle(&mut order);
            let (mut spent, mut picked) = (Real::ZERO, Vec::new());
            for at in order {
                let cost = choice.options[at].cost();
                if spent + cost <= choice.budget() {
                    spent += cost;
                    picked.push(at);
                }
            }
            in_order(picked)
        }
        ChoiceKind::Range => {
            let (min, max) = (
                choice.min.unwrap_or(Real::ZERO),
                choice.max.unwrap_or(Real::ZERO),
            );
            Selection::Number(min + (max - min) * rng.unit())
        }
        ChoiceKind::Text => match rng.pick(&choice.suggestions) {
            Some(text) => Selection::Text(text.clone()),
            None => default_of(choice),
        },
        ChoiceKind::Assemble => default_of(choice),
    }
}

/// What resolving is working through.
struct Resolver<'a> {
    defs: &'a Defs,
    spec: &'a CharacterSpec,
    rng: Rng,
    out: Resolved,
    /// The keys of `spec.select` a choice has answered to.
    used: Vec<&'a str>,
}

impl<'a> Resolver<'a> {
    /// The selection the spec makes for a choice named `key`, or with
    /// the template's bare id `alias`.
    fn given(&mut self, key: &str, alias: Option<&str>) -> Option<&'a Selection> {
        for name in [Some(key), alias].into_iter().flatten() {
            if let Some((found, selection)) = self.spec.select.get_key_value(name) {
                self.used.push(found.as_str());
                return Some(selection);
            }
        }
        None
    }

    fn choices(
        &mut self,
        choices: &'a [ChoiceDef],
        prefix: &str,
        alias: Option<&str>,
    ) -> Result<(), TemplateError> {
        for choice in choices {
            let key = format!("{prefix}.{}", choice.id.value);
            let alias_key = alias.map(|alias| format!("{alias}.{}", choice.id.value));
            let selection = match self.given(&key, alias_key.as_deref()) {
                Some(given) => given.clone(),
                None => match self.spec.unselected {
                    Unselected::Default => default_of(choice),
                    Unselected::Random => random_of(choice, &mut self.rng),
                },
            };
            check(self.defs, &key, choice, &selection)?;
            self.apply(choice, &key, alias_key.as_deref(), &selection)?;
            self.out.select.insert(key, selection);
        }
        Ok(())
    }

    fn apply(
        &mut self,
        choice: &'a ChoiceDef,
        key: &str,
        alias: Option<&str>,
        selection: &Selection,
    ) -> Result<(), TemplateError> {
        match selection {
            Selection::Options(ids) if choice.kind == ChoiceKind::Assemble => {
                for id in ids {
                    if let Some(part) = allowed_part(self.defs, choice, id) {
                        self.out.assembled.push(part);
                    }
                }
            }
            Selection::Options(ids) => {
                // In the order the options are written, whatever the
                // order they were chosen in.
                for option in &choice.options {
                    if ids.contains(&option.id.value) {
                        gather(&mut self.out, option.gives());
                        let prefix = format!("{key}.{}", option.id.value);
                        let alias = alias.map(|alias| format!("{alias}.{}", option.id.value));
                        self.choices(&option.choices, &prefix, alias.as_deref())?;
                    }
                }
            }
            Selection::Number(value) => {
                if choice.sets == Some(Sets::Scale) {
                    self.out.scale *= *value;
                }
                if let Some(attr) = choice.attribute {
                    self.out.attributes.insert(attr.id(), *value);
                }
            }
            Selection::Text(text) => {
                let text = (!text.is_empty()).then(|| text.clone());
                match choice.sets {
                    Some(Sets::Name) => self.out.name = text.or(self.out.name.take()),
                    Some(Sets::Description) => {
                        self.out.description = text.or(self.out.description.take());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// Resolves a spec against loaded definitions: checks every selection,
/// makes the rest, and gathers what the templates and the chosen
/// options give.
pub fn resolve(defs: &Defs, spec: &CharacterSpec) -> Result<Resolved, TemplateError> {
    let templates = defs
        .try_registry::<TemplateDef>()
        .ok_or(TemplateError::NotRegistered)?;
    if spec.templates.is_empty() {
        return Err(TemplateError::NoTemplates);
    }
    let mut resolver = Resolver {
        defs,
        spec,
        rng: Rng::from_seed(spec.seed),
        out: Resolved {
            scale: Real::ONE,
            blocks: false,
            ..Resolved::default()
        },
        used: Vec::new(),
    };
    let mut blocks = None;
    for name in &spec.templates {
        let id = template_by_name(defs, name)
            .ok_or_else(|| TemplateError::Unknown { what: name.clone() })?;
        let def = templates.get(id);
        let qualified = templates.name(id).to_owned();
        let bare = qualified.rsplit(':').next().map(str::to_owned);
        resolver.out.templates.push(id);
        if let Some(body) = def.body {
            resolver.out.body = Some(body.id());
        }
        if def.remains.is_some() {
            resolver.out.remains.clone_from(&def.remains);
        }
        if def.blocks.is_some() {
            blocks = def.blocks;
        }
        gather(&mut resolver.out, def.gives());
        resolver.choices(&def.choices, &qualified, bare.as_deref())?;
    }
    resolver.out.blocks = blocks.unwrap_or(resolver.out.body.is_some());
    // A selection that answered to no choice names something the
    // templates do not have.
    for key in spec.select.keys() {
        if !resolver.used.contains(&key.as_str()) {
            return Err(TemplateError::UnknownChoice {
                choice: key.clone(),
            });
        }
    }
    Ok(resolver.out)
}

// The template type is named in the documentation above.
const _: &str = TemplateDef::TYPE;
