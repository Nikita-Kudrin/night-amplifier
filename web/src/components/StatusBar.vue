<script setup lang="ts">
import {computed} from 'vue'
import {captureStateDisplay, HELP_TEXTS} from '../constants'
import BaseInfoIcon from './ui/BaseInfoIcon.vue'
import BaseSpinner from './ui/BaseSpinner.vue'
import StatusIcon from './ui/StatusIcon.vue'
import StatusNotice from './StatusNotice.vue'
import type {StatusTone} from './StatusNotice.vue'
import type {STATUS_ICON_SHAPES} from './ui/StatusIcon.vue'
import {useEventStreamContext, useSelectedCamera} from '../composables/appContext'
import type {CameraRole, SolveVerdict} from '../composables/api.types'
import {formatPercent} from '../utils/format'

/** A chip: `kind` is its class, `dismiss` (when it has one) what a click on it clears. */
interface Chip {
  kind: string
  tone: StatusTone
  icon?: keyof typeof STATUS_ICON_SHAPES
  spinner?: boolean
  text: string
  dismiss?: () => void
}

// The bar is nothing but a view of the stream `App.vue` provides; it has no stand-in.
const eventStream = useEventStreamContext()
if (!eventStream) throw new Error('StatusBar needs the event stream App.vue provides')
const currentCamera = useSelectedCamera()

const captureState = computed(() => captureStateDisplay(eventStream.captureState.value))

const connected = computed(() => eventStream.connected.value)

const rejectedCount = computed(() => eventStream.rejectedCount.value)

// "Dropped: 40" on its own says nothing — it is a ruined evening at 30 s subs and a
// rounding error at 100 ms. The share of delivered frames is the number that tells the
// observer they are integrating at a fraction of the cadence they set.
const dropRate = computed(() => {
  const delivered = eventStream.deliveredCount.value
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

const SOLVE_RESULT_CHIPS: Record<SolveVerdict, Pick<Chip, 'tone' | 'icon'>> = {
  success: {tone: 'success', icon: 'check'},
  cancelled: {tone: 'muted', icon: 'stop'},
  failed: {tone: 'error', icon: 'cross'},
}

/** A verdict this build does not know reads as a failure: an old cached tab can meet a newer server. */
function solveResultChip(result: string) {
  const byResult: Partial<Record<string, Pick<Chip, 'tone' | 'icon'>>> = SOLVE_RESULT_CHIPS
  return byResult[result] ?? SOLVE_RESULT_CHIPS.failed
}

/**
 * Push-To's one chip: a solve running, else why Push-To is idle (no target, ASTAP
 * missing, a backoff, a scope being pushed), else the last result. A blocker outranks the
 * result because it is *newer* — the same precedence `solvingMessage` uses; behind it, a
 * blocker was announced with the previous solve's green tick, so pushing away from M31
 * read as a fresh success.
 */
const pushToChip = computed((): Chip | null => {
  const solving = eventStream.plateSolving.value
  if (solving.inProgress) {
    return {kind: 'solving', tone: 'primary', spinner: true, text: eventStream.solvingMessage.value ?? ''}
  }
  const blocked = eventStream.pushToBlocked.value
  if (blocked) return {kind: 'push-to-blocked', tone: 'plain', icon: 'blocked', text: blocked}
  const result = solving.lastResult
  if (!result) return null
  return {
    kind: `solve-result ${result}`,
    ...solveResultChip(result),
    text: eventStream.solvingMessage.value ?? '',
    dismiss: () => eventStream.clearPlateSolving(),
  }
})

const STORAGE_ROLES: readonly CameraRole[] = ['main', 'guide']

/** "3 files", or "3 guide frames": the guide camera saves only its raw frames. */
function savedFiles(role: CameraRole, n: number) {
  const noun = role === 'guide' ? 'guide frame' : 'file'
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}

/**
 * Whether what a camera's last capture saved is on the storage device, not just in the OS's
 * cache: 2026-10-16's Orange Pi lost power seconds after a Stop and kept an empty stack. One
 * per camera, since each syncs when it stops. Saving is not dismissible — the observer must
 * wait it out — and outranks Push-To while it lasts. A finished save shows nothing: the
 * chip going away is the all-clear.
 */
const storageChip = (role: CameraRole): Chip | null => {
  const sync = eventStream.storageSync.value[role]
  if (!sync) return null
  const kind = `storage-sync ${role} ${sync.state}`
  if (sync.state === 'syncing') {
    const what = role === 'guide' ? 'Saving guide frames' : 'Saving'
    return {kind, tone: 'primary', spinner: true, text: `${what} to disk — keep the power on`}
  }
  if (sync.state === 'failed') {
    const dismiss = () => eventStream.clearStorageSync(role)
    // No count when the sync itself never answered (the drive stopped responding): which
    // files made it is unknown, so none is named lost.
    const what = sync.failed > 0
      ? `${savedFiles(role, sync.failed)} not saved to disk`
      : `${role === 'guide' ? 'Guide frames' : 'Captures'} not confirmed on disk`
    return {kind, tone: 'error', icon: 'alert', text: `${what}${sync.error ? `: ${sync.error}` : ''}`, dismiss}
  }
  return null
}

const liveChips = computed(() =>
  [...STORAGE_ROLES.map(storageChip), pushToChip.value].filter((chip) => chip !== null))

/** The dismissible notices, in the order they show. */
const notices = computed(() => {
  const es = eventStream
  const list: Required<Pick<Chip, 'kind' | 'tone' | 'icon' | 'text' | 'dismiss'>>[] = []
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
        v-for="chip in liveChips"
        :key="chip.kind"
        :class="chip.kind"
        :tone="chip.tone"
        :icon="chip.icon"
        :text="chip.text"
        :dismissible="Boolean(chip.dismiss)"
        @dismiss="chip.dismiss?.()"
    >
      <template v-if="chip.spinner" #icon>
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
