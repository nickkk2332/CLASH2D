//! The load-time checks of maps (10 §3.7): each mistake has a fixture
//! here and comes out as a diagnostic that says where it is.

mod common;

use clash_data::Diagnostics;
use clash_expr::{FunctionSpec, Registry, Type};
use clash_gen::map::MapLimits;
use common::{build, build_errors, build_errors_with, column_of, line_of, load, the_one};

/// A diagnostic of the build stage is attributed to the map it is in.
fn in_map(diagnostics: &Diagnostics, needle: &str, map: &str) {
    let diagnostic = the_one(diagnostics, needle);
    let label = diagnostic
        .definition
        .as_ref()
        .unwrap_or_else(|| panic!("no definition on:\n{}", diagnostic.render()));
    assert_eq!(label.id, format!("t:{map}"), "{}", diagnostic.render());
    assert!(diagnostic.is_error());
}

#[test]
fn a_row_of_the_wrong_length_is_reported_at_the_row() {
    let maps = r##"[
  { "type": "map", "id": "short_row", "size": [4, 2],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "WWWW",
                       "WWW" ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "this row has 3 characters", "short_row");
    let diagnostic = the_one(&diagnostics, "this row has 3 characters");
    let location = diagnostic.location.as_ref().unwrap();
    assert_eq!(location.line, line_of(maps, "\"WWW\""));
    assert_eq!(location.column, column_of(maps, "\"WWW\""));
    assert_eq!(diagnostic.path, "levels.0[1]");
}

#[test]
fn a_level_with_the_wrong_number_of_rows_is_reported() {
    let maps = r##"[
  { "type": "map", "id": "few_rows", "size": [2, 3],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "WW", "WW" ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(
        &diagnostics,
        "level 0 has 2 rows, and the map is 3 deep",
        "few_rows",
    );
}

#[test]
fn an_unknown_character_is_reported_at_the_character() {
    let maps = r##"[
  { "type": "map", "id": "stray", "size": [4, 1],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "WWxW" ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "unknown character `x`", "stray");
    let location = the_one(&diagnostics, "unknown character `x`")
        .location
        .clone()
        .unwrap();
    assert_eq!(location.line, line_of(maps, "\"WWxW\""));
    // The quote, two characters, then the stray one.
    assert_eq!(location.column, column_of(maps, "\"WWxW\"") + 3);
}

#[test]
fn a_parameter_a_state_names_and_the_map_lacks_is_unresolved() {
    let maps = r##"[
  { "type": "map", "id": "typo", "size": [1, 1],
    "params": { "wall": { "pick": ["stone", "oak"] } },
    "palette": [ { "W": "wall@$wal" } ],
    "levels": { "0": [ "W" ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "unresolved parameter `$wal`", "typo");
    let diagnostic = the_one(&diagnostics, "unresolved parameter `$wal`");
    assert!(
        diagnostic
            .suggestion
            .as_deref()
            .is_some_and(|text| text.contains("wall")),
        "{}",
        diagnostic.render()
    );
    let location = diagnostic.location.as_ref().unwrap();
    assert_eq!(location.column, column_of(maps, "$wal\""));
}

#[test]
fn a_parameter_an_expression_names_and_the_map_lacks_fails_to_load() {
    let maps = r##"[
  { "type": "map", "id": "typo", "size": [1, 1],
    "params": { "wealth": 2 },
    "if": "$welth > 1",
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ] } }
]"##;
    let (defs, diagnostics) = load(maps, None);
    assert!(defs.is_none(), "the map loaded");
    let diagnostic = the_one(&diagnostics, "welth");
    assert!(diagnostic.is_error());
    assert_eq!(
        diagnostic.location.as_ref().unwrap().line,
        line_of(maps, "$welth")
    );
}

#[test]
fn a_value_a_parameter_can_take_must_name_something() {
    let maps = r##"[
  { "type": "map", "id": "paper_house", "size": [1, 1],
    "params": { "wall": { "pick": ["stone", "paper"] } },
    "palette": [ { "W": "wall@$wall" } ],
    "levels": { "0": [ "W" ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(
        &diagnostics,
        "with `$wall` as `paper`, `wall@$wall` is `wall@paper`",
        "paper_house",
    );
}

#[test]
fn a_state_that_names_nothing_is_reported() {
    let maps = r##"[
  { "type": "map", "id": "ghost", "size": [3, 1],
    "palette": [ { "a": { "block": "rampart" }, "b": "door#ajar", "c": "nothing_at_all" } ],
    "levels": { "0": [ "abc" ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "there is no block `rampart`", "ghost");
    in_map(&diagnostics, "has no state `ajar`", "ghost");
    in_map(
        &diagnostics,
        "there is no floor, block or fixture `nothing_at_all`",
        "ghost",
    );
}

#[test]
fn a_short_form_that_could_be_two_layers_asks_for_the_long_one() {
    let maps = r##"[
  { "type": "map", "id": "doormat", "size": [1, 1],
    "palette": [ { "m": "mat" } ],
    "levels": { "0": [ "m" ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(
        &diagnostics,
        "`mat` is both a floor and a fixture",
        "doormat",
    );
    let diagnostic = the_one(&diagnostics, "`mat` is both a floor and a fixture");
    assert!(diagnostic.message.contains("Write the long form"));
    assert_eq!(
        diagnostic.location.as_ref().unwrap().column,
        column_of(maps, "\"mat\"")
    );

    // The long form says which, and loads.
    let fixed = maps.replace("\"m\": \"mat\"", "\"m\": { \"fixture\": \"mat\" }");
    assert_eq!(build(&fixed).len(), 1);
}

#[test]
fn a_nested_map_that_cannot_fit_is_reported() {
    let maps = r##"[
  { "type": "map", "id": "big", "size": [5, 2],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "WWWWW", "WWWWW" ] } },
  { "type": "map", "id": "small", "size": [3, 3],
    "palette": [ { ".": { "floor": "boards" } } ],
    "levels": { "0": [ "...", "...", "..." ] },
    "place": [ { "map": "big", "at": [0, 0, 0] } ] },
  { "type": "map", "id": "low", "size": [5, 5],
    "palette": [ { ".": { "floor": "boards" } } ],
    "levels": { "0": [ ".....", ".....", ".....", ".....", "....." ] },
    "place": [ { "map": "tall", "at": [0, 0, 0] } ] },
  { "type": "map", "id": "tall", "size": [1, 1],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ], "1": [ "W" ] } },
  { "type": "map", "id": "edge", "size": [5, 5],
    "palette": [ { ".": { "floor": "boards" } } ],
    "levels": { "0": [ ".....", ".....", ".....", ".....", "....." ] },
    "place": [ { "map": "big", "at": [3, 0, 0], "turn": "any" } ] }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "the nested map `t:big` cannot fit", "small");
    in_map(&diagnostics, "the nested map `t:tall` cannot fit", "low");
    // Turned a quarter, `big` is 2 by 5 and fits at [3, 0].
    assert_eq!(diagnostics.len(), 2, "{}", diagnostics.render());
    let location = the_one(&diagnostics, "the nested map `t:big` cannot fit")
        .location
        .clone()
        .unwrap();
    assert_eq!(
        location.line,
        line_of(maps, "\"place\": [ { \"map\": \"big\"")
    );
}

#[test]
fn a_map_that_nests_inside_itself_is_reported() {
    let maps = r##"[
  { "type": "map", "id": "a", "size": [2, 2],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "WW", "WW" ] },
    "place": [ { "map": "b", "at": [0, 0, 0] } ] },
  { "type": "map", "id": "b", "size": [2, 2],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "WW", "WW" ] },
    "place": [ { "map": "a", "at": [0, 0, 0] } ] },
  { "type": "map", "id": "narcissus", "tags": ["pool"], "size": [1, 1],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ] },
    "place": [ { "map": { "tag": "pool" }, "at": [0, 0, 0] } ] }
]"##;
    let diagnostics = build_errors(maps);
    in_map(
        &diagnostics,
        "a map nests inside itself: t:a → t:b → t:a",
        "b",
    );
    in_map(
        &diagnostics,
        "a map nests inside itself: t:narcissus → t:narcissus",
        "narcissus",
    );
}

#[test]
fn maps_nested_deeper_than_the_limit_are_reported() {
    let maps = r##"[
  { "type": "map", "id": "d0", "size": [1, 1], "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ] }, "place": [ { "map": "d1", "at": [0, 0, 0] } ] },
  { "type": "map", "id": "d1", "size": [1, 1], "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ] }, "place": [ { "map": "d2", "at": [0, 0, 0] } ] },
  { "type": "map", "id": "d2", "size": [1, 1], "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ] }, "place": [ { "map": "d3", "at": [0, 0, 0] } ] },
  { "type": "map", "id": "d3", "size": [1, 1], "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ] } }
]"##;
    // Three deep is within the default of eight.
    assert_eq!(build(maps).len(), 4);
    let diagnostics = build_errors_with(maps, MapLimits { max_nesting: 2 });
    in_map(
        &diagnostics,
        "maps nest 3 deep below `t:d0`, and `gen.max_nesting` is 2",
        "d0",
    );
    assert_eq!(diagnostics.len(), 1, "{}", diagnostics.render());
}

#[test]
fn a_link_to_a_device_no_cell_names_is_reported() {
    let maps = r##"[
  { "type": "map", "id": "vault", "size": [3, 1],
    "palette": [ { "l": { "fixture": "table", "device": "lever_a" },
                   "d": { "block": "door", "device": { "name": "vault_door" } },
                   ".": {} } ],
    "levels": { "0": [ "l.d" ] },
    "wiring": {
      "palette": { "=": ["cable"] },
      "levels": { "0": [ "===" ] },
      "links": [ { "from": "lever_a", "to": "vault_door", "resource": "control" },
                 { "from": "lever_b", "to": "vault_door" } ] } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "this link runs from `lever_b`", "vault");
    assert_eq!(diagnostics.len(), 1, "{}", diagnostics.render());
    let diagnostic = the_one(&diagnostics, "this link runs from `lever_b`");
    assert_eq!(diagnostic.path, "wiring.links[1].from");
    assert!(
        diagnostic
            .suggestion
            .as_deref()
            .is_some_and(|text| text.contains("lever_a"))
    );
}

#[test]
fn wiring_is_held_to_the_shape_of_the_map() {
    let maps = r##"[
  { "type": "map", "id": "wired", "size": [3, 1],
    "palette": [ { ".": {} } ],
    "levels": { "0": [ "..." ] },
    "wiring": {
      "palette": { "=": ["cable"] },
      "levels": { "0": [ "=~=" ], "1": [ "===" ] } } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(
        &diagnostics,
        "unknown character `~`: the wiring palette",
        "wired",
    );
    in_map(&diagnostics, "wiring is drawn on level 1", "wired");
}

#[test]
fn a_joint_off_the_boundary_is_reported() {
    let maps = r##"[
  { "type": "map", "id": "room", "size": [3, 3],
    "palette": [ { "W": "wall", ".": {} } ],
    "levels": { "0": [ "WWW", "W.W", "WWW" ] },
    "joints": [ { "id": "ok", "at": [1, 2, 0], "dir": "south", "kind": "doorway" },
                { "id": "inside", "at": [1, 1, 0], "dir": "south", "kind": "doorway" },
                { "id": "wrong_side", "at": [1, 0, 0], "dir": "east", "kind": "doorway" },
                { "id": "hatch", "at": [1, 1, 0], "dir": "up", "kind": "hatch" },
                { "id": "out", "at": [1, 3, 0], "dir": "south", "kind": "doorway" } ] }
]"##;
    let diagnostics = build_errors(maps);
    in_map(
        &diagnostics,
        "this joint opens south from [1, 1, 0], which is not on that edge",
        "room",
    );
    in_map(
        &diagnostics,
        "this joint opens east from [1, 0, 0], which is not on that edge",
        "room",
    );
    in_map(
        &diagnostics,
        "the joint is at [1, 3], outside the map",
        "room",
    );
    // The one on the south edge and the hatch through the only storey
    // are sound.
    assert_eq!(diagnostics.len(), 3, "{}", diagnostics.render());
}

#[test]
fn what_a_map_refers_to_must_be_there() {
    let maps = r##"[
  { "type": "map", "id": "loose", "size": [2, 2],
    "palette": [ { ".": { "floor": "boards", "clear": ["fixtures"] },
                   "k": { "area": "cellar" } } ],
    "levels": { "0": [ "..", ".k" ], "2": "attic" },
    "marks": { "door": [5, 0, 0] },
    "anchor": "mark:dor",
    "areas": [ { "kind": "ballroom", "box": [[0, 0], [1, 1]] } ],
    "spawns": [ { "creature": "rat", "in": "area:pantry" } ],
    "place": [ { "map": { "tag": "no_such_tag" }, "at": [0, 0, 0] } ],
    "front": "north_east",
    "sit": "float" }
]"##;
    let diagnostics = build_errors(maps);
    for needle in [
        "`fixtures` is not a layer `clear` takes",
        "there is no area_kind `cellar`",
        "`attic` is not a preset",
        "the mark `door` is at [5, 0], outside the map",
        "the map has no mark `dor`",
        "there is no area_kind `ballroom`",
        "the map has no area `pantry`",
        "no map carries the tag `no_such_tag`",
        "a map's front is one of its four sides",
        "`float` is not a way to sit",
    ] {
        in_map(&diagnostics, needle, "loose");
    }
}

#[test]
fn a_palette_key_is_one_character_and_never_a_space() {
    let maps = r##"[
  { "type": "palette", "id": "shared", "entries": { "ab": "wall" } },
  { "type": "map", "id": "keys", "size": [1, 1],
    "palette": [ { " ": "wall", "W": "wall" } ],
    "levels": { "0": [ "W" ] } }
]"##;
    let diagnostics = build_errors(maps);
    the_one(&diagnostics, "`ab` is not one");
    in_map(&diagnostics, "a space always means", "keys");
}

#[test]
fn a_nested_map_is_given_only_its_own_parameters_and_turned_only_if_it_may_be() {
    let maps = r##"[
  { "type": "map", "id": "fixed", "size": [2, 1], "turn": false,
    "params": { "wall": { "default": "stone" } },
    "palette": [ { "W": "wall@$wall" } ],
    "levels": { "0": [ "WW" ] } },
  { "type": "map", "id": "outer", "size": [3, 3],
    "palette": [ { ".": { "floor": "boards" } } ],
    "levels": { "0": [ "...", "...", "..." ] },
    "place": [ { "map": "fixed", "at": [0, 0, 0], "params": { "wal": "'oak'" } },
               { "map": "fixed", "at": [0, 1, 0], "turn": 90 } ] }
]"##;
    let diagnostics = build_errors(maps);
    in_map(
        &diagnostics,
        "the map `t:fixed` has no parameter `wal`",
        "outer",
    );
    in_map(
        &diagnostics,
        "the map `t:fixed` says `\"turn\": false` and is placed turned",
        "outer",
    );
}

#[test]
fn a_function_only_a_running_simulation_answers_is_refused() {
    // A registry as the simulation fills it: `now` is answered by the
    // host, not worked out from its arguments.
    let mut registry = Registry::with_builtins();
    registry
        .add_function(FunctionSpec::new("now", Type::Real))
        .unwrap();
    let maps = r##"[
  { "type": "map", "id": "clock", "size": [1, 1],
    "palette": [ { "t": { "fixture": "table",
                          "items": { "item": "coin", "chance": "now() / 100" } } } ],
    "levels": { "0": [ "t" ] } }
]"##;
    let (defs, diagnostics) = load(maps, Some(&registry));
    assert!(defs.is_none(), "the map loaded");
    let diagnostic = the_one(&diagnostics, "`now` is answered by a running simulation");
    assert!(diagnostic.is_error());
    let location = diagnostic.location.as_ref().unwrap();
    assert_eq!(location.line, line_of(maps, "now()"));
    assert_eq!(location.column, column_of(maps, "now()"));

    // The random functions are allowed: they draw from the placement.
    let random = maps.replace("now() / 100", "rand(0, 1)");
    let (defs, diagnostics) = load(&random, Some(&registry));
    assert!(defs.is_some(), "{}", diagnostics.render());
}

#[test]
fn the_linters_check_reports_the_same_and_is_quiet_for_sound_maps() {
    let bad = r##"[
  { "type": "map", "id": "stray", "size": [2, 1],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W?" ] } }
]"##;
    let (defs, _) = load(bad, None);
    let diagnostics = clash_gen::check_defs(&defs.unwrap());
    the_one(&diagnostics, "unknown character `?`");

    let good = bad.replace("W?", "WW");
    let (defs, _) = load(&good, None);
    assert!(clash_gen::check_defs(&defs.unwrap()).is_empty());
}

#[test]
fn a_map_with_no_sound_shape_is_reported_and_does_not_panic() {
    let maps = r##"[
  { "type": "map", "id": "flat", "size": [0, 3],
    "palette": [ { "W": "wall" } ],
    "levels": { "ground": [ "W" ] } },
  { "type": "map", "id": "lid", "size": [1, 1],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": "roof" } }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "`size` is `[width, depth]`", "flat");
    in_map(&diagnostics, "`ground` is not one", "flat");
    in_map(&diagnostics, "a roof covers the storey below it", "lid");
}

#[test]
fn a_map_says_what_its_roof_is_and_how_large_it_may_be() {
    let maps = r##"[
  { "type": "map", "id": "bare", "size": [1, 1],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ], "1": "roof" } },
  { "type": "map", "id": "vast", "size": [512, 512],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [], "8": [] } },
  { "type": "map", "id": "dot", "size": [1, 1],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "W" ] } },
  { "type": "map", "id": "twice", "size": [2, 2],
    "palette": [ { "W": "wall" } ],
    "levels": { "0": [ "WW", "WW" ] },
    "place": [ { "map": "dot", "at": [0, 0, 0], "count": 3 } ] }
]"##;
    let diagnostics = build_errors(maps);
    in_map(&diagnostics, "this level is a roof", "bare");
    in_map(&diagnostics, "a map has at most 1048576", "vast");
    in_map(&diagnostics, "`count` and `apart` scatter copies", "twice");
    assert_eq!(diagnostics.len(), 3, "{}", diagnostics.render());
}

#[test]
fn a_parameter_is_held_to_its_type() {
    let maps = r##"[
  { "type": "map", "id": "muddle", "size": [1, 1],
    "params": { "n": { "expr": "1 > 0", "type": "int" },
                "m": { "pick": [1, "two"] },
                "k": { "default": "three", "type": "int" },
                "r": { "range": [5, 1] },
                "q": { "type": "list" },
                "post": 3 },
    "palette": [ { "W": "wall@$post" } ],
    "levels": { "0": [ "W" ] } }
]"##;
    let diagnostics = build_errors(maps);
    for needle in [
        "the parameter `$n` is declared a int, and its expression gives a bool",
        "the parameter `$m` is a real, and `two` is a string",
        "the parameter `$k` is a int, and `three` is a string",
        "`range` is `[low, high]`, two numbers with `low` first",
        "`list` is not a type a parameter can have",
        "the parameter `$post` is a int, and a state is written with a text",
    ] {
        in_map(&diagnostics, needle, "muddle");
    }
    assert_eq!(diagnostics.len(), 6, "{}", diagnostics.render());
}
