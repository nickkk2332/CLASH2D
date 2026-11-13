//! Play session: every bundled power (08), each used once from the
//! abilities screen as a player uses it, in the test town as someone
//! with every power of the sampler (the `armoury` fixture's game
//! `gifted`), with a neighbour beside them for the powers that act on
//! someone. Each power's effect is looked for where a player sees it:
//! the log, the map, the character's reserves.

mod session;

use clash_app::headless::Headless;
use session::*;

/// Opens the abilities screen and uses the row of the uses list or of
/// the modes list that begins with `name`; a place or someone asked
/// for is chosen by moving the cursor `east` cells east first. Returns
/// the lines the log gained, once the use has played out.
fn use_power(run: &mut Headless, name: &str, mode: bool, east: usize) -> Vec<String> {
    press(run, "a");
    assert_eq!(top(run), "ui:abilities");
    let rows: Vec<String> = run
        .text()
        .lines()
        .filter_map(|line| line.strip_prefix('│').map(str::to_owned))
        .collect();
    let list: Vec<&String> = if mode {
        rows.iter()
            .skip_while(|row| !row.starts_with("Modes"))
            .skip(1)
            .take_while(|row| row.starts_with('['))
            .collect()
    } else {
        rows.iter()
            .skip(1)
            .take_while(|row| !row.trim().is_empty())
            .collect()
    };
    let index = list
        .iter()
        .position(|row| {
            let row = row.trim_start_matches("[  ] ").trim_start_matches("[on] ");
            row.starts_with(name)
        })
        .unwrap_or_else(|| panic!("no `{name}` in {list:?}"));
    if mode {
        press(run, "tab");
    }
    for _ in 0..index {
        press(run, "down");
    }
    let before = log_len(run);
    press(run, "enter");
    if mode {
        press(run, "escape");
    } else if top(run) != "ui:play" {
        for _ in 0..east {
            press(run, "l");
        }
        press(run, "enter");
    }
    assert_eq!(top(run), "ui:play", "{}", run.text());
    for _ in 0..30 {
        run.pass();
    }
    settle(run);
    let gained = log_since(run, before);
    // A line said again is folded into the one before it: the last line
    // stands for it then.
    let last = log_text(run).last().cloned().unwrap_or_default();
    let mut said = gained;
    said.push(last);
    said
}

/// Lets the reserves come back.
fn rest(run: &mut Headless) {
    for _ in 0..20 {
        press(run, ".");
    }
}

/// How many creatures tagged `animal` are in the world.
fn animals(run: &Headless) -> usize {
    run.game()
        .with_session(|session| {
            session
                .sim
                .truth()
                .entities()
                .iter()
                .filter(|entity| entity.tags.contains("animal"))
                .count()
        })
        .unwrap_or_default()
}

#[test]
fn every_power_is_used_from_the_abilities_screen() {
    let data = DataDir::new("powers");
    let mut run = boot_armoury(data.path());
    script(&mut run, "enter\ntype Ada\ntab\ntab\nenter\nwait 5\n");
    settle(&mut run);
    assert!(run.game().playing());
    let shown = run.text();
    // What powers are paid from is on the play screen.
    assert!(shown.contains("Stam") && shown.contains("Rsrv"), "{shown}");

    let dash = use_power(&mut run, "Dash", false, 0);
    assert!(
        dash.iter().any(|line| line == "Ada uses a power."),
        "{dash:?}"
    );
    rest(&mut run);

    let push = use_power(&mut run, "Force push", false, 2);
    assert!(push.iter().any(|line| line.contains("blast")), "{push:?}");
    rest(&mut run);

    let bolt = use_power(&mut run, "Fire bolt", false, 3);
    assert!(bolt.iter().any(|line| line.contains("blast")), "{bolt:?}");
    rest(&mut run);

    let heal = use_power(&mut run, "Heal", false, 0);
    assert!(
        heal.iter().any(|line| line == "Ada uses a power."),
        "{heal:?}"
    );
    rest(&mut run);

    let well = use_power(&mut run, "Gravity well", false, 3);
    assert!(
        well.iter().any(|line| line == "Ada uses a power."),
        "{well:?}"
    );
    rest(&mut run);

    let before = animals(&run);
    let hound = use_power(&mut run, "Summon hound", false, 2);
    assert!(
        hound.iter().any(|line| line == "Ada uses a power."),
        "{hound:?}"
    );
    assert_eq!(animals(&run), before + 1, "a hound came");
    rest(&mut run);

    let shown = run.text();
    // The line that says a power was used folds into the hound's.
    let wall = use_power(&mut run, "Stone wall", false, 2);
    assert!(!wall.iter().any(|line| line.contains("cannot")), "{wall:?}");
    assert_ne!(run.text(), shown, "a wall stands across the street");
    rest(&mut run);

    // A mode says when it takes hold and when it is let go.
    for mode in ["Shield", "Time stop", "Null field", "Float", "Shadow"] {
        let on = use_power(&mut run, mode, true, 0);
        assert!(
            on.iter().any(|line| line == "Ada calls up a power."),
            "{mode}: {on:?}"
        );
        let off = use_power(&mut run, mode, true, 0);
        assert!(
            off.iter().any(|line| line == "Ada lets a power go."),
            "{mode}: {off:?}"
        );
        rest(&mut run);
    }
}
