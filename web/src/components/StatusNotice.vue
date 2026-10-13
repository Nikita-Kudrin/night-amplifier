<script setup>
/**
 * One chip in the status bar: an icon, one line of text, tinted by its tone. A
 * dismissible one clears on a click anywhere on it, and shows a × to say so.
 */
import StatusIcon from './ui/StatusIcon.vue'

defineProps({
  /** `success`, `warning`, `error` or `muted`. */
  tone: {type: String, default: 'muted'},
  icon: {type: String, default: ''},
  text: {type: String, required: true},
  dismissible: {type: Boolean, default: false},
})

const emit = defineEmits(['dismiss'])
</script>

<template>
  <div
      class="status-item status-chip"
      :class="[`tone-${tone}`, {dismissible}]"
      @click="dismissible && emit('dismiss')"
  >
    <slot name="icon">
      <StatusIcon v-if="icon" :name="icon"/>
    </slot>
    <span class="status-chip-text">{{ text }}</span>
    <button v-if="dismissible" class="btn-close btn-dismiss" aria-label="Dismiss">&times;</button>
  </div>
</template>

<style scoped>
.status-chip {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  white-space: nowrap;
  padding: 0.25rem 0.5rem;
  border-radius: var(--radius-sm);
  max-width: 300px;
  color: var(--chip-fg);
  background: var(--chip-bg);
}

.status-chip-text {
  overflow: hidden;
  text-overflow: ellipsis;
}

.dismissible {
  cursor: pointer;
}

.tone-success {
  --chip-fg: var(--success);
  --chip-bg: var(--success-bg);
}

.tone-warning {
  --chip-fg: var(--warning);
  --chip-bg: var(--warning-bg);
}

.tone-error {
  --chip-fg: var(--error);
  --chip-bg: var(--error-bg);
}

/* A cancel or a blocker is not a fault: muted, not red. */
.tone-muted {
  --chip-fg: var(--text-secondary);
  --chip-bg: var(--surface-hover);
}

.tone-plain {
  --chip-fg: var(--text-secondary);
  --chip-bg: transparent;
}

.tone-primary {
  --chip-fg: var(--primary);
  --chip-bg: transparent;
}

.btn-dismiss {
  opacity: 0.7;
}

.btn-dismiss:hover {
  opacity: 1;
}
</style>
