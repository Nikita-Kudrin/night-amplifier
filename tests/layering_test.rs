//! The image pipeline, cameras and plugin traits must not depend on the HTTP server:
//! Pro links them, and every server refactor used to break Pro through them. Only
//! `server` itself and the composition root (`app.rs`) may name `crate::server`; test
//! code is exempt. A Cargo workspace would let the compiler say this instead.

use std::path::{Path, PathBuf};

/// Modules on the server's side of the boundary.
const ADAPTERS: &[&str] = &["server", "app.rs"];

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

#[test]
fn only_the_server_and_the_composition_root_depend_on_the_server() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);

    let mut offenders: Vec<String> = files
        .iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&src).unwrap();
            let top = relative.components().next()?.as_os_str().to_str()?;
            if ADAPTERS.contains(&top) || is_test_file(relative) {
                return None;
            }
            let code = production_code(path);
            let names_server = code.contains("crate::server") || code.contains("super::server");
            names_server.then(|| relative.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    offenders.sort();

    assert!(
        offenders.is_empty(),
        "{offenders:?} name `crate::server` — move the type they need into the domain instead"
    );
}
