//! The console and the test-script runner from outside: properties of
//! the language, the "Done when" items of tooling packages TL1 and TL6
//! that can be shown without a simulation, and the scripts under
//! `scenarios/console`.

use std::path::{Path, PathBuf};

use clash_core::Real;
use clash_expr::{KindSpec, Registry, Type, Value, VerbSpec};
use clash_sim::console::testing::FakeHost;
use clash_sim::console::{
    CommandSource, CommandTable, ConsoleHost, Expression, HostKind, Mode, ModeOutcome, Runner,
    Session, parse_line, parse_script, print_line, read_scripts, run_dir,
};
use proptest::prelude::*;

/// The directory of the scripts that use only the fake host.
fn scenarios() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/console")
}

fn fresh() -> Box<dyn ConsoleHost> {
    Box::new(FakeHost::new())
}

// ---- TL1 ---------------------------------------------------------------

/// A registry with verbs of every shape: no arguments, a direct
/// argument, optional arguments, subjects, ids, numbers and texts.
fn verbs() -> Registry {
    let mut registry = Registry::with_builtins();
    let thing = registry.add_kind(KindSpec::new("thing")).unwrap();
    let material = registry.add_def_type("material", "A material.").unwrap();
    for verb in [
        VerbSpec::new("damage")
            .doc("Damages what is at a place.")
            .arg("at", Type::Subject(thing), "Where.")
            .arg("energy", Type::Real, "How much, in joules.")
            .optional_arg("kind", Type::Id(material), "The kind of damage."),
        VerbSpec::new("message")
            .doc("Puts a line in a log.")
            .arg("text", Type::Str, "The line.")
            .optional_arg("to", Type::Subject(thing), "Whose log.")
            .direct("text"),
        VerbSpec::new("shove")
            .doc("Pushes the subject.")
            .arg("force", Type::Real, "How hard, in newtons.")
            .direct("force"),
        VerbSpec::new("stop").doc("Ends the rule."),
        VerbSpec::new("teleport")
            .doc("Moves something.")
            .arg("who", Type::Subject(thing), "What to move.")
            .arg("to", Type::Subject(thing), "Where to."),
    ] {
        registry.add_verb(verb).unwrap();
    }
    registry
}

#[test]
fn every_registered_verb_has_a_console_command_without_further_code() {
    let registry = verbs();
    let table = FakeHost::table_with_verbs(&registry);
    // One command for each verb, with the verb's arguments as its
    // parameters and its documentation as its help.
    for verb in registry.verbs() {
        let spec = table
            .get(&verb.spec.name)
            .unwrap_or_else(|| panic!("no command for the verb `{}`", verb.spec.name));
        assert_eq!(spec.source, CommandSource::Verb);
        assert!(spec.changes, "a verb changes the simulation");
        let help = table.help(&verb.spec.name).unwrap();
        assert!(help.contains(&verb.spec.doc), "{help}");
        for arg in &verb.spec.args {
            assert!(spec.params.iter().any(|param| param.name == arg.name));
            assert!(help.contains(&arg.doc), "{help}");
        }
        if let Some(direct) = &verb.spec.direct {
            assert_eq!(&spec.params[0].name, direct);
        }
    }

    // And each can be called.
    let mut host = FakeHost::new();
    let mut session = Session::new(HostKind::Game);
    for line in [
        "new a as=a",
        "new b as=b",
        "damage at @a energy=5kJ kind=blast",
        "message \"The door creaks.\" to=@b",
        "shove 2kN",
        "stop",
        "as @a teleport who=self to=@b",
    ] {
        session
            .run_line(&table, &mut host, line)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    assert_eq!(host.calls(), 5);
    let a = session.name("a").unwrap().clone();
    let b = session.name("b").unwrap().clone();
    let calls: Vec<_> = session
        .records()
        .iter()
        .filter(|record| record.name == "verb")
        .collect();
    let names: Vec<_> = calls
        .iter()
        .map(|call| call.field("name").unwrap().to_string())
        .collect();
    assert_eq!(names, ["damage", "message", "shove", "stop", "teleport"]);
    assert_eq!(calls[0].field("at"), Some(&a));
    assert_eq!(
        calls[0].field("energy"),
        Some(&Value::Real(Real::of(5000.0)))
    );
    assert_eq!(calls[0].field("kind"), Some(&Value::text("blast")));
    assert_eq!(
        calls[1].field("text"),
        Some(&Value::text("The door creaks."))
    );
    assert_eq!(calls[1].field("to"), Some(&b));
    assert_eq!(
        calls[2].field("force"),
        Some(&Value::Real(Real::of(2000.0)))
    );
    assert_eq!(calls[3].fields.len(), 1);
    assert_eq!(calls[4].field("who"), Some(&a));
    assert_eq!(calls[4].field("to"), Some(&b));

    // A verb registered later is a command with no change here.
    let mut later = verbs();
    later
        .add_verb(VerbSpec::new("ignite").arg("at", Type::Any, "Where."))
        .unwrap();
    let table = FakeHost::table_with_verbs(&later);
    session.run_line(&table, &mut host, "ignite @a").unwrap();
    assert_eq!(host.calls(), 6);
}

#[test]
fn a_console_session_saved_as_a_file_replays_to_the_same_hash() {
    let table = FakeHost::table_with_verbs(&verbs());
    let mut host = FakeHost::new();
    let mut session = Session::new(HostKind::Game);
    for line in [
        "use fake   seed=11",
        "gen flat size=3",
        "new a value=2 as=a   # the counter",
        "timer @a every=100ms times=30",
        "advance 1.5s",
        "lock @c:1,0,0",
        "set @c:1,0,0 4",
        "frobnicate",
        "until \"@a.value >= 20\" max=1min",
        "as @a shove 3kN",
        "do { \"emit\": { \"event\": \"x\" } }",
        "save here",
        "add @a 1000",
        "load here",
        "focus @a",
        "eval self.value + cells()",
    ] {
        // Some of these are refused or unknown, as a person's typing is.
        let _ = session.run_line(&table, &mut host, line);
    }
    let hash = host.state_hash();
    let file = format!("{}expect hash {hash}\n", session.transcript());
    assert!(
        file.contains("set @c:1,0,0 4\nexpect reason cell_locked\n"),
        "{file}"
    );
    assert!(!file.contains("frobnicate"), "{file}");

    // The saved file is a test script that passes in every mode, which
    // includes giving the same hash.
    let script = parse_script(&file).unwrap_or_else(|errors| panic!("{errors:?}\n{file}"));
    let result = Runner::new(&table, &fresh).run(&script);
    assert!(result.passed(), "{result:?}\n{file}");
}

// ---- TL6 ---------------------------------------------------------------

#[test]
fn the_scripts_under_scenarios_pass_in_all_four_modes() {
    let table = FakeHost::table();
    let dir = scenarios();
    if std::env::var_os("BLESS").is_some() {
        for (path, text) in read_scripts(&dir).unwrap() {
            let folder = Path::new(&path).parent().unwrap().to_path_buf();
            let beside = |file: &str| std::fs::read_to_string(folder.join(file)).ok();
            let blessed = Runner::new(&table, &fresh)
                .files(&beside)
                .bless(&text)
                .unwrap_or_else(|error| panic!("{path}: {error:?}"));
            std::fs::write(&path, &blessed.text).unwrap();
            for (file, frame) in &blessed.frames {
                std::fs::write(folder.join(file), frame).unwrap();
            }
        }
    }
    let report = run_dir(&dir, &table, &fresh).unwrap();
    assert!(
        !report.scripts.is_empty(),
        "no scripts under {}",
        dir.display()
    );
    assert!(report.passed(), "{}", report.render());
    for script in &report.scripts {
        assert_eq!(script.path.rsplit('.').next(), Some("scn"));
    }
    assert!(report.render().ends_with("passed, 0 failed\n"));
}

#[test]
fn the_same_scripts_fail_under_save_and_load_on_a_host_that_loses_state() {
    use clash_sim::console::testing::Flaws;
    let table = FakeHost::table();
    let flawed = || -> Box<dyn ConsoleHost> {
        Box::new(FakeHost::with_flaws(Flaws {
            save_forgets_timers: true,
            ..Flaws::default()
        }))
    };
    let report = run_dir(&scenarios(), &table, &flawed).unwrap();
    assert!(!report.passed());
    let text = report.render();
    assert!(text.contains("  straight      passed\n"), "{text}");
    assert!(text.contains("  save-and-load FAILED at line"), "{text}");
    assert!(text.contains("(split after line "), "{text}");
    let clash_sim::console::ScriptOutcome::Ran(result) = &report.scripts[0].outcome else {
        panic!("{text}")
    };
    assert_eq!(result.outcome(Mode::Twice), &ModeOutcome::Passed);
    assert_eq!(result.outcome(Mode::Threads), &ModeOutcome::Passed);
}

// ---- The language ------------------------------------------------------

/// A value as it might be typed, in each of its forms.
fn value() -> impl Strategy<Value = String> {
    let fixed = |text: &'static str| Just(text.to_owned());
    prop_oneof![
        // Numbers, with and without units.
        any::<i32>().prop_map(|number| number.to_string()),
        (any::<i16>(), 0_u32..1000).prop_map(|(whole, fraction)| format!("{whole}.{fraction}")),
        (1_u32..999, -30_i32..30).prop_map(|(digits, exponent)| format!("{digits}e{exponent}")),
        (
            0_u32..5000,
            prop::sample::select(vec!["us", "ms", "s", "min", "kg", "kPa", "MJ", "m", "K"])
        )
            .prop_map(|(number, unit)| format!("{number}{unit}")),
        // Words and ids.
        "[a-z_][a-z0-9_]{0,8}",
        "[a-z]{1,5}:[a-z_]{1,6}",
        // References.
        fixed("@here"),
        fixed("@self"),
        fixed("@sel"),
        (any::<i16>(), any::<i16>(), -3_i32..9).prop_map(|(x, y, z)| format!("@c:{x},{y},{z}")),
        (0_u32..500).prop_map(|slot| format!("@e:{slot}")),
        (0_u32..500, 0_u32..9).prop_map(|(slot, generation)| format!("@e:{slot}v{generation}")),
        "@[a-z]{2,6}:[a-z_]{1,6}",
        "@[a-z][a-z0-9_]{0,6}",
        // Quoted, with anything printable inside.
        "[ -~]{0,12}"
            .prop_map(|text| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))),
        // JSON.
        fixed("{}"),
        fixed("{ \"a\": 1 }"),
        fixed("{\"k\": {\"n\": [1, 2.5, -3e7, \"}\"], \"t\": true, \"z\": null}}"),
        fixed("{ \"add_trait\": { \"to\": \"self\", \"trait\": \"adrenaline\" } }"),
        // Raw text.
        fixed("self.pool.blood.frac"),
        fixed("dist(self, @sel)"),
        fixed("'two  words'"),
        fixed("a.b+1"),
        fixed("9c41f0aa"),
        fixed("frames/menu.txt"),
        fixed("=="),
        fixed("xs[0 + 1]"),
        fixed("@a.value"),
        fixed("2sec"),
    ]
}

/// A whole line: an optional `as`, a command, arguments, a comment,
/// with uneven spacing.
fn line() -> impl Strategy<Value = String> {
    let argument =
        (prop::option::of("[a-z_][a-z0-9_]{0,6}"), value()).prop_map(|(key, value)| match key {
            Some(key) => format!("{key}={value}"),
            None => value,
        });
    let gap = prop::sample::select(vec![" ", "  ", "\t", " \t "]);
    (
        prop::option::of(prop::sample::select(vec![
            "@g1",
            "@self",
            "@c:1,2,3",
            "@site:mill",
        ])),
        prop::sample::select(vec!["spawn", "act", "eval", "core:verb", "x", "trace"]),
        prop::collection::vec((argument, gap.clone()), 0..6),
        prop::option::of("[ -~]{0,10}"),
        gap,
    )
        .prop_map(|(subject, command, arguments, comment, gap)| {
            let mut out = String::from(gap);
            if let Some(subject) = subject {
                out.push_str(&format!("as{gap}{subject}{gap}"));
            }
            out.push_str(command);
            for (argument, gap) in arguments {
                out.push_str(gap);
                out.push_str(&argument);
            }
            if let Some(comment) = comment {
                out.push_str(&format!("{gap}#{comment}"));
            }
            out
        })
}

/// Checks that printing and parsing again gives the same tree, and
/// that the printed form is a fixed point.
fn round_trips(source: &str) -> Result<(), TestCaseError> {
    let tree = match parse_line(source) {
        Ok(tree) => tree,
        Err(error) => {
            // An error says something, and can be shown.
            prop_assert!(!error.message.is_empty());
            prop_assert!(error.render(source).contains('^'));
            return Ok(());
        }
    };
    let printed = print_line(&tree);
    let again = parse_line(&printed);
    prop_assert!(
        again.is_ok(),
        "`{}` printed as `{}`, which fails: {:?}",
        source,
        printed,
        again
    );
    let again = again.unwrap();
    prop_assert_eq!(
        again.without_spans(),
        tree.without_spans(),
        "`{}` printed as `{}`",
        source,
        printed
    );
    prop_assert_eq!(print_line(&again), printed);
    Ok(())
}

proptest! {
    #[test]
    fn parse_print_parse_gives_the_same_tree(source in line()) {
        // What the generator writes is always a line.
        let parsed = parse_line(&source);
        prop_assert!(parsed.is_ok(), "`{}`: {:?}", source, parsed);
        round_trips(&source)?;
    }

    #[test]
    fn nothing_typed_makes_the_parser_panic(source in "[ -~\\t]{0,80}") {
        round_trips(&source)?;
        // Nor the binder, nor the expression rewriter.
        if let Ok(line) = parse_line(&source) {
            let _ = CommandTable::builtin().bind(&line, HostKind::File);
        }
        let _ = Expression::parse(&source);
    }

    #[test]
    fn nothing_written_makes_the_parser_panic(source in "\\PC{0,60}") {
        round_trips(&source)?;
        if let Err(errors) = parse_script(&source) {
            for error in errors {
                let _ = error.render("a.scn");
            }
        }
    }

    #[test]
    fn json_with_anything_in_it_is_parsed_or_refused(inside in "\\PC{0,30}") {
        round_trips(&format!("do {{{inside}}}"))?;
        round_trips(&format!("do {{\"k\": \"{inside}\"}} after"))?;
    }

    #[test]
    fn nothing_in_a_file_makes_the_script_parser_panic(
        lines in prop::collection::vec(prop_oneof![line().boxed(), "[ -~]{0,30}".boxed()], 0..8)
    ) {
        let text = lines.join("\n");
        match parse_script(&text) {
            Ok(script) => {
                for step in &script.steps {
                    prop_assert!(step.line >= 1 && step.line <= lines.len());
                    prop_assert_eq!(&text[step.offset..step.offset + step.source.len()], step.source.as_str());
                }
            }
            Err(errors) => {
                for error in &errors {
                    prop_assert!(error.line >= 1 && error.line <= lines.len().max(1));
                    prop_assert!(error.column >= 1);
                    prop_assert!(!error.diagnostic.message.is_empty());
                    let _ = error.render("a.scn");
                }
            }
        }
    }

    #[test]
    fn anything_typed_at_a_fake_host_is_carried_out_refused_or_explained(
        lines in prop::collection::vec(
            prop_oneof![
                Just("new a as=a".to_owned()),
                Just("add @a".to_owned()),
                Just("timer @a every=10ms".to_owned()),
                Just("advance 35ms".to_owned()),
                Just("lock @a".to_owned()),
                Just("save".to_owned()),
                Just("load".to_owned()),
                Just("hash".to_owned()),
                line(),
            ],
            0..12,
        )
    ) {
        // The same lines give the same hash and the same transcript.
        let table = FakeHost::table();
        let run = || {
            let mut host = FakeHost::new();
            let mut session = Session::new(HostKind::File);
            for line in &lines {
                let _ = session.run_line(&table, &mut host, line);
            }
            (host.state_hash(), session.transcript())
        };
        prop_assert_eq!(run(), run());
    }
}
