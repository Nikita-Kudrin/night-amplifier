<script setup>
import {ref, computed, inject, onMounted, onUnmounted, watch} from 'vue'
import {useImageStream} from '../composables/useWebSocket.js'
import {useWebGLRenderer} from '../composables/useWebGLRenderer.js'
import {useCanvas2DRenderer} from '../composables/useCanvas2DRenderer.js'
import {useFullscreen} from '../composables/useFullscreen.js'
import {useOverlayVisibility} from '../composables/useOverlayVisibility.js'
import {getAppState} from '../composables/useAppState.js'
import {fetchEyepieceSnapshot} from '../composables/api.js'
import {saveBlob} from '../utils/saveBlob.js'
import GuideArrow from './GuideArrow.vue'
import {BaseSpinner, BaseSplitButton} from './ui'

const eventStream = inject('eventStream')
const settings = inject('settings')
const appState = getAppState()
const capabilities = appState.capabilities

const routePath = window.location.pathname
const endpoint = routePath === '/eyepiece_quality' ? '/ws/eyepiece_quality' : '/ws/eyepiece'
const {connected, frameData, dimensions, isJpeg, sendResolution} = useImageStream({ 
  endpoint,
  width: Math.round(window.innerWidth * (window.devicePixelRatio || 1)),
  height: Math.round(window.innerHeight * (window.devicePixelRatio || 1)),
})

const canvasLeftRef = ref(null)
const canvasRightRef = ref(null)
const canvasSingleRef = ref(null)

const webglLeft = useWebGLRenderer()
const canvas2dLeft = useCanvas2DRenderer()

const webglRight = useWebGLRenderer()
const canvas2dRight = useCanvas2DRenderer()

const webglSingle = useWebGLRenderer()
const canvas2dSingle = useCanvas2DRenderer()

const leftEyeRef = ref(null)
const rightEyeRef = ref(null)
const singleEyeRef = ref(null)

const leftEyeBounds = ref({ left: 0, top: 0, width: 0, height: 0 })
const rightEyeBounds = ref({ left: 0, top: 0, width: 0, height: 0 })
const singleViewBounds = ref({ left: 0, top: 0, width: 0, height: 0 })

const pushDirection = computed(() => eventStream?.pushDirection?.value ?? null)
const currentTarget = computed(() => eventStream?.currentTarget?.value ?? null)
const showGuideArrow = computed(() => currentTarget.value !== null && pushDirection.value !== null)

/**
 * `/eyepiece` is monocular whatever the Binoview setting says — it is the view an
 * observer puts their eye to. `/eyepiece_quality` still honours the setting.
 */
const isBinoview = computed(() => {
  if (routePath === '/eyepiece') return false
  return settings.value?.eyepiece?.binoview ?? true
})
const isCircularView = computed(() => settings.value?.eyepiece?.circular_view ?? true)
const showFocusImage = computed(() => settings.value?.show_focus_image ?? false)
const forceFocusImageNow = computed(() => settings.value?.force_focus_image_now ?? false)

const hasFrame = computed(() => frameData.value !== null && dimensions.value.width > 0)
const effectiveHasFrame = computed(() => hasFrame.value && !forceFocusImageNow.value)

// Active renderer backend (assuming both eyes use same backend)
const renderBackend = computed(() => {
  if (isJpeg.value) return 'jpeg'
  if (isBinoview.value) {
    if (webglLeft.isInitialized()) return webglLeft.backend.value
    if (canvas2dLeft.isInitialized()) return canvas2dLeft.backend.value
  } else {
    if (webglSingle.isInitialized()) return webglSingle.backend.value
    if (canvas2dSingle.isInitialized()) return canvas2dSingle.backend.value
  }
  return 'none'
})

// Map backend names to user-friendly labels
const backendLabel = computed(() => {
  const res = dimensions.value.width > 0 
    ? ` (${dimensions.value.width}x${dimensions.value.height})`
    : ''

  const labels = {
    jpeg: `Dynamic JPEG${res}`,
    'webgl2-16bit': `WebGL2 16-bit${res}`,
    'webgl2-8bit': `WebGL2 8-bit${res}`,
    webgl1: `WebGL1${res}`,
    canvas2d: `Canvas 2D${res}`,
    none: 'No renderer',
    unknown: '...',
  }
  return labels[renderBackend.value] || renderBackend.value
})

function initRenderer() {
  if (canvasLeftRef.value) {
    if (!webglLeft.init(canvasLeftRef.value)) canvas2dLeft.init(canvasLeftRef.value)
  }
  if (canvasRightRef.value) {
    if (!webglRight.init(canvasRightRef.value)) canvas2dRight.init(canvasRightRef.value)
  }
  if (canvasSingleRef.value) {
    if (!webglSingle.init(canvasSingleRef.value)) canvas2dSingle.init(canvasSingleRef.value)
  }
}

function renderFrame() {
  if (!frameData.value) return
  const {width, height} = dimensions.value

  if (isJpeg.value) {
    window.createImageBitmap(frameData.value).then((bitmap) => {
      if (isBinoview.value) {
        if (webglLeft.isInitialized()) webglLeft.render(canvasLeftRef.value, bitmap, bitmap.width, bitmap.height)
        else if (canvas2dLeft.isInitialized()) canvas2dLeft.render(canvasLeftRef.value, bitmap, bitmap.width, bitmap.height)
    
        if (webglRight.isInitialized()) webglRight.render(canvasRightRef.value, bitmap, bitmap.width, bitmap.height)
        else if (canvas2dRight.isInitialized()) canvas2dRight.render(canvasRightRef.value, bitmap, bitmap.width, bitmap.height)
      } else {
        if (webglSingle.isInitialized()) webglSingle.render(canvasSingleRef.value, bitmap, bitmap.width, bitmap.height)
        else if (canvas2dSingle.isInitialized()) canvas2dSingle.render(canvasSingleRef.value, bitmap, bitmap.width, bitmap.height)
      }
      bitmap.close()
    }).catch(() => { /* frame was replaced before decode finished */ })
    return
  }

  if (isBinoview.value) {
    if (webglLeft.isInitialized()) webglLeft.render(canvasLeftRef.value, frameData.value, width, height)
    else if (canvas2dLeft.isInitialized()) canvas2dLeft.render(canvasLeftRef.value, frameData.value, width, height)

    if (webglRight.isInitialized()) webglRight.render(canvasRightRef.value, frameData.value, width, height)
    else if (canvas2dRight.isInitialized()) canvas2dRight.render(canvasRightRef.value, frameData.value, width, height)
  } else {
    if (webglSingle.isInitialized()) webglSingle.render(canvasSingleRef.value, frameData.value, width, height)
    else if (canvas2dSingle.isInitialized()) canvas2dSingle.render(canvasSingleRef.value, frameData.value, width, height)
  }
}

function cleanupRenderer() {
  webglLeft.cleanup()
  canvas2dLeft.cleanup()
  webglRight.cleanup()
  canvas2dRight.cleanup()
  webglSingle.cleanup()
  canvas2dSingle.cleanup()
}

watch(frameData, () => {
  if (frameData.value) renderFrame()
})

watch(isBinoview, () => {
  // Re-render immediately if we switch views so it's not blank
  setTimeout(() => {
    if (frameData.value) renderFrame()
  }, 10)
})

let initialDist = 0
let initialScale = 1
let initialCx = 0
let initialCy = 0
let initialPanX = 0
let initialPanY = 0

const isZoomAllowed = computed(() => routePath === '/eyepiece')
const zoomScale = ref(1)
const panX = ref(0)
const panY = ref(0)
const isPinching = ref(false)

const zoomStyle = computed(() => {
  if (zoomScale.value === 1) return {}
  return {
    transform: `translate(${panX.value}px, ${panY.value}px) scale(${zoomScale.value})`,
    transformOrigin: '0 0',
  }
})

function clampPan(x, y, scale) {
  if (scale <= 1) return { x: 0, y: 0 }
  const W = window.innerWidth
  const H = window.innerHeight
  const minX = W * (1 - scale)
  const minY = H * (1 - scale)
  return {
    x: Math.min(0, Math.max(minX, x)),
    y: Math.min(0, Math.max(minY, y))
  }
}

function resetZoom() {
  zoomScale.value = 1
  panX.value = 0
  panY.value = 0
}

const rootRef = ref(null)

// Fit all on both fullscreen edges — the viewport changes size going in and
// coming out — and again whenever it changes shape under fullscreen, so a
// rotation mid-session does not leave the view panned off.
const {isFullscreen, toggleFullscreen: toggleFullscreenBase, handleFullscreenChange} =
    useFullscreen({onChange: resetZoom})

/**
 * iPhone Safari implements no Fullscreen API at all, so the button would be a
 * control that visibly does nothing on a phone at the eyepiece. Hide it there.
 */
const canFullscreen = computed(() => !!document.fullscreenEnabled)

function toggleFullscreen() {
  toggleFullscreenBase(rootRef.value)
}

const {
  visible: overlayVisible,
  show: showOverlay,
  setHold: holdOverlay,
  cancelPress,
  handlePressStart,
  handlePressEnd,
} = useOverlayVisibility()

/** How long a failed download stays on screen before it clears itself. */
const DOWNLOAD_ERROR_MS = 4000

const downloading = ref(false)
const downloadError = ref(null)
const menuOpen = ref(false)
let errorTimer = null

// The controls stay put while a menu is open or a download is running: a retry
// against a busy server can take fifteen seconds, and the spinner saying so must
// not fade out halfway through it.
watch([menuOpen, downloading], ([open, busy]) => holdOverlay(open || busy))

function reportDownloadError(message) {
  if (errorTimer) clearTimeout(errorTimer)
  downloadError.value = message
  errorTimer = setTimeout(() => {
    errorTimer = null
    downloadError.value = null
  }, DOWNLOAD_ERROR_MS)
}

/**
 * Save the frame the server last rendered, at its own resolution rather than the
 * tier this screen happens to be streaming. `circular` is the round eyepiece
 * image; without it, the same picture as the uncropped stretched result.
 *
 * The server names the file — the timestamp in it is its to stamp — and the name
 * travels with the bytes, because the blob the fetch produced has none of the
 * headers it arrived with.
 */
async function downloadSnapshot(circular) {
  if (downloading.value) return
  downloading.value = true
  showOverlay()
  try {
    const {blob, filename} = await fetchEyepieceSnapshot(circular)
    saveBlob(blob, filename)
  } catch (e) {
    reportDownloadError(e.message || 'Download failed')
  } finally {
    downloading.value = false
  }
}

function handleWheel(e) {
  showOverlay()
  if (!isZoomAllowed.value || !effectiveHasFrame.value) return

  const zoomSensitivity = 0.001
  const delta = -e.deltaY * zoomSensitivity
  
  const oldScale = zoomScale.value
  let newScale = oldScale + delta
  newScale = Math.min(2, Math.max(1, newScale))
  
  if (newScale === oldScale) return

  const cx = e.clientX
  const cy = e.clientY
  
  let newX = cx - (cx - panX.value) * (newScale / oldScale)
  let newY = cy - (cy - panY.value) * (newScale / oldScale)
  
  const clamped = clampPan(newX, newY, newScale)
  panX.value = clamped.x
  panY.value = clamped.y
  zoomScale.value = newScale
}

function getDist(touches) {
  const dx = touches[0].clientX - touches[1].clientX
  const dy = touches[0].clientY - touches[1].clientY
  return Math.sqrt(dx * dx + dy * dy)
}

function getCenter(touches) {
  if (touches.length === 1) {
    return { x: touches[0].clientX, y: touches[0].clientY }
  }
  return {
    x: (touches[0].clientX + touches[1].clientX) / 2,
    y: (touches[0].clientY + touches[1].clientY) / 2
  }
}

function handleTouchStart(e) {
  handlePressStart(e)
  if (!isZoomAllowed.value || !effectiveHasFrame.value) return

  if (e.touches.length === 2) {
    isPinching.value = true
    initialDist = getDist(e.touches)
    initialScale = zoomScale.value
    const center = getCenter(e.touches)
    initialCx = center.x
    initialCy = center.y
    initialPanX = panX.value
    initialPanY = panY.value
  } else if (e.touches.length === 1 && zoomScale.value > 1) {
    const center = getCenter(e.touches)
    initialCx = center.x
    initialCy = center.y
    initialPanX = panX.value
    initialPanY = panY.value
  }
}

function handleTouchMove(e) {
  showOverlay()
  if (!isZoomAllowed.value || !effectiveHasFrame.value) return

  if (e.touches.length === 2 && isPinching.value) {
    const currentDist = getDist(e.touches)
    const scaleRatio = currentDist / initialDist
    let newScale = initialScale * scaleRatio
    newScale = Math.min(2, Math.max(1, newScale))
    
    const currentCenter = getCenter(e.touches)
    let newX = currentCenter.x - (initialCx - initialPanX) * (newScale / initialScale)
    let newY = currentCenter.y - (initialCy - initialPanY) * (newScale / initialScale)
    
    const clamped = clampPan(newX, newY, newScale)
    panX.value = clamped.x
    panY.value = clamped.y
    zoomScale.value = newScale
  } else if (e.touches.length === 1 && zoomScale.value > 1 && !isPinching.value) {
    const currentCenter = getCenter(e.touches)
    const dx = currentCenter.x - initialCx
    const dy = currentCenter.y - initialCy
    
    let newX = initialPanX + dx
    let newY = initialPanY + dy
    
    const clamped = clampPan(newX, newY, zoomScale.value)
    panX.value = clamped.x
    panY.value = clamped.y
  }
}

function handleTouchCancel(e) {
  cancelPress(e)
  isPinching.value = false
}

function handleTouchEnd(e) {
  handlePressEnd(e)
  if (e.touches.length < 2) {
    isPinching.value = false
  }
  if (e.touches.length === 1 && zoomScale.value > 1) {
    const center = getCenter(e.touches)
    initialCx = center.x
    initialCy = center.y
    initialPanX = panX.value
    initialPanY = panY.value
  }
}

let resizeObserver = null
let windowResizeTimeout = null

/**
 * The canvas a frame is actually drawn into, in device pixels.
 *
 * Not the window: in binoview each eye canvas shows the whole frame at roughly
 * half the window's width, so reporting the window would have the server send
 * twice the pixels either eye can display and leave the GPU to minify the rest
 * away — which is exactly the averaging this reporting exists to reclaim.
 */
function displayedCanvasSize() {
  const dpr = window.devicePixelRatio || 1
  const canvas = isBinoview.value ? canvasLeftRef.value : canvasSingleRef.value
  const width = canvas?.offsetWidth || window.innerWidth
  const height = canvas?.offsetHeight || window.innerHeight
  return {width: width * dpr, height: height * dpr}
}

/**
 * Report the canvas size to the server.
 *
 * Deliberately not memoised here: `sendResolution` remembers the last viewport
 * and replays it on reconnect, so a local "same size, skip it" guard would
 * suppress exactly the report a fresh socket needs.
 */
function reportViewportResolution() {
  const {width, height} = displayedCanvasSize()
  sendResolution(width, height)
}

function handleWindowResize() {
  if (windowResizeTimeout) clearTimeout(windowResizeTimeout)
  windowResizeTimeout = setTimeout(() => {
    reportViewportResolution()
    // Fullscreen only, for the same reason as the live view: a windowed resize
    // must not throw away a zoom the user set deliberately.
    if (isFullscreen.value) resetZoom()
  }, 200)
}

function updateBounds() {
  if (isBinoview.value) {
    if (leftEyeRef.value && canvasLeftRef.value) {
      leftEyeBounds.value = {
        left: canvasLeftRef.value.offsetLeft,
        top: canvasLeftRef.value.offsetTop,
        width: canvasLeftRef.value.offsetWidth,
        height: canvasLeftRef.value.offsetHeight,
      }
    }
    if (rightEyeRef.value && canvasRightRef.value) {
      rightEyeBounds.value = {
        left: canvasRightRef.value.offsetLeft,
        top: canvasRightRef.value.offsetTop,
        width: canvasRightRef.value.offsetWidth,
        height: canvasRightRef.value.offsetHeight,
      }
    }
  } else {
    if (singleEyeRef.value && canvasSingleRef.value) {
      singleViewBounds.value = {
        left: canvasSingleRef.value.offsetLeft,
        top: canvasSingleRef.value.offsetTop,
        width: canvasSingleRef.value.offsetWidth,
        height: canvasSingleRef.value.offsetHeight,
      }
    }
  }
  // Runs on mount, on every layout change the ResizeObserver sees, and on the
  // first frame — so a binoview toggle re-reports without its own watcher.
  reportViewportResolution()
}

watch(hasFrame, (newVal) => {
  if (newVal) setTimeout(updateBounds, 10)
})

onMounted(() => {
  window.addEventListener('resize', handleWindowResize)
  // Not redundant with `resize`: mobile Safari rotates without always firing one.
  window.addEventListener('orientationchange', handleWindowResize)
  document.addEventListener('fullscreenchange', handleFullscreenChange)
  initRenderer()
  updateBounds()
  resizeObserver = new ResizeObserver(updateBounds)
  if (leftEyeRef.value) resizeObserver.observe(leftEyeRef.value)
  if (rightEyeRef.value) resizeObserver.observe(rightEyeRef.value)
  if (singleEyeRef.value) resizeObserver.observe(singleEyeRef.value)
})

onUnmounted(() => {
  window.removeEventListener('resize', handleWindowResize)
  window.removeEventListener('orientationchange', handleWindowResize)
  document.removeEventListener('fullscreenchange', handleFullscreenChange)
  if (windowResizeTimeout) clearTimeout(windowResizeTimeout)
  if (errorTimer) clearTimeout(errorTimer)
  cleanupRenderer()
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
})
</script>

<template>
  <div
    ref="rootRef"
    class="eyepiece-view"
    :class="{ 'zoom-active': isZoomAllowed && zoomScale > 1 }"
    @wheel="handleWheel"
    @mousedown="handlePressStart"
    @mouseup="handlePressEnd"
    @touchstart="handleTouchStart"
    @touchmove="handleTouchMove"
    @touchend="handleTouchEnd"
    @touchcancel="handleTouchCancel"
    @focusin="showOverlay"
  >
    <div v-show="!effectiveHasFrame" class="placeholder">
      <p v-if="!connected">Connecting to stream...</p>
      <template v-else>
        <img v-if="showFocusImage || forceFocusImageNow" src="../assets/focusing-star-black-white.png" class="focus-image" alt="Focusing Star" />
        <p v-else>Waiting for frames...</p>
      </template>
    </div>

    <div v-show="effectiveHasFrame" class="zoom-wrapper" :style="zoomStyle">
      <div v-show="isBinoview" class="binoview-container">
        <div ref="leftEyeRef" class="eye left-eye">
          <canvas ref="canvasLeftRef" :class="['live-canvas', {circular: isCircularView}]"></canvas>
          <GuideArrow
            v-if="showGuideArrow"
            :angle-deg="pushDirection.angleDeg"
            :distance-deg="pushDirection.distanceDeg"
            :is-close="pushDirection.isClose"
            :direction-hint="pushDirection.directionHint"
            :image-left="leftEyeBounds.left"
            :image-top="leftEyeBounds.top"
            :image-width="leftEyeBounds.width"
            :image-height="leftEyeBounds.height"
            :fov-deg="pushDirection.fovDeg || 0"
            :is-circular="isCircularView"
          />
        </div>
        <div ref="rightEyeRef" class="eye right-eye">
          <canvas ref="canvasRightRef" :class="['live-canvas', {circular: isCircularView}]"></canvas>
          <GuideArrow
            v-if="showGuideArrow"
            :angle-deg="pushDirection.angleDeg"
            :distance-deg="pushDirection.distanceDeg"
            :is-close="pushDirection.isClose"
            :direction-hint="pushDirection.directionHint"
            :image-left="rightEyeBounds.left"
            :image-top="rightEyeBounds.top"
            :image-width="rightEyeBounds.width"
            :image-height="rightEyeBounds.height"
            :fov-deg="pushDirection.fovDeg || 0"
            :is-circular="isCircularView"
          />
        </div>
      </div>

      <div v-show="!isBinoview" ref="singleEyeRef" class="single-view">
        <canvas ref="canvasSingleRef" :class="['live-canvas', {circular: isCircularView}]"></canvas>
        <GuideArrow
          v-if="showGuideArrow"
          :angle-deg="pushDirection.angleDeg"
          :distance-deg="pushDirection.distanceDeg"
          :is-close="pushDirection.isClose"
          :direction-hint="pushDirection.directionHint"
          :image-left="singleViewBounds.left"
          :image-top="singleViewBounds.top"
          :image-width="singleViewBounds.width"
          :image-height="singleViewBounds.height"
          :fov-deg="pushDirection.fovDeg || 0"
          :is-circular="isCircularView"
        />
      </div>
    </div>

    <!-- One bottom-right stack so the buttons sit above the backend readout instead
         of on top of it. Everything here fades together; the Push-To chevrons do not. -->
    <div
        class="eyepiece-overlay overlay-fade"
        :class="{ 'overlay-hidden': !overlayVisible }"
        data-overlay-control
    >
      <div class="eyepiece-controls">
        <span v-if="downloadError" class="download-error">{{ downloadError }}</span>

        <BaseSplitButton
            class="download-btn"
            variant="secondary"
            label="Download"
            menu-label="More download options"
            :disabled="!effectiveHasFrame || downloading"
            :options="[{ value: 'original', label: 'Download original', disabled: !effectiveHasFrame || downloading }]"
            @click="downloadSnapshot(true)"
            @select="downloadSnapshot(false)"
            @menu-toggle="menuOpen = $event"
        >
          <BaseSpinner v-if="downloading" size="xs" light />
          <svg v-else xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4M7 10l5 5 5-5M12 15V3" />
          </svg>
        </BaseSplitButton>

        <button v-if="canFullscreen" class="eyepiece-btn" :title="isFullscreen ? 'Exit fullscreen' : 'Fullscreen'" @click="toggleFullscreen">
          <svg v-if="!isFullscreen" xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M15 3h6v6M9 21H3v-6M21 15v6h-6M3 9V3h6" />
          </svg>
          <svg v-else xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4 14h6v6M20 10h-6V4M14 10l7-7M10 14l-7 7" />
          </svg>
        </button>

        <button v-if="isZoomAllowed && zoomScale > 1" class="eyepiece-btn fit-all-btn" @click="resetZoom">
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7M21 21v-6h-6M3 3v6h6M21 21l-7-7M3 3l7 7" />
          </svg>
          Fit all
        </button>
      </div>

      <div v-if="capabilities?.debug_logging && effectiveHasFrame" class="debug-overlay">
        {{ backendLabel }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.eyepiece-view {
  width: 100vw;
  height: 100vh;
  background: black;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}

.eyepiece-view.zoom-active {
  touch-action: none;
}

.zoom-wrapper {
  width: 100%;
  height: 100%;
  will-change: transform;
}

/* Column, so the buttons stack above the backend readout rather than over it.
   With debug logging off the readout is absent and the buttons keep the corner. */
.eyepiece-overlay {
  position: absolute;
  bottom: 1rem;
  right: 1rem;
  z-index: 10;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 0.5rem;
}

.eyepiece-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.eyepiece-btn {
  background: rgba(30, 30, 30, 0.8);
  backdrop-filter: blur(4px);
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 8px;
  color: #fff;
  padding: 0.5rem 1rem;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.9rem;
  cursor: pointer;
  transition: background 0.2s;
}

.eyepiece-btn:hover {
  background: rgba(50, 50, 50, 0.9);
}

.eyepiece-btn:active {
  background: rgba(70, 70, 70, 0.9);
}

/* The split button's own halves, styled to match the plain buttons beside it. */
.download-btn :deep(.btn) {
  background: rgba(30, 30, 30, 0.8);
  backdrop-filter: blur(4px);
  border: 1px solid rgba(255, 255, 255, 0.2);
  color: #fff;
  padding: 0.5rem 0.75rem;
}

.download-btn :deep(.btn:hover:not(:disabled)) {
  background: rgba(50, 50, 50, 0.9);
}

.download-btn :deep(.btn:disabled) {
  opacity: 0.45;
  cursor: not-allowed;
}

.download-btn :deep(.split-button-main) {
  border-radius: 8px 0 0 8px;
}

.download-btn :deep(.split-button-toggle) {
  border-radius: 0 8px 8px 0;
  border-left: 1px solid rgba(255, 255, 255, 0.2);
}

.download-error {
  background: rgba(30, 30, 30, 0.85);
  border: 1px solid var(--error, #ef4444);
  border-radius: 8px;
  color: #fff;
  font-size: 0.8rem;
  padding: 0.4rem 0.6rem;
}

.overlay-fade {
  transition: opacity 0.35s ease;
}

.overlay-hidden {
  opacity: 0;
  pointer-events: none;
}

.placeholder {
  color: #fff;
  font-family: sans-serif;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.focus-image {
  max-width: 80%;
  max-height: 80vh;
  object-fit: contain;
  opacity: 0.8;
}

.binoview-container {
  display: flex;
  width: 100%;
  height: 100%;
}

.eye {
  position: relative;
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  border-right: 1px solid #333;
  container-type: size;
}

.eye:last-child {
  border-right: none;
}

.single-view {
  position: relative;
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  container-type: size;
}

.live-canvas {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}

.live-canvas.circular {
  clip-path: circle(closest-side at 50% 50%);
  width: 100cqmin;
  height: 100cqmin;
  object-fit: cover;
}

.debug-overlay {
  background: rgba(0, 0, 0, 0.6);
  color: #fff;
  font-family: monospace;
  font-size: 0.75rem;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  pointer-events: none;
}
</style>
