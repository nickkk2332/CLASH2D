//! Play sessions in the market town (town life; 09 §3, 15 §9, §11),
//! from the title. A quarrel in the square: a word with someone, who
//! answers a stranger; a blow; and a wardrobe upstairs in a house by
//! the square to hide in. (Struck, the watchman of the square walked on,
//! and nobody came looking while she hid: `docs/playtest-beta.md`, the
//! second round.) And the guardhouse's armoury: its key from the
//! sergeant's desk, the locked door opened with it by walking into it,
//! and a weapon taken from a rack.
//!
//! Waiting in the populated town is slow until performance 2 lands:
//! these sessions are kept short.

mod session;

use clash_app::headless::Headless;
use clash_core::EntityId;
use clash_expr::Value;
use clash_world::Location;
use session::*;

/// The market town is the fourth game of the bundled list.
const TOWN_LIFE: usize = 3;

/// From the title to the town square, past the setup and the
/// introduction.
fn begin_in_the_square(run: &mut Headless) {
    new_game(run, TOWN_LIFE, "Ada");
    until_top(run, "ui:setup", 60);
    press(run, "down");
    press(run, "enter");
    until_top(run, "ui:intro", 60);
    press(run, "enter");
    settle(run);
    assert_eq!(top(run), "ui:play");
}

/// The cells within a box round a place, on every storey from one below
/// to three above, whose block or fixture is named by a word, nearest
/// first.
fn near(run: &Headless, at: Location, word: &str, reach: i32) -> Vec<Location> {
    let mut found = run
        .game()
        .with_session(|session| {
            let cosmos = session.sim.truth().cosmos();
            let defs = cosmos.defs();
            let mut found = Vec::new();
            for dz in -1..=3 {
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let loc =
                            Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, dz));
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
            }
            found
        })
        .unwrap();
    found.sort_by_key(|loc| {
        let delta = loc.cell - at.cell;
        delta.x.abs().max(delta.y.abs()) + delta.z.abs() * 4
    });
    found
}

/// Answers a blow coming as a player would.
fn answer(run: &mut Headless) {
    if top(run) == "ui:reaction" {
        react(run, &["Step back", "Dodge", "Step aside", "Block"]);
    }
}

/// Says the first thing the talk screen offers to the target.
fn greet(run: &mut Headless) -> Vec<String> {
    press(run, "shift+t");
    assert_eq!(top(run), "ui:talk", "{}", run.text());
    let before = log_len(run);
    press(run, "enter");
    for _ in 0..3 {
        if top(run) == "ui:talk" {
            press(run, "escape");
        }
    }
    for _ in 0..4 {
        answer(run);
        press(run, ".");
    }
    log_since(run, before)
}

#[test]
fn a_word_a_blow_the_watch_and_a_wardrobe() {
    let data = DataDir::new("market-quarrel");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);

    // Someone in the square: up to them, and a greeting. Nobody here
    // knows the newcomer: they are greeted back as a stranger.
    assert!(tab_until_someone(&mut run, 30), "nobody in the square");
    assert!(walk_up_to_target(&mut run, 40), "{}", run.text());
    let said = greet(&mut run);
    assert!(
        said.iter().any(|line| line.starts_with("Ada speaks to")),
        "{said:?}"
    );
    assert!(said.iter().any(|line| line.contains(" says: ")), "{said:?}");

    // Struck, while they stay in reach.
    let before = log_len(&run);
    for _ in 0..6 {
        answer(&mut run);
        press(&mut run, "f");
        if !["ui:play", "ui:reaction"].contains(&top(&run).as_str()) {
            press(&mut run, "escape");
        }
        press(&mut run, ".");
    }
    let struck = log_since(&run, before);
    assert!(
        struck.iter().any(|line| line.starts_with("Ada strikes")),
        "{struck:?}"
    );

    // Half a minute, then upstairs to the nearest wardrobe, and in.
    for _ in 0..30 {
        answer(&mut run);
        press(&mut run, ".");
    }
    let at = player_loc(&run).unwrap();
    let wardrobe = *near(&run, at, "wardrobe", 30)
        .first()
        .expect("a wardrobe in a house by the square");
    walk_to(&mut run, wardrobe, 1, 120);
    answer(&mut run);
    assert!(use_near(&mut run, "Hide"), "{}", run.text());
    until_asked(&mut run, 60);
    let hidden = run
        .game()
        .with_session(|session| {
            let record = session.sim.truth().entities().get(session.player)?;
            Some(matches!(record.place(), clash_sim::Place::In(_)))
        })
        .flatten();
    assert_eq!(hidden, Some(true), "inside the wardrobe: {}", run.text());
    // Three minutes in the wardrobe: nobody came looking.
    for _ in 0..3 {
        answer(&mut run);
        press(&mut run, "p");
    }
    assert_eq!(top(&run), "ui:play");
}

/// What a property of an entity reads as text, or empty (the session's
/// assertions read truth).
fn text_of(run: &Headless, entity: EntityId, property: &str) -> String {
    run.game()
        .with_session(|session| {
            session
                .sim
                .property_named(clash_sim::Subject::Entity(entity), property, &[])
        })
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Everyone in the world whose mind is named by a word, by id.
fn minds_named(run: &Headless, word: &str) -> Vec<EntityId> {
    let all: Vec<EntityId> = run
        .game()
        .with_session(|session| session.sim.truth().entities().ids().collect())
        .unwrap_or_default();
    all.into_iter()
        .filter(|entity| text_of(run, *entity, "mind_def").contains(word))
        .collect()
}

/// Whether a character's mind holds another hostile.
fn holds_hostile(run: &Headless, who: EntityId, of: EntityId) -> bool {
    about(run, who, "hostile", of)
}

/// A property of a character's mind about another (`hostile`,
/// `perceives`), false where it reads anything but true.
fn about(run: &Headless, who: EntityId, property: &str, of: EntityId) -> bool {
    run.game()
        .with_session(|session| {
            let of = session
                .sim
                .exprs()
                .kinds()
                .handle(clash_sim::Subject::Entity(of))?;
            session
                .sim
                .property_named(
                    clash_sim::Subject::Entity(who),
                    property,
                    &[Value::from(of)],
                )
                .as_bool()
        })
        .flatten()
        .unwrap_or(false)
}

/// The market fight: someone in the square struck until a blow lands,
/// and the watch, which holds hostile whoever it sees hurt someone, comes
/// for the one who struck. It does not (the second round's known issue
/// 2): a watchman struck by a rowdy, or by the same character delegated,
/// answers in a test script, but struck by the player's character he
/// walks on. Kept, ignored, until the AI stream finds why.
#[test]
#[ignore = "known issue: struck by the player, the watch does not answer (docs/playtest-beta.md, the second round, issue 2)"]
fn a_fight_in_the_square_and_the_watch_answers() {
    let data = DataDir::new("market-fight");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);
    let ada = player(&run);

    // Someone in the square who is not of the watch.
    let mut victim = None;
    for _ in 0..40 {
        press(&mut run, "tab");
        if let Some(sprite) = run.game().scene().target() {
            let entity = EntityId::from_bits(sprite.raw());
            let mind = text_of(&run, entity, "mind_def");
            if !mind.is_empty() && !mind.contains("watch") {
                victim = Some(entity);
                break;
            }
        }
        press(&mut run, ".");
    }
    let victim = victim.expect("someone in the square");
    eprintln!("struck: {}", text_of(&run, victim, "mind_def"));
    assert!(walk_up_to_target(&mut run, 40), "{}", run.text());

    // Blows until one lands, or a minute's worth of keys.
    let before = log_len(&run);
    let mut hurt = false;
    for _ in 0..60 {
        answer(&mut run);
        if !walk_up_to_target(&mut run, 6) {
            break;
        }
        press(&mut run, "f");
        if !["ui:play", "ui:reaction"].contains(&top(&run).as_str()) {
            press(&mut run, "escape");
        }
        press(&mut run, ".");
        hurt = log_since(&run, before)
            .iter()
            .any(|line| line.contains("is wounded") || line.contains("staggers"));
        if hurt {
            break;
        }
    }
    let fight = log_since(&run, before);
    assert!(hurt, "no blow landed: {fight:?}");
    if let Some(sprite) = run.game().scene().target() {
        let struck = EntityId::from_bits(sprite.raw());
        eprintln!(
            "struck {}: perceives her {}, hostile {}, goal {:?}",
            text_of(&run, struck, "mind_def"),
            about(&run, struck, "perceives", ada),
            about(&run, struck, "hostile", ada),
            text_of(&run, struck, "goal"),
        );
    }

    // Then the character stands and waits: the watch is told, or sees,
    // and comes for the one who struck.
    let watch = minds_named(&run, "watch");
    assert!(!watch.is_empty(), "the town has a watch");
    let mut answered = false;
    for _ in 0..180 {
        answer(&mut run);
        press(&mut run, ".");
        answered = watch.iter().any(|guard| holds_hostile(&run, *guard, ada));
        if answered || top(&run) != "ui:play" {
            break;
        }
    }
    let after = log_since(&run, before);
    for guard in &watch {
        eprintln!(
            "watchman {guard}: perceives her {}, goal {:?}",
            about(&run, *guard, "perceives", ada),
            text_of(&run, *guard, "goal")
        );
    }
    assert!(answered, "the watch never came: {after:?}");
}

/// How many of the town's people a routine has doing something now.
fn doing(run: &Headless, people: &[EntityId], what: &str) -> usize {
    people
        .iter()
        .filter(|who| text_of(run, **who, "routine_doing") == what)
        .count()
}

/// A day of routines, its morning: from nine to noon in the square,
/// waited an hour at a time. Three hours among the town's hot minds
/// take too long in a debug build for the suite (the second round,
/// responsiveness); run it with `--ignored`.
#[test]
#[ignore = "slow: three hours waited in the populated square (docs/playtest-beta.md, the second round)"]
fn a_morning_and_a_noon_in_the_square() {
    let data = DataDir::new("market-day");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);
    let people: Vec<EntityId> = run
        .game()
        .with_session(|session| session.sim.truth().entities().ids().collect())
        .unwrap_or_default();
    let people: Vec<EntityId> = people
        .into_iter()
        .filter(|who| !text_of(&run, *who, "routine").is_empty())
        .collect();
    assert!(people.len() > 40, "{} keep routines", people.len());

    // Nine in the morning: the stalls are kept.
    let at_nine = time_of_day(&run);
    assert!((32_400.0..33_000.0).contains(&at_nine), "{at_nine}");
    assert!(doing(&run, &people, "keeping_stall") > 0);
    let seen = |run: &Headless| {
        run.game()
            .scene()
            .world()
            .map(|world| world.model().sprites().count())
            .unwrap_or(0)
    };
    let at_first = seen(&run);

    // Three hours waited an hour at a time, as a player would, till
    // noon: the market fills, and the character sees people come and go.
    let started = std::time::Instant::now();
    for _ in 0..3 {
        press(&mut run, "shift+p");
        until_asked(&mut run, 600);
        answer(&mut run);
    }
    let waited = started.elapsed();
    let at_noon = time_of_day(&run);
    eprintln!(
        "three hours in the square took {:.1} s; {} then {} seen; at noon {} at market, {} keeping stalls, {} asleep",
        waited.as_secs_f64(),
        at_first,
        seen(&run),
        doing(&run, &people, "at_market"),
        doing(&run, &people, "keeping_stall"),
        doing(&run, &people, "asleep"),
    );
    assert!(at_noon >= 43_000.0, "waited to {at_noon}");
    assert!(doing(&run, &people, "at_market") > 0);
    assert!(doing(&run, &people, "keeping_stall") > 0);
    assert_eq!(doing(&run, &people, "asleep"), 0);
    assert_eq!(top(&run), "ui:play");
}

/// The state name of the block in a cell (`locked`, `closed`, `open`).
fn block_at(run: &Headless, at: Location) -> String {
    run.game()
        .with_session(|session| {
            session.sim.property_named(
                clash_sim::Subject::Element(at, clash_world::Layer::Block),
                "state_name",
                &[],
            )
        })
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn the_armoury_opened_with_its_key_and_a_weapon_taken() {
    let data = DataDir::new("market-armoury");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);

    // To the guardhouse, and to the key on the sergeant's desk.
    let guardhouse = site_named(&run, "Guardhouse");
    walk_to(&mut run, guardhouse, 1, 200);
    let at = player_loc(&run).unwrap();
    let key = lying_near(&run, at, "core:key", 14).expect("the key lies in the guardhouse");
    walk_to(&mut run, key, 1, 40);
    press(&mut run, "g");
    if top(&run) == "ui:pick_list" {
        press(&mut run, "enter");
    }
    until_asked(&mut run, 60);
    assert!(run.text().contains("Holding a key"), "{}", run.text());

    // The armoury's door, locked: walking into it unlocks it with the
    // key in hand, and again opens it.
    let at = player_loc(&run).unwrap();
    let door = (-10..=10)
        .flat_map(|dy| (-10..=10).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, 0)))
        .find(|cell| block_at(&run, *cell) == "locked")
        .expect("the armoury's door is locked");
    walk_to(&mut run, door, 1, 30);
    let step = |run: &mut Headless| {
        let at = player_loc(run).unwrap();
        let delta = door.cell - at.cell;
        let key = match (delta.x.signum(), delta.y.signum()) {
            (0, -1) => "k",
            (0, 1) => "j",
            (-1, 0) => "h",
            (1, 0) => "l",
            (1, -1) => "u",
            (-1, -1) => "y",
            (1, 1) => "n",
            _ => "b",
        };
        press(run, key);
        until_asked(run, 60);
        settle(run);
    };
    step(&mut run);
    assert_eq!(
        block_at(&run, door),
        "closed",
        "unlocked: {:?}",
        log_text(&run)
    );
    step(&mut run);
    assert_eq!(block_at(&run, door), "open", "{:?}", log_text(&run));

    // In, and a weapon taken from the rack.
    walk_to(&mut run, door, 0, 10);
    let at = player_loc(&run).unwrap();
    let racks: Vec<Location> = (-4..=4)
        .flat_map(|dy| (-4..=4).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, 0)))
        .filter(|cell| {
            run.game()
                .with_session(|session| {
                    let cosmos = session.sim.truth().cosmos();
                    let fixture = cosmos.cell(*cell).ok()?.fixture;
                    Some(
                        cosmos
                            .defs()
                            .state_text(clash_world::Layer::Fixture, fixture.raw())
                            .contains("rack"),
                    )
                })
                .flatten()
                .unwrap_or(false)
        })
        .collect();
    let rack = *racks.first().expect("a rack in the armoury");
    walk_to(&mut run, rack, 1, 12);
    assert!(use_near(&mut run, "Take out"), "{}", run.text());
    until_asked(&mut run, 60);
    let held = run.text();
    assert!(
        ["club", "buckler", "bow", "knife"]
            .iter()
            .any(|weapon| held.contains(weapon)),
        "{held}"
    );
}
