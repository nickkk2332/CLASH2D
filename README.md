# CLASH2D

A deterministic simulation engine in Rust for turn-based ASCII immersive
sims, and a small game built with it: a town to live in, wreck, sneak
through and fight in, with superpowers if you want them. Every world,
creature, power, rule and screen is JSON in a mod, so the same engine can
run a superhero city, a horror house or a fantasy dungeon. The engine
provides tools; creators provide the results.

**Status: beta.** You can play it in a window or a terminal, and mod all
of it. What is in the beta, and what a player will trip over, is in the
[release notes](docs/release-notes-beta.md).

What works now:

- A generated market town with streets, houses of several storeys,
  shops, a tavern and a guardhouse. Walls, floors and doors can be
  broken; buildings collapse when their supports go; fire, smoke and
  blasts spread by physical rules.
- People who see and hear (light and noise matter), remember what they
  perceived, talk and pass on what they saw, go about near home, look
  into noises, and run from danger and from anyone they saw hurt
  someone. Others hunt: guards go after anyone they hold hostile or saw
  hurt someone, and search where they last saw them.
- Bodies made of parts and tissues, with wounds, bleeding and pain, and
  no hit points. One bad cut can end a fight.
- Fighting on one shared timeline: blows with a wind-up you can see
  coming and answer (parry, block, dodge, step back), weapons, guns and
  bows, grips, throws and takedowns, quick or chosen in detail, and
  powers with costs, cooldowns and quick slots.
- Doors, windows, locks and keys, wardrobes to hide in, chests and
  cupboards, ovens, hearths, stills and workbenches; making things from
  recipes; eating, drinking and bandaging.
- Three game scenarios, saving and loading, and death that each game
  handles its own way.
- A modding kit: a linter with located errors, a generated reference of
  everything a mod can name, JSON schemas for editors, a scaffold for new
  mods, and test scripts with golden hashes.

## Quick start

You need Rust, installed with [rustup](https://rustup.rs). The file
`rust-toolchain.toml` pins the compiler version, and rustup fetches it
the first time you build. From a clone of this repository:

```bash
cargo run -p clash-app --release
```

The first build takes several minutes. The game opens in a window when
a display is available and in the terminal otherwise. To choose:

```bash
cargo run -p clash-app --release -- --window
cargo run -p clash-app --release -- --terminal
```

`-- --help` lists the other options (`--seed`, `--profile`, `--mods`,
`--data`).

The game is one program, `target/release/clash`, of about 60 MB (49 MB
stripped, 16 MB compressed), and the `mods` directory beside it, under
1 MB. The source is about 5 MB to download, compressed, without its
history. A release build needs about 0.8 GB of disk and took about 15
minutes with two build jobs. The game reaches its title screen in under
a tenth of a second, and a new game in the market town is ready in
about half a second.

**Linux.** The window needs, when the game runs (not to build it), either
Wayland (`libwayland-client` and `libxkbcommon`) or X11 (`libX11`,
`libxcb`, `libXcursor`, `libXrandr`, `libXi` and `libxkbcommon-x11`; on
Debian and Ubuntu, `libxkbcommon-x11-0`). If the window cannot open, the
game says why and plays in the terminal instead. macOS and Windows need
nothing extra. `cargo run -p clash-front-window --example effects` is a
quick check that a machine can show the window at all.

**The terminal** needs at least 80 columns by 24 rows. Leave with the
title screen's Quit, or Control-C.

## The games

New game lists what the loaded mods offer. The three games:

| Game | What it is |
|------|------------|
| **The market town** | The sandbox: a generated town and its people, no goals and no ending. Begin on the north street or in the town square. If you die, you carry on as a newcomer. |
| **The errand** | Take a parcel across the market town to the town hall before nightfall, five hours away. The journal points the way. |
| **Last stand** | Ten at night, three strangers on the lane who mean you harm. Get into the house and hold out until dawn. One save, and death ends it. |

Two places to walk about in are listed beside them: **The town**, a grid
of streets with every door open, and **The lane**, houses between fields.

On the new-game screen, type a name, choose a seed (the same seed makes
the same world), and press Enter on a game.

## Keys

The essentials. `?` in play lists every key from the keymap in force, and
Options rebinds them. [How to play](docs/how-to-play.md) has them all.

| Key | Does |
|-----|------|
| `h` `j` `k` `l` or the arrows; `y` `u` `b` `n` | Walk (the last four diagonally); walk into a door to open it |
| `<` `>` | Go up or down stairs |
| `.` | Wait a second |
| `x` | Look around |
| `e` | Use something within a step: doors, windows, wardrobes, chests, what you hold (eat, drink, bandage) |
| Tab, Shift+Tab | Choose a target among those in sight |
| `f`; `F` | Strike the target; choose the blow, the part and how hard |
| `s`; `S` | Shoot or throw at the target; aim |
| `r`, `t`, `z` | Reload, throw what is in hand, stop |
| `v` | Grab, throw or take down the target |
| `T` | Talk to the target |
| `a`; `1` to `9` | Abilities; use the one on that quick slot |
| `i`, `c` | Inventory; your body and wounds |
| `m`, `J` | Map; journal, with what you are trying to do |
| `M`, `K` | Make something; know-how |
| `?` or F1 | Every key |
| Esc | Pause menu: save, load, options, quit |

Capital letters are Shift with the letter. When a blow or a shot you see
coming is about to land, the game stops and asks how to answer it.

## Saves and settings

Saves, preferences and key bindings live in the user's data directory:
`$XDG_DATA_HOME/clash2d`, or `~/.local/share/clash2d` on Linux;
`~/Library/Application Support/clash2d` on macOS; `%APPDATA%\clash2d` on
Windows. The environment variable `CLASH_DATA_DIR` or the option
`--data dir` puts it elsewhere. Inside it, `saves/` holds one file per
save, `preferences.json` the options and keys, and `mods/` any mods of
your own, which are found before the installed ones.

## Making a mod

Everything the game has is in `mods/`: `core` (the world, people,
items, powers and games), `ui` (the screens) and `core_ui` (core's parts
of those screens). A mod of your own needs no Rust:

```bash
cargo run -p clash-tools -- init-mod hearth
cargo run -p clash-tools -- lint hearth
cargo run -p clash-app -- --profile hearth
```

`init-mod` writes `mods/hearth` with an example of each common kind of
definition and a profile that loads it beside the bundled mods; `lint`
checks it and prints every problem with its file, line and path; the
last line plays it. The [modding guide](docs/modding-guide.md) walks
through an item, a creature, a power, a rule, a screen and a game of your
own. The [reference](docs/reference/README.md) lists every definition
type, field, event, hook, verb, effect, function and property, and is
generated from the engine.

## Tools

`cargo run -p clash-tools -- <command>`; `help` lists them all.

| Command | Does |
|---------|------|
| `lint <profile or dir>` | Load mods as the game does and print every diagnostic; `explain <code>` says what one means |
| `docs --out docs/reference` | Write the modding reference from the engine's registries |
| `schema` | Write JSON Schemas for editors |
| `init-mod <id>`, `new <type> <id>` | Make a mod; print a skeleton definition |
| `show`, `refs`, `find` | Print a finished definition and where each value came from; what refers to one; definitions that meet conditions |
| `scenario run <dir> [--bless]` | Run test scripts four ways and check their golden hashes |
| `worldview` | Look at a world through the console in the terminal |
| `save inspect`, `save verify`, `save list`; `replay` | Read and check save files; replay between two saves |
| `bench` | The benchmarks against `budgets.toml` (use `cargo xtask bench`) |

## Checks

```bash
cargo xtask check
```

runs the gate every change must pass: formatting, clippy with warnings
denied, every test, the documentation, and the check that world sizes
are written in one place. `cargo xtask bench` builds the tools in
release and runs the benchmarks against the budgets in `budgets.toml`;
`cargo xtask bench --check` fails when one is over. The test scripts
under `scenarios/` run, each four ways, with

```bash
cargo run -p clash-tools -- scenario run scenarios
```

(41 scripts, about ten minutes in a debug build).

## Documents

| Read | For |
|------|-----|
| [docs/how-to-play.md](docs/how-to-play.md) | Playing the bundled game: the screen, every key, fighting, sneaking, saving |
| [docs/release-notes-beta.md](docs/release-notes-beta.md) | What is in the beta, and known issues |
| [docs/modding-guide.md](docs/modding-guide.md) | Making a mod, step by step |
| [docs/reference/README.md](docs/reference/README.md) | The modding reference, generated from the engine |
| [docs/00-vision.md](docs/00-vision.md) | What the engine is for, and the scenes it must be able to produce |
| [docs/01-architecture.md](docs/01-architecture.md) | The whole system: layers, rules, determinism, time, data |
| [docs/02-world-model.md](docs/02-world-model.md) | Space, matter, destruction, loading, travel |
| [docs/03-time-and-actions.md](docs/03-time-and-actions.md) | Timeline, clocks, actions, reactions, decisions |
| [docs/04-data-and-modding.md](docs/04-data-and-modding.md) | Mods, profiles, inheritance and patching, diagnostics, hot reload |
| [docs/05-expressions-and-eoc.md](docs/05-expressions-and-eoc.md) | The rule system: expressions, events, effects, queries |
| [docs/06-entities-and-bodies.md](docs/06-entities-and-bodies.md) | Bodies, wounds, minds, items, species and character creation |
| [docs/07-combat.md](docs/07-combat.md) | Strikes, defences, clashes, grapples, takedowns, ranged weapons |
| [docs/08-abilities-and-powers.md](docs/08-abilities-and-powers.md) | What an ability is, costs, suppression, bindings, and a catalogue of powers built from existing parts |
| [docs/09-ai-and-simulation.md](docs/09-ai-and-simulation.md) | How characters decide, navigate and work together; agents, incidents, knowledge, weather |
| [docs/10-world-generation.md](docs/10-world-generation.md) | The map format, placements, ground, settlements, realm plans, ruin, and seeding the world with people |
| [docs/11-vehicles-and-travel.md](docs/11-vehicles-and-travel.md) | Mounts, craft, vessels, the sea, submarines, space, journeys |
| [docs/12-presentation-and-ui.md](docs/12-presentation-and-ui.md) | The application, playback, input, the UI toolkit, frontends |
| [docs/13-scenarios-and-events.md](docs/13-scenarios-and-events.md) | What a game is: scenarios, modes, missions, the director, investigations, death, saving and endings |
| [docs/14-tooling.md](docs/14-tooling.md) | Console, inspector, traces, profiler, test scripts, replay, mod and map tools |
| [docs/15-perception-and-social.md](docs/15-perception-and-social.md) | Senses, stealth, beliefs, disguise, relations, speech |
| [docs/16-devices-and-machines.md](docs/16-devices-and-machines.md) | Interactions, resource grids, automation, crafting, construction, gadgets, powered suits |
| [docs/plans/beta.md](docs/plans/beta.md) | What is being built now, in what order, and the log of what landed |
| [docs/plans/world-model.md](docs/plans/world-model.md) | Work packages for the foundation and the world model |
| [docs/plans/sim-kernel.md](docs/plans/sim-kernel.md) | Work packages for data, rules and actions |
| [docs/plans/first-playable.md](docs/plans/first-playable.md) | Work packages for entities, perception and presentation |
| [docs/plans/combat.md](docs/plans/combat.md) | Work packages for combat |
| [docs/plans/abilities.md](docs/plans/abilities.md) | Work packages for abilities and powers |
| [docs/plans/ai.md](docs/plans/ai.md) | Work packages for AI and the layered simulation |
| [docs/plans/devices.md](docs/plans/devices.md) | Work packages for devices, networks, building and kit |
| [docs/plans/worldgen.md](docs/plans/worldgen.md) | Work packages for world generation and maps |
| [docs/plans/scenarios.md](docs/plans/scenarios.md) | Work packages for scenarios, modes and events |
| [docs/plans/vehicles.md](docs/plans/vehicles.md) | Work packages for vehicles, water and travel |
| [docs/plans/tooling.md](docs/plans/tooling.md) | Work packages for tooling |
| [docs/roadmap.md](docs/roadmap.md) | What is designed and built, and what comes after the beta |
| [docs/decisions.md](docs/decisions.md) | Settled decisions and their reasons |
| [scenarios/README.md](scenarios/README.md) | The test scripts and how they are run |
| [CLAUDE.md](CLAUDE.md) | Working rules for agents contributing to this repository |
