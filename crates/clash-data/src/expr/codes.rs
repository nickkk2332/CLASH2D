//! The diagnostic codes of the compile stage: group `06` (04 §8.2).
//!
//! The expression compiler reports with codes of its own, `X101` and so
//! on, and a range of bytes inside the expression. The loader turns each
//! into a diagnostic of the form every other stage uses: a code of this
//! table, the file, line and column, the definition and the JSON path.
//! The second and third digits follow the compiler's groups: `060` and
//! `061` syntax, `062` and `063` names, `064` types, `066` purity and
//! belief, `067` what this stage finds itself, `068` what the rule
//! runtime finds in a list of effects or a rule, `069` what goes wrong
//! while an expression runs.

use crate::diag::{Code, CodeInfo, Severity};

macro_rules! expr_codes {
    ($( $name:ident = $id:literal, $severity:ident, $stage:literal, $from:expr, $title:literal, $explanation:literal; )*) => {
        $(
            #[doc = $title]
            pub const $name: Code = Code::new($id, Severity::$severity);
        )*

        /// Every code of the compile stage, in code order, with its
        /// explanation.
        pub const CODES: &[CodeInfo] = &[
            $( CodeInfo { code: $name, stage: $stage, title: $title, explanation: $explanation }, )*
        ];

        /// Each code of this table beside the code of the expression
        /// compiler it stands for, if it stands for one.
        const FROM: &[(Option<&str>, Code)] = &[
            $( ($from, $name), )*
        ];
    };
}

expr_codes! {
    UNEXPECTED_CHARACTER = "E0601", Error, "compile", Some("X101"),
        "An expression holds a character that cannot start anything.",
        "Expressions are written in the language of 05 §4: names, numbers, texts in single quotation marks, operators and brackets. The caret is under a character that is none of those. A text is written 'like this'; a double quotation mark inside a JSON string would end the string.";
    UNTERMINATED_STRING = "E0602", Error, "compile", Some("X102"),
        "A text in an expression has no closing quotation mark.",
        "A text in an expression starts and ends with a single quotation mark: 'ash'. A quotation mark inside the text is written twice: 'it''s'. The caret is at the mark that opened the text that never closed.";
    BAD_NUMBER = "E0603", Error, "compile", Some("X103"),
        "A number in an expression is malformed or too large.",
        "A whole number must fit in 64 bits. A number with a point needs a digit on both sides of it (`0.5`, not `.5`), and an exponent needs digits (`1e6`). There are no negative literals: `-5` is the negation of `5`.";
    UNKNOWN_UNIT = "E0604", Error, "compile", Some("X104"),
        "A unit after a number in an expression is not one of the simple units.",
        "Inside an expression a number may be followed by a unit that is a scale: `us ms s min h d`, `mm cm m km`, `ml L`, `g kg t`, `J kJ MJ GJ`, `W kW MW`, `N kN`, `Pa kPa MPa GPa` and `K` (05 §4.7). Compound units such as `m/s` are written in JSON unit strings, and `C` is refused because it is an offset: `-5C` would read as minus 278.15.";
    UNEXPECTED_TOKEN = "E0605", Error, "compile", Some("X105"),
        "An expression has something where the grammar expects something else.",
        "The expression could not be read by the grammar of 05 §4.1. The caret is under the first thing that does not fit, and the message says what was expected there. Common causes: two values with no operator between them, `=` for `==`, `&&` for `and`, a bracket that was never closed.";
    UNEXPECTED_END = "E0606", Error, "compile", Some("X106"),
        "An expression ends where more is needed.",
        "The expression stops in the middle: after an operator, inside brackets, or after `if` without its `then` and `else`. The caret is at the end of what was written.";
    CHAINED_COMPARISON = "E0607", Error, "compile", Some("X107"),
        "Two comparisons are written in a row.",
        "Comparisons do not chain: `a < b < c` would compare a boolean with a number. Write `a < b and b < c`.";
    UNBALANCED_BRACE = "E0608", Error, "compile", Some("X108"),
        "A brace in a text template is not closed or not opened.",
        "In a text, anything in braces is an expression: \"{the(self)} staggers back.\" (05 §4.6). A brace that is meant literally is written twice: `{{` and `}}`. The caret is under a `{` with no `}` after it, or a `}` with no `{` before it.";
    BAD_FORMAT = "E0609", Error, "compile", Some("X109"),
        "A number format in a text template is not `.N`.",
        "An expression in a text may be followed by a colon and a format: `{event.energy:.0}`. The format is a point and the number of digits to show after it, from 0 to 17, and nothing else.";
    TOO_DEEP = "E0610", Error, "compile", Some("X110"),
        "An expression is too long or nested too deeply.",
        "So that nothing a mod writes can exhaust the compiler, brackets, `if`, `not` and unary minus nest at most 32 deep, a syntax tree is at most 100 operations deep, and a source string is at most 65,536 bytes (05 §4.1). Break the expression up with a function definition.";
    EMPTY_EXPRESSION = "E0611", Error, "compile", Some("X111"),
        "An expression has nothing in it.",
        "An expression field holds an empty string, or a text holds `{}`. Write the expression, or leave the field out if it has a default.";

    UNKNOWN_NAME = "E0621", Error, "compile", Some("X201"),
        "A name in an expression is not a subject bound where the expression appears.",
        "Each place an expression appears offers certain subjects by name: `self`, `target`, `here` and so on (05 §5.1, §7.3). The name under the caret is not one that this field offers. If a close name is bound, the diagnostic suggests it. A function is only a function where it is called: `clamp(…)`.";
    UNKNOWN_LOCAL = "E0622", Error, "compile", Some("X202"),
        "A `$name` is not a local or a parameter where the expression appears.",
        "`$name` reads a local variable or a parameter (05 §4.2). The field under the caret has no local of that name: a function sees only its own parameters, and a rule the locals defined before the expression runs.";
    UNKNOWN_PROPERTY = "E0623", Error, "compile", Some("X203"),
        "A subject has no property of that name.",
        "`subject.name` reads a property of the subject's kind (05 §5.2). The kind of the subject under the caret has no property of that name; if a close one exists the diagnostic suggests it. A variable is read as `subject.var.name`.";
    UNKNOWN_FUNCTION = "E0624", Error, "compile", Some("X204"),
        "An expression calls a function that does not exist.",
        "A bare function name is looked up among the engine's functions first, then in the mod's own namespace, then in each namespace it imports (05 §4.2). Nothing of the name under the caret was found. Functions defined by mods are `function` definitions (05 §12).";
    UNKNOWN_EVENT_FIELD = "E0625", Error, "compile", Some("X205"),
        "An expression reads an event field where there is no event or no such field.",
        "`event.name` reads a field of the event that triggered a rule (05 §4.2). Either the place this expression appears has no event, or the event has no field of that name. `event` is read one field at a time.";
    UNKNOWN_ID = "E0626", Error, "compile", Some("X206"),
        "A text used as an id names no definition of that type.",
        "A text compared with an id, or written where an id is wanted, is resolved against the definitions of that type when mods load (05 §5.2): `here.block.material == 'glass'` is checked against the materials. The text under the caret names none of them. A bare name is looked up in the mod's own namespace and then in its imports.";
    UNDECLARED_VARIABLE = "W0627", Warning, "compile", Some("X207"),
        "An expression reads a variable that no definition declares.",
        "Using an undeclared variable works, and is reported because a misspelt variable name that silently creates a new variable is the commonest mistake in rules (05 §6). Declare it with a `variable` definition. A mod with `\"strict_vars\": true` in its `mod.json` makes this an error for everything it writes.";
    NOT_CALLABLE = "E0628", Error, "compile", Some("X208"),
        "An expression calls something that is neither a function nor a property.",
        "Only a function (`clamp(x, 0, 1)`) or a property that takes arguments (`here.field('fire')`) can be called. What is under the caret is a subject, a value or something else.";
    VARIABLE_SCOPE = "E0629", Error, "compile", Some("X209"),
        "A variable is read on a kind of subject it is not declared on.",
        "A `variable` definition says what kind of thing the variable lives on: its `scope` (05 §6). The expression reads it from a subject of another kind, where it would never be found.";
    AMBIGUOUS_NAME = "W0630", Warning, "compile", Some("X210"),
        "A bare name in an expression is found in more than one namespace.",
        "A bare name is looked up in the mod's own namespace and then in each namespace it imports, and the first match wins (04 §4.3). Another namespace on the list has the same name, so a change in load order or in an import could change what is meant. Write the name with its namespace.";
    NOT_ASSIGNABLE = "E0631", Error, "compile", Some("X211"),
        "The target of an assignment cannot be assigned to.",
        "What `set` and `add` assign to is a place: a local (`$x`), a field of the event (`event.energy`), a variable (`self.var.kills`) or a settable property (`self.health`), optionally followed by keys (05 §10.3). What is under the caret is none of those.";

    OPERAND_TYPE = "E0641", Error, "compile", Some("X301"),
        "An operator is given operands of types it does not accept.",
        "Arithmetic takes numbers, `+` also joins two texts or two lists, `and`, `or` and `not` take booleans, and `<` orders two numbers or two texts (05 §4.3). The message names the types that were found. To join a text with something else, turn it into a text with `str`.";
    ARGUMENT_COUNT = "E0642", Error, "compile", Some("X302"),
        "A function or property is given the wrong number of arguments.",
        "The message shows the signature that was expected. Parameters marked with `?` may be left out, from the end.";
    ARGUMENT_TYPE = "E0643", Error, "compile", Some("X303"),
        "An argument is of the wrong type.",
        "The argument under the caret does not have the type its parameter declares. The message shows the signature. An `int` is accepted where a `real` is wanted, and not the other way round: use `floor`, `ceil` or `round`.";
    NULL_IN_ARITHMETIC = "W0644", Warning, "compile", Some("X304"),
        "A value that may be null is used where a value is needed.",
        "Things go missing: a target dies, a variable was never set (05 §4.4). The compiler can see that what is under the caret may be null, and arithmetic with null gives null and a warning every time it runs. Give a value for that case with `??`: `target.mass ?? 0`.";
    RESULT_TYPE = "E0645", Error, "compile", Some("X305"),
        "An expression's type is not the one its field wants.",
        "Each expression field declares the type of value it wants (05 §7.1): a condition wants a boolean, a duration a number. The expression under the caret gives something else. The message names both types.";
    UNRELATED_COMPARISON = "E0646", Error, "compile", Some("X306"),
        "A comparison is between types that can never be equal or ordered.",
        "`==` compares numbers with numbers, texts with texts, an id with an id of the same type or with a text, and subjects with subjects (05 §4.3). The two sides here can never be equal, so the comparison would always be false: one of them is not what was meant.";
    NOT_INDEXABLE = "E0647", Error, "compile", Some("X307"),
        "An index is applied to something that is not a list or a map.",
        "`xs[i]` reads an element of a list, counting from zero, and `m['key']` an entry of a map (05 §4.3). What is under the caret is neither.";
    NO_PROPERTIES = "E0648", Error, "compile", Some("X308"),
        "A property is read from a value that is not a subject or a map.",
        "`value.name` reads a property of a subject or an entry of a map. A number, a text, a boolean or a list has neither. If the value is an id, read the definition through the subject it came from.";
    ALWAYS_FAILS = "W0649", Warning, "compile", Some("X309"),
        "An operation on constants fails every time it runs.",
        "The compiler can see that an operation such as `1 / 0` cannot succeed. It is left as written, gives null and warns each time it runs (05 §4.5).";

    RANDOM_IN_PURE = "E0661", Error, "compile", Some("X401"),
        "A random function is used where the value must be pure.",
        "Some fields must give the same answer every time they are asked: what a screen shows, a preview, a sorting key (05 §7.3). Random functions (`rand`, `chance`, `pick`, …), a `pick` value source and any function definition that uses one are refused there.";
    NOT_BELIEF_SAFE = "E0662", Error, "compile", Some("X402"),
        "A character's decision reads something the character could not know.",
        "Goals, methods, tactics, an action's defaults and ordinary screens are compiled in the mind context (05 §7.3): they may read the character's own state, what it has perceived and whatever is registered as safe for belief. The read under the caret is ground truth about something else (01 R11). Read what the character has perceived instead (15 §7). A function definition is safe for belief only if everything it reads is.";

    BAD_TYPE_NAME = "E0671", Error, "compile", None,
        "A type name is not a type.",
        "A function's `params` and `returns` and a variable's `value` name types as the language writes them (05 §3): `bool`, `int`, `real`, `number`, `string`, `any`, a kind of subject such as `entity`, `id<material>`, `list<int>`, `map<real>`, and any of those followed by `?` for one that may be null.";
    ENDLESS_RECURSION = "E0672", Error, "compile", None,
        "A function always calls itself.",
        "Functions may call each other, and themselves, to a fixed depth (05 §12). This one calls itself, directly or through others, on every path through its body, so it could never finish. Put the call inside an `if … then … else`, or after `and`, `or` or `??`, so that there is a case in which it is not made.";
    BAD_FUNCTION = "E0673", Error, "compile", None,
        "A function definition is not well formed.",
        "A function's parameters are named like locals: letters, digits and underscores, not starting with a digit, each name once. A parameter with a `default` may be left out when the function is called, so those come last, and a default is a constant. A function defined in place inside another definition has no name to be called by.";
    BAD_VARIABLE = "E0674", Error, "compile", None,
        "A variable declaration is not well formed.",
        "A `variable` definition names the kind of thing it lives on in `scope` (`entity`, `cell`, `site`, `global`, …) and its type in `value` (05 §6). Its `default`, if it has one, is a constant of that type; without one the variable reads as null until it is set.";
    BAD_SCOPE = "E0675", Error, "compile", None,
        "A definition type declares a scope the registry cannot supply.",
        "Each expression field says what is in scope there, by the names of kinds, types and events (04 §7.3). One of those names is not registered. This is a mistake in the engine or in how the tool was started, not in the mod: the loader was given a registry without the kinds this definition type needs.";
    BAD_SOURCE = "E0676", Error, "compile", None,
        "A value source is not well formed.",
        "Where an expression string cannot say a value, an object can (05 §7.4): `{ \"expr\": \"…\" }`, `{ \"call\": \"function\", \"args\": { … } }`, or a query under `count`, `collect`, `first`, `pick`, `sum`, `min` or `max`. A `call` names a function definition and gives its arguments by parameter name.";

    UNKNOWN_EFFECT = "E0681", Error, "compile", None,
        "An effect names nothing that can be done.",
        "An effect is an object with one key, its verb (05 §10.1): one of the effects of 05 §10.2 to §10.5, a verb a subsystem registered, or a rule that takes parameters. Nothing of the name under the caret was found. A rule is looked up in the mod's own namespace and then in each namespace it imports; one with no `params` is run with `{ \"run\": \"its_id\" }`.";
    BAD_EFFECT = "E0682", Error, "compile", None,
        "An effect is not well formed.",
        "Each effect takes its arguments in a fixed form (05 §10): a verb takes them by name in an object, or its one obvious argument directly; `if` takes `then` and `else`; a loop takes `do`. The message says what is missing, unknown or misplaced, and shows what the effect takes.";
    MISPLACED_EFFECT = "E0683", Error, "compile", None,
        "An effect is used where it has no meaning.",
        "`return` gives a value to whoever called a rule, so it is written in a rule and nowhere else. `modify`, `cancel_event` and `consume` act on the event a rule is handling: `modify` and `cancel_event` only in a rule that listens for a hook, and `modify` only on a field the hook declares mutable (05 §8.2, §10.5). A `run` or `emit` whose result is wanted is written as the value of a `let`.";
    BAD_RULE = "E0684", Error, "compile", None,
        "A rule, a rule group or a reason is not well formed.",
        "A rule names the event it listens for in `on`, which must be registered; `as` and `filter` name a subject and fields of that event; `params` are named like locals, and those with a default come last; `returns` and the kinds in `subjects` are types as the language writes them (05 §9). A reason's `values` are named like locals and typed the same way (14 §15).";
    RUN_NULL_ARITHMETIC = "W0691", Warning, "run time", Some("X901"),
        "While running, arithmetic met a null.",
        "A value was needed and what arrived was null: a target that no longer exists, a variable that was never set, a property of something that is not loaded (05 §4.4). The operation gave null. Give a value for that case with `??`.";
    RUN_DIVISION_BY_ZERO = "W0692", Warning, "run time", Some("X902"),
        "While running, a number was divided by zero.",
        "Division by zero gives null and this warning (05 §4.5). Test the divisor first, or give the result a fallback with `??`.";
    RUN_DOMAIN = "W0693", Warning, "run time", Some("X903"),
        "While running, a function was given a number it has no answer for.",
        "The square root of a negative number, the logarithm of a number that is not positive and the like give null and this warning (05 §4.5). The message says which.";
    RUN_OVERFLOW = "W0694", Warning, "run time", Some("X904"),
        "While running, integer arithmetic left the range of a 64-bit integer.",
        "The result would not fit, so the operation gave null (05 §4.5). Use real numbers for quantities that large.";
    RUN_TYPE = "W0695", Warning, "run time", Some("X905"),
        "While running, a value had a type the operation does not accept.",
        "This can only happen where the type was `any` when the mod loaded: an undeclared variable, a parameter typed `any`. The operation gave null. Declare the variable or the parameter with its type and the mistake is found at load.";
    RUN_TOO_LARGE = "W0696", Warning, "run time", Some("X906"),
        "While running, a list, a map or a text grew past the size limit.",
        "One value may hold at most `rules.max_value_size` elements, or bytes of text (05 §13.2). The operation that would have passed the limit gave null.";
    RUN_NO_RANDOM = "W0697", Warning, "run time", Some("X907"),
        "While running, a random function had no stream to draw from.",
        "Random functions draw from a stream the simulation supplies. This evaluation had none, which is how the engine evaluates a field that must be pure, so the function gave null.";
    RUN_CALL_DEPTH = "W0698", Warning, "run time", Some("X908"),
        "While running, functions called each other too deeply.",
        "A function definition may call itself, directly or through others, to a fixed depth: `rules.max_call_depth`, 32 unless set (05 §12, §13.2). The call that would have gone deeper was refused and gave null. Check the case that is meant to end the recursion.";
    RUN_UNAVAILABLE = "W0699", Warning, "run time", Some("X909"),
        "While running, something was asked for that the engine does not offer yet.",
        "The value source was read and kept when the mod loaded, and what it needs is not built in this version of the engine. It gave null. The message says what is missing.";
}

/// The code of this table that stands for a code of the expression
/// compiler, such as `X206`. `None` for a code this table does not know,
/// which a test rules out.
pub fn for_expr_code(code: clash_expr::Code) -> Option<Code> {
    FROM.iter()
        .find(|(from, _)| *from == Some(code.as_str()))
        .map(|(_, to)| *to)
}

/// The explanation of a code of the compile stage, or `None` if the code
/// is not one of them. Letter case is ignored.
pub fn explain(code: &str) -> Option<&'static CodeInfo> {
    CODES
        .iter()
        .find(|info| info.code.id.eq_ignore_ascii_case(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_compiler_code_has_a_code_of_this_stage() {
        for (code, _) in clash_expr::codes::ALL {
            let mapped = for_expr_code(*code);
            assert!(mapped.is_some(), "{code} has no code in group 06");
        }
        // And nothing stands for a code the compiler does not have.
        for (from, to) in FROM {
            if let Some(from) = from {
                assert!(
                    clash_expr::codes::ALL
                        .iter()
                        .any(|(code, _)| code.as_str() == *from),
                    "{} stands for {from}, which does not exist",
                    to.id
                );
            }
        }
    }

    #[test]
    fn every_code_has_an_explanation_and_is_in_group_06() {
        let mut seen = std::collections::BTreeSet::new();
        for info in CODES {
            let id = info.code.id;
            assert!(seen.insert(id), "code {id} is declared twice");
            assert_eq!(id.len(), 5, "code {id} is a letter and four digits");
            assert_eq!(&id[1..3], "06", "code {id} is in the expression group");
            let letter = match info.code.severity {
                Severity::Error => 'E',
                Severity::Warning => 'W',
                Severity::Note => 'N',
            };
            assert!(id.starts_with(letter), "code {id} starts with its letter");
            assert!(info.title.len() > 10, "code {id} needs a title");
            assert!(info.explanation.len() > 60, "code {id} needs explaining");
            assert_eq!(explain(id).map(|info| info.code.id), Some(id));
            assert!(
                crate::diag::CODES.iter().all(|other| other.code.id != id),
                "code {id} is also a code of the loader"
            );
        }
        let numbers: Vec<&str> = CODES.iter().map(|info| &info.code.id[1..]).collect();
        let mut sorted = numbers.clone();
        sorted.sort_unstable();
        assert_eq!(numbers, sorted);
        assert_eq!(
            explain("w0627").map(|info| info.code),
            Some(UNDECLARED_VARIABLE)
        );
        assert_eq!(
            crate::diag::explain("E0626").map(|info| info.code),
            Some(UNKNOWN_ID)
        );
    }
}
