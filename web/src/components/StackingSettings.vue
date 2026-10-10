<script setup>
/**
 * Settings → Stacking, plus Planetary while that mode is selected: frame weighting,
 * rejection and the planetary alignment switches. Shown only while stacking is on.
 *
 * Mirrors its values and emits `apply(key, value)` per setting, as `ImageQualitySettings`
 * does. The sigma slider is sent 300 ms after the last drag.
 */
import {reactive, watch} from 'vue'
import {BaseToggle, BaseSlider, BaseInfoIcon, BaseProLock} from './ui'
import {WEIGHTING_PRESET_OPTIONS, REJECTION_METHOD_OPTIONS, HELP_TEXTS} from '../constants'

const props = defineProps({
  weightingPreset: {type: String, required: true},
  rejectionMethod: {type: String, required: true},
  rejectionSigma: {type: Number, required: true},
  planetaryAutoTracking: {type: Boolean, required: true},
  planetaryMultiPointAlignment: {type: Boolean, required: true},
  /** Whether the stacking type is planetary, which adds that mode's section. */
  planetary: {type: Boolean, default: false},
  advancedRejectionAvailable: {type: Boolean, default: false},
  advancedPlanetaryAvailable: {type: Boolean, default: false},
})

const emit = defineEmits(['apply'])

const HELP = HELP_TEXTS

const local = reactive({})

watch(
    () => [
      props.weightingPreset,
      props.rejectionMethod,
      props.rejectionSigma,
      props.planetaryAutoTracking,
      props.planetaryMultiPointAlignment,
    ],
    ([weighting, rejection, sigma, autoTracking, multiPoint]) => {
      local.weighting_preset = weighting
      local.rejection_method = rejection
      local.rejection_sigma = sigma
      local.planetary_auto_tracking = autoTracking
      local.planetary_multi_point_alignment = multiPoint
    },
    {immediate: true}
)

function apply(key, value) {
  emit('apply', key, value)
}

let debounceTimer = null

function applyDebounced(key, value) {
  clearTimeout(debounceTimer)
  debounceTimer = setTimeout(() => apply(key, value), 300)
}
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Stacking</h3>

    <div class="control-group">
      <div class="control-row">
        <label class="control-label" style="margin-bottom: 0; flex: 1">
          Frame Weighting
          <BaseInfoIcon :message="HELP.weighting_preset"/>
        </label>
        <select
            id="weighting-preset-select"
            v-model="local.weighting_preset"
            class="select"
            style="width: 120px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
            @change="apply('weighting_preset', $event.target.value)"
        >
          <option v-for="opt in WEIGHTING_PRESET_OPTIONS" :key="opt.value" :value="opt.value">
            {{ opt.label }}
          </option>
        </select>
      </div>
    </div>

    <div class="control-group">
      <div class="control-row">
        <label class="control-label" style="margin-bottom: 0; flex: 1">
          Rejection Method
          <BaseProLock v-if="!advancedRejectionAvailable" feature="Advanced Rejection"/>
          <BaseInfoIcon :message="HELP.rejection_method"/>
        </label>
        <select
            id="rejection-method-select"
            v-model="local.rejection_method"
            class="select"
            style="width: 150px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
            @change="apply('rejection_method', $event.target.value)"
        >
          <option
              v-for="opt in REJECTION_METHOD_OPTIONS"
              :key="opt.value"
              :value="opt.value"
              :disabled="opt.pro && !advancedRejectionAvailable"
          >
            {{ opt.label }} {{ opt.pro && !advancedRejectionAvailable ? '🔒' : '' }}
          </option>
        </select>
      </div>
    </div>

    <BaseSlider
        v-if="local.rejection_method !== 'None'"
        v-model="local.rejection_sigma"
        label="Rejection Sigma"
        :min="0.5"
        :max="10.0"
        :step="0.1"
        :disabled="!advancedRejectionAvailable"
        :help="HELP.rejection_sigma"
        @change="applyDebounced('rejection_sigma', local.rejection_sigma)"
    >
      <template #label-extra>
        <BaseProLock v-if="!advancedRejectionAvailable" feature="Advanced Rejection"/>
      </template>
    </BaseSlider>
  </div>

  <div v-if="planetary" class="settings-section">
    <h3 class="section-title">Planetary</h3>

    <div class="control-group">
      <BaseToggle
          v-model="local.planetary_auto_tracking"
          label="Auto Tracking"
          help="Automatically track the planet's centroid in the ROI to prevent drift."
          @update:model-value="apply('planetary_auto_tracking', $event)"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.planetary_multi_point_alignment"
          label="Multi-Point Alignment"
          help="Use Inverse Distance Weighting on multiple surface points to combat seeing distortion."
          :disabled="!advancedPlanetaryAvailable"
          @update:model-value="apply('planetary_multi_point_alignment', $event)"
      >
        <template #label-extra>
          <BaseProLock v-if="!advancedPlanetaryAvailable" feature="Multi-Point Alignment"/>
        </template>
      </BaseToggle>
    </div>
  </div>
</template>

<style scoped>
.settings-section {
  margin-bottom: 0.625rem;
}
</style>
