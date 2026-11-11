//! Tests for W10, persistence (02 §11): saving and loading a whole
//! world, the pristine policy and the generator fingerprint, the entity
//! section, what a change of mods does to a save, damaged data, and the
//! store on disk.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use clash_core::{Dur, EntityId, Hash128, HashDomain, Real, Tick, Value, VarKey, codec};
use clash_time::{Timeline, WakeDomain};
use clash_world::{
    Activity, AreaShape, AreaSpec, BlockState, Cause, CauseKind, CellBox, CellEdit, CellPos,
    ColumnBuilder, Cosmos, DamagePacket, DecalId, DefKind, EngineFlag, EventSink, FloorState,
    GenCtx, GenError, GeneratorRegistry, GeneratorRequest, Generators, GridId, Layer, LoadError,
    Location, MemoryStore, PendingEdit, RealizedTract, RealmId, RealmInfo, RedbStore, RegionPos,
    RegionRecord, RegionSurvey, RegionSurveyResult, RegionSurveyor, SaveError, SiteDefId, SiteSpec,
    Ticket, TicketAnchor, TicketReason, TractIo, TractPos, TractRealizer, TractRecord,
    TractRecords3x3, TractState, TractStore, WarnAt, WorldCtx, WorldDefs, WorldEvent,
    WorldGenerator, decode_tract,
};
use proptest::prelude::*;

// ---- A generator, two mods and a world to save -------------------------

/// A generator for these tests: flat ground under a floor, and in each
/// tract one wall, where the positional hash of the tract puts it.
/// Everything it builds from is named by the realm's parameters.
#[derive(Debug)]
struct Plots {
    ground: BlockState,
    floor: FloorState,
    wall: BlockState,
}

impl RegionSurveyor for Plots {
    fn survey(&self, _: &GenCtx<'_>, _: &RealmInfo, _: RegionPos) -> RegionSurveyResult {
        RegionSurveyResult::new(RegionRecord::new(RegionSurvey::default()))
    }
}

impl TractRealizer for Plots {
    fn realize(
        &self,
        cx: &GenCtx<'_>,
        realm: &RealmInfo,
        pos: TractPos,
        _: &TractRecord,
        _: &TractRecords3x3,
    ) -> RealizedTract {
        let mut builder = ColumnBuilder::new(cx.defs(), self.ground, BlockState::NONE, 0);
        builder.fill_floor(0, self.floor);
        let row = cx.hash(realm, HashDomain::new("plots"), pos.origin(0), 1) % 24;
        let row = i32::try_from(row).unwrap() + 4;
        for x in 4..28 {
            builder.set_block(CellPos::new(x, row, 0), self.wall);
        }
        RealizedTract::new(builder.build())
    }
}

fn plots(request: &GeneratorRequest<'_>) -> Result<Arc<dyn WorldGenerator>, GenError> {
    request.only(&["floor", "wall"])?;
    Ok(Arc::new(Plots {
        ground: request.kind.below_fill,
        floor: request.floor("floor")?,
        wall: request.block("wall")?,
    }))
}

fn generators(defs: &Arc<WorldDefs>) -> Generators {
    let mut registry = GeneratorRegistry::new();
    registry.register("plots", plots);
    registry.bind(defs)
}

/// The mod `t`: a realm for the `plots` generator, and a kind of area.
/// `wall` is the block its tracts' walls are made of.
fn base_mod(wall: &str) -> String {
    format!(
        r#"[
        {{ "type": "realm", "id": "plots",
          "size_regions": [1, 1], "wrap": [], "z_range": [-16, 60],
          "generator": "plots",
          "generator_params": {{ "floor": "core:grass", "wall": "{wall}" }},
          "fill": {{ "below": "core:solid@core:granite", "above": null }},
          "strata": [ {{ "from_z": -16, "gravity": 9.81, "atmosphere": "core:air" }} ] }},
        {{ "type": "area_kind", "id": "yard", "tags": ["fenced"], "priority": 2 }}
    ]"#
    )
}

/// The mod `x`, which a save can come to be without: one definition of
/// every kind a save names.
const EXTRA_MOD: &str = r#"[
    { "type": "material", "id": "alloy", "density": 4000, "toughness": 2.0e6,
      "span": 6, "rubble": "core:rubble" },
    { "type": "block", "id": "gate", "material": "alloy", "height": 3.0, "fill": 0.3,
      "flags": ["opaque", "load_bearing"],
      "states": { "closed": {}, "open": { "add_flags": ["passable"], "remove_flags": ["opaque"] } } },
    { "type": "floor", "id": "planks", "material": "alloy", "thickness": 0.1 },
    { "type": "fixture", "id": "crate", "material": "alloy", "height": 1.0, "fill": 0.2 },
    { "type": "area_kind", "id": "vault", "tags": ["locked"], "priority": 5 },
    { "type": "realm", "id": "pocket",
      "size_regions": [1, 1], "wrap": [], "z_range": [-8, 20],
      "generator": "plots",
      "generator_params": { "floor": "core:slab", "wall": "gate" },
      "fill": { "below": "core:solid@core:granite", "above": null },
      "strata": [ { "from_z": -8, "gravity": 3.0, "atmosphere": null } ] }
]"#;

/// The world's definitions out of the loader: `mods/core`, the mod `t`
/// and, if asked for, the mod `x`.
fn defs_of(wall: &str, with_extra: bool) -> Arc<WorldDefs> {
    let types = common::world_types();
    let base = base_mod(wall);
    let requires_core = |id: &str| format!(r#"{{ "id": "{id}", "requires": ["core"] }}"#);
    let mut loader =
        common::loader(&types).memory_mod("t", &requires_core("t"), &[("data/t.json", &base)]);
    let mut mods = vec!["core", "t"];
    if with_extra {
        loader = loader.memory_mod("x", &requires_core("x"), &[("data/x.json", EXTRA_MOD)]);
        mods.push("x");
    }
    let loaded = match loader.load_mods(&mods).into_result() {
        Ok(loaded) => loaded,
        Err(diagnostics) => panic!("the test mods do not load:\n{}", diagnostics.render()),
    };
    match WorldDefs::from_defs(&loaded) {
        Ok(defs) => defs,
        Err(diagnostics) => panic!("the test mods do not build:\n{}", diagnostics.render()),
    }
}

fn base_defs() -> Arc<WorldDefs> {
    defs_of("core:wall", false)
}

/// A world with its timeline, the way the simulation holds them.
struct Session {
    defs: Arc<WorldDefs>,
    generators: Generators,
    timeline: Timeline,
    cosmos: Cosmos,
    events: EventSink<WorldEvent>,
    realm: RealmId,
    grid: GridId,
}

const TIMELINE_SECTION: &str = "sim.timeline";

impl Session {
    /// A new world over `defs` with one realm of `t:plots`.
    fn new(defs: Arc<WorldDefs>) -> Session {
        let mut timeline = Timeline::new();
        let mut cosmos = Cosmos::new(Arc::clone(&defs));
        let def = defs.realms().lookup("t:plots").unwrap();
        let realm = cosmos.create_realm(&mut timeline, def, 9).unwrap();
        let grid = cosmos.realm(realm).unwrap().base_grid;
        Session {
            generators: generators(&defs),
            defs,
            timeline,
            cosmos,
            events: EventSink::new(),
            realm,
            grid,
        }
    }

    /// Saves the way the simulation does: the world, then its own
    /// sections, then one commit.
    fn save(&mut self, store: &mut dyn TractStore) {
        self.cosmos.save_world(store).unwrap();
        store.put_section(TIMELINE_SECTION, &codec::to_bytes(&self.timeline).unwrap());
        store.commit().unwrap();
    }

    /// Loads what [`save`](Session::save) wrote, over `defs`.
    fn load(defs: &Arc<WorldDefs>, store: &mut dyn TractStore) -> Result<Session, SaveError> {
        let generators = generators(defs);
        let cosmos = Cosmos::load_world(defs, &mut TractIo::new(&generators, store))?;
        let saved = store.section(TIMELINE_SECTION).unwrap().unwrap();
        let timeline: Timeline = codec::from_bytes(&saved).unwrap();
        let realm = RealmId::new(0);
        let grid = cosmos.realm(realm).unwrap().base_grid;
        Ok(Session {
            defs: Arc::clone(cosmos.defs()),
            generators,
            timeline,
            cosmos,
            events: EventSink::new(),
            realm,
            grid,
        })
    }

    fn ctx(&mut self) -> WorldCtx<'_> {
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
    }

    fn loc(&self, tract: (i32, i32), x: i32, y: i32, z: i32) -> Location {
        let origin = TractPos::new(tract.0, tract.1).origin(0);
        Location::new(self.grid, CellPos::new(origin.x + x, origin.y + y, z))
    }

    fn load_tract(
        &mut self,
        store: &mut dyn TractStore,
        tract: (i32, i32),
    ) -> clash_world::LoadedTract {
        let (generators, realm) = (self.generators.clone(), self.realm);
        let mut io = TractIo::new(&generators, store);
        self.ctx()
            .load_tract(&mut io, realm, TractPos::new(tract.0, tract.1))
            .unwrap()
    }

    fn unload_tract(&mut self, store: &mut dyn TractStore, tract: (i32, i32)) {
        let (generators, realm) = (self.generators.clone(), self.realm);
        let mut io = TractIo::new(&generators, store);
        self.ctx()
            .unload_tract(&mut io, realm, TractPos::new(tract.0, tract.1))
            .unwrap();
    }

    fn reconcile(&mut self, store: &mut dyn TractStore) -> clash_world::Reconciled {
        let generators = self.generators.clone();
        let mut io = TractIo::new(&generators, store);
        self.ctx().reconcile_activity(&mut io)
    }

    fn set_block(&mut self, at: Location, name: &str) {
        let state = self.defs.block_state(name, None).unwrap();
        self.ctx().edit(Cause::DEBUG).set_block(at, state).unwrap();
    }

    fn set_floor(&mut self, at: Location, name: &str) {
        let state = self.defs.floor_state(name, None).unwrap();
        self.ctx().edit(Cause::DEBUG).set_floor(at, state).unwrap();
    }

    /// Runs every wake-up due up to a tick and moves time on to it.
    fn run_until(&mut self, until: Tick) {
        while let Some((_, wake)) = self.timeline.pop(until) {
            if wake.domain == WakeDomain::World {
                self.ctx().handle_wake(&wake).unwrap();
            }
        }
        self.timeline.advance_to(until);
    }

    fn run_out(&mut self) {
        while let Some(due) = self.timeline.peek() {
            self.run_until(due);
        }
    }

    fn state_hash(&self) -> (Hash128, Hash128) {
        (Hash128::of(&self.cosmos), Hash128::of(&self.timeline))
    }
}

/// Lays a strip of slab in the air over a tract and asks for a check of
/// it, so that the strip comes down as falling masses.
fn hang_a_strip(session: &mut Session, tract: (i32, i32), y: i32, z: i32) {
    for x in 10..16 {
        let at = session.loc(tract, x, y, z);
        session.set_floor(at, "core:slab");
    }
    let at = session.loc(tract, 12, y, z);
    session.ctx().check_stability(at, Cause::DEBUG).unwrap();
}

/// A world that holds something of everything that is saved: loaded
/// tracts that are pristine and that have changed, unloaded tracts of
/// both sorts, a tract with changes waiting, tickets, areas with a clock
/// and a kind, a site, a stability check waiting, and a collapse in
/// progress.
fn busy_world(defs: Arc<WorldDefs>, store: &mut dyn TractStore) -> Session {
    let mut s = Session::new(defs);
    // A ticket keeps nine tracts loaded.
    let ticket = Ticket::new(
        s.realm,
        TicketAnchor::Fixed(TractPos::new(2, 2)),
        1,
        Activity::Warm,
        TicketReason::Debug,
    );
    s.cosmos.hold_ticket(ticket).unwrap();
    let loaded = s.reconcile(store).loaded.len();
    assert_eq!(loaded, 9);

    // One of them changes: cells, damage, a mark and a variable.
    let at = s.loc((2, 2), 3, 3, 0);
    s.set_block(at, "core:wall@core:brick");
    s.set_block(s.loc((2, 2), 4, 3, 0), "core:door#open");
    {
        let mut ctx = s.ctx();
        let mut edit = ctx.edit(Cause::new(CauseKind::Action).by(EntityId::new(5, 0)));
        edit.set_damage(at, Layer::Block, 99).unwrap();
        edit.add_decal(at, DecalId::new(1), 4).unwrap();
        edit.set_var(at, VarKey::new("t:mark"), Value::from(7))
            .unwrap();
    }

    // Tracts beyond the ticket: one changed and unloaded, one unloaded
    // as it was made, one never loaded with a change waiting for it.
    s.load_tract(store, (6, 6));
    s.set_block(s.loc((6, 6), 9, 9, 0), "core:pillar");
    s.unload_tract(store, (6, 6));
    s.load_tract(store, (7, 7));
    s.unload_tract(store, (7, 7));
    let wall = s.defs.block_state("core:wall", None).unwrap();
    let waiting = PendingEdit::Cells(vec![CellEdit {
        cell: TractPos::new(6, 6).origin(0),
        layer: Layer::Block,
        state: wall.raw(),
    }]);
    s.cosmos
        .add_pending_edit(s.realm, TractPos::new(6, 6), waiting)
        .unwrap();

    // Areas: one of a kind with a clock of its own, one plain.
    let yard = s.defs.area_kinds().lookup("t:yard").unwrap();
    let corner = s.loc((2, 2), 0, 0, 0).cell;
    let mut spec = AreaSpec::new(
        s.grid,
        AreaShape::Box(CellBox::new(
            corner,
            CellPos::new(corner.x + 5, corner.y + 5, 2),
        )),
    );
    spec.kind = Some(yard);
    spec.clock_rate = Some(Real::of(0.5));
    spec.tags = vec!["quiet".to_owned()];
    s.ctx().create_area(spec, Cause::DEBUG).unwrap();
    let bubble = AreaSpec::new(
        s.grid,
        AreaShape::Radius {
            centre: s.loc((3, 3), 16, 16, 1).cell,
            radius: Real::of(4.0),
        },
    );
    s.ctx().create_area(bubble, Cause::DEBUG).unwrap();

    // A site.
    s.cosmos
        .create_site(SiteSpec {
            def: SiteDefId::new(0),
            name: "The Plot".to_owned(),
            realm: s.realm,
            tracts: vec![TractPos::new(2, 2), TractPos::new(6, 6)],
            anchor: corner,
            vars: Default::default(),
            tags: vec!["known".to_owned()],
        })
        .unwrap();

    // A collapse in progress: a strip of floor five storeys up, which
    // takes more than a second to come down.
    hang_a_strip(&mut s, (1, 1), 20, 5);
    s.run_until(Tick(400_000));
    assert!(s.cosmos.falling_masses().count() > 0, "nothing is falling");
    assert!(s.cosmos.collapses().count() > 0);
    // And a check that has been asked for and not made yet.
    hang_a_strip(&mut s, (3, 1), 20, 2);
    assert!(!s.cosmos.stability_requests().is_empty());
    s
}

/// What both worlds do after a save, to show that they go on alike.
fn carry_on(s: &mut Session, store: &mut dyn TractStore) -> (Hash128, Hash128, Hash128) {
    s.run_until(Tick(900_000));
    s.set_block(s.loc((2, 2), 6, 6, 0), "core:wall");
    // The ticket goes: everything it kept is unloaded, and what changed
    // is written.
    let ticket = s.cosmos.tickets().iter().next().map(|(id, _)| id).unwrap();
    s.cosmos.release_ticket(ticket);
    s.run_out();
    let gone = s.reconcile(store);
    assert!(gone.failed.is_empty(), "{:?}", gone.failed);
    // And a tract comes back with the change that was waiting for it.
    s.load_tract(store, (6, 6));
    let (cosmos, timeline) = s.state_hash();
    (cosmos, timeline, Hash128::of(&s.events.drain()))
}

// ---- Saving and loading the whole world --------------------------------

#[test]
fn a_whole_cosmos_hashes_equal_after_save_and_load() {
    let defs = base_defs();
    let mut store = MemoryStore::new();
    let mut first = busy_world(Arc::clone(&defs), &mut store);
    let before = first.state_hash();
    first.save(&mut store);
    // Saving changes nothing that is hashed.
    assert_eq!(first.state_hash(), before);
    assert_eq!(store.commits(), 1);

    let mut saved = store.clone();
    let mut second = Session::load(&defs, &mut saved).unwrap();
    assert_eq!(second.state_hash(), before);
    assert!(second.cosmos.take_save_warnings().is_empty());
    // Loading reads and changes nothing in the store.
    assert_eq!(saved, store);

    // What came back is the same world, not only the same hash.
    assert_eq!(
        second.cosmos.loaded_tracts(second.realm),
        first.cosmos.loaded_tracts(first.realm)
    );
    assert_eq!(second.cosmos.tickets(), first.cosmos.tickets());
    assert_eq!(second.cosmos.processes(), first.cosmos.processes());
    assert_eq!(second.cosmos.save_ids(), first.cosmos.save_ids());
    let at = first.loc((2, 2), 3, 3, 0);
    assert_eq!(
        second.cosmos.cell(at).unwrap(),
        first.cosmos.cell(at).unwrap()
    );
    assert_eq!(second.cosmos.damage(at).unwrap(), [0, 99, 0]);
    assert_eq!(
        second.cosmos.decals(at).unwrap(),
        first.cosmos.decals(at).unwrap()
    );
    assert_eq!(
        second.cosmos.var(at, "t:mark").unwrap(),
        Some(&Value::from(7))
    );
    assert_eq!(second.cosmos.areas_at(at).unwrap().len(), 1);
    assert_eq!(
        second.cosmos.activity(second.realm, TractPos::new(2, 2)),
        Some(Activity::Warm)
    );
    for tract in first.cosmos.loaded_tracts(first.realm) {
        let column = |s: &Session| {
            s.cosmos
                .grid(s.grid)
                .unwrap()
                .column(tract)
                .unwrap()
                .is_dirty()
        };
        assert_eq!(column(&second), column(&first), "{tract}");
    }

    // Saving what was loaded writes the same world again.
    let mut again = saved.clone();
    second.save(&mut again);
    let names = store.section_names().unwrap();
    for name in names.iter().filter(|name| name.starts_with("world.")) {
        let body = |store: &MemoryStore| {
            let blob = store.section(name).unwrap().unwrap();
            let header = codec::read_blob_header(&blob).unwrap();
            codec::open_blob(header.kind, &blob).unwrap()
        };
        assert_eq!(body(&again), body(&store), "{name}");
    }

    // And the two go on alike (01 R7): the same state, and the same
    // events from here on.
    first.events.drain();
    let straight = carry_on(&mut first, &mut store);
    let resumed = carry_on(&mut second, &mut saved);
    assert_eq!(resumed, straight);
    assert_eq!(saved, store);
}

#[test]
fn only_what_changed_is_written() {
    let defs = base_defs();
    let mut store = MemoryStore::new();
    let mut s = busy_world(Arc::clone(&defs), &mut store);
    // Before any save: one blob, of the tract that changed and was
    // unloaded. The pristine tract that was unloaded wrote none.
    assert_eq!(
        store.tracts().collect::<Vec<_>>(),
        [(s.realm, TractPos::new(6, 6))]
    );
    let report = s.cosmos.save_world(&mut store).unwrap();
    // Nine tracts are loaded; three have changed: the one that was
    // edited and the two the strips were hung over.
    assert_eq!(report.loaded_tracts, 9);
    assert_eq!(report.tract_blobs, 3);
    assert_eq!(store.len(), 4);
    assert_eq!(report.sections, 8);
    assert_eq!(
        store.section_names().unwrap(),
        [
            "world.activity",
            "world.areas",
            "world.cosmos",
            "world.ids",
            "world.sites",
            "world.stability",
            "world.tickets",
            "world.tracts"
        ]
    );
    // A section of the world that this world does not write is from
    // another world and goes; the simulation's own stay.
    store.put_section("world.left_over", b"old");
    store.put_section("sim.entities", b"kept");
    s.cosmos.save_world(&mut store).unwrap();
    assert_eq!(store.section("world.left_over"), Ok(None));
    assert_eq!(store.section("sim.entities"), Ok(Some(b"kept".to_vec())));
    // Storing everything writes the six pristine tracts too.
    s.cosmos.set_store_pristine(true);
    let report = s.cosmos.save_world(&mut store).unwrap();
    assert_eq!(report.tract_blobs, 9);
    assert_eq!(store.len(), 10);
    s.save(&mut store);
    let mut back = Session::load(&defs, &mut store).unwrap();
    assert!(back.cosmos.store_pristine());
    assert_eq!(back.state_hash(), s.state_hash());
    assert!(back.cosmos.take_save_warnings().is_empty());
}

#[test]
fn a_save_that_is_not_all_there_is_refused() {
    let defs = base_defs();
    let mut store = MemoryStore::new();
    // An empty store holds no world.
    assert!(matches!(
        Session::load(&defs, &mut store).err(),
        Some(SaveError::MissingSection("world.ids"))
    ));
    let mut s = busy_world(Arc::clone(&defs), &mut store);
    s.save(&mut store);
    for name in store.section_names().unwrap() {
        if !name.starts_with("world.") {
            continue;
        }
        let mut without = store.clone();
        without.delete_section(&name);
        match Session::load(&defs, &mut without).err() {
            Some(SaveError::MissingSection(missing)) => assert_eq!(missing, name),
            other => panic!("without {name}: {other:?}"),
        }
    }
    // A section of the world this build does not know would be dropped.
    let mut newer = store.clone();
    newer.put_section("world.bodies", b"?");
    assert_eq!(
        Session::load(&defs, &mut newer).err(),
        Some(SaveError::UnknownSection("world.bodies".to_owned()))
    );
    // A loaded tract that had changed, and whose cells have gone.
    let mut without = store.clone();
    without.delete(s.realm, TractPos::new(2, 2));
    assert_eq!(
        Session::load(&defs, &mut without).err(),
        Some(SaveError::MissingTract {
            realm: s.realm,
            tract: TractPos::new(2, 2)
        })
    );
    // A section of another layout.
    let mut other = store.clone();
    let blob = other.section("world.areas").unwrap().unwrap();
    let (version, body) = codec::open_blob(*b"WARE", &blob).unwrap();
    let later = codec::seal_blob(*b"WARE", version + 1, &body).unwrap();
    other.put_section("world.areas", &later);
    assert!(matches!(
        Session::load(&defs, &mut other).err(),
        Some(SaveError::Section { name, .. }) if name == "world.areas"
    ));
    // No generator for a pristine tract that was loaded.
    let none = Generators::none(&defs);
    let failed = Cosmos::load_world(&defs, &mut TractIo::new(&none, &mut store.clone())).err();
    assert!(
        matches!(&failed, Some(SaveError::Tract { error, .. }) if matches!(**error, LoadError::Gen(_))),
        "{failed:?}"
    );
}

// ---- The pristine policy, the fingerprint and the entity section -------

#[test]
fn a_pristine_tract_writes_no_blob() {
    let defs = base_defs();
    let mut store = MemoryStore::new();
    let mut s = Session::new(Arc::clone(&defs));
    let tract = TractPos::new(4, 4);
    let first = s.load_tract(&mut store, (4, 4));
    assert!(first.first && !first.from_store && !first.regenerated);
    let made = Hash128::of(s.cosmos.grid(s.grid).unwrap().column(tract).unwrap());
    let fingerprint = s.cosmos.generator_fingerprint(s.realm).unwrap();
    let record = s.cosmos.tract_record(s.realm, tract).unwrap();
    assert_eq!(record.state, TractState::Realized { pristine: true });
    assert_eq!(record.extra.as_ref().unwrap().generated, Some(fingerprint));

    s.unload_tract(&mut store, (4, 4));
    assert!(store.is_empty(), "a pristine tract was stored");
    // Saved, too, it writes nothing: loaded or not.
    s.load_tract(&mut store, (4, 4));
    let report = s.cosmos.save_world(&mut store).unwrap();
    assert_eq!((report.loaded_tracts, report.tract_blobs), (1, 0));
    assert!(store.is_empty());
    s.unload_tract(&mut store, (4, 4));

    // It is generated again, the same.
    let again = s.load_tract(&mut store, (4, 4));
    assert!(!again.first && !again.from_store && !again.regenerated);
    assert_eq!(
        Hash128::of(s.cosmos.grid(s.grid).unwrap().column(tract).unwrap()),
        made
    );

    // With everything stored, it is written and read back, and stays
    // pristine: turning the setting off again leaves it readable.
    s.cosmos.set_store_pristine(true);
    s.unload_tract(&mut store, (4, 4));
    assert_eq!(store.len(), 1);
    let record = s.cosmos.tract_record(s.realm, tract).unwrap();
    assert_eq!(record.state, TractState::Realized { pristine: true });
    s.cosmos.set_store_pristine(false);
    let read = s.load_tract(&mut store, (4, 4));
    assert!(read.from_store && !read.regenerated);
    assert_eq!(
        Hash128::of(s.cosmos.grid(s.grid).unwrap().column(tract).unwrap()),
        made
    );

    // Once it changes it is stored like any other.
    s.set_block(s.loc((4, 4), 1, 1, 0), "core:pillar");
    s.unload_tract(&mut store, (4, 4));
    let record = s.cosmos.tract_record(s.realm, tract).unwrap();
    assert_eq!(record.state, TractState::Realized { pristine: false });
    let stored = store.get(s.realm, tract).unwrap().unwrap();
    let held = decode_tract(&s.defs, s.cosmos.save_ids(), &stored).unwrap();
    assert!(held.cells.is_some() && held.entities.is_empty());
}

#[test]
fn a_pristine_tract_follows_its_generators_inputs() {
    let mut store = MemoryStore::new();
    let defs = base_defs();
    let mut s = Session::new(Arc::clone(&defs));
    s.load_tract(&mut store, (4, 4));
    s.load_tract(&mut store, (5, 4));
    s.set_block(s.loc((5, 4), 1, 1, 0), "core:pillar");
    let old = s.cosmos.generator_fingerprint(s.realm).unwrap();
    let wall_row = |s: &Session, tract: (i32, i32), block: BlockState| {
        (0..32).any(|y| s.cosmos.cell(s.loc(tract, 10, y, 0)).unwrap().block == block)
    };
    let wall = defs.block_state("core:wall", None).unwrap();
    assert!(wall_row(&s, (4, 4), wall));
    s.unload_tract(&mut store, (4, 4));
    s.unload_tract(&mut store, (5, 4));
    s.save(&mut store);

    // The same save under mods that build the walls of something else.
    let changed = defs_of("core:wall@core:brick", false);
    let mut s = Session::load(&changed, &mut store).unwrap();
    let new = s.cosmos.generator_fingerprint(s.realm).unwrap();
    assert_ne!(new, old);
    let brick = changed.block_state("core:wall@core:brick", None).unwrap();
    // The pristine tract is generated with the new content, and says
    // so; the record takes the new fingerprint.
    let loaded = s.load_tract(&mut store, (4, 4));
    assert!(loaded.regenerated && !loaded.first && !loaded.from_store);
    assert!(wall_row(&s, (4, 4), brick) && !wall_row(&s, (4, 4), wall));
    let record = s.cosmos.tract_record(s.realm, TractPos::new(4, 4)).unwrap();
    assert_eq!(record.extra.as_ref().unwrap().generated, Some(new));
    s.unload_tract(&mut store, (4, 4));
    assert!(!s.load_tract(&mut store, (4, 4)).regenerated);
    // The tract that had changed is read as it was stored.
    let loaded = s.load_tract(&mut store, (5, 4));
    assert!(loaded.from_store && !loaded.regenerated);
    assert!(wall_row(&s, (5, 4), wall) && !wall_row(&s, (5, 4), brick));
    // A tract never seen before is simply new.
    assert!(!s.load_tract(&mut store, (6, 4)).regenerated);
    // A different seed is a different fingerprint too.
    let def = changed.realms().lookup("t:plots").unwrap();
    let other = s.cosmos.create_realm(&mut s.timeline, def, 10).unwrap();
    assert_ne!(s.cosmos.generator_fingerprint(other).unwrap(), new);
}

#[test]
fn the_entity_section_goes_out_with_a_tract_and_comes_back_once() {
    let defs = base_defs();
    let mut store = MemoryStore::new();
    let mut s = Session::new(Arc::clone(&defs));
    let (pristine, changed) = (TractPos::new(4, 4), TractPos::new(5, 4));
    assert!(s.load_tract(&mut store, (4, 4)).entities.is_empty());
    s.load_tract(&mut store, (5, 4));
    s.set_block(s.loc((5, 4), 1, 1, 0), "core:pillar");
    // Only a loaded tract takes a section.
    assert!(
        s.cosmos
            .set_tract_entities(s.realm, TractPos::new(9, 9), vec![1])
            .is_err()
    );
    s.cosmos
        .set_tract_entities(s.realm, pristine, b"two sleepers".to_vec())
        .unwrap();
    s.cosmos
        .set_tract_entities(s.realm, changed, b"a cat".to_vec())
        .unwrap();
    assert_eq!(s.cosmos.tract_entities(s.realm, changed), b"a cat");

    // A save in between keeps what was handed in, and it comes back
    // still waiting.
    let before = s.state_hash();
    s.save(&mut store);
    let mut s = Session::load(&defs, &mut store).unwrap();
    assert_eq!(s.state_hash(), before);
    assert_eq!(s.cosmos.tract_entities(s.realm, pristine), b"two sleepers");
    assert_eq!(s.cosmos.tract_entities(s.realm, changed), b"a cat");

    s.unload_tract(&mut store, (4, 4));
    s.unload_tract(&mut store, (5, 4));
    assert!(s.cosmos.tract_entities(s.realm, pristine).is_empty());
    // The pristine tract's blob holds its entities and no cells.
    let blob = store.get(s.realm, pristine).unwrap().unwrap();
    let held = decode_tract(&s.defs, s.cosmos.save_ids(), &blob).unwrap();
    assert!(held.cells.is_none());
    assert_eq!(held.entities, b"two sleepers");
    assert_eq!(
        s.cosmos.tract_record(s.realm, pristine).unwrap().state,
        TractState::Realized { pristine: true }
    );

    // Loading hands each section over, and the store no longer holds
    // it: the pristine tract's blob is gone, the other keeps its cells.
    let back = s.load_tract(&mut store, (4, 4));
    assert_eq!(back.entities, b"two sleepers");
    assert!(!back.from_store);
    assert_eq!(store.get(s.realm, pristine), Ok(None));
    let back = s.load_tract(&mut store, (5, 4));
    assert_eq!(back.entities, b"a cat");
    assert!(back.from_store);
    let blob = store.get(s.realm, changed).unwrap().unwrap();
    let held = decode_tract(&s.defs, s.cosmos.save_ids(), &blob).unwrap();
    assert!(held.cells.is_some() && held.entities.is_empty());
    // Unloaded with nothing handed in, nothing is stored for them
    // again.
    s.unload_tract(&mut store, (4, 4));
    s.unload_tract(&mut store, (5, 4));
    assert_eq!(store.get(s.realm, pristine), Ok(None));
    assert!(s.load_tract(&mut store, (5, 4)).entities.is_empty());
}

// ---- A change of mods --------------------------------------------------

/// The cells the mod-change test looks at, all in tract (1, 1).
const GATE: (i32, i32, i32) = (3, 3, 0);
const SPARE_GATE: (i32, i32, i32) = (5, 3, 0);
const ALLOY_WALL: (i32, i32, i32) = (7, 3, 0);
const PLANKS: (i32, i32, i32) = (9, 3, 0);
const CRATE: (i32, i32, i32) = (9, 5, 0);

/// A world built with the mod `x`: its block, material, floor and
/// fixture in a tract that is unloaded and in one that stays loaded, an
/// area of its kind, a change waiting that names it, a realm of its
/// definition, and a mass of its material in flight.
fn world_with_extra(defs: Arc<WorldDefs>, store: &mut dyn TractStore) -> Session {
    let mut s = Session::new(defs);
    for tract in [(1, 1), (2, 1)] {
        s.load_tract(store, tract);
        let cell = |s: &Session, at: (i32, i32, i32)| s.loc(tract, at.0, at.1, at.2);
        s.set_block(cell(&s, GATE), "x:gate#open");
        s.set_block(cell(&s, SPARE_GATE), "x:gate");
        s.set_block(cell(&s, ALLOY_WALL), "core:wall@x:alloy");
        s.set_floor(cell(&s, PLANKS), "x:planks");
        let crate_ = s.defs.fixture_state("x:crate", None).unwrap();
        let at = cell(&s, CRATE);
        s.ctx().edit(Cause::DEBUG).set_fixture(at, crate_).unwrap();
    }
    s.unload_tract(store, (1, 1));

    let vault = s.defs.area_kinds().lookup("x:vault").unwrap();
    let corner = s.loc((2, 1), 0, 0, 0).cell;
    let mut spec = AreaSpec::new(s.grid, AreaShape::Box(CellBox::new(corner, corner)));
    spec.kind = Some(vault);
    s.ctx().create_area(spec, Cause::DEBUG).unwrap();

    let gate = s.defs.block_state("x:gate#open", None).unwrap();
    s.load_tract(store, (3, 3));
    s.unload_tract(store, (3, 3));
    let waiting = PendingEdit::Cells(vec![CellEdit {
        cell: TractPos::new(3, 3).origin(0),
        layer: Layer::Block,
        state: gate.raw(),
    }]);
    s.cosmos
        .add_pending_edit(s.realm, TractPos::new(3, 3), waiting)
        .unwrap();

    // A realm of the mod's own definition, with a tract that changed.
    let pocket = s.defs.realms().lookup("x:pocket").unwrap();
    let pocket = s.cosmos.create_realm(&mut s.timeline, pocket, 3).unwrap();
    let generators = s.generators.clone();
    let mut io = TractIo::new(&generators, store);
    s.ctx()
        .load_tract(&mut io, pocket, TractPos::new(0, 0))
        .unwrap();
    let grid = s.cosmos.realm(pocket).unwrap().base_grid;
    let at = Location::new(grid, CellPos::new(1, 1, 0));
    s.set_block(at, "core:pillar");

    // Planks hung in the air come down as a mass of the mod's material.
    for x in 20..24 {
        let at = s.loc((2, 1), x, 20, 6);
        s.set_floor(at, "x:planks");
    }
    let at = s.loc((2, 1), 21, 20, 6);
    s.ctx().check_stability(at, Cause::DEBUG).unwrap();
    s.run_until(Tick(300_000));
    let alloy = s.defs.materials().lookup("x:alloy").unwrap();
    assert!(
        s.cosmos
            .falling_masses()
            .any(|mass| mass.parts.iter().any(|part| part.material == alloy))
    );
    s
}

#[test]
fn removing_a_mod_yields_placeholders_and_restoring_it_the_content() {
    let with = defs_of("core:wall", true);
    let without = base_defs();
    let mut store = MemoryStore::new();
    let mut first = world_with_extra(Arc::clone(&with), &mut store);
    let original = first.state_hash();
    first.save(&mut store);

    // ---- Without the mod ----
    let mut second = Session::load(&without, &mut store).unwrap();
    // The definitions to use are the cosmos's: those passed in, and a
    // placeholder for each one the save names and the mods lack.
    let defs = Arc::clone(second.cosmos.defs());
    assert!(without.placeholders().is_empty());
    let mut missing: Vec<(DefKind, String)> = defs
        .placeholders()
        .into_iter()
        .map(|(kind, id)| (kind, id.to_owned()))
        .collect();
    missing.sort();
    assert_eq!(
        missing,
        [
            (DefKind::Floor, "x:planks".to_owned()),
            (DefKind::Block, "x:gate".to_owned()),
            (DefKind::Fixture, "x:crate".to_owned()),
            (DefKind::Material, "x:alloy".to_owned()),
            (DefKind::AreaKind, "x:vault".to_owned()),
            (DefKind::Realm, "x:pocket".to_owned()),
        ]
    );
    // Every definition there was keeps its dense id.
    assert_eq!(
        defs.block_state("core:wall", None),
        without.block_state("core:wall", None)
    );
    // Each is reported with where it was met: among the save's ids,
    // then in what holds it.
    let warnings = second.cosmos.take_save_warnings();
    for (kind, id) in &missing {
        assert!(
            warnings
                .iter()
                .any(|w| w.kind == *kind && w.id == *id && w.at == WarnAt::Ids),
            "{kind} {id}"
        );
    }
    let met = |kind: DefKind, id: &str, at: WarnAt| {
        warnings
            .iter()
            .any(|w| w.kind == kind && w.id == id && w.at == at)
    };
    assert!(met(
        DefKind::AreaKind,
        "x:vault",
        WarnAt::Section("world.areas")
    ));
    assert!(met(
        DefKind::Realm,
        "x:pocket",
        WarnAt::Section("world.cosmos")
    ));
    assert!(met(
        DefKind::Block,
        "x:gate",
        WarnAt::Section("world.cosmos")
    ));
    assert!(met(
        DefKind::Material,
        "x:alloy",
        WarnAt::Section("world.activity")
    ));
    let here = WarnAt::Tract {
        realm: second.realm,
        tract: TractPos::new(2, 1),
    };
    assert!(met(DefKind::Block, "x:gate", here) && met(DefKind::Fixture, "x:crate", here));
    assert!(
        warnings[0]
            .to_string()
            .contains("is not defined by the loaded mods")
    );

    // What stood there is a placeholder that keeps what it was.
    let cell = |s: &Session, tract: (i32, i32), at: (i32, i32, i32)| {
        s.cosmos.cell(s.loc(tract, at.0, at.1, at.2)).unwrap()
    };
    let gate = cell(&second, (2, 1), GATE).block;
    assert!(defs.is_placeholder_state(Layer::Block, gate.raw()));
    assert_eq!(
        defs.state_text(Layer::Block, gate.raw()),
        "x:gate@x:alloy#1"
    );
    // It is inert: it holds up what stood on it and is in nobody's way.
    let info = defs.block_info(gate);
    assert!(info.flags.has(EngineFlag::Anchor) && info.flags.has(EngineFlag::LoadBearing));
    assert!(!info.flags.has(EngineFlag::Opaque));
    assert_eq!(
        second
            .cosmos
            .obstacle_height(second.loc((2, 1), 3, 3, 0))
            .unwrap(),
        Real::ZERO
    );
    // A wall of the missing material is still a wall.
    let wall = cell(&second, (2, 1), ALLOY_WALL).block;
    assert_eq!(
        wall.def(),
        without.block_state("core:wall", None).unwrap().def()
    );
    assert!(defs.is_placeholder_state(Layer::Block, wall.raw()));
    assert_eq!(
        defs.state_text(Layer::Block, wall.raw()),
        "core:wall@x:alloy"
    );
    assert!(
        second
            .cosmos
            .standable(second.loc((2, 1), 9, 3, 0))
            .unwrap()
    );
    // The realm is there, with the extent it had, and cannot be
    // generated further.
    let pocket = RealmId::new(1);
    let kind = second.cosmos.realm_kind(pocket).unwrap();
    assert_eq!(kind.topology.z_min, -8);
    assert!(second.cosmos.is_tract_loaded(pocket, TractPos::new(0, 0)));
    let generators = second.generators.clone();
    let mut io = TractIo::new(&generators, &mut store);
    let refused = second
        .ctx()
        .load_tract(&mut io, pocket, TractPos::new(1, 0));
    assert!(matches!(refused, Err(LoadError::Gen(_))), "{refused:?}");

    // A tract read from the store later says where its placeholders are.
    second.load_tract(&mut store, (1, 1));
    let later = second.cosmos.take_save_warnings();
    let there = WarnAt::Tract {
        realm: second.realm,
        tract: TractPos::new(1, 1),
    };
    assert!(
        later.iter().all(|w| w.at == there) && later.len() == 4,
        "{later:?}"
    );
    assert!(defs.is_placeholder_state(Layer::Block, cell(&second, (1, 1), GATE).block.raw()));

    // Saved and loaded without the mod, nothing more is lost.
    let unchanged = second.state_hash();
    let mut kept = store.clone();
    second.save(&mut kept);
    let again = Session::load(&without, &mut kept).unwrap();
    assert_eq!(again.state_hash(), unchanged);

    // A placeholder breaks at a blow and leaves nothing.
    let impact = defs.damage_types().lookup("core:impact").unwrap();
    let blow = DamagePacket::new(impact, Real::of(1.0e3), Real::of(1.0e9), Cause::DEBUG);
    let spare = second.loc((2, 1), SPARE_GATE.0, SPARE_GATE.1, SPARE_GATE.2);
    second
        .ctx()
        .edit(Cause::DEBUG)
        .damage(spare, Layer::Block, blow)
        .unwrap();
    assert!(second.cosmos.cell(spare).unwrap().block.is_none());
    assert!(second.cosmos.cell(spare).unwrap().fixture.is_none());
    // The mass of the missing material lands and the world goes on.
    second.run_out();
    assert!(second.cosmos.processes().is_idle());
    second.unload_tract(&mut store, (1, 1));
    second.save(&mut store);

    // ---- With the mod again ----
    let mut third = Session::load(&with, &mut store).unwrap();
    assert!(third.cosmos.take_save_warnings().is_empty());
    assert!(third.cosmos.defs().placeholders().is_empty());
    third.load_tract(&mut store, (1, 1));
    for tract in [(1, 1), (2, 1)] {
        let found = cell(&third, tract, GATE);
        assert_eq!(found.block, with.block_state("x:gate#open", None).unwrap());
        assert!(
            third
                .defs
                .block_info(found.block)
                .flags
                .has(EngineFlag::Passable)
        );
        assert_eq!(
            cell(&third, tract, ALLOY_WALL).block,
            with.block_state("core:wall@x:alloy", None).unwrap()
        );
        assert_eq!(
            cell(&third, tract, PLANKS).floor,
            with.floor_state("x:planks", None).unwrap()
        );
        assert_eq!(
            cell(&third, tract, CRATE).fixture,
            with.fixture_state("x:crate", None).unwrap()
        );
    }
    // What was broken while the mod was away stays broken.
    assert!(cell(&third, (2, 1), SPARE_GATE).block.is_none());
    assert_eq!(
        cell(&third, (1, 1), SPARE_GATE).block,
        with.block_state("x:gate", None).unwrap()
    );
    let area = third.cosmos.areas().next().unwrap();
    assert_eq!(area.kind, with.area_kinds().lookup("x:vault"));
    assert_eq!(
        third.cosmos.realm(pocket).unwrap().def,
        with.realms().lookup("x:pocket").unwrap()
    );
    let waiting = third
        .cosmos
        .tract_record(third.realm, TractPos::new(3, 3))
        .unwrap()
        .pending();
    let gate = with.block_state("x:gate#open", None).unwrap();
    assert!(matches!(waiting, [PendingEdit::Cells(cells)] if cells[0].state == gate.raw()));
    third.load_tract(&mut store, (3, 3));
    assert_eq!(cell(&third, (3, 3), (0, 0, 0)).block, gate);

    // And a save that was only loaded and saved without the mod is,
    // with the mod again, the world it was.
    let mut round = MemoryStore::new();
    let mut first = world_with_extra(Arc::clone(&with), &mut round);
    first.save(&mut round);
    let mut between = Session::load(&without, &mut round).unwrap();
    between.save(&mut round);
    let back = Session::load(&with, &mut round).unwrap();
    assert_eq!(back.state_hash(), original);
    assert_eq!(first.state_hash(), original);
}

// ---- Damaged data ------------------------------------------------------

/// A small saved world: two loaded tracts, one of them changed, one
/// unloaded and changed, an area and a check waiting.
fn small_save() -> (Arc<WorldDefs>, MemoryStore, (Hash128, Hash128)) {
    let defs = base_defs();
    let mut store = MemoryStore::new();
    let mut s = Session::new(Arc::clone(&defs));
    s.load_tract(&mut store, (0, 0));
    s.load_tract(&mut store, (1, 0));
    s.load_tract(&mut store, (2, 0));
    s.set_block(s.loc((1, 0), 2, 2, 0), "core:pillar");
    s.set_block(s.loc((2, 0), 2, 2, 0), "core:wall@core:brick");
    s.unload_tract(&mut store, (2, 0));
    let corner = s.loc((0, 0), 1, 1, 0).cell;
    let spec = AreaSpec::new(s.grid, AreaShape::Box(CellBox::new(corner, corner)));
    s.ctx().create_area(spec, Cause::DEBUG).unwrap();
    hang_a_strip(&mut s, (0, 0), 20, 3);
    s.cosmos
        .set_tract_entities(s.realm, TractPos::new(0, 0), b"someone".to_vec())
        .unwrap();
    s.save(&mut store);
    let hash = s.state_hash();
    (defs, store, hash)
}

/// Everything a store holds for the world, each with a way to put
/// other bytes in its place.
fn stored_items(store: &MemoryStore) -> Vec<(String, Vec<u8>)> {
    let mut items: Vec<(String, Vec<u8>)> = store
        .section_names()
        .unwrap()
        .into_iter()
        .filter(|name| name.starts_with("world."))
        .map(|name| {
            let blob = store.section(&name).unwrap().unwrap();
            (name, blob)
        })
        .collect();
    for (realm, tract) in store.tracts() {
        let blob = store.get(realm, tract).unwrap().unwrap();
        items.push((
            format!("tract {} {} {}", realm.raw(), tract.x, tract.y),
            blob,
        ));
    }
    items
}

fn replace(store: &mut MemoryStore, name: &str, bytes: &[u8]) {
    match name.strip_prefix("tract ") {
        Some(rest) => {
            let parts: Vec<i32> = rest.split(' ').map(|part| part.parse().unwrap()).collect();
            let realm = RealmId::new(u32::try_from(parts[0]).unwrap());
            store.put(realm, TractPos::new(parts[1], parts[2]), bytes);
        }
        None => store.put_section(name, bytes),
    }
}

/// Loads a store that may be damaged. It must not panic; if it loads,
/// and every tract in it can still be read, it is the world that was
/// saved.
fn load_damaged(defs: &Arc<WorldDefs>, store: &mut MemoryStore, hash: (Hash128, Hash128)) -> bool {
    match Session::load(defs, store) {
        Err(_) => false,
        Ok(mut s) => {
            assert_eq!(s.state_hash(), hash);
            // The unloaded tract is read when it is loaded: damage to
            // it shows then, as an error.
            let generators = s.generators.clone();
            let mut io = TractIo::new(&generators, store);
            let realm = s.realm;
            s.ctx()
                .load_tract(&mut io, realm, TractPos::new(2, 0))
                .is_ok()
        }
    }
}

#[test]
fn a_save_cut_short_anywhere_is_an_error_and_never_a_panic() {
    let (defs, store, hash) = small_save();
    assert!(load_damaged(&defs, &mut store.clone(), hash));
    let items = stored_items(&store);
    assert_eq!(
        items.len(),
        8 + 3,
        "{:?}",
        items.iter().map(|i| &i.0).collect::<Vec<_>>()
    );
    for (name, blob) in &items {
        for len in 0..blob.len() {
            let mut cut = store.clone();
            replace(&mut cut, name, &blob[..len]);
            assert!(!load_damaged(&defs, &mut cut, hash), "{name} cut to {len}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any bytes of any stored item changed: an error, or (where the
    /// compression absorbs the change) the world that was saved. Never
    /// a panic, never another world.
    #[test]
    fn a_save_with_flipped_bytes_is_an_error_or_the_same_world(
        pick in any::<prop::sample::Index>(),
        flips in prop::collection::vec((any::<prop::sample::Index>(), 1_u8..=255), 1..4),
    ) {
        let (defs, store, hash) = small_save();
        let items = stored_items(&store);
        let (name, blob) = pick.get(&items);
        let mut damaged = blob.clone();
        for (at, mask) in &flips {
            let at = at.index(damaged.len());
            damaged[at] ^= mask;
        }
        let mut hurt = store.clone();
        replace(&mut hurt, name, &damaged);
        // Either outcome is allowed; `load_damaged` checks the world.
        let _ = load_damaged(&defs, &mut hurt, hash);
    }

}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// The same for the file on disk: a save file that has been written
    /// over is refused when it is opened or when it is read.
    #[test]
    fn a_save_file_written_over_is_an_error_or_the_same_world(
        flips in prop::collection::vec((any::<prop::sample::Index>(), 1_u8..=255), 1..6),
        cut in prop::option::of(any::<prop::sample::Index>()),
    ) {
        let (defs, store, hash) = small_save();
        let path = temp_file("damaged");
        {
            let mut file = RedbStore::open(&path).unwrap();
            copy_store(&store, &mut file);
            file.commit().unwrap();
        }
        let mut bytes = std::fs::read(&path).unwrap();
        for (at, mask) in &flips {
            let at = at.index(bytes.len());
            bytes[at] ^= mask;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut.index(bytes.len()));
        }
        std::fs::write(&path, &bytes).unwrap();
        if let Ok(mut file) = RedbStore::open(&path)
            && let Ok(s) = Session::load(&defs, &mut file)
        {
            prop_assert_eq!(s.state_hash(), hash);
        }
        std::fs::remove_file(&path).unwrap();
    }
}

// ---- The store on disk -------------------------------------------------

static NEXT_FILE: AtomicU32 = AtomicU32::new(0);

/// A path for a save file that no other test uses.
fn temp_file(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let number = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{name}-{}-{number}.c2save", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// Puts everything one store holds into another.
fn copy_store(from: &MemoryStore, to: &mut dyn TractStore) {
    for (realm, tract) in from.tracts() {
        to.put(realm, tract, &from.get(realm, tract).unwrap().unwrap());
    }
    for name in from.section_names().unwrap() {
        to.put_section(&name, &from.section(&name).unwrap().unwrap());
    }
}

#[test]
fn the_file_store_keeps_what_was_committed_and_nothing_else() {
    let path = temp_file("store");
    let (here, there) = (RealmId::new(0), RealmId::new(1));
    let tract = TractPos::new(-3, 8);
    {
        let mut store = RedbStore::open(&path).unwrap();
        // A new file holds nothing.
        assert_eq!(store.get(here, tract), Ok(None));
        assert_eq!(store.section("sim.clock"), Ok(None));
        assert_eq!(store.section_names(), Ok(vec![]));
        assert_eq!(store.tract_count(), Ok(0));
        // What is put is there at once.
        store.put(here, tract, b"one");
        store.put(there, tract, b"two");
        store.put_section("sim.clock", b"noon");
        assert_eq!(store.get(here, tract), Ok(Some(b"one".to_vec())));
        assert_eq!(store.section("sim.clock"), Ok(Some(b"noon".to_vec())));
        assert_eq!(store.pending(), 3);
        assert_eq!(store.tract_count(), Ok(2));
        store.commit().unwrap();
        assert_eq!(store.pending(), 0);
        // After the commit: changed, deleted and added, and not
        // committed.
        store.put(here, tract, b"changed");
        store.delete(there, tract);
        store.put(here, TractPos::new(0, 0), b"new");
        store.put_section("sim.late", b"late");
        store.delete_section("sim.clock");
        assert_eq!(store.get(here, tract), Ok(Some(b"changed".to_vec())));
        assert_eq!(store.get(there, tract), Ok(None));
        assert_eq!(store.section("sim.clock"), Ok(None));
        assert_eq!(store.section_names(), Ok(vec!["sim.late".to_owned()]));
        assert_eq!(store.tract_count(), Ok(2));
    }
    // The file holds what was committed and nothing else.
    {
        let mut store = RedbStore::open(&path).unwrap();
        assert_eq!(store.get(here, tract), Ok(Some(b"one".to_vec())));
        assert_eq!(store.get(there, tract), Ok(Some(b"two".to_vec())));
        assert_eq!(store.get(here, TractPos::new(0, 0)), Ok(None));
        assert_eq!(store.section_names(), Ok(vec!["sim.clock".to_owned()]));
        assert_eq!(store.tract_count(), Ok(2));
        // What is put and then discarded is as if it never was.
        store.delete(here, tract);
        store.discard();
        assert_eq!(store.get(here, tract), Ok(Some(b"one".to_vec())));
        // A deletion that is committed is gone for good.
        store.delete(here, tract);
        store.delete_section("sim.clock");
        store.commit().unwrap();
    }
    let store = RedbStore::open(&path).unwrap();
    assert_eq!(store.get(here, tract), Ok(None));
    assert_eq!(store.get(there, tract), Ok(Some(b"two".to_vec())));
    assert_eq!(store.section_names(), Ok(vec![]));
    drop(store);
    std::fs::remove_file(&path).unwrap();

    // Something that is not a save file is refused, not opened.
    let path = temp_file("not-a-save");
    std::fs::write(&path, vec![0x5a; 3000]).unwrap();
    assert!(RedbStore::open(&path).is_err());
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_save_file_with_a_damaged_start_is_an_error_or_reads_the_same() {
    // The start of the file is the database's own header: the part of
    // a file where one wrong byte could do the most.
    let path = temp_file("header");
    {
        let mut store = RedbStore::open(&path).unwrap();
        store.put_section("sim.clock", b"noon");
        store.commit().unwrap();
    }
    let good = std::fs::read(&path).unwrap();
    for at in (0..good.len().min(600)).step_by(7) {
        let mut bytes = good.clone();
        bytes[at] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        if let Ok(store) = RedbStore::open(&path)
            && let Ok(found) = store.section("sim.clock")
        {
            assert_eq!(found, Some(b"noon".to_vec()), "byte {at}");
        }
    }
    for len in [0, 1, 63, 64, 511, 4096, good.len() / 2, good.len() - 1] {
        std::fs::write(&path, &good[..len.min(good.len())]).unwrap();
        if let Ok(store) = RedbStore::open(&path)
            && let Ok(Some(found)) = store.section("sim.clock")
        {
            assert_eq!(found, b"noon", "cut to {len}");
        }
    }
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_world_saved_to_a_file_comes_back_from_it() {
    let defs = base_defs();
    let path = temp_file("world");
    let mut memory = MemoryStore::new();
    let (before, straight) = {
        let mut store = RedbStore::open(&path).unwrap();
        let mut first = busy_world(Arc::clone(&defs), &mut store);
        let before = first.state_hash();
        first.save(&mut store);
        // The same world through the in-memory store, to compare with.
        let mut twin = busy_world(Arc::clone(&defs), &mut memory);
        twin.save(&mut memory);
        assert_eq!(twin.state_hash(), before);
        // What happens after the save and is never committed.
        first.events.drain();
        (before, carry_on(&mut first, &mut store))
    };
    // The file holds the save, and none of what came after it.
    let mut store = RedbStore::open(&path).unwrap();
    assert_eq!(store.tract_count(), Ok(memory.len()));
    for name in memory.section_names().unwrap() {
        assert_eq!(store.section(&name), memory.section(&name), "{name}");
    }
    let mut second = Session::load(&defs, &mut store).unwrap();
    assert_eq!(second.state_hash(), before);
    assert_eq!(carry_on(&mut second, &mut store), straight);
    drop(store);
    std::fs::remove_file(&path).unwrap();
}

/// Prints how large a save of the busy world is. Run with
/// `cargo test -p clash-world --test save -- --ignored --nocapture`.
#[test]
#[ignore = "prints sizes; nothing is asserted"]
fn sizes_of_a_saved_world() {
    let defs = base_defs();
    let mut store = MemoryStore::new();
    let mut s = busy_world(Arc::clone(&defs), &mut store);
    let report = s.cosmos.save_world(&mut store).unwrap();
    println!("{report:?}");
    for name in store.section_names().unwrap() {
        let blob = store.section(&name).unwrap().unwrap();
        let header = codec::read_blob_header(&blob).unwrap();
        println!(
            "{name}: {} bytes, body {} bytes",
            blob.len(),
            header.body_len
        );
    }
    for (realm, tract) in store.tracts() {
        let blob = store.get(realm, tract).unwrap().unwrap();
        let header = codec::read_blob_header(&blob).unwrap();
        println!(
            "tract {tract}: {} bytes, body {} bytes",
            blob.len(),
            header.body_len
        );
    }
    let _ = Dur::ZERO;
}
