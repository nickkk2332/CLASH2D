//! The view model and how the world is drawn (12 §5, §6; package U4):
//! what a cell shows, in what order and how shaded; that the view shows
//! what was put into the view model and nothing else; overlays; the
//! views that are not built; and the `ambience` and `overlay`
//! definitions through the mod loader.
//!
//! The definitions are a small mod written here, so that these tests do
//! not depend on the content of `mods/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_data::{DefTypes, Defs, Loader};
use clash_scene::ui::{UiData, ViewPainter};
use clash_scene::view::{
    Ambience, BuiltinOverlay, CellContents, Composer, DEPTH_KEEP, EntitySprite, FieldLook,
    FieldSeen, HeardMarker, KnownCell, LIGHT_FLOOR, LIGHT_FULL, MEMORY_KEEP, Overlay, OverlayDraw,
    OverlayValues, Rank, SpriteId, SpriteKind, ViewModel, WorldGlyphs, WorldView, omniscient,
};
use clash_scene::{
    Ansi16, Attrs, Color, Frame, FrameCell, GlyphDef, GlyphInputs, GlyphPart, Rgb, SectionAxis,
    Theme, UiDefs, assert_golden,
};
use clash_time::Timeline;
use clash_world::defs::RealmDef;
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, CellBox, CellPos, ColumnBuilder, Cosmos, FieldId, FixtureState, FloorState, GridId,
    Location, TractPos, WorldDefs,
};

const MOD: &str = r##"[
  { "type": "material", "id": "stone", "density": 2000, "toughness": 1.0e6, "color": "#808080" },
  { "type": "material", "id": "oak", "density": 600, "toughness": 3.0e5, "color": "tan" },
  { "type": "color", "id": "tan", "rgb": "#c8a060", "fallback": "yellow" },

  { "type": "floor", "id": "boards", "material": "oak", "thickness": 0.1,
    "glyph": { "ch": ".", "fg": "material" } },
  { "type": "floor", "id": "carpet", "material": "oak", "thickness": 0.1,
    "glyph": { "ch": "~", "fg": "#ff0000", "bg": "#000080" } },
  { "type": "block", "id": "rock", "material": "stone", "height": 3.0, "fill": 1.0,
    "flags": ["opaque"], "glyph": { "ch": " ", "bg": "material" } },
  { "type": "block", "id": "wall", "material": "stone", "height": 3.0, "fill": 0.2,
    "flags": ["opaque"], "glyph": { "ch": "#", "fg": "material" } },
  { "type": "block", "id": "gate", "material": "oak", "height": 3.0, "fill": 0.1,
    "states": { "closed": {}, "open": { "glyph": { "ch": "'" } } },
    "glyph": { "ch": "+", "fg": "material" } },
  { "type": "block", "id": "ghost", "material": "stone", "height": 1.0, "fill": 0.1 },
  { "type": "fixture", "id": "chair", "material": "oak", "height": 1.0, "fill": 0.1,
    "glyph": { "ch": "h", "fg": "material" } },

  { "type": "realm", "id": "room", "size_regions": [1, 1], "z_range": [-8, 16],
    "strata": [ { "from_z": -8, "gravity": 9.81 } ] },

  { "type": "theme", "id": "plain",
    "colors": { "text": "#c0c0c0", "background": "#000000" },
    "marks": { "pile": { "ch": "%", "fg": "yellow" } } },

  { "type": "ambience", "id": "dusk", "tint": "#ff8040", "dim": 0.5 },
  { "type": "overlay", "id": "seats", "name": "Seats", "cell": "has_fixture(it)", "shade": "green" },

  { "type": "screen", "id": "views", "layout": { "rows": [
    { "cols": [ { "map_view": { "id": "map" } }, { "section_view": { "id": "section" } } ] },
    { "cols": [ { "region_map": { "id": "region" } }, { "chart_view": { "id": "chart" } } ] } ] } },
  { "type": "screen", "id": "lit", "layout": { "map_view": { "overlay": "light" } } },
  { "type": "screen", "id": "seated", "layout": { "map_view": { "overlay": "demo:seats" } } },
  { "type": "screen", "id": "far", "layout": { "map_view": { "zoom": 2 } } }
]"##;

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_world::register_defs(&mut types);
    clash_scene::register_defs(&mut types);
    types
}

fn load(text: &str) -> Arc<Defs> {
    let types = types();
    let outcome = Loader::new(&types)
        .memory_mod("demo", r#"{ "id": "demo" }"#, &[("data/demo.json", text)])
        .load_mods(&["demo"]);
    match outcome.into_result() {
        Ok(defs) => defs,
        Err(diagnostics) => panic!("the mod does not load:\n{}", diagnostics.render()),
    }
}

/// Everything a test draws with: the definitions, a world with one
/// loaded tract of bare rock under open air, and a view of it.
struct Fixture {
    ui: UiDefs,
    world: Arc<WorldDefs>,
    theme: Theme,
    cosmos: Cosmos,
    grid: GridId,
}

impl Fixture {
    fn new() -> Fixture {
        let defs = load(MOD);
        let ui = UiDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
        let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
        let theme = ui
            .theme("demo:plain")
            .unwrap_or_else(|d| panic!("{}", d.render()));
        let mut cosmos = Cosmos::new(Arc::clone(&world));
        let realm_def = defs.lookup::<RealmDef>("demo:room").expect("the realm");
        let realm = cosmos
            .create_realm(&mut Timeline::new(), realm_def, 1)
            .expect("a realm");
        let grid = cosmos.realm(realm).expect("the realm").base_grid;
        let mut fixture = Fixture {
            ui,
            world,
            theme,
            cosmos,
            grid,
        };
        fixture.build(&[]);
        fixture
    }

    fn floor(&self, name: &str) -> FloorState {
        self.world.floor_state(name, None).unwrap()
    }

    fn block(&self, name: &str) -> BlockState {
        self.world.block_state(name, None).unwrap()
    }

    fn fixture(&self, name: &str) -> FixtureState {
        self.world.fixture_state(name, None).unwrap()
    }

    /// Loads the one tract again: rock below storey 0, boards on it, and
    /// these blocks.
    fn build(&mut self, blocks: &[(CellPos, &str)]) {
        let mut builder = ColumnBuilder::new(&self.world, self.block("rock"), BlockState::NONE, 0);
        builder.fill_floor(0, self.floor("boards"));
        for (cell, block) in blocks {
            builder.set_block(*cell, self.block(block));
        }
        load_column(
            &mut self.cosmos,
            self.grid,
            TractPos::new(0, 0),
            builder.build(),
        )
        .unwrap();
    }

    fn at(&self, x: i32, y: i32, z: i32) -> Location {
        Location::new(self.grid, CellPos::new(x, y, z))
    }

    fn glyphs(&self) -> WorldGlyphs {
        self.ui
            .world_glyphs(&self.world, &self.theme)
            .unwrap_or_else(|d| panic!("{}", d.render()))
    }

    /// A view following a cell, knowing nothing yet.
    fn view(&self, x: i32, y: i32, z: i32) -> WorldView {
        WorldView::new(self.glyphs(), self.at(x, y, z))
    }

    /// Draws a screen of the mod with a view.
    fn draw(&self, screen: &str, view: &WorldView, cols: u16, rows: u16) -> Frame {
        let mut screen = self
            .ui
            .screen(screen, &self.theme)
            .unwrap_or_else(|d| panic!("{}", d.render()));
        let mut frame = Frame::new(cols, rows);
        frame.clear(FrameCell::blank(self.theme.base()));
        let data = UiData {
            log: None,
            views: Some(view as &dyn ViewPainter),
        };
        screen.draw(&mut frame, &data);
        frame
    }
}

fn creature(loc: Location, ch: char) -> EntitySprite {
    EntitySprite {
        loc,
        kind: SpriteKind::Creature,
        glyph: GlyphPart {
            ch: Some(ch),
            ..GlyphPart::default()
        },
        own: None,
        dress: None,
        facing: None,
        suspected: false,
    }
}

fn scaled(rgb: Rgb, keep: u32) -> Rgb {
    rgb.toward(Rgb::BLACK, (1000 - keep) as u16)
}

const STONE: Rgb = Rgb::new(0x80, 0x80, 0x80);
const TAN: Rgb = Rgb::new(0xc8, 0xa0, 0x60);

// ---------------------------------------------------------------------
// What a cell shows
// ---------------------------------------------------------------------

#[test]
fn the_first_that_applies_is_shown_and_backgrounds_show_through() {
    let fx = Fixture::new();
    let glyphs = fx.glyphs();
    let mut model = ViewModel::new();
    let here = fx.at(5, 5, 0);
    let shown = |model: &ViewModel| Composer::new(model, &glyphs).plan(here.grid, here.cell);
    let base = fx.theme.base();

    // Never seen: nothing.
    assert_eq!(
        shown(&model),
        clash_scene::view::Shown {
            cell: FrameCell::blank(base),
            rank: Rank::Unknown
        }
    );

    // A floor, in the colours its glyph gives.
    let mut contents = CellContents {
        floor: fx.floor("carpet"),
        ..CellContents::EMPTY
    };
    model.observe_cell(here, contents);
    let carpet = shown(&model);
    assert_eq!(
        (carpet.rank, carpet.cell.ch, carpet.cell.fg.rgb),
        (Rank::Floor, '~', Rgb::new(0xff, 0, 0))
    );
    let carpet_bg = Rgb::new(0, 0, 0x80);
    assert_eq!(carpet.cell.bg.rgb, carpet_bg);

    // A fixture over it: its own character and colour, the carpet's
    // background.
    contents.fixture = fx.fixture("chair");
    model.observe_cell(here, contents);
    let chair = shown(&model);
    assert_eq!(
        (
            chair.rank,
            chair.cell.ch,
            chair.cell.fg.rgb,
            chair.cell.bg.rgb
        ),
        (Rank::Fixture, 'h', TAN, carpet_bg)
    );

    // A block is under a fixture and over a floor; a block with no
    // glyph is not drawn at all.
    contents.fixture = FixtureState::NONE;
    contents.block = fx.block("ghost");
    model.observe_cell(here, contents);
    assert_eq!(shown(&model), carpet);
    contents.block = fx.block("gate#open");
    model.observe_cell(here, contents);
    let gate = shown(&model);
    assert_eq!(
        (gate.rank, gate.cell.ch, gate.cell.fg.rgb),
        (Rank::Block, '\'', TAN)
    );
    contents.block = fx.block("gate");
    contents.fixture = fx.fixture("chair");
    model.observe_cell(here, contents);
    assert_eq!(shown(&model).cell.ch, 'h');

    // An object over the fixture, the pile mark over a single object,
    // and a creature over everything, in its own and its dress colours.
    let key = EntitySprite {
        kind: SpriteKind::Object,
        ..creature(here, '-')
    };
    model.upsert_sprite(SpriteId::new(2), key);
    let object = shown(&model);
    assert_eq!((object.rank, object.cell.ch), (Rank::Items, '-'));
    assert_eq!(
        object.cell.fg, base.fg,
        "a glyph with no colour is the theme's text"
    );
    contents.pile = true;
    model.observe_cell(here, contents);
    let pile = shown(&model);
    assert_eq!(
        (pile.rank, pile.cell.ch, pile.cell.fg),
        (Rank::Items, '%', Color::ansi(Ansi16::Yellow))
    );
    let dressed: GlyphDef =
        serde_json::from_str(r#"{ "ch": "@", "fg": "dress", "bg": "own" }"#).unwrap();
    let red = Color::ansi(Ansi16::Red);
    let cyan = Color::ansi(Ansi16::Cyan);
    let person = EntitySprite {
        glyph: dressed
            .compile(&fx.theme)
            .unwrap()
            .part(&GlyphInputs::default()),
        own: Some(cyan),
        dress: Some(red),
        ..creature(here, '@')
    };
    model.upsert_sprite(SpriteId::new(1), person);
    let seen = shown(&model);
    assert_eq!(
        (seen.rank, seen.cell.ch, seen.cell.fg, seen.cell.bg),
        (Rank::Creature, '@', red, cyan)
    );

    // A presence suspected and not detected is a mark, not a glyph.
    model.upsert_sprite(
        SpriteId::new(1),
        EntitySprite {
            suspected: true,
            ..person
        },
    );
    let suspected = shown(&model);
    assert_eq!((suspected.rank, suspected.cell.ch), (Rank::Creature, '?'));
    assert!(suspected.cell.attrs.contains(Attrs::BOLD));
    assert_eq!(suspected.cell.bg.rgb, carpet_bg);
}

#[test]
fn a_field_is_under_a_block_and_over_a_floor_and_a_weak_one_tints() {
    let fx = Fixture::new();
    let mut glyphs = fx.glyphs();
    let fire = FieldId::new(3);
    let glyph: GlyphDef = serde_json::from_str(
        r##"{ "ch": "^", "by_level": [ { "fg": "yellow" }, { "fg": "red" } ] }"##,
    )
    .unwrap();
    glyphs.set_field_look(
        fire,
        FieldLook {
            glyph: glyph.compile(&fx.theme).unwrap(),
            glyph_from: 100,
            band: 100,
            tint: Some(Color::ansi(Ansi16::BrightRed)),
        },
    );
    let mut model = ViewModel::new();
    let here = fx.at(1, 1, 0);
    let mut contents = CellContents {
        floor: fx.floor("boards"),
        field: Some(FieldSeen {
            field: fire,
            level: 120,
        }),
        ..CellContents::EMPTY
    };
    let shown = |model: &ViewModel| Composer::new(model, &glyphs).plan(here.grid, here.cell);

    model.observe_cell(here, contents);
    let low = shown(&model);
    assert_eq!(
        (low.rank, low.cell.ch, low.cell.fg),
        (Rank::Field, '^', Color::ansi(Ansi16::Yellow))
    );
    contents.field = Some(FieldSeen {
        field: fire,
        level: 250,
    });
    model.observe_cell(here, contents);
    assert_eq!(shown(&model).cell.fg, Color::ansi(Ansi16::Red));

    // A wall hides it.
    contents.block = fx.block("wall");
    model.observe_cell(here, contents);
    assert_eq!(
        (shown(&model).rank, shown(&model).cell.ch),
        (Rank::Block, '#')
    );

    // Too weak for its glyph, it tints the background of the floor.
    contents.block = BlockState::NONE;
    contents.field = Some(FieldSeen {
        field: fire,
        level: 50,
    });
    model.observe_cell(here, contents);
    let weak = shown(&model);
    assert_eq!((weak.rank, weak.cell.ch), (Rank::Floor, '.'));
    assert_eq!(weak.cell.bg.rgb, Rgb::new(64, 0, 0));

    // A field with no look draws nothing.
    contents.field = Some(FieldSeen {
        field: FieldId::new(9),
        level: 255,
    });
    model.observe_cell(here, contents);
    assert_eq!(shown(&model).cell.bg, fx.theme.base().bg);
}

#[test]
fn an_open_cell_shows_the_cell_below_dimmer_and_only_what_is_known() {
    let fx = Fixture::new();
    let glyphs = fx.glyphs();
    let mut model = ViewModel::new();
    let floor = CellContents {
        floor: fx.floor("boards"),
        ..CellContents::EMPTY
    };
    let grid = fx.grid;
    let top = CellPos::new(3, 3, 5);
    let shown = |model: &ViewModel, look_down: u8| {
        Composer {
            look_down,
            ..Composer::new(model, &glyphs)
        }
        .plan(grid, top)
    };

    // Open cells down to a floor three storeys below.
    for z in 3..=5 {
        model.observe_cell(fx.at(3, 3, z), CellContents::EMPTY);
    }
    // What is below them was never seen: nothing is drawn, however far
    // the view may look.
    assert_eq!(shown(&model, 8).rank, Rank::Unknown);

    model.observe_cell(fx.at(3, 3, 2), floor);
    let below = shown(&model, 8);
    assert_eq!((below.rank, below.cell.ch), (Rank::Below, '.'));
    let keep = DEPTH_KEEP * DEPTH_KEEP / 1000 * DEPTH_KEEP / 1000;
    assert_eq!(below.cell.fg.rgb, scaled(TAN, keep));

    // Looking down fewer storeys than that finds only the backdrop.
    assert_eq!(shown(&model, 3).rank, Rank::Below);
    let backdrop = shown(&model, 2);
    assert_eq!((backdrop.rank, backdrop.cell.ch), (Rank::Backdrop, ' '));

    // A creature on the floor below is seen through the opening.
    model.upsert_sprite(SpriteId::new(1), creature(fx.at(3, 3, 2), 'r'));
    assert_eq!(shown(&model, 8).cell.ch, 'r');
    assert_eq!(shown(&model, 8).rank, Rank::Below);

    // If any cell on the way is only remembered, so is what is shown.
    model.unsee_cell(fx.at(3, 3, 4));
    let remembered = shown(&model, 8);
    assert_eq!(remembered.cell.fg.fallback, Ansi16::BrightBlack);
}

#[test]
fn light_ambience_and_memory_shade_in_order() {
    let fx = Fixture::new();
    let glyphs = fx.glyphs();
    let mut model = ViewModel::new();
    let here = fx.at(2, 2, 0);
    let wall = CellContents {
        block: fx.block("wall"),
        ..CellContents::EMPTY
    };
    model.observe_cell(here, wall);
    let plain = Composer::new(&model, &glyphs);
    assert_eq!(
        plain.plan(here.grid, here.cell).cell.fg,
        Color::from_rgb(STONE)
    );

    // Light scales brightness, down to a floor.
    model.observe_light(here, 0);
    let dark = Composer::new(&model, &glyphs).plan(here.grid, here.cell);
    assert_eq!(dark.cell.fg.rgb, scaled(STONE, LIGHT_FLOOR));
    assert_eq!(
        dark.cell.fg.fallback,
        Ansi16::BrightBlack,
        "sixteen colours show the darker of the pair"
    );
    model.observe_light(here, LIGHT_FULL);

    // The ambience is a filter and a dimming.
    let dusk = fx
        .ui
        .ambience("demo:dusk", &fx.theme)
        .unwrap_or_else(|d| panic!("{}", d.render()));
    assert_eq!(dusk, Ambience::plain(Some(Rgb::new(0xff, 0x80, 0x40)), 500));
    let composer = Composer {
        ambience: Some(&dusk),
        ..Composer::new(&model, &glyphs)
    };
    let filtered = dusk.filter(STONE);
    assert_eq!(filtered, Rgb::new(0x80, 0x40, 0x20));
    let at_dusk = composer.plan(here.grid, here.cell);
    assert_eq!(at_dusk.cell.fg.rgb, scaled(filtered, 500));

    // A remembered cell loses most of its colour and some brightness,
    // and falls back to dark grey.
    model.unsee_cell(here);
    let composer = Composer {
        ambience: Some(&dusk),
        ..Composer::new(&model, &glyphs)
    };
    let remembered = composer.plan(here.grid, here.cell);
    // The grey of the filtered colour is (79, 79, 79); a quarter of the
    // colour is kept.
    let drained = filtered.toward(Rgb::new(79, 79, 79), 750);
    assert_eq!(
        remembered.cell.fg.rgb,
        scaled(drained, 500 * MEMORY_KEEP / 1000)
    );
    assert_eq!(remembered.cell.fg.fallback, Ansi16::BrightBlack);
    assert_eq!(remembered.cell.ch, '#');
    assert!(remembered.cell.attrs.is_empty());
}

// ---------------------------------------------------------------------
// Belief, not truth
// ---------------------------------------------------------------------

#[test]
fn the_view_shows_what_was_observed_and_the_omniscient_fill_shows_truth() {
    let mut fx = Fixture::new();
    let mut view = fx.view(4, 2, 0);
    let cells = CellBox::new(CellPos::new(0, 0, -1), CellPos::new(8, 4, 1));

    // Nothing was put into the view model: nothing is drawn, though the
    // world is there.
    assert_eq!(fx.draw("demo:far", &view, 9, 5).text().trim(), "");
    let filled = omniscient::fill(view.model_mut(), &fx.cosmos, fx.grid, cells).unwrap();
    assert_eq!(filled.seen, cells.volume());
    assert_eq!(view.model().known_cells() as u64, cells.volume());
    let screen = "demo:seated";
    let before = fx.draw(screen, &view, 9, 5);
    assert_eq!(before.row_text(2), ".........");

    // A wall goes up in the world. The view model was not told.
    fx.build(&[(CellPos::new(6, 2, 0), "wall")]);
    assert!(!fx.cosmos.cell(fx.at(6, 2, 0)).unwrap().block.is_none());
    assert_eq!(fx.draw(screen, &view, 9, 5), before);

    // The cell is observed: one call, and it shows.
    let truth = fx.cosmos.cell(fx.at(6, 2, 0)).unwrap();
    view.model_mut().observe_cell(
        fx.at(6, 2, 0),
        CellContents::elements(truth.floor, truth.block, truth.fixture),
    );
    assert_eq!(fx.draw(screen, &view, 9, 5).row_text(2), "......#..");

    // Another, and the omniscient refill shows it.
    fx.build(&[
        (CellPos::new(6, 2, 0), "wall"),
        (CellPos::new(1, 2, 0), "gate"),
    ]);
    assert_eq!(fx.draw(screen, &view, 9, 5).row_text(2), "......#..");
    omniscient::fill(view.model_mut(), &fx.cosmos, fx.grid, cells).unwrap();
    assert_eq!(fx.draw(screen, &view, 9, 5).row_text(2), ".+....#..");

    // The fill told the view model the grid's shape, so a view at the
    // edge of the realm draws nothing beyond it and does not wrap.
    assert!(view.model().grid_shape(fx.grid).is_some());
    view.camera_mut().set_cursor(0, 0);
    let corner = fx.draw(screen, &view, 9, 5);
    assert_eq!(corner.row_text(1), "         ");
    assert_eq!(corner.row_text(2), "    .....");

    // Cells that are not loaded are forgotten by a fill, not kept.
    let wide = CellBox::new(CellPos::new(30, 0, 0), CellPos::new(33, 0, 0));
    let filled = omniscient::fill(view.model_mut(), &fx.cosmos, fx.grid, wide).unwrap();
    assert_eq!((filled.seen, filled.not_loaded), (2, 2));
    assert!(
        omniscient::fill(view.model_mut(), &fx.cosmos, GridId::new(99), wide).is_err(),
        "a grid that does not exist is refused"
    );
}

/// The files of this crate's source, with their paths from `src`.
fn source_files(dir: &Path, prefix: &str, into: &mut Vec<(String, String)>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let shown = format!("{prefix}{name}");
        if path.is_dir() {
            source_files(&path, &format!("{shown}/"), into);
        } else if name.ends_with(".rs") {
            into.push((shown, std::fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn only_the_omniscient_fill_can_read_the_world() {
    // The world is read through `Cosmos` and `GridView`. Neither may be
    // named in this crate's code outside the omniscient module: the view
    // model, the composer, the painters, the scene and the toolkit have
    // no way to reach truth. Comments and documentation may mention
    // them.
    let mut files = Vec::new();
    source_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        "",
        &mut files,
    );
    assert!(files.iter().any(|(path, _)| path == "view/omniscient.rs"));
    assert!(files.len() > 15, "the source was found");
    let mut named = Vec::new();
    for (path, text) in &files {
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let reads = ["Cosmos", "GridView"]
                .iter()
                .any(|name| code.contains(name));
            if reads {
                named.push(format!("{path}:{}", number + 1));
            }
        }
    }
    assert!(!named.is_empty());
    assert!(
        named
            .iter()
            .all(|place| place.starts_with("view/omniscient.rs:")),
        "the world is read outside the omniscient fill: {named:?}"
    );
}

// ---------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------

/// A view of a small room: walls round boards, a chair, an open gate, a
/// creature, and a storey above with a hole in its floor.
fn room(fx: &Fixture) -> WorldView {
    let mut view = fx.view(4, 2, 0);
    let boards = fx.floor("boards");
    let rows = [
        "#########",
        "#...h...#",
        "#.......'",
        "#.......#",
        "#########",
    ];
    let model = view.model_mut();
    for (y, row) in rows.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            let (block, fixture) = match ch {
                '#' => (fx.block("wall"), FixtureState::NONE),
                '\'' => (fx.block("gate#open"), FixtureState::NONE),
                'h' => (BlockState::NONE, fx.fixture("chair")),
                _ => (BlockState::NONE, FixtureState::NONE),
            };
            let (x, y) = (x as i32, y as i32);
            model.observe_cell(
                fx.at(x, y, 0),
                CellContents::elements(boards, block, fixture),
            );
            model.observe_cell(
                fx.at(x, y, -1),
                CellContents::elements(FloorState::NONE, fx.block("rock"), FixtureState::NONE),
            );
            // The storey above: boards, but for a hole over the chair.
            let above = if ch == 'h' { FloorState::NONE } else { boards };
            model.observe_cell(
                fx.at(x, y, 1),
                CellContents::elements(above, BlockState::NONE, FixtureState::NONE),
            );
        }
    }
    model.upsert_sprite(SpriteId::new(1), creature(fx.at(2, 3, 0), '@'));
    view
}

#[test]
fn a_screen_draws_the_four_views() {
    let fx = Fixture::new();
    let mut view = room(&fx);
    view.model_mut().add_heard(HeardMarker {
        loc: fx.at(6, 3, 0),
        ch: None,
    });
    view.model_mut().add_heard(HeardMarker {
        loc: fx.at(7, 1, 0),
        ch: Some('*'),
    });
    // A marker on another storey is not drawn on this one.
    view.model_mut().add_heard(HeardMarker {
        loc: fx.at(3, 3, 1),
        ch: Some('X'),
    });
    view.camera_mut().move_cursor(0, 0);
    let frame = fx.draw("demo:views", &view, 22, 10);
    assert_golden(golden("view_four_views"), &frame);
    let text = frame.text();
    let rows: Vec<&str> = text.lines().collect();
    // The map, with the cursor on the middle cell, two heard markers and
    // a creature.
    assert_eq!(rows[0], " #########            ");
    assert_eq!(&rows[1][..11], " #...h..*# ");
    assert_eq!(&rows[2][..11], " #.......' ");
    assert_eq!(&rows[3][..11], " #.@...!.# ");
    let cursor = frame.get(5, 2).unwrap();
    assert!(cursor.attrs.contains(Attrs::REVERSE));
    assert!(!frame.get(4, 2).unwrap().attrs.contains(Attrs::REVERSE));
    assert!(frame.get(7, 3).unwrap().attrs.contains(Attrs::BOLD));
    // The views that are not built say what they are.
    assert!(text.contains("region_map"), "{text}");
    assert!(text.contains("chart_view"), "{text}");

    // The section is the row through the cursor, the camera's storey in
    // the middle: the floor of the storey above, this storey with its
    // walls edge on, and rock beneath. Nothing is known higher or lower.
    let section: Vec<String> = (0..5)
        .map(|row| frame.row_text(row)[11..].to_owned())
        .collect();
    assert_eq!(section[0].trim(), "");
    assert_eq!(section[1], " _________ ");
    assert_eq!(section[2], " #_______' ");
    assert_eq!(section[3].trim(), "");
    assert_ne!(frame.get(14, 3).unwrap().bg, fx.theme.base().bg, "rock");
    assert_eq!(frame.get(14, 4).unwrap().bg, fx.theme.base().bg);
    assert!(frame.get(16, 2).unwrap().attrs.contains(Attrs::REVERSE));

    // Turned, the section runs north to south through the cursor: the
    // chair, and the hole in the floor above it.
    view.camera_mut().set_section_axis(SectionAxis::NorthSouth);
    let turned = fx.draw("demo:views", &view, 22, 10);
    assert_eq!(&turned.row_text(1)[11..], "   _ ___   ");
    assert_eq!(&turned.row_text(2)[11..], "   #h__#   ");

    // Up a storey, the hole shows the chair below.
    view.camera_mut().shift_storey(1);
    let above = fx.draw("demo:views", &view, 22, 10);
    assert_eq!(&above.row_text(1)[..11], " ....h.... ");
    assert_eq!(&above.row_text(2)[..11], " ......... ");
}

#[test]
fn a_view_zoomed_out_shows_the_most_significant_cell_of_each_block() {
    let fx = Fixture::new();
    let view = room(&fx);
    // The screen's own zoom of 2: each glyph is a block of two by two.
    let frame = fx.draw("demo:far", &view, 7, 5);
    let rows: Vec<String> = (0..5).map(|row| frame.row_text(row)).collect();
    // Walls over floor, the chair over walls, the creature over all, and
    // the gate, a block like the walls beside it, where it comes first.
    assert_eq!(rows[1], " ##h## ");
    assert_eq!(rows[2], " #@..' ");
    assert_eq!(rows[3], " ##### ");
    // The camera's zoom multiplies the screen's.
    let mut far = room(&fx);
    far.camera_mut().set_zoom(2);
    let frame = fx.draw("demo:far", &far, 7, 5);
    assert_eq!(frame.row_text(2), "  @h#  ");
}

// ---------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------

/// Stands in for the binding package: answers the one expression the
/// test mod's overlay uses, from the view model's cell alone.
#[derive(Debug)]
struct Seats;

impl OverlayValues for Seats {
    fn value(&self, overlay: &Overlay, _at: Location, cell: &KnownCell) -> Option<f64> {
        assert_eq!(overlay.cell, "has_fixture(it)");
        Some(if cell.fixture.is_none() { 0.0 } else { 1.0 })
    }
}

#[test]
fn a_map_view_draws_the_overlay_it_names() {
    let fx = Fixture::new();
    let mut view = room(&fx);
    let plain = fx.draw("demo:far", &view, 7, 5);

    // The built-in `light` overlay shades every known cell by its light.
    assert_eq!(
        BuiltinOverlay::from_name("light"),
        Some(BuiltinOverlay::Light)
    );
    view.model_mut().observe_light(fx.at(1, 1, 0), 0);
    let lit = fx.draw("demo:lit", &view, 9, 5);
    let yellow = Ansi16::Yellow.rgb();
    assert_eq!(lit.get(2, 1).unwrap().bg.rgb, yellow);
    assert_eq!(lit.get(1, 1).unwrap().bg.rgb, Rgb::BLACK);
    assert_eq!(lit.get(2, 1).unwrap().ch, '.');
    view.model_mut().observe_light(fx.at(1, 1, 0), LIGHT_FULL);

    // An overlay defined by a mod draws nothing until something can
    // evaluate its expression.
    let seats = fx
        .ui
        .overlay("demo:seats", &fx.theme)
        .unwrap_or_else(|d| panic!("{}", d.render()));
    assert_eq!(seats.name.as_deref(), Some("Seats"));
    view.add_overlay(seats);
    let unevaluated = fx.draw("demo:seated", &view, 9, 5);
    assert!(
        unevaluated
            .cells
            .iter()
            .all(|cell| cell.bg == fx.theme.base().bg)
    );
    view.set_overlay_values(Some(Box::new(Seats)));
    let seated = fx.draw("demo:seated", &view, 9, 5);
    let green = Color::ansi(Ansi16::Green);
    let shaded: Vec<(usize, char)> = seated
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.bg == green)
        .map(|(index, cell)| (index, cell.ch))
        .collect();
    assert_eq!(shaded, [(9 + 4, 'h')], "only the chair's cell is shaded");

    // A screen that names no overlay is drawn as before.
    assert_eq!(fx.draw("demo:far", &view, 7, 5), plain);
    let draw = OverlayDraw::default();
    assert!(matches!(draw, OverlayDraw::None));
}

// ---------------------------------------------------------------------
// Definitions
// ---------------------------------------------------------------------

#[test]
fn glyphs_are_compiled_once_by_state() {
    let fx = Fixture::new();
    let glyphs = fx.glyphs();
    let part = |state: clash_world::ElementState, layer| glyphs.element(layer, state);
    let gate = fx.block("gate");
    let open = fx.block("gate#open");
    let block = clash_world::Layer::Block;
    assert_eq!(part(gate.raw(), block).unwrap().ch, Some('+'));
    assert_eq!(part(open.raw(), block).unwrap().ch, Some('\''));
    // Bits past the last state mean the first, as in the world's tables.
    assert_eq!(
        part(gate.with_bits(7).raw(), block),
        part(gate.raw(), block)
    );
    // No element, and an element with no glyph, are not drawn.
    assert_eq!(part(BlockState::NONE.raw(), block), None);
    assert_eq!(part(fx.block("ghost").raw(), block), None);
    // A material's colour, by name through the theme or written out.
    assert_eq!(
        glyphs.material(gate.material()),
        Some(Color::new(TAN, Ansi16::Yellow))
    );
    assert_eq!(
        glyphs.material(fx.block("wall").material()),
        Some(Color::from_rgb(STONE))
    );
    // The marks are the theme's, or plain defaults.
    assert_eq!(glyphs.marks().pile.ch, Some('%'));
    assert_eq!(glyphs.marks().heard.ch, Some('!'));
    assert_eq!(glyphs.marks().section_floor, '_');
}

#[test]
fn a_glyph_that_cannot_be_drawn_is_reported_where_it_is_written() {
    let defs = load(
        r##"[
  { "type": "material", "id": "stone", "density": 2000, "toughness": 1.0e6, "color": "octarine" },
  { "type": "block", "id": "wall", "material": "stone", "height": 3.0, "fill": 0.2,
    "glyph": { "ch": "ab", "fg": "material" } },
  { "type": "block", "id": "gate", "material": "stone", "height": 3.0, "fill": 0.1,
    "states": { "closed": {}, "open": { "glyph": { "fg": "ultraviolet" } } },
    "glyph": { "ch": "+", "bg": "infrared" } },
  { "type": "ambience", "id": "fog", "tint": "mauve", "dim": 1.5 },
  { "type": "overlay", "id": "odd", "cell": "", "shade": "own" }
]"##,
    );
    let diagnostics = clash_scene::check_defs(&defs);
    let found: Vec<(u32, String, &str)> = diagnostics
        .iter()
        .map(|diagnostic| {
            let location = diagnostic.location.as_ref().expect("a place");
            (
                location.line,
                diagnostic.path.clone(),
                diagnostic.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (2, "color".to_owned(), "unknown colour `octarine`"),
            (
                4,
                "glyph.ch".to_owned(),
                "`ab` is not one character: a glyph's `ch` is exactly one"
            ),
            (
                6,
                "states.open.glyph.fg".to_owned(),
                "unknown colour `ultraviolet`"
            ),
            (7, "glyph.bg".to_owned(), "unknown colour `infrared`"),
            (8, "tint".to_owned(), "unknown colour `mauve` in `tint`"),
            (8, "dim".to_owned(), "`dim` must be from 0 to 1, and is 1.5"),
            (
                9,
                "cell".to_owned(),
                "`cell` is empty: an overlay needs an expression over a cell"
            ),
            (
                9,
                "shade".to_owned(),
                "`own` in `shade` only means something in a glyph"
            ),
        ],
        "{}",
        diagnostics.render()
    );

    // Asked for all the same, the tables draw what they can.
    let world = WorldDefs::from_defs(&defs).unwrap();
    let (glyphs, problems) = WorldGlyphs::build(&world, &Theme::default());
    assert_eq!(problems.len(), 4);
    let gate = world.block_state("demo:gate", None).unwrap();
    let part = glyphs
        .element(clash_world::Layer::Block, gate.raw())
        .unwrap();
    assert_eq!((part.ch, part.bg), (Some('+'), None));
    assert_eq!(glyphs.material(gate.material()), None);
}

#[test]
fn an_ambience_or_overlay_that_does_not_exist_is_an_error_with_a_suggestion() {
    let fx = Fixture::new();
    let text = fx.ui.ambience("demo:dask", &fx.theme).unwrap_err().render();
    assert!(text.contains("there is no ambience `demo:dask`"), "{text}");
    assert!(text.contains("did you mean `demo:dusk`?"), "{text}");
    let text = fx.ui.overlay("seats", &fx.theme).unwrap_err().render();
    assert!(text.contains("there is no overlay `seats`"), "{text}");
    assert!(clash_scene::check_defs(fx.ui.defs()).is_empty());
}
