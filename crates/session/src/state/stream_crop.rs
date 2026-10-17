//! The crop the main stream's frames are cut to, for `/ws/events` clients: changes are
//! announced, and a client that connects or fell behind is sent the current one.
//!
//! Runtime state, set only by the render task as it publishes frames.

use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::events::{ServerEvent, StreamCrop, StreamFrameSize};

pub struct StreamCropView {
    current: Mutex<Announced>,
    events: broadcast::Sender<ServerEvent>,
}

/// The crop last published, and the live stream's frame size it was published with.
#[derive(Default)]
struct Announced {
    crop: Option<StreamCrop>,
    frame: Option<StreamFrameSize>,
}

impl Announced {
    fn event(&self) -> ServerEvent {
        ServerEvent::StackCropChanged { crop: self.crop, frame: self.frame }
    }
}

impl StreamCropView {
    pub fn new(events: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            current: Mutex::new(Announced::default()),
            events,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Announced> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self) -> Option<StreamCrop> {
        self.lock().crop
    }

    /// The current crop, as the event that replaces a client's copy.
    pub fn event(&self) -> ServerEvent {
        self.lock().event()
    }

    /// Records the crop of the frame about to be published and its live-stream size,
    /// announcing a crop change from under the lock so two changes reach clients in the order
    /// they were made. A size change alone (a resolution setting) is not announced.
    pub fn publish(&self, crop: Option<StreamCrop>, frame: StreamFrameSize) {
        let mut current = self.lock();
        current.frame = Some(frame);
        if current.crop == crop {
            return;
        }
        current.crop = crop;
        let _ = self.events.send(current.event());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crop(x: usize) -> StreamCrop {
        StreamCrop { x, y: 0, width: 100 - x, height: 80, source_width: 100, source_height: 80 }
    }

    #[test]
    fn only_a_change_is_announced() {
        let (tx, mut rx) = broadcast::channel(8);
        let view = StreamCropView::new(tx);

        let size = |width| StreamFrameSize { width, height: 80 };
        view.publish(None, size(100));
        view.publish(Some(crop(8)), size(92));
        view.publish(Some(crop(8)), size(46));
        view.publish(Some(crop(16)), size(84));
        view.publish(None, size(100));

        let mut announced = Vec::new();
        while let Ok(ServerEvent::StackCropChanged { crop, frame }) = rx.try_recv() {
            announced.push((crop, frame.map(|f| f.width)));
        }
        assert_eq!(announced, vec![(Some(crop(8)), Some(92)), (Some(crop(16)), Some(84)), (None, Some(100))]);
        assert_eq!(view.get(), None);
    }

    /// A client that connects is told the size frames are cut to now, not when the crop last
    /// changed: a resolution change in between moved it.
    #[test]
    fn the_snapshot_carries_the_latest_frame_size() {
        let (tx, _rx) = broadcast::channel(8);
        let view = StreamCropView::new(tx);
        view.publish(Some(crop(8)), StreamFrameSize { width: 92, height: 80 });
        view.publish(Some(crop(8)), StreamFrameSize { width: 46, height: 40 });
        let ServerEvent::StackCropChanged { crop: now, frame } = view.event() else { unreachable!() };
        assert_eq!((now, frame), (Some(crop(8)), Some(StreamFrameSize { width: 46, height: 40 })));
    }
}
