//! Simulated camera configuration API handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;
use tracing::{info, warn};

use super::super::dto::{ApiResponse, ConfigureSimulatorRequest, SimulatorConfigResponse};
use super::super::state::AppState;
use super::super::util::count_image_files;
use crate::camera::{
    add_simulated_directory, get_simulated_directories, remove_simulated_directory,
};

/// POST /api/simulator/configure
///
/// Add a simulated camera directory (supports multiple cameras)
pub async fn configure_simulator(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfigureSimulatorRequest>,
) -> impl IntoResponse {
    use std::path::PathBuf;

    let path = PathBuf::from(&request.directory);

    match add_simulated_directory(path) {
        Ok(was_added) => {
            let dirs = get_simulated_directories();
            let total_files: usize = dirs.iter().filter_map(|d| count_image_files(d)).sum();

            let message = if was_added {
                info!(
                    directory = %request.directory,
                    camera_count = dirs.len(),
                    "Simulated camera added"
                );

                // Persist the updated list
                state.save_settings().await;

                "Simulated camera added"
            } else {
                info!(
                    directory = %request.directory,
                    "Simulated camera directory already exists"
                );
                "Camera with this directory already exists"
            };

            (
                StatusCode::OK,
                ApiResponse::ok(SimulatorConfigResponse {
                    configured: true,
                    directory: Some(request.directory),
                    file_count: Some(total_files),
                    camera_count: Some(dirs.len()),
                    was_added: Some(was_added),
                    message: Some(message.to_string()),
                }),
            )
        }
        Err(e) => {
            warn!(
                directory = %request.directory,
                error = %e,
                "Failed to add simulated camera"
            );
            (
                StatusCode::BAD_REQUEST,
                ApiResponse::err(format!("Failed to add simulator: {}", e)),
            )
        }
    }
}

/// GET /api/simulator/config
///
/// Get the current simulated camera configuration
pub async fn get_simulator_config() -> impl IntoResponse {
    let dirs = get_simulated_directories();
    let total_files: usize = dirs.iter().filter_map(|d| count_image_files(d)).sum();

    (
        StatusCode::OK,
        ApiResponse::ok(SimulatorConfigResponse {
            configured: !dirs.is_empty(),
            directory: dirs.first().map(|p| p.display().to_string()),
            file_count: if dirs.is_empty() {
                None
            } else {
                Some(total_files)
            },
            camera_count: Some(dirs.len()),
            was_added: None,
            message: None,
        }),
    )
}

/// DELETE /api/simulator/:index
///
/// Remove a simulated camera by index
pub async fn remove_simulator(
    State(state): State<Arc<AppState>>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    match remove_simulated_directory(index) {
        Ok(removed_path) => {
            info!(
                index = index,
                path = %removed_path.display(),
                "Simulated camera removed"
            );

            // Persist the updated list
            state.save_settings().await;

            let dirs = get_simulated_directories();
            let total_files: usize = dirs.iter().filter_map(|d| count_image_files(d)).sum();

            (
                StatusCode::OK,
                ApiResponse::ok(SimulatorConfigResponse {
                    configured: !dirs.is_empty(),
                    directory: dirs.first().map(|p| p.display().to_string()),
                    file_count: if dirs.is_empty() {
                        None
                    } else {
                        Some(total_files)
                    },
                    camera_count: Some(dirs.len()),
                    was_added: None,
                    message: Some(format!(
                        "Removed simulated camera: {}",
                        removed_path.display()
                    )),
                }),
            )
        }
        Err(e) => {
            warn!(
                index = index,
                error = %e,
                "Failed to remove simulated camera"
            );
            (
                StatusCode::BAD_REQUEST,
                ApiResponse::err(format!("Failed to remove simulator: {}", e)),
            )
        }
    }
}
