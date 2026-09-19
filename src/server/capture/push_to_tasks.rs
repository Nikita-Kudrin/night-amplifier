//! Push-To as pipeline tasks, like storage and render: dedicated threads fed by one-slot
//! channels. Frames for plate solving are never queued, and the plugin's synchronous work
//! (star detection, the FITS write) runs on these threads rather than holding a server
//! runtime worker.
//!
//! Two consumers because the plugin has two claims that run at once: a solve can wait on
//! ASTAP for minutes, and the movement watch must keep seeing frames meanwhile to notice a
//! slew. A producer hands a frame only to an idle consumer (`QueueDepth::try_claim_idle`,
//! the render gate's rule) and drops it otherwise. Spawning a task per offer instead kept
//! 27 of 32 frames alive, the oldest 5.7 s stale, whenever solve and watch were both busy.

use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Weak};

use tokio::runtime::Handle;
use tracing::error;

use super::channel::QueueDepth;
use super::solving::{self, SolveSource};
use crate::frame::Frame;
use crate::server::state::AppState;

/// Which of the two consumers a frame is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    /// May start a solve and wait for it.
    Solve,
    /// Looks for movement while a solve runs, and returns promptly.
    Watch,
}

impl Lane {
    /// The consumer an offer made now belongs to.
    pub(crate) fn for_solving(solve_in_flight: bool) -> Self {
        if solve_in_flight {
            Self::Watch
        } else {
            Self::Solve
        }
    }

    fn thread_name(self) -> &'static str {
        match self {
            Self::Solve => "push-to-solve",
            Self::Watch => "push-to-watch",
        }
    }
}

struct Offer {
    frame: Arc<Frame>,
    source: SolveSource,
}

/// What a consumer does with a frame. Production dispatches into `solving`; tests
/// substitute their own.
type Handler = fn(&Handle, Lane, &Arc<AppState>, Arc<Frame>, SolveSource);

fn dispatch(rt: &Handle, lane: Lane, state: &Arc<AppState>, frame: Arc<Frame>, source: SolveSource) {
    rt.block_on(async {
        match lane {
            Lane::Solve => solving::solve_frame(state, frame, source).await,
            Lane::Watch => solving::watch_frame(state, frame, source).await,
        }
    })
}

struct Consumer {
    tx: SyncSender<Offer>,
    /// Offered and not yet handled: one while the consumer holds a frame, so
    /// `pending() == 0` is "idle".
    depth: QueueDepth,
    /// Detached on drop; kept so a test can see the thread end with its state.
    #[cfg_attr(not(test), allow(dead_code))]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Consumer {
    /// A consumer that could not start keeps a sender whose receiver is gone, so every
    /// offer to it is refused rather than queued.
    fn start(lane: Lane, state: Weak<AppState>, rt: Handle, handler: Handler) -> Self {
        let (tx, rx) = sync_channel(1);
        let depth = QueueDepth::default();
        let handled = depth.clone();
        let thread = std::thread::Builder::new()
            .name(lane.thread_name().to_string())
            .spawn(move || consume(lane, rx, handled, state, rt, handler))
            .inspect_err(|e| {
                error!(lane = lane.thread_name(), error = %e, "Could not start a Push-To task; plate solving is off");
            })
            .ok();
        Self { tx, depth, thread }
    }
}

/// The two Push-To consumers of one `AppState`, started by its first offer.
pub(crate) struct PushToTasks {
    solve: Consumer,
    watch: Consumer,
}

impl PushToTasks {
    /// The threads hold the state weakly and end once it is dropped, which also drops
    /// the senders their `recv` waits on.
    fn start(state: &Arc<AppState>, rt: &Handle, handler: Handler) -> Self {
        Self {
            solve: Consumer::start(Lane::Solve, Arc::downgrade(state), rt.clone(), handler),
            watch: Consumer::start(Lane::Watch, Arc::downgrade(state), rt.clone(), handler),
        }
    }

    fn consumer(&self, lane: Lane) -> &Consumer {
        match lane {
            Lane::Solve => &self.solve,
            Lane::Watch => &self.watch,
        }
    }
}

/// Whether `lane`'s consumer would take a frame now. Before the first offer neither task
/// has started, and both are free.
pub(crate) fn is_idle(state: &AppState, lane: Lane) -> bool {
    state
        .push_to_tasks
        .get()
        .is_none_or(|tasks| tasks.consumer(lane).depth.pending() == 0)
}

/// Hand `frame` to `lane`'s consumer if it is idle, and drop it otherwise. Returns
/// whether the consumer took it.
pub(crate) fn offer(
    state: &Arc<AppState>,
    rt: &Handle,
    lane: Lane,
    frame: Arc<Frame>,
    source: SolveSource,
) -> bool {
    let consumer = state
        .push_to_tasks
        .get_or_init(|| PushToTasks::start(state, rt, dispatch))
        .consumer(lane);
    if !consumer.depth.try_claim_idle() {
        return false;
    }
    if consumer.tx.try_send(Offer { frame, source }).is_err() {
        consumer.depth.taken();
        return false;
    }
    true
}

/// Gives the consumer's slot back however handling ends.
struct Handled<'a>(&'a QueueDepth);

impl Drop for Handled<'_> {
    fn drop(&mut self) {
        self.0.taken();
    }
}

fn consume(
    lane: Lane,
    rx: Receiver<Offer>,
    depth: QueueDepth,
    state: Weak<AppState>,
    rt: Handle,
    handler: Handler,
) {
    while let Ok(Offer { frame, source }) = rx.recv() {
        let _handled = Handled(&depth);
        let Some(state) = state.upgrade() else {
            return;
        };
        // A panicking plugin loses this frame, not the task: a dead consumer would end
        // plate solving for the life of the process. The latches release on unwind.
        let handled = std::panic::catch_unwind(AssertUnwindSafe(|| {
            handler(&rt, lane, &state, frame, source)
        }));
        if handled.is_err() {
            error!(lane = lane.thread_name(), "Push-To panicked on a frame; dropping it");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Arc<Frame> {
        frame_valued(0.0)
    }

    fn frame_valued(value: f32) -> Arc<Frame> {
        Arc::new(Frame::from_f32_vec(vec![value; 4], 2, 2, 1).unwrap())
    }

    fn state_with_tasks(rt: &Handle, handler: Handler) -> Arc<AppState> {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);
        state.push_to_tasks.get_or_init(|| PushToTasks::start(&state, rt, handler));
        state
    }

    fn wait_idle(state: &AppState, lane: Lane) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !is_idle(state, lane) {
            if std::time::Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        true
    }

    static HANDLED: std::sync::Mutex<Vec<(Lane, f32)>> = std::sync::Mutex::new(Vec::new());

    /// Panics on a frame valued 1.0, records every other.
    fn panics_on_one(_: &Handle, lane: Lane, _: &Arc<AppState>, frame: Arc<Frame>, _: SolveSource) {
        let value = frame.data()[0];
        assert!(value != 1.0, "a plugin panicking on this frame");
        HANDLED.lock().unwrap().push((lane, value));
    }

    /// A panic inside the plugin must cost that frame only: a consumer that died with it
    /// would refuse every later offer and end plate solving for the process.
    #[test]
    fn a_panicking_frame_does_not_end_the_task() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let state = state_with_tasks(rt.handle(), panics_on_one);

        assert!(offer(&state, rt.handle(), Lane::Watch, frame_valued(1.0), SolveSource::Guide));
        assert!(wait_idle(&state, Lane::Watch), "the slot was not given back after the panic");

        assert!(offer(&state, rt.handle(), Lane::Watch, frame_valued(2.0), SolveSource::Guide));
        assert!(wait_idle(&state, Lane::Watch));
        assert!(HANDLED.lock().unwrap().contains(&(Lane::Watch, 2.0)), "the task stopped taking frames");
    }

    /// A consumer holding a frame refuses the next one instead of queueing it, and the
    /// refused frame is released straight away.
    #[test]
    fn an_offer_to_a_busy_consumer_is_dropped_not_queued() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let tasks = state
            .push_to_tasks
            .get_or_init(|| PushToTasks::start(&state, rt.handle(), panics_on_one));
        assert!(tasks.solve.depth.try_claim_idle(), "stand in for a frame being handled");

        let refused = frame();
        assert!(!is_idle(&state, Lane::Solve));
        assert!(!offer(&state, rt.handle(), Lane::Solve, Arc::clone(&refused), SolveSource::Main));
        assert_eq!(Arc::strong_count(&refused), 1, "the refused frame must not be held");
        assert_eq!(tasks.solve.depth.pending(), 1, "a refusal must not give back the busy slot");

        assert!(is_idle(&state, Lane::Watch), "the watch lane is independent");
    }

    #[test]
    fn the_lane_follows_whether_a_solve_is_in_flight() {
        assert_eq!(Lane::for_solving(false), Lane::Solve);
        assert_eq!(Lane::for_solving(true), Lane::Watch);
    }

    /// The consumers hold the state weakly, and dropping it closes the channels they wait
    /// on: both threads end instead of leaking a pair per `AppState`.
    #[test]
    fn the_tasks_end_with_their_state() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        state
            .push_to_tasks
            .get_or_init(|| PushToTasks::start(&state, rt.handle(), panics_on_one));
        assert!(offer(&state, rt.handle(), Lane::Solve, frame(), SolveSource::Main));
        assert!(wait_idle(&state, Lane::Solve), "precondition: the task handled a frame");

        let mut state = Arc::try_unwrap(state)
            .unwrap_or_else(|_| panic!("a Push-To task kept its AppState alive"));
        let mut tasks = state.push_to_tasks.take().expect("started above");
        let threads = [tasks.solve.thread.take(), tasks.watch.thread.take()].map(|t| t.expect("spawned"));
        drop(tasks);
        drop(state);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while threads.iter().any(|t| !t.is_finished()) {
            assert!(std::time::Instant::now() < deadline, "a Push-To thread outlived its state");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
