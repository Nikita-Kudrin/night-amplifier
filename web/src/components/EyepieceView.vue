<script setup>
import {ref, computed, inject, onMounted, onUnmounted, watch} from 'vue'
import {useImageStream} from '../composables/useWebSocket.js'
import {useWebGLRenderer} from '../composables/useWebGLRenderer.js'
import {useCanvas2DRenderer} from '../composables/useCanvas2DRenderer.js'

const settings = inject('settings')

const {connected, frameData, dimensions} = useImageStream()

const canvasLeftRef = ref(null)
const canvasRightRef = ref(null)
const canvasSingleRef = ref(null)

const webglLeft = useWebGLRenderer()
const canvas2dLeft = useCanvas2DRenderer()

const webglRight = useWebGLRenderer()
const canvas2dRight = useCanvas2DRenderer()

const webglSingle = useWebGLRenderer()
const canvas2dSingle = useCanvas2DRenderer()

const isBinoview = computed(() => settings.value?.eyepiece?.binoview ?? true)

const hasFrame = computed(() => frameData.value !== null && dimensions.value.width > 0)

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

onMounted(() => {
  initRenderer()
})

onUnmounted(() => {
  cleanupRenderer()
})
</script>

<template>
  <div class="eyepiece-view">
    <div v-show="!hasFrame" class="placeholder">
      <p v-if="!connected">Connecting to stream...</p>
      <p v-else>Waiting for frames...</p>
    </div>

    <div v-show="hasFrame && isBinoview" class="binoview-container">
      <div class="eye left-eye">
        <canvas ref="canvasLeftRef" class="live-canvas"></canvas>
      </div>
      <div class="eye right-eye">
        <canvas ref="canvasRightRef" class="live-canvas"></canvas>
      </div>
    </div>

    <div v-show="hasFrame && !isBinoview" class="single-view">
      <canvas ref="canvasSingleRef" class="live-canvas"></canvas>
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
}

.binoview-container {
  display: flex;
  width: 100%;
  height: 100%;
}

.eye {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  border-right: 1px solid #333;
}

.eye:last-child {
  border-right: none;
}

.single-view {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

.live-canvas {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}
</style>
