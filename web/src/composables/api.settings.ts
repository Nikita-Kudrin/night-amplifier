/**
 * The server's default settings and the ranges it clamps settings to, generated from the
 * Rust code that enforces them by `tests/api_types.rs`. Do not edit: change the Rust, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */

import type {Settings} from './api.types'

export const SERVER_DEFAULT_SETTINGS = {
  "exposure_us": 1000000,
  "gain": 0,
  "offset": 10,
  "bin": 1,
  "auto_stretch": true,
  "stacking": true,
  "rejection_sigma": 2.5,
  "rejection_method": "None",
  "background_subtraction": true,
  "background_extraction_algorithm": "grid_bilinear",
  "raw_frame_saving": {
    "live_view": false,
    "wanderer": false,
    "stacking": false,
    "guide": false
  },
  "save_stacked_image": false,
  "stacking_type": "deep_sky",
  "weighting_preset": "balanced",
  "stretch_aggressiveness": "medium",
  "auto_stretch_intensity": 0.3,
  "saturation_boost": false,
  "saturation_boost_strength": 0.5,
  "use_simulated_camera": false,
  "simulated_preload_images": 5,
  "show_focus_image": true,
  "force_focus_image_now": false,
  "cooler_enabled": false,
  "cooler_fast_mode": false,
  "planetary_auto_tracking": true,
  "planetary_multi_point_alignment": false,
  "dew_heater_enabled": true,
  "dew_heater_power": 10,
  "wanderer_mode": false,
  "auto_crop": false,
  "auto_reconnect": true,
  "auto_resume_capture": true,
  "sensor_correction": {
    "hot_pixel_sigma": 5.0,
    "fpn_removal": true,
    "superpixel_debayer": false
  },
  "denoise": {
    "enabled": true,
    "chroma": true,
    "chroma_strength": 1.0,
    "background_grain": 0.5,
    "luma_strength": 1.0,
    "detail": 0.5,
    "ai": false,
    "ai_compute": "auto"
  },
  "preview_resolution": "native",
  "streaming_resolution": "native",
  "eyepiece": {
    "binoview": true,
    "screen_width": 140.0,
    "screen_height": 67.0,
    "screen_measurement": "mm",
    "screen_resolution_x": 2880,
    "screen_resolution_y": 1440,
    "circular_view": true,
    "intensity": 0.3,
    "black_floor": 0.04,
    "darker_sky": false,
    "dither": true,
    "stream_resolution": "native"
  },
  "telescope": {
    "focal_length_mm": null,
    "pixel_size_x_um": null,
    "pixel_size_y_um": null,
    "sensor_width_px": null,
    "sensor_height_px": null,
    "barlow_coeff": null
  },
  "guide_camera": {
    "exposure_us": 1000000,
    "gain": 0,
    "offset": 10,
    "bin": 1,
    "cooler_enabled": false,
    "target_temp_c": null,
    "sensor_mode_override": null,
    "cooler_fast_mode": false,
    "dew_heater_enabled": true,
    "dew_heater_power": 10
  },
  "eula_accepted": false,
  "indi_server_host": "127.0.0.1",
  "indi_server_port": 7624,
  "focus_mode": false
} satisfies Settings

/**
 * The range the server clamps each ranged setting to, by its dotted path in `Settings`;
 * `null` is an open end.
 */
export const SERVER_SETTING_RANGES = {
  "rejection_sigma": {
    "min": 0.5,
    "max": 10.0
  },
  "auto_stretch_intensity": {
    "min": 0.0,
    "max": 1.0
  },
  "saturation_boost_strength": {
    "min": 0.0,
    "max": 1.0
  },
  "simulated_preload_images": {
    "min": 1,
    "max": null
  },
  "target_temp_c": {
    "min": -60.0,
    "max": 30.0
  },
  "dew_heater_power": {
    "min": 0,
    "max": 100
  },
  "guide_camera.target_temp_c": {
    "min": -60.0,
    "max": 30.0
  },
  "guide_camera.dew_heater_power": {
    "min": 0,
    "max": 100
  },
  "sensor_correction.hot_pixel_sigma": {
    "min": 3.0,
    "max": 12.0
  },
  "denoise.chroma_strength": {
    "min": 0.0,
    "max": 1.0
  },
  "denoise.background_grain": {
    "min": 0.0,
    "max": 1.0
  },
  "denoise.luma_strength": {
    "min": 0.0,
    "max": 1.0
  },
  "denoise.detail": {
    "min": 0.0,
    "max": 1.0
  },
  "eyepiece.black_floor": {
    "min": -0.045,
    "max": 0.5
  }
} as const satisfies Record<string, {min: number | null; max: number | null}>

/** A setting the server clamps, by its dotted path in `Settings`. */
export type ClampedPath = keyof typeof SERVER_SETTING_RANGES

/** The longest edge the server streams a frame at, Native included (`MAX_STREAM_EDGE`). */
export const SERVER_MAX_STREAM_EDGE = 3840
