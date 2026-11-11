//! The data subcommands (04 §13): `lint`, `explain`, `show` and `schema`.
//!
//! Each command is a function from arguments to an exit code that writes
//! into strings, so that it can be tested without starting a process.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_data::{
    CODES, CodeInfo, DefTypes, Defs, Diagnostics, FinishedTree, LoadOutcome, Loader, Node,
    NodeKind, Severity,
};

/// The process exit code for success.
pub const EXIT_OK: u8 = 0;
/// The process exit code when the mods have errors, or what was asked
/// for does not exist.
pub const EXIT_FAILED: u8 = 1;
/// The process exit code when the command line was not understood.
pub const EXIT_USAGE: u8 = 2;

/// The directory mods are searched for in when none is named.
pub const DEFAULT_MODS_DIR: &str = "mods";

/// A check a crate runs over loaded definitions: what it finds wrong when
/// it builds its own tables from them, as located diagnostics
/// (04 §8.1, the build stage). Each crate that builds something from
/// definitions offers one, and `lint` runs them all after a load that
/// had no errors.
pub type BuildCheck = fn(&Arc<Defs>) -> Diagnostics;

/// What a command wrote and how it ended.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// What goes to standard output.
    pub out: String,
    /// What goes to standard error.
    pub err: String,
    /// The process exit code.
    pub code: u8,
}

impl Output {
    fn usage(message: impl Into<String>) -> Output {
        Output {
            out: String::new(),
            err: format!("clash-tools: {}\n\n{USAGE}", message.into()),
            code: EXIT_USAGE,
        }
    }
}

/// The usage text of the data subcommands.
pub const USAGE: &str = "\
data commands:
  lint <profile or dir> [--mods <dir>]... [--lenient] [--json]
        Run the whole loading pipeline, build what the engine builds from
        the definitions, and print every diagnostic.
        <dir> is a directory of mods or one mod's directory; a profile is
        a file, or a name looked for in profiles/. Exits 1 on errors.
  explain <code>
        Print the long explanation of a diagnostic code, such as E0203.
  show [<type>] <id> [--mods <dir>]... [--profile <file or name>]
        Print a finished definition and where each value came from.
  schema --out <dir>
        Write JSON Schemas for the definition types. Not built yet.
";

/// The options the data commands share.
#[derive(Debug, Default)]
struct Options {
    positional: Vec<String>,
    mods: Vec<PathBuf>,
    profile: Option<String>,
    lenient: bool,
    json: bool,
    /// What `lint` runs once the definitions are loaded.
    checks: Vec<BuildCheck>,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mods" => match args.next() {
                Some(dir) => options.mods.push(PathBuf::from(dir)),
                None => return Err("`--mods` needs a directory".to_owned()),
            },
            "--profile" => match args.next() {
                Some(profile) => options.profile = Some(profile.clone()),
                None => return Err("`--profile` needs a file or a name".to_owned()),
            },
            "--lenient" => options.lenient = true,
            "--json" => options.json = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            _ => options.positional.push(arg.clone()),
        }
    }
    Ok(options)
}

/// Runs a data subcommand. `command` is its name and `args` what follows
/// it. `checks` are run by `lint` over definitions that loaded. Returns
/// `None` if `command` is not one of the data subcommands.
pub fn run(
    types: &DefTypes,
    checks: &[BuildCheck],
    command: &str,
    args: &[String],
) -> Option<Output> {
    let run: fn(&DefTypes, &Options) -> Output = match command {
        "lint" => lint,
        "explain" => explain,
        "show" => show,
        "schema" => schema,
        _ => return None,
    };
    Some(match parse_options(args) {
        Ok(mut options) => {
            options.checks = checks.to_vec();
            run(types, &options)
        }
        Err(message) => Output::usage(message),
    })
}

fn loader<'t>(types: &'t DefTypes, options: &Options) -> Loader<'t> {
    let mut loader = Loader::new(types).lenient(options.lenient);
    for dir in &options.mods {
        loader = loader.search_path(dir.clone());
    }
    loader
}

/// Loads a profile given as a file or as a name. With no `--mods`, mods
/// are looked for beside the profile's own `profiles/` directory, or in
/// `mods`.
fn load_profile(types: &DefTypes, options: &Options, profile: &str) -> LoadOutcome {
    let path = Path::new(profile);
    let mut loader = loader(types, options);
    if path.is_file() {
        if options.mods.is_empty() {
            let beside = path
                .parent()
                .and_then(Path::parent)
                .map(|root| root.join(DEFAULT_MODS_DIR))
                .filter(|dir| dir.is_dir());
            loader = loader.search_path(beside.unwrap_or_else(|| PathBuf::from(DEFAULT_MODS_DIR)));
        }
        loader.load_profile_file(path)
    } else {
        if options.mods.is_empty() {
            loader = loader.search_path(DEFAULT_MODS_DIR);
        }
        loader.load_profile(profile)
    }
}

/// `lint <profile or dir>`: runs the pipeline, then each crate's build
/// check over what loaded, and prints every diagnostic. Non-zero exit on
/// errors.
fn lint(types: &DefTypes, options: &Options) -> Output {
    let [target] = options.positional.as_slice() else {
        return Output::usage("`lint` takes one profile or directory");
    };
    let path = Path::new(target);
    let outcome = if path.is_dir() {
        if path.join("mod.json").is_file() {
            loader(types, options).load_mod_dir(path)
        } else {
            loader(types, options).search_path(path).load_all()
        }
    } else {
        load_profile(types, options, target)
    };
    let loaded = !outcome.has_errors();
    let mut diagnostics = outcome.diagnostics;
    // What is built from the definitions is checked once they have all
    // loaded: a placeholder would only add mistakes of its own.
    if let Some(defs) = &outcome.defs
        && loaded
    {
        for check in &options.checks {
            diagnostics.extend(check(defs).iter().cloned());
        }
        diagnostics.sort();
    }
    let failed = diagnostics.has_errors() || outcome.defs.is_none();
    let mut out = String::new();
    if options.json {
        out.push_str(&diagnostics.to_json());
        out.push('\n');
    } else {
        out.push_str(&diagnostics.render());
        match &outcome.defs {
            Some(defs) if !failed => out.push_str(&summary(defs)),
            Some(defs) if loaded => {
                out.push_str(
                    "loaded, but what the engine builds from the definitions has errors\n",
                );
                out.push_str(&summary(defs));
            }
            Some(defs) => {
                out.push_str("loaded leniently, with placeholders for what is broken\n");
                out.push_str(&summary(defs));
            }
            None => out.push_str("not loaded\n"),
        }
    }
    Output {
        out,
        err: String::new(),
        code: if failed { EXIT_FAILED } else { EXIT_OK },
    }
}

fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

fn summary(defs: &Defs) -> String {
    let definitions: usize = defs
        .type_names()
        .iter()
        .filter_map(|name| defs.ids_of(name))
        .map(<[String]>::len)
        .sum();
    let mods: Vec<&str> = defs
        .mods()
        .iter()
        .map(|loaded| loaded.manifest.id.as_str())
        .collect();
    let mut text = format!(
        "ok: {} in {}",
        plural(definitions, "definition"),
        plural(mods.len(), "mod"),
    );
    if !mods.is_empty() {
        text.push_str(&format!(" ({})", mods.join(", ")));
    }
    text.push_str(&format!("; fingerprint {}\n", defs.fingerprint()));
    text
}

/// The explanation of a code, as `explain` prints it.
pub fn explanation(info: &CodeInfo) -> String {
    let severity = match info.code.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
    };
    format!(
        "{} ({severity}, {} stage)\n\n{}\n\n{}\n",
        info.code.id,
        info.stage,
        info.title,
        wrap(info.explanation, 76)
    )
}

/// Breaks text into lines of at most `width` characters at spaces.
fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = 0;
    for word in text.split_whitespace() {
        let length = word.chars().count();
        if line > 0 && line + 1 + length > width {
            out.push('\n');
            line = 0;
        } else if line > 0 {
            out.push(' ');
            line += 1;
        }
        out.push_str(word);
        line += length;
    }
    out
}

/// Every code `explain` knows: this is where a crate that adds a
/// pipeline stage adds its table.
fn all_codes() -> Vec<&'static CodeInfo> {
    CODES.iter().chain(clash_data::expr::CODES).collect()
}

/// `explain <code>`: prints the long explanation of a diagnostic.
fn explain(_types: &DefTypes, options: &Options) -> Output {
    let [code] = options.positional.as_slice() else {
        return Output::usage("`explain` takes one diagnostic code, such as E0203");
    };
    let codes = all_codes();
    match codes
        .iter()
        .find(|info| info.code.id.eq_ignore_ascii_case(code))
    {
        Some(info) => Output {
            out: explanation(info),
            err: String::new(),
            code: EXIT_OK,
        },
        None => {
            let upper = code.to_ascii_uppercase();
            let near = clash_data::diag::closest(&upper, codes.iter().map(|info| info.code.id));
            let mut err = format!("clash-tools: there is no diagnostic code `{code}`\n");
            if let Some(near) = near {
                err.push_str(&format!("did you mean `{near}`?\n"));
            }
            Output {
                out: String::new(),
                err,
                code: EXIT_FAILED,
            }
        }
    }
}

/// `show [<type>] <id>`: prints a finished definition with the origin of
/// each value.
fn show(types: &DefTypes, options: &Options) -> Output {
    let (type_name, id) = match options.positional.as_slice() {
        [id] => (None, id.as_str()),
        [type_name, id] => (Some(type_name.as_str()), id.as_str()),
        _ => return Output::usage("`show` takes a definition type and an id, or an id alone"),
    };
    let outcome = match &options.profile {
        Some(profile) => load_profile(types, options, profile),
        None => {
            let mut loader = loader(types, options);
            if options.mods.is_empty() {
                loader = loader.search_path(DEFAULT_MODS_DIR);
            }
            loader.load_all()
        }
    };
    let Some(defs) = &outcome.defs else {
        return Output {
            out: outcome.diagnostics.render(),
            err: "clash-tools: the mods did not load, so there is nothing to show\n".to_owned(),
            code: EXIT_FAILED,
        };
    };
    let found: Vec<&FinishedTree> = defs
        .trees()
        .iter()
        .filter(|tree| tree.id == id && type_name.is_none_or(|name| tree.type_name == name))
        .collect();
    if found.is_empty() {
        let what = type_name.unwrap_or("definition");
        let mut err = format!("clash-tools: there is no {what} `{id}`\n");
        let ids = defs
            .trees()
            .iter()
            .filter(|tree| type_name.is_none_or(|name| tree.type_name == name))
            .map(|tree| tree.id.as_str());
        if let Some(near) = clash_data::diag::closest(id, ids) {
            err.push_str(&format!("did you mean `{near}`?\n"));
        } else if !id.contains(':') {
            err.push_str("an id is written in full: `namespace:name`\n");
        }
        return Output {
            out: String::new(),
            err,
            code: EXIT_FAILED,
        };
    }
    let mut out = String::new();
    for (index, tree) in found.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&show_tree(defs, tree));
    }
    Output {
        out,
        err: String::new(),
        code: EXIT_OK,
    }
}

/// One line of a shown definition: the JSON, and where the value on it
/// was last written.
struct ShownLine {
    text: String,
    origin: Option<String>,
}

/// A finished definition as indented JSON, each value followed by the
/// file and line that set it and the mod that file belongs to.
pub fn show_tree(defs: &Defs, tree: &FinishedTree) -> String {
    let mut lines = Vec::new();
    show_node(defs, &tree.tree, 0, None, false, &mut lines);
    let width = lines
        .iter()
        .filter(|line| line.origin.is_some())
        .map(|line| line.text.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = format!("{} {}", tree.type_name, tree.id);
    if tree.is_abstract {
        out.push_str("  (abstract)");
    }
    if tree.inline {
        out.push_str("  (written in place)");
    }
    out.push('\n');
    for line in lines {
        match line.origin {
            Some(origin) => {
                let pad = width - line.text.chars().count();
                out.push_str(&format!("{}{}  <- {origin}\n", line.text, " ".repeat(pad)));
            }
            None => {
                out.push_str(&line.text);
                out.push('\n');
            }
        }
    }
    out
}

/// Where a value was last written: `file:line (mod)`.
fn origin(defs: &Defs, node: &Node) -> String {
    let location = defs.sources().locate(node.span);
    let owner = defs
        .mods()
        .iter()
        .find(|loaded| {
            location
                .file
                .strip_prefix(loaded.root.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .map_or("engine", |loaded| loaded.manifest.id.as_str());
    format!("{}:{} ({owner})", location.file, location.line)
}

fn quoted(text: &str) -> String {
    Node::new(
        NodeKind::String(text.to_owned()),
        clash_data::Span::new(clash_data::FileId(0), 0, 0),
    )
    .to_json()
}

fn show_node(
    defs: &Defs,
    node: &Node,
    depth: usize,
    key: Option<&str>,
    comma: bool,
    lines: &mut Vec<ShownLine>,
) {
    let indent = "  ".repeat(depth);
    let prefix = match key {
        Some(key) => format!("{indent}{}: ", quoted(key)),
        None => indent.clone(),
    };
    let tail = if comma { "," } else { "" };
    match &node.kind {
        NodeKind::Object(members) if !members.is_empty() => {
            lines.push(ShownLine {
                text: format!("{prefix}{{"),
                origin: None,
            });
            for (index, member) in members.iter().enumerate() {
                show_node(
                    defs,
                    &member.value,
                    depth + 1,
                    Some(&member.key),
                    index + 1 < members.len(),
                    lines,
                );
            }
            lines.push(ShownLine {
                text: format!("{indent}}}{tail}"),
                origin: None,
            });
        }
        NodeKind::Array(items) if !items.is_empty() => {
            lines.push(ShownLine {
                text: format!("{prefix}["),
                origin: None,
            });
            for (index, item) in items.iter().enumerate() {
                show_node(defs, item, depth + 1, None, index + 1 < items.len(), lines);
            }
            lines.push(ShownLine {
                text: format!("{indent}]{tail}"),
                origin: None,
            });
        }
        _ => lines.push(ShownLine {
            text: format!("{prefix}{}{tail}", node.to_json()),
            origin: Some(origin(defs, node)),
        }),
    }
}

/// `schema --out <dir>`: not built yet.
fn schema(_types: &DefTypes, _options: &Options) -> Output {
    Output {
        out: String::new(),
        err: "clash-tools: `schema` is not built yet. JSON Schemas are generated by package P5 \
              of docs/plans/sim-kernel.md.\n"
            .to_owned(),
        code: EXIT_USAGE,
    }
}

#[cfg(test)]
mod tests {
    use clash_core::{Real, define_id};
    use clash_data::{Def, TagSet};

    use super::*;

    define_id! {
        /// A test material.
        struct MaterialId(u16);
    }

    /// A stand-in for a definition type another crate will register.
    #[derive(Debug, serde::Deserialize)]
    struct MaterialDef {
        #[allow(dead_code)]
        density: Real,
        #[allow(dead_code)]
        toughness: Real,
        #[serde(default)]
        #[allow(dead_code)]
        span: Option<u32>,
        #[serde(default)]
        #[allow(dead_code)]
        tags: TagSet,
    }

    impl Def for MaterialDef {
        type Id = MaterialId;
        const TYPE: &'static str = "material";
    }

    fn types() -> DefTypes {
        let mut types = DefTypes::new();
        types.register::<MaterialDef>();
        types
    }

    /// The fixtures of `clash-data`, by a path relative to this crate.
    fn fixture(set: &str) -> String {
        format!("../clash-data/tests/fixtures/{set}/mods")
    }

    fn run_with(types: &DefTypes, words: &[&str]) -> Output {
        let args: Vec<String> = words[1..].iter().map(|word| (*word).to_owned()).collect();
        run(types, &[], words[0], &args).expect("a data command")
    }

    /// Runs a command with the engine's own types and build checks, as
    /// the binary does.
    fn run_engine(words: &[&str]) -> Output {
        let args: Vec<String> = words[1..].iter().map(|word| (*word).to_owned()).collect();
        run(
            &crate::engine_types(),
            crate::ENGINE_CHECKS,
            words[0],
            &args,
        )
        .expect("a data command")
    }

    #[test]
    fn lint_passes_on_the_repository_as_it_stands() {
        // Every mod in the directory, then the default profile, then each
        // bundled mod by itself: no errors and no warnings.
        for target in [
            "../../mods",
            "../../profiles/default.json",
            "../../mods/core",
        ] {
            let output = run_engine(&["lint", target]);
            assert_eq!(
                output.code, EXIT_OK,
                "{target}:\n{}{}",
                output.out, output.err
            );
            assert!(output.out.starts_with("ok: "), "{target}:\n{}", output.out);
        }
        let output = run_engine(&["lint", "../../mods"]);
        assert!(output.out.contains("(core, ui)"), "{}", output.out);
        let output = run_engine(&["lint", "../../mods/ui"]);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(output.out.starts_with("ok: "), "{}", output.out);
    }

    #[test]
    fn show_prints_a_bundled_definition_with_its_origins() {
        let output = run_engine(&["show", "block", "core:wall", "--mods", "../../mods"]);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(
            output.out.starts_with("block core:wall\n"),
            "{}",
            output.out
        );
        assert!(
            output
                .out
                .contains("<- ../../mods/core/data/world/blocks.json:"),
            "{}",
            output.out
        );
        assert!(output.out.contains("(core)"), "{}", output.out);
        // A screen of the interface mod is shown the same way.
        let output = run_engine(&["show", "theme", "ui:default", "--mods", "../../mods"]);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(output.out.contains("(ui)"), "{}", output.out);
    }

    #[test]
    fn lint_reports_what_only_shows_when_the_engine_builds_its_tables() {
        // A mod that loads, and whose block names a material that does
        // not exist: found when the world's tables are built.
        let dir = std::env::temp_dir().join(format!("clash-tools-lint-{}", std::process::id()));
        let data = dir.join("mods/broken/data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(dir.join("mods/broken/mod.json"), r#"{ "id": "broken" }"#).unwrap();
        std::fs::write(
            data.join("blocks.json"),
            "[\n  { \"type\": \"block\", \"id\": \"gate\", \"height\": 3.0, \"fill\": 0.1,\n    \"material\": \"unobtainium\" }\n]\n",
        )
        .unwrap();
        let mods = dir.join("mods");
        let output = run_engine(&["lint", &mods.to_string_lossy()]);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(output.code, EXIT_FAILED, "{}", output.out);
        assert!(
            output.out.contains("there is no material `unobtainium`"),
            "{}",
            output.out
        );
        assert!(
            output.out.contains("broken/data/blocks.json:3:17"),
            "{}",
            output.out
        );
        assert!(
            output.out.contains("= in: block broken:gate, at material"),
            "{}",
            output.out
        );
        assert!(
            output
                .out
                .contains("loaded, but what the engine builds from the definitions has errors"),
            "{}",
            output.out
        );
    }

    #[test]
    fn lint_prints_every_diagnostic_and_fails_on_errors() {
        // The show fixture is three sound mods of one type.
        let output = run_with(&types(), &["lint", &fixture("show")]);
        assert_eq!(output.code, EXIT_OK, "{}", output.out);
        assert!(
            output
                .out
                .starts_with("ok: 2 definitions in 3 mods (core, ash, tweak); fingerprint "),
            "{}",
            output.out
        );

        // One mod's directory: the mod and what it requires.
        let one = format!("{}/ash", fixture("show"));
        let output = run_with(&types(), &["lint", &one]);
        assert_eq!(output.code, EXIT_OK, "{}", output.out);
        assert!(
            output.out.contains("in 2 mods (core, ash)"),
            "{}",
            output.out
        );

        // With no types registered every definition is of an unknown
        // type: errors, and a failing exit code.
        let output = run_with(&DefTypes::new(), &["lint", &fixture("show")]);
        assert_eq!(output.code, EXIT_FAILED);
        assert!(
            output
                .out
                .contains("error[E0303]: unknown definition type \"material\"")
        );
        assert!(output.out.ends_with("not loaded\n"), "{}", output.out);
    }

    #[test]
    fn lint_reads_a_profile_by_file() {
        let profile = "../clash-data/tests/fixtures/show/profiles/pair.json";
        let output = run_with(&types(), &["lint", profile]);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(
            output.out.contains("in 2 mods (core, ash)"),
            "{}",
            output.out
        );
        // And by name, given where the mods are.
        let output = run_with(&types(), &["lint", "pair", "--mods", &fixture("show")]);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        let output = run_with(&types(), &["lint", "nonesuch", "--mods", &fixture("show")]);
        assert_eq!(output.code, EXIT_FAILED);
        assert!(output.out.contains("error[E0008]"), "{}", output.out);
    }

    #[test]
    fn lint_writes_json_for_machines() {
        let output = run_with(&DefTypes::new(), &["lint", &fixture("show"), "--json"]);
        assert_eq!(output.code, EXIT_FAILED);
        let back = clash_data::Diagnostics::from_json(&output.out).expect("JSON output");
        assert!(back.has_errors());
        assert!(back.iter().all(|diagnostic| diagnostic.code == "E0303"));
        let clean = run_with(&types(), &["lint", &fixture("show"), "--json"]);
        assert_eq!(clean.out, "[]\n");
        assert_eq!(clean.code, EXIT_OK);
    }

    #[test]
    fn lenient_lint_still_fails_and_says_what_it_did() {
        let output = run_with(&DefTypes::new(), &["lint", &fixture("show"), "--lenient"]);
        assert_eq!(output.code, EXIT_FAILED);
        assert!(output.out.contains("loaded leniently"), "{}", output.out);
    }

    #[test]
    fn explain_prints_the_long_explanation_of_every_code() {
        let output = run_with(&types(), &["explain", "E0203"]);
        assert_eq!(output.code, EXIT_OK);
        assert!(
            output
                .out
                .starts_with("E0203 (error, typed load stage)\n\n")
        );
        assert!(
            output
                .out
                .contains("A key is not one the definition type knows.")
        );
        assert!(output.out.contains("Unknown keys are errors"));
        assert!(output.out.lines().all(|line| line.chars().count() <= 76));
        // Letter case does not matter.
        assert_eq!(run_with(&types(), &["explain", "e0203"]), output);
        // Every code has an explanation to print.
        for info in all_codes() {
            let output = run_with(&types(), &["explain", info.code.id]);
            assert_eq!(output.code, EXIT_OK, "{}", info.code.id);
            assert!(output.out.len() > 100, "{}", info.code.id);
        }
        let unknown = run_with(&types(), &["explain", "E0204x"]);
        assert_eq!(unknown.code, EXIT_FAILED);
        assert!(unknown.err.contains("there is no diagnostic code `E0204x`"));
        assert!(unknown.err.contains("did you mean `E0204`?"));
    }

    #[test]
    fn show_attributes_every_value_to_the_right_file_and_line() {
        let output = run_with(
            &types(),
            &["show", "material", "ash:cinder", "--mods", &fixture("show")],
        );
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        let expected = "\
material ash:cinder
{
  \"density\": 1300.0,      <- ../clash-data/tests/fixtures/show/mods/tweak/data/tweak.json:5 (tweak)
  \"toughness\": 900000.0,  <- ../clash-data/tests/fixtures/show/mods/ash/data/ash.json:3 (ash)
  \"tags\": [
    \"mineral\",            <- ../clash-data/tests/fixtures/show/mods/core/data/stone.json:5 (core)
    \"burnt\"               <- ../clash-data/tests/fixtures/show/mods/ash/data/ash.json:4 (ash)
  ],
  \"span\": 3               <- ../clash-data/tests/fixtures/show/mods/tweak/data/tweak.json:4 (tweak)
}
";
        assert_eq!(output.out, expected);

        // By id alone, and an abstract parent can be shown too.
        let output = run_with(&types(), &["show", "core:base", "--mods", &fixture("show")]);
        assert_eq!(output.code, EXIT_OK);
        assert!(output.out.starts_with("material core:base  (abstract)\n"));
        assert!(output.out.contains("stone.json:3 (core)"), "{}", output.out);
    }

    #[test]
    fn show_says_what_it_cannot_find() {
        let output = run_with(
            &types(),
            &["show", "material", "ash:cindre", "--mods", &fixture("show")],
        );
        assert_eq!(output.code, EXIT_FAILED);
        assert_eq!(
            output.err,
            "clash-tools: there is no material `ash:cindre`\ndid you mean `ash:cinder`?\n"
        );
        let output = run_with(&types(), &["show", "cinder", "--mods", &fixture("show")]);
        assert!(
            output.err.contains("an id is written in full"),
            "{}",
            output.err
        );
        // Mods that do not load show their diagnostics.
        let output = run_with(
            &DefTypes::new(),
            &["show", "ash:cinder", "--mods", &fixture("show")],
        );
        assert_eq!(output.code, EXIT_FAILED);
        assert!(output.out.contains("error[E0303]"));
    }

    #[test]
    fn mistakes_on_the_command_line_show_the_usage() {
        for words in [
            &["lint"][..],
            &["lint", "a", "b"],
            &["lint", "a", "--frobnicate"],
            &["lint", "a", "--mods"],
            &["explain"],
            &["show"],
            &["show", "a", "b", "c"],
        ] {
            let output = run_with(&types(), words);
            assert_eq!(output.code, EXIT_USAGE, "{words:?}");
            assert!(output.err.contains("data commands:"), "{words:?}");
        }
        assert!(run(&types(), &[], "worldview", &[]).is_none());
        let schema = run_with(&types(), &["schema", "--json"]);
        assert_eq!(schema.code, EXIT_USAGE);
        assert!(schema.err.contains("not built yet"));
    }

    #[test]
    fn text_wraps_at_spaces() {
        assert_eq!(wrap("one two three four", 9), "one two\nthree\nfour");
        assert_eq!(wrap("  spaced   out ", 80), "spaced out");
        assert_eq!(wrap("unbreakable", 4), "unbreakable");
    }
}
