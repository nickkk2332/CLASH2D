//! The widget gallery run headless: the `ui` mod's gallery screen driven
//! through the null frontend with scripted input and a fixed time step,
//! with golden frames along the way (12 §12), and the same frames shown
//! through the terminal renderer.

use std::path::PathBuf;
use std::process::Command;

use clash_front_term::Renderer;
use clash_scene::defs::UiDefs;
use clash_scene::{
    ColorDepth, Frame, Frontend, InputEvent, NoHost, NullFrontend, RichText, Scene, assert_golden,
};

/// The fixed time step of scripted runs, in seconds.
const STEP: f64 = 1.0 / 60.0;

fn mod_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/ui")
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

fn gallery() -> Scene<NoHost> {
    let describe = |errors: Vec<clash_scene::defs::DefError>| {
        let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
        lines.join("\n")
    };
    let defs = UiDefs::load_mod(&mod_dir()).unwrap_or_else(|e| panic!("{}", describe(e)));
    let mut scene = defs
        .scene("default", "default")
        .unwrap_or_else(|e| panic!("{}", describe(e)));
    let screen = defs
        .screen("gallery", scene.theme())
        .unwrap_or_else(|e| panic!("{}", describe(e)));
    scene.open(screen);
    for n in 1..=12 {
        let line = format!("Log line {n}.");
        scene.log_mut().push("demo", RichText::plain_text(&line));
    }
    scene
}

/// A headless run: scripted input in, frames out.
struct Run {
    scene: Scene<NoHost>,
    front: NullFrontend,
    frame: Frame,
    turns: u32,
}

impl Run {
    fn new(cols: u16, rows: u16) -> Run {
        Run {
            scene: gallery(),
            front: NullFrontend::new(cols, rows),
            frame: Frame::new(0, 0),
            turns: 0,
        }
    }

    /// Presses keys, one turn of the loop each, and returns whether any
    /// turn asked to close.
    fn press(&mut self, keys: &[&str]) -> bool {
        let mut close = false;
        for key in keys {
            self.front.script([InputEvent::key(key).unwrap()]);
            self.turns += 1;
            let turn = self.scene.turn(&mut self.front, &mut self.frame, STEP);
            close |= turn.close_requested || turn.asks_for("close");
        }
        close
    }

    /// One turn with no input.
    fn idle(&mut self) -> &Frame {
        self.turns += 1;
        self.scene.turn(&mut self.front, &mut self.frame, STEP);
        self.front.last().unwrap()
    }

    fn focus(&self) -> Option<&str> {
        self.scene.top().unwrap().focus()
    }

    fn text(&self) -> String {
        self.front.last().unwrap().text()
    }
}

#[test]
fn the_gallery_runs_headless_with_scripted_input() {
    let mut run = Run::new(80, 24);
    let first = run.idle().clone();
    assert_golden(golden("gallery_lists"), &first);
    assert!(run.text().contains("Widget gallery"));
    assert_eq!(run.scene.top().unwrap().focus_kind(), Some("tabs"));

    // Move the focus through the first page: list, menu, tree, table.
    let mut seen = Vec::new();
    for _ in 0..4 {
        assert!(!run.press(&["tab"]));
        seen.push(run.focus().unwrap().to_owned());
    }
    assert_eq!(seen, ["list", "menu", "tree", "table"]);
    run.press(&["shift+tab", "shift+tab", "shift+tab"]);
    assert_eq!(run.focus(), Some("list"));

    // Scroll the list by moving its selection past what fits.
    run.press(&["page_down", "page_down"]);
    assert!(
        run.text().contains("Whistle"),
        "the list scrolled to its end"
    );
    assert_golden(golden("gallery_list_scrolled"), run.front.last().unwrap());

    // A menu hotkey works from anywhere on the page.
    run.press(&["o"]);
    assert_eq!(run.focus(), Some("menu"));

    // The second page: a scroll that follows the focus.
    run.press(&["shift+tab", "shift+tab", "right"]);
    assert_eq!(run.focus(), Some("pages"));
    assert_golden(golden("gallery_controls"), run.front.last().unwrap());
    assert!(!run.text().contains("The last thing on the page"));
    run.press(&["tab", "tab", "tab", "tab", "tab", "tab"]);
    assert_eq!(run.focus(), Some("bottom"));
    assert!(
        run.text().contains("The last thing on the page"),
        "the scroll followed the focus"
    );
    assert_golden(
        golden("gallery_controls_scrolled"),
        run.front.last().unwrap(),
    );

    // Typing in the input does not quit, though `q` is bound to `back`.
    run.press(&[
        "shift+tab",
        "shift+tab",
        "shift+tab",
        "shift+tab",
        "shift+tab",
    ]);
    assert_eq!(run.focus(), Some("name"));
    assert!(!run.press(&["q"]));
    assert!(run.text().contains("Maraq"));

    // The remaining pages.
    run.press(&["shift+tab", "right"]);
    assert_golden(golden("gallery_views"), run.front.last().unwrap());
    assert!(run.text().contains("Log line 12."));
    run.press(&["right"]);
    assert_golden(golden("gallery_debug"), run.front.last().unwrap());

    // Quit with q, and with Esc.
    assert!(run.press(&["q"]));
    assert!(run.press(&["escape"]));
    let expected = f64::from(run.turns) * STEP;
    assert!(
        (run.scene.elapsed() - expected).abs() < 1e-9,
        "one fixed step per turn"
    );
}

#[test]
fn the_gallery_follows_the_size_of_the_grid() {
    let mut run = Run::new(100, 30);
    let large = run.idle().clone();
    assert_golden(golden("gallery_lists_100x30"), &large);

    run.front
        .script([InputEvent::Resize { cols: 60, rows: 20 }]);
    let small = run.idle().clone();
    assert_eq!((small.cols, small.rows), (60, 20));
    assert_golden(golden("gallery_too_small"), &small);
    assert!(small.text().contains("at least 80 by 24"));

    run.front
        .script([InputEvent::Resize { cols: 80, rows: 24 }]);
    let back = run.idle().clone();
    let mut fresh = Run::new(80, 24);
    assert_eq!(
        &back,
        fresh.idle(),
        "a resized screen is the same as one made at that size"
    );

    run.front.script([InputEvent::Close]);
    let turn = run.scene.turn(&mut run.front, &mut run.frame, STEP);
    assert!(turn.close_requested);
}

#[test]
fn scripted_runs_are_reproducible() {
    let script = [
        "tab", "down", "down", "tab", "tab", "right", "tab", "down", "left", "right", "tab",
    ];
    let frames = || {
        let mut run = Run::new(80, 24);
        run.press(&script);
        run.front.frames().to_vec()
    };
    assert_eq!(
        frames(),
        frames(),
        "the same script gives the same frames (12 invariant V8)"
    );
}

#[test]
fn the_terminal_shows_the_same_characters_in_the_same_cells() {
    let mut run = Run::new(80, 24);
    let frame = run.idle().clone();
    let mut out = Vec::new();
    Renderer::new(ColorDepth::Ansi16)
        .render(&frame, &mut out)
        .unwrap();
    let text = String::from_utf8(out).unwrap();

    // Strip the escape sequences; what is left is the grid, row by row.
    let mut shown = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for end in chars.by_ref() {
                if end.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            shown.push(ch);
        }
    }
    assert_eq!(shown, frame.text().replace('\n', ""));

    // With sixteen colours only the fallbacks of the `ui` mod's colours
    // appear: amber is yellow (33), ash is bright black (90).
    assert!(
        text.contains(";33;") && text.contains(";90;"),
        "fallback colours are used"
    );
    assert!(
        !text.contains("38;2;"),
        "no 24-bit colour on a sixteen-colour terminal"
    );

    // A second frame that changes one row writes far less than the first.
    run.press(&["tab", "down"]);
    let mut renderer = Renderer::new(ColorDepth::TrueColor);
    let mut first = Vec::new();
    renderer.render(&frame, &mut first).unwrap();
    let mut second = Vec::new();
    renderer
        .render(run.front.last().unwrap(), &mut second)
        .unwrap();
    assert!(!second.is_empty());
    assert!(
        second.len() * 2 < first.len(),
        "only what changed is written: {} of {}",
        second.len(),
        first.len()
    );
    assert_eq!(run.front.caps().color, ColorDepth::TrueColor);
}

#[test]
fn the_example_exits_cleanly_without_a_terminal() {
    // Cargo builds examples before running integration tests and tells us
    // where the test binary is; the example is beside it.
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    let example = path
        .join("examples")
        .join(format!("gallery{}", std::env::consts::EXE_SUFFIX));
    if !example.exists() {
        // Built some other way (for example with `--tests` alone).
        return;
    }
    let output = Command::new(&example)
        .arg(mod_dir())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Widget gallery"), "{stdout}");
    assert!(stdout.contains("not both a terminal"), "{stdout}");
    assert!(
        !stdout.contains('\x1b'),
        "no escape sequences without a terminal"
    );

    let missing = Command::new(&example)
        .arg(mod_dir().join("no-such-mod"))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        stderr.contains("gallery: ") && stderr.contains("theme `default`: not defined"),
        "{stderr}"
    );
}
