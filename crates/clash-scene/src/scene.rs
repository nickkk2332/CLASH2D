//! [`Scene`]: what stands between a frontend and the simulation (12 §3).
//!
//! The scene takes input, keeps the stack of screens, and builds frames.
//! Its link to the simulation is the [`SceneHost`] trait, and that link
//! is deliberately narrow: the scene can `submit` a command and do
//! nothing else to its host (12 invariant V1).
//!
//! # A seam, for now
//!
//! The simulation does not exist yet, so `SceneHost` holds only what
//! package U1 needs: `submit`. When the simulation lands (packages U4 and
//! U11), the host gains a read-only view (`SimView`) and the scene is
//! given drained events to play back; `Command` and `Refusal` become the
//! simulation's `Command` and `CommandError`. [`NoHost`] is the host of a
//! scene with no simulation behind it: tools, tests and the widget
//! gallery.

use core::convert::Infallible;

use clash_core::Rng;

use crate::frame::{Frame, FrameCell};
use crate::frontend::Frontend;
use crate::input::{InputEvent, KeymapDef};
use crate::theme::Theme;
use crate::ui::{MessageLog, Screen, UiData, UiEvent};

/// The scene's handle on the simulation: it can submit a command, and
/// that is all (12 invariant V1).
pub trait SceneHost {
    /// What is submitted: the simulation's `Command` (03 §4.4).
    type Command;
    /// Why a command was refused: the simulation's `CommandError`.
    type Refusal;

    /// Hands a command to the simulation. A refusal carries its reason.
    fn submit(&mut self, command: Self::Command) -> Result<(), Self::Refusal>;
}

/// The host of a scene with no simulation behind it. It has no commands,
/// so nothing can be submitted to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoHost;

impl SceneHost for NoHost {
    type Command = Infallible;
    type Refusal = Infallible;

    fn submit(&mut self, command: Infallible) -> Result<(), Infallible> {
        match command {}
    }
}

/// What one [`Scene::turn`] produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    /// What the screens asked for in answer to the input.
    pub events: Vec<UiEvent>,
    /// Whether the player asked to close the window.
    pub close_requested: bool,
}

impl Turn {
    /// Whether any event asks for an effect with this name to be run.
    pub fn asks_for(&self, effect: &str) -> bool {
        self.events.iter().any(|event| event.asks_for(effect))
    }
}

/// The seed of the scene's own random stream. Presentation randomness
/// (particles, shake) never touches the simulation's streams.
const SCENE_SEED: u64 = 0x5ce4e;

/// The scene: the stack of screens, the theme and keymap in force, the
/// message log, and the one route to the simulation.
///
/// # The only route to the simulation is `submit`
///
/// The host is private, and no method hands it out. This compiles:
///
/// ```
/// use clash_scene::{KeymapDef, Scene, SceneHost, Theme};
///
/// struct Counter(u32);
/// impl SceneHost for Counter {
///     type Command = u32;
///     type Refusal = String;
///     fn submit(&mut self, command: u32) -> Result<(), String> {
///         if command == 0 {
///             return Err("nothing to add".to_owned());
///         }
///         self.0 += command;
///         Ok(())
///     }
/// }
///
/// let mut scene = Scene::new(Counter(0), Theme::default(), KeymapDef::fallback());
/// assert_eq!(scene.submit(2), Ok(()));
/// assert_eq!(scene.submit(0), Err("nothing to add".to_owned()));
/// ```
///
/// and each of these does not, because it reaches for the host some other
/// way:
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn reach<H: SceneHost>(scene: &mut Scene<H>) -> &mut H {
///     &mut scene.host
/// }
/// ```
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn reach<H: SceneHost>(scene: &mut Scene<H>) -> &mut H {
///     scene.host_mut()
/// }
/// ```
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn take<H: SceneHost>(scene: Scene<H>) -> H {
///     scene.into_host()
/// }
/// ```
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn peek<H: SceneHost>(scene: &Scene<H>) -> &H {
///     scene.host()
/// }
/// ```
///
/// Once the simulation exists these tests must be joined by ones that
/// use the real handle: that a scene holding it cannot call `advance`,
/// `drain_events` or anything that takes the simulation mutably, and
/// that the view it is given is read-only.
#[derive(Debug)]
pub struct Scene<H: SceneHost = NoHost> {
    host: H,
    theme: Theme,
    keymap: KeymapDef,
    screens: Vec<Screen>,
    log: MessageLog,
    rng: Rng,
    elapsed: f64,
}

impl<H: SceneHost> Scene<H> {
    /// A scene with no screens open.
    pub fn new(host: H, theme: Theme, keymap: KeymapDef) -> Scene<H> {
        Scene {
            host,
            theme,
            keymap,
            screens: Vec::new(),
            log: MessageLog::new(),
            rng: Rng::from_seed(SCENE_SEED),
            elapsed: 0.0,
        }
    }

    /// Submits a command to the simulation: the scene's only way to
    /// change it.
    pub fn submit(&mut self, command: H::Command) -> Result<(), H::Refusal> {
        self.host.submit(command)
    }

    /// The theme in force.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The keymap in force.
    pub fn keymap(&self) -> &KeymapDef {
        &self.keymap
    }

    /// The message log.
    pub fn log(&self) -> &MessageLog {
        &self.log
    }

    /// The message log, to add lines to it.
    pub fn log_mut(&mut self) -> &mut MessageLog {
        &mut self.log
    }

    /// The scene's own random stream, for presentation only.
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// Puts a screen on top of the stack.
    pub fn open(&mut self, screen: Screen) {
        self.screens.push(screen);
    }

    /// Takes the top screen off the stack.
    pub fn close(&mut self) -> Option<Screen> {
        self.screens.pop()
    }

    /// The open screens, bottom first.
    pub fn screens(&self) -> &[Screen] {
        &self.screens
    }

    /// The top screen: the one that is drawn and receives input.
    pub fn top(&self) -> Option<&Screen> {
        self.screens.last()
    }

    /// The top screen, to change it.
    pub fn top_mut(&mut self) -> Option<&mut Screen> {
        self.screens.last_mut()
    }

    /// Passes input to the top screen and returns what it asks for.
    pub fn input(&mut self, event: &InputEvent) -> Vec<UiEvent> {
        match self.screens.last_mut() {
            Some(screen) => screen.input(event, &self.keymap),
            None => Vec::new(),
        }
    }

    /// One pass of the application loop of 12 §3, without a simulation:
    /// takes the frontend's input, advances the scene by `seconds` of real
    /// time, builds a frame of the frontend's size and presents it.
    /// Returns what the input asked for. Running effects is the caller's
    /// business until package U6b.
    pub fn turn(&mut self, front: &mut dyn Frontend, frame: &mut Frame, seconds: f64) -> Turn {
        let mut turn = Turn::default();
        for event in front.poll() {
            if event == InputEvent::Close {
                turn.close_requested = true;
            }
            turn.events.extend(self.input(&event));
        }
        self.advance(seconds);
        let caps = front.caps();
        if (frame.cols, frame.rows) != (caps.cols, caps.rows) {
            frame.resize(caps.cols, caps.rows);
        }
        self.build_frame(frame);
        front.present(frame);
        turn
    }

    /// Advances the scene by real time, in seconds. The application
    /// measures it; tests pass a fixed step (12 §12).
    pub fn advance(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds > 0.0 {
            self.elapsed += seconds;
        }
    }

    /// Real time the scene has been advanced by, in seconds.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Builds the frame: the top screen, or an empty frame in the theme's
    /// colours when no screen is open. The frame keeps its size and its
    /// storage.
    pub fn build_frame(&mut self, frame: &mut Frame) {
        frame.clear(FrameCell::blank(self.theme.base()));
        let data = UiData {
            log: Some(&self.log),
            views: None,
        };
        if let Some(screen) = self.screens.last_mut() {
            screen.draw(frame, &data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_with_no_screens_builds_an_empty_frame_in_the_theme() {
        let mut scene: Scene = Scene::new(NoHost, Theme::default(), KeymapDef::fallback());
        let mut frame = Frame::new(4, 2);
        frame.fx.shake = 1.0;
        scene.build_frame(&mut frame);
        assert_eq!(frame.text(), "    \n    \n");
        assert_eq!(frame.cells[0].style(), Theme::default().base());
        assert_eq!(frame.fx.shake, 0.0);
        assert!(scene.input(&InputEvent::Close).is_empty());
        assert!(scene.close().is_none());
    }

    #[test]
    fn time_only_moves_forward() {
        let mut scene: Scene = Scene::new(NoHost, Theme::default(), KeymapDef::fallback());
        scene.advance(0.25);
        scene.advance(-1.0);
        scene.advance(f64::NAN);
        scene.advance(0.5);
        assert_eq!(scene.elapsed(), 0.75);
        let first = scene.rng().next_u64();
        let mut again: Scene = Scene::new(NoHost, Theme::default(), KeymapDef::fallback());
        assert_eq!(
            again.rng().next_u64(),
            first,
            "the scene's stream is seeded the same every run"
        );
    }
}
