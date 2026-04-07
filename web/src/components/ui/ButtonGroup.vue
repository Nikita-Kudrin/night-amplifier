<script setup>
defineProps({
  modelValue: [String, Number],
  options: {
    type: Array,
    required: true,
  },
  disabled: Boolean,
})

const emit = defineEmits(['update:modelValue'])
</script>

<template>
  <div class="button-group">
    <button
        v-for="option in options"
        :key="option.value"
        class="btn btn-option"
        :class="{ active: modelValue === option.value }"
        :disabled="disabled"
        @click="emit('update:modelValue', option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

<style scoped>
.button-group {
  display: flex;
  gap: 0.25rem;
}

.btn-option {
  flex: 1;
  padding: 0.25rem;
  font-size: 0.7rem;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
}

.btn-option:hover:not(:disabled) {
  background: var(--surface-hover);
}

.btn-option.active {
  background: var(--primary);
  border-color: var(--primary);
  color: white;
}

.btn-option:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
