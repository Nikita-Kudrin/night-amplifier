<script setup lang="ts">
import {computed, ref} from 'vue'
import {calibrateDaylightBalance} from '../composables/api'
import {useCameraHardwareSettings} from '../composables/useCameraHardwareSettings'
import {BaseAlert, BaseInfoIcon} from './ui'
import SelectedCameraTag from './SelectedCameraTag.vue'
import {HELP_TEXTS, SENSOR_TYPES} from '../constants'
import type {DaylightBalance} from '../composables/api.types'

// Each camera has its own colour response, so the balance is saved against the selected one.
const {settingsRole, cameraSettings, currentCamera, error, clearError, withErrorHandling, apply} =
    useCameraHardwareSettings()

const IDENTITY: DaylightBalance = {red: 1, green: 1, blue: 1}

const isColour = computed(() => currentCamera.value?.info.sensor_type === SENSOR_TYPES.COLOR)
const balance = computed(() => cameraSettings.value?.daylight_balance ?? IDENTITY)
const calibrated = computed(() =>
    balance.value.red !== 1 || balance.value.green !== 1 || balance.value.blue !== 1)
const calibrating = ref(false)

function gain(value: number) {
  return `×${value.toFixed(2)}`
}

async function calibrate() {
  calibrating.value = true
  try {
    // Bound to the camera selected when the button was pressed.
    const role = settingsRole.value
    await withErrorHandling(async () => {
      await calibrateDaylightBalance(role)
    })
  } finally {
    calibrating.value = false
  }
}

function reset() {
  void apply({daylight_balance: IDENTITY})
}
</script>

<template>
  <div v-if="isColour" class="settings-section daylight-balance-control">
    <h3 class="section-title">
      Daylight White Balance <SelectedCameraTag/>
      <BaseInfoIcon :message="HELP_TEXTS.daylight_balance"/>
    </h3>

    <BaseAlert v-if="error" @dismiss="clearError">{{ error }}</BaseAlert>

    <div class="control-group balance-row">
      <span v-if="calibrated" class="balance-gains">
        R {{ gain(balance.red) }} · G {{ gain(balance.green) }} · B {{ gain(balance.blue) }}
      </span>
      <span v-else class="balance-gains text-muted">Not calibrated</span>
      <div class="balance-actions">
        <button class="btn btn-sm calibrate-button" :disabled="calibrating" @click="calibrate">
          {{ calibrating ? 'Calibrating…' : 'Calibrate' }}
        </button>
        <button class="btn btn-sm reset-button" :disabled="!calibrated || calibrating" @click="reset">
          Reset
        </button>
      </div>
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
}

.balance-gains {
  font-size: var(--fs-sm);
  font-variant-numeric: tabular-nums;
}

.balance-actions {
  display: flex;
  gap: 0.5rem;
}
</style>
