//! Play sessions of making things and using what is made (16 §8, §9.5),
//! from the title in the market town, as a player makes them: a bandage
//! made at the workshop's bench and a wound dressed with it; nightshade
//! gathered in the healer's garden, its extract made, a blade coated and
//! someone cut with it. `docs/playtest-beta.md`, the sixth round.

mod session;
mod town;

use clash_app::headless::Headless;
use clash_world::Location;
use session::*;
use town::*;

/// How many of the character's wounds are open.
fn open_wounds(run: &Headless) -> usize {
    run.game()
        .with_session(|session| {
            let body = session
                .sim
                .components()
                .get::<clash_sim::body::Body>(session.player)?;
            Some(
                body.wounds()
                    .iter()
                    .filter(|wound| wound.state() == clash_sim::body::WoundState::Open)
                    .count(),
            )
        })
        .flatten()
        .unwrap_or(0)
}

/// The workshop's chest of tools, its racks of stuff (the two in a line
/// with the chest, not the pile of wood) and its benches.
fn workshop(run: &Headless) -> (Location, Vec<Location>, Vec<Location>) {
    let anchor = site_named(run, "Workshop");
    let chest = nearest(run, anchor, "chest", 9);
    let racks: Vec<Location> = near(run, chest, "rack", 6)
        .into_iter()
        .filter(|rack| rack.cell.x == chest.cell.x || rack.cell.y == chest.cell.y)
        .collect();
    let benches = near(run, chest, "workbench", 8);
    (chest, racks, benches)
}

/// Goes to the workshop by the streets and takes what a rack of stuff
/// holds until there is cloth in hand.
fn cloth_from_the_workshop(run: &mut Headless) {
    let (_, racks, _) = workshop(run);
    let door = site_named(run, "Workshop");
    walk_to(run, door, 1, 600);
    for rack in &racks {
        walk_to(run, *rack, 1, 60);
        assert!(use_and_wait(run, "Take out"), "{}", run.text());
        if carries(run, "core:cloth") > 0 {
            return;
        }
    }
    panic!("no cloth on the workshop's racks: {:?}", carried(run));
}

/// Tab through whoever is in sight until a watchman is the target, at
/// most once round. Returns whether one is.
fn tab_to_the_watch(run: &mut Headless, watch: &[clash_core::EntityId]) -> bool {
    for _ in 0..12 {
        press(run, "tab");
        if target(run).is_some_and(|entity| watch.contains(&entity)) {
            return true;
        }
    }
    false
}

/// A fight with the watch until a blow wounds the character: a
/// watchman in sight is struck; with none in sight, whoever is in sight
/// is struck once, and the watch comes for the one who struck. Blows
/// coming are met as a player would. Returns the log since the start.
fn wounded_by_the_watch(run: &mut Headless) -> Vec<String> {
    let watch = minds_named(run, "watch");
    let before = log_len(run);
    let mut provoked = false;
    for _ in 0..300 {
        if open_wounds(run) > 0 {
            break;
        }
        answer(run);
        if top(run) != "ui:play" {
            continue;
        }
        let at_the_watch = tab_to_the_watch(run, &watch);
        if !at_the_watch && provoked {
            press(run, ".");
            continue;
        }
        if !at_the_watch && !tab_until_someone(run, 3) {
            press(run, ".");
            continue;
        }
        if !walk_up_to_target(run, 8) {
            continue;
        }
        press(run, "f");
        if !["ui:play", "ui:reaction"].contains(&top(run).as_str()) {
            press(run, "escape");
        }
        press(run, ".");
        provoked = provoked
            || log_since(run, before)
                .iter()
                .any(|line| line.starts_with("Ada strikes"));
    }
    settle(run);
    log_since(run, before)
}

/// A bandage made at the workshop's bench, and a wound dressed with it.
/// The cloth comes from a rack of the workshop's; the bandages, made by
/// hand with both hands full, are left at the character's feet, and the
/// log says so (D-972); a hand is freed and they are picked up. Then,
/// back in the square, a watchman struck answers with a blow that
/// wounds, and the wound is bound with what was made.
#[test]
fn a_bandage_made_at_the_workshop_bench_dresses_a_wound() {
    let data = DataDir::new("bandage-made");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);
    let square = player_loc(&run).unwrap();

    // To the workshop, by the streets: cloth from a rack of stuff, and
    // whatever else the hands take with it. A stack is named once, with
    // its count, the plural on the noun (D-972).
    cloth_from_the_workshop(&mut run);
    let rows = inventory_rows(&mut run);
    let cloth = carries(&run, "core:cloth");
    assert!(
        rows.iter()
            .any(|row| row == &format!("{cloth} strips of cloth  in hand")),
        "{rows:?}"
    );

    // At the bench: the make list offers a bandage by hand, and it is
    // made with both hands full.
    let (_, _, benches) = workshop(&run);
    walk_to(&mut run, benches[0], 1, 40);
    let before = log_len(&run);
    let row = make(&mut run, "Bandage");
    assert!(
        row.starts_with("Bandage  hand: 1 strip of cloth gives 2 bandage"),
        "{row}"
    );
    let lines = log_since(&run, before);
    assert_eq!(carries(&run, "core:cloth"), cloth - 1, "{lines:?}");
    assert!(
        lines.iter().any(|line| line
            == "Ada finishes the work: their hands are full, and what it gave lies at their feet."),
        "{lines:?}"
    );
    // A hand freed (what came with the cloth put down), and the bandages
    // picked up.
    let rows = inventory_rows(&mut run);
    let other = rows
        .iter()
        .find(|row| !row.contains("cloth"))
        .expect("something in the other hand")
        .split("  ")
        .next()
        .unwrap()
        .to_owned();
    assert!(with_carried(&mut run, &other, "drop"), "{rows:?}");
    press(&mut run, "g");
    if top(&run) != "ui:play" {
        // More than one thing lies here: the bandages.
        let text = run.text();
        let index = text
            .lines()
            .filter(|line| line.contains('│'))
            .position(|line| line.contains("bandage"))
            .unwrap_or(0);
        eprintln!("picking from:\n{text}");
        for _ in 0..index.saturating_sub(1) {
            press(&mut run, "down");
        }
        press(&mut run, "enter");
    }
    until_free(&mut run, 60);
    assert_eq!(carries(&run, "core:bandage"), 2, "{:?}", carried(&run));

    // Back in the square, a watchman struck answers; once a blow of his
    // wounds, the wound is bound with a bandage made.
    walk_to(&mut run, square, 2, 600);
    let fight = wounded_by_the_watch(&mut run);
    assert!(open_wounds(&run) > 0, "never wounded: {fight:?}");
    assert!(
        fight.iter().any(|line| line == "Ada is wounded."),
        "{fight:?}"
    );
    let before = log_len(&run);
    // A blow coming while the bandage goes on is let happen: a dodge
    // would break the binding off.
    for _ in 0..20 {
        answer(&mut run);
        if top(&run) == "ui:play" && start_offer_at(&mut run, "Bandage your wounds", None) {
            for _ in 0..120 {
                if top(&run) == "ui:reaction" {
                    press(&mut run, "escape");
                }
                let free = run
                    .game()
                    .with_session(|session| session.sim.decisions().has_free(session.player))
                    .unwrap_or(true);
                if free && top(&run) == "ui:play" {
                    break;
                }
                run.pass();
            }
            if carries(&run, "core:bandage") < 2 {
                break;
            }
            eprintln!("broken off: {:?}", log_since(&run, before));
        }
        press(&mut run, ".");
    }
    settle(&mut run);
    let bound = log_since(&run, before);
    eprintln!("binding: {bound:?}");
    assert!(
        bound.iter().any(|line| line == "Ada binds their wounds."),
        "{bound:?}"
    );
    assert_eq!(carries(&run, "core:bandage"), 1, "one bandage used");
}

/// The healer's house: its shelf of books, its cupboard of medicine and
/// the beds of its garden, nearest the door first.
fn healer(run: &Headless) -> (Location, Location, Vec<Location>) {
    let door = site_named(run, "Healer's house");
    let cupboard = nearest(run, door, "cupboard", 9);
    let shelf = nearest(run, cupboard, "rack", 9);
    let beds = near(run, door, "herb_bed", 8);
    (shelf, cupboard, beds)
}

/// Puts down what the character carries that is not of the
/// definitions kept, through the inventory.
fn put_down_all_but(run: &mut Headless, keep: &[&str]) {
    for (have, _) in carried(run) {
        if keep.contains(&have.as_str()) {
            continue;
        }
        let rows = inventory_rows(run);
        let word = have.trim_start_matches("core:").replace('_', " ");
        let row = rows
            .iter()
            .find(|row| row.contains(&word))
            .or_else(|| {
                rows.iter().find(|row| {
                    !keep.iter().any(|kept| {
                        row.contains(&kept.trim_start_matches("core:").replace('_', " "))
                    })
                })
            })
            .cloned()
            .unwrap_or_default();
        let name = row.split("  ").next().unwrap_or_default().to_owned();
        assert!(with_carried(run, &name, "drop"), "{name}: {rows:?}");
    }
}

/// Takes things out of a container within a step, one hand's worth at a
/// time, putting down what is not wanted, until something `wanted` says
/// yes to is in hand, or the container has nothing more to give. What
/// is kept stays in hand.
fn take_out_until(run: &mut Headless, wanted: &dyn Fn(&Headless) -> bool, keep: &[&str]) -> bool {
    for _ in 0..8 {
        if wanted(run) {
            return true;
        }
        if !use_offer(run, "Take out") {
            return false;
        }
        if wanted(run) {
            return true;
        }
        put_down_all_but(run, keep);
    }
    wanted(run)
}

/// How much nightshade is in or on what the character carries, in
/// millilitres: what it holds (`mixture`) or what is on its surfaces
/// (`coating`).
fn nightshade_carried(run: &Headless, on: &str) -> f64 {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let mut total = 0.0;
            for &thing in sim.truth().contents(session.player) {
                let value = sim.property_named(clash_sim::Subject::Entity(thing), on, &[]);
                if let Some(amount) = value
                    .as_map()
                    .and_then(|map| map.get("core:nightshade"))
                    .and_then(clash_expr::Value::as_number)
                {
                    total += amount.get() * 1.0e6;
                }
            }
            Some(total)
        })
        .flatten()
        .unwrap_or(0.0)
}

/// Nightshade gathered in the healer's garden, its extract made, a
/// blade coated with it and someone cut: the poison takes effect, and
/// the log says so. A knife from the workshop's chest; the herbal from
/// the healer's shelf, read for herbalism; the nightshade from the bed
/// grown with it (D-970); the extract pressed by hand into a vial of the
/// healer's cupboard; the vial taken out and a blade coated; then a cut
/// through the skin of someone in the street.
#[test]
fn nightshade_gathered_pressed_and_a_coated_blade_drops_someone() {
    let data = DataDir::new("poisoned-blade");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);
    let square = player_loc(&run).unwrap();

    // A knife from the workshop's chest of tools.
    let (chest, _, _) = workshop(&run);
    let door = site_named(&run, "Workshop");
    walk_to(&mut run, door, 1, 600);
    walk_to(&mut run, chest, 1, 60);
    assert!(use_and_wait(&mut run, "Open"), "{:?}", near_rows(&mut run));
    assert!(
        use_and_wait(&mut run, "Take out"),
        "{:?}",
        near_rows(&mut run)
    );
    eprintln!("from the chest: {:?}", carried(&run));
    assert!(carries(&run, "core:knife") > 0, "{:?}", carried(&run));
    for (have, _) in carried(&run) {
        if have != "core:knife" {
            let name = have.trim_start_matches("core:").replace('_', " ");
            assert!(with_carried(&mut run, &name, "drop"), "{name}");
        }
    }

    // To the healer's house: the herbal from the shelf, read.
    let (shelf, cupboard, beds) = healer(&run);
    eprintln!("shelf {shelf:?} cupboard {cupboard:?} beds {beds:?}");
    let door = site_named(&run, "Healer's house");
    walk_to(&mut run, square, 3, 600);
    walk_to(&mut run, door, 1, 600);
    walk_to(&mut run, shelf, 1, 120);
    let herbal = |run: &Headless| carries(run, "core:herbal") > 0;
    assert!(
        take_out_until(&mut run, &herbal, &["core:knife"]),
        "{:?}",
        carried(&run)
    );
    let before = log_len(&run);
    assert!(use_and_wait(&mut run, "Read"), "{:?}", near_rows(&mut run));
    settle(&mut run);
    eprintln!("read: {:?}", log_since(&run, before));
    assert!(with_carried(&mut run, "herbal", "drop"));

    // Nightshade from the bed grown with it.
    let mut gathered = false;
    for bed in &beds {
        walk_to(&mut run, *bed, 1, 30);
        eprintln!("at a bed: {:?}", near_rows(&mut run));
        if use_and_wait(&mut run, "Take out") && carries(&run, "core:nightshade_plant") > 0 {
            gathered = true;
            break;
        }
    }
    assert!(gathered, "no nightshade in the garden: {:?}", carried(&run));
    eprintln!("gathered: {:?}", carried(&run));

    // The cupboard opened, and the extract pressed by hand.
    walk_to(&mut run, cupboard, 1, 120);
    eprintln!(
        "at the cupboard {cupboard:?} from {:?}: {:?}\n{}",
        player_loc(&run),
        near_rows(&mut run),
        run.text()
    );
    assert!(use_and_wait(&mut run, "Open"), "{:?}", near_rows(&mut run));
    let before = log_len(&run);
    let row = make(&mut run, "Nightshade extract");
    assert!(
        row.starts_with("Nightshade extract  hand: 2 nightshade, tool: cutting 1 gives 20 ml"),
        "{row}"
    );
    let lines = log_since(&run, before);
    eprintln!("made: {lines:?}; carried {:?}", carried(&run));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("Ada finishes the work: what it gave is in the vial")),
        "{lines:?}"
    );

    // The vial out of the cupboard, the rest put down as it comes, and a
    // blade coated: ten millilitres on the knife (D-970).
    put_down_all_but(&mut run, &["core:knife"]);
    let vial = |run: &Headless| nightshade_carried(run, "mixture") >= 19.0;
    let found = take_out_until(&mut run, &vial, &["core:knife"]);
    assert!(
        found,
        "{:?} {} {:?}\n{}",
        carried(&run),
        nightshade_carried(&run, "mixture"),
        near_rows(&mut run),
        run.text()
    );
    eprintln!("in hand: {:?}", carried(&run));
    let before = log_len(&run);
    assert!(
        use_offer(&mut run, "Coat a blade"),
        "{:?}",
        near_rows(&mut run)
    );
    settle(&mut run);
    let lines = log_since(&run, before);
    assert!(
        lines.iter().any(|line| line == "Ada coats a blade."),
        "{lines:?}"
    );
    let coated = nightshade_carried(&run, "coating");
    assert!((coated - 10.0).abs() < 0.01, "{coated} ml on the blade");

    // Out to the street, and someone not of the watch cut with it.
    let watch = minds_named(&run, "watch");
    let door = site_named(&run, "Healer's house");
    walk_to(&mut run, door, 2, 120);
    let before = log_len(&run);
    let mut struck = None;
    for _ in 0..300 {
        answer(&mut run);
        if top(&run) != "ui:play" {
            continue;
        }
        if log_since(&run, before)
            .iter()
            .any(|line| line.ends_with("falls senseless."))
        {
            break;
        }
        if target(&run).is_none_or(|entity| watch.contains(&entity)) {
            press(&mut run, "tab");
            if target(&run).is_none_or(|entity| watch.contains(&entity)) {
                press(&mut run, ".");
                continue;
            }
        }
        if !walk_up_to_target(&mut run, 10) {
            continue;
        }
        struck = target(&run);
        press(&mut run, "f");
        if !["ui:play", "ui:reaction"].contains(&top(&run).as_str()) {
            press(&mut run, "escape");
        }
        press(&mut run, ".");
    }
    settle(&mut run);
    let lines = log_since(&run, before);
    let cut = struck.expect("someone in the street struck");
    let toxin = pool_share(&run, cut, "core:toxin").unwrap_or(0.0);
    let state = text_of(&run, cut, "vital");
    eprintln!("cut {cut}: {lines:?}; toxin {toxin}, {state}");
    // The one cut falls senseless, and the log says so; the poison did
    // it: the toxin is past the half that knocks out (D-970).
    assert!(
        lines
            .iter()
            .any(|line| line == "The person falls senseless."),
        "{lines:?}"
    );
    assert_eq!(state, "unconscious", "{lines:?}");
    assert!(toxin > 0.5, "toxin {toxin}");
    // The blade keeps what the cut did not take or wipe off.
    let left = nightshade_carried(&run, "coating");
    assert!(left > 0.0 && left < 10.0, "{left} ml left on the blade");
}
