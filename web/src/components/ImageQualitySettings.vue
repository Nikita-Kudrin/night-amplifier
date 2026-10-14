<script setup lang="ts">
/**
 * Sensor corrections and noise reduction: raw-mosaic corrections (pre-demosaic) and the
 * spatial filters run at view resolution — the two groups deciding image cleanliness
 * before anything cosmetic. Extracted from `SettingsPanel.vue` once its sections grew hard to find.
 *
 * Edits a local mirror, emitting `apply(key, value)` on commit (toggle immediately,
 * slider at drag end) — the parent stays sole owner of the settings.
 */
import {computed, reactive, watch} from 'vue'
import {BaseToggle, BaseSlider, BaseProLock, SettingSelect} from './ui'
import {formatPercent, formatSigma} from '../utils/format'
import {
  DENOISE_CHROMA_STRENGTH_LIMITS,
  BACKGROUND_GRAIN_LIMITS,
  DENOISE_LUMA_STRENGTH_LIMITS,
  DETAIL_LIMITS,
  HOT_PIXEL_SIGMA_LIMITS,
  PREVIEW_RESOLUTION_OPTIONS,
  STREAMING_RESOLUTION_OPTIONS,
  HELP_TEXTS,
} from '../constants'
import {applyArgs} from '../utils/settings'
import type {SettingApply} from '../utils/settings'
import type {DenoiseSettings, Resolution, SensorCorrectionSettings, Settings} from '../composables/api.types'

const props = withDefaults(defineProps<{
  sensorCorrection: SensorCorrectionSettings
  denoise: DenoiseSettings
  previewResolution: Resolution
  streamingResolution: Resolution
  /**
   * Focus/Finder mode holds three of these toggles off and owns the snapshot that
   * restores them, so while it is on they are read-only — an edit that looked like it
   * took and then reverted on the next toggle is worse than one the UI refuses.
   */
  focusMode?: boolean
  /**
   * Whether the denoise plugin is present. The filters and every control over them are
   * a Pro feature; without it the section stays visible, locked, so it is clear what
   * the build does not do rather than silently missing.
   */
  denoiseAvailable?: boolean
  /** Whether the AI denoiser is present; its switch lives in Settings → Advanced. */
  aiDenoiseAvailable?: boolean
}>(), {
  focusMode: false,
  denoiseAvailable: false,
  aiDenoiseAvailable: false,
})

const emit = defineEmits<{apply: SettingApply}>()

const HELP = HELP_TEXTS

type Group = 'sensor_correction' | 'denoise'
type Choice = 'preview_resolution' | 'streaming_resolution'

const local = reactive<Pick<Settings, Group | Choice>>({
  sensor_correction: {...props.sensorCorrection},
  denoise: {...props.denoise},
  preview_resolution: props.previewResolution,
  streaming_resolution: props.streamingResolution,
})

/**
 * The four filter controls are inert without the plugin and while the master switch is
 * off. The switch itself is deliberately not held by Focus/Finder mode: the mode reaches
 * "off" through the strengths it snapshots, and a switch it forced false is how a saved
 * file once lost the Background Grain dial for good.
 */
const filtersLocked = computed(() => !props.denoiseAvailable || !local.denoise.enabled)

/**
 * While the network runs it has the fine and mid scales, so Structure strength and Detail
 * grey out; Colour Mottle and Background Grain keep working. A switch saved on a build
 * without the network, or held off by Focus/Finder mode, replaces nothing.
 */
const aiReplacing = computed(
    () =>
        props.aiDenoiseAvailable && !filtersLocked.value && !props.focusMode && Boolean(local.denoise.ai)
)

/**
 * Detail is a gain on the brightness denoiser's own planes, so it has nothing to work on
 * while that is held off — by Focus/Finder mode, a zero Structure strength, or the network
 * taking those scales. The mode does not manage it: the value is the observer's and is
 * not snapshotted.
 */
const detailLocked = computed(
    () =>
        filtersLocked.value ||
        props.focusMode ||
        !(local.denoise.luma_strength > 0) ||
        aiReplacing.value
)

watch(
    () => [props.sensorCorrection, props.denoise, props.previewResolution, props.streamingResolution] as const,
    ([sensorCorrection, denoise, previewResolution, streamingResolution]) => {
      Object.assign(local.sensor_correction, sensorCorrection)
      Object.assign(local.denoise, denoise)
      local.preview_resolution = previewResolution
      local.streaming_resolution = streamingResolution
    },
    {deep: true}
)

function apply(key: Group) {
  emit('apply', ...applyArgs(local, key, (group) => ({...group})))
}

/** For the settings that are a bare value rather than a group. */
function applyValue(key: Choice, value: Resolution) {
  local[key] = value
  emit('apply', ...applyArgs(local, key))
}
</script>

<template>
  <!-- Sensor corrections: applied to the raw mosaic, before demosaic -->
  <div class="settings-section">
    <h3 class="section-title">Sensor</h3>

    <span v-if="focusMode" class="hint">{{ HELP.focus_mode_held }}</span>

    <div class="control-group control-group-offset control-group-roomy">
      <BaseSlider
          v-model="local.sensor_correction.hot_pixel_sigma"
          label="Hot Pixel Threshold"
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
          :disabled="focusMode"
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

  <!-- Preview and streaming resolutions: settings, deliberately not client-driven -->
  <div class="settings-section">
    <h3 class="section-title">Preview</h3>

    <div class="control-group control-group-offset">
      <SettingSelect
          id="preview-resolution-select"
          v-model="local.preview_resolution"
          label="Processing Resolution"
          :help="HELP.preview_resolution"
          :options="PREVIEW_RESOLUTION_OPTIONS"
          @change="applyValue('preview_resolution', $event)"
      />
    </div>

    <div class="control-group">
      <SettingSelect
          id="streaming-resolution-select"
          v-model="local.streaming_resolution"
          label="Streaming Resolution"
          :help="HELP.streaming_resolution"
          :options="STREAMING_RESOLUTION_OPTIONS"
          @change="applyValue('streaming_resolution', $event)"
      />
    </div>
  </div>

  <!-- Noise reduction: runs on the streamed image, at its streaming resolution -->
  <div class="settings-section">
    <h3 class="section-title">
      Noise Reduction
      <BaseProLock v-if="!denoiseAvailable" feature="Noise Reduction"/>
    </h3>

    <span v-if="focusMode" class="hint">{{ HELP.focus_mode_held }}</span>
    <span v-if="aiReplacing" class="hint">
      AI denoising (Settings → Advanced) replaces Structure strength and Detail while it runs.
    </span>

    <div class="control-group control-group-offset">
      <BaseToggle
          v-model="local.denoise.enabled"
          label="Denoise"
          :help="HELP.denoise_enabled"
          :disabled="!denoiseAvailable"
          @update:model-value="apply('denoise')"
      />
    </div>

    <div class="control-group">
      <BaseToggle
          v-model="local.denoise.chroma"
          label="Colour Mottle"
          :help="HELP.denoise_chroma"
          :disabled="filtersLocked || focusMode"
          @update:model-value="apply('denoise')"
      />
    </div>

    <div v-if="local.denoise.chroma" class="control-group control-group-roomy">
      <BaseSlider
          v-model="local.denoise.chroma_strength"
          label="Colour strength"
          large-gap
          :min="DENOISE_CHROMA_STRENGTH_LIMITS.min"
          :max="DENOISE_CHROMA_STRENGTH_LIMITS.max"
          :step="DENOISE_CHROMA_STRENGTH_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_chroma_strength"
          :disabled="filtersLocked"
          @change="apply('denoise')"
      />
    </div>

    <div class="control-group control-group-roomy">
      <BaseSlider
          v-model="local.denoise.background_grain"
          label="Background Grain"
          large-gap
          :min="BACKGROUND_GRAIN_LIMITS.min"
          :max="BACKGROUND_GRAIN_LIMITS.max"
          :step="BACKGROUND_GRAIN_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_background_grain"
          :disabled="filtersLocked"
          @change="apply('denoise')"
      />
      <span v-if="aiReplacing" class="hint">
        Above 50% it also smooths the larger mottle the network cannot reach.
      </span>
    </div>

    <div class="control-group control-group-roomy">
      <BaseSlider
          v-model="local.denoise.luma_strength"
          label="Structure strength"
          large-gap
          :min="DENOISE_LUMA_STRENGTH_LIMITS.min"
          :max="DENOISE_LUMA_STRENGTH_LIMITS.max"
          :step="DENOISE_LUMA_STRENGTH_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_luma_strength"
          :disabled="filtersLocked || focusMode || aiReplacing"
          @change="apply('denoise')"
      />
    </div>

    <div class="control-group control-group-roomy">
      <BaseSlider
          v-model="local.denoise.detail"
          label="Detail"
          large-gap
          :min="DETAIL_LIMITS.min"
          :max="DETAIL_LIMITS.max"
          :step="DETAIL_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_detail"
          :disabled="detailLocked"
          @change="apply('denoise')"
      />
    </div>

  </div>
</template>
