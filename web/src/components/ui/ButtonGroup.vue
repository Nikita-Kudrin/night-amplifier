<script setup lang="ts" generic="T extends string | number">
import type {Option} from '../../constants/tables'
import { useScrollProtect } from '../../composables/useScrollProtect'

withDefaults(defineProps<{
  modelValue?: NoInfer<T> | null
  options: readonly Option<T>[]
  disabled?: boolean
}>(), {
  modelValue: null,
  disabled: false
})

const emit = defineEmits<{'update:modelValue': [value: T]}>()

const { blockIfScrolling } = useScrollProtect()

function handleClick(event: Event, value: T) {
  if (blockIfScrolling(event)) return
  emit('update:modelValue', value)
}
</script>

<template>
  <div class="button-group">
    <button
        v-for="option in options"
        :key="option.value"
        class="btn btn-option"
        :class="{ active: modelValue === option.value }"
        :disabled="disabled"
        @click="handleClick($event, option.value)"
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
  font-size: var(--fs-xs);
  background: var(--surface-elevated);
  border: 1px solid var(--border);
}

.btn-option:hover:not(:disabled) {
  background: var(--surface-hover);
}

.btn-option.active {
  background: var(--primary);
  border-color: var(--primary);
  color: var(--on-primary);
}

.btn-option:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
