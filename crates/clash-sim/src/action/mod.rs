//! Actions (03 §6 to §8): what a character does over time.
//!
//! An action is a small state machine of **phases**, each with a
//! duration. It occupies **channels** while it runs, it may open a
//! reaction **window** when a phase starts, and with **roles** it has
//! several participants. This module holds the definitions: the
//! `action`, `channel` and `window_kind` definition types, loaded,
//! checked and compiled by the mod loader. Running them is the runtime's
//! (packages T3 to T6 of `docs/plans/sim-kernel.md`), which is built
//! beside this module and reads what is here.
//!
//! | Part | What it is |
//! |------|------------|
//! | [`ChannelSet`], [`ActionDefId`], [`ChannelId`], [`WindowKindId`], [`PhaseIndex`] | The ids, and channels as a bitset. They depend on nothing else in the crate. |
//! | [`ActionDef`], [`ChannelDef`], [`WindowKindDef`] | The definition types. |
//! | [`Phase`], [`Transition`], [`PhaseWindow`], [`Tick`], [`Param`], [`Var`], [`Role`], [`ChannelUse`] | What an action is made of. |
//! | [`Effects`] | A list of effects, compiled by the rule runtime ([`rule`](crate::rule)). |
//! | [`reason`] | The reasons the action system gives when it declines (14 §5). |
//!
//! # Loading
//!
//! [`register_defs`] registers the three types with the loader. A
//! definition is then read, resolved and compiled in place, in three
//! steps, and a mistake at any of them is a diagnostic with file, line,
//! column and JSON path:
//!
//! 1. **Read.** Keys and kinds of value; references to channels and
//!    window kinds resolve as they are read.
//! 2. **Resolve.** Phase ids become positions: every `goto`, every
//!    window's `closes`, a reaction's `ready`. Every phase must be
//!    reachable from the first and every window must be able to close.
//!    Names of roles, parameters and variables must be distinct.
//! 3. **Compile.** Types of parameters and variables are read, the
//!    locals are laid out in slots, and every expression and every list
//!    of effects is compiled against that one layout. What each field
//!    may read is in the table on [`ActionDef`].
//!
//! What comes out is immutable and is the same struct the registry
//! holds: `defs.get::<ActionDef>(id)`.
//!
//! # How the runtime walks a compiled action
//!
//! Nothing below looks anything up by name. The steps are 03 §6.6; the
//! refusals are the constants of [`reason`].
//!
//! **Starting.** Given an [`ActionDefId`], an actor and the parameters a
//! command supplied:
//!
//! 1. Make one value for each of [`ActionDef::locals`], in order. That
//!    vector is the action instance's locals for as long as it runs, and
//!    its positions are the slots every expression was compiled with.
//!    The one bound subject, `self`, is the actor, at slot 0.
//!    - [`LocalSource::Window`]: the window being answered, or null.
//!    - [`LocalSource::Role`]: the participant. Role 0 is the actor; the
//!      others are given by name like parameters
//!      ([`reason::ROLE_UNFILLED`]).
//!    - [`LocalSource::Param`]: the value given, or else the parameter's
//!      `default`, evaluated now in the actor's mind, in order, so that a
//!      default may read those before it. None of either is
//!      [`reason::PARAMETER_MISSING`]. A list is held to its
//!      [`ListBounds`] ([`reason::LIST_LENGTH`]).
//!    - [`LocalSource::Var`]: the variable's constant, or null.
//! 2. If [`ActionDef::footing`] and nothing is under the actor:
//!    [`reason::NO_FOOTING`].
//! 3. Evaluate [`ActionDef::requires`] against truth
//!    ([`reason::REQUIRES`], with `fail_text` as the sentence).
//! 4. Raise `ActionStarting`; a handler may change parameters, or
//!    cancel ([`reason::VETOED`]).
//! 5. Claim slots. For an ordinary action, [`ActionDef::uses`] on the
//!    actor; for a joint one, each [`Role::uses`] on its participant.
//!    [`ChannelUse::set`] against the set of channels the participant's
//!    running actions hold tells at once whether slots need counting at
//!    all; [`ChannelUse::needs`] says how many of each
//!    ([`reason::CHANNEL_BUSY`], [`reason::CHANNEL_LACKING`]).
//! 6. Evaluate [`ActionDef::requires_at_start`] against truth. If it
//!    fails the action has begun and is interrupted at once
//!    ([`reason::START_CONDITION`]).
//! 7. Run `on_start`, then enter phase 0.
//!
//! Where 03 §6.6 leaves the order of two of these open, the runtime
//! package settles it there; nothing in a definition depends on it.
//!
//! **Entering a phase** at a [`PhaseIndex`], with [`ActionDef::phase`]:
//!
//! 1. Open [`Phase::window`], if any: its kind, those its `for`
//!    expressions give, and the index of the phase whose start closes
//!    it.
//! 2. Run the phase's `do`.
//! 3. For [`PhaseTime::For`], evaluate the duration and divide it by
//!    [`ActionDef::tempo`] unless the phase is `fixed`; raise
//!    `PhaseTiming`; schedule one wake-up that far ahead on the actor's
//!    clock. Zero does not schedule: the phase ends in the same tick,
//!    counted against `rules.max_transitions`
//!    ([`reason::TRANSITION_LIMIT`]). For [`PhaseTime::Hold`], schedule
//!    `max` if there is one.
//! 4. Schedule the first tick, [`Tick::every`] ahead, if the phase has
//!    one. At each tick run its effects, then test `until`.
//!
//! **Leaving a phase**: run `on_exit`, then take the first of
//! [`Phase::next`] whose condition holds, by [`Transition::to`]. If none
//! holds, take [`Phase::after`]. [`Next::Phase`] is entered;
//! [`Next::Finish`] runs `on_finish` and releases the slots.
//!
//! **Ending early.** A `Cancel` command is allowed by the current
//! phase's [`CancelRule`] or refused ([`reason::CANNOT_CANCEL`],
//! [`reason::CANCEL_ONLY_FOR`]); on a held phase it is release. The
//! `interrupt` effect is ignored while [`Phase::uninterruptible`].
//!
//! **Running effects.** An effect list is run with
//! [`RuleSet::run_effects`](crate::rule::RuleSet::run_effects): the one
//! subject is the actor, and the locals are the instance's, which `set`
//! and `add` change in place.
//!
//! **Saving.** An instance is its [`ActionDefId`], its [`PhaseIndex`],
//! its locals and its wake-ups. All of them are plain values.
//!
//! # What is not here yet
//!
//! - The fields later documents add to an action: `attacks` and
//!   `defence` (07), `cutscene` (12), the `choice` parameter type
//!   (08 §9.2). A definition that writes one is told the key is unknown.
//! - Costs, cooldowns and ranks belong to grants (08 §5.1), not to the
//!   action.
//! - The events, hooks, effects and queries of 03 §13 are registered by
//!   the runtime packages that raise and run them.

pub mod def;
mod ids;
pub mod reason;

use clash_data::DefTypes;

pub use def::{
    ACTOR_SUBJECT, ActionDef, CancelRule, ChannelDef, ChannelNeed, ChannelUse, Effects, ListBounds,
    Local, LocalSource, MIN_ROLES, Next, Param, Phase, PhaseRef, PhaseTime, PhaseWindow, Role,
    Slots, Tick, Transition, Var, WINDOW_LOCAL, WindowKindDef,
};
pub use ids::{ActionDefId, ChannelId, ChannelSet, PhaseIndex, WindowKindId};

/// Registers the definition types of the simulation crate with the mod
/// loader: `action`, `channel` and `window_kind` (03 §13).
///
/// This is the crate's one registration function. The application and
/// the tools call it beside each other crate's function of the same
/// name, and a package that adds a definition type to this crate adds it
/// here.
///
/// The three types add nothing to the expression registry themselves:
/// the loader makes the ids of every definition type known to
/// expressions, so `id<action>` and `id<channel>` are types as soon as
/// the types are registered.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<ChannelDef>()
        .register::<WindowKindDef>()
        .register::<ActionDef>();
}
