//! What the operators mean (05 §4.3 to §4.5).
//!
//! Every operation on values is defined once, here, and used by the
//! constant folder, the stack machine and the tree-walking evaluator
//! alike, so the three cannot disagree about what `1 / 0` is.
//!
//! An operation that cannot give a value gives a [`Fault`] instead. The
//! caller turns that into null (or false, for a comparison) and a
//! run-time warning. Nothing here panics, whatever the values.

use core::cmp::Ordering;
use core::fmt::Write as _;
use std::sync::Arc;

use clash_core::Real;

use crate::ast::Format;
use crate::hir::{ArithOp, CmpOp};
use crate::host::Host;
use crate::value::{Map, Value};
use crate::warn::Fault;

/// The limits a machine keeps values within (05 §13.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// The most elements a list or a map, or bytes a text, may have. An
    /// operation that would make a larger one gives null, with a
    /// warning. The setting `rules.max_value_size`.
    pub max_value_size: usize,
}

/// The default of `rules.max_value_size`.
pub const DEFAULT_MAX_VALUE_SIZE: usize = 65_536;

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_value_size: DEFAULT_MAX_VALUE_SIZE,
        }
    }
}

pub(crate) type Res = Result<Value, Fault>;

/// A number, before it is known whether integer or real arithmetic
/// applies.
#[derive(Clone, Copy)]
pub(crate) enum Num {
    Int(i64),
    Real(Real),
}

impl Num {
    pub(crate) const fn real(self) -> Real {
        match self {
            Num::Int(int) => Real::from_i64(int),
            Num::Real(real) => real,
        }
    }
}

pub(crate) const WANT_NUMBER: &str = "a number";
pub(crate) const WANT_INT: &str = "an int";
pub(crate) const WANT_BOOL: &str = "a bool";
pub(crate) const WANT_TEXT: &str = "a text";
pub(crate) const WANT_LIST: &str = "a list";
pub(crate) const WANT_MAP: &str = "a map";
pub(crate) const WANT_SUBJECT: &str = "a subject";
const WANT_KEY: &str = "a text or an id as a key";
const WANT_COLLECTION: &str = "a list or a map";
const WANT_ADDABLE: &str = "two numbers, two texts or two lists";
const WANT_ORDERED: &str = "two numbers or two texts";
const UNDEFINED: &str = "the result is not defined";

pub(crate) const fn wrong(wanted: &'static str, found: &Value) -> Fault {
    Fault::Type {
        wanted,
        found: found.type_name(),
    }
}

/// The value as a number. Null is its own fault, so that the warning
/// says what happened.
pub(crate) const fn num(value: &Value) -> Result<Num, Fault> {
    match value {
        Value::Int(int) => Ok(Num::Int(*int)),
        Value::Real(real) => Ok(Num::Real(*real)),
        Value::Null => Err(Fault::Null),
        other => Err(wrong(WANT_NUMBER, other)),
    }
}

/// The value as a real; an int is converted.
pub(crate) fn real(value: &Value) -> Result<Real, Fault> {
    num(value).map(Num::real)
}

/// The value as an int. A real is not converted.
pub(crate) const fn int(value: &Value) -> Result<i64, Fault> {
    match value {
        Value::Int(int) => Ok(*int),
        Value::Null => Err(Fault::Null),
        other => Err(wrong(WANT_INT, other)),
    }
}

/// A real result, refusing what IEEE arithmetic calls not a number.
fn defined(value: f64) -> Res {
    Real::new(value)
        .map(Value::Real)
        .map_err(|_| Fault::Domain(UNDEFINED))
}

/// Whether a value counts as true where a bool is wanted. Null counts as
/// false (05 §4.4). Anything else that is not a bool is a fault, which
/// the caller reports and treats as false.
pub(crate) const fn truthy(value: &Value) -> Result<bool, Fault> {
    match value {
        Value::Bool(value) => Ok(*value),
        Value::Null => Ok(false),
        other => Err(wrong(WANT_BOOL, other)),
    }
}

/// `-value`.
pub(crate) fn neg(value: &Value) -> Res {
    match num(value)? {
        Num::Int(int) => int.checked_neg().map(Value::Int).ok_or(Fault::Overflow),
        Num::Real(real) => Ok(Value::Real(-real)),
    }
}

/// Integer division rounding toward minus infinity.
pub(crate) fn floor_div(a: i64, b: i64) -> Result<i64, Fault> {
    if b == 0 {
        return Err(Fault::DivisionByZero);
    }
    let quotient = a.checked_div(b).ok_or(Fault::Overflow)?;
    let remainder = a.wrapping_rem(b);
    if remainder != 0 && ((remainder < 0) != (b < 0)) {
        Ok(quotient - 1)
    } else {
        Ok(quotient)
    }
}

/// The remainder that goes with [`floor_div`]: it has the sign of the
/// divisor.
fn floor_rem(a: i64, b: i64) -> Result<i64, Fault> {
    if b == 0 {
        return Err(Fault::DivisionByZero);
    }
    let remainder = a.wrapping_rem(b);
    if remainder != 0 && ((remainder < 0) != (b < 0)) {
        Ok(remainder + b)
    } else {
        Ok(remainder)
    }
}

/// `a ^ b`, always real. A whole exponent is repeated multiplication, so
/// that `x ^ 2` is exactly `x * x`.
fn power(base: Real, exponent: Num) -> Res {
    const NO_POWER: &str = "the power is not defined for this base and exponent";
    match exponent {
        Num::Int(whole) => {
            if base == Real::ZERO && whole < 0 {
                return Err(Fault::Domain(NO_POWER));
            }
            match i32::try_from(whole) {
                Ok(whole) => defined(base.powi(whole).get()),
                Err(_) => base
                    .try_powf(Real::from_i64(whole))
                    .map(Value::Real)
                    .map_err(|_| Fault::Domain(NO_POWER)),
            }
        }
        Num::Real(exponent) => base
            .try_powf(exponent)
            .map(Value::Real)
            .map_err(|_| Fault::Domain(NO_POWER)),
    }
}

/// `a op b` for the arithmetic operators (05 §4.3).
///
/// Arithmetic on two ints is integer arithmetic, except `/` and `^`,
/// which always give a real. An int meeting a real becomes real. `+`
/// also joins two texts and two lists. `%` takes ints only.
pub(crate) fn arith(op: ArithOp, a: &Value, b: &Value, limits: &Limits) -> Res {
    if a.is_null() || b.is_null() {
        return Err(Fault::Null);
    }
    if op == ArithOp::Add {
        match (a, b) {
            (Value::Str(x), Value::Str(y)) => {
                if x.len() + y.len() > limits.max_value_size {
                    return Err(Fault::TooLarge);
                }
                let mut joined = String::with_capacity(x.len() + y.len());
                joined.push_str(x);
                joined.push_str(y);
                return Ok(Value::Str(Arc::from(joined)));
            }
            (Value::List(x), Value::List(y)) => {
                if x.len() + y.len() > limits.max_value_size {
                    return Err(Fault::TooLarge);
                }
                return Ok(Value::List(x.iter().chain(y.iter()).cloned().collect()));
            }
            (Value::Int(_) | Value::Real(_), Value::Int(_) | Value::Real(_)) => {}
            (Value::Int(_) | Value::Real(_), other) | (other, _) => {
                return Err(wrong(WANT_ADDABLE, other));
            }
        }
    }
    let (x, y) = match (num(a), num(b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(fault), _) | (_, Err(fault)) => return Err(fault),
    };
    match (op, x, y) {
        (ArithOp::Add, Num::Int(x), Num::Int(y)) => {
            x.checked_add(y).map(Value::Int).ok_or(Fault::Overflow)
        }
        (ArithOp::Sub, Num::Int(x), Num::Int(y)) => {
            x.checked_sub(y).map(Value::Int).ok_or(Fault::Overflow)
        }
        (ArithOp::Mul, Num::Int(x), Num::Int(y)) => {
            x.checked_mul(y).map(Value::Int).ok_or(Fault::Overflow)
        }
        (ArithOp::Rem, Num::Int(x), Num::Int(y)) => floor_rem(x, y).map(Value::Int),
        (ArithOp::Rem, Num::Real(_), _) => Err(wrong(WANT_INT, a)),
        (ArithOp::Rem, _, Num::Real(_)) => Err(wrong(WANT_INT, b)),
        (ArithOp::Add, x, y) => defined(x.real().get() + y.real().get()),
        (ArithOp::Sub, x, y) => defined(x.real().get() - y.real().get()),
        (ArithOp::Mul, x, y) => defined(x.real().get() * y.real().get()),
        (ArithOp::Div, x, y) => {
            if y.real() == Real::ZERO {
                Err(Fault::DivisionByZero)
            } else {
                defined(x.real().get() / y.real().get())
            }
        }
        (ArithOp::Pow, x, y) => power(x.real(), y),
    }
}

/// Whether two values are equal as the language sees it: an int equals
/// the real of the same value, an id equals the text of its qualified
/// id, lists and maps are equal when their contents are.
pub(crate) fn equal<H: Host + ?Sized>(a: &Value, b: &Value, host: &H) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Int(_) | Value::Real(_), Value::Int(_) | Value::Real(_)) => {
            a.as_number() == b.as_number()
        }
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Id(x), Value::Id(y)) => x == y,
        (Value::Id(id), Value::Str(text)) | (Value::Str(text), Value::Id(id)) => {
            host.def_name(*id) == Some(&**text)
        }
        (Value::Subject(x), Value::Subject(y)) => x == y,
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(x, y)| equal(x, y, host))
        }
        (Value::Map(x), Value::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|((kx, vx), (ky, vy))| kx == ky && equal(vx, vy, host))
        }
        _ => false,
    }
}

/// `a op b` for the comparisons. A comparison with null is false
/// (05 §4.4); `== null` and `!= null` never reach here, because the
/// checker turns them into null tests.
pub(crate) fn compare<H: Host + ?Sized>(
    op: CmpOp,
    a: &Value,
    b: &Value,
    host: &H,
) -> Result<bool, Fault> {
    if a.is_null() || b.is_null() {
        return Ok(false);
    }
    let ordering = match op {
        CmpOp::Eq => return Ok(equal(a, b, host)),
        CmpOp::Ne => return Ok(!equal(a, b, host)),
        _ => match (a, b) {
            (Value::Int(x), Value::Int(y)) => x.cmp(y),
            (Value::Str(x), Value::Str(y)) => x.cmp(y),
            (Value::Int(_) | Value::Real(_), Value::Int(_) | Value::Real(_)) => {
                match (a.as_number(), b.as_number()) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    _ => return Ok(false),
                }
            }
            (Value::Int(_) | Value::Real(_) | Value::Str(_), other) | (other, _) => {
                return Err(wrong(WANT_ORDERED, other));
            }
        },
    };
    Ok(match op {
        CmpOp::Lt => ordering == Ordering::Less,
        CmpOp::Le => ordering != Ordering::Greater,
        CmpOp::Gt => ordering == Ordering::Greater,
        CmpOp::Ge => ordering != Ordering::Less,
        CmpOp::Eq | CmpOp::Ne => false,
    })
}

/// The key a value stands for in a map: a text is itself, an id is its
/// qualified id. Null has no key.
pub(crate) fn key_of<H: Host + ?Sized>(value: &Value, host: &H) -> Result<Option<Arc<str>>, Fault> {
    match value {
        Value::Str(text) => Ok(Some(Arc::clone(text))),
        Value::Id(id) => Ok(Some(match host.def_name(*id) {
            Some(name) => Arc::from(name),
            None => Arc::from(value.to_string()),
        })),
        Value::Null => Ok(None),
        other => Err(wrong(WANT_KEY, other)),
    }
}

/// `item in collection`: membership in a list, or among a map's keys.
/// Null on either side is false.
pub(crate) fn contains<H: Host + ?Sized>(
    item: &Value,
    collection: &Value,
    host: &H,
) -> Result<bool, Fault> {
    if item.is_null() {
        return Ok(false);
    }
    match collection {
        Value::Null => Ok(false),
        Value::List(items) => Ok(items.iter().any(|element| equal(item, element, host))),
        Value::Map(entries) => {
            Ok(key_of(item, host)?.is_some_and(|key| entries.contains_key(&key)))
        }
        other => Err(wrong(WANT_COLLECTION, other)),
    }
}

/// `object[index]`: a list element counting from zero, or a map entry.
/// Out of range, absent, or read from null, is null.
pub(crate) fn index<H: Host + ?Sized>(object: &Value, index: &Value, host: &H) -> Res {
    match object {
        Value::Null => Ok(Value::Null),
        Value::List(items) => match index {
            Value::Int(at) => Ok(usize::try_from(*at)
                .ok()
                .and_then(|at| items.get(at))
                .cloned()
                .unwrap_or(Value::Null)),
            Value::Null => Ok(Value::Null),
            other => Err(wrong(WANT_INT, other)),
        },
        Value::Map(entries) => Ok(key_of(index, host)?
            .and_then(|key| entries.get(&key).cloned())
            .unwrap_or(Value::Null)),
        other => Err(wrong(WANT_COLLECTION, other)),
    }
}

/// A list from its elements.
pub(crate) fn make_list(items: &[Value], limits: &Limits) -> Res {
    if items.len() > limits.max_value_size {
        return Err(Fault::TooLarge);
    }
    Ok(Value::List(items.into()))
}

/// A map from keys and values laid out in turn: key, value, key, value.
/// A key given twice keeps its last value. A null key is a fault.
pub(crate) fn make_map<H: Host + ?Sized>(flat: &[Value], limits: &Limits, host: &H) -> Res {
    let mut entries = Map::new();
    for pair in flat.chunks_exact(2) {
        let key = key_of(&pair[0], host)?.ok_or(Fault::Null)?;
        entries.insert(key, pair[1].clone());
    }
    if entries.len() > limits.max_value_size {
        return Err(Fault::TooLarge);
    }
    Ok(Value::Map(Arc::new(entries)))
}

/// Writes a value as text: what `str` gives and what a text template
/// shows. An id is written as its qualified id.
pub(crate) fn write_text<H: Host + ?Sized>(out: &mut String, value: &Value, host: &H) {
    match value {
        Value::Id(id) => match host.def_name(*id) {
            Some(name) => out.push_str(name),
            None => {
                let _ = write!(out, "{value}");
            }
        },
        Value::List(items) => {
            out.push('[');
            for (at, item) in items.iter().enumerate() {
                if at > 0 {
                    out.push_str(", ");
                }
                write_text(out, item, host);
            }
            out.push(']');
        }
        Value::Map(entries) => {
            out.push('{');
            for (at, (key, item)) in entries.iter().enumerate() {
                if at > 0 {
                    out.push_str(", ");
                }
                out.push_str(key);
                out.push_str(": ");
                write_text(out, item, host);
            }
            out.push('}');
        }
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

/// Writes a value as a text template shows it: a number with the digits
/// its format asks for, anything else as [`write_text`] does.
pub(crate) fn write_formatted<H: Host + ?Sized>(
    out: &mut String,
    value: &Value,
    format: Option<Format>,
    host: &H,
) {
    match (value, format) {
        (Value::Real(real), Some(format)) => {
            let _ = write!(out, "{:.*}", usize::from(format.precision), real.get());
        }
        (Value::Int(int), Some(format)) => {
            let _ = write!(out, "{int}");
            if format.precision > 0 {
                out.push('.');
                for _ in 0..format.precision {
                    out.push('0');
                }
            }
        }
        _ => write_text(out, value, host),
    }
}
