//! `start`: a way a game can begin, until game scenarios exist
//! (13 §3, 12 §10.4).
//!
//! **A stand-in.** A game scenario (13) says which worlds a game starts
//! with, who the player is and where they begin. That definition type
//! comes with the scenario packages. Until it does, a mod lists its
//! *starts*: each a name, a description, a realm definition, where the
//! character begins and how the character is drawn. The application
//! offers them as `app.scenarios`, and the `new_game` service takes the
//! id of one as its `scenario`.
//!
//! ```json
//! { "type": "start", "id": "lane", "name": "The lane",
//!   "doc": "A lane of houses between fields.",
//!   "realm": "countryside", "at": [48, 13],
//!   "glyph": { "ch": "@", "fg": "bright_white", "bold": true } }
//! ```
//!
//! When the `scenario` type lands, a start is a scenario with one
//! world, one place in `start.at` and one ready-made character, and
//! this type is deleted.

use clash_data::{CrossCx, Def, DefLabel, Diagnostic, Located, ValidateCx, code};
use clash_world::defs::RealmDef;
use serde::Deserialize;

use crate::defs::namespace_of;
use crate::glyph::GlyphDef;

clash_core::define_id! {
    /// A `start` definition.
    pub struct StartId(u16);
}

/// A `start` definition: one way a game can begin. See the
/// [module](self) documentation.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartDef {
    /// What the start is called where the player chooses one. Left out,
    /// its id.
    #[serde(default)]
    pub name: String,
    /// A line or two about it.
    #[serde(default)]
    pub doc: String,
    /// The realm definition the game's world is made from: a qualified
    /// id, or a bare name, which means the realm of that name in the
    /// start's own namespace.
    pub realm: Located<String>,
    /// Where the character begins, in cells of the realm's base grid:
    /// `[x, y]`, which means the highest cell of that column a walker
    /// can stand in, or `[x, y, z]`, which means exactly that cell.
    pub at: Vec<i32>,
    /// How the player's character is drawn, until characters are made
    /// from templates that say (06 §10).
    pub glyph: GlyphDef,
}

impl StartDef {
    /// The qualified id of the realm definition, for a start defined in
    /// `namespace`.
    pub fn realm_id(&self, namespace: &str) -> String {
        if self.realm.value.contains(':') {
            self.realm.value.clone()
        } else {
            format!("{namespace}:{}", self.realm.value)
        }
    }
}

impl Def for StartDef {
    type Id = StartId;
    const TYPE: &'static str = "start";
    const DOC: &'static str = "A way a game can begin, until game scenarios exist: a realm, where the character begins and how it is drawn (12 §10.4).";

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !(2..=3).contains(&self.at.len()) {
            cx.error(format!(
                "`at` is `[x, y]` or `[x, y, z]`, and this has {} numbers",
                self.at.len()
            ));
        }
        if self.glyph.ch.is_none() {
            cx.error("`glyph` needs a `ch`: the character the player's character is drawn as");
        }
    }
}

/// The cross check: the realm a start names must be defined.
pub(crate) fn check_starts(cx: &mut CrossCx<'_>) {
    let defs = cx.defs();
    let mut missing = Vec::new();
    for (id, start) in defs.iter::<StartDef>() {
        let Some(name) = defs.name::<StartDef>(id) else {
            continue;
        };
        let realm = start.realm_id(namespace_of(name));
        if defs.lookup::<RealmDef>(&realm).is_none() {
            missing.push((name.to_owned(), realm, start.realm.span));
        }
    }
    for (start, realm, span) in missing {
        let diagnostic = Diagnostic::new(
            code::UNKNOWN_REFERENCE,
            format!("there is no realm `{realm}` for this start to begin in"),
        )
        .in_def(Some(&DefLabel::new(StartDef::TYPE, start)))
        .with_path("realm")
        .at(cx.sources(), span);
        cx.report(diagnostic);
    }
}
