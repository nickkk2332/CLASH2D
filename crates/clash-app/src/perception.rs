//! What the party perceives, as the simulation's perception says
//! (15 §13; 12 §5, §7.5).
//!
//! The simulation decides what the party sees and hears: its senses
//! sweep, what a member's sight takes in goes into its map memory and a
//! frame, and every event delivered is tagged seen, heard or neither
//! (`clash_sim::perception::party`). This module is how the application
//! turns that into the scene's view model and into the host's answers
//! to playback. It reads the world only for cells the simulation says
//! are in the party's view, and for the things standing in them.
//!
//! | When | What |
//! |------|------|
//! | A character becomes the player's | [`give_senses`]: the senses its templates gave, or else those the start names, or else every sense the loaded mods define (`attach_sense`) |
//! | The simulation has moved | [`look`]: the frames are taken (`drain_frames`), and what is in view as the batch ends is worked out: the cells of the party's view (`party_view`) with what is in them now, and who stands in them. Playback is answered from it. |
//! | Playback asks | [`perceived`] is `Sim::party_perceived`; [`seen_cell`] and [`seen_sprite`] answer from what is in view |
//! | Playback has caught up | [`show`] gives the view model what no event said: the cells the frames recorded, as map memory holds them, the cells that left view, the cells in view as they are now, and who is in view |
//! | A game begins, is loaded, or frames were dropped | The view model is built afresh from the map memory (`snapshot`) and what is in view |
//! | A debug build checks the picture | [`fresh`]: a view model from the map memory and what is in view alone |
//!
//! **What is shown of a cell.** A cell in the party's view is shown as it
//! is now, so that a door seen opening is drawn open at once, before the
//! next sweep records it. Every other cell is shown as the member's map
//! memory holds it: a cell seen changing that leaves view before a sweep
//! records the change is remembered as the sweep last saw it. So the
//! remembered picture is always the simulation's own memory, and a game
//! loaded from a save draws what it drew before (01 R7).
//!
//! **Who is drawn.** The party's own members, always, where they are
//! (12 §10.4: the party's own are known in full), with their own look;
//! and whatever stands in a cell in view and gives off something a sense
//! of light could see: a creature with the glyph of its templates, an
//! item with its definition's, anything else as the theme's `suspected`
//! mark. A thing that leaves view leaves the picture.
//!
//! **What was only heard** reaches the picture through playback: an
//! event tagged heard puts its show's marker and line where it seemed to
//! come from. The frames' lists of what was heard are the same signals,
//! and are not shown a second time.
//!
//! # The omniscient view
//!
//! [`feed_omniscient`] is the debugging view of 12 §5: the view model is
//! filled from truth round the character, and the simulation's switch
//! `perception.omniscient` makes every event seen (`set_omniscient`).
//! The application calls it only while the preference named
//! [`PREF_OMNISCIENT`] is on. Leaving it empties the view model and
//! builds it again from the map memory, which only ever holds what was
//! really seen.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{EntityId, IVec3, Real};
use clash_scene::view::{
    CellContents, DecalSeen, EntitySprite, FieldSeen, LIGHT_FULL, SpriteId, SpriteKind, ViewModel,
    WorldView, omniscient,
};
use clash_scene::{GlyphDef, GlyphInputs, GlyphPart, Perceived, Theme};
use clash_sim::Sim;
use clash_sim::perception::{self, PartyFrame, PartyPerception, Remembered, SenseDef};
use clash_world::{CellBox, CellPos, GridId, Location, OccupantFilter, OccupantKind, SkyTime};

/// The bare name of the preference that switches the omniscient view
/// on. The application acts on it wherever a loaded mod defines it.
pub const PREF_OMNISCIENT: &str = "omniscient";

/// How far the omniscient view reaches from the character along `x`
/// and `y`, in cells.
pub const OMNISCIENT_REACH_CELLS: i32 = 64;

/// How many storeys above and below the character the omniscient view
/// fills.
pub const OMNISCIENT_REACH_STOREYS: i32 = 4;

/// Light as a cell holds it: a level from 0 to 1 in this many steps, as
/// map memory keeps it.
const LIGHT_STEPS: f64 = 255.0;

/// What the party perceives at one moment: as the last batch ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sight {
    /// Where the player's character is, if it is in the world.
    pub at: Option<Location>,
    /// Each cell in the party's view, with what is in it now.
    pub cells: BTreeMap<Location, CellContents>,
    /// Each thing perceived now, by the number of its sprite.
    pub sprites: BTreeMap<SpriteId, EntitySprite>,
}

/// What the application keeps of the party's perception between
/// passes.
#[derive(Clone, Debug, Default)]
pub struct Percepts {
    /// What is perceived now: as of the end of the last batch.
    pub sight: Sight,
    /// Every entity ever perceived.
    pub ever: BTreeSet<EntityId>,
    /// The frames taken from the simulation that the view model has not
    /// been given yet, oldest first.
    pub frames: Vec<PartyFrame>,
    /// The cells the view model was last given as seen.
    pub shown: BTreeSet<Location>,
    /// Whether the view model must be built afresh from the map memory:
    /// a game begun or loaded, frames dropped, the omniscient view left.
    pub rebuild: bool,
    /// Whether the view model has yet to be given `sight`.
    pub unshown: bool,
}

impl Percepts {
    /// Percepts that build the view model afresh at the next chance.
    pub fn afresh() -> Percepts {
        Percepts {
            rebuild: true,
            unshown: true,
            ..Percepts::default()
        }
    }
}

/// A light level as the view model holds it: 0 to [`LIGHT_FULL`], full
/// at daylight (level 1) and above. The same rounding as map memory's.
fn light_byte(level: Real) -> u8 {
    let share = level.clamp(Real::ZERO, Real::ONE).get();
    let byte = (share * LIGHT_STEPS).round();
    if byte >= f64::from(LIGHT_FULL) {
        LIGHT_FULL
    } else if byte <= 0.0 {
        0
    } else {
        byte as u8
    }
}

/// What the view model is told of a remembered cell.
pub fn remembered(seen: &Remembered) -> CellContents {
    CellContents {
        floor: seen.floor,
        block: seen.block,
        fixture: seen.fixture,
        field: seen
            .field
            .filter(|(_, level)| *level > 0)
            .map(|(field, level)| FieldSeen { field, level }),
        decal: seen.decal.map(|(kind, amount)| DecalSeen { kind, amount }),
        pile: false,
        open_sky: seen.open_sky,
        light: seen.light,
    }
}

/// Gives a character the senses it perceives with, if its templates gave
/// it none (15 §4): those named, by qualified id, or with none named
/// every `sense` the loaded mods define, in order of qualified id. It
/// then belongs to the party once its control is direct (15 §13).
///
/// Senses are given directly (`attach_sense`), not by command: the
/// simulation has no command for it until templates carry `senses`
/// (15 §15). The application does it once, before the character is put
/// in the world.
pub fn give_senses(sim: &mut Sim, entity: EntityId, named: &[String]) -> Result<(), String> {
    if sim.components().senses.contains(entity) {
        return Ok(());
    }
    let names: Vec<String> = if named.is_empty() {
        sim.defs()
            .try_registry::<SenseDef>()
            .map(|senses| {
                let mut names: Vec<String> = senses.names().to_vec();
                names.sort();
                names
            })
            .unwrap_or_default()
    } else {
        named.to_vec()
    };
    for name in names {
        let sense = sim
            .perception_defs()
            .sense_named(&name)
            .ok_or_else(|| format!("there is no sense `{name}` to give the character"))?;
        perception::attach_sense(sim, entity, sense)
            .map_err(|reason| format!("the character could not be given `{name}`: {reason}"))?;
    }
    Ok(())
}

/// The glyph of the interface for a look written as a character and two
/// colours, if it reads as one.
fn glyph_of(ch: &str, fg: Option<&str>, bg: Option<&str>) -> Option<GlyphDef> {
    let mut written = serde_json::Map::new();
    written.insert("ch".to_owned(), serde_json::Value::from(ch));
    if let Some(fg) = fg {
        written.insert("fg".to_owned(), serde_json::Value::from(fg));
    }
    if let Some(bg) = bg {
        written.insert("bg".to_owned(), serde_json::Value::from(bg));
    }
    serde_json::from_value(serde_json::Value::Object(written)).ok()
}

/// How a thing is drawn: the glyph of its templates (06 §10.4), or of
/// its item definition, if either gives one that reads.
pub fn look_of(sim: &Sim, entity: EntityId) -> Option<GlyphDef> {
    if let Some(glyph) = sim.glyph(entity) {
        return glyph_of(&glyph.ch, glyph.fg.as_deref(), glyph.bg.as_deref());
    }
    let def = sim.components().item.get(entity)?.def()?;
    let item = sim
        .defs()
        .try_registry::<clash_sim::item::ItemDef>()?
        .try_get(def)?;
    let glyph = item.glyph.as_ref()?;
    glyph_of(
        glyph.ch.as_deref()?,
        glyph.fg.as_deref(),
        glyph.bg.as_deref(),
    )
}

/// A look under a theme: its glyph, or the character alone in the
/// theme's own text colour where a colour of it is one the theme lacks.
pub fn part_of(look: &GlyphDef, theme: &Theme) -> GlyphPart {
    match look.compile(theme) {
        Ok(glyph) => glyph.part(&GlyphInputs::default()),
        Err(_) => GlyphPart {
            ch: look.ch,
            ..GlyphPart::default()
        },
    }
}

/// The sprite a thing is drawn as: with its own look if it has one,
/// else as a presence whose look nothing says.
fn sprite(loc: Location, creature: bool, look: Option<GlyphPart>) -> EntitySprite {
    EntitySprite {
        loc,
        kind: if creature {
            SpriteKind::Creature
        } else {
            SpriteKind::Object
        },
        glyph: look.unwrap_or_default(),
        own: None,
        dress: None,
        facing: None,
        suspected: look.is_none(),
    }
}

/// Whether nothing of an entity can be seen: it was given a standing
/// signature, and gives off nothing of any kind of signal that uses
/// light (15 §3.2). Such a thing is not in a frame either.
fn unseen_by_light(sim: &Sim, entity: EntityId) -> bool {
    let Some(signature) = sim.components().signature.get(entity) else {
        return false;
    };
    let defs = sim.perception_defs();
    !signature.entries().is_empty()
        && signature.entries().iter().all(|(kind, strength)| {
            *strength <= Real::ZERO && defs.kind(*kind).is_some_and(|info| info.uses_light)
        })
}

/// What is in the cells of a view now, as the simulation's perception
/// would record it: the elements, the strongest field and the most
/// marked decal, whether the cell is open to the sky, and its light by
/// the sky and the fixed sources (15 §13).
fn contents_now(sim: &Sim, cells: &[Location]) -> BTreeMap<Location, CellContents> {
    let cosmos = sim.truth().cosmos();
    let mut by_grid: BTreeMap<GridId, Vec<Location>> = BTreeMap::new();
    for loc in cells {
        by_grid.entry(loc.grid).or_default().push(*loc);
    }
    let mut out = BTreeMap::new();
    for (grid, locs) in by_grid {
        let Some(first) = locs.first() else {
            continue;
        };
        let (mut low, mut high) = (first.cell, first.cell);
        for loc in &locs {
            low = CellPos::new(
                low.x.min(loc.cell.x),
                low.y.min(loc.cell.y),
                low.z.min(loc.cell.z),
            );
            high = CellPos::new(
                high.x.max(loc.cell.x),
                high.y.max(loc.cell.y),
                high.z.max(loc.cell.z),
            );
        }
        let bounds = CellBox::new(low, high);
        let mut light = Vec::new();
        let lit = SkyTime::now(cosmos, sim.timeline(), grid)
            .and_then(|time| cosmos.fill_light(grid, bounds, time, &[], &mut light))
            .is_ok();
        let width = (high.x - low.x + 1) as usize;
        let rows = (high.y - low.y + 1) as usize;
        let any_fields = cosmos.field_state().tracts().next().is_some();
        for loc in locs {
            let Ok(states) = cosmos.cell(loc) else {
                continue;
            };
            let level = if lit {
                let index = ((loc.cell.z - low.z) as usize * rows + (loc.cell.y - low.y) as usize)
                    * width
                    + (loc.cell.x - low.x) as usize;
                light.get(index).copied().unwrap_or(Real::ZERO)
            } else {
                Real::ZERO
            };
            let field = if any_fields {
                cosmos.fields_at(loc).ok().and_then(|fields| {
                    fields
                        .into_iter()
                        .max_by_key(|(id, level)| (*level, core::cmp::Reverse(*id)))
                })
            } else {
                None
            };
            let decal = cosmos
                .decals(loc)
                .ok()
                .and_then(|decals| decals.iter().max_by_key(|decal| decal.amount))
                .map(|decal| DecalSeen {
                    kind: decal.kind,
                    amount: decal.amount,
                });
            out.insert(
                loc,
                CellContents {
                    floor: states.floor,
                    block: states.block,
                    fixture: states.fixture,
                    field: field
                        .filter(|(_, level)| *level > 0)
                        .map(|(field, level)| FieldSeen { field, level }),
                    decal,
                    pile: false,
                    open_sky: cosmos
                        .static_light(loc)
                        .is_ok_and(|light| light.sky_steps == 0),
                    light: light_byte(level),
                },
            );
        }
    }
    out
}

/// Works out what the party perceives as a batch ends, and takes the
/// simulation's frames: see the [module](self) documentation. `player`
/// is the player's character and `own` its look under the theme in
/// force; `theme` draws everything else.
pub fn look(
    sim: &mut Sim,
    player: EntityId,
    own: Option<GlyphPart>,
    theme: &Theme,
    percepts: &mut Percepts,
) {
    let (frames, dropped) = perception::drain_frames(sim);
    percepts.frames.extend(frames);
    percepts.rebuild |= dropped;
    percepts.unshown = true;

    let view = perception::party::party_view(sim);
    let cells = contents_now(sim, &view);
    let mut sprites = BTreeMap::new();
    let at = sim.truth().location(player);
    // The party's own are known in full, wherever they are.
    if let Some(at) = at {
        sprites.insert(SpriteId::new(player.to_bits()), sprite(at, true, own));
    }
    // Whoever stands in a cell in view.
    let mut boxes: BTreeMap<GridId, CellBox> = BTreeMap::new();
    for loc in &view {
        let bounds = boxes
            .entry(loc.grid)
            .or_insert(CellBox::new(loc.cell, loc.cell));
        bounds.min = CellPos::new(
            bounds.min.x.min(loc.cell.x),
            bounds.min.y.min(loc.cell.y),
            bounds.min.z.min(loc.cell.z),
        );
        bounds.max = CellPos::new(
            bounds.max.x.max(loc.cell.x),
            bounds.max.y.max(loc.cell.y),
            bounds.max.z.max(loc.cell.z),
        );
    }
    for (grid, bounds) in boxes {
        let Ok(found) = sim
            .truth()
            .cosmos()
            .occupants_in_box(grid, bounds, &OccupantFilter::ANY)
        else {
            continue;
        };
        for occupied in found {
            let loc = Location::new(grid, occupied.cell);
            let id = SpriteId::new(occupied.id.to_bits());
            if occupied.id == player
                || sprites.contains_key(&id)
                || !cells.contains_key(&loc)
                || unseen_by_light(sim, occupied.id)
            {
                continue;
            }
            let look = look_of(sim, occupied.id).map(|look| part_of(&look, theme));
            sprites.insert(
                id,
                sprite(loc, occupied.kind == OccupantKind::Creature, look),
            );
            percepts.ever.insert(occupied.id);
        }
    }
    percepts.ever.insert(player);
    percepts.sight = Sight { at, cells, sprites };
}

/// How the party perceived an event, as the simulation tagged it.
pub fn perceived(sim: &Sim, event: &clash_sim::SimEvent) -> Perceived {
    match sim.party_perceived(event) {
        PartyPerception::Seen => Perceived::Seen,
        PartyPerception::Heard { from } => Perceived::Heard { from },
        PartyPerception::Not => Perceived::Not,
    }
}

/// What the party sees in a cell now, if it is in view.
pub fn seen_cell(percepts: &Percepts, at: Location) -> Option<CellContents> {
    percepts.sight.cells.get(&at).copied()
}

/// How the party perceives a thing: its sprite's number and how it is
/// perceived now, or no look if it is not perceived now; nothing for a
/// thing never perceived.
pub fn seen_sprite(
    percepts: &Percepts,
    entity: EntityId,
) -> Option<(SpriteId, Option<EntitySprite>)> {
    let id = SpriteId::new(entity.to_bits());
    match percepts.sight.sprites.get(&id) {
        Some(seen) => Some((id, Some(*seen))),
        None => percepts.ever.contains(&entity).then_some((id, None)),
    }
}

/// Every cell a member remembers, from its map memory.
fn memory_of(sim: &Sim, member: EntityId) -> Vec<(Location, CellContents)> {
    perception::party::map_memory(sim, member)
        .map(|memory| {
            memory
                .cells()
                .map(|(loc, seen)| (loc, remembered(seen)))
                .collect()
        })
        .unwrap_or_default()
}

/// Gives the view model the shapes of the grids it holds cells of.
fn shapes(sim: &Sim, model: &mut ViewModel, grids: impl IntoIterator<Item = GridId>) {
    for grid in grids.into_iter().collect::<BTreeSet<_>>() {
        if let Ok(topology) = sim.truth().cosmos().topology(grid) {
            model.set_grid_shape(grid, *topology);
        }
    }
}

/// A view model initialised from the party's map memory and what it
/// perceives now (12 §5): what the played picture must equal once
/// playback has caught up.
pub fn fresh(sim: &Sim, player: EntityId, percepts: &Percepts) -> ViewModel {
    let mut model = ViewModel::new();
    let memory = memory_of(sim, player);
    shapes(
        sim,
        &mut model,
        memory
            .iter()
            .map(|(loc, _)| loc.grid)
            .chain(percepts.sight.cells.keys().map(|loc| loc.grid)),
    );
    for (loc, seen) in memory {
        model.observe_cell(loc, seen);
        model.unsee_cell(loc);
    }
    for (loc, seen) in &percepts.sight.cells {
        model.observe_cell(*loc, *seen);
    }
    for (id, seen) in &percepts.sight.sprites {
        model.upsert_sprite(*id, *seen);
    }
    model
}

/// Gives the view model what the party perceives now that no event
/// said, once playback has caught up: see the [module](self)
/// documentation. Built afresh when [`Percepts::rebuild`] says so.
pub fn show(sim: &Sim, player: EntityId, view: &mut WorldView, percepts: &mut Percepts) {
    percepts.unshown = false;
    if std::mem::take(&mut percepts.rebuild) {
        let fresh = fresh(sim, player, percepts);
        *view.model_mut() = fresh;
    } else {
        let memory = perception::party::map_memory(sim, player);
        let model = view.model_mut();
        let mut touched: BTreeSet<Location> = std::mem::take(&mut percepts.shown);
        for frame in &percepts.frames {
            touched.extend(frame.seen.iter().map(|cell| cell.loc));
            touched.extend(frame.unseen.iter().copied());
        }
        shapes(sim, model, touched.iter().map(|loc| loc.grid));
        for loc in touched {
            if percepts.sight.cells.contains_key(&loc) {
                continue;
            }
            match memory.and_then(|memory| memory.cell(loc)) {
                Some(seen) => {
                    let seen = remembered(seen);
                    if model.cell(loc).map(|known| known.contents()) != Some(seen) {
                        model.observe_cell(loc, seen);
                    }
                    model.unsee_cell(loc);
                }
                None => {
                    model.forget_cell(loc);
                }
            }
        }
        shapes(sim, model, percepts.sight.cells.keys().map(|loc| loc.grid));
        for (loc, seen) in &percepts.sight.cells {
            let known = model.cell(*loc);
            if known.is_none_or(|known| !known.seen() || known.contents() != *seen) {
                model.observe_cell(*loc, *seen);
            }
        }
        let gone: Vec<SpriteId> = model
            .sprites()
            .map(|(id, _)| id)
            .filter(|id| !percepts.sight.sprites.contains_key(id))
            .collect();
        for id in gone {
            model.remove_sprite(id);
        }
        for (id, seen) in &percepts.sight.sprites {
            if model.sprite(*id) != Some(seen) {
                model.upsert_sprite(*id, *seen);
            }
        }
    }
    percepts.frames.clear();
    percepts.shown = percepts.sight.cells.keys().copied().collect();
    if let Some(at) = percepts.sight.at {
        view.camera_mut().follow(at);
    }
}

/// Feeds the view model from truth round the player's character: the
/// debugging view. See the [module](self) documentation. `own` is the
/// character's look; `theme` draws everything else.
pub fn feed_omniscient(
    sim: &Sim,
    player: EntityId,
    own: Option<GlyphPart>,
    theme: &Theme,
    view: &mut WorldView,
) -> Result<(), String> {
    let Some(at) = sim.truth().location(player) else {
        return Ok(());
    };
    let cosmos = sim.truth().cosmos();
    let reach = IVec3::new(
        OMNISCIENT_REACH_CELLS,
        OMNISCIENT_REACH_CELLS,
        OMNISCIENT_REACH_STOREYS,
    );
    let cells = CellBox::around(at.cell, reach);
    omniscient::fill(view.model_mut(), cosmos, at.grid, cells)
        .map_err(|error| error.to_string())?;
    let occupants = cosmos
        .occupants_in_box(at.grid, cells, &OccupantFilter::ANY)
        .map_err(|error| error.to_string())?;
    let mut seen = vec![SpriteId::new(player.to_bits())];
    view.model_mut()
        .upsert_sprite(seen[0], sprite(at, true, own));
    for occupied in occupants {
        let id = SpriteId::new(occupied.id.to_bits());
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let loc = Location::new(at.grid, occupied.cell);
        let look = look_of(sim, occupied.id).map(|look| part_of(&look, theme));
        view.model_mut().upsert_sprite(
            id,
            sprite(loc, occupied.kind == OccupantKind::Creature, look),
        );
    }
    let gone: Vec<SpriteId> = view
        .model()
        .sprites()
        .map(|(id, _)| id)
        .filter(|id| !seen.contains(id))
        .collect();
    for id in gone {
        view.model_mut().remove_sprite(id);
    }
    view.camera_mut().follow(at);
    Ok(())
}
