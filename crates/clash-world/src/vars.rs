//! Variables at the scopes the world keeps (05 §6): a realm's, a
//! tract's and a region's. A cell's are changed through
//! [`WorldEdit`](crate::WorldEdit), an area's and a site's through
//! `set_area_var` and `set_site_var`, and the global ones through
//! [`Cosmos::set_global_var`].
//!
//! These change records, not cells, so they emit nothing: whoever writes
//! a variable on behalf of a rule raises `var_changed` for the ones that
//! are watched.

use clash_core::{Value, VarKey};

use crate::cosmos::{Cosmos, RealmId};
use crate::error::WorldError;
use crate::pos::{RegionPos, TractPos};

/// Sets or clears one variable of a map and returns the value it had.
fn write(vars: &mut clash_core::VarMap, key: VarKey, value: Option<Value>) -> Option<Value> {
    match value {
        Some(value) => vars.set(key, value),
        None => vars.remove(key.as_str()),
    }
}

impl Cosmos {
    /// Sets or clears a variable of a realm and returns the value it
    /// had. Refused for an unknown realm.
    pub fn set_realm_var(
        &mut self,
        realm: RealmId,
        key: VarKey,
        value: Option<Value>,
    ) -> Result<Option<Value>, WorldError> {
        let record = self
            .realm_mut(realm)
            .ok_or(WorldError::UnknownRealm(realm))?;
        Ok(write(&mut record.vars, key, value))
    }

    /// A variable of a tract, if the tract has a record and the variable
    /// has been set. The tract need not be loaded.
    pub fn tract_var(&self, realm: RealmId, tract: TractPos, key: &str) -> Option<&Value> {
        self.tract_record(realm, tract)?
            .extra
            .as_ref()?
            .vars
            .get(key)
    }

    /// Sets or clears a variable of a tract and returns the value it
    /// had. The tract need not be loaded.
    ///
    /// Refused for an unknown realm, a tract outside it, and a tract
    /// whose region has not been surveyed
    /// ([`WorldError::NotSurveyed`]): it has no record to keep the
    /// variable in.
    pub fn set_tract_var(
        &mut self,
        realm: RealmId,
        tract: TractPos,
        key: VarKey,
        value: Option<Value>,
    ) -> Result<Option<Value>, WorldError> {
        let tract = self.normalize_tract(realm, tract)?;
        let record = self
            .realm_mut(realm)
            .ok_or(WorldError::UnknownRealm(realm))?
            .regions
            .get_mut(&tract.region())
            .ok_or(WorldError::NotSurveyed { realm, tract })?
            .tract_mut(tract);
        if value.is_none() && record.extra.is_none() {
            // Nothing to clear, and no reason to make room for it.
            return Ok(None);
        }
        Ok(write(&mut record.extra_mut().vars, key, value))
    }

    /// Sets or clears a variable of a surveyed region and returns the
    /// value it had. Refused for an unknown realm and for a region that
    /// has not been surveyed.
    pub fn set_region_var(
        &mut self,
        realm: RealmId,
        region: RegionPos,
        key: VarKey,
        value: Option<Value>,
    ) -> Result<Option<Value>, WorldError> {
        let record = self
            .realm_mut(realm)
            .ok_or(WorldError::UnknownRealm(realm))?
            .regions
            .get_mut(&region)
            .ok_or(WorldError::NotSurveyed {
                realm,
                tract: region.origin(),
            })?;
        Ok(write(&mut record.vars, key, value))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use clash_core::{Hash128, Value, VarKey};
    use clash_time::Timeline;

    use crate::defs::RawWorldDefs;
    use crate::{Cosmos, RealmDefId, RealmId, TractPos, WorldDefs, WorldError};

    #[test]
    fn variables_of_the_world_and_of_a_realm_are_set_cleared_and_hashed() {
        let defs = Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap());
        let mut cosmos = Cosmos::new(defs);
        let empty = Hash128::of(&cosmos);
        let key = || VarKey::new("core:season");

        assert_eq!(cosmos.set_global_var(key(), Some(Value::from(2))), None);
        assert_eq!(cosmos.vars().get("core:season"), Some(&Value::Int(2)));
        assert_ne!(Hash128::of(&cosmos), empty);
        assert_eq!(cosmos.set_global_var(key(), None), Some(Value::Int(2)));
        assert_eq!(Hash128::of(&cosmos), empty);

        // There is no realm to hold one, and the refusal says so.
        let nowhere = RealmId::new(4);
        assert_eq!(
            cosmos.set_realm_var(nowhere, key(), Some(Value::from(1))),
            Err(WorldError::UnknownRealm(nowhere))
        );
        assert_eq!(cosmos.tract_var(nowhere, TractPos::new(0, 0), "x"), None);
        assert!(
            cosmos
                .set_tract_var(nowhere, TractPos::new(0, 0), key(), None)
                .is_err()
        );
        let mut timeline = Timeline::new();
        assert!(
            cosmos
                .create_realm(&mut timeline, RealmDefId::new(0), 1)
                .is_err(),
            "no realm definitions were loaded"
        );
    }
}
