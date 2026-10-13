<script setup>
/**
 * The box that adds a simulated camera: a directory of FITS, TIFF or PNG files. Emits
 * `submit(path)`; the panel configures the simulator and closes the box once it took.
 */
import {ref} from 'vue'
import {BaseInfoIcon} from './ui'

defineProps({
  /** The panel is configuring the simulator. */
  busy: {type: Boolean, default: false},
})

const emit = defineEmits(['submit', 'close'])

const HELP_DIRECTORY = 'The local path where the simulator looks for source images (FITS, TIFF, or PNG).'

const directoryPath = ref('')

function submit() {
  emit('submit', directoryPath.value.trim())
}
</script>

<template>
  <div class="simulator-config">
    <div class="config-header">
      <span>
        Configure Simulator Directory
        <BaseInfoIcon :message="HELP_DIRECTORY"/>
      </span>
      <button class="btn-close" @click="emit('close')">&times;</button>
    </div>
    <div class="config-body">
      <input
          v-model="directoryPath"
          type="text"
          placeholder="Enter path to image directory..."
          class="input input-dense directory-input"
          @keyup.enter="submit"
      />
      <button
          class="btn btn-sm btn-primary"
          :disabled="busy"
          @click="submit"
      >
        {{ busy ? '...' : 'Set' }}
      </button>
    </div>
    <div class="config-hint">
      Enter the full path to a directory containing FITS, TIFF, or PNG files
    </div>
  </div>
</template>

<style scoped>
.simulator-config {
  background: var(--surface-elevated);
  border-radius: var(--radius-md);
  padding: 0.5rem;
  margin-bottom: 0.375rem;
}

.config-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text-primary);
  margin-bottom: 0.375rem;
}

.config-body {
  display: flex;
  gap: 0.375rem;
  align-items: center;
}

.directory-input {
  flex: 1;
  min-width: 0;
  background: var(--surface);
}

.config-hint {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  margin-top: 0.25rem;
}
</style>
