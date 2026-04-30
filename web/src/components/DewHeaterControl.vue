<script setup>
import {computed, inject, ref, watch} from 'vue'
import {updateSettings} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {BaseSlider, BaseToggle} from './ui'
import {HELP_TEXTS} from '../constants'

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

const hasDewHeater = computed(() => Boolean(currentCamera.value?.info?.has_dew_heater))

const localHeaterEnabled = ref(true)
const localHeaterPower = ref(10)

watch(
    settings,
    (newSettings) => {
      if (!newSettings) return
      localHeaterEnabled.value = newSettings.dew_heater_enabled ?? true
      if (typeof newSettings.dew_heater_power === 'number') {
        localHeaterPower.value = newSettings.dew_heater_power
      }
    },
    {immediate: true}
)

const liveStatus = computed(() => {
  const name = currentCamera.value?.name
  return name ? cameraStatus.value?.[name] : null
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

function handleHeaterToggle(enabled) {
  localHeaterEnabled.value = enabled
  applySetting({dew_heater_enabled: enabled})
}

function handlePowerChange(value) {
  localHeaterPower.value = value
  debouncedApply({dew_heater_power: value})
}

function formatPercent(v) {
  return `${Math.round(v)}%`
}
</script>

<template>
  <div v-if="hasDewHeater" class="settings-section dew-heater-control">
    <h3 class="section-title">Dew Heater</h3>

    <div v-if="error" class="heater-error">{{ error }}</div>

    <div class="control-group">
      <BaseToggle
          v-model="localHeaterEnabled"
          label="Heater Enabled"
          :help="HELP_TEXTS.dew_heater_enabled"
          @update:model-value="handleHeaterToggle"
      />
    </div>

    <BaseSlider
        v-if="localHeaterEnabled"
        v-model="localHeaterPower"
        label="Heater Power"
        :min="0"
        :max="100"
        :step="1"
        :format-value="formatPercent"
        :help="HELP_TEXTS.dew_heater_power"
        @change="handlePowerChange"
    >
      <template #label-left>
        <span class="status-pill" :class="liveStatus?.dew_heater_on ? 'tone-good' : 'tone-idle'">
          {{ liveStatus?.dew_heater_on ? 'On' : 'Off' }}
        </span>
      </template>
    </BaseSlider>
  </div>
</template>

<style scoped>
.dew-heater-control {
  margin-bottom: 0.625rem;
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

.tone-good {
  background: rgba(16, 185, 129, 0.15);
  color: #10b981;
}

.heater-error {
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
  border-radius: 4px;
  padding: 0.375rem 0.5rem;
  font-size: 0.75rem;
  margin-bottom: 0.5rem;
}
</style>
