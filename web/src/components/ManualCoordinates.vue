<script setup>
/**
 * Push-To's manual target: RA/Dec typed as sexagesimal or degrees, behind a switch.
 *
 * `submit(coords)` sets the target and resolves `true` once it took; only then are the
 * fields cleared, so a refused target keeps what was typed.
 */
import {ref} from 'vue'
import {useCoordinateInput} from '../composables/useCoordinates.js'
import {BaseToggle} from './ui'

const props = defineProps({
  /** `({ra, dec}) => Promise<boolean>`. */
  submit: {type: Function, required: true},
})

const enabled = ref(false)
const {raInput, decInput, coordError, validateCoordinates, clearInputs} = useCoordinateInput()

async function setTarget() {
  const coords = validateCoordinates()
  if (!coords) return
  if (await props.submit(coords)) clearInputs()
}
</script>

<template>
  <div class="section manual-coords-section">
    <div class="manual-coords-header">
      <h3 class="section-title">Manual Coordinates</h3>
      <BaseToggle :model-value="enabled" size="small" @update:model-value="enabled = $event"/>
    </div>
    <div v-if="enabled" class="manual-coords-content">
      <div class="coord-inputs">
        <div class="coord-field">
          <label>RA</label>
          <input
              v-model="raInput"
              type="text"
              placeholder="HH:MM:SS or degrees"
              class="coord-input"
          />
        </div>
        <div class="coord-field">
          <label>Dec</label>
          <input
              v-model="decInput"
              type="text"
              placeholder="DD:MM:SS or degrees"
              class="coord-input"
          />
        </div>
      </div>
      <div v-if="coordError" class="coord-error">{{ coordError }}</div>
      <button
          class="btn btn-sm btn-primary set-coords-btn"
          :disabled="!raInput || !decInput"
          @click="setTarget"
      >
        Set Target
      </button>
    </div>
  </div>
</template>

<style scoped>
.section-title {
  font-size: 0.7rem;
  color: var(--text-muted);
  text-transform: uppercase;
  margin-bottom: 0.375rem;
  padding-bottom: 0;
  border-bottom: none;
}

.manual-coords-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.375rem;
}

.manual-coords-header .section-title {
  margin-bottom: 0;
}

.manual-coords-content {
  margin-top: 0.375rem;
}

.coord-inputs {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 0.375rem;
}

.coord-field {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
}

.coord-field label {
  font-size: 0.65rem;
  color: var(--text-muted);
}

.coord-input {
  width: 100%;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.375rem;
  font-size: 0.75rem;
  color: var(--text-primary);
  font-family: monospace;
}

.coord-input:focus {
  outline: none;
  border-color: var(--primary);
}

.coord-input::placeholder {
  color: var(--text-muted);
  font-family: inherit;
}

.coord-error {
  font-size: 0.65rem;
  color: var(--danger);
  margin-bottom: 0.25rem;
}

.set_coords-btn {
  width: 100%;
}
</style>
