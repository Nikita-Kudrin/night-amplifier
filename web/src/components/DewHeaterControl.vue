<script setup>
import {computed, inject, ref, watch} from 'vue'
import {updateSettings} from '../composables/api.js'
import {useDebouncedCall} from '../composables/useDebouncedCall.js'
import {useError} from '../composables/useError.js'
import {useSelectedCamera} from '../composables/appContext.js'
import {formatPercent} from '../utils/format.js'
import {BaseAlert, BaseSlider, BaseToggle} from './ui'
import SelectedCameraTag from './SelectedCameraTag.vue'
import {HELP_TEXTS} from '../constants'

const settings = inject('settings', ref(null))
const refreshSettings = inject('refreshSettings', async () => {
})
const cameraStatus = inject('cameraStatus', ref({}))
// Same reason as the cooler: each camera carries its own dew-heater state.
const settingsRole = inject('selectedCameraRole', computed(() => 'main'))

const {error, clearError, withErrorHandling} = useError()

const cameraSettings = computed(() => {
  if (!settings.value) return null
  return settingsRole.value === 'guide' ? settings.value.guide_camera : settings.value
})

const currentCamera = useSelectedCamera()

const hasDewHeater = computed(() => Boolean(currentCamera.value?.info?.has_dew_heater))

const localHeaterEnabled = ref(true)
const localHeaterPower = ref(10)

watch(
    cameraSettings,
    (hardware) => {
      if (!hardware) return
      localHeaterEnabled.value = hardware.dew_heater_enabled ?? true
      if (typeof hardware.dew_heater_power === 'number') {
        localHeaterPower.value = hardware.dew_heater_power
      }
    },
    {immediate: true}
)

const liveStatus = computed(() => {
  const name = currentCamera.value?.name
  return name ? cameraStatus.value?.[name] : null
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

function handleHeaterToggle(enabled) {
  localHeaterEnabled.value = enabled
  applySetting({dew_heater_enabled: enabled})
}

function handlePowerChange(value) {
  localHeaterPower.value = value
  debouncedApply({dew_heater_power: value}, settingsRole.value)
}

/** The power control runs 0–100. */
function formatPower(v) {
  return formatPercent(v / 100)
}
</script>

<template>
  <div v-if="hasDewHeater" class="settings-section dew-heater-control">
    <h3 class="section-title">Dew Heater <SelectedCameraTag/></h3>

    <BaseAlert v-if="error" @dismiss="clearError">{{ error }}</BaseAlert>

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
        :format-value="formatPower"
        :help="HELP_TEXTS.dew_heater_power"
        @change="handlePowerChange"
    >
      <template #label-left>
        <span class="pill status-pill" :class="liveStatus?.dew_heater_on ? 'pill-success' : 'pill-muted'">
          {{ liveStatus?.dew_heater_on ? 'On' : 'Off' }}
        </span>
      </template>
    </BaseSlider>
  </div>
</template>

<style scoped>
.status-pill {
  font-size: var(--fs-xs);
  padding: 0.1rem 0.5rem;
}
</style>
