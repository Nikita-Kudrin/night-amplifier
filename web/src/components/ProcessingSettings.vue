<script setup lang="ts">
/**
 * Settings → Processing: colour intensity, the eyepiece group's tone fields, background
 * subtraction and the shadow saturation boost.
 *
 * Mirrors those keys of the panel's settings and emits `apply(key, value)`. The tone fields
 * belong to the `eyepiece` group, which Eyepiece edits too, so only the changed field goes
 * (`apply('eyepiece', {field})`) and the panel sends it over its own copy of the group.
 */
import {BaseToggle, BaseSlider, BaseProLock, SettingSelect} from './ui'
import {useSectionMirror} from '../composables/useSectionMirror'
import {formatPercent} from '../utils/format'
import {applyArgs} from '../utils/settings'
import type {SettingApply} from '../utils/settings'
import {
  AUTO_STRETCH_INTENSITY,
  BACKGROUND_ALGORITHM_OPTIONS,
  BLACK_FLOOR_LIMITS,
  BLACK_LEVEL_LIMITS,
  HELP_TEXTS,
  SATURATION_BOOST_LIMITS,
} from '../constants'
import type {EyepieceSettings, Settings} from '../composables/api.types'

const props = withDefaults(defineProps<{
  /** The panel's settings mirror (`settingsMirror`). */
  settings: Settings
  /** Focus/Finder mode holds dithering, background subtraction and the boost off. */
  focusMode?: boolean
  /** Planetary renders through its own levels: colour intensity, Black level, background
   *  subtraction, the boost and Darker sky do nothing there, so they are held rather than
   *  hidden. Black floor stays, since its lifting half still applies. */
  planetary?: boolean
  rbfBackgroundAvailable?: boolean
  saturationBoostAvailable?: boolean
}>(), {
  focusMode: false,
  planetary: false,
  rbfBackgroundAvailable: false,
  saturationBoostAvailable: false,
})

const emit = defineEmits<{apply: SettingApply}>()

const HELP = HELP_TEXTS

const local = useSectionMirror(() => props.settings, [
  'auto_stretch_intensity',
  'eyepiece',
  'background_subtraction',
  'background_extraction_algorithm',
  'saturation_boost',
  'saturation_boost_strength',
])

function apply(key: keyof typeof local) {
  emit('apply', ...applyArgs(local, key))
}

/** The eyepiece group's fields this section edits. */
type ToneField = keyof Pick<EyepieceSettings, 'intensity' | 'black_floor' | 'darker_sky' | 'dither'>

function applyTone(field: ToneField) {
  emit('apply', 'eyepiece', {[field]: local.eyepiece[field]})
}
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Processing</h3>
    <span v-if="planetary" class="hint">{{ HELP.planetary_held }}</span>

    <div class="control-group control-group-offset control-group-roomy">
      <BaseSlider
          v-model="local.auto_stretch_intensity"
          label="Color Intensity"
          large-gap
          :min="AUTO_STRETCH_INTENSITY.min"
          :max="AUTO_STRETCH_INTENSITY.max"
          :step="AUTO_STRETCH_INTENSITY.step"
          :format-value="formatPercent"
          :help="HELP.auto_stretch_intensity"
          :disabled="planetary"
          @change="apply('auto_stretch_intensity')"
      />
    </div>

    <div class="control-group control-group-roomy">
      <BaseSlider
          v-model="local.eyepiece.intensity"
          label="Black level"
          large-gap
          :min="BLACK_LEVEL_LIMITS.min"
          :max="BLACK_LEVEL_LIMITS.max"
          :step="BLACK_LEVEL_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.eyepiece_intensity"
          :disabled="planetary"
          @change="applyTone('intensity')"
      />
    </div>

    <div class="control-group control-group-roomy">
      <BaseSlider
          v-model="local.eyepiece.black_floor"
          label="Black floor"
          large-gap
          :min="BLACK_FLOOR_LIMITS.min"
          :max="BLACK_FLOOR_LIMITS.max"
          :step="BLACK_FLOOR_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.eyepiece_black_floor"
          @change="applyTone('black_floor')"
      />
    </div>

    <div class="control-group control-group-roomy">
      <BaseToggle
          v-model="local.eyepiece.darker_sky"
          label="Darker sky"
          :disabled="local.eyepiece.black_floor >= 0 || planetary"
          :help="HELP.eyepiece_darker_sky"
          @update:model-value="applyTone('darker_sky')"
      />
    </div>

    <span v-if="focusMode" class="hint">{{ HELP.focus_mode_held }}</span>

    <div class="control-group control-group-roomy">
      <BaseToggle
          v-model="local.eyepiece.dither"
          label="Dithering"
          :help="HELP.eyepiece_dither"
          :disabled="focusMode"
          @update:model-value="applyTone('dither')"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.background_subtraction"
          label="Background Subtraction"
          :help="HELP.background_subtraction"
          :disabled="focusMode || planetary"
          @update:model-value="apply('background_subtraction')"
      />
    </div>

    <div v-if="local.background_subtraction" class="control-group">
      <SettingSelect
          id="bg-algorithm-select"
          v-model="local.background_extraction_algorithm"
          label="Algorithm"
          :help="HELP.background_extraction_algorithm"
          :options="BACKGROUND_ALGORITHM_OPTIONS"
          :disabled="planetary"
          :locked="!rbfBackgroundAvailable"
          pro-feature="RBF Background"
          @change="apply('background_extraction_algorithm')"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.saturation_boost"
          label="Shadow Saturation Boost"
          :help="HELP.saturation_boost"
          :disabled="!saturationBoostAvailable || focusMode || planetary"
          @update:model-value="apply('saturation_boost')"
      >
        <template #label-extra>
          <BaseProLock v-if="!saturationBoostAvailable" feature="Saturation Boost"/>
        </template>
      </BaseToggle>
    </div>

    <BaseSlider
        v-if="local.saturation_boost"
        v-model="local.saturation_boost_strength"
        label="Saturation Strength"
        large-gap
        :min="SATURATION_BOOST_LIMITS.min"
        :max="SATURATION_BOOST_LIMITS.max"
        :step="SATURATION_BOOST_LIMITS.step"
        :format-value="formatPercent"
        :disabled="!saturationBoostAvailable || planetary"
        :help="HELP.saturation_boost_strength"
        @change="apply('saturation_boost_strength')"
    >
      <template #label-extra>
        <BaseProLock v-if="!saturationBoostAvailable" feature="Saturation Boost"/>
      </template>
    </BaseSlider>
  </div>
</template>
