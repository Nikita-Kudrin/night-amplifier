//! ASTAP and Catalog installation API handlers
//!
//! These endpoints delegate to the Push-To installer plugin if present,
//! otherwise they return an error indicating the feature requires the Pro version.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::future::Future;
use std::sync::Arc;

use super::super::dto::{ApiResponse, AstapInstallRequest, CatalogInstallRequest, MessageResponse};
use crate::server::error::HttpStatus;
use crate::session::state::{AppState, Installer};
use crate::push_to::{
    AstapStatusResponse, CatalogStatusResponse, DatabaseTypeResponse, PushToEvents, PushToResult,
};

/// Begin `installer`'s run and hand the plugin its port: 409 while one already runs, the
/// plugin's refusal recorded as the install's failure.
async fn start_install<F, Fut>(
    state: &AppState,
    installer: Installer,
    started: &str,
    install: F,
) -> (StatusCode, Json<ApiResponse<MessageResponse>>)
where
    F: FnOnce(Arc<dyn PushToEvents>) -> Fut,
    Fut: Future<Output = PushToResult<()>>,
{
    let events = match state.installs.begin(installer) {
        Ok(events) => events,
        Err(e) => return (e.status_code(), ApiResponse::err(e.to_string())),
    };
    match install(events.clone()).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            ApiResponse::ok(MessageResponse {
                message: started.to_string(),
                camera_id: None,
            }),
        ),
        Err(e) => {
            events.refused(e.to_string());
            (e.status_code(), ApiResponse::err(e.to_string()))
        }
    }
}

/// GET /api/astap/status
///
/// Get ASTAP installation status
pub async fn get_astap_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(plugin) = state.plugins.push_to_installer() {
        let status = plugin.get_astap_status().await;
        (StatusCode::OK, ApiResponse::ok(status))
    } else {
        (
            StatusCode::OK,
            ApiResponse::ok(AstapStatusResponse {
                binary_installed: false,
                binary_path: None,
                database_installed: false,
                database_path: None,
                database_type: None,
                installed_databases: vec![],
                ready: false,
            }),
        )
    }
}

/// GET /api/astap/databases
///
/// Get available database types for installation (in display order)
pub async fn get_astap_databases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(plugin) = state.plugins.push_to_installer() {
        let databases = plugin.get_astap_databases().await;
        (StatusCode::OK, ApiResponse::ok(databases))
    } else {
        (
            StatusCode::OK,
            ApiResponse::ok(Vec::<DatabaseTypeResponse>::new()),
        )
    }
}

/// POST /api/astap/install
///
/// Start ASTAP installation (binary and database)
pub async fn install_astap(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AstapInstallRequest>,
) -> impl IntoResponse {
    let database_types = request.into_database_types();
    if let Some(plugin) = state.plugins.push_to_installer() {
        start_install(&state, Installer::Astap, "Installation started.", |events| {
            plugin.install_astap(&database_types, events)
        })
        .await
    } else {
        (
            StatusCode::FORBIDDEN,
            ApiResponse::err("ASTAP installation requires Night Amplifier Pro."),
        )
    }
}

/// GET /api/catalog/status
///
/// Get OpenNGC catalog installation status
pub async fn get_catalog_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(plugin) = state.plugins.push_to_installer() {
        let status = plugin.get_catalog_status().await;
        (StatusCode::OK, ApiResponse::ok(status))
    } else {
        (
            StatusCode::OK,
            ApiResponse::ok(CatalogStatusResponse {
                installed: false,
                catalog_path: None,
                ngc_file_exists: false,
                addendum_file_exists: false,
                hyg_file_exists: false,
                object_count: None,
            }),
        )
    }
}

/// POST /api/catalog/install
///
/// Start OpenNGC catalog installation (downloads NGC.csv and addendum.csv)
pub async fn install_catalog(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CatalogInstallRequest>,
) -> impl IntoResponse {
    if let Some(plugin) = state.plugins.push_to_installer() {
        start_install(&state, Installer::Catalog, "Catalog installation started.", |events| {
            plugin.install_catalog(request.include_stars, events)
        })
        .await
    } else {
        (
            StatusCode::FORBIDDEN,
            ApiResponse::err("Catalog installation requires Night Amplifier Pro."),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::Plugins;
    use crate::push_to::{InstallStage, PushToError, PushToEvent, PushToInstallerPlugin};
    use crate::session::state::InstallStatus;
    use async_trait::async_trait;
    use axum::response::Response;
    use std::sync::Mutex;

    /// Keeps the port each install is handed, as the real plugin's install task does, or
    /// refuses the install.
    #[derive(Default)]
    struct FakeInstaller {
        ports: Mutex<Vec<Arc<dyn PushToEvents>>>,
        refusal: Option<&'static str>,
    }

    impl FakeInstaller {
        fn start(&self, events: Arc<dyn PushToEvents>) -> PushToResult<()> {
            if let Some(reason) = self.refusal {
                return Err(PushToError::InvalidRequest(reason.to_string()));
            }
            self.ports.lock().unwrap().push(events);
            Ok(())
        }

        fn port(&self) -> Arc<dyn PushToEvents> {
            self.ports.lock().unwrap().pop().expect("no install was started")
        }
    }

    #[async_trait]
    impl PushToInstallerPlugin for FakeInstaller {
        async fn get_astap_status(&self) -> AstapStatusResponse {
            unreachable!("not exercised by these tests")
        }

        async fn get_astap_databases(&self) -> Vec<DatabaseTypeResponse> {
            unreachable!("not exercised by these tests")
        }

        async fn install_astap(&self, _database_types: &[String], events: Arc<dyn PushToEvents>) -> PushToResult<()> {
            self.start(events)
        }

        async fn get_catalog_status(&self) -> CatalogStatusResponse {
            unreachable!("not exercised by these tests")
        }

        async fn install_catalog(&self, _include_stars: bool, events: Arc<dyn PushToEvents>) -> PushToResult<()> {
            self.start(events)
        }
    }

    fn state_with(installer: &Arc<FakeInstaller>) -> Arc<AppState> {
        let (mut state, _disk_writer) = AppState::new_for_testing();
        state.plugins = Plugins::none().with_push_to_installer(installer.clone()).always_licensed();
        Arc::new(state)
    }

    async fn post_astap(state: &Arc<AppState>) -> Response {
        let request = AstapInstallRequest { database_types: vec!["D80".into()], database_type: None };
        install_astap(State(Arc::clone(state)), Json(request)).await.into_response()
    }

    #[tokio::test]
    async fn an_accepted_install_runs_until_the_plugin_lets_go_of_its_port() {
        let installer = Arc::new(FakeInstaller::default());
        let state = state_with(&installer);

        assert_eq!(post_astap(&state).await.status(), StatusCode::ACCEPTED);
        assert!(matches!(state.installs.status(Installer::Astap), InstallStatus::Running { progress: None }));

        let port = installer.port();
        port.emit(PushToEvent::astap_install_completed("D80 Database", Some(&InstallStage::DatabaseCompleted)));
        assert!(matches!(state.installs.status(Installer::Astap), InstallStatus::Running { progress: Some(_) }));
        drop(port);

        assert!(matches!(state.installs.status(Installer::Astap), InstallStatus::Completed));
    }

    #[tokio::test]
    async fn a_refused_install_fails_with_the_plugins_reason() {
        let installer = Arc::new(FakeInstaller { refusal: Some("No valid database types specified"), ..Default::default() });
        let state = state_with(&installer);
        let request = CatalogInstallRequest { include_stars: false };

        let response = install_catalog(State(Arc::clone(&state)), Json(request)).await.into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "the request named nothing installable");
        assert!(matches!(
            state.installs.status(Installer::Catalog),
            InstallStatus::Failed { error } if error == "No valid database types specified"
        ));
    }

    /// Another tab's click while an install runs is refused, and the running one carries
    /// on to its own end: the plugin's refusal of a second one once failed the first.
    #[tokio::test]
    async fn a_second_install_is_refused_while_one_runs() {
        let installer = Arc::new(FakeInstaller::default());
        let state = state_with(&installer);
        assert_eq!(post_astap(&state).await.status(), StatusCode::ACCEPTED);

        assert_eq!(post_astap(&state).await.status(), StatusCode::CONFLICT);

        let port = installer.port();
        assert!(installer.ports.lock().unwrap().is_empty(), "the plugin was not asked twice");
        port.emit(PushToEvent::astap_install_completed("D80 Database", Some(&InstallStage::DatabaseCompleted)));
        drop(port);
        assert!(matches!(state.installs.status(Installer::Astap), InstallStatus::Completed));
    }
}
