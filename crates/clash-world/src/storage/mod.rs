//! The containers a chunk is made of (02 §6.1, §6.2).
//!
//! | Type | Holds |
//! |------|-------|
//! | [`Paletted`] | one value for every cell of a chunk, packed through a palette |
//! | [`SparseCells`] | a value for a few cells of a chunk |
//! | [`CellBits`] | one bit for every cell of a chunk |
//!
//! All three hash, compare and serialise by what they hold, never by how
//! they came to hold it.

mod bitset;
mod paletted;
mod sparse;

pub use bitset::CellBits;
pub use paletted::Paletted;
pub use sparse::SparseCells;
