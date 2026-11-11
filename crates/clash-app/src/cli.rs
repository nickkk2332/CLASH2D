//! The command line (12 §3.1).
//!
//! ```text
//! clash [--profile name] [--mods dir] [--seed n] [--data dir] [--headless script]
//! ```
//!
//! | Option | Means |
//! |--------|-------|
//! | `--profile name` | The profile to start: a name looked up in `profiles/`, or a file. Left out, `default`. |
//! | `--mods dir` | The directory mods are read from. Left out, it is looked for ([`crate::paths`]). |
//! | `--seed n` | The seed a new game is offered. Left out, 1. |
//! | `--data dir` | Where preferences and key bindings are kept, in place of the user's data directory. |
//! | `--headless script` | Runs without a terminal, taking input from a script ([`crate::headless`]); `-` reads it from standard input. |
//! | `--help` | Prints this and leaves. |

use std::path::PathBuf;

/// The profile started when none is named (04 §3.3).
pub const DEFAULT_PROFILE: &str = "default";

/// The seed a new game is offered when none is given.
pub const DEFAULT_SEED: u64 = 1;

/// What the command line says to all who ask how it is used.
pub const USAGE: &str = "\
usage: clash [--profile name] [--mods dir] [--seed n] [--data dir] [--headless script]

  --profile name     the profile to start: a name in `profiles/`, or a file (default: default)
  --mods dir         the directory mods are read from (default: `mods` beside the
                     working directory or the executable)
  --seed n           the seed a new game is offered (default: 1)
  --data dir         where preferences and key bindings are kept (default: the
                     user's data directory; see CLASH_DATA_DIR)
  --headless script  run without a terminal, taking input from a script of key
                     names; `-` reads the script from standard input
  --help             print this
";

/// What the command line asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The profile to start: a name, or a file.
    pub profile: String,
    /// The directory mods are read from, if one was named.
    pub mods: Option<PathBuf>,
    /// The seed a new game is offered.
    pub seed: u64,
    /// Where preferences and key bindings are kept, if a place was
    /// named.
    pub data: Option<PathBuf>,
    /// The script to run without a terminal, if one was named.
    pub headless: Option<PathBuf>,
    /// Whether `--help` was given.
    pub help: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            profile: DEFAULT_PROFILE.to_owned(),
            mods: None,
            seed: DEFAULT_SEED,
            data: None,
            headless: None,
            help: false,
        }
    }
}

/// Reads the arguments that follow the program's name. A mistake is
/// returned as a sentence to print with [`USAGE`].
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        // `--name=value` and `--name value` mean the same.
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_owned(), Some(value.to_owned()))
            }
            _ => (arg.clone(), None),
        };
        let mut value = |what: &str| {
            inline
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| format!("`{name}` needs {what} after it"))
        };
        match name.as_str() {
            "--profile" => options.profile = value("a profile's name or file")?,
            "--mods" => options.mods = Some(PathBuf::from(value("a directory")?)),
            "--data" => options.data = Some(PathBuf::from(value("a directory")?)),
            "--headless" => options.headless = Some(PathBuf::from(value("a script file")?)),
            "--seed" => {
                let text = value("a whole number")?;
                options.seed = text.parse().map_err(|_| {
                    format!("`--seed` needs a whole number, and `{text}` is not one")
                })?;
            }
            "--help" | "-h" => options.help = true,
            other => return Err(format!("`{other}` is not an option of clash")),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_are_read_in_both_spellings() {
        let options = parse(args(
            "--profile ashlands --mods=other/mods --seed 7 --headless s.txt",
        ))
        .unwrap();
        assert_eq!(options.profile, "ashlands");
        assert_eq!(options.mods, Some(PathBuf::from("other/mods")));
        assert_eq!(options.seed, 7);
        assert_eq!(options.headless, Some(PathBuf::from("s.txt")));
        assert_eq!(parse(Vec::new()).unwrap(), Options::default());
        assert!(parse(args("--help")).unwrap().help);
    }

    #[test]
    fn a_mistake_says_what_is_wrong() {
        assert_eq!(
            parse(args("--seed many")).unwrap_err(),
            "`--seed` needs a whole number, and `many` is not one"
        );
        assert_eq!(
            parse(args("--profile")).unwrap_err(),
            "`--profile` needs a profile's name or file after it"
        );
        assert_eq!(
            parse(args("--fast")).unwrap_err(),
            "`--fast` is not an option of clash"
        );
    }
}
