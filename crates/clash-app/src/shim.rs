//! **A shim, to be deleted.** Walking and waiting without the action
//! runtime (03 §4.4, §6).
//!
//! The simulation in this tree has no `Act` and no `Wait` command: its
//! [`Command`](clash_sim::Command) holds debug commands only. So that a
//! character can be walked round a town before actions exist, this
//! module carries out, by debug commands, the two actions the bundled
//! controls name and the one command:
//!
//! | The interface submits | The shim does |
//! |-----------------------|---------------|
//! | `act step { dir }` | Checks the step against the world as a walker would make it (`Cosmos::neighbours`, [`MoveProfile::walker`]): walls, closed doors, drops and corners stop it. Then moves the character with `DebugCommand::MoveEntity` and owes the simulation the time the step takes at [`WALK_SPEED_M_PER_S`]. |
//! | `act go_to { to }` | Finds a way over the cells the character has seen, and takes it a step at a time as the frame loop asks for the next ([`next_route_step`]). |
//! | `command wait { dur }` | Owes the simulation `dur` seconds. |
//! | Anything else | Refuses, with the reason. |
//!
//! It also says what the action runtime will say: that a directly
//! controlled character with nothing to do is to be asked what it does
//! next ([`idle_decision`]).
//!
//! Time owed is paid by the frame loop, which alone calls `advance`.
//!
//! # When actions land
//!
//! `Command::Act` and `Command::Wait` exist, the simulation raises the
//! free decision itself, and `step` and `go_to` are actions of a mod.
//! Then [`crate::host`] turns a `UiCommand` straight into a `Command`,
//! and this file, `Session::busy_until` and `Session::route` are
//! deleted. Nothing else in the application knows an action by name.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{Dir8, Dur, Real};
use clash_expr::Value;
use clash_scene::ui::CommandKind;
use clash_scene::{DecisionAsk, PendingDecision, UiCommand};
use clash_sim::{DebugCommand, Place, Subject};
use clash_world::{
    CellGeometry, CellPos, Location, MoveProfile, MoveRules, PathLimits, Step, StepKind,
};

use crate::session::{Session, player_value};

/// The bare name of the action that takes one step.
pub const ACTION_STEP: &str = "step";
/// The bare name of the action that walks to a cell.
pub const ACTION_GO_TO: &str = "go_to";
/// The command that lets time pass.
pub const COMMAND_WAIT: &str = "wait";

/// How fast the shim's walker walks, in metres a second: an ordinary
/// walking pace. The real figure comes from the body (06 §11).
pub const WALK_SPEED_M_PER_S: Real = Real::of(1.4);

/// The longest a single `wait` may be, in seconds: a day.
pub const MAX_WAIT_S: i64 = 86_400;

/// The ten directions a step is written with, and the step each is.
const DIRECTIONS: [(&str, StepKind); 10] = [
    ("north", StepKind::Level(Dir8::North)),
    ("north_east", StepKind::Level(Dir8::NorthEast)),
    ("east", StepKind::Level(Dir8::East)),
    ("south_east", StepKind::Level(Dir8::SouthEast)),
    ("south", StepKind::Level(Dir8::South)),
    ("south_west", StepKind::Level(Dir8::SouthWest)),
    ("west", StepKind::Level(Dir8::West)),
    ("north_west", StepKind::Level(Dir8::NorthWest)),
    ("up", StepKind::Up),
    ("down", StepKind::Down),
];

/// The part of a qualified id after its namespace.
fn bare(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// How a direction reads in a sentence.
fn spoken(dir: &str) -> String {
    dir.replace('_', "-")
}

/// A walker that goes only where the character has seen: `go_to` plans
/// by what the character knows, not by what is there (01 R11).
struct KnownWalker<'a> {
    walker: MoveProfile,
    known: &'a BTreeSet<CellPos>,
}

impl MoveRules for KnownWalker<'_> {
    fn can_occupy(&self, cell: &CellGeometry) -> bool {
        self.known.contains(&cell.cell) && self.walker.can_occupy(cell)
    }

    fn step_cost(&self, step: &Step) -> Option<Real> {
        self.walker.step_cost(step)
    }
}

/// Why a walker cannot make a step from where it stands, in words.
fn why_not(step: Option<&Step>, kind: StepKind) -> &'static str {
    let Some(step) = step else {
        return "there is nothing there";
    };
    match kind {
        StepKind::Up => "there is nothing here to climb",
        StepKind::Down => "there is nothing here to climb down",
        StepKind::Level(_) => {
            if step.to.walk_blocked() {
                if step.to.opens {
                    "something closed is in the way"
                } else {
                    "something is in the way"
                }
            } else if step.to.liquid {
                "it is too deep to walk through"
            } else if step.to.footing == clash_world::Footing::None {
                "there is nothing to stand on"
            } else {
                "there is no room to pass the corner"
            }
        }
    }
}

/// The time owed for a step of a length, at the walker's pace.
fn step_time(length_m: Real) -> Dur {
    Dur::from_secs_real(length_m / WALK_SPEED_M_PER_S)
}

/// Makes the character busy for a while longer, from when it is next
/// free.
fn owe(session: &mut Session, time: Dur) {
    let from = session.busy_until.unwrap_or_else(|| session.sim.now());
    session.busy_until = Some(from + time);
}

/// Takes one step of a kind, if a walker can.
fn step(session: &mut Session, kind: StepKind, dir: &str) -> Result<(), String> {
    let refused = |why: &str| format!("You cannot go {}: {why}.", spoken(dir));
    let from = session
        .player_loc()
        .ok_or("Your character is not in the world.")?;
    let cosmos = session.sim.truth().cosmos();
    let around = cosmos
        .neighbours(from)
        .map_err(|error| refused(&error.to_string()))?;
    let walker = MoveProfile::walker();
    let tried = around.step(kind);
    let allowed = around
        .steps(&walker)
        .find(|(step, _)| step.kind == kind)
        .map(|(step, _)| step);
    let Some(made) = allowed else {
        return Err(refused(why_not(tried.as_ref(), kind)));
    };
    let to = Location::new(from.grid, made.to.cell);
    session
        .debug(DebugCommand::MoveEntity {
            entity: session.player,
            to: Place::at(to),
        })
        .map_err(|reason| refused(&reason))?;
    owe(session, step_time(made.length_m));
    session.stale_view = true;
    Ok(())
}

/// Plans a walk to a cell over what the character has seen.
fn go_to(session: &mut Session, to: &Value) -> Result<(), String> {
    let Some(Subject::Loc(goal)) = session.sim.subject_of(to) else {
        return Err("`go_to` needs a place to go to.".to_owned());
    };
    let from = session
        .player_loc()
        .ok_or("Your character is not in the world.")?;
    if !session.known.contains(&goal.cell) {
        return Err("You have not seen that place.".to_owned());
    }
    let rules = KnownWalker {
        walker: MoveProfile::walker(),
        known: &session.known,
    };
    let path = session
        .sim
        .truth()
        .cosmos()
        .find_path(from, goal, &rules, PathLimits::DEFAULT)
        .map_err(|_| "You know no way there.".to_owned())?;
    // Last step first, so that the next is popped off the end.
    session.route = path
        .cells()
        .iter()
        .skip(1)
        .rev()
        .map(|cell| Location::new(path.grid(), *cell))
        .collect();
    Ok(())
}

/// Takes the next step of a `go_to` under way, if the character is free
/// to. Returns a line for the log if the walk had to be given up.
pub fn next_route_step(session: &mut Session) -> Option<String> {
    if session.busy_until.is_some() {
        return None;
    }
    let next = session.route.pop()?;
    let from = session.player_loc()?;
    let delta = (
        next.cell.x - from.cell.x,
        next.cell.y - from.cell.y,
        next.cell.z - from.cell.z,
    );
    let found = DIRECTIONS.iter().find(|(_, kind)| {
        let d = kind.delta();
        (d.x, d.y, d.z) == delta
    });
    let result = match found {
        Some((dir, kind)) => step(session, *kind, dir),
        None => Err("The way is no longer there.".to_owned()),
    };
    match result {
        Ok(()) => None,
        Err(reason) => {
            session.route.clear();
            Some(reason)
        }
    }
}

/// Carries out a command of the interface, or says why not. See the
/// [module](self) documentation.
pub fn submit(session: &mut Session, command: &UiCommand) -> Result<(), String> {
    // Whatever the player asks for next replaces a walk under way.
    session.route.clear();
    let arg = |name: &str| command.args.get(name);
    match (command.kind, bare(&command.name)) {
        (CommandKind::Act, ACTION_STEP) => {
            if session.busy_until.is_some() {
                return Err("You are still moving.".to_owned());
            }
            let dir = arg("dir").and_then(Value::as_str).unwrap_or("");
            let Some((_, kind)) = DIRECTIONS.iter().find(|(name, _)| *name == dir) else {
                return Err(format!("`{dir}` is not a direction to step in."));
            };
            step(session, *kind, dir)
        }
        (CommandKind::Act, ACTION_GO_TO) => match arg("to") {
            Some(to) => go_to(session, to),
            None => Err("`go_to` needs a place to go to.".to_owned()),
        },
        (CommandKind::Command, COMMAND_WAIT) => {
            let seconds = arg("dur")
                .and_then(Value::as_number)
                .unwrap_or(Real::ONE)
                .clamp(Real::ZERO, Real::from_i64(MAX_WAIT_S));
            owe(session, Dur::from_secs_real(seconds));
            Ok(())
        }
        (CommandKind::Act, _) => Err(format!(
            "Nothing can do `{}` yet: the simulation has no actions, and only `{ACTION_STEP}` and `{ACTION_GO_TO}` are stood in for.",
            command.name
        )),
        (CommandKind::Command, _) => Err(format!(
            "The simulation has no command `{}` yet.",
            command.name
        )),
        (CommandKind::Rule, _) => Err(format!(
            "The rule `{}` cannot be run: the simulation has no `Ui` command yet.",
            command.name
        )),
    }
}

/// What the action runtime will raise: a directly controlled character
/// with nothing to do is asked what it does next (03 §5.2). The
/// simulation raises no such decision yet, so the shim says it for the
/// player's character whenever it is in the world and owes no time.
pub fn idle_decision(session: &Session) -> Option<PendingDecision> {
    if session.busy_until.is_some() || session.player_loc().is_none() {
        return None;
    }
    Some(PendingDecision {
        entity: player_value(session),
        kind: DecisionAsk::Free,
    })
}

/// The arguments of a `step`, for tests and for the headless runner.
pub fn step_args(dir: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([("dir".to_owned(), Value::text(dir))])
}
