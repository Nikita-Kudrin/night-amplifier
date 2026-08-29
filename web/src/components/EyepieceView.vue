<script setup>
import {ref, computed, inject, onMounted, onUnmounted, watch} from 'vue'
import {useImageStream} from '../composables/useWebSocket.js'
import {useWebGLRenderer} from '../composables/useWebGLRenderer.js'
import {useCanvas2DRenderer} from '../composables/useCanvas2DRenderer.js'
import {getAppState} from '../composables/useAppState.js'
import GuideArrow from './GuideArrow.vue'

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

const isBinoview = computed(() => settings.value?.eyepiece?.binoview ?? true)
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

function handleWheel(e) {
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

function handleTouchEnd(e) {
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

let lastReported = {width: 0, height: 0}

function reportViewportResolution() {
  const {width, height} = displayedCanvasSize()
  if (width === lastReported.width && height === lastReported.height) return
  lastReported = {width, height}
  sendResolution(width, height)
}

function handleWindowResize() {
  if (windowResizeTimeout) clearTimeout(windowResizeTimeout)
  windowResizeTimeout = setTimeout(reportViewportResolution, 200)
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
  initRenderer()
  updateBounds()
  resizeObserver = new ResizeObserver(updateBounds)
  if (leftEyeRef.value) resizeObserver.observe(leftEyeRef.value)
  if (rightEyeRef.value) resizeObserver.observe(rightEyeRef.value)
  if (singleEyeRef.value) resizeObserver.observe(singleEyeRef.value)
})

onUnmounted(() => {
  window.removeEventListener('resize', handleWindowResize)
  if (windowResizeTimeout) clearTimeout(windowResizeTimeout)
  cleanupRenderer()
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
})
</script>

<template>
  <div 
    class="eyepiece-view" 
    :class="{ 'zoom-active': isZoomAllowed && zoomScale > 1 }"
    @wheel="handleWheel" 
    @touchstart="handleTouchStart" 
    @touchmove="handleTouchMove" 
    @touchend="handleTouchEnd" 
    @touchcancel="handleTouchEnd"
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

    <div v-if="capabilities?.debug_logging && effectiveHasFrame" class="debug-overlay">
      {{ backendLabel }}
    </div>

    <button v-if="isZoomAllowed && zoomScale > 1" class="fit-all-btn" @click="resetZoom">
      <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7M21 21v-6h-6M3 3v6h6M21 21l-7-7M3 3l7 7" />
      </svg>
      Fit all
    </button>
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

.fit-all-btn {
  position: absolute;
  bottom: 1rem;
  right: 1rem;
  z-index: 10;
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

.fit-all-btn:hover {
  background: rgba(50, 50, 50, 0.9);
}

.fit-all-btn:active {
  background: rgba(70, 70, 70, 0.9);
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
  position: absolute;
  bottom: 0.5rem;
  right: 0.5rem;
  background: rgba(0, 0, 0, 0.6);
  color: #fff;
  font-family: monospace;
  font-size: 0.75rem;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  pointer-events: none;
}
</style>
