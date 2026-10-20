//! AGENTS.md's comment budget: a comment block states its non-obvious "why" in at most
//! `ITEM_LIMIT` lines, `MODULE_LIMIT` for `//!` module docs. Blank lines, code fences, table
//! rows, rules and one-line list items don't count: they are structure, not prose. Pro runs
//! this file over its own sources through `#[path]`, so it names nothing of this crate.

use std::path::{Path, PathBuf};

const ITEM_LIMIT: usize = 10;
const MODULE_LIMIT: usize = 15;
/// Source roots under the crate being checked; a missing one is skipped.
const ROOTS: [&str; 5] = ["src", "crates", "tests", "benches", "build.rs"];
/// bindgen's header: generated code is not ours to trim.
const GENERATED: &str = "/* automatically generated";

#[test]
fn comment_blocks_stay_within_budget() {
    // At run time first: nextest runs prebuilt archives from another checkout.
    let root = std::env::var_os("CARGO_MANIFEST_DIR").map_or(PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let root = root.as_path();
    let files: Vec<PathBuf> = ROOTS.iter().flat_map(|r| rust_files(&root.join(r))).collect();
    assert!(!files.is_empty(), "no Rust sources under {}", root.display());
    let mut over = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        if text.starts_with(GENERATED) {
            continue;
        }
        let path = file.strip_prefix(root).unwrap().display().to_string();
        over.extend(over_budget(&text).into_iter().map(|b| {
            format!("{path}:{}: {} lines (limit {})", b.line, b.lines, b.limit)
        }));
    }
    assert!(
        over.is_empty(),
        "comments over AGENTS.md's budget: keep the non-obvious why, or split the point\n{}",
        over.join("\n")
    );
}

#[test]
fn structure_is_free_and_prose_is_counted() {
    let prose = "/// line\n".repeat(ITEM_LIMIT + 1) + "fn f() {}\n";
    assert_eq!(over_budget(&prose).len(), 1);

    let mut structured = "/// line\n".repeat(ITEM_LIMIT);
    structured += "///\n/// | a | b |\n/// - one fact\n/// ```\n/// let x = 1;\n/// ```\n// ---\nfn f() {}\n";
    assert!(over_budget(&structured).is_empty());

    let module = "//! line\n".repeat(MODULE_LIMIT) + &"/// line\n".repeat(ITEM_LIMIT);
    assert!(over_budget(&module).is_empty(), "a module doc and the item doc after it are two blocks");
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let root = std::env::temp_dir().join(format!("comment-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "fn f() {}\n").unwrap();
    std::os::unix::fs::symlink(&root, root.join("src/loop")).unwrap();
    std::os::unix::fs::symlink(root.join("src/lib.rs"), root.join("src/linked.rs")).unwrap();

    let files = rust_files(&root.join("src"));
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(files, vec![root.join("src/lib.rs")]);
}

/// The `.rs` files under `path`, never through a symlink: Pro's fixture links lead into
/// Community's tree, which would be judged as Pro's, and a link to an ancestor never ends.
fn rust_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.map(Result::unwrap) {
        let (kind, path) = (entry.file_type().unwrap(), entry.path());
        if kind.is_dir() && path.file_name().is_some_and(|n| n != "target") {
            files.extend(rust_files(&path));
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

struct Block {
    line: usize,
    lines: usize,
    limit: usize,
}

/// Comment blocks over their limit. A block is a run of lines with the same marker
/// (`//`, `///` or `//!`), so a module doc and the item doc after it are two.
fn over_budget(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut open: Option<(&str, Block)> = None;
    let mut fence = false;
    for (i, line) in lines.iter().enumerate() {
        let parsed = comment(line);
        if open.as_ref().map(|(m, _)| *m) != parsed.map(|(m, _)| m) {
            found.extend(open.take().map(|(_, b)| b).filter(|b| b.lines > b.limit));
            fence = false;
        }
        let Some((marker, body)) = parsed else {
            continue;
        };
        let limit = if marker == "//!" { MODULE_LIMIT } else { ITEM_LIMIT };
        let (_, block) = open.get_or_insert((marker, Block { line: i + 1, lines: 0, limit }));
        if body.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        let next = lines.get(i + 1).and_then(|l| comment(l)).filter(|(m, _)| *m == marker);
        if !fence && !is_structure(body, next.map(|(_, b)| b)) {
            block.lines += 1;
        }
    }
    found.extend(open.map(|(_, b)| b).filter(|b| b.lines > b.limit));
    found
}

/// The marker and the text after it, for a line that is only a comment.
fn comment(line: &str) -> Option<(&'static str, &str)> {
    let line = line.trim_start();
    ["//!", "///", "//"]
        .into_iter()
        .find_map(|marker| line.strip_prefix(marker).map(|body| (marker, body)))
}

/// A line that is structure rather than prose: blank, a table row, a rule, or a list
/// item the next line does not continue.
fn is_structure(body: &str, next: Option<&str>) -> bool {
    let text = body.trim();
    if text.is_empty() || text.starts_with('|') || text.chars().all(|c| "-=*─#/".contains(c)) {
        return true;
    }
    let is_item = |t: &str| {
        let t = t.trim_start();
        t.starts_with("- ") || t.starts_with("* ") || t.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok())
    };
    let indent = |t: &str| t.len() - t.trim_start().len();
    let continued = next.is_some_and(|n| !n.trim().is_empty() && !is_item(n) && indent(n) > indent(body));
    is_item(body) && !continued
}
