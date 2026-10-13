<script setup>
import {computed} from 'vue'
import {captureStateDisplay, HELP_TEXTS} from '../constants'
import BaseInfoIcon from './ui/BaseInfoIcon.vue'
import BaseSpinner from './ui/BaseSpinner.vue'
import StatusIcon from './ui/StatusIcon.vue'
import StatusNotice from './StatusNotice.vue'
import {useEventStreamContext, useSelectedCamera} from '../composables/appContext.js'
import {formatPercent} from '../utils/format.js'

const eventStream = useEventStreamContext()
const currentCamera = useSelectedCamera()

const captureState = computed(() => captureStateDisplay(eventStream.captureState.value))

const connected = computed(() => eventStream.connected.value)

const rejectedCount = computed(() => eventStream.rejectedCount.value)

// "Dropped: 40" on its own says nothing — it is a ruined evening at 30 s subs and a
// rounding error at 100 ms. The share of delivered frames is the number that tells the
// observer they are integrating at a fraction of the cadence they set.
const dropRate = computed(() => {
  const delivered = eventStream.deliveredCount?.value ?? 0
  if (!delivered) return null
  return eventStream.droppedCount.value / delivered
})

// "Rejected 9" on its own leaves the user with no idea whether to refocus,
// wait out a cloud, or check the mount. The server already decides why.
const framesTooltip = computed(() => {
  const base = HELP_TEXTS.frame_counts
  const reason = eventStream.lastRejectionReason.value
  return reason ? `${base} Most recent rejection: ${reason}.` : base
})

const SOLVE_RESULT_CHIPS = {
  success: {tone: 'success', icon: 'check'},
  cancelled: {tone: 'muted', icon: 'stop'},
  failed: {tone: 'error', icon: 'cross'},
}

/**
 * Push-To's one chip: a solve running, else why Push-To is idle (no target, ASTAP
 * missing, a backoff, a scope being pushed), else the last result. A blocker outranks the
 * result because it is *newer* — the same precedence `solvingMessage` uses; behind it, a
 * blocker was announced with the previous solve's green tick, so pushing away from M31
 * read as a fresh success.
 */
const pushToChip = computed(() => {
  const solving = eventStream.plateSolving.value
  if (solving?.inProgress) {
    return {kind: 'solving', tone: 'primary', spinner: true, text: eventStream.solvingMessage.value}
  }
  const blocked = eventStream.pushToBlocked?.value
  if (blocked) return {kind: 'push-to-blocked', tone: 'plain', icon: 'blocked', text: blocked}
  const result = solving?.lastResult
  if (!result) return null
  return {
    kind: `solve-result ${result}`,
    ...(SOLVE_RESULT_CHIPS[result] ?? SOLVE_RESULT_CHIPS.failed),
    text: eventStream.solvingMessage.value,
    dismiss: () => eventStream.clearPlateSolving(),
  }
})

/** The dismissible notices, in the order they show. */
const notices = computed(() => {
  const es = eventStream
  const list = []
  if (es.diskWriterWarning.value) {
    list.push({
      kind: 'disk-warning', tone: 'warning', icon: 'warning',
      text: `Slow disk: ${es.diskWriterWarning.value} frames queued`, dismiss: es.clearDiskWriterWarning,
    })
  }
  if (es.unresponsiveWarning.value) {
    list.push({
      kind: 'unresponsive', tone: 'error', icon: 'alert',
      text: es.unresponsiveWarning.value, dismiss: es.clearUnresponsiveWarning,
    })
  }
  if (es.resumeNotice.value) {
    list.push({kind: 'resumed', tone: 'success', icon: 'resume', text: es.resumeNotice.value, dismiss: es.clearResumeNotice})
  }
  // The server left Focus/Finder mode because the running capture switched to stacking.
  if (es.focusModeNotice.value) {
    list.push({
      kind: 'resumed focus-mode-left', tone: 'success', icon: 'info',
      text: es.focusModeNotice.value, dismiss: es.clearFocusModeNotice,
    })
  }
  // Not beside a camera warning: the two describe the same incident, and stacking them
  // just doubles the noise.
  if (es.lastError.value && !es.unresponsiveWarning.value) {
    list.push({kind: 'last-error', tone: 'error', icon: 'alert', text: es.lastError.value, dismiss: es.clearError})
  }
  return list
})
</script>

<template>
  <footer class="status-bar">
    <div class="status-item connection" :class="connected ? 'connected' : 'disconnected'">
      <span class="status-dot"></span>
      <span class="status-text">{{ connected ? 'Connected' : 'Disconnected' }}</span>
    </div>

    <div v-if="currentCamera" class="status-item camera">
      <StatusIcon name="camera"/>
      <span>{{ currentCamera.name }}</span>
    </div>

    <div class="spacer"></div>

    <StatusNotice
        v-if="pushToChip"
        :class="pushToChip.kind"
        :tone="pushToChip.tone"
        :icon="pushToChip.icon"
        :text="pushToChip.text"
        :dismissible="Boolean(pushToChip.dismiss)"
        @dismiss="pushToChip.dismiss()"
    >
      <template v-if="pushToChip.spinner" #icon>
        <BaseSpinner size="xs"/>
      </template>
    </StatusNotice>

    <StatusNotice
        v-for="notice in notices"
        :key="notice.kind"
        :class="notice.kind"
        :tone="notice.tone"
        :icon="notice.icon"
        :text="notice.text"
        dismissible
        @dismiss="notice.dismiss()"
    />

    <div v-if="eventStream.frameCount.value > 0" class="status-item frames">
      <StatusIcon name="frames"/>
      <span v-if="rejectedCount > 0">Rejected {{ rejectedCount }} | Total {{ eventStream.frameCount.value }}</span>
      <span v-else>Total {{ eventStream.frameCount.value }}</span>
      <BaseInfoIcon :message="framesTooltip"/>
    </div>

    <div v-if="eventStream.droppedCount.value > 0" class="status-item frames dropped">
      <StatusIcon name="dropped"/>
      <span>
        Dropped: {{ eventStream.droppedCount.value }}<template v-if="dropRate !== null">
          ({{ formatPercent(dropRate) }})</template>
      </span>
    </div>

    <div class="status-item state" :class="captureState.tone">
      <span class="state-indicator"></span>
      <span>{{ captureState.label }}</span>
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
  font-size: var(--fs-2xs);
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

.frames.dropped {
  color: var(--warning);
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
