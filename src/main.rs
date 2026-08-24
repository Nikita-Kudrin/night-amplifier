//! Night Amplifier CLI (Community Edition)
//!
//! Runs a web server for remote camera control and image streaming.

#[tokio::main]
async fn main() {
    let version = option_env!("NIGHT_AMPLIFIER_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    night_amplifier::app::APP_VERSION
        .set(version.to_string())
        .ok();
    night_amplifier::app::run(|| {}).await;
}
