<script setup>
defineProps({
  progressText: {
    type: String,
    default: '',
  },
  progressPercent: {
    type: Number,
    default: null,
  },
  overallProgressText: {
    type: String,
    default: '',
  },
  hint: {
    type: String,
    default: '',
  },
})
</script>

<template>
  <div class="install-progress">
    <slot name="before"/>

    <div class="progress-icon">
      <div class="spinner"></div>
    </div>

    <p v-if="progressText" class="progress-text">{{ progressText }}</p>

    <div v-if="progressPercent !== null" class="progress-bar">
      <div class="progress-fill" :style="{ width: progressPercent + '%' }"></div>
    </div>

    <p v-if="overallProgressText" class="overall-progress-text">{{ overallProgressText }}</p>

    <p v-if="hint" class="progress-hint">{{ hint }}</p>

    <slot name="after"/>
  </div>
</template>

<style scoped>
.install-progress {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
  width: 100%;
}

.progress-icon {
  width: 64px;
  height: 64px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.spinner {
  width: 48px;
  height: 48px;
  border: 3px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.progress-text {
  font-size: 0.9rem;
  color: var(--text-primary);
  margin: 0;
  text-align: center;
}

.progress-bar {
  width: 100%;
  max-width: 300px;
  height: 8px;
  background: var(--surface-elevated);
  border-radius: 4px;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: var(--primary);
  transition: width 0.3s ease;
}

.overall-progress-text {
  font-size: 0.8rem;
  color: var(--text-secondary);
  margin: 0.5rem 0 0;
  font-weight: 500;
}

.progress-hint {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin: 0;
  text-align: center;
}
</style>
