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

/// Discovery reaches these on any machine with their SDK installed, and a lazily bound SDK
/// with an unresolvable symbol aborts the whole server at its first call (SVBony, libusb).
#[test]
fn newly_offered_sdks_bind_eagerly() {
    let camera_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/camera");
    for provider in ["qhy", "touptek", "svbony"] {
        let loader = fs::read_to_string(camera_dir.join(provider).join("sdk.rs")).unwrap();
        assert!(
            loader.contains("sdk_library::load_eagerly"),
            "{provider}/sdk.rs must open its library through sdk_library::load_eagerly"
        );
    }
}

/// `c_char` is `u8` on Linux ARM, so an `i8` C-string buffer compiles everywhere but the Pi —
/// QHY and ToupTek never built for it until 2026-09-14. CI's Linux jobs cannot see that.
#[test]
fn c_strings_are_never_i8_buffers() {
    let camera_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/camera");
    let mut sources = Vec::new();
    rust_sources(&camera_dir, &mut sources);
    // Split so this file does not match itself.
    let patterns = [
        concat!("[0", "i8;"),
        concat!("&[", "i8]"),
        concat!("*const ", "i8"),
        concat!("*mut ", "i8"),
    ];
    let offending: Vec<String> = sources
        .iter()
        .flat_map(|path| {
            let text = fs::read_to_string(path).unwrap();
            patterns
                .iter()
                .filter(|pattern| text.contains(**pattern))
                .map(|pattern| format!("{}: {pattern}", path.display()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offending.is_empty(),
        "use c_char (ToupTek strings: TChar) instead: {offending:?}"
    );
}
