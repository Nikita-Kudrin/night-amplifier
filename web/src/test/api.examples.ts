/**
 * One event of each type, generated from `ServerEvent::examples()` by `tests/api_types.rs`, for
 * tests only. Do not edit: change the Rust, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */

import type {EventOf, ServerEventType} from '../composables/api.events'

/**
 * Each event with only the fields it requires, a nullable one `null`: complete, yet saying
 * nothing a test did not ask for.
 */
export const SERVER_EVENT_EXAMPLES: {readonly [K in ServerEventType]: EventOf<K>} = {
    state_changed: {"type":"state_changed","state":"Capturing"},
    frame_captured: {"type":"frame_captured","frame_number":42,"stacked_count":40,"rejected_count":2},
    frame_rejected: {"type":"frame_rejected","frame_number":43,"stacked_count":40,"rejected_count":3,"reason":"camera timeout"},
    settings_updated: {"type":"settings_updated"},
    camera_connected: {"type":"camera_connected","name":"ASI533MC Pro"},
    camera_disconnected: {"type":"camera_disconnected","name":"ASI533MC Pro"},
    camera_discovered: {"type":"camera_discovered","camera":{"id":"zwo_0","name":"ASI533MC Pro","connected":false,"provider":"ZWO","index":0,"info":{"id":"zwo_0","name":"ASI533MC Pro","max_width":3008,"max_height":3008,"pixel_size_x_um":3.76,"pixel_size_y_um":3.76,"sensor_type":"Color","has_cooler":true,"has_dew_heater":true,"min_temp_c":-30.0,"max_temp_c":30.0,"bit_depth":14,"min_exposure_us":32,"max_exposure_us":2000000000,"min_gain":0,"max_gain":450}}},
    camera_status_updated: {"type":"camera_status_updated","name":"ASI533MC Pro","temperature_c":-9.8,"cooler_on":true,"dew_heater_on":false},
    camera_phase_changed: {"type":"camera_phase_changed","name":"ASI533MC Pro","role":"main","phase":"warming_up"},
    camera_phases: {"type":"camera_phases","cameras":[]},
    viewed_camera_changed: {"type":"viewed_camera_changed","camera":"guide"},
    stack_crop_changed: {"type":"stack_crop_changed"},
    camera_persistently_unresponsive: {"type":"camera_persistently_unresponsive","name":"ASI533MC Pro","consecutive_timeouts":3},
    camera_reconnecting: {"type":"camera_reconnecting","name":"ASI533MC Pro","role":"main","attempt":1,"of":5,"next_attempt_in_s":2},
    camera_reconnect_failed: {"type":"camera_reconnect_failed","name":"ASI533MC Pro","role":"main","attempts":5,"reason":"unplugged"},
    capture_resumed: {"type":"capture_resumed","name":"ASI533MC Pro","stacked_count":40},
    camera_recovery_cleared: {"type":"camera_recovery_cleared","role":"main"},
    focus_mode_left: {"type":"focus_mode_left"},
    ai_compute_changed: {"type":"ai_compute_changed"},
    error: {"type":"error","message":"Camera lost"},
    disk_writer_warning: {"type":"disk_writer_warning","queue_depth":40},
    disk_writer_warning_cleared: {"type":"disk_writer_warning_cleared"},
    storage_sync: {"type":"storage_sync","role":"main","state":"synced","files":166,"failed":0,"error":null},
    frame_dropped: {"type":"frame_dropped","dropped_count":3,"delivered_count":120},
    capture_counts: {"type":"capture_counts","frame_count":43,"stacked_count":40,"rejected_count":3,"dropped_count":3,"delivered_count":120,"last_rejection_reason":null},
    warning: {"type":"warning","message":"Disk nearly full"},
    synced: {"type":"synced","cause":"lagged","dropped":144},
    plate_solving_started: {"type":"plate_solving_started","target_name":null},
    plate_solving_progress: {"type":"plate_solving_progress","stage":"blind","attempt":2,"total":3},
    position_solved: {"type":"position_solved","ra_degrees":10.68,"dec_degrees":41.27,"ra_string":"00h 42m 44s","dec_string":"+41° 16′ 09″","stars_detected":null,"confidence":0.98,"rotation_deg":12.5},
    position_solve_failed: {"type":"position_solve_failed","reason":"too few stars"},
    plate_solving_cancelled: {"type":"plate_solving_cancelled"},
    plate_solving_restarted: {"type":"plate_solving_restarted","reason":"binning changed"},
    push_to_blocked: {"type":"push_to_blocked","reason":null},
    push_direction_updated: {"type":"push_direction_updated","angle_deg":37.0,"distance_deg":2.6,"direction_hint":"NE","is_close":false,"fov_deg":null,"solved_camera":null,"stale":false},
    push_direction_cleared: {"type":"push_direction_cleared"},
    plate_solving_status: {"type":"plate_solving_status","in_progress":true,"target_name":null,"stage":null,"last_result":null},
    target_changed: {"type":"target_changed","name":null,"designation":null,"ra_degrees":10.68,"dec_degrees":41.27},
    target_cleared: {"type":"target_cleared"},
    astap_install_starting: {"type":"astap_install_starting","component":"astap"},
    astap_install_progress: {"type":"astap_install_progress","component":"astap","bytes_downloaded":1024,"total_bytes":null,"percent":null,"stage_id":null,"overall_percent":null},
    astap_install_extracting: {"type":"astap_install_extracting","component":"astap","progress":50.0,"stage_id":null,"overall_percent":null},
    astap_install_completed: {"type":"astap_install_completed","component":"astap","stage_id":null,"overall_percent":null},
    astap_install_failed: {"type":"astap_install_failed","component":"astap","error":"disk full"},
    catalog_install_starting: {"type":"catalog_install_starting"},
    catalog_install_progress: {"type":"catalog_install_progress","file_name":"d50.1476","bytes_downloaded":1024,"total_bytes":null,"percent":null},
    catalog_file_completed: {"type":"catalog_file_completed","file_name":"d50.1476"},
    catalog_install_completed: {"type":"catalog_install_completed","object_count":13000},
    catalog_install_failed: {"type":"catalog_install_failed","error":"disk full"},
    astap_install_status: {"type":"astap_install_status","state":"completed"},
    catalog_install_status: {"type":"catalog_install_status","state":"running","progress":null},
}
