//! Announces AI compute report changes on `/ws/events`. The plugin keeps no handle on the
//! event channel, so a watcher polls its generation counter — cheap, and only a changed
//! counter produces an event.

use std::time::Duration;

use tokio::sync::broadcast;

use super::ServerEvent;
use crate::render::denoise::ai;

const POLL: Duration = Duration::from_millis(500);

pub fn spawn_ai_compute_watcher(events: broadcast::Sender<ServerEvent>) {
    tokio::spawn(async move {
        let mut last = ai::compute_generation();
        let mut ticker = tokio::time::interval(POLL);
        loop {
            ticker.tick().await;
            announce_if_changed(&mut last, ai::compute_generation(), &events);
        }
    });
}

fn announce_if_changed(last: &mut u64, now: u64, events: &broadcast::Sender<ServerEvent>) {
    if now == *last {
        return;
    }
    *last = now;
    let _ = events.send(ServerEvent::AiComputeChanged);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_changed_generation_is_announced() {
        let (events, mut received) = broadcast::channel(8);
        let mut last = 3;
        announce_if_changed(&mut last, 3, &events);
        assert!(received.try_recv().is_err(), "an unchanged report must stay quiet");

        announce_if_changed(&mut last, 4, &events);
        assert!(matches!(received.try_recv(), Ok(ServerEvent::AiComputeChanged)));
        assert_eq!(last, 4);

        announce_if_changed(&mut last, 4, &events);
        assert!(received.try_recv().is_err(), "one change, one event");
    }

    #[test]
    fn the_event_serialises_as_its_bare_type() {
        assert_eq!(ServerEvent::AiComputeChanged.to_json(), r#"{"type":"ai_compute_changed"}"#);
    }
}
