/**
 * The events `/ws/events` sends: a type of each, and the topic each states, generated
 * from the Rust `ServerEvent` by `tests/api_types.rs`. Do not edit: change the Rust, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */

import type {
    Camera,
    CameraPhase,
    CameraRole,
    CaptureState,
    DirectionHint,
    SolveStage,
    SolveVerdict,
} from './api.types'

/** Capture state changed */
export interface StateChangedEvent {
    type: 'state_changed'
    state: CaptureState
}

/** New frame captured */
export interface FrameCapturedEvent {
    type: 'frame_captured'
    frame_number: number
    stacked_count: number
    rejected_count: number
    /**
     * Why this frame did not join the stack, when it did not.
     *
     * Separate from [`ServerEvent::FrameRejected`], which means the camera
     * failed to deliver a frame at all and feeds the capture-abort burst
     * detector. A frame that arrived fine and merely aligned badly must not
     * go through that path — a night of soft seeing would stop the capture.
     */
    rejection_reason?: string | null
}

/** Frame was rejected */
export interface FrameRejectedEvent {
    type: 'frame_rejected'
    frame_number: number
    stacked_count: number
    rejected_count: number
    reason: string
}

/** Settings were updated */
export interface SettingsUpdatedEvent {
    type: 'settings_updated'
}

/** Camera connected */
export interface CameraConnectedEvent {
    type: 'camera_connected'
    name: string
}

/** Camera disconnected */
export interface CameraDisconnectedEvent {
    type: 'camera_disconnected'
    name: string
}

/** Camera discovered asynchronously (e.g. INDI) */
export interface CameraDiscoveredEvent {
    type: 'camera_discovered'
    camera: Camera
}

/** Cooled camera status sample (sensor temperature, cooler power, cooler state) */
export interface CameraStatusUpdatedEvent {
    type: 'camera_status_updated'
    name: string
    temperature_c: number
    cooler_power?: number | null
    cooler_on: boolean
    dew_heater_on: boolean
    target_temp_c?: number | null
}

/**
 * Camera lifecycle phase changed (Precooling, Idle, Capturing, WarmingUp, Disconnected).
 * `role` tells two bodies of one model apart, which `name` cannot.
 */
export interface CameraPhaseChangedEvent {
    type: 'camera_phase_changed'
    name: string
    role: CameraRole
    phase: CameraPhase
    /** Seconds until a warm-up is cut short, at the latest; with `WarmingUp` only. */
    warmup_remaining_s?: number | null
}

/**
 * Every connected camera's phase at once, replacing what the client holds. Sent when
 * a client connects and after it fell behind: phases otherwise arrive only as
 * changes, and a page that missed one showed "Start guide" for a running loop.
 */
export interface CameraPhasesEvent {
    type: 'camera_phases'
    cameras: CameraPhaseEntry[]
}

/**
 * The camera the eyepiece pages show. Also sent when a client connects and after it
 * fell behind, since `/` derives its Guide toggle from it.
 */
export interface ViewedCameraChangedEvent {
    type: 'viewed_camera_changed'
    camera: CameraRole
}

/**
 * The main stream's frames became, or stopped being, an `auto_crop` cut of the stack.
 * Announced just before the first frame cut the new way is published, and sent when a
 * client connects and after it fell behind: the live view shifts its pan by the
 * origin's move rather than refitting, which would undo the observer's zoom every few subs.
 * `frame` is the live stream's payload size cut the new way: the event and the frame
 * travel on different sockets, so a frame cut the old way can land after the event, and
 * the view waits for this size rather than reframing the old picture.
 */
export interface StackCropChangedEvent {
    type: 'stack_crop_changed'
    crop?: StreamCrop | null
    frame?: StreamFrameSize | null
}

/**
 * The camera hit a run of consecutive status-poll watchdog timeouts across
 * reconnects — likely a persistent hardware/USB fault rather than an
 * isolated hiccup. Sent in addition to (not instead of) the ordinary
 * `Error` event that accompanies every individual timeout.
 */
export interface CameraPersistentlyUnresponsiveEvent {
    type: 'camera_persistently_unresponsive'
    name: string
    consecutive_timeouts: number
}

/**
 * An automatic reconnect is under way after the camera dropped out.
 * `attempt` counts from 1; `next_attempt_in_s` is the wait before this
 * attempt runs.
 */
export interface CameraReconnectingEvent {
    type: 'camera_reconnecting'
    name: string
    role: CameraRole
    attempt: number
    of: number
    next_attempt_in_s: number
}

/**
 * Automatic reconnect gave up. The camera stays disconnected until
 * somebody intervenes.
 */
export interface CameraReconnectFailedEvent {
    type: 'camera_reconnect_failed'
    name: string
    role: CameraRole
    attempts: number
    reason: string
}

/**
 * A reconnect succeeded and the interrupted capture resumed, keeping the
 * stack it had already accumulated.
 */
export interface CaptureResumedEvent {
    type: 'capture_resumed'
    name: string
    stacked_count: number
}

/**
 * `role` has no recovery to report: its camera came back, or the observer disconnected
 * or replaced it. The "none" of `camera_reconnecting` / `camera_reconnect_failed`.
 */
export interface CameraRecoveryClearedEvent {
    type: 'camera_recovery_cleared'
    role: CameraRole
}

/**
 * The server left Focus/Finder mode on its own: a running capture switched to stacking,
 * which needs the correction the mode holds off. Pressing Start stays silent — the
 * observer asked for the stack, and `SettingsUpdated` moves the toggle.
 */
export interface FocusModeLeftEvent {
    type: 'focus_mode_left'
}

/**
 * The AI compute report changed (benchmark progress, a result, a unit that failed at
 * runtime). Clients refetch `GET /api/ai-compute`, which resolves their preference.
 */
export interface AiComputeChangedEvent {
    type: 'ai_compute_changed'
}

/** Error occurred */
export interface ErrorEvent {
    type: 'error'
    message: string
}

/** Disk writer queue warning (queue depth exceeds threshold) */
export interface DiskWriterWarningEvent {
    type: 'disk_writer_warning'
    queue_depth: number
}

/** Disk writer queue warning cleared */
export interface DiskWriterWarningClearedEvent {
    type: 'disk_writer_warning_cleared'
}

/**
 * Whether `role`'s last capture is on the storage device, not just in the OS's cache:
 * what the observer waits for before powering off. `files` were synced; `failed`
 * writes or syncs did not land, the first one's reason in `error`.
 */
export interface StorageSyncEvent {
    type: 'storage_sync'
    role: CameraRole
    state: StorageSyncState
    files: number
    failed: number
    error: string | null
}

/**
 * Frame was dropped because the pipeline couldn't keep up.
 *
 * `delivered_count` travels with it so the client can show a *rate*. The count
 * alone grows all night and means nothing without the cadence it grew against.
 */
export interface FrameDroppedEvent {
    type: 'frame_dropped'
    dropped_count: number
    delivered_count: number
}

/**
 * Every frame counter at once, replacing the client's: sent when a client connects
 * and after it fell behind, since the per-frame events carry only some of them.
 */
export interface CaptureCountsEvent {
    type: 'capture_counts'
    frame_count: number
    stacked_count: number
    rejected_count: number
    dropped_count: number
    delivered_count: number
    last_rejection_reason: string | null
}

/** Warning message */
export interface WarningEvent {
    type: 'warning'
    message: string
}

/**
 * The sync snapshot just sent is complete: the client re-reads every
 * [`TopicKind::Refetch`] topic. `dropped` counts the events a lagged client missed.
 */
export interface SyncedEvent {
    type: 'synced'
    cause: SyncCause
    dropped: number
}

/** Plate solving started */
export interface PlateSolvingStartedEvent {
    type: 'plate_solving_started'
    target_name: string | null
}

/**
 * Progress within a plate solve that is trying several strategies in turn.
 *
 * A cold solve works down an ordered list of attempts, and the last of those can
 * legitimately run for a minute or more. Without this the UI cannot tell a slow
 * solve from a hung one. `stage` is a human-readable description supplied by the
 * solver, not an enum, so the strategy list can change without a protocol change.
 */
export interface PlateSolvingProgressEvent {
    type: 'plate_solving_progress'
    stage: string
    attempt: number
    total: number
}

/** Position solved successfully */
export interface PositionSolvedEvent {
    type: 'position_solved'
    ra_degrees: number
    dec_degrees: number
    ra_string: string
    dec_string: string
    /** Stars ASTAP reported finding, or `None` when it never said. */
    stars_detected: number | null
    confidence: number
    rotation_deg: number
}

/** Position solve failed */
export interface PositionSolveFailedEvent {
    type: 'position_solve_failed'
    reason: string
}

/**
 * A solve in flight was abandoned on request.
 *
 * Separate from [`ServerEvent::PositionSolveFailed`] because the two mean
 * opposite things to the user: a failure says the sky could not be matched,
 * a cancel says "you asked me to stop". Reporting a cancel as a failure also
 * left the last known position looking untrustworthy when it was still good.
 */
export interface PlateSolvingCancelledEvent {
    type: 'plate_solving_cancelled'
}

/**
 * A solve was abandoned and will be retried, because something it depended on
 * changed (telescope optics, binning, sensor mode).
 */
export interface PlateSolvingRestartedEvent {
    type: 'plate_solving_restarted'
    reason: string
}

/**
 * Plate solving cannot run, and why.
 *
 * Emitted only when the reason changes, so it costs nothing per frame. `reason`
 * is `None` once solving is possible again, which clears the notice.
 */
export interface PushToBlockedEvent {
    type: 'push_to_blocked'
    reason: string | null
}

/** Push direction updated. */
export interface PushDirectionUpdatedEvent {
    type: 'push_direction_updated'
    angle_deg: number
    distance_deg: number
    direction_hint: DirectionHint
    is_close: boolean
    fov_deg: number | null
    solved_camera: string | null
    stale: boolean
}

/** No push direction: no target, or none computed for the current one yet. */
export interface PushDirectionClearedEvent {
    type: 'push_direction_cleared'
}

/** The whole solve state, as the sync snapshot restates it. */
export interface PlateSolvingStatusEvent {
    type: 'plate_solving_status'
    in_progress: boolean
    target_name: string | null
    stage: SolveStage | null
    /** `None` until a solve ends, and after a restart or a cleared target. */
    last_result: SolveVerdict | null
}

/** Target changed */
export interface TargetChangedEvent {
    type: 'target_changed'
    name: string | null
    designation: string | null
    ra_degrees: number
    dec_degrees: number
}

/** Target cleared */
export interface TargetClearedEvent {
    type: 'target_cleared'
}

/** ASTAP installation starting */
export interface AstapInstallStartingEvent {
    type: 'astap_install_starting'
    component: string
}

/** ASTAP installation progress (downloading) */
export interface AstapInstallProgressEvent {
    type: 'astap_install_progress'
    component: string
    bytes_downloaded: number
    total_bytes: number | null
    /** Percentage of current operation (0-100) */
    percent: number | null
    /** Current installation stage, which the UI names and branches on */
    stage_id: InstallStage | null
    /** Overall installation progress (0-100) */
    overall_percent: number | null
}

/** ASTAP installation extracting */
export interface AstapInstallExtractingEvent {
    type: 'astap_install_extracting'
    component: string
    /** Percentage of extraction (0-100) */
    progress: number
    /** Current installation stage, which the UI names and branches on */
    stage_id: InstallStage | null
    /** Overall installation progress (0-100) */
    overall_percent: number | null
}

/** ASTAP installation completed successfully */
export interface AstapInstallCompletedEvent {
    type: 'astap_install_completed'
    component: string
    /** Current installation stage, which the UI names and branches on */
    stage_id: InstallStage | null
    /** Overall installation progress (0-100) */
    overall_percent: number | null
}

/** ASTAP installation failed */
export interface AstapInstallFailedEvent {
    type: 'astap_install_failed'
    component: string
    error: string
}

/** Catalog installation starting */
export interface CatalogInstallStartingEvent {
    type: 'catalog_install_starting'
}

/** Catalog download progress */
export interface CatalogInstallProgressEvent {
    type: 'catalog_install_progress'
    file_name: string
    bytes_downloaded: number
    total_bytes: number | null
    percent: number | null
}

/** Catalog file downloaded */
export interface CatalogFileCompletedEvent {
    type: 'catalog_file_completed'
    file_name: string
}

/** Catalog installation completed */
export interface CatalogInstallCompletedEvent {
    type: 'catalog_install_completed'
    object_count: number
}

/** Catalog installation failed */
export interface CatalogInstallFailedEvent {
    type: 'catalog_install_failed'
    error: string
}

/**
 * Where the ASTAP install stands: sent as one begins and ends, and restated by a sync
 * so a page that missed the end stops waiting on it.
 */
export type AstapInstallStatusEvent = {
    type: 'astap_install_status'
} & (
    | {
        state: 'idle'
    }
    | {
        state: 'running'
        progress: ServerEvent | null
    }
    | {
        state: 'completed'
    }
    | {
        state: 'failed'
        error: string
    }
)

/** Where the catalog install stands, as [`ServerEvent::AstapInstallStatus`]. */
export type CatalogInstallStatusEvent = {
    type: 'catalog_install_status'
} & (
    | {
        state: 'idle'
    }
    | {
        state: 'running'
        progress: ServerEvent | null
    }
    | {
        state: 'completed'
    }
    | {
        state: 'failed'
        error: string
    }
)

/** Any event `/ws/events` sends. */
export type ServerEvent =
    | StateChangedEvent
    | FrameCapturedEvent
    | FrameRejectedEvent
    | SettingsUpdatedEvent
    | CameraConnectedEvent
    | CameraDisconnectedEvent
    | CameraDiscoveredEvent
    | CameraStatusUpdatedEvent
    | CameraPhaseChangedEvent
    | CameraPhasesEvent
    | ViewedCameraChangedEvent
    | StackCropChangedEvent
    | CameraPersistentlyUnresponsiveEvent
    | CameraReconnectingEvent
    | CameraReconnectFailedEvent
    | CaptureResumedEvent
    | CameraRecoveryClearedEvent
    | FocusModeLeftEvent
    | AiComputeChangedEvent
    | ErrorEvent
    | DiskWriterWarningEvent
    | DiskWriterWarningClearedEvent
    | StorageSyncEvent
    | FrameDroppedEvent
    | CaptureCountsEvent
    | WarningEvent
    | SyncedEvent
    | PlateSolvingStartedEvent
    | PlateSolvingProgressEvent
    | PositionSolvedEvent
    | PositionSolveFailedEvent
    | PlateSolvingCancelledEvent
    | PlateSolvingRestartedEvent
    | PushToBlockedEvent
    | PushDirectionUpdatedEvent
    | PushDirectionClearedEvent
    | PlateSolvingStatusEvent
    | TargetChangedEvent
    | TargetClearedEvent
    | AstapInstallStartingEvent
    | AstapInstallProgressEvent
    | AstapInstallExtractingEvent
    | AstapInstallCompletedEvent
    | AstapInstallFailedEvent
    | CatalogInstallStartingEvent
    | CatalogInstallProgressEvent
    | CatalogFileCompletedEvent
    | CatalogInstallCompletedEvent
    | CatalogInstallFailedEvent
    | AstapInstallStatusEvent
    | CatalogInstallStatusEvent

export type ServerEventType = ServerEvent['type']

/** The event `/ws/events` sends as `type`. */
export type EventOf<K extends ServerEventType> = Extract<ServerEvent, {type: K}>

/** One camera in [`ServerEvent::CameraPhases`]. */
export interface CameraPhaseEntry {
    name: string
    role: CameraRole
    phase: CameraPhase
    /** Seconds until a warm-up in progress is cut short, at the latest. */
    warmup_remaining_s?: number | null
}

/**
 * Where an ASTAP install stands, sent as `stage_id`: the UI names each stage itself and
 * branches on the id, never on wording.
 */
export const INSTALL_STAGE_VALUES = [
    'downloading_cli',
    'extracting_cli',
    'cli_completed',
    'downloading_database',
    'extracting_database',
    'database_completed',
    'catalog_files',
] as const
export type InstallStage = (typeof INSTALL_STAGE_VALUES)[number]

/**
 * A push direction as announced: the plugin's answer plus the solved field of view.
 * `solved_camera` names the camera whose solve oriented `angle_deg` — see
 * [`PushToDirectionResponse::solved_camera`]. `stale`: the last solve failed, so these are
 * the numbers of the fix before it — stamped by `PushToAnnouncer`, which re-sends a held
 * direction marked when a solve fails, so a client's dismissed verdict cannot unmark it.
 */
export interface PushDirection {
    angle_deg: number
    distance_deg: number
    direction_hint: DirectionHint
    is_close: boolean
    fov_deg: number | null
    solved_camera: string | null
    stale: boolean
}

/** The target clients were told about. */
export interface PushToTarget {
    name: string | null
    designation: string | null
    ra_degrees: number
    dec_degrees: number
}

export const STORAGE_SYNC_STATE_VALUES = ['idle', 'syncing', 'synced', 'failed'] as const
export type StorageSyncState = (typeof STORAGE_SYNC_STATE_VALUES)[number]

/** Where the main stream's frames were cut from the stack, in the stack's pixels. */
export interface StreamCrop {
    x: number
    y: number
    width: number
    height: number
    /** The uncropped stack's size. */
    source_width: number
    source_height: number
}

/** The size of the `/ws/stream` frame a [`ServerEvent::StackCropChanged`] arrives with. */
export interface StreamFrameSize {
    width: number
    height: number
}

/** Why a client is being synced. */
export const SYNC_CAUSE_VALUES = ['connect', 'lagged'] as const
export type SyncCause = (typeof SYNC_CAUSE_VALUES)[number]

/**
 * Every event by `type`, in declaration order: the topic it states or announces, and how a
 * sync restates that topic (`TOPICS`); `notice` for a one-off a lost copy of goes without.
 */
export const SERVER_EVENTS = {
    state_changed: {topic: 'capture_state', kind: 'value'},
    frame_captured: {topic: 'capture_counts', kind: 'value'},
    frame_rejected: {topic: 'capture_counts', kind: 'value'},
    settings_updated: {topic: 'settings', kind: 'refetch'},
    camera_connected: {topic: 'camera_list', kind: 'refetch'},
    camera_disconnected: {topic: 'camera_list', kind: 'refetch'},
    camera_discovered: {topic: 'camera_list', kind: 'refetch'},
    camera_status_updated: {topic: null, kind: 'notice'},
    camera_phase_changed: {topic: 'camera_phases', kind: 'value'},
    camera_phases: {topic: 'camera_phases', kind: 'value'},
    viewed_camera_changed: {topic: 'viewed_camera', kind: 'value'},
    stack_crop_changed: {topic: 'stack_crop', kind: 'value'},
    camera_persistently_unresponsive: {topic: null, kind: 'notice'},
    camera_reconnecting: {topic: 'camera_recovery', kind: 'value'},
    camera_reconnect_failed: {topic: 'camera_recovery', kind: 'value'},
    capture_resumed: {topic: 'camera_recovery', kind: 'value'},
    camera_recovery_cleared: {topic: 'camera_recovery', kind: 'value'},
    focus_mode_left: {topic: null, kind: 'notice'},
    ai_compute_changed: {topic: 'ai_compute', kind: 'refetch'},
    error: {topic: null, kind: 'notice'},
    disk_writer_warning: {topic: 'disk_writer', kind: 'value'},
    disk_writer_warning_cleared: {topic: 'disk_writer', kind: 'value'},
    storage_sync: {topic: 'storage_sync', kind: 'value'},
    frame_dropped: {topic: 'capture_counts', kind: 'value'},
    capture_counts: {topic: 'capture_counts', kind: 'value'},
    warning: {topic: null, kind: 'notice'},
    synced: {topic: null, kind: 'notice'},
    plate_solving_started: {topic: 'push_to_solve', kind: 'value'},
    plate_solving_progress: {topic: 'push_to_solve', kind: 'value'},
    position_solved: {topic: 'push_to_solve', kind: 'value'},
    position_solve_failed: {topic: 'push_to_solve', kind: 'value'},
    plate_solving_cancelled: {topic: 'push_to_solve', kind: 'value'},
    plate_solving_restarted: {topic: 'push_to_solve', kind: 'value'},
    push_to_blocked: {topic: 'push_to_blocker', kind: 'value'},
    push_direction_updated: {topic: 'push_to_direction', kind: 'value'},
    push_direction_cleared: {topic: 'push_to_direction', kind: 'value'},
    plate_solving_status: {topic: 'push_to_solve', kind: 'value'},
    target_changed: {topic: 'push_to_target', kind: 'value'},
    target_cleared: {topic: 'push_to_target', kind: 'value'},
    astap_install_starting: {topic: 'astap_install', kind: 'value'},
    astap_install_progress: {topic: 'astap_install', kind: 'value'},
    astap_install_extracting: {topic: 'astap_install', kind: 'value'},
    astap_install_completed: {topic: 'astap_install', kind: 'value'},
    astap_install_failed: {topic: 'astap_install', kind: 'value'},
    catalog_install_starting: {topic: 'catalog_install', kind: 'value'},
    catalog_install_progress: {topic: 'catalog_install', kind: 'value'},
    catalog_file_completed: {topic: 'catalog_install', kind: 'value'},
    catalog_install_completed: {topic: 'catalog_install', kind: 'value'},
    catalog_install_failed: {topic: 'catalog_install', kind: 'value'},
    astap_install_status: {topic: 'astap_install', kind: 'value'},
    catalog_install_status: {topic: 'catalog_install', kind: 'value'},
} as const satisfies Record<ServerEventType, {topic: Topic | null; kind: 'value' | 'refetch' | 'notice'}>

/**
 * How a sync restates each topic: `value` by the events of the snapshot before `synced`,
 * `refetch` by the client reading its REST endpoint again on `synced`.
 */
export const TOPICS = {
    capture_state: 'value',
    capture_counts: 'value',
    camera_phases: 'value',
    camera_recovery: 'value',
    viewed_camera: 'value',
    stack_crop: 'value',
    disk_writer: 'value',
    storage_sync: 'value',
    push_to_target: 'value',
    push_to_direction: 'value',
    push_to_solve: 'value',
    push_to_blocker: 'value',
    astap_install: 'value',
    catalog_install: 'value',
    settings: 'refetch',
    ai_compute: 'refetch',
    camera_list: 'refetch',
} as const satisfies Record<string, 'value' | 'refetch'>

export type Topic = keyof typeof TOPICS
