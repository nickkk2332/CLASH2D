//! Re-evaluating a screen's bindings (12 §10.4), and what a screen keeps
//! when it is made again from its definition.
//!
//! Only what is on show is evaluated: a node whose `if` does not hold is
//! passed over with everything inside it, and so is every page of a
//! `tabs` but the one shown.

use clash_core::{Real, Value, VarMap};
use clash_expr::Value as ExprValue;

use crate::frame::Rect;
use crate::text::RichText;
use crate::theme::Theme;

use super::bind::Binding;
use super::def::{PopupDef, UiEffects};
use super::screen::{
    BindScope, Evaluator, Kind, Listing, LiveFlag, LiveNum, LiveText, MAX_BOUND_DEPTH,
    MAX_BOUND_ROWS, Node, Row, RowSource, Screen,
};

/// The key of a map that says how a row or an option is shown.
const KEY_LABEL: &str = "label";
/// The key of a map that holds an option's value.
const KEY_VALUE: &str = "value";

/// A value of the expression language as a screen variable can hold it:
/// a flag, a number or a text. Anything else has no such form.
pub fn to_var(value: &ExprValue) -> Option<Value> {
    match value {
        ExprValue::Bool(flag) => Some(Value::Bool(*flag)),
        ExprValue::Int(number) => Some(Value::Int(*number)),
        ExprValue::Real(number) => Some(Value::Real(*number)),
        ExprValue::Str(text) => Some(Value::Str(text.to_string())),
        _ => None,
    }
}

/// A screen variable's value as the expression language holds it.
pub fn from_var(value: &Value) -> ExprValue {
    match value {
        Value::Bool(flag) => ExprValue::Bool(*flag),
        Value::Int(number) => ExprValue::Int(*number),
        Value::Real(number) => ExprValue::Real(*number),
        Value::Str(text) => ExprValue::text(text),
        _ => ExprValue::Null,
    }
}

/// How a value is shown where nothing says how: a map by its `label`,
/// anything else as text.
fn label_of(value: &ExprValue) -> String {
    match value.as_map().and_then(|map| map.get(KEY_LABEL)) {
        Some(label) => label.to_string(),
        None => value.to_string(),
    }
}

struct Refresher<'a> {
    eval: &'a mut dyn Evaluator,
    theme: &'a Theme,
    screen: &'a str,
    vars: &'a VarMap,
    params: &'a [ExprValue],
}

impl Refresher<'_> {
    fn value(&mut self, binding: &Binding, it: Option<&ExprValue>) -> Option<ExprValue> {
        let scope = BindScope {
            screen: self.screen,
            vars: self.vars,
            params: self.params,
            it,
        };
        self.eval.value(binding, &scope)
    }

    /// The text a template gives now, styled. A result whose style tags
    /// are broken is shown as it is.
    fn render(&mut self, binding: &Binding, it: Option<&ExprValue>) -> Option<RichText> {
        let scope = BindScope {
            screen: self.screen,
            vars: self.vars,
            params: self.params,
            it,
        };
        let result = self.eval.text(binding, &scope)?;
        Some(RichText::parse(&result, self.theme).unwrap_or_else(|_| RichText::plain_text(&result)))
    }

    fn text(&mut self, live: &mut LiveText) {
        if let Some(binding) = &live.bound
            && let Some(text) = self.render(binding, None)
        {
            live.text = text;
        }
    }

    fn opt_text(&mut self, live: &mut Option<LiveText>) {
        if let Some(live) = live {
            self.text(live);
        }
    }

    fn number(&mut self, live: &mut LiveNum) {
        if let Some(binding) = &live.bound
            && let Some(result) = self.value(binding, None)
        {
            // Null, as of a character who is not there, shows as nothing.
            live.value = result.as_number().map_or(0.0, Real::get);
        }
    }

    fn flag(&mut self, live: &mut LiveFlag) {
        if let Some(binding) = &live.bound
            && let Some(result) = self.value(binding, None)
        {
            live.value = result.as_bool().unwrap_or(false);
        }
    }

    /// What a text over a row shows for one row.
    fn row_text(&mut self, template: &LiveText, it: &ExprValue) -> RichText {
        match &template.bound {
            Some(binding) => self
                .render(binding, Some(it))
                .unwrap_or_else(|| template.text.clone()),
            None => template.text.clone(),
        }
    }

    fn add_rows(
        &mut self,
        source: &RowSource,
        items: &[ExprValue],
        parent: Option<usize>,
        depth: u16,
        rows: &mut Vec<Row>,
        values: &mut Vec<ExprValue>,
    ) {
        for item in items {
            if rows.len() >= MAX_BOUND_ROWS {
                return;
            }
            let label = match &source.item {
                Some(template) => self.row_text(template, item),
                None => RichText::plain_text(&label_of(item)),
            };
            let cells = source
                .cells
                .iter()
                .map(|cell| LiveText {
                    bound: None,
                    text: match cell {
                        Some(template) => self.row_text(template, item),
                        None => RichText::plain_text(""),
                    },
                })
                .collect();
            let beneath: Vec<ExprValue> = match &source.children {
                Some(binding) if depth < MAX_BOUND_DEPTH => self
                    .value(binding, Some(item))
                    .and_then(|value| value.as_list().map(<[ExprValue]>::to_vec))
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            let at = rows.len();
            rows.push(Row {
                label: LiveText {
                    bound: None,
                    text: label,
                },
                value: None,
                key: None,
                key_text: String::new(),
                cells,
                depth,
                parent,
                has_children: !beneath.is_empty(),
                open: false,
                tip: None,
                on_activate: UiEffects::default(),
            });
            values.push(item.clone());
            self.add_rows(source, &beneath, Some(at), depth + 1, rows, values);
        }
    }

    fn listing(&mut self, listing: &mut Listing) {
        self.opt_text(&mut listing.empty);
        for column in &mut listing.columns {
            self.text(&mut column.title);
        }
        let Some(source) = listing.dynamic.take() else {
            for row in &mut listing.rows {
                self.text(&mut row.label);
                self.opt_text(&mut row.value);
                for cell in &mut row.cells {
                    self.text(cell);
                }
                self.opt_text(&mut row.tip);
            }
            return;
        };
        if let Some(value) = self.value(&source.rows, None) {
            let items = value.as_list().unwrap_or(&[]);
            let open: Vec<bool> = listing.rows.iter().map(|row| row.open).collect();
            let mut rows = Vec::with_capacity(items.len());
            let mut values = Vec::with_capacity(items.len());
            self.add_rows(&source, items, None, 0, &mut rows, &mut values);
            // The same rows as before keep which branches are open.
            if rows.len() == open.len() {
                for (row, open) in rows.iter_mut().zip(open) {
                    row.open = open && row.has_children;
                }
            }
            listing.rows = rows;
            listing.values = values;
            listing.reshow();
        }
        listing.dynamic = Some(source);
    }

    fn node(&mut self, nodes: &mut [Node], index: usize) {
        if let Some(flag) = &mut nodes[index].show_if {
            self.flag(flag);
            if !flag.value {
                return;
            }
        }
        let node = &mut nodes[index];
        self.opt_text(&mut node.tip);
        if let Some(Some(title)) = &mut node.frame {
            self.text(title);
        }
        let mut only = None;
        match &mut node.kind {
            Kind::Tabs { titles, active, .. } => {
                for title in titles.iter_mut() {
                    self.text(title);
                }
                only = Some(*active);
            }
            Kind::Text { value, .. } => self.text(value),
            Kind::Listing(listing) => self.listing(listing),
            Kind::Bar {
                value, max, label, ..
            } => {
                self.number(value);
                self.number(max);
                self.opt_text(label);
            }
            Kind::Input { label, .. } | Kind::Toggle { label, .. } => self.opt_text(label),
            Kind::Number {
                label,
                min,
                max,
                step,
                ..
            } => {
                self.opt_text(label);
                if let Some(min) = min {
                    self.number(min);
                }
                if let Some(max) = max {
                    self.number(max);
                }
                self.number(step);
            }
            Kind::Choice {
                label,
                options,
                from,
                ..
            } => {
                self.opt_text(label);
                match from {
                    Some(binding) => {
                        if let Some(value) = self.value(binding, None) {
                            *options = value
                                .as_list()
                                .unwrap_or(&[])
                                .iter()
                                .filter_map(|item| {
                                    let held = item
                                        .as_map()
                                        .and_then(|map| map.get(KEY_VALUE))
                                        .unwrap_or(item);
                                    Some((to_var(held)?, LiveText::plain(&label_of(item))))
                                })
                                .collect();
                        }
                    }
                    None => {
                        for (_, shown) in options {
                            self.text(shown);
                        }
                    }
                }
            }
            Kind::Button { label, .. } => self.text(label),
            Kind::Log {
                filter,
                filter_from,
                ..
            } => {
                if let Some(binding) = filter_from
                    && let Some(value) = self.value(binding, None)
                {
                    *filter = value.as_list().map(|names| {
                        names
                            .iter()
                            .filter_map(|name| name.as_str().map(str::to_owned))
                            .collect()
                    });
                }
            }
            Kind::Figure {
                region_names,
                shades,
                shade_from: Some(binding),
                ..
            } => {
                for (region, shade) in region_names.iter().zip(shades.iter_mut()) {
                    let it = ExprValue::text(region);
                    if let Some(value) = self.value(binding, Some(&it)) {
                        *shade = value.as_str().and_then(|name| self.theme.style(name));
                    }
                }
            }
            Kind::Tooltip { value } => self.opt_text(value),
            _ => {}
        }
        let children = nodes[index].children.clone();
        for (place, child) in children.into_iter().enumerate() {
            if only.is_none_or(|active| active == place) {
                self.node(nodes, child);
            }
        }
    }
}

impl LiveText {
    fn plain(text: &str) -> LiveText {
        LiveText {
            bound: None,
            text: RichText::plain_text(text),
        }
    }
}

impl Screen {
    /// Re-evaluates the bindings of everything on show (12 §10.4). The
    /// scene calls this when the simulation has advanced or a variable
    /// has changed. A text whose result has broken style tags is shown
    /// as written.
    pub fn refresh(&mut self, evaluator: &mut dyn Evaluator, theme: &Theme) {
        let Screen {
            id,
            title,
            vars,
            params,
            nodes,
            ..
        } = self;
        let mut refresher = Refresher {
            eval: evaluator,
            theme,
            screen: id,
            vars,
            params,
        };
        refresher.opt_text(title);
        if !nodes.is_empty() {
            refresher.node(nodes, 0);
        }
        self.pull_vars();
        self.stale = true;
    }

    /// The names of the screen's parameters, in the order their values
    /// are kept in.
    pub fn param_names(&self) -> &[String] {
        &self.param_names
    }

    /// The values the screen was opened with, beside
    /// [`param_names`](Screen::param_names).
    pub fn params(&self) -> &[ExprValue] {
        &self.params
    }

    /// Gives a parameter its value, as opening the screen does. Returns
    /// false, changing nothing, if the screen has no such parameter.
    pub fn set_param(&mut self, name: &str, value: ExprValue) -> bool {
        match self.param_names.iter().position(|known| known == name) {
            Some(slot) => {
                self.params[slot] = value;
                self.stale = true;
                true
            }
            None => false,
        }
    }

    /// How the screen is placed, if it does not fill the frame.
    pub fn popup(&self) -> Option<&PopupDef> {
        self.popup.as_ref()
    }

    /// The part of the frame the last layout gave the screen: all of it,
    /// unless the screen is a popup.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// What `it` is for a row of the listing with this id: its value
    /// where the rows come from an expression, its label where they are
    /// written out.
    pub fn row_value(&self, id: &str, row: usize) -> Option<ExprValue> {
        match &self.nodes[self.node_by_id(id)?].kind {
            Kind::Listing(listing) if row < listing.rows.len() => Some(listing.it_of(row)),
            _ => None,
        }
    }

    /// Takes over the state of a screen made from the same definition:
    /// its variables, its parameters, the focus, and each widget's
    /// selection, scroll position and open branches. This is how a
    /// screen is restyled: made again with another theme, then given
    /// what the old one held.
    pub fn adopt(&mut self, old: &Screen) {
        for (name, value) in old.vars.iter() {
            if self.vars.contains(name.as_str()) {
                self.vars.set(name.as_str(), value.clone());
            }
        }
        for (id, value) in old.prefs.iter() {
            self.prefs.set(id.as_str(), value.clone());
        }
        for (name, value) in old.param_names.iter().zip(&old.params) {
            self.set_param(name, value.clone());
        }
        if self.nodes.len() == old.nodes.len() {
            for (new, old) in self.nodes.iter_mut().zip(&old.nodes) {
                if let Some((flag, was)) = new.show_if.as_mut().zip(old.show_if.as_ref()) {
                    flag.value = was.value;
                }
                match (&mut new.kind, &old.kind) {
                    (Kind::Listing(new), Kind::Listing(old)) if new.flavour == old.flavour => {
                        if new.rows.len() == old.rows.len() {
                            for (row, was) in new.rows.iter_mut().zip(&old.rows) {
                                row.open = was.open;
                            }
                        }
                        new.sort = old.sort;
                        new.reshow();
                        new.sel = old.sel;
                        new.top = old.top;
                    }
                    (Kind::Tabs { active, .. }, Kind::Tabs { active: was, .. }) => *active = *was,
                    (Kind::Scroll { x, y, .. }, Kind::Scroll { x: ox, y: oy, .. }) => {
                        (*x, *y) = (*ox, *oy);
                    }
                    (Kind::Input { cursor, .. }, Kind::Input { cursor: was, .. }) => {
                        *cursor = *was;
                    }
                    (Kind::Log { back, .. }, Kind::Log { back: was, .. }) => *back = *was,
                    _ => {}
                }
            }
            self.focus = old.focus;
        }
        self.stale = true;
    }
}
