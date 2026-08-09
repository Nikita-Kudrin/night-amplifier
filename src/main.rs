//! Night Amplifier CLI (Community Edition)
//!
//! Runs a web server for remote camera control and image streaming.

#[tokio::main]
async fn main() {
    night_amplifier::app::APP_VERSION.set(env!("CARGO_PKG_VERSION").to_string()).ok();
    night_amplifier::app::run(|| {}).await;
}
