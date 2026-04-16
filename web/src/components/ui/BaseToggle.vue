<script setup>
import BaseInfoIcon from './BaseInfoIcon.vue'

defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  label: {
    type: String,
    default: ''
  },
  size: {
    type: String,
    default: 'normal',
    validator: (v) => ['small', 'normal'].includes(v),
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

const emit = defineEmits(['update:modelValue'])
</script>

<template>
  <label class="toggle-label" :class="{ 'toggle-small': size === 'small', 'toggle-disabled': disabled }">
    <input
        type="checkbox"
        :checked="modelValue"
        :disabled="disabled"
        class="toggle"
        @change="emit('update:modelValue', $event.target.checked)"
    />
    <span v-if="label" class="toggle-text">
      {{ label }}
      <BaseInfoIcon v-if="help" :message="help"/>
      <slot name="label-extra"></slot>
    </span>
    <slot v-else/>
  </label>
</template>

<style scoped>
.toggle-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
}

.toggle {
  width: 36px;
  height: 20px;
  appearance: none;
  background: var(--surface-elevated);
  border-radius: 10px;
  position: relative;
  cursor: pointer;
  transition: background 0.2s;
  flex-shrink: 0;
}

.toggle::before {
  content: '';
  position: absolute;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--text-muted);
  top: 3px;
  left: 3px;
  transition: transform 0.2s,
  background 0.2s;
}

.toggle:checked {
  background: var(--primary);
}

.toggle:checked::before {
  transform: translateX(16px);
  background: white;
}

.toggle-text {
  font-size: 0.825rem;
  color: var(--text-primary);
}

/* Small variant */
.toggle-small .toggle {
  width: 32px;
  height: 18px;
}

.toggle-small .toggle::before {
  width: 12px;
  height: 12px;
}

.toggle-small .toggle:checked::before {
  transform: translateX(14px);
}

.toggle-small .toggle-text {
  font-size: 0.715rem;
  font-weight: 500;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.toggle-disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.toggle-disabled .toggle {
  cursor: not-allowed;
}
</style>
