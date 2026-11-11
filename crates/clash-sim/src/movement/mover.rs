//! [`Mover`]: the component that holds what movement keeps about an
//! entity (06 §11): the ways of moving it has, the posture it is in and
//! the path it is following.
//!
//! It also keeps where an entity the world carries as a body set off
//! from, until it lands.
//!
//! An entity with no `Mover` is a plain mover: it moves by the
//! definitions flagged `default`, is in no posture and follows no path.
//! The component is made the first time something is said about one of
//! those.

use clash_core::{Real, impl_state_hash};
use clash_world::{CellPos, Location};
use serde::{Deserialize, Serialize};

use super::defs::{MovementId, PostureId};

/// A path being followed (the `go_to` action): where it leads, the
/// cells still to enter, and what the mover has found out by trying.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Journey {
    /// Where the mover means to get to.
    pub(super) to: Location,
    /// The cells of the path after the one the mover was in when it was
    /// planned, in order.
    pub(super) cells: Vec<CellPos>,
    /// How many of them the mover has entered.
    pub(super) at: u32,
    /// Cells a step into which was refused on this journey. The mover
    /// has learned by trying that it cannot enter them, and plans round
    /// them.
    pub(super) avoid: Vec<CellPos>,
    /// Whether the path can no longer be followed as it is: a step was
    /// refused, or the mover is not where the path left it.
    pub(super) stale: bool,
}

impl_state_hash!(Journey {
    to,
    cells,
    at,
    avoid,
    stale
});

impl Journey {
    /// Where the journey leads.
    pub fn to(&self) -> Location {
        self.to
    }

    /// The cells still to enter, in order.
    pub fn remaining(&self) -> &[CellPos] {
        self.cells.get(self.at as usize..).unwrap_or(&[])
    }

    /// The next cell to enter, if any is left.
    pub fn next(&self) -> Option<CellPos> {
        self.remaining().first().copied()
    }

    /// The cells the mover has learned it cannot enter.
    pub fn avoided(&self) -> &[CellPos] {
        &self.avoid
    }

    /// Whether the path must be planned again before it is followed.
    pub fn is_stale(&self) -> bool {
        self.stale
    }
}

/// Where an entity the world carries as a body set off from (02 §15),
/// kept from `body_launched` until `body_landed` so that the fall can
/// be measured.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Flight {
    /// The cell it set off from.
    pub(super) from: Location,
    /// Its height there, in metres in its grid.
    pub(super) z_m: Real,
}

impl_state_hash!(Flight { from, z_m });

/// What movement keeps about an entity (06 §11).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Mover {
    /// The ways of moving the entity has, in the order they are tried.
    /// Empty: the definitions flagged `default`.
    pub(super) modes: Vec<MovementId>,
    /// The posture it is in. `None`: no posture, which changes nothing.
    pub(super) posture: Option<PostureId>,
    /// The factor its posture has put on the height of a `Physical`
    /// that no body supplies.
    pub(super) height_factor: Real,
    /// The path it is following.
    pub(super) journey: Option<Journey>,
    /// Where it set off from, while the world carries it as a body.
    pub(super) flight: Option<Flight>,
}

impl_state_hash!(Mover {
    modes,
    posture,
    height_factor,
    journey,
    flight
});

impl Default for Mover {
    fn default() -> Mover {
        Mover {
            modes: Vec::new(),
            posture: None,
            height_factor: Real::ONE,
            journey: None,
            flight: None,
        }
    }
}

impl Mover {
    /// The ways of moving the entity has of its own, in the order they
    /// are tried. Empty for one that moves by the defaults.
    pub fn modes(&self) -> &[MovementId] {
        &self.modes
    }

    /// The posture the entity is in.
    pub fn posture(&self) -> Option<PostureId> {
        self.posture
    }

    /// The path the entity is following.
    pub fn journey(&self) -> Option<&Journey> {
        self.journey.as_ref()
    }
}
