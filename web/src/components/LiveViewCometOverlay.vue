<script setup>
defineProps({
  isCometMode: {
    type: Boolean,
    default: false
  },
  hasFrame: {
    type: Boolean,
    default: false
  },
  roiDisplayRect: {
    type: Object,
    default: null
  },
  isSelectingCometRoi: {
    type: Boolean,
    default: false
  },
  selectionRect: {
    type: Object,
    default: null
  },
})

defineEmits(['cancel'])
</script>

<template>
  <div class="comet-overlay-container">
    <!-- Comet ROI overlay (shows current ROI when in comet mode) -->
    <div
        v-if="isCometMode && hasFrame && roiDisplayRect && !isSelectingCometRoi"
        class="comet-roi-overlay"
        :style="{
          left: roiDisplayRect.left + 'px',
          top: roiDisplayRect.top + 'px',
          width: roiDisplayRect.width + 'px',
          height: roiDisplayRect.height + 'px',
        }"
    >
      <span class="roi-label">Comet ROI</span>
    </div>

    <!-- ROI selection rectangle (while drawing) -->
    <div
        v-if="isSelectingCometRoi && selectionRect"
        class="roi-selection"
        :style="{
          left: selectionRect.left + 'px',
          top: selectionRect.top + 'px',
          width: selectionRect.width + 'px',
          height: selectionRect.height + 'px',
        }"
    />

    <!-- ROI selection prompt -->
    <div v-if="isSelectingCometRoi" class="roi-prompt">
      <div class="hud-chip roi-prompt-content">
        <span>Draw a box around the comet nucleus</span>
        <button class="btn btn-sm" @click="$emit('cancel')">Cancel</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.comet-roi-overlay {
  position: absolute;
  border: 2px solid var(--primary);
  border-radius: var(--radius-sm);
  pointer-events: none;
  z-index: var(--z-raised);
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.5);
}

.roi-label {
  position: absolute;
  top: -20px;
  left: 0;
  font-size: var(--fs-2xs);
  color: var(--primary);
  background: var(--surface-elevated);
  padding: 2px 6px;
  border-radius: var(--radius-xs);
  white-space: nowrap;
}

.roi-selection {
  position: absolute;
  border: 2px dashed var(--primary);
  background: var(--primary-bg);
  pointer-events: none;
  z-index: var(--z-hud);
}

.roi-prompt {
  position: absolute;
  top: 1rem;
  left: 50%;
  transform: translateX(-50%);
  z-index: var(--z-hud-top);
}

.roi-prompt-content {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.5rem 0.75rem;
  font-size: var(--fs-base);
  color: var(--text-primary);
}
</style>
