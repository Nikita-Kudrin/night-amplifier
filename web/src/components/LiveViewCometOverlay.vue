<script setup>
defineProps({
  isCometMode: Boolean,
  hasFrame: Boolean,
  roiDisplayRect: Object,
  isSelectingCometRoi: Boolean,
  selectionRect: Object,
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
      <div class="roi-prompt-content">
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
  border-radius: 4px;
  pointer-events: none;
  z-index: 5;
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.5);
}

.roi-label {
  position: absolute;
  top: -20px;
  left: 0;
  font-size: 0.65rem;
  color: var(--primary);
  background: var(--surface-elevated);
  padding: 2px 6px;
  border-radius: 3px;
  white-space: nowrap;
}

.roi-selection {
  position: absolute;
  border: 2px dashed var(--primary);
  background: rgba(var(--primary-rgb), 0.1);
  pointer-events: none;
  z-index: 20;
}

.roi-prompt {
  position: absolute;
  top: 1rem;
  left: 50%;
  transform: translateX(-50%);
  z-index: 25;
}

.roi-prompt-content {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  background: var(--surface-elevated);
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  font-size: 0.875rem;
  color: var(--text);
}
</style>
