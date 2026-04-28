use night_amplifier::frame::Frame;
use night_amplifier::server::encode_rgb8_lz4;
use lz4_flex::decompress_size_prepended;
use serial_test::serial;

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_encode_imx464_no_downsample() {
    println!("\n=== Encoding Test: IMX464 No Downsample ===\n");
    let width = 2712;
    let height = 1538;
    let frame = Frame::zeros(width, height, 3).unwrap();
    
    let encoded = encode_rgb8_lz4(&frame).unwrap();
    let enc_width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let enc_height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    
    // IMX464 should not be downsampled since it is < 3840x2160
    assert_eq!(enc_width as usize, width);
    assert_eq!(enc_height as usize, height);
    
    let decompressed = decompress_size_prepended(&encoded[16..]).unwrap();
    assert_eq!(decompressed.len(), width * height * 3);
}

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_encode_8k_downsamples_to_4k() {
    println!("\n=== Encoding Test: 8K Downsamples to 4K ===\n");
    let width = 7680;
    let height = 4320;
    let mut frame = Frame::zeros(width, height, 3).unwrap();
    
    // Set a known pattern to verify downsampling math
    for y in 0..height {
        for x in 0..width {
            frame.set_pixel(x, y, 0, 0.5);
        }
    }
    
    let encoded = encode_rgb8_lz4(&frame).unwrap();
    let enc_width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let enc_height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    
    // 8K should be downsampled by 2x to 4K (3840x2160)
    assert_eq!(enc_width as usize, 3840);
    assert_eq!(enc_height as usize, 2160);
    
    let decompressed = decompress_size_prepended(&encoded[16..]).unwrap();
    assert_eq!(decompressed.len(), 3840 * 2160 * 3);
    
    // The average of 0 and 1 should be around 0.5 (which scales to ~128)
    let val = decompressed[0];
    assert!((val as i32 - 128).abs() <= 1);
}
