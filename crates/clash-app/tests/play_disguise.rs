//! Play session: a disguise (15 §8.3; 06 §9.2; decision D-971), from the
//! title in the market town, as a player plays it: a hooded cloak cut and
//! sewn at the workshop's bench from the cloth on its racks with the
//! knife from its chest, put on, and worn to the square; someone struck
//! there in it; the watch, told of a hooded stranger, comes for whoever
//! wears the hood; a wardrobe in a house by the square, and the cloak
//! taken off inside it and left there; and out again, past a watchman,
//! who lets her by. `docs/playtest-beta.md`, the fifth round.

mod session;
mod town;

use clash_app::headless::Headless;
use clash_core::EntityId;
use clash_world::Location;
use session::*;
use town::*;

/// The workshop's chest of tools, its racks of stuff (the two in a line
/// with the chest) and its benches.
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

/// Puts down whatever the character carries that is not of the
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
            .cloned()
            .unwrap_or_default();
        let name = row.split("  ").next().unwrap_or_default().to_owned();
        assert!(with_carried(run, &name, "drop"), "{name}: {rows:?}");
    }
}

/// The personas of the character, read from truth for the assertions:
/// the one it presents, and its own true one.
fn personas(run: &Headless) -> (String, String) {
    let ada = player(run);
    (
        text_of(run, ada, "presenting"),
        text_of(run, ada, "persona"),
    )
}

/// Whether any watchman holds the character hostile.
fn the_watch_hunts(run: &Headless, watch: &[EntityId]) -> bool {
    let ada = player(run);
    watch.iter().any(|guard| holds(run, *guard, "hostile", ada))
}

/// Whether a mind remembers whom it takes `of` for as having struck
/// without cause (`core:unprovoked_blow`).
fn remembers_a_blow(run: &Headless, who: EntityId, of: EntityId) -> bool {
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
                    "remembers_actor",
                    &[
                        clash_expr::Value::from(of),
                        clash_expr::Value::from("core:unprovoked_blow"),
                    ],
                )
                .as_bool()
        })
        .flatten()
        == Some(true)
}

#[test]
fn a_hooded_cloak_worn_struck_in_taken_off_and_the_watch_lets_her_by() {
    let data = DataDir::new("hooded-cloak");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);
    let square = player_loc(&run).unwrap();
    let watch = minds_named(&run, "watch");

    // The knife from the workshop's chest, the cloth from a rack.
    let (chest, racks, benches) = workshop(&run);
    let door = site_named(&run, "Workshop");
    walk_to(&mut run, door, 1, 600);
    walk_to(&mut run, chest, 1, 60);
    assert!(use_offer(&mut run, "Open"), "{:?}", near_rows(&mut run));
    assert!(use_offer(&mut run, "Take out"), "{:?}", near_rows(&mut run));
    assert!(carries(&run, "core:knife") > 0, "{:?}", carried(&run));
    put_down_all_but(&mut run, &["core:knife"]);
    for rack in &racks {
        walk_to(&mut run, *rack, 1, 60);
        assert!(use_offer_at(&mut run, "Take out", Some(*rack)));
        if carries(&run, "core:cloth") >= 2 {
            break;
        }
        put_down_all_but(&mut run, &["core:knife"]);
    }
    assert!(carries(&run, "core:cloth") >= 2, "{:?}", carried(&run));

    // At the bench, the cloak: a minute's work, and it lies in the bench.
    let bench = benches[0];
    walk_to(&mut run, bench, 1, 40);
    let before = log_len(&run);
    let row = make(&mut run, "Hooded cloak");
    assert!(
        row.starts_with("Hooded cloak  ") && row.contains("2 strip of cloth, tool: cutting 1"),
        "{row}"
    );
    let lines = log_since(&run, before);
    assert!(
        lines
            .iter()
            .any(|line| line == "Ada finishes the work: what it gave is in the workbench."),
        "{lines:?}"
    );
    put_down_all_but(&mut run, &["core:knife"]);
    assert!(
        use_offer_at(&mut run, "Take out", Some(bench)),
        "{:?}",
        near_rows(&mut run)
    );
    assert_eq!(carries(&run, "core:hooded_cloak"), 1, "{:?}", carried(&run));

    // Worn: she presents a stranger in a hood.
    let (shown, own) = personas(&run);
    assert_eq!(shown, own, "herself before the hood");
    assert!(
        with_carried(&mut run, "hooded cloak", "wear"),
        "{:?}",
        carried(&run)
    );
    let rows = inventory_rows(&mut run);
    assert!(
        rows.iter().any(|row| row == "hooded cloak  worn"),
        "{rows:?}"
    );
    let (hood, own) = personas(&run);
    assert_ne!(hood, own, "the hood is someone else");

    // Back to the square, and someone not of the watch struck in it.
    walk_to(&mut run, square, 2, 600);
    let mut victim = None;
    for _ in 0..40 {
        press(&mut run, "tab");
        if let Some(entity) = target(&run)
            && !watch.contains(&entity)
            && !text_of(&run, entity, "mind_def").is_empty()
        {
            victim = Some(entity);
            break;
        }
        press(&mut run, ".");
    }
    victim.expect("someone in the square");
    let before = log_len(&run);
    let mut landed = false;
    for _ in 0..60 {
        answer(&mut run);
        if !walk_up_to_target(&mut run, 12) {
            press(&mut run, "tab");
            continue;
        }
        press(&mut run, "f");
        if !["ui:play", "ui:reaction"].contains(&top(&run).as_str()) {
            press(&mut run, "escape");
        }
        press(&mut run, ".");
        landed = log_since(&run, before)
            .iter()
            .any(|line| line.contains("is wounded") || line.contains("staggers"));
        if landed {
            break;
        }
    }
    let fight = log_since(&run, before);
    assert!(landed, "no blow landed: {fight:?}");
    let struck = target(&run).unwrap_or_else(|| victim.unwrap());
    let ada = player(&run);
    eprintln!(
        "struck {}: takes her for {:?} (hood {hood}, own {own})",
        text_of(&run, struck, "mind_def"),
        about(&run, struck, "persona_of", ada)
    );
    // The one struck takes her for the hooded stranger, and remembers
    // the blow as the stranger's.
    assert_eq!(
        about(&run, struck, "persona_of", ada).as_str(),
        Some(hood.as_str())
    );
    assert!(remembers_a_blow(&run, struck, ada), "{fight:?}");

    // The watch is told of a hooded stranger, and comes for whoever wears
    // the hood: her, while she wears it.
    let mut hunted = false;
    for _ in 0..90 {
        answer(&mut run);
        hunted = the_watch_hunts(&run, &watch);
        if hunted {
            break;
        }
        press(&mut run, ".");
    }
    assert!(
        hunted,
        "the watch never came: {:?}",
        log_since(&run, before)
    );
    for guard in &watch {
        if holds(&run, *guard, "hostile", ada) {
            assert_eq!(
                about(&run, *guard, "persona_of", ada).as_str(),
                Some(hood.as_str()),
                "the watch knows the hood, not her"
            );
        }
    }

    // To the nearest wardrobe in a house by the square, and in.
    let here = player_loc(&run).unwrap();
    let wardrobe = *near_any_storey(&run, here, "wardrobe", 30)
        .first()
        .expect("a wardrobe in a house by the square");
    for _ in 0..80 {
        assert!(
            !caught(&run),
            "caught on the way: {:?}",
            log_since(&run, before)
        );
        answer(&mut run);
        let Some(at) = player_loc(&run) else { break };
        let delta = wardrobe.cell - at.cell;
        if delta.x.abs().max(delta.y.abs()) <= 1 && delta.z == 0 {
            break;
        }
        if walk_to(&mut run, wardrobe, 1, 4) == 0 && top(&run) == "ui:play" {
            press(&mut run, ".");
        }
    }
    answer(&mut run);
    assert!(
        use_offer(&mut run, "Hide inside"),
        "{:?}",
        near_rows(&mut run)
    );
    assert!(hidden(&run), "not inside");

    // Inside, out of everyone's sight, the cloak comes off and is left
    // there: she is herself again.
    assert!(
        with_carried(&mut run, "hooded cloak", "remove"),
        "{:?}",
        inventory_rows(&mut run)
    );
    assert!(
        with_carried(&mut run, "hooded cloak", "drop"),
        "{:?}",
        inventory_rows(&mut run)
    );
    assert_eq!(carries(&run, "core:hooded_cloak"), 0);
    let (shown, own) = personas(&run);
    assert_eq!(shown, own, "herself again");
    for _ in 0..3 {
        answer(&mut run);
        press(&mut run, "p");
    }
    assert!(!caught(&run), "found in the wardrobe");

    // Out, and up to a watchman: he sees her, takes her for herself, and
    // lets her by.
    assert!(use_offer(&mut run, "Get out"), "{:?}", near_rows(&mut run));
    assert!(!hidden(&run));
    walk_to(&mut run, square, 2, 200);
    let mut guard = None;
    for _ in 0..60 {
        press(&mut run, "tab");
        if let Some(entity) = target(&run)
            && watch.contains(&entity)
        {
            guard = Some(entity);
            break;
        }
        press(&mut run, ".");
    }
    let guard = guard.expect("a watchman in sight");
    assert!(walk_up_to_target(&mut run, 80), "{}", run.text());
    let before = log_len(&run);
    for _ in 0..10 {
        answer(&mut run);
        press(&mut run, ".");
    }
    let passing = log_since(&run, before);
    eprintln!("passing the watchman: {passing:?}");
    assert!(holds(&run, guard, "perceives", ada), "he does not see her");
    assert_eq!(
        about(&run, guard, "persona_of", ada).as_str(),
        Some(own.as_str())
    );
    assert!(!holds(&run, guard, "hostile", ada), "{passing:?}");
    assert!(!remembers_a_blow(&run, guard, ada), "{passing:?}");
    assert!(
        !passing
            .iter()
            .any(|line| line.starts_with("The watchman strikes Ada")),
        "{passing:?}"
    );
}
