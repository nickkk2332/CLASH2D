//! Running without a terminal: the game driven by a script, for tests
//! and for agents (12 §12).
//!
//! `clash --headless script.txt` runs the same loop as a terminal
//! does, on the null frontend, with a fixed time step and a grid of
//! [`HEADLESS_SIZE`]. Input comes from the script, one line at a time.
//! Each line of input is followed by one pass of the loop, in which the
//! simulation is advanced until the player's character is free again,
//! so a script is reproducible (12 V8).
//!
//! # The script
//!
//! | Line | Does |
//! |------|------|
//! | A key, as a keymap writes it: `enter`, `down`, `j`, `shift+a`, `ctrl+d` (12 §9.1) | Presses the key, then one pass |
//! | `type some text` | Presses the key of each character of the rest of the line in turn, a pass after each |
//! | `wait`, or `wait n` | One pass, or `n`, with no input |
//! | `frame` | Prints the current frame as plain text: a line `--- frame ---`, then one line for each row |
//! | `frame golden` | Prints it as a golden frame is written: characters, colours, attributes, sprites and effects (12 §12) |
//! | `size cols rows` | Changes the grid, then one pass |
//! | `# anything`, or nothing | Nothing |
//!
//! A line that is none of these stops the run with the line's number
//! and what is wrong with it. Once the game has ended (the player quit,
//! or the last screen closed) the rest of the script is not read.

use std::io::Write;

use clash_scene::{
    Frame, Frontend, InputEvent, KeyChord, KeyCode, Mods, NullFrontend, frame_to_text,
};

use crate::game::Game;

/// The grid a headless run is drawn on: the smallest the bundled
/// screens support (12 §10.3).
pub const HEADLESS_SIZE: (u16, u16) = (80, 24);

/// The real time a headless pass stands for, in seconds: one frame at
/// sixty a second.
pub const HEADLESS_STEP_S: f64 = 1.0 / 60.0;

/// The line that precedes a frame printed as plain text.
pub const FRAME_HEADER: &str = "--- frame ---";

/// A game on the null frontend, driven a pass at a time.
pub struct Headless {
    game: Game,
    front: NullFrontend,
    running: bool,
}

impl Headless {
    /// Puts a booted game on the null frontend and makes the first
    /// pass, so that there is a frame: the boot screen.
    pub fn new(game: Game) -> Headless {
        let mut headless = Headless {
            game,
            front: NullFrontend::new(HEADLESS_SIZE.0, HEADLESS_SIZE.1).keep(1),
            running: true,
        };
        headless.pass();
        headless
    }

    /// The game.
    pub fn game(&self) -> &Game {
        &self.game
    }

    /// The current frame.
    pub fn frame(&self) -> &Frame {
        self.game.frame()
    }

    /// The current frame as plain text, one line for each row.
    pub fn text(&self) -> String {
        self.game.frame().text()
    }

    /// Whether the game is still running: the player has not left.
    pub fn running(&self) -> bool {
        self.running
    }

    /// How many frames have been presented.
    pub fn presented(&self) -> u64 {
        self.front.presented()
    }

    /// One pass of the loop with whatever input is waiting. Returns
    /// whether the game goes on.
    pub fn pass(&mut self) -> bool {
        if self.running {
            self.running = self.game.pass(&mut self.front, HEADLESS_STEP_S, None);
        }
        self.running
    }

    /// Delivers input, then one pass.
    pub fn input(&mut self, event: InputEvent) -> bool {
        self.front.script([event]);
        self.pass()
    }

    /// Presses a key written as a keymap writes it, then one pass.
    pub fn key(&mut self, key: &str) -> Result<bool, String> {
        let chord = KeyChord::parse(key).map_err(|error| error.to_string())?;
        Ok(self.input(InputEvent::Key(chord)))
    }

    /// Presses the key of each character of a text in turn, a pass
    /// after each.
    pub fn type_text(&mut self, text: &str) -> bool {
        for ch in text.chars() {
            self.input(InputEvent::Key(KeyChord {
                code: KeyCode::Char(ch),
                mods: Mods::empty(),
            }));
        }
        self.running
    }

    /// Presses each of several keys in turn, a pass after each.
    pub fn keys(&mut self, keys: &[&str]) -> Result<bool, String> {
        for key in keys {
            self.key(key)?;
        }
        Ok(self.running)
    }

    /// Runs a script, writing what its `frame` lines print to `out`.
    /// See the [module](self) documentation. An error names the line.
    pub fn run_script(&mut self, script: &str, out: &mut dyn Write) -> Result<(), String> {
        for (index, raw) in script.lines().enumerate() {
            if !self.running {
                break;
            }
            let line = raw.trim();
            let at = |problem: String| format!("line {}: {problem}", index + 1);
            let wrote = |error: std::io::Error| format!("the frame could not be written: {error}");
            let (word, rest) = match line.split_once(char::is_whitespace) {
                Some((word, rest)) => (word, rest.trim()),
                None => (line, ""),
            };
            match word {
                "" => {}
                _ if word.starts_with('#') => {}
                "frame" if rest.is_empty() => {
                    write!(out, "{FRAME_HEADER}\n{}", self.text()).map_err(wrote)?;
                }
                "frame" if rest == "golden" => {
                    write!(out, "{}", frame_to_text(self.frame())).map_err(wrote)?;
                }
                "wait" => {
                    let passes: u32 = if rest.is_empty() {
                        1
                    } else {
                        rest.parse().map_err(|_| {
                            at(format!(
                                "`wait` takes a number of passes, and `{rest}` is not one"
                            ))
                        })?
                    };
                    for _ in 0..passes {
                        self.pass();
                    }
                }
                "type" => {
                    // The text is the rest of the line as written, so
                    // that it may hold several spaces.
                    let text = raw.trim_start().strip_prefix("type").unwrap_or("");
                    self.type_text(text.strip_prefix(' ').unwrap_or(text));
                }
                "size" => {
                    let mut numbers = rest.split_whitespace().map(str::parse::<u16>);
                    let (Some(Ok(cols)), Some(Ok(rows)), None) =
                        (numbers.next(), numbers.next(), numbers.next())
                    else {
                        return Err(at(format!(
                            "`size` takes the columns and the rows, and `{rest}` is not that"
                        )));
                    };
                    self.input(InputEvent::Resize { cols, rows });
                }
                _ if rest.is_empty() => {
                    self.key(word).map_err(|error| {
                        at(format!(
                            "`{word}` is not a key or a command of a script: {error}"
                        ))
                    })?;
                }
                _ => {
                    return Err(at(format!(
                        "`{line}` is not a key or a command of a script"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// The size a frontend reports, for a caller that wants to know what a
/// headless frame is drawn on.
pub fn headless_caps() -> clash_scene::Caps {
    NullFrontend::new(HEADLESS_SIZE.0, HEADLESS_SIZE.1).caps()
}
