<script setup>
/**
 * Settings: one section component per group, each mirroring its keys of `localSettings`
 * and emitting `apply(key, value)`. The panel owns the one save path and error banner.
 */
import {ref, inject, watch, computed} from 'vue'
import {updateSettings} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {useCapabilities} from '../composables/appContext.js'
import {settingsMirror} from '../utils/settings.js'
import {STACKING_TYPES} from '../constants'
import {BasePanel, BaseAlert} from './ui'
import AdvancedSettings from './AdvancedSettings.vue'
import CoolerControl from './CoolerControl.vue'
import DewHeaterControl from './DewHeaterControl.vue'
import EyepieceSettings from './EyepieceSettings.vue'
import ImageQualitySettings from './ImageQualitySettings.vue'
import ProcessingSettings from './ProcessingSettings.vue'
import ReconnectSettings from './ReconnectSettings.vue'
import StackingSettings from './StackingSettings.vue'
import StorageSettings from './StorageSettings.vue'

const settings = inject('settings')
const refreshSettings = inject('refreshSettings')
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

/** Chosen on the capture panel; the server omits it only before its first answer. */
const stackingType = computed(() => settings?.value?.stacking_type ?? STACKING_TYPES.DEEP_SKY)

const localSettings = ref(settingsMirror(null))

watch(
    settings,
    (newSettings) => {
      if (newSettings) localSettings.value = settingsMirror(newSettings)
    },
    {immediate: true}
)

async function applySetting(key, value) {
  await withErrorHandling(async () => {
    await updateSettings({[key]: value})
    await refreshSettings()
  })
}

/** Persist a setting a section edited, keeping the panel's own copy in step. */
function applyGroup(key, value) {
  localSettings.value[key] = value
  return applySetting(key, value)
}
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

    <ProcessingSettings
        :settings="localSettings"
        :focus-mode="focusMode"
        :rbf-background-available="rbfBackground"
        :saturation-boost-available="saturationBoost"
        @apply="applyGroup"
    />

    <StorageSettings :settings="localSettings" :has-guide-camera="hasGuideCamera" @apply="applyGroup"/>

    <ReconnectSettings :settings="localSettings" @apply="applyGroup"/>

    <EyepieceSettings :eyepiece="localSettings.eyepiece" @apply="applyGroup"/>

    <StackingSettings
        v-if="localSettings.stacking"
        :weighting-preset="localSettings.weighting_preset"
        :rejection-method="localSettings.rejection_method"
        :rejection-sigma="localSettings.rejection_sigma"
        :planetary-auto-tracking="localSettings.planetary_auto_tracking"
        :planetary-multi-point-alignment="localSettings.planetary_multi_point_alignment"
        :planetary="stackingType === STACKING_TYPES.PLANETARY"
        :auto-crop="localSettings.auto_crop"
        :deep-sky="stackingType === STACKING_TYPES.DEEP_SKY"
        :advanced-rejection-available="advancedRejection"
        :advanced-planetary-available="advancedPlanetary"
        @apply="applyGroup"
    />

    <AdvancedSettings
        :settings="localSettings"
        :focus-mode="focusMode"
        :ai-denoise-available="aiDenoiseAvailable"
        :save="applyGroup"
        :guard="withErrorHandling"
        @apply="applyGroup"
    />
  </BasePanel>
</template>
