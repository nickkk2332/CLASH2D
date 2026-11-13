//! Items and holding (`docs/06-entities-and-bodies.md` section 9): things
//! that can be picked up, carried, worn, put into containers and broken.
//!
//! An item is an entity with an [`Item`] component. What is inside what
//! is the kernel's containment ([`Place::In`]); *how* a thing is held
//! there is a [`Holds`] component on the holder: in a grasp, worn over
//! parts, stowed in a container's capacity. Every move goes through
//! [`Sim::relocate`], and nothing here writes a place.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`ItemDef`] | The `item` definition type | `defs` |
//! | [`Item`] | The component: definition, material, count, damage, colour, seed, latent contents | here |
//! | [`Holds`], [`Hold`] | The component on a holder: how each thing inside it is held | [`hold`] |
//! | [`create`], [`destroy`] | Making and unmaking items | `make` |
//! | [`take`], [`give`], [`drop`], [`wear`], [`take_off`], [`put_in`], [`stow`], [`take_out`] | The holding functions | [`hold`] |
//! | [`split`], [`merge`], [`rejoin`] | Stacks | `stack` |
//! | [`damage`], [`repair`], [`worn_layers`], [`apply_worn_hits`] | Item damage, and worn items as layers for hits | `wear` |
//! | [`part_item`], [`on_body_changes`] | What a body's changes do to what it holds | `wear` |
//! | [`materialise`] | Latent contents | `latent` |
//! | [`ContentTableDef`], [`table::draw`] | The `content_table` type: what generation fills cells and containers with | [`table`] |
//! | [`properties`], [`verbs()`], [`event_types`], [`ITEM_REASONS`] | What rules read, call, listen for and are refused with | `props`, [`verbs`](mod@verbs), here |
//! | [`conferred`], [`reconfer`] | What held and worn things confer, from scratch | `confer` |
//!
//! # The rules, in short
//!
//! - **One mutation path.** An item changes place only through
//!   [`Sim::relocate`]. When anything leaves a container, by these
//!   functions or by any other caller of `relocate`, the kernel calls
//!   [`left_holder`], which forgets how it was held and takes back what
//!   it conferred. So a hold record never names something that is not
//!   there, and what an item confers is removed exactly when it leaves.
//! - **Grasps** are the channel slots a body's working parts supply
//!   (03 §6.2). An item's `grip` says how many it needs of which channel;
//!   a holder with too few free is refused.
//! - **Worn** items add layers over the parts their `wear` entries
//!   select. A part takes one item at each layer.
//! - **Conferring.** An item held in a grasp or worn confers its
//!   `modifiers`, under itself as their source (06 §4.1), and its
//!   `holder_tags`. Stowed, it confers nothing.
//! - **Stacks.** Equal items of a definition that stacks may share one
//!   entity with a count. Whatever would change one member splits it off
//!   first, and equal items in the same hold rejoin.
//! - **Damage** is the element rule (02 §12.2) with a capacity of the
//!   material's toughness times the item's volume.
//! - Nothing here asks how far away a thing is or how strong its holder
//!   is: an action's `requires` and the hooks `item_taking` and
//!   `item_leaving` do.
//!
//! # What the kernel holds and calls
//!
//! | When | The kernel |
//! |------|-----------|
//! | Building its definition types | Calls the crate's `register_defs`, which registers `item`. |
//! | An entity leaves a container | `Sim::relocate` calls [`left_holder`]. |
//! | A simulation is made or loaded | Registers [`event_types`] with its bus. |
//! | A body function returns changes | Whoever applies a hit calls [`part_item`] for each part that came off, then [`on_body_changes`] with the changes: what a part that can no longer hold was holding is dropped. |
//! | A hit is applied | Hands [`worn_layers_of`] to `body::apply_hit` and the report's `worn` to [`apply_worn_hits`]. |
//!
//! # The seam to stats
//!
//! The stats package is not yet joined to the simulation, so the one
//! function that reaches it, `confer::with_stats`, builds the stats'
//! tables for the length of a call and borrows the store and the
//! timeline out of the simulation while it runs. When the kernel offers
//! its own way to change stats, that function is the one place to
//! change. The `attr_changed` events of what an item confers are the
//! stats' to raise: the changes are handed to `confer::stat_changes`,
//! which does nothing yet.
//!
//! # The seam to bodies
//!
//! Bodies are not yet applied by the kernel either. Whoever applies a
//! hit or changes a body calls [`worn_layers_of`], [`apply_worn_hits`],
//! [`part_item`] and [`on_body_changes`] as the table above says. The
//! body package's `part_severed` and `part_destroyed` events are not yet
//! raised with the part that came off, so nothing listens for them:
//! [`part_item`] is called with the `Detached` value instead.
//!
//! # What is not built here
//!
//! - **Ageing** (06 §9.4). `ages` and `keeps` are read, checked and
//!   carried; nothing ages.
//! - **`grants`, `traits`, `rules`, `guards`, `contains`** are carried
//!   for the packages that give them effect: actions, traits, the rule
//!   runtime, exposure and substances.
//! - **`round`, `interactions`, `device`, `look`** are not fields yet:
//!   they arrive with ammunition (07 §11.5), devices (16) and looks
//!   (12 §4.4).
//! - **`content_table`** definitions are built ([`table`]) and drawn from
//!   when generation furnishes a cell ([`crate::furnish`]); an item's own
//!   `contents` list still names items only.
//! - **Seats and fixtures.** [`Hold::Seat`] and [`Hold::Fixed`] can be
//!   recorded with [`hold::place_in`]; what they mean belongs to
//!   vehicles and devices.

mod confer;
mod defs;
pub mod fault;
pub mod hold;
mod latent;
mod make;
mod props;
mod stack;
pub mod table;
pub mod verbs;
mod wear;

use clash_core::{EntityId, impl_state_hash};
use clash_data::{Def, DefTypes, Defs};
use clash_expr::{EventSpec, Registry, Type};
use clash_world::defs::MaterialId;
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::action::ChannelDef;
use crate::body::Detached;
use crate::bus::{EventType, FieldSpec, FieldType};
use crate::event::CAUSE_SUBJECTS;
use crate::reason::{Reason, ReasonInfo};

pub use confer::{Conferred, conferred, reconfer};
pub use defs::{
    ALL_PARTS, AgesDef, ColorChoice, ContentDef, CountRange, HoldsDef, ItemDef, ItemId, KeepsDef,
    MAX_WEAR_ENTRIES, ModifierDef, PartSelector, StageDef, StoreOrder, Wear, WearDef,
};
pub use hold::{
    GraspSlot, Hold, Holds, WornOn, drop, free_grasp, give, hold_of, holder_of, left_holder,
    next_in_store, put_in, store_order, stow, take, take_off, take_out, take_with, wear,
};
pub use latent::materialise;
pub use make::{ITEM_STREAM, create, create_seeded, destroy, set_color, spend};
pub use props::properties;
pub use stack::{merge, rejoin, split};
pub use table::{ContentTableDef, ContentTableId, Drawn, MAX_TABLE_DEPTH, TableEntryDef};
pub use verbs::verbs;
pub use wear::{
    DAMAGE_PARTS, ItemHit, apply_worn_hits, damage, on_body_changes, part_item, repair,
    worn_layers, worn_layers_of,
};

#[cfg(doc)]
use crate::entity::Place;

/// Registers the `item` definition type. Called from the crate's
/// `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<ItemDef>();
    types
        .register::<ContentTableDef>()
        .add_cross_check(table::check_table_loops)
        .add_cross_check(crate::furnish::check_map_items);
}

/// Enters the definition types the properties and verbs of this module
/// name (`item`, and `channel` for grasps) into an expression registry,
/// those it lacks. Called when the engine's registry is built, before
/// its properties.
pub fn register_types(registry: &mut Registry) {
    for (name, doc) in [
        (ItemDef::TYPE, ItemDef::DOC),
        (ChannelDef::TYPE, ChannelDef::DOC),
    ] {
        if registry.def_type_id(name).is_none() {
            let _ = registry.add_def_type(name, doc);
        }
    }
}

/// What makes a thing an item (06 §9.1): the `Item` component.
///
/// An item starts from its definition and is then live data: its
/// material, its damage and its colour are its own. Its mass and size
/// are in its `Physical`, which the functions here keep at one member's
/// times the count.
///
/// Its fields change only through the functions of this module.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Item {
    /// The definition it was made from. `None` for what was never
    /// defined as an item: a part that came off a body.
    def: Option<ItemId>,
    /// What it is made of.
    material: MaterialId,
    /// How many equal members share this entity. At least one.
    count: u32,
    /// How damaged it is, in 255ths of what its material and volume can
    /// take. 255 is broken.
    damage: u8,
    /// The colour it carries, by name.
    color: Option<String>,
    /// What its latent contents and its colour are drawn from.
    seed: u64,
    /// Whether it has contents that have not been made yet (06 §9.3).
    latent: bool,
    /// For a part that came off a body: the part as it was.
    remains: Option<Box<Detached>>,
}

impl_state_hash!(Item {
    def,
    material,
    count,
    damage,
    color,
    seed,
    latent,
    remains
});

impl Item {
    /// The definition it was made from, if it was made from one.
    pub fn def(&self) -> Option<ItemId> {
        self.def
    }

    /// What it is made of.
    pub fn material(&self) -> MaterialId {
        self.material
    }

    /// How many equal members share this entity.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// How damaged it is, in 255ths.
    pub fn damage(&self) -> u8 {
        self.damage
    }

    /// Whether it is broken: its damage is all 255 parts.
    pub fn is_broken(&self) -> bool {
        self.damage == DAMAGE_PARTS
    }

    /// The colour it carries, by name.
    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }

    /// What its latent contents and its colour are drawn from.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Whether it has contents that have not been made yet.
    pub fn is_latent(&self) -> bool {
        self.latent
    }

    /// For a part that came off a body: the part as it was, for whoever
    /// grafts it back (06 §6.5).
    pub fn remains(&self) -> Option<&Detached> {
        self.remains.as_deref()
    }

    /// An item of no definition that differs by `seed`: for the tests of
    /// the component store.
    #[cfg(test)]
    pub(crate) fn sample(seed: u32) -> Item {
        Item {
            def: ItemId::from_index(seed as usize),
            material: MaterialId::new(0),
            count: seed.max(1),
            damage: 0,
            color: Some(format!("colour {seed}")),
            seed: u64::from(seed),
            latent: false,
            remains: None,
        }
    }
}

/// The stable codes of the reasons this module refuses for (14 §5). Each
/// is listed in [`ITEM_REASONS`].
pub mod reason {
    /// The id is not an `item` definition. Values: `def`.
    pub const UNKNOWN_DEF: &str = "item.unknown_def";
    /// The entity is not an item. Values: `entity`.
    pub const NOT_AN_ITEM: &str = "item.not_an_item";
    /// A count is zero, or more than there are. Values: `count`, `have`.
    pub const BAD_COUNT: &str = "item.bad_count";
    /// More than one of something that does not stack. Values: `item`.
    pub const NOT_STACKABLE: &str = "item.not_stackable";
    /// Two items are not equal in every respect, or not in the same
    /// hold. Values: `item`, `into`.
    pub const NOT_EQUAL: &str = "item.not_equal";
    /// The thing has no `grip`. Values: `item`.
    pub const NO_GRIP: &str = "item.no_grip";
    /// The holder's working parts supply too few free slots. Values:
    /// `holder`, `channel`, `needed`, `free`.
    pub const NO_FREE_GRASP: &str = "item.no_free_grasp";
    /// The holder has no body to hold or wear with. Values: `entity`.
    pub const NO_BODY: &str = "item.no_body";
    /// The item has no `wear`. Values: `item`.
    pub const NOT_WEARABLE: &str = "item.not_wearable";
    /// The wearer has no part the item goes on. Values: `item`,
    /// `wearer`.
    pub const NO_PART: &str = "item.no_part_to_wear_on";
    /// A part is already covered at that layer. Values: `item`, `part`,
    /// `layer`, `by`.
    pub const LAYER_TAKEN: &str = "item.layer_taken";
    /// The thing has no `holds`. Values: `container`.
    pub const NOT_A_CONTAINER: &str = "item.not_a_container";
    /// The container takes none of the thing's tags. Values: `item`,
    /// `container`.
    pub const NOT_ACCEPTED: &str = "item.not_accepted";
    /// The thing is too large or too heavy for what room is left.
    /// Values: `item`, `container`, `what`, `needed`, `room`.
    pub const DOES_NOT_FIT: &str = "item.does_not_fit";
    /// The thing is inside nothing. Values: `item`.
    pub const NOT_HELD: &str = "item.not_held";
    /// The item is not worn. Values: `item`.
    pub const NOT_WORN: &str = "item.not_worn";
    /// The thing is not stowed in or enclosed by anything. Values:
    /// `item`.
    pub const NOT_CONTAINED: &str = "item.not_contained";
    /// Nothing the holder carries would take the thing. Values: `who`,
    /// `item`.
    pub const NO_CONTAINER: &str = "item.no_container";
    /// A share or a packet's number is negative or not finite. Values:
    /// `amount`.
    pub const BAD_AMOUNT: &str = "item.bad_amount";
    /// The definitions were loaded without the `item` type. Values:
    /// none.
    pub const NOT_REGISTERED: &str = "item.not_registered";
    /// A verb's argument is missing or of the wrong kind. Values: `arg`.
    pub const BAD_ARGUMENT: &str = "item.bad_argument";
}

/// The reasons this module refuses for, for the generated reference.
pub const ITEM_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: reason::UNKNOWN_DEF,
        doc: "An item was to be made from an id that is not an `item` definition. Values: `def`.",
    },
    ReasonInfo {
        code: reason::NOT_AN_ITEM,
        doc: "The entity has no `Item` component. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::BAD_COUNT,
        doc: "A count is zero, or is not less than the stack holds where some must stay. Values: `count`, `have`.",
    },
    ReasonInfo {
        code: reason::NOT_STACKABLE,
        doc: "More than one was asked of a definition that does not `stack`. Values: `item`.",
    },
    ReasonInfo {
        code: reason::NOT_EQUAL,
        doc: "Two items were to be merged that are not equal in every respect, or are not in the same hold. Values: `item`, `into`.",
    },
    ReasonInfo {
        code: reason::NO_GRIP,
        doc: "The thing has no `grip`, so it cannot be held in a grasp. Values: `item`.",
    },
    ReasonInfo {
        code: reason::NO_FREE_GRASP,
        doc: "The holder's working parts supply fewer free slots of a channel than the thing's `grip` needs. Values: `holder`, `channel`, `needed`, `free`.",
    },
    ReasonInfo {
        code: reason::NO_BODY,
        doc: "Something with no body was to hold a thing in a grasp or wear it. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::NOT_WEARABLE,
        doc: "The item has no `wear`. Values: `item`.",
    },
    ReasonInfo {
        code: reason::NO_PART,
        doc: "The wearer's body has no outer part that the item's `wear` selects. Values: `item`, `wearer`.",
    },
    ReasonInfo {
        code: reason::LAYER_TAKEN,
        doc: "A part the item would cover already wears something at that layer. Values: `item`, `part`, `layer`, `by`.",
    },
    ReasonInfo {
        code: reason::NOT_A_CONTAINER,
        doc: "Something was to be put into a thing whose definition has no `holds`. Values: `container`.",
    },
    ReasonInfo {
        code: reason::NOT_ACCEPTED,
        doc: "The container's `accepts` lists none of the thing's tags. Values: `item`, `container`.",
    },
    ReasonInfo {
        code: reason::DOES_NOT_FIT,
        doc: "The thing takes more volume or weighs more than the container has room for. Values: `item`, `container`, `what` (`volume` or `mass`), `needed`, `room`.",
    },
    ReasonInfo {
        code: reason::NOT_HELD,
        doc: "The thing is inside nothing, so it cannot be dropped or given. Values: `item`.",
    },
    ReasonInfo {
        code: reason::NOT_WORN,
        doc: "An item that is not worn was to be taken off. Values: `item`.",
    },
    ReasonInfo {
        code: reason::NOT_CONTAINED,
        doc: "A thing that is not stowed in or enclosed by anything was to be taken out. Values: `item`.",
    },
    ReasonInfo {
        code: reason::NO_CONTAINER,
        doc: "Nothing the holder wears or holds would take the thing. Values: `who`, `item`.",
    },
    ReasonInfo {
        code: reason::BAD_AMOUNT,
        doc: "A share to repair, or a packet's energy or intensity, is negative or not finite. Values: `amount`.",
    },
    ReasonInfo {
        code: reason::NOT_REGISTERED,
        doc: "The definitions were loaded without the `item` definition type. Values: none.",
    },
    ReasonInfo {
        code: reason::BAD_ARGUMENT,
        doc: "A verb of items was given an argument that is missing or of the wrong kind. Values: `arg`, the argument's name.",
    },
];

/// A refusal with one of this module's codes.
pub(crate) fn refuse(code: &'static str, message: impl Into<String>) -> Reason {
    Reason::new(code, message)
}

/// The refusal for something that is not an item.
pub(crate) fn not_an_item(entity: EntityId) -> Reason {
    refuse(reason::NOT_AN_ITEM, format!("{entity} is not an item"))
        .with("entity", entity.to_string())
}

/// The item component of an entity, or the refusal that it has none.
pub(crate) fn item_of(sim: &Sim, entity: EntityId) -> Result<&Item, Reason> {
    sim.known(entity)?;
    sim.components
        .get::<Item>(entity)
        .ok_or_else(|| not_an_item(entity))
}

/// The `item` definition with a qualified id, or the refusal that there
/// is none (`item.unknown_def`).
pub fn def_named(sim: &Sim, name: &str) -> Result<ItemId, Reason> {
    sim.defs
        .try_registry::<ItemDef>()
        .and_then(|registry| registry.lookup(name))
        .ok_or_else(|| {
            refuse(
                reason::UNKNOWN_DEF,
                format!("there is no item definition `{name}`"),
            )
            .with("def", name)
        })
}

/// The definition an item was made from, if it was made from one.
pub fn def_of<'a>(defs: &'a Defs, item: &Item) -> Option<&'a ItemDef> {
    defs.try_registry::<ItemDef>()?.try_get(item.def?)
}

// ---- Events ------------------------------------------------------------------

/// The hook that runs before an item enters a hold.
pub const ITEM_TAKING: &str = "item_taking";
/// The hook that runs before an item leaves a hold.
pub const ITEM_LEAVING: &str = "item_leaving";
/// An item entered a hold other than a worn one.
pub const ITEM_TAKEN: &str = "item_taken";
/// An item left whatever held it and lies where its holder was.
pub const ITEM_DROPPED: &str = "item_dropped";
/// An item was put on.
pub const ITEM_WORN: &str = "item_worn";
/// A worn item came off.
pub const ITEM_REMOVED: &str = "item_removed";
/// An item's damage rose.
pub const ITEM_DAMAGED: &str = "item_damaged";
/// An item's damage fell.
pub const ITEM_REPAIRED: &str = "item_repaired";
/// An item's damage reached all 255 parts.
pub const ITEM_BROKEN: &str = "item_broken";
/// Members of a stack were split off as an entity of their own.
pub const ITEM_SPLIT: &str = "item_split";
/// Equal items joined into one stack.
pub const ITEM_MERGED: &str = "item_merged";
/// A container's latent contents were made.
pub const CONTENTS_MATERIALISED: &str = "contents_materialised";

/// One event of this module, as both registries are told of it.
struct EventRow {
    name: &'static str,
    doc: &'static str,
    /// Name and kind. A name ending in `?` is bound only when there is
    /// one.
    subjects: &'static [(&'static str, &'static str)],
    /// Name, type as the language writes it, and what it holds.
    fields: &'static [(&'static str, &'static str, &'static str)],
    hook: bool,
}

const HOLD_FIELD: (&str, &str, &str) = (
    "hold",
    "string",
    "The kind of hold: `grasp`, `worn`, `stowed`, `seat`, `inside` or `fixed`.",
);

const EVENTS: &[EventRow] = &[
    EventRow {
        name: ITEM_TAKING,
        doc: "Runs before a thing enters a hold: a grasp, being worn, a container. A handler may cancel: a thing that will not be picked up. Not raised for a thing put somewhere by the kernel's `relocate` alone.",
        subjects: &[("self", "entity"), ("holder", "entity")],
        fields: &[HOLD_FIELD],
        hook: true,
    },
    EventRow {
        name: ITEM_LEAVING,
        doc: "Runs before a thing leaves the hold it is in, with `holder` what holds it now. A handler may cancel: a thing that will not come off. Not raised when a part that can no longer hold lets go, nor when the holder is removed.",
        subjects: &[("self", "entity"), ("holder", "entity")],
        fields: &[HOLD_FIELD],
        hook: true,
    },
    EventRow {
        name: ITEM_TAKEN,
        doc: "A thing entered a hold other than a worn one: it was taken into a grasp, stowed, or put inside. `from` is what held it before, if anything did.",
        subjects: &[
            ("self", "entity"),
            ("holder", "entity"),
            ("from?", "entity"),
        ],
        fields: &[
            HOLD_FIELD,
            ("count", "int", "How many members the entity has."),
        ],
        hook: false,
    },
    EventRow {
        name: ITEM_DROPPED,
        doc: "A thing left whatever held it and lies where its holder was: in the holder's cell, or inside whatever encloses the holder. The cause says who or what made it happen.",
        subjects: &[("self", "entity"), ("holder", "entity")],
        fields: &[("count", "int", "How many members the entity has.")],
        hook: false,
    },
    EventRow {
        name: ITEM_WORN,
        doc: "An item was put on: it now covers parts of `holder` and is a layer for hits on them.",
        subjects: &[("self", "entity"), ("holder", "entity")],
        fields: &[("parts", "int", "How many parts it covers.")],
        hook: false,
    },
    EventRow {
        name: ITEM_REMOVED,
        doc: "A worn item came off `holder`. `item_taken` or `item_dropped` follows, saying where it went.",
        subjects: &[("self", "entity"), ("holder", "entity")],
        fields: &[],
        hook: false,
    },
    EventRow {
        name: ITEM_DAMAGED,
        doc: "An item's damage rose, by a packet or by what it absorbed as a worn layer.",
        subjects: &[("self", "entity")],
        fields: &[
            ("old", "int", "Its damage before, in 255ths."),
            ("new", "int", "Its damage now."),
            (
                "kind",
                "string",
                "The kind of damage: the qualified id of a `damage_type`.",
            ),
        ],
        hook: false,
    },
    EventRow {
        name: ITEM_REPAIRED,
        doc: "An item's damage fell.",
        subjects: &[("self", "entity")],
        fields: &[
            ("old", "int", "Its damage before, in 255ths."),
            ("new", "int", "Its damage now."),
        ],
        hook: false,
    },
    EventRow {
        name: ITEM_BROKEN,
        doc: "An item's damage reached all 255 parts. It is still there: it adds no layer when worn, and what else being broken means is a rule's to say.",
        subjects: &[("self", "entity")],
        fields: &[],
        hook: false,
    },
    EventRow {
        name: ITEM_SPLIT,
        doc: "Members of a stack were split off as an entity of their own, in the same place and hold.",
        subjects: &[("self", "entity"), ("piece", "entity")],
        fields: &[("count", "int", "How many members the piece has.")],
        hook: false,
    },
    EventRow {
        name: ITEM_MERGED,
        doc: "Equal items joined into one stack. `self` is the stack that remains; the entity that joined it is gone.",
        subjects: &[("self", "entity")],
        fields: &[("count", "int", "How many members joined.")],
        hook: false,
    },
    EventRow {
        name: CONTENTS_MATERIALISED,
        doc: "A container's latent contents were made (06 §9.3): it was opened, searched or hit for the first time.",
        subjects: &[("self", "entity")],
        fields: &[("count", "int", "How many entities were made.")],
        hook: false,
    },
];

/// What a raised event's field of a written type is checked against.
fn field_check(written: &str) -> FieldType {
    FieldType::parse(written).unwrap_or(FieldType::Any)
}

/// The event types of this module, for the simulation's bus (05 §8.1).
pub fn event_types() -> Vec<EventType> {
    EVENTS
        .iter()
        .map(|row| {
            let mut ty = EventType::new(row.name).doc(row.doc);
            for (name, kind) in row.subjects {
                ty = ty.subject(*name, *kind);
            }
            ty.fields = row
                .fields
                .iter()
                .map(|(name, written, doc)| FieldSpec {
                    name: (*name).to_owned(),
                    ty: (*written).to_owned(),
                    check: Some(field_check(written)),
                    doc: (*doc).to_owned(),
                })
                .collect();
            if row.hook {
                ty = ty.hook(Vec::<String>::new());
            }
            ty
        })
        .collect()
}

/// Enters this module's events into an expression registry, leaving
/// alone those it already holds. Called when the engine's registry is
/// built.
pub fn register_events(registry: &mut Registry) {
    for row in EVENTS {
        if registry.event_id(row.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(row.name).doc(row.doc);
        for (name, kind) in row.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in row.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        if row.hook {
            spec = spec.hook(Vec::<String>::new());
        }
        let _ = registry.add_event(spec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut codes: Vec<&str> = ITEM_REASONS.iter().map(|info| info.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count, "a code is listed twice");
        for info in ITEM_REASONS {
            assert!(info.code.starts_with("item."));
            assert!(!info.doc.is_empty());
        }
    }

    #[test]
    fn every_event_registers_in_both_registries() {
        let mut registry = crate::engine_registry();
        register_events(&mut registry);
        for ty in event_types() {
            let id = registry
                .event_id(&ty.name)
                .unwrap_or_else(|| panic!("`{}` is not in the expression registry", ty.name));
            let spec = &registry.event(id).unwrap().spec;
            assert_eq!(spec.fields.len(), ty.fields.len(), "{}", ty.name);
            assert!(!ty.doc.is_empty());
            for field in &ty.fields {
                assert!(!field.doc.is_empty(), "{}.{}", ty.name, field.name);
            }
        }
    }
}
