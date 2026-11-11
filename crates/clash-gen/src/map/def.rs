//! The `map` and `palette` definition types as a mod writes them
//! (10 §3).
//!
//! These structs are what the loader reads. They keep where every value
//! was written, so that the checks of the build stage
//! ([`MapDefs::from_defs`](super::MapDefs::from_defs)) can point into the
//! file. Nothing here is resolved: states are text, characters are
//! strings, and a nested map is an id or a tag.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use clash_core::{Dir8, IVec3, Real, Rot4, Turn, define_id};
use clash_data::expr::{Any, ExprType, Int, Number};
use clash_data::{
    CompileCx, Cond, Def, Diagnostic, Expr, FromSite, Kind, KindValue, Located, Ref, Scope, Site,
    TagSet, by_site, code,
};
use clash_expr::{Expr as Tree, ExprKind};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

define_id! {
    /// A `map` definition.
    pub struct MapId;
    /// A `palette` definition.
    pub struct PaletteId;
}

/// The prefix a mark is named with where a point is wanted:
/// `"mark:entrance"`.
pub const MARK_PREFIX: &str = "mark:";

/// The prefix an area may be named with where one is wanted:
/// `"area:parlour"`.
pub const AREA_PREFIX: &str = "area:";

/// Implements [`FromSite`] and `Deserialize` for a type that is written
/// in more than one JSON shape.
macro_rules! shapes {
    ($ty:ty, $expected:literal, |$site:ident, $value:ident| { $($kind:pat => $body:expr),+ $(,)? }) => {
        impl<'de> FromSite<'de> for $ty {
            fn from_site<A: MapAccess<'de>>(
                $site: Site,
                kind: Kind,
                $value: KindValue<'_, A>,
            ) -> Result<$ty, A::Error> {
                #[allow(unreachable_patterns, reason = "a type may take every kind")]
                match kind {
                    $($kind => $body,)+
                    other => Err(other.unexpected($expected)),
                }
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$ty, D::Error> {
                by_site(deserializer)
            }
        }
    };
}

fn refuse<T, E: de::Error>(message: impl fmt::Display) -> Result<T, E> {
    Err(E::custom(message))
}

/// A value with where it was written and its JSON path.
#[derive(Clone, Debug)]
pub struct Sited<T> {
    /// The value.
    pub value: T,
    /// Where it was written.
    pub site: Site,
}

impl<'de, T: Deserialize<'de>> FromSite<'de> for Sited<T> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        _kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Sited<T>, A::Error> {
        Ok(Sited {
            value: value.read::<T>()?,
            site,
        })
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Sited<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Sited<T>, D::Error> {
        by_site(deserializer)
    }
}

/// A JSON object read as its entries in the order they were written:
/// a parameter may use those declared before it.
#[derive(Clone, Debug)]
pub struct Ordered<V>(pub Vec<(String, V)>);

impl<V> Default for Ordered<V> {
    fn default() -> Ordered<V> {
        Ordered(Vec::new())
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Ordered<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Ordered<V>, D::Error> {
        struct Entries<V>(PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for Entries<V> {
            type Value = Ordered<V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered<V>, A::Error> {
                let mut entries: Vec<(String, V)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, V>()? {
                    if entries.iter().any(|(seen, _)| *seen == key) {
                        return refuse(format!("the key `{key}` is given twice"));
                    }
                    entries.push((key, value));
                }
                Ok(Ordered(entries))
            }
        }

        deserializer.deserialize_map(Entries(PhantomData))
    }
}

/// A plain value written in data: `true`, `3`, `0.5` or a text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lit(pub clash_core::Value);

impl<'de> Deserialize<'de> for Lit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Lit, D::Error> {
        struct LitVisitor;

        impl Visitor<'_> for LitVisitor {
            type Value = Lit;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number, a boolean or a text")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Lit, E> {
                Ok(Lit(clash_core::Value::Bool(value)))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Lit, E> {
                Ok(Lit(clash_core::Value::Int(value)))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Lit, E> {
                match i64::try_from(value) {
                    Ok(int) => Ok(Lit(clash_core::Value::Int(int))),
                    Err(_) => refuse("this number is too large"),
                }
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Lit, E> {
                match Real::new(value) {
                    Ok(real) => Ok(Lit(clash_core::Value::Real(real))),
                    Err(_) => refuse("this is not a number"),
                }
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Lit, E> {
                Ok(Lit(clash_core::Value::Str(value.to_owned())))
            }
        }

        deserializer.deserialize_any(LitVisitor)
    }
}

/// One name or a list of them: `"bed"` or `["bed", "guest_bed"]`.
#[derive(Clone, Debug, Default)]
pub struct Names(pub Vec<String>);

shapes!(Names, "a name or a list of names", |_site, value| {
    Kind::String => Ok(Names(vec![value.read::<String>()?])),
    Kind::List => Ok(Names(value.read::<Vec<String>>()?)),
});

/// How a parameter gets its value when whoever placed the map gave none
/// (10 §3.4).
#[derive(Clone, Debug)]
pub enum ParamRule {
    /// `{ "pick": [...] }`: one of these, each equally likely.
    Pick(Vec<Lit>),
    /// `{ "range": [low, high] }`: a whole number from `low` to `high`,
    /// or a real from `low` up to `high` if either has a fraction.
    Range(Lit, Lit),
    /// A plain value, or `{ "default": value }`.
    Default(Lit),
    /// An expression over the parameters declared before this one.
    Expr(Box<Expr<Any>>),
    /// `{ "type": "int" }` alone: the placement must give it.
    Required,
}

/// One entry of a map's `params`.
#[derive(Clone, Debug)]
pub struct ParamDef {
    /// How it gets its value.
    pub rule: ParamRule,
    /// The type, if it was written: `bool`, `int`, `real` or `string`.
    pub ty: Option<Located<String>>,
    /// Where the parameter was written.
    pub site: Site,
}

#[derive(Deserialize)]
struct ParamObject {
    #[serde(default)]
    pick: Option<Vec<Lit>>,
    #[serde(default)]
    range: Option<Vec<Lit>>,
    #[serde(default)]
    default: Option<Lit>,
    #[serde(default)]
    expr: Option<Expr<Any>>,
    #[serde(default, rename = "type")]
    ty: Option<Located<String>>,
}

shapes!(ParamDef, "a value, an expression or a parameter rule", |site, value| {
    Kind::Bool | Kind::Number => Ok(ParamDef {
        rule: ParamRule::Default(value.read::<Lit>()?),
        ty: None,
        site,
    }),
    Kind::String => Ok(ParamDef {
        rule: ParamRule::Expr(Box::new(value.read::<Expr<Any>>()?)),
        ty: None,
        site,
    }),
    Kind::Object => {
        let object = value.read::<ParamObject>()?;
        let given = usize::from(object.pick.is_some())
            + usize::from(object.range.is_some())
            + usize::from(object.default.is_some())
            + usize::from(object.expr.is_some());
        if given > 1 {
            return refuse("a parameter has one of `pick`, `range`, `default` and `expr`");
        }
        let rule = if let Some(pick) = object.pick {
            if pick.is_empty() {
                return refuse("`pick` needs at least one value");
            }
            ParamRule::Pick(pick)
        } else if let Some(range) = object.range {
            match <[Lit; 2]>::try_from(range) {
                Ok([low, high]) => ParamRule::Range(low, high),
                Err(_) => return refuse("`range` is `[low, high]`"),
            }
        } else if let Some(default) = object.default {
            ParamRule::Default(default)
        } else if let Some(expr) = object.expr {
            ParamRule::Expr(Box::new(expr))
        } else if object.ty.is_some() {
            ParamRule::Required
        } else {
            return refuse(
                "a parameter needs `pick`, `range`, `default` or `expr`, or a `type` alone \
                 for one the placement must give",
            );
        };
        Ok(ParamDef { rule, ty: object.ty, site })
    },
});

/// How many: a number, `[low, high]`, or an expression.
#[derive(Clone, Debug)]
pub enum CountDef {
    /// `[low, high]`: a whole number from `low` to `high`.
    Range(i64, i64),
    /// A number or an expression.
    Expr(Box<Expr<Int>>),
}

shapes!(CountDef, "a count: a number, `[low, high]` or an expression", |_site, value| {
    Kind::List => {
        let range = value.read::<Vec<i64>>()?;
        match range.as_slice() {
            [low, high] if low <= high && *low >= 0 => Ok(CountDef::Range(*low, *high)),
            _ => refuse("a count range is `[low, high]`, neither below zero, `low` first"),
        }
    },
    Kind::Number | Kind::String | Kind::Object => {
        Ok(CountDef::Expr(Box::new(value.read::<Expr<Int>>()?)))
    },
});

/// Items laid in a cell: an item, or a content table (10 §3.2).
#[derive(Clone, Debug, Deserialize)]
pub struct ItemsDef {
    /// The id of one item definition.
    #[serde(default)]
    pub item: Option<String>,
    /// The id of a content table.
    #[serde(default)]
    pub table: Option<String>,
    /// How many. One if left out.
    #[serde(default)]
    pub count: Option<CountDef>,
    /// The chance that there is anything. Certain if left out.
    #[serde(default)]
    pub chance: Option<Expr<Number>>,
}

/// The `items` field of a cell spec: an item id, or an [`ItemsDef`].
#[derive(Clone, Debug)]
pub struct ItemsField(pub ItemsDef);

shapes!(ItemsField, "an item id or an object", |_site, value| {
    Kind::String => Ok(ItemsField(ItemsDef {
        item: Some(value.read::<String>()?),
        table: None,
        count: None,
        chance: None,
    })),
    Kind::Object => {
        let items = value.read::<ItemsDef>()?;
        if items.item.is_some() == items.table.is_some() {
            return refuse("`items` names one of `item` and `table`");
        }
        Ok(ItemsField(items))
    },
});

/// Who is spawned: a creature or a member of a population (06 §12).
#[derive(Clone, Debug, Deserialize)]
pub struct SpawnWhoDef {
    /// The id of a creature definition.
    #[serde(default)]
    pub creature: Option<String>,
    /// The id of a population definition.
    #[serde(default)]
    pub population: Option<String>,
    /// How many. One if left out.
    #[serde(default)]
    pub count: Option<CountDef>,
    /// The chance that anyone is there. Certain if left out.
    #[serde(default)]
    pub chance: Option<Expr<Number>>,
    /// For an entry of a map's `spawns`: the area they are spread
    /// through.
    #[serde(default, rename = "in")]
    pub within: Option<Located<String>>,
    /// For an entry of a map's `spawns`: the cell or mark they stand at.
    #[serde(default)]
    pub at: Option<PointDef>,
}

/// The `spawn` field of a cell spec: a creature id, or a
/// [`SpawnWhoDef`].
#[derive(Clone, Debug)]
pub struct SpawnField(pub SpawnWhoDef);

shapes!(SpawnField, "a creature id or an object", |_site, value| {
    Kind::String => Ok(SpawnField(SpawnWhoDef {
        creature: Some(value.read::<String>()?),
        population: None,
        count: None,
        chance: None,
        within: None,
        at: None,
    })),
    Kind::Object => {
        let spawn = value.read::<SpawnWhoDef>()?;
        if spawn.creature.is_some() == spawn.population.is_some() {
            return refuse("a spawn names one of `creature` and `population`");
        }
        if spawn.within.is_some() || spawn.at.is_some() {
            return refuse("a spawn in a cell spec is in that cell: it takes no `in` or `at`");
        }
        Ok(SpawnField(spawn))
    },
});

/// A device's configuration and the name it is linked by (10 §3.2,
/// §3.6; 16 §4.2).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeviceDef {
    /// The name `wiring.links` refer to it by.
    #[serde(default)]
    pub name: Option<String>,
    /// Settings handed to the device when it is made.
    #[serde(default)]
    pub config: BTreeMap<String, Lit>,
}

/// The `device` field of a cell spec: a name, or a [`DeviceDef`].
#[derive(Clone, Debug)]
pub struct DeviceField(pub DeviceDef);

shapes!(DeviceField, "a device name or an object", |_site, value| {
    Kind::String => Ok(DeviceField(DeviceDef {
        name: Some(value.read::<String>()?),
        config: BTreeMap::new(),
    })),
    Kind::Object => Ok(DeviceField(value.read::<DeviceDef>()?)),
});

/// A berth a cell belongs to (10 §3.2).
#[derive(Clone, Debug, Deserialize)]
pub struct BerthDef {
    /// The berth's name within the map.
    pub name: String,
    /// What kind of berth it is: a parking bay, a mooring. Free text.
    #[serde(default)]
    pub kind: String,
}

/// The `berth` field of a cell spec: a name, or a [`BerthDef`].
#[derive(Clone, Debug)]
pub struct BerthField(pub BerthDef);

shapes!(BerthField, "a berth name or an object", |_site, value| {
    Kind::String => Ok(BerthField(BerthDef {
        name: value.read::<String>()?,
        kind: String::new(),
    })),
    Kind::Object => Ok(BerthField(value.read::<BerthDef>()?)),
});

/// A cell spec as written (10 §3.2): what one character of a map puts
/// in a cell.
#[derive(Clone, Debug, Default)]
pub struct SpecDef {
    /// Where the spec was written.
    pub site: Site,
    /// The short form: a bare state, whose layer its definition decides.
    pub short: Option<String>,
    /// The floor state.
    pub floor: Option<Located<String>>,
    /// The block state.
    pub block: Option<Located<String>>,
    /// The fixture state.
    pub fixture: Option<Located<String>>,
    /// Layers emptied.
    pub clear: Vec<Located<String>>,
    /// Layers left as the world has them.
    pub keep: Vec<Located<String>>,
    /// Conduits laid in the cell.
    pub conduits: Vec<String>,
    /// Field amounts by field id.
    pub field: BTreeMap<String, Real>,
    /// Items.
    pub items: Option<ItemsDef>,
    /// A creature or population member.
    pub spawn: Option<SpawnWhoDef>,
    /// Device configuration and name.
    pub device: Option<DeviceDef>,
    /// Marks the cell is added to.
    pub mark: Vec<String>,
    /// The berth the cell belongs to.
    pub berth: Option<BerthDef>,
    /// The berth whose approach line the cell is on.
    pub approach: Option<String>,
    /// Whether what is placed here is watched by the site's people.
    pub watched: bool,
    /// The kind of area the cell is added to.
    pub area: Option<Located<String>>,
    /// Cell variables.
    pub var: BTreeMap<String, Lit>,
    /// Decals.
    pub decal: Vec<String>,
    /// For a pattern: any floor carrying this tag.
    pub floor_tag: Option<String>,
    /// For a pattern: any block carrying this tag.
    pub block_tag: Option<String>,
    /// For a pattern: any fixture carrying this tag.
    pub fixture_tag: Option<String>,
    /// For a pattern: anything at all.
    pub any: bool,
}

/// One alternative of a palette value.
#[derive(Clone, Debug)]
pub struct AltDef {
    /// What it puts in the cell.
    pub spec: SpecDef,
    /// Its weight among the alternatives. One if left out.
    pub weight: Option<Expr<Number>>,
}

/// A palette value: one cell spec, or weighted alternatives.
#[derive(Clone, Debug)]
pub struct EntryDef {
    /// The alternatives: exactly one for a plain cell spec.
    pub alternatives: Vec<AltDef>,
    /// Whether one alternative is chosen for the whole placement and
    /// not for each cell.
    pub once: bool,
    /// Where the value was written.
    pub site: Site,
}

#[derive(Deserialize, Default)]
struct SpecObject {
    #[serde(default)]
    floor: Option<Located<String>>,
    #[serde(default)]
    block: Option<Located<String>>,
    #[serde(default)]
    fixture: Option<Located<String>>,
    #[serde(default)]
    clear: Vec<Located<String>>,
    #[serde(default)]
    keep: Vec<Located<String>>,
    #[serde(default)]
    conduits: Vec<String>,
    #[serde(default)]
    field: BTreeMap<String, Real>,
    #[serde(default)]
    items: Option<ItemsField>,
    #[serde(default)]
    spawn: Option<SpawnField>,
    #[serde(default)]
    device: Option<DeviceField>,
    #[serde(default)]
    mark: Option<Names>,
    #[serde(default)]
    berth: Option<BerthField>,
    #[serde(default)]
    approach: Option<String>,
    #[serde(default)]
    watched: bool,
    #[serde(default)]
    area: Option<Located<String>>,
    #[serde(default)]
    var: BTreeMap<String, Lit>,
    #[serde(default)]
    decal: Option<Names>,
    #[serde(default)]
    floor_tag: Option<String>,
    #[serde(default)]
    block_tag: Option<String>,
    #[serde(default)]
    fixture_tag: Option<String>,
    #[serde(default)]
    any: bool,
    #[serde(default)]
    weight: Option<Expr<Number>>,
    #[serde(default)]
    one_of: Option<Vec<AltItem>>,
    #[serde(default)]
    once: bool,
}

impl SpecObject {
    fn is_plain(&self) -> bool {
        self.one_of.is_none() && !self.once
    }

    fn states_nothing(&self) -> bool {
        self.floor.is_none()
            && self.block.is_none()
            && self.fixture.is_none()
            && self.clear.is_empty()
            && self.keep.is_empty()
            && self.conduits.is_empty()
            && self.field.is_empty()
            && self.items.is_none()
            && self.spawn.is_none()
            && self.device.is_none()
            && self.mark.is_none()
            && self.berth.is_none()
            && self.approach.is_none()
            && !self.watched
            && self.area.is_none()
            && self.var.is_empty()
            && self.decal.is_none()
            && self.floor_tag.is_none()
            && self.block_tag.is_none()
            && self.fixture_tag.is_none()
            && !self.any
    }

    fn into_alt(self, site: Site) -> AltDef {
        AltDef {
            weight: self.weight,
            spec: SpecDef {
                site,
                short: None,
                floor: self.floor,
                block: self.block,
                fixture: self.fixture,
                clear: self.clear,
                keep: self.keep,
                conduits: self.conduits,
                field: self.field,
                items: self.items.map(|items| items.0),
                spawn: self.spawn.map(|spawn| spawn.0),
                device: self.device.map(|device| device.0),
                mark: self.mark.map(|names| names.0).unwrap_or_default(),
                berth: self.berth.map(|berth| berth.0),
                approach: self.approach,
                watched: self.watched,
                area: self.area,
                var: self.var,
                decal: self.decal.map(|names| names.0).unwrap_or_default(),
                floor_tag: self.floor_tag,
                block_tag: self.block_tag,
                fixture_tag: self.fixture_tag,
                any: self.any,
            },
        }
    }
}

fn short_alt(text: String, site: Site) -> AltDef {
    AltDef {
        weight: None,
        spec: SpecDef {
            site,
            short: Some(text),
            ..SpecDef::default()
        },
    }
}

/// One alternative as written inside a list.
struct AltItem(AltDef);

shapes!(AltItem, "a state or a cell spec", |site, value| {
    Kind::String => Ok(AltItem(short_alt(value.read::<String>()?, site))),
    Kind::Object => {
        let object = value.read::<SpecObject>()?;
        if !object.is_plain() {
            return refuse("alternatives do not nest: `one_of` and `once` belong to the list");
        }
        Ok(AltItem(object.into_alt(site)))
    },
});

shapes!(EntryDef, "a state, a cell spec or a list of alternatives", |site, value| {
    Kind::String => Ok(EntryDef {
        alternatives: vec![short_alt(value.read::<String>()?, site.clone())],
        once: false,
        site,
    }),
    Kind::List => {
        let alternatives: Vec<AltDef> = value
            .read::<Vec<AltItem>>()?
            .into_iter()
            .map(|item| item.0)
            .collect();
        if alternatives.is_empty() {
            return refuse("a list of alternatives needs at least one");
        }
        Ok(EntryDef { alternatives, once: false, site })
    },
    Kind::Object => {
        let mut object = value.read::<SpecObject>()?;
        match object.one_of.take() {
            Some(alternatives) => {
                if alternatives.is_empty() {
                    return refuse("`one_of` needs at least one alternative");
                }
                if !object.states_nothing() || object.weight.is_some() {
                    return refuse(
                        "an object with `one_of` holds only `one_of` and `once`: what goes in \
                         the cell is written in the alternatives",
                    );
                }
                Ok(EntryDef {
                    alternatives: alternatives.into_iter().map(|item| item.0).collect(),
                    once: object.once,
                    site,
                })
            }
            None => {
                if object.once {
                    return refuse("`once` belongs with `one_of`, which is not here");
                }
                if object.weight.is_some() {
                    return refuse("`weight` belongs to an alternative of a list");
                }
                Ok(EntryDef {
                    alternatives: vec![object.into_alt(site.clone())],
                    once: false,
                    site,
                })
            }
        }
    },
});

/// One item of a map's `palette` list: a shared palette by id, or
/// entries written in place.
#[derive(Clone, Debug)]
pub enum PaletteItem {
    /// A `palette` definition.
    Named(Ref<PaletteDef>),
    /// Entries by character.
    Inline(BTreeMap<String, EntryDef>, Site),
}

shapes!(PaletteItem, "a palette id or an object of entries", |site, value| {
    Kind::String => Ok(PaletteItem::Named(value.read::<Ref<PaletteDef>>()?)),
    Kind::Object => Ok(PaletteItem::Inline(value.read::<BTreeMap<String, EntryDef>>()?, site)),
});

/// One storey of a map: rows of characters, or a named preset.
#[derive(Clone, Debug)]
pub enum LevelDef {
    /// Rows, north to south.
    Rows(Vec<Located<String>>, Site),
    /// A preset by name.
    Preset(String, Site),
}

shapes!(LevelDef, "a list of rows or the name of a preset", |site, value| {
    Kind::List => Ok(LevelDef::Rows(value.read::<Vec<Located<String>>>()?, site)),
    Kind::String => Ok(LevelDef::Preset(value.read::<String>()?, site)),
});

/// A point of a map: a cell `[x, y]` or `[x, y, level]`, or a mark,
/// `"mark:name"`.
#[derive(Clone, Debug)]
pub enum PointDef {
    /// A cell. The level is 0 if left out.
    Cell(IVec3, Site),
    /// The first cell of a mark.
    Mark(String, Site),
}

impl PointDef {
    /// Where the point was written.
    pub fn site(&self) -> &Site {
        match self {
            PointDef::Cell(_, site) | PointDef::Mark(_, site) => site,
        }
    }
}

fn cell_of<E: de::Error>(numbers: &[i64]) -> Result<IVec3, E> {
    let narrow = |n: i64| i32::try_from(n).map_err(|_| E::custom("this coordinate is too large"));
    match numbers {
        [x, y] => Ok(IVec3::new(narrow(*x)?, narrow(*y)?, 0)),
        [x, y, z] => Ok(IVec3::new(narrow(*x)?, narrow(*y)?, narrow(*z)?)),
        _ => refuse("a cell is `[x, y]` or `[x, y, level]`"),
    }
}

shapes!(PointDef, "a cell `[x, y, level]` or a mark `\"mark:name\"`", |site, value| {
    Kind::List => Ok(PointDef::Cell(cell_of(&value.read::<Vec<i64>>()?)?, site)),
    Kind::String => {
        let text = value.read::<String>()?;
        match text.strip_prefix(MARK_PREFIX) {
            Some(name) if !name.is_empty() => Ok(PointDef::Mark(name.to_owned(), site)),
            _ => refuse("a point written as text is a mark: `\"mark:name\"`"),
        }
    },
});

/// The cells of a mark: one cell, or a list of cells.
#[derive(Clone, Debug)]
pub struct MarkDef {
    /// The cells.
    pub cells: Vec<IVec3>,
    /// Where the mark was written.
    pub site: Site,
}

enum MarkItem {
    Number(i64),
    Cell(Vec<i64>),
}

shapes!(MarkItem, "a coordinate or a cell", |_site, value| {
    Kind::Number => Ok(MarkItem::Number(value.read::<i64>()?)),
    Kind::List => Ok(MarkItem::Cell(value.read::<Vec<i64>>()?)),
});

shapes!(MarkDef, "a cell `[x, y, level]` or a list of cells", |site, value| {
    Kind::List => {
        let items = value.read::<Vec<MarkItem>>()?;
        let mut numbers = Vec::new();
        let mut cells = Vec::new();
        for item in items {
            match item {
                MarkItem::Number(n) => numbers.push(n),
                MarkItem::Cell(cell) => cells.push(cell_of(&cell)?),
            }
        }
        if !numbers.is_empty() {
            if !cells.is_empty() {
                return refuse("a mark is one cell `[x, y, level]` or a list of cells, not both");
            }
            cells.push(cell_of(&numbers)?);
        }
        if cells.is_empty() {
            return refuse("a mark needs at least one cell");
        }
        Ok(MarkDef { cells, site })
    },
});

/// A span of storeys: one level, or `[lowest, highest]`.
#[derive(Clone, Copy, Debug)]
pub struct LevelsDef(pub i32, pub i32);

shapes!(LevelsDef, "a level or `[lowest, highest]`", |_site, value| {
    Kind::Number => {
        let level = value.read::<i32>()?;
        Ok(LevelsDef(level, level))
    },
    Kind::List => {
        let range = value.read::<Vec<i32>>()?;
        match range.as_slice() {
            [low, high] if low <= high => Ok(LevelsDef(*low, *high)),
            _ => refuse("a span of levels is `[lowest, highest]`"),
        }
    },
});

/// Which map a `place` entry places: one by id, or any that carries a
/// tag.
#[derive(Clone, Debug)]
pub enum MapPickDef {
    /// A map by id.
    Id(Ref<MapDef>, Site),
    /// Any map carrying this tag.
    Tag(String, Site),
}

impl MapPickDef {
    /// Where it was written.
    pub fn site(&self) -> &Site {
        match self {
            MapPickDef::Id(_, site) | MapPickDef::Tag(_, site) => site,
        }
    }
}

#[derive(Deserialize)]
struct TagObject {
    tag: String,
}

shapes!(MapPickDef, "a map id or `{ \"tag\": … }`", |site, value| {
    Kind::String => Ok(MapPickDef::Id(value.read::<Ref<MapDef>>()?, site)),
    Kind::Object => Ok(MapPickDef::Tag(value.read::<TagObject>()?.tag, site)),
});

/// How a nested map is turned: a fixed turn, or whichever fits.
#[derive(Clone, Copy, Debug)]
pub enum TurnDef {
    /// This turn.
    Fixed(Turn),
    /// Any turn the map allows that fits, chosen by the placement.
    Any,
}

#[derive(Deserialize)]
struct TurnObject {
    #[serde(default)]
    rot: i32,
    #[serde(default)]
    mirror: bool,
}

/// Degrees in a quarter-turn.
const QUARTER_DEGREES: i32 = 90;

fn rot_of<E: de::Error>(degrees: i32) -> Result<Rot4, E> {
    if degrees % QUARTER_DEGREES != 0 {
        return refuse("a turn is a whole number of quarter-turns: 0, 90, 180 or 270");
    }
    Ok(Rot4::from_quarter_turns(degrees / QUARTER_DEGREES))
}

shapes!(
    TurnDef,
    "a turn: degrees, `\"any\"` or `{ \"rot\": …, \"mirror\": … }`",
    |_site, value| {
        Kind::Number => Ok(TurnDef::Fixed(Turn::from(rot_of(value.read::<i32>()?)?))),
        Kind::String => match value.read::<String>()?.as_str() {
            "any" => Ok(TurnDef::Any),
            "mirror" => Ok(TurnDef::Fixed(Turn::new(Rot4::R0, true))),
            _ => refuse("a turn written as text is `\"any\"` or `\"mirror\"`"),
        },
        Kind::Object => {
            let object = value.read::<TurnObject>()?;
            Ok(TurnDef::Fixed(Turn::new(rot_of(object.rot)?, object.mirror)))
        },
    }
);

/// One entry of a map's `place`: another map put inside this one
/// (10 §3.5).
#[derive(Clone, Debug, Deserialize)]
pub struct PlaceDef {
    /// Which map.
    pub map: MapPickDef,
    /// Where its anchor goes: a cell or a mark.
    #[serde(default)]
    pub at: Option<PointDef>,
    /// The area it is scattered through, in place of `at`.
    #[serde(default, rename = "in")]
    pub within: Option<Located<String>>,
    /// How it is turned. Not turned if left out.
    #[serde(default)]
    pub turn: Option<TurnDef>,
    /// The chance that it is placed at all.
    #[serde(default)]
    pub chance: Option<Expr<Number>>,
    /// How many are scattered through the area. One if left out.
    #[serde(default)]
    pub count: Option<CountDef>,
    /// How many cells must lie between two scattered copies.
    #[serde(default)]
    pub apart: Option<u32>,
    /// Parameters handed to the nested map, each an expression over
    /// this map's parameters.
    #[serde(default)]
    pub params: BTreeMap<String, Expr<Any>>,
}

/// One entry of a map's `set`: a cell spec applied at a cell or over a
/// box (10 §3.1).
#[derive(Clone, Debug, Deserialize)]
pub struct SetDef {
    /// The cell or mark.
    #[serde(default)]
    pub at: Option<PointDef>,
    /// The box, `[[x0, y0], [x1, y1]]`, both corners included.
    #[serde(default, rename = "box")]
    pub bounds: Option<Sited<Vec<Vec<i64>>>>,
    /// The storeys of the box. Level 0 if left out.
    #[serde(default)]
    pub level: Option<LevelsDef>,
    /// What is put there.
    pub spec: EntryDef,
}

/// One entry of a map's `areas` (02 §8.3).
#[derive(Clone, Debug, Deserialize)]
pub struct AreaDef {
    /// The `area_kind` definition.
    pub kind: Located<String>,
    /// The name `place` and `spawns` refer to it by. Its kind if left
    /// out.
    #[serde(default)]
    pub name: Option<String>,
    /// The storeys it covers. Level 0 if left out.
    #[serde(default)]
    pub level: Option<LevelsDef>,
    /// The box, `[[x0, y0], [x1, y1]]`, both corners included.
    #[serde(rename = "box")]
    pub bounds: Sited<Vec<Vec<i64>>>,
    /// Tags the area carries besides its kind's.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The `wiring` of a map: conduit layers over the same grid, and links
/// between named devices (10 §3.6).
#[derive(Clone, Debug, Deserialize)]
pub struct WiringDef {
    /// The conduits each character lays.
    #[serde(default)]
    pub palette: BTreeMap<String, Sited<Vec<String>>>,
    /// Rows by storey.
    #[serde(default)]
    pub levels: BTreeMap<String, Sited<Vec<Located<String>>>>,
    /// Links between devices named in cell specs.
    #[serde(default)]
    pub links: Vec<LinkDef>,
}

/// One link between two named devices.
#[derive(Clone, Debug, Deserialize)]
pub struct LinkDef {
    /// The name of the device it runs from.
    pub from: Located<String>,
    /// The name of the device it runs to.
    pub to: Located<String>,
    /// The resource it carries.
    #[serde(default)]
    pub resource: Option<String>,
}

/// One entry of a map's `joints`: a labelled opening on its edge where
/// another map can connect (10 §7.2).
#[derive(Clone, Debug, Deserialize)]
pub struct JointDef {
    /// The joint's label within the map.
    #[serde(default)]
    pub id: Option<String>,
    /// The cell, `[x, y]` or `[x, y, level]`.
    pub at: Sited<Vec<i64>>,
    /// Which way it opens: `north`, `east`, `south`, `west`, `up` or
    /// `down`.
    pub dir: Located<String>,
    /// What kind of opening it is: a doorway, a hatch. Free text;
    /// joints of the same kind match.
    pub kind: String,
}

/// What level 0 of a map sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Base {
    /// The ground under the placement.
    #[default]
    Ground,
    /// The realm's sea level.
    SeaLevel,
    /// The sea floor under the placement.
    SeaFloor,
    /// An absolute storey of the realm.
    Storey(i32),
}

shapes!(
    Base,
    "`ground`, `sea_level`, `sea_floor` or a storey",
    |_site, value| {
        Kind::Number => Ok(Base::Storey(value.read::<i32>()?)),
        Kind::String => match value.read::<String>()?.as_str() {
            "ground" => Ok(Base::Ground),
            "sea_level" => Ok(Base::SeaLevel),
            "sea_floor" => Ok(Base::SeaFloor),
            _ => refuse("`base` is `ground`, `sea_level`, `sea_floor` or a storey"),
        },
    }
);

/// One entry of a map's `portals` (02 §19).
#[derive(Clone, Debug, Deserialize)]
pub struct PortalDef {
    /// The mark one end stands at: `"mark:name"`.
    pub from: Located<String>,
    /// The other end: `"mark:name"` in this map, or a named entry
    /// elsewhere.
    pub to: Located<String>,
}

/// One entry of a map's `vehicles` (11 §12.1).
#[derive(Clone, Debug, Deserialize)]
pub struct VehicleDef {
    /// The vehicle definition.
    pub vehicle: String,
    /// The cell or mark it stands at.
    pub at: PointDef,
    /// Which way it points.
    #[serde(default)]
    pub facing: Option<Dir8>,
}

fn yes() -> bool {
    true
}

/// A place written as rows of characters, one layer for each storey
/// (10 §3.1).
#[derive(Debug, Deserialize)]
pub struct MapDef {
    /// Free-form tags; a map may be chosen by one.
    #[serde(default)]
    pub tags: TagSet,
    /// Width and depth in cells.
    pub size: Sited<Vec<i64>>,
    /// Which side faces a street.
    #[serde(default)]
    pub front: Option<Located<Dir8>>,
    /// Values chosen once for each placement.
    #[serde(default)]
    pub params: Ordered<ParamDef>,
    /// What each character means: named palettes and entries written in
    /// place, later overriding earlier.
    #[serde(default)]
    pub palette: Vec<PaletteItem>,
    /// The cell spec laid under every character that is not a space.
    #[serde(default)]
    pub default: Option<EntryDef>,
    /// The cell spec the `roof` preset lays. The `default` if left out.
    #[serde(default)]
    pub roof: Option<EntryDef>,
    /// Storeys by number.
    pub levels: BTreeMap<String, LevelDef>,
    /// Other maps placed inside this one.
    #[serde(default)]
    pub place: Vec<PlaceDef>,
    /// Cell specs applied at cells and over boxes.
    #[serde(default)]
    pub set: Vec<SetDef>,
    /// Areas.
    #[serde(default)]
    pub areas: Vec<AreaDef>,
    /// Named cells.
    #[serde(default)]
    pub marks: BTreeMap<String, MarkDef>,
    /// Creatures and populations.
    #[serde(default)]
    pub spawns: Vec<SpawnWhoDef>,
    /// Conduit layers and device links.
    #[serde(default)]
    pub wiring: Option<WiringDef>,
    /// Where it can connect to other maps.
    #[serde(default)]
    pub joints: Vec<JointDef>,
    /// The cell a placement names when it says where the map goes. The
    /// north-west corner at level 0 if left out.
    #[serde(default)]
    pub anchor: Option<PointDef>,
    /// What level 0 sits on.
    #[serde(default)]
    pub base: Option<Base>,
    /// How it meets uneven ground: `level`, `foundation`, `follow` or
    /// `none`.
    #[serde(default)]
    pub sit: Option<Located<String>>,
    /// For `"sit": "foundation"`: the block state that fills gaps under
    /// the map.
    #[serde(default)]
    pub foundation: Option<Located<String>>,
    /// Whether a placement may turn it.
    #[serde(default = "yes")]
    pub turn: bool,
    /// Whether it may overlap another placement.
    #[serde(default)]
    pub over: bool,
    /// Portals between marks.
    #[serde(default)]
    pub portals: Vec<PortalDef>,
    /// Vehicles standing at marks.
    #[serde(default)]
    pub vehicles: Vec<VehicleDef>,
    /// A condition on the parameters; if false the map is not placed.
    #[serde(default, rename = "if")]
    pub condition: Option<Cond>,
}

/// A shared palette (10 §3.2): what each character means, for any map
/// that lists it.
#[derive(Debug, Deserialize)]
pub struct PaletteDef {
    /// Entries by character.
    pub entries: BTreeMap<String, EntryDef>,
}

/// The type a parameter has in expressions, by what its rule says.
fn lit_type(value: &clash_core::Value) -> &'static str {
    value.type_name()
}

impl ParamDef {
    /// The type the parameter's rule gives it, as the expression
    /// language names it, or `None` for an expression that has not been
    /// compiled.
    pub fn rule_type(&self) -> Option<String> {
        match &self.rule {
            ParamRule::Pick(values) => {
                let mut ty = lit_type(&values.first()?.0);
                for value in values {
                    let other = lit_type(&value.0);
                    if other != ty {
                        let numeric = |name: &str| name == "int" || name == "real";
                        ty = if numeric(ty) && numeric(other) {
                            "real"
                        } else {
                            "any"
                        };
                    }
                }
                Some(ty.to_owned())
            }
            ParamRule::Range(low, high) => {
                let whole = matches!(
                    (&low.0, &high.0),
                    (clash_core::Value::Int(_), clash_core::Value::Int(_))
                );
                Some(if whole { "int" } else { "real" }.to_owned())
            }
            ParamRule::Default(value) => Some(lit_type(&value.0).to_owned()),
            ParamRule::Expr(_) | ParamRule::Required => None,
        }
    }
}

/// Compiles the expression fields of maps in the generation context
/// (10 §3.4) and refuses what a map may not read.
struct GenCompiler<'a, 'b> {
    cx: &'a mut CompileCx<'b>,
    scope: Scope,
}

impl GenCompiler<'_, '_> {
    fn expr<T: ExprType>(&mut self, field: &mut Expr<T>) {
        self.cx.compile(field, &self.scope);
        let reads_world = field.compiled().is_some_and(|code| {
            !code.properties_read().is_empty() || !code.variables_read().is_empty()
        });
        let site = field.site().clone();
        if reads_world {
            self.refuse_at(
                &site,
                None,
                "an expression in a map reads only its parameters: a map is resolved before \
                 there is a world to ask",
            );
        }
        if let Some(source) = field.source() {
            let source = source.to_owned();
            self.check_calls(&source, &site);
        }
    }

    fn cond(&mut self, field: &mut Cond) {
        self.cx.compile(field, &self.scope);
        if let Some(source) = field.source() {
            let source = source.to_owned();
            let site = field.site().clone();
            self.check_calls(&source, &site);
        }
    }

    fn count(&mut self, field: &mut Option<CountDef>) {
        if let Some(CountDef::Expr(expr)) = field {
            self.expr(expr);
        }
    }

    fn chance(&mut self, field: &mut Option<Expr<Number>>) {
        if let Some(expr) = field {
            self.expr(expr);
        }
    }

    fn entry(&mut self, entry: &mut EntryDef) {
        for alt in &mut entry.alternatives {
            self.chance(&mut alt.weight);
            if let Some(items) = &mut alt.spec.items {
                self.count(&mut items.count);
                self.chance(&mut items.chance);
            }
            if let Some(spawn) = &mut alt.spec.spawn {
                self.count(&mut spawn.count);
                self.chance(&mut spawn.chance);
            }
        }
    }

    /// Refuses a call of a function that only a running simulation can
    /// answer: a map is resolved without one.
    fn check_calls(&mut self, source: &str, site: &Site) {
        let Ok(tree) = clash_expr::parse_expr(source) else {
            return;
        };
        let mut calls = Vec::new();
        collect_calls(&tree, &mut calls);
        for (name, start, end) in calls {
            let registry = self.cx.registry();
            let answered_by_host = registry
                .function_id(&name)
                .and_then(|id| registry.function(id))
                .is_some_and(|function| !function.is_builtin() && !name.contains(':'));
            if answered_by_host {
                self.refuse_at(
                    site,
                    Some((start, end)),
                    format!(
                        "`{name}` is answered by a running simulation, and a map is resolved \
                         without one: an expression in a map uses its parameters, the pure \
                         built-in functions and the random functions, which draw from the \
                         placement"
                    ),
                );
            }
        }
    }

    fn refuse_at(
        &mut self,
        site: &Site,
        within: Option<(usize, usize)>,
        message: impl Into<String>,
    ) {
        let mut diagnostic =
            Diagnostic::new(code::VALIDATION, message).with_path(site.path.clone());
        if let Some(span) = site.span {
            let span = match within {
                Some((start, end)) => self.cx.sources().span_in_string(span, start, end),
                None => span,
            };
            diagnostic = diagnostic.at(self.cx.sources(), span);
        }
        self.cx.report(diagnostic);
    }
}

/// The names of the functions an expression calls, with where each is
/// written in the source.
fn collect_calls(tree: &Tree, out: &mut Vec<(String, usize, usize)>) {
    match &tree.kind {
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Number { .. }
        | ExprKind::Str(_)
        | ExprKind::Name(_)
        | ExprKind::Local(_) => {}
        ExprKind::List(items) => items.iter().for_each(|item| collect_calls(item, out)),
        ExprKind::Map(entries) => {
            for (key, value) in entries {
                collect_calls(key, out);
                collect_calls(value, out);
            }
        }
        ExprKind::Member { object, .. } => collect_calls(object, out),
        ExprKind::Call { callee, args } => {
            match &callee.kind {
                ExprKind::Name(name) => out.push((
                    name.to_string(),
                    callee.span.start as usize,
                    callee.span.end as usize,
                )),
                _ => collect_calls(callee, out),
            }
            args.iter().for_each(|arg| collect_calls(arg, out));
        }
        ExprKind::Index { object, index } => {
            collect_calls(object, out);
            collect_calls(index, out);
        }
        ExprKind::Unary { operand, .. } => collect_calls(operand, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_calls(lhs, out);
            collect_calls(rhs, out);
        }
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            collect_calls(cond, out);
            collect_calls(then, out);
            collect_calls(otherwise, out);
        }
    }
}

/// The type parameters take when nothing says: a number.
const DEFAULT_PARAM_TYPE: &str = "real";

/// The types a parameter may be declared with.
pub const PARAM_TYPES: [&str; 4] = ["bool", "int", "real", "string"];

impl Def for MapDef {
    type Id = MapId;
    const TYPE: &'static str = "map";
    const DOC: &'static str = "A place written as rows of characters, one layer for each storey, \
                               with a palette saying what each character stands for (10 §3).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        // The generation context (10 §3.4): the parameters, in the
        // order they are declared, and nothing of the world. Random
        // functions are allowed: when a map is resolved they draw from
        // a generator seeded by the placement and the cell.
        let mut gen_cx = GenCompiler {
            cx,
            scope: Scope::new(),
        };
        let mut params = Scope::new();
        for (name, param) in &mut self.params.0 {
            let declared = param
                .ty
                .as_ref()
                .filter(|ty| PARAM_TYPES.contains(&ty.value.as_str()))
                .map(|ty| ty.value.clone());
            let ty = match &mut param.rule {
                ParamRule::Expr(expr) => {
                    // A parameter sees those declared before it. Its
                    // text is an expression whatever type it is
                    // declared with, so the field is not told to expect
                    // a text: building the maps compares the types.
                    gen_cx.scope = params.clone();
                    gen_cx.expr(expr);
                    let found = expr
                        .compiled()
                        .map(|code| gen_cx.cx.registry().type_name(code.ty().non_null()))
                        .or_else(|| {
                            expr.constant()
                                .and_then(|value| value_type(value).map(str::to_owned))
                        });
                    declared
                        .or(found.filter(|ty| PARAM_TYPES.contains(&ty.as_str())))
                        .unwrap_or_else(|| DEFAULT_PARAM_TYPE.to_owned())
                }
                _ => declared
                    .or_else(|| param.rule_type())
                    .unwrap_or_else(|| DEFAULT_PARAM_TYPE.to_owned()),
            };
            params = params.local(name.clone(), ty.as_str());
        }
        gen_cx.scope = params;
        if let Some(condition) = &mut self.condition {
            gen_cx.cond(condition);
        }
        for item in &mut self.palette {
            if let PaletteItem::Inline(entries, _) = item {
                entries.values_mut().for_each(|entry| gen_cx.entry(entry));
            }
        }
        if let Some(entry) = &mut self.default {
            gen_cx.entry(entry);
        }
        if let Some(entry) = &mut self.roof {
            gen_cx.entry(entry);
        }
        for set in &mut self.set {
            gen_cx.entry(&mut set.spec);
        }
        for place in &mut self.place {
            gen_cx.chance(&mut place.chance);
            gen_cx.count(&mut place.count);
            for value in place.params.values_mut() {
                gen_cx.expr(value);
            }
        }
        for spawn in &mut self.spawns {
            gen_cx.count(&mut spawn.count);
            gen_cx.chance(&mut spawn.chance);
        }
    }
}

/// The parameter type a constant has, if it is one a parameter can be.
fn value_type(value: &clash_expr::Value) -> Option<&'static str> {
    match value {
        clash_expr::Value::Bool(_) => Some("bool"),
        clash_expr::Value::Int(_) => Some("int"),
        clash_expr::Value::Real(_) => Some("real"),
        clash_expr::Value::Str(_) => Some("string"),
        _ => None,
    }
}

impl Def for PaletteDef {
    type Id = PaletteId;
    const TYPE: &'static str = "palette";
    const DOC: &'static str =
        "What each character of a map stands for, shared between maps (10 §3.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        // A palette is shared by maps with different parameters, so its
        // expressions see none: only the random functions.
        let mut gen_cx = GenCompiler {
            cx,
            scope: Scope::new(),
        };
        self.entries
            .values_mut()
            .for_each(|entry| gen_cx.entry(entry));
    }
}
