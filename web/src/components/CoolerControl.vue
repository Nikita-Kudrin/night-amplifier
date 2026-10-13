<script setup>
import {computed, inject, ref, watch} from 'vue'
import {updateSettings} from '../composables/api.js'
import {useDebouncedCall} from '../composables/useDebouncedCall.js'
import {useError} from '../composables/useError.js'
import {useSelectedCamera} from '../composables/appContext.js'
import {formatPercent, formatTemperature, MISSING} from '../utils/format.js'
import {BaseAlert, BaseSlider, BaseToggle} from './ui'
import SelectedCameraTag from './SelectedCameraTag.vue'
import {COOLER_TEMP_LIMITS, HELP_TEXTS} from '../constants'

const settings = inject('settings', ref(null))
const refreshSettings = inject('refreshSettings', async () => {
})
const cameraStatus = inject('cameraStatus', ref({}))
// The guide camera keeps its own cooler settings; the selection says which set these
// controls are showing and editing.
const settingsRole = inject('selectedCameraRole', computed(() => 'main'))

const {error, clearError, withErrorHandling} = useError()

const cameraSettings = computed(() => {
  if (!settings.value) return null
  return settingsRole.value === 'guide' ? settings.value.guide_camera : settings.value
})

const currentCamera = useSelectedCamera()

const hasCooler = computed(() => Boolean(currentCamera.value?.info?.has_cooler))

const minTemp = computed(() => currentCamera.value?.info?.min_temp_c ?? COOLER_TEMP_LIMITS.min)
const maxTemp = computed(() => currentCamera.value?.info?.max_temp_c ?? COOLER_TEMP_LIMITS.max)

const localCoolerEnabled = ref(false)
const localTargetTemp = ref(COOLER_TEMP_LIMITS.default)
const localCoolerFastMode = ref(false)

watch(
    cameraSettings,
    (hardware) => {
      if (!hardware) return
      localCoolerEnabled.value = hardware.cooler_enabled ?? false
      if (typeof hardware.target_temp_c === 'number') {
        localTargetTemp.value = hardware.target_temp_c
      }
      localCoolerFastMode.value = hardware.cooler_fast_mode ?? false
    },
    {immediate: true}
)

const liveStatus = computed(() => {
  const name = currentCamera.value?.name
  return name ? cameraStatus.value?.[name] : null
})

const statusBadge = computed(() => {
  if (!liveStatus.value) return {label: 'Idle', tone: 'muted'}
  if (!liveStatus.value.cooler_on) return {label: 'Off', tone: 'muted'}
  if (typeof liveStatus.value.target_temp_c !== 'number') {
    return {label: 'Cooling', tone: 'warning'}
  }
  const delta = Math.abs(liveStatus.value.temperature_c - liveStatus.value.target_temp_c)
  if (delta <= 0.5) return {label: 'Stable', tone: 'success'}
  if (liveStatus.value.temperature_c > liveStatus.value.target_temp_c) {
    return {label: 'Cooling', tone: 'warning'}
  }
  return {label: 'Warming', tone: 'warning'}
})

/** `role` is the camera selected when the edit was made, not when a debounce fires. */
async function applySetting(payload, role = settingsRole.value) {
  await withErrorHandling(async () => {
    await updateSettings({camera_role: role, ...payload})
    await refreshSettings()
  })
}

const {schedule: debouncedApply, flush: flushPending} = useDebouncedCall(applySetting, 300)
// A pending edit is the outgoing camera's: sent now, it is never coalesced with the next one's.
watch(settingsRole, flushPending)

function handleCoolerToggle(enabled) {
  localCoolerEnabled.value = enabled
  applySetting({cooler_enabled: enabled})
}

function handleTargetChange(value) {
  localTargetTemp.value = value
  debouncedApply({target_temp_c: value}, settingsRole.value)
}

function handleFastModeToggle(enabled) {
  localCoolerFastMode.value = enabled
  applySetting({cooler_fast_mode: enabled})
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
        <span class="pill status-pill" :class="`pill-${statusBadge.tone}`">
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

.status-pill {
  font-size: var(--fs-xs);
  padding: 0.1rem 0.5rem;
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
