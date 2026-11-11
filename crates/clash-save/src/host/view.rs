//! The host's frame: one storey of the world round the cursor, drawn
//! from truth through the scene's view model (12 §5, the omniscient
//! mode).

use std::sync::Arc;

use clash_core::EntityId;
use clash_data::Defs;
use clash_scene::ui::ViewDef;
use clash_scene::view::omniscient;
use clash_scene::view::{FieldSeen, WorldGlyphs};
use clash_scene::{
    Canvas, EntitySprite, Frame, GlyphPart, SpriteId, SpriteKind, Theme, ThemeDef, UiDefs,
    WorldView,
};
use clash_sim::Sim;
use clash_world::{CellBox, CellPos, Location, OccupantFilter, OccupantKind, WorldDefs};

/// How many storeys below the cursor's the frame is filled from, so that
/// open cells show what lies beneath them.
pub(crate) const FRAME_STOREYS_BELOW: i32 = 4;

/// The character a creature is drawn with. Entities have no look of
/// their own until templates give them one (06).
const CREATURE_MARK: char = '@';
/// The character anything else that occupies a cell is drawn with.
const OBJECT_MARK: char = '!';

/// What frames are drawn with: the look of every element under a theme.
/// Built once for a set of world definitions and kept.
#[derive(Debug)]
pub(crate) struct Painter {
    /// The definitions the glyphs were built for.
    world: Arc<WorldDefs>,
    glyphs: WorldGlyphs,
    def: ViewDef,
}

impl Painter {
    /// A painter for a world's definitions, under the first theme the
    /// loaded mods define, or the plain theme if they define none.
    pub fn new(defs: &Arc<Defs>, world: &Arc<WorldDefs>) -> Result<Painter, String> {
        let theme = UiDefs::from_defs(defs)
            .ok()
            .and_then(|ui| {
                let first = ui.ids::<ThemeDef>().first().cloned()?;
                ui.theme(&first).ok()
            })
            .unwrap_or_default();
        Painter::with_theme(world, &theme)
    }

    fn with_theme(world: &Arc<WorldDefs>, theme: &Theme) -> Result<Painter, String> {
        // What cannot be drawn as written is drawn as well as it can be:
        // this is a debugging view, and `lint` is what reports looks.
        let (glyphs, _) = WorldGlyphs::build(world, theme);
        let def: ViewDef = serde_json::from_str("{}").map_err(|error| error.to_string())?;
        Ok(Painter {
            world: Arc::clone(world),
            glyphs,
            def,
        })
    }

    /// Whether the painter was built for these definitions.
    pub fn is_for(&self, world: &Arc<WorldDefs>) -> bool {
        Arc::ptr_eq(&self.world, world)
    }

    /// The frame: the storey of `at` from above, `at` in the middle,
    /// with what really is there, the strongest field of each cell and
    /// whoever stands in it.
    pub fn paint(&self, sim: &Sim, at: Location, cols: u16, rows: u16) -> Result<Frame, String> {
        let cosmos = sim.truth().cosmos();
        let mut view = WorldView::new(self.glyphs.clone(), at);
        let (half_w, half_h) = (i32::from(cols) / 2 + 1, i32::from(rows) / 2 + 1);
        let cells = CellBox::new(
            CellPos::new(
                at.cell.x - half_w,
                at.cell.y - half_h,
                at.cell.z - FRAME_STOREYS_BELOW,
            ),
            CellPos::new(at.cell.x + half_w, at.cell.y + half_h, at.cell.z),
        );
        omniscient::fill(view.model_mut(), cosmos, at.grid, cells)
            .map_err(|error| error.to_string())?;

        // The strongest field of each cell.
        if let Ok(fields) = cosmos.fields_in_box(at.grid, cells) {
            let mut strongest: Vec<(CellPos, FieldSeen)> = Vec::new();
            for sample in fields {
                let seen = FieldSeen {
                    field: sample.field,
                    level: sample.level,
                };
                match strongest.last_mut() {
                    Some((cell, have)) if *cell == sample.cell => {
                        if seen.level > have.level {
                            *have = seen;
                        }
                    }
                    _ => strongest.push((sample.cell, seen)),
                }
            }
            for (cell, seen) in strongest {
                let loc = Location::new(at.grid, cell);
                if let Some(known) = view.model().cell(loc) {
                    let mut contents = known.contents();
                    contents.field = Some(seen);
                    view.model_mut().observe_cell(loc, contents);
                }
            }
        }

        // Whoever stands there.
        if let Ok(found) = cosmos.occupants_in_box(at.grid, cells, &OccupantFilter::ANY) {
            let mut drawn: Vec<EntityId> = Vec::new();
            for occupied in found {
                if drawn.contains(&occupied.id) {
                    continue;
                }
                drawn.push(occupied.id);
                let creature = occupied.kind == OccupantKind::Creature;
                view.model_mut().upsert_sprite(
                    SpriteId::new(occupied.id.to_bits()),
                    EntitySprite {
                        loc: Location::new(at.grid, occupied.cell),
                        kind: if creature {
                            SpriteKind::Creature
                        } else {
                            SpriteKind::Object
                        },
                        glyph: GlyphPart {
                            ch: Some(if creature { CREATURE_MARK } else { OBJECT_MARK }),
                            ..GlyphPart::default()
                        },
                        own: None,
                        dress: None,
                        facing: None,
                        suspected: false,
                    },
                );
            }
        }

        let mut frame = Frame::new(cols, rows);
        let area = frame.area();
        view.paint_map(&self.def, area, &mut Canvas::new(&mut frame));
        Ok(frame)
    }
}
