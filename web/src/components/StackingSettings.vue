<script setup>
/**
 * Settings → Stacking, plus Planetary while that mode is selected: frame weighting,
 * rejection, Deep Sky's auto crop and the planetary alignment switches. Shown only while
 * stacking is on.
 *
 * Mirrors its values and emits `apply(key, value)` per setting, as `ImageQualitySettings`
 * does. The sigma slider is sent 300 ms after the last drag.
 */
import {reactive, watch} from 'vue'
import {BaseToggle, BaseSlider, BaseInfoIcon, BaseProLock} from './ui'
import {useDebouncedCall} from '../composables/useDebouncedCall.js'
import {WEIGHTING_PRESET_OPTIONS, REJECTION_METHOD_OPTIONS, HELP_TEXTS} from '../constants'

const props = defineProps({
  weightingPreset: {type: String, required: true},
  rejectionMethod: {type: String, required: true},
  rejectionSigma: {type: Number, required: true},
  planetaryAutoTracking: {type: Boolean, required: true},
  planetaryMultiPointAlignment: {type: Boolean, required: true},
  autoCrop: {type: Boolean, default: false},
  /** Whether the stacking type is deep sky, the one mode whose stack has a coverage map to crop by. */
  deepSky: {type: Boolean, default: true},
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
      props.autoCrop,
    ],
    ([weighting, rejection, sigma, autoTracking, multiPoint, autoCrop]) => {
      local.weighting_preset = weighting
      local.rejection_method = rejection
      local.rejection_sigma = sigma
      local.planetary_auto_tracking = autoTracking
      local.planetary_multi_point_alignment = multiPoint
      local.auto_crop = autoCrop
    },
    {immediate: true}
)

function apply(key, value) {
  emit('apply', key, value)
}

const {schedule: applyDebounced} = useDebouncedCall(apply, 300)
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Stacking</h3>

    <div class="control-group">
      <div class="control-row">
        <label class="control-label control-label-inline">
          Frame Weighting
          <BaseInfoIcon :message="HELP.weighting_preset"/>
        </label>
        <select
            id="weighting-preset-select"
            v-model="local.weighting_preset"
            class="select select-compact weighting-select"
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
        <label class="control-label control-label-inline">
          Rejection Method
          <BaseProLock v-if="!advancedRejectionAvailable" feature="Advanced Rejection"/>
          <BaseInfoIcon :message="HELP.rejection_method"/>
        </label>
        <select
            id="rejection-method-select"
            v-model="local.rejection_method"
            class="select select-compact"
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

    <div v-if="deepSky" class="control-group">
      <BaseToggle
          v-model="local.auto_crop"
          label="Auto Crop"
          :help="HELP.auto_crop"
          @update:model-value="apply('auto_crop', $event)"
      />
    </div>
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
.weighting-select {
  width: 120px;
}
</style>
