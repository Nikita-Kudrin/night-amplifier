/**
 * The events `/ws/events` sends: a JSDoc type of each, and the topic each states, generated
 * from the Rust `ServerEvent` by `tests/api_types.rs`. Do not edit: change the Rust, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */

/**
 * Capture state changed
 * @typedef {object} StateChangedEvent
 * @property {'state_changed'} type
 * @property {CaptureState} state
 */

/**
 * New frame captured
 * @typedef {object} FrameCapturedEvent
 * @property {'frame_captured'} type
 * @property {number} frame_number
 * @property {number} stacked_count
 * @property {number} rejected_count
 * @property {string|null} [rejection_reason] - Why this frame did not join the stack, when it did not.
 */

/**
 * Frame was rejected
 * @typedef {object} FrameRejectedEvent
 * @property {'frame_rejected'} type
 * @property {number} frame_number
 * @property {number} stacked_count
 * @property {number} rejected_count
 * @property {string} reason
 */

/**
 * Settings were updated
 * @typedef {object} SettingsUpdatedEvent
 * @property {'settings_updated'} type
 */

/**
 * Camera connected
 * @typedef {object} CameraConnectedEvent
 * @property {'camera_connected'} type
 * @property {string} name
 */

/**
 * Camera disconnected
 * @typedef {object} CameraDisconnectedEvent
 * @property {'camera_disconnected'} type
 * @property {string} name
 */

/**
 * Camera discovered asynchronously (e.g. INDI)
 * @typedef {object} CameraDiscoveredEvent
 * @property {'camera_discovered'} type
 * @property {Camera} camera
 */

/**
 * Cooled camera status sample (sensor temperature, cooler power, cooler state)
 * @typedef {object} CameraStatusUpdatedEvent
 * @property {'camera_status_updated'} type
 * @property {string} name
 * @property {number} temperature_c
 * @property {number|null} [cooler_power]
 * @property {boolean} cooler_on
 * @property {boolean} dew_heater_on
 * @property {number|null} [target_temp_c]
 */

/**
 * Camera lifecycle phase changed (Precooling, Idle, Capturing, WarmingUp, Disconnected). `role` tells two bodies of one model apart, which `name` cannot.
 * @typedef {object} CameraPhaseChangedEvent
 * @property {'camera_phase_changed'} type
 * @property {string} name
 * @property {CameraRole} role
 * @property {CameraPhase} phase
 * @property {number|null} [warmup_remaining_s] - Seconds until a warm-up is cut short, at the latest; with `WarmingUp` only.
 */

/**
 * Every connected camera's phase at once, replacing what the client holds. Sent when a client connects and after it fell behind: phases otherwise arrive only as changes, and a page that missed one showed "Start guide" for a running loop.
 * @typedef {object} CameraPhasesEvent
 * @property {'camera_phases'} type
 * @property {Array<CameraPhaseEntry>} cameras
 */

/**
 * The camera the eyepiece pages show. Also sent when a client connects and after it fell behind, since `/` derives its Guide toggle from it.
 * @typedef {object} ViewedCameraChangedEvent
 * @property {'viewed_camera_changed'} type
 * @property {CameraRole} camera
 */

/**
 * The main stream's frames became, or stopped being, an `auto_crop` cut of the stack. Announced just before the first frame cut the new way is published, and sent when a client connects and after it fell behind: the live view shifts its pan by the origin's move rather than refitting, which would undo the observer's zoom every few subs. `frame` is the live stream's payload size cut the new way: the event and the frame travel on different sockets, so a frame cut the old way can land after the event, and the view waits for this size rather than reframing the old picture.
 * @typedef {object} StackCropChangedEvent
 * @property {'stack_crop_changed'} type
 * @property {StreamCrop|null} [crop]
 * @property {StreamFrameSize|null} [frame]
 */

/**
 * The camera hit a run of consecutive status-poll watchdog timeouts across reconnects — likely a persistent hardware/USB fault rather than an isolated hiccup. Sent in addition to (not instead of) the ordinary `Error` event that accompanies every individual timeout.
 * @typedef {object} CameraPersistentlyUnresponsiveEvent
 * @property {'camera_persistently_unresponsive'} type
 * @property {string} name
 * @property {number} consecutive_timeouts
 */

/**
 * An automatic reconnect is under way after the camera dropped out. `attempt` counts from 1; `next_attempt_in_s` is the wait before this attempt runs.
 * @typedef {object} CameraReconnectingEvent
 * @property {'camera_reconnecting'} type
 * @property {string} name
 * @property {CameraRole} role
 * @property {number} attempt
 * @property {number} of
 * @property {number} next_attempt_in_s
 */

/**
 * Automatic reconnect gave up. The camera stays disconnected until somebody intervenes.
 * @typedef {object} CameraReconnectFailedEvent
 * @property {'camera_reconnect_failed'} type
 * @property {string} name
 * @property {CameraRole} role
 * @property {number} attempts
 * @property {string} reason
 */

/**
 * A reconnect succeeded and the interrupted capture resumed, keeping the stack it had already accumulated.
 * @typedef {object} CaptureResumedEvent
 * @property {'capture_resumed'} type
 * @property {string} name
 * @property {number} stacked_count
 */

/**
 * `role` has no recovery to report: its camera came back, or the observer disconnected or replaced it. The "none" of `camera_reconnecting` / `camera_reconnect_failed`.
 * @typedef {object} CameraRecoveryClearedEvent
 * @property {'camera_recovery_cleared'} type
 * @property {CameraRole} role
 */

/**
 * The server left Focus/Finder mode on its own: a running capture switched to stacking, which needs the correction the mode holds off. Pressing Start stays silent — the observer asked for the stack, and `SettingsUpdated` moves the toggle.
 * @typedef {object} FocusModeLeftEvent
 * @property {'focus_mode_left'} type
 */

/**
 * The AI compute report changed (benchmark progress, a result, a unit that failed at runtime). Clients refetch `GET /api/ai-compute`, which resolves their preference.
 * @typedef {object} AiComputeChangedEvent
 * @property {'ai_compute_changed'} type
 */

/**
 * Error occurred
 * @typedef {object} ErrorEvent
 * @property {'error'} type
 * @property {string} message
 */

/**
 * Disk writer queue warning (queue depth exceeds threshold)
 * @typedef {object} DiskWriterWarningEvent
 * @property {'disk_writer_warning'} type
 * @property {number} queue_depth
 */

/**
 * Disk writer queue warning cleared
 * @typedef {object} DiskWriterWarningClearedEvent
 * @property {'disk_writer_warning_cleared'} type
 */

/**
 * Frame was dropped because the pipeline couldn't keep up.
 * @typedef {object} FrameDroppedEvent
 * @property {'frame_dropped'} type
 * @property {number} dropped_count
 * @property {number} delivered_count
 */

/**
 * Every frame counter at once, replacing the client's: sent when a client connects and after it fell behind, since the per-frame events carry only some of them.
 * @typedef {object} CaptureCountsEvent
 * @property {'capture_counts'} type
 * @property {number} frame_count
 * @property {number} stacked_count
 * @property {number} rejected_count
 * @property {number} dropped_count
 * @property {number} delivered_count
 * @property {string|null} last_rejection_reason
 */

/**
 * Warning message
 * @typedef {object} WarningEvent
 * @property {'warning'} type
 * @property {string} message
 */

/**
 * The sync snapshot just sent is complete: the client re-reads every [`TopicKind::Refetch`] topic. `dropped` counts the events a lagged client missed.
 * @typedef {object} SyncedEvent
 * @property {'synced'} type
 * @property {SyncCause} cause
 * @property {number} dropped
 */

/**
 * Plate solving started
 * @typedef {object} PlateSolvingStartedEvent
 * @property {'plate_solving_started'} type
 * @property {string|null} target_name
 */

/**
 * Progress within a plate solve that is trying several strategies in turn.
 * @typedef {object} PlateSolvingProgressEvent
 * @property {'plate_solving_progress'} type
 * @property {string} stage
 * @property {number} attempt
 * @property {number} total
 */

/**
 * Position solved successfully
 * @typedef {object} PositionSolvedEvent
 * @property {'position_solved'} type
 * @property {number} ra_degrees
 * @property {number} dec_degrees
 * @property {string} ra_string
 * @property {string} dec_string
 * @property {number|null} stars_detected - Stars ASTAP reported finding, or `None` when it never said.
 * @property {number} confidence
 * @property {number} rotation_deg
 */

/**
 * Position solve failed
 * @typedef {object} PositionSolveFailedEvent
 * @property {'position_solve_failed'} type
 * @property {string} reason
 */

/**
 * A solve in flight was abandoned on request.
 * @typedef {object} PlateSolvingCancelledEvent
 * @property {'plate_solving_cancelled'} type
 */

/**
 * A solve was abandoned and will be retried, because something it depended on changed (telescope optics, binning, sensor mode).
 * @typedef {object} PlateSolvingRestartedEvent
 * @property {'plate_solving_restarted'} type
 * @property {string} reason
 */

/**
 * Plate solving cannot run, and why.
 * @typedef {object} PushToBlockedEvent
 * @property {'push_to_blocked'} type
 * @property {string|null} reason
 */

/**
 * Push direction updated.
 * @typedef {object} PushDirectionUpdatedEvent
 * @property {'push_direction_updated'} type
 * @property {number} angle_deg
 * @property {number} distance_deg
 * @property {string} direction_hint
 * @property {boolean} is_close
 * @property {number|null} fov_deg
 * @property {string|null} solved_camera
 */

/**
 * No push direction: no target, or none computed for the current one yet.
 * @typedef {object} PushDirectionClearedEvent
 * @property {'push_direction_cleared'} type
 */

/**
 * The whole solve state, as the sync snapshot restates it.
 * @typedef {object} PlateSolvingStatusEvent
 * @property {'plate_solving_status'} type
 * @property {boolean} in_progress
 * @property {string|null} target_name
 * @property {SolveStage|null} stage
 * @property {SolveVerdict|null} last_result - `None` until a solve ends, and after a restart or a cleared target.
 */

/**
 * Target changed
 * @typedef {object} TargetChangedEvent
 * @property {'target_changed'} type
 * @property {string|null} name
 * @property {string|null} designation
 * @property {number} ra_degrees
 * @property {number} dec_degrees
 */

/**
 * Target cleared
 * @typedef {object} TargetClearedEvent
 * @property {'target_cleared'} type
 */

/**
 * ASTAP installation starting
 * @typedef {object} AstapInstallStartingEvent
 * @property {'astap_install_starting'} type
 * @property {string} component
 */

/**
 * ASTAP installation progress (downloading)
 * @typedef {object} AstapInstallProgressEvent
 * @property {'astap_install_progress'} type
 * @property {string} component
 * @property {number} bytes_downloaded
 * @property {number|null} total_bytes
 * @property {number|null} percent - Percentage of current operation (0-100)
 * @property {string|null} stage - Current installation stage name
 * @property {number|null} overall_percent - Overall installation progress (0-100)
 */

/**
 * ASTAP installation extracting
 * @typedef {object} AstapInstallExtractingEvent
 * @property {'astap_install_extracting'} type
 * @property {string} component
 * @property {number} progress - Percentage of extraction (0-100)
 * @property {string|null} stage - Current installation stage name
 * @property {number|null} overall_percent - Overall installation progress (0-100)
 */

/**
 * ASTAP installation completed successfully
 * @typedef {object} AstapInstallCompletedEvent
 * @property {'astap_install_completed'} type
 * @property {string} component
 * @property {string|null} stage - Current installation stage name
 * @property {number|null} overall_percent - Overall installation progress (0-100)
 */

/**
 * ASTAP installation failed
 * @typedef {object} AstapInstallFailedEvent
 * @property {'astap_install_failed'} type
 * @property {string} component
 * @property {string} error
 */

/**
 * Catalog installation starting
 * @typedef {object} CatalogInstallStartingEvent
 * @property {'catalog_install_starting'} type
 */

/**
 * Catalog download progress
 * @typedef {object} CatalogInstallProgressEvent
 * @property {'catalog_install_progress'} type
 * @property {string} file_name
 * @property {number} bytes_downloaded
 * @property {number|null} total_bytes
 * @property {number|null} percent
 */

/**
 * Catalog file downloaded
 * @typedef {object} CatalogFileCompletedEvent
 * @property {'catalog_file_completed'} type
 * @property {string} file_name
 */

/**
 * Catalog installation completed
 * @typedef {object} CatalogInstallCompletedEvent
 * @property {'catalog_install_completed'} type
 * @property {number} object_count
 */

/**
 * Catalog installation failed
 * @typedef {object} CatalogInstallFailedEvent
 * @property {'catalog_install_failed'} type
 * @property {string} error
 */

/**
 * Where the ASTAP install stands: sent as one begins and ends, and restated by a sync so a page that missed the end stops waiting on it.
 * @typedef {object} AstapInstallStatusEvent
 * @property {'astap_install_status'} type
 * @property {'idle'|'running'|'completed'|'failed'} state
 * @property {ServerEvent|null} [progress]
 * @property {string} [error]
 */

/**
 * Where the catalog install stands, as [`ServerEvent::AstapInstallStatus`].
 * @typedef {object} CatalogInstallStatusEvent
 * @property {'catalog_install_status'} type
 * @property {'idle'|'running'|'completed'|'failed'} state
 * @property {ServerEvent|null} [progress]
 * @property {string} [error]
 */

/**
 * Any event `/ws/events` sends.
 * @typedef {StateChangedEvent|FrameCapturedEvent|FrameRejectedEvent|SettingsUpdatedEvent|CameraConnectedEvent|CameraDisconnectedEvent|CameraDiscoveredEvent|CameraStatusUpdatedEvent|CameraPhaseChangedEvent|CameraPhasesEvent|ViewedCameraChangedEvent|StackCropChangedEvent|CameraPersistentlyUnresponsiveEvent|CameraReconnectingEvent|CameraReconnectFailedEvent|CaptureResumedEvent|CameraRecoveryClearedEvent|FocusModeLeftEvent|AiComputeChangedEvent|ErrorEvent|DiskWriterWarningEvent|DiskWriterWarningClearedEvent|FrameDroppedEvent|CaptureCountsEvent|WarningEvent|SyncedEvent|PlateSolvingStartedEvent|PlateSolvingProgressEvent|PositionSolvedEvent|PositionSolveFailedEvent|PlateSolvingCancelledEvent|PlateSolvingRestartedEvent|PushToBlockedEvent|PushDirectionUpdatedEvent|PushDirectionClearedEvent|PlateSolvingStatusEvent|TargetChangedEvent|TargetClearedEvent|AstapInstallStartingEvent|AstapInstallProgressEvent|AstapInstallExtractingEvent|AstapInstallCompletedEvent|AstapInstallFailedEvent|CatalogInstallStartingEvent|CatalogInstallProgressEvent|CatalogFileCompletedEvent|CatalogInstallCompletedEvent|CatalogInstallFailedEvent|AstapInstallStatusEvent|CatalogInstallStatusEvent} ServerEvent
 */

/** @typedef {import('./api.types.js').Camera} Camera */

/** @typedef {import('./api.types.js').CameraInfo} CameraInfo */

/** @typedef {import('./api.types.js').CameraPhase} CameraPhase */

/**
 * One camera in [`ServerEvent::CameraPhases`].
 * @typedef {object} CameraPhaseEntry
 * @property {string} name
 * @property {CameraRole} role
 * @property {CameraPhase} phase
 * @property {number|null} [warmup_remaining_s] - Seconds until a warm-up in progress is cut short, at the latest.
 */

/** @typedef {import('./api.types.js').CameraRole} CameraRole */

/**
 * DTO for CaptureState serialization
 * @typedef {'Idle'|'Starting'|'Capturing'|'Stopping'|'Error'|'Recovering'} CaptureState
 */

/**
 * A push direction as announced: the plugin's answer plus the solved field of view. `solved_camera` names the camera whose solve oriented `angle_deg` — see [`PushToDirectionResponse::solved_camera`].
 * @typedef {object} PushDirection
 * @property {number} angle_deg
 * @property {number} distance_deg
 * @property {string} direction_hint
 * @property {boolean} is_close
 * @property {number|null} fov_deg
 * @property {string|null} solved_camera
 */

/**
 * The target clients were told about.
 * @typedef {object} PushToTarget
 * @property {string|null} name
 * @property {string|null} designation
 * @property {number} ra_degrees
 * @property {number} dec_degrees
 */

/** @typedef {import('./api.types.js').SensorMode} SensorMode */

/** @typedef {import('./api.types.js').SolveStage} SolveStage */

/** @typedef {import('./api.types.js').SolveStatus} SolveStatus */

/** @typedef {import('./api.types.js').SolveVerdict} SolveVerdict */

/**
 * Where the main stream's frames were cut from the stack, in the stack's pixels.
 * @typedef {object} StreamCrop
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
 * @property {number} source_width - The uncropped stack's size.
 * @property {number} source_height
 */

/**
 * The size of the `/ws/stream` frame a [`ServerEvent::StackCropChanged`] arrives with.
 * @typedef {object} StreamFrameSize
 * @property {number} width
 * @property {number} height
 */

/**
 * Why a client is being synced.
 * @typedef {'connect'|'lagged'} SyncCause
 */

/**
 * Every event by `type`, in declaration order: the topic it states or announces, and how a
 * sync restates that topic (`TOPICS`); `notice` for a one-off a lost copy of goes without.
 * @type {Readonly<Object<string, Readonly<{topic: string|null, kind: 'value'|'refetch'|'notice'}>>>}
 */
export const SERVER_EVENTS = Object.freeze({
    state_changed: Object.freeze({topic: 'capture_state', kind: 'value'}),
    frame_captured: Object.freeze({topic: 'capture_counts', kind: 'value'}),
    frame_rejected: Object.freeze({topic: 'capture_counts', kind: 'value'}),
    settings_updated: Object.freeze({topic: 'settings', kind: 'refetch'}),
    camera_connected: Object.freeze({topic: 'camera_list', kind: 'refetch'}),
    camera_disconnected: Object.freeze({topic: 'camera_list', kind: 'refetch'}),
    camera_discovered: Object.freeze({topic: 'camera_list', kind: 'refetch'}),
    camera_status_updated: Object.freeze({topic: null, kind: 'notice'}),
    camera_phase_changed: Object.freeze({topic: 'camera_phases', kind: 'value'}),
    camera_phases: Object.freeze({topic: 'camera_phases', kind: 'value'}),
    viewed_camera_changed: Object.freeze({topic: 'viewed_camera', kind: 'value'}),
    stack_crop_changed: Object.freeze({topic: 'stack_crop', kind: 'value'}),
    camera_persistently_unresponsive: Object.freeze({topic: null, kind: 'notice'}),
    camera_reconnecting: Object.freeze({topic: 'camera_recovery', kind: 'value'}),
    camera_reconnect_failed: Object.freeze({topic: 'camera_recovery', kind: 'value'}),
    capture_resumed: Object.freeze({topic: 'camera_recovery', kind: 'value'}),
    camera_recovery_cleared: Object.freeze({topic: 'camera_recovery', kind: 'value'}),
    focus_mode_left: Object.freeze({topic: null, kind: 'notice'}),
    ai_compute_changed: Object.freeze({topic: 'ai_compute', kind: 'refetch'}),
    error: Object.freeze({topic: null, kind: 'notice'}),
    disk_writer_warning: Object.freeze({topic: 'disk_writer', kind: 'value'}),
    disk_writer_warning_cleared: Object.freeze({topic: 'disk_writer', kind: 'value'}),
    frame_dropped: Object.freeze({topic: 'capture_counts', kind: 'value'}),
    capture_counts: Object.freeze({topic: 'capture_counts', kind: 'value'}),
    warning: Object.freeze({topic: null, kind: 'notice'}),
    synced: Object.freeze({topic: null, kind: 'notice'}),
    plate_solving_started: Object.freeze({topic: 'push_to_solve', kind: 'value'}),
    plate_solving_progress: Object.freeze({topic: 'push_to_solve', kind: 'value'}),
    position_solved: Object.freeze({topic: 'push_to_solve', kind: 'value'}),
    position_solve_failed: Object.freeze({topic: 'push_to_solve', kind: 'value'}),
    plate_solving_cancelled: Object.freeze({topic: 'push_to_solve', kind: 'value'}),
    plate_solving_restarted: Object.freeze({topic: 'push_to_solve', kind: 'value'}),
    push_to_blocked: Object.freeze({topic: 'push_to_blocker', kind: 'value'}),
    push_direction_updated: Object.freeze({topic: 'push_to_direction', kind: 'value'}),
    push_direction_cleared: Object.freeze({topic: 'push_to_direction', kind: 'value'}),
    plate_solving_status: Object.freeze({topic: 'push_to_solve', kind: 'value'}),
    target_changed: Object.freeze({topic: 'push_to_target', kind: 'value'}),
    target_cleared: Object.freeze({topic: 'push_to_target', kind: 'value'}),
    astap_install_starting: Object.freeze({topic: 'astap_install', kind: 'value'}),
    astap_install_progress: Object.freeze({topic: 'astap_install', kind: 'value'}),
    astap_install_extracting: Object.freeze({topic: 'astap_install', kind: 'value'}),
    astap_install_completed: Object.freeze({topic: 'astap_install', kind: 'value'}),
    astap_install_failed: Object.freeze({topic: 'astap_install', kind: 'value'}),
    catalog_install_starting: Object.freeze({topic: 'catalog_install', kind: 'value'}),
    catalog_install_progress: Object.freeze({topic: 'catalog_install', kind: 'value'}),
    catalog_file_completed: Object.freeze({topic: 'catalog_install', kind: 'value'}),
    catalog_install_completed: Object.freeze({topic: 'catalog_install', kind: 'value'}),
    catalog_install_failed: Object.freeze({topic: 'catalog_install', kind: 'value'}),
    astap_install_status: Object.freeze({topic: 'astap_install', kind: 'value'}),
    catalog_install_status: Object.freeze({topic: 'catalog_install', kind: 'value'}),
})

/**
 * How a sync restates each topic: `value` by the events of the snapshot before `synced`,
 * `refetch` by the client reading its REST endpoint again on `synced`.
 * @type {Readonly<Object<string, 'value'|'refetch'>>}
 */
export const TOPICS = Object.freeze({
    capture_state: 'value',
    capture_counts: 'value',
    camera_phases: 'value',
    camera_recovery: 'value',
    viewed_camera: 'value',
    stack_crop: 'value',
    disk_writer: 'value',
    push_to_target: 'value',
    push_to_direction: 'value',
    push_to_solve: 'value',
    push_to_blocker: 'value',
    astap_install: 'value',
    catalog_install: 'value',
    settings: 'refetch',
    ai_compute: 'refetch',
    camera_list: 'refetch',
})
