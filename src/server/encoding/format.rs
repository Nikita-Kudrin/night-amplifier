/// Binary header magic number for RGB8+LZ4 stream format (legacy single-block)
///
/// Header layout (16 bytes):
/// - bytes 0-3:   Magic number "SA08" (0x53413038)
/// - bytes 4-7:   Width (u32, little-endian)
/// - bytes 8-11:  Height (u32, little-endian)
/// - bytes 12-15: Compressed size (u32, little-endian)
///
/// Followed by LZ4-compressed RGB8 pixel data (3 bytes per pixel)
pub const RGB8_MAGIC: u32 = 0x53413038; // "SA08" in little-endian

/// Binary header magic number for chunked RGB8+LZ4 stream format
///
/// Header (20 bytes):
/// - bytes 0-3:    Magic "SA09" (0x53413039)
/// - bytes 4-7:    Width (u32 LE)
/// - bytes 8-11:   Height (u32 LE)
/// - bytes 12-15:  Total payload size (u32 LE) — everything after header
/// - bytes 16-19:  Chunk count (u32 LE)
///
/// Per-chunk descriptor (8 bytes each, chunk_count entries):
/// - bytes 0-3:    Compressed size of this chunk (u32 LE)
/// - bytes 4-7:    Decompressed size of this chunk (u32 LE)
///
/// Followed by concatenated compressed chunk data
pub const RGB8_CHUNKED_MAGIC: u32 = 0x53413039; // "SA09" in little-endian

/// Binary header magic number for JPEG stream format (SA10)
///
/// Header (16 bytes):
/// - bytes 0-3:    Magic "SA10" (0x53413130)
/// - bytes 4-7:    Width (u32 LE)
/// - bytes 8-11:   Height (u32 LE)
/// - bytes 12-15:  Payload size (u32 LE)
/// - Followed by raw JPEG bytes
pub const JPEG_MAGIC: u32 = 0x53413130; // "SA10" in little-endian

pub const SA09_HEADER_SIZE: usize = 20;
pub const SA09_CHUNK_DESCRIPTOR_SIZE: usize = 8;
pub const SA10_HEADER_SIZE: usize = 16;
