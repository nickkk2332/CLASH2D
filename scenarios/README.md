# Test scripts

This directory holds the engine's **test scripts**: console sessions with
expectations, each ending in a golden state hash. The plans call them
scenarios. They are not the game scenarios of
`docs/13-scenarios-and-events.md`; see the note in `CLAUDE.md`.

The format and the runner are specified in `docs/14-tooling.md` section 7
and built in `crates/clash-sim/src/console`.

## What goes here

One `.scn` file for each behaviour that can be scripted, in a directory
named for its subsystem: `world/`, `combat/`, `console/`. A frame that a
script compares with `expect frame` is kept beside the script.

```
# scenarios/combat/parry_in_time.scn
use profile=test_core seed=7
gen flat size=1
spawn duellist at @c:10,10,0 as=a
advance 400ms
assert "@a.wounds.count == 0"
expect hash 9c41f0aa0f1e5c6b7d8e9fa0b1c2d3e4
```

A line is a console command, or one of the expectations `assert`,
`expect event`, `expect trace`, `expect reason`, `expect hash` and
`expect frame`. A command that is refused fails the script unless the next
line is `expect reason`. JSON in braces may run over several lines.

## How they are run

```bash
cargo run -p clash-tools -- scenario run scenarios
```

Each script is run four ways and must give the same result in all:
straight; twice in one process; with a save and a load at each split
point; with different thread counts. `--bless` rewrites the expected
hashes and frames and changes nothing else in a file. A change that alters
a golden hash says which and why in its description.

Until the simulation is built there is no host for the tool to run scripts
against. It checks every script's syntax, reports mistakes with file, line
and column, says that no host is registered, and exits with code 3.

## `console/`

The scripts under `console/` test the console and the runner themselves.
They use only the commands of the fake host in
`clash_sim::console::testing`, and are run by the tests of `clash-sim`:

```bash
cargo test -p clash-sim --test console
```

To write a new hash or frame into them after a deliberate change:

```bash
BLESS=1 cargo test -p clash-sim --test console the_scripts_under_scenarios
```
