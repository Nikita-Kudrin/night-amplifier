<script setup>
defineProps({
  size: {
    type: String,
    default: 'md', // 'xs' (12px), 'sm' (16px), 'md' (32px), 'lg' (48px)
    validator: (value) => ['xs', 'sm', 'md', 'lg'].includes(value)
  },
  light: {
    type: Boolean,
    default: false // Use white border for dark buttons
  }
})
</script>

<template>
  <span class="base-spinner" :class="[`spinner-${size}`, { 'spinner-light': light }]"></span>
</template>

<style scoped>
.spinner-xs {
  width: 12px;
  height: 12px;
  border: 2px solid var(--border);
  border-top-color: var(--primary);
}

.base-spinner {
  display: inline-block;
  border-radius: 50%;
  animation: base-spin 1s linear infinite;
  flex-shrink: 0;
}

.spinner-sm {
  width: 16px;
  height: 16px;
  border: 2px solid var(--border);
  border-top-color: var(--primary);
}

.spinner-md {
  width: 32px;
  height: 32px;
  border: 3px solid var(--border);
  border-top-color: var(--primary);
}

.spinner-lg {
  width: 48px;
  height: 48px;
  border: 3px solid var(--border);
  border-top-color: var(--primary);
}

.spinner-light {
  border-color: rgba(255, 255, 255, 0.3);
  border-top-color: white;
}

@keyframes base-spin {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}
</style>
