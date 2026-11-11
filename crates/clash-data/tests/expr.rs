//! Expressions as definition fields: the compile stage, the `function`
//! and `variable` definition types, value sources and evaluation
//! (plans/sim-kernel.md, package X4).
//!
//! The registry and the host are the ones `clash_expr::testing` offers:
//! they name things (`entity`, `mass`, `damaged`) so that fixtures read
//! like rules. The definition types here stand in for the ones other
//! crates will register.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use clash_core::{Real, define_id};
use clash_data::expr::{Any, IdOf, Int, Number, Str, register_defs};
use clash_data::{
    CompileCx, Cond, Def, DefTypes, Defs, Diagnostic, Expr, LoadOutcome, Loader, Runner, Scope,
    Severity, Text, explain,
};
use clash_expr::testing::{TestHost, slot};
use clash_expr::{DefRef, Fault, Value};
use serde::Deserialize;

define_id! {
    pub struct MaterialId(u16);
    pub struct TrapId;
    pub struct GoalId;
}

#[derive(Debug, Deserialize)]
pub struct MaterialDef {
    pub density: Real,
}

impl Def for MaterialDef {
    type Id = MaterialId;
    const TYPE: &'static str = "material";
    const DOC: &'static str = "What things are made of.";
}

/// A rule of the world: its fields read truth.
#[derive(Debug, Deserialize)]
pub struct TrapDef {
    #[serde(default)]
    pub when: Cond,
    pub energy: Expr<Number>,
    #[serde(default)]
    pub count: Option<Expr<Int>>,
    #[serde(default)]
    pub delay: Option<Expr<Number>>,
    /// What a screen would show: it must be pure.
    #[serde(default)]
    pub shown: Option<Expr<Number>>,
    #[serde(default)]
    pub made_of: Option<Expr<IdOf<MaterialDef>>>,
    #[serde(default)]
    pub label: Option<Expr<Str>>,
    #[serde(default)]
    pub log: Option<Text>,
    #[serde(default)]
    pub extra: BTreeMap<String, Expr<Any>>,
}

impl Def for TrapDef {
    type Id = TrapId;
    const TYPE: &'static str = "trap";
    const DOC: &'static str = "Something that goes off when a condition holds.";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        // The same subjects, local and event, in the same order, as the
        // test host binds, so its slots line up.
        let scope = Scope::new()
            .own("self", "entity")
            .subject("target", "entity?")
            .subject("here", "loc")
            .local("power", "real")
            .event("damaged");
        cx.compile(&mut self.when, &scope);
        cx.compile(&mut self.energy, &scope);
        cx.compile(&mut self.count, &scope);
        cx.compile(&mut self.delay, &scope);
        cx.compile(&mut self.shown, &scope.clone().pure());
        cx.compile(&mut self.made_of, &scope);
        cx.compile(&mut self.label, &scope);
        cx.compile(&mut self.log, &scope);
        cx.compile(&mut self.extra, &scope);
    }
}

/// Something a character decides by: its fields are a mind context.
#[derive(Debug, Deserialize)]
pub struct GoalDef {
    pub score: Expr<Number>,
    #[serde(default)]
    pub when: Cond,
}

impl Def for GoalDef {
    type Id = GoalId;
    const TYPE: &'static str = "goal";
    const DOC: &'static str = "Something a character wants.";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let scope = Scope::new()
            .mind("self", "entity")
            .subject("target", "entity?")
            .subject("here", "loc");
        cx.compile(&mut self.score, &scope);
        cx.compile(&mut self.when, &scope);
    }
}

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    register_defs(&mut types);
    types
        .register::<MaterialDef>()
        .register::<TrapDef>()
        .register::<GoalDef>();
    types
}

/// A mod in memory: its id, extra `mod.json` fields and its data files.
type TestMod<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

fn load_mods(mods: &[TestMod<'_>]) -> LoadOutcome {
    let types = types();
    let host = TestHost::new();
    let mut loader = Loader::new(&types).expr_registry(host.registry());
    for (id, extra, files) in mods {
        let manifest = if extra.is_empty() {
            format!("{{ \"id\": \"{id}\" }}")
        } else {
            format!("{{ \"id\": \"{id}\", {extra} }}")
        };
        loader = loader.memory_mod(id, &manifest, files);
    }
    let outcome = loader.load_all();
    for diagnostic in &outcome.diagnostics {
        assert!(
            explain(&diagnostic.code).is_some(),
            "code {} has no explanation",
            diagnostic.code
        );
    }
    outcome
}

/// Loads the mod `core` holding one data file, `data/rules.json`.
fn load_one(data: &str) -> LoadOutcome {
    load_mods(&[("core", "", &[("data/rules.json", data)])])
}

const RULES_FILE: &str = "mods/core/data/rules.json";

fn clean(outcome: LoadOutcome) -> Arc<Defs> {
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("a clean load gives definitions")
}

fn only<'a>(outcome: &'a LoadOutcome, code: &str) -> &'a Diagnostic {
    let found: Vec<&Diagnostic> = outcome
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

/// The line, from one, and the column, from one, at which `needle`
/// first appears in `text`.
fn position(text: &str, needle: &str) -> (u32, u32) {
    let at = text.find(needle).expect("the fixture holds the needle");
    let line = text[..at].matches('\n').count();
    let line_start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let column = text[line_start..at].chars().count();
    (line as u32 + 1, column as u32 + 1)
}

fn at(diagnostic: &Diagnostic) -> (&str, u32, u32) {
    let location = diagnostic.location.as_ref().expect("it has a place");
    (location.file.as_str(), location.line, location.column)
}

fn trap<'a>(defs: &'a Defs, id: &str) -> &'a TrapDef {
    defs.get::<TrapDef>(defs.lookup::<TrapDef>(id).expect("the trap loaded"))
}

/// A host whose registry is the one the definitions were compiled
/// against, so that it knows the variables and ids the mods declared.
fn host_for(defs: &Defs) -> TestHost {
    let mut host = TestHost::new();
    *host.registry_mut() = (**defs.expr_registry()).clone();
    host
}

// ---- Done when: a subject outside the context ---------------------------

#[test]
fn a_subject_outside_its_context_fails_to_load_pointing_at_the_name() {
    let data = r#"[
  { "type": "trap", "id": "pit",
    "when": "self.mass > 10 and victim.mass > 10",
    "energy": 100 }
]"#;
    let outcome = load_one(data);
    assert!(outcome.defs.is_none());
    let diagnostic = only(&outcome, "E0621");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "unknown name `victim`: nothing of that name is bound here"
    );
    // File, line and column of the name itself, inside the string.
    assert_eq!(position(data, "victim"), (3, 33));
    assert_eq!(at(diagnostic), (RULES_FILE, 3, 33));
    let location = diagnostic.location.as_ref().unwrap();
    assert_eq!((location.end_line, location.end_column), (3, 39));
    assert_eq!(diagnostic.path, "when");
    let label = diagnostic.definition.as_ref().unwrap();
    assert_eq!(
        (label.type_name.as_str(), label.id.as_str()),
        ("trap", "core:pit")
    );
    assert_eq!(outcome.diagnostics.error_count(), 1);
    // The rendering carries the caret under the name.
    let rendered = diagnostic.render();
    assert!(
        rendered.contains("--> mods/core/data/rules.json:3:33"),
        "{rendered}"
    );
    assert!(
        rendered.contains("= in: trap core:pit, at when"),
        "{rendered}"
    );

    // A local and an event field the place does not offer fail the same
    // way; a subject that is bound does not.
    let outcome = load_one(
        r#"[ { "type": "trap", "id": "pit", "energy": "$strength + event.speed + target.mass" } ]"#,
    );
    let local = only(&outcome, "E0622");
    assert_eq!(local.message, "unknown local `$strength`");
    assert_eq!(local.path, "energy");
    let field = only(&outcome, "E0625");
    assert_eq!(field.message, "event `damaged` has no field `speed`");
    // `target` may be null, which is a warning that suggests `??`.
    let null = only(&outcome, "W0644");
    assert_eq!(null.severity, Severity::Warning);
}

// ---- Done when: functions call functions ---------------------------------

const FUNCTIONS: &str = r#"[
  { "type": "function", "id": "twice", "params": { "x": "number" }, "returns": "number",
    "value": "$x * 2" },
  { "type": "function", "id": "four_times", "params": { "x": "number" }, "returns": "number",
    "value": "twice(twice($x))" },
  { "type": "function", "id": "lucky", "returns": "int", "value": "rand_int(1, 6)" },
  { "type": "function", "id": "luckier", "returns": "int", "value": "lucky() + 1" },
  { "type": "function", "id": "weigh", "params": { "who": "entity" }, "returns": "real",
    "value": "$who.mass" },
  { "type": "function", "id": "weigh_twice", "params": { "who": "entity" },
    "value": { "call": "twice", "args": { "x": "weigh($who)" } } },
  { "type": "function", "id": "east", "params": { "at": "loc" }, "returns": "int",
    "value": "$at.x" },
  { "type": "function", "id": "count_down", "params": { "n": "int" }, "returns": "int",
    "value": "if $n <= 0 then 0 else count_down($n - 1) + 1" }
]"#;

#[test]
fn functions_call_functions() {
    let trap_file = r#"[
  { "type": "trap", "id": "pit", "energy": "four_times(3) + twice($power)" }
]"#;
    let defs = clean(load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            ("data/rules.json", trap_file),
        ],
    )]));
    let mut host = host_for(&defs);
    host.set_local(slot::POWER, TestHost::real(0.5));
    let mut runner = Runner::new();
    let mut eval = runner.on(&defs, &mut host);
    assert_eq!(
        trap(&defs, "core:pit").energy.eval(&mut eval),
        Some(Real::of(13.0))
    );
    // By name, and through a value source.
    assert_eq!(
        eval.call_named("core:four_times", &[Value::Int(5)]),
        Value::Int(20)
    );
    assert_eq!(eval.call_named("core:nothing", &[]), Value::Null);

    // What the stage worked out about each.
    let functions = defs.functions();
    let flags = |name: &str| {
        let id = defs.expr_registry().function_id(name).unwrap();
        let function = functions.get(id).unwrap();
        assert_eq!(function.name, name);
        assert!(function.is_compiled());
        // The registry and the table agree.
        let spec = &defs.expr_registry().function(id).unwrap().spec;
        assert_eq!(
            (spec.random, spec.belief_safe),
            (function.random, function.belief_safe)
        );
        (function.random, function.belief_safe)
    };
    assert_eq!(flags("core:twice"), (false, true));
    assert_eq!(flags("core:four_times"), (false, true));
    // Random through what it calls.
    assert_eq!(flags("core:lucky"), (true, true));
    assert_eq!(flags("core:luckier"), (true, true));
    // Not safe for belief: it reads truth of whoever it is given; and so
    // is not whatever calls it.
    assert_eq!(flags("core:weigh"), (false, false));
    assert_eq!(flags("core:weigh_twice"), (false, false));
    assert_eq!(flags("core:east"), (false, true));
    assert_eq!(functions.len(), 8);
    assert_eq!(
        defs.expr_registry().signature(
            defs.expr_registry()
                .function(defs.expr_registry().function_id("core:twice").unwrap())
                .unwrap()
        ),
        "core:twice(x: number) -> number"
    );
}

#[test]
fn a_function_is_checked_like_any_expression() {
    let outcome = load_one(
        r#"[
  { "type": "function", "id": "bad", "params": { "x": "real", "9y": "real", "z": "reel" },
    "returns": "real", "value": "$x + $w + self.mass" }
]"#,
    );
    assert!(outcome.defs.is_none());
    let name = only(&outcome, "E0673");
    assert!(
        name.message.contains("\"9y\" cannot be a parameter"),
        "{}",
        name.message
    );
    assert_eq!(name.path, "params.9y");
    let ty = only(&outcome, "E0671");
    assert_eq!(ty.message, "\"reel\" is not a type");
    assert_eq!(ty.suggestion.as_deref(), Some("did you mean \"real\"?"));
    assert_eq!(ty.path, "params.z");

    // A function sees its parameters and nothing else.
    let outcome = load_one(
        r#"[
  { "type": "function", "id": "bad", "params": { "x": "real" },
    "returns": "real", "value": "$x + $w + self.mass" }
]"#,
    );
    assert_eq!(only(&outcome, "E0622").message, "unknown local `$w`");
    assert_eq!(only(&outcome, "E0621").path, "value");

    // The body must give what `returns` says; a required parameter may
    // not follow one with a default; a default is a constant.
    let outcome = load_one(
        r#"[
  { "type": "function", "id": "a", "params": { "x": "real" }, "returns": "bool", "value": "$x + 1" },
  { "type": "function", "id": "b",
    "params": { "x": { "type": "real", "default": 1 }, "y": "real" }, "value": "$x + $y" },
  { "type": "function", "id": "c",
    "params": { "x": { "type": "int", "default": { "expr": "rand_int(1, 6)" } } }, "value": "$x" }
]"#,
    );
    assert_eq!(
        only(&outcome, "E0645").message,
        "this must be bool, and it is real"
    );
    let order = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("comes after"))
        .unwrap();
    assert_eq!(order.code, "E0673");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0661"
                && diagnostic.path == "params.x.default.expr"),
        "{}",
        outcome.diagnostics.render()
    );
}

// ---- Done when: recursion past the depth limit is an error ---------------

#[test]
fn recursion_that_can_be_seen_is_an_error_at_load() {
    let data = r#"[
  { "type": "function", "id": "forever", "params": { "x": "int" }, "returns": "int",
    "value": "forever($x) + 1" },
  { "type": "function", "id": "ping", "params": { "x": "int" }, "returns": "int",
    "value": "1 + pong($x)" },
  { "type": "function", "id": "pong", "params": { "x": "int" }, "returns": "int",
    "value": { "call": "ping", "args": { "x": "$x" } } }
]"#;
    let outcome = load_one(data);
    assert!(outcome.defs.is_none());
    let found: Vec<&Diagnostic> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0672")
        .collect();
    assert_eq!(found.len(), 3, "{}", outcome.diagnostics.render());
    assert_eq!(
        found[0].message,
        "function `core:forever` always calls itself: core:forever → core:forever"
    );
    // At the call, inside the string.
    assert_eq!(at(found[0]), (RULES_FILE, 3, 15));
    assert_eq!(position(data, "forever($x)"), (3, 15));
    assert_eq!(found[0].path, "value");
    assert_eq!(
        found[1].message,
        "function `core:ping` always calls itself: core:ping → core:pong → core:ping"
    );
    assert_eq!(at(found[1]), (RULES_FILE, 5, 19));
    assert_eq!(
        found[2].message,
        "function `core:pong` always calls itself: core:pong → core:ping → core:pong"
    );
    assert_eq!(found[2].path, "value.call");
    assert_eq!(outcome.diagnostics.error_count(), 3);
}

#[test]
fn recursion_past_the_depth_limit_is_an_error_when_it_runs() {
    let defs = clean(load_mods(&[(
        "core",
        "",
        &[("data/functions.json", FUNCTIONS)],
    )]));
    let mut host = host_for(&defs);
    let mut runner = Runner::new();
    assert_eq!(runner.max_call_depth(), 32);
    {
        let mut eval = runner.on(&defs, &mut host);
        // Within the limit a function may call itself.
        assert_eq!(
            eval.call_named("core:count_down", &[Value::Int(5)]),
            Value::Int(5)
        );
        assert_eq!(
            eval.call_named("core:count_down", &[Value::Int(31)]),
            Value::Int(31)
        );
    }
    assert!(host.warnings.is_empty());
    {
        let mut eval = runner.on(&defs, &mut host);
        // Past it the call is refused: null, and the host is told.
        assert_eq!(
            eval.call_named("core:count_down", &[Value::Int(100)]),
            Value::Null
        );
    }
    assert_eq!(host.warnings[0].fault, Fault::CallDepth);
    assert_eq!(host.warnings[0].code().as_str(), "X908");
    // The warning names the function it came from.
    let diagnostic = defs
        .exprs()
        .warning_diagnostic(defs.sources(), &host.warnings[0]);
    assert_eq!(diagnostic.code, "W0698");
    assert_eq!(
        diagnostic.definition.as_ref().unwrap().id,
        "core:count_down"
    );
    assert_eq!(diagnostic.path, "value");

    // The limit is the host's to set.
    host.warnings.clear();
    runner.set_max_call_depth(3);
    let mut eval = runner.on(&defs, &mut host);
    assert_eq!(
        eval.call_named("core:count_down", &[Value::Int(2)]),
        Value::Int(2)
    );
    assert_eq!(
        eval.call_named("core:count_down", &[Value::Int(3)]),
        Value::Null
    );
    // No setting can ask for a depth that would exhaust the stack.
    runner.set_max_call_depth(usize::MAX);
    assert_eq!(
        runner.max_call_depth(),
        clash_data::expr::MAX_CALL_DEPTH_LIMIT
    );
    let mut eval = runner.on(&defs, &mut host);
    assert_eq!(
        eval.call_named("core:count_down", &[Value::Int(200)]),
        Value::Int(200)
    );
    assert_eq!(
        eval.call_named("core:count_down", &[Value::Int(100_000)]),
        Value::Null
    );
}

// ---- Done when: a random function where purity is required ---------------

#[test]
fn a_random_function_cannot_be_used_where_purity_is_required() {
    let trap_file = r#"[
  { "type": "trap", "id": "pit", "energy": "luckier()", "shown": "luckier() * 2" }
]"#;
    let outcome = load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            ("data/rules.json", trap_file),
        ],
    )]);
    assert!(outcome.defs.is_none());
    let diagnostic = only(&outcome, "E0661");
    assert_eq!(
        diagnostic.message,
        "`core:luckier` draws random numbers, which is not allowed where the value must be pure"
    );
    assert_eq!(diagnostic.path, "shown");
    assert_eq!(at(diagnostic), (RULES_FILE, 2, 67));
    assert_eq!(position(trap_file, "luckier() * 2"), (2, 67));
    // The same call where random draws are allowed is no error.
    assert_eq!(outcome.diagnostics.error_count(), 1);

    // A value source is held to the same rule.
    let outcome = load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            (
                "data/rules.json",
                r#"[
  { "type": "trap", "id": "a", "energy": 1, "shown": { "call": "lucky" } },
  { "type": "trap", "id": "b", "energy": 1,
    "shown": { "pick": { "entities": { "around": "self" } } } },
  { "type": "trap", "id": "c", "energy": { "pick": { "entities": { "around": "self" } } } }
]"#,
            ),
        ],
    )]);
    let found: Vec<(&str, &str)> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                diagnostic.definition.as_ref().unwrap().id.as_str(),
            )
        })
        .collect();
    assert_eq!(found, [("E0661", "core:a"), ("E0661", "core:b")]);

    // A pure function is fine there, and runs.
    let defs = clean(load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            (
                "data/rules.json",
                r#"[ { "type": "trap", "id": "pit", "energy": "lucky()", "shown": "four_times(2)" } ]"#,
            ),
        ],
    )]));
    let mut host = host_for(&defs);
    let mut runner = Runner::new();
    let mut eval = runner.on(&defs, &mut host);
    let pit = trap(&defs, "core:pit");
    assert_eq!(
        pit.shown.as_ref().unwrap().eval(&mut eval),
        Some(Real::of(8.0))
    );
    let rolled = pit.energy.eval(&mut eval).unwrap();
    assert!((Real::of(1.0)..=Real::of(6.0)).contains(&rolled));
}

// ---- The id comparison, inside an escaped string -------------------------

const MATERIALS: &str = r#"[
  { "type": "material", "id": "glass", "density": 2500 },
  { "type": "material", "id": "iron", "density": 7800 }
]"#;

#[test]
fn a_misspelt_id_is_a_load_error_that_suggests_the_closest() {
    // The expression holds a text with quotation marks in it, which the
    // file writes as escapes: the column must account for them.
    let data = r#"[
  { "type": "trap", "id": "pit",
    "when": "self.name != 'say \"hi\"' and self.material == 'glas'",
    "energy": 100 }
]"#;
    let outcome = load_mods(&[(
        "core",
        "",
        &[
            ("data/materials.json", MATERIALS),
            ("data/rules.json", data),
        ],
    )]);
    assert!(outcome.defs.is_none());
    let diagnostic = only(&outcome, "E0626");
    assert_eq!(
        diagnostic.message,
        "there is no material with the id 'glas'"
    );
    assert_eq!(
        diagnostic.suggestion.as_deref(),
        Some("did you mean 'glass'?")
    );
    assert_eq!(position(data, "'glas'"), (3, 61));
    assert_eq!(at(diagnostic), (RULES_FILE, 3, 61));
    let location = diagnostic.location.as_ref().unwrap();
    assert_eq!(location.end_column, 67);
    assert_eq!(diagnostic.path, "when");
    let rendered = diagnostic.render();
    let caret = rendered.lines().find(|line| line.contains('^')).unwrap();
    assert!(
        caret.ends_with("^^^^^^ did you mean 'glass'?"),
        "{rendered}"
    );

    // Spelt right it loads, and the comparison is made against the id.
    let defs = clean(load_mods(&[(
        "core",
        "",
        &[
            ("data/materials.json", MATERIALS),
            (
                "data/rules.json",
                r#"[ { "type": "trap", "id": "pit", "energy": 1, "made_of": "iron",
                      "when": "self.material == 'glass'" } ]"#,
            ),
        ],
    )]));
    let registry = defs.expr_registry();
    let material = registry.def_type_id("material").unwrap();
    let glass = defs.lookup::<MaterialDef>("core:glass").unwrap();
    let mut host = host_for(&defs);
    let me = host.entity(1);
    host.set(
        me,
        "material",
        Value::Id(DefRef::new(material, u32::from(glass.raw()))),
    );
    let mut runner = Runner::new();
    let mut eval = runner.on(&defs, &mut host);
    let pit = trap(&defs, "core:pit");
    assert!(pit.when.holds(&mut eval));
    // An id field holds an id, checked at load, and gives the dense id.
    let iron = defs.lookup::<MaterialDef>("core:iron").unwrap();
    assert_eq!(pit.made_of.as_ref().unwrap().eval(&mut eval), Some(iron));
    host.set(
        me,
        "material",
        Value::Id(DefRef::new(material, u32::from(iron.raw()))),
    );
    assert!(!pit.when.holds(&mut runner.on(&defs, &mut host)));

    // An id field with a misspelt id points at the whole string.
    let data = r#"[ { "type": "trap", "id": "pit", "energy": 1, "made_of": "iorn" } ]"#;
    let outcome = load_mods(&[(
        "core",
        "",
        &[
            ("data/materials.json", MATERIALS),
            ("data/rules.json", data),
        ],
    )]);
    let diagnostic = only(&outcome, "E0626");
    assert_eq!(
        diagnostic.suggestion.as_deref(),
        Some("did you mean 'iron'?")
    );
    assert_eq!(
        at(diagnostic),
        (RULES_FILE, 1, position(data, "\"iorn\"").1)
    );
    assert_eq!(diagnostic.path, "made_of");
}

// ---- Variables -----------------------------------------------------------

#[test]
fn an_undeclared_variable_warns_and_is_an_error_under_strict_vars() {
    let data = r#"[
  { "type": "variable", "id": "rage", "scope": "entity", "value": "int", "default": 0 },
  { "type": "trap", "id": "pit", "energy": 1,
    "when": "self.var.rage > 3 and self.var.raeg > 3" }
]"#;
    // A warning: the load succeeds.
    let outcome = load_one(data);
    let diagnostic = only(&outcome, "W0627");
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(diagnostic.message, "variable `core:raeg` is not declared");
    assert_eq!(
        diagnostic.suggestion.as_deref(),
        Some("did you mean `rage`?")
    );
    assert_eq!(
        at(diagnostic),
        (RULES_FILE, 4, position(data, "self.var.raeg").1)
    );
    assert_eq!(diagnostic.path, "when");
    assert_eq!(outcome.diagnostics.len(), 1);
    assert!(outcome.defs.is_some());

    // The mod asks for strictness: the same thing is an error.
    let outcome = load_mods(&[(
        "core",
        "\"strict_vars\": true",
        &[("data/rules.json", data)],
    )]);
    let diagnostic = only(&outcome, "W0627");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert!(outcome.defs.is_none());

    // Strictness belongs to the mod that wrote the expression.
    let outcome = load_mods(&[
        ("core", "\"strict_vars\": true", &[]),
        (
            "loose",
            "\"requires\": [\"core\"]",
            &[(
                "data/rules.json",
                r#"[ { "type": "trap", "id": "pit", "energy": "self.var.whatever ?? 0" } ]"#,
            )],
        ),
    ]);
    assert_eq!(only(&outcome, "W0627").severity, Severity::Warning);
    assert!(outcome.defs.is_some());

    // `strict_vars` is a boolean.
    let outcome = load_mods(&[("core", "\"strict_vars\": \"yes\"", &[])]);
    assert!(only(&outcome, "E0001").message.contains("`strict_vars`"));
}

#[test]
fn a_variable_declares_its_scope_type_and_default() {
    let defs = clean(load_one(
        r#"[
  { "type": "variable", "id": "rage", "scope": "entity", "value": "int", "default": 2,
    "doc": "How angry it is.", "watch": true, "belief_safe": true },
  { "type": "variable", "id": "sealed", "scope": "cell", "value": "bool", "default": false },
  { "type": "variable", "id": "temper", "scope": "entity", "value": "string", "default": "calm" },
  { "type": "variable", "id": "nickname", "scope": "entity", "value": "string" },
  { "type": "variable", "id": "tally", "scope": "entity", "value": "map<int>",
    "default": { "expr": "{'iron': 1, 'ash': 3}" } },
  { "type": "trap", "id": "pit",
    "when": "not here.var.sealed and self.var.temper == 'calm'",
    "energy": "self.var.rage * 10",
    "count": "self.var.tally['ash'] ?? 0",
    "extra": { "keys": "keys(self.var.tally)" } }
]"#,
    ));
    let variables = defs.variables();
    assert_eq!(variables.len(), 5);
    let rage = variables.get("core:rage").unwrap();
    assert_eq!(rage.default, Value::Int(2));
    assert!(rage.watch && rage.belief_safe);
    assert_eq!(rage.doc, "How angry it is.");
    let registry = defs.expr_registry();
    assert_eq!(registry.type_name(&rage.ty), "int");
    assert_eq!(rage.kind, registry.kind_id("entity").unwrap());
    // `cell` is the scope of what lives on a location.
    let sealed = variables.get("core:sealed").unwrap();
    assert_eq!(sealed.kind, registry.kind_id("loc").unwrap());
    assert_eq!(variables.default_of("core:sealed"), Value::Bool(false));
    assert_eq!(variables.default_of("core:temper"), Value::from("calm"));
    // With no default it reads as null until set, and its type says so.
    let nickname = variables.get("core:nickname").unwrap();
    assert_eq!(registry.type_name(&nickname.ty), "string?");
    assert_eq!(variables.default_of("core:nickname"), Value::Null);
    assert_eq!(variables.default_of("core:undeclared"), Value::Null);
    // The registry knows them, so reading one is checked.
    assert!(registry.variable_id("core:rage").is_some());
    let names: Vec<&str> = variables.iter().map(|decl| decl.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "core:nickname",
            "core:rage",
            "core:sealed",
            "core:tally",
            "core:temper"
        ]
    );
    // A map is a value like any other, and a constant one is a default.
    let tally = variables.get("core:tally").unwrap();
    assert_eq!(registry.type_name(&tally.ty), "map<int>");
    assert_eq!(
        tally.default,
        Value::map([("ash", Value::Int(3)), ("iron", Value::Int(1))])
    );

    // Evaluated against a host that answers with the defaults.
    let mut host = host_for(&defs);
    let (me, here) = (host.entity(1), host.place(100));
    host.set_var(me, "core:tally", variables.default_of("core:tally"));
    host.set_var(me, "core:rage", variables.default_of("core:rage"));
    host.set_var(me, "core:temper", variables.default_of("core:temper"));
    host.set_var(here, "core:sealed", variables.default_of("core:sealed"));
    let mut runner = Runner::new();
    let mut eval = runner.on(&defs, &mut host);
    let pit = trap(&defs, "core:pit");
    assert!(pit.when.holds(&mut eval));
    assert_eq!(pit.energy.eval(&mut eval), Some(Real::of(20.0)));
    // A map is read by key, and its keys come in order of key.
    assert_eq!(pit.count.as_ref().unwrap().eval(&mut eval), Some(3));
    assert_eq!(
        pit.extra["keys"].value(&mut eval),
        Value::list([Value::from("ash"), Value::from("iron")])
    );
}

#[test]
fn a_bad_variable_declaration_says_what_is_wrong() {
    let data = r#"[
  { "type": "variable", "id": "a", "scope": "entty", "value": "int" },
  { "type": "variable", "id": "b", "scope": "entity", "value": "integer" },
  { "type": "variable", "id": "c", "scope": "entity", "value": "int", "default": "many" },
  { "type": "variable", "id": "d", "scope": "site", "value": "bool", "default": false },
  { "type": "trap", "id": "pit", "energy": 1, "when": "self.var.d" }
]"#;
    let outcome = load_one(data);
    assert!(outcome.defs.is_none());
    let scope = only(&outcome, "E0674");
    assert_eq!(
        scope.message,
        "\"entty\" is not something a variable can live on"
    );
    assert_eq!(
        scope.suggestion.as_deref(),
        Some("did you mean \"entity\"?")
    );
    assert_eq!(at(scope), (RULES_FILE, 2, position(data, "\"entty\"").1));
    let ty = only(&outcome, "E0671");
    assert_eq!(ty.message, "\"integer\" is not a type");
    assert_eq!(ty.definition.as_ref().unwrap().id, "core:b");
    // A text where an int is wanted is an expression, and `many` names
    // nothing.
    assert_eq!(only(&outcome, "E0621").path, "default");
    // Declared on a site, read on an entity.
    let wrong = only(&outcome, "E0629");
    assert_eq!(
        wrong.message,
        "variable `core:d` is declared on `site`, and this is `entity`"
    );
}

// ---- Text ----------------------------------------------------------------

#[test]
fn a_bad_expression_in_a_text_points_inside_the_braces() {
    let data = r#"[
  { "type": "trap", "id": "pit", "energy": 1,
    "log": "{the(self)} takes {self.mas:.0} J, {{really}}." }
]"#;
    let outcome = load_one(data);
    assert!(outcome.defs.is_none());
    let diagnostic = only(&outcome, "E0623");
    assert_eq!(diagnostic.message, "`entity` has no property `mas`");
    assert_eq!(
        diagnostic.suggestion.as_deref(),
        Some("did you mean `mass`?")
    );
    // Inside the second pair of braces, not at the string or the brace.
    assert_eq!(position(data, "self.mas"), (3, 32));
    assert_eq!(at(diagnostic), (RULES_FILE, 3, 32));
    assert_eq!(diagnostic.path, "log");

    // A brace that was never opened, and a format that is not `.N`.
    let outcome = load_one(
        r#"[ { "type": "trap", "id": "a", "energy": 1, "log": "{the(self)} falls}." },
            { "type": "trap", "id": "b", "energy": 1, "log": "{self.mass:.x}" } ]"#,
    );
    assert_eq!(
        only(&outcome, "E0608").definition.as_ref().unwrap().id,
        "core:a"
    );
    assert_eq!(
        only(&outcome, "E0609").definition.as_ref().unwrap().id,
        "core:b"
    );

    // A good one renders for its reader, with the helpers the host
    // answers.
    let defs = clean(load_one(
        r#"[ { "type": "trap", "id": "pit", "energy": 1,
              "log": "{cap(the(self))} {verb(self, 'take')} {event.energy:.0} J from {the(target)}, {{really}}." } ]"#,
    ));
    let mut host = host_for(&defs);
    host.set_event(slot::ENERGY, TestHost::real(12.4));
    let other = host.entity(2);
    host.set(other, "name", "guard");
    let mut runner = Runner::new();
    let pit = trap(&defs, "core:pit");
    let log = pit.log.as_ref().unwrap();
    assert!(log.compiled().is_some());
    assert_eq!(
        log.render(&mut runner.on(&defs, &mut host)),
        "You take 12 J from the guard, {really}."
    );
    assert_eq!(
        log.raw(),
        "{cap(the(self))} {verb(self, 'take')} {event.energy:.0} J from {the(target)}, {{really}}."
    );
    assert!(log.span().is_some());
    // A text made in Rust was never compiled, and reads as written.
    let plain = Text::plain("as {written}");
    assert_eq!(
        plain.render(&mut runner.on(&defs, &mut host)),
        "as {written}"
    );
}

// ---- The mind context ----------------------------------------------------

#[test]
fn a_mind_context_refuses_what_is_not_safe_for_belief() {
    let data = r#"[
  { "type": "goal", "id": "flee", "score": "target.mass ?? 0" }
]"#;
    let outcome = load_one(data);
    assert!(outcome.defs.is_none());
    let diagnostic = only(&outcome, "E0662");
    assert_eq!(
        diagnostic.message,
        "`entity.mass` is not safe for belief: a character may read it only of itself"
    );
    assert_eq!(
        at(diagnostic),
        (RULES_FILE, 2, position(data, "target.mass").1)
    );
    assert_eq!(diagnostic.path, "score");
    assert_eq!(diagnostic.definition.as_ref().unwrap().type_name, "goal");

    // The character's own state, and what is registered safe, may be
    // read. The same read is fine in a rule of the world.
    clean(load_one(
        r#"[
  { "type": "goal", "id": "flee", "score": "self.mass + here.x", "when": "self.var.kills > 0" },
  { "type": "trap", "id": "pit", "energy": "target.mass ?? 0" }
]"#,
    ));

    // Every function a mind context reaches is held to it: one that
    // reads truth is refused, by name or through a value source, and so
    // is whatever calls it.
    let goals = r#"[
  { "type": "goal", "id": "a", "score": "weigh(self)" },
  { "type": "goal", "id": "b", "score": { "call": "weigh_twice", "args": { "who": "self" } } },
  { "type": "goal", "id": "c", "score": "east(here) + twice(2)" }
]"#;
    let outcome = load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            ("data/rules.json", goals),
        ],
    )]);
    let refused: Vec<(&str, &str)> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                diagnostic.definition.as_ref().unwrap().id.as_str(),
            )
        })
        .collect();
    assert_eq!(refused, [("E0662", "core:a"), ("E0662", "core:b")]);
    assert_eq!(
        outcome.diagnostics.as_slice()[0].message,
        "`core:weigh` is not safe for belief: it reads what a character may not know"
    );
}

// ---- A fixture mod, loaded and evaluated ---------------------------------

fn fixture_mods() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("expr")
        .join("mods")
}

fn load_fixture() -> Arc<Defs> {
    let types = types();
    let host = TestHost::new();
    let outcome = Loader::new(&types)
        .expr_registry(host.registry())
        .search_path(fixture_mods())
        .load_mods(&["core", "ashlands"]);
    clean(outcome)
}

#[test]
fn a_mod_with_a_function_a_variable_and_a_definition_using_both_evaluates() {
    let defs = load_fixture();
    let registry = defs.expr_registry();
    let material = registry.def_type_id("material").unwrap();
    let glass = defs.lookup::<MaterialDef>("core:glass").unwrap();
    let iron = defs.lookup::<MaterialDef>("core:iron").unwrap();

    // What the simulation would hold: the entity, the place and the
    // variables at their declared defaults.
    let mut host = host_for(&defs);
    let (me, here) = (host.entity(1), host.place(100));
    host.set(
        me,
        "material",
        Value::Id(DefRef::new(material, u32::from(glass.raw()))),
    );
    host.set_var(me, "core:heat", defs.variables().default_of("core:heat"));
    host.set_var(
        here,
        "core:sealed",
        defs.variables().default_of("core:sealed"),
    );
    host.set_local(slot::POWER, TestHost::real(0.5));

    let vent = trap(&defs, "ashlands:vent");
    let mut runner = Runner::new();
    let mut eval = runner.on(&defs, &mut host);

    // The condition: a structure of two expressions.
    assert!(vent.when.holds(&mut eval));
    // fall_energy(2) with the default mass, plus the variable's default
    // times ten, plus the local.
    let scorch = 70.0 * 9.81 * 2.0 + 20.0 * 10.0;
    assert_eq!(vent.energy.eval(&mut eval), Some(Real::of(scorch + 0.5)));
    // Literals: a whole number, a quantity with a unit, an id, a text.
    assert_eq!(vent.count.as_ref().unwrap().eval(&mut eval), Some(3));
    assert_eq!(
        vent.delay.as_ref().unwrap().eval(&mut eval),
        Some(Real::of(0.25))
    );
    assert_eq!(vent.made_of.as_ref().unwrap().eval(&mut eval), Some(iron));
    assert_eq!(
        vent.label.as_ref().unwrap().eval(&mut eval).as_deref(),
        Some("a vent")
    );
    assert_eq!(
        vent.count.as_ref().unwrap().constant(),
        Some(&Value::Int(3))
    );
    // The text, for its reader.
    assert_eq!(
        vent.log.as_ref().unwrap().render(&mut eval),
        format!("You feel {scorch:.1} J.")
    );
    // A function result as a value source, with a default filled in.
    assert_eq!(
        vent.extra["drop"].value(&mut eval),
        TestHost::real(70.0 * 9.81 * 1.0)
    );
    assert!(host.warnings.is_empty());

    // The variable set, the function reads the new value; the place
    // sealed, the condition no longer holds.
    host.set_var(me, "core:heat", TestHost::real(30.0));
    host.set_var(here, "core:sealed", true);
    let mut eval = runner.on(&defs, &mut host);
    assert_eq!(
        vent.energy.eval(&mut eval),
        Some(Real::of(70.0 * 9.81 * 2.0 + 300.0 + 0.5))
    );
    assert!(!vent.when.holds(&mut eval));

    // A query is read and kept, and says so when it is evaluated.
    assert_eq!(vent.extra["nearby"].value(&mut eval), Value::Null);
    assert_eq!(host.warnings.len(), 1);
    assert_eq!(host.warnings[0].fault, Fault::Unavailable("queries"));
    let diagnostic = defs
        .exprs()
        .warning_diagnostic(defs.sources(), &host.warnings[0]);
    assert_eq!(diagnostic.code, "W0699");
    assert_eq!(diagnostic.message, "queries are not available yet");
    assert_eq!(diagnostic.path, "extra.nearby");
    assert_eq!(diagnostic.definition.as_ref().unwrap().id, "ashlands:vent");

    // Once, without keeping a runner; and a function by its definition.
    assert_eq!(
        vent.count.as_ref().unwrap().eval_on(&defs, &mut host),
        Some(3)
    );
    assert!(!vent.when.holds_on(&defs, &mut host));
    assert!(
        vent.log
            .as_ref()
            .unwrap()
            .render_on(&defs, &mut host)
            .starts_with("You feel ")
    );
    let fall = defs
        .lookup::<clash_data::expr::FunctionDef>("core:fall_energy")
        .unwrap();
    assert_eq!(
        runner
            .on(&defs, &mut host)
            .call_def(fall, &[Value::Int(1), Value::Int(2)]),
        TestHost::real(2.0 * 9.81)
    );

    // What `Defs` offers the simulation.
    let scorch = defs
        .functions()
        .get(registry.function_id("ashlands:scorch").unwrap());
    let scorch = scorch.unwrap();
    assert_eq!(scorch.params.len(), 1);
    assert!(!scorch.random);
    // It reads a variable of whoever it is given, which belief may not.
    assert!(!scorch.belief_safe);
    let fall = defs
        .functions()
        .get(registry.function_id("core:fall_energy").unwrap());
    assert_eq!(fall.unwrap().params[1].default, Some(Value::Int(70)));
    assert_eq!(
        defs.variables().get("core:sealed").map(|decl| decl.watch),
        Some(true)
    );
    assert_eq!(
        defs.ids_of("function").unwrap(),
        ["ashlands:scorch", "core:fall_energy"]
    );
    // The loaded ids of a type replace whatever the host registered.
    let ids: Vec<(&str, u32)> = registry.def_type(material).unwrap().ids().collect();
    assert_eq!(ids, [("core:glass", 0), ("core:iron", 1)]);
}

#[test]
fn a_run_time_warning_is_traced_to_its_field() {
    let data = r#"[
  { "type": "trap", "id": "pit", "energy": "100 / $power" }
]"#;
    let defs = clean(load_one(data));
    let mut host = host_for(&defs);
    host.set_local(slot::POWER, TestHost::real(0.0));
    let mut runner = Runner::new();
    let pit = trap(&defs, "core:pit");
    assert_eq!(pit.energy.eval(&mut runner.on(&defs, &mut host)), None);
    let warning = host.warnings[0];
    assert_eq!(warning.fault, Fault::DivisionByZero);
    let origin = defs.exprs().origin(warning.origin).unwrap();
    assert_eq!(origin.definition.id, "core:pit");
    assert_eq!(origin.path, "energy");
    let diagnostic = defs.exprs().warning_diagnostic(defs.sources(), &warning);
    assert_eq!(diagnostic.code, "W0692");
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        at(&diagnostic),
        (RULES_FILE, 2, position(data, "100 / $power").1)
    );
    assert!(defs.exprs().origin(0).is_none());
}

// ---- What a field may hold (05 §7.1, §7.2, §7.4) -------------------------

#[test]
fn a_field_holds_a_literal_an_expression_or_a_source() {
    let defs = clean(load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            ("data/materials.json", MATERIALS),
            (
                "data/rules.json",
                r#"[
  { "type": "trap", "id": "literal", "energy": 12, "count": 2.0e1, "delay": "5 m/s",
    "when": true, "label": "it's 'here'",
    "extra": { "list": [1, 2.5, "three", [true]], "nothing": null, "text": "'a' + 'b'" } },
  { "type": "trap", "id": "written", "energy": { "expr": "3 * 4" },
    "label": { "expr": "'a' + 'b'" }, "made_of": { "expr": "if $power > 1 then 'glass' else 'iron'" },
    "when": { "any": [ false, { "all": [ "$power > 0", { "not": { "expr": "$power > 10" } } ] } ] } },
  { "type": "trap", "id": "plain", "energy": "$power" }
]"#,
            ),
        ],
    )]));
    let mut host = host_for(&defs);
    host.set_local(slot::POWER, TestHost::real(2.0));
    let mut runner = Runner::new();
    let mut eval = runner.on(&defs, &mut host);

    let literal = trap(&defs, "core:literal");
    assert_eq!(literal.energy.eval(&mut eval), Some(Real::of(12.0)));
    assert_eq!(literal.energy.constant(), Some(&Value::Int(12)));
    // A whole number may be written with an exponent.
    assert_eq!(literal.count.as_ref().unwrap().eval(&mut eval), Some(20));
    // A quantity in a compound unit the expression language has not.
    assert_eq!(
        literal.delay.as_ref().unwrap().eval(&mut eval),
        Some(Real::of(5.0))
    );
    assert_eq!(literal.when.constant_value(), Some(true));
    assert!(literal.when.holds(&mut eval));
    // A text field holds the text itself, quotation marks and all.
    assert_eq!(
        literal.label.as_ref().unwrap().eval(&mut eval).as_deref(),
        Some("it's 'here'")
    );
    assert_eq!(
        literal.extra["list"].value(&mut eval),
        Value::list([
            Value::Int(1),
            TestHost::real(2.5),
            Value::from("three"),
            Value::list([Value::Bool(true)])
        ])
    );
    assert_eq!(literal.extra["nothing"].value(&mut eval), Value::Null);
    // Where the type is not a text, a string is an expression.
    assert_eq!(literal.extra["text"].value(&mut eval), Value::from("ab"));
    assert_eq!(literal.extra["text"].source(), Some("'a' + 'b'"));

    let written = trap(&defs, "core:written");
    assert_eq!(written.energy.eval(&mut eval), Some(Real::of(12.0)));
    assert_eq!(
        written.label.as_ref().unwrap().eval(&mut eval).as_deref(),
        Some("ab")
    );
    let glass = defs.lookup::<MaterialDef>("core:glass").unwrap();
    assert_eq!(
        written.made_of.as_ref().unwrap().eval(&mut eval),
        Some(glass)
    );
    assert!(written.when.holds(&mut eval));
    assert!(written.when.compiled().is_none());

    let plain = trap(&defs, "core:plain");
    assert_eq!(plain.energy.eval(&mut eval), Some(Real::of(2.0)));
    assert!(plain.energy.compiled().is_some());
    // A rule with no `if` always holds.
    assert!(plain.when.holds(&mut eval));
    host.set_local(slot::POWER, TestHost::real(50.0));
    assert!(!written.when.holds(&mut runner.on(&defs, &mut host)));

    // Made in Rust, for a default or a placeholder.
    let constant = Expr::<Number>::literal(3);
    assert!(constant.is_compiled());
    assert_eq!(
        constant.eval(&mut runner.on(&defs, &mut host)),
        Some(Real::of(3.0))
    );
    assert!(Cond::default().holds(&mut runner.on(&defs, &mut host)));
}

#[test]
fn a_value_of_the_wrong_shape_is_refused_where_it_is() {
    let data = r#"[
  { "type": "trap", "id": "a", "energy": true },
  { "type": "trap", "id": "b", "energy": 1, "count": 2.5 },
  { "type": "trap", "id": "c", "energy": { "expr": "1", "count": {} } },
  { "type": "trap", "id": "d", "energy": { "call": "fall", "args": { "x": 1 } } },
  { "type": "trap", "id": "e", "energy": { "sum": {}, "weight": "it.mass" } },
  { "type": "trap", "id": "f", "energy": { "exprs": "1" } },
  { "type": "trap", "id": "g", "energy": 1, "when": { "all": [ "true" ], "any": [] } },
  { "type": "trap", "id": "h", "energy": 1, "when": "$power" },
  { "type": "trap", "id": "i", "energy": null }
]"#;
    let outcome = load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            ("data/rules.json", data),
        ],
    )]);
    assert!(outcome.defs.is_none());
    let by_id = |id: &str| -> Vec<(&str, &str)> {
        outcome
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .definition
                    .as_ref()
                    .is_some_and(|label| label.id == id)
            })
            .map(|diagnostic| (diagnostic.code.as_str(), diagnostic.message.as_str()))
            .collect()
    };
    assert_eq!(
        by_id("core:a"),
        [("E0645", "this must be real, and it is bool")]
    );
    assert_eq!(
        by_id("core:b"),
        [("E0645", "this must be int, and it is real")]
    );
    assert_eq!(by_id("core:c").len(), 1);
    assert!(
        by_id("core:c")[0]
            .1
            .contains("exactly one of `expr`, `call`")
    );
    assert_eq!(by_id("core:d"), [("E0624", "unknown function \"fall\"")]);
    assert_eq!(
        by_id("core:e"),
        [("E0207", "`weight` belongs with `pick`, which is not here")]
    );
    assert!(by_id("core:f").contains(&("E0203", "unknown key \"exprs\"")));
    assert!(
        by_id("core:g")[0]
            .1
            .contains("exactly one of `all`, `any`, `not`")
    );
    assert_eq!(
        by_id("core:h"),
        [("E0645", "this must be bool?, and it is real")]
    );
    assert_eq!(
        by_id("core:i"),
        [("E0645", "this must be real, and it is null")]
    );

    // A call names a function definition, and gives arguments by name.
    let outcome = load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            (
                "data/rules.json",
                r#"[
  { "type": "trap", "id": "a", "energy": { "call": "twice", "args": { "y": 1 } } },
  { "type": "trap", "id": "b", "energy": { "call": "clamp", "args": {} } },
  { "type": "trap", "id": "c", "energy": { "call": "twice", "args": { "x": "'two'" } } }
]"#,
            ),
        ],
    )]);
    let found: Vec<(&str, &str)> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.code.as_str(), diagnostic.path.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("E0642", "energy.call"),
            ("E0642", "energy.args.y"),
            ("E0676", "energy.call"),
            ("E0645", "energy.args.x")
        ]
    );
    assert_eq!(
        outcome.diagnostics.as_slice()[1].suggestion.as_deref(),
        Some("did you mean \"x\"?")
    );
}

// ---- The loader's side ---------------------------------------------------

#[test]
fn the_registry_is_the_hosts_with_what_the_mods_define() {
    let defs = clean(load_mods(&[(
        "core",
        "",
        &[
            ("data/functions.json", FUNCTIONS),
            ("data/materials.json", MATERIALS),
        ],
    )]));
    let registry = defs.expr_registry();
    // What the host registered is still there.
    assert!(registry.kind_id("entity").is_some());
    assert!(registry.function_id("dist").is_some());
    assert!(registry.function_id("clamp").is_some());
    // Every definition type is one ids can refer to.
    for name in ["function", "goal", "material", "trap", "variable"] {
        assert!(registry.def_type_id(name).is_some(), "{name}");
    }
    let material = registry.def_type_id("material").unwrap();
    assert!(registry.def_ref(material, "core:glass").is_some());
    // Shared, and the host's own registry is untouched.
    let shared: Arc<clash_data::expr::ExprRegistry> = Arc::clone(registry);
    assert!(shared.function_id("core:twice").is_some());
    assert!(
        TestHost::new()
            .registry()
            .function_id("core:twice")
            .is_none()
    );

    // A function may not take the name of one the engine has.
    let outcome = load_one(
        r#"[ { "type": "function", "id": "double", "params": { "x": "number" }, "value": "$x * 2" } ]"#,
    );
    let diagnostic = only(&outcome, "E0673");
    assert_eq!(
        diagnostic.message,
        "function `core:double` is already registered"
    );

    // With no registry given, the built-in functions are there and no
    // kinds are: a scope that names one says so.
    let types = types();
    let outcome = Loader::new(&types)
        .memory_mod(
            "core",
            r#"{ "id": "core" }"#,
            &[(
                "data/rules.json",
                r#"[ { "type": "function", "id": "half", "params": { "x": "real" }, "value": "$x / 2" },
                     { "type": "trap", "id": "pit", "energy": "clamp(half(8), 0, 3)" } ]"#,
            )],
        )
        .load_all();
    assert!(outcome.defs.is_none());
    assert!(
        outcome
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "E0675"),
        "{}",
        outcome.diagnostics.render()
    );
    assert!(
        outcome.diagnostics.as_slice()[0]
            .message
            .contains("the type `entity`, which is not registered")
    );
}

#[test]
fn the_stage_does_not_depend_on_the_order_mods_are_given_in() {
    let core: &[(&str, &str)] = &[
        ("data/functions.json", FUNCTIONS),
        ("data/materials.json", MATERIALS),
    ];
    let extra: &[(&str, &str)] = &[(
        "data/more.json",
        r#"[ { "type": "function", "id": "eight_times", "params": { "x": "number" },
              "value": "four_times(twice($x))" },
            { "type": "variable", "id": "seen", "scope": "entity", "value": "bool", "default": false },
            { "type": "trap", "id": "pit", "energy": "eight_times(1)", "when": "self.var.seen" } ]"#,
    )];
    let one = clean(load_mods(&[
        ("core", "", core),
        ("extra", "\"requires\": [\"core\"]", extra),
    ]));
    let other = clean(load_mods(&[
        ("extra", "\"requires\": [\"core\"]", extra),
        ("core", "", core),
    ]));
    assert_eq!(one.fingerprint(), other.fingerprint());
    for name in ["core:twice", "core:count_down", "extra:eight_times"] {
        assert_eq!(
            one.expr_registry().function_id(name),
            other.expr_registry().function_id(name),
            "{name}"
        );
    }
    assert_eq!(
        one.expr_registry().variable_id("extra:seen"),
        other.expr_registry().variable_id("extra:seen")
    );
    let mut host = host_for(&one);
    let mut runner = Runner::new();
    let mut eval = runner.on(&one, &mut host);
    assert_eq!(
        trap(&one, "extra:pit").energy.eval(&mut eval),
        Some(Real::of(8.0))
    );
}

#[test]
fn a_lenient_load_stops_at_a_broken_expression_with_no_placeholder() {
    let types = types();
    let host = TestHost::new();
    let outcome = Loader::new(&types)
        .expr_registry(host.registry())
        .lenient(true)
        .memory_mod(
            "core",
            r#"{ "id": "core" }"#,
            &[(
                "data/rules.json",
                r#"[ { "type": "trap", "id": "pit", "energy": "nobody.mass" } ]"#,
            )],
        )
        .load_all();
    assert!(outcome.defs.is_none());
    assert_eq!(only(&outcome, "E0621").path, "energy");
    assert!(
        only(&outcome, "E0902")
            .message
            .contains("trap core:pit is broken")
    );
}

#[test]
fn defs_with_compiled_fields_are_shared_across_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Defs>();
    assert_send_sync::<Expr<Number>>();
    assert_send_sync::<Cond>();
    assert_send_sync::<Text>();

    let defs = load_fixture();
    let handles: Vec<_> = (0..4)
        .map(|power| {
            let defs = Arc::clone(&defs);
            std::thread::spawn(move || {
                let mut host = host_for(&defs);
                let me = host.entity(1);
                host.set_var(me, "core:heat", TestHost::real(0.0));
                host.set_local(slot::POWER, TestHost::real(f64::from(power)));
                let mut runner = Runner::new();
                trap(&defs, "ashlands:vent")
                    .energy
                    .eval(&mut runner.on(&defs, &mut host))
            })
        })
        .collect();
    for (power, handle) in handles.into_iter().enumerate() {
        let expected = 70.0 * 9.81 * 2.0 + power as f64;
        assert_eq!(handle.join().unwrap(), Some(Real::of(expected)));
    }
}
