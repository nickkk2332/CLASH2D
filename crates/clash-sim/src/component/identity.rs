//! [`Identity`]: what a thing is (06 §3.1).

use clash_core::{StateHash, StateHasher, define_id, impl_state_hash};
use clash_expr::{DefRef, DefTypeId};
use serde::{Deserialize, Serialize};

define_id! {
    /// A `template` definition (06 §10.1).
    ///
    /// A typed stub: the definition type itself is package E9's, which
    /// uses this as its `Def::Id`.
    pub struct TemplateId;
}

/// A definition of any type: the type, and the definition's dense id
/// within it (04 §7.2).
///
/// An entity may be made from a `creature`, an `item` or any other type
/// of definition that a later package adds, so [`Identity`] names its
/// definition by this pair instead of by one typed id. It is the
/// expression language's [`DefRef`] in a form that can be saved, and
/// converts to and from it.
///
/// Both numbers are runtime ids. Writing them to a save by qualified id,
/// through the save's id table (01 §7), is the work of the simulation's
/// save path.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct DefKey {
    /// The type of definition, as the expression registry numbers it.
    pub ty: DefTypeId,
    /// The definition's dense id within its type.
    pub index: u32,
}

impl DefKey {
    /// The definition numbered `index` of type `ty`.
    pub const fn new(ty: DefTypeId, index: u32) -> DefKey {
        DefKey { ty, index }
    }
}

impl From<DefRef> for DefKey {
    fn from(def: DefRef) -> DefKey {
        DefKey::new(def.ty, def.index)
    }
}

impl From<DefKey> for DefRef {
    fn from(def: DefKey) -> DefRef {
        DefRef::new(def.ty, def.index)
    }
}

impl StateHash for DefKey {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.ty.state_hash(hasher);
        hasher.write_u32(self.index);
    }
}

/// What a thing is: the definition it was made from, the name and
/// description that are its own, and the templates applied to it
/// (06 §3.1).
///
/// A name or description that is absent means "as the definition says":
/// an unnamed guard is called what its `creature` definition calls it.
/// Only a thing given a name of its own, by a `text` choice, a scenario
/// or `rename`, carries one, so a thousand unnamed swords hold no text.
///
/// Tags are not here. They are the kernel entity's (03 §4.2).
///
/// ```
/// use clash_sim::component::{Identity, TemplateId};
///
/// let mut who = Identity::new(None);
/// who.apply_template(TemplateId::new(2));
/// assert_eq!(who.rename(Some("Maren".to_owned())), None);
/// assert_eq!(who.name(), Some("Maren"));
/// assert_eq!(who.templates(), &[TemplateId::new(2)]);
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Identity {
    def: Option<DefKey>,
    name: Option<String>,
    description: Option<String>,
    templates: Vec<TemplateId>,
}

impl_state_hash!(Identity {
    def,
    name,
    description,
    templates
});

impl Identity {
    /// The identity of a thing made from `def`, or from no definition,
    /// with no name of its own and no templates applied.
    pub const fn new(def: Option<DefKey>) -> Identity {
        Identity {
            def,
            name: None,
            description: None,
            templates: Vec::new(),
        }
    }

    /// The definition the thing was made from, if it was made from one.
    pub const fn def(&self) -> Option<DefKey> {
        self.def
    }

    /// The thing's own name. `None` means it goes by its definition's.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The thing's own description. `None` means its definition's.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The templates applied to the thing, in the order they were
    /// applied. A template applied twice is listed twice.
    pub fn templates(&self) -> &[TemplateId] {
        &self.templates
    }

    /// Whether `template` has been applied to the thing.
    pub fn has_template(&self, template: TemplateId) -> bool {
        self.templates.contains(&template)
    }

    /// Changes which definition the thing counts as made from: an item
    /// that turns into another (06 §9.4). Returns the one before.
    pub fn set_def(&mut self, def: Option<DefKey>) -> Option<DefKey> {
        core::mem::replace(&mut self.def, def)
    }

    /// Gives the thing a name of its own, or with `None` takes it away.
    /// Returns the name before, for the event the caller raises.
    pub fn rename(&mut self, name: Option<String>) -> Option<String> {
        core::mem::replace(&mut self.name, name)
    }

    /// Gives the thing a description of its own, or with `None` takes it
    /// away. Returns the description before.
    pub fn set_description(&mut self, description: Option<String>) -> Option<String> {
        core::mem::replace(&mut self.description, description)
    }

    /// Records that `template` was applied to the thing.
    pub fn apply_template(&mut self, template: TemplateId) {
        self.templates.push(template);
    }
}
