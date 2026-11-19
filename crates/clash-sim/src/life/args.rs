//! Reading a verb's arguments (05 §10.1): one value for each declared
//! argument, in the order of the verb's specification, null for an
//! optional one left out. Each was checked against its declared type at
//! load where the type was known, and may be null all the same, so every
//! read here answers with a [`Reason`] and never panics.

use clash_core::{EntityId, Real};
use clash_expr::{Handle, Value};

use crate::Sim;
use crate::event::Subject;
use crate::reason::Reason;
use crate::rule::VerbCall;

/// The code of the reason for an argument a verb cannot use: null where
/// one is needed, or a value of the wrong sort. Values: `argument`.
pub const BAD_ARGUMENT: &str = "verb.bad_argument";

fn bad(name: &str, wanted: &str) -> Reason {
    Reason::new(
        BAD_ARGUMENT,
        format!("the argument `{name}` must be {wanted}"),
    )
    .with("argument", name)
}

/// The value of the argument at a position: null for one left out.
pub(crate) fn value<'a>(call: &VerbCall<'a>, at: usize) -> &'a Value {
    const NULL: &Value = &Value::Null;
    call.args.get(at).unwrap_or(NULL)
}

/// An entity that still exists.
pub(crate) fn entity(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<EntityId, Reason> {
    match sim.subject_of(value(call, at)) {
        Some(Subject::Entity(id)) if sim.truth().entities().contains(id) => Ok(id),
        _ => Err(bad(name, "an entity that exists")),
    }
}

/// An entity, or `None` for an argument left out.
pub(crate) fn optional_entity(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<Option<EntityId>, Reason> {
    if value(call, at).is_null() {
        return Ok(None);
    }
    entity(sim, call, at, name).map(Some)
}

/// A number, or `None` for an argument left out.
pub(crate) fn optional_real(
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<Option<Real>, Reason> {
    match value(call, at) {
        Value::Null => Ok(None),
        other => other
            .as_number()
            .map(Some)
            .ok_or_else(|| bad(name, "a number")),
    }
}

/// A number.
pub(crate) fn real(call: &VerbCall<'_>, at: usize, name: &str) -> Result<Real, Reason> {
    optional_real(call, at, name)?.ok_or_else(|| bad(name, "a number"))
}

/// A truth value: false for an argument left out.
pub(crate) fn flag(call: &VerbCall<'_>, at: usize, name: &str) -> Result<bool, Reason> {
    match value(call, at) {
        Value::Null => Ok(false),
        Value::Bool(flag) => Ok(*flag),
        _ => Err(bad(name, "true or false")),
    }
}

/// A text, or `None` for an argument left out.
pub(crate) fn optional_text<'a>(
    call: &VerbCall<'a>,
    at: usize,
    name: &str,
) -> Result<Option<&'a str>, Reason> {
    match value(call, at) {
        Value::Null => Ok(None),
        Value::Str(text) => Ok(Some(text)),
        _ => Err(bad(name, "a text")),
    }
}

/// A text.
pub(crate) fn text<'a>(call: &VerbCall<'a>, at: usize, name: &str) -> Result<&'a str, Reason> {
    optional_text(call, at, name)?.ok_or_else(|| bad(name, "a text"))
}

/// A handle of some kind of subject, or `None` for an argument left
/// out.
pub(crate) fn optional_handle(
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<Option<Handle>, Reason> {
    match value(call, at) {
        Value::Null => Ok(None),
        Value::Subject(handle) => Ok(Some(*handle)),
        _ => Err(bad(name, "a subject")),
    }
}

/// The dense number of a definition of one type: an id of that type, or
/// a text holding its qualified id. `None` for an argument left out.
pub(crate) fn optional_def(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
    ty: &str,
) -> Result<Option<usize>, Reason> {
    let registry = sim.exprs().registry();
    let wanted = || bad(name, &format!("the id of a `{ty}`"));
    let ty_id = registry.def_type_id(ty).ok_or_else(wanted)?;
    match value(call, at) {
        Value::Null => Ok(None),
        Value::Id(id) if id.ty == ty_id => Ok(Some(id.index as usize)),
        Value::Str(qualified) => registry
            .def_ref(ty_id, qualified)
            .map(|found| Some(found.index as usize))
            .ok_or_else(wanted),
        _ => Err(wanted()),
    }
}

/// The dense number of a definition of one type.
pub(crate) fn def(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
    ty: &str,
) -> Result<usize, Reason> {
    optional_def(sim, call, at, name, ty)?.ok_or_else(|| bad(name, &format!("the id of a `{ty}`")))
}

/// A direction: three numbers, east, south and up. `None` for an
/// argument left out.
pub(crate) fn optional_dir(
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<Option<[Real; 3]>, Reason> {
    let wanted = || bad(name, "a list of three numbers: east, south and up");
    match value(call, at) {
        Value::Null => Ok(None),
        Value::List(items) => match &**items {
            [x, y, z] => Ok(Some([
                x.as_number().ok_or_else(wanted)?,
                y.as_number().ok_or_else(wanted)?,
                z.as_number().ok_or_else(wanted)?,
            ])),
            _ => Err(wanted()),
        },
        _ => Err(wanted()),
    }
}
