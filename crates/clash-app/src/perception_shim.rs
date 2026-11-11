//! **A stand-in, to be deleted.** What the player's character sees,
//! until perception exists (15 §13, 12 §5).
//!
//! The scene's view model shows only what it is told. Perception, which
//! will tell it what each controlled character perceived, is not built.
//! So that the play screen already behaves as belief will, this module
//! feeds the view model from sight alone, each time the simulation has
//! moved:
//!
//! 1. Everything known becomes remembered (`unsee_all`).
//! 2. The field of view is worked out from the character's cell, with
//!    an eye [`EYE_HEIGHT_MM`] above its base, out to
//!    [`SIGHT_RADIUS_M`] (`Cosmos::field_of_view`).
//! 3. Each cell in it is observed as it is now, with its light
//!    (`Cosmos::fill_light`).
//! 4. Each occupant standing in a cell in it becomes a sprite; a sprite
//!    whose owner is no longer in view is removed.
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
//! creature looks. A thing that leaves view is forgotten, not kept at
//! its last known place.
//!
//! # The omniscient view
//!
//! [`feed_omniscient`] is the debugging view of 12 §5: it fills the
//! view model from truth round the character. The application calls it
//! only while the preference named [`PREF_OMNISCIENT`] is on, which
//! only a development profile defines. Switching either way empties the
//! view model first, so that nothing learnt from truth is left behind
//! as memory.
//!
//! # When perception lands
//!
//! Perception makes the `observe` calls from each character's percepts
//! and playback applies events as they are played. This file and
//! `Session::fov` are deleted, and `Session::known` with the movement
//! shim that reads it.

use clash_core::{IVec3, Real};
use clash_scene::GlyphPart;
use clash_scene::view::{
    CellContents, DecalSeen, EntitySprite, LIGHT_FULL, SpriteId, SpriteKind, WorldView, omniscient,
};
use clash_world::{CellBox, Location, OccupantFilter, OccupantKind, SkyTime};

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

/// A light level as the view model holds it: 0 to [`LIGHT_FULL`], full
/// at daylight (level 1) and above.
fn light_byte(level: Real) -> u8 {
    let share = level.clamp(Real::ZERO, Real::ONE).get();
    (share * f64::from(LIGHT_FULL)).round() as u8
}

/// The sprite an occupant is shown as: the player's character with its
/// own look, anything else as a presence whose look nothing says.
fn sprite(
    session: &Session,
    at: Location,
    id: clash_core::EntityId,
    kind: OccupantKind,
    look: GlyphPart,
) -> EntitySprite {
    let own = id == session.player;
    EntitySprite {
        loc: at,
        kind: if kind == OccupantKind::Creature {
            SpriteKind::Creature
        } else {
            SpriteKind::Object
        },
        glyph: if own { look } else { GlyphPart::default() },
        own: None,
        dress: None,
        facing: None,
        suspected: !own,
    }
}

/// Removes every sprite whose number is not in `keep`.
fn drop_unseen(view: &mut WorldView, keep: &[SpriteId]) {
    let gone: Vec<SpriteId> = view
        .model()
        .sprites()
        .map(|(id, _)| id)
        .filter(|id| !keep.contains(id))
        .collect();
    for id in gone {
        view.model_mut().remove_sprite(id);
    }
}

/// Feeds the view model with what the player's character sees now. See
/// the [module](self) documentation. `look` is the character's own
/// glyph under the theme in force.
pub fn feed(session: &mut Session, view: &mut WorldView, look: GlyphPart) -> Result<(), String> {
    if session.omniscient {
        // What truth put there is not the character's to remember.
        view.model_mut().clear();
        session.omniscient = false;
    }
    let Some(at) = session.player_loc() else {
        view.model_mut().unsee_all();
        drop_unseen(view, &[]);
        return Ok(());
    };
    let world = |error: clash_world::WorldError| error.to_string();
    let cosmos = session.sim.truth().cosmos();
    let grid = cosmos.grid(at.grid).map_err(world)?;
    cosmos
        .field_of_view(at, EYE_HEIGHT_MM, SIGHT_RADIUS_M, &mut session.fov)
        .map_err(world)?;
    let fov = &session.fov;

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
    let time = SkyTime::now(cosmos, session.sim.timeline(), at.grid).map_err(world)?;
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

    let model = view.model_mut();
    model.set_grid_shape(at.grid, *grid.topology());
    model.unsee_all();
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
        model.observe_cell(
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
        session.known.insert(cell);
    }

    // Who stands in view.
    let occupants = cosmos
        .occupants_in_box(at.grid, cells, &OccupantFilter::ANY)
        .map_err(world)?;
    let mut seen = Vec::new();
    for occupied in occupants {
        let loc = Location::new(at.grid, occupied.cell);
        if !fov.contains(loc) {
            continue;
        }
        let id = SpriteId::new(occupied.id.to_bits());
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let sprite = sprite(session, loc, occupied.id, occupied.kind, look);
        view.model_mut().upsert_sprite(id, sprite);
    }
    drop_unseen(view, &seen);
    view.camera_mut().follow(at);
    Ok(())
}

/// Feeds the view model from truth round the player's character: the
/// debugging view. See the [module](self) documentation.
pub fn feed_omniscient(
    session: &mut Session,
    view: &mut WorldView,
    look: GlyphPart,
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
        let sprite = sprite(session, loc, occupied.id, occupied.kind, look);
        view.model_mut().upsert_sprite(id, sprite);
    }
    drop_unseen(view, &seen);
    view.camera_mut().follow(at);
    Ok(())
}
