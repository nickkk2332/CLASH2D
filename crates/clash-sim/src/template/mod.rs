//! Templates, choices and creation (`docs/06-entities-and-bodies.md`
//! section 10). Not built yet: a typed stub.

use std::sync::Arc;

use clash_data::{DefTypes, Defs};

use crate::reason::ReasonInfo;
use crate::verb::VerbDef;

/// The reasons of templates and creation.
pub const TEMPLATE_REASONS: &[ReasonInfo] = &[];

/// The templates' tables. Not state.
#[derive(Clone, Debug, Default)]
pub struct TemplateDefs;

impl TemplateDefs {
    /// The tables for a set of loaded definitions.
    pub(crate) fn new(_defs: &Arc<Defs>) -> TemplateDefs {
        TemplateDefs
    }
}

/// Registers the definition types of templates.
pub fn register_defs(_types: &mut DefTypes) {}

/// The verbs of templates, for the engine's verb table.
pub fn verbs() -> Vec<VerbDef> {
    Vec::new()
}
