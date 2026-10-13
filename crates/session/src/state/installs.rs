//! Where the ASTAP and catalog installs stand. Both run inside the Push-To plugin, which
//! has no progress query, so each install's events are recorded as they pass through the
//! port it reports to: a page that missed `*_completed` (a lag, a reconnect, another tab)
//! is told the outcome by the sync snapshot instead of showing a frozen bar forever.

use std::sync::{Arc, Mutex, MutexGuard};

use night_amplifier_core::push_to::{PushToError, PushToEvent, PushToEvents, PushToResult};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::events::{ServerEvent, Topic};

/// What an install that let go of its port before finishing is reported as.
const UNFINISHED: &str = "The installation stopped before it finished";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    Astap,
    Catalog,
}

/// What an installer has done since the server started.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallStatus {
    #[default]
    Idle,
    /// `progress` is the last event the install sent, `None` before its first.
    Running { progress: Option<Box<ServerEvent>> },
    Completed,
    Failed { error: String },
}

pub struct Installs {
    astap: Mutex<Run>,
    catalog: Mutex<Run>,
    events: broadcast::Sender<ServerEvent>,
}

#[derive(Default)]
struct Run {
    status: InstallStatus,
    /// Counts installs begun, so the port of one that ended cannot speak for its successor.
    begun: u64,
}

impl Installs {
    pub fn new(events: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            astap: Mutex::default(),
            catalog: Mutex::default(),
            events,
        }
    }

    fn lock(&self, installer: Installer) -> MutexGuard<'_, Run> {
        let run = match installer {
            Installer::Astap => &self.astap,
            Installer::Catalog => &self.catalog,
        };
        run.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self, installer: Installer) -> InstallStatus {
        self.lock(installer).status.clone()
    }

    /// The installer's status, as the event that replaces a client's copy.
    pub fn event(&self, installer: Installer) -> ServerEvent {
        status_event(installer, self.status(installer))
    }

    /// Marks `installer` running and hands out the port its install reports to, or refuses
    /// while one runs: a second install downloaded into the same files, and failing the
    /// plugin's refusal of it failed the first.
    ///
    /// The install is over when the plugin drops the port: ASTAP announces each component's
    /// completion, never the whole install's, so letting go after a completed component is
    /// what completes it. Letting go mid-way fails it.
    pub fn begin(self: &Arc<Self>, installer: Installer) -> PushToResult<Arc<InstallEvents>> {
        let mut run = self.lock(installer);
        if matches!(run.status, InstallStatus::Running { .. }) {
            return Err(PushToError::InstallInProgress);
        }
        run.begun += 1;
        self.announce(installer, &mut run, InstallStatus::Running { progress: None });
        Ok(Arc::new(InstallEvents {
            installs: Arc::clone(self),
            installer,
            begun: run.begun,
        }))
    }

    /// Records the event under the lock it is sent from, so a snapshot read meanwhile is
    /// either older than the event or already includes it. An event from the port of an
    /// install that has since been followed by another of its kind is dropped.
    fn record(&self, (port, begun): (Installer, u64), event: ServerEvent) {
        let installer = match event.topic() {
            Some(Topic::AstapInstall) => Installer::Astap,
            Some(Topic::CatalogInstall) => Installer::Catalog,
            _ => {
                let _ = self.events.send(event);
                return;
            }
        };
        let mut run = self.lock(installer);
        if installer == port && run.begun != begun {
            return;
        }
        run.status = match &event {
            ServerEvent::AstapInstallFailed { error, .. } | ServerEvent::CatalogInstallFailed { error } => {
                InstallStatus::Failed { error: error.clone() }
            }
            ServerEvent::CatalogInstallCompleted { .. } => InstallStatus::Completed,
            _ => InstallStatus::Running { progress: Some(Box::new(event.clone())) },
        };
        let _ = self.events.send(event);
    }

    fn end(&self, installer: Installer, begun: u64, error: Option<String>) {
        let mut run = self.lock(installer);
        let InstallStatus::Running { progress } = &run.status else { return };
        if run.begun != begun {
            return;
        }
        let finished = matches!(progress.as_deref(), Some(ServerEvent::AstapInstallCompleted { .. }));
        let status = match error {
            Some(error) => InstallStatus::Failed { error },
            None if finished => InstallStatus::Completed,
            None => InstallStatus::Failed { error: UNFINISHED.to_string() },
        };
        self.announce(installer, &mut run, status);
    }

    fn announce(&self, installer: Installer, run: &mut Run, status: InstallStatus) {
        run.status = status.clone();
        let _ = self.events.send(status_event(installer, status));
    }
}

fn status_event(installer: Installer, status: InstallStatus) -> ServerEvent {
    match installer {
        Installer::Astap => ServerEvent::AstapInstallStatus { status },
        Installer::Catalog => ServerEvent::CatalogInstallStatus { status },
    }
}

/// One install's `PushToEvents` port. See [`Installs::begin`].
pub struct InstallEvents {
    installs: Arc<Installs>,
    installer: Installer,
    begun: u64,
}

impl InstallEvents {
    /// The plugin refused to start the install.
    pub fn refused(&self, error: String) {
        self.installs.end(self.installer, self.begun, Some(error));
    }
}

impl PushToEvents for InstallEvents {
    fn emit(&self, event: PushToEvent) {
        self.installs.record((self.installer, self.begun), event.into());
    }
}

impl Drop for InstallEvents {
    fn drop(&mut self) {
        self.installs.end(self.installer, self.begun, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use night_amplifier_core::push_to::InstallStage;

    fn installs() -> (Arc<Installs>, broadcast::Receiver<ServerEvent>) {
        let (tx, rx) = broadcast::channel(32);
        (Arc::new(Installs::new(tx)), rx)
    }

    fn json(event: &ServerEvent) -> serde_json::Value {
        serde_json::from_str(&event.to_json()).unwrap()
    }

    fn drain(rx: &mut broadcast::Receiver<ServerEvent>) -> Vec<serde_json::Value> {
        std::iter::from_fn(|| rx.try_recv().ok()).map(|event| json(&event)).collect()
    }

    fn database_completed() -> PushToEvent {
        PushToEvent::astap_install_completed("D80 Database", Some(&InstallStage::DatabaseCompleted))
    }

    #[test]
    fn nothing_installed_since_the_start_is_idle() {
        let (installs, _rx) = installs();

        assert_eq!(
            json(&installs.event(Installer::Astap)),
            serde_json::json!({"type": "astap_install_status", "state": "idle"})
        );
        assert_eq!(
            json(&installs.event(Installer::Catalog)),
            serde_json::json!({"type": "catalog_install_status", "state": "idle"})
        );
    }

    #[test]
    fn a_running_install_is_restated_by_its_last_event() {
        let (installs, mut rx) = installs();
        let events = installs.begin(Installer::Astap).unwrap();
        assert_eq!(
            json(&installs.event(Installer::Astap)),
            serde_json::json!({"type": "astap_install_status", "state": "running", "progress": null})
        );

        events.emit(PushToEvent::astap_install_progress("D80 Database", 500, Some(2000), None));

        let status = json(&installs.event(Installer::Astap));
        assert_eq!(status["state"], "running");
        assert_eq!(status["progress"]["type"], "astap_install_progress");
        assert_eq!(status["progress"]["bytes_downloaded"], 500);
        let sent: Vec<_> = drain(&mut rx).into_iter().map(|e| e["type"].clone()).collect();
        assert_eq!(sent, ["astap_install_status", "astap_install_progress"]);
    }

    #[test]
    fn the_status_is_recorded_before_its_event_is_sent() {
        let (installs, mut rx) = installs();
        let events = installs.begin(Installer::Catalog).unwrap();
        let _ = rx.try_recv();

        events.emit(PushToEvent::CatalogInstallCompleted { object_count: 13957 });

        assert!(matches!(rx.try_recv(), Ok(ServerEvent::CatalogInstallCompleted { .. })));
        assert!(matches!(installs.status(Installer::Catalog), InstallStatus::Completed));
    }

    /// ASTAP says when each component is in, never when the whole install is: letting go
    /// of the port after the last one is the only "done" it gives.
    #[test]
    fn astap_completes_when_its_install_lets_go_after_a_completed_component() {
        let (installs, mut rx) = installs();
        let events = installs.begin(Installer::Astap).unwrap();
        events.emit(database_completed());
        assert!(matches!(installs.status(Installer::Astap), InstallStatus::Running { .. }));
        drain(&mut rx);

        drop(events);

        assert!(matches!(installs.status(Installer::Astap), InstallStatus::Completed));
        assert_eq!(drain(&mut rx), [serde_json::json!({"type": "astap_install_status", "state": "completed"})]);
    }

    #[test]
    fn an_install_that_lets_go_mid_way_failed() {
        let (installs, _rx) = installs();
        let events = installs.begin(Installer::Catalog).unwrap();
        events.emit(PushToEvent::catalog_install_progress("NGC.csv", 300, Some(1200)));

        drop(events);

        assert_eq!(
            json(&installs.event(Installer::Catalog)),
            serde_json::json!({"type": "catalog_install_status", "state": "failed", "error": UNFINISHED})
        );
    }

    #[test]
    fn a_failure_is_kept_as_the_outcome() {
        let (installs, mut rx) = installs();
        let events = installs.begin(Installer::Astap).unwrap();
        events.emit(PushToEvent::AstapInstallFailed { component: "ASTAP CLI".into(), error: "network".into() });
        drop(events);

        assert_eq!(
            json(&installs.event(Installer::Astap)),
            serde_json::json!({"type": "astap_install_status", "state": "failed", "error": "network"})
        );
        let sent: Vec<_> = drain(&mut rx).into_iter().map(|e| e["type"].clone()).collect();
        assert_eq!(sent, ["astap_install_status", "astap_install_failed"], "letting go after a failure says nothing more");
    }

    #[test]
    fn a_refused_install_fails_with_the_plugins_reason() {
        let (installs, _rx) = installs();
        let events = installs.begin(Installer::Astap).unwrap();

        events.refused("No valid database types specified".into());
        drop(events);

        assert!(matches!(
            installs.status(Installer::Astap),
            InstallStatus::Failed { error } if error == "No valid database types specified"
        ));
    }

    #[test]
    fn a_second_install_is_refused_while_one_runs() {
        let (installs, mut rx) = installs();
        let running = installs.begin(Installer::Astap).unwrap();
        drain(&mut rx);

        assert!(matches!(installs.begin(Installer::Astap), Err(PushToError::InstallInProgress)));
        assert!(drain(&mut rx).is_empty(), "a refused begin announces nothing");

        running.emit(database_completed());
        drop(running);
        assert!(matches!(installs.status(Installer::Astap), InstallStatus::Completed));
        assert!(installs.begin(Installer::Astap).is_ok(), "the next one may begin once it ended");
    }

    #[test]
    fn the_port_of_an_install_that_ended_cannot_speak_for_the_next() {
        let (installs, _rx) = installs();
        let refused = installs.begin(Installer::Catalog).unwrap();
        refused.refused("offline".into());
        let _next = installs.begin(Installer::Catalog).unwrap();

        refused.emit(PushToEvent::CatalogInstallFailed { error: "the old install".into() });
        drop(refused);

        assert!(
            matches!(installs.status(Installer::Catalog), InstallStatus::Running { progress: None }),
            "{:?}",
            installs.status(Installer::Catalog)
        );
    }

    #[test]
    fn the_two_installers_are_tracked_apart() {
        let (installs, _rx) = installs();
        let astap = installs.begin(Installer::Astap).unwrap();

        astap.emit(PushToEvent::CatalogInstallFailed { error: "disk full".into() });

        assert!(matches!(installs.status(Installer::Astap), InstallStatus::Running { progress: None }));
        assert!(matches!(installs.status(Installer::Catalog), InstallStatus::Failed { .. }));
    }
}
