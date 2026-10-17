#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::{c_char, c_int, c_long, c_uchar, c_uint};

#[repr(C)]
pub struct SVB_CAMERA_INFO {
    pub FriendlyName: [c_char; 32],
    pub CameraSN: [c_char; 32],
    pub PortType: [c_char; 32],
    pub DeviceID: c_uint,
    pub CameraID: c_int,
}

#[repr(C)]
pub struct SVB_CAMERA_PROPERTY {
    pub MaxHeight: c_long,
    pub MaxWidth: c_long,
    pub IsColorCam: c_int,
    pub BayerPattern: c_int,
    pub SupportedBins: [c_int; 16],
    pub SupportedVideoFormat: [c_int; 8],
    pub MaxBitDepth: c_int,
    pub IsTriggerCam: c_int,
}

#[repr(C)]
pub struct SVB_CAMERA_PROPERTY_EX {
    pub bSupportPulseGuide: c_int,
    pub bSupportControlTemp: c_int,
    pub Unused: [c_int; 64],
}

#[repr(C)]
pub struct SVB_CONTROL_CAPS {
    pub Name: [c_char; 64],
    pub Description: [c_char; 128],
    pub MaxValue: c_long,
    pub MinValue: c_long,
    pub DefaultValue: c_long,
    pub IsAutoSupported: c_int,
    pub IsWritable: c_int,
    pub ControlType: c_int,
    pub Unused: [c_char; 32],
}

#[repr(C)]
pub struct SVB_ID {
    pub id: [c_uchar; 64],
}

#[repr(C)]
pub struct SVB_SUPPORTED_MODE {
    pub SupportedCameraMode: [c_int; 16],
}

pub const SVB_SUCCESS: c_int = 0;
pub const SVB_ERROR_INVALID_INDEX: c_int = 1;
pub const SVB_ERROR_INVALID_ID: c_int = 2;
pub const SVB_ERROR_INVALID_CONTROL_TYPE: c_int = 3;
pub const SVB_ERROR_CAMERA_CLOSED: c_int = 4;
pub const SVB_ERROR_CAMERA_REMOVED: c_int = 5;
pub const SVB_ERROR_INVALID_PATH: c_int = 6;
pub const SVB_ERROR_INVALID_FILEFORMAT: c_int = 7;
pub const SVB_ERROR_INVALID_SIZE: c_int = 8;
pub const SVB_ERROR_INVALID_IMGTYPE: c_int = 9;
pub const SVB_ERROR_OUTOF_BOUNDARY: c_int = 10;
pub const SVB_ERROR_TIMEOUT: c_int = 11;
pub const SVB_ERROR_INVALID_SEQUENCE: c_int = 12;
pub const SVB_ERROR_BUFFER_TOO_SMALL: c_int = 13;
pub const SVB_ERROR_VIDEO_MODE_ACTIVE: c_int = 14;
pub const SVB_ERROR_EXPOSURE_IN_PROGRESS: c_int = 15;
pub const SVB_ERROR_GENERAL_ERROR: c_int = 16;
pub const SVB_ERROR_INVALID_MODE: c_int = 17;
pub const SVB_ERROR_INVALID_DIRECTION: c_int = 18;
pub const SVB_ERROR_UNKNOW_SENSOR_TYPE: c_int = 19;
pub const SVB_ERROR_END: c_int = 20;

pub const SVB_IMG_RAW8: c_int = 0;
pub const SVB_IMG_RAW10: c_int = 1;
pub const SVB_IMG_RAW12: c_int = 2;
pub const SVB_IMG_RAW14: c_int = 3;
pub const SVB_IMG_RAW16: c_int = 4;
pub const SVB_IMG_Y8: c_int = 5;
pub const SVB_IMG_Y10: c_int = 6;
pub const SVB_IMG_Y12: c_int = 7;
pub const SVB_IMG_Y14: c_int = 8;
pub const SVB_IMG_Y16: c_int = 9;
pub const SVB_IMG_RGB24: c_int = 10;
pub const SVB_IMG_RGB32: c_int = 11;
pub const SVB_IMG_END: c_int = -1;

pub const SVB_BAYER_RG: c_int = 0;
pub const SVB_BAYER_BG: c_int = 1;
pub const SVB_BAYER_GR: c_int = 2;
pub const SVB_BAYER_GB: c_int = 3;

pub const SVB_GAIN: c_int = 0;
pub const SVB_EXPOSURE: c_int = 1;
pub const SVB_GAMMA: c_int = 2;
pub const SVB_GAMMA_CONTRAST: c_int = 3;
pub const SVB_WB_R: c_int = 4;
pub const SVB_WB_G: c_int = 5;
pub const SVB_WB_B: c_int = 6;
pub const SVB_FLIP: c_int = 7;
pub const SVB_FRAME_SPEED_MODE: c_int = 8;
pub const SVB_CONTRAST: c_int = 9;
pub const SVB_SHARPNESS: c_int = 10;
pub const SVB_SATURATION: c_int = 11;
pub const SVB_AUTO_TARGET_BRIGHTNESS: c_int = 12;
pub const SVB_BLACK_LEVEL: c_int = 13;
pub const SVB_COOLER_ENABLE: c_int = 14;
pub const SVB_TARGET_TEMPERATURE: c_int = 15;
pub const SVB_CURRENT_TEMPERATURE: c_int = 16;
pub const SVB_COOLER_POWER: c_int = 17;
pub const SVB_BAD_PIXEL_CORRECTION_ENABLE: c_int = 18;
pub const SVB_BAD_PIXEL_CORRECTION_THRESHOLD: c_int = 19;

pub const SVB_MODE_NORMAL: c_int = 0;
pub const SVB_MODE_TRIG_SOFT: c_int = 1;
pub const SVB_MODE_TRIG_RISE_EDGE: c_int = 2;
pub const SVB_MODE_TRIG_FALL_EDGE: c_int = 3;
pub const SVB_MODE_TRIG_DOUBLE_EDGE: c_int = 4;
pub const SVB_MODE_TRIG_HIGH_LEVEL: c_int = 5;
pub const SVB_MODE_TRIG_LOW_LEVEL: c_int = 6;
pub const SVB_MODE_END: c_int = -1;

pub const SVB_GUIDE_NORTH: c_int = 0;
pub const SVB_GUIDE_SOUTH: c_int = 1;
pub const SVB_GUIDE_EAST: c_int = 2;
pub const SVB_GUIDE_WEST: c_int = 3;

pub const SVB_FLIP_NONE: c_int = 0;
pub const SVB_FLIP_HORIZ: c_int = 1;
pub const SVB_FLIP_VERT: c_int = 2;
pub const SVB_FLIP_BOTH: c_int = 3;

pub const SVB_TRIG_OUTPUT_PINA: c_int = 0;
pub const SVB_TRIG_OUTPUT_PINB: c_int = 1;
pub const SVB_TRIG_OUTPUT_NONE: c_int = -1;

pub const SVB_EXP_IDLE: c_int = 0;
pub const SVB_EXP_WORKING: c_int = 1;
pub const SVB_EXP_SUCCESS: c_int = 2;
pub const SVB_EXP_FAILED: c_int = 3;

pub const SVB_FALSE: c_int = 0;
pub const SVB_TRUE: c_int = 1;
