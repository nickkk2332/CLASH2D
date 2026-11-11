//! The CLASH2D game binary, `clash`: starts a profile and runs it, in a
//! terminal or from a script. Everything it does is
//! [`clash_app::run`]; the command line is described in
//! [`clash_app::cli`].

use std::process::ExitCode;

fn main() -> ExitCode {
    clash_app::run(std::env::args().skip(1))
}
