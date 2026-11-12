//! The documentation of the engine's data types, as their Rust doc
//! comments give it (decision D-801).
//!
//! `build.rs` reads the workspace's sources and writes, for every struct
//! and enum that is read from JSON, its own documentation and that of
//! each field and value under the name JSON gives it. The reference
//! joins it to what `clash_data::shape` reads from the types themselves:
//! the shapes come from the types, the words from the comments beside
//! them.

include!(concat!(env!("OUT_DIR"), "/type_docs.rs"));

/// The documentation of a Rust type: its own, and that of each field or
/// value by its JSON name.
#[derive(Clone, Copy, Debug)]
pub struct TypeDoc {
    /// The type's own documentation.
    pub doc: &'static str,
    /// Each field or value, by the name JSON gives it.
    pub items: &'static [(&'static str, &'static str)],
}

impl TypeDoc {
    /// The documentation of a field or a value, if it has any.
    pub fn item(&self, name: &str) -> Option<&'static str> {
        self.items
            .iter()
            .find(|(item, _)| *item == name)
            .map(|(_, doc)| *doc)
            .filter(|doc| !doc.is_empty())
    }
}

/// The documentation of a type, by its path as `std::any::type_name`
/// writes it: generic arguments are dropped.
pub fn find(rust: &str) -> Option<TypeDoc> {
    let path = rust.split('<').next().unwrap_or(rust);
    TYPE_DOCS
        .binary_search_by(|(known, _, _)| (*known).cmp(path))
        .ok()
        .map(|at| TYPE_DOCS[at])
        .map(|(_, doc, items)| TypeDoc { doc, items })
}

/// The first sentence of a text, for a summary.
pub fn first_sentence(text: &str) -> String {
    let paragraph = text.split("\n\n").next().unwrap_or(text);
    let mut out = String::new();
    let mut chars = paragraph.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '.' && chars.peek().is_none_or(|next| next.is_whitespace()) {
            break;
        }
    }
    out.trim().to_owned()
}

/// A doc comment made fit for Markdown outside the crate: intra-doc
/// links lose their Rust targets, and paragraphs are kept.
pub fn clean(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            // `[text](target)` keeps the text, `[text]` likewise, where
            // the text is code: a link into the Rust documentation.
            if let Some(close) = chars[i..].iter().position(|&c| c == ']') {
                let inner: String = chars[i + 1..i + close].iter().collect();
                let after = i + close + 1;
                let is_code = inner.starts_with('`') && inner.ends_with('`');
                if is_code {
                    out.push_str(&inner);
                    i = after;
                    if chars.get(i) == Some(&'(')
                        && let Some(end) = chars[i..].iter().position(|&c| c == ')')
                    {
                        i += end + 1;
                    }
                    continue;
                }
                if chars.get(after) == Some(&'(')
                    && let Some(end) = chars[after..].iter().position(|&c| c == ')')
                {
                    let target: String = chars[after + 1..after + end].iter().collect();
                    if target.contains("::")
                        || target.starts_with("crate")
                        || target.starts_with("mod@")
                    {
                        out.push_str(&inner);
                        i = after + end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_are_found_by_path_without_generic_arguments() {
        let doc = find("clash_data::fields::Text").expect("the text type is documented");
        assert!(doc.doc.contains("Player-facing text"), "{}", doc.doc);
        assert!(find("clash_gen::map::def::Sited<i64>").is_some());
        assert!(find("no::such::Type").is_none());
    }

    #[test]
    fn rust_links_lose_their_targets() {
        assert_eq!(
            clean("Read with [`by_kind`](crate::by_kind) or [`Text`]; see [the book](https://x)."),
            "Read with `by_kind` or `Text`; see [the book](https://x)."
        );
        assert_eq!(first_sentence("One. Two."), "One.");
        assert_eq!(first_sentence("A 1.5 m wall. More."), "A 1.5 m wall.");
    }
}
