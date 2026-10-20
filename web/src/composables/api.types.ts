/**
 * TypeScript types of the server's JSON answers, generated from the Rust wire types by
 * `tests/api_types.rs`. Do not edit: change the Rust type, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */

/**
 * What `GET`/`POST /api/settings` answer: every setting, in `settings.json`'s shape.
 *
 * Except the Focus/Finder snapshot: it is the server's record of what to restore, and a
 * client that could read it would be tempted to write it.
 */
export interface Settings {
    /** Exposure time in microseconds */
    exposure_us: number
    /** Gain value */
    gain: number
    /** Offset (black level) */
    offset: number
    /** Binning factor */
    bin: number
    /** Enable auto-stretch for preview */
    auto_stretch: boolean
    /** Enable live stacking */
    stacking: boolean
    /** Sigma for rejection during stacking */
    rejection_sigma: number
    /**
     * Outlier rejection method. A fresh install and a file without the key both start on
     * [`RejectionMethod::best_available`].
     */
    rejection_method: RejectionMethod
    /** Enable background subtraction */
    background_subtraction: boolean
    /** Algorithm for background extraction (GridBilinear or RBF) */
    background_extraction_algorithm: BackgroundExtractionAlgorithm
    /** Which capture modes write their raw frames to disk (FITS format) */
    raw_frame_saving: RawFrameSaving
    /** Enable saving stacked image to disk (FITS + PNG) */
    save_stacked_image: boolean
    /** Stacking type (Deep Sky or Planetary) */
    stacking_type: StackingType
    /** Quality-based frame weighting preset for stacking */
    weighting_preset: WeightingPreset
    /** Auto stretch aggressiveness (Low, Medium, High) */
    stretch_aggressiveness: StretchAggressiveness
    /** Auto Stretch intensity multiplier (0.0 to 1.0, where 0.0 means no color boost, default 0.3) */
    auto_stretch_intensity: number
    /** Enable shadow saturation boost */
    saturation_boost: boolean
    /** Shadow saturation boost strength (0.0-1.0) */
    saturation_boost_strength: number
    /** Use simulated camera instead of a real one */
    use_simulated_camera: boolean
    /** Number of images to preload for simulated camera */
    simulated_preload_images: number
    /** Show the focus image when waiting for frames */
    show_focus_image: boolean
    /** Force showing the focus image even when the stream is active */
    force_focus_image_now: boolean
    /** Whether the cooler should be active during capture (cooled cameras only) */
    cooler_enabled: boolean
    /** Target sensor temperature in Celsius (None means "no target set") */
    target_temp_c?: number | null
    /**
     * Bypass the 5 °C/min ramp and cool/warm as fast as the hardware allows.
     * Defeats sensor-stress / condensation protections — user-opt-in only.
     */
    cooler_fast_mode: boolean
    /** Manual override for camera sensor mode. None means "derive from stacking_type". */
    sensor_mode_override?: DualSamplingMode | null
    /** Region of interest for comet nucleus tracking */
    comet_roi?: AlignmentRoi | null
    /** Region of interest for planetary alignment */
    planetary_roi?: AlignmentRoi | null
    /** Enable auto tracking of planetary ROI */
    planetary_auto_tracking: boolean
    /** Enable multi-point alignment for planetary (Pro only) */
    planetary_multi_point_alignment: boolean
    /** Whether anti-dew heater is enabled */
    dew_heater_enabled: boolean
    /** Anti-dew heater power level (0-100) */
    dew_heater_power: number
    /** Enable "Wanderer" mode for automatic stack reset on movement */
    wanderer_mode: boolean
    /**
     * Crop a deep-sky stack to the area most of its subs cover, live and in the saved
     * FITS/PNG. Off crops nothing.
     */
    auto_crop: boolean
    /**
     * Reopen the camera automatically after it drops out mid-session (a USB
     * stall or an unplug), instead of leaving the session dead until someone
     * clicks Connect.
     */
    auto_reconnect: boolean
    /**
     * After an automatic reconnect, resume the capture that was interrupted —
     * same mode, same settings, same stack. Without this the camera comes
     * back but the session does not.
     */
    auto_resume_capture: boolean
    /** Corrections applied to the raw sensor mosaic, before demosaic */
    sensor_correction: SensorCorrectionSettings
    /** Spatial denoising, applied at stream resolution inside the encoders */
    denoise: DenoiseSettings
    /** How much sensor resolution the preview pipeline may bin away before it runs */
    preview_resolution: Resolution
    /** The JPEG size every `/` and `/eyepiece` client receives */
    streaming_resolution: Resolution
    /** Eyepiece view settings */
    eyepiece: EyepieceSettings
    /** Telescope and camera parameters for FOV calculation */
    telescope: TelescopeSettings
    /** Per-camera telescope profiles keyed by camera name */
    camera_telescope_profiles?: Partial<Record<string, TelescopeSettings>>
    /**
     * Per-camera capture profiles keyed by `"{provider}/{model_name}"`.
     * Holds the seven hardware-specific fields so switching between cameras
     * doesn't leak stale values (e.g. cooler=true from a cooled camera into
     * an uncooled one).
     */
    camera_profiles?: Partial<Record<string, CameraCaptureProfile>>
    /**
     * The guide camera's live hardware values.
     *
     * The flat fields above are the *main* camera's; the guide camera cannot share them
     * because both are connected at once and a guide sub is typically seconds where an
     * imaging sub is minutes. Same shape as a stored profile, so `camera_profiles`
     * remembers it across reconnects exactly as it does the main camera's.
     */
    guide_camera: CameraCaptureProfile
    /** Name of the last active camera (for profile inheritance) */
    last_camera_name?: string | null
    /** Whether the user has accepted the End User License Agreement */
    eula_accepted: boolean
    /** INDI server host */
    indi_server_host: string
    /** INDI server port */
    indi_server_port: number
    /**
     * Hold the cosmetic pipeline stages off while framing and focusing.
     *
     * Invariant: true exactly when [`Self::focus_mode_snapshot`] is `Some`. Drive it
     * through [`super::focus_mode::set`], never by assignment — a bare write leaves the
     * snapshot behind and the next toggle restores the wrong values.
     */
    focus_mode: boolean
}

/** Comprehensive information about a stacking type */
export interface StackingTypeInfo {
    /** The stacking type identifier */
    id: StackingType
    /** Human-readable display name */
    name: string
    /** Description of when to use this type */
    description: string
    /** Whether this type uses star-based registration */
    uses_star_registration: boolean
    /** Whether live stacking is supported */
    supports_stacking: boolean
    /** Whether quality-based frame weighting is supported */
    supports_quality_weighting: boolean
}

/** What `GET /api/capabilities` answers: which Pro plugins are active. */
export interface Capabilities {
    has_pro: boolean
    deep_sky: DeepSkyCapabilities
    planetary: PlanetaryCapabilities
    push_to: PushToCapabilities
    comet: CometCapabilities
    debug_logging: boolean
}

/** Capture status response */
export interface CaptureStatus {
    state: CaptureState
    frame_count: number
    stacked_count: number
    rejected_count: number
    /** Frames the pipeline's back-pressure dropped, out of `delivered_count`. */
    dropped_count: number
    delivered_count: number
    /** Why the most recently rejected frame was kept out of the stack. */
    last_rejection_reason: string | null
    started_at: number | null
    exposure_us: number
    gain: number
}

/** Camera list entry */
export interface Camera {
    id: string
    name: string
    connected: boolean
    provider?: string | null
    index?: number | null
    /** Which position this camera occupies, or `None` if it is not connected. */
    role?: CameraRole | null
    /**
     * Lifecycle phase of a connected camera. The client's phase map is otherwise built
     * from `camera_phase_changed` events alone, which a page opened later never saw.
     */
    phase?: CameraPhase | null
    /** Seconds until a warm-up in progress is cut short, at the latest. */
    warmup_remaining_s?: number | null
    info: CameraInfo
}

/** Camera info response */
export interface CameraInfo {
    id: string
    name: string
    max_width: number
    max_height: number
    pixel_size_x_um: number
    pixel_size_y_um: number
    sensor_type: string
    has_cooler: boolean
    has_dew_heater: boolean
    min_temp_c?: number | null
    max_temp_c?: number | null
    bit_depth: number
    min_exposure_us: number
    max_exposure_us: number
    min_gain: number
    max_gain: number
    /** Sensor (dual sampling) modes reported by the camera. Empty when unsupported. */
    sensor_modes?: SensorMode[]
}

/** Simulated camera configuration response */
export interface SimulatorConfig {
    configured: boolean
    directory?: string | null
    file_count?: number | null
    camera_count?: number | null
    was_added?: boolean | null
    message?: string | null
}

/** `GET /api/push-to/status`: the plugin's answer, plus what the sync snapshot restates. */
export interface PushToStatus {
    /** Whether the solver database is loaded */
    solver_ready: boolean
    /** Whether a plate solve is currently in progress */
    is_solving: boolean
    current_target: CatalogEntry | null
    last_position: Coordinate | null
    direction: PushToStatusDirection | null
    solve: SolveStatus
    /** Why solving is idle, as `push_to_blocked` last said it. */
    blocker: string | null
}

/** Catalog entry response */
export interface CatalogEntry {
    designation: string
    name?: string | null
    catalog_type: string
    ra_degrees: number
    dec_degrees: number
    ra_string: string
    dec_string: string
    object_type: string
    magnitude?: number | null
    constellation: string
    /** Messier number ("M4") when the object is in the Messier catalog */
    messier?: string | null
    /** The alias or identifier a search matched ("C 69"), when neither designation nor name did */
    matched_name?: string | null
}

/** ASTAP installation status response */
export interface AstapStatus {
    /** Whether ASTAP CLI binary is installed and executable */
    binary_installed: boolean
    /** Path to the ASTAP binary (if installed) */
    binary_path?: string | null
    /** Whether at least one star database is installed */
    database_installed: boolean
    /** Path to the primary database directory (if installed) */
    database_path?: string | null
    /** Primary installed database type (if any) */
    database_type?: string | null
    /** All installed databases with their paths */
    installed_databases: InstalledDatabaseInfo[]
    /** Whether the system is ready for plate solving */
    ready: boolean
}

/** Available database types for installation */
export interface DatabaseType {
    /** Database identifier (D80, G05, W08) */
    id: string
    /** Human-readable description */
    description: string
    /** Minimum FOV in degrees this database supports */
    min_fov_deg: number
    /** Maximum FOV in degrees this database supports */
    max_fov_deg: number
    /** Approximate download size (e.g., "~3GB") */
    size: string
    /** Whether this database is already installed */
    installed: boolean
}

/** Catalog installation status response */
export interface CatalogStatus {
    /** Whether the catalog is installed */
    installed: boolean
    /** Path to the catalog directory (if installed) */
    catalog_path?: string | null
    /** Whether NGC.csv exists */
    ngc_file_exists: boolean
    /** Whether addendum.csv exists */
    addendum_file_exists: boolean
    /** Whether hyg_stars.csv exists */
    hyg_file_exists: boolean
    /** Number of objects loaded (if catalog was parsed) */
    object_count?: number | null
}

/** What `GET /api/ai-compute` and `POST /api/ai-compute/benchmark` answer. */
export interface AiComputeReport {
    /** Advances on every change, so a watcher can tell without comparing reports. */
    generation: number
    state: BenchmarkState
    progress: BenchmarkProgress | null
    /** One entry per rung, in ladder order. */
    rungs: RungReport[]
    /** What Auto picks: the fastest usable unit other than the CPU, else the CPU. */
    auto: ComputeRung | null
    /** Where the network runs for the preference asked about. */
    effective: ComputeRung | null
    /** Something the observer should know, e.g. a forced rung this machine cannot use. */
    notice: string | null
}

/** What `GET`/`POST /api/about/license` answer. */
export interface LicenseStatus {
    active: boolean
    details: LicenseDetails | null
}

/** What `GET /api/about/software-licenses` answers. */
export interface SoftwareLicenses {
    version: string
    core_license: string
    third_party_licenses: string | null
}

/** Answer to a Disconnect. */
export interface Disconnection {
    message: string
    camera_id: string
    /** The camera is warming up and disconnects on its own once warm. */
    warming_up: boolean
    /** Seconds until a warm-up is cut short, at the latest. */
    warmup_remaining_s?: number | null
}

/** Simple message response */
export interface Message {
    message: string
    camera_id?: string | null
}

/** Body and answer of `PUT /api/view/camera`: the camera the eyepiece pages show. */
export interface ViewedCamera {
    camera: CameraRole
}

/** The "AI compute" setting: let the benchmark decide, or force one rung. */
export const AI_COMPUTE_PREFERENCE_VALUES = ['auto', 'npu', 'discrete_gpu', 'integrated_gpu', 'cpu'] as const
export type AiComputePreference = (typeof AI_COMPUTE_PREFERENCE_VALUES)[number]

/** Region of interest for alignment */
export interface AlignmentRoi {
    /** X coordinate of top-left corner */
    x: number
    /** Y coordinate of top-left corner */
    y: number
    /** Width of the ROI */
    width: number
    /** Height of the ROI */
    height: number
}

/** Algorithm used for background extraction */
export const BACKGROUND_EXTRACTION_ALGORITHM_VALUES = ['grid_bilinear', 'rbf'] as const
export type BackgroundExtractionAlgorithm = (typeof BACKGROUND_EXTRACTION_ALGORITHM_VALUES)[number]

export interface BenchmarkProgress {
    /** Units finished, and how many there are. */
    done: number
    total: number
    /** The unit being measured, as the UI names it. */
    testing: string | null
}

export const BENCHMARK_STATE_VALUES = ['ready', 'unavailable', 'checking', 'benchmarking'] as const
export type BenchmarkState = (typeof BENCHMARK_STATE_VALUES)[number]

/**
 * Hardware-specific capture settings scoped to a single camera
 * (keyed by `"{provider}/{model_name}"` in `CaptureSettings::camera_profiles`).
 *
 * These are swapped into the flat `CaptureSettings` fields on connect so the
 * rest of the pipeline (capture loop, cooler monitor, UI DTO) stays unaware
 * of the per-camera indirection.
 */
export interface CameraCaptureProfile {
    exposure_us: number
    gain: number
    offset: number
    bin: number
    cooler_enabled: boolean
    target_temp_c: number | null
    sensor_mode_override: DualSamplingMode | null
    cooler_fast_mode: boolean
    dew_heater_enabled: boolean
    dew_heater_power: number
}

/** DTO for CameraPhase serialization (snake_case to match JS event handling). */
export const CAMERA_PHASE_VALUES = [
    'disconnected',
    'idle',
    'precooling',
    'capturing',
    'guiding',
    'warming_up',
    'recovering',
] as const
export type CameraPhase = (typeof CAMERA_PHASE_VALUES)[number]

/**
 * What a connected camera is for.
 *
 * The rig holds at most one of each: the imaging camera on the main scope, and an
 * optional guide camera on a guide scope. The role — not the camera id — is what
 * every consumer addresses, because "which handle drives the stack" and "which
 * handle feeds the plate solver" are questions about the *position* a camera
 * occupies, not about the device that happens to be in it tonight.
 */
export const CAMERA_ROLE_VALUES = ['main', 'guide'] as const
export type CameraRole = (typeof CAMERA_ROLE_VALUES)[number]

/** DTO for CaptureState serialization */
export const CAPTURE_STATE_VALUES = [
    'Idle',
    'Starting',
    'Capturing',
    'Stopping',
    'Error',
    'Recovering',
] as const
export type CaptureState = (typeof CAPTURE_STATE_VALUES)[number]

export interface CometCapabilities {
    pro_stacking: boolean
}

/** A kind of unit the network can run on, in the order the benchmark tries them. */
export const COMPUTE_RUNG_VALUES = ['npu', 'discrete_gpu', 'integrated_gpu', 'cpu'] as const
export type ComputeRung = (typeof COMPUTE_RUNG_VALUES)[number]

/** Coordinate response (simplified) */
export interface Coordinate {
    ra_degrees: number
    dec_degrees: number
    ra_string: string
    dec_string: string
}

export interface DeepSkyCapabilities {
    advanced_rejection: boolean
    rbf_background: boolean
    saturation_boost: boolean
    /**
     * The spatial denoisers and every control over them — Denoise, Colour Mottle,
     * Background Grain, Structure strength. Without them the Noise Reduction section is
     * locked and the render is the plain one.
     */
    denoise: boolean
    /** The AI denoiser behind the "AI denoising" switch; locked without it. */
    ai_denoise: boolean
}

/**
 * Spatial denoising of the streamed image.
 *
 * Runs in the encoders at the resolution the client asked for, not at sensor
 * resolution: the filters cost `1/4.5` as much there and the box downsample
 * has already halved the noise before they start. Both are off for
 * `StackingType::Planetary`, where the fine detail lucky imaging exists to
 * recover is exactly what a denoiser removes.
 */
export interface DenoiseSettings {
    /**
     * The master switch, and a Pro control like the rest of this block. Off gives
     * `DenoiseConfig::OFF`, which the encoders guarantee is *byte-identical* to the
     * pre-denoise output rather than merely equivalent, so the toggle costs nothing.
     *
     * Deliberately **not** managed by Focus/Finder mode, unlike `chroma` and
     * `luma_strength` (the mode already reaches off through those): forcing this one
     * false once made a saved file migrate the Background Grain dial to `0.0` with
     * nothing to put it back. The observer's master switch stays theirs.
     */
    enabled: boolean
    /**
     * Guided-filter smoothing of the chroma planes, against luminance as the
     * guide. Removes colour mottle; the eye resolves little chroma detail, so
     * this is the cheap half with almost nothing to lose.
     */
    chroma: boolean
    /** How far the chroma planes move toward the filtered result, `0..=1`. */
    chroma_strength: number
    /**
     * How hard the render fights sky grain, `0..=1`: the "Background Grain" control, `0.5` its
     * middle ([`DEFAULT_BACKGROUND_GRAIN`]). Spent cheapest-first by *scale* reached; on a
     * 1440p IMX533 stack the 16-32 and 32-64 px bands dominate perceived grain.
     *
     * - **Below the middle**: the finest wavelet level (`LumaDenoiseConfig::thresholds_for`)
     *   and the tone-curve split (`AutoStretchConfig::grain_split`, `MIN_GRAIN_SPLIT` up to
     *   `DEFAULT_GRAIN_SPLIT`): brightness for fine speckle, moving 8-128 px noise under 2%.
     * - **Above the middle**: wavelet levels 5-6, the only reach into 16-64 px: 8-128 px noise
     *   down 12-15% for 3% of target brightness over six sessions, vs. the curve's 1:1.
     *
     * The split never reaches `MAX_GRAIN_SPLIT`: 1/4 cost 38-43% of brightness for 37-39% of
     * grain, the same 1:1 exchange, not a trade a user control should offer.
     */
    background_grain: number
    /**
     * Scales the mid-scale wavelet thresholds (levels 2-4), `0..=1`. `1.0` is both the
     * tuned value and the ceiling; `0.0` is the wavelet's genuine off switch.
     *
     * Note what this can and cannot reach: it leaves the finest level and the coarse
     * pair alone, so it is the "the target still looks too noisy" control, not the "the
     * sky still looks grainy" one — that is the Background Grain dial.
     */
    luma_strength: number
    /**
     * How much the wavelet raises the target's own structure, `0..=1`. The "Detail"
     * control; `0` renders exactly as before it existed.
     *
     * A gain on the levels [`luma_strength`](Self::luma_strength) thresholds, applied to
     * what survived the threshold and held off the sky and every star by the plugin — so
     * it rides on the brightness denoiser and does nothing while that is off.
     */
    detail: number
    /**
     * The AI denoiser (Pro): a small network run after the stretch, taking over the
     * finest four wavelet levels and Detail while it runs. Off by default. Needs the
     * master switch on, like every filter here.
     *
     * Not managed by Focus/Finder mode either: the mode holds the network off where the
     * frame's config is built (`stage_config::requested_denoise`), so this stays the
     * observer's value — a switch the mode forced false once cost a saved file its dial.
     */
    ai: boolean
    /**
     * The "AI compute" choice (Pro): which unit runs the network. Auto takes the hardware
     * benchmark's pick; a forced rung this machine cannot use falls back to Auto. Unknown
     * values read as Auto ([`AiComputePreference`]'s own `Deserialize`).
     */
    ai_compute: AiComputePreference
}

/**
 * A push direction's compass label, in the celestial frame; the wire spells it `N`…`NW`,
 * and `OK` when on target.
 */
export const DIRECTION_HINT_VALUES = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW', 'OK'] as const
export type DirectionHint = (typeof DIRECTION_HINT_VALUES)[number]

/**
 * Dual sampling sensor mode (Player One terminology). Only meaningful for
 * cameras that advertise sensor-mode selection — other providers ignore it.
 */
export const DUAL_SAMPLING_MODE_VALUES = ['normal', 'low_readout_noise'] as const
export type DualSamplingMode = (typeof DUAL_SAMPLING_MODE_VALUES)[number]

/** Settings specifically for the eyepiece view feature */
export interface EyepieceSettings {
    /** Enable Binoview */
    binoview: boolean
    /** Screen width */
    screen_width: number
    /** Screen height */
    screen_height: number
    /** Measurement unit (e.g. "mm", "inches") */
    screen_measurement: string
    /** Screen resolution X */
    screen_resolution_x: number
    /** Screen resolution Y */
    screen_resolution_y: number
    /** Enable Circular view */
    circular_view: boolean
    /** Dark background enhancement intensity (0.0 to 1.0) */
    intensity: number
    /**
     * Where black sits, as a signed fraction of full scale, in `[-0.09, 0.5]`.
     * **Positive** lifts the output floor — an OLED shows a zero pixel fully off, and
     * the autostretch black point clamps a few percent of sky pixels to zero, reading
     * as black speckle at the eyepiece. **Negative** pushes the sky toward black
     * instead, anchored to the sky rather than full scale: `-0.052` puts the floor at
     * sky level whether the sky is nominal or bright, so one setting behaves the same
     * on every target (sky otherwise sits at 14-17 output levels, a visible grey —
     * dimming via the stretch would dim the target too; see [`intensity`](Self::intensity)).
     */
    black_floor: number
    /**
     * Let the darkening half of `black_floor` clip to true black instead of
     * scaling the sky down (`render::output::sky_shadow`).
     *
     * Sky noise is as wide as the sky level, so a hard floor puts around 40 %
     * of all samples on exactly zero. That buys the deepest possible sky and a
     * little more separation between target and background, and costs the black
     * speckle the positive half of `black_floor` exists to remove. Off by
     * default; no effect while `black_floor` is positive.
     */
    darker_sky: boolean
    /**
     * Blue-noise dithering at the 8-bit conversion, to keep smooth gradients from
     * banding once denoising removes the noise that currently masks the steps.
     */
    dither: boolean
    /** The RGB8+LZ4 size every `/eyepiece_quality` client receives. */
    stream_resolution: EyepieceStreamResolution
}

/**
 * The subset of [`Resolution`] the eyepiece quality stream offers. No 1080p: the eyepiece
 * screens this view exists for are at least 1440 px on their short edge, and a variant
 * that cannot be stored cannot be selected by a stale client either — `serde` rejects it.
 */
export const EYEPIECE_STREAM_RESOLUTION_VALUES = ['native', 'uhd2160', 'qhd1440'] as const
export type EyepieceStreamResolution = (typeof EYEPIECE_STREAM_RESOLUTION_VALUES)[number]

/** Information about a single installed database */
export interface InstalledDatabaseInfo {
    /** Database identifier (D80, G05, W08) */
    id: string
    /** Path to this database's directory */
    database_path: string
    /** Minimum FOV in degrees this database supports */
    min_fov_deg: number
    /** Maximum FOV in degrees this database supports */
    max_fov_deg: number
}

export interface LicenseDetails {
    name: string
    email: string
    issued_at: string
    expires_at: string
}

export interface PlanetaryCapabilities {
    advanced_stacking: boolean
}

export interface PushToCapabilities {
    astap_solver: boolean
}

/** The plugin's direction with the field of view the last announced one carried. */
export interface PushToStatusDirection {
    /**
     * Angle to push in degrees, in the image frame:
     * 0 = screen up, 90 = screen right, rotation is clockwise.
     * The plate-solved camera rotation and parity are already applied, so
     * this can be used directly as an SVG/CSS rotation for a chevron that
     * points "up" at 0°. For a celestial-frame label use `direction_hint`.
     */
    angle_deg: number
    /** Angular distance to target in degrees */
    distance_deg: number
    /** Whether within fine-adjustment range (<1 degree) */
    is_close: boolean
    /** Which way to push, in the celestial frame; `OK` once on target. */
    direction_hint: DirectionHint
    /** Current position (if solved) */
    current_position: Coordinate | null
    /** Target position */
    target: Coordinate | null
    /**
     * The camera whose plate solve oriented `angle_deg`, `None` if unknown. A guide
     * camera solves for both views, and its rotation and parity are not the imaging
     * camera's, so a client draws the chevron only over this camera's picture.
     */
    solved_camera: string | null
    fov_deg: number | null
}

/**
 * Which capture modes write their raw frames to disk.
 *
 * Independent switches rather than one boolean plus a mode gate: the modes are not
 * alternatives to each other, they are separate occasions on which the same decision
 * applies, and an observer who wants raw subs from a Wanderer sweep does not thereby
 * want them from every focusing run. `guide` is the same idea one step further out —
 * it is an occasion that runs *alongside* the other three rather than instead of them,
 * so a night can be saving imaging subs and guide subs at once, into separate folders.
 */
export interface RawFrameSaving {
    live_view: boolean
    wanderer: boolean
    stacking: boolean
    guide: boolean
}

/** Rejection method for stacking. */
export const REJECTION_METHOD_VALUES = ['None', 'SigmaClip', 'WinsorizedSigmaClip', 'MinMax'] as const
export type RejectionMethod = (typeof REJECTION_METHOD_VALUES)[number]

/**
 * A resolution the observer picks, as a bounding box: the frame is fitted inside it with
 * its aspect ratio intact and never upscaled. Serves three settings:
 *
 * - **Processing Resolution** (`preview_resolution`): how far the preview pipeline may bin
 *   first, which runs its every-sample stages on a quarter of the data. A setting, not
 *   derived from clients: re-deriving it per frame moved the tone curve under every viewer
 *   (**25.7% shadow lift at the 1% input point**) when someone else's tab joined.
 * - **Streaming Resolution** (`streaming_resolution`): the one JPEG size every `/` and
 *   `/eyepiece` client receives.
 * - **Eyepiece Streaming Resolution** ([`EyepieceStreamResolution`]): the one RGB8+LZ4 size
 *   every `/eyepiece_quality` client receives.
 */
export const RESOLUTION_VALUES = ['uhd2160', 'qhd1440', 'hd1080', 'native'] as const
export type Resolution = (typeof RESOLUTION_VALUES)[number]

/** The best unit found on one rung, or why there is none. */
export interface RungReport {
    rung: ComputeRung
    usable: boolean
    device: string | null
    /** The API and driver it runs through, e.g. "Vulkan 1.3 · Mesa 25.0.7". */
    api: string | null
    precision: string | null
    /** Median time for one benchmark frame, hand-off to results. */
    ms_per_frame: number | null
    /** Why it is not usable, when it is not. */
    reason: string | null
}

/**
 * Corrections that run on the raw CFA mosaic, before demosaic.
 *
 * These target defects stacking cannot remove: a hot pixel and a readout offset sit
 * in the same place in every sub, so averaging leaves them untouched. Both are only
 * well defined on the mosaic — after demosaic a hot site is smeared into a coloured 3x3 cross, and neighbouring sensor rows are mixed together.
 *
 * Hot-pixel rejection has no switch: it always runs, `hot_pixel_sigma` is its only
 * tuning. See `stage_config::build_cfa_pipeline` for why.
 */
export interface SensorCorrectionSettings {
    /**
     * How far above its brightest same-colour neighbour a sample must sit to
     * count as hot, in sigmas of that colour site's own noise.
     */
    hot_pixel_sigma: number
    /** Flatten per-row and per-column readout offsets. */
    fpn_removal: boolean
    /**
     * Bin each 2x2 CFA quad to one RGB pixel instead of interpolating.
     *
     * Halves both dimensions. Free on a sensor that oversamples the eyepiece
     * screen (IMX533's 3008² becomes 1504², still above 1440²) and a real loss
     * on one that does not (IMX464 lands at 1356x769), which is why it is off
     * by default.
     */
    superpixel_debayer: boolean
}

/** Camera sensor mode DTO (dual sampling mode slot) */
export interface SensorMode {
    index: number
    name: string
    description?: string
}

/**
 * Which attempt of a multi-strategy solve is running — see
 * [`ServerEvent::PlateSolvingProgress`](super::ServerEvent::PlateSolvingProgress).
 */
export interface SolveStage {
    stage: string
    attempt: number
    total: number
}

/** Whether a plate solve is running, and how the last one ended. */
export interface SolveStatus {
    in_progress: boolean
    target_name: string | null
    stage: SolveStage | null
    /** `None` until a solve ends, and after a restart or a cleared target. */
    last_result: SolveVerdict | null
}

/** How the last solve ended. */
export const SOLVE_VERDICT_VALUES = ['success', 'failed', 'cancelled'] as const
export type SolveVerdict = (typeof SOLVE_VERDICT_VALUES)[number]

/** Available stacking types */
export const STACKING_TYPE_VALUES = ['deep_sky', 'planetary', 'comet'] as const
export type StackingType = (typeof STACKING_TYPE_VALUES)[number]

/**
 * The stretch modes: Star Fields (`Low`) and Deep Sky (`Medium`). Nebulae (`high`) was
 * dropped once the background model stopped leaving glow behind: it lifted the sky ~6
 * output levels for 1-4 on the target, and Deep Sky's target-to-grain matched or beat
 * it on nine of ten real stacks (M33 6.1 vs 4.3). Old settings files are migrated.
 */
export const STRETCH_AGGRESSIVENESS_VALUES = ['low', 'medium'] as const
export type StretchAggressiveness = (typeof STRETCH_AGGRESSIVENESS_VALUES)[number]

/**
 * Telescope and camera parameters for FOV calculation
 *
 * `PartialEq` so callers can act on a telescope block that actually *changed*
 * rather than one that was merely present in the request. Every field feeds the
 * FOV, so any difference matters and a whole-struct comparison is the right test.
 */
export interface TelescopeSettings {
    /** Telescope focal length in mm */
    focal_length_mm: number | null
    /** Pixel size X in micrometers (manual override or from camera database) */
    pixel_size_x_um: number | null
    /** Pixel size Y in micrometers (manual override or from camera database) */
    pixel_size_y_um: number | null
    /** Sensor width in pixels */
    sensor_width_px: number | null
    /** Sensor height in pixels */
    sensor_height_px: number | null
    /** Barlow/reducer coefficient (effective_fl = focal_length * coeff; default 1.0) */
    barlow_coeff: number | null
}

/** Weighting preset for quality-based frame weighting during stacking */
export const WEIGHTING_PRESET_VALUES = [
    'disabled',
    'balanced',
    'galaxies',
    'nebulae',
    'fwhm_only',
    'snr_only',
] as const
export type WeightingPreset = (typeof WEIGHTING_PRESET_VALUES)[number]

// Request bodies, as the server reads them: a field it defaults may be left out.

/** Body of `POST /api/settings`: the settings to change, every other one left as it is. */
export interface UpdateSettingsRequest {
    /**
     * Which camera the hardware fields below (exposure, gain, offset, bin, cooler and
     * dew heater) apply to. Absent means the imaging camera.
     */
    camera_role?: CameraRole | null
    exposure_us?: number | null
    gain?: number | null
    offset?: number | null
    bin?: number | null
    auto_stretch?: boolean | null
    stacking?: boolean | null
    rejection_sigma?: number | null
    rejection_method?: RejectionMethod | null
    background_subtraction?: boolean | null
    /** Algorithm for background extraction */
    background_extraction_algorithm?: BackgroundExtractionAlgorithm | null
    /** Which capture modes write their raw frames to disk */
    raw_frame_saving?: RawFrameSavingInput | null
    save_stacked_image?: boolean | null
    stacking_type?: StackingType | null
    /** Quality-based frame weighting preset for stacking */
    weighting_preset?: WeightingPreset | null
    /** Auto stretch aggressiveness (Low, Medium, High) */
    stretch_aggressiveness?: StretchAggressiveness | null
    /** Auto Stretch intensity multiplier */
    auto_stretch_intensity?: number | null
    /** Enable shadow saturation boost */
    saturation_boost?: boolean | null
    /** Shadow saturation boost strength (0.0-1.0) */
    saturation_boost_strength?: number | null
    /** Use simulated camera */
    use_simulated_camera?: boolean | null
    /** Number of images to preload for simulated camera */
    simulated_preload_images?: number | null
    /** Show the focus image when waiting for frames */
    show_focus_image?: boolean | null
    /** Force showing the focus image even when the stream is active */
    force_focus_image_now?: boolean | null
    /** Region of interest for comet nucleus tracking (used in Comet stacking mode) */
    comet_roi?: AlignmentRoi | null
    /** Region of interest for planetary alignment (used in Planetary stacking mode) */
    planetary_roi?: AlignmentRoi | null
    /** Enable auto tracking of planetary ROI */
    planetary_auto_tracking?: boolean | null
    /** Enable multi-point alignment for planetary (Pro only) */
    planetary_multi_point_alignment?: boolean | null
    /** Enable "Wanderer" mode */
    wanderer_mode?: boolean | null
    /** Crop a deep-sky stack to its well-covered area */
    auto_crop?: boolean | null
    auto_reconnect?: boolean | null
    auto_resume_capture?: boolean | null
    sensor_correction?: SensorCorrectionSettingsInput | null
    denoise?: DenoiseSettingsInput | null
    preview_resolution?: Resolution | null
    streaming_resolution?: Resolution | null
    eyepiece?: EyepieceSettingsInput | null
    telescope?: TelescopeSettingsInput | null
    /** Per-camera telescope profiles keyed by camera name, upserted: one left out is kept */
    camera_telescope_profiles?: Partial<Record<string, TelescopeSettingsInput>> | null
    /**
     * Per-camera capture profiles (mainly for tests to seed the map without
     * going through a camera connect).
     */
    camera_profiles?: Partial<Record<string, CameraCaptureProfileInput>> | null
    /** Name of the last active camera */
    last_camera_name?: string | null
    /** Whether the cooler should be active during capture */
    cooler_enabled?: boolean | null
    /**
     * Target sensor temperature in Celsius. Absent or `null` leaves the setpoint as it
     * is: a request cannot clear one.
     */
    target_temp_c?: number | null
    /** Bypass the 5 °C/min cool/warm ramp (advanced users only) */
    cooler_fast_mode?: boolean | null
    sensor_mode_override?: DualSamplingMode | null
    /** Whether anti-dew heater is enabled */
    dew_heater_enabled?: boolean | null
    /** Anti-dew heater power level (0-100) */
    dew_heater_power?: number | null
    /** Whether the user has accepted the End User License Agreement */
    eula_accepted?: boolean | null
    /** INDI server host */
    indi_server_host?: string | null
    /** INDI server port */
    indi_server_port?: number | null
    /**
     * Enter or leave Focus/Finder mode. Applied after every other field in the
     * request, so it wins over a managed setting sent alongside it.
     */
    focus_mode?: boolean | null
}

/**
 * Body of `POST /api/capture/start`.
 *
 * `role` says which camera the button was pressed for, mirroring
 * [`ConnectCameraRequest`]: the panel edits whichever camera is selected in the list,
 * so Start has to address the same one. Absent means the imaging camera, so a client
 * that has never heard of roles keeps working.
 */
export interface StartCaptureRequest {
    camera_id?: string | null
    role?: CameraRole | null
}

/** Body of `POST /api/capture/stop`. See [`StartCaptureRequest`] for `role`. */
export interface StopCaptureRequest {
    role?: CameraRole | null
}

/**
 * Body of `POST /api/cameras/{id}/connect`.
 *
 * Absent (or an empty body) means the imaging camera, so a client that has never heard
 * of roles keeps working.
 */
export interface ConnectCameraRequest {
    role?: CameraRole | null
}

/** Body of `POST /api/cameras/{id}/disconnect`. Absent means an ordinary Disconnect. */
export interface DisconnectCameraRequest {
    /** Close at once instead of warming a cooled camera up first. */
    skip_warmup?: boolean
}

/** Configure simulated camera request */
export interface ConfigureSimulatorRequest {
    /** Path to directory containing image files */
    directory: string
}

/** Body of `POST /api/push-to/target`: a name, which wins, or both coordinates. */
export interface SetTargetRequest {
    /** Target name (e.g., "M31", "NGC 7000", "Andromeda Galaxy") */
    name?: string | null
    /** Or set by coordinates */
    ra_degrees?: number | null
    dec_degrees?: number | null
}

/** Query of `GET /api/push-to/catalog/search`. */
export interface SearchCatalogRequest {
    /** Search query */
    query: string
    /** Maximum results to return */
    limit?: number
}

/** Push-To configuration request */
export interface PushToConfigRequest {
    /** Field of view hint in degrees */
    fov_degrees?: number | null
    /** Path to solver database */
    database_path?: string | null
}

/** ASTAP installation request */
export interface AstapInstallRequest {
    /** Which databases to install (D80, G05, W08) */
    database_types?: string[]
    /** Legacy single database field for backward compatibility */
    database_type?: string | null
}

/** Catalog installation request */
export interface CatalogInstallRequest {
    /** Whether to also download the HYG star database (~15MB compressed) */
    include_stars?: boolean
}

/** Body of `POST /api/about/license`. */
export interface UpdateLicenseRequest {
    token: string
}

/**
 * Hardware-specific capture settings scoped to a single camera
 * (keyed by `"{provider}/{model_name}"` in `CaptureSettings::camera_profiles`).
 *
 * These are swapped into the flat `CaptureSettings` fields on connect so the
 * rest of the pipeline (capture loop, cooler monitor, UI DTO) stays unaware
 * of the per-camera indirection.
 */
export interface CameraCaptureProfileInput {
    exposure_us: number
    gain: number
    offset: number
    bin: number
    cooler_enabled: boolean
    target_temp_c?: number | null
    sensor_mode_override?: DualSamplingMode | null
    cooler_fast_mode?: boolean
    dew_heater_enabled?: boolean
    dew_heater_power?: number
}

/**
 * Spatial denoising of the streamed image.
 *
 * Runs in the encoders at the resolution the client asked for, not at sensor
 * resolution: the filters cost `1/4.5` as much there and the box downsample
 * has already halved the noise before they start. Both are off for
 * `StackingType::Planetary`, where the fine detail lucky imaging exists to
 * recover is exactly what a denoiser removes.
 */
export interface DenoiseSettingsInput {
    /**
     * The master switch, and a Pro control like the rest of this block. Off gives
     * `DenoiseConfig::OFF`, which the encoders guarantee is *byte-identical* to the
     * pre-denoise output rather than merely equivalent, so the toggle costs nothing.
     *
     * Deliberately **not** managed by Focus/Finder mode, unlike `chroma` and
     * `luma_strength` (the mode already reaches off through those): forcing this one
     * false once made a saved file migrate the Background Grain dial to `0.0` with
     * nothing to put it back. The observer's master switch stays theirs.
     */
    enabled?: boolean
    /**
     * Guided-filter smoothing of the chroma planes, against luminance as the
     * guide. Removes colour mottle; the eye resolves little chroma detail, so
     * this is the cheap half with almost nothing to lose.
     */
    chroma?: boolean
    /** How far the chroma planes move toward the filtered result, `0..=1`. */
    chroma_strength?: number
    /**
     * How hard the render fights sky grain, `0..=1`: the "Background Grain" control, `0.5` its
     * middle ([`DEFAULT_BACKGROUND_GRAIN`]). Spent cheapest-first by *scale* reached; on a
     * 1440p IMX533 stack the 16-32 and 32-64 px bands dominate perceived grain.
     *
     * - **Below the middle**: the finest wavelet level (`LumaDenoiseConfig::thresholds_for`)
     *   and the tone-curve split (`AutoStretchConfig::grain_split`, `MIN_GRAIN_SPLIT` up to
     *   `DEFAULT_GRAIN_SPLIT`): brightness for fine speckle, moving 8-128 px noise under 2%.
     * - **Above the middle**: wavelet levels 5-6, the only reach into 16-64 px: 8-128 px noise
     *   down 12-15% for 3% of target brightness over six sessions, vs. the curve's 1:1.
     *
     * The split never reaches `MAX_GRAIN_SPLIT`: 1/4 cost 38-43% of brightness for 37-39% of
     * grain, the same 1:1 exchange, not a trade a user control should offer.
     */
    background_grain?: number
    /**
     * Scales the mid-scale wavelet thresholds (levels 2-4), `0..=1`. `1.0` is both the
     * tuned value and the ceiling; `0.0` is the wavelet's genuine off switch.
     *
     * Note what this can and cannot reach: it leaves the finest level and the coarse
     * pair alone, so it is the "the target still looks too noisy" control, not the "the
     * sky still looks grainy" one — that is the Background Grain dial.
     */
    luma_strength?: number
    /**
     * How much the wavelet raises the target's own structure, `0..=1`. The "Detail"
     * control; `0` renders exactly as before it existed.
     *
     * A gain on the levels [`luma_strength`](Self::luma_strength) thresholds, applied to
     * what survived the threshold and held off the sky and every star by the plugin — so
     * it rides on the brightness denoiser and does nothing while that is off.
     */
    detail?: number
    /**
     * The AI denoiser (Pro): a small network run after the stretch, taking over the
     * finest four wavelet levels and Detail while it runs. Off by default. Needs the
     * master switch on, like every filter here.
     *
     * Not managed by Focus/Finder mode either: the mode holds the network off where the
     * frame's config is built (`stage_config::requested_denoise`), so this stays the
     * observer's value — a switch the mode forced false once cost a saved file its dial.
     */
    ai?: boolean
    /**
     * The "AI compute" choice (Pro): which unit runs the network. Auto takes the hardware
     * benchmark's pick; a forced rung this machine cannot use falls back to Auto. Unknown
     * values read as Auto ([`AiComputePreference`]'s own `Deserialize`).
     */
    ai_compute?: AiComputePreference
}

/** Settings specifically for the eyepiece view feature */
export interface EyepieceSettingsInput {
    /** Enable Binoview */
    binoview: boolean
    /** Screen width */
    screen_width: number
    /** Screen height */
    screen_height: number
    /** Measurement unit (e.g. "mm", "inches") */
    screen_measurement: string
    /** Screen resolution X */
    screen_resolution_x: number
    /** Screen resolution Y */
    screen_resolution_y: number
    /** Enable Circular view */
    circular_view?: boolean
    /** Dark background enhancement intensity (0.0 to 1.0) */
    intensity?: number
    /**
     * Where black sits, as a signed fraction of full scale, in `[-0.09, 0.5]`.
     * **Positive** lifts the output floor — an OLED shows a zero pixel fully off, and
     * the autostretch black point clamps a few percent of sky pixels to zero, reading
     * as black speckle at the eyepiece. **Negative** pushes the sky toward black
     * instead, anchored to the sky rather than full scale: `-0.052` puts the floor at
     * sky level whether the sky is nominal or bright, so one setting behaves the same
     * on every target (sky otherwise sits at 14-17 output levels, a visible grey —
     * dimming via the stretch would dim the target too; see [`intensity`](Self::intensity)).
     */
    black_floor?: number
    /**
     * Let the darkening half of `black_floor` clip to true black instead of
     * scaling the sky down (`render::output::sky_shadow`).
     *
     * Sky noise is as wide as the sky level, so a hard floor puts around 40 %
     * of all samples on exactly zero. That buys the deepest possible sky and a
     * little more separation between target and background, and costs the black
     * speckle the positive half of `black_floor` exists to remove. Off by
     * default; no effect while `black_floor` is positive.
     */
    darker_sky?: boolean
    /**
     * Blue-noise dithering at the 8-bit conversion, to keep smooth gradients from
     * banding once denoising removes the noise that currently masks the steps.
     */
    dither?: boolean
    /** The RGB8+LZ4 size every `/eyepiece_quality` client receives. */
    stream_resolution?: EyepieceStreamResolution
}

/**
 * Which capture modes write their raw frames to disk.
 *
 * Independent switches rather than one boolean plus a mode gate: the modes are not
 * alternatives to each other, they are separate occasions on which the same decision
 * applies, and an observer who wants raw subs from a Wanderer sweep does not thereby
 * want them from every focusing run. `guide` is the same idea one step further out —
 * it is an occasion that runs *alongside* the other three rather than instead of them,
 * so a night can be saving imaging subs and guide subs at once, into separate folders.
 */
export interface RawFrameSavingInput {
    live_view?: boolean
    wanderer?: boolean
    stacking?: boolean
    guide?: boolean
}

/**
 * Corrections that run on the raw CFA mosaic, before demosaic.
 *
 * These target defects stacking cannot remove: a hot pixel and a readout offset sit
 * in the same place in every sub, so averaging leaves them untouched. Both are only
 * well defined on the mosaic — after demosaic a hot site is smeared into a coloured 3x3 cross, and neighbouring sensor rows are mixed together.
 *
 * Hot-pixel rejection has no switch: it always runs, `hot_pixel_sigma` is its only
 * tuning. See `stage_config::build_cfa_pipeline` for why.
 */
export interface SensorCorrectionSettingsInput {
    /**
     * How far above its brightest same-colour neighbour a sample must sit to
     * count as hot, in sigmas of that colour site's own noise.
     */
    hot_pixel_sigma?: number
    /** Flatten per-row and per-column readout offsets. */
    fpn_removal?: boolean
    /**
     * Bin each 2x2 CFA quad to one RGB pixel instead of interpolating.
     *
     * Halves both dimensions. Free on a sensor that oversamples the eyepiece
     * screen (IMX533's 3008² becomes 1504², still above 1440²) and a real loss
     * on one that does not (IMX464 lands at 1356x769), which is why it is off
     * by default.
     */
    superpixel_debayer?: boolean
}

/**
 * Telescope and camera parameters for FOV calculation
 *
 * `PartialEq` so callers can act on a telescope block that actually *changed*
 * rather than one that was merely present in the request. Every field feeds the
 * FOV, so any difference matters and a whole-struct comparison is the right test.
 */
export interface TelescopeSettingsInput {
    /** Telescope focal length in mm */
    focal_length_mm?: number | null
    /** Pixel size X in micrometers (manual override or from camera database) */
    pixel_size_x_um?: number | null
    /** Pixel size Y in micrometers (manual override or from camera database) */
    pixel_size_y_um?: number | null
    /** Sensor width in pixels */
    sensor_width_px?: number | null
    /** Sensor height in pixels */
    sensor_height_px?: number | null
    /** Barlow/reducer coefficient (effective_fl = focal_length * coeff; default 1.0) */
    barlow_coeff?: number | null
}
