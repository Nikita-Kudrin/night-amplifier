<script setup lang="ts">
import {computed, getCurrentInstance} from 'vue'
import type {CSSProperties} from 'vue'
import {formatAngle} from '../utils/format'
import {
  ARROW_ASPECT,
  arrowScale as scaleFor,
  baseSize,
  edgeOffset,
  formatScreens,
  frameSpan,
  infoOffset,
  isOffScreen,
  screensAway,
  viewSpan,
} from '../utils/guideArrowLayout'
import {solveProgress} from '../composables/events/pushToEvents'
import type {ClientPushDirection, PlateSolving} from '../composables/events/pushToEvents'
import type {OtherSolvedCamera} from '../composables/useDisplayedFov'
import type {ScreenRect} from '../composables/useCometRoi'
import type {FrameSize} from '../composables/useImageStream'
import PushToReadout from './PushToReadout.vue'
import PushToSolving from './PushToSolving.vue'
import SolveProgressBar from './SolveProgressBar.vue'

const props = withDefaults(defineProps<{
  /** `pushDirection` from the event stream; `null` before a target's first solve, drawn only while solving. */
  direction: ClientPushDirection | null
  /**
   * The streamed frame's size, which the screens count is measured on. Not `bounds`: a
   * circular eyepiece's canvas is square whatever the frame, and before the first frame `/`
   * hands over its container, so the count followed the phone's shape.
   */
  frame: FrameSize
  /** Where the image sits in the arrow's positioned parent, in CSS pixels. */
  bounds?: ScreenRect
  fovDeg?: number
  isCircular?: boolean
  /**
   * The camera the angle was solved on, when it is not the one on screen: the chevron
   * would carry that camera's rotation, so only distance and compass point show. It is
   * named only while connected — a disconnected guide is nowhere to look.
   */
  otherCamera?: OtherSolvedCamera | null
  /**
   * Push-To is plate solving: a turning compass stands in for the readout, whose numbers
   * are the last solve's, while the chevrons keep pointing the last known way.
   */
  solving?: boolean
  solveStage?: PlateSolving['stage']
}>(), {
  bounds: () => ({left: 0, top: 0, width: 400, height: 300}),
  fovDeg: 0,
  isCircular: false,
  otherCamera: null,
  solving: false,
  solveStage: null,
})

const angleDeg = computed(() => props.direction?.angleDeg ?? 0)
const distanceDeg = computed(() => props.direction?.distanceDeg ?? 0)
const directionHint = computed(() => props.direction?.directionHint ?? null)
// The last solve failed: the readout is the fix before it, dimmed and named so.
const stale = computed(() => props.direction?.stale ?? false)

const uid = getCurrentInstance()?.uid || Math.random().toString(36).substring(2, 9)
const gradientId = `chevronGradient-${uid}`

const frameAspect = computed(() => props.frame.width / props.frame.height)
// The readout counts frames, the same on every page; placement follows what this view shows.
// Another camera's angle says nothing about which way this view's frame runs.
const screenSizes = computed(() => screensAway(
    distanceDeg.value, props.fovDeg, props.otherCamera ? 1 : frameSpan(angleDeg.value, frameAspect.value)))
const viewScreens = computed(() => screensAway(
    distanceDeg.value, props.fovDeg, props.otherCamera ? 1 : viewSpan(angleDeg.value, frameAspect.value, props.isCircular)))
const offScreen = computed(() => isOffScreen(viewScreens.value))
const arrowScale = computed(() => scaleFor(distanceDeg.value, viewScreens.value))
const responsiveBaseSize = computed(() => baseSize(props.bounds.width, props.bounds.height, offScreen.value))
const arrowSize = computed(() => responsiveBaseSize.value * arrowScale.value)

const isOnTarget = computed(() => {
  return props.direction !== null && (distanceDeg.value < 0.1 || directionHint.value === 'OK')
})

const containerStyle = computed((): CSSProperties => {
  const pinned = offScreen.value && !isOnTarget.value && !props.otherCamera
  const offset = pinned
      ? edgeOffset({
        angleDeg: angleDeg.value,
        arrowSize: arrowSize.value,
        imageWidth: props.bounds.width,
        imageHeight: props.bounds.height,
        isCircular: props.isCircular,
      })
      : {x: 0, y: 0}
  const {left, top, width, height} = props.bounds
  return {
    position: 'absolute',
    top: `${top + height / 2 + offset.y}px`,
    left: `${left + width / 2 + offset.x}px`,
    transform: 'translate(-50%, -50%)',
  }
})

const rotationStyle = computed(() => {
  return {
    transform: `rotate(${angleDeg.value}deg)`,
  }
})

const distanceInfoStyle = computed(() => {
  if (props.otherCamera) return {}
  const offset = infoOffset(angleDeg.value, arrowSize.value)
  return {
    top: `calc(50% + ${offset.y}px)`,
    left: `calc(50% + ${offset.x}px)`,
  }
})

const CHEVRON_COUNT = 5

const chevrons = computed(() => {
  const base = arrowScale.value
  const baseOpacity = 0.8
  const result = []

  for (let i = 0; i < CHEVRON_COUNT; i++) {
    const progress = i / (CHEVRON_COUNT - 1)
    result.push({
      strokeWidth: 2 + base * 2 + progress * 2,
      opacity: baseOpacity * (0.4 + progress * 0.5 + base * 0.1),
      scale: 0.6 + base * 0.2 + progress * 0.4,
      yOffset: i * 25,
    })
  }

  return result
})

const screensLabel = computed(() => formatScreens(screenSizes.value))

// One readout, drawn as the chip or, while solving, unseen under the compass to size it.
const readout = computed(() => ({
  distance: formatAngle(distanceDeg.value),
  screens: screensLabel.value,
  hint: directionHint.value,
  stale: stale.value,
  otherCameraName: props.otherCamera?.connected ? props.otherCamera.name : null,
}))
</script>

<template>
  <div class="guide-arrow-container" :style="containerStyle">
    <template v-if="!direction">
      <PushToSolving v-if="solving" :stage="solveStage"/>
    </template>

    <div v-else-if="isOnTarget" class="on-target">
      <svg viewBox="0 0 100 100" :width="responsiveBaseSize * 0.8" :height="responsiveBaseSize * 0.8">
        <circle
            cx="50"
            cy="50"
            r="35"
            fill="none"
            stroke="var(--success)"
            stroke-width="3"
            opacity="0.8"
        />
        <path
            d="M35 50 L45 60 L65 40"
            fill="none"
            stroke="var(--success)"
            stroke-width="4"
            stroke-linecap="round"
            stroke-linejoin="round"
        />
      </svg>
      <!-- The step goes under the label: the status bar no longer names it. -->
      <div class="on-target-caption">
        <span class="on-target-label">On Target</span>
        <div v-if="solving && solveProgress(solveStage)" class="on-target-solve" role="status" aria-label="Plate solving">
          <SolveProgressBar :stage="solveStage"/>
        </div>
      </div>
    </div>

    <template v-else>
      <div v-if="!otherCamera" class="arrow-wrapper" :style="rotationStyle">
        <svg
            viewBox="0 0 100 160" :width="responsiveBaseSize * arrowScale"
            :height="responsiveBaseSize * ARROW_ASPECT * arrowScale">
          <defs>
            <linearGradient :id="gradientId" x1="0%" y1="100%" x2="0%" y2="0%">
              <stop offset="0%" stop-color="var(--primary)" stop-opacity="0.32"/>
              <stop offset="100%" stop-color="var(--primary)" stop-opacity="0.8"/>
            </linearGradient>
          </defs>

          <g transform="translate(50, 130)">
            <path
                v-for="(chevron, index) in chevrons"
                :key="index"
                :d="`M${-25 * chevron.scale} ${-chevron.yOffset} L0 ${-chevron.yOffset - 20 * chevron.scale} L${25 * chevron.scale} ${-chevron.yOffset}`"
                fill="none"
                :stroke="`url(#${gradientId})`"
                :stroke-width="chevron.strokeWidth"
                stroke-linecap="round"
                stroke-linejoin="round"
                :opacity="chevron.opacity"
            />
          </g>
        </svg>
      </div>

      <PushToSolving
          v-if="solving"
          class="distance-info"
          :class="{centered: otherCamera}"
          :style="distanceInfoStyle"
          :stage="solveStage"
      >
        <PushToReadout v-bind="readout"/>
      </PushToSolving>
      <div v-else class="hud-chip distance-info readout-chip" :class="{centered: otherCamera, stale}" :style="distanceInfoStyle">
        <PushToReadout v-bind="readout"/>
      </div>
    </template>
  </div>
</template>

<style scoped>
.guide-arrow-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  pointer-events: none;
  z-index: var(--z-hud);
}

.on-target {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  animation: pulse-success 2s ease-in-out infinite;
}

/* As wide as the label, which the bar spans. */
.on-target-caption {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}

.on-target-solve {
  height: 4px;
}

.on-target-label {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--success);
  text-shadow: 0 1px 4px rgba(0, 0, 0, 0.8);
}

@keyframes pulse-success {
  0%,
  100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.7;
    transform: scale(0.95);
  }
}

.arrow-wrapper {
  display: flex;
  flex-direction: column;
  justify-content: flex-start;
  align-items: center;
  filter: drop-shadow(0 2px 8px rgba(0, 0, 0, 0.5));
  animation: pulse-arrow 1.5s ease-in-out infinite;
}

@keyframes pulse-arrow {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.7;
  }
}

/* Placement only: the solving chip lays itself out (a grid), and this rule comes later. */
.distance-info {
  position: absolute;
  transform: translate(-50%, -50%);
}

.readout-chip {
  padding: 0.45rem 0.9rem;
  opacity: 0.8;
}

/* The numbers of a fix the last solve could not confirm. */
.distance-info.stale {
  opacity: 0.5;
}

/* No chevron to sit behind: the box itself marks the centre of the view. */
.distance-info.centered {
  position: static;
  transform: none;
}

@media (max-width: 768px) {
  .distance-info {
    transform: translate(-50%, -50%) scale(0.5);
  }

  .distance-info.centered {
    transform: scale(0.5);
  }
}
</style>
