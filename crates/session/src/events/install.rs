//! The Push-To plugin's events on the wire: each [`PushToEvent`] is one `/ws/events`
//! message, field for field.

use tokio::sync::broadcast;

use super::ServerEvent;
use std::sync::Arc;

use night_amplifier_core::push_to::{PushToEvent, PushToEvents};

impl From<PushToEvent> for ServerEvent {
    fn from(event: PushToEvent) -> Self {
        match event {
            PushToEvent::SolveStarted { target_name } => Self::PlateSolvingStarted { target_name },
            PushToEvent::SolveProgress { stage, attempt, total } => {
                Self::PlateSolvingProgress { stage, attempt, total }
            }
            PushToEvent::AstapInstallStarting { component } => Self::AstapInstallStarting { component },
            PushToEvent::AstapInstallProgress {
                component,
                bytes_downloaded,
                total_bytes,
                percent,
                stage,
                overall_percent,
            } => Self::AstapInstallProgress {
                component,
                bytes_downloaded,
                total_bytes,
                percent,
                stage,
                overall_percent,
            },
            PushToEvent::AstapInstallExtracting {
                component,
                progress,
                stage,
                overall_percent,
            } => Self::AstapInstallExtracting {
                component,
                progress,
                stage,
                overall_percent,
            },
            PushToEvent::AstapInstallCompleted {
                component,
                stage,
                overall_percent,
            } => Self::AstapInstallCompleted {
                component,
                stage,
                overall_percent,
            },
            PushToEvent::AstapInstallFailed { component, error } => {
                Self::AstapInstallFailed { component, error }
            }
            PushToEvent::CatalogInstallStarting => Self::CatalogInstallStarting,
            PushToEvent::CatalogInstallProgress {
                file_name,
                bytes_downloaded,
                total_bytes,
                percent,
            } => Self::CatalogInstallProgress {
                file_name,
                bytes_downloaded,
                total_bytes,
                percent,
            },
            PushToEvent::CatalogFileCompleted { file_name } => Self::CatalogFileCompleted { file_name },
            PushToEvent::CatalogInstallCompleted { object_count } => {
                Self::CatalogInstallCompleted { object_count }
            }
            PushToEvent::CatalogInstallFailed { error } => Self::CatalogInstallFailed { error },
        }
    }
}

/// The event bus as a plugin's `PushToEvents` port. Every `/ws/events` client hears the
/// plugin; a send with none connected is not an error.
pub fn push_to_events(sender: &broadcast::Sender<ServerEvent>) -> Arc<dyn PushToEvents> {
    Arc::new(Broadcast(sender.clone()))
}

/// A newtype only because the trait and `broadcast::Sender` both live in other crates.
struct Broadcast(broadcast::Sender<ServerEvent>);

impl PushToEvents for Broadcast {
    fn emit(&self, event: PushToEvent) {
        let _ = self.0.send(event.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use night_amplifier_core::push_to::InstallStage;

    /// Captured from the `ServerEvent` constructors these events replaced: the wire is
    /// the contract the frontend reads, so the mapping must reproduce it byte for byte.
    #[test]
    fn push_to_events_keep_their_wire_shape() {
        let cases = [
            (
                PushToEvent::SolveStarted { target_name: Some("M31".into()) },
                r#"{"type":"plate_solving_started","target_name":"M31"}"#,
            ),
            (
                PushToEvent::SolveStarted { target_name: None },
                r#"{"type":"plate_solving_started","target_name":null}"#,
            ),
            (
                PushToEvent::SolveProgress { stage: "Hinted FOV 1.2°".into(), attempt: 2, total: 4 },
                r#"{"type":"plate_solving_progress","stage":"Hinted FOV 1.2°","attempt":2,"total":4}"#,
            ),
            (
                PushToEvent::AstapInstallStarting { component: "ASTAP CLI".into() },
                r#"{"type":"astap_install_starting","component":"ASTAP CLI"}"#,
            ),
            (
                PushToEvent::astap_install_progress(
                    "D80 Database",
                    500,
                    Some(2000),
                    Some(&InstallStage::DownloadingDatabase),
                ),
                r#"{"type":"astap_install_progress","component":"D80 Database","bytes_downloaded":500,"total_bytes":2000,"percent":25.0,"stage":"Downloading Database","overall_percent":37.5}"#,
            ),
            (
                PushToEvent::astap_install_progress("D80 Database", 500, None, None),
                r#"{"type":"astap_install_progress","component":"D80 Database","bytes_downloaded":500,"total_bytes":null,"percent":null,"stage":null,"overall_percent":null}"#,
            ),
            (
                PushToEvent::astap_install_extracting(
                    "D80 Database",
                    40.0,
                    Some(&InstallStage::ExtractingDatabase),
                ),
                r#"{"type":"astap_install_extracting","component":"D80 Database","progress":40.0,"stage":"Extracting Database","overall_percent":94.0}"#,
            ),
            (
                PushToEvent::astap_install_completed("ASTAP CLI", Some(&InstallStage::CliCompleted)),
                r#"{"type":"astap_install_completed","component":"ASTAP CLI","stage":"ASTAP CLI Installed","overall_percent":20.0}"#,
            ),
            (
                PushToEvent::AstapInstallFailed { component: "ASTAP CLI".into(), error: "network".into() },
                r#"{"type":"astap_install_failed","component":"ASTAP CLI","error":"network"}"#,
            ),
            (
                PushToEvent::AstapInstallProgress {
                    component: "x".into(),
                    bytes_downloaded: 1,
                    total_bytes: Some(4),
                    percent: Some(25.0),
                    stage: Some("s".into()),
                    overall_percent: Some(3.5),
                },
                r#"{"type":"astap_install_progress","component":"x","bytes_downloaded":1,"total_bytes":4,"percent":25.0,"stage":"s","overall_percent":3.5}"#,
            ),
            (PushToEvent::CatalogInstallStarting, r#"{"type":"catalog_install_starting"}"#),
            (
                PushToEvent::catalog_install_progress("NGC.csv", 300, Some(1200)),
                r#"{"type":"catalog_install_progress","file_name":"NGC.csv","bytes_downloaded":300,"total_bytes":1200,"percent":25.0}"#,
            ),
            (
                PushToEvent::CatalogFileCompleted { file_name: "NGC.csv".into() },
                r#"{"type":"catalog_file_completed","file_name":"NGC.csv"}"#,
            ),
            (
                PushToEvent::CatalogInstallCompleted { object_count: 13957 },
                r#"{"type":"catalog_install_completed","object_count":13957}"#,
            ),
            (
                PushToEvent::CatalogInstallFailed { error: "disk full".into() },
                r#"{"type":"catalog_install_failed","error":"disk full"}"#,
            ),
        ];
        for (event, wire) in cases {
            let json = serde_json::to_string(&ServerEvent::from(event.clone())).unwrap();
            assert_eq!(json, wire, "{event:?}");
        }
    }

    #[test]
    fn the_broadcast_port_delivers_to_every_listener() {
        let (sender, mut first) = broadcast::channel(4);
        let mut second = sender.subscribe();
        push_to_events(&sender).emit(PushToEvent::CatalogInstallStarting);
        for receiver in [&mut first, &mut second] {
            assert!(matches!(receiver.try_recv(), Ok(ServerEvent::CatalogInstallStarting)));
        }
    }
}
