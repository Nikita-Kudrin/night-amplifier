<script setup lang="ts">
/**
 * One camera in the camera list: its name, a details line (the resolution unless the
 * `details` slot says more), its actions, and a remove button for a simulated camera.
 */
import type {Camera} from '../composables/api.types.js'

withDefaults(defineProps<{
  camera: Pick<Camera, 'name'>
  /** The resolution line, shown when the `details` slot is not used. */
  resolution?: string
  selected?: boolean
  /** Listed under Available: not connected, so not selectable. */
  available?: boolean
  removable?: boolean
  removeDisabled?: boolean
}>(), {
  resolution: '',
  selected: false,
  available: false,
  removable: false,
  removeDisabled: false,
})

const emit = defineEmits<{select: []; remove: []}>()
</script>

<template>
  <div class="camera-item" :class="{ selected, available }" @click="emit('select')">
    <button
        v-if="removable"
        class="btn btn-sm btn-icon"
        :disabled="removeDisabled"
        title="Remove simulated camera"
        @click.stop="emit('remove')"
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/>
      </svg>
    </button>
    <div class="camera-info">
      <span class="camera-name">{{ camera.name }}</span>
      <span class="camera-details">
        <slot name="details">{{ resolution }}</slot>
      </span>
    </div>
    <div class="camera-actions">
      <slot name="actions"/>
    </div>
  </div>
</template>

<style scoped>
.camera-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.375rem;
  padding: 0.375rem 0.5rem;
  background: var(--surface-elevated);
  border-radius: var(--radius-md);
  cursor: pointer;
  border: 1px solid transparent;
  transition: border-color var(--transition-fast),
  background var(--transition-fast);
  min-height: 2.25rem;
}

.camera-item:hover {
  background: var(--surface-hover);
}

.camera-item.selected {
  border-color: var(--primary);
}

.camera-item.available {
  cursor: default;
  opacity: 0.8;
}

.camera-info {
  display: flex;
  flex-direction: column;
  gap: 0;
  min-width: 0;
  flex: 1;
}

.camera-name {
  font-size: var(--fs-md);
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.camera-details {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  display: flex;
  align-items: center;
  gap: 0.375rem;
}

.camera-actions {
  display: flex;
  gap: 0.25rem;
  align-items: center;
}

.btn-icon {
  padding: 0.25rem;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: color var(--transition-fast),
  background var(--transition-fast),
  border-color var(--transition-fast);
}

.btn-icon:hover:not(:disabled) {
  color: var(--error);
  background: var(--surface-hover);
  border-color: var(--error);
}

.btn-icon:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
