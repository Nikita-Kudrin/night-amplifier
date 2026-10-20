//! INDI Client

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, warn};

use crate::indi::connection::IndiConnection;
use crate::indi::device::{IndiDevice, IndiProperty};
use crate::indi::error::{IndiError, Result};
use crate::indi::xml::{
    BlobEnable, EnableBlob, GetProperties, IndiMessage, NewNumber, NewNumberVector, NewSwitch,
    NewSwitchVector, SwitchState,
};

#[derive(Debug, Clone)]
pub struct IndiClient {
    devices: Arc<RwLock<HashMap<String, IndiDevice>>>,
    connection: Option<IndiConnection>,
    updates_tx: broadcast::Sender<IndiMessage>,
    connected: Arc<RwLock<bool>>,
}

impl Default for IndiClient {
    fn default() -> Self {
        Self::new()
    }
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

    pub async fn enable_blob(
        &self,
        device: &str,
        property: Option<&str>,
        rule: BlobEnable,
    ) -> Result<()> {
        let req = EnableBlob {
            device: device.to_string(),
            name: property.map(|s| s.to_string()),
            value: rule,
        };
        self.send_message(&req).await
    }

    pub async fn set_number(
        &self,
        device: &str,
        property: &str,
        elements: Vec<(&str, f64)>,
    ) -> Result<()> {
        let req = NewNumberVector {
            device: device.to_string(),
            name: property.to_string(),
            elements: elements
                .into_iter()
                .map(|(n, v)| NewNumber {
                    name: n.to_string(),
                    value: v,
                })
                .collect(),
        };
        self.send_message(&req).await
    }

    pub async fn set_switch(
        &self,
        device: &str,
        property: &str,
        elements: Vec<(&str, SwitchState)>,
    ) -> Result<()> {
        let req = NewSwitchVector {
            device: device.to_string(),
            name: property.to_string(),
            elements: elements
                .into_iter()
                .map(|(n, v)| NewSwitch {
                    name: n.to_string(),
                    value: v,
                })
                .collect(),
        };
        self.send_message(&req).await
    }

    /// Every BLOB `device` sends on `property` from now on. Taken before an exposure is
    /// triggered, so a frame that arrives before the first wait is not lost.
    pub fn watch_blobs(&self, device: &str, property: &str) -> BlobWatch {
        BlobWatch {
            updates: self.updates_tx.subscribe(),
            device: device.to_string(),
            property: property.to_string(),
        }
    }

    async fn update_device_state(
        devices: &Arc<RwLock<HashMap<String, IndiDevice>>>,
        msg: &IndiMessage,
    ) {
        let mut devs = devices.write().await;

        match msg {
            IndiMessage::DefNumberVector(v) => {
                let elements = by_name(&v.elements, |el| &el.name);
                let property = IndiProperty::Number { state: v.state.clone(), elements };
                define(&mut devs, &v.device, &v.name, property);
            }
            IndiMessage::DefSwitchVector(v) => {
                let elements = by_name(&v.elements, |el| &el.name);
                let (state, rule) = (v.state.clone(), v.rule.clone());
                define(&mut devs, &v.device, &v.name, IndiProperty::Switch { state, rule, elements });
            }
            IndiMessage::DefTextVector(v) => {
                let elements = by_name(&v.elements, |el| &el.name);
                let property = IndiProperty::Text { state: v.state.clone(), elements };
                define(&mut devs, &v.device, &v.name, property);
            }
            IndiMessage::DefLightVector(v) => {
                let elements = by_name(&v.elements, |el| &el.name);
                let property = IndiProperty::Light { state: v.state.clone(), elements };
                define(&mut devs, &v.device, &v.name, property);
            }
            IndiMessage::DefBlobVector(v) => {
                let elements = by_name(&v.elements, |el| &el.name);
                let property = IndiProperty::Blob { state: v.state.clone(), elements };
                define(&mut devs, &v.device, &v.name, property);
            }
            IndiMessage::SetNumberVector(v) => {
                let property = defined(&mut devs, &v.device, &v.name);
                if let Some(IndiProperty::Number { state, elements }) = property {
                    *state = v.state.clone();
                    for el in &v.elements {
                        if let Some(def) = elements.get_mut(&el.name) {
                            def.value = el.value;
                        }
                    }
                }
            }
            IndiMessage::SetSwitchVector(v) => {
                let property = defined(&mut devs, &v.device, &v.name);
                if let Some(IndiProperty::Switch { state, elements, .. }) = property {
                    *state = v.state.clone();
                    for el in &v.elements {
                        if let Some(def) = elements.get_mut(&el.name) {
                            def.value = el.value.clone();
                        }
                    }
                }
            }
            IndiMessage::SetTextVector(v) => {
                let property = defined(&mut devs, &v.device, &v.name);
                if let Some(IndiProperty::Text { state, elements }) = property {
                    *state = v.state.clone();
                    for el in &v.elements {
                        if let Some(def) = elements.get_mut(&el.name) {
                            def.value = el.value.clone();
                        }
                    }
                }
            }
            IndiMessage::SetLightVector(v) => {
                let property = defined(&mut devs, &v.device, &v.name);
                if let Some(IndiProperty::Light { state, elements }) = property {
                    *state = v.state.clone();
                    for el in &v.elements {
                        if let Some(def) = elements.get_mut(&el.name) {
                            def.value = el.value.clone();
                        }
                    }
                }
            }
            IndiMessage::DelProperty(v) => match &v.name {
                Some(name) => {
                    if let Some(dev) = devs.get_mut(&v.device) {
                        dev.properties.remove(name);
                    }
                }
                // Delete entire device
                None => {
                    devs.remove(&v.device);
                }
            },
            _ => {}
        }
    }
}

/// Registers a (re)defined property, creating its device on first sight.
fn define(devs: &mut HashMap<String, IndiDevice>, device: &str, name: &str, property: IndiProperty) {
    devs.entry(device.to_string())
        .or_insert_with(|| IndiDevice::new(device.to_string()))
        .properties
        .insert(name.to_string(), property);
}

/// The property a `set*Vector` updates; `None` if its device or definition is unknown.
fn defined<'a>(
    devs: &'a mut HashMap<String, IndiDevice>,
    device: &str,
    name: &str,
) -> Option<&'a mut IndiProperty> {
    devs.get_mut(device)?.properties.get_mut(name)
}

fn by_name<T: Clone>(elements: &[T], name: impl Fn(&T) -> &String) -> HashMap<String, T> {
    elements.iter().map(|el| (name(el).clone(), el.clone())).collect()
}

/// BLOBs from one property, from the moment [`IndiClient::watch_blobs`] was called.
pub struct BlobWatch {
    updates: broadcast::Receiver<IndiMessage>,
    device: String,
    property: String,
}

impl BlobWatch {
    /// The next BLOB, or `None` if none arrived within `wait`. Nothing is lost to the
    /// timeout: `broadcast::Receiver::recv` is cancel-safe.
    pub async fn next(&mut self, wait: Duration) -> Result<Option<crate::indi::xml::SetBlob>> {
        match tokio::time::timeout(wait, self.recv()).await {
            Ok(blob) => blob.map(Some),
            Err(_) => Ok(None),
        }
    }

    async fn recv(&mut self) -> Result<crate::indi::xml::SetBlob> {
        loop {
            match self.updates.recv().await {
                Ok(IndiMessage::SetBlobVector(v))
                    if v.device == self.device && v.name == self.property =>
                {
                    if let Some(blob) = v.elements.into_iter().next() {
                        return Ok(blob);
                    }
                }
                Ok(IndiMessage::Message(m)) if m.message == "Disconnected" => {
                    return Err(IndiError::Disconnected);
                }
                Err(broadcast::error::RecvError::Closed) => return Err(IndiError::Disconnected),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indi::xml::{parse_message, PropertyState, SwitchState};

    async fn apply(devices: &Arc<RwLock<HashMap<String, IndiDevice>>>, xml: &str) {
        let msg = parse_message(xml).unwrap_or_else(|e| panic!("{xml}: {e}"));
        IndiClient::update_device_state(devices, &msg).await;
    }

    /// A `set*Vector` moves only elements its `def*Vector` declared, and never creates a device.
    #[tokio::test]
    async fn a_set_vector_updates_only_what_was_defined() {
        let devices = Arc::new(RwLock::new(HashMap::new()));
        apply(&devices, r#"<defNumberVector device="CCD" name="TEMP" state="Idle"><defNumber name="T" min="-50" max="50" step="1">0</defNumber></defNumberVector>"#).await;
        apply(&devices, r#"<defSwitchVector device="CCD" name="STREAM" state="Idle" rule="OneOfMany"><defSwitch name="ON">Off</defSwitch></defSwitchVector>"#).await;

        apply(&devices, r#"<setNumberVector device="CCD" name="TEMP" state="Busy"><oneNumber name="T">-10</oneNumber><oneNumber name="U">5</oneNumber></setNumberVector>"#).await;
        apply(&devices, r#"<setSwitchVector device="CCD" name="STREAM" state="Ok"><oneSwitch name="ON">On</oneSwitch></setSwitchVector>"#).await;
        apply(&devices, r#"<setSwitchVector device="Mount" name="STREAM" state="Ok"><oneSwitch name="ON">On</oneSwitch></setSwitchVector>"#).await;
        apply(&devices, r#"<setNumberVector device="CCD" name="UNDEFINED" state="Ok"><oneNumber name="T">1</oneNumber></setNumberVector>"#).await;

        let devs = devices.read().await;
        let ccd = &devs["CCD"];
        assert_eq!(ccd.get_number("TEMP", "T").map(|n| n.value), Some(-10.0));
        assert!(ccd.get_number("TEMP", "U").is_none(), "an undeclared element was inserted");
        assert_eq!(ccd.get_property_state("TEMP"), Some(PropertyState::Busy));
        assert_eq!(ccd.get_switch("STREAM", "ON").map(|s| s.value.clone()), Some(SwitchState::On));
        assert!(!ccd.properties.contains_key("UNDEFINED"));
        assert!(!devs.contains_key("Mount"), "a set for an unknown device created it");
    }

    #[tokio::test]
    async fn del_property_removes_one_property_or_the_whole_device() {
        let devices = Arc::new(RwLock::new(HashMap::new()));
        apply(&devices, r#"<defNumberVector device="CCD" name="TEMP" state="Idle"><defNumber name="T" min="-50" max="50" step="1">0</defNumber></defNumberVector>"#).await;
        apply(&devices, r#"<defNumberVector device="CCD" name="GAIN" state="Idle"><defNumber name="G" min="0" max="500" step="1">100</defNumber></defNumberVector>"#).await;

        apply(&devices, r#"<delProperty device="CCD" name="TEMP"/>"#).await;
        apply(&devices, r#"<delProperty device="Mount" name="TEMP"/>"#).await;
        {
            let devs = devices.read().await;
            assert!(!devs["CCD"].properties.contains_key("TEMP"));
            assert!(devs["CCD"].properties.contains_key("GAIN"), "deleting one property took another");
            assert!(!devs.contains_key("Mount"));
        }

        apply(&devices, r#"<delProperty device="CCD"/>"#).await;
        assert!(devices.read().await.is_empty());
    }
}
