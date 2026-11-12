//! The social layer (`docs/15-perception-and-social.md` sections 8 to
//! 12): who someone is taken for, how minds stand toward each other and
//! toward groups, orders, and speech that carries what one knows to
//! another.
//!
//! Everything here is held in the mind that holds it, by persona, never
//! by entity (15 invariant P2): a disguise changes whom a witness blames
//! without changing how anything is kept, and the world's cause records
//! still name the real actor (scene S15).
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`RelationKindDef`], [`GroupDef`], [`MessageKindDef`], [`LinesDef`] | The definition types `relation_kind`, `group`, `message_kind` and `lines` | [`defs`] |
//! | [`presenting`], [`persona_of`], [`create_persona`], [`assume_persona`], [`copy_persona`], [`reveal_persona`], [`create_illusion`] | Personas: what an entity presents, what an observer takes it for, disguises and seeing through them | [`persona`] |
//! | [`rel`], [`stance_to`], [`intervenes`], [`set_relation`] | Relations held by a mind toward a persona or a group, with fade and group fallback | [`relation`] |
//! | [`join_group`], [`report_to_group`], [`create_group`] | Groups, roles, claims and group knowledge | [`group`] |
//! | [`give_order`], [`standing_orders`], [`cancel_order`] | Orders: given aloud, kept by whoever obeys | [`order`] |
//! | [`say`], [`Message`], [`talk_options`] | Speech: messages carried by a signal, reports, `message_heard` | [`speech`] |
//! | [`social_lens`] | What a reader calls someone: a known name, else a description | [`lens`] |
//!
//! # What runs when
//!
//! | When | What |
//! |------|------|
//! | An observer detects an entity (15 §8.2) | [`identify`]: the hook `identity_check` decides whether a disguise holds for it; seen through, the observer takes it for its true persona and `disguise_seen_through` is raised |
//! | An observer stops perceiving an entity through every sense | [`lost_sight`]: it is taken for whatever it presents when next it is detected |
//! | An entity presents another persona | Every observer that perceives it at that moment sees the change: it goes on taking it for whom it took it for |
//! | A signal carrying a message is perceived (15 §11) | [`speech::heard`]: `understands`, `message_heard`, a report's beliefs, an order |
//! | An entity is made from templates | It joins the groups its templates name ([`TemplateDef`](crate::template::TemplateDef)'s `groups`) |
//! | An entity is removed | What the social layer kept of it goes |
//!
//! # What the kernel holds and calls
//!
//! The simulation holds a [`SocialState`], saved and hashed with the
//! kernel's state. Perception calls [`persona_of`], [`identify`],
//! [`lost_sight`] and [`speech::heard`]; the reader lens every simulation
//! is bound with is [`social_lens`] (05 §4.6).

pub mod defs;
mod events;
pub mod group;
pub mod lens;
pub mod order;
pub mod persona;
pub(crate) mod props;
pub mod relation;
pub mod speech;

use std::collections::BTreeMap;

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, Tick, impl_state_hash};
use clash_data::DefTypes;
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::bus::Handler;
use crate::perception::{Memory, PersonaId};
use crate::reason::ReasonInfo;

pub use defs::{
    Carries, GroupDef, GroupDefId, LineDef, LinesDef, LinesId, MessageKindDef, MessageKindId,
    RelationKindDef, RelationKindId, RelationShape, RoleDef,
};
pub use events::{SOCIAL_EVENTS, event_types, hook_types, register_types};
pub use group::{
    Group, Member, claims_of, create_group, dissolve_group, group_key, groups_of, join_group,
    leave_group, report_to_group, set_group_relation, set_role,
};
pub use lens::social_lens;
pub use order::{Order, OrderPlan, cancel_order, give_order, standing_orders};
pub use persona::{
    PersonaRecord, PersonaSpec, assume_persona, copy_persona, create_illusion, create_persona,
    drop_persona, knows_name, learn_name, persona_name, persona_of, persona_tags, presenting,
    reveal_persona, set_persona,
};
pub(crate) use persona::{identify, lost_sight};
pub use props::properties;
pub use relation::{
    Relation, Stance, Toward, add_relation, avoids, clear_relation, intervenes, rel, rel_toward,
    set_relation, stance_to,
};
pub use speech::{Message, Report, SayArgs, TalkOption, say, talk_options};

/// The random stream speech draws its lines from (01 §4).
pub const SPEECH_STREAM: &str = "social";

/// The stable codes of the reasons the social layer refuses for.
pub mod reason {
    /// A text that names no persona, or a persona that is gone. Values:
    /// `persona`.
    pub const UNKNOWN_PERSONA: &str = "social.unknown_persona";
    /// A persona that is not the entity's own to present. Values:
    /// `entity`, `persona`.
    pub const NOT_OWN_PERSONA: &str = "social.not_own_persona";
    /// A relation kind the definitions do not have. Values: `kind`.
    pub const UNKNOWN_RELATION_KIND: &str = "social.unknown_relation_kind";
    /// A group that neither a definition nor play has made. Values:
    /// `group`.
    pub const UNKNOWN_GROUP: &str = "social.unknown_group";
    /// A role the group's definition does not have. Values: `group`,
    /// `role`.
    pub const UNKNOWN_ROLE: &str = "social.unknown_role";
    /// An entity that is not a member of the group. Values: `entity`,
    /// `group`.
    pub const NOT_MEMBER: &str = "social.not_member";
    /// A message kind the definitions do not have. Values: `kind`.
    pub const UNKNOWN_MESSAGE_KIND: &str = "social.unknown_message_kind";
    /// A `lines` table the definitions do not have. Values: `lines`.
    pub const UNKNOWN_LINES: &str = "social.unknown_lines";
    /// Something said by one who is not in the world, or with no kind of
    /// signal to say it with. Values: `entity`.
    pub const NO_VOICE: &str = "social.no_voice";
    /// An argument that is missing or of the wrong sort, or a number out
    /// of its range. Values: `argument`.
    pub const BAD_ARGUMENT: &str = "social.bad_argument";
    /// An order whose giver the hearer holds no relation flagged
    /// `orders` toward. Values: `entity`.
    pub const NOT_OBEYED: &str = "social.not_obeyed";
    /// An order from someone the hearer did not make out. Values:
    /// `entity`.
    pub const GIVER_UNKNOWN: &str = "social.giver_unknown";
    /// A message kind that does not carry orders, given to `order`.
    /// Values: `kind`.
    pub const NOT_AN_ORDER: &str = "social.not_an_order";
    /// An order the entity does not keep. Values: `entity`, `order`.
    pub const UNKNOWN_ORDER: &str = "social.unknown_order";
    /// The definitions were loaded without the social types. Values:
    /// `type`.
    pub const NOT_REGISTERED: &str = "social.not_registered";
}

/// Every reason the social layer refuses for, for the generated
/// reference.
pub const SOCIAL_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: reason::UNKNOWN_PERSONA,
        doc: "A text that names no persona, or a persona whose owner is gone. Values: `persona`.",
    },
    ReasonInfo {
        code: reason::NOT_OWN_PERSONA,
        doc: "`assume_persona` named a persona that is not the entity's own: one it made, or its true one. Values: `entity`, `persona`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_RELATION_KIND,
        doc: "The relation kind named is not one of the loaded definitions'. Values: `kind`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_GROUP,
        doc: "The group named is neither a `group` definition nor one made in play and not dissolved. Values: `group`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_ROLE,
        doc: "The role named is not one of the group definition's roles. Values: `group`, `role`.",
    },
    ReasonInfo {
        code: reason::NOT_MEMBER,
        doc: "The entity is not a member of the group. Values: `entity`, `group`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_MESSAGE_KIND,
        doc: "The message kind named is not one of the loaded definitions'. Values: `kind`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_LINES,
        doc: "The `lines` table named is not one of the loaded definitions'. Values: `lines`.",
    },
    ReasonInfo {
        code: reason::NO_VOICE,
        doc: "Something was to be said by one who is not in the world, or no kind of signal says it: the message kind names none and no kind is flagged `noise`. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::BAD_ARGUMENT,
        doc: "An argument of a social verb is missing or of the wrong sort, or a number is out of its range. Values: `argument`.",
    },
    ReasonInfo {
        code: reason::NOT_OBEYED,
        doc: "An order was heard by one who holds no relation of a kind flagged `orders` toward the one it took for the giver, or a handler of `order_received` said it is not obeyed. The reason `order_refused` carries. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::GIVER_UNKNOWN,
        doc: "An order was heard from someone the hearer did not make out: it cannot tell whose order it is. The reason `order_refused` carries. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::NOT_AN_ORDER,
        doc: "`order` was given a message kind that does not carry orders. Values: `kind`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_ORDER,
        doc: "`cancel_order` named an order the entity does not keep. Values: `entity`, `order`.",
    },
    ReasonInfo {
        code: reason::NOT_REGISTERED,
        doc: "The definitions were loaded without the social definition types. Values: `type`.",
    },
];

/// Registers the definition types `relation_kind`, `group`,
/// `message_kind` and `lines`, and the check of what groups name. Called
/// from the crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<RelationKindDef>()
        .register::<GroupDef>()
        .register::<MessageKindDef>()
        .register::<LinesDef>()
        .add_cross_check(defs::check_group_relations)
        .add_cross_check(defs::check_template_groups);
}

// ---- State ---------------------------------------------------------------------

/// Who an observer took an entity for when it last detected it, and
/// what the entity presented then (15 §8.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct SeenAs {
    pub(crate) entity: EntityId,
    pub(crate) shown: PersonaId,
    pub(crate) taken: PersonaId,
}

impl_state_hash!(SeenAs {
    entity,
    shown,
    taken
});

/// What the social layer keeps for one mind: whom it has seen through,
/// whose names it knows, its relations and the orders it keeps.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub(crate) struct MindSocial {
    /// Personas it knows to be another: an own persona of someone, seen
    /// through, and that one's true persona. In order of the first.
    pub(crate) links: Vec<(PersonaId, PersonaId)>,
    /// Who it took each entity it perceives now for, in order of entity.
    pub(crate) seen_as: Vec<SeenAs>,
    /// The personas whose names it knows, in order.
    pub(crate) names: Vec<PersonaId>,
    /// Its relations, in order of whom and then kind.
    pub(crate) relations: Vec<Relation>,
    /// The orders it keeps, in the order received.
    pub(crate) orders: Vec<Order>,
}

impl_state_hash!(MindSocial {
    links,
    seen_as,
    names,
    relations,
    orders
});

impl MindSocial {
    fn is_empty(&self) -> bool {
        self.links.is_empty()
            && self.seen_as.is_empty()
            && self.names.is_empty()
            && self.relations.is_empty()
            && self.orders.is_empty()
    }
}

/// What the social layer keeps (15 §8 to §11): personas made and what
/// each entity presents, what each mind holds, groups, and the numbers
/// the next persona, group, message and order take. Saved and hashed
/// with the kernel; an empty one hashes as nothing, so that a world
/// with no social life hashes as it did before.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct SocialState {
    /// Personas made, and true personas something was set on.
    pub(crate) personas: BTreeMap<PersonaId, PersonaRecord>,
    /// The number the next persona made takes; never 0.
    pub(crate) next_persona: u32,
    /// What each entity presents, where it is not its true persona.
    pub(crate) presenting: BTreeMap<EntityId, persona::Presented>,
    /// What each mind holds.
    pub(crate) minds: BTreeMap<EntityId, MindSocial>,
    /// Every group with something of its own, by key.
    pub(crate) groups: BTreeMap<String, Group>,
    /// Keys of groups dissolved: a definition's group that was dissolved
    /// is not made again by being named.
    pub(crate) dissolved: Vec<String>,
    /// The number the next group made in play takes.
    pub(crate) next_group: u64,
    /// The number the next message takes.
    pub(crate) next_message: u64,
    /// The number the next order takes.
    pub(crate) next_order: u64,
}

impl SocialState {
    /// Whether nothing at all is kept.
    pub fn is_empty(&self) -> bool {
        self.personas.is_empty()
            && self.next_persona == 0
            && self.presenting.is_empty()
            && self.minds.is_empty()
            && self.groups.is_empty()
            && self.dissolved.is_empty()
            && self.next_group == 0
            && self.next_message == 0
            && self.next_order == 0
    }

    /// Every group with something of its own, by key.
    pub fn groups(&self) -> &BTreeMap<String, Group> {
        &self.groups
    }
}

impl StateHash for SocialState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        if self.is_empty() {
            return;
        }
        hasher.write_u8(1);
        self.personas.state_hash(hasher);
        hasher.write_u32(self.next_persona);
        self.presenting.state_hash(hasher);
        self.minds.state_hash(hasher);
        self.groups.state_hash(hasher);
        self.dissolved.state_hash(hasher);
        hasher.write_u64(self.next_group);
        hasher.write_u64(self.next_message);
        hasher.write_u64(self.next_order);
    }
}

impl Sim {
    /// What the social layer keeps (15 §8 to §11).
    pub fn social(&self) -> &SocialState {
        &self.social
    }
}

/// What a mind holds, if anything.
pub(crate) fn mind(sim: &Sim, entity: EntityId) -> Option<&MindSocial> {
    sim.social.minds.get(&entity)
}

/// What a mind holds, to change; made empty if it held nothing.
pub(crate) fn mind_mut(sim: &mut Sim, entity: EntityId) -> &mut MindSocial {
    sim.social.minds.entry(entity).or_default()
}

/// Forgets a mind's record if it holds nothing.
pub(crate) fn tidy(sim: &mut Sim, entity: EntityId) {
    if sim
        .social
        .minds
        .get(&entity)
        .is_some_and(MindSocial::is_empty)
    {
        sim.social.minds.remove(&entity);
    }
}

/// A mind's own time now, or zero for one on no clock.
pub(crate) fn local_now(sim: &Sim, entity: EntityId) -> Dur {
    sim.clock_of(entity)
        .and_then(|clock| sim.timeline.local_now(clock).ok())
        .unwrap_or(Dur::ZERO)
}

/// Universal time now, as a length since the start: what groups, which
/// have no clock, keep time by.
pub(crate) fn universal_now(sim: &Sim) -> Dur {
    sim.now().since(Tick::ZERO)
}

/// Sets up what is not state: the reader lens and the handlers through
/// which made things join their groups and removed things are
/// forgotten. Called when a simulation is made and again after a load.
pub(crate) fn bind(sim: &mut Sim) {
    sim.set_reader_lens(social_lens);
    let _ = sim.on("entity_spawned", Handler::new(group::spawned));
    let _ = sim.on("entity_removed", Handler::new(removed));
}

/// An entity was removed: whatever was kept of it goes, but for the
/// personas others hold beliefs about, which stay for their names.
fn removed(sim: &mut Sim, delivery: &mut crate::bus::Delivery<'_>) {
    let crate::event::EventBody::EntityRemoved { entity } = &delivery.event().body else {
        return;
    };
    let entity = *entity;
    sim.social.presenting.remove(&entity);
    sim.social.minds.remove(&entity);
    for group in sim.social.groups.values_mut() {
        group.members.remove(&entity);
    }
    for mind in sim.social.minds.values_mut() {
        mind.seen_as.retain(|seen| seen.entity != entity);
    }
}

/// A number kept from 0 to 1.
pub(crate) fn share(value: Real) -> Real {
    if value.is_finite() {
        value.clamp(Real::ZERO, Real::ONE)
    } else {
        Real::ZERO
    }
}

/// The memories of a mind, if it has beliefs.
pub(crate) fn memories_of(sim: &Sim, entity: EntityId) -> &[Memory] {
    sim.components
        .beliefs
        .get(entity)
        .map_or(&[], |beliefs| beliefs.memories())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut found: Vec<&str> = SOCIAL_REASONS.iter().map(|info| info.code).collect();
        let count = found.len();
        found.sort_unstable();
        found.dedup();
        assert_eq!(found.len(), count, "a code is listed twice");
        for info in SOCIAL_REASONS {
            assert!(info.code.starts_with("social."));
            assert!(!info.doc.is_empty());
        }
    }

    #[test]
    fn an_empty_state_hashes_as_nothing() {
        let mut hasher = StateHasher::new();
        SocialState::default().state_hash(&mut hasher);
        assert_eq!(hasher.finish(), StateHasher::new().finish());
    }
}
