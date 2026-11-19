//! Making a mod and skeleton definitions (04 §13, 14 §9.1).
//!
//! `init-mod <id>` makes a mod that loads and lints clean as it is: a
//! `mod.json`, a profile that starts it with `core` and `ui`, a small
//! example of each common kind of definition under `data/`, each with a
//! comment, and editor settings that attach the data file's schema,
//! which is written beside them. `new <type> <id>` prints a definition
//! of a type with every field it requires, read from the type itself.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clash_data::DefTypes;
use clash_data::shape::{NamedShape, Shape, Shapes, VariantForm};

use crate::data::{DEFAULT_MODS_DIR, EXIT_FAILED, EXIT_OK, Options, Output};
use crate::reference::json::Json;
use crate::reference::{TypeEntry, type_entries, type_entry};

/// The files of a new mod, by their path inside its directory. `{id}`
/// stands for the mod's id.
const FILES: &[(&str, &str)] = &[
    (
        "mod.json",
        r#"{
  "id": "{id}",
  "name": "{id}",
  "version": "0.1.0",
  "engine": ">=0.1",
  "description": "What this mod adds. Made with `clash-tools init-mod`; every type, field and event is in docs/reference/.",
  "authors": [],
  "requires": ["core", "ui"]
}
"#,
    ),
    (
        "data/items.json",
        r#"[
  { "//": "An item: what it is made of, its mass and length, what of it can strike (a surface: a damage type over an area), and the channel slots needed to hold it. Every field is in docs/reference/types/item.md. Bare ids such as `wood` and `impact` are looked up in this mod, then in the mods it requires.",
    "type": "item", "id": "walking_stick", "name": "walking stick",
    "material": "wood", "mass": "0.6 kg", "length": "1 m",
    "surfaces": [
      { "id": "tip", "kind": "impact", "area": 3.0e-4, "give": 0.01, "span": 0.03 } ],
    "grip": { "arm": 1 },
    "tags": ["stick"],
    "glyph": { "ch": "/", "fg": "brown" } }
]
"#,
    ),
    (
        "data/creatures.json",
        r#"[
  { "//": "A creature template: the body it is built on, the reserves it has, its attributes and how it is drawn (06 §10). `core:quadruped` is core's four-legged body plan. Every field is in docs/reference/types/template.md.",
    "type": "template", "id": "goat", "kind": "species", "name": "Goat",
    "body": "quadruped",
    "pools": ["blood", "pain"],
    "attributes": { "strength": 5, "wits": 2 },
    "tags": ["animal"],
    "glyph": { "ch": "g", "fg": "white" } }
]
"#,
    ),
    (
        "data/actions.json",
        r#"[
  { "//": "An action: something a character does over time, in phases, holding a channel slot while it does (03 §6). The message is a text template read by whoever sees it. Every field is in docs/reference/types/action.md.",
    "type": "action", "id": "wave", "name": "Wave",
    "uses": { "arm": 1 },
    "phases": [
      { "id": "raise", "time": "400 ms", "cancel": true },
      { "id": "lower", "time": "300 ms",
        "do": [ { "message": "You wave." } ] }
    ] }
]
"#,
    ),
    (
        "data/rules.json",
        r#"[
  { "//": "A variable and a rule reacting to an event (05 §9): when anyone finishes waving, the count of their waves grows. `self` is the event's subject and `event` holds its fields. The events and what they carry are in docs/reference/events.md, the effects in docs/reference/effects.md.",
    "type": "variable", "id": "waves", "scope": "entity", "value": "int", "default": 0,
    "doc": "How many times a character has waved." },
  { "type": "eoc", "id": "count_waves",
    "on": "action_finished", "if": "event.action == '{id}:wave'",
    "doc": "Counts the waves of whoever waves.",
    "do": [ { "add": { "self.var.waves": 1 } } ] }
]
"#,
    ),
    (
        "data/screens.json",
        r#"[
  { "//": "A change to the interface: a line put into a slot the `ui` mod's play screen leaves for content (12 §10.6), shown while the character carries anything. Every field is in docs/reference/types/ui_part.md.",
    "type": "ui_part", "id": "burden", "into": "ui:play.bars", "order": 90, "required": false,
    "if": "(view.carried_mass ?? 0) > 0",
    "widget": { "text": "<dim>Carrying {view.carried_mass ?? 0:.1} kg</>" } }
]
"#,
    ),
    (
        ".vscode/settings.json",
        r#"{
  "//": "Attaches the schema of a data file, written by `clash-tools schema`, so that keys, values and units are checked as they are typed.",
  "json.schemas": [
    { "fileMatch": ["data/*.json", "data/**/*.json"], "url": "./.schemas/data.schema.json" }
  ]
}
"#,
    ),
];

/// `init-mod <id> [--dir <mods dir>]`: makes a mod.
pub(crate) fn init_mod_command(types: &DefTypes, options: &Options) -> Output {
    let [id] = options.positional.as_slice() else {
        return Output::usage("`init-mod` takes the id of the mod to make");
    };
    if !clash_data::is_id_part(id) {
        return Output {
            out: String::new(),
            err: format!(
                "clash-tools: `{id}` cannot be a mod's id: it starts with a lowercase letter and holds lowercase letters, digits and `_`\n"
            ),
            code: EXIT_FAILED,
        };
    }
    if id == clash_data::ENGINE_NAMESPACE || types.reserved_namespaces().contains(&id.as_str()) {
        return Output {
            out: String::new(),
            err: format!("clash-tools: `{id}` is reserved for the engine\n"),
            code: EXIT_FAILED,
        };
    }
    let mods = options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODS_DIR));
    let root = mods.join(id);
    if root.exists() {
        return Output {
            out: String::new(),
            err: format!("clash-tools: {} is there already\n", root.display()),
            code: EXIT_FAILED,
        };
    }
    match init_mod(types, &root, id) {
        Ok(written) => {
            let mut out = format!("made the mod `{id}` in {}:\n", root.display());
            for path in written {
                let _ = writeln!(out, "  {path}");
            }
            let _ = writeln!(
                out,
                "\nCheck it with `clash-tools lint {}`; start it with its profile `{id}`.",
                root.display()
            );
            Output {
                out,
                err: String::new(),
                code: EXIT_OK,
            }
        }
        Err(error) => Output {
            out: String::new(),
            err: format!("clash-tools: cannot make {}: {error}\n", root.display()),
            code: EXIT_FAILED,
        },
    }
}

/// Writes the files of a new mod with id `id` into `root`, which is made.
/// Gives the paths written, inside the mod.
pub fn init_mod(types: &DefTypes, root: &Path, id: &str) -> std::io::Result<Vec<String>> {
    let mut written = Vec::new();
    for (path, text) in FILES {
        let path = path.replace("{id}", id);
        let target = root.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, text.replace("{id}", id))?;
        written.push(path);
    }
    let path = format!("profiles/{id}.json");
    let target = root.join(&path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, profile(id).pretty())?;
    written.push(path);
    let entries = type_entries(types);
    let schema = crate::schema::Schemas::new(&entries, None).data_schema();
    let path = format!(".schemas/{}", crate::schema::DATA_SCHEMA);
    let target = root.join(&path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, schema.pretty())?;
    written.push(path);
    Ok(written)
}

/// The repository's default profile, which a new mod's profile extends.
const DEFAULT_PROFILE: &str = include_str!("../../../profiles/default.json");

/// The profile of a new mod: the repository's default profile, under the
/// mod's id, with the mod last in its list, so that it starts the game
/// the bundled mods make with the new one added (04 §3.3).
pub fn profile(id: &str) -> Json {
    let mut sources = clash_data::SourceMap::new();
    let file = sources.add("profiles/default.json", DEFAULT_PROFILE);
    let (tree, _) =
        clash_data::parse_json(file, sources.text(file), &clash_data::Limits::default());
    let mut profile = match tree.as_ref().map(from_node) {
        Some(Json::Object(members)) => members,
        _ => Vec::new(),
    };
    let mut has_mods = false;
    for (key, value) in &mut profile {
        match key.as_str() {
            "id" | "name" => *value = Json::str(id),
            "mods" => {
                has_mods = true;
                if let Json::Array(mods) = value {
                    mods.push(Json::str(id));
                }
            }
            _ => {}
        }
    }
    if profile.is_empty() {
        profile.push(("id".to_owned(), Json::str(id)));
        profile.push(("name".to_owned(), Json::str(id)));
    }
    if !has_mods {
        profile.push(("mods".to_owned(), Json::Array(vec![Json::str(id)])));
    }
    Json::Object(profile)
}

/// A parsed JSON value as one the tools write.
fn from_node(node: &clash_data::Node) -> Json {
    use clash_data::{NodeKind, Num};
    match &node.kind {
        NodeKind::Null => Json::Null,
        NodeKind::Bool(value) => Json::Bool(*value),
        NodeKind::Number(Num::Int(value)) => Json::Int(*value),
        NodeKind::Number(number) => Json::Real(number.as_f64()),
        NodeKind::Quantity { value, .. } => Json::Real(*value),
        NodeKind::String(text) => Json::str(text.as_str()),
        NodeKind::Array(items) => Json::Array(items.iter().map(from_node).collect()),
        NodeKind::Object(members) => Json::Object(
            members
                .iter()
                .map(|member| (member.key.clone(), from_node(&member.value)))
                .collect(),
        ),
    }
}

/// `new <type> <id>`: prints a skeleton definition.
pub(crate) fn new_command(types: &DefTypes, options: &Options) -> Output {
    let [type_name, id] = options.positional.as_slice() else {
        return Output::usage("`new` takes a definition type and an id");
    };
    let Some(entry) = type_entry(types, type_name) else {
        let mut err = format!("clash-tools: there is no definition type `{type_name}`\n");
        if let Some(near) = clash_data::diag::closest(type_name, types.names()) {
            let _ = writeln!(err, "did you mean `{near}`?");
        }
        return Output {
            out: String::new(),
            err,
            code: EXIT_FAILED,
        };
    };
    Output {
        out: skeleton(&entry, id).pretty(),
        err: String::new(),
        code: EXIT_OK,
    }
}

/// A definition of a type with its required fields, each with a value
/// to be replaced, and a comment that names the rest.
pub fn skeleton(entry: &TypeEntry, id: &str) -> Json {
    let mut members = Vec::new();
    let mut optional = Vec::new();
    let mut fields = Vec::new();
    if let Shape::Named(key) = &entry.shapes.root
        && let Some(named) = entry.shapes.named.get(key)
        && let NamedShape::Struct(shape) = &named.shape
    {
        for field in &shape.fields {
            if field.required {
                fields.push((
                    field.name.to_owned(),
                    placeholder(&entry.shapes, &field.shape, 0),
                ));
            } else {
                optional.push(format!("`{}`", field.name));
            }
        }
    }
    let mut comment = format!(
        "A new {}: every field it requires, each with a value to replace.",
        entry.name
    );
    if !optional.is_empty() {
        let _ = write!(comment, " It may also give {}.", optional.join(", "));
    }
    let _ = write!(comment, " See docs/reference/types/{}.md.", entry.name);
    members.push(("//".to_owned(), Json::str(comment)));
    members.push(("type".to_owned(), Json::str(entry.name)));
    members.push(("id".to_owned(), Json::str(id)));
    members.extend(fields);
    Json::Object(members)
}

/// A value of a shape to be replaced: the plainest one it takes.
fn placeholder(shapes: &Shapes, shape: &Shape, depth: usize) -> Json {
    if depth > 8 {
        return Json::Null;
    }
    match shape {
        Shape::Unknown | Shape::Any | Shape::Unit | Shape::Optional(_) => Json::Null,
        Shape::Bool => Json::Bool(false),
        Shape::Int { .. } => Json::Int(0),
        Shape::Real => Json::Real(0.0),
        Shape::Char => Json::str("?"),
        Shape::String | Shape::Tag => Json::str(""),
        Shape::Quantity(dim) => Json::str(format!("0 {}", dim.si_unit())),
        Shape::Duration => Json::str("0 s"),
        Shape::Ref(ty) | Shape::Inline(ty) => Json::str(format!("<id of a {ty}>")),
        // A list of objects shows one, with what each requires; any
        // other list starts empty.
        Shape::List(inner) => match inner.as_ref() {
            Shape::Named(key)
                if matches!(
                    shapes.named.get(key).map(|named| &named.shape),
                    Some(NamedShape::Struct(_))
                ) =>
            {
                Json::Array(vec![placeholder(shapes, inner, depth + 1)])
            }
            _ => Json::Array(Vec::new()),
        },
        Shape::Tuple(items) => Json::Array(
            items
                .iter()
                .map(|item| placeholder(shapes, item, depth + 1))
                .collect(),
        ),
        Shape::Map(..) => Json::Object(Vec::new()),
        Shape::Gated(_, inner) => placeholder(shapes, inner, depth + 1),
        Shape::Named(key) => {
            let Some(named) = shapes.named.get(key) else {
                return Json::Null;
            };
            match &named.shape {
                NamedShape::Struct(shape) => Json::Object(
                    shape
                        .fields
                        .iter()
                        .filter(|field| field.required)
                        .map(|field| {
                            (
                                field.name.to_owned(),
                                placeholder(shapes, &field.shape, depth + 1),
                            )
                        })
                        .collect(),
                ),
                NamedShape::Enum(shape) => match shape.variants.first() {
                    Some(variant) => match &variant.form {
                        VariantForm::Unit | VariantForm::Unknown => Json::str(variant.name),
                        VariantForm::Newtype(inner) => {
                            Json::object([(variant.name, placeholder(shapes, inner, depth + 1))])
                        }
                        VariantForm::Tuple(items) => Json::object([(
                            variant.name,
                            placeholder(shapes, &Shape::Tuple(items.clone()), depth + 1),
                        )]),
                        VariantForm::Struct(key) => Json::object([(
                            variant.name,
                            placeholder(shapes, &Shape::Named(key.clone()), depth + 1),
                        )]),
                    },
                    None => Json::Null,
                },
                NamedShape::Kinds(shape) => match shape.accepted.first() {
                    Some((kind, inner)) => match (kind, inner) {
                        (_, Shape::Any | Shape::Unknown) => match kind {
                            clash_data::shape::JsonKind::String => Json::str(""),
                            clash_data::shape::JsonKind::Object => Json::Object(Vec::new()),
                            clash_data::shape::JsonKind::Integer => Json::Int(0),
                            clash_data::shape::JsonKind::Number => Json::Real(0.0),
                            clash_data::shape::JsonKind::Bool => Json::Bool(false),
                            clash_data::shape::JsonKind::List => Json::Array(Vec::new()),
                            clash_data::shape::JsonKind::Null => Json::Null,
                        },
                        (_, inner) => placeholder(shapes, inner, depth + 1),
                    },
                    None => Json::Null,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of mods holding copies of the bundled mods and a new
    /// one, made with `init-mod`.
    fn scaffolded(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("clash-tools-init-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("a directory");
        base
    }

    fn run(words: &[&str]) -> Output {
        let args: Vec<String> = words[1..].iter().map(|word| (*word).to_owned()).collect();
        crate::data::run(
            &crate::engine_types(),
            crate::ENGINE_CHECKS,
            words[0],
            &args,
        )
        .expect("a data command")
    }

    #[test]
    fn a_new_mod_lints_clean_and_loads_in_its_profile() {
        let base = scaffolded("lint");
        let mods = base.join("mods");
        let output = run(&["init-mod", "my_mod", "--dir", &mods.to_string_lossy()]);
        assert_eq!(output.code, EXIT_OK, "{}", output.err);
        assert!(output.out.contains("data/items.json"), "{}", output.out);
        let root = mods.join("my_mod");
        for (path, _) in FILES {
            assert!(
                root.join(path.replace("{id}", "my_mod")).is_file(),
                "{path}"
            );
        }
        assert!(root.join(".schemas/data.schema.json").is_file());
        let profile =
            std::fs::read_to_string(root.join("profiles/my_mod.json")).expect("a profile");
        assert!(
            profile.contains("\"boot_screen\": \"ui:title\""),
            "{profile}"
        );
        assert!(profile.contains("\"decision.free\""), "{profile}");

        // The mod alone, with what it requires from the repository.
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods");
        let output = run(&[
            "lint",
            &root.to_string_lossy(),
            "--mods",
            &bundled.to_string_lossy(),
            "--mods",
            &mods.to_string_lossy(),
        ]);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(output.out.starts_with("ok: "), "{}", output.out);
        assert!(output.out.contains("my_mod"), "{}", output.out);

        // Its profile, with the bundled mods.
        let output = run(&[
            "lint",
            &root.join("profiles/my_mod.json").to_string_lossy(),
            "--mods",
            &bundled.to_string_lossy(),
            "--mods",
            &mods.to_string_lossy(),
        ]);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(
            output.out.contains("(core, ui, core_ui, my_mod)"),
            "{}",
            output.out
        );

        // Its data files are sound by the schema it was given.
        let text =
            std::fs::read_to_string(root.join(".schemas/data.schema.json")).expect("the schema");
        assert!(text.contains("\"$schema\""));
        let entries = &crate::reference::tests::catalog().types;
        let schema = crate::schema::Schemas::new(entries, None).data_schema();
        let validator = crate::schema::validate::Validator::new(&schema);
        let mut sources = clash_data::SourceMap::new();
        for (path, _) in FILES.iter().filter(|(path, _)| path.starts_with("data/")) {
            let text = std::fs::read_to_string(root.join(path)).expect("a data file");
            let file = sources.add(*path, &text);
            let (tree, problems) =
                clash_data::parse_json(file, sources.text(file), &clash_data::Limits::default());
            assert!(problems.is_empty(), "{path}: {problems:?}");
            let problems = validator.validate(&tree.expect("a tree"));
            assert!(problems.is_empty(), "{path}: {problems:?}");
        }

        // A second time it refuses, rather than write over the first.
        let again = run(&["init-mod", "my_mod", "--dir", &mods.to_string_lossy()]);
        assert_eq!(again.code, EXIT_FAILED);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_mod_id_must_be_one() {
        let base = scaffolded("ids");
        let dir = base.to_string_lossy().into_owned();
        assert_eq!(
            run(&["init-mod", "My-Mod", "--dir", &dir]).code,
            EXIT_FAILED
        );
        assert_eq!(
            run(&["init-mod", "engine", "--dir", &dir]).code,
            EXIT_FAILED
        );
        assert_eq!(run(&["init-mod"]).code, crate::data::EXIT_USAGE);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_skeleton_has_every_required_field_and_names_the_rest() {
        let output = run(&["new", "item", "lantern"]);
        assert_eq!(output.code, EXIT_OK, "{}", output.err);
        let text = output.out;
        assert!(text.contains("\"type\": \"item\""), "{text}");
        assert!(text.contains("\"id\": \"lantern\""), "{text}");
        assert!(
            text.contains("\"material\": \"<id of a material>\""),
            "{text}"
        );
        assert!(text.contains("\"mass\": \"0 kg\""), "{text}");
        assert!(text.contains("It may also give `name`"), "{text}");
        let wrong = run(&["new", "iten", "x"]);
        assert_eq!(wrong.code, EXIT_FAILED);
        assert!(wrong.err.contains("did you mean `item`?"), "{}", wrong.err);
        // Every type has one, and it is valid JSON.
        for entry in &crate::reference::tests::catalog().types {
            let text = skeleton(entry, "x").pretty();
            let mut sources = clash_data::SourceMap::new();
            let file = sources.add("new.json", &text);
            let (tree, problems) =
                clash_data::parse_json(file, sources.text(file), &clash_data::Limits::default());
            assert!(
                problems.is_empty() && tree.is_some(),
                "{}: {text}",
                entry.name
            );
        }
    }
}
