//! Gathers the documentation of the engine's data types for the
//! generated reference (04 §13, decision D-801).
//!
//! What a definition type's fields are is read from the type itself, by
//! `clash_data::shape`; what each means is written in Rust doc comments,
//! which do not reach a running program. This script reads the source
//! of every crate of the workspace and writes, for every struct and enum
//! that is read from JSON, its documentation and that of each field and
//! value under the name JSON gives it, keyed by the type's path as
//! `std::any::type_name` writes it. The reference joins the two.
//!
//! It is a reader of plain Rust items, not of the language: a struct or
//! an enum written by a macro is not found, and its fields are then
//! reported as undocumented by the reference's test.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let crates = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut docs: BTreeMap<String, TypeDoc> = BTreeMap::new();
    let mut crate_dirs: Vec<PathBuf> = fs::read_dir(&crates)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.join("src").is_dir() && path.join("Cargo.toml").is_file())
                .collect()
        })
        .unwrap_or_default();
    crate_dirs.sort();
    for dir in &crate_dirs {
        let Some(name) = dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let src = dir.join("src");
        println!("cargo:rerun-if-changed={}", src.display());
        let crate_name = name.replace('-', "_");
        let mut files = Vec::new();
        collect_files(&src, &mut files);
        files.sort();
        let mut read: Vec<(Vec<String>, String)> = Vec::new();
        for file in files {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let Ok(relative) = file.strip_prefix(&src) else {
                continue;
            };
            read.push((module_path(&crate_name, relative), text));
        }
        // Types whose `Deserialize` is written by hand are read too.
        let mut by_hand = BTreeSet::new();
        for (_, text) in &read {
            for marker in [
                "Deserialize<'de> for ",
                "FromKind<'de> for ",
                "FromSite<'de> for ",
            ] {
                let mut rest = text.as_str();
                while let Some(at) = rest.find(marker) {
                    rest = &rest[at + marker.len()..];
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        by_hand.insert(name);
                    }
                }
            }
        }
        // A type named as the first argument of a macro invoked in a
        // file that reads data: `shapes!(Base, "…", …)`.
        let mut macro_read = BTreeSet::new();
        for (_, text) in &read {
            if !text.contains("Deserialize") {
                continue;
            }
            let mut rest = text.as_str();
            while let Some(at) = rest.find("!(") {
                let after = &rest[at + 2..];
                rest = after;
                let name: String = after
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.chars().next().is_some_and(char::is_uppercase) {
                    macro_read.insert(name);
                }
            }
        }
        let tokenized: Vec<(&Vec<String>, Vec<Token>)> = read
            .iter()
            .map(|(module, text)| (module, tokenize(text)))
            .collect();
        // The fields a macro adds to a struct written inside it, with
        // their documentation, by the macro's name.
        let mut templates: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for (_, tokens) in &tokenized {
            collect_templates(tokens, &mut templates);
        }
        for (module, tokens) in &tokenized {
            let mut parser = Parser {
                tokens,
                at: 0,
                templates: &templates,
            };
            for item in parser.items((*module).clone()) {
                // Every type read from JSON: derived, written by hand, or
                // given its reading by a macro of its crate's own, whose
                // name is then written after the macro's.
                if item.derives_deserialize
                    || by_hand.contains(&item.name)
                    || macro_read.contains(&item.name)
                {
                    let path = format!("{}::{}", item.module.join("::"), item.name);
                    docs.insert(
                        path,
                        TypeDoc {
                            doc: item.doc,
                            items: item.members,
                        },
                    );
                }
            }
        }
    }
    let mut out = String::from(
        "// Written by build.rs from the doc comments of the workspace's sources.\n\
         /// The documentation of every struct and enum read from JSON, by its\n\
         /// path: the type's own and that of each field or value, by the name\n\
         /// JSON gives it.\n\
         pub static TYPE_DOCS: &[TypeDocEntry] = &[\n",
    );
    for (path, doc) in &docs {
        let _ = write!(out, "    ({path:?}, {:?}, &[", doc.doc);
        for (name, text) in &doc.items {
            let _ = write!(out, "({name:?}, {text:?}), ");
        }
        out.push_str("]),\n");
    }
    out.push_str("];\n");
    let target = PathBuf::from(env::var("OUT_DIR").expect("set by cargo")).join("type_docs.rs");
    fs::write(target, out).expect("the generated file is written");
}

/// The documentation of one type.
struct TypeDoc {
    doc: String,
    items: Vec<(String, String)>,
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// The module a file is, from its path below `src`.
fn module_path(crate_name: &str, relative: &Path) -> Vec<String> {
    let mut module = vec![crate_name.to_owned()];
    let parts: Vec<String> = relative
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect();
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        if last {
            let stem = part.trim_end_matches(".rs");
            if !matches!(stem, "lib" | "main" | "mod") {
                module.push(stem.to_owned());
            }
        } else {
            module.push(part.clone());
        }
    }
    module
}

#[derive(Clone, PartialEq, Debug)]
enum Token {
    /// The text of a `///` comment line.
    Doc(String),
    /// The text inside `#[ ... ]`.
    Attr(String),
    Ident(String),
    Open(char),
    Close(char),
    Punct(char),
    Literal,
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            let line: String = chars[start..i].iter().collect();
            if let Some(doc) = line.strip_prefix("///")
                && !doc.starts_with('/')
            {
                tokens.push(Token::Doc(doc.strip_prefix(' ').unwrap_or(doc).to_owned()));
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == '#' && (next == Some('[') || (next == Some('!') && chars.get(i + 2) == Some(&'[')))
        {
            let inner = next == Some('!');
            i += if inner { 2 } else { 1 };
            let start = i + 1;
            let mut depth = 0;
            while i < chars.len() {
                match chars[i] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    '"' => i = skip_string(&chars, i),
                    _ => {}
                }
                i += 1;
            }
            if !inner {
                tokens.push(Token::Attr(
                    chars[start..i.min(chars.len())].iter().collect(),
                ));
            }
            i += 1;
            continue;
        }
        if c == '"' {
            i = skip_string(&chars, i) + 1;
            tokens.push(Token::Literal);
            continue;
        }
        if c == 'r' && (next == Some('"') || next == Some('#')) {
            // A raw string: r"..." or r#"..."#.
            let mut j = i + 1;
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                j += 1;
                loop {
                    if j >= chars.len() {
                        break;
                    }
                    if chars[j] == '"' && (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')) {
                        j += hashes + 1;
                        break;
                    }
                    j += 1;
                }
                i = j;
                tokens.push(Token::Literal);
                continue;
            }
        }
        if c == 'b' && next == Some('"') {
            i = skip_string(&chars, i + 1) + 1;
            tokens.push(Token::Literal);
            continue;
        }
        if c == '\'' {
            // A character literal, or a lifetime.
            if next == Some('\\') {
                let mut j = i + 2;
                while j < chars.len() && chars[j] != '\'' {
                    j += 1;
                }
                i = j + 1;
                tokens.push(Token::Literal);
                continue;
            }
            if chars.get(i + 2) == Some(&'\'') {
                i += 3;
                tokens.push(Token::Literal);
                continue;
            }
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
            continue;
        }
        if c.is_ascii_digit() {
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                if chars[i] == '.' && chars.get(i + 1) == Some(&'.') {
                    break;
                }
                i += 1;
            }
            tokens.push(Token::Literal);
            continue;
        }
        match c {
            '{' | '(' | '[' => tokens.push(Token::Open(c)),
            '}' | ')' | ']' => tokens.push(Token::Close(c)),
            c if c.is_whitespace() => {}
            c => tokens.push(Token::Punct(c)),
        }
        i += 1;
    }
    tokens
}

/// The position of the quote that ends a string starting at `start`.
fn skip_string(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i,
            _ => i += 1,
        }
    }
    i
}

/// A struct or an enum.
struct Item {
    module: Vec<String>,
    name: String,
    doc: String,
    derives_deserialize: bool,
    /// Fields or values: the name JSON gives each, and its documentation.
    members: Vec<(String, String)>,
}

struct Parser<'t> {
    tokens: &'t [Token],
    at: usize,
    templates: &'t BTreeMap<String, Vec<(String, String)>>,
}

/// Finds the macros whose body writes a struct with fields of its own
/// beside those it is given (`pub struct $name { $( ... )* pub field:
/// Type, }`). Those fields, with their documentation, are added to any
/// struct written inside an invocation of the macro.
fn collect_templates(tokens: &[Token], templates: &mut BTreeMap<String, Vec<(String, String)>>) {
    let mut i = 0;
    while i + 3 < tokens.len() {
        if tokens[i] == Token::Ident("macro_rules".to_owned())
            && tokens[i + 1] == Token::Punct('!')
            && let Token::Ident(name) = &tokens[i + 2]
            && tokens[i + 3] == Token::Open('{')
        {
            // The end of the macro's body.
            let mut depth = 0;
            let mut end = i + 3;
            for (at, token) in tokens.iter().enumerate().skip(i + 3) {
                match token {
                    Token::Open(_) => depth += 1,
                    Token::Close(_) => {
                        depth -= 1;
                        if depth == 0 {
                            end = at;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &tokens[i + 4..end];
            for (at, token) in body.iter().enumerate() {
                if *token == Token::Ident("struct".to_owned())
                    && body.get(at + 1) == Some(&Token::Punct('$'))
                    && body.get(at + 3) == Some(&Token::Open('{'))
                {
                    let fields = template_fields(&body[at + 4..]);
                    if !fields.is_empty() {
                        templates.insert(name.clone(), fields);
                    }
                    break;
                }
            }
            i = end;
        }
        i += 1;
    }
}

/// The fields with literal names in the body of a struct a macro writes,
/// up to the brace that closes it.
fn template_fields(tokens: &[Token]) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut docs: Vec<String> = Vec::new();
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Open(_) => depth += 1,
            Token::Close(_) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Token::Doc(line) if depth == 0 => docs.push(line.clone()),
            Token::Ident(word) if depth == 0 && word == "pub" => {}
            Token::Ident(field)
                if depth == 0
                    && tokens.get(i + 1) == Some(&Token::Punct(':'))
                    && i > 0
                    && tokens.get(i - 1) != Some(&Token::Punct('$')) =>
            {
                fields.push((field.clone(), join_doc(&docs)));
                docs.clear();
            }
            Token::Punct(',') if depth == 0 => docs.clear(),
            _ => {}
        }
    }
    fields
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn bump(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.at);
        self.at += 1;
        token
    }

    /// Skips from an opening bracket to the one that closes it.
    fn skip_group(&mut self) {
        let mut depth = 0;
        while let Some(token) = self.bump() {
            match token {
                Token::Open(_) => depth += 1,
                Token::Close(_) => {
                    depth -= 1;
                    if depth <= 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// The items of a module, up to the brace that closes it.
    fn items(&mut self, module: Vec<String>) -> Vec<Item> {
        let mut items = Vec::new();
        let mut docs: Vec<String> = Vec::new();
        let mut attrs: Vec<String> = Vec::new();
        while let Some(token) = self.peek().cloned() {
            match token {
                Token::Doc(line) => {
                    docs.push(line);
                    self.at += 1;
                }
                Token::Attr(attr) => {
                    attrs.push(attr);
                    self.at += 1;
                }
                Token::Close('}') => {
                    self.at += 1;
                    return items;
                }
                Token::Ident(word) if word == "struct" || word == "enum" => {
                    self.at += 1;
                    let Some(Token::Ident(name)) = self.bump().cloned() else {
                        continue;
                    };
                    let item = if word == "struct" {
                        self.struct_body(&module, name, &docs, &attrs)
                    } else {
                        self.enum_body(&module, name, &docs, &attrs)
                    };
                    if let Some(item) = item {
                        items.push(item);
                    }
                    docs.clear();
                    attrs.clear();
                }
                Token::Ident(word) if word == "mod" => {
                    self.at += 1;
                    let Some(Token::Ident(name)) = self.bump().cloned() else {
                        continue;
                    };
                    let test = attrs.iter().any(|attr| attr.contains("cfg(test)"));
                    if self.peek() == Some(&Token::Open('{')) {
                        self.at += 1;
                        let mut inner = module.clone();
                        inner.push(name);
                        let found = self.items(inner);
                        if !test {
                            items.extend(found);
                        }
                    }
                    docs.clear();
                    attrs.clear();
                }
                Token::Punct('!')
                    if self.at > 0
                        && matches!(self.tokens.get(self.at - 1), Some(Token::Ident(_)))
                        && self.tokens.get(self.at + 1) == Some(&Token::Open('{')) =>
                {
                    // A macro invoked with items inside: they are read
                    // as items of this module, and a struct among them
                    // gets the fields the macro adds.
                    let macro_name = match self.tokens.get(self.at - 1) {
                        Some(Token::Ident(name)) => name.clone(),
                        _ => String::new(),
                    };
                    self.at += 2;
                    let mut found = self.items(module.clone());
                    if let Some(extra) = self.templates.get(&macro_name) {
                        for item in &mut found {
                            item.members.extend(extra.iter().cloned());
                        }
                    }
                    items.extend(found);
                    docs.clear();
                    attrs.clear();
                }
                Token::Open('{') => {
                    // The body of a function, an impl, a trait: nothing
                    // in it is a type read from JSON by its path.
                    self.skip_group();
                    docs.clear();
                    attrs.clear();
                }
                Token::Open(_) => {
                    self.skip_group();
                }
                Token::Punct(';') => {
                    self.at += 1;
                    docs.clear();
                    attrs.clear();
                }
                _ => {
                    self.at += 1;
                }
            }
        }
        items
    }

    /// Skips generic parameters and a where clause, up to the body.
    fn skip_to_body(&mut self) -> Option<char> {
        let mut angle = 0;
        while let Some(token) = self.peek().cloned() {
            match token {
                Token::Punct('<') => angle += 1,
                Token::Punct('>') => angle -= 1,
                Token::Open(c) if angle <= 0 && (c == '{' || c == '(') => return Some(c),
                Token::Open(_) => {
                    self.skip_group();
                    continue;
                }
                Token::Punct(';') if angle <= 0 => return Some(';'),
                _ => {}
            }
            self.at += 1;
        }
        None
    }

    fn struct_body(
        &mut self,
        module: &[String],
        name: String,
        docs: &[String],
        attrs: &[String],
    ) -> Option<Item> {
        let body = self.skip_to_body()?;
        let rename_all = rename_all(attrs);
        let mut item = Item {
            module: module.to_vec(),
            name,
            doc: join_doc(docs),
            derives_deserialize: derives_deserialize(attrs),
            members: Vec::new(),
        };
        if body != '{' {
            if body == '(' {
                self.skip_group();
            }
            return Some(item);
        }
        self.at += 1;
        let mut field_docs: Vec<String> = Vec::new();
        let mut field_attrs: Vec<String> = Vec::new();
        loop {
            match self.peek().cloned() {
                None => return Some(item),
                Some(Token::Close('}')) => {
                    self.at += 1;
                    return Some(item);
                }
                Some(Token::Doc(line)) => {
                    field_docs.push(line);
                    self.at += 1;
                }
                Some(Token::Attr(attr)) => {
                    field_attrs.push(attr);
                    self.at += 1;
                }
                Some(Token::Ident(word)) if word == "pub" => {
                    self.at += 1;
                    if self.peek() == Some(&Token::Open('(')) {
                        self.skip_group();
                    }
                }
                Some(Token::Ident(field)) => {
                    self.at += 1;
                    let field = field.trim_start_matches("r#").to_owned();
                    // The type, up to the comma at this depth.
                    let mut angle = 0;
                    while let Some(token) = self.peek().cloned() {
                        match token {
                            Token::Punct('<') => angle += 1,
                            Token::Punct('>') => {
                                // `->` in a function pointer type.
                                if self.at > 0
                                    && self.tokens.get(self.at - 1) != Some(&Token::Punct('-'))
                                {
                                    angle -= 1;
                                }
                            }
                            Token::Punct(',') if angle <= 0 => {
                                self.at += 1;
                                break;
                            }
                            Token::Close('}') => break,
                            Token::Open(_) => {
                                self.skip_group();
                                continue;
                            }
                            _ => {}
                        }
                        self.at += 1;
                    }
                    let skipped = field_attrs.iter().any(|attr| {
                        serde_word(attr, "skip") || serde_word(attr, "skip_deserializing")
                    });
                    if !skipped {
                        let key = renamed(&field_attrs)
                            .unwrap_or_else(|| apply_field_case(&field, rename_all.as_deref()));
                        item.members.push((key, join_doc(&field_docs)));
                    }
                    field_docs.clear();
                    field_attrs.clear();
                }
                Some(_) => {
                    self.at += 1;
                }
            }
        }
    }

    fn enum_body(
        &mut self,
        module: &[String],
        name: String,
        docs: &[String],
        attrs: &[String],
    ) -> Option<Item> {
        let body = self.skip_to_body()?;
        let rename_all = rename_all(attrs);
        let mut item = Item {
            module: module.to_vec(),
            name,
            doc: join_doc(docs),
            derives_deserialize: derives_deserialize(attrs),
            members: Vec::new(),
        };
        if body != '{' {
            return Some(item);
        }
        self.at += 1;
        let mut variant_docs: Vec<String> = Vec::new();
        let mut variant_attrs: Vec<String> = Vec::new();
        loop {
            match self.peek().cloned() {
                None => return Some(item),
                Some(Token::Close('}')) => {
                    self.at += 1;
                    return Some(item);
                }
                Some(Token::Doc(line)) => {
                    variant_docs.push(line);
                    self.at += 1;
                }
                Some(Token::Attr(attr)) => {
                    variant_attrs.push(attr);
                    self.at += 1;
                }
                Some(Token::Ident(variant)) => {
                    self.at += 1;
                    // Its data, a discriminant, up to the comma.
                    while let Some(token) = self.peek().cloned() {
                        match token {
                            Token::Punct(',') => {
                                self.at += 1;
                                break;
                            }
                            Token::Close('}') => break,
                            Token::Open(_) => {
                                self.skip_group();
                                continue;
                            }
                            _ => {}
                        }
                        self.at += 1;
                    }
                    let skipped = variant_attrs.iter().any(|attr| {
                        serde_word(attr, "skip") || serde_word(attr, "skip_deserializing")
                    });
                    if !skipped {
                        let doc = join_doc(&variant_docs);
                        let key = renamed(&variant_attrs)
                            .unwrap_or_else(|| apply_variant_case(&variant, rename_all.as_deref()));
                        for alias in aliases(&variant_attrs) {
                            item.members.push((alias, doc.clone()));
                        }
                        item.members.push((key, doc));
                    }
                    variant_docs.clear();
                    variant_attrs.clear();
                }
                Some(_) => {
                    self.at += 1;
                }
            }
        }
    }
}

fn join_doc(lines: &[String]) -> String {
    let mut out = String::new();
    let mut fence = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if trimmed.is_empty() {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push_str("\n\n");
            }
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        out.push_str(trimmed);
    }
    out.trim().to_owned()
}

fn derives_deserialize(attrs: &[String]) -> bool {
    attrs.iter().any(|attr| {
        attr.trim_start().starts_with("derive")
            && attr
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == "Deserialize")
    })
}

/// Whether a `serde(...)` attribute holds a bare word.
fn serde_word(attr: &str, word: &str) -> bool {
    let Some(inner) = serde_inner(attr) else {
        return false;
    };
    inner.split(',').any(|part| part.trim() == word)
}

fn serde_inner(attr: &str) -> Option<&str> {
    let attr = attr.trim();
    let rest = attr.strip_prefix("serde")?.trim_start();
    let rest = rest.strip_prefix('(')?;
    rest.strip_suffix(')')
}

/// The value of `key = "..."` in a `serde(...)` attribute.
fn serde_value(attr: &str, key: &str) -> Vec<String> {
    let Some(inner) = serde_inner(attr) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut rest = inner;
    while let Some(at) = rest.find(key) {
        let before = rest[..at].chars().last();
        let after = &rest[at + key.len()..];
        rest = after;
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('"') else {
            continue;
        };
        if let Some(end) = after.find('"') {
            found.push(after[..end].to_owned());
        }
    }
    found
}

fn rename_all(attrs: &[String]) -> Option<String> {
    attrs
        .iter()
        .flat_map(|attr| serde_value(attr, "rename_all"))
        .next()
}

fn renamed(attrs: &[String]) -> Option<String> {
    attrs
        .iter()
        .flat_map(|attr| serde_value(attr, "rename"))
        .next()
}

fn aliases(attrs: &[String]) -> Vec<String> {
    attrs
        .iter()
        .flat_map(|attr| serde_value(attr, "alias"))
        .collect()
}

/// A field's name under `rename_all`, as `serde` makes it.
fn apply_field_case(field: &str, rule: Option<&str>) -> String {
    match rule {
        Some("lowercase") => field.to_lowercase(),
        Some("UPPERCASE" | "SCREAMING_SNAKE_CASE") => field.to_uppercase(),
        Some("kebab-case") => field.replace('_', "-"),
        Some("SCREAMING-KEBAB-CASE") => field.to_uppercase().replace('_', "-"),
        Some("camelCase" | "PascalCase") => {
            let mut out = String::new();
            let mut upper = rule == Some("PascalCase");
            for c in field.chars() {
                if c == '_' {
                    upper = true;
                } else if upper {
                    out.extend(c.to_uppercase());
                    upper = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
        _ => field.to_owned(),
    }
}

/// A value's name under `rename_all`, as `serde` makes it.
fn apply_variant_case(variant: &str, rule: Option<&str>) -> String {
    let snake = || {
        let mut out = String::new();
        for (index, c) in variant.chars().enumerate() {
            if c.is_uppercase() && index > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        }
        out
    };
    match rule {
        Some("lowercase") => variant.to_lowercase(),
        Some("UPPERCASE") => variant.to_uppercase(),
        Some("snake_case") => snake(),
        Some("SCREAMING_SNAKE_CASE") => snake().to_uppercase(),
        Some("kebab-case") => snake().replace('_', "-"),
        Some("SCREAMING-KEBAB-CASE") => snake().to_uppercase().replace('_', "-"),
        Some("camelCase") => {
            let mut chars = variant.chars();
            match chars.next() {
                Some(first) => first.to_lowercase().chain(chars).collect(),
                None => String::new(),
            }
        }
        _ => variant.to_owned(),
    }
}
