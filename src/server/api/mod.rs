//! REST API router and handler re-exports
//!
//! This module organizes API handlers into resource-based submodules
//! and provides a unified router assembly.

pub mod cameras;
pub mod capabilities;
pub mod capture;
pub mod install;
pub mod push_to;
pub mod settings;
pub mod simulator;
pub mod about;
pub mod indi;

// Re-export all handlers to maintain backward compatibility for existing router definitions
pub use cameras::*;
pub use capabilities::*;
pub use capture::*;
pub use install::*;
pub use push_to::*;
pub use settings::*;
pub use simulator::*;
pub use about::*;
pub use indi::*;

use crate::server::state::AppState;
use axum::body::Body;
use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;

/// Assembles the full API router
pub fn create_router() -> Router<Arc<AppState>> {
    Router::new()
        // About & License
        .route("/about/license", get(about::get_license).post(about::update_license))
        .route("/about/software-licenses", get(about::get_software_licenses))
        // Capabilities
        .route("/capabilities", get(capabilities::get_capabilities))
        // Capture
        .route("/capture/start", post(capture::start_capture))
        .route("/capture/stop", post(capture::stop_capture))
        .route("/capture/status", get(capture::get_capture_status))
        // Settings
        .route(
            "/settings",
            get(settings::get_settings).post(settings::update_settings),
        )
        .route(
            "/settings/stacking-types",
            get(settings::get_stacking_types),
        )
        // Cameras
        .route("/cameras", get(cameras::list_cameras))
        .route("/cameras/{camera_id}", get(cameras::get_camera_info))
        .route(
            "/cameras/{camera_id}/connect",
            post(cameras::connect_camera),
        )
        .route(
            "/cameras/{camera_id}/disconnect",
            post(cameras::disconnect_camera),
        )
        // INDI
        .route("/indi/test", post(indi::test_connection))
        // Simulator
        .route("/simulator/configure", post(simulator::configure_simulator))
        .route("/simulator/config", get(simulator::get_simulator_config))
        .route("/simulator/{index}", delete(simulator::remove_simulator))
        // Push-To
        .route("/push-to/status", get(push_to::get_push_to_status))
        .route(
            "/push-to/target",
            post(push_to::set_push_to_target).delete(push_to::clear_push_to_target),
        )
        .route("/push-to/direction", get(push_to::get_push_to_direction))
        .route("/push-to/cancel", post(push_to::cancel_push_to_solve))
        .route("/push-to/catalog/search", get(push_to::search_catalog))
        .route(
            "/push-to/catalog/messier",
            get(push_to::get_messier_catalog),
        )
        .route("/push-to/catalog/ngc", get(push_to::get_ngc_catalog))
        .route("/push-to/catalog/ic", get(push_to::get_ic_catalog))
        .route("/push-to/config", post(push_to::update_push_to_config))
        // Installation
        .route("/astap/status", get(install::get_astap_status))
        .route("/astap/databases", get(install::get_astap_databases))
        .route("/astap/install", post(install::install_astap))
        .route("/catalog/status", get(install::get_catalog_status))
        .route("/catalog/install", post(install::install_catalog))
}
