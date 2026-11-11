//! Diagnostics: what the loader tells a modder (04 §8).
//!
//! A [`Diagnostic`] has a severity, a stable code, the file, line and
//! column of the offending JSON, the definition and JSON path it is in,
//! a message and, where possible, a suggestion. It renders in the form of
//! 04 §8.2 with a caret under the right characters, and as JSON for
//! editors and tools.
//!
//! Every code is declared once, in [`code`], with a title and a long
//! explanation. `clash-tools explain <code>` prints the explanation, and a
//! test fails if a code has none.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::source::{Location, SourceMap, Span};

/// How serious a diagnostic is (04 §8.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Information attached to something else, or about what the loader
    /// did.
    Note,
    /// Something probably wrong. It does not stop a profile from starting.
    Warning,
    /// Something wrong. It stops a profile from starting (04 §8.3).
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Note => "note",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// A stable diagnostic code and the severity it has unless said otherwise.
///
/// The codes of this crate are the constants in [`code`]. A crate that
/// adds a pipeline stage (the expression compiler, for one) declares its
/// own with [`Code::new`] and keeps its own table of [`CodeInfo`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Code {
    /// The code as shown: a letter and four digits, such as `E0203`.
    pub id: &'static str,
    /// The severity a diagnostic with this code has by default.
    pub severity: Severity,
}

impl Code {
    /// Declares a code.
    pub const fn new(id: &'static str, severity: Severity) -> Code {
        Code { id, severity }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id)
    }
}

/// What the generated reference and `clash-tools explain` say about a
/// code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CodeInfo {
    /// The code.
    pub code: Code,
    /// The pipeline stage that reports it (04 §8.1).
    pub stage: &'static str,
    /// One line: what went wrong.
    pub title: &'static str,
    /// The long explanation: what it means, why it is a mistake and how to
    /// fix it.
    pub explanation: &'static str,
}

macro_rules! codes {
    ($( $name:ident = $id:literal, $severity:ident, $stage:literal, $title:literal, $explanation:literal; )*) => {
        /// The diagnostic codes of this crate. Each is explained in
        /// [`CODES`](super::CODES).
        pub mod code {
            use super::{Code, Severity};
            $(
                #[doc = $title]
                pub const $name: Code = Code::new($id, Severity::$severity);
            )*
        }

        /// Every code this crate can report, in code order, with its
        /// explanation.
        pub const CODES: &[CodeInfo] = &[
            $( CodeInfo { code: code::$name, stage: $stage, title: $title, explanation: $explanation }, )*
        ];
    };
}

codes! {
    BAD_MANIFEST = "E0001", Error, "mods",
        "A `mod.json` or profile file is missing, or a field in it is wrong.",
        "A mod is a directory with a `mod.json` (04 §3.2). The file must be one JSON object with at least an `id`. Each field has a fixed form: `requires` is a list of mod ids or of objects with `id` and `version`; `after`, `conflicts`, `imports`, `authors` and `experimental` are lists of strings. A profile (04 §3.3) is one object with an `id` and a list of `mods`. The message says which field is wrong and what was expected.";
    BAD_MOD_ID = "E0002", Error, "mods",
        "A mod id does not have the required form.",
        "A mod's id is its namespace (04 §3.2, §4.1). It starts with a lowercase letter and holds only lowercase letters, digits and underscores. The namespace `engine` is reserved for the engine and cannot be a mod's id.";
    DUPLICATE_MOD = "E0003", Error, "mods",
        "Two mod directories declare the same id.",
        "Each mod id may be found once in a search directory. Two directories in one search directory that declare the same id would make the result depend on which is read first, so neither is preferred: rename or remove one. (A mod found in an earlier search directory hides one with the same id in a later directory; that is reported as W0011, not as this error.)";
    MISSING_MOD = "E0004", Error, "mods",
        "A mod that is needed is not present.",
        "Every mod named in a profile's `mods` must be found in the search directories, and every mod in a mod's `requires` must be in the same profile (04 §3.2, §3.4). A profile is the whole list of what a game is made of, so a required mod is never added silently: add it to the profile's `mods`, or install it.";
    VERSION_MISMATCH = "E0005", Error, "mods",
        "A mod or the engine is not of a version another mod asks for.",
        "`\"requires\": [{ \"id\": \"core\", \"version\": \">=0.1\" }]` asks for a version of another mod, and `\"engine\": \">=0.1\"` for a version of the engine. A requirement is one or more comparisons separated by commas, each `>=`, `>`, `<=`, `<` or `=` and a version. Versions compare number by number, and a missing number counts as zero.";
    MOD_CONFLICT = "E0006", Error, "mods",
        "Two mods that may not be loaded together are both in the profile.",
        "A mod lists in `conflicts` the mods it cannot be loaded alongside (04 §3.2). Remove one of the two from the profile.";
    MOD_CYCLE = "E0007", Error, "mods",
        "Mods depend on each other in a circle.",
        "Everything in a mod's `requires`, and every present mod in its `after`, is loaded before it (04 §3.4). If following those links comes back to where it started there is no order that satisfies them all. The message names the mods in the circle; one of them must stop naming the next.";
    BAD_PROFILE = "E0008", Error, "mods",
        "A profile cannot be found or names something that does not exist.",
        "A profile is looked for by file path, or by name in `profiles/<name>.json` beside the mods directory and in the `profiles/` directory of each mod (04 §3.3). The message says what was looked for and where.";
    BAD_VERSION = "E0009", Error, "mods",
        "A version or a version requirement is not written correctly.",
        "A version is numbers separated by dots: `0.3.0`, `1.2`. A requirement is one or more comparisons separated by commas: `>=0.1`, `>=1.0, <2.0`, `=0.3.0`. A bare version means exactly that version.";
    MOD_DIR_NAME = "W0010", Warning, "mods",
        "A mod's directory is not named after its id.",
        "A mod lives in `mods/<mod_id>/` (04 §3.1). The mod loads under the id in its `mod.json` whatever the directory is called, but a directory with another name is hard to find from an error message. Rename the directory.";
    MOD_SHADOWED = "W0011", Warning, "mods",
        "A mod is hidden by one with the same id in an earlier search directory.",
        "Mods are looked for in each search directory in turn, and the first directory that holds a given id supplies it (04 §3.1). A copy in a later directory is ignored. This is how a working copy in the user's directory replaces an installed mod; if it is not intended, remove one.";
    MALFORMED_JSON = "E0101", Error, "syntax",
        "A file is not valid JSON.",
        "The text could not be read as JSON. The caret is at the first character that makes no sense. Common causes: a comma after the last element of a list or object, a missing comma between elements, a key without quotation marks, single quotation marks, a `//` or `/* */` comment (write a comment as a member with the key \"//\"), a quantity written without quotation marks (`250 ms` must be `\"250 ms\"`). Nothing after the first syntax error in a file is read, so fixing it may reveal more.";
    DUPLICATE_KEY = "E0102", Error, "syntax",
        "The same key appears twice in one object.",
        "JSON does not say which of two members with the same key counts, and tools disagree. The loader keeps the first and reports the second. Remove one. The comment key \"//\" is the exception and may be repeated.";
    FILE_TOO_LARGE = "E0103", Error, "syntax",
        "A file is larger than the loader will read.",
        "Loading has a limit on file size so that a malformed file cannot exhaust memory (04 §15). Split the file: a mod may have any number of data files in any directory structure.";
    NESTING_TOO_DEEP = "E0104", Error, "syntax",
        "Lists and objects are nested deeper than the loader allows.",
        "Loading has a limit on nesting depth so that a malformed file cannot exhaust the stack (04 §15). No definition needs nesting this deep; look for an unclosed bracket repeated by a generator.";
    TOO_MANY_DEFINITIONS = "E0105", Error, "structure",
        "The mods hold more definitions than the loader allows.",
        "Loading has a limit on the number of definitions (04 §15), and each definition type has as many ids as its dense id is wide: a type whose ids are sixteen bits holds 65,536 definitions. The message says which limit was reached.";
    UNREADABLE_FILE = "E0106", Error, "syntax",
        "A file or directory could not be read.",
        "The operating system refused to open a file or list a directory, or the file is not UTF-8 text. The message carries the system's reason. Check that the path exists, that it can be read, and that the file is saved as UTF-8.";
    NOT_A_DEFINITION = "E0107", Error, "structure",
        "A data file holds something that is not a definition.",
        "A data file is a JSON list of objects, or a single object (04 §5). Each object is one definition. A string, a number or a nested list in that position is not.";
    TYPE_MISMATCH = "E0201", Error, "typed load",
        "A value is of the wrong kind for its field.",
        "The field expects one kind of JSON value and was given another: a string where a number belongs, a number where a list belongs. The message says what was expected and what was found. Remember that a quantity with a unit is a string (`\"70 kPa\"`) and a bare number is in SI units.";
    MISSING_FIELD = "E0202", Error, "typed load",
        "A required field is missing.",
        "The definition type requires this field and gives it no default. Add it. If the definition uses `copy_from`, the field must be present in the parent or in the definition itself. The caret points at the object that should hold the field.";
    UNKNOWN_KEY = "E0203", Error, "typed load",
        "A key is not one the definition type knows.",
        "Unknown keys are errors (04 §5), because a misspelt key would otherwise be silently ignored and the field would keep its default. If a known key is close to the one written, the diagnostic suggests it. To leave a note in a definition, use the comment key \"//\".";
    BAD_UNIT = "E0204", Error, "typed load",
        "A quantity is not a number followed by a known unit.",
        "A quantity is a number in SI units, or a string holding a number and a unit from the fixed list (04 §7.1): `us ms s min h d`; `mm cm m km`; `ml L`; `g kg t`; `J kJ MJ GJ`; `W kW MW`; `N kN`; `Pa kPa MPa GPa`; `K C`; `m/s km/h`. A rate is a unit, or nothing, followed by `/s`, `/min`, `/h` or `/d`: `\"0.012 kg/s\"`, `\"0.15 /h\"`. Units are case-sensitive: `MJ` is a megajoule and `mJ` is not a unit.";
    WRONG_DIMENSION = "E0205", Error, "typed load",
        "A quantity has a unit of the wrong dimension for its field.",
        "The unit is a real unit, but it measures something else than the field holds: a mass where a length belongs. The message names both dimensions and lists the units the field accepts.";
    UNKNOWN_VARIANT = "E0206", Error, "typed load",
        "A value is not one of the fixed set its field allows.",
        "The field takes one of a fixed set of strings, and the value written is not among them. The message lists the set, and suggests the closest.";
    INVALID_VALUE = "E0207", Error, "typed load",
        "A value is of the right kind but is not acceptable.",
        "The value could be read but the field's type refused it: a list of the wrong length, an empty string where a name is needed, a value its type cannot hold. The message carries the type's reason.";
    NUMBER_OUT_OF_RANGE = "E0208", Error, "typed load",
        "A number does not fit the field.",
        "The field holds a whole number of limited size, and the number written is negative where only positive numbers fit, too large, or has a fraction.";
    MISSING_TYPE = "E0301", Error, "structure",
        "A definition has no `type`.",
        "Every definition is an object with a `type` and an `id` (04 §5). `type` is a string naming the definition type: `\"material\"`, `\"block\"`.";
    MISSING_ID = "E0302", Error, "structure",
        "A definition has no `id`.",
        "Every definition is an object with a `type` and an `id` (04 §5). `id` is a string: a bare name defines it in the mod's own namespace.";
    UNKNOWN_TYPE = "E0303", Error, "structure",
        "A definition's `type` is not a definition type the engine knows.",
        "Definition types are registered by the engine; data cannot add one. If a known type is close to the one written, the diagnostic suggests it.";
    BAD_ID = "E0304", Error, "structure",
        "An id does not have the required form.",
        "A qualified id is `namespace:name`, and a bare id is `name` (04 §4.1). Each part starts with a lowercase letter and holds only lowercase letters, digits and underscores.";
    DUPLICATE_ID = "E0305", Error, "structure",
        "An id is defined twice for the same definition type.",
        "Ids are unique per definition type (04 §4.1). Defining an id that already exists is an error that says where the first definition is. To change an existing definition use `\"patch\": true`; to replace it use `\"override\": true` (04 §6).";
    FOREIGN_NAMESPACE = "E0306", Error, "structure",
        "A mod defines an id in a namespace that is not its own.",
        "A mod may define new ids only in its own namespace (04 §4.2). An id qualified with another namespace is allowed only with `\"patch\": true` or `\"override\": true`, which change a definition that already exists. The namespace `engine` is reserved and can be neither defined in nor changed.";
    BAD_PATCH_TARGET = "E0307", Error, "structure",
        "A patch names a definition that does not exist at that point.",
        "`\"patch\": true` changes an existing definition in place (04 §6.3). The definition must have been defined by a mod loaded earlier, or earlier in this mod. If the target is in another mod, list that mod in `requires` or `after` so that it loads first. If the target may legitimately be absent, add `\"optional\": true` and the patch is skipped when it is.";
    BAD_OVERRIDE_TARGET = "E0308", Error, "structure",
        "An override names a definition that does not exist at that point.",
        "`\"override\": true` replaces an existing definition (04 §6.4). The definition must have been defined by a mod loaded earlier, or earlier in this mod. To define something new, leave `override` out.";
    BAD_DIRECTIVE = "E0310", Error, "structure",
        "One of the keys every definition may carry has a wrong value.",
        "`abstract`, `patch`, `override`, `optional` and `obsolete` are `true` or `false`. `copy_from` is an id. `if_mod` and `unless_mod` are a mod id or a list of them. `extend`, `delete`, `scale` and `add` are objects whose keys are field paths. A definition cannot be both a patch and an override, and `optional` only means something on a patch.";
    BAD_EXTEND = "E0401", Error, "merge",
        "`extend` cannot be applied.",
        "`\"extend\": { \"tags\": [\"x\"] }` appends to a list or set and merges into a map (04 §6.1). The value given must be a list when the field is a list and an object when the field is a map, and the field must be a list or a map.";
    BAD_DELETE = "E0402", Error, "merge",
        "`delete` cannot be applied.",
        "`\"delete\": { \"tags\": [\"y\"] }` removes entries from a list or set and keys from a map (04 §6.1). It takes a list: of values for a plain list, of entry ids for a list of objects with ids, of keys for a map. Deleting something that is not there is an error, because it usually means the thing was renamed and the patch no longer does what it was written to do.";
    BAD_SCALE = "E0403", Error, "merge",
        "`scale` cannot be applied.",
        "`\"scale\": { \"density\": 1.2 }` multiplies a number (04 §6.1). The factor must be a number, and the field must hold a number or a quantity with a unit. On a field the definition leaves out, `scale` acts on the field's default if the type declares one and is an error otherwise.";
    BAD_ADD = "E0404", Error, "merge",
        "`add` cannot be applied.",
        "`\"add\": { \"span\": 2 }` adds to a number (04 §6.1). The amount must be a number or a quantity with a unit, of the same dimension as the field's value if that was written with a unit. On a field the definition leaves out, `add` acts on the field's default if the type declares one and is an error otherwise.";
    INHERITANCE_CYCLE = "E0405", Error, "merge",
        "Definitions inherit from each other in a circle.",
        "`copy_from` gives a definition a parent (04 §6.2). If following parents comes back to where it started, none of them can be finished. The message names the definitions in the circle.";
    UNKNOWN_PARENT = "E0406", Error, "merge",
        "`copy_from` names a definition that does not exist.",
        "The parent of a definition must be a definition of the same type, in this mod or in a namespace it imports (04 §4.3, §6.2). It may be defined later or in another mod, and it may be abstract. If a definition of that type with a similar id exists, the diagnostic suggests it.";
    PARENT_TYPE_MISMATCH = "E0407", Error, "merge",
        "`copy_from` names a definition of another type.",
        "A definition inherits from one of the same type (04 §6.2): a block cannot copy from a material. The id written exists, but as another definition type.";
    BAD_PATH = "E0408", Error, "merge",
        "A field path in `extend`, `delete`, `scale` or `add` leads nowhere.",
        "Keys inside `extend`, `delete`, `scale` and `add` may be dotted paths (04 §6.1). Each step names a key of an object, or the id of an entry in a list of objects that carry ids: `phases.windup.time`. The path given steps into something that is not an object or such a list, or names an entry that is not there.";
    BAD_ANCHOR = "E0409", Error, "merge",
        "`before` or `after` names a list entry that is not there.",
        "In a list of objects that carry ids, a new entry is appended unless it says `\"before\": id` or `\"after\": id` (04 §6.1). The id given is not the id of an entry in the list.";
    DISPUTED_FIELD = "W0410", Warning, "cross",
        "Two mods replace the same field with different values.",
        "Two mods that `scale`, `add` to or `extend` a field compose. Two mods that replace it do not: the later wins, silently undoing the earlier (04 §6.3). If the later mod knows of the earlier and means to win, it says so by listing the earlier in its `requires` or `after`, and the warning goes away.";
    UNKNOWN_REFERENCE = "E0501", Error, "link",
        "A reference names a definition that does not exist.",
        "A bare name in a reference is looked up in the namespace of the mod that wrote it, then in each namespace that mod imports, in order (04 §4.3). A qualified reference is looked up exactly. Nothing of the expected type was found under the name. The diagnostic suggests the closest existing ids of that type.";
    ABSTRACT_REFERENCE = "E0502", Error, "link",
        "A reference names an abstract definition.",
        "A definition marked `\"abstract\": true` exists only to be inherited from. It is never instantiated and cannot be referred to except by `copy_from` (04 §5).";
    BAD_REFERENCE = "E0503", Error, "link",
        "A reference is not written as an id.",
        "A reference is a bare name or `namespace:name` (04 §4.1). Inline definitions have generated ids containing `#` and cannot be referred to from elsewhere (04 §5); give the inline definition an `id` of its own, or move it out to a definition of its own.";
    SHADOWED_IMPORT = "W0504", Warning, "link",
        "A bare reference matches in more than one imported namespace.",
        "A bare name is looked up in the mod's own namespace and then in each import in order, and the first match wins (04 §4.3). The name also exists in a later namespace, so which definition is meant depends on the order of `imports`. Write the reference qualified, `namespace:name`, to be clear.";
    VALIDATION = "E0701", Error, "semantic",
        "A definition was read but breaks one of its type's own rules.",
        "Each definition type checks what cannot be expressed as the kind of a single field: ranges, fields that exclude each other, limits (04 §8.1). The message is the type's own.";
    VALIDATION_WARNING = "W0702", Warning, "semantic",
        "A definition is valid but probably not what was meant.",
        "Each definition type may warn of what is allowed and usually a mistake. The message is the type's own.";
    UNUSED_ABSTRACT = "W0801", Warning, "cross",
        "An abstract definition is never inherited from.",
        "A definition marked `\"abstract\": true` exists only as a parent for `copy_from` (04 §5). Nothing copies from this one, so it has no effect. Remove it, or check the `copy_from` that was meant to name it.";
    EXPERIMENTAL_NOT_ENABLED = "E0802", Error, "cross",
        "A mod uses an experimental feature it has not opted in to.",
        "An experimental feature is specified so that it can be tried. It may change incompatibly or be removed (04 §8.4). A mod that uses one must name the feature in the `experimental` list of its `mod.json`. The message says which feature and where it is used.";
    EXPERIMENTAL_IN_USE = "W0803", Warning, "cross",
        "A mod relies on an experimental feature.",
        "The mod names an experimental feature in its `mod.json` (04 §8.4). This warning is given at every load, by name, as a reminder that the feature may change incompatibly or be removed, and that a save records which experimental features its mods used.";
    PLACEHOLDER = "N0901", Note, "lenient",
        "A broken definition was replaced by a placeholder.",
        "With `--lenient`, each definition that could not be loaded is replaced by an inert placeholder of its type and loading carries on (04 §8.3). The errors are still reported. This is for working on a large mod with something temporarily broken; the result is not fit to play or to save from.";
    NO_PLACEHOLDER = "E0902", Error, "lenient",
        "A broken definition cannot be replaced by a placeholder.",
        "`--lenient` replaces each broken definition with a placeholder (04 §8.3), but this definition type does not supply one, so loading cannot carry on past it. Fix the definition.";
}

/// The explanation of a code of this crate, or `None` if the code is not
/// one of them. Letter case is ignored.
pub fn explain(code: &str) -> Option<&'static CodeInfo> {
    CODES
        .iter()
        .find(|info| info.code.id.eq_ignore_ascii_case(code))
}

/// Which definition a diagnostic is in: its type and its qualified id.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct DefLabel {
    /// The definition type's JSON name.
    #[serde(rename = "type")]
    pub type_name: String,
    /// The qualified id.
    pub id: String,
}

impl DefLabel {
    /// A label for definition `id` of type `type_name`.
    pub fn new(type_name: impl Into<String>, id: impl Into<String>) -> DefLabel {
        DefLabel {
            type_name: type_name.into(),
            id: id.into(),
        }
    }
}

impl fmt::Display for DefLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.type_name, self.id)
    }
}

/// Something more a diagnostic has to say, with or without a place.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Note {
    /// The text.
    pub message: String,
    /// The place it is about, if any.
    pub location: Option<Location>,
}

/// One thing the loader has to say about the mods (04 §8.2).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// The stable code, such as `E0203`.
    pub code: String,
    /// What is wrong. Makes sense after `error: `.
    pub message: String,
    /// Where the offending JSON is. Absent only for a problem that has no
    /// place, such as a directory that cannot be read.
    pub location: Option<Location>,
    /// The definition it is in, if it is in one.
    pub definition: Option<DefLabel>,
    /// The JSON path inside the definition: `resist.impact.threshold`.
    /// Empty at the definition's top level.
    pub path: String,
    /// What was probably meant, as a sentence: `did you mean "toughness"?`
    pub suggestion: Option<String>,
    /// Anything more: where the first definition is, which mods dispute a
    /// field.
    pub notes: Vec<Note>,
}

impl Diagnostic {
    /// A diagnostic with a code's default severity and no place yet.
    pub fn new(code: Code, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: code.severity,
            code: code.id.to_owned(),
            message: message.into(),
            location: None,
            definition: None,
            path: String::new(),
            suggestion: None,
            notes: Vec::new(),
        }
    }

    /// Sets where the offending JSON is.
    pub fn at(mut self, sources: &SourceMap, span: Span) -> Diagnostic {
        self.location = Some(sources.locate(span));
        self
    }

    /// Sets the definition it is in.
    pub fn in_def(mut self, label: Option<&DefLabel>) -> Diagnostic {
        self.definition = label.cloned();
        self
    }

    /// Sets the JSON path inside the definition.
    pub fn with_path(mut self, path: impl Into<String>) -> Diagnostic {
        self.path = path.into();
        self
    }

    /// Sets the suggestion.
    pub fn suggest(mut self, suggestion: impl Into<String>) -> Diagnostic {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Sets the suggestion if there is one.
    pub fn suggest_opt(mut self, suggestion: Option<String>) -> Diagnostic {
        self.suggestion = suggestion;
        self
    }

    /// Adds a note with no place.
    pub fn note(mut self, message: impl Into<String>) -> Diagnostic {
        self.notes.push(Note {
            message: message.into(),
            location: None,
        });
        self
    }

    /// Adds a note about a place.
    pub fn note_at(
        mut self,
        sources: &SourceMap,
        span: Span,
        message: impl Into<String>,
    ) -> Diagnostic {
        self.notes.push(Note {
            message: message.into(),
            location: Some(sources.locate(span)),
        });
        self
    }

    /// Changes the severity from the code's default.
    pub fn with_severity(mut self, severity: Severity) -> Diagnostic {
        self.severity = severity;
        self
    }

    /// Whether this is an error.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// The diagnostic in the form of 04 §8.2:
    ///
    /// ```text
    /// error[E0203]: unknown key "thougness"
    ///   --> mods/ashlands/data/materials.json:14:5
    ///    |
    /// 14 |     "thougness": 2.0e4,
    ///    |     ^^^^^^^^^^^ did you mean "toughness"?
    ///    = in: material ashlands:ash
    /// ```
    pub fn render(&self) -> String {
        let mut out = format!("{}[{}]: {}\n", self.severity, self.code, self.message);
        let width = self
            .location
            .as_ref()
            .map_or(1, |location| location.line.to_string().len());
        let pad = " ".repeat(width);
        if let Some(location) = &self.location {
            out.push_str(&format!("{pad}--> {}\n", location.short()));
            if !location.source_line.is_empty() || location.line > 0 {
                out.push_str(&format!("{pad} |\n"));
                out.push_str(&format!("{} | {}\n", location.line, location.source_line));
                let line_len = location.source_line.chars().count();
                let before = (location.column.saturating_sub(1) as usize).min(line_len);
                let carets = if location.end_line == location.line {
                    location.end_column.saturating_sub(location.column) as usize
                } else {
                    line_len.saturating_sub(before)
                };
                out.push_str(&format!(
                    "{pad} | {}{}",
                    " ".repeat(before),
                    "^".repeat(carets.max(1))
                ));
                if let Some(suggestion) = &self.suggestion {
                    out.push(' ');
                    out.push_str(suggestion);
                }
                out.push('\n');
            }
        } else if let Some(suggestion) = &self.suggestion {
            out.push_str(&format!("{pad} = help: {suggestion}\n"));
        }
        if let Some(definition) = &self.definition {
            out.push_str(&format!("{pad} = in: {definition}"));
            if !self.path.is_empty() {
                out.push_str(&format!(", at {}", self.path));
            }
            out.push('\n');
        } else if !self.path.is_empty() {
            out.push_str(&format!("{pad} = at: {}\n", self.path));
        }
        for note in &self.notes {
            match &note.location {
                Some(location) => out.push_str(&format!(
                    "{pad} = note: {} ({})\n",
                    note.message,
                    location.short()
                )),
                None => out.push_str(&format!("{pad} = note: {}\n", note.message)),
            }
        }
        out
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// Everything one load had to say, in a fixed order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// An empty list.
    pub fn new() -> Diagnostics {
        Diagnostics::default()
    }

    /// Adds one.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    /// Adds several.
    pub fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        self.items.extend(diagnostics);
    }

    /// The diagnostics, in order.
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.items.iter()
    }

    /// The diagnostics as a slice.
    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.items
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many are errors.
    pub fn error_count(&self) -> usize {
        self.count(Severity::Error)
    }

    /// How many are warnings.
    pub fn warning_count(&self) -> usize {
        self.count(Severity::Warning)
    }

    /// How many have a given severity.
    pub fn count(&self, severity: Severity) -> usize {
        self.items
            .iter()
            .filter(|diagnostic| diagnostic.severity == severity)
            .count()
    }

    /// Whether any is an error. Errors prevent a profile from starting
    /// (04 §8.3).
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    /// The diagnostics with a given code.
    pub fn with_code(&self, code: Code) -> Vec<&Diagnostic> {
        self.items
            .iter()
            .filter(|diagnostic| diagnostic.code == code.id)
            .collect()
    }

    /// Puts the diagnostics in reading order: by file, then by position,
    /// keeping the order they were found in where those are equal. Those
    /// with no place come first. The order of files is the order of their
    /// paths, so it does not depend on how the file system lists them.
    pub fn sort(&mut self) {
        self.items.sort_by(|a, b| {
            let key = |diagnostic: &Diagnostic| {
                diagnostic
                    .location
                    .as_ref()
                    .map(|location| (location.file.clone(), location.start))
            };
            key(a).cmp(&key(b))
        });
    }

    /// All diagnostics rendered as text (04 §8.2), followed by a line that
    /// counts them. Empty when there is nothing to say.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for diagnostic in &self.items {
            out.push_str(&diagnostic.render());
            out.push('\n');
        }
        let errors = self.error_count();
        let warnings = self.warning_count();
        if errors > 0 || warnings > 0 {
            out.push_str(&format!(
                "{errors} error{}, {warnings} warning{}\n",
                if errors == 1 { "" } else { "s" },
                if warnings == 1 { "" } else { "s" },
            ));
        }
        out
    }

    /// All diagnostics as a JSON list, for editors and tools (04 §8.2).
    /// [`Diagnostics::from_json`] reads it back unchanged.
    pub fn to_json(&self) -> String {
        // Serialising plain strings and numbers cannot fail.
        serde_json::to_string_pretty(&self.items).unwrap_or_else(|_| "[]".to_owned())
    }

    /// Reads what [`Diagnostics::to_json`] wrote.
    pub fn from_json(text: &str) -> Result<Diagnostics, String> {
        serde_json::from_str::<Vec<Diagnostic>>(text)
            .map(|items| Diagnostics { items })
            .map_err(|error| error.to_string())
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<I: IntoIterator<Item = Diagnostic>>(iter: I) -> Diagnostics {
        Diagnostics {
            items: iter.into_iter().collect(),
        }
    }
}

/// The closest of `candidates` to `target`, if any is close enough to be
/// worth suggesting.
///
/// Closeness is the number of single-character insertions, deletions,
/// substitutions and swaps of neighbours needed to turn one into the
/// other. A candidate is suggested when that is at most a third of the
/// target's length (and at least one). A candidate that the target is the
/// start of, or that is the start of the target, counts as two away at
/// most: `boot` for `boot_screen`. Ties go to the candidate that comes
/// first in sorted order, so the suggestion does not depend on the order
/// candidates are supplied in.
pub fn closest<'a>(target: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    /// The fewest characters a name must share with another to be taken
    /// for a shortened form of it.
    const MIN_PREFIX: usize = 3;
    /// How far apart a name and its shortened form are counted as.
    const PREFIX_DISTANCE: usize = 2;
    let limit = (target.chars().count() / 3).max(1);
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        if candidate == target {
            continue;
        }
        let mut distance = edit_distance(target, candidate);
        let shortened = target.len().min(candidate.len()) >= MIN_PREFIX
            && (candidate.starts_with(target) || target.starts_with(candidate));
        if shortened {
            distance = distance.min(PREFIX_DISTANCE);
        } else if distance > limit {
            continue;
        }
        let better = match best {
            None => true,
            Some((best_distance, best_candidate)) => {
                distance < best_distance
                    || (distance == best_distance && candidate < best_candidate)
            }
        };
        if better {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The sentence a suggestion is shown as.
pub fn did_you_mean(candidate: Option<&str>) -> Option<String> {
    candidate.map(|candidate| format!("did you mean \"{candidate}\"?"))
}

/// Optimal string alignment distance between two strings, in characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut before: Vec<usize> = vec![0; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current: Vec<usize> = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let mut best = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
            if i > 0 && j > 0 && ca == b[j - 1] && a[i - 1] == cb {
                best = best.min(before[j - 1] + 1);
            }
            current[j + 1] = best;
        }
        std::mem::swap(&mut before, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(sources: &mut SourceMap) -> Diagnostic {
        let text = "[\n  {\n    \"type\": \"material\",\n    \"thougness\": 2.0e4,\n  }\n]";
        let file = sources.add("mods/ashlands/data/materials.json", text);
        let start = crate::source::to_u32(text.find("\"thougness\"").unwrap());
        Diagnostic::new(code::UNKNOWN_KEY, "unknown key \"thougness\"")
            .at(sources, Span::new(file, start, start + 11))
            .in_def(Some(&DefLabel::new("material", "ashlands:ash")))
            .suggest("did you mean \"toughness\"?")
    }

    #[test]
    fn a_diagnostic_renders_in_the_specified_form() {
        let mut sources = SourceMap::new();
        let diagnostic = sample(&mut sources);
        let expected = "\
error[E0203]: unknown key \"thougness\"
 --> mods/ashlands/data/materials.json:4:5
  |
4 |     \"thougness\": 2.0e4,
  |     ^^^^^^^^^^^ did you mean \"toughness\"?
  = in: material ashlands:ash
";
        assert_eq!(diagnostic.render(), expected);
    }

    #[test]
    fn the_gutter_widens_with_the_line_number() {
        let mut sources = SourceMap::new();
        let text = format!("{}\"x\": 1", "\n".repeat(13));
        let file = sources.add("a.json", text);
        let rendered = Diagnostic::new(code::TYPE_MISMATCH, "expected a string, found a number")
            .at(&sources, Span::new(file, 18, 19))
            .in_def(Some(&DefLabel::new("block", "core:wall")))
            .with_path("resist.impact")
            .note("a note")
            .render();
        let expected = "\
error[E0201]: expected a string, found a number
  --> a.json:14:6
   |
14 | \"x\": 1
   |      ^
   = in: block core:wall, at resist.impact
   = note: a note
";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn a_diagnostic_without_a_place_still_renders() {
        let rendered = Diagnostic::new(code::MISSING_MOD, "mod \"core\" was not found")
            .suggest("add it to the profile")
            .render();
        assert_eq!(
            rendered,
            "error[E0004]: mod \"core\" was not found\n  = help: add it to the profile\n"
        );
    }

    #[test]
    fn json_output_round_trips() {
        let mut sources = SourceMap::new();
        let mut diagnostics = Diagnostics::new();
        diagnostics.push(sample(&mut sources));
        let file = sources
            .file_by_path("mods/ashlands/data/materials.json")
            .unwrap();
        diagnostics.push(
            Diagnostic::new(code::DISPUTED_FIELD, "two mods replace `toughness`")
                .with_path("toughness")
                .note_at(&sources, Span::new(file, 2, 3), "first set here")
                .note("the later wins"),
        );
        diagnostics.push(Diagnostic::new(code::PLACEHOLDER, "replaced"));
        let json = diagnostics.to_json();
        let back = Diagnostics::from_json(&json).unwrap();
        assert_eq!(back, diagnostics);
        assert_eq!(back.to_json(), json);
        // The fields an editor needs are there by name.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let first = &value[0];
        assert_eq!(first["severity"], "error");
        assert_eq!(first["code"], "E0203");
        assert_eq!(
            first["location"]["file"],
            "mods/ashlands/data/materials.json"
        );
        assert_eq!(first["location"]["line"], 4);
        assert_eq!(first["location"]["column"], 5);
        assert_eq!(first["definition"]["type"], "material");
        assert_eq!(first["definition"]["id"], "ashlands:ash");
        assert_eq!(first["suggestion"], "did you mean \"toughness\"?");
        assert_eq!(value[1]["severity"], "warning");
        assert_eq!(value[2]["severity"], "note");
        assert!(Diagnostics::from_json("{").is_err());
    }

    #[test]
    fn every_code_has_an_explanation() {
        let mut seen = std::collections::BTreeSet::new();
        for info in CODES {
            let id = info.code.id;
            assert!(seen.insert(id), "code {id} is declared twice");
            assert_eq!(id.len(), 5, "code {id} is a letter and four digits");
            let letter = id.chars().next().unwrap();
            let expected = match info.code.severity {
                Severity::Error => 'E',
                Severity::Warning => 'W',
                Severity::Note => 'N',
            };
            assert_eq!(
                letter, expected,
                "code {id} starts with its severity's letter"
            );
            assert!(id[1..].chars().all(|ch| ch.is_ascii_digit()), "{id}");
            assert!(info.title.len() > 10, "code {id} needs a title");
            assert!(
                info.explanation.len() > 60,
                "code {id} needs a long explanation"
            );
            assert!(!info.stage.is_empty(), "code {id} needs a stage");
            assert_eq!(explain(id).map(|info| info.code.id), Some(id));
        }
        // In code order by number, so the reference lists them predictably.
        let numbers: Vec<&str> = CODES.iter().map(|info| &info.code.id[1..]).collect();
        let mut sorted = numbers.clone();
        sorted.sort_unstable();
        assert_eq!(numbers, sorted);
        assert_eq!(explain("e0203").unwrap().code, code::UNKNOWN_KEY);
        assert!(explain("E9999").is_none());
    }

    #[test]
    fn suggestions_pick_the_closest_candidate() {
        let keys = ["density", "toughness", "tags", "span"];
        assert_eq!(closest("thougness", keys), Some("toughness"));
        assert_eq!(closest("tag", keys), Some("tags"));
        assert_eq!(closest("spna", keys), Some("span"));
        assert_eq!(closest("glyph", keys), None);
        assert_eq!(closest("span", keys), None);
        // A tie goes to the first in sorted order, whatever order is given.
        assert_eq!(closest("ab", ["ac", "aa"]), Some("aa"));
        assert_eq!(closest("ab", ["aa", "ac"]), Some("aa"));
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("ab", "ba"), 1);
        assert_eq!(
            did_you_mean(Some("toughness")).as_deref(),
            Some("did you mean \"toughness\"?")
        );
    }

    #[test]
    fn diagnostics_sort_by_file_then_position() {
        let mut sources = SourceMap::new();
        let b = sources.add("b.json", "0123456789");
        let a = sources.add("a.json", "0123456789");
        let mut diagnostics = Diagnostics::new();
        let make = |sources: &SourceMap, file, at: u32, message: &str| {
            Diagnostic::new(code::TYPE_MISMATCH, message).at(sources, Span::new(file, at, at + 1))
        };
        diagnostics.push(make(&sources, b, 5, "b5"));
        diagnostics.push(make(&sources, a, 7, "a7"));
        diagnostics.push(Diagnostic::new(code::MISSING_MOD, "nowhere"));
        diagnostics.push(make(&sources, a, 2, "a2"));
        diagnostics.push(make(&sources, a, 2, "a2 again"));
        diagnostics.sort();
        let order: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(order, ["nowhere", "a2", "a2 again", "a7", "b5"]);
        assert_eq!(diagnostics.error_count(), 5);
        assert!(diagnostics.has_errors());
        assert!(diagnostics.render().ends_with("5 errors, 0 warnings\n"));
    }
}
