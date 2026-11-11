//! Places: what the `set` and `add` effects may assign to (05 §10.3).

use clash_expr::testing::{TestHost, slot};
use clash_expr::{Machine, PlaceRoot, Type, Value, codes, compile_place};

#[test]
fn places_are_what_set_and_add_may_assign_to() {
    let mut host = TestHost::new();
    let here = host.place(100);
    host.set(here, "site", host.site(9));
    host.set_local(slot::NAME, "iron");
    let context = host.context();
    let registry = host.registry().clone();
    let place = |source: &str| {
        let outcome = compile_place(source, &registry, &context);
        assert!(
            !outcome.has_errors(),
            "`{source}`: {:?}",
            outcome.diagnostics
        );
        outcome.value.unwrap()
    };

    // A local and an event field live on no subject.
    let found = place("$power");
    assert_eq!(found.root(), &PlaceRoot::Local(slot::POWER));
    assert!(found.subject().is_none());
    assert!(found.keys().is_empty());
    assert_eq!(found.ty(), &Type::Real);
    let found = place("event.energy");
    assert_eq!(found.root(), &PlaceRoot::EventField(slot::ENERGY));
    assert!(found.subject().is_none());

    // A variable: its qualified name, and whose it is.
    let found = place("self.var.kills");
    assert_eq!(found.root(), &PlaceRoot::Variable("core:kills".into()));
    assert_eq!(found.ty(), &Type::Int);
    let whose = found.subject().unwrap().clone();
    let found = place("here.site.var.alarm_raised");
    assert_eq!(
        found.root(),
        &PlaceRoot::Variable("core:alarm_raised".into())
    );
    assert_eq!(found.ty(), &Type::Bool);
    let site = found.subject().unwrap().clone();
    let found = place("self.var('ashlands:kills')");
    assert_eq!(found.root(), &PlaceRoot::Variable("ashlands:kills".into()));
    // What is assigned is a value, not "a value or null".
    let found = place("self.var('core:mood')");
    assert_eq!(found.ty(), &Type::Str);

    // An entry of a map held in a local or a variable: the keys follow.
    let found = place("$m['ash']");
    assert_eq!(found.root(), &PlaceRoot::Local(slot::M));
    assert_eq!(found.keys().len(), 1);
    assert_eq!(found.ty(), &Type::Real);
    let ash = found.keys()[0].clone();
    let found = place("$m.ash");
    assert_eq!(found.keys().len(), 1);
    let found = place("$x['a'][$name]");
    assert_eq!(found.root(), &PlaceRoot::Local(slot::X));
    assert_eq!(found.keys().len(), 2);
    let inner = found.keys()[1].clone();

    // A settable property.
    let found = place("target.health");
    let health = registry.property_id(host.entity, "health").unwrap();
    assert_eq!(found.root(), &PlaceRoot::Property(health));
    let target = found.subject().unwrap().clone();

    // The subject and the keys are evaluated when the effect runs.
    let mut machine = Machine::new();
    assert_eq!(
        machine.eval(&whose, &mut host),
        Value::Subject(host.entity(1))
    );
    assert_eq!(machine.eval(&site, &mut host), Value::Subject(host.site(9)));
    assert_eq!(
        machine.eval(&target, &mut host),
        Value::Subject(host.entity(2))
    );
    assert_eq!(machine.eval(&ash, &mut host), Value::text("ash"));
    assert_eq!(machine.eval(&inner, &mut host), Value::text("iron"));

    // What is not a place is refused, with what a place is.
    for source in [
        "self.mass",
        "self.has_tag('guard')",
        "self",
        "1 + 2",
        "clamp(1, 2, 3)",
        "[1, 2][0]",
        "self.tags[0]",
        "-$power",
    ] {
        let outcome = compile_place(source, &registry, &context);
        assert_eq!(outcome.diagnostics.len(), 1, "`{source}`");
        assert_eq!(
            outcome.diagnostics[0].code,
            codes::NOT_ASSIGNABLE,
            "`{source}`"
        );
        assert_eq!(outcome.diagnostics[0].span.slice(source), source);
        assert!(outcome.diagnostics[0].suggestion.is_some());
        assert!(outcome.value.is_none());
    }
    let outcome = compile_place("self.mass", &registry, &context);
    assert_eq!(
        outcome.diagnostics[0].message,
        "`entity.mass -> real` cannot be set"
    );
    // Mistakes in the place itself are reported as in any expression.
    let outcome = compile_place("slef.var.kills", &registry, &context);
    assert_eq!(outcome.diagnostics[0].code, codes::UNKNOWN_NAME);
    let outcome = compile_place("self.var.", &registry, &context);
    assert_eq!(outcome.diagnostics[0].code, codes::UNEXPECTED_END);
    // An undeclared variable can be assigned, with the usual warning.
    let outcome = compile_place("self.var.tally['ash']", &registry, &context);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, codes::UNDECLARED_VARIABLE);
    let found = outcome.value.unwrap();
    assert_eq!(found.root(), &PlaceRoot::Variable("core:tally".into()));
    assert_eq!(found.keys().len(), 1);
}

#[test]
fn a_number_is_read_as_the_type_that_is_wanted() {
    let mut host = TestHost::new();
    host.set_local(slot::N, 4);
    host.set_local(slot::POWER, TestHost::real(2.5));
    let context = host.context();
    let mut machine = Machine::new();
    let mut eval = |source: &str, expected: &Type| {
        let compiled = clash_expr::compile(source, host.registry(), &context, expected)
            .into_result()
            .unwrap();
        (
            machine.eval_real(&compiled, &mut host),
            machine.eval_int(&compiled, &mut host),
        )
    };
    // An int is accepted where a real is wanted, and stays an int.
    let (real, int) = eval("$n", &Type::Real);
    assert_eq!(real, Some(clash_core::Real::of(4.0)));
    assert_eq!(int, Some(4));
    let (real, int) = eval("$power * 2", &Type::Real);
    assert_eq!(real, Some(clash_core::Real::of(5.0)));
    assert_eq!(int, None);
    // Null and other types are neither.
    assert_eq!(eval("$x", &Type::Any), (None, None));
    assert_eq!(eval("'four'", &Type::Any), (None, None));
}
