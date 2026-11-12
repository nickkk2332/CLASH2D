//! Templates, choices and creation (`docs/06-entities-and-bodies.md`
//! section 10).
//!
//! A **template** says what a kind of thing is made of: a body plan and
//! its size, attributes, pools and tracks, tags, variables, rules,
//! starting traits and items, a mind, how it is drawn, and **choices**.
//! A **character spec** ([`CharacterSpec`]) lists templates in the order
//! applied and the selection made for each choice. Applying a spec is
//! deterministic: the same spec and seed make the same entity
//! (invariant B4).
//!
//! | Need | Use |
//! |------|-----|
//! | To make an entity from templates | [`Sim::spawn_template`]: the verb `spawn`, the debug command `spawn`, and a tract's `spawn_requested` that names a template |
//! | What a spec makes, without making it | [`resolve`] |
//! | The state a creation screen edits | [`CreationSession`] |
//! | How a made thing is drawn | [`Sim::glyph`] |
//!
//! # What spawning does
//!
//! In order: the spec is resolved and checked, and a refusal leaves no
//! trace; the place is checked for the body's footprint; the entity is
//! made out of the world with its tags and variables; it is given its
//! body, then stats if a template gives a body or any stat, the bases
//! of its attributes, its pools, its tracks; its identity, with the
//! templates applied and its own name; a mind if a template gives one;
//! its rules; it is put in its place; and `entity_spawned` is raised
//! with the traits and items it is to start with, for whoever gives
//! those (06 §5, §9).
//!
//! # Not built
//!
//! The `creature` definition type (06 §10.3) and `apply_template` on an
//! entity that exists. A choice's `assemble` is checked and its parts
//! are reported, and not yet added to the body: that waits for
//! `add_part` (06 §6.5). Conditions on options, a range's `apply`
//! effects and `body_edits` wait for the rule runtime and body editing.

mod defs;
mod session;
pub mod spec;
mod verbs;

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::{Def, DefTypes, Defs};
use clash_world::{Cause, Footprint, Location, OccupantFlags, OccupantKind, SpawnRequest};
use thiserror::Error;

use crate::Sim;
use crate::body::BodyPartId;
use crate::component::{DefKey, Identity};
use crate::entity::{EntitySpec, Place, Presence};
use crate::event::{Subject, Where};
use crate::life::events::{about, texts_field};
use crate::reason::{Reason, ReasonInfo};

pub use defs::{
    ChoiceDef, ChoiceDefault, ChoiceKind, DEFAULT_TEXT_LENGTH, Gives, GlyphDef, MAX_CHOICE_DEPTH,
    MindField, OptionDef, Sets, TemplateDef,
};
pub use session::{ChoiceView, CreationSession, OptionView};
pub use spec::{CharacterSpec, Resolved, Selection, Unselected, resolve};
pub use verbs::verbs;

pub use crate::component::TemplateId;

/// Why a spec could not be resolved or applied (14 §5).
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum TemplateError {
    /// The definitions were loaded without the `template` type.
    #[error("templates are not registered with the loader")]
    NotRegistered,
    /// A spec names no template.
    #[error("a spec names no template")]
    NoTemplates,
    /// An id names no template.
    #[error("`{what}` is not a template")]
    Unknown {
        /// The id as written.
        what: String,
    },
    /// A selection names a choice none of the templates has.
    #[error("no template of the spec has the choice `{choice}`")]
    UnknownChoice {
        /// The choice as written.
        choice: String,
    },
    /// A selection does not suit its choice.
    #[error("the selection for `{choice}` does not suit it: {why}")]
    BadSelection {
        /// The choice.
        choice: String,
        /// What is wrong.
        why: String,
    },
}

/// The codes of the reasons templates refuse for (14 §5).
pub mod reason {
    /// The definitions were loaded without the `template` type.
    pub const TEMPLATES_NOT_REGISTERED: &str = "templates_not_registered";
    /// A spec names no template.
    pub const NO_TEMPLATES: &str = "no_templates";
    /// An id names no template. Values: `what`.
    pub const UNKNOWN_TEMPLATE: &str = "unknown_template";
    /// A selection names a choice none of the templates has. Values:
    /// `choice`.
    pub const UNKNOWN_CHOICE: &str = "unknown_choice";
    /// A selection does not suit its choice. Values: `choice`, `why`.
    pub const BAD_SELECTION: &str = "bad_selection";
}

impl TemplateError {
    /// The stable code of the refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            TemplateError::NotRegistered => reason::TEMPLATES_NOT_REGISTERED,
            TemplateError::NoTemplates => reason::NO_TEMPLATES,
            TemplateError::Unknown { .. } => reason::UNKNOWN_TEMPLATE,
            TemplateError::UnknownChoice { .. } => reason::UNKNOWN_CHOICE,
            TemplateError::BadSelection { .. } => reason::BAD_SELECTION,
        }
    }

    /// The refusal as a [`Reason`], with its values.
    pub fn reason(&self) -> Reason {
        let reason = Reason::new(self.code(), self.to_string());
        match self {
            TemplateError::NotRegistered | TemplateError::NoTemplates => reason,
            TemplateError::Unknown { what } => reason.with("what", what.as_str()),
            TemplateError::UnknownChoice { choice } => reason.with("choice", choice.as_str()),
            TemplateError::BadSelection { choice, why } => reason
                .with("choice", choice.as_str())
                .with("why", why.as_str()),
        }
    }
}

/// The reasons of templates and creation, for the generated reference.
pub const TEMPLATE_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: reason::TEMPLATES_NOT_REGISTERED,
        doc: "The definitions were loaded without the `template` definition type.",
    },
    ReasonInfo {
        code: reason::NO_TEMPLATES,
        doc: "A spec names no template.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_TEMPLATE,
        doc: "An id names no template. Values: `what`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_CHOICE,
        doc: "A selection names a choice none of the spec's templates has. Values: `choice`.",
    },
    ReasonInfo {
        code: reason::BAD_SELECTION,
        doc: "A selection does not suit its choice: an option it does not have, too many or too few, over budget, out of range, too long, or of the wrong shape. Values: `choice`, `why`.",
    },
];

/// The template a name gives: a qualified id exactly, or a bare one in
/// every namespace in sorted order (as the world resolves its own
/// references, decision D-283).
pub(crate) fn template_by_name(defs: &Defs, name: &str) -> Option<TemplateId> {
    let templates = defs.try_registry::<TemplateDef>()?;
    if let Some(id) = templates.lookup(name) {
        return Some(id);
    }
    if name.contains(':') {
        return None;
    }
    let mut found: Vec<&String> = templates
        .names()
        .iter()
        .filter(|qualified| qualified.rsplit(':').next() == Some(name))
        .collect();
    found.sort();
    found
        .first()
        .and_then(|qualified| templates.lookup(qualified))
}

/// The templates' tables. Not state: made from the definitions.
#[derive(Clone, Debug)]
pub struct TemplateDefs {
    defs: Arc<Defs>,
}

impl TemplateDefs {
    /// The tables for a set of loaded definitions.
    pub(crate) fn new(defs: &Arc<Defs>) -> TemplateDefs {
        TemplateDefs {
            defs: Arc::clone(defs),
        }
    }

    /// The loaded definitions.
    pub fn defs(&self) -> &Arc<Defs> {
        &self.defs
    }

    /// The template a name gives, qualified or bare.
    pub fn lookup(&self, name: &str) -> Option<TemplateId> {
        template_by_name(&self.defs, name)
    }

    /// A template's definition.
    pub fn get(&self, id: TemplateId) -> Option<&TemplateDef> {
        let templates = self.defs.try_registry::<TemplateDef>()?;
        (id.index() < templates.len()).then(|| templates.get(id))
    }

    /// A template's qualified id.
    pub fn name(&self, id: TemplateId) -> Option<&str> {
        let templates = self.defs.try_registry::<TemplateDef>()?;
        (id.index() < templates.len()).then(|| templates.name(id))
    }
}

/// Registers the `template` definition type. Called from the crate's
/// `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<TemplateDef>();
}

/// What [`Sim::spawn_template`] made.
#[derive(Clone, PartialEq, Debug)]
pub struct Spawned {
    /// The entity.
    pub entity: EntityId,
    /// The traits it is to start with, as written, for whoever gives
    /// traits.
    pub traits: Vec<String>,
    /// What it is to start with, as written, for the item package.
    pub items: Vec<String>,
    /// The parts `assemble` choices chose, not yet added to its body.
    pub assembled: Vec<BodyPartId>,
    /// What is left of its body destroyed entirely, as written.
    pub remains: Option<String>,
}

impl Sim {
    /// The templates' tables for the loaded definitions.
    pub fn template_defs(&self) -> &Arc<TemplateDefs> {
        self.life.templates()
    }

    /// Makes an entity from a character spec and puts it at a place
    /// (06 §10.3): see the [module documentation](self) for what is done
    /// in what order. Returns the entity and what it is to start with
    /// that other packages give.
    ///
    /// Refused, changing nothing, for a spec that does not resolve
    /// ([`TemplateError`]) and for a place its footprint cannot take.
    pub fn spawn_template(
        &mut self,
        spec: &CharacterSpec,
        place: Place,
        cause: Cause,
    ) -> Result<Spawned, Reason> {
        let resolved = resolve(self.defs.as_ref(), spec).map_err(|error| error.reason())?;
        let plans = self.life.body_defs().ok();
        let footprint = match (resolved.body, &plans) {
            (Some(plan), Some(defs)) => defs
                .plan(plan)
                .map_or(Footprint::SINGLE, |plan| plan.footprint()),
            (Some(_), None) => {
                return Err(self.life.body_defs().err().unwrap_or_else(|| {
                    Reason::new("bodies_not_registered", "bodies are not registered")
                }));
            }
            (None, _) => Footprint::SINGLE,
        };
        let place = self.check_place(None, footprint, place)?;
        let has_stats = resolved.body.is_some()
            || !resolved.attributes.is_empty()
            || !resolved.pools.is_empty()
            || !resolved.tracks.is_empty();
        if has_stats {
            self.life.stat_defs()?;
        }

        let flags = if resolved.blocks {
            OccupantFlags::BLOCKS_MOVE
        } else {
            OccupantFlags::NONE
        };
        let mut entity_spec = EntitySpec::new(OccupantKind::Creature).with_presence(
            Presence::new(OccupantKind::Creature)
                .with_flags(flags)
                .with_footprint(footprint),
        );
        for tag in &resolved.tags {
            entity_spec = entity_spec.with_tag(tag.as_str());
        }
        for (name, value) in &resolved.variables {
            entity_spec.vars.set(name.as_str(), value.to_stored());
        }
        let entity = self.create_entity(entity_spec, cause)?;
        match self.furnish(entity, &resolved, has_stats, cause) {
            Ok(()) => {}
            Err(reason) => {
                let _ = self.remove_entity(entity, cause);
                return Err(reason);
            }
        }
        if place != Place::Stored
            && let Err(reason) = self.relocate(entity, place, cause)
        {
            let _ = self.remove_entity(entity, cause);
            return Err(reason);
        }

        let names: Vec<String> = resolved
            .templates
            .iter()
            .filter_map(|id| self.life.templates().name(*id).map(str::to_owned))
            .collect();
        let mut payload = about("entity_spawned", entity)
            .with("templates", texts_field(&names))
            .with("items", texts_field(&resolved.items))
            .with("traits", texts_field(&resolved.traits));
        if let Some(loc) = self.entities.location(entity) {
            payload = payload.subject("here", Subject::Loc(loc));
        }
        if let Err(reason) = self.raise(Where::Entity(entity), cause, payload) {
            self.fault(reason);
        }
        // What it is now may stop it: a body built without what its
        // plan needs, a pool that starts beyond a stop.
        self.settle_vital(entity, cause);
        Ok(Spawned {
            entity,
            traits: resolved.traits,
            items: resolved.items,
            assembled: resolved.assembled,
            remains: resolved.remains,
        })
    }

    /// Gives a new entity, still out of the world, what a resolved spec
    /// says it is made of.
    fn furnish(
        &mut self,
        entity: EntityId,
        resolved: &Resolved,
        has_stats: bool,
        cause: Cause,
    ) -> Result<(), Reason> {
        // The body first: the stats may read what it can do and weighs.
        if let Some(plan) = resolved.body {
            self.attach_body(entity, plan, resolved.scale, cause)?;
        }
        if has_stats {
            self.attach_stats(entity, cause)?;
        }
        for (attr, base) in &resolved.attributes {
            self.set_attr_base(entity, *attr, Some(*base), cause)?;
        }
        for pool in &resolved.pools {
            self.give_pool(entity, *pool, cause)?;
        }
        for (track, amount) in &resolved.tracks {
            if *amount > Real::ZERO {
                self.advance_track(entity, *track, *amount, cause)?;
            }
        }
        let ty = self
            .exprs
            .registry()
            .def_type_id(TemplateDef::TYPE)
            .zip(resolved.templates.first())
            .and_then(|(ty, first)| Some(DefKey::new(ty, u32::try_from(first.index()).ok()?)));
        let mut identity = Identity::new(ty);
        for template in &resolved.templates {
            identity.apply_template(*template);
        }
        identity.rename(resolved.name.clone());
        identity.set_description(resolved.description.clone());
        identity.set_anonymous(false);
        self.components.identity.insert(entity, identity);
        if let Some(profile) = &resolved.mind {
            self.create_mind(entity, profile.clone(), cause)?;
        }
        // Senses where perception's definitions are loaded; the
        // template's check says whether each names one.
        for name in &resolved.senses {
            if let Some(sense) = self.perception_defs.sense_named(name) {
                crate::perception::attach_sense(self, entity, sense)?;
            }
        }
        if !resolved.rules.is_empty()
            && let Some(handle) = self.exprs.kinds().handle(Subject::Entity(entity))
        {
            for rule in &resolved.rules {
                self.rules.attach(handle, *rule);
            }
        }
        Ok(())
    }

    /// [`Sim::spawn_template`] for an anonymous individual (06 §12.2).
    pub fn spawn_anonymous(
        &mut self,
        spec: &CharacterSpec,
        place: Place,
        cause: Cause,
    ) -> Result<Spawned, Reason> {
        let spawned = self.spawn_template(spec, place, cause)?;
        if let Some(identity) = self.components.identity.get_mut(spawned.entity) {
            identity.set_anonymous(true);
        }
        Ok(spawned)
    }

    /// Answers a loaded tract's request to put something into the world
    /// (02 §9.4), if what it names is a template: an anonymous
    /// individual, its unselected choices drawn from a seed that
    /// depends only on where it stands and what it is, so that the same
    /// street has the same people in it each time it loads (06 §12.2).
    /// A request that names no template is left to others. A refusal is
    /// reported as a fault.
    pub(crate) fn honour_spawn_request(&mut self, loc: Location, request: &SpawnRequest) {
        if template_by_name(&self.defs, &request.what).is_none() {
            return;
        }
        let seed = clash_core::pos_hash(
            self.rng.seed(),
            clash_core::HashDomain::new("spawn_request"),
            loc.cell.x,
            loc.cell.y,
            loc.cell.z,
            u64::from(loc.grid.raw()),
        );
        let spec = CharacterSpec::of([request.what.clone()]).random(seed);
        let cause = Cause::new(clash_world::CauseKind::Process);
        if let Err(reason) = self.spawn_anonymous(&spec, Place::at(loc), cause) {
            self.fault(reason.with("what", request.what.as_str()));
        }
    }

    /// How an entity made from templates is drawn: the glyph of the last
    /// template applied to it that gives one.
    pub fn glyph(&self, entity: EntityId) -> Option<&GlyphDef> {
        let identity = self.components.identity.get(entity)?;
        let templates = self.life.templates();
        identity
            .templates()
            .iter()
            .rev()
            .find_map(|id| templates.get(*id)?.glyph.as_ref())
    }

    /// Whether an entity is notable (06 §12.2): anything but an
    /// anonymous individual of a population.
    pub fn is_notable(&self, entity: EntityId) -> bool {
        self.components
            .identity
            .get(entity)
            .is_none_or(|identity| !identity.is_anonymous())
    }

    /// Makes an anonymous individual notable (06 §12.2), raising
    /// `became_notable`. Nothing for one that is notable already.
    pub fn make_notable(&mut self, entity: EntityId, cause: Cause) {
        let Some(identity) = self.components.identity.get_mut(entity) else {
            return;
        };
        if identity.set_anonymous(false) {
            let payload = about("became_notable", entity);
            if let Err(reason) = self.raise(Where::Entity(entity), cause, payload) {
                self.fault(reason);
            }
        }
    }
}
