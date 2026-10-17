use serde::{Deserialize, Serialize};

/// Spelled as the INDI protocol spells it, `Idle`/`Ok`/`Busy`/`Alert`: a lower-case
/// spelling once failed every real `def*Vector`, so no device was ever discovered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[derive(Default)]
pub enum PropertyState {
    #[default]
    Idle,
    Ok,
    Busy,
    Alert,
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
//
// An enum-valued element (a switch, a light) reads its value from `$text`: under
// `$value` quick-xml expects a child element and rejected every real switch vector.

/// A value a driver wrote on a line of its own, as indiserver does: `\nOff\n    `.
fn trimmed<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let text = String::deserialize(deserializer)?;
    T::deserialize(serde::de::value::StrDeserializer::<D::Error>::new(text.trim()))
}

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
    #[serde(rename = "$text", deserialize_with = "trimmed")]
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
    #[serde(rename = "$text", deserialize_with = "trimmed")]
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
    #[serde(rename = "$text", deserialize_with = "trimmed")]
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
    #[serde(rename = "$text", deserialize_with = "trimmed")]
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
    /// `$text`, not `$value`: an enum in `$value` serialises as an element, `<On/>`,
    /// which no driver reads.
    #[serde(rename = "$text")]
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
    #[serde(rename = "$text")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drivers_property_state_parses() {
        let xml = r#"<defNumberVector device="CCD Simulator" name="CCD_TEMPERATURE" state="Busy"><defNumber name="CCD_TEMPERATURE_VALUE" min="-50" max="50" step="0.1">-9.5</defNumber></defNumberVector>"#;
        let Ok(IndiMessage::DefNumberVector(vector)) = parse_message(xml) else {
            panic!("{:?}", parse_message(xml));
        };
        assert_eq!(vector.state, PropertyState::Busy);
        assert_eq!(vector.elements[0].value, -9.5);
    }

    /// A driver indents its values; switches and lights are text, not elements.
    #[test]
    fn a_drivers_switch_and_light_vectors_parse() {
        let xml = r#"<defSwitchVector device="CCD" name="CCD_VIDEO_STREAM" state="Idle" rule="OneOfMany">
    <defSwitch name="STREAM_ON" label="Stream On">
Off
    </defSwitch>
    <defSwitch name="STREAM_OFF" label="Stream Off">
On
    </defSwitch>
</defSwitchVector>"#;
        let Ok(IndiMessage::DefSwitchVector(vector)) = parse_message(xml) else {
            panic!("{:?}", parse_message(xml));
        };
        assert_eq!(vector.elements[0].value, SwitchState::Off);
        assert_eq!(vector.elements[1].value, SwitchState::On);

        let xml = r#"<setLightVector device="CCD" name="STATUS" state="Ok"><oneLight name="LINK">Alert</oneLight></setLightVector>"#;
        let Ok(IndiMessage::SetLightVector(vector)) = parse_message(xml) else {
            panic!("{:?}", parse_message(xml));
        };
        assert_eq!(vector.elements[0].value, PropertyState::Alert);

        let xml = r#"<defNumberVector device="CCD" name="CCD_EXPOSURE" state="Idle">
    <defNumber name="CCD_EXPOSURE_VALUE" min="0.01" max="3600" step="1">
1.5
    </defNumber>
</defNumberVector>"#;
        let Ok(IndiMessage::DefNumberVector(vector)) = parse_message(xml) else {
            panic!("{:?}", parse_message(xml));
        };
        assert_eq!(vector.elements[0].value, 1.5);
    }

    /// Element values go out as the protocol's text: `On`, `Also`.
    #[test]
    fn switches_and_blob_rules_are_sent_as_text() {
        let switch = NewSwitchVector {
            device: "CCD".into(),
            name: "CCD_VIDEO_STREAM".into(),
            elements: vec![NewSwitch {
                name: "STREAM_ON".into(),
                value: SwitchState::On,
            }],
        };
        assert_eq!(
            serialize_message(&switch).unwrap(),
            r#"<newSwitchVector device="CCD" name="CCD_VIDEO_STREAM"><oneSwitch name="STREAM_ON">On</oneSwitch></newSwitchVector>"#
        );
        let blobs = EnableBlob {
            device: "CCD".into(),
            name: Some("CCD1".into()),
            value: BlobEnable::Also,
        };
        assert_eq!(
            serialize_message(&blobs).unwrap(),
            r#"<enableBLOB device="CCD" name="CCD1">Also</enableBLOB>"#
        );
    }
}
