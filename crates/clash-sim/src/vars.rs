//! Variables, and what the `set` and `add` effects write (05 §6, §10.3).
//!
//! Content keeps its own state in variables on subjects. This module is
//! where they are read and written.
//!
//! # Where a variable is kept
//!
//! | Scope | Kind | Kept in | Written through |
//! |-------|------|---------|-----------------|
//! | `entity` | `entity` | The kernel entity's `vars` | [`Sim::set_var`] |
//! | `cell` | `loc` | The cell's sparse variables in its chunk | The world's `WorldEdit`, which emits `cell_var_changed` |
//! | `area` | `area` | The area's record | `Cosmos::set_area_var` |
//! | `site` | `site` | The site's record | `Cosmos::set_site_var` |
//! | `tract` | `tract` | The tract's record, loaded or not | `Cosmos::set_tract_var` |
//! | `realm` | `realm` | The realm's record | `Cosmos::set_realm_var` |
//! | `global` | `global` | The cosmos | `Cosmos::set_global_var` |
//!
//! Every one of them is state that is saved and hashed where it is
//! kept. An element has no variables of its own: they are its cell's.
//!
//! # Reading
//!
//! [`Sim::var`] gives what was set; for a variable that was never set,
//! its declared default (`defs.variables()`) if it is declared for that
//! kind of subject; otherwise null. A subject that no longer exists has
//! no variables: everything reads as null.
//!
//! # Writing
//!
//! [`Sim::set_var`] is the one way a variable changes. Setting null
//! unsets. A declared variable is held to its declaration: it lives on
//! the kind of subject it was declared for and holds a value of its
//! type, where an int serves as a real. An undeclared variable holds
//! anything on anything (the linter warns about those at load, 05 §6).
//! A value nested deeper than [`MAX_VAR_NESTING`] is refused, so that
//! whatever is stored can be saved (01 §11.1).
//!
//! A variable declared with `"watch": true` raises `var_changed` when
//! what it reads as changes, with what it read as before and what it
//! reads as now.
//!
//! # Places
//!
//! What `set` and `add` assign to is a [`Place`]: a variable or a
//! settable property of a subject, optionally followed by keys.
//! [`Sim::assign`] evaluates the subject and the keys and writes;
//! [`Sim::assign_to`] writes where the caller has evaluated them. A
//! property is set through its registered setter, which goes through
//! the engine's own mutation path (01 R3): setting an entity's `loc`
//! relocates it, with the events that brings.

use std::sync::Arc;

use clash_core::VarKey;
use clash_data::Runner;
use clash_expr::{Map, Place, PlaceRoot, PropertyId, Type, Value};
use clash_world::{Cause, WorldError};

use crate::Sim;
use crate::bus::FieldValue;
use crate::event::{EventBody, Subject, Where};
use crate::host::Bound;
use crate::reason::{Reason, codes};

/// How deeply lists and maps may be nested in a value a variable holds.
/// The codec writes at most 128 levels (01 §11.1), and a value sits
/// inside the records that hold it.
pub const MAX_VAR_NESTING: usize = 32;

/// How a place is assigned to (05 §10.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Assign {
    /// `set`: the place takes the value.
    Set,
    /// `add`: a number is added to, and a list is appended to.
    Add,
}

/// Whether a value may be held where a type is declared. An int serves
/// as a real; `any` holds anything; null only where the type says so.
fn fits(ty: &Type, value: &Value) -> bool {
    match (ty, value) {
        (Type::Any, _) => true,
        (Type::Nullable(inner), value) => value.is_null() || fits(inner, value),
        (Type::Null, value) => value.is_null(),
        (Type::Bool, Value::Bool(_))
        | (Type::Int, Value::Int(_))
        | (Type::Real | Type::Number, Value::Int(_) | Value::Real(_))
        | (Type::Str, Value::Str(_)) => true,
        (Type::Id(ty), Value::Id(id)) => id.ty == *ty,
        (Type::Subject(kind), Value::Subject(handle)) => handle.kind == *kind,
        (Type::List(element), Value::List(items)) => items.iter().all(|item| fits(element, item)),
        (Type::Map(element), Value::Map(entries)) => {
            entries.values().all(|entry| fits(element, entry))
        }
        _ => false,
    }
}

/// The key a value stands for in a map: a text as it is, an id as its
/// qualified id (05 §3).
fn map_key(sim: &Sim, key: &Value) -> Option<Arc<str>> {
    match key {
        Value::Str(text) => Some(Arc::clone(text)),
        Value::Id(id) => sim.exprs.registry().def_name(*id).map(Arc::from),
        _ => None,
    }
}

fn bad_key(why: String) -> Reason {
    Reason::new(codes::WRITE_BAD_KEY, why)
}

/// `current` with `value` assigned at the end of a path of keys.
fn assign_at(
    sim: &Sim,
    current: Value,
    keys: &[Value],
    how: Assign,
    value: Value,
) -> Result<Value, Reason> {
    let Some((key, rest)) = keys.split_first() else {
        return match how {
            Assign::Set => Ok(value),
            Assign::Add => added(current, value),
        };
    };
    match current {
        Value::List(items) => {
            let index = key
                .as_int()
                .and_then(|index| usize::try_from(index).ok())
                .filter(|index| *index < items.len())
                .ok_or_else(|| {
                    bad_key(format!(
                        "`{key}` is not the position of an element: the list has {}",
                        items.len()
                    ))
                    .with("length", items.len() as i64)
                })?;
            let mut items: Vec<Value> = items.to_vec();
            let inner = core::mem::take(&mut items[index]);
            items[index] = assign_at(sim, inner, rest, how, value)?;
            Ok(Value::from(items))
        }
        // A map, or nothing yet: a key makes the map.
        Value::Map(_) | Value::Null => {
            let name = map_key(sim, key).ok_or_else(|| {
                bad_key(format!(
                    "a {} cannot be the key of a map entry: a key is a text or an id",
                    key.type_name()
                ))
            })?;
            let mut entries: Map = match current {
                Value::Map(entries) => (*entries).clone(),
                _ => Map::new(),
            };
            let inner = entries.remove(&name).unwrap_or_default();
            let assigned = assign_at(sim, inner, rest, how, value)?;
            if assigned.is_null() {
                // An entry set to null is an entry removed.
                return Ok(Value::from(entries));
            }
            entries.insert(name, assigned);
            Ok(Value::from(entries))
        }
        other => Err(bad_key(format!(
            "a {} has no entries to assign to",
            other.type_name()
        ))),
    }
}

/// What `add` makes of a value: a sum, or a list with one more element.
fn added(current: Value, value: Value) -> Result<Value, Reason> {
    let cannot = |current: &Value, value: &Value| {
        Reason::new(
            codes::WRITE_CANNOT_ADD,
            format!(
                "a {} cannot be added to a {}: `add` adds to numbers and appends to lists",
                value.type_name(),
                current.type_name()
            ),
        )
    };
    match (current, value) {
        (Value::List(items), value) => {
            let mut items: Vec<Value> = items.to_vec();
            items.push(value);
            Ok(Value::from(items))
        }
        (Value::Null, value @ (Value::Int(_) | Value::Real(_))) => Ok(value),
        (Value::Int(a), Value::Int(b)) => a.checked_add(b).map(Value::Int).ok_or_else(|| {
            Reason::new(
                codes::WRITE_CANNOT_ADD,
                "the sum is beyond what an int holds",
            )
        }),
        (a @ (Value::Int(_) | Value::Real(_)), b @ (Value::Int(_) | Value::Real(_))) => {
            match (a.as_number(), b.as_number()) {
                (Some(a), Some(b)) => Ok(Value::Real(a + b)),
                _ => Err(cannot(&a, &b)),
            }
        }
        (current, value) => Err(cannot(&current, &value)),
    }
}

/// The reason for a subject that is not there to be written to.
fn gone(subject: Subject) -> Reason {
    Reason::new(
        codes::SUBJECT_GONE,
        format!(
            "the {} this is to be written to no longer exists",
            subject.kind()
        ),
    )
    .with("kind", subject.kind())
}

impl Sim {
    /// What a variable of a subject holds, if it has been set. Truth.
    fn stored_var(&self, subject: Subject, name: &str) -> Option<&clash_core::Value> {
        match subject {
            Subject::Entity(id) => self.entities.get(id)?.vars.get(name),
            Subject::Loc(loc) | Subject::Element(loc, _) => self.cosmos.var(loc, name).ok()?,
            Subject::Area(id) => self.cosmos.area(id)?.vars.get(name),
            Subject::Site(id) => self.cosmos.site(id)?.vars.get(name),
            Subject::Tract(realm, tract) => self.cosmos.tract_var(realm, tract, name),
            Subject::Realm(id) => self.cosmos.realm(id)?.vars.get(name),
            Subject::Global => self.cosmos.vars().get(name),
        }
    }

    /// The declared default of a variable, where the variable is
    /// declared for the kind of this subject; null otherwise.
    fn default_var(&self, subject: Subject, name: &str) -> Value {
        match self.defs.variables().get(name) {
            Some(decl) if decl.kind == self.exprs.kinds().kind_of(&subject) => decl.default.clone(),
            _ => Value::Null,
        }
    }

    /// A variable of a subject, by qualified name (05 §6): what was
    /// set; for one that was never set, its declared default; otherwise
    /// null. Everything reads as null of a subject that no longer
    /// exists. Truth.
    pub fn var(&self, subject: Subject, name: &str) -> Value {
        if !self.exists(subject) {
            return Value::Null;
        }
        match self.stored_var(subject, name) {
            Some(value) => Value::from(value),
            None => self.default_var(subject, name),
        }
    }

    /// Sets a variable of a subject, or unsets it with null, and returns
    /// what it read as before. **This is the one way a variable
    /// changes.** A cell's variable is written through the world's
    /// `WorldEdit` and emits `cell_var_changed`; a variable declared
    /// with `"watch": true` raises `var_changed` if what it reads as
    /// changed.
    ///
    /// Refused, changing nothing: a subject that no longer exists; an
    /// element, which has no variables of its own; a declared variable
    /// on a subject of another kind than it was declared for, or given
    /// a value that is not of its type; a value nested deeper than
    /// [`MAX_VAR_NESTING`]; and whatever the world refuses, such as a
    /// cell that is not loaded or a tract whose region has not been
    /// surveyed.
    pub fn set_var(
        &mut self,
        subject: Subject,
        name: &str,
        value: Value,
        cause: Cause,
    ) -> Result<Value, Reason> {
        if !self.exists(subject) {
            return Err(gone(subject));
        }
        if matches!(subject, Subject::Element(..)) {
            return Err(Reason::new(
                codes::VARS_WRONG_SCOPE,
                "an element has no variables of its own: they are its cell's",
            )
            .with("variable", name));
        }
        let mut watched = false;
        if let Some(decl) = self.defs.variables().get(name) {
            let registry = self.exprs.registry();
            if decl.kind != self.exprs.kinds().kind_of(&subject) {
                let declared = registry
                    .kind(decl.kind)
                    .map_or("?", |kind| kind.spec.name.as_str());
                return Err(Reason::new(
                    codes::VARS_WRONG_SCOPE,
                    format!(
                        "`{name}` is declared for a {declared} and cannot be set on a {}",
                        subject.kind()
                    ),
                )
                .with("variable", name)
                .with("declared", declared)
                .with("found", subject.kind()));
            }
            if !value.is_null() && !fits(&decl.ty, &value) {
                let wanted = registry.type_name(&decl.ty);
                return Err(Reason::new(
                    codes::VARS_WRONG_TYPE,
                    format!(
                        "`{name}` is declared as {wanted} and cannot hold a {}",
                        value.type_name()
                    ),
                )
                .with("variable", name)
                .with("declared", wanted)
                .with("found", value.type_name()));
            }
            watched = decl.watch;
        }
        let stored = (!value.is_null()).then(|| value.to_stored());
        if stored
            .as_ref()
            .is_some_and(|value| value.nesting() > MAX_VAR_NESTING)
        {
            return Err(Reason::new(
                codes::VARS_TOO_DEEP,
                format!("a variable holds lists and maps nested at most {MAX_VAR_NESTING} deep"),
            )
            .with("variable", name)
            .with("limit", MAX_VAR_NESTING as i64));
        }

        let before = self.var(subject, name);
        let key = VarKey::new(name);
        let world = |error: WorldError| Reason::from_world(&error);
        match subject {
            Subject::Entity(id) => {
                let entity = self.entities.get_mut(id).ok_or_else(|| gone(subject))?;
                match stored {
                    Some(value) => entity.vars.set(key, value),
                    None => entity.vars.remove(name),
                };
            }
            Subject::Loc(loc) => {
                self.world(|ctx| {
                    let mut edit = ctx.edit(cause);
                    match stored {
                        Some(value) => edit.set_var(loc, key, value),
                        None => edit.clear_var(loc, key),
                    }
                })
                .map_err(world)?;
            }
            Subject::Area(id) => {
                self.cosmos.set_area_var(id, key, stored).map_err(world)?;
            }
            Subject::Site(id) => {
                self.cosmos.set_site_var(id, key, stored).map_err(world)?;
            }
            Subject::Tract(realm, tract) => {
                self.cosmos
                    .set_tract_var(realm, tract, key, stored)
                    .map_err(world)?;
            }
            Subject::Realm(id) => {
                self.cosmos.set_realm_var(id, key, stored).map_err(world)?;
            }
            Subject::Global => {
                self.cosmos.set_global_var(key, stored);
            }
            Subject::Element(..) => {}
        }
        // A stat whose expression reads this variable is worked out
        // again (06 §4.1).
        if let Subject::Entity(id) = subject {
            self.life_var_changed(id, name, cause);
        }
        // An expression kept current that read it is stale (05 §7.5).
        crate::ability::reactive::var_written(self, subject, name);
        if watched {
            let after = self.var(subject, name);
            if after != before {
                let on = match subject {
                    Subject::Entity(id) => Where::Entity(id),
                    Subject::Loc(loc) => Where::Loc(loc),
                    _ => Where::Global,
                };
                self.emit(
                    on,
                    cause,
                    EventBody::VarChanged {
                        owner: subject,
                        name: name.to_owned(),
                        old: self.field_of(&before),
                        new: self.field_of(&after),
                    },
                );
            }
        }
        Ok(before)
    }

    /// A value as the field of an event: null, a reference to one of
    /// the simulation's subjects, or the value as a variable stores it.
    pub fn field_of(&self, value: &Value) -> FieldValue {
        match value {
            Value::Null => FieldValue::Null,
            Value::Subject(handle) => match self.exprs.subject(*handle) {
                Some(subject) => FieldValue::Subject(subject),
                None => FieldValue::Plain(value.to_stored()),
            },
            other => FieldValue::Plain(other.to_stored()),
        }
    }

    /// Sets a settable property of a subject, by the id the registry
    /// gave it, through the setter registered for it: the engine's own
    /// mutation path, with the events that brings (01 R3).
    ///
    /// Refused for a property that is not the simulation's or has no
    /// setter, and for whatever the setter refuses.
    pub fn set_property(
        &mut self,
        property: PropertyId,
        subject: Subject,
        value: &Value,
        cause: Cause,
    ) -> Result<(), Reason> {
        let exprs = Arc::clone(&self.exprs);
        let setter = exprs
            .property(property)
            .filter(|def| def.kind == subject.kind())
            .and_then(|def| Some((def.name, def.set?)));
        let Some((name, set)) = setter else {
            return Err(Reason::new(
                codes::WRITE_NOT_SETTABLE,
                format!("a {} has no such property that can be set", subject.kind()),
            )
            .with("kind", subject.kind()));
        };
        set(self, subject, value, cause).map_err(|reason| reason.with("property", name))
    }

    /// Sets a settable property of a subject, by the names rules write.
    /// For tools and tests; an effect names a property by id.
    pub fn set_property_named(
        &mut self,
        subject: Subject,
        name: &str,
        value: &Value,
        cause: Cause,
    ) -> Result<(), Reason> {
        match self.exprs.property_id(subject.kind(), name) {
            Some(property) => self.set_property(property, subject, value, cause),
            None => Err(Reason::new(
                codes::WRITE_NOT_SETTABLE,
                format!("a {} has no property `{name}`", subject.kind()),
            )
            .with("kind", subject.kind())
            .with("property", name)),
        }
    }

    /// Assigns to a variable or a settable property of a subject, where
    /// the caller has already worked out whose it is and which entries
    /// to step into (05 §10.3). With keys, the value is read, changed at
    /// the end of the path and written back whole: a key into nothing
    /// makes a map, and an entry set to null is removed.
    ///
    /// A local and a field of the event are the caller's own to assign:
    /// they are refused here, as is everything [`set_var`](Sim::set_var)
    /// and [`set_property`](Sim::set_property) refuse, a key that names
    /// no element, and an `add` to what is neither a number nor a list.
    pub fn assign_to(
        &mut self,
        subject: Subject,
        root: &PlaceRoot,
        keys: &[Value],
        how: Assign,
        value: Value,
        cause: Cause,
    ) -> Result<(), Reason> {
        match root {
            PlaceRoot::Variable(name) => {
                let value = if keys.is_empty() && how == Assign::Set {
                    value
                } else {
                    assign_at(self, self.var(subject, name), keys, how, value)?
                };
                self.set_var(subject, name, value, cause).map(|_| ())
            }
            PlaceRoot::Property(property) => {
                let value = if keys.is_empty() && how == Assign::Set {
                    value
                } else {
                    let current = match self.exprs.property(*property) {
                        Some(def) => (def.get)(self, subject, &[]),
                        None => Value::Null,
                    };
                    assign_at(self, current, keys, how, value)?
                };
                self.set_property(*property, subject, &value, cause)
            }
            PlaceRoot::Local(_) | PlaceRoot::EventField(_) => Err(Reason::new(
                codes::WRITE_NOT_A_SUBJECT,
                "a local and a field of the event are kept by whoever runs the rule, not by the simulation",
            )),
        }
    }

    /// Assigns to a compiled place (05 §10.3): evaluates the subject it
    /// lives on and its keys against what is bound, reading only, and
    /// writes as [`assign_to`](Sim::assign_to) does.
    ///
    /// Also refused: a place whose subject comes to null or to
    /// something that is not one of the simulation's subjects.
    pub fn assign(
        &mut self,
        place: &Place,
        how: Assign,
        value: Value,
        bound: &Bound,
        cause: Cause,
    ) -> Result<(), Reason> {
        let Some(code) = place.subject() else {
            return self.assign_to(Subject::Global, place.root(), &[], how, value, cause);
        };
        let (owner, keys) = {
            let mut host = self.host(bound.clone());
            let mut runner = Runner::new();
            let mut eval = runner.on(&self.defs, &mut host);
            let owner = eval.value(code);
            let keys: Vec<Value> = place.keys().iter().map(|key| eval.value(key)).collect();
            (owner, keys)
        };
        let subject = self.subject_of(&owner).ok_or_else(|| {
            Reason::new(
                codes::SUBJECT_GONE,
                format!(
                    "what this is to be written to came to {}, which is not a subject",
                    owner.type_name()
                ),
            )
        })?;
        self.assign_to(subject, place.root(), &keys, how, value, cause)
    }
}
