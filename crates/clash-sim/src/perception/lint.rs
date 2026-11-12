//! The linter's check of 15 §7.3: a rule attached to an entity decides
//! what that entity does, and must read the world as the entity believes
//! it, not as it is.
//!
//! Each attached rule's condition is compiled again in the mind context
//! (05 §7.3), with the rule's `self` as the character's own subject.
//! Whatever that compile refuses as unsafe for belief (another entity's
//! place, a cell's contents, the light somewhere) is reported as a
//! warning on the rule, with the belief property to read instead. The
//! rule still loads and runs on truth.

use clash_data::{CrossCx, Def, DefLabel, Diagnostic, Severity, code};
use clash_expr::{Context, Type};

use crate::rule::EocDef;

/// The diagnostic code the compiler gives a read that is not safe for
/// belief.
const NOT_BELIEF_SAFE: &str = "X402";

/// Reports every attached rule whose condition reads truth it could not
/// have perceived.
pub(super) fn check_belief_reads(cx: &mut CrossCx<'_>) {
    let view = cx.defs();
    let Some(registry) = view.expr_registry() else {
        return;
    };
    let mut found: Vec<Diagnostic> = Vec::new();
    for (id, def) in view.iter::<EocDef>() {
        if !def.is_attached() {
            continue;
        }
        let Some(source) = def.condition.source() else {
            continue;
        };
        let names = def.subject_names();
        let mut context = Context::new();
        let mut own_entity = false;
        for (subject, name) in def.subjects().iter().zip(names.iter()) {
            let ty = subject.kind.map_or(Type::Any, Type::Subject);
            let own = &**name == "self";
            if own && registry.kind_id("entity") == subject.kind {
                own_entity = true;
            }
            context.add_subject(name.to_string(), ty, own);
        }
        if !own_entity {
            continue;
        }
        for param in def.params() {
            context.add_local(param.name.to_string(), param.ty.clone());
        }
        if let Some(event) = def.event() {
            context = context.event(event);
        }
        let context = context.belief();
        let outcome = clash_expr::compile(source, registry, &context, &Type::Bool.nullable());
        let Some(refused) = outcome
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_str() == NOT_BELIEF_SAFE)
        else {
            continue;
        };
        let name = view.name::<EocDef>(id).unwrap_or("?");
        let label = DefLabel::new(EocDef::TYPE, name);
        let mut diagnostic = Diagnostic::new(
            code::VALIDATION,
            format!(
                "this rule is attached to an entity and decides what it does, but its condition reads what the entity has not perceived: {}",
                refused.message
            ),
        )
        .with_severity(Severity::Warning)
        .in_def(Some(&label))
        .with_path("if")
        .suggest("read what the character believes: `self.believed_loc(x)`, `self.perceives(x)`, `self.knows(x)` (15 §7.2)");
        if let Some(span) = def.condition.span() {
            diagnostic = diagnostic.at(cx.sources(), span);
        }
        found.push(diagnostic);
    }
    for diagnostic in found {
        cx.report(diagnostic);
    }
}
