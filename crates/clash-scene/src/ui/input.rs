//! Focus, keyboard navigation and the mouse.
//!
//! The order in which a key press is offered is in the documentation of
//! [`crate::ui`].

use clash_core::Value;

use crate::input::{
    CONTEXT_MENU, InputEvent, KeyChord, KeyCode, KeymapDef, MouseButton, MouseEvent, MouseKind,
};

use super::layout::keep_selection_in_view;
use super::screen::{Flavour, Kind, Listing, Screen, UiEvent, WidgetEvent};
use super::{
    NAV_ACCEPT, NAV_DELETE, NAV_ERASE, NAV_FIRST, NAV_FOCUS_NEXT, NAV_FOCUS_PREV, NAV_LAST,
    NAV_LEFT, NAV_NEXT, NAV_PAGE_NEXT, NAV_PAGE_PREV, NAV_PREV, NAV_RIGHT, NAV_TOGGLE,
};

/// How many lines one notch of the mouse wheel scrolls.
const WHEEL_LINES: i32 = 3;

impl Screen {
    /// Gives the screen one input event and returns what it asks for in
    /// answer: effects to run and variables that changed. `keymap` names
    /// the keys; the screen looks them up in its own context.
    ///
    /// The screen must have been laid out (by [`Screen::draw`] or
    /// [`Screen::layout`]) for focus and the mouse to mean anything.
    pub fn input(&mut self, event: &InputEvent, keymap: &KeymapDef) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if self.stale {
            let (cols, rows) = self.size;
            self.layout(cols, rows);
        }
        match event {
            InputEvent::Key(chord) => self.key(*chord, keymap, &mut out),
            InputEvent::Text(text) => {
                for ch in text.chars() {
                    if let Some(focus) = self.focus
                        && matches!(self.nodes[focus].kind, Kind::Input { .. })
                    {
                        self.widget_key(focus, KeyChord::char(ch), None, &mut out);
                    }
                }
            }
            InputEvent::Mouse(mouse) => self.mouse(mouse, &mut out),
            InputEvent::Resize { cols, rows } => {
                self.layout(*cols, *rows);
                self.reveal_focus();
            }
            InputEvent::Close => {}
        }
        if self.stale {
            let (cols, rows) = self.size;
            self.layout(cols, rows);
        }
        out
    }

    /// Moves the focus to the widget with this id, as the `focus` effect
    /// does. Returns false if there is none or it cannot take the focus
    /// now.
    pub fn set_focus(&mut self, id: &str) -> bool {
        match self.node_by_id(id) {
            Some(index) if self.focus_order.contains(&index) => {
                self.focus = Some(index);
                self.reveal_focus();
                true
            }
            _ => false,
        }
    }

    /// Moves the focus to the next (`1`) or previous (`-1`) widget that
    /// can take it, going round at the ends.
    pub fn move_focus(&mut self, by: i32) {
        if self.focus_order.is_empty() {
            return;
        }
        let count = self.focus_order.len() as i32;
        let at = self
            .focus
            .and_then(|focus| self.focus_order.iter().position(|f| *f == focus))
            .map_or(0, |at| (at as i32 + by).rem_euclid(count));
        self.focus = Some(self.focus_order[at as usize]);
        self.reveal_focus();
    }

    fn key(&mut self, chord: KeyChord, keymap: &KeymapDef, out: &mut Vec<UiEvent>) {
        // A context the keymap does not list, or a key it does not name
        // there, falls back to the menu context.
        let name = keymap
            .name(&self.context, chord)
            .or_else(|| keymap.name(CONTEXT_MENU, chord))
            .map(str::to_owned);
        let name = name.as_deref();
        if let Some(focus) = self.focus
            && self.widget_key(focus, chord, name, out)
        {
            return;
        }
        if self.menu_hotkey(chord, out) {
            return;
        }
        let Some(name) = name else { return };
        if let Some(effects) = self.keys.get(name) {
            out.push(UiEvent::Key {
                name: name.to_owned(),
                effects: effects.clone(),
            });
            return;
        }
        match name {
            NAV_FOCUS_NEXT | NAV_NEXT | NAV_RIGHT => self.move_focus(1),
            NAV_FOCUS_PREV | NAV_PREV | NAV_LEFT => self.move_focus(-1),
            NAV_PAGE_NEXT | NAV_PAGE_PREV | NAV_FIRST | NAV_LAST => self.scroll_nearest(name),
            _ => {}
        }
    }

    /// Scrolls the `scroll` around the focus, or failing that the first
    /// one on show.
    fn scroll_nearest(&mut self, name: &str) {
        let is_scroll = |screen: &Screen, index: usize| {
            screen.nodes[index].shown && matches!(screen.nodes[index].kind, Kind::Scroll { .. })
        };
        let mut found = None;
        let mut at = self.focus;
        while let Some(index) = at {
            if is_scroll(self, index) {
                found = Some(index);
                break;
            }
            at = self.nodes[index].parent;
        }
        let Some(index) = found.or_else(|| (0..self.nodes.len()).find(|i| is_scroll(self, *i)))
        else {
            return;
        };
        let page = self.nodes[index].inner.h.max(1);
        if let Kind::Scroll { y, .. } = &mut self.nodes[index].kind {
            match name {
                NAV_PAGE_NEXT => *y += page,
                NAV_PAGE_PREV => *y -= page,
                NAV_FIRST => *y = 0,
                _ => *y = i32::MAX / 2,
            }
            self.stale = true;
        }
    }

    /// A key that is the hotkey of an option of a menu on show selects
    /// and activates that option.
    fn menu_hotkey(&mut self, chord: KeyChord, out: &mut Vec<UiEvent>) -> bool {
        for index in 0..self.nodes.len() {
            if !self.nodes[index].shown {
                continue;
            }
            let Kind::Listing(listing) = &mut self.nodes[index].kind else {
                continue;
            };
            if listing.flavour != Flavour::Menu {
                continue;
            }
            let Some(at) = listing
                .shown
                .iter()
                .position(|row| listing.rows[*row].key == Some(chord))
            else {
                continue;
            };
            let before = listing.selected_row();
            listing.sel = at;
            self.focus = Some(index);
            let mut kind = core::mem::replace(&mut self.nodes[index].kind, Kind::Spacer);
            if let Kind::Listing(listing) = &mut kind {
                if listing.selected_row() != before {
                    self.selection_moved(index, listing, out);
                }
                self.activate_row(index, listing, out);
            }
            self.nodes[index].kind = kind;
            self.stale = true;
            return true;
        }
        false
    }

    fn selection_moved(&mut self, index: usize, listing: &Listing, out: &mut Vec<UiEvent>) {
        let row = listing.selected_row();
        if let (Some(name), Some(row)) = (&listing.var, row) {
            self.vars.set(name.as_str(), Value::Int(row as i64));
            out.push(UiEvent::Changed {
                var: super::VarRef::Screen(name.clone()),
            });
        }
        if !listing.on_select.is_empty() {
            out.push(UiEvent::Widget {
                id: self.nodes[index].id.clone(),
                event: WidgetEvent::Select,
                row,
                it: row.map(|row| listing.it_of(row)),
                effects: listing.on_select.clone(),
            });
        }
    }

    fn activate_row(&mut self, index: usize, listing: &mut Listing, out: &mut Vec<UiEvent>) {
        let Some(row) = listing.selected_row() else {
            return;
        };
        if listing.flavour.nests() && listing.rows[row].has_children {
            listing.rows[row].open = !listing.rows[row].open;
            listing.reshow();
            return;
        }
        for effects in [&listing.rows[row].on_activate, &listing.on_activate] {
            if !effects.is_empty() {
                out.push(UiEvent::Widget {
                    id: self.nodes[index].id.clone(),
                    event: WidgetEvent::Activate,
                    row: Some(row),
                    it: Some(listing.it_of(row)),
                    effects: effects.clone(),
                });
            }
        }
    }

    fn changed(
        &mut self,
        index: usize,
        var: &super::VarRef,
        effects: &super::UiEffects,
        out: &mut Vec<UiEvent>,
    ) {
        out.push(UiEvent::Changed { var: var.clone() });
        if !effects.is_empty() {
            out.push(UiEvent::Widget {
                id: self.nodes[index].id.clone(),
                event: WidgetEvent::Change,
                row: None,
                it: None,
                effects: effects.clone(),
            });
        }
        self.stale = true;
    }

    /// Offers a key to a widget. Returns whether the widget used it.
    fn widget_key(
        &mut self,
        index: usize,
        chord: KeyChord,
        name: Option<&str>,
        out: &mut Vec<UiEvent>,
    ) -> bool {
        // The widget's state is taken out while it is worked on, so that
        // the screen's variables can be read and written beside it.
        let mut kind = core::mem::replace(&mut self.nodes[index].kind, Kind::Spacer);
        let used = self.kind_key(index, &mut kind, chord, name, out);
        self.nodes[index].kind = kind;
        used
    }

    fn kind_key(
        &mut self,
        index: usize,
        kind: &mut Kind,
        chord: KeyChord,
        name: Option<&str>,
        out: &mut Vec<UiEvent>,
    ) -> bool {
        let inner = self.nodes[index].inner;
        match kind {
            Kind::Listing(listing) => {
                let header = i32::from(listing.flavour == Flavour::Table);
                let visible = (inner.h - header).max(1) as usize;
                let last = listing.shown.len().saturating_sub(1);
                let before = listing.selected_row();
                let Some(row) = before else { return false };
                match name {
                    Some(NAV_PREV) => listing.sel = listing.sel.saturating_sub(1),
                    Some(NAV_NEXT) => listing.sel = (listing.sel + 1).min(last),
                    Some(NAV_PAGE_PREV) => listing.sel = listing.sel.saturating_sub(visible),
                    Some(NAV_PAGE_NEXT) => listing.sel = (listing.sel + visible).min(last),
                    Some(NAV_FIRST) => listing.sel = 0,
                    Some(NAV_LAST) => listing.sel = last,
                    Some(NAV_ACCEPT) => self.activate_row(index, listing, out),
                    Some(NAV_RIGHT) if listing.flavour.nests() => {
                        if listing.rows[row].has_children && !listing.rows[row].open {
                            listing.rows[row].open = true;
                            listing.reshow();
                        } else if listing.rows[row].has_children {
                            listing.sel = (listing.sel + 1).min(last);
                        }
                    }
                    Some(NAV_LEFT) if listing.flavour.nests() => {
                        if listing.rows[row].has_children && listing.rows[row].open {
                            listing.rows[row].open = false;
                            listing.reshow();
                        } else if let Some(parent) = listing.rows[row].parent
                            && let Some(at) = listing.shown.iter().position(|r| *r == parent)
                        {
                            listing.sel = at;
                        }
                    }
                    Some(direction @ (NAV_LEFT | NAV_RIGHT))
                        if listing.flavour == Flavour::Table =>
                    {
                        // Step through: unsorted, then each column in turn.
                        let columns = listing.columns.len();
                        let forward = direction == NAV_RIGHT;
                        listing.sort = match (listing.sort, forward) {
                            (None, true) if columns > 0 => Some((0, false)),
                            (None, false) if columns > 0 => Some((columns - 1, false)),
                            (Some((c, _)), true) if c + 1 < columns => Some((c + 1, false)),
                            (Some((c, _)), false) if c > 0 => Some((c - 1, false)),
                            _ => None,
                        };
                        listing.reshow();
                    }
                    Some(NAV_TOGGLE)
                        if listing.flavour == Flavour::Table && listing.sort.is_some() =>
                    {
                        listing.sort = listing.sort.map(|(c, descending)| (c, !descending));
                        listing.reshow();
                    }
                    _ => return false,
                }
                keep_selection_in_view(listing, visible);
                if listing.selected_row() != before {
                    self.selection_moved(index, listing, out);
                }
                self.stale = true;
                true
            }
            Kind::Tabs {
                titles,
                var,
                active,
            } => {
                let pages = titles.len();
                if pages == 0 {
                    return false;
                }
                match name {
                    Some(NAV_RIGHT) => *active = (*active + 1) % pages,
                    Some(NAV_LEFT) => *active = (*active + pages - 1) % pages,
                    _ => return false,
                }
                if let Some(name) = var {
                    self.vars.set(name.as_str(), Value::Int(*active as i64));
                    out.push(UiEvent::Changed {
                        var: super::VarRef::Screen(name.clone()),
                    });
                }
                self.stale = true;
                true
            }
            Kind::Scroll { x, y, .. } => {
                let page = inner.h.max(1);
                match name {
                    Some(NAV_PREV) => *y -= 1,
                    Some(NAV_NEXT) => *y += 1,
                    Some(NAV_LEFT) => *x -= 1,
                    Some(NAV_RIGHT) => *x += 1,
                    Some(NAV_PAGE_PREV) => *y -= page,
                    Some(NAV_PAGE_NEXT) => *y += page,
                    Some(NAV_FIRST) => *y = 0,
                    Some(NAV_LAST) => *y = i32::MAX / 2,
                    _ => return false,
                }
                self.stale = true;
                true
            }
            Kind::Log { back, .. } => {
                let page = inner.h.max(1) as usize;
                match name {
                    Some(NAV_PREV) => *back = back.saturating_add(1),
                    Some(NAV_NEXT) => *back = back.saturating_sub(1),
                    Some(NAV_PAGE_PREV) => *back = back.saturating_add(page),
                    Some(NAV_PAGE_NEXT) => *back = back.saturating_sub(page),
                    Some(NAV_FIRST) => *back = usize::MAX,
                    Some(NAV_LAST) => *back = 0,
                    _ => return false,
                }
                true
            }
            Kind::Input {
                var,
                max_len,
                cursor,
                on_change,
                ..
            } => {
                let mut text: Vec<char> = match self.read(var) {
                    Some(Value::Str(text)) => text.chars().collect(),
                    _ => Vec::new(),
                };
                let mut at = (*cursor).min(text.len());
                let mut edited = false;
                if let Some(ch) = chord.typed().filter(|ch| !ch.is_control()) {
                    if text.len() < *max_len {
                        text.insert(at, ch);
                        at += 1;
                        edited = true;
                    }
                } else {
                    match name {
                        Some(NAV_ERASE) if at > 0 => {
                            text.remove(at - 1);
                            at -= 1;
                            edited = true;
                        }
                        Some(NAV_DELETE) if at < text.len() => {
                            text.remove(at);
                            edited = true;
                        }
                        Some(NAV_ERASE | NAV_DELETE) => {}
                        Some(NAV_LEFT) => at = at.saturating_sub(1),
                        Some(NAV_RIGHT) => at = (at + 1).min(text.len()),
                        Some(NAV_FIRST) => at = 0,
                        Some(NAV_LAST) => at = text.len(),
                        _ => return false,
                    }
                }
                *cursor = at;
                if edited {
                    self.write(var, Value::Str(text.into_iter().collect()));
                    self.changed(index, var, on_change, out);
                }
                true
            }
            Kind::Number {
                var,
                min,
                max,
                step,
                on_change,
                ..
            } => {
                let by = match name {
                    Some(NAV_RIGHT) => step.value,
                    Some(NAV_LEFT) => -step.value,
                    _ => return false,
                };
                let before = self.number_of(var);
                let mut value = before + by;
                if let Some(max) = max {
                    value = value.min(max.value);
                }
                if let Some(min) = min {
                    value = value.max(min.value);
                }
                if value != before {
                    self.store_number(var, value);
                    self.changed(index, var, on_change, out);
                }
                true
            }
            Kind::Toggle { var, on_change, .. } => {
                if !matches!(name, Some(NAV_ACCEPT | NAV_TOGGLE)) {
                    return false;
                }
                let on = matches!(self.read(var), Some(Value::Bool(true)));
                self.write(var, Value::Bool(!on));
                self.changed(index, var, on_change, out);
                true
            }
            Kind::Choice {
                var,
                options,
                on_change,
                ..
            } => {
                let count = options.len();
                let forward = match name {
                    Some(NAV_RIGHT | NAV_ACCEPT | NAV_TOGGLE) => true,
                    Some(NAV_LEFT) => false,
                    _ => return false,
                };
                if count == 0 {
                    return true;
                }
                let held = self.read(var);
                let at = options
                    .iter()
                    .position(|(value, _)| Some(value) == held)
                    .unwrap_or(0);
                let next = if forward {
                    (at + 1) % count
                } else {
                    (at + count - 1) % count
                };
                if next != at || held != Some(&options[next].0) {
                    self.write(var, options[next].0.clone());
                    self.changed(index, var, on_change, out);
                }
                true
            }
            Kind::Button { on_activate, .. } => {
                if !matches!(name, Some(NAV_ACCEPT | NAV_TOGGLE)) {
                    return false;
                }
                out.push(UiEvent::Widget {
                    id: self.nodes[index].id.clone(),
                    event: WidgetEvent::Activate,
                    row: None,
                    it: None,
                    effects: on_activate.clone(),
                });
                true
            }
            _ => false,
        }
    }

    fn mouse(&mut self, mouse: &MouseEvent, out: &mut Vec<UiEvent>) {
        let (x, y) = (i32::from(mouse.col), i32::from(mouse.row));
        // The last node on show under the pointer is the one on top.
        let hit = (0..self.nodes.len()).rev().find(|index| {
            let node = &self.nodes[*index];
            node.shown && node.outer.intersect(node.bounds).contains(x, y)
        });
        let up_from = |screen: &Screen, from: Option<usize>, wanted: &dyn Fn(&Kind) -> bool| {
            let mut at = from;
            while let Some(index) = at {
                if wanted(&screen.nodes[index].kind) {
                    return Some(index);
                }
                at = screen.nodes[index].parent;
            }
            None
        };
        match mouse.kind {
            MouseKind::Move => {
                let mut at = hit;
                self.hover = None;
                while let Some(index) = at {
                    if self.nodes[index].tip.is_some() {
                        self.hover = Some(index);
                        break;
                    }
                    at = self.nodes[index].parent;
                }
            }
            MouseKind::Down(MouseButton::Left) => {
                let Some(index) = up_from(self, hit, &|kind| kind.focusable()) else {
                    return;
                };
                if !self.focus_order.contains(&index) {
                    return;
                }
                self.focus = Some(index);
                self.click(index, x, y, out);
            }
            MouseKind::Wheel(notches) => {
                let scrolls = |kind: &Kind| {
                    matches!(
                        kind,
                        Kind::Listing(_) | Kind::Log { .. } | Kind::Scroll { .. }
                    )
                };
                let Some(index) = up_from(self, hit, &scrolls) else {
                    return;
                };
                let lines = i32::from(notches) * WHEEL_LINES;
                let inner = self.nodes[index].inner;
                match &mut self.nodes[index].kind {
                    Kind::Scroll { y, .. } => *y += lines,
                    Kind::Log { back, .. } => {
                        *back = if lines < 0 {
                            back.saturating_add(lines.unsigned_abs() as usize)
                        } else {
                            back.saturating_sub(lines as usize)
                        };
                    }
                    Kind::Listing(listing) => {
                        // A listing keeps its selection in view, so the
                        // wheel moves the selection.
                        let before = listing.selected_row();
                        let last = listing.shown.len().saturating_sub(1) as i32;
                        listing.sel = (listing.sel as i32 + i32::from(notches))
                            .clamp(0, last.max(0)) as usize;
                        let header = i32::from(listing.flavour == Flavour::Table);
                        keep_selection_in_view(listing, (inner.h - header).max(1) as usize);
                        if listing.selected_row() != before {
                            let mut kind =
                                core::mem::replace(&mut self.nodes[index].kind, Kind::Spacer);
                            if let Kind::Listing(listing) = &kind {
                                self.selection_moved(index, listing, out);
                            }
                            core::mem::swap(&mut self.nodes[index].kind, &mut kind);
                        }
                    }
                    _ => {}
                }
                self.stale = true;
            }
            _ => {}
        }
    }

    /// A click on a widget that has just been given the focus.
    fn click(&mut self, index: usize, x: i32, y: i32, out: &mut Vec<UiEvent>) {
        let inner = self.nodes[index].inner;
        let accept = KeyChord::plain(KeyCode::Enter);
        let mut press = None;
        let mut tab = None;
        match &mut self.nodes[index].kind {
            Kind::Button { .. } | Kind::Toggle { .. } | Kind::Choice { .. } => {
                press = Some(NAV_ACCEPT)
            }
            Kind::Tabs { titles, .. } if y == inner.y => {
                let mut left = inner.x;
                for (page, title) in titles.iter().enumerate() {
                    let width = title.text.len() as i32 + 2;
                    if x >= left && x < left + width {
                        tab = Some(page);
                        break;
                    }
                    left += width + 1;
                }
            }
            Kind::Listing(listing) => {
                let header = i32::from(listing.flavour == Flavour::Table);
                let line = y - inner.y - header;
                if line < 0 {
                    return;
                }
                let at = listing.top + line as usize;
                if at >= listing.shown.len() {
                    return;
                }
                if at == listing.sel {
                    press = Some(NAV_ACCEPT);
                } else {
                    listing.sel = at;
                    let mut kind = core::mem::replace(&mut self.nodes[index].kind, Kind::Spacer);
                    if let Kind::Listing(listing) = &kind {
                        self.selection_moved(index, listing, out);
                    }
                    core::mem::swap(&mut self.nodes[index].kind, &mut kind);
                    self.stale = true;
                }
            }
            _ => {}
        }
        if let Some(page) = tab {
            self.set_tab(index, page, out);
        }
        if let Some(name) = press {
            self.widget_key(index, accept, Some(name), out);
        }
    }

    fn set_tab(&mut self, index: usize, page: usize, out: &mut Vec<UiEvent>) {
        if let Kind::Tabs { var, active, .. } = &mut self.nodes[index].kind {
            *active = page;
            if let Some(name) = var.clone() {
                self.vars.set(name.as_str(), Value::Int(page as i64));
                out.push(UiEvent::Changed {
                    var: super::VarRef::Screen(name),
                });
            }
            self.stale = true;
        }
    }
}
