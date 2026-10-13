<script setup>
/**
 * Exposure: a number, its unit, and the unit's presets. `v-model` is the exposure as
 * typed, in microseconds, kept current on every keystroke — Start sends it even if the
 * field was never blurred. `apply(us)` is a commit: a change, or a preset.
 *
 * A new value from outside (the server, another camera selected) is shown in its own
 * unit, unless it is what the field already reads: 0.5 s must not flip to 500 ms.
 */
import {computed, ref, watch} from 'vue'
import {BaseInfoIcon} from './ui'
import {splitExposure, toMicros} from '../utils/exposure.js'
import {EXPOSURE_PRESETS, HELP_TEXTS} from '../constants'

const model = defineModel({type: Number, required: true})

defineProps({
  /** The exposure the camera has now, whose preset is highlighted. */
  activeUs: {type: Number, default: null},
  disabled: {type: Boolean, default: false},
})

const emit = defineEmits(['apply'])

const value = ref(0)
const unit = ref('ms')

watch(model, (us) => {
  if (toMicros(value.value, unit.value) === us) return
  ;({value: value.value, unit: unit.value} = splitExposure(us))
}, {immediate: true})

watch([value, unit], () => {
  model.value = toMicros(value.value, unit.value)
})

const presets = computed(() => EXPOSURE_PRESETS[unit.value] || EXPOSURE_PRESETS.ms)

function commit() {
  emit('apply', toMicros(value.value, unit.value))
}
</script>

<template>
  <div class="control-row">
    <label class="type-label-inline">
      Exposure
      <BaseInfoIcon :message="HELP_TEXTS.exposure"/>
    </label>
    <div class="input-group">
      <input
          v-model.number="value"
          type="number"
          min="0.001"
          step="0.1"
          class="input"
          :disabled="disabled"
          @change="commit"
      />
      <select v-model="unit" class="select" :disabled="disabled" @change="commit">
        <option value="us">us</option>
        <option value="ms">ms</option>
        <option value="s">s</option>
      </select>
    </div>
  </div>
  <div class="presets">
    <button
        v-for="preset in presets"
        :key="preset.us"
        class="btn btn-preset"
        :class="{ active: activeUs === preset.us }"
        :disabled="disabled"
        @click="emit('apply', preset.us)"
    >
      {{ preset.label }}
    </button>
  </div>
</template>

<style scoped>
.control-row .input-group {
  flex: 1;
}

.presets {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem;
  margin-top: 0.25rem;
}

.btn-preset {
  padding: 0.125rem 0.375rem;
  font-size: var(--fs-2xs);
  background: var(--surface-elevated);
  border: 1px solid var(--border);
}

.btn-preset:hover {
  background: var(--surface-hover);
}

.btn-preset.active {
  background: var(--primary);
  border-color: var(--primary);
  color: var(--on-primary);
}
</style>
