//! Tests of fields that reach inside the crate: what a tract's fields
//! do when it is loaded after time has passed (02 §16).

use std::sync::Arc;

use clash_core::Dur;
use clash_time::Timeline;

use super::{FieldDef, FieldId};
use crate::cause::{Cause, CauseKind};
use crate::cosmos::Cosmos;
use crate::defs::{BlockState, FloorState, RawWorldDefs, WorldDefs};
use crate::edit::WorldCtx;
use crate::event::{EventSink, WorldEvent};
use crate::pos::{CellPos, GridId, Location, TractPos};
use crate::test_support::load_flat;

/// Reads one definition from JSON. Under plain `serde` a duration is a
/// count of microseconds: units are the loader's.
fn def<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|error| panic!("{json}: {error}"))
}

fn field(id: &str, rules: &str, catch_up: &str) -> (String, FieldDef) {
    let json = format!(r#"{{ "step": 1000000, "rules": [{rules}], "catch_up": "{catch_up}" }}"#);
    (id.to_owned(), def(&json))
}

fn world() -> (Cosmos, Timeline, GridId) {
    let mut raw = RawWorldDefs::default();
    raw.realms.push((
        "t:land".to_owned(),
        def(r#"{ "size_regions": [1, 1], "z_range": [-8, 40],
                 "strata": [ { "from_z": -8, "gravity": 9.81 } ] }"#),
    ));
    let decay = r#"{ "rule": "decay", "amount": 2 }"#;
    raw.fields.push(field("t:ember", decay, "burn_out"));
    raw.fields.push(field("t:fog", decay, "disperse"));
    raw.fields.push(field("t:mark", "", "keep"));
    raw.fields
        .push(field("t:pool", r#"{ "rule": "flow" }"#, "settle"));
    raw.fields.push(field("t:spark", decay, "none"));
    let defs = Arc::new(WorldDefs::build(raw).unwrap_or_else(|errors| panic!("{errors:?}")));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let land = defs
        .realms()
        .lookup("t:land")
        .unwrap_or_else(|| unreachable!());
    let realm = cosmos
        .create_realm(&mut timeline, land, 1)
        .unwrap_or_else(|error| panic!("{error}"));
    let grid = cosmos
        .realm(realm)
        .map(|realm| realm.base_grid)
        .unwrap_or_else(|| unreachable!());
    load_flat(
        &mut cosmos,
        grid,
        [TractPos::new(0, 0)],
        BlockState::NONE,
        FloorState::NONE,
        0,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    (cosmos, timeline, grid)
}

#[test]
fn fields_catch_up_on_the_time_nobody_watched() {
    let (mut cosmos, mut timeline, grid) = world();
    let mut events = EventSink::new();
    let id = |cosmos: &Cosmos, name: &str| -> FieldId {
        cosmos
            .defs()
            .fields()
            .lookup(name)
            .unwrap_or_else(|| panic!("no field {name}"))
    };
    let (ember, fog, mark, pool, spark) = (
        id(&cosmos, "t:ember"),
        id(&cosmos, "t:fog"),
        id(&cosmos, "t:mark"),
        id(&cosmos, "t:pool"),
        id(&cosmos, "t:spark"),
    );
    let at = |x: i32| Location::new(grid, CellPos::new(x, 3, 5));
    let who = clash_core::EntityId::new(4, 0);
    let lit = Cause::new(CauseKind::Action).by(who);
    let mut ctx = WorldCtx::new(&mut cosmos, &mut timeline, &mut events);
    for (x, which, level) in [
        (1, ember, 200),
        (2, fog, 100),
        (3, mark, 7),
        (4, pool, 50),
        (5, spark, 9),
    ] {
        ctx.edit(lit)
            .set_field(at(x), which, level)
            .unwrap_or_else(|error| panic!("{error}"));
    }
    // The pool lies still, as it would have been stored.
    ctx.cosmos.rest_field(at(4), pool);
    ctx.events.clear();
    let tract = TractPos::new(0, 0);

    // Less than a step has passed: only what never outlasts unloading
    // goes.
    ctx.fields_catch_up(grid, tract, Dur::from_millis(900));
    assert_eq!(ctx.events.len(), 1);
    assert_eq!(ctx.cosmos.field(at(1), ember), Ok(200));
    assert_eq!(ctx.cosmos.field(at(2), fog), Ok(100));
    assert_eq!(ctx.cosmos.field(at(5), spark), Ok(0));

    // Twenty seconds have passed, which is twenty steps of each.
    ctx.cosmos.rest_field(at(4), pool);
    ctx.fields_catch_up(grid, tract, Dur::from_secs(20));
    // A fire has burnt out; what does not outlast unloading is gone.
    assert_eq!(ctx.cosmos.field(at(1), ember), Ok(0));
    assert_eq!(ctx.cosmos.field(at(5), spark), Ok(0));
    // Fog lost its decay for each step, and half of the rest once.
    assert_eq!(ctx.cosmos.field(at(2), fog), Ok((100 - 2 * 20) / 2));
    // What keeps is kept; what settles is kept and woken.
    assert_eq!(ctx.cosmos.field(at(3), mark), Ok(7));
    assert_eq!(ctx.cosmos.field(at(4), pool), Ok(50));
    let woken = ctx
        .cosmos
        .field_cell(at(4), pool)
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(woken.is_some_and(|cell| cell.awake));
    // What went is reported with the coarse cause, for whoever was
    // responsible; and what is still active is stepped again.
    let gone: Vec<(Location, FieldId, Cause)> = ctx
        .events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::FieldVanished {
                loc, field, cause, ..
            } => Some((*loc, *field, *cause)),
            _ => None,
        })
        .collect();
    let coarse = Cause {
        actor: None,
        instigator: Some(who),
        kind: CauseKind::Coarse,
    };
    assert_eq!(gone, [(at(5), spark, coarse), (at(1), ember, coarse)]);
    assert_eq!(ctx.cosmos.field_state().steps().count(), 1);
}
