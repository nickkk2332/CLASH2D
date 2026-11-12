//! Making a mod and skeleton definitions (04 §13, 14 §9.1).

use clash_data::DefTypes;

use crate::data::{EXIT_USAGE, Options, Output};

/// `init-mod`: not built yet.
pub(crate) fn init_mod_command(_types: &DefTypes, _options: &Options) -> Output {
    Output {
        out: String::new(),
        err: "clash-tools: `init-mod` is not built yet.\n".to_owned(),
        code: EXIT_USAGE,
    }
}

/// `new`: not built yet.
pub(crate) fn new_command(_types: &DefTypes, _options: &Options) -> Output {
    Output {
        out: String::new(),
        err: "clash-tools: `new` is not built yet.\n".to_owned(),
        code: EXIT_USAGE,
    }
}
