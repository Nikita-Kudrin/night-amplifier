<script setup>
import {ref, computed, inject, onMounted, onUnmounted, watch} from 'vue'
import {useImageStream} from '../composables/useImageStream.js'
import {useFrameRenderer} from '../composables/useFrameRenderer.js'
import {useEventListener} from '../composables/useEventListener.js'
import {useFullscreen} from '../composables/useFullscreen.js'
import {useOverlayVisibility} from '../composables/useOverlayVisibility.js'
import {useEyepieceZoom} from '../composables/useEyepieceZoom.js'
import {useTimeouts} from '../composables/useTimeouts.js'
import {useCapabilities} from '../composables/appContext.js'
import {fetchEyepieceSnapshot} from '../composables/api.js'
import {useDisplayedFov, useOtherSolvedCamera} from '../composables/useDisplayedFov.js'
import {saveBlob} from '../utils/saveBlob.js'
import {offsetBounds} from '../utils/viewGeometry.js'
import {eyepieceStreamEndpoint, PAGES} from '../constants'
import GuideArrow from './GuideArrow.vue'
import {BaseSpinner, BaseSplitButton} from './ui'

const eventStream = inject('eventStream')
const settings = inject('settings')
const {debugLogging} = useCapabilities()
const {later} = useTimeouts()

const mainCamera = inject('mainCamera', computed(() => null))
const guideCamera = inject('guideCamera', computed(() => null))

const routePath = window.location.pathname
// No source in the URL: the server streams whichever camera the operator views on `/`,
// switching this socket in place.
const endpoint = eyepieceStreamEndpoint(routePath)
const {connected, frameData, dimensions, isJpeg} = useImageStream({endpoint})


const pushDirection = computed(() => eventStream?.pushDirection?.value ?? null)
const currentTarget = computed(() => eventStream?.currentTarget?.value ?? null)
const showGuideArrow = computed(() => currentTarget.value !== null && pushDirection.value !== null)

const showsGuideCamera = computed(() => eventStream?.viewedCamera?.value === 'guide')
const displayedCamera = computed(() => (showsGuideCamera.value ? guideCamera.value : mainCamera.value))
const displayedFovDeg = useDisplayedFov(displayedCamera, settings, pushDirection)
const arrowCamera = useOtherSolvedCamera(displayedCamera, pushDirection)

/**
 * `/eyepiece` is monocular whatever the Binoview setting says — it is the view an
 * observer puts their eye to. `/eyepiece_quality` still honours the setting.
 */
const isBinoview = computed(() => {
  if (routePath === PAGES.EYEPIECE) return false
  return settings.value?.eyepiece?.binoview ?? true
})
const isCircularView = computed(() => settings.value?.eyepiece?.circular_view ?? true)
const showFocusImage = computed(() => settings.value?.show_focus_image ?? false)
const forceFocusImageNow = computed(() => settings.value?.force_focus_image_now ?? false)

const hasFrame = computed(() => frameData.value !== null && dimensions.value.width > 0)
const effectiveHasFrame = computed(() => hasFrame.value && !forceFocusImageNow.value)

/** The eyes on screen, by class name: each one is a canvas drawn from the same frame. */
const BINOVIEW_EYES = ['left-eye', 'right-eye']
const SINGLE_EYES = ['single-view']
const eyes = computed(() => (isBinoview.value ? BINOVIEW_EYES : SINGLE_EYES))

const eyeElements = {}
const canvasElements = {}
const eyeBounds = ref({})
const NO_BOUNDS = Object.freeze({left: 0, top: 0, width: 0, height: 0})

let resizeObserver = null

function setEye(eye, element) {
  const previous = eyeElements[eye]
  if (previous === element) return
  if (previous) resizeObserver?.unobserve(previous)
  if (element) {
    eyeElements[eye] = element
    resizeObserver?.observe(element)
  } else {
    delete eyeElements[eye]
  }
}

function setCanvas(eye, element) {
  if (element) canvasElements[eye] = element
  else delete canvasElements[eye]
}

function updateBounds() {
  const bounds = {}
  for (const eye of eyes.value) {
    const canvas = canvasElements[eye]
    if (canvas) bounds[eye] = offsetBounds(canvas)
  }
  eyeBounds.value = bounds
}

// Off the circular view the canvas takes the frame's own aspect, so a new size moves it
// inside an eye that stays put, and the eye's ResizeObserver never fires.
const {label: rendererLabel, redraw} = useFrameRenderer({
  frameData,
  dimensions,
  isJpeg,
  canvases: () => eyes.value.map((eye) => canvasElements[eye]),
  onSizeChange: updateBounds,
})

const backendLabel = computed(() =>
  showsGuideCamera.value ? `${rendererLabel.value} · Guide camera` : rendererLabel.value
)

// Switching layout mounts new canvases: draw the held frame on them and measure them.
watch(eyes, () => {
  redraw()
  updateBounds()
}, {flush: 'post'})

const isZoomAllowed = computed(() => routePath === PAGES.EYEPIECE)
const zoom = useEyepieceZoom(() => isZoomAllowed.value && effectiveHasFrame.value)
const zoomScale = zoom.scale
const zoomStyle = zoom.style
const resetZoom = zoom.reset

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
 * image; without it, the uncropped stretched result.
 *
 * The server names the file (it owns the timestamp); the name travels with the
 * bytes since the fetch's blob carries none of the headers it arrived with.
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
  zoom.wheel(e)
}

function handleTouchStart(e) {
  handlePressStart(e)
  zoom.touchStart(e)
}

function handleTouchMove(e) {
  showOverlay()
  zoom.touchMove(e)
}

function handleTouchCancel(e) {
  cancelPress(e)
  zoom.touchCancel()
}

function handleTouchEnd(e) {
  handlePressEnd(e)
  zoom.touchEnd(e)
}

watch(hasFrame, (newVal) => {
  if (newVal) later(updateBounds, 10)
})

// Fullscreen only, for the same reason as the live view: a windowed resize must not
// throw away a zoom the user set deliberately. Rotation is listened to three ways, as
// the live view does: Safari before 16.4 has no `screen.orientation`, and mobile Safari
// rotates without always firing a `resize`.
function handleWindowResize() {
  if (isFullscreen.value) resetZoom()
}

useEventListener(window, 'resize', handleWindowResize)
useEventListener(window, 'orientationchange', handleWindowResize)
useEventListener(window.screen.orientation, 'change', handleWindowResize)
useEventListener(document, 'fullscreenchange', handleFullscreenChange)

onMounted(() => {
  updateBounds()
  resizeObserver = new ResizeObserver(updateBounds)
  Object.values(eyeElements).forEach((element) => resizeObserver.observe(element))
})

onUnmounted(() => {
  if (errorTimer) clearTimeout(errorTimer)
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
      <div class="eyes" :class="{ 'binoview-container': isBinoview }">
        <div
            v-for="eye in eyes"
            :key="eye"
            :ref="(element) => setEye(eye, element)"
            :class="['eye', eye]"
        >
          <canvas
              :ref="(element) => setCanvas(eye, element)"
              :class="['live-canvas', {circular: isCircularView}]"
          ></canvas>
          <GuideArrow
              v-if="showGuideArrow"
              :direction="pushDirection"
              :bounds="eyeBounds[eye] ?? NO_BOUNDS"
              :fov-deg="displayedFovDeg"
              :other-camera="arrowCamera"
              :is-circular="isCircularView"
          />
        </div>
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

      <div v-if="debugLogging && effectiveHasFrame" class="debug-overlay">
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
  z-index: var(--z-overlay);
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
  background: var(--glass-bg);
  backdrop-filter: blur(4px);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-lg);
  color: var(--on-primary);
  padding: 0.5rem 1rem;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: var(--fs-base);
  cursor: pointer;
  transition: background var(--transition-normal);
}

.eyepiece-btn:hover {
  background: var(--glass-bg-hover);
}

.eyepiece-btn:active {
  background: var(--glass-bg-active);
}

/* The split button's own halves, styled to match the plain buttons beside it. */
.download-btn :deep(.btn) {
  background: var(--glass-bg);
  backdrop-filter: blur(4px);
  border: 1px solid var(--glass-border);
  color: var(--on-primary);
  padding: 0.5rem 0.75rem;
}

.download-btn :deep(.btn:hover:not(:disabled)) {
  background: var(--glass-bg-hover);
}

.download-btn :deep(.btn:disabled) {
  opacity: 0.45;
  cursor: not-allowed;
}

.download-btn :deep(.split-button-main) {
  border-radius: var(--radius-lg) 0 0 var(--radius-lg);
}

.download-btn :deep(.split-button-toggle) {
  border-radius: 0 var(--radius-lg) var(--radius-lg) 0;
  border-left: 1px solid var(--glass-border);
}

.download-error {
  background: var(--glass-bg);
  border: 1px solid var(--error);
  border-radius: var(--radius-lg);
  color: var(--on-primary);
  font-size: var(--fs-md);
  padding: 0.4rem 0.6rem;
}

.placeholder {
  color: var(--on-primary);
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

.eyes {
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
  border-right: 1px solid var(--border);
  container-type: size;
}

.eye:last-child {
  border-right: none;
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
  color: var(--on-primary);
  font-family: monospace;
  font-size: var(--fs-sm);
  padding: 0.25rem 0.5rem;
  border-radius: var(--radius-sm);
  pointer-events: none;
}
</style>
