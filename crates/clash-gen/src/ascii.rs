//! One storey of generated cells as plain text, so that a person or an
//! agent can look at what a generator made. No colours and no scene:
//! each cell is the character of its topmost element's glyph.
//!
//! ```text
//! ...,##=#=#=#=##,...
//! ...,#...#.....#,...
//! ...,=.>.+..n..=,...
//! ```
//!
//! `cargo run -p clash-gen --example city_ascii -- [seed] [storey]`
//! prints a block of `test_city` this way.

use std::collections::BTreeMap;

use clash_world::dims::{CHUNK_Z_BITS, TRACT_XY};
use clash_world::{CellFlags, CellPos, CellView, Column, Layer, TractPos, WorldDefs};

/// What stands for a cell of a tract that was not given, and for an
/// element whose definition gives its glyph no character.
pub const UNKNOWN_CHAR: char = '?';

/// What stands for a cell that holds nothing.
pub const EMPTY_CHAR: char = ' ';

/// The character that stands for a cell: that of its fixture's glyph if
/// it has a fixture, else its block's, else its floor's, and
/// [`EMPTY_CHAR`] for a cell that holds nothing.
pub fn cell_char(defs: &WorldDefs, cell: CellView) -> char {
    let (layer, state) = if !cell.fixture.is_none() {
        (Layer::Fixture, cell.fixture.raw())
    } else if !cell.block.is_none() {
        (Layer::Block, cell.block.raw())
    } else if !cell.floor.is_none() {
        (Layer::Floor, cell.floor.raw())
    } else {
        return EMPTY_CHAR;
    };
    defs.element(layer, state)
        .and_then(|element| {
            element
                .glyphs
                .get(usize::from(state.bits))
                .or(element.glyphs.first())
        })
        .and_then(|glyph| glyph.ch.as_deref())
        .and_then(|text| text.chars().next())
        .unwrap_or(UNKNOWN_CHAR)
}

/// The storey of the surface of a cell column: the first storey above
/// the highest block with a solid top. Where the ground rolls this
/// differs from column to column. `cell` names the column; its storey
/// is not used.
pub fn surface_storey(column: &Column, cell: CellPos) -> i32 {
    let Some((low, high)) = column.stored_range() else {
        return 0;
    };
    let bottom = low << CHUNK_Z_BITS;
    let top = ((high + 1) << CHUNK_Z_BITS) - 1;
    (bottom..=top)
        .rev()
        .find(|z| {
            column
                .flags(CellPos::new(cell.x, cell.y, *z))
                .contains(CellFlags::SOLID_TOP)
        })
        .map_or(bottom, |z| z + 1)
}

/// A set of tracts as text, seen along the ground: each cell column
/// shows the cell `storey` storeys above its own surface (see
/// [`surface_storey`]), so that 0 is what stands on the ground however
/// the ground rolls, and 1 is the storey over that. Laid out as
/// [`storey_text`] lays tracts out.
pub fn surface_text(defs: &WorldDefs, tracts: &[(TractPos, &Column)], storey: i32) -> String {
    layout_text(tracts, |column, x, y| {
        let surface = surface_storey(column, CellPos::new(x, y, 0));
        cell_char(defs, column.cell(CellPos::new(x, y, surface + storey)))
    })
}

/// One storey of a set of tracts as text: a line for each row of cells,
/// north to south, with the tracts laid out by their positions. A tract
/// that lies within the block the others span and was not given prints
/// as [`UNKNOWN_CHAR`]. Spaces at the end of a line are left off.
pub fn storey_text(defs: &WorldDefs, tracts: &[(TractPos, &Column)], z: i32) -> String {
    layout_text(tracts, |column, x, y| {
        cell_char(defs, column.cell(CellPos::new(x, y, z)))
    })
}

/// Tracts laid out by their positions, a line for each row of cells,
/// with the character `show` gives for each cell column of each tract.
fn layout_text(tracts: &[(TractPos, &Column)], show: impl Fn(&Column, i32, i32) -> char) -> String {
    let by_pos: BTreeMap<TractPos, &Column> = tracts.iter().copied().collect();
    let (Some(west), Some(east), Some(north), Some(south)) = (
        by_pos.keys().map(|tract| tract.x).min(),
        by_pos.keys().map(|tract| tract.x).max(),
        by_pos.keys().map(|tract| tract.y).min(),
        by_pos.keys().map(|tract| tract.y).max(),
    ) else {
        return String::new();
    };
    let mut text = String::new();
    for tract_y in north..=south {
        for row in 0..TRACT_XY {
            let mut line = String::new();
            for tract_x in west..=east {
                let column = by_pos.get(&TractPos::new(tract_x, tract_y));
                for cell_x in 0..TRACT_XY {
                    line.push(match column {
                        Some(column) => show(column, cell_x, row),
                        None => UNKNOWN_CHAR,
                    });
                }
            }
            text.push_str(line.trim_end_matches(EMPTY_CHAR));
            text.push('\n');
        }
    }
    text
}
