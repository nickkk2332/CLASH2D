//! Input beyond the toolkit (12 §9): controls, putting a command
//! together with pickers, and the screens for decisions.
//!
//! # From a key to a command
//!
//! 1. The keymap in force names the key in the context of the screen on
//!    top (12 §9.2): `h` is `west` in the context `play`.
//! 2. The name is offered to the widget with the focus, to the hotkeys
//!    of menus, to the screen's own `keys`, and then to the **controls**:
//!    a [`ControlDef`] of that name has its `do` run. Only then does the
//!    toolkit use the name itself.
//! 3. A control's `act` effect names an action. With parameters given it
//!    is submitted at once. With `"needed"` or `"all"` the scene reads
//!    the action's parameter list ([`ask`]), fills in defaults through
//!    the host, opens a picker for each parameter that is to be asked
//!    for, and submits one whole [`UiCommand`](crate::UiCommand).
//!    Closing a picker abandons the command: nothing is submitted.
//!
//! Rebinding a key changes step 1 and nothing else
//! ([`Scene::rebind`](crate::Scene::rebind)).
//!
//! # Pickers
//!
//! A picker is a screen, named by the profile for the role
//! `picker.<name>`. Which picker asks for a parameter is said by the
//! `act` effect's `pick`, or else by the parameter's type:
//!
//! | Type | Picker | Its screen is given |
//! |------|--------|---------------------|
//! | `entity` | `entity` | A cursor over the map that cycles through what could be chosen |
//! | `cell` (`loc`) | `cell` | A cursor over the map |
//! | A list of cells | `route`, or else `cell` | The same; each answer is one more step |
//! | Any other kind of subject | Its kind's name, or else `list` | `options` |
//! | `int`, `real`, `number` | `number` | `value`, and its variable `value` filled in |
//! | `string` | `text` | The same |
//! | A list of anything else | `multi` | `options`, each with `chosen`; `min` and `max` |
//! | Anything else | `list` | `options` |
//!
//! Every picker is opened with `action` (what the action is called),
//! `param` (the parameter's name), `title` (the two together), `value`
//! (what it is filled in with, or null) and `options` (a list of maps
//! with `label`, `value` and `chosen`), of which it declares the ones it
//! has a use for. It answers with the `answer` effect; an answer the
//! parameter cannot hold is refused with the reason and the picker
//! stays. For a list parameter each answer is one more entry, and the
//! `picker` effect finishes the list, takes an entry back or toggles
//! one.
//!
//! What may be chosen comes from the `act` effect's `from`, or else from
//! the host ([`SceneHost::candidates`](crate::SceneHost::candidates)),
//! or else, for an entity, from the view model: what the party
//! perceives. Never from the world.
//!
//! # Decisions
//!
//! See [`decide`].

pub mod ask;
pub mod decide;
mod def;

pub(crate) use ask::{Ask, Collecting};
pub use ask::{
    COMMAND_REACT, PICKER_CELL, PICKER_DIRECTION, PICKER_ENTITY, PICKER_LIST, PICKER_MULTI,
    PICKER_NUMBER, PICKER_ROLE_PREFIX, PICKER_ROUTE, PICKER_TEXT, ParamAsk,
};
pub use decide::{
    COMMAND_CHOOSE, COMMAND_PASS, DecisionAsk, PendingDecision, ROLE_DECISION_CHOICE,
    ROLE_DECISION_FREE, ROLE_DECISION_REACTION, ROLE_PLAY, ReactionOption,
};
pub use def::ControlDef;
