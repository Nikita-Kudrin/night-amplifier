use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PropertyState {
    Idle,
    Ok,
    Busy,
    Alert,
}

impl Default for PropertyState {
    fn default() -> Self {
        PropertyState::Idle
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SwitchRule {
    OneOfMany,
    AtMostOne,
    AnyOfMany,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SwitchState {
    On,
    Off,
}

// --- Incoming Messages (Definitions and Updates) ---

#[derive(Debug, Clone, Deserialize)]
pub struct DefNumber {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@label", default)]
    pub label: String,
    #[serde(rename = "@format", default)]
    pub format: String,
    #[serde(rename = "@min")]
    pub min: f64,
    #[serde(rename = "@max")]
    pub max: f64,
    #[serde(rename = "@step")]
    pub step: f64,
    #[serde(rename = "$value")]
    pub value: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefNumberVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "defNumber", default)]
    pub elements: Vec<DefNumber>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetNumber {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$value")]
    pub value: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetNumberVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "oneNumber", default)]
    pub elements: Vec<SetNumber>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefSwitch {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@label", default)]
    pub label: String,
    #[serde(rename = "$value")]
    pub value: SwitchState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefSwitchVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "@rule", default = "default_switch_rule")]
    pub rule: SwitchRule,
    #[serde(rename = "defSwitch", default)]
    pub elements: Vec<DefSwitch>,
}

fn default_switch_rule() -> SwitchRule {
    SwitchRule::OneOfMany
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetSwitch {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$value")]
    pub value: SwitchState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetSwitchVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "oneSwitch", default)]
    pub elements: Vec<SetSwitch>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefText {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@label", default)]
    pub label: String,
    #[serde(rename = "$value")]
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefTextVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "defText", default)]
    pub elements: Vec<DefText>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetText {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$value", default)]
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetTextVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "oneText", default)]
    pub elements: Vec<SetText>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefLight {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@label", default)]
    pub label: String,
    #[serde(rename = "$value")]
    pub value: PropertyState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefLightVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "defLight", default)]
    pub elements: Vec<DefLight>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetLight {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$value")]
    pub value: PropertyState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetLightVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "oneLight", default)]
    pub elements: Vec<SetLight>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefBlob {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@label", default)]
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefBlobVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "defBLOB", default)]
    pub elements: Vec<DefBlob>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetBlob {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@size")]
    pub size: usize,
    #[serde(rename = "@format")]
    pub format: String,
    #[serde(rename = "$value")]
    pub value: String, // base64 encoded
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetBlobVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@state", default)]
    pub state: PropertyState,
    #[serde(rename = "oneBLOB", default)]
    pub elements: Vec<SetBlob>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    #[serde(rename = "@device", default)]
    pub device: Option<String>,
    #[serde(rename = "@message", default)]
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DelProperty {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name", default)]
    pub name: Option<String>,
}

// Wrapping Enum for all incoming messages
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndiMessage {
    DefNumberVector(DefNumberVector),
    SetNumberVector(SetNumberVector),
    DefSwitchVector(DefSwitchVector),
    SetSwitchVector(SetSwitchVector),
    DefTextVector(DefTextVector),
    SetTextVector(SetTextVector),
    DefLightVector(DefLightVector),
    SetLightVector(SetLightVector),
    #[serde(rename = "defBLOBVector")]
    DefBlobVector(DefBlobVector),
    #[serde(rename = "setBLOBVector")]
    SetBlobVector(SetBlobVector),
    Message(Message),
    DelProperty(DelProperty),
}

// --- Outgoing Messages ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename = "getProperties")]
pub struct GetProperties {
    #[serde(rename = "@version")]
    pub version: String,
    #[serde(rename = "@device", skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(rename = "@name", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewNumber {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$value")]
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename = "newNumberVector")]
pub struct NewNumberVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "oneNumber")]
    pub elements: Vec<NewNumber>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewSwitch {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$value")]
    pub value: SwitchState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename = "newSwitchVector")]
pub struct NewSwitchVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "oneSwitch")]
    pub elements: Vec<NewSwitch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewText {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "$value")]
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename = "newTextVector")]
pub struct NewTextVector {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "oneText")]
    pub elements: Vec<NewText>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BlobEnable {
    Never,
    Also,
    Only,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename = "enableBLOB")]
pub struct EnableBlob {
    #[serde(rename = "@device")]
    pub device: String,
    #[serde(rename = "@name", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "$value")]
    pub value: BlobEnable,
}

// Helper to parse an XML string into an IndiMessage
pub fn parse_message(xml: &str) -> Result<IndiMessage, quick_xml::DeError> {
    quick_xml::de::from_str(xml)
}

// Helper to serialize an outgoing message
pub fn serialize_message<T: Serialize>(msg: &T) -> Result<String, quick_xml::SeError> {
    quick_xml::se::to_string(msg)
}
