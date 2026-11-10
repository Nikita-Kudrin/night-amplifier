<script setup lang="ts">
import type {PlateSolving} from '../composables/events/pushToEvents'
import {CompassIcon} from './ui'
import SolveProgressBar from './SolveProgressBar.vue'

withDefaults(defineProps<{
  /** The step a multi-strategy solve is on; the bar tells a slow solve from a hung one. */
  stage?: PlateSolving['stage']
}>(), {
  stage: null,
})

defineSlots<{
  /** The readout the chip stands in for, laid out unseen so the chip keeps its size and the bar spans it. */
  default?: () => unknown
}>()
</script>

<template>
  <div class="hud-chip push-to-solving" role="status" aria-label="Plate solving">
    <div v-if="$slots.default" class="stand-in" aria-hidden="true">
      <slot/>
    </div>
    <div class="solving">
      <CompassIcon spinning class="solving-compass"/>
      <!-- Always laid out, so the compass does not jump when the first step arrives. -->
      <div class="progress-row">
        <SolveProgressBar :stage="stage"/>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* One cell for both layers: the chip is the larger of the readout and the compass. */
.push-to-solving {
  display: grid;
  padding: 0.45rem 0.9rem;
  opacity: 0.8;
  white-space: nowrap;
}

.stand-in,
.solving {
  grid-area: 1 / 1;
}

.stand-in {
  visibility: hidden;
}

.solving {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.3rem;
}

.solving-compass {
  width: 2.5rem;
  height: 2.5rem;
  color: var(--primary);
}

/* Stretched to the cell: the readout's width, else the compass's. */
.progress-row {
  align-self: stretch;
  height: 4px;
}
</style>
