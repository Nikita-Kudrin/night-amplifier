<script setup lang="ts">
import {ref, computed, onMounted} from 'vue'
import type {DeepReadonly} from 'vue'
import {
  connectCamera,
  disconnectCamera,
  configureSimulator,
  getSimulatorConfig,
  removeSimulatedCamera,
} from '../composables/api'
import {useCameraBadges} from '../composables/useCameraBadges'
import {useDisconnectConfirm} from '../composables/useDisconnectConfirm'
import {useError} from '../composables/useError'
import {
  isSimulator,
  useCameraPhase,
  useCameras,
  useCameraStatus,
  useEventStreamContext,
  useRefreshCameras,
  useRefreshSettings,
  useSelectedCamera,
  useSelectedCameraId,
  useSettings,
  useSimulatorEnabled,
  useWarmupEndsAt,
} from '../composables/appContext'
import {BaseAlert, BaseInfoIcon, BaseModal, BasePanel, BaseSpinner, BaseSplitButton, CollapseToggle} from './ui'
import CameraRow from './CameraRow.vue'
import SimulatorDirectoryInput from './SimulatorDirectoryInput.vue'
import {CAPTURE_STATES, HELP_TEXTS, isCaptureRunning} from '../constants'
import {CAMERA_ROLE_VALUES} from '../composables/api.types'
import type {Camera, CameraRole, SimulatorConfig} from '../composables/api.types'
import type {SplitButtonOption} from './ui/BaseSplitButton.vue'

/** A camera as the camera list hands it out. */
type ListedCamera = DeepReadonly<Camera>

const cameras = useCameras()
const selectedCamera = useSelectedCameraId()
const refreshCameras = useRefreshCameras()
const refreshSettings = useRefreshSettings()
const eventStream = useEventStreamContext()
const simulatorEnabledRef = useSimulatorEnabled()
const cameraStatus = useCameraStatus()
const cameraPhase = useCameraPhase()
const warmupEndsAt = useWarmupEndsAt()
const settings = useSettings()

const {error, clearError, setError, withErrorHandling} = useError()

/** A role is a kind of camera, not a state: told apart by hue. */
const ROLE_TONES: Record<CameraRole, string> = {main: 'category-blue', guide: 'category-green'}
const {roleLabel, formatResolution, temperaturePill, isWarmingUp, phaseLabel, sensorModePill} =
    useCameraBadges({cameraStatus, cameraPhase, warmupEndsAt, settings})

const isSimulatorEnabled = computed(() => simulatorEnabledRef.value)

const connecting = ref<string | null>(null)
const camerasCollapsed = ref(false)

// Simulator state
const simulatorConfig = ref<SimulatorConfig>({configured: false, directory: null, file_count: null})
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
const connectOptions = computed((): SplitButtonOption[] => [
  {value: 'guide', label: 'As guide', disabled: hasGuideCamera.value},
])

const availableCameras = computed(() => filteredCameras.value.filter((c) => !c.connected))

const currentCamera = useSelectedCamera()

/** Without a provided event stream the imaging camera reads as idle, as `useCaptureTarget` has it. */
const isCapturing = computed(() => isCaptureRunning(eventStream?.captureState.value ?? CAPTURE_STATES.IDLE))

/** Runs `operation` for camera `id`, which reads as busy (its buttons disabled) meanwhile. */
async function withBusy(id: string, operation: () => Promise<unknown>) {
  connecting.value = id
  await withErrorHandling(operation)
  connecting.value = null
}

function selectFirstConnectedIf(cameraId: string) {
  if (selectedCamera.value === cameraId) {
    selectedCamera.value = connectedCameras.value[0]?.id || null
  }
}

function handleConnect(cameraId: string, role: CameraRole = 'main') {
  return withBusy(cameraId, async () => {
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
}

/** A connect menu option's `value` is the role it connects as (`connectOptions`). */
function handleConnectOption(cameraId: string, value: string) {
  const role = CAMERA_ROLE_VALUES.find((r) => r === value)
  return role ? handleConnect(cameraId, role) : undefined
}

/** Only the imaging camera captures; a guide camera disconnects whatever the capture does. */
function capturesOn(cam: ListedCamera) {
  return cam.role !== 'guide' && isCapturing.value
}

function handleDisconnect(cam: ListedCamera, {skipWarmup = false} = {}) {
  return withBusy(cam.id, async () => {
    if (skipWarmup) {
      await disconnectCamera(cam.id, {skipWarmup: true})
    } else {
      await disconnectCamera(cam.id)
    }
    await refreshCameras()
    selectFirstConnectedIf(cam.id)
  })
}

const {
  copy: pendingDisconnectCopy,
  request: requestDisconnect,
  confirm: confirmDisconnect,
  cancel: cancelDisconnect,
} = useDisconnectConfirm({cameras, isWarmingUp, capturesOn, disconnect: handleDisconnect})

function selectCamera(cameraId: string) {
  selectedCamera.value = cameraId
}

async function handleConfigureSimulator(path: string) {
  if (!path) {
    setError('Please enter a directory path')
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

async function handleRemoveSimulatedCamera(cam: ListedCamera) {
  // The camera index within the Simulator provider
  const index = cam.index
  if (!isSimulator(cam) || index == null) return

  await withBusy(cam.id, async () => {
    await removeSimulatedCamera(index)
    await refreshCameras()
    simulatorConfig.value = await getSimulatorConfig()
    selectFirstConnectedIf(cam.id)
  })
}
</script>

<template>
  <BasePanel class="camera-panel" :bordered="false">
    <template #header>
      <CollapseToggle v-model="camerasCollapsed" title="Toggle camera list"/>
      <h2>
        Camera
        <BaseInfoIcon :message="HELP_TEXTS.cameras"/>
      </h2>
      <button class="btn btn-sm" title="Refresh" @click="refreshCameras()">
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
          <CameraRow
              v-for="cam in connectedCameras"
              :key="cam.id"
              :camera="cam"
              :selected="cam.id === selectedCamera"
              :removable="isSimulator(cam)"
              :remove-disabled="connecting === cam.id || isCapturing"
              @select="selectCamera(cam.id)"
              @remove="handleRemoveSimulatedCamera(cam)"
          >
            <template #details>
              {{ formatResolution(cam) }}
              <span v-if="roleLabel(cam)" class="pill role-pill" :class="cam.role && ROLE_TONES[cam.role]">{{
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
            </template>
            <template #actions>
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
            </template>
          </CameraRow>
        </div>
      </div>

      <!-- Available cameras -->
      <div v-if="availableCameras.length > 0" class="camera-section">
        <h3 class="section-title section-title-plain">Available</h3>
        <div class="camera-list">
          <CameraRow
              v-for="cam in availableCameras"
              :key="cam.id"
              :camera="cam"
              :resolution="formatResolution(cam)"
              available
              :removable="isSimulator(cam)"
              :remove-disabled="connecting === cam.id"
              @remove="handleRemoveSimulatedCamera(cam)"
          >
            <template #actions>
              <BaseSplitButton
                  :label="connecting === cam.id ? '...' : 'Connect'"
                  menu-label="More connect options"
                  :options="connectOptions"
                  :disabled="connecting === cam.id"
                  @click="handleConnect(cam.id, 'main')"
                  @select="(value) => handleConnectOption(cam.id, value)"
              />
            </template>
          </CameraRow>
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
        <button class="btn btn-sm" @click="refreshCameras()">Scan</button>
      </div>
    </div>

    <Teleport to="body">
      <BaseModal
          v-if="pendingDisconnectCopy"
          :title="pendingDisconnectCopy.title"
          max-width="420px"
          @close="cancelDisconnect"
      >
        <p class="confirm-text">{{ pendingDisconnectCopy.body }}</p>
        <template #footer>
          <button class="btn btn-sm btn-secondary confirm-cancel" @click="cancelDisconnect">Cancel</button>
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

/* The rows are `CameraRow`; the folded list's one-line summary reads the same way. */
.camera-name {
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.camera-details {
  color: var(--text-muted);
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

/* The details slot renders in CameraRow, but its pills are this component's markup. */
.warmup-spinner {
  margin-right: 0.25rem;
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
