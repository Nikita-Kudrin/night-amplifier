<script setup lang="ts">
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
import type {Option} from '../constants/tables.js'
import type {SettingApply} from '../utils/settings.js'
import type {RejectionMethod, Settings, WeightingPreset} from '../composables/api.types.js'

const props = withDefaults(defineProps<{
  weightingPreset: WeightingPreset
  rejectionMethod: RejectionMethod
  rejectionSigma: number
  planetaryAutoTracking: boolean
  planetaryMultiPointAlignment: boolean
  autoCrop?: boolean
  /** Whether the stacking type is deep sky, the one mode whose stack has a coverage map to crop by. */
  deepSky?: boolean
  /** Whether the stacking type is planetary, which adds that mode's section. */
  planetary?: boolean
  advancedRejectionAvailable?: boolean
  advancedPlanetaryAvailable?: boolean
}>(), {
  autoCrop: false,
  deepSky: true,
  planetary: false,
  advancedRejectionAvailable: false,
  advancedPlanetaryAvailable: false,
})

const emit = defineEmits<{apply: SettingApply}>()

const HELP = HELP_TEXTS

/** The select holds any method the server may report; it offers the ones a live stack runs. */
const REJECTION_METHODS: readonly Option<RejectionMethod>[] = REJECTION_METHOD_OPTIONS

const local = reactive<Pick<Settings,
  | 'weighting_preset'
  | 'rejection_method'
  | 'rejection_sigma'
  | 'planetary_auto_tracking'
  | 'planetary_multi_point_alignment'
  | 'auto_crop'
>>({
  weighting_preset: props.weightingPreset,
  rejection_method: props.rejectionMethod,
  rejection_sigma: props.rejectionSigma,
  planetary_auto_tracking: props.planetaryAutoTracking,
  planetary_multi_point_alignment: props.planetaryMultiPointAlignment,
  auto_crop: props.autoCrop,
})

watch(
    () => [
      props.weightingPreset,
      props.rejectionMethod,
      props.rejectionSigma,
      props.planetaryAutoTracking,
      props.planetaryMultiPointAlignment,
      props.autoCrop,
    ] as const,
    ([weighting, rejection, sigma, autoTracking, multiPoint, autoCrop]) => {
      local.weighting_preset = weighting
      local.rejection_method = rejection
      local.rejection_sigma = sigma
      local.planetary_auto_tracking = autoTracking
      local.planetary_multi_point_alignment = multiPoint
      local.auto_crop = autoCrop
    }
)

function apply(...args: SettingApply) {
  emit('apply', ...args)
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
          :options="REJECTION_METHODS"
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
