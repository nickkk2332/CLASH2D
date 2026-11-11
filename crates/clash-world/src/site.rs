//! Sites: named places with identity (02 §8.4). A bank, a headquarters,
//! a lair.
//!
//! A [`Site`] spans one or more tracts of a realm and survives their
//! being unloaded. The world stores sites and maps tracts to them; the
//! simulation attaches to a site what the cold layer needs. A tract
//! belongs to at most one site.

use std::collections::BTreeMap;

use clash_core::{StateHash, StateHasher, Value, VarKey, VarMap, define_id, sorted};
use serde::{Deserialize, Serialize};

use crate::cosmos::{Cosmos, RealmId};
use crate::error::WorldError;
use crate::pos::{CellPos, Location, TractPos};

define_id! {
    /// A site of the cosmos. Never reused.
    pub struct SiteId;
    /// A `site` definition (document 10). The world stores the id and
    /// gives it no meaning: the definition type is built with world
    /// generation.
    pub struct SiteDefId(u16);
}

/// A named place with identity, spanning one or more tracts (02 §8.4).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Site {
    /// The site's id.
    pub id: SiteId,
    /// Its definition.
    pub def: SiteDefId,
    /// Its name, as shown.
    pub name: String,
    /// The realm it is in.
    pub realm: RealmId,
    /// The tracts it spans, in order.
    pub tracts: Vec<TractPos>,
    /// The cell that stands for the place: its door, its heart.
    pub anchor: CellPos,
    /// Its variables.
    pub vars: VarMap,
    /// Its tags, in order.
    pub tags: Vec<String>,
}

impl Site {
    /// Whether the site carries a tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags
            .binary_search_by(|have| have.as_str().cmp(tag))
            .is_ok()
    }
}

clash_core::impl_state_hash!(Site {
    id,
    def,
    name,
    realm,
    tracts,
    anchor,
    vars,
    tags
});

/// What to make a site from.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SiteSpec {
    /// Its definition.
    pub def: SiteDefId,
    /// Its name, as shown.
    pub name: String,
    /// The realm it is in.
    pub realm: RealmId,
    /// The tracts it spans, in any order.
    pub tracts: Vec<TractPos>,
    /// The cell that stands for the place.
    pub anchor: CellPos,
    /// Its variables.
    pub vars: VarMap,
    /// Its tags, in any order.
    pub tags: Vec<String>,
}

/// Every site of the cosmos, and the site of each tract (02 §8.4).
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct SiteTable {
    sites: BTreeMap<SiteId, Site>,
    /// The site each tract belongs to. Derived from the sites.
    by_tract: BTreeMap<(RealmId, TractPos), SiteId>,
    next: u32,
}

impl SiteTable {
    /// A table of no sites.
    pub fn new() -> SiteTable {
        SiteTable::default()
    }

    /// How many sites there are.
    pub fn len(&self) -> usize {
        self.sites.len()
    }

    /// Whether there are no sites.
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// A site by id.
    pub fn get(&self, id: SiteId) -> Option<&Site> {
        self.sites.get(&id)
    }

    /// Every site, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Site> + '_ {
        self.sites.values()
    }

    /// The site a tract of a realm belongs to.
    pub fn at(&self, realm: RealmId, tract: TractPos) -> Option<SiteId> {
        self.by_tract.get(&(realm, tract)).copied()
    }

    fn site_mut(&mut self, id: SiteId) -> Result<&mut Site, WorldError> {
        self.sites.get_mut(&id).ok_or(WorldError::UnknownSite(id))
    }

    /// Refuses if one of these tracts already belongs to a site other
    /// than `own`.
    fn check_free(
        &self,
        realm: RealmId,
        tracts: &[TractPos],
        own: Option<SiteId>,
    ) -> Result<(), WorldError> {
        for tract in tracts {
            if let Some(site) = self.at(realm, *tract)
                && Some(site) != own
            {
                return Err(WorldError::TractHasSite {
                    realm,
                    tract: *tract,
                    site,
                });
            }
        }
        Ok(())
    }

    fn create(&mut self, spec: SiteSpec) -> Result<SiteId, WorldError> {
        let tracts = sorted::from_unsorted(spec.tracts);
        self.check_free(spec.realm, &tracts, None)?;
        let next = self
            .next
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted("site"))?;
        let id = SiteId::new(self.next);
        self.next = next;
        for tract in &tracts {
            self.by_tract.insert((spec.realm, *tract), id);
        }
        self.sites.insert(
            id,
            Site {
                id,
                def: spec.def,
                name: spec.name,
                realm: spec.realm,
                tracts,
                anchor: spec.anchor,
                vars: spec.vars,
                tags: sorted::from_unsorted(spec.tags),
            },
        );
        Ok(id)
    }

    fn remove(&mut self, id: SiteId) -> Option<Site> {
        let site = self.sites.remove(&id)?;
        for tract in &site.tracts {
            self.by_tract.remove(&(site.realm, *tract));
        }
        Some(site)
    }
}

/// Hashes the sites in id order and the next id. The map from tracts to
/// sites is derived and is not hashed.
impl StateHash for SiteTable {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.sites.state_hash(hasher);
        hasher.write_u32(self.next);
    }
}

impl Cosmos {
    /// A site by id.
    pub fn site(&self, id: SiteId) -> Option<&Site> {
        self.sites.get(id)
    }

    /// Every site, in id order.
    pub fn sites(&self) -> impl Iterator<Item = &Site> + '_ {
        self.sites.iter()
    }

    /// The table of sites.
    pub fn site_table(&self) -> &SiteTable {
        &self.sites
    }

    /// The site a tract of a realm belongs to.
    pub fn site_at(&self, realm: RealmId, tract: TractPos) -> Option<SiteId> {
        self.sites.at(realm, tract)
    }

    /// The site a location is in: the site of its tract, where its grid
    /// is a realm's base grid. The location's tract need not be loaded.
    pub fn site_of(&self, loc: Location) -> Result<Option<SiteId>, WorldError> {
        let loc = self.normalize(loc)?;
        Ok(self
            .realm_of(loc.grid)
            .filter(|realm| {
                self.realm(*realm)
                    .is_some_and(|realm| realm.base_grid == loc.grid)
            })
            .and_then(|realm| self.sites.at(realm, loc.cell.tract())))
    }

    /// Makes a site. Refused if its realm is unknown
    /// ([`WorldError::UnknownRealm`]) or one of its tracts already
    /// belongs to another site ([`WorldError::TractHasSite`]).
    pub fn create_site(&mut self, spec: SiteSpec) -> Result<SiteId, WorldError> {
        if self.realm(spec.realm).is_none() {
            return Err(WorldError::UnknownRealm(spec.realm));
        }
        self.sites.create(spec)
    }

    /// Removes a site and returns it. An area that named it keeps the
    /// id, which now names nothing.
    pub fn remove_site(&mut self, id: SiteId) -> Result<Site, WorldError> {
        self.sites.remove(id).ok_or(WorldError::UnknownSite(id))
    }

    /// Adds a tract to a site. Refused if the tract already belongs to
    /// another site. Returns whether that changed anything.
    pub fn claim_site_tract(&mut self, id: SiteId, tract: TractPos) -> Result<bool, WorldError> {
        let realm = self.sites.site_mut(id)?.realm;
        self.sites.check_free(realm, &[tract], Some(id))?;
        self.sites.by_tract.insert((realm, tract), id);
        Ok(sorted::insert(&mut self.sites.site_mut(id)?.tracts, tract))
    }

    /// Takes a tract away from a site. Returns whether that changed
    /// anything.
    pub fn release_site_tract(&mut self, id: SiteId, tract: TractPos) -> Result<bool, WorldError> {
        let site = self.sites.site_mut(id)?;
        let realm = site.realm;
        let removed = sorted::remove(&mut site.tracts, &tract);
        if removed {
            self.sites.by_tract.remove(&(realm, tract));
        }
        Ok(removed)
    }

    /// Sets or clears a variable of a site and returns the value it had.
    pub fn set_site_var(
        &mut self,
        id: SiteId,
        key: VarKey,
        value: Option<Value>,
    ) -> Result<Option<Value>, WorldError> {
        let site = self.sites.site_mut(id)?;
        Ok(match value {
            Some(value) => site.vars.set(key, value),
            None => site.vars.remove(key.as_str()),
        })
    }

    /// Gives a site a tag or takes it away. Returns whether that changed
    /// anything.
    pub fn set_site_tag(&mut self, id: SiteId, tag: &str, on: bool) -> Result<bool, WorldError> {
        let site = self.sites.site_mut(id)?;
        Ok(if on {
            sorted::insert(&mut site.tags, tag.to_owned())
        } else {
            match site.tags.binary_search_by(|have| have.as_str().cmp(tag)) {
                Ok(at) => {
                    site.tags.remove(at);
                    true
                }
                Err(_) => false,
            }
        })
    }

    /// Gives a site another name.
    pub fn set_site_name(&mut self, id: SiteId, name: String) -> Result<(), WorldError> {
        self.sites.site_mut(id)?.name = name;
        Ok(())
    }
}
