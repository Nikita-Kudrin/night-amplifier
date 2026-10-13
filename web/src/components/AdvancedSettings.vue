<script setup>
/**
 * Settings → Advanced: binning, the AI compute unit (`AiComputeSettings`), the simulated
 * camera and the focus image. Mirrors its keys and emits `apply(key, value)`.
 *
 * Binning is per camera, so it reads and saves the selected camera's like the cooler does
 * (`useCameraHardwareSettings`): posted without `camera_role`, a click with the guide
 * selected rebinned the imaging camera and reset its stack.
 *
 * `save` and `guard` are only handed on: the AI section must await its save, so it takes
 * the panel's save rather than an emit (see `AiComputeSettings`).
 */
import {computed, ref, watch} from 'vue'
import {useSimulatorEnabled} from '../composables/appContext.js'
import {BaseAlert, BaseToggle, BaseSlider, BaseInfoIcon, ButtonGroup} from './ui'
import AiComputeSettings from './AiComputeSettings.vue'
import SelectedCameraTag from './SelectedCameraTag.vue'
import {useSectionMirror} from '../composables/useSectionMirror.js'
import {useCameraHardwareSettings} from '../composables/useCameraHardwareSettings.js'
import {BINNING_OPTIONS, DEFAULT_SETTINGS, HELP_TEXTS, SIMULATED_PRELOAD_LIMITS} from '../constants'

const props = defineProps({
  /** The panel's settings mirror (`settingsMirror`). */
  settings: {type: Object, required: true},
  focusMode: {type: Boolean, default: false},
  aiDenoiseAvailable: {type: Boolean, default: false},
  /** `(key, value) => Promise`: persists a settings group. */
  save: {type: Function, required: true},
  /** `(operation) => Promise`: runs `operation`, reporting a failure to the panel. */
  guard: {type: Function, required: true},
})

const emit = defineEmits(['apply'])

const HELP = HELP_TEXTS

const local = useSectionMirror(() => props.settings, [
  'simulated_preload_images',
  'show_focus_image',
  'force_focus_image_now',
])

const {
  cameraSettings,
  error: binError,
  clearError: clearBinError,
  apply: applyHardware,
} = useCameraHardwareSettings()

const bin = ref(DEFAULT_SETTINGS.bin)
watch(() => cameraSettings.value?.bin, (value) => (bin.value = value ?? DEFAULT_SETTINGS.bin), {immediate: true})

function applyBin(value) {
  bin.value = value
  applyHardware({bin: value})
}

/** The app's own switch (it decides which cameras the list offers), mirrored to the server. */
const simulatorEnabledRef = useSimulatorEnabled()
const simulatorEnabled = computed({
  get: () => simulatorEnabledRef?.value ?? false,
  set: (val) => {
    if (simulatorEnabledRef) simulatorEnabledRef.value = val
  },
})
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Advanced</h3>

    <div class="control-group">
      <BaseAlert v-if="binError" @dismiss="clearBinError">{{ binError }}</BaseAlert>
      <div class="control-row">
        <label class="type-label-inline">
          Binning
          <BaseInfoIcon :message="HELP.bin"/>
          <SelectedCameraTag/>
        </label>
        <ButtonGroup :model-value="bin" :options="BINNING_OPTIONS" @update:model-value="applyBin"/>
      </div>
    </div>

    <AiComputeSettings
        :denoise="settings.denoise"
        :ai-denoise-available="aiDenoiseAvailable"
        :focus-mode="focusMode"
        :save="save"
        :guard="guard"
    />

    <div class="control-group">
      <BaseToggle
          v-model="simulatorEnabled"
          label="Simulated Camera"
          data-test="simulator-toggle"
          :help="HELP.simulated_camera"
          @update:model-value="emit('apply', 'use_simulated_camera', $event)"
      />
    </div>
    <div v-if="simulatorEnabled" class="control-group control-group-offset">
      <BaseSlider
          v-model="local.simulated_preload_images"
          label="Preload Count"
          :min="SIMULATED_PRELOAD_LIMITS.min"
          :max="SIMULATED_PRELOAD_LIMITS.max"
          :step="SIMULATED_PRELOAD_LIMITS.step"
          :help="HELP.simulated_preload_count"
          @change="emit('apply', 'simulated_preload_images', $event)"
      />
    </div>

    <div class="control-group">
      <div class="control-row">
        <BaseToggle
            v-model="local.show_focus_image"
            label="Show focus image"
            size="small"
            :help="HELP.show_focus_image"
            @update:model-value="emit('apply', 'show_focus_image', $event)"
        />
        <BaseToggle
            v-if="local.show_focus_image"
            v-model="local.force_focus_image_now"
            label="Now"
            size="small"
            :help="HELP.force_focus_image_now"
            @update:model-value="emit('apply', 'force_focus_image_now', $event)"
        />
      </div>
    </div>
  </div>
</template>
