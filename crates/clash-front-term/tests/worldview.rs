//! The world viewer run headless: the test world of the `worldview`
//! example drawn from the view model through the null frontend, with
//! golden frames (12 §12) for each storey, a section, a zoomed-out view,
//! remembered cells, looking down through a hole and low light; and the
//! check that the view shows only what was put into the view model
//! (package U4, 12 §5).

use std::path::PathBuf;
use std::process::Command;

use clash_scene::view::{CellContents, LIGHT_FULL};
use clash_scene::{Frame, FrameCell, Frontend, InputEvent, NullFrontend, assert_golden};
use clash_world::{CellPos, Location};

// The example is the support module: the viewer and its test world.
#[path = "../examples/worldview.rs"]
#[allow(dead_code)]
mod worldview;

use worldview::{Demo, HALF_X, HEADLESS_SIZE, START, default_mods_dir};

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

/// A headless run: keys in, frames out through the null frontend.
struct Run {
    demo: Demo,
    front: NullFrontend,
    frame: Frame,
}

impl Run {
    fn new() -> Run {
        let (cols, rows) = HEADLESS_SIZE;
        Run {
            demo: Demo::new(&default_mods_dir()).unwrap_or_else(|error| panic!("{error}")),
            front: NullFrontend::new(cols, rows),
            frame: Frame::new(cols, rows),
        }
    }

    /// Presses keys. None of them may ask to close.
    fn press(&mut self, keys: &[&str]) {
        for key in keys {
            let InputEvent::Key(chord) = InputEvent::key(key).unwrap() else {
                panic!("`{key}` is not a key");
            };
            let close = self
                .demo
                .key(chord)
                .unwrap_or_else(|error| panic!("{error}"));
            assert!(!close, "`{key}` closed the viewer");
        }
    }

    /// Builds a frame and presents it; returns what the frontend got.
    fn show(&mut self) -> Frame {
        self.demo.draw(&mut self.frame);
        self.front.present(&self.frame);
        self.front.last().unwrap().clone()
    }

    /// The frame cell a world cell of the viewed storey is drawn in, in
    /// the map view at no zoom.
    fn at(&self, frame: &Frame, x: i32, y: i32) -> FrameCell {
        let centre = self.demo.scene.world().unwrap().camera().centre();
        // The map's area: inside its border, under the heading.
        let (left, top) = (1, 2);
        let (width, height) = (i32::from(frame.cols) - 2, self.map_rows(frame));
        let col = left + (width - 1) / 2 + (x - centre.x);
        let row = top + (height - 1) / 2 + (y - centre.y);
        *frame.get(col, row).expect("the cell is in view")
    }

    /// How many rows the map view has: the frame less the heading, the
    /// status row and the map's border.
    fn map_rows(&self, frame: &Frame) -> i32 {
        i32::from(frame.rows) - 4
    }
}

#[test]
fn the_town_is_drawn_storey_by_storey() {
    let mut run = Run::new();
    let ground = run.show();
    assert_golden(golden("worldview_z0"), &ground);
    let text = ground.text();
    for row in [
        "##########=#####\"\"\"\"\"\"\"\"###=#####",
        "#....n...#.....#\"\"\"\"\"\"\"\"#.......#",
        "#........+.....=\"\"\"\"\"\"\"\"'.......#",
        "#..O.....#..;..#\"\"\"\"\"\"\"\"#...<...#",
        "####+#####=#####\"\"\"\"\"\"\"\"####=####",
        // The street: asphalt is drawn as a dot, as a slab is, in its
        // own colour.
        "\"\"\"\"........................................\"\"\"\"",
    ] {
        assert!(text.contains(row), "missing `{row}` in:\n{text}");
    }
    // A wall takes the colour of its material and a window of its own.
    let wall = run.at(&ground, 6, 4);
    let window = run.at(&ground, 16, 4);
    assert_eq!((wall.ch, window.ch), ('#', '='));
    assert_ne!(wall.fg, window.fg);
    // Beyond the loaded tracts and beyond the realm's edge nothing is
    // known, and nothing is drawn.
    assert_eq!(run.at(&ground, -3, 7).ch, ' ');
    assert_eq!(run.at(&ground, 24, -1).ch, ' ');

    run.press(&["<"]);
    let first = run.show();
    assert_golden(golden("worldview_z1"), &first);
    // The upper storey of the western building, and the ground around it
    // seen from above, dimmer.
    assert_eq!(run.at(&first, 6, 4).ch, '#');
    let below = run.at(&first, 24, 7);
    assert_eq!(below.ch, '"');
    assert!(below.fg.rgb.g < run.at(&ground, 24, 7).fg.rgb.g);

    run.press(&["<"]);
    let second = run.show();
    assert_golden(golden("worldview_z2"), &second);
    // The roof of the western building; the terrace of the eastern one a
    // storey down; the ground two storeys down, dimmer again.
    assert_eq!(run.at(&second, 10, 6).ch, '.');
    assert!(run.at(&second, 24, 7).fg.rgb.g < below.fg.rgb.g);

    // Under the ground there is rock, which is its material's colour
    // behind an empty character.
    run.press(&[">", ">", ">"]);
    let rock = run.show();
    let cell = run.at(&rock, 24, 7);
    assert_eq!(cell.ch, ' ');
    assert_ne!(cell.bg, run.at(&rock, -3, 7).bg);
}

#[test]
fn a_section_cuts_through_the_town() {
    let mut run = Run::new();
    run.press(&["s"]);
    let frame = run.show();
    assert_golden(golden("worldview_section"), &frame);
    let text = frame.text();
    // The row of the pillar, the rubble and the ramp, edge on: the roof,
    // the upper storey with its hole and the opening over the ramp, and
    // the ground storey.
    for row in [
        "________________",
        "#__O_____#__  _#        ____ ____",
        "#__O_____#__;__#________#___<___#",
    ] {
        assert!(text.contains(row), "missing `{row}` in:\n{text}");
    }
    // Turned, the section runs north to south through the same cell.
    run.press(&["x"]);
    let turned = run.show();
    assert_ne!(turned.text(), text);
    assert_golden(golden("worldview_section_north_south"), &turned);
    // The section goes away again.
    run.press(&["s"]);
    assert!(text.contains("┌ section"));
    assert!(!run.show().text().contains("┌ section"));
}

#[test]
fn zoomed_out_each_glyph_is_the_most_significant_of_its_block() {
    let mut run = Run::new();
    run.press(&["-"]);
    let frame = run.show();
    assert_golden(golden("worldview_zoom2"), &frame);
    let text = frame.text();
    // Half the size: the western building is eight glyphs wide and three
    // high. A wall shows over a floor; the desk, a fixture, over a wall;
    // and of a door and a wall, which are both blocks, the first, row by
    // row.
    for row in ["##n##=##", "#O..+.;=", "##+##=##"] {
        assert!(text.contains(row), "missing `{row}` in:\n{text}");
    }
    run.press(&["-", "-"]);
    let far = run.show();
    assert_golden(golden("worldview_zoom4"), &far);
    run.press(&["+", "+", "+", "+"]);
    assert_eq!(run.demo.scene.world().unwrap().camera().zoom(), 1);
}

#[test]
fn half_the_map_can_be_remembered() {
    let mut run = Run::new();
    let seen = run.show();
    run.press(&["m"]);
    let frame = run.show();
    assert_golden(golden("worldview_remembered"), &frame);
    // The same characters, and west of the line greyer and darker.
    assert_eq!(frame.text().lines().nth(5), seen.text().lines().nth(5));
    let (west, east) = (HALF_X - 1, HALF_X);
    let remembered = run.at(&frame, west, 7);
    assert_eq!(remembered.ch, '"');
    assert_ne!(remembered.fg, run.at(&seen, west, 7).fg);
    assert_eq!(run.at(&frame, east, 7), run.at(&seen, east, 7));
    let green = |cell: FrameCell| i32::from(cell.fg.rgb.g) - i32::from(cell.fg.rgb.r);
    assert!(green(remembered) < green(run.at(&frame, east, 7)));
    assert_eq!(
        remembered.fg.fallback,
        clash_scene::Ansi16::BrightBlack,
        "a terminal with sixteen colours shows what is remembered in dark grey"
    );
    // Seen again.
    run.press(&["m"]);
    assert_eq!(run.show(), seen);
}

#[test]
fn an_open_cell_shows_what_lies_below_it() {
    let mut run = Run::new();
    // On the ground the rubble lies in the eastern room of the western
    // building.
    let ground = run.show();
    let rubble = run.at(&ground, 18, 7);
    assert_eq!(rubble.ch, ';');

    // A storey up, the floor above it has a hole: the rubble shows
    // through, dimmer. The cursor is put beside the hole.
    run.press(&["<", "left", "left", "left", "left"]);
    let frame = run.show();
    assert_golden(golden("worldview_look_down"), &frame);
    let through = run.at(&frame, 18, 7);
    assert_eq!(through.ch, ';');
    assert!(through.fg.rgb.r < rubble.fg.rgb.r);
    // The other cell of the hole shows the ground floor's slab.
    assert_eq!(run.at(&frame, 19, 7).ch, '.');
    assert!(run.at(&frame, 19, 7).fg.rgb.r < run.at(&frame, 17, 7).fg.rgb.r);
    // The opening over the ramp shows the ramp.
    assert_eq!(run.at(&frame, 34, 7).ch, '<');
    assert!(run.demo.status().contains("(20, 7, 1)"));

    // With no looking down, an open cell over a known one shows the
    // backdrop.
    run.demo.scene.world_mut().unwrap().set_look_down(0);
    let shallow = run.show();
    assert_eq!(run.at(&shallow, 18, 7).ch, ' ');
}

#[test]
fn light_scales_brightness() {
    let mut run = Run::new();
    let lit = run.show();
    // A lamp where the view starts: full light there, falling off with
    // the square of the distance.
    let grid = run.demo.grid;
    let model = run.demo.scene.world_mut().unwrap().model_mut();
    for y in 0..32 {
        for x in 0..64 {
            for z in -1..=2 {
                let (dx, dy) = (i64::from(x - START.x), i64::from(y - START.y));
                let fall = 24 + dx * dx + dy * dy;
                let light = (i64::from(LIGHT_FULL) * 24 / fall) as u8;
                model.observe_light(Location::new(grid, CellPos::new(x, y, z)), light);
            }
        }
    }
    let frame = run.show();
    assert_golden(golden("worldview_low_light"), &frame);
    assert_eq!(frame.text(), lit.text(), "light changes no character");
    assert_eq!(
        run.at(&frame, START.x, START.y),
        run.at(&lit, START.x, START.y)
    );
    let near = run.at(&frame, START.x + 3, START.y);
    let far = run.at(&frame, START.x + 30, START.y);
    assert!(near.fg.rgb.g < run.at(&lit, START.x + 3, START.y).fg.rgb.g);
    assert!(far.fg.rgb.g < near.fg.rgb.g);
    assert!(far.fg.rgb.g > 0, "a seen cell is never black");
}

#[test]
fn what_is_put_into_the_world_later_shows_only_once_observed() {
    let mut run = Run::new();
    let before = run.show();
    // A desk appears in the street. The world has it; the view model was
    // not told.
    let street = CellPos::new(30, 11, 0);
    run.demo.place_fixture(street, "core:desk").unwrap();
    let loc = Location::new(run.demo.grid, street);
    let truth = run.demo.cosmos.cell(loc).unwrap();
    assert!(!truth.fixture.is_none());
    assert_eq!(run.show(), before, "nothing was observed, so nothing shows");

    // Someone sees the cell: one observation, and it shows.
    let model = run.demo.scene.world_mut().unwrap().model_mut();
    model.observe_cell(
        loc,
        CellContents::elements(truth.floor, truth.block, truth.fixture),
    );
    let observed = run.show();
    assert_eq!(run.at(&observed, street.x, street.y).ch, 'n');

    // A second desk, and this time the omniscient refill shows it, with
    // everything else as it is.
    let second = CellPos::new(31, 11, 0);
    run.demo.place_fixture(second, "core:desk").unwrap();
    assert_eq!(run.show(), observed);
    run.demo.refill().unwrap();
    let refilled = run.show();
    assert_eq!(run.at(&refilled, second.x, second.y).ch, 'n');
    assert_eq!(run.at(&refilled, street.x, street.y).ch, 'n');
}

#[test]
fn the_example_prints_its_first_frame_without_a_terminal() {
    // Cargo builds examples before running integration tests and tells us
    // where the test binary is; the example is beside it.
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    let example = path
        .join("examples")
        .join(format!("worldview{}", std::env::consts::EXE_SUFFIX));
    if !example.exists() {
        // Built some other way (for example with `--tests` alone).
        return;
    }
    let output = Command::new(&example)
        .arg(default_mods_dir())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The first frame is the one the scripted run starts with.
    let mut run = Run::new();
    assert!(stdout.starts_with(&run.show().text()), "{stdout}");
    assert!(stdout.contains("not both a terminal"), "{stdout}");
    assert!(
        !stdout.contains('\x1b'),
        "no escape sequences without a terminal"
    );

    let missing = Command::new(&example)
        .arg(default_mods_dir().join("no-such-directory"))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("worldview: "), "{stderr}");
}
