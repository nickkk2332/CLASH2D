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

use clash_world::dims::TRACT_XY;
use clash_world::{CellPos, CellView, Column, Layer, TractPos, WorldDefs};

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

/// One storey of a set of tracts as text: a line for each row of cells,
/// north to south, with the tracts laid out by their positions. A tract
/// that lies within the block the others span and was not given prints
/// as [`UNKNOWN_CHAR`]. Spaces at the end of a line are left off.
pub fn storey_text(defs: &WorldDefs, tracts: &[(TractPos, &Column)], z: i32) -> String {
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
                        Some(column) => cell_char(defs, column.cell(CellPos::new(cell_x, row, z))),
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
