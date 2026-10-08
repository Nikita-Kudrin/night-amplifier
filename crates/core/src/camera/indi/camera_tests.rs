//! `IndiCamera` against a fake INDI driver over a real socket: the client, the XML and
//! the FITS decoding all run for real, only the driver is scripted.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use super::IndiCamera;
use crate::camera::{AcquisitionMode, Camera, CameraError, CaptureConfig, ImageFormat};

const DEVICE: &str = "CCD Simulator";
const WIDTH: usize = 8;
const HEIGHT: usize = 4;

/// What the driver does, and every message it was sent.
#[derive(Default)]
struct Driver {
    /// Defines `CCD_VIDEO_STREAM` and streams a frame every 20 ms while it is on.
    video: bool,
    /// Never answers an exposure, so only a cancel ends one.
    silent: bool,
    received: Mutex<Vec<String>>,
}

impl Driver {
    /// `newNumberVector CCD_EXPOSURE`, `newSwitchVector CCD_VIDEO_STREAM STREAM_ON`, …:
    /// the tag, the property and the switch turned on, in the order they arrived.
    fn sent(&self) -> Vec<String> {
        self.received.lock().unwrap().iter().map(|xml| summary(xml)).collect()
    }

    fn count(&self, summary: &str) -> usize {
        self.sent().iter().filter(|sent| *sent == summary).count()
    }

    /// What was sent, once nothing more has arrived for 50 ms: the socket is read on
    /// its own task, so a capture can return before its last message is.
    async fn settled(&self) -> Vec<String> {
        loop {
            let seen = self.received.lock().unwrap().len();
            tokio::time::sleep(Duration::from_millis(50)).await;
            if self.received.lock().unwrap().len() == seen {
                return self.sent();
            }
        }
    }

    fn clear(&self) {
        self.received.lock().unwrap().clear();
    }
}

fn attribute<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let start = xml.find(&format!("{name}=\""))? + name.len() + 2;
    Some(&xml[start..start + xml[start..].find('"')?])
}

fn summary(xml: &str) -> String {
    let tag = xml.trim_start_matches('<').split([' ', '>', '/']).next().unwrap_or("");
    let mut summary = format!("{tag} {}", attribute(xml, "name").unwrap_or(""));
    let switched_on = xml
        .split("<oneSwitch ")
        .skip(1)
        .find(|element| element.contains(">On<"))
        .and_then(|element| attribute(element, "name"));
    if let Some(on) = switched_on {
        summary.push(' ');
        summary.push_str(on);
    }
    summary.trim_end().to_string()
}

/// A FITS file of 16-bit pixels counting up from 1000.
fn fits_frame() -> Vec<u8> {
    let mut header = String::new();
    for record in [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        format!("NAXIS1  = {WIDTH:>20}"),
        format!("NAXIS2  = {HEIGHT:>20}"),
        "END".to_string(),
    ] {
        header.push_str(&format!("{record:<80}"));
    }
    let mut fits = format!("{header:<2880}").into_bytes();
    for value in 0..(WIDTH * HEIGHT) as u16 {
        fits.extend_from_slice(&(1000 + value).to_be_bytes());
    }
    fits.resize(2880 * 2, 0);
    fits
}

/// Formatted as indiserver writes it: single quotes, each value on a line of its own.
fn blob_message() -> String {
    let fits = fits_frame();
    format!(
        "<setBLOBVector device='{DEVICE}' name='CCD1' state='Ok'>\n  <oneBLOB\n    name='CCD1'\n    size='{}'\n    format='.fits'>\n{}\n  </oneBLOB>\n</setBLOBVector>\n",
        fits.len(),
        BASE64_STANDARD.encode(&fits)
    )
}

fn definitions(video: bool) -> String {
    let number = |name: &str, value: usize| {
        format!("  <defNumber name='{name}' min='0' max='10000' step='1'>\n{value}\n  </defNumber>\n")
    };
    let mut xml = format!(
        "<defNumberVector device='{DEVICE}' name='CCD_EXPOSURE' state='Idle'>\n{}</defNumberVector>\n\
         <defNumberVector device='{DEVICE}' name='CCD_INFO' state='Idle'>\n{}{}</defNumberVector>\n",
        number("CCD_EXPOSURE_VALUE", 1),
        number("CCD_MAX_X", WIDTH),
        number("CCD_MAX_Y", HEIGHT),
    );
    if video {
        xml.push_str(&format!(
            "<defSwitchVector device='{DEVICE}' name='CCD_VIDEO_STREAM' state='Idle' rule='OneOfMany'>\n  \
             <defSwitch name='STREAM_ON'>\nOff\n  </defSwitch>\n  \
             <defSwitch name='STREAM_OFF'>\nOn\n  </defSwitch>\n</defSwitchVector>\n"
        ));
    }
    xml
}

/// Serves one client: definitions on `getProperties`, a frame per `CCD_EXPOSURE`, and
/// a frame every 20 ms while the stream is on.
async fn serve(driver: Arc<Driver>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (mut read, mut write) = socket.into_split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(xml) = outgoing.recv().await {
                if write.write_all(xml.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let streaming = Arc::new(AtomicBool::new(false));
        let mut pending = String::new();
        let mut chunk = [0u8; 4096];
        while let Ok(n) = read.read(&mut chunk).await {
            if n == 0 {
                break;
            }
            pending.push_str(&String::from_utf8_lossy(&chunk[..n]));
            while let Some(end) = pending.find('\n') {
                let xml = pending[..end].trim().to_string();
                pending.drain(..=end);
                driver.received.lock().unwrap().push(xml.clone());
                match summary(&xml).as_str() {
                    "getProperties" => drop(out.send(definitions(driver.video))),
                    "newNumberVector CCD_EXPOSURE" if !driver.silent => {
                        drop(out.send(blob_message()))
                    }
                    "newSwitchVector CCD_VIDEO_STREAM STREAM_ON" => {
                        streaming.store(true, Ordering::SeqCst);
                        let (streaming, out) = (Arc::clone(&streaming), out.clone());
                        tokio::spawn(async move {
                            while streaming.load(Ordering::SeqCst) && out.send(blob_message()).is_ok() {
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                        });
                    }
                    "newSwitchVector CCD_VIDEO_STREAM STREAM_OFF" => {
                        streaming.store(false, Ordering::SeqCst)
                    }
                    _ => {}
                }
            }
        }
    });
    port
}

async fn camera_on(driver: &Arc<Driver>) -> IndiCamera {
    let port = serve(Arc::clone(driver)).await;
    let camera = IndiCamera::connect("127.0.0.1".to_string(), port, 0).await.unwrap();
    assert_eq!(driver.settled().await.last().map(String::as_str), Some("enableBLOB CCD1"));
    driver.clear();
    camera
}

fn config(acquisition: AcquisitionMode, exposure_us: u64) -> CaptureConfig {
    CaptureConfig {
        exposure_us,
        acquisition,
        format: ImageFormat::Raw16,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_exposure_is_configured_once_and_returns_the_frame_it_triggered() {
    let driver = Arc::new(Driver::default());
    let mut camera = camera_on(&driver).await;
    let snap = config(AcquisitionMode::Snap, 1_000);

    let frame = camera.capture(&snap).unwrap();
    assert_eq!((frame.width, frame.height), (WIDTH as u32, HEIGHT as u32));
    assert_eq!(frame.format, ImageFormat::Raw16);
    assert_eq!(&frame.data[..4], &[0xE8, 0x03, 0xE9, 0x03], "1000, 1001 little-endian");
    assert_eq!(
        driver.settled().await,
        [
            "newSwitchVector CCD_FRAME_TYPE FRAME_LIGHT",
            "newNumberVector CCD_BINNING",
            "newNumberVector CCD_FRAME",
            "newNumberVector CCD_GAIN",
            "newNumberVector CCD_OFFSET",
            "newNumberVector CCD_EXPOSURE",
        ]
    );

    driver.clear();
    camera.capture(&snap).unwrap();
    assert_eq!(
        driver.settled().await,
        ["newNumberVector CCD_EXPOSURE"],
        "same config, not re-applied"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_is_started_once_and_stopped_before_a_new_config() {
    let driver = Arc::new(Driver {
        video: true,
        ..Default::default()
    });
    let mut camera = camera_on(&driver).await;

    for _ in 0..3 {
        camera.capture(&config(AcquisitionMode::Video, 1_000)).unwrap();
    }
    driver.settled().await;
    assert_eq!(driver.count("newSwitchVector CCD_VIDEO_STREAM STREAM_ON"), 1);
    assert_eq!(driver.count("newNumberVector CCD_EXPOSURE"), 1);

    driver.clear();
    camera.capture(&config(AcquisitionMode::Video, 2_000)).unwrap();
    let sent = driver.settled().await;
    assert_eq!(sent.first().map(String::as_str), Some("newSwitchVector CCD_VIDEO_STREAM STREAM_OFF"));
    assert_eq!(
        &sent[sent.len() - 2..],
        ["newSwitchVector CCD_VIDEO_STREAM STREAM_ON", "newNumberVector CCD_EXPOSURE"]
    );
}

/// A driver without a video stream takes the stream's frames one exposure at a time,
/// as it always did.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_video_stream_every_frame_is_its_own_exposure() {
    let driver = Arc::new(Driver::default());
    let mut camera = camera_on(&driver).await;

    for _ in 0..3 {
        camera.capture(&config(AcquisitionMode::Video, 1_000)).unwrap();
    }
    let sent = driver.settled().await;
    assert_eq!(driver.count("newNumberVector CCD_EXPOSURE"), 3);
    assert!(!sent.iter().any(|sent| sent.contains("CCD_VIDEO_STREAM")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_aborts_the_exposure_in_flight() {
    let driver = Arc::new(Driver {
        silent: true,
        ..Default::default()
    });
    let mut camera = camera_on(&driver).await;
    let cancel = camera.cancel_token();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.store(true, Ordering::SeqCst);
    });

    let result = camera.capture(&config(AcquisitionMode::Snap, 30_000_000));
    assert!(matches!(result, Err(CameraError::Cancelled)), "{result:?}");
    assert_eq!(
        driver.settled().await.last().map(String::as_str),
        Some("newSwitchVector CCD_ABORT_EXPOSURE ABORT")
    );
}

