<script setup lang="ts">
/**
 * Settings → Storage: raw frames are saved per capture mode, the stack only in Stacking.
 *
 * Mirrors `raw_frame_saving` and `save_stacked_image` and emits `apply(key, value)`; a
 * raw-frame switch sends the whole group, so the server never sees a partial selection.
 */
import {computed} from 'vue'
import {BaseToggle, BaseInfoIcon} from './ui'
import {useSectionMirror} from '../composables/useSectionMirror'
import {HELP_TEXTS, RAW_FRAME_SAVING_MODES} from '../constants'
import type {SettingApply} from '../utils/settings'
import type {RawFrameSaving, Settings} from '../composables/api.types'

const props = withDefaults(defineProps<{
  /** The panel's settings mirror (`settingsMirror`). */
  settings: Settings
  /** The guide switch is only meaningful with a guide camera attached. */
  hasGuideCamera?: boolean
}>(), {
  hasGuideCamera: false,
})

const emit = defineEmits<{apply: SettingApply}>()

const HELP = HELP_TEXTS

const local = useSectionMirror(() => props.settings, ['raw_frame_saving', 'save_stacked_image'])

const modes = computed(() =>
    RAW_FRAME_SAVING_MODES.filter((mode) => !mode.requiresGuideCamera || props.hasGuideCamera)
)

/** The stack is saved only by Stacking itself; Wanderer restarts it and saves none. */
const savesStack = computed(() => props.settings.stacking && !props.settings.wanderer_mode)

function applyRawFrameSaving(mode: keyof RawFrameSaving, enabled: boolean) {
  emit('apply', 'raw_frame_saving', {...local.raw_frame_saving, [mode]: enabled})
}
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Storage</h3>

    <div class="control-group">
      <span class="storage-group-label">
        Save Raw Frames
        <BaseInfoIcon :message="HELP.raw_frame_saving"/>
      </span>

      <BaseToggle
          v-for="mode in modes"
          :key="mode.key"
          v-model="local.raw_frame_saving[mode.key]"
          :label="mode.label"
          class="storage-mode-toggle"
          @update:model-value="applyRawFrameSaving(mode.key, $event)"
      />
    </div>

    <div v-if="savesStack" class="control-group">
      <BaseToggle
          v-model="local.save_stacked_image"
          label="Save Stacked Image"
          :help="HELP.save_stacked_image"
          @update:model-value="emit('apply', 'save_stacked_image', $event)"
      />
    </div>
  </div>
</template>

<style scoped>
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
