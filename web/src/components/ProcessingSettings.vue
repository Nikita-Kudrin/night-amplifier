<script setup>
/**
 * Settings → Processing: colour intensity, the eyepiece group's tone fields, background
 * subtraction and the shadow saturation boost.
 *
 * Mirrors those keys of the panel's settings and emits `apply(key, value)`; the tone
 * fields belong to the `eyepiece` group, so they are sent as the whole group, which
 * `EyepieceSettings` follows.
 */
import {BaseToggle, BaseSlider, BaseProLock, SettingSelect} from './ui'
import {useSectionMirror} from '../composables/useSectionMirror.js'
import {formatPercent} from '../utils/format.js'
import {copySetting} from '../utils/settings.js'
import {
  AUTO_STRETCH_INTENSITY,
  BACKGROUND_ALGORITHM_OPTIONS,
  BLACK_FLOOR_LIMITS,
  BLACK_LEVEL_LIMITS,
  HELP_TEXTS,
  SATURATION_BOOST_LIMITS,
} from '../constants'

const props = defineProps({
  /** The panel's settings mirror (`settingsMirror`). */
  settings: {type: Object, required: true},
  /** Focus/Finder mode holds dithering, background subtraction and the boost off. */
  focusMode: {type: Boolean, default: false},
  rbfBackgroundAvailable: {type: Boolean, default: false},
  saturationBoostAvailable: {type: Boolean, default: false},
})

const emit = defineEmits(['apply'])

const HELP = HELP_TEXTS

const local = useSectionMirror(() => props.settings, [
  'auto_stretch_intensity',
  'eyepiece',
  'background_subtraction',
  'background_extraction_algorithm',
  'saturation_boost',
  'saturation_boost_strength',
])

function apply(key) {
  emit('apply', key, copySetting(local[key]))
}
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Processing</h3>

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
          @change="apply('eyepiece')"
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
          @change="apply('eyepiece')"
      />
    </div>

    <div class="control-group control-group-roomy">
      <BaseToggle
          v-model="local.eyepiece.darker_sky"
          label="Darker sky"
          :disabled="local.eyepiece.black_floor >= 0"
          :help="HELP.eyepiece_darker_sky"
          @update:model-value="apply('eyepiece')"
      />
    </div>

    <span v-if="focusMode" class="hint">{{ HELP.focus_mode_held }}</span>

    <div class="control-group control-group-roomy">
      <BaseToggle
          v-model="local.eyepiece.dither"
          label="Dithering"
          :help="HELP.eyepiece_dither"
          :disabled="focusMode"
          @update:model-value="apply('eyepiece')"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.background_subtraction"
          label="Background Subtraction"
          :help="HELP.background_subtraction"
          :disabled="focusMode"
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
          :disabled="!saturationBoostAvailable || focusMode"
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
        :disabled="!saturationBoostAvailable"
        :help="HELP.saturation_boost_strength"
        @change="apply('saturation_boost_strength')"
    >
      <template #label-extra>
        <BaseProLock v-if="!saturationBoostAvailable" feature="Saturation Boost"/>
      </template>
    </BaseSlider>
  </div>
</template>
