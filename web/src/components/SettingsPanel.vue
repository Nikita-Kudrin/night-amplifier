<script setup>
import {ref, inject, watch, computed} from 'vue'
import {updateSettings} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {useCapabilities} from '../composables/appContext.js'
import {formatPercent} from '../utils/format.js'
import {useDebouncedCall} from '../composables/useDebouncedCall.js'
import {
  BasePanel,
  BaseToggle,
  BaseSlider,
  ButtonGroup,
  BaseAlert,
  BaseInfoIcon,
  BaseProLock,
} from './ui'
import AiComputeSettings from './AiComputeSettings.vue'
import CoolerControl from './CoolerControl.vue'
import DewHeaterControl from './DewHeaterControl.vue'
import EyepieceSettings from './EyepieceSettings.vue'
import ImageQualitySettings from './ImageQualitySettings.vue'
import StackingSettings from './StackingSettings.vue'
import {
  SATURATION_BOOST_LIMITS,
  AUTO_STRETCH_INTENSITY,
  BLACK_LEVEL_LIMITS,
    BLACK_FLOOR_LIMITS,
  BINNING_OPTIONS,
  DEFAULT_SETTINGS,
  defaultSettings,
  RAW_FRAME_SAVING_MODES,
  BACKGROUND_ALGORITHM_OPTIONS,
  SIMULATED_PRELOAD_LIMITS,
  HELP_TEXTS,
} from '../constants'

const settings = inject('settings')
const refreshSettings = inject('refreshSettings')
const simulatorEnabledRef = inject('simulatorEnabled')
const hasGuideCamera = inject('hasGuideCamera', computed(() => false))
const {
  rbfBackground,
  saturationBoost,
  denoise: denoiseAvailable,
  aiDenoise: aiDenoiseAvailable,
  advancedRejection,
  advancedPlanetary,
} = useCapabilities()

const {error, clearError, withErrorHandling} = useError()

/**
 * Read straight from the server settings rather than the local mirror: the mode is
 * toggled from the capture panel, and the mirror only refreshes on a save from here.
 */
const focusMode = computed(() => settings?.value?.focus_mode ?? false)

const simulatorEnabled = computed({
  get: () => simulatorEnabledRef?.value ?? false,
  set: (val) => {
    if (simulatorEnabledRef) simulatorEnabledRef.value = val
  },
})

const localSettings = ref(defaultSettings())

watch(
    settings,
    (newSettings) => {
      if (newSettings) {
        localSettings.value = {
          bin: newSettings.bin ?? DEFAULT_SETTINGS.bin,
          stacking: newSettings.stacking ?? DEFAULT_SETTINGS.stacking,
          background_subtraction:
              newSettings.background_subtraction ?? DEFAULT_SETTINGS.background_subtraction,
          background_extraction_algorithm:
              newSettings.background_extraction_algorithm ??
              DEFAULT_SETTINGS.background_extraction_algorithm,
          auto_stretch_intensity:
              newSettings.auto_stretch_intensity ?? DEFAULT_SETTINGS.auto_stretch_intensity,
          raw_frame_saving: newSettings.raw_frame_saving
              ? {...newSettings.raw_frame_saving}
              : {...DEFAULT_SETTINGS.raw_frame_saving},
          save_stacked_image: newSettings.save_stacked_image ?? DEFAULT_SETTINGS.save_stacked_image,
          wanderer_mode: newSettings.wanderer_mode ?? DEFAULT_SETTINGS.wanderer_mode,
          auto_crop: newSettings.auto_crop ?? DEFAULT_SETTINGS.auto_crop,
          auto_reconnect: newSettings.auto_reconnect ?? DEFAULT_SETTINGS.auto_reconnect,
          auto_resume_capture:
              newSettings.auto_resume_capture ?? DEFAULT_SETTINGS.auto_resume_capture,
          weighting_preset: newSettings.weighting_preset ?? DEFAULT_SETTINGS.weighting_preset,
          rejection_method: newSettings.rejection_method ?? DEFAULT_SETTINGS.rejection_method,
          rejection_sigma: newSettings.rejection_sigma ?? DEFAULT_SETTINGS.rejection_sigma,
          planetary_auto_tracking: newSettings.planetary_auto_tracking ?? true,
          planetary_multi_point_alignment: newSettings.planetary_multi_point_alignment ?? false,
          saturation_boost: newSettings.saturation_boost ?? DEFAULT_SETTINGS.saturation_boost,
          saturation_boost_strength:
              newSettings.saturation_boost_strength ?? DEFAULT_SETTINGS.saturation_boost_strength,
          simulated_preload_images:
              newSettings.simulated_preload_images ?? DEFAULT_SETTINGS.simulated_preload_images,
          show_focus_image: newSettings.show_focus_image ?? DEFAULT_SETTINGS.show_focus_image,
          force_focus_image_now: newSettings.force_focus_image_now ?? DEFAULT_SETTINGS.force_focus_image_now,
          eyepiece: newSettings.eyepiece
              ? {...DEFAULT_SETTINGS.eyepiece, ...newSettings.eyepiece}
              : {...DEFAULT_SETTINGS.eyepiece},
          sensor_correction: newSettings.sensor_correction
              ? {...newSettings.sensor_correction}
              : {...DEFAULT_SETTINGS.sensor_correction},
          denoise: {...DEFAULT_SETTINGS.denoise, ...newSettings.denoise},
          preview_resolution:
              newSettings.preview_resolution ?? DEFAULT_SETTINGS.preview_resolution,
          streaming_resolution:
              newSettings.streaming_resolution ?? DEFAULT_SETTINGS.streaming_resolution,
        }
      }
    },
    {immediate: true}
)

/**
 * Persist a whole settings group edited by a child component, keeping the
 * panel's own copy in step.
 */
function applyGroup(key, value) {
  localSettings.value[key] = value
  return applySetting(key, value)
}

/**
 * Flip one mode's raw-frame switch, sending the whole group so the server never sees a
 * partial selection.
 */
/** The guide switch is only meaningful with a guide camera attached. */
const rawFrameSavingModes = computed(() =>
    RAW_FRAME_SAVING_MODES.filter((mode) => !mode.requiresGuideCamera || hasGuideCamera.value)
)

function applyRawFrameSaving(mode, enabled) {
  return applyGroup('raw_frame_saving', {...localSettings.value.raw_frame_saving, [mode]: enabled})
}

async function applySetting(key, value) {
  await withErrorHandling(async () => {
    await updateSettings({[key]: value})
    await refreshSettings()
  })
}

const {schedule: debouncedApply} = useDebouncedCall(applySetting, 300)

const HELP = HELP_TEXTS
</script>

<template>
  <BasePanel title="Settings">
    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <CoolerControl/>
    <DewHeaterControl/>

    <ImageQualitySettings
        :sensor-correction="localSettings.sensor_correction"
        :denoise="localSettings.denoise"
        :preview-resolution="localSettings.preview_resolution"
        :streaming-resolution="localSettings.streaming_resolution"
        :focus-mode="focusMode"
        :denoise-available="denoiseAvailable"
        :ai-denoise-available="aiDenoiseAvailable"
        @apply="applyGroup"
    />

    <!-- Processing settings -->
    <div class="settings-section">
      <h3 class="section-title">Processing</h3>

      <div class="control-group control-group-offset control-group-roomy">
        <BaseSlider
            v-model="localSettings.auto_stretch_intensity"
            label="Color Intensity"
            large-gap
            :min="AUTO_STRETCH_INTENSITY.min"
            :max="AUTO_STRETCH_INTENSITY.max"
            :step="AUTO_STRETCH_INTENSITY.step"
            :format-value="formatPercent"
            :help="HELP.auto_stretch_intensity"
            @change="applySetting('auto_stretch_intensity', localSettings.auto_stretch_intensity)"
        />
      </div>

      <div class="control-group control-group-roomy">
        <BaseSlider
            v-model="localSettings.eyepiece.intensity"
            label="Black level"
            large-gap
            :min="BLACK_LEVEL_LIMITS.min"
            :max="BLACK_LEVEL_LIMITS.max"
            :step="BLACK_LEVEL_LIMITS.step"
            :format-value="formatPercent"
            :help="HELP.eyepiece_intensity"
            @change="applySetting('eyepiece', localSettings.eyepiece)"
        />
      </div>

      <div class="control-group control-group-roomy">
        <BaseSlider
            v-model="localSettings.eyepiece.black_floor"
            label="Black floor"
            large-gap
            :min="BLACK_FLOOR_LIMITS.min"
            :max="BLACK_FLOOR_LIMITS.max"
            :step="BLACK_FLOOR_LIMITS.step"
            :format-value="formatPercent"
            :help="HELP.eyepiece_black_floor"
            @change="applySetting('eyepiece', localSettings.eyepiece)"
        />
      </div>

      <div class="control-group control-group-roomy">
        <BaseToggle
            v-model="localSettings.eyepiece.darker_sky"
            label="Darker sky"
            :disabled="localSettings.eyepiece.black_floor >= 0"
            :help="HELP.eyepiece_darker_sky"
            @update:model-value="applySetting('eyepiece', localSettings.eyepiece)"
        />
      </div>

      <span v-if="focusMode" class="hint">Greyed settings are held off by Focus/Finder mode.</span>

      <div class="control-group control-group-roomy">
        <BaseToggle
            v-model="localSettings.eyepiece.dither"
            label="Dithering"
            :help="HELP.eyepiece_dither"
            :disabled="focusMode"
            @update:model-value="applySetting('eyepiece', localSettings.eyepiece)"
        />
      </div>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.background_subtraction"
            label="Background Subtraction"
            :help="HELP.background_subtraction"
            :disabled="focusMode"
            @update:model-value="applySetting('background_subtraction', $event)"
        />
      </div>

      <div v-if="localSettings.background_subtraction" class="control-group">
        <div class="control-row">
          <label class="control-label control-label-inline">
            Algorithm
            <BaseProLock v-if="!rbfBackground" feature="RBF Background"/>
            <BaseInfoIcon :message="HELP.background_extraction_algorithm"/>
          </label>
          <select
              id="bg-algorithm-select"
              v-model="localSettings.background_extraction_algorithm"
              class="select select-compact"
              @change="applySetting('background_extraction_algorithm', $event.target.value)"
          >
            <option
                v-for="opt in BACKGROUND_ALGORITHM_OPTIONS"
                :key="opt.value"
                :value="opt.value"
                :disabled="opt.pro && !rbfBackground"
            >
              {{ opt.label }} {{ opt.pro && !rbfBackground ? '🔒' : '' }}
            </option>
          </select>
        </div>
      </div>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.saturation_boost"
            label="Shadow Saturation Boost"
            :help="HELP.saturation_boost"
            :disabled="!saturationBoost || focusMode"
            @update:model-value="applySetting('saturation_boost', $event)"
        >
          <template #label-extra>
            <BaseProLock
                v-if="!saturationBoost"
                feature="Saturation Boost"
            />
          </template>
        </BaseToggle>
      </div>


      <BaseSlider
          v-if="localSettings.saturation_boost"
          v-model="localSettings.saturation_boost_strength"
          label="Saturation Strength"
          large-gap
          :min="SATURATION_BOOST_LIMITS.min"
          :max="SATURATION_BOOST_LIMITS.max"
          :step="SATURATION_BOOST_LIMITS.step"
          :format-value="formatPercent"
          :disabled="!saturationBoost"
          :help="HELP.saturation_boost_strength"
          @change="
          debouncedApply('saturation_boost_strength', localSettings.saturation_boost_strength)
        "
      >
        <template #label-extra>
          <BaseProLock v-if="!saturationBoost" feature="Saturation Boost"/>
        </template>
      </BaseSlider>
    </div>

    <!-- Storage settings: raw frames are per capture mode, the stack only exists in Stacking -->
    <div class="settings-section">
      <h3 class="section-title">Storage</h3>

      <div class="control-group">
        <span class="storage-group-label">
          Save Raw Frames
          <BaseInfoIcon :message="HELP.raw_frame_saving"/>
        </span>

        <BaseToggle
            v-for="mode in rawFrameSavingModes"
            :key="mode.key"
            v-model="localSettings.raw_frame_saving[mode.key]"
            :label="mode.label"
            class="storage-mode-toggle"
            @update:model-value="applyRawFrameSaving(mode.key, $event)"
        />
      </div>

      <div v-if="localSettings.stacking && !localSettings.wanderer_mode" class="control-group">
        <BaseToggle
            v-model="localSettings.save_stacked_image"
            label="Save Stacked Image"
            :help="HELP.save_stacked_image"
            @update:model-value="applySetting('save_stacked_image', $event)"
        />
      </div>
    </div>

    <!-- Camera dropout recovery -->
    <div class="settings-section">
      <h3 class="section-title">If the camera drops out</h3>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.auto_reconnect"
            label="Reconnect automatically"
            :help="HELP.auto_reconnect"
            @update:model-value="applySetting('auto_reconnect', $event)"
        />
      </div>

      <div v-if="localSettings.auto_reconnect" class="control-group">
        <BaseToggle
            v-model="localSettings.auto_resume_capture"
            label="Resume the capture"
            :help="HELP.auto_resume_capture"
            @update:model-value="applySetting('auto_resume_capture', $event)"
        />
      </div>
    </div>

    <EyepieceSettings :eyepiece="localSettings.eyepiece" @apply="applyGroup"/>

    <StackingSettings
        v-if="localSettings.stacking"
        :weighting-preset="localSettings.weighting_preset"
        :rejection-method="localSettings.rejection_method"
        :rejection-sigma="localSettings.rejection_sigma"
        :planetary-auto-tracking="localSettings.planetary_auto_tracking"
        :planetary-multi-point-alignment="localSettings.planetary_multi_point_alignment"
        :planetary="settings?.stacking_type === 'planetary'"
        :auto-crop="localSettings.auto_crop"
        :deep-sky="(settings?.stacking_type ?? 'deep_sky') === 'deep_sky'"
        :advanced-rejection-available="advancedRejection"
        :advanced-planetary-available="advancedPlanetary"
        @apply="applyGroup"
    />

    <!-- Advanced settings -->
    <div class="settings-section">
      <h3 class="section-title">Advanced</h3>

      <div class="control-group">
        <div class="control-row">
          <label class="type-label-inline">
            Binning
            <BaseInfoIcon :message="HELP.bin"/>
          </label>
          <ButtonGroup
              v-model="localSettings.bin"
              :options="BINNING_OPTIONS"
              @update:model-value="applySetting('bin', $event)"
          />
        </div>
      </div>

      <AiComputeSettings
          :denoise="localSettings.denoise"
          :ai-denoise-available="aiDenoiseAvailable"
          :focus-mode="focusMode"
          :save="applyGroup"
          :guard="withErrorHandling"
      />

      <div class="control-group">
        <BaseToggle
            v-model="simulatorEnabled"
            label="Simulated Camera"
            data-test="simulator-toggle"
            :help="HELP.simulated_camera"
            @update:model-value="applySetting('use_simulated_camera', $event)"
        />
      </div>
      <div v-if="simulatorEnabled" class="control-group control-group-offset">
        <BaseSlider
            v-model="localSettings.simulated_preload_images"
            label="Preload Count"
            :min="SIMULATED_PRELOAD_LIMITS.min"
            :max="SIMULATED_PRELOAD_LIMITS.max"
            :step="SIMULATED_PRELOAD_LIMITS.step"
            :help="HELP.simulated_preload_count"
            @change="
            debouncedApply('simulated_preload_images', localSettings.simulated_preload_images)
          "
        />
      </div>

      <div class="control-group">
        <div class="control-row">
          <BaseToggle
              v-model="localSettings.show_focus_image"
              label="Show focus image"
              size="small"
              :help="HELP.show_focus_image"
              @update:model-value="applySetting('show_focus_image', $event)"
          />
          <BaseToggle
              v-if="localSettings.show_focus_image"
              v-model="localSettings.force_focus_image_now"
              label="Now"
              size="small"
              :help="HELP.force_focus_image_now"
              @update:model-value="applySetting('force_focus_image_now', $event)"
          />
        </div>
      </div>
    </div>
  </BasePanel>
</template>

<style scoped>
/* Uses global .section-title, .control-group, .control-label, .hint from main.css */

/* Heading for a set of related switches, matching .control-label's type without its
   space-between, which would strand the info icon at the far edge. */
.storage-group-label {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  font-size: var(--fs-md);
  font-weight: 500;
  color: var(--text-secondary);
  margin-bottom: 0.375rem;
}

.storage-mode-toggle {
  margin-left: 0.75rem;
  margin-top: 0.25rem;
}
</style>
