<script setup>
import {computed, inject, ref, watch} from 'vue'
import {updateSettings} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {BaseSlider, BaseToggle} from './ui'
import {COOLER_TEMP_LIMITS, HELP_TEXTS} from '../constants'

const settings = inject('settings', ref(null))
const refreshSettings = inject('refreshSettings', async () => {
})
const cameras = inject('cameras', ref([]))
const selectedCameraId = inject('selectedCamera', ref(null))
const cameraStatus = inject('cameraStatus', ref({}))

const {error, withErrorHandling} = useError()

const currentCamera = computed(() =>
    (cameras.value ?? []).find((c) => c.id === selectedCameraId.value)
)

const hasCooler = computed(() => Boolean(currentCamera.value?.info?.has_cooler))

const minTemp = computed(() => currentCamera.value?.info?.min_temp_c ?? COOLER_TEMP_LIMITS.min)
const maxTemp = computed(() => currentCamera.value?.info?.max_temp_c ?? COOLER_TEMP_LIMITS.max)

const localCoolerEnabled = ref(false)
const localTargetTemp = ref(COOLER_TEMP_LIMITS.default)
const localCoolerFastMode = ref(false)

watch(
    settings,
    (newSettings) => {
      if (!newSettings) return
      localCoolerEnabled.value = newSettings.cooler_enabled ?? false
      if (typeof newSettings.target_temp_c === 'number') {
        localTargetTemp.value = newSettings.target_temp_c
      }
      localCoolerFastMode.value = newSettings.cooler_fast_mode ?? false
    },
    {immediate: true}
)

const liveStatus = computed(() => {
  const name = currentCamera.value?.name
  return name ? cameraStatus.value?.[name] : null
})

const statusBadge = computed(() => {
  if (!liveStatus.value) return {label: 'Idle', tone: 'idle'}
  if (!liveStatus.value.cooler_on) return {label: 'Off', tone: 'idle'}
  if (typeof liveStatus.value.target_temp_c !== 'number') {
    return {label: 'Cooling', tone: 'busy'}
  }
  const delta = Math.abs(liveStatus.value.temperature_c - liveStatus.value.target_temp_c)
  if (delta <= 0.5) return {label: 'Stable', tone: 'good'}
  if (liveStatus.value.temperature_c > liveStatus.value.target_temp_c) {
    return {label: 'Cooling', tone: 'busy'}
  }
  return {label: 'Warming', tone: 'busy'}
})

let debounceTimer = null

async function applySetting(payload) {
  await withErrorHandling(async () => {
    await updateSettings(payload)
    await refreshSettings()
  })
}

function debouncedApply(payload) {
  clearTimeout(debounceTimer)
  debounceTimer = setTimeout(() => applySetting(payload), 300)
}

function handleCoolerToggle(enabled) {
  localCoolerEnabled.value = enabled
  applySetting({cooler_enabled: enabled})
}

function handleTargetChange(value) {
  localTargetTemp.value = value
  debouncedApply({target_temp_c: value})
}

function handleFastModeToggle(enabled) {
  localCoolerFastMode.value = enabled
  applySetting({cooler_fast_mode: enabled})
}

function formatTemp(v) {
  return `${Number(v).toFixed(1)}°C`
}

function formatPower(v) {
  return v == null ? '—' : `${Math.round(v)}%`
}
</script>

<template>
  <div v-if="hasCooler" class="settings-section cooler-control">
    <h3 class="section-title">Camera cooler</h3>

    <div v-if="error" class="cooler-error">{{ error }}</div>

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
        :format-value="formatTemp"
        :help="HELP_TEXTS.target_temp_c"
        @change="handleTargetChange"
    />



    <div class="cooler-status">
      <div class="status-item">
        <span class="status-label">Sensor</span>
        <span class="status-value">
          {{ liveStatus ? formatTemp(liveStatus.temperature_c) : '—' }}
        </span>
      </div>
      <div class="status-item">
        <span class="status-label">Power</span>
        <span class="status-value">{{ formatPower(liveStatus?.cooler_power) }}</span>
      </div>
      <div class="status-item">
        <span class="status-label">State</span>
        <span class="status-pill" :class="`tone-${statusBadge.tone}`">
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
.cooler-control {
  margin-bottom: 0.625rem;
}

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
  border-radius: 6px;
  margin-top: 0.5rem;
}

.status-item {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.75rem;
}

.status-label {
  color: var(--text-secondary);
}

.status-value {
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

.status-pill {
  font-size: 0.7rem;
  padding: 0.1rem 0.5rem;
  border-radius: 999px;
  font-weight: 500;
}

.tone-idle {
  background: var(--surface-hover);
  color: var(--text-muted);
}

.tone-busy {
  background: rgba(245, 158, 11, 0.15);
  color: #f59e0b;
}

.tone-good {
  background: rgba(16, 185, 129, 0.15);
  color: #10b981;
}

.cooler-hint {
  font-size: 0.7rem;
  color: var(--text-muted);
  margin: 0.5rem 0 0;
}

.cooler-error {
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
  border-radius: 4px;
  padding: 0.375rem 0.5rem;
  font-size: 0.75rem;
  margin-bottom: 0.5rem;
}



.fast-warning-icon {
  width: 12px;
  height: 12px;
  color: var(--warning);
  margin-left: 0.25rem;
  vertical-align: middle;
}
</style>
