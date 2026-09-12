use std::fs;

fn main() {
    // Ensure the web/dist directory exists so rust-embed doesn't fail compilation
    // for users or CI pipelines that haven't built the frontend yet.
    if !std::path::Path::new("web/dist").exists() {
        if let Err(e) = fs::create_dir_all("web/dist") {
            println!("cargo:warning=Failed to create web/dist directory: {}", e);
        } else {
            let _ = fs::write("web/dist/.keep", "");
        }
    }

    // Ensure the manual/.vitepress/dist directory exists for the same reason.
    if !std::path::Path::new("manual/.vitepress/dist").exists() {
        if let Err(e) = fs::create_dir_all("manual/.vitepress/dist") {
            println!(
                "cargo:warning=Failed to create manual/.vitepress/dist directory: {}",
                e
            );
        } else {
            let _ = fs::write("manual/.vitepress/dist/.keep", "");
        }
    }

    export_build_metadata();
}

/// Build facts for the startup system report (`system_info`). Each falls back rather than
/// failing the build: a source tarball has no `.git`, and not every machine has git.
///
/// No `cargo:rerun-if-changed`: emitting one stops Cargo rerunning this script on every
/// package change, which the directory guards above rely on. The cost is a stale commit id
/// when a `git commit` is not followed by any file change before the next build.
fn export_build_metadata() {
    let git = command_output("git", &["describe", "--always", "--dirty", "--abbrev=12"]);
    let rustc = std::env::var("RUSTC")
        .ok()
        .and_then(|rustc| command_output(&rustc, &["--version"]));
    let target_cpu = std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .ok()
        .and_then(|flags| target_cpu(&flags));
    let unknown = || "unknown".to_string();

    let exports = [
        ("NIGHT_AMPLIFIER_GIT_DESCRIBE", git.unwrap_or_else(unknown)),
        (
            "NIGHT_AMPLIFIER_RUSTC_VERSION",
            rustc.unwrap_or_else(unknown),
        ),
        (
            "NIGHT_AMPLIFIER_TARGET",
            std::env::var("TARGET").unwrap_or_else(|_| unknown()),
        ),
        (
            "NIGHT_AMPLIFIER_TARGET_CPU",
            target_cpu.unwrap_or_else(|| "default".to_string()),
        ),
    ];
    for (key, value) in exports {
        println!("cargo:rustc-env={key}={value}");
    }
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// The last `target-cpu` in the encoded rustflags (rustc applies the last one), in either the
/// `-C target-cpu=x` or `-Ctarget-cpu=x` spelling.
fn target_cpu(encoded_rustflags: &str) -> Option<String> {
    let mut flags = encoded_rustflags.split('\x1f');
    let mut cpu = None;
    while let Some(flag) = flags.next() {
        let codegen = match flag {
            "-C" => flags.next().unwrap_or_default(),
            _ => flag.strip_prefix("-C").unwrap_or_default(),
        };
        if let Some(value) = codegen.strip_prefix("target-cpu=") {
            cpu = Some(value.to_string());
        }
    }
    cpu
}
