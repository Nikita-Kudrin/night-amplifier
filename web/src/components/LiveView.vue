<script setup>
import {ref, computed, inject, onMounted, onUnmounted, watch} from 'vue'
import {useImageStream} from '../composables/useWebSocket.js'
import {useWebGLRenderer} from '../composables/useWebGLRenderer.js'
import {useCanvas2DRenderer} from '../composables/useCanvas2DRenderer.js'
import {usePanZoom} from '../composables/usePanZoom.js'
import {useCometRoi} from '../composables/useCometRoi.js'
import {CAPTURE_STATES} from '../constants'
import GuideArrow from './GuideArrow.vue'
import LiveViewControls from './LiveViewControls.vue'
import LiveViewCometOverlay from './LiveViewCometOverlay.vue'

const eventStream = inject('eventStream')
const settings = inject('settings')

const {connected, frameData, dimensions, frameNumber, clearFrameData} = useImageStream()

const pushDirection = computed(() => eventStream.pushDirection.value)
const currentTarget = computed(() => eventStream.currentTarget.value)
const showGuideArrow = computed(() => currentTarget.value !== null && pushDirection.value !== null)

const containerRef = ref(null)
const canvasRef = ref(null)

const containerSize = ref({width: 400, height: 300})
const canvasBounds = ref({left: 0, top: 0, width: 400, height: 300})

// Renderers
const webglRenderer = useWebGLRenderer()
const canvas2dRenderer = useCanvas2DRenderer()

// Pan/zoom controls
const {
  scale,
  isFullscreen,
  canvasStyle,
  handleWheel,
  handleMouseDown: handlePanMouseDown,
  handleMouseMove: handlePanMouseMove,
  handleMouseUp: handlePanMouseUp,
  handleTouchStart,
  handleTouchMove,
  handleTouchEnd,
  zoomIn,
  zoomOut,
  resetView,
  fitToView: fitToViewBase,
  toggleFullscreen: toggleFullscreenBase,
  handleFullscreenChange,
} = usePanZoom()

// Comet ROI selection logic
const {
  isSelectingCometRoi,
  isCometMode,
  selectionRect,
  roiDisplayRect,
  startCometRoiSelection,
  cancelCometRoiSelection,
  handleMouseDown: handleRoiMouseDown,
  handleMouseMove: handleRoiMouseMove,
  handleMouseUp: handleRoiMouseUp,
} = useCometRoi(settings, dimensions, canvasRef, containerRef)

// Active renderer backend
const renderBackend = computed(() => {
  if (webglRenderer.isInitialized()) {
    return webglRenderer.backend.value
  }
  if (canvas2dRenderer.isInitialized()) {
    return canvas2dRenderer.backend.value
  }
  return 'none'
})

const isCapturing = computed(() => eventStream.captureState.value === CAPTURE_STATES.CAPTURING)
const hasFrame = computed(() => frameData.value !== null && dimensions.value.width > 0)

function initRenderer() {
  const canvas = canvasRef.value
  if (!canvas) return false
  if (webglRenderer.init(canvas)) return true
  if (canvas2dRenderer.init(canvas)) return true
  return false
}

function renderFrame() {
  if (!frameData.value) return
  const canvas = canvasRef.value
  const {width, height} = dimensions.value
  if (webglRenderer.isInitialized()) {
    webglRenderer.render(canvas, frameData.value, width, height)
  } else if (canvas2dRenderer.isInitialized()) {
    canvas2dRenderer.render(canvas, frameData.value, width, height)
  }
}

function cleanupRenderer() {
  webglRenderer.cleanup()
  canvas2dRenderer.cleanup()
}

function fitToView() {
  if (!containerRef.value || !canvasRef.value) return
  const container = containerRef.value.getBoundingClientRect()
  const canvas = canvasRef.value
  if (canvas.width && canvas.height) {
    fitToViewBase(container, canvas.width, canvas.height)
  }
}

function toggleFullscreen() {
  toggleFullscreenBase(containerRef.value)
}

// Watch for new frame data and render
watch(frameData, () => {
  if (frameData.value) {
    renderFrame()
  }
})

// Watch for dimension changes to fit the view
watch(dimensions, (newDims, oldDims) => {
  if (newDims.width !== oldDims?.width || newDims.height !== oldDims?.height) {
    setTimeout(fitToView, 10)
  }
})

// Clear frame data when a new capture session starts
watch(() => eventStream.captureState.value, (newState) => {
  if (newState === CAPTURE_STATES.STARTING) {
    clearFrameData()
  }
})

// Update canvas bounds when scale changes or frame data changes
watch([scale, frameData, hasFrame], () => {
  setTimeout(updateContainerSize, 10)
})

let resizeObserver = null

function updateContainerSize() {
  if (containerRef.value) {
    const containerRect = containerRef.value.getBoundingClientRect()
    containerSize.value = {width: containerRect.width, height: containerRect.height}

    if (canvasRef.value && hasFrame.value) {
      const canvasRect = canvasRef.value.getBoundingClientRect()
      canvasBounds.value = {
        left: canvasRect.left - containerRect.left,
        top: canvasRect.top - containerRect.top,
        width: canvasRect.width,
        height: canvasRect.height,
      }
    } else {
      canvasBounds.value = {
        left: 0,
        top: 0,
        width: containerRect.width,
        height: containerRect.height,
      }
    }
  }
}

onMounted(() => {
  document.addEventListener('fullscreenchange', handleFullscreenChange)
  initRenderer()
  updateContainerSize()
  resizeObserver = new ResizeObserver(updateContainerSize)
  if (containerRef.value) {
    resizeObserver.observe(containerRef.value)
  }
})

onUnmounted(() => {
  document.removeEventListener('fullscreenchange', handleFullscreenChange)
  cleanupRenderer()
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
})

// Map backend names to user-friendly labels
const backendLabel = computed(() => {
  const labels = {
    'webgl2-16bit': 'WebGL2 16-bit',
    'webgl2-8bit': 'WebGL2 8-bit',
    webgl1: 'WebGL1',
    canvas2d: 'Canvas 2D',
    none: 'No renderer',
    unknown: '...',
  }
  return labels[renderBackend.value] || renderBackend.value
})
</script>

<template>
  <div
      ref="containerRef"
      class="live-view"
      :class="{ capturing: isCapturing, fullscreen: isFullscreen, 'selecting-roi': isSelectingCometRoi }"
      @wheel="handleWheel"
      @mousedown="isSelectingCometRoi ? handleRoiMouseDown($event) : handlePanMouseDown($event)"
      @mousemove="isSelectingCometRoi ? handleRoiMouseMove($event) : handlePanMouseMove($event)"
      @mouseup="isSelectingCometRoi ? handleRoiMouseUp() : handlePanMouseUp()"
      @mouseleave="isSelectingCometRoi ? null : handlePanMouseUp()"
      @touchstart.passive="handleTouchStart"
      @touchmove.passive="handleTouchMove"
      @touchend="handleTouchEnd"
  >
    <!-- Placeholder when no frame -->
    <div v-if="!hasFrame" class="placeholder">
      <svg viewBox="0 0 100 100" width="80" height="80">
        <circle
            cx="50"
            cy="50"
            r="45"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            opacity="0.3"
        />
        <circle cx="50" cy="50" r="8" fill="currentColor" opacity="0.3"/>
        <circle cx="30" cy="35" r="3" fill="currentColor" opacity="0.2"/>
        <circle cx="70" cy="30" r="2" fill="currentColor" opacity="0.15"/>
        <circle cx="65" cy="65" r="2.5" fill="currentColor" opacity="0.2"/>
      </svg>
      <p v-if="!connected">Connecting to stream...</p>
      <p v-else-if="!isCapturing">Start capture to see live view</p>
      <p v-else>Waiting for frames...</p>
    </div>

    <!-- Canvas for rendering -->
    <canvas v-show="hasFrame" ref="canvasRef" :style="canvasStyle" class="live-canvas"/>

    <!-- Guide arrow -->
    <GuideArrow
        v-if="showGuideArrow"
        :angle-deg="pushDirection.angleDeg"
        :distance-deg="pushDirection.distanceDeg"
        :is-close="pushDirection.isClose"
        :direction-hint="pushDirection.directionHint"
        :image-left="canvasBounds.left"
        :image-top="canvasBounds.top"
        :image-width="canvasBounds.width"
        :image-height="canvasBounds.height"
        :fov-deg="pushDirection.fovDeg || 0"
    />

    <!-- Comet ROI Overlay -->
    <LiveViewCometOverlay
        :is-comet-mode="isCometMode"
        :has-frame="hasFrame"
        :roi-display-rect="roiDisplayRect"
        :is-selecting-comet-roi="isSelectingCometRoi"
        :selection-rect="selectionRect"
        @cancel="cancelCometRoiSelection"
    />

    <!-- Controls (Zoom, Frame Info) -->
    <LiveViewControls
        :scale="scale"
        :frame-number="frameNumber"
        :backend-label="backendLabel"
        :is-fullscreen="isFullscreen"
        :has-frame="hasFrame"
        :is-comet-mode="isCometMode"
        :is-selecting-comet-roi="isSelectingCometRoi"
        @zoom-in="zoomIn"
        @zoom-out="zoomOut"
        @fit-to-view="fitToView"
        @reset-view="resetView"
        @toggle-fullscreen="toggleFullscreen"
        @start-comet-roi-selection="startCometRoiSelection"
    />


    <!-- Connection status -->
    <div v-if="!connected" class="connection-status disconnected">
      <span class="status-dot"></span>
      Disconnected
    </div>
  </div>
</template>

<style scoped>
.live-view {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: var(--bg);
  display: flex;
  align-items: center;
  justify-content: center;
  user-select: none;
  touch-action: none;
}

.live-view.fullscreen {
  background: black;
}

.live-view.selecting-roi {
  cursor: crosshair;
}

.placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
  color: var(--text-muted);
}

.placeholder p {
  margin: 0;
  font-size: 0.875rem;
}

.live-canvas {
  max-width: none;
  max-height: none;
  transform-origin: center center;
  transition: none;
  image-rendering: high-quality;
}

.connection-status {
  position: absolute;
  top: 1rem;
  right: 1rem;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.75rem;
  padding: 0.375rem 0.75rem;
  border-radius: 4px;
  background: var(--surface-elevated);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}

.status-dot {
  width: 8px;
  height: 8px;
  background: var(--error);
  animation: pulse 2s infinite;
}

</style>
