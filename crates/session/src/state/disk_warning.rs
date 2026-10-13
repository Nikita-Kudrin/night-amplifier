//! The slow-disk warning as clients are told it: raised while the raw-frame queue is backed
//! up, with its depth. Written by the storage task before its event goes out, so a client
//! that connects or fell behind is sent the warning, or its clearing — a lost clear once left
//! "Slow disk" up for the rest of the night.

use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::events::ServerEvent;

pub struct DiskWarning {
    queue_depth: Mutex<Option<usize>>,
    events: broadcast::Sender<ServerEvent>,
}

fn event(queue_depth: Option<usize>) -> ServerEvent {
    match queue_depth {
        Some(depth) => ServerEvent::disk_writer_warning(depth),
        None => ServerEvent::DiskWriterWarningCleared,
    }
}

impl DiskWarning {
    pub fn new(events: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            queue_depth: Mutex::new(None),
            events,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<usize>> {
        self.queue_depth.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The queue depth last announced, while the warning is raised.
    pub fn queue_depth(&self) -> Option<usize> {
        *self.lock()
    }

    pub fn is_raised(&self) -> bool {
        self.queue_depth().is_some()
    }

    /// The warning or its clearing, as the event that replaces a client's copy.
    pub fn event(&self) -> ServerEvent {
        event(self.queue_depth())
    }

    /// Announce the queue backed up to `queue_depth`. Sent from under the lock, so a raise and
    /// a clear reach clients in the order they were made.
    pub fn raise(&self, queue_depth: usize) {
        let mut current = self.lock();
        *current = Some(queue_depth);
        let _ = self.events.send(event(*current));
    }

    /// Clear a raised warning. Returns whether one was raised; nothing is sent otherwise.
    pub fn clear(&self) -> bool {
        let mut current = self.lock();
        if current.take().is_none() {
            return false;
        }
        let _ = self.events.send(event(None));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warning_is_recorded_before_it_is_announced_and_cleared_once() {
        let (tx, mut rx) = broadcast::channel(8);
        let warning = DiskWarning::new(tx);
        assert!(matches!(warning.event(), ServerEvent::DiskWriterWarningCleared));

        warning.raise(12);
        assert!(matches!(rx.try_recv(), Ok(ServerEvent::DiskWriterWarning { queue_depth: 12 })));
        assert!(matches!(warning.event(), ServerEvent::DiskWriterWarning { queue_depth: 12 }));

        assert!(warning.clear());
        assert!(!warning.clear(), "already clear");
        assert!(matches!(rx.try_recv(), Ok(ServerEvent::DiskWriterWarningCleared)));
        assert!(rx.try_recv().is_err(), "a second clear announces nothing");
        assert_eq!(warning.queue_depth(), None);
    }
}
