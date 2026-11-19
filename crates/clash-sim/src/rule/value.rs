//! What effects do to values: holding them to the limits on a write,
//! stepping into the entries of a place, adding, and telling whether a
//! value is of a type.
//!
//! Values are immutable and shared, so "changing" an entry builds a new
//! value around the old ones. Nothing here panics, whatever the values.

use std::sync::Arc;

use clash_core::Real;
use clash_expr::{Map, Type, Value};

use super::host::RuleLimits;
use super::reason;

/// Why a value could not be written or changed: the code of the fault
/// and what to say.
pub(crate) type ValueFault = (&'static str, String);

/// Holds a value an effect is about to write to the limits (05 §13.2):
/// no list or map of more elements, and no text of more bytes, than
/// `max_value_size`, and no nesting deeper than `max_value_depth`.
///
/// The walk goes no deeper than the limit plus one, so a value built by
/// `$x = [$x]` in a loop is refused long before it could exhaust the
/// stack of whatever later hashes, compares or drops it.
pub(crate) fn measure(value: &Value, limits: &RuleLimits) -> Result<(), ValueFault> {
    measure_at(value, limits, 0)
}

fn measure_at(value: &Value, limits: &RuleLimits, depth: usize) -> Result<(), ValueFault> {
    let too_large = |what: &str, len: usize| {
        (
            reason::VALUE_TOO_LARGE,
            format!(
                "a {what} of {len} is larger than a value may be ({})",
                limits.max_value_size
            ),
        )
    };
    let too_deep = || {
        (
            reason::VALUE_TOO_DEEP,
            format!(
                "the value nests lists and maps more than {} deep",
                limits.max_value_depth
            ),
        )
    };
    match value {
        Value::Str(text) if text.len() > limits.max_value_size => {
            Err(too_large("text of bytes", text.len()))
        }
        Value::List(items) => {
            if depth >= limits.max_value_depth {
                return Err(too_deep());
            }
            if items.len() > limits.max_value_size {
                return Err(too_large("list of elements", items.len()));
            }
            items
                .iter()
                .try_for_each(|item| measure_at(item, limits, depth + 1))
        }
        Value::Map(entries) => {
            if depth >= limits.max_value_depth {
                return Err(too_deep());
            }
            if entries.len() > limits.max_value_size {
                return Err(too_large("map of entries", entries.len()));
            }
            entries
                .values()
                .try_for_each(|entry| measure_at(entry, limits, depth + 1))
        }
        _ => Ok(()),
    }
}

/// A key as the text a map is keyed by: a text as it is, an id as its
/// qualified id (05 §3).
pub(crate) fn key_text(
    key: &Value,
    def_name: &dyn Fn(clash_expr::DefRef) -> Option<String>,
) -> Option<Arc<str>> {
    match key {
        Value::Str(text) => Some(text.clone()),
        Value::Id(id) => def_name(*id).map(Arc::from),
        _ => None,
    }
}

fn bad_key(message: impl Into<String>) -> ValueFault {
    (reason::BAD_KEY, message.into())
}

/// The entry of `current` that `keys` lead to; null where there is none.
pub(crate) fn get(
    current: &Value,
    keys: &[Value],
    def_name: &dyn Fn(clash_expr::DefRef) -> Option<String>,
) -> Value {
    let mut at = current.clone();
    for key in keys {
        at = match (&at, key) {
            (Value::Map(entries), key) => key_text(key, def_name)
                .and_then(|text| entries.get(&text).cloned())
                .unwrap_or_default(),
            (Value::List(items), Value::Int(index)) => usize::try_from(*index)
                .ok()
                .and_then(|index| items.get(index).cloned())
                .unwrap_or_default(),
            _ => Value::Null,
        };
    }
    at
}

/// `current` with the entry `keys` lead to replaced by `value`, or
/// removed if `value` is `None`.
///
/// A map is stepped into by a text or an id and gains the entry if it
/// has none; a null where a map is needed becomes one, so that a tally
/// can be kept in a variable that was never set. A list is stepped into
/// by an index it has, or by its length, which appends.
pub(crate) fn put(
    current: &Value,
    keys: &[Value],
    value: Option<Value>,
    def_name: &dyn Fn(clash_expr::DefRef) -> Option<String>,
) -> Result<Value, ValueFault> {
    let Some((key, rest)) = keys.split_first() else {
        return Ok(value.unwrap_or_default());
    };
    match current {
        Value::Map(_) | Value::Null => {
            let Some(text) = key_text(key, def_name) else {
                return Err(bad_key(format!(
                    "a map is keyed by a text or an id, and the key is a {}",
                    key.type_name()
                )));
            };
            let mut entries: Map = match current {
                Value::Map(entries) => (**entries).clone(),
                _ => Map::new(),
            };
            let inner = entries.get(&text).cloned().unwrap_or_default();
            if rest.is_empty() && value.is_none() {
                entries.remove(&text);
            } else {
                entries.insert(text, put(&inner, rest, value, def_name)?);
            }
            Ok(Value::Map(Arc::new(entries)))
        }
        Value::List(items) => {
            let Value::Int(index) = key else {
                return Err(bad_key(format!(
                    "a list is indexed by an int, and the index is a {}",
                    key.type_name()
                )));
            };
            let at = usize::try_from(*index).ok().filter(|at| *at <= items.len());
            let Some(at) = at else {
                return Err(bad_key(format!(
                    "index {index} is outside a list of {}",
                    items.len()
                )));
            };
            let mut list: Vec<Value> = items.to_vec();
            if rest.is_empty() && value.is_none() {
                if at < list.len() {
                    list.remove(at);
                }
                return Ok(Value::from(list));
            }
            let inner = list.get(at).cloned().unwrap_or_default();
            let entry = put(&inner, rest, value, def_name)?;
            if at == list.len() {
                list.push(entry);
            } else {
                list[at] = entry;
            }
            Ok(Value::from(list))
        }
        other => Err(bad_key(format!(
            "a {} has no entries to step into",
            other.type_name()
        ))),
    }
}

/// What `add` makes of what is there and what is added (05 §10.3): the
/// sum of two numbers, an int if both are; a list with one more
/// element; and, for something never set, the number added.
pub(crate) fn add(current: &Value, amount: &Value) -> Result<Value, ValueFault> {
    let mismatch = |message: String| (reason::TYPE_MISMATCH, message);
    match (current, amount) {
        (Value::List(items), amount) => {
            let mut list: Vec<Value> = items.to_vec();
            list.push(amount.clone());
            Ok(Value::from(list))
        }
        (Value::Null, Value::Int(_) | Value::Real(_)) => Ok(amount.clone()),
        (Value::Int(a), Value::Int(b)) => a
            .checked_add(*b)
            .map(Value::Int)
            .ok_or_else(|| mismatch("the sum does not fit in a 64-bit integer".to_owned())),
        (Value::Int(_) | Value::Real(_), Value::Int(_) | Value::Real(_)) => {
            let sum = current.as_number().unwrap_or(Real::ZERO)
                + amount.as_number().unwrap_or(Real::ZERO);
            Ok(Value::Real(sum))
        }
        (current, amount) => Err(mismatch(format!(
            "`add` adds a number to a number or appends to a list; this adds a {} to a {}",
            amount.type_name(),
            current.type_name()
        ))),
    }
}

/// Whether a value may stand where a type is wanted, as far as can be
/// told without walking it. Null passes everywhere: the language
/// carries it (05 §4.4).
pub(crate) fn conforms(value: &Value, ty: &Type) -> bool {
    match (ty.non_null(), value) {
        (Type::Any, _) | (_, Value::Null) => true,
        (Type::Bool, Value::Bool(_))
        | (Type::Int, Value::Int(_))
        | (Type::Real | Type::Number, Value::Int(_) | Value::Real(_))
        | (Type::Str, Value::Str(_))
        | (Type::List(_), Value::List(_))
        | (Type::Map(_), Value::Map(_)) => true,
        (Type::Id(ty), Value::Id(id)) => id.ty == *ty,
        (Type::Subject(kind), Value::Subject(handle)) => handle.kind == *kind,
        _ => false,
    }
}

/// The type of a constant, for a local that is given one.
pub(crate) fn literal_type(value: &Value) -> Type {
    match value {
        Value::Bool(_) => Type::Bool,
        Value::Int(_) => Type::Int,
        Value::Real(_) => Type::Real,
        Value::Str(_) => Type::Str,
        Value::Id(id) => Type::Id(id.ty),
        Value::Subject(handle) => Type::Subject(handle.kind),
        Value::List(items) => {
            let mut joined: Option<Type> = None;
            for item in items.iter() {
                let ty = literal_type(item);
                joined = Some(match joined {
                    None => ty,
                    Some(so_far) => so_far.join(&ty),
                });
            }
            Type::list(joined.unwrap_or(Type::Any))
        }
        Value::Map(_) => Type::map(Type::Any),
        // A local that starts as null may be given anything later.
        Value::Null => Type::Any,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_names(_: clash_expr::DefRef) -> Option<String> {
        None
    }

    #[test]
    fn a_value_is_held_to_size_and_depth() {
        let limits = RuleLimits {
            max_value_size: 4,
            max_value_depth: 3,
            ..RuleLimits::default()
        };
        assert!(measure(&Value::from("four"), &limits).is_ok());
        assert_eq!(
            measure(&Value::from("fiver"), &limits).unwrap_err().0,
            reason::VALUE_TOO_LARGE
        );
        let four = Value::list((0..4).map(Value::from));
        assert!(measure(&four, &limits).is_ok());
        let five = Value::list((0..5).map(Value::from));
        assert_eq!(
            measure(&five, &limits).unwrap_err().0,
            reason::VALUE_TOO_LARGE
        );
        // A large list inside a small one is found too.
        assert!(measure(&Value::list([five]), &limits).is_err());

        let mut nested = Value::Int(0);
        for _ in 0..3 {
            nested = Value::list([nested]);
        }
        assert!(measure(&nested, &limits).is_ok());
        let deeper = Value::map([("k", nested)]);
        assert_eq!(
            measure(&deeper, &limits).unwrap_err().0,
            reason::VALUE_TOO_DEEP
        );
    }

    #[test]
    fn a_very_deep_value_is_refused_without_walking_all_of_it() {
        let limits = RuleLimits::default();
        let mut nested = Value::Int(0);
        // Deep enough that walking all of it by recursion would be
        // unwise, and shallow enough to drop safely in a test.
        for _ in 0..2_000 {
            nested = Value::list([nested]);
        }
        assert_eq!(
            measure(&nested, &limits).unwrap_err().0,
            reason::VALUE_TOO_DEEP
        );
        // Take it apart from the outside in, so that dropping it does
        // not recurse either.
        while let Value::List(items) = nested {
            nested = items.first().cloned().unwrap_or_default();
        }
    }

    #[test]
    fn entries_are_read_put_and_removed() {
        let tally = put(
            &Value::Null,
            &[Value::from("ash")],
            Some(Value::Int(3)),
            &no_names,
        )
        .expect("a null becomes a map");
        assert_eq!(tally, Value::map([("ash", Value::Int(3))]));
        let tally = put(
            &tally,
            &[Value::from("iron")],
            Some(Value::Int(1)),
            &no_names,
        )
        .unwrap();
        assert_eq!(
            get(&tally, &[Value::from("iron")], &no_names),
            Value::Int(1)
        );
        assert_eq!(get(&tally, &[Value::from("gold")], &no_names), Value::Null);
        let without = put(&tally, &[Value::from("ash")], None, &no_names).unwrap();
        assert_eq!(without, Value::map([("iron", Value::Int(1))]));

        let list = Value::list([Value::Int(1), Value::Int(2)]);
        let changed = put(&list, &[Value::Int(1)], Some(Value::Int(9)), &no_names).unwrap();
        assert_eq!(changed, Value::list([Value::Int(1), Value::Int(9)]));
        let longer = put(&list, &[Value::Int(2)], Some(Value::Int(3)), &no_names).unwrap();
        assert_eq!(longer.as_list().map(<[Value]>::len), Some(3));
        assert_eq!(
            put(&list, &[Value::Int(5)], Some(Value::Int(3)), &no_names)
                .unwrap_err()
                .0,
            reason::BAD_KEY
        );
        assert_eq!(
            put(&list, &[Value::from("x")], Some(Value::Int(3)), &no_names)
                .unwrap_err()
                .0,
            reason::BAD_KEY
        );
        assert_eq!(
            put(
                &Value::Int(1),
                &[Value::from("x")],
                Some(Value::Int(3)),
                &no_names
            )
            .unwrap_err()
            .0,
            reason::BAD_KEY
        );
        // Two keys deep, through a map that is made on the way.
        let nested = put(
            &Value::Null,
            &[Value::from("a"), Value::from("b")],
            Some(Value::Int(1)),
            &no_names,
        )
        .unwrap();
        assert_eq!(
            get(&nested, &[Value::from("a"), Value::from("b")], &no_names),
            Value::Int(1)
        );
    }

    #[test]
    fn add_sums_numbers_and_appends_to_lists() {
        assert_eq!(add(&Value::Int(2), &Value::Int(3)).unwrap(), Value::Int(5));
        assert_eq!(
            add(&Value::Int(2), &Value::Real(Real::of(0.5))).unwrap(),
            Value::Real(Real::of(2.5))
        );
        assert_eq!(add(&Value::Null, &Value::Int(1)).unwrap(), Value::Int(1));
        assert_eq!(
            add(&Value::list([Value::Int(1)]), &Value::from("x")).unwrap(),
            Value::list([Value::Int(1), Value::from("x")])
        );
        assert_eq!(
            add(&Value::Int(i64::MAX), &Value::Int(1)).unwrap_err().0,
            reason::TYPE_MISMATCH
        );
        assert_eq!(
            add(&Value::from("a"), &Value::Int(1)).unwrap_err().0,
            reason::TYPE_MISMATCH
        );
    }

    #[test]
    fn a_value_is_told_from_a_type() {
        assert!(conforms(&Value::Int(1), &Type::Real));
        assert!(conforms(&Value::Null, &Type::Int));
        assert!(!conforms(&Value::from("x"), &Type::Int));
        assert!(conforms(&Value::from("x"), &Type::Any));
        assert!(conforms(&Value::list([]), &Type::list(Type::Int)));
        assert!(!conforms(&Value::Bool(true), &Type::Str.nullable()));
    }
}
