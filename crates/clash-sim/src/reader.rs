//! Text for a reader (05 §4.6): what the text helpers `name`, `the`,
//! `a`, `they`, `them`, `their`, `themself`, `verb`, `count`, `plural`
//! and `list` give.
//!
//! Text shown to a player is written for a **reader**: the character in
//! whose log it appears. The reader is "you"; anyone else is named as
//! the reader knows them. What a reader knows of another is asked of a
//! [`ReaderLens`], a function the simulation holds: until perception
//! and personas are built (15 §8) the lens every simulation starts with
//! answers with the truth, an entity's own name, and a package or a
//! test puts another in its place with
//! [`Sim::set_reader_lens`](crate::Sim::set_reader_lens).
//!
//! The words are English, built in: `verb` and `plural` follow the
//! regular forms and a short list of irregular ones. A `language`
//! definition (05 §4.6) will supply them once translation is built.
//!
//! | Helper | For the reader | For another |
//! |--------|----------------|-------------|
//! | `name(x)` | `you` | Its name, or how the reader would describe it |
//! | `the(x)`, `a(x)` | `you` | A proper name as it is; a description with its article: `the guard`, `an old guard` |
//! | `they(x)`, `them(x)`, `their(x)`, `themself(x)` | `you`, `you`, `your`, `yourself` | `they`, `them`, `their`, `themself` |
//! | `verb(x, word)` | The word as it is: `you strike` | The form that goes with one other: `strikes`, `is`, `has`, `carries` |
//! | `count(n, word)` | | `1 arrow`, `3 arrows` |
//! | `plural(word)` | | `arrows`, `boxes`, `bodies` |
//! | `list(xs)` | | `a knife, a rope and Mara` |

use clash_core::{EntityId, Real};
use clash_expr::Value;

use crate::Sim;
use crate::component::Identity;
use crate::event::Subject;

/// What a reader calls something.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Known {
    /// By a name of its own, which takes no article: `Mara`.
    Proper(String),
    /// By a description, which takes one: `tall figure in red`.
    Common(String),
}

/// What a reader knows of another entity: asked for every name a text
/// for that reader holds. `None` for something the reader has no word
/// for. It reads the simulation and changes nothing; it is not state,
/// and a loaded simulation has the one every simulation starts with.
pub type ReaderLens = fn(&Sim, EntityId, EntityId) -> Option<Known>;

/// The lens every simulation starts with: an entity goes by its own
/// name, whoever reads. Truth, until perception supplies belief
/// (01 R11).
pub fn plain_lens(sim: &Sim, _reader: EntityId, of: EntityId) -> Option<Known> {
    sim.components()
        .get::<Identity>(of)
        .and_then(Identity::name)
        .map(|name| Known::Proper(name.to_owned()))
        .or_else(|| kind_name(sim, of))
}

/// What a thing with no name of its own is called by the name of what
/// it was made from (05 §4.6; decision D-862): an item by its
/// definition's `name`, a stack of more than one by how many and the
/// plural, `12 arrows`; a device by its definition's; anything else
/// made from templates by the name of the first, `person`. `None` for
/// what has none of them, which reads "something".
pub fn kind_name(sim: &Sim, of: EntityId) -> Option<Known> {
    let render = |text: &clash_data::Text| {
        let mut host = sim.host(crate::host::Bound::new());
        text.render_on(sim.defs(), &mut host)
    };
    if let Some(item) = sim.components().get::<crate::item::Item>(of) {
        let name = crate::item::def_of(sim.defs(), item)
            .and_then(|def| def.name.as_ref())
            .map(render)
            .filter(|name| !name.trim().is_empty())?;
        let count = item.count();
        return Some(if count > 1 {
            Known::Proper(format!("{count} {}", plural(&name)))
        } else {
            Known::Common(name)
        });
    }
    if let Some(def) = crate::device::def_of(sim, of) {
        return def
            .name
            .as_ref()
            .map(render)
            .filter(|name| !name.trim().is_empty())
            .map(Known::Common);
    }
    let first = sim.components().get::<Identity>(of)?.templates().first()?;
    let template = sim
        .defs()
        .try_registry::<crate::template::TemplateDef>()?
        .try_get(*first)?;
    template
        .name
        .as_ref()
        .map(render)
        .filter(|name| !name.trim().is_empty())
        .map(|name| Known::Common(name.to_lowercase()))
}

/// What is said of something nobody has a word for.
const UNNAMED: &str = "something";

/// The form of a verb that goes with one other than the speaker and the
/// one spoken to: `strike` gives `strikes`.
///
/// ```
/// use clash_sim::reader::third_person;
///
/// assert_eq!(third_person("strike"), "strikes");
/// assert_eq!(third_person("be"), "is");
/// assert_eq!(third_person("have"), "has");
/// assert_eq!(third_person("carry"), "carries");
/// assert_eq!(third_person("pass"), "passes");
/// assert_eq!(third_person("go"), "goes");
/// assert_eq!(third_person("play"), "plays");
/// ```
pub fn third_person(verb: &str) -> String {
    match verb {
        "be" | "are" => "is".to_owned(),
        "have" => "has".to_owned(),
        "do" => "does".to_owned(),
        "go" => "goes".to_owned(),
        "" => String::new(),
        _ => sibilant_form(verb),
    }
}

/// The form of a verb that goes with "you": the verb itself, and `are`
/// for `be`.
fn second_person(verb: &str) -> String {
    match verb {
        "be" | "is" => "are".to_owned(),
        _ => verb.to_owned(),
    }
}

/// A word with the ending English gives both a plural noun and a verb
/// after "he" or "she": `es` after a hiss, `ies` for a consonant and
/// `y`, and `s` otherwise.
fn sibilant_form(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    let hiss = ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|ending| lower.ends_with(ending));
    if hiss {
        return format!("{word}es");
    }
    let mut letters = lower.chars().rev();
    if let (Some('y'), Some(before)) = (letters.next(), letters.next())
        && !"aeiou".contains(before)
    {
        let stem = &word[..word.len() - 1];
        return format!("{stem}ies");
    }
    format!("{word}s")
}

/// More than one of something: `arrow` gives `arrows`. A name of the
/// form "this of that" takes the plural of its first part: `strip of
/// cloth` gives `strips of cloth`, not `strip of cloths` (decision
/// D-972).
///
/// ```
/// use clash_sim::reader::plural;
///
/// assert_eq!(plural("arrow"), "arrows");
/// assert_eq!(plural("box"), "boxes");
/// assert_eq!(plural("body"), "bodies");
/// assert_eq!(plural("key"), "keys");
/// assert_eq!(plural("man"), "men");
/// assert_eq!(plural("strip of cloth"), "strips of cloth");
/// assert_eq!(plural("bunch of herbs"), "bunches of herbs");
/// assert_eq!(plural("pot of healing salve"), "pots of healing salve");
/// ```
pub fn plural(word: &str) -> String {
    if let Some((head, rest)) = word.split_once(" of ")
        && !head.is_empty()
    {
        return format!("{} of {rest}", plural(head));
    }
    match word {
        "man" => "men".to_owned(),
        "woman" => "women".to_owned(),
        "child" => "children".to_owned(),
        "person" => "people".to_owned(),
        "foot" => "feet".to_owned(),
        "tooth" => "teeth".to_owned(),
        "knife" => "knives".to_owned(),
        "" => String::new(),
        _ => sibilant_form(word),
    }
}

/// A description with the article that goes before it: `a guard`, `an
/// old guard`.
///
/// ```
/// use clash_sim::reader::with_indefinite;
///
/// assert_eq!(with_indefinite("guard"), "a guard");
/// assert_eq!(with_indefinite("old guard"), "an old guard");
/// ```
pub fn with_indefinite(noun: &str) -> String {
    let vowel = noun
        .chars()
        .next()
        .is_some_and(|first| "aeiouAEIOU".contains(first));
    format!("{} {noun}", if vowel { "an" } else { "a" })
}

/// Several things in a row: `a knife, a rope and Mara`.
///
/// ```
/// use clash_sim::reader::in_a_row;
///
/// assert_eq!(in_a_row(&[]), "");
/// assert_eq!(in_a_row(&["a knife".to_owned()]), "a knife");
/// assert_eq!(
///     in_a_row(&["a knife".to_owned(), "a rope".to_owned(), "Mara".to_owned()]),
///     "a knife, a rope and Mara"
/// );
/// ```
pub fn in_a_row(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((only, [])) => only.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// A line with a capital first letter, as a finished line has
/// (05 §4.6).
pub fn capitalised(line: &str) -> String {
    let mut letters = line.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// Which article a helper puts before a description.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Article {
    None,
    The,
    A,
}

impl Sim {
    /// Puts another lens in place of the one that says what a reader
    /// knows of someone (see the [module documentation](self)). Not
    /// state: whoever sets one sets it again after a load.
    pub fn set_reader_lens(&mut self, lens: ReaderLens) {
        self.rt.lens = lens;
    }

    /// What a reader calls something, with an article where one
    /// belongs.
    fn called(&self, value: &Value, reader: Option<EntityId>, article: Article) -> String {
        let known = match self.subject_of(value) {
            Some(Subject::Entity(id)) => {
                if reader == Some(id) {
                    return "you".to_owned();
                }
                match reader {
                    Some(reader) => (self.rt.lens)(self, reader, id),
                    None => plain_lens(self, id, id),
                }
            }
            Some(Subject::Site(id)) => self
                .truth()
                .cosmos()
                .site(id)
                .map(|site| Known::Proper(site.name.clone())),
            _ => match value {
                Value::Str(text) => Some(Known::Common(text.to_string())),
                Value::Subject(_) | Value::Null => None,
                other => Some(Known::Proper(other.to_string())),
            },
        };
        match (known, article) {
            (None, _) => UNNAMED.to_owned(),
            (Some(Known::Proper(name)), _) | (Some(Known::Common(name)), Article::None) => name,
            (Some(Known::Common(noun)), Article::The) => format!("the {noun}"),
            (Some(Known::Common(noun)), Article::A) => with_indefinite(&noun),
        }
    }

    /// Whether a value is the reader: the one the text says "you" of.
    fn is_reader(&self, value: &Value, reader: Option<EntityId>) -> bool {
        match (self.subject_of(value), reader) {
            (Some(Subject::Entity(id)), Some(reader)) => id == reader,
            _ => false,
        }
    }

    /// What one of the text helpers of 05 §4.6 gives, for a reader if
    /// the text has one. `None` for a name that is not one of them.
    pub(crate) fn reader_text(
        &self,
        helper: &str,
        first: &Value,
        second: Option<&Value>,
        reader: Option<EntityId>,
    ) -> Option<String> {
        let word = second.and_then(Value::as_str).unwrap_or("");
        let you = self.is_reader(first, reader);
        let pronoun = |yours: &str, theirs: &str| {
            if you {
                yours.to_owned()
            } else {
                theirs.to_owned()
            }
        };
        Some(match helper {
            "name" => self.called(first, reader, Article::None),
            "the" => self.called(first, reader, Article::The),
            "a" => self.called(first, reader, Article::A),
            "they" => pronoun("you", "they"),
            "them" => pronoun("you", "them"),
            "their" => pronoun("your", "their"),
            "themself" => pronoun("yourself", "themself"),
            "verb" => {
                if you {
                    second_person(word)
                } else {
                    third_person(word)
                }
            }
            "count" => {
                let one = first.as_number() == Some(Real::ONE);
                format!(
                    "{first} {}",
                    if one { word.to_owned() } else { plural(word) }
                )
            }
            "plural" => plural(first.as_str().unwrap_or("")),
            "list" => {
                let names: Vec<String> = first
                    .as_list()
                    .unwrap_or(&[])
                    .iter()
                    .map(|item| self.called(item, reader, Article::A))
                    .collect();
                in_a_row(&names)
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_agree_with_who_does_it() {
        for (verb, third) in [
            ("hit", "hits"),
            ("strike", "strikes"),
            ("be", "is"),
            ("have", "has"),
            ("do", "does"),
            ("go", "goes"),
            ("carry", "carries"),
            ("stay", "stays"),
            ("watch", "watches"),
            ("push", "pushes"),
            ("miss", "misses"),
            ("fix", "fixes"),
            ("lose", "loses"),
            ("need", "needs"),
            ("see", "sees"),
        ] {
            assert_eq!(third_person(verb), third);
        }
        assert_eq!(second_person("be"), "are");
        assert_eq!(second_person("strike"), "strike");
    }

    #[test]
    fn nouns_take_their_plurals_and_articles() {
        for (one, many) in [
            ("arrow", "arrows"),
            ("box", "boxes"),
            ("body", "bodies"),
            ("day", "days"),
            ("torch", "torches"),
            ("glass", "glasses"),
            ("knife", "knives"),
            ("person", "people"),
        ] {
            assert_eq!(plural(one), many);
        }
        assert_eq!(with_indefinite("apple"), "an apple");
        assert_eq!(
            with_indefinite("tall figure in red"),
            "a tall figure in red"
        );
        assert_eq!(capitalised("you wait."), "You wait.");
        assert_eq!(capitalised(""), "");
    }
}
