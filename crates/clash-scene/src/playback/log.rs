//! [`MessageLog`]: the message log (12 §7.4).
//!
//! Lines from `show` definitions and from `message` effects go here,
//! each with the tick it is about, a category and an importance. The
//! `log` widget draws it: wrapped to the widget's width, with inline
//! styles, following the newest line unless it has been scrolled back,
//! and filtered by category and by importance.
//!
//! # Folding
//!
//! A line that says exactly what the line before it says (the same
//! text in the same styles, category and importance) is not added
//! again: the line before counts one more and takes the newer tick. The
//! widget writes the count after the text: `The door rattles. (x3)`.
//!
//! # Scrollback
//!
//! The log keeps its newest [`LOG_LIMIT_DEFAULT`] lines unless told
//! otherwise ([`MessageLog::set_limit`]); older ones are dropped.
//!
//! ```
//! use clash_scene::playback::{Importance, MessageLog};
//! use clash_scene::RichText;
//!
//! let mut log = MessageLog::new();
//! log.push("world", RichText::plain_text("The door rattles."));
//! log.push("world", RichText::plain_text("The door rattles."));
//! log.add("world", RichText::plain_text("The wall gives way."), None, Importance::Major);
//! assert_eq!(log.lines().len(), 2);
//! assert_eq!(log.lines()[0].count, 2);
//! assert_eq!(log.lines()[1].importance, Importance::Major);
//! ```

use clash_core::Tick;
use serde::Deserialize;

use crate::text::RichText;

/// How many lines a log keeps unless told otherwise.
pub const LOG_LIMIT_DEFAULT: usize = 2000;

/// The category of a line that is given none: what a `show` definition's
/// `log` goes under unless it names a `category`.
pub const CATEGORY_EVENT: &str = "event";

/// How much a line, or the showing of an event, matters (12 §7.3). A
/// `log` widget may show only lines of at least some importance, and
/// the preference `effects` may play only the animations of what is
/// `major`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    /// Detail: a step, a door, a scuff.
    Minor,
    /// What a player would want to have read.
    #[default]
    Normal,
    /// What must not be missed.
    Major,
}

impl Importance {
    /// The name as data writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Importance::Minor => "minor",
            Importance::Normal => "normal",
            Importance::Major => "major",
        }
    }

    /// Reads a name as data writes it.
    pub fn from_name(name: &str) -> Option<Importance> {
        match name {
            "minor" => Some(Importance::Minor),
            "normal" => Some(Importance::Normal),
            "major" => Some(Importance::Major),
            _ => None,
        }
    }
}

/// One line of the message log (12 §7.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The line's category, which a `log` widget's `filter` selects by.
    pub category: String,
    /// The text.
    pub text: RichText,
    /// The tick of the event the line is about: for a folded line, of
    /// the newest. `None` for a line that is about no moment of the
    /// simulation.
    pub tick: Option<Tick>,
    /// How much the line matters.
    pub importance: Importance,
    /// How many times in a row the line was said. At least 1.
    pub count: u32,
}

impl LogLine {
    /// The text as the widget shows it: with the count after it where
    /// the line was said more than once.
    pub fn shown(&self) -> String {
        let mut text = self.text.plain();
        if self.count > 1 {
            text.push_str(&count_suffix(self.count));
        }
        text
    }
}

/// What is written after a line that was said `count` times.
pub(crate) fn count_suffix(count: u32) -> String {
    format!(" (x{count})")
}

/// The message log a `log` widget shows. See the [module](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageLog {
    lines: Vec<LogLine>,
    limit: usize,
    /// Bumped by every change, so that a widget knows when what it
    /// worked out about the log is stale.
    revision: u64,
}

impl Default for MessageLog {
    fn default() -> MessageLog {
        MessageLog {
            lines: Vec::new(),
            limit: LOG_LIMIT_DEFAULT,
            revision: 0,
        }
    }
}

impl MessageLog {
    /// An empty log.
    pub fn new() -> MessageLog {
        MessageLog::default()
    }

    /// Adds a line of ordinary importance that is about no moment of
    /// the simulation, or folds it into the last line if it says the
    /// same.
    pub fn push(&mut self, category: &str, text: RichText) {
        self.add(category, text, None, Importance::Normal);
    }

    /// Adds a line, or folds it into the last line if it says the same
    /// in the same category and importance. Returns whether it was
    /// folded.
    pub fn add(
        &mut self,
        category: &str,
        text: RichText,
        tick: Option<Tick>,
        importance: Importance,
    ) -> bool {
        self.revision = self.revision.wrapping_add(1);
        if let Some(last) = self.lines.last_mut()
            && last.category == category
            && last.importance == importance
            && last.text == text
        {
            last.count = last.count.saturating_add(1);
            last.tick = tick.or(last.tick);
            return true;
        }
        self.lines.push(LogLine {
            category: category.to_owned(),
            text,
            tick,
            importance,
            count: 1,
        });
        // Dropped a quarter of the limit at a time, so that a full log
        // does not move every line for every line added.
        let slack = (self.limit / 4).max(1);
        if self.lines.len() > self.limit + slack {
            let excess = self.lines.len() - self.limit;
            self.lines.drain(..excess);
        }
        false
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> &[LogLine] {
        &self.lines
    }

    /// How many lines there are.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the log holds no line.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Removes every line.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    /// How many lines the log keeps: its scrollback.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Sets how many lines the log keeps, at least one, and drops the
    /// oldest beyond that.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
        if self.lines.len() > self.limit {
            let excess = self.lines.len() - self.limit;
            self.lines.drain(..excess);
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// A number that changes whenever the log does.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> RichText {
        RichText::plain_text(text)
    }

    #[test]
    fn a_repeated_line_is_folded_with_a_count_and_the_newer_tick() {
        let mut log = MessageLog::new();
        assert!(!log.add("a", plain("Drip."), Some(Tick(1)), Importance::Minor));
        assert!(log.add("a", plain("Drip."), Some(Tick(5)), Importance::Minor));
        assert!(log.add("a", plain("Drip."), None, Importance::Minor));
        assert_eq!(log.len(), 1);
        assert_eq!(log.lines()[0].count, 3);
        assert_eq!(log.lines()[0].tick, Some(Tick(5)));
        assert_eq!(log.lines()[0].shown(), "Drip. (x3)");
        // Another category, another importance or another text is
        // another line.
        assert!(!log.add("b", plain("Drip."), None, Importance::Minor));
        assert!(!log.add("b", plain("Drip."), None, Importance::Normal));
        assert!(!log.add("b", plain("Drop."), None, Importance::Normal));
        // And a line is only folded into the one just before it.
        assert!(!log.add("b", plain("Drip."), None, Importance::Normal));
        assert_eq!(log.len(), 5);
        assert_eq!(log.lines()[4].shown(), "Drip.");
    }

    #[test]
    fn the_oldest_lines_are_dropped_beyond_the_limit() {
        let mut log = MessageLog::new();
        log.set_limit(8);
        for n in 0..100 {
            log.push("n", plain(&format!("line {n}")));
        }
        assert!(log.len() <= 10, "{}", log.len());
        assert_eq!(log.lines().last().unwrap().text.plain(), "line 99");
        log.set_limit(3);
        let kept: Vec<String> = log.lines().iter().map(|line| line.text.plain()).collect();
        assert_eq!(kept, ["line 97", "line 98", "line 99"]);
        let before = log.revision();
        log.clear();
        assert!(log.is_empty());
        assert_ne!(log.revision(), before);
    }

    #[test]
    fn importance_is_ordered_and_named() {
        assert!(Importance::Minor < Importance::Normal);
        assert!(Importance::Normal < Importance::Major);
        for importance in [Importance::Minor, Importance::Normal, Importance::Major] {
            assert_eq!(Importance::from_name(importance.name()), Some(importance));
        }
        assert_eq!(Importance::from_name("huge"), None);
        assert_eq!(Importance::default(), Importance::Normal);
    }
}
