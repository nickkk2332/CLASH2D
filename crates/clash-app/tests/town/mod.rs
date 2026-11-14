//! What the market town's sessions of the fifth round share
//! (`play_making.rs`, `play_pistol.rs`, `play_disguise.rs`): the game
//! begun in the square, the town's furniture found by name, and reads of
//! what characters hold and believe for the sessions' assertions (they
//! read truth only to check what the player was shown, never to play).

#![allow(dead_code, reason = "each session uses some of these")]

use clash_app::headless::Headless;
use clash_core::EntityId;
use clash_expr::Value;
use clash_sim::Subject;
use clash_world::Location;

use crate::session::*;

/// The market town is the fourth game of the bundled list.
pub const TOWN_LIFE: usize = 3;

/// The marksman is the third of the playable characters.
pub const MARKSMAN: usize = 2;

/// From the title to the town square, past the setup and the
/// introduction.
pub fn begin_in_the_square(run: &mut Headless) {
    new_game(run, TOWN_LIFE, "Ada");
    until_top(run, "ui:setup", 60);
    press(run, "down");
    press(run, "enter");
    until_top(run, "ui:intro", 60);
    press(run, "enter");
    settle(run);
    assert_eq!(top(run), "ui:play");
}

/// The cells within a box round a place, on its storey, whose block or
/// fixture is named by a word, nearest first.
pub fn near(run: &Headless, at: Location, word: &str, reach: i32) -> Vec<Location> {
    let mut found = run
        .game()
        .with_session(|session| {
            let cosmos = session.sim.truth().cosmos();
            let defs = cosmos.defs();
            let mut found = Vec::new();
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let loc = Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, 0));
                    let Ok(cell) = cosmos.cell(loc) else { continue };
                    let named = [
                        defs.state_text(clash_world::Layer::Block, cell.block.raw()),
                        defs.state_text(clash_world::Layer::Fixture, cell.fixture.raw()),
                    ]
                    .iter()
                    .any(|name| name.contains(word));
                    if named {
                        found.push(loc);
                    }
                }
            }
            found
        })
        .unwrap();
    found.sort_by_key(|loc| {
        let delta = loc.cell - at.cell;
        (delta.x.abs().max(delta.y.abs()), loc.cell.y, loc.cell.x)
    });
    found
}

/// The cells within a box round a place, on its storey, whose block or
/// fixture is named by a word, nearest first: [`near`] that panics with
/// what it looked for if there is none.
pub fn nearest(run: &Headless, at: Location, word: &str, reach: i32) -> Location {
    *near(run, at, word, reach)
        .first()
        .unwrap_or_else(|| panic!("no {word} within {reach} cells of {at:?}"))
}

/// The definitions of the things the character carries, held, worn or
/// stowed, at any depth (`core:knife`), with how many of each.
pub fn carried(run: &Headless) -> Vec<(String, i64)> {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let items = sim.defs().try_registry::<clash_sim::item::ItemDef>()?;
            let mut found = Vec::new();
            let mut open = vec![session.player];
            while let Some(holder) = open.pop() {
                for &thing in sim.truth().contents(holder) {
                    open.push(thing);
                    let Some(item) = sim.components().get::<clash_sim::item::Item>(thing) else {
                        continue;
                    };
                    if let Some(def) = item.def() {
                        found.push((items.name(def).to_owned(), i64::from(item.count())));
                    }
                }
            }
            found.sort();
            Some(found)
        })
        .flatten()
        .unwrap_or_default()
}

/// How many of a definition the character carries.
pub fn carries(run: &Headless, def: &str) -> i64 {
    carried(run)
        .iter()
        .filter(|(name, _)| name == def)
        .map(|(_, count)| count)
        .sum()
}

/// A property of an entity read from truth for the session's
/// assertions, with its arguments.
pub fn property(run: &Headless, who: EntityId, name: &str, args: &[Value]) -> Value {
    run.game()
        .with_session(|session| session.sim.property_named(Subject::Entity(who), name, args))
        .unwrap_or(Value::Null)
}

/// What a property of an entity reads as text, or empty.
pub fn text_of(run: &Headless, entity: EntityId, name: &str) -> String {
    property(run, entity, name, &[])
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default()
}

/// A property of a character's mind about another (`hostile`,
/// `perceives`, `persona_of`), as the value it reads.
pub fn about(run: &Headless, who: EntityId, name: &str, of: EntityId) -> Value {
    run.game()
        .with_session(|session| {
            let of = session.sim.exprs().kinds().handle(Subject::Entity(of))?;
            Some(
                session
                    .sim
                    .property_named(Subject::Entity(who), name, &[Value::from(of)]),
            )
        })
        .flatten()
        .unwrap_or(Value::Null)
}

/// Whether a property of a character's mind about another reads true.
pub fn holds(run: &Headless, who: EntityId, name: &str, of: EntityId) -> bool {
    about(run, who, name, of).as_bool() == Some(true)
}

/// Everyone in the world whose mind is named by a word, by id.
pub fn minds_named(run: &Headless, word: &str) -> Vec<EntityId> {
    let all: Vec<EntityId> = run
        .game()
        .with_session(|session| session.sim.truth().entities().ids().collect())
        .unwrap_or_default();
    all.into_iter()
        .filter(|entity| text_of(run, *entity, "mind_def").contains(word))
        .collect()
}

/// The entity the character has selected as its target.
pub fn target(run: &Headless) -> Option<EntityId> {
    run.game()
        .scene()
        .target()
        .map(|sprite| EntityId::from_bits(sprite.raw()))
}

/// Answers a blow coming as a player would: a dodge, a step back or a
/// step aside where one is in time.
pub fn answer(run: &mut Headless) {
    if top(run) == "ui:reaction" {
        react(run, &["Dodge", "Step back", "Step aside", "Parry", "Block"]);
    }
}

/// Presses keys until the character is free to act again, answering
/// any blow, at most `passes` passes.
pub fn until_free(run: &mut Headless, passes: u32) {
    for _ in 0..passes {
        answer(run);
        let free = run
            .game()
            .with_session(|session| session.sim.decisions().has_free(session.player))
            .unwrap_or(true);
        if free && top(run) == "ui:play" {
            return;
        }
        run.pass();
    }
}

/// From play: `e`, and the first thing near whose row begins with `name`
/// and says no reason it cannot be done; then until the character is
/// free again. Returns whether there was one.
pub fn use_and_wait(run: &mut Headless, name: &str) -> bool {
    use_offer(run, name)
}

/// The rows of the `e` list, as shown.
pub fn near_rows(run: &mut Headless) -> Vec<String> {
    press(run, "e");
    let rows = popup_rows(run, "Use what is near");
    press(run, "escape");
    rows
}

/// From play: the make list (Shift+M), and Enter on the recipe whose row
/// begins with `name`, if it is offered with no reason against it; then
/// until the work is done or stops. Returns the row as shown.
pub fn make(run: &mut Headless, name: &str) -> String {
    press(run, "shift+m");
    assert_eq!(top(run), "ui:make", "{}", run.text());
    let rows = popup_rows(run, "Make");
    let index = rows
        .iter()
        .position(|row| row.starts_with(name))
        .unwrap_or_else(|| panic!("no {name} in {rows:?}\n{}", run.text()));
    let row = rows[index].clone();
    for _ in 0..index {
        press(run, "down");
    }
    press(run, "enter");
    until_free(run, 900);
    settle(run);
    row
}

/// From play: the inventory, and the button of that id in the popup of
/// the first thing carried whose row begins with `name`; then until the
/// character is free again. Returns whether there was one.
pub fn with_carried(run: &mut Headless, name: &str, button: &str) -> bool {
    let done = do_with_named(run, name, button);
    if done {
        until_free(run, 120);
    }
    done
}

/// What the inventory lists as carried, row by row.
pub fn inventory_rows(run: &mut Headless) -> Vec<String> {
    press(run, "i");
    let rows: Vec<String> = run
        .text()
        .lines()
        .skip_while(|line| !line.contains("Carried"))
        .skip(1)
        .take_while(|line| !line.contains('└'))
        .filter_map(|line| {
            let inner = line.split('│').nth(2)?;
            Some(inner.trim().to_owned())
        })
        .filter(|row| !row.is_empty())
        .collect();
    press(run, "escape");
    rows
}

/// The level of a pool of someone's (`core:toxin`), as a share of its
/// most, read from truth for the sessions' assertions.
pub fn pool_share(run: &Headless, who: EntityId, pool: &str) -> Option<f64> {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let id = sim.stat_defs()?.pool_id(pool)?;
            let stats = sim.components().get::<clash_sim::stats::Stats>(who)?;
            stats.pool_frac(sim.timeline(), id).map(|share| share.get())
        })
        .flatten()
}

/// From play: `e`, and the first thing near that can be done whose name
/// is `name`, wherever it is in the list: what lies loose has a list of
/// its own above, and a long list scrolls. Then until the character is
/// free again. Returns whether there was one.
pub fn use_offer(run: &mut Headless, name: &str) -> bool {
    use_offer_at(run, name, None)
}

/// [`use_offer`], on what stands in a cell if one is given: the bench,
/// not the rack beside it.
pub fn use_offer_at(run: &mut Headless, name: &str, at: Option<Location>) -> bool {
    let started = start_offer_at(run, name, at);
    if started {
        until_free(run, 120);
    }
    started
}

/// [`use_offer_at`] without waiting for it to be done. (The sessions'
/// `use_near` counts what lies loose and what can be done as one list,
/// and presses Enter in the first when something lies loose: a fault of
/// that helper, `docs/playtest-beta.md`, the fifth round.)
pub fn start_offer_at(run: &mut Headless, name: &str, at: Option<Location>) -> bool {
    let index = run
        .game()
        .with_session(|session| {
            let sim = &session.sim;
            let mut index = 0;
            for (target, offers) in clash_sim::interaction::offers_near(sim, session.player) {
                let place = match target {
                    clash_sim::interaction::Target::Element(loc, _) => Some(loc),
                    clash_sim::interaction::Target::Entity(id) => sim.truth().location(id),
                };
                for offer in offers {
                    let there = at.is_none_or(|at| place.is_some_and(|loc| loc.cell == at.cell));
                    if offer.name == name && offer.available() && there {
                        return Some(index);
                    }
                    index += 1;
                }
            }
            None
        })
        .flatten();
    let Some(index) = index else {
        return false;
    };
    press(run, "e");
    assert_eq!(top(run), "ui:interact", "{}", run.text());
    let loose = popup_rows(run, "Use what is near")
        .iter()
        .take_while(|row| row.starts_with("Pick up "))
        .count();
    if loose > 0 {
        press(run, "tab");
    }
    for _ in 0..index {
        press(run, "down");
    }
    press(run, "enter");
    true
}

/// [`near`], on every storey from one below to three above, a storey
/// counting as four cells.
pub fn near_any_storey(run: &Headless, at: Location, word: &str, reach: i32) -> Vec<Location> {
    let mut found = Vec::new();
    for dz in -1..=3 {
        let level = Location::new(at.grid, at.cell + clash_core::IVec3::new(0, 0, dz));
        found.extend(near(run, level, word, reach));
    }
    found.sort_by_key(|loc| {
        let delta = loc.cell - at.cell;
        delta.x.abs().max(delta.y.abs()) + delta.z.abs() * 4
    });
    found
}

/// Whether the character is caught: held by someone, or down.
pub fn caught(run: &Headless) -> bool {
    let ada = player(run);
    !property(run, ada, "gripped_by", &[]).is_null()
        || property(run, ada, "helpless", &[]).as_bool() == Some(true)
}

/// Whether the character is inside something (a wardrobe), out of the
/// world.
pub fn hidden(run: &Headless) -> bool {
    run.game()
        .with_session(|session| {
            let record = session.sim.truth().entities().get(session.player)?;
            Some(matches!(record.place(), clash_sim::Place::In(_)))
        })
        .flatten()
        .unwrap_or(false)
}
