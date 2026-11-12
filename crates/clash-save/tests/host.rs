//! The console's host over the real simulation (14 §3.3): every method
//! of `ConsoleHost` against a game started from the repository's
//! default profile.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::{Dur, Tick};
use clash_expr::{Registry, Type, Value, VerbSpec};
use clash_save::host::{GameHost, HostEnv, host_commands, reasons};
use clash_sim::console::{
    ConsoleError, ConsoleHost, Expression, HostKind, Output, Reference, Session,
};

fn mods_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// A console over a host with nothing running yet.
struct Console {
    table: clash_sim::console::CommandTable,
    host: GameHost,
    session: Session,
}

impl Console {
    fn new() -> Console {
        Console::with_env(Arc::new(HostEnv::new(mods_dir())))
    }

    fn with_env(env: Arc<HostEnv>) -> Console {
        Console {
            table: GameHost::table(),
            host: GameHost::new(env),
            session: Session::new(HostKind::File),
        }
    }

    fn run(&mut self, line: &str) -> Result<Output, ConsoleError> {
        self.session.run_line(&self.table, &mut self.host, line)
    }

    fn ok(&mut self, line: &str) -> Output {
        self.run(line)
            .unwrap_or_else(|error| panic!("`{line}`: {error}"))
    }

    fn refused(&mut self, line: &str) -> String {
        match self.run(line) {
            Err(ConsoleError::Refused(refusal)) => refusal.code,
            other => panic!("`{line}` was not refused: {other:?}"),
        }
    }

    /// Evaluates an expression as the console does, its references
    /// resolved first.
    fn eval(&mut self, source: &str) -> Value {
        self.ok(&format!("eval \"{source}\""))
            .subject
            .expect("an evaluation gives its value")
    }
}

#[test]
fn the_hosts_commands_register_beside_the_built_in_ones_and_the_verbs() {
    let mut table = clash_sim::console::CommandTable::builtin();
    for spec in host_commands() {
        let name = spec.name.clone();
        assert!(!spec.doc.is_empty(), "`{name}` has no documentation");
        for param in &spec.params {
            assert!(
                !param.doc.is_empty(),
                "`{name} {}` has no documentation",
                param.name
            );
        }
        table
            .add(spec)
            .unwrap_or_else(|error| panic!("`{name}`: {error}"));
    }
    // A verb becomes a command with no further code; one that has the
    // name of a built-in command does not.
    let mut registry = Registry::new();
    registry
        .add_verb(VerbSpec::new("ignite").arg("at", Type::Any, "Where."))
        .unwrap();
    registry.add_verb(VerbSpec::new("damage")).unwrap();
    let table = GameHost::table_with(&registry);
    assert!(table.get("ignite").is_some());
    assert!(table.get("spawn").is_some());
    let codes: Vec<&str> = reasons::ALL.iter().map(|(code, _)| *code).collect();
    assert!(codes.contains(&reasons::RULES_UNBOUND));
    assert!(reasons::ALL.iter().all(|(_, doc)| !doc.is_empty()));

    // Through to the host, a verb reaches the rule runtime: one the
    // simulation does not carry out is refused with a reason, an effect
    // that does not compile says why, and one that does runs.
    let mut console = Console::new();
    console.table = table;
    console.ok("use default seed=1");
    console.ok("gen flat size=1");
    assert_eq!(
        console.refused("ignite @here"),
        clash_sim::rule::reason::VERB_UNBOUND
    );
    let refusal = console.host.last_refusal().expect("the refusal is kept");
    assert_eq!(refusal.code, clash_sim::rule::reason::VERB_UNBOUND);
    match console.run(r#"do { "note": { "text": "hello" } }"#) {
        Err(ConsoleError::Failed(message)) => {
            assert!(message.contains("does not compile"), "{message}");
        }
        other => panic!("an unknown effect was not reported: {other:?}"),
    }
    console.ok(r#"do { "let": { "x": 1 } }"#);
}

#[test]
fn a_game_is_started_built_poked_and_read_from_the_console() {
    let mut console = Console::new();
    assert_eq!(console.refused("spawn creature"), reasons::NO_GAME);
    assert_eq!(console.host.now(), Tick::ZERO);

    let started = console.ok("use default seed=21");
    assert!(started.text.contains("started `default` with seed 21"));
    assert_eq!(console.refused("gen flat size=99"), reasons::TOO_LARGE);
    assert_eq!(console.refused("gen nowhere"), reasons::UNKNOWN_KIND);
    let made = console.ok("gen flat size=2");
    assert!(made.text.contains("loaded 4 tracts"), "{}", made.text);

    // References: the cursor, a cell, an entity by number, a name, the
    // active character and the selection.
    let here = console.host.here().expect("gen puts the cursor down");
    assert_eq!(console.eval("@here.x"), Value::Int(i64::from(here.cell.x)));
    console.ok("spawn creature at=@c:5,6,0 control=direct as=hero");
    let spawned = console
        .session
        .records()
        .iter()
        .any(|record| record.name == "entity_created");
    assert!(spawned, "the event is handed over as a record");
    assert_eq!(
        console.eval("self.loc.x"),
        Value::Int(5),
        "`self` is the active character"
    );
    assert_eq!(console.eval("@self == @hero"), Value::Bool(true));
    assert_eq!(console.eval("@e:0 == @hero"), Value::Bool(true));
    assert_eq!(console.eval("@sel == @hero"), Value::Bool(true));
    let unknown = console
        .host
        .resolve(&Reference::Named("nobody".to_owned()))
        .unwrap_err();
    assert!(
        unknown.to_string().contains(reasons::UNKNOWN_NAME),
        "{unknown}"
    );
    let site = console.run("look @site:nowhere").unwrap_err();
    assert!(
        site.to_string().contains(reasons::UNKNOWN_REFERENCE),
        "{site}"
    );

    // Commands that change the world.
    console.ok("move @hero @c:9,6,0");
    assert_eq!(console.eval("@hero.loc.x"), Value::Int(9));
    console.ok("set @c:10,6,0 block core:wall");
    assert_eq!(
        console.eval("@c:10,6,0.block.def_id == 'core:wall'"),
        Value::Bool(true)
    );
    assert_eq!(
        console.refused("set @c:10,6,0 block core:no_such_wall"),
        reasons::UNKNOWN_KIND
    );
    let struck = console.ok("damage @c:10,6,0 2MJ intensity=100MPa");
    assert!(struck.text.contains("block: destroyed"), "{}", struck.text);
    let field = console.ok("field @c:3,3,0 core:smoke 100");
    assert!(
        field.text.contains("core:smoke: 0 to 100"),
        "{}",
        field.text
    );
    let looked = console.ok("look @c:10,6,0");
    assert!(looked.text.contains("core:rubble"), "{}", looked.text);
    assert_eq!(console.refused("remove @c:1,1,0"), reasons::NOT_AN_ENTITY);
    console.ok("spawn item at=@hero as=stone");
    assert_eq!(console.eval("@stone.container == @hero"), Value::Bool(true));

    // Reading.
    let inspected = console.ok("inspect @hero");
    assert!(
        inspected.text.contains("occupant_kind"),
        "{}",
        inspected.text
    );
    let hashes = console.ok("hash all");
    assert!(hashes.text.contains("total"), "{}", hashes.text);
    assert_eq!(
        console.ok("hash total").text,
        console.host.state_hash().to_string()
    );
    assert_eq!(console.refused("hash weather"), reasons::UNKNOWN_SUBSYSTEM);
    assert_eq!(console.ok("check").text, "2 checks, no violations");
    assert_eq!(console.refused("diff first second"), reasons::NOT_BUILT);

    // Time.
    console.ok("advance 1500ms");
    assert_eq!(console.host.now(), Tick::ZERO + Dur::from_millis(1500));
    let stepped = console.ok("step");
    assert!(stepped.text.contains("wake-ups"), "{}", stepped.text);
    let until = Expression::parse("now() >= 3").unwrap();
    assert!(
        console
            .host
            .advance_until(&until, None, Dur::from_secs(10))
            .unwrap()
    );

    // The frame: the storey round the cursor, with the hero on it.
    console.ok("size 21 9");
    console.ok("look @hero");
    let frame = console.ok("frame").text;
    let rows: Vec<&str> = frame.lines().skip(1).collect();
    assert_eq!(rows.len(), 9, "{frame}");
    assert_eq!(rows[4].chars().nth(10), Some('@'), "{frame}");
    assert!(console.host.frame().unwrap().starts_with("clash-frame"));
}

#[test]
fn save_and_load_are_invisible_and_carry_the_consoles_marks() {
    let mut console = Console::new();
    console.ok("use default seed=8");
    console.ok("gen flat size=1");
    console.ok("spawn creature at=@c:4,4,0 control=direct as=hero");
    console.ok("field @c:6,6,0 core:smoke 200");
    console.ok("look @c:7,7,0");
    console.ok("advance 1s");
    let bytes = console.host.save().unwrap();
    let saved = console.host.state_hash();
    console.ok("advance 3s");
    let straight = console.host.state_hash();

    // Into a fresh host over the same environment.
    let env = Arc::new(HostEnv::new(mods_dir()));
    let mut other = Console::with_env(env);
    other.host.load(&bytes).unwrap();
    assert_eq!(other.host.state_hash(), saved);
    assert_eq!(other.host.here(), {
        let mut here = console.host.here().unwrap();
        here.cell = clash_world::CellPos::new(7, 7, 0);
        Some(here)
    });
    assert_eq!(
        other.eval("self.loc.x"),
        Value::Int(4),
        "the active character"
    );
    other.ok("advance 3s");
    assert_eq!(other.host.state_hash(), straight);

    // The thread count changes nothing.
    other.host.set_threads(3).unwrap();
    assert!(other.host.set_threads(0).is_err());
    other.ok("advance 1s");
    console.ok("advance 1s");
    assert_eq!(other.host.state_hash(), console.host.state_hash());

    // And through a file.
    let dir = std::env::temp_dir().join(format!("clash-save-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("game.c2save");
    console
        .host
        .save_to(&file, "At the console", "smoke")
        .unwrap();
    let mut third = Console::new();
    let meta = third.host.load_from(&file).unwrap();
    assert_eq!(meta.name, "At the console");
    assert_eq!(meta.seed, 8);
    assert_eq!(third.host.state_hash(), console.host.state_hash());
    assert!(third.host.load_from(dir.join("missing.c2save")).is_err());
    assert!(third.host.load(b"not a save").is_err());
    let _ = std::fs::remove_dir_all(&dir);

    // No game saves as no game.
    let mut empty = Console::new();
    let none = empty.host.save().unwrap();
    console.host.load(&none).unwrap();
    assert!(console.host.sim().is_none());
}

#[test]
fn a_save_made_while_a_tower_falls_goes_on_to_the_same_end() {
    let mut console = Console::new();
    console.ok("use default seed=11");
    console.ok("gen flat size=1");
    let cells = [(10, 10), (11, 10), (10, 11), (11, 11)];
    for z in 0..3 {
        for (x, y) in cells {
            console.ok(&format!("set @c:{x},{y},{z} block core:wall"));
        }
    }
    for z in 1..4 {
        for (x, y) in cells {
            console.ok(&format!("set @c:{x},{y},{z} floor core:slab"));
        }
    }
    console.ok("advance 1s");
    for (x, y) in cells {
        console.ok(&format!("set @c:{x},{y},0 block none"));
    }
    // As the test script tower_pancake does, to its third split point
    // in the fall.
    for _ in 0..13 {
        console.ok("advance 50ms");
    }
    let falling = console
        .host
        .sim()
        .unwrap()
        .truth()
        .cosmos()
        .falling_masses()
        .count();
    assert!(falling > 0, "the save is made in the middle of the fall");
    let bytes = console.host.save().unwrap();
    console.ok("advance 10s");

    let mut other = Console::new();
    other.host.load(&bytes).unwrap();
    other.ok("advance 10s");
    assert_eq!(other.host.state_hash(), console.host.state_hash());
    assert!(
        other
            .session
            .records()
            .iter()
            .any(|record| record.name == "collapse_settled")
    );
}
