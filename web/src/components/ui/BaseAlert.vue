<script setup>
defineProps({
  type: {
    type: String,
    default: 'error',
    validator: (v) => ['error', 'warning', 'info', 'success'].includes(v),
  },
  dismissible: {
    type: Boolean,
    default: true,
  },
  /** Dialog size: roomier, with a rule in its own colour. */
  bordered: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['dismiss'])
</script>

<template>
  <div class="alert" :class="[`alert-${type}`, {bordered}]">
    <slot/>
    <button v-if="dismissible" class="btn-close" @click="emit('dismiss')">&times;</button>
  </div>
</template>

<style scoped>
/* Uses global .btn-close from main.css */

.alert {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.375rem 0.5rem;
  border-radius: var(--radius-sm);
  margin-bottom: 0.375rem;
  font-size: var(--fs-sm);
  gap: 0.5rem;
}

.alert-error {
  background: var(--error-bg);
  color: var(--error);
}

.alert-warning {
  background: var(--warning-bg);
  color: var(--warning);
}

.alert-info {
  background: var(--primary-bg);
  color: var(--primary);
}

.alert-success {
  background: var(--success-bg);
  color: var(--success);
}

.alert.bordered {
  padding: 0.75rem 1rem;
  margin-bottom: 0;
  border: 1px solid color-mix(in srgb, currentColor 30%, transparent);
  border-radius: var(--radius-md);
  font-size: var(--fs-base);
}
</style>
