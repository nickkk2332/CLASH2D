//! The interface toolkit (12 §10): screens made of a fixed set of
//! containers and widgets, laid out on the character grid and drawn into
//! a [`Frame`](crate::Frame).
//!
//! # Using a screen
//!
//! 1. Read a [`ScreenDef`] from data.
//! 2. Make a [`Screen`] from it with [`Screen::new`], giving the theme and
//!    the figures and art it may name in a [`UiEnv`]. Every problem in the
//!    definition is reported here, as a list of [`UiError`]s with paths.
//! 3. Each frame, call [`Screen::draw`]. It lays the screen out for the
//!    frame's size when that or the screen has changed.
//! 4. Give input to [`Screen::input`] with the keymap in force. It
//!    returns [`UiEvent`]s: effects to run and variables that changed.
//!
//! # Where bindings and effects plug in (package U6b)
//!
//! Every property that can be an expression is a [`Prop`]: `Static` with
//! its value, or `Bound` with its source unparsed. A screen shows a
//! placeholder for a bound property (a text shows its source, a number is
//! 0, a condition is true) until [`Screen::refresh`] is called with an
//! [`Evaluator`], which is the one thing U6b has to supply for texts,
//! numbers and conditions. Rows and options that come from a query
//! ([`RowsDef::Bound`], [`OptionsDef::Bound`]) are held and shown as
//! empty: filling them is U6b's.
//!
//! Effects ([`UiEffects`]) are held as written and handed back in
//! [`UiEvent`]s. Nothing here runs them.
//!
//! # Keys
//!
//! A screen never looks at keys directly. A key press is first named by
//! the keymap, in the screen's `context` (12 §9.2), and then offered, in
//! order, to:
//!
//! 1. the widget with the focus. A widget that edits text takes any key
//!    that types a character before its name is considered;
//! 2. the options of every `menu` on show, by their hotkeys;
//! 3. the screen's own `keys`, by name;
//! 4. the toolkit itself: moving the focus and scrolling.
//!
//! The names the toolkit and its widgets understand:
//!
//! | Name | Does |
//! |------|------|
//! | `prev`, `next` | Move the selection up and down in a list, table, tree, menu or log; scroll a `scroll`. Unused, move the focus. |
//! | `left`, `right` | Collapse and expand in a tree; choose the sort column of a table; step a `number`; change a `choice`; switch `tabs`; move the cursor in an `input`. Unused, move the focus. |
//! | `accept` | Activate the selected row or the button; flip a `toggle`; open or close a tree row with children. |
//! | `toggle` | Flip a `toggle`; reverse a table's sort; activate a button. |
//! | `page_prev`, `page_next`, `first`, `last` | Move by a page or to an end. Unused, scroll the `scroll` around the focus. |
//! | `focus_next`, `focus_prev` | Move the focus to the next or previous widget that can take it. |
//! | `erase`, `delete` | Remove the character before or at the cursor of an `input`. |
//! | `back` | Nothing by itself: screens bind it in `keys`, usually to `close`. |
//!
//! # Styles and marks the toolkit draws with
//!
//! The toolkit draws its own parts with theme styles of fixed names. A
//! theme that leaves one out gets the plain default in brackets.
//!
//! | Style | Used for |
//! |-------|----------|
//! | `title` | Frame titles (bold) |
//! | `border`, `border_focus` | Frame borders; the frame that holds the focus (plain; bold) |
//! | `selected` | The selected row or active part of the widget with the focus (reverse) |
//! | `selected_idle` | The selected row of a widget without the focus (underline) |
//! | `header` | Table headers and the shown tab (bold, underline) |
//! | `dim` | Unshown tabs, empty-list text, scrollbar tracks, empty views (dim) |
//! | `bar`, `bar_empty` | The filled and unfilled parts of a bar (plain; `dim`) |
//! | `field` | The field of an `input` (underline) |
//! | `button` | A button without the focus (plain) |
//! | `hotkey` | Menu hotkeys (bold) |
//! | `tip` | The `tooltip` widget's text (`dim`) |
//! | `notice` | The notice shown on a grid smaller than `min_size` (bold) |
//!
//! And with theme `marks` of fixed names, of which only the character is
//! used: `bar_full` (`█`), `bar_empty` (`░`), `scroll_thumb` (`█`),
//! `scroll_track` (`│`), `tree_open` (`▾`), `tree_closed` (`▸`),
//! `toggle_on` (`x`), `toggle_off` (space), `choice_prev` (`<`),
//! `choice_next` (`>`), `sort_up` (`▲`), `sort_down` (`▼`), `view_empty`
//! (`·`).
//!
//! # Not built yet
//!
//! - Bindings and effects: U6b, as above.
//! - `slot` holds its name and nothing else; `ui_part` is U8.
//! - `map_view`, `section_view`, `region_map` and `chart_view` have their
//!   place, focus and fields; a [`ViewPainter`] draws them. The scene's
//!   is [`crate::view::WorldView`], which draws the first two from the
//!   view model and the other two as empty areas with their names.
//! - `console`, `canvas` and `trace_view` have their place and focus;
//!   their content comes with the tooling packages.
//! - `log` shows a [`MessageLog`]; playback (U5) fills it.

pub mod bind;
pub mod def;
mod draw;
pub mod effect;
mod input;
mod layout;
mod refresh;
mod screen;

pub use bind::{
    Binding, KIND_APP, KIND_APP_ENTRY, KIND_ENTITY, SLOT_APP, SLOT_IT, SLOT_PARTY, SLOT_SCREEN,
    SLOT_VIEW, TYPE_PARTY_MEMBER,
};
pub use def::{
    Align, ArtDef, BarDef, ButtonDef, ChoiceDef, ColumnDef, CommonDef, DebugDef, FigureDef,
    FigureWidgetDef, FrameDef, GridDef, InputDef, InspectDef, ListDef, Literal, LogDef, MenuDef,
    NODE_FIELDS, NODE_KINDS, NodeDef, NodeKind, NumberDef, OptionDef, OptionsDef, Place, PopupDef,
    PrefKind, PreferenceDef, Prop, RowDef, RowsDef, ScreenDef, Size, SpacerDef, TabDef, TableDef,
    TabsDef, TextDef, TextProp, ToggleDef, TooltipDef, TreeDef, UiPartDef, ViewDef, text_prop,
};
pub use effect::{
    AppEffect, AppService, Args, Collect, CommandEffect, CommandKind, ConfirmEffect, EFFECT_NAMES,
    OpenEffect, PickEffect, UiEffect, UiEffects, ValueProp,
};
pub use refresh::{from_var, to_var};
pub use screen::{
    BindScope, CONTEXT_DEBUG, Evaluator, LogLine, MessageLog, Screen, SlotParts, UiData, UiEnv,
    UiError, UiEvent, UiProblem, VarRef, ViewKind, ViewPainter, WidgetEvent,
};

/// Navigation name: the previous row, or the line above.
pub const NAV_PREV: &str = "prev";
/// Navigation name: the next row, or the line below.
pub const NAV_NEXT: &str = "next";
/// Navigation name: leftward, less, collapse.
pub const NAV_LEFT: &str = "left";
/// Navigation name: rightward, more, expand.
pub const NAV_RIGHT: &str = "right";
/// Navigation name: activate what is selected.
pub const NAV_ACCEPT: &str = "accept";
/// Navigation name: flip what is selected.
pub const NAV_TOGGLE: &str = "toggle";
/// Navigation name: a page toward the start.
pub const NAV_PAGE_PREV: &str = "page_prev";
/// Navigation name: a page toward the end.
pub const NAV_PAGE_NEXT: &str = "page_next";
/// Navigation name: the first row, or the start.
pub const NAV_FIRST: &str = "first";
/// Navigation name: the last row, or the end.
pub const NAV_LAST: &str = "last";
/// Navigation name: focus the next widget.
pub const NAV_FOCUS_NEXT: &str = "focus_next";
/// Navigation name: focus the previous widget.
pub const NAV_FOCUS_PREV: &str = "focus_prev";
/// Navigation name: remove the character before the cursor.
pub const NAV_ERASE: &str = "erase";
/// Navigation name: remove the character at the cursor.
pub const NAV_DELETE: &str = "delete";
/// Navigation name: leave. The toolkit does nothing with it; screens bind
/// it in `keys`.
pub const NAV_BACK: &str = "back";
