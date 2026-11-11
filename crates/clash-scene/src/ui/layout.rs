//! Layout on the character grid (12 §10.2).
//!
//! A layout pass gives every node on show a rectangle. Sizes come from,
//! in order: the node's own `width` and `height`; among `rows` and `cols`
//! its `fill` or `fit`; otherwise what its kind does by default, which is
//! to fit its content, except for the kinds that are useful at any size
//! (lists, tables, trees, the log, views, scrolls, spacers), which fill.
//! A container fills if anything in it does.

use crate::frame::Rect;

use super::def::Size;
use super::screen::{Flavour, Kind, Screen};

/// The width of the drawn part of a `bar` that is given no size.
pub(super) const BAR_NATURAL: i32 = 10;
/// The width of the field of an `input` that is given no size.
pub(super) const FIELD_NATURAL: i32 = 16;
/// The size of a view of the world that is given none.
const VIEW_NATURAL: (i32, i32) = (20, 8);
/// The size of a `log` that is given none.
const LOG_NATURAL: (i32, i32) = (20, 3);
/// How far each level of a tree is indented.
pub(super) const TREE_INDENT: i32 = 2;

/// The size a `width` or `height` fixes, if it fixes one.
fn fixed(size: Option<Size>, avail: i32) -> Option<i32> {
    match size? {
        Size::Cells(cells) => Some(i32::from(cells)),
        Size::Percent(percent) => Some(avail * i32::from(percent) / 100),
        Size::Fit | Size::Fill(_) => None,
    }
}

/// Shares what `avail` leaves after the fixed sizes among the entries
/// with a weight, in proportion, giving any cells left over by rounding
/// to the first of them.
fn share(sizes: &mut [i32], weights: &[u16], avail: i32) {
    let total: i32 = weights.iter().map(|w| i32::from(*w)).sum();
    if total == 0 {
        return;
    }
    let fixed: i32 = sizes
        .iter()
        .zip(weights)
        .filter(|(_, w)| **w == 0)
        .map(|(s, _)| *s)
        .sum();
    let rest = (avail - fixed).max(0);
    let mut given = 0;
    for (size, weight) in sizes.iter_mut().zip(weights) {
        if *weight > 0 {
            *size = rest * i32::from(*weight) / total;
            given += *size;
        }
    }
    let mut left = rest - given;
    for (size, weight) in sizes.iter_mut().zip(weights) {
        if left == 0 {
            break;
        }
        if *weight > 0 {
            *size += 1;
            left -= 1;
        }
    }
}

/// Writes a number as a gauge and a number widget show it: whole if it
/// is whole, otherwise to one decimal place.
pub(super) fn write_number(out: &mut impl core::fmt::Write, value: f64) -> core::fmt::Result {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        write!(out, "{}", value as i64)
    } else {
        write!(out, "{value:.1}")
    }
}

/// A number as [`write_number`] writes it.
pub(super) fn number_text(value: f64) -> String {
    let mut text = String::new();
    let _ = write_number(&mut text, value);
    text
}

enum Tag {
    Line { vertical: bool },
    Stack,
    Grid,
    Tabs,
    Scroll,
    Leaf,
}

impl Screen {
    pub(super) fn visible(&self, index: usize) -> bool {
        self.nodes[index]
            .show_if
            .as_ref()
            .is_none_or(|flag| flag.value)
    }

    /// Whether the grid of the last layout is smaller than the screen's
    /// `min_size`.
    pub fn too_small(&self) -> bool {
        self.size.0 < self.min_size.0 || self.size.1 < self.min_size.1
    }

    fn tag(&self, index: usize) -> Tag {
        match &self.nodes[index].kind {
            Kind::Rows => Tag::Line { vertical: true },
            Kind::Cols => Tag::Line { vertical: false },
            Kind::Stack => Tag::Stack,
            Kind::Grid { .. } => Tag::Grid,
            Kind::Tabs { .. } => Tag::Tabs,
            Kind::Scroll { .. } => Tag::Scroll,
            _ => Tag::Leaf,
        }
    }

    /// One past the last node of the subtree rooted at `index`. Nodes are
    /// numbered parent first, so a subtree is a run of numbers.
    pub(super) fn subtree_end(&self, index: usize) -> usize {
        let mut last = index;
        while let Some(child) = self.nodes[last].children.last() {
            last = *child;
        }
        last + 1
    }

    /// Whether a node takes whatever space is left along an axis when
    /// nothing says how large it is.
    fn wants_fill(&self, index: usize, vertical: bool) -> bool {
        let node = &self.nodes[index];
        match if vertical { node.height } else { node.width } {
            Some(Size::Fill(_)) => return true,
            Some(_) => return false,
            None => {}
        }
        let any_child = |along_line: bool| {
            node.children.iter().any(|child| {
                self.visible(*child)
                    && (self.wants_fill(*child, vertical)
                        || (along_line && self.nodes[*child].fill.is_some()))
            })
        };
        match &node.kind {
            Kind::Rows => any_child(vertical),
            Kind::Cols => any_child(!vertical),
            Kind::Stack => any_child(false),
            Kind::Grid { columns, rows } => {
                let tracks = if vertical { rows } else { columns };
                tracks.iter().any(|size| matches!(size, Size::Fill(_)))
            }
            Kind::Tabs { .. } => !vertical || any_child(false),
            Kind::Scroll { .. }
            | Kind::Log { .. }
            | Kind::View { .. }
            | Kind::Debug { .. }
            | Kind::Spacer => true,
            Kind::Listing(listing) => listing.flavour != Flavour::Menu,
            Kind::Bar { gauge, .. } => !vertical && !gauge,
            Kind::Input { .. } | Kind::Tooltip { .. } => !vertical,
            _ => false,
        }
    }

    /// Lays the screen out for a grid of this size. Drawing does this
    /// itself when the size or the screen has changed.
    pub fn layout(&mut self, cols: u16, rows: u16) {
        self.size = (cols, rows);
        self.stale = false;
        for node in &mut self.nodes {
            node.shown = false;
        }
        let area = Rect::new(0, 0, i32::from(cols), i32::from(rows));
        if !self.nodes.is_empty() && self.visible(0) && !self.too_small() {
            self.place(0, area, area);
        }
        self.focus_order.clear();
        for index in 0..self.nodes.len() {
            if self.nodes[index].shown && self.nodes[index].kind.focusable() {
                self.focus_order.push(index);
            }
        }
        if !self
            .focus
            .is_some_and(|focus| self.focus_order.contains(&focus))
        {
            self.focus = self.focus_order.first().copied();
        }
        if self.hover.is_some_and(|hover| !self.nodes[hover].shown) {
            self.hover = None;
        }
    }

    /// Scrolls every `scroll` around the focused node so that the node is
    /// in view, and lays out again if anything moved.
    pub(super) fn reveal_focus(&mut self) {
        // One pass per level of nesting is enough; the bound is a guard.
        for _ in 0..8 {
            let Some(focus) = self.focus else { return };
            let target = self.nodes[focus].outer;
            let mut moved = false;
            let mut ancestor = self.nodes[focus].parent;
            while let Some(index) = ancestor {
                let view = self.nodes[index].inner;
                if let Kind::Scroll { x, y, .. } = &mut self.nodes[index].kind {
                    let before = (*x, *y);
                    if target.y < view.y {
                        *y -= view.y - target.y;
                    } else if target.bottom() > view.bottom() {
                        *y += (target.bottom() - view.bottom()).min(target.y - view.y);
                    }
                    if target.x < view.x {
                        *x -= view.x - target.x;
                    } else if target.right() > view.right() {
                        *x += (target.right() - view.right()).min(target.x - view.x);
                    }
                    moved |= before != (*x, *y);
                }
                ancestor = self.nodes[index].parent;
            }
            if !moved {
                return;
            }
            let (cols, rows) = self.size;
            self.layout(cols, rows);
        }
    }

    /// The size a node would like, border included, given the space on
    /// offer. It may be larger than the space.
    fn measure(&mut self, index: usize, avail_w: i32, avail_h: i32) -> (i32, i32) {
        let node = &self.nodes[index];
        let border = if node.frame.is_some() { 2 } else { 0 };
        let set_w = fixed(node.width, avail_w);
        let set_h = fixed(node.height, avail_h);
        let content_w = (set_w.unwrap_or(avail_w) - border).max(0);
        let content_h = (set_h.unwrap_or(avail_h) - border).max(0);
        let title_w = match &node.frame {
            Some(Some(title)) => title.text.len() as i32 + 4,
            _ => 0,
        };
        let (w, h) = self.measure_content(index, content_w, content_h);
        (
            set_w.unwrap_or((w + border).max(title_w)),
            set_h.unwrap_or(h + border),
        )
    }

    fn measure_content(&mut self, index: usize, avail_w: i32, avail_h: i32) -> (i32, i32) {
        let count = self.nodes[index].children.len();
        match self.tag(index) {
            Tag::Line { vertical } => {
                let (mut along, mut across) = (0, 0);
                for k in 0..count {
                    let child = self.nodes[index].children[k];
                    if !self.visible(child) {
                        continue;
                    }
                    let (w, h) = self.measure(child, avail_w, avail_h);
                    let (a, c) = if vertical { (h, w) } else { (w, h) };
                    along += a;
                    across = across.max(c);
                }
                if vertical {
                    (across, along)
                } else {
                    (along, across)
                }
            }
            Tag::Stack => {
                let (mut w, mut h) = (0, 0);
                for k in 0..count {
                    let child = self.nodes[index].children[k];
                    if self.visible(child) {
                        let (cw, ch) = self.measure(child, avail_w, avail_h);
                        w = w.max(cw);
                        h = h.max(ch);
                    }
                }
                (w, h)
            }
            Tag::Grid => {
                let (widths, heights) = self.grid_tracks(index, avail_w, avail_h);
                (widths.iter().sum(), heights.iter().sum())
            }
            Tag::Tabs => {
                let strip = self.tab_strip_width(index);
                let (mut w, mut h) = (strip, 0);
                // Every page counts, so the size does not jump on a switch.
                for k in 0..count {
                    let child = self.nodes[index].children[k];
                    let (cw, ch) = self.measure(child, avail_w, (avail_h - 1).max(0));
                    w = w.max(cw);
                    h = h.max(ch);
                }
                (w, h + 1)
            }
            Tag::Scroll => match self.nodes[index].children.first().copied() {
                Some(child) => {
                    let (w, h) = self.measure(child, (avail_w - 1).max(0), avail_h);
                    (w + 1, h)
                }
                None => (0, 0),
            },
            Tag::Leaf => self.leaf_size(index, avail_w),
        }
    }

    pub(super) fn tab_strip_width(&self, index: usize) -> i32 {
        match &self.nodes[index].kind {
            Kind::Tabs { titles, .. } => {
                let text: i32 = titles.iter().map(|title| title.text.len() as i32 + 2).sum();
                text + (titles.len() as i32 - 1).max(0)
            }
            _ => 0,
        }
    }

    /// The natural size of a widget's content.
    fn leaf_size(&mut self, index: usize, avail_w: i32) -> (i32, i32) {
        let label_w = |label: &Option<super::screen::LiveText>| {
            label.as_ref().map_or(0, |l| l.text.len() as i32 + 1)
        };
        match &mut self.nodes[index].kind {
            Kind::Text {
                value, wrap, lines, ..
            } => {
                let width = if *wrap {
                    avail_w.max(1) as usize
                } else {
                    usize::MAX
                };
                value.text.wrap(width, lines);
                let widest = lines.iter().map(|(from, to)| to - from).max().unwrap_or(0);
                (widest as i32, lines.len() as i32)
            }
            Kind::Art { rows } | Kind::Figure { rows, .. } => (
                rows.iter().map(Vec::len).max().unwrap_or(0) as i32,
                rows.len() as i32,
            ),
            Kind::Listing(listing) => {
                let empty_w = listing.empty.as_ref().map_or(0, |e| e.text.len() as i32);
                let rows = listing.shown.len().max(1) as i32;
                let row_w = |row: &super::screen::Row| -> i32 {
                    let label = row.label.text.len() as i32;
                    match listing.flavour {
                        Flavour::List | Flavour::Table => label,
                        Flavour::Menu => listing.key_width + label,
                        Flavour::Tree => i32::from(row.depth) * TREE_INDENT + 2 + label,
                        Flavour::Inspect => {
                            let value = row.value.as_ref().map_or(0, |v| v.text.len() as i32 + 2);
                            i32::from(row.depth) * TREE_INDENT + 2 + label + value
                        }
                    }
                };
                if listing.flavour == Flavour::Table {
                    let widths: i32 = (0..listing.columns.len())
                        .map(|c| natural_column(listing, c))
                        .sum();
                    let gaps = (listing.columns.len() as i32 - 1).max(0);
                    ((widths + gaps).max(empty_w), rows + 1)
                } else {
                    let widest = listing
                        .shown
                        .iter()
                        .map(|r| row_w(&listing.rows[*r]))
                        .max()
                        .unwrap_or(0);
                    (widest.max(empty_w), rows)
                }
            }
            Kind::Bar {
                value,
                max,
                label,
                gauge,
            } => {
                if *gauge {
                    let numbers = number_text(value.value).len() + 1 + number_text(max.value).len();
                    (label_w(label) + numbers as i32, 1)
                } else {
                    (label_w(label) + BAR_NATURAL, 1)
                }
            }
            Kind::Input { label, .. } => (label_w(label) + FIELD_NATURAL, 1),
            Kind::Number { label, .. } => (label_w(label) + 8, 1),
            Kind::Toggle { label, .. } => (3 + label_w(label), 1),
            Kind::Choice { label, options, .. } => {
                let widest = options
                    .iter()
                    .map(|(_, shown)| shown.text.len())
                    .max()
                    .unwrap_or(0) as i32;
                (label_w(label) + widest + 4, 1)
            }
            Kind::Button { label, .. } => (label.text.len() as i32 + 4, 1),
            Kind::Log { .. } => LOG_NATURAL,
            Kind::View { .. } | Kind::Debug { .. } => VIEW_NATURAL,
            Kind::Tooltip { value } => (value.as_ref().map_or(0, |v| v.text.len() as i32), 1),
            _ => (0, 0),
        }
    }

    /// The widths of a grid's columns and the heights of its rows.
    fn grid_tracks(&mut self, index: usize, avail_w: i32, avail_h: i32) -> (Vec<i32>, Vec<i32>) {
        let (col_specs, row_specs) = match &self.nodes[index].kind {
            Kind::Grid { columns, rows } => (columns.clone(), rows.clone()),
            _ => return (Vec::new(), Vec::new()),
        };
        let ncols = col_specs.len().max(1);
        let count = self.nodes[index].children.len();
        let nrows = count.div_ceil(ncols);
        let cell = |screen: &Screen, row: usize, col: usize| -> Option<usize> {
            let child = screen.nodes[index]
                .children
                .get(row * ncols + col)
                .copied()?;
            screen.visible(child).then_some(child)
        };

        let mut widths = vec![0; ncols];
        let mut weights = vec![0u16; ncols];
        for (col, spec) in col_specs.iter().enumerate() {
            match spec {
                Size::Cells(_) | Size::Percent(_) => {
                    widths[col] = fixed(Some(*spec), avail_w).unwrap_or(0)
                }
                Size::Fill(weight) => weights[col] = *weight,
                Size::Fit => {
                    for row in 0..nrows {
                        if let Some(child) = cell(self, row, col) {
                            widths[col] = widths[col].max(self.measure(child, avail_w, avail_h).0);
                        }
                    }
                }
            }
        }
        share(&mut widths, &weights, avail_w);

        let mut heights = vec![0; nrows];
        let mut weights = vec![0u16; nrows];
        for row in 0..nrows {
            let spec = row_specs.get(row).copied().unwrap_or(Size::Fit);
            match spec {
                Size::Cells(_) | Size::Percent(_) => {
                    heights[row] = fixed(Some(spec), avail_h).unwrap_or(0)
                }
                Size::Fill(weight) => weights[row] = weight,
                Size::Fit => {
                    for (col, width) in widths.iter().enumerate() {
                        if let Some(child) = cell(self, row, col) {
                            heights[row] = heights[row].max(self.measure(child, *width, avail_h).1);
                        }
                    }
                }
            }
        }
        share(&mut heights, &weights, avail_h);
        (widths, heights)
    }

    /// Gives a node its rectangle and lays out what is inside it.
    fn place(&mut self, index: usize, rect: Rect, clip: Rect) {
        let inner = if self.nodes[index].frame.is_some() {
            rect.inset(1)
        } else {
            rect
        };
        let content_clip = clip.intersect(inner);
        {
            let node = &mut self.nodes[index];
            node.shown = true;
            node.outer = rect;
            node.inner = inner;
            node.clip = content_clip;
            node.bounds = clip;
        }
        let count = self.nodes[index].children.len();
        match self.tag(index) {
            Tag::Line { vertical } => self.place_line(index, vertical, inner, content_clip),
            Tag::Stack => {
                for k in 0..count {
                    let child = self.nodes[index].children[k];
                    if !self.visible(child) {
                        continue;
                    }
                    let natural = self.measure(child, inner.w, inner.h);
                    let node = &self.nodes[child];
                    let w = match node.width {
                        Some(Size::Fill(_)) | None => inner.w,
                        Some(_) => natural.0.min(inner.w),
                    };
                    let h = match node.height {
                        Some(Size::Fill(_)) | None => inner.h,
                        Some(_) => natural.1.min(inner.h),
                    };
                    let rect = Rect::new(
                        inner.x + (inner.w - w) / 2,
                        inner.y + (inner.h - h) / 2,
                        w,
                        h,
                    );
                    self.place(child, rect, content_clip);
                }
            }
            Tag::Grid => {
                let (widths, heights) = self.grid_tracks(index, inner.w, inner.h);
                let ncols = widths.len().max(1);
                let mut y = inner.y;
                for (row, height) in heights.iter().enumerate() {
                    let mut x = inner.x;
                    for (col, width) in widths.iter().enumerate() {
                        if let Some(child) =
                            self.nodes[index].children.get(row * ncols + col).copied()
                            && self.visible(child)
                        {
                            self.place(child, Rect::new(x, y, *width, *height), content_clip);
                        }
                        x += width;
                    }
                    y += height;
                }
            }
            Tag::Tabs => {
                let active = match &mut self.nodes[index].kind {
                    Kind::Tabs { active, .. } => {
                        *active = (*active).min(count.saturating_sub(1));
                        *active
                    }
                    _ => 0,
                };
                if let Some(child) = self.nodes[index].children.get(active).copied()
                    && self.visible(child)
                {
                    let page = Rect::new(inner.x, inner.y + 1, inner.w, inner.h - 1);
                    self.place(child, page, content_clip.intersect(page));
                }
            }
            Tag::Scroll => self.place_scroll(index, inner, content_clip),
            Tag::Leaf => self.settle_leaf(index),
        }
    }

    fn place_scroll(&mut self, index: usize, inner: Rect, clip: Rect) {
        let Some(child) = self.nodes[index].children.first().copied() else {
            return;
        };
        if !self.visible(child) {
            return;
        }
        let (mut natural_w, mut natural_h) = self.measure(child, inner.w, inner.h);
        let mut view_w = inner.w;
        if natural_h > inner.h {
            // Keep a column for the scrollbar.
            view_w = (inner.w - 1).max(0);
            (natural_w, natural_h) = self.measure(child, view_w, inner.h);
        }
        let w = if natural_w <= view_w || self.wants_fill(child, false) {
            view_w
        } else {
            natural_w
        };
        let h = if self.wants_fill(child, true) {
            natural_h.max(inner.h)
        } else {
            natural_h
        };
        let (mut off_x, mut off_y) = (0, 0);
        if let Kind::Scroll {
            x,
            y,
            content_w,
            content_h,
            ..
        } = &mut self.nodes[index].kind
        {
            *content_w = w;
            *content_h = h;
            *x = (*x).clamp(0, (w - view_w).max(0));
            *y = (*y).clamp(0, (h - inner.h).max(0));
            (off_x, off_y) = (*x, *y);
        }
        let view = Rect::new(inner.x, inner.y, view_w, inner.h);
        self.place(
            child,
            Rect::new(inner.x - off_x, inner.y - off_y, w, h),
            clip.intersect(view),
        );
        let inside = index + 1..self.subtree_end(index);
        let holds_focusable = inside
            .into_iter()
            .any(|k| self.nodes[k].shown && self.nodes[k].kind.focusable());
        if let Kind::Scroll { own_focus, .. } = &mut self.nodes[index].kind {
            *own_focus = !holds_focusable && (h > inner.h || w > view_w);
        }
    }

    /// Lays out the children of `rows` (vertical) or `cols`.
    fn place_line(&mut self, index: usize, vertical: bool, area: Rect, clip: Rect) {
        let count = self.nodes[index].children.len();
        let (main_avail, cross_avail) = if vertical {
            (area.h, area.w)
        } else {
            (area.w, area.h)
        };
        let mut kids = Vec::with_capacity(count);
        let mut mains = Vec::with_capacity(count);
        let mut crosses = Vec::with_capacity(count);
        let mut weights = Vec::with_capacity(count);
        for k in 0..count {
            let child = self.nodes[index].children[k];
            if !self.visible(child) {
                continue;
            }
            let node = &self.nodes[child];
            let (main_spec, cross_spec) = if vertical {
                (node.height, node.width)
            } else {
                (node.width, node.height)
            };
            let (fill, fit) = (node.fill, node.fit);
            let natural = self.measure(child, area.w, area.h);
            let cross = match cross_spec {
                Some(Size::Fill(_)) | None => cross_avail,
                Some(_) => (if vertical { natural.0 } else { natural.1 }).min(cross_avail),
            };
            // Measure again along the line now the other side is known:
            // wrapped text grows taller as it gets narrower.
            let along = |screen: &mut Screen| {
                let (w, h) = if vertical {
                    screen.measure(child, cross, main_avail)
                } else {
                    screen.measure(child, main_avail, cross)
                };
                if vertical { h } else { w }
            };
            let (main, weight) = match main_spec {
                Some(Size::Fill(weight)) => (0, weight),
                Some(_) => (along(self), 0),
                None => match fill {
                    Some(weight) => (0, weight),
                    None if fit || !self.wants_fill(child, vertical) => (along(self), 0),
                    None => (0, 1),
                },
            };
            kids.push(child);
            mains.push(main);
            crosses.push(cross);
            weights.push(weight);
        }
        share(&mut mains, &weights, main_avail);
        let end = if vertical {
            area.bottom()
        } else {
            area.right()
        };
        let mut at = if vertical { area.y } else { area.x };
        for ((child, main), cross) in kids.into_iter().zip(mains).zip(crosses) {
            let main = main.min(end - at).max(0);
            let rect = if vertical {
                Rect::new(area.x, at, cross, main)
            } else {
                Rect::new(at, area.y, main, cross)
            };
            self.place(child, rect, clip);
            at += main;
        }
    }

    /// Fits a widget's own state to the rectangle it was given.
    fn settle_leaf(&mut self, index: usize) {
        let inner = self.nodes[index].inner;
        match &mut self.nodes[index].kind {
            Kind::Text {
                value, wrap, lines, ..
            } => {
                let width = if *wrap {
                    inner.w.max(1) as usize
                } else {
                    usize::MAX
                };
                value.text.wrap(width, lines);
            }
            Kind::Listing(listing) => {
                let header = i32::from(listing.flavour == Flavour::Table);
                let visible = (inner.h - header).max(0) as usize;
                if listing.flavour == Flavour::Table {
                    let scrollbar = i32::from(listing.shown.len() > visible);
                    let gaps = (listing.columns.len() as i32 - 1).max(0);
                    let avail = (inner.w - gaps - scrollbar).max(0);
                    let mut widths = Vec::with_capacity(listing.columns.len());
                    let mut weights = Vec::with_capacity(listing.columns.len());
                    for (c, column) in listing.columns.iter().enumerate() {
                        match column.width {
                            Some(Size::Fill(weight)) => {
                                widths.push(0);
                                weights.push(weight);
                            }
                            Some(size @ (Size::Cells(_) | Size::Percent(_))) => {
                                widths.push(fixed(Some(size), avail).unwrap_or(0));
                                weights.push(0);
                            }
                            Some(Size::Fit) | None => {
                                widths.push(natural_column(listing, c));
                                weights.push(0);
                            }
                        }
                    }
                    share(&mut widths, &weights, avail);
                    listing.col_widths = widths;
                }
                keep_selection_in_view(listing, visible);
            }
            _ => {}
        }
    }
}

/// The width a table column needs for its title, its sort mark and its
/// widest cell.
fn natural_column(listing: &super::screen::Listing, column: usize) -> i32 {
    let title = listing.columns[column].title.text.len() as i32 + 2;
    let widest = listing
        .rows
        .iter()
        .filter_map(|row| row.cells.get(column))
        .map(|cell| cell.text.len() as i32)
        .max()
        .unwrap_or(0);
    title.max(widest)
}

/// Moves a listing's first displayed row so that the selection shows.
pub(super) fn keep_selection_in_view(listing: &mut super::screen::Listing, visible: usize) {
    if visible == 0 {
        return;
    }
    if listing.sel < listing.top {
        listing.top = listing.sel;
    } else if listing.sel >= listing.top + visible {
        listing.top = listing.sel + 1 - visible;
    }
    listing.top = listing.top.min(listing.shown.len().saturating_sub(visible));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leftover_space_is_shared_by_weight() {
        let mut sizes = [5, 0, 0];
        share(&mut sizes, &[0, 1, 2], 20);
        assert_eq!(sizes, [5, 5, 10]);
        let mut sizes = [0, 0, 0];
        share(&mut sizes, &[1, 1, 1], 10);
        assert_eq!(sizes, [4, 3, 3], "cells lost to rounding go to the first");
        let mut sizes = [30, 0];
        share(&mut sizes, &[0, 1], 20);
        assert_eq!(sizes, [30, 0], "nothing is left to share");
        let mut sizes = [3, 4];
        share(&mut sizes, &[0, 0], 20);
        assert_eq!(sizes, [3, 4]);
    }

    #[test]
    fn numbers_are_written_whole_when_they_are() {
        assert_eq!(number_text(37.0), "37");
        assert_eq!(number_text(-2.0), "-2");
        assert_eq!(number_text(0.25), "0.2");
        assert_eq!(number_text(12.75), "12.8");
    }
}
