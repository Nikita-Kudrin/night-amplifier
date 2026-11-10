<script setup lang="ts">
import {computed} from 'vue'
import {solveProgress} from '../composables/events/pushToEvents'
import type {PlateSolving} from '../composables/events/pushToEvents'

const props = withDefaults(defineProps<{
  /** The step a multi-strategy solve is on; nothing is drawn for a single-step one. */
  stage?: PlateSolving['stage']
}>(), {
  stage: null,
})

const progress = computed(() => solveProgress(props.stage))
</script>

<!--
  One segment per step, as wide as its container: the readout it stands in for, so a whole
  plan spans the readout. The step is also text, since the live region around it announces
  text changes, not attribute ones.
-->
<template>
  <div v-if="progress" class="solve-progress">
    <div class="segments" aria-hidden="true">
      <span
          v-for="i in progress.total"
          :key="i"
          class="segment"
          :class="{done: i < progress.attempt, current: i === progress.attempt}"
      />
    </div>
    <span class="visually-hidden">step {{ progress.attempt }} of {{ progress.total }}</span>
  </div>
</template>

<style scoped>
.solve-progress,
.segments {
  width: 100%;
  height: 100%;
}

.segments {
  display: flex;
  gap: 2px;
}

/* `--text-muted`, not `--border`: on the chip's surface `--border` was ~1.15:1, so a plan's untried steps vanished. */
.segment {
  flex: 1;
  border-radius: var(--radius-xs);
  background: var(--text-muted);
}

.segment.done,
.segment.current {
  background: var(--primary);
}

.segment.current {
  animation: blink 1.5s ease-in-out infinite;
}

/* Still, the current step reads as reached, like the ones before it. */
@media (prefers-reduced-motion: reduce) {
  .segment.current {
    animation: none;
  }
}
</style>
