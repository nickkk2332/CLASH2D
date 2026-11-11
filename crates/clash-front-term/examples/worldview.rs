//! The world viewer: a small test world drawn from the view model, in a
//! terminal.
//!
//! ```text
//! cargo run -p clash-front-term --example worldview
//! ```
//!
//! It loads the default profile from the `mods` directory beside the
//! crates (or the directory given as the first argument), builds a few
//! buildings with `clash_world::test_support`, fills the scene's view
//! model from truth (the omniscient mode of 12 §5) and opens the `ui`
//! mod's `worldview` screen, whose `map_view` draws it.
//!
//! | Key | Does |
//! |-----|------|
//! | Arrows | Move the cursor, and the view with it |
//! | `<`, `>` | View the storey above, below |
//! | `-`, `+` | Zoom out, in |
//! | `s` | Show or hide a section through the middle of the view |
//! | `x` | Turn the section: west to east, or north to south |
//! | `m` | Make the western half remembered, or seen again |
//! | `q`, Esc | Quit |
//!
//! Without a terminal (input or output redirected) it prints the first
//! frame as plain text and exits successfully.
//!
//! The same file is the support module of `tests/worldview.rs`, which
//! drives [`Demo`] through the null frontend.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clash_data::{Diagnostics, Loader};
use clash_front_term::{TermError, TermFrontend};
use clash_scene::view::{WorldView, omniscient};
use clash_scene::{Canvas, Frame, Frontend, InputEvent, KeyChord, KeyCode, NoHost, Scene, UiDefs};
use clash_time::Timeline;
use clash_world::defs::RealmDef;
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, CellBox, CellPos, Column, ColumnBuilder, Cosmos, FixtureState, FloorState, GridId,
    Layer, Location, TractPos, WorldDefs,
};

/// How long the loop rests between frames.
const FRAME_TIME: Duration = Duration::from_millis(16);
/// The grid of the frame printed when there is no terminal.
pub const HEADLESS_SIZE: (u16, u16) = (80, 24);

/// The profile the viewer loads.
const PROFILE: &str = "default";
/// The theme the viewer is shown with.
const THEME: &str = "ui:default";
/// The keymap the viewer is used with.
const KEYMAP: &str = "ui:default";
/// The screen with the map alone.
pub const SCREEN_MAP: &str = "ui:worldview";
/// The screen with the map and a section beneath it.
pub const SCREEN_SECTION: &str = "ui:worldview_section";
/// The realm the test world is made in.
const REALM: &str = "core:testland";
/// The seed of the realm. Nothing here is generated, so it decides
/// nothing.
const SEED: u64 = 1;

/// The storey of the ground.
const GROUND_Z: i32 = 0;
/// Where the town's north-west corner is. The town lies across the edge
/// between two tracts.
const TOWN_ORIGIN: (i32, i32) = (4, 3);
/// The tracts the test world loads.
const TRACTS: [TractPos; 2] = [TractPos::new(0, 0), TractPos::new(1, 0)];
/// The cell the view starts on: between the two buildings, on the
/// ground, in the row of the pillar, the rubble and the ramp.
pub const START: CellPos = CellPos::new(24, 7, GROUND_Z);
/// The lowest and highest storeys the viewer fills its view model with.
const FILL_Z: (i32, i32) = (-4, 6);
/// The first column of the eastern half: `m` makes everything west of
/// it remembered.
pub const HALF_X: i32 = 24;

/// The town, storey by storey from the ground up, north at the top. The
/// legend is [`Legend::cell`].
const TOWN: [[&str; 11]; 3] = [
    [
        "\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"",
        "\"\"##########=#####\"\"\"\"\"\"\"\"###=#####\"\"\"\"\"",
        "\"\"#....n...#.....#\"\"\"\"\"\"\"\"#.......#\"\"\"\"\"",
        "\"\"#........+.....=\"\"\"\"\"\"\"\"'.......#\"\"\"\"\"",
        "\"\"#..O.....#..;..#\"\"\"\"\"\"\"\"#...<...#\"\"\"\"\"",
        "\"\"#........#.....#\"\"\"\"\"\"\"\"#.......#\"\"\"\"\"",
        "\"\"####+#####=#####\"\"\"\"\"\"\"\"####=####\"\"\"\"\"",
        "\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"",
        ",,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,",
        ",,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,",
        "\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"",
    ],
    [
        "                                        ",
        "  ##########=#####        .........     ",
        "  #....n...#.....#        .........     ",
        "  #........'.....=        .........     ",
        "  #..O.....#..  .#        .... ....     ",
        "  #........#.....#        .........     ",
        "  ################        .........     ",
        "                                        ",
        "                                        ",
        "                                        ",
        "                                        ",
    ],
    [
        "                                        ",
        "  ................                      ",
        "  ................                      ",
        "  ................                      ",
        "  ................                      ",
        "  ................                      ",
        "  ................                      ",
        "                                        ",
        "                                        ",
        "                                        ",
        "                                        ",
    ],
];

fn describe(diagnostics: Diagnostics) -> String {
    diagnostics.render()
}

/// The mods directory: the first argument, or the one beside the crates.
fn mods_dir() -> PathBuf {
    match std::env::args_os().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => default_mods_dir(),
    }
}

/// The repository's `mods` directory.
pub fn default_mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// What each character of [`TOWN`] stands for.
struct Legend {
    rock: BlockState,
    grass: FloorState,
    asphalt: FloorState,
    slab: FloorState,
    wall: BlockState,
    window: BlockState,
    door: BlockState,
    door_open: BlockState,
    pillar: BlockState,
    ramp: BlockState,
    rubble: FixtureState,
    desk: FixtureState,
}

impl Legend {
    fn new(world: &WorldDefs) -> Result<Legend, String> {
        let floor = |name: &str| world.floor_state(name, None).map_err(|e| e.to_string());
        let block = |name: &str| world.block_state(name, None).map_err(|e| e.to_string());
        let fixture = |name: &str| world.fixture_state(name, None).map_err(|e| e.to_string());
        Ok(Legend {
            rock: block("core:solid")?,
            grass: floor("core:grass")?,
            asphalt: floor("core:asphalt")?,
            slab: floor("core:slab")?,
            wall: block("core:wall")?,
            window: block("core:window")?,
            door: block("core:door")?,
            door_open: block("core:door#open")?,
            pillar: block("core:pillar")?,
            ramp: block("core:ramp")?,
            rubble: fixture("core:rubble")?,
            desk: fixture("core:desk")?,
        })
    }

    /// The floor, block and fixture a character of the town stands for.
    /// A space is nothing at all: open air.
    fn cell(&self, ch: char) -> Option<(FloorState, BlockState, FixtureState)> {
        let (no_block, no_fixture) = (BlockState::NONE, FixtureState::NONE);
        Some(match ch {
            '"' => (self.grass, no_block, no_fixture),
            ',' => (self.asphalt, no_block, no_fixture),
            '.' => (self.slab, no_block, no_fixture),
            '#' => (self.slab, self.wall, no_fixture),
            '=' => (self.slab, self.window, no_fixture),
            '+' => (self.slab, self.door, no_fixture),
            '\'' => (self.slab, self.door_open, no_fixture),
            'O' => (self.slab, self.pillar, no_fixture),
            '<' => (self.slab, self.ramp, no_fixture),
            ';' => (self.slab, no_block, self.rubble),
            'n' => (self.slab, no_block, self.desk),
            _ => return None,
        })
    }
}

/// The columns of the test world: rock below the ground, grass on it,
/// and the town of [`TOWN`], with `extras` (a cell and a fixture) placed
/// on top.
fn town_columns(
    world: &WorldDefs,
    extras: &[(CellPos, FixtureState)],
) -> Result<Vec<(TractPos, Column)>, String> {
    let legend = Legend::new(world)?;
    let mut columns = Vec::new();
    for tract in TRACTS {
        let mut builder = ColumnBuilder::new(world, legend.rock, BlockState::NONE, GROUND_Z);
        builder.fill_floor(GROUND_Z, legend.grass);
        for (storey, rows) in TOWN.iter().enumerate() {
            for (row, text) in rows.iter().enumerate() {
                for (col, ch) in text.chars().enumerate() {
                    let cell = CellPos::new(
                        TOWN_ORIGIN.0 + col as i32,
                        TOWN_ORIGIN.1 + row as i32,
                        GROUND_Z + storey as i32,
                    );
                    if cell.tract() != tract {
                        continue;
                    }
                    if let Some((floor, block, fixture)) = legend.cell(ch) {
                        builder
                            .set_floor(cell, floor)
                            .set_block(cell, block)
                            .set_fixture(cell, fixture);
                    }
                }
            }
        }
        for (cell, fixture) in extras {
            if cell.tract() == tract {
                builder.set_fixture(*cell, *fixture);
            }
        }
        columns.push((tract, builder.build()));
    }
    Ok(columns)
}

/// The viewer: a scene whose view model is filled from a small test
/// world.
pub struct Demo {
    /// The scene, with the viewer's screen open.
    pub scene: Scene<NoHost>,
    /// The world: truth. The scene never sees it; [`Demo::refill`] shows
    /// it to the view model.
    pub cosmos: Cosmos,
    /// The grid the town is in.
    pub grid: GridId,
    world: Arc<WorldDefs>,
    ui: UiDefs,
    extras: Vec<(CellPos, FixtureState)>,
    section: bool,
    half_remembered: bool,
}

impl Demo {
    /// Loads the default profile from `mods`, builds the test world and
    /// fills the view model from it.
    pub fn new(mods: &Path) -> Result<Demo, String> {
        let types = clash_scene::engine_types();
        let loaded = Loader::new(&types)
            .search_path(mods)
            .load_profile(PROFILE)
            .into_result()
            .map_err(describe)?;
        let world = WorldDefs::from_defs(&loaded).map_err(describe)?;
        let ui = UiDefs::from_defs(&loaded).map_err(describe)?;
        let mut scene = ui.scene(THEME, KEYMAP).map_err(describe)?;

        let mut timeline = Timeline::new();
        let mut cosmos = Cosmos::new(Arc::clone(&world));
        let realm_def = loaded
            .lookup::<RealmDef>(REALM)
            .ok_or_else(|| format!("there is no realm `{REALM}`"))?;
        let realm = cosmos
            .create_realm(&mut timeline, realm_def, SEED)
            .map_err(|e| e.to_string())?;
        let grid = cosmos
            .realm(realm)
            .map(|realm| realm.base_grid)
            .ok_or("the realm was not made")?;
        for (tract, column) in town_columns(&world, &[])? {
            load_column(&mut cosmos, grid, tract, column).map_err(|e| e.to_string())?;
        }

        let glyphs = ui.world_glyphs(&world, scene.theme()).map_err(describe)?;
        scene.set_world(WorldView::new(glyphs, Location::new(grid, START)));
        let screen = ui.screen(SCREEN_MAP, scene.theme()).map_err(describe)?;
        scene.open(screen);
        let mut demo = Demo {
            scene,
            cosmos,
            grid,
            world,
            ui,
            extras: Vec::new(),
            section: false,
            half_remembered: false,
        };
        demo.refill()?;
        Ok(demo)
    }

    /// The box of cells the viewer knows about: the loaded tracts, from
    /// below the ground to above the roofs.
    fn known_box() -> CellBox {
        let last = TRACTS[TRACTS.len() - 1];
        let far = last.cell(clash_world::dims::CHUNK_AREA - 1, FILL_Z.1);
        CellBox::new(TRACTS[0].origin(FILL_Z.0), far)
    }

    /// Fills the view model from truth again: the omniscient mode. What
    /// has changed in the world since the last fill shows from now on.
    pub fn refill(&mut self) -> Result<(), String> {
        let view = self.scene.world_mut().ok_or("the scene has no view")?;
        omniscient::fill(view.model_mut(), &self.cosmos, self.grid, Demo::known_box())
            .map_err(|e| e.to_string())?;
        self.half_remembered = false;
        Ok(())
    }

    /// Puts a fixture into the world, behind the view model's back: the
    /// view does not show it until the cell is observed or the view
    /// model is refilled.
    ///
    /// Cells of a loaded column change only through `WorldEdit`, which
    /// is not built yet, so the town's columns are built again with the
    /// fixture in place and loaded over the old ones.
    pub fn place_fixture(&mut self, at: CellPos, fixture: &str) -> Result<(), String> {
        let state = self
            .world
            .fixture_state(fixture, None)
            .map_err(|e| e.to_string())?;
        self.extras.push((at, state));
        for (tract, column) in town_columns(&self.world, &self.extras)? {
            load_column(&mut self.cosmos, self.grid, tract, column).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Shows the section beneath the map, or hides it.
    fn toggle_section(&mut self) -> Result<(), String> {
        let id = if self.section {
            SCREEN_MAP
        } else {
            SCREEN_SECTION
        };
        let screen = self.ui.screen(id, self.scene.theme()).map_err(describe)?;
        self.scene.close();
        self.scene.open(screen);
        self.section = !self.section;
        Ok(())
    }

    /// Makes the western half of the map remembered, or seen again.
    fn toggle_memory(&mut self) -> Result<(), String> {
        if self.half_remembered {
            return self.refill();
        }
        let all = Demo::known_box();
        let west = CellBox::new(all.min, CellPos::new(HALF_X - 1, all.max.y, all.max.z));
        let grid = self.grid;
        let view = self.scene.world_mut().ok_or("the scene has no view")?;
        view.model_mut().unsee_box(grid, west);
        self.half_remembered = true;
        Ok(())
    }

    /// Acts on a key. Returns whether the viewer should close.
    pub fn key(&mut self, key: KeyChord) -> Result<bool, String> {
        let Some(view) = self.scene.world_mut() else {
            return Ok(false);
        };
        let camera = view.camera_mut();
        // The cursor moves one glyph, however many cells that stands for.
        let step = camera.zoom() as i32;
        match key.code {
            KeyCode::Left => camera.move_cursor(-step, 0),
            KeyCode::Right => camera.move_cursor(step, 0),
            KeyCode::Up => camera.move_cursor(0, -step),
            KeyCode::Down => camera.move_cursor(0, step),
            KeyCode::Char('<') => camera.shift_storey(1),
            KeyCode::Char('>') => camera.shift_storey(-1),
            KeyCode::Char('+' | '=') => camera.zoom_in(),
            KeyCode::Char('-') => camera.zoom_out(),
            KeyCode::Char('x') => camera.set_section_axis(camera.section_axis().turned()),
            KeyCode::Char('s') => self.toggle_section()?,
            KeyCode::Char('m') => self.toggle_memory()?,
            _ => {
                let events = self.scene.input(&InputEvent::Key(key));
                return Ok(events.iter().any(|event| event.asks_for("close")));
            }
        }
        Ok(false)
    }

    /// One line saying where the view is and what the view model holds
    /// there.
    pub fn status(&self) -> String {
        let Some(view) = self.scene.world() else {
            return String::new();
        };
        let camera = view.camera();
        let at = camera.centre();
        let mut line = format!("{at}  zoom {}", camera.zoom());
        if self.half_remembered {
            line.push_str("  west half remembered");
        }
        match view.model().cell_in(self.grid, at) {
            None => line.push_str("  not known"),
            Some(known) => {
                let states = [
                    (Layer::Fixture, known.fixture.raw()),
                    (Layer::Block, known.block.raw()),
                    (Layer::Floor, known.floor.raw()),
                ];
                let mut any = false;
                for (layer, state) in states {
                    if !state.is_none() {
                        line.push_str("  ");
                        line.push_str(&self.world.state_text(layer, state));
                        any = true;
                    }
                }
                if !any {
                    line.push_str("  open");
                }
            }
        }
        line
    }

    /// Builds the frame: the screen, and the status on its last row.
    pub fn draw(&mut self, frame: &mut Frame) {
        self.scene.build_frame(frame);
        let style = self.scene.theme().base();
        let row = i32::from(frame.rows) - 1;
        let status = self.status();
        Canvas::new(frame).print(1, row, &status, style);
    }
}

fn run() -> Result<(), String> {
    let mut demo = Demo::new(&mods_dir())?;
    let mut frame = Frame::new(0, 0);
    let mut front = match TermFrontend::start(false) {
        Ok(front) => front,
        Err(TermError::NotATerminal) => {
            frame.resize(HEADLESS_SIZE.0, HEADLESS_SIZE.1);
            demo.draw(&mut frame);
            print!("{}", frame.text());
            println!(
                "worldview: not started, because standard input and output are not both a terminal."
            );
            println!(
                "The definitions loaded, the world was built and the first frame is shown above. Run it in a terminal to use it."
            );
            return Ok(());
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut last = Instant::now();
    loop {
        for event in front.poll() {
            let close = match event {
                InputEvent::Close => true,
                InputEvent::Key(key) => demo.key(key)?,
                other => {
                    demo.scene.input(&other);
                    false
                }
            };
            if close {
                return Ok(());
            }
        }
        let now = Instant::now();
        demo.scene.advance(now.duration_since(last).as_secs_f64());
        last = now;
        let caps = front.caps();
        if (frame.cols, frame.rows) != (caps.cols, caps.rows) {
            frame.resize(caps.cols, caps.rows);
        }
        demo.draw(&mut frame);
        front.present(&frame);
        std::thread::sleep(FRAME_TIME);
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("worldview: {message}");
            ExitCode::FAILURE
        }
    }
}
