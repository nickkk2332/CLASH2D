//! The benchmarks. Each builds its scene from the repository's mods and
//! times the operation of one row of a budget table.
//!
//! | Scene | Used by |
//! |-------|---------|
//! | A block of nine tracts of the test city, loaded into a world of its own | cell reads, views, lines, paths, light |
//! | Six wooden storeys alight in a dozen places on each | the field step |
//! | The simulation over the test city, with a crowd in a street | perception, expressions |
//! | The console's host over a generated city | the frame |
//! | A full screen of glyphs and sprites in the default theme's font | the window frontend's drawing |

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real};
use clash_data::Defs;
use clash_data::expr::Runner;
use clash_expr::{Context, Type, compile};
use clash_front_window::{GlyphAtlas, PixelBuffer, Renderer};
use clash_save::host::{GameHost, HostEnv};
use clash_scene::{Attrs, Canvas, Color, Frame, Rgb, Sprite, Wash};
use clash_sim::console::{HostKind, Session};
use clash_sim::perception::{self, Signal, signal::Spot};
use clash_sim::{
    AdvanceLimit, Bound, Command, DebugCommand, EntitySpec, Place, Presence, Sim, Subject,
};
use clash_time::{Timeline, Wake, WakeDomain, WakeId};
use clash_world::dims::TRACT_XY;
use clash_world::{
    BlockState, Cause, CellBox, CellPos, ColumnBuilder, Cosmos, EventSink, FovSet, GridId,
    Location, MemoryStore, MoveProfile, OccupantFlags, OccupantKind, PathFinder, PathLimits,
    RealmId, SkyTime, TractIo, TractPos, WorldCtx, WorldDefs, WorldEvent,
};

use super::{Bench, Measured, best_ns};

/// Every benchmark, in the order they run.
pub const BENCHES: &[Bench] = &[
    Bench {
        name: "world.cell_flag_read",
        what: "the flags of a cell, read through the cosmos, over scattered cells of the city",
        run: cell_flag_read,
    },
    Bench {
        name: "world.cell_state_read",
        what: "the three elements of a cell, read through the cosmos",
        run: cell_state_read,
    },
    Bench {
        name: "world.realize_city_tract",
        what: "realising one tract of the test city, on one thread",
        run: realize_city_tract,
    },
    Bench {
        name: "world.field_of_view_30m",
        what: "a field of view of 30 m worked out afresh in a street of the city",
        run: field_of_view,
    },
    Bench {
        name: "world.line_of_sight_30m",
        what: "one line of sight of 30 m along a street of the city",
        run: line_of_sight,
    },
    Bench {
        name: "world.path_60_steps",
        what: "a path of about sixty steps through the city, for a walker",
        run: path_of_sixty_steps,
    },
    Bench {
        name: "world.field_step",
        what: "a building fire stepped: time a step, scaled to 50,000 active cells",
        run: field_step,
    },
    Bench {
        name: "light.fill_cell",
        what: "the light of one cell of an 80 x 50 view filled from the cache",
        run: light_fill_cell,
    },
    Bench {
        name: "light.one_cell",
        what: "the light at one cell asked for alone, cached",
        run: light_one_cell,
    },
    Bench {
        name: "light.view_after_edit",
        what: "an 80 x 50 view filled again after a wall is put up or taken down",
        run: light_view_after_edit,
    },
    Bench {
        name: "perception.sight_30m",
        what: "one evaluation of sight at 30 m in a street: lines of sight, light, distance",
        run: sight_at_thirty_metres,
    },
    Bench {
        name: "perception.signal_to_50",
        what: "a sound given off among fifty listeners, sent to each",
        run: signal_to_fifty,
    },
    Bench {
        name: "perception.crowd_of_50",
        what: "one simulated second of fifty people walking in a street, seeing and hearing",
        run: crowd_of_fifty,
    },
    Bench {
        name: "perception.party_sweep",
        what: "a sweep of sight by a directly controlled character: its view, map memory and frame",
        run: party_sweep,
    },
    Bench {
        name: "expr.condition",
        what: "a condition of about ten operations over a person and locals, through the runner",
        run: condition,
    },
    Bench {
        name: "recipe.start_and_complete",
        what: "a recipe by hand started and completed: its checks, its input used up, its wake-up scheduled and handled, its output made",
        run: recipe_cycle,
    },
    Bench {
        name: "scene.map_frame",
        what: "a 200 x 60 frame of the map of a generated city, from the view model",
        run: map_frame,
    },
    Bench {
        name: "window.draw_frame",
        what: "a frame of 20,000 glyphs and sprites drawn by the window frontend into a 1920 x 1080 pixel buffer",
        run: window_frame,
    },
];

/// What the benchmarks share: the default profile, loaded once.
#[derive(Default)]
pub struct Kit {
    defs: Option<Arc<Defs>>,
}

impl Kit {
    /// The default profile of the repository's mods.
    fn defs(&mut self) -> Result<Arc<Defs>, String> {
        if let Some(defs) = &self.defs {
            return Ok(Arc::clone(defs));
        }
        let types = crate::engine_types();
        let outcome = clash_scene::engine_loader(&types)
            .search_path(mods_dir())
            .load_profile("default");
        if outcome.has_errors() {
            return Err(outcome.diagnostics.render());
        }
        let defs = outcome
            .defs
            .ok_or_else(|| "the default profile did not load".to_owned())?;
        self.defs = Some(Arc::clone(&defs));
        Ok(defs)
    }

    fn world_defs(&mut self) -> Result<Arc<WorldDefs>, String> {
        let defs = self.defs()?;
        WorldDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())
    }
}

/// The repository's mods.
fn mods_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// The realm the benchmarks stand in, and its seed.
const CITY_REALM: &str = "core:test_city";
const CITY_SEED: u64 = 11;

/// The tract the scenes are centred on: well inside the city.
const START: TractPos = TractPos::new(20, 20);

/// A world of its own over the test city, nine tracts round [`START`]
/// loaded.
struct City {
    cosmos: Cosmos,
    timeline: Timeline,
    events: EventSink<WorldEvent>,
    grid: GridId,
}

impl City {
    fn new(world: &Arc<WorldDefs>) -> Result<City, String> {
        let mut timeline = Timeline::new();
        let mut cosmos = Cosmos::new(Arc::clone(world));
        let def = world
            .realms()
            .lookup(CITY_REALM)
            .ok_or("no test city realm")?;
        let realm: RealmId = cosmos
            .create_realm(&mut timeline, def, CITY_SEED)
            .map_err(|error| error.to_string())?;
        let grid = cosmos.realm(realm).ok_or("no realm")?.base_grid;
        let mut events = EventSink::new();
        let generators = clash_gen::generators(world);
        let mut store = MemoryStore::new();
        let mut io = TractIo::new(&generators, &mut store);
        let tracts: Vec<(RealmId, TractPos)> = (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| TractPos::new(START.x + dx, START.y + dy)))
            .map(|tract| (realm, tract))
            .collect();
        let loaded =
            WorldCtx::new(&mut cosmos, &mut timeline, &mut events).load_tracts(&mut io, &tracts);
        if let Some(Err(error)) = loaded.into_iter().find(Result::is_err) {
            return Err(format!("a tract did not load: {error:?}"));
        }
        Ok(City {
            cosmos,
            timeline,
            events,
            grid,
        })
    }

    /// A cell of the street along the north edge of the start tract.
    fn street(&self, dx: i32) -> Location {
        Location::new(
            self.grid,
            CellPos::new(START.x * TRACT_XY + dx, START.y * TRACT_XY + 1, 0),
        )
    }

    /// A fixed scatter of cells over the loaded tracts and three storeys.
    fn scatter(&self) -> Vec<Location> {
        let span = 3 * TRACT_XY;
        let (x0, y0) = ((START.x - 1) * TRACT_XY, (START.y - 1) * TRACT_XY);
        let mut cells = Vec::new();
        let mut state = 0x9e37_79b9_u32;
        for _ in 0..4096 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (state >> 8) as i32 % span;
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let y = (state >> 8) as i32 % span;
            let z = (state >> 28) as i32 % 3 - 1;
            cells.push(Location::new(self.grid, CellPos::new(x0 + x, y0 + y, z)));
        }
        cells
    }

    fn noon(&self) -> SkyTime {
        SkyTime::at(Dur::from_secs(12 * 3600))
    }
}

// ---- The world ---------------------------------------------------------

fn cell_flag_read(kit: &mut Kit) -> Result<Measured, String> {
    let city = City::new(&kit.world_defs()?)?;
    let cells = city.scatter();
    let ns = best_ns(7, 200, || {
        cells
            .iter()
            .map(|loc| {
                city.cosmos
                    .flags(std::hint::black_box(*loc))
                    .map_or(0, |flags| u64::from(flags.bits()))
            })
            .sum::<u64>()
    }) / cells.len() as f64;
    Ok(Measured::new(
        ns,
        format!("{} scattered cells", cells.len()),
    ))
}

fn cell_state_read(kit: &mut Kit) -> Result<Measured, String> {
    let city = City::new(&kit.world_defs()?)?;
    let cells = city.scatter();
    let ns = best_ns(7, 200, || {
        cells
            .iter()
            .map(|loc| {
                city.cosmos
                    .cell(std::hint::black_box(*loc))
                    .map_or(0, |cell| {
                        u64::from(cell.block.raw().def) + u64::from(cell.floor.raw().def)
                    })
            })
            .sum::<u64>()
    }) / cells.len() as f64;
    Ok(Measured::new(
        ns,
        format!("{} scattered cells", cells.len()),
    ))
}

fn realize_city_tract(kit: &mut Kit) -> Result<Measured, String> {
    let world = kit.world_defs()?;
    let def = world
        .realms()
        .lookup(CITY_REALM)
        .ok_or("no test city realm")?;
    let mut registry = clash_gen::registry();
    registry.set_runner(Arc::new(clash_world::SerialRunner));
    let generators = registry.bind(&world);
    let tracts: Vec<TractPos> = (0..4)
        .flat_map(|y| (0..4).map(move |x| TractPos::new(START.x + x, START.y + y)))
        .collect();
    let ns = best_ns(5, 1, || {
        clash_gen::realize_tracts(&generators, def, CITY_SEED, &tracts).map(|done| done.len())
    }) / tracts.len() as f64;
    Ok(Measured::new(
        ns,
        format!("{} tracts realised together", tracts.len()),
    ))
}

fn field_of_view(kit: &mut Kit) -> Result<Measured, String> {
    let city = City::new(&kit.world_defs()?)?;
    let origin = city.street(24);
    let mut view = FovSet::new();
    let ns = best_ns(7, 100, || {
        view.clear();
        city.cosmos
            .field_of_view(origin, 1500, Real::of(30.0), &mut view)
            .map(|_| view.len())
    });
    let again = best_ns(5, 2000, || {
        city.cosmos
            .field_of_view(origin, 1500, Real::of(30.0), &mut view)
            .map(|_| view.len())
    });
    Ok(Measured::new(
        ns,
        format!(
            "{} cells seen; asked again of an unchanged world {}",
            view.len(),
            super::show_ns(again)
        ),
    ))
}

fn line_of_sight(kit: &mut Kit) -> Result<Measured, String> {
    let city = City::new(&kit.world_defs()?)?;
    let a = city.street(0).point_at(1500);
    let b = city.street(30).point_at(900);
    if city.cosmos.line_of_sight(a, b) != Ok(true) {
        return Err("the street is not clear".to_owned());
    }
    let ns = best_ns(7, 20_000, || city.cosmos.line_of_sight(a, b));
    Ok(Measured::new(ns, "eye to the middle of a body"))
}

fn path_of_sixty_steps(kit: &mut Kit) -> Result<Measured, String> {
    let city = City::new(&kit.world_defs()?)?;
    let from = city.street(0);
    let walker = MoveProfile::walker();
    let mut finder = PathFinder::new();
    // The first of these places a walker can reach in fifty steps or
    // more: across the tract, round what stands in the way.
    let (to, steps, settled) = PATH_GOALS
        .iter()
        .find_map(|(dx, dy)| {
            let to = Location::new(
                city.grid,
                CellPos::new(from.cell.x + dx, from.cell.y + dy, 0),
            );
            let path = finder
                .find(&city.cosmos, from, to, &walker, PathLimits::DEFAULT)
                .ok()?;
            (path.steps() >= 50).then(|| (to, path.steps(), path.settled()))
        })
        .ok_or("no goal of the path is reachable")?;
    let ns = best_ns(7, 50, || {
        finder
            .find(&city.cosmos, from, to, &walker, PathLimits::DEFAULT)
            .map(|path| path.steps())
    });
    Ok(Measured::new(
        ns,
        format!("{steps} steps, {settled} cells settled"),
    ))
}

/// Where the path is asked to go, as cells from the start of the
/// street, in the order they are tried.
const PATH_GOALS: [(i32, i32); 6] = [(40, 40), (45, 35), (50, 30), (30, 45), (55, 20), (60, 0)];

/// How many active cells the field step's budget row is for.
const FIELD_BUDGET_CELLS: f64 = 50_000.0;

fn field_step(kit: &mut Kit) -> Result<Measured, String> {
    let world = kit.world_defs()?;
    let mut best_per_cell = f64::INFINITY;
    let mut most = 0;
    let mut steps = 0;
    for _ in 0..3 {
        let (per_cell, peak, count) = building_fire(&world)?;
        best_per_cell = best_per_cell.min(per_cell);
        most = peak;
        steps = count;
    }
    Ok(Measured::new(
        best_per_cell * FIELD_BUDGET_CELLS,
        format!(
            "{:.0} ns an active cell a step; {steps} steps, at most {most} active cells",
            best_per_cell
        ),
    ))
}

/// Six storeys of wooden floor sixty cells by twenty-eight, open to each
/// other at one end, alight in a dozen places on every storey, burnt
/// out: the time of each step over the active cells it began with.
fn building_fire(world: &Arc<WorldDefs>) -> Result<(f64, u64, u32), String> {
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(world));
    let def = world
        .realms()
        .lookup("core:testland")
        .ok_or("no testland realm")?;
    let realm = cosmos
        .create_realm(&mut timeline, def, 7)
        .map_err(|error| error.to_string())?;
    let grid = cosmos.realm(realm).ok_or("no realm")?.base_grid;
    let rock = world
        .block_state("core:solid", None)
        .map_err(|error| format!("{error:?}"))?;
    let boards = world
        .floor_state("core:boards", None)
        .map_err(|error| format!("{error:?}"))?;
    for tract in [TractPos::new(0, 0), TractPos::new(1, 0)] {
        let mut builder = ColumnBuilder::new(world, rock, BlockState::NONE, 0);
        for z in 0..6 {
            for y in 2..30 {
                for x in 0..32 {
                    if z == 0 || (tract.x, x) != (0, 2) {
                        builder.set_floor(CellPos::new(x, y, z), boards);
                    }
                }
            }
        }
        clash_world::test_support::load_column(&mut cosmos, grid, tract, builder.build())
            .map_err(|error| error.to_string())?;
    }
    let fire = world
        .fields()
        .resolve("core:fire", None)
        .ok_or("no core:fire")?;
    let mut events = EventSink::new();
    let lit = Cause::DEBUG;
    for z in 0..6 {
        for i in 0..12 {
            let at = Location::new(grid, CellPos::new(4 + 5 * i, 4 + (7 * i + 3 * z) % 24, z));
            WorldCtx::new(&mut cosmos, &mut timeline, &mut events)
                .edit(lit)
                .add_field(at, fire, 120)
                .map_err(|error| error.to_string())?;
        }
    }
    let mut steps = 0u32;
    let mut most = 0u64;
    let mut cell_steps = 0u64;
    let began = std::time::Instant::now();
    while cosmos.active_field_cells() > 0 && steps < 400 {
        let active = cosmos.active_field_cells();
        let until = timeline.now() + Dur::from_millis(500);
        while let Some((_, wake)) = timeline.pop(until) {
            if wake.domain == WakeDomain::World {
                WorldCtx::new(&mut cosmos, &mut timeline, &mut events)
                    .handle_wake(&wake)
                    .map_err(|error| error.to_string())?;
            }
        }
        timeline.advance_to(until);
        events.clear();
        most = most.max(active);
        cell_steps += active;
        steps += 1;
    }
    let took = began.elapsed().as_nanos() as f64;
    Ok((took / cell_steps.max(1) as f64, most, steps))
}

// ---- Light ---------------------------------------------------------------

/// An 80 × 50 view of the city round the start of the street.
fn light_view(city: &City) -> CellBox {
    let corner = city.street(0).cell;
    CellBox::new(
        CellPos::new(corner.x - 30, corner.y - 20, 0),
        CellPos::new(corner.x + 49, corner.y + 29, 0),
    )
}

fn light_fill_cell(kit: &mut Kit) -> Result<Measured, String> {
    let city = City::new(&kit.world_defs()?)?;
    let view = light_view(&city);
    let mut out = Vec::new();
    let fill = |out: &mut Vec<Real>| {
        city.cosmos
            .fill_light(city.grid, view, city.noon(), &[], out)
            .map(|()| out.len())
    };
    fill(&mut out).map_err(|error| error.to_string())?;
    let ns = best_ns(7, 500, || fill(&mut out)) / view.volume() as f64;
    Ok(Measured::new(ns, format!("{} cells a fill", view.volume())))
}

fn light_one_cell(kit: &mut Kit) -> Result<Measured, String> {
    let city = City::new(&kit.world_defs()?)?;
    let at = city.street(20);
    city.cosmos
        .light(at, city.noon())
        .map_err(|error| error.to_string())?;
    let ns = best_ns(7, 20_000, || city.cosmos.light(at, city.noon()));
    Ok(Measured::new(ns, "a cell of the street at noon"))
}

fn light_view_after_edit(kit: &mut Kit) -> Result<Measured, String> {
    let world = kit.world_defs()?;
    let mut city = City::new(&world)?;
    let view = light_view(&city);
    let wall = world
        .block_state("core:wall", None)
        .map_err(|error| format!("{error:?}"))?;
    let spot = city.street(10);
    let noon = city.noon();
    let mut out = Vec::new();
    city.cosmos
        .fill_light(city.grid, view, noon, &[], &mut out)
        .map_err(|error| error.to_string())?;
    let mut up = false;
    let ns = best_ns(7, 40, || {
        up = !up;
        let state = if up { wall } else { BlockState::NONE };
        let City {
            cosmos,
            timeline,
            events,
            grid,
        } = &mut city;
        let _ = WorldCtx::new(cosmos, timeline, events)
            .edit(Cause::DEBUG)
            .set_block(spot, state);
        events.clear();
        cosmos
            .fill_light(*grid, view, noon, &[], &mut out)
            .map(|()| out.len())
    });
    Ok(Measured::new(ns, "a wall in the street, up and down"))
}

// ---- The simulation --------------------------------------------------------

/// A simulation over the test city with the tracts round the start
/// loaded and hot, as the perception tests make it.
fn city_sim(kit: &mut Kit) -> Result<Sim, String> {
    let defs = kit.defs()?;
    let world = WorldDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())?;
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world
        .realms()
        .lookup(CITY_REALM)
        .ok_or("no test city realm")?;
    cosmos
        .create_realm(&mut timeline, def, CITY_SEED)
        .map_err(|error| error.to_string())?;
    let mut sim = Sim::from_parts(defs, cosmos, timeline, 5);
    let anchor = create(&mut sim, Place::Stored)?;
    let far = Location::new(
        GridId::new(0),
        CellPos::new(START.x * TRACT_XY + 1, (START.y + 3) * TRACT_XY + 1, 0),
    );
    debug(
        &mut sim,
        DebugCommand::StartSession {
            entity: anchor,
            at: far,
        },
    )?;
    for dx in 0..48 {
        let cell = sim
            .truth()
            .cosmos()
            .cell(sim_street(dx))
            .map_err(|error| error.to_string())?;
        if !(cell.block.is_none() && cell.fixture.is_none()) {
            return Err(format!("the street is not clear {dx} cells along"));
        }
    }
    sim.drain_events();
    Ok(sim)
}

/// A recipe by hand (16 §8.2), started and completed over and over by a
/// creature in the street: the cloth it uses is made in it each time,
/// and five seconds are advanced for the work.
fn recipe_cycle(kit: &mut Kit) -> Result<Measured, String> {
    let mut sim = city_sim(kit)?;
    let worker = create(&mut sim, Place::at(sim_street(2)))?;
    let recipe = sim
        .defs()
        .lookup::<clash_sim::recipe::RecipeDef>("core:bandage")
        .ok_or("no recipe `core:bandage`")?;
    let cloth = clash_sim::item::def_named(&sim, "core:cloth").map_err(|reason| reason.message)?;
    let ns = best_ns(5, 200, || {
        let made = clash_sim::item::create(&mut sim, cloth, 1, Place::In(worker), Cause::DEBUG);
        let started =
            clash_sim::recipe::work::start(&mut sim, Some(worker), recipe, None, Cause::DEBUG);
        let until = clash_core::Tick(sim.now().as_micros() + 5_000_001);
        let _ = sim.advance(AdvanceLimit::Until(until));
        (made.is_ok(), started.is_ok())
    });
    Ok(Measured::new(
        ns,
        "start and completion together, with making the cloth and advancing the clock",
    ))
}

fn sim_street(dx: i32) -> Location {
    Location::new(
        GridId::new(0),
        CellPos::new(START.x * TRACT_XY + dx, START.y * TRACT_XY + 1, 0),
    )
}

fn debug(sim: &mut Sim, command: DebugCommand) -> Result<(), String> {
    sim.submit(Command::Debug(command))
        .map_err(|error| format!("{error:?}"))
}

/// A person, made by debug command.
fn create(sim: &mut Sim, place: Place) -> Result<EntityId, String> {
    let id = sim
        .truth()
        .entities()
        .next_id()
        .ok_or("no entity id left")?;
    let spec = EntitySpec::new(OccupantKind::Creature).with_presence(
        Presence::new(OccupantKind::Creature).with_flags(OccupantFlags::BLOCKS_MOVE),
    );
    debug(sim, DebugCommand::CreateEntity { spec, place })?;
    Ok(id)
}

fn give(sim: &mut Sim, entity: EntityId, sense: &str) -> Result<(), String> {
    let id = sim
        .perception_defs()
        .sense_named(sense)
        .ok_or_else(|| format!("no sense {sense}"))?;
    perception::attach_sense(sim, entity, id).map_err(|reason| reason.to_string())
}

fn advance_by(sim: &mut Sim, secs: f64) {
    let until = sim.now() + Dur::from_secs_real(Real::of(secs));
    sim.advance(AdvanceLimit::Until(until));
}

/// A wake-up handler that walks an entity a cell east and back, a step
/// every 700 ms of its own time.
fn pacer(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let entity = EntityId::from_bits(wake.id);
    let Some(Place::At { loc, .. }) = sim.truth().place(entity) else {
        return;
    };
    let dx = if wake.arg.is_multiple_of(2) { 1 } else { -1 };
    let to = Location::new(
        loc.grid,
        CellPos::new(loc.cell.x + dx, loc.cell.y, loc.cell.z),
    );
    let _ = sim.relocate(entity, Place::at(to), Cause::DEBUG.by(entity));
    let _ = sim.schedule_for(
        entity,
        Dur::from_millis(700),
        Wake::new(WakeDomain::Stepper, wake.id, wake.arg + 1),
    );
}

/// Fifty people in two rows of a street, all in range of each other,
/// with sight and hearing, facing up and down it, walking.
fn crowd(sim: &mut Sim) -> Result<Vec<EntityId>, String> {
    sim.set_wake_handler(WakeDomain::Stepper, pacer);
    let mut crowd = Vec::new();
    for i in 0..50 {
        let at = Location::new(
            GridId::new(0),
            CellPos::new(
                sim_street(0).cell.x + (i % 25) * 2,
                sim_street(0).cell.y + i / 25,
                0,
            ),
        );
        let one = create(sim, Place::at(at))?;
        give(sim, one, "core:human_sight")?;
        give(sim, one, "core:human_hearing")?;
        let facing = if i % 2 == 0 { 90.0 } else { 270.0 };
        perception::set_facing(sim, one, Real::of(facing)).map_err(|reason| reason.to_string())?;
        sim.schedule_for(
            one,
            Dur::from_millis(i64::from(i) * 20),
            Wake::new(WakeDomain::Stepper, one.to_bits(), 0),
        )
        .map_err(|error| format!("{error:?}"))?;
        crowd.push(one);
    }
    Ok(crowd)
}

fn sight_at_thirty_metres(kit: &mut Kit) -> Result<Measured, String> {
    let sim = city_sim(kit)?;
    let sight = sim
        .perception_defs()
        .kind_named("core:sight")
        .ok_or("no sight")?;
    let info = sim.perception_defs().kind(sight).ok_or("no sight")?.clone();
    let eye = Spot::point(sim_street(0), 1500);
    let body = Spot {
        loc: sim_street(30),
        alt_mm: 0,
        height_mm: 1700,
    };
    let seen = perception::signal::strength_at(&sim, &info, Real::ONE, body, eye);
    if seen <= Real::ZERO {
        return Err("nothing is seen down the street".to_owned());
    }
    let ns = best_ns(7, 20_000, || {
        perception::signal::strength_at(&sim, &info, Real::ONE, body, eye)
    });
    Ok(Measured::new(ns, format!("received {seen}")))
}

fn signal_to_fifty(kit: &mut Kit) -> Result<Measured, String> {
    let mut sim = city_sim(kit)?;
    crowd(&mut sim)?;
    advance_by(&mut sim, 1.0);
    sim.set_wake_handler(WakeDomain::Stepper, |_, _, _| {});
    sim.drain_events();
    let sound = sim
        .perception_defs()
        .kind_named("core:sound")
        .ok_or("no sound")?;
    let mut best_given = f64::INFINITY;
    let mut best_received = f64::INFINITY;
    let mut delayed = 0;
    for _ in 0..7 {
        let started = std::time::Instant::now();
        let emitted = perception::emit_signal(
            &mut sim,
            Signal::new(sound, sim_street(24), Real::of(5.0), Cause::DEBUG),
        )
        .map_err(|reason| reason.to_string())?;
        best_given = best_given.min(started.elapsed().as_nanos() as f64);
        delayed = emitted.delayed;
        let started = std::time::Instant::now();
        advance_by(&mut sim, 0.2);
        best_received = best_received.min(started.elapsed().as_nanos() as f64);
        sim.drain_events();
    }
    Ok(Measured::new(
        best_given,
        format!(
            "on its way to {delayed}; all of them receive it, with their events, in {}",
            super::show_ns(best_received)
        ),
    ))
}

/// How many simulated seconds of the crowd are timed.
const CROWD_SECONDS: f64 = 5.0;

fn crowd_of_fifty(kit: &mut Kit) -> Result<Measured, String> {
    let mut best = f64::INFINITY;
    let mut note = String::new();
    for _ in 0..3 {
        let mut sim = city_sim(kit)?;
        crowd(&mut sim)?;
        advance_by(&mut sim, 1.0);
        sim.drain_events();
        let started = std::time::Instant::now();
        advance_by(&mut sim, CROWD_SECONDS);
        let took = started.elapsed().as_nanos() as f64 / CROWD_SECONDS;
        let events = sim.drain_events();
        let count = |name: &str| events.iter().filter(|event| event.name() == name).count();
        if took < best {
            best = took;
            note = format!(
                "{} perceived and {} detected in {CROWD_SECONDS} s; the budget's row is for 300 in a crowd",
                count("perceived"),
                count("detected")
            );
        }
    }
    Ok(Measured::new(best, note))
}

fn party_sweep(kit: &mut Kit) -> Result<Measured, String> {
    let mut sim = city_sim(kit)?;
    let me = create(&mut sim, Place::at(sim_street(20)))?;
    give(&mut sim, me, "core:human_sight")?;
    perception::set_facing(&mut sim, me, Real::of(90.0)).map_err(|reason| reason.to_string())?;
    debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: me,
            control: clash_sim::Control::Direct,
        },
    )?;
    // The first sweep fills the light cache and the map memory.
    let started = std::time::Instant::now();
    perception::search(&mut sim, me, Real::ONE).map_err(|reason| reason.to_string())?;
    let first = started.elapsed().as_nanos() as f64;
    let ns = best_ns(5, 4, || {
        let swept = perception::search(&mut sim, me, Real::ONE);
        let _ = perception::drain_frames(&mut sim);
        swept
    });
    let memory = perception::party::map_memory(&sim, me).map_or(0, |memory| memory.len());
    Ok(Measured::new(
        ns,
        format!(
            "again from where it stands; the first sweep {}; {memory} cells remembered",
            super::show_ns(first)
        ),
    ))
}

fn condition(kit: &mut Kit) -> Result<Measured, String> {
    let mut sim = city_sim(kit)?;
    let me = create(&mut sim, Place::at(sim_street(3)))?;
    let own = sim.subject_value(Subject::Entity(me));
    let clash_expr::Value::Subject(handle) = &own else {
        return Err("a person is not a subject".to_owned());
    };
    let context = Context::new()
        .subject("self", Type::Subject(handle.kind))
        .local("n", Type::Int)
        .local("flag", Type::Bool);
    let source = "self.mass * 2 > $n and $n % 2 == 1 or $flag and $n + 3 < 10";
    let code = compile(
        source,
        sim.defs().expr_registry(),
        &context,
        &Type::Bool.nullable(),
    )
    .into_result()
    .map_err(|errors| format!("`{source}` does not compile: {errors:?}"))?;
    let bound = Bound::new()
        .subject(own)
        .local(clash_expr::Value::Int(7))
        .local(clash_expr::Value::from(false));
    let defs = Arc::clone(sim.defs());
    let mut host = sim.host(bound);
    let mut runner = Runner::new();
    let ns = best_ns(7, 200_000, || runner.on(&defs, &mut host).condition(&code));
    Ok(Measured::new(
        ns,
        format!("{} operations: `{source}`", code.len()),
    ))
}

fn map_frame(_kit: &mut Kit) -> Result<Measured, String> {
    let env = Arc::new(HostEnv::new(mods_dir()));
    let mut host = GameHost::new(env);
    let table = GameHost::table();
    let mut session = Session::new(HostKind::Viewer);
    for line in ["use default seed=3", "gen city size=2"] {
        session
            .run_line(&table, &mut host, line)
            .map_err(|error| format!("`{line}`: {error}"))?;
    }
    let mut frame = Frame::new(200, 60);
    let area = frame.area();
    host.draw_map(&mut Canvas::new(&mut frame), area)
        .map_err(|error| error.to_string())?;
    let ns = best_ns(7, 20, || {
        let mut frame = Frame::new(200, 60);
        let area = frame.area();
        host.draw_map(&mut Canvas::new(&mut frame), area)
            .map(|()| frame.text().len())
    });
    Ok(Measured::new(
        ns,
        "the map of the play screen; the widgets round it are not drawn",
    ))
}

/// The picture the window frontend's benchmark draws into: a screen of
/// full HD.
const WINDOW_PICTURE: (u32, u32) = (1920, 1080);

/// How many glyphs and sprites the window frontend's frame holds in all.
const WINDOW_GLYPHS: usize = 20_000;

/// A frame for the window frontend's benchmark: every cell of the grid a
/// window of [`WINDOW_PICTURE`] shows in the default theme's font,
/// holding characters of every kind in several colours and attributes,
/// and sprites to make [`WINDOW_GLYPHS`] in all, some between cells,
/// larger or partly clear.
fn window_frame_scene(atlas: &GlyphAtlas) -> Frame {
    let (cols, rows) = clash_front_window::grid_size(WINDOW_PICTURE.0, WINDOW_PICTURE.1, atlas);
    let mut frame = Frame::new(cols, rows);
    let chars: Vec<char> = ('!'..='~').chain("  ─│┌┐└┘█░▒▓·•é".chars()).collect();
    let colors = [
        Rgb::new(0xc8, 0xc8, 0xc8),
        Rgb::new(0x10, 0x10, 0x10),
        Rgb::new(0xe0, 0xb0, 0x40),
        Rgb::new(0x50, 0x90, 0xd0),
        Rgb::new(0xa0, 0x18, 0x28),
        Rgb::new(0x60, 0x90, 0x40),
        Rgb::new(0x70, 0x70, 0x70),
    ];
    for (n, cell) in frame.cells.iter_mut().enumerate() {
        cell.ch = chars[(n * 7) % chars.len()];
        cell.fg = Color::from_rgb(colors[n % colors.len()]);
        cell.bg = Color::from_rgb(colors[(n / 3 + 1) % colors.len()]);
        cell.attrs = match n % 13 {
            0 => Attrs::BOLD,
            1 => Attrs::UNDERLINE,
            2 => Attrs::ITALIC,
            3 => Attrs::REVERSE,
            4 => Attrs::DIM,
            _ => Attrs::empty(),
        };
    }
    let sprites = WINDOW_GLYPHS.saturating_sub(frame.cells.len());
    for n in 0..sprites {
        let x = (n * 37 % usize::from(cols)) as f32 + 0.25 * (n % 4) as f32;
        let y = (n * 13 % usize::from(rows)) as f32 + 0.5 * (n % 2) as f32;
        frame.sprites.push(Sprite {
            scale: if n % 10 == 0 { 1.5 } else { 1.0 },
            alpha: if n % 3 == 0 { 0.5 } else { 1.0 },
            layer: (n % 3) as u8,
            ..Sprite::new(
                chars[n % chars.len()],
                Color::from_rgb(colors[n % colors.len()]),
                x,
                y,
            )
        });
    }
    frame
}

fn window_frame(kit: &mut Kit) -> Result<Measured, String> {
    let defs = kit.defs()?;
    let font = clash_front_window::theme_font(&defs, "ui:default")?
        .ok_or("the default theme names no font")?;
    let mut atlas =
        GlyphAtlas::open(&font.path, u32::from(font.size)).map_err(|error| error.to_string())?;
    let mut frame = window_frame_scene(&atlas);
    let mut renderer = Renderer::new(Rgb::BLACK);
    let mut target = PixelBuffer::new(WINDOW_PICTURE.0, WINDOW_PICTURE.1);
    let mut time = |frame: &Frame| {
        renderer.draw(frame, &mut atlas, &mut target);
        best_ns(9, 4, || {
            renderer.draw(frame, &mut atlas, &mut target);
            target.pixels()[0]
        })
    };
    let ns = time(&frame);
    frame.fx.zoom = 1.5;
    frame.fx.flash = Some(Wash {
        color: Rgb::WHITE,
        strength: 0.4,
    });
    frame.fx.shake = 0.5;
    let with_effects = time(&frame);
    let (cell_width, cell_height) = (atlas.cell_width(), atlas.cell_height());
    Ok(Measured::new(
        ns,
        format!(
            "{} by {} cells of {cell_width} by {cell_height} pixels and {} sprites; with zoom, flash and shake {}",
            frame.cols,
            frame.rows,
            frame.sprites.len(),
            super::show_ns(with_effects)
        ),
    ))
}
