<script setup>
defineProps({
  scale: {
    type: Number,
    default: 1.0
  },
  frameNumber: {
    type: Number,
    default: 0
  },
  backendLabel: {
    type: String,
    default: 'Canvas2D'
  },
  isFullscreen: {
    type: Boolean,
    default: false
  },
  hasFrame: {
    type: Boolean,
    default: false
  },
  isCometMode: {
    type: Boolean,
    default: false
  },
  isSelectingCometRoi: {
    type: Boolean,
    default: false
  },
})

defineEmits(['zoomIn', 'zoomOut', 'fitToView', 'resetView', 'toggleFullscreen', 'startCometRoiSelection'])
</script>

<template>
  <div class="controls-overlay">
    <div v-if="hasFrame" class="frame-info">
      <span class="frame-info-line">
        <span class="zoom-level">{{ (scale * 100).toFixed(0) }}%</span>
        <span class="frame-number">Frame {{ frameNumber }}</span>
      </span>
      <span class="render-backend" :title="'Rendering: ' + backendLabel">{{ backendLabel }}</span>
    </div>

    <div class="zoom-controls">
      <button class="btn btn-icon btn-overlay" title="Zoom in" @click="$emit('zoomIn')">
        <svg
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <circle cx="11" cy="11" r="8"/>
          <path d="M21 21l-4.35-4.35M11 8v6M8 11h6"/>
        </svg>
      </button>
      <button class="btn btn-icon btn-overlay" title="Zoom out" @click="$emit('zoomOut')">
        <svg
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <circle cx="11" cy="11" r="8"/>
          <path d="M21 21l-4.35-4.35M8 11h6"/>
        </svg>
      </button>
      <button class="btn btn-icon btn-overlay" title="Fit to view" @click="$emit('fitToView')">
        <svg
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path
              d="M8 3H5a2 2 0 00-2 2v3M21 8V5a2 2 0 00-2-2h-3M3 16v3a2 2 0 002 2h3M16 21h3a2 2 0 002-2v-3"
          />
        </svg>
      </button>
      <button class="btn btn-icon btn-overlay" title="Reset view" @click="$emit('resetView')">
        <svg
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path d="M3 12a9 9 0 109-9 9.75 9.75 0 00-6.74 2.74L3 8"/>
          <path d="M3 3v5h5"/>
        </svg>
      </button>
      <button class="btn btn-icon btn-overlay" title="Fullscreen" @click="$emit('toggleFullscreen')">
        <svg
            v-if="!isFullscreen"
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path
              d="M8 3H5a2 2 0 00-2 2v3m18 0V5a2 2 0 00-2-2h-3m0 18h3a2 2 0 002-2v-3M3 16v3a2 2 0 002 2h3"
          />
        </svg>
        <svg
            v-else
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path
              d="M8 3v3a2 2 0 01-2 2H3m18 0h-3a2 2 0 01-2-2V3m0 18v-3a2 2 0 012-2h3M3 16h3a2 2 0 012 2v3"
          />
        </svg>
      </button>
      <!-- Comet ROI selection button -->
      <button
          v-if="isCometMode && hasFrame"
          class="btn btn-icon btn-overlay"
          :class="{ active: isSelectingCometRoi }"
          title="Select comet nucleus region"
          @click="$emit('startCometRoiSelection')"
      >
        <svg
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <circle cx="12" cy="12" r="3"/>
          <path d="M12 2v4M12 18v4M2 12h4M18 12h4"/>
          <rect x="4" y="4" width="16" height="16" rx="2" stroke-dasharray="4 2"/>
        </svg>
      </button>
    </div>
  </div>
</template>

<style scoped>
.controls-overlay {
  position: absolute;
  bottom: 1rem;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  flex-direction: row;
  align-items: stretch;
  gap: 0.25rem;
  z-index: 10;
}

.zoom-controls {
  display: flex;
  gap: 0.25rem;
  background: var(--surface-elevated);
  border-radius: 8px;
  padding: 0.25rem;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}

.btn-overlay {
  width: 36px;
  height: 36px;
  padding: 0.5rem;
  background: transparent;
  border-radius: 6px;
}

.btn-overlay:hover {
  background: var(--surface-hover);
}

.btn-overlay.active {
  background: var(--primary);
  color: white;
}

.frame-info {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  justify-content: center;
  gap: 0;
  font-size: 0.75rem;
  color: var(--text-secondary);
  background: var(--surface-elevated);
  padding: 0.25rem 0.5rem;
  border-radius: 8px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  white-space: nowrap;
}

.frame-info-line {
  display: flex;
  gap: 0.5rem;
}

.zoom-level, .frame-number, .render-backend {
  font-family: var(--font-mono);
}

.render-backend {
  color: var(--text-muted);
  font-size: 0.65rem;
}

@media (max-width: 768px) {
  .controls-overlay {
    bottom: 0.5rem;
  }

  .zoom-controls {
    padding: 0.125rem;
  }

  .btn-overlay {
    width: 44px;
    height: 44px;
  }
}
</style>
