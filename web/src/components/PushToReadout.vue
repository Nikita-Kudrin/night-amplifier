<script setup lang="ts">
import type {ClientPushDirection} from '../composables/events/pushToEvents'

withDefaults(defineProps<{
  distance: string
  screens?: string | null
  hint?: ClientPushDirection['directionHint'] | null
  /** The last solve failed: these are the numbers from before it. */
  stale?: boolean
  /** Named when the angle was solved on a connected camera other than the one on screen. */
  otherCameraName?: string | null
}>(), {
  screens: null,
  hint: null,
  stale: false,
  otherCameraName: null,
})
</script>

<template>
  <div class="readout">
    <span v-if="stale" class="last-fix">last fix</span>
    <span class="distance">{{ distance }}</span>
    <span v-if="screens" class="screen-size">{{ screens }}</span>
    <span class="hint">{{ hint }}</span>
    <span v-if="otherCameraName" class="other-camera">Arrow on {{ otherCameraName }}</span>
  </div>
</template>

<style scoped>
.readout {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.15rem;
  white-space: nowrap;
}

.distance {
  font-size: 1.2rem;
  font-weight: 600;
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.screen-size {
  font-size: var(--fs-lg);
  font-weight: 500;
  color: var(--primary);
  font-family: var(--font-mono);
}

.hint {
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

.last-fix {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.other-camera {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}
</style>
