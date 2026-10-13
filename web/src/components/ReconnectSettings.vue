<script setup lang="ts">
/**
 * Settings → "If the camera drops out": reconnect, then resume the capture that was
 * running. Mirrors both and emits `apply(key, value)`.
 */
import {BaseToggle} from './ui'
import {useSectionMirror} from '../composables/useSectionMirror.js'
import {HELP_TEXTS} from '../constants'
import type {SettingApply} from '../utils/settings.js'
import type {Settings} from '../composables/api.types.js'

const props = defineProps<{
  /** The panel's settings mirror (`settingsMirror`). */
  settings: Settings
}>()

const emit = defineEmits<{apply: SettingApply}>()

const HELP = HELP_TEXTS

const local = useSectionMirror(() => props.settings, ['auto_reconnect', 'auto_resume_capture'])
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">If the camera drops out</h3>

    <div class="control-group">
      <BaseToggle
          v-model="local.auto_reconnect"
          label="Reconnect automatically"
          :help="HELP.auto_reconnect"
          @update:model-value="emit('apply', 'auto_reconnect', $event)"
      />
    </div>

    <div v-if="local.auto_reconnect" class="control-group">
      <BaseToggle
          v-model="local.auto_resume_capture"
          label="Resume the capture"
          :help="HELP.auto_resume_capture"
          @update:model-value="emit('apply', 'auto_resume_capture', $event)"
      />
    </div>
  </div>
</template>
