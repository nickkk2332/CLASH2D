//! The compile stage (04 §8.1; 05 §14): the loader's registry is
//! completed from what mods define, and then every expression field of
//! every definition is compiled against it.
//!
//! In order:
//!
//! 1. The registry the host supplied is copied, and the ids of every
//!    definition type are set in it, so that a text compared with an id
//!    is checked (05 §5.2).
//! 2. Each crate's registry steps run.
//! 3. `variable` declarations are registered.
//! 4. `function` definitions are registered by signature, so that
//!    bodies may call each other in any order. Their bodies are read to
//!    find which functions each calls; from that, whether each is random
//!    and whether it is safe for belief (05 §12, §7.3), and whether any
//!    calls itself on every path, which is an error.
//! 5. Function bodies and variable defaults are compiled.
//! 6. Every definition type compiles its own fields.
//!
//! Nothing here depends on the order files were read in: definitions
//! are walked in order of dense id, which is order of qualified id.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use clash_expr::{
    BinaryOp, Context, ExprKind, FunctionId, FunctionSpec, Type, Undeclared, VariableSpec,
};

use super::ExprRegistry;
use super::codes;
use super::defs::{
    CompiledFunction, ExprDefs, FunctionDef, FunctionParam, Functions, VariableDecl, VariableDef,
    Variables,
};
use super::field::{Any, Expr, QueryOp, Ready, ValueSource, Written};
use super::scope::{CompileCx, CompileEnv, Scope, apply_mod_scope};
use crate::de::IdTable;
use crate::defs::{
    AnyStaged, Def, DefTypes, DefsView, RegistryCx, StagedDefs, TagTable, staged_defs, stand_in_for,
};
use crate::diag::{DefLabel, Diagnostic, Diagnostics, closest, did_you_mean};
use crate::fields::Site;
use crate::merge::ModTable;
use crate::source::{SourceMap, Span, to_u32};
use crate::units::parse_quantity;

/// What the compile stage needs from the load.
pub(crate) struct StageInput<'a> {
    /// The registry the host supplied, if it supplied one.
    pub(crate) base: Option<&'a ExprRegistry>,
    pub(crate) types: &'a DefTypes,
    pub(crate) ids: &'a BTreeMap<&'static str, IdTable>,
    pub(crate) sources: &'a SourceMap,
    pub(crate) mods: &'a ModTable,
    pub(crate) tags: &'a TagTable,
    pub(crate) lenient: bool,
}

/// The scope name of 05 §6 for the kind that a cell is a subject of.
const CELL_SCOPE: &str = "cell";
/// The kind a variable with the scope `cell` lives on.
const CELL_KIND: &str = "loc";
/// The heading functions defined by mods are listed under in the
/// reference.
const FUNCTION_GROUP: &str = "Defined by mods";

/// The parameters of a function: each name, its type and whether it may
/// be left out.
type ParamList = Vec<(String, Type, bool)>;

/// One `function` definition on its way through the stage.
struct FnInfo {
    /// Its position among the staged definitions: its dense id.
    index: usize,
    label: DefLabel,
    id_span: Span,
    params: ParamList,
    returns: Type,
    function: Option<FunctionId>,
}

/// A call of one function defined in data from the body of another.
struct CallSite {
    /// The callee, as a position in the list of [`FnInfo`].
    callee: usize,
    /// Where the callee's name was written.
    span: Option<Span>,
    path: String,
    /// Whether the call is made on every path through the body.
    unconditional: bool,
}

/// What reading a body found.
#[derive(Default)]
struct Scan {
    calls: Vec<CallSite>,
    random: bool,
    belief_unsafe: bool,
}

/// Marks a definition broken, with what a lenient load puts in its
/// place.
fn break_def<T: Def>(
    defs: &mut StagedDefs<'_, T>,
    index: usize,
    input: &StageInput<'_>,
    diagnostics: &mut Diagnostics,
) {
    defs.items[index] = None;
    if input.lenient {
        let label = DefLabel::new(T::TYPE, defs.names[index].clone());
        defs.items[index] =
            stand_in_for::<T>(&label, defs.spans[index], input.sources, diagnostics);
        defs.placeholders[index] = defs.items[index].is_some();
    }
}

/// Runs the compile stage over every staged definition.
pub(crate) fn compile_stage(
    input: &StageInput<'_>,
    staged: &mut BTreeMap<&'static str, Box<dyn AnyStaged>>,
    diagnostics: &mut Diagnostics,
) -> ExprDefs {
    let sources = input.sources;
    let mut registry = input
        .base
        .cloned()
        .unwrap_or_else(ExprRegistry::with_builtins);

    // 1. Definition ids. One written in place cannot be referred to.
    for info in input.types.infos() {
        let Some(table) = input.ids.get(info.name) else {
            continue;
        };
        let ty = match registry.def_type_id(info.name) {
            Some(ty) => ty,
            None => match registry.add_def_type(info.name, info.doc) {
                Ok(ty) => ty,
                Err(_) => continue,
            },
        };
        let ids = table
            .names
            .iter()
            .enumerate()
            .filter(|(_, name)| !name.contains('#'))
            .map(|(index, name)| (name.clone(), to_u32(index)));
        registry.set_def_ids(ty, ids);
    }

    // 2. What other crates' definitions declare.
    {
        let view = DefsView {
            staged: &*staged,
            tags: input.tags,
            exprs: None,
        };
        for step in input.types.registry_steps() {
            let mut cx = RegistryCx {
                view: &view,
                registry: &mut registry,
                sources,
                diagnostics: &mut *diagnostics,
            };
            step(&mut cx);
        }
    }

    let no_functions = Functions::default();
    let origins = RefCell::new(Vec::new());

    // 3. Variables: read each declaration, then register it.
    let mut variable_specs: Vec<(usize, VariableSpec)> = Vec::new();
    if let Some(mut defs) = staged_defs::<VariableDef>(staged) {
        let env = CompileEnv {
            sources,
            registry: &registry,
            mods: input.mods,
            functions: &no_functions,
            origins: &origins,
        };
        for index in 0..defs.items.len() {
            let (Some(def), Some(id_span), false) = (
                &defs.items[index],
                defs.spans[index],
                defs.placeholders[index],
            ) else {
                continue;
            };
            let label = DefLabel::new(VariableDef::TYPE, defs.names[index].clone());
            let mut cx = CompileCx::new(&env, &label, Some(id_span));
            let spec = variable_spec(def, &defs.names[index], &mut cx);
            let failed = cx.failed();
            diagnostics.extend(cx.into_diagnostics());
            match spec {
                Some(spec) if !failed => variable_specs.push((index, spec)),
                _ => break_def(&mut defs, index, input, diagnostics),
            }
        }
    }
    let mut variables = Variables::default();
    for (index, spec) in variable_specs {
        let Some(mut defs) = staged_defs::<VariableDef>(staged) else {
            break;
        };
        match registry.add_variable(spec.clone()) {
            Ok(variable) => {
                let Some(def) = &defs.items[index] else {
                    continue;
                };
                variables.items.insert(
                    spec.name.clone(),
                    VariableDecl {
                        name: spec.name,
                        variable,
                        kind: spec.kind,
                        ty: spec.ty,
                        default: clash_expr::Value::Null,
                        watch: def.watch,
                        belief_safe: def.belief_safe,
                        doc: def.doc.clone(),
                    },
                );
            }
            Err(error) => {
                let label = DefLabel::new(VariableDef::TYPE, defs.names[index].clone());
                let mut diagnostic =
                    Diagnostic::new(codes::BAD_VARIABLE, error.to_string()).in_def(Some(&label));
                if let Some(span) = defs.spans[index] {
                    diagnostic = diagnostic.at(sources, span);
                }
                diagnostics.push(diagnostic);
                break_def(&mut defs, index, input, diagnostics);
            }
        }
    }

    // 4. Functions: signatures first.
    let mut infos: Vec<FnInfo> = Vec::new();
    let mut function_count = 0;
    if let Some(mut defs) = staged_defs::<FunctionDef>(staged) {
        function_count = defs.items.len();
        let env = CompileEnv {
            sources,
            registry: &registry,
            mods: input.mods,
            functions: &no_functions,
            origins: &origins,
        };
        for index in 0..defs.items.len() {
            let (Some(def), Some(id_span), false) = (
                &defs.items[index],
                defs.spans[index],
                defs.placeholders[index],
            ) else {
                continue;
            };
            let label = DefLabel::new(FunctionDef::TYPE, defs.names[index].clone());
            let mut cx = CompileCx::new(&env, &label, Some(id_span));
            let signature = function_signature(def, &mut cx);
            let failed = cx.failed();
            diagnostics.extend(cx.into_diagnostics());
            match signature {
                Some((params, returns)) if !failed => infos.push(FnInfo {
                    index,
                    label,
                    id_span,
                    params,
                    returns,
                    function: None,
                }),
                _ => break_def(&mut defs, index, input, diagnostics),
            }
        }
    }
    let mut by_function: BTreeMap<FunctionId, usize> = BTreeMap::new();
    let mut sound = vec![true; infos.len()];
    for (at, info) in infos.iter_mut().enumerate() {
        // One written in place has no name to be called by.
        if info.label.id.contains('#') {
            continue;
        }
        let Some(defs) = staged_defs::<FunctionDef>(staged) else {
            break;
        };
        let def = defs.items[info.index].as_ref();
        let doc = def.map_or("", |def| def.doc.as_str());
        let mut spec = FunctionSpec::new(info.label.id.clone(), info.returns.clone())
            .doc(doc)
            .group(FUNCTION_GROUP);
        for (at, (name, ty, optional)) in info.params.iter().enumerate() {
            let doc = def
                .and_then(|def| def.params.0.get(at))
                .map_or("", |decl| decl.doc.as_str());
            spec = if *optional {
                spec.optional_param(name.clone(), ty.clone(), doc)
            } else {
                spec.param(name.clone(), ty.clone(), doc)
            };
        }
        match registry.add_function(spec) {
            Ok(function) => {
                info.function = Some(function);
                by_function.insert(function, at);
            }
            Err(error) => {
                diagnostics.push(
                    Diagnostic::new(codes::BAD_FUNCTION, error.to_string())
                        .at(sources, info.id_span)
                        .in_def(Some(&info.label)),
                );
                sound[at] = false;
            }
        }
    }

    // What each body calls; from that, randomness, belief and recursion.
    let mut scans: Vec<Scan> = Vec::with_capacity(infos.len());
    if let Some(defs) = staged_defs::<FunctionDef>(staged) {
        let analysis = Analysis {
            registry: &registry,
            sources,
            mods: input.mods,
            by_function: &by_function,
        };
        for info in &infos {
            let mut scan = Scan::default();
            if let Some(def) = &defs.items[info.index] {
                let mut context = Context::new().belief().undeclared(Undeclared::Allow);
                for (name, ty, _) in &info.params {
                    context.add_local(name.clone(), ty.clone());
                }
                analysis.scan_field(&def.value, &info.returns, &context, true, &mut scan);
            }
            scans.push(scan);
        }
    }
    let count = infos.len();
    let mut callers: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut always: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (at, scan) in scans.iter().enumerate() {
        for call in &scan.calls {
            callers[call.callee].push(at);
            if call.unconditional {
                always[at].push(call.callee);
            }
        }
    }
    let mut random: Vec<bool> = scans.iter().map(|scan| scan.random).collect();
    let mut belief_unsafe: Vec<bool> = scans.iter().map(|scan| scan.belief_unsafe).collect();
    spread(&mut random, &callers);
    spread(&mut belief_unsafe, &callers);
    for (at, info) in infos.iter().enumerate() {
        if let Some(function) = info.function {
            registry.set_function_traits(function, random[at], !belief_unsafe[at]);
        }
        let Some(cycle) = cycle_through(at, &always) else {
            continue;
        };
        let names: Vec<&str> = cycle
            .iter()
            .map(|&member| infos[member].label.id.as_str())
            .collect();
        let site = scans[at]
            .calls
            .iter()
            .find(|call| call.unconditional && Some(&call.callee) == cycle.get(1));
        let mut diagnostic = Diagnostic::new(
            codes::ENDLESS_RECURSION,
            format!(
                "function `{}` always calls itself: {}",
                info.label.id,
                names.join(" → ")
            ),
        )
        .in_def(Some(&info.label))
        .suggest("make the call depend on a condition: `if … then … else`, `and`, `or` or `??`");
        diagnostic = match site {
            Some(CallSite {
                span: Some(span),
                path,
                ..
            }) => diagnostic.at(sources, *span).with_path(path.clone()),
            _ => diagnostic.at(sources, info.id_span),
        };
        diagnostics.push(diagnostic);
        sound[at] = false;
    }

    // 5. Defaults, then bodies, against the finished registry.
    let mut functions = Functions {
        items: (0..function_count)
            .map(|_| CompiledFunction {
                name: String::new(),
                function: None,
                params: Vec::new(),
                returns: Type::Any,
                random: false,
                belief_safe: false,
                doc: String::new(),
                body: None,
                origin: 0,
            })
            .collect(),
        by_function: Vec::new(),
    };
    if let Some(mut defs) = staged_defs::<FunctionDef>(staged) {
        for (index, name) in defs.names.iter().enumerate() {
            if let Some(slot) = functions.items.get_mut(index) {
                slot.name.clone_from(name);
            }
        }
        let env = CompileEnv {
            sources,
            registry: &registry,
            mods: input.mods,
            functions: &no_functions,
            origins: &origins,
        };
        for (at, info) in infos.iter().enumerate() {
            if !sound[at] {
                break_def(&mut defs, info.index, input, diagnostics);
                continue;
            }
            let Some(def) = &mut defs.items[info.index] else {
                continue;
            };
            let mut cx = CompileCx::new(&env, &info.label, Some(info.id_span));
            let mut params = Vec::with_capacity(info.params.len());
            for (decl, (name, ty, _)) in def.params.0.iter_mut().zip(&info.params) {
                let mut default = None;
                if let Some(written) = &mut decl.default {
                    cx.compile(written, &Scope::new().pure().expecting(ty.clone()));
                    default = written.constant().cloned();
                    if default.is_none() && written.is_compiled() {
                        cx.error_at(
                            codes::BAD_FUNCTION,
                            written.site(),
                            format!("the default of `{name}` is not a constant"),
                        );
                    }
                }
                params.push(FunctionParam {
                    name: name.clone(),
                    ty: ty.clone(),
                    default,
                });
            }
            let failed = cx.failed();
            diagnostics.extend(cx.into_diagnostics());
            if failed {
                sound[at] = false;
                break_def(&mut defs, info.index, input, diagnostics);
                continue;
            }
            let slot = &mut functions.items[info.index];
            slot.function = info.function;
            slot.params = params;
            slot.returns = info.returns.clone();
            slot.random = random[at];
            slot.belief_safe = !belief_unsafe[at];
            slot.doc.clone_from(&def.doc);
            if let Some(function) = info.function {
                functions.index(function, info.index);
            }
        }
    }
    let mut bodies: Vec<(usize, Option<Ready>, u64)> = Vec::new();
    if let Some(mut defs) = staged_defs::<FunctionDef>(staged) {
        let env = CompileEnv {
            sources,
            registry: &registry,
            mods: input.mods,
            functions: &functions,
            origins: &origins,
        };
        for (at, info) in infos.iter().enumerate() {
            if !sound[at] {
                continue;
            }
            let Some(def) = &mut defs.items[info.index] else {
                continue;
            };
            let mut cx = CompileCx::new(&env, &info.label, Some(info.id_span));
            let mut scope = Scope::new().expecting(info.returns.clone());
            for (name, ty, _) in &info.params {
                scope = scope.local(name.clone(), ty.clone());
            }
            let origin = cx.origin(def.value.site());
            cx.compile(&mut def.value, &scope);
            let body = def.value.ready.clone();
            let failed = cx.failed();
            diagnostics.extend(cx.into_diagnostics());
            if failed {
                break_def(&mut defs, info.index, input, diagnostics);
            } else {
                bodies.push((info.index, body, origin));
            }
        }
    }
    for (index, body, origin) in bodies {
        functions.items[index].body = body;
        functions.items[index].origin = origin;
    }

    let env = CompileEnv {
        sources,
        registry: &registry,
        mods: input.mods,
        functions: &functions,
        origins: &origins,
    };
    if let Some(mut defs) = staged_defs::<VariableDef>(staged) {
        for index in 0..defs.items.len() {
            let name = &defs.names[index];
            let Some(decl) = variables.items.get_mut(name) else {
                continue;
            };
            let (Some(def), Some(id_span)) = (&mut defs.items[index], defs.spans[index]) else {
                continue;
            };
            let Some(written) = &mut def.default else {
                continue;
            };
            let label = DefLabel::new(VariableDef::TYPE, name.clone());
            let mut cx = CompileCx::new(&env, &label, Some(id_span));
            cx.compile(written, &Scope::new().pure().expecting(decl.ty.clone()));
            match written.constant() {
                Some(value) => decl.default = value.clone(),
                None if written.is_compiled() => cx.error_at(
                    codes::BAD_VARIABLE,
                    written.site(),
                    "a variable's default is a constant",
                ),
                None => {}
            }
            let failed = cx.failed();
            diagnostics.extend(cx.into_diagnostics());
            if failed {
                break_def(&mut defs, index, input, diagnostics);
            }
        }
    }

    // 6. Every definition type's own fields.
    for one in staged.values_mut() {
        one.compile(&env, input.lenient, diagnostics);
    }

    let origins = origins.into_inner();
    ExprDefs {
        registry: Arc::new(registry),
        functions,
        variables,
        origins,
    }
}

/// Sets every flag that follows from a set one: a function is random,
/// or unsafe for belief, if anything it calls is.
fn spread(flags: &mut [bool], callers: &[Vec<usize>]) {
    let mut work: Vec<usize> = (0..flags.len()).filter(|&at| flags[at]).collect();
    while let Some(at) = work.pop() {
        for &caller in &callers[at] {
            if !flags[caller] {
                flags[caller] = true;
                work.push(caller);
            }
        }
    }
}

/// A chain of calls made on every path that leads from `start` back to
/// it, if there is one: `start`, what it calls, …, `start`.
fn cycle_through(start: usize, edges: &[Vec<usize>]) -> Option<Vec<usize>> {
    let mut parent: Vec<Option<usize>> = vec![None; edges.len()];
    let mut seen = vec![false; edges.len()];
    let mut queue = VecDeque::new();
    seen[start] = true;
    queue.push_back(start);
    while let Some(at) = queue.pop_front() {
        for &next in &edges[at] {
            if next == start {
                let mut chain = vec![at];
                let mut back = at;
                while let Some(before) = parent[back] {
                    chain.push(before);
                    back = before;
                }
                chain.reverse();
                chain.push(start);
                return Some(chain);
            }
            if !seen[next] {
                seen[next] = true;
                parent[next] = Some(at);
                queue.push_back(next);
            }
        }
    }
    None
}

/// Whether a name can be a parameter: an identifier of the language.
fn is_param_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Reads a function's parameters and result type. `None`, with the
/// reasons reported, if any of it is wrong.
fn function_signature(def: &FunctionDef, cx: &mut CompileCx<'_>) -> Option<(ParamList, Type)> {
    let sources = cx.sources();
    let mut params: Vec<(String, Type, bool)> = Vec::new();
    let mut sound = true;
    let mut optional_seen: Option<&str> = None;
    for decl in &def.params.0 {
        let name = decl.name.value.as_str();
        let path = format!("params.{name}");
        let refuse = |cx: &mut CompileCx<'_>, message: String| {
            cx.report(
                Diagnostic::new(codes::BAD_FUNCTION, message)
                    .at(sources, decl.name.span)
                    .with_path(path.clone()),
            );
        };
        if !is_param_name(name) {
            refuse(
                cx,
                format!(
                    "\"{name}\" cannot be a parameter: a name is letters, digits and `_`, not starting with a digit"
                ),
            );
            sound = false;
        }
        if params.iter().any(|(known, _, _)| known == name) {
            refuse(cx, format!("the parameter `{name}` is declared twice"));
            sound = false;
        }
        let optional = decl.default.is_some();
        match (optional, optional_seen) {
            (true, None) => optional_seen = Some(name),
            (false, Some(earlier)) => {
                refuse(
                    cx,
                    format!(
                        "the parameter `{name}` has no default and comes after `{earlier}`, which has one"
                    ),
                );
                sound = false;
            }
            _ => {}
        }
        match cx.parse_type(&decl.ty.value, Some(decl.ty.span), &path) {
            Some(ty) => params.push((name.to_owned(), ty, optional)),
            None => sound = false,
        }
    }
    let returns = match &def.returns {
        Some(written) => match cx.parse_type(&written.value, Some(written.span), "returns") {
            Some(ty) => ty,
            None => {
                sound = false;
                Type::Any
            }
        },
        None => Type::Any,
    };
    sound.then_some((params, returns))
}

/// Reads a variable declaration into what the registry takes. `None`,
/// with the reasons reported, if any of it is wrong.
fn variable_spec(def: &VariableDef, name: &str, cx: &mut CompileCx<'_>) -> Option<VariableSpec> {
    let registry = cx.registry();
    let sources = cx.sources();
    if name.contains('#') {
        cx.report(Diagnostic::new(
            codes::BAD_VARIABLE,
            "a variable is declared at the top level of a data file, not in place inside another definition",
        ));
        return None;
    }
    let scope = def.scope.value.as_str();
    let kind_name = if scope == CELL_SCOPE {
        CELL_KIND
    } else {
        scope
    };
    let kind = registry.kind_id(kind_name);
    if kind.is_none() {
        let kinds = registry
            .kinds()
            .map(|kind| kind.spec.name.as_str())
            .chain([CELL_SCOPE]);
        cx.report(
            Diagnostic::new(
                codes::BAD_VARIABLE,
                format!("\"{scope}\" is not something a variable can live on"),
            )
            .at(sources, def.scope.span)
            .with_path("scope")
            .suggest_opt(did_you_mean(closest(scope, kinds))),
        );
    }
    let ty = cx.parse_type(&def.value.value, Some(def.value.span), "value");
    let (kind, ty) = (kind?, ty?);
    // With no default the variable reads as null until it is set.
    let ty = if def.default.is_some() {
        ty
    } else {
        ty.nullable()
    };
    Some(
        VariableSpec::new(name, kind, ty)
            .doc(def.doc.clone())
            .belief_safe(def.belief_safe),
    )
}

/// What reading function bodies needs.
struct Analysis<'a> {
    registry: &'a ExprRegistry,
    sources: &'a SourceMap,
    mods: &'a ModTable,
    by_function: &'a BTreeMap<FunctionId, usize>,
}

impl Analysis<'_> {
    /// Reads one field of a body: what it calls, whether it draws at
    /// random and whether it reads what belief may not. `context` is the
    /// function's parameters in a mind context; mistakes in the field
    /// are left for compiling to report.
    fn scan_field(
        &self,
        field: &Expr<Any>,
        expected: &Type,
        context: &Context,
        unconditional: bool,
        scan: &mut Scan,
    ) {
        match field.written() {
            Written::Literal(_) => {}
            Written::Text(text) => {
                let wanted = expected.non_null();
                let literal = matches!(wanted, Type::Str | Type::Id(_))
                    || (wanted.is_numeric() && parse_quantity(text).is_ok());
                if !literal {
                    self.scan_source(text, field.site(), context, unconditional, scan);
                }
            }
            Written::Code { source, site } => {
                self.scan_source(source, site, context, unconditional, scan);
            }
            Written::Source(source) => match &**source {
                ValueSource::Query { op, query, .. } => {
                    // A query is safe for belief if its kind is, and its
                    // `where` and `sort`, read with `it` bound, read
                    // nothing that is not; `weight` and `of` likewise.
                    scan.random |= *op == QueryOp::Pick;
                    let Some((kind, body)) = query.raw.as_object().and_then(|object| {
                        let mut entries = object.iter();
                        let first = entries.next()?;
                        entries.next().is_none().then_some(first)
                    }) else {
                        scan.belief_unsafe = true;
                        return;
                    };
                    let safe = match kind.as_str() {
                        super::query::LIST_QUERY | super::query::DEFS_QUERY => true,
                        name => self
                            .registry
                            .query_id(name)
                            .and_then(|id| self.registry.query(id))
                            .is_some_and(|kind| kind.spec.belief_safe),
                    };
                    if !safe {
                        scan.belief_unsafe = true;
                        return;
                    }
                    let mut with_it = context.clone();
                    with_it.add_subject(super::query::IT, Type::Any, false);
                    for key in ["where", "sort"] {
                        match body.get(key) {
                            None => {}
                            Some(serde_json::Value::String(text)) => {
                                self.scan_source(text, field.site(), &with_it, false, scan);
                            }
                            // A structure of conditions is not looked into.
                            Some(_) => scan.belief_unsafe = true,
                        }
                    }
                }
                ValueSource::Run(_) => {
                    scan.belief_unsafe = true;
                    scan.random = true;
                }
                ValueSource::Call { function, args } => {
                    let context = apply_mod_scope(context.clone(), self.mods, Some(function.span));
                    let found = self
                        .registry
                        .resolve_function(&function.value, context.scope())
                        .and_then(|resolved| self.registry.function(resolved.found));
                    if let Some(found) = found {
                        self.note_call(
                            found.id,
                            Some(function.span),
                            &field.site().child("call").path,
                            unconditional,
                            scan,
                        );
                    }
                    for (name, arg) in args {
                        let ty = found
                            .and_then(|found| {
                                found.spec.params.iter().find(|param| param.name == *name)
                            })
                            .map_or(Type::Any, |param| param.ty.clone());
                        self.scan_field(arg, &ty, &context, unconditional, scan);
                    }
                }
            },
        }
    }

    fn note_call(
        &self,
        function: FunctionId,
        span: Option<Span>,
        path: &str,
        unconditional: bool,
        scan: &mut Scan,
    ) {
        if let Some(&callee) = self.by_function.get(&function) {
            scan.calls.push(CallSite {
                callee,
                span,
                path: path.to_owned(),
                unconditional,
            });
        } else if let Some(found) = self.registry.function(function) {
            scan.random |= found.spec.random;
            scan.belief_unsafe |= !found.spec.belief_safe;
        }
    }

    fn scan_source(
        &self,
        source: &str,
        site: &Site,
        context: &Context,
        unconditional: bool,
        scan: &mut Scan,
    ) {
        let context = apply_mod_scope(context.clone(), self.mods, site.span);
        let Ok(tree) = clash_expr::parse_expr(source) else {
            return;
        };
        let mut calls = Vec::new();
        walk_calls(&tree, unconditional, &mut calls);
        for (name, span, always) in calls {
            let found = self.registry.resolve_function(&name, context.scope());
            if let Some(found) = found {
                let span = site.span.map(|string| {
                    self.sources
                        .span_in_string(string, span.start as usize, span.end as usize)
                });
                self.note_call(found.found, span, &site.path, always, scan);
            }
        }
        // Every function of a mod counts as safe at this point, so what
        // the checker refuses here is what this body reads itself.
        let outcome = clash_expr::check(source, self.registry, &context, &Type::Any);
        scan.belief_unsafe |= outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == clash_expr::codes::NOT_BELIEF_SAFE);
    }
}

/// Collects the functions an expression calls: each name as written,
/// where, and whether the call is made whatever the values are. What
/// follows `and`, `or` or `??`, and the branches of an `if`, may not be
/// evaluated.
fn walk_calls(
    expr: &clash_expr::Expr,
    always: bool,
    out: &mut Vec<(String, clash_expr::Span, bool)>,
) {
    match &expr.kind {
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Number { .. }
        | ExprKind::Str(_)
        | ExprKind::Name(_)
        | ExprKind::Local(_) => {}
        ExprKind::List(items) => {
            for item in items {
                walk_calls(item, always, out);
            }
        }
        ExprKind::Map(entries) => {
            for (key, value) in entries {
                walk_calls(key, always, out);
                walk_calls(value, always, out);
            }
        }
        ExprKind::Member { object, .. } => walk_calls(object, always, out),
        ExprKind::Call { callee, args } => {
            match &callee.kind {
                ExprKind::Name(name) => out.push((name.to_string(), callee.span, always)),
                _ => walk_calls(callee, always, out),
            }
            for arg in args {
                walk_calls(arg, always, out);
            }
        }
        ExprKind::Index { object, index } => {
            walk_calls(object, always, out);
            walk_calls(index, always, out);
        }
        ExprKind::Unary { operand, .. } => walk_calls(operand, always, out),
        ExprKind::Binary { op, lhs, rhs } => {
            walk_calls(lhs, always, out);
            let short = matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Coalesce);
            walk_calls(rhs, always && !short, out);
        }
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            walk_calls(cond, always, out);
            walk_calls(then, false, out);
            walk_calls(otherwise, false, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cycle_is_found_with_its_chain() {
        // 0 → 1 → 2 → 0, and 3 → 0 which is on no cycle.
        let edges = vec![vec![1], vec![2], vec![0], vec![0]];
        assert_eq!(cycle_through(0, &edges), Some(vec![0, 1, 2, 0]));
        assert_eq!(cycle_through(1, &edges), Some(vec![1, 2, 0, 1]));
        assert_eq!(cycle_through(3, &edges), None);
        assert_eq!(cycle_through(0, &[vec![0]]), Some(vec![0, 0]));
        assert_eq!(cycle_through(0, &[vec![]]), None);
    }

    #[test]
    fn a_flag_spreads_to_every_caller() {
        // 0 is called by 1, 1 by 2; 3 stands alone.
        let callers = vec![vec![1], vec![2], vec![], vec![]];
        let mut flags = vec![true, false, false, false];
        spread(&mut flags, &callers);
        assert_eq!(flags, [true, true, true, false]);
    }

    #[test]
    fn calls_after_a_short_circuit_are_conditional() {
        let tree =
            clash_expr::parse_expr("f(1) + (g(2) ?? h(3)) + (if a() then b() else c())").unwrap();
        let mut calls = Vec::new();
        walk_calls(&tree, true, &mut calls);
        let seen: Vec<(&str, bool)> = calls
            .iter()
            .map(|(name, _, always)| (name.as_str(), *always))
            .collect();
        assert_eq!(
            seen,
            [
                ("f", true),
                ("g", true),
                ("h", false),
                ("a", true),
                ("b", false),
                ("c", false)
            ]
        );
    }

    #[test]
    fn parameter_names_are_identifiers() {
        assert!(is_param_name("height"));
        assert!(is_param_name("_x9"));
        assert!(!is_param_name("9x"));
        assert!(!is_param_name("a-b"));
        assert!(!is_param_name(""));
    }
}
