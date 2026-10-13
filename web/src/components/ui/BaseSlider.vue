<script setup lang="ts">
import BaseInfoIcon from './BaseInfoIcon.vue'

const props = withDefaults(defineProps<{
  modelValue: number
  label?: string
  min?: number
  max?: number
  step?: number
  /**
   * Show a number input beside the slider. It replaces the read-only value in
   * the label rather than joining it — two copies of the same number a few
   * pixels apart read as two different settings.
   */
  showInput?: boolean
  formatValue?: (value: number) => string | number
  help?: string
  disabled?: boolean
  largeGap?: boolean
}>(), {
  label: '',
  min: 0,
  max: 100,
  step: 1,
  showInput: false,
  formatValue: (v: number) => v,
  help: '',
  disabled: false,
  largeGap: false,
})

const emit = defineEmits<{
  'update:modelValue': [value: number]
  change: [value: number]
}>()

import { useScrollProtect } from '../../composables/useScrollProtect.js'

const { blockIfScrolling } = useScrollProtect()

/** The value the number box held when focused: what an emptied one goes back to. */
let valueAtFocus: number | null = null

function rememberValueAtFocus() {
  valueAtFocus = props.modelValue
}

/** `null` while the number box holds no number: `Number('')` is 0, which an emptied Gain committed. */
function readNumber(event: Event) {
  const raw = (event.target as HTMLInputElement).value
  const value = raw === '' ? NaN : Number(raw)
  return Number.isFinite(value) ? value : null
}

function handleInput(event: Event) {
  if (blockIfScrolling(event, props.modelValue)) return
  const value = readNumber(event)
  if (value !== null) emit('update:modelValue', value)
}

function handleChange(event: Event) {
  if (blockIfScrolling(event, props.modelValue)) return
  const value = readNumber(event)
  if (value !== null) {
    valueAtFocus = value
    emit('change', value)
    return
  }
  const previous = valueAtFocus ?? props.modelValue
  const input = event.target as HTMLInputElement
  input.value = String(previous)
  if (previous !== props.modelValue) emit('update:modelValue', previous)
}
</script>

<template>
  <div class="slider-control" :class="{'large-gap': largeGap}">
    <label v-if="label" class="control-label">
      <span class="label-text-wrapper">
        <slot name="label-left"></slot>
        {{ label }}
        <BaseInfoIcon v-if="help" :message="help"/>
        <slot name="label-extra"></slot>
      </span>
      <span v-if="!showInput" class="current-value">{{ formatValue(modelValue) }}</span>
    </label>
    <div class="slider-group">
      <input
          type="range"
          :value="modelValue"
          :min="min"
          :max="max"
          :step="step"
          :disabled="disabled"
          class="slider"
          @input="handleInput"
          @change="handleChange"
      />
      <input
          v-if="showInput"
          type="number"
          :value="modelValue"
          :min="min"
          :max="max"
          :step="step"
          :disabled="disabled"
          class="input input-sm"
          @focus="rememberValueAtFocus"
          @input="handleInput"
          @change="handleChange"
      />
    </div>
  </div>
</template>

<style scoped>
/* Uses global .control-label, .current-value, .slider-group from main.css */

.slider-control {
  margin-bottom: 0.375rem;
}

.current-value {
  font-size: var(--fs-sm);
}

.label-text-wrapper {
  display: flex;
  align-items: center;
  gap: 0.375rem;
}

.slider:disabled, .input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.slider-control.large-gap .control-label {
  margin-bottom: 0.5rem;
}
</style>
