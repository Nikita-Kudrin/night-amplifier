<script setup>
import BaseInfoIcon from './BaseInfoIcon.vue'

defineProps({
  modelValue: {
    type: Number,
    required: true,
  },
  label: {
    type: String,
    default: ''
  },
  min: {
    type: Number,
    default: 0,
  },
  max: {
    type: Number,
    default: 100,
  },
  step: {
    type: Number,
    default: 1,
  },
  showInput: {
    type: Boolean,
    default: false,
  },
  formatValue: {
    type: Function,
    default: (v) => v,
  },
  help: {
    type: String,
    default: ''
  },
  disabled: {
    type: Boolean,
    default: false
  },
})

const emit = defineEmits(['update:modelValue', 'change'])

function handleInput(event) {
  const value = Number(event.target.value)
  emit('update:modelValue', value)
}

function handleChange(event) {
  const value = Number(event.target.value)
  emit('change', value)
}
</script>

<template>
  <div class="slider-control">
    <label v-if="label" class="control-label">
      <span class="label-text-wrapper">
        <slot name="label-left"></slot>
        {{ label }}
        <BaseInfoIcon v-if="help" :message="help"/>
        <slot name="label-extra"></slot>
      </span>
      <span class="current-value">{{ formatValue(modelValue) }}</span>
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
  font-size: 0.77rem;
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
</style>
