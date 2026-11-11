//! [`Screen`]: a screen definition made ready to lay out, draw and use.
//!
//! This file holds what a screen is made of and how it is built from its
//! definition. Layout is in `layout.rs`, drawing in `draw.rs`, input in
//! `input.rs`.

use std::collections::BTreeMap;

use clash_core::{Real, Value, VarMap};
use thiserror::Error;

use crate::frame::{Attrs, Borders, Canvas, CellStyle, Rect};
use crate::input::{CONTEXT_MENU, KeyChord};
use crate::text::{RichText, TextError, tag_names};
use crate::theme::{Style, Theme};

use super::def::{
    Align, ColumnDef, FigureDef, FrameDef, Literal, NodeDef, NodeKind, OptionsDef, Prop, RowDef,
    RowsDef, ScreenDef, Size, TextProp, UiEffects, ViewDef,
};

/// The keymap context in which a screen sees truth and may use the
/// development widgets.
pub const CONTEXT_DEBUG: &str = "debug";

/// What is wrong with a screen definition, found when a screen is made
/// from it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UiProblem {
    /// A text's inline style tags are wrong.
    #[error(transparent)]
    Text(#[from] TextError),
    /// A `style` that is neither a theme style nor a colour.
    #[error("unknown style or colour `{name}`")]
    UnknownStyle {
        /// The name.
        name: String,
    },
    /// A widget names a screen variable the screen does not declare.
    #[error("the screen has no variable `{name}`: declare it in `vars`")]
    UnknownVariable {
        /// The name.
        name: String,
    },
    /// A widget is bound to a variable of the wrong kind.
    #[error("the variable `{name}` holds {found}, but this needs {expected}")]
    WrongVariableType {
        /// The name.
        name: String,
        /// What the widget needs.
        expected: &'static str,
        /// What the variable holds.
        found: &'static str,
    },
    /// A `figure` widget names a figure that is not defined.
    #[error("unknown figure `{name}`")]
    UnknownFigure {
        /// The name.
        name: String,
    },
    /// A figure definition that cannot be drawn.
    #[error("the figure `{name}` is wrong: {reason}")]
    BadFigure {
        /// The figure's id.
        name: String,
        /// What is wrong with it.
        reason: String,
    },
    /// An `art` widget names an asset that was not loaded.
    #[error("unknown art asset `{name}`")]
    UnknownArt {
        /// The asset's path.
        name: String,
    },
    /// Two nodes of one screen have the same `id`.
    #[error("the id `{id}` is used by more than one node")]
    DuplicateId {
        /// The id.
        id: String,
    },
    /// A development widget in a screen whose context is not `debug`.
    #[error("`{widget}` may only be used in a screen whose context is `debug`")]
    DebugOnly {
        /// The widget.
        widget: &'static str,
    },
    /// A `choice` with no options.
    #[error("a choice needs at least one option")]
    NoOptions,
    /// A `scroll` or a tab page that is not exactly one node.
    #[error("a grid needs at least one column")]
    NoColumns,
}

/// A problem in a screen definition, with where it is.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("screen `{screen}`, at `{path}`: {problem}")]
pub struct UiError {
    /// The screen's id.
    pub screen: String,
    /// The path to the node or field, such as `layout.cols[1].bar.label`.
    pub path: String,
    /// What is wrong.
    pub problem: UiProblem,
}

/// What a screen is made with: the theme in force and the definitions and
/// assets its widgets may name.
#[derive(Clone, Copy, Debug)]
pub struct UiEnv<'a> {
    /// The active theme. Styles are resolved once, when the screen is
    /// made; to restyle, make the screen again.
    pub theme: &'a Theme,
    /// The `figure` definitions, by id.
    pub figures: &'a BTreeMap<String, FigureDef>,
    /// The text of each art asset, by its path within its mod.
    pub art: &'a BTreeMap<String, String>,
}

/// A variable a widget reads and writes (12 §10.3: "widgets write only to
/// screen variables and preferences").
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VarRef {
    /// A variable of the screen: `"screen.name"` or just `"name"`.
    Screen(String),
    /// A player preference: `"pref.id"`.
    Pref(String),
}

impl VarRef {
    /// Reads a `bind` or `selected` field.
    pub fn parse(text: &str) -> VarRef {
        if let Some(id) = text.strip_prefix("pref.") {
            VarRef::Pref(id.to_owned())
        } else {
            VarRef::Screen(text.strip_prefix("screen.").unwrap_or(text).to_owned())
        }
    }
}

/// Which event of a widget a list of effects belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetEvent {
    /// `on_activate`.
    Activate,
    /// `on_select`.
    Select,
    /// `on_change`.
    Change,
    /// `on_pick`.
    Pick,
}

/// Something a screen asks of whoever runs it, in answer to input.
///
/// Running interface effects is package U6b. Until then the effects are
/// handed back as written.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    /// A key the screen handles in `keys` was pressed.
    Key {
        /// The key's name in the screen's keymap context.
        name: String,
        /// The effects to run.
        effects: UiEffects,
    },
    /// A widget's event happened and it has effects to run.
    Widget {
        /// The widget's `id`, if it has one.
        id: Option<String>,
        /// Which of its events.
        event: WidgetEvent,
        /// The row the event is about, counted from 0 in the order the
        /// rows were given: `it` for the effects.
        row: Option<usize>,
        /// The effects to run.
        effects: UiEffects,
    },
    /// A widget changed a screen variable or a preference. Bindings that
    /// read it are out of date.
    Changed {
        /// What changed.
        var: VarRef,
    },
}

impl UiEvent {
    /// The effects the event asks to be run, if it carries any.
    pub fn effects(&self) -> Option<&UiEffects> {
        match self {
            UiEvent::Key { effects, .. } | UiEvent::Widget { effects, .. } => Some(effects),
            UiEvent::Changed { .. } => None,
        }
    }

    /// Whether the event asks for an effect with this name to be run,
    /// such as `close`.
    pub fn asks_for(&self, effect: &str) -> bool {
        self.effects()
            .is_some_and(|effects| effects.names().any(|name| name == effect))
    }
}

/// One line of the message log (12 §7.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The line's category, which a `log` widget's `filter` selects by.
    pub category: String,
    /// The text.
    pub text: RichText,
}

/// The message log a `log` widget shows. Playback (package U5) fills it;
/// this is the store and nothing more.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageLog {
    lines: Vec<LogLine>,
}

impl MessageLog {
    /// An empty log.
    pub fn new() -> MessageLog {
        MessageLog::default()
    }

    /// Adds a line at the end.
    pub fn push(&mut self, category: &str, text: RichText) {
        self.lines.push(LogLine {
            category: category.to_owned(),
            text,
        });
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> &[LogLine] {
        &self.lines
    }
}

/// Which view of the world a widget is (12 §6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    /// `map_view`: one storey from above.
    Map,
    /// `section_view`: a vertical slice.
    Section,
    /// `region_map`: the tract-scale map.
    Region,
    /// `chart_view`: a chart and travelers on it.
    Chart,
}

impl ViewKind {
    /// The widget's name.
    pub const fn name(self) -> &'static str {
        match self {
            ViewKind::Map => "map_view",
            ViewKind::Section => "section_view",
            ViewKind::Region => "region_map",
            ViewKind::Chart => "chart_view",
        }
    }
}

/// Draws the views of the world into the places the toolkit gives them.
/// The view model (package U4) implements it; without one, a view is
/// drawn as an empty area with its name.
pub trait ViewPainter {
    /// Draws one view into `area` of the canvas, which is already clipped
    /// to it.
    fn paint(
        &self,
        kind: ViewKind,
        def: &ViewDef,
        focused: bool,
        area: Rect,
        canvas: &mut Canvas<'_>,
    );
}

/// What a screen shows that is not its own: the message log and the
/// views of the world.
#[derive(Clone, Copy, Default)]
pub struct UiData<'a> {
    /// The message log, for `log` widgets.
    pub log: Option<&'a MessageLog>,
    /// What draws `map_view` and the other views.
    pub views: Option<&'a dyn ViewPainter>,
}

impl core::fmt::Debug for UiData<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiData")
            .field("log", &self.log.map(|log| log.lines().len()))
            .field("views", &self.views.is_some())
            .finish()
    }
}

/// What bound properties are evaluated against.
#[derive(Clone, Copy, Debug)]
pub struct BindScope<'a> {
    /// The screen's id.
    pub screen: &'a str,
    /// The screen's variables: `screen.name` in a binding.
    pub vars: &'a VarMap,
    /// The preferences the screen's widgets are bound to.
    pub prefs: &'a VarMap,
}

/// Evaluates bound properties. This is where package U6b plugs in: it
/// implements this over the expression language and the names of
/// 12 §10.4, and calls [`Screen::refresh`] whenever the simulation has
/// advanced or a screen variable has changed.
///
/// Each method is given a property's unparsed source and returns its
/// value now, or `None` to leave the property as it was.
pub trait Evaluator {
    /// A text template, such as `"{name(view)} is here."`.
    fn text(&mut self, source: &str, scope: &BindScope<'_>) -> Option<String>;
    /// A numeric expression, such as `"view.pool.blood.frac"`.
    fn number(&mut self, source: &str, scope: &BindScope<'_>) -> Option<f64>;
    /// A condition, such as `"screen.tab == 1"`.
    fn flag(&mut self, source: &str, scope: &BindScope<'_>) -> Option<bool>;
}

// ---------------------------------------------------------------------
// What a screen is made of.
// ---------------------------------------------------------------------

/// A text that may be bound: its source if so, and what it shows now.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LiveText {
    pub(super) source: Option<String>,
    pub(super) text: RichText,
}

/// A number that may be bound.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LiveNum {
    pub(super) source: Option<String>,
    pub(super) value: f64,
}

/// A flag that may be bound.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LiveFlag {
    pub(super) source: Option<String>,
    pub(super) value: bool,
}

/// The styles and characters the toolkit draws its own parts with,
/// resolved from the theme once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Chrome {
    pub(super) base: CellStyle,
    pub(super) title: Style,
    pub(super) border: Style,
    pub(super) border_focus: Style,
    pub(super) selected: Style,
    pub(super) selected_idle: Style,
    pub(super) header: Style,
    pub(super) dim: Style,
    pub(super) bar: Style,
    pub(super) bar_empty: Style,
    pub(super) field: Style,
    pub(super) button: Style,
    pub(super) hotkey: Style,
    pub(super) tip: Style,
    pub(super) notice: Style,
    pub(super) borders: Borders,
    pub(super) bar_full_ch: char,
    pub(super) bar_empty_ch: char,
    pub(super) scroll_thumb: char,
    pub(super) scroll_track: char,
    pub(super) tree_open: char,
    pub(super) tree_closed: char,
    pub(super) toggle_on: char,
    pub(super) toggle_off: char,
    pub(super) choice_prev: char,
    pub(super) choice_next: char,
    pub(super) sort_up: char,
    pub(super) sort_down: char,
    pub(super) view_empty: char,
}

impl Chrome {
    fn new(theme: &Theme) -> Chrome {
        let style = |name: &str, fallback: Style| theme.style(name).unwrap_or(fallback);
        let mark = |name: &str, fallback: char| {
            theme
                .mark(name)
                .and_then(|glyph| glyph.part(&crate::glyph::GlyphInputs::default()).ch)
                .unwrap_or(fallback)
        };
        let dim = style("dim", Style::attrs(Attrs::DIM));
        Chrome {
            base: theme.base(),
            title: style("title", Style::attrs(Attrs::BOLD)),
            border: style("border", Style::default()),
            border_focus: style("border_focus", Style::attrs(Attrs::BOLD)),
            selected: style("selected", Style::attrs(Attrs::REVERSE)),
            selected_idle: style("selected_idle", Style::attrs(Attrs::UNDERLINE)),
            header: style("header", Style::attrs(Attrs::BOLD | Attrs::UNDERLINE)),
            dim,
            bar: style("bar", Style::default()),
            bar_empty: style("bar_empty", dim),
            field: style("field", Style::attrs(Attrs::UNDERLINE)),
            button: style("button", Style::default()),
            hotkey: style("hotkey", Style::attrs(Attrs::BOLD)),
            tip: style("tip", dim),
            notice: style("notice", Style::attrs(Attrs::BOLD)),
            borders: theme.borders(),
            bar_full_ch: mark("bar_full", '█'),
            bar_empty_ch: mark("bar_empty", '░'),
            scroll_thumb: mark("scroll_thumb", '█'),
            scroll_track: mark("scroll_track", '│'),
            tree_open: mark("tree_open", '▾'),
            tree_closed: mark("tree_closed", '▸'),
            toggle_on: mark("toggle_on", 'x'),
            toggle_off: mark("toggle_off", ' '),
            choice_prev: mark("choice_prev", '<'),
            choice_next: mark("choice_next", '>'),
            sort_up: mark("sort_up", '▲'),
            sort_down: mark("sort_down", '▼'),
            view_empty: mark("view_empty", '·'),
        }
    }
}

/// Which of the row widgets a listing is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flavour {
    List,
    Menu,
    Tree,
    Table,
    Inspect,
}

impl Flavour {
    pub(super) fn nests(self) -> bool {
        matches!(self, Flavour::Tree | Flavour::Inspect)
    }
}

/// One row of a listing, with its place in the hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub(super) label: LiveText,
    pub(super) value: Option<LiveText>,
    pub(super) key: Option<KeyChord>,
    /// The key as it is shown beside a menu option.
    pub(super) key_text: String,
    pub(super) cells: Vec<LiveText>,
    pub(super) depth: u16,
    pub(super) parent: Option<usize>,
    pub(super) has_children: bool,
    pub(super) open: bool,
    pub(super) tip: Option<LiveText>,
    pub(super) on_activate: UiEffects,
}

/// One column of a table.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Column {
    pub(super) title: LiveText,
    pub(super) width: Option<Size>,
    pub(super) align: Align,
}

/// The state shared by `list`, `menu`, `tree`, `table` and `inspect`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Listing {
    pub(super) flavour: Flavour,
    /// Every row, in the order written, children after their parent.
    pub(super) rows: Vec<Row>,
    /// The rows on show, as indices into `rows`, in display order.
    pub(super) shown: Vec<usize>,
    /// The selected row, as an index into `shown`.
    pub(super) sel: usize,
    /// The first row displayed, as an index into `shown`.
    pub(super) top: usize,
    pub(super) var: Option<String>,
    pub(super) empty: Option<LiveText>,
    pub(super) columns: Vec<Column>,
    pub(super) col_widths: Vec<i32>,
    /// The column sorted by and whether descending.
    pub(super) sort: Option<(usize, bool)>,
    pub(super) on_select: UiEffects,
    pub(super) on_activate: UiEffects,
    /// The width of the hotkey column of a menu.
    pub(super) key_width: i32,
}

impl Listing {
    /// Recomputes which rows are on show and in what order, keeping the
    /// same row selected where it is still on show.
    pub(super) fn reshow(&mut self) {
        let selected_row = self.shown.get(self.sel).copied();
        self.shown.clear();
        for index in 0..self.rows.len() {
            let mut ancestor = self.rows[index].parent;
            let mut visible = true;
            while let Some(parent) = ancestor {
                if !self.rows[parent].open {
                    visible = false;
                    break;
                }
                ancestor = self.rows[parent].parent;
            }
            if visible {
                self.shown.push(index);
            }
        }
        if let Some((column, descending)) = self.sort {
            let rows = &self.rows;
            let cell = |row: usize| rows[row].cells.get(column).map(|c| c.text.plain());
            self.shown.sort_by(|a, b| {
                let (left, right) = (cell(*a), cell(*b));
                let order = match (
                    left.as_deref().and_then(|t| t.trim().parse::<f64>().ok()),
                    right.as_deref().and_then(|t| t.trim().parse::<f64>().ok()),
                ) {
                    (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(core::cmp::Ordering::Equal),
                    _ => left.cmp(&right),
                };
                if descending { order.reverse() } else { order }
            });
        }
        self.sel = selected_row
            .and_then(|row| self.shown.iter().position(|r| *r == row))
            .unwrap_or_else(|| self.sel.min(self.shown.len().saturating_sub(1)));
    }

    /// The selected row as an index into `rows`.
    pub(super) fn selected_row(&self) -> Option<usize> {
        self.shown.get(self.sel).copied()
    }
}

/// What a node is, with the state it keeps.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Kind {
    Rows,
    Cols,
    Stack,
    Grid {
        columns: Vec<Size>,
        rows: Vec<Size>,
    },
    Tabs {
        titles: Vec<LiveText>,
        var: Option<String>,
        active: usize,
    },
    Scroll {
        x: i32,
        y: i32,
        content_w: i32,
        content_h: i32,
        /// Whether the scroll takes the focus itself: it has nothing
        /// inside that can.
        own_focus: bool,
    },
    Slot {
        name: String,
    },
    Text {
        value: LiveText,
        wrap: bool,
        align: Align,
        lines: Vec<(usize, usize)>,
    },
    Art {
        rows: Vec<Vec<char>>,
    },
    Listing(Box<Listing>),
    Bar {
        value: LiveNum,
        max: LiveNum,
        label: Option<LiveText>,
        gauge: bool,
    },
    Input {
        var: VarRef,
        label: Option<LiveText>,
        max_len: usize,
        cursor: usize,
        on_change: UiEffects,
    },
    Number {
        var: VarRef,
        label: Option<LiveText>,
        min: Option<LiveNum>,
        max: Option<LiveNum>,
        step: LiveNum,
        on_change: UiEffects,
    },
    Toggle {
        var: VarRef,
        label: Option<LiveText>,
        on_change: UiEffects,
    },
    Choice {
        var: VarRef,
        label: Option<LiveText>,
        options: Vec<(Value, LiveText)>,
        on_change: UiEffects,
    },
    Button {
        label: LiveText,
        on_activate: UiEffects,
    },
    Log {
        filter: Option<Vec<String>>,
        /// How many lines the view is scrolled back from the newest.
        back: usize,
    },
    View {
        kind: ViewKind,
        def: Box<ViewDef>,
    },
    Figure {
        rows: Vec<Vec<char>>,
        /// For each cell, the index of its region in `shades`.
        region_of: Vec<Vec<Option<u8>>>,
        region_names: Vec<String>,
        shades: Vec<Option<Style>>,
    },
    Tooltip {
        value: Option<LiveText>,
    },
    Spacer,
    Debug {
        name: &'static str,
    },
}

impl Kind {
    /// The widget's or container's name in data.
    pub(super) fn name(&self) -> &'static str {
        match self {
            Kind::Rows => "rows",
            Kind::Cols => "cols",
            Kind::Stack => "stack",
            Kind::Grid { .. } => "grid",
            Kind::Tabs { .. } => "tabs",
            Kind::Scroll { .. } => "scroll",
            Kind::Slot { .. } => "slot",
            Kind::Text { .. } => "text",
            Kind::Art { .. } => "art",
            Kind::Listing(listing) => match listing.flavour {
                Flavour::List => "list",
                Flavour::Menu => "menu",
                Flavour::Tree => "tree",
                Flavour::Table => "table",
                Flavour::Inspect => "inspect",
            },
            Kind::Bar { gauge: false, .. } => "bar",
            Kind::Bar { gauge: true, .. } => "gauge",
            Kind::Input { .. } => "input",
            Kind::Number { .. } => "number",
            Kind::Toggle { .. } => "toggle",
            Kind::Choice { .. } => "choice",
            Kind::Button { .. } => "button",
            Kind::Log { .. } => "log",
            Kind::View { kind, .. } => kind.name(),
            Kind::Figure { .. } => "figure",
            Kind::Tooltip { .. } => "tooltip",
            Kind::Spacer => "spacer",
            Kind::Debug { name } => name,
        }
    }

    /// Whether the node can take the focus.
    pub(super) fn focusable(&self) -> bool {
        match self {
            Kind::Tabs { titles, .. } => !titles.is_empty(),
            Kind::Scroll { own_focus, .. } => *own_focus,
            Kind::Listing(listing) => !listing.rows.is_empty(),
            Kind::Input { .. }
            | Kind::Number { .. }
            | Kind::Toggle { .. }
            | Kind::Choice { .. }
            | Kind::Button { .. }
            | Kind::Log { .. }
            | Kind::View { .. }
            | Kind::Debug { .. } => true,
            _ => false,
        }
    }
}

/// One node of a screen: a container or a widget.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Node {
    pub(super) id: Option<String>,
    pub(super) style: Option<Style>,
    pub(super) show_if: Option<LiveFlag>,
    pub(super) tip: Option<LiveText>,
    pub(super) width: Option<Size>,
    pub(super) height: Option<Size>,
    pub(super) fill: Option<u16>,
    pub(super) fit: bool,
    /// `Some(None)` is a border with no title.
    pub(super) frame: Option<Option<LiveText>>,
    pub(super) kind: Kind,
    pub(super) children: Vec<usize>,
    pub(super) parent: Option<usize>,
    /// Whether the last layout gave the node a place.
    pub(super) shown: bool,
    /// The node's rectangle, border included.
    pub(super) outer: Rect,
    /// The rectangle of its content.
    pub(super) inner: Rect,
    /// What its content's drawing is confined to.
    pub(super) clip: Rect,
    /// What the node itself, border included, is confined to.
    pub(super) bounds: Rect,
}

/// A screen ready to lay out, draw and use: a [`ScreenDef`] with its
/// texts and styles resolved against a theme, and the state its widgets
/// keep.
///
/// ```
/// use std::collections::BTreeMap;
/// use clash_scene::ui::{Screen, ScreenDef, UiData, UiEnv};
/// use clash_scene::{Frame, InputEvent, KeymapDef, Theme};
///
/// let def: ScreenDef = serde_json::from_str(
///     r#"{ "layout": { "frame": "Things", "list": { "rows": ["A knife", "A rope"] } } }"#,
/// ).unwrap();
/// let (theme, none, no_art) = (Theme::default(), BTreeMap::new(), BTreeMap::new());
/// let env = UiEnv { theme: &theme, figures: &none, art: &no_art };
/// let mut screen = Screen::new("things", &def, &env).unwrap();
///
/// let mut frame = Frame::new(12, 4);
/// screen.draw(&mut frame, &UiData::default());
/// assert_eq!(frame.row_text(0), "┌ Things ──┐");
/// assert_eq!(frame.row_text(1), "│A knife   │");
///
/// screen.input(&InputEvent::key("down").unwrap(), &KeymapDef::fallback());
/// assert_eq!(screen.selected_row("layout"), Some(1));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub(super) id: String,
    pub(super) title: Option<LiveText>,
    pub(super) context: String,
    pub(super) min_size: (u16, u16),
    pub(super) vars: VarMap,
    pub(super) prefs: VarMap,
    pub(super) keys: BTreeMap<String, UiEffects>,
    pub(super) nodes: Vec<Node>,
    pub(super) chrome: Chrome,
    /// The node with the focus.
    pub(super) focus: Option<usize>,
    /// The nodes that can take the focus now, in order.
    pub(super) focus_order: Vec<usize>,
    /// The node with a tip under the pointer.
    pub(super) hover: Option<usize>,
    /// The grid size of the last layout.
    pub(super) size: (u16, u16),
    /// Whether something changed that the layout depends on.
    pub(super) stale: bool,
}

struct Builder<'a> {
    env: &'a UiEnv<'a>,
    screen: &'a str,
    debug: bool,
    vars: &'a VarMap,
    prefs: VarMap,
    nodes: Vec<Node>,
    errors: Vec<UiError>,
    ids: Vec<String>,
}

/// What kind of value a widget needs its variable to hold.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Flag,
    Number,
    Whole,
    Text,
    Any,
}

impl Builder<'_> {
    fn problem(&mut self, path: &str, problem: UiProblem) {
        self.errors.push(UiError {
            screen: self.screen.to_owned(),
            path: path.to_owned(),
            problem,
        });
    }

    fn text(&mut self, prop: &TextProp, path: &str) -> LiveText {
        match prop {
            Prop::Static(source) => match RichText::parse(source, self.env.theme) {
                Ok(text) => LiveText { source: None, text },
                Err(error) => {
                    self.problem(path, error.into());
                    LiveText {
                        source: None,
                        text: RichText::plain_text(source),
                    }
                }
            },
            Prop::Bound(source) => {
                // The template is filled in later; its tags can be
                // checked now.
                match tag_names(source) {
                    Ok(names) => {
                        for name in names {
                            if self.env.theme.style(&name).is_none() {
                                self.problem(path, TextError::UnknownStyle { name }.into());
                            }
                        }
                    }
                    Err(error) => self.problem(path, error.into()),
                }
                LiveText {
                    source: Some(source.clone()),
                    text: RichText::plain_text(source),
                }
            }
        }
    }

    fn opt_text(&mut self, prop: &Option<TextProp>, path: &str) -> Option<LiveText> {
        prop.as_ref().map(|prop| self.text(prop, path))
    }

    fn num(prop: &Prop<f64>) -> LiveNum {
        match prop {
            Prop::Static(value) => LiveNum {
                source: None,
                value: *value,
            },
            Prop::Bound(source) => LiveNum {
                source: Some(source.clone()),
                value: 0.0,
            },
        }
    }

    fn style(&mut self, name: &Option<String>, path: &str) -> Option<Style> {
        let name = name.as_ref()?;
        let style = self.env.theme.style(name);
        if style.is_none() {
            self.problem(path, UiProblem::UnknownStyle { name: name.clone() });
        }
        style
    }

    /// Checks a `bind` or `selected` field and gives a preference a
    /// starting value if it has none.
    fn bind(&mut self, name: &str, want: Want, start: Value, path: &str) -> VarRef {
        let var = VarRef::parse(name);
        let held = match &var {
            VarRef::Screen(name) => match self.vars.get(name) {
                Some(value) => value.clone(),
                None => {
                    self.problem(path, UiProblem::UnknownVariable { name: name.clone() });
                    return var;
                }
            },
            VarRef::Pref(id) => {
                if !self.prefs.contains(id) {
                    self.prefs.set(id.as_str(), start);
                }
                return var;
            }
        };
        let (fits, expected) = match want {
            Want::Flag => (matches!(held, Value::Bool(_)), "true or false"),
            Want::Number => (matches!(held, Value::Int(_) | Value::Real(_)), "a number"),
            Want::Whole => (matches!(held, Value::Int(_)), "a whole number"),
            Want::Text => (matches!(held, Value::Str(_)), "a text"),
            Want::Any => (true, "anything"),
        };
        if !fits {
            self.problem(
                path,
                UiProblem::WrongVariableType {
                    name: name.to_owned(),
                    expected,
                    found: held.type_name(),
                },
            );
        }
        var
    }

    fn selected(&mut self, name: &Option<String>, path: &str) -> Option<String> {
        let name = name.as_ref()?;
        match self.bind(name, Want::Whole, Value::Int(0), path) {
            VarRef::Screen(name) => Some(name),
            // A selection is screen state; a preference cannot hold it.
            VarRef::Pref(id) => {
                self.problem(
                    path,
                    UiProblem::UnknownVariable {
                        name: format!("pref.{id}"),
                    },
                );
                None
            }
        }
    }

    fn rows(&mut self, def: &RowsDef, path: &str) -> Vec<Row> {
        let mut rows = Vec::new();
        if let RowsDef::Static(defs) = def {
            self.add_rows(defs, None, 0, path, &mut rows);
        }
        rows
    }

    fn add_rows(
        &mut self,
        defs: &[RowDef],
        parent: Option<usize>,
        depth: u16,
        path: &str,
        out: &mut Vec<Row>,
    ) {
        for (index, def) in defs.iter().enumerate() {
            let path = format!("{path}[{index}]");
            let row = Row {
                label: self.text(&def.label, &format!("{path}.label")),
                value: self.opt_text(&def.value, &format!("{path}.value")),
                key: def.key,
                key_text: def.key.map(|key| key.to_string()).unwrap_or_default(),
                cells: def
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(c, cell)| self.text(cell, &format!("{path}.cells[{c}]")))
                    .collect(),
                depth,
                parent,
                has_children: !def.children.is_empty(),
                open: def.open,
                tip: self.opt_text(&def.tip, &format!("{path}.tip")),
                on_activate: def.on_activate.clone(),
            };
            let at = out.len();
            out.push(row);
            self.add_rows(
                &def.children,
                Some(at),
                depth + 1,
                &format!("{path}.children"),
                out,
            );
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the five row widgets differ only in which of these they give"
    )]
    fn listing(
        &mut self,
        flavour: Flavour,
        rows: &RowsDef,
        columns: &[ColumnDef],
        empty: &Option<TextProp>,
        selected: &Option<String>,
        on_select: &UiEffects,
        on_activate: &UiEffects,
        path: &str,
    ) -> Kind {
        let rows = self.rows(rows, &format!("{path}.rows"));
        let columns: Vec<Column> = columns
            .iter()
            .enumerate()
            .map(|(index, column)| Column {
                title: self.text(&column.title, &format!("{path}.columns[{index}].title")),
                width: column.width,
                align: column.align,
            })
            .collect();
        let key_width = rows
            .iter()
            .filter(|row| row.key.is_some())
            .map(|row| row.key_text.chars().count() as i32 + 1)
            .max()
            .unwrap_or(0);
        let var = self.selected(selected, &format!("{path}.selected"));
        let mut listing = Listing {
            flavour,
            rows,
            shown: Vec::new(),
            sel: 0,
            top: 0,
            var,
            empty: self.opt_text(empty, &format!("{path}.empty")),
            col_widths: vec![0; columns.len()],
            columns,
            sort: None,
            on_select: on_select.clone(),
            on_activate: on_activate.clone(),
            key_width,
        };
        listing.reshow();
        Kind::Listing(Box::new(listing))
    }

    fn figure(
        &mut self,
        name: &str,
        shade: &Option<Prop<BTreeMap<String, String>>>,
        path: &str,
    ) -> Kind {
        let empty = Kind::Figure {
            rows: Vec::new(),
            region_of: Vec::new(),
            region_names: Vec::new(),
            shades: Vec::new(),
        };
        let Some(def) = self.env.figures.get(name) else {
            self.problem(
                path,
                UiProblem::UnknownFigure {
                    name: name.to_owned(),
                },
            );
            return empty;
        };
        let bad = |reason: String| UiProblem::BadFigure {
            name: name.to_owned(),
            reason,
        };
        let rows: Vec<Vec<char>> = def.rows.iter().map(|row| row.chars().collect()).collect();
        let mut region_names: Vec<String> = Vec::new();
        let mut keys: Vec<char> = Vec::new();
        for (key, region) in &def.regions {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) if ch != ' ' => {
                    keys.push(ch);
                    region_names.push(region.clone());
                }
                _ => {
                    self.problem(
                        path,
                        bad(format!(
                            "the region key `{key}` must be one character, not a space"
                        )),
                    );
                    return empty;
                }
            }
        }
        if keys.len() > usize::from(u8::MAX) {
            self.problem(path, bad("too many regions".to_owned()));
            return empty;
        }
        let mut region_of: Vec<Vec<Option<u8>>> =
            rows.iter().map(|row| vec![None; row.len()]).collect();
        if !def.map.is_empty() {
            if def.map.len() != rows.len() {
                self.problem(
                    path,
                    bad("`map` must have as many rows as `rows`".to_owned()),
                );
                return empty;
            }
            for (r, line) in def.map.iter().enumerate() {
                let line: Vec<char> = line.chars().collect();
                if line.len() != rows[r].len() {
                    self.problem(
                        path,
                        bad(format!(
                            "row {r} of `map` is not as long as row {r} of `rows`"
                        )),
                    );
                    return empty;
                }
                for (c, ch) in line.iter().enumerate() {
                    if *ch == ' ' {
                        continue;
                    }
                    match keys.iter().position(|key| key == ch) {
                        Some(index) => region_of[r][c] = Some(index as u8),
                        None => {
                            self.problem(
                                path,
                                bad(format!("`map` uses `{ch}`, which `regions` does not name")),
                            );
                            return empty;
                        }
                    }
                }
            }
        }
        let mut shades = vec![None; region_names.len()];
        if let Some(Prop::Static(by_region)) = shade {
            for (region, style_name) in by_region {
                let Some(index) = region_names.iter().position(|name| name == region) else {
                    self.problem(
                        &format!("{path}.shade"),
                        bad(format!(
                            "`shade` names the region `{region}`, which the figure does not have"
                        )),
                    );
                    continue;
                };
                shades[index] =
                    self.style(&Some(style_name.clone()), &format!("{path}.shade.{region}"));
            }
        }
        Kind::Figure {
            rows,
            region_of,
            region_names,
            shades,
        }
    }

    fn children(&mut self, defs: &[NodeDef], parent: usize, path: &str) -> Vec<usize> {
        defs.iter()
            .enumerate()
            .map(|(index, def)| self.node(def, Some(parent), &format!("{path}[{index}]")))
            .collect()
    }

    fn node(&mut self, def: &NodeDef, parent: Option<usize>, path: &str) -> usize {
        let index = self.nodes.len();
        let kind_name = def.kind.name();
        let path = format!("{path}.{kind_name}");
        if let Some(id) = &def.common.id {
            if self.ids.contains(id) {
                self.problem(&path, UiProblem::DuplicateId { id: id.clone() });
            }
            self.ids.push(id.clone());
        }
        let show_if = def.common.show_if.as_ref().map(|prop| match prop {
            Prop::Static(value) => LiveFlag {
                source: None,
                value: *value,
            },
            Prop::Bound(source) => LiveFlag {
                source: Some(source.clone()),
                value: true,
            },
        });
        let frame = match &def.frame {
            None => None,
            Some(FrameDef::Plain) => Some(None),
            Some(FrameDef::Titled(title)) => Some(Some(self.text(title, &format!("{path}.frame")))),
        };
        let node = Node {
            id: def.common.id.clone(),
            style: self.style(&def.common.style, &format!("{path}.style")),
            show_if,
            tip: self.opt_text(&def.common.tip, &format!("{path}.tip")),
            width: def.width,
            height: def.height,
            fill: def.fill,
            fit: def.fit,
            frame,
            kind: Kind::Spacer,
            children: Vec::new(),
            parent,
            shown: false,
            outer: Rect::default(),
            inner: Rect::default(),
            clip: Rect::default(),
            bounds: Rect::default(),
        };
        self.nodes.push(node);

        let mut children = Vec::new();
        let label = |b: &mut Builder<'_>, text: &Option<TextProp>| {
            b.opt_text(text, &format!("{path}.label"))
        };
        let kind = match &def.kind {
            NodeKind::Rows(defs) => {
                children = self.children(defs, index, &path);
                Kind::Rows
            }
            NodeKind::Cols(defs) => {
                children = self.children(defs, index, &path);
                Kind::Cols
            }
            NodeKind::Stack(defs) => {
                children = self.children(defs, index, &path);
                Kind::Stack
            }
            NodeKind::Grid(grid) => {
                if grid.columns.is_empty() {
                    self.problem(&path, UiProblem::NoColumns);
                }
                children = self.children(&grid.cells, index, &format!("{path}.cells"));
                Kind::Grid {
                    columns: grid.columns.clone(),
                    rows: grid.rows.clone(),
                }
            }
            NodeKind::Tabs(tabs) => {
                let mut titles = Vec::with_capacity(tabs.pages.len());
                for (page, tab) in tabs.pages.iter().enumerate() {
                    let page_path = format!("{path}.pages[{page}]");
                    titles.push(self.text(&tab.title, &format!("{page_path}.title")));
                    children.push(self.node(
                        &tab.content,
                        Some(index),
                        &format!("{page_path}.content"),
                    ));
                }
                Kind::Tabs {
                    titles,
                    var: self.selected(&tabs.selected, &format!("{path}.selected")),
                    active: 0,
                }
            }
            NodeKind::Scroll(child) => {
                children.push(self.node(child, Some(index), &path));
                Kind::Scroll {
                    x: 0,
                    y: 0,
                    content_w: 0,
                    content_h: 0,
                    own_focus: false,
                }
            }
            NodeKind::Slot(name) => Kind::Slot { name: name.clone() },
            NodeKind::Text(text) => Kind::Text {
                value: self.text(&text.value, &format!("{path}.value")),
                wrap: text.wrap,
                align: text.align,
                lines: Vec::new(),
            },
            NodeKind::Art(art) => {
                let rows = match &art.value {
                    Prop::Static(name) => match self.env.art.get(name) {
                        Some(text) => text.lines().map(|line| line.chars().collect()).collect(),
                        None => {
                            self.problem(
                                &format!("{path}.value"),
                                UiProblem::UnknownArt { name: name.clone() },
                            );
                            Vec::new()
                        }
                    },
                    Prop::Bound(_) => Vec::new(),
                };
                Kind::Art { rows }
            }
            NodeKind::List(w) => self.listing(
                Flavour::List,
                &w.rows,
                &[],
                &w.empty,
                &w.selected,
                &w.on_select,
                &w.on_activate,
                &path,
            ),
            NodeKind::Menu(w) => self.listing(
                Flavour::Menu,
                &w.rows,
                &[],
                &None,
                &w.selected,
                &w.on_select,
                &w.on_activate,
                &path,
            ),
            NodeKind::Tree(w) => self.listing(
                Flavour::Tree,
                &w.rows,
                &[],
                &w.empty,
                &w.selected,
                &w.on_select,
                &w.on_activate,
                &path,
            ),
            NodeKind::Table(w) => self.listing(
                Flavour::Table,
                &w.rows,
                &w.columns,
                &w.empty,
                &w.selected,
                &w.on_select,
                &w.on_activate,
                &path,
            ),
            NodeKind::Inspect(w) => self.listing(
                Flavour::Inspect,
                &w.rows,
                &[],
                &None,
                &w.selected,
                &w.on_select,
                &w.on_activate,
                &path,
            ),
            NodeKind::Bar(bar) | NodeKind::Gauge(bar) => Kind::Bar {
                value: Builder::num(&bar.value),
                max: Builder::num(&bar.max),
                label: label(self, &bar.label),
                gauge: matches!(def.kind, NodeKind::Gauge(_)),
            },
            NodeKind::Input(w) => Kind::Input {
                var: self.bind(
                    &w.bind,
                    Want::Text,
                    Value::Str(String::new()),
                    &format!("{path}.bind"),
                ),
                label: label(self, &w.label),
                max_len: w.max_len.map_or(usize::MAX, usize::from),
                cursor: usize::MAX,
                on_change: w.on_change.clone(),
            },
            NodeKind::Number(w) => Kind::Number {
                var: self.bind(
                    &w.bind,
                    Want::Number,
                    Value::Int(0),
                    &format!("{path}.bind"),
                ),
                label: label(self, &w.label),
                min: w.min.as_ref().map(Builder::num),
                max: w.max.as_ref().map(Builder::num),
                step: Builder::num(&w.step),
                on_change: w.on_change.clone(),
            },
            NodeKind::Toggle(w) => Kind::Toggle {
                var: self.bind(
                    &w.bind,
                    Want::Flag,
                    Value::Bool(false),
                    &format!("{path}.bind"),
                ),
                label: label(self, &w.label),
                on_change: w.on_change.clone(),
            },
            NodeKind::Choice(w) => {
                let mut options = Vec::new();
                if let OptionsDef::Static(defs) = &w.options {
                    if defs.is_empty() {
                        self.problem(&format!("{path}.options"), UiProblem::NoOptions);
                    }
                    for (o, option) in defs.iter().enumerate() {
                        let Literal(value) = &option.value;
                        let shown = match &option.label {
                            Some(text) => self.text(text, &format!("{path}.options[{o}].label")),
                            None => LiveText {
                                source: None,
                                text: RichText::plain_text(&value.to_string()),
                            },
                        };
                        options.push((value.clone(), shown));
                    }
                }
                let start = options
                    .first()
                    .map_or(Value::Int(0), |(value, _)| value.clone());
                Kind::Choice {
                    var: self.bind(&w.bind, Want::Any, start, &format!("{path}.bind")),
                    label: label(self, &w.label),
                    options,
                    on_change: w.on_change.clone(),
                }
            }
            NodeKind::Button(w) => Kind::Button {
                label: self.text(&w.label, &format!("{path}.label")),
                on_activate: w.on_activate.clone(),
            },
            NodeKind::Log(w) => Kind::Log {
                filter: match &w.filter {
                    Some(Prop::Static(names)) => Some(names.clone()),
                    _ => None,
                },
                back: 0,
            },
            NodeKind::MapView(v) => Kind::View {
                kind: ViewKind::Map,
                def: Box::new(v.clone()),
            },
            NodeKind::SectionView(v) => Kind::View {
                kind: ViewKind::Section,
                def: Box::new(v.clone()),
            },
            NodeKind::RegionMap(v) => Kind::View {
                kind: ViewKind::Region,
                def: Box::new(v.clone()),
            },
            NodeKind::ChartView(v) => Kind::View {
                kind: ViewKind::Chart,
                def: Box::new(v.clone()),
            },
            NodeKind::Figure(w) => self.figure(&w.figure, &w.shade, &path),
            NodeKind::Tooltip(w) => Kind::Tooltip {
                value: self.opt_text(&w.value, &format!("{path}.value")),
            },
            NodeKind::Spacer(_) => Kind::Spacer,
            NodeKind::Console(_) | NodeKind::Canvas(_) | NodeKind::TraceView(_) => {
                if !self.debug {
                    self.problem(&path, UiProblem::DebugOnly { widget: kind_name });
                }
                Kind::Debug { name: kind_name }
            }
        };
        self.nodes[index].kind = kind;
        self.nodes[index].children = children;
        index
    }
}

impl Screen {
    /// Makes a screen from its definition: resolves its styles and texts
    /// against the theme, checks what its widgets name, and gives its
    /// variables their starting values. Every problem found is reported,
    /// not only the first.
    pub fn new(id: &str, def: &ScreenDef, env: &UiEnv<'_>) -> Result<Screen, Vec<UiError>> {
        let context = def
            .context
            .clone()
            .unwrap_or_else(|| CONTEXT_MENU.to_owned());
        let vars: VarMap = def
            .vars
            .iter()
            .map(|(name, Literal(value))| (name.as_str(), value.clone()))
            .collect();
        let mut builder = Builder {
            env,
            screen: id,
            debug: context == CONTEXT_DEBUG,
            vars: &vars,
            prefs: VarMap::new(),
            nodes: Vec::new(),
            errors: Vec::new(),
            ids: Vec::new(),
        };
        let title = builder.opt_text(&def.title, "title");
        builder.node(&def.layout, None, "layout");
        let Builder {
            prefs,
            nodes,
            errors,
            ..
        } = builder;
        if !errors.is_empty() {
            return Err(errors);
        }
        let [min_cols, min_rows] = def.min_size.unwrap_or([0, 0]);
        let mut screen = Screen {
            id: id.to_owned(),
            title,
            context,
            min_size: (min_cols, min_rows),
            vars,
            prefs,
            keys: def.keys.clone(),
            nodes,
            chrome: Chrome::new(env.theme),
            focus: None,
            focus_order: Vec::new(),
            hover: None,
            size: (0, 0),
            stale: true,
        };
        screen.pull_vars();
        Ok(screen)
    }

    /// The screen's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The screen's title as it reads now.
    pub fn title(&self) -> Option<&RichText> {
        self.title.as_ref().map(|title| &title.text)
    }

    /// The keymap context the screen's keys are named in.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// The smallest grid the screen can be shown on, in columns and rows.
    pub fn min_size(&self) -> (u16, u16) {
        self.min_size
    }

    /// The screen's variables.
    pub fn vars(&self) -> &VarMap {
        &self.vars
    }

    /// One of the screen's variables.
    pub fn var(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    /// Changes one of the screen's variables, as the `set` effect does.
    /// Returns false, changing nothing, if the screen declares no such
    /// variable.
    pub fn set_var(&mut self, name: &str, value: Value) -> bool {
        if !self.vars.contains(name) {
            return false;
        }
        self.vars.set(name, value);
        self.pull_vars();
        self.stale = true;
        true
    }

    /// The preferences the screen's widgets are bound to, with the values
    /// the widgets show. The application copies the player's preferences
    /// in with [`Screen::set_pref`] and reads changes back from here.
    pub fn prefs(&self) -> &VarMap {
        &self.prefs
    }

    /// Sets the value a preference-bound widget shows.
    pub fn set_pref(&mut self, id: &str, value: Value) {
        self.prefs.set(id, value);
        self.stale = true;
    }

    /// The names of the screen's slots (12 §10.6), in layout order.
    /// Filling them is package U8.
    pub fn slots(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().filter_map(|node| match &node.kind {
            Kind::Slot { name } => Some(name.as_str()),
            _ => None,
        })
    }

    pub(super) fn node_by_id(&self, id: &str) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.id.as_deref() == Some(id))
    }

    /// The rectangle the last layout gave the node with this id, border
    /// included, if it is on show.
    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        let node = &self.nodes[self.node_by_id(id)?];
        node.shown.then_some(node.outer)
    }

    /// The id of the widget with the focus, if it has one.
    pub fn focus(&self) -> Option<&str> {
        self.nodes[self.focus?].id.as_deref()
    }

    /// The kind of the widget with the focus, such as `list`.
    pub fn focus_kind(&self) -> Option<&'static str> {
        Some(self.nodes[self.focus?].kind.name())
    }

    /// The tip of whatever has the focus, or failing that the pointer:
    /// the selected row's own tip if it has one, otherwise the widget's.
    pub fn tip(&self) -> Option<&RichText> {
        [self.focus, self.hover]
            .into_iter()
            .flatten()
            .find_map(|index| {
                let node = &self.nodes[index];
                let row_tip = match &node.kind {
                    Kind::Listing(listing) => listing
                        .selected_row()
                        .and_then(|row| listing.rows[row].tip.as_ref()),
                    _ => None,
                };
                row_tip.or(node.tip.as_ref()).map(|tip| &tip.text)
            })
    }

    /// The selected row of the `list`, `table`, `tree`, `menu` or
    /// `inspect` with this id, counted from 0 in the order its rows were
    /// given. The path `"layout"` names the screen's root node.
    pub fn selected_row(&self, id: &str) -> Option<usize> {
        let index = if id == "layout" {
            0
        } else {
            self.node_by_id(id)?
        };
        match &self.nodes[index].kind {
            Kind::Listing(listing) => listing.selected_row(),
            _ => None,
        }
    }

    pub(super) fn read(&self, var: &VarRef) -> Option<&Value> {
        match var {
            VarRef::Screen(name) => self.vars.get(name),
            VarRef::Pref(id) => self.prefs.get(id),
        }
    }

    pub(super) fn write(&mut self, var: &VarRef, value: Value) {
        match var {
            VarRef::Screen(name) => self.vars.set(name.as_str(), value),
            VarRef::Pref(id) => self.prefs.set(id.as_str(), value),
        };
    }

    /// Brings widget state that mirrors a screen variable (the selected
    /// row, the shown tab) in line with the variables.
    pub(super) fn pull_vars(&mut self) {
        for node in &mut self.nodes {
            match &mut node.kind {
                Kind::Listing(listing) => {
                    if let Some(Value::Int(row)) =
                        listing.var.as_ref().and_then(|v| self.vars.get(v))
                    {
                        let row = usize::try_from(*row).unwrap_or(0);
                        // Open the way to a row hidden inside a closed parent.
                        let mut ancestor = listing.rows.get(row).and_then(|r| r.parent);
                        while let Some(parent) = ancestor {
                            listing.rows[parent].open = true;
                            ancestor = listing.rows[parent].parent;
                        }
                        listing.reshow();
                        if let Some(at) = listing.shown.iter().position(|r| *r == row) {
                            listing.sel = at;
                        }
                    }
                }
                Kind::Tabs {
                    titles,
                    var,
                    active,
                } => {
                    if let Some(Value::Int(page)) = var.as_ref().and_then(|v| self.vars.get(v)) {
                        let pages = titles.len().max(1) as i64;
                        *active = page.rem_euclid(pages) as usize;
                    }
                }
                _ => {}
            }
        }
    }

    /// Re-evaluates every bound property of the screen. Package U6b calls
    /// this when the simulation has advanced or a variable has changed.
    /// A text whose result has broken style tags is shown as written.
    pub fn refresh(&mut self, evaluator: &mut dyn Evaluator, theme: &Theme) {
        let scope = BindScope {
            screen: &self.id,
            vars: &self.vars,
            prefs: &self.prefs,
        };
        let mut text = |live: &mut LiveText| {
            if let Some(source) = &live.source
                && let Some(result) = evaluator.text(source, &scope)
            {
                live.text = RichText::parse(&result, theme)
                    .unwrap_or_else(|_| RichText::plain_text(&result));
            }
        };
        if let Some(title) = &mut self.title {
            text(title);
        }
        for node in &mut self.nodes {
            if let Some(tip) = &mut node.tip {
                text(tip);
            }
            if let Some(Some(title)) = &mut node.frame {
                text(title);
            }
            match &mut node.kind {
                Kind::Tabs { titles, .. } => titles.iter_mut().for_each(&mut text),
                Kind::Text { value, .. } => text(value),
                Kind::Listing(listing) => {
                    for row in &mut listing.rows {
                        text(&mut row.label);
                        row.value.iter_mut().for_each(&mut text);
                        row.cells.iter_mut().for_each(&mut text);
                        row.tip.iter_mut().for_each(&mut text);
                    }
                    listing.empty.iter_mut().for_each(&mut text);
                    for column in &mut listing.columns {
                        text(&mut column.title);
                    }
                }
                Kind::Bar { label, .. }
                | Kind::Input { label, .. }
                | Kind::Number { label, .. }
                | Kind::Toggle { label, .. } => label.iter_mut().for_each(&mut text),
                Kind::Choice { label, options, .. } => {
                    label.iter_mut().for_each(&mut text);
                    for (_, shown) in options {
                        text(shown);
                    }
                }
                Kind::Button { label, .. } => text(label),
                Kind::Tooltip { value } => value.iter_mut().for_each(&mut text),
                _ => {}
            }
        }
        let mut number = |live: &mut LiveNum| {
            if let Some(source) = &live.source
                && let Some(result) = evaluator.number(source, &scope)
                && !result.is_nan()
            {
                live.value = result;
            }
        };
        for node in &mut self.nodes {
            match &mut node.kind {
                Kind::Bar { value, max, .. } => {
                    number(value);
                    number(max);
                }
                Kind::Number { min, max, step, .. } => {
                    min.iter_mut().for_each(&mut number);
                    max.iter_mut().for_each(&mut number);
                    number(step);
                }
                _ => {}
            }
        }
        for node in &mut self.nodes {
            if let Some(flag) = &mut node.show_if
                && let Some(source) = &flag.source
                && let Some(result) = evaluator.flag(source, &scope)
            {
                flag.value = result;
            }
        }
        self.stale = true;
    }

    /// The value a number widget's variable holds, as a float.
    pub(super) fn number_of(&self, var: &VarRef) -> f64 {
        match self.read(var) {
            Some(Value::Int(n)) => *n as f64,
            Some(Value::Real(r)) => r.get(),
            _ => 0.0,
        }
    }

    /// Stores a number in a variable, keeping a whole number whole.
    pub(super) fn store_number(&mut self, var: &VarRef, value: f64) {
        let whole = matches!(self.read(var), Some(Value::Int(_)) | None);
        let stored = if whole {
            Value::Int(value.round() as i64)
        } else {
            Value::Real(Real::new(value).unwrap_or(Real::ZERO))
        };
        self.write(var, stored);
    }
}
