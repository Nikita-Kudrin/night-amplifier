//! INDI Connection

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, warn};

use crate::indi::error::{IndiError, Result};
use crate::indi::xml::{parse_message, serialize_message, IndiMessage};

const READ_BUFFER_SIZE: usize = 65536; // 64KB chunks
const MAX_BUFFER_SIZE: usize = 100 * 1024 * 1024; // 100MB max buffer (for large BLOBs)

#[derive(Debug, Clone)]
pub struct IndiConnection {
    sender: mpsc::Sender<String>,
}

impl IndiConnection {
    pub async fn connect(
        host: &str,
        port: u16,
        timeout: Duration,
        message_tx: mpsc::Sender<Result<IndiMessage>>,
    ) -> Result<Self> {
        let addr = format!("{}:{}", host, port);
        let stream = tokio::time::timeout(timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| IndiError::Timeout(format!("Connection to {}", addr)))??;

        let (mut read_half, mut write_half) = stream.into_split();
        let (tx, mut rx) = mpsc::channel::<String>(100);

        // Writer task
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = write_half.write_all(msg.as_bytes()).await {
                    error!("INDI connection write error: {}", e);
                    break;
                }
                if let Err(e) = write_half.write_all(b"\n").await {
                    error!("INDI connection write error: {}", e);
                    break;
                }
            }
        });

        // Reader task
        tokio::spawn(async move {
            let mut buffer = Vec::with_capacity(READ_BUFFER_SIZE);
            let mut read_buf = [0u8; READ_BUFFER_SIZE];

            loop {
                match read_half.read(&mut read_buf).await {
                    Ok(0) => {
                        // EOF
                        let _ = message_tx.send(Err(IndiError::Disconnected)).await;
                        break;
                    }
                    Ok(n) => {
                        buffer.extend_from_slice(&read_buf[..n]);
                        if buffer.len() > MAX_BUFFER_SIZE {
                            error!("INDI buffer exceeded max size, dropping connection");
                            let _ = message_tx.send(Err(IndiError::Disconnected)).await;
                            break;
                        }

                        // Try to extract full XML elements
                        Self::process_buffer(&mut buffer, &message_tx).await;
                    }
                    Err(e) => {
                        error!("INDI connection read error: {}", e);
                        let _ = message_tx.send(Err(IndiError::Network(e))).await;
                        break;
                    }
                }
            }
        });

        Ok(Self { sender: tx })
    }

    pub async fn send_message<T: serde::Serialize>(&self, msg: &T) -> Result<()> {
        let xml = serialize_message(msg)?;
        self.sender
            .send(xml)
            .await
            .map_err(|_| IndiError::Disconnected)?;
        Ok(())
    }

    async fn process_buffer(buffer: &mut Vec<u8>, message_tx: &mpsc::Sender<Result<IndiMessage>>) {
        let mut reader = quick_xml::Reader::from_reader(buffer.as_slice());
        let mut depth = 0;
        let mut start_pos = 0;
        let mut current_pos = 0;
        let mut found_element = false;

        loop {
            // Check if we have a full tag
            let buf = reader.get_ref();
            let mut byte_reader = quick_xml::Reader::from_reader(*buf);
            byte_reader.config_mut().trim_text(true);
            let mut event_buf = Vec::new();
            
            loop {
                let pos_before = byte_reader.buffer_position();
                match byte_reader.read_event_into(&mut event_buf) {
                    Ok(quick_xml::events::Event::Start(_)) => {
                        if depth == 0 {
                            start_pos = pos_before;
                        }
                        depth += 1;
                    }
                    Ok(quick_xml::events::Event::Empty(_)) => {
                        if depth == 0 {
                            start_pos = pos_before;
                            current_pos = byte_reader.buffer_position();
                            found_element = true;
                            break;
                        }
                    }
                    Ok(quick_xml::events::Event::End(_)) => {
                        depth -= 1;
                        if depth == 0 {
                            current_pos = byte_reader.buffer_position();
                            found_element = true;
                            break;
                        }
                    }
                    Ok(quick_xml::events::Event::Eof) => {
                        break; // Need more data
                    }
                    Err(_) => {
                        break; // Parsing error, wait for more data
                    }
                    _ => {}
                }
            }

            if found_element {
                let element_bytes = &buffer[start_pos as usize..current_pos as usize];
                if let Ok(xml_str) = std::str::from_utf8(element_bytes) {
                    // Try to parse the message
                    match parse_message(xml_str) {
                        Ok(msg) => {
                            let _ = message_tx.send(Ok(msg)).await;
                        }
                        Err(e) => {
                            warn!("Failed to parse INDI message: {} (XML: {:.100})", e, xml_str);
                        }
                    }
                }
                
                // Consume parsed part
                buffer.drain(0..current_pos as usize);
                // Reset state
                reader = quick_xml::Reader::from_reader(buffer.as_slice());
                depth = 0;
                start_pos = 0;
                current_pos = 0;
                found_element = false;
            } else {
                break; // Need more data to complete an element
            }
        }
    }
}

