//! INDI Client

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info, warn};

use crate::indi::connection::IndiConnection;
use crate::indi::device::IndiDevice;
use crate::indi::error::{IndiError, Result};
use crate::indi::xml::{BlobEnable, EnableBlob, GetProperties, IndiMessage, NewNumber, NewNumberVector, NewSwitch, NewSwitchVector, SwitchState};

#[derive(Debug, Clone)]
pub struct IndiClient {
    devices: Arc<RwLock<HashMap<String, IndiDevice>>>,
    connection: Option<IndiConnection>,
    updates_tx: broadcast::Sender<IndiMessage>,
    connected: Arc<RwLock<bool>>,
}

impl IndiClient {
    pub fn new() -> Self {
        let (updates_tx, _) = broadcast::channel(100);
        Self {
            devices: Arc::new(RwLock::new(HashMap::new())),
            connection: None,
            updates_tx,
            connected: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn connect(&mut self, host: &str, port: u16, timeout: Duration) -> Result<()> {
        let (msg_tx, mut msg_rx) = mpsc::channel(100);
        let connection = IndiConnection::connect(host, port, timeout, msg_tx).await?;
        self.connection = Some(connection);
        *self.connected.write().await = true;

        let devices = self.devices.clone();
        let connected = self.connected.clone();
        let updates_tx = self.updates_tx.clone();

        // Background task to process incoming messages
        tokio::spawn(async move {
            while let Some(result) = msg_rx.recv().await {
                match result {
                    Ok(msg) => {
                        Self::update_device_state(&devices, &msg).await;
                        let _ = updates_tx.send(msg);
                    }
                    Err(IndiError::Disconnected) => {
                        warn!("INDI client disconnected from server");
                        *connected.write().await = false;
                        let _ = updates_tx.send(IndiMessage::Message(crate::indi::xml::Message {
                            device: None,
                            message: "Disconnected".to_string(),
                        }));
                        break;
                    }
                    Err(e) => {
                        error!("INDI client error: {}", e);
                    }
                }
            }
        });

        // Request all properties
        let req = GetProperties {
            version: "1.7".to_string(),
            device: None,
            name: None,
        };
        self.send_message(&req).await?;

        Ok(())
    }

    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    pub async fn disconnect(&mut self) {
        self.connection = None;
        *self.connected.write().await = false;
        self.devices.write().await.clear();
    }

    pub async fn get_device(&self, name: &str) -> Option<IndiDevice> {
        self.devices.read().await.get(name).cloned()
    }

    pub async fn list_devices(&self) -> Vec<IndiDevice> {
        self.devices.read().await.values().cloned().collect()
    }

    pub async fn send_message<T: serde::Serialize>(&self, msg: &T) -> Result<()> {
        if let Some(conn) = &self.connection {
            conn.send_message(msg).await
        } else {
            Err(IndiError::Disconnected)
        }
    }

    pub async fn enable_blob(&self, device: &str, property: Option<&str>, rule: BlobEnable) -> Result<()> {
        let req = EnableBlob {
            device: device.to_string(),
            name: property.map(|s| s.to_string()),
            value: rule,
        };
        self.send_message(&req).await
    }

    pub async fn set_number(&self, device: &str, property: &str, elements: Vec<(&str, f64)>) -> Result<()> {
        let req = NewNumberVector {
            device: device.to_string(),
            name: property.to_string(),
            elements: elements.into_iter().map(|(n, v)| NewNumber {
                name: n.to_string(),
                value: v,
            }).collect(),
        };
        self.send_message(&req).await
    }

    pub async fn set_switch(&self, device: &str, property: &str, elements: Vec<(&str, SwitchState)>) -> Result<()> {
        let req = NewSwitchVector {
            device: device.to_string(),
            name: property.to_string(),
            elements: elements.into_iter().map(|(n, v)| NewSwitch {
                name: n.to_string(),
                value: v,
            }).collect(),
        };
        self.send_message(&req).await
    }

    pub async fn wait_for_blob(
        &self,
        device: &str,
        property: &str,
        timeout: Duration,
    ) -> Result<crate::indi::xml::SetBlob> {
        let mut rx = self.updates_tx.subscribe();
        
        let wait_future = async {
            loop {
                match rx.recv().await {
                    Ok(IndiMessage::SetBlobVector(v)) if v.device == device && v.name == property => {
                        if let Some(blob) = v.elements.into_iter().next() {
                            return Ok(blob);
                        }
                    }
                    Ok(IndiMessage::Message(m)) if m.message == "Disconnected" => {
                        return Err(IndiError::Disconnected);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(IndiError::Disconnected);
                    }
                    _ => {}
                }
            }
        };

        tokio::time::timeout(timeout, wait_future)
            .await
            .map_err(|_| IndiError::Timeout(format!("BLOB from {}.{}", device, property)))?
    }

    async fn update_device_state(devices: &Arc<RwLock<HashMap<String, IndiDevice>>>, msg: &IndiMessage) {
        let mut devs = devices.write().await;
        
        match msg {
            IndiMessage::DefNumberVector(v) => {
                let dev = devs.entry(v.device.clone()).or_insert_with(|| IndiDevice::new(v.device.clone()));
                let mut elements = HashMap::new();
                for el in &v.elements {
                    elements.insert(el.name.clone(), el.clone());
                }
                dev.properties.insert(v.name.clone(), crate::indi::device::IndiProperty::Number {
                    state: v.state.clone(),
                    elements,
                });
            }
            IndiMessage::SetNumberVector(v) => {
                if let Some(dev) = devs.get_mut(&v.device) {
                    if let Some(crate::indi::device::IndiProperty::Number { state, elements }) = dev.properties.get_mut(&v.name) {
                        *state = v.state.clone();
                        for el in &v.elements {
                            if let Some(def) = elements.get_mut(&el.name) {
                                def.value = el.value;
                            }
                        }
                    }
                }
            }
            IndiMessage::DefSwitchVector(v) => {
                let dev = devs.entry(v.device.clone()).or_insert_with(|| IndiDevice::new(v.device.clone()));
                let mut elements = HashMap::new();
                for el in &v.elements {
                    elements.insert(el.name.clone(), el.clone());
                }
                dev.properties.insert(v.name.clone(), crate::indi::device::IndiProperty::Switch {
                    state: v.state.clone(),
                    rule: v.rule.clone(),
                    elements,
                });
            }
            IndiMessage::SetSwitchVector(v) => {
                if let Some(dev) = devs.get_mut(&v.device) {
                    if let Some(crate::indi::device::IndiProperty::Switch { state, elements, .. }) = dev.properties.get_mut(&v.name) {
                        *state = v.state.clone();
                        for el in &v.elements {
                            if let Some(def) = elements.get_mut(&el.name) {
                                def.value = el.value.clone();
                            }
                        }
                    }
                }
            }
            IndiMessage::DefTextVector(v) => {
                let dev = devs.entry(v.device.clone()).or_insert_with(|| IndiDevice::new(v.device.clone()));
                let mut elements = HashMap::new();
                for el in &v.elements {
                    elements.insert(el.name.clone(), el.clone());
                }
                dev.properties.insert(v.name.clone(), crate::indi::device::IndiProperty::Text {
                    state: v.state.clone(),
                    elements,
                });
            }
            IndiMessage::SetTextVector(v) => {
                if let Some(dev) = devs.get_mut(&v.device) {
                    if let Some(crate::indi::device::IndiProperty::Text { state, elements }) = dev.properties.get_mut(&v.name) {
                        *state = v.state.clone();
                        for el in &v.elements {
                            if let Some(def) = elements.get_mut(&el.name) {
                                def.value = el.value.clone();
                            }
                        }
                    }
                }
            }
            IndiMessage::DefLightVector(v) => {
                let dev = devs.entry(v.device.clone()).or_insert_with(|| IndiDevice::new(v.device.clone()));
                let mut elements = HashMap::new();
                for el in &v.elements {
                    elements.insert(el.name.clone(), el.clone());
                }
                dev.properties.insert(v.name.clone(), crate::indi::device::IndiProperty::Light {
                    state: v.state.clone(),
                    elements,
                });
            }
            IndiMessage::SetLightVector(v) => {
                if let Some(dev) = devs.get_mut(&v.device) {
                    if let Some(crate::indi::device::IndiProperty::Light { state, elements }) = dev.properties.get_mut(&v.name) {
                        *state = v.state.clone();
                        for el in &v.elements {
                            if let Some(def) = elements.get_mut(&el.name) {
                                def.value = el.value.clone();
                            }
                        }
                    }
                }
            }
            IndiMessage::DefBlobVector(v) => {
                let dev = devs.entry(v.device.clone()).or_insert_with(|| IndiDevice::new(v.device.clone()));
                let mut elements = HashMap::new();
                for el in &v.elements {
                    elements.insert(el.name.clone(), el.clone());
                }
                dev.properties.insert(v.name.clone(), crate::indi::device::IndiProperty::Blob {
                    state: v.state.clone(),
                    elements,
                });
            }
            IndiMessage::DelProperty(v) => {
                if let Some(dev) = devs.get_mut(&v.device) {
                    if let Some(name) = &v.name {
                        dev.properties.remove(name);
                    } else {
                        // Delete entire device
                        devs.remove(&v.device);
                    }
                }
            }
            _ => {}
        }
    }
}

