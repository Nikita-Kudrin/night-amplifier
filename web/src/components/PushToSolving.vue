<script setup lang="ts">
import {computed} from 'vue'
import type {PlateSolving} from '../composables/events/pushToEvents'
import {CompassIcon} from './ui'

const props = withDefaults(defineProps<{
  /** The step a multi-strategy solve is on; the counter tells a slow solve from a hung one. */
  stage?: PlateSolving['stage']
}>(), {
  stage: null,
})

const step = computed(() => {
  const {stage} = props
  return stage && stage.total > 1 ? `step ${stage.attempt}/${stage.total}` : null
})
</script>

<template>
  <div class="hud-chip push-to-solving" role="status" aria-label="Plate solving">
    <CompassIcon spinning class="solving-compass"/>
    <span v-if="step" class="step">{{ step }}</span>
  </div>
</template>

<style scoped>
.push-to-solving {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.15rem;
  padding: 0.45rem 0.9rem;
  opacity: 0.8;
  white-space: nowrap;
}

.solving-compass {
  width: 2.5rem;
  height: 2.5rem;
  color: var(--primary);
}

.step {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  font-family: var(--font-mono);
}
</style>
