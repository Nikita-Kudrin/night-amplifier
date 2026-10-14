<script setup lang="ts">
/**
 * Names the camera a section edits: the one selected on the Main tab, which the Push-To
 * and Settings tabs do not show. Without it a guide rig's cooler was set from Settings
 * believing it was the imaging camera's.
 *
 * `fallbackName` is for a section that edits a camera with none selected (Equipment
 * edits the last camera's profile): naming nobody let the imaging scope's focal length
 * land in the guide's profile before either was connected.
 */
import {computed} from 'vue'
import {roleLabel} from '../composables/useCameraBadges'
import {useSelectedCamera} from '../composables/appContext'

const props = withDefaults(defineProps<{
  fallbackName?: string | null
}>(), {
  fallbackName: null,
})

const camera = useSelectedCamera()
const role = computed(() => roleLabel(camera.value))
</script>

<template>
  <span v-if="camera" class="camera-tag" data-test="selected-camera-tag">
    <template v-if="role">{{ role }} · </template>{{ camera.name }}
  </span>
  <span v-else-if="props.fallbackName" class="camera-tag" data-test="selected-camera-tag">
    {{ props.fallbackName }} · not connected
  </span>
</template>

<style scoped>
.camera-tag {
  margin-left: 0.25rem;
  font-weight: 400;
  text-transform: none;
  color: var(--text-primary);
}
</style>
