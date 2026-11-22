use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::{Attribute, Item, ItemFn, ItemMod, Meta};

const CRATE_ROOTS: &[&str] = &[
    "src/main.rs",
    "src/core/src/lib.rs",
    "src/database/src/lib.rs",
    "src/shared/src/lib.rs",
    "src/web/src/lib.rs",
    "src/match/builder/src/lib.rs",
    "src/match/src/lib.rs",
];

const BASELINE: &str = "build/function-structure-baseline.txt";

struct FunctionStructureCheck {
    root: PathBuf,
    violations: BTreeSet<String>,
    visited: BTreeSet<PathBuf>,
}

impl FunctionStructureCheck {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            violations: BTreeSet::new(),
            visited: BTreeSet::new(),
        }
    }

    fn scan(&mut self) {
        self.check_crate_coverage();
        self.check_module_names();
        for root in CRATE_ROOTS {
            self.visit_file(&self.root.join(root), "", false);
        }
    }

    fn check_module_names(&self) {
        for root in CRATE_ROOTS {
            if !root.ends_with("src/lib.rs") {
                continue;
            }
            let source_dir = self.root.join(root).parent().unwrap().to_path_buf();
            self.walk_module_names(&source_dir);
        }
    }

    fn walk_module_names(&self, dir: &Path) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if matches!(name, "target" | ".git" | "node_modules") {
                continue;
            }
            assert!(
                !matches!(name, "core" | "local" | "common"),
                "module directory {} uses a legacy name; use model/process/ticks/result instead",
                path.display()
            );
            self.walk_module_names(&path);
        }
    }

    fn check_crate_coverage(&self) {
        let expected: BTreeSet<PathBuf> = CRATE_ROOTS
            .iter()
            .map(|root| self.root.join(root))
            .filter_map(|root| self.crate_dir(&root))
            .collect();

        for manifest in self.manifests_under(&self.root.join("src")) {
            let dir = manifest.parent().unwrap().to_path_buf();
            assert!(
                expected.contains(&dir),
                "crate at {} has no entry in CRATE_ROOTS; the free-function gate would not cover it",
                dir.display()
            );
        }
    }

    fn crate_dir(&self, root: &Path) -> Option<PathBuf> {
        if root == self.root.join("src/main.rs") {
            return Some(self.root.clone());
        }
        let src = root.parent()?;
        let dir = src.parent()?;
        dir.join("Cargo.toml").exists().then(|| dir.to_path_buf())
    }

    fn manifests_under(&self, dir: &Path) -> Vec<PathBuf> {
        let mut manifests = Vec::new();
        self.collect_manifests(dir, &mut manifests);
        manifests
    }

    fn collect_manifests(&self, dir: &Path, manifests: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("target" | ".git" | "node_modules")
                ) {
                    continue;
                }
                self.collect_manifests(&path, manifests);
            } else if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
                manifests.push(path);
            }
        }
    }

    fn visit_file(&mut self, path: &Path, module_path: &str, parent_is_test: bool) {
        if self.skip_file(path) {
            return;
        }
        let absolute = self.absolute(path);
        if !self.visited.insert(absolute.clone()) {
            return;
        }
        let source = fs::read_to_string(&absolute)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", absolute.display()));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("cannot parse {}: {error}", absolute.display()));
        if file_is_test(&file.attrs) || parent_is_test {
            return;
        }
        let (file_dir, module_base) = module_dirs(&absolute);
        self.visit_items(
            &file.items,
            &absolute,
            &file_dir,
            &module_base,
            module_path,
            false,
        );
    }

    fn skip_file(&self, path: &Path) -> bool {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        if relative.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("tests" | "examples" | "benches" | ".dev")
            )
        }) {
            return true;
        }
        relative.file_name().and_then(|name| name.to_str()) == Some("build.rs")
            && relative
                .parent()
                .is_some_and(|parent| self.absolute(parent).join("Cargo.toml").exists())
    }

    fn visit_items(
        &mut self,
        items: &[Item],
        file: &Path,
        file_dir: &Path,
        module_base: &Path,
        module_path: &str,
        parent_is_test: bool,
    ) {
        for item in items {
            match item {
                Item::Fn(function) => {
                    if parent_is_test || is_test_annotated(&function.attrs) {
                        continue;
                    }
                    if !self.is_entry_point(file, function) {
                        let name = function.sig.ident.to_string();
                        let path = self.display_path(file);
                        let key = if module_path.is_empty() {
                            format!("{path}::{name}")
                        } else {
                            format!("{path}::{module_path}::{name}")
                        };
                        self.violations.insert(key);
                    }
                }
                Item::Mod(module) => {
                    self.visit_module(module, file, file_dir, module_base, module_path);
                }
                _ => {}
            }
        }
    }

    fn visit_module(
        &mut self,
        module: &ItemMod,
        file: &Path,
        file_dir: &Path,
        module_base: &Path,
        module_path: &str,
    ) {
        if is_test_annotated(&module.attrs) {
            return;
        }
        let name = module.ident.to_string();
        let child_path = if module_path.is_empty() {
            name.clone()
        } else {
            format!("{module_path}::{name}")
        };
        match &module.content {
            Some((_, items)) => {
                let child_base = module_base.join(&name);
                self.visit_items(items, file, file_dir, &child_base, &child_path, false);
            }
            None => {
                if let Some(resolved) = resolve_module_file(module, file_dir, module_base, &name) {
                    self.visit_file(&resolved, &child_path, false);
                }
            }
        }
    }

    fn is_entry_point(&self, file: &Path, function: &ItemFn) -> bool {
        if file == self.absolute(&self.root.join("src/main.rs")) && function.sig.ident == "main" {
            return true;
        }
        if is_wasm_start(&function.attrs) {
            return true;
        }
        is_no_mangle_c(function)
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn display_path(&self, file: &Path) -> String {
        file.strip_prefix(&self.root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn baseline(&self) -> BTreeSet<String> {
        fs::read_to_string(self.root.join(BASELINE))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(ToOwned::to_owned)
            .collect()
    }

    fn report(&self) {
        let baseline = self.baseline();
        let added: Vec<_> = self.violations.difference(&baseline).collect();
        let stale: Vec<_> = baseline.difference(&self.violations).collect();
        if added.is_empty() && stale.is_empty() {
            return;
        }
        let mut message = String::from("production function structure is out of date");
        if !added.is_empty() {
            message.push_str("\n\nnew module-level functions (move them onto a type):");
            for entry in added {
                message.push_str(&format!("\n  {entry}"));
            }
        }
        if !stale.is_empty() {
            message.push_str(
                "\n\nbaseline entries that no longer exist (delete them from the baseline):",
            );
            for entry in stale {
                message.push_str(&format!("\n  {entry}"));
            }
        }
        panic!("{message}");
    }

    fn write_baseline(&self) {
        let path = self.root.join(BASELINE);
        let mut contents = self
            .violations
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        contents.push('\n');
        fs::write(&path, contents)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
    }

    fn scan_source(&mut self, source: &str) {
        let file = syn::parse_file(source).expect("fixture must parse");
        self.visit_items(
            &file.items,
            Path::new("fixture.rs"),
            Path::new("."),
            Path::new("."),
            "",
            false,
        );
    }
}

fn module_dirs(file: &Path) -> (PathBuf, PathBuf) {
    let dir = file.parent().unwrap().to_path_buf();
    let stem = file.file_stem().and_then(|stem| stem.to_str());
    let base = match stem {
        Some("lib" | "main" | "mod") => dir.clone(),
        Some(stem) => dir.join(stem),
        None => dir.clone(),
    };
    (dir, base)
}

fn resolve_module_file(
    module: &ItemMod,
    file_dir: &Path,
    module_base: &Path,
    name: &str,
) -> Option<PathBuf> {
    if let Some(path) = path_attribute(module) {
        let resolved = file_dir.join(path);
        return resolved.exists().then_some(resolved);
    }
    let flat = module_base.join(format!("{name}.rs"));
    if flat.exists() {
        return Some(flat);
    }
    let nested = module_base.join(name).join("mod.rs");
    nested.exists().then_some(nested)
}

fn path_attribute(module: &ItemMod) -> Option<String> {
    module.attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("path") {
            return None;
        }
        let Meta::NameValue(value) = &attr.meta else {
            return None;
        };
        let syn::Expr::Lit(expr) = &value.value else {
            return None;
        };
        let syn::Lit::Str(value) = &expr.lit else {
            return None;
        };
        Some(value.value())
    })
}

fn is_test_annotated(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let Meta::List(list) = &attr.meta else {
            return false;
        };
        list.tokens
            .to_string()
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .any(|token| token == "test")
    })
}

fn file_is_test(attrs: &[Attribute]) -> bool {
    is_test_annotated(attrs)
}

fn is_wasm_start(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("wasm_bindgen")
            && attr
                .meta
                .require_list()
                .map(|list| {
                    list.tokens
                        .to_string()
                        .split(|character: char| !character.is_alphanumeric() && character != '_')
                        .any(|token| token == "start")
                })
                .unwrap_or(false)
    })
}

fn is_no_mangle_c(function: &ItemFn) -> bool {
    let no_mangle = function.attrs.iter().any(|attr| {
        attr.path().is_ident("no_mangle")
            || (attr.path().is_ident("unsafe")
                && attr
                    .meta
                    .require_list()
                    .map(|list| list.tokens.to_string().contains("no_mangle"))
                    .unwrap_or(false))
    });
    let c_abi = function
        .sig
        .abi
        .as_ref()
        .is_some_and(|abi| abi.name.as_ref().is_some_and(|name| name.value() == "C"));
    no_mangle && c_abi
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

#[test]
fn production_code_has_no_module_level_functions() {
    let mut check = FunctionStructureCheck::new(repo_root());
    check.scan();
    check.report();
}

#[test]
fn scanner_accepts_methods_and_test_modules() {
    let mut check = FunctionStructureCheck::new(PathBuf::new());
    check.scan_source(
        r#"
        struct Helper;

        impl Helper {
            fn method(&self) {}
        }

        #[cfg(test)]
        mod tests {
            fn test_helper() {}
        }
        "#,
    );
    assert!(check.violations.is_empty());
}

#[test]
fn scanner_rejects_module_level_function() {
    let mut check = FunctionStructureCheck::new(PathBuf::new());
    check.scan_source(
        r#"
        mod helpers {
            fn stray() {}
        }
        "#,
    );
    assert_eq!(
        check.violations.iter().next().map(String::as_str),
        Some("fixture.rs::helpers::stray")
    );
}

#[test]
#[ignore = "regenerates build/function-structure-baseline.txt"]
fn regenerate_baseline() {
    let mut check = FunctionStructureCheck::new(repo_root());
    check.scan();
    check.write_baseline();
}
