//! What the render task last showed the stack through (`capture::analysis::PreviewAnalysis::
//! noise_window`, and the crop it was measured on), for the stack saved at the end of the
//! session: it renders through a fresh analysis, and without the window its gap was the final
//! crop's own MAD, 17 % over the live view's on the 106-sub dumbbell.
//!
//! Runtime state, set only by the render task, cleared as each capture starts its threads:
//! cleared by the render thread itself, a save racing its start could read the last session's.

use std::sync::Mutex;

use night_amplifier_core::stacking::CropRect;

/// The crop and noise window the render task showed one stack generation through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShownStack {
    pub crop: Option<CropRect>,
    pub window: f32,
}

#[derive(Default)]
pub struct StackNoiseWindow(Mutex<Option<(u64, ShownStack)>>);

impl StackNoiseWindow {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(u64, ShownStack)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The stack of `generation` was last rendered cut to `crop`, through `window`.
    pub fn publish(&self, generation: u64, crop: Option<CropRect>, window: f32) {
        *self.lock() = Some((generation, ShownStack { crop, window }));
    }

    /// No stack rendered yet: a capture starts.
    pub fn clear(&self) {
        *self.lock() = None;
    }

    /// How the stack of `generation` was last rendered; `None` for any stack the render task
    /// did not show (a restack it never caught up with, a session it never rendered).
    pub fn shown(&self, generation: u64) -> Option<ShownStack> {
        match *self.lock() {
            Some((shown, stack)) if shown == generation => Some(stack),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_stack_rendered_reads_how_it_was_shown() {
        let cell = StackNoiseWindow::default();
        assert_eq!(cell.shown(0), None);
        let crop = Some(CropRect { x: 8, y: 0, width: 64, height: 64 });
        cell.publish(2, crop, 0.83);
        assert_eq!((cell.shown(2), cell.shown(1)), (Some(ShownStack { crop, window: 0.83 }), None));
        cell.clear();
        assert_eq!(cell.shown(2), None);
    }
}
