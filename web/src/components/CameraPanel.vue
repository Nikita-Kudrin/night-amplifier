<script setup>
import {ref, inject, computed, watch, onMounted} from 'vue'
import {
  connectCamera,
  disconnectCamera,
  configureSimulator,
  getSimulatorConfig,
  removeSimulatedCamera,
} from '../composables/api.js'
import {useCameraBadges} from '../composables/useCameraBadges.js'
import {useError} from '../composables/useError.js'
import {BaseAlert, BaseInfoIcon, BaseModal, BasePanel, BaseSpinner, BaseSplitButton} from './ui'
import SimulatorDirectoryInput from './SimulatorDirectoryInput.vue'
import {isCaptureRunning} from '../constants'

const cameras = inject('cameras')
const selectedCamera = inject('selectedCamera')
const refreshCameras = inject('refreshCameras')
const eventStream = inject('eventStream')
const simulatorEnabledRef = inject('simulatorEnabled')
const cameraStatus = inject('cameraStatus', {value: {}})
const cameraPhase = inject('cameraPhase', {value: {}})
const warmupEndsAt = inject('warmupEndsAt', ref({}))
const settings = inject('settings', ref(null))

const {error, clearError, withErrorHandling} = useError()
const {roleLabel, formatResolution, temperaturePill, isWarmingUp, phaseLabel, sensorModePill} =
    useCameraBadges({cameraStatus, cameraPhase, warmupEndsAt, settings})

const isSimulatorEnabled = computed(() => simulatorEnabledRef?.value ?? false)

const connecting = ref(null)
const camerasCollapsed = ref(false)

// Simulator state
const simulatorConfig = ref({configured: false, directory: null, file_count: null})
const configuringSimulator = ref(false)
const showDirectoryInput = ref(false)

onMounted(async () => {
  try {
    simulatorConfig.value = await getSimulatorConfig()
  } catch {
    // Ignore - simulator not configured
  }
})

const filteredCameras = computed(() => {
  if (isSimulatorEnabled.value) {
    return cameras.value
  }
  return cameras.value.filter((c) => c.provider !== 'Simulator')
})

const connectedCameras = computed(() => filteredCameras.value.filter((c) => c.connected))

const hasGuideCamera = computed(() => connectedCameras.value.some((c) => c.role === 'guide'))

/**
 * Only one camera can hold each position, so the menu offers the guide slot only while
 * it is free. The main action stays enabled even with an imaging camera connected: an
 * idle one is swapped, and the server refuses only while it is busy.
 */
const connectOptions = computed(() => [
  {value: 'guide', label: 'As guide', disabled: hasGuideCamera.value},
])

const availableCameras = computed(() => filteredCameras.value.filter((c) => !c.connected))

const currentCamera = computed(() => cameras.value.find((c) => c.id === selectedCamera.value))

const isCapturing = computed(() => isCaptureRunning(eventStream.captureState.value))

async function handleConnect(cameraId, role = 'main') {
  connecting.value = cameraId
  await withErrorHandling(async () => {
    await connectCamera(cameraId, role)
    await refreshCameras()
    // Focus the newly connected camera for editing, whichever position it took: it is
    // the one the user is about to set an exposure on.
    selectedCamera.value = cameraId
  })
  connecting.value = null
}

/**
 * A Disconnect the user has to confirm first: one that stops a running capture, or one
 * that cuts a warm-up short. `null` when no confirmation is open.
 */
const pendingDisconnect = ref(null)

/** Only the imaging camera captures; a guide camera disconnects whatever the capture does. */
function capturesOn(cam) {
  return cam?.role !== 'guide' && isCapturing.value
}

/**
 * Disconnect is always available — the server ends any session in bounded time — but two
 * cases cost something the user should agree to first.
 */
function requestDisconnect(cam) {
  if (isWarmingUp(cam)) {
    pendingDisconnect.value = {camera: cam, kind: 'skip_warmup'}
    return
  }
  if (capturesOn(cam)) {
    pendingDisconnect.value = {camera: cam, kind: 'capture'}
    return
  }
  handleDisconnect(cam.id)
}

async function confirmDisconnect() {
  const pending = pendingDisconnect.value
  pendingDisconnect.value = null
  if (!pending) return
  await handleDisconnect(pending.camera.id, {skipWarmup: pending.kind === 'skip_warmup'})
}

/**
 * Whether the confirmation still describes the camera: it may have finished warming up
 * and gone, or its capture ended, while the dialog was open. Confirming then answered
 * "not connected", or asked about a cost that no longer applied.
 */
function pendingStillApplies(pending) {
  const cam = cameras.value.find((c) => c.id === pending.camera.id && c.connected)
  if (!cam) return false
  return pending.kind === 'skip_warmup' ? isWarmingUp(cam) : capturesOn(cam)
}

watch(
    () => pendingDisconnect.value && pendingStillApplies(pendingDisconnect.value),
    (applies) => {
      if (pendingDisconnect.value && !applies) pendingDisconnect.value = null
    }
)

const pendingDisconnectCopy = computed(() => {
  const pending = pendingDisconnect.value
  if (!pending) return null
  const name = pending.camera.name
  if (pending.kind === 'capture') {
    return {
      title: 'Stop the capture and disconnect?',
      body: `The capture running on ${name} stops first and its stack is saved, as on Stop. The sub being exposed is discarded.`,
      confirm: 'Stop and disconnect',
    }
  }
  return {
    title: 'Disconnect without warming up?',
    body: `${name} is warming up slowly so its sensor is not shocked by a sudden temperature change. Disconnecting now switches the cooler off at once.`,
    confirm: 'Disconnect now',
  }
})

async function handleDisconnect(cameraId, {skipWarmup = false} = {}) {
  connecting.value = cameraId
  await withErrorHandling(async () => {
    if (skipWarmup) {
      await disconnectCamera(cameraId, {skipWarmup: true})
    } else {
      await disconnectCamera(cameraId)
    }
    await refreshCameras()
    if (selectedCamera.value === cameraId) {
      selectedCamera.value = connectedCameras.value[0]?.id || null
    }
  })
  connecting.value = null
}

function selectCamera(cameraId) {
  selectedCamera.value = cameraId
}

async function handleConfigureSimulator(path) {
  if (!path) {
    error.value = 'Please enter a directory path'
    return
  }

  configuringSimulator.value = true
  await withErrorHandling(async () => {
    simulatorConfig.value = await configureSimulator(path)
    showDirectoryInput.value = false
    await refreshCameras()
  })
  configuringSimulator.value = false
}

function promptSimulatorConfig() {
  showDirectoryInput.value = true
  clearError()
}

function isSimulatedCamera(cam) {
  return cam.provider === 'Simulator'
}

async function handleRemoveSimulatedCamera(cam) {
  if (!isSimulatedCamera(cam)) return

  // The camera index within the Simulator provider
  const index = cam.index
  connecting.value = cam.id
  await withErrorHandling(async () => {
    await removeSimulatedCamera(index)
    await refreshCameras()
    simulatorConfig.value = await getSimulatorConfig()
    if (selectedCamera.value === cam.id) {
      selectedCamera.value = connectedCameras.value[0]?.id || null
    }
  })
  connecting.value = null
}

const HELP = {
  cameras:
      'Choose the camera to configure. One imaging camera and one guide camera can be connected at once — use the arrow next to Connect to attach a guide camera.',
}
</script>

<template>
  <BasePanel class="camera-panel" :bordered="false">
    <template #header>
      <button
          class="collapse-toggle"
          title="Toggle camera list"
          @click="camerasCollapsed = !camerasCollapsed"
      >
        <svg
            :class="{ collapsed: camerasCollapsed }"
            viewBox="0 0 24 24"
            width="12"
            height="12"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path d="M6 9l6 6 6-6"/>
        </svg>
      </button>
      <h2>
        Camera
        <BaseInfoIcon :message="HELP.cameras"/>
      </h2>
      <button class="btn btn-sm" title="Refresh" @click="refreshCameras">
        <svg
            viewBox="0 0 24 24"
            width="14"
            height="14"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path d="M23 4v6h-6M1 20v-6h6"/>
          <path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/>
        </svg>
      </button>
    </template>

    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <SimulatorDirectoryInput
        v-if="showDirectoryInput"
        :busy="configuringSimulator"
        @submit="handleConfigureSimulator"
        @close="showDirectoryInput = false"
    />

    <!-- Collapsible camera sections -->
    <div v-show="!camerasCollapsed" class="cameras-container">
      <!-- Connected cameras -->
      <div v-if="connectedCameras.length > 0" class="camera-section">
        <h3 class="section-title">Connected</h3>
        <div class="camera-list">
          <div
              v-for="cam in connectedCameras"
              :key="cam.id"
              class="camera-item"
              :class="{ selected: cam.id === selectedCamera }"
              @click="selectCamera(cam.id)"
          >
            <button
                v-if="isSimulatedCamera(cam)"
                class="btn btn-sm btn-icon"
                :disabled="connecting === cam.id || isCapturing"
                title="Remove simulated camera"
                @click.stop="handleRemoveSimulatedCamera(cam)"
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
                    d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"
                />
              </svg>
            </button>
            <div class="camera-info">
              <span class="camera-name">{{ cam.name }}</span>
              <span class="camera-details">
                {{ formatResolution(cam) }}
                <span v-if="roleLabel(cam)" class="role-pill" :class="`role-${cam.role}`">{{
                    roleLabel(cam)
                  }}</span>
                <span v-if="phaseLabel(cam)" class="phase-pill">
                  <BaseSpinner v-if="isWarmingUp(cam)" size="sm" light class="warmup-spinner" aria-hidden="true" />
                  {{ phaseLabel(cam) }}
                </span>
                <span v-if="sensorModePill(cam)" class="sensor-mode-pill">{{
                    sensorModePill(cam)
                  }}</span>
                <span v-if="temperaturePill(cam)" class="temp-pill">{{
                    temperaturePill(cam)
                  }}</span>
              </span>
            </div>
            <div class="camera-actions">
              <button
                  class="btn btn-sm btn-danger"
                  :disabled="connecting === cam.id"
                  :title="isWarmingUp(cam) ? 'Disconnect without finishing the warm-up' : 'Disconnect'"
                  @click.stop="requestDisconnect(cam)"
              >
                <span>{{
                    connecting === cam.id
                        ? '...'
                        : isWarmingUp(cam)
                            ? 'Disconnect now'
                            : 'Disconnect'
                  }}</span>
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- Available cameras -->
      <div v-if="availableCameras.length > 0" class="camera-section">
        <h3 class="section-title">Available</h3>
        <div class="camera-list">
          <div v-for="cam in availableCameras" :key="cam.id" class="camera-item available">
            <button
                v-if="isSimulatedCamera(cam)"
                class="btn btn-sm btn-icon"
                :disabled="connecting === cam.id"
                title="Remove simulated camera"
                @click.stop="handleRemoveSimulatedCamera(cam)"
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
                    d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"
                />
              </svg>
            </button>
            <div class="camera-info">
              <span class="camera-name">{{ cam.name }}</span>
              <span class="camera-details">{{ formatResolution(cam) }}</span>
            </div>
            <div class="camera-actions">
              <BaseSplitButton
                  :label="connecting === cam.id ? '...' : 'Connect'"
                  menu-label="More connect options"
                  :options="connectOptions"
                  :disabled="connecting === cam.id"
                  @click="handleConnect(cam.id, 'main')"
                  @select="(role) => handleConnect(cam.id, role)"
              />
            </div>
          </div>
        </div>
      </div>

      <!-- Simulator section - always show when simulator enabled -->
      <div v-if="isSimulatorEnabled" class="camera-section">
        <h3 class="section-title">Simulator</h3>
        <div class="simulator-add">
          <button class="btn btn-sm btn-secondary" @click="promptSimulatorConfig">
            + Add Simulated Camera
          </button>
          <span v-if="simulatorConfig.camera_count" class="simulator-count">
            {{ simulatorConfig.camera_count }} configured
          </span>
        </div>
      </div>

      <!-- No cameras -->
      <div
          v-if="filteredCameras.length === 0 && (!isSimulatorEnabled || simulatorConfig.configured)"
          class="empty-state"
      >
        <p>No cameras found</p>
        <button class="btn btn-sm" @click="refreshCameras">Scan</button>
      </div>
    </div>

    <Teleport to="body">
      <BaseModal
          v-if="pendingDisconnectCopy"
          :title="pendingDisconnectCopy.title"
          max-width="420px"
          @close="pendingDisconnect = null"
      >
        <p class="confirm-text">{{ pendingDisconnectCopy.body }}</p>
        <template #footer>
          <button class="btn btn-sm btn-secondary confirm-cancel" @click="pendingDisconnect = null">Cancel</button>
          <button class="btn btn-sm btn-danger confirm-disconnect" @click="confirmDisconnect">
            {{ pendingDisconnectCopy.confirm }}
          </button>
        </template>
      </BaseModal>
    </Teleport>

    <div v-if="camerasCollapsed && currentCamera" class="collapsed-summary">
      <span class="camera-name">{{ currentCamera.name }}</span>
      <span class="camera-details">{{ formatResolution(currentCamera) }}</span>
    </div>
  </BasePanel>
</template>

<style scoped>
/* Panel header variant with collapse toggle and flex-1 title */
.panel-header {
  gap: 0.375rem;
}

.panel-header h2 {
  flex: 1;
}

/* Section title without border (simpler variant) */
.section-title {
  padding-bottom: 0;
  border-bottom: none;
  margin-bottom: 0.25rem;
}

.cameras-container {
  max-height: calc(2 * 2.5rem + 0.75rem);
  overflow-y: auto;
}

.camera-section {
  margin-bottom: 0.5rem;
}

.camera-section:last-child {
  margin-bottom: 0;
}

.camera-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.camera-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.375rem;
  padding: 0.375rem 0.5rem;
  background: var(--surface-elevated);
  border-radius: 6px;
  cursor: pointer;
  border: 1px solid transparent;
  transition: border-color 0.15s,
  background 0.15s;
  min-height: 2.25rem;
}

.camera-item:hover {
  background: var(--surface-hover);
}

.camera-item.selected {
  border-color: var(--primary);
}

.camera-item.available {
  cursor: default;
  opacity: 0.8;
}

.camera-info {
  display: flex;
  flex-direction: column;
  gap: 0;
  min-width: 0;
  flex: 1;
}

.camera-name {
  font-size: 0.8rem;
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.camera-details {
  font-size: 0.65rem;
  color: var(--text-muted);
  display: flex;
  align-items: center;
  gap: 0.375rem;
}

.temp-pill {
  background: rgba(59, 130, 246, 0.15);
  color: var(--primary);
  padding: 0.05rem 0.375rem;
  border-radius: 999px;
  font-variant-numeric: tabular-nums;
}

.phase-pill {
  display: inline-flex;
  align-items: center;
  background: rgba(234, 179, 8, 0.18);
  color: #eab308;
  padding: 0.05rem 0.375rem;
  border-radius: 999px;
  font-weight: 500;
}

.confirm-text {
  margin: 0;
  line-height: 1.5;
}

.sensor-mode-pill {
  background: rgba(168, 85, 247, 0.18);
  color: #c084fc;
  padding: 0.05rem 0.375rem;
  border-radius: 999px;
  font-weight: 500;
}

.role-pill {
  padding: 0.05rem 0.375rem;
  border-radius: 999px;
  font-weight: 600;
}

.role-pill.role-main {
  background: rgba(59, 130, 246, 0.18);
  color: #60a5fa;
}

.role-pill.role-guide {
  background: rgba(34, 197, 94, 0.18);
  color: #4ade80;
}

.camera-actions {
  display: flex;
  gap: 0.25rem;
  align-items: center;
}

.warmup-spinner {
  margin-right: 0.25rem;
}

.btn-icon {
  padding: 0.25rem;
  background: transparent;
  border: 1px solid transparent;
  border-radius: 4px;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: color 0.15s,
  background 0.15s,
  border-color 0.15s;
}

.btn-icon:hover:not(:disabled) {
  color: var(--danger);
  background: var(--surface-hover);
  border-color: var(--danger);
}

.btn-icon:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.collapsed-summary {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.25rem 0;
}

.collapsed-summary .camera-name {
  font-size: 0.8rem;
}

.collapsed-summary .camera-details {
  font-size: 0.65rem;
}

/* empty-state and btn-close now in main.css */

.simulator-add {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem;
  background: var(--surface-elevated);
  border-radius: 6px;
}

.simulator-count {
  font-size: 0.7rem;
  color: var(--text-muted);
}

/* btn-secondary now in main.css */
</style>
