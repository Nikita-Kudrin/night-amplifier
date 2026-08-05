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
            println!("cargo:warning=Failed to create manual/.vitepress/dist directory: {}", e);
        } else {
            let _ = fs::write("manual/.vitepress/dist/.keep", "");
        }
    }
}
