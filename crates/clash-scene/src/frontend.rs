//! The [`Frontend`] trait (12 §3) and the null frontend that records
//! frames for tests (12 §11).

use std::collections::VecDeque;

use crate::frame::Frame;
use crate::input::InputEvent;

/// How many colours a frontend can show.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ColorDepth {
    /// The sixteen terminal colours: a cell's `fallback` is used.
    Ansi16,
    /// The 256-colour palette: the nearest entry to a cell's `rgb`.
    Ansi256,
    /// 24-bit colour: a cell's `rgb` exactly.
    TrueColor,
}

/// What a frontend can do. The scene may use it to decide what to put in
/// a frame; a frontend ignores whatever it cannot draw either way.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Caps {
    /// Sprites are drawn between cells and at their scale.
    pub subcell: bool,
    /// The frame's zoom, shake, flash and bars are drawn.
    pub zoom: bool,
    /// Sprites are drawn at their opacity.
    pub alpha: bool,
    /// Mouse events are delivered.
    pub mouse: bool,
    /// How many colours can be shown.
    pub color: ColorDepth,
    /// The width of the grid in cells.
    pub cols: u16,
    /// The height of the grid in cells.
    pub rows: u16,
}

/// What shows frames and delivers input: a window, a terminal, or
/// nothing at all.
pub trait Frontend {
    /// What this frontend can do, and the size of its grid now.
    fn caps(&self) -> Caps;
    /// The input that has arrived since the last call, oldest first.
    /// Never waits.
    fn poll(&mut self) -> Vec<InputEvent>;
    /// Shows a frame.
    fn present(&mut self, frame: &Frame);
}

/// A frontend that shows nothing: it hands out scripted input and records
/// the frames it is given, for tests and headless runs.
///
/// ```
/// use clash_scene::{Frame, Frontend, InputEvent, NullFrontend};
///
/// let mut front = NullFrontend::new(80, 24);
/// front.script([InputEvent::key("down").unwrap()]);
/// assert_eq!(front.poll().len(), 1);
/// assert!(front.poll().is_empty());
/// front.present(&Frame::new(80, 24));
/// assert_eq!(front.frames().len(), 1);
/// ```
#[derive(Clone, Debug)]
pub struct NullFrontend {
    cols: u16,
    rows: u16,
    script: VecDeque<Vec<InputEvent>>,
    frames: Vec<Frame>,
    keep: usize,
    presented: u64,
}

impl NullFrontend {
    /// How many frames are kept unless [`NullFrontend::keep`] says
    /// otherwise.
    pub const DEFAULT_KEEP: usize = 64;

    /// A null frontend with a grid of this size.
    pub fn new(cols: u16, rows: u16) -> NullFrontend {
        NullFrontend {
            cols,
            rows,
            script: VecDeque::new(),
            frames: Vec::new(),
            keep: NullFrontend::DEFAULT_KEEP,
            presented: 0,
        }
    }

    /// Sets how many of the latest frames are kept. At least one.
    pub fn keep(mut self, frames: usize) -> NullFrontend {
        self.keep = frames.max(1);
        self
    }

    /// Queues input to be delivered together by one later call of
    /// [`Frontend::poll`]. Each call of `script` is one such batch, so a
    /// test controls which frame each input falls before. A
    /// [`InputEvent::Resize`] also changes the size the frontend reports,
    /// when it is delivered.
    pub fn script(&mut self, events: impl IntoIterator<Item = InputEvent>) {
        self.script.push_back(events.into_iter().collect());
    }

    /// Whether scripted input is still waiting to be polled.
    pub fn has_input(&self) -> bool {
        !self.script.is_empty()
    }

    /// The frames presented, oldest first, up to the number kept.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// The latest frame presented.
    pub fn last(&self) -> Option<&Frame> {
        self.frames.last()
    }

    /// How many frames have been presented in all.
    pub fn presented(&self) -> u64 {
        self.presented
    }
}

impl Frontend for NullFrontend {
    fn caps(&self) -> Caps {
        Caps {
            subcell: true,
            zoom: true,
            alpha: true,
            mouse: true,
            color: ColorDepth::TrueColor,
            cols: self.cols,
            rows: self.rows,
        }
    }

    fn poll(&mut self) -> Vec<InputEvent> {
        let events = self.script.pop_front().unwrap_or_default();
        for event in &events {
            if let InputEvent::Resize { cols, rows } = event {
                self.cols = *cols;
                self.rows = *rows;
            }
        }
        events
    }

    fn present(&mut self, frame: &Frame) {
        if self.frames.len() >= self.keep {
            self.frames.remove(0);
        }
        self.frames.push(frame.clone());
        self.presented += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_input_arrives_one_batch_per_poll() {
        let mut front = NullFrontend::new(10, 5);
        front.script([InputEvent::key("a").unwrap(), InputEvent::key("b").unwrap()]);
        front.script([InputEvent::Resize { cols: 20, rows: 8 }]);
        assert!(front.has_input());
        assert_eq!(front.poll().len(), 2);
        assert_eq!((front.caps().cols, front.caps().rows), (10, 5));
        assert_eq!(front.poll(), [InputEvent::Resize { cols: 20, rows: 8 }]);
        assert_eq!((front.caps().cols, front.caps().rows), (20, 8));
        assert!(front.poll().is_empty());
        assert!(!front.has_input());
    }

    #[test]
    fn only_the_latest_frames_are_kept() {
        let mut front = NullFrontend::new(2, 1).keep(2);
        for cols in 1..=3 {
            front.present(&Frame::new(cols, 1));
        }
        assert_eq!(front.presented(), 3);
        let kept: Vec<u16> = front.frames().iter().map(|f| f.cols).collect();
        assert_eq!(kept, [2, 3]);
        assert_eq!(front.last().unwrap().cols, 3);
    }
}
