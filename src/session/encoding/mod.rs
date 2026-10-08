//! The wire formats: SA08/SA09 lossless RGB8 and SA10 JPEG, framed around the pixels
//! `render::display` produces.

pub mod format;
pub mod jpeg;
pub mod lz4;

#[cfg(test)]
pub mod tests;

pub use format::*;
pub use jpeg::*;
pub use lz4::*;
