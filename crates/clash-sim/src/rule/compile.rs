//! Compiling a list of effects into a tree (05 §14).
//!
//! The list arrives as the loader's own JSON tree, with the place of
//! every value. Each effect is recognised by its key; each argument is
//! read as an expression field ([`Expr`](clash_data::Expr),
//! [`Cond`](clash_data::Cond)) and compiled in the scope the effect
//! stands in, or as a place with the expression compiler. Every mistake
//! is a diagnostic with file, line, column, definition and JSON path.

use std::sync::Arc;

use clash_data::diag::{closest, did_you_mean};
use clash_data::expr::codes;
use clash_data::expr::{TypeSpec, Written};
use clash_data::{
    Code, CompileCx, Cond, Diagnostic, Diagnostics, Member, Node, Scope, Site, Span, Text,
    ValidateCx, ValueSource, from_node_at,
};
use clash_expr::{DefRef, EventId, Place, PlaceRoot, Type, Value, is_keyword};

use super::def::{EOC_TYPE, EocDef, REASON_TYPE, RULE_GROUP_TYPE};
use super::effects::Compiled;
use super::tree::{
    Arg, Ask, AskOption, AskOptions, Block, Call, Effect, EffectKind, Emit, Items, Target, Timer,
    TimerKind,
};
use super::value::literal_type;
use super::{EocId, ReasonDefId, RuleGroupId};

/// How deeply lists of effects may nest inside one another. One more is
/// a load error, so that nothing a mod writes can exhaust the compiler
/// or the interpreter.
pub const MAX_BLOCK_DEPTH: usize = 32;

/// One of the effects that belong to no subsystem (05 §10.2 to §10.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EffectInfo {
    /// The key the effect is written with.
    pub name: &'static str,
    /// How it is written.
    pub form: &'static str,
    /// What it does and what its arguments are.
    pub doc: &'static str,
}

/// The effects of the rule system itself, for the generated reference
/// and for suggestions. Everything else an effect list may hold is a
/// verb in the registry or a rule with parameters.
pub const CORE_EFFECTS: &[EffectInfo] = &[
    EffectInfo {
        name: "if",
        form: r#"{ "if": cond, "then": […], "else": […] }"#,
        doc: "Runs `then` if the condition holds and `else` if it does not. Either list may be left out, not both.",
    },
    EffectInfo {
        name: "first",
        form: r#"{ "first": [ { "if": cond, "do": […] }, { "do": […] } ] }"#,
        doc: "Runs the first entry whose condition holds. An entry with no `if` always holds.",
    },
    EffectInfo {
        name: "switch",
        form: r#"{ "switch": expr, "cases": { "a": […], "b": […] }, "default": […] }"#,
        doc: "Runs the case whose key the value reads as: a text as it is, a number as it is written, an id as its qualified id. `default`, which may be left out, runs when no case matches.",
    },
    EffectInfo {
        name: "for_each",
        form: r#"{ "for_each": list, "as": "x", "do": […] }"#,
        doc: "Runs `do` once for each element of a list, bound to the local `$x`. The list is an expression, `{ \"list\": expr }`, or a query (05 §11).",
    },
    EffectInfo {
        name: "repeat",
        form: r#"{ "repeat": n, "do": […] }"#,
        doc: "Runs `do` `n` times, an int; the local `$i` counts from zero.",
    },
    EffectInfo {
        name: "while",
        form: r#"{ "while": cond, "do": […] }"#,
        doc: "Runs `do` for as long as the condition holds, within the step budget.",
    },
    EffectInfo {
        name: "weighted",
        form: r#"{ "weighted": [ { "weight": 3, "do": […] }, … ] }"#,
        doc: "Runs one entry, chosen at random with the chance its `weight`, a number, gives it. Weights of zero or less are never chosen.",
    },
    EffectInfo {
        name: "as",
        form: r#"{ "as": { "self": expr }, "do": […] }"#,
        doc: "Binds subjects by name for the effects in `do`. Each expression is worked out before any name is bound.",
    },
    EffectInfo {
        name: "stop",
        form: r#"{ "stop": {} }"#,
        doc: "Ends this rule, or this list of effects.",
    },
    EffectInfo {
        name: "return",
        form: r#"{ "return": expr }"#,
        doc: "Ends this rule and gives a value of the rule's `returns` to whoever called it. Only in a rule.",
    },
    EffectInfo {
        name: "let",
        form: r#"{ "let": { "x": expr, "y": { "run": … } } }"#,
        doc: "Defines locals, in the order written, for the rest of the list they are in. A value is an expression, a value source, `{ \"run\": … }` for what a rule returns, or `{ \"emit\": … }` for what came of an event.",
    },
    EffectInfo {
        name: "set",
        form: r#"{ "set": { "self.var.kills": expr, "$x": expr } }"#,
        doc: "Assigns to places: locals, variables, settable properties, mutable fields of a hook, and entries of those.",
    },
    EffectInfo {
        name: "add",
        form: r#"{ "add": { "self.var.kills": 1 } }"#,
        doc: "Adds to a number, or appends one element to a list. Something never set counts as nothing: adding a number to it gives that number.",
    },
    EffectInfo {
        name: "unset",
        form: r#"{ "unset": [ "self.var.marked" ] }"#,
        doc: "Removes variables, so that each reads as its default again, and entries of maps and lists. A local is made null.",
    },
    EffectInfo {
        name: "run",
        form: r#"{ "run": { "eoc": id, "with": { … } } }"#,
        doc: "Calls a rule. `eoc` is the rule's id, or `{ \"expr\": … }` for one worked out when the effect runs; `with` gives its parameters by name. `{ \"run\": \"id\" }` calls a rule that needs none.",
    },
    EffectInfo {
        name: "queue",
        form: r#"{ "queue": { "eoc": id, "with": { … } } }"#,
        doc: "Calls a rule once the rule that is running, and whatever called it, has finished. Its arguments and the subjects it is given are worked out now. Queued rules run in the order they were queued.",
    },
    EffectInfo {
        name: "attach_rule",
        form: r#"{ "attach_rule": { "to": subject, "eoc": id } }"#,
        doc: "Attaches an attached rule to a subject. Attaching one that is already there does nothing.",
    },
    EffectInfo {
        name: "detach_rule",
        form: r#"{ "detach_rule": { "to": subject, "eoc": id } }"#,
        doc: "Takes a rule attached at run time off a subject, and forgets when it last ran there.",
    },
    EffectInfo {
        name: "set_rule_group",
        form: r#"{ "set_rule_group": { "group": id, "on": bool } }"#,
        doc: "Switches a rule group on or off, whatever its `enabled` says. `\"on\": null` leaves it to `enabled` again.",
    },
    EffectInfo {
        name: "emit",
        form: r#"{ "emit": { "event": id, … } }"#,
        doc: "Raises an event a mod defined. The other keys are the event's subjects and fields by name; every subject must be given, and a field left out is null.",
    },
    EffectInfo {
        name: "modify",
        form: r#"{ "modify": { "event.energy": expr } }"#,
        doc: "In a rule that handles a hook: changes fields the hook declares mutable.",
    },
    EffectInfo {
        name: "cancel_event",
        form: r#"{ "cancel_event": { "reason": id, "values": { … } } }"#,
        doc: "In a rule that handles a hook: prevents what the hook precedes. `reason` is a `reason` definition and `values` what it carries, by name; both may be left out.",
    },
    EffectInfo {
        name: "consume",
        form: r#"{ "consume": {} }"#,
        doc: "Marks the event being handled as consumed: later handlers marked `skip_if_consumed` are skipped.",
    },
    EffectInfo {
        name: "after",
        form: r#"{ "after": "2 s", "do": […] }"#,
        doc: "Runs effects once, later, on the owner's clock (05 §10.4), with the subjects and locals as they are now. `name` (a text unique to the owner; a timer of that name is replaced), `owner` (a subject with a clock; default the rule's `self`, else the realm of `here`, else the simulation) and `while_unloaded` (`pause`, the default, or `elapse`).",
    },
    EffectInfo {
        name: "every",
        form: r#"{ "every": "1 s", "times": 5, "do": […] }"#,
        doc: "Runs effects after each interval of the owner's clock (05 §10.4), `times` times (an int) or for ever. Takes `name`, `owner` and `while_unloaded` as `after` does, and `all` among the modes: fire once for each interval missed.",
    },
    EffectInfo {
        name: "cancel_timer",
        form: r#"{ "cancel_timer": "name" }"#,
        doc: "Cancels the timer of that name (05 §10.4) of the owner `after` would choose, or of `{ \"cancel_timer\": { \"name\": …, \"owner\": subject } }`. Cancelling none does nothing.",
    },
    EffectInfo {
        name: "message",
        form: r#"{ "message": { "text": template, "to": entity, "style": string } }"#,
        doc: "Puts a line in the message log of whoever controls `to`, written for `to` as its reader (05 §4.6). `to` defaults to `self`; `style` names how the interface shows it. `{ \"message\": \"…\" }` gives the text alone.",
    },
    EffectInfo {
        name: "ask",
        form: r#"{ "ask": { "who": entity, "prompt": template, "options": [ { "label": template, "if": cond, "do": […] } ] } }"#,
        doc: "Offers a choice. A directly controlled entity is asked and time stops until it answers; anyone else takes the first option whose `if` holds. The chosen option's `do` runs with the subjects and locals of where the `ask` is written. `options` may instead be an expression giving a list of maps, each with `label`, `eoc` and `with`. `who` defaults to `self`.",
    },
];

/// What `emit` gives a `let` (05 §8.1): the event's fields by name, as
/// the handlers left them, and these two entries.
pub const EMIT_CANCELLED: &str = "cancelled";
/// The code of the reason a hook was cancelled for, or null.
pub const EMIT_REASON: &str = "reason";

/// Where a list of effects stands.
#[derive(Clone, Debug)]
pub(crate) enum Within {
    /// In a definition that is not a rule: `return` has no meaning.
    List,
    /// In a rule that returns values of this type.
    Rule(Type),
}

/// A list with nothing in it, compiled: it still knows its subjects and
/// how many locals its scope has.
pub(crate) fn nothing(site: &Site, cx: &mut CompileCx<'_>, scope: &Scope) -> Compiled {
    let context = cx.context(scope, site);
    Compiled {
        block: Block {
            effects: Arc::from(Vec::new()),
            path: Arc::from(site.path.as_str()),
        },
        frame: context.locals().len(),
        subjects: context
            .subjects()
            .iter()
            .map(|binding| Arc::from(binding.name.as_str()))
            .collect(),
    }
}

/// Compiles a list of effects written at `node`. `None`, with the
/// reasons reported, if anything in it is wrong.
pub(crate) fn compile_list(
    node: &Node,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
    within: &Within,
) -> Option<Compiled> {
    let context = cx.context(scope, site);
    let subjects: Vec<(Arc<str>, Type)> = context
        .subjects()
        .iter()
        .map(|binding| (Arc::from(binding.name.as_str()), binding.ty.clone()))
        .collect();
    let locals: Vec<(String, Type)> = context
        .locals()
        .iter()
        .map(|binding| (binding.name.clone(), binding.ty.clone()))
        .collect();
    let names: Arc<[Arc<str>]> = subjects.iter().map(|(name, _)| name.clone()).collect();
    let mut compiler = Compiler {
        frame: locals.len(),
        cx,
        scope: scope.clone(),
        subjects,
        locals,
        event: context.event_id(),
        within,
        depth: 0,
        sound: true,
    };
    let block = compiler.block(node, &site.path);
    compiler.sound.then_some(Compiled {
        block,
        frame: compiler.frame,
        subjects: names,
    })
}

struct Compiler<'c, 'a> {
    cx: &'c mut CompileCx<'a>,
    /// What is in scope where the effect being compiled stands.
    scope: Scope,
    /// The subjects of `scope`, by slot.
    subjects: Vec<(Arc<str>, Type)>,
    /// The locals of `scope`, by slot.
    locals: Vec<(String, Type)>,
    /// The most locals in scope at once.
    frame: usize,
    event: Option<EventId>,
    within: &'c Within,
    depth: usize,
    sound: bool,
}

/// The keys of one written effect that the compiler knows how to read.
struct Keys<'n> {
    members: &'n [Member],
}

impl<'n> Keys<'n> {
    fn get(&self, key: &str) -> Option<&'n Member> {
        self.members.iter().find(|member| member.key == key)
    }
}

fn child(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn is_local_name(name: &str) -> bool {
    clash_data::is_id_part(name) && !is_keyword(name)
}

const LOCAL_NAME_RULE: &str = "a name starts with a lowercase letter, holds lowercase letters, digits and `_`, and is not a word of the expression language";

impl Compiler<'_, '_> {
    // ---- reporting ------------------------------------------------------

    fn error(&mut self, code: Code, span: Span, path: &str, message: impl Into<String>) {
        self.error_with(code, span, path, message, None);
    }

    fn error_with(
        &mut self,
        code: Code,
        span: Span,
        path: &str,
        message: impl Into<String>,
        suggestion: Option<String>,
    ) {
        self.sound = false;
        let diagnostic = Diagnostic::new(code, message)
            .at(self.cx.sources(), span)
            .with_path(path)
            .suggest_opt(suggestion);
        self.cx.report(diagnostic);
    }

    fn origin(&mut self, span: Span, path: &str) -> u64 {
        let site = Site {
            span: Some(span),
            path: path.to_owned(),
        };
        self.cx.context(&Scope::new(), &site).origin_id()
    }

    /// The namespaces a bare name written at `span` is looked up in.
    fn namespaces(&mut self, span: Span, path: &str) -> clash_expr::Scope {
        let site = Site {
            span: Some(span),
            path: path.to_owned(),
        };
        self.cx.context(&Scope::new(), &site).scope().clone()
    }

    /// Reports the keys of an effect that it does not take.
    fn only(&mut self, keys: &Keys<'_>, path: &str, what: &str, allowed: &[&str]) {
        for member in keys.members {
            if !allowed.contains(&member.key.as_str()) {
                let suggestion = did_you_mean(closest(&member.key, allowed.iter().copied()));
                self.error_with(
                    codes::BAD_EFFECT,
                    member.key_span,
                    &child(path, &member.key),
                    format!(
                        "`{what}` has no `{}`: it takes {}",
                        member.key,
                        allowed
                            .iter()
                            .map(|key| format!("`{key}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    suggestion,
                );
            }
        }
    }

    // ---- leaves ----------------------------------------------------------

    /// Reads and compiles an argument: an expression field that must
    /// give a value of `expected`.
    fn arg(&mut self, node: &Node, path: &str, expected: &Type) -> Option<Arg> {
        if let Some([only]) = node.as_object()
            && (only.key == "run" || only.key == "emit")
        {
            self.error(
                codes::MISPLACED_EFFECT,
                only.key_span,
                path,
                format!(
                    "what `{0}` gives is taken by a `let`: write `{{ \"let\": {{ \"x\": {{ \"{0}\": … }} }} }}` and use `$x` here",
                    only.key
                ),
            );
            return None;
        }
        let mut read = Diagnostics::new();
        let field: Option<Arg> = from_node_at(node, self.cx.sources(), &mut read, None, path);
        for diagnostic in read.iter() {
            self.sound &= !diagnostic.is_error();
            self.cx.report(diagnostic.clone());
        }
        let mut field = field?;
        let scope = self
            .scope
            .clone()
            .expecting(TypeSpec::Exact(expected.clone()));
        self.cx.compile(&mut field, &scope);
        if field.is_compiled() {
            Some(field)
        } else {
            self.sound = false;
            None
        }
    }

    /// Reads and compiles a condition. A mistake in it is reported and
    /// breaks the definition, whatever is returned here.
    fn cond(&mut self, node: &Node, path: &str) -> Option<Cond> {
        let mut read = Diagnostics::new();
        let cond: Option<Cond> = from_node_at(node, self.cx.sources(), &mut read, None, path);
        for diagnostic in read.iter() {
            self.sound &= !diagnostic.is_error();
            self.cx.report(diagnostic.clone());
        }
        let mut cond = cond?;
        self.cx.compile(&mut cond, &self.scope);
        Some(cond)
    }

    /// Compiles what `set`, `add` and `unset` assign to.
    fn place(&mut self, source: &str, span: Span, path: &str) -> Option<Place> {
        let site = Site {
            span: Some(span),
            path: path.to_owned(),
        };
        let context = self.cx.context(&self.scope, &site);
        let outcome = clash_expr::compile_place(source, self.cx.registry(), &context);
        if outcome.value.is_none() {
            self.sound = false;
        }
        self.cx.report_compiler(outcome.diagnostics, &site);
        outcome.value
    }

    /// The type an argument gives, for a local that takes it.
    fn arg_type(&mut self, arg: &Arg) -> Type {
        if let Some(code) = arg.compiled() {
            return code.ty().clone();
        }
        if let Some(value) = arg.constant() {
            return literal_type(value);
        }
        if let Written::Source(source) = arg.written()
            && let ValueSource::Call { function, .. } = &**source
        {
            let registry = self.cx.registry();
            let context = self.cx.context(&Scope::new(), arg.site());
            if let Some(found) = registry
                .resolve_function(&function.value, context.scope())
                .and_then(|resolved| registry.function(resolved.found))
            {
                return found.spec.returns.clone();
            }
        }
        Type::Any
    }

    // ---- scope -----------------------------------------------------------

    fn bind_local(&mut self, name: &str, ty: Type) -> u16 {
        let slot = u16::try_from(self.locals.len()).unwrap_or(u16::MAX);
        self.scope = self.scope.clone().local(name, TypeSpec::Exact(ty.clone()));
        self.locals.push((name.to_owned(), ty));
        self.frame = self.frame.max(self.locals.len());
        slot
    }

    fn bind_subject(&mut self, name: &str, ty: Type) {
        self.scope = self
            .scope
            .clone()
            .subject(name, TypeSpec::Exact(ty.clone()));
        self.subjects.push((Arc::from(name), ty));
    }

    /// Checks that a name can be written `$name`.
    fn local_name(&mut self, member: &Member, path: &str) -> bool {
        if is_local_name(&member.key) {
            return true;
        }
        self.error(
            codes::BAD_EFFECT,
            member.key_span,
            path,
            format!(
                "\"{0}\" cannot be written `${0}`: {LOCAL_NAME_RULE}",
                member.key
            ),
        );
        false
    }

    // ---- lists -----------------------------------------------------------

    /// Compiles a list of effects. What its effects bind goes out of
    /// scope where the list ends.
    fn block(&mut self, node: &Node, path: &str) -> Block {
        let empty = Block {
            effects: Arc::from(Vec::new()),
            path: Arc::from(path),
        };
        let Some(items) = node.as_array() else {
            self.error(
                codes::BAD_EFFECT,
                node.span,
                path,
                format!("expected a list of effects, found {}", node.kind_name()),
            );
            return empty;
        };
        if self.depth >= MAX_BLOCK_DEPTH {
            self.error(
                codes::BAD_EFFECT,
                node.span,
                path,
                format!(
                    "lists of effects nest at most {MAX_BLOCK_DEPTH} deep; move the inner ones into a rule and `run` it"
                ),
            );
            return empty;
        }
        let saved = (self.scope.clone(), self.locals.len(), self.subjects.len());
        self.depth += 1;
        let mut effects = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            self.effect(item, &format!("{path}[{index}]"), &mut effects);
        }
        self.depth -= 1;
        self.scope = saved.0;
        self.locals.truncate(saved.1);
        self.subjects.truncate(saved.2);
        Block {
            effects: effects.into(),
            path: Arc::from(path),
        }
    }

    /// The list under a key, or an empty one if the key is absent.
    fn block_of(&mut self, keys: &Keys<'_>, key: &str, path: &str) -> Block {
        let path = child(path, key);
        match keys.get(key) {
            Some(member) => self.block(&member.value, &path),
            None => Block {
                effects: Arc::from(Vec::new()),
                path: Arc::from(path),
            },
        }
    }

    /// The list under a key, which must be there.
    fn body(&mut self, keys: &Keys<'_>, span: Span, path: &str, what: &str) -> Block {
        if keys.get("do").is_none() {
            self.error(
                codes::BAD_EFFECT,
                span,
                path,
                format!("`{what}` needs `do`: the effects it runs"),
            );
        }
        self.block_of(keys, "do", path)
    }

    // ---- one effect ------------------------------------------------------

    fn effect(&mut self, node: &Node, path: &str, out: &mut Vec<Effect>) {
        let members = match node.as_object() {
            Some(members) if !members.is_empty() => members,
            _ => {
                self.error(
                    codes::BAD_EFFECT,
                    node.span,
                    path,
                    format!(
                        "an effect is an object with one key, its verb: `{{ \"verb\": {{ … }} }}`; this is {}",
                        node.kind_name()
                    ),
                );
                return;
            }
        };
        let keys = Keys { members };
        // The effects that are written with more than one key.
        const HEADS: &[&str] = &[
            "if", "switch", "for_each", "repeat", "while", "after", "every",
        ];
        let head = HEADS
            .iter()
            .copied()
            .find(|head| keys.get(head).is_some())
            .or_else(|| (keys.get("as").is_some() && keys.get("do").is_some()).then_some("as"));
        let head = match head {
            Some(head) => keys.get(head),
            None if members.len() == 1 => members.first(),
            None => {
                let written: Vec<String> = members
                    .iter()
                    .map(|member| format!("`{}`", member.key))
                    .collect();
                self.error(
                    codes::BAD_EFFECT,
                    node.span,
                    path,
                    format!(
                        "an effect is an object with one key, its verb; this has {}. Write each as an effect of its own",
                        written.join(", ")
                    ),
                );
                return;
            }
        };
        let Some(head) = head else { return };
        let origin = self.origin(node.span, path);
        let mut kinds: Vec<EffectKind> = Vec::new();
        self.kind(head, &keys, node.span, path, &mut kinds);
        for kind in kinds {
            out.push(Effect {
                kind,
                origin,
                span: node.span,
                path: Arc::from(path),
            });
        }
    }

    #[allow(clippy::too_many_lines)]
    fn kind(
        &mut self,
        head: &Member,
        keys: &Keys<'_>,
        span: Span,
        path: &str,
        out: &mut Vec<EffectKind>,
    ) {
        let name = head.key.as_str();
        let at = child(path, name);
        let value = &head.value;
        match name {
            "if" => {
                self.only(keys, path, "if", &["if", "then", "else"]);
                let cond = self.cond(value, &at);
                if keys.get("then").is_none() && keys.get("else").is_none() {
                    self.error(
                        codes::BAD_EFFECT,
                        span,
                        path,
                        "`if` needs `then`, `else` or both: the effects to run",
                    );
                }
                let then = self.block_of(keys, "then", path);
                let otherwise = self.block_of(keys, "else", path);
                if let Some(cond) = cond {
                    out.push(EffectKind::If {
                        cond,
                        then,
                        otherwise,
                    });
                }
            }
            "first" => {
                let Some(items) = value.as_array() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`first` takes a list of entries, each `{ \"if\": cond, \"do\": […] }`",
                    );
                    return;
                };
                let mut entries = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let entry_path = format!("{at}[{index}]");
                    let Some(members) = item.as_object() else {
                        self.error(
                            codes::BAD_EFFECT,
                            item.span,
                            &entry_path,
                            "an entry of `first` is `{ \"if\": cond, \"do\": […] }`",
                        );
                        continue;
                    };
                    let entry = Keys { members };
                    self.only(&entry, &entry_path, "an entry of `first`", &["if", "do"]);
                    let cond = match entry.get("if") {
                        Some(member) => {
                            let Some(cond) = self.cond(&member.value, &child(&entry_path, "if"))
                            else {
                                continue;
                            };
                            Some(cond)
                        }
                        None => None,
                    };
                    let body = self.body(&entry, item.span, &entry_path, "an entry of `first`");
                    entries.push((cond, body));
                }
                out.push(EffectKind::First { entries });
            }
            "switch" => {
                self.only(keys, path, "switch", &["switch", "cases", "default"]);
                let on = self.arg(value, &at, &Type::Any);
                let on_type = on.as_ref().map_or(Type::Any, |on| self.arg_type(on));
                let mut cases = Vec::new();
                match keys.get("cases") {
                    Some(member) => {
                        let cases_path = child(path, "cases");
                        if let Some(members) = member.value.as_object() {
                            for case in members {
                                let case_path = child(&cases_path, &case.key);
                                let key = self.case_key(case, &on_type, &case_path);
                                let block = self.block(&case.value, &case_path);
                                if let Some(key) = key {
                                    cases.push((key, block));
                                }
                            }
                        } else {
                            self.error(
                                codes::BAD_EFFECT,
                                member.value.span,
                                &cases_path,
                                "`cases` is an object: each key a value, each value the effects to run for it",
                            );
                        }
                    }
                    None => self.error(
                        codes::BAD_EFFECT,
                        span,
                        path,
                        "`switch` needs `cases`: an object of the effects to run for each value",
                    ),
                }
                let default = self.block_of(keys, "default", path);
                if let Some(on) = on {
                    out.push(EffectKind::Switch { on, cases, default });
                }
            }
            "for_each" => {
                self.only(keys, path, "for_each", &["for_each", "as", "do"]);
                let (items, element) = self.items(value, &at);
                let saved = (self.scope.clone(), self.locals.len());
                let slot = match keys.get("as") {
                    Some(member) => match member.value.as_str() {
                        Some(local) if is_local_name(local) => {
                            Some(self.bind_local(local, element))
                        }
                        _ => {
                            self.error(
                                codes::BAD_EFFECT,
                                member.value.span,
                                &child(path, "as"),
                                format!(
                                    "`as` names the local each element is bound to: {LOCAL_NAME_RULE}"
                                ),
                            );
                            None
                        }
                    },
                    None => {
                        self.error(
                            codes::BAD_EFFECT,
                            span,
                            path,
                            "`for_each` needs `as`: the name of the local each element is bound to",
                        );
                        None
                    }
                };
                let body = self.body(keys, span, path, "for_each");
                self.scope = saved.0;
                self.locals.truncate(saved.1);
                if let (Some(items), Some(slot)) = (items, slot) {
                    out.push(EffectKind::ForEach { items, slot, body });
                }
            }
            "repeat" => {
                self.only(keys, path, "repeat", &["repeat", "do"]);
                let times = self.arg(value, &at, &Type::Int);
                let saved = (self.scope.clone(), self.locals.len());
                let slot = self.bind_local("i", Type::Int);
                let body = self.body(keys, span, path, "repeat");
                self.scope = saved.0;
                self.locals.truncate(saved.1);
                if let Some(times) = times {
                    out.push(EffectKind::Repeat { times, slot, body });
                }
            }
            "while" => {
                self.only(keys, path, "while", &["while", "do"]);
                let cond = self.cond(value, &at);
                let body = self.body(keys, span, path, "while");
                if let Some(cond) = cond {
                    out.push(EffectKind::While { cond, body });
                }
            }
            "weighted" => {
                let Some(items) = value.as_array() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`weighted` takes a list of entries, each `{ \"weight\": number, \"do\": […] }`",
                    );
                    return;
                };
                let mut entries = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let entry_path = format!("{at}[{index}]");
                    let Some(members) = item.as_object() else {
                        self.error(
                            codes::BAD_EFFECT,
                            item.span,
                            &entry_path,
                            "an entry of `weighted` is `{ \"weight\": number, \"do\": […] }`",
                        );
                        continue;
                    };
                    let entry = Keys { members };
                    self.only(&entry, &entry_path, "an entry of `weighted`", &["weight", "do"]);
                    let weight = match entry.get("weight") {
                        Some(member) => {
                            self.arg(&member.value, &child(&entry_path, "weight"), &Type::Real)
                        }
                        None => {
                            self.error(
                                codes::BAD_EFFECT,
                                item.span,
                                &entry_path,
                                "an entry of `weighted` needs `weight`: a number",
                            );
                            None
                        }
                    };
                    let body = self.body(&entry, item.span, &entry_path, "an entry of `weighted`");
                    if let Some(weight) = weight {
                        entries.push((weight, body));
                    }
                }
                out.push(EffectKind::Weighted { entries });
            }
            "as" => {
                self.only(keys, path, "as", &["as", "do"]);
                let Some(members) = value.as_object() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`as` is an object: each key the name of a subject, each value what it is bound to",
                    );
                    return;
                };
                let mut binds = Vec::with_capacity(members.len());
                let mut types = Vec::with_capacity(members.len());
                for member in members {
                    let bind_path = child(&at, &member.key);
                    if !is_local_name(&member.key) {
                        self.error(
                            codes::BAD_EFFECT,
                            member.key_span,
                            &bind_path,
                            format!("\"{}\" cannot name a subject: {LOCAL_NAME_RULE}", member.key),
                        );
                        continue;
                    }
                    if let Some(arg) = self.arg(&member.value, &bind_path, &Type::Any) {
                        types.push(self.arg_type(&arg));
                        binds.push((Arc::<str>::from(member.key.as_str()), arg));
                    }
                }
                let saved = (self.scope.clone(), self.subjects.len());
                for ((name, _), ty) in binds.iter().zip(types) {
                    self.bind_subject(name, ty);
                }
                let body = self.body(keys, span, path, "as");
                self.scope = saved.0;
                self.subjects.truncate(saved.1);
                out.push(EffectKind::Rebind { binds, body });
            }
            "stop" => out.push(EffectKind::Stop),
            "return" => match self.returns() {
                None => self.error(
                    codes::MISPLACED_EFFECT,
                    head.key_span,
                    &at,
                    "`return` gives a value to whoever called a rule, and this list of effects is not in a rule; `stop` ends it",
                ),
                Some(returns) => {
                    let empty = value.as_object().is_some_and(<[Member]>::is_empty);
                    let value = if empty || matches!(value.kind, clash_data::NodeKind::Null) {
                        None
                    } else {
                        match self.arg(value, &at, &returns) {
                            Some(value) => Some(value),
                            None => return,
                        }
                    };
                    out.push(EffectKind::Return { value });
                }
            },
            "let" => {
                let Some(members) = value.as_object() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`let` is an object: each key the name of a local, each value what it starts as",
                    );
                    return;
                };
                for member in members {
                    let entry_path = child(&at, &member.key);
                    if !self.local_name(member, &entry_path) {
                        continue;
                    }
                    let source = match member.value.as_object() {
                        Some([only]) if only.key == "run" || only.key == "emit" => Some(only),
                        _ => None,
                    };
                    match source {
                        Some(only) if only.key == "run" => {
                            let call = self.run(only, &child(&entry_path, "run"));
                            let slot = self.bind_local(&member.key, Type::Any);
                            if let Some(call) = call {
                                out.push(EffectKind::Run {
                                    call,
                                    into: Some(slot),
                                });
                            }
                        }
                        Some(only) => {
                            let emit = self.emit(only, &child(&entry_path, "emit"));
                            let slot = self.bind_local(&member.key, Type::map(Type::Any));
                            if let Some(emit) = emit {
                                out.push(EffectKind::Emit {
                                    emit,
                                    into: Some(slot),
                                });
                            }
                        }
                        None => {
                            let arg = self.arg(&member.value, &entry_path, &Type::Any);
                            let ty = arg.as_ref().map_or(Type::Any, |arg| self.arg_type(arg));
                            let slot = self.bind_local(&member.key, ty);
                            if let Some(value) = arg {
                                out.push(EffectKind::Let { slot, value });
                            }
                        }
                    }
                }
            }
            "set" | "modify" | "add" => {
                let Some(members) = value.as_object() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        format!(
                            "`{name}` is an object: each key a place such as `self.var.kills` or `$x`, each value an expression"
                        ),
                    );
                    return;
                };
                for member in members {
                    let entry_path = child(&at, &member.key);
                    let Some(place) = self.place(&member.key, member.key_span, &entry_path) else {
                        continue;
                    };
                    if !self.writable(&place, name, member.key_span, &entry_path) {
                        continue;
                    }
                    let wanted = if name == "add" {
                        match place.ty().non_null() {
                            Type::List(element) => (**element).clone(),
                            ty if ty.is_numeric() || *ty == Type::Any => ty.clone(),
                            other => {
                                let found = self.cx.registry().type_name(other);
                                self.error(
                                    codes::BAD_EFFECT,
                                    member.key_span,
                                    &entry_path,
                                    format!(
                                        "`add` adds to a number or appends to a list, and this is {found}; use `set`"
                                    ),
                                );
                                continue;
                            }
                        }
                    } else {
                        // A place may be given null: what reads it is
                        // warned where it reads.
                        place.ty().clone().nullable()
                    };
                    let Some(value) = self.arg(&member.value, &entry_path, &wanted) else {
                        continue;
                    };
                    out.push(if name == "add" {
                        EffectKind::Add { place, value }
                    } else {
                        EffectKind::Set { place, value }
                    });
                }
            }
            "unset" => {
                let Some(items) = value.as_array() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`unset` takes a list of places: `[ \"self.var.marked\" ]`",
                    );
                    return;
                };
                for (index, item) in items.iter().enumerate() {
                    let entry_path = format!("{at}[{index}]");
                    let Some(source) = item.as_str() else {
                        self.error(
                            codes::BAD_EFFECT,
                            item.span,
                            &entry_path,
                            "an entry of `unset` is a place written as a string: `\"self.var.marked\"`",
                        );
                        continue;
                    };
                    let Some(place) = self.place(source, item.span, &entry_path) else {
                        continue;
                    };
                    let removable = match place.root() {
                        PlaceRoot::Variable(_) | PlaceRoot::Local(_) => true,
                        _ => !place.keys().is_empty(),
                    };
                    if !removable {
                        self.error(
                            codes::BAD_EFFECT,
                            item.span,
                            &entry_path,
                            "`unset` removes a variable, a local or an entry; a property or a field of an event is given a value with `set`",
                        );
                        continue;
                    }
                    if self.writable(&place, "unset", item.span, &entry_path) {
                        out.push(EffectKind::Unset { place });
                    }
                }
            }
            "run" => {
                if let Some(call) = self.run(head, &at) {
                    out.push(EffectKind::Run { call, into: None });
                }
            }
            "queue" => {
                if let Some(call) = self.run(head, &at) {
                    out.push(EffectKind::Queue { call });
                }
            }
            "attach_rule" | "detach_rule" => {
                let Some(members) = value.as_object() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        format!("`{name}` takes `{{ \"to\": subject, \"eoc\": id }}`"),
                    );
                    return;
                };
                let inner = Keys { members };
                self.only(&inner, &at, name, &["to", "eoc"]);
                let to = self.required(&inner, "to", value.span, &at, name, &Type::Any);
                let rule_type = self.id_type(EOC_TYPE, head.key_span, &at);
                let rule = rule_type
                    .and_then(|ty| self.required(&inner, "eoc", value.span, &at, name, &ty));
                if let (Some(to), Some(rule)) = (to, rule) {
                    out.push(if name == "attach_rule" {
                        EffectKind::AttachRule { to, rule }
                    } else {
                        EffectKind::DetachRule { to, rule }
                    });
                }
            }
            "set_rule_group" => {
                let Some(members) = value.as_object() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`set_rule_group` takes `{ \"group\": id, \"on\": bool }`",
                    );
                    return;
                };
                let inner = Keys { members };
                self.only(&inner, &at, name, &["group", "on"]);
                let group = match inner.get("group") {
                    Some(member) => self
                        .def_id(RULE_GROUP_TYPE, &member.value, &child(&at, "group"))
                        .and_then(|id| RuleGroupId::from_index(id.index as usize)),
                    None => {
                        self.error(
                            codes::BAD_EFFECT,
                            value.span,
                            &at,
                            "`set_rule_group` needs `group`: the id of a `rule_group`",
                        );
                        None
                    }
                };
                let on = self.required(
                    &inner,
                    "on",
                    value.span,
                    &at,
                    name,
                    &Type::Bool.nullable(),
                );
                if let (Some(group), Some(on)) = (group, on) {
                    out.push(EffectKind::SetRuleGroup { group, on });
                }
            }
            "emit" => {
                if let Some(emit) = self.emit(head, &at) {
                    out.push(EffectKind::Emit { emit, into: None });
                }
            }
            "cancel_event" => {
                let hook = self
                    .event
                    .and_then(|event| self.cx.registry().event(event))
                    .is_some_and(|event| event.spec.hook);
                if !hook {
                    self.error(
                        codes::MISPLACED_EFFECT,
                        head.key_span,
                        &at,
                        "`cancel_event` prevents what a hook precedes, and this is not in a rule that listens for a hook: a notification has already happened",
                    );
                    return;
                }
                let members = value.as_object().unwrap_or_default();
                if value.as_object().is_none() {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`cancel_event` takes `{ \"reason\": id, \"values\": { … } }`, or `{}`",
                    );
                    return;
                }
                let inner = Keys { members };
                self.only(&inner, &at, name, &["reason", "values"]);
                let reason = inner.get("reason").and_then(|member| {
                    self.def_id(REASON_TYPE, &member.value, &child(&at, "reason"))
                        .and_then(|id| ReasonDefId::from_index(id.index as usize))
                });
                let mut values = Vec::new();
                if let Some(member) = inner.get("values") {
                    let values_path = child(&at, "values");
                    match member.value.as_object() {
                        Some(entries) => {
                            for entry in entries {
                                let entry_path = child(&values_path, &entry.key);
                                if let Some(arg) = self.arg(&entry.value, &entry_path, &Type::Any) {
                                    values.push((Arc::<str>::from(entry.key.as_str()), arg));
                                }
                            }
                        }
                        None => self.error(
                            codes::BAD_EFFECT,
                            member.value.span,
                            &values_path,
                            "`values` is an object: what the reason carries, by name",
                        ),
                    }
                }
                out.push(EffectKind::CancelEvent { reason, values });
            }
            "consume" => {
                if self.event.is_none() {
                    self.error(
                        codes::MISPLACED_EFFECT,
                        head.key_span,
                        &at,
                        "`consume` marks the event a rule is handling, and this is not in a rule that listens for one",
                    );
                    return;
                }
                out.push(EffectKind::Consume);
            }
            "after" | "every" => {
                let allowed: &[&str] = if name == "after" {
                    &["after", "do", "name", "owner", "while_unloaded"]
                } else {
                    &["every", "times", "do", "name", "owner", "while_unloaded"]
                };
                self.only(keys, path, name, allowed);
                let span_of = self.arg(value, &at, &Type::Real);
                if let Some(seconds) = span_of
                    .as_ref()
                    .and_then(|arg| arg.constant())
                    .and_then(Value::as_number)
                    && seconds <= clash_core::Real::ZERO
                {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        format!("`{name}` needs a span of time above zero"),
                    );
                }
                let times = keys
                    .get("times")
                    .and_then(|member| self.arg(&member.value, &child(path, "times"), &Type::Int));
                let timer_name = keys
                    .get("name")
                    .and_then(|member| self.arg(&member.value, &child(path, "name"), &Type::Str));
                let owner = keys
                    .get("owner")
                    .and_then(|member| self.arg(&member.value, &child(path, "owner"), &Type::Any));
                let while_unloaded = keys.get("while_unloaded").and_then(|member| {
                    let allowed: &[&str] = if name == "after" {
                        &["pause", "elapse"]
                    } else {
                        &["pause", "elapse", "all"]
                    };
                    match member.value.as_str() {
                        Some(mode) if allowed.contains(&mode) => Some(mode.to_owned()),
                        _ => {
                            self.error(
                                codes::BAD_EFFECT,
                                member.value.span,
                                &child(path, "while_unloaded"),
                                format!(
                                    "`while_unloaded` is one of {}",
                                    allowed
                                        .iter()
                                        .map(|mode| format!("\"{mode}\""))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            );
                            None
                        }
                    }
                });
                let body = self.body(keys, span, path, name);
                let Some(span_of) = span_of else { return };
                let kind = if name == "after" {
                    TimerKind::After { delay: span_of }
                } else {
                    TimerKind::Every {
                        interval: span_of,
                        times,
                    }
                };
                out.push(EffectKind::Timer(Box::new(Timer {
                    kind,
                    name: timer_name,
                    owner,
                    while_unloaded,
                    body,
                })));
            }
            "cancel_timer" => {
                // `"name"`, or `{ "name": "…", "owner": subject }`.
                let named = value
                    .as_object()
                    .filter(|members| members.iter().any(|member| member.key == "name"));
                let (timer_name, owner) = match named {
                    Some(members) => {
                        let inner = Keys { members };
                        self.only(&inner, &at, name, &["name", "owner"]);
                        let timer_name =
                            self.required(&inner, "name", value.span, &at, name, &Type::Str);
                        let owner = inner.get("owner").and_then(|member| {
                            self.arg(&member.value, &child(&at, "owner"), &Type::Any)
                        });
                        (timer_name, owner)
                    }
                    None => (self.arg(value, &at, &Type::Str), None),
                };
                if let Some(timer_name) = timer_name {
                    out.push(EffectKind::Timer(Box::new(Timer {
                        kind: TimerKind::Cancel { name: timer_name },
                        name: None,
                        owner,
                        while_unloaded: None,
                        body: Block::empty(),
                    })));
                }
            }
            "message" => {
                let (text_node, text_path, inner) = match value.as_object() {
                    Some(members) => {
                        let inner = Keys { members };
                        self.only(&inner, &at, "message", &["text", "to", "style"]);
                        let Some(text) = inner.get("text") else {
                            self.error(
                                codes::BAD_EFFECT,
                                value.span,
                                &at,
                                "`message` needs `text`: the line, as a text template",
                            );
                            return;
                        };
                        (&text.value, child(&at, "text"), Some(inner))
                    }
                    None => (value, at.clone(), None),
                };
                let text = self.text(text_node, &text_path);
                let to = self.reader(inner.as_ref(), "to", head, &at, "message");
                let style = inner
                    .as_ref()
                    .and_then(|inner| inner.get("style"))
                    .and_then(|member| self.arg(&member.value, &child(&at, "style"), &Type::Str));
                if let (Some(text), Ok(to)) = (text, to) {
                    out.push(EffectKind::Message { text, to, style });
                }
            }
            "ask" => {
                let Some(members) = value.as_object() else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        &at,
                        "`ask` takes `{ \"who\": entity, \"prompt\": text, \"options\": [ … ] }`",
                    );
                    return;
                };
                let inner = Keys { members };
                self.only(&inner, &at, "ask", &["who", "prompt", "options"]);
                let who = self.reader(Some(&inner), "who", head, &at, "ask");
                let prompt = match inner.get("prompt") {
                    Some(member) => self.text(&member.value, &child(&at, "prompt")),
                    None => {
                        self.error(
                            codes::BAD_EFFECT,
                            value.span,
                            &at,
                            "`ask` needs `prompt`: the question, as a text template",
                        );
                        None
                    }
                };
                let options = match inner.get("options") {
                    Some(member) => self.ask_options(member, &child(&at, "options")),
                    None => {
                        self.error(
                            codes::BAD_EFFECT,
                            value.span,
                            &at,
                            "`ask` needs `options`: what may be chosen",
                        );
                        None
                    }
                };
                if let (Ok(who), Some(prompt), Some(options)) = (who, prompt, options) {
                    out.push(EffectKind::Ask(Box::new(Ask {
                        who,
                        prompt,
                        options,
                    })));
                }
            }
            _ => self.verb(head, path, out),
        }
    }

    /// Reads and compiles a text template in the scope the effect
    /// stands in. A mistake in it is reported and breaks the definition.
    fn text(&mut self, node: &Node, path: &str) -> Option<Text> {
        let mut read = Diagnostics::new();
        let text: Option<Text> = from_node_at(node, self.cx.sources(), &mut read, None, path);
        for diagnostic in read.iter() {
            self.sound &= !diagnostic.is_error();
            self.cx.report(diagnostic.clone());
        }
        let mut text = text?;
        self.cx.compile(&mut text, &self.scope);
        Some(text)
    }

    /// Who a `message` or an `ask` is for: the argument under `key`,
    /// or, if it is left out, `self`, which must then be bound where
    /// the effect is written (05 §10.6). `Err` if neither is there.
    fn reader(
        &mut self,
        keys: Option<&Keys<'_>>,
        key: &str,
        head: &Member,
        path: &str,
        what: &str,
    ) -> Result<Option<Arg>, ()> {
        if let Some(member) = keys.and_then(|keys| keys.get(key)) {
            let wanted = self
                .cx
                .registry()
                .kind_id("entity")
                .map_or(Type::Any, Type::Subject);
            return self
                .arg(&member.value, &child(path, key), &wanted)
                .map(Some)
                .ok_or(());
        }
        if self
            .subjects
            .iter()
            .any(|(name, _)| &**name == super::OWNER_SUBJECT)
        {
            return Ok(None);
        }
        self.error(
            codes::MISPLACED_EFFECT,
            head.key_span,
            path,
            format!(
                "`{what}` is for `self` unless `{key}` says who, and no `self` is bound here: add `\"{key}\"`"
            ),
        );
        Err(())
    }

    /// The options of an `ask`: a list written in place, or an
    /// expression that gives one when the effect runs.
    fn ask_options(&mut self, member: &Member, path: &str) -> Option<AskOptions> {
        let Some(items) = member.value.as_array() else {
            let wanted = Type::list(Type::map(Type::Any));
            return self
                .arg(&member.value, path, &wanted)
                .map(AskOptions::Computed);
        };
        if items.is_empty() {
            self.error(
                codes::BAD_EFFECT,
                member.value.span,
                path,
                "`ask` offers at least one option",
            );
            return None;
        }
        let mut options = Vec::with_capacity(items.len());
        let mut sound = true;
        for (index, item) in items.iter().enumerate() {
            let entry_path = format!("{path}[{index}]");
            let Some(members) = item.as_object() else {
                self.error(
                    codes::BAD_EFFECT,
                    item.span,
                    &entry_path,
                    "an option of `ask` is `{ \"label\": text, \"if\": cond, \"do\": […] }`",
                );
                sound = false;
                continue;
            };
            let entry = Keys { members };
            self.only(
                &entry,
                &entry_path,
                "an option of `ask`",
                &["label", "if", "do"],
            );
            let label = match entry.get("label") {
                Some(label) => self.text(&label.value, &child(&entry_path, "label")),
                None => {
                    self.error(
                        codes::BAD_EFFECT,
                        item.span,
                        &entry_path,
                        "an option of `ask` needs `label`: what it says",
                    );
                    None
                }
            };
            let cond = match entry.get("if") {
                Some(cond) => match self.cond(&cond.value, &child(&entry_path, "if")) {
                    Some(cond) => Some(cond),
                    None => {
                        sound = false;
                        None
                    }
                },
                None => None,
            };
            let body = self.body(&entry, item.span, &entry_path, "an option of `ask`");
            match label {
                Some(label) => options.push(AskOption { label, cond, body }),
                None => sound = false,
            }
        }
        sound.then_some(AskOptions::Listed(options))
    }

    /// The type a `return` here must give, if this is in a rule.
    fn returns(&self) -> Option<Type> {
        match self.within {
            Within::List => None,
            Within::Rule(returns) => Some(returns.clone()),
        }
    }

    /// Whether an effect may write to a place: a field of an event only
    /// if the event is a hook that declares it mutable.
    fn writable(&mut self, place: &Place, effect: &str, span: Span, path: &str) -> bool {
        let slot = match place.root() {
            PlaceRoot::EventField(slot) => *slot,
            _ if effect == "modify" => {
                self.error(
                    codes::MISPLACED_EFFECT,
                    span,
                    path,
                    "`modify` changes a field of the hook being handled, written `event.field`; use `set` for anything else",
                );
                return false;
            }
            _ => return true,
        };
        let registry = self.cx.registry();
        let spec = self
            .event
            .and_then(|event| registry.event(event))
            .map(|event| &event.spec);
        let field = spec.and_then(|spec| spec.fields.get(usize::from(slot)));
        let (Some(spec), Some(field)) = (spec, field) else {
            return true;
        };
        if !spec.hook {
            self.error(
                codes::MISPLACED_EFFECT,
                span,
                path,
                format!(
                    "`{}` is a notification: it has happened, and its fields cannot be changed",
                    spec.name
                ),
            );
            return false;
        }
        if !spec.mutable.contains(&field.name) {
            let allowed = if spec.mutable.is_empty() {
                "none".to_owned()
            } else {
                spec.mutable
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            self.error(
                codes::MISPLACED_EFFECT,
                span,
                path,
                format!(
                    "handlers of `{}` may not change `{}`; the fields it declares mutable: {allowed}",
                    spec.name, field.name
                ),
            );
            return false;
        }
        true
    }

    /// An argument under a key, which must be there.
    fn required(
        &mut self,
        keys: &Keys<'_>,
        key: &str,
        span: Span,
        path: &str,
        what: &str,
        expected: &Type,
    ) -> Option<Arg> {
        match keys.get(key) {
            Some(member) => self.arg(&member.value, &child(path, key), expected),
            None => {
                self.error(
                    codes::BAD_EFFECT,
                    span,
                    path,
                    format!("`{what}` needs `{key}`"),
                );
                None
            }
        }
    }

    /// The type `id<name>`, if the loader knows the definition type.
    fn id_type(&mut self, type_name: &str, span: Span, path: &str) -> Option<Type> {
        match self.cx.registry().def_type_id(type_name) {
            Some(ty) => Some(Type::Id(ty)),
            None => {
                self.error(
                    codes::BAD_SCOPE,
                    span,
                    path,
                    format!(
                        "the definition type `{type_name}` is not registered with the loader, so nothing can name one here"
                    ),
                );
                None
            }
        }
    }

    /// The definition a string names, of a type by name.
    fn def_id(&mut self, type_name: &str, node: &Node, path: &str) -> Option<DefRef> {
        let ty = self.id_type(type_name, node.span, path)?;
        let arg = self.arg(node, path, &ty)?;
        match arg.constant() {
            Some(Value::Id(id)) => Some(*id),
            _ => {
                self.error(
                    codes::BAD_EFFECT,
                    node.span,
                    path,
                    format!("this names a `{type_name}` by its id, written as a string"),
                );
                None
            }
        }
    }

    /// The key of a case of `switch`, as the value will read.
    fn case_key(&mut self, case: &Member, on: &Type, path: &str) -> Option<Arc<str>> {
        let Type::Id(ty) = on.non_null() else {
            return Some(Arc::from(case.key.as_str()));
        };
        let registry = self.cx.registry();
        let namespaces = self.namespaces(case.key_span, path);
        match registry.resolve_def(*ty, &case.key, &namespaces) {
            Some(resolved) => Some(Arc::from(resolved.name)),
            None => {
                let known = registry
                    .def_type(*ty)
                    .map(|def_type| def_type.ids().map(|(name, _)| name).collect::<Vec<_>>())
                    .unwrap_or_default();
                let bare = known
                    .iter()
                    .filter_map(|name| name.split_once(':').map(|(_, bare)| bare));
                let suggestion =
                    did_you_mean(closest(&case.key, known.iter().copied().chain(bare)));
                self.error_with(
                    codes::UNKNOWN_ID,
                    case.key_span,
                    path,
                    format!(
                        "no {} \"{}\": the value switched on is an id, and each case names one",
                        registry.type_name(on.non_null()),
                        case.key
                    ),
                    suggestion,
                );
                None
            }
        }
    }

    /// What `for_each` iterates, and the type of each element.
    fn items(&mut self, node: &Node, path: &str) -> (Option<Items>, Type) {
        let list = Type::list(Type::Any);
        let long_list = |value: &Node| {
            value
                .as_object()
                .is_some_and(|members| members.iter().any(|member| member.key == "items"))
        };
        let arg = match node.as_object() {
            Some([only]) if only.key == "list" && !long_list(&only.value) => {
                self.arg(&only.value, &child(path, "list"), &list)
            }
            Some([only]) if only.key == "expr" => self.arg(node, path, &list),
            // Any other object is a query (05 §11).
            Some(_) => {
                let site = Site {
                    span: Some(node.span),
                    path: path.to_owned(),
                };
                let errors = self.cx.error_count();
                let query = clash_data::compile_query(node, &site, self.cx, &self.scope.clone());
                if self.cx.error_count() > errors {
                    self.sound = false;
                }
                return match query {
                    Some(query) => {
                        let element = query.element().clone();
                        (Some(Items::Compiled(Arc::new(query))), element)
                    }
                    None => {
                        self.sound = false;
                        (None, Type::Any)
                    }
                };
            }
            None => self.arg(node, path, &list),
        };
        let element = arg
            .as_ref()
            .map(|arg| self.arg_type(arg))
            .and_then(|ty| ty.element().cloned())
            .unwrap_or(Type::Any);
        (arg.map(Items::List), element)
    }

    // ---- rules -----------------------------------------------------------

    /// `run` and `queue`: `"rule_id"`, or `{ "eoc": id, "with": { … } }`.
    fn run(&mut self, head: &Member, path: &str) -> Option<Call> {
        let value = &head.value;
        let ty = self.id_type(EOC_TYPE, head.key_span, path)?;
        let (rule_node, rule_path, with) = match value.as_object() {
            Some(members) if !members.iter().any(|member| member.key == "expr") => {
                let inner = Keys { members };
                self.only(&inner, path, &head.key, &["eoc", "with"]);
                let Some(rule) = inner.get("eoc") else {
                    self.error(
                        codes::BAD_EFFECT,
                        value.span,
                        path,
                        format!("`{}` needs `eoc`: the id of the rule", head.key),
                    );
                    return None;
                };
                (&rule.value, child(path, "eoc"), inner.get("with"))
            }
            _ => (value, path.to_owned(), None),
        };
        let rule = self.arg(rule_node, &rule_path, &ty)?;
        let target = match rule.constant() {
            Some(Value::Id(id)) => Some(*id),
            _ => None,
        };
        let with_path = child(path, "with");
        let entries: &[Member] = match with {
            Some(member) => match member.value.as_object() {
                Some(entries) => entries,
                None => {
                    self.error(
                        codes::BAD_EFFECT,
                        member.value.span,
                        &with_path,
                        "`with` is an object: the rule's parameters by name",
                    );
                    return None;
                }
            },
            None => &[],
        };
        match target {
            Some(id) => {
                let with = self.call_args(id, entries, rule_node.span, &with_path)?;
                Some(Call {
                    rule: Target::Rule(EocId::from_index(id.index as usize)?),
                    with,
                })
            }
            None => {
                // The rule is not known until the effect runs: its
                // arguments are checked then.
                let mut with = Vec::with_capacity(entries.len());
                for entry in entries {
                    let arg = self.arg(&entry.value, &child(&with_path, &entry.key), &Type::Any)?;
                    with.push((Arc::<str>::from(entry.key.as_str()), arg));
                }
                Some(Call {
                    rule: Target::Computed(rule),
                    with,
                })
            }
        }
    }

    /// The arguments of a call of a rule that is known at load, checked
    /// against the rule's parameters. A rule with parameters is in the
    /// registry as a verb under its qualified id.
    fn call_args(
        &mut self,
        rule: DefRef,
        entries: &[Member],
        span: Span,
        path: &str,
    ) -> Option<Vec<(Arc<str>, Arg)>> {
        let registry = self.cx.registry();
        let name = registry.def_name(rule).unwrap_or("?").to_owned();
        let spec = registry
            .verb_id(&name)
            .and_then(|verb| registry.verb(verb))
            .map(|verb| &verb.spec);
        let params: &[clash_expr::ParamSpec] = spec.map_or(&[], |spec| &spec.args);
        let signature = || {
            if params.is_empty() {
                format!("`{name}` takes no parameters")
            } else {
                let listed: Vec<String> = params
                    .iter()
                    .map(|param| {
                        format!(
                            "`{}{}`: {}",
                            param.name,
                            if param.optional { "?" } else { "" },
                            registry.type_name(&param.ty)
                        )
                    })
                    .collect();
                format!("`{name}` takes {}", listed.join(", "))
            }
        };
        let mut with = Vec::with_capacity(entries.len());
        let mut sound = true;
        for entry in entries {
            let entry_path = child(path, &entry.key);
            let Some(param) = params.iter().find(|param| param.name == entry.key) else {
                let suggestion = did_you_mean(closest(
                    &entry.key,
                    params.iter().map(|param| param.name.as_str()),
                ));
                self.error_with(
                    codes::BAD_EFFECT,
                    entry.key_span,
                    &entry_path,
                    format!("unknown parameter `{}`: {}", entry.key, signature()),
                    suggestion,
                );
                sound = false;
                continue;
            };
            match self.arg(&entry.value, &entry_path, &param.ty) {
                Some(arg) => with.push((Arc::<str>::from(entry.key.as_str()), arg)),
                None => sound = false,
            }
        }
        for param in params {
            if !param.optional && !entries.iter().any(|entry| entry.key == param.name) {
                self.error(
                    codes::BAD_EFFECT,
                    span,
                    path,
                    format!("missing parameter `{}`: {}", param.name, signature()),
                );
                sound = false;
            }
        }
        sound.then_some(with)
    }

    /// `emit`: `{ "event": id, … }` with the event's subjects and fields
    /// by name.
    fn emit(&mut self, head: &Member, path: &str) -> Option<Emit> {
        let value = &head.value;
        let Some(members) = value.as_object() else {
            self.error(
                codes::BAD_EFFECT,
                value.span,
                path,
                "`emit` takes `{ \"event\": id, … }` with the event's subjects and fields by name",
            );
            return None;
        };
        let inner = Keys { members };
        let Some(written) = inner.get("event") else {
            self.error(
                codes::BAD_EFFECT,
                value.span,
                path,
                "`emit` needs `event`: the id of the event to raise",
            );
            return None;
        };
        let event_path = child(path, "event");
        let Some(event_name) = written.value.as_str() else {
            self.error(
                codes::BAD_EFFECT,
                written.value.span,
                &event_path,
                "`event` is the id of an event, written as a string",
            );
            return None;
        };
        let registry = self.cx.registry();
        let namespaces = self.namespaces(written.value.span, &event_path);
        let Some(event) = resolve_event(registry, event_name, &namespaces) else {
            let known: Vec<&str> = registry
                .events()
                .map(|event| event.spec.name.as_str())
                .collect();
            let bare = known
                .iter()
                .filter_map(|name| name.split_once(':').map(|(_, bare)| bare));
            let suggestion = did_you_mean(closest(event_name, known.iter().copied().chain(bare)));
            self.error_with(
                codes::BAD_EFFECT,
                written.value.span,
                &event_path,
                format!("there is no event `{event_name}`"),
                suggestion,
            );
            return None;
        };
        let spec = &registry.event(event)?.spec;
        let mut subjects: Vec<Option<Arg>> = vec![None; spec.subjects.len()];
        let mut fields: Vec<Option<Arg>> = vec![None; spec.fields.len()];
        let mut sound = true;
        for member in members {
            if member.key == "event" {
                continue;
            }
            let entry_path = child(path, &member.key);
            if let Some(at) = spec
                .subjects
                .iter()
                .position(|(name, _)| *name == member.key)
            {
                let wanted = Type::Subject(spec.subjects[at].1);
                subjects[at] = self.arg(&member.value, &entry_path, &wanted);
                sound &= subjects[at].is_some();
            } else if let Some(at) = spec
                .fields
                .iter()
                .position(|field| field.name == member.key)
            {
                fields[at] = self.arg(&member.value, &entry_path, &spec.fields[at].ty);
                sound &= fields[at].is_some();
            } else {
                let known = spec
                    .subjects
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .chain(spec.fields.iter().map(|field| field.name.as_str()));
                let suggestion = did_you_mean(closest(&member.key, known));
                self.error_with(
                    codes::BAD_EFFECT,
                    member.key_span,
                    &entry_path,
                    format!(
                        "the event `{}` has no subject or field `{}`",
                        spec.name, member.key
                    ),
                    suggestion,
                );
                sound = false;
            }
        }
        for (at, (name, _)) in spec.subjects.iter().enumerate() {
            // Who acted and who is behind it come with every event of
            // the simulation, from the cause it is raised with: a rule
            // need not name them.
            let from_cause = crate::event::CAUSE_SUBJECTS
                .iter()
                .any(|(cause, _)| cause.trim_end_matches('?') == name.as_str());
            if from_cause {
                continue;
            }
            if subjects[at].is_none() && !members.iter().any(|member| member.key == *name) {
                self.error(
                    codes::BAD_EFFECT,
                    value.span,
                    path,
                    format!(
                        "the event `{}` needs its subject `{name}`: every subject an event declares is given",
                        spec.name
                    ),
                );
                sound = false;
            }
        }
        sound.then_some(Emit {
            event,
            subjects,
            fields,
        })
    }

    /// An effect whose key is not one of the rule system's own: a verb
    /// the simulation registered, or a rule with parameters.
    fn verb(&mut self, head: &Member, path: &str, out: &mut Vec<EffectKind>) {
        let name = head.key.as_str();
        let at = child(path, name);
        let registry = self.cx.registry();
        let namespaces = self.namespaces(head.key_span, &at);
        let eoc_type = registry.def_type_id(EOC_TYPE);
        // The engine's verbs come first; then a rule, looked up like any
        // reference: in the mod's own namespace, then in its imports.
        let exact = registry.verb_id(name);
        let rule = eoc_type.and_then(|ty| registry.resolve_def(ty, name, &namespaces));
        if let Some(verb) = exact
            && !rule.as_ref().is_some_and(|rule| rule.name == name)
        {
            self.verb_call(verb, head, &at, out);
            return;
        }
        if let Some(rule) = rule {
            if registry.verb_id(&rule.name).is_none() {
                self.error(
                    codes::UNKNOWN_EFFECT,
                    head.key_span,
                    &at,
                    format!(
                        "`{}` is a rule with no `params`, so it cannot be used as a verb: run it with `{{ \"run\": \"{name}\" }}`",
                        rule.name
                    ),
                );
                return;
            }
            let Some(entries) = head.value.as_object() else {
                self.error(
                    codes::BAD_EFFECT,
                    head.value.span,
                    &at,
                    format!(
                        "the rule `{}` takes its parameters by name, in an object",
                        rule.name
                    ),
                );
                return;
            };
            let Some(with) = self.call_args(rule.found, entries, head.key_span, &at) else {
                return;
            };
            if let Some(id) = EocId::from_index(rule.found.index as usize) {
                out.push(EffectKind::Run {
                    call: Call {
                        rule: Target::Rule(id),
                        with,
                    },
                    into: None,
                });
            }
            return;
        }
        let verbs: Vec<&str> = registry
            .verbs()
            .map(|verb| verb.spec.name.as_str())
            .collect();
        let visible = verbs.iter().filter_map(|verb| {
            let (namespace, bare) = verb.split_once(':')?;
            namespaces
                .namespaces()
                .any(|have| have == namespace)
                .then_some(bare)
        });
        let core = CORE_EFFECTS.iter().map(|effect| effect.name);
        let suggestion = did_you_mean(closest(
            name,
            core.chain(verbs.iter().copied()).chain(visible),
        ));
        self.error_with(
            codes::UNKNOWN_EFFECT,
            head.key_span,
            &at,
            format!("unknown effect `{name}`"),
            suggestion,
        );
    }

    /// A call of a registered verb: its arguments by name, or its one
    /// obvious argument directly (05 §10.1).
    fn verb_call(
        &mut self,
        verb: clash_expr::VerbId,
        head: &Member,
        path: &str,
        out: &mut Vec<EffectKind>,
    ) {
        let registry = self.cx.registry();
        let Some(spec) = registry.verb(verb).map(|verb| &verb.spec) else {
            return;
        };
        let signature = || {
            let listed: Vec<String> = spec
                .args
                .iter()
                .map(|arg| {
                    format!(
                        "`{}{}`: {}",
                        arg.name,
                        if arg.optional { "?" } else { "" },
                        registry.type_name(&arg.ty)
                    )
                })
                .collect();
            if listed.is_empty() {
                format!("`{}` takes no arguments", spec.name)
            } else {
                format!("`{}` takes {}", spec.name, listed.join(", "))
            }
        };
        let value = &head.value;
        let named = match value.as_object() {
            // An object is the arguments by name, unless the verb has a
            // direct argument and no key of the object is an argument:
            // then it is that argument, written as a value source.
            Some(members) => {
                members.is_empty()
                    || spec.direct.is_none()
                    || members
                        .iter()
                        .any(|member| spec.args.iter().any(|arg| arg.name == member.key))
            }
            None => false,
        };
        let mut args: Vec<Option<Arg>> = vec![None; spec.args.len()];
        let mut sound = true;
        if named {
            let members = value.as_object().unwrap_or_default();
            for member in members {
                let entry_path = child(path, &member.key);
                let Some(at) = spec.args.iter().position(|arg| arg.name == member.key) else {
                    let suggestion = did_you_mean(closest(
                        &member.key,
                        spec.args.iter().map(|arg| arg.name.as_str()),
                    ));
                    self.error_with(
                        codes::BAD_EFFECT,
                        member.key_span,
                        &entry_path,
                        format!("unknown argument `{}`: {}", member.key, signature()),
                        suggestion,
                    );
                    sound = false;
                    continue;
                };
                args[at] = self.arg(&member.value, &entry_path, &spec.args[at].ty);
                sound &= args[at].is_some();
            }
        } else {
            let Some(direct) = spec
                .direct
                .as_ref()
                .and_then(|direct| spec.args.iter().position(|arg| arg.name == *direct))
            else {
                self.error(
                    codes::BAD_EFFECT,
                    value.span,
                    path,
                    format!(
                        "`{}` takes its arguments by name, in an object: {}",
                        spec.name,
                        signature()
                    ),
                );
                return;
            };
            args[direct] = self.arg(value, path, &spec.args[direct].ty);
            sound &= args[direct].is_some();
        }
        for (at, arg) in spec.args.iter().enumerate() {
            let given = args[at].is_some()
                || (named
                    && value.as_object().is_some_and(|members| {
                        members.iter().any(|member| member.key == arg.name)
                    }));
            if !arg.optional && !given && (named || spec.direct.as_deref() != Some(&arg.name)) {
                self.error(
                    codes::BAD_EFFECT,
                    head.key_span,
                    path,
                    format!("missing argument `{}`: {}", arg.name, signature()),
                );
                sound = false;
            }
        }
        if sound {
            out.push(EffectKind::Verb { verb, args });
        }
    }
}

/// The event a name written in a mod refers to: the engine's of that
/// name, or else a mod's, looked up like any reference.
pub(crate) fn resolve_event(
    registry: &clash_data::expr::ExprRegistry,
    name: &str,
    namespaces: &clash_expr::Scope,
) -> Option<EventId> {
    if let Some(event) = registry.event_id(name) {
        return Some(event);
    }
    if name.contains(':') {
        return None;
    }
    namespaces
        .namespaces()
        .find_map(|namespace| registry.event_id(&format!("{namespace}:{name}")))
}

/// Checks the calls in a compiled list against the rules they name: a
/// rule that says it needs a subject from its caller must be called
/// where a subject of that name is bound.
pub(crate) fn check_calls(block: &Block, subjects: &[Arc<str>], cx: &mut ValidateCx<'_>) {
    let mut names: Vec<Arc<str>> = subjects.to_vec();
    check_block(block, &mut names, cx);
}

fn check_block(block: &Block, names: &mut Vec<Arc<str>>, cx: &mut ValidateCx<'_>) {
    for effect in block.effects() {
        match &effect.kind {
            EffectKind::If {
                then, otherwise, ..
            } => {
                check_block(then, names, cx);
                check_block(otherwise, names, cx);
            }
            EffectKind::First { entries } => {
                for (_, body) in entries {
                    check_block(body, names, cx);
                }
            }
            EffectKind::Switch { cases, default, .. } => {
                for (_, body) in cases {
                    check_block(body, names, cx);
                }
                check_block(default, names, cx);
            }
            EffectKind::ForEach { body, .. }
            | EffectKind::Repeat { body, .. }
            | EffectKind::While { body, .. } => check_block(body, names, cx),
            EffectKind::Weighted { entries } => {
                for (_, body) in entries {
                    check_block(body, names, cx);
                }
            }
            EffectKind::Rebind { binds, body } => {
                let before = names.len();
                names.extend(binds.iter().map(|(name, _)| name.clone()));
                check_block(body, names, cx);
                names.truncate(before);
            }
            EffectKind::Timer(timer) => check_block(&timer.body, names, cx),
            EffectKind::Ask(ask) => {
                if let AskOptions::Listed(options) = &ask.options {
                    for option in options {
                        check_block(&option.body, names, cx);
                    }
                }
            }
            EffectKind::Run { call, .. } | EffectKind::Queue { call } => {
                let Target::Rule(id) = &call.rule else {
                    continue;
                };
                let Some(rule) = cx.defs().get::<EocDef>(*id) else {
                    continue;
                };
                for needed in rule.needs_from_caller() {
                    if !names.iter().any(|name| **name == *needed) {
                        let called = cx.defs().name::<EocDef>(*id).unwrap_or("?").to_owned();
                        let diagnostic = Diagnostic::new(
                            codes::BAD_EFFECT,
                            format!(
                                "the rule `{called}` is given the subject `{needed}` by whoever calls it, and no subject of that name is bound here; bind one with `{{ \"as\": {{ \"{needed}\": … }}, \"do\": […] }}`"
                            ),
                        )
                        .at(cx.sources(), effect.span)
                        .with_path(&*effect.path);
                        cx.report(diagnostic);
                    }
                }
            }
            _ => {}
        }
    }
}
