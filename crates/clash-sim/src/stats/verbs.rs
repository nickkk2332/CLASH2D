//! The six verbs of stats (06 §4.4), as the engine's verb table lists
//! them: each reads its arguments and calls the function of
//! [`bind`](super::bind) that the engine itself uses.
//!
//! | Verb | Arguments | Gives |
//! |------|-----------|-------|
//! | `set_attr_base` | `who`, `attr`, `base?` | null |
//! | `add_modifier` | `who`, `attr`, `key`, `add?`, `mul?` | null |
//! | `remove_modifier` | `who`, `key`, `attr?` | null |
//! | `pool_add` | `who`, `pool`, `amount` | What was added, a real |
//! | `pool_rate` | `who`, `pool`, `key`, `rate?` | null |
//! | `advance_track` | `who`, `track`, `amount` | null |
//!
//! A `key` names a source of the kind `rule`: whatever is added under a
//! key is removed by the same key, whichever rule asks
//! ([`key_source`]).

use clash_core::Real;
use clash_data::Def;
use clash_expr::{Registry, Type, Value, VerbSpec};

use super::{
    AttributeDef, AttributeId, Modifier, PoolDef, PoolId, Source, SourceKind, TrackDef, TrackId,
};
use crate::life::args;
use crate::reason::Reason;
use crate::verb::VerbDef;

/// The source a verb's `key` stands for: the kind `rule`, numbered by
/// the 64-bit FNV-1a hash of the key's bytes, so that a key always
/// names the same source.
pub fn key_source(key: &str) -> Source {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    Source::new(SourceKind::Rule, hash)
}

/// The id of a definition of a stat, from its dense number.
fn stat_id<I>(index: usize, from_index: fn(usize) -> Option<I>, ty: &str) -> Result<I, Reason> {
    from_index(index).ok_or_else(|| {
        Reason::new(
            args::BAD_ARGUMENT,
            format!("there is no `{ty}` numbered {index}"),
        )
        .with("argument", ty)
    })
}

fn who(registry: &Registry, name: &str) -> Option<VerbSpec> {
    let entity = registry.kind_id("entity")?;
    Some(VerbSpec::new(name).arg("who", Type::Subject(entity), "Whose stats."))
}

/// The verbs of 06 §4.4, for the engine's verb table.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "set_attr_base",
            build: |registry| {
                let attribute = registry.def_type_id(AttributeDef::TYPE)?;
                Some(
                    who(registry, "set_attr_base")?
                        .arg("attr", Type::Id(attribute), "Which attribute.")
                        .optional_arg(
                            "base",
                            Type::Number,
                            "The base of its own; left out, the definition's is given back.",
                        )
                        .doc("Gives an entity a base of its own for an attribute, in place of the definition's (06 §4.4)."),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                let attr = args::def(sim, call, 1, "attr", AttributeDef::TYPE)?;
                let attr = stat_id(attr, AttributeId::from_index, AttributeDef::TYPE)?;
                let base = args::optional_real(call, 2, "base")?;
                sim.set_attr_base(entity, attr, base, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "add_modifier",
            build: |registry| {
                let attribute = registry.def_type_id(AttributeDef::TYPE)?;
                Some(
                    who(registry, "add_modifier")?
                        .arg("attr", Type::Id(attribute), "Which attribute.")
                        .arg(
                            "key",
                            Type::Str,
                            "The name it is added under and removed by.",
                        )
                        .optional_arg("add", Type::Number, "Added to the base. Left out, nothing.")
                        .optional_arg(
                            "mul",
                            Type::Number,
                            "Multiplied in after every addition. Left out, one.",
                        )
                        .doc("Adds a modifier to an attribute under a key (06 §4.4)."),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                let attr = args::def(sim, call, 1, "attr", AttributeDef::TYPE)?;
                let attr = stat_id(attr, AttributeId::from_index, AttributeDef::TYPE)?;
                let source = key_source(args::text(call, 2, "key")?);
                let modifier = Modifier {
                    attr,
                    source,
                    add: args::optional_real(call, 3, "add")?.unwrap_or(Real::ZERO),
                    mul: args::optional_real(call, 4, "mul")?.unwrap_or(Real::ONE),
                };
                sim.add_modifier(entity, modifier, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "remove_modifier",
            build: |registry| {
                let attribute = registry.def_type_id(AttributeDef::TYPE)?;
                Some(
                    who(registry, "remove_modifier")?
                        .arg("key", Type::Str, "The key the modifiers were added under.")
                        .optional_arg(
                            "attr",
                            Type::Id(attribute),
                            "The one attribute to take them from. Left out, every attribute.",
                        )
                        .doc("Removes what was added under a key, to one attribute or to all (06 §4.4)."),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                let source = key_source(args::text(call, 1, "key")?);
                let attr = match args::optional_def(sim, call, 2, "attr", AttributeDef::TYPE)? {
                    Some(attr) => Some(stat_id(attr, AttributeId::from_index, AttributeDef::TYPE)?),
                    None => None,
                };
                sim.remove_modifier(entity, source, attr, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "pool_add",
            build: |registry| {
                let pool = registry.def_type_id(PoolDef::TYPE)?;
                Some(
                    who(registry, "pool_add")?
                        .arg("pool", Type::Id(pool), "Which pool.")
                        .arg("amount", Type::Number, "What to add; negative to drain.")
                        .doc("Adds to a pool, negative to drain, after the hook `pool_changing`. Gives what was in fact added: less where the pool's bounds stopped it (06 §4.4)."),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                let pool = args::def(sim, call, 1, "pool", PoolDef::TYPE)?;
                let pool = stat_id(pool, PoolId::from_index, PoolDef::TYPE)?;
                let amount = args::real(call, 2, "amount")?;
                sim.pool_add(entity, pool, amount, cause).map(Value::Real)
            },
        },
        VerbDef {
            name: "pool_rate",
            build: |registry| {
                let pool = registry.def_type_id(PoolDef::TYPE)?;
                Some(
                    who(registry, "pool_rate")?
                        .arg("pool", Type::Id(pool), "Which pool.")
                        .arg("key", Type::Str, "The name the rate is applied under.")
                        .optional_arg(
                            "rate",
                            Type::Number,
                            "What the pool gains each second of its owner's time; negative to drain. Left out, the rate under this key is removed.",
                        )
                        .doc("Applies a rate to a pool under a key, replacing the one that key had applied (06 §4.4)."),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                let pool = args::def(sim, call, 1, "pool", PoolDef::TYPE)?;
                let pool = stat_id(pool, PoolId::from_index, PoolDef::TYPE)?;
                let source = key_source(args::text(call, 2, "key")?);
                let rate = args::optional_real(call, 3, "rate")?;
                sim.pool_rate(entity, pool, source, rate, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "advance_track",
            build: |registry| {
                let track = registry.def_type_id(TrackDef::TYPE)?;
                Some(
                    who(registry, "advance_track")?
                        .arg("track", Type::Id(track), "Which track.")
                        .arg("amount", Type::Number, "What to add to its counter: not negative.")
                        .doc("Adds to a track's counter; `track_advanced` is raised for each level reached (06 §4.4)."),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                let track = args::def(sim, call, 1, "track", TrackDef::TYPE)?;
                let track = stat_id(track, TrackId::from_index, TrackDef::TYPE)?;
                let amount = args::real(call, 2, "amount")?;
                sim.advance_track(entity, track, amount, cause)?;
                Ok(Value::Null)
            },
        },
    ]
}
