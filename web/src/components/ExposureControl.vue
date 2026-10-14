<script setup lang="ts">
/**
 * Exposure: a number, its unit, and the unit's presets. `v-model` is the exposure as
 * typed, in microseconds, kept current on every keystroke — Start sends it even if the
 * field was never blurred. `apply(us)` is a commit: a change, or a preset.
 *
 * A new value from outside (the server, another camera selected) is shown in its own
 * unit, unless it is what the field already reads: 0.5 s must not flip to 500 ms. A field
 * holding no exposure (emptied, 0) leaves the model alone and reads it again on change.
 */
import {computed, ref, watch} from 'vue'
import {BaseInfoIcon} from './ui'
import {splitExposure, toMicros} from '../utils/exposure'
import type {ExposureUnit} from '../utils/exposure'
import {EXPOSURE_PRESETS, HELP_TEXTS} from '../constants'

const model = defineModel<number>({required: true})

withDefaults(defineProps<{
  /** The exposure the camera has now, whose preset is highlighted. */
  activeUs?: number | null
  disabled?: boolean
}>(), {
  activeUs: null,
  disabled: false,
})

const emit = defineEmits<{apply: [us: number]}>()

/** As typed: an emptied field holds `''`. */
const value = ref<number | ''>(0)
const unit = ref<ExposureUnit>('ms')

watch(model, (us) => {
  if (toMicros(value.value, unit.value) === us) return
  ;({value: value.value, unit: unit.value} = splitExposure(us))
}, {immediate: true})

watch([value, unit], () => {
  const us = toMicros(value.value, unit.value)
  if (us !== null) model.value = us
})

const presets = computed(() => EXPOSURE_PRESETS[unit.value])

function commit() {
  const us = toMicros(value.value, unit.value)
  if (us === null) {
    ;({value: value.value, unit: unit.value} = splitExposure(model.value))
    return
  }
  emit('apply', us)
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
