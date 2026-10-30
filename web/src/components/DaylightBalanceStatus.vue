<script setup lang="ts">
import {computed} from 'vue'
import {useCameraHardwareSettings} from '../composables/useCameraHardwareSettings'
import {BaseInfoIcon} from './ui'
import SelectedCameraTag from './SelectedCameraTag.vue'
import {HELP_TEXTS, SENSOR_TYPES} from '../constants'

// Read-only: the balance is measured off the Moon by the server, per camera, with no button.
const {cameraSettings, currentCamera} = useCameraHardwareSettings()

const isColour = computed(() => currentCamera.value?.info.sensor_type === SENSOR_TYPES.COLOR)
const measured = computed(() => cameraSettings.value?.daylight_balance_source === 'moon')
const gains = computed(() => {
  const balance = cameraSettings.value?.daylight_balance
  if (!balance) return ''
  const gain = (value: number) => `×${value.toFixed(2)}`
  return `R ${gain(balance.red)} · G ${gain(balance.green)} · B ${gain(balance.blue)}`
})
</script>

<template>
  <div v-if="isColour" class="settings-section daylight-balance-status">
    <h3 class="section-title">
      Daylight White Balance <SelectedCameraTag/>
      <BaseInfoIcon :message="HELP_TEXTS.daylight_balance"/>
    </h3>
    <div class="control-group balance-row">
      <template v-if="measured">
        <span class="balance-source">Read off the Moon</span>
        <span class="balance-gains">{{ gains }}</span>
      </template>
      <span v-else class="balance-source text-muted">Typical balance until Planetary shows the Moon</span>
    </div>
  </div>
</template>

<style scoped>
.balance-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  flex-wrap: wrap;
  font-size: var(--fs-sm);
}

.balance-gains {
  font-variant-numeric: tabular-nums;
}
</style>
