//! X1: the parser and the printer.
//!
//! Every production of the grammar (05 §4.1) has fixtures that parse and
//! fixtures that fail with a code at a position. Printing a tree and
//! parsing it again gives the same tree, for random trees. A syntax error
//! inside a JSON string is mapped to the right column of the file.

use clash_core::{Real, Rng};
use clash_expr::{
    BinaryOp, Diagnostic, Expr, ExprKind, Format, MAX_NESTING, MAX_SOURCE_LEN, MAX_TREE_HEIGHT,
    Name, Number, Span, Template, TemplatePart, UnaryOp, Unit, codes, is_keyword,
    offset_in_json_string, parse_expr, parse_template, print_expr, print_template,
};
use proptest::prelude::*;

/// Parses, and checks that the canonical print is as given and reads
/// back to the same tree.
fn canon(source: &str, printed: &str) -> Expr {
    let tree = parse_expr(source).unwrap_or_else(|error| panic!("`{source}`: {error}"));
    assert_eq!(print_expr(&tree), printed, "printing `{source}`");
    let again = parse_expr(printed).unwrap_or_else(|error| panic!("`{printed}`: {error}"));
    assert_eq!(
        again.without_spans(),
        tree.without_spans(),
        "reparsing `{printed}`"
    );
    tree
}

/// Parses something already in canonical form.
fn same(source: &str) -> Expr {
    canon(source, source)
}

/// Fails to parse, with this code at this byte.
fn fails(source: &str, code: clash_expr::Code, at: u32) -> Diagnostic {
    let error = match parse_expr(source) {
        Ok(tree) => panic!("`{source}` parsed as `{}`", print_expr(&tree)),
        Err(error) => error,
    };
    assert_eq!(error.code, code, "`{source}`: {error}");
    assert_eq!(error.span.start, at, "`{source}`: {error}");
    assert!(error.is_error());
    assert!(!error.message.is_empty());
    error
}

fn name(text: &str) -> Expr {
    Expr::new(ExprKind::Name(Name::bare(text)), Span::default())
}

fn int(value: i64) -> Expr {
    Expr::new(
        ExprKind::Number {
            value: Number::Int(value),
            unit: None,
        },
        Span::default(),
    )
}

fn binary(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        Span::default(),
    )
}

fn unary(op: UnaryOp, operand: Expr) -> Expr {
    Expr::new(
        ExprKind::Unary {
            op,
            operand: Box::new(operand),
        },
        Span::default(),
    )
}

fn shape(source: &str) -> Expr {
    parse_expr(source).unwrap().without_spans()
}

#[test]
fn production_expr() {
    same("if a then b else c");
    same("if a then b else if c then d else e");
    same("if if a then b else c then d else e");
    same("if a then if b then c else d else e");
    canon("(if a then b else c) + 1", "(if a then b else c) + 1");
    let tree = shape("if a then 1 else 2");
    assert_eq!(
        tree,
        Expr::new(
            ExprKind::If {
                cond: Box::new(name("a")),
                then: Box::new(int(1)),
                otherwise: Box::new(int(2)),
            },
            Span::default()
        )
    );

    fails("if a then b", codes::UNEXPECTED_END, 11);
    fails("if a b else c", codes::UNEXPECTED_TOKEN, 5);
    fails("if then b else c", codes::UNEXPECTED_TOKEN, 3);
    fails("if a then b else", codes::UNEXPECTED_END, 16);
    fails("1 + if a then b else c", codes::UNEXPECTED_TOKEN, 4);
    fails("a then b", codes::UNEXPECTED_TOKEN, 2);
    fails("a else b", codes::UNEXPECTED_TOKEN, 2);
}

#[test]
fn production_or() {
    same("a or b");
    assert_eq!(
        shape("a or b or c"),
        binary(
            BinaryOp::Or,
            binary(BinaryOp::Or, name("a"), name("b")),
            name("c")
        )
    );
    canon("a or (b or c)", "a or (b or c)");
    // `and` binds tighter than `or`.
    assert_eq!(
        shape("a or b and c"),
        binary(
            BinaryOp::Or,
            name("a"),
            binary(BinaryOp::And, name("b"), name("c"))
        )
    );
    canon("(a or b) and c", "(a or b) and c");

    fails("a or", codes::UNEXPECTED_END, 4);
    fails("or a", codes::UNEXPECTED_TOKEN, 0);
    fails("a or or b", codes::UNEXPECTED_TOKEN, 5);
}

#[test]
fn production_and() {
    same("a and b");
    assert_eq!(
        shape("a and b and c"),
        binary(
            BinaryOp::And,
            binary(BinaryOp::And, name("a"), name("b")),
            name("c")
        )
    );
    canon("a and (b and c)", "a and (b and c)");
    same("not a and not b");

    fails("a and", codes::UNEXPECTED_END, 5);
    fails("and a", codes::UNEXPECTED_TOKEN, 0);
    fails("a && b", codes::UNEXPECTED_CHARACTER, 2);
}

#[test]
fn production_not() {
    same("not a");
    same("not not a");
    // `not` is looser than a comparison and tighter than `and`.
    assert_eq!(
        shape("not a == b"),
        unary(UnaryOp::Not, binary(BinaryOp::Eq, name("a"), name("b")))
    );
    assert_eq!(
        shape("not a and b"),
        binary(BinaryOp::And, unary(UnaryOp::Not, name("a")), name("b"))
    );
    canon("not (a and b)", "not (a and b)");
    canon("(not a) == b", "(not a) == b");

    fails("not", codes::UNEXPECTED_END, 3);
    fails("a not b", codes::UNEXPECTED_TOKEN, 2);
    fails("1 + not a", codes::UNEXPECTED_TOKEN, 4);
    fails("!a", codes::UNEXPECTED_CHARACTER, 0);
}

#[test]
fn production_compare() {
    for (source, op) in [
        ("a == b", BinaryOp::Eq),
        ("a != b", BinaryOp::Ne),
        ("a < b", BinaryOp::Lt),
        ("a <= b", BinaryOp::Le),
        ("a > b", BinaryOp::Gt),
        ("a >= b", BinaryOp::Ge),
        ("a in b", BinaryOp::In),
    ] {
        same(source);
        assert_eq!(shape(source), binary(op, name("a"), name("b")));
        assert!(op.is_comparison());
    }
    assert!(!BinaryOp::Add.is_comparison());
    // Arithmetic and `??` bind tighter.
    assert_eq!(
        shape("a + 1 < b ?? 2"),
        binary(
            BinaryOp::Lt,
            binary(BinaryOp::Add, name("a"), int(1)),
            binary(BinaryOp::Coalesce, name("b"), int(2))
        )
    );
    canon("(a < b) == c", "(a < b) == c");
    canon("a == (b < c)", "a == (b < c)");

    // Comparisons do not chain.
    let error = fails("a < b < c", codes::CHAINED_COMPARISON, 6);
    assert!(error.suggestion.is_some());
    fails("a == b != c", codes::CHAINED_COMPARISON, 7);
    fails("a in b in c", codes::CHAINED_COMPARISON, 7);
    fails("a <", codes::UNEXPECTED_END, 3);
    fails("a == == b", codes::UNEXPECTED_TOKEN, 5);
    fails("== a", codes::UNEXPECTED_TOKEN, 0);
    fails("a = b", codes::UNEXPECTED_CHARACTER, 2);
    fails("a in", codes::UNEXPECTED_END, 4);
}

#[test]
fn production_coalesce() {
    same("a ?? b");
    assert_eq!(
        shape("a ?? b ?? c"),
        binary(
            BinaryOp::Coalesce,
            binary(BinaryOp::Coalesce, name("a"), name("b")),
            name("c")
        )
    );
    canon("a ?? (b ?? c)", "a ?? (b ?? c)");
    // A sum binds tighter than `??`.
    assert_eq!(
        shape("a ?? b + 1"),
        binary(
            BinaryOp::Coalesce,
            name("a"),
            binary(BinaryOp::Add, name("b"), int(1))
        )
    );
    canon("(a ?? b) + 1", "(a ?? b) + 1");

    fails("a ??", codes::UNEXPECTED_END, 4);
    fails("?? a", codes::UNEXPECTED_TOKEN, 0);
    fails("a ? b", codes::UNEXPECTED_CHARACTER, 2);
}

#[test]
fn production_sum() {
    same("a + b");
    same("a - b");
    assert_eq!(
        shape("a + b - c"),
        binary(
            BinaryOp::Sub,
            binary(BinaryOp::Add, name("a"), name("b")),
            name("c")
        )
    );
    canon("a - (b - c)", "a - (b - c)");
    canon("(a - b) - c", "a - b - c");
    assert_eq!(
        shape("a + b * c"),
        binary(
            BinaryOp::Add,
            name("a"),
            binary(BinaryOp::Mul, name("b"), name("c"))
        )
    );

    fails("a +", codes::UNEXPECTED_END, 3);
    fails("a + * b", codes::UNEXPECTED_TOKEN, 4);
    fails("+ a", codes::UNEXPECTED_TOKEN, 0);
}

#[test]
fn production_term() {
    same("a * b");
    same("a / b");
    same("a % b");
    assert_eq!(
        shape("a * b / c % d"),
        binary(
            BinaryOp::Rem,
            binary(
                BinaryOp::Div,
                binary(BinaryOp::Mul, name("a"), name("b")),
                name("c")
            ),
            name("d")
        )
    );
    canon("a / (b * c)", "a / (b * c)");
    canon("(a + b) * c", "(a + b) * c");
    same("a * -b");

    fails("a *", codes::UNEXPECTED_END, 3);
    fails("* a", codes::UNEXPECTED_TOKEN, 0);
    fails("a / / b", codes::UNEXPECTED_TOKEN, 4);
    fails("a %", codes::UNEXPECTED_END, 3);
}

#[test]
fn production_unary() {
    same("-a");
    same("--a");
    canon("- a", "-a");
    // A negative literal is a negation.
    assert_eq!(shape("-5"), unary(UnaryOp::Neg, int(5)));
    assert_eq!(
        shape("a - -b"),
        binary(BinaryOp::Sub, name("a"), unary(UnaryOp::Neg, name("b")))
    );
    canon("-(a + b)", "-(a + b)");

    fails("-", codes::UNEXPECTED_END, 1);
    fails("a -", codes::UNEXPECTED_END, 3);
    fails("- *", codes::UNEXPECTED_TOKEN, 2);
}

#[test]
fn production_power() {
    same("a ^ b");
    // The exponent is a `unary`, so powers group to the right and a
    // minus in front applies to the whole power.
    assert_eq!(
        shape("a ^ b ^ c"),
        binary(
            BinaryOp::Pow,
            name("a"),
            binary(BinaryOp::Pow, name("b"), name("c"))
        )
    );
    assert_eq!(
        shape("-a ^ b"),
        unary(UnaryOp::Neg, binary(BinaryOp::Pow, name("a"), name("b")))
    );
    assert_eq!(
        shape("a ^ -b ^ c"),
        binary(
            BinaryOp::Pow,
            name("a"),
            unary(UnaryOp::Neg, binary(BinaryOp::Pow, name("b"), name("c")))
        )
    );
    canon("(a ^ b) ^ c", "(a ^ b) ^ c");
    canon("(-a) ^ b", "(-a) ^ b");
    canon("a * b ^ 2", "a * b ^ 2");

    fails("a ^", codes::UNEXPECTED_END, 3);
    fails("^ a", codes::UNEXPECTED_TOKEN, 0);
    fails("a ^ ^ b", codes::UNEXPECTED_TOKEN, 4);
}

#[test]
fn production_postfix() {
    same("a.b");
    same("a.b.c");
    same("a.b(1, 2)");
    same("f()");
    same("f(1)");
    same("xs[0]");
    same("m['k'].x[1](2)");
    same("a.ns:b");
    same("here.field('fire')");
    same("self.action('block').phase");
    canon("( a + b ).c", "(a + b).c");
    canon("(a)(b)", "a(b)");
    canon("(-a)[0]", "(-a)[0]");
    let tree = shape("a.b(1)[2]");
    let ExprKind::Index { object, index } = tree.kind else {
        panic!("not an index");
    };
    assert_eq!(*index, int(2));
    let ExprKind::Call { callee, args } = object.kind else {
        panic!("not a call");
    };
    assert_eq!(args, [int(1)]);
    assert_eq!(
        callee.kind,
        ExprKind::Member {
            object: Box::new(name("a")),
            name: Name::bare("b")
        }
    );

    fails("a.", codes::UNEXPECTED_END, 2);
    fails("a.1", codes::UNEXPECTED_TOKEN, 2);
    fails("a.(b)", codes::UNEXPECTED_TOKEN, 2);
    fails("a.if", codes::UNEXPECTED_TOKEN, 2);
    fails("f(1,", codes::UNEXPECTED_END, 4);
    fails("f(1 2)", codes::UNEXPECTED_TOKEN, 4);
    fails("f(1", codes::UNEXPECTED_END, 3);
    fails("xs[", codes::UNEXPECTED_END, 3);
    fails("xs[1", codes::UNEXPECTED_END, 4);
    fails("xs[]", codes::UNEXPECTED_TOKEN, 3);
    fails("xs[1, 2]", codes::UNEXPECTED_TOKEN, 4);
}

#[test]
fn production_primary() {
    same("42");
    same("3.5");
    same("true");
    same("false");
    same("null");
    same("self");
    same("$power");
    same("'ash'");
    same("''");
    same("'it''s'");
    same("[]");
    same("[1, 2, 3]");
    same("[[1], []]");
    same("{}");
    same("{'ash': 3, 'iron': 1}");
    canon("(a)", "a");
    canon("((a))", "a");
    canon("( 1 )", "1");
    // Reals are printed so that they read back as reals, to the bit.
    canon("1e6", "1000000.0");
    canon("1E21", "1e21");
    canon("0.0000001", "1e-7");
    canon("2.50", "2.5");
    canon("3.0", "3.0");
    assert_eq!(
        shape("1e6").kind,
        ExprKind::Number {
            value: Number::Real(Real::of(1e6)),
            unit: None
        }
    );
    // A bracketed tree's span covers the brackets.
    assert_eq!(parse_expr(" ( a ) ").unwrap().span, Span::new(1, 6));

    fails("", codes::EMPTY_EXPRESSION, 0);
    fails("   ", codes::EMPTY_EXPRESSION, 3);
    fails("(", codes::UNEXPECTED_END, 1);
    fails("(a", codes::UNEXPECTED_END, 2);
    fails("()", codes::UNEXPECTED_TOKEN, 1);
    fails(")", codes::UNEXPECTED_TOKEN, 0);
    fails("a)", codes::UNEXPECTED_TOKEN, 1);
    fails("[1,", codes::UNEXPECTED_END, 3);
    fails("[1 2]", codes::UNEXPECTED_TOKEN, 3);
    fails("[1,]", codes::UNEXPECTED_TOKEN, 3);
    fails("1 2", codes::UNEXPECTED_TOKEN, 2);
    fails("a b", codes::UNEXPECTED_TOKEN, 2);
    fails("$", codes::UNEXPECTED_END, 1);
    fails("$ x", codes::UNEXPECTED_TOKEN, 2);
    fails("$1", codes::UNEXPECTED_TOKEN, 1);
    fails("'abc", codes::UNTERMINATED_STRING, 0);
    fails("\"abc\"", codes::UNEXPECTED_CHARACTER, 0);
    fails("a # b", codes::UNEXPECTED_CHARACTER, 2);
    fails("99999999999999999999", codes::BAD_NUMBER, 0);
    fails("1e999", codes::BAD_NUMBER, 0);
}

#[test]
fn production_number_unit() {
    let literal = |source: &str| match shape(source).kind {
        ExprKind::Number { value, unit } => (value, unit.map(Unit::symbol)),
        other => panic!("`{source}` is {other:?}"),
    };
    assert_eq!(literal("300ms"), (Number::Int(300), Some("ms")));
    assert_eq!(literal("70kPa"), (Number::Int(70), Some("kPa")));
    assert_eq!(literal("2h"), (Number::Int(2), Some("h")));
    assert_eq!(literal("1.5MJ"), (Number::Real(Real::of(1.5)), Some("MJ")));
    assert_eq!(literal("1e3kg"), (Number::Real(Real::of(1e3)), Some("kg")));
    // A space before the unit is allowed, and printing closes it up.
    canon("90 s", "90s");
    canon("2 h * 3", "2h * 3");
    same("300ms");
    same("1.5kPa");
    same("[1s, 2min, 3h, 4d]");
    same("5min + min(1, 2)");
    // Every simple unit is accepted.
    for unit in Unit::all() {
        same(&format!("7{}", unit.symbol()));
    }

    let error = fails("3foo", codes::UNKNOWN_UNIT, 1);
    assert!(error.suggestion.unwrap().starts_with("the units are"));
    let error = fails("3kpa", codes::UNKNOWN_UNIT, 1);
    assert_eq!(error.suggestion.as_deref(), Some("did you mean `kPa`?"));
    // `C` is an offset, not a scale.
    let error = fails("5C", codes::UNKNOWN_UNIT, 1);
    assert!(error.suggestion.unwrap().contains("offset"));
    // Compound units belong in JSON unit strings: here the slash divides.
    canon("3m/s", "3m / s");
    // A name after a space that is not a unit is a missing operator.
    fails("3 foo", codes::UNEXPECTED_TOKEN, 2);
    // A unit follows a number and nothing else.
    fails("(3)ms", codes::UNEXPECTED_TOKEN, 3);
}

#[test]
fn production_args() {
    same("f(a, b, c)");
    same("f(if a then b else c, [1, 2], g(h()))");
    let ExprKind::Call { args, .. } = shape("f(a, 1)").kind else {
        panic!("not a call");
    };
    assert_eq!(args, [name("a"), int(1)]);

    fails("f(a,)", codes::UNEXPECTED_TOKEN, 4);
    fails("f(,)", codes::UNEXPECTED_TOKEN, 2);
    fails("f(a b)", codes::UNEXPECTED_TOKEN, 4);
    fails("f(a; b)", codes::UNEXPECTED_CHARACTER, 3);
}

#[test]
fn production_entry() {
    same("{'a': 1}");
    same("{'a': [1], 'b': {'c': null}}");
    canon("{a: 1}", "{a: 1}");
    canon("{ 'a' : 1 }", "{'a': 1}");
    canon("{$k: $v}", "{$k: $v}");
    let ExprKind::Map(entries) = shape("{'b': 2, 'a': 1}").kind else {
        panic!("not a map");
    };
    // Entries are kept in the order written.
    assert_eq!(entries[0].1, int(2));
    assert_eq!(entries[1].1, int(1));

    fails("{'a' 1}", codes::UNEXPECTED_TOKEN, 5);
    fails("{'a': }", codes::UNEXPECTED_TOKEN, 6);
    fails("{'a': 1", codes::UNEXPECTED_END, 7);
    fails("{'a': 1,}", codes::UNEXPECTED_TOKEN, 8);
    fails("{'a'}", codes::UNEXPECTED_TOKEN, 4);
    // With nothing between them, `a:b` is a qualified name, not an entry.
    fails("{a:b}", codes::UNEXPECTED_TOKEN, 4);
    fails("{", codes::UNEXPECTED_END, 1);
}

#[test]
fn production_name() {
    same("clamp(1, 2, 3)");
    same("ashlands:fall_damage(1)");
    same("$core:x");
    assert_eq!(
        shape("ashlands:fall_damage").kind,
        ExprKind::Name(Name::qualified("ashlands", "fall_damage"))
    );
    assert!(Name::qualified("a", "b").is_qualified());
    assert!(!Name::bare("b").is_qualified());
    assert_eq!(Name::qualified("a", "b").to_string(), "a:b");

    fails("a:", codes::UNEXPECTED_TOKEN, 1);
    fails(":a", codes::UNEXPECTED_TOKEN, 0);
    fails("a:b:c", codes::UNEXPECTED_TOKEN, 3);
    fails("a :b", codes::UNEXPECTED_TOKEN, 2);
}

#[test]
fn whitespace_is_free() {
    canon("  a\t+\nb  ", "a + b");
    canon("f ( a , b )", "f(a, b)");
    canon("a . b", "a.b");
    canon("xs [ 0 ]", "xs[0]");
    assert_eq!(parse_expr("  a + b ").unwrap().span, Span::new(2, 7));
}

#[test]
fn spans_cover_what_was_written() {
    let tree = parse_expr("self.kills * 2 >= $limit").unwrap();
    assert_eq!(tree.span, Span::new(0, 24));
    let ExprKind::Binary { lhs, rhs, .. } = &tree.kind else {
        panic!("not a comparison");
    };
    assert_eq!(lhs.span.slice("self.kills * 2 >= $limit"), "self.kills * 2");
    assert_eq!(rhs.span.slice("self.kills * 2 >= $limit"), "$limit");
}

#[test]
fn deep_and_long_expressions_are_refused_without_overflowing() {
    // Brackets.
    let depth = MAX_NESTING as usize + 1;
    let source = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    fails(&source, codes::TOO_DEEP, MAX_NESTING);
    let source = format!("{}1{}", "(".repeat(20), ")".repeat(20));
    canon(&source, "1");
    // Prefix operators.
    fails(&"-".repeat(10_000), codes::TOO_DEEP, MAX_NESTING);
    fails(&"not ".repeat(10_000), codes::TOO_DEEP, MAX_NESTING * 4);
    fails(&"[".repeat(10_000), codes::TOO_DEEP, MAX_NESTING);
    fails(&"{".repeat(10_000), codes::TOO_DEEP, MAX_NESTING);
    fails(&"if ".repeat(10_000), codes::TOO_DEEP, MAX_NESTING * 3);
    fails(&"a^".repeat(10_000), codes::TOO_DEEP, MAX_NESTING * 2);
    // A chain of operators has no nesting, but its tree is as deep as it
    // is long.
    let chain = vec!["1"; MAX_TREE_HEIGHT as usize + 1].join("+");
    let error = fails(&chain, codes::TOO_DEEP, 0);
    assert!(error.suggestion.is_some());
    let chain = vec!["1"; MAX_TREE_HEIGHT as usize - 1].join("+");
    parse_expr(&chain).unwrap();
    let dots = format!("a{}", ".b".repeat(1_000));
    fails(&dots, codes::TOO_DEEP, 0);
    // A long list is wide, not deep.
    let list = format!("[{}]", vec!["1"; 5_000].join(","));
    parse_expr(&list).unwrap();
    // A source longer than a value may be.
    let long = "1".repeat(MAX_SOURCE_LEN + 1);
    fails(&long, codes::TOO_DEEP, 0);
    assert_eq!(parse_template(&long).unwrap_err()[0].code, codes::TOO_DEEP);
}

fn text(part: &str) -> TemplatePart {
    TemplatePart::Text(part.to_owned())
}

fn template(source: &str, printed: &str) -> Template {
    let parsed = parse_template(source).unwrap_or_else(|errors| panic!("`{source}`: {errors:?}"));
    assert_eq!(print_template(&parsed), printed, "printing `{source}`");
    let again = parse_template(printed).unwrap_or_else(|errors| panic!("`{printed}`: {errors:?}"));
    assert_eq!(again.without_spans(), parsed.without_spans());
    parsed
}

fn template_fails(source: &str, expected: &[(clash_expr::Code, u32)]) {
    let errors = match parse_template(source) {
        Ok(parsed) => panic!("`{source}` parsed as {parsed:?}"),
        Err(errors) => errors,
    };
    let found: Vec<(clash_expr::Code, u32)> = errors
        .iter()
        .map(|error| (error.code, error.span.start))
        .collect();
    assert_eq!(found, expected, "`{source}`: {errors:?}");
}

#[test]
fn templates_parse() {
    assert_eq!(template("", "").parts, []);
    assert_eq!(
        template("plain text", "plain text").parts,
        [text("plain text")]
    );
    let parsed = template("{the(self)} staggers back.", "{the(self)} staggers back.");
    assert_eq!(parsed.parts.len(), 2);
    assert!(matches!(
        &parsed.parts[0],
        TemplatePart::Expr { format: None, span, .. } if *span == Span::new(0, 11)
    ));
    assert_eq!(parsed.parts[1], text(" staggers back."));

    // Doubled braces are literal.
    assert_eq!(
        template("{{not an expression}}", "{{not an expression}}").parts,
        [text("{not an expression}")]
    );
    assert_eq!(template("a {{ b", "a {{ b").parts, [text("a { b")]);
    assert_eq!(template("{{{a}}}", "{{{a}}}").parts.len(), 3);

    // A number format.
    let parsed = template("{event.energy:.0} J", "{event.energy:.0} J");
    assert!(matches!(
        &parsed.parts[0],
        TemplatePart::Expr {
            format: Some(Format { precision: 0 }),
            ..
        }
    ));
    template("{ x : .2 }", "{x:.2}");
    template("{x:.17}", "{x:.17}");

    // A qualified name is not a format.
    let parsed = template("{core:f(1)}", "{core:f(1)}");
    assert!(matches!(
        &parsed.parts[0],
        TemplatePart::Expr { format: None, .. }
    ));

    // Braces inside the expression belong to it.
    template("{'}'}", "{'}'}");
    template("{({'a': 1})['a']}", "{({'a': 1}['a'])}");
    template("{len({'a': 1})}", "{len({'a': 1})}");
    template("{ a }{b}", "{a}{b}");

    // Style tags and everything else pass through as text.
    let line = "<blood>{the(self)} {verb(self, 'bleed')}.</> <<not a tag";
    let parsed = template(line, line);
    assert_eq!(parsed.parts[0], text("<blood>"));
    assert_eq!(parsed.parts[4], text(".</> <<not a tag"));

    // Spans are positions in the whole template.
    let parsed = parse_template("ab {cd + 1} ef").unwrap();
    let TemplatePart::Expr { expr, span, .. } = &parsed.parts[1] else {
        panic!("not an expression");
    };
    assert_eq!(*span, Span::new(3, 11));
    assert_eq!(expr.span, Span::new(4, 10));
}

#[test]
fn templates_fail() {
    template_fails("{", &[(codes::UNBALANCED_BRACE, 0)]);
    template_fails("{a", &[(codes::UNBALANCED_BRACE, 0)]);
    template_fails("text {a + b", &[(codes::UNBALANCED_BRACE, 5)]);
    template_fails("}", &[(codes::UNBALANCED_BRACE, 0)]);
    template_fails("a } b", &[(codes::UNBALANCED_BRACE, 2)]);
    template_fails("{}", &[(codes::EMPTY_EXPRESSION, 0)]);
    template_fails("{ }", &[(codes::EMPTY_EXPRESSION, 0)]);
    template_fails("{a:}", &[(codes::BAD_FORMAT, 3)]);
    template_fails("{a: x}", &[(codes::BAD_FORMAT, 3)]);
    template_fails("{a:.}", &[(codes::BAD_FORMAT, 3)]);
    template_fails("{a:.x}", &[(codes::BAD_FORMAT, 3)]);
    template_fails("{a:.99}", &[(codes::BAD_FORMAT, 3)]);
    template_fails("{a:2}", &[(codes::BAD_FORMAT, 3)]);
    template_fails("{a:.1", &[(codes::UNBALANCED_BRACE, 0)]);
    template_fails("{a b}", &[(codes::UNEXPECTED_TOKEN, 3)]);
    template_fails("{a +}", &[(codes::UNEXPECTED_TOKEN, 4)]);
    template_fails(
        "x {1 +} y {'a} z",
        &[
            (codes::UNEXPECTED_TOKEN, 6),
            (codes::UNTERMINATED_STRING, 11),
        ],
    );
    // Each broken expression is reported, and the good one between them
    // is not.
    template_fails(
        "{1 +} {ok} {2 *}",
        &[(codes::UNEXPECTED_TOKEN, 4), (codes::UNEXPECTED_TOKEN, 15)],
    );
}

/// The line and column, both from one, of a byte of a file.
fn line_and_column(file: &str, offset: usize) -> (usize, usize) {
    let before = &file[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.chars().rev().take_while(|c| *c != '\n').count() + 1;
    (line, column)
}

#[test]
fn a_syntax_error_inside_a_json_string_reports_the_right_column_of_the_file() {
    // Line 4 holds a condition with an escaped quote and an escaped tab
    // before the mistake, which is the second `and`.
    let file = concat!(
        "{\n",
        "  \"type\": \"eoc\",\n",
        "  \"id\": \"vault_alarm\",\n",
        "  \"if\": \"self.name != 'a\\\"b\\tc' and and here.sealed\"\n",
        "}\n",
    );
    let literal_start = file.find("\"self").unwrap();
    let literal_end = file.rfind('"').unwrap() + 1;
    let raw = &file[literal_start..literal_end];
    // What the JSON reader hands the compiler.
    let decoded = "self.name != 'a\"b\tc' and and here.sealed";

    let error = parse_expr(decoded).unwrap_err();
    assert_eq!(error.code, codes::UNEXPECTED_TOKEN);
    assert_eq!(error.span.slice(decoded), "and");
    assert_eq!(error.span.start, 25);

    let in_file = literal_start + offset_in_json_string(raw, error.span.start as usize);
    assert!(file[in_file..].starts_with("and here"));
    // The two escapes moved it two columns to the right of where the
    // decoded offset alone would put it.
    assert_eq!(line_and_column(file, in_file), (4, 37));
    let end_in_file = literal_start + offset_in_json_string(raw, error.span.end as usize);
    assert_eq!(end_in_file - in_file, 3);

    // The same for an expression inside a text template.
    let raw = r#""He says \"{count(n 'coin')}\".""#;
    let decoded = "He says \"{count(n 'coin')}\".";
    let errors = parse_template(decoded).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].span.slice(decoded), "'coin'");
    let at = offset_in_json_string(raw, errors[0].span.start as usize);
    assert!(raw[at..].starts_with("'coin'"));
}

// ---------------------------------------------------------------------
// Random trees.

const NAMES: &[&str] = &[
    "self", "target", "here", "x", "f", "e5", "ms", "min", "a_b", "it", "event", "ns",
];
const STRINGS: &[&str] = &[
    "",
    "ash",
    "it's",
    "{brace}",
    "a b",
    "é",
    "\"",
    "'",
    "''",
    "line\nbreak",
    "$x",
];
const REALS: &[f64] = &[
    0.0,
    0.5,
    1.0,
    3.25,
    1e21,
    1e-7,
    123.456,
    f64::MAX,
    f64::MIN_POSITIVE,
    5e-324,
    1e16,
    0.1,
];
const BINARY: &[BinaryOp] = &[
    BinaryOp::Or,
    BinaryOp::And,
    BinaryOp::Eq,
    BinaryOp::Ne,
    BinaryOp::Lt,
    BinaryOp::Le,
    BinaryOp::Gt,
    BinaryOp::Ge,
    BinaryOp::In,
    BinaryOp::Coalesce,
    BinaryOp::Add,
    BinaryOp::Sub,
    BinaryOp::Mul,
    BinaryOp::Div,
    BinaryOp::Rem,
    BinaryOp::Pow,
];

fn pick<'a, T>(rng: &mut Rng, items: &'a [T]) -> &'a T {
    rng.pick(items).expect("the list is not empty")
}

fn random_name(rng: &mut Rng) -> Name {
    let bare = *pick(rng, NAMES);
    assert!(!is_keyword(bare));
    if rng.below(5) == 0 {
        Name::qualified(*pick(rng, NAMES), bare)
    } else {
        Name::bare(bare)
    }
}

fn random_leaf(rng: &mut Rng) -> ExprKind {
    match rng.below(8) {
        0 => ExprKind::Null,
        1 => ExprKind::Bool(rng.below(2) == 0),
        2 | 3 => {
            let value = if rng.below(2) == 0 {
                Number::Int(match rng.below(4) {
                    0 => 0,
                    1 => i64::MAX,
                    _ => rng.range(0, 1000),
                })
            } else {
                Number::Real(Real::of(*pick(rng, REALS)))
            };
            let unit = if rng.below(3) == 0 {
                let units: Vec<Unit> = Unit::all().collect();
                Some(*pick(rng, &units))
            } else {
                None
            };
            ExprKind::Number { value, unit }
        }
        4 => ExprKind::Str((*pick(rng, STRINGS)).to_owned()),
        5 => ExprKind::Local(random_name(rng)),
        _ => ExprKind::Name(random_name(rng)),
    }
}

fn random_expr(rng: &mut Rng, depth: u32) -> Expr {
    let boxed = |rng: &mut Rng| Box::new(random_expr(rng, depth - 1));
    let several = |rng: &mut Rng| {
        (0..rng.below(4))
            .map(|_| random_expr(rng, depth - 1))
            .collect::<Vec<Expr>>()
    };
    let kind = if depth == 0 {
        random_leaf(rng)
    } else {
        match rng.below(14) {
            0 | 1 => random_leaf(rng),
            2 => ExprKind::List(several(rng)),
            3 => ExprKind::Map(
                (0..rng.below(3))
                    .map(|_| (random_expr(rng, depth - 1), random_expr(rng, depth - 1)))
                    .collect(),
            ),
            4 => ExprKind::Member {
                object: boxed(rng),
                name: random_name(rng),
            },
            5 => ExprKind::Call {
                callee: boxed(rng),
                args: several(rng),
            },
            6 => ExprKind::Index {
                object: boxed(rng),
                index: boxed(rng),
            },
            7 => ExprKind::Unary {
                op: if rng.below(2) == 0 {
                    UnaryOp::Neg
                } else {
                    UnaryOp::Not
                },
                operand: boxed(rng),
            },
            8 => ExprKind::If {
                cond: boxed(rng),
                then: boxed(rng),
                otherwise: boxed(rng),
            },
            _ => ExprKind::Binary {
                op: *pick(rng, BINARY),
                lhs: boxed(rng),
                rhs: boxed(rng),
            },
        }
    };
    Expr::new(kind, Span::default())
}

fn random_template(rng: &mut Rng) -> Template {
    const TEXTS: &[&str] = &["a", " and ", "{", "}", "{{}}", "<b>", "</>", ": ", "é\n"];
    let mut parts = Vec::new();
    let mut last_was_text = false;
    for _ in 0..rng.below(6) {
        if !last_was_text && rng.below(2) == 0 {
            parts.push(TemplatePart::Text((*pick(rng, TEXTS)).to_owned()));
            last_was_text = true;
        } else {
            let format = (rng.below(3) == 0).then(|| Format {
                precision: rng.below(18) as u8,
            });
            parts.push(TemplatePart::Expr {
                expr: random_expr(rng, 3),
                format,
                span: Span::default(),
            });
            last_was_text = false;
        }
    }
    Template { parts }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Printing a tree and parsing it again gives the same tree.
    #[test]
    fn print_then_parse_gives_the_same_tree(seed in any::<u64>(), depth in 0_u32..6) {
        let mut rng = Rng::from_seed(seed);
        let tree = random_expr(&mut rng, depth);
        let printed = print_expr(&tree);
        let parsed = parse_expr(&printed);
        prop_assert!(parsed.is_ok(), "`{}` does not parse: {:?}", printed, parsed);
        let parsed = parsed.unwrap();
        prop_assert_eq!(parsed.without_spans(), tree, "`{}`", printed);
        // And the printed form is a fixed point.
        prop_assert_eq!(print_expr(&parsed), printed);
    }

    /// The same for text templates.
    #[test]
    fn print_then_parse_gives_the_same_template(seed in any::<u64>()) {
        let mut rng = Rng::from_seed(seed);
        let tree = random_template(&mut rng);
        let printed = print_template(&tree);
        let parsed = parse_template(&printed);
        prop_assert!(parsed.is_ok(), "`{}` does not parse: {:?}", printed, parsed);
        let parsed = parsed.unwrap();
        prop_assert_eq!(parsed.without_spans(), tree, "`{}`", printed);
        prop_assert_eq!(print_template(&parsed), printed);
    }

    /// Whatever the parser is given, it answers and does not panic, and
    /// an error lies inside the source.
    #[test]
    fn the_parser_never_panics(source in "[ -~é\n]{0,40}") {
        if let Err(error) = parse_expr(&source) {
            prop_assert!(error.span.start <= error.span.end);
            prop_assert!(error.span.end as usize <= source.len());
        }
        if let Err(errors) = parse_template(&source) {
            for error in errors {
                prop_assert!(error.span.end as usize <= source.len());
            }
        }
    }

    /// The same, for sources made of the language's own tokens, which get
    /// much further into the grammar than random characters do.
    #[test]
    fn the_parser_never_panics_on_token_soup(seed in any::<u64>(), len in 0_usize..30) {
        const TOKENS: &[&str] = &[
            "a", "$x", "1", "2.5", "3ms", "'s'", "(", ")", "[", "]", "{", "}", ",", ".", ":",
            "+", "-", "*", "/", "%", "^", "==", "!=", "<", "<=", ">", ">=", "??", "if", "then",
            "else", "and", "or", "not", "in", "true", "null", " ", "ns:f",
        ];
        let mut rng = Rng::from_seed(seed);
        let source: String = (0..len).map(|_| *pick(&mut rng, TOKENS)).collect::<Vec<_>>().join(" ");
        match parse_expr(&source) {
            Ok(tree) => {
                let printed = print_expr(&tree);
                let again = parse_expr(&printed);
                prop_assert!(again.is_ok(), "`{}` printed as `{}`", source, printed);
                prop_assert_eq!(again.unwrap().without_spans(), tree.without_spans());
            }
            Err(error) => prop_assert!(error.span.end as usize <= source.len()),
        }
        let _ = parse_template(&source);
    }
}
