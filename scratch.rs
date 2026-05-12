use quick_xml::de::from_str;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
enum IndiMessage {
    DefNumberVector { device: String },
}

fn main() {
    let xml = r#"<defNumberVector device="Test"></defNumberVector>"#;
    let res: Result<IndiMessage, _> = from_str(xml);
    println!("{:?}", res);
}
