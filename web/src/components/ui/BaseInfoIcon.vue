<script setup>
import {ref, onMounted, onUnmounted, nextTick} from 'vue'

defineProps({
  message: {
    type: String,
    required: true
  },
  type: {
    type: String,
    default: 'info',
    validator: (v) => ['info', 'warning'].includes(v)
  }
})

const showTooltip = ref(false)
const iconRef = ref(null)
const bubbleRef = ref(null)
const bubbleStyle = ref({})
const arrowPosition = ref('50%')
const isFlipped = ref(false)

async function toggleTooltip(e) {
  e.preventDefault()
  e.stopPropagation()
  showTooltip.value = !showTooltip.value

  if (showTooltip.value) {
    await nextTick()
    calculatePosition()
  }
}

function calculatePosition() {
  if (!bubbleRef.value || !iconRef.value) return

  const iconRect = iconRef.value.getBoundingClientRect()
  const bubbleRect = bubbleRef.value.getBoundingClientRect()

  // Find boundaries - prefer .sidebar if exists, else viewport
  const sidebar = iconRef.value.closest('.sidebar')
  const boundaryRect = sidebar ? sidebar.getBoundingClientRect() : {
    left: 0,
    top: 0,
    right: window.innerWidth,
    bottom: window.innerHeight
  }

  const padding = 12

  // 1. Horizontal positioning
  let leftOffset = -bubbleRect.width / 2 + iconRect.width / 2
  const absoluteLeft = iconRect.left + leftOffset

  // Check left overflow
  if (absoluteLeft < boundaryRect.left + padding) {
    leftOffset = boundaryRect.left + padding - iconRect.left
  }
  // Check right overflow
  else if (absoluteLeft + bubbleRect.width > boundaryRect.right - padding) {
    leftOffset = boundaryRect.right - padding - iconRect.left - bubbleRect.width
  }

  // 2. Vertical positioning (Flip if no space at top)
  // Distance from icon top to boundary top
  const spaceAtTop = iconRect.top - boundaryRect.top
  const neededSpace = bubbleRect.height + 16 // bubble height + gap + arrow

  isFlipped.value = spaceAtTop < neededSpace

  bubbleStyle.value = {
    left: `${leftOffset}px`,
    top: isFlipped.value ? 'calc(100% + 12px)' : 'auto',
    bottom: isFlipped.value ? 'auto' : 'calc(100% + 12px)',
    transform: 'none'
  }

  // 3. Arrow position relative to bubble
  const arrowPx = iconRect.left + iconRect.width / 2 - (iconRect.left + leftOffset)
  const arrowPercent = (arrowPx / bubbleRect.width) * 100
  arrowPosition.value = `${Math.max(10, Math.min(90, arrowPercent))}%`
}

function handleClickOutside(e) {
  if (showTooltip.value && iconRef.value && !iconRef.value.contains(e.target)) {
    showTooltip.value = false
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
  window.addEventListener('resize', calculatePosition)
})

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
  window.removeEventListener('resize', calculatePosition)
})
</script>

<template>
  <div ref="iconRef" class="info-icon-container">
    <button
        class="info-icon"
        :class="type"
        type="button"
        title="Click for help"
        @click="toggleTooltip"
    >
      <svg
          v-if="type === 'info'"
          viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"
          stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"></circle>
        <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"></path>
        <line x1="12" y1="17" x2="12.01" y2="17"></line>
      </svg>
      <svg
          v-else-if="type === 'warning'"
          viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"
          stroke-linejoin="round">
        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
        <line x1="12" y1="9" x2="12" y2="13"/>
        <line x1="12" y1="17" x2="12.01" y2="17"/>
      </svg>
    </button>

    <div
        v-if="showTooltip"
        ref="bubbleRef"
        class="help-bubble animate-fade-in"
        :class="{ 'is-flipped': isFlipped }"
        :style="bubbleStyle"
    >
      <div class="bubble-content">
        {{ message }}
      </div>
      <div class="bubble-arrow" :style="{ left: arrowPosition }"></div>
    </div>
  </div>
</template>

<style scoped>
.info-icon-container {
  display: inline-block;
  position: relative;
  line-height: 0;
  vertical-align: middle;
}

.info-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  padding: 0;
  border: none;
  background: none;
  color: var(--text-muted);
  cursor: pointer;
  transition: color var(--transition-fast), transform var(--transition-fast);
  margin-left: 4px;
}

.info-icon:hover,
.info-icon.info:hover {
  color: var(--primary);
  transform: scale(1.1);
}

.info-icon.warning {
  color: #f59e0b;
}

.info-icon.warning:hover {
  color: #d97706;
  transform: scale(1.1);
}

.info-icon svg {
  width: 100%;
  height: 100%;
}

.help-bubble {
  position: absolute;
  width: min(300px, 80vw);
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 0.75rem;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.6);
  z-index: 1000;
  pointer-events: auto;
}

.bubble-content {
  font-size: 0.75rem;
  line-height: 1.5;
  color: var(--text-primary);
  text-align: left;
  white-space: pre-wrap;
  text-transform: none !important;
  font-weight: 400 !important;
  letter-spacing: normal !important;
  font-style: normal !important;
}

.bubble-arrow {
  position: absolute;
  top: 100%;
  transform: translateX(-50%);
  border-left: 6px solid transparent;
  border-right: 6px solid transparent;
  border-top: 6px solid var(--border);
}

.bubble-arrow::after {
  content: '';
  position: absolute;
  top: -7px;
  left: -6px;
  border-left: 6px solid transparent;
  border-right: 6px solid transparent;
  border-top: 6px solid var(--surface-elevated);
}

/* Flipped state (arrow on top) */
.help-bubble.is-flipped .bubble-arrow {
  top: auto;
  bottom: 100%;
  border-top: none;
  border-bottom: 6px solid var(--border);
}

.help-bubble.is-flipped .bubble-arrow::after {
  top: 1px;
  border-top: none;
  border-bottom: 6px solid var(--surface-elevated);
}

.animate-fade-in {
  animation: fadeIn 0.2s ease-out;
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
</style>
