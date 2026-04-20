<script setup>
import { ref, inject, computed, onMounted } from 'vue'
import {
  connectCamera,
  disconnectCamera,
  configureSimulator,
  getSimulatorConfig,
  removeSimulatedCamera,
} from '../composables/api.js'
import { useError } from '../composables/useError.js'
import { BaseAlert, BaseInfoIcon } from './ui'
import { CAPTURE_STATES } from '../constants'

const cameras = inject('cameras')
const selectedCamera = inject('selectedCamera')
const refreshCameras = inject('refreshCameras')
const eventStream = inject('eventStream')
const simulatorEnabledRef = inject('simulatorEnabled')
const cameraStatus = inject('cameraStatus', { value: {} })

const { error, clearError, withErrorHandling } = useError()

const isSimulatorEnabled = computed(() => simulatorEnabledRef?.value ?? false)

const connecting = ref(null)
const camerasCollapsed = ref(false)

// Simulator state
const simulatorConfig = ref({ configured: false, directory: null, file_count: null })
const configuringSimulator = ref(false)
const showDirectoryInput = ref(false)
const directoryPath = ref('')

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

const availableCameras = computed(() => filteredCameras.value.filter((c) => !c.connected))

const currentCamera = computed(() => cameras.value.find((c) => c.id === selectedCamera.value))

const isCapturing = computed(() => eventStream.captureState.value === CAPTURE_STATES.CAPTURING)

async function handleConnect(cameraId) {
  connecting.value = cameraId
  await withErrorHandling(async () => {
    await connectCamera(cameraId)
    await refreshCameras()
    selectedCamera.value = cameraId
  })
  connecting.value = null
}

async function handleDisconnect(cameraId) {
  connecting.value = cameraId
  await withErrorHandling(async () => {
    await disconnectCamera(cameraId)
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

function formatResolution(cam) {
  return `${cam.info.max_width}x${cam.info.max_height}`
}

function temperaturePill(cam) {
  if (!cam?.info?.has_cooler) return null
  const status = cameraStatus.value?.[cam.name]
  if (!status) return null
  return `${status.temperature_c.toFixed(1)}°C`
}

async function handleConfigureSimulator() {
  if (!directoryPath.value.trim()) {
    error.value = 'Please enter a directory path'
    return
  }

  configuringSimulator.value = true
  await withErrorHandling(async () => {
    simulatorConfig.value = await configureSimulator(directoryPath.value.trim())
    showDirectoryInput.value = false
    directoryPath.value = ''
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
  cameras: 'Choose the active camera for capture. Only one camera can be connected at a time.',
  simulator_dir: 'The local path where the simulator looks for source images (FITS, TIFF, or PNG).',
}
</script>

<template>
  <div class="panel">
    <div class="panel-header">
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
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>
      <h2>
        Camera
        <BaseInfoIcon :message="HELP.cameras" />
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
          <path d="M23 4v6h-6M1 20v-6h6" />
          <path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15" />
        </svg>
      </button>
    </div>

    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <div v-if="showDirectoryInput" class="simulator-config">
      <div class="config-header">
        <span>
          Configure Simulator Directory
          <BaseInfoIcon :message="HELP.simulator_dir" />
        </span>
        <button class="btn-close" @click="showDirectoryInput = false">&times;</button>
      </div>
      <div class="config-body">
        <input
          v-model="directoryPath"
          type="text"
          placeholder="Enter path to image directory..."
          class="directory-input"
          @keyup.enter="handleConfigureSimulator"
        />
        <button
          class="btn btn-sm btn-primary"
          :disabled="configuringSimulator"
          @click="handleConfigureSimulator"
        >
          {{ configuringSimulator ? '...' : 'Set' }}
        </button>
      </div>
      <div class="config-hint">
        Enter the full path to a directory containing FITS, TIFF, or PNG files
      </div>
    </div>

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
            <div class="camera-info">
              <span class="camera-name">{{ cam.name }}</span>
              <span class="camera-details">
                {{ formatResolution(cam) }}
                <span v-if="temperaturePill(cam)" class="temp-pill">{{
                  temperaturePill(cam)
                }}</span>
              </span>
            </div>
            <div class="camera-actions">
              <button
                class="btn btn-sm btn-danger"
                :disabled="connecting === cam.id || isCapturing"
                title="Disconnect"
                @click.stop="handleDisconnect(cam.id)"
              >
                {{ connecting === cam.id ? '...' : 'Disconnect' }}
              </button>
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
            </div>
          </div>
        </div>
      </div>

      <!-- Available cameras -->
      <div v-if="availableCameras.length > 0" class="camera-section">
        <h3 class="section-title">Available</h3>
        <div class="camera-list">
          <div v-for="cam in availableCameras" :key="cam.id" class="camera-item available">
            <div class="camera-info">
              <span class="camera-name">{{ cam.name }}</span>
              <span class="camera-details">{{ formatResolution(cam) }}</span>
            </div>
            <div class="camera-actions">
              <button
                class="btn btn-sm btn-primary"
                :disabled="connecting === cam.id"
                @click="handleConnect(cam.id)"
              >
                {{ connecting === cam.id ? '...' : 'Connect' }}
              </button>
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

    <!-- Collapsed summary -->
    <div v-if="camerasCollapsed && currentCamera" class="collapsed-summary">
      <span class="camera-name">{{ currentCamera.name }}</span>
      <span class="camera-details">{{ formatResolution(currentCamera) }}</span>
    </div>
  </div>
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
  padding: 0.375rem 0.5rem;
  background: var(--surface-elevated);
  border-radius: 6px;
  cursor: pointer;
  border: 1px solid transparent;
  transition:
    border-color 0.15s,
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
  margin-right: 0.375rem;
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

.camera-actions {
  display: flex;
  gap: 0.25rem;
  align-items: center;
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
  transition:
    color 0.15s,
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

.simulator-config {
  background: var(--surface-elevated);
  border-radius: 6px;
  padding: 0.5rem;
  margin-bottom: 0.375rem;
}

.config-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 0.75rem;
  font-weight: 500;
  color: var(--text-primary);
  margin-bottom: 0.375rem;
}

.config-body {
  display: flex;
  gap: 0.375rem;
  align-items: center;
}

.directory-input {
  flex: 1;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.375rem 0.5rem;
  font-size: 0.75rem;
  color: var(--text-primary);
  min-width: 0;
}

.directory-input:focus {
  outline: none;
  border-color: var(--primary);
}

.directory-input::placeholder {
  color: var(--text-muted);
}

.config-hint {
  font-size: 0.65rem;
  color: var(--text-muted);
  margin-top: 0.25rem;
}

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
