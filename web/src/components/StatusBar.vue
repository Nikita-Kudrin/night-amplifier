<script setup>
import {computed, inject} from 'vue'
import {CAPTURE_STATES} from '../constants'

const eventStream = inject('eventStream')
const selectedCamera = inject('selectedCamera')
const cameras = inject('cameras')

const currentCamera = computed(() => cameras.value.find((c) => c.id === selectedCamera.value))

const stateClass = computed(() => {
  const state = eventStream.captureState.value
  switch (state) {
    case CAPTURE_STATES.CAPTURING:
      return 'capturing'
    case CAPTURE_STATES.STARTING:
      return 'starting'
    case CAPTURE_STATES.STOPPING:
      return 'stopping'
    case CAPTURE_STATES.ERROR:
      return 'error'
    default:
      return 'idle'
  }
})

const stateLabel = computed(() => {
  const state = eventStream.captureState.value
  switch (state) {
    case CAPTURE_STATES.CAPTURING:
      return 'Capturing'
    case CAPTURE_STATES.STARTING:
      return 'Starting...'
    case CAPTURE_STATES.STOPPING:
      return 'Stopping...'
    case CAPTURE_STATES.ERROR:
      return 'Error'
    default:
      return 'Idle'
  }
})

const connectionStatus = computed(() => {
  if (!eventStream.connected.value) return 'disconnected'
  return 'connected'
})

const rejectedCount = computed(() => {
  return eventStream.frameCount.value - eventStream.stackedCount.value
})

function formatDuration(startedAt) {
  if (!startedAt) return '--:--'
  const elapsed = Date.now() - startedAt
  const seconds = Math.floor(elapsed / 1000)
  const minutes = Math.floor(seconds / 60)
  const hours = Math.floor(minutes / 60)

  if (hours > 0) {
    return `${hours}:${String(minutes % 60).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`
  }
  return `${minutes}:${String(seconds % 60).padStart(2, '0')}`
}
</script>

<template>
  <footer class="status-bar">
    <!-- Connection status -->
    <div class="status-item connection" :class="connectionStatus">
      <span class="status-dot"></span>
      <span class="status-text">{{
          connectionStatus === 'connected' ? 'Connected' : 'Disconnected'
        }}</span>
    </div>

    <!-- Camera info -->
    <div v-if="currentCamera" class="status-item camera">
      <svg
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
      >
        <path d="M23 19a2 2 0 01-2 2H3a2 2 0 01-2-2V8a2 2 0 012-2h4l2-3h6l2 3h4a2 2 0 012 2z"/>
        <circle cx="12" cy="13" r="4"/>
      </svg>
      <span>{{ currentCamera.name }}</span>
    </div>

    <!-- Spacer -->
    <div class="spacer"></div>

    <!-- Plate solving indicator -->
    <div v-if="eventStream.plateSolving.value?.inProgress" class="status-item solving">
      <svg
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          class="solving-spinner"
      >
        <circle cx="12" cy="12" r="10"/>
        <path d="M12 6v6l4 2"/>
      </svg>
      <span>Plate solving{{
          eventStream.plateSolving.value.targetName ? ` for ${eventStream.plateSolving.value.targetName}` : ''
        }}...</span>
    </div>

    <!-- Plate solve result -->
    <div
        v-else-if="eventStream.plateSolving.value?.lastResult"
        class="status-item solve-result"
        :class="eventStream.plateSolving.value.lastResult"
        @click="eventStream.clearPlateSolving()"
    >
      <svg
          v-if="eventStream.plateSolving.value.lastResult === 'success'"
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
      >
        <path d="M20 6L9 17l-5-5"/>
      </svg>
      <svg
          v-else
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
      >
        <circle cx="12" cy="12" r="10"/>
        <path d="M15 9l-6 6M9 9l6 6"/>
      </svg>
      <span>{{
          eventStream.plateSolving.value.lastResult === 'success' ? 'Solved' : 'Failed solving'
        }}{{
          eventStream.plateSolving.value.targetName ? ` for ${eventStream.plateSolving.value.targetName}` : ''
        }}</span>
    </div>

    <!-- Disk writer warning -->
    <div
        v-if="eventStream.diskWriterWarning.value"
        class="status-item warning"
        @click="eventStream.clearDiskWriterWarning()"
    >
      <svg
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
      >
        <path
            d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
        />
        <path d="M12 9v4M12 17h.01"/>
      </svg>
      <span>Slow disk: {{ eventStream.diskWriterWarning.value }} frames queued</span>
      <button class="btn-close btn-dismiss">&times;</button>
    </div>

    <!-- Error indicator -->
    <div
        v-if="eventStream.lastError.value"
        class="status-item error"
        @click="eventStream.clearError()"
    >
      <svg
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
      >
        <circle cx="12" cy="12" r="10"/>
        <path d="M12 8v4M12 16h.01"/>
      </svg>
      <span>{{ eventStream.lastError.value }}</span>
      <button class="btn-close btn-dismiss">&times;</button>
    </div>

    <!-- Frame counter -->
    <div v-if="eventStream.frameCount.value > 0" class="status-item frames">
      <svg
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
      >
        <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
        <circle cx="8.5" cy="8.5" r="1.5"/>
        <polyline points="21,15 16,10 5,21"/>
      </svg>
      <span v-if="rejectedCount > 0">Rejected {{ rejectedCount }} | Total {{ eventStream.frameCount.value }}</span>
      <span v-else>Total {{ eventStream.frameCount.value }}</span>
    </div>

    <!-- Capture state -->
    <div class="status-item state" :class="stateClass">
      <span class="state-indicator"></span>
      <span>{{ stateLabel }}</span>
    </div>
  </footer>
</template>

<style scoped>
.status-bar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.25rem 0.75rem;
  background: var(--surface-elevated);
  border-top: 1px solid var(--border);
  font-size: 0.65rem;
  color: var(--text-secondary);
  flex-shrink: 0;
  overflow-x: auto;
}

.status-item {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  white-space: nowrap;
}

.spacer {
  flex: 1;
}

/* Uses global .status-dot and @keyframes blink from main.css */

.connection.connected .status-dot {
  background: var(--success);
}

.connection.disconnected .status-dot {
  background: var(--error);
  animation: blink 1s infinite;
}

.status-item.warning {
  color: var(--warning);
  background: var(--warning-bg, rgba(251, 191, 36, 0.15));
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  cursor: pointer;
  max-width: 300px;
}

.status-item.warning span {
  overflow: hidden;
  text-overflow: ellipsis;
}

.status-item.error {
  color: var(--error);
  background: var(--error-bg);
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  cursor: pointer;
  max-width: 300px;
}

.status-item.error span {
  overflow: hidden;
  text-overflow: ellipsis;
}

/* btn-dismiss uses global .btn-close from main.css with opacity variant */
.btn-dismiss {
  opacity: 0.7;
}

.btn-dismiss:hover {
  opacity: 1;
}

.state-indicator {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-muted);
}

.state.idle .state-indicator {
  background: var(--text-muted);
}

.state.capturing .state-indicator {
  background: var(--success);
  animation: pulse 1.5s infinite;
}

.state.starting .state-indicator,
.state.stopping .state-indicator {
  background: var(--warning);
  animation: pulse 0.5s infinite;
}

.state.error .state-indicator {
  background: var(--error);
}

/* Uses global @keyframes pulse from main.css */

.frames {
  font-family: var(--font-mono);
}

.status-item.solving {
  color: var(--primary);
}

.solving-spinner {
  animation: spin 1.5s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.status-item.solve-result {
  cursor: pointer;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
}

.status-item.solve-result.success {
  color: var(--success);
  background: rgba(34, 197, 94, 0.15);
}

.status-item.solve-result.failed {
  color: var(--error);
  background: var(--error-bg);
}

/* Mobile adjustments */
@media (max-width: 768px) {
  .status-bar {
    padding: 0.25rem 0.5rem;
    gap: 0.5rem;
  }

  .status-text {
    display: none;
  }

  .camera span {
    max-width: 100px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
}
</style>
