//! [`Type`]: what the checker knows about a value before it exists
//! (05 §3).

use crate::value::{DefTypeId, KindId};

/// The type of an expression, a property, a parameter or a variable.
///
/// ```
/// use clash_expr::Type;
///
/// let reading = Type::Real.nullable();
/// assert!(reading.may_be_null());
/// assert_eq!(reading.non_null(), &Type::Real);
/// // An integer is accepted where a real is wanted, not the reverse.
/// assert!(Type::Real.accepts(&Type::Int));
/// assert!(!Type::Int.accepts(&Type::Real));
/// assert!(Type::list(Type::Real).accepts(&Type::list(Type::Int)));
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Type {
    /// Unknown until run time. Accepted everywhere and accepts everything;
    /// what is done with it is checked when it runs.
    Any,
    /// Only `null`.
    Null,
    /// `true` or `false`.
    Bool,
    /// A 64-bit integer.
    Int,
    /// A real number.
    Real,
    /// An integer or a real: which is not known until run time. As a
    /// parameter it accepts either.
    Number,
    /// A text.
    Str,
    /// A definition of one type: `id<material>`.
    Id(DefTypeId),
    /// A reference to a subject of one kind: `entity`, `loc`.
    Subject(KindId),
    /// Ordered values of one type: `list<int>`.
    List(Box<Type>),
    /// Values of one type by key: `map<real>`.
    Map(Box<Type>),
    /// A value of the inner type, or null: `real?`.
    Nullable(Box<Type>),
}

impl Type {
    /// `list<element>`.
    pub fn list(element: Type) -> Type {
        Type::List(Box::new(element))
    }

    /// `map<value>`.
    pub fn map(value: Type) -> Type {
        Type::Map(Box::new(value))
    }

    /// This type, or null. `any` and `null` already include null and are
    /// returned unchanged, as is a type that is already nullable.
    #[must_use]
    pub fn nullable(self) -> Type {
        match self {
            Type::Any | Type::Null | Type::Nullable(_) => self,
            other => Type::Nullable(Box::new(other)),
        }
    }

    /// The type without its "or null".
    pub fn non_null(&self) -> &Type {
        match self {
            Type::Nullable(inner) => inner,
            other => other,
        }
    }

    /// Whether the checker knows that a value of this type may be null.
    /// `any` is not counted: nothing is known about it.
    pub const fn may_be_null(&self) -> bool {
        matches!(self, Type::Null | Type::Nullable(_))
    }

    /// Whether this is `int`, `real` or `number`.
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Type::Int | Type::Real | Type::Number)
    }

    /// The type of an element of this list or a value of this map; `any`
    /// for `any`, and `None` for anything else.
    pub fn element(&self) -> Option<&Type> {
        match self.non_null() {
            Type::List(element) | Type::Map(element) => Some(element),
            Type::Any => Some(&Type::Any),
            _ => None,
        }
    }

    /// Whether a value of type `from` may be used where this type is
    /// wanted, leaving aside whether it may be null.
    ///
    /// An `int` is accepted as a `real`. `any` is accepted everywhere and
    /// accepts everything, as is `number` among numbers: those are checked
    /// when the expression runs. Lists and maps follow their elements.
    pub fn accepts(&self, from: &Type) -> bool {
        match (self.non_null(), from.non_null()) {
            (Type::Any, _) | (_, Type::Any) => true,
            // Null is a question of nullability, which the caller asks
            // separately.
            (_, Type::Null) => true,
            (Type::Null, _) => false,
            (Type::Number | Type::Real, from) => from.is_numeric(),
            (Type::Int, Type::Int | Type::Number) => true,
            (Type::List(to), Type::List(from)) | (Type::Map(to), Type::Map(from)) => {
                to.accepts(from)
            }
            (to, from) => to == from,
        }
    }

    /// The narrowest type that holds values of both types: what a list of
    /// the two, or the two branches of an `if`, has.
    #[must_use]
    pub fn join(&self, other: &Type) -> Type {
        let nullable = self.may_be_null() || other.may_be_null();
        let joined = match (self.non_null(), other.non_null()) {
            (Type::Null, other) | (other, Type::Null) => other.clone(),
            (a, b) if a == b => a.clone(),
            (Type::Int | Type::Real, Type::Int | Type::Real) => Type::Real,
            (a, b) if a.is_numeric() && b.is_numeric() => Type::Number,
            (Type::List(a), Type::List(b)) => Type::list(a.join(b)),
            (Type::Map(a), Type::Map(b)) => Type::map(a.join(b)),
            _ => Type::Any,
        };
        if nullable { joined.nullable() } else { joined }
    }

    /// The type of arithmetic on numbers of these types: `int` if all
    /// are, `real` if all are known and one is real, `number` if any is
    /// not known until run time.
    pub fn numeric<'a>(types: impl IntoIterator<Item = &'a Type>) -> Type {
        let mut result = Type::Int;
        for ty in types {
            match ty.non_null() {
                Type::Int => {}
                Type::Real if result != Type::Number => result = Type::Real,
                Type::Real => {}
                _ => result = Type::Number,
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(raw: u16) -> Type {
        Type::Subject(KindId::new(raw))
    }

    fn id(raw: u16) -> Type {
        Type::Id(DefTypeId::new(raw))
    }

    #[test]
    fn nullable_wraps_once() {
        assert_eq!(Type::Real.nullable(), Type::Nullable(Box::new(Type::Real)));
        assert_eq!(Type::Real.nullable().nullable(), Type::Real.nullable());
        assert_eq!(Type::Any.nullable(), Type::Any);
        assert_eq!(Type::Null.nullable(), Type::Null);
        assert_eq!(Type::Real.nullable().non_null(), &Type::Real);
        assert_eq!(Type::Real.non_null(), &Type::Real);
        assert!(Type::Null.may_be_null());
        assert!(Type::Int.nullable().may_be_null());
        assert!(!Type::Int.may_be_null());
        assert!(!Type::Any.may_be_null());
    }

    #[test]
    fn accepts_follows_the_rules_of_the_language() {
        let all = [
            Type::Bool,
            Type::Int,
            Type::Real,
            Type::Number,
            Type::Str,
            id(0),
            id(1),
            kind(0),
            kind(1),
            Type::list(Type::Int),
            Type::map(Type::Int),
        ];
        for ty in &all {
            // Everything accepts itself, and `any` goes both ways.
            assert!(ty.accepts(ty), "{ty:?}");
            assert!(Type::Any.accepts(ty), "{ty:?}");
            assert!(ty.accepts(&Type::Any), "{ty:?}");
            // Nullability is not this function's business.
            assert!(ty.accepts(&ty.clone().nullable()), "{ty:?}");
            assert!(ty.clone().nullable().accepts(ty), "{ty:?}");
            assert!(ty.accepts(&Type::Null), "{ty:?}");
            assert!(!Type::Null.accepts(ty), "{ty:?}");
        }
        assert!(Type::Null.accepts(&Type::Null));
        // Numbers widen and do not narrow.
        assert!(Type::Real.accepts(&Type::Int));
        assert!(!Type::Int.accepts(&Type::Real));
        assert!(Type::Number.accepts(&Type::Int));
        assert!(Type::Number.accepts(&Type::Real));
        assert!(Type::Real.accepts(&Type::Number));
        assert!(Type::Int.accepts(&Type::Number));
        // Nothing else mixes.
        assert!(!Type::Bool.accepts(&Type::Int));
        assert!(!Type::Str.accepts(&Type::Int));
        assert!(!Type::Int.accepts(&Type::Str));
        assert!(!id(0).accepts(&id(1)));
        assert!(!id(0).accepts(&Type::Str));
        assert!(!kind(0).accepts(&kind(1)));
        assert!(!kind(0).accepts(&id(0)));
        // Lists and maps follow their elements.
        assert!(Type::list(Type::Real).accepts(&Type::list(Type::Int)));
        assert!(!Type::list(Type::Int).accepts(&Type::list(Type::Real)));
        assert!(Type::list(Type::Any).accepts(&Type::list(Type::Str)));
        assert!(Type::list(Type::Str).accepts(&Type::list(Type::Any)));
        assert!(Type::map(Type::Real).accepts(&Type::map(Type::Int)));
        assert!(!Type::map(Type::Int).accepts(&Type::list(Type::Int)));
        assert!(!Type::list(Type::Int).accepts(&Type::Int));
    }

    #[test]
    fn join_finds_the_narrowest_common_type() {
        assert_eq!(Type::Int.join(&Type::Int), Type::Int);
        assert_eq!(Type::Int.join(&Type::Real), Type::Real);
        assert_eq!(Type::Real.join(&Type::Int), Type::Real);
        assert_eq!(Type::Int.join(&Type::Number), Type::Number);
        assert_eq!(Type::Int.join(&Type::Str), Type::Any);
        assert_eq!(Type::Int.join(&Type::Any), Type::Any);
        assert_eq!(Type::Int.join(&Type::Null), Type::Int.nullable());
        assert_eq!(Type::Null.join(&Type::Int), Type::Int.nullable());
        assert_eq!(Type::Null.join(&Type::Null), Type::Null);
        assert_eq!(
            Type::Int.nullable().join(&Type::Real),
            Type::Real.nullable()
        );
        assert_eq!(
            Type::list(Type::Int).join(&Type::list(Type::Real)),
            Type::list(Type::Real)
        );
        assert_eq!(
            Type::map(Type::Int).join(&Type::map(Type::Str)),
            Type::map(Type::Any)
        );
        assert_eq!(kind(0).join(&kind(0)), kind(0));
        assert_eq!(kind(0).join(&kind(1)), Type::Any);
    }

    #[test]
    fn numeric_is_int_only_if_everything_is() {
        assert_eq!(Type::numeric([&Type::Int, &Type::Int]), Type::Int);
        assert_eq!(Type::numeric([&Type::Int, &Type::Real]), Type::Real);
        assert_eq!(Type::numeric([&Type::Real, &Type::Int]), Type::Real);
        assert_eq!(Type::numeric([&Type::Int, &Type::Any]), Type::Number);
        assert_eq!(Type::numeric([&Type::Number, &Type::Real]), Type::Number);
        assert_eq!(Type::numeric([&Type::Any, &Type::Real]), Type::Number);
        assert_eq!(
            Type::numeric([&Type::Int.nullable(), &Type::Int]),
            Type::Int
        );
        assert_eq!(Type::numeric([]), Type::Int);
    }

    #[test]
    fn element_reads_lists_and_maps() {
        assert_eq!(Type::list(Type::Int).element(), Some(&Type::Int));
        assert_eq!(Type::map(Type::Str).element(), Some(&Type::Str));
        assert_eq!(Type::list(Type::Int).nullable().element(), Some(&Type::Int));
        assert_eq!(Type::Any.element(), Some(&Type::Any));
        assert_eq!(Type::Int.element(), None);
        assert!(Type::Number.is_numeric());
        assert!(!Type::Str.is_numeric());
    }
}
