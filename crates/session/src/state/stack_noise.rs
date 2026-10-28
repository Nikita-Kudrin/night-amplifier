//! The window the render task carries the stack's noise figure through
//! (`capture::analysis::PreviewAnalysis::noise_window`), for the stack saved at the end of
//! the session: it renders through a fresh analysis, and without the window its gap was the
//! final crop's own MAD, 17 % over the live view's on the 106-sub dumbbell.
//!
//! Runtime state, set only by the render task, cleared as each session's render task starts.

use std::sync::Mutex;

#[derive(Default)]
pub struct StackNoiseWindow(Mutex<Option<(u64, f32)>>);

impl StackNoiseWindow {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(u64, f32)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The window the stack of `generation` was last rendered through.
    pub fn publish(&self, generation: u64, window: f32) {
        *self.lock() = Some((generation, window));
    }

    /// No stack rendered yet: a session starts.
    pub fn clear(&self) {
        *self.lock() = None;
    }

    /// The window the stack of `generation` was rendered through, 1 for any stack the render
    /// task did not show (a restack it never caught up with, a session it never rendered).
    pub fn of(&self, generation: u64) -> f32 {
        match *self.lock() {
            Some((shown, window)) if shown == generation => window,
            _ => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_stack_rendered_reads_its_window() {
        let cell = StackNoiseWindow::default();
        assert_eq!(cell.of(0), 1.0);
        cell.publish(2, 0.83);
        assert_eq!((cell.of(2), cell.of(1)), (0.83, 1.0));
        cell.clear();
        assert_eq!(cell.of(2), 1.0);
    }
}
