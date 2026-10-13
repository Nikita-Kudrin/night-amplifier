<script setup>
/**
 * Settings → Stacking, plus Planetary while that mode is selected: frame weighting,
 * rejection, Deep Sky's auto crop and the planetary alignment switches. Shown only while
 * stacking is on.
 *
 * Mirrors its values and emits `apply(key, value)` per setting, as `ImageQualitySettings`
 * does; the sigma slider sends on release, as every slider does.
 */
import {reactive, watch} from 'vue'
import {BaseToggle, BaseSlider, BaseProLock, SettingSelect} from './ui'
import {
  HELP_TEXTS,
  REJECTION_METHOD_OPTIONS,
  REJECTION_SIGMA_LIMITS,
  WEIGHTING_PRESET_OPTIONS,
} from '../constants'

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
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Stacking</h3>

    <div class="control-group">
      <SettingSelect
          id="weighting-preset-select"
          v-model="local.weighting_preset"
          label="Frame Weighting"
          :help="HELP.weighting_preset"
          :options="WEIGHTING_PRESET_OPTIONS"
          select-class="weighting-select"
          @change="apply('weighting_preset', $event)"
      />
    </div>

    <div class="control-group">
      <SettingSelect
          id="rejection-method-select"
          v-model="local.rejection_method"
          label="Rejection Method"
          :help="HELP.rejection_method"
          :options="REJECTION_METHOD_OPTIONS"
          :locked="!advancedRejectionAvailable"
          pro-feature="Advanced Rejection"
          @change="apply('rejection_method', $event)"
      />
    </div>

    <BaseSlider
        v-if="local.rejection_method !== 'None'"
        v-model="local.rejection_sigma"
        label="Rejection Sigma"
        :min="REJECTION_SIGMA_LIMITS.min"
        :max="REJECTION_SIGMA_LIMITS.max"
        :step="REJECTION_SIGMA_LIMITS.step"
        :disabled="!advancedRejectionAvailable"
        :help="HELP.rejection_sigma"
        @change="apply('rejection_sigma', $event)"
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
:help="HELP.planetary_auto_tracking"
          @update:model-value="apply('planetary_auto_tracking', $event)"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.planetary_multi_point_alignment"
          label="Multi-Point Alignment"
:help="HELP.planetary_multi_point_alignment"
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
:deep(.weighting-select) {
  width: 120px;
}
</style>
