//! A validator of the part of JSON Schema the schemas use (see the
//! module above), for the tests: it checks a parsed JSON file, with the
//! place of every value, against a schema as the command writes it.

use std::collections::BTreeMap;

use clash_data::{Node, NodeKind, Num, Span};

use super::regex::Regex;
use crate::reference::json::Json;

/// What a value does not satisfy, and where it is.
#[derive(Clone, Debug)]
pub struct Problem {
    /// Where the value was written.
    pub span: Span,
    /// Its JSON path.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

/// Checks values against one schema document.
pub struct Validator<'a> {
    root: &'a Json,
    patterns: std::cell::RefCell<BTreeMap<String, Regex>>,
}

fn get<'j>(schema: &'j Json, key: &str) -> Option<&'j Json> {
    match schema {
        Json::Object(members) => members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value),
        _ => None,
    }
}

fn kind_of(node: &Node) -> &'static str {
    match &node.kind {
        NodeKind::Null => "null",
        NodeKind::Bool(_) => "boolean",
        NodeKind::Number(_) | NodeKind::Quantity { .. } => "number",
        NodeKind::String(_) => "string",
        NodeKind::Array(_) => "array",
        NodeKind::Object(_) => "object",
    }
}

fn is_integer(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Number(Num::Int(_) | Num::UInt(_)) => true,
        NodeKind::Number(Num::Float(value)) => value.fract() == 0.0,
        _ => false,
    }
}

fn number(node: &Node) -> Option<f64> {
    match &node.kind {
        NodeKind::Number(number) => Some(number.as_f64()),
        NodeKind::Quantity { value, .. } => Some(*value),
        _ => None,
    }
}

/// Whether a value equals a constant of a schema.
fn equals(node: &Node, constant: &Json) -> bool {
    match (&node.kind, constant) {
        (NodeKind::Null, Json::Null) => true,
        (NodeKind::Bool(a), Json::Bool(b)) => a == b,
        (NodeKind::String(a), Json::Str(b)) => a == b,
        (NodeKind::Number(_), Json::Int(b)) => number(node) == Some(*b as f64),
        (NodeKind::Number(_), Json::Real(b)) => number(node) == Some(*b),
        _ => false,
    }
}

impl<'a> Validator<'a> {
    /// A validator of the schema `root`.
    pub fn new(root: &'a Json) -> Validator<'a> {
        Validator {
            root,
            patterns: std::cell::RefCell::new(BTreeMap::new()),
        }
    }

    /// What `value` does not satisfy, in the order found.
    pub fn validate(&self, value: &Node) -> Vec<Problem> {
        let mut problems = Vec::new();
        self.check(self.root, value, "", &mut problems);
        problems
    }

    fn resolve(&self, reference: &str) -> Option<&'a Json> {
        let name = reference.strip_prefix("#/$defs/")?;
        get(get(self.root, "$defs")?, name)
    }

    fn pattern_matches(&self, pattern: &str, text: &str) -> bool {
        let mut patterns = self.patterns.borrow_mut();
        if !patterns.contains_key(pattern) {
            match Regex::new(pattern) {
                Ok(regex) => {
                    patterns.insert(pattern.to_owned(), regex);
                }
                Err(_) => return false,
            }
        }
        patterns
            .get(pattern)
            .is_some_and(|regex| regex.is_match(text))
    }

    fn check(&self, schema: &Json, node: &Node, path: &str, problems: &mut Vec<Problem>) {
        let problem = |problems: &mut Vec<Problem>, message: String| {
            problems.push(Problem {
                span: node.span,
                path: path.to_owned(),
                message,
            });
        };
        let schema = match schema {
            Json::Bool(true) => return,
            Json::Bool(false) => {
                problem(problems, "nothing is allowed here".to_owned());
                return;
            }
            other => other,
        };
        if let Some(Json::Str(reference)) = get(schema, "$ref") {
            match self.resolve(reference) {
                Some(target) => self.check(target, node, path, problems),
                None => problem(problems, format!("the schema names no `{reference}`")),
            }
        }
        if let Some(ty) = get(schema, "type") {
            let allowed: Vec<&str> = match ty {
                Json::Str(name) => vec![name.as_str()],
                Json::Array(names) => names
                    .iter()
                    .filter_map(|name| match name {
                        Json::Str(name) => Some(name.as_str()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let kind = kind_of(node);
            let fits = allowed
                .iter()
                .any(|name| *name == kind || (*name == "integer" && is_integer(node)));
            if !fits {
                problem(
                    problems,
                    format!("expected {}, found {kind}", allowed.join(" or ")),
                );
                return;
            }
        }
        if let Some(Json::Array(values)) = get(schema, "enum")
            && !values.iter().any(|value| equals(node, value))
        {
            let shown = match &node.kind {
                NodeKind::String(text) => format!("\"{text}\""),
                _ => node.to_json(),
            };
            problem(
                problems,
                format!("{shown} is not one of the values allowed here"),
            );
        }
        if let Some(constant) = get(schema, "const")
            && !equals(node, constant)
        {
            problem(problems, format!("expected {}", constant.pretty().trim()));
        }
        if let Some(minimum) = get(schema, "minimum")
            && let (Some(value), Json::Int(low)) = (number(node), minimum)
            && value < *low as f64
        {
            problem(problems, format!("{value} is below {low}"));
        }
        if let (Some(Json::Str(pattern)), NodeKind::String(text)) =
            (get(schema, "pattern"), &node.kind)
            && !self.pattern_matches(pattern, text)
        {
            problem(
                problems,
                format!("\"{text}\" is not of the form this field takes"),
            );
        }
        if let NodeKind::Array(items) = &node.kind {
            self.check_array(schema, node, items, path, problems);
        }
        if let NodeKind::Object(members) = &node.kind {
            self.check_object(schema, node, members, path, problems);
        }
        if let Some(Json::Array(branches)) = get(schema, "allOf") {
            for branch in branches {
                self.check(branch, node, path, problems);
            }
        }
        if let Some(Json::Array(branches)) = get(schema, "anyOf") {
            // What is reported is what is wrong by the branch meant for
            // a value of this kind: the one with the fewest problems
            // among those whose type fits.
            let fitting: Vec<&Json> = branches
                .iter()
                .filter(|branch| self.type_fits(branch, node, 0))
                .collect();
            let candidates = if fitting.is_empty() {
                branches.iter().collect()
            } else {
                fitting
            };
            let mut best: Option<Vec<Problem>> = None;
            let mut valid = false;
            for branch in candidates {
                let mut found = Vec::new();
                self.check(branch, node, path, &mut found);
                if found.is_empty() {
                    valid = true;
                    break;
                }
                if best.as_ref().is_none_or(|best| found.len() < best.len()) {
                    best = Some(found);
                }
            }
            if !valid && let Some(found) = best {
                problems.extend(found);
            }
        }
        if let Some(condition) = get(schema, "if") {
            let mut found = Vec::new();
            self.check(condition, node, path, &mut found);
            let branch = if found.is_empty() {
                get(schema, "then")
            } else {
                get(schema, "else")
            };
            if let Some(branch) = branch {
                self.check(branch, node, path, problems);
            }
        }
        if let Some(inner) = get(schema, "not") {
            let mut found = Vec::new();
            self.check(inner, node, path, &mut found);
            if found.is_empty() {
                problem(problems, "a value of a form not allowed here".to_owned());
            }
        }
    }

    /// Whether a schema's own `type`, followed through `$ref` and
    /// `anyOf`, allows the kind of the value.
    fn type_fits(&self, schema: &Json, node: &Node, depth: usize) -> bool {
        if depth > 16 {
            return true;
        }
        if let Some(Json::Str(reference)) = get(schema, "$ref")
            && let Some(target) = self.resolve(reference)
            && !self.type_fits(target, node, depth + 1)
        {
            return false;
        }
        if let Some(Json::Array(branches)) = get(schema, "anyOf")
            && !branches
                .iter()
                .any(|branch| self.type_fits(branch, node, depth + 1))
        {
            return false;
        }
        match get(schema, "type") {
            Some(Json::Str(name)) => {
                let kind = kind_of(node);
                name == kind || (name == "integer" && is_integer(node))
            }
            _ => true,
        }
    }

    fn check_array(
        &self,
        schema: &Json,
        node: &Node,
        items: &[Node],
        path: &str,
        problems: &mut Vec<Problem>,
    ) {
        let count = items.len() as i64;
        if let Some(Json::Int(min)) = get(schema, "minItems")
            && count < *min
        {
            problems.push(Problem {
                span: node.span,
                path: path.to_owned(),
                message: format!("a list of at least {min}, found {count}"),
            });
        }
        if let Some(Json::Int(max)) = get(schema, "maxItems")
            && count > *max
        {
            problems.push(Problem {
                span: node.span,
                path: path.to_owned(),
                message: format!("a list of at most {max}, found {count}"),
            });
        }
        let prefix = match get(schema, "prefixItems") {
            Some(Json::Array(prefix)) => prefix.as_slice(),
            _ => &[],
        };
        for (index, item) in items.iter().enumerate() {
            let at = format!("{path}[{index}]");
            if let Some(schema) = prefix.get(index) {
                self.check(schema, item, &at, problems);
            } else if let Some(schema) = get(schema, "items") {
                self.check(schema, item, &at, problems);
            }
        }
    }

    fn check_object(
        &self,
        schema: &Json,
        node: &Node,
        members: &[clash_data::Member],
        path: &str,
        problems: &mut Vec<Problem>,
    ) {
        let properties = get(schema, "properties");
        if let Some(Json::Array(required)) = get(schema, "required") {
            for key in required {
                if let Json::Str(key) = key
                    && !members.iter().any(|member| member.key == *key)
                {
                    problems.push(Problem {
                        span: node.span,
                        path: path.to_owned(),
                        message: format!("missing field \"{key}\""),
                    });
                }
            }
        }
        for member in members {
            let at = if path.is_empty() {
                member.key.clone()
            } else {
                format!("{path}.{}", member.key)
            };
            if let Some(names) = get(schema, "propertyNames") {
                let key = Node::new(NodeKind::String(member.key.clone()), member.key_span);
                self.check(names, &key, &at, problems);
            }
            match properties.and_then(|properties| get(properties, &member.key)) {
                Some(schema) => self.check(schema, &member.value, &at, problems),
                None => match get(schema, "additionalProperties") {
                    Some(Json::Bool(false)) => problems.push(Problem {
                        span: member.key_span,
                        path: at,
                        message: format!("unknown key \"{}\"", member.key),
                    }),
                    Some(other) => self.check(other, &member.value, &at, problems),
                    None => {}
                },
            }
        }
    }
}
