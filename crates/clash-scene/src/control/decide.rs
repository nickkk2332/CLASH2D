//! Decisions (12 §9.5): the simulation stops and asks a directly
//! controlled character to choose (03 §5.2); the scene opens the screen
//! for the kind of decision and submits the answer as a command.
//!
//! The simulation's own `DecisionRequest` is not named here. The host
//! hands the scene a [`PendingDecision`], which holds what a screen
//! needs: who decides and what is asked, in values and words.
//!
//! | Kind | Role of the screen | The answer becomes |
//! |------|--------------------|--------------------|
//! | Free | `decision.free`: the play screen itself | Whatever the player does next: `act`, `wait` |
//! | Reaction | `decision.reaction` | `react` with the chosen action, or `pass` |
//! | Choice | `decision.choice` | `choose` with the option's number |
//!
//! A prompt answers with the `answer` effect: the number of the option
//! chosen, from 0. A reaction prompt that answers null passes. A prompt
//! closed any other way has not answered: the decision is still waiting
//! and the prompt is opened again.

use std::collections::BTreeMap;

use clash_expr::Value as ExprValue;

use crate::scene::{Pending, Scene, SceneHost, UiCommand, UiOutcome, Wait};
use crate::ui::{Collect, CommandKind};

/// The role of the screen a game is played on (12 §10.7).
pub const ROLE_PLAY: &str = "play";
/// The role of the screen that answers a free decision: the play
/// screen. It is opened if it is not already open.
pub const ROLE_DECISION_FREE: &str = "decision.free";
/// The role of the reaction prompt.
pub const ROLE_DECISION_REACTION: &str = "decision.reaction";
/// The role of the choice prompt.
pub const ROLE_DECISION_CHOICE: &str = "decision.choice";

/// The command that declines to react (03 §4.4).
pub const COMMAND_PASS: &str = "pass";
/// The command that answers a choice (03 §4.4).
pub const COMMAND_CHOOSE: &str = "choose";

/// One way to answer a reaction window.
#[derive(Clone, Debug, PartialEq)]
pub struct ReactionOption {
    /// The reaction: an action's qualified id.
    pub action: String,
    /// What it is called, as the player is shown it.
    pub label: String,
    /// Whether it would be ready before the window closes (03 §7.3). One
    /// that would not may still be chosen.
    pub in_time: bool,
}

/// What a decision asks.
#[derive(Clone, Debug, PartialEq)]
pub enum DecisionAsk {
    /// What to do next. The play screen answers.
    Free,
    /// How to answer a window before it closes.
    Reaction {
        /// The window, as the simulation's value for it.
        window: ExprValue,
        /// The reactions that may answer it.
        options: Vec<ReactionOption>,
        /// How long the character has before the window closes, in
        /// seconds of its own time.
        time_left: f64,
        /// How long the window was open for, for the bar that shows
        /// what is left.
        time_total: f64,
    },
    /// One of several options, put by a rule's `ask` effect.
    Choice {
        /// The choice, as the simulation's value for it.
        choice: ExprValue,
        /// The question.
        prompt: String,
        /// The options, as the player is shown them.
        options: Vec<String>,
    },
}

/// A decision the simulation is waiting for (03 §5.2), as the host hands
/// it to the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingDecision {
    /// Who decides: a directly controlled character.
    pub entity: ExprValue,
    /// What is asked.
    pub kind: DecisionAsk,
}

impl<H> Scene<H>
where
    H: SceneHost,
    H::Command: From<UiCommand>,
    H::Refusal: core::fmt::Display,
{
    /// Asks the host whether a decision is waiting and, if it is one the
    /// scene has not shown yet, opens the screen for it (12 §9.5).
    /// [`Scene::turn`] does this each pass; an application that drives
    /// the scene itself calls it when playback reaches the decision.
    ///
    /// While a command is being put together nothing is opened: the
    /// decision waits for it.
    pub fn sync_decision(&mut self) -> Vec<UiOutcome> {
        let mut out = Vec::new();
        if self.collecting.is_some() {
            return out;
        }
        let pending = self.host.pending_decision();
        if pending == self.deciding {
            return out;
        }
        // A prompt for a decision that is no longer the one waiting.
        if matches!(
            self.pending.last().and_then(Option::as_ref),
            Some(Pending {
                wait: Wait::Decision,
                ..
            })
        ) {
            self.pending.pop();
            if let Some(closed) = self.pop_screen() {
                out.push(UiOutcome::Closed(closed.id().to_owned()));
            }
        }
        self.deciding = None;
        let Some(decision) = pending else {
            return out;
        };
        let who = ("who".to_owned(), decision.entity.clone());
        let asked = Some(Pending::bare(Wait::Decision));
        let opened = match &decision.kind {
            // The play screen answers: it is opened if it is not open
            // already, and nothing waits for it.
            DecisionAsk::Free => match self.role(ROLE_DECISION_FREE).map(str::to_owned) {
                Some(id) if !self.screens.iter().any(|screen| screen.id() == id) => self
                    .open_role_with(ROLE_DECISION_FREE, BTreeMap::new(), None)
                    .map(Some),
                _ => Ok(None),
            },
            DecisionAsk::Reaction {
                options,
                time_left,
                time_total,
                ..
            } => {
                let rows = options.iter().enumerate().map(|(index, option)| {
                    ExprValue::map([
                        ("label", ExprValue::text(&option.label)),
                        ("value", ExprValue::Int(index as i64)),
                        ("action", ExprValue::text(&option.action)),
                        ("late", ExprValue::Bool(!option.in_time)),
                    ])
                });
                let real = |seconds: f64| {
                    ExprValue::Real(clash_core::Real::new(seconds).unwrap_or(clash_core::Real::ZERO))
                };
                let with = BTreeMap::from([
                    who,
                    ("options".to_owned(), ExprValue::list(rows)),
                    ("time_left".to_owned(), real(*time_left)),
                    ("time_total".to_owned(), real(*time_total)),
                ]);
                self.open_role_with(ROLE_DECISION_REACTION, with, asked)
                    .map(Some)
            }
            DecisionAsk::Choice {
                prompt, options, ..
            } => {
                let rows = options.iter().enumerate().map(|(index, label)| {
                    ExprValue::map([
                        ("label", ExprValue::text(label)),
                        ("value", ExprValue::Int(index as i64)),
                    ])
                });
                let with = BTreeMap::from([
                    who,
                    ("prompt".to_owned(), ExprValue::text(prompt)),
                    ("options".to_owned(), ExprValue::list(rows)),
                ]);
                self.open_role_with(ROLE_DECISION_CHOICE, with, asked)
                    .map(Some)
            }
        };
        match opened {
            Ok(id) => {
                out.extend(id.map(UiOutcome::Opened));
                self.deciding = Some(decision);
            }
            Err(reason) => out.push(UiOutcome::Failed {
                effect: "open",
                reason,
            }),
        }
        out
    }

    /// The decision the scene has shown and not yet seen answered.
    pub fn decision(&self) -> Option<&PendingDecision> {
        self.deciding.as_ref()
    }

    /// A decision prompt answered: the answer becomes a command.
    pub(crate) fn decision_answered(&mut self, answer: ExprValue, out: &mut Vec<UiOutcome>) {
        let Some(decision) = self.deciding.clone() else {
            return;
        };
        let option = answer.as_int().and_then(|index| usize::try_from(index).ok());
        let entity = decision.entity.clone();
        let command = |name: &str, args: BTreeMap<String, ExprValue>| UiCommand {
            kind: CommandKind::Command,
            name: name.to_owned(),
            args,
            collect: Collect::Given,
        };
        let close = |scene: &mut Scene<H>, out: &mut Vec<UiOutcome>| {
            scene.pending.pop();
            if let Some(closed) = scene.pop_screen() {
                out.push(UiOutcome::Closed(closed.id().to_owned()));
            }
            // Answered, or to be asked again if the answer is refused.
            scene.deciding = None;
        };
        match decision.kind {
            DecisionAsk::Free => {}
            DecisionAsk::Reaction {
                window, options, ..
            } => {
                close(self, out);
                match option.and_then(|index| options.get(index)) {
                    Some(chosen) => self.begin_reaction(&chosen.action, entity, window, out),
                    None => {
                        let args = BTreeMap::from([
                            ("entity".to_owned(), entity),
                            ("window".to_owned(), window),
                        ]);
                        self.submit_command(command(COMMAND_PASS, args), out);
                    }
                }
            }
            DecisionAsk::Choice {
                choice, options, ..
            } => match option.filter(|index| *index < options.len()) {
                Some(index) => {
                    close(self, out);
                    let args = BTreeMap::from([
                        ("entity".to_owned(), entity),
                        ("choice".to_owned(), choice),
                        ("option".to_owned(), ExprValue::Int(index as i64)),
                    ]);
                    self.submit_command(command(COMMAND_CHOOSE, args), out);
                }
                // A choice must be made: the prompt stays.
                None => out.push(UiOutcome::Failed {
                    effect: "answer",
                    reason: format!("{answer} is not one of the options"),
                }),
            },
        }
    }
}
