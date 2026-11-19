//! Definition types for the tests. They stand in for the types other
//! crates will register: none of them means anything to the engine.

#![allow(dead_code, missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use clash_core::{Dur, Real, define_id};
use clash_data::{
    CODES, CrossCx, Def, DefTypes, Diagnostics, Energy, Feature, FinishCx, Frequency, FromKind,
    Gated, Inline, Kind, KindValue, Length, LoadOutcome, Loader, Located, Mass, Power, Pressure,
    Rate, Ref, Speed, TagSet, Temperature, Text, Time, ValidateCx, Volume, by_kind, code,
};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

define_id! {
    pub struct DamageTypeId(u16);
    pub struct MaterialId(u16);
    pub struct BlockId(u16);
    pub struct ActionId;
    pub struct RuleId;
    pub struct GizmoId(u8);
}

#[derive(Debug, Deserialize)]
pub struct DamageTypeDef {
    #[serde(default)]
    pub name: Option<Text>,
}

impl Def for DamageTypeDef {
    type Id = DamageTypeId;
    const TYPE: &'static str = "damage_type";
    const DOC: &'static str = "A kind of harm.";
}

fn one() -> Real {
    Real::ONE
}

#[derive(Debug, Deserialize)]
pub struct Resist {
    #[serde(default)]
    pub threshold: Option<Pressure>,
    #[serde(default = "one")]
    pub factor: Real,
}

/// The experimental feature of the test material's `glow` field.
pub struct ColouredLight;

impl Feature for ColouredLight {
    const NAME: &'static str = "coloured_light";
}

#[derive(Debug, Deserialize)]
pub struct MaterialDef {
    pub density: Real,
    pub toughness: Real,
    #[serde(default)]
    pub resist: BTreeMap<Ref<DamageTypeDef>, Resist>,
    #[serde(default = "default_span")]
    pub span: u32,
    #[serde(default)]
    pub bearing: Option<Pressure>,
    #[serde(default)]
    pub melts_at: Option<Temperature>,
    #[serde(default)]
    pub flammable: bool,
    #[serde(default)]
    pub tags: TagSet,
    #[serde(default)]
    pub name: Option<Text>,
    #[serde(default)]
    pub rubble: Option<Rubble>,
    #[serde(default)]
    pub glow: Option<Gated<ColouredLight, String>>,
}

/// A field that takes two shapes: one block, or a block per damage type.
#[derive(Debug, PartialEq)]
pub enum Rubble {
    One(Ref<BlockDef>),
    ByDamage(BTreeMap<Ref<DamageTypeDef>, Ref<BlockDef>>),
}

impl<'de> FromKind<'de> for Rubble {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Rubble, A::Error> {
        match kind {
            Kind::String => value.read().map(Rubble::One),
            Kind::Object => value.read().map(Rubble::ByDamage),
            other => Err(other.unexpected("a block id, or a block id for each damage type")),
        }
    }
}

impl<'de> Deserialize<'de> for Rubble {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Rubble, D::Error> {
        by_kind(deserializer)
    }
}

fn default_span() -> u32 {
    1
}

impl MaterialDef {
    fn inert() -> MaterialDef {
        MaterialDef {
            density: Real::ONE,
            toughness: Real::ONE,
            resist: BTreeMap::new(),
            span: 1,
            bearing: None,
            melts_at: None,
            flammable: false,
            tags: TagSet::new(),
            name: None,
            rubble: None,
            glow: None,
        }
    }
}

impl Def for MaterialDef {
    type Id = MaterialId;
    const TYPE: &'static str = "material";
    const DOC: &'static str = "What things are made of.";
    const DEFAULTS: &'static str = r#"{ "span": 1 }"#;

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.density <= Real::ZERO {
            cx.error("`density` must be above zero");
        }
        if self.span > 64 {
            cx.warn("a `span` above 64 cells is unusual");
        }
    }

    fn placeholder() -> Option<MaterialDef> {
        Some(MaterialDef::inert())
    }

    fn builtins() -> Vec<(&'static str, MaterialDef)> {
        vec![("none", MaterialDef::inert())]
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    Opaque,
    Passable,
    LoadBearing,
    Anchor,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Full,
    Slope { rise: Length },
    Pillar(Length),
}

#[derive(Debug, Deserialize)]
pub struct BlockDef {
    pub material: Ref<MaterialDef>,
    pub height: Length,
    #[serde(default)]
    pub fill: Real,
    #[serde(default)]
    pub flags: Vec<Flag>,
    #[serde(default)]
    pub shape: Option<Shape>,
    #[serde(default)]
    pub size: Option<(u8, u8)>,
}

impl Def for BlockDef {
    type Id = BlockId;
    const TYPE: &'static str = "block";
    const DOC: &'static str = "What fills a cell.";
}

#[derive(Debug, Deserialize)]
pub struct Phase {
    pub id: String,
    pub time: Time,
    #[serde(default)]
    pub on_end: Option<Inline<RuleDef>>,
}

#[derive(Debug, Deserialize)]
pub struct ActionDef {
    pub name: Text,
    #[serde(default)]
    pub phases: Vec<Phase>,
    #[serde(default)]
    pub cost: Option<Energy>,
    #[serde(default)]
    pub reach: Option<Length>,
    #[serde(default)]
    pub speed: Option<Speed>,
    #[serde(default)]
    pub drain: Option<Rate<Mass>>,
    #[serde(default)]
    pub power: Option<Power>,
    #[serde(default)]
    pub every: Option<Frequency>,
    #[serde(default)]
    pub dose: Option<Volume>,
    #[serde(default, with = "clash_data::dur::option")]
    pub cooldown: Option<Dur>,
    #[serde(default)]
    pub rules: Vec<Inline<RuleDef>>,
}

impl Def for ActionDef {
    type Id = ActionId;
    const TYPE: &'static str = "action";
    const DOC: &'static str = "Something a character does over time.";
}

/// A stand-in for a rule. `when` stands for an expression: the test
/// "compiler" refuses the character `@` and reports where it is.
/// `then` stands for effects: the verb `rewind` is experimental.
#[derive(Debug, Deserialize)]
pub struct RuleDef {
    pub when: Located<String>,
    #[serde(default)]
    pub then: Vec<Located<String>>,
    /// A reference in a syntax of the type's own: `material@state`.
    #[serde(default)]
    pub target: Option<Located<String>>,
    #[serde(skip)]
    pub target_material: Option<MaterialId>,
}

impl Def for RuleDef {
    type Id = RuleId;
    const TYPE: &'static str = "rule";
    const DOC: &'static str = "Something that happens when a condition holds.";

    fn finish(&mut self, cx: &mut FinishCx<'_>) {
        if let Some(at) = self.when.value.find('@') {
            let span = cx.sources().span_in_string(self.when.span, at, at + 1);
            cx.error(span, "unexpected `@` in an expression");
        }
        for verb in &self.then {
            if verb.value == "rewind" {
                cx.use_experimental("world_rewind", verb.span);
            }
        }
        if let Some(target) = &self.target {
            let material = target.value.split('@').next().unwrap_or("");
            let span = cx.sources().span_in_string(target.span, 0, material.len());
            self.target_material = cx.resolve::<MaterialDef>(material, span);
        }
    }
}

/// A whole type that is an experimental feature, with a narrow id.
#[derive(Debug, Deserialize)]
pub struct GizmoDef {
    #[serde(default)]
    pub charge: Option<Energy>,
}

impl Def for GizmoDef {
    type Id = GizmoId;
    const TYPE: &'static str = "gizmo";
    const EXPERIMENTAL: Option<&'static str> = Some("gizmos");
}

/// A whole-set check: no two materials may share a name.
fn unique_material_names(cx: &mut CrossCx<'_>) {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let defs = cx.defs();
    let mut clashes = Vec::new();
    for (id, material) in defs.iter::<MaterialDef>() {
        let (Some(name), Some(qualified)) = (&material.name, defs.name::<MaterialDef>(id)) else {
            continue;
        };
        if let Some(first) = seen.insert(name.raw().to_owned(), qualified.to_owned())
            && let Some(span) = name.span()
        {
            clashes.push((span, format!("{qualified} has the same name as {first}")));
        }
    }
    for (span, message) in clashes {
        cx.report_at(code::VALIDATION_WARNING, span, message);
    }
}

pub fn types() -> DefTypes {
    let mut types = DefTypes::new();
    types
        .register::<DamageTypeDef>()
        .register::<MaterialDef>()
        .register::<BlockDef>()
        .register::<ActionDef>()
        .register::<RuleDef>()
        .register::<GizmoDef>()
        .add_cross_check(unique_material_names);
    types
}

/// The directory of on-disk fixtures.
pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// A mod in memory: directory name, extra `mod.json` fields, data files.
pub struct TestMod {
    pub id: &'static str,
    pub manifest: String,
    pub files: Vec<(&'static str, String)>,
}

/// A mod whose `mod.json` is `{ "id": id, <extra> }`.
pub fn test_mod(id: &'static str, extra: &str, files: &[(&'static str, &str)]) -> TestMod {
    let manifest = if extra.is_empty() {
        format!("{{ \"id\": \"{id}\" }}")
    } else {
        format!("{{ \"id\": \"{id}\", {extra} }}")
    };
    TestMod {
        id,
        manifest,
        files: files
            .iter()
            .map(|(path, text)| (*path, (*text).to_owned()))
            .collect(),
    }
}

pub fn loader<'t>(types: &'t DefTypes, mods: &[TestMod]) -> Loader<'t> {
    let mut loader = Loader::new(types);
    for one in mods {
        let files: Vec<(&str, &str)> = one
            .files
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .collect();
        loader = loader.memory_mod(one.id, &one.manifest, &files);
    }
    loader
}

/// Loads every given mod, ordered by id.
pub fn load(mods: &[TestMod]) -> LoadOutcome {
    let types = types();
    let outcome = loader(&types, mods).load_all();
    assert_known_codes(&outcome.diagnostics);
    outcome
}

/// Loads one mod named `m` holding one data file.
pub fn load_one(data: &str) -> LoadOutcome {
    load(&[test_mod("m", "", &[("data/a.json", data)])])
}

/// Every diagnostic carries a code that `explain` knows.
pub fn assert_known_codes(diagnostics: &Diagnostics) {
    for diagnostic in diagnostics {
        assert!(
            CODES.iter().any(|info| info.code.id == diagnostic.code),
            "code {} has no explanation",
            diagnostic.code
        );
    }
}

/// The `(code, message)` of every diagnostic, in order.
pub fn summary(outcome: &LoadOutcome) -> Vec<(String, String)> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.code.clone(), diagnostic.message.clone()))
        .collect()
}

/// Asserts a load had nothing to say and returns its definitions.
pub fn clean(outcome: LoadOutcome) -> std::sync::Arc<clash_data::Defs> {
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("a clean load gives definitions")
}

/// The one diagnostic with `code`, or a panic that shows them all.
pub fn only<'a>(outcome: &'a LoadOutcome, code: &str) -> &'a clash_data::Diagnostic {
    let found: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected one {code}:\n{}",
        outcome.diagnostics.render()
    );
    found[0]
}
