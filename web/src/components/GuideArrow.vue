<script setup lang="ts">
import {computed, getCurrentInstance} from 'vue'
import type {CSSProperties} from 'vue'
import {formatAngle} from '../utils/format.js'
import {
  ARROW_ASPECT,
  arrowScale as scaleFor,
  baseSize,
  edgeOffset,
  formatScreens,
  infoOffset,
  isOffScreen,
  screensAway,
} from '../utils/guideArrowLayout.js'
import type {ClientPushDirection} from '../composables/events/pushToEvents.js'
import type {ScreenRect} from '../composables/useCometRoi.js'

const props = withDefaults(defineProps<{
  /** `pushDirection` from the event stream. */
  direction: ClientPushDirection
  /** Where the image sits in the arrow's positioned parent, in CSS pixels. */
  bounds?: ScreenRect
  fovDeg?: number
  isCircular?: boolean
  /**
   * Name of the camera the angle was solved on, when it is not the one on screen: the
   * chevron would carry that camera's rotation, so only distance and compass point show.
   */
  otherCamera?: string
}>(), {
  bounds: () => ({left: 0, top: 0, width: 400, height: 300}),
  fovDeg: 0,
  isCircular: false,
  otherCamera: '',
})

const angleDeg = computed(() => props.direction.angleDeg)
const distanceDeg = computed(() => props.direction.distanceDeg)
const directionHint = computed(() => props.direction.directionHint)

const uid = getCurrentInstance()?.uid || Math.random().toString(36).substring(2, 9)
const gradientId = `chevronGradient-${uid}`

const screenSizes = computed(() => screensAway(distanceDeg.value, props.fovDeg))
const offScreen = computed(() => isOffScreen(screenSizes.value))
const arrowScale = computed(() => scaleFor(distanceDeg.value, screenSizes.value))
const responsiveBaseSize = computed(() => baseSize(props.bounds.width, props.bounds.height, offScreen.value))
const arrowSize = computed(() => responsiveBaseSize.value * arrowScale.value)

const isOnTarget = computed(() => {
  return distanceDeg.value < 0.1 || directionHint.value === 'OK'
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
</script>

<template>
  <div class="guide-arrow-container" :style="containerStyle">
    <div v-if="isOnTarget" class="on-target">
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
      <span class="on-target-label">On Target</span>
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

      <div class="hud-chip distance-info" :class="{centered: otherCamera}" :style="distanceInfoStyle">
        <span class="distance">{{ formatAngle(distanceDeg) }}</span>
        <span v-if="screensLabel" class="screen-size">{{ screensLabel }}</span>
        <span class="hint">{{ directionHint }}</span>
        <span v-if="otherCamera" class="other-camera">Arrow on {{ otherCamera }}</span>
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

.distance-info {
  position: absolute;
  transform: translate(-50%, -50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.15rem;
  padding: 0.45rem 0.9rem;
  opacity: 0.8;
  white-space: nowrap;
}

.distance {
  font-size: 1.2rem;
  font-weight: 600;
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.screen-size {
  font-size: var(--fs-lg);
  font-weight: 500;
  color: var(--primary);
  font-family: var(--font-mono);
}

.hint {
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

/* No chevron to sit behind: the box itself marks the centre of the view. */
.distance-info.centered {
  position: static;
  transform: none;
}

.other-camera {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
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
