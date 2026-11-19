use std::path::{Path, PathBuf};

use clash_data::{Limits, Node, SourceMap};

use super::validate::{Problem, Validator};
use super::*;

/// The engine's definition types with what they read, worked out once
/// for every test.
fn engine_entries() -> &'static [TypeEntry] {
    &crate::reference::tests::catalog().types
}

/// The repository's directory of mods.
fn mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

fn json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            json_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
}

/// Parses a file, with the place of every value.
fn parse(sources: &mut SourceMap, path: &Path) -> Node {
    let text = std::fs::read_to_string(path).expect("the file reads");
    let file = sources.add(path.to_string_lossy(), &text);
    let (tree, problems) = clash_data::parse_json(file, sources.text(file), &Limits::default());
    assert!(problems.is_empty(), "{}: {problems:?}", path.display());
    tree.expect("a tree")
}

fn render(sources: &SourceMap, problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|problem| {
            let at = sources.locate(problem.span);
            format!(
                "{}:{}: {}: {}",
                at.file, at.line, problem.path, problem.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Checks every data file of the bundled mods against a schema.
fn check_bundled(schema: &Json) {
    let validator = Validator::new(schema);
    let mut files = Vec::new();
    let Ok(mods) = std::fs::read_dir(mods_dir()) else {
        panic!("no mods directory");
    };
    let mut mods: Vec<PathBuf> = mods.filter_map(Result::ok).map(|e| e.path()).collect();
    mods.sort();
    for dir in mods {
        json_files(&dir.join("data"), &mut files);
    }
    assert!(files.len() > 20, "the bundled mods have their data files");
    let mut sources = SourceMap::new();
    let mut problems = Vec::new();
    for path in &files {
        let tree = parse(&mut sources, path);
        problems.extend(validator.validate(&tree));
    }
    assert!(
        problems.is_empty(),
        "{} problems in the bundled mods:\n{}",
        problems.len(),
        render(&sources, &problems)
    );
}

#[test]
fn the_schemas_accept_every_bundled_mod() {
    let schema = Schemas::new(engine_entries(), None).data_schema();
    check_bundled(&schema);
}

#[test]
fn the_schemas_with_ids_accept_every_bundled_mod() {
    let types = crate::engine_types();
    let outcome = clash_scene::engine_loader(&types)
        .search_path(mods_dir())
        .load_all();
    let defs = outcome.defs.expect("the bundled mods load");
    let ids = ids_of(&defs);
    assert!(ids["material"].contains(&"core:glass".to_owned()));
    assert!(ids["material"].contains(&"glass".to_owned()));
    let schema = Schemas::new(engine_entries(), Some(&ids)).data_schema();
    check_bundled(&schema);
}

#[test]
fn a_types_own_schema_accepts_its_bundled_definitions_and_refuses_others() {
    let entries = engine_entries();
    let schemas = Schemas::new(entries, None);
    let entry = entries
        .iter()
        .find(|entry| entry.name == "action")
        .expect("action");
    let schema = schemas.type_schema(entry);
    let validator = Validator::new(&schema);
    let mut files = Vec::new();
    json_files(&mods_dir().join("core/data"), &mut files);
    let mut sources = SourceMap::new();
    let mut checked = 0;
    for path in &files {
        let tree = parse(&mut sources, path);
        for definition in tree.as_array().unwrap_or(&[]) {
            if definition.get("type").and_then(Node::as_str) == Some("action") {
                let problems = validator.validate(definition);
                assert!(problems.is_empty(), "{}", render(&sources, &problems));
                checked += 1;
            } else if definition.get("type").and_then(Node::as_str) == Some("item") {
                assert!(!validator.validate(definition).is_empty());
            }
        }
    }
    assert!(checked > 5, "the bundled actions were checked");
}

#[test]
fn quantities_are_a_number_or_a_string_with_a_unit_of_their_dimension() {
    let check = |dim: Dim, text: &str| {
        super::regex::Regex::new(&quantity_pattern(dim))
            .expect("the pattern compiles")
            .is_match(text)
    };
    assert!(check(Dim::PRESSURE, "30 MPa"));
    assert!(!check(Dim::PRESSURE, "30 MPA"));
    assert!(!check(Dim::PRESSURE, "30 m"));
    assert!(check(Dim::TIME, "250 ms"));
    assert!(check(Dim::TEMPERATURE, "-5 C"));
    assert!(check(Dim::MASS.rate(), "0.012 kg/s"));
    assert!(check(Dim::MASS.rate(), "0.012 kg / h"));
    assert!(check(Dim::FREQUENCY, "0.15 /h"));
    assert!(check(Dim::POWER, "5 W"));
    assert!(check(Dim::POWER, "5 J/s"));
    assert!(check(Dim::SPEED, "18 km/h"));
    assert!(!check(Dim::SPEED, "18 km"));
}

// The definition types of the loader's own tests
// (`crates/clash-data/tests/common`), which its broken fixture is
// written for.
mod fixture_types {
    #![allow(dead_code)]

    use std::collections::BTreeMap;

    use clash_core::{Real, define_id};
    use clash_data::{
        Def, Feature, FromKind, Gated, Kind, KindValue, Length, Pressure, Ref, TagSet, Temperature,
        Text, by_kind,
    };
    use serde::Deserialize;
    use serde::de::{Deserializer, MapAccess};

    define_id! {
        pub struct DamageTypeId(u16);
        pub struct MaterialId(u16);
        pub struct BlockId(u16);
    }

    #[derive(Deserialize)]
    pub struct DamageTypeDef {
        #[serde(default)]
        pub name: Option<Text>,
    }

    impl Def for DamageTypeDef {
        type Id = DamageTypeId;
        const TYPE: &'static str = "damage_type";
    }

    fn one() -> Real {
        Real::ONE
    }

    #[derive(Deserialize)]
    pub struct Resist {
        #[serde(default)]
        pub threshold: Option<Pressure>,
        #[serde(default = "one")]
        pub factor: Real,
    }

    pub struct ColouredLight;

    impl Feature for ColouredLight {
        const NAME: &'static str = "coloured_light";
    }

    #[derive(Deserialize)]
    pub struct MaterialDef {
        pub density: Real,
        pub toughness: Real,
        #[serde(default)]
        pub resist: BTreeMap<Ref<DamageTypeDef>, Resist>,
        #[serde(default)]
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

    impl Def for MaterialDef {
        type Id = MaterialId;
        const TYPE: &'static str = "material";
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Flag {
        Opaque,
        Passable,
        LoadBearing,
        Anchor,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Shape {
        Full,
        Slope { rise: Length },
        Pillar(Length),
    }

    #[derive(Deserialize)]
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
    }

    /// The loader's test types.
    pub fn types() -> clash_data::DefTypes {
        let mut types = clash_data::DefTypes::new();
        types.register::<DamageTypeDef>();
        types.register::<MaterialDef>();
        types.register::<BlockDef>();
        types
    }
}

/// The loader's fixtures, by a path relative to this crate.
fn fixture(set: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../clash-data/tests/fixtures/{set}/mods"))
}

#[test]
fn the_schemas_refuse_the_loaders_broken_fixture() {
    let types = fixture_types::types();
    let entries = type_entries(&types);
    let mut sources = SourceMap::new();
    let tree = parse(
        &mut sources,
        &fixture("broken").join("broken/data/errors.json"),
    );
    let definitions = tree.as_array().expect("a list");
    // `--with-ids` lists what the files define; the broken mod does not
    // load, so its ids are those written in it.
    let mut ids = Ids::new();
    for definition in definitions {
        if let (Some(ty), Some(id)) = (
            definition.get("type").and_then(Node::as_str),
            definition.get("id").and_then(Node::as_str),
        ) {
            ids.entry(ty.to_owned()).or_default().push(id.to_owned());
            ids.entry(ty.to_owned())
                .or_default()
                .push(format!("broken:{id}"));
        }
    }
    let schema = Schemas::new(&entries, Some(&ids)).data_schema();
    let validator = Validator::new(&schema);

    // The file as a whole is refused.
    let problems = validator.validate(&tree);
    assert!(!problems.is_empty());

    // Each mistake within one definition is found, at its place.
    let found = |index: usize| {
        let problems = validator.validate(&definitions[index]);
        render(&sources, &problems)
    };
    let unknown = found(0);
    assert!(unknown.contains("unknown key \"thougness\""), "{unknown}");
    assert!(unknown.contains("missing field \"toughness\""), "{unknown}");
    assert!(unknown.contains("errors.json:4:"), "{unknown}");
    let unit = found(1);
    assert!(
        unit.contains("bearing: \"30 MPA\" is not of the form"),
        "{unit}"
    );
    let reference = found(2);
    assert!(
        reference.contains("material: \"ashh\" is not one of the values allowed here"),
        "{reference}"
    );
    let mismatch = found(6);
    assert!(
        mismatch.contains("fill: expected number, found string"),
        "{mismatch}"
    );

    // A cycle of inheritance and a duplicate id are mistakes of the
    // whole set, which no schema of one definition can see: the loader
    // reports them (E0405, E0305). Each definition alone is sound.
    for index in [3, 4, 5] {
        assert!(
            validator.validate(&definitions[index]).is_empty(),
            "{}",
            found(index)
        );
    }

    // And the loader agrees on every one the schema refuses.
    let outcome = clash_data::Loader::new(&types)
        .search_path(fixture("broken"))
        .load_all();
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    for code in ["E0202", "E0203", "E0204", "E0501", "E0201"] {
        assert!(codes.contains(&code), "{codes:?}");
    }
}

#[test]
fn the_schemas_accept_the_loaders_sound_fixtures() {
    let types = fixture_types::types();
    let entries = type_entries(&types);
    let schema = Schemas::new(&entries, None).data_schema();
    let validator = Validator::new(&schema);
    let mut files = Vec::new();
    json_files(&fixture("show"), &mut files);
    let mut sources = SourceMap::new();
    let mut checked = 0;
    for path in files
        .iter()
        .filter(|path| path.components().any(|c| c.as_os_str() == "data"))
    {
        let tree = parse(&mut sources, path);
        let problems = validator.validate(&tree);
        assert!(problems.is_empty(), "{}", render(&sources, &problems));
        checked += 1;
    }
    assert!(checked >= 3);
}

#[test]
fn schemas_are_written_for_every_type_and_a_data_file() {
    let dir = std::env::temp_dir().join(format!("clash-tools-schema-{}", std::process::id()));
    let types = crate::engine_types();
    let output = crate::data::run(
        &types,
        &[],
        "schema",
        &["--out".to_owned(), dir.to_string_lossy().into_owned()],
    )
    .expect("a data command");
    assert_eq!(output.code, crate::data::EXIT_OK, "{}", output.err);
    let written: Vec<String> = std::fs::read_dir(&dir)
        .expect("the directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(written.contains(&DATA_SCHEMA.to_owned()));
    for name in types.names() {
        assert!(written.contains(&format!("{name}.schema.json")), "{name}");
    }
    // One type alone, to standard output.
    let output = crate::data::run(
        &types,
        &[],
        "schema",
        &["--type".to_owned(), "material".to_owned()],
    )
    .expect("a data command");
    assert!(
        output
            .out
            .contains("\"title\": \"CLASH2D definition: material\""),
        "{}",
        output.out
    );
    let output = crate::data::run(
        &types,
        &[],
        "schema",
        &["--type".to_owned(), "materail".to_owned()],
    )
    .expect("a data command");
    assert!(
        output.err.contains("did you mean `material`?"),
        "{}",
        output.err
    );
}
