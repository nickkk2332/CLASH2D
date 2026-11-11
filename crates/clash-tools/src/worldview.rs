//! The `worldview` subcommand (W11; 14 §3.1, the terminal viewer): a
//! small debugging tool to look at a world and poke it from a console
//! line.
//!
//! In a terminal it shows one storey of the world round the cursor, from
//! above and as it truly is (the scene's `WorldView` in omniscient
//! mode), drawn by the terminal frontend, with a console line under it.
//! Without a terminal, or with `--script`, it runs console lines from a
//! file or from standard input and prints what each gives, so that an
//! agent can look at a world:
//!
//! ```text
//! printf 'use default seed=3\ngen city size=2\nframe\n' | clash-tools worldview
//! ```
//!
//! | Key | Does |
//! |-----|------|
//! | Arrows, `h` `j` `k` `l` | Move the cursor a cell; ten with shift |
//! | `<`, `>`, Page Up, Page Down | A storey up, a storey down |
//! | `:` or Enter | Open the console line; Enter runs it, Escape closes it |
//! | `q`, Escape | Leave |
//!
//! The host is [`clash_save::host::GameHost`], the same the test-script
//! runner drives, so every console command works here.

use std::fmt::Write as _;
use std::io::{IsTerminal, Read};
use std::path::Path;
use std::sync::Arc;

use clash_core::Tick;
use clash_save::host::{GameHost, HostEnv};
use clash_scene::{Canvas, CellStyle, Frame, Frontend, InputEvent, KeyChord, KeyCode, Mods, Rect};
use clash_sim::console::{CommandTable, ConsoleHost, HostKind, Session};
use clash_world::{CellPos, Location};

use crate::data::{EXIT_FAILED, EXIT_OK, EXIT_USAGE, Output};
use crate::scenario::mods_dir;

/// The usage text of the `worldview` subcommand.
pub const USAGE: &str = "\
world viewer:
  worldview [--mods <dir>] [--load <save>] [--script <file>]
        Look at a world through the console. In a terminal: one storey
        round the cursor, moved with the arrows or hjkl (ten cells with
        shift), a storey up and down with < and >, a console line opened
        with : or Enter, q or Escape to leave. Without a terminal, or
        with --script, run console lines from the file or from standard
        input and print what each gives; `frame` prints the map as text.
        Exits 1 if a line failed.
";

/// How many cells a move with shift held goes.
const FAST_MOVE: i32 = 10;

/// How many rows under the map hold the status, the last output and
/// the console line.
const PANEL_ROWS: u16 = 3;

/// How long the terminal loop waits when nothing happened.
const IDLE_WAIT: std::time::Duration = std::time::Duration::from_millis(20);

fn usage(message: &str) -> Output {
    Output {
        out: String::new(),
        err: format!("clash-tools: {message}\n\n{USAGE}"),
        code: EXIT_USAGE,
    }
}

/// A console over a real simulation, with the map as a view of it.
pub struct Viewer {
    table: CommandTable,
    host: GameHost,
    session: Session,
    /// The console line being typed, if it is open.
    typing: Option<String>,
    /// What the last line gave, one line of it per row.
    said: Vec<String>,
}

impl Viewer {
    /// A viewer with no world yet, finding profiles in `env`.
    pub fn new(env: Arc<HostEnv>) -> Viewer {
        Viewer {
            table: GameHost::table(),
            host: GameHost::new(env),
            session: Session::new(HostKind::Viewer),
            typing: None,
            said: vec!["`:` opens the console: `use default`, then `gen city size=2`".to_owned()],
        }
    }

    /// Loads a save file into the viewer.
    pub fn load(&mut self, path: &Path) -> Result<(), String> {
        self.host
            .load_from(path)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Runs one console line, giving what it printed or why it failed.
    pub fn line(&mut self, line: &str) -> Result<String, String> {
        self.session
            .run_line(&self.table, &mut self.host, line)
            .map(|output| output.text)
            .map_err(|error| error.to_string())
    }

    /// Moves the cursor by cells and storeys, if there is one.
    fn nudge(&mut self, dx: i32, dy: i32, dz: i32) {
        if let Some(here) = self.host.here() {
            let cell = here.cell;
            self.host.set_here(Location::new(
                here.grid,
                CellPos::new(cell.x + dx, cell.y + dy, cell.z + dz),
            ));
        }
    }

    /// Handles input; false when the viewer should close.
    pub fn handle(&mut self, events: Vec<InputEvent>) -> bool {
        for event in events {
            match event {
                InputEvent::Close => return false,
                InputEvent::Text(text) => {
                    if let Some(typed) = &mut self.typing {
                        typed.push_str(&text);
                    }
                }
                InputEvent::Key(chord) => {
                    if self.typing.is_some() {
                        self.type_key(chord);
                    } else if !self.view_key(chord) {
                        return false;
                    }
                }
                InputEvent::Mouse(_) | InputEvent::Resize { .. } => {}
            }
        }
        true
    }

    /// A key while the console line is open.
    fn type_key(&mut self, chord: KeyChord) {
        let Some(typed) = &mut self.typing else {
            return;
        };
        match chord.code {
            KeyCode::Enter => {
                let line = std::mem::take(typed);
                self.typing = None;
                let said = match self.line(&line) {
                    Ok(text) => text,
                    Err(error) => format!("error: {error}"),
                };
                self.said = said.lines().map(str::to_owned).collect();
            }
            KeyCode::Escape => self.typing = None,
            KeyCode::Backspace => {
                typed.pop();
            }
            KeyCode::Char(ch) if !chord.mods.contains(Mods::CTRL) => {
                typed.push(if chord.mods.contains(Mods::SHIFT) {
                    ch.to_ascii_uppercase()
                } else {
                    ch
                });
            }
            _ => {}
        }
    }

    /// A key while looking at the map; false to leave.
    fn view_key(&mut self, chord: KeyChord) -> bool {
        let step = if chord.mods.contains(Mods::SHIFT) {
            FAST_MOVE
        } else {
            1
        };
        match chord.code {
            KeyCode::Escape | KeyCode::Char('q') => return false,
            KeyCode::Enter | KeyCode::Char(':') => self.typing = Some(String::new()),
            KeyCode::Left | KeyCode::Char('h') => self.nudge(-step, 0, 0),
            KeyCode::Right | KeyCode::Char('l') => self.nudge(step, 0, 0),
            KeyCode::Up | KeyCode::Char('k') => self.nudge(0, -step, 0),
            KeyCode::Down | KeyCode::Char('j') => self.nudge(0, step, 0),
            KeyCode::PageUp | KeyCode::Char('<') => self.nudge(0, 0, 1),
            KeyCode::PageDown | KeyCode::Char('>') => self.nudge(0, 0, -1),
            _ => {}
        }
        true
    }

    /// The whole screen: the map, a status line, what the last line
    /// gave and the console line.
    pub fn draw(&mut self, cols: u16, rows: u16) -> Frame {
        let mut frame = Frame::new(cols, rows);
        let map_rows = rows.saturating_sub(PANEL_ROWS).max(1);
        let style = CellStyle::default();
        let mut canvas = Canvas::new(&mut frame);
        let map = Rect::new(0, 0, i32::from(cols), i32::from(map_rows));
        if let Err(error) = self.host.draw_map(&mut canvas, map) {
            canvas.print(0, 0, &format!("{error}"), style);
        }
        let status = match self.host.here() {
            Some(here) => format!(
                "@c:{},{},{}  t={}",
                here.cell.x,
                here.cell.y,
                here.cell.z,
                self.host.now().since(Tick::ZERO)
            ),
            None => "no world".to_owned(),
        };
        let row = i32::from(map_rows);
        canvas.print(0, row, &status, style);
        if let Some(last) = self.said.last() {
            canvas.print(0, row + 1, last, style);
        }
        let prompt = match &self.typing {
            Some(typed) => format!(": {typed}_"),
            None => "[: console] [arrows move] [< > storey] [q leave]".to_owned(),
        };
        canvas.print(0, row + 2, &prompt, style);
        frame
    }
}

/// Runs console lines one by one and prints what each gives: the
/// viewer without a terminal.
pub fn run_script(viewer: &mut Viewer, text: &str) -> Output {
    let mut out = String::new();
    let mut failed = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let _ = writeln!(out, "> {trimmed}");
        match viewer.line(trimmed) {
            Ok(text) if text.is_empty() => {}
            Ok(text) => {
                let _ = writeln!(out, "{}", text.trim_end());
            }
            Err(error) => {
                failed += 1;
                let _ = writeln!(out, "error: {error}");
            }
        }
    }
    Output {
        err: if failed == 0 {
            String::new()
        } else {
            format!("clash-tools: {failed} lines failed\n")
        },
        out,
        code: if failed == 0 { EXIT_OK } else { EXIT_FAILED },
    }
}

/// The viewer in a terminal, until it is left.
fn run_terminal(viewer: &mut Viewer) -> Output {
    let mut front = match clash_front_term::TermFrontend::start(false) {
        Ok(front) => front,
        Err(error) => {
            return Output {
                out: String::new(),
                err: format!("clash-tools: {error}\n"),
                code: EXIT_FAILED,
            };
        }
    };
    let mut drawn = false;
    loop {
        let events = front.poll();
        let quiet = events.is_empty();
        if !viewer.handle(events) {
            break;
        }
        if quiet && drawn {
            std::thread::sleep(IDLE_WAIT);
            continue;
        }
        let caps = front.caps();
        front.present(&viewer.draw(caps.cols, caps.rows));
        drawn = true;
        if let Some(error) = front.take_error() {
            drop(front);
            return Output {
                out: String::new(),
                err: format!("clash-tools: the terminal failed: {error}\n"),
                code: EXIT_FAILED,
            };
        }
    }
    Output {
        out: String::new(),
        err: String::new(),
        code: EXIT_OK,
    }
}

/// Runs the `worldview` subcommand with the arguments after its name.
pub fn run(args: &[String]) -> Output {
    let mut mods = None;
    let mut load = None;
    let mut script = None;
    let mut words = args.iter().map(String::as_str);
    while let Some(word) = words.next() {
        let mut value = |name: &str| {
            words
                .next()
                .ok_or_else(|| usage(&format!("`{name}` needs a value")))
        };
        match word {
            "--mods" => match value("--mods") {
                Ok(given) => mods = Some(given),
                Err(output) => return output,
            },
            "--load" => match value("--load") {
                Ok(given) => load = Some(given),
                Err(output) => return output,
            },
            "--script" => match value("--script") {
                Ok(given) => script = Some(given),
                Err(output) => return output,
            },
            other => return usage(&format!("`worldview` does not take `{other}`")),
        }
    }
    let beside = script.map_or(Path::new("."), |file| {
        Path::new(file).parent().unwrap_or(Path::new("."))
    });
    let env = Arc::new(HostEnv::new(mods_dir(mods, beside)));
    let mut viewer = Viewer::new(env);
    if let Some(file) = load
        && let Err(error) = viewer.load(Path::new(file))
    {
        return Output {
            out: String::new(),
            err: format!("clash-tools: {error}\n"),
            code: EXIT_FAILED,
        };
    }
    let text = match script {
        Some(file) => match std::fs::read_to_string(file) {
            Ok(text) => Some(text),
            Err(error) => {
                return Output {
                    out: String::new(),
                    err: format!("clash-tools: cannot read `{file}`: {error}\n"),
                    code: EXIT_FAILED,
                };
            }
        },
        None if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() => {
            let mut text = String::new();
            if let Err(error) = std::io::stdin().read_to_string(&mut text) {
                return Output {
                    out: String::new(),
                    err: format!("clash-tools: cannot read standard input: {error}\n"),
                    code: EXIT_FAILED,
                };
            }
            Some(text)
        }
        None => None,
    };
    match text {
        Some(text) => run_script(&mut viewer, &text),
        None => run_terminal(&mut viewer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer() -> Viewer {
        let mods = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods");
        Viewer::new(Arc::new(HostEnv::new(mods)))
    }

    #[test]
    fn a_script_of_console_lines_prints_a_frame_of_the_city() {
        let mut viewer = viewer();
        let output = run_script(
            &mut viewer,
            "# Look at the test city.\nuse default seed=3\ngen city size=2\nsize 40 12\nframe\n",
        );
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(output.out.contains("> gen city size=2"), "{}", output.out);
        assert!(
            output.out.contains("made a realm of `core:test_city`"),
            "{}",
            output.out
        );
        // The frame: a line saying where, then twelve rows of forty.
        let frame: Vec<&str> = output
            .out
            .lines()
            .skip_while(|line| !line.starts_with("> frame"))
            .skip(2)
            .collect();
        assert_eq!(frame.len(), 12, "{}", output.out);
        assert!(frame.iter().all(|row| row.chars().count() == 40));
        assert!(
            frame.iter().any(|row| row.trim() != ""),
            "the city shows: {}",
            output.out
        );
    }

    #[test]
    fn a_line_that_fails_is_said_and_counted() {
        let mut viewer = viewer();
        let output = run_script(&mut viewer, "frame\nuse nowhere_at_all\n");
        assert_eq!(output.code, EXIT_FAILED);
        assert!(
            output.out.contains("error: refused (console.no_game"),
            "{}",
            output.out
        );
        assert!(output.err.contains("2 lines failed"), "{}", output.err);
    }

    #[test]
    fn keys_move_the_cursor_change_storey_and_run_a_console_line() {
        let mut viewer = viewer();
        viewer.line("use default seed=1").unwrap();
        viewer.line("gen flat size=1").unwrap();
        let start = viewer.host.here().expect("a cursor");
        let key = |name: &str| InputEvent::key(name).unwrap();
        assert!(viewer.handle(vec![key("right"), key("right"), key("down"), key("<")]));
        let here = viewer.host.here().unwrap();
        assert_eq!(here.cell.x, start.cell.x + 2);
        assert_eq!(here.cell.y, start.cell.y + 1);
        assert_eq!(here.cell.z, start.cell.z + 1);

        // A console line typed and run.
        let mut typed = vec![key(":")];
        typed.push(InputEvent::Text("spawn creature".to_owned()));
        typed.push(key("enter"));
        assert!(viewer.handle(typed));
        assert!(
            viewer
                .said
                .iter()
                .any(|line| line.contains("made the creature"))
        );
        let frame = viewer.draw(50, 16);
        let text = frame.text();
        assert!(text.contains("@c:"), "{text}");
        assert!(text.contains('@'), "{text}");
        assert!(text.contains("[: console]"), "{text}");

        assert!(!viewer.handle(vec![key("q")]));
    }

    #[test]
    fn the_command_line_is_checked() {
        assert_eq!(run(&["--paranoid".to_owned()]).code, EXIT_USAGE);
        assert_eq!(run(&["--script".to_owned()]).code, EXIT_USAGE);
        let missing = run(&["--script".to_owned(), "no/such/file.txt".to_owned()]);
        assert_eq!(missing.code, EXIT_FAILED);
    }
}
