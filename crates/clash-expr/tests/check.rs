//! X2: the registry, contexts, name resolution and type checking.
//!
//! Each kind of name error and type error has a fixture with the
//! diagnostic and the suggestion it must give. A random function in a
//! pure context is refused, a read that is not safe for belief is
//! refused in a mind context, and a misspelt id compared with a material
//! is a load error that suggests the right one.

use clash_expr::testing::TestHost;
use clash_expr::{
    Code, Context, Diagnostic, EventSpec, FunctionSpec, KindSpec, ParamSpec, PropertySpec,
    Registry, RegistryError, Scope, Severity, Type, Undeclared, VariableSpec, VerbSpec, check,
    check_template, codes, compile,
};

fn diagnostics_in(source: &str, context: &Context, host: &TestHost) -> Vec<Diagnostic> {
    check(source, host.registry(), context, &Type::Any).diagnostics
}

fn diagnostics(source: &str) -> Vec<Diagnostic> {
    let host = TestHost::new();
    diagnostics_in(source, &host.context(), &host)
}

/// Checks that `source` gives exactly one diagnostic: this code and
/// severity, covering this text, with this suggestion.
#[track_caller]
fn one_in(
    source: &str,
    context: &Context,
    host: &TestHost,
    severity: Severity,
    code: Code,
    covers: &str,
    suggestion: Option<&str>,
) -> Diagnostic {
    let found = diagnostics_in(source, context, host);
    assert_eq!(found.len(), 1, "`{source}`: {found:#?}");
    let only = found.into_iter().next().unwrap();
    assert_eq!(only.code, code, "`{source}`: {only}");
    assert_eq!(only.severity, severity, "`{source}`: {only}");
    assert_eq!(only.span.slice(source), covers, "`{source}`: {only}");
    match suggestion {
        Some(wanted) => assert_eq!(
            only.suggestion.as_deref(),
            Some(wanted),
            "`{source}`: {only}"
        ),
        None => assert_eq!(only.suggestion, None, "`{source}`: {only}"),
    }
    assert!(!only.message.is_empty());
    only
}

#[track_caller]
fn error(source: &str, code: Code, covers: &str, suggestion: Option<&str>) -> Diagnostic {
    let host = TestHost::new();
    let context = host.context();
    one_in(
        source,
        &context,
        &host,
        Severity::Error,
        code,
        covers,
        suggestion,
    )
}

#[track_caller]
fn warning(source: &str, code: Code, covers: &str, suggestion: Option<&str>) -> Diagnostic {
    let host = TestHost::new();
    let context = host.context();
    let found = one_in(
        source,
        &context,
        &host,
        Severity::Warning,
        code,
        covers,
        suggestion,
    );
    // A warning does not stop the expression from compiling.
    assert!(
        compile(source, host.registry(), &context, &Type::Any)
            .value
            .is_some()
    );
    found
}

/// Checks that `source` is accepted without a word, and gives its type
/// as the language writes it.
#[track_caller]
fn ty(source: &str) -> String {
    let host = TestHost::new();
    let outcome = check(source, host.registry(), &host.context(), &Type::Any);
    assert_eq!(outcome.diagnostics, [], "`{source}`");
    host.registry().type_name(outcome.value.unwrap().ty())
}

const USE_COALESCE: Option<&str> = Some("use `??` to give a value for when it is null");

// ---------------------------------------------------------------------
// Names.

#[test]
fn an_unknown_name_suggests_a_bound_one() {
    error(
        "slef.mass",
        codes::UNKNOWN_NAME,
        "slef",
        Some("did you mean `self`?"),
    );
    error(
        "targte == self",
        codes::UNKNOWN_NAME,
        "targte",
        Some("did you mean `target`?"),
    );
    error("zzzzzz", codes::UNKNOWN_NAME, "zzzzzz", None);
    // A function named without calling it.
    error(
        "clamp + 1",
        codes::UNKNOWN_NAME,
        "clamp",
        Some("`clamp` is a function; call it: `clamp(…)`"),
    );
    error(
        "core:double",
        codes::UNKNOWN_NAME,
        "core:double",
        Some("`core:double` is a function; call it: `core:double(…)`"),
    );
    // A compound unit reads as a division by a name.
    error(
        "3m/s",
        codes::UNKNOWN_NAME,
        "s",
        Some("compound units such as `m/s` are written in JSON unit strings"),
    );
    // Both mistakes in one expression are reported.
    let found = diagnostics("slef.mass + targte.mass");
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|d| d.code == codes::UNKNOWN_NAME));
}

#[test]
fn an_unknown_local_suggests_a_declared_one() {
    error(
        "$powr * 2",
        codes::UNKNOWN_LOCAL,
        "$powr",
        Some("did you mean `$power`?"),
    );
    error("$zzzzzz", codes::UNKNOWN_LOCAL, "$zzzzzz", None);
    // A subject is not a local, and a local is not a subject.
    error("$self", codes::UNKNOWN_LOCAL, "$self", None);
    error("power", codes::UNKNOWN_NAME, "power", None);
}

#[test]
fn an_unknown_property_suggests_one_of_the_kind() {
    error(
        "self.mas",
        codes::UNKNOWN_PROPERTY,
        "self.mas",
        Some("did you mean `mass`?"),
    );
    error(
        "here.sit.name",
        codes::UNKNOWN_PROPERTY,
        "here.sit",
        Some("did you mean `site`?"),
    );
    // `mass` is a property of entities, not of locations.
    error("here.mass", codes::UNKNOWN_PROPERTY, "here.mass", None);
    error(
        "self.vra.kills",
        codes::UNKNOWN_PROPERTY,
        "self.vra",
        Some("did you mean `var`?"),
    );
    error(
        "self.has_tga('x')",
        codes::UNKNOWN_PROPERTY,
        "self.has_tga('x')",
        Some("did you mean `has_tag`?"),
    );
    // `var` alone is not a value.
    error(
        "self.var",
        codes::UNKNOWN_PROPERTY,
        "self.var",
        Some("write `.var.name`, or `.var('mod:name')` to name one exactly"),
    );
}

#[test]
fn an_unknown_function_suggests_a_registered_one() {
    error(
        "clmap(1, 2, 3)",
        codes::UNKNOWN_FUNCTION,
        "clmap",
        Some("did you mean `clamp`?"),
    );
    error(
        "sqr(4)",
        codes::UNKNOWN_FUNCTION,
        "sqr",
        Some("did you mean `sqrt`?"),
    );
    // A bare name is offered for a function in a namespace in scope.
    error(
        "doubel(2)",
        codes::UNKNOWN_FUNCTION,
        "doubel",
        Some("did you mean `double`?"),
    );
    error(
        "ashlands:fall_damge(3)",
        codes::UNKNOWN_FUNCTION,
        "ashlands:fall_damge",
        Some("did you mean `ashlands:fall_damage`?"),
    );
    error("zzzzzzzz(1)", codes::UNKNOWN_FUNCTION, "zzzzzzzz", None);
    // The arguments of an unknown function are still checked.
    let found = diagnostics("zzzzzzzz(slef)");
    assert_eq!(found.len(), 2);
}

#[test]
fn function_names_resolve_like_ids() {
    // The engine's bare names, the mod's own namespace, a qualified name.
    assert_eq!(ty("clamp(1, 2, 3)"), "int");
    assert_eq!(ty("double(2)"), "number");
    assert_eq!(ty("core:double(2)"), "number");
    assert_eq!(ty("ashlands:fall_damage(3)"), "real");
    assert_eq!(ty("ashlands:fall_damage(3, 80)"), "real");
    // Another mod's function is not visible bare…
    error(
        "fall_damage(3)",
        codes::UNKNOWN_FUNCTION,
        "fall_damage",
        None,
    );
    // …unless its namespace is imported.
    let host = TestHost::new();
    let importing = host.context().imports(["ashlands"]);
    assert_eq!(diagnostics_in("fall_damage(3)", &importing, &host), []);
    // A name in both the mod's namespace and an import is the mod's own,
    // with a warning.
    one_in(
        "double(2)",
        &importing,
        &host,
        Severity::Warning,
        codes::AMBIGUOUS_NAME,
        "double",
        Some("qualify the name"),
    );
}

#[test]
fn event_fields_are_checked_against_the_event() {
    assert_eq!(ty("event.energy"), "real");
    assert_eq!(ty("event.kind"), "string");
    assert_eq!(ty("event.source"), "entity?");
    error(
        "event.enrgy",
        codes::UNKNOWN_EVENT_FIELD,
        "event.enrgy",
        Some("did you mean `energy`?"),
    );
    error(
        "event.zzzzzz",
        codes::UNKNOWN_EVENT_FIELD,
        "event.zzzzzz",
        None,
    );
    error("event", codes::UNKNOWN_EVENT_FIELD, "event", None);
    // A context with no event has no fields to read.
    let host = TestHost::new();
    let context = Context::new().subject("self", Type::Subject(host.entity));
    one_in(
        "event.energy",
        &context,
        &host,
        Severity::Error,
        codes::UNKNOWN_EVENT_FIELD,
        "event.energy",
        None,
    );
    // A subject bound as `event` is a subject.
    let context = Context::new().subject("event", Type::Subject(host.entity));
    assert_eq!(diagnostics_in("event.mass", &context, &host), []);
}

#[test]
fn a_misspelt_id_is_a_load_error_with_a_suggestion() {
    error(
        "self.material == 'glas'",
        codes::UNKNOWN_ID,
        "'glas'",
        Some("did you mean 'glass'?"),
    );
    error(
        "'glas' == self.material",
        codes::UNKNOWN_ID,
        "'glas'",
        Some("did you mean 'glass'?"),
    );
    error(
        "self.material != 'irn'",
        codes::UNKNOWN_ID,
        "'irn'",
        Some("did you mean 'iron'?"),
    );
    error(
        "here.material == 'core:glas'",
        codes::UNKNOWN_ID,
        "'core:glas'",
        Some("did you mean 'core:glass'?"),
    );
    // Another mod's id is offered qualified.
    error("self.material == 'slag'", codes::UNKNOWN_ID, "'slag'", None);
    error(
        "self.material == 'ashlands:slug'",
        codes::UNKNOWN_ID,
        "'ashlands:slug'",
        Some("did you mean 'ashlands:slag'?"),
    );
    // `wall` is a block, not a material.
    error("self.material == 'wall'", codes::UNKNOWN_ID, "'wall'", None);
    // Wherever an id is wanted: a list, membership, `??`, `if`.
    error(
        "self.material in ['glass', 'irn']",
        codes::UNKNOWN_ID,
        "'irn'",
        Some("did you mean 'iron'?"),
    );
    error(
        "'irn' in self.materials",
        codes::UNKNOWN_ID,
        "'irn'",
        Some("did you mean 'iron'?"),
    );
    error(
        "self.materials == ['glas']",
        codes::UNKNOWN_ID,
        "'glas'",
        Some("did you mean 'glass'?"),
    );
    error(
        "self.material == (if $flag then 'glass' else 'irn')",
        codes::UNKNOWN_ID,
        "'irn'",
        Some("did you mean 'iron'?"),
    );
    error(
        "(target.material ?? 'glas') == 'glass'",
        codes::UNKNOWN_ID,
        "'glas'",
        Some("did you mean 'glass'?"),
    );

    // The right ids are accepted, bare or qualified.
    assert_eq!(ty("self.material == 'glass'"), "bool");
    assert_eq!(ty("self.material == 'core:glass'"), "bool");
    assert_eq!(ty("self.material == 'ashlands:slag'"), "bool");
    assert_eq!(ty("self.material in ['glass', 'iron']"), "bool");
    assert_eq!(ty("'iron' in self.materials"), "bool");
    assert_eq!(ty("target.material ?? 'glass'"), "id<material>");
    assert_eq!(
        ty("if $flag then self.material else 'glass'"),
        "id<material>"
    );
    // A text that is not known at load is compared by name at run time.
    assert_eq!(ty("self.material == $name"), "bool");

    // The expected type of the whole expression counts too.
    let host = TestHost::new();
    let wanted = Type::Id(host.material);
    let context = host.context();
    let outcome = check("'glass'", host.registry(), &context, &wanted);
    assert_eq!(outcome.diagnostics, []);
    assert_eq!(outcome.value.unwrap().ty(), &wanted);
    let outcome = check("'glas'", host.registry(), &context, &wanted);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, codes::UNKNOWN_ID);
    assert!(outcome.value.is_none());

    // An id found in the mod's namespace and in an import is the mod's
    // own, with a warning.
    let importing = host.context().imports(["ashlands"]);
    one_in(
        "self.material == 'ash'",
        &importing,
        &host,
        Severity::Warning,
        codes::AMBIGUOUS_NAME,
        "'ash'",
        Some("qualify the id"),
    );
    assert_eq!(
        diagnostics_in("self.material == 'slag'", &importing, &host),
        []
    );
}

#[test]
fn variables_are_resolved_declared_or_not() {
    assert_eq!(ty("self.var.kills"), "int");
    assert_eq!(ty("self.var('core:kills')"), "int");
    assert_eq!(ty("self.var('ashlands:kills')"), "int");
    assert_eq!(ty("self.var.mood"), "string?");
    assert_eq!(ty("here.site.var.alarm_raised"), "bool");
    assert_eq!(ty("global.var.day"), "int");
    // A variable of something that may be null may be null.
    assert_eq!(ty("target.var.kills"), "int?");

    // An undeclared variable works, with a warning.
    let found = warning(
        "self.var.kils",
        codes::UNDECLARED_VARIABLE,
        "self.var.kils",
        Some("did you mean `kills`?"),
    );
    assert!(found.message.contains("core:kils"));
    warning(
        "self.var.zzzzzzzz",
        codes::UNDECLARED_VARIABLE,
        "self.var.zzzzzzzz",
        Some("declare it with a `variable` definition"),
    );
    warning(
        "self.var('other:thing')",
        codes::UNDECLARED_VARIABLE,
        "self.var('other:thing')",
        Some("declare it with a `variable` definition"),
    );

    // Strict mods refuse it; tools that have checked already allow it.
    let host = TestHost::new();
    let strict = host.context().undeclared(Undeclared::Deny);
    one_in(
        "self.var.kils",
        &strict,
        &host,
        Severity::Error,
        codes::UNDECLARED_VARIABLE,
        "self.var.kils",
        Some("did you mean `kills`?"),
    );
    let lax = host.context().undeclared(Undeclared::Allow);
    assert_eq!(diagnostics_in("self.var.kils", &lax, &host), []);

    // A variable is declared on one kind of subject.
    error(
        "self.var.alarm_raised",
        codes::VARIABLE_SCOPE,
        "self.var.alarm_raised",
        None,
    );
    error(
        "here.var.kills",
        codes::VARIABLE_SCOPE,
        "here.var.kills",
        None,
    );
    // Only subjects have variables.
    error("$n.var.kills", codes::NO_PROPERTIES, "$n", None);
    // `var(…)` takes the name as a text known at load.
    error(
        "self.var($name)",
        codes::ARGUMENT_COUNT,
        "self.var($name)",
        Some("write `.var('mod:name')`"),
    );
    error(
        "self.var()",
        codes::ARGUMENT_COUNT,
        "self.var()",
        Some("write `.var('mod:name')`"),
    );

    // A name in the mod's namespace and in an import is the mod's own,
    // with a warning.
    let importing = host.context().imports(["ashlands"]);
    one_in(
        "self.var.kills",
        &importing,
        &host,
        Severity::Warning,
        codes::AMBIGUOUS_NAME,
        "self.var.kills",
        Some("qualify the name"),
    );
}

#[test]
fn only_functions_and_properties_can_be_called() {
    error("self(1)", codes::NOT_CALLABLE, "self", None);
    error("(1 + 2)(3)", codes::NOT_CALLABLE, "(1 + 2)", None);
    error("$n.foo(1)", codes::NOT_CALLABLE, "$n.foo", None);
    error("$xs[0](1)", codes::NOT_CALLABLE, "$xs[0]", None);
    error(
        "clamp(1, 2, 3)(4)",
        codes::NOT_CALLABLE,
        "clamp(1, 2, 3)",
        None,
    );
}

// ---------------------------------------------------------------------
// Types.

#[test]
fn operators_refuse_operands_of_the_wrong_type() {
    let join_hint = Some("use `str(…)` to join a text with something else");
    error("1 + 'a'", codes::OPERAND_TYPE, "1 + 'a'", join_hint);
    error("'a' + $n", codes::OPERAND_TYPE, "'a' + $n", join_hint);
    error("[1] + 2", codes::OPERAND_TYPE, "[1] + 2", None);
    error("true + false", codes::OPERAND_TYPE, "true + false", None);
    error("'a' * 2", codes::OPERAND_TYPE, "'a' * 2", None);
    error("self - 1", codes::OPERAND_TYPE, "self - 1", None);
    error("$flag / 2", codes::OPERAND_TYPE, "$flag / 2", None);
    error("2 ^ 'a'", codes::OPERAND_TYPE, "2 ^ 'a'", None);
    let round_hint = Some("round with `floor`, `ceil` or `round` first");
    error("1.5 % 2", codes::OPERAND_TYPE, "1.5 % 2", round_hint);
    error(
        "$n % $power",
        codes::OPERAND_TYPE,
        "$n % $power",
        round_hint,
    );
    error("-'a'", codes::OPERAND_TYPE, "'a'", None);
    error("-$flag", codes::OPERAND_TYPE, "$flag", None);
    error("not 3", codes::OPERAND_TYPE, "3", None);
    error("1 and true", codes::OPERAND_TYPE, "1", None);
    error("$flag or 'a'", codes::OPERAND_TYPE, "'a'", None);
    error("if 1 then 2 else 3", codes::OPERAND_TYPE, "1", None);
    error("'a' < 1", codes::OPERAND_TYPE, "'a' < 1", None);
    error("$flag >= true", codes::OPERAND_TYPE, "$flag >= true", None);
    error("self < target", codes::OPERAND_TYPE, "self < target", None);
    error("$xs['a']", codes::OPERAND_TYPE, "'a'", None);
    error("$m[1]", codes::OPERAND_TYPE, "1", None);
    error("{1: 2}", codes::OPERAND_TYPE, "1", None);
    error("1 in $m", codes::OPERAND_TYPE, "1", None);
    error("1 in 2", codes::OPERAND_TYPE, "2", None);
    error(
        "'a' in 'abc'",
        codes::OPERAND_TYPE,
        "'abc'",
        Some("use `contains(text, part)` for a text in a text"),
    );
}

#[test]
fn calls_are_checked_against_signatures() {
    let clamp = Some("its signature is `clamp(x: number, low: number, high: number) -> number`");
    let found = error("clamp(1, 2)", codes::ARGUMENT_COUNT, "clamp(1, 2)", clamp);
    assert_eq!(
        found.message,
        "function `clamp` takes 3 arguments, and 2 are given"
    );
    error(
        "clamp(1, 2, 3, 4)",
        codes::ARGUMENT_COUNT,
        "clamp(1, 2, 3, 4)",
        clamp,
    );
    let found = error(
        "min(1)",
        codes::ARGUMENT_COUNT,
        "min(1)",
        Some("its signature is `min(a: number, b: number...) -> number`"),
    );
    assert_eq!(
        found.message,
        "function `min` takes at least 2 arguments, and 1 is given"
    );
    error(
        "sqrt()",
        codes::ARGUMENT_COUNT,
        "sqrt()",
        Some("its signature is `sqrt(x: real) -> real`"),
    );
    let fall = Some("its signature is `ashlands:fall_damage(height: real, mass?: real) -> real`");
    let found = error(
        "ashlands:fall_damage()",
        codes::ARGUMENT_COUNT,
        "ashlands:fall_damage()",
        fall,
    );
    assert!(found.message.contains("takes 1 to 2 arguments"));
    error(
        "ashlands:fall_damage(1, 2, 3)",
        codes::ARGUMENT_COUNT,
        "ashlands:fall_damage(1, 2, 3)",
        fall,
    );
    // Properties with arguments are calls too.
    let has_tag = Some("its signature is `entity.has_tag(tag: string) -> bool`");
    error(
        "self.has_tag()",
        codes::ARGUMENT_COUNT,
        "self.has_tag()",
        has_tag,
    );
    error(
        "self.has_tag",
        codes::ARGUMENT_COUNT,
        "self.has_tag",
        has_tag,
    );
    error(
        "self.mass(1)",
        codes::ARGUMENT_COUNT,
        "self.mass(1)",
        Some("its signature is `entity.mass -> real`"),
    );

    error("sqrt('a')", codes::ARGUMENT_TYPE, "'a'", None);
    error("clamp(1, 'a', 3)", codes::ARGUMENT_TYPE, "'a'", None);
    error("min(1, 2, self)", codes::ARGUMENT_TYPE, "self", None);
    error("div(1.5, 2)", codes::ARGUMENT_TYPE, "1.5", None);
    error("self.has_tag(3)", codes::ARGUMENT_TYPE, "3", None);
    error("dist(here, self)", codes::ARGUMENT_TYPE, "self", None);
    error("first(3)", codes::ARGUMENT_TYPE, "3", None);
    error("sum(['a'])", codes::ARGUMENT_TYPE, "['a']", None);
    error("keys($xs)", codes::ARGUMENT_TYPE, "$xs", None);
    // The functions that take several shapes say which.
    let found = error("len(3)", codes::ARGUMENT_TYPE, "len(3)", None);
    assert_eq!(
        found.message,
        "`len` takes a list, a map or a text, found int"
    );
    error(
        "contains('abc', 1)",
        codes::ARGUMENT_TYPE,
        "contains('abc', 1)",
        None,
    );
    error(
        "contains(1, 1)",
        codes::ARGUMENT_TYPE,
        "contains(1, 1)",
        None,
    );
    error("has($m, 1)", codes::ARGUMENT_TYPE, "has($m, 1)", None);
}

#[test]
fn a_value_that_may_be_null_in_arithmetic_is_a_warning_that_suggests_coalesce() {
    warning(
        "$maybe + 1",
        codes::NULL_IN_ARITHMETIC,
        "$maybe",
        USE_COALESCE,
    );
    warning(
        "1 - $maybe",
        codes::NULL_IN_ARITHMETIC,
        "$maybe",
        USE_COALESCE,
    );
    warning("-$maybe", codes::NULL_IN_ARITHMETIC, "$maybe", USE_COALESCE);
    // A property of something that may be null may be null.
    warning(
        "target.mass * 2",
        codes::NULL_IN_ARITHMETIC,
        "target.mass",
        USE_COALESCE,
    );
    warning(
        "self.target.mass / 2",
        codes::NULL_IN_ARITHMETIC,
        "self.target.mass",
        USE_COALESCE,
    );
    // So may a map entry and a list element.
    warning(
        "$m['k'] * 2",
        codes::NULL_IN_ARITHMETIC,
        "$m['k']",
        USE_COALESCE,
    );
    warning(
        "$xs[0] + 1",
        codes::NULL_IN_ARITHMETIC,
        "$xs[0]",
        USE_COALESCE,
    );
    warning(
        "here.site.danger ^ 2",
        codes::NULL_IN_ARITHMETIC,
        "here.site.danger",
        USE_COALESCE,
    );
    let found = warning("null + 1", codes::NULL_IN_ARITHMETIC, "null", USE_COALESCE);
    assert!(found.message.starts_with("this is null"));
    // And an argument a function needs.
    warning(
        "sqrt($maybe)",
        codes::NULL_IN_ARITHMETIC,
        "$maybe",
        USE_COALESCE,
    );
    warning(
        "clamp($maybe, 0, 1)",
        codes::NULL_IN_ARITHMETIC,
        "$maybe",
        USE_COALESCE,
    );
    warning(
        "first($xs) * 2",
        codes::NULL_IN_ARITHMETIC,
        "first($xs)",
        USE_COALESCE,
    );

    // `??` settles it.
    assert_eq!(ty("($maybe ?? 0) + 1"), "real");
    assert_eq!(ty("(target.mass ?? 70) * 2"), "real");
    assert_eq!(ty("sqrt($maybe ?? 4)"), "real");
    // Comparisons, conditions and null tests take null without comment.
    assert_eq!(ty("$maybe > 1"), "bool");
    assert_eq!(ty("$maybe == null"), "bool");
    assert_eq!(ty("target != null and target.mass > 1"), "bool");
    assert_eq!(ty("not self.var.mood == 'calm'"), "bool");
    assert_eq!(ty("is_null($maybe)"), "bool");
    assert_eq!(ty("str($maybe)"), "string");
    assert_eq!(ty("if here.site.var.alarm_raised then 1 else 2"), "int");
}

#[test]
fn the_expected_type_is_checked() {
    let host = TestHost::new();
    let context = host.context();
    let with = |source: &str, expected: &Type| check(source, host.registry(), &context, expected);

    let outcome = with("1 + 2", &Type::Bool);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, codes::RESULT_TYPE);
    assert_eq!(
        outcome.diagnostics[0].message,
        "this must be bool, and it is int"
    );
    assert!(outcome.value.is_none());
    assert!(outcome.has_errors());
    assert_eq!(outcome.errors().count(), 1);
    assert_eq!(outcome.warnings().count(), 0);
    assert!(outcome.into_result().is_err());

    assert_eq!(
        with("'a'", &Type::Real).diagnostics[0].code,
        codes::RESULT_TYPE
    );
    assert_eq!(
        with("self", &Type::Subject(host.loc)).diagnostics[0].code,
        codes::RESULT_TYPE
    );
    assert_eq!(
        with("1.5", &Type::Int).diagnostics[0].code,
        codes::RESULT_TYPE
    );

    // An int is accepted as a real; anything as `any`; `any` anywhere.
    assert_eq!(with("1 + 2", &Type::Real).diagnostics, []);
    assert_eq!(with("self", &Type::Any).diagnostics, []);
    assert_eq!(with("$x", &Type::Bool).diagnostics, []);
    assert_eq!(with("self", &Type::Subject(host.entity)).diagnostics, []);
    assert_eq!(with("[1, 2]", &Type::list(Type::Real)).diagnostics, []);

    // A value that may be null where one is needed is a warning.
    let outcome = with("$maybe", &Type::Real);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, codes::NULL_IN_ARITHMETIC);
    assert_eq!(outcome.diagnostics[0].severity, Severity::Warning);
    assert!(outcome.value.is_some());
    assert_eq!(outcome.warnings().count(), 1);
    // A condition is a bool that may be null: null counts as false.
    let condition = Type::Bool.nullable();
    assert_eq!(
        with("here.site.var.alarm_raised", &condition).diagnostics,
        []
    );
    assert_eq!(with("self.var.mood == 'calm'", &condition).diagnostics, []);
    assert_eq!(with("null", &condition).diagnostics, []);
    assert_eq!(
        with("1", &condition).diagnostics[0].code,
        codes::RESULT_TYPE
    );

    // A syntax error is reported in the same way.
    let outcome = with("1 +", &Type::Any);
    assert_eq!(outcome.diagnostics[0].code, codes::UNEXPECTED_END);
    assert!(outcome.value.is_none());
}

#[test]
fn comparisons_between_unrelated_types_are_errors() {
    error(
        "self == here",
        codes::UNRELATED_COMPARISON,
        "self == here",
        None,
    );
    error("1 == 'a'", codes::UNRELATED_COMPARISON, "1 == 'a'", None);
    error("true != 1", codes::UNRELATED_COMPARISON, "true != 1", None);
    error(
        "self.material == $n",
        codes::UNRELATED_COMPARISON,
        "self.material == $n",
        None,
    );
    error(
        "self.material == here",
        codes::UNRELATED_COMPARISON,
        "self.material == here",
        None,
    );
    error("$xs == $m", codes::UNRELATED_COMPARISON, "$xs == $m", None);
    error(
        "$xs == ['a']",
        codes::UNRELATED_COMPARISON,
        "$xs == ['a']",
        None,
    );
    error(
        "'a' in $xs",
        codes::UNRELATED_COMPARISON,
        "'a' in $xs",
        None,
    );
    error(
        "self in $xs",
        codes::UNRELATED_COMPARISON,
        "self in $xs",
        None,
    );

    assert_eq!(ty("self == target"), "bool");
    assert_eq!(ty("1 == 1.0"), "bool");
    assert_eq!(ty("$n in $xs"), "bool");
    assert_eq!(ty("$power in $xs"), "bool");
    assert_eq!(ty("'k' in $m"), "bool");
    assert_eq!(ty("$xs == [1, 2]"), "bool");
    assert_eq!(ty("'guard' in self.tags"), "bool");
}

#[test]
fn only_lists_and_maps_are_indexed_and_only_subjects_and_maps_have_members() {
    error("$n[0]", codes::NOT_INDEXABLE, "$n", None);
    error("self[0]", codes::NOT_INDEXABLE, "self", None);
    error("'abc'[0]", codes::NOT_INDEXABLE, "'abc'", None);
    error("$n.foo", codes::NO_PROPERTIES, "$n.foo", None);
    error("'abc'.len", codes::NO_PROPERTIES, "'abc'.len", None);
    error("$xs.first", codes::NO_PROPERTIES, "$xs.first", None);

    // A member of a map is its entry for that key.
    assert_eq!(ty("$m.speed"), "real?");
    assert_eq!(ty("self.stats.speed"), "real?");
    assert_eq!(ty("{'cancelled': true}.cancelled"), "bool?");
}

#[test]
fn an_operation_on_constants_that_always_fails_is_a_warning() {
    let host = TestHost::new();
    let context = host.context();
    for (source, covers) in [
        ("1 / 0", "1 / 0"),
        ("$n + 7 % 0", "7 % 0"),
        ("sqrt(-1)", "sqrt(-1)"),
        ("ln(0)", "ln(0)"),
        ("div(1, 0)", "div(1, 0)"),
        ("9223372036854775807 + 1", "9223372036854775807 + 1"),
        ("0 ^ -1", "0 ^ -1"),
    ] {
        let outcome = compile(source, host.registry(), &context, &Type::Any);
        let found: Vec<&Diagnostic> = outcome
            .diagnostics
            .iter()
            .filter(|d| d.code == codes::ALWAYS_FAILS)
            .collect();
        assert_eq!(found.len(), 1, "`{source}`: {:?}", outcome.diagnostics);
        assert_eq!(found[0].severity, Severity::Warning);
        assert_eq!(found[0].span.slice(source), covers);
        assert!(
            found[0]
                .message
                .starts_with("this fails every time it runs: ")
        );
        assert!(outcome.value.is_some(), "`{source}` still compiles");
    }
}

#[test]
fn types_are_worked_out() {
    for (source, wanted) in [
        ("null", "null"),
        ("true", "bool"),
        ("42", "int"),
        ("3.5", "real"),
        ("300ms", "real"),
        ("2h", "real"),
        ("'ash'", "string"),
        ("self", "entity"),
        ("target", "entity?"),
        ("here", "loc"),
        ("$power", "real"),
        ("$x", "any"),
        // Arithmetic.
        ("1 + 2", "int"),
        ("1 + 2.0", "real"),
        ("1 - 2", "int"),
        ("2 * 3.5", "real"),
        ("1 / 2", "real"),
        ("2 ^ 3", "real"),
        ("7 % 2", "int"),
        ("-1", "int"),
        ("-1.5", "real"),
        ("'a' + 'b'", "string"),
        ("[1] + [2.5]", "list<real>"),
        // `any` passes and is checked at run time.
        ("$x + 1", "number"),
        ("$x + $x", "any"),
        ("$x + 'a'", "string"),
        ("-$x", "number"),
        ("$x % 2", "int"),
        ("$x.foo", "any"),
        ("$x[0]", "any"),
        ("$x == 1", "bool"),
        ("$x < 'a'", "bool"),
        ("not $x", "bool"),
        ("$x and true", "bool"),
        ("1 in $x", "bool"),
        ("sqrt($x)", "real"),
        ("len($x)", "int"),
        // Lists and maps.
        ("[]", "list<any>"),
        ("[1, 2]", "list<int>"),
        ("[1, 2.5]", "list<real>"),
        ("[1, 'a']", "list<any>"),
        ("[1, null]", "list<int?>"),
        ("{}", "map<any>"),
        ("{'a': 1}", "map<int>"),
        ("{'a': 1, 'b': 2.5}", "map<real>"),
        ("$xs[0]", "int?"),
        ("$m['k']", "real?"),
        ("$m[self.material]", "real?"),
        ("{'ash': 3}['ash']", "int?"),
        // Properties.
        ("self.mass", "real"),
        ("self.loc.site.name", "string"),
        ("self.target", "entity?"),
        ("self.target.mass", "real?"),
        ("self.target.target.target", "entity?"),
        ("self.material", "id<material>"),
        ("self.materials", "list<id<material>>"),
        ("self.has_tag('guard')", "bool"),
        ("self.part('arm')", "part?"),
        ("self.part('arm').size", "real?"),
        ("here.field('fire')", "real"),
        ("here.site.danger", "real?"),
        ("self.stats", "map<real>"),
        // Logic and choice.
        ("1 < 2", "bool"),
        ("'a' < 'b'", "bool"),
        ("$maybe ?? 0", "real"),
        ("null ?? 1", "int"),
        ("$maybe ?? null", "real?"),
        ("$x ?? 1", "any"),
        ("if $flag then 1 else 2.5", "real"),
        ("if $flag then 1 else null", "int?"),
        ("if $flag then self else target", "entity?"),
        ("if $flag then 1 else 'a'", "any"),
        // Built-ins.
        ("min(1, 2)", "int"),
        ("min(1, 2.5)", "real"),
        ("max(1, 2, 3, 4)", "int"),
        ("clamp($power, 0, 1)", "real"),
        ("abs(-3)", "int"),
        ("abs(-3.5)", "real"),
        ("sign(-3.5)", "int"),
        ("floor(2.5)", "int"),
        ("ceil(2.5)", "int"),
        ("round(2.5)", "int"),
        ("sqrt(4)", "real"),
        ("lerp(0, 10, 0.5)", "real"),
        ("div(7, 2)", "int"),
        ("rand(0, 1)", "real"),
        ("rand_int(1, 6)", "int"),
        ("chance(0.5)", "bool"),
        ("pick($xs)", "int?"),
        ("len($xs)", "int"),
        ("len('abc')", "int"),
        ("len($m)", "int"),
        ("contains($xs, 1)", "bool"),
        ("contains('abc', 'b')", "bool"),
        ("first($xs)", "int?"),
        ("last(self.tags)", "string?"),
        ("sum($xs)", "int"),
        ("sum([1.5, 2])", "real"),
        ("str(42)", "string"),
        ("keys($m)", "list<string>"),
        ("values($m)", "list<real>"),
        ("has($m, 'k')", "bool"),
        ("has($m, self.material)", "bool"),
        ("without($xs, 1)", "list<int>"),
        ("without_key($m, 'k')", "map<real>"),
        ("cap('ash')", "string"),
        ("is_null($x)", "bool"),
        ("exists(target)", "bool"),
        ("the(self)", "string"),
        ("verb(self, 'hit')", "string"),
        ("count(3, 'arrow')", "string"),
        ("list([self, target])", "string"),
    ] {
        assert_eq!(ty(source), wanted, "`{source}`");
    }
}

// ---------------------------------------------------------------------
// Contexts.

#[test]
fn a_random_function_in_a_pure_context_is_rejected() {
    let host = TestHost::new();
    let random = host.context();
    let pure = host.context().pure();
    for (source, covers) in [
        ("rand(0, 1)", "rand"),
        ("rand_int(1, 6) > 3", "rand_int"),
        ("chance(0.5)", "chance"),
        ("pick($xs)", "pick"),
        // A random function the host registered.
        ("roll(6)", "roll"),
        // However deep it is.
        ("if $flag then 1 else clamp(rand(0, 1), 0, 1)", "rand"),
    ] {
        let found = one_in(
            source,
            &pure,
            &host,
            Severity::Error,
            codes::RANDOM_IN_PURE,
            covers,
            None,
        );
        assert!(found.message.contains("draws random numbers"));
        assert!(
            compile(source, host.registry(), &pure, &Type::Any)
                .value
                .is_none()
        );
        // The same expression is fine where random draws are allowed.
        assert_eq!(diagnostics_in(source, &random, &host), [], "`{source}`");
    }
    // Everything else is allowed in a pure context.
    assert_eq!(
        diagnostics_in("clamp(self.mass, 0, 1) + sqrt(4)", &pure, &host),
        []
    );
}

#[test]
fn a_mind_context_reads_only_belief() {
    let host = TestHost::new();
    let truth = host.context();
    let mind = host.context().belief();
    let refused = |source: &str, covers: &str| {
        one_in(
            source,
            &mind,
            &host,
            Severity::Error,
            codes::NOT_BELIEF_SAFE,
            covers,
            None,
        )
    };
    let refused_with_hint = |source: &str, covers: &str| {
        one_in(
            source,
            &mind,
            &host,
            Severity::Error,
            codes::NOT_BELIEF_SAFE,
            covers,
            Some("read what the character has perceived instead (15 §7)"),
        )
    };

    // Another entity's state is truth.
    refused_with_hint("target.mass", "target.mass");
    refused_with_hint("target.loc", "target.loc");
    refused_with_hint("target.has_tag('guard')", "target.has_tag('guard')");
    // So is the state of the world.
    refused_with_hint("here.material", "here.material");
    refused_with_hint("here.field('fire') > 0", "here.field('fire')");
    refused_with_hint("here.site.danger", "here.site.danger");
    // A place the character is in is not the character.
    refused_with_hint("self.loc.material", "self.loc.material");
    // Functions and variables say whether they are safe.
    refused("secret()", "secret");
    refused("exists(target)", "exists");
    refused("target.var.mood", "target.var.mood");
    refused("here.site.var.alarm_raised", "here.site.var.alarm_raised");
    // An undeclared variable is refused as well as warned about.
    let found = diagnostics_in("target.var.undeclared_thing", &mind, &host);
    let found: Vec<Code> = found.iter().map(|d| d.code).collect();
    assert_eq!(found, [codes::UNDECLARED_VARIABLE, codes::NOT_BELIEF_SAFE]);

    for source in [
        // The character's own state.
        "self.mass",
        "self.loc",
        "self.has_tag('guard')",
        "self.var.mood",
        "self.var.kills",
        // What belongs to it.
        "self.part('arm').size",
        "self.part('arm').wounds > 0",
        // What is registered as safe: static geometry, safe functions,
        // safe variables.
        "self.loc.x",
        "here.x",
        "here.site.name",
        "dist(here, self.loc)",
        "target.var.kills",
        // The subjects themselves, locals, event fields, pure maths.
        "target == self",
        "target != null",
        "$power * 2 + event.energy",
        "clamp($n, 0, 10)",
        "the(target)",
    ] {
        assert_eq!(diagnostics_in(source, &mind, &host), [], "`{source}`");
    }
    // All of it may be read where truth is.
    for source in ["target.mass", "here.material", "secret()", "exists(target)"] {
        assert_eq!(diagnostics_in(source, &truth, &host), [], "`{source}`");
    }
    // A text template is held to the same rule.
    let outcome = check_template("{the(target)} weighs {target.mass}", host.registry(), &mind);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, codes::NOT_BELIEF_SAFE);
    assert!(outcome.value.is_none());
}

#[test]
fn templates_are_checked_part_by_part() {
    let host = TestHost::new();
    let context = host.context();
    let outcome = check_template(
        "{the(self)} has {self.var.kills:.0} kills.",
        host.registry(),
        &context,
    );
    assert_eq!(outcome.diagnostics, []);
    assert!(outcome.value.is_some());

    let source = "{slef.mass} and {self.mas} and {1 +}";
    let outcome = check_template(source, host.registry(), &context);
    // The syntax error is found first, and alone: nothing is checked
    // until the template parses.
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, codes::UNEXPECTED_TOKEN);

    let source = "{slef.mass} and {self.mas}";
    let outcome = check_template(source, host.registry(), &context);
    let found: Vec<(Code, &str)> = outcome
        .diagnostics
        .iter()
        .map(|d| (d.code, d.span.slice(source)))
        .collect();
    assert_eq!(
        found,
        [
            (codes::UNKNOWN_NAME, "slef"),
            (codes::UNKNOWN_PROPERTY, "self.mas")
        ]
    );
    assert!(outcome.value.is_none());
}

// ---------------------------------------------------------------------
// The registry.

#[test]
fn the_registry_refuses_what_it_cannot_hold() {
    let mut registry = Registry::new();
    let entity = registry.add_kind(KindSpec::new("entity")).unwrap();
    let duplicate = |what, name: &str| RegistryError::Duplicate {
        what,
        name: name.to_owned(),
    };
    assert_eq!(
        registry.add_kind(KindSpec::new("entity")),
        Err(duplicate("kind", "entity"))
    );

    let mass = PropertySpec::new(entity, "mass", Type::Real);
    registry.add_property(mass.clone()).unwrap();
    assert_eq!(
        registry.add_property(mass),
        Err(duplicate("property", "mass"))
    );
    registry
        .add_function(FunctionSpec::new("f", Type::Int))
        .unwrap();
    assert_eq!(
        registry.add_function(FunctionSpec::new("f", Type::Real)),
        Err(duplicate("function", "f"))
    );
    registry.add_event(EventSpec::new("e")).unwrap();
    assert_eq!(
        registry.add_event(EventSpec::new("e")),
        Err(duplicate("event", "e"))
    );
    registry.add_verb(VerbSpec::new("v")).unwrap();
    assert_eq!(
        registry.add_verb(VerbSpec::new("v")),
        Err(duplicate("verb", "v"))
    );
    registry
        .add_variable(VariableSpec::new("m:x", entity, Type::Int))
        .unwrap();
    assert_eq!(
        registry.add_variable(VariableSpec::new("m:x", entity, Type::Int)),
        Err(duplicate("variable", "m:x"))
    );
    registry.add_def_type("material", "").unwrap();
    assert_eq!(
        registry.add_def_type("material", ""),
        Err(duplicate("definition type", "material"))
    );

    // Names the language could not write.
    for bad in ["", "9lives", "two words", "a:b:c", "if", "né"] {
        assert!(
            matches!(
                registry.add_function(FunctionSpec::new(bad, Type::Int)),
                Err(RegistryError::BadName { .. })
            ),
            "`{bad}`"
        );
    }
    assert!(matches!(
        registry.add_kind(KindSpec::new("core:entity")),
        Err(RegistryError::BadName { .. })
    ));
    // `var` reads variables.
    let error = registry
        .add_property(PropertySpec::new(entity, "var", Type::Any))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "`var` is not a usable name for a property: `var` reads variables"
    );

    // A kind that is not registered.
    let stranger = clash_expr::KindId::new(99);
    assert!(matches!(
        registry.add_property(PropertySpec::new(stranger, "x", Type::Int)),
        Err(RegistryError::UnknownKind { .. })
    ));
    assert!(matches!(
        registry.add_event(EventSpec::new("e2").subject("who", stranger)),
        Err(RegistryError::UnknownKind { .. })
    ));
    assert!(matches!(
        registry.add_variable(VariableSpec::new("m:y", stranger, Type::Int)),
        Err(RegistryError::UnknownKind { .. })
    ));

    // A required parameter after an optional one, and a variadic
    // function whose last parameter is optional or missing.
    let backwards = FunctionSpec::new("g", Type::Int)
        .optional_param("a", Type::Int, "")
        .param("b", Type::Int, "");
    assert!(matches!(
        registry.add_function(backwards),
        Err(RegistryError::ParameterOrder { .. })
    ));
    assert!(matches!(
        registry.add_function(FunctionSpec::new("h", Type::Int).variadic()),
        Err(RegistryError::ParameterOrder { .. })
    ));
    let error = registry
        .add_property(
            PropertySpec::new(entity, "p", Type::Int)
                .optional_param("a", Type::Int, "")
                .param("b", Type::Int, ""),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "property `p` has a required parameter after an optional one"
    );
    assert_eq!(
        duplicate("kind", "entity").to_string(),
        "kind `entity` is already registered"
    );
}

#[test]
fn everything_registered_can_be_read_back_in_order_with_its_documentation() {
    let host = TestHost::new();
    let registry = host.registry();

    let kinds: Vec<&str> = registry.kinds().map(|k| k.spec.name.as_str()).collect();
    assert_eq!(kinds, ["entity", "global", "loc", "part", "site"]);
    assert!(registry.kinds().all(|kind| !kind.spec.doc.is_empty()));
    assert_eq!(registry.kind(host.entity).unwrap().id, host.entity);
    assert_eq!(registry.kind_id("loc"), Some(host.loc));
    assert_eq!(registry.kind_id("nothing"), None);

    // Properties: by kind name, then by property name.
    let properties: Vec<String> = registry
        .properties()
        .map(|p| registry.property_signature(p))
        .collect();
    assert_eq!(
        properties,
        [
            "entity.has_tag(tag: string) -> bool",
            "entity.health -> real",
            "entity.loc -> loc",
            "entity.mass -> real",
            "entity.material -> id<material>",
            "entity.materials -> list<id<material>>",
            "entity.name -> string",
            "entity.part(name: string) -> part?",
            "entity.stats -> map<real>",
            "entity.tags -> list<string>",
            "entity.target -> entity?",
            "loc.field(kind: string) -> real",
            "loc.material -> id<material>",
            "loc.site -> site",
            "loc.x -> int",
            "part.size -> real",
            "part.wounds -> int",
            "site.danger -> real?",
            "site.name -> string",
        ]
    );
    for property in registry.properties() {
        assert!(!property.spec.doc.is_empty(), "{}", property.spec.name);
        for param in &property.spec.params {
            assert!(!param.doc.is_empty());
        }
        assert_eq!(registry.property(property.id), Some(property));
        assert_eq!(
            registry.property_id(property.spec.kind, &property.spec.name),
            Some(property.id)
        );
    }
    let of_part: Vec<&str> = registry
        .properties_of(host.part)
        .map(|p| p.spec.name.as_str())
        .collect();
    assert_eq!(of_part, ["size", "wounds"]);
    assert_eq!(registry.properties_of(host.global).count(), 0);
    // Flags.
    let flag = |kind, name: &str| {
        &registry
            .property(registry.property_id(kind, name).unwrap())
            .unwrap()
            .spec
    };
    assert!(flag(host.entity, "health").settable);
    assert_eq!(flag(host.entity, "health").changed_by, ["damaged"]);
    assert!(!flag(host.entity, "mass").settable);
    assert!(!flag(host.entity, "mass").belief_safe);
    assert!(flag(host.loc, "x").belief_safe);
    assert!(flag(host.entity, "part").yields_own);

    // Functions: by name, with signature, purity and belief safety.
    let names: Vec<&str> = registry.functions().map(|f| f.spec.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    for function in registry.functions() {
        assert!(!function.spec.doc.is_empty(), "{}", function.spec.name);
        assert!(!function.spec.group.is_empty(), "{}", function.spec.name);
        assert_eq!(registry.function(function.id), Some(function));
        assert_eq!(registry.function_id(&function.spec.name), Some(function.id));
        assert!(function.min_args() <= function.spec.params.len());
    }
    let function = |name: &str| {
        registry
            .function(registry.function_id(name).unwrap())
            .unwrap()
    };
    assert_eq!(
        registry.signature(function("dist")),
        "dist(a: loc, b: loc) -> real"
    );
    assert_eq!(
        registry.signature(function("ashlands:fall_damage")),
        "ashlands:fall_damage(height: real, mass?: real) -> real"
    );
    assert_eq!(
        registry.signature(function("max")),
        "max(a: number, b: number...) -> number"
    );
    assert_eq!(registry.signature(function("secret")), "secret() -> int");
    assert!(function("roll").spec.random && !function("roll").is_builtin());
    assert!(!function("secret").spec.belief_safe);
    assert!(function("dist").spec.belief_safe);
    assert_eq!(
        function("dist").spec.changed_by,
        Some(vec!["relocated".to_owned()])
    );
    assert_eq!(function("secret").spec.changed_by, None);
    assert_eq!(function("ashlands:fall_damage").min_args(), 1);
    assert_eq!(function("ashlands:fall_damage").max_args(), Some(2));
    assert_eq!(function("max").max_args(), None);

    // Events: subjects, fields, hooks and what a handler may change.
    let events: Vec<&str> = registry.events().map(|e| e.spec.name.as_str()).collect();
    assert_eq!(events, ["damage_incoming", "damaged"]);
    let damaged = registry.event(host.damaged).unwrap();
    assert_eq!(registry.event_id("damaged"), Some(host.damaged));
    assert_eq!(
        damaged.spec.subjects,
        [
            ("target".to_owned(), host.entity),
            ("here".to_owned(), host.loc)
        ]
    );
    let fields: Vec<(&str, String)> = damaged
        .spec
        .fields
        .iter()
        .map(|f| (f.name.as_str(), registry.type_name(&f.ty)))
        .collect();
    assert_eq!(
        fields,
        [
            ("energy", "real".to_owned()),
            ("kind", "string".to_owned()),
            ("amount", "int".to_owned()),
            ("source", "entity?".to_owned()),
        ]
    );
    assert!(!damaged.spec.hook);
    let hook = registry
        .event(registry.event_id("damage_incoming").unwrap())
        .unwrap();
    assert!(hook.spec.hook);
    assert_eq!(hook.spec.mutable, ["energy"]);
    assert!(registry.events().all(|e| !e.spec.doc.is_empty()));
    assert!(
        registry
            .events()
            .all(|e| e.spec.fields.iter().all(|f| !f.doc.is_empty()))
    );

    // Verbs: arguments, which are required, and the direct one.
    let verbs: Vec<&str> = registry.verbs().map(|v| v.spec.name.as_str()).collect();
    assert_eq!(verbs, ["damage", "message"]);
    let message = registry.verb(registry.verb_id("message").unwrap()).unwrap();
    assert_eq!(message.spec.direct.as_deref(), Some("text"));
    assert_eq!(
        message.spec.args,
        [
            ParamSpec::new("text", Type::Str, "The line to show."),
            ParamSpec::optional("to", Type::Subject(host.entity), "Whose log it goes in."),
        ]
    );
    assert!(registry.verbs().all(|v| !v.spec.doc.is_empty()));
    assert_eq!(registry.verb_id("nothing"), None);

    // Variables.
    let variables: Vec<&str> = registry.variables().map(|v| v.spec.name.as_str()).collect();
    assert_eq!(
        variables,
        [
            "ashlands:kills",
            "core:alarm_raised",
            "core:day",
            "core:kills",
            "core:mood"
        ]
    );
    let kills = registry
        .variable(registry.variable_id("core:kills").unwrap())
        .unwrap();
    assert_eq!(kills.spec.kind, host.entity);
    assert!(kills.spec.belief_safe);
    assert!(registry.variables().all(|v| !v.spec.doc.is_empty()));

    // Definition types and their ids, in order of id.
    let def_types: Vec<&str> = registry.def_types().map(|d| d.name.as_str()).collect();
    assert_eq!(def_types, ["block", "material"]);
    let material = registry.def_type(host.material).unwrap();
    let ids: Vec<(&str, u32)> = material.ids().collect();
    assert_eq!(
        ids,
        [
            ("ashlands:ash", 21),
            ("ashlands:slag", 22),
            ("core:ash", 11),
            ("core:glass", 12),
            ("core:iron", 13),
        ]
    );
    let glass = registry.def_ref(host.material, "core:glass").unwrap();
    assert_eq!(glass.index, 12);
    assert_eq!(registry.def_name(glass), Some("core:glass"));
    assert_eq!(registry.def_ref(host.material, "glass"), None);
    assert_eq!(registry.def_ref(host.block, "core:glass").unwrap().index, 1);
    assert_eq!(registry.def_type_id("material"), Some(host.material));
}

#[test]
fn definition_ids_can_be_replaced_when_mods_are_linked() {
    let mut host = TestHost::new();
    let material = host.material;
    host.registry_mut().set_def_ids(
        material,
        [("core:glass", 1), ("core:stone", 2), ("core:glass", 5)],
    );
    let registry = host.registry();
    // An id given twice keeps its last number.
    assert_eq!(registry.def_ref(material, "core:glass").unwrap().index, 5);
    assert_eq!(registry.def_ref(material, "core:iron"), None);
    assert_eq!(registry.def_type(material).unwrap().ids().count(), 2);
    let context = host.context();
    let outcome = check("self.material == 'iron'", registry, &context, &Type::Bool);
    assert_eq!(outcome.diagnostics[0].code, codes::UNKNOWN_ID);
    let outcome = check("self.material == 'stone'", registry, &context, &Type::Bool);
    assert_eq!(outcome.diagnostics, []);
    // A type that is not registered is ignored.
    host.registry_mut()
        .set_def_ids(clash_expr::DefTypeId::new(99), [("a:b", 1)]);
}

#[test]
fn names_resolve_through_a_scope() {
    let host = TestHost::new();
    let registry = host.registry();
    let core = Scope::of("core");
    let both = Scope {
        namespace: Some("core".to_owned()),
        imports: vec!["ashlands".to_owned()],
    };
    let none = Scope::default();

    let found = registry.resolve_def(host.material, "glass", &core).unwrap();
    assert_eq!(found.name, "core:glass");
    assert_eq!(found.also_in, None);
    // The mod's own namespace wins; the import is noted.
    let found = registry.resolve_def(host.material, "ash", &both).unwrap();
    assert_eq!(found.name, "core:ash");
    assert_eq!(found.also_in.as_deref(), Some("ashlands"));
    // Found only in the import.
    let found = registry.resolve_def(host.material, "slag", &both).unwrap();
    assert_eq!(found.name, "ashlands:slag");
    assert_eq!(found.also_in, None);
    assert!(registry.resolve_def(host.material, "slag", &core).is_none());
    // A qualified name is exact, whatever the scope.
    assert!(
        registry
            .resolve_def(host.material, "ashlands:slag", &none)
            .is_some()
    );
    assert!(
        registry
            .resolve_def(host.material, "glass", &none)
            .is_none()
    );

    // The engine's functions come first for a bare name.
    assert_eq!(
        registry.resolve_function("clamp", &both).unwrap().name,
        "clamp"
    );
    assert_eq!(
        registry.resolve_function("double", &both).unwrap().name,
        "core:double"
    );
    assert_eq!(
        registry
            .resolve_function("double", &Scope::of("ashlands"))
            .unwrap()
            .name,
        "ashlands:double"
    );
    assert!(registry.resolve_function("double", &none).is_none());
    assert_eq!(
        registry.resolve_variable("kills", &core).unwrap().name,
        "core:kills"
    );
    assert!(registry.resolve_variable("kills", &none).is_none());
}

#[test]
fn types_are_written_and_read_the_same_way() {
    let host = TestHost::new();
    let registry = host.registry();
    for text in [
        "any",
        "null",
        "bool",
        "int",
        "real",
        "number",
        "string",
        "entity",
        "loc",
        "id<material>",
        "list<int>",
        "map<real>",
        "list<list<id<block>>>",
        "map<entity?>",
        "real?",
        "list<int>?",
    ] {
        let ty = registry
            .parse_type(text)
            .unwrap_or_else(|| panic!("`{text}`"));
        assert_eq!(registry.type_name(&ty), text);
    }
    assert_eq!(registry.parse_type("list"), Some(Type::list(Type::Any)));
    assert_eq!(registry.parse_type("map"), Some(Type::map(Type::Any)));
    assert_eq!(
        registry.parse_type(" list < int > "),
        Some(Type::list(Type::Int))
    );
    assert_eq!(registry.parse_type("real??"), Some(Type::Real.nullable()));
    for bad in ["", "wibble", "id<wibble>", "list<wibble>", "id", "list<int"] {
        assert_eq!(registry.parse_type(bad), None, "`{bad}`");
    }
    // Ids that are not registered still have a name.
    assert_eq!(
        registry.type_name(&Type::Subject(clash_expr::KindId::new(99))),
        "subject<?>"
    );
    assert_eq!(
        registry.type_name(&Type::Id(clash_expr::DefTypeId::new(99))),
        "id<?>"
    );
}

#[test]
fn the_built_in_registry_needs_no_host() {
    // The functions of 05 §4.7 that belong to no subsystem can be used
    // with nothing else registered.
    let registry = Registry::with_builtins();
    let context = Context::new().local("x", Type::Real);
    let outcome = check(
        "clamp(sqrt($x), 0, 1) > 0.5",
        &registry,
        &context,
        &Type::Bool,
    );
    assert_eq!(outcome.diagnostics, []);
    // An empty registry has no functions at all.
    let outcome = check("clamp(1, 2, 3)", &Registry::new(), &context, &Type::Any);
    assert_eq!(outcome.diagnostics[0].code, codes::UNKNOWN_FUNCTION);
}

#[test]
fn what_is_own_follows_locals_and_elements() {
    let host = TestHost::new();
    let entity = host.entity(1).kind;
    let mut mind = Context::new()
        .own_subject("party", Type::list(Type::Subject(entity)))
        .subject("others", Type::list(Type::Subject(entity)))
        .local("stranger", Type::Subject(entity))
        .belief();
    mind.add_own_local("member", Type::Subject(entity));

    // An element of the character's own list is its own, and so is a
    // local declared as own.
    for source in ["party[0].mass", "$member.mass", "$member.part('arm').size"] {
        assert_eq!(diagnostics_in(source, &mind, &host), [], "`{source}`");
    }
    // An element of any other list and an ordinary local are not.
    for source in ["others[0].mass", "$stranger.mass"] {
        let found = diagnostics_in(source, &mind, &host);
        assert_eq!(found.len(), 1, "`{source}`: {found:#?}");
        assert_eq!(found[0].code, codes::NOT_BELIEF_SAFE, "`{source}`");
    }

    // Whether a value is own can be asked, to bind it by another name.
    let own = |source: &str| {
        check(source, host.registry(), &mind, &Type::Any)
            .value
            .is_some_and(|checked| checked.is_own())
    };
    assert!(own("party"));
    assert!(own("party[0]"));
    assert!(own("$member"));
    assert!(own("$member.part('arm')"));
    assert!(!own("others"));
    assert!(!own("$member.mass"), "a number belongs to nobody");
    assert!(!own("$stranger"));
}
