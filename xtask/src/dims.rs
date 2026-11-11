//! The check that the sizes of chunks, tracts and regions are written as
//! literals in one place only: `clash_world::dims` (01 R9).
//!
//! It reads the source of the crates that deal in those sizes and looks
//! for an integer literal equal to one of them outside comments, strings
//! and test modules. A line that has to carry such a number for another
//! reason says so with a `dims-ok` comment on the same line.
//!
//! The height of a chunk in storeys (8) is not looked for: the number is
//! far too common to tell apart. Its companions are checked, and a size
//! built from it is caught through them.

use std::fs;
use std::path::{Path, PathBuf};

/// The sizes that may be written only in the dims module: a chunk's and a
/// tract's side, a region's side in tracts, and the products built from
/// them (cell columns per chunk, cells per region side, tracts per
/// region, cells per chunk).
const SIZE_LITERALS: [u64; 6] = [32, 64, 1024, 2048, 4096, 8192];

/// The source directories checked, relative to the workspace root.
const CHECKED: [&str; 2] = ["crates/clash-world/src", "crates/clash-gen/src"];

/// The one file that may write the sizes.
const DIMS_FILE: &str = "crates/clash-world/src/dims.rs";

/// The marker that exempts a line.
const EXEMPT: &str = "dims-ok";

/// The start of a test module. Everything from here to the end of the
/// file is test code, which may name sizes to check them.
const TEST_MODULE: &str = "#[cfg(test)]";

/// The suffixes an integer literal may carry.
const SUFFIXES: [&str; 12] = [
    "usize", "isize", "u128", "i128", "u64", "i64", "u32", "i32", "u16", "i16", "u8", "i8",
];

/// Every place a size literal is written outside the dims module, as
/// `path:line: text`. Empty when the rule holds.
pub fn violations(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for dir in CHECKED {
        collect(&root.join(dir), &mut files);
    }
    files.sort();
    let dims = root.join(DIMS_FILE);
    let mut found = Vec::new();
    for file in files.into_iter().filter(|file| *file != dims) {
        let Ok(text) = fs::read_to_string(&file) else {
            found.push(format!("{}: could not be read", file.display()));
            continue;
        };
        let shown = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (line, literal) in size_literals(&text) {
            found.push(format!(
                "{shown}:{line}: the size {literal} is written as a literal; use a constant of clash_world::dims"
            ));
        }
    }
    found
}

/// The Rust files under a directory, at any depth.
fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// The size literals in one file's text, each with its line number, from
/// the first line down to the first test module.
fn size_literals(text: &str) -> Vec<(usize, u64)> {
    let mut found = Vec::new();
    let mut in_block_comment = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with(TEST_MODULE) {
            break;
        }
        if line.contains(EXEMPT) {
            continue;
        }
        let code = code_of(line, &mut in_block_comment);
        for literal in integer_literals(&code) {
            if SIZE_LITERALS.contains(&literal) {
                found.push((index + 1, literal));
            }
        }
    }
    found
}

/// A line with its comments, strings and character literals blanked out.
fn code_of(line: &str, in_block_comment: &mut bool) -> String {
    let mut code = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if *in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
            }
            continue;
        }
        if in_string {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_string = false,
                _ => {}
            }
            code.push(' ');
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'/') => break,
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                *in_block_comment = true;
            }
            '"' => {
                in_string = true;
                code.push(' ');
            }
            _ => code.push(c),
        }
    }
    code
}

/// The values of the decimal integer literals in a line of code.
fn integer_literals(code: &str) -> Vec<u64> {
    let mut found = Vec::new();
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    for token in code.split(|c: char| !is_word(c)) {
        if !token.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let mut digits = token.replace('_', "");
        for suffix in SUFFIXES {
            if let Some(stripped) = digits.strip_suffix(suffix) {
                digits = stripped.to_owned();
                break;
            }
        }
        // Hexadecimal, octal and binary literals are bit patterns, not
        // sizes, and do not parse as decimal.
        if let Ok(value) = digits.parse::<u64>() {
            found.push(value);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(text: &str) -> Vec<usize> {
        size_literals(text)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    #[test]
    fn a_size_literal_in_code_is_found() {
        assert_eq!(lines_of("let side = 32;\nlet n = 8192_usize;\n"), [1, 2]);
        assert_eq!(lines_of("let cells = [0u8; 1_024];"), [1]);
        assert_eq!(lines_of("let metres = Real::of(64.0);"), [1]);
        assert_eq!(
            lines_of("x >> 5; y & 31; z * 33; w + 320"),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn comments_strings_and_types_are_not_code() {
        assert_eq!(lines_of("// a chunk is 32 cells wide"), Vec::<usize>::new());
        assert_eq!(lines_of("/// 32 × 32 × 8 cells"), Vec::<usize>::new());
        assert_eq!(lines_of("let text = \"32 \\\" 64\";"), Vec::<usize>::new());
        assert_eq!(
            lines_of("let x: u32 = y as u64 as u32;"),
            Vec::<usize>::new()
        );
        assert_eq!(
            lines_of("/* 4096\n   2048 */ let a = 1;\nlet b = 2048;"),
            [3]
        );
        assert_eq!(
            lines_of("let mask = 0x40; let bits = 0b100000;"),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn an_exempt_line_and_test_modules_are_skipped() {
        assert_eq!(
            lines_of("let planes = 32; // dims-ok: bits in a plane mask"),
            Vec::<usize>::new()
        );
        let text = "let a = 1;\n#[cfg(test)]\nmod tests {\n    const SIDE: i32 = 32;\n}\n";
        assert_eq!(lines_of(text), Vec::<usize>::new());
    }

    #[test]
    fn the_workspace_keeps_the_rule() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        assert_eq!(violations(&root), Vec::<String>::new());
    }
}
