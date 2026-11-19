//! The built-in functions of 05 §4.7 that belong to no subsystem: maths,
//! random, lists, maps and text, and types.
//!
//! Each has a row in [`TABLE`] giving its signature and documentation,
//! from which it is registered like any other function. The text helpers
//! for a reader (05 §4.6) and `exists` are registered here too, with
//! their signatures, and answered by the host: what a reader calls
//! someone is not something this crate can know.

use std::sync::Arc;

use clash_core::Real;

use crate::host::Host;
use crate::ops::{self, Limits, Num, Res};
use crate::registry::{FunctionSpec, Registry};
use crate::types::Type;
use crate::value::{Map, Value};
use crate::warn::Fault;

/// A function this crate computes itself.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Builtin {
    Min,
    Max,
    Clamp,
    Abs,
    Sign,
    Floor,
    Ceil,
    Round,
    Sqrt,
    Exp,
    Ln,
    Log10,
    Sin,
    Cos,
    Atan2,
    Lerp,
    Div,
    Rand,
    RandInt,
    Chance,
    Pick,
    Len,
    Contains,
    First,
    Last,
    Sum,
    Str,
    Keys,
    Values,
    Has,
    Without,
    WithoutKey,
    Cap,
    IsNull,
}

/// The heading maths functions are listed under.
pub(crate) const GROUP_MATHS: &str = "Maths";
/// The heading random functions are listed under.
pub(crate) const GROUP_RANDOM: &str = "Random";
/// The heading list, map and text functions are listed under.
pub(crate) const GROUP_LISTS: &str = "Lists, maps and text";
/// The heading type functions are listed under.
pub(crate) const GROUP_TYPES: &str = "Types";
/// The heading the reader's text helpers are listed under.
pub(crate) const GROUP_READER: &str = "Text for a reader";

fn any_list() -> Type {
    Type::list(Type::Any)
}

fn any_map() -> Type {
    Type::map(Type::Any)
}

/// The built-in functions with their signatures.
fn table() -> Vec<(Builtin, FunctionSpec)> {
    use Builtin as B;
    let f = FunctionSpec::new;
    let n = || Type::Number;
    let r = || Type::Real;
    let i = || Type::Int;
    vec![
        (
            B::Min,
            f("min", n())
                .group(GROUP_MATHS)
                .param("a", n(), "A number.")
                .param("b", n(), "Another number; more may follow.")
                .variadic()
                .doc("The smallest of its arguments. An int if all are ints, otherwise a real."),
        ),
        (
            B::Max,
            f("max", n())
                .group(GROUP_MATHS)
                .param("a", n(), "A number.")
                .param("b", n(), "Another number; more may follow.")
                .variadic()
                .doc("The largest of its arguments. An int if all are ints, otherwise a real."),
        ),
        (
            B::Clamp,
            f("clamp", n())
                .group(GROUP_MATHS)
                .param("x", n(), "The number to limit.")
                .param("low", n(), "The least it may be.")
                .param("high", n(), "The most it may be.")
                .doc(
                    "`x` limited to the range from `low` to `high`. If `low` is greater than \
                     `high` the result is `high`. An int if all three are ints.",
                ),
        ),
        (
            B::Abs,
            f("abs", n())
                .group(GROUP_MATHS)
                .param("x", n(), "A number.")
                .doc("The magnitude of `x`, of the same type as `x`."),
        ),
        (
            B::Sign,
            f("sign", i())
                .group(GROUP_MATHS)
                .param("x", n(), "A number.")
                .doc("`1` for a positive number, `-1` for a negative one, `0` for zero."),
        ),
        (
            B::Floor,
            f("floor", i())
                .group(GROUP_MATHS)
                .param("x", n(), "A number.")
                .doc("The largest int that is not greater than `x`."),
        ),
        (
            B::Ceil,
            f("ceil", i())
                .group(GROUP_MATHS)
                .param("x", n(), "A number.")
                .doc("The smallest int that is not less than `x`."),
        ),
        (
            B::Round,
            f("round", i())
                .group(GROUP_MATHS)
                .param("x", n(), "A number.")
                .doc("The nearest int to `x`. Halves round away from zero."),
        ),
        (
            B::Sqrt,
            f("sqrt", r())
                .group(GROUP_MATHS)
                .param("x", r(), "A number that is not negative.")
                .doc("The square root. Null, with a warning, for a negative number."),
        ),
        (
            B::Exp,
            f("exp", r())
                .group(GROUP_MATHS)
                .param("x", r(), "The power.")
                .doc("`e` raised to the power `x`."),
        ),
        (
            B::Ln,
            f("ln", r())
                .group(GROUP_MATHS)
                .param("x", r(), "A positive number.")
                .doc("The natural logarithm. Null, with a warning, for zero or less."),
        ),
        (
            B::Log10,
            f("log10", r())
                .group(GROUP_MATHS)
                .param("x", r(), "A positive number.")
                .doc("The base-10 logarithm. Null, with a warning, for zero or less."),
        ),
        (
            B::Sin,
            f("sin", r())
                .group(GROUP_MATHS)
                .param("x", r(), "An angle in radians.")
                .doc("The sine of an angle."),
        ),
        (
            B::Cos,
            f("cos", r())
                .group(GROUP_MATHS)
                .param("x", r(), "An angle in radians.")
                .doc("The cosine of an angle."),
        ),
        (
            B::Atan2,
            f("atan2", r())
                .group(GROUP_MATHS)
                .param("y", r(), "The point's y.")
                .param("x", r(), "The point's x.")
                .doc(
                    "The angle in radians, from -pi to pi, of the point (x, y) measured from \
                     the positive x axis.",
                ),
        ),
        (
            B::Lerp,
            f("lerp", r())
                .group(GROUP_MATHS)
                .param("a", r(), "The value at `t` = 0.")
                .param("b", r(), "The value at `t` = 1.")
                .param("t", r(), "How far from `a` to `b`. Not limited to 0..1.")
                .doc("`a + (b - a) * t`: the value `t` of the way from `a` to `b`."),
        ),
        (
            B::Div,
            f("div", i())
                .group(GROUP_MATHS)
                .param("a", i(), "The dividend.")
                .param("b", i(), "The divisor.")
                .doc(
                    "Integer division, rounding toward minus infinity, so that \
                     `div(a, b) * b + a % b == a`. Null, with a warning, when `b` is zero.",
                ),
        ),
        (
            B::Rand,
            f("rand", r())
                .group(GROUP_RANDOM)
                .random()
                .param("a", r(), "The least the result may be.")
                .param("b", r(), "The result is less than this.")
                .doc("A random real from `a` up to but not including `b`."),
        ),
        (
            B::RandInt,
            f("rand_int", i())
                .group(GROUP_RANDOM)
                .random()
                .param("a", i(), "The least the result may be.")
                .param("b", i(), "The most the result may be.")
                .doc("A random int from `a` to `b`, both included. `a` if `b` is less than `a`."),
        ),
        (
            B::Chance,
            f("chance", Type::Bool)
                .group(GROUP_RANDOM)
                .random()
                .param("p", r(), "A probability from 0 to 1.")
                .doc("True with probability `p`. Zero or less is never; one or more is always."),
        ),
        (
            B::Pick,
            f("pick", Type::Any)
                .group(GROUP_RANDOM)
                .random()
                .param("list", any_list(), "The list to choose from.")
                .doc("One element of a list, each equally likely. Null for an empty list."),
        ),
        (
            B::Len,
            f("len", i())
                .group(GROUP_LISTS)
                .param("x", Type::Any, "A list, a map or a text.")
                .doc(
                    "The number of elements of a list, entries of a map or characters of a \
                     text.",
                ),
        ),
        (
            B::Contains,
            f("contains", Type::Bool)
                .group(GROUP_LISTS)
                .param("xs", Type::Any, "A list or a text.")
                .param(
                    "x",
                    Type::Any,
                    "An element to find in a list, or a text in a text.",
                )
                .doc("Whether a list has an element equal to `x`, or a text contains `x`."),
        ),
        (
            B::First,
            f("first", Type::Any)
                .group(GROUP_LISTS)
                .param("xs", any_list(), "A list.")
                .doc("The first element of a list. Null for an empty list."),
        ),
        (
            B::Last,
            f("last", Type::Any)
                .group(GROUP_LISTS)
                .param("xs", any_list(), "A list.")
                .doc("The last element of a list. Null for an empty list."),
        ),
        (
            B::Sum,
            f("sum", n())
                .group(GROUP_LISTS)
                .param("xs", Type::list(n()), "A list of numbers.")
                .doc(
                    "The total of a list of numbers. An int if all are ints. Zero for an empty \
                     list.",
                ),
        ),
        (
            B::Str,
            f("str", Type::Str)
                .group(GROUP_LISTS)
                .param("x", Type::Any, "Any value.")
                .doc(
                    "The value as text: a number as written, an id as its qualified id, \
                     `null` for null.",
                ),
        ),
        (
            B::Keys,
            f("keys", Type::list(Type::Str))
                .group(GROUP_LISTS)
                .param("m", any_map(), "A map.")
                .doc("The keys of a map, in order of key."),
        ),
        (
            B::Values,
            f("values", any_list())
                .group(GROUP_LISTS)
                .param("m", any_map(), "A map.")
                .doc("The values of a map, in order of key."),
        ),
        (
            B::Has,
            f("has", Type::Bool)
                .group(GROUP_LISTS)
                .param("m", any_map(), "A map.")
                .param("key", Type::Any, "A text or an id.")
                .doc("Whether a map has an entry for a key."),
        ),
        (
            B::Without,
            f("without", any_list())
                .group(GROUP_LISTS)
                .param("xs", any_list(), "A list.")
                .param("x", Type::Any, "The value to leave out.")
                .doc("A copy of a list without the elements equal to `x`."),
        ),
        (
            B::WithoutKey,
            f("without_key", any_map())
                .group(GROUP_LISTS)
                .param("m", any_map(), "A map.")
                .param("key", Type::Any, "A text or an id.")
                .doc("A copy of a map without the entry for a key."),
        ),
        (
            B::Cap,
            f("cap", Type::Str)
                .group(GROUP_LISTS)
                .param("text", Type::Str, "A text.")
                .doc("The text with a capital first letter."),
        ),
        (
            B::IsNull,
            f("is_null", Type::Bool)
                .group(GROUP_TYPES)
                .param("x", Type::Any, "Any value.")
                .doc("Whether a value is null."),
        ),
    ]
}

/// The functions whose signatures are fixed here and whose answers come
/// from the host.
fn host_table() -> Vec<FunctionSpec> {
    let f = |name: &str| FunctionSpec::new(name, Type::Str).group(GROUP_READER);
    let x = "A subject, or anything a reader can be shown.";
    vec![
        f("name").param("x", Type::Any, x).doc(
            "What the reader calls `x`: \"you\" for the reader, a known name, or a \
             description of what the reader made out.",
        ),
        f("the")
            .param("x", Type::Any, x)
            .doc("What the reader calls `x`, with a definite article where one belongs."),
        f("a")
            .param("x", Type::Any, x)
            .doc("What the reader calls `x`, with an indefinite article where one belongs."),
        f("they")
            .param("x", Type::Any, x)
            .doc("The subject pronoun for `x`; \"you\" for the reader."),
        f("them")
            .param("x", Type::Any, x)
            .doc("The object pronoun for `x`; \"you\" for the reader."),
        f("their")
            .param("x", Type::Any, x)
            .doc("The possessive for `x`; \"your\" for the reader."),
        f("themself")
            .param("x", Type::Any, x)
            .doc("The reflexive pronoun for `x`; \"yourself\" for the reader."),
        f("verb")
            .param("x", Type::Any, x)
            .param("word", Type::Str, "The verb in its plain form: 'hit'.")
            .doc("The verb agreeing with `x`: \"you hit\", \"the guard hits\"."),
        f("count")
            .param("n", Type::Number, "How many.")
            .param("word", Type::Str, "The noun in the singular.")
            .doc("A number with its noun: \"3 arrows\", \"1 arrow\"."),
        f("plural")
            .param("word", Type::Str, "The noun in the singular.")
            .doc("The plural of a noun."),
        f("list")
            .param("xs", any_list(), "The things to list.")
            .doc("A list written out: \"a knife, a rope and two coins\"."),
        FunctionSpec::new("exists", Type::Bool)
            .group(GROUP_TYPES)
            .belief_safe(false)
            .param("x", Type::Any, "A reference to a subject.")
            .doc("Whether what a reference points at still exists."),
    ]
}

/// Registers every built-in. The names cannot collide in a registry that
/// has not registered them already; if one does, the first registration
/// stands.
pub(crate) fn register(registry: &mut Registry) {
    for (builtin, spec) in table() {
        let _ = registry.add_function_impl(spec, Some(builtin));
    }
    for spec in host_table() {
        let _ = registry.add_function(spec);
    }
}

/// What is wrong with the argument types of a built-in whose parameters
/// are written as `any` because they take more than one shape.
pub(crate) fn check_args(builtin: Builtin, args: &[Type]) -> Option<&'static str> {
    let shape = |index: usize| args.get(index).map(Type::non_null);
    match builtin {
        Builtin::Len => match shape(0) {
            Some(Type::List(_) | Type::Map(_) | Type::Str | Type::Any | Type::Null) | None => None,
            Some(_) => Some("`len` takes a list, a map or a text"),
        },
        Builtin::Contains => match (shape(0), shape(1)) {
            (Some(Type::List(_) | Type::Any | Type::Null), _) | (None, _) => None,
            (Some(Type::Str), Some(Type::Str | Type::Any | Type::Null) | None) => None,
            (Some(Type::Str), Some(_)) => Some("a text can only contain a text"),
            (Some(_), _) => Some("`contains` takes a list or a text"),
        },
        Builtin::Has | Builtin::WithoutKey => match shape(1) {
            Some(Type::Str | Type::Id(_) | Type::Any | Type::Null) | None => None,
            Some(_) => Some("a map's key is a text or an id"),
        },
        _ => None,
    }
}

/// The type of a call of a built-in, given the types of its arguments.
/// `declared` is the result type in its signature, which stands for
/// those whose result does not depend on what they are given.
pub(crate) fn result_type(builtin: Builtin, args: &[Type], declared: &Type) -> Type {
    let element = |index: usize| {
        args.get(index)
            .and_then(Type::element)
            .cloned()
            .unwrap_or(Type::Any)
    };
    let own = |index: usize| {
        args.get(index)
            .map_or_else(|| declared.clone(), |ty| ty.non_null().clone())
    };
    match builtin {
        Builtin::Min | Builtin::Max | Builtin::Clamp | Builtin::Abs => Type::numeric(args),
        Builtin::Sum => Type::numeric([&element(0)]),
        Builtin::Pick | Builtin::First | Builtin::Last => element(0).nullable(),
        Builtin::Values => Type::list(element(0)),
        Builtin::Without | Builtin::WithoutKey => match own(0) {
            Type::Any | Type::Null => declared.clone(),
            ty => ty,
        },
        _ => declared.clone(),
    }
}

/// Whether every value is an int, or the first fault if one is not a
/// number at all.
fn all_ints(values: &[Value]) -> Result<bool, Fault> {
    let mut whole = true;
    for value in values {
        if let Num::Real(_) = ops::num(value)? {
            whole = false;
        }
    }
    Ok(whole)
}

/// The smallest or the largest of some numbers: an int if all are ints,
/// otherwise a real.
fn extreme(values: &[Value], smallest: bool) -> Res {
    let whole = all_ints(values)?;
    let mut best: Option<Num> = None;
    for value in values {
        let value = ops::num(value)?;
        best = Some(match best {
            None => value,
            Some(best) => {
                let better = match (best, value) {
                    (Num::Int(best), Num::Int(value)) => {
                        if smallest {
                            value < best
                        } else {
                            value > best
                        }
                    }
                    (best, value) => {
                        if smallest {
                            value.real() < best.real()
                        } else {
                            value.real() > best.real()
                        }
                    }
                };
                if better { value } else { best }
            }
        });
    }
    match best {
        Some(Num::Int(int)) if whole => Ok(Value::Int(int)),
        Some(number) => Ok(Value::Real(number.real())),
        None => Err(Fault::Null),
    }
}

fn arg(args: &[Value], index: usize) -> &Value {
    args.get(index).unwrap_or(&Value::Null)
}

fn list(value: &Value) -> Result<&[Value], Fault> {
    match value {
        Value::List(items) => Ok(items),
        Value::Null => Err(Fault::Null),
        other => Err(ops::wrong(ops::WANT_LIST, other)),
    }
}

fn map(value: &Value) -> Result<&Map, Fault> {
    match value {
        Value::Map(entries) => Ok(entries),
        Value::Null => Err(Fault::Null),
        other => Err(ops::wrong(ops::WANT_MAP, other)),
    }
}

fn text(value: &Value) -> Result<&str, Fault> {
    match value {
        Value::Str(text) => Ok(text),
        Value::Null => Err(Fault::Null),
        other => Err(ops::wrong(ops::WANT_TEXT, other)),
    }
}

/// A real rounded to a whole number, as an int.
fn whole(value: &Value, round: fn(Real) -> Real) -> Res {
    match ops::num(value)? {
        Num::Int(int) => Ok(Value::Int(int)),
        Num::Real(real) if real.is_finite() => Ok(Value::Int(round(real).to_i64())),
        Num::Real(_) => Err(Fault::Domain("an infinite number has no whole part")),
    }
}

/// Computes a built-in. The arguments have been counted by the checker;
/// one that is missing all the same reads as null.
pub(crate) fn call<H: Host + ?Sized>(
    builtin: Builtin,
    args: &[Value],
    host: &mut H,
    limits: &Limits,
) -> Res {
    let real = |index: usize| ops::real(arg(args, index));
    match builtin {
        Builtin::Min => extreme(args, true),
        Builtin::Max => extreme(args, false),
        Builtin::Clamp => {
            let numbers = (
                ops::num(arg(args, 0))?,
                ops::num(arg(args, 1))?,
                ops::num(arg(args, 2))?,
            );
            if let (Num::Int(x), Num::Int(low), Num::Int(high)) = numbers {
                return Ok(Value::Int(x.max(low).min(high)));
            }
            let (x, low, high) = numbers;
            Ok(Value::Real(x.real().clamp(low.real(), high.real())))
        }
        Builtin::Abs => match ops::num(arg(args, 0))? {
            Num::Int(int) => int.checked_abs().map(Value::Int).ok_or(Fault::Overflow),
            Num::Real(real) => Ok(Value::Real(real.abs())),
        },
        Builtin::Sign => Ok(Value::Int(match ops::num(arg(args, 0))? {
            Num::Int(int) => int.signum(),
            Num::Real(real) => real.signum().to_i64(),
        })),
        Builtin::Floor => whole(arg(args, 0), Real::floor),
        Builtin::Ceil => whole(arg(args, 0), Real::ceil),
        Builtin::Round => whole(arg(args, 0), Real::round),
        Builtin::Sqrt => real(0)?
            .try_sqrt()
            .map(Value::Real)
            .map_err(|_| Fault::Domain("the square root of a negative number")),
        Builtin::Exp => Ok(Value::Real(real(0)?.exp())),
        Builtin::Ln => real(0)?
            .try_ln()
            .map(Value::Real)
            .map_err(|_| Fault::Domain("the logarithm of a number that is not positive")),
        Builtin::Log10 => real(0)?
            .try_log10()
            .map(Value::Real)
            .map_err(|_| Fault::Domain("the logarithm of a number that is not positive")),
        Builtin::Sin => Ok(Value::Real(real(0)?.sin())),
        Builtin::Cos => Ok(Value::Real(real(0)?.cos())),
        Builtin::Atan2 => Ok(Value::Real(real(0)?.atan2(real(1)?))),
        Builtin::Lerp => {
            let (a, b, t) = (real(0)?, real(1)?, real(2)?);
            Real::new(a.get() + (b.get() - a.get()) * t.get())
                .map(Value::Real)
                .map_err(|_| Fault::Domain("the result is not defined"))
        }
        Builtin::Div => {
            ops::floor_div(ops::int(arg(args, 0))?, ops::int(arg(args, 1))?).map(Value::Int)
        }
        Builtin::Rand => {
            let (low, high) = (real(0)?, real(1)?);
            let unit = host.rng().ok_or(Fault::NoRandom)?.unit();
            Real::new(low.get() + (high.get() - low.get()) * unit.get())
                .map(Value::Real)
                .map_err(|_| Fault::Domain("the result is not defined"))
        }
        Builtin::RandInt => {
            let (low, high) = (ops::int(arg(args, 0))?, ops::int(arg(args, 1))?);
            Ok(Value::Int(
                host.rng().ok_or(Fault::NoRandom)?.range(low, high),
            ))
        }
        Builtin::Chance => {
            let probability = real(0)?;
            Ok(Value::Bool(
                host.rng().ok_or(Fault::NoRandom)?.chance(probability),
            ))
        }
        Builtin::Pick => {
            let items = list(arg(args, 0))?;
            if items.is_empty() {
                return Ok(Value::Null);
            }
            let rng = host.rng().ok_or(Fault::NoRandom)?;
            Ok(rng.pick(items).cloned().unwrap_or(Value::Null))
        }
        Builtin::Len => {
            let len = match arg(args, 0) {
                Value::List(items) => items.len(),
                Value::Map(entries) => entries.len(),
                Value::Str(text) => text.chars().count(),
                Value::Null => return Err(Fault::Null),
                other => return Err(ops::wrong("a list, a map or a text", other)),
            };
            Ok(Value::Int(i64::try_from(len).unwrap_or(i64::MAX)))
        }
        Builtin::Contains => match (arg(args, 0), arg(args, 1)) {
            (Value::List(items), wanted) => Ok(Value::Bool(
                items.iter().any(|item| ops::equal(item, wanted, &*host)),
            )),
            (Value::Str(haystack), part) => Ok(Value::Bool(haystack.contains(text(part)?))),
            (Value::Null, _) => Err(Fault::Null),
            (other, _) => Err(ops::wrong("a list or a text", other)),
        },
        Builtin::First => Ok(list(arg(args, 0))?.first().cloned().unwrap_or(Value::Null)),
        Builtin::Last => Ok(list(arg(args, 0))?.last().cloned().unwrap_or(Value::Null)),
        Builtin::Sum => {
            let items = list(arg(args, 0))?;
            if all_ints(items)? {
                let mut total = 0_i64;
                for item in items {
                    total = total.checked_add(ops::int(item)?).ok_or(Fault::Overflow)?;
                }
                return Ok(Value::Int(total));
            }
            let mut total = 0.0_f64;
            for item in items {
                total += ops::real(item)?.get();
            }
            Real::new(total)
                .map(Value::Real)
                .map_err(|_| Fault::Domain("the result is not defined"))
        }
        Builtin::Str => {
            let mut out = String::new();
            ops::write_text(&mut out, arg(args, 0), &*host);
            if out.len() > limits.max_value_size {
                return Err(Fault::TooLarge);
            }
            Ok(Value::Str(Arc::from(out)))
        }
        Builtin::Keys => Ok(Value::List(
            map(arg(args, 0))?
                .keys()
                .map(|key| Value::Str(Arc::clone(key)))
                .collect(),
        )),
        Builtin::Values => Ok(Value::List(map(arg(args, 0))?.values().cloned().collect())),
        Builtin::Has => {
            let entries = map(arg(args, 0))?;
            let key = ops::key_of(arg(args, 1), &*host)?;
            Ok(Value::Bool(
                key.is_some_and(|key| entries.contains_key(&key)),
            ))
        }
        Builtin::Without => {
            let unwanted = arg(args, 1);
            Ok(Value::List(
                list(arg(args, 0))?
                    .iter()
                    .filter(|item| !ops::equal(item, unwanted, &*host))
                    .cloned()
                    .collect(),
            ))
        }
        Builtin::WithoutKey => {
            let entries = map(arg(args, 0))?;
            let mut copy = entries.clone();
            if let Some(key) = ops::key_of(arg(args, 1), &*host)? {
                copy.remove(&key);
            }
            Ok(Value::Map(Arc::new(copy)))
        }
        Builtin::Cap => {
            let text = text(arg(args, 0))?;
            let mut characters = text.chars();
            let capped: String = match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            };
            Ok(Value::Str(Arc::from(capped)))
        }
        Builtin::IsNull => Ok(Value::Bool(arg(args, 0).is_null())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_is_registered_with_documentation() {
        let registry = Registry::with_builtins();
        let rows = table();
        for (builtin, spec) in &rows {
            let id = registry
                .function_id(&spec.name)
                .unwrap_or_else(|| panic!("`{}` is not registered", spec.name));
            let function = registry.function(id).unwrap();
            assert_eq!(function.builtin, Some(*builtin));
            assert!(function.is_builtin());
            assert!(!spec.doc.is_empty(), "{}", spec.name);
            assert!(!spec.group.is_empty(), "{}", spec.name);
            assert!(spec.belief_safe, "{}", spec.name);
            for param in &spec.params {
                assert!(!param.doc.is_empty(), "{}({})", spec.name, param.name);
            }
        }
        for spec in host_table() {
            let id = registry.function_id(&spec.name).unwrap();
            assert!(!registry.function(id).unwrap().is_builtin());
            assert!(!spec.doc.is_empty(), "{}", spec.name);
            assert!(!spec.group.is_empty(), "{}", spec.name);
        }
        assert_eq!(
            registry.functions().count(),
            rows.len() + host_table().len()
        );
        // Registering twice changes nothing.
        let mut again = Registry::with_builtins();
        register(&mut again);
        assert_eq!(again.functions().count(), registry.functions().count());
    }

    #[test]
    fn only_the_random_functions_are_random() {
        let registry = Registry::with_builtins();
        let random: Vec<&str> = registry
            .functions()
            .filter(|function| function.spec.random)
            .map(|function| function.spec.name.as_str())
            .collect();
        assert_eq!(random, ["chance", "pick", "rand", "rand_int"]);
    }

    #[test]
    fn every_function_of_the_specification_that_needs_no_host_is_here() {
        let registry = Registry::with_builtins();
        for name in [
            "min",
            "max",
            "clamp",
            "abs",
            "sign",
            "floor",
            "ceil",
            "round",
            "sqrt",
            "exp",
            "ln",
            "log10",
            "sin",
            "cos",
            "atan2",
            "lerp",
            "div",
            "rand",
            "rand_int",
            "chance",
            "pick",
            "len",
            "contains",
            "first",
            "last",
            "sum",
            "str",
            "keys",
            "values",
            "has",
            "without",
            "without_key",
            "cap",
            "is_null",
            "exists",
            "name",
            "the",
            "a",
            "they",
            "them",
            "their",
            "themself",
            "verb",
            "count",
            "plural",
            "list",
        ] {
            assert!(registry.function_id(name).is_some(), "{name}");
        }
    }

    #[test]
    fn result_types_follow_the_arguments() {
        let int = Type::Int;
        let real = Type::Real;
        assert_eq!(
            result_type(Builtin::Min, &[int.clone(), int.clone()], &Type::Number),
            Type::Int
        );
        assert_eq!(
            result_type(Builtin::Max, &[int.clone(), real.clone()], &Type::Number),
            Type::Real
        );
        assert_eq!(
            result_type(Builtin::Abs, &[Type::Any], &Type::Number),
            Type::Number
        );
        assert_eq!(
            result_type(Builtin::Sum, &[Type::list(Type::Int)], &Type::Number),
            Type::Int
        );
        assert_eq!(
            result_type(Builtin::First, &[Type::list(Type::Str)], &Type::Any),
            Type::Str.nullable()
        );
        assert_eq!(
            result_type(Builtin::Pick, &[Type::Any], &Type::Any),
            Type::Any
        );
        assert_eq!(
            result_type(Builtin::Values, &[Type::map(Type::Real)], &any_list()),
            Type::list(Type::Real)
        );
        assert_eq!(
            result_type(Builtin::Without, &[Type::list(Type::Int), int], &any_list()),
            Type::list(Type::Int)
        );
        assert_eq!(
            result_type(Builtin::Without, &[Type::Any, real], &any_list()),
            any_list()
        );
        assert_eq!(
            result_type(Builtin::Sqrt, &[Type::Int], &Type::Real),
            Type::Real
        );
    }

    #[test]
    fn shapes_are_checked_for_the_functions_that_take_several() {
        assert_eq!(check_args(Builtin::Len, &[Type::list(Type::Int)]), None);
        assert_eq!(check_args(Builtin::Len, &[Type::Str.nullable()]), None);
        assert_eq!(check_args(Builtin::Len, &[Type::Any]), None);
        assert!(check_args(Builtin::Len, &[Type::Int]).is_some());
        assert_eq!(
            check_args(Builtin::Contains, &[Type::list(Type::Int), Type::Int]),
            None
        );
        assert_eq!(check_args(Builtin::Contains, &[Type::Str, Type::Str]), None);
        assert!(check_args(Builtin::Contains, &[Type::Str, Type::Int]).is_some());
        assert!(check_args(Builtin::Contains, &[Type::Int, Type::Int]).is_some());
        assert_eq!(check_args(Builtin::Has, &[any_map(), Type::Str]), None);
        assert!(check_args(Builtin::Has, &[any_map(), Type::Int]).is_some());
        assert_eq!(check_args(Builtin::Sqrt, &[Type::Str]), None);
    }
}
