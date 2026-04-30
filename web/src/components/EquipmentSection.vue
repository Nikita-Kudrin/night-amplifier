<script setup>
import {ref, computed, watch} from 'vue'
import {useError} from '../composables/useError.js'
import {useTelescopeSetup} from '../composables/useTelescopeSetup.js'
import {CAMERA_DATABASE} from '../constants/cameras.js'
import {TELESCOPE_LIMITS, HELP_TEXTS} from '../constants'
import {BaseToggle, BaseInfoIcon} from './ui'

const props = defineProps({
  connectedCameraInfo: {type: Object, default: null},
  installedDatabases: {type: Array, default: () => []},
  activeDatabaseType: {type: String, default: null},
})

const emit = defineEmits(['fov-changed', 'database-select'])

const {withErrorHandling} = useError()

const equipmentCollapsed = ref(true)
const manualSensorExpanded = ref(false)

const {
  focalLength, pixelSizeX, pixelSizeY,
  sensorWidthPx, sensorHeightPx, barlowCoeff,
  manualPixelSize, calculatedFov, autoFillFromCamera, selectCamera,
} = useTelescopeSetup({withErrorHandling, connectedCameraInfo: computed(() => props.connectedCameraInfo)})

// Emit FOV changes to parent (for FOV warning)
watch(calculatedFov, (fov) => emit('fov-changed', fov), {immediate: true})

// Camera sensor search
const cameraSearchQuery = ref('')
const showCameraResults = ref(false)

const filteredCameras = computed(() => {
  const q = cameraSearchQuery.value.toLowerCase().trim()
  if (!q) return []
  return CAMERA_DATABASE.filter(c =>
      c.brand.toLowerCase().includes(q) ||
      c.model.toLowerCase().includes(q) ||
      c.sensor.toLowerCase().includes(q)
  ).slice(0, 15)
})

function selectCameraEntry(entry) {
  selectCamera(entry)
  cameraSearchQuery.value = `${entry.brand} ${entry.model}`
  showCameraResults.value = false
}

function fillFromConnectedCamera() {
  if (props.connectedCameraInfo) {
    autoFillFromCamera(props.connectedCameraInfo)
    cameraSearchQuery.value = props.connectedCameraInfo.name || ''
  }
}

function formatFov(deg) {
  if (deg >= 1) return `${deg.toFixed(2)}\u00B0`
  return `${(deg * 60).toFixed(1)}'`
}

function handleClickOutside(event) {
  if (!event.target.closest('.camera-search-container')) {
    showCameraResults.value = false
  }
}

const hasMultipleDatabases = computed(() => props.installedDatabases.length > 1)

defineExpose({calculatedFov})
</script>

<template>
  <div class="section equipment-section">
    <div class="equipment-header" @click="equipmentCollapsed = !equipmentCollapsed">
      <svg
          :class="['collapse-chevron', { collapsed: equipmentCollapsed }]"
          viewBox="0 0 24 24"
          width="10"
          height="10"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
      >
        <path d="M6 9l6 6 6-6"/>
      </svg>
      <h3 class="section-title">Equipment</h3>
      <span v-if="calculatedFov && equipmentCollapsed" class="fov-badge">
        {{ formatFov(calculatedFov.x) }} &times; {{ formatFov(calculatedFov.y) }}
      </span>
    </div>

    <div v-show="!equipmentCollapsed" class="equipment-content" @click="handleClickOutside">
      <!-- Focal Length -->
      <div class="eq-row">
        <label class="eq-label">
          Focal length
          <BaseInfoIcon :message="HELP_TEXTS.telescope_focal_length"/>
        </label>
        <div class="input-with-unit eq-input-area">
          <input
              v-model.number="focalLength"
              type="number"
              :min="TELESCOPE_LIMITS.focal_length_min"
              :max="TELESCOPE_LIMITS.focal_length_max"
              placeholder="1000"
              class="telescope-input"
          />
          <span class="input-unit">mm</span>
        </div>
      </div>

      <!-- Camera Sensor -->
      <div class="eq-row eq-row-camera">
        <label class="eq-label">
          Camera
          <BaseInfoIcon :message="HELP_TEXTS.telescope_camera_sensor"/>
        </label>
        <div class="camera-search-container eq-input-area">
          <input
              v-model="cameraSearchQuery"
              type="text"
              placeholder="Search..."
              class="telescope-input"
              @focus="showCameraResults = true"
              @input="showCameraResults = true"
          />
          <div v-if="showCameraResults && filteredCameras.length > 0" class="camera-results">
            <div
                v-for="cam in filteredCameras"
                :key="`${cam.brand}-${cam.model}`"
                class="camera-result-item"
                @click="selectCameraEntry(cam)"
            >
              <div class="camera-result-main">
                <span class="camera-brand">{{ cam.brand }}</span>
                <span class="camera-model">{{ cam.model }}</span>
              </div>
              <div class="camera-result-details">
                <span>{{ cam.sensor }}</span>
                <span>{{ cam.pixel_size_x }}&micro;m</span>
                <span>{{ cam.width }}&times;{{ cam.height }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Auto-fill from connected camera -->
      <button
          v-if="connectedCameraInfo"
          class="btn-autofill"
          @click="fillFromConnectedCamera"
      >
        Use connected: {{ connectedCameraInfo.name }}
      </button>

      <!-- Sensor info (when selected from DB or auto-filled, not manual) -->
      <div v-if="!manualPixelSize && pixelSizeX && sensorWidthPx" class="sensor-info">
        {{ pixelSizeX }}&micro;m &middot; {{ sensorWidthPx }}&times;{{ sensorHeightPx }}px
      </div>

      <!-- Manual Sensor Specs -->
      <div class="manual-sensor-section">
        <div class="manual-sensor-header" @click="manualSensorExpanded = !manualSensorExpanded">
          <svg
              :class="['collapse-chevron', { collapsed: !manualSensorExpanded }]"
              viewBox="0 0 24 24"
              width="9"
              height="9"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
          >
            <path d="M6 9l6 6 6-6"/>
          </svg>
          <span class="manual-sensor-label">Manual sensor specs</span>
          <BaseToggle
              :model-value="manualPixelSize"
              size="small"
              @click.stop
              @update:model-value="manualPixelSize = $event; if ($event) manualSensorExpanded = true"
          />
        </div>
        <div v-show="manualSensorExpanded" class="manual-sensor-content">
          <div class="eq-row">
            <label class="eq-label">Pixel size</label>
            <div class="eq-dual-input">
              <div class="input-with-unit">
                <input
                    v-model.number="pixelSizeX"
                    type="number"
                    :min="TELESCOPE_LIMITS.pixel_size_min"
                    :max="TELESCOPE_LIMITS.pixel_size_max"
                    :step="TELESCOPE_LIMITS.pixel_size_step"
                    placeholder="X"
                    class="telescope-input pixel-input"
                    :disabled="!manualPixelSize"
                />
                <span class="input-unit">&micro;m</span>
              </div>
              <span class="dual-sep">&times;</span>
              <div class="input-with-unit">
                <input
                    v-model.number="pixelSizeY"
                    type="number"
                    :min="TELESCOPE_LIMITS.pixel_size_min"
                    :max="TELESCOPE_LIMITS.pixel_size_max"
                    :step="TELESCOPE_LIMITS.pixel_size_step"
                    placeholder="Y"
                    class="telescope-input pixel-input"
                    :disabled="!manualPixelSize"
                />
                <span class="input-unit">&micro;m</span>
              </div>
            </div>
          </div>
          <div class="eq-row">
            <label class="eq-label">Resolution</label>
            <div class="eq-dual-input">
              <input
                  v-model.number="sensorWidthPx"
                  type="number"
                  min="1"
                  placeholder="W"
                  class="telescope-input pixel-input"
                  :disabled="!manualPixelSize"
              />
              <span class="dual-sep">&times;</span>
              <input
                  v-model.number="sensorHeightPx"
                  type="number"
                  min="1"
                  placeholder="H"
                  class="telescope-input pixel-input"
                  :disabled="!manualPixelSize"
              />
            </div>
          </div>
        </div>
      </div>

      <!-- Barlow/Reducer -->
      <div class="eq-row">
        <label class="eq-label">
          Barlow / Reducer
          <BaseInfoIcon :message="HELP_TEXTS.telescope_barlow"/>
        </label>
        <div class="input-with-unit eq-input-area">
          <input
              v-model.number="barlowCoeff"
              type="number"
              :min="TELESCOPE_LIMITS.barlow_min"
              :max="TELESCOPE_LIMITS.barlow_max"
              :step="TELESCOPE_LIMITS.barlow_step"
              placeholder="1.0"
              class="telescope-input"
          />
          <span class="input-unit">&times;</span>
        </div>
      </div>

      <!-- Calculated FOV -->
      <div v-if="calculatedFov" class="fov-display">
        <span class="fov-label">FOV</span>
        <span class="fov-value">{{ formatFov(calculatedFov.x) }} &times; {{ formatFov(calculatedFov.y) }}</span>
      </div>

      <!-- Active Database selector (shown when multiple databases are installed) -->
      <div v-if="hasMultipleDatabases" class="eq-row database-select-row">
        <label class="eq-label">Database</label>
        <div class="eq-input-area">
          <select
              :value="activeDatabaseType"
              class="telescope-input"
              @change="emit('database-select', $event.target.value)"
          >
            <option v-for="db in installedDatabases" :key="db.id" :value="db.id">
              {{ db.id }} ({{ db.min_fov_deg }}°–{{ db.max_fov_deg }}°)
            </option>
          </select>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.section {
  margin-top: 0.25rem;
}

.section-title {
  font-size: 0.7rem;
  color: var(--text-muted);
  text-transform: uppercase;
  margin-bottom: 0.375rem;
  padding-bottom: 0;
  border-bottom: none;
}

.equipment-header {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  cursor: pointer;
  margin-bottom: 0.25rem;
  user-select: none;
}

.equipment-header .section-title {
  margin-bottom: 0;
  flex: 1;
}

.collapse-chevron {
  transition: transform 0.15s;
  color: var(--text-muted);
  flex-shrink: 0;
}

.collapse-chevron.collapsed {
  transform: rotate(-90deg);
}

.fov-badge {
  font-size: 0.6rem;
  font-family: monospace;
  color: var(--text-secondary);
  white-space: nowrap;
}

.equipment-content {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

.eq-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.eq-label {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: 0.7rem;
  color: var(--text-muted);
  white-space: nowrap;
  flex-shrink: 0;
  min-width: 0;
}

.eq-input-area {
  flex: 1;
  min-width: 0;
}

.eq-row-camera {
  position: relative;
}

.telescope-input {
  width: 100%;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.3rem 0.375rem;
  font-size: 0.75rem;
  color: var(--text-primary);
}

.telescope-input:focus {
  outline: none;
  border-color: var(--primary);
}

.telescope-input::placeholder {
  color: var(--text-muted);
}

.telescope-input:disabled {
  opacity: 0.4;
}

.input-with-unit {
  display: flex;
  align-items: center;
  gap: 0;
}

.input-with-unit .telescope-input {
  border-radius: 4px 0 0 4px;
  border-right: none;
}

.input-unit {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 0 4px 4px 0;
  padding: 0.3rem 0.375rem;
  font-size: 0.65rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.eq-dual-input {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 0;
  min-width: 0;
}

.eq-dual-input .input-with-unit {
  flex: 1;
  min-width: 0;
}

.eq-dual-input > input {
  flex: 1;
  min-width: 0;
}

.dual-sep {
  font-size: 0.6rem;
  color: var(--text-muted);
  padding: 0 0.2rem;
  flex-shrink: 0;
}

.pixel-input {
  font-family: monospace;
  font-size: 0.7rem;
}

.camera-search-container {
  position: relative;
}

.camera-results {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 6px;
  margin-top: 0.25rem;
  max-height: 180px;
  overflow-y: auto;
  z-index: 100;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
}

.camera-result-item {
  padding: 0.375rem 0.5rem;
  cursor: pointer;
  border-bottom: 1px solid var(--border);
}

.camera-result-item:last-child {
  border-bottom: none;
}

.camera-result-item:hover {
  background: var(--surface-hover);
}

.camera-result-main {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  margin-bottom: 0.125rem;
}

.camera-brand {
  font-size: 0.6rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
}

.camera-model {
  font-size: 0.7rem;
  color: var(--text-primary);
}

.camera-result-details {
  display: flex;
  gap: 0.5rem;
  font-size: 0.6rem;
  color: var(--text-muted);
}

.btn-autofill {
  background: none;
  border: 1px dashed var(--border);
  border-radius: 4px;
  padding: 0.2rem 0.5rem;
  font-size: 0.6rem;
  color: var(--primary);
  cursor: pointer;
  width: 100%;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.btn-autofill:hover {
  border-color: var(--primary);
  background: var(--surface);
}

.sensor-info {
  font-size: 0.6rem;
  color: var(--text-secondary);
  font-family: monospace;
  padding: 0.125rem 0;
}

.manual-sensor-section {
  border-top: 1px solid var(--border);
  padding-top: 0.375rem;
}

.manual-sensor-header {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  cursor: pointer;
  user-select: none;
}

.manual-sensor-label {
  font-size: 0.7rem;
  color: var(--text-muted);
  flex: 1;
}

.manual-sensor-content {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  margin-top: 0.375rem;
}

.fov-display {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  background: var(--surface-elevated);
  border-radius: 6px;
  padding: 0.3rem 0.5rem;
  border-left: 3px solid var(--primary);
}

.fov-label {
  font-size: 0.6rem;
  color: var(--text-muted);
  text-transform: uppercase;
  font-weight: 600;
}

.fov-value {
  font-size: 0.75rem;
  color: var(--text-primary);
  font-family: monospace;
}
</style>
