<script setup>
/**
 * Sensor corrections and noise reduction.
 *
 * The two groups that decide how clean the image is before anything cosmetic
 * happens to it: corrections applied to the raw mosaic before demosaic, and the
 * spatial filters the encoders run at the resolution you view. Extracted from
 * `SettingsPanel.vue`, which had grown past the point where its own sections
 * were findable.
 *
 * Edits a local mirror rather than the props, and emits `apply(key, value)` when
 * a control is committed — a toggle immediately, a slider at the end of a drag.
 * The parent stays the single owner of the settings object and of persistence.
 */
import {reactive, watch} from 'vue'
import {BaseToggle, BaseSlider} from './ui'
import {
  DENOISE_CHROMA_STRENGTH_LIMITS,
  DENOISE_LUMA_STRENGTH_LIMITS,
  STAR_PROTECTION_LIMITS,
  HOT_PIXEL_SIGMA_LIMITS,
  HELP_TEXTS,
} from '../constants'

const props = defineProps({
  sensorCorrection: {type: Object, required: true},
  denoise: {type: Object, required: true},
  formatPercent: {type: Function, required: true},
  formatSigma: {type: Function, required: true},
})

const emit = defineEmits(['apply'])

const HELP = HELP_TEXTS

const local = reactive({
  sensor_correction: {...props.sensorCorrection},
  denoise: {...props.denoise},
})

watch(
    () => [props.sensorCorrection, props.denoise],
    ([sensorCorrection, denoise]) => {
      Object.assign(local.sensor_correction, sensorCorrection)
      Object.assign(local.denoise, denoise)
    },
    {deep: true}
)

function apply(key) {
  emit('apply', key, {...local[key]})
}
</script>

<template>
  <!-- Sensor corrections: applied to the raw mosaic, before demosaic -->
  <div class="settings-section">
    <h3 class="section-title">Sensor</h3>

    <div class="control-group" style="margin-top: 0.5rem">
      <BaseToggle
          v-model="local.sensor_correction.hot_pixel_rejection"
          label="Hot Pixel Rejection"
          :help="HELP.hot_pixel_rejection"
          @update:model-value="apply('sensor_correction')"
      />
    </div>

    <div
        v-if="local.sensor_correction.hot_pixel_rejection"
        class="control-group"
        style="margin-bottom: 1.5rem"
    >
      <BaseSlider
          v-model="local.sensor_correction.hot_pixel_sigma"
          label="Detection threshold"
          large-gap
          :min="HOT_PIXEL_SIGMA_LIMITS.min"
          :max="HOT_PIXEL_SIGMA_LIMITS.max"
          :step="HOT_PIXEL_SIGMA_LIMITS.step"
          :format-value="formatSigma"
          :help="HELP.hot_pixel_sigma"
          @change="apply('sensor_correction')"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.sensor_correction.fpn_removal"
          label="Row/Column Pattern Removal"
          :help="HELP.fpn_removal"
          @update:model-value="apply('sensor_correction')"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.sensor_correction.superpixel_debayer"
          label="Superpixel Debayer"
          :help="HELP.superpixel_debayer"
          @update:model-value="apply('sensor_correction')"
      />
    </div>
  </div>

  <!-- Noise reduction: runs on the streamed image, at the size you view it -->
  <div class="settings-section">
    <h3 class="section-title">Noise Reduction</h3>

    <div class="control-group" style="margin-top: 0.5rem">
      <BaseToggle
          v-model="local.denoise.chroma"
          label="Colour Mottle"
          :help="HELP.denoise_chroma"
          @update:model-value="apply('denoise')"
      />
    </div>

    <div v-if="local.denoise.chroma" class="control-group" style="margin-bottom: 1.5rem">
      <BaseSlider
          v-model="local.denoise.chroma_strength"
          label="Colour strength"
          large-gap
          :min="DENOISE_CHROMA_STRENGTH_LIMITS.min"
          :max="DENOISE_CHROMA_STRENGTH_LIMITS.max"
          :step="DENOISE_CHROMA_STRENGTH_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_chroma_strength"
          @change="apply('denoise')"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.denoise.luma"
          label="Background Grain"
          :help="HELP.denoise_luma"
          @update:model-value="apply('denoise')"
      />
    </div>

    <div v-if="local.denoise.luma" class="control-group" style="margin-bottom: 1.5rem">
      <BaseSlider
          v-model="local.denoise.luma_strength"
          label="Structure strength"
          large-gap
          :min="DENOISE_LUMA_STRENGTH_LIMITS.min"
          :max="DENOISE_LUMA_STRENGTH_LIMITS.max"
          :step="DENOISE_LUMA_STRENGTH_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_luma_strength"
          @change="apply('denoise')"
      />
    </div>

    <div v-if="local.denoise.luma" class="control-group" style="margin-bottom: 1.5rem">
      <BaseSlider
          v-model="local.denoise.star_protection"
          label="Star protection"
          large-gap
          :min="STAR_PROTECTION_LIMITS.min"
          :max="STAR_PROTECTION_LIMITS.max"
          :step="STAR_PROTECTION_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_star_protection"
          @change="apply('denoise')"
      />
    </div>
  </div>
</template>
