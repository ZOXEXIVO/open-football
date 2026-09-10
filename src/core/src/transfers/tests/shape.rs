//! Source-level guards on the two shape rules that keep this tree readable.
//!
//! Both are the kind of rule that is obeyed for months and then quietly
//! broken by one hurried edit, at which point nobody notices until the next
//! person reads the file. Prose in `CLAUDE.md` states them; this asserts
//! them, because a rule with no test is a preference.
//!
//! Scoped to `src/transfers` — the subsystem that has been brought to the
//! rules. Widen the root as other subsystems are cleaned up.

use std::fs;
use std::path::{Path, PathBuf};

/// Walks `src/transfers` and answers questions about the shape of its sources.
struct ShapeScan;

impl ShapeScan {
    /// This file names the forbidden shapes in prose; it is never an offender.
    const SELF: &'static str = "shape.rs";

    fn sources() -> Vec<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("transfers");
        let mut out = Vec::new();
        Self::walk(&root, &mut out);
        assert!(
            !out.is_empty(),
            "no sources found under src/transfers — the scanner's path is wrong, \
             not the codebase"
        );
        out
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    fn label(path: &Path) -> String {
        path.to_string_lossy()
            .replace('\\', "/")
            .rsplit_once("/src/")
            .map(|(_, tail)| tail.to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string())
    }

    /// Every line for which `hit` holds, as `path:line  text`. `hit` receives
    /// the raw line and the one above it, so a check can look at the attribute
    /// sitting over a declaration.
    fn offenders(hit: impl Fn(&str, &str) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        for path in Self::sources() {
            if path.file_name().and_then(|f| f.to_str()) == Some(Self::SELF) {
                continue;
            }
            let Ok(src) = fs::read_to_string(&path) else {
                continue;
            };
            let lines: Vec<&str> = src.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                let above = if index == 0 { "" } else { lines[index - 1].trim() };
                if line.trim_start().starts_with("//") || !hit(line, above) {
                    continue;
                }
                out.push(format!(
                    "{}:{}  {}",
                    Self::label(&path),
                    index + 1,
                    line.trim()
                ));
            }
        }
        out
    }

    /// A `fn` at column 0 is a free function. Everything else hangs off
    /// something.
    fn is_free_fn(line: &str) -> bool {
        let after_vis = line
            .strip_prefix("pub ")
            .or_else(|| {
                line.strip_prefix("pub(")
                    .and_then(|rest| rest.split_once(") "))
                    .map(|(_, tail)| tail)
            })
            .unwrap_or(line);
        for keyword in ["const ", "async ", "unsafe "] {
            if let Some(rest) = after_vis.strip_prefix(keyword) {
                return rest.starts_with("fn ");
            }
        }
        after_vis.starts_with("fn ")
    }

    /// Strips the two things that legitimately spell a full path: a
    /// `pub(in crate::a::b)` visibility scope, and a string literal.
    fn without_scopes_and_strings(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut rest = line;
        while let Some(at) = rest.find("pub(in ") {
            out.push_str(&rest[..at]);
            match rest[at..].find(')') {
                Some(close) => rest = &rest[at + close + 1..],
                None => return out,
            }
        }
        out.push_str(rest);

        let mut stripped = String::with_capacity(out.len());
        let mut in_string = false;
        let mut escaped = false;
        for ch in out.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            if ch == '"' {
                in_string = true;
                continue;
            }
            stripped.push(ch);
        }
        stripped
    }

    /// `crate::` followed by two or more segments, i.e. a path to an item
    /// rather than to a module the code is merely naming.
    fn names_an_inline_item(line: &str) -> bool {
        let clean = Self::without_scopes_and_strings(line);
        let Some(at) = clean.find("crate::") else {
            return false;
        };
        clean[at + "crate::".len()..]
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .next()
            .is_some_and(|tail| tail.matches("::").count() >= 1)
    }
}

/// Every function hangs off a struct that names what it is.
///
/// A unit struct is a perfectly good namespace and is the house pattern —
/// `ClubView::can_accept_player`, `GroupNeedScan::needs`,
/// `ListedTargetScreen::evaluate`. A loose `fn` at module level says nothing
/// about which layer it belongs to, which is how `helpers.rs` came to hold
/// four unrelated ones.
///
/// `#[test]` functions are the only exception; the harness requires them free.
/// Test *helpers* are not exempt — wrap them in an `Fx` struct.
#[test]
fn every_function_hangs_off_a_struct() {
    let offenders = ShapeScan::offenders(|line, above| {
        ShapeScan::is_free_fn(line) && !above.starts_with("#[test]") && !above.starts_with("#[bench]")
    });

    assert!(
        offenders.is_empty(),
        "no free functions: every fn hangs off a struct that names what it is. \
         A unit struct is a fine namespace. Only `#[test]` fns may be free.\n{}",
        offenders.join("\n")
    );
}

/// A `crate::a::b::Type` written at a use site is a `use` that was never added.
///
/// Import the name, then use it bare — in signatures, in struct fields and at
/// call sites alike. Two things are not violations and the scanner strips
/// both: `pub(in crate::a::b)`, which is a visibility scope Rust gives no
/// other spelling for, and a path inside a string literal.
///
/// If the name is only wanted by `#[cfg(test)]` code, put the `use` inside the
/// test module rather than at the top of the file — otherwise the lib build
/// reports an import it cannot see used.
#[test]
fn every_type_is_imported_rather_than_spelled_out() {
    let offenders = ShapeScan::offenders(|line, _| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
            return false;
        }
        ShapeScan::names_an_inline_item(line)
    });

    assert!(
        offenders.is_empty(),
        "no inline type paths: `crate::a::b::Type` at a use site is a missing \
         `use`. Import the name, then use it bare.\n{}",
        offenders.join("\n")
    );
}
