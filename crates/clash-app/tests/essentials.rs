//! What a player meets in the first minute, headless from the title
//! (decisions D-860 to D-866): a character made, chosen or given as
//! each bundled game says (13 §4.2), a gifted one made with the powers
//! chosen and one of them used; things picked up, worn, stowed, taken
//! out, given and dropped through the inventory's actions; the key of
//! the guardhouse's armoury taken from the sergeant's desk and the
//! armoury opened with it; a made character saved and loaded unchanged;
//! and no internal name in what the screens and the log show.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod common;

use std::path::Path;

use clash_app::boot;
use clash_app::game::{BootOptions, Game};
use clash_app::headless::Headless;
use clash_core::{EntityId, IVec3};
use clash_expr::Value;
use clash_scene::assert_golden;
use clash_sim::component::Identity;
use clash_sim::item::{Item, ItemDef, hold};
use clash_sim::template::TemplateDef;
use clash_sim::{Command, DebugCommand, Place};
use clash_world::Location;
use common::*;

/// The bundled games in the new-game list, in order of qualified id.
const LAST_STAND: usize = 0;
const ERRAND: usize = 1;
const TOWN: usize = 2;
const TOWN_LIFE: usize = 3;
const WAYSIDE: usize = 4;

/// The playable templates as the creation screen lists them, after the
/// adept.
const BRAWLER: usize = 1;
const MARKSMAN: usize = 2;

/// Presses keys until the screen on top has a widget of that id in
/// focus.
fn focus_on(run: &mut Headless, id: &str) {
    for _ in 0..24 {
        let focus = run
            .game()
            .scene()
            .top()
            .and_then(|screen| screen.focus().map(str::to_owned));
        if focus.as_deref() == Some(id) {
            return;
        }
        script(run, "tab\n");
    }
    panic!(
        "`{id}` never came into focus on {}:\n{}",
        top(run),
        run.text()
    );
}

/// Presses Down until the widget in focus is the one of that id: a
/// button of a column of them.
fn down_to(run: &mut Headless, id: &str) {
    for _ in 0..16 {
        let focus = run
            .game()
            .scene()
            .top()
            .and_then(|screen| screen.focus().map(str::to_owned));
        if focus.as_deref() == Some(id) {
            return;
        }
        script(run, "down\n");
    }
    panic!("`{id}` was never reached on {}:\n{}", top(run), run.text());
}

/// From the title, the new-game screen with the name typed and the game
/// at `index` selected in the list.
fn new_game_at(run: &mut Headless, index: usize, name: &str) {
    script(run, "enter\n");
    assert_eq!(top(run), "ui:new_game");
    script(run, &format!("type {name}\ntab\ntab\n"));
    for _ in 0..index {
        script(run, "down\n");
    }
}

/// The qualified ids of `app.templates`.
fn offered(run: &Headless) -> Vec<String> {
    run.game()
        .scene()
        .app()
        .templates()
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

/// An entry of `app.creation`.
fn creation(run: &Headless, field: &str) -> Value {
    run.game()
        .scene()
        .app()
        .creation()
        .as_map()
        .and_then(|map| map.get(field).cloned())
        .unwrap_or_default()
}

/// The templates the player's character was made from.
fn made_from(run: &Headless) -> Vec<String> {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let templates = sim.defs().try_registry::<TemplateDef>().unwrap();
            sim.components()
                .get::<Identity>(session.player)
                .map(|identity| {
                    identity
                        .templates()
                        .iter()
                        .map(|id| templates.name(*id).to_owned())
                        .collect()
                })
                .unwrap_or_default()
        })
        .expect("a game is being played")
}

/// An attribute of the player's character.
fn attribute(run: &Headless, name: &str) -> f64 {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let id = sim.stat_defs().unwrap().attribute_id(name).unwrap();
            sim.components()
                .stats
                .get(session.player)
                .and_then(|stats| stats.attr(id))
                .map(|value| value.get())
        })
        .flatten()
        .expect("the character has the attribute")
}

/// The bare id of an item's definition.
fn def_of(sim: &clash_sim::Sim, id: EntityId) -> Option<String> {
    let items = sim.defs().try_registry::<ItemDef>()?;
    let item = sim.components().get::<Item>(id)?;
    Some(items.name(item.def()?).rsplit(':').next()?.to_owned())
}

/// What the player carries, each container before what is in it: the
/// thing, how it is held and what holds it (`you`, or a thing).
fn carried(run: &Headless) -> Vec<(String, String, String)> {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            sim.truth()
                .entities()
                .contents_deep(session.player)
                .into_iter()
                .filter_map(|id| {
                    let def = def_of(sim, id)?;
                    let how = hold::hold_of(sim, id).map_or("", |how| how.name());
                    let holder = hold::holder_of(sim, id)
                        .filter(|holder| *holder != session.player)
                        .and_then(|holder| def_of(sim, holder))
                        .unwrap_or_else(|| "you".to_owned());
                    Some((def, how.to_owned(), holder))
                })
                .collect()
        })
        .expect("a game is being played")
}

fn has(carried: &[(String, String, String)], what: &str, how: &str, holder: &str) -> bool {
    carried
        .iter()
        .any(|(def, held, by)| def == what && held == how && by == holder)
}

/// A thing of a definition lying loose within a few cells of a place,
/// if one is there.
fn lying_near(
    run: &Headless,
    at: Location,
    what: &str,
    radius: i32,
) -> Option<(EntityId, Location)> {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            sim.truth()
                .entities()
                .iter()
                .filter(|entity| matches!(entity.place(), Place::At { .. }))
                .filter(|entity| def_of(sim, entity.id()).as_deref() == Some(what))
                .filter_map(|entity| Some((entity.id(), sim.truth().location(entity.id())?)))
                .find(|(_, loc)| {
                    loc.grid == at.grid
                        && (loc.cell.z - at.cell.z).abs() <= radius.min(1)
                        && (loc.cell.x - at.cell.x).abs() <= radius
                        && (loc.cell.y - at.cell.y).abs() <= radius
                })
        })
        .flatten()
}

/// Begins the game at `index`, making the character on the creation
/// screen from the template at `template` with its choices as they
/// are, and Begin. A game with a choice of place begins at its first.
fn begin_made(run: &mut Headless, index: usize, template: usize, name: &str) {
    new_game_at(run, index, name);
    script(run, "f2\n");
    assert_eq!(top(run), "ui:create");
    for _ in 0..template {
        script(run, "down\n");
    }
    script(run, "enter\n");
    finish_made(run);
}

/// Begin, on the creation screen, through setup and the introduction.
fn finish_made(run: &mut Headless) {
    focus_on(run, "begin");
    script(run, "enter\nwait 5\n");
    if top(run) == "ui:setup" {
        script(run, "enter\nwait 5\n");
    }
    assert!(run.game().playing(), "a game began: {}", run.text());
    if top(run) == "ui:intro" {
        script(run, "escape\n");
    }
    settle(run);
    assert_eq!(top(run), "ui:play");
}

/// Hands the game a command as a tool would, and lets it run.
fn tool_then(run: &mut Headless, command: Command) {
    tool(run, command);
    run.pass();
}

#[test]
fn characters_are_made_chosen_or_given_as_each_game_says() {
    let data = data_dir("ess-kinds");
    let mut run = boot_default(&data);
    let playable = [
        "core:adept",
        "core:brawler",
        "core:marksman",
        "core:ordinary_person",
        "core:power_sampler",
    ];
    script(&mut run, "enter\ntab\ntab\n");
    for (index, offers, editable) in [
        (LAST_STAND, true, false),
        (ERRAND, true, false),
        (TOWN, true, true),
        (TOWN_LIFE, true, true),
        (WAYSIDE, false, true),
    ] {
        script(&mut run, "f2\n");
        assert_eq!(top(&run), "ui:create");
        let listed = run.text();
        if offers {
            assert_eq!(offered(&run), playable, "game {index}");
            for name in [
                "Adept",
                "Brawler",
                "Marksman",
                "Ordinary person",
                "Gifted person",
            ] {
                assert!(listed.contains(name), "{name}: {listed}");
            }
            // Begun: made, or ready-made with a name and nothing more.
            script(&mut run, "enter\n");
            assert_eq!(
                creation(&run, "editable"),
                Value::Bool(editable),
                "game {index}"
            );
            assert_eq!(creation(&run, "template"), Value::text("core:adept"));
            if editable {
                assert!(run.text().contains("Choices"), "{}", run.text());
            } else {
                assert!(run.text().contains("Ready-made"), "{}", run.text());
            }
        } else {
            assert!(offered(&run).is_empty());
            assert!(listed.contains("you are given who you are"), "{listed}");
        }
        script(&mut run, "escape\ndown\n");
        assert_eq!(top(&run), "ui:new_game");
    }
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_creation_screen_lists_what_a_game_offers() {
    let data = data_dir("ess-create-golden");
    let mut run = boot_default(&data);
    new_game_at(&mut run, TOWN_LIFE, "Ada");
    script(&mut run, "f2\n");
    assert_golden(golden("create_templates"), run.frame());
    script(&mut run, "enter\n");
    assert_golden(golden("create_adept"), run.frame());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_brawler_chosen_for_the_errand_starts_with_a_club_in_hand() {
    let data = data_dir("ess-brawler");
    let mut run = boot_default(&data);
    begin_made(&mut run, ERRAND, BRAWLER, "Bea");
    assert_eq!(made_from(&run), ["core:brawler"]);
    assert_eq!(attribute(&run, "core:strength"), 15.0);
    let things = carried(&run);
    assert!(has(&things, "club", "grasp", "you"), "{things:?}");
    assert!(has(&things, "parcel", "grasp", "you"), "{things:?}");
    // What the status column says the character holds, as it knows it.
    let shown = run.text();
    assert!(
        shown.contains("Holding a parcel and a") && shown.contains("club"),
        "{shown}"
    );
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_lane_gives_an_ordinary_person() {
    let data = data_dir("ess-given");
    let mut run = boot_default(&data);
    begin(&mut run, WAYSIDE, "Cy");
    settle(&mut run);
    assert_eq!(made_from(&run), ["core:person"]);
    assert!(carried(&run).is_empty());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn an_adept_is_made_with_the_powers_chosen_and_uses_one() {
    let data = data_dir("ess-adept");
    let mut run = boot_default(&data);
    new_game_at(&mut run, TOWN_LIFE, "Ada");
    script(&mut run, "f2\n");
    // The adept is the first of the list.
    script(&mut run, "enter\n");
    // The powers, the fifth choice: Heal put back, Force push taken.
    focus_on(&mut run, "choices");
    script(&mut run, "down\ndown\ndown\ndown\nenter\n");
    assert_eq!(top(&run), "ui:create_choice");
    script(
        &mut run,
        "down\ndown\ndown\ndown\nenter\nup\nup\nup\nenter\nescape\n",
    );
    assert_eq!(top(&run), "ui:create");
    let powers = run
        .game()
        .scene()
        .app()
        .creation()
        .as_map()
        .and_then(|map| map.get("choices").cloned())
        .and_then(|choices| {
            choices.as_list().and_then(|list| {
                list.iter()
                    .find(|choice| {
                        choice
                            .as_map()
                            .and_then(|map| map.get("key"))
                            .and_then(Value::as_str)
                            == Some("core:adept.powers")
                    })
                    .and_then(|choice| choice.as_map()?.get("shown").cloned())
            })
        });
    assert_eq!(
        powers,
        Some(Value::text("Dash, Force push, Fire bolt")),
        "{}",
        run.text()
    );
    finish_made(&mut run);
    assert_eq!(made_from(&run), ["core:adept"]);

    // The abilities screen lists what was chosen and nothing else.
    script(&mut run, "a\n");
    assert_eq!(top(&run), "ui:abilities");
    let listed = run.text();
    for power in ["Dash", "Force push", "Fire bolt"] {
        assert!(listed.contains(power), "{power}: {listed}");
    }
    for power in ["Heal", "Time stop", "Shield raised"] {
        assert!(!listed.contains(power), "{power}: {listed}");
    }
    // Dash, used from there: its grant supplies what it needs.
    let row = run
        .game()
        .with_session(|session| {
            clash_sim::offer::ability_uses(&session.sim, session.player, &Value::Null)
                .iter()
                .position(|use_of| {
                    use_of
                        .as_map()
                        .and_then(|map| map.get("label"))
                        .and_then(Value::as_str)
                        == Some("Dash")
                })
        })
        .flatten()
        .expect("Dash is listed");
    for _ in 0..row {
        script(&mut run, "down\n");
    }
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:play");
    match last_command(&run) {
        Command::UseAbility { ability, .. } => {
            let name = run
                .game()
                .with_session(|session| {
                    session
                        .sim
                        .defs()
                        .try_registry::<clash_sim::ability::AbilityDef>()
                        .map(|abilities| abilities.name(ability).to_owned())
                })
                .flatten();
            assert_eq!(name.as_deref(), Some("core:dash"));
        }
        other => panic!("the last command was {other:?}"),
    }
    script(&mut run, "wait 30\n");
    assert!(
        !log(&run).iter().any(|(category, _)| category == "refusal"),
        "{:?}",
        log(&run)
    );
    let _ = std::fs::remove_dir_all(&data);
}

/// Opens the inventory with the row of the thing of a definition
/// selected, and Enter on it: the item's popup.
fn item_popup(run: &mut Headless, what: &str) {
    let row = carried(run)
        .iter()
        .position(|(def, _, _)| def == what)
        .unwrap_or_else(|| panic!("{what} is carried: {:?}", carried(run)));
    script(run, "i\n");
    assert_eq!(top(run), "ui:inventory");
    for _ in 0..row {
        script(run, "down\n");
    }
    script(run, "enter\n");
    assert_eq!(top(run), "ui:item");
}

/// In the item's popup, the button of that id, and Enter; then time
/// for the action to run.
fn do_item(run: &mut Headless, what: &str, button: &str) {
    item_popup(run, what);
    down_to(run, button);
    script(run, "enter\nwait 60\n");
    settle(run);
    if top(run) == "ui:inventory" {
        script(run, "escape\n");
    }
    assert_eq!(top(run), "ui:play");
}

#[test]
fn things_are_picked_up_worn_stowed_taken_out_given_and_dropped() {
    let data = data_dir("ess-things");
    let mut run = boot_armoury(&data);
    // The gifted, with someone beside them.
    begin(&mut run, 0, "Ada");
    settle(&mut run);
    let (hero, here) = player(&run);
    let east = Location::new(here.grid, here.cell.offset(IVec3::new(0, -1, 0)));
    for (def, place) in [
        ("core:backpack", Place::at(here)),
        ("core:knife", Place::at(east)),
    ] {
        tool_then(
            &mut run,
            Command::Debug(DebugCommand::CreateItem {
                def: def.to_owned(),
                count: 1,
                place,
            }),
        );
    }
    script(&mut run, "wait 2\n");
    settle(&mut run);

    // `e` lists what lies within reach to pick up.
    script(&mut run, "e\n");
    assert_eq!(top(&run), "ui:interact");
    let listed = run.text();
    assert!(listed.contains("Pick up a backpack"), "{listed}");
    assert!(listed.contains("Pick up a knife"), "{listed}");
    script(&mut run, "escape\n");

    // Two things within reach: `g` asks which, the one underfoot first.
    script(&mut run, "g\n");
    assert_eq!(top(&run), "ui:pick_list", "{}", run.text());
    script(&mut run, "enter\nwait 60\n");
    settle(&mut run);
    assert!(
        has(&carried(&run), "backpack", "grasp", "you"),
        "{:?}",
        carried(&run)
    );

    // Worn, from the inventory.
    item_popup(&mut run, "backpack");
    assert_golden(golden("inventory_item"), run.frame());
    script(&mut run, "escape\nescape\n");
    do_item(&mut run, "backpack", "wear");
    assert!(
        has(&carried(&run), "backpack", "worn", "you"),
        "{:?}",
        carried(&run)
    );

    // One thing left within reach: `,` picks it up without asking.
    script(&mut run, ",\nwait 60\n");
    settle(&mut run);
    assert!(
        has(&carried(&run), "knife", "grasp", "you"),
        "{:?} {:?} {}",
        carried(&run),
        log(&run),
        top(&run)
    );

    // A knife is not worn: the action's reason, in the log.
    do_item(&mut run, "knife", "wear");
    assert!(
        log(&run)
            .iter()
            .any(|(_, line)| line == "The knife is not something to wear."),
        "{:?}",
        log(&run)
    );

    // Stowed in the backpack, and taken out again.
    do_item(&mut run, "knife", "stow");
    assert!(
        has(&carried(&run), "knife", "stowed", "backpack"),
        "{:?}",
        carried(&run)
    );
    do_item(&mut run, "knife", "take_out");
    assert!(
        has(&carried(&run), "knife", "grasp", "you"),
        "{:?}",
        carried(&run)
    );

    // Given to the one beside: the entity picker starts on them. They
    // go about as they please, so the character is put beside them
    // again for each try.
    for _ in 0..3 {
        let beside = run
            .game()
            .with_session(|session| {
                let sim = &session.sim;
                let at = session.player_loc()?;
                sim.truth()
                    .entities()
                    .iter()
                    .filter(|entity| entity.id() != hero && def_of(sim, entity.id()).is_none())
                    .filter_map(|entity| sim.truth().location(entity.id()))
                    .filter(|loc| loc.grid == at.grid && loc.cell.z == at.cell.z)
                    .min_by_key(|loc| {
                        (loc.cell.x - at.cell.x).abs() + (loc.cell.y - at.cell.y).abs()
                    })
            })
            .flatten()
            .expect("someone else is in the town");
        if let Some(next) = round(beside)
            .into_iter()
            .find(|cell| standable(&run, *cell))
        {
            put_player(&mut run, next);
        }
        item_popup(&mut run, "knife");
        down_to(&mut run, "give");
        script(&mut run, "enter\n");
        if top(&run) == "ui:pick_entity" {
            script(&mut run, "enter\nwait 60\n");
            settle(&mut run);
        } else {
            // Only heard, behind her: the picker is not opened on her
            // alone, and says why (D-904); another try.
            assert_eq!(
                log(&run).last().map(|(_, line)| line.as_str()),
                Some("Nobody is in sight to choose."),
                "{}",
                run.text()
            );
        }
        // A refusal leaves the popup open; either way, back to play.
        for _ in 0..3 {
            if top(&run) != "ui:play" {
                script(&mut run, "escape\n");
            }
        }
        if !carried(&run).iter().any(|(def, _, _)| def == "knife") {
            break;
        }
    }
    assert!(
        !carried(&run).iter().any(|(def, _, _)| def == "knife"),
        "{:?} {:?}",
        carried(&run),
        log(&run)
    );
    let given = run
        .game()
        .with_session(|session| {
            session
                .sim
                .truth()
                .entities()
                .iter()
                .filter(|entity| entity.id() != hero)
                .any(|entity| {
                    session
                        .sim
                        .truth()
                        .contents(entity.id())
                        .iter()
                        .any(|thing| def_of(&session.sim, *thing).as_deref() == Some("knife"))
                })
        })
        .unwrap_or(false);
    assert!(given, "{:?}", log(&run));

    // Taken off, into a hand, then dropped where the character stands.
    do_item(&mut run, "backpack", "remove");
    assert!(
        has(&carried(&run), "backpack", "grasp", "you"),
        "{:?}",
        carried(&run)
    );
    do_item(&mut run, "backpack", "drop");
    assert!(carried(&run).is_empty(), "{:?}", carried(&run));
    assert!(lying_near(&run, player(&run).1, "backpack", 0).is_some());
    assert!(
        !log(&run).iter().any(|(category, _)| category == "fault"),
        "{:?}",
        log(&run)
    );
    let _ = std::fs::remove_dir_all(&data);
}

/// The fixture `guardhouse`: the market town begun at its guardhouse.
fn boot_guardhouse(data: &Path) -> Headless {
    let fixture = manifest().join("tests/fixtures/guardhouse");
    let search = [fixture.join("mods"), mods_dir()];
    let profile = fixture.join("profiles/guardhouse.json");
    let loaded = boot::load(&search, &profile.to_string_lossy())
        .unwrap_or_else(|problem| panic!("{problem}"));
    let game = Game::boot(
        loaded,
        BootOptions {
            data_dir: Some(data.to_path_buf()),
            ..BootOptions::default()
        },
    )
    .unwrap_or_else(|problem| panic!("{problem}"));
    Headless::new(game)
}

/// The state of the block of a cell, by name.
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

/// Whether a walker can stand in a cell.
fn standable(run: &Headless, at: Location) -> bool {
    run.game()
        .with_session(|session| session.sim.truth().cosmos().standable(at).unwrap_or(false))
        .unwrap_or(false)
}

/// Moves the player's character to a cell, as a tool would.
fn put_player(run: &mut Headless, at: Location) {
    let (hero, _) = player(run);
    tool_then(
        run,
        Command::Debug(DebugCommand::MoveEntity {
            entity: hero,
            to: Place::at(at),
        }),
    );
    // A moment's wait, in which the character looks round.
    script(run, ".\nwait 60\n");
    settle(run);
}

/// The cells round one on its storey, the four sides first.
fn round(at: Location) -> Vec<Location> {
    [
        (0, -1),
        (1, 0),
        (0, 1),
        (-1, 0),
        (1, -1),
        (1, 1),
        (-1, 1),
        (-1, -1),
    ]
    .into_iter()
    .map(|(dx, dy)| Location::new(at.grid, at.cell.offset(IVec3::new(dx, dy, 0))))
    .collect()
}

#[test]
fn the_armoury_key_is_taken_from_the_desk_and_opens_the_armoury() {
    let data = data_dir("ess-armoury");
    let mut run = boot_guardhouse(&data);
    // The fixture's game comes after the five of `core`.
    begin(&mut run, 5, "Ada");
    settle(&mut run);
    let (_, door) = player(&run);
    let keys: Vec<String> = run
        .game()
        .with_session(|session| {
            let sim = &session.sim;
            sim.truth()
                .entities()
                .iter()
                .filter(|entity| def_of(sim, entity.id()).is_some())
                .map(|entity| {
                    format!(
                        "{:?} {:?} {:?}",
                        def_of(sim, entity.id()),
                        entity.place(),
                        sim.truth().location(entity.id())
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let (_, desk) = lying_near(&run, door, "key", 12)
        .unwrap_or_else(|| panic!("the key lies on the sergeant's desk: at {door:?}, {keys:#?}"));
    // Beside the desk, as a tool puts the character there.
    let beside = round(desk)
        .into_iter()
        .find(|cell| standable(&run, *cell))
        .expect("somewhere to stand beside the desk");
    put_player(&mut run, beside);

    // `g` picks the key up: it is the one thing within reach.
    script(&mut run, "g\nwait 60\n");
    settle(&mut run);
    assert!(
        has(&carried(&run), "key", "grasp", "you"),
        "{:?} {:?}",
        carried(&run),
        log(&run)
    );

    // The armoury's door, locked; beside it, walking into it unlocks it
    // with the key in hand, and again opens it.
    let locked = run
        .game()
        .with_session(|session| session.player_loc())
        .flatten()
        .and_then(|at| {
            let mut found = None;
            for dy in -10..=10 {
                for dx in -10..=10 {
                    let cell = Location::new(at.grid, at.cell.offset(IVec3::new(dx, dy, 0)));
                    if found.is_none() && block_at(&run, cell) == "locked" {
                        found = Some(cell);
                    }
                }
            }
            found
        })
        .expect("the armoury's door is locked");
    // Outside is the side of the door nearer the desk; from there a step
    // toward the door walks into it.
    let distance = |at: Location| (at.cell.x - desk.cell.x).abs() + (at.cell.y - desk.cell.y).abs();
    let (outside, way) = [(0, 1, "k"), (-1, 0, "l"), (0, -1, "j"), (1, 0, "h")]
        .into_iter()
        .map(|(dx, dy, key)| {
            (
                Location::new(locked.grid, locked.cell.offset(IVec3::new(dx, dy, 0))),
                key,
            )
        })
        .filter(|(cell, _)| standable(&run, *cell))
        .min_by_key(|(cell, _)| distance(*cell))
        .expect("somewhere to stand by the armoury's door");
    put_player(&mut run, outside);
    script(&mut run, &format!("{way}\nwait 60\n"));
    settle(&mut run);
    assert_eq!(
        block_at(&run, locked),
        "closed",
        "unlocked: {:?}",
        log(&run)
    );
    script(&mut run, &format!("{way}\nwait 60\n"));
    settle(&mut run);
    assert_eq!(block_at(&run, locked), "open", "{:?}", log(&run));

    // In through it, and what the racks hold can be taken.
    script(&mut run, &format!("{way}\n{way}\nwait 60\n"));
    settle(&mut run);
    script(&mut run, "e\n");
    assert_eq!(top(&run), "ui:interact");
    assert!(run.text().contains("Take out"), "{}", run.text());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_made_character_is_saved_and_loaded_unchanged() {
    let data = data_dir("ess-save");
    let mut run = boot_default(&data);
    new_game_at(&mut run, TOWN, "Mo");
    script(&mut run, "f2\n");
    for _ in 0..MARKSMAN {
        script(&mut run, "down\n");
    }
    script(&mut run, "enter\n");
    // The weapon, the fifth choice: the pistol.
    focus_on(&mut run, "choices");
    script(
        &mut run,
        "down\ndown\ndown\ndown\nenter\ndown\nenter\nescape\n",
    );
    finish_made(&mut run);
    assert_eq!(made_from(&run), ["core:marksman"]);
    let things = carried(&run);
    assert!(has(&things, "pistol", "grasp", "you"), "{things:?}");
    assert!(
        has(&things, "pistol_magazine", "fixed", "pistol"),
        "{things:?}"
    );
    assert!(
        has(&things, "pistol_round", "stowed", "pistol_magazine"),
        "{things:?}"
    );
    assert!(has(&things, "backpack", "worn", "you"), "{things:?}");
    assert!(
        has(&things, "pistol_round", "stowed", "backpack"),
        "{things:?}"
    );
    assert_eq!(attribute(&run, "core:wits"), 14.0);

    // Saved, left, and carried on from the title.
    let before = run
        .game()
        .with_session(|session| session.sim.state_hash())
        .unwrap();
    script(&mut run, "escape\ndown\nenter\ntype Marksman\ntab\nenter\n");
    assert_eq!(top(&run), "ui:play");
    script(&mut run, "escape\ndown\ndown\ndown\ndown\nenter\nenter\n");
    assert_eq!(top(&run), "ui:title");
    script(&mut run, "down\nenter\nwait 5\n");
    assert!(run.game().playing());
    let after = run
        .game()
        .with_session(|session| session.sim.state_hash())
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(made_from(&run), ["core:marksman"]);
    assert_eq!(carried(&run), things);
    assert_eq!(attribute(&run, "core:wits"), 14.0);
    let _ = std::fs::remove_dir_all(&data);
}

/// Everything in a text that looks like an internal name rather than
/// words for a player: a backtick, a namespace and a colon before a
/// name (`core:knife`), a word joined by underscores (`l_arm`); and a
/// line of the log that begins with a small letter. `allowed` lists
/// what is genuinely a name.
fn internal_names(text: &str, allowed: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        if line.contains('`') {
            found.push(format!("a backtick in {line:?}"));
        }
        for word in line.split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == ':')) {
            let word = word.trim_matches(':');
            if word.is_empty() || allowed.contains(&word) {
                continue;
            }
            if word.contains('_') && word.chars().any(char::is_alphabetic) {
                found.push(format!("`{word}` in {line:?}"));
            }
            if let Some((space, name)) = word.split_once(':')
                && !space.is_empty()
                && space.chars().all(|ch| ch.is_ascii_lowercase())
                && name
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_lowercase())
            {
                found.push(format!("`{word}` in {line:?}"));
            }
        }
    }
    found
}

#[test]
fn nothing_a_player_reads_is_an_internal_name() {
    let data = data_dir("ess-names");
    let mut run = boot_default(&data);
    // What is genuinely a name, though it looks like an id.
    let allowed: &[&str] = &[];
    let mut problems: Vec<String> = Vec::new();
    let mut look = |run: &Headless, what: &str| {
        for problem in internal_names(&run.text(), allowed) {
            problems.push(format!("{what}: {problem}"));
        }
    };

    look(&run, "the title");
    script(&mut run, "enter\n");
    look(&run, "the new-game screen");
    script(&mut run, "tab\ntab\ndown\ndown\ndown\nf2\n");
    look(&run, "the creation screen's list");
    script(&mut run, "enter\n");
    look(&run, "the creation screen");
    focus_on(&mut run, "choices");
    script(&mut run, "down\ndown\ndown\ndown\nenter\n");
    look(&run, "a choice");
    script(&mut run, "escape\n");
    focus_on(&mut run, "begin");
    script(&mut run, "enter\nwait 5\n");
    assert_eq!(top(&run), "ui:setup");
    look(&run, "the setup screen");
    script(&mut run, "enter\nwait 5\n");
    if top(&run) == "ui:intro" {
        look(&run, "the introduction");
        script(&mut run, "escape\n");
    }
    settle(&mut run);
    look(&run, "the play screen");
    for (keys, what) in [
        ("i", "the inventory"),
        ("c", "the condition"),
        ("a", "the abilities"),
        ("?", "the keys"),
        ("o", "the options"),
        ("e", "what is near"),
        ("shift+j", "the journal"),
        ("m", "the map"),
        ("x", "look mode"),
    ] {
        script(&mut run, &format!("{keys}\n"));
        look(&run, what);
        script(&mut run, "escape\n");
        assert_eq!(top(&run), "ui:play", "back from {what}");
    }
    // The errand's parcel, held: a parcel in the status column and the
    // inventory, and the inventory's popup about it.
    let mut errand = boot_default(&data_dir("ess-names-errand"));
    begin(&mut errand, ERRAND, "Ada");
    if top(&errand) == "ui:intro" {
        script(&mut errand, "escape\n");
    }
    settle(&mut errand);
    assert!(
        errand.text().contains("Holding a parcel"),
        "{}",
        errand.text()
    );
    look(&errand, "the errand");
    item_popup(&mut errand, "parcel");
    look(&errand, "the parcel's popup");
    assert!(errand.text().contains("Parcel"), "{}", errand.text());
    script(&mut errand, "escape\nescape\n");
    // A blow that cannot be made, a thing that cannot be worn, nobody to
    // choose: each says why in words.
    script(&mut errand, "shift+f\n");
    look(&errand, "the blows");
    script(&mut errand, "escape\n");
    do_item(&mut errand, "parcel", "wear");
    script(&mut errand, "g\nwait 30\n");
    settle(&mut errand);

    for run in [&run, &errand] {
        for (category, line) in log(run) {
            if line.chars().next().is_some_and(char::is_lowercase) {
                problems.push(format!("a log line ({category}) begins small: {line:?}"));
            }
            for problem in internal_names(&line, allowed) {
                problems.push(format!("the log ({category}): {problem}"));
            }
        }
    }
    let said: Vec<String> = log(&errand).into_iter().map(|(_, line)| line).collect();
    assert!(
        said.iter()
            .any(|line| line == "The parcel is not something to wear."),
        "{said:?}"
    );
    assert!(problems.is_empty(), "{problems:#?}");
    let _ = std::fs::remove_dir_all(&data);
}
