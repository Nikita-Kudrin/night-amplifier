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
const {connected, frameData, dimensions, isJpeg} = useImageStream({ endpoint })

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

const hasFrame = computed(() => frameData.value !== null && dimensions.value.width > 0)

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
  const labels = {
    jpeg: 'Dynamic JPEG (SIMD)',
    'webgl2-16bit': 'WebGL2 16-bit',
    'webgl2-8bit': 'WebGL2 8-bit',
    webgl1: 'WebGL1',
    canvas2d: 'Canvas 2D',
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

let resizeObserver = null

function updateBounds() {
  if (isBinoview.value) {
    if (leftEyeRef.value && canvasLeftRef.value) {
      const containerRect = leftEyeRef.value.getBoundingClientRect()
      const canvasRect = canvasLeftRef.value.getBoundingClientRect()
      leftEyeBounds.value = {
        left: canvasRect.left - containerRect.left,
        top: canvasRect.top - containerRect.top,
        width: canvasRect.width,
        height: canvasRect.height,
      }
    }
    if (rightEyeRef.value && canvasRightRef.value) {
      const containerRect = rightEyeRef.value.getBoundingClientRect()
      const canvasRect = canvasRightRef.value.getBoundingClientRect()
      rightEyeBounds.value = {
        left: canvasRect.left - containerRect.left,
        top: canvasRect.top - containerRect.top,
        width: canvasRect.width,
        height: canvasRect.height,
      }
    }
  } else {
    if (singleEyeRef.value && canvasSingleRef.value) {
      const containerRect = singleEyeRef.value.getBoundingClientRect()
      const canvasRect = canvasSingleRef.value.getBoundingClientRect()
      singleViewBounds.value = {
        left: canvasRect.left - containerRect.left,
        top: canvasRect.top - containerRect.top,
        width: canvasRect.width,
        height: canvasRect.height,
      }
    }
  }
}

watch(hasFrame, (newVal) => {
  if (newVal) setTimeout(updateBounds, 10)
})

onMounted(() => {
  initRenderer()
  updateBounds()
  resizeObserver = new ResizeObserver(updateBounds)
  if (leftEyeRef.value) resizeObserver.observe(leftEyeRef.value)
  if (rightEyeRef.value) resizeObserver.observe(rightEyeRef.value)
  if (singleEyeRef.value) resizeObserver.observe(singleEyeRef.value)
})

onUnmounted(() => {
  cleanupRenderer()
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
})
</script>

<template>
  <div class="eyepiece-view">
    <div v-show="!hasFrame" class="placeholder">
      <p v-if="!connected">Connecting to stream...</p>
      <template v-else>
        <img v-if="showFocusImage" src="../assets/focusing-star-black-white.png" class="focus-image" alt="Focusing Star" />
        <p v-else>Waiting for frames...</p>
      </template>
    </div>

    <div v-show="hasFrame && isBinoview" class="binoview-container">
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

    <div v-show="hasFrame && !isBinoview" ref="singleEyeRef" class="single-view">
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

    <div v-if="capabilities?.debug_logging && hasFrame" class="debug-overlay">
      {{ backendLabel }}
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
