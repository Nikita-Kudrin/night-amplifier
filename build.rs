use std::fs;

fn main() {
    // Ensure the web/dist directory exists so rust-embed doesn't fail compilation
    // for users or CI pipelines that haven't built the frontend yet.
    if !std::path::Path::new("web/dist").exists() {
        if let Err(e) = fs::create_dir_all("web/dist") {
            println!("cargo:warning=Failed to create web/dist directory: {}", e);
        } else {
            // Create a dummy file so rust-embed has something to embed
            // Some versions of rust-embed fail if the directory is completely empty
            let _ = fs::write("web/dist/.keep", "");
        }
    }
}
