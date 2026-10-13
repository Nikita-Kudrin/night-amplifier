<script setup>
/**
 * Settings: one section component per group, each mirroring its keys of `localSettings`
 * and emitting `apply(key, value)`. The panel owns the one save path and error banner, and
 * is the one writer of the `eyepiece` group (`applyEyepiece`).
 */
import {ref, watch, computed} from 'vue'
import {useError} from '../composables/useError.js'
import {useApplySettings, useCapabilities, useHasGuideCamera, useSettings} from '../composables/appContext.js'
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

const settings = useSettings()
const applySettings = useApplySettings()
const hasGuideCamera = useHasGuideCamera()
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

/** `eyepiece` fields posted and not yet answered (see `applyEyepiece`). */
let eyepieceInFlight = {}
let eyepieceWrites = 0

watch(
    settings,
    (newSettings) => {
      if (!newSettings) return
      const mirror = settingsMirror(newSettings)
      Object.assign(mirror.eyepiece, eyepieceInFlight)
      localSettings.value = mirror
    },
    {immediate: true}
)

async function applySetting(key, value) {
  await withErrorHandling(async () => {
    await applySettings({[key]: value})
  })
}

/**
 * The server replaces the `eyepiece` group whole, and two sections edit it (Eyepiece its
 * layout and screen, Processing its tone). Each hands over only the fields it changed, sent
 * over the panel's copy: a section's own stale copy reverted the other's field. Fields still
 * in flight are laid over any answer adopted meanwhile, which an earlier write may predate.
 */
function applyEyepiece(fields) {
  const eyepiece = {...localSettings.value.eyepiece, ...fields}
  localSettings.value.eyepiece = eyepiece
  Object.assign(eyepieceInFlight, fields)
  eyepieceWrites++
  return applySetting('eyepiece', {...eyepiece}).finally(() => {
    if (--eyepieceWrites === 0) eyepieceInFlight = {}
  })
}

/** Persist a setting a section edited, keeping the panel's own copy in step. */
function applyGroup(key, value) {
  if (key === 'eyepiece') return applyEyepiece(value)
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
