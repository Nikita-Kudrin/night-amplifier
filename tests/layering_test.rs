//! Layers, outermost first: `server` (HTTP adapter) → `session` (application) → the
//! domain (image pipeline, cameras, plugin traits). Pro links the domain, and every
//! server refactor used to break Pro through it. Only `server` and the composition root
//! (`app.rs`) may name `crate::server`; only they and `session` may name
//! `crate::session`. Test code is exempt. A Cargo workspace would let the compiler say
//! this instead.

use std::path::{Path, PathBuf};

/// Modules on the server's side of the boundary.
const ADAPTERS: &[&str] = &["server", "app.rs"];

/// Modules allowed to depend on the application layer.
const APPLICATION: &[&str] = &["server", "app.rs", "session"];

fn is_test_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || path.components().any(|c| c.as_os_str() == "tests")
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// The file up to its first `#[cfg(test)]` — tests sit at the bottom by convention.
fn production_code(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    text.split("#[cfg(test)]").next().unwrap_or("").to_string()
}

/// Production files outside `allowed` that name `module`.
fn offenders(module: &str, allowed: &[&str]) -> Vec<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);

    let mut offenders: Vec<String> = files
        .iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&src).unwrap();
            let top = relative.components().next()?.as_os_str().to_str()?;
            if allowed.contains(&top) || is_test_file(relative) {
                return None;
            }
            let code = production_code(path);
            let names = code.contains(&format!("crate::{module}"))
                || code.contains(&format!("super::{module}"));
            names.then(|| relative.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    offenders.sort();
    offenders
}

#[test]
fn only_the_server_and_the_composition_root_depend_on_the_server() {
    let offenders = offenders("server", ADAPTERS);
    assert!(
        offenders.is_empty(),
        "{offenders:?} name `crate::server` — move the type they need into the domain or the session"
    );
}

#[test]
fn the_domain_does_not_depend_on_the_application_layer() {
    let offenders = offenders("session", APPLICATION);
    assert!(
        offenders.is_empty(),
        "{offenders:?} name `crate::session` — move the type they need into the domain instead"
    );
}
