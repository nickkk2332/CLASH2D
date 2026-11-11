//! A resolved map as plain characters, for looking at one without a
//! world or a screen.

use clash_world::Layer;

use super::compile::{LEAVE, MapDefs};
use super::resolve::{ResolvedCell, ResolvedMap};

/// What is printed for a cell the map empties: every layer written,
/// and nothing in any of them. A stairwell, for one.
pub const ASCII_EMPTY: char = ':';

/// What is printed for a state whose definition gives it no glyph.
pub const ASCII_NO_GLYPH: char = '?';

impl MapDefs {
    /// The character that stands for a cell of a resolved map: the
    /// glyph of its fixture if it has one, else of its block, else of
    /// its floor. A cell the map leaves alone is a space.
    pub fn ascii_cell(&self, cell: ResolvedCell) -> char {
        if cell.is_leave() {
            return LEAVE;
        }
        let states = [
            (Layer::Fixture, cell.fixture.map(|state| state.raw())),
            (Layer::Block, cell.block.map(|state| state.raw())),
            (Layer::Floor, cell.floor.map(|state| state.raw())),
        ];
        for (layer, state) in states {
            let Some(state) = state.filter(|state| !state.is_none()) else {
                continue;
            };
            let glyph = self.world.element(layer, state).and_then(|element| {
                element
                    .glyphs
                    .get(usize::from(state.bits))
                    .or_else(|| element.glyphs.first())
            });
            return glyph
                .and_then(|glyph| glyph.ch.as_deref())
                .and_then(|ch| ch.chars().next())
                .unwrap_or(ASCII_NO_GLYPH);
        }
        ASCII_EMPTY
    }

    /// A resolved map printed storey by storey, lowest first: a line
    /// naming each level, then its rows from the north.
    ///
    /// ```text
    /// level 0
    /// ###=###
    /// #b...T#
    /// ###+###
    /// ```
    pub fn ascii(&self, map: &ResolvedMap) -> String {
        let mut out = String::new();
        for level in &map.levels {
            out.push_str(&format!("level {}\n", level.z));
            for y in 0..map.size.y {
                let row: String = (0..map.size.x)
                    .map(|x| self.ascii_cell(map.cell(x, y, level.z)))
                    .collect();
                out.push_str(row.trim_end());
                out.push('\n');
            }
        }
        out
    }
}
