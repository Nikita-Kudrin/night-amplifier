<script setup>
import {computed, ref, watch} from 'vue'
import {useCameraHardwareSettings} from '../composables/useCameraHardwareSettings.js'
import {formatPercent, formatTemperature, MISSING} from '../utils/format.js'
import {BaseAlert, BaseSlider, BaseToggle} from './ui'
import SelectedCameraTag from './SelectedCameraTag.vue'
import {COOLER_STABLE_BAND_C, COOLER_TEMP_LIMITS, HELP_TEXTS} from '../constants'

// The guide camera keeps its own cooler settings; the selection says which set these
// controls are showing and editing.
const {cameraSettings, currentCamera, liveStatus, error, clearError, apply} = useCameraHardwareSettings()

const hasCooler = computed(() => Boolean(currentCamera.value?.info?.has_cooler))

const minTemp = computed(() => currentCamera.value?.info?.min_temp_c ?? COOLER_TEMP_LIMITS.min)
const maxTemp = computed(() => currentCamera.value?.info?.max_temp_c ?? COOLER_TEMP_LIMITS.max)

const localCoolerEnabled = ref(false)
const localTargetTemp = ref(COOLER_TEMP_LIMITS.default)
const localCoolerFastMode = ref(false)

function hasSetpoint(hardware) {
  return typeof hardware?.target_temp_c === 'number'
}

watch(
    cameraSettings,
    (hardware) => {
      if (!hardware) return
      localCoolerEnabled.value = hardware.cooler_enabled ?? false
      // Every camera's own, or the default: keeping the last number showed the previous
      // camera's setpoint for a guide that has none (the server's default is null).
      localTargetTemp.value = hasSetpoint(hardware) ? hardware.target_temp_c : COOLER_TEMP_LIMITS.default
      localCoolerFastMode.value = hardware.cooler_fast_mode ?? false
    },
    {immediate: true}
)

const statusBadge = computed(() => {
  if (!liveStatus.value) return {label: 'Idle', tone: 'muted'}
  if (!liveStatus.value.cooler_on) return {label: 'Off', tone: 'muted'}
  if (typeof liveStatus.value.target_temp_c !== 'number') {
    return {label: 'Cooling', tone: 'warning'}
  }
  const delta = Math.abs(liveStatus.value.temperature_c - liveStatus.value.target_temp_c)
  if (delta <= COOLER_STABLE_BAND_C) return {label: 'Stable', tone: 'success'}
  if (liveStatus.value.temperature_c > liveStatus.value.target_temp_c) {
    return {label: 'Cooling', tone: 'warning'}
  }
  return {label: 'Warming', tone: 'warning'}
})

/** Turning Cool on with no stored setpoint sends the one shown, which was never saved. */
function handleCoolerToggle(enabled) {
  localCoolerEnabled.value = enabled
  const setpoint = enabled && !hasSetpoint(cameraSettings.value) ? {target_temp_c: localTargetTemp.value} : {}
  apply({cooler_enabled: enabled, ...setpoint})
}

function handleTargetChange(value) {
  localTargetTemp.value = value
  apply({target_temp_c: value})
}

function handleFastModeToggle(enabled) {
  localCoolerFastMode.value = enabled
  apply({cooler_fast_mode: enabled})
}

/** The camera reports cooler power as 0–100. */
function formatPower(v) {
  return v == null ? MISSING : formatPercent(v / 100)
}
</script>

<template>
  <div v-if="hasCooler" class="settings-section cooler-control">
    <h3 class="section-title">Camera cooler <SelectedCameraTag/></h3>

    <BaseAlert v-if="error" @dismiss="clearError">{{ error }}</BaseAlert>

    <div class="toggles-row">
      <BaseToggle
          v-model="localCoolerEnabled"
          label="Cool"
          :help="HELP_TEXTS.cooler_enabled"
          @update:model-value="handleCoolerToggle"
      />

      <BaseToggle
          v-model="localCoolerFastMode"
          label="Fast"
          size="small"
          :help="HELP_TEXTS.cooler_fast_mode"
          @update:model-value="handleFastModeToggle"
      >
        <template #label-extra>
          <svg
              v-if="localCoolerFastMode"
              class="fast-warning-icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-label="Warning"
          >
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"/>
            <line x1="12" y1="9" x2="12" y2="13"/>
            <line x1="12" y1="17" x2="12.01" y2="17"/>
          </svg>
        </template>
      </BaseToggle>
    </div>

    <BaseSlider
        v-if="localCoolerEnabled"
        v-model="localTargetTemp"
        label="Target Temperature"
        :min="minTemp"
        :max="maxTemp"
        :step="1"
        :format-value="formatTemperature"
        :help="HELP_TEXTS.target_temp_c"
        @change="handleTargetChange"
    />

    <div class="cooler-status">
      <div class="status-item">
        <span class="status-label">Sensor</span>
        <span class="status-value">
          {{ formatTemperature(liveStatus?.temperature_c) }}
        </span>
      </div>
      <div class="status-item">
        <span class="status-label">Power</span>
        <span class="status-value">{{ formatPower(liveStatus?.cooler_power) }}</span>
      </div>
      <div class="status-item">
        <span class="status-label">State</span>
        <span class="pill pill-status" :class="`pill-${statusBadge.tone}`">
          {{ statusBadge.label }}
        </span>
      </div>
    </div>

    <p v-if="localCoolerEnabled" class="cooler-hint">
      Cooler activates while capturing — start a capture session to engage the TEC.
    </p>
  </div>
</template>

<style scoped>
.toggles-row {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 0.75rem;
}

.cooler-status {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.375rem 0.5rem;
  background: var(--surface-elevated);
  border-radius: var(--radius-md);
  margin-top: 0.5rem;
}

.status-item {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  font-size: var(--fs-sm);
}

.status-label {
  color: var(--text-secondary);
}

.status-value {
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}


.cooler-hint {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin: 0.5rem 0 0;
}

.fast-warning-icon {
  width: 12px;
  height: 12px;
  color: var(--warning);
  margin-left: 0.25rem;
  vertical-align: middle;
}
</style>
