<script setup>
import { onMounted, onUnmounted } from 'vue'

const emit = defineEmits(['close'])

const props = defineProps({
  title: {
    type: String,
    default: ''
  },
  maxWidth: {
    type: String,
    default: '600px'
  },
  noPadding: {
    type: Boolean,
    default: false
  },
  overlayClass: {
    type: String,
    default: ''
  },
  persistent: {
    type: Boolean,
    default: false
  },
  showCloseButton: {
    type: Boolean,
    default: true
  }
})

// Handle escape key
function handleKeydown(e) {
  if (e.key === 'Escape' && !props.persistent) {
    emit('close')
  }
}

onMounted(() => {
  document.addEventListener('keydown', handleKeydown)
})

onUnmounted(() => {
  document.removeEventListener('keydown', handleKeydown)
})
</script>

<template>
  <div class="base-modal-overlay" :class="overlayClass" @mousedown.self="!persistent && emit('close')">
    <div class="base-modal-panel" :style="{ maxWidth: maxWidth }" role="dialog" aria-modal="true">
      <div v-if="title || $slots.header" class="base-modal-header">
        <slot name="header">
          <h2 class="base-modal-title">{{ title }}</h2>
          <button v-if="showCloseButton" class="btn-close" title="Close" @click="emit('close')">&times;</button>
        </slot>
      </div>
      
      <div class="base-modal-body" :class="{ 'no-padding': noPadding }">
        <slot></slot>
      </div>
      
      <div v-if="$slots.footer" class="base-modal-footer">
        <slot name="footer"></slot>
      </div>
    </div>
  </div>
</template>

<style scoped>
.base-modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.7);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 1rem;
  animation: modal-fade-in 0.2s ease-out;
}

.base-modal-panel {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 12px;
  width: 100%;
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 25px -5px rgba(0, 0, 0, 0.5), 0 10px 10px -5px rgba(0, 0, 0, 0.3);
  animation: modal-slide-in 0.3s ease-out;
}

.base-modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 1.25rem 1.5rem;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
  border-radius: 12px 12px 0 0;
  flex-shrink: 0;
}

.base-modal-title {
  margin: 0;
  font-size: 1.15rem;
  font-weight: 600;
  color: var(--text-primary);
  display: flex;
  align-items: center;
}

.base-modal-body {
  padding: 1.25rem 1.5rem;
  overflow-y: auto;
  flex: 1;
}

.base-modal-body.no-padding {
  padding: 0;
}

.base-modal-footer {
  padding: 1rem 1.5rem;
  border-top: 1px solid var(--border);
  background: var(--surface);
  border-radius: 0 0 12px 12px;
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  flex-shrink: 0;
}

@keyframes modal-fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}

@keyframes modal-slide-in {
  from {
    opacity: 0;
    transform: translateY(16px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

/* Mobile */
@media (max-width: 768px) {
  .base-modal-panel {
    max-height: 95vh;
  }
  
  .base-modal-body {
    padding: 1rem;
  }
  
  .base-modal-body.no-padding {
    padding: 0;
  }
}
</style>
