<script setup lang="ts">
import {ref, computed, onMounted, onUnmounted, useTemplateRef, watch} from 'vue'
import {useImageStream} from '../composables/useImageStream'
import {useFrameRenderer} from '../composables/useFrameRenderer'
import {useEventListener} from '../composables/useEventListener'
import {usePanZoom} from '../composables/usePanZoom'
import {useAutoFit} from '../composables/useAutoFit'
import {useOverlayVisibility} from '../composables/useOverlayVisibility'
import {useCometRoi} from '../composables/useCometRoi'
import {useTimeouts} from '../composables/useTimeouts'
import {isFitted, reframeForCrop, useCropFraming} from '../composables/useCropFraming'
import {CAPTURE_STATES, isCaptureRunning, liveStreamEndpoint} from '../constants'
import GuideArrow from './GuideArrow.vue'
import LiveViewControls from './LiveViewControls.vue'
import LiveViewCometOverlay from './LiveViewCometOverlay.vue'
import {useAppState} from '../composables/useAppState'
import {
  useApplySettings,
  useCapabilities,
  useEventStreamContext,
  useGuideCamera,
  useHasGuideCamera,
  useMainCamera,
  useSettings,
} from '../composables/appContext'
import {useDisplayedFov, useOtherSolvedCamera} from '../composables/useDisplayedFov'
import {setViewedCamera} from '../composables/api'
import type {ScreenRect} from '../composables/useCometRoi'

/**
 * How long to wait before fitting after a frame changes the image's size. Viewport
 * changes (fullscreen, rotation) wait for the container itself — see `useAutoFit`.
 */
const FIT_AFTER_LAYOUT_MS = 10

const eventStream = useEventStreamContext()
const settings = useSettings()
const {setGlobalError} = useAppState()
const {later} = useTimeouts()
const {debugLogging} = useCapabilities()
const hasGuideCamera = useHasGuideCamera()
const guideCamera = useGuideCamera()
const mainCamera = useMainCamera()

/**
 * The operator's Guide toggle, held by the server rather than this tab: the eyepiece
 * pages show the same camera, and a reloaded page finds it where it was left. The server
 * falls back to the imaging camera when the guide camera goes away.
 */
const showGuide = computed(() => hasGuideCamera.value && eventStream?.viewedCamera.value === 'guide')

async function setShowGuide(show: boolean) {
  const revision = eventStream?.viewedCameraRevision.value
  try {
    const answer = await setViewedCamera(show ? 'guide' : 'main')
    // The event follows, but this tab need not wait for it to move its own view — unless
    // one already arrived: it may be another operator's later toggle, which this older
    // answer must not undo while every eyepiece shows the other camera.
    if (eventStream && eventStream.viewedCameraRevision.value === revision) {
      eventStream.viewedCamera.value = answer.camera
    }
  } catch (e) {
    setGlobalError(e instanceof Error ? e.message : String(e))
  }
}

const streamEndpoint = computed(() => liveStreamEndpoint(showGuide.value ? 'guide' : 'main'))

const {connected, frameData, isJpeg, dimensions, fps, clearFrameData} = useImageStream({
  endpoint: streamEndpoint,
})

/** The camera whose image is on screen right now. */
const displayedCamera = computed(() =>
    showGuide.value ? guideCamera.value : mainCamera.value
)

const pushDirection = computed(() => eventStream?.pushDirection.value ?? null)
const displayedFovDeg = useDisplayedFov(displayedCamera, settings, pushDirection)
const arrowCamera = useOtherSolvedCamera(displayedCamera, pushDirection)
const currentTarget = computed(() => eventStream?.currentTarget.value ?? null)
const showGuideArrow = computed(() => currentTarget.value !== null && pushDirection.value !== null)

const containerRef = useTemplateRef<HTMLDivElement>('container')
const canvasRef = useTemplateRef<HTMLCanvasElement>('canvas')

const containerSize = ref({width: 400, height: 300})
const canvasBounds = ref<ScreenRect>({left: 0, top: 0, width: 400, height: 300})

const {
  requestFit,
  handleContainerResize,
  dispose: disposeAutoFit,
} = useAutoFit({fit: fitToView})

// Pan/zoom controls. Both fullscreen edges fit the image to the new viewport.
const {
  scale,
  position,
  isDragging,
  isFullscreen,
  canvasStyle,
  handleWheel: handleWheelBase,
  handleMouseDown: handlePanMouseDown,
  handleMouseMove: handlePanMouseMove,
  handleMouseUp: handlePanMouseUp,
  handleTouchStart: handleTouchStartBase,
  handleTouchMove: handleTouchMoveBase,
  handleTouchEnd: handleTouchEndBase,
  zoomIn,
  zoomOut,
  resetView,
  fitToView: fitToViewBase,
  toggleFullscreen: toggleFullscreenBase,
  handleFullscreenChange,
} = usePanZoom({onChange: requestFit})

const {
  visible: overlayVisible,
  show: showOverlay,
  cancelPress,
  handlePressStart,
  handlePressEnd,
} = useOverlayVisibility()

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
} = useCometRoi(settings, dimensions, canvasBounds, containerRef, {save: useApplySettings()})

const isCapturing = computed(() => isCaptureRunning(eventStream?.captureState.value ?? CAPTURE_STATES.IDLE))
const hasFrame = computed(() => frameData.value !== null && dimensions.value.width > 0)

// A new frame size resizes the canvas, which the arrow and the comet ROI follow.
const {label: backendLabel} = useFrameRenderer({
  frameData,
  dimensions,
  isJpeg,
  canvases: () => [canvasRef.value],
  onSizeChange: () => updateContainerSize(),
})

function fitToView() {
  if (!containerRef.value) return
  const container = containerRef.value.getBoundingClientRect()
  // Use canvas intrinsic dimensions when available, otherwise use stream dimensions
  const canvas = canvasRef.value
  const imgW = dimensions.value.width || (canvas && canvas.width)
  const imgH = dimensions.value.height || (canvas && canvas.height)
  if (imgW && imgH) {
    fitToViewBase(container, imgW, imgH)
  }
}

function toggleFullscreen() {
  toggleFullscreenBase(containerRef.value)
}

// Gestures keep the overlay up for as long as they last: a pan or a pinch is the
// user working with the image, and the controls must not vanish mid-drag.
function handleWheel(e: WheelEvent) {
  showOverlay()
  handleWheelBase(e)
}

function handleTouchStart(e: TouchEvent) {
  handlePressStart(e)
  handleTouchStartBase(e)
}

function handleTouchMove(e: TouchEvent) {
  showOverlay()
  handleTouchMoveBase(e)
}

function handleTouchEnd(e: TouchEvent) {
  handlePressEnd(e)
  handleTouchEndBase()
}

// The system taking the gesture away — a notification swipe, palm rejection — is
// not the observer asking for anything, so it must not read as a tap.
function handleTouchCancel(e: TouchEvent) {
  cancelPress(e)
  handleTouchEndBase()
}

function handleMouseDown(e: MouseEvent) {
  handlePressStart(e)
  if (isSelectingCometRoi.value) {
    handleRoiMouseDown(e)
    return
  }
  handlePanMouseDown(e)
}

function handleMouseMove(e: MouseEvent) {
  if (isSelectingCometRoi.value) {
    showOverlay()
    handleRoiMouseMove(e)
    return
  }
  if (isDragging.value) showOverlay()
  handlePanMouseMove(e)
}

function handleMouseUp(e: MouseEvent) {
  handlePressEnd(e)
  if (isSelectingCometRoi.value) {
    void handleRoiMouseUp()
    return
  }
  handlePanMouseUp()
}

// Fit the view on the very first frame
watch(hasFrame, (newHasFrame, oldHasFrame) => {
  if (newHasFrame && !oldHasFrame) {
    later(fitToView, FIT_AFTER_LAYOUT_MS)
  }
})

// The server's auto crop moved: held until the frame cut the new way arrives. Only the
// imaging stream is ever cropped, so a change pending across a switch of camera is dropped.
const cropFraming = useCropFraming()
if (eventStream) {
  watch(() => eventStream.stackCrop.value, (to, from) => {
    if (!showGuide.value) cropFraming.note(to.crop, from.crop, to.frame)
  })
}
watch(showGuide, () => cropFraming.clear())

// Fit the view when a frame changes the image's size — unless the frame is a new crop of
// a view the observer zoomed or panned, which keeps its framing (see `useCropFraming`).
watch([dimensions, frameData], ([newDims], [oldDims]) => {
  const resized = newDims.width !== oldDims.width || newDims.height !== oldDims.height
  const crop = frameData.value ? cropFraming.take(newDims) : null
  if (crop && oldDims.width) {
    const view = {scale: scale.value, position: position.value}
    const container = containerRef.value?.getBoundingClientRect()
    const reframed = !isFitted(view, container, oldDims) &&
        reframeForCrop({...crop, oldDims, newDims, view})
    if (reframed) {
      scale.value = reframed.scale
      position.value = reframed.position
      return
    }
  }
  if (resized) later(fitToView, FIT_AFTER_LAYOUT_MS)
})

// Clear frame data when a new capture session starts. A capture resuming after a camera
// reconnect is the same session: blanking it would show the dropout the server hid.
watch(() => eventStream?.captureState.value, (newState, oldState) => {
  if (newState === CAPTURE_STATES.STARTING && oldState !== CAPTURE_STATES.RECOVERING) {
    clearFrameData()
  }
})

// The arrow and the comet ROI follow the canvas: re-measure once the view has moved it.
// A new frame size re-measures from the renderer's `onSizeChange` — not every frame,
// which forced two layouts per frame.
watch([scale, position, hasFrame], () => updateContainerSize(), {flush: 'post'})

let resizeObserver: ResizeObserver | null = null

function handleWindowResize() {
  // Only in fullscreen: windowed, the user's own pan and zoom survive a resize
  // (dragging the sidebar, say), and re-fitting would throw their framing away.
  if (isFullscreen.value) requestFit()
}

// Rotating fits windowed too: `/` lays out differently in landscape, so the old
// framing no longer describes anything. Both events are listened to — Safari before
// 16.4 has no `screen.orientation`, and mobile Safari rotates without always firing
// a `resize`.
function handleOrientationChange() {
  requestFit()
  handleWindowResize()
}

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
    handleContainerResize(containerRect.width, containerRect.height)
  }
}

useEventListener(document, 'fullscreenchange', handleFullscreenChange)
useEventListener(window, 'resize', handleWindowResize)
useEventListener(window, 'orientationchange', handleOrientationChange)
useEventListener(window.screen.orientation, 'change', handleOrientationChange)

onMounted(() => {
  updateContainerSize()
  resizeObserver = new ResizeObserver(updateContainerSize)
  if (containerRef.value) {
    resizeObserver.observe(containerRef.value)
  }
})

onUnmounted(() => {
  disposeAutoFit()
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
})
</script>

<template>
  <div
      ref="container"
      class="live-view"
      :class="{ capturing: isCapturing, fullscreen: isFullscreen, 'selecting-roi': isSelectingCometRoi }"
      @wheel="handleWheel"
      @mousedown="handleMouseDown"
      @mousemove="handleMouseMove"
      @mouseup="handleMouseUp"
      @mouseleave="isSelectingCometRoi ? null : handlePanMouseUp()"
      @touchstart.passive="handleTouchStart"
      @touchmove.passive="handleTouchMove"
      @touchend="handleTouchEnd"
      @touchcancel="handleTouchCancel"
      @focusin="showOverlay"
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

    <!-- Canvas for rendering (used for both LZ4 and JPEG paths) -->
    <canvas v-show="hasFrame" ref="canvas" :style="canvasStyle" class="live-canvas"/>

    <!-- Guide arrow -->
    <GuideArrow
        v-if="showGuideArrow && pushDirection"
        :direction="pushDirection"
        :bounds="canvasBounds"
        :fov-deg="displayedFovDeg"
        :other-camera="arrowCamera"
    />

    <!-- Comet ROI Overlay -->
    <LiveViewCometOverlay
        class="overlay-fade"
        :class="{ 'overlay-hidden': !overlayVisible && !isSelectingCometRoi }"
        :is-comet-mode="isCometMode"
        :has-frame="hasFrame"
        :roi-display-rect="roiDisplayRect"
        :is-selecting-comet-roi="isSelectingCometRoi"
        :selection-rect="selectionRect"
        @cancel="cancelCometRoiSelection"
    />

    <!-- Controls (Zoom, Frame Info). Fades out with the rest of the overlay; the
         Push-To chevron above deliberately does not. -->
    <LiveViewControls
        class="overlay-fade"
        :class="{ 'overlay-hidden': !overlayVisible }"
        :scale="scale"
        :fps="fps"
        :backend-label="backendLabel"
        :debug-logging="debugLogging"
        :is-fullscreen="isFullscreen"
        :has-frame="hasFrame"
        :is-comet-mode="isCometMode"
        :is-selecting-comet-roi="isSelectingCometRoi"
        :has-guide-camera="hasGuideCamera"
        :show-guide="showGuide"
        @update:show-guide="setShowGuide"
        @zoom-in="zoomIn"
        @zoom-out="zoomOut"
        @fit-to-view="fitToView"
        @reset-view="resetView"
        @toggle-fullscreen="toggleFullscreen"
        @start-comet-roi-selection="startCometRoiSelection"
    />


    <!-- Connection status -->
    <div
        v-if="!connected"
        class="hud-chip connection-status disconnected overlay-fade"
        :class="{ 'overlay-hidden': !overlayVisible }"
    >
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
  font-size: var(--fs-base);
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
  font-size: var(--fs-sm);
  padding: 0.375rem 0.75rem;
}

.status-dot {
  width: 8px;
  height: 8px;
  background: var(--error);
  animation: pulse 2s infinite;
}
</style>
