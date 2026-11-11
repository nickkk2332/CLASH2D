//! The interface toolkit, widget by widget: for each container and each
//! widget of 12 §10.2 and §10.3 a golden frame and a navigation test, and
//! layout fixtures at several sizes.
//!
//! Golden frames are under `tests/golden/`. Run with `BLESS=1` to write
//! them after a deliberate change, and look at the difference.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clash_core::{Real, Value};
use clash_scene::ui::{
    BindScope, Evaluator, FigureDef, MessageLog, Screen, ScreenDef, UiData, UiEnv, UiError,
    UiEvent, UiProblem, VarRef, ViewDef, ViewKind, ViewPainter, WidgetEvent,
};
use clash_scene::{
    Attrs, Canvas, CellStyle, Frame, InputEvent, KeymapDef, Mods, MouseButton, MouseEvent,
    MouseKind, Rect, RichText, Theme, ThemeDef, assert_golden,
};

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

/// What the screens under test are made with: the theme of 12 §4.3, one
/// figure and one piece of art.
struct Fixture {
    theme: Theme,
    figures: BTreeMap<String, FigureDef>,
    art: BTreeMap<String, String>,
    keymap: KeymapDef,
}

fn fixture() -> Fixture {
    let theme: ThemeDef = serde_json::from_str(
        r##"{ "colors": { "text": "#c8c8c8", "background": "#101010", "dim": "#707070",
                          "accent": "#e0b040", "blood": "#b02020" },
              "styles": { "title": { "fg": "accent", "bold": true },
                          "selected": { "fg": "#000000", "bg": "accent" } } }"##,
    )
    .unwrap();
    let figure: FigureDef = serde_json::from_str(
        r#"{ "rows": [ " O ", "/|\\", "/ \\" ], "map": [ " h ", "ata", "l l" ],
             "regions": { "h": "head", "t": "torso", "a": "arms", "l": "legs" } }"#,
    )
    .unwrap();
    Fixture {
        theme: Theme::build(&theme, &BTreeMap::new()).unwrap(),
        figures: BTreeMap::from([("body".to_owned(), figure)]),
        art: BTreeMap::from([("assets/wave.txt".to_owned(), "/\\/\\\n\\/\\/\n".to_owned())]),
        keymap: KeymapDef::fallback(),
    }
}

impl Fixture {
    fn try_screen(&self, json: &str) -> Result<Screen, Vec<UiError>> {
        let def: ScreenDef =
            serde_json::from_str(json).unwrap_or_else(|e| panic!("screen json: {e}"));
        let env = UiEnv {
            theme: &self.theme,
            figures: &self.figures,
            art: &self.art,
        };
        Screen::new("test", &def, &env)
    }

    fn screen(&self, json: &str) -> Screen {
        self.try_screen(json).unwrap_or_else(|errors| {
            let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
            panic!("screen has problems:\n{}", lines.join("\n"))
        })
    }

    /// The problems of a screen that should not load, as text.
    fn problems(&self, json: &str) -> Vec<String> {
        match self.try_screen(json) {
            Ok(_) => panic!("the screen should have problems"),
            Err(errors) => errors.iter().map(ToString::to_string).collect(),
        }
    }

    fn press(&self, screen: &mut Screen, keys: &[&str]) -> Vec<UiEvent> {
        keys.iter()
            .flat_map(|key| screen.input(&InputEvent::key(key).unwrap(), &self.keymap))
            .collect()
    }
}

fn draw(screen: &mut Screen, cols: u16, rows: u16) -> Frame {
    draw_with(screen, cols, rows, &UiData::default())
}

fn draw_with(screen: &mut Screen, cols: u16, rows: u16, data: &UiData<'_>) -> Frame {
    let mut frame = Frame::new(cols, rows);
    screen.draw(&mut frame, data);
    frame
}

fn int(screen: &Screen, name: &str) -> i64 {
    screen
        .var(name)
        .and_then(Value::as_int)
        .unwrap_or_else(|| panic!("no int `{name}`"))
}

/// A widget that cannot take the focus is passed over: the focus goes
/// from the button before it to the button after it.
fn assert_focus_passes_over(
    fix: &Fixture,
    widget: &str,
    golden_name: &str,
    cols: u16,
    rows: u16,
) -> Frame {
    let mut screen = fix.screen(&format!(
        r#"{{ "layout": {{ "rows": [ {{ "button": {{ "id": "before", "label": "Before" }} }},
                                     {widget},
                                     {{ "button": {{ "id": "after", "label": "After" }} }} ] }} }}"#
    ));
    let frame = draw(&mut screen, cols, rows);
    assert_golden(golden(golden_name), &frame);
    assert_eq!(screen.focus(), Some("before"));
    fix.press(&mut screen, &["tab"]);
    assert_eq!(
        screen.focus(),
        Some("after"),
        "the focus passes over the widget"
    );
    fix.press(&mut screen, &["tab"]);
    assert_eq!(screen.focus(), Some("before"), "and goes round");
    fix.press(&mut screen, &["shift+tab"]);
    assert_eq!(screen.focus(), Some("after"));
    frame
}

// ---------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------

const LAYOUT: &str = r#"{ "layout": { "rows": [
    { "text": "<title>Header</> fits its one line", "id": "head" },
    { "cols": [
        { "width": 14, "frame": "Fixed 14", "list": { "rows": ["one", "two", "three"] } },
        { "width": "25%", "frame": "25%", "text": "a quarter of the width, wrapped" },
        { "fill": 2, "frame": "fill 2", "rows": [
            { "bar": { "label": "bar", "value": 0.5 } },
            { "spacer": {} },
            { "text": { "value": "bottom right", "align": "right" } } ] },
        { "fill": 1, "frame": true, "text": "fill 1" } ] },
    { "height": 3, "frame": "Three rows high", "text": "footer" } ] } }"#;

#[test]
fn layout_fixture_at_several_sizes() {
    let fix = fixture();
    for (cols, rows) in [(40, 10), (60, 16), (80, 24), (100, 30)] {
        let mut screen = fix.screen(LAYOUT);
        let frame = draw(&mut screen, cols, rows);
        assert_golden(golden(&format!("layout_{cols}x{rows}")), &frame);
    }
}

#[test]
fn layout_sizes_are_cells_percent_fill_and_fit() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "cols": [
            { "id": "a", "width": 10, "spacer": {} },
            { "id": "b", "width": "25%", "spacer": {} },
            { "id": "c", "fit": true, "text": "seven.." },
            { "id": "d", "fill": 1, "spacer": {} },
            { "id": "e", "fill": 2, "spacer": {} } ] } }"#,
    );
    draw(&mut screen, 80, 5);
    let rect = |id: &str| screen.rect_of(id).unwrap();
    assert_eq!(rect("a"), Rect::new(0, 0, 10, 5));
    assert_eq!(rect("b"), Rect::new(10, 0, 20, 5));
    assert_eq!(rect("c"), Rect::new(30, 0, 7, 5));
    assert_eq!(
        rect("d"),
        Rect::new(37, 0, 15, 5),
        "43 left over: a third, and the cell lost to rounding"
    );
    assert_eq!(rect("e"), Rect::new(52, 0, 28, 5));

    // Too little room: later children get what is left, then nothing.
    draw(&mut screen, 25, 5);
    let rect = |id: &str| screen.rect_of(id).unwrap();
    assert_eq!(rect("b"), Rect::new(10, 0, 6, 5));
    assert_eq!(rect("c"), Rect::new(16, 0, 7, 5));
    assert_eq!(rect("d").w + rect("e").w, 2);
}

#[test]
fn layout_grid_and_stack() {
    let fix = fixture();
    let json = r#"{ "layout": { "stack": [
        { "grid": { "columns": [ 8, "fill", { "fill": 2 } ], "rows": [ 3, "fill" ], "cells": [
            { "frame": "a", "spacer": {} }, { "frame": "b", "spacer": {} }, { "frame": "c", "spacer": {} },
            { "frame": "d", "spacer": {} }, { "frame": "e", "text": "five" } ] } },
        { "id": "over", "width": 16, "height": 3, "frame": "stack", "text": "on top", "style": "selected" } ] } }"#;
    for (cols, rows) in [(40, 10), (61, 15)] {
        let mut screen = fix.screen(json);
        let frame = draw(&mut screen, cols, rows);
        assert_golden(golden(&format!("layout_grid_stack_{cols}x{rows}")), &frame);
        let over = screen.rect_of("over").unwrap();
        assert_eq!((over.w, over.h), (16, 3));
        assert_eq!(
            over.x,
            (i32::from(cols) - 16) / 2,
            "a sized child of a stack is centred"
        );
    }
}

#[test]
fn a_screen_smaller_than_its_minimum_shows_a_notice() {
    let fix = fixture();
    let mut screen =
        fix.screen(r#"{ "min_size": [30, 6], "layout": { "button": { "label": "OK" } } }"#);
    let frame = draw(&mut screen, 28, 4);
    assert_golden(golden("min_size_notice"), &frame);
    assert!(frame.text().contains("at least 30 by 6"));
    assert!(screen.too_small());
    assert_eq!(screen.focus_kind(), None, "nothing can be used");
    let frame = draw(&mut screen, 30, 6);
    assert_eq!(frame.row_text(0).trim_end(), "[ OK ]");
    assert_eq!(screen.focus_kind(), Some("button"));
}

#[test]
fn a_hidden_node_takes_no_space() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "rows": [ { "if": false, "text": "hidden" }, { "id": "shown", "text": "shown" },
                                   { "if": "view.alive", "text": "bound conditions show until evaluated" } ] } }"#,
    );
    let frame = draw(&mut screen, 40, 3);
    assert_eq!(frame.row_text(0).trim_end(), "shown");
    assert_eq!(
        frame.row_text(1).trim_end(),
        "bound conditions show until evaluated"
    );
}

// ---------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------

#[test]
fn tabs_show_one_page_at_a_time() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "vars": { "page": 1 },
             "layout": { "frame": "Tabs", "tabs": { "selected": "page", "pages": [
                { "title": "One", "content": { "button": { "id": "first", "label": "On page one" } } },
                { "title": "Two", "content": { "text": "The second page has nothing to focus." } },
                { "title": "Three", "content": { "list": { "id": "third", "rows": ["x", "y"] } } } ] } } }"#,
    );
    let frame = draw(&mut screen, 44, 6);
    assert_golden(golden("tabs_page_two"), &frame);
    assert_eq!(
        screen.focus_kind(),
        Some("tabs"),
        "the page is set by the variable"
    );
    assert!(frame.text().contains("The second page"));

    let events = fix.press(&mut screen, &["right"]);
    assert_eq!(int(&screen, "page"), 2);
    assert_eq!(
        events,
        [UiEvent::Changed {
            var: VarRef::Screen("page".into())
        }]
    );
    let frame = draw(&mut screen, 44, 6);
    assert_golden(golden("tabs_page_three"), &frame);

    fix.press(&mut screen, &["tab"]);
    assert_eq!(
        screen.focus(),
        Some("third"),
        "the focus goes into the page shown"
    );
    fix.press(&mut screen, &["tab", "right"]);
    assert_eq!(int(&screen, "page"), 0, "pages go round");
    fix.press(&mut screen, &["left"]);
    assert_eq!(int(&screen, "page"), 2);

    assert!(screen.set_var("page", Value::Int(0)));
    let frame = draw(&mut screen, 44, 6);
    assert!(frame.text().contains("[ On page one ]"));
    assert!(!screen.set_var("nope", Value::Int(0)));
}

const SCROLL: &str = r#"{ "layout": { "frame": "Scroll", "scroll": { "rows": [
    { "button": { "id": "top", "label": "Top" } },
    { "text": "one\ntwo\nthree\nfour\nfive\nsix" },
    { "button": { "id": "bottom", "label": "Bottom" } } ] } } }"#;

#[test]
fn scroll_follows_the_focus_and_pages() {
    let fix = fixture();
    let mut screen = fix.screen(SCROLL);
    let frame = draw(&mut screen, 20, 6);
    assert_golden(golden("scroll_top"), &frame);
    assert!(frame.text().contains("[ Top ]") && !frame.text().contains("Bottom"));

    fix.press(&mut screen, &["tab"]);
    assert_eq!(screen.focus(), Some("bottom"));
    let frame = draw(&mut screen, 20, 6);
    assert_golden(golden("scroll_bottom"), &frame);
    assert!(
        frame.text().contains("[ Bottom ]"),
        "moving the focus scrolled to it"
    );
    assert!(!frame.text().contains("Top"));

    fix.press(&mut screen, &["home"]);
    let frame = draw(&mut screen, 20, 6);
    assert!(
        frame.text().contains("[ Top ]"),
        "home scrolls the scroll around the focus"
    );
    fix.press(&mut screen, &["page_down"]);
    let frame = draw(&mut screen, 20, 6);
    assert!(frame.text().contains("five") && !frame.text().contains("[ Top ]"));
}

#[test]
fn a_scroll_with_nothing_to_focus_takes_the_focus_itself() {
    let fix = fixture();
    let mut screen =
        fix.screen(r#"{ "layout": { "scroll": { "text": "1\n2\n3\n4\n5\n6\n7\n8" } } }"#);
    let frame = draw(&mut screen, 10, 3);
    assert_eq!(screen.focus_kind(), Some("scroll"));
    assert_eq!(frame.row_text(0).trim_end(), "1        █");
    fix.press(&mut screen, &["down", "down"]);
    let frame = draw(&mut screen, 10, 3);
    assert_eq!(frame.row_text(0).chars().next(), Some('3'));
    fix.press(&mut screen, &["end"]);
    let frame = draw(&mut screen, 10, 3);
    assert_eq!(frame.row_text(2).chars().next(), Some('8'));
    assert_golden(golden("scroll_text_end"), &frame);

    // Content that fits needs no scrolling and takes no focus.
    let mut fits = fix.screen(r#"{ "layout": { "scroll": { "text": "short" } } }"#);
    draw(&mut fits, 10, 3);
    assert_eq!(fits.focus_kind(), None);
}

#[test]
fn a_slot_is_named_and_empty() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "rows": [ { "text": "above" }, { "slot": "hud.bars" }, { "text": "below" } ] } }"#,
    );
    let frame = draw(&mut screen, 10, 3);
    assert_eq!(screen.slots().collect::<Vec<_>>(), ["hud.bars"]);
    assert_eq!(frame.text(), "above     \nbelow     \n          \n");
}

// ---------------------------------------------------------------------
// Widgets that show
// ---------------------------------------------------------------------

#[test]
fn text_wraps_aligns_and_styles() {
    let fix = fixture();
    let frame = assert_focus_passes_over(
        &fix,
        r#"{ "frame": "text", "text": { "value": "Wrapped text with <blood>a styled part</> and a << bracket.", "align": "centre" } }"#,
        "widget_text",
        24,
        8,
    );
    assert_eq!(frame.row_text(2), "│ Wrapped text with a  │");
    let styled = frame.get(2, 3).unwrap();
    assert_eq!(styled.fg, fix.theme.color("blood").unwrap());

    let mut cut =
        fix.screen(r#"{ "layout": { "text": { "value": "not wrapped at all", "wrap": false } } }"#);
    assert_eq!(draw(&mut cut, 10, 2).text(), "not wrappe\n          \n");
}

#[test]
fn bar_and_gauge_show_a_value_against_a_maximum() {
    let fix = fixture();
    let frame = assert_focus_passes_over(
        &fix,
        r#"{ "rows": [ { "bar": { "label": "Blood", "value": 0.5, "style": "blood" } },
                       { "bar": { "value": 3, "max": 4 } },
                       { "bar": { "label": "Over", "value": 9, "max": 4 } },
                       { "bar": { "label": "None", "value": 1, "max": 0 } },
                       { "gauge": { "label": "Charge", "value": 37, "max": 120 } },
                       { "gauge": { "label": "Half", "value": 0.5, "max": 1 } } ] }"#,
        "widget_bar_gauge",
        26,
        8,
    );
    assert_eq!(frame.row_text(1), "Blood ██████████░░░░░░░░░░");
    assert_eq!(
        frame.row_text(2),
        format!("{}{}", "█".repeat(20), "░".repeat(6))
    );
    assert_eq!(frame.row_text(3), format!("Over {}", "█".repeat(21)));
    assert_eq!(frame.row_text(4), format!("None {}", "░".repeat(21)));
    assert_eq!(frame.row_text(5).trim_end(), "Charge 37/120");
    assert_eq!(frame.row_text(6).trim_end(), "Half 0.5/1");
    assert_eq!(
        frame.get(6, 1).unwrap().fg,
        fix.theme.color("blood").unwrap(),
        "a bar's style colours its fill"
    );
    assert_eq!(
        frame.get(0, 1).unwrap().fg,
        fix.theme.base().fg,
        "and not its label"
    );
}

#[test]
fn figure_colours_its_regions() {
    let fix = fixture();
    let frame = assert_focus_passes_over(
        &fix,
        r#"{ "figure": { "figure": "body", "shade": { "head": "blood", "legs": "accent" } } }"#,
        "widget_figure",
        12,
        5,
    );
    assert_eq!(frame.row_text(1).trim_end(), " O");
    assert_eq!(frame.row_text(2).trim_end(), "/|\\");
    assert_eq!(
        frame.get(1, 1).unwrap().fg,
        fix.theme.color("blood").unwrap()
    );
    assert_eq!(
        frame.get(0, 3).unwrap().fg,
        fix.theme.color("accent").unwrap()
    );
    assert_eq!(
        frame.get(1, 2).unwrap().fg,
        fix.theme.base().fg,
        "a region with no shade keeps the text colour"
    );
}

#[test]
fn art_draws_an_asset_as_written() {
    let fix = fixture();
    let frame = assert_focus_passes_over(
        &fix,
        r#"{ "art": { "value": "assets/wave.txt", "style": "accent" } }"#,
        "widget_art",
        12,
        4,
    );
    assert_eq!(frame.row_text(1).trim_end(), "/\\/\\");
    assert_eq!(frame.row_text(2).trim_end(), "\\/\\/");
    assert_eq!(
        frame.get(0, 1).unwrap().fg,
        fix.theme.color("accent").unwrap()
    );
}

#[test]
fn spacer_takes_what_is_left() {
    let fix = fixture();
    assert_focus_passes_over(&fix, r#"{ "spacer": {} }"#, "widget_spacer", 12, 5);
    let mut screen = fix.screen(
        r#"{ "layout": { "cols": [ { "text": "L" }, { "spacer": {} }, { "text": "R" } ] } }"#,
    );
    assert_eq!(draw(&mut screen, 9, 1).text(), "L       R\n");
}

#[test]
fn tooltip_shows_the_tip_of_what_has_the_focus() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "rows": [
            { "button": { "id": "a", "label": "A", "tip": "The first button." } },
            { "button": { "id": "b", "label": "B" } },
            { "list": { "id": "l", "tip": "A list.", "rows": [ { "label": "x", "tip": "Row x." }, "y" ] } },
            { "tooltip": { "value": "Nothing to say." } } ] } }"#,
    );
    let frame = draw(&mut screen, 24, 6);
    assert_golden(golden("widget_tooltip"), &frame);
    assert_eq!(frame.row_text(5).trim_end(), "The first button.");
    fix.press(&mut screen, &["tab"]);
    assert_eq!(
        draw(&mut screen, 24, 6).row_text(5).trim_end(),
        "Nothing to say."
    );
    assert_eq!(
        screen.focus(),
        Some("b"),
        "the tooltip itself is passed over"
    );
    fix.press(&mut screen, &["tab"]);
    assert_eq!(
        draw(&mut screen, 24, 6).row_text(5).trim_end(),
        "Row x.",
        "a row's own tip wins"
    );
    fix.press(&mut screen, &["down"]);
    assert_eq!(draw(&mut screen, 24, 6).row_text(5).trim_end(), "A list.");
    fix.press(&mut screen, &["tab"]);
    assert_eq!(screen.focus(), Some("a"));

    // The pointer over something with a tip shows it when the focus has none.
    fix.press(&mut screen, &["tab"]);
    let hover = MouseEvent {
        kind: MouseKind::Move,
        col: 1,
        row: 0,
        mods: Mods::empty(),
    };
    screen.input(&InputEvent::Mouse(hover), &fix.keymap);
    assert_eq!(
        draw(&mut screen, 24, 6).row_text(5).trim_end(),
        "The first button."
    );
}

// ---------------------------------------------------------------------
// Rows: list, menu, tree, table, inspect
// ---------------------------------------------------------------------

const LIST: &str = r#"{ "vars": { "row": 0 },
    "layout": { "frame": "Items", "list": { "id": "items", "selected": "row",
        "rows": ["Bandage", "Candle", "Chalk", "Crowbar", "Lantern", "Notebook", "Rope", "Whistle"],
        "on_select": [ { "set": { "screen.seen": "it" } } ],
        "on_activate": [ { "close": {} } ] } } }"#;

#[test]
fn list_selects_scrolls_and_activates() {
    let fix = fixture();
    let mut screen = fix.screen(LIST);
    let frame = draw(&mut screen, 16, 6);
    assert_golden(golden("widget_list"), &frame);
    assert_eq!(frame.row_text(1), "│Bandage      █│");
    assert_eq!(
        frame.get(1, 1).unwrap().bg,
        fix.theme.color("accent").unwrap(),
        "the selection is drawn selected"
    );

    let events = fix.press(&mut screen, &["down"]);
    assert_eq!(screen.selected_row("items"), Some(1));
    assert_eq!(int(&screen, "row"), 1);
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0],
        UiEvent::Changed {
            var: VarRef::Screen("row".into())
        }
    );
    assert!(
        matches!(&events[1], UiEvent::Widget { id: Some(id), event: WidgetEvent::Select, row: Some(1), effects }
        if id == "items" && effects.names().eq(["set"]))
    );

    fix.press(&mut screen, &["down", "down", "down", "down"]);
    assert_eq!(screen.selected_row("items"), Some(5));
    let frame = draw(&mut screen, 16, 6);
    assert_golden(golden("widget_list_scrolled"), &frame);
    assert_eq!(
        frame.row_text(4),
        "│Notebook     ││",
        "the list scrolled to keep the selection in view"
    );

    fix.press(&mut screen, &["end"]);
    assert_eq!(screen.selected_row("items"), Some(7));
    assert!(
        fix.press(&mut screen, &["down"]).is_empty(),
        "the end is the end"
    );
    fix.press(&mut screen, &["page_up"]);
    assert_eq!(screen.selected_row("items"), Some(3));
    fix.press(&mut screen, &["home"]);
    assert_eq!(screen.selected_row("items"), Some(0));
    fix.press(&mut screen, &["up", "page_down"]);
    assert_eq!(screen.selected_row("items"), Some(4));

    let events = fix.press(&mut screen, &["enter"]);
    assert_eq!(events.len(), 1);
    assert!(events[0].asks_for("close"));
    assert!(matches!(
        &events[0],
        UiEvent::Widget {
            event: WidgetEvent::Activate,
            row: Some(4),
            ..
        }
    ));

    assert!(screen.set_var("row", Value::Int(6)));
    assert_eq!(
        screen.selected_row("items"),
        Some(6),
        "the variable moves the selection"
    );
}

#[test]
fn list_with_no_rows_shows_its_empty_text() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "rows": [ { "frame": "Wounds", "list": { "rows": [], "empty": "Unhurt." } },
                                   { "frame": "Query", "list": { "rows": { "wounds": { "of": "$who" } },
                                                                 "item": "{it.kind.name}", "empty": "Nothing yet." } } ] } }"#,
    );
    let frame = draw(&mut screen, 16, 6);
    assert_golden(golden("widget_list_empty"), &frame);
    assert_eq!(frame.row_text(1), "│Unhurt.       │");
    assert_eq!(
        frame.row_text(4),
        "│Nothing yet.  │",
        "rows from a query are empty until bindings exist"
    );
    assert_eq!(screen.focus_kind(), None, "an empty list takes no focus");
}

#[test]
fn menu_options_have_hotkeys() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "rows": [
            { "input": { "id": "name", "bind": "name" } },
            { "frame": "Menu", "menu": { "id": "menu",
                "rows": [ { "label": "New game", "key": "n", "on_activate": [ { "open": "new_game" } ] },
                          { "label": "Load", "key": "l", "on_activate": [ { "open": "load" } ] },
                          { "label": "Quit", "key": "ctrl+q", "on_activate": [ { "app": "quit" } ] } ],
                "on_activate": [ { "close": {} } ] } } ] },
             "vars": { "name": "" } }"#,
    );
    fix.press(&mut screen, &["tab"]);
    let frame = draw(&mut screen, 20, 6);
    assert_golden(golden("widget_menu"), &frame);
    assert_eq!(frame.row_text(2), "│n      New game   │");
    assert_eq!(frame.row_text(4), "│ctrl+q Quit       │");

    fix.press(&mut screen, &["down"]);
    assert_eq!(screen.selected_row("menu"), Some(1));
    let events = fix.press(&mut screen, &["enter"]);
    assert_eq!(events.len(), 2, "the option's own effects, then the menu's");
    assert!(events[0].asks_for("open") && events[1].asks_for("close"));

    let events = fix.press(&mut screen, &["n"]);
    assert_eq!(
        screen.selected_row("menu"),
        Some(0),
        "a hotkey selects its option"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::Widget {
            event: WidgetEvent::Activate,
            row: Some(0),
            ..
        }
    )));

    // With the focus in a text field, a letter is typed, not taken as a hotkey.
    fix.press(&mut screen, &["shift+tab"]);
    assert_eq!(screen.focus(), Some("name"));
    let events = fix.press(&mut screen, &["l"]);
    assert_eq!(screen.var("name"), Some(&Value::Str("l".into())));
    assert!(!events.iter().any(|e| e.asks_for("open")));
    let events = fix.press(&mut screen, &["ctrl+q"]);
    assert!(
        events.iter().any(|e| e.asks_for("app")),
        "a hotkey that types nothing still works"
    );
    assert_eq!(screen.focus(), Some("menu"));
}

const TREE: &str = r#"{ "layout": { "frame": "Body", "tree": { "id": "tree", "rows": [
    { "label": "Head", "children": [ "Left eye", "Right eye" ] },
    { "label": "Torso", "open": true, "children": [ { "label": "Heart" }, { "label": "Lungs", "children": ["Left lung"] } ] },
    "Tail" ],
    "on_activate": [ { "open": "part" } ] } } }"#;

#[test]
fn tree_opens_and_closes_branches() {
    let fix = fixture();
    let mut screen = fix.screen(TREE);
    let frame = draw(&mut screen, 20, 9);
    assert_golden(golden("widget_tree"), &frame);
    assert_eq!(frame.row_text(1), "│▸ Head            │");
    assert_eq!(frame.row_text(2), "│▾ Torso           │");
    assert_eq!(frame.row_text(3), "│    Heart         │");

    // Rows are numbered through the whole tree as written: Head 0, Left eye 1,
    // Right eye 2, Torso 3, Heart 4, Lungs 5, Left lung 6, Tail 7.
    fix.press(&mut screen, &["right"]);
    let frame = draw(&mut screen, 20, 9);
    assert_golden(golden("widget_tree_opened"), &frame);
    assert_eq!(frame.row_text(2), "│    Left eye      │");
    fix.press(&mut screen, &["right"]);
    assert_eq!(
        screen.selected_row("tree"),
        Some(1),
        "right on an open branch goes to its first child"
    );
    fix.press(&mut screen, &["left"]);
    assert_eq!(
        screen.selected_row("tree"),
        Some(0),
        "left on a leaf goes to its parent"
    );
    fix.press(&mut screen, &["left"]);
    assert_eq!(
        draw(&mut screen, 20, 9).row_text(2),
        "│▾ Torso           │",
        "left closes an open branch"
    );

    fix.press(&mut screen, &["down", "down"]);
    assert_eq!(screen.selected_row("tree"), Some(4));
    let events = fix.press(&mut screen, &["enter"]);
    assert!(matches!(
        &events[..],
        [UiEvent::Widget {
            event: WidgetEvent::Activate,
            row: Some(4),
            ..
        }]
    ));
    fix.press(&mut screen, &["down"]);
    assert!(
        fix.press(&mut screen, &["enter"]).is_empty(),
        "accept on a branch opens it instead"
    );
    assert_eq!(draw(&mut screen, 20, 9).row_text(5), "│      Left lung   │");
    fix.press(&mut screen, &["end"]);
    assert_eq!(screen.selected_row("tree"), Some(7));
}

const TABLE: &str = r#"{ "vars": { "row": 2 },
    "layout": { "frame": "Pack", "table": { "id": "pack", "selected": "row",
        "columns": [ { "title": "Item", "width": "fill" },
                     { "title": "Count", "align": "right" },
                     { "title": "Mass", "width": 6, "align": "centre" } ],
        "rows": [ ["Rope", "1", "3.0"], ["Arrow", "24", "1.2"], ["Bandage", "3", "0.1"], ["Lantern", "10", "0.9"] ] } } }"#;

#[test]
fn table_sorts_by_column() {
    let fix = fixture();
    let mut screen = fix.screen(TABLE);
    let frame = draw(&mut screen, 30, 7);
    assert_golden(golden("widget_table"), &frame);
    assert_eq!(frame.row_text(1), "│Item          Count   Mass  │");
    assert_eq!(frame.row_text(2), "│Rope                1  3.0  │");
    assert_eq!(
        screen.selected_row("pack"),
        Some(2),
        "the variable set the selection"
    );
    assert!(
        frame.get(1, 1).unwrap().attrs.contains(Attrs::BOLD),
        "headers are drawn as headers"
    );

    fix.press(&mut screen, &["right"]);
    let frame = draw(&mut screen, 30, 7);
    assert_golden(golden("widget_table_sorted"), &frame);
    assert_eq!(frame.row_text(1), "│Item ▲        Count   Mass  │");
    assert_eq!(frame.row_text(2), "│Arrow              24  1.2  │");
    assert_eq!(
        screen.selected_row("pack"),
        Some(2),
        "sorting keeps the same row selected"
    );

    fix.press(&mut screen, &["right"]);
    let frame = draw(&mut screen, 30, 7);
    assert_eq!(
        frame.row_text(2),
        "│Rope                1  3.0  │",
        "numbers sort as numbers: 1, 3, 10, 24"
    );
    assert_eq!(frame.row_text(5), "│Arrow              24  1.2  │");
    fix.press(&mut screen, &["space"]);
    let frame = draw(&mut screen, 30, 7);
    assert_eq!(
        frame.row_text(2),
        "│Arrow              24  1.2  │",
        "toggle reverses the order"
    );
    fix.press(&mut screen, &["right", "right"]);
    let frame = draw(&mut screen, 30, 7);
    assert_eq!(
        frame.row_text(2),
        "│Rope                1  3.0  │",
        "past the last column is unsorted again"
    );

    fix.press(&mut screen, &["down"]);
    assert_eq!(int(&screen, "row"), 3);
}

#[test]
fn inspect_browses_properties() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "frame": "Mara", "inspect": { "id": "props", "rows": [
            { "label": "name", "value": "Mara" },
            { "label": "pools", "children": [ { "label": "blood", "value": "0.62" }, { "label": "breath", "value": "0.9" } ] },
            { "label": "alive", "value": "true" } ] } } }"#,
    );
    let frame = draw(&mut screen, 22, 7);
    assert_golden(golden("widget_inspect"), &frame);
    assert_eq!(frame.row_text(1), "│  name: Mara        │");
    assert_eq!(frame.row_text(2), "│▸ pools             │");
    fix.press(&mut screen, &["down", "right"]);
    let frame = draw(&mut screen, 22, 7);
    assert_golden(golden("widget_inspect_opened"), &frame);
    assert_eq!(frame.row_text(3), "│    blood: 0.62     │");
    fix.press(&mut screen, &["down", "down", "down"]);
    assert_eq!(screen.selected_row("props"), Some(4));
}

// ---------------------------------------------------------------------
// Widgets that edit
// ---------------------------------------------------------------------

#[test]
fn input_edits_a_text() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "vars": { "name": "Mara" },
             "layout": { "rows": [ { "input": { "id": "name", "label": "Name", "bind": "name", "max_len": 8,
                                                "on_change": [ { "run": "rename" } ] } },
                                   { "button": { "id": "ok", "label": "OK" } } ] },
             "keys": { "back": [ { "close": {} } ] } }"#,
    );
    let keymap: KeymapDef = serde_json::from_str(
        r#"{ "menu": { "q": "back", "escape": "back", "left": "left", "right": "right", "home": "first",
                       "end": "last", "backspace": "erase", "delete": "delete", "tab": "focus_next" } }"#,
    )
    .unwrap();
    let press = |screen: &mut Screen, keys: &[&str]| -> Vec<UiEvent> {
        keys.iter()
            .flat_map(|key| screen.input(&InputEvent::key(key).unwrap(), &keymap))
            .collect()
    };
    let text = |screen: &Screen| {
        screen
            .var("name")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned()
    };

    let frame = draw(&mut screen, 24, 3);
    assert_golden(golden("widget_input"), &frame);
    assert_eq!(frame.row_text(0).trim_end(), "Name Mara");
    assert!(
        frame.get(9, 0).unwrap().attrs.contains(Attrs::REVERSE),
        "the cursor is after the text"
    );

    let events = press(&mut screen, &["q"]);
    assert_eq!(
        text(&screen),
        "Maraq",
        "a key that types goes into the field, whatever it is named"
    );
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0],
        UiEvent::Changed {
            var: VarRef::Screen("name".into())
        }
    );
    assert!(events[1].asks_for("run"));

    press(&mut screen, &["backspace", "home", "shift+a", "space"]);
    assert_eq!(text(&screen), "A Mara");
    press(&mut screen, &["delete", "end", "left", "backspace"]);
    assert_eq!(
        text(&screen),
        "A aa",
        "delete at the cursor, erase before it"
    );
    press(&mut screen, &["right", "x", "y", "z", "w", "v"]);
    assert_eq!(text(&screen), "A aaxyzw", "no more than max_len characters");
    screen.input(&InputEvent::Text("pasted".into()), &keymap);
    assert_eq!(text(&screen), "A aaxyzw");
    press(
        &mut screen,
        &["backspace", "backspace", "backspace", "backspace"],
    );
    screen.input(&InputEvent::Text("é!".into()), &keymap);
    assert_eq!(text(&screen), "A aaé!");

    let events = press(&mut screen, &["escape"]);
    assert!(
        events[0].asks_for("close"),
        "a key that types nothing reaches the screen's keys"
    );
    press(&mut screen, &["tab"]);
    assert_eq!(screen.focus(), Some("ok"));
    assert!(
        press(&mut screen, &["q"])[0].asks_for("close"),
        "away from the field, q is `back` again"
    );
}

#[test]
fn number_steps_within_its_limits() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "vars": { "level": 9, "share": 0.5 },
             "layout": { "rows": [
                { "number": { "id": "level", "label": "Level", "bind": "level", "min": 1, "max": 10 } },
                { "number": { "id": "share", "label": "Share", "bind": "share", "step": 0.25, "min": 0, "max": 1 } } ] } }"#,
    );
    let frame = draw(&mut screen, 20, 3);
    assert_golden(golden("widget_number"), &frame);
    assert_eq!(frame.row_text(0).trim_end(), "Level < 9 >");
    assert_eq!(frame.row_text(1).trim_end(), "Share < 0.5 >");

    let events = fix.press(&mut screen, &["right"]);
    assert_eq!(screen.var("level"), Some(&Value::Int(10)));
    assert_eq!(
        events,
        [UiEvent::Changed {
            var: VarRef::Screen("level".into())
        }]
    );
    assert!(
        fix.press(&mut screen, &["right"]).is_empty(),
        "the maximum holds"
    );
    fix.press(&mut screen, &["left", "left", "left"]);
    assert_eq!(
        screen.var("level"),
        Some(&Value::Int(7)),
        "a whole number stays whole"
    );

    fix.press(&mut screen, &["tab", "right", "right", "right"]);
    assert_eq!(screen.var("share"), Some(&Value::Real(Real::of(1.0))));
    fix.press(&mut screen, &["left"]);
    assert_eq!(screen.var("share"), Some(&Value::Real(Real::of(0.75))));
    assert_eq!(
        draw(&mut screen, 20, 3).row_text(1).trim_end(),
        "Share < 0.8 >"
    );
}

#[test]
fn toggle_flips_a_flag() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "vars": { "sound": false },
             "layout": { "rows": [ { "toggle": { "id": "sound", "label": "Sound", "bind": "sound" } },
                                   { "toggle": { "id": "hints", "label": "Hints", "bind": "pref.hints" } } ] } }"#,
    );
    let frame = draw(&mut screen, 14, 2);
    assert_golden(golden("widget_toggle"), &frame);
    assert_eq!(frame.text(), "[ ] Sound     \n[ ] Hints     \n");
    let events = fix.press(&mut screen, &["space"]);
    assert_eq!(screen.var("sound"), Some(&Value::Bool(true)));
    assert_eq!(
        events,
        [UiEvent::Changed {
            var: VarRef::Screen("sound".into())
        }]
    );
    fix.press(&mut screen, &["enter", "enter"]);
    assert_eq!(draw(&mut screen, 14, 2).row_text(0), "[x] Sound     ");

    // A preference is kept beside the screen's variables.
    let events = fix.press(&mut screen, &["tab", "space"]);
    assert_eq!(
        events,
        [UiEvent::Changed {
            var: VarRef::Pref("hints".into())
        }]
    );
    assert_eq!(screen.prefs().get("hints"), Some(&Value::Bool(true)));
    screen.set_pref("hints", Value::Bool(false));
    assert_eq!(draw(&mut screen, 14, 2).row_text(1), "[ ] Hints     ");
}

#[test]
fn choice_goes_through_its_options() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "vars": { "speed": "normal", "size": 2 },
             "layout": { "rows": [
                { "choice": { "id": "speed", "label": "Speed", "bind": "speed", "options": ["slow", "normal", "fast"] } },
                { "choice": { "id": "size", "label": "Size", "bind": "size",
                              "options": [ { "value": 1, "label": "Small" }, { "value": 2, "label": "Large" } ] } } ] } }"#,
    );
    let frame = draw(&mut screen, 20, 2);
    assert_golden(golden("widget_choice"), &frame);
    assert_eq!(frame.text(), "Speed < normal >    \nSize < Large >      \n");
    fix.press(&mut screen, &["right"]);
    assert_eq!(screen.var("speed"), Some(&Value::Str("fast".into())));
    fix.press(&mut screen, &["right"]);
    assert_eq!(
        screen.var("speed"),
        Some(&Value::Str("slow".into())),
        "the options go round"
    );
    fix.press(&mut screen, &["left", "left"]);
    assert_eq!(screen.var("speed"), Some(&Value::Str("normal".into())));
    let events = fix.press(&mut screen, &["tab", "enter"]);
    assert_eq!(screen.var("size"), Some(&Value::Int(1)));
    assert_eq!(
        events,
        [UiEvent::Changed {
            var: VarRef::Screen("size".into())
        }]
    );
    assert_eq!(
        draw(&mut screen, 20, 2).row_text(1).trim_end(),
        "Size < Small >"
    );
}

#[test]
fn button_activates() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "layout": { "cols": [
            { "button": { "id": "ok", "label": "OK", "on_activate": [ { "close": {} } ] } },
            { "button": { "id": "more", "label": "More" } } ] } }"#,
    );
    let frame = draw(&mut screen, 16, 1);
    assert_golden(golden("widget_button"), &frame);
    assert_eq!(frame.text(), "[ OK ][ More ]  \n");
    assert_eq!(
        frame.get(0, 0).unwrap().bg,
        fix.theme.color("accent").unwrap(),
        "the focused button is drawn selected"
    );
    assert_eq!(frame.get(6, 0).unwrap().bg, fix.theme.base().bg);

    let events = fix.press(&mut screen, &["enter"]);
    assert!(
        matches!(&events[..], [UiEvent::Widget { id: Some(id), event: WidgetEvent::Activate, row: None, .. }] if id == "ok")
    );
    assert!(events[0].asks_for("close"));
    let events = fix.press(&mut screen, &["right", "space"]);
    assert_eq!(
        screen.focus(),
        Some("more"),
        "an arrow no widget used moves the focus"
    );
    assert!(matches!(&events[..], [UiEvent::Widget { id: Some(id), .. }] if id == "more"));
}

// ---------------------------------------------------------------------
// Log, views, development widgets
// ---------------------------------------------------------------------

#[test]
fn log_shows_the_latest_lines_and_scrolls_back() {
    let fix = fixture();
    let mut log = MessageLog::new();
    for n in 1..=9 {
        let category = if n % 3 == 0 { "speech" } else { "combat" };
        let line = RichText::parse(&format!("<blood>{category}</> line {n}"), &fix.theme).unwrap();
        log.push(category, line);
    }
    let data = UiData {
        log: Some(&log),
        views: None,
    };
    let mut screen = fix.screen(
        r#"{ "layout": { "cols": [ { "frame": "All", "log": { "id": "all" } },
                                   { "frame": "Speech", "log": { "id": "speech", "filter": ["speech"] } } ] } }"#,
    );
    let frame = draw_with(&mut screen, 40, 6, &data);
    assert_golden(golden("widget_log"), &frame);
    assert_eq!(
        frame.row_text(1),
        "│speech line 6    │││speech line 3     │"
    );
    assert_eq!(
        frame.row_text(3),
        "│combat line 8    │││speech line 9     │"
    );
    assert_eq!(
        frame.row_text(4),
        "│speech line 9    █││                  │"
    );
    assert_eq!(
        frame.get(1, 1).unwrap().fg,
        fix.theme.color("blood").unwrap()
    );

    fix.press(&mut screen, &["up", "up"]);
    let frame = draw_with(&mut screen, 40, 6, &data);
    assert!(
        frame.row_text(1).starts_with("│combat line 4"),
        "scrolled back two lines"
    );
    fix.press(&mut screen, &["home"]);
    let frame = draw_with(&mut screen, 40, 6, &data);
    assert_golden(golden("widget_log_oldest"), &frame);
    assert!(frame.row_text(1).starts_with("│combat line 1"));
    fix.press(&mut screen, &["down"]);
    assert!(
        draw_with(&mut screen, 40, 6, &data)
            .row_text(1)
            .starts_with("│combat line 2")
    );
    fix.press(&mut screen, &["end"]);
    assert!(
        draw_with(&mut screen, 40, 6, &data)
            .row_text(4)
            .starts_with("│speech line 9")
    );

    // With no log at hand the widget is empty, not an error.
    let frame = draw(&mut screen, 40, 6);
    assert_eq!(
        frame.row_text(1),
        "│                  ││                  │"
    );
}

const VIEWS: &str = r#"{ "layout": { "grid": { "columns": ["fill", "fill"], "rows": ["fill", "fill"], "cells": [
    { "frame": "map", "map_view": { "id": "map", "zoom": 2, "overlay": "danger" } },
    { "frame": "section", "section_view": { "id": "section" } },
    { "frame": "region", "region_map": { "id": "region" } },
    { "frame": "chart", "chart_view": { "id": "chart", "on_pick": [ { "set": { "screen.to": "it" } } ] } } ] } } }"#;

#[test]
fn views_have_their_place_and_take_the_focus() {
    let fix = fixture();
    let mut screen = fix.screen(VIEWS);
    let frame = draw(&mut screen, 40, 10);
    assert_golden(golden("widget_views"), &frame);
    assert_eq!(
        frame.row_text(2),
        "│·····map_view·····││···section_view···│"
    );
    let mut seen = vec![screen.focus().unwrap().to_owned()];
    for _ in 0..4 {
        fix.press(&mut screen, &["tab"]);
        seen.push(screen.focus().unwrap().to_owned());
    }
    assert_eq!(seen, ["map", "section", "region", "chart", "map"]);
    assert_eq!(screen.focus_kind(), Some("map_view"));

    // A painter draws them when there is one.
    struct Letters;
    impl ViewPainter for Letters {
        fn paint(
            &self,
            kind: ViewKind,
            def: &ViewDef,
            focused: bool,
            area: Rect,
            canvas: &mut Canvas<'_>,
        ) {
            let ch = if focused {
                'F'
            } else {
                kind.name().chars().next().unwrap_or('?')
            };
            canvas.fill(area, ch, CellStyle::default());
            if def.overlay.is_some() {
                canvas.put(area.x, area.y, '!', CellStyle::default());
            }
            // Drawing outside the area is clipped away.
            canvas.fill(Rect::new(0, 0, 100, 100), '#', CellStyle::default());
        }
    }
    let data = UiData {
        log: None,
        views: Some(&Letters),
    };
    let frame = draw_with(&mut screen, 40, 10, &data);
    assert_golden(golden("widget_views_painted"), &frame);
    assert_eq!(
        frame.row_text(1),
        "│##################││##################│"
    );
    assert_eq!(
        frame.row_text(0),
        "┌ map ─────────────┐┌ section ─────────┐"
    );
}

/// One screen with every container and widget, for trying sizes and keys.
const EVERYTHING: &str = r#"{ "vars": { "page": 0, "name": "x", "n": 1, "on": true, "pick": "a" },
    "layout": { "rows": [
        { "tabs": { "selected": "page", "pages": [
            { "title": "A", "content": { "cols": [
                { "frame": "l", "list": { "rows": ["a", "b", "c", "d", "e", "f"] } },
                { "frame": "t", "tree": { "rows": [ { "label": "p", "open": true, "children": ["q", "r"] } ] } },
                { "frame": "m", "menu": { "rows": [ { "label": "go", "key": "g" } ] } } ] } },
            { "title": "B", "content": { "scroll": { "rows": [
                { "table": { "columns": [ { "title": "A" }, { "title": "B", "width": "fill" } ],
                             "rows": [ ["1", "x"], ["2", "y"], ["3", "z"] ] } },
                { "input": { "bind": "name", "label": "Name" } },
                { "number": { "bind": "n" } }, { "toggle": { "bind": "on" } },
                { "choice": { "bind": "pick", "options": ["a", "b"] } },
                { "bar": { "value": 0.3, "label": "bar" } }, { "gauge": { "value": 1, "max": 3 } },
                { "figure": { "figure": "body" } }, { "art": { "value": "assets/wave.txt" } },
                { "button": { "label": "ok" } } ] } } },
            { "title": "C", "content": { "grid": { "columns": ["fill", "50%"], "cells": [
                { "frame": true, "map_view": {} }, { "log": {} },
                { "inspect": { "rows": [ { "label": "k", "value": "v" } ] } },
                { "stack": [ { "chart_view": {} }, { "text": "over", "width": 4, "height": 1 } ] } ] } } } ] } },
        { "tooltip": {} } ] } }"#;

#[test]
fn no_size_and_no_key_makes_a_screen_panic() {
    let fix = fixture();
    let mut log = MessageLog::new();
    for n in 0..20 {
        log.push("any", RichText::plain_text(&format!("line {n}")));
    }
    let data = UiData {
        log: Some(&log),
        views: None,
    };
    let keys = [
        "tab",
        "down",
        "right",
        "enter",
        "space",
        "end",
        "page_up",
        "left",
        "x",
        "backspace",
        "home",
        "g",
    ];
    for cols in 0..=34 {
        for rows in 0..=12 {
            let mut screen = fix.screen(EVERYTHING);
            for page in 0..3 {
                assert!(screen.set_var("page", Value::Int(page)));
                draw_with(&mut screen, cols, rows, &data);
                for key in keys {
                    fix.press(&mut screen, &[key]);
                    let click = MouseEvent {
                        kind: MouseKind::Down(MouseButton::Left),
                        col: cols / 2,
                        row: rows / 2,
                        mods: Mods::empty(),
                    };
                    screen.input(&InputEvent::Mouse(click), &fix.keymap);
                    let wheel = MouseEvent {
                        kind: MouseKind::Wheel(-2),
                        ..click
                    };
                    screen.input(&InputEvent::Mouse(wheel), &fix.keymap);
                    draw_with(&mut screen, cols, rows, &data);
                }
            }
        }
    }
}

#[test]
fn development_widgets_need_a_debug_screen() {
    let fix = fixture();
    let layout = r#""layout": { "cols": [ { "frame": "console", "console": { "id": "c" } },
                                          { "frame": "canvas", "canvas": { "id": "v" } },
                                          { "frame": "trace", "trace_view": { "id": "t" } } ] }"#;
    let mut screen = fix.screen(&format!(r#"{{ "context": "debug", {layout} }}"#));
    let frame = draw(&mut screen, 48, 5);
    assert_golden(golden("widget_debug"), &frame);
    assert_eq!(
        frame.row_text(2),
        "│···console····││····canvas····││··trace_view··│"
    );
    fix.press(&mut screen, &["tab", "tab"]);
    assert_eq!(
        screen.focus(),
        Some("t"),
        "a debug screen's keys fall back to the menu context"
    );
    assert_eq!(screen.context(), "debug");

    let problems = fix.problems(&format!("{{ {layout} }}"));
    assert_eq!(problems.len(), 3);
    assert_eq!(
        problems[0],
        "screen `test`, at `layout.cols[0].console`: `console` may only be used in a screen whose context is `debug`"
    );
}

// ---------------------------------------------------------------------
// Mistakes, the mouse, bindings
// ---------------------------------------------------------------------

#[test]
fn mistakes_in_a_screen_are_reported_with_their_paths() {
    let fix = fixture();
    let problems = fix.problems(
        r#"{ "title": "<nope>Title</>",
             "vars": { "name": "x", "count": 1 },
             "layout": { "rows": [
                { "id": "twin", "style": "glowing", "text": "a" },
                { "id": "twin", "text": "unclosed <blood>tag" },
                { "toggle": { "bind": "name" } },
                { "input": { "bind": "missing" } },
                { "number": { "bind": "screen.name" } },
                { "list": { "rows": ["a"], "selected": "name" } },
                { "figure": { "figure": "ghost" } },
                { "figure": { "figure": "body", "shade": { "wings": "blood", "head": "glowing" } } },
                { "art": { "value": "assets/none.txt" } },
                { "choice": { "bind": "count", "options": [] } },
                { "grid": { "columns": [], "cells": [] } },
                { "frame": "{view.name} <nope>x</>", "text": "templates are checked for tags too" } ] } }"#,
    );
    let expected = [
        "at `title`: unknown style or colour `nope` in a tag",
        "at `layout.rows[0].text.style`: unknown style or colour `glowing`",
        "at `layout.rows[1].text`: the id `twin` is used by more than one node",
        "at `layout.rows[1].text.value`: the tag `<blood>` is never closed with `</>`",
        "at `layout.rows[2].toggle.bind`: the variable `name` holds string, but this needs true or false",
        "at `layout.rows[3].input.bind`: the screen has no variable `missing`: declare it in `vars`",
        "at `layout.rows[4].number.bind`: the variable `screen.name` holds string, but this needs a number",
        "at `layout.rows[5].list.selected`: the variable `name` holds string, but this needs a whole number",
        "at `layout.rows[6].figure`: unknown figure `ghost`",
        "at `layout.rows[7].figure.shade`: the figure `body` is wrong: `shade` names the region `wings`, which the figure does not have",
        "at `layout.rows[7].figure.shade.head`: unknown style or colour `glowing`",
        "at `layout.rows[8].art.value`: unknown art asset `assets/none.txt`",
        "at `layout.rows[9].choice.options`: a choice needs at least one option",
        "at `layout.rows[10].grid`: a grid needs at least one column",
        "at `layout.rows[11].text.frame`: unknown style or colour `nope` in a tag",
    ];
    for line in expected {
        assert!(
            problems
                .iter()
                .any(|p| p == &format!("screen `test`, {line}")),
            "missing: {line}\nin: {problems:#?}"
        );
    }
    assert_eq!(problems.len(), expected.len(), "{problems:#?}");
    assert!(matches!(
        fix.try_screen(r#"{ "layout": { "figure": { "figure": "ghost" } } }"#)
            .unwrap_err()[0]
            .problem,
        UiProblem::UnknownFigure { .. }
    ));
}

#[test]
fn the_mouse_focuses_selects_activates_and_scrolls() {
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "vars": { "on": false },
             "layout": { "rows": [
                { "cols": [ { "button": { "id": "ok", "label": "OK" } }, { "toggle": { "id": "t", "label": "T", "bind": "on" } } ] },
                { "list": { "id": "items", "rows": ["a", "b", "c", "d", "e", "f"], "on_activate": [ { "close": {} } ] } } ] } }"#,
    );
    draw(&mut screen, 20, 4);
    let mouse = |screen: &mut Screen, kind: MouseKind, col: u16, row: u16| {
        screen.input(
            &InputEvent::Mouse(MouseEvent {
                kind,
                col,
                row,
                mods: Mods::empty(),
            }),
            &fix.keymap,
        )
    };
    let click = MouseKind::Down(MouseButton::Left);

    let events = mouse(&mut screen, click, 8, 0);
    assert_eq!(screen.focus(), Some("t"));
    assert_eq!(
        screen.var("on"),
        Some(&Value::Bool(true)),
        "a click flips a toggle"
    );
    assert_eq!(events.len(), 1);

    let events = mouse(&mut screen, click, 2, 0);
    assert!(
        matches!(&events[..], [UiEvent::Widget { id: Some(id), event: WidgetEvent::Activate, .. }] if id == "ok")
    );

    assert!(mouse(&mut screen, click, 0, 2).is_empty());
    assert_eq!(
        (screen.focus(), screen.selected_row("items")),
        (Some("items"), Some(1)),
        "a click selects a row"
    );
    assert!(
        mouse(&mut screen, click, 0, 2)[0].asks_for("close"),
        "a click on the selected row activates it"
    );

    mouse(&mut screen, MouseKind::Wheel(2), 0, 1);
    assert_eq!(screen.selected_row("items"), Some(3));
    mouse(&mut screen, MouseKind::Wheel(-9), 0, 1);
    assert_eq!(screen.selected_row("items"), Some(0));
    assert!(
        mouse(&mut screen, click, 19, 0).is_empty(),
        "a click on nothing does nothing"
    );
    assert!(mouse(&mut screen, MouseKind::Up(MouseButton::Left), 2, 0).is_empty());
}

#[test]
fn bound_properties_show_placeholders_until_an_evaluator_fills_them() {
    struct Fake;
    impl Evaluator for Fake {
        fn text(&mut self, source: &str, scope: &BindScope<'_>) -> Option<String> {
            let name = scope
                .vars
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("?");
            match source {
                "{$who.name}" => Some(format!("<blood>{name}</>")),
                "{broken}" => Some("a < b".to_owned()),
                _ => None,
            }
        }
        fn number(&mut self, source: &str, _: &BindScope<'_>) -> Option<f64> {
            (source == "$who.pool.blood.frac").then_some(0.5)
        }
        fn flag(&mut self, source: &str, scope: &BindScope<'_>) -> Option<bool> {
            (source == "screen.tab == 1").then(|| scope.screen == "nope")
        }
    }
    let fix = fixture();
    let mut screen = fix.screen(
        r#"{ "title": "{$who.name}", "vars": { "name": "Mara" },
             "layout": { "rows": [
                { "frame": "{$who.name}", "bar": { "label": "{$who.name}", "value": "$who.pool.blood.frac" } },
                { "text": "{broken}" },
                { "if": "screen.tab == 1", "text": "only on tab one" },
                { "text": "{unknown} stays as written" } ] } }"#,
    );
    let frame = draw(&mut screen, 30, 6);
    assert_golden(golden("bound_placeholders"), &frame);
    assert_eq!(frame.row_text(0), "┌ {$who.name} ───────────────┐");
    assert_eq!(
        frame.row_text(1),
        format!("│{{$who.name}} {}│", "░".repeat(16)),
        "an unevaluated number is 0"
    );
    assert_eq!(
        frame.row_text(4).trim_end(),
        "only on tab one",
        "an unevaluated condition is true"
    );

    screen.refresh(&mut Fake, &fix.theme);
    let frame = draw(&mut screen, 30, 6);
    assert_golden(golden("bound_evaluated"), &frame);
    assert_eq!(screen.title().unwrap().plain(), "Mara");
    assert_eq!(frame.row_text(0), "┌ Mara ──────────────────────┐");
    assert_eq!(
        frame.row_text(1),
        format!("│Mara {}{}│", "█".repeat(12), "░".repeat(11))
    );
    assert_eq!(
        frame.get(1, 1).unwrap().fg,
        fix.theme.color("blood").unwrap(),
        "the result's tags are styled"
    );
    assert_eq!(
        frame.row_text(3).trim_end(),
        "a < b",
        "a result with a broken tag is shown as it is"
    );
    assert_eq!(
        frame.row_text(4).trim_end(),
        "{unknown} stays as written",
        "the hidden row gave up its place"
    );
}

#[test]
fn drawing_is_the_same_every_time() {
    let fix = fixture();
    let mut first = fix.screen(LAYOUT);
    let mut second = fix.screen(LAYOUT);
    let a = draw(&mut first, 80, 24);
    let b = draw(&mut second, 80, 24);
    assert_eq!(a, b);
    // Drawing again into the same frame changes nothing and keeps its storage.
    let mut frame = a.clone();
    let cells = frame.cells.as_ptr();
    first.draw(&mut frame, &UiData::default());
    assert_eq!(frame, a);
    assert_eq!(frame.cells.as_ptr(), cells);
}
