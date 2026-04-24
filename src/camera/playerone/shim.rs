use super::ffi_types::*;
use super::sdk::PlayerOneSdk;
use std::os::raw::c_long;

pub struct CameraDescription {
    properties: POACameraProperties,
}

impl CameraDescription {
    pub fn properties(&self) -> &POACameraProperties {
        &self.properties
    }

    pub fn open(&self) -> Result<Camera, String> {
        let sdk = PlayerOneSdk::try_load().ok_or("SDK not loaded")?;
        let err = unsafe { sdk.api.POAOpenCamera(self.properties.cameraID) };
        if err != POAErrors::POA_OK {
            return Err(format!("POAOpenCamera failed: {:?}", err));
        }
        let err = unsafe { sdk.api.POAInitCamera(self.properties.cameraID) };
        if err != POAErrors::POA_OK {
            unsafe {
                sdk.api.POACloseCamera(self.properties.cameraID);
            }
            return Err(format!("POAInitCamera failed: {:?}", err));
        }
        Ok(Camera {
            id: self.properties.cameraID,
        })
    }
}

pub struct Camera {
    id: i32,
}

impl Camera {
    pub fn all_cameras() -> Vec<CameraDescription> {
        let Some(sdk) = PlayerOneSdk::try_load() else {
            return vec![];
        };
        let count = unsafe { sdk.api.POAGetCameraCount() };
        let mut cameras = Vec::new();
        for i in 0..count {
            let mut props = POACameraProperties::default();
            if unsafe { sdk.api.POAGetCameraProperties(i, &mut props) } == POAErrors::POA_OK {
                cameras.push(CameraDescription { properties: props });
            }
        }
        cameras
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    fn get_config(&self, conf_id: POAConfig) -> Result<(POAConfigValue, bool), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let mut val = POAConfigValue::default();
        let mut is_auto = POABool::POA_FALSE;
        let err = unsafe {
            sdk.api
                .POAGetConfig(self.id, conf_id, &mut val, &mut is_auto)
        };
        if err == POAErrors::POA_OK {
            Ok((val, is_auto == POABool::POA_TRUE))
        } else {
            Err(format!("POAGetConfig failed: {:?}", err))
        }
    }

    fn set_config(
        &self,
        conf_id: POAConfig,
        val: POAConfigValue,
        is_auto: bool,
    ) -> Result<(), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let auto_val = if is_auto {
            POABool::POA_TRUE
        } else {
            POABool::POA_FALSE
        };
        let err = unsafe { sdk.api.POASetConfig(self.id, conf_id, val, auto_val) };
        if err == POAErrors::POA_OK {
            Ok(())
        } else {
            Err(format!("POASetConfig failed: {:?}", err))
        }
    }

    pub fn temperature(&self) -> Result<f32, String> {
        self.get_config(POAConfig::POA_TEMPERATURE)
            .map(|(v, _)| unsafe { v.floatValue } as f32)
    }

    pub fn gain(&self) -> Result<(i64, bool), String> {
        self.get_config(POAConfig::POA_GAIN)
            .map(|(v, auto)| (unsafe { v.intValue } as i64, auto))
    }

    pub fn offset(&self) -> Result<i64, String> {
        self.get_config(POAConfig::POA_OFFSET)
            .map(|(v, _)| unsafe { v.intValue } as i64)
    }

    pub fn exposure(&self) -> Result<(i64, bool), String> {
        self.get_config(POAConfig::POA_EXPOSURE)
            .map(|(v, auto)| (unsafe { v.intValue } as i64, auto))
    }

    pub fn cooler_power(&self) -> Result<i64, String> {
        self.get_config(POAConfig::POA_COOLER_POWER)
            .map(|(v, _)| unsafe { v.intValue } as i64)
    }

    pub fn cooler(&self) -> Result<bool, String> {
        self.get_config(POAConfig::POA_COOLER)
            .map(|(v, _)| unsafe { v.boolValue } == POABool::POA_TRUE)
    }

    pub fn dew_heater_power(&self) -> Result<i64, String> {
        self.get_config(POAConfig::POA_HEATER_POWER)
            .map(|(v, _)| unsafe { v.intValue } as i64)
    }

    pub fn is_config_supported(&self, conf_id: POAConfig) -> bool {
        self.get_config(conf_id).is_ok()
    }

    pub fn set_target_temperature(&mut self, temp: i64) -> Result<(), String> {
        self.set_config(
            POAConfig::POA_TARGET_TEMP,
            POAConfigValue {
                intValue: temp as std::os::raw::c_long,
            },
            false,
        )
    }

    pub fn set_cooler(&mut self, enabled: bool) -> Result<(), String> {
        self.set_config(
            POAConfig::POA_COOLER,
            POAConfigValue {
                boolValue: if enabled {
                    POABool::POA_TRUE
                } else {
                    POABool::POA_FALSE
                },
            },
            false,
        )
    }

    pub fn set_dew_heater_power(&mut self, power: i64) -> Result<(), String> {
        self.set_config(
            POAConfig::POA_HEATER_POWER,
            POAConfigValue {
                intValue: power as std::os::raw::c_long,
            },
            false,
        )
    }

    pub fn set_exposure(&mut self, exp: i64, auto: bool) -> Result<(), String> {
        self.set_config(
            POAConfig::POA_EXPOSURE,
            POAConfigValue {
                intValue: exp as std::os::raw::c_long,
            },
            auto,
        )
    }

    pub fn set_gain(&mut self, gain: i64, auto: bool) -> Result<(), String> {
        self.set_config(
            POAConfig::POA_GAIN,
            POAConfigValue {
                intValue: gain as std::os::raw::c_long,
            },
            auto,
        )
    }

    pub fn set_offset(&mut self, offset: i64) -> Result<(), String> {
        self.set_config(
            POAConfig::POA_OFFSET,
            POAConfigValue {
                intValue: offset as std::os::raw::c_long,
            },
            false,
        )
    }

    pub fn set_bin(&mut self, bin: u32) -> Result<(), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let err = unsafe { sdk.api.POASetImageBin(self.id, bin as i32) };
        if err == POAErrors::POA_OK {
            Ok(())
        } else {
            Err(format!("POASetImageBin failed: {:?}", err))
        }
    }

    pub fn set_image_format(&mut self, format: POAImgFormat) -> Result<(), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let err = unsafe { sdk.api.POASetImageFormat(self.id, format) };
        if err == POAErrors::POA_OK {
            Ok(())
        } else {
            Err(format!("POASetImageFormat failed: {:?}", err))
        }
    }

    pub fn set_roi(&mut self, roi: &crate::camera::playerone::capture::ROI) -> Result<(), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let err = unsafe {
            sdk.api
                .POASetImageStartPos(self.id, roi.start_x as i32, roi.start_y as i32)
        };
        if err != POAErrors::POA_OK {
            return Err(format!("POASetImageStartPos failed: {:?}", err));
        }

        let err = unsafe {
            sdk.api
                .POASetImageSize(self.id, roi.width as i32, roi.height as i32)
        };
        if err == POAErrors::POA_OK {
            Ok(())
        } else {
            Err(format!("POASetImageSize failed: {:?}", err))
        }
    }

    pub fn start_exposure(&mut self) -> Result<(), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let err = unsafe { sdk.api.POAStartExposure(self.id, POABool::POA_TRUE) };
        if err == POAErrors::POA_OK {
            Ok(())
        } else {
            Err(format!("POAStartExposure failed: {:?}", err))
        }
    }

    pub fn stop_exposure(&mut self) -> Result<(), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let err = unsafe { sdk.api.POAStopExposure(self.id) };
        if err == POAErrors::POA_OK {
            Ok(())
        } else {
            Err(format!("POAStopExposure failed: {:?}", err))
        }
    }

    pub fn is_image_ready(&self) -> Result<bool, String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let mut ready = POABool::POA_FALSE;
        let err = unsafe { sdk.api.POAImageReady(self.id, &mut ready) };
        if err == POAErrors::POA_OK {
            Ok(ready == POABool::POA_TRUE)
        } else {
            Err(format!("POAImageReady failed: {:?}", err))
        }
    }

    pub fn get_image_data(&self, buf: &mut [u8], timeout_ms: Option<i32>) -> Result<(), String> {
        let sdk = PlayerOneSdk::try_load().unwrap();
        let timeout = timeout_ms.unwrap_or(-1);
        let err = unsafe {
            sdk.api
                .POAGetImageData(self.id, buf.as_mut_ptr(), buf.len() as c_long, timeout)
        };
        if err == POAErrors::POA_OK {
            Ok(())
        } else {
            Err(format!("POAGetImageData failed: {:?}", err))
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        if let Some(sdk) = PlayerOneSdk::try_load() {
            unsafe {
                sdk.api.POACloseCamera(self.id);
            }
        }
    }
}
