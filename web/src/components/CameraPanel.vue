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
import {isSimulator, useSelectedCamera} from '../composables/appContext.js'
import {BaseAlert, BaseInfoIcon, BaseModal, BasePanel, BaseSpinner, BaseSplitButton} from './ui'
import SimulatorDirectoryInput from './SimulatorDirectoryInput.vue'
import {isCaptureRunning} from '../constants'

const cameras = inject('cameras')
const selectedCamera = inject('selectedCamera')
const refreshCameras = inject('refreshCameras')
const refreshSettings = inject('refreshSettings', async () => {})
const eventStream = inject('eventStream')
const simulatorEnabledRef = inject('simulatorEnabled')
const cameraStatus = inject('cameraStatus', {value: {}})
const cameraPhase = inject('cameraPhase', {value: {}})
const warmupEndsAt = inject('warmupEndsAt', ref({}))
const settings = inject('settings', ref(null))

const {error, clearError, withErrorHandling} = useError()

/** A role is a kind of camera, not a state: told apart by hue. */
const ROLE_TONES = {main: 'category-blue', guide: 'category-green'}
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
  return cameras.value.filter((c) => !isSimulator(c))
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

const currentCamera = useSelectedCamera()

const isCapturing = computed(() => isCaptureRunning(eventStream.captureState.value))

async function handleConnect(cameraId, role = 'main') {
  connecting.value = cameraId
  await withErrorHandling(async () => {
    await connectCamera(cameraId, role)
    // The settings first: the connect stored this camera's profiles, and Push-To resolved a
    // camera it found no profile for from the previous camera's optics. Not in parallel:
    // with nothing selected, the list refresh selects the new camera itself.
    await refreshSettings()
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

async function handleRemoveSimulatedCamera(cam) {
  if (!isSimulator(cam)) return

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
        <h3 class="section-title section-title-plain">Connected</h3>
        <div class="camera-list">
          <div
              v-for="cam in connectedCameras"
              :key="cam.id"
              class="camera-item"
              :class="{ selected: cam.id === selectedCamera }"
              @click="selectCamera(cam.id)"
          >
            <button
                v-if="isSimulator(cam)"
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
                <span v-if="roleLabel(cam)" class="pill role-pill" :class="ROLE_TONES[cam.role]">{{
                    roleLabel(cam)
                  }}</span>
                <span v-if="phaseLabel(cam)" class="pill pill-warning phase-pill">
                  <BaseSpinner v-if="isWarmingUp(cam)" size="sm" light class="warmup-spinner" aria-hidden="true" />
                  {{ phaseLabel(cam) }}
                </span>
                <span v-if="sensorModePill(cam)" class="pill category-purple sensor-mode-pill">{{
                    sensorModePill(cam)
                  }}</span>
                <span v-if="temperaturePill(cam)" class="pill pill-primary temp-pill">{{
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
        <h3 class="section-title section-title-plain">Available</h3>
        <div class="camera-list">
          <div v-for="cam in availableCameras" :key="cam.id" class="camera-item available">
            <button
                v-if="isSimulator(cam)"
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
        <h3 class="section-title section-title-plain">Simulator</h3>
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
/* Plain: the camera lists sit tight under their titles */
.section-title {
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
  border-radius: var(--radius-md);
  cursor: pointer;
  border: 1px solid transparent;
  transition: border-color var(--transition-fast),
  background var(--transition-fast);
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
  font-size: var(--fs-md);
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.camera-details {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  display: flex;
  align-items: center;
  gap: 0.375rem;
}

.temp-pill {
  font-variant-numeric: tabular-nums;
}

.confirm-text {
  margin: 0;
  line-height: 1.5;
}

.role-pill {
  font-weight: 600;
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
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: color var(--transition-fast),
  background var(--transition-fast),
  border-color var(--transition-fast);
}

.btn-icon:hover:not(:disabled) {
  color: var(--error);
  background: var(--surface-hover);
  border-color: var(--error);
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
  font-size: var(--fs-md);
}

.collapsed-summary .camera-details {
  font-size: var(--fs-2xs);
}

.simulator-add {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem;
  background: var(--surface-elevated);
  border-radius: var(--radius-md);
}

.simulator-count {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
</style>
