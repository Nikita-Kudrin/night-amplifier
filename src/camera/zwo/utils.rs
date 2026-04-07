/// Convert DynamicSerialImage to raw bytes
pub(crate) fn image_to_bytes(image: cameraunit::DynamicSerialImage) -> Vec<u8> {
    use cameraunit::DynamicSerialImage;

    match image {
        DynamicSerialImage::U8(img) => img.into_vec(),
        DynamicSerialImage::U16(img) => {
            // Convert u16 to bytes (little endian)
            let raw = img.into_vec();
            let mut bytes = Vec::with_capacity(raw.len() * 2);
            for val in raw {
                bytes.extend_from_slice(&val.to_le_bytes());
            }
            bytes
        }
        DynamicSerialImage::F32(img) => {
            // Convert f32 to u16 bytes (scale 0.0-1.0 to 0-65535)
            let raw = img.into_vec();
            let mut bytes = Vec::with_capacity(raw.len() * 2);
            for val in raw {
                let scaled = (val.clamp(0.0, 1.0) * 65535.0) as u16;
                bytes.extend_from_slice(&scaled.to_le_bytes());
            }
            bytes
        }
    }
}
