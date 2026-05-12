//! INDI Device

use std::collections::HashMap;

use crate::indi::xml::{
    DefBlob, DefLight, DefNumber, DefSwitch, DefText, PropertyState, SwitchRule, SwitchState,
};

#[derive(Debug, Clone)]
pub enum IndiProperty {
    Number {
        state: PropertyState,
        elements: HashMap<String, DefNumber>,
    },
    Switch {
        state: PropertyState,
        rule: SwitchRule,
        elements: HashMap<String, DefSwitch>,
    },
    Text {
        state: PropertyState,
        elements: HashMap<String, DefText>,
    },
    Light {
        state: PropertyState,
        elements: HashMap<String, DefLight>,
    },
    Blob {
        state: PropertyState,
        elements: HashMap<String, DefBlob>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct IndiDevice {
    pub name: String,
    pub properties: HashMap<String, IndiProperty>,
}

impl IndiDevice {
    pub fn new(name: String) -> Self {
        Self {
            name,
            properties: HashMap::new(),
        }
    }

    pub fn is_ccd(&self) -> bool {
        self.properties.contains_key("CCD_EXPOSURE")
    }

    pub fn is_telescope(&self) -> bool {
        self.properties.contains_key("EQUATORIAL_EOD_COORD")
    }

    pub fn get_number(&self, property_name: &str, element_name: &str) -> Option<&DefNumber> {
        if let Some(IndiProperty::Number { elements, .. }) = self.properties.get(property_name) {
            elements.get(element_name)
        } else {
            None
        }
    }

    pub fn get_switch(&self, property_name: &str, element_name: &str) -> Option<&DefSwitch> {
        if let Some(IndiProperty::Switch { elements, .. }) = self.properties.get(property_name) {
            elements.get(element_name)
        } else {
            None
        }
    }

    pub fn get_text(&self, property_name: &str, element_name: &str) -> Option<&DefText> {
        if let Some(IndiProperty::Text { elements, .. }) = self.properties.get(property_name) {
            elements.get(element_name)
        } else {
            None
        }
    }

    pub fn get_property_state(&self, property_name: &str) -> Option<PropertyState> {
        self.properties.get(property_name).map(|p| match p {
            IndiProperty::Number { state, .. } => state.clone(),
            IndiProperty::Switch { state, .. } => state.clone(),
            IndiProperty::Text { state, .. } => state.clone(),
            IndiProperty::Light { state, .. } => state.clone(),
            IndiProperty::Blob { state, .. } => state.clone(),
        })
    }
}

