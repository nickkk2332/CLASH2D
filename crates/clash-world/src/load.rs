//! Joining the world's definitions to the mod loader (04 §7 to §9).
//!
//! The definition types of [`crate::defs`] are registered with
//! `clash-data` here, and [`WorldDefs::from_defs`] is the one way from
//! loaded definitions to the world's tables:
//!
//! ```no_run
//! use clash_data::{DefTypes, Loader};
//! use clash_world::WorldDefs;
//!
//! let mut types = DefTypes::new();
//! clash_world::register_defs(&mut types);
//! // ... every other crate's `register_defs` ...
//! let defs = Loader::new(&types)
//!     .search_path("mods")
//!     .load_profile("default")
//!     .into_result()
//!     .map_err(|diagnostics| diagnostics.render())?;
//! let world = WorldDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())?;
//! # Ok::<(), String>(())
//! ```
//!
//! # Ids agree
//!
//! The loader numbers the definitions of a type in sorted order of
//! qualified id, after the engine's own (04 §7.2), and
//! [`WorldDefs::build`] does the same. The built-in `engine:none` of
//! materials, floors, blocks and fixtures is registered as the engine's
//! own definition of each, so dense id 0 is the same thing on both sides:
//! `defs.lookup::<BlockDef>("mymod:gate")` and
//! `world.blocks().lookup("mymod:gate")` give the same [`BlockId`].
//!
//! # What is checked where
//!
//! The loader reads each definition into its struct: unknown keys, wrong
//! types and bad units are its diagnostics. References between the
//! world's definitions are still text when it has finished; they are
//! resolved, and ranges checked, when the tables are built. A mistake
//! found then is a [`Diagnostic`] at the value in the file that wrote it,
//! found through the finished tree the loader keeps for every definition.
//! [`check_defs`] is that step for a tool that only wants the mistakes:
//! the linter runs it after a load that had no errors.
//!
//! Until the reference fields become `clash_data::Ref`s, a bare reference
//! is looked up in the namespace of the definition that holds it and then
//! in every namespace in sorted order, not through the import list of the
//! mod that wrote it (decision D-283).

use std::sync::Arc;

use clash_core::{DenseId, Real};
use clash_data::{Def, DefLabel, DefTypes, Defs, Diagnostic, Diagnostics, Span, code};

use crate::defs::{
    AreaKindDef, AreaKindId, AtmosphereDef, AtmosphereId, BlockDef, BlockId, DamageTypeDef,
    DamageTypeId, DefError, DefRef, Density, ENGINE_NAMESPACE, EnergyDensity, FixtureDef,
    FixtureId, FloorDef, FloorId, Length, MaterialDef, MaterialId, NONE_ID, OrderedMap, PlaneDef,
    PlaneId, RawWorldDefs, RealmDef, RealmDefId, Table, WorldDefs,
};
use crate::field::{FieldDef, FieldReactionDef};
use crate::light::{SkyDef, SkyId};

/// The name of the built-in "nothing here" within the engine's namespace.
const NONE_NAME: &str = "none";

/// The built-in plane, id 0, without its namespace: `engine:default`.
const DEFAULT_PLANE_NAME: &str = "default";

/// The key of an element's or a material's resistances, whose entries are
/// named by damage type.
const RESIST_KEY: &str = "resist";

impl Def for DamageTypeDef {
    type Id = DamageTypeId;
    const TYPE: &'static str = "damage_type";
    const DOC: &'static str =
        "A kind of harm, with the resistance of anything that states none of its own (02 §12.2).";
}

impl Def for MaterialDef {
    type Id = MaterialId;
    const TYPE: &'static str = "material";
    const DOC: &'static str =
        "What elements are made of: density, toughness and resistances (02 §5.4).";

    fn builtins() -> Vec<(&'static str, MaterialDef)> {
        vec![(
            NONE_NAME,
            MaterialDef {
                density: Density(Real::ZERO),
                toughness: EnergyDensity(Real::ZERO),
                resist: Default::default(),
                span: None,
                bearing: None,
                flammable: false,
                rubble: None,
                color: None,
                tags: Vec::new(),
                leak: None,
                signal: None,
                heal: None,
                friction: None,
            },
        )]
    }
}

impl Def for AtmosphereDef {
    type Id = AtmosphereId;
    const TYPE: &'static str = "atmosphere";
    const DOC: &'static str =
        "What fills open cells of a stratum: density, pressure and the speed of sound (02 §4.1).";
}

impl Def for FloorDef {
    type Id = FloorId;
    const TYPE: &'static str = "floor";
    const DOC: &'static str = "The surface at the base of a cell (02 §5.4).";

    fn builtins() -> Vec<(&'static str, FloorDef)> {
        vec![(
            NONE_NAME,
            FloorDef {
                material: DefRef(NONE_ID.to_owned()),
                thickness: Length(Real::ZERO),
                flags: Vec::new(),
                tags: Vec::new(),
                resist: Default::default(),
                states: OrderedMap::default(),
                on_destroy: None,
                contact: None,
                interactions: Vec::new(),
                device: None,
                glyph: None,
                light: None,
            },
        )]
    }
}

impl Def for BlockDef {
    type Id = BlockId;
    const TYPE: &'static str = "block";
    const DOC: &'static str = "What occupies a cell's volume (02 §5.4).";

    fn builtins() -> Vec<(&'static str, BlockDef)> {
        vec![(
            NONE_NAME,
            BlockDef {
                material: DefRef(NONE_ID.to_owned()),
                height: Length(Real::ZERO),
                fill: Real::ZERO,
                coverage: None,
                flags: Vec::new(),
                tags: Vec::new(),
                resist: Default::default(),
                states: OrderedMap::default(),
                on_destroy: None,
                contact: None,
                interactions: Vec::new(),
                device: None,
                glyph: None,
                light: None,
            },
        )]
    }
}

impl Def for FixtureDef {
    type Id = FixtureId;
    const TYPE: &'static str = "fixture";
    const DOC: &'static str = "A static prop standing in a cell (02 §5.4).";

    fn builtins() -> Vec<(&'static str, FixtureDef)> {
        vec![(
            NONE_NAME,
            FixtureDef {
                material: DefRef(NONE_ID.to_owned()),
                height: Length(Real::ZERO),
                fill: Real::ZERO,
                coverage: None,
                flags: Vec::new(),
                tags: Vec::new(),
                resist: Default::default(),
                states: OrderedMap::default(),
                on_destroy: None,
                heap: None,
                contact: None,
                interactions: Vec::new(),
                device: None,
                glyph: None,
                light: None,
            },
        )]
    }
}

impl Def for PlaneDef {
    type Id = PlaneId;
    const TYPE: &'static str = "plane";
    const DOC: &'static str =
        "An interaction layer: occupants that share no plane do not meet (02 §8.2).";

    fn builtins() -> Vec<(&'static str, PlaneDef)> {
        vec![(DEFAULT_PLANE_NAME, PlaneDef::default())]
    }
}

impl Def for AreaKindDef {
    type Id = AreaKindId;
    const TYPE: &'static str = "area_kind";
    const DOC: &'static str =
        "What every area of a kind starts with: tags, variables, clock, priority (02 §8.3).";
}

impl Def for SkyDef {
    type Id = SkyId;
    const TYPE: &'static str = "sky";
    const DOC: &'static str = "The light a stratum's sky gives by time of day, how it spreads under cover, and a backdrop (15 §3.3).";
}

impl Def for RealmDef {
    type Id = RealmDefId;
    const TYPE: &'static str = "realm";
    const DOC: &'static str =
        "A world: its extent, wrap, storeys, strata, fill, clock rate and edges (02 §4.1).";
    const DEFAULTS: &'static str = r#"{ "time_rate": 1.0 }"#;
}

/// Registers the world's definition types with the loader: `damage_type`,
/// `material`, `atmosphere`, `floor`, `block`, `fixture`, `realm`, `plane`,
/// `area_kind`, `sky`, `field` and `field_reaction`.
///
/// The application and the tools call this, with every other crate's
/// function of the same name, before they load (04 §9).
pub fn register_defs(types: &mut DefTypes) {
    types.register::<DamageTypeDef>();
    types.register::<MaterialDef>();
    types.register::<AtmosphereDef>();
    types.register::<FloorDef>();
    types.register::<BlockDef>();
    types.register::<FixtureDef>();
    types.register::<RealmDef>();
    types.register::<PlaneDef>();
    types.register::<AreaKindDef>();
    types.register::<SkyDef>();
    types.register::<FieldDef>();
    types.register::<FieldReactionDef>();
}

/// Whether a qualified id is one of the engine's own.
fn is_engine_id(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(namespace, _)| namespace == ENGINE_NAMESPACE)
}

/// The definitions of one type as mods wrote them, with their qualified
/// ids. The engine's own are left out: [`WorldDefs::build`] supplies them.
fn written<T: Def + Clone>(defs: &Defs) -> Vec<(String, T)> {
    let Some(registry) = defs.try_registry::<T>() else {
        return Vec::new();
    };
    registry
        .iter()
        .map(|(id, def)| (registry.name(id), def))
        .filter(|(name, _)| !is_engine_id(name))
        .map(|(name, def)| (name.to_owned(), def.clone()))
        .collect()
}

impl RawWorldDefs {
    /// The world's definitions out of a finished load, as
    /// [`WorldDefs::build`] takes them. A type that was not registered
    /// with the loader has none.
    pub fn from_defs(defs: &Defs) -> RawWorldDefs {
        RawWorldDefs {
            damage_types: written(defs),
            materials: written(defs),
            atmospheres: written(defs),
            floors: written(defs),
            blocks: written(defs),
            fixtures: written(defs),
            realms: written(defs),
            planes: written(defs),
            area_kinds: written(defs),
            skies: written(defs),
            fields: written(defs),
            field_reactions: written(defs),
        }
    }
}

/// A [`DefError`]'s path in the form diagnostics use (04 §8.2): an entry
/// of a list is `[index]`.
fn json_path(path: &str) -> String {
    let mut out = String::new();
    for segment in path.split('.').filter(|segment| !segment.is_empty()) {
        if segment.bytes().all(|byte| byte.is_ascii_digit()) {
            out.push_str(&format!("[{segment}]"));
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(segment);
        }
    }
    out
}

/// Where the value at a path of a definition was written, as far along
/// the path as the finished tree goes: a value the definition left out
/// is reported at what would have held it.
fn locate(defs: &Defs, error: &DefError, unknown_reference: bool) -> Option<Span> {
    let tree = defs.tree(error.kind, &error.id)?;
    let mut node = &tree.tree;
    // The whole definition: point at its opening brace, not at every line.
    let mut span = Span::new(node.span.file, node.span.start, node.span.start + 1);
    let segments: Vec<&str> = error
        .path
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect();
    for (position, segment) in segments.iter().enumerate() {
        if let Some(items) = node.as_array() {
            let Some(item) = segment.parse::<usize>().ok().and_then(|at| items.get(at)) else {
                break;
            };
            node = item;
            span = node.span;
        } else if let Some(member) = node.member(segment) {
            node = &member.value;
            // An unknown damage type in `resist` is the key, not what the
            // key holds.
            let names_key = unknown_reference
                && position + 1 == segments.len()
                && position > 0
                && segments[position - 1] == RESIST_KEY;
            span = if names_key {
                member.key_span
            } else {
                node.span
            };
        } else {
            break;
        }
    }
    Some(span)
}

/// A mistake found while building the tables, as a diagnostic at the
/// value that is wrong.
fn diagnostic_of(defs: &Defs, error: &DefError) -> Diagnostic {
    let unknown_reference = error.message.starts_with("there is no ");
    let code = if unknown_reference {
        code::UNKNOWN_REFERENCE
    } else {
        code::VALIDATION
    };
    let path = json_path(&error.path);
    // "must be from 0 to 3" says of what once the path is not beside it.
    let message = match error.path.rsplit('.').next() {
        Some(field) if error.message.starts_with("must ") && !field.is_empty() => {
            format!("`{field}` {}", error.message)
        }
        _ => error.message.clone(),
    };
    let label = DefLabel::new(error.kind, error.id.clone());
    let mut diagnostic = Diagnostic::new(code, message)
        .in_def(Some(&label))
        .with_path(path);
    if let Some(span) = locate(defs, error, unknown_reference) {
        diagnostic = diagnostic.at(defs.sources(), span);
    }
    diagnostic
}

/// Whether the loader and the built table gave every definition of one
/// type the same dense id.
fn ids_agree<T: Def, V>(defs: &Defs, table: &Table<T::Id, V>) -> bool {
    let Some(registry) = defs.try_registry::<T>() else {
        return true;
    };
    registry.len() == table.len()
        && registry.iter().all(|(id, _)| {
            table.lookup(registry.name(id)).map(|found| found.index()) == Some(id.index())
        })
}

impl WorldDefs {
    /// Builds the world's tables from loaded definitions.
    ///
    /// This is the one way from a [`Defs`] to a `WorldDefs`. The tables
    /// use the loader's dense ids: an id looked up in either means the
    /// same definition in the other.
    ///
    /// Every mistake found is returned as a located diagnostic: a
    /// reference to something that does not exist, a value out of its
    /// range, a state that is not declared. This never panics, whatever
    /// the definitions hold.
    pub fn from_defs(defs: &Defs) -> Result<Arc<WorldDefs>, Diagnostics> {
        let mut diagnostics = Diagnostics::new();
        let mut world = match WorldDefs::build(RawWorldDefs::from_defs(defs)) {
            Ok(world) => world,
            Err(errors) => {
                diagnostics.extend(errors.iter().map(|error| diagnostic_of(defs, error)));
                diagnostics.sort();
                return Err(diagnostics);
            }
        };
        let agree = ids_agree::<DamageTypeDef, _>(defs, world.damage_types())
            && ids_agree::<MaterialDef, _>(defs, world.materials())
            && ids_agree::<AtmosphereDef, _>(defs, world.atmospheres())
            && ids_agree::<FloorDef, _>(defs, world.floors())
            && ids_agree::<BlockDef, _>(defs, world.blocks())
            && ids_agree::<FixtureDef, _>(defs, world.fixtures())
            && ids_agree::<RealmDef, _>(defs, world.realms())
            && ids_agree::<PlaneDef, _>(defs, world.planes())
            && ids_agree::<AreaKindDef, _>(defs, world.area_kinds())
            && ids_agree::<SkyDef, _>(defs, world.skies())
            && ids_agree::<FieldDef, _>(defs, world.fields())
            && ids_agree::<FieldReactionDef, _>(defs, world.field_reactions());
        if !agree {
            // Not a mistake in a mod: the two numberings are meant to be
            // the same by construction. Refuse to go on with ids that
            // would mean different things on each side.
            diagnostics.push(Diagnostic::new(
                code::VALIDATION,
                "the world's tables and the loader disagree about the dense ids of the \
                 world's definitions; this is a fault in the engine, not in a mod",
            ));
            return Err(diagnostics);
        }
        world.set_fingerprint(defs.fingerprint());
        Ok(Arc::new(world))
    }
}

/// Everything wrong with the world's definitions that only shows when the
/// tables are built (see the module documentation). Empty when they
/// build.
pub fn check_defs(defs: &Defs) -> Diagnostics {
    match WorldDefs::from_defs(defs) {
        Ok(_) => Diagnostics::new(),
        Err(diagnostics) => diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_entries_are_written_as_indices() {
        assert_eq!(json_path("strata.1.atmosphere"), "strata[1].atmosphere");
        assert_eq!(json_path("size_regions.0"), "size_regions[0]");
        assert_eq!(
            json_path("resist.blast.threshold"),
            "resist.blast.threshold"
        );
        assert_eq!(json_path(""), "");
    }

    #[test]
    fn the_engines_own_ids_are_told_apart() {
        assert!(is_engine_id(NONE_ID));
        assert!(!is_engine_id("engineer:none"));
        assert!(!is_engine_id("none"));
    }
}
