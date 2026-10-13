<script setup>
import {ref, inject, computed, watch, onMounted} from 'vue'
import {startCapture, stopCapture, getStackingTypes} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {findCamera, useApplySettings, useCapabilities} from '../composables/appContext.js'
import {hardwareBlock} from '../composables/useCameraHardwareSettings.js'
import {useCaptureTarget} from '../composables/useCaptureTarget.js'
import {BaseAlert, BaseToggle, BaseInfoIcon, ButtonGroup, BaseProLock, BasePanel, BaseSlider} from './ui'
import ExposureControl from './ExposureControl.vue'
import {
  CAMERA_PHASES,
  CAPTURE_MODE_OPTIONS,
  GAIN_LIMITS,
  GUIDE_CAPTURE_MODE_OPTIONS,
  DEFAULT_SETTINGS,
  STACKING_TYPES,
  STRETCH_AGGRESSIVENESS_OPTIONS,
  HELP_TEXTS
} from '../constants'

const settings = inject('settings')
const selectedCamera = inject('selectedCamera')
const applySettings = useApplySettings()
const cameras = inject('cameras', {value: []})
const cameraPhase = inject('cameraPhase', {value: {}})
const {cometPro} = useCapabilities()

const {error, loading, clearError, setError, withErrorHandling} = useError()

/**
 * Which camera this panel addresses.
 *
 * Clicking a camera in the list picks it; the guide camera keeps its own exposure and
 * gain, so the same two controls address a different set of values depending on the
 * selection. Start/Stop follows the same selection — see `isCapturing` below.
 */
const settingsRole = inject('selectedCameraRole', computed(() => 'main'))
const mainCamera = inject('mainCamera', computed(() => null))

const {
  isGuide: isEditingGuide,
  mainCapturing,
  isCapturing,
  isStopping,
  canStart,
  startLabel,
  stopLabel,
} = useCaptureTarget({role: settingsRole, busy: loading})

/** The hardware block belonging to the camera currently selected. */
const cameraSettings = computed(() => hardwareBlock(settings?.value, settingsRole.value))

/** The exposure as typed, in microseconds; Start sends it even if never blurred. */
const exposureUs = ref(100000)
const gain = ref(GAIN_LIMITS.default)

const stackingTypes = ref([])
const selectedStackingType = ref(STACKING_TYPES.DEEP_SKY)
const stackingEnabled = ref(DEFAULT_SETTINGS.stacking)
const autoStretch = ref(DEFAULT_SETTINGS.auto_stretch)
const stretchAggressiveness = ref(DEFAULT_SETTINGS.stretch_aggressiveness)
const wandererMode = ref(DEFAULT_SETTINGS.wanderer_mode)
const focusMode = ref(DEFAULT_SETTINGS.focus_mode)

const stackingMode = computed(() => {
  if (wandererMode.value) return 'wanderer'
  if (stackingEnabled.value) return 'stacking'
  return 'off'
})

// Sync from settings. Keyed on the selected camera too, so switching cameras in the
// list re-reads the controls against that camera's own values.
watch(
    [settings, settingsRole],
    ([allSettings]) => {
      const newSettings = allSettings
      const hardware = hardwareBlock(allSettings, settingsRole.value)
      if (newSettings && hardware) {
        exposureUs.value = hardware.exposure_us
        gain.value = hardware.gain
        if (newSettings.stacking_type) {
          selectedStackingType.value = newSettings.stacking_type
        }
        if (newSettings.stacking !== undefined) {
          stackingEnabled.value = newSettings.stacking
        }
        if (newSettings.auto_stretch !== undefined) {
          autoStretch.value = newSettings.auto_stretch
        }
        if (newSettings.stretch_aggressiveness !== undefined) {
          stretchAggressiveness.value = newSettings.stretch_aggressiveness
        }
        if (newSettings.wanderer_mode !== undefined) {
          wandererMode.value = newSettings.wanderer_mode
        }
        if (newSettings.focus_mode !== undefined) {
          focusMode.value = newSettings.focus_mode
        }
      }
    },
    {immediate: true}
)

// Without the list the Type select is empty, so say why rather than only logging it.
onMounted(async () => {
  try {
    stackingTypes.value = await getStackingTypes()
  } catch (e) {
    setError(`Could not load the stacking types: ${e.message}`)
  }
})

const captureModeOptions = computed(() => (isEditingGuide.value ? GUIDE_CAPTURE_MODE_OPTIONS : CAPTURE_MODE_OPTIONS))

const selectedCameraName = computed(() => {
  const id = selectedCamera.value
  if (!id) return null
  return findCamera(cameras?.value, id)?.name || null
})

const selectedCameraPhase = computed(() => {
  const name = selectedCameraName.value
  return name ? cameraPhase.value?.[name] || null : null
})

const showPrecoolWarning = computed(
    () => selectedCameraPhase.value === CAMERA_PHASES.PRECOOLING && !isCapturing.value
)


async function handleStart() {
  await withErrorHandling(async () => {
    // Flush the exposure and gain shown above before starting, so an edit the user
    // typed but never blurred is not lost. It lands on the camera being started, which
    // is the one those controls are editing.
    await applySettings({
      camera_role: settingsRole.value,
      exposure_us: exposureUs.value,
      gain: gain.value,
    })
    if (isEditingGuide.value) {
      // No id: the guide slot holds at most one camera, so the server resolves it.
      await startCapture(null, 'guide')
      return
    }
    // No fallback to `selectedCamera`: with only a guide camera connected that
    // fallback sent the *guide* camera's id, and the server answered with a
    // role-mismatch message about connecting — which is not what the user did.
    // Sending nothing lets the server resolve the imaging camera itself, and say
    // plainly that there isn't one.
    await startCapture(mainCamera.value?.id ?? null)
  })
}

async function handleStop() {
  await withErrorHandling(async () => {
    await stopCapture(settingsRole.value)
  })
}

async function applySetting(settings, onError = null) {
  await withErrorHandling(async () => {
    await applySettings(settings)
  }, onError ? {onError} : {})
}

/** Hardware edits carry the role, so they land on the camera the user selected. */
const applyHardware = (patch) => applySetting({camera_role: settingsRole.value, ...patch})

const applyExposure = (us) => applyHardware({exposure_us: us})
const applyGain = () => applyHardware({gain: gain.value})
const applyStackingType = () => applySetting({stacking_type: selectedStackingType.value})
const applyAutoStretch = () => applySetting({auto_stretch: autoStretch.value})
const applyStretchAggressiveness = () => applySetting({stretch_aggressiveness: stretchAggressiveness.value})
/**
 * Whether entering Focus/Finder mode would damage a stack already being integrated.
 *
 * Row/column pattern removal (one of six dropped settings) runs before demosaic, so
 * integrated frames lose banding correction that averaging can't undo. Backend refuses
 * with 409; disabling the toggle spares the user that error. Mirrors backend exemptions —
 * live view, planetary, and a stopping capture — leaving is never blocked (`!focusMode`).
 */
const focusModeBlocked = computed(
    () =>
        !focusMode.value &&
        mainCapturing.value &&
        stackingMode.value !== 'off' &&
        selectedStackingType.value !== STACKING_TYPES.PLANETARY
)

// A pipeline setting, not a hardware one: it goes through `applySetting` rather than
// `applyHardware` so it never carries a `camera_role`, and stays available while the
// guide camera is selected — framing is exactly when you want it.
//
// On a failed save the ref is put back to what the server still holds: the Settings
// panel greys its seven toggles off the server value, so an optimistic ref left in place
// would have the two panels disagreeing about the same setting.
const applyFocusMode = () =>
    applySetting({focus_mode: focusMode.value}, () => {
      focusMode.value = settings?.value?.focus_mode ?? DEFAULT_SETTINGS.focus_mode
    })

function applyStackingMode(val) {
  const modes = {
    wanderer: {stacking: true, wanderer_mode: true},
    stacking: {stacking: true, wanderer_mode: false},
    off: {stacking: false, wanderer_mode: false},
  }
  applySetting(modes[val] || modes.off)
}

const showCometLock = computed(() => {
  if (isEditingGuide.value) return false
  return selectedStackingType.value === STACKING_TYPES.COMET && !cometPro.value
})

const HELP = HELP_TEXTS
</script>

<template>
  <BasePanel class="capture-controls">
    <template #header>
      <div class="capture-header-row">
        <div class="header-control-item">
          <label class="type-label-inline">Capture mode</label>
          <ButtonGroup
              :model-value="isEditingGuide ? 'off' : stackingMode"
              :options="captureModeOptions"
              :disabled="isEditingGuide"
              @update:model-value="applyStackingMode"
          />
          <BaseInfoIcon :message="isEditingGuide ? HELP.guide_capture_mode : HELP.stacking"/>
        </div>
      </div>
    </template>

    <template #default>

    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <!-- Type selector and Start/Stop button -->
    <div class="capture-row">
      <div class="type-selector">
        <label class="type-label-inline">
          Type
          <BaseInfoIcon :message="HELP.stacking_type"/>
        </label>
        <select
            v-model="selectedStackingType"
            class="select type-select"
            :disabled="isEditingGuide || isCapturing || isStopping"
            @change="applyStackingType"
        >
          <option v-for="type in stackingTypes" :key="type.id" :value="type.id">
            {{ type.name }}
          </option>
        </select>
      </div>

      <!-- Pro lock for Comet mode -->
      <div v-if="showCometLock" class="comet-pro-lock">
        <BaseProLock
            title="Comet Stacking"
            :message="HELP.comet_pro"
        />
      </div>

      <template v-else>
        <button
            v-if="!isCapturing"
            class="btn btn-capture btn-start"
            :disabled="!canStart || loading"
            @click="handleStart"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor">
            <polygon points="5,3 19,12 5,21"/>
          </svg>
          <span>{{ loading ? 'Starting...' : startLabel }}</span>
        </button>
        <button
            v-else
            class="btn btn-capture btn-stop"
            :disabled="isStopping || loading"
            @click="handleStop"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor">
            <rect x="4" y="4" width="16" height="16" rx="2"/>
          </svg>
          <span>{{ isStopping ? 'Stopping...' : stopLabel }}</span>
        </button>
      </template>
    </div>

    <div v-if="showPrecoolWarning" class="precool-warning">
      {{ HELP.precooling }}
    </div>

    <div class="control-group" :class="{ 'control-disabled': showCometLock }">
      <ExposureControl
          v-model="exposureUs"
          :active-us="cameraSettings?.exposure_us ?? null"
          :disabled="showCometLock"
          @apply="applyExposure"
      />
    </div>

    <div class="control-group" :class="{ 'control-disabled': showCometLock }">
      <div class="control-row">
        <div class="slider-control-wrapper">
          <BaseSlider
              :model-value="gain"
              label="Gain"
              :min="GAIN_LIMITS.min"
              :max="GAIN_LIMITS.max"
              :step="1"
              :disabled="showCometLock"
              :help="HELP.gain"
              show-input
              @update:model-value="gain = $event"
              @change="applyGain"
          />
        </div>
      </div>
    </div>

    <div class="control-group">
      <div class="control-row">
        <label class="type-label-inline" :class="{ 'text-muted': showCometLock }">
          Color mode
          <BaseInfoIcon :message="HELP.auto_stretch"/>
        </label>
        <div class="stretch-controls">
          <BaseToggle
              v-model="autoStretch"
              size="small"
              :disabled="showCometLock"
              @update:model-value="applyAutoStretch"
          />
          <select
              v-if="autoStretch"
              v-model="stretchAggressiveness"
              class="select aggressiveness-select"
              :disabled="showCometLock"
              @change="applyStretchAggressiveness"
          >
            <option v-for="opt in STRETCH_AGGRESSIVENESS_OPTIONS" :key="opt.value" :value="opt.value">
              {{ opt.label }}
            </option>
          </select>
        </div>
      </div>
    </div>

    <div class="control-group">
      <div class="control-row focus-mode-controls">
        <label class="type-label-inline" :class="{ 'text-muted': focusModeBlocked }">
          Focus/Finder mode
          <BaseInfoIcon :message="focusModeBlocked ? HELP.focus_mode_while_stacking : HELP.focus_mode"/>
        </label>
        <BaseToggle
            v-model="focusMode"
            size="small"
            :disabled="focusModeBlocked"
            @update:model-value="applyFocusMode"
        />
      </div>
    </div>
    </template>
  </BasePanel>
</template>

<style scoped>
/* Uses global .panel, .panel-bordered, .panel-header from main.css */

.capture-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.5rem;
}

.type-selector {
  flex: 1.5;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.type-select {
  width: 100%;
  padding: 0.375rem 0.5rem;
  font-size: var(--fs-md);
}

.type-select:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-capture {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.25rem;
  padding: 0.375rem 0.75rem;
  font-size: var(--fs-md);
  font-weight: 600;
  border-radius: var(--radius-md);
  white-space: nowrap;
  flex: 1;
  min-width: 80px;
}

.btn-start {
  background: var(--success);
  color: var(--on-primary);
}

.btn-start:hover:not(:disabled) {
  background: var(--success-hover);
}

.btn-stop {
  background: var(--error);
  color: var(--on-primary);
}

.btn-stop:hover:not(:disabled) {
  background: var(--error-hover);
}

/* Uses global .control-group, .control-label, .current-value, .input-group, .slider-group from main.css */

.control-group {
  margin-bottom: 0.5rem;
}

/* Without this the slider is a bare flex item and shrinks to fit, leaving the
   number input stranded well short of the panel's right edge. */
.slider-control-wrapper {
  flex: 1;
  min-width: 0;
}

.stretch-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex: 1;
}

.aggressiveness-select {
  flex: 1;
  padding: 0.25rem 0.5rem;
  font-size: var(--fs-sm);
}

.capture-header-row {
  display: flex;
  justify-content: flex-start;
  gap: 0.5rem;
  margin-bottom: 0.5rem;
  width: 100%;
}

.header-control-item {
  display: flex;
  align-items: center;
  gap: 0.25rem;
}

.comet-pro-lock {
  padding: 1rem;
  background: var(--surface-elevated);
  border-radius: var(--radius-lg);
  margin-bottom: 0.5rem;
  border: 1px solid var(--border);
}

.control-disabled {
  opacity: 0.5;
  pointer-events: none;
}

.precool-warning {
  margin: 0.25rem 0 0.5rem 0;
  padding: 0.35rem 0.5rem;
  background: var(--warning-bg);
  border-left: 3px solid var(--warning);
  color: var(--warning);
  font-size: var(--fs-xs);
  border-radius: var(--radius-xs);
}
</style>
