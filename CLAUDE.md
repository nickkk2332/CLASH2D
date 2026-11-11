# Working in this repository

CLASH2D is a deterministic, data-driven simulation engine in Rust: a
genre-neutral kit for ASCII immersive sims, with all content in JSON mods. It
is built by agents working from written specifications. You have no memory of
earlier sessions; the documents are the memory.

## Before you write anything

Read, in this order:

1. `docs/00-vision.md`
2. `docs/01-architecture.md`, especially section 2 (hard rules) and section 4
   (determinism)
3. `docs/decisions.md`
4. The specification for your area (for example `docs/02-world-model.md`)
5. Your work package in `docs/plans/`

## Taking a work package

- Pick a package whose dependencies are all `done`.
- Set it to `in progress` in the plan's status table in your first commit.
- Build what the package lists. Do not build ahead into later packages; leave
  a typed stub and a note instead.
- Set it to `done` in the commit that completes it.

## The specification is the source of truth

If the specification is wrong, unclear or cannot be built as written, fix the
document in the same change set as the code, add an entry to
`docs/decisions.md`, and say so in your description. Never let code and
documents disagree silently.

## Hard rules (summary of 01 §2 and §4)

- **Deterministic.** No hash-map iteration, wall-clock time, OS randomness or
  `std` float transcendentals in simulation crates. Use `LookupMap`,
  `BTreeMap`, `Tick`, the RNG streams and `Real`.
- **No content in the engine.** Never name a specific material, creature or
  power in Rust. Read it from definitions.
- **Mechanism, not meaning.** No engine concept of magic, race, class, era,
  tech level, difficulty or balance. Never scale or soften an outcome. If a
  feature only makes sense in one genre, it belongs in a mod.
- **Belief, not truth.** Character decisions read only what that character
  has perceived.
- **One mutation path.** Cells change only through `WorldEdit`. Occupant
  positions change only through the relocation functions.
- **Events out, never calls up.** A lower crate never depends on a higher one.
- **No panics from content.** Bad mod data is a reported error with file,
  line and path.
- **SI units. Named constants.** No magic numbers for dimensions.
- **No `unsafe`** without a decision entry.
- **No new dependency** without a decision entry.

## A word used two ways

In the plans and in this file, a *scenario* is a scripted regression test
with a golden hash, kept under `scenarios/`. In `docs/13-scenarios-and-events.md`
a *scenario* is a definition type: a game the player can start. Say
"test script" and "game scenario" when it could be unclear.

## Definition of done

- `cargo xtask check` passes (format, clippy with warnings denied, tests,
  docs).
- Every "Done when" item in the package is covered by a test.
- New behaviour that can be scripted has a scenario under `scenarios/` with a
  golden hash, and passes in all runner modes (straight, repeated, save and
  load, different thread count).
- If a golden hash changed, the description says which and why.
- Benchmarks exist for any budget your package touches.
- Public items are documented.
- Every verb, event, hook, property, query and setting the package
  registers states its arguments, subjects and fields, and for a hook what
  a handler may change, and appears in the generated reference. Where the
  specification only names one, write the rest into the specification in
  the same change.
- Nothing a character decides by reads truth. Whatever is reached from a
  goal, a method, a tactic, an action's defaults or an ordinary screen
  compiles in the mind context (05 §7.3).
- Every refusal carries a reason.
- Anything reachable only from Rust that a modder would want is listed as a
  gap in the relevant document's modding surface section.
- Documents and the plan's status table are up to date.

## Commands

Available once work package F1 has landed:

```bash
cargo xtask check
```

```bash
cargo run -p clash-tools -- lint mods
```

```bash
cargo run -p clash-tools -- scenario run scenarios
```

```bash
cargo run -p clash-tools -- worldview
```
