use thiserror::Error;

#[derive(Error, Debug)]
pub enum IndiError {
    #[error("Network error: {0}")]
    Network(#[from] std::io::Error),
    #[error("XML parsing error: {0}")]
    Xml(#[from] quick_xml::DeError),
    #[error("XML serialization error: {0}")]
    XmlSerialize(#[from] quick_xml::SeError),
    #[error("Connection disconnected")]
    Disconnected,
    #[error("Device not found: {0}")]
    DeviceNotFound(String),
    #[error("Property not found: {0}")]
    PropertyNotFound(String),
    #[error("Timeout waiting for {0}")]
    Timeout(String),
}

pub type Result<T> = std::result::Result<T, IndiError>;
