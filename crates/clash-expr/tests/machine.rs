//! X3: constant folding, bytecode and the stack machine.
//!
//! Every row of 05 §4.4 is tested. The machine agrees with the direct
//! tree-walking evaluator on random expressions: the same value, the
//! same warnings and the same random draws. Faults give null and a
//! warning that says where; warnings are rate-limited per site.

use clash_core::{Hash128, Real, Rng};
use clash_expr::testing::{TestHost, slot};
use clash_expr::{
    Compiled, CompiledText, Context, Fault, Limits, Machine, Registry, RuntimeWarning, Span,
    TextSink, Type, Value, WarningLimiter, check, check_template, codes, compile, compile_template,
};
use proptest::prelude::*;

/// The origin number the test context compiles with.
const ORIGIN: u64 = 77;

fn context(host: &TestHost) -> Context {
    host.context().origin(ORIGIN)
}

/// Evaluates `source` both ways, by the machine and by walking the tree,
/// each against its own copy of the host, and checks that they agree on
/// the value, on every warning, on every call made to the host and on
/// the random stream afterwards. Leaves the host as the machine left it.
#[track_caller]
fn run(host: &mut TestHost, source: &str) -> (Value, Vec<RuntimeWarning>) {
    let context = context(host);
    let checked = check(source, host.registry(), &context, &Type::Any);
    assert!(
        !checked.has_errors(),
        "`{source}`: {:#?}",
        checked.diagnostics
    );
    let checked = checked.value.expect("no errors");
    let compiled = checked.compile();
    assert!(
        !compiled.has_errors(),
        "`{source}`: {:#?}",
        compiled.diagnostics
    );
    let compiled = compiled.value.expect("no errors");

    let mut walked = host.clone();
    walked.warnings.clear();
    walked.log.clear();
    let direct = checked.interpret(&mut walked, &Limits::default());

    host.warnings.clear();
    host.log.clear();
    let value = Machine::new().eval(&compiled, host);

    assert_eq!(
        value, direct,
        "`{source}`: the machine and the tree disagree"
    );
    assert_eq!(
        host.warnings, walked.warnings,
        "`{source}`: warnings differ"
    );
    assert_eq!(host.log, walked.log, "`{source}`: calls to the host differ");
    assert_eq!(host.rng, walked.rng, "`{source}`: random draws differ");
    (value, host.warnings.clone())
}

/// Evaluates against a fresh host and expects no warning.
#[track_caller]
fn value_of(source: &str) -> Value {
    let mut host = TestHost::new();
    let (value, warnings) = run(&mut host, source);
    assert_eq!(warnings, [], "`{source}`");
    value
}

/// Evaluates and expects null with exactly one warning: this fault,
/// covering this text of the source.
#[track_caller]
fn faults(host: &mut TestHost, source: &str, fault: Fault, covers: &str) {
    let (value, warnings) = run(host, source);
    assert_eq!(value, Value::Null, "`{source}`");
    assert_eq!(warnings.len(), 1, "`{source}`: {warnings:?}");
    assert_eq!(warnings[0].fault, fault, "`{source}`");
    assert_eq!(warnings[0].span.slice(source), covers, "`{source}`");
    assert_eq!(warnings[0].origin, ORIGIN, "`{source}`");
}

fn real(value: f64) -> Value {
    Value::Real(Real::of(value))
}

fn int(value: i64) -> Value {
    Value::Int(value)
}

fn text(value: &str) -> Value {
    Value::text(value)
}

fn ints(values: &[i64]) -> Value {
    Value::list(values.iter().copied().map(Value::Int))
}

// ---------------------------------------------------------------------
// 05 §4.4, row by row.

#[test]
fn null_row_1_a_property_of_null_is_null() {
    let mut host = TestHost::new();
    host.set_subject(slot::TARGET, Value::Null);
    for source in [
        "target.mass",
        "target.loc",
        "target.loc.site.name",
        "target.target.target.mass",
        "target.has_tag('guard')",
        "target.part('arm').size",
        // A variable of null and an element of null are null too.
        "target.var.kills",
        "target.var.mood",
        "target.stats['speed']",
        "target.tags[0]",
        "$x.anything",
        "$x[3]",
        "$x.var.thing",
    ] {
        let (value, warnings) = run(&mut host, source);
        assert_eq!(value, Value::Null, "`{source}`");
        assert_eq!(warnings, [], "`{source}`");
        // The host is never asked for a property of nothing.
        assert_eq!(host.log, Vec::<String>::new(), "`{source}`");
    }
    // A property that reads as null passes it on the same way.
    host.set_subject(slot::TARGET, host.entity(2));
    let (value, warnings) = run(&mut host, "target.target.mass");
    assert_eq!(value, Value::Null);
    assert_eq!(warnings, []);
    assert_eq!(host.log, ["subject#0:2.target"]);
}

#[test]
fn null_row_2_arithmetic_with_null_is_null_and_warns_with_the_position() {
    let mut host = TestHost::new();
    host.set_local(slot::POWER, real(2.0));
    host.set_local(slot::N, 3);
    // `$maybe` is null.
    for (source, covers) in [
        ("$maybe + 1", "$maybe + 1"),
        ("1 + $maybe", "1 + $maybe"),
        ("$maybe - $power", "$maybe - $power"),
        ("$power * $maybe", "$power * $maybe"),
        ("$maybe / 2", "$maybe / 2"),
        ("$maybe ^ 2", "$maybe ^ 2"),
        ("$x % 2", "$x % 2"),
        ("-$maybe", "-$maybe"),
        ("null + 1", "null + 1"),
        ("$x + 'text'", "$x + 'text'"),
        ("$x + [1]", "$x + [1]"),
        // A function that needs a value is the same.
        ("sqrt($maybe)", "sqrt($maybe)"),
        ("clamp($maybe, 0, 1)", "clamp($maybe, 0, 1)"),
        ("min(1, $maybe)", "min(1, $maybe)"),
        ("abs($x)", "abs($x)"),
        ("len($x)", "len($x)"),
        ("first($x)", "first($x)"),
        ("sum([1, $maybe])", "sum([1, $maybe])"),
        ("{$x: 1}", "{$x: 1}"),
    ] {
        faults(&mut host, source, Fault::Null, covers);
    }
    // Null spreads: each operation that meets it warns in turn.
    let (value, warnings) = run(&mut host, "($maybe + 1) * 2");
    assert_eq!(value, Value::Null);
    let covered: Vec<&str> = warnings
        .iter()
        .map(|w| w.span.slice("($maybe + 1) * 2"))
        .collect();
    assert_eq!(covered, ["($maybe + 1)", "($maybe + 1) * 2"]);
    assert!(
        warnings
            .iter()
            .all(|w| w.code() == codes::RUN_NULL_ARITHMETIC)
    );
    // The first warning is at the operation that met the null first,
    // and a fault of another kind is followed by the nulls it causes.
    let source = "$n * 2 + ($maybe - 1) * 4";
    let (_, warnings) = run(&mut host, source);
    assert_eq!(warnings.len(), 3);
    assert_eq!(warnings[0].span.slice(source), "($maybe - 1)");
    let source = "1 + 2 / ($n - 3)";
    let (value, warnings) = run(&mut host, source);
    assert_eq!(value, Value::Null);
    let found: Vec<(Fault, &str)> = warnings
        .iter()
        .map(|w| (w.fault, w.span.slice(source)))
        .collect();
    assert_eq!(
        found,
        [
            (Fault::DivisionByZero, "2 / ($n - 3)"),
            (Fault::Null, source)
        ]
    );
    // The compiler saw it coming and said so at load.
    let outcome = compile("$maybe + 1", host.registry(), &context(&host), &Type::Any);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, codes::NULL_IN_ARITHMETIC);
    assert_eq!(
        outcome.diagnostics[0].suggestion.as_deref(),
        Some("use `??` to give a value for when it is null")
    );
    // With `??` there is nothing to warn of.
    let (value, warnings) = run(&mut host, "($maybe ?? 4.5) + 1");
    assert_eq!(value, real(5.5));
    assert_eq!(warnings, []);
}

#[test]
fn null_row_3_a_comparison_with_null_is_false_except_the_null_tests() {
    let mut host = TestHost::new();
    host.set_local(slot::XS, ints(&[1, 2]));
    // `$maybe` and `$x` are null.
    for source in [
        "$maybe < 1",
        "$maybe <= 1",
        "$maybe > 1",
        "$maybe >= 1",
        "$maybe == 1",
        "$maybe != 1",
        "1 < $maybe",
        "1 != $maybe",
        "$x == 'a'",
        "$x != 'a'",
        "$x < 'a'",
        // Two nulls that are not written `null` compare false as well.
        "$maybe == $x",
        "$maybe != $x",
        // Membership is a comparison.
        "$maybe in $xs",
        "$x in [null]",
        "1 in $x",
        "self.var.mood == 'calm'",
        "self.var.mood != 'calm'",
    ] {
        let (value, warnings) = run(&mut host, source);
        assert_eq!(value, Value::Bool(false), "`{source}`");
        assert_eq!(warnings, [], "`{source}`");
    }
    // `== null` and `!= null` are the tests for it.
    for (source, wanted) in [
        ("$maybe == null", true),
        ("$maybe != null", false),
        ("null == $maybe", true),
        ("null != $maybe", false),
        ("null == null", true),
        ("null != null", false),
        ("$x == null", true),
        ("self.var.mood == null", true),
        ("$xs == null", false),
        ("$xs != null", true),
        ("self != null", true),
        ("1 == null", false),
        ("1 != null", true),
    ] {
        let (value, warnings) = run(&mut host, source);
        assert_eq!(value, Value::Bool(wanted), "`{source}`");
        assert_eq!(warnings, [], "`{source}`");
    }
    host.set_local(slot::MAYBE, real(0.5));
    assert_eq!(run(&mut host, "$maybe == null").0, Value::Bool(false));
    assert_eq!(run(&mut host, "$maybe != null").0, Value::Bool(true));
    assert_eq!(run(&mut host, "$maybe < 1").0, Value::Bool(true));
}

#[test]
fn null_row_4_a_condition_that_is_null_counts_as_false() {
    let mut host = TestHost::new();
    let context = context(&host);
    let condition = Type::Bool.nullable();
    let mut machine = Machine::new();
    // `$x` is null; `self.var.mood` was never set.
    for source in [
        "$x",
        "null",
        "self.var.mood",
        "$m['missing']",
        "target.target.var.kills",
    ] {
        let compiled = compile(source, host.registry(), &context, &Type::Any)
            .value
            .unwrap();
        assert_eq!(
            machine.eval(&compiled, &mut host),
            Value::Null,
            "`{source}`"
        );
        assert!(!machine.eval_condition(&compiled, &mut host), "`{source}`");
        assert_eq!(host.warnings, [], "`{source}`");
    }
    let alarm = compile(
        "here.site.var.alarm_raised",
        host.registry(),
        &context,
        &condition,
    )
    .into_result()
    .unwrap();
    assert!(!machine.eval_condition(&alarm, &mut host));
    // Inside an expression, null where a bool is wanted is false too.
    for (source, wanted) in [
        ("if $x then 1 else 2", int(2)),
        ("not $x", Value::Bool(true)),
        ("$x and true", Value::Bool(false)),
        ("$x or true", Value::Bool(true)),
        ("$x or $x", Value::Bool(false)),
        ("true and $x", Value::Bool(false)),
        ("not self.var.mood == 'calm'", Value::Bool(true)),
        // The example of 05 §2: a site whose alarm was never set.
        (
            "self.has_tag('guard') or not here.site.var.alarm_raised",
            Value::Bool(true),
        ),
    ] {
        let (value, warnings) = run(&mut host, source);
        assert_eq!(value, wanted, "`{source}`");
        assert_eq!(warnings, [], "`{source}`");
    }
    // A condition that is neither a bool nor null is false, with a
    // warning.
    host.set_local(slot::X, 3);
    let compiled = compile("$x", host.registry(), &context, &condition)
        .value
        .unwrap();
    host.warnings.clear();
    assert!(!machine.eval_condition(&compiled, &mut host));
    assert_eq!(host.warnings.len(), 1);
    assert_eq!(
        host.warnings[0].fault,
        Fault::Type {
            wanted: "a bool",
            found: "int"
        }
    );
    host.set_local(slot::X, true);
    assert!(machine.eval_condition(&compiled, &mut host));
}

#[test]
fn a_rule_never_fails_because_something_was_missing() {
    // Every binding gone: nothing panics, and what cannot be known is
    // null or false.
    let mut host = TestHost::new();
    host.subjects.clear();
    host.locals.clear();
    host.event.clear();
    host.rng = None;
    for source in [
        "self.mass * 2 > target.mass",
        "here.site.var.alarm_raised and self.has_tag('guard')",
        "$power + event.energy + $xs[0]",
        "the(self) + ' ' + verb(self, 'hit')",
        "dist(here, self.loc) < 5",
        "sum($xs) / len($xs)",
        "rand(0, 1) < 0.5 or chance(0.5)",
        "self.material == 'glass'",
        "$m[self.material] ?? 0",
    ] {
        let (value, _) = run(&mut host, source);
        assert!(
            !matches!(value, Value::Subject(_) | Value::Id(_)),
            "`{source}` gave {value}"
        );
    }
}

// ---------------------------------------------------------------------
// 05 §4.5: numbers and errors.

#[test]
fn numeric_errors_give_null_and_a_warning() {
    let mut host = TestHost::new();
    host.set_local(slot::N, 0);
    host.set_local(slot::POWER, real(-4.0));
    let domain = Fault::Domain;
    for (source, fault, covers) in [
        ("1 / $n", Fault::DivisionByZero, "1 / $n"),
        ("1.5 / $n", Fault::DivisionByZero, "1.5 / $n"),
        ("$n / 0.0", Fault::DivisionByZero, "$n / 0.0"),
        ("7 % $n", Fault::DivisionByZero, "7 % $n"),
        ("div(7, $n)", Fault::DivisionByZero, "div(7, $n)"),
        (
            "sqrt($power)",
            domain("the square root of a negative number"),
            "sqrt($power)",
        ),
        (
            "ln($n)",
            domain("the logarithm of a number that is not positive"),
            "ln($n)",
        ),
        (
            "log10($power)",
            domain("the logarithm of a number that is not positive"),
            "log10($power)",
        ),
        (
            "$n ^ -1",
            domain("the power is not defined for this base and exponent"),
            "$n ^ -1",
        ),
        (
            "$power ^ 0.5",
            domain("the power is not defined for this base and exponent"),
            "$power ^ 0.5",
        ),
        (
            "9223372036854775807 + (1 + $n)",
            Fault::Overflow,
            "9223372036854775807 + (1 + $n)",
        ),
        (
            "9223372036854775807 * (2 + $n)",
            Fault::Overflow,
            "9223372036854775807 * (2 + $n)",
        ),
        (
            "-9223372036854775807 - (2 + $n)",
            Fault::Overflow,
            "-9223372036854775807 - (2 + $n)",
        ),
        (
            "sum([9223372036854775807, 1 + $n])",
            Fault::Overflow,
            "sum([9223372036854775807, 1 + $n])",
        ),
    ] {
        faults(&mut host, source, fault, covers);
    }
    // The most negative int has no negation and no magnitude.
    host.set_local(slot::N, i64::MIN);
    faults(&mut host, "-$n", Fault::Overflow, "-$n");
    faults(&mut host, "abs($n)", Fault::Overflow, "abs($n)");
    faults(&mut host, "div($n, -1)", Fault::Overflow, "div($n, -1)");
    assert_eq!(run(&mut host, "$n % -1"), (int(0), vec![]));
    // Infinity is a number; what IEEE calls not-a-number is not.
    host.set_local(slot::POWER, real(f64::INFINITY));
    assert_eq!(run(&mut host, "$power + 1"), (real(f64::INFINITY), vec![]));
    assert_eq!(run(&mut host, "1 / $power"), (real(0.0), vec![]));
    let undefined = domain("the result is not defined");
    faults(&mut host, "$power - $power", undefined, "$power - $power");
    faults(&mut host, "$power * 0", undefined, "$power * 0");
    faults(&mut host, "$power / $power", undefined, "$power / $power");
    faults(
        &mut host,
        "floor($power)",
        domain("an infinite number has no whole part"),
        "floor($power)",
    );
    // Each kind of fault has its own code.
    assert_eq!(Fault::DivisionByZero.code(), codes::RUN_DIVISION_BY_ZERO);
    assert_eq!(undefined.code(), codes::RUN_DOMAIN);
    assert_eq!(Fault::Overflow.code(), codes::RUN_OVERFLOW);
}

#[test]
fn a_value_of_the_wrong_type_at_run_time_gives_null_and_a_warning() {
    let mut host = TestHost::new();
    let wrong = |wanted, found| Fault::Type { wanted, found };
    host.set_local(slot::X, "text");
    faults(
        &mut host,
        "$x + 1",
        wrong("two numbers, two texts or two lists", "string"),
        "$x + 1",
    );
    faults(&mut host, "$x * 2", wrong("a number", "string"), "$x * 2");
    faults(&mut host, "-$x", wrong("a number", "string"), "-$x");
    faults(
        &mut host,
        "sqrt($x)",
        wrong("a number", "string"),
        "sqrt($x)",
    );
    faults(
        &mut host,
        "$x[0]",
        wrong("a list or a map", "string"),
        "$x[0]",
    );
    faults(
        &mut host,
        "$x.field",
        wrong("a list or a map", "string"),
        "$x.field",
    );
    faults(
        &mut host,
        "$x.var.thing",
        wrong("a subject", "string"),
        "$x.var.thing",
    );
    faults(
        &mut host,
        "first($x)",
        wrong("a list", "string"),
        "first($x)",
    );
    faults(&mut host, "keys($x)", wrong("a map", "string"), "keys($x)");
    host.set_local(slot::X, real(1.5));
    faults(&mut host, "$x % 2", wrong("an int", "real"), "$x % 2");
    faults(
        &mut host,
        "div($x, 2)",
        wrong("an int", "real"),
        "div($x, 2)",
    );
    faults(
        &mut host,
        "[1, 2][$x]",
        wrong("an int", "real"),
        "[1, 2][$x]",
    );
    faults(
        &mut host,
        "len($x)",
        wrong("a list, a map or a text", "real"),
        "len($x)",
    );
    faults(
        &mut host,
        "{$x: 1}",
        wrong("a text or an id as a key", "real"),
        "{$x: 1}",
    );
    faults(&mut host, "cap($x)", wrong("a text", "real"), "cap($x)");
    host.set_local(slot::X, host.entity(5));
    faults(
        &mut host,
        "$x.mass",
        wrong("a list or a map", "subject"),
        "$x.mass",
    );

    // A comparison or a condition of the wrong type is false, with the
    // same warning.
    let mut expect_false = |source: &str, fault: Fault| {
        let (value, warnings) = run(&mut host, source);
        assert_eq!(value, Value::Bool(false), "`{source}`");
        assert_eq!(warnings.len(), 1, "`{source}`");
        assert_eq!(warnings[0].fault, fault, "`{source}`");
    };
    expect_false("$x < 1", wrong("two numbers or two texts", "subject"));
    expect_false("1 >= $x", wrong("two numbers or two texts", "subject"));
    expect_false("1 in $x", wrong("a list or a map", "subject"));
    expect_false("$x and true", wrong("a bool", "subject"));
    expect_false("$x or false", wrong("a bool", "subject"));
    expect_false("true and $x", wrong("a bool", "subject"));
    let (value, warnings) = run(&mut host, "not $x");
    assert_eq!(value, Value::Bool(true));
    assert_eq!(warnings.len(), 1);
    let (value, warnings) = run(&mut host, "if $x then 1 else 2");
    assert_eq!(value, int(2));
    assert_eq!(warnings[0].span.slice("if $x then 1 else 2"), "$x");
    // Equality between values of different types is simply false.
    assert_eq!(run(&mut host, "$x == 1"), (Value::Bool(false), vec![]));
    assert_eq!(run(&mut host, "$x != 1"), (Value::Bool(true), vec![]));
}

// ---------------------------------------------------------------------
// 05 §4.3: operators.

#[test]
fn operators_compute_what_the_specification_says() {
    let half_pi = core::f64::consts::FRAC_PI_2;
    for (source, wanted) in [
        // Arithmetic on ints stays int; an int meeting a real becomes
        // real.
        ("1 + 2", int(3)),
        ("7 - 10", int(-3)),
        ("6 * 7", int(42)),
        ("1 + 2.5", real(3.5)),
        ("2.5 * 2", real(5.0)),
        ("1 + 2 * 3", int(7)),
        ("(1 + 2) * 3", int(9)),
        ("-3 + 1", int(-2)),
        ("--3", int(3)),
        // `/` always gives a real.
        ("7 / 2", real(3.5)),
        ("6 / 3", real(2.0)),
        ("1 / 4.0", real(0.25)),
        // `%` and `div` are integer operations that round toward minus
        // infinity, so that div(a, b) * b + a % b == a.
        ("7 % 2", int(1)),
        ("-7 % 2", int(1)),
        ("7 % -2", int(-1)),
        ("-7 % -2", int(-1)),
        ("6 % 3", int(0)),
        ("div(7, 2)", int(3)),
        ("div(-7, 2)", int(-4)),
        ("div(7, -2)", int(-4)),
        ("div(-7, -2)", int(3)),
        ("div(-7, 2) * 2 + -7 % 2", int(-7)),
        // `^` always gives a real, groups to the right and binds tighter
        // than a minus in front.
        ("2 ^ 3", real(8.0)),
        ("2 ^ -1", real(0.5)),
        ("2 ^ 3 ^ 2", real(512.0)),
        ("-2 ^ 2", real(-4.0)),
        ("(-2) ^ 2", real(4.0)),
        ("1.5 ^ 2", real(2.25)),
        ("4 ^ 0.5", real(2.0)),
        ("0 ^ 0", real(1.0)),
        // `+` joins texts and lists.
        ("'ab' + 'cd'", text("abcd")),
        ("'' + ''", text("")),
        ("[1] + [2, 3]", ints(&[1, 2, 3])),
        ("[] + []", ints(&[])),
        // Comparisons.
        ("1 < 2", Value::Bool(true)),
        ("2 <= 2", Value::Bool(true)),
        ("1 > 2", Value::Bool(false)),
        ("2 >= 3", Value::Bool(false)),
        ("1 == 1.0", Value::Bool(true)),
        ("1 != 1.0", Value::Bool(false)),
        ("1 < 1.5", Value::Bool(true)),
        ("'a' < 'b'", Value::Bool(true)),
        ("'b' <= 'a'", Value::Bool(false)),
        ("'ash' == 'ash'", Value::Bool(true)),
        ("'ash' != 'iron'", Value::Bool(true)),
        ("[1, 2] == [1, 2.0]", Value::Bool(true)),
        ("[1, 2] == [1]", Value::Bool(false)),
        ("{'a': 1} == {'a': 1}", Value::Bool(true)),
        ("{'a': 1} != {'a': 2}", Value::Bool(true)),
        ("[null] == [null]", Value::Bool(true)),
        ("true == true", Value::Bool(true)),
        // Logic.
        ("true and false", Value::Bool(false)),
        ("true and true", Value::Bool(true)),
        ("false or true", Value::Bool(true)),
        ("false or false", Value::Bool(false)),
        ("not true", Value::Bool(false)),
        ("not false and true", Value::Bool(true)),
        ("not (false and true)", Value::Bool(true)),
        // Membership.
        ("2 in [1, 2, 3]", Value::Bool(true)),
        ("4 in [1, 2, 3]", Value::Bool(false)),
        ("2.0 in [1, 2, 3]", Value::Bool(true)),
        ("'a' in ['a', 'b']", Value::Bool(true)),
        ("'a' in {'a': 1}", Value::Bool(true)),
        ("'z' in {'a': 1}", Value::Bool(false)),
        ("1 in []", Value::Bool(false)),
        // `??`.
        ("null ?? 5", int(5)),
        ("1 ?? 5", int(1)),
        ("null ?? null ?? 3", int(3)),
        ("false ?? true", Value::Bool(false)),
        // Indexing counts from zero; out of range is null.
        ("[10, 20, 30][0]", int(10)),
        ("[10, 20, 30][2]", int(30)),
        ("[10, 20, 30][3]", Value::Null),
        ("[10, 20, 30][-1]", Value::Null),
        ("{'ash': 3, 'iron': 1}['ash']", int(3)),
        ("{'ash': 3}['iron']", Value::Null),
        ("{'ash': 3}.ash", int(3)),
        ("{'ash': 3}.iron", Value::Null),
        ("[[1, 2], [3]][1][0]", int(3)),
        // `if`.
        ("if 1 < 2 then 'yes' else 'no'", text("yes")),
        ("if false then 1 else if false then 2 else 3", int(3)),
        // Unit literals are reals in SI units.
        ("300ms", real(0.3)),
        ("2h", real(7200.0)),
        ("90 s", real(90.0)),
        ("70kPa", real(70_000.0)),
        ("1.5MJ + 500kJ", real(2_000_000.0)),
        ("250ms == 0.25", Value::Bool(true)),
        // Maths.
        ("min(3, 1, 2)", int(1)),
        ("max(3, 1, 2)", int(3)),
        ("min(1, 2.5)", real(1.0)),
        ("max(1, 2.5)", real(2.5)),
        ("min(2.0, 2)", real(2.0)),
        ("clamp(5, 0, 3)", int(3)),
        ("clamp(-1, 0, 3)", int(0)),
        ("clamp(2, 0, 3)", int(2)),
        ("clamp(1.5, 0, 1)", real(1.0)),
        ("clamp(5, 3, 0)", int(0)),
        ("abs(-3)", int(3)),
        ("abs(-3.5)", real(3.5)),
        ("sign(-3.5)", int(-1)),
        ("sign(0)", int(0)),
        ("sign(7)", int(1)),
        ("floor(2.7)", int(2)),
        ("floor(-2.5)", int(-3)),
        ("floor(7)", int(7)),
        ("ceil(2.1)", int(3)),
        ("ceil(-2.5)", int(-2)),
        ("round(2.5)", int(3)),
        ("round(-2.5)", int(-3)),
        ("round(2.4)", int(2)),
        ("sqrt(9)", real(3.0)),
        ("sqrt(2.25)", real(1.5)),
        ("exp(0)", real(1.0)),
        ("ln(1)", real(0.0)),
        ("log10(1)", real(0.0)),
        ("sin(0)", real(0.0)),
        ("cos(0)", real(1.0)),
        ("atan2(0, 1)", real(0.0)),
        ("atan2(1, 0)", real(half_pi)),
        ("lerp(0, 10, 0.5)", real(5.0)),
        ("lerp(2, 4, 2)", real(6.0)),
        // Lists, maps and text.
        ("len([1, 2, 3])", int(3)),
        ("len([])", int(0)),
        ("len('héllo')", int(5)),
        ("len({'a': 1, 'b': 2})", int(2)),
        ("contains([1, 2], 2)", Value::Bool(true)),
        ("contains([1, 2], 2.0)", Value::Bool(true)),
        ("contains([1, 2], 3)", Value::Bool(false)),
        ("contains([1, null], null)", Value::Bool(true)),
        ("contains('abc', 'bc')", Value::Bool(true)),
        ("contains('abc', 'x')", Value::Bool(false)),
        ("first([4, 5])", int(4)),
        ("last([4, 5])", int(5)),
        ("first([])", Value::Null),
        ("last([])", Value::Null),
        ("sum([1, 2, 3])", int(6)),
        ("sum([1, 2.5])", real(3.5)),
        ("sum([])", int(0)),
        ("str(42)", text("42")),
        ("str(1.5)", text("1.5")),
        ("str(3.0)", text("3")),
        ("str(true)", text("true")),
        ("str(null)", text("null")),
        ("str('ash')", text("ash")),
        ("str([1, 'a', null])", text("[1, a, null]")),
        ("str({'b': 1, 'a': 2})", text("{a: 2, b: 1}")),
        ("'n = ' + str(3)", text("n = 3")),
        ("without([1, 2, 1, 3], 1)", ints(&[2, 3])),
        ("without([1, 2], 5)", ints(&[1, 2])),
        ("cap('ash')", text("Ash")),
        ("cap('')", text("")),
        ("cap('élan vital')", text("Élan vital")),
        // Types.
        ("is_null(null)", Value::Bool(true)),
        ("is_null(0)", Value::Bool(false)),
        ("is_null([])", Value::Bool(false)),
    ] {
        assert_eq!(value_of(source), wanted, "`{source}`");
    }
}

#[test]
fn and_or_and_if_evaluate_only_what_they_need() {
    let mut host = TestHost::new();
    host.set_local(slot::FLAG, false);
    // `secret()` is logged by the host when it is called.
    for (source, calls) in [
        ("$flag and secret() > 0", 0),
        ("not $flag and secret() > 0", 1),
        ("not $flag or secret() > 0", 0),
        ("$flag or secret() > 0", 1),
        ("if $flag then secret() else 1", 0),
        ("if not $flag then secret() else 1", 1),
        ("1 ?? secret()", 0),
        ("$x ?? secret()", 1),
        ("secret() ?? secret()", 1),
    ] {
        run(&mut host, source);
        assert_eq!(host.log.len(), calls, "`{source}`: {:?}", host.log);
    }
    // Arguments are evaluated left to right, the object first.
    run(&mut host, "dist(self.loc, here) + core:double(secret())");
    assert_eq!(
        host.log,
        [
            "subject#0:1.loc",
            "dist(null,subject#1:100)",
            "secret()",
            "core:double(42)"
        ]
    );
}

// ---------------------------------------------------------------------
// Subjects, properties, variables, events, host functions.

fn populated() -> TestHost {
    let mut host = TestHost::new();
    let (me, them, here) = (host.entity(1), host.entity(2), host.place(100));
    let site = host.site(9);
    host.set(me, "mass", real(70.0));
    host.set(me, "name", "Mara");
    host.set(me, "loc", here);
    host.set(me, "target", them);
    host.set(me, "tags", Value::list([text("guard"), text("tall")]));
    host.set(me, "material", host.material("core:glass"));
    host.set(
        me,
        "materials",
        Value::list([host.material("core:glass"), host.material("core:iron")]),
    );
    host.set(
        me,
        "stats",
        Value::map([("speed", real(4.0)), ("reach", real(1.5))]),
    );
    host.set(me, "part(arm)", host.part(7));
    host.set(host.part(7), "size", real(0.5));
    host.set(them, "mass", real(90.0));
    host.set(them, "name", "the guard");
    host.set(them, "loc", host.place(104));
    host.set(them, "material", host.material("ashlands:slag"));
    host.set(here, "site", site);
    host.set(here, "x", 12);
    host.set(here, "field(fire)", real(3.0));
    host.set(here, "material", host.material("core:iron"));
    host.set(site, "name", "the vault");
    host.set_var(me, "core:kills", 4);
    host.set_var(site, "core:alarm_raised", true);
    host.set_local(slot::POWER, real(2.0));
    host.set_local(slot::N, 3);
    host.set_local(slot::XS, ints(&[5, 6, 7]));
    host.set_local(slot::NAME, "core:glass");
    host.set_local(
        slot::M,
        Value::map([("core:glass", real(2.5)), ("k", real(1.0))]),
    );
    host.set_local(slot::FLAG, true);
    host.set_event(slot::ENERGY, real(5000.0));
    host.set_event(slot::KIND, "blast");
    host.set_event(slot::AMOUNT, 2);
    host.set_event(slot::SOURCE, them);
    host
}

#[test]
fn expressions_read_the_host() {
    let mut host = populated();
    let me = Value::Subject(host.entity(1));
    for (source, wanted) in [
        ("self", me.clone()),
        ("self == self", Value::Bool(true)),
        ("self == target", Value::Bool(false)),
        ("self.target == target", Value::Bool(true)),
        ("event.source == target", Value::Bool(true)),
        ("self.mass", real(70.0)),
        ("self.mass * 2 > target.mass", Value::Bool(true)),
        ("self.target.mass", real(90.0)),
        ("self.loc.site.name", text("the vault")),
        ("self.loc == here", Value::Bool(true)),
        ("here.x + 1", int(13)),
        ("here.field('fire')", real(3.0)),
        ("here.field('ice')", Value::Null),
        ("self.has_tag('guard')", Value::Bool(true)),
        ("self.has_tag('thief')", Value::Bool(false)),
        ("'tall' in self.tags", Value::Bool(true)),
        ("len(self.tags)", int(2)),
        ("self.part('arm').size", real(0.5)),
        ("self.part('leg').size", Value::Null),
        ("self.stats.speed", real(4.0)),
        ("self.stats['reach'] * 2", real(3.0)),
        ("sum(values(self.stats))", real(5.5)),
        // Variables: set, declared and unset (the default), undeclared.
        ("self.var.kills", int(4)),
        ("self.var.kills * 2 >= $n", Value::Bool(true)),
        ("target.var.kills", int(0)),
        ("self.var('core:kills')", int(4)),
        ("self.var('ashlands:kills')", int(0)),
        ("here.site.var.alarm_raised", Value::Bool(true)),
        ("self.var.mood ?? 'calm'", text("calm")),
        // Locals and event fields.
        ("$power * $n", real(6.0)),
        ("$xs[1] + $xs[2]", int(13)),
        ("event.energy / event.amount", real(2500.0)),
        ("event.kind == 'blast'", Value::Bool(true)),
        // Host functions.
        ("dist(here, target.loc)", real(4.0)),
        ("dist(self.loc, here) == 0", Value::Bool(true)),
        ("secret() + 1", int(43)),
        ("double(21)", int(42)),
        ("core:double(1.5)", real(3.0)),
        (
            "ashlands:fall_damage(2)",
            Value::Real(Real::of(70.0) * Real::of(9.806_65) * Real::of(2.0)),
        ),
        ("ashlands:fall_damage(2, 1) > 19.6", Value::Bool(true)),
        ("exists(target)", Value::Bool(true)),
        ("exists($x)", Value::Bool(false)),
    ] {
        let (value, warnings) = run(&mut host, source);
        assert_eq!(value, wanted, "`{source}`");
        assert_eq!(warnings, [], "`{source}`");
    }
}

#[test]
fn ids_compare_by_definition_and_print_as_their_qualified_id() {
    let mut host = populated();
    for (source, wanted) in [
        ("self.material == 'glass'", Value::Bool(true)),
        ("self.material == 'core:glass'", Value::Bool(true)),
        ("self.material != 'glass'", Value::Bool(false)),
        ("self.material == 'iron'", Value::Bool(false)),
        ("'glass' == self.material", Value::Bool(true)),
        ("target.material == 'ashlands:slag'", Value::Bool(true)),
        ("self.material == target.material", Value::Bool(false)),
        ("self.material == self.material", Value::Bool(true)),
        ("self.material in ['iron', 'glass']", Value::Bool(true)),
        ("self.material in ['iron', 'ash']", Value::Bool(false)),
        ("'iron' in self.materials", Value::Bool(true)),
        ("'ash' in self.materials", Value::Bool(false)),
        ("here.material in self.materials", Value::Bool(true)),
        ("self.materials == ['glass', 'iron']", Value::Bool(true)),
        (
            "(if $flag then self.material else 'iron') == 'glass'",
            Value::Bool(true),
        ),
        // A text not known at load is compared by qualified id.
        ("self.material == $name", Value::Bool(true)),
        ("target.material == $name", Value::Bool(false)),
        ("self.material != $name", Value::Bool(false)),
        // As text, an id is its qualified id.
        ("str(self.material)", text("core:glass")),
        (
            "'made of ' + str(target.material)",
            text("made of ashlands:slag"),
        ),
        // As a map key too.
        ("$m[self.material]", real(2.5)),
        ("$m[target.material]", Value::Null),
        ("has($m, self.material)", Value::Bool(true)),
        ("self.material in $m", Value::Bool(true)),
        (
            "keys({self.material: 1})",
            Value::list([text("core:glass")]),
        ),
        (
            "without_key($m, self.material)",
            Value::map([("k", real(1.0))]),
        ),
    ] {
        let (value, warnings) = run(&mut host, source);
        assert_eq!(value, wanted, "`{source}`");
        assert_eq!(warnings, [], "`{source}`");
    }
    // The id itself is a value the host can take back.
    let (value, _) = run(&mut host, "self.material ?? 'iron'");
    assert_eq!(value, host.material("core:glass"));
    let (value, _) = run(&mut host, "self.part('leg').size ?? 0");
    assert_eq!(value, int(0));
}

#[test]
fn maps_are_values_ordered_by_key() {
    for (source, wanted) in [
        (
            "keys({'b': 1, 'a': 2, 'c': 3})",
            Value::list([text("a"), text("b"), text("c")]),
        ),
        ("values({'b': 1, 'a': 2, 'c': 3})", ints(&[2, 1, 3])),
        ("has({'a': 1}, 'a')", Value::Bool(true)),
        ("has({'a': 1}, 'b')", Value::Bool(false)),
        ("has({}, 'a')", Value::Bool(false)),
        ("{'ash': 3, 'iron': 1}['iron']", int(1)),
        ("{'ash': 3}['ash'] + 1", int(4)),
        ("len({})", int(0)),
        ("keys({})", ints(&[])),
        (
            "without_key({'a': 1, 'b': 2}, 'a')",
            Value::map([("b", int(2))]),
        ),
        ("without_key({'a': 1}, 'z')", Value::map([("a", int(1))])),
        // A key written twice keeps its last value.
        ("{'a': 1, 'a': 2}", Value::map([("a", int(2))])),
        ("{'a': {'b': [1, 2]}}.a.b[1]", int(2)),
        ("sum(values({'a': 1.5, 'b': 2}))", real(3.5)),
    ] {
        assert_eq!(value_of(source), wanted, "`{source}`");
    }
    // Iteration order is by key whatever order the map was written or
    // built in, which the state hash shows.
    let forward = value_of("{'a': 1, 'b': 2, 'c': 3}");
    let backward = value_of("{'c': 3, 'b': 2, 'a': 1}");
    let mut host = TestHost::new();
    host.set_local(slot::NAME, "b");
    let (built, _) = run(&mut host, "{'c': 3, $name: 2, 'a': 1}");
    assert_eq!(forward, backward);
    assert_eq!(forward, built);
    assert_eq!(Hash128::of(&forward), Hash128::of(&backward));
    assert_eq!(Hash128::of(&forward), Hash128::of(&built));
    assert_ne!(
        Hash128::of(&forward),
        Hash128::of(&value_of("{'a': 1, 'b': 2, 'c': 4}"))
    );
    assert_eq!(
        value_of("str({'c': 3, 'b': 2, 'a': 1})"),
        text("{a: 1, b: 2, c: 3}")
    );
}

// ---------------------------------------------------------------------
// Random functions.

#[test]
fn random_functions_draw_from_the_supplied_stream() {
    let mut host = TestHost::new();
    host.set_local(slot::XS, ints(&[10, 20, 30, 40]));
    host.rng = Some(Rng::from_seed(2024));
    let mut expected = Rng::from_seed(2024);

    let (value, _) = run(&mut host, "rand(2, 12)");
    let unit = expected.unit();
    assert_eq!(value, Value::Real(Real::of(2.0) + Real::of(10.0) * unit));
    let (value, _) = run(&mut host, "rand_int(1, 6)");
    assert_eq!(value, int(expected.range(1, 6)));
    let (value, _) = run(&mut host, "chance(0.5)");
    assert_eq!(value, Value::Bool(expected.chance(Real::of(0.5))));
    let (value, _) = run(&mut host, "pick($xs)");
    assert_eq!(value, int(*expected.pick(&[10, 20, 30, 40]).unwrap()));
    // A function the host draws for uses the same stream.
    let (value, _) = run(&mut host, "roll(20)");
    assert_eq!(value, int(expected.range(1, 20)));
    // Draws happen in the order the expression is written.
    let (value, _) = run(
        &mut host,
        "[rand_int(1, 100), rand_int(1, 100), rand_int(1, 100)]",
    );
    let wanted: Vec<i64> = (0..3).map(|_| expected.range(1, 100)).collect();
    assert_eq!(value, ints(&wanted));
    assert_eq!(host.rng.as_ref(), Some(&expected));

    // The same seed gives the same answers.
    let mut again = TestHost::new();
    again.rng = Some(Rng::from_seed(2024));
    assert_eq!(run(&mut again, "rand(2, 12)").0, {
        let mut rng = Rng::from_seed(2024);
        Value::Real(Real::of(2.0) + Real::of(10.0) * rng.unit())
    });

    // Nothing is drawn where the answer needs no draw or cannot be had.
    let before = host.rng.clone();
    assert_eq!(run(&mut host, "pick([])"), (Value::Null, vec![]));
    assert_eq!(
        run(&mut host, "false and chance(0.5)"),
        (Value::Bool(false), vec![])
    );
    faults(&mut host, "rand($maybe, 1)", Fault::Null, "rand($maybe, 1)");
    assert_eq!(host.rng, before);
    // Limits that leave no room give the lower one.
    assert_eq!(run(&mut host, "rand_int(5, 1)").0, int(5));
    assert!(matches!(run(&mut host, "chance(1)").0, Value::Bool(true)));
    assert!(matches!(run(&mut host, "chance(0)").0, Value::Bool(false)));

    // With no stream, a random function is null with a warning.
    host.rng = None;
    for source in ["rand(0, 1)", "rand_int(1, 6)", "chance(0.5)", "pick($xs)"] {
        faults(&mut host, source, Fault::NoRandom, source);
    }
    assert_eq!(Fault::NoRandom.code(), codes::RUN_NO_RANDOM);
}

// ---------------------------------------------------------------------
// Text.

/// A sink that keeps the two kinds of text apart.
#[derive(Default)]
struct Pieces(Vec<(bool, String)>);

impl TextSink for Pieces {
    fn literal(&mut self, text: &str) {
        self.0.push((false, text.to_owned()));
    }

    fn value(&mut self, text: &str) {
        self.0.push((true, text.to_owned()));
    }
}

/// Renders a template both ways and checks they agree.
#[track_caller]
fn render(host: &mut TestHost, source: &str) -> String {
    let context = context(host);
    let checked = check_template(source, host.registry(), &context);
    assert!(
        !checked.has_errors(),
        "`{source}`: {:#?}",
        checked.diagnostics
    );
    let checked = checked.value.unwrap();
    let compiled = checked.compile().into_result().unwrap();
    let mut walked = String::new();
    checked.interpret(&mut host.clone(), &Limits::default(), &mut walked);
    let rendered = Machine::new().render_to_string(&compiled, host);
    assert_eq!(rendered, walked, "`{source}`");
    rendered
}

#[test]
fn templates_render_for_a_reader() {
    let mut host = populated();
    for (source, wanted) in [
        ("plain text", "plain text"),
        ("", ""),
        ("{{braces}} stay", "{braces} stay"),
        ("{self.mass}", "70"),
        ("{self.mass:.2} kg", "70.00 kg"),
        ("{event.energy / 3:.1} J", "1666.7 J"),
        ("{event.energy:.0}", "5000"),
        ("{$n:.2}", "3.00"),
        ("{$n:.0}", "3"),
        ("{$n} of {len($xs)}", "3 of 3"),
        ("{1 / 3:.3}", "0.333"),
        // A format applies to numbers and leaves the rest alone.
        ("{self.name:.2}", "Mara"),
        ("{$x}", "null"),
        ("{$flag}", "true"),
        ("{self.material}", "core:glass"),
        ("{$xs}", "[5, 6, 7]"),
        ("{self.materials}", "[core:glass, core:iron]"),
        // The example of 05 §4.6, to the one doing it…
        (
            "{the(self)} {verb(self, 'drive')} {their(self)} blade into {the(target)}.",
            "you drive your blade into the the guard.",
        ),
        // …and helpers for someone else.
        (
            "{cap(name(target))} {verb(target, 'hit')} {them(self)}.",
            "The guard hits you.",
        ),
        (
            "{they(target)} / {themself(target)} / {a(target)}",
            "they / themself / a the guard",
        ),
        (
            "{count(3, 'arrow')}, {count(1, 'arrow')}, {plural('coin')}",
            "3 arrows, 1 arrow, coins",
        ),
        (
            "{list([self, target, 'a rope'])}",
            "you, the guard and a rope",
        ),
        (
            "<blood>{the(self)} {verb(self, 'bleed')}.</>",
            "<blood>you bleed.</>",
        ),
    ] {
        assert_eq!(render(&mut host, source), wanted, "`{source}`");
    }
    // A fault inside a template shows as null and warns like any other.
    host.warnings.clear();
    assert_eq!(render(&mut host, "{1 / ($n - 3)} left"), "null left");
    assert_eq!(host.warnings.len(), 1);
    assert_eq!(host.warnings[0].fault, Fault::DivisionByZero);
    assert_eq!(host.warnings[0].span, Span::new(1, 13));
}

#[test]
fn a_sink_is_told_which_text_was_written_and_which_was_computed() {
    let mut host = populated();
    host.set(host.entity(2), "name", "<b>Eve</>");
    let context = context(&host);
    let compiled = compile_template(
        "<dim>{name(target)}</> has {2 + 2} and {'text'}.",
        host.registry(),
        &context,
    )
    .into_result()
    .unwrap();
    let mut pieces = Pieces::default();
    Machine::new().render(&compiled, &mut host, &mut pieces);
    assert_eq!(
        pieces.0,
        [
            (false, "<dim>".to_owned()),
            (true, "<b>Eve</>".to_owned()),
            (false, "</> has ".to_owned()),
            (true, "4".to_owned()),
            // A constant text joins the text around it.
            (false, " and text.".to_owned()),
        ]
    );
    // A template with nothing left to evaluate is a constant.
    let constant = |source: &str| {
        compile_template(source, host.registry(), &context)
            .into_result()
            .unwrap()
            .constant()
            .map(str::to_owned)
    };
    assert_eq!(constant("plain"), Some("plain".to_owned()));
    assert_eq!(constant(""), Some(String::new()));
    assert_eq!(constant("a {'b' + 'c'} d"), Some("a bc d".to_owned()));
    assert_eq!(constant("{self.name}"), None);
    assert_eq!(constant("{1 + 1}"), None);
}

// ---------------------------------------------------------------------
// Folding and bytecode.

#[test]
fn constants_are_folded_at_load() {
    let host = TestHost::new();
    let context = context(&host);
    let compile_any = |source: &str| {
        let outcome = compile(source, host.registry(), &context, &Type::Any);
        assert!(
            !outcome.has_errors(),
            "`{source}`: {:?}",
            outcome.diagnostics
        );
        outcome.value.unwrap()
    };
    for (source, wanted) in [
        ("2 + 3 * 4", int(14)),
        ("300ms * 2", real(0.6)),
        ("'a' + 'b' + 'c'", text("abc")),
        ("[1, 2] + [3]", ints(&[1, 2, 3])),
        ("{'a': 1 + 1}", Value::map([("a", int(2))])),
        ("{'a': 1}['a']", int(1)),
        ("[1, 2, 3][1]", int(2)),
        ("clamp(5, 0, 3) + len('abc')", int(6)),
        ("sqrt(16) > 3", Value::Bool(true)),
        ("not (1 < 2)", Value::Bool(false)),
        ("1 in [1, 2]", Value::Bool(true)),
        ("null == null", Value::Bool(true)),
        ("null ?? 4", int(4)),
        ("3 ?? self.mass", int(3)),
        ("if 1 < 2 then 'a' else 'b'", text("a")),
        // A branch that is never taken is dropped, with whatever it
        // would have read or drawn.
        ("false and chance(0.5)", Value::Bool(false)),
        ("true or secret() > 0", Value::Bool(true)),
        ("if false then rand(0, 1) else 7", int(7)),
        ("-(-3)", int(3)),
        ("str(1.5) + cap('x')", text("1.5X")),
    ] {
        let compiled = compile_any(source);
        assert_eq!(compiled.constant(), Some(&wanted), "`{source}`");
        assert_eq!(compiled.len(), 1, "`{source}`");
        assert!(!compiled.is_empty());
    }
    // What needs the host is left for run time, and the constant parts
    // around it are still folded.
    for (source, operations) in [
        ("self.mass", 2),
        ("self.mass * (2 + 3)", 4),
        ("$n + 1 * 1", 3),
        ("if true then $n else secret()", 1),
        ("null ?? $n", 1),
        // Random draws and host functions are never folded.
        ("rand(0, 1)", 3),
        ("secret()", 1),
        ("chance(1)", 2),
        // Nor is anything that needs the name of an id.
        ("str(self.material ?? 'glass')", 5),
    ] {
        let compiled = compile_any(source);
        assert_eq!(compiled.constant(), None, "`{source}`");
        assert_eq!(compiled.len(), operations, "`{source}`");
    }
    // An operation that always fails is not folded away: it warns at
    // load and again each time it runs.
    let mut host = TestHost::new();
    let outcome = compile("1 / 0", host.registry(), &context, &Type::Any);
    assert_eq!(outcome.diagnostics[0].code, codes::ALWAYS_FAILS);
    let compiled = outcome.value.unwrap();
    assert_eq!(compiled.constant(), None);
    assert_eq!(Machine::new().eval(&compiled, &mut host), Value::Null);
    assert_eq!(host.warnings.len(), 1);
    assert_eq!(host.warnings[0].fault, Fault::DivisionByZero);
}

#[test]
fn a_compiled_expression_says_what_it_reads() {
    let host = TestHost::new();
    let context = context(&host);
    let compiled = compile(
        "self.mass + target.mass > here.site.var.alarm_raised ?? self.var.kills + self.loc.x",
        host.registry(),
        &context,
        &Type::Any,
    )
    .value
    .unwrap();
    let registry = host.registry();
    let names: Vec<String> = compiled
        .properties_read()
        .into_iter()
        .map(|id| registry.property_signature(registry.property(id).unwrap()))
        .collect();
    assert_eq!(
        names,
        [
            "entity.mass -> real",
            "loc.site -> site",
            "entity.loc -> loc",
            "loc.x -> int"
        ]
    );
    assert_eq!(
        compiled.variables_read(),
        ["core:alarm_raised", "core:kills"]
    );
    assert_eq!(compiled.origin(), ORIGIN);
    assert_eq!(compiled.ty(), &Type::Bool);
    let condition = compile("self.mass > 1", registry, &context, &Type::Bool)
        .value
        .unwrap();
    assert_eq!(condition.ty(), &Type::Bool);
}

#[test]
fn compiled_expressions_are_shared_and_cross_threads() {
    fn assert_send_sync<T: Send + Sync + Clone>() {}
    assert_send_sync::<Compiled>();
    assert_send_sync::<CompiledText>();
    assert_send_sync::<Registry>();
    assert_send_sync::<Context>();
    assert_send_sync::<Value>();

    let mut host = populated();
    let context = context(&host);
    let compiled = compile("self.mass * $n", host.registry(), &context, &Type::Real)
        .value
        .unwrap();
    let copy = compiled.clone();
    // A clone shares the code, and another thread can run it.
    let answer = std::thread::spawn(move || {
        let mut host = populated();
        Machine::new().eval(&copy, &mut host)
    })
    .join()
    .unwrap();
    assert_eq!(answer, real(210.0));
    assert_eq!(Machine::new().eval(&compiled, &mut host), real(210.0));
}

#[test]
fn one_machine_evaluates_many_expressions() {
    let mut host = populated();
    let context = context(&host);
    let mut machine = Machine::default();
    let sources = [
        "self.mass * 2",
        "$xs + $xs",
        "if $flag then 'a' else 'b'",
        "1 / ($n - 3)",
    ];
    let compiled: Vec<Compiled> = sources
        .iter()
        .map(|source| {
            compile(source, host.registry(), &context, &Type::Any)
                .value
                .unwrap()
        })
        .collect();
    for _ in 0..3 {
        assert_eq!(machine.eval(&compiled[0], &mut host), real(140.0));
        assert_eq!(
            machine.eval(&compiled[1], &mut host),
            ints(&[5, 6, 7, 5, 6, 7])
        );
        assert_eq!(machine.eval(&compiled[2], &mut host), text("a"));
        assert_eq!(machine.eval(&compiled[3], &mut host), Value::Null);
    }
    assert_eq!(host.warnings.len(), 3);
}

#[test]
fn values_are_kept_within_the_size_limit() {
    let mut host = populated();
    let context = context(&host);
    let limits = Limits { max_value_size: 5 };
    let mut machine = Machine::with_limits(limits);
    assert_eq!(machine.limits(), limits);
    assert_eq!(
        Limits::default().max_value_size,
        clash_expr::DEFAULT_MAX_VALUE_SIZE
    );
    for (source, wanted) in [
        ("$xs + $xs", None),
        ("$xs + [1, 2]", Some(ints(&[5, 6, 7, 1, 2]))),
        ("$name + $name", None),
        ("'ab' + str($n)", Some(text("ab3"))),
        ("[$n, $n, $n, $n, $n, $n]", None),
        ("[$n, $n]", Some(ints(&[3, 3]))),
        ("str($name)", None),
    ] {
        let compiled = compile(source, host.registry(), &context, &Type::Any)
            .value
            .unwrap();
        let checked = check(source, host.registry(), &context, &Type::Any)
            .value
            .unwrap();
        host.warnings.clear();
        let value = machine.eval(&compiled, &mut host);
        assert_eq!(
            checked.interpret(&mut host.clone(), &limits),
            value,
            "`{source}`"
        );
        match wanted {
            Some(wanted) => {
                assert_eq!(value, wanted, "`{source}`");
                assert_eq!(host.warnings, [], "`{source}`");
            }
            None => {
                assert_eq!(value, Value::Null, "`{source}`");
                assert_eq!(host.warnings.len(), 1, "`{source}`");
                assert_eq!(host.warnings[0].fault, Fault::TooLarge, "`{source}`");
                assert_eq!(host.warnings[0].code(), codes::RUN_TOO_LARGE);
            }
        }
    }
    machine.set_limits(Limits::default());
    let compiled = compile("$xs + $xs", host.registry(), &context, &Type::Any)
        .value
        .unwrap();
    assert_eq!(
        machine.eval(&compiled, &mut host),
        ints(&[5, 6, 7, 5, 6, 7])
    );
}

#[test]
fn run_time_warnings_are_rate_limited_per_site() {
    let mut host = populated();
    host.set_local(slot::N, 0);
    let context = context(&host);
    let source = "1 / $n + 2 / $n";
    let compiled = compile(source, host.registry(), &context, &Type::Any)
        .value
        .unwrap();
    let mut machine = Machine::new();
    let mut limiter = WarningLimiter::new();
    let mut reported: Vec<(String, u64)> = Vec::new();
    for _ in 0..1000 {
        host.warnings.clear();
        machine.eval(&compiled, &mut host);
        // Each run raises three: two divisions and the sum of two nulls.
        assert_eq!(host.warnings.len(), 3);
        for warning in &host.warnings {
            if let Some(count) = limiter.admit(warning) {
                reported.push((warning.span.slice(source).to_owned(), count));
            }
        }
    }
    // Three sites, each let through three times and then at 4, 8, …, 512.
    assert_eq!(limiter.sites(), 3);
    assert_eq!(reported.len(), 3 * 11);
    assert_eq!(limiter.suppressed(), 3000 - 33);
    let first: Vec<&(String, u64)> = reported.iter().take(3).collect();
    assert_eq!(
        first,
        [
            &("1 / $n".to_owned(), 1),
            &("2 / $n".to_owned(), 1),
            &("1 / $n + 2 / $n".to_owned(), 1)
        ]
    );
    assert_eq!(reported.last(), Some(&("1 / $n + 2 / $n".to_owned(), 512)));
    // A warning names its origin, its position and its code.
    let warning = host.warnings[0];
    assert_eq!(warning.origin, ORIGIN);
    assert_eq!(warning.span, Span::new(0, 6));
    assert_eq!(warning.to_string(), "warning[X902]: division by zero");
}

#[test]
fn the_deepest_expression_the_parser_allows_compiles_and_runs() {
    // On the default stack of a test thread, in a debug build.
    let mut host = populated();
    let context = context(&host);
    let depth = clash_expr::MAX_TREE_HEIGHT as usize - 1;
    for source in [
        vec!["$n"; depth].join(" + "),
        vec!["$flag"; depth].join(" and "),
        format!("$n{}", " ?? $n".repeat(depth - 1)),
        format!("self{}", ".target".repeat(depth - 1)),
        vec!["'a'"; depth].join(" + "),
    ] {
        let outcome = compile(&source, host.registry(), &context, &Type::Any);
        assert!(!outcome.has_errors(), "{:?}", outcome.diagnostics.first());
        run(&mut host, &source);
    }
    let brackets = clash_expr::MAX_NESTING as usize - 1;
    let source = format!("{}$n * 2{}", "(".repeat(brackets), ")".repeat(brackets));
    assert_eq!(run(&mut host, &source).0, int(6));
    let source = format!("{}$n{}", "[".repeat(brackets), "]".repeat(brackets));
    run(&mut host, &source);
    let source = format!(
        "{}1{}",
        "if $flag then ".repeat(brackets),
        " else 0".repeat(brackets)
    );
    assert_eq!(run(&mut host, &source).0, int(1));
    // A call with more arguments than a call may have is refused.
    let many = format!(
        "max({})",
        vec!["$n"; clash_expr::MAX_ARGUMENTS + 1].join(", ")
    );
    let outcome = compile(&many, host.registry(), &context, &Type::Any);
    assert_eq!(outcome.diagnostics[0].code, codes::ARGUMENT_COUNT);
    assert!(outcome.value.is_none());
    let most = format!("max({})", vec!["$n"; clash_expr::MAX_ARGUMENTS].join(", "));
    assert_eq!(run(&mut host, &most).0, int(3));
}

// ---------------------------------------------------------------------
// The machine against the tree, on random expressions.

/// Builds random expressions that pass the checker. Each function gives
/// an expression of one static type; `$x`, whose type is `any`, is how
/// values of the wrong type reach operators at run time.
struct Maker<'a> {
    rng: &'a mut Rng,
}

impl Maker<'_> {
    fn pick<'s>(&mut self, options: &[&'s str]) -> &'s str {
        options[self.rng.below(options.len() as u64) as usize]
    }

    fn roll(&mut self, sides: u64) -> u64 {
        self.rng.below(sides)
    }

    fn int(&mut self, depth: u32) -> String {
        if depth == 0 {
            return self
                .pick(&[
                    "0",
                    "1",
                    "2",
                    "7",
                    "100",
                    "9223372036854775807",
                    "$n",
                    "self.var.kills",
                    "event.amount",
                    "len($xs)",
                    "here.x",
                ])
                .to_owned();
        }
        let d = depth - 1;
        match self.roll(18) {
            0 => format!("({} + {})", self.int(d), self.int(d)),
            1 => format!("({} - {})", self.int(d), self.int(d)),
            2 => format!("({} * {})", self.int(d), self.int(d)),
            3 => format!("({} % {})", self.int(d), self.int(d)),
            4 => format!("div({}, {})", self.int(d), self.int(d)),
            5 => format!("-{}", self.int(d)),
            6 => format!("min({}, {})", self.int(d), self.int(d)),
            7 => format!("max({}, {}, {})", self.int(d), self.int(d), self.int(d)),
            8 => format!("abs({})", self.int(d)),
            9 => format!("floor({})", self.real(d)),
            10 => format!("round({})", self.real(d)),
            11 => format!("sign({})", self.number(d)),
            12 => format!(
                "(if {} then {} else {})",
                self.boolean(d),
                self.int(d),
                self.int(d)
            ),
            13 => format!("clamp({}, {}, {})", self.int(d), self.int(d), self.int(d)),
            14 => format!("sum({})", self.list(d)),
            15 => format!("len({})", self.list(d)),
            16 => format!("({} ?? {})", self.maybe_int(d), self.int(d)),
            _ => format!("rand_int({}, {})", self.int(d), self.int(d)),
        }
    }

    /// An int that may be null.
    fn maybe_int(&mut self, depth: u32) -> String {
        match self.roll(5) {
            0 => format!("first({})", self.list(depth)),
            1 => format!("{}[{}]", self.list(depth), self.int(depth)),
            2 => format!("pick({})", self.list(depth)),
            3 => "target.var.kills".to_owned(),
            _ => format!("last({})", self.list(depth)),
        }
    }

    fn real(&mut self, depth: u32) -> String {
        if depth == 0 {
            return self
                .pick(&[
                    "0.0",
                    "1.5",
                    "2.5",
                    "0.25",
                    "1e300",
                    "300ms",
                    "$power",
                    "self.mass",
                    "event.energy",
                ])
                .to_owned();
        }
        let d = depth - 1;
        match self.roll(16) {
            0 => format!("({} + {})", self.real(d), self.number(d)),
            1 => format!("({} - {})", self.number(d), self.real(d)),
            2 => format!("({} * {})", self.real(d), self.number(d)),
            3 => format!("({} / {})", self.number(d), self.number(d)),
            4 => format!("({} ^ {})", self.number(d), self.number(d)),
            5 => format!("sqrt({})", self.number(d)),
            6 => format!("ln({})", self.real(d)),
            7 => format!("exp({})", self.real(d)),
            8 => format!("sin({}) + cos({})", self.real(d), self.real(d)),
            9 => format!(
                "lerp({}, {}, {})",
                self.real(d),
                self.number(d),
                self.real(d)
            ),
            10 => format!(
                "(if {} then {} else {})",
                self.boolean(d),
                self.real(d),
                self.real(d)
            ),
            11 => format!("rand({}, {})", self.real(d), self.real(d)),
            12 => format!("({} ?? {})", self.maybe_real(), self.real(d)),
            13 => format!("-{}", self.real(d)),
            14 => format!("clamp({}, {}, {})", self.real(d), self.int(d), self.real(d)),
            _ => format!("atan2({}, {})", self.real(d), self.real(d)),
        }
    }

    /// A real that may be null.
    fn maybe_real(&mut self) -> String {
        self.pick(&[
            "$maybe",
            "target.mass",
            "here.site.danger",
            "$m['k']",
            "$m['missing']",
            "self.stats.speed",
        ])
        .to_owned()
    }

    /// A number: an int, a real, one that may be null, or `$x`.
    fn number(&mut self, depth: u32) -> String {
        match self.roll(10) {
            0..=3 => self.int(depth),
            4..=7 => self.real(depth),
            8 => self.maybe_real(),
            _ => self.dynamic(depth),
        }
    }

    /// Something whose type is `any` or `number` because `$x` is in it.
    fn dynamic(&mut self, depth: u32) -> String {
        if depth == 0 {
            return "$x".to_owned();
        }
        let d = depth - 1;
        match self.roll(8) {
            0 => format!("($x + {})", self.number(d)),
            1 => "-$x".to_owned(),
            2 => format!("$x[{}]", self.int(d)),
            3 => "$x.k".to_owned(),
            4 => format!("($x ?? {})", self.number(d)),
            5 => "first($x)".to_owned(),
            6 => format!("({} * $x)", self.number(d)),
            _ => "$x".to_owned(),
        }
    }

    fn boolean(&mut self, depth: u32) -> String {
        if depth == 0 {
            return self
                .pick(&[
                    "true",
                    "false",
                    "$flag",
                    "self.has_tag('guard')",
                    "here.site.var.alarm_raised",
                    "$x",
                ])
                .to_owned();
        }
        let d = depth - 1;
        match self.roll(20) {
            0 => format!("({} and {})", self.boolean(d), self.boolean(d)),
            1 => format!("({} or {})", self.boolean(d), self.boolean(d)),
            2 => format!("not {}", self.boolean(d)),
            3 => format!("({} < {})", self.number(d), self.number(d)),
            4 => format!("({} >= {})", self.number(d), self.number(d)),
            5 => format!("({} == {})", self.number(d), self.number(d)),
            6 => format!("({} != {})", self.number(d), self.number(d)),
            7 => format!("({} == {})", self.text(d), self.text(d)),
            8 => format!("({} < {})", self.text(d), self.text(d)),
            9 => format!("({} in {})", self.int(d), self.list(d)),
            10 => format!("({} == null)", self.number(d)),
            11 => format!("({} != null)", self.maybe_real()),
            12 => format!("chance({})", self.real(d)),
            13 => format!("contains({}, {})", self.list(d), self.int(d)),
            14 => format!("is_null({})", self.number(d)),
            15 => format!("({} == {})", self.list(d), self.list(d)),
            16 => format!(
                "(self.material == {})",
                self.pick(&["'glass'", "'iron'", "$name", "here.material"])
            ),
            17 => format!("({} in $x)", self.int(d)),
            18 => format!(
                "(if {} then {} else {})",
                self.boolean(d),
                self.boolean(d),
                self.boolean(d)
            ),
            _ => format!("(secret() > {})", self.int(d)),
        }
    }

    fn text(&mut self, depth: u32) -> String {
        if depth == 0 {
            return self
                .pick(&["'ash'", "''", "'Iron'", "$name", "event.kind"])
                .to_owned();
        }
        let d = depth - 1;
        match self.roll(6) {
            0 => format!("({} + {})", self.text(d), self.text(d)),
            1 => format!("str({})", self.number(d)),
            2 => format!("cap({})", self.text(d)),
            3 => format!(
                "(if {} then {} else {})",
                self.boolean(d),
                self.text(d),
                self.text(d)
            ),
            4 => format!("str({})", self.list(d)),
            _ => format!("str({})", self.boolean(d)),
        }
    }

    fn list(&mut self, depth: u32) -> String {
        if depth == 0 {
            return self.pick(&["$xs", "[]", "[1, 2, 3]", "[7]"]).to_owned();
        }
        let d = depth - 1;
        match self.roll(6) {
            0 => format!("({} + {})", self.list(d), self.list(d)),
            1 => format!("[{}, {}]", self.int(d), self.int(d)),
            2 => format!("without({}, {})", self.list(d), self.int(d)),
            3 => format!(
                "(if {} then {} else {})",
                self.boolean(d),
                self.list(d),
                self.list(d)
            ),
            4 => format!("[{}]", self.int(d)),
            _ => format!("[core:double({}), roll({})]", self.int(d), self.int(d)),
        }
    }

    fn any(&mut self, depth: u32) -> String {
        match self.roll(7) {
            0 => self.int(depth),
            1 => self.real(depth),
            2 => self.boolean(depth),
            3 => self.text(depth),
            4 => self.list(depth),
            5 => self.dynamic(depth),
            _ => format!(
                "{{'a': {}, {}: {}}}",
                self.number(depth),
                self.text(depth),
                self.boolean(depth)
            ),
        }
    }
}

/// A host whose state is drawn from the same stream as the expression.
fn random_host(rng: &mut Rng) -> TestHost {
    let mut host = populated();
    host.rng = Some(Rng::from_seed(rng.next_u64()));
    let numbers = [0, 1, -1, 3, 1000, i64::MAX, i64::MIN];
    let reals = [0.0, 0.5, -2.5, 1e308, f64::INFINITY, -1e-9];
    host.set_local(slot::N, *rng.pick(&numbers).unwrap());
    host.set_local(slot::POWER, real(*rng.pick(&reals).unwrap()));
    host.set_local(slot::FLAG, rng.below(2) == 0);
    if rng.below(3) == 0 {
        host.set_local(slot::MAYBE, real(*rng.pick(&reals).unwrap()));
    }
    if rng.below(4) == 0 {
        host.set_subject(slot::TARGET, Value::Null);
    }
    let x = match rng.below(9) {
        0 => Value::Null,
        1 => int(*rng.pick(&numbers).unwrap()),
        2 => real(*rng.pick(&reals).unwrap()),
        3 => text("ash"),
        4 => Value::Bool(true),
        5 => ints(&[4, 5]),
        6 => Value::map([("k", int(1))]),
        7 => Value::Subject(host.entity(3)),
        _ => host.material("core:iron"),
    };
    host.set_local(slot::X, x);
    host
}

/// How many random expressions to try. `CLASH_EXPR_CASES` asks for more,
/// for a long run by hand.
fn cases() -> u32 {
    std::env::var("CLASH_EXPR_CASES")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(3000)
}

/// Folding may put one node in another's place. What the operation above
/// reports about that operand must still be reported where the operand
/// was written, and what the node itself reports, where it was written.
/// (Found by the property test below.)
#[test]
fn folding_does_not_move_where_a_fault_is_reported() {
    let mut host = populated();
    host.set_local(slot::X, 3);
    let wrong = Fault::Type {
        wanted: "a bool",
        found: "int",
    };
    for (source, value, covers) in [
        (
            "if (if true then $x else $flag) then 1 else 2",
            int(2),
            "(if true then $x else $flag)",
        ),
        ("not (null ?? $x)", Value::Bool(true), "(null ?? $x)"),
        (
            "(if false then $flag else $x) and true",
            Value::Bool(false),
            "(if false then $flag else $x)",
        ),
        (
            "true and (if true then $x else $flag)",
            Value::Bool(false),
            "(if true then $x else $flag)",
        ),
        (
            "$flag or (1 ?? 2) == 1 and (null ?? $x)",
            Value::Bool(true),
            "",
        ),
        ("(null ?? $x) or false", Value::Bool(false), "(null ?? $x)"),
    ] {
        let (found, warnings) = run(&mut host, source);
        assert_eq!(found, value, "`{source}`");
        if covers.is_empty() {
            assert_eq!(warnings, [], "`{source}`");
        } else {
            assert_eq!(warnings.len(), 1, "`{source}`");
            assert_eq!(warnings[0].fault, wrong, "`{source}`");
            assert_eq!(warnings[0].span.slice(source), covers, "`{source}`");
        }
    }
    // The node that moved in reports its own faults where it was
    // written.
    host.set_local(slot::N, 0);
    let source = "if true then 1 / $n else 2";
    let (_, warnings) = run(&mut host, source);
    assert_eq!(warnings[0].span.slice(source), "1 / $n");
    let source = "not (if true then 1 / $n > 0 else $flag)";
    let (_, warnings) = run(&mut host, source);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].span.slice(source), "1 / $n");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases(),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// The machine agrees with the direct tree-walking evaluator: the
    /// same value, the same warnings in the same order, the same calls to
    /// the host and the same random draws.
    #[test]
    fn the_machine_agrees_with_the_tree(seed in any::<u64>(), depth in 0_u32..5) {
        let mut rng = Rng::from_seed(seed);
        let source = Maker { rng: &mut rng }.any(depth);
        let mut host = random_host(&mut rng);
        // `run` checks the agreement and panics with the source if there
        // is none.
        run(&mut host, &source);
    }

    /// The same for text templates.
    #[test]
    fn the_machine_renders_what_the_tree_renders(seed in any::<u64>(), depth in 0_u32..3) {
        let mut rng = Rng::from_seed(seed);
        let mut maker = Maker { rng: &mut rng };
        let source = format!(
            "{} is {{{}}}, {{{}:.2}} and {{{}}}.",
            "{{it}}",
            maker.any(depth),
            maker.number(depth),
            maker.text(depth)
        );
        let mut host = random_host(&mut rng);
        render(&mut host, &source);
    }
}

// ---------------------------------------------------------------------
// The budget of 05 §19.

/// Prints the time one evaluation of a condition of about ten operations
/// takes, to compare with the target of 05 §19 (under 300 ns). Run it
/// with `cargo test --release -p clash-expr --test machine -- --ignored
/// --nocapture`.
#[test]
#[ignore = "a measurement, not a check: run it by hand in release mode"]
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a measurement needs a clock; this test is not part of the simulation and nothing \
              in the crate reads one"
)]
fn time_per_evaluation() {
    let registry_host = populated();
    let context = context(&registry_host);
    let mut host = LeanHost {
        subjects: registry_host.subjects.clone(),
        locals: registry_host.locals.clone(),
        warnings: 0,
    };
    let cases = [
        (
            "locals only",
            "$power * 2 + $n > 10 and $n % 2 == 1 or $flag",
        ),
        (
            "properties",
            "self.mass * 2 > target.mass and self.var.kills >= $n and here.x != 0",
        ),
    ];
    let mut machine = Machine::new();
    for (name, source) in cases {
        let compiled = compile(
            source,
            registry_host.registry(),
            &context,
            &Type::Bool.nullable(),
        )
        .into_result()
        .unwrap();
        let rounds = 5_000_000_u32;
        let mut holds = 0_u32;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            holds += u32::from(machine.eval_condition(&compiled, &mut host));
        }
        let elapsed = start.elapsed();
        println!(
            "{name}: {} operations, {:.1} ns per evaluation ({holds} of {rounds} held, {} warnings)",
            compiled.len(),
            elapsed.as_nanos() as f64 / f64::from(rounds),
            host.warnings
        );
    }
}

/// A host that answers from fixed values and keeps no log, so that what
/// is timed is the machine.
struct LeanHost {
    subjects: Vec<Value>,
    locals: Vec<Value>,
    warnings: u64,
}

impl clash_expr::Host for LeanHost {
    fn subject(&mut self, slot: u16) -> Value {
        self.subjects
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn local(&mut self, slot: u16) -> Value {
        self.locals
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(
        &mut self,
        property: clash_expr::PropertyId,
        subject: clash_expr::Handle,
        _args: &[Value],
    ) -> Value {
        // Something that depends on what was asked, as a real getter's
        // answer would.
        Value::Real(Real::from_i64(
            100 + i64::from(property.raw()) + subject.payload as i64,
        ))
    }

    fn variable(&mut self, _subject: clash_expr::Handle, _name: &str) -> Value {
        Value::Int(4)
    }

    fn call(&mut self, _function: clash_expr::FunctionId, _args: &[Value]) -> Value {
        Value::Null
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {
        self.warnings += 1;
    }
}
