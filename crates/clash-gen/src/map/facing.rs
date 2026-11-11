//! States that have a facing, and how a turn changes them (10 §3.3).
//!
//! The world gives a definition named states and gives the names no
//! meaning. A map does: a state named for a compass direction, alone
//! (`north`) or after an underscore (`open_north`), **faces** that way,
//! provided its definition has the states for the other three
//! quarter-turns as well. When a map is turned, such a state becomes
//! the state of the same definition that faces the turned direction.
//! Everything else is carried over as it is.

use clash_core::{Dir8, Rot4, Turn};
use clash_world::defs::Element;
use clash_world::{ElementState, Layer, WorldDefs};

/// The eight directions with their names in data, longest names first
/// so that `north_east` is not read as something facing east.
const DIRECTIONS: [(Dir8, &str); 8] = [
    (Dir8::NorthEast, "north_east"),
    (Dir8::SouthEast, "south_east"),
    (Dir8::SouthWest, "south_west"),
    (Dir8::NorthWest, "north_west"),
    (Dir8::North, "north"),
    (Dir8::South, "south"),
    (Dir8::East, "east"),
    (Dir8::West, "west"),
];

/// What joins a state's own name to the direction it faces.
const FACING_SEPARATOR: char = '_';

/// The name of a direction in data.
pub fn direction_name(dir: Dir8) -> &'static str {
    DIRECTIONS
        .iter()
        .find(|(have, _)| *have == dir)
        .map_or("north", |(_, name)| name)
}

/// The direction a state's name says it faces, with the part of the
/// name before it: `("open_", East)` for `open_east`, `("", North)` for
/// `north`. `None` for a name that names no direction.
pub fn facing_of(name: &str) -> Option<(&str, Dir8)> {
    for (dir, word) in DIRECTIONS {
        if name == word {
            return Some(("", dir));
        }
        if let Some(prefix) = name.strip_suffix(word)
            && prefix.ends_with(FACING_SEPARATOR)
        {
            return Some((prefix, dir));
        }
    }
    None
}

/// The position of a turn in [`Turn::ALL`].
pub(crate) fn turn_index(turn: Turn) -> usize {
    turn.rot.quarter_turns() as usize + if turn.mirror { Rot4::ALL.len() } else { 0 }
}

/// For every state that has a facing, the state it becomes under each
/// of the eight turns.
#[derive(Clone, Debug, Default)]
pub(crate) struct FacingTable {
    /// By layer, then by definition, then by state: the bits under each
    /// turn. A definition none of whose states face anywhere has no
    /// rows.
    layers: [Vec<Vec<[u16; 8]>>; 3],
}

impl FacingTable {
    pub(crate) fn build(world: &WorldDefs) -> FacingTable {
        FacingTable {
            layers: [
                world.floors().iter().map(|(_, e)| rows(e)).collect(),
                world.blocks().iter().map(|(_, e)| rows(e)).collect(),
                world.fixtures().iter().map(|(_, e)| rows(e)).collect(),
            ],
        }
    }

    /// The state an element state becomes when its map is turned.
    pub(crate) fn turned(&self, layer: Layer, state: ElementState, turn: Turn) -> ElementState {
        let bits = self.layers[layer.index()]
            .get(usize::from(state.def))
            .and_then(|rows| rows.get(usize::from(state.bits)))
            .map(|row| row[turn_index(turn)]);
        match bits {
            Some(bits) => ElementState { bits, ..state },
            None => state,
        }
    }
}

/// The rows of one definition: empty unless some state has a facing.
fn rows(element: &Element) -> Vec<[u16; 8]> {
    let names = &element.state_names;
    let index_of = |name: &str| {
        names
            .iter()
            .position(|have| have == name)
            .and_then(|index| u16::try_from(index).ok())
    };
    let mut any = false;
    let mut table = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let own = u16::try_from(index).unwrap_or(0);
        let mut row = [own; 8];
        if let Some((prefix, dir)) = facing_of(name) {
            let target =
                |turn: Turn| index_of(&format!("{prefix}{}", direction_name(dir.turned(turn))));
            // A state faces somewhere only if the definition can face
            // every way a quarter-turn leads: then turning composes.
            let complete = Rot4::ALL
                .into_iter()
                .all(|rot| target(Turn::from(rot)).is_some());
            if complete {
                for turn in Turn::ALL {
                    if let Some(bits) = target(turn) {
                        row[turn_index(turn)] = bits;
                    }
                }
                any = true;
            }
        }
        table.push(row);
    }
    if any { table } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_say_which_way_a_state_faces() {
        assert_eq!(facing_of("north"), Some(("", Dir8::North)));
        assert_eq!(facing_of("open_east"), Some(("open_", Dir8::East)));
        assert_eq!(facing_of("north_east"), Some(("", Dir8::NorthEast)));
        assert_eq!(facing_of("lit_south_west"), Some(("lit_", Dir8::SouthWest)));
        assert_eq!(facing_of("open"), None);
        assert_eq!(facing_of("northern"), None);
        assert_eq!(facing_of("least"), None);
    }

    #[test]
    fn a_turn_has_one_place_among_the_eight() {
        for (index, turn) in Turn::ALL.into_iter().enumerate() {
            assert_eq!(turn_index(turn), index);
        }
    }
}
