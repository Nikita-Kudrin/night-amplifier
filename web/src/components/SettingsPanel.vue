<script setup>
import {ref, inject, watch, computed} from 'vue'
import {updateSettings} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {
  BasePanel,
  BaseToggle,
  BaseSlider,
  ButtonGroup,
  BaseAlert,
  BaseInfoIcon,
  BaseProLock,
} from './ui'
import CoolerControl from './CoolerControl.vue'
import DewHeaterControl from './DewHeaterControl.vue'
import {
  SATURATION_BOOST_LIMITS,
  BINNING_OPTIONS,
  DEFAULT_SETTINGS,
  WEIGHTING_PRESET_OPTIONS,
  BACKGROUND_ALGORITHM_OPTIONS,
  REJECTION_METHOD_OPTIONS,
  SIMULATED_PRELOAD_LIMITS,
  HELP_TEXTS,
} from '../constants'

const settings = inject('settings')
const refreshSettings = inject('refreshSettings')
const simulatorEnabledRef = inject('simulatorEnabled')
const capabilities = inject('capabilities', {
  has_pro: false,
  deep_sky: {advanced_rejection: false, rbf_background: false, saturation_boost: false},
  planetary: {advanced_stacking: false},
  push_to: {astap_solver: false},
})

const {error, clearError, withErrorHandling} = useError()

const simulatorEnabled = computed({
  get: () => simulatorEnabledRef?.value ?? false,
  set: (val) => {
    if (simulatorEnabledRef) simulatorEnabledRef.value = val
  },
})

const localSettings = ref({...DEFAULT_SETTINGS})

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
          save_raw_frames: newSettings.save_raw_frames ?? DEFAULT_SETTINGS.save_raw_frames,
          save_stacked_image: newSettings.save_stacked_image ?? DEFAULT_SETTINGS.save_stacked_image,
          wanderer_mode: newSettings.wanderer_mode ?? DEFAULT_SETTINGS.wanderer_mode,
          weighting_preset: newSettings.weighting_preset ?? DEFAULT_SETTINGS.weighting_preset,
          rejection_method: newSettings.rejection_method ?? DEFAULT_SETTINGS.rejection_method,
          rejection_sigma: newSettings.rejection_sigma ?? DEFAULT_SETTINGS.rejection_sigma,
          saturation_boost: newSettings.saturation_boost ?? DEFAULT_SETTINGS.saturation_boost,
          saturation_boost_strength:
              newSettings.saturation_boost_strength ?? DEFAULT_SETTINGS.saturation_boost_strength,
          simulated_camera: newSettings.simulated_camera ?? DEFAULT_SETTINGS.simulated_camera,
          simulated_preload_images:
              newSettings.simulated_preload_images ?? DEFAULT_SETTINGS.simulated_preload_images,
          eyepiece: newSettings.eyepiece
              ? {...newSettings.eyepiece}
              : {...DEFAULT_SETTINGS.eyepiece},
        }
      }
    },
    {immediate: true}
)

async function applySetting(key, value) {
  await withErrorHandling(async () => {
    await updateSettings({[key]: value})
    await refreshSettings()
  })
}

let debounceTimer = null

function debouncedApply(key, value) {
  clearTimeout(debounceTimer)
  debounceTimer = setTimeout(() => applySetting(key, value), 300)
}

function formatPercent(v) {
  return `${(v * 100).toFixed(0)}%`
}

const HELP = HELP_TEXTS
</script>

<template>
  <BasePanel title="Settings">
    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <CoolerControl/>
    <DewHeaterControl/>

    <!-- Processing settings -->
    <div class="settings-section">
      <h3 class="section-title">Processing</h3>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.background_subtraction"
            label="Background Subtraction"
            :help="HELP.background_subtraction"
            @update:model-value="applySetting('background_subtraction', $event)"
        />
      </div>

      <div v-if="localSettings.background_subtraction" class="control-group">
        <div class="control-row">
          <label class="control-label" style="margin-bottom: 0; flex: 1">
            Algorithm
            <BaseProLock v-if="!capabilities.deep_sky.rbf_background" feature="RBF Background"/>
            <BaseInfoIcon :message="HELP.background_extraction_algorithm"/>
          </label>
          <select
              id="bg-algorithm-select"
              v-model="localSettings.background_extraction_algorithm"
              class="select"
              style="width: 150px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
              @change="applySetting('background_extraction_algorithm', $event.target.value)"
          >
            <option
                v-for="opt in BACKGROUND_ALGORITHM_OPTIONS"
                :key="opt.value"
                :value="opt.value"
                :disabled="opt.pro && !capabilities.deep_sky.rbf_background"
            >
              {{ opt.label }} {{ opt.pro && !capabilities.deep_sky.rbf_background ? '🔒' : '' }}
            </option>
          </select>
        </div>
      </div>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.saturation_boost"
            label="Shadow Saturation Boost"
            :help="HELP.saturation_boost"
            :disabled="!capabilities.deep_sky.saturation_boost"
            @update:model-value="applySetting('saturation_boost', $event)"
        >
          <template #label-extra>
            <BaseProLock
                v-if="!capabilities.deep_sky.saturation_boost"
                feature="Saturation Boost"
            />
          </template>
        </BaseToggle>
      </div>

      <BaseSlider
          v-if="localSettings.saturation_boost"
          v-model="localSettings.saturation_boost_strength"
          label="Saturation Strength"
          :min="SATURATION_BOOST_LIMITS.min"
          :max="SATURATION_BOOST_LIMITS.max"
          :step="SATURATION_BOOST_LIMITS.step"
          :format-value="formatPercent"
          :disabled="!capabilities.deep_sky.saturation_boost"
          :help="HELP.saturation_boost_strength"
          @change="
          debouncedApply('saturation_boost_strength', localSettings.saturation_boost_strength)
        "
      >
        <template #label-extra>
          <BaseProLock v-if="!capabilities.deep_sky.saturation_boost" feature="Saturation Boost"/>
        </template>
      </BaseSlider>
    </div>

    <!-- Storage settings -->
    <div v-if="localSettings.stacking && !localSettings.wanderer_mode" class="settings-section">
      <h3 class="section-title">Storage</h3>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.save_raw_frames"
            label="Save Raw Frames"
            :help="HELP.save_raw_frames"
            @update:model-value="applySetting('save_raw_frames', $event)"
        />
      </div>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.save_stacked_image"
            label="Save Stacked Image"
            :help="HELP.save_stacked_image"
            @update:model-value="applySetting('save_stacked_image', $event)"
        />
      </div>
    </div>

    <!-- Eyepiece settings -->
    <div class="settings-section">
      <h3 class="section-title">Eyepiece</h3>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.eyepiece.circular_view"
            label="Circular view"
            :help="HELP.eyepiece_circular_view"
            @update:model-value="applySetting('eyepiece', localSettings.eyepiece)"
        />
      </div>

      <div class="control-group">
        <BaseToggle
            v-model="localSettings.eyepiece.binoview"
            label="Binoview"
            :help="HELP.eyepiece_binoview"
            @update:model-value="applySetting('eyepiece', localSettings.eyepiece)"
        />
      </div>

      <div
          v-if="localSettings.eyepiece.binoview"
          class="control-group"
          style="flex-direction: column; align-items: stretch; margin-top: 0.5rem"
      >
        <label class="control-label" style="margin-bottom: 0.5rem">
          Screen settings
          <BaseInfoIcon :message="HELP.eyepiece_screen_settings"/>
        </label>

        <div class="control-row" style="justify-content: flex-start; margin-bottom: 0.5rem">
          <input
              v-model.number="localSettings.eyepiece.screen_width"
              type="number"
              min="1"
              step="0.1"
              style="
              width: 70px;
              background: var(--surface);
              color: var(--text-primary);
              border: 1px solid var(--border);
              border-radius: 4px;
              padding: 4px;
            "
              title="Width"
              @change="debouncedApply('eyepiece', localSettings.eyepiece)"
          />
          <span style="margin: 0 4px">x</span>
          <input
              v-model.number="localSettings.eyepiece.screen_height"
              type="number"
              min="1"
              step="0.1"
              style="
              width: 70px;
              background: var(--surface);
              color: var(--text-primary);
              border: 1px solid var(--border);
              border-radius: 4px;
              padding: 4px;
            "
              title="Height"
              @change="debouncedApply('eyepiece', localSettings.eyepiece)"
          />
          <select
              v-model="localSettings.eyepiece.screen_measurement"
              style="
              margin-left: 8px;
              background: var(--surface);
              color: var(--text-primary);
              border: 1px solid var(--border);
              border-radius: 4px;
              padding: 4px;
            "
              @change="applySetting('eyepiece', localSettings.eyepiece)"
          >
            <option value="mm">mm</option>
            <option value="inches">inches</option>
          </select>
        </div>

        <div class="control-row" style="justify-content: flex-start">
          <label style="margin-right: 8px; font-size: 0.9em; color: var(--text-secondary)"
          >Resolution</label
          >
          <input
              v-model.number="localSettings.eyepiece.screen_resolution_x"
              type="number"
              min="1"
              step="1"
              style="
              width: 70px;
              background: var(--surface);
              color: var(--text-primary);
              border: 1px solid var(--border);
              border-radius: 4px;
              padding: 4px;
            "
              title="Resolution X"
              @change="debouncedApply('eyepiece', localSettings.eyepiece)"
          />
          <span style="margin: 0 4px">x</span>
          <input
              v-model.number="localSettings.eyepiece.screen_resolution_y"
              type="number"
              min="1"
              step="1"
              style="
              width: 70px;
              background: var(--surface);
              color: var(--text-primary);
              border: 1px solid var(--border);
              border-radius: 4px;
              padding: 4px;
            "
              title="Resolution Y"
              @change="debouncedApply('eyepiece', localSettings.eyepiece)"
          />
        </div>
      </div>
    </div>

    <!-- Stacking settings -->
    <div v-if="localSettings.stacking" class="settings-section">
      <h3 class="section-title">Stacking</h3>

      <div class="control-group">
        <div class="control-row">
          <label class="control-label" style="margin-bottom: 0; flex: 1">
            Frame Weighting
            <BaseInfoIcon :message="HELP.weighting_preset"/>
          </label>
          <select
              id="weighting-preset-select"
              v-model="localSettings.weighting_preset"
              class="select"
              style="width: 120px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
              @change="applySetting('weighting_preset', $event.target.value)"
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
            <BaseProLock
                v-if="!capabilities.deep_sky.advanced_rejection"
                feature="Advanced Rejection"
            />
            <BaseInfoIcon :message="HELP.rejection_method"/>
          </label>
          <select
              id="rejection-method-select"
              v-model="localSettings.rejection_method"
              class="select"
              style="width: 150px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
              @change="applySetting('rejection_method', $event.target.value)"
          >
            <option
                v-for="opt in REJECTION_METHOD_OPTIONS"
                :key="opt.value"
                :value="opt.value"
                :disabled="opt.pro && !capabilities.deep_sky.advanced_rejection"
            >
              {{ opt.label }} {{ opt.pro && !capabilities.deep_sky.advanced_rejection ? '🔒' : '' }}
            </option>
          </select>
        </div>
      </div>

      <BaseSlider
          v-if="localSettings.rejection_method !== 'None'"
          v-model="localSettings.rejection_sigma"
          label="Rejection Sigma"
          :min="0.5"
          :max="10.0"
          :step="0.1"
          :disabled="!capabilities.deep_sky.advanced_rejection"
          :help="HELP.rejection_sigma"
          @change="debouncedApply('rejection_sigma', localSettings.rejection_sigma)"
      >
        <template #label-extra>
          <BaseProLock
              v-if="!capabilities.deep_sky.advanced_rejection"
              feature="Advanced Rejection"
          />
        </template>
      </BaseSlider>
    </div>

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

      <div class="control-group">
        <BaseToggle
            v-model="simulatorEnabled"
            label="Simulated Camera"
            :help="HELP.simulated_camera"
            @update:model-value="applySetting('use_simulated_camera', $event)"
        />
      </div>
      <div v-if="simulatorEnabled" class="control-group" style="margin-top: 0.5rem">
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
    </div>
  </BasePanel>
</template>

<style scoped>
/* Uses global .section-title, .control-group, .control-label, .hint from main.css */

.settings-section {
  margin-bottom: 0.625rem;
}
</style>
