<script setup>
/**
 * Sensor corrections and noise reduction: raw-mosaic corrections (pre-demosaic)
 * and the spatial filters the encoders run at view resolution — the two groups
 * that decide image cleanliness before anything cosmetic. Extracted from
 * `SettingsPanel.vue`, which had grown past the point its sections were findable.
 *
 * Edits a local mirror, not the props, and emits `apply(key, value)` when a
 * control commits (a toggle immediately, a slider at drag end) — the parent
 * stays the single owner of the settings object and persistence.
 */
import {computed, reactive, watch} from 'vue'
import {BaseToggle, BaseSlider, BaseInfoIcon, BaseProLock} from './ui'
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

const props = defineProps({
  sensorCorrection: {type: Object, required: true},
  denoise: {type: Object, required: true},
  previewResolution: {type: String, required: true},
  streamingResolution: {type: String, required: true},
  /**
   * Focus/Finder mode holds three of these toggles off and owns the snapshot that
   * restores them, so while it is on they are read-only — an edit that looked like it
   * took and then reverted on the next toggle is worse than one the UI refuses.
   */
  focusMode: {type: Boolean, default: false},
  /**
   * Whether the denoise plugin is present. The filters and every control over them are
   * a Pro feature; without it the section stays visible, locked, so it is clear what
   * the build does not do rather than silently missing.
   */
  denoiseAvailable: {type: Boolean, default: false},
  formatPercent: {type: Function, required: true},
  formatSigma: {type: Function, required: true},
})

const emit = defineEmits(['apply'])

const HELP = HELP_TEXTS

const local = reactive({
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
 * Detail is a gain on the brightness denoiser's own planes, so it has nothing to work on
 * while that is held off — by Focus/Finder mode or by a zero Structure strength. The
 * mode does not manage it: the value is the observer's and is not snapshotted.
 */
const detailLocked = computed(
    () => filtersLocked.value || props.focusMode || !(local.denoise.luma_strength > 0)
)

watch(
    () => [props.sensorCorrection, props.denoise, props.previewResolution, props.streamingResolution],
    ([sensorCorrection, denoise, previewResolution, streamingResolution]) => {
      Object.assign(local.sensor_correction, sensorCorrection)
      Object.assign(local.denoise, denoise)
      local.preview_resolution = previewResolution
      local.streaming_resolution = streamingResolution
    },
    {deep: true}
)

function apply(key) {
  emit('apply', key, {...local[key]})
}

/** For the settings that are a bare value rather than a group. */
function applyValue(key, value) {
  local[key] = value
  emit('apply', key, value)
}
</script>

<template>
  <!-- Sensor corrections: applied to the raw mosaic, before demosaic -->
  <div class="settings-section">
    <h3 class="section-title">Sensor</h3>

    <span v-if="focusMode" class="hint">Greyed settings are held off by Focus/Finder mode.</span>

    <div class="control-group" style="margin-top: 0.5rem; margin-bottom: 1.5rem">
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

    <div class="control-group" style="margin-top: 0.5rem">
      <div class="control-row">
        <label class="control-label" style="margin-bottom: 0; flex: 1">
          Processing Resolution
          <BaseInfoIcon :message="HELP.preview_resolution"/>
        </label>
        <select
            id="preview-resolution-select"
            v-model="local.preview_resolution"
            class="select"
            style="width: 150px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
            @change="applyValue('preview_resolution', $event.target.value)"
        >
          <option v-for="opt in PREVIEW_RESOLUTION_OPTIONS" :key="opt.value" :value="opt.value">
            {{ opt.label }}
          </option>
        </select>
      </div>
    </div>

    <div class="control-group">
      <div class="control-row">
        <label class="control-label" style="margin-bottom: 0; flex: 1">
          Streaming Resolution
          <BaseInfoIcon :message="HELP.streaming_resolution"/>
        </label>
        <select
            id="streaming-resolution-select"
            v-model="local.streaming_resolution"
            class="select"
            style="width: 150px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
            @change="applyValue('streaming_resolution', $event.target.value)"
        >
          <option v-for="opt in STREAMING_RESOLUTION_OPTIONS" :key="opt.value" :value="opt.value">
            {{ opt.label }}
          </option>
        </select>
      </div>
    </div>
  </div>

  <!-- Noise reduction: runs on the streamed image, at its streaming resolution -->
  <div class="settings-section">
    <h3 class="section-title">
      Noise Reduction
      <BaseProLock v-if="!denoiseAvailable" feature="Noise Reduction"/>
    </h3>

    <span v-if="focusMode" class="hint">Greyed settings are held off by Focus/Finder mode.</span>

    <div class="control-group" style="margin-top: 0.5rem">
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
          :disabled="filtersLocked"
          @change="apply('denoise')"
      />
    </div>

    <div class="control-group" style="margin-bottom: 1.5rem">
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
    </div>

    <div class="control-group" style="margin-bottom: 1.5rem">
      <BaseSlider
          v-model="local.denoise.luma_strength"
          label="Structure strength"
          large-gap
          :min="DENOISE_LUMA_STRENGTH_LIMITS.min"
          :max="DENOISE_LUMA_STRENGTH_LIMITS.max"
          :step="DENOISE_LUMA_STRENGTH_LIMITS.step"
          :format-value="formatPercent"
          :help="HELP.denoise_luma_strength"
          :disabled="filtersLocked || focusMode"
          @change="apply('denoise')"
      />
    </div>

    <div class="control-group" style="margin-bottom: 1.5rem">
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
