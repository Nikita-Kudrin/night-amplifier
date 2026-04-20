<script setup>
import {ref, inject, computed, watch, onMounted} from 'vue'
import {startCapture, stopCapture, updateSettings, getStackingTypes} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {BaseAlert, BaseToggle, BaseInfoIcon, ButtonGroup, BaseProLock} from './ui'
import {
  EXPOSURE_PRESETS,
  GAIN_LIMITS,
  CAPTURE_STATES,
  DEFAULT_SETTINGS,
  STRETCH_AGGRESSIVENESS_OPTIONS,
  HELP_TEXTS
} from '../constants'

const settings = inject('settings')
const selectedCamera = inject('selectedCamera')
const eventStream = inject('eventStream')
const refreshSettings = inject('refreshSettings')
const cameras = inject('cameras', {value: []})
const cameraPhase = inject('cameraPhase', {value: {}})
const capabilities = inject('capabilities', {
  has_pro: false,
  deep_sky: {advanced_rejection: false, rbf_background: false},
  planetary: {advanced_stacking: false},
  push_to: {astap_solver: false},
  comet: {pro_stacking: false},
})

const {error, loading, clearError, withErrorHandling} = useError()

const exposure = ref(100)
const exposureUnit = ref('ms')
const gain = ref(GAIN_LIMITS.default)

const stackingTypes = ref([])
const selectedStackingType = ref('deep_sky')
const stackingEnabled = ref(DEFAULT_SETTINGS.stacking)
const autoStretch = ref(DEFAULT_SETTINGS.auto_stretch)
const stretchAggressiveness = ref(DEFAULT_SETTINGS.stretch_aggressiveness)
const wandererMode = ref(DEFAULT_SETTINGS.wanderer_mode)

const stackingMode = computed(() => {
  if (wandererMode.value) return 'wanderer'
  if (stackingEnabled.value) return 'stacking'
  return 'off'
})

// Sync from settings
watch(
    settings,
    (newSettings) => {
      if (newSettings) {
        const us = newSettings.exposure_us
        if (us >= 1000000) {
          exposure.value = us / 1000000
          exposureUnit.value = 's'
        } else if (us >= 1000) {
          exposure.value = us / 1000
          exposureUnit.value = 'ms'
        } else {
          exposure.value = us
          exposureUnit.value = 'us'
        }
        gain.value = newSettings.gain
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
      }
    },
    {immediate: true}
)

onMounted(async () => {
  try {
    stackingTypes.value = await getStackingTypes()
  } catch (e) {
    console.error('Failed to load stacking types:', e)
  }
})

const isCapturing = computed(
    () =>
        eventStream.captureState.value === CAPTURE_STATES.CAPTURING ||
        eventStream.captureState.value === CAPTURE_STATES.STARTING
)

const isStopping = computed(() => eventStream.captureState.value === CAPTURE_STATES.STOPPING)

const canStart = computed(() => selectedCamera.value && !isCapturing.value && !isStopping.value)

const selectedCameraName = computed(() => {
  const id = selectedCamera.value
  if (!id) return null
  const cam = (cameras?.value || []).find((c) => c.id === id)
  return cam?.name || null
})

const selectedCameraPhase = computed(() => {
  const name = selectedCameraName.value
  return name ? cameraPhase.value?.[name] || null : null
})

const showPrecoolWarning = computed(
    () => selectedCameraPhase.value === 'precooling' && !isCapturing.value
)

const exposureUs = computed(() => {
  const val = exposure.value
  switch (exposureUnit.value) {
    case 's':
      return val * 1000000
    case 'ms':
      return val * 1000
    default:
      return val
  }
})


const exposurePresets = computed(() => EXPOSURE_PRESETS[exposureUnit.value] || EXPOSURE_PRESETS.ms)

async function handleStart() {
  await withErrorHandling(async () => {
    await updateSettings({exposure_us: exposureUs.value, gain: gain.value})
    await startCapture(selectedCamera.value)
  })
}

async function handleStop() {
  await withErrorHandling(async () => {
    await stopCapture()
  })
}

async function applySetting(settings) {
  await withErrorHandling(async () => {
    await updateSettings(settings)
    await refreshSettings()
  })
}

const applyExposure = () => applySetting({exposure_us: exposureUs.value})
const applyGain = () => applySetting({gain: gain.value})
const setExposurePreset = (us) => applySetting({exposure_us: us})
const applyStackingType = () => applySetting({stacking_type: selectedStackingType.value})
const applyAutoStretch = () => applySetting({auto_stretch: autoStretch.value})
const applyStretchAggressiveness = () => applySetting({stretch_aggressiveness: stretchAggressiveness.value})

function applyStackingMode(val) {
  const modes = {
    wanderer: {stacking: true, wanderer_mode: true},
    stacking: {stacking: true, wanderer_mode: false},
    off: {stacking: false, wanderer_mode: false},
  }
  applySetting(modes[val] || modes.off)
}

const showCometLock = computed(() => {
  return selectedStackingType.value === 'comet' && !capabilities.comet?.pro_stacking
})

const HELP = HELP_TEXTS
</script>

<template>
  <div class="panel panel-bordered">
    <div class="panel-header" style="justify-content: flex-start; gap: 0.5rem; margin-bottom: 0.5rem;">
      <div class="header-control-item">
        <label class="type-label-inline">Capture mode</label>
        <ButtonGroup
            :model-value="stackingMode"
            :options="[
              {value: 'off', label: 'Live view'},
              {value: 'wanderer', label: 'Wanderer'},
              {value: 'stacking', label: 'Stacking'}
            ]"
            @update:model-value="applyStackingMode"
        />
        <BaseInfoIcon :message="HELP.stacking"/>
      </div>
    </div>

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
            :disabled="isCapturing || isStopping"
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
            message="Comet centroid tracking and aggressive star rejection are available in Night Amplifier Pro."
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
          <span>{{ loading ? 'Starting...' : 'Start' }}</span>
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
          <span>{{ isStopping ? 'Stopping...' : 'Stop' }}</span>
        </button>
      </template>
    </div>

    <div v-if="showPrecoolWarning" class="precool-warning">
      Sensor is still cooling — capture will start with elevated dark current.
    </div>

    <!-- Exposure control -->
    <div class="control-group" :class="{ 'control-disabled': showCometLock }">
      <div class="control-row">
        <label class="type-label-inline">
          Exposure
          <BaseInfoIcon :message="HELP.exposure"/>
        </label>
        <div class="input-group">
          <input
              v-model.number="exposure"
              type="number"
              min="0.001"
              step="0.1"
              class="input"
              :disabled="showCometLock"
              @change="applyExposure"
          />
          <select v-model="exposureUnit" class="select" :disabled="showCometLock" @change="applyExposure">
            <option value="us">us</option>
            <option value="ms">ms</option>
            <option value="s">s</option>
          </select>
        </div>
      </div>
      <div class="presets">
        <button
            v-for="preset in exposurePresets"
            :key="preset.us"
            class="btn btn-preset"
            :class="{ active: settings?.exposure_us === preset.us }"
            :disabled="showCometLock"
            @click="setExposurePreset(preset.us)"
        >
          {{ preset.label }}
        </button>
      </div>
    </div>

    <div class="control-group" :class="{ 'control-disabled': showCometLock }">
      <div class="control-row">
        <label class="type-label-inline">
          Gain
          <BaseInfoIcon :message="HELP.gain"/>
        </label>
        <div class="slider-group">
          <input
              v-model.number="gain"
              type="range"
              :min="GAIN_LIMITS.min"
              :max="GAIN_LIMITS.max"
              step="1"
              class="slider"
              :disabled="showCometLock"
              @change="applyGain"
          />
          <input
              v-model.number="gain"
              type="number"
              :min="GAIN_LIMITS.min"
              :max="GAIN_LIMITS.max"
              class="input input-sm"
              :disabled="showCometLock"
              @change="applyGain"
          />
        </div>
      </div>
    </div>

    <div class="control-group">
      <div class="control-row">
        <label class="type-label-inline" :class="{ 'text-muted': showCometLock }">
          Auto Stretch
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
  </div>
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
  font-size: 0.8rem;
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
  font-size: 0.8rem;
  font-weight: 600;
  border-radius: 6px;
  white-space: nowrap;
  flex: 1;
  min-width: 80px;
}

.btn-start {
  background: var(--success);
  color: white;
}

.btn-start:hover:not(:disabled) {
  background: var(--success-hover);
}

.btn-stop {
  background: var(--error);
  color: white;
}

.btn-stop:hover:not(:disabled) {
  background: var(--error-hover);
}

/* Uses global .control-group, .control-label, .current-value, .input-group, .slider-group from main.css */

.control-group {
  margin-bottom: 0.5rem;
}


.control-row .input-group,
.control-row .slider-group {
  flex: 1;
}

.presets {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem;
  margin-top: 0.25rem;
}

.btn-preset {
  padding: 0.125rem 0.375rem;
  font-size: 0.65rem;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
}

.btn-preset:hover {
  background: var(--surface-hover);
}

.btn-preset.active {
  background: var(--primary);
  border-color: var(--primary);
  color: white;
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
  font-size: 0.75rem;
}

.header-control-item {
  display: flex;
  align-items: center;
  gap: 0.25rem;
}

.btn-toggle {
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  min-width: 70px;
}

.btn-toggle.active {
  background: var(--primary);
  border-color: var(--primary);
  color: white;
}

.btn-toggle:hover:not(.active) {
  background: var(--surface-hover);
}

.comet-pro-lock {
  padding: 1rem;
  background: var(--surface-elevated);
  border-radius: 8px;
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
  background: rgba(234, 179, 8, 0.12);
  border-left: 3px solid #eab308;
  color: #eab308;
  font-size: 0.72rem;
  border-radius: 3px;
}
</style>
