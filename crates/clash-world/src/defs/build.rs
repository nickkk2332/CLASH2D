//! Building [`WorldDefs`] from raw definitions: sorting, numbering,
//! resolving references, checking ranges and deriving the per-state
//! tables.

use std::collections::BTreeMap;

use clash_core::{DenseId, Real, Value, VarKey, VarMap};

use super::state::{StatePart, is_name};
use super::{
    AreaKind, AreaKindId, Atmosphere, BlockId, BlockState, CellFlags, Contact, ContactSpec,
    DEFAULT_PLANE_ID, DamageType, DamageTypeId, DefError, DefRef, ENGINE_NAMESPACE, EdgeKind,
    EdgeRule, EdgeSpec, Element, ElementFlags, ElementState, EngineFlag, FixtureId, GlyphSpec,
    Layer, Material, MaterialId, NONE_ID, Plane, PlaneId, RawWorldDefs, RealmKind, Resist,
    ResistSpec, SOLID_TOP_MIN_FILL, StateInfo, StateRef, StateRefError, Stratum, Table,
    WALK_STEP_M, WorldDefs, WrapAxis,
};
use crate::dims::{CELL_M, REGION_CELLS, STOREY_M};
use crate::light::{LightSource, LightSpec, Sky, SkyId};
use crate::topology::{Edge, Extent, Topology};

/// The most states one element may declare: what sixteen bits can index.
const MAX_STATES: usize = 1 << u16::BITS;

/// The most planes there can be, counting the built-in one: one for each
/// bit of a plane mask (02 §8.2).
const MAX_PLANES: usize = u32::BITS as usize;

/// The mistakes found so far.
#[derive(Default)]
struct Errors {
    list: Vec<DefError>,
}

impl Errors {
    fn push(&mut self, kind: &'static str, id: &str, path: &str, message: impl Into<String>) {
        self.list.push(DefError {
            kind,
            id: id.to_owned(),
            path: path.to_owned(),
            message: message.into(),
        });
    }
}

/// Where a mistake is: the definition and a path inside it.
#[derive(Clone, Copy)]
struct At<'a> {
    kind: &'static str,
    id: &'a str,
}

impl At<'_> {
    fn error(self, errors: &mut Errors, path: &str, message: impl Into<String>) {
        errors.push(self.kind, self.id, path, message);
    }
}

/// The namespace of a qualified id.
fn namespace_of(id: &str) -> &str {
    id.split_once(':').map_or("", |(namespace, _)| namespace)
}

/// Sorts definitions by qualified id and drops, with an error, any whose
/// id is malformed, reserved or given twice.
fn sorted<D>(
    kind: &'static str,
    mut defs: Vec<(String, D)>,
    errors: &mut Errors,
) -> Vec<(String, D)> {
    defs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut kept: Vec<(String, D)> = Vec::with_capacity(defs.len());
    for (id, def) in defs {
        let qualified = id
            .split_once(':')
            .is_some_and(|(namespace, name)| is_name(namespace) && is_name(name));
        if !qualified {
            errors.push(
                kind,
                &id,
                "id",
                "an id is `namespace:name`; each part starts with a lowercase letter and \
                 holds lowercase letters, digits and underscores",
            );
        } else if namespace_of(&id) == ENGINE_NAMESPACE {
            errors.push(kind, &id, "id", "the namespace `engine` is reserved");
        } else if kept.last().is_some_and(|(previous, _)| *previous == id) {
            errors.push(kind, &id, "id", "this id is defined more than once");
        } else {
            kept.push((id, def));
        }
    }
    kept
}

/// The names of a table, numbered, with no entries yet. With `with_none`,
/// id 0 is the built-in `engine:none`.
fn index<I: DenseId, T, D>(
    kind: &'static str,
    defs: &[(String, D)],
    with_none: bool,
    errors: &mut Errors,
) -> Table<I, T> {
    let mut names: Vec<String> = Vec::with_capacity(defs.len() + 1);
    if with_none {
        names.push(NONE_ID.to_owned());
    }
    names.extend(defs.iter().map(|(id, _)| id.clone()));
    let mut by_name = BTreeMap::new();
    for (position, name) in names.iter().enumerate() {
        match I::from_index(position) {
            Some(id) => {
                by_name.insert(name.clone(), id);
            }
            None => {
                errors.push(
                    kind,
                    name,
                    "id",
                    "there are too many definitions of this type",
                );
                break;
            }
        }
    }
    Table {
        names,
        by_name,
        entries: Vec::new(),
    }
}

/// Resolves a reference, or records that it names nothing.
fn resolve<I: DenseId, T>(
    table: &Table<I, T>,
    what: &str,
    reference: &str,
    at: At<'_>,
    path: &str,
    errors: &mut Errors,
) -> Option<I> {
    let found = table.resolve(reference, Some(namespace_of(at.id)));
    if found.is_none() {
        at.error(errors, path, format!("there is no {what} `{reference}`"));
    }
    found
}

/// Tags sorted and without repeats.
fn tags(given: &[String]) -> Vec<String> {
    let mut tags = given.to_vec();
    tags.sort();
    tags.dedup();
    tags
}

fn resist_of(spec: &ResistSpec, at: At<'_>, path: &str, errors: &mut Errors) -> Resist {
    let threshold = spec.threshold.get();
    if threshold < Real::ZERO || !threshold.is_finite() {
        at.error(errors, &format!("{path}.threshold"), "must be zero or more");
    }
    if spec.factor < Real::ZERO || !spec.factor.is_finite() {
        at.error(errors, &format!("{path}.factor"), "must be zero or more");
    }
    Resist {
        threshold,
        factor: spec.factor,
    }
}

/// Checks that a value lies in a range, and returns it clamped into it.
fn in_range(
    value: Real,
    low: Real,
    high: Real,
    at: At<'_>,
    path: &str,
    errors: &mut Errors,
) -> Real {
    if value < low || value > high {
        at.error(
            errors,
            path,
            format!("must be from {low} to {high}, not {value}"),
        );
    }
    value.clamp(low, high)
}

fn not_negative(value: Real, at: At<'_>, path: &str, errors: &mut Errors) -> Real {
    if value < Real::ZERO || !value.is_finite() {
        at.error(errors, path, format!("must be zero or more, not {value}"));
        return Real::ZERO;
    }
    value
}

/// What one declared state changes.
struct StateChange<'a> {
    name: &'a str,
    add_flags: &'a [EngineFlag],
    remove_flags: &'a [EngineFlag],
    height: Option<Real>,
    fill: Option<Real>,
    coverage: Option<Real>,
    thickness: Option<Real>,
    glyph: Option<&'a GlyphSpec>,
    light: Option<&'a LightSpec>,
}

/// A floor, block or fixture definition, whichever it was written as.
struct ElementInput<'a> {
    layer: Layer,
    material: &'a DefRef,
    height: Real,
    fill: Real,
    coverage: Option<Real>,
    thickness: Real,
    flags: &'a [EngineFlag],
    tags: &'a [String],
    resist: &'a BTreeMap<String, ResistSpec>,
    states: Vec<StateChange<'a>>,
    heap: Option<&'a DefRef>,
    contact: Option<&'a ContactSpec>,
    interactions: &'a [String],
    device: Option<&'a DefRef>,
    glyph: Option<&'a GlyphSpec>,
    light: Option<&'a LightSpec>,
}

/// The light a definition or one of its states gives off, or nothing
/// and a reported mistake.
fn light_of(
    spec: Option<&LightSpec>,
    at: At<'_>,
    path: &str,
    errors: &mut Errors,
) -> Option<LightSource> {
    match LightSource::from_spec(spec?) {
        Ok(light) => light,
        Err(message) => {
            at.error(errors, path, message);
            None
        }
    }
}

/// The geometry of one state before it is checked.
#[derive(Clone, Copy)]
struct Geometry {
    height: Real,
    fill: Real,
    coverage: Option<Real>,
    thickness: Real,
}

fn state_info(
    layer: Layer,
    flags: ElementFlags,
    geometry: Geometry,
    at: At<'_>,
    path: &str,
    errors: &mut Errors,
) -> StateInfo {
    let join = |field: &str| {
        if path.is_empty() {
            field.to_owned()
        } else {
            format!("{path}.{field}")
        }
    };
    let fragile = CellFlags::when(CellFlags::FRAGILE, flags.has(EngineFlag::Fragile));
    if layer == Layer::Floor {
        let thickness = in_range(
            geometry.thickness,
            Real::ZERO,
            STOREY_M,
            at,
            &join("thickness"),
            errors,
        );
        return StateInfo {
            flags,
            cell: CellFlags::HAS_FLOOR | fragile,
            height: Real::ZERO,
            fill: Real::ZERO,
            coverage: Real::ZERO,
            thickness,
            volume: thickness * CELL_M * CELL_M,
        };
    }
    let height = in_range(
        geometry.height,
        Real::ZERO,
        STOREY_M,
        at,
        &join("height"),
        errors,
    );
    let fill = in_range(
        geometry.fill,
        Real::ZERO,
        Real::ONE,
        at,
        &join("fill"),
        errors,
    );
    let coverage = match geometry.coverage {
        Some(coverage) => in_range(
            coverage,
            Real::ZERO,
            Real::ONE,
            at,
            &join("coverage"),
            errors,
        ),
        None => height / STOREY_M,
    };
    let liquid = flags.has(EngineFlag::Liquid);
    let in_the_way = height > WALK_STEP_M
        && !liquid
        && !flags.has(EngineFlag::Passable)
        && !flags.has(EngineFlag::Ramp)
        && !flags.has(EngineFlag::Stairs);
    let solid_top =
        layer == Layer::Block && !liquid && fill >= SOLID_TOP_MIN_FILL && height >= STOREY_M;
    StateInfo {
        flags,
        cell: CellFlags::when(CellFlags::OPAQUE, flags.has(EngineFlag::Opaque))
            | CellFlags::when(CellFlags::WALK_BLOCKED, in_the_way)
            | CellFlags::when(CellFlags::SOLID_TOP, solid_top)
            | fragile,
        height,
        fill,
        coverage,
        thickness: Real::ZERO,
        volume: fill * CELL_M * CELL_M * STOREY_M,
    }
}

/// The tables an element's references are resolved against.
struct Refs<'a> {
    damage_types: &'a Table<DamageTypeId, DamageType>,
    materials: &'a Table<MaterialId, Material>,
    blocks: &'a Table<BlockId, Element>,
}

fn build_element(
    input: &ElementInput<'_>,
    at: At<'_>,
    refs: &Refs<'_>,
    errors: &mut Errors,
) -> Element {
    let material = resolve(
        refs.materials,
        "material",
        input.material.as_str(),
        at,
        "material",
        errors,
    )
    .unwrap_or(MaterialId::new(0));
    let base_flags = ElementFlags::of(input.flags);
    let base = Geometry {
        height: input.height,
        fill: input.fill,
        coverage: input.coverage,
        thickness: input.thickness,
    };
    let base_glyph = input.glyph.cloned().unwrap_or_default();
    let base_light = light_of(input.light, at, "light", errors);

    let mut state_names = Vec::new();
    let mut states = Vec::new();
    let mut glyphs = Vec::new();
    let mut lights = Vec::new();
    if input.states.is_empty() {
        states.push(state_info(input.layer, base_flags, base, at, "", errors));
        glyphs.push(base_glyph);
        lights.push(base_light);
    } else {
        if input.states.len() > MAX_STATES {
            at.error(errors, "states", "an element has at most 65,536 states");
        }
        for change in input.states.iter().take(MAX_STATES) {
            let path = format!("states.{}", change.name);
            if !is_name(change.name) {
                at.error(
                    errors,
                    &path,
                    "a state's name starts with a lowercase letter and holds lowercase \
                     letters, digits and underscores",
                );
            }
            let mut flags = base_flags;
            for flag in change.add_flags {
                flags = flags.with(*flag);
            }
            for flag in change.remove_flags {
                flags = flags.without(*flag);
            }
            let geometry = Geometry {
                height: change.height.unwrap_or(base.height),
                fill: change.fill.unwrap_or(base.fill),
                // A state that changes the height without giving a coverage
                // takes the default for its new height, unless the
                // definition fixed a coverage.
                coverage: change.coverage.or(base.coverage),
                thickness: change.thickness.unwrap_or(base.thickness),
            };
            states.push(state_info(input.layer, flags, geometry, at, &path, errors));
            lights.push(match change.light {
                Some(_) => light_of(change.light, at, &format!("{path}.light"), errors),
                None => base_light,
            });
            glyphs.push(match change.glyph {
                Some(over) => base_glyph.overlaid(over),
                None => base_glyph.clone(),
            });
            state_names.push(change.name.to_owned());
        }
    }

    let mut resist = vec![None; refs.damage_types.len()];
    for (kind, spec) in input.resist {
        let path = format!("resist.{kind}");
        if let Some(kind) = resolve(refs.damage_types, "damage type", kind, at, &path, errors) {
            resist[kind.index()] = Some(resist_of(spec, at, &path, errors));
        }
    }

    let heap = input
        .heap
        .and_then(|heap| resolve(refs.blocks, "block", heap.as_str(), at, "heap", errors));

    let contact = input.contact.and_then(|contact| {
        let surface = &contact.surface;
        let kind = resolve(
            refs.damage_types,
            "damage type",
            surface.kind.as_str(),
            at,
            "contact.surface.kind",
            errors,
        )?;
        if surface.area <= Real::ZERO || !surface.area.is_finite() {
            at.error(errors, "contact.surface.area", "must be more than zero");
        }
        Some(Contact {
            kind,
            area: surface.area,
            give: not_negative(surface.give.get(), at, "contact.surface.give", errors),
            span: surface
                .span
                .map(|span| not_negative(span.get(), at, "contact.surface.span", errors)),
            energy: not_negative(contact.energy.get(), at, "contact.energy", errors),
        })
    });

    Element {
        layer: input.layer,
        material,
        state_names,
        states,
        glyphs,
        resist,
        tags: tags(input.tags),
        on_destroy: None,
        heap,
        contact,
        interactions: input.interactions.to_vec(),
        device: input.device.map(|device| device.0.clone()),
        lights,
    }
}

/// Resolves a state reference against a layer's definitions.
pub(super) fn resolve_state<I: DenseId>(
    table: &Table<I, Element>,
    materials: &Table<MaterialId, Material>,
    layer: Layer,
    text: &str,
    namespace: Option<&str>,
) -> Result<ElementState, StateRefError> {
    let reference = StateRef::parse(text)?;
    let unknown_def = || StateRefError::UnknownDef {
        layer: layer.name(),
        id: reference.def.to_owned(),
    };
    let def = table
        .resolve(reference.def, namespace)
        .ok_or_else(unknown_def)?;
    let element = table.get(def).ok_or_else(unknown_def)?;
    let material = match reference.material {
        Some(material) => materials
            .resolve(material, namespace)
            .ok_or_else(|| StateRefError::UnknownMaterial(material.to_owned()))?,
        None => element.material,
    };
    let bits = match reference.state {
        None => 0,
        Some(StatePart::Bits(bits)) => bits,
        Some(StatePart::Named(name)) => {
            element
                .state_index(name)
                .ok_or_else(|| StateRefError::UnknownState {
                    layer: layer.name(),
                    def: table.name(def).to_owned(),
                    state: name.to_owned(),
                })?
        }
    };
    let def = u16::try_from(def.index()).map_err(|_| unknown_def())?;
    Ok(ElementState {
        def,
        material,
        bits,
    }
    .normalised())
}

/// The built-in "nothing here" of an element layer.
fn none_element(layer: Layer, damage_types: usize) -> Element {
    Element {
        layer,
        material: MaterialId::new(0),
        state_names: Vec::new(),
        states: vec![StateInfo::NONE],
        glyphs: vec![GlyphSpec::default()],
        resist: vec![None; damage_types],
        tags: Vec::new(),
        on_destroy: None,
        heap: None,
        contact: None,
        interactions: Vec::new(),
        device: None,
        lights: vec![None],
    }
}

/// Resolves the `on_destroy` of every element of one layer. Done after the
/// layer is built, because a state reference needs the definitions it
/// names.
fn link_on_destroy<I: DenseId>(
    table: &mut Table<I, Element>,
    materials: &Table<MaterialId, Material>,
    layer: Layer,
    written: &[Option<&str>],
    errors: &mut Errors,
) {
    let resolved: Vec<Option<ElementState>> = written
        .iter()
        .enumerate()
        .map(|(position, text)| {
            let text = (*text)?;
            // Position 0 is the built-in none, which declares nothing.
            let id = table.names.get(position + 1)?;
            match resolve_state(table, materials, layer, text, Some(namespace_of(id))) {
                Ok(state) => Some(state),
                Err(error) => {
                    errors.push(layer.name(), id, "on_destroy", error.to_string());
                    None
                }
            }
        })
        .collect();
    for (element, state) in table.entries.iter_mut().skip(1).zip(resolved) {
        element.on_destroy = state;
    }
}

pub(super) fn build(raw: RawWorldDefs) -> Result<WorldDefs, Vec<DefError>> {
    let mut errors = Errors::default();
    let errors = &mut errors;

    let damage_defs = sorted("damage_type", raw.damage_types, errors);
    let material_defs = sorted("material", raw.materials, errors);
    let atmosphere_defs = sorted("atmosphere", raw.atmospheres, errors);
    let floor_defs = sorted("floor", raw.floors, errors);
    let block_defs = sorted("block", raw.blocks, errors);
    let fixture_defs = sorted("fixture", raw.fixtures, errors);
    let realm_defs = sorted("realm", raw.realms, errors);
    let plane_defs = sorted("plane", raw.planes, errors);
    let area_kind_defs = sorted("area_kind", raw.area_kinds, errors);
    let sky_defs = sorted("sky", raw.skies, errors);

    // Every id of every type is numbered before anything is resolved, so
    // that a reference may point forwards (04 §7.2).
    let mut damage_types: Table<DamageTypeId, DamageType> =
        index("damage_type", &damage_defs, false, errors);
    let mut materials: Table<MaterialId, Material> =
        index("material", &material_defs, true, errors);
    let mut atmospheres = index("atmosphere", &atmosphere_defs, false, errors);
    let mut floors = index("floor", &floor_defs, true, errors);
    let mut blocks: Table<BlockId, Element> = index("block", &block_defs, true, errors);
    let mut fixtures: Table<FixtureId, Element> = index("fixture", &fixture_defs, true, errors);
    let mut realms = index("realm", &realm_defs, false, errors);
    let mut skies: Table<SkyId, Sky> = index("sky", &sky_defs, false, errors);

    // Skies.
    for (id, def) in &sky_defs {
        let at = At { kind: "sky", id };
        let sky = Sky::from_def(def, |path, message| at.error(errors, path, message));
        skies.entries.push(sky);
    }
    let sky_reach = skies.entries.iter().map(|sky| sky.reach).max().unwrap_or(0);

    // Damage types.
    let mut kinetic: Option<DamageTypeId> = None;
    for (position, (id, def)) in damage_defs.iter().enumerate() {
        let at = At {
            kind: "damage_type",
            id,
        };
        let default_resist = def.default_resist.as_ref().map_or(Resist::NONE, |spec| {
            resist_of(spec, at, "default_resist", errors)
        });
        if def.kinetic {
            match kinetic {
                Some(first) => at.error(
                    errors,
                    "kinetic",
                    format!(
                        "only one damage type may be kinetic, and `{}` already is",
                        damage_types.name(first)
                    ),
                ),
                None => kinetic = DamageTypeId::from_index(position),
            }
        }
        damage_types.entries.push(DamageType {
            default_resist,
            kinetic: def.kinetic,
            devastation: def.devastation.as_ref().map(|kind| kind.0.clone()),
            tags: tags(&def.tags),
        });
    }
    let default_resists: Vec<Resist> = damage_types
        .entries
        .iter()
        .map(|kind| kind.default_resist)
        .collect();

    // Materials.
    materials.entries.push(Material {
        density: Real::ZERO,
        toughness: Real::ZERO,
        span: None,
        bearing: None,
        flammable: false,
        rubble: None,
        color: None,
        tags: Vec::new(),
        resist: default_resists.clone(),
    });
    for (id, def) in &material_defs {
        let at = At {
            kind: "material",
            id,
        };
        let mut resist = default_resists.clone();
        for (kind, spec) in &def.resist {
            let path = format!("resist.{kind}");
            if let Some(kind) = resolve(&damage_types, "damage type", kind, at, &path, errors) {
                resist[kind.index()] = resist_of(spec, at, &path, errors);
            }
        }
        let rubble = def.rubble.as_ref().and_then(|rubble| {
            resolve(&fixtures, "fixture", rubble.as_str(), at, "rubble", errors)
        });
        materials.entries.push(Material {
            density: not_negative(def.density.get(), at, "density", errors),
            toughness: not_negative(def.toughness.get(), at, "toughness", errors),
            span: def.span,
            bearing: def
                .bearing
                .map(|bearing| not_negative(bearing.get(), at, "bearing", errors)),
            flammable: def.flammable,
            rubble,
            color: def.color.clone(),
            tags: tags(&def.tags),
            resist,
        });
    }

    // Atmospheres.
    for (id, def) in &atmosphere_defs {
        let at = At {
            kind: "atmosphere",
            id,
        };
        atmospheres.entries.push(Atmosphere {
            density: not_negative(def.density.get(), at, "density", errors),
            pressure: not_negative(def.pressure.get(), at, "pressure", errors),
            sound_speed: not_negative(def.sound_speed.get(), at, "sound_speed", errors),
            breathable: def.breathable,
            tags: tags(&def.tags),
        });
    }

    // Elements. Built against the name tables, so a fixture's heap may
    // name a block and a material's rubble a fixture.
    let damage_count = damage_types.len();
    let mut floor_entries = vec![none_element(Layer::Floor, damage_count)];
    let mut block_entries = vec![none_element(Layer::Block, damage_count)];
    let mut fixture_entries = vec![none_element(Layer::Fixture, damage_count)];
    {
        let refs = Refs {
            damage_types: &damage_types,
            materials: &materials,
            blocks: &blocks,
        };
        for (id, def) in &floor_defs {
            let input = ElementInput {
                layer: Layer::Floor,
                material: &def.material,
                height: Real::ZERO,
                fill: Real::ZERO,
                coverage: None,
                thickness: def.thickness.get(),
                flags: &def.flags,
                tags: &def.tags,
                resist: &def.resist,
                states: def
                    .states
                    .0
                    .iter()
                    .map(|(name, state)| StateChange {
                        name,
                        add_flags: &state.add_flags,
                        remove_flags: &state.remove_flags,
                        height: None,
                        fill: None,
                        coverage: None,
                        thickness: state.thickness.map(|thickness| thickness.get()),
                        glyph: state.glyph.as_ref(),
                        light: state.light.as_ref(),
                    })
                    .collect(),
                heap: None,
                contact: def.contact.as_ref(),
                interactions: &def.interactions,
                device: def.device.as_ref(),
                glyph: def.glyph.as_ref(),
                light: def.light.as_ref(),
            };
            let at = At { kind: "floor", id };
            floor_entries.push(build_element(&input, at, &refs, errors));
        }
        for (id, def) in &block_defs {
            let input = ElementInput {
                layer: Layer::Block,
                material: &def.material,
                height: def.height.get(),
                fill: def.fill,
                coverage: def.coverage,
                thickness: Real::ZERO,
                flags: &def.flags,
                tags: &def.tags,
                resist: &def.resist,
                states: solid_states(&def.states.0),
                heap: None,
                contact: def.contact.as_ref(),
                interactions: &def.interactions,
                device: def.device.as_ref(),
                glyph: def.glyph.as_ref(),
                light: def.light.as_ref(),
            };
            let at = At { kind: "block", id };
            block_entries.push(build_element(&input, at, &refs, errors));
        }
        for (id, def) in &fixture_defs {
            let input = ElementInput {
                layer: Layer::Fixture,
                material: &def.material,
                height: def.height.get(),
                fill: def.fill,
                coverage: def.coverage,
                thickness: Real::ZERO,
                flags: &def.flags,
                tags: &def.tags,
                resist: &def.resist,
                states: solid_states(&def.states.0),
                heap: def.heap.as_ref(),
                contact: def.contact.as_ref(),
                interactions: &def.interactions,
                device: def.device.as_ref(),
                glyph: def.glyph.as_ref(),
                light: def.light.as_ref(),
            };
            let at = At {
                kind: "fixture",
                id,
            };
            fixture_entries.push(build_element(&input, at, &refs, errors));
        }
    }
    floors.entries = floor_entries;
    blocks.entries = block_entries;
    fixtures.entries = fixture_entries;
    let source_reach = [&floors.entries, &blocks.entries, &fixtures.entries]
        .into_iter()
        .flatten()
        .flat_map(|element| element.lights.iter().flatten())
        .map(LightSource::reach_cells)
        .max();

    let written: Vec<Option<&str>> = floor_defs
        .iter()
        .map(|(_, d)| d.on_destroy.as_deref())
        .collect();
    link_on_destroy(&mut floors, &materials, Layer::Floor, &written, errors);
    let written: Vec<Option<&str>> = block_defs
        .iter()
        .map(|(_, d)| d.on_destroy.as_deref())
        .collect();
    link_on_destroy(&mut blocks, &materials, Layer::Block, &written, errors);
    let written: Vec<Option<&str>> = fixture_defs
        .iter()
        .map(|(_, d)| d.on_destroy.as_deref())
        .collect();
    link_on_destroy(&mut fixtures, &materials, Layer::Fixture, &written, errors);

    // Realms.
    for (id, def) in &realm_defs {
        let at = At { kind: "realm", id };
        let namespace = Some(namespace_of(id));
        let wraps = |axis: WrapAxis| def.wrap.contains(&axis);
        let extent = |regions: u32, wrapped: bool, path: &str, errors: &mut Errors| {
            let cells = i64::from(regions) * i64::from(REGION_CELLS);
            match i32::try_from(cells) {
                Ok(cells) if cells > 0 => {
                    if wrapped {
                        Extent::Wrapped(cells)
                    } else {
                        Extent::Bounded(cells)
                    }
                }
                _ => {
                    let most = i32::MAX / REGION_CELLS;
                    at.error(errors, path, format!("must be from 1 to {most} regions"));
                    Extent::Bounded(REGION_CELLS)
                }
            }
        };
        let x = extent(
            def.size_regions[0],
            wraps(WrapAxis::X),
            "size_regions.0",
            errors,
        );
        let y = extent(
            def.size_regions[1],
            wraps(WrapAxis::Y),
            "size_regions.1",
            errors,
        );
        let [z_min, z_max] = def.z_range;
        if z_min > z_max {
            at.error(errors, "z_range", "the lowest storey comes first");
        }
        let topology = Topology::new(x, y, z_min.min(z_max), z_max.max(z_min));

        let mut strata: Vec<Stratum> = Vec::with_capacity(def.strata.len());
        for (position, stratum) in def.strata.iter().enumerate() {
            let path = format!("strata.{position}");
            if !stratum.gravity.is_finite() {
                at.error(errors, &format!("{path}.gravity"), "must be finite");
            }
            let atmosphere = stratum.atmosphere.as_ref().and_then(|atmosphere| {
                resolve(
                    &atmospheres,
                    "atmosphere",
                    atmosphere.as_str(),
                    at,
                    &format!("{path}.atmosphere"),
                    errors,
                )
            });
            let sky_id = stratum.sky.as_ref().and_then(|sky| {
                resolve(
                    &skies,
                    "sky",
                    sky.as_str(),
                    at,
                    &format!("{path}.sky"),
                    errors,
                )
            });
            strata.push(Stratum {
                from_z: stratum.from_z,
                gravity: stratum.gravity,
                atmosphere,
                sky: stratum.sky.as_ref().map(|sky| sky.0.clone()),
                sky_id,
                temp_offset: stratum.temp_offset,
                temp: stratum.temp.map(|temp| temp.get()),
            });
        }
        if strata.is_empty() {
            at.error(errors, "strata", "a realm needs at least one stratum");
        }
        strata.sort_by_key(|stratum| stratum.from_z);
        if strata
            .windows(2)
            .any(|pair| pair[0].from_z == pair[1].from_z)
        {
            at.error(errors, "strata", "two strata start at the same storey");
        }

        let fill = |text: Option<&String>, path: &str, errors: &mut Errors| match text {
            None => BlockState::NONE,
            Some(text) => match resolve_state(&blocks, &materials, Layer::Block, text, namespace) {
                Ok(state) => BlockState::from_raw(state),
                Err(error) => {
                    at.error(errors, path, error.to_string());
                    BlockState::NONE
                }
            },
        };
        let fills = def.fill.as_ref();
        let below_fill = fill(fills.and_then(|f| f.below.as_ref()), "fill.below", errors);
        let above_fill = fill(fills.and_then(|f| f.above.as_ref()), "fill.above", errors);

        if def.time_rate < Real::ZERO || !def.time_rate.is_finite() {
            at.error(errors, "time_rate", "must be zero or more");
        }

        let edges = Edge::ALL.map(|edge| {
            let (spec, wrapped) = match edge {
                Edge::West => (&def.edges.west, wraps(WrapAxis::X)),
                Edge::East => (&def.edges.east, wraps(WrapAxis::X)),
                Edge::North => (&def.edges.north, wraps(WrapAxis::Y)),
                Edge::South => (&def.edges.south, wraps(WrapAxis::Y)),
                Edge::Bottom => (&def.edges.bottom, false),
                Edge::Top => (&def.edges.top, false),
            };
            if wrapped {
                if spec.is_some() {
                    let path = format!("edges.{}", edge.name());
                    at.error(errors, &path, "this axis wraps, so it has no edge");
                }
                return EdgeRule::Wrapped;
            }
            match spec {
                None | Some(EdgeSpec::Kind(EdgeKind::Barrier)) => EdgeRule::Barrier,
                Some(EdgeSpec::Kind(EdgeKind::Void)) => EdgeRule::Void,
                Some(EdgeSpec::Link(link)) => EdgeRule::Link(link.clone()),
            }
        });

        realms.entries.push(RealmKind {
            topology,
            size_regions: def.size_regions,
            sea_level: def.sea_level,
            calendar: def.calendar.as_ref().map(|calendar| calendar.0.clone()),
            generator: def.generator.clone(),
            generator_params: def.generator_params.clone(),
            below_fill,
            above_fill,
            strata,
            time_rate: def.time_rate,
            edges,
        });
    }

    // Planes. Plane 0 is the built-in one; a plane's id is its bit in a
    // plane mask, so there is room for a fixed number.
    let mut planes: Table<PlaneId, Plane> = Table {
        names: vec![DEFAULT_PLANE_ID.to_owned()],
        by_name: BTreeMap::from([(DEFAULT_PLANE_ID.to_owned(), PlaneId::new(0))]),
        entries: vec![Plane::default()],
    };
    for (id, def) in &plane_defs {
        let position = planes.names.len();
        match PlaneId::from_index(position).filter(|_| position < MAX_PLANES) {
            Some(plane) => {
                planes.names.push(id.clone());
                planes.by_name.insert(id.clone(), plane);
                planes.entries.push(Plane {
                    tags: tags(&def.tags),
                });
            }
            None => errors.push(
                "plane",
                id,
                "id",
                format!("there are at most {MAX_PLANES} planes, counting the built-in one"),
            ),
        }
    }

    // Area kinds.
    let mut area_kinds: Table<AreaKindId, AreaKind> =
        index("area_kind", &area_kind_defs, false, errors);
    for (id, def) in &area_kind_defs {
        let at = At {
            kind: "area_kind",
            id,
        };
        let mut vars = VarMap::new();
        for (name, value) in &def.vars {
            match plain_value(value) {
                Some(value) => {
                    vars.set(VarKey::new(name.as_str()), value);
                }
                None => at.error(
                    errors,
                    &format!("vars.{name}"),
                    "must be a boolean, a number or a text",
                ),
            }
        }
        if let Some(rate) = def.clock_rate
            && (rate < Real::ZERO || !rate.is_finite())
        {
            at.error(errors, "clock_rate", "must be zero or more");
        }
        area_kinds.entries.push(AreaKind {
            tags: tags(&def.tags),
            vars,
            clock_rate: def.clock_rate,
            priority: def.priority,
            sealed: def.sealed,
            leak: def.leak.clone(),
            ambience: def.ambience.as_ref().map(|ambience| ambience.0.clone()),
            rules: def.rules.clone(),
        });
    }

    if !errors.list.is_empty() {
        return Err(core::mem::take(&mut errors.list));
    }
    Ok(WorldDefs {
        damage_types,
        materials,
        atmospheres,
        floors,
        blocks,
        fixtures,
        realms,
        planes,
        area_kinds,
        kinetic,
        skies,
        sky_reach,
        source_reach,
    })
}

/// A variable's value as data writes it: a boolean, a whole number, a
/// number or a text. Anything else is not a value.
fn plain_value(value: &serde_json::Value) -> Option<Value> {
    match value {
        serde_json::Value::Bool(flag) => Some(Value::Bool(*flag)),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(whole) => Some(Value::Int(whole)),
            None => number
                .as_f64()
                .and_then(|real| Real::new(real).ok())
                .map(Value::Real),
        },
        serde_json::Value::String(text) => Some(Value::Str(text.clone())),
        _ => None,
    }
}

/// The declared states of a block or fixture.
fn solid_states(states: &[(String, super::StateSpec)]) -> Vec<StateChange<'_>> {
    states
        .iter()
        .map(|(name, state)| StateChange {
            name,
            add_flags: &state.add_flags,
            remove_flags: &state.remove_flags,
            height: state.height.map(|height| height.get()),
            fill: state.fill,
            coverage: state.coverage,
            thickness: None,
            glyph: state.glyph.as_ref(),
            light: state.light.as_ref(),
        })
        .collect()
}
