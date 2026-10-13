<script setup>
import BaseSpinner from './BaseSpinner.vue'

/**
 * Progress of a long job: a line of text over a bar. Full size in a dialog (large spinner
 * above, overall line and hint below); `compact` inside a setup section, the spinner
 * inline with the text. `spinner: false` leaves it to a host that shows its own.
 */
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
  compact: {
    type: Boolean,
    default: false,
  },
  spinner: {
    type: Boolean,
    default: true,
  },
})
</script>

<template>
  <div class="install-progress" :class="{ compact }">
    <slot name="before"/>

    <div v-if="spinner && !compact" class="progress-icon">
      <BaseSpinner size="lg" />
    </div>

    <p v-if="progressText || (compact && spinner)" class="progress-text">
      <BaseSpinner v-if="compact && spinner" size="sm" />
      <span>{{ progressText }}</span>
    </p>

    <div
        v-if="progressPercent !== null"
        class="progress-bar"
        :class="{ 'progress-bar-lg': !compact }"
        role="progressbar"
        aria-valuemin="0"
        aria-valuemax="100"
        :aria-valuenow="Math.round(progressPercent)"
    >
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

.progress-text {
  font-size: var(--fs-base);
  color: var(--text-primary);
  margin: 0;
  text-align: center;
}

.install-progress:not(.compact) .progress-bar {
  max-width: 300px;
}

.install-progress.compact {
  align-items: stretch;
  gap: 0.5rem;
}

.compact .progress-text {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: var(--fs-md);
  color: var(--text-secondary);
  text-align: left;
}

.overall-progress-text {
  font-size: var(--fs-md);
  color: var(--text-secondary);
  margin: 0.5rem 0 0;
  font-weight: 500;
}

.progress-hint {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  margin: 0;
  text-align: center;
}
</style>
