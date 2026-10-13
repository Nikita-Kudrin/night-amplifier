<script setup>
import {computed, ref, watch} from 'vue'
import {useCameraHardwareSettings} from '../composables/useCameraHardwareSettings.js'
import {formatPercent} from '../utils/format.js'
import {BaseAlert, BaseSlider, BaseToggle} from './ui'
import SelectedCameraTag from './SelectedCameraTag.vue'
import {DEFAULT_SETTINGS, DEW_HEATER_POWER_LIMITS, HELP_TEXTS} from '../constants'

// Same reason as the cooler: each camera carries its own dew-heater state.
const {cameraSettings, currentCamera, liveStatus, error, clearError, apply, applyDebounced} =
    useCameraHardwareSettings()

const hasDewHeater = computed(() => Boolean(currentCamera.value?.info?.has_dew_heater))

const localHeaterEnabled = ref(DEFAULT_SETTINGS.dew_heater_enabled)
const localHeaterPower = ref(DEFAULT_SETTINGS.dew_heater_power)

watch(
    cameraSettings,
    (hardware) => {
      if (!hardware) return
      localHeaterEnabled.value = hardware.dew_heater_enabled ?? DEFAULT_SETTINGS.dew_heater_enabled
      if (typeof hardware.dew_heater_power === 'number') {
        localHeaterPower.value = hardware.dew_heater_power
      }
    },
    {immediate: true}
)

function handleHeaterToggle(enabled) {
  localHeaterEnabled.value = enabled
  apply({dew_heater_enabled: enabled})
}

function handlePowerChange(value) {
  localHeaterPower.value = value
  applyDebounced({dew_heater_power: value})
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
        :min="DEW_HEATER_POWER_LIMITS.min"
        :max="DEW_HEATER_POWER_LIMITS.max"
        :step="DEW_HEATER_POWER_LIMITS.step"
        :format-value="formatPower"
        :help="HELP_TEXTS.dew_heater_power"
        @change="handlePowerChange"
    >
      <template #label-left>
        <span class="pill pill-status" :class="liveStatus?.dew_heater_on ? 'pill-success' : 'pill-muted'">
          {{ liveStatus?.dew_heater_on ? 'On' : 'Off' }}
        </span>
      </template>
    </BaseSlider>
  </div>
</template>

