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
          class="directory-input"
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
  border-radius: 6px;
  padding: 0.5rem;
  margin-bottom: 0.375rem;
}

.config-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 0.75rem;
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
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.375rem 0.5rem;
  font-size: 0.75rem;
  color: var(--text-primary);
  min-width: 0;
}

.directory-input:focus {
  outline: none;
  border-color: var(--primary);
}

.directory-input::placeholder {
  color: var(--text-muted);
}

.config-hint {
  font-size: 0.65rem;
  color: var(--text-muted);
  margin-top: 0.25rem;
}
</style>
