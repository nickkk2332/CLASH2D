//! Content tables (06 §9.3): what a drawer, a shelf, a rack or a street
//! is found to hold, written once and shared by every map that names it.
//!
//! ```json
//! { "type": "content_table", "id": "kitchen_goods",
//!   "each": true,
//!   "entries": [ { "item": "loaf", "count": [1, 2], "chance": 0.8 },
//!                { "item": "knife", "chance": 0.5 },
//!                { "table": "drinks", "chance": 0.6 } ] }
//! ```
//!
//! A table is drawn from with a seed and a number of draws. Each draw
//! picks one entry by `weight`, or with `each` takes every entry; an
//! entry that is there by its `chance` gives `count` of its item, or
//! `count` draws from its nested table, or nothing (`none`). Every
//! random number a draw could use is drawn whether or not it matters,
//! from a generator seeded by the seed alone, so the same seed gives the
//! same things whenever and in whatever order tables are drawn from.

use clash_core::{Real, Rng, define_id};
use clash_data::{CrossCx, Def, DefLabel, Defs, DefsView, Diagnostic, Ref, ValidateCx, code};
use serde::Deserialize;

use super::{CountRange, ItemDef, ItemId};

define_id! {
    /// A `content_table` definition (06 §9.3).
    pub struct ContentTableId;
}

/// How deep tables may nest in one another when drawn from. A table
/// that nests itself is a load error; this bounds a chain of distinct
/// ones.
pub const MAX_TABLE_DEPTH: u32 = 8;

/// What a table's seed is mixed with before it is drawn from, so that
/// a table and an item made from the same seed draw apart.
const TABLE_SALT: u64 = 0x7461_626c_6573_0001;

fn one() -> Real {
    Real::ONE
}

/// A content table (06 §9.3): weighted entries of items, of other
/// tables and of nothing, drawn from when generation furnishes a cell
/// or a container.
#[derive(Debug, Deserialize)]
pub struct ContentTableDef {
    /// Whether every entry is rolled on each draw, each with its own
    /// `chance`, rather than one entry picked by `weight`. Default
    /// false: one entry is picked.
    #[serde(default)]
    pub each: bool,
    /// The entries, in order. At least one.
    pub entries: Vec<TableEntryDef>,
}

/// One entry of a content table: an item, another table, or nothing.
#[derive(Debug, Deserialize)]
pub struct TableEntryDef {
    /// The item it gives.
    #[serde(default)]
    pub item: Option<Ref<ItemDef>>,
    /// The table it draws from, `count` times.
    #[serde(default)]
    pub table: Option<Ref<ContentTableDef>>,
    /// That it gives nothing: an empty shelf, a pocket with nothing in
    /// it. Default false.
    #[serde(default)]
    pub none: bool,
    /// How likely it is to be picked, against the other entries'
    /// weights: a number above zero. Not read with `each`. Default 1.
    #[serde(default = "one")]
    pub weight: Real,
    /// How many of its item, or how many draws from its table: a number
    /// or `[least, most]`. Default 1.
    #[serde(default)]
    pub count: CountRange,
    /// The chance that, picked, it gives anything, from 0 to 1.
    /// Default 1.
    #[serde(default = "one")]
    pub chance: Real,
}

impl Def for ContentTableDef {
    type Id = ContentTableId;
    const TYPE: &'static str = "content_table";
    const DOC: &'static str = "A content table (06 §9.3): what a cell, a shelf or a container is found to hold, as weighted entries of items, of other tables and of nothing, shared by every map and table that names it.";

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.entries.is_empty() {
            cx.error(
                "a content table needs at least one entry: an empty one is an entry with `none`",
            );
        }
        for (index, entry) in self.entries.iter().enumerate() {
            let named = usize::from(entry.item.is_some())
                + usize::from(entry.table.is_some())
                + usize::from(entry.none);
            if named != 1 {
                cx.error(format!(
                    "entries {index}: an entry names exactly one of `item`, `table` and `none`"
                ));
            }
            if !(entry.weight.is_finite() && entry.weight > Real::ZERO) {
                cx.error(format!("entries {index}: `weight` must be above zero"));
            }
            if !(Real::ZERO..=Real::ONE).contains(&entry.chance) {
                cx.error(format!("entries {index}: `chance` is from 0 to 1"));
            }
            if entry.count.min > entry.count.max {
                cx.error(format!(
                    "entries {index}: `count` is `[least, most]`, and {} is more than {}",
                    entry.count.min, entry.count.max
                ));
            }
        }
    }
}

/// Reports every content table that nests itself, by way of any number
/// of others, at the entry that closes the loop. A cross check of the
/// loader.
pub fn check_table_loops(cx: &mut CrossCx<'_>) {
    let defs = cx.defs();
    let mut found: Vec<(String, usize)> = Vec::new();
    for (id, def) in defs.iter::<ContentTableDef>() {
        for (index, entry) in def.entries.iter().enumerate() {
            let Some(nested) = entry.table.as_ref().map(|table| table.id()) else {
                continue;
            };
            if reaches(defs, nested, id, 0) {
                let name = defs.name::<ContentTableDef>(id).unwrap_or_default();
                found.push((name.to_owned(), index));
            }
        }
    }
    for (name, index) in found {
        let diagnostic = Diagnostic::new(
            code::VALIDATION,
            "this table draws from a table that draws from it again: tables may not nest themselves",
        )
        .in_def(Some(&DefLabel::new(ContentTableDef::TYPE, &name)))
        .with_path(format!("entries[{index}].table"));
        cx.report(diagnostic);
    }
}

/// Whether drawing from `from` can reach `to`, within the depth limit.
fn reaches(defs: &DefsView<'_>, from: ContentTableId, to: ContentTableId, depth: u32) -> bool {
    if from == to {
        return true;
    }
    if depth > MAX_TABLE_DEPTH {
        return false;
    }
    let Some(def) = defs.get::<ContentTableDef>(from) else {
        return false;
    };
    def.entries
        .iter()
        .filter_map(|entry| entry.table.as_ref().map(|table| table.id()))
        .any(|next| reaches(defs, next, to, depth + 1))
}

/// Items a table gave: so many of a definition, with the seed they are
/// made from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Drawn {
    /// What.
    pub item: ItemId,
    /// How many.
    pub count: u32,
    /// What they are made from: their seeds, and so their colours and
    /// latent contents.
    pub seed: u64,
}

/// Draws `draws` times from a table with a seed (06 §9.3), and returns
/// what it gave, in the order it gave it. The same table, draws and
/// seed always give the same list. A table that is not defined gives
/// nothing; one nested deeper than [`MAX_TABLE_DEPTH`] gives nothing.
pub fn draw(defs: &Defs, table: ContentTableId, draws: u32, seed: u64) -> Vec<Drawn> {
    let mut out = Vec::new();
    let mut rng = Rng::from_seed(seed ^ TABLE_SALT);
    for _ in 0..draws {
        draw_once(defs, table, &mut rng, 0, &mut out);
    }
    out
}

/// One draw from a table.
fn draw_once(defs: &Defs, table: ContentTableId, rng: &mut Rng, depth: u32, out: &mut Vec<Drawn>) {
    if depth > MAX_TABLE_DEPTH {
        return;
    }
    let Some(def) = defs
        .try_registry::<ContentTableDef>()
        .and_then(|r| r.try_get(table))
    else {
        return;
    };
    if def.each {
        for entry in &def.entries {
            roll(defs, entry, rng, depth, out);
        }
        return;
    }
    let total = def
        .entries
        .iter()
        .fold(Real::ZERO, |sum, entry| sum + entry.weight);
    let mut at = rng.unit() * total;
    let mut chosen = def.entries.last();
    for entry in &def.entries {
        if at < entry.weight {
            chosen = Some(entry);
            break;
        }
        at -= entry.weight;
    }
    if let Some(entry) = chosen {
        roll(defs, entry, rng, depth, out);
    }
}

/// What an entry gives: its chance, its count and its seed are drawn
/// whatever they come to.
fn roll(defs: &Defs, entry: &TableEntryDef, rng: &mut Rng, depth: u32, out: &mut Vec<Drawn>) {
    let there = rng.chance(entry.chance);
    let count = rng.range(i64::from(entry.count.min), i64::from(entry.count.max));
    let seed = rng.next_u64();
    let count = u32::try_from(count).unwrap_or(0);
    if !there || count == 0 {
        return;
    }
    if let Some(item) = &entry.item {
        out.push(Drawn {
            item: item.id(),
            count,
            seed,
        });
    } else if let Some(nested) = &entry.table {
        let mut inner = Rng::from_seed(seed ^ TABLE_SALT);
        for _ in 0..count {
            draw_once(defs, nested.id(), &mut inner, depth + 1, out);
        }
    }
}
