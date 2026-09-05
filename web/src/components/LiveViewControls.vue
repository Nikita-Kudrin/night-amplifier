<script setup>
import BaseInfoIcon from './ui/BaseInfoIcon.vue'

defineProps({
  scale: {
    type: Number,
    default: 1.0
  },
  fps: {
    type: Number,
    default: 0
  },
  backendLabel: {
    type: String,
    default: 'Canvas2D'
  },
  debugLogging: {
    type: Boolean,
    default: false
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
  /** A guide camera is connected, so the source toggle has something to switch to. */
  hasGuideCamera: {
    type: Boolean,
    default: false
  },
  /** True while the view is showing the guide camera rather than the imaging one. */
  showGuide: {
    type: Boolean,
    default: false
  },
})

defineEmits([
  'zoomIn',
  'zoomOut',
  'fitToView',
  'resetView',
  'toggleFullscreen',
  'startCometRoiSelection',
  'update:showGuide',
])
</script>

<template>
  <!-- `data-overlay-control` marks the whole cluster as controls rather than image,
       so a press on the FPS readout refreshes the auto-hide instead of toggling it. -->
  <div class="controls-overlay" data-overlay-control>
    <div v-if="hasFrame" class="frame-info">
      <span class="frame-info-line">
        <span class="fps-container">
          <span class="fps-display">FPS {{ fps }}</span>
          <BaseInfoIcon message="Frames Per Second (FPS) indicates how many images are being processed and displayed per second from the live stream." />
        </span>
      </span>
      <span v-if="debugLogging" class="render-backend" :title="'Rendering: ' + backendLabel">{{ backendLabel }}</span>
    </div>

    <!-- Outside `.zoom-controls` on purpose: that group is the zoom cluster, and the
         source switch is not one of its buttons. -->
    <button
        v-if="hasGuideCamera"
        class="btn btn-sm guide-toggle"
        :class="{ active: showGuide }"
        type="button"
        :aria-pressed="showGuide ? 'true' : 'false'"
        title="Show the guide camera's view instead of the imaging camera's"
        @click="$emit('update:showGuide', !showGuide)"
    >
      Guide camera
    </button>

    <div class="zoom-controls">
      <button class="btn btn-icon btn-overlay" title="Fit to view" @click="$emit('fitToView')">
        <svg
            viewBox="0 0 24 24"
            width="18"
            height="18"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path
              d="M8 3H5a2 2 0 00-2 2v3M21 8V5a2 2 0 00-2-2h-3M3 16v3a2 2 0 002 2h3M16 21h3a2 2 0 002-2v-3"
          />
        </svg>
      </button>
      <button class="btn btn-icon btn-overlay" title="Fullscreen" @click="$emit('toggleFullscreen')">
        <svg
            v-if="!isFullscreen"
            viewBox="0 0 24 24"
            width="18"
            height="18"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path d="M15 3h6v6M9 21H3v-6M21 15v6h-6M3 9V3h6" />
        </svg>
        <svg
            v-else
            viewBox="0 0 24 24"
            width="18"
            height="18"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path d="M4 14h6v6M20 10h-6V4M14 10l7-7M10 14l-7 7" />
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
            width="18"
            height="18"
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
  bottom: 0.75rem;
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
  border-radius: 6px;
  padding: 0.15rem;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}

.btn-overlay {
  width: 30px;
  height: 30px;
  padding: 0.35rem;
  background: transparent;
  border-radius: 4px;
}

.btn-overlay:hover {
  background: var(--surface-hover);
}

.btn-overlay.active {
  background: var(--primary);
  color: white;
}

.guide-toggle {
  display: flex;
  align-items: center;
  padding: 0 0.6rem;
  background: var(--surface-elevated);
  border: none;
  border-radius: 6px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  color: var(--text-secondary, #9ca3af);
  font-size: 0.78rem;
  white-space: nowrap;
}

.guide-toggle.active {
  background: var(--primary);
  color: #fff;
}

.frame-info {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  justify-content: center;
  gap: 0;
  font-size: 0.7rem;
  color: var(--text-secondary);
  background: var(--surface-elevated);
  padding: 0.15rem 0.5rem;
  border-radius: 6px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  white-space: nowrap;
}

.frame-info-line {
  display: flex;
  gap: 0.5rem;
}

.zoom-level, .fps-display, .render-backend {
  font-family: var(--font-mono);
}

.fps-container {
  display: flex;
  align-items: center;
  gap: 2px;
}

.render-backend {
  color: var(--text-muted);
  font-size: 0.6rem;
}

@media (max-width: 768px) {
  .controls-overlay {
    bottom: 0.5rem;
  }

  .zoom-controls {
    padding: 0.125rem;
  }

  .btn-overlay {
    width: 36px;
    height: 36px;
  }
}
</style>

