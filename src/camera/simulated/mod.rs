//! Simulated camera for testing and development
//!
//! This module provides a simulated camera that reads image files from a directory,
//! allowing users to test the stacking pipeline without physical camera hardware.
//! Multiple simulated cameras can be added, each pointing to a different directory.

mod camera;
mod loaders;
mod probe;
mod registry;

pub use camera::SimulatedCamera;
pub use registry::{
    add_simulated_directory, clear_simulated_directories, clear_simulated_directory,
    get_simulated_directories, get_simulated_directory, remove_simulated_directory,
    set_simulated_directory, SimulatedProvider,
};
