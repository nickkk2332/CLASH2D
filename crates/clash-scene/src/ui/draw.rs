//! Drawing a screen into a frame.
//!
//! Drawing reads the rectangles the last layout gave each node and
//! allocates nothing: texts were resolved when the screen was made, and
//! numbers are written into a buffer on the stack.

use core::fmt::{self, Write as _};

use clash_core::Value;

use crate::frame::{Attrs, Canvas, CellStyle, Frame, Rect};
use crate::text::RichText;

use super::def::Align;
use super::layout::{TREE_INDENT, write_number};
use super::screen::{
    Flavour, Kind, Listing, LiveText, LogLine, LogRows, MessageLog, Screen, UiData,
};
use crate::playback::log::Importance;

/// A short text built on the stack.
struct Small {
    bytes: [u8; 96],
    len: usize,
}

impl Small {
    fn new() -> Small {
        Small {
            bytes: [0; 96],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl fmt::Write for Small {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        if end > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Whether a log line passes a widget's filter: its category is one of
/// those shown and it matters at least as much as the widget asks.
fn passes(filter: &Option<Vec<String>>, least: Importance, line: &LogLine) -> bool {
    line.importance >= least
        && filter
            .as_ref()
            .is_none_or(|categories| categories.contains(&line.category))
}

/// The count written after a log line that was said more than once:
/// ` (x3)`.
fn count_suffix(count: u32) -> Small {
    let mut suffix = Small::new();
    let _ = write!(suffix, " (x{count})");
    suffix
}

/// How a log line falls into rows at a width: how many, and whether the
/// count of a folded line has a row of its own because it does not fit
/// after the text.
fn line_rows(line: &LogLine, width: usize) -> (usize, bool) {
    let (mut rows, mut last) = (0, 0);
    line.text.wrap_each(width, |(from, to)| {
        rows += 1;
        last = to - from;
    });
    let apart = line.count > 1 && last + count_suffix(line.count).as_str().len() > width;
    (rows + usize::from(apart), apart)
}

/// How many rows the lines of a log that pass a filter take at a width.
fn log_rows(
    log: &MessageLog,
    filter: &Option<Vec<String>>,
    least: Importance,
    width: usize,
) -> usize {
    log.lines()
        .iter()
        .filter(|line| passes(filter, least, line))
        .map(|line| line_rows(line, width).0)
        .sum()
}

/// Where a text of `len` cells starts in a space of `width` cells.
fn aligned(align: Align, width: i32, len: i32) -> i32 {
    match align {
        Align::Left => 0,
        Align::Centre => (width - len).max(0) / 2,
        Align::Right => (width - len).max(0),
    }
}

impl Screen {
    /// Draws the screen into the frame, laying it out first if the
    /// frame's size or the screen has changed since the last layout. A
    /// screen fills the frame, unless it is a popup: that is drawn over
    /// its own rectangle and leaves the rest of the frame as it was.
    ///
    /// On a grid smaller than the screen's `min_size` a notice is drawn
    /// instead, saying how large the window must be.
    pub fn draw(&mut self, frame: &mut Frame, data: &UiData<'_>) {
        let size = (frame.cols, frame.rows);
        if self.size != size {
            self.layout(size.0, size.1);
            self.reveal_focus();
        } else if self.stale {
            self.layout(size.0, size.1);
        }
        self.clamp_logs(data);

        let base = self.chrome.base;
        let mut canvas = Canvas::new(frame);
        let area = if self.too_small() {
            canvas.clip()
        } else {
            self.area
        };
        canvas.fill(area, ' ', base);
        if self.too_small() {
            self.draw_notice(&mut canvas);
        } else if !self.nodes.is_empty() {
            self.draw_node(0, &mut canvas, base, data);
        }
    }

    /// Works out how each log's lines fall into rows, and keeps its
    /// scroll position within what the log holds. A log scrolled back
    /// stays on the lines it shows as newer ones arrive; one that is
    /// not follows the newest.
    fn clamp_logs(&mut self, data: &UiData<'_>) {
        for node in &mut self.nodes {
            let size = (node.inner.w, node.inner.h);
            if let Kind::Log {
                filter,
                least,
                back,
                rows_seen,
                rows_cache,
                ..
            } = &mut node.kind
            {
                let visible = size.1.max(0) as usize;
                let rows = match (data.log, *rows_cache) {
                    (None, _) => LogRows {
                        revision: 0,
                        size,
                        width: size.0.max(1) as usize,
                        total: 0,
                    },
                    (Some(log), Some(rows))
                        if rows.revision == log.revision() && rows.size == size =>
                    {
                        rows
                    }
                    (Some(log), _) => {
                        // One column goes to the scrollbar when there
                        // are more rows than fit.
                        let full = size.0.max(1) as usize;
                        let mut width = full;
                        let mut total = log_rows(log, filter, *least, width);
                        if total > visible && full > 1 {
                            width = full - 1;
                            total = log_rows(log, filter, *least, width);
                        }
                        LogRows {
                            revision: log.revision(),
                            size,
                            width,
                            total,
                        }
                    }
                };
                *rows_cache = data.log.map(|_| rows);
                if *back > 0 && rows.total > *rows_seen {
                    *back = back.saturating_add(rows.total - *rows_seen);
                }
                *rows_seen = rows.total;
                *back = (*back).min(rows.total.saturating_sub(visible));
            }
        }
    }

    fn draw_notice(&self, canvas: &mut Canvas<'_>) {
        let mut text = String::new();
        let _ = write!(
            text,
            "This screen needs a window of at least {} by {} characters. This one is {} by {}.",
            self.min_size.0, self.min_size.1, self.size.0, self.size.1
        );
        let notice = RichText::plain_text(&text);
        let mut lines = Vec::new();
        notice.wrap(usize::from(self.size.0).max(1), &mut lines);
        let style = self.chrome.notice.over(self.chrome.base);
        for (row, range) in lines.into_iter().enumerate() {
            notice.draw(canvas, 0, row as i32, range, style);
        }
    }

    fn draw_scrollbar(
        &self,
        canvas: &mut Canvas<'_>,
        column: Rect,
        total: usize,
        visible: usize,
        top: usize,
        base: CellStyle,
    ) {
        if total <= visible || column.h <= 0 {
            return;
        }
        let height = column.h as usize;
        let thumb = (height * visible / total).clamp(1, height);
        let travel = total - visible;
        let start = (height - thumb) * top.min(travel) / travel;
        let track = self.chrome.dim.over(base);
        for row in 0..height {
            let on_thumb = row >= start && row < start + thumb;
            let (ch, style) = if on_thumb {
                (self.chrome.scroll_thumb, base)
            } else {
                (self.chrome.scroll_track, track)
            };
            canvas.put(column.x, column.y + row as i32, ch, style);
        }
    }

    /// An area the toolkit has nothing to draw in yet: dots and a name.
    fn draw_placeholder(
        &self,
        canvas: &mut Canvas<'_>,
        area: Rect,
        name: &str,
        focused: bool,
        base: CellStyle,
    ) {
        canvas.fill(area, self.chrome.view_empty, self.chrome.dim.over(base));
        let style = if focused {
            self.chrome.selected.over(base)
        } else {
            base
        };
        let len = name.chars().count() as i32;
        if area.h > 0 && area.w >= len {
            canvas.print(
                area.x + (area.w - len) / 2,
                area.y + area.h / 2,
                name,
                style,
            );
        }
    }

    fn draw_label(
        &self,
        canvas: &mut Canvas<'_>,
        label: &Option<LiveText>,
        at: (i32, i32),
        max: i32,
        style: CellStyle,
    ) -> i32 {
        match label {
            Some(label) => label.text.draw_line(canvas, at.0, at.1, max, style) + 1,
            None => 0,
        }
    }

    fn draw_node(
        &self,
        index: usize,
        root: &mut Canvas<'_>,
        inherited: CellStyle,
        data: &UiData<'_>,
    ) {
        let node = &self.nodes[index];
        if !node.shown {
            return;
        }
        let chrome = &self.chrome;
        let focused = self.focus == Some(index);
        let is_bar = matches!(node.kind, Kind::Bar { .. });
        // A bar's style colours its filled part, not the whole widget.
        let base = match node.style {
            Some(style) if !is_bar => style.over(inherited),
            _ => inherited,
        };
        if !is_bar && node.style.is_some_and(|style| style.bg.is_some()) {
            root.within(node.bounds).fill(node.outer, ' ', base);
        }

        if let Some(title) = &node.frame {
            let focus_inside = self
                .focus
                .is_some_and(|f| f >= index && f < self.subtree_end(index));
            let line = if focus_inside {
                chrome.border_focus
            } else {
                chrome.border
            }
            .over(base);
            let mut canvas = root.within(node.bounds);
            canvas.border(node.outer, &chrome.borders, line);
            if let Some(title) = title
                && node.outer.w >= 6
            {
                let (x, y) = (node.outer.x, node.outer.y);
                canvas.put(x + 1, y, ' ', line);
                let len = title.text.draw_line(
                    &mut canvas,
                    x + 2,
                    y,
                    node.outer.w - 4,
                    chrome.title.over(base),
                );
                canvas.put(x + 2 + len, y, ' ', line);
            }
        }

        let inner = node.inner;
        // Containers hand the whole canvas on to their children, each of
        // which confines itself to its own rectangle.
        match &node.kind {
            Kind::Rows
            | Kind::Cols
            | Kind::Stack
            | Kind::Grid { .. }
            | Kind::Scroll { .. }
            | Kind::Slot { .. } => {
                for child in &node.children {
                    self.draw_node(*child, root, base, data);
                }
            }
            Kind::Tabs { active: page, .. } => {
                if let Some(child) = node.children.get(*page) {
                    self.draw_node(*child, root, base, data);
                }
            }
            _ => {}
        }
        let mut canvas = root.within(node.clip);
        // A scrollbar's thumb shows whether its widget has the focus.
        let thumb = if focused {
            chrome.border_focus.over(base)
        } else {
            base
        };
        let active = if focused {
            chrome.selected.over(base)
        } else {
            base
        };
        match &node.kind {
            Kind::Rows | Kind::Cols | Kind::Stack | Kind::Grid { .. } => {}
            Kind::Tabs {
                titles,
                active: page,
                ..
            } => {
                let mut x = inner.x;
                for (k, title) in titles.iter().enumerate() {
                    let style = if k != *page {
                        chrome.dim.over(base)
                    } else if focused {
                        chrome.selected.over(base)
                    } else {
                        chrome.header.over(base)
                    };
                    canvas.put(x, inner.y, ' ', style);
                    let len = title.text.draw_line(
                        &mut canvas,
                        x + 1,
                        inner.y,
                        inner.right() - x - 1,
                        style,
                    );
                    canvas.put(x + 1 + len, inner.y, ' ', style);
                    x += len + 3;
                }
            }
            Kind::Scroll { y, content_h, .. } => {
                let column = Rect::new(inner.right() - 1, inner.y, 1, inner.h);
                self.draw_scrollbar(
                    &mut canvas,
                    column,
                    (*content_h).max(0) as usize,
                    inner.h.max(0) as usize,
                    (*y).max(0) as usize,
                    thumb,
                );
            }
            Kind::Slot { .. } | Kind::Spacer => {}
            Kind::Text {
                value,
                align,
                lines,
                ..
            } => {
                for (row, (from, to)) in lines.iter().enumerate() {
                    let x = inner.x + aligned(*align, inner.w, (to - from) as i32);
                    value
                        .text
                        .draw(&mut canvas, x, inner.y + row as i32, (*from, *to), base);
                }
            }
            Kind::Art { rows } => {
                for (r, row) in rows.iter().enumerate() {
                    for (c, ch) in row.iter().enumerate() {
                        canvas.put(inner.x + c as i32, inner.y + r as i32, *ch, base);
                    }
                }
            }
            Kind::Figure {
                rows,
                region_of,
                shades,
                ..
            } => {
                for (r, row) in rows.iter().enumerate() {
                    for (c, ch) in row.iter().enumerate() {
                        let shade = region_of[r][c].and_then(|region| shades[usize::from(region)]);
                        let style = shade.map_or(base, |shade| shade.over(base));
                        canvas.put(inner.x + c as i32, inner.y + r as i32, *ch, style);
                    }
                }
            }
            Kind::Listing(listing) => self.draw_listing(listing, &mut canvas, inner, focused, base),
            Kind::Bar {
                value,
                max,
                label,
                gauge,
            } => {
                let x = inner.x
                    + self.draw_label(&mut canvas, label, (inner.x, inner.y), inner.w, base);
                if *gauge {
                    let mut text = Small::new();
                    let _ = write_number(&mut text, value.value);
                    let _ = text.write_char('/');
                    let _ = write_number(&mut text, max.value);
                    canvas.print(
                        x,
                        inner.y,
                        text.as_str(),
                        node.style.map_or(base, |s| s.over(base)),
                    );
                } else {
                    let width = inner.right() - x;
                    let share = if max.value > 0.0 {
                        (value.value / max.value).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let filled = (share * f64::from(width) + 0.5).floor() as i32;
                    let full = node.style.unwrap_or(chrome.bar).over(base);
                    let empty = chrome.bar_empty.over(base);
                    for k in 0..width {
                        let (ch, style) = if k < filled {
                            (chrome.bar_full_ch, full)
                        } else {
                            (chrome.bar_empty_ch, empty)
                        };
                        canvas.put(x + k, inner.y, ch, style);
                    }
                }
            }
            Kind::Input {
                var, label, cursor, ..
            } => {
                let x = inner.x
                    + self.draw_label(&mut canvas, label, (inner.x, inner.y), inner.w, base);
                let width = (inner.right() - x).max(0);
                let text = match self.read(var) {
                    Some(Value::Str(text)) => text.as_str(),
                    _ => "",
                };
                let cursor = (*cursor).min(text.chars().count());
                let style = chrome.field.over(base);
                canvas.fill(Rect::new(x, inner.y, width, 1), ' ', style);
                // Show the end of a text longer than the field, so the
                // cursor is always in view.
                let first = (cursor + 1).saturating_sub(width.max(1) as usize);
                for (k, ch) in text.chars().skip(first).take(width as usize).enumerate() {
                    canvas.put(x + k as i32, inner.y, ch, style);
                }
                if focused {
                    canvas.edit(x + (cursor - first) as i32, inner.y, |cell| {
                        cell.attrs |= Attrs::REVERSE
                    });
                }
            }
            Kind::Number {
                var, label, typing, ..
            } => {
                let x = inner.x
                    + self.draw_label(&mut canvas, label, (inner.x, inner.y), inner.w, base);
                let mut text = Small::new();
                let _ = write!(text, "{} ", chrome.choice_prev);
                match typing {
                    // What is being typed, and where the next digit goes.
                    Some(typed) => {
                        let _ = write!(text, "{typed}_");
                    }
                    None => {
                        let _ = write_number(&mut text, self.number_of(var));
                    }
                }
                let _ = write!(text, " {}", chrome.choice_next);
                canvas.print(x, inner.y, text.as_str(), active);
            }
            Kind::Toggle { var, label, .. } => {
                let on = matches!(self.read(var), Some(Value::Bool(true)));
                canvas.put(inner.x, inner.y, '[', active);
                canvas.put(
                    inner.x + 1,
                    inner.y,
                    if on {
                        chrome.toggle_on
                    } else {
                        chrome.toggle_off
                    },
                    active,
                );
                canvas.put(inner.x + 2, inner.y, ']', active);
                if let Some(label) = label {
                    canvas.put(inner.x + 3, inner.y, ' ', active);
                    label
                        .text
                        .draw_line(&mut canvas, inner.x + 4, inner.y, inner.w - 4, active);
                }
            }
            Kind::Choice {
                var,
                label,
                options,
                ..
            } => {
                let x = inner.x
                    + self.draw_label(&mut canvas, label, (inner.x, inner.y), inner.w, base);
                let held = self.read(var);
                let shown = options
                    .iter()
                    .find(|(value, _)| Some(value) == held)
                    .or(options.first())
                    .map(|(_, shown)| &shown.text);
                canvas.put(x, inner.y, chrome.choice_prev, active);
                canvas.put(x + 1, inner.y, ' ', active);
                let len = shown.map_or(0, |text| {
                    text.draw_line(&mut canvas, x + 2, inner.y, inner.right() - x - 4, active)
                });
                canvas.put(x + 2 + len, inner.y, ' ', active);
                canvas.put(x + 3 + len, inner.y, chrome.choice_next, active);
            }
            Kind::Button { label, .. } => {
                let style = if focused {
                    active
                } else {
                    chrome.button.over(base)
                };
                canvas.print(inner.x, inner.y, "[ ", style);
                let len =
                    label
                        .text
                        .draw_line(&mut canvas, inner.x + 2, inner.y, inner.w - 4, style);
                canvas.print(inner.x + 2 + len, inner.y, " ]", style);
            }
            Kind::Log {
                filter,
                least,
                back,
                rows_cache,
                ..
            } => {
                if let (Some(log), Some(rows)) = (data.log, rows_cache) {
                    let visible = inner.h.max(0) as usize;
                    let first = rows.total.saturating_sub(visible + back);
                    // The newest line that starts at or before the first
                    // row on show: found from the end, so that a long
                    // log costs what is on show and not its length.
                    let mut row = rows.total;
                    let mut start = log.lines().len();
                    for (index, line) in log.lines().iter().enumerate().rev() {
                        if row <= first {
                            break;
                        }
                        if passes(filter, *least, line) {
                            row -= line_rows(line, rows.width).0;
                            start = index;
                        }
                    }
                    let shown = first..first + visible;
                    let count_style = chrome.dim.over(base);
                    for line in &log.lines()[start..] {
                        if row >= shown.end {
                            break;
                        }
                        if !passes(filter, *least, line) {
                            continue;
                        }
                        let (_, apart) = line_rows(line, rows.width);
                        let mut end = (inner.x, row);
                        line.text.wrap_each(rows.width, |(from, to)| {
                            if shown.contains(&row) {
                                let y = inner.y + (row - first) as i32;
                                let len = line.text.draw(&mut canvas, inner.x, y, (from, to), base);
                                end = (inner.x + len, row);
                            }
                            row += 1;
                        });
                        if line.count > 1 {
                            let suffix = count_suffix(line.count);
                            if apart {
                                if shown.contains(&row) {
                                    let y = inner.y + (row - first) as i32;
                                    canvas.print(
                                        inner.x,
                                        y,
                                        suffix.as_str().trim_start(),
                                        count_style,
                                    );
                                }
                                row += 1;
                            } else if shown.contains(&end.1) && end.1 + 1 == row {
                                let y = inner.y + (end.1 - first) as i32;
                                canvas.print(end.0, y, suffix.as_str(), count_style);
                            }
                        }
                    }
                    let column = Rect::new(inner.right() - 1, inner.y, 1, inner.h);
                    self.draw_scrollbar(&mut canvas, column, rows.total, visible, first, thumb);
                }
            }
            Kind::View { kind, def } => match data.views {
                Some(painter) => painter.paint(*kind, def, focused, inner, &mut canvas),
                None => self.draw_placeholder(&mut canvas, inner, kind.name(), focused, base),
            },
            Kind::Debug { name } => self.draw_placeholder(&mut canvas, inner, name, focused, base),
            Kind::Tooltip { value } => {
                let text = self.tip().or(value.as_ref().map(|value| &value.text));
                if let Some(text) = text {
                    text.draw_line(
                        &mut canvas,
                        inner.x,
                        inner.y,
                        inner.w,
                        chrome.tip.over(base),
                    );
                }
            }
        }
    }

    fn draw_listing(
        &self,
        listing: &Listing,
        canvas: &mut Canvas<'_>,
        inner: Rect,
        focused: bool,
        base: CellStyle,
    ) {
        let chrome = &self.chrome;
        let table = listing.flavour == Flavour::Table;
        let header = i32::from(table);
        let visible = (inner.h - header).max(0) as usize;
        let overflow = listing.shown.len() > visible;
        let row_w = inner.w - i32::from(overflow);

        if table {
            let style = chrome.header.over(base);
            let mut x = inner.x;
            for (c, column) in listing.columns.iter().enumerate() {
                let width = listing.col_widths.get(c).copied().unwrap_or(0);
                let len = column
                    .title
                    .text
                    .draw_line(canvas, x, inner.y, width, style);
                if let Some((sorted, descending)) = listing.sort
                    && sorted == c
                    && len + 1 < width
                {
                    let mark = if descending {
                        chrome.sort_down
                    } else {
                        chrome.sort_up
                    };
                    canvas.put(x + len + 1, inner.y, mark, style);
                }
                x += width + 1;
            }
        }
        if listing.shown.is_empty() {
            if let Some(empty) = &listing.empty {
                empty.text.draw_line(
                    canvas,
                    inner.x,
                    inner.y + header,
                    inner.w,
                    chrome.dim.over(base),
                );
            }
            return;
        }

        let end = (listing.top + visible).min(listing.shown.len());
        for (line, at) in (listing.top..end).enumerate() {
            let row = &listing.rows[listing.shown[at]];
            let y = inner.y + header + line as i32;
            let style = if at != listing.sel {
                base
            } else if focused {
                chrome.selected.over(base)
            } else {
                chrome.selected_idle.over(base)
            };
            if at == listing.sel {
                canvas.fill(Rect::new(inner.x, y, row_w, 1), ' ', style);
            }
            let mut row_canvas = canvas.within(Rect::new(inner.x, y, row_w, 1));
            match listing.flavour {
                Flavour::List => {
                    row.label
                        .text
                        .draw_line(&mut row_canvas, inner.x, y, row_w, style);
                }
                Flavour::Menu => {
                    if row.key.is_some() {
                        row_canvas.print(inner.x, y, &row.key_text, chrome.hotkey.over(style));
                    }
                    row.label.text.draw_line(
                        &mut row_canvas,
                        inner.x + listing.key_width,
                        y,
                        row_w,
                        style,
                    );
                }
                Flavour::Tree | Flavour::Inspect => {
                    let mut x = inner.x + i32::from(row.depth) * TREE_INDENT;
                    if row.has_children {
                        let mark = if row.open {
                            chrome.tree_open
                        } else {
                            chrome.tree_closed
                        };
                        row_canvas.put(x, y, mark, style);
                    }
                    x += 2;
                    x += row
                        .label
                        .text
                        .draw_line(&mut row_canvas, x, y, row_w, style);
                    if let Some(value) = &row.value {
                        x += row_canvas.print(x, y, ": ", style);
                        value.text.draw_line(&mut row_canvas, x, y, row_w, style);
                    }
                }
                Flavour::Table => {
                    let mut x = inner.x;
                    for (c, width) in listing.col_widths.iter().enumerate() {
                        if let Some(cell) = row.cells.get(c) {
                            let len = (cell.text.len() as i32).min(*width);
                            let align = listing.columns[c].align;
                            cell.text.draw_line(
                                &mut row_canvas,
                                x + aligned(align, *width, len),
                                y,
                                len,
                                style,
                            );
                        }
                        x += width + 1;
                    }
                }
            }
        }
        if overflow {
            let column = Rect::new(inner.right() - 1, inner.y + header, 1, inner.h - header);
            let bar_style = if focused {
                chrome.border_focus.over(base)
            } else {
                base
            };
            self.draw_scrollbar(
                canvas,
                column,
                listing.shown.len(),
                visible,
                listing.top,
                bar_style,
            );
        }
    }
}
