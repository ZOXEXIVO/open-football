//! Source-level guard: a player's attributes change only through his own
//! development.
//!
//! `Player::skills` is a public field read in hundreds of places, the match
//! engine's hot path among them, so it stays public and the writes are
//! asserted here instead. A write outside the owner takes one line to add
//! and a full read of the tree to notice.
//!
//! Allowed to assign an attribute:
//! - `club/player/development/`, the owner;
//! - `club/player/generators/` and the player builder, which create players;
//! - `club/player/ability/`, which fits a profile to an ability;
//! - the match engine (`match/`), which works on its own match-day copies;
//! - match readiness, a condition rather than an attribute, anywhere.

use std::fs;
use std::path::{Path, PathBuf};

struct AttributeWrites;

impl AttributeWrites {
    const FAMILIES: [&'static str; 4] = ["technical", "mental", "physical", "goalkeeping"];
    const OWNERS: [&'static str; 5] = [
        "club/player/development/",
        "club/player/generators/",
        "club/player/core/builder.rs",
        "club/player/ability/",
        "match/",
    ];
    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
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
        let root = Self::root();
        path.strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn is_test_file(label: &str, test_modules: &[String]) -> bool {
        label.split('/').any(|part| part == "tests")
            || label.ends_with("/tests.rs")
            || label.ends_with("_tests.rs")
            || test_modules.iter().any(|module| {
                label == format!("{module}.rs") || label.starts_with(&format!("{module}/"))
            })
    }

    /// Modules declared `#[cfg(test)] mod name;`, as the path their file
    /// lives at without its extension.
    fn test_modules(files: &[PathBuf]) -> Vec<String> {
        let mut out = Vec::new();
        for path in files {
            let label = Self::label(path);
            let Ok(src) = fs::read_to_string(path) else {
                continue;
            };
            let dir = if label.ends_with("mod.rs") || label == "lib.rs" {
                label
                    .rsplit_once('/')
                    .map_or(String::new(), |(d, _)| format!("{d}/"))
            } else {
                format!("{}/", label.trim_end_matches(".rs"))
            };
            let lines: Vec<&str> = src.lines().map(str::trim).collect();
            for pair in lines.windows(2) {
                if pair[0] != "#[cfg(test)]" {
                    continue;
                }
                let declared = pair[1]
                    .trim_start_matches("pub(crate) ")
                    .trim_start_matches("pub ")
                    .strip_prefix("mod ")
                    .and_then(|rest| rest.strip_suffix(';'));
                if let Some(name) = declared {
                    out.push(format!("{dir}{name}"));
                }
            }
        }
        out
    }

    /// The source before its test tail, comments dropped, with a method
    /// chain continued on the next line folded into the line it continues.
    fn production_statements(src: &str) -> Vec<(usize, String)> {
        let lines: Vec<&str> = src.lines().collect();
        let mut out: Vec<(usize, String)> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]"
                && lines[index + 1..]
                    .iter()
                    .map(|l| l.trim())
                    .find(|l| !l.is_empty())
                    .is_some_and(|next| {
                        let next = next
                            .trim_start_matches("pub(crate) ")
                            .trim_start_matches("pub ");
                        next.starts_with("mod ") && next.ends_with('{')
                    })
            {
                break;
            }
            if trimmed.starts_with("//") || trimmed.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some((_, statement)) if trimmed.starts_with('.') => statement.push_str(trimmed),
                _ => out.push((index + 1, trimmed.to_string())),
            }
        }
        out
    }

    /// Whether `statement` assigns, compound-assigns, mutably borrows or
    /// rescales a player's attributes.
    fn writes(statement: &str) -> bool {
        let bytes = statement.as_bytes();
        let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let assigns = |rest: &str| {
            let rest = rest.trim_start();
            (rest.starts_with('=') && !rest.starts_with("=="))
                || ["+=", "-=", "*=", "/="]
                    .iter()
                    .any(|op| rest.starts_with(op))
        };
        for (at, _) in statement.match_indices(".skills") {
            let mut end = at + ".skills".len();
            if bytes.get(end).copied().is_some_and(is_word) {
                continue;
            }
            let receiver = statement[..at]
                .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || "_.[]".contains(c));
            let borrowed = receiver.trim_end().ends_with("&mut");
            let mut attribute = None;
            if let Some(rest) = statement[end..].strip_prefix('.') {
                if let Some(family) = Self::FAMILIES.iter().find(|f| {
                    rest.starts_with(*f)
                        && !rest.as_bytes().get(f.len()).copied().is_some_and(is_word)
                }) {
                    end += 1 + family.len();
                    if let Some(rest) = statement[end..].strip_prefix('.') {
                        let name: String = rest
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        end += 1 + name.len();
                        attribute = Some(name);
                    }
                } else {
                    let call: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if ["shift", "fit_to_ability"].contains(&call.as_str()) {
                        return true;
                    }
                    continue;
                }
            }
            if attribute.as_deref() == Some("match_readiness") {
                continue;
            }
            if borrowed || assigns(&statement[end..]) {
                return true;
            }
        }
        false
    }

    fn offenders() -> Vec<String> {
        let mut files = Vec::new();
        Self::walk(&Self::root(), &mut files);
        assert!(
            files.len() > 100,
            "the scanner found no sources: its path is wrong"
        );
        let test_modules = Self::test_modules(&files);
        let mut out = Vec::new();
        for path in files {
            let label = Self::label(&path);
            if Self::is_test_file(&label, &test_modules)
                || label.ends_with("layering.rs")
                || Self::OWNERS.iter().any(|owner| label.starts_with(owner))
            {
                continue;
            }
            let Ok(src) = fs::read_to_string(&path) else {
                continue;
            };
            for (line, statement) in Self::production_statements(&src) {
                if Self::writes(&statement) {
                    out.push(format!("{label}:{line}  {statement}"));
                }
            }
        }
        out
    }
}

#[test]
fn nothing_else_writes_an_attribute() {
    let offenders = AttributeWrites::offenders();
    assert!(
        offenders.is_empty(),
        "attributes are written outside the player's development; state the fact through an \
         `on_*` method on `Player` instead:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_scanner_reads_every_kind_of_write() {
    for write in [
        "player.skills.mental.decisions += 0.1;",
        "mentee.skills.mental.determination = det_new.clamp(0.0, 20.0);",
        "let s = &mut player.skills.physical;",
        "p.skills = PlayerSkills::flat_for_ability(150);",
        "player.skills.shift(1.0, 0.0);",
    ] {
        assert!(AttributeWrites::writes(write), "missed: {write}");
    }
    for read in [
        "let pace = player.skills.physical.pace;",
        "if p.skills.mental.leadership == 20.0 {",
        "player.skills.physical.match_readiness = 12.0;",
        "let s = &player.skills.physical;",
    ] {
        assert!(!AttributeWrites::writes(read), "flagged a read: {read}");
    }
}
