//! Finding definitions and what refers to them (14 §9.2).

use clash_data::DefTypes;

use crate::data::{EXIT_USAGE, Options, Output};

/// `refs`: not built yet.
pub(crate) fn refs_command(_types: &DefTypes, _options: &Options) -> Output {
    Output {
        out: String::new(),
        err: "clash-tools: `refs` is not built yet.\n".to_owned(),
        code: EXIT_USAGE,
    }
}

/// `find`: not built yet.
pub(crate) fn find_command(_types: &DefTypes, _options: &Options) -> Output {
    Output {
        out: String::new(),
        err: "clash-tools: `find` is not built yet.\n".to_owned(),
        code: EXIT_USAGE,
    }
}
