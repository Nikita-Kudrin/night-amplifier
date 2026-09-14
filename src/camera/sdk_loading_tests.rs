//! Vendor SDKs are loaded with `dlopen2`, never linked. rustc passes `-l` to the linker even for an
//! unused `#[link]` extern block, so one makes every build machine need the vendor library. CI builds
//! `--no-default-features`, so only this source scan catches it there.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn camera_sdks_are_never_link_time_dependencies() {
    let camera_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/camera");
    let mut sources = Vec::new();
    rust_sources(&camera_dir, &mut sources);
    assert!(
        sources.iter().any(|path| path.ends_with("qhy/sdk.rs")),
        "scan did not reach the vendor SDK loaders under {}",
        camera_dir.display()
    );

    // Split so this file does not match itself.
    let link_attribute = concat!("#[", "link(");
    let linked: Vec<_> = sources
        .iter()
        .filter(|path| fs::read_to_string(path).unwrap().contains(link_attribute))
        .collect();
    assert!(
        linked.is_empty(),
        "load vendor SDKs through dlopen2 in the provider's sdk.rs instead: {linked:?}"
    );
}
