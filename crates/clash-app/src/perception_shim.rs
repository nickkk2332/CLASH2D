//! **A stand-in, to be deleted.** What the party perceives, until
//! perception exists (15 §13, 12 §5, §7.5).
//!
//! Playback asks the host what the party perceived of each event, what
//! it sees in a cell and how it perceives a thing (12 §7.5); the view
//! model is initialised from the party's map memory and what it
//! perceives now (12 §5). Perception, which will answer all of that, is
//! not built. So that the play screen already behaves as belief will,
//! this module answers from sight alone, kept in [`Percepts`]:
//!
//! 1. Each time the simulation has moved, [`look`] works out the field
//!    of view from the character's cell, with an eye [`EYE_HEIGHT_MM`]
//!    above its base, out to [`SIGHT_RADIUS_M`]
//!    (`Cosmos::field_of_view`), observes each cell in it as it is now,
//!    with its light (`Cosmos::fill_light`), and makes each occupant
//!    standing in a cell in it a sprite. That is the [`Sight`]: what is
//!    perceived as of the end of the batch the events came in. Every
//!    cell seen is written into the party's map memory.
//! 2. Playback is answered from it: an event was seen if a place it is
//!    about is in sight now or seen in the picture ([`perceived`]); a
//!    cell is seen as [`seen_cell`] says; a thing is perceived as
//!    [`seen_sprite`] says. Nothing is only heard: the stand-in has no
//!    ears.
//! 3. Once playback has caught up, [`show`] gives the view model what
//!    no event says: cells that came into view and went out of it, and
//!    who is in view. A debug build then checks the picture against
//!    [`fresh`], a view model initialised from memory and sight alone.
//!
//! Nothing beyond the field of view reaches the view model: walk into a
//! house and the street behind the wall goes to memory. The world is
//! read here and nowhere else on the way to an ordinary screen, and
//! what is read is only what a pair of eyes at that cell would see.
//!
//! # What it does not do
//!
//! No hearing, no other senses, no light too dim to see by, no
//! personas: every creature but the player's own is drawn as the
//! theme's `suspected` mark, because nothing in this tree says how a
//! creature looks to another. A thing that leaves view is forgotten, not
//! kept at its last known place.
//!
//! # The omniscient view
//!
//! [`feed_omniscient`] is the debugging view of 12 §5: it fills the
//! view model from truth round the character. The application calls it
//! only while the preference named [`PREF_OMNISCIENT`] is on. The `ui`
//! mod defines it with a control that switches it only where the
//! preference `ui:development` holds, which only a development profile
//! sets; it is never kept between runs ([`crate::prefs::kept`]).
//! Switching either way empties the view model first, and the party's
//! memory with it, so that nothing learnt from truth is left behind.
//!
//! # When perception lands
//!
//! Perception keeps each character's percepts and memory and answers
//! the host's questions; this file is deleted, and with it
//! `Session::percepts`. Nothing else in the application changes: the
//! host and the frame loop call the five functions above by name.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{EntityId, IVec3, Real};
use clash_scene::view::{
    CellContents, DecalSeen, EntitySprite, LIGHT_FULL, SpriteId, SpriteKind, ViewModel, WorldView,
    omniscient,
};
use clash_scene::{GlyphPart, Perceived};
use clash_sim::{SimEvent, Subject};
use clash_world::{
    CellBox, FovSet, GridId, Location, OccupantFilter, OccupantKind, SkyTime, Topology,
};

use crate::session::Session;

/// How high the stand-in's eye is above the base of its cell, in
/// millimetres. The real height comes from the body and its posture
/// (06 §11.1).
pub const EYE_HEIGHT_MM: u16 = 1_600;

/// How far the stand-in sees, in metres. The real range comes from the
/// senses and the light (15 §3).
pub const SIGHT_RADIUS_M: Real = Real::of(40.0);

/// The bare name of the preference that switches the omniscient view
/// on. The application acts on it wherever a loaded mod defines it.
pub const PREF_OMNISCIENT: &str = "omniscient";

/// How far the omniscient view reaches from the character along `x`
/// and `y`, in cells.
pub const OMNISCIENT_REACH_CELLS: i32 = 64;

/// How many storeys above and below the character the omniscient view
/// fills.
pub const OMNISCIENT_REACH_STOREYS: i32 = 4;

/// The subjects of an event that say who did it, not where it
/// happened: playback does not count them as places the event is about.
const NOT_PLACES: [&str; 2] = ["actor", "instigator"];

/// What the character perceives at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sight {
    /// The cell its eyes are in, if it is in the world.
    pub at: Option<Location>,
    /// Each cell in view and what is seen there.
    pub cells: BTreeMap<Location, CellContents>,
    /// Each thing in view, by the number of its sprite: how it is
    /// perceived.
    pub sprites: BTreeMap<SpriteId, EntitySprite>,
}

/// What the stand-in keeps for the party between moments.
#[derive(Clone, Debug, Default)]
pub struct Percepts {
    /// The field of view: kept for its storage.
    pub fov: FovSet,
    /// What is perceived now: as of the end of the last batch.
    pub sight: Sight,
    /// Each cell ever seen, as it was last seen: the party's map memory.
    pub memory: BTreeMap<Location, CellContents>,
    /// The shape of each grid seen.
    pub shapes: BTreeMap<GridId, Topology>,
    /// Every entity ever perceived.
    pub ever: BTreeSet<EntityId>,
    /// Whether the view model has yet to be given `sight`.
    pub unshown: bool,
}

impl Percepts {
    /// Forgets everything: the map memory, what was ever perceived, and
    /// what is perceived now.
    pub fn forget(&mut self) {
        self.sight = Sight::default();
        self.memory.clear();
        self.shapes.clear();
        self.ever.clear();
        self.unshown = true;
    }
}

/// A light level as the view model holds it: 0 to [`LIGHT_FULL`], full
/// at daylight (level 1) and above.
fn light_byte(level: Real) -> u8 {
    let share = level.clamp(Real::ZERO, Real::ONE).get();
    (share * f64::from(LIGHT_FULL)).round() as u8
}

/// The sprite an occupant is shown as: the player's character with its
/// own look, anything else as a presence whose look nothing says. A
/// character whose look nothing gives is shown as a presence too.
fn sprite(
    player: EntityId,
    at: Location,
    id: EntityId,
    kind: OccupantKind,
    look: Option<GlyphPart>,
) -> EntitySprite {
    let own = (id == player).then_some(look).flatten();
    EntitySprite {
        loc: at,
        kind: if kind == OccupantKind::Creature {
            SpriteKind::Creature
        } else {
            SpriteKind::Object
        },
        glyph: own.unwrap_or_default(),
        own: None,
        dress: None,
        facing: None,
        suspected: own.is_none(),
    }
}

/// Works out what the player's character perceives now, and keeps it in
/// its percepts: see the [module](self) documentation. `look` is the
/// character's own glyph under the theme in force, if it has one.
pub fn look(session: &mut Session, look: Option<GlyphPart>) -> Result<(), String> {
    let Session {
        sim,
        player,
        percepts,
        ..
    } = session;
    percepts.unshown = true;
    let Some(at) = sim.truth().location(*player) else {
        percepts.sight = Sight::default();
        return Ok(());
    };
    let world = |error: clash_world::WorldError| error.to_string();
    let cosmos = sim.truth().cosmos();
    let grid = cosmos.grid(at.grid).map_err(world)?;
    cosmos
        .field_of_view(at, EYE_HEIGHT_MM, SIGHT_RADIUS_M, &mut percepts.fov)
        .map_err(world)?;
    let fov = &percepts.fov;

    // The box the view fits in, as offsets from the character's cell: a
    // view across a wrapped edge is one piece.
    let mut low = IVec3::new(0, 0, 0);
    let mut high = IVec3::new(0, 0, 0);
    for offset in fov.offsets() {
        low = IVec3::new(
            low.x.min(offset.x),
            low.y.min(offset.y),
            low.z.min(offset.z),
        );
        high = IVec3::new(
            high.x.max(offset.x),
            high.y.max(offset.y),
            high.z.max(offset.z),
        );
    }
    let cells = CellBox::new(at.cell.offset(low), at.cell.offset(high));
    let time = SkyTime::now(cosmos, sim.timeline(), at.grid).map_err(world)?;
    let mut light = Vec::new();
    cosmos
        .fill_light(at.grid, cells, time, &[], &mut light)
        .map_err(world)?;
    let width = (high.x - low.x + 1) as usize;
    let rows = (high.y - low.y + 1) as usize;
    let light_at = |offset: IVec3| {
        let index = ((offset.z - low.z) as usize * rows + (offset.y - low.y) as usize) * width
            + (offset.x - low.x) as usize;
        light.get(index).copied().unwrap_or(Real::ZERO)
    };

    let mut sight = Sight {
        at: Some(at),
        ..Sight::default()
    };
    for offset in fov.offsets() {
        let cell = at.cell.offset(offset);
        let Ok(states) = grid.cell(cell) else {
            continue;
        };
        let decal = grid
            .decals(cell)
            .unwrap_or_default()
            .iter()
            .max_by_key(|decal| decal.amount)
            .map(|decal| DecalSeen {
                kind: decal.kind,
                amount: decal.amount,
            });
        let Ok(cell) = grid.normalize(cell) else {
            continue;
        };
        sight.cells.insert(
            Location::new(at.grid, cell),
            CellContents {
                floor: states.floor,
                block: states.block,
                fixture: states.fixture,
                field: None,
                decal,
                pile: false,
                open_sky: grid.under_open_sky(cell).unwrap_or(false),
                light: light_byte(light_at(offset)),
            },
        );
    }

    // Who stands in view.
    let occupants = cosmos
        .occupants_in_box(at.grid, cells, &OccupantFilter::ANY)
        .map_err(world)?;
    for occupied in occupants {
        let loc = Location::new(at.grid, occupied.cell);
        if !fov.contains(loc) {
            continue;
        }
        let id = SpriteId::new(occupied.id.to_bits());
        if sight.sprites.contains_key(&id) {
            continue;
        }
        let shown = sprite(*player, loc, occupied.id, occupied.kind, look);
        sight.sprites.insert(id, shown);
        percepts.ever.insert(occupied.id);
    }

    percepts.shapes.insert(at.grid, *grid.topology());
    percepts
        .memory
        .extend(sight.cells.iter().map(|(at, seen)| (*at, *seen)));
    percepts.sight = sight;
    Ok(())
}

/// Gives the view model what the party perceives now that no event
/// said: every known cell becomes remembered, the cells in view are
/// observed, the things in view are where they are perceived and the
/// rest leave the picture, and the camera follows the character. Called
/// once playback has caught up.
pub fn show(view: &mut WorldView, percepts: &mut Percepts) {
    percepts.unshown = false;
    let model = view.model_mut();
    for (grid, shape) in &percepts.shapes {
        model.set_grid_shape(*grid, *shape);
    }
    model.unsee_all();
    for (at, seen) in &percepts.sight.cells {
        model.observe_cell(*at, *seen);
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
    if let Some(at) = percepts.sight.at {
        view.camera_mut().follow(at);
    }
}

/// A view model initialised from the party's map memory and what it
/// perceives now (12 §5): what the played picture must equal once
/// playback has caught up.
pub fn fresh(percepts: &Percepts) -> ViewModel {
    let mut model = ViewModel::new();
    for (grid, shape) in &percepts.shapes {
        model.set_grid_shape(*grid, *shape);
    }
    for (at, seen) in &percepts.memory {
        model.observe_cell(*at, *seen);
        if !percepts.sight.cells.contains_key(at) {
            model.unsee_cell(*at);
        }
    }
    for (id, seen) in &percepts.sight.sprites {
        model.upsert_sprite(*id, *seen);
    }
    model
}

/// How the party perceived an event: seen if a cell it is about, or the
/// cell a thing it is about is drawn or perceived in, is in sight now or
/// seen in the picture as playback has it. Who did it does not count:
/// they may be somewhere else. Nothing is only heard.
pub fn perceived(percepts: &Percepts, event: &SimEvent, picture: &ViewModel) -> Perceived {
    let in_sight = |at: Location| {
        percepts.sight.cells.contains_key(&at) || picture.cell(at).is_some_and(|cell| cell.seen())
    };
    let seen = event
        .subjects()
        .iter()
        .filter(|(name, _)| !NOT_PLACES.contains(name))
        .any(|(_, subject)| match subject {
            Subject::Entity(entity) => {
                let id = SpriteId::new(entity.to_bits());
                let drawn = picture.sprite(id).map(|sprite| sprite.loc);
                let perceived = percepts.sight.sprites.get(&id).map(|sprite| sprite.loc);
                drawn.into_iter().chain(perceived).any(in_sight)
            }
            other => other.loc().is_some_and(in_sight),
        });
    if seen {
        Perceived::Seen
    } else {
        Perceived::Not
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

/// Feeds the view model from truth round the player's character: the
/// debugging view. See the [module](self) documentation.
pub fn feed_omniscient(
    session: &mut Session,
    view: &mut WorldView,
    look: Option<GlyphPart>,
) -> Result<(), String> {
    if !session.omniscient {
        view.model_mut().clear();
        session.omniscient = true;
    }
    let Some(at) = session.player_loc() else {
        return Ok(());
    };
    let cosmos = session.sim.truth().cosmos();
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
    let mut seen = Vec::new();
    for occupied in occupants {
        let id = SpriteId::new(occupied.id.to_bits());
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let loc = Location::new(at.grid, occupied.cell);
        let shown = sprite(session.player, loc, occupied.id, occupied.kind, look);
        view.model_mut().upsert_sprite(id, shown);
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

/// Leaves the omniscient view: the picture and the party's memory are
/// emptied, so that nothing learnt from truth is kept, and what is in
/// sight is given again.
pub fn leave_omniscient(session: &mut Session, view: &mut WorldView) {
    view.model_mut().clear();
    session.omniscient = false;
    let sight = std::mem::take(&mut session.percepts.sight);
    session.percepts.forget();
    session
        .percepts
        .memory
        .extend(sight.cells.iter().map(|(at, seen)| (*at, *seen)));
    if let Some(at) = sight.at
        && let Ok(grid) = session.sim.truth().cosmos().grid(at.grid)
    {
        session.percepts.shapes.insert(at.grid, *grid.topology());
    }
    session
        .percepts
        .ever
        .extend(sight.sprites.keys().map(|id| EntityId::from_bits(id.raw())));
    session.percepts.sight = sight;
    session.percepts.unshown = true;
}
