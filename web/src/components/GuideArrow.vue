<script setup>
import {computed} from 'vue'

const props = defineProps({
  angleDeg: {
    type: Number,
    required: true,
  },
  distanceDeg: {
    type: Number,
    required: true,
  },
  isClose: {
    type: Boolean,
    default: false,
  },
  directionHint: {
    type: String,
    default: '',
  },
  imageLeft: {
    type: Number,
    default: 0,
  },
  imageTop: {
    type: Number,
    default: 0,
  },
  imageWidth: {
    type: Number,
    default: 400,
  },
  imageHeight: {
    type: Number,
    default: 300,
  },
  fovDeg: {
    type: Number,
    default: 0,
  },
})

const MAX_DISTANCE = 30
const MIN_SCALE = 0.3
const MAX_SCALE = 1.6

const MIN_BASE_SIZE = 60
const MAX_BASE_SIZE = 240
const MAX_BASE_SIZE_OFF_SCREEN = 290
const REFERENCE_CONTAINER_SIZE = 600

const OFF_SCREEN_THRESHOLD = 0.4
const EDGE_MARGIN_PERCENT = 0.10

// Quadratic ease-in: near-centre targets get squashed more aggressively than
// a linear mapping would, while near-edge targets reach a larger ceiling.
const EDGE_PROXIMITY_CURVE = 2

const normalizedDistance = computed(() => {
  return Math.min(props.distanceDeg / MAX_DISTANCE, 1.0)
})

const screenSizes = computed(() => {
  if (props.fovDeg <= 0) return null
  return props.distanceDeg / props.fovDeg
})

const isOffScreen = computed(() => {
  if (screenSizes.value === null) return false
  return screenSizes.value > OFF_SCREEN_THRESHOLD
})

// Drive on-screen growth by how close the target is to the frame edge, so
// chevrons visibly lengthen as the target drifts outward. Fall back to the
// linear distance-based signal when FOV is unknown (no plate solve yet).
const edgeProximity = computed(() => {
  if (screenSizes.value === null) return normalizedDistance.value
  const raw = Math.min(screenSizes.value / OFF_SCREEN_THRESHOLD, 1.0)
  return Math.pow(raw, EDGE_PROXIMITY_CURVE)
})

const arrowScale = computed(() => {
  return MIN_SCALE + (MAX_SCALE - MIN_SCALE) * edgeProximity.value
})

const responsiveBaseSize = computed(() => {
  const minDimension = Math.min(props.imageWidth, props.imageHeight)
  const sizeFactor = minDimension / REFERENCE_CONTAINER_SIZE
  const baseSize = 100 * sizeFactor
  const maxSize = isOffScreen.value ? MAX_BASE_SIZE_OFF_SCREEN : MAX_BASE_SIZE
  return Math.max(MIN_BASE_SIZE, Math.min(maxSize, baseSize))
})

const isOnTarget = computed(() => {
  return props.distanceDeg < 0.1 || props.directionHint === 'OnTarget'
})

const containerStyle = computed(() => {
  const imageCenterX = props.imageLeft + props.imageWidth / 2
  const imageCenterY = props.imageTop + props.imageHeight / 2

  if (!isOffScreen.value || isOnTarget.value) {
    return {
      position: 'absolute',
      top: `${imageCenterY}px`,
      left: `${imageCenterX}px`,
      transform: 'translate(-50%, -50%)',
    }
  }

  const angleRad = (props.angleDeg * Math.PI) / 180
  const arrowSize = responsiveBaseSize.value * arrowScale.value
  // After rotation by angleRad, the SVG's axis-aligned bounding box has
  // half-extents W|cos θ| + H|sin θ| (x) and W|sin θ| + H|cos θ| (y).
  const cosA = Math.abs(Math.cos(angleRad))
  const sinA = Math.abs(Math.sin(angleRad))
  const svgHalfW = arrowSize * 0.5
  const svgHalfH = arrowSize * 1.6 * 0.5
  const halfWRot = svgHalfW * cosA + svgHalfH * sinA
  const halfHRot = svgHalfW * sinA + svgHalfH * cosA

  const marginX = props.imageWidth * EDGE_MARGIN_PERCENT
  const marginY = props.imageHeight * EDGE_MARGIN_PERCENT

  // Clamp to 0 so the arrow pins to image centre on viewports where the
  // rotated arrow plus margin already spans the image half-size.
  const maxOffsetX = Math.max(0, props.imageWidth / 2 - halfWRot - marginX)
  const maxOffsetY = Math.max(0, props.imageHeight / 2 - halfHRot - marginY)

  const dirX = Math.sin(angleRad)
  const dirY = -Math.cos(angleRad)

  let offsetX, offsetY

  if (Math.abs(dirX) < 0.001 && Math.abs(dirY) < 0.001) {
    offsetX = 0
    offsetY = 0
  } else {
    const scaleX = Math.abs(dirX) > 0.001 ? maxOffsetX / Math.abs(dirX) : Infinity
    const scaleY = Math.abs(dirY) > 0.001 ? maxOffsetY / Math.abs(dirY) : Infinity
    const boundaryScale = Math.min(scaleX, scaleY)

    offsetX = dirX * boundaryScale
    offsetY = dirY * boundaryScale
  }

  const finalX = imageCenterX + offsetX
  const finalY = imageCenterY + offsetY

  return {
    position: 'absolute',
    top: `${finalY}px`,
    left: `${finalX}px`,
    transform: 'translate(-50%, -50%)',
  }
})

const rotationStyle = computed(() => {
  return {
    transform: `rotate(${props.angleDeg}deg)`,
  }
})

// Place the info box on the chevrons' "tail" side (opposite the target direction)
// so it doesn't occlude the target the arrow is pointing at. The SVG drawing tail
// sits at ~0.3125 of SVG height below SVG centre (viewBox y=130 of 160), so push
// the info box past that plus a small gap.
const INFO_TAIL_GAP_PX = 20
const distanceInfoStyle = computed(() => {
  const angleRad = (props.angleDeg * Math.PI) / 180
  const tailDirX = -Math.sin(angleRad)
  const tailDirY = Math.cos(angleRad)
  const arrowSize = responsiveBaseSize.value * arrowScale.value
  const tailOffsetPx = arrowSize * 1.6 * 0.3125 + INFO_TAIL_GAP_PX
  return {
    top: `calc(50% + ${tailDirY * tailOffsetPx}px)`,
    left: `calc(50% + ${tailDirX * tailOffsetPx}px)`,
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

function formatDistance(degrees) {
  if (degrees < 1) {
    return `${(degrees * 60).toFixed(1)}'`
  }
  return `${degrees.toFixed(1)}°`
}

function formatScreenSizes(screens) {
  if (screens === null) return null
  if (screens < 0.1) return null
  if (screens < 1) {
    return `${(screens * 100).toFixed(0)}% screen`
  }
  if (screens < 10) {
    return `${screens.toFixed(1)}× screen`
  }
  return `${Math.round(screens)}× screen`
}
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
      <div class="arrow-wrapper" :style="rotationStyle">
        <svg
            viewBox="0 0 100 160" :width="responsiveBaseSize * arrowScale"
            :height="responsiveBaseSize * 1.6 * arrowScale">
          <defs>
            <linearGradient id="chevronGradient" x1="0%" y1="100%" x2="0%" y2="0%">
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
                stroke="url(#chevronGradient)"
                :stroke-width="chevron.strokeWidth"
                stroke-linecap="round"
                stroke-linejoin="round"
                :opacity="chevron.opacity"
            />
          </g>
        </svg>
      </div>

      <div class="distance-info" :style="distanceInfoStyle">
        <span class="distance">{{ formatDistance(distanceDeg) }}</span>
        <span v-if="formatScreenSizes(screenSizes)" class="screen-size">{{ formatScreenSizes(screenSizes) }}</span>
        <span class="hint">{{ directionHint }}</span>
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
  z-index: 20;
}

.on-target {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  animation: pulse-success 2s ease-in-out infinite;
}

.on-target-label {
  font-size: 0.875rem;
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
  background: var(--surface-elevated);
  padding: 0.45rem 0.9rem;
  border-radius: 6px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
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
  font-size: 0.975rem;
  font-weight: 500;
  color: var(--primary);
  font-family: var(--font-mono);
}

.hint {
  font-size: 0.9rem;
  color: var(--text-secondary);
}
</style>
